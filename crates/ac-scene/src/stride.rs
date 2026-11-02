//! The body's step through static geometry, as pure functions over what lies underfoot: a
//! frame's walk on its feet ([`stride`]) and in the air ([`fall_step`]). The body steps by them
//! every frame, and the walking graph tests its edges by the same step.

use glam::Vec3;

use crate::collision::{Capsule, Vertical, Walk, GRAVITY};

/// How far under itself a body stepping off an interior ledge looks for a floor (metres): with
/// none within it the step stays put, and a fall longer than it has missed its floor.
pub const FALL_SEARCH: f32 = 200.0;

/// What a step stands on and walks against: the body's one or two blocks, or a region's world.
pub trait Underfoot {
    /// One walking step from `from` to `to` against the walls (only `pos.x`, `pos.y` are read).
    fn walk(&self, from: Vec3, to: Vec3, cap: &Capsule) -> Walk;
    /// A floor over the feet at `at`, higher than a step and lower than the head.
    fn floor_over_feet(&self, at: Vec3, cap: &Capsule) -> bool;
    /// A floor from a step over the feet down to `depth` under them.
    fn floor_below(&self, at: Vec3, cap: &Capsule, depth: f32) -> Option<(f32, u32)>;
    /// The terrain's height at world `(x, y)`; `None` with no terrain there.
    fn terrain_at(&self, x: f32, y: f32) -> Option<f32>;
    /// A dungeon: no terrain anywhere, and no outside.
    fn is_dungeon(&self) -> bool;
    /// A vertical move by `dz`: the highest floor landed on, else the lowest ceiling met.
    fn vertical(&self, from: Vec3, dz: f32, cap: &Capsule) -> Vertical;
    /// A falling capsule from `from` to `to`, pushed out of the walls.
    fn resolve_falling(&self, from: Vec3, to: Vec3, cap: &Capsule) -> Vec3;
    /// A falling capsule fits under every ceiling at `at`.
    fn fits_falling(&self, at: Vec3, cap: &Capsule) -> bool;
}

/// Where a step on the feet ends.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Stride {
    /// On a floor: `cell` is its interior cell, or 0 outdoors.
    Stand { at: Vec3, cell: u32 },
    /// Something solid in the way: the body stays where it was.
    Blocked,
    /// Off an edge at `at`, with a floor or the ground somewhere below.
    Falls { at: Vec3 },
    /// Nothing under it at all (bad data, past the known terrain): the body keeps its height and
    /// cell, and no route walks there.
    Adrift { at: Vec3, cell: u32 },
}

/// Where a step in the air ends.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Air {
    /// Still in the air at `at`, moving up at `vz`; `stopped` when it no longer fits where it
    /// drifted and loses its speed over the ground.
    Flying { at: Vec3, vz: f32, stopped: bool },
    /// Down on a floor: `cell` is its interior cell, or 0 outdoors.
    Landed { at: Vec3, cell: u32 },
}

/// Whether `cell` is an interior cell (the low word at least 0x100).
fn is_interior(cell: u32) -> bool {
    (cell & 0xFFFF) >= 0x100
}

