//! What stands in the room, and the way round it.
//!
//! The steering routes on the landblock's own geometry: walls, floors,
//! the statues and braziers the DAT files place, the trees the terrain
//! grows. What the server places is another matter. A chest, a hook, a
//! bush that can be picked, a cart in the road -- these arrive as
//! objects, with the packets, and leave the same way, and the
//! collision world and the graph over it (shared by every character in
//! the process, built once a block) never hear of them. A walk that was
//! planned clear ran into them, and the only thing that noticed was the
//! stuck clock, which skips a waypoint rather than go round.
//!
//! So they are carried apart from the ground, as the shapes the retail
//! client collided with them by: the `CylSphere`s of the object's
//! Setup, vertical cylinders in the object's frame, and failing those
//! its `Sphere`s. [`Cluttered`] lays them over a [`Ground`] for one
//! steer: the straight line is blocked where it meets one, and a route
//! is threaded round each one it crosses. The list is rebuilt from the
//! object table every tick rather than cached: a few hundred objects at
//! most, a distance test each, and nothing to invalidate when one is
//! moved, rescaled or deleted between frames.
//!
//! What is left out is as important as what goes in, and every case is
//! the retail rule or the project's own: a creature (the client walked
//! through them), anything carried (it is in a pack, not on the floor),
//! anything Ethereal (an open door, a portal, a corpse), a missile, and
//! a door whatever its state, because a character here walks straight
//! through doors and the navigation must never route round one.

use ac_formats::setup::Setup;
use ac_scene::collision::Capsule;
use ac_world::{item_type, object, object_desc_flags, World, WorldObject};
use glam::{Vec2, Vec3};

use crate::steering::Ground;

/// A vertical cylinder standing in the world: an object's collision
/// shape as the retail client kept it, placed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cylinder {
    /// Where its axis stands, world x and y.
    pub center: Vec2,
    pub radius: f32,
    /// World height of its base and its top.
    pub bottom: f32,
    pub top: f32,
}

/// Objects further off than this are not gathered. The server only
/// describes what is in sight, and a walk plans a landblock at most.
pub const NEARBY: f32 = 64.0;
/// Room a detour keeps between the character's side and the object
/// (metres), on top of both radii: the walking code slides along what
/// it touches, and a corner cut to the millimetre is a touch.
const BERTH: f32 = 0.3;
/// How many objects a single leg may be threaded round, one after
/// another, before it is left as it was for the stuck clocks.
const DEPTH: u32 = 3;

/// Whether the retail client would have stopped a character walking
/// into `o`, and this one should walk round it.
pub fn in_the_way(o: &WorldObject) -> bool {
    // In a pack, on a belt, in a chest: not on the floor.
    let carried =
        o.position.is_none() || o.parent.is_some() || o.container.is_some() || o.wielder.is_some();
    // An open door, a portal, a corpse: solid to look at, not to walk.
    let ethereal = o.physics_state & object::PHYSICS_STATE_ETHEREAL != 0;
    let flying = o.physics_state & object::PHYSICS_STATE_MISSILE != 0;
    // The client never collided the viewer with a creature, and a
    // vendor or a monster is what most walks are to.
    let creature = o.is_player
        || o.item_type & item_type::CREATURE != 0
        || o.object_desc_flags & object_desc_flags::PLAYER != 0;
    // A door is walked straight through, open or shut: the rule holds
    // whatever the server says its physics are, because a route that
    // goes round one strands the character at the doorway.
    let door = o.object_desc_flags & object_desc_flags::DOOR != 0;
    !(carried || ethereal || flying || creature || door)
}

