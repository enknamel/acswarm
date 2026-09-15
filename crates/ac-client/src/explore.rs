//! Walking a dungeon to find the fight in it.
//!
//! The server describes a dungeon a room at a time: the creatures in
//! the cell the character stands in and in the cells that one can see,
//! and nothing else. Holtburg Dungeon holds fifty-eight creatures
//! across seventy-four rooms and the room its portal drops you in holds
//! none of them -- only two monster generators, which are server-side
//! and never described to anybody. A character that arrives and waits
//! for something to fight therefore waits for ever and reports an empty
//! dungeon. It was never empty; it was never walked.
//!
//! Which doorway to take is [`ac_nav::explore::next_step`] and is
//! tested there. This is the part that needs a world: which cells there
//! are, where their thresholds lie, and handing the one we pick to the
//! travel system like every other destination.

use crate::autoplay::Doing;
use crate::Client;
use ac_nav::explore::{next_step, Doors};
use ac_scene::interior::CellScene;
use glam::Vec3;
use std::time::{Duration, Instant};

/// How long to spend getting through one doorway before writing the
/// room beyond it off. The next room is a few metres away; longer than
/// this and something is in the way that more walking will not move.
const GIVE_UP_ON_A_ROOM: Duration = Duration::from_secs(10);

/// How far past a threshold to aim. A threshold is the line between
/// two rooms: walk to it exactly and the character stops standing in
/// the room it started in, the server goes on describing that room and
/// no other, and the dungeon stays as empty as it looked.
const PAST_THE_DOOR: f32 = 2.5;

/// How close to the aiming point counts as being there.
const STOP: f32 = 1.0;

/// The dungeon's rooms and what opens on to what.
struct Rooms<'a>(&'a [CellScene]);

impl Doors for Rooms<'_> {
    fn beyond(&self, cell: u32) -> Vec<u32> {
        let Some(room) = self.0.iter().find(|c| c.cell_id == cell) else {
            return Vec::new();
        };
        // Only openings into other rooms of this dungeon. A portal to
        // the outdoors is the way out, not somewhere to explore, and it
        // names a cell that is not in this list.
        room.portal_cells
            .iter()
            .copied()
            .filter(|id| self.0.iter().any(|c| c.cell_id == *id))
            .collect()
    }
}

/// A walk into the next room: where we set out from, the room, the
/// point past its threshold we are aiming at, and whether that point
/// has already been pushed further in once because reaching it did not
/// put us in the room.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoomWalk {
    pub from: u32,
    pub room: u32,
    pub at: Vec3,
    pub pushed: bool,
}

/// The point to aim at to be through a doorway whose sill is at `sill`
/// and whose wall stands across `normal` (a unit vector on the map,
/// pointing either way through it), coming from `from` and facing
/// `facing`: `past` metres beyond the sill, straight through the wall,
/// on the far side from where we came.
///
/// Straight through, not along the line we approached on: a character
/// coming at a corridor's door from the corner of the room before it
/// was sent two and a half metres past the sill on that diagonal, into
/// the corridor's side wall, where there was no floor to reach and every
/// step was a centimetre of jitter for ten seconds. Which side is the
/// far side is read off where we stand; standing on the sill itself
/// there is no side, and the way we face -- the way we were walking --
/// says. Aiming at the sill from the sill was the other stall: the walk
/// had arrived by its own measure while the server still had the
/// character in the room it started in.
fn past_the_door(sill: Vec3, normal: Vec3, from: Vec3, facing: glam::Vec2, past: f32) -> Vec3 {
    let across = glam::Vec2::new(sill.x - from.x, sill.y - from.y);
    let n = glam::Vec2::new(normal.x, normal.y);
    let through = if n.length() > 0.5 {
        let side = across.dot(n);
        let ahead = facing.dot(n);
        if side.abs() >= STOP * 0.5 {
            n * side.signum()
        } else if ahead.abs() > 0.1 {
            n * ahead.signum()
        } else {
            n
        }
    } else if across.length() >= STOP {
        across.normalize()
    } else if facing.length() > 0.1 {
        facing.normalize()
    } else {
        glam::Vec2::ZERO
    };
    let on = through * past;
    Vec3::new(sill.x + on.x, sill.y + on.y, sill.z)
}

/// The doorway of `room` nearest `from`: where its sill is and the way
/// through it, or the middle of the cell when the data has no opening.
fn nearest_door(cells: &[CellScene], room: u32, from: Vec3) -> Option<(Vec3, Vec3)> {
    let cell = cells.iter().find(|c| c.cell_id == room)?;
    let Some((i, sill)) = cell
        .doorways
        .iter()
        .copied()
        .enumerate()
        .min_by(|a, b| a.1.distance(from).total_cmp(&b.1.distance(from)))
    else {
        return Some((cell.transform.transform_point3(Vec3::ZERO), Vec3::ZERO));
    };
    let normal = cell.doorway_normals.get(i).copied().unwrap_or(Vec3::ZERO);
    Some((sill, normal))
}

