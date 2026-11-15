//! Every portal in the world: where it stands and where it leads.
//!
//! Portals are server-side objects, so unlike the landscape they are not
//! in the client's own data files. `data/portals.csv` is a copy of the
//! list (see `reference/scripts/data/portals.sh` for how it is made and
//! where it comes from); it is parsed once on first use.
//!
//! A trip planner uses this to travel further than a walk: the Town
//! Network from a town to the hub and out again, a dungeon's "Surface"
//! portal, a town's own portal. [`near`] finds the portals whose mouth
//! is within reach of a spot, and [`to_place`] the ones that come out
//! near somewhere.

use glam::{Vec2, Vec3};
use std::sync::OnceLock;

/// One portal: its mouth (where the character walks in) and its exit.
#[derive(Clone, Debug, PartialEq)]
pub struct Portal {
    pub name: String,
    /// Cell the mouth stands in, and its world position.
    pub from_cell: u32,
    pub from: Vec3,
    /// Cell it comes out in, and that world position.
    pub to_cell: u32,
    pub to: Vec3,
    /// Level range the character must be in, 0 for no limit.
    pub min_level: u32,
    pub max_level: u32,
    /// The quest that must be finished first, empty when there is none.
    pub quest: String,
    /// The item's `Usable` word. 1 is `No`: the portal cannot be used at
    /// all, which is how the ruined ones are marked (and a good many
    /// decorations besides).
    pub usable: u32,
    /// Its `PortalBitmask` (ACE `PortalBitmask.cs`); Unrestricted when the database has none.
    pub bitmask: u32,
}

/// `PortalBitmask::NoRecall`: going through leaves Portal Recall where it was (`Portal.cs:284`).
pub const NO_RECALL: u32 = 0x20;
/// `PortalBitmask` bars by who is asking (ACE `PortalBitmask.cs`), refused at `Portal.cs:185-225`.
pub const UNRESTRICTED: u32 = 0x01;
pub const NO_PK: u32 = 0x02;
pub const NO_PK_LITE: u32 = 0x04;
pub const NO_NPK: u32 = 0x08;
pub const ONLY_OLTHOI: u32 = 0x40;
pub const NO_VITAE: u32 = 0x100;

/// The character asking to use a portal, as its bitmask is checked against.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Traveller {
    /// `PlayerKillerStatus` (`object::pk_status`), 0 unknown: then no PK bar is read into it.
    pub pk_status: u32,
    /// Weakened by vitae now.
    pub vitae: bool,
}

impl Portal {
    /// Whether Portal Recall remembers this portal once it is used.
    pub fn is_recallable(&self) -> bool {
        self.bitmask & NO_RECALL == 0
    }

    pub fn from_xy(&self) -> Vec2 {
        self.from.truncate()
    }

    pub fn to_xy(&self) -> Vec2 {
        self.to.truncate()
    }

    /// Whether the mouth is outdoors (a walk can reach it).
    pub fn mouth_outdoors(&self) -> bool {
        self.from_cell & 0xFFFF < 0x100
    }

    /// Whether it comes out outdoors.
    pub fn exit_outdoors(&self) -> bool {
        self.to_cell & 0xFFFF < 0x100
    }

    /// Whether the portal can be used at all. The ruined ones say so
    /// themselves: their `Usable` word is `No`. Nothing is read into a
    /// word of zero, which only means the data does not say.
    pub fn works(&self) -> bool {
        self.usable != crate::usable::NO
    }

    /// A portal of the Town Network: the ones that link every town to
    /// the hub, and the hub's own portals back out.
    pub fn is_town_network(&self) -> bool {
        self.name.to_lowercase().contains("town network")
    }

