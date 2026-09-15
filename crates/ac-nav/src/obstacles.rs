//! What stands in the room, and the way round it.
//!
//! The walking physics collides with the landblock alone: walls,
//! floors, the statues and braziers the DAT files place, the trees and
//! bushes the terrain grows. What the server places -- a chest, a hook,
//! a cart in the road -- arrives as objects, with the packets, and
//! nothing here stops at one: the physics walks straight through it,
//! and the server takes the position it is sent (ACE's
//! `update_object_server` runs the transition only to note the
//! collision and sets the requested position regardless). So an object
//! in the way never stalled a walk; a stall is always the landblock's
//! own geometry, and belongs to the steering and the graph over that.
//! What this buys is the walk looking right: the retail client did stop
//! at a chest, and a character that goes round one moves the way a
//! player would rather than through the furniture.
//!
//! The objects are carried apart from the ground, as the shapes the
//! retail client collided with them by: the `CylSphere`s of the
//! object's Setup, vertical cylinders in the object's frame, and
//! failing those its `Sphere`s. They are not laid over the ground for
//! the steering to plan on -- the graph and the neighbourhood planner
//! are for what actually stops the character -- but applied to the
//! answer: each frame the leg from here to wherever the steering aims,
//! the goal or the next waypoint of its route, is threaded round the
//! first cylinder it meets ([`detour`]), and the character heads for
//! the first corner of that. Asked again next frame from a little
//! further on, the corner moves with it, and the walk hugs the object
//! round to where the straight line is clear again. A leg no detour
//! clears is left as it was.
//!
//! What is left out is as important as what goes in, and every case is
//! the retail rule or the project's own: a creature (the client walked
//! through them), anything carried (it is in a pack, not on the floor),
//! anything Ethereal (an open door, a portal, a corpse), a missile, an
//! object the client collided with by its parts' own BSP rather than by
//! any cylinder (there is no cylinder to stand in for it), and a door
//! whatever its state, because a character here walks straight through
//! doors and the navigation must never route round one.

use std::rc::Rc;

use ac_formats::setup::Setup;
use ac_scene::collision::Capsule;
use ac_world::{item_type, object, object_desc_flags, World, WorldObject};
use glam::{Vec2, Vec3};

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

/// Objects further off than this from the character are not in play.
/// Only the first thing on the leg being walked matters this frame;
/// anything further off is met when the walk gets there.
pub const NEARBY: f32 = 32.0;
/// How far the character may move before the objects gathered round
/// it are gathered again. They are gathered a margin wider than
/// [`NEARBY`] so that everything within it stays in the list until
/// then.
const MARGIN: f32 = 8.0;
/// Room a detour keeps between the character's side and the object
/// (metres), on top of both radii: the walking code slides along what
/// it touches, and a corner cut to the millimetre is a touch.
const BERTH: f32 = 0.3;
/// How many objects a single leg may be threaded round, one after
/// another, before it is left as it was. The search this bounds is
/// cheap in practice: a side is dropped at its first leg the ground
/// refuses, and a leg that meets one thing too many is dropped without
/// asking the ground at all, so a room packed with crates costs a
/// detour a dozen or so questions, not the hundreds two sides by a few
/// legs by three deep could reach.
const DEPTH: u32 = 3;