/// Where to walk to to be in `room`: through the opening between us and
/// it, and a couple of paces beyond, so that the character ends up
/// inside rather than standing in the doorway. `facing` is the way the
/// character is pointing, for when it stands on the sill already.
fn way_into(
    cells: &[CellScene],
    room: u32,
    from: Vec3,
    facing: glam::Vec2,
    past: f32,
) -> Option<Vec3> {
    let (sill, normal) = nearest_door(cells, room, from)?;
    if normal == Vec3::ZERO
        && cells
            .iter()
            .any(|c| c.cell_id == room && c.doorways.is_empty())
    {
        // No opening in the data: the middle of the cell structure is
        // the best guess there is.
        return Some(sill);
    }
    Some(past_the_door(sill, normal, from, facing, past))
}

/// `at`, or the nearest thing to it past `sill` that `has_floor`: an aim
/// with nothing to stand on is inside a wall, and a walk at it jitters
/// against the wall until the room is given up.
///
/// Straight through the door is tried first; then the same distance
/// bearing forty-five degrees either way, for a corridor that turns at
/// its door (the Holtburg Dungeon's run diagonally from theirs); then
/// shorter, straight through, down to a pace past the sill.
pub fn aim_on_floor(sill: Vec3, at: Vec3, has_floor: impl Fn(Vec3) -> bool) -> Vec3 {
    if has_floor(at) {
        return at;
    }
    let through = glam::Vec2::new(at.x - sill.x, at.y - sill.y);
    let past = through.length();
    if past < 1e-3 {
        return at;
    }
    let turned = |a: f32| {
        let d = glam::Vec2::from_angle(a).rotate(through);
        Vec3::new(sill.x + d.x, sill.y + d.y, at.z)
    };
    let quarter = std::f32::consts::FRAC_PI_4;
    for p in [turned(quarter), turned(-quarter)] {
        if has_floor(p) {
            return p;
        }
    }
    for k in [0.6, 0.4, 0.25] {
        let p = sill.lerp(at, k);
        if has_floor(p) {
            return p;
        }
    }
    at
}

/// The way a character with `heading` is pointing, on the map.
/// Headings turn the other way from the maths: see `Player::heading`.
fn facing(heading: f32) -> glam::Vec2 {
    glam::Vec2::new(-heading.sin(), heading.cos())
}

impl Client {
    /// [`aim_on_floor`] against the block's own collision.
    fn on_a_floor(&self, block: u32, sill: Vec3, at: Vec3) -> Vec3 {
        let Ok(coll) = self.assets.block_collision(block) else {
            return at;
        };
        let aim = aim_on_floor(sill, at, |p| coll.world.floor_at(p, 0.6, 1.5).is_some());
        if aim != at {
            tracing::debug!("explore: no floor at {at:?}; aiming at {aim:?} instead");
        }
        aim
    }

