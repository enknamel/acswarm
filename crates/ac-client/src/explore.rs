//! Walking a dungeon to find the fight in it: the server describes only the cell the character
//! stands in and the cells visible from it (Holtburg Dungeon: 58 creatures in 74 rooms, none in its
//! portal room, whose two generators are server-side and never described), so waiting finds none.
//! Which doorway is [`ac_nav::explore::next_step`]; this half reads the cells and their thresholds
//! and walks into the room chosen, to [`spot_in_room`].

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

/// How far past a threshold to aim (metres): stood on the line itself, the server goes on
/// describing the room the character came from, and no other.
const PAST_THE_DOOR: f32 = 2.5;

/// The walk into a room ends within this of its spot (metres, 3D), or within half the spot's depth
/// past the sill when that is less ([`within_room`]).
const STOP: f32 = 1.0;

/// The least a walk into a room ends within (metres): a steady stop, not an orbit of the spot.
const STOP_AT_LEAST: f32 = 0.25;

/// The most rooms tried in one tick; the rest wait for the next (the frame is the player's too).
const ROOMS_PER_TICK: usize = 3;

/// The dungeon's rooms and what opens on to what.
struct Rooms<'a>(&'a [CellScene]);

impl Doors for Rooms<'_> {
    fn beyond(&self, cell: u32) -> Vec<u32> {
        let Some(room) = self.0.iter().find(|c| c.cell_id == cell) else {
            return Vec::new();
        };
        // Only openings into this dungeon's rooms: a portal outdoors is the way out, and names a
        // cell not in this list.
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
    /// How near `at` the walk ends: inside the room, past the sill (`within_room`).
    pub within: f32,
}

