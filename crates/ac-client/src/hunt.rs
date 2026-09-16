//! Where to hunt: an area a player draws on the map.
//!
//! Players do not hunt "near here". They hunt a place -- a dungeon, the
//! graveyard, the volcano on Singularity Caul -- and do not want the time
//! spent on lesser things that wander at its edges. So a hunting area is
//! a shape with a name:
//!
//! - an **outline** drawn on the map, its corners in world xy, for a place
//!   out in the open (buildings inside it count as inside);
//! - a **dungeon**, the whole of it or only the rooms chosen on its map.
//!
//! Areas are saved by the map panel and used by whatever hunts in them;
//! this is the shape and the one question asked of it: is this inside?

use std::time::Instant;

use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};

use crate::autoplay::Doing;
use crate::Client;

/// Something hitting the character is fought if it stands this near,
/// wherever that is: an area says where to look for fights, not which
/// ones to lose.
pub const DEFEND_REACH: f32 = 30.0;
/// Walking back into an outline stops this close to the way in.
const BACK_IN: f32 = 3.0;

/// A named place to hunt.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HuntArea {
    pub name: String,
    pub shape: Shape,
}

/// What a hunting area covers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Shape {
    /// An outline on the map: its corners in world xy, in order.
    Outline { points: Vec<[f32; 2]> },
    /// A dungeon (its landblock, `xxyy0000`): every room when `rooms` is
    /// empty, otherwise only those cells.
    Dungeon { landblock: u32, rooms: Vec<u32> },
}

/// The fewest corners that enclose anything.
pub const LEAST_CORNERS: usize = 3;

impl HuntArea {
    /// Whether something at world position `at`, in cell `cell`, is
    /// inside the area. `underground` says whether that cell is a
    /// dungeon's, which the cell id alone cannot: a dungeon shares its
    /// landblock numbering with the ground above, and only the one who is
    /// there knows which it is in.
    pub fn contains(&self, at: Vec3, cell: u32, underground: bool) -> bool {
        match &self.shape {
            Shape::Outline { points } => {
                let corners: Vec<Vec2> = points.iter().map(|p| Vec2::from(*p)).collect();
                // An outline is drawn on the surface map: a dungeon
                // underneath it is somewhere else.
                !underground && inside(at.truncate(), &corners)
            }
            Shape::Dungeon { landblock, rooms } => {
                cell & 0xFFFF_0000 == landblock & 0xFFFF_0000
                    && cell & 0xFFFF >= 0x100
                    && (rooms.is_empty() || rooms.contains(&cell))
            }
        }
    }

    /// Whether the shape is complete enough to save: an outline needs at
    /// least [`LEAST_CORNERS`] corners.
    pub fn usable(&self) -> bool {
        match &self.shape {
            Shape::Outline { points } => points.len() >= LEAST_CORNERS,
            Shape::Dungeon { .. } => true,
        }
    }

    /// A few words for a list: "outline, 5 corners", "dungeon 0x01F6, 3
    /// rooms", "dungeon 0x01F6, all of it".
    pub fn describe(&self) -> String {
        match &self.shape {
            Shape::Outline { points } => format!("outline, {} corners", points.len()),
            Shape::Dungeon { landblock, rooms } if rooms.is_empty() => {
                format!("dungeon {:#06x}, all of it", landblock >> 16)
            }
            Shape::Dungeon { landblock, rooms } => {
                format!("dungeon {:#06x}, {} rooms", landblock >> 16, rooms.len())
            }
        }
    }
}

