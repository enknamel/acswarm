//! A quest's chain in the data's terms: who starts it, the flags it needs and who stamps them,
//! where its item or kills come from and the way in, who rewards it; a place in words; and
//! one card per quest for a list's filters.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use glam::{Vec2, Vec3};

use crate::portals::Portal;
use crate::quests::{Kind, Repeat, Row, Source, Want, Yield};
use crate::unlocks::Unlock;
use crate::Place;

/// Who stamps a flag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FlagBy {
    /// A talk to an unlock NPC at its level (`crate::unlocks`).
    Unlock(&'static Unlock),
    /// A quest row's reward or talk (`crate::quests::stamped_by`).
    Quest(&'static Row),
}

/// One place what a quest wants comes from.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceGround {
    pub landblock: u32,
    pub source: &'static Source,
    /// The source's spawn entries there.
    pub entries: u32,
    /// Where a journey to hunt there ends; `None` when no ground or portal is known.
    pub place: Option<Place>,
    /// The portal in, when only a portal reaches it: its level and flag gate the hunt.
    pub way_in: Option<&'static Portal>,
}

/// One link of a quest's chain, in the order a player does them.
#[derive(Clone, Debug, PartialEq)]
pub enum ChainLink {
    /// Talk to a giver who starts it (a `Kind::Start` row, at any of its placements).
    TakeUp(&'static Row),
    /// A flag held first, who stamps it (empty: nobody known), and the portal asking it, if one.
    Flag {
        flag: String,
        by: Vec<FlagBy>,
        portal: Option<&'static Portal>,
    },
    /// Kill or loot it, best place first.
    Hunt(Vec<SourceGround>),
    /// An NPC hands the item out.
    Fetch(&'static Source),
    /// Hand it in to any of these rows' givers.
    HandIn(Vec<&'static Row>),
}

/// One quest's facets for the Quests window's filters, built once from the data.
#[derive(Clone, Debug, PartialEq)]
pub struct QuestCard {
    pub id: String,
    /// The title, or the wanted item's name when the title is internal.
    pub title: String,
    pub kind: Kind,
    /// The first wanted item: it names a quest whose title is internal.
    pub item: Option<String>,
    /// Every wanted item and every kind of its rows (start talks left out), for the filters.
    pub items: Vec<String>,
    pub kinds: Vec<Kind>,
    /// Giver names with their places in words, one per placement.
    pub givers: Vec<(String, String)>,
    /// Source creatures or NPCs: name, level, chance per kill.
    pub creatures: Vec<(String, u32, f32)>,
    /// Source places in words.
    pub places: Vec<String>,
    /// The giver band, 0 for open.
    pub levels: (u32, u32),
    /// The easiest source's level.
    pub easiest: Option<u32>,
    /// The way-in portal's level, 0 for none.
    pub entry: u32,
    /// The fixed XP (the board gives it at the character's level).
    pub xp: u64,
    /// Reward items in words ("Trade Note (75,000) x1", "item 31606"); a random reply's are
    /// alternatives.
    pub gives: Vec<String>,
    pub repeat: Repeat,
    /// Flags it needs: its rows' needs and its way in's.
    pub flags: Vec<String>,
    /// What its rows do, in the data's words (`xp`, `item`, `flag`, `luminance`...).
    pub does: Vec<String>,
    /// What its rows ask that the client cannot check, in words.
    pub unread: Vec<String>,
}

/// Map units from a gazetteer pin that read as the town itself, and as near it.
const IN_TOWN: f32 = 1.0;
const NEAR_TOWN: f32 = 5.0;

/// Title openings the data uses for timers and stamps rather than a quest's name.
const INTERNAL_TITLES: &[&str] = &[
    "player ",
    "you ",
    "you've ",
    "quest timer",
    "countdown timer",
    "cooldown",
    "kill counter",
    "permanent - ",
];

fn block_of(cell: u32) -> u32 {
    cell >> 16
}

fn has_flag(p: &Portal) -> bool {
    !p.quest.is_empty()
}

fn has_word(words: &[String], w: &str) -> bool {
    words.iter().any(|x| x.eq_ignore_ascii_case(w))
}

/// Working portals into each landblock from another, lowest level first (data order on a tie).
fn ways_by_block() -> &'static BTreeMap<u32, Vec<&'static Portal>> {
    static WAYS: OnceLock<BTreeMap<u32, Vec<&'static Portal>>> = OnceLock::new();
    WAYS.get_or_init(|| {
        let mut m: BTreeMap<u32, Vec<&'static Portal>> = BTreeMap::new();
        for p in crate::portals::all() {
            if p.works() && block_of(p.to_cell) != block_of(p.from_cell) {
                m.entry(block_of(p.to_cell)).or_default().push(p);
            }
        }
        for v in m.values_mut() {
            v.sort_by_key(|p| p.min_level);
        }
        m
    })
}

/// Working portals into `landblock` from another, lowest level first, flagged ones included.
pub fn ways_in(landblock: u32) -> Vec<&'static Portal> {
    ways_by_block().get(&landblock).cloned().unwrap_or_default()
}

/// Landblocks with a known spot under the sky: a portal's outdoor end, an outdoor landmark,
/// giver or hunting ground. Any other is taken for a dungeon.
fn surface_blocks() -> &'static BTreeSet<u32> {
    static SURFACE: OnceLock<BTreeSet<u32>> = OnceLock::new();
    SURFACE.get_or_init(|| {
        let mut s = BTreeSet::new();
        for p in crate::portals::all() {
            if p.mouth_outdoors() {
                s.insert(block_of(p.from_cell));
            }
            if p.exit_outdoors() {
                s.insert(block_of(p.to_cell));
            }
        }
        let landmarks = crate::landmarks::all().iter().filter(|l| l.outdoors());
        s.extend(landmarks.map(|l| block_of(l.cell)));
        for r in crate::quests::all() {
            let outdoors = r.places.iter().filter(|p| !p.is_indoors());
            s.extend(outdoors.map(|p| block_of(p.cell)));
        }
        let open = crate::hunting::all().iter().filter(|g| !g.is_dungeon());
        s.extend(open.map(|g| g.landblock));
        s
    })
}

fn is_surface(landblock: u32) -> bool {
    surface_blocks().contains(&landblock)
}

fn ground_at(landblock: u32) -> Option<&'static crate::hunting::Ground> {
    static BY_BLOCK: OnceLock<BTreeMap<u32, &'static crate::hunting::Ground>> = OnceLock::new();
    BY_BLOCK
        .get_or_init(|| {
            crate::hunting::all()
                .iter()
                .map(|g| (g.landblock, g))
                .collect()
        })
        .get(&landblock)
        .copied()
}

