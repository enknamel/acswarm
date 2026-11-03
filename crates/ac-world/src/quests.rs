//! Quests an NPC rewards with XP: who gives each, what it asks (kills, an item, a talk), the
//! reward and how soon it repeats, and where what it asks comes from.
//!
//! Server data (emote sets), copied in `data/quests.csv` and `data/quest_sources.csv`
//! (`reference/scripts/data/quests.sh`); rows sharing an id are alternative ways to one quest.

use std::sync::OnceLock;

use crate::Place;

/// What a row is: the way it completes a quest, or the talk that takes one up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Kill,
    Item,
    Talk,
    /// Stamps what the quest's other rows need (a kill task's counter); no reward.
    Start,
}

/// How the giver is approached: handed the item, or used (talked to).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handing {
    Give,
    Talk,
}

/// What a row asks for.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Want {
    /// An item, by weenie class.
    Item(u32),
    /// Kills counted under this quest counter (a creature's KillQuest string, lowercased).
    Kill(String),
    Nothing,
}

/// How soon a quest may be done again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    Any,
    /// Seconds after the last solve (the quest table's `min_Delta`).
    After(u32),
    Once,
}

/// Who else the XP goes to (EmoteManager.cs:126-184).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Share {
    /// The fellowship (AwardXP).
    All,
    /// Nobody (AwardNoShareXP).
    None,
    /// Passed up the allegiance only (AwardLevelProportionalXP).
    Allegiance,
}

/// The XP a row awards: fixed, plus a share of the XP to the next level, clamped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reward {
    pub xp: u64,
    /// Fraction of the XP from the character's level to the next; 0 for none.
    pub xp_level: f64,
    /// Clamps on the scaled part, 0 for none.
    pub xp_min: u64,
    pub xp_max: u64,
    pub share: Share,
}

impl Reward {
    /// The XP at a level whose next level costs `to_next` (`XpTable::xp_to_next_level`):
    /// Player_Xp.cs:441-455, `Math.Round` rounding half to even.
    pub fn xp_at(&self, to_next: u64) -> u64 {
        if self.xp_level <= 0.0 {
            return self.xp;
        }
        let mut scaled = (to_next as f64 * self.xp_level).round_ties_even() as u64;
        if self.xp_max > 0 {
            scaled = scaled.min(self.xp_max);
        }
        if self.xp_min > 0 {
            scaled = scaled.max(self.xp_min);
        }
        self.xp + scaled
    }
}

/// The wanted item, as the loot rules read it.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub wcid: u32,
    pub name: String,
    /// ItemType bits (PropertyInt 1).
    pub item_type: u32,
    pub value: u32,
}

/// One way to complete a quest (or to take it up): a giver placement and level band.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// The quest; rows sharing it are alternatives.
    pub id: String,
    pub kind: Kind,
    pub title: String,
    pub giver_wcid: u32,
    pub giver: String,
    pub place: Place,
    pub hand: Handing,
    pub want: Want,
    /// Items one hand-in takes, or kills.
    pub count: u32,
    /// The giver's level band, 0 for open.
    pub min_level: u32,
    pub max_level: u32,
    pub reward: Reward,
    /// Reward items, `(wcid, count)`.
    pub gives: Vec<(u32, u32)>,
    /// Quest flags the reward stamps, lowercased.
    pub stamps: Vec<String>,
    pub repeat: Repeat,
    /// The quest the wait or once-flag is kept under; empty when it repeats any time.
    pub timer: String,
    /// Flags held first; a kill row needs its own counter, which its start row stamps.
    pub needs: Vec<String>,
    pub item: Option<Item>,
}

impl Row {
    /// Whether the giver's level band holds `level`.
    pub fn fits_level(&self, level: u32) -> bool {
        level >= self.min_level && (self.max_level == 0 || level <= self.max_level)
    }

    /// The flag a giver in a dungeon is shut behind: every working portal into its landblock
    /// asks a flag not in `held`. `None` for a giver a walk or an open portal reaches.
    pub fn locked_by(&self, held: &[String]) -> Option<&'static str> {
        if !self.place.is_indoors() {
            return None;
        }
        let block = self.place.cell >> 16;
        let portals = crate::portals::all();
        let touches_surface = portals.iter().any(|p| {
            (p.from_cell >> 16 == block && p.mouth_outdoors())
                || (p.to_cell >> 16 == block && p.exit_outdoors())
        });
        if touches_surface {
            return None;
        }
        let into: Vec<&crate::portals::Portal> = portals
            .iter()
            .filter(|p| p.works() && p.to_cell >> 16 == block && p.from_cell >> 16 != block)
            .collect();
        let shut = |p: &&crate::portals::Portal| {
            !p.quest.is_empty() && !held.iter().any(|h| h.eq_ignore_ascii_case(&p.quest))
        };
        if into.is_empty() || !into.iter().all(shut) {
            return None;
        }
        into.first().map(|p| p.quest.as_str())
    }
}

/// How a source yields what a quest wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Yield {
    /// A corpse drop, at `chance` per kill.
    Drop,
    /// A creature whose death counts for the counter.
    Kill,
    /// A placed NPC whose talk gives the item.
    Npc,
}

