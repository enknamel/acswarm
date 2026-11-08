//! Quests the player asked for, worked before hunting for XP: each pursued quest's step for this
//! character, worked out with the board; the town run calls on givers, `grow_hunt` hunts sources.
//! A flag a way on asks is earned first: from an unlock NPC, or by the quest stamping it, worked
//! ahead of the one that needs it (`with_chain`). An item is fetched before it is hunted (`fetch`).
//! What a chain needs is kept and bought only through the profile's Quests section (`section`).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use ac_world::quest_chain::{ChainLink, FlagBy, SourceGround};
use ac_world::quests::{Row, Source, Want, Yield};
use ac_world::unlocks::Unlock;
use ac_world::Place;

use super::hunt::{GROUND_REACH, SKIP_GROUND_FOR, UNLOCK_RETRY};
use super::town_run::calls::Asking;
use crate::autoplay::{Doing, Pursuit};
use crate::quest_log::{Blocked, QuestFacts, QuestLine, QuestState, KEY_DEPTH};
use crate::Client;

mod fetch;
mod section;
pub(crate) use fetch::Fetching;

/// A pursued quest that could not go on is passed over this long before it is tried again (guess).
pub const PURSUIT_RETRY: Duration = Duration::from_secs(10 * 60);
/// Flags looked through for a quest to work first, a quest needing one needing another (guess).
const CHAIN_DEPTH: u32 = 3;
/// Most grounds a hunt tries a journey to, best first: each is a plan over every portal.
const HUNT_GROUNDS: usize = 6;