/// Whether `p` lies inside the polygon `corners` (in order, either way
/// round; the last joins the first). Crossing count: a line from `p`
/// crosses the edge of the polygon an odd number of times from inside.
pub fn inside(p: Vec2, corners: &[Vec2]) -> bool {
    if corners.len() < LEAST_CORNERS {
        return false;
    }
    let mut inside = false;
    let mut j = corners.len() - 1;
    for i in 0..corners.len() {
        let (a, b) = (corners[i], corners[j]);
        if (a.y > p.y) != (b.y > p.y) {
            let x = a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y);
            if p.x < x {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// The corners of an outline area, `None` for a dungeon.
pub fn corners(area: &HuntArea) -> Option<Vec<Vec2>> {
    match &area.shape {
        Shape::Outline { points } => Some(points.iter().map(|p| Vec2::from(*p)).collect()),
        Shape::Dungeon { .. } => None,
    }
}

/// Where to walk to get inside an outline: its middle when that is inside
/// it, and otherwise somewhere that is (an L or a crescent can have its
/// middle outside).
pub fn way_in(corners: &[Vec2]) -> Vec2 {
    let n = corners.len().max(1) as f32;
    let middle = corners.iter().copied().sum::<Vec2>() / n;
    if inside(middle, corners) {
        return middle;
    }
    // Between each corner and the middle, the first point found inside.
    corners
        .iter()
        .flat_map(|c| [0.25f32, 0.5, 0.75].map(|t| c.lerp(middle, t)))
        .find(|p| inside(*p, corners))
        .unwrap_or(middle)
}

/// The `n`th place to walk to while looking about an outline for
/// something to fight: towards each corner in turn, well inside it.
pub fn patrol_point(corners: &[Vec2], n: u32) -> Vec2 {
    let middle = way_in(corners);
    if corners.is_empty() {
        return middle;
    }
    let toward = corners[n as usize % corners.len()];
    [0.7f32, 0.5, 0.3]
        .into_iter()
        .map(|t| middle.lerp(toward, t))
        .find(|p| inside(*p, corners))
        .unwrap_or(middle)
}

/// The portal into the dungeon of `landblock` nearest `from`: one that
/// works, stands out in the open, and comes out in that dungeon.
pub fn entrance(landblock: u32, from: Vec2) -> Option<&'static ac_world::portals::Portal> {
    ac_world::portals::all()
        .iter()
        .filter(|p| p.works() && p.mouth_outdoors())
        .filter(|p| p.to_cell & 0xFFFF_0000 == landblock & 0xFFFF_0000)
        .min_by(|a, b| {
            a.from_xy()
                .distance(from)
                .total_cmp(&b.from_xy().distance(from))
        })
}

impl Client {
    /// Whether the character stands in a dungeon (a building on the
    /// surface is not one).
    pub(crate) fn underground(&mut self) -> bool {
        let assets = self.assets.clone();
        self.player
            .as_mut()
            .is_some_and(|pl| pl.is_indoors() && pl.in_dungeon(&assets))
    }

    /// Whether `o` may be fought under the hunting area: it stands inside
    /// the area, or it is what has just swung at the character and is
    /// near. With no area, anything may.
    pub(crate) fn area_allows(&self, o: &ac_world::WorldObject, underground: bool) -> bool {
        let Some(area) = &self.autoplay.config.fight.area else {
            return true;
        };
        let Some(p) = o.position else {
            return false;
        };
        let at = ac_world::landblock_origin(p.cell) + p.local;
        if area.contains(at, p.cell, underground) {
            return true;
        }
        let near = self
            .my_position()
            .is_some_and(|me| me.distance(at) <= DEFEND_REACH);
        near && self.hit_lately_by(&o.name)
    }

    /// Whether the object `guid` may still be fought under the area (see
    /// [`Client::area_allows`]); something no longer known may not.
    pub(crate) fn area_allows_guid(&self, guid: u32, underground: bool) -> bool {
        self.world
            .objects
            .get(&guid)
            .is_some_and(|o| self.area_allows(o, underground))
    }

    /// Keep to the hunting area: outside it, and not in a fight or on the
    /// way somewhere already, go back. Into an outline on foot; to a
    /// dungeon through the portal into it; within the dungeon, to the
    /// nearest room picked. True while it does.
    pub(crate) fn autoplay_keep_to_area(&mut self, now: Instant) -> bool {
        let Some(area) = self.autoplay.config.fight.area.clone() else {
            return false;
        };
        if !self.autoplay.config.fight.enabled {
            return false;
        }
        // Nor on a run to town, which leaves the area on purpose. At the
        // counter, or between journeys once a corpse or a fight on the way
        // has ended one, a walk back to the area would take the character
        // off its run, and the run's own step, ranked below this one, would
        // never see it again.
        if self.autoplay.growth.town_run_under_way() {
            return false;
        }
        // Never in the middle of a fight: something hitting the character
        // is fought where it stands, and so is what it has taken on.
        if self.under_attack()
            || self.attack_target.is_some()
            || self.autoplay.casting_at().is_some()
        {
            return false;
        }
        let Some((me, cell)) = self.player.as_ref().map(|p| (p.world_position(), p.cell)) else {
            return false;
        };
        let underground = self.underground();
        if area.contains(me, cell, underground) {
            return false;
        }
        // Already on the way: travel and visits see it through.
        if self.traveling() || self.visiting().is_some() {
            return true;
        }
        match &area.shape {
            Shape::Outline { points } => {
                let corners: Vec<Vec2> = points.iter().map(|p| Vec2::from(*p)).collect();
                if corners.len() < LEAST_CORNERS {
                    return false;
                }
                let goal = way_in(&corners);
                if !self
                    .head_for(goal.extend(me.z), BACK_IN, &area.name)
                    .acting()
                {
                    self.autoplay
                        .note(format!("no way back to {} from here", area.name), now);
                    return false;
                }
                self.autoplay
                    .say(Doing::Traveling, format!("back to {}", area.name));
                true
            }
            Shape::Dungeon { landblock, .. } => {
                if underground && cell & 0xFFFF_0000 == landblock & 0xFFFF_0000 {
                    // In the dungeon, outside the rooms picked: the explorer
                    // takes it from here, through these rooms to the picked
                    // ones and never to the others. Walking back to the
                    // nearest picked room as well pulled the character out
                    // of each doorway the explorer took it through, and the
                    // two held it on the threshold for two minutes.
                    return false;
                }
                let Some(portal) = entrance(*landblock, me.truncate()) else {
                    self.autoplay
                        .note(format!("no portal into {} is known", area.name), now);
                    return false;
                };
                if !self.visit_portal(portal) {
                    self.autoplay
                        .note(format!("no way to the portal into {}", area.name), now);
                    return false;
                }
                self.autoplay.say(
                    Doing::Traveling,
                    format!("on the way into {} ({})", area.name, portal.name),
                );
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_way_into_an_outline_is_inside_it() {
        let square = [
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 10.0),
            Vec2::new(0.0, 10.0),
        ];
        assert_eq!(way_in(&square), Vec2::new(5.0, 5.0));
        // A thin crescent-like L whose middle falls outside it.
        let l = [
            Vec2::new(0.0, 0.0),
            Vec2::new(100.0, 0.0),
            Vec2::new(100.0, 10.0),
            Vec2::new(10.0, 10.0),
            Vec2::new(10.0, 100.0),
            Vec2::new(0.0, 100.0),
        ];
        assert!(inside(way_in(&l), &l), "{}", way_in(&l));
        // Patrolling keeps inside, whichever corner it heads for.
        for n in 0..12 {
            assert!(inside(patrol_point(&l, n), &l), "{n}");
            assert!(inside(patrol_point(&square, n), &square), "{n}");
        }
    }

    #[test]
    fn the_holtburg_dungeon_is_entered_by_its_portal() {
        let from = Vec2::new(32600.0, 34690.0);
        let p = entrance(0x01F6_0000, from).expect("a portal leads into the Holtburg Dungeon");
        assert_eq!(p.name, "Holtburg Dungeon");
        assert_eq!(p.to_cell & 0xFFFF_0000, 0x01F6_0000);
        // A landblock nothing leads into.
        assert!(entrance(0xFE00_0000, from).is_none());
    }

    fn outline(points: &[[f32; 2]]) -> HuntArea {
        HuntArea {
            name: "graveyard".into(),
            shape: Shape::Outline {
                points: points.to_vec(),
            },
        }
    }

    #[test]
    fn an_outline_holds_what_is_inside_it_even_round_a_corner() {
        // An L: a square with its top right quarter cut out.
        let l = outline(&[
            [0.0, 0.0],
            [100.0, 0.0],
            [100.0, 50.0],
            [50.0, 50.0],
            [50.0, 100.0],
            [0.0, 100.0],
        ]);
        let outdoors = 0xA9B4_0024;
        assert!(l.contains(Vec3::new(25.0, 75.0, 60.0), outdoors, false));
        assert!(l.contains(Vec3::new(75.0, 25.0, 60.0), outdoors, false));
        // The cut-out corner is outside, and so is past the edges.
        assert!(!l.contains(Vec3::new(75.0, 75.0, 60.0), outdoors, false));
        assert!(!l.contains(Vec3::new(-1.0, 50.0, 60.0), outdoors, false));
        // A building's room inside the outline is inside; a dungeon under
        // the same spot is not.
        assert!(l.contains(Vec3::new(25.0, 25.0, 66.0), 0xA9B4_0117, false));
        assert!(!l.contains(Vec3::new(25.0, 25.0, -20.0), 0xA9B4_0117, true));
        // Two corners enclose nothing.
        assert!(!outline(&[[0.0, 0.0], [10.0, 10.0]]).usable());
        assert!(l.usable());
    }

    #[test]
    fn a_dungeon_is_all_its_rooms_or_the_ones_chosen() {
        let whole = HuntArea {
            name: "Holtburg Dungeon".into(),
            shape: Shape::Dungeon {
                landblock: 0x01F6_0000,
                rooms: Vec::new(),
            },
        };
        let here = Vec3::ZERO;
        assert!(whole.contains(here, 0x01F6_012F, true));
        assert!(whole.contains(here, 0x01F6_0289, true));
        // Another dungeon, and the landblock's own outdoor numbering.
        assert!(!whole.contains(here, 0x01F7_012F, true));
        assert!(!whole.contains(here, 0x01F6_0001, false));
        let wing = HuntArea {
            shape: Shape::Dungeon {
                landblock: 0x01F6_0000,
                rooms: vec![0x01F6_012F, 0x01F6_0134],
            },
            ..whole.clone()
        };
        assert!(wing.contains(here, 0x01F6_012F, true));
        assert!(!wing.contains(here, 0x01F6_0289, true));
        assert_eq!(wing.describe(), "dungeon 0x01f6, 2 rooms");
        assert_eq!(whole.describe(), "dungeon 0x01f6, all of it");
    }

    #[test]
    fn an_area_is_saved_and_read_back_as_it_was() {
        let a = outline(&[[1.0, 2.0], [3.0, 4.0], [5.0, 1.0]]);
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains("\"kind\":\"outline\""), "{json}");
        let back: HuntArea = serde_json::from_str(&json).unwrap();
        assert_eq!(back, a);
    }
}