/// Who stamps `flag`: unlock NPCs, then quest rows; empty when nobody known.
pub fn flag_givers(flag: &str) -> Vec<FlagBy> {
    let unlocks = crate::unlocks::all()
        .iter()
        .filter(|u| u.flag.eq_ignore_ascii_case(flag))
        .map(FlagBy::Unlock);
    unlocks
        .chain(crate::quests::stamped_by(flag).map(FlagBy::Quest))
        .collect()
}

/// The portal a source landblock is entered by when only a portal reaches it: a hunting
/// ground's own entrance, else the lowest way in asking no flag, else the lowest.
fn entry_portal(landblock: u32) -> Option<&'static Portal> {
    let ways = ways_by_block().get(&landblock)?;
    if let Some(e) = ground_at(landblock).and_then(|g| g.entrance) {
        return ways
            .iter()
            .find(|p| p.to_cell == e.cell && !has_flag(p))
            .copied();
    }
    if is_surface(landblock) {
        return None;
    }
    ways.iter()
        .find(|p| !has_flag(p))
        .or_else(|| ways.first())
        .copied()
}

fn source_ground(source: &'static Source, landblock: u32, entries: u32) -> SourceGround {
    let way_in = entry_portal(landblock);
    // The way in lands where `Ground::place` would, without its scan of every portal.
    let place = match (way_in, ground_at(landblock)) {
        (Some(p), _) => Some(Place {
            at: p.to,
            cell: p.to_cell,
        }),
        (None, Some(g)) => Some(g.place()),
        (None, None) if is_surface(landblock) => {
            let middle = crate::landblock_origin(landblock << 16) + Vec3::new(96.0, 96.0, 0.0);
            Some(Place::on_map(middle.truncate()))
        }
        (None, None) => None,
    };
    SourceGround {
        landblock,
        source,
        entries,
        place,
        way_in,
    }
}

