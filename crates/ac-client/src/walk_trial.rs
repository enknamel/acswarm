//! The route harness (docs/movement-plan.md, phase 0): a character set down at a start and sent to
//! a goal the way every system sends it (`Client::head_for`), with the real steering, planners and
//! body run at 20 Hz until it arrives, is refused, gives up, or runs out of time still pushing.
//! Trials come from places named by hand (the stalls seen live) and from seeded random points on
//! the collision's floors, not the walking graph's, whose holes are what is being measured.

use std::time::{Duration, Instant};

use glam::Vec3;

use crate::Client;

/// Frames a second the harness runs the body at, as the headless client does.
const HZ: f32 = 20.0;
/// The autoplay re-asks for its walk about this often (seconds): every caller asks each turn.
const ASK_EVERY: f32 = 0.3;
/// Within the stop distance plus this (metres, 3D) is arrived.
const ARRIVED_SLACK: f32 = 1.0;
/// The steering saying "no way" this long (seconds) is giving up.
const NO_WAY_FOR: f32 = 3.0;
/// The planner's answer is waited for this long each frame (the live planner answers in a frame
/// or two once its area is built; building one takes up to a second).
const PLANNER_WAIT: Duration = Duration::from_secs(5);

/// Where a character is put down and where it is sent.
#[derive(Clone, Debug)]
pub struct Trial {
    pub name: String,
    /// The start, a world position on a floor, and the cell it stands in.
    pub start: Vec3,
    pub start_cell: u32,
    /// The goal, a world position on a floor.
    pub goal: Vec3,
    /// How near is near enough (metres): what the caller would ask for.
    pub stop: f32,
    /// How long it is given (seconds).
    pub limit: f32,
}

/// How a trial ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// Within reach of the goal, height and all.
    Arrived { secs: f32 },
    /// Turned down before a step was taken.
    Refused { why: String },
    /// The walk stopped short, after `secs`, `left` metres from the goal.
    GaveUp { secs: f32, left: f32 },
    /// Still walking at the limit, `left` metres from the goal, `moved` from the start.
    StillPushing { left: f32, moved: f32 },
}

impl Outcome {
    /// The kind of outcome, for counting.
    pub fn kind(&self) -> &'static str {
        match self {
            Outcome::Arrived { .. } => "arrived",
            Outcome::Refused { .. } => "refused",
            Outcome::GaveUp { .. } => "gave up",
            Outcome::StillPushing { .. } => "still pushing",
        }
    }
}

/// Run one trial on a character of `level`.
pub fn run(level: i32, t: &Trial) -> Outcome {
    let origin = ac_world::landblock_origin(t.start_cell);
    let mut c = crate::testkit::standing_in_the_field(level, t.start_cell, t.start - origin);
    run_on(&mut c, t)
}

/// Run one trial on `c`, already standing at the trial's start.
pub fn run_on(c: &mut Client, t: &Trial) -> Outcome {
    let dt = 1.0 / HZ;
    let t0 = Instant::now();
    let start = c.my_position().unwrap_or(t.start);
    let first = c.head_for(t.goal, t.stop, &t.name);
    if !first.acting() {
        return Outcome::Refused {
            why: first
                .because()
                .map_or_else(|| format!("{first:?}"), |b| format!("{b:?}")),
        };
    }
    let mut no_way_since: Option<f32> = None;
    let mut asked = 0.0;
    let frames = (t.limit * HZ) as u32;
    for frame in 0..frames {
        let secs = frame as f32 * dt;
        let now = t0 + Duration::from_secs_f32(secs);
        if secs - asked >= ASK_EVERY {
            asked = secs;
            let again = c.head_for(t.goal, t.stop, &t.name);
            if !again.acting() {
                let left = c.my_position().map_or(f32::NAN, |p| p.distance(t.goal));
                return if frame == 0 {
                    Outcome::Refused {
                        why: format!("{again:?}"),
                    }
                } else {
                    Outcome::GaveUp { secs, left }
                };
            }
        }
        c.tick_player(crate::player::Input::default(), dt, now);
        c.pathfinder.settle(PLANNER_WAIT);
        let Some(me) = c.my_position() else {
            return Outcome::GaveUp {
                secs,
                left: f32::NAN,
            };
        };
        if me.distance(t.goal) <= t.stop + ARRIVED_SLACK {
            return Outcome::Arrived { secs };
        }
        if c.steering.no_way() {
            let since = *no_way_since.get_or_insert(secs);
            if secs - since >= NO_WAY_FOR {
                return if since < 1.0 {
                    Outcome::Refused {
                        why: "the steering has no way".into(),
                    }
                } else {
                    Outcome::GaveUp {
                        secs,
                        left: me.distance(t.goal),
                    }
                };
            }
        } else {
            no_way_since = None;
        }
    }
    let me = c.my_position().unwrap_or(start);
    Outcome::StillPushing {
        left: me.distance(t.goal),
        moved: me.distance(start),
    }
}

/// A seeded generator, so a list of random trials is the same list every run.
pub struct Seeded(u64);

