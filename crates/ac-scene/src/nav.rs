//! The walking graph of a rectangle (a block, an area): feet positions on a lattice over the
//! static collision and the terrain, one node per level in a column, snapped out of walls. Edges
//! are directed, each walked by the body's own step ([`Ground::hop`], [`crate::stride`]). Chunks
//! build on demand; routes are searched in [`crate::wayfind`].

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use glam::{Vec2, Vec3};

use crate::collision::{Capsule, CollisionWorld, Tri, Vertical, Walk};
use crate::stride::{fall_step, stride, Air, Stride, Underfoot};

/// Grid spacing inside dungeons and buildings (metres).
pub const INDOOR_SPACING: f32 = 1.5;
/// Grid spacing over open terrain.
pub const OUTDOOR_SPACING: f32 = 4.0;
/// How far apart the capsule is sampled along an edge.
const SUBSTEP: f32 = 0.5;
/// Height above the feet of the ray that must be clear between samples.
const CHEST: f32 = 1.0;
/// Levels closer than this in one column are one floor.
const LEVEL_MERGE: f32 = 0.3;
/// Largest height difference between two neighbouring nodes worth
/// testing (stairs and ramps within one grid step).
const MAX_EDGE_RISE: f32 = 2.0;
/// Ramp points stand this far apart (metres), well under an edge's [`MAX_EDGE_RISE`] up a slope.
const RAMP_STEP: f32 = 1.0;
/// A floor sloping between these (its normal's z) is a ramp: steeper is a wall, flatter is level
/// ground the lattice already covers (`CollisionWorld::floors_at_xy` takes 0.5 as a floor).
const RAMP_FLOOR: f32 = 0.5;
const RAMP_LEVEL: f32 = 0.97;
/// A snapped node may move at most this fraction of the spacing.
const SNAP_FRACTION: f32 = 0.6;
/// How far one step of the body's walk goes (metres): a frame's run at 20 Hz.
const BODY_STEP: f32 = 0.2;
/// How near the body's step must come to the far node of an edge, flat (metres).
pub const BODY_ARRIVE: f32 = 0.25;
/// The least share of a step the body's walk must close on the far node; less is a wall.
const BODY_CLOSING: f32 = 0.3;
/// How far ahead (metres) a walk test gathers what it may step on at once: every step scans it.
const SWEPT_SPAN: f32 = 4.0;
/// A walk this near its end (metres) is there: the float error of its last step.
const TOUCHING: f32 = 0.01;
/// How fast a body steps off a ledge (m/s): the phase-2 follower's pace at a drop.
pub const DROP_SPEED: f32 = 1.0;
/// The deepest drop (metres): ACE hurts a landing over 11.25 + 4.5 m/s (Player_Move.cs:254-259).
const DROP_MAX: f32 = 12.0;
/// A fall still in the air this long (seconds) is over nothing: the highest jump's flight.
const OVER_NOTHING: f32 = 2.8;
/// The steps a fall is followed in (seconds): a frame at 20 Hz.
const FALL_DT: f32 = 0.05;

/// Where the ground is: static collision plus, outdoors, a terrain
/// height function over world `(x, y)`.
/// Metres above or below a portal's mouth its berth reaches: under a storey, so the floors over and
/// under it stay walkable (the Facility Hub's stand 6 m apart, a Holtburg house's 3 m).
pub const BERTH_HEIGHT: f32 = 2.5;

/// Whether feet at `p` are within `berth` of the portal mouth `mouth`, on its floor
/// (test: a_portal_berth_keeps_to_its_own_floor).
pub fn in_berth(mouth: Vec3, p: Vec3, berth: f32) -> bool {
    flat(p - mouth).length() < berth && (p.z - mouth.z).abs() < BERTH_HEIGHT
}

pub struct Ground<'a> {
    pub collision: &'a CollisionWorld,
    pub terrain: Option<&'a dyn Fn(f32, f32) -> Option<f32>>,
    /// Where the terrain is open sea. A character cannot walk into it:
    /// the server refuses the move and the character stops dead against
    /// nothing, which is what an invisible wall in the ocean is. Only
    /// the terrain is ruled out, not the static geometry over it, so
    /// piers and bridges over water are still walked.
    pub sea: Option<&'a dyn Fn(f32, f32) -> bool>,
    /// The middle of every opening between cells: doorways, arches, the
    /// gaps between rooms (see `interior::CellScene::doorways`).
    ///
    /// A node is placed in each whatever the lattice would have done,
    /// because a doorway is narrower than the lattice is coarse and a
    /// character is nearly as wide as a doorway. Left to the lattice,
    /// Holtburg came out as ninety-odd islands with every building
    /// sealed, and a monster ten feet away inside one had no path to it
    /// at all -- so the steering ran at the wall between instead.
    pub doorways: &'a [Vec3],
}

impl<'a> Ground<'a> {
    /// The surface a capsule with its feet at `p` stands on: the highest
    /// interior floor within step range, else the higher of an outdoor
    /// floor and the terrain (the rule `Player::update` walks by).
    pub fn surface_at(&self, p: Vec3, cap: &Capsule) -> Option<(f32, u32)> {
        let floor = self
            .collision
            .footing(p, cap.radius, cap.step_up, cap.step_down);
        // Inside something: its floor is what we stand on -- unless the
        // ground runs above it, when it is a cellar or a dungeon under
        // a hill, and from up here the hill is the floor.
        //
        // "From up here" is the whole of it, and the cells say where we
        // are exactly: the client picks the cells it collides against
        // before it collides with anything, and a capsule inside an
        // interior cell never meets the land cell's terrain at all.
        // Holtburg is built down a slope and several houses have a
        // lower storey below the outside grade, so without this the
        // hillside outside is taken for the floor of the room we are
        // standing in and the stairs up become unreachable.
        if let Some((z, cell)) = floor {
            if cell != 0 {
                let outside = !self.collision.in_known_cell(p + Vec3::new(0.0, 0.0, 0.1));
                let buried = outside
                    && self
                        .terrain_under(p.x, p.y)
                        .is_some_and(|t| t > z + 0.5 && p.z >= t - cap.step_down);
                if !buried {
                    return Some((z, cell));
                }
            }
        }
        match self.terrain_under(p.x, p.y) {
            Some(t) => match floor {
                Some((z, _)) if z >= t => Some((z, 0)),
                _ if t <= p.z + cap.step_up && t >= p.z - cap.step_down => Some((t, 0)),
                _ => None,
            },
            // A dungeon: nothing but its own floors.
            None if self.terrain.is_none() => floor,
            // Open sea: only what has been built over it counts, so a
            // pier is walked and the water beside it is not.
            None if self.sea.is_some_and(|w| w(p.x, p.y)) => floor,
            // Off the edge of the terrain we know.
            None => None,
        }
    }

    /// The height of the walkable terrain at a world `(x, y)`: `None`
    /// where there is no terrain at all, and `None` over open sea.
    pub fn terrain_under(&self, x: f32, y: f32) -> Option<f32> {
        if self.sea.is_some_and(|w| w(x, y)) {
            return None;
        }
        self.terrain?(x, y)
    }

    /// The capsule fits at `feet`: touching no wall, head room above.
    pub fn fits_here(&self, feet: Vec3, cap: &Capsule) -> bool {
        self.fits(feet, cap)
    }

    /// The capsule fits at `feet`: touching no wall, head room above.
    fn fits(&self, feet: Vec3, cap: &Capsule) -> bool {
        if self
            .collision
            .wall_contact(feet, cap.radius, cap.height, cap.step_up)
        {
            return false;
        }
        match self.collision.ceiling_at(feet, cap.radius) {
            Some(cz) => cz - feet.z >= cap.height,
            None => true,
        }
    }

