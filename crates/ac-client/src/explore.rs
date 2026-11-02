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
use crate::did::Did;
use crate::{Client, Goal};
use ac_nav::explore::{next_step, Doors};
use ac_scene::interior::CellScene;
use ac_world::Place;
use glam::Vec3;
use std::time::{Duration, Instant};

/// The least time between starting a dungeon over: walked round faster, it is a corner with no
/// way on (Mountain Keep's landing, three rooms out of reach, went round every 7 s).
pub const RESTART_EVERY: Duration = Duration::from_secs(60);

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

/// A walk into the next room: the room set out from, the room, and the place in it walked to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoomWalk {
    pub from: u32,
    pub room: u32,
    pub at: Place,
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

/// Of `spots`, the one in `room`'s own cell nearest `aim` on the map: where a walk into the room
/// ends, on the room's floor however a stair climbs from its sill.
pub fn spot_in_room(spots: &[Place], room: u32, aim: Vec3) -> Option<Place> {
    let off = |p: &Place| p.at.truncate().distance(aim.truncate());
    spots
        .iter()
        .filter(|p| p.cell == room)
        .min_by(|a, b| off(a).total_cmp(&off(b)))
        .copied()
}

/// The way a character with `heading` is pointing, on the map.
/// Headings turn the other way from the maths: see `Player::heading`.
fn facing(heading: f32) -> glam::Vec2 {
    glam::Vec2::new(-heading.sin(), heading.cos())
}