/// The step a pursued quest is at for this character.
#[derive(Clone, Debug, PartialEq)]
pub enum PursuitStep {
    /// Talk to `row`'s giver at `at`, who starts it.
    TakeUp { row: &'static Row, at: Place },
    /// Earn `flag` from an unlock NPC first: a way on asks it.
    EarnFlag { flag: String, from: &'static Unlock },
    /// Hunt `creature` in `landblock` (`place` in words), `more` to go.
    Hunt {
        landblock: u32,
        creature: String,
        place: String,
        more: u32,
    },
    /// Buy `name` (weenie class `wcid`) at `vendor`'s counter, `more` to go: a town run buys it by
    /// its line in the Quests section, as any line of the buy list.
    Buy {
        wcid: u32,
        name: String,
        vendor: String,
        more: u32,
    },
    /// Walk to `at` and pick up `name` (weenie class `wcid`) lying there.
    PickUp { wcid: u32, name: String, at: Place },
    /// Walk to `chest` at `at`, unlock it with the carried `key` when it is locked, and empty it.
    OpenChest {
        chest: &'static Source,
        at: Place,
        key: Option<u32>,
    },
    /// Walk to `source`'s object at `at` and use the carried `with` (weenie class) on it.
    UseOn {
        source: &'static Source,
        at: Place,
        with: u32,
    },
    /// Use the carried `with` on the carried `on` (weenie classes), where the character stands.
    Combine {
        source: &'static Source,
        with: u32,
        on: u32,
    },
    /// Hand in to `row`'s giver at `at`.
    HandIn { row: &'static Row, at: Place },
    /// Done; may be done again at this Unix second.
    Waiting { until: u64 },
    /// Done since asked for and not to be done again.
    Finished,
    /// Cannot go on, in words ("nobody known gives the eaterpit flag").
    Stuck { why: String },
}

impl PursuitStep {
    /// Whether autoplay can act on it now.
    pub fn can_progress(&self) -> bool {
        matches!(
            self,
            PursuitStep::TakeUp { .. }
                | PursuitStep::EarnFlag { .. }
                | PursuitStep::Hunt { .. }
                | PursuitStep::Buy { .. }
                | PursuitStep::PickUp { .. }
                | PursuitStep::OpenChest { .. }
                | PursuitStep::UseOn { .. }
                | PursuitStep::Combine { .. }
                | PursuitStep::HandIn { .. }
        )
    }

    /// The step in words, `now` in Unix seconds for a wait ("hunt Insatiable Eater in ...").
    pub fn words(&self, now: u64) -> String {
        match self {
            PursuitStep::TakeUp { row, .. } => format!("take it up from {}", row.giver),
            PursuitStep::EarnFlag { flag, from } => {
                format!("earn the {flag} flag from {}", from.npc)
            }
            PursuitStep::Hunt {
                creature,
                place,
                more,
                ..
            } => format!("hunt {creature} in {place} ({more} to go)"),
            PursuitStep::Buy {
                name, vendor, more, ..
            } => format!("buy {name} from {vendor} ({more} to go)"),
            PursuitStep::PickUp { name, at, .. } => format!(
                "pick up {name} in {}",
                ac_world::quest_chain::place_name(at.cell)
            ),
            PursuitStep::OpenChest { chest, at, key } => {
                let with = key
                    .and_then(|k| chest.keys.iter().find(|c| c.0 == k))
                    .map_or(String::new(), |k| format!(" with {}", k.1));
                let place = ac_world::quest_chain::place_name(at.cell);
                format!("open {}{with} in {place}", chest.name)
            }
            PursuitStep::UseOn { source, at, with } => {
                let used = source.keys.iter().find(|k| k.0 == *with);
                let used = used.map_or_else(|| format!("item {with}"), |k| k.1.clone());
                let place = ac_world::quest_chain::place_name(at.cell);
                format!("use {used} on {} in {place}", source.name)
            }
            PursuitStep::Combine { source, with, on } => {
                let name = |w: &u32| source.keys.iter().find(|k| k.0 == *w).map(|k| k.1.clone());
                let (used, onto) = (name(with), name(on));
                format!(
                    "use {} on {}",
                    used.unwrap_or_else(|| format!("item {with}")),
                    onto.unwrap_or_else(|| format!("item {on}"))
                )
            }
            PursuitStep::HandIn { row, .. } => format!("hand it in to {}", row.giver),
            PursuitStep::Waiting { until } => {
                let left = until.saturating_sub(now);
                format!("waits {}h {}m", left / 3600, left % 3600 / 60)
            }
            PursuitStep::Finished => "finished".to_string(),
            PursuitStep::Stuck { why } => format!("passed over: {why}"),
        }
    }
}

/// One pursued quest as the Quests window shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct PursuitLine {
    pub id: String,
    pub title: String,
    pub step: PursuitStep,
    /// The one autoplay works now: the first that can progress.
    pub working: bool,
    /// The pursued quest this one is worked first for (it stamps a flag that one needs).
    pub via: Option<String>,
}

impl PursuitLine {
    /// The line in words, `now` in Unix seconds: "Engorged Eater Jaw: working: hunt ...".
    pub fn words(&self, now: u64) -> String {
        let working = if self.working { "working: " } else { "" };
        let via = self
            .via
            .as_ref()
            .map_or(String::new(), |v| format!(" (first, for {v})"));
        format!("{}: {working}{}{via}", self.title, self.step.words(now))
    }
}

/// What the chain data answers: `ac_world::quest_chain`'s, or a test's own.
pub(crate) trait Chains {
    /// Quest `id`'s chain, in order (`quest_chain::chain_of`).
    fn chain(&self, id: &str) -> Vec<ChainLink>;
    /// Where `want` comes from, best first (`quest_chain::source_grounds`).
    fn grounds(&self, want: &Want) -> Vec<SourceGround>;
    /// Who stamps `flag` (`quest_chain::flag_givers`).
    fn givers(&self, flag: &str) -> Vec<FlagBy>;
    /// Every source of `want` (`quests::sources_of`), the fetched ones read here.
    fn sources(&self, want: &Want) -> Vec<&'static Source>;
}

/// The bundled data's chains.
pub(crate) struct ChainData;

impl Chains for ChainData {
    fn chain(&self, id: &str) -> Vec<ChainLink> {
        ac_world::quest_chain::chain_of(id)
    }

    fn grounds(&self, want: &Want) -> Vec<SourceGround> {
        ac_world::quest_chain::source_grounds(want)
    }

    fn givers(&self, flag: &str) -> Vec<FlagBy> {
        ac_world::quest_chain::flag_givers(flag)
    }

    fn sources(&self, want: &Want) -> Vec<&'static Source> {
        ac_world::quests::sources_of(want).collect()
    }
}

/// One quest on the worked pursuit list: pursued, or worked first for the one it is `via`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Chained {
    pub(crate) pursuit: Pursuit,
    pub(crate) via: Option<String>,
}