/// The cylinders `o` stands as, in world space, from its Setup: its
/// `CylSphere`s, and when it has none its `Sphere`s as cylinders of the
/// same reach. Empty for anything [`in_the_way`] says to walk through,
/// and for a Setup with neither.
///
/// Placed the way the retail client placed them: the shape's origin
/// scaled, turned by the object's own frame and set at its position,
/// the axis left upright. A `CylSphere`'s origin is its base and its
/// height goes up from there; a `Sphere` reaches its radius either way.
pub fn cylinders(o: &WorldObject, setup: &Setup) -> Vec<Cylinder> {
    if !in_the_way(o) {
        return Vec::new();
    }
    let Some(p) = o.position else {
        return Vec::new();
    };
    let at = ac_world::landblock_origin(p.cell) + p.local;
    let place = |origin: Vec3| at + p.rotation * (origin * o.scale);
    if !setup.cyl_spheres.is_empty() {
        setup
            .cyl_spheres
            .iter()
            .map(|c| {
                let low = place(c.origin);
                Cylinder {
                    center: low.truncate(),
                    radius: c.radius * o.scale,
                    bottom: low.z,
                    top: low.z + c.height * o.scale,
                }
            })
            .collect()
    } else {
        setup
            .spheres
            .iter()
            .map(|s| {
                let c = place(s.origin);
                let r = s.radius * o.scale;
                Cylinder {
                    center: c.truncate(),
                    radius: r,
                    bottom: c.z - r,
                    top: c.z + r,
                }
            })
            .collect()
    }
}

/// Everything within [`NEARBY`] of `at` that a walk must go round,
/// gathered afresh from the object table.
pub fn around(world: &World, assets: &ac_scene::Assets, at: Vec3) -> Vec<Cylinder> {
    world
        .objects
        .values()
        .filter(|o| in_the_way(o))
        .filter(|o| o.world_pos().is_some_and(|p| p.distance(at) < NEARBY))
        .flat_map(|o| {
            assets
                .setup(o.setup_id)
                .map(|s| cylinders(o, &s))
                .unwrap_or_default()
        })
        .collect()
}

impl Cylinder {
    /// How far the character must keep its axis from this one to pass.
    fn reach(&self, cap: &Capsule) -> f32 {
        self.radius + cap.radius
    }

    /// Whether a capsule walking the straight line from `from` to `to`
    /// (feet positions) meets this cylinder. Too low to matter is
    /// stepped onto, as the walking code steps onto any ledge within
    /// `step_up`; too high is walked under.
    ///
    /// A walk that begins inside the reach -- the character is already
    /// leaning on the thing -- is blocked only if it gets closer still:
    /// away from it is the way out, and calling every direction blocked
    /// left it standing there.
    pub fn blocks(&self, from: Vec3, to: Vec3, cap: &Capsule) -> bool {
        let feet = from.z.min(to.z);
        let head = from.z.max(to.z) + cap.height;
        if self.top <= feet + cap.step_up || self.bottom >= head {
            return false;
        }
        let reach = self.reach(cap);
        let a = from.truncate();
        let b = to.truncate();
        let nearest = nearest_on_segment(a, b, self.center);
        let start = a.distance(self.center);
        if start < reach {
            return nearest.distance(self.center) < start - 1e-3;
        }
        nearest.distance(self.center) < reach
    }
}

fn nearest_on_segment(a: Vec2, b: Vec2, p: Vec2) -> Vec2 {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 < 1e-8 {
        return a;
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    a + ab * t
}

/// The first cylinder the walk from `from` to `to` meets, if any.
pub fn first_hit<'c>(
    cyls: &'c [Cylinder],
    from: Vec3,
    to: Vec3,
    cap: &Capsule,
) -> Option<&'c Cylinder> {
    let a = from.truncate();
    let b = to.truncate();
    let ab = b - a;
    cyls.iter()
        .filter(|c| c.blocks(from, to, cap))
        .min_by(|c, d| {
            let along = |c: &Cylinder| (c.center - a).dot(ab);
            along(c).total_cmp(&along(d))
        })
}

/// `p` moved out of any cylinder it stands within reach of, radially,
/// to a berth's clearance. For a waypoint the graph put inside an
/// object it never knew about: a leg to a point inside a chest cannot
/// be threaded round the chest.
fn pushed_out(p: Vec3, cyls: &[Cylinder], cap: &Capsule) -> Vec3 {
    let mut q = p;
    for c in cyls {
        if c.top <= q.z + cap.step_up || c.bottom >= q.z + cap.height {
            continue;
        }
        let flat = q.truncate();
        let d = flat.distance(c.center);
        let want = c.reach(cap) + BERTH;
        if d < want {
            let dir = if d > 1e-4 {
                (flat - c.center) / d
            } else {
                Vec2::X
            };
            let moved = c.center + dir * want;
            q = Vec3::new(moved.x, moved.y, q.z);
        }
    }
    q
}

