//! The walking graph of a rectangle (a block, an area): feet positions on a lattice over the
//! static collision and the terrain, one node per level in a column, snapped out of walls. Edges
//! are directed and checked by sampling the capsule every half metre (floor, walls, head room,
//! chest ray), indoors by the body's own step too (`Ground::joins`). Chunks build on demand;
//! routes are searched in [`crate::wayfind`].

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use glam::{Vec2, Vec3};

use crate::collision::{Capsule, CollisionWorld};
use crate::landblock::LandblockScene;
use crate::{lbid, BLOCK_SIZE};

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
/// How far one step of `body_reaches` goes (metres): a frame's run at 20 Hz.
const BODY_STEP: f32 = 0.2;
/// How near `body_reaches` must come to the far node, flat (metres).
const BODY_ARRIVE: f32 = 0.25;
/// The least share of a step `body_reaches` must close on the far node; less is a wall.
const BODY_CLOSING: f32 = 0.3;

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
    /// Where nothing may stand at all, floor or no floor: the berth
    /// around a portal that must not be touched, on its own floor ([`in_berth`]).
    pub no_go: Option<&'a dyn Fn(Vec3) -> bool>,
    /// Keep out of buildings: interior floors are not walkable. For a
    /// walk from one outdoor spot to another, which should go over the
    /// hill rather than through the halls beneath it.
    pub outdoors_only: bool,
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