/// The pursuit list with the quests to work first put in front of each: one stamping a flag its
/// chain asks and `holds` says is not held, no unlock NPC giving it, [`CHAIN_DEPTH`] deep.
pub(crate) fn with_chain(
    pursue: &[Pursuit],
    holds: &dyn Fn(&str) -> bool,
    chains: &dyn Chains,
) -> Vec<Chained> {
    let mut out: Vec<Chained> = Vec::new();
    for p in pursue {
        let mut firsts = Vec::new();
        firsts_of(&p.id, holds, chains, 0, &mut firsts);
        for f in firsts {
            if out.iter().any(|c| c.pursuit.id == f) {
                continue;
            }
            let pursuit = pursue.iter().find(|q| q.id == f).cloned();
            out.push(Chained {
                pursuit: pursuit.unwrap_or(Pursuit {
                    id: f.to_string(),
                    again: true,
                    since: 0,
                }),
                via: Some(p.id.clone()),
            });
        }
        match out.iter_mut().find(|c| c.pursuit.id == p.id) {
            Some(c) => c.via = None,
            None => out.push(Chained {
                pursuit: p.clone(),
                via: None,
            }),
        }
    }
    out
}

/// The quests stamping a flag `id`'s chain asks, deepest first, into `firsts`.
fn firsts_of(
    id: &str,
    holds: &dyn Fn(&str) -> bool,
    chains: &dyn Chains,
    depth: u32,
    firsts: &mut Vec<&'static str>,
) {
    if depth >= CHAIN_DEPTH {
        return;
    }
    for link in chains.chain(id) {
        let ChainLink::Flag { flag, by, .. } = link else {
            continue;
        };
        // An unlock NPC's flag is earned by a visit (`EarnFlag`), not a quest worked first.
        if holds(&flag) || by.iter().any(|b| matches!(b, FlagBy::Unlock(_))) {
            continue;
        }
        let stamper = by.iter().find_map(|b| match b {
            FlagBy::Quest(row) if row.id != id => Some(row.id),
            _ => None,
        });
        if let Some(first) = stamper.filter(|f| !firsts.contains(f)) {
            firsts_of(first, holds, chains, depth + 1, firsts);
            if !firsts.contains(&first) {
                firsts.push(first);
            }
        }
    }
}

/// What working the pursuit list out came to: each line, and the working hunt's facts.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Pursued {
    pub(crate) lines: Vec<PursuitLine>,
    /// The working hunt's grounds a journey may end at, best first: landblock and place.
    pub(crate) grounds: Vec<(u32, Place)>,
    /// The working hunt's creatures, by weenie class: fought before their namesakes.
    pub(crate) sought: Vec<u32>,
}

/// What each line's step is worked out from, besides the board.
pub(crate) struct Ask<'a> {
    pub(crate) facts: &'a QuestFacts,
    pub(crate) asking: &'a Asking<'a>,
    /// This unlock NPC was set off to lately for its flag, and is not being walked to now.
    pub(crate) tried: &'a dyn Fn(&Unlock) -> bool,
    /// Quests passed over lately, by id, and why.
    pub(crate) held: &'a BTreeMap<String, String>,
    /// The leader a follower goes with: it works nothing of its own.
    pub(crate) led: Option<&'a str>,
    pub(crate) chains: &'a dyn Chains,
    /// A source a step at it was refused lately ([`Client::drop_source`]): left out.
    pub(crate) dropped: &'a dyn Fn(&Source) -> bool,
    /// A chest this character unlocked within `fetch::UNLOCKED_FOR`: opened with no key.
    pub(crate) unlocked: &'a dyn Fn(&Source) -> bool,
}