    /// The old steering's straight check (`Player::line_blocked`), not an edge test: the walk from
    /// `a` to `b` sampled every `SUBSTEP` (fits, floor, chest ray), each way.
    pub fn walkable(&self, a: Vec3, b: Vec3, cap: &Capsule) -> (bool, bool) {
        let d = b - a;
        let len = flat(d).length();
        let steps = (len / SUBSTEP).ceil().max(1.0) as usize;
        // Track the floor with the wider of the two limits both ways and
        // judge the profile per direction afterwards.
        let range = cap.step_up.max(cap.step_down);
        let probe = Capsule {
            step_up: range,
            step_down: range,
            ..*cap
        };
        let (mut forward, mut backward) = (true, true);
        let mut prev = a;
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let at = Vec3::new(a.x + d.x * t, a.y + d.y * t, prev.z);
            let Some((z, _)) = self.surface_at(at, &probe) else {
                return (false, false);
            };
            let here = Vec3::new(at.x, at.y, z);
            if !self.fits(here, cap) || !line_clear(self.collision, prev, here) {
                return (false, false);
            }
            let dz = here.z - prev.z;
            if dz > cap.step_up || dz < -cap.step_down {
                forward = false;
            }
            if -dz > cap.step_up || -dz < -cap.step_down {
                backward = false;
            }
            if !forward && !backward {
                return (false, false);
            }
            prev = here;
        }
        // We must have arrived on b's floor, not one above or below it.
        if (prev.z - b.z).abs() > LEVEL_MERGE {
            return (false, false);
        }
        (forward, backward)
    }

    /// The edge test between two nodes, each way: the chest line clear ([`is_chest_clear`]) and
    /// the body's own step ([`stride_toward`](Self::stride_toward)) to within [`BODY_ARRIVE`].
    pub fn hop(
        &self,
        a: &NavNode,
        b: &NavNode,
        cap: &Capsule,
    ) -> (Option<EdgeKind>, Option<EdgeKind>) {
        if (a.pos.z - b.pos.z).abs() > MAX_EDGE_RISE
            || !is_chest_clear(self.collision, a.pos, b.pos)
        {
            return (None, None);
        }
        let mut swept = Swept::over(self, a.pos, b.pos, cap);
        let mut one_way = |p: &NavNode, q: &NavNode| {
            matches!(
                self.stride_over(&mut swept, p.pos, p.cell, q.pos, BODY_ARRIVE, cap),
                Reach::Reached { .. }
            )
            .then(|| walk_kind(p.cell, q.cell))
        };
        let there = one_way(a, b);
        (there, one_way(b, a))
    }

    /// Whether a route's leg from `a` to `b` (feet, each with its cell) is walked: the chest line
    /// clear and the body's step one way, as [`hop`](Self::hop) tests an edge.
    pub fn can_walk(&self, a: (Vec3, u32), b: (Vec3, u32), cap: &Capsule) -> bool {
        is_chest_clear(self.collision, a.0, b.0) && self.body_reaches(a.0, a.1, b.0, cap)
    }

    /// Whether the body's step carries it from `a` (feet, in `cell`) to within [`BODY_ARRIVE`]
    /// of `b`, whatever the chest line meets.
    pub fn body_reaches(&self, a: Vec3, cell: u32, b: Vec3, cap: &Capsule) -> bool {
        self.body_stops_within(a, cell, b, BODY_ARRIVE, cap)
            .is_some()
    }

    /// Where the walk of [`body_reaches`](Self::body_reaches) stops when `within` metres of `b`
    /// (flat, on its floor) is there, as a corridor's aim by its wall is walked to; `None` short.
    pub fn body_stops_within(
        &self,
        a: Vec3,
        cell: u32,
        b: Vec3,
        within: f32,
        cap: &Capsule,
    ) -> Option<Vec3> {
        match self.stride_toward(a, cell, b, within, cap) {
            Reach::Reached { at, .. } => Some(at),
            _ => None,
        }
    }

    /// Where the body's step from `a` (feet, in `cell`) toward `b`, level, goes over an edge and lands,
    /// and in which cell, stepping off at [`DROP_SPEED`] as every drop edge is simulated; else `None`.
    pub fn drop_toward(&self, a: Vec3, cell: u32, b: Vec3, cap: &Capsule) -> Option<(Vec3, u32)> {
        let level = Vec3::new(b.x, b.y, a.z);
        let Reach::Falls { at } = self.stride_toward(a, cell, level, BODY_ARRIVE, cap) else {
            return None;
        };
        let dir = flat(b - a).normalize_or_zero() * DROP_SPEED;
        self.fall_from(at, Vec3::new(dir.x, dir.y, 0.0), cell, cap)
    }

    /// Where a body going over an edge at `at` (feet, from `cell`) at `vel` lands, and in which
    /// cell, followed as it falls ([`fall_step`]); `None` deeper than `DROP_MAX` or over nothing.
    pub fn fall_from(&self, at: Vec3, vel: Vec3, cell: u32, cap: &Capsule) -> Option<(Vec3, u32)> {
        let drift = at + vel * OVER_NOTHING - Vec3::new(0.0, 0.0, DROP_MAX);
        let u = Swept::over(self, at, drift, cap);
        let (mut p, mut vel, mut vz) = (at, vel, 0.0);
        for _ in 0..(OVER_NOTHING / FALL_DT) as usize {
            match fall_step(&u, p, p + vel * FALL_DT, vz, FALL_DT, cell, cap) {
                Air::Landed { at: land, cell } => {
                    return (at.z - land.z <= DROP_MAX).then_some((land, cell));
                }
                Air::Flying {
                    at: next,
                    vz: v,
                    stopped,
                } => {
                    if stopped {
                        vel = Vec3::ZERO;
                    }
                    (p, vz) = (next, v);
                    if at.z - p.z > DROP_MAX {
                        return None;
                    }
                }
            }
        }
        None
    }

    /// Where the body's own step ([`stride`]) carries it from `a` (feet, in `cell`) toward `b`, a
    /// fifth of a metre at a time and always closing on it, until `within` of `b` on its floor.
    pub fn stride_toward(&self, a: Vec3, cell: u32, b: Vec3, within: f32, cap: &Capsule) -> Reach {
        let near = a + (b - a).clamp_length_max(SWEPT_SPAN);
        self.stride_over(
            &mut Swept::over(self, a, near, cap),
            a,
            cell,
            b,
            within,
            cap,
        )
    }

    /// [`stride_toward`](Self::stride_toward) stepping on `u`, gathered again [`SWEPT_SPAN`] on
    /// whenever a step leaves it: the same steps, each scanning only what lies near it.
    fn stride_over<'g>(
        &'g self,
        u: &mut Swept<'g, 'a>,
        a: Vec3,
        cell: u32,
        b: Vec3,
        within: f32,
        cap: &Capsule,
    ) -> Reach {
        let (mut p, mut cell) = (a, cell);
        let mut left = flat(b - a).length();
        for _ in 0..(left / BODY_STEP).ceil() as usize + 4 {
            if left <= within.max(TOUCHING) {
                // A step from `b`'s floor is on it: down a stair, the ramp walked lies 0.33 m under the
                // tread its node stands on (test: the_stairs_up_to_a_scrivener_are_walked_down_too).
                return if (p.z - b.z).abs() <= cap.step_up {
                    Reach::Reached { at: p, cell }
                } else {
                    Reach::Stopped
                };
            }
            let d = flat(b - p);
            let step = d.normalize() * d.length().min(BODY_STEP);
            let to = Vec3::new(p.x + step.x, p.y + step.y, p.z);
            if !u.covers(p, to, cap) {
                let ahead = Vec3::new(b.x, b.y, p.z) - p;
                *u = Swept::over(self, p, p + ahead.clamp_length_max(SWEPT_SPAN), cap);
            }
            let (next, next_cell) = match stride(&*u, p, cell, to, cap) {
                Stride::Stand { at, cell } => (at, cell),
                Stride::Falls { at } => return Reach::Falls { at },
                Stride::Blocked | Stride::Adrift { .. } => return Reach::Stopped,
            };
            // A step that closes little is sliding along a wall the hop runs into.
            let now_left = flat(b - next).length();
            if now_left > left - BODY_STEP * BODY_CLOSING {
                return Reach::Stopped;
            }
            left = now_left;
            (p, cell) = (next, next_cell);
        }
        Reach::Stopped
    }
}

