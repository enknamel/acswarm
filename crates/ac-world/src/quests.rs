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

/// How the giver is approached: handed the item, or used (talked to); or no giver at all, the
/// wanted scroll read for the recall spell it teaches (a `learn:SPELL` row).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handing {
    Give,
    Talk,
    Read,
    /// The wanted item, kept, used while the body overlaps the giver: an object hearing the
    /// item's signal (Gem.cs:69-72, Landblock.cs:985-998), radius 0 for the Aetheria fields.
    Use,
}

/// What a row asks for.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Want {
    /// An item, by weenie class.
    Item(u32),
    /// Kills counted under this quest counter (a creature's KillQuest string, lowercased).
    Kill(String),
    /// A server event started (EventManager.cs), lower case: what a source's [`Source::event`] asks.
    Event(String),
    /// A flag a creature's death stamps on its killer (EmoteManager.cs:1998-2008), lower case:
    /// what a portal asks (the Guardian of the Singularity's VirindiIsland).
    Flag(String),
    /// A quest a creature's death stamps on its killer's fellowship (StampFellowQuest,
    /// EmoteManager.cs:1252-1263), lower case: what a `fellow:` need asks (Kivik Lir's).
    Fellow(String),
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
    /// Every placement of the giver (`quest_givers.csv`), empty only for a scroll read.
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
    /// The gives column as `quests.csv` writes it: with `giver_wcid`, which way to complete it this is.
    pub gives_column: &'static str,
    /// A random reply: one of its alternatives' items is given, drawn by chance.
    pub random: bool,
    /// Quest flags the reward stamps, lowercased.
    pub stamps: Vec<String>,
    pub repeat: Repeat,
    /// The quest the wait or once-flag is kept under; empty when it repeats any time.
    pub timer: &'static str,
    /// Flags held first; a kill row needs its own counter, which its start row stamps. Also
    /// `intSTAT=VALUE` ([`stat_need`]) and `fellow:QUEST` ([`fellow_need`]).
    pub needs: Vec<String>,
    pub item: Option<Item>,
    /// Luminance the path awards (AwardLuminance, EmoteManager.cs:135-140).
    pub luminance: u64,
    /// What the path does, in the data's words (`xp`, `item`, `flag`, `luminance`, `type N`...).
    pub does: Vec<&'static str>,
    /// Conditions the client cannot check, in words ("a random reply", "a check of stat 134").
    pub unread: Vec<&'static str>,
    /// Creatures `(wcid, count)` the giver generates, all killed shortly before a use: their
    /// deaths start the events it asks (EmoteManager.cs:1287-1295, 300 s for the Aetheria wisps).
    pub clears: Vec<(u32, u32)>,
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

    /// The spell a `learn:SPELL` row's scroll teaches: done once the spellbook holds it.
    pub fn teaches(&self) -> Option<u32> {
        self.id.strip_prefix("learn:")?.parse().ok()
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
    /// Made by using one of its `keys`, carried, on this placed object (a recipe's target).
    Use,
    /// Made by using its first key on its second, both carried: nowhere to go (a town stamp on
    /// a Lucky Gold Letter).
    Combine,
    /// A shop sells it (`crate::shops`): bought on a town run.
    Vendor,
    /// An event started by handing one of its `keys` to this NPC.
    Give,
    /// A portal stamping the flag on whoever goes through it (its Portal emote set: Portal.cs:287),
    /// in the world from the landblock's load (placed, or a generator's that fills then).
    Portal,
    /// As [`Yield::Portal`], made only once something reveals it (a Colosseum room's waves passed:
    /// `crate::arenas`); where its generator puts it.
    Revealed,
    /// An object whose use starts the event once the events in `needs` run (a Sealed Valve; the
    /// Font Guardian, its four heat events running).
    Start,
    /// A watcher whose heartbeat starts the event once the events in `needs` run: had by having
    /// them, nothing used (the Temple Consort Controller's four summons: EmoteManager.cs:1904-1914).
    Watch,
}

impl Yield {
    /// Whether the source is hunted: a creature killed where it spawns.
    pub fn is_hunted(self) -> bool {
        matches!(self, Yield::Drop | Yield::Kill)
    }
}