    /// Whether its bitmask lets `who` through (`Portal.cs:185-225`). An explicit 0 is `Undef`,
    /// "Players may not interact with that portal"; a missing one reads Unrestricted (:10).
    /// Being a player killer is what the character is, never a way to a route.
    pub fn admits(&self, who: Traveller) -> bool {
        use crate::object::pk_status;
        let barred = |bit: u32, status: u32| self.bitmask & bit != 0 && who.pk_status == status;
        self.bitmask != 0
            && self.bitmask & ONLY_OLTHOI == 0
            && !barred(NO_PK, pk_status::PK)
            && !barred(NO_PK_LITE, pk_status::PK_LITE)
            && !barred(NO_NPK, pk_status::NPK)
            && !(who.vitae && self.bitmask & NO_VITAE != 0)
    }

    /// Whether a character of this level may use it. A quest-locked
    /// portal is refused unless its quest is in `quests_done`, since
    /// walking to one we cannot take wastes the trip.
    pub fn usable_by(&self, level: u32, quests_done: &[String]) -> bool {
        if self.min_level > 0 && level < self.min_level {
            return false;
        }
        if self.max_level > 0 && level > self.max_level {
            return false;
        }
        if !self.quest.is_empty()
            && !quests_done
                .iter()
                .any(|q| q.eq_ignore_ascii_case(&self.quest))
        {
            return false;
        }
        true
    }

    /// Why a character of this level cannot take it, for a message.
    pub fn refusal(&self, level: u32) -> Option<String> {
        if self.min_level > 0 && level < self.min_level {
            return Some(format!("needs level {}", self.min_level));
        }
        if self.max_level > 0 && level > self.max_level {
            return Some(format!("is for levels up to {}", self.max_level));
        }
        if !self.quest.is_empty() {
            return Some(format!("needs the quest {}", self.quest));
        }
        None
    }
}

const DATA: &str = include_str!("../data/portals.csv");

fn parse(text: &str) -> Vec<Portal> {
    let mut out = Vec::with_capacity(4600);
    for line in text.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        if f.len() < 9 {
            continue;
        }
        let hex = |s: &str| u32::from_str_radix(s, 16).ok();
        let num = |s: &str| s.parse::<f32>().ok();
        let (Some(from_cell), Some(to_cell)) = (hex(f[1]), hex(f[5])) else {
            continue;
        };
        let (Some(sx), Some(sy), Some(sz)) = (num(f[2]), num(f[3]), num(f[4])) else {
            continue;
        };
        let (Some(dx), Some(dy), Some(dz)) = (num(f[6]), num(f[7]), num(f[8])) else {
            continue;
        };
        let level = |i: usize| f.get(i).and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
        out.push(Portal {
            name: crate::data_name(f[0]),
            from_cell,
            from: crate::landblock_origin(from_cell) + Vec3::new(sx, sy, sz),
            to_cell,
            to: crate::landblock_origin(to_cell) + Vec3::new(dx, dy, dz),
            min_level: level(9),
            max_level: level(10),
            quest: f.get(11).map(|q| q.trim().to_string()).unwrap_or_default(),
            usable: level(12),
            // Missing or NULL reads Unrestricted, as ACE's getter does (Portal_Properties.cs:10).
            bitmask: f
                .get(13)
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(UNRESTRICTED),
        });
    }
    out
}

/// Every portal, parsed on first use.
pub fn all() -> &'static [Portal] {
    static PORTALS: OnceLock<Vec<Portal>> = OnceLock::new();
    PORTALS.get_or_init(|| parse(DATA))
}

/// The portals whose mouth is within `radius` metres of `world`, nearest
/// first.
pub fn near(world: Vec2, radius: f32) -> Vec<&'static Portal> {
    let mut v: Vec<&Portal> = all()
        .iter()
        .filter(|p| p.from_xy().distance(world) <= radius)
        .collect();
    v.sort_by(|a, b| {
        a.from_xy()
            .distance(world)
            .total_cmp(&b.from_xy().distance(world))
    });
    v
}