/// Where what a quest wants comes from.
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub want: Want,
    pub kind: Yield,
    /// The creature or NPC.
    pub wcid: u32,
    pub name: String,
    pub level: u32,
    /// Per kill; 1 for a kill or an NPC.
    pub chance: f32,
    /// `(landblock, spawn entries)` as `hunting::Ground::count` counts; an NPC's is `(cell, 1)`.
    pub landblocks: Vec<(u32, u32)>,
}

const QUESTS: &str = include_str!("../data/quests.csv");
const SOURCES: &str = include_str!("../data/quest_sources.csv");

fn words(s: &str) -> Vec<String> {
    s.split(';')
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

fn pairs(s: &str, radix: u32) -> Option<Vec<(u32, u32)>> {
    s.split(';')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let (a, b) = w.split_once(':')?;
            Some((u32::from_str_radix(a, radix).ok()?, b.parse().ok()?))
        })
        .collect()
}

fn want_of(kind: Kind, s: &str) -> Option<Want> {
    Some(match kind {
        Kind::Item => Want::Item(s.parse().ok()?),
        Kind::Kill => Want::Kill(s.to_string()),
        Kind::Talk | Kind::Start => Want::Nothing,
    })
}

fn parse_row(line: &str) -> Option<Row> {
    let f: Vec<&str> = line.split(',').collect();
    if f.len() != 27 {
        return None;
    }
    let kind = match f[1] {
        "kill" => Kind::Kill,
        "item" => Kind::Item,
        "talk" => Kind::Talk,
        "start" => Kind::Start,
        _ => return None,
    };
    let cell = u32::from_str_radix(f[5], 16).ok()?;
    let local = glam::Vec3::new(f[6].parse().ok()?, f[7].parse().ok()?, f[8].parse().ok()?);
    let repeat = match f[21].parse::<i64>().ok()? {
        0 => Repeat::Any,
        -1 => Repeat::Once,
        s => Repeat::After(u32::try_from(s).ok()?),
    };
    let item = match f[24] {
        "" => None,
        name => Some(Item {
            wcid: f[10].parse().ok()?,
            name: name.to_string(),
            item_type: u32::from_str_radix(f[25], 16).ok()?,
            value: f[26].parse().ok()?,
        }),
    };
    Some(Row {
        id: f[0].to_string(),
        kind,
        title: f[2].to_string(),
        giver_wcid: f[3].parse().ok()?,
        giver: f[4].to_string(),
        place: Place {
            at: crate::landblock_origin(cell) + local,
            cell,
        },
        hand: if f[9] == "give" {
            Handing::Give
        } else {
            Handing::Talk
        },
        want: want_of(kind, f[10])?,
        count: f[11].parse().ok()?,
        min_level: f[12].parse().ok()?,
        max_level: f[13].parse().ok()?,
        reward: Reward {
            xp: f[14].parse().ok()?,
            xp_level: f[15].parse().ok()?,
            xp_min: f[16].parse().ok()?,
            xp_max: f[17].parse().ok()?,
            share: match f[18] {
                "all" => Share::All,
                "allegiance" => Share::Allegiance,
                _ => Share::None,
            },
        },
        gives: pairs(f[19], 10)?,
        stamps: words(f[20]),
        repeat,
        timer: f[22].to_string(),
        needs: words(f[23]),
        item,
    })
}

fn parse_source(line: &str) -> Option<Source> {
    let f: Vec<&str> = line.split(',').collect();
    if f.len() != 7 {
        return None;
    }
    let kind = match f[1] {
        "drop" => Yield::Drop,
        "kill" => Yield::Kill,
        "npc" => Yield::Npc,
        _ => return None,
    };
    Some(Source {
        want: if kind == Yield::Kill {
            Want::Kill(f[0].to_string())
        } else {
            Want::Item(f[0].parse().ok()?)
        },
        kind,
        wcid: f[2].parse().ok()?,
        name: f[3].to_string(),
        level: f[4].parse().ok()?,
        chance: f[5].parse().ok()?,
        landblocks: pairs(f[6], 16)?,
    })
}

fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
}

/// Every row, parsed on first use.
pub fn all() -> &'static [Row] {
    static ROWS: OnceLock<Vec<Row>> = OnceLock::new();
    ROWS.get_or_init(|| lines(QUESTS).filter_map(parse_row).collect())
}

/// Every source, parsed on first use.
pub fn sources() -> &'static [Source] {
    static SOURCES_PARSED: OnceLock<Vec<Source>> = OnceLock::new();
    SOURCES_PARSED.get_or_init(|| lines(SOURCES).filter_map(parse_source).collect())
}

/// The rows of one quest, its start rows among them.
pub fn rows_of(id: &str) -> impl Iterator<Item = &'static Row> + '_ {
    all().iter().filter(move |r| r.id == id)
}

/// Where what `want` asks for comes from.
pub fn sources_of(want: &Want) -> impl Iterator<Item = &'static Source> + '_ {
    sources().iter().filter(move |s| &s.want == want)
}

/// The rows whose reward or talk stamps `flag`.
pub fn stamped_by(flag: &str) -> impl Iterator<Item = &'static Row> + '_ {
    all()
        .iter()
        .filter(move |r| r.stamps.iter().any(|s| s.eq_ignore_ascii_case(flag)))
}

#[cfg(test)]
mod tests;