/// Where the body's step toward a point ends ([`Ground::stride_toward`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Reach {
    /// Within reach of it, on its floor, standing in `cell` (0 outdoors).
    Reached { at: Vec3, cell: u32 },
    /// Off an edge at `at` on the way.
    Falls { at: Vec3 },
    /// Held by a wall, sliding along one, or off every floor.
    Stopped,
}

impl Underfoot for Ground<'_> {
    fn walk(&self, from: Vec3, to: Vec3, cap: &Capsule) -> Walk {
        self.collision.walk(from, to, cap)
    }

    fn floor_over_feet(&self, at: Vec3, cap: &Capsule) -> bool {
        self.collision
            .floor_at(at, cap.height, 0.0)
            .is_some_and(|(z, _)| z > at.z + cap.step_up)
    }

    fn floor_below(&self, at: Vec3, cap: &Capsule, depth: f32) -> Option<(f32, u32)> {
        self.collision.floor_at(at, cap.step_up, depth)
    }

    /// The walkable terrain: none over open sea, so only what is built over it is stood on.
    fn terrain_at(&self, x: f32, y: f32) -> Option<f32> {
        self.terrain_under(x, y)
    }

    fn is_dungeon(&self) -> bool {
        self.terrain.is_none()
    }

    fn vertical(&self, from: Vec3, dz: f32, cap: &Capsule) -> Vertical {
        self.collision.vertical(from, dz, cap)
    }

    fn resolve_falling(&self, from: Vec3, to: Vec3, cap: &Capsule) -> Vec3 {
        self.collision
            .resolve_from(Some(from), to, cap.radius, cap.height, 0.0)
    }

    fn fits_falling(&self, at: Vec3, cap: &Capsule) -> bool {
        self.collision
            .ceiling_at(at, cap.radius)
            .is_none_or(|cz| cz - at.z >= cap.height)
    }
}

/// A [`Ground`] for one hop, with what near it can push a body or hang over it: a step that has
/// none of those within its reach is its footing alone, which is all the full walk does there.
struct Swept<'g, 'a> {
    ground: &'g Ground<'a>,
    /// The bounds of every wall, ceiling and two-sided floor near the hop.
    pushers: Vec<(Vec3, Vec3)>,
    /// Every floor near the hop, in the order the world holds them.
    floors: Vec<&'g Tri>,
    /// The box they were gathered in.
    lo: Vec3,
    hi: Vec3,
}

impl<'g, 'a> Swept<'g, 'a> {
    fn over(ground: &'g Ground<'a>, a: Vec3, b: Vec3, cap: &Capsule) -> Self {
        let reach = cap.radius + 1.0;
        let lo = a.min(b) - Vec3::new(reach, reach, MAX_EDGE_RISE + cap.step_down + 0.1);
        let hi = a.max(b) + Vec3::new(reach, reach, MAX_EDGE_RISE + cap.step_up + cap.height + 0.1);
        let r = flat(b - a).length() * 0.5 + reach;
        let pushers = ground
            .collision
            .around((a + b) * 0.5, r)
            .map(|(_, t)| t)
            .filter(|t| {
                t.normal.z.abs() <= 0.6 || t.normal.z < -0.5 || (t.two_sided && t.normal.z > 0.5)
            })
            .map(|t| (t.a.min(t.b).min(t.c), t.a.max(t.b).max(t.c)))
            .filter(|(tlo, thi)| tlo.cmple(hi).all() && thi.cmpge(lo).all())
            .collect();
        let mut ids: Vec<u32> = ground
            .collision
            .around((a + b) * 0.5, r)
            .filter(|(_, t)| {
                let (tlo, thi) = (t.a.min(t.b).min(t.c), t.a.max(t.b).max(t.c));
                t.normal.z >= 0.5
                    && flat(tlo).cmple(flat(hi)).all()
                    && flat(thi).cmpge(flat(lo)).all()
            })
            .map(|(i, _)| i)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        let floors = ground.collision.tris_at(&ids);
        Swept {
            ground,
            pushers,
            floors,
            lo,
            hi,
        }
    }

    /// Whether `p` lies inside the box the floors were gathered in, on the flat.
    fn holds(&self, p: Vec3) -> bool {
        flat(p).cmpge(flat(self.lo)).all() && flat(p).cmple(flat(self.hi)).all()
    }

    /// The box a walking step from `from` to `to` may meet anything in: within a radius of its
    /// line, from a step down under its feet to a step and a height over.
    fn step_box(from: Vec3, to: Vec3, cap: &Capsule) -> (Vec3, Vec3) {
        let m = cap.radius + 0.05;
        let lo = from.min(to) - Vec3::new(m, m, cap.step_down + 0.05);
        let hi = from.max(to) + Vec3::new(m, m, cap.step_up + cap.height + 0.05);
        (lo, hi)
    }

    /// Whether the step from `from` to `to` lies inside what was gathered, so its fast answer holds.
    fn covers(&self, from: Vec3, to: Vec3, cap: &Capsule) -> bool {
        let (lo, hi) = Self::step_box(from, to, cap);
        lo.cmpge(self.lo).all() && hi.cmple(self.hi).all()
    }

    /// Whether a walking step from `from` to `to` can meet nothing that pushes or hangs over.
    fn is_clear(&self, from: Vec3, to: Vec3, cap: &Capsule) -> bool {
        let (lo, hi) = Self::step_box(from, to, cap);
        lo.cmpge(self.lo).all()
            && hi.cmple(self.hi).all()
            && !self
                .pushers
                .iter()
                .any(|(tlo, thi)| tlo.cmple(hi).all() && thi.cmpge(lo).all())
    }
}

impl Underfoot for Swept<'_, '_> {
    fn walk(&self, from: Vec3, to: Vec3, cap: &Capsule) -> Walk {
        if !self.is_clear(from, Vec3::new(to.x, to.y, from.z), cap) {
            return self.ground.walk(from, to, cap);
        }
        let probe = Vec3::new(to.x, to.y, from.z);
        let floor = self.ground.collision.footing_among(
            &self.floors,
            probe,
            cap.radius,
            cap.step_up,
            cap.step_down,
        );
        Walk {
            pos: Vec3::new(to.x, to.y, floor.map_or(from.z, |(z, _)| z)),
            floor,
            blocked: false,
        }
    }

    fn floor_over_feet(&self, at: Vec3, cap: &Capsule) -> bool {
        self.ground.floor_over_feet(at, cap)
    }

    fn floor_below(&self, at: Vec3, cap: &Capsule, depth: f32) -> Option<(f32, u32)> {
        self.ground.floor_below(at, cap, depth)
    }

    fn terrain_at(&self, x: f32, y: f32) -> Option<f32> {
        self.ground.terrain_at(x, y)
    }

    fn is_dungeon(&self) -> bool {
        self.ground.is_dungeon()
    }

    fn vertical(&self, from: Vec3, dz: f32, cap: &Capsule) -> Vertical {
        let to = from + Vec3::new(0.0, 0.0, dz);
        if dz < 0.0 && self.holds(from) {
            let floors = self.floors.iter().copied();
            return match self
                .ground
                .collision
                .floor_among(floors, from, cap.step_up, -dz)
            {
                Some((z, cell)) => Vertical::Landed(Vec3::new(from.x, from.y, z), cell),
                None => Vertical::Free(to),
            };
        }
        if dz >= 0.0 && dz <= cap.step_up && self.is_clear(from, from, cap) {
            return Vertical::Free(to);
        }
        self.ground.vertical(from, dz, cap)
    }

    fn resolve_falling(&self, from: Vec3, to: Vec3, cap: &Capsule) -> Vec3 {
        if self.is_clear(from, to, cap) {
            return to;
        }
        self.ground.resolve_falling(from, to, cap)
    }