/// Where `want` comes from, best first (spawn entries x chance, then the lower way in).
pub fn source_grounds(want: &Want) -> Vec<SourceGround> {
    let mut grounds: Vec<SourceGround> = crate::quests::sources_of(want)
        .filter(|s| s.kind != Yield::Npc)
        .flat_map(|s| {
            s.landblocks
                .iter()
                .map(move |&(lb, entries)| source_ground(s, lb, entries))
        })
        .collect();
    let level = |g: &SourceGround| g.way_in.map_or(0, |p| p.min_level);
    let yields = |g: &SourceGround| g.entries as f32 * g.source.chance;
    grounds.sort_by(|a, b| {
        (b.place.is_some().cmp(&a.place.is_some()))
            .then(yields(b).total_cmp(&yields(a)))
            .then(level(a).cmp(&level(b)))
    });
    grounds
}

fn push_flag(links: &mut Vec<ChainLink>, flag: &str, portal: Option<&'static Portal>) {
    let listed = links
        .iter()
        .any(|l| matches!(l, ChainLink::Flag { flag: f, .. } if f.eq_ignore_ascii_case(flag)));
    if !listed {
        links.push(ChainLink::Flag {
            flag: flag.to_string(),
            by: flag_givers(flag),
            portal,
        });
    }
}

/// The flags every completing row needs first, but for those the quest's own start rows stamp.
fn needs_first(doing: &[&'static Row], starts: &[&'static Row]) -> Vec<String> {
    let Some(first) = doing.first() else {
        return Vec::new();
    };
    let own: Vec<String> = starts.iter().flat_map(|r| r.stamps.clone()).collect();
    first
        .needs
        .iter()
        .filter(|n| doing.iter().all(|r| has_word(&r.needs, n)))
        .filter(|n| !has_word(&own, n))
        .cloned()
        .collect()
}

/// The flag each giver's landblock is shut behind with none held ([`crate::quests::locked_at`]
/// reads every portal, so it is read once a landblock).
fn block_lock(block: u32) -> Option<&'static str> {
    static LOCKS: OnceLock<BTreeMap<u32, Option<&'static str>>> = OnceLock::new();
    let locks = LOCKS.get_or_init(|| {
        let mut m = BTreeMap::new();
        let indoors = crate::quests::all().iter().flat_map(|r| r.places);
        for p in indoors.filter(|p| p.is_indoors()) {
            m.entry(block_of(p.cell))
                .or_insert_with(|| crate::quests::locked_at(p, &[]));
        }
        m
    });
    locks.get(&block).copied().flatten()
}