/// How fast a hunted creature comes on one landblock, from a simulation of its generators
/// (`reference/scripts/data/quests_walk.py`, WorldObject_Generators.cs).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpawnRate {
    /// How many stand there once the landblock loads (Landblock.cs:269-303).
    pub load: f32,
    /// How many its generators make an hour while everything they make is killed.
    pub per_hour: f32,
    /// The others an hour that clearing kills, when killing it alone leaves its generators'
    /// slots held (a share of a region roll); 0 when its slots are its own (a linked placement).
    pub others: f32,
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
    /// A placed source's spots, one per `landblocks` entry; a hunted one's only when the encounter
    /// table alone spawns it (each landblock's busiest encounter corner).
    pub places: Vec<Place>,
    /// The keys that open a locked container, or the items used on a use's object, `(wcid, name)`:
    /// each a want with sources of its own.
    pub keys: Vec<(u32, String)>,
    /// A hunted source's rate on each `landblocks` entry; empty when the data has none.
    pub rates: Vec<SpawnRate>,
    /// The server event (lower case) whose generator alone puts the creature or NPC out: it comes
    /// only once that event is started, and its rates are those it then has.
    pub event: Option<String>,
    /// For a [`Yield::Start`] or [`Yield::Watch`], the events (lower case) it asks running:
    /// started first.
    pub needs: Vec<String>,
    /// For a floor item, seconds its generator takes to put it back once taken.
    pub respawn: Option<f32>,
    /// For a [`Yield::Start`], the level band its use asks, `(lo, hi)` with hi 0 open (the
    /// Guardian of the High Matriarch's 40-69, 70-99 and 100 up); `None` for any level.
    pub band: Option<(u32, u32)>,
    /// For a [`Yield::Start`], seconds its use's harm runs on the user (the Eye of the Watcher's
    /// drains, 292 s); `None` when it casts none.
    pub drain: Option<f32>,
}

impl Source {
    /// Whether its use's level band holds `level`.
    pub fn fits_level(&self, level: u32) -> bool {
        self.band
            .is_none_or(|(lo, hi)| level >= lo && (hi == 0 || level <= hi))
    }

    /// How fast it comes on `landblock`: the data's figure, else its spawn entries there taken
    /// for the load and none an hour.
    pub fn rate_at(&self, landblock: u32) -> SpawnRate {
        let i = self.landblocks.iter().position(|&(lb, _)| lb == landblock);
        match i.and_then(|i| self.rates.get(i).map(|r| (i, *r))) {
            Some((_, r)) => r,
            None => SpawnRate {
                load: i.map_or(0.0, |i| self.landblocks[i].1 as f32),
                ..SpawnRate::default()
            },
        }
    }

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
const PICKUPS: &str = include_str!("../data/pickups.csv");

/// An item's pickup limit (its Quest, PropertyString 33): picked up while `stamp` waits, the
/// server says "You have solved this quest too recently!" (Player_Inventory.cs:1249-1262).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pickup {
    /// The quest the limit is kept under, lower case as the quest log keys it.
    pub stamp: &'static str,
    /// Seconds between pickups, 0 for none.
    pub wait: u32,
    /// The most pickups ever, -1 for no limit.
    pub solves: i32,
}

