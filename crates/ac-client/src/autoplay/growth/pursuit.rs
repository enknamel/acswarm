//! Quests the player asked for, worked before hunting for XP: each pursued quest's step for this
//! character, worked out with the board; the town run calls on givers, `grow_hunt` hunts sources.
//! A flag a way on asks is earned first: from an unlock NPC, by a kill (`flag_way`), or by the quest
//! stamping it, worked ahead of the one that needs it (`with_chain`). An item is fetched before it is hunted (`fetch`).
//! What a chain needs is kept and bought only through the profile's Quests section (`section`).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use ac_world::quest_chain::{ChainLink, FlagBy, SourceGround};
use ac_world::quests::{Handing, Kind, Row, Source, Want, Yield};
use ac_world::unlocks::Unlock;
use ac_world::Place;

use super::hunt::{GROUND_REACH, SKIP_GROUND_FOR, UNLOCK_RETRY};
use super::town_run::calls::Asking;
use crate::autoplay::{Doing, Pursuit, QuestEnd};
use crate::quest_log::{Blocked, QuestFacts, QuestLine, QuestState};
use crate::Client;

mod events;
mod fetch;
mod flag_way;
mod gem_way;
mod kept;
pub(crate) mod pace;
mod section;
mod sweep;
pub(crate) use fetch::Fetching;
pub(crate) use gem_way::GemWanted;