/// `past` metres beyond `sill` along the wall's `normal` (on the map, either sign), on the far side
/// from `from`, else (on the sill) the side `room_at` lies on, else the way `facing` points.
fn past_the_door(
    sill: Vec3,
    normal: Vec3,
    from: Vec3,
    (room_at, facing): (Vec3, glam::Vec2),
    past: f32,
) -> Vec3 {
    let across = glam::Vec2::new(sill.x - from.x, sill.y - from.y);
    let n = glam::Vec2::new(normal.x, normal.y);
    let through = if n.length() > 0.5 {
        let side = across.dot(n);
        let room = (room_at - sill).truncate().dot(n);
        let ahead = facing.dot(n);
        if side.abs() >= STOP * 0.5 {
            n * side.signum()
        } else if room.abs() > STOP * 0.5 {
            n * room.signum()
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

/// Where to walk to be in `room`, and the sill crossed: [`past_the_door`] of its doorway nearest
/// `from` (the room's middle, else `facing`, decides on the sill), or the middle of the cell, as both,
/// when the data has no opening.
fn way_into(
    cells: &[CellScene],
    room: u32,
    from: Vec3,
    facing: glam::Vec2,
    past: f32,
) -> Option<(Vec3, Vec3)> {
    let (sill, normal) = nearest_door(cells, room, from)?;
    let cell = cells.iter().find(|c| c.cell_id == room)?;
    if normal == Vec3::ZERO && cell.doorways.is_empty() {
        // No opening in the data: the middle of the cell structure is
        // the best guess there is.
        return Some((sill, sill));
    }
    let middle = cell.transform.transform_point3(Vec3::ZERO);
    Some((
        past_the_door(sill, normal, from, (middle, facing), past),
        sill,
    ))
}

/// How near `spot` a walk through `sill` toward `aim` ends so it ends in the room: [`STOP`], or half
/// the spot's depth past the sill (a 2.5 m room's only spot may be 0.5 m in), at least `STOP_AT_LEAST`.
fn within_room(sill: Vec3, aim: Vec3, spot: Vec3) -> f32 {
    let inward = (aim - sill).truncate();
    if inward.length() < 0.1 {
        return STOP;
    }
    let depth = (spot - sill).truncate().dot(inward.normalize());
    if depth <= 0.0 {
        return STOP;
    }
    (depth * 0.5).clamp(STOP_AT_LEAST, STOP)
}

/// Of `spots`, the one in `room`'s own cell nearest `aim` on the map (on its floor however a stair
/// climbs), never within `STOP` of `from`: that walk arrives unwalked, still on the sill.
pub fn spot_in_room(spots: &[Place], room: u32, aim: Vec3, from: Vec3) -> Option<Place> {
    let off = |p: &Place| p.at.truncate().distance(aim.truncate());
    spots
        .iter()
        .filter(|p| p.cell == room && p.at.distance(from) > STOP)
        .min_by(|a, b| off(a).total_cmp(&off(b)))
        .copied()
}

/// The way a character with `heading` is pointing, on the map.
/// Headings turn the other way from the maths: see `Player::heading`.
fn facing(heading: f32) -> glam::Vec2 {
    glam::Vec2::new(-heading.sin(), heading.cos())
}

impl Client {
    /// Underground with nothing to fight or loot: walk into a room not yet stood in. Claims the
    /// tick while it has somewhere to go.
    pub fn autoplay_explore(&mut self, now: Instant) -> bool {
        // Not on a journey (the hub, a dungeon crossed), another row's visit or a town run: a
        // room takes that row's tick (grows_visit_across_the_facility_hub_is_not_taken_exploring).
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
        // Nor while a quest step is worked but a hunt on this ground: the stone for the Vortex
        // was carried round the Mines (a_quest_step_elsewhere_is_not_taken_exploring).
        let here = self.player.as_ref().map_or(0, |p| p.cell >> 16);
        let hunting_here = self.is_pursued_hunt() && self.pursued_place(here).is_some();
        if self.is_pursuing() && !hunting_here {
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
        // Outside the area's dungeon, keeping to the area takes it back; inside, a room not the
        // area's own is walked through to one that is (stopping there went back and round again).
        let elsewhere = match self.hunting_area().map(|a| &a.shape) {
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

        if let Some(RoomWalk {
            from,
            room,
            at,
            within,
        }) = self.autoplay.room_bound
        {
            // Arrived: in the room, or in one not walked yet, where the next doorway is chosen; not
            // in a walked room on the way (exploring_does_not_pace_between_two_rooms).
            if cell == room || (cell != from && new_here) {
                self.autoplay.rooms_seen.insert(room);
                self.autoplay.room_bound = None;
                return true;
            }
            let did = self.walk_into(at, within, now);
            if !matches!(did, Did::Done | Did::Blocked(_)) {
                self.autoplay
                    .say(Doing::Traveling, "looking for something to fight");
                return true;
            }
            // At its spot: the room is stood in, whichever of two overlapping cells the body says
            // (Mossy Cave's 0x0178 over 0x0179, which six rooms open off: shut, it cut them off).
            let at_spot = self
                .my_position()
                .is_some_and(|p| p.distance(at.at) <= within.max(STOP));
            if matches!(did, Did::Done) && at_spot {
                self.autoplay.rooms_seen.insert(room);
                self.autoplay.room_bound = None;
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
        let outside_area: std::collections::HashSet<u32> = match self
            .hunting_area()
            .map(|a| &a.shape)
        {
            Some(crate::hunt::Shape::Dungeon { rooms: picked, .. }) if !picked.is_empty() => scene
                .cells
                .iter()
                .map(|c| c.cell_id)
                .filter(|id| !picked.contains(id))
                .collect(),
            _ => Default::default(),
        };
        let seen = |s: &std::collections::HashSet<u32>| -> std::collections::HashSet<u32> {
            s.union(&outside_area).copied().collect()
        };
        // A room no walk gets into is shut and the next chosen in the same tick, up to
        // ROOMS_PER_TICK: each costs a floor build and a plan, and Mossy Cave's ran a frame minutes.
        // Where the way on is looked for from: this room, or a sliver of one stood in already.
        let mut from = cell;
        for _ in 0..ROOMS_PER_TICK {
            let mut next = next_step(
                from,
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
                // The whole dungeon walked: what we came for has respawned behind us, so it is
                // walked again, and the rooms that would not open are tried again.
                self.autoplay.rooms_seen.clear();
                self.autoplay.rooms_shut.clear();
                self.autoplay.rooms_seen.insert(cell);
                next = next_step(
                    from,
                    &rooms,
                    &seen(&self.autoplay.rooms_seen),
                    &self.autoplay.rooms_shut,
                );
            }
            let Some(room) = next else {
                return false;
            };
            let Some((aim, sill)) = way_into(&scene.cells, room, me, facing, PAST_THE_DOOR) else {
                self.autoplay.rooms_shut.insert(room);
                continue;
            };
            // Not built round the aim yet: asked again next tick, the room neither guessed at nor
            // shut. Built, a room with no standing spot of its own past the sill is shut.
            let Some(spots) = self.standable_near(aim, PAST_THE_DOOR) else {
                self.autoplay
                    .say(Doing::Traveling, "looking for something to fight");
                return true;
            };
            let Some(at) = spot_in_room(&spots, room, aim, me) else {
                // Its floor only where we stand, or its floor this room's (two cells over one
                // floor): stood in already, the way on looked for from it (Mossy Cave's 0x0112
                // walled 0x011A in, 0x0178 over 0x0179 cut off the six rooms beyond).
                let shared = |p: &Place| p.cell == cell && p.at.distance(aim) <= STOP;
                if spots
                    .iter()
                    .any(|p| p.cell == room && p.at.distance(me) <= STOP)
                    || (!spots.iter().any(|p| p.cell == room) && spots.iter().any(shared))
                {
                    self.autoplay.rooms_seen.insert(room);
                    from = room;
                    continue;
                }
                self.autoplay.rooms_shut.insert(room);
                self.autoplay.note(
                    format!("no floor past the door of {room:#06x}; going round"),
                    now,
                );
                continue;
            };
            // A room is gone into only when a walk gets there: the doorway of 0x01F60296 opens 6 m
            // up a wall (exploring_the_holtburg_dungeon_does_not_stand_at_the_bookcases).
            let within = within_room(sill, aim, at.at);
            if let Did::Blocked(why) = self.walk_into(at, within, now) {
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
                within,
            });
            self.autoplay
                .say(Doing::Traveling, "looking for something to fight");
            return true;
        }
        // Out of tries this tick: the next room is chosen on the next.
        self.autoplay
            .say(Doing::Traveling, "looking for something to fight");
        true
    }

    /// Walk to `place`, in the room past a threshold, to `within` (see [`within_room`]).
    fn walk_into(&mut self, place: Place, within: f32, now: Instant) -> Did {
        let goal = Goal::At { place, within };
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

    #[test]
    #[ignore = "needs AC_DATA_DIR"]
    fn exploring_the_gredaline_consulate_leaves_its_arrival_room() {
        // Blargerton, through the Gredaline Consulate portal for its Tusker Guards: the journey
        // ended in 0x029A01F1, explore walked to 0x1F2, and the server set it down on their sill.
        // Aimed the way it faced, back into 0x1F2, the room ahead was shut and it stood an hour.
        let mut c = crate::testkit::standing_in_the_field(
            41,
            0x029A_01F1,
            glam::Vec3::new(169.97, -220.48, 0.0),
        );
        c.world.player_guid = Some(crate::testkit::ME);
        c.autoplay.config.enabled = true;
        c.autoplay.config.fight.enabled = true;
        play(&mut c, 1);
        crate::testkit::stand(&mut c, 0x029A_01F2, glam::Vec3::new(169.63, -225.29, 0.0));
        play(&mut c, 60);
        let seen = c.autoplay.rooms_seen.len();
        assert!(
            seen >= 10,
            "explored {seen} rooms, shut {:x?}",
            c.autoplay.rooms_shut
        );
    }

    #[test]
    #[ignore = "needs AC_DATA_DIR"]
    fn exploring_the_black_spawn_den_goes_on_past_its_first_rooms() {
        // Blargerton, at the Black Spawn Den for its Tusker Guards: after a Tusker Slave, every
        // room round 0x0104_0378 was "cannot get into" or "no floor past the door", and it stood.
        let mut c = crate::testkit::standing_in_the_field(
            41,
            0x0104_0378,
            glam::Vec3::new(109.26, -143.77, 0.0),
        );
        c.world.player_guid = Some(crate::testkit::ME);
        c.autoplay.config.enabled = true;
        c.autoplay.config.fight.enabled = true;
        play(&mut c, 60);
        let seen = c.autoplay.rooms_seen.len();
        assert!(
            seen >= 10,
            "explored {seen} rooms, shut {:x?}",
            c.autoplay.rooms_shut
        );
    }

    #[test]
    #[ignore = "needs AC_DATA_DIR"]
    fn a_quests_dungeon_is_explored_under_a_hunting_area_elsewhere() {
        // Blargerton, hunting Burun Ruuk Scamps in Mossy Cave for the Necklace of Leerargh's key
        // with the Tusker Barracks as his area: the scamps in sight killed, he stood, the rooms
        // being outside the area; then walled into 0x011A by its sliver of a doorway, 0x0112.
        let mut c = crate::testkit::standing_in_the_field(
            51,
            0x0177_0176,
            glam::Vec3::new(48.33, -81.29, -2.02),
        );
        c.world.player_guid = Some(crate::testkit::ME);
        c.autoplay.config.enabled = true;
        c.autoplay.config.fight.enabled = true;
        c.autoplay.config.fight.area = Some(crate::hunt::HuntArea {
            name: "tusker barracks".into(),
            shape: crate::hunt::Shape::Dungeon {
                landblock: 0x5A48_0000,
                rooms: Vec::new(),
            },
        });
        c.autoplay.config.quests.on = true;
        c.autoplay
            .config
            .quests
            .pursue_quest("magicitemtraining3", true, 0);
        c.refresh_quests(Instant::now());
        assert!(c.is_pursuing(), "{:?}", c.pursuit);
        let stood = play(&mut c, 60);
        assert_keeps_going(&c, &stood);
    }

    #[test]
    #[ignore = "needs AC_DATA_DIR"]
    fn a_quest_step_elsewhere_is_not_taken_exploring() {
        // Blargerton in the Mines of Despair with the Focusing Stone for the Vortex: the explorer,
        // ranked above growing, walked the rooms and the pursuit never set off.
        let mut c = crate::testkit::standing_in_the_field(
            51,
            0x0177_0176,
            glam::Vec3::new(48.33, -81.29, -2.02),
        );
        c.world.player_guid = Some(crate::testkit::ME);
        c.autoplay.config.enabled = true;
        let row = ac_world::quests::rows_of("magicitemtraining3")
            .find(|r| r.kind == ac_world::quests::Kind::Item)
            .unwrap();
        assert!(
            c.autoplay_explore(Instant::now()),
            "nothing pursued: explored"
        );
        c.autoplay.room_bound = None;
        c.pursuit = vec![crate::autoplay::PursuitLine {
            id: row.id.into(),
            title: row.title.into(),
            step: crate::autoplay::PursuitStep::HandIn {
                row,
                at: row.places[0],
            },
            working: true,
            via: None,
        }];
        assert!(
            !c.autoplay_explore(Instant::now()),
            "explored with a hand-in to make"
        );
    }

    #[test]
    fn a_walk_into_a_shallow_room_ends_past_its_sill() {
        let sill = Vec3::new(110.0, -145.0, 0.0);
        let aim = Vec3::new(110.0, -147.5, 0.0);
        // The Black Spawn Den's 2.5 m rooms: the one spot in 0x037C is 0.5 m past the sill.
        let shallow = within_room(sill, aim, Vec3::new(109.5, -145.5, 0.0));
        assert!(shallow < 0.5, "{shallow} lets the walk stop on the sill");
        assert_eq!(within_room(sill, aim, Vec3::new(110.0, -148.0, 0.0)), STOP);
        assert_eq!(within_room(sill, sill, Vec3::new(110.0, -146.0, 0.0)), STOP);
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
        let at = past_the_door(SILL, THROUGH, corner, (SILL, Vec2::new(-0.7, 0.7)), 2.5);
        assert_eq!(at, Vec3::new(202.0, 47179.5, 0.1));
        // And back the other way from the far side, whichever way the
        // normal happens to point.
        let at = past_the_door(
            SILL,
            -THROUGH,
            Vec3::new(199.0, 47180.0, 0.0),
            (SILL, Vec2::ZERO),
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
        let at = past_the_door(SILL, THROUGH, on_it, (SILL, Vec2::new(-0.7, 0.7)), 2.5);
        assert_eq!(at, Vec3::new(202.0, 47179.5, 0.1));
        let at = past_the_door(SILL, THROUGH, on_it, (SILL, Vec2::new(0.3, -0.95)), 2.5);
        assert_eq!(at, Vec3::new(202.0, 47174.5, 0.1));
    }

    #[test]
    fn standing_on_the_sill_the_way_through_is_the_side_the_room_is_on() {
        // Set down by a portal on the sill facing back the way it had come, the aim was the room
        // stood in, and the room ahead was shut (exploring_the_gredaline_consulate_leaves_its_arrival_room).
        let on_it = Vec3::new(201.43, 47177.42, 0.0);
        let room = Vec3::new(202.0, 47182.0, 0.0);
        let at = past_the_door(SILL, THROUGH, on_it, (room, Vec2::new(0.3, -0.95)), 2.5);
        assert_eq!(at, Vec3::new(202.0, 47179.5, 0.1));
    }

    #[test]
    fn without_a_normal_the_approach_line_serves_and_never_degenerates() {
        let far = past_the_door(
            SILL,
            Vec3::ZERO,
            Vec3::new(202.0, 47167.0, 0.0),
            (SILL, Vec2::X),
            2.5,
        );
        assert_eq!(far, Vec3::new(202.0, 47179.5, 0.1));
        let near = past_the_door(
            SILL,
            Vec3::ZERO,
            Vec3::new(202.0, 47177.3, 0.0),
            (SILL, Vec2::Y),
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