/// The corners of the way round `hit` from `a` to `b` on `side` (the
/// sign of the cross product with the direction of travel: left is
/// positive), in order, `a` and `b` themselves left out. Each corner
/// stands `berth` from the cylinder's middle.
///
/// From each end the walk goes to its tangent point on the circle of
/// that radius (an end already on or inside the circle starts from
/// where it is), and between the two tangent points it follows the arc
/// in chords short enough that none cuts back inside the cylinder's
/// reach. A single corner off the line beside the cylinder was the
/// first answer, and it clipped the cylinder whenever an end stood
/// close by: the leg from a corner to a waypoint just past a crate ran
/// through the crate's side.
fn corners_round(hit: &Cylinder, a: Vec2, b: Vec2, side: f32, berth: f32) -> Vec<Vec2> {
    let c = hit.center;
    let dir = (b - a).normalize_or_zero();
    let on_side = |q: Vec2| dir.perp_dot(q - c) * side;
    let tangent = |p: Vec2| -> Vec2 {
        let d = p.distance(c);
        if d <= berth + 1e-3 {
            return p;
        }
        let u = (p - c) / d;
        let alpha = (berth / d).clamp(-1.0, 1.0).acos();
        let one = c + Vec2::from_angle(alpha).rotate(u) * berth;
        let other = c + Vec2::from_angle(-alpha).rotate(u) * berth;
        if on_side(one) >= on_side(other) {
            one
        } else {
            other
        }
    };
    let ta = tangent(a);
    let tb = tangent(b);
    let angle = |q: Vec2| (q - c).to_angle();
    let (theta_a, theta_b) = (angle(ta), angle(tb));
    // Round by the arc whose middle lies on our side; the shorter of
    // the two when both do.
    let short = {
        let mut d = theta_b - theta_a;
        while d > std::f32::consts::PI {
            d -= std::f32::consts::TAU;
        }
        while d <= -std::f32::consts::PI {
            d += std::f32::consts::TAU;
        }
        d
    };
    let long = short - std::f32::consts::TAU * short.signum();
    let mid = |sweep: f32| c + Vec2::from_angle(theta_a + sweep * 0.5) * berth;
    let sweep = if on_side(mid(short)) >= 0.0 || on_side(mid(long)) < 0.0 {
        short
    } else {
        long
    };
    // Chords of a sixth of a turn at most keep the walk further from
    // the middle than the reach the berth was built on.
    let chords = (sweep.abs() / (std::f32::consts::PI / 3.0)).ceil().max(1.0) as usize;
    let mut out = Vec::new();
    if ta != a {
        out.push(ta);
    }
    for k in 1..chords {
        out.push(c + Vec2::from_angle(theta_a + sweep * k as f32 / chords as f32) * berth);
    }
    if tb != b {
        out.push(tb);
    }
    out
}

/// The way from `from` to `to` round `cyls`: `to` alone when the
/// straight walk meets none of them, else the corners of a detour and
/// then `to`. `clear(a, b)` is the ground's own word on a straight
/// walk, asked of every leg the detour adds. `None` when no detour
/// clears within `depth` objects.
///
/// A detour goes one side or the other of the first cylinder met, the
/// side it stands less across tried first. Each leg of it is then
/// threaded in its turn round the other cylinders, without the one
/// just rounded: its corners are tangent to a circle wider than the
/// reach, so no leg meets it again.
pub fn thread(
    cyls: &[Cylinder],
    from: Vec3,
    to: Vec3,
    cap: &Capsule,
    clear: &mut impl FnMut(Vec3, Vec3) -> bool,
    depth: u32,
) -> Option<Vec<Vec3>> {
    let Some(hit) = first_hit(cyls, from, to, cap) else {
        return Some(vec![to]);
    };
    if depth == 0 {
        return None;
    }
    let a = from.truncate();
    let b = to.truncate();
    let ab = b - a;
    let len = ab.length();
    if len < 1e-4 {
        return None;
    }
    let dir = ab / len;
    // Standing left of the line, it is shorter to pass on the right.
    let across = dir.perp_dot(hit.center - a);
    let sides = if across > 0.0 {
        [-1.0, 1.0]
    } else {
        [1.0, -1.0]
    };
    let berth = hit.reach(cap) + BERTH;
    let rest: Vec<Cylinder> = cyls.iter().filter(|c| *c != hit).copied().collect();
    'side: for side in sides {
        let corners = corners_round(hit, a, b, side, berth);
        let lift = |q: Vec2| {
            let t = ((q - a).dot(dir) / len).clamp(0.0, 1.0);
            Vec3::new(q.x, q.y, from.z + (to.z - from.z) * t)
        };
        let mut out = Vec::new();
        let mut at = from;
        for p in corners.into_iter().map(lift).chain(std::iter::once(to)) {
            if !clear(at, p) {
                continue 'side;
            }
            let Some(legs) = thread(&rest, at, p, cap, clear, depth - 1) else {
                continue 'side;
            };
            out.extend(legs);
            at = p;
        }
        return Some(out);
    }
    None
}

