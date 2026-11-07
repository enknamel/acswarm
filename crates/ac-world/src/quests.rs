//! Every quest a placed NPC holds, XP or not: who gives each and where they stand, what it asks
//! (kills, an item, a talk), what it does and what the client cannot check, how soon it repeats,
//! and where what it asks comes from: hunted, given, on the floor, in a chest, or in a shop.
//! Server data in `data/quests.csv`, `quest_givers.csv` and `quest_sources.csv`
//! (`reference/scripts/data/quests.sh`); rows sharing an id are alternative ways to one quest.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use glam::Vec3;

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
    pub name: &'static str,
    /// ItemType bits (PropertyInt 1).
    pub item_type: u32,
    pub value: u32,
}

/// One way to complete a quest (or to take it up): a giver, at any of its placements, and a level
/// band. Strings borrow the bundled text.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// The quest; rows sharing it are alternatives.
    pub id: &'static str,
    pub kind: Kind,
    pub title: &'static str,
    pub giver_wcid: u32,
    pub giver: &'static str,
    /// Every placement of the giver, never empty (`quest_givers.csv`).
    pub places: &'static [Place],
    pub hand: Handing,
    pub want: Want,
    /// Items one hand-in takes, or kills.
    pub count: u32,
    /// The giver's level band, 0 for open.
    pub min_level: u32,
    pub max_level: u32,
    pub reward: Reward,
    /// Reward items, `(wcid, count)`; with `random`, every alternative's items.
    pub gives: Vec<(u32, u32)>,
    /// A random reply: one of its alternatives' items is given, drawn by chance.
    pub random: bool,
    /// Quest flags the reward stamps, lowercased.
    pub stamps: Vec<String>,
    pub repeat: Repeat,
    /// The quest the wait or once-flag is kept under; empty when it repeats any time.
    pub timer: &'static str,
    /// Flags held first; a kill row needs its own counter, which its start row stamps.
    pub needs: Vec<String>,
    pub item: Option<Item>,
    /// What the path does, in the data's words (`xp`, `item`, `flag`, `luminance`, `type N`...).
    pub does: Vec<&'static str>,
    /// Conditions the client cannot check, in words ("a random reply", "a check of stat 134").
    pub unread: Vec<&'static str>,
}

impl Row {
    /// Whether the giver's level band holds `level`.
    pub fn fits_level(&self, level: u32) -> bool {
        level >= self.min_level && (self.max_level == 0 || level <= self.max_level)
    }

    /// Whether the row awards XP, fixed or level-scaled.
    pub fn has_xp(&self) -> bool {
        self.reward.xp > 0 || self.reward.xp_level > 0.0
    }

    /// Whether the row asks something the client cannot check (`unread`).
    pub fn has_unread(&self) -> bool {
        !self.unread.is_empty()
    }

    /// The giver's placement nearest `me` by straight distance; the first with no position.
    pub fn nearest(&self, me: Option<Vec3>) -> Place {
        let away = |p: &&Place| me.map_or(0.0, |m| p.at.distance(m));
        self.places
            .iter()
            .min_by(|a, b| away(a).total_cmp(&away(b)))
            .copied()
            .unwrap_or(Place {
                at: Vec3::ZERO,
                cell: 0,
            })
    }

    /// The straight distance from `me` to the giver's nearest placement.
    pub fn distance_to(&self, me: Vec3) -> f32 {
        self.places
            .iter()
            .map(|p| p.at.distance(me))
            .fold(f32::MAX, f32::min)
    }

    /// The flag the giver is shut behind: every placement is in a dungeon whose working portals
    /// in all ask a flag not in `held` ([`locked_at`]). `None` when one placement is open.
    pub fn locked_by(&self, held: &[String]) -> Option<&'static str> {
        let mut flag = None;
        for p in self.places {
            let f = locked_at(p, held)?;
            flag.get_or_insert(f);
        }
        flag
    }
}

/// The flag a placement in a dungeon is shut behind: every working portal into its landblock
/// asks a flag not in `held`. `None` for a place a walk or an open portal reaches.
pub fn locked_at(place: &Place, held: &[String]) -> Option<&'static str> {
    if !place.is_indoors() {
        return None;
    }
    let block = place.cell >> 16;
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

/// How a source yields what a quest wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Yield {
    /// A corpse drop, at `chance` per kill.
    Drop,
    /// A creature whose death counts for the counter.
    Kill,
    /// A placed NPC whose talk gives the item.
    Npc,
    /// Made on the ground by a generator: walked to and picked up.
    Floor,
    /// Held in a chest, opened with one of its `keys` when it is locked.
    Container,
    /// A shop sells it (`crate::shops`): bought on a town run.
    Vendor,
}