/// Each line's step, worked out with the `board` the chained list's facts gave; the first that
/// can go on is the one worked. One worked first waits while the quest it is for is not due.
pub(crate) fn pursuit_of(board: &[QuestLine], chained: &[Chained], ask: &Ask) -> Pursued {
    let mut lines: Vec<(PursuitLine, Option<Hunting>)> = Vec::new();
    for c in chained {
        let id = c.pursuit.id.as_str();
        let line = board.iter().find(|l| l.id == id);
        let title = line.map_or_else(
            || ac_world::quests::rows_of(id).next().map_or(id, |r| r.title),
            |l| l.title.as_str(),
        );
        let done_since = ask
            .facts
            .log
            .done
            .get(id)
            .is_some_and(|d| d.at >= c.pursuit.since);
        let mut hunt = None;
        let step = if let Some(leader) = ask.led {
            stuck(format!("following {leader}; waiting"))
        } else if let Some(why) = ask.held.get(id) {
            stuck(why.clone())
        } else if !c.pursuit.again && done_since {
            PursuitStep::Finished
        } else if let Some(line) = line {
            let (step, h) = ask.step_of(line);
            hunt = h;
            step
        } else {
            stuck("no way to do it is known".to_string())
        };
        let line = PursuitLine {
            id: id.to_string(),
            title: title.trim().to_string(),
            step,
            working: false,
            via: c.via.clone(),
        };
        lines.push((line, hunt));
    }
    // The quest a line is worked first for comes after it: its step is known only now.
    for i in 0..lines.len() {
        let Some(via) = lines[i].0.via.clone() else {
            continue;
        };
        let after = lines
            .iter()
            .find(|(l, _)| l.id == via)
            .map(|(l, _)| &l.step);
        let rests = match after {
            Some(PursuitStep::Waiting { until }) => Some(PursuitStep::Waiting { until: *until }),
            Some(PursuitStep::Finished) => Some(PursuitStep::Finished),
            _ => None,
        };
        if let Some(step) = rests {
            lines[i] = (
                PursuitLine {
                    step,
                    ..lines[i].0.clone()
                },
                None,
            );
        }
    }
    let mut out = Pursued::default();
    for (mut line, hunt) in lines {
        line.working = line.step.can_progress() && !out.lines.iter().any(|l| l.working);
        if let (true, Some((grounds, sought))) = (line.working, hunt) {
            out.grounds = grounds;
            out.sought = sought;
        }
        out.lines.push(line);
    }
    out
}

fn stuck(why: String) -> PursuitStep {
    PursuitStep::Stuck { why }
}

/// A hunt's grounds and the creatures sought there.
type Hunting = (Vec<(u32, Place)>, Vec<u32>);