/// A [`Ground`] with the server's objects standing on it, for one
/// steer to one goal.
///
/// The ground answers for its geometry as before; this answers for the
/// cylinders on top. The straight line is blocked where it meets one,
/// and every route -- the block's own, the neighbourhood planner's --
/// has its legs threaded round the ones they cross before the steering
/// sees it. A leg that cannot be threaded is left as it was: the stuck
/// clocks remain the last resort, not the first.
///
/// An object the goal itself stands in is not an obstacle to that
/// goal. A walk to a chest ends at the chest, within arm's reach of
/// its middle, and a cylinder round the goal would have every approach
/// blocked and the character refusing the last stride.
pub struct Cluttered<'a, G: Ground> {
    ground: &'a mut G,
    cylinders: Vec<Cylinder>,
    cap: Capsule,
}

impl<'a, G: Ground> Cluttered<'a, G> {
    pub fn new(ground: &'a mut G, clutter: &[Cylinder], cap: Capsule, goal: Vec3) -> Self {
        let cylinders = clutter
            .iter()
            .filter(|c| c.center.distance(goal.truncate()) >= c.reach(&cap))
            .copied()
            .collect();
        Cluttered {
            ground,
            cylinders,
            cap,
        }
    }

    /// The obstacles this steer goes round.
    pub fn cylinders(&self) -> &[Cylinder] {
        &self.cylinders
    }

    /// `waypoints` from `from`, each leg threaded round the cylinders
    /// it crosses. The last waypoint is the goal and stays where it is;
    /// any other standing inside an object is first moved out of it.
    fn threaded(&mut self, block: u32, from: Vec3, waypoints: Vec<Vec3>) -> Vec<Vec3> {
        let Cluttered {
            ground,
            cylinders,
            cap,
        } = self;
        let mut clear = |a: Vec3, b: Vec3| !ground.line_blocked(block, a, b);
        let n = waypoints.len();
        let mut out = Vec::with_capacity(n);
        let mut at = from;
        for (i, w) in waypoints.into_iter().enumerate() {
            let w = if i + 1 < n {
                pushed_out(w, cylinders, cap)
            } else {
                w
            };
            match thread(cylinders, at, w, cap, &mut clear, DEPTH) {
                Some(legs) => out.extend(legs),
                None => {
                    tracing::debug!("route: no way round what stands between {at:?} and {w:?}");
                    out.push(w)
                }
            }
            at = w;
        }
        out
    }
}

