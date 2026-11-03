//! Quests from one character's side: its log (kills counted, quests done, waits running), kept
//! between sessions because the server never sends the registry, and the board of each quest's
//! state ([`quest_board`]), worked out every [`QUEST_BOARD_EVERY`]. It sends nothing and decides
//! nothing; docs/quests-design.md's Seams section says which builder fills which part.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use ac_loot::quest_wants::QuestWant;
use ac_world::quests::{Kind, Repeat, Row, Want, Yield};
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::autoplay::{LootAction, Quests};
use crate::Client;

/// How often the board is worked out again (design §2.3, guess).
pub const QUEST_BOARD_EVERY: Duration = Duration::from_secs(2);
/// A leg this short joins a run without its XP paying for it (seconds, design §5.3, guess).
pub const SHORT_LEG: f32 = 60.0;
/// Hand-ins of a quest repeated any time kept at once: each takes a slot, and a giver is met
/// only on a run that passes (guess).
pub const ANY_HAND_INS: u32 = 5;
/// A giver's reward is waited for this long after the use or give (design §4.3, guess).
pub const XP_WATCH: Duration = Duration::from_secs(10);

/// One quest done: when last (Unix seconds) and how many times.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Done {
    pub at: u64,
    pub times: u32,
}

/// What this character has done of its quests, as far as the client saw it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct QuestLog {
    /// Kills so far, by the counter of a kill task taken up.
    pub kills: BTreeMap<String, u32>,
    /// Quests done, by id.
    pub done: BTreeMap<String, Done>,
    /// Unix seconds a quest may be done again, by its timer.
    pub waits: BTreeMap<String, u64>,
}

impl QuestLog {
    /// A kill task taken up: its counter starts at nothing (ACE's SetQuestCompletions 0).
    pub fn note_taken(&mut self, counter: &str) {
        self.kills.entry(counter.to_string()).or_insert(0);
    }

    /// The server's count for `counter` ("You have killed 7 ...").
    pub fn note_kills(&mut self, counter: &str, done: u32) {
        self.kills.insert(counter.to_string(), done);
    }

    /// `row` was completed at `now`: its wait starts and a kill counter is forgotten (ACE erased it).
    pub fn note_done(&mut self, row: &Row, now: u64) {
        let d = self.done.entry(row.id.clone()).or_default();
        d.at = now;
        d.times += 1;
        if let (Repeat::After(s), false) = (row.repeat, row.timer.is_empty()) {
            self.waits.insert(row.timer.clone(), now + u64::from(s));
        }
        if let Want::Kill(counter) = &row.want {
            self.kills.remove(counter);
        }
    }

    /// The giver said the quest under `timer` may be done again at `until` (Unix seconds).
    pub fn note_wait(&mut self, timer: &str, until: u64) {
        self.waits.insert(timer.to_string(), until);
    }

    /// When a quest kept under `timer` may be done again, if that is after `now`.
    pub fn wait_until(&self, timer: &str, now: u64) -> Option<u64> {
        self.waits.get(timer).copied().filter(|&t| t > now)
    }
}

/// Why a quest can't be done yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Blocked {
    /// No giver's band holds the character's level.
    Level { min: u32, max: u32 },
    /// The easiest source creature is this level, too far above the character's.
    TooHard { level: u32 },
    /// Every giver stands behind a portal asking this flag.
    Locked { flag: String },
    /// Another quest, not a start talk, stamps what this one needs.
    NeedsFirst { title: String },
}

/// Where one quest stands for one character.
#[derive(Clone, Debug, PartialEq)]
pub enum QuestState {
    /// Switched off, or quests off.
    Off,
    /// Hand in now: items carried, kills done, or a talk open.
    Ready,
    /// `more` to go of `of` (an item or a creature, by name).
    Needs {
        more: u32,
        of: String,
    },
    /// Not taken up yet: talk to `from` first.
    NotTaken {
        from: String,
    },
    /// A loot rule above the quest rule does this to the item.
    WontComplete {
        rule: String,
        does: LootAction,
    },
    CantYet(Blocked),
    /// Done; may be done again at this Unix second.
    Waiting {
        until: u64,
    },
    /// A once-quest done.
    Done,
}