/// The flag every completing row's giver is shut behind ([`Row::locked_by`] with none held).
fn giver_lock(doing: &[&'static Row]) -> Option<&'static str> {
    let mut flag = None;
    for p in doing.iter().flat_map(|r| r.places) {
        let shut = if p.is_indoors() {
            block_lock(block_of(p.cell))
        } else {
            None
        };
        flag.get_or_insert(shut?);
    }
    flag
}

/// The completing rows grouped by what they ask, talks first, then in the data's order.
fn by_want(doing: &[&'static Row]) -> Vec<(&'static Want, Vec<&'static Row>)> {
    let mut groups: Vec<(&'static Want, Vec<&'static Row>)> = Vec::new();
    for &r in doing {
        match groups.iter_mut().find(|g| *g.0 == r.want) {
            Some(g) => g.1.push(r),
            None => groups.push((&r.want, vec![r])),
        }
    }
    groups.sort_by_key(|g| *g.0 != Want::Nothing);
    groups
}

/// Quest `id`'s chain, in order.
pub fn chain_of(id: &str) -> Vec<ChainLink> {
    let (starts, doing): (Vec<&'static Row>, Vec<&'static Row>) =
        crate::quests::rows_of(id).partition(|r| r.kind == Kind::Start);
    let mut links: Vec<ChainLink> = Vec::new();
    let mut givers: Vec<u32> = Vec::new();
    for &r in &starts {
        if !givers.contains(&r.giver_wcid) {
            givers.push(r.giver_wcid);
            links.push(ChainLink::TakeUp(r));
        }
    }
    for flag in needs_first(&doing, &starts) {
        push_flag(&mut links, &flag, None);
    }
    let mut steps: Vec<ChainLink> = Vec::new();
    for (want, group) in by_want(&doing) {
        if *want != Want::Nothing {
            let grounds = source_grounds(want);
            let gate = grounds
                .first()
                .and_then(|g| g.way_in)
                .filter(|p| has_flag(p));
            if let Some(p) = gate {
                push_flag(&mut links, &p.quest, Some(p));
            }
            let npc = crate::quests::sources_of(want).find(|s| s.kind == Yield::Npc);
            match npc {
                _ if !grounds.is_empty() => steps.push(ChainLink::Hunt(grounds)),
                Some(npc) => steps.push(ChainLink::Fetch(npc)),
                None => {}
            }
        }
        steps.push(ChainLink::HandIn(group));
    }
    if let Some(flag) = giver_lock(&doing) {
        let block = doing[0].places.first().map_or(0, |p| block_of(p.cell));
        let portal = ways_in(block).into_iter().find(|p| p.quest == flag);
        push_flag(&mut links, flag, portal);
    }
    links.extend(steps);
    links
}

fn clean_landing(name: &str) -> &str {
    let name = name.strip_prefix("Portal to ").unwrap_or(name);
    name.strip_prefix("the ").unwrap_or(name)
}

fn is_surface_word(name: &str) -> bool {
    name.to_ascii_lowercase().contains("surface")
}

/// The towns' outdoor landings ("Portal to X") by landblock: a town by the name it is portalled
/// to, as the gazetteer's map pin can sit a dozen map units off (Westwatch).
fn town_landings() -> &'static BTreeMap<u32, Vec<(&'static str, Vec2)>> {
    static TOWNS: OnceLock<BTreeMap<u32, Vec<(&'static str, Vec2)>>> = OnceLock::new();
    TOWNS.get_or_init(|| {
        let mut m: BTreeMap<u32, Vec<(&'static str, Vec2)>> = BTreeMap::new();
        for p in crate::portals::all() {
            let town = clean_landing(&p.name);
            if p.works()
                && p.exit_outdoors()
                && p.name.starts_with("Portal to ")
                && !is_surface_word(town)
                && !p.is_town_network()
            {
                let at = (town, p.to_xy());
                m.entry(block_of(p.to_cell)).or_default().push(at);
            }
        }
        m
    })
}

/// Each dungeon's name: a way in from outdoors first, then the lowest, "Surface" ones left out.
fn dungeon_names() -> &'static BTreeMap<u32, &'static str> {
    static NAMES: OnceLock<BTreeMap<u32, &'static str>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let mut m = BTreeMap::new();
        for (&block, ways) in ways_by_block() {
            let named = |p: &&&'static Portal| !p.exit_outdoors() && !is_surface_word(&p.name);
            let best = ways
                .iter()
                .filter(named)
                .find(|p| p.mouth_outdoors())
                .or_else(|| ways.iter().find(named));
            if let Some(p) = best {
                m.insert(block, clean_landing(&p.name));
            }
        }
        m
    })
}

/// `72.8N, 73.6W`, rounded as `crate::map_coord_str` rounds.
fn map_words(at: Vec2) -> String {
    let (ns, ew) = crate::towns::map_of(at);
    format!(
        "{:.1}{}, {:.1}{}",
        (ns.abs() - 0.05).max(0.0),
        if ns >= 0.0 { "N" } else { "S" },
        (ew.abs() - 0.05).max(0.0),
        if ew >= 0.0 { "E" } else { "W" }
    )
}

/// The town a portal lands in within one landblock of `at`, nearest landing first.
fn landed_town(at: Vec2) -> Option<&'static str> {
    let (bx, by) = ((at.x / 192.0).floor() as i32, (at.y / 192.0).floor() as i32);
    let mut best: Option<(&'static str, f32)> = None;
    for (dx, dy) in (-1..=1).flat_map(|dx| (-1..=1).map(move |dy| (dx, dy))) {
        let (x, y) = (bx + dx, by + dy);
        if !(0..=255).contains(&x) || !(0..=255).contains(&y) {
            continue;
        }
        let block = (x as u32) << 8 | y as u32;
        for &(town, to) in town_landings().get(&block).into_iter().flatten() {
            let d = to.distance(at);
            if best.is_none_or(|b| d < b.1) {
                best = Some((town, d));
            }
        }
    }
    best.map(|b| b.0)
}

/// A surface spot in words: the town a portal lands it in, the gazetteer's town within
/// [`NEAR_TOWN`] map units, else its map coordinates.
fn surface_name(at: Vec2) -> String {
    if let Some(town) = landed_town(at) {
        return town.to_string();
    }
    let (ns, ew) = crate::towns::map_of(at);
    let away = |t: &crate::towns::Town| Vec2::new(t.ns - ns, t.ew - ew).length();
    let town = crate::towns::PLACES
        .iter()
        .min_by(|a, b| away(a).total_cmp(&away(b)));
    match town {
        Some(t) if away(t) <= IN_TOWN => t.name.to_string(),
        Some(t) if away(t) <= NEAR_TOWN => format!("near {}", t.name),
        _ => map_words(at),
    }
}

/// A cell's place in words: its dungeon, the town by its portal, near a town, else map coordinates.
pub fn place_name(cell: u32) -> String {
    let block = block_of(cell);
    let indoors = cell & 0xFFFF >= 0x100;
    if indoors && !is_surface(block) {
        if let Some(name) = dungeon_names().get(&block) {
            return (*name).to_string();
        }
        return match ground_at(block) {
            Some(g) => format!("{} dungeon", g.name),
            None => format!("dungeon {block:04X}"),
        };
    }
    let origin = crate::landblock_origin(cell);
    let at = if indoors || cell & 0xFFFF == 0 {
        origin + Vec3::new(96.0, 96.0, 0.0)
    } else {
        origin + crate::outdoor_cell_centre(cell)
    };
    surface_name(at.truncate())
}

/// Item names by weenie class: wanted items' then shop wares', the data's ';' read back as ','.
fn item_names() -> &'static BTreeMap<u32, String> {
    static NAMES: OnceLock<BTreeMap<u32, String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let mut m = BTreeMap::new();
        for item in crate::quests::all().iter().filter_map(|r| r.item.as_ref()) {
            m.entry(item.wcid)
                .or_insert_with(|| item.name.trim().replace(';', ","));
        }
        for ware in crate::shops::all().iter().flat_map(|s| &s.sells) {
            m.entry(ware.wcid)
                .or_insert_with(|| ware.name.trim().replace(';', ","));
        }
        m
    })
}

