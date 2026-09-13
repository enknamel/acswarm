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

/// Where to walk to to be in `room`: through the opening between us and
/// it, and a couple of paces beyond, so that the character ends up
/// inside rather than standing in the doorway.
fn way_into(cells: &[CellScene], room: u32, from: Vec3) -> Option<Vec3> {
    let cell = cells.iter().find(|c| c.cell_id == room)?;
    let Some(sill) = cell
        .doorways
        .iter()
        .copied()
        .min_by(|a, b| a.distance(from).total_cmp(&b.distance(from)))
    else {
        // No opening in the data: the middle of the cell structure is
        // the best guess there is.
        return Some(cell.transform.transform_point3(Vec3::ZERO));
    };
    let across = glam::Vec2::new(sill.x - from.x, sill.y - from.y);
    let on = if across.length() > 0.1 {
        across.normalize() * PAST_THE_DOOR
    } else {
        glam::Vec2::ZERO
    };
    Some(Vec3::new(sill.x + on.x, sill.y + on.y, sill.z))
}

impl Client {
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
        if self.traveling() {
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

        if let Some((from, room, at)) = self.autoplay.room_bound {
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
            if too_long {
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
        let Some(at) = way_into(&scene.cells, room, me) else {
            self.autoplay.rooms_shut.insert(room);
            return true;
        };
        tracing::info!("explore: {cell:#010x} -> {room:#010x} at {at:?}");
        self.autoplay.room_bound = Some((cell, room, at));
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