/// One line of the board: a quest and its state for this character.
#[derive(Clone, Debug, PartialEq)]
pub struct QuestLine {
    pub id: String,
    pub title: String,
    pub kind: Kind,
    pub state: QuestState,
    /// The reward at the character's level.
    pub xp: u64,
    /// Giver placements that complete it.
    pub givers: usize,
    /// The nearest giver's name, by straight distance.
    pub giver: String,
    pub on: bool,
}

/// What the board is worked out from, so [`quest_board`] runs with no `Client`.
#[derive(Clone, Debug, Default)]
pub struct QuestFacts {
    pub level: u32,
    /// XP from the character's level to the next (`XpTable::xp_to_next_level`).
    pub to_next: u64,
    /// Quest flags held (`Travel::quests_done`).
    pub held: Vec<String>,
    pub log: QuestLog,
    /// Items carried, by weenie class.
    pub carried: BTreeMap<u32, u32>,
    /// `Growth::level_margin`.
    pub margin: u32,
    pub config: Quests,
    /// Unix seconds.
    pub now: u64,
    /// Where the character stands, for the nearest giver.
    pub me: Option<Vec3>,
    pub profile: Option<std::sync::Arc<crate::profile::Profile>>,
    pub wielder: crate::weapons::Wielder,
    pub name: String,
}

/// The state of every quest in `rows`, one line per id, in the data's order.
pub fn quest_board(rows: &[Row], facts: &QuestFacts) -> Vec<QuestLine> {
    board_of(rows, facts).0
}

/// The quests in `rows` no giver's band or easiest source suits at the character's level,
/// whether switched on or not: what the list's "for my level" leaves out.
pub fn unsuited_quests(rows: &[Row], facts: &QuestFacts) -> BTreeSet<String> {
    board_of(rows, facts).1
}

/// [`quest_board`] and [`unsuited_quests`] in one pass over the rows.
fn board_of(rows: &[Row], facts: &QuestFacts) -> (Vec<QuestLine>, BTreeSet<String>) {
    let mut locks = Locks::default();
    let mut unsuited = BTreeSet::new();
    let mut board = Vec::new();
    for (id, doing) in by_quest(rows) {
        let (Some(first), Some(giver)) = (doing.first(), nearest(&doing, facts.me)) else {
            continue;
        };
        if beyond_level(&doing, facts).is_some() {
            unsuited.insert(id.to_string());
        }
        let on = facts.config.is_on(id);
        board.push(QuestLine {
            id: id.to_string(),
            title: first.title.clone(),
            kind: first.kind,
            state: if on {
                state_of(&doing, facts, &mut locks)
            } else {
                QuestState::Off
            },
            xp: doing
                .iter()
                .map(|r| r.reward.xp_at(facts.to_next))
                .max()
                .unwrap_or(0),
            givers: doing.len(),
            giver: giver.giver.clone(),
            on,
        });
    }
    (board, unsuited)
}

/// Each quest's rows that complete it (start rows left out), by id in the data's order.
fn by_quest(rows: &[Row]) -> Vec<(&str, Vec<&Row>)> {
    let mut ids: Vec<&str> = Vec::new();
    let mut by_id: BTreeMap<&str, Vec<&Row>> = BTreeMap::new();
    for r in rows {
        if !by_id.contains_key(r.id.as_str()) {
            ids.push(&r.id);
        }
        let doing = by_id.entry(&r.id).or_default();
        if r.kind != Kind::Start {
            doing.push(r);
        }
    }
    ids.into_iter()
        .map(|id| (id, by_id.remove(id).unwrap_or_default()))
        .collect()
}

/// The row whose giver stands nearest `me` by straight distance; the first with no position.
fn nearest<'r>(rows: &[&'r Row], me: Option<Vec3>) -> Option<&'r Row> {
    let near = |r: &&Row| me.map_or(0.0, |m| r.place.at.distance(m));
    rows.iter()
        .copied()
        .min_by(|a, b| near(a).total_cmp(&near(b)))
}