/// An item's name from the quest and shop data, by weenie class.
pub fn reward_name(wcid: u32) -> Option<&'static str> {
    item_names().get(&wcid).map(String::as_str)
}

fn is_internal(title: &str) -> bool {
    let lower = title.to_ascii_lowercase();
    INTERNAL_TITLES.iter().any(|t| lower.starts_with(t))
}

fn push_new<T: PartialEq>(v: &mut Vec<T>, x: T) {
    if !v.contains(&x) {
        v.push(x);
    }
}

fn push_word(v: &mut Vec<String>, w: &str) {
    if !has_word(v, w) {
        v.push(w.to_string());
    }
}

/// What a quest's sources give its card.
#[derive(Default)]
struct SourceFacets {
    creatures: Vec<(String, u32, f32)>,
    places: Vec<String>,
    flags: Vec<String>,
    easiest: Option<u32>,
    entry: Option<u32>,
}

fn add_sources(f: &mut SourceFacets, want: &Want) {
    if *want == Want::Nothing {
        return;
    }
    for s in crate::quests::sources_of(want) {
        push_new(&mut f.creatures, (s.name.clone(), s.level, s.chance));
        // An NPC hands it out at no level, as the board's `easiest` counts it.
        let level = if s.kind == Yield::Npc { 0 } else { s.level };
        f.easiest = Some(f.easiest.map_or(level, |e| e.min(level)));
        if s.kind == Yield::Npc {
            for &(cell, _) in &s.landblocks {
                push_new(&mut f.places, place_name(cell));
            }
        }
    }
    for g in source_grounds(want) {
        let cell = g.place.map_or(g.landblock << 16 | 0x100, |p| p.cell);
        push_new(&mut f.places, place_name(cell));
        if let Some(p) = g.way_in.filter(|p| has_flag(p)) {
            push_word(&mut f.flags, &p.quest);
        }
        if g.place.is_some() {
            let level = g.way_in.map_or(0, |p| p.min_level);
            f.entry = Some(f.entry.map_or(level, |e| e.min(level)));
        }
    }
}