/// The pickup limit of item `wcid`, when the server keeps one.
pub fn pickup_limit(wcid: u32) -> Option<Pickup> {
    static BY_WCID: OnceLock<BTreeMap<u32, Pickup>> = OnceLock::new();
    BY_WCID
        .get_or_init(|| {
            PICKUPS
                .lines()
                .filter(|l| l.starts_with(|c: char| c.is_ascii_digit()))
                .filter_map(|l| {
                    let f: Vec<&'static str> = l.split(',').collect();
                    let limit = Pickup {
                        stamp: f.get(1)?,
                        wait: f.get(2)?.parse().ok()?,
                        solves: f.get(3)?.parse().ok()?,
                    };
                    Some((f.first()?.parse().ok()?, limit))
                })
                .collect()
        })
        .get(&wcid)
        .copied()
}

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
    // Leaked once into the OnceLock: the 38 names with a comma back (see `crate::data_name`).
    let name: &'static str = if f[1].contains(';') {
        Box::leak(crate::data_name(f[1]).into_boxed_str())
    } else {
        f[1]
    };
    Some((f[0].parse().ok()?, name, Place { at, cell }))
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
    // A 26th column, clears, only where there is one.
    if f.len() != 25 && f.len() != 26 {
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
    let hand = match f[4] {
        "give" => Handing::Give,
        "read" => Handing::Read,
        "use" => Handing::Use,
        _ => Handing::Talk,
    };
    // A scroll read has no giver.
    let (giver, places): (&'static str, &'static [Place]) = match table.by_wcid.get(&giver_wcid) {
        Some((giver, range)) => (giver, &table.places[range.clone()]),
        None if hand == Handing::Read => ("", &[]),
        None => return None,
    };
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
        places,
        hand,
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
        gives_column: f[14],
        random: f[14].contains('|'),
        stamps: words(f[15]),
        repeat,
        timer: f[17],
        needs: words(f[18]),
        item,
        luminance: f[22].parse().ok()?,
        does: split_on(f[23], ';'),
        unread: split_on(f[24], '|'),
        clears: pairs(f.get(25).copied().unwrap_or(""), 10)?,
    })
}

/// The stat and value a `intSTAT=VALUE` need or stamp names: a character stat (PropertyInt) set
/// to, or asked to be, that value. `None` for a quest flag.
pub fn stat_need(flag: &str) -> Option<(u32, i32)> {
    let (stat, value) = flag.strip_prefix("int")?.split_once('=')?;
    Some((stat.parse().ok()?, value.parse().ok()?))
}