/// The portals that come out within `radius` metres of `world`.
pub fn to_place(world: Vec2, radius: f32) -> Vec<&'static Portal> {
    let mut v: Vec<&Portal> = all()
        .iter()
        .filter(|p| p.to_xy().distance(world) <= radius)
        .collect();
    v.sort_by(|a, b| {
        a.to_xy()
            .distance(world)
            .total_cmp(&b.to_xy().distance(world))
    });
    v
}

/// The portals out of an enclosed place: those whose mouth stands in
/// `block`'s landblock, that work, and that come out under the sky.
///
/// This is what a character in a hub or a dungeon can reach without a
/// recall. It cannot walk out -- an indoor cell is only walkable to the
/// rest of its own landblock -- so these are the onward ways there are.
pub fn out_of(block: u32) -> impl Iterator<Item = &'static Portal> {
    all().iter().filter(move |p| {
        p.from_cell & 0xFFFF_0000 == block & 0xFFFF_0000 && p.works() && p.exit_outdoors()
    })
}

/// A portal a creature's death, or an NPC handed an item or used, opens: a summon of its linked
/// portal, 3 m in front of it, for the spell's lifetime (`data/summoned_portals.csv`;
/// WorldObject_Magic.cs:1185-1266).
#[derive(Clone, Debug, PartialEq)]
pub struct Summoned {
    /// The creature whose death opens it.
    pub by: u32,
    pub by_name: String,
    /// The portal's weenie class, as it stands in the world once opened.
    pub wcid: u32,
    pub name: String,
    pub to_cell: u32,
    pub to: Vec3,
    /// The portal's own level band (0: open) and flag ("" for none).
    pub min_level: u32,
    pub max_level: u32,
    pub quest: String,
}

impl Summoned {
    /// The portal standing at `mouth` in `cell`: what a trip takes while it is in view.
    pub fn standing_at(&self, cell: u32, mouth: Vec3) -> Portal {
        Portal {
            name: self.name.clone(),
            from_cell: cell,
            from: mouth,
            to_cell: self.to_cell,
            to: self.to,
            min_level: self.min_level,
            max_level: self.max_level,
            quest: self.quest.clone(),
            usable: 0,
            bitmask: UNRESTRICTED,
        }
    }
}

/// A portal a generator in the world puts out: a camp's rare pick, and the only way into some
/// dungeons (the Small Hive Portals of the Marae Lassel plateau; `data/spawned_portals.csv`).
#[derive(Clone, Debug, PartialEq)]
pub struct Spawned {
    pub wcid: u32,
    /// As it stands where it comes on one landblock (its generator's spot, or the encounter
    /// table's busiest corner there): where it leads, its level band and its flag.
    pub way: Portal,
    /// How fast it comes on that landblock, taken as it comes.
    pub rate: crate::quests::SpawnRate,
}

impl Spawned {
    /// The landblock it comes on.
    pub fn landblock(&self) -> u32 {
        self.way.from_cell >> 16
    }

    /// Where it comes, to be looked for.
    pub fn place(&self) -> crate::Place {
        crate::Place {
            at: self.way.from,
            cell: self.way.from_cell,
        }
    }

    /// The portal standing at `mouth` in `cell`: what a trip takes while it is in view.
    pub fn standing_at(&self, cell: u32, mouth: Vec3) -> Portal {
        Portal {
            from_cell: cell,
            from: mouth,
            ..self.way.clone()
        }
    }
}

const SPAWNED: &str = include_str!("../data/spawned_portals.csv");