/// The flag each dungeon landblock's givers are locked behind, worked out once a board.
#[derive(Default)]
struct Locks(BTreeMap<u32, Option<&'static str>>);

impl Locks {
    /// Only a landblock locked with no flag held can be locked with some held, so the few of
    /// those, found once, are all `Row::locked_by` is asked about.
    fn of(&mut self, row: &Row, held: &[String]) -> Option<&'static str> {
        static MAYBE: OnceLock<BTreeSet<u32>> = OnceLock::new();
        let maybe = MAYBE.get_or_init(|| {
            let quests = ac_world::quests::all().iter();
            quests
                .filter(|r| r.locked_by(&[]).is_some())
                .map(|r| r.place.cell >> 16)
                .collect()
        });
        let block = row.place.cell >> 16;
        if !row.place.is_indoors() || !maybe.contains(&block) {
            return None;
        }
        *self.0.entry(block).or_insert_with(|| row.locked_by(held))
    }
}

/// The level of the easiest creature yielding `want`: 0 when a placed NPC gives it.
fn easiest(want: &Want) -> Option<u32> {
    static EASIEST: OnceLock<BTreeMap<Want, u32>> = OnceLock::new();
    let map = EASIEST.get_or_init(|| {
        let mut m: BTreeMap<Want, u32> = BTreeMap::new();
        for s in ac_world::quests::sources() {
            let level = if s.kind == Yield::Npc { 0 } else { s.level };
            let l = m.entry(s.want.clone()).or_insert(level);
            *l = (*l).min(level);
        }
        m
    });
    map.get(want).copied()
}

/// The rows of every quest stamping a flag, by the flag lowercased.
fn stampers(flag: &str) -> &'static [&'static Row] {
    static STAMPERS: OnceLock<BTreeMap<String, Vec<&'static Row>>> = OnceLock::new();
    let map = STAMPERS.get_or_init(|| {
        let mut m: BTreeMap<String, Vec<&'static Row>> = BTreeMap::new();
        for r in ac_world::quests::all() {
            for s in &r.stamps {
                m.entry(s.to_ascii_lowercase()).or_default().push(r);
            }
        }
        m
    });
    map.get(&flag.to_ascii_lowercase())
        .map_or(&[], Vec::as_slice)
}

/// Whether the character holds `flag`: a quest flag, a kill task's counter taken up, or a stamp
/// of a quest the log has done.
fn holds(facts: &QuestFacts, flag: &str) -> bool {
    facts.held.iter().any(|h| h.eq_ignore_ascii_case(flag))
        || facts.log.kills.contains_key(&flag.to_ascii_lowercase())
        || stampers(flag)
            .iter()
            .any(|r| r.kind != Kind::Start && facts.log.done.contains_key(&r.id))
}

/// The band `rows` cover between them: the lowest minimum, the highest maximum (0 for open).
fn band(rows: &[&Row]) -> Blocked {
    let min = rows.iter().map(|r| r.min_level).min().unwrap_or(0);
    let max = if rows.iter().any(|r| r.max_level == 0) {
        0
    } else {
        rows.iter().map(|r| r.max_level).max().unwrap_or(0)
    };
    Blocked::Level { min, max }
}

/// Why the character's level rules the quest out: no giver's band holds it, or the easiest
/// source of anything it asks is past `level + 2 * margin` (`Ground::suits`' bound on strays).
fn beyond_level(doing: &[&Row], facts: &QuestFacts) -> Option<Blocked> {
    if !doing.iter().any(|r| r.fits_level(facts.level)) {
        return Some(band(doing));
    }
    let level = doing.iter().filter_map(|r| easiest(&r.want)).min()?;
    (level > facts.level + 2 * facts.margin).then_some(Blocked::TooHard { level })
}