/// The quest a `fellow:QUEST` need asks stamped on the character's fellowship (InqFellowQuest:
/// in one, and holding it: EmoteManager.cs:500-517). `None` for any other need.
pub fn fellow_need(need: &str) -> Option<&str> {
    need.strip_prefix("fellow:")
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
        // A hunted source's entry is a landblock (`10FC`), a placed one's a cell.
        let cell = if cell <= 0xFFFF { cell << 16 } else { cell };
        let local = Vec3::new(x, y, z);
        let at = crate::landblock_origin(cell) + local;
        let cell = if cell & 0xFFFF == 0 {
            crate::outdoor_cell(cell, local)
        } else {
            cell
        };
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

/// Each `landblocks` entry's `load:per_hour[:others]`, `;`-separated; empty for none known.
fn parse_rates(s: &str, entries: usize) -> Option<Vec<SpawnRate>> {
    if s.is_empty() {
        return Some(Vec::new());
    }
    let rates: Vec<SpawnRate> = s
        .split(';')
        .map(|r| {
            let mut f = r.split(':').map(str::parse::<f32>);
            let rate = SpawnRate {
                load: f.next()?.ok()?,
                per_hour: f.next()?.ok()?,
                others: f.next().unwrap_or(Ok(0.0)).ok()?,
            };
            f.next().is_none().then_some(rate)
        })
        .collect::<Option<_>>()?;
    (rates.len() == entries).then_some(rates)
}

/// One row: the first seven columns, a placed source's spots and a lock's keys when there, and a
/// hunted one's rates (the tenth, a creature's spawn share, is read by [`spawn_share`]).
fn parse_source(line: &str) -> Option<Source> {
    let f: Vec<&str> = line.split(',').collect();
    if ![7, 9, 10, 11, 12, 13, 14, 15, 16].contains(&f.len()) {
        return None;
    }
    let kind = match f[1] {
        "drop" => Yield::Drop,
        "kill" => Yield::Kill,
        "npc" => Yield::Npc,
        "floor" => Yield::Floor,
        "container" => Yield::Container,
        "use" => Yield::Use,
        "combine" => Yield::Combine,
        "give" => Yield::Give,
        "portal" => Yield::Portal,
        "revealed" => Yield::Revealed,
        "start" => Yield::Start,
        "watch" => Yield::Watch,
        _ => return None,
    };
    let landblocks = pairs(f[6], 16)?;
    let (at, keys) = if f.len() >= 9 { (f[7], f[8]) } else { ("", "") };
    Some(Source {
        want: match (f[0].strip_prefix("event:"), f[0].strip_prefix("flag:")) {
            (Some(event), _) => Want::Event(event.to_string()),
            (_, Some(flag)) => Want::Flag(flag.to_string()),
            _ if fellow_need(f[0]).is_some() => Want::Fellow(f[0][7..].to_string()),
            _ if kind == Yield::Kill => Want::Kill(f[0].to_string()),
            _ => Want::Item(f[0].parse().ok()?),
        },
        kind,
        wcid: f[2].parse().ok()?,
        name: crate::data_name(f[3]),
        level: f[4].parse().ok()?,
        chance: f[5].parse().ok()?,
        places: parse_spots(&landblocks, at)?,
        rates: parse_rates(f.get(10).copied().unwrap_or(""), landblocks.len())?,
        landblocks,
        keys: parse_keys(keys)?,
        event: f
            .get(11)
            .filter(|e| !e.is_empty() && !matches!(kind, Yield::Start | Yield::Watch))
            .map(|e| e.to_string()),
        needs: match (kind, f.get(11)) {
            (Yield::Start | Yield::Watch, Some(e)) => e
                .split(';')
                .filter(|n| !n.is_empty())
                .map(String::from)
                .collect(),
            _ => Vec::new(),
        },
        respawn: f.get(12).and_then(|r| r.parse().ok()),
        band: match f.get(14).filter(|b| !b.is_empty()) {
            Some(b) => {
                let (lo, hi) = b.split_once('-')?;
                Some((lo.parse().ok()?, hi.parse().ok()?))
            }
            None => None,
        },
        drain: f.get(15).and_then(|d| d.parse().ok()),
    })
}

/// Every shop placement selling an item a quest, a key or a way in (a portal gem) wants, as a source.
fn vendor_sources(parsed: &[Source]) -> Vec<Source> {
    let mut wants: std::collections::BTreeSet<u32> =
        crate::gems::all().iter().map(|g| g.wcid).collect();
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
                rates: Vec::new(),
                event: None,
                needs: Vec::new(),
                respawn: None,
                band: None,
                drain: None,
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

/// The best share of a generator roll that puts creature `wcid` in the world, 1 a sure spawn
/// (WorldObject_Generators.cs:108-213): the Mosswart Mire-Witch's 0.01 is a rare one. 1 when no
/// generator chain is known.
pub fn spawn_share(wcid: u32) -> f32 {
    static SHARES: OnceLock<BTreeMap<u32, f32>> = OnceLock::new();
    let shares = SHARES.get_or_init(|| {
        let mut m = BTreeMap::new();
        for f in lines(SOURCES).map(|l| l.split(',').collect::<Vec<_>>()) {
            let (Some(Ok(wcid)), Some(Ok(share))) = (
                f.get(2).map(|w| w.parse::<u32>()),
                f.get(9).map(|s| s.parse::<f32>()),
            ) else {
                continue;
            };
            m.insert(wcid, share);
        }
        m
    });
    shares.get(&wcid).copied().unwrap_or(1.0)
}

/// The deaths of creature `wcid` event `event`'s watcher must hear before it starts it (the
/// sources' fourteenth column: the Stone Fists Event Watcher's eight golems); 1 for any other kill.
pub fn deaths_heard(event: &str, wcid: u32) -> u32 {
    static HEARD: OnceLock<BTreeMap<(String, u32), u32>> = OnceLock::new();
    let heard = HEARD.get_or_init(|| {
        let mut m = BTreeMap::new();
        for f in lines(SOURCES).map(|l| l.split(',').collect::<Vec<_>>()) {
            let (Some(e), Some(Ok(w)), Some(Ok(n))) = (
                f[0].strip_prefix("event:"),
                f.get(2).map(|w| w.parse::<u32>()),
                f.get(13).map(|n| n.parse::<u32>()),
            ) else {
                continue;
            };
            m.insert((e.to_string(), w), n);
        }
        m
    });
    heard.get(&(event.to_string(), wcid)).copied().unwrap_or(1)
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