/// Every portal a generator puts out, a row a landblock it comes on, parsed on first use.
pub fn spawned() -> &'static [Spawned] {
    static PARSED: OnceLock<Vec<Spawned>> = OnceLock::new();
    PARSED.get_or_init(|| {
        let row = |line: &str| {
            let f: Vec<&str> = line.split(',').collect();
            let num = |i: usize| f.get(i)?.parse::<f32>().ok();
            let cell = u32::from_str_radix(f.get(2)?, 16).ok()?;
            let to_cell = u32::from_str_radix(f.get(9)?, 16).ok()?;
            let way = Portal {
                name: crate::data_name(f.get(1)?),
                from_cell: cell,
                from: crate::landblock_origin(cell) + Vec3::new(num(3)?, num(4)?, num(5)?),
                to_cell,
                to: crate::landblock_origin(to_cell) + Vec3::new(num(10)?, num(11)?, num(12)?),
                min_level: f.get(13)?.parse().ok()?,
                max_level: f.get(14)?.parse().ok()?,
                quest: f.get(15).map_or_else(String::new, |q| q.to_string()),
                usable: 0,
                bitmask: UNRESTRICTED,
            };
            Some(Spawned {
                wcid: f.first()?.parse().ok()?,
                way,
                rate: crate::quests::SpawnRate {
                    load: num(6)?,
                    per_hour: num(7)?,
                    others: num(8)?,
                },
            })
        };
        let lines = SPAWNED
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty());
        lines.filter_map(row).collect()
    })
}

/// A row of the portal of class `wcid` a generator puts out, any: each leads the same way.
pub fn spawned_of(wcid: u32) -> Option<&'static Spawned> {
    static BY_CLASS: OnceLock<std::collections::HashMap<u32, usize>> = OnceLock::new();
    let by = BY_CLASS.get_or_init(|| {
        let rows = spawned().iter().enumerate();
        rows.rev().map(|(i, s)| (s.wcid, i)).collect()
    });
    spawned().get(*by.get(&wcid)?)
}

const SUMMONED: &str = include_str!("../data/summoned_portals.csv");

/// Every portal a creature's death opens, parsed on first use.
pub fn summoned() -> &'static [Summoned] {
    static PARSED: OnceLock<Vec<Summoned>> = OnceLock::new();
    PARSED.get_or_init(|| {
        let row = |line: &str| {
            let f: Vec<&str> = line.split(',').collect();
            let num = |i: usize| f.get(i)?.parse::<f32>().ok();
            let to_cell = u32::from_str_radix(f.get(4)?, 16).ok()?;
            Some(Summoned {
                by: f.first()?.parse().ok()?,
                by_name: crate::data_name(f.get(1)?),
                wcid: f.get(2)?.parse().ok()?,
                name: crate::data_name(f.get(3)?),
                to_cell,
                to: crate::landblock_origin(to_cell) + Vec3::new(num(5)?, num(6)?, num(7)?),
                min_level: f.get(8).and_then(|v| v.parse().ok()).unwrap_or(0),
                max_level: f.get(9).and_then(|v| v.parse().ok()).unwrap_or(0),
                quest: f.get(10).map_or_else(String::new, |q| q.to_string()),
            })
        };
        let lines = SUMMONED
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty());
        lines.filter_map(row).collect()
    })
}

/// A way a placed NPC opens for whoever hands it an item or uses it: a portal it summons in front
/// of it (Faladha the Emissary's to Aerlinthe, for a 10,000 note), or a teleport
/// (`data/npc_ways.csv`).
#[derive(Clone, Debug, PartialEq)]
pub struct NpcWay {
    pub npc: u32,
    pub name: String,
    /// Where it stands: walked to, then handed the item or used.
    pub cell: u32,
    pub at: Vec3,
    /// What is handed; `None` for a use.
    pub item: Option<u32>,
    /// Flags held first, lower case.
    pub needs: Vec<String>,
    /// Level band, 0 for open.
    pub min_level: u32,
    pub max_level: u32,
    /// The portal it summons; `None` for a teleport.
    pub portal: Option<u32>,
    pub to_cell: u32,
    pub to: Vec3,
    /// Seconds of the NPC's emote delays before the portal is summoned or the teleport made.
    pub secs: f32,
    /// Placed objects used first, each starting a server event the NPC asks running: weenie
    /// class and where it stands (Browerk's Portal Device and the four Eyes of the Watcher).
    pub first: Vec<(u32, crate::Place)>,
}

