//! The Colosseum's arenas as their emote tables run them (`data/arenas.csv`, from
//! `reference/scripts/data/quests_walk.py`): each room's bell, whose waves once passed reveal the
//! room's portal to the next room; the NPC ways in a fellowship's access quest opens; and who
//! stamps that quest on the fellowship for a ticket (EmoteManager.cs:500-517, 854-861).

use std::sync::OnceLock;

use glam::Vec3;

use crate::Place;

const ARENAS: &str = include_str!("../data/arenas.csv");

/// One arena room.
#[derive(Clone, Debug, PartialEq)]
pub struct Room {
    pub landblock: u32,
    /// The bell whose use starts the room's waves, and where it stands.
    pub bell: u32,
    pub bell_at: Place,
    /// The portal the waves passed reveal, where it comes, and where it sends.
    pub portal: u32,
    pub portal_at: Place,
    pub to: Place,
    /// The flags going through the portal stamps (lower case): Arena 3's AccessBoss3.
    pub flags: Vec<String>,
}

/// An NPC whose use casts the teleport into an arena for a fellowship holding `access`.
#[derive(Clone, Debug, PartialEq)]
pub struct Way {
    pub landblock: u32,
    pub npc: u32,
    pub name: String,
    pub at: Place,
    /// Where the teleport lands: the room the arena is begun in.
    pub land: Place,
    /// The fellowship quest the NPC asks (lower case), and the lowest level it lets in.
    pub access: String,
    pub min_level: u32,
}

/// The NPC who, handed `item` by a fellow, stamps `access` on the fellowship for `lasts` seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Ticket {
    pub landblock: u32,
    pub giver: u32,
    pub name: String,
    pub at: Place,
    pub item: u32,
    pub access: String,
    pub lasts: u32,
}

/// Every room, way in and ticket of the bundled data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Arenas {
    pub rooms: Vec<Room>,
    pub ways: Vec<Way>,
    pub tickets: Vec<Ticket>,
}

/// The bundled arenas, parsed on first use.
pub fn all() -> &'static Arenas {
    static ALL: OnceLock<Arenas> = OnceLock::new();
    ALL.get_or_init(|| parse(ARENAS))
}

fn place(cell: &str, at: &str) -> Option<Place> {
    let cell = u32::from_str_radix(cell, 16).ok()?;
    let xyz: Vec<f32> = at
        .split(' ')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let [x, y, z] = xyz[..] else { return None };
    Some(Place {
        at: crate::landblock_origin(cell) + Vec3::new(x, y, z),
        cell,
    })
}

fn parse(text: &str) -> Arenas {
    let mut out = Arenas::default();
    for f in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
    {
        let f: Vec<&str> = f.split(',').collect();
        let [kind, lb, wcid, name, cell, at, other, other_cell, other_at, quest, to] = f[..] else {
            continue;
        };
        let (Ok(lb), Ok(wcid), Some(spot)) = (
            u32::from_str_radix(lb, 16),
            wcid.parse::<u32>(),
            place(cell, at),
        ) else {
            continue;
        };
        let other: u32 = other.parse().unwrap_or(0);
        match kind {
            "room" => {
                let (Some(portal_at), Some(to)) =
                    (place(other_cell, other_at), place(other_cell, to))
                else {
                    continue;
                };
                out.rooms.push(Room {
                    landblock: lb,
                    bell: wcid,
                    bell_at: spot,
                    portal: other,
                    portal_at,
                    to,
                    flags: quest
                        .split(';')
                        .filter(|q| !q.is_empty())
                        .map(String::from)
                        .collect(),
                });
            }
            "way" => {
                let Some(land) = place(other_cell, other_at) else {
                    continue;
                };
                out.ways.push(Way {
                    landblock: lb,
                    npc: wcid,
                    name: crate::data_name(name),
                    at: spot,
                    land,
                    access: quest.to_string(),
                    min_level: to.parse().unwrap_or(0),
                });
            }
            "ticket" => out.tickets.push(Ticket {
                landblock: lb,
                giver: wcid,
                name: crate::data_name(name),
                at: spot,
                item: other,
                access: quest.to_string(),
                lasts: to.parse().unwrap_or(0),
            }),
            _ => {}
        }
    }
    out
}

impl Arenas {
    /// The room of arena `landblock` whose bell stands nearest `at` (world): the one it is in.
    pub fn room_at(&self, landblock: u32, at: Vec3) -> Option<&Room> {
        self.rooms
            .iter()
            .filter(|r| r.landblock == landblock)
            .min_by(|a, b| {
                a.bell_at
                    .at
                    .distance(at)
                    .total_cmp(&b.bell_at.at.distance(at))
            })
    }

    /// Whether `landblock` is an arena's.
    pub fn is_arena(&self, landblock: u32) -> bool {
        self.rooms.iter().any(|r| r.landblock == landblock)
    }

    /// The rooms from `way`'s landing room through to the one whose portal stamps `flag`, each
    /// room's portal sending into the next; `None` when its rooms never come to that one.
    pub fn rooms_to(&self, way: &Way, flag: &str) -> Option<Vec<&Room>> {
        let mut room = self.room_at(way.landblock, way.land.at)?;
        let mut out = vec![room];
        while !room.flags.iter().any(|f| f.eq_ignore_ascii_case(flag)) {
            room = self.room_at(way.landblock, room.to.at)?;
            if out.contains(&room) {
                return None;
            }
            out.push(room);
        }
        Some(out)
    }

    /// The ways in whose rooms come to the portal stamping `flag`, for a character of `level`.
    pub fn ways_to(&self, flag: &str, level: u32) -> impl Iterator<Item = &Way> + '_ {
        let flag = flag.to_string();
        self.ways
            .iter()
            .filter(move |w| w.min_level <= level && self.rooms_to(w, &flag).is_some())
    }

    /// Whether some room's portal stamps `flag`.
    pub fn reveals(&self, flag: &str) -> bool {
        self.rooms
            .iter()
            .any(|r| r.flags.iter().any(|f| f.eq_ignore_ascii_case(flag)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arena_3_is_reached_from_the_colosseum_arena_s_first_room_not_the_advanced_one() {
        // Arena 3's portal comes once room 2's waves pass (EmoteManager.cs:1265-1290); room 2 only
        // through room 1's Arena 2 portal. The Advanced Arena lands in room 6, past it.
        let a = all();
        assert_eq!(a.rooms.len(), 85);
        let ways: Vec<(u32, u32)> = a
            .ways_to("accessboss3", 200)
            .map(|w| (w.landblock, w.npc))
            .collect();
        assert_eq!(
            ways,
            [
                (0xB0, 34727),
                (0xB1, 34727),
                (0xB2, 34727),
                (0xB3, 34727),
                (0xB4, 34727)
            ]
        );
        let rooms = a
            .rooms_to(&a.ways[0], "accessboss3")
            .expect("room 1 to room 2");
        let bells: Vec<u32> = rooms.iter().map(|r| r.bell).collect();
        assert_eq!(bells, [34604, 34718]);
        assert_eq!(rooms[1].portal, 34518);
        let ticket = a
            .tickets
            .iter()
            .find(|t| t.access == "coloarenaoneaccess")
            .unwrap();
        assert_eq!(
            (ticket.giver, ticket.item, ticket.lasts),
            (34441, 34449, 3600)
        );
    }
}