fn card_title(main: &Row, item: Option<&str>) -> String {
    let title = main.title.trim();
    if !title.is_empty() && !is_internal(title) {
        return title.to_string();
    }
    match (item, main.kind) {
        (Some(name), _) => name.to_string(),
        (None, Kind::Kill) => format!("Kill task for {}", main.giver),
        (None, _) => format!("Talk to {}", main.giver),
    }
}

fn card_of(rows: &'static [Row]) -> QuestCard {
    let (starts, doing): (Vec<&'static Row>, Vec<&'static Row>) =
        rows.iter().partition(|r| r.kind == Kind::Start);
    let main = doing
        .first()
        .or(starts.first())
        .copied()
        .unwrap_or(&rows[0]);
    let item = rows
        .iter()
        .find_map(|r| r.item.as_ref())
        .map(|i| i.name.trim().replace(';', ","));
    let (mut givers, mut wants) = (Vec::new(), Vec::new());
    let (mut items, mut kinds) = (Vec::new(), Vec::new());
    for r in &doing {
        if let Some(i) = &r.item {
            push_new(&mut items, i.name.trim().replace(';', ","));
        }
        push_new(&mut kinds, r.kind);
    }
    let (mut gives, mut does, mut unread) = (Vec::new(), Vec::new(), Vec::new());
    // Rows of one giver repeat its placements: each is named once.
    let mut placed: BTreeSet<(u32, u32)> = BTreeSet::new();
    for r in rows {
        for p in r.places {
            if placed.insert((r.giver_wcid, p.cell)) {
                push_new(&mut givers, (r.giver.to_string(), place_name(p.cell)));
            }
        }
        push_new(&mut wants, &r.want);
        for &(wcid, n) in &r.gives {
            let name = reward_name(wcid).map_or_else(|| format!("item {wcid}"), str::to_string);
            push_new(&mut gives, format!("{name} x{n}"));
        }
        r.does
            .iter()
            .for_each(|d| push_new(&mut does, d.to_string()));
        r.unread
            .iter()
            .for_each(|u| push_new(&mut unread, u.to_string()));
    }
    let mut src = SourceFacets::default();
    wants.iter().for_each(|w| add_sources(&mut src, w));
    let own: Vec<String> = starts.iter().flat_map(|r| r.stamps.clone()).collect();
    let mut flags = Vec::new();
    for n in doing.iter().flat_map(|r| &r.needs) {
        if !has_word(&own, n) {
            push_word(&mut flags, n);
        }
    }
    src.flags.iter().for_each(|f| push_word(&mut flags, f));
    if let Some(flag) = giver_lock(&doing) {
        push_word(&mut flags, flag);
    }
    let open = rows.iter().any(|r| r.max_level == 0);
    let top = rows.iter().map(|r| r.max_level).max().unwrap_or(0);
    QuestCard {
        id: main.id.to_string(),
        title: card_title(main, item.as_deref()),
        kind: main.kind,
        item,
        items,
        kinds,
        givers,
        creatures: src.creatures,
        places: src.places,
        levels: (
            rows.iter().map(|r| r.min_level).min().unwrap_or(0),
            if open { 0 } else { top },
        ),
        easiest: src.easiest,
        entry: src.entry.unwrap_or(0),
        xp: rows.iter().map(|r| r.reward.xp).max().unwrap_or(0),
        gives,
        repeat: main.repeat,
        flags,
        does,
        unread,
    }
}

/// One card per quest id, in the data's order, built on first use.
pub fn quest_cards() -> &'static [QuestCard] {
    static CARDS: OnceLock<Vec<QuestCard>> = OnceLock::new();
    CARDS.get_or_init(|| {
        crate::quests::all()
            .chunk_by(|a, b| a.id == b.id)
            .map(card_of)
            .collect()
    })
}

#[cfg(test)]
mod tests;