    fn fits_falling(&self, at: Vec3, cap: &Capsule) -> bool {
        self.is_clear(at, at, cap) || self.ground.fits_falling(at, cap)
    }
}

/// A straight walk from `a` to `b` has a clear ray at chest height:
/// parallel to the ground, or level with the start when `b` is a drop
/// (walking off a ledge is fine; the ray should not dip into its face).
pub fn line_clear(collision: &CollisionWorld, a: Vec3, b: Vec3) -> bool {
    let up = Vec3::new(0.0, 0.0, CHEST);
    let end = if b.z < a.z - LEVEL_MERGE {
        Vec3::new(b.x, b.y, a.z)
    } else {
        b
    };
    collision.segment_hit(a + up, end + up).is_none()
}

/// The chest line between two feet positions, one line for both ways: clear when the ray from
/// either end is ([`line_clear`] levels a ray that goes down), so a stair is climbed as walked down.
pub fn is_chest_clear(collision: &CollisionWorld, a: Vec3, b: Vec3) -> bool {
    line_clear(collision, a, b) || line_clear(collision, b, a)
}

pub(crate) fn flat(v: Vec3) -> Vec2 {
    Vec2::new(v.x, v.y)
}

#[derive(Debug, Clone, Copy)]
pub struct NavNode {
    /// Feet position, world space.
    pub pos: Vec3,
    /// Interior cell the floor belongs to, 0 outdoors.
    pub cell: u32,
    /// Lattice column the node was sampled in (it may have been snapped
    /// a little way out of it).
    pub column: (i32, i32),
}

/// How a route steps from one node to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    /// Along one cell's floor, or outdoors.
    Walk,
    /// Off a ledge onto a floor below: one way.
    Drop,
    /// From one cell into another, walked straight through.
    Doorway,
}

/// An edge out of a node: the node it leads to, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavEdge {
    pub to: u32,
    pub kind: EdgeKind,
}

/// The kind of a walk between two floors' cells (0 outdoors).
pub fn walk_kind(from_cell: u32, to_cell: u32) -> EdgeKind {
    if from_cell == to_cell {
        EdgeKind::Walk
    } else {
        EdgeKind::Doorway
    }
}

/// Landing nodes tried for a drop, nearest first.
const JOIN_TRIES: usize = 8;

/// Join the sets of `a` and `b` in a union-find, halving the way up as it goes.
fn unite(roots: &mut [u32], a: u32, b: u32) {
    let (a, b) = (find_root(roots, a), find_root(roots, b));
    if a != b {
        roots[a.max(b) as usize] = a.min(b);
    }
}

/// The root of `node`'s set, halving the way up.
fn find_root(roots: &mut [u32], node: u32) -> u32 {
    let mut n = node;
    while roots[n as usize] != n {
        let up = roots[roots[n as usize] as usize];
        roots[n as usize] = up;
        n = up;
    }
    n
}

/// The root of `node`'s set, read only, for a graph shared.
fn root_of(roots: &[u32], node: u32) -> u32 {
    let mut n = node;
    while roots[n as usize] != n {
        n = roots[n as usize];
    }
    n
}

/// Columns per chunk side: chunks are built on demand, so a path only
/// pays for the part of the block it crosses (24 m squares indoors).
const CHUNK: i32 = 16;

pub struct NavGraph {
    pub spacing: f32,
    pub capsule: Capsule,
    /// Lattice range (inclusive) the graph may cover.
    gx: (i32, i32),
    gy: (i32, i32),
    pub nodes: Vec<NavNode>,
    /// Outgoing edges per node.
    edges: Vec<Vec<NavEdge>>,
    /// Union-find over the edges of every kind, each way: nodes under different roots are apart.
    roots: Vec<u32>,
    /// The same over walks and doorways only, for a walk that takes no drop.
    walk_roots: Vec<u32>,
    /// Nodes per grid column `(x / spacing, y / spacing)`.
    columns: HashMap<(i32, i32), Vec<u32>>,
    /// Chunks whose nodes and edges exist.
    built: HashSet<(i32, i32)>,
    /// Drops whose landing lies in a chunk not built yet: the node dropped from, where it lands
    /// and in which cell.
    drops_waiting: Vec<(u32, Vec3, u32)>,
    /// Time spent building so far.
    pub build_time: Duration,
}

impl NavGraph {
    /// An empty graph over the rectangle `min..=max` (world x, y) on a
    /// lattice of `spacing`, filled in chunk by chunk as paths need it.
    pub fn new(min: Vec2, max: Vec2, spacing: f32, cap: &Capsule) -> Self {
        NavGraph {
            spacing,
            capsule: *cap,
            gx: (
                (min.x / spacing).floor() as i32,
                (max.x / spacing).ceil() as i32,
            ),
            gy: (
                (min.y / spacing).floor() as i32,
                (max.y / spacing).ceil() as i32,
            ),
            nodes: Vec::new(),
            edges: Vec::new(),
            roots: Vec::new(),
            walk_roots: Vec::new(),
            columns: HashMap::new(),
            built: HashSet::new(),
            drops_waiting: Vec::new(),
            build_time: Duration::ZERO,
        }
    }

    /// Build every chunk now (tests and benchmarks; paths build lazily).
    pub fn build_all(&mut self, ground: &Ground) {
        let (cx0, cy0) = chunk_of(self.gx.0, self.gy.0);
        let (cx1, cy1) = chunk_of(self.gx.1, self.gy.1);
        for cx in cx0..=cx1 {
            for cy in cy0..=cy1 {
                self.ensure_chunk(ground, cx, cy);
            }
        }
    }

    /// Nodes and edges of one chunk, if not there yet.
    fn ensure_chunk(&mut self, ground: &Ground, cx: i32, cy: i32) {
        if !self.built.contains(&(cx, cy)) && self.covers_chunk((cx, cy)) {
            let built = self.build_chunk(ground, (cx, cy));
            self.apply(built);
        }
    }

    /// The chunk `c` lies inside the graph's range.
    fn covers_chunk(&self, c: (i32, i32)) -> bool {
        let (lo, hi) = (
            chunk_of(self.gx.0, self.gy.0),
            chunk_of(self.gx.1, self.gy.1),
        );
        (lo.0..=hi.0).contains(&c.0) && (lo.1..=hi.1).contains(&c.1)
    }

    /// What building chunk `c` adds, the graph only read: its nodes, their edges to each other and
    /// to built chunks (each pair tested once, by the second chunk built), and its drops.
    pub fn build_chunk(&self, ground: &Ground, c: (i32, i32)) -> ChunkBuilt {
        let started = Instant::now();
        let mut d = Draft {
            nav: self,
            chunk: c,
            nodes: Vec::new(),
            columns: HashMap::new(),
            edges: Vec::new(),
            out: HashMap::new(),
            waiting: self.drops_waiting.clone(),
        };
        if !self.built.contains(&c) && self.covers_chunk(c) {
            d.fill(ground);
        }
        ChunkBuilt {
            chunk: c,
            base: self.nodes.len(),
            nodes: d.nodes,
            edges: d.edges,
            waiting: d.waiting,
            took: started.elapsed(),
        }
    }

    /// Add a chunk [`build_chunk`](Self::build_chunk) made: appends only, so it is quick under a
    /// write lock. A chunk built meanwhile, or against another graph state, is dropped.
    pub fn apply(&mut self, b: ChunkBuilt) {
        if self.built.contains(&b.chunk)
            || !self.covers_chunk(b.chunk)
            || b.base != self.nodes.len()
        {
            return;
        }
        self.built.insert(b.chunk);
        for node in b.nodes {
            let id = self.nodes.len() as u32;
            self.columns.entry(node.column).or_default().push(id);
            self.nodes.push(node);
            self.edges.push(Vec::new());
            self.roots.push(id);
            self.walk_roots.push(id);
        }
        for (from, e) in b.edges {
            self.add_edge(from, e.to, e.kind);
        }
        self.drops_waiting = b.waiting;
        self.build_time += b.took;
    }