impl Seeded {
    pub fn new(seed: u64) -> Self {
        Seeded(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    /// The next number in `0.0..1.0`.
    pub fn unit(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// A place a body of `c`'s shape can stand within `radius` (flat) of `around`, on a collision floor
/// or the terrain, and the cell it is in: `None` after a few tries finds none. Taken from the
/// collision, not the walking graph.
pub fn standing_place(
    c: &mut Client,
    around: Vec3,
    radius: f32,
    rng: &mut Seeded,
) -> Option<(Vec3, u32)> {
    let assets = c.assets.clone();
    let pl = c.player.as_mut()?;
    let cap = pl.capsule();
    let dungeon = pl.is_indoors() && pl.in_dungeon(&assets);
    for _ in 0..64 {
        let a = rng.unit() * std::f32::consts::TAU;
        let d = radius * rng.unit().sqrt();
        let (x, y) = (around.x + a.cos() * d, around.y + a.sin() * d);
        let block = crate::player::block_of(Vec3::new(x, y, 0.0));
        let collision_block = if dungeon { pl.landblock() } else { block };
        let Ok(bc) = assets.block_collision(collision_block) else {
            continue;
        };
        let mut levels: Vec<(f32, u32)> = bc.world.floors_at_xy(x, y);
        if !dungeon {
            pl.collision(&assets, block);
            if let Some(z) = pl.terrain_height(x, y) {
                levels.push((z, 0));
            }
        }
        if levels.is_empty() {
            continue;
        }
        let (z, cell) = levels[(rng.unit() * levels.len() as f32) as usize % levels.len()];
        let p = Vec3::new(x, y, z);
        if bc
            .world
            .wall_contact(p, cap.radius, cap.height, cap.step_up)
            || bc
                .world
                .ceiling_at(p, cap.radius)
                .is_some_and(|c| c - z < cap.height)
        {
            continue;
        }
        let cell = if cell != 0 {
            cell
        } else {
            let origin = ac_world::landblock_origin(block);
            ac_world::outdoor_cell(block, p - origin)
        };
        return Some((p, cell));
    }
    None
}

/// How random trials are drawn: starts within `radius` (metres) of a place, each goal within
/// `reach` of its start and at least `least` from it, `n` of them from `seed`.
#[derive(Clone, Copy, Debug)]
pub struct Spread {
    pub radius: f32,
    pub reach: f32,
    pub least: f32,
    pub n: usize,
    pub seed: u64,
}

/// Seeded random trials around `around` (a world position and its cell), named `name #i`.
pub fn random_trials(level: i32, name: &str, around: (Vec3, u32), s: Spread) -> Vec<Trial> {
    let origin = ac_world::landblock_origin(around.1);
    let mut c = crate::testkit::standing_in_the_field(level, around.1, around.0 - origin);
    let mut rng = Seeded::new(s.seed);
    let mut out = Vec::new();
    let mut tries = 0;
    while out.len() < s.n && tries < s.n * 20 {
        tries += 1;
        let Some((start, start_cell)) = standing_place(&mut c, around.0, s.radius, &mut rng) else {
            continue;
        };
        let Some((goal, _)) = standing_place(&mut c, start, s.reach, &mut rng) else {
            continue;
        };
        if goal.truncate().distance(start.truncate()) < s.least {
            continue;
        }
        out.push(Trial {
            name: format!("{name} #{}", out.len()),
            start,
            start_cell,
            goal,
            stop: 1.5,
            limit: 120.0,
        });
    }
    out
}

/// `p` put on the floor (or terrain) at its x, y nearest its height, as `c` would stand there.
pub fn on_floor(c: &mut Client, p: Vec3) -> Vec3 {
    let assets = c.assets.clone();
    let Some(pl) = c.player.as_mut() else {
        return p;
    };
    let dungeon = pl.is_indoors() && pl.in_dungeon(&assets);
    let block = if dungeon {
        pl.landblock()
    } else {
        crate::player::block_of(p)
    };
    let mut levels: Vec<f32> = assets
        .block_collision(block)
        .map(|bc| {
            bc.world
                .floors_at_xy(p.x, p.y)
                .into_iter()
                .map(|f| f.0)
                .collect()
        })
        .unwrap_or_default();
    if !dungeon {
        pl.collision(&assets, block);
        levels.extend(pl.terrain_height(p.x, p.y));
    }
    levels
        .into_iter()
        .min_by(|a, b| (a - p.z).abs().total_cmp(&(b - p.z).abs()))
        .map_or(p, |z| Vec3::new(p.x, p.y, z))
}

/// A trial between two places named by hand, both put on their floors.
pub fn named(level: i32, name: &str, start: Vec3, start_cell: u32, goal: Vec3) -> Trial {
    let origin = ac_world::landblock_origin(start_cell);
    let mut c = crate::testkit::standing_in_the_field(level, start_cell, start - origin);
    let start = on_floor(&mut c, start);
    let goal = on_floor(&mut c, goal);
    Trial {
        name: name.into(),
        start,
        start_cell,
        goal,
        stop: 1.5,
        limit: 120.0,
    }
}