impl Ask<'_> {
    /// One board line's step, and its grounds when it is a hunt.
    fn step_of(&self, line: &QuestLine) -> (PursuitStep, Option<Hunting>) {
        let level = self.facts.level;
        let step = match &line.state {
            QuestState::Off => stuck("switched off".to_string()),
            QuestState::Done => PursuitStep::Finished,
            QuestState::Waiting { until } => PursuitStep::Waiting { until: *until },
            QuestState::Ready => match self.asking.hand_in_at(line) {
                Some((_, row, at)) => PursuitStep::HandIn { row, at },
                None => stuck(format!("{} cannot be asked now", line.giver)),
            },
            QuestState::NotTaken { from, flag } => match self.asking.take_up_at(from, flag) {
                Some((_, row, at)) => PursuitStep::TakeUp { row, at },
                None => stuck(format!("{from} cannot be asked now")),
            },
            QuestState::Needs { more, of } => return self.hunt_step(line, *more, of),
            QuestState::WontComplete { rule, does } => stuck(format!(
                "the loot rule {rule} would {} the item: add the quest rule",
                format!("{does:?}").to_lowercase()
            )),
            QuestState::CantYet(Blocked::Level { min, max }) => {
                let band = match max {
                    0 => format!("from level {min}"),
                    max => format!("levels {min}-{max}"),
                };
                stuck(format!("its givers take {band}; this character is {level}"))
            }
            QuestState::CantYet(Blocked::Locked { flag }) => self.flag_step(flag),
            QuestState::CantYet(Blocked::NeedsFirst { title }) => {
                stuck(format!("{} comes first", title.trim()))
            }
        };
        (step, None)
    }

    /// Come by what `line` still needs, `more` of it (`of` in words).
    fn hunt_step(&self, line: &QuestLine, more: u32, of: &str) -> (PursuitStep, Option<Hunting>) {
        let mut wants: Vec<Want> = Vec::new();
        for r in crate::quest_log::worked_rows(line).filter(|r| r.kind == line.kind) {
            if r.want != Want::Nothing && !wants.contains(&r.want) {
                wants.push(r.want.clone());
            }
        }
        let what = if of.is_empty() { line.title.trim() } else { of };
        self.obtain(&wants, more, what, 0)
    }

    /// `wants` fetched where nothing is hunted, else hunted where they drop, else the key a chest
    /// holding one asks obtained first, [`KEY_DEPTH`] deep; else why not.
    fn obtain(
        &self,
        wants: &[Want],
        more: u32,
        what: &str,
        depth: usize,
    ) -> (PursuitStep, Option<Hunting>) {
        let mut locked: Vec<(u32, &'static str)> = Vec::new();
        if let Some(step) = self.fetch_step(wants, more, &mut locked) {
            return (step, None);
        }
        let hunted = self.ground_step(wants, more, what);
        if !matches!(hunted.0, PursuitStep::Stuck { .. }) || depth >= KEY_DEPTH {
            return hunted;
        }
        let mut first_why = None;
        for (key, name) in locked {
            let got = self.obtain(&[Want::Item(key)], 1, name, depth + 1);
            match got.0 {
                PursuitStep::Stuck { why } => {
                    first_why.get_or_insert(why);
                }
                _ => return got,
            }
        }
        match first_why {
            Some(why) => (stuck(format!("{what} lies in a locked chest: {why}")), None),
            None => hunted,
        }
    }

    /// Hunt the best grounds a way in this character may use reaches; else why not.
    fn ground_step(&self, wants: &[Want], more: u32, what: &str) -> (PursuitStep, Option<Hunting>) {
        let level = self.facts.level;
        let grounds: Vec<SourceGround> =
            wants.iter().flat_map(|w| self.chains.grounds(w)).collect();
        let holds = |flag: &str| self.facts.held.iter().any(|h| h.eq_ignore_ascii_case(flag));
        // Only the game's rules shut a ground: its way in's level band, never its creatures' level.
        let level_fits = |g: &SourceGround| {
            g.way_in
                .is_none_or(|p| level >= p.min_level && (p.max_level == 0 || level <= p.max_level))
        };
        let flag_held = |g: &SourceGround| {
            g.way_in
                .is_none_or(|p| p.quest.is_empty() || holds(&p.quest))
        };
        let mut open: Vec<(u32, Place)> = Vec::new();
        let mut first: Option<&SourceGround> = None;
        for g in &grounds {
            let Some(place) = g.place.filter(|_| level_fits(g) && flag_held(g)) else {
                continue;
            };
            if !open.iter().any(|(lb, _)| *lb == g.landblock) && open.len() < HUNT_GROUNDS {
                first.get_or_insert(g);
                open.push((g.landblock, place));
            }
        }
        if let Some(g) = first {
            let mut sought: Vec<u32> = Vec::new();
            for s in grounds.iter().map(|s| s.source) {
                if s.kind.is_hunted() && !sought.contains(&s.wcid) {
                    sought.push(s.wcid);
                }
            }
            let step = PursuitStep::Hunt {
                landblock: g.landblock,
                creature: g.source.name.clone(),
                place: ac_world::quest_chain::place_name(g.place.map_or(0, |p| p.cell)),
                more,
            };
            return (step, Some((open, sought)));
        }
        let shut = grounds
            .iter()
            .filter(|g| g.place.is_some() && level_fits(g));
        if let Some(p) = shut.filter_map(|g| g.way_in).find(|p| !p.quest.is_empty()) {
            return (self.flag_step(&p.quest), None);
        }
        let lowest = grounds
            .iter()
            .filter_map(|g| g.way_in)
            .min_by_key(|p| p.min_level);
        if let Some(p) = lowest.filter(|p| p.min_level > level) {
            let why = format!(
                "the way in to where {what} is found takes level {}",
                p.min_level
            );
            return (stuck(why), None);
        }
        let npc = wants
            .iter()
            .flat_map(|w| self.chains.sources(w))
            .find(|s| s.kind == Yield::Npc);
        let why = match (npc, grounds.is_empty()) {
            (Some(s), true) => {
                format!("{what} comes from {}: autoplay cannot fetch it yet", s.name)
            }
            (None, true) => format!("nobody known yields {what}"),
            (_, false) => format!("no way in is known to where {what} is found"),
        };
        (stuck(why), None)
    }

    /// The step for a flag a way on asks: earned from an unlock NPC the level earns, else why not.
    fn flag_step(&self, flag: &str) -> PursuitStep {
        let by = self.chains.givers(flag);
        let unlocks = || {
            by.iter().filter_map(|b| match b {
                FlagBy::Unlock(u) => Some(*u),
                FlagBy::Quest(_) => None,
            })
        };
        if let Some(u) = unlocks().find(|u| u.min_level <= self.facts.level) {
            if (self.tried)(u) {
                return stuck(format!(
                    "{} was set off to lately for the {flag} flag",
                    u.npc
                ));
            }
            return PursuitStep::EarnFlag {
                flag: u.flag.clone(),
                from: u,
            };
        }
        if let Some(u) = unlocks().next() {
            return stuck(format!(
                "{} gives the {flag} flag from level {}",
                u.npc, u.min_level
            ));
        }
        let quest = by.iter().find_map(|b| match b {
            FlagBy::Quest(row) => Some(*row),
            FlagBy::Unlock(_) => None,
        });
        match quest {
            Some(row) => stuck(format!(
                "the {flag} flag comes first, from {} ({})",
                row.giver,
                row.title.trim()
            )),
            None => stuck(format!("nobody known gives the {flag} flag")),
        }
    }
}