/// The giver to talk to first for `flag`, nearest first, or why the character can't yet.
fn see_first(flag: &str, facts: &QuestFacts, locks: &mut Locks, depth: u32) -> QuestState {
    let by = stampers(flag);
    let starts: Vec<&Row> = by
        .iter()
        .copied()
        .filter(|r| r.kind == Kind::Start)
        .collect();
    if starts.is_empty() {
        let title = by.first().map_or(flag, |r| r.title.as_str());
        return QuestState::CantYet(Blocked::NeedsFirst {
            title: title.to_string(),
        });
    }
    let fit: Vec<&Row> = starts
        .iter()
        .copied()
        .filter(|r| r.fits_level(facts.level))
        .collect();
    if fit.is_empty() {
        return QuestState::CantYet(band(&starts));
    }
    let mut open: Vec<&Row> = fit
        .iter()
        .copied()
        .filter(|r| locks.of(r, &facts.held).is_none())
        .collect();
    if open.is_empty() {
        return QuestState::CantYet(Blocked::Locked {
            flag: locks
                .of(fit[0], &facts.held)
                .unwrap_or_default()
                .to_string(),
        });
    }
    let near = |r: &&Row| facts.me.map_or(0.0, |m| r.place.at.distance(m));
    open.sort_by(|a, b| near(a).total_cmp(&near(b)));
    let unmet = |r: &Row| r.needs.iter().find(|n| !holds(facts, n)).cloned();
    if let Some(r) = open.iter().find(|r| unmet(r).is_none()) {
        return QuestState::NotTaken {
            from: r.giver.clone(),
        };
    }
    match unmet(open[0]) {
        Some(need) if depth < 3 => see_first(&need, facts, locks, depth + 1),
        _ => QuestState::CantYet(Blocked::NeedsFirst {
            title: open[0].title.clone(),
        }),
    }
}

/// One switched-on quest's state: done or waiting, then what rules it out, then what is left.
fn state_of(doing: &[&Row], facts: &QuestFacts, locks: &mut Locks) -> QuestState {
    let first = doing[0];
    if let Some(until) = facts.log.wait_until(&first.timer, facts.now) {
        return QuestState::Waiting { until };
    }
    // A once-quest's own flag held is done too: stamped before the log was kept.
    let flagged = !first.timer.is_empty() && holds(facts, &first.timer);
    if first.repeat == Repeat::Once && (facts.log.done.contains_key(&first.id) || flagged) {
        return QuestState::Done;
    }
    if let Some(why) = beyond_level(doing, facts) {
        return QuestState::CantYet(why);
    }
    let fit: Vec<&Row> = doing
        .iter()
        .copied()
        .filter(|r| r.fits_level(facts.level))
        .collect();
    let open: Vec<&Row> = fit
        .iter()
        .copied()
        .filter(|r| locks.of(r, &facts.held).is_none())
        .collect();
    if open.is_empty() {
        let flag = locks.of(fit[0], &facts.held).unwrap_or_default();
        return QuestState::CantYet(Blocked::Locked {
            flag: flag.to_string(),
        });
    }
    let taken: Vec<&Row> = open
        .iter()
        .copied()
        .filter(|r| r.needs.iter().all(|n| holds(facts, n)))
        .collect();
    let Some(&row) = taken.first() else {
        let need = open[0].needs.iter().find(|n| !holds(facts, n));
        return see_first(need.map_or("", String::as_str), facts, locks, 0);
    };
    match &row.want {
        Want::Kill(counter) => match facts.log.kills.get(counter) {
            None => see_first(counter, facts, locks, 0),
            Some(&n) if n < row.count => QuestState::Needs {
                more: row.count - n,
                of: ac_world::quests::sources_of(&row.want)
                    .next()
                    .map_or_else(|| counter.clone(), |s| s.name.clone()),
            },
            Some(_) => QuestState::Ready,
        },
        Want::Item(_) => item_state(&taken, facts),
        Want::Nothing => QuestState::Ready,
    }
}

/// Each item the rows take, once, in the rows' order: any one handed in completes the quest.
fn items_of<'r>(rows: impl IntoIterator<Item = &'r Row>) -> Vec<u32> {
    let mut wcids = Vec::new();
    for r in rows {
        if let Want::Item(w) = r.want {
            if !wcids.contains(&w) {
                wcids.push(w);
            }
        }
    }
    wcids
}

