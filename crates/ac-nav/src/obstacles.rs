//! Server-placed objects and the way round them: [`detour`] over what [`Clutter::refresh`] gathers.
//! Looks only: the walking physics collides with the landblock alone and ACE sets the position it is
//! sent whatever the transition found (`PhysicsObj.cs:4252`), so a stall is never an object, always
//! the landblock's own geometry. Retail did stop at a chest, so a character that goes round one
//! walks the way a player would; [`in_the_way`] holds what is walked through instead.

use std::rc::Rc;

use ac_formats::setup::Setup;
use ac_scene::collision::Capsule;
use ac_world::{item_type, object, object_desc_flags, World, WorldObject};
use glam::{Vec2, Vec3};

/// An object's retail collision shape placed in the world: a vertical cylinder.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cylinder {
    /// Where its axis stands, world x and y.
    pub center: Vec2,
    pub radius: f32,
    /// World height of its base and its top.
    pub bottom: f32,
    pub top: f32,
}

/// Objects beyond this (metres) are not in play: only the first thing on the leg matters this frame.
pub const NEARBY: f32 = 32.0;
/// Metres walked before objects are gathered again; gathered this much wider than [`NEARBY`]
/// so everything within it stays listed until then.
const MARGIN: f32 = 8.0;
/// Clearance (metres) a detour keeps beyond both radii: the walk slides along what it touches.
const BERTH: f32 = 0.3;
/// Objects one leg may be threaded round before it is left as it was.
/// A side drops at its first refused leg, so a packed room costs about a dozen ground questions
/// (measured: a_crowded_room_costs_the_ground_few_questions).
const DEPTH: u32 = 3;

/// Whether a walk goes round `o`: solid to retail, on the floor, and collided with by its Setup's
/// shapes rather than a part's own BSP.
pub fn in_the_way(o: &WorldObject) -> bool {
    // In a pack, on a belt, in a chest: not on the floor.
    let carried =
        o.position.is_none() || o.parent.is_some() || o.container.is_some() || o.wielder.is_some();
    // An open door, a portal, a corpse: solid to look at, not to walk.
    let ethereal = o.physics_state & object::PHYSICS_STATE_ETHEREAL != 0;
    let flying = o.physics_state & object::PHYSICS_STATE_MISSILE != 0;
    // A table or bench: retail collided its parts' BSP, never the Setup's bounding cylinder (as wide
    // as the table is long); left to the physics, which walks through it.
    let by_bsp = o.physics_state & object::PHYSICS_STATE_HAS_PHYSICS_BSP != 0;
    // Retail never collided the player with a creature, and most walks are to a vendor or a monster.
    let creature = o.is_player
        || o.item_type & item_type::CREATURE != 0
        || o.object_desc_flags & object_desc_flags::PLAYER != 0;
    // Never route round a door, open or shut, whatever its physics: that strands the character at
    // the doorway.
    let door = o.object_desc_flags & object_desc_flags::DOOR != 0;
    !(carried || ethereal || flying || by_bsp || creature || door)
}

/// `o`'s world-space cylinders: its Setup's `CylSphere`s, else its `Sphere`s; empty when
/// [`in_the_way`] says to walk through it, and for a Setup with neither.
/// Placed as retail did: origin scaled, turned by the object's frame, set at its position, axis upright.
/// A `CylSphere`'s origin is its base and its height goes up; a `Sphere` reaches its radius either way.
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

/// The cylinders within `within` metres of `at`, from the object table; `setup` is never asked
/// about an object described without one.
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
        // No Setup: nothing to look up, and the archive would be searched for it every time.
        .filter(|o| o.setup_id != 0)
        .filter(|o| o.world_pos().is_some_and(|p| p.distance(at) < within))
        .flat_map(|o| {
            setup(o.setup_id)
                .map(|s| cylinders(o, &s))
                .unwrap_or_default()
        })
        .collect()
}

/// The cylinders round one character, kept between frames: a walk asks every frame, and the table
/// is a few hundred objects with a Setup lookup each.
/// `World::generation` moves on every object made, moved, hidden or deleted.
#[derive(Debug, Default)]
pub struct Clutter {
    /// The world generation the list was gathered at.
    generation: Option<u64>,
    /// Where the character stood when it was gathered.
    at: Vec3,
    cylinders: Vec<Cylinder>,
}

impl Clutter {
    /// The cylinders within [`NEARBY`] of `at`, gathered afresh when the world changes or `at` has
    /// moved `MARGIN` from the last gather.
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

    /// Whether the walk from `from` to `to` (feet, world space) meets this: a top within `step_up` is
    /// stepped onto as any ledge is, a bottom above the head walked under.
    /// Begun inside the reach, only a walk closer in is blocked (already_in_it_the_way_out_is_open).
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

/// Corners round `hit` from `a` to `b` on `side` (sign of the cross product with the direction of
/// travel, left positive), in order, ends left out; each stands `berth` from the middle, past `reach`.
/// Tangent from each end to that circle (an end inside it starts where it is), then the arc in
/// chords short enough that none cuts back inside `reach`.
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
    // A chord comes nearest the centre at its middle, berth * cos(sweep / 2): kept BERTH / 2 outside
    // the reach and at most a sixth of a turn, which alone holds only under 2 m across
    // (a_big_round_thing_is_rounded_without_cutting_into_it).
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

/// The legs from `from` to `to` round `cyls`, or `to` alone when the straight walk meets none;
/// `clear(a, b)` is the ground's word on every leg added, `None` when none clears within `depth`.
/// Each leg is threaded in turn round the rest, without the cylinder just rounded: its corners are
/// tangent to a circle wider than the reach, so no leg meets it again.
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

/// Where to head this frame instead of `aim`: the first corner of the way round what stands on the
/// walk from `from`, else `aim` itself, including when no way round clears within `DEPTH` objects.
/// An object the leg ends in is no obstacle to it, or a walk to a chest refuses its last stride.
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