impl Yield {
    /// Whether the source is hunted: a creature killed where it spawns.
    pub fn is_hunted(self) -> bool {
        matches!(self, Yield::Drop | Yield::Kill)
    }
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
    /// `(landblock, spawn entries)` as `hunting::Ground::count` counts; a placed one's `(cell, 1)`.
    pub landblocks: Vec<(u32, u32)>,
    /// A placed source's spots, one per `landblocks` entry; empty for a hunted one.
    pub places: Vec<Place>,
    /// The keys that open a locked container, `(wcid, name)`: each a want with sources of its own.
    pub keys: Vec<(u32, String)>,
}

impl Source {
    /// The spot nearest `me` by straight distance; the first with no position.
    pub fn nearest(&self, me: Option<Vec3>) -> Option<Place> {
        let away = |p: &&Place| me.map_or(0.0, |m| p.at.distance(m));
        self.places
            .iter()
            .min_by(|a, b| away(a).total_cmp(&away(b)))
            .copied()
    }
}

const QUESTS: &str = include_str!("../data/quests.csv");
const GIVERS: &str = include_str!("../data/quest_givers.csv");
const SOURCES: &str = include_str!("../data/quest_sources.csv");

fn words(s: &str) -> Vec<String> {
    s.split(';')
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

fn split_on(s: &'static str, sep: char) -> Vec<&'static str> {
    s.split(sep).filter(|w| !w.is_empty()).collect()
}

/// Each giver's placements, grouped by wcid, and its name and range into them.
struct Givers {
    places: Vec<Place>,
    by_wcid: BTreeMap<u32, (&'static str, std::ops::Range<usize>)>,
}

fn parse_giver(line: &'static str) -> Option<(u32, &'static str, Place)> {
    let f: Vec<&'static str> = line.split(',').collect();
    if f.len() != 6 {
        return None;
    }
    let cell = u32::from_str_radix(f[2], 16).ok()?;
    let local = Vec3::new(f[3].parse().ok()?, f[4].parse().ok()?, f[5].parse().ok()?);
    let at = crate::landblock_origin(cell) + local;
    Some((f[0].parse().ok()?, f[1], Place { at, cell }))
}

fn givers() -> &'static Givers {
    static PARSED: OnceLock<Givers> = OnceLock::new();
    PARSED.get_or_init(|| {
        let mut all: Vec<(u32, &'static str, Place)> =
            lines(GIVERS).filter_map(parse_giver).collect();
        all.sort_by_key(|g| g.0);
        let mut by_wcid = BTreeMap::new();
        for (i, &(wcid, name, _)) in all.iter().enumerate() {
            let e = by_wcid.entry(wcid).or_insert((name, i..i));
            e.1.end = i + 1;
        }
        Givers {
            places: all.into_iter().map(|g| g.2).collect(),
            by_wcid,
        }
    })
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

fn parse_row(line: &'static str) -> Option<Row> {
    let f: Vec<&'static str> = line.split(',').collect();
    if f.len() != 24 {
        return None;
    }
    let kind = match f[1] {
        "kill" => Kind::Kill,
        "item" => Kind::Item,
        "talk" => Kind::Talk,
        "start" => Kind::Start,
        _ => return None,
    };
    let repeat = match f[16].parse::<i64>().ok()? {
        0 => Repeat::Any,
        -1 => Repeat::Once,
        s => Repeat::After(u32::try_from(s).ok()?),
    };
    let item = match f[19] {
        "" => None,
        name => Some(Item {
            wcid: f[5].parse().ok()?,
            name,
            item_type: u32::from_str_radix(f[20], 16).ok()?,
            value: f[21].parse().ok()?,
        }),
    };
    let giver_wcid = f[3].parse().ok()?;
    let table = givers();
    let (giver, range) = table.by_wcid.get(&giver_wcid)?;
    let mut gives: Vec<(u32, u32)> = Vec::new();
    for alt in f[14].split('|') {
        for g in pairs(alt, 10)? {
            if !gives.contains(&g) {
                gives.push(g);
            }
        }
    }
    Some(Row {
        id: f[0],
        kind,
        title: f[2],
        giver_wcid,
        giver,
        places: &table.places[range.clone()],
        hand: if f[4] == "give" {
            Handing::Give
        } else {
            Handing::Talk
        },
        want: want_of(kind, f[5])?,
        count: f[6].parse().ok()?,
        min_level: f[7].parse().ok()?,
        max_level: f[8].parse().ok()?,
        reward: Reward {
            xp: f[9].parse().ok()?,
            xp_level: f[10].parse().ok()?,
            xp_min: f[11].parse().ok()?,
            xp_max: f[12].parse().ok()?,
            share: match f[13] {
                "all" => Share::All,
                "allegiance" => Share::Allegiance,
                _ => Share::None,
            },
        },
        gives,
        random: f[14].contains('|'),
        stamps: words(f[15]),
        repeat,
        timer: f[17],
        needs: words(f[18]),
        item,
        does: split_on(f[22], ';'),
        unread: split_on(f[23], '|'),
    })
}

fn parse_spots(cells: &[(u32, u32)], at: &str) -> Option<Vec<Place>> {
    if at.is_empty() {
        return Some(Vec::new());
    }
    let spots: Vec<&str> = at.split(';').collect();
    if spots.len() != cells.len() {
        return None;
    }
    let mut out = Vec::new();
    for (&(cell, _), spot) in cells.iter().zip(spots) {
        let xyz: Vec<f32> = spot
            .split(' ')
            .map(str::parse)
            .collect::<Result<_, _>>()
            .ok()?;
        let [x, y, z] = xyz[..] else { return None };
        let at = crate::landblock_origin(cell) + Vec3::new(x, y, z);
        out.push(Place { at, cell });
    }
    Some(out)
}

fn parse_keys(s: &str) -> Option<Vec<(u32, String)>> {
    s.split(';')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let (wcid, name) = w.split_once('=')?;
            Some((wcid.parse().ok()?, name.to_string()))
        })
        .collect()
}

/// One row: the first seven columns, and a placed source's spots and a lock's keys when there.
fn parse_source(line: &str) -> Option<Source> {
    let f: Vec<&str> = line.split(',').collect();
    if f.len() != 7 && f.len() != 9 {
        return None;
    }
    let kind = match f[1] {
        "drop" => Yield::Drop,
        "kill" => Yield::Kill,
        "npc" => Yield::Npc,
        "floor" => Yield::Floor,
        "container" => Yield::Container,
        _ => return None,
    };
    let landblocks = pairs(f[6], 16)?;
    let (at, keys) = if f.len() == 9 { (f[7], f[8]) } else { ("", "") };
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
        places: parse_spots(&landblocks, at)?,
        landblocks,
        keys: parse_keys(keys)?,
    })
}