impl NpcWay {
    /// Whether a character of `level` holding `quests_done` may take it, the item had.
    pub fn usable_by(&self, level: u32, quests_done: &[String]) -> bool {
        let held = |f: &String| quests_done.iter().any(|q| q.eq_ignore_ascii_case(f));
        level >= self.min_level
            && (self.max_level == 0 || level <= self.max_level)
            && self.needs.iter().all(held)
    }

    /// The way as a portal from its NPC's spot: what the trip planner searches.
    pub fn as_portal(&self) -> Portal {
        Portal {
            name: self.name.clone(),
            from_cell: self.cell,
            from: self.at,
            to_cell: self.to_cell,
            to: self.to,
            min_level: self.min_level,
            max_level: self.max_level,
            quest: String::new(),
            usable: 0,
            bitmask: UNRESTRICTED,
        }
    }
}

const NPC_WAYS: &str = include_str!("../data/npc_ways.csv");

/// One object an NPC way uses first, `wcid:CELL:x:y:z` (x/y/z local to the cell's landblock).
fn first_use(entry: &str) -> Option<(u32, crate::Place)> {
    let f: Vec<&str> = entry.split(':').collect();
    let num = |i: usize| f.get(i)?.parse::<f32>().ok();
    let cell = u32::from_str_radix(f.get(1)?, 16).ok()?;
    let local = Vec3::new(num(2)?, num(3)?, num(4)?);
    let cell = match cell & 0xFFFF {
        0 => crate::outdoor_cell(cell, local),
        _ => cell,
    };
    let at = crate::landblock_origin(cell) + local;
    Some((f.first()?.parse().ok()?, crate::Place { at, cell }))
}

/// Every way a placed NPC opens, parsed on first use.
pub fn npc_ways() -> &'static [NpcWay] {
    static PARSED: OnceLock<Vec<NpcWay>> = OnceLock::new();
    PARSED.get_or_init(|| {
        let row = |line: &str| {
            let f: Vec<&str> = line.split(',').collect();
            let num = |i: usize| f.get(i)?.parse::<f32>().ok();
            let int = |i: usize| f.get(i)?.parse::<u32>().ok();
            let cell = u32::from_str_radix(f.get(2)?, 16).ok()?;
            let local = Vec3::new(num(3)?, num(4)?, num(5)?);
            // An outdoor placement may name its landblock alone: its cell is where it stands.
            let cell = match cell & 0xFFFF {
                0 => crate::outdoor_cell(cell, local),
                _ => cell,
            };
            let to_cell = u32::from_str_radix(f.get(11)?, 16).ok()?;
            let first = f
                .get(16)
                .copied()
                .unwrap_or("")
                .split(';')
                .filter(|e| !e.is_empty());
            let first = first.map(first_use).collect::<Option<Vec<_>>>()?;
            Some(NpcWay {
                npc: int(0)?,
                name: crate::data_name(f.get(1)?),
                cell,
                at: crate::landblock_origin(cell) + local,
                item: int(6).filter(|w| *w != 0),
                needs: f
                    .get(7)?
                    .split(';')
                    .filter(|n| !n.is_empty())
                    .map(str::to_string)
                    .collect(),
                min_level: int(8)?,
                max_level: int(9)?,
                portal: int(10).filter(|w| *w != 0),
                to_cell,
                to: crate::landblock_origin(to_cell) + Vec3::new(num(12)?, num(13)?, num(14)?),
                secs: num(15).unwrap_or(0.0),
                first,
            })
        };
        let lines = NPC_WAYS
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty());
        lines.filter_map(row).collect()
    })
}