    /// Ground whose walls refuse a straight walk whose middle lies within a metre of one.
    fn walls(at: &[(f32, f32)]) -> impl FnMut(Vec3, Vec3) -> bool + '_ {
        move |a: Vec3, b: Vec3| {
            let mid = (a + b).truncate() * 0.5;
            !at.iter().any(|&(x, y)| Vec2::new(x, y).distance(mid) < 1.0)
        }
    }

    /// The positions visited walking `me` to `goal` 0.25 m a frame, asking `detour` afresh each frame
    /// as the client does, ending within a stride of the goal.
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
        // A crate 5 m along the way to a goal 10 m off: the physics would walk straight through it,
        // the walk goes round, and it is a step aside, not a tour.
        let me = Vec3::ZERO;
        let goal = Vec3::new(10.0, 0.0, 0.0);
        let clutter = [crate_at(5.0, 0.0)];
        let first = detour(&clutter, me, goal, &cap(), |_, _| true);
        assert!(first.distance(goal) > 1.0, "walked straight at the crate");
        let path = walk(&clutter, me, goal, |_, _| true);
        assert!(keeps_clear(&path, &clutter), "{path:?}");
        assert!(
            path.iter().all(|p| p.y.abs() < 2.0),
            "wandered off: {path:?}"
        );
        assert!(path.len() < 60, "{} frames for ten metres", path.len());
    }

    #[test]
    fn the_side_against_the_wall_is_not_taken() {
        // The crate stands by a wall on its left: the way round is on the right, whatever the nearer
        // side's geometry says.
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
        // Walls on both sides of the crate: the answer is the aim itself.
        let me = Vec3::ZERO;
        let goal = Vec3::new(10.0, 0.0, 0.0);
        let clutter = [crate_at(5.0, 0.0)];
        let both_sides = [(2.5, 0.6), (7.5, 0.6), (2.5, -0.6), (7.5, -0.6)];
        assert_eq!(detour(&clutter, me, goal, &cap(), walls(&both_sides)), goal);
    }

    #[test]
    fn the_leg_that_ends_in_an_object_is_still_walked() {
        // A walk to a chest ends at the chest, and a waypoint the graph put in one is reached the same
        // way: its cylinder is no obstacle to that leg, or the last stride is refused.
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
        // The physics walked us into it: from inside its reach only a walk closer in is blocked.
        let c = crate_at(5.0, 0.0);
        let inside = Vec3::new(4.3, 0.0, 0.0);
        assert!(c.blocks(inside, Vec3::new(6.0, 0.0, 0.0), &cap()));
        assert!(!c.blocks(inside, Vec3::new(0.0, 0.0, 0.0), &cap()));
        assert!(!c.blocks(inside, Vec3::new(4.3, 3.0, 0.0), &cap()));
        // A walk that starts in it gets out and on to the goal.
        let path = walk(&[c], inside, Vec3::new(10.0, 0.0, 0.0), |_, _| true);
        assert!(keeps_clear(&path[8..], &[c]), "{path:?}");
    }

    #[test]
    fn a_big_round_thing_is_rounded_without_cutting_into_it() {
        // A lifestone, radius 1.5 m: sixth-of-a-turn chords round something that size cut back inside
        // its reach, and every leg of the detour read as blocked by the stone.
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
        // Every question asked of the ground is a raycast and a sampled walk on the frame thread, asked
        // again every frame a detour fails. Crates closer than a character is wide with a wall across
        // the far end, so no detour ever succeeds: a dozen or so questions each, measured.
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
        // A cylinder a metre along the object's own x, on an object turned a quarter left and twice its
        // size: it stands two metres along the world's y, twice as wide and twice as tall.
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
        // A table: the server says its Setup has a physics BSP, so retail collided with the slab, never
        // with the cylinder as long as the table that its Setup also carries.
        let setup = setup_with_cylinder(Vec3::ZERO, 2.0, 1.0);
        let mut table = chest();
        table.physics_state |= object::PHYSICS_STATE_HAS_PHYSICS_BSP;
        assert!(!in_the_way(&table));
        assert!(cylinders(&table, &setup).is_empty());
    }

    #[test]
    fn a_door_is_walked_through_whatever_its_physics_say() {
        // The project's rule: a character walks straight through doors and no route goes round one.
        // The server marks a shut door solid; that is not this client's concern.
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

    /// A world with a chest at (5, 0) and a second one beside it described without a Setup.
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
        world.generation += 1;
        clutter.refresh(&world, Vec3::new(2.0, 0.0, 0.0), lookup);
        assert_eq!(lookups.get(), 2);
        // Walked far enough for the edge of its reach to have moved.
        clutter.refresh(&world, Vec3::new(2.0 + MARGIN + 1.0, 0.0, 0.0), lookup);
        assert_eq!(lookups.get(), 3);
    }
}