impl Client {
    /// Work each pursued quest's step out with the board just worked out from `facts`, whose
    /// pursuit list is `chained`'s (`refresh_quests`), into `pursuit`.
    pub(crate) fn refresh_pursuit(
        &mut self,
        facts: &QuestFacts,
        chained: &[Chained],
        now: Instant,
    ) {
        let st = &mut self.autoplay.growth;
        st.pursuit_held
            .retain(|_, (_, at)| now.duration_since(*at) < PURSUIT_RETRY);
        st.unlock_tried.expire(now, UNLOCK_RETRY);
        st.sources_dropped.expire(now, PURSUIT_RETRY);
        if chained.is_empty() {
            self.pursuit.clear();
            st.pursuit_grounds.clear();
            st.pursuit_sought.clear();
            self.keep_quest_section(&ChainData);
            return;
        }
        let st = &self.autoplay.growth;
        let held: BTreeMap<String, String> = st
            .pursuit_held
            .iter()
            .map(|(id, (why, _))| (id.clone(), why.clone()))
            .collect();
        let me = facts.me;
        let carried = |w| self.carried_count(w);
        let refused = |r: &Row, p: &Place| self.is_giver_refused(r, p, now);
        let away = |p: &Place| me.map_or(0.0, |m| p.at.distance(m));
        let asking = Asking {
            level: facts.level,
            held: &facts.held,
            carried: &carried,
            refused: &refused,
            away: &away,
        };
        // The visit set off on is the step still: it is not passed over while it walks.
        let visiting = self.visiting();
        let tried = |u: &Unlock| {
            st.unlock_tried.since(&u.flag).is_some() && visiting != Some(u.npc.as_str())
        };
        let leader = self
            .is_led()
            .then(|| self.followed_leader().map(|m| m.name.clone()))
            .flatten();
        let dropped = |s: &Source| self.is_source_dropped(s, now);
        let unlocked = |s: &Source| {
            st.chests_unlocked
                .since(&s.wcid)
                .is_some_and(|t| now.saturating_duration_since(t) < fetch::UNLOCKED_FOR)
        };
        let ask = Ask {
            facts,
            asking: &asking,
            tried: &tried,
            held: &held,
            led: leader.as_deref(),
            chains: &ChainData,
            dropped: &dropped,
            unlocked: &unlocked,
        };
        let pursued = pursuit_of(&self.quest_board, chained, &ask);
        self.pursuit = pursued.lines;
        let st = &mut self.autoplay.growth;
        st.pursuit_grounds = pursued.grounds;
        st.pursuit_sought = pursued.sought;
        self.keep_quest_section(&ChainData);
    }

    /// The working pursued quest, if one can go on.
    fn working_pursuit(&self) -> Option<&PursuitLine> {
        self.pursuit.iter().find(|l| l.working)
    }

    /// Whether the working pursued quest's step is a hunt.
    pub(crate) fn is_pursued_hunt(&self) -> bool {
        self.working_pursuit()
            .is_some_and(|l| matches!(l.step, PursuitStep::Hunt { .. }))
    }