impl Ground<'_> {
    /// The surface a capsule with its feet at `p` stands on: the highest
    /// interior floor within step range, else the higher of an outdoor
    /// floor and the terrain (the rule `Player::update` walks by).
    pub fn surface_at(&self, p: Vec3, cap: &Capsule) -> Option<(f32, u32)> {
        if self.no_go.is_some_and(|f| f(p)) {
            return None;
        }
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
                if self.outdoors_only {
                    // Not ours to stand on: only what is outside counts.
                } else if !buried {
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

    /// Sample the walk from `a` to `b` (feet positions) every
    /// `SUBSTEP`: the capsule must fit at every sample, the floor must
    /// continue, and the chest ray between samples must be clear. Returns
    /// whether the walk is possible from `a` to `b` and from `b` to `a`
    /// (they differ by the step-up and step-down limits).
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

    /// [`walkable`](Self::walkable), and where it refuses between two nodes `indoors`, the body's
    /// own step each way ([`body_reaches`](Self::body_reaches)): the test for a hop between
    /// neighbours. Only indoors: the step knows no terrain, and tried on every refused hop outside
    /// it made Holtburg's graph five times slower to build (92 -> 445 ms).
    pub fn joins(&self, a: Vec3, b: Vec3, indoors: bool, cap: &Capsule) -> (bool, bool) {
        let (fwd, back) = self.walkable(a, b, cap);
        // A wall at chest height between them is not slid past: the step is for what the sampling
        // over-counts, a door frame brushed or a landing's lip, never for a wall.
        if !indoors || (fwd && back) || !line_clear(self.collision, a, b) {
            return (fwd, back);
        }
        (
            fwd || self.body_reaches(a, b, cap),
            back || self.body_reaches(b, a, cap),
        )
    }

    /// Whether a route's leg from `a` to `b` (feet) is walked: the chest line clear and the edge
    /// test one way ([`joins`](Self::joins)), `indoors` when both ends stand in interior cells.
    pub fn can_walk(&self, a: Vec3, b: Vec3, indoors: bool, cap: &Capsule) -> bool {
        line_clear(self.collision, a, b)
            && (self.walkable(a, b, cap).0 || (indoors && self.body_reaches(a, b, cap)))
    }

    /// Whether the body gets from `a` to `b` (feet, a hop of a node or two) by the step
    /// `Player::update` takes: `walk` a fifth of a metre at a time, onto what is below within a
    /// step down where a step leaves the floor, always closing on `b`. The sampled test refuses a
    /// door frame the body slides past and the lip of a landing over a stair's top step, and so
    /// found no way into a mine at ACB5 the body walks (test: a_mine_is_walked_down_to_its_floor).
    pub fn body_reaches(&self, a: Vec3, b: Vec3, cap: &Capsule) -> bool {
        self.body_comes_within(a, b, BODY_ARRIVE, cap)
    }

    /// [`body_reaches`](Self::body_reaches), but `within` metres of `b` (flat, on its floor) is
    /// there: a walk that stops short, as a corridor's aim by its wall is walked to.
    pub fn body_comes_within(&self, a: Vec3, b: Vec3, within: f32, cap: &Capsule) -> bool {
        self.body_stops_within(a, b, within, cap).is_some()
    }

    /// Where the walk of [`body_comes_within`](Self::body_comes_within) stops, `within` of `b` on
    /// its floor; `None` when it does not get there.
    pub fn body_stops_within(&self, a: Vec3, b: Vec3, within: f32, cap: &Capsule) -> Option<Vec3> {
        let mut p = a;
        let mut left = flat(b - a).length();
        for _ in 0..(left / BODY_STEP).ceil() as usize + 4 {
            if left <= within {
                // A step from `b`'s floor is on it: down a stair, the ramp walked lies 0.33 m under the
                // tread its node stands on (test: the_stairs_up_to_a_scrivener_are_walked_down_too).
                return ((p.z - b.z).abs() <= cap.step_up).then_some(p);
            }
            let d = flat(b - p);
            let step = d.normalize() * d.length().min(BODY_STEP);
            let walk = self
                .collision
                .walk(p, Vec3::new(p.x + step.x, p.y + step.y, p.z), cap);
            if walk.blocked {
                return None;
            }
            let mut next = walk.pos;
            if walk.floor.is_none() {
                match self.collision.floor_at(next, 0.0, cap.step_down) {
                    Some((z, _)) => next.z = z,
                    None => return None,
                }
            }
            if self.outdoors_only && walk.floor.is_some_and(|(_, cell)| cell != 0) {
                return None;
            }
            if self.no_go.is_some_and(|f| f(next)) || self.sea.is_some_and(|f| f(next.x, next.y)) {
                return None;
            }
            // A step that closes little is sliding along a wall the hop runs into.
            let now_left = flat(b - next).length();
            if now_left > left - BODY_STEP * BODY_CLOSING {
                return None;
            }
            left = now_left;
            p = next;
        }
        None
    }

    /// Whether the straight walk from `a` to `b` (feet positions) runs
    /// off an edge with nothing under it before anything solid stops it,
    /// or keeps a floor all the way and ends under (or over) a `b` that
    /// stands a storey away, at the foot of an edge it never crosses.
    ///
    /// The walk is sampled every `SUBSTEP` as in
    /// [`walkable`](Self::walkable), and followed the way the walking
    /// code would follow it. Something solid first -- the capsule does
    /// not fit, the chest ray is blocked, or the ground rises over the
    /// feet faster than a step climbs -- and the walk ends there: a
    /// character leaning on it stays on its own side, and what lies
    /// beyond does not matter. No floor within a step, and the walk goes
    /// over the edge: down onto the highest floor within `depth` under
    /// it, as `Player::update` falls to one, and on from there. Only an
    /// edge with nothing under it within `depth` is a drop.
    ///
    /// A ledge over a floor is not refused. Stepping off is how a
    /// character upstairs in a house comes down when the graph finds no
    /// way, and how one stranded on a ledge in a dungeon gets back to
    /// the room it looks over; refusing it left both standing there. The
    /// edge of everything is another matter: past it the walking code
    /// has nothing to come down on and carries the character on through
    /// the void at the height it left.
    pub fn drops_along(&self, a: Vec3, b: Vec3, cap: &Capsule, depth: f32) -> bool {
        let d = b - a;
        let len = flat(d).length();
        let steps = (len / SUBSTEP).ceil().max(1.0) as usize;
        let mut prev = a;
        for i in 1..=steps {
            let t = i as f32 / steps as f32;
            let at = Vec3::new(a.x + d.x * t, a.y + d.y * t, prev.z);
            // The wall before the floor. Under a wall there is often no
            // floor at all, only the sealed gap between two rooms, and
            // nobody walking into the wall ever gets there to fall.
            if !self.fits(at, cap) || !line_clear(self.collision, prev, at) {
                return false;
            }
            // Where nothing may stand is no edge: a walk leaning on it
            // goes round, not down.
            if self.no_go.is_some_and(|f| f(at)) {
                continue;
            }
            let z = match self.surface_at(at, cap) {
                Some((z, _)) => z,
                // Ground over the feet: a slope steeper than a step, a
                // ledge too tall. Solid, like a wall -- and the storey
                // that is likely under it is no floor of this walk's.
                None if self.rises_over(at, cap) => return false,
                None => match self.landing_under(at, cap, depth) {
                    Some(z) => z,
                    None => return true,
                },
            };
            prev = Vec3::new(at.x, at.y, z);
        }
        // Along a floor all the way, but whose? Something standing on
        // the top of a vault rib is reached, on the flat, by walking in
        // underneath it: no edge is crossed on the way, and it is eight
        // metres up all the same. Only a `b` with a floor of its own
        // under it is judged so. One with none is not standing anywhere
        // -- a point off the overland grid, floating over a hillside --
        // and the walk to the ground under it is the walk wanted.
        match self.surface_at(b, cap) {
            Some((z, _)) => z - prev.z > cap.step_up || prev.z - z > cap.step_down,
            None => false,
        }
    }

    /// Ground over the feet at `at`: a floor, or outdoors the terrain,
    /// higher than a step and (for a floor) lower than the head.
    fn rises_over(&self, at: Vec3, cap: &Capsule) -> bool {
        let over = at.z + cap.step_up;
        self.collision
            .floor_at(at, cap.height, 0.0)
            .is_some_and(|(z, _)| z > over)
            || self.terrain_under(at.x, at.y).is_some_and(|t| t > over)
    }

    /// Where a character going over an edge at `at` comes down: the
    /// highest floor, or outdoors the ground, from a step over its feet
    /// to `depth` under them. `None` when there is nothing.
    fn landing_under(&self, at: Vec3, cap: &Capsule, depth: f32) -> Option<f32> {
        let floor = self
            .collision
            .floor_at(at, cap.step_up, depth)
            .map(|(z, _)| z);
        let ground = self
            .terrain_under(at.x, at.y)
            .filter(|&t| t <= at.z + cap.step_up && t >= at.z - depth);
        match (floor, ground) {
            (Some(f), Some(g)) => Some(f.max(g)),
            (f, g) => f.or(g),
        }
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
    /// Nodes per grid column `(x / spacing, y / spacing)`.
    columns: HashMap<(i32, i32), Vec<u32>>,
    /// Chunks whose nodes and edges exist.
    built: HashSet<(i32, i32)>,
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
            columns: HashMap::new(),
            built: HashSet::new(),
            build_time: Duration::ZERO,
        }
    }

    /// The graph of an assembled landblock: the fine lattice in dungeons
    /// and blocks with buildings, the coarse one over open terrain. The
    /// `Ground` passed to the other methods must describe the same block
    /// (its collision world and, unless it is a dungeon, its terrain).
    pub fn for_scene(scene: &LandblockScene, collision: &CollisionWorld, cap: &Capsule) -> Self {
        let origin = lbid::world_origin(scene.id);
        let square = (
            Vec2::new(origin.x, origin.y),
            Vec2::new(origin.x + BLOCK_SIZE, origin.y + BLOCK_SIZE),
        );
        let hull = collision
            .bounds()
            .map(|(lo, hi)| (flat(lo), flat(hi)))
            .unwrap_or(square);
        if scene.is_dungeon {
            NavGraph::new(hull.0, hull.1, INDOOR_SPACING, cap)
        } else {
            let margin = Vec2::splat(2.0 * OUTDOOR_SPACING);
            let min = square.0.min(hull.0).max(square.0 - margin);
            let max = square.1.max(hull.1).min(square.1 + margin);
            let spacing = if scene.cells.is_empty() {
                OUTDOOR_SPACING
            } else {
                INDOOR_SPACING
            };
            NavGraph::new(min, max, spacing, cap)
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

    /// Nodes and edges of one chunk, if not there yet. Edges to nodes in
    /// neighbouring chunks are added when the second of the two chunks is
    /// built, so each pair is tested once.
    fn ensure_chunk(&mut self, ground: &Ground, cx: i32, cy: i32) {
        if self.built.contains(&(cx, cy)) {
            return;
        }
        let (cx0, cy0) = chunk_of(self.gx.0, self.gy.0);
        let (cx1, cy1) = chunk_of(self.gx.1, self.gy.1);
        if cx < cx0 || cx > cx1 || cy < cy0 || cy > cy1 {
            return;
        }
        let started = Instant::now();
        self.built.insert((cx, cy));
        let cap = self.capsule;
        let mut fresh: Vec<u32> = Vec::new();
        for gx in (cx * CHUNK..(cx + 1) * CHUNK).filter(|x| (self.gx.0..=self.gx.1).contains(x)) {
            for gy in (cy * CHUNK..(cy + 1) * CHUNK).filter(|y| (self.gy.0..=self.gy.1).contains(y))
            {
                let x = gx as f32 * self.spacing;
                let y = gy as f32 * self.spacing;
                let mut levels = ground.collision.floors_at_xy(x, y);
                if ground.outdoors_only {
                    levels.retain(|(_, cell)| *cell == 0);
                }
                if let Some(t) = ground.terrain_under(x, y) {
                    levels.push((t, 0));
                }
                levels.sort_by(|a, b| b.0.total_cmp(&a.0));
                let mut placed: Vec<f32> = Vec::new();
                for (z, _) in levels {
                    if placed.iter().any(|&pz| (pz - z).abs() < LEVEL_MERGE)
                        || ground.no_go.is_some_and(|f| f(Vec3::new(x, y, z)))
                    {
                        continue;
                    }
                    let Some(node) = self.place(ground, Vec3::new(x, y, z), (gx, gy), &cap) else {
                        continue;
                    };
                    if placed
                        .iter()
                        .any(|&pz| (pz - node.pos.z).abs() < LEVEL_MERGE)
                    {
                        continue;
                    }
                    placed.push(node.pos.z);
                    let id = self.nodes.len() as u32;
                    self.nodes.push(node);
                    self.edges.push(Vec::new());
                    self.columns.entry((gx, gy)).or_default().push(id);
                    fresh.push(id);
                }
            }
        }
        // The openings the data names, and points along every ramp, wherever the lattice fell:
        // a ramp narrower than the lattice lies between its columns and joins nothing
        // (a_counter_on_a_deck_up_a_ramp_is_walked_up_to, in ac-client).
        let span = CHUNK as f32 * self.spacing;
        let middle = Vec3::new((cx as f32 + 0.5) * span, (cy as f32 + 0.5) * span, 0.0);
        let ramps: Vec<Vec3> = ground
            .collision
            .nearby(middle, span * 0.75)
            .flat_map(|t| ramp_points(t, self.spacing))
            .collect();
        let mut seeded: Vec<u32> = Vec::new();
        for d in ground.doorways.iter().chain(&ramps) {
            let gx = (d.x / self.spacing).round() as i32;
            let gy = (d.y / self.spacing).round() as i32;
            if chunk_of(gx, gy) != (cx, cy) {
                continue;
            }
            if !(self.gx.0..=self.gx.1).contains(&gx) || !(self.gy.0..=self.gy.1).contains(&gy) {
                continue;
            }
            if ground.no_go.is_some_and(|f| f(*d)) {
                continue;
            }
            let Some(node) = self.place(ground, *d, (gx, gy), &cap) else {
                continue;
            };
            // One per opening per floor: a doorway sampled twice is two
            // nodes a hand's breadth apart and nothing gained.
            if self.columns.get(&(gx, gy)).is_some_and(|ids| {
                ids.iter().any(|&j| {
                    let p = self.nodes[j as usize].pos;
                    (p.z - node.pos.z).abs() < LEVEL_MERGE
                        && flat(p - node.pos).length() < self.spacing * 0.25
                })
            }) {
                continue;
            }
            let id = self.nodes.len() as u32;
            self.nodes.push(node);
            self.edges.push(Vec::new());
            self.columns.entry((gx, gy)).or_default().push(id);
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
            let a = self.nodes[i as usize].pos;
            let (gx, gy) = self.nodes[i as usize].column;
            for (k, (dx, dy)) in DIRS.iter().enumerate() {
                let col = (gx + dx, gy + dy);
                let other = chunk_of(col.0, col.1);
                // Inside the chunk, the four forward directions cover
                // every pair; toward an older chunk, all eight.
                if other == (cx, cy) {
                    if k >= 4 {
                        continue;
                    }
                } else if !self.built.contains(&other) {
                    continue;
                }
                let Some(others) = self.columns.get(&col) else {
                    continue;
                };
                for j in others.clone() {
                    let b = self.nodes[j as usize].pos;
                    if (a.z - b.z).abs() > MAX_EDGE_RISE {
                        continue;
                    }
                    let indoors =
                        self.nodes[i as usize].cell != 0 && self.nodes[j as usize].cell != 0;
                    let (fwd, back) = ground.joins(a, b, indoors, &cap);
                    let (ci, cj) = (self.nodes[i as usize].cell, self.nodes[j as usize].cell);
                    if fwd {
                        self.edges[i as usize].push(NavEdge {
                            to: j,
                            kind: walk_kind(ci, cj),
                        });
                    }
                    if back {
                        self.edges[j as usize].push(NavEdge {
                            to: i,
                            kind: walk_kind(cj, ci),
                        });
                    }
                }
            }
        }
        for &i in &seeded {
            let a = self.nodes[i as usize].pos;
            let col = self.nodes[i as usize].column;
            let Some(others) = self.columns.get(&col).cloned() else {
                continue;
            };
            for j in others {
                if j == i {
                    continue;
                }
                let b = self.nodes[j as usize].pos;
                if (a.z - b.z).abs() > MAX_EDGE_RISE {
                    continue;
                }
                let indoors = self.nodes[i as usize].cell != 0 && self.nodes[j as usize].cell != 0;
                let (fwd, back) = ground.joins(a, b, indoors, &cap);
                let (ci, cj) = (self.nodes[i as usize].cell, self.nodes[j as usize].cell);
                if fwd && !self.edges[i as usize].iter().any(|e| e.to == j) {
                    self.edges[i as usize].push(NavEdge {
                        to: j,
                        kind: walk_kind(ci, cj),
                    });
                }
                if back && !self.edges[j as usize].iter().any(|e| e.to == i) {
                    self.edges[j as usize].push(NavEdge {
                        to: i,
                        kind: walk_kind(cj, ci),
                    });
                }
            }
        }
        self.build_time += started.elapsed();
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

    /// The lattice column holding a world point.
    pub(crate) fn column_of(&self, p: Vec3) -> (i32, i32) {
        (
            (p.x / self.spacing).round() as i32,
            (p.y / self.spacing).round() as i32,
        )
    }

    /// Where a character can stand within `radius` (metres, flat) of `p`, building what it looks
    /// at: the choice for a caller picking where to stand near something, as a caster does to see
    /// onto a roof.
    pub fn standable_near(&mut self, ground: &Ground, p: Vec3, radius: f32) -> Vec<Vec3> {
        let r = (radius / self.spacing).ceil() as i32;
        let gx = (p.x / self.spacing).round() as i32;
        let gy = (p.y / self.spacing).round() as i32;
        self.ensure_columns(ground, gx - r, gx + r, gy - r, gy + r);
        let mut out = Vec::new();
        for dx in -r..=r {
            for dy in -r..=r {
                for &id in self.columns.get(&(gx + dx, gy + dy)).into_iter().flatten() {
                    let q = self.nodes[id as usize].pos;
                    if flat(q - p).length() <= radius {
                        out.push(q);
                    }
                }
            }
        }
        out
    }

    /// The nodes within `r` columns of `p` and [`MAX_EDGE_RISE`] of its height, nearest first
    /// (height counting triple), from the chunks built so far.
    pub(crate) fn near(&self, p: Vec3, r: i32) -> Vec<(f32, u32)> {
        let (gx, gy) = self.column_of(p);
        let mut out = Vec::new();
        for dx in -r..=r {
            for dy in -r..=r {
                let Some(ids) = self.columns.get(&(gx + dx, gy + dy)) else {
                    continue;
                };
                for &id in ids {
                    let d = self.nodes[id as usize].pos - p;
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

    /// Inside, with nothing but `w` to stand on.
    pub(crate) fn inside(w: &CollisionWorld) -> Ground<'_> {
        Ground {
            collision: w,
            terrain: None,
            sea: None,
            no_go: None,
            outdoors_only: false,
            doorways: &[],
        }
    }

    /// An upper floor three metres up from x = 0 to 4, and under it and
    /// on past its edge, when `below`, the floor of the room.
    pub(crate) fn a_ledge(below: bool) -> CollisionWorld {
        let mut w = CollisionWorld::default();
        floor(&mut w, 0.0, 4.0, -2.0, 2.0, 3.0, 1);
        if below {
            floor(&mut w, 0.0, 12.0, -2.0, 2.0, 0.0, 2);
        }
        w
    }

    /// The deepest a fall is looked for, as the walking code looks.
    const FALL: f32 = 200.0;

    #[test]
    fn a_walk_off_a_ledge_onto_the_floor_below_is_no_drop() {
        let w = a_ledge(true);
        let ground = inside(&w);
        let cap = Capsule::default();
        let (from, to) = (Vec3::new(1.0, 0.0, 3.0), Vec3::new(9.0, 0.0, 0.0));
        assert!(
            !ground.walkable(from, to, &cap).0,
            "a three-metre drop walked"
        );
        assert!(!ground.drops_along(from, to, &cap, FALL));
    }

    #[test]
    fn a_walk_off_an_edge_with_nothing_under_it_is_a_drop() {
        let cap = Capsule::default();
        let (from, to) = (Vec3::new(1.0, 0.0, 3.0), Vec3::new(9.0, 0.0, 3.0));
        let w = a_ledge(false);
        assert!(inside(&w).drops_along(from, to, &cap, FALL));
        // A floor further down than a fall is looked for is no floor to
        // come down on.
        let mut deep = a_ledge(false);
        floor(&mut deep, 0.0, 12.0, -2.0, 2.0, -250.0, 2);
        assert!(inside(&deep).drops_along(from, to, &cap, FALL));
    }

    #[test]
    fn a_ledge_too_tall_for_a_step_is_solid_not_an_edge_over_the_storey_below() {
        // A floor to x = 2, then a platform a metre up with no face to
        // its edge -- only its top, which the walls never count -- and
        // under it all, the storey twelve metres down.
        let mut w = CollisionWorld::default();
        floor(&mut w, 0.0, 2.0, -2.0, 2.0, 0.0, 1);
        floor(&mut w, 2.0, 6.0, -2.0, 2.0, 1.0, 1);
        floor(&mut w, 0.0, 12.0, -2.0, 2.0, -12.0, 2);
        let ground = inside(&w);
        let cap = Capsule::default();
        // Nothing holds the capsule back where the platform is over its
        // feet: only the height does.
        assert!(ground.fits_here(Vec3::new(2.5, 0.0, 0.0), &cap));
        assert!(!ground.drops_along(
            Vec3::new(0.5, 0.0, 0.0),
            Vec3::new(5.0, 0.0, 1.0),
            &cap,
            FALL
        ));
    }

    #[test]
    fn a_hillside_too_steep_to_climb_is_not_an_edge() {
        let w = CollisionWorld::default();
        let terrain = |x: f32, _y: f32| Some(if x < 10.0 { 0.0 } else { (x - 10.0) * 2.0 });
        let ground = Ground {
            collision: &w,
            terrain: Some(&terrain),
            sea: None,
            no_go: None,
            outdoors_only: false,
            doorways: &[],
        };
        let cap = Capsule::default();
        assert!(!ground.drops_along(
            Vec3::new(8.0, 0.0, 0.0),
            Vec3::new(14.0, 0.0, 8.0),
            &cap,
            FALL
        ));
    }
}