impl Client {
    /// The place a walk into `room` aims for past its sill, at `aim`: [`spot_in_room`] of what
    /// stands within [`PAST_THE_DOOR`], else (none built there yet) `aim` in the room's cell,
    /// which the planner puts on the room's floor or refuses.
    fn room_goal(&mut self, room: u32, aim: Vec3) -> Place {
        let spots = self.standable_near(aim, PAST_THE_DOOR);
        spot_in_room(&spots, room, aim).unwrap_or(Place {
            at: aim,
            cell: room,
        })
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
        // and the character exploring and filling up and never selling. Another row's visit is one
        // too (`grows_visit_across_the_facility_hub_is_not_taken_exploring`).
        if self.traveling()
            || (self.visiting().is_some() && !self.is_own_visit())
            || self.autoplay.growth.town_run_under_way()
        {
            return false;
        }
        // Nor for a follower: its leader walks the rooms, and it keeps up.
        if self.is_led() {
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
        // Nor a dungeon crossed on the way to a ground elsewhere: the ground's walk, a step below,
        // is taken up again (a_dungeon_crossed_on_the_way_to_a_ground_is_not_explored_in_its_place).
        if self
            .autoplay
            .growth
            .bound_landblock()
            .is_some_and(|lb| lb != cell >> 16)
        {
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
        let new_here = self.autoplay.rooms_seen.insert(cell);
        self.autoplay.rooms_shut.remove(&cell);

        if let Some(RoomWalk { from, room, at }) = self.autoplay.room_bound {
            // Arriving is the server putting us in the room, or in one not walked yet (progress, the
            // next doorway chosen from there); a walked room on the route is not, or 0x21A and 0x216
            // took turns (exploring_does_not_pace_between_two_rooms).
            if cell == room || (cell != from && new_here) {
                self.autoplay.rooms_seen.insert(room);
                self.autoplay.room_bound = None;
                return true;
            }
            let did = self.walk_into(at, now);
            if !matches!(did, Did::Done | Did::Blocked(_)) {
                self.autoplay
                    .say(Doing::Traveling, "looking for something to fight");
                return true;
            }
            // Not a room to keep trying: shut for now, and the way on is planned round it.
            self.autoplay.rooms_shut.insert(room);
            self.autoplay.room_bound = None;
            self.autoplay
                .note(format!("cannot get into {room:#06x}; going round"), now);
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
        // A room no walk gets into is shut and the next chosen in the same tick; each pass shuts
        // one, so the rooms run out.
        loop {
            let mut next = next_step(
                cell,
                &rooms,
                &seen(&self.autoplay.rooms_seen),
                &self.autoplay.rooms_shut,
            );
            if next.is_none() {
                // Walked round again within the minute: a corner with no way on, where starting
                // over only goes round the shut rooms on the spot. The rules below have the tick.
                if self
                    .autoplay
                    .rooms_restarted
                    .is_some_and(|t| now.saturating_duration_since(t) < RESTART_EVERY)
                {
                    return false;
                }
                self.autoplay.rooms_restarted = Some(now);
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
            let Some(aim) = way_into(&scene.cells, room, me, facing, PAST_THE_DOOR) else {
                self.autoplay.rooms_shut.insert(room);
                continue;
            };
            let at = self.room_goal(room, aim);
            // A room is gone into only when a walk gets there: the doorway of 0x01F60296 opens 6 m
            // up a wall (exploring_the_holtburg_dungeon_does_not_stand_at_the_bookcases).
            if let Did::Blocked(why) = self.walk_into(at, now) {
                self.autoplay.rooms_shut.insert(room);
                self.autoplay.note(
                    format!("no way into {room:#06x} ({}); going round", why.what),
                    now,
                );
                continue;
            }
            tracing::info!("explore: {cell:#010x} -> {room:#010x} at {:?}", at.at);
            self.autoplay.room_bound = Some(RoomWalk {
                from: cell,
                room,
                at,
            });
            self.autoplay
                .say(Doing::Traveling, "looking for something to fight");
            return true;
        }
    }

    /// Walk to `place`, in the room past a threshold, to within [`STOP`].
    fn walk_into(&mut self, place: Place, now: Instant) -> Did {
        let goal = Goal::At {
            place,
            within: STOP,
        };
        self.go(self.step_owner(), goal, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;

    /// Autoplay and the body run offline together, a tick at a time, for `seconds`: where the
    /// character stood each second.
    fn play(c: &mut Client, seconds: u32) -> Vec<glam::Vec3> {
        let dt = 0.05;
        let t0 = Instant::now();
        let mut stood = Vec::new();
        for frame in 0..seconds * 20 {
            let now = t0 + std::time::Duration::from_secs_f32(frame as f32 * dt);
            c.tick_autoplay(now);
            c.tick_player(crate::player::Input::default(), dt, now);
            if frame % 20 == 0 {
                stood.push(c.my_position().unwrap());
            }
        }
        stood
    }

    #[test]
    #[ignore = "needs AC_DATA_DIR"]
    fn exploring_the_holtburg_dungeon_does_not_stand_at_the_bookcases() {
        // Blargerton, exploring, walked into the bookcases in 0x01F60233 again and again: the next
        // rooms, 0x295 and 0x296, open off a doorway 6 m up the wall, no path reaches it, and the
        // steering leaned on the straight line to it.
        let mut c = crate::testkit::standing_in_the_field(
            26,
            0x01F6_0233,
            glam::Vec3::new(35.19, -49.43, 0.0),
        );
        c.world.player_guid = Some(crate::testkit::ME);
        c.autoplay.config.enabled = true;
        c.autoplay.config.fight.enabled = true;
        // Most of the dungeon walked already, as it was: the rooms left are the two up the wall.
        let scene = ac_scene::landblock::load(&c.assets, 0x01F6_0000).unwrap();
        for cs in &scene.cells {
            if cs.cell_id & 0xFFFF != 0x295 && cs.cell_id & 0xFFFF != 0x296 {
                c.autoplay.rooms_seen.insert(cs.cell_id);
            }
        }
        let stood = play(&mut c, 90);
        assert_keeps_going(&c, &stood);
    }

    /// In every twenty seconds of `stood` (a place a second), the character gets six metres from
    /// where it began them: neither standing nor pacing between two doorways, which a
    /// ten-seconds-in-one-place test let through.
    fn assert_keeps_going(c: &Client, stood: &[glam::Vec3]) {
        let origin = ac_world::landblock_origin(0x01F6_0000);
        for w in stood.windows(20) {
            let moved = w.iter().map(|p| p.distance(w[0])).fold(0.0f32, f32::max);
            assert!(
                moved >= 6.0,
                "kept within {moved:.1} m of {:?} for twenty seconds, bound for {:?}",
                w[0] - origin,
                c.autoplay
                    .room_bound
                    .map(|r| (format!("{:#x}", r.room & 0xFFFF), r.at.at - origin))
            );
        }
    }

    #[test]
    #[ignore = "needs AC_DATA_DIR"]
    fn exploring_does_not_pace_between_two_rooms() {
        // Blargerton, 172 s in the Holtburg Dungeon: bound for 0x21B from 0x21A, the route went
        // back through 0x216 first, and stepping into 0x216 counted as arriving; from there the
        // way on was 0x21A again. Round every 0.8 s.
        let mut c = crate::testkit::standing_in_the_field(
            27,
            0x01F6_021A,
            glam::Vec3::new(16.5, -61.5, 0.0),
        );
        c.world.player_guid = Some(crate::testkit::ME);
        c.autoplay.config.enabled = true;
        c.autoplay.config.fight.enabled = true;
        let scene = ac_scene::landblock::load(&c.assets, 0x01F6_0000).unwrap();
        for cs in &scene.cells {
            if cs.cell_id & 0xFFFF != 0x21B {
                c.autoplay.rooms_seen.insert(cs.cell_id);
            }
        }
        let stood = play(&mut c, 90);
        assert_keeps_going(&c, &stood);
    }

    #[test]
    #[ignore = "needs AC_DATA_DIR"]
    fn exploring_climbs_the_stairs() {
        // On the stairs 0x01F602A3, whose sills are 6 m apart in height: an aim at a sill's height
        // past it had no floor, was drawn back onto the sill, and read as no way into either room.
        let mut c = crate::testkit::standing_in_the_field(
            27,
            0x01F6_02A3,
            glam::Vec3::new(94.5, -79.5, 0.3),
        );
        c.world.player_guid = Some(crate::testkit::ME);
        c.autoplay.config.enabled = true;
        c.autoplay.config.fight.enabled = true;
        let stood = play(&mut c, 60);
        assert_keeps_going(&c, &stood);
    }

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
    fn facing_follows_the_heading_convention() {
        // A walk in +y gives heading atan2(-0, 1) = 0; in +x, atan2(-1, 0).
        let north = facing(0.0);
        assert!((north - Vec2::new(0.0, 1.0)).length() < 1e-6);
        let east = facing((-1.0f32).atan2(0.0));
        assert!((east - Vec2::new(1.0, 0.0)).length() < 1e-6, "{east:?}");
    }
}