/// An item quest's state: ready with any one item carried, else the item nearest done, else the
/// rule that blocks the first.
fn item_state(taken: &[&Row], facts: &QuestFacts) -> QuestState {
    let mut nearest: Option<(u32, String)> = None;
    let mut blocked: Option<(String, LootAction)> = None;
    for wcid in items_of(taken.iter().copied()) {
        let rows = || taken.iter().filter(move |r| r.want == Want::Item(wcid));
        let need = rows().map(|r| r.count).min().unwrap_or(1);
        let held = facts.carried.get(&wcid).copied().unwrap_or(0);
        let item = rows().find_map(|r| r.item.as_ref());
        let block = item.and_then(|i| {
            crate::autoplay::quest_item_blocked(
                facts.profile.as_deref(),
                i,
                &facts.wielder,
                &facts.name,
                held,
            )
        });
        match block {
            Some(b) => {
                blocked.get_or_insert(b);
            }
            None if held >= need => return QuestState::Ready,
            None if nearest.as_ref().is_some_and(|(m, _)| *m <= need - held) => {}
            None => {
                nearest = Some((
                    need - held,
                    item.map_or_else(String::new, |i| i.name.clone()),
                ))
            }
        }
    }
    match (nearest, blocked) {
        (Some((more, of)), _) => QuestState::Needs { more, of },
        (None, Some((rule, does))) => QuestState::WontComplete { rule, does },
        (None, None) => QuestState::Needs {
            more: 1,
            of: String::new(),
        },
    }
}

/// What the board's item quests want carried, for the loot rule: a quest ready, needing more,
/// or blocked by a rule (which may be moved) wants each of its items; [`ANY_HAND_INS`] at most.
pub fn quest_wants(board: &[QuestLine]) -> Vec<QuestWant> {
    let mut wants: BTreeMap<u32, u32> = BTreeMap::new();
    for line in board {
        let wanting = matches!(
            line.state,
            QuestState::Ready | QuestState::Needs { .. } | QuestState::WontComplete { .. }
        );
        if !wanting || line.kind != Kind::Item {
            continue;
        }
        let rows: Vec<&Row> = ac_world::quests::rows_of(&line.id)
            .filter(|r| r.kind == Kind::Item)
            .collect();
        for wcid in items_of(rows.iter().copied()) {
            let of = rows.iter().filter(|r| r.want == Want::Item(wcid));
            let count = of.clone().map(|r| r.count.max(1)).max().unwrap_or(1);
            let any = of.clone().any(|r| r.repeat == Repeat::Any);
            *wants.entry(wcid).or_insert(0) += if any { count * ANY_HAND_INS } else { count };
        }
    }
    wants
        .into_iter()
        .map(|(wcid, need)| QuestWant {
            wcid,
            need: Some(need),
        })
        .collect()
}

/// The kill task a count is for, as the line names only the creature: the counters whose
/// creatures the plural begins with (less the name's last letter, for "Harpies"), the longest
/// name only, then those asking the count, then those taken up; none named, the only one taken.
fn counter_for(k: &crate::refusals::KillCount, log: &QuestLog) -> Option<String> {
    let mut named: Vec<(&str, usize)> = Vec::new();
    for s in ac_world::quests::sources() {
        let (Yield::Kill, Want::Kill(counter)) = (s.kind, &s.want) else {
            continue;
        };
        let mut stem = s.name.chars();
        stem.next_back();
        let stem = stem.as_str();
        if !stem.is_empty() && k.plural.starts_with(stem) {
            named.push((counter, stem.len()));
        }
    }
    let longest = named.iter().map(|&(_, n)| n).max().unwrap_or(0);
    let mut counters: Vec<&str> = named
        .into_iter()
        .filter(|&(_, n)| n == longest)
        .map(|(c, _)| c)
        .collect();
    counters.sort_unstable();
    counters.dedup();
    if counters.is_empty() && log.kills.len() == 1 {
        return log.kills.keys().next().cloned();
    }
    let asks = k.of.unwrap_or(k.done);
    let asking =
        |c: &&str| ac_world::quests::rows_of(c).any(|r| r.kind == Kind::Kill && r.count == asks);
    if counters.len() > 1 && counters.iter().any(asking) {
        counters.retain(asking);
    }
    if counters.len() > 1 {
        counters.retain(|c| log.kills.contains_key(*c));
    }
    match counters.as_slice() {
        [one] => Some(one.to_string()),
        _ => None,
    }
}