    /// Build the chunks covering the columns `gx0..=gx1` x `gy0..=gy1`.
    pub(crate) fn ensure_columns(
        &mut self,
        ground: &Ground,
        gx0: i32,
        gx1: i32,
        gy0: i32,
        gy1: i32,
    ) {
        let (cx0, cy0) = chunk_of(gx0, gy0);
        let (cx1, cy1) = chunk_of(gx1, gy1);
        for cx in cx0..=cx1 {
            for cy in cy0..=cy1 {
                self.ensure_chunk(ground, cx, cy);
            }
        }
    }

    /// Make sure every neighbour column of `node` lies in a built chunk,
    /// so its edge list is complete.
    pub(crate) fn ensure_neighbours(&mut self, ground: &Ground, node: u32) {
        let (gx, gy) = self.nodes[node as usize].column;
        let here = chunk_of(gx, gy);
        if chunk_of(gx - 1, gy - 1) == here && chunk_of(gx + 1, gy + 1) == here {
            return;
        }
        self.ensure_columns(ground, gx - 1, gx + 1, gy - 1, gy + 1);
    }

    /// Snap a sample point out of walls and onto its floor; `None` when
    /// no capsule stands there.
    fn place(
        &self,
        ground: &Ground,
        p: Vec3,
        column: (i32, i32),
        cap: &Capsule,
    ) -> Option<NavNode> {
        let q = ground
            .collision
            .resolve_above(p, cap.radius, cap.height, cap.step_up);
        if flat(q - p).length() > SNAP_FRACTION * self.spacing {
            return None;
        }
        let (z, cell) = ground.surface_at(Vec3::new(q.x, q.y, p.z), cap)?;
        let feet = Vec3::new(q.x, q.y, z);
        // Standing on interior geometry means standing in a cell. The
        // probe is a hand's breadth up: a point exactly on the floor
        // plane is on the boundary of the cell BSP, not inside it.
        if cell != 0
            && !ground
                .collision
                .inside_cell(feet + Vec3::new(0.0, 0.0, 0.1))
        {
            return None;
        }
        ground.fits(feet, cap).then_some(NavNode {
            pos: feet,
            cell,
            column,
        })
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// The rectangle the graph covers (world x, y).
    pub fn min(&self) -> Vec2 {
        Vec2::new(self.gx.0 as f32, self.gy.0 as f32) * self.spacing
    }

    pub fn max(&self) -> Vec2 {
        Vec2::new(self.gx.1 as f32, self.gy.1 as f32) * self.spacing
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.iter().map(Vec::len).sum()
    }

    /// Chunks built so far.
    pub fn chunk_count(&self) -> usize {
        self.built.len()
    }

    pub fn neighbours(&self, node: u32) -> &[NavEdge] {
        &self.edges[node as usize]
    }

    /// An edge from `from` to `to`, its ends joined in one component (of walks too, unless a drop).
    fn add_edge(&mut self, from: u32, to: u32, kind: EdgeKind) {
        self.edges[from as usize].push(NavEdge { to, kind });
        unite(&mut self.roots, from, to);
        if kind != EdgeKind::Drop {
            unite(&mut self.walk_roots, from, to);
        }
    }

    /// The component `node` is in: a root no edge of any kind leads out of, either way.
    pub fn component(&self, node: u32) -> u32 {
        root_of(&self.roots, node)
    }

    /// The component `node` is in over walks and doorways alone, for a walk that takes no drop.
    pub fn walk_component(&self, node: u32) -> u32 {
        root_of(&self.walk_roots, node)
    }

    /// Every chunk the graph covers is built, so its components are final.
    pub fn is_complete(&self) -> bool {
        let (lo, hi) = (
            chunk_of(self.gx.0, self.gy.0),
            chunk_of(self.gx.1, self.gy.1),
        );
        self.built.len() as i64 >= i64::from(hi.0 - lo.0 + 1) * i64::from(hi.1 - lo.1 + 1)
    }

    /// A chunk inside the graph's range not built yet, if any is left.
    pub fn next_unbuilt(&self) -> Option<(i32, i32)> {
        if self.is_complete() {
            return None;
        }
        self.unbuilt(self.gx.0, self.gx.1, self.gy.0, self.gy.1)
            .into_iter()
            .next()
    }

    /// The chunks covering columns `gx0..=gx1` x `gy0..=gy1` inside the graph's range that are
    /// not built yet.
    pub(crate) fn unbuilt(&self, gx0: i32, gx1: i32, gy0: i32, gy1: i32) -> Vec<(i32, i32)> {
        let (lo, hi) = (
            chunk_of(self.gx.0, self.gy.0),
            chunk_of(self.gx.1, self.gy.1),
        );
        let (cx0, cy0) = chunk_of(gx0.max(self.gx.0), gy0.max(self.gy.0));
        let (cx1, cy1) = chunk_of(gx1.min(self.gx.1), gy1.min(self.gy.1));
        let mut out = Vec::new();
        for cx in cx0.max(lo.0)..=cx1.min(hi.0) {
            for cy in cy0.max(lo.1)..=cy1.min(hi.1) {
                if !self.built.contains(&(cx, cy)) {
                    out.push((cx, cy));
                }
            }
        }
        out
    }

    /// The nodes sampled in lattice column `col`, built so far.
    pub(crate) fn column_ids(&self, col: (i32, i32)) -> &[u32] {
        self.columns.get(&col).map_or(&[][..], Vec::as_slice)
    }

    /// The lattice column holding a world point.
    pub(crate) fn column_of(&self, p: Vec3) -> (i32, i32) {
        (
            (p.x / self.spacing).round() as i32,
            (p.y / self.spacing).round() as i32,
        )
    }

    /// The nodes within `r` columns of `p` and [`MAX_EDGE_RISE`] of its height, nearest first
    /// (height counting triple), from the chunks built so far.
    pub(crate) fn near(&self, p: Vec3, r: i32) -> Vec<(f32, u32)> {
        near_in(
            self.column_of(p),
            p,
            r,
            |col| self.column_ids(col),
            |id| self.nodes[id as usize].pos,
        )
    }
}

/// [`NavGraph::near`] over any columns: `ids` of a column, `pos` of a node.
fn near_in<'c>(
    (gx, gy): (i32, i32),
    p: Vec3,
    r: i32,
    ids: impl Fn((i32, i32)) -> &'c [u32],
    pos: impl Fn(u32) -> Vec3,
) -> Vec<(f32, u32)> {
    let mut out = Vec::new();
    for dx in -r..=r {
        for dy in -r..=r {
            for &id in ids((gx + dx, gy + dy)) {
                let d = pos(id) - p;
                if d.z.abs() > MAX_EDGE_RISE {
                    continue;
                }
                out.push((flat(d).length_squared() + (3.0 * d.z).powi(2), id));
            }
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

/// What one chunk adds to a [`NavGraph`]: made by [`NavGraph::build_chunk`] while others read the
/// graph, added by [`NavGraph::apply`].
pub struct ChunkBuilt {
    chunk: (i32, i32),
    /// The graph's node count when built: the new nodes' ids start there.
    base: usize,
    nodes: Vec<NavNode>,
    edges: Vec<(u32, NavEdge)>,
    /// Every drop still waiting for its landing's chunk, this one's included.
    waiting: Vec<(u32, Vec3, u32)>,
    took: Duration,
}

/// A chunk being built over a graph only read: its new nodes numbered after the graph's, and the
/// edges added so far, in the order [`NavGraph::apply`] adds them.
struct Draft<'g> {
    nav: &'g NavGraph,
    chunk: (i32, i32),
    nodes: Vec<NavNode>,
    /// The chunk's own columns: every node in them is new.
    columns: HashMap<(i32, i32), Vec<u32>>,
    edges: Vec<(u32, NavEdge)>,
    /// The new edges by the node they leave.
    out: HashMap<u32, Vec<NavEdge>>,
    waiting: Vec<(u32, Vec3, u32)>,
}

impl Draft<'_> {
    fn node(&self, id: u32) -> NavNode {
        match (id as usize).checked_sub(self.nav.nodes.len()) {
            Some(i) => self.nodes[i],
            None => self.nav.nodes[id as usize],
        }
    }

    /// The nodes of column `col`: new in this chunk, else the graph's.
    fn ids(&self, col: (i32, i32)) -> &[u32] {
        if chunk_of(col.0, col.1) == self.chunk {
            self.columns.get(&col).map_or(&[][..], Vec::as_slice)
        } else {
            self.nav.column_ids(col)
        }
    }

    fn is_built(&self, c: (i32, i32)) -> bool {
        c == self.chunk || self.nav.built.contains(&c)
    }

    /// The edges out of `from`, the graph's and the new ones.
    fn edges_of(&self, from: u32) -> impl Iterator<Item = &NavEdge> {
        let old = self
            .nav
            .edges
            .get(from as usize)
            .map_or(&[][..], Vec::as_slice);
        old.iter().chain(self.out.get(&from).into_iter().flatten())
    }

    fn has_edge(&self, from: u32, to: u32) -> bool {
        self.edges_of(from).any(|e| e.to == to)
    }

    fn add_edge(&mut self, from: u32, to: u32, kind: EdgeKind) {
        let e = NavEdge { to, kind };
        self.edges.push((from, e));
        self.out.entry(from).or_default().push(e);
    }

    fn push_node(&mut self, node: NavNode) -> u32 {
        let id = (self.nav.nodes.len() + self.nodes.len()) as u32;
        self.columns.entry(node.column).or_default().push(id);
        self.nodes.push(node);
        id
    }

    fn near(&self, p: Vec3, r: i32) -> Vec<(f32, u32)> {
        near_in(
            self.nav.column_of(p),
            p,
            r,
            |col| self.ids(col),
            |id| self.node(id).pos,
        )
    }

    /// The chunk's nodes, their edges and drops, and the drops waiting on it.
    fn fill(&mut self, ground: &Ground) {
        let nav = self.nav;
        let (cx, cy) = self.chunk;
        let cap = nav.capsule;
        let mut fresh: Vec<u32> = Vec::new();
        for gx in (cx * CHUNK..(cx + 1) * CHUNK).filter(|x| (nav.gx.0..=nav.gx.1).contains(x)) {
            for gy in (cy * CHUNK..(cy + 1) * CHUNK).filter(|y| (nav.gy.0..=nav.gy.1).contains(y)) {
                let x = gx as f32 * nav.spacing;
                let y = gy as f32 * nav.spacing;
                let mut levels = ground.collision.floors_at_xy(x, y);
                if let Some(t) = ground.terrain_under(x, y) {
                    levels.push((t, 0));
                }
                levels.sort_by(|a, b| b.0.total_cmp(&a.0));
                let mut placed: Vec<f32> = Vec::new();
                for (z, _) in levels {
                    if placed.iter().any(|&pz| (pz - z).abs() < LEVEL_MERGE) {
                        continue;
                    }
                    let Some(node) = nav.place(ground, Vec3::new(x, y, z), (gx, gy), &cap) else {
                        continue;
                    };
                    if placed
                        .iter()
                        .any(|&pz| (pz - node.pos.z).abs() < LEVEL_MERGE)
                    {
                        continue;
                    }
                    placed.push(node.pos.z);
                    fresh.push(self.push_node(node));
                }
            }
        }
        // The openings the data names, and points along every ramp, wherever the lattice fell:
        // a ramp narrower than the lattice lies between its columns and joins nothing
        // (a_counter_on_a_deck_up_a_ramp_is_walked_up_to, in ac-client).
        let span = CHUNK as f32 * nav.spacing;
        let middle = Vec3::new((cx as f32 + 0.5) * span, (cy as f32 + 0.5) * span, 0.0);
        let ramps: Vec<Vec3> = ground
            .collision
            .nearby(middle, span * 0.75)
            .flat_map(|t| ramp_points(t, nav.spacing))
            .collect();
        let mut seeded: Vec<u32> = Vec::new();
        for d in ground.doorways.iter().chain(&ramps) {
            let gx = (d.x / nav.spacing).round() as i32;
            let gy = (d.y / nav.spacing).round() as i32;
            if chunk_of(gx, gy) != (cx, cy) {
                continue;
            }
            if !(nav.gx.0..=nav.gx.1).contains(&gx) || !(nav.gy.0..=nav.gy.1).contains(&gy) {
                continue;
            }
            let Some(node) = nav.place(ground, *d, (gx, gy), &cap) else {
                continue;
            };
            // One per opening per floor: a doorway sampled twice is two
            // nodes a hand's breadth apart and nothing gained.
            if self.ids((gx, gy)).iter().any(|&j| {
                let p = self.node(j).pos;
                (p.z - node.pos.z).abs() < LEVEL_MERGE
                    && flat(p - node.pos).length() < nav.spacing * 0.25
            }) {
                continue;
            }
            let id = self.push_node(node);
            fresh.push(id);
            seeded.push(id);
        }
        const DIRS: [(i32, i32); 8] = [
            (1, 0),
            (0, 1),
            (1, 1),
            (1, -1),
            (-1, 0),
            (0, -1),
            (-1, -1),
            (-1, 1),
        ];
        for &i in &fresh {
            let (gx, gy) = self.node(i).column;
            for (k, (dx, dy)) in DIRS.iter().enumerate() {
                let col = (gx + dx, gy + dy);
                let other = chunk_of(col.0, col.1);
                // Inside the chunk, the four forward directions cover
                // every pair; toward an older chunk, all eight.
                if other == (cx, cy) {
                    if k >= 4 {
                        continue;
                    }
                } else if !nav.built.contains(&other) {
                    continue;
                }
                for j in self.ids(col).to_vec() {
                    let (fwd, back) = ground.hop(&self.node(i), &self.node(j), &cap);
                    if let Some(kind) = fwd {
                        self.add_edge(i, j, kind);
                    }
                    if let Some(kind) = back {
                        self.add_edge(j, i, kind);
                    }
                }
            }
        }
        for &i in &seeded {
            let col = self.node(i).column;
            for j in self.ids(col).to_vec() {
                if j == i {
                    continue;
                }
                let (fwd, back) = ground.hop(&self.node(i), &self.node(j), &cap);
                if let Some(kind) = fwd.filter(|_| !self.has_edge(i, j)) {
                    self.add_edge(i, j, kind);
                }
                if let Some(kind) = back.filter(|_| !self.has_edge(j, i)) {
                    self.add_edge(j, i, kind);
                }
            }
        }
        for &i in &fresh {
            self.drops_from(ground, i, &cap);
        }
        let waiting = std::mem::take(&mut self.waiting);
        for (from, at, cell) in waiting {
            self.land(ground, from, at, cell, &cap);
        }
    }

    /// The drop edges out of `node`: toward each neighbouring column it has no edge into and whose
    /// floor falls away, a walk that goes over the edge, followed down to where it lands.
    fn drops_from(&mut self, ground: &Ground, node: u32, cap: &Capsule) {
        let a = self.node(node);
        let spacing = self.nav.spacing;
        for k in 0..8 {
            let angle = k as f32 * std::f32::consts::FRAC_PI_4;
            let (dx, dy) = (angle.cos().round() as i32, angle.sin().round() as i32);
            let col = (a.column.0 + dx, a.column.1 + dy);
            if self.edges_of(node).any(|e| self.node(e.to).column == col) {
                continue;
            }
            let toward = Vec3::new(col.0 as f32, col.1 as f32, 0.0) * spacing;
            let toward = Vec3::new(toward.x, toward.y, a.pos.z);
            // Only where the floor falls away: none at that column within a step of this one.
            let within_step = |z: f32| z <= a.pos.z + cap.step_up && z >= a.pos.z - cap.step_down;
            let level = ground
                .collision
                .floors_at_xy(toward.x, toward.y)
                .into_iter()
                .map(|f| f.0)
                .chain(ground.terrain_under(toward.x, toward.y))
                .any(within_step);
            if level {
                continue;
            }
            if let Some((land, cell)) = ground.drop_toward(a.pos, a.cell, toward, cap) {
                self.land(ground, node, land, cell, cap);
            }
        }
    }

    /// A drop from `from` landing at `at` (in `cell`) joined to the node on that floor within a
    /// column the body walks to from there, or kept until the chunk it lands in is built.
    fn land(&mut self, ground: &Ground, from: u32, at: Vec3, cell: u32, cap: &Capsule) {
        let nav = self.nav;
        let (gx, gy) = nav.column_of(at);
        if !self.is_built(chunk_of(gx, gy)) {
            if (nav.gx.0..=nav.gx.1).contains(&gx) && (nav.gy.0..=nav.gy.1).contains(&gy) {
                self.waiting.push((from, at, cell));
            }
            return;
        }
        let onto = self
            .near(at, 1)
            .into_iter()
            .map(|(_, id)| id)
            .filter(|&id| id != from && (self.node(id).pos.z - at.z).abs() <= cap.step_up)
            .take(JOIN_TRIES)
            .find(|&id| {
                let (a, b) = (self.node(from), self.node(id));
                ground.body_reaches(at, cell, b.pos, cap) && is_dropped_to(ground, a, b, cap)
            });
        if let Some(to) = onto {
            if !self.has_edge(from, to) {
                self.add_edge(from, to, EdgeKind::Drop);
            }
        }
    }
}

/// Whether the follower's walk of a drop, at node `b` from node `a` at [`DROP_SPEED`], goes over
/// the edge and lands where the body walks on to `b`: the walk the drop edge stands for.
fn is_dropped_to(ground: &Ground, a: NavNode, b: NavNode, cap: &Capsule) -> bool {
    ground
        .drop_toward(a.pos, a.cell, b.pos, cap)
        .is_some_and(|(land, cell)| ground.body_reaches(land, cell, b.pos, cap))
}

/// Points a metre apart down a sloping floor `t` (a ramp) narrower than the lattice `spacing`: a
/// line of them inside it along its longest side. None on a level, steep or wide one, which the
/// lattice already stands on (a roof).
fn ramp_points(t: &crate::collision::Tri, spacing: f32) -> Vec<Vec3> {
    if !(RAMP_FLOOR..RAMP_LEVEL).contains(&t.normal.z) {
        return Vec::new();
    }
    let sides = [(t.a, t.b, t.c), (t.b, t.c, t.a), (t.c, t.a, t.b)];
    let Some(&(p, q, r)) = sides
        .iter()
        .max_by(|x, y| x.0.distance(x.1).total_cmp(&y.0.distance(y.1)))
    else {
        return Vec::new();
    };
    let long = p.distance(q);
    let width = (q - p).cross(r - p).length() / long.max(1e-3);
    if width >= spacing {
        return Vec::new();
    }
    let n = (long / RAMP_STEP).ceil().max(1.0) as usize;
    (0..n)
        .map(|i| {
            let base = p + (q - p) * ((i as f32 + 0.5) / n as f32);
            base + (r - base) / 3.0
        })
        .collect()
}

fn chunk_of(gx: i32, gy: i32) -> (i32, i32) {
    (gx.div_euclid(CHUNK), gy.div_euclid(CHUNK))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A quad `a b c d`, counter-clockwise seen from its normal's side.
    pub(crate) fn quad(w: &mut CollisionWorld, a: Vec3, b: Vec3, c: Vec3, d: Vec3, cell: u32) {
        w.add_tri(a, b, c, cell, false);
        w.add_tri(a, c, d, cell, false);
    }

    pub(crate) fn floor(
        w: &mut CollisionWorld,
        x0: f32,
        x1: f32,
        y0: f32,
        y1: f32,
        z: f32,
        cell: u32,
    ) {
        quad(
            w,
            Vec3::new(x0, y0, z),
            Vec3::new(x1, y0, z),
            Vec3::new(x1, y1, z),
            Vec3::new(x0, y1, z),
            cell,
        );
    }

    /// A wall along the y axis at `x` from `y0` to `y1`, 3 m tall, facing -x.
    pub(crate) fn wall_x(w: &mut CollisionWorld, x: f32, y0: f32, y1: f32, cell: u32) {
        w.add_tri(
            Vec3::new(x, y0, 0.0),
            Vec3::new(x, y0, 3.0),
            Vec3::new(x, y1, 3.0),
            cell,
            true,
        );
        w.add_tri(
            Vec3::new(x, y0, 0.0),
            Vec3::new(x, y1, 3.0),
            Vec3::new(x, y1, 0.0),
            cell,
            true,
        );
    }

    /// Two 8 m rooms side by side along x, sharing the wall at x = 8 with
    /// a doorway `door` wide centred on y = 0 (or none).
    pub(crate) fn two_rooms(door: Option<f32>) -> CollisionWorld {
        let mut w = CollisionWorld::default();
        floor(&mut w, 0.0, 8.0, -4.0, 4.0, 0.0, 1);
        floor(&mut w, 8.0, 16.0, -4.0, 4.0, 0.0, 2);
        // Outer walls.
        wall_x(&mut w, 0.0, -4.0, 4.0, 1);
        wall_x(&mut w, 16.0, -4.0, 4.0, 2);
        for y in [-4.0, 4.0] {
            w.add_tri(
                Vec3::new(0.0, y, 0.0),
                Vec3::new(16.0, y, 0.0),
                Vec3::new(16.0, y, 3.0),
                1,
                true,
            );
            w.add_tri(
                Vec3::new(0.0, y, 0.0),
                Vec3::new(16.0, y, 3.0),
                Vec3::new(0.0, y, 3.0),
                1,
                true,
            );
        }
        match door {
            Some(d) => {
                wall_x(&mut w, 8.0, -4.0, -d / 2.0, 1);
                wall_x(&mut w, 8.0, d / 2.0, 4.0, 1);
            }
            None => wall_x(&mut w, 8.0, -4.0, 4.0, 1),
        }
        w
    }

    #[test]
    fn a_portal_berth_keeps_to_its_own_floor() {
        // The Facility Hub's Stronghold portal stands on the floor at z 0, over a corridor 12 m
        // below: its berth is on its own floor only.
        let mouth = Vec3::new(80.0, -24.7, -0.1);
        assert!(
            in_berth(mouth, Vec3::new(81.0, -24.0, 0.0), 3.0),
            "beside it"
        );
        assert!(
            !in_berth(mouth, Vec3::new(81.0, -24.0, -12.0), 3.0),
            "the corridor under it"
        );
        assert!(
            !in_berth(mouth, Vec3::new(81.0, -24.0, 6.0), 3.0),
            "the floor over it"
        );
        assert!(
            !in_berth(mouth, Vec3::new(84.0, -24.7, 0.0), 3.0),
            "past the berth"
        );
    }

    #[test]
    fn a_chunk_built_against_an_older_graph_is_dropped_not_added() {
        // Three chunks along x at the indoor lattice.
        let mut w = CollisionWorld::default();
        floor(&mut w, 0.0, 70.0, 0.0, 6.0, 0.0, 1);
        let ground = inside(&w);
        let cap = Capsule::default();
        let (lo, hi) = (Vec2::new(0.0, 0.0), Vec2::new(70.0, 6.0));
        let mut whole = NavGraph::new(lo, hi, INDOOR_SPACING, &cap);
        whole.build_all(&ground);
        let mut shared = NavGraph::new(lo, hi, INDOOR_SPACING, &cap);
        let first = shared.build_chunk(&ground, (0, 0));
        let stale = shared.build_chunk(&ground, (1, 0));
        shared.apply(first);
        shared.apply(stale);
        assert_eq!(shared.chunk_count(), 1, "built before the first was added");
        for c in [(1, 0), (2, 0)] {
            let built = shared.build_chunk(&ground, c);
            shared.apply(built);
        }
        assert_eq!(shared.chunk_count(), whole.chunk_count());
        assert_eq!(shared.len(), whole.len());
        assert_eq!(shared.edge_count(), whole.edge_count());
        assert!(shared.is_complete());
    }

    /// Inside, with nothing but `w` to stand on.
    pub(crate) fn inside(w: &CollisionWorld) -> Ground<'_> {
        Ground {
            collision: w,
            terrain: None,
            sea: None,
            doorways: &[],
        }
    }

    fn node(x: f32, y: f32, z: f32, cell: u32) -> NavNode {
        NavNode {
            pos: Vec3::new(x, y, z),
            cell,
            column: (0, 0),
        }
    }

    #[test]
    fn a_door_frame_the_body_slides_past_is_an_edge() {
        // A doorway a body's width and a hand wide, met at a slant: the sampled walk touches the
        // jamb and refuses, the body is slid off it and walks through.
        let w = two_rooms(Some(1.0));
        let ground = inside(&w);
        let cap = Capsule::default();
        let (a, b) = (node(7.0, 0.6, 0.0, 1), node(9.0, 0.0, 0.0, 2));
        assert!(
            !ground.walkable(a.pos, b.pos, &cap).0,
            "the sampler walks it"
        );
        assert_eq!(ground.hop(&a, &b, &cap).0, Some(EdgeKind::Doorway));
    }

    #[test]
    fn a_low_overhang_beside_the_way_is_walked_as_the_body_walks_it() {
        // A bracket at 1.2 m over the side of a corridor: the body is pushed out from under it.
        let mut w = CollisionWorld::default();
        floor(&mut w, 0.0, 8.0, -3.0, 3.0, 0.0, 1);
        quad(
            &mut w,
            Vec3::new(2.0, 0.3, 1.2),
            Vec3::new(2.0, 1.5, 1.2),
            Vec3::new(6.0, 1.5, 1.2),
            Vec3::new(6.0, 0.3, 1.2),
            1,
        );
        let ground = inside(&w);
        let cap = Capsule::default();
        let (a, b) = (node(1.0, 0.0, 0.0, 1), node(7.0, 0.0, 0.0, 1));
        assert!(
            !ground.walkable(a.pos, b.pos, &cap).0,
            "the sampler walks it"
        );
        assert_eq!(
            ground.hop(&a, &b, &cap),
            (Some(EdgeKind::Walk), Some(EdgeKind::Walk))
        );
    }

    #[test]
    fn the_fast_path_and_the_full_step_agree_on_open_ground() {
        // Open terrain with a wall and a ledge in it: every step on the hop's swept ground is the
        // step the full walk takes, near the wall and away from it.
        let mut w = CollisionWorld::default();
        wall_x(&mut w, 5.0, -1.0, 1.0, 0);
        floor(&mut w, 7.0, 9.0, -2.0, 2.0, 0.4, 0);
        let terrain = |x: f32, _y: f32| Some(x * 0.05);
        let ground = Ground {
            terrain: Some(&terrain),
            ..inside(&w)
        };
        let cap = Capsule::default();
        for (from, to) in [((0.0, -3.0), (10.0, -3.0)), ((0.0, 0.4), (10.0, 0.2))] {
            let (a, b) = (Vec3::new(from.0, from.1, 0.0), Vec3::new(to.0, to.1, 0.5));
            let swept = Swept::over(&ground, a, b, &cap);
            let (mut p, mut cell) = (a, 0);
            for _ in 0..50 {
                let d = flat(b - p);
                if d.length() < 0.05 {
                    break;
                }
                let step = d.normalize() * d.length().min(BODY_STEP);
                let to = Vec3::new(p.x + step.x, p.y + step.y, p.z);
                let fast = stride(&swept, p, cell, to, &cap);
                assert_eq!(fast, stride(&ground, p, cell, to, &cap), "at {p}");
                match fast {
                    Stride::Stand { at, cell: c } => (p, cell) = (at, c),
                    _ => break,
                }
            }
        }
    }

    /// An upper floor 3 m up from x = 0 to 4 (cell 0x0101) over, when `below` is some, a floor that
    /// far under it from x = 0 to 12 (cell 0x0102), in a world with no terrain.
    pub(crate) fn a_ledge_in_cells(below: Option<f32>) -> CollisionWorld {
        let mut w = CollisionWorld::default();
        floor(&mut w, 0.0, 4.0, -2.0, 2.0, 3.0, 0x0101);
        if let Some(depth) = below {
            floor(&mut w, 0.0, 12.0, -2.0, 2.0, 3.0 - depth, 0x0102);
        }
        w
    }

    /// The drop edges of the ledge's graph, as (from, to) heights.
    fn drops(w: &CollisionWorld) -> Vec<(NavNode, NavNode)> {
        let ground = inside(w);
        let mut g = NavGraph::new(
            Vec2::new(0.0, -2.0),
            Vec2::new(12.0, 2.0),
            1.0,
            &Capsule::default(),
        );
        g.build_all(&ground);
        let mut out = Vec::new();
        for i in 0..g.len() as u32 {
            for e in g.neighbours(i) {
                if e.kind == EdgeKind::Drop {
                    out.push((g.nodes[i as usize], g.nodes[e.to as usize]));
                }
            }
        }
        out
    }

    #[test]
    fn a_drop_is_joined_only_where_the_walk_at_its_far_node_lands() {
        // The follower walks a drop at its far node: a node straight under the rim is landed near
        // by a fall off it, but walking at it from the rim steps nowhere.
        let w = a_ledge_in_cells(Some(3.0));
        let ground = inside(&w);
        let cap = Capsule::default();
        let mut g = NavGraph::new(Vec2::new(0.0, -2.0), Vec2::new(12.0, 2.0), 1.0, &cap);
        g.build_all(&ground);
        let at = |x: f32, z: f32| {
            *g.nodes
                .iter()
                .find(|n| {
                    (n.pos.x - x).abs() < 0.3 && n.pos.y.abs() < 0.3 && (n.pos.z - z).abs() < 0.3
                })
                .unwrap_or_else(|| panic!("no node at x {x}, z {z}"))
        };
        let rim = at(4.0, 3.0);
        assert!(
            !is_dropped_to(&ground, rim, at(4.0, 0.0), &cap),
            "under the rim"
        );
        assert!(
            is_dropped_to(&ground, rim, at(6.0, 0.0), &cap),
            "past the edge"
        );
        for (a, b) in drops(&w) {
            assert!(is_dropped_to(&ground, a, b, &cap), "{a:?} -> {b:?}");
        }
    }

    #[test]
    fn a_ledge_over_a_floor_is_a_drop_one_way() {
        let w = a_ledge_in_cells(Some(3.0));
        let found = drops(&w);
        assert!(!found.is_empty(), "no drop off the ledge");
        for (a, b) in &found {
            assert!(
                (a.pos.z - 3.0).abs() < 0.1 && b.pos.z.abs() < 0.1,
                "{a:?} -> {b:?}"
            );
            assert_eq!(b.cell, 0x0102);
        }
        // Nothing climbs back: no edge of any kind from the floor up to the ledge.
        let ground = inside(&w);
        let mut g = NavGraph::new(
            Vec2::new(0.0, -2.0),
            Vec2::new(12.0, 2.0),
            1.0,
            &Capsule::default(),
        );
        g.build_all(&ground);
        for i in 0..g.len() as u32 {
            for e in g.neighbours(i) {
                let (a, b) = (g.nodes[i as usize], g.nodes[e.to as usize]);
                assert!(
                    !(a.cell == 0x0102 && b.cell == 0x0101),
                    "{a:?} -> {b:?} climbs"
                );
            }
        }
    }

    #[test]
    fn a_ledge_over_nothing_is_no_edge() {
        assert!(drops(&a_ledge_in_cells(None)).is_empty());
    }

    #[test]
    fn a_drop_deeper_than_falling_hurts_is_no_edge() {
        assert!(drops(&a_ledge_in_cells(Some(15.0))).is_empty());
        assert!(
            !drops(&a_ledge_in_cells(Some(10.0))).is_empty(),
            "10 m is a drop"
        );
    }
}