/// Every shop placement selling an item a quest or a key wants, as a source.
fn vendor_sources(parsed: &[Source]) -> Vec<Source> {
    let mut wants: std::collections::BTreeSet<u32> = std::collections::BTreeSet::new();
    for r in all() {
        if let Want::Item(w) = r.want {
            wants.insert(w);
        }
    }
    for s in parsed {
        if let Want::Item(w) = s.want {
            wants.insert(w);
        }
        wants.extend(s.keys.iter().map(|k| k.0));
    }
    let mut out = Vec::new();
    for shop in crate::shops::all() {
        for w in shop.sells.iter().filter(|w| wants.contains(&w.wcid)) {
            out.push(Source {
                want: Want::Item(w.wcid),
                kind: Yield::Vendor,
                wcid: shop.wcid,
                name: shop.name.clone(),
                level: 0,
                chance: 1.0,
                landblocks: vec![(shop.cell, 1)],
                places: vec![shop.place()],
                keys: Vec::new(),
            });
        }
    }
    out
}

fn lines(text: &'static str) -> impl Iterator<Item = &'static str> {
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
}

/// Every row, parsed on first use, in id order (the file's).
pub fn all() -> &'static [Row] {
    static ROWS: OnceLock<Vec<Row>> = OnceLock::new();
    ROWS.get_or_init(|| {
        let mut rows: Vec<Row> = lines(QUESTS).filter_map(parse_row).collect();
        rows.sort_by_key(|r| r.id);
        rows
    })
}

/// Every source, parsed on first use, the shops selling a wanted item joined after the file's.
pub fn sources() -> &'static [Source] {
    static SOURCES_PARSED: OnceLock<Vec<Source>> = OnceLock::new();
    SOURCES_PARSED.get_or_init(|| {
        let mut all: Vec<Source> = lines(SOURCES).filter_map(parse_source).collect();
        let shops = vendor_sources(&all);
        all.extend(shops);
        all
    })
}

/// The rows of one quest, its start rows among them.
pub fn rows_of(id: &str) -> impl Iterator<Item = &'static Row> + '_ {
    static BY_ID: OnceLock<BTreeMap<&'static str, std::ops::Range<usize>>> = OnceLock::new();
    let by_id = BY_ID.get_or_init(|| {
        let mut m: BTreeMap<&'static str, std::ops::Range<usize>> = BTreeMap::new();
        for (i, r) in all().iter().enumerate() {
            m.entry(r.id).or_insert(i..i).end = i + 1;
        }
        m
    });
    let range = by_id.get(id).cloned().unwrap_or(0..0);
    all()[range].iter()
}

/// Where what `want` asks for comes from.
pub fn sources_of(want: &Want) -> impl Iterator<Item = &'static Source> + '_ {
    static BY_WANT: OnceLock<BTreeMap<&'static Want, Vec<&'static Source>>> = OnceLock::new();
    let by_want = BY_WANT.get_or_init(|| {
        let mut m: BTreeMap<&'static Want, Vec<&'static Source>> = BTreeMap::new();
        for s in sources() {
            m.entry(&s.want).or_default().push(s);
        }
        m
    });
    by_want.get(want).into_iter().flatten().copied()
}

/// The rows whose reward or talk stamps `flag`.
pub fn stamped_by(flag: &str) -> impl Iterator<Item = &'static Row> + '_ {
    all()
        .iter()
        .filter(move |r| r.stamps.iter().any(|s| s.eq_ignore_ascii_case(flag)))
}

#[cfg(test)]
mod tests;