impl<G: Ground> Ground for Cluttered<'_, G> {
    fn at(&self) -> Vec3 {
        self.ground.at()
    }

    fn cell(&self) -> u32 {
        self.ground.cell()
    }

    fn block(&self) -> u32 {
        self.ground.block()
    }

    fn line_blocked(&mut self, block: u32, from: Vec3, to: Vec3) -> bool {
        self.ground.line_blocked(block, from, to)
            || first_hit(&self.cylinders, from, to, &self.cap).is_some()
    }

    fn line_drops(&mut self, block: u32, from: Vec3, to: Vec3) -> bool {
        self.ground.line_drops(block, from, to)
    }

    fn find_path(&mut self, block: u32, from: Vec3, to: Vec3, goal_cell: u32) -> Option<Vec<Vec3>> {
        // The graph knows nothing of objects, so a goal with only an
        // object between here and it may have no graph route at all
        // (the straight line is the route): thread that line.
        let waypoints = self
            .ground
            .find_path(block, from, to, goal_cell)
            .or_else(|| (!self.ground.line_blocked(block, from, to)).then(|| vec![to]))?;
        Some(self.threaded(block, from, waypoints))
    }

    fn ask_wide(&mut self, from: Vec3, to: Vec3, block: u32, outdoors: bool, exact_to: bool) {
        self.ground.ask_wide(from, to, block, outdoors, exact_to)
    }

    fn take_wide(&mut self, from: Vec3, to: Vec3) -> Option<Vec<Vec3>> {
        let block = self.ground.block();
        let waypoints = self.ground.take_wide(from, to)?;
        Some(self.threaded(block, from, waypoints))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::steering::{Aim, Steering};
    use ac_formats::geom::{CylSphere, Sphere};
    use ac_world::object::Position;
    use glam::Quat;
    use std::time::Instant;

    /// Open ground: nothing built on it, every straight walk clear, and
    /// the graph's route to anywhere the straight line.
    #[derive(Default)]
    struct Open {
        at: Vec3,
        /// Straight walks the ground itself refuses, as (from, to) pairs
        /// judged by their midpoint: a wall standing there.
        walls: Vec<Vec2>,
    }

    impl Ground for Open {
        fn at(&self) -> Vec3 {
            self.at
        }
        fn cell(&self) -> u32 {
            0
        }
        fn block(&self) -> u32 {
            0
        }
        fn line_blocked(&mut self, _b: u32, from: Vec3, to: Vec3) -> bool {
            let mid = (from + to).truncate() * 0.5;
            self.walls.iter().any(|w| w.distance(mid) < 1.0)
        }
        fn line_drops(&mut self, _b: u32, _f: Vec3, _t: Vec3) -> bool {
            false
        }
        fn find_path(&mut self, _b: u32, _f: Vec3, to: Vec3, _c: u32) -> Option<Vec<Vec3>> {
            Some(vec![to])
        }
        fn ask_wide(&mut self, _f: Vec3, _t: Vec3, _b: u32, _o: bool, _e: bool) {}
        fn take_wide(&mut self, _f: Vec3, _t: Vec3) -> Option<Vec<Vec3>> {
            None
        }
    }

    fn cap() -> Capsule {
        Capsule::default()
    }

    /// A chest-sized thing standing at `(x, y)` on the floor.
    fn crate_at(x: f32, y: f32) -> Cylinder {
        Cylinder {
            center: Vec2::new(x, y),
            radius: 0.5,
            bottom: 0.0,
            top: 1.0,
        }
    }

    /// Every leg of a route from `me` through `route`, none crossing a
    /// cylinder.
    fn clears(route: &[Vec3], me: Vec3, cyls: &[Cylinder]) -> bool {
        let mut at = me;
        for w in route {
            if first_hit(cyls, at, *w, &cap()).is_some() {
                return false;
            }
            at = *w;
        }
        true
    }

    /// A setup with one cylinder: base at `origin`, `radius`, `height`.
    fn setup_with_cylinder(origin: Vec3, radius: f32, height: f32) -> Setup {
        Setup {
            id: 0x0200_0001,
            flags: 0,
            parts: vec![],
            parent_index: vec![],
            default_scale: vec![],
            holding_locations: vec![],
            connection_points: vec![],
            placement_frames: vec![],
            cyl_spheres: vec![CylSphere {
                origin,
                radius,
                height,
            }],
            spheres: vec![],
            height,
            radius,
            step_up_height: 0.0,
            step_down_height: 0.0,
            sorting_sphere: Sphere {
                origin: Vec3::ZERO,
                radius: 1.0,
            },
            selection_sphere: Sphere {
                origin: Vec3::ZERO,
                radius: 1.0,
            },
            lights: vec![],
            default_animation: 0,
            default_script: 0,
            default_motion_table: 0,
            default_sound_table: 0,
            default_script_table: 0,
        }
    }

    /// A chest standing on the floor of an outdoor cell at (5, 0).
    fn chest() -> WorldObject {
        WorldObject {
            guid: 0x8000_0001,
            setup_id: 0x0200_0001,
            scale: 1.0,
            position: Some(Position::new_flat(0x0000_0001, Vec3::new(5.0, 0.0, 0.0))),
            physics_state: object::PHYSICS_STATE_STATIC,
            object_desc_flags: object_desc_flags::STUCK,
            item_type: item_type::CONTAINER,
            ..Default::default()
        }
    }

    #[test]
    fn a_cylinder_between_character_and_goal_is_routed_around() {
        // Standing at the origin with a crate five metres off on the
        // way to a goal ten metres off. The ground is open, so the
        // steering used to walk straight and lean on the crate.
        let me = Vec3::ZERO;
        let goal = Vec3::new(10.0, 0.0, 0.0);
        let clutter = [crate_at(5.0, 0.0)];
        let mut open = Open {
            at: me,
            ..Default::default()
        };
        let mut g = Cluttered::new(&mut open, &clutter, cap(), goal);
        let mut st = Steering::new(Instant::now());
        let aim = match st.steer(&mut g, goal, 0, Instant::now()) {
            Aim::Go(at) => at,
            Aim::NoWay => panic!("refused a walk round a crate"),
        };
        assert!(
            aim.distance(goal) > 1.0,
            "walked straight at the crate: {aim:?}"
        );
        let route = st
            .route
            .as_ref()
            .expect("a route round it")
            .waypoints
            .clone();
        assert_eq!(route.last().copied(), Some(goal), "ends at the goal");
        assert!(
            clears(&route, me, &clutter),
            "the route crosses the crate: {route:?}"
        );
        // And it is a step aside, not a tour: the two tangent points
        // beside the crate, and on to the goal.
        assert!(route.len() <= 3, "{route:?}");
        assert!(
            route[..route.len() - 1]
                .iter()
                .all(|w| w.truncate().distance(clutter[0].center) < 2.0),
            "wandered off: {route:?}"
        );
    }

    #[test]
    fn the_side_against_the_wall_is_not_taken() {
        // The crate stands by a wall on its left; the way round is on
        // the right, whatever the geometry of the nearer side says.
        let me = Vec3::ZERO;
        let goal = Vec3::new(10.0, 0.0, 0.0);
        let clutter = [crate_at(5.0, 0.2)];
        // A wall where the left-hand detour's legs would cross.
        let mut open = Open {
            at: me,
            walls: vec![Vec2::new(2.5, 0.75), Vec2::new(7.5, 0.75)],
        };
        let mut g = Cluttered::new(&mut open, &clutter, cap(), goal);
        let mut st = Steering::new(Instant::now());
        st.steer(&mut g, goal, 0, Instant::now());
        let route = st
            .route
            .as_ref()
            .expect("a route round it")
            .waypoints
            .clone();
        assert!(
            route[0].y < 0.0,
            "took the side against the wall: {route:?}"
        );
        assert!(clears(&route, me, &clutter));
    }

    #[test]
    fn two_things_in_a_row_are_both_gone_round() {
        let me = Vec3::ZERO;
        let goal = Vec3::new(12.0, 0.0, 0.0);
        let clutter = [crate_at(4.0, 0.0), crate_at(8.0, 0.4)];
        let mut open = Open {
            at: me,
            ..Default::default()
        };
        let mut g = Cluttered::new(&mut open, &clutter, cap(), goal);
        let mut st = Steering::new(Instant::now());
        assert!(matches!(
            st.steer(&mut g, goal, 0, Instant::now()),
            Aim::Go(_)
        ));
        let route = st
            .route
            .as_ref()
            .expect("a route round them")
            .waypoints
            .clone();
        assert!(clears(&route, me, &clutter), "{route:?}");
    }

    #[test]
    fn no_way_round_leaves_the_leg_to_the_stuck_clocks() {
        // Walls on both sides of the crate: nothing to be done here,
        // and the answer is the old one -- walk, and let the clocks
        // decide -- not a refusal.
        let me = Vec3::ZERO;
        let goal = Vec3::new(10.0, 0.0, 0.0);
        let clutter = [crate_at(5.0, 0.0)];
        let mut open = Open {
            at: me,
            walls: vec![
                Vec2::new(2.5, 0.6),
                Vec2::new(7.5, 0.6),
                Vec2::new(2.5, -0.6),
                Vec2::new(7.5, -0.6),
            ],
        };
        let mut g = Cluttered::new(&mut open, &clutter, cap(), goal);
        let mut st = Steering::new(Instant::now());
        assert_eq!(st.steer(&mut g, goal, 0, Instant::now()), Aim::Go(goal));
        assert!(!st.no_way());
    }

    #[test]
    fn the_goal_inside_an_object_is_still_walked_to() {
        // A walk to a chest ends at the chest. Its own cylinder is no
        // obstacle to that walk, or the last stride is refused.
        let me = Vec3::ZERO;
        let goal = Vec3::new(5.0, 0.0, 0.0);
        let clutter = [crate_at(5.0, 0.0)];
        let mut open = Open {
            at: me,
            ..Default::default()
        };
        let mut g = Cluttered::new(&mut open, &clutter, cap(), goal);
        assert!(g.cylinders().is_empty());
        let mut st = Steering::new(Instant::now());
        assert_eq!(st.steer(&mut g, goal, 0, Instant::now()), Aim::Go(goal));
    }

    #[test]
    fn a_route_is_not_cut_through_an_object() {
        // A corner waypoint is passed early when the walk to the one
        // after it is clear -- clear of objects too, now.
        let me = Vec3::new(0.0, 0.0, 0.0);
        let corner = Vec3::new(0.5, 0.0, 0.0);
        let goal = Vec3::new(10.0, 0.0, 0.0);
        let clutter = [crate_at(5.0, 0.0)];
        let mut open = Open {
            at: me,
            ..Default::default()
        };
        let mut g = Cluttered::new(&mut open, &clutter, cap(), goal);
        let mut r = crate::steering::Route::new(goal, vec![corner, goal], Instant::now());
        let aim = r.target(me, |a, b| !g.line_blocked(0, a, b));
        assert_eq!(aim, corner, "cut the corner straight into the crate");
    }

    #[test]
    fn a_low_thing_is_stepped_onto_and_a_high_one_walked_under() {
        let low = Cylinder {
            top: 0.4,
            ..crate_at(5.0, 0.0)
        };
        let high = Cylinder {
            bottom: 2.5,
            top: 3.0,
            ..crate_at(5.0, 0.0)
        };
        let (a, b) = (Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0));
        assert!(!low.blocks(a, b, &cap()), "a step is not an obstacle");
        assert!(!high.blocks(a, b, &cap()), "a lintel is not an obstacle");
        assert!(crate_at(5.0, 0.0).blocks(a, b, &cap()));
        assert!(
            !crate_at(5.0, 1.5).blocks(a, b, &cap()),
            "a thing beside the path is not on it"
        );
    }

    #[test]
    fn already_leaning_on_it_the_way_out_is_open() {
        // The physics walked us into the thing (it collides with
        // nothing the server places). From inside its reach only a
        // walk closer in is blocked; away is the answer.
        let c = crate_at(5.0, 0.0);
        let inside = Vec3::new(4.3, 0.0, 0.0);
        assert!(c.blocks(inside, Vec3::new(6.0, 0.0, 0.0), &cap()));
        assert!(!c.blocks(inside, Vec3::new(0.0, 0.0, 0.0), &cap()));
        assert!(!c.blocks(inside, Vec3::new(4.3, 3.0, 0.0), &cap()));
    }

    #[test]
    fn cylinders_stand_where_the_object_stands() {
        // A cylinder a metre along the object's own x, on an object
        // turned a quarter left and twice its size: it stands two
        // metres along the world's y, twice as wide and twice as tall.
        let setup = setup_with_cylinder(Vec3::new(1.0, 0.0, 0.0), 0.5, 1.0);
        let mut o = chest();
        o.scale = 2.0;
        o.position = Some(Position {
            cell: 0x0000_0001,
            local: Vec3::new(5.0, 0.0, 3.0),
            rotation: Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
        });
        let cyls = cylinders(&o, &setup);
        assert_eq!(cyls.len(), 1);
        let c = cyls[0];
        assert!(c.center.distance(Vec2::new(5.0, 2.0)) < 1e-4, "{c:?}");
        assert!((c.radius - 1.0).abs() < 1e-5);
        assert!((c.bottom - 3.0).abs() < 1e-5 && (c.top - 5.0).abs() < 1e-5);
    }

    #[test]
    fn spheres_stand_in_when_there_are_no_cylinders() {
        let mut setup = setup_with_cylinder(Vec3::ZERO, 0.5, 1.0);
        setup.cyl_spheres.clear();
        setup.spheres = vec![Sphere {
            origin: Vec3::new(0.0, 0.0, 0.5),
            radius: 0.5,
        }];
        let cyls = cylinders(&chest(), &setup);
        assert_eq!(cyls.len(), 1);
        assert!((cyls[0].bottom - 0.0).abs() < 1e-5 && (cyls[0].top - 1.0).abs() < 1e-5);
        assert!((cyls[0].radius - 0.5).abs() < 1e-5);
    }

    #[test]
    fn an_ethereal_object_is_walked_through() {
        let setup = setup_with_cylinder(Vec3::ZERO, 0.5, 1.0);
        assert!(!cylinders(&chest(), &setup).is_empty(), "a chest is solid");
        let mut ghost = chest();
        ghost.physics_state |= object::PHYSICS_STATE_ETHEREAL;
        assert!(!in_the_way(&ghost));
        assert!(cylinders(&ghost, &setup).is_empty());
    }

    #[test]
    fn a_door_is_walked_through_whatever_its_physics_say() {
        // The project's rule: a character walks straight through
        // doors, and no route goes round one. The server marks a shut
        // door solid; that is not this client's concern.
        let setup = setup_with_cylinder(Vec3::ZERO, 0.5, 2.5);
        let mut door = chest();
        door.object_desc_flags = object_desc_flags::DOOR | object_desc_flags::STUCK;
        door.physics_state = object::PHYSICS_STATE_STATIC;
        door.item_type = item_type::LOCKABLE;
        assert!(!in_the_way(&door));
        assert!(cylinders(&door, &setup).is_empty());
    }

    #[test]
    fn a_creature_is_not_an_obstacle() {
        let setup = setup_with_cylinder(Vec3::ZERO, 0.5, 1.8);
        let mut drudge = chest();
        drudge.item_type = item_type::CREATURE;
        drudge.object_desc_flags = object_desc_flags::ATTACKABLE;
        assert!(cylinders(&drudge, &setup).is_empty());
        let mut player = chest();
        player.object_desc_flags = object_desc_flags::PLAYER;
        assert!(cylinders(&player, &setup).is_empty());
        let mut me = chest();
        me.is_player = true;
        assert!(cylinders(&me, &setup).is_empty());
    }

    #[test]
    fn what_is_carried_is_not_on_the_floor() {
        let setup = setup_with_cylinder(Vec3::ZERO, 0.5, 1.0);
        let mut packed = chest();
        packed.container = Some(0x5000_0001);
        assert!(cylinders(&packed, &setup).is_empty());
        let mut held = chest();
        held.position = None;
        assert!(cylinders(&held, &setup).is_empty());
        let mut wielded = chest();
        wielded.wielder = Some(0x5000_0001);
        assert!(cylinders(&wielded, &setup).is_empty());
    }

    #[test]
    fn a_missile_in_flight_is_not_an_obstacle() {
        let setup = setup_with_cylinder(Vec3::ZERO, 0.5, 1.0);
        let mut bolt = chest();
        bolt.physics_state = object::PHYSICS_STATE_MISSILE;
        assert!(cylinders(&bolt, &setup).is_empty());
    }

    #[test]
    fn a_waypoint_inside_an_object_is_moved_out_of_it() {
        // The graph put a corner in the middle of a crate it had never
        // heard of. The leg to it cannot be threaded; the corner is
        // moved out to the crate's side first.
        let me = Vec3::ZERO;
        let goal = Vec3::new(10.0, 5.0, 0.0);
        let clutter = [crate_at(5.0, 0.0)];
        let mut open = Open {
            at: me,
            ..Default::default()
        };
        let mut g = Cluttered::new(&mut open, &clutter, cap(), goal);
        let route = g.threaded(0, me, vec![Vec3::new(5.0, 0.0, 0.0), goal]);
        assert!(clears(&route, me, &clutter), "{route:?}");
        assert_eq!(route.last().copied(), Some(goal));
    }
}