/// The portals whose name contains `needle`, case-insensitively.
pub fn named(needle: &str) -> Vec<&'static Portal> {
    let needle = needle.to_lowercase();
    all()
        .iter()
        .filter(|p| p.name.to_lowercase().contains(&needle))
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_portal_marked_no_recall_is_not_where_portal_recall_goes() {
        // The Facility Hub's Tusker Barracks portal (bitmask 49) left Blargerton's Portal Recall at
        // the Hub, the last recallable portal's exit; the town's portal into the Hub is recallable.
        let named = |name: &str, cell: u32| {
            super::all()
                .iter()
                .find(|p| p.name == name && p.from_cell == cell)
                .unwrap_or_else(|| panic!("no {name}"))
        };
        assert!(!named("Tusker Barracks", 0x8A02_011E).is_recallable());
        assert!(named("Portal to Facility Hub", 0x0007_0126).is_recallable());
    }

    use super::*;

    #[test]
    fn the_table_reads_and_places_portals_in_the_world() {
        let all = all();
        assert!(all.len() > 4000, "{} portals", all.len());
        // Holtburg's Town Network portal stands in Holtburg and comes out
        // in the hub, which is indoors.
        let holtburg = ac_world_holtburg();
        let network: Vec<&Portal> = near(holtburg, 400.0)
            .into_iter()
            .filter(|p| p.is_town_network())
            .collect();
        assert!(
            !network.is_empty(),
            "no Town Network portal within 400 m of Holtburg"
        );
        let p = network[0];
        assert!(p.mouth_outdoors(), "the mouth should be outdoors");
        assert!(!p.exit_outdoors(), "the hub is indoors");
        // And a portal comes out near Arwic.
        let arwic = crate::towns::find("Arwic").unwrap().world_xy();
        assert!(
            !to_place(arwic, 400.0).is_empty(),
            "nothing comes out near Arwic"
        );
        assert!(!named("town network").is_empty());
        // The ruined ones say so: their Usable word is No.
        let dead: Vec<&Portal> = all.iter().filter(|p| !p.works()).collect();
        assert!(dead.len() > 500, "{} that do not work", dead.len());
        assert!(
            dead.iter().any(|p| p.name.starts_with("Destroyed")),
            "no destroyed portal among them"
        );
        assert!(named("Portal to Town Network")[0].works());
        assert!(named("no such portal anywhere").is_empty());
    }

    #[test]
    fn the_town_network_hub_leads_out_to_the_towns() {
        // A Town Network gem lands in the hub, indoors. What makes it
        // worth carrying is where the hub's own portals go.
        let hub = named("Portal to Town Network")[0].to_cell;
        assert!(hub & 0xFFFF >= 0x100, "the hub is indoors");
        let out: Vec<&Portal> = out_of(hub).collect();
        assert!(out.len() >= 30, "{} ways out of the hub", out.len());
        assert!(out.iter().all(|p| p.works() && p.exit_outdoors()));
        let cragstone = out
            .iter()
            .find(|p| p.name == "Portal to Cragstone")
            .expect("the hub leads to Cragstone");
        assert_eq!(cragstone.to_cell & 0xFFFF_0000, 0xBB9F_0000);
        // Any cell of the block will do: the question is the landblock.
        assert_eq!(out_of(hub & 0xFFFF_0000).count(), out.len());
    }

    fn ac_world_holtburg() -> Vec2 {
        crate::towns::find("Holtburg").unwrap().world_xy()
    }

    #[test]
    fn level_and_quest_requirements_are_honoured() {
        // Something in the world asks for a level range and something
        // asks for a quest.
        let all = all();
        let ranged: Vec<&Portal> = all
            .iter()
            .filter(|p| p.max_level > 0 && p.quest.is_empty())
            .collect();
        assert!(!ranged.is_empty(), "no portal has a level range");
        let quested: Vec<&Portal> = all.iter().filter(|p| !p.quest.is_empty()).collect();
        assert!(!quested.is_empty(), "no portal asks for a quest");
        let p = ranged[0];
        assert!(!p.usable_by(p.max_level + 1, &[]));
        assert!(p.usable_by(p.max_level.max(p.min_level), &[]));
        assert!(p.refusal(p.max_level + 1).is_some());
        let q = quested[0];
        assert!(!q.usable_by(200, &[]));
        assert!(q.usable_by(200, std::slice::from_ref(&q.quest)));
        assert!(q.refusal(200).unwrap().contains("quest"));
    }

    #[test]
    fn a_portal_bars_who_its_bitmask_names() {
        use crate::object::pk_status;
        let npk = Traveller {
            pk_status: pk_status::NPK,
            vitae: false,
        };
        let pk = Traveller {
            pk_status: pk_status::PK,
            ..npk
        };
        // "Non p ks may not use portal": Blargerton went round every Olthoi Tunnel in Dereth.
        let tunnels: Vec<&Portal> = named("Olthoi Tunnel");
        assert!(tunnels.iter().any(|p| p.bitmask & NO_NPK != 0));
        for p in tunnels.iter().filter(|p| p.bitmask & NO_NPK != 0) {
            assert!(!p.admits(npk), "{p:?} let a non-PK through");
            assert!(p.admits(pk) || p.bitmask & ONLY_OLTHOI != 0);
        }
        let mut p = tunnels[0].clone();
        p.bitmask = UNRESTRICTED | NO_PK | NO_VITAE;
        assert!(p.admits(npk) && !p.admits(pk));
        assert!(
            !p.admits(Traveller { vitae: true, ..npk }),
            "through with vitae"
        );
        assert!(p.admits(Traveller::default()), "an unknown status barred");
        p.bitmask = 0;
        assert!(!p.admits(npk), "Undef: players may not use it");
        // The database leaves most Surface portals' bitmask out, which ACE reads Unrestricted:
        // exported as 0, they were all shut. Only the Unstable Portal says Undef.
        let shut: Vec<&str> = all()
            .iter()
            .filter(|p| p.works() && !p.admits(npk))
            .filter(|p| p.bitmask & (NO_NPK | ONLY_OLTHOI) == 0)
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(shut, ["Unstable Portal"]);
        p.bitmask = UNRESTRICTED | ONLY_OLTHOI;
        assert!(!p.admits(pk), "only Olthoi");
    }

    #[test]
    fn bad_lines_are_skipped() {
        let v = parse(
            "# comment\n\nbad,line\nName,A9B40019,1.0,2.0,3.0,70145,4.0,5.0,6.0,10,50,Quest,1\n",
        );
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].name, "Name");
        assert_eq!(
            v[0].bitmask, UNRESTRICTED,
            "no bitmask said reads Unrestricted"
        );
        assert!(!v[0].exit_outdoors());
        assert!(v[0].mouth_outdoors());
        assert_eq!((v[0].min_level, v[0].max_level), (10, 50));
        assert!(!v[0].usable_by(60, &[]));
        assert!(v[0].usable_by(20, &["quest".into()]));
        assert!(!v[0].works(), "usable 1 is No");
    }

    #[test]
    fn a_small_hive_portal_comes_on_the_plateau_and_leads_into_its_hive() {
        // The Royal Chest's hive (0x028A) has no other way in: a Marae Lassel plateau camp makes
        // its portal on a 3% pick of a roll.
        let hive: Vec<_> = super::spawned()
            .iter()
            .filter(|s| s.wcid == 11221)
            .collect();
        assert_eq!(hive.len(), 64, "the 64 landblocks it comes soonest on");
        assert!(hive.iter().all(|s| s.way.name == "Small Hive Portal"
            && s.way.to_cell == 0x028A_023F
            && s.way.min_level == 40
            && !s.place().is_indoors()));
        let best = hive
            .iter()
            .find(|s| s.landblock() == 0x20B6)
            .expect("0x20B6");
        assert_eq!(best.place().cell, 0x20B6_0026);
        assert!(
            best.rate.load > 0.2 && best.rate.per_hour > 4.0,
            "{:?}",
            best.rate
        );
    }
}