/// Whether `o` is something this client walks round: solid to the
/// retail client, standing on the floor, and collided with by the
/// shapes its Setup carries rather than by a part's own BSP.
pub fn in_the_way(o: &WorldObject) -> bool {
    // In a pack, on a belt, in a chest: not on the floor.
    let carried =
        o.position.is_none() || o.parent.is_some() || o.container.is_some() || o.wielder.is_some();
    // An open door, a portal, a corpse: solid to look at, not to walk.
    let ethereal = o.physics_state & object::PHYSICS_STATE_ETHEREAL != 0;
    let flying = o.physics_state & object::PHYSICS_STATE_MISSILE != 0;
    // Collided with by its parts' physics BSP (a table, a bench, most
    // furniture with a real shape). The Setup's cylinder, where it has
    // one, is a bounding shape the retail client never consulted for
    // these -- as wide as the table is long -- and there is no cylinder
    // that stands in for a slab. Left to the physics, which walks
    // through it.
    let by_bsp = o.physics_state & object::PHYSICS_STATE_HAS_PHYSICS_BSP != 0;
    // The client never collided the viewer with a creature, and a
    // vendor or a monster is what most walks are to.
    let creature = o.is_player
        || o.item_type & item_type::CREATURE != 0
        || o.object_desc_flags & object_desc_flags::PLAYER != 0;
    // A door is walked straight through, open or shut: the rule holds
    // whatever the server says its physics are, because a route that
    // goes round one strands the character at the doorway.
    let door = o.object_desc_flags & object_desc_flags::DOOR != 0;
    !(carried || ethereal || flying || by_bsp || creature || door)
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

/// Everything within `within` of `at` that a walk goes round, gathered
/// from the object table; `setup` answers for an object's Setup, and
/// is never asked about an object described without one.
pub fn around(
    world: &World,
    at: Vec3,
    within: f32,
    mut setup: impl FnMut(u32) -> Option<Rc<Setup>>,
) -> Vec<Cylinder> {
    world
        .objects
        .values()
        .filter(|o| in_the_way(o))
        // Described without a Setup: nothing to look up, and the
        // archive would be searched for it every time.
        .filter(|o| o.setup_id != 0)
        .filter(|o| o.world_pos().is_some_and(|p| p.distance(at) < within))
        .flat_map(|o| {
            setup(o.setup_id)
                .map(|s| cylinders(o, &s))
                .unwrap_or_default()
        })
        .collect()
}

/// The objects gathered round one character, kept from one frame to
/// the next.
///
/// A walk asks about them every frame, and the object table is a few
/// hundred entries with a Setup lookup for each one in reach. The
/// world says when anything about them changes (`World::generation`
/// moves on every object made, moved, hidden or deleted), and the
/// character says when it has walked far enough that things at the
/// edge of its reach have changed; between those the list stands.
#[derive(Debug, Default)]
pub struct Clutter {
    /// The world generation the list was gathered at.
    generation: Option<u64>,
    /// Where the character stood when it was gathered.
    at: Vec3,
    cylinders: Vec<Cylinder>,
}

impl Clutter {
    /// The cylinders within [`NEARBY`] of `at`, gathered afresh only
    /// when the world has changed or the character has moved
    /// [`MARGIN`] from where they were last gathered.
    pub fn refresh(
        &mut self,
        world: &World,
        at: Vec3,
        setup: impl FnMut(u32) -> Option<Rc<Setup>>,
    ) -> &[Cylinder] {
        let stale = self.generation != Some(world.generation) || self.at.distance(at) > MARGIN;
        if stale {
            self.cylinders = around(world, at, NEARBY + MARGIN, setup);
            self.generation = Some(world.generation);
            self.at = at;
        }
        &self.cylinders
    }
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
    /// in the thing, the physics having no reason to keep it out -- is
    /// blocked only if it gets closer still: away from it is the way
    /// out, and calling every direction blocked left it standing there.
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

/// The corners of the way round `hit` from `a` to `b` on `side` (the
/// sign of the cross product with the direction of travel: left is
/// positive), in order, `a` and `b` themselves left out. Each corner
/// stands `berth` from the cylinder's middle, which is further than
/// `reach`, the distance the walk must keep.
///
/// From each end the walk goes to its tangent point on the circle of
/// that radius (an end already on or inside the circle starts from
/// where it is), and between the two tangent points it follows the arc
/// in chords short enough that none cuts back inside the reach. A
/// single corner off the line beside the cylinder was the first
/// answer, and it clipped the cylinder whenever an end stood close by:
/// the leg from a corner to a waypoint just past a crate ran through
/// the crate's side.
fn corners_round(hit: &Cylinder, a: Vec2, b: Vec2, side: f32, berth: f32, reach: f32) -> Vec<Vec2> {
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
    // A chord's middle is the nearest it comes to the centre, at
    // berth * cos(half the sweep): the sweep is chosen so that it stays
    // half a berth's clearance outside the reach, and never wider than
    // a sixth of a turn. A fixed sixth of a turn was the first answer,
    // and it holds only for things under two metres across -- a
    // lifestone's chords cut back inside its reach, and the walk
    // reported its own detour as blocked by the thing it was rounding.
    let chord = (2.0 * ((reach + BERTH * 0.5) / berth).clamp(-1.0, 1.0).acos())
        .clamp(0.05, std::f32::consts::FRAC_PI_3);
    let chords = (sweep.abs() / chord).ceil().max(1.0) as usize;
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
    let reach = hit.reach(cap);
    let berth = reach + BERTH;
    let rest: Vec<Cylinder> = cyls.iter().filter(|c| *c != hit).copied().collect();
    'side: for side in sides {
        let corners = corners_round(hit, a, b, side, berth, reach);
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

/// Where to head this frame instead of `aim`: the first corner of the
/// way round whatever stands on the straight walk from `from` to it,
/// or `aim` itself when nothing does, or when no way round clears the
/// ground within [`DEPTH`] objects.
///
/// An object the leg ends in is no obstacle to that leg. A walk to a
/// chest ends at the chest, within arm's reach of its middle, and a
/// waypoint the graph put in one (it never heard of the chest) is
/// reached the same way; a cylinder round the end would have every
/// approach blocked and the character refusing the last stride.
pub fn detour(
    cyls: &[Cylinder],
    from: Vec3,
    aim: Vec3,
    cap: &Capsule,
    mut clear: impl FnMut(Vec3, Vec3) -> bool,
) -> Vec3 {
    let ends_in = |c: &Cylinder| c.center.distance(aim.truncate()) < c.reach(cap);
    if !cyls.iter().any(|c| !ends_in(c) && c.blocks(from, aim, cap)) {
        return aim;
    }
    let in_play: Vec<Cylinder> = cyls.iter().filter(|c| !ends_in(c)).copied().collect();
    match thread(&in_play, from, aim, cap, &mut clear, DEPTH) {
        Some(legs) => legs.first().copied().unwrap_or(aim),
        None => {
            tracing::trace!(target: "steer", "no way round what stands between {from:?} and {aim:?}");
            aim
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_formats::geom::{CylSphere, Sphere};
    use ac_world::object::Position;
    use glam::Quat;

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

    /// Open ground, but for walls: a straight walk is refused when its
    /// middle lies within a metre of one.
    fn walls(at: &[(f32, f32)]) -> impl FnMut(Vec3, Vec3) -> bool + '_ {
        move |a: Vec3, b: Vec3| {
            let mid = (a + b).truncate() * 0.5;
            !at.iter().any(|&(x, y)| Vec2::new(x, y).distance(mid) < 1.0)
        }
    }

    /// Walks from `me` towards `goal` a quarter-metre a frame, asking
    /// [`detour`] afresh each frame as the client does: the positions
    /// visited, ending within a stride of the goal.
    fn walk(
        cyls: &[Cylinder],
        me: Vec3,
        goal: Vec3,
        mut clear: impl FnMut(Vec3, Vec3) -> bool,
    ) -> Vec<Vec3> {
        let mut at = me;
        let mut seen = vec![me];
        for _ in 0..400 {
            if at.truncate().distance(goal.truncate()) < 0.3 {
                return seen;
            }
            let aim = detour(cyls, at, goal, &cap(), &mut clear);
            let d = (aim - at).truncate();
            let step = d.normalize_or_zero() * d.length().min(0.25);
            at += step.extend(0.0);
            seen.push(at);
        }
        panic!("never arrived: ended at {at:?}");
    }

    /// No point of `path` stands within reach of a cylinder.
    fn keeps_clear(path: &[Vec3], cyls: &[Cylinder]) -> bool {
        path.iter().all(|p| {
            cyls.iter()
                .all(|c| p.truncate().distance(c.center) >= c.reach(&cap()) - 1e-3)
        })
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
    fn a_cylinder_between_character_and_goal_is_walked_round() {
        // Standing at the origin with a crate five metres off on the
        // way to a goal ten metres off. The physics would walk straight
        // through it; the walk goes round.
        let me = Vec3::ZERO;
        let goal = Vec3::new(10.0, 0.0, 0.0);
        let clutter = [crate_at(5.0, 0.0)];
        let first = detour(&clutter, me, goal, &cap(), |_, _| true);
        assert!(first.distance(goal) > 1.0, "walked straight at the crate");
        let path = walk(&clutter, me, goal, |_, _| true);
        assert!(keeps_clear(&path, &clutter), "{path:?}");
        // And it is a step aside, not a tour.
        assert!(
            path.iter().all(|p| p.y.abs() < 2.0),
            "wandered off: {path:?}"
        );
        assert!(path.len() < 60, "{} frames for ten metres", path.len());
    }

    #[test]
    fn the_side_against_the_wall_is_not_taken() {
        // The crate stands by a wall on its left; the way round is on
        // the right, whatever the geometry of the nearer side says.
        let me = Vec3::ZERO;
        let goal = Vec3::new(10.0, 0.0, 0.0);
        let clutter = [crate_at(5.0, 0.2)];
        let path = walk(&clutter, me, goal, walls(&[(2.5, 0.75), (7.5, 0.75)]));
        assert!(
            path.iter().all(|p| p.y < 0.05),
            "took the side against the wall: {path:?}"
        );
        assert!(keeps_clear(&path, &clutter));
    }

    #[test]
    fn two_things_in_a_row_are_both_gone_round() {
        let me = Vec3::ZERO;
        let goal = Vec3::new(12.0, 0.0, 0.0);
        let clutter = [crate_at(4.0, 0.0), crate_at(8.0, 0.4)];
        let path = walk(&clutter, me, goal, |_, _| true);
        assert!(keeps_clear(&path, &clutter), "{path:?}");
    }

    #[test]
    fn no_way_round_leaves_the_leg_as_it_was() {
        // Walls on both sides of the crate: nothing to be done here,
        // and the answer is the old one -- walk at the aim.
        let me = Vec3::ZERO;
        let goal = Vec3::new(10.0, 0.0, 0.0);
        let clutter = [crate_at(5.0, 0.0)];
        let both_sides = [(2.5, 0.6), (7.5, 0.6), (2.5, -0.6), (7.5, -0.6)];
        assert_eq!(detour(&clutter, me, goal, &cap(), walls(&both_sides)), goal);
    }

    #[test]
    fn the_leg_that_ends_in_an_object_is_still_walked() {
        // A walk to a chest ends at the chest, and a waypoint the graph
        // put in one is reached the same way. The chest's own cylinder
        // is no obstacle to that leg, or the last stride is refused.
        let me = Vec3::ZERO;
        let clutter = [crate_at(5.0, 0.0)];
        let goal = Vec3::new(5.0, 0.0, 0.0);
        assert_eq!(detour(&clutter, me, goal, &cap(), |_, _| true), goal);
        let corner = Vec3::new(4.5, 0.3, 0.0);
        assert_eq!(detour(&clutter, me, corner, &cap(), |_, _| true), corner);
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
    fn already_in_it_the_way_out_is_open() {
        // The physics walked us into the thing. From inside its reach
        // only a walk closer in is blocked; away is the answer.
        let c = crate_at(5.0, 0.0);
        let inside = Vec3::new(4.3, 0.0, 0.0);
        assert!(c.blocks(inside, Vec3::new(6.0, 0.0, 0.0), &cap()));
        assert!(!c.blocks(inside, Vec3::new(0.0, 0.0, 0.0), &cap()));
        assert!(!c.blocks(inside, Vec3::new(4.3, 3.0, 0.0), &cap()));
        // And a walk that starts in it gets out and on to the goal.
        let path = walk(&[c], inside, Vec3::new(10.0, 0.0, 0.0), |_, _| true);
        assert!(keeps_clear(&path[8..], &[c]), "{path:?}");
    }

    #[test]
    fn a_big_round_thing_is_rounded_without_cutting_into_it() {
        // A lifestone: a sphere a metre and a half across. Chords of a
        // sixth of a turn round something that size cut back inside
        // its reach, and every leg of the detour read as blocked by
        // the thing being rounded.
        let stone = Cylinder {
            center: Vec2::new(5.0, 0.0),
            radius: 1.504,
            bottom: -1.5,
            top: 1.5,
        };
        let (me, goal) = (Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0));
        let legs = thread(&[stone], me, goal, &cap(), &mut |_, _| true, DEPTH).expect("a way");
        let mut at = me;
        for leg in &legs {
            assert!(
                !stone.blocks(at, *leg, &cap()),
                "the leg {at:?} -> {leg:?} cuts into the stone"
            );
            at = *leg;
        }
        let path = walk(&[stone], me, goal, |_, _| true);
        assert!(keeps_clear(&path, &[stone]), "{path:?}");
    }

    #[test]
    fn a_crowded_room_costs_the_ground_few_questions() {
        // The search runs two sides by a few legs by DEPTH deep, and
        // every question it asks the ground is a raycast and a sampled
        // walk on the frame thread, asked again every frame while the
        // detour keeps failing. It stays cheap because a side is
        // dropped at the first leg the ground refuses and a leg that
        // meets one thing too many is dropped unasked. Rooms packed
        // with crates closer together than a character is wide, with
        // a wall across the far end so that no detour ever succeeds:
        // a dozen or so questions each, measured.
        let (me, goal) = (Vec3::ZERO, Vec3::new(12.0, 0.0, 0.0));
        let rooms: Vec<Vec<Cylinder>> = vec![
            vec![crate_at(3.0, 0.0), crate_at(3.0, -1.5), crate_at(3.0, 1.5)],
            (0..4)
                .flat_map(|i| {
                    [
                        crate_at(3.0 + i as f32 * 1.6, -0.8),
                        crate_at(3.8 + i as f32 * 1.6, 0.8),
                    ]
                })
                .collect(),
            (0..4)
                .flat_map(|i| {
                    (0..5).map(move |j| crate_at(3.0 + i as f32 * 2.4, j as f32 * 2.4 - 4.8))
                })
                .collect(),
        ];
        for room in rooms {
            let mut asked = 0u32;
            let way = thread(
                &room,
                me,
                goal,
                &cap(),
                &mut |_, b: Vec3| {
                    asked += 1;
                    b.x < 11.5
                },
                DEPTH,
            );
            assert!(way.is_none(), "found a way through the wall");
            assert!(asked <= 20, "{asked} questions asked of the ground");
        }
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
    fn a_thing_collided_with_by_its_bsp_is_left_to_the_physics() {
        // A table: the server says its Setup has a physics BSP, so the
        // retail client collided with the slab, never with the
        // cylinder as long as the table that its Setup also carries.
        let setup = setup_with_cylinder(Vec3::ZERO, 2.0, 1.0);
        let mut table = chest();
        table.physics_state |= object::PHYSICS_STATE_HAS_PHYSICS_BSP;
        assert!(!in_the_way(&table));
        assert!(cylinders(&table, &setup).is_empty());
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

    /// A world with a chest at (5, 0) and, `far` metres along x, a
    /// second one described without a Setup.
    fn room() -> World {
        let mut world = World::default();
        world.objects.insert(chest().guid, chest());
        let mut bare = chest();
        bare.guid = 0x8000_0002;
        bare.setup_id = 0;
        world.objects.insert(bare.guid, bare);
        world
    }

    #[test]
    fn an_object_without_a_setup_is_not_looked_up() {
        let setup = Rc::new(setup_with_cylinder(Vec3::ZERO, 0.5, 1.0));
        let mut asked = Vec::new();
        let cyls = around(&room(), Vec3::ZERO, NEARBY, |id| {
            asked.push(id);
            Some(setup.clone())
        });
        assert_eq!(cyls.len(), 1);
        assert_eq!(
            asked,
            vec![0x0200_0001],
            "looked up a Setup it was never given"
        );
    }

    #[test]
    fn the_gather_stands_until_the_world_changes_or_the_character_moves() {
        let setup = Rc::new(setup_with_cylinder(Vec3::ZERO, 0.5, 1.0));
        let mut world = room();
        let mut clutter = Clutter::default();
        let lookups = std::cell::Cell::new(0);
        let lookup = |_: u32| {
            lookups.set(lookups.get() + 1);
            Some(setup.clone())
        };
        assert_eq!(clutter.refresh(&world, Vec3::ZERO, lookup).len(), 1);
        // The next frames, a stride on: nothing to gather again.
        for i in 1..20 {
            clutter.refresh(&world, Vec3::new(i as f32 * 0.1, 0.0, 0.0), lookup);
        }
        assert_eq!(lookups.get(), 1, "gathered again with nothing changed");
        // Something in the world changed: gathered afresh.
        world.generation += 1;
        clutter.refresh(&world, Vec3::new(2.0, 0.0, 0.0), lookup);
        assert_eq!(lookups.get(), 2);
        // Walked far enough for the edge of its reach to have moved.
        clutter.refresh(&world, Vec3::new(2.0 + MARGIN + 1.0, 0.0, 0.0), lookup);
        assert_eq!(lookups.get(), 3);
    }
}