    /// With nothing to fight and nothing to loot, and underground: step
    /// into a room we have not been in. Claims the tick while it has
    /// somewhere to go.
    pub fn autoplay_explore(&mut self, now: Instant) -> bool {
        // A journey that passes through somewhere enclosed -- the Town
        // Network hub reads as a dungeon, and so does a dungeon crossed
        // on the way out -- is not an invitation to explore it. Walking
        // to the next room hands the steering a nearby goal, and a
        // nearby goal ends the journey outright: that is how a run to
        // town came through the hub and never left it.
        //
        // Nor is a run to town between journeys an invitation. A corpse on
        // the way ends the run's journey, and a room chosen then would keep
        // the tick from the run's own step for good, round every room and
        // back again: its walk never planned again, its clock never read,
        // and the character exploring and filling up and never selling.
        if self.traveling() || self.autoplay.growth.town_run_under_way() {
            return false;
        }
        let assets = self.assets.clone();
        let Some(pl) = self.player.as_mut() else {
            return false;
        };
        let cell = pl.cell;
        // Outdoors there is nothing to explore this way: the world is
        // one open landblock and the hunting-ground rules cover it.
        if cell & 0xFFFF < 0x100 || !pl.in_dungeon(&assets) {
            return false;
        }
        let me = pl.world_position();
        let facing = facing(pl.heading);
        // Outside the hunting area's dungeon there is nothing here to look
        // for: keeping to the area takes the character back to it. Inside
        // it, a room that is not one of the area's own is walked through
        // on the way to one that is -- bailing out there put the character
        // back in the room it had just left, and round again.
        let elsewhere = match self.autoplay.config.fight.area.as_ref().map(|a| &a.shape) {
            None => false,
            Some(crate::hunt::Shape::Dungeon { landblock, .. }) => {
                cell & 0xFFFF_0000 != landblock & 0xFFFF_0000
            }
            Some(crate::hunt::Shape::Outline { .. }) => true,
        };
        if elsewhere {
            return false;
        }
        let Ok(scene) = ac_scene::landblock::load(&assets, cell & 0xFFFF_0000) else {
            return false;
        };
        // Standing in a room is having explored it, and clears whatever
        // was held against it when it would not let us in before.
        self.autoplay.rooms_seen.insert(cell);
        self.autoplay.rooms_shut.remove(&cell);

        if let Some(RoomWalk {
            from,
            room,
            at,
            pushed,
        }) = self.autoplay.room_bound
        {
            // Arriving is the server putting us in another room, not
            // reaching a point on the floor. Any other room will do:
            // being somewhere new is progress and the next doorway is
            // chosen again from there.
            let arrived = cell != from;
            let too_long = self
                .autoplay
                .room_since
                .is_some_and(|t| now.duration_since(t) > GIVE_UP_ON_A_ROOM);
            if arrived {
                self.autoplay.rooms_seen.insert(room);
                self.forget_the_room();
                return true;
            }
            // Standing at the point aimed at, and still in the room we
            // set out from: the point was not far enough in. Once, it
            // is pushed the same way again; waiting out the clock here
            // was a ten-second stand at every doorway this happened at.
            let there = glam::Vec2::new(at.x - me.x, at.y - me.y).length() <= STOP;
            if there && !pushed {
                let sill = nearest_door(&scene.cells, room, me)
                    .map(|d| d.0)
                    .unwrap_or(at);
                if let Some(further) = way_into(&scene.cells, room, me, facing, PAST_THE_DOOR * 2.0)
                    .map(|p| self.on_a_floor(cell & 0xFFFF_0000, sill, p))
                {
                    tracing::info!("explore: at the sill of {room:#010x} and not in it; aiming further, at {further:?}");
                    self.autoplay.room_bound = Some(RoomWalk {
                        from,
                        room,
                        at: further,
                        pushed: true,
                    });
                    self.head_for(further, STOP, "the next room");
                    return true;
                }
            }
            if too_long || (there && pushed) {
                // Not a room to keep trying: shut for now, and the way
                // on is planned round it.
                self.autoplay.rooms_shut.insert(room);
                self.forget_the_room();
                self.autoplay
                    .note(format!("cannot get into {room:#06x}; going round"), now);
                return true;
            }
            let did = self.head_for(at, STOP, "the next room");
            if did.acting() {
                self.autoplay
                    .say(Doing::Traveling, "looking for something to fight");
                return true;
            }
            self.autoplay.rooms_shut.insert(room);
            self.forget_the_room();
            return true;
        }

        let rooms = Rooms(&scene.cells);
        // With a hunting area of chosen rooms, the others are walked
        // through but never gone to: counted as seen already.
        let outside_area: std::collections::HashSet<u32> =
            match self.autoplay.config.fight.area.as_ref().map(|a| &a.shape) {
                Some(crate::hunt::Shape::Dungeon { rooms: picked, .. }) if !picked.is_empty() => {
                    scene
                        .cells
                        .iter()
                        .map(|c| c.cell_id)
                        .filter(|id| !picked.contains(id))
                        .collect()
                }
                _ => Default::default(),
            };
        let seen = |s: &std::collections::HashSet<u32>| -> std::collections::HashSet<u32> {
            s.union(&outside_area).copied().collect()
        };
        let mut next = next_step(
            cell,
            &rooms,
            &seen(&self.autoplay.rooms_seen),
            &self.autoplay.rooms_shut,
        );
        if next.is_none() {
            // The whole dungeon walked. What we came for has respawned
            // behind us, so it is walked again rather than left, and the
            // rooms that would not open are given another chance.
            self.autoplay.rooms_seen.clear();
            self.autoplay.rooms_shut.clear();
            self.autoplay.rooms_seen.insert(cell);
            next = next_step(
                cell,
                &rooms,
                &seen(&self.autoplay.rooms_seen),
                &self.autoplay.rooms_shut,
            );
        }
        let Some(room) = next else {
            return false;
        };
        let Some(at) = way_into(&scene.cells, room, me, facing, PAST_THE_DOOR) else {
            self.autoplay.rooms_shut.insert(room);
            return true;
        };
        let sill = nearest_door(&scene.cells, room, me)
            .map(|d| d.0)
            .unwrap_or(at);
        let at = self.on_a_floor(cell & 0xFFFF_0000, sill, at);
        tracing::info!("explore: {cell:#010x} -> {room:#010x} at {at:?}");
        self.autoplay.room_bound = Some(RoomWalk {
            from: cell,
            room,
            at,
            pushed: false,
        });
        self.autoplay.room_since = Some(now);
        self.autoplay
            .say(Doing::Traveling, "looking for something to fight");
        self.head_for(at, STOP, "the next room").acting()
    }