/// A pursued quest that could not go on is passed over this long before it is tried again (guess).
pub const PURSUIT_RETRY: Duration = Duration::from_secs(10 * 60);
/// Flags looked through for a quest to work first, a quest needing one needing another (guess).
const CHAIN_DEPTH: u32 = 3;
/// Links a want is followed through: another quest's reward, a missing item, a chest's key, an
/// event. Aerfalle's Token's Sluice Gate Bar is the eighth (key, three events, golem, two events).
const OBTAIN_DEPTH: usize = 10;
/// Most grounds a rotating hunt sweeps at once, nearest first, or an empty ground weighs leaving
/// for: each is a plan over every portal.
const HUNT_GROUNDS: usize = 6;
/// Most grounds a hunt holds, soonest first at the ground: those that cannot beat the best
/// journey and time there found yet are never planned (see `grow_pursuit`).
const PACED_GROUNDS: usize = 20;
/// Most grounds a hunt holds when they come only by rotating (a rare camp's), nearest first: the
/// data keeps 64 a creature.
const SWEEP_GROUNDS: usize = 64;

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
    /// Walk to `source`'s NPC at `at` and use it: it hands the wanted item over (Shin Ro's key).
    AskFor { source: &'static Source, at: Place },
    /// Walk to `source`'s NPC at `at` and hand it the carried `item` (weenie class): that starts
    /// the event the source is for (the Sluice Gate Bar to the Repair Golem).
    GiveTo {
        source: &'static Source,
        at: Place,
        item: u32,
    },
    /// Use the carried `with` on the carried `on` (weenie classes), where the character stands.
    Combine {
        source: &'static Source,
        with: u32,
        on: u32,
    },
    /// Hand in to `row`'s giver at `at`.
    HandIn { row: &'static Row, at: Place },
    /// Read the carried scroll `name` (weenie class `wcid`) for `spell`: no giver, the scroll
    /// reader (`autoplay_learn_scrolls`) reads it while nothing else is on.
    Read { wcid: u32, name: String, spell: u32 },
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
                | PursuitStep::AskFor { .. }
                | PursuitStep::GiveTo { .. }
                | PursuitStep::Combine { .. }
                | PursuitStep::HandIn { .. }
                | PursuitStep::Read { .. }
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
            PursuitStep::AskFor { source, at } => {
                let place = ac_world::quest_chain::place_name(at.cell);
                format!("ask {} for it in {place}", source.name)
            }
            PursuitStep::GiveTo { source, at, item } => {
                let given = source.keys.iter().find(|k| k.0 == *item);
                let given = given.map_or_else(|| format!("item {item}"), |k| k.1.clone());
                let place = ac_world::quest_chain::place_name(at.cell);
                format!("give {given} to {} in {place}", source.name)
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
            PursuitStep::Read { name, .. } => format!("read {name}"),
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
                    outcome: QuestEnd::HandIn,
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
    /// Each of `grounds`' droppers and pace there, in its order.
    pub(crate) paces: Vec<pace::Ground>,
    /// The working hunt's creatures, by weenie class: fought before their namesakes.
    pub(crate) sought: Vec<u32>,
    /// Every creature the hunt's item drops from that this character can kill, wherever: class,
    /// chance a kill and seconds a kill (`pace::road_droppers` picks those worth a road fight).
    pub(crate) droppers: Vec<(u32, f32, f32)>,
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
    /// The portal gem each quest, by id, is had for first: no journey reaches its ground but
    /// from the gem's exit. Gone once carried.
    pub(crate) ways: &'a BTreeMap<String, (u32, String)>,
    /// The quests, by id, whose ground only a gem reaches, carried, used or still wanted.
    pub(crate) gem_grounds: &'a std::collections::BTreeSet<String>,
    /// The flag each quest, by id, is had for first: no journey reaches its ground but past a
    /// portal asking it. Gone once held.
    pub(crate) flag_ways: &'a BTreeMap<String, String>,
    /// The landblock the character stands in, by its cell (a dungeon's rooms run past its square).
    pub(crate) here: Option<u32>,
    /// The pursued ground the character set off for, by landblock.
    pub(crate) bound: Option<u32>,
    /// Seconds a kill of a creature takes this character, by class; `None` for one it cannot
    /// kill reliably ([`pace::Killing::secs`]).
    pub(crate) kill_secs: &'a dyn Fn(u32) -> Option<f32>,
    /// Whether a server event is taken for on (`events::belief`): what it gates is about.
    pub(crate) event_on: &'a dyn Fn(&str) -> bool,
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
            let (step, h) = ask.step_of(line, &c.pursuit.outcome);
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
    // A hunt whose ground only a gem reaches is worked first while the character stands on it:
    // leaving to come back costs another gem (logged in in Ithaenc's catacombs). Any other ground
    // under foot waits its turn in the player's order (the Tusker Barracks, a letter to stamp).
    let on_ground = |l: &PursuitLine, h: &Option<Hunting>| {
        ask.gem_grounds.contains(&l.id)
            && h.as_ref()
                .is_some_and(|h| h.grounds.iter().any(|(lb, _)| Some(*lb) == ask.here))
    };
    let first = lines
        .iter()
        .position(|(l, h)| l.step.can_progress() && on_ground(l, h));
    let mut out = Pursued::default();
    for (i, (mut line, hunt)) in lines.into_iter().enumerate() {
        line.working = match first {
            Some(f) => i == f,
            None => line.step.can_progress() && !out.lines.iter().any(|l| l.working),
        };
        if let (true, Some(h)) = (line.working, hunt) {
            out.grounds = h.grounds;
            out.paces = h.paces;
            out.sought = h.sought;
            out.droppers = h.droppers;
        }
        out.lines.push(line);
    }
    out
}

fn stuck(why: String) -> PursuitStep {
    PursuitStep::Stuck { why }
}

/// The scroll `line`'s quest is done by reading, and the spell it teaches: a `learn:` row's.
fn line_reads(line: &QuestLine) -> Option<(u32, String, u32)> {
    let row = crate::quest_log::worked_rows(line).find(|r| r.hand == Handing::Read)?;
    let Want::Item(wcid) = row.want else {
        return None;
    };
    let name = row
        .item
        .as_ref()
        .map_or_else(|| format!("item {wcid}"), |i| i.name.to_string());
    Some((wcid, name, row.teaches()?))
}

/// A hunt's grounds, soonest first at the ground, and the creatures sought there.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Hunting {
    grounds: Vec<(u32, Place)>,
    paces: Vec<pace::Ground>,
    sought: Vec<u32>,
    droppers: Vec<(u32, f32, f32)>,
}

impl Ask<'_> {
    /// One board line's step towards `end`, and its grounds when it is a hunt.
    fn step_of(&self, line: &QuestLine, end: &QuestEnd) -> (PursuitStep, Option<Hunting>) {
        let level = self.facts.level;
        let step = match &line.state {
            QuestState::Off => stuck("switched off".to_string()),
            QuestState::Done => PursuitStep::Finished,
            QuestState::Waiting { until } => PursuitStep::Waiting { until: *until },
            QuestState::Ready => match (line_reads(line), self.asking.hand_in_at(line, end)) {
                (Some((wcid, name, spell)), _) => PursuitStep::Read { wcid, name, spell },
                (None, Some((_, row, at))) => PursuitStep::HandIn { row, at },
                // Kept: what a hand-in takes is carried, and `finish_kept` ends the pursuit.
                (None, None) if *end == QuestEnd::Keep && line.kind == Kind::Item => {
                    PursuitStep::Finished
                }
                (None, None) => match self.chosen_short(line, end) {
                    Some((more, of)) => return self.hunt_step(line, more, &of, end),
                    None => stuck(format!("{} cannot be asked now", line.giver)),
                },
            },
            QuestState::NotTaken { from, flag } => match self.asking.take_up_at(from, flag) {
                Some((_, row, at)) => PursuitStep::TakeUp { row, at },
                None => stuck(format!("{from} cannot be asked now")),
            },
            QuestState::Needs { more, of } => {
                let (more, of) = self
                    .chosen_short(line, end)
                    .unwrap_or_else(|| (*more, of.clone()));
                return self.hunt_step(line, more, &of, end);
            }
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
            QuestState::CantYet(Blocked::Locked { flag }) => return self.flag_step(flag, 0),
            QuestState::CantYet(Blocked::NeedsFirst { title }) => {
                stuck(format!("{} comes first", title.trim()))
            }
            QuestState::CantYet(Blocked::CantLearn) => {
                stuck("its spell is beyond this character's magic".to_string())
            }
        };
        (step, None)
    }

    /// For a hand-in to one giver, its item short and how many: the board reads any giver's.
    fn chosen_short(&self, line: &QuestLine, end: &QuestEnd) -> Option<(u32, String)> {
        if !matches!(end, QuestEnd::HandInTo { .. }) {
            return None;
        }
        crate::quest_log::worked_rows(line)
            .filter(|r| r.kind == line.kind && end.can_hand_in(r))
            .find_map(|r| {
                let Want::Item(w) = r.want else { return None };
                let more = r.count.max(1).saturating_sub((self.asking.carried)(w));
                let name = r.item.as_ref().map_or("", |i| i.name);
                (more > 0).then(|| (more, name.to_string()))
            })
    }

    /// Come by what `line` still needs for `end`, `more` of it (`of` in words).
    fn hunt_step(
        &self,
        line: &QuestLine,
        more: u32,
        of: &str,
        end: &QuestEnd,
    ) -> (PursuitStep, Option<Hunting>) {
        let mut wants: Vec<Want> = Vec::new();
        let rows = crate::quest_log::worked_rows(line).filter(|r| r.kind == line.kind);
        // A kept quest comes by what any hand-in takes; one to a chosen giver by that one's.
        for r in rows.filter(|r| *end == QuestEnd::Keep || end.can_hand_in(r)) {
            if r.want != Want::Nothing && !wants.contains(&r.want) {
                wants.push(r.want.clone());
            }
        }
        let what = if of.is_empty() { line.title.trim() } else { of };
        let step = self.obtain(&wants, more, what, 0);
        // No journey reaches where it comes from but from a gem's exit: the gem is had first,
        // unless the character stands on the ground already (bought and used between boards).
        let there =
            matches!(step.0, PursuitStep::Hunt { landblock, .. } if Some(landblock) == self.here);
        if let (Some(flag), false) = (self.flag_ways.get(&line.id), there) {
            return self.flag_step(flag, 0);
        }
        if let (Some((gem, name)), false) = (self.ways.get(&line.id), there) {
            return self.obtain(&[Want::Item(*gem)], 1, name, 0);
        }
        step
    }

    /// `wants` fetched where nothing is hunted, else hunted where they drop, else the key a chest
    /// holding one asks obtained first, [`OBTAIN_DEPTH`] deep; else why not.
    fn obtain(
        &self,
        wants: &[Want],
        more: u32,
        what: &str,
        depth: usize,
    ) -> (PursuitStep, Option<Hunting>) {
        // An item whose pickup stamp waits is not gone for: the server turns the pickup down (the
        // Ancient Key hunted with novquest1 2.6 h off). With every one so, the quest waits.
        let waits = |w: &Want| match w {
            Want::Item(i) => self.facts.log.pickup_wait(*i, self.facts.now),
            _ => None,
        };
        let open: Vec<Want> = wants
            .iter()
            .filter(|w| waits(w).is_none())
            .cloned()
            .collect();
        if open.is_empty() {
            if let Some(until) = wants.iter().filter_map(waits).min() {
                return (PursuitStep::Waiting { until }, None);
            }
        }
        let wants = &open[..];
        let mut locked: Vec<(u32, &'static str)> = Vec::new();
        let mut events: Vec<&'static str> = Vec::new();
        if let Some(step) = self.fetch_step(wants, more, &mut locked, &mut events) {
            return (step, None);
        }
        let hunted = self.ground_step(wants, more, what, &mut events, depth);
        if !matches!(hunted.0, PursuitStep::Stuck { .. }) || depth >= OBTAIN_DEPTH {
            return hunted;
        }
        // Another quest's reward: quests chain (a Singularity Scepter handed in for the Bound one).
        if let Some(got) = self.reward_step(wants, depth) {
            return got;
        }
        let mut first_why = None;
        for (key, name) in locked {
            let got = self.obtain(&[Want::Item(key)], 1, name, depth + 1);
            match got.0 {
                PursuitStep::Stuck { why } => {
                    first_why.get_or_insert((name.to_string(), why));
                }
                _ => return got,
            }
        }
        // What only an event puts out: the event started first, as a key is had.
        for e in events {
            let name = format!("the {e} event");
            let got = self.obtain(&[Want::Event(e.to_string())], 1, &name, depth + 1);
            match got.0 {
                PursuitStep::Stuck { why } => {
                    first_why.get_or_insert((name, why));
                }
                _ => return got,
            }
        }
        // What is missing is a chest's key or an item used to make it: named, as either.
        match first_why {
            Some((name, why)) => (stuck(format!("{what} needs {name} first: {why}")), None),
            None => hunted,
        }
    }

    /// `wants` had as another quest's reward: its hand-in made when what it takes is carried, else
    /// that come by first, [`OBTAIN_DEPTH`] deep; `None` when no quest gives any of them.
    fn reward_step(&self, wants: &[Want], depth: usize) -> Option<(PursuitStep, Option<Hunting>)> {
        // A swap of an item for one like it (a Glowing Virindi Cloak for a Glowing Virindi Cloak) is
        // no way to come by it.
        let gives = |r: &Row| {
            r.gives
                .iter()
                .any(|(w, _)| wants.contains(&Want::Item(*w)) && r.want != Want::Item(*w))
        };
        let mut first_stuck = None;
        for r in ac_world::quests::all()
            .iter()
            .filter(|r| r.kind == Kind::Item && r.hand == Handing::Give && gives(r))
        {
            let Want::Item(input) = r.want else {
                continue;
            };
            let Some(at) = r.places.iter().find(|p| !(self.asking.refused)(r, p)) else {
                continue;
            };
            if (self.asking.carried)(input) >= r.count.max(1) {
                return Some((PursuitStep::HandIn { row: r, at: *at }, None));
            }
            let name = r.item.as_ref().map_or("", |i| i.name);
            let got = self.obtain(
                std::slice::from_ref(&r.want),
                r.count.max(1),
                name,
                depth + 1,
            );
            if !matches!(got.0, PursuitStep::Stuck { .. }) {
                return Some(got);
            }
            first_stuck.get_or_insert(got);
        }
        first_stuck
    }

    /// Hunt the grounds a way in this character may use reaches, soonest to the item at the ground
    /// first ([`pace::ground_pace`]), leaving out what it cannot kill; else why not.
    fn ground_step(
        &self,
        wants: &[Want],
        more: u32,
        what: &str,
        events: &mut Vec<&'static str>,
        depth: usize,
    ) -> (PursuitStep, Option<Hunting>) {
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
        // Each open landblock's droppers this character can kill, the data's best one first.
        let mut blocks: Vec<(&SourceGround, Place, Vec<pace::Dropper>)> = Vec::new();
        let mut droppers: Vec<(u32, f32, f32)> = Vec::new();
        let mut unkillable = false;
        for g in grounds.iter().filter(|g| g.source.kind.is_hunted()) {
            // One only an event puts out is hunted once the event is on.
            if let Some(e) = g.source.event.as_deref().filter(|e| !(self.event_on)(e)) {
                if !events.contains(&e) {
                    events.push(e);
                }
                continue;
            }
            let Some(place) = g.place.filter(|_| level_fits(g) && flag_held(g)) else {
                continue;
            };
            let Some(kill) = (self.kill_secs)(g.source.wcid) else {
                unkillable = true;
                continue;
            };
            if !droppers.iter().any(|d| d.0 == g.source.wcid) {
                droppers.push((g.source.wcid, g.source.chance, kill));
            }
            let d = pace::Dropper {
                chance: g.source.chance,
                kill,
                rate: g.rate,
            };
            match blocks.iter_mut().find(|b| b.0.landblock == g.landblock) {
                Some(b) => b.2.push(d),
                None => blocks.push((g, place, vec![d])),
            }
        }
        let mut paced: Vec<(&SourceGround, Place, pace::Ground)> = blocks
            .into_iter()
            .filter_map(|(g, place, ds)| Some((g, place, pace::Ground::of(ds)?)))
            .collect();
        paced.sort_by(|a, b| a.2.pace.soonest().total_cmp(&b.2.pace.soonest()));
        // A hunt whose best ground comes only by rotating holds them all to sweep, nearest first.
        let rotating = paced
            .first()
            .is_some_and(|p| p.2.pace.camp > p.2.pace.rotate);
        paced.truncate(if rotating {
            SWEEP_GROUNDS
        } else {
            PACED_GROUNDS
        });
        if let Some(&(g, ..)) = paced.first() {
            // Only the creatures of the soonest grounds there, and of the one stood on or set off
            // for: the Sturdy Iron Key drops from 398, and a Gotrok Tiatus at 2% met on the road
            // was fought, never walked past.
            let most = if rotating {
                SWEEP_GROUNDS
            } else {
                HUNT_GROUNDS
            };
            let goes = |lb: u32| Some(lb) == self.here || Some(lb) == self.bound;
            let held = |g: &&SourceGround| {
                let soon = paced
                    .iter()
                    .take(most)
                    .any(|p| p.0.landblock == g.landblock);
                soon || (goes(g.landblock) && paced.iter().any(|p| p.0.landblock == g.landblock))
            };
            let mut sought: Vec<u32> = Vec::new();
            for s in grounds.iter().filter(held).map(|g| g.source) {
                let can = droppers.iter().any(|d| d.0 == s.wcid);
                if s.kind.is_hunted() && can && !sought.contains(&s.wcid) {
                    sought.push(s.wcid);
                }
            }
            let step = PursuitStep::Hunt {
                landblock: g.landblock,
                creature: g.source.name.clone(),
                place: ac_world::quest_chain::place_name(g.place.map_or(0, |p| p.cell)),
                more,
            };
            let hunting = Hunting {
                grounds: paced.iter().map(|p| (p.0.landblock, p.1)).collect(),
                paces: paced.iter().map(|p| p.2.clone()).collect(),
                sought,
                droppers,
            };
            return (step, Some(hunting));
        }
        let shut = grounds
            .iter()
            .filter(|g| g.place.is_some() && level_fits(g));
        if let Some(p) = shut.filter_map(|g| g.way_in).find(|p| !p.quest.is_empty()) {
            return self.flag_step(&p.quest, depth);
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
            (_, false) if !droppers.is_empty() => {
                format!("where a way in reaches, nothing that drops {what} is ever about")
            }
            (_, false) if unkillable => {
                format!("{what} drops only from creatures this character cannot kill yet")
            }
            (Some(s), true) => {
                format!("{what} comes from {}: autoplay cannot fetch it yet", s.name)
            }
            (None, true) => format!("nobody known yields {what}"),
            (_, false) => format!("no way in is known to where {what} is found"),
        };
        (stuck(why), None)
    }

    /// The step for a flag a way on asks: earned from an unlock NPC the level earns, else by a
    /// kill that stamps it ([`OBTAIN_DEPTH`] deep, as a key), else why not.
    fn flag_step(&self, flag: &str, depth: usize) -> (PursuitStep, Option<Hunting>) {
        let by = self.chains.givers(flag);
        let unlocks = || {
            by.iter().filter_map(|b| match b {
                FlagBy::Unlock(u) => Some(*u),
                _ => None,
            })
        };
        if let Some(u) = unlocks().find(|u| u.min_level <= self.facts.level) {
            if (self.tried)(u) {
                let why = format!("{} was set off to lately for the {flag} flag", u.npc);
                return (stuck(why), None);
            }
            let step = PursuitStep::EarnFlag {
                flag: u.flag.clone(),
                from: u,
            };
            return (step, None);
        }
        if let Some(u) = unlocks().next() {
            let why = format!("{} gives the {flag} flag from level {}", u.npc, u.min_level);
            return (stuck(why), None);
        }
        let mut killed = None;
        if by.iter().any(|b| matches!(b, FlagBy::Kill(_))) && depth < OBTAIN_DEPTH {
            let want = Want::Flag(flag.to_ascii_lowercase());
            let got = self.obtain(&[want], 1, &format!("the {flag} flag"), depth + 1);
            if !matches!(got.0, PursuitStep::Stuck { .. }) {
                return got;
            }
            killed = Some(got);
        }
        let quest = by.iter().find_map(|b| match b {
            FlagBy::Quest(row) => Some(*row),
            _ => None,
        });
        match (quest, killed) {
            (Some(row), _) => {
                let why = format!(
                    "the {flag} flag comes first, from {} ({})",
                    row.giver,
                    row.title.trim()
                );
                (stuck(why), None)
            }
            (None, Some(got)) => got,
            (None, None) => (stuck(format!("nobody known gives the {flag} flag")), None),
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
        self.see_events(now);
        let st = &mut self.autoplay.growth;
        st.pursuit_held
            .retain(|_, (_, at)| now.duration_since(*at) < PURSUIT_RETRY);
        st.unlock_tried.expire(now, UNLOCK_RETRY);
        st.sources_dropped.expire(now, PURSUIT_RETRY);
        if chained.is_empty() {
            self.pursuit.clear();
            st.pursuit_grounds.clear();
            st.pursuit_paces.clear();
            st.pursuit_gems.clear();
            st.pursuit_flags.clear();
            st.pursuit_sought.clear();
            st.pursuit_droppers.clear();
            st.pursuit_road.clear();
            st.plan_secs = None;
            self.keep_quest_section(&ChainData);
            return;
        }
        self.keep_gem_ways();
        let held_flags = &facts.held;
        let st = &mut self.autoplay.growth;
        st.pursuit_flags
            .retain(|_, f| !held_flags.iter().any(|h| h.eq_ignore_ascii_case(f)));
        let flag_ways = st.pursuit_flags.clone();
        let st = &self.autoplay.growth;
        // A gem carried is the way already: the planner takes it.
        let ways: BTreeMap<String, (u32, String)> = st
            .pursuit_gems
            .iter()
            .filter(|g| !g.had)
            .map(|g| (g.quest.clone(), (g.wcid, g.name.clone())))
            .collect();
        let gem_grounds: std::collections::BTreeSet<String> =
            st.pursuit_gems.iter().map(|g| g.quest.clone()).collect();
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
        // Or its key sent and the answer not yet in: the key leaves the pack before the server says
        // it unlocked, and the board, asked between, went for another key (test:
        // a_chest_whose_key_is_out_is_opened_not_another_key_hunted).
        let unlocked = |s: &Source| {
            let unlocking = st
                .fetching
                .as_ref()
                .is_some_and(|f| f.act == fetch::Act::Unlock && f.key.1 == s.wcid);
            unlocking
                || st
                    .chests_unlocked
                    .since(&s.wcid)
                    .is_some_and(|t| now.saturating_duration_since(t) < fetch::UNLOCKED_FOR)
        };
        let killing = self.killing();
        let kill_secs = |wcid: u32| killing.secs(wcid);
        let event_on = |e: &str| self.is_event_on(e, now);
        let ask = Ask {
            facts,
            asking: &asking,
            tried: &tried,
            held: &held,
            led: leader.as_deref(),
            chains: &ChainData,
            dropped: &dropped,
            unlocked: &unlocked,
            ways: &ways,
            gem_grounds: &gem_grounds,
            flag_ways: &flag_ways,
            here: self.player.as_ref().map(|p| p.cell >> 16),
            bound: self.autoplay.growth.bound_landblock(),
            kill_secs: &kill_secs,
            event_on: &event_on,
        };
        let pursued = pursuit_of(&self.quest_board, chained, &ask);
        drop(killing);
        self.pursuit = pursued.lines;
        let st = &mut self.autoplay.growth;
        // Another hunt, or its droppers another: the plan set off on is not its plan.
        let classes = |d: &[(u32, f32, f32)]| d.iter().map(|d| d.0).collect::<Vec<u32>>();
        if classes(&st.pursuit_droppers) != classes(&pursued.droppers) {
            st.plan_secs = None;
        }
        st.pursuit_grounds = pursued.grounds;
        st.pursuit_paces = pursued.paces;
        st.pursuit_sought = pursued.sought;
        st.pursuit_droppers = pursued.droppers;
        self.keep_road_droppers();
        self.keep_quest_section(&ChainData);
    }

    /// The working pursued quest, if one can go on.
    pub(super) fn working_pursuit(&self) -> Option<&PursuitLine> {
        self.pursuit.iter().find(|l| l.working)
    }

    /// Whether `quest` is the working pursuit's hunt and the body is on its way to one of its
    /// grounds, there, or where a journey reaches one as things are: none of its own buys is an
    /// errand then (its gem was just used, or logged in by the gem's exit).
    pub(super) fn is_on_quest_road(&self, quest: &str) -> bool {
        let hunting = self
            .working_pursuit()
            .is_some_and(|l| l.id == quest && matches!(l.step, PursuitStep::Hunt { .. }));
        let here = self.player.as_ref().map_or(0, |p| p.cell >> 16);
        let bound = self.autoplay.growth.bound_landblock();
        let ground = |lb: u32| self.pursued_place(lb).is_some();
        let reached = matches!(&self.autoplay.growth.quest_road,
            Some((lb, q, true)) if *lb == here && q == quest);
        hunting && (ground(here) || reached || (self.traveling() && bound.is_some_and(ground)))
    }

    /// Plan, once per landblock stood in, whether a journey from here reaches the working
    /// pursuit's hunting grounds as things are (`State.quest_road`).
    pub(super) fn look_down_quest_road(&mut self) {
        let Some(line) = self.working_pursuit().cloned() else {
            return;
        };
        if !matches!(line.step, PursuitStep::Hunt { .. }) {
            return;
        }
        let here = self.player.as_ref().map_or(0, |p| p.cell >> 16);
        let looked = &self.autoplay.growth.quest_road;
        if looked
            .as_ref()
            .is_some_and(|(lb, q, _)| *lb == here && *q == line.id)
        {
            return;
        }
        let quests = self.travel.quests_done.clone();
        let grounds = self.autoplay.growth.pursuit_grounds.clone();
        let reached = grounds
            .iter()
            .any(|(_, at)| self.journey_plan(*at, &quests).is_some());
        self.autoplay.growth.quest_road = Some((here, line.id, reached));
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

    /// The working pursued quest's buy, its title and the ware's name: a town run's errand.
    pub(crate) fn pursued_buy(&self) -> Option<(String, String)> {
        let line = self.working_pursuit()?;
        match &line.step {
            PursuitStep::Buy { name, .. } => Some((line.title.clone(), name.clone())),
            _ => None,
        }
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
    /// A kill at `at`: one of the pursued hunt's creatures is noted, its body to be stood by.
    pub(crate) fn note_sought_kill(&mut self, at: glam::Vec3, now: Instant) {
        let it = self.world.objects.values().any(|o| {
            self.is_sought(o.weenie_class_id) && o.world_pos().is_some_and(|p| p.distance(at) < 1.0)
        });
        if it {
            self.autoplay.growth.sought_kill = Some((at, now));
        }
        self.note_event_kill(at, now);
        self.note_flag_kill(at);
    }

    /// The body of the pursued hunt's creature killed, once it lies there (`sought_kill`): opened
    /// under blows too (a_sought_kill_s_body_is_opened_under_blows).
    pub(crate) fn sought_body(&self) -> Option<u32> {
        use crate::autoplay::loot::owed::KILL_SPOT;
        let (at, _) = self.autoplay.growth.sought_kill?;
        // Kin killed beside it lie there too: the one not yet looted, then the nearest.
        let looted = &self.autoplay.looted;
        self.world
            .objects
            .values()
            .filter(|o| o.object_desc_flags & ac_world::object_desc_flags::CORPSE != 0)
            .filter_map(|o| Some((o.guid, o.world_pos()?.truncate().distance(at.truncate()))))
            .filter(|(_, d)| *d <= KILL_SPOT)
            .min_by(|a, b| {
                let key = |(g, d): &(u32, f32)| (looted.contains(g), *d);
                key(a)
                    .partial_cmp(&key(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(g, _)| g)
    }

    /// The pursued hunt's creature killed: its body stood by, walked back to, till the loot rules
    /// empty it. The walk on left Martine's cloak behind, and a hit stopped the looting.
    pub(super) fn stand_by_sought_body(&mut self, now: Instant) -> Option<bool> {
        use crate::autoplay::loot::choose::{CORPSE_APPEARS, CORPSE_LIFE, LOOT_NEAR};
        let (at, fell) = self.autoplay.growth.sought_kill?;
        let since = now.saturating_duration_since(fell);
        let body = self.sought_body();
        let waiting = match body {
            // The body comes a moment after the blow (twice that allowed).
            None => since < CORPSE_APPEARS * 2,
            // Only a body the looting would open: one it will not (a pack low on room) is no wait.
            Some(g) => {
                let room = self.room_for_loot();
                since < CORPSE_LIFE && self.autoplay.corpse_waiting(g, now, room)
            }
        };
        if !waiting {
            self.autoplay.growth.sought_kill = None;
            return None;
        }
        self.autoplay.growth.drop_ground_walk();
        let me = self.my_position()?;
        if me.truncate().distance(at.truncate()) > LOOT_NEAR * 0.5 {
            if self.grow_travel(self.place_at(at), 2.0) {
                self.autoplay
                    .say(Doing::Traveling, "back to the quest creature's body");
                return Some(true);
            }
            self.autoplay.growth.sought_kill = None;
            return None;
        }
        if self.traveling() {
            self.cancel_travel();
        }
        self.autoplay
            .say(Doing::Idle, "by the quest creature's body");
        Some(true)
    }

    /// A pursued hunt's creature in sight on the road between its grounds, a rare camp's or one
    /// the road rule finds worth its kill ([`pace::road_droppers`]): gone to, the ground walk kept
    /// so this is asked again, not its plan; `Some` is the hunt row's answer (false: left to the
    /// fight rule, in its reach). In reach or the chase over, the ground is planned afresh.
    pub(super) fn chase_sought_on_road(&mut self, now: Instant) -> Option<bool> {
        let line = self.working_pursuit()?.clone();
        let PursuitStep::Hunt { creature, .. } = &line.step else {
            return None;
        };
        let st = &self.autoplay.growth;
        let mut chased = st.pursuit_sought.clone();
        if !self.is_rotating_hunt() {
            chased.retain(|w| st.pursuit_road.contains(w));
        }
        let chasing = self.autoplay.growth.chased.is_some();
        match self.chase_in_sight(&line.title, creature, &chased, now) {
            Some(Some(walking)) => Some(walking),
            Some(None) => {
                self.autoplay.growth.bound = None;
                Some(false)
            }
            None => {
                if chasing {
                    self.autoplay.growth.bound = None;
                }
                None
            }
        }
    }

    pub(super) fn grow_pursuit(&mut self, now: Instant) -> Option<bool> {
        let line = self.working_pursuit()?.clone();
        // Handed in and not yet answered: the item gone, the board would have another made.
        if let Some(giver) = self.quest_asked.as_ref().filter(|a| a.row.id == line.id) {
            let why = format!(
                "for {}: waiting for {}'s answer",
                line.title, giver.row.giver
            );
            self.autoplay.say(Doing::Shopping, why);
            return Some(true);
        }
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
                // Worked out once: the sweep, the camp and the holds read one ground stood on.
                let stood = self.ground_stood_on(&grounds, here);
                if let Some(answer) = self.sweep(&line.title, creature, &mut grounds, stood, now) {
                    return answer;
                }
                if let Some(answer) = self.wait_out_unload(&line.title) {
                    return answer;
                }
                let stood = stood.filter(|lb| grounds.iter().any(|g| g.0 == *lb));
                let camped = stood.is_some();
                let here = stood.unwrap_or(here);
                // Cleared where camping beats leaving and others hold its slots: they make way.
                let clears = camped && self.is_cleared_ground(here, now);
                self.autoplay.growth.clearing = clears.then_some(here);
                if camped {
                    // There: the road's plan is done, and the gem that was the way is bought no more.
                    let st = &mut self.autoplay.growth;
                    if st.plan_secs.take().is_some() {
                        self.keep_road_droppers();
                    }
                    let st = &mut self.autoplay.growth;
                    let before = st.pursuit_gems.len();
                    st.pursuit_gems.retain(|g| g.quest != line.id);
                    if st.pursuit_gems.len() != before {
                        self.quest_board_at = None;
                    }
                    self.autoplay.growth.hunting_at = Some(here);
                    if let Some(answer) = self.hunt_ground(&line.title, creature, here, now) {
                        return answer;
                    }
                    // Left for the next: it is gone back to once it has unloaded.
                    grounds.retain(|(lb, _)| *lb != here);
                    // A rotating hunt's last ground left: on to those left longest ago.
                    if grounds.is_empty() && self.is_rotating_hunt() {
                        grounds = self.grounds_left_longest(here).unwrap_or_default();
                    }
                }
                let every = grounds.clone();
                // The item soonest first: the journey there and the seconds there. The most spawns
                // may be a 75-minute road away, and the far Chakron's 80% is worse than a near 20%.
                // A ground whose time there alone is past the best total yet is not planned: it
                // keeps that time, a bound, and is tried only once the better ones have no way.
                let quests = self.travel.quests_done.clone();
                let mut secs: Vec<(f32, (u32, Place))> = Vec::new();
                let mut best = f32::INFINITY;
                self.autoplay.growth.skip.expire(now, SKIP_GROUND_FOR);
                grounds.retain(|(lb, _)| self.autoplay.growth.skip.since(lb).is_none());
                for g in grounds.drain(..) {
                    let there = self.pace_now(g.0, now).map_or(0.0, |p| p.soonest());
                    if there >= best {
                        secs.push((there, g));
                        continue;
                    }
                    let t = self
                        .journey_plan(g.1, &quests)
                        .map_or(f32::MAX, |t| t.seconds + there);
                    best = best.min(t);
                    secs.push((t, g));
                }
                secs.sort_by(|a, b| a.0.total_cmp(&b.0));
                for (plan, (lb, at)) in secs {
                    let st = &mut self.autoplay.growth;
                    st.skip.expire(now, SKIP_GROUND_FOR);
                    if st.skip.since(&lb).is_some() {
                        continue;
                    }
                    if self.grow_travel(at, self.ground_reach(lb)) {
                        let st = &mut self.autoplay.growth;
                        st.bound = Some((lb, at.at.truncate(), creature.clone()));
                        st.quiet_since = None;
                        st.plan_secs = Some(plan);
                        // Past the soonest few there, its creatures are sought once worked out again.
                        let i = st.pursuit_grounds.iter().position(|g| g.0 == lb);
                        if i.is_some_and(|i| i >= HUNT_GROUNDS) && !self.is_rotating_hunt() {
                            self.quest_board_at = None;
                        }
                        self.keep_road_droppers();
                        // The ground taken, which need not be the step's first.
                        let to = ac_world::quest_chain::place_name(at.cell);
                        let why =
                            format!("for {}: on the way to hunt {creature} in {to}", line.title);
                        self.autoplay.say(Doing::Traveling, why);
                        return Some(true);
                    }
                    self.autoplay.growth.skip.mark(lb, now);
                }
                // No way but past a portal asking a flag (VirindiIsland, onto the Singularity Caul's
                // island): the flag first, ahead of any gem, whose sources may lie past it too.
                if let Some(flag) = self.flag_way(&every, &quests) {
                    let why = format!(
                        "for {}: the {flag} flag first, to reach {creature}",
                        line.title
                    );
                    tracing::info!("pursuit: {}: {why}", line.id);
                    self.autoplay.note(why, now);
                    self.autoplay.growth.want_flag(&line.id, flag);
                    self.quest_board_at = None;
                    return Some(false);
                }
                // No way but from a gem's exit: the gem is had first, bought, hunted or fetched
                // as any wanted item, and the next plan takes it (the Wedding Planner's
                // invitation to Ithaenc; Captain Ramelle's Copper Ward gem to Corcima Castle).
                // One gem a quest: the gem's own source out of reach is not looked past again.
                let wanted = self.autoplay.growth.pursuit_gems.iter();
                if let Some(g) = wanted.clone().find(|g| g.quest == line.id) {
                    let why = format!(
                        "no journey reaches any place {creature} is found, {} wanted",
                        g.name
                    );
                    self.hold_pursuit(&line.id, why, now);
                    return Some(false);
                }
                if let Some(gem) = self.gem_way(&every, &quests) {
                    let why = format!(
                        "for {}: {} first, to reach {creature}",
                        line.title, gem.name
                    );
                    tracing::info!("pursuit: {}: {why}", line.id);
                    self.autoplay.note(why, now);
                    let st = &mut self.autoplay.growth;
                    st.want_gem(&line.id, gem);
                    // Passed over for want of a way: with the gem they are tried again.
                    for (lb, _) in &every {
                        st.skip.forget(lb);
                    }
                    self.quest_board_at = None;
                    return Some(false);
                }
                let why = format!("no journey reaches any place {creature} is found");
                self.hold_pursuit(&line.id, why, now);
                Some(false)
            }
            // Stood still for the scroll reader, which reads only while nothing else is on.
            PursuitStep::Read { name, .. } => {
                let why = format!("for {}: reading {name}", line.title);
                self.autoplay.say(Doing::Idle, why);
                Some(true)
            }
            // The town run buys it: stood for while only the run's clock holds it back; held back
            // for anything else (no coin, no way), the ground is hunted meanwhile.
            PursuitStep::Buy { name, vendor, .. } => {
                if self.autoplay.growth.run_waits != Some(now) {
                    return None;
                }
                let why = format!(
                    "for {}: waiting for the town run to {vendor} for {name}",
                    line.title
                );
                self.autoplay.say(Doing::Shopping, why);
                Some(true)
            }
            PursuitStep::PickUp { .. }
            | PursuitStep::OpenChest { .. }
            | PursuitStep::UseOn { .. }
            | PursuitStep::AskFor { .. }
            | PursuitStep::Combine { .. } => self.grow_fetch(&line, now),
            _ => None,
        }
    }

    /// The droppers met on the road worth a fight: those a kill of saves more of the plan's seconds
    /// than it costs ([`pace::road_droppers`]), into `pursuit_road`.
    pub(crate) fn keep_road_droppers(&mut self) {
        let st = &mut self.autoplay.growth;
        let plan = st.plan_secs.unwrap_or(0.0);
        st.pursuit_road = pace::road_droppers(&st.pursuit_droppers, plan);
    }

    /// Whether `wcid` is one the working hunt fights: its grounds' creatures, or a dropper met on
    /// the road worth the fight.
    pub(crate) fn is_sought(&self, wcid: u32) -> bool {
        let st = &self.autoplay.growth;
        st.pursuit_sought.contains(&wcid) || st.pursuit_road.contains(&wcid)
    }

    /// How soon the pursued ground of `lb` gives the item there, when `lb` is one.
    pub(crate) fn pursued_pace(&self, lb: u32) -> Option<&pace::Ground> {
        let st = &self.autoplay.growth;
        let i = st.pursuit_grounds.iter().position(|(l, _)| *l == lb)?;
        st.pursuit_paces.get(i)
    }

    /// Where a journey to the pursued ground of `lb` ends, when `lb` is one.
    pub(crate) fn pursued_place(&self, lb: u32) -> Option<Place> {
        let grounds = &self.autoplay.growth.pursuit_grounds;
        grounds.iter().find(|(l, _)| *l == lb).map(|(_, p)| *p)
    }

    /// The landblock of the ground of `grounds` stood on: `here`'s, or outdoors one whose place
    /// is within the [`GROUND_REACH`] its walk ends at, which may lie over a landblock's edge.
    pub(crate) fn ground_stood_on(&self, grounds: &[(u32, Place)], here: u32) -> Option<u32> {
        if grounds.iter().any(|(lb, _)| *lb == here) {
            return Some(here);
        }
        let cell = self.player.as_ref()?.cell;
        let me = self.my_position()?.truncate();
        grounds
            .iter()
            .filter(|(_, p)| cell & 0xFFFF < 0x100 && !p.is_indoors())
            .find(|(_, p)| p.at.truncate().distance(me) <= GROUND_REACH)
            .map(|(lb, _)| *lb)
    }

    /// Whether the working pursuit hunts in `lb`, at spots a creature it seeks spawns at.
    pub(crate) fn hunts_spawn_spots(&self, lb: u32) -> bool {
        let hunting = matches!(
            self.working_pursuit().map(|l| &l.step),
            Some(PursuitStep::Hunt { .. })
        );
        hunting && self.pursued_place(lb).is_some() && !self.spawn_spots(lb).is_empty()
    }

    /// The spots of `lb` a creature the pursuit seeks spawns at, dungeon rooms and outdoors alike,
    /// by class: the key Putrid Moarsman (8866) is not its thirteen namesakes (4248).
    fn spawn_spots(&self, lb: u32) -> Vec<Place> {
        let sought = &self.autoplay.growth.pursuit_sought;
        ac_world::spawns::in_block(lb << 16)
            .iter()
            .filter(|s| sought.contains(&s.wcid))
            .map(|s| Place {
                at: s.at,
                cell: s.cell,
            })
            .collect()
    }

    /// The nearest of the pursuit's spawn spots in `lb` not gone round yet, the one stood at
    /// counted; `None` once the round is done, or with none known.
    pub(super) fn next_spawn_spot(&mut self, lb: u32) -> Option<Place> {
        let me = self.my_position()?;
        let spots = self.spawn_spots(lb);
        let walked = &mut self.autoplay.growth.spawns_walked;
        walked.retain(|c| *c >> 16 == lb);
        for r in spots
            .iter()
            .filter(|r| r.at.distance(me) <= super::hunt::ROOM_HELD)
        {
            if !walked.contains(&r.cell) {
                walked.push(r.cell);
            }
        }
        spots
            .into_iter()
            .filter(|r| !walked.contains(&r.cell))
            .min_by(|a, b| a.at.distance(me).total_cmp(&b.at.distance(me)))
    }

    /// Start the round of `lb`'s spawn spots again, the one stood at counted as gone round.
    pub(super) fn start_spawn_round(&mut self, lb: u32) {
        let Some(me) = self.my_position() else {
            return;
        };
        let near: Vec<u32> = self
            .spawn_spots(lb)
            .iter()
            .filter(|r| r.at.distance(me) <= super::hunt::ROOM_HELD)
            .map(|r| r.cell)
            .collect();
        self.autoplay
            .growth
            .spawns_walked
            .retain(|c| near.contains(c));
    }

    /// How near the ground on `lb` a journey there ends: in a pursued creature's own room, else
    /// within [`GROUND_REACH`].
    pub(super) fn ground_reach(&self, lb: u32) -> f32 {
        match self.pursued_place(lb) {
            Some(p) if p.is_indoors() => super::hunt::ROOM_REACH,
            _ => GROUND_REACH,
        }
    }
}

#[cfg(test)]
mod tests;