/// Whether a call whose detour costs `detour` seconds pays for itself at `xp_per_hour` (design
/// §5.3's second rule). Unmeasured, it never does: only the short-leg rule applies.
pub fn worth_detour(xp: u64, detour: f32, xp_per_hour: Option<f32>) -> bool {
    xp_per_hour.is_some_and(|rate| xp as f32 >= detour.max(0.0) * rate / 3600.0)
}

/// A giver the client asked, waiting for the answer (design §4.3).
#[derive(Clone, Debug)]
pub struct Asked {
    pub row: &'static Row,
    pub at: Instant,
    /// Total XP when asked: a rise is the reward.
    pub xp_before: i64,
    /// The wanted item carried when asked, 0 for a talk: fewer after is a give taken.
    pub carried: u32,
}

impl Client {
    /// The quest ids autoplay considers: quests on, and each not switched off.
    pub fn quests_on(&self) -> Vec<&'static str> {
        let cfg = &self.autoplay.config.quests;
        let mut ids: Vec<&'static str> = Vec::new();
        for r in ac_world::quests::all() {
            if r.kind != Kind::Start && cfg.is_on(&r.id) && ids.last() != Some(&r.id.as_str()) {
                ids.push(&r.id);
            }
        }
        ids
    }

    /// The board's inputs, as this character stands now.
    pub fn quest_facts(&self) -> QuestFacts {
        let stats = &self.world.stats;
        let level = stats.level.max(0) as u32;
        let mut carried = BTreeMap::new();
        for o in self.world.inventory() {
            *carried.entry(o.weenie_class_id).or_insert(0) += o.stack_size.max(1);
        }
        QuestFacts {
            level,
            to_next: self
                .assets
                .xp_table()
                .ok()
                .and_then(|t| t.xp_to_next_level(level))
                .unwrap_or(0),
            held: self.travel.quests_done.clone(),
            log: self.quest_log.clone(),
            carried,
            margin: self.autoplay.config.growth.level_margin,
            config: self.autoplay.config.quests.clone(),
            now: crate::holdings::unix_now(),
            me: self.my_position(),
            profile: self.loot_profile(),
            wielder: self.wielder(),
            name: stats.name.clone(),
        }
    }

    /// Work the board out again when [`QUEST_BOARD_EVERY`] has passed, or at once when the
    /// quests switched on, the level or the log changed, into `quest_board`.
    pub fn refresh_quests(&mut self, now: Instant) {
        let cfg = &self.autoplay.config.quests;
        let level = self.world.stats.level.max(0) as u32;
        let changed = self
            .quest_board_for
            .as_ref()
            .is_none_or(|(q, l, log)| q != cfg || *l != level || *log != self.quest_log);
        let due = self
            .quest_board_at
            .is_none_or(|at| now.duration_since(at) >= QUEST_BOARD_EVERY);
        let settled = !self.quest_board.is_empty() && !self.quest_board.iter().any(|l| l.on);
        if !changed && (!due || (!cfg.on && settled)) {
            return;
        }
        self.quest_board_for = Some((cfg.clone(), level, self.quest_log.clone()));
        self.quest_board_at = Some(now);
        (self.quest_board, self.quests_unsuited) =
            board_of(ac_world::quests::all(), &self.quest_facts());
    }

    /// `row`'s giver was asked: its answer is watched for, and one asked before is answered now.
    pub(crate) fn ask_giver(&mut self, row: &'static Row, now: Instant) {
        if let Some(old) = self.quest_asked.take() {
            let rose = self.world.stats.total_xp > old.xp_before;
            if rose || !std::ptr::eq(old.row, row) {
                self.quest_answered(&old, rose, now);
            }
        }
        self.quest_asked = Some(Asked {
            row,
            at: now,
            xp_before: self.world.stats.total_xp,
            carried: match row.want {
                Want::Item(wcid) => self.carried_count(wcid),
                _ => 0,
            },
        });
    }

    /// The answer to the giver asked: XP came, or [`XP_WATCH`] passed without it.
    pub(crate) fn watch_quest_answer(&mut self, now: Instant) {
        let Some(asked) = &self.quest_asked else {
            return;
        };
        let rose = self.world.stats.total_xp > asked.xp_before;
        if !rose && now.duration_since(asked.at) < XP_WATCH {
            return;
        }
        if let Some(asked) = self.quest_asked.take() {
            self.quest_answered(&asked, rose, now);
        }
    }

    /// Paid, or unpaid with the item taken or the talk heard (the giver's own "done" reply, the
    /// safe side): done, its wait begun and its flags held. Else the giver is let be a while.
    fn quest_answered(&mut self, asked: &Asked, rose: bool, now: Instant) {
        let row = asked.row;
        let taken = match row.want {
            Want::Item(wcid) => self.carried_count(wcid) < asked.carried,
            _ => true,
        };
        if !rose && (!taken || row.repeat == Repeat::Any) {
            let key = (row.giver_wcid, row.place.cell);
            self.autoplay.growth.givers_refused.mark(key, now);
            let why = format!("{} gave nothing for {}", row.giver, row.title);
            self.autoplay.note(why, now);
            return;
        }
        tracing::info!("quest: {} done at {} (paid: {rose})", row.id, row.giver);
        self.quest_log.note_done(row, crate::holdings::unix_now());
        self.save_quest_log();
        self.hold_quest_flags(&row.stamps);
    }

    /// The board's wants less what is carried: what a mate may take for this character.
    pub fn quest_wants_left(&self) -> Vec<QuestWant> {
        let left = |w: &QuestWant| w.need.map(|n| n.saturating_sub(self.carried_count(w.wcid)));
        self.quest_wants
            .iter()
            .map(|w| QuestWant {
                wcid: w.wcid,
                need: left(w),
            })
            .filter(|w| w.need != Some(0))
            .collect()
    }

    /// How many of the items of weenie class `wcid` the packs hold, worn ones left out.
    pub(crate) fn carried_count(&self, wcid: u32) -> u32 {
        self.world
            .inventory()
            .filter(|o| o.weenie_class_id == wcid && o.wielder.is_none())
            .map(|o| o.stack_size.max(1))
            .sum()
    }

    /// Go and see `row`'s giver: a visit, journey and all. False when there is no way.
    pub fn visit_giver(&mut self, row: &Row) -> bool {
        let l = ac_world::landmarks::Landmark {
            kind: ac_world::landmarks::Kind::Npc,
            name: row.giver.clone(),
            cell: row.place.cell,
            at: row.place.at,
        };
        let ok = self.visit_landmark(&l);
        tracing::info!("quest: to {} for {} ({ok})", row.giver, row.id);
        ok
    }

    /// The character's XP per hour lately; `None` until measured. Seam: the hunt builder fills it.
    pub fn xp_per_hour(&self) -> Option<f32> {
        None
    }

    /// A kill task's count heard (the server sends it only for a task held): into the log
    /// under its counter, and saved. It only observes.
    pub(crate) fn hear_kill_count(&mut self, text: &str) {
        let Some(k) = crate::refusals::kill_count(text) else {
            return;
        };
        let Some(counter) = counter_for(&k, &self.quest_log) else {
            tracing::info!("quest: no one kill task counts {}", k.plural);
            return;
        };
        if self.quest_log.kills.get(&counter) != Some(&k.done) {
            tracing::info!("quest: {counter} at {} kills", k.done);
            self.quest_log.note_kills(&counter, k.done);
            self.save_quest_log();
        }
    }

    /// Read back the quest log of earlier sessions, at login.
    pub(crate) fn load_quest_log(&mut self) {
        let Some(path) = self.quest_file(".log.json") else {
            return;
        };
        match ac_store::read_json::<QuestLog>(&path) {
            Ok(Some(log)) => self.quest_log = log,
            Ok(None) => {}
            Err(e) => tracing::warn!("quest: reading {}: {e}", path.display()),
        }
    }

    /// Write the quest log out, after a change to it.
    pub fn save_quest_log(&self) {
        let Some(path) = self.quest_file(".log.json") else {
            return;
        };
        let visible = ac_store::Visibility::Normal;
        if let Err(e) = ac_store::write_json_atomic(&path, &self.quest_log, visible) {
            tracing::warn!("quest: writing {}: {e}", path.display());
        }
    }
}

#[cfg(test)]
mod tests;