    /// Whether a pursued quest is being worked: its step outranks the ground and the area.
    pub fn is_pursuing(&self) -> bool {
        self.working_pursuit().is_some()
    }

    /// The giver the working pursued quest calls on now, and where: to take it up or hand it in.
    pub(crate) fn pursuit_call(&self) -> Option<(&'static Row, Place)> {
        match self.working_pursuit()?.step {
            PursuitStep::TakeUp { row, at } | PursuitStep::HandIn { row, at } => Some((row, at)),
            _ => None,
        }
    }

    /// Pass the pursued quest `id` over for [`PURSUIT_RETRY`], saying `why`; the board is worked
    /// out again at once, so the next pursued quest takes its turn.
    pub(crate) fn hold_pursuit(&mut self, id: &str, why: String, now: Instant) {
        tracing::info!("pursuit: {id} passed over: {why}");
        self.autoplay
            .note(format!("pursuit passed over: {why}"), now);
        let held = &mut self.autoplay.growth.pursuit_held;
        held.insert(id.to_string(), (why, now));
        self.quest_board_at = None;
    }

    /// The working pursued quest's hunt or flag, from `grow_hunt` ahead of any ground: `None` when
    /// it has neither or the character stands on its ground, else whether it set off.
    pub(super) fn grow_pursuit(&mut self, now: Instant) -> Option<bool> {
        let line = self.working_pursuit()?.clone();
        match &line.step {
            PursuitStep::EarnFlag { flag, from } => {
                self.autoplay.growth.unlock_tried.mark(flag.clone(), now);
                if self.visit_unlock(from) {
                    let why = format!("for {}: to {} for the {flag} flag", line.title, from.npc);
                    self.autoplay.say(Doing::Traveling, why);
                    return Some(true);
                }
                let why = format!("no way to {} for the {flag} flag", from.npc);
                self.hold_pursuit(&line.id, why, now);
                Some(false)
            }
            PursuitStep::Hunt { creature, .. } => {
                let here = self.player.as_ref()?.cell >> 16;
                let mut grounds = self.autoplay.growth.pursuit_grounds.clone();
                if grounds.iter().any(|(lb, _)| *lb == here) {
                    self.autoplay.growth.hunting_at = Some(here);
                    return None;
                }
                // The ground reached soonest first: the most spawns may be a 75-minute road away.
                let quests = self.travel.quests_done.clone();
                let mut secs: Vec<(f32, (u32, Place))> = Vec::new();
                for g in grounds.drain(..) {
                    let t = self
                        .journey_plan(g.1, &quests)
                        .map_or(f32::MAX, |t| t.seconds);
                    secs.push((t, g));
                }
                secs.sort_by(|a, b| a.0.total_cmp(&b.0));
                for (lb, at) in secs.into_iter().map(|(_, g)| g) {
                    let st = &mut self.autoplay.growth;
                    st.skip.expire(now, SKIP_GROUND_FOR);
                    if st.skip.since(&lb).is_some() {
                        continue;
                    }
                    if self.grow_travel(at, GROUND_REACH) {
                        let st = &mut self.autoplay.growth;
                        st.bound = Some((lb, at.at.truncate(), creature.clone()));
                        st.quiet_since = None;
                        // The ground taken, which need not be the step's first.
                        let to = ac_world::quest_chain::place_name(at.cell);
                        let why =
                            format!("for {}: on the way to hunt {creature} in {to}", line.title);
                        self.autoplay.say(Doing::Traveling, why);
                        return Some(true);
                    }
                    self.autoplay.growth.skip.mark(lb, now);
                }
                let why = format!("no journey reaches any place {creature} is found");
                self.hold_pursuit(&line.id, why, now);
                Some(false)
            }
            PursuitStep::PickUp { .. }
            | PursuitStep::OpenChest { .. }
            | PursuitStep::UseOn { .. }
            | PursuitStep::Combine { .. } => self.grow_fetch(&line, now),
            _ => None,
        }
    }

    /// Where a journey to the pursued ground of `lb` ends, when `lb` is one.
    pub(crate) fn pursued_place(&self, lb: u32) -> Option<Place> {
        let grounds = &self.autoplay.growth.pursuit_grounds;
        grounds.iter().find(|(l, _)| *l == lb).map(|(_, p)| *p)
    }
}

#[cfg(test)]
mod tests;