    fn forget_the_room(&mut self) {
        self.autoplay.room_bound = None;
        self.autoplay.room_since = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;

    /// The doorway between 0x01F60216 and 0x01F60215 in the Holtburg
    /// Dungeon: a wall along x, the way through along y.
    const SILL: Vec3 = Vec3::new(202.0, 47177.0, 0.1);
    const THROUGH: Vec3 = Vec3::new(0.0, 1.0, 0.0);

    #[test]
    fn the_way_through_is_straight_through_the_wall_whatever_the_approach() {
        // Brynvor, 2026-09-15: from the corner of the room before, the
        // aim along the approach was (200.3, 47178.85), inside the
        // corridor's west wall, and the walk jittered there ten seconds.
        let corner = Vec3::new(205.04, 47173.55, 0.0);
        let at = past_the_door(SILL, THROUGH, corner, Vec2::new(-0.7, 0.7), 2.5);
        assert_eq!(at, Vec3::new(202.0, 47179.5, 0.1));
        // And back the other way from the far side, whichever way the
        // normal happens to point.
        let at = past_the_door(
            SILL,
            -THROUGH,
            Vec3::new(199.0, 47180.0, 0.0),
            Vec2::ZERO,
            2.5,
        );
        assert_eq!(at, Vec3::new(202.0, 47174.5, 0.1));
    }

    #[test]
    fn standing_on_the_sill_the_way_through_is_the_way_we_face() {
        // Bound to the room again from the sill itself, the aim was the
        // sill: "reached" at once, and ten seconds for the room to be
        // written off.
        let on_it = Vec3::new(201.43, 47177.42, 0.0);
        let at = past_the_door(SILL, THROUGH, on_it, Vec2::new(-0.7, 0.7), 2.5);
        assert_eq!(at, Vec3::new(202.0, 47179.5, 0.1));
        let at = past_the_door(SILL, THROUGH, on_it, Vec2::new(0.3, -0.95), 2.5);
        assert_eq!(at, Vec3::new(202.0, 47174.5, 0.1));
    }

    #[test]
    fn without_a_normal_the_approach_line_serves_and_never_degenerates() {
        let far = past_the_door(
            SILL,
            Vec3::ZERO,
            Vec3::new(202.0, 47167.0, 0.0),
            Vec2::X,
            2.5,
        );
        assert_eq!(far, Vec3::new(202.0, 47179.5, 0.1));
        let near = past_the_door(
            SILL,
            Vec3::ZERO,
            Vec3::new(202.0, 47177.3, 0.0),
            Vec2::Y,
            2.5,
        );
        assert!(
            near.distance(SILL) > 2.0,
            "aimed at the sill itself: {near:?}"
        );
    }

    #[test]
    fn an_aim_in_the_wall_is_turned_to_the_corridor_or_drawn_in() {
        // The corridor beyond the (202, 47177) door runs north-east at
        // forty-five degrees: straight through, 2.5 m past the sill, is
        // wall. A floor only within a metre of the line y - 47177 = x - 202.
        let diagonal = |p: Vec3| ((p.y - 47177.0) - (p.x - 202.0)).abs() < 1.0;
        let straight = Vec3::new(202.0, 47179.5, 0.1);
        let aim = aim_on_floor(SILL, straight, diagonal);
        assert!(diagonal(aim), "{aim:?}");
        assert!(
            (aim.distance(SILL) - 2.5).abs() < 0.01,
            "the full distance, turned: {aim:?}"
        );
        assert!(
            aim.x > 203.0 && aim.y > 47178.0,
            "north-east, not south-west: {aim:?}"
        );
        // No corridor either way: drawn in along the line instead.
        let near_only = |p: Vec3| p.distance(SILL) < 1.2;
        let aim = aim_on_floor(SILL, straight, near_only);
        assert!(
            aim.distance(Vec3::new(202.0, 47178.0, 0.1)) < 1e-3,
            "{aim:?}"
        );
        // A floor where aimed is left alone.
        assert_eq!(aim_on_floor(SILL, straight, |_| true), straight);
    }

    #[test]
    fn facing_follows_the_heading_convention() {
        // A walk in +y gives heading atan2(-0, 1) = 0; in +x, atan2(-1, 0).
        let north = facing(0.0);
        assert!((north - Vec2::new(0.0, 1.0)).length() < 1e-6);
        let east = facing((-1.0f32).atan2(0.0));
        assert!((east - Vec2::new(1.0, 0.0)).length() < 1e-6, "{east:?}");
    }
}