/// One step on the feet from `from` (in `cell`) toward `to`: walls push, ledges up to a step are
/// climbed and drops up to a step down are walked; anything deeper is an edge to fall from.
pub fn stride(u: &impl Underfoot, from: Vec3, cell: u32, to: Vec3, cap: &Capsule) -> Stride {
    let indoors = is_interior(cell);
    let dungeon = indoors && u.is_dungeon();
    let w = u.walk(from, to, cap);
    if w.blocked {
        return Stride::Blocked;
    }
    let mut world = Vec3::new(w.pos.x, w.pos.y, to.z);
    let mut floor = w.floor;
    // No floor within a step and one over the feet: ground rising faster than a step climbs is
    // solid. Taken for an edge, the storey under the stairs was fallen to.
    if floor.is_none() && indoors && u.floor_over_feet(world, cap) {
        return Stride::Blocked;
    }
    // An interior floor under the ground walked on (a cellar, the halls under a hill) is not
    // stood on from outside: the hill is.
    if let Some((z, c)) = floor {
        if c != 0 && !indoors {
            if let Some(t) = u.terrain_at(world.x, world.y) {
                if t > z + 0.5 {
                    floor = Some((t, 0));
                }
            }
        }
    }
    match floor {
        Some((z, c)) if c != 0 => {
            world.z = z;
            Stride::Stand { at: world, cell: c }
        }
        // Untagged geometry in a dungeon (a placed object): stood on, in the same cell.
        Some((z, _)) if dungeon => {
            world.z = z;
            Stride::Stand { at: world, cell }
        }
        // An outdoor floor (a dock, a bridge, a building's step), stood on over the terrain.
        Some((z, _)) if !indoors || z > from.z - 0.5 => {
            world.z = u.terrain_at(world.x, world.y).map_or(z, |t| t.max(z));
            Stride::Stand { at: world, cell: 0 }
        }
        _ if indoors => {
            // No interior floor within a step: the ground outside a step away is the way out of a
            // building whose door opens onto bare terrain; else a fall, or bad data stood on.
            let ground = if dungeon {
                None
            } else {
                u.terrain_at(world.x, world.y)
                    .filter(|&t| t <= from.z + cap.step_up && t >= from.z - cap.step_down)
            };
            if let Some(t) = ground {
                world.z = t;
                Stride::Stand { at: world, cell: 0 }
            } else if u.floor_below(world, cap, FALL_SEARCH).is_some() {
                Stride::Falls { at: world }
            } else {
                world.z = from.z;
                Stride::Adrift { at: world, cell }
            }
        }
        // Bare terrain: followed, unless it drops away by more than a step.
        _ => match u.terrain_at(world.x, world.y) {
            Some(t) if t < from.z - cap.step_down => Stride::Falls { at: world },
            Some(t) => {
                world.z = t;
                Stride::Stand { at: world, cell: 0 }
            }
            None => {
                world.z = from.z;
                Stride::Adrift { at: world, cell: 0 }
            }
        },
    }
}

/// One step in the air from `from` (in `cell`) drifting to `to`, moving up at `vz` before
/// gravity over `dt` seconds: walls push the whole capsule, then land on a floor or bump a ceiling.
pub fn fall_step(
    u: &impl Underfoot,
    from: Vec3,
    to: Vec3,
    vz: f32,
    dt: f32,
    cell: u32,
    cap: &Capsule,
) -> Air {
    let indoors = is_interior(cell);
    let dungeon = indoors && u.is_dungeon();
    let mut world = u.resolve_falling(from, to, cap);
    // No sliding under a porch slab or a staircase while falling past it.
    let stopped = !u.fits_falling(world, cap);
    if stopped {
        world.x = from.x;
        world.y = from.y;
    }
    let vz = vz - GRAVITY * dt;
    let dz = vz * dt;
    let (mut landed, ceiling) = match u.vertical(world, dz, cap) {
        Vertical::Landed(p, c) => (Some((p, c)), None),
        Vertical::Ceiling(p) => (None, Some(p)),
        Vertical::Free(_) => (None, None),
    };
    // Outside a dungeon the terrain is the last resort: under no interior cell, under outdoor
    // geometry, and under nothing at all (a body still carrying a building's cell outside).
    if dz < 0.0 && !dungeon && (!indoors || landed.is_none_or(|(_, c)| c == 0)) {
        if let Some(t) = u.terrain_at(world.x, world.y) {
            let above_landing = landed.is_none_or(|(l, _)| t > l.z);
            if t >= world.z + dz && above_landing {
                landed = Some((Vec3::new(world.x, world.y, t), 0));
            }
        }
    }
    if let Some((at, c)) = landed {
        // Landing on untagged geometry in a dungeon keeps the cell.
        let cell = if c == 0 && dungeon { cell } else { c };
        return Air::Landed { at, cell };
    }
    match ceiling {
        Some(at) => Air::Flying {
            at,
            vz: 0.0,
            stopped,
        },
        None => Air::Flying {
            at: world + Vec3::new(0.0, 0.0, dz),
            vz,
            stopped,
        },
    }
}
