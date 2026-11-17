//! The fastest way to come by what a pursuit wants (`obtain`): every way on one scale, seconds for
//! all `more` still wanted with the journey there -- each fetched source, each hunting ground, a
//! buy, a hand-in chain [`OBTAIN_DEPTH`] deep, a locked chest's key had first -- and the least
//! taken. A hand-in whose item is carried is made outright. What no way reaches is had as before:
//! another quest's reward, a key, an event first, else why not.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use ac_world::quests::{Handing, Kind, Repeat, Row, Source, Want, Yield};
use ac_world::Place;

use super::fetch::{ware_charge, Fetched};
use super::flag_way::FLAG_SECONDS;
use super::{stuck, Ask, Hunting, PursuitStep, OBTAIN_DEPTH};

/// Seconds one try at a fetched source takes once there: the walk up, the act, its answer (guess).
const TRY_SECS: f32 = 20.0;
/// Seconds a chest takes to fill again once opened, with no ResetInterval of its own (`Chest.cs:46-59`).
const CHEST_RESET_SECS: f32 = 120.0;
/// Seconds a hand-in takes at its giver: the give and its answer (guess).
const HAND_SECS: f32 = 10.0;
/// A way no journey reaches, or behind a task the data does not follow, reckoned as a portal
/// past a flag not held is: a day.
const NO_WAY_SECS: f32 = FLAG_SECONDS;
/// Most journeys planned a board: each a search over every portal, 30-100 ms in a test build.
const JOURNEY_PLANS: u32 = 4;
/// A journey planned is read this long, then planned again from where the body is (as `ONWARD_KEEP`).
const JOURNEY_KEEP: Duration = Duration::from_secs(600);

/// A journey as far as it is planned (`Ask::journey`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Leg {
    /// Planned: its seconds, `None` when nothing reaches the place.
    Planned(Option<f32>),
    /// Not planned yet: taken as nothing, a bound, till a way it is on is chosen.
    Unplanned,
}

/// One way to come by what is wanted: its step now, a hunt's grounds, seconds for all of it, and
/// the places on it whose journey is not planned yet (counted as nothing).
#[derive(Clone, Debug)]
struct Way {
    got: (PursuitStep, Option<Hunting>),
    secs: f32,
    unplanned: Vec<Place>,
}

/// The fastest ways found, least first; the step when there is none: why not, or a wait.
#[derive(Clone, Debug)]
struct Ranked {
    ways: Vec<Way>,
    none: (PursuitStep, Option<Hunting>),
}

/// What one ranking keeps from working out twice, by wants, count and name: what was found
/// under the bound it was asked with.
type Memo = RefCell<BTreeMap<(Vec<Want>, u32, String), (f32, Ranked)>>;

/// The way a pursuit's step was chosen by and the next best, for the log (`Pursued::choices`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Choice {
    pub(crate) more: u32,
    /// The way in words, its counts left out, and its seconds.
    pub(crate) best: (String, f32),
    pub(crate) next: Option<(String, f32)>,
}

impl Choice {
    /// What was chosen over what, seconds aside: the log says it again only once this changes.
    pub(crate) fn key(&self) -> String {
        let next = self.next.as_ref().map_or("", |n| n.0.as_str());
        format!("{} / {next}", self.best.0)
    }

    /// "fastest way hunt X in Y (~12 min for 10), next buy Z from W (~30 min)".
    pub(crate) fn words(&self) -> String {
        let mins = |s: f32| (s / 60.0).round() as u64;
        let next = self.next.as_ref().map_or("nothing".to_string(), |(w, s)| {
            format!("{w} (~{} min)", mins(*s))
        });
        let (way, secs) = &self.best;
        format!(
            "fastest way {way} (~{} min for {}), next {next}",
            mins(*secs),
            self.more
        )
    }
}

/// Journey seconds to the places of the ways chosen, planned from where the body stood, at most
/// [`JOURNEY_PLANS`] a board: what the ranking reads stays put while the body walks, so the
/// choice does not turn round on the road.
#[derive(Debug, Default)]
pub(crate) struct Journeys {
    secs: BTreeMap<(u32, [i32; 3]), (Option<f32>, Instant)>,
    /// Plans made this board, and those asked past [`JOURNEY_PLANS`].
    planned: u32,
    wanted: u32,
}

impl Journeys {
    /// Ready for a board worked out at `now`: one planned past [`JOURNEY_KEEP`] is forgotten.
    pub(crate) fn start(&mut self, now: Instant) {
        (self.secs).retain(|_, (_, at)| now.saturating_duration_since(*at) < JOURNEY_KEEP);
        self.planned = 0;
        self.wanted = 0;
    }

    /// Whether the last board left a chosen way's journey unplanned: the choice may change yet.
    pub(crate) fn is_planning(&self) -> bool {
        self.wanted > 0
    }

    /// The journey to `p` as planned; with `plan`, planned now by `plan` unless this board's
    /// plans are made.
    pub(crate) fn leg(
        &mut self,
        p: Place,
        plan: bool,
        now: Instant,
        planner: impl FnOnce() -> Option<f32>,
    ) -> Leg {
        // Ten metres: one generator's scatter.
        let key = (p.cell, p.at.to_array().map(|v| (v / 10.0).round() as i32));
        if let Some((secs, _)) = self.secs.get(&key) {
            return Leg::Planned(*secs);
        }
        if !plan {
            return Leg::Unplanned;
        }
        if self.planned >= JOURNEY_PLANS {
            self.wanted += 1;
            return Leg::Unplanned;
        }
        self.planned += 1;
        let secs = planner();
        self.secs.insert(key, (secs, now));
        Leg::Planned(secs)
    }
}

/// A way not yet worked out, and the least it can take.
struct Lead<'f> {
    least: f32,
    kind: LeadKind<'f>,
}

enum LeadKind<'f> {
    /// A fetched source with nothing to have first, `there` seconds at it.
    Fetch { f: &'f Fetched, there: f32 },
    /// A fetched source whose key or item is had first.
    Keyed { f: &'f Fetched, there: f32 },
    /// The hunt `ground_step` chose.
    Hunt,
    /// A hand-in for it, `need` of its item still to come by: `hands` of them, waiting `waits`
    /// between and `wait` before the first.
    Hand {
        row: &'static Row,
        at: Place,
        need: u32,
        hands: f32,
        waits: f32,
        wait: f32,
    },
}

/// A hand-in giving what is wanted: where it is made and, unless what it takes is carried, how
/// many of that are still to come by and how many hand-ins it takes.
type HandIn = (&'static Row, Place, Option<(u32, f32)>);

impl Ask<'_> {
    /// `wants`, `more` of them, by the fastest way ([`OBTAIN_DEPTH`] deep); else why not. At
    /// `depth` 0 the journeys of the ways chosen are planned, and it is ranked again until the
    /// best's are all planned, and the choice is kept for the log (`chosen`).
    pub(super) fn obtain(
        &self,
        wants: &[Want],
        more: u32,
        what: &str,
        depth: usize,
    ) -> (PursuitStep, Option<Hunting>) {
        let mut ranked = self.rank(&Memo::default(), wants, more, what, depth, f32::INFINITY, 2);
        // Each round plans at least one journey, and a board plans only so many.
        for _ in 0..JOURNEY_PLANS * 2 {
            let todo = ranked.ways.iter().flat_map(|w| &w.unplanned);
            let todo: Vec<Place> = todo.copied().collect();
            if depth > 0 || todo.is_empty() {
                break;
            }
            let planned = todo
                .iter()
                .filter(|p| (self.journey)(**p, true) != Leg::Unplanned);
            if planned.count() == 0 {
                break;
            }
            ranked = self.rank(&Memo::default(), wants, more, what, depth, f32::INFINITY, 2);
        }
        if depth == 0 && !ranked.ways.is_empty() {
            let label = |w: &Way| (way_label(&w.got.0), w.secs);
            *self.chosen.borrow_mut() = Some(Choice {
                more,
                best: label(&ranked.ways[0]),
                next: ranked.ways.get(1).map(label),
            });
        }
        match ranked.ways.into_iter().next() {
            Some(w) => w.got,
            None => ranked.none,
        }
    }

    /// [`Self::rank`]'s best under `bound`, kept in `memo`; a way round back to what is being
    /// worked out is none.
    #[allow(clippy::too_many_arguments)] // the memo and bound thread one search's recursion
    fn ranked(
        &self,
        memo: &Memo,
        wants: &[Want],
        more: u32,
        what: &str,
        depth: usize,
        bound: f32,
    ) -> Ranked {
        let key = (wants.to_vec(), more, what.to_string());
        if let Some((under, r)) = memo.borrow().get(&key) {
            match r.ways.first() {
                Some(w) if w.secs < bound => return r.clone(),
                Some(_) => {
                    let none = (stuck(format!("{what} comes no sooner")), None);
                    return Ranked {
                        ways: Vec::new(),
                        none,
                    };
                }
                None if *under >= bound => return r.clone(),
                None => {}
            }
        }
        let round = Ranked {
            ways: Vec::new(),
            none: (stuck(format!("{what} comes only by itself")), None),
        };
        memo.borrow_mut()
            .insert(key.clone(), (f32::INFINITY, round));
        let r = self.rank(memo, wants, more, what, depth, bound, 1);
        memo.borrow_mut().insert(key, (bound, r.clone()));
        r
    }

    /// Every way to `more` of `wants`, least seconds first, the `keep` best under `bound`: a way
    /// whose least is past the `keep`th found is not worked out.
    #[allow(clippy::too_many_arguments)] // as `ranked`
    fn rank(
        &self,
        memo: &Memo,
        wants: &[Want],
        more: u32,
        what: &str,
        depth: usize,
        bound: f32,
        keep: usize,
    ) -> Ranked {
        // An item whose pickup stamp waits is not gone for: the server turns the pickup down (the
        // Ancient Key hunted with novquest1 2.6 h off). With every one so, the quest waits. One
        // carried is had: its stamp bars only another (the Sacrificial Dagger held for the Ghost).
        let carried = |i: &u32| self.facts.carried.get(i).is_some_and(|n| *n > 0);
        let waits = |w: &Want| match w {
            Want::Item(i) if !carried(i) => self.facts.log.pickup_wait(*i, self.facts.now),
            _ => None,
        };
        let open: Vec<Want> = wants
            .iter()
            .filter(|w| waits(w).is_none())
            .cloned()
            .collect();
        if open.is_empty() {
            if let Some(until) = wants.iter().filter_map(waits).min() {
                let none = (PursuitStep::Waiting { until }, None);
                return Ranked {
                    ways: Vec::new(),
                    none,
                };
            }
        }
        let wants = &open[..];
        let mut events: Vec<&'static str> = Vec::new();
        let fetched = self.fetch_ways(wants, more, &mut events);
        let hunted = self.ground_step(wants, more, what, &mut events, depth);
        let hands = self.hand_ins(wants, more);
        let done = |ways: Vec<Way>| Ranked {
            ways,
            none: (stuck(format!("no way to {what}")), None),
        };
        // A hand-in of what is carried is made outright, the nearest.
        let mut held: Vec<Way> = (hands.iter())
            .filter(|h| h.2.is_none())
            .map(|&(row, at, _)| {
                let mut w = self.way_to(at, (PursuitStep::HandIn { row, at }, None), 0.0);
                w.secs += HAND_SECS + self.timer_wait(row);
                w
            })
            .collect();
        if !held.is_empty() {
            held.sort_by(|a, b| a.secs.total_cmp(&b.secs));
            held.truncate(keep);
            return done(held);
        }
        let mut leads: Vec<Lead> = Vec::new();
        // A buy is the town run's, which picks its own counter: one lead, the nearest's.
        let away = |f: &Fetched| match (f.at, self.facts.me) {
            (Some(at), Some(me)) => at.at.distance(me),
            _ => 0.0,
        };
        let shop = |f: &Fetched| f.source.kind == Yield::Vendor;
        let nearest_shop = (fetched.iter().enumerate())
            .filter(|(_, f)| shop(f))
            .min_by(|a, b| away(a.1).total_cmp(&away(b.1)))
            .map(|(i, _)| i);
        for (i, f) in fetched.iter().enumerate() {
            if shop(f) && nearest_shop != Some(i) {
                continue;
            }
            let there = self.there_secs(f, more);
            let kind = match (&f.step, depth < OBTAIN_DEPTH) {
                (Some(_), _) => LeadKind::Fetch { f, there },
                (None, true) => LeadKind::Keyed { f, there },
                (None, false) => continue,
            };
            leads.push(Lead { least: there, kind });
        }
        if is_usable(&hunted.0) {
            let least = hunt_least(&hunted, wants, more);
            leads.push(Lead {
                least,
                kind: LeadKind::Hunt,
            });
        }
        for &(row, at, need) in hands.iter().filter(|_| depth < OBTAIN_DEPTH) {
            let Some((need, hands)) = need else { continue };
            let waits = (hands - 1.0).max(0.0) * repeat_secs(row);
            let wait = self.timer_wait(row);
            leads.push(Lead {
                least: waits + wait + hands * HAND_SECS,
                kind: LeadKind::Hand {
                    row,
                    at,
                    need,
                    hands,
                    waits,
                    wait,
                },
            });
        }
        leads.sort_by(|a, b| a.least.total_cmp(&b.least));
        let mut ways: Vec<Way> = Vec::new();
        for lead in leads {
            let cut = ways.get(keep - 1).map_or(bound, |w| w.secs.min(bound));
            if lead.least >= cut {
                break;
            }
            let way = match lead.kind {
                LeadKind::Fetch { f, there } => (f.step.clone()).map(|step| match f.at {
                    Some(at) => self.way_to(at, (step, None), there),
                    None => Way {
                        got: (step, None),
                        secs: there,
                        unplanned: Vec::new(),
                    },
                }),
                LeadKind::Keyed { f, there } => self.keyed_way(memo, f, there, more, depth, cut),
                LeadKind::Hunt => self.hunt_way(&hunted, wants, more, cut),
                LeadKind::Hand {
                    row,
                    at,
                    need,
                    hands,
                    waits,
                    wait,
                } => {
                    let name = row.item.as_ref().map_or("", |i| i.name);
                    let over = wait + hands * HAND_SECS;
                    let want = std::slice::from_ref(&row.want);
                    let sub = self.ranked(memo, want, need, name, depth + 1, cut - over);
                    sub.ways.into_iter().next().map(|w| {
                        let mut way = self.way_to(at, w.got, w.secs.max(waits) + over);
                        way.unplanned.extend(w.unplanned);
                        way
                    })
                }
            };
            // One step by two hand-ins is one way: the faster stands.
            let Some(w) = way.filter(|w| w.secs < cut) else {
                continue;
            };
            if let Some(i) = ways.iter().position(|o| o.got.0 == w.got.0) {
                if ways[i].secs <= w.secs {
                    continue;
                }
                ways.remove(i);
            }
            let i = ways.partition_point(|o| o.secs <= w.secs);
            ways.insert(i, w);
            ways.truncate(keep);
        }
        if !ways.is_empty() {
            return done(ways);
        }
        // Past the bound: some way is better, and why not here is never read.
        if bound.is_finite() {
            let none = (stuck(format!("{what} comes no sooner")), None);
            return Ranked { ways, none };
        }
        let none = self.no_way(memo, (&fetched, hunted, &hands, events), what, depth);
        Ranked { ways, none }
    }

    /// The step when no way to `what` was found: another quest's reward, then a chest's key or an
    /// item used to make it, then an event, each had first; else why not.
    #[allow(clippy::type_complexity)] // what `rank` worked out, handed on whole
    fn no_way(
        &self,
        memo: &Memo,
        (fetched, hunted, hands, events): (
            &[Fetched],
            (PursuitStep, Option<Hunting>),
            &[HandIn],
            Vec<&'static str>,
        ),
        what: &str,
        depth: usize,
    ) -> (PursuitStep, Option<Hunting>) {
        if !matches!(hunted.0, PursuitStep::Stuck { .. }) || depth >= OBTAIN_DEPTH {
            return hunted;
        }
        let first = |r: Ranked| r.ways.into_iter().next().map_or(r.none, |w| w.got);
        let is_stuck =
            |got: &(PursuitStep, Option<Hunting>)| matches!(got.0, PursuitStep::Stuck { .. });
        // Another quest's reward: quests chain (a Singularity Scepter handed in for the Bound one).
        let mut first_stuck = None;
        for (row, _, need) in hands {
            let Some((need, _)) = need else { continue };
            let name = row.item.as_ref().map_or("", |i| i.name);
            let want = std::slice::from_ref(&row.want);
            let got = first(self.ranked(memo, want, *need, name, depth + 1, f32::INFINITY));
            if !is_stuck(&got) {
                return got;
            }
            first_stuck.get_or_insert(got);
        }
        if let Some(got) = first_stuck {
            return got;
        }
        let mut first_why = None;
        let mut keys: Vec<(u32, &'static str)> = Vec::new();
        for k in fetched
            .iter()
            .filter(|f| f.step.is_none())
            .flat_map(|f| &f.first)
        {
            if !keys.contains(k) {
                keys.push(*k);
            }
        }
        for (key, name) in keys {
            let want = [Want::Item(key)];
            let got = first(self.ranked(memo, &want, 1, name, depth + 1, f32::INFINITY));
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
            let want = [Want::Event(e.to_string())];
            let got = first(self.ranked(memo, &want, 1, &name, depth + 1, f32::INFINITY));
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

    /// The way `got` at `at`: the journey there and `secs` more. One no journey reaches is
    /// [`NO_WAY_SECS`] (what opens a way is had first); one not planned yet, nothing for now.
    fn way_to(&self, at: Place, got: (PursuitStep, Option<Hunting>), secs: f32) -> Way {
        let (journey, unplanned) = match (self.journey)(at, false) {
            Leg::Planned(s) => (s.unwrap_or(NO_WAY_SECS), Vec::new()),
            Leg::Unplanned => (0.0, vec![at]),
        };
        Way {
            got,
            secs: journey + secs,
            unplanned,
        }
    }

    /// Seconds before `row` may be done again, by the quest log.
    fn timer_wait(&self, row: &Row) -> f32 {
        if row.timer.is_empty() {
            return 0.0;
        }
        let now = self.facts.now;
        let until = self.facts.log.wait_until(row.timer, now);
        until.map_or(0.0, |t| t.saturating_sub(now) as f32)
    }

    /// The hand-ins giving any of `wants`, for `more` of them (`HandIn`).
    fn hand_ins(&self, wants: &[Want], more: u32) -> Vec<HandIn> {
        let level = self.facts.level;
        let mut out = Vec::new();
        for item in wants.iter().filter_map(|w| match w {
            Want::Item(i) => Some(*i),
            _ => None,
        }) {
            // A swap of an item for one like it (a Glowing Virindi Cloak for a Glowing Virindi
            // Cloak) is no way to come by it.
            let gives = rows_giving(item).iter().filter(|r| {
                r.kind == Kind::Item
                    && r.hand == Handing::Give
                    && r.want != Want::Item(item)
                    && r.fits_level(level)
            });
            for &r in gives {
                let Want::Item(input) = r.want else {
                    continue;
                };
                let Some(at) = self.open_place(r) else {
                    continue;
                };
                let count = r.count.max(1);
                let have = (self.asking.carried)(input);
                let per = yield_of(r, item);
                if have >= count {
                    out.push((r, at, None));
                } else if per > 0.0 {
                    let hands = (more as f32 / per).ceil().max(1.0);
                    let need = (hands as u32 * count).saturating_sub(have).max(1);
                    out.push((r, at, Some((need, hands))));
                }
            }
        }
        out
    }

    /// Seconds at a fetched source for `more`: a try a fill, `more` over its chance of them, each
    /// after the last as it fills again (a chest's reset, a floor item's respawn, an NPC's wait).
    fn there_secs(&self, f: &Fetched, more: u32) -> f32 {
        let s = f.source;
        let item = match s.want {
            Want::Item(w) => w,
            _ => 0,
        };
        let (chance, again, task) = match s.kind {
            // Bought at one counter, all at once, coin allowing: short of it, the coin is had
            // first, reckoned as a flag not held is (200 trade notes for a gamble's Poison).
            Yield::Vendor => {
                let bill = ware_charge(s, item).map_or(0, |c| c.saturating_mul(more));
                let short = bill > self.spendable;
                return TRY_SECS + if short { NO_WAY_SECS } else { 0.0 };
            }
            Yield::Container | Yield::Floor => {
                let reset = s.respawn.unwrap_or(CHEST_RESET_SECS);
                (s.chance, reset, 0.0)
            }
            // An NPC's talk rows say how often it gives, and after what task (Aun Teriona's
            // livers): one only a task not done opens is reckoned as a flag not held.
            Yield::Npc => match self.talk_terms(s, item) {
                Some((chance, again, open)) => {
                    let task = if open { 0.0 } else { NO_WAY_SECS };
                    (chance, again, task)
                }
                None => (s.chance, TRY_SECS, 0.0),
            },
            _ => (s.chance, TRY_SECS, 0.0),
        };
        let tries = (more.max(1) as f32 / chance.clamp(0.001, 1.0)).max(1.0);
        task + TRY_SECS + (tries - 1.0) * again.max(TRY_SECS)
    }

    /// An NPC source's talk giving `item`: its chance a talk, seconds before it talks so again,
    /// and whether its needs are held; the best talk; `None` when no talk row is known.
    fn talk_terms(&self, s: &Source, item: u32) -> Option<(f32, f32, bool)> {
        let holds = |f: &String| crate::quest_log::holds(self.facts, f);
        let terms = |r: &&&Row| {
            (
                yield_of(r, item).min(1.0),
                repeat_secs(r),
                r.needs.iter().all(holds),
            )
        };
        rows_giving(item)
            .iter()
            .filter(|r| r.giver_wcid == s.wcid && r.hand == Handing::Talk)
            .map(|r| terms(&r))
            .filter(|t| t.0 > 0.0)
            .max_by(|a, b| {
                (a.2, a.0)
                    .partial_cmp(&(b.2, b.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// A fetched source whose key or item is had first: the fastest of its keys (each, for a
    /// combine) under `cut`, then the journey there and `there` seconds at it. A recipe uses its
    /// items up, one a make of `more`; a lock's key opens it again.
    #[allow(clippy::too_many_arguments)] // as `ranked`
    fn keyed_way(
        &self,
        memo: &Memo,
        f: &Fetched,
        there: f32,
        more: u32,
        depth: usize,
        cut: f32,
    ) -> Option<Way> {
        let budget = cut - there;
        let used_up = matches!(f.source.kind, Yield::Combine | Yield::Use);
        let n = if used_up { more.max(1) } else { 1 };
        let mut key_ways: Vec<Way> = Vec::new();
        for (key, name) in &f.first {
            let want = [Want::Item(*key)];
            let sub = self.ranked(memo, &want, n, name, depth + 1, budget);
            match sub.ways.into_iter().next() {
                Some(w) => key_ways.push(w),
                // A combine needs each: one with no way is no way.
                None if f.each => return None,
                None => {}
            }
        }
        let keys = if f.each {
            let got = key_ways.first()?.got.clone();
            let secs = key_ways.iter().map(|w| w.secs).sum();
            let unplanned = key_ways.into_iter().flat_map(|w| w.unplanned).collect();
            Way {
                got,
                secs,
                unplanned,
            }
        } else {
            key_ways
                .into_iter()
                .min_by(|a, b| a.secs.total_cmp(&b.secs))?
        };
        let mut way = match f.at {
            Some(at) => self.way_to(at, keys.got, keys.secs + there),
            None => Way {
                got: keys.got,
                secs: keys.secs + there,
                unplanned: Vec::new(),
            },
        };
        way.unplanned.extend(keys.unplanned);
        Some(way)
    }

    /// The hunt `ground_step` chose: its soonest ground, the journey there and `more` at its
    /// pace, the grounds tried soonest-paced first while one could beat `cut`.
    fn hunt_way(
        &self,
        hunted: &(PursuitStep, Option<Hunting>),
        wants: &[Want],
        more: u32,
        cut: f32,
    ) -> Option<Way> {
        let Some(h) = hunted.1.as_ref().filter(|h| hunts_wants(h, wants)) else {
            return Some(Way {
                got: hunted.clone(),
                secs: NO_WAY_SECS,
                unplanned: Vec::new(),
            });
        };
        let mut best: Option<Way> = None;
        for ((_, at), g) in h.grounds.iter().zip(&h.paces) {
            let there = g.pace.soonest() * more as f32;
            if there >= best.as_ref().map_or(cut, |b| b.secs.min(cut)) {
                break;
            }
            let way = self.way_to(*at, hunted.clone(), there);
            if best.as_ref().is_none_or(|b| way.secs < b.secs) {
                best = Some(way);
            }
        }
        best.filter(|b| b.secs < cut)
    }
}

/// Whether `h` hunts one of `wants` (not a flag's creature hunted first).
fn hunts_wants(h: &Hunting, wants: &[Want]) -> bool {
    wants
        .iter()
        .any(|w| matches!(w, Want::Item(i) if h.items.contains(i)))
}

/// The least a hunt takes: its soonest ground's pace for `more`, or a flag first's reckoning.
fn hunt_least(hunted: &(PursuitStep, Option<Hunting>), wants: &[Want], more: u32) -> f32 {
    match hunted.1.as_ref().filter(|h| hunts_wants(h, wants)) {
        Some(h) => h
            .paces
            .first()
            .map_or(f32::INFINITY, |g| g.pace.soonest() * more as f32),
        None => NO_WAY_SECS,
    }
}

/// Whether a step is a way: one that can be worked now.
fn is_usable(step: &PursuitStep) -> bool {
    !matches!(
        step,
        PursuitStep::Stuck { .. } | PursuitStep::Waiting { .. } | PursuitStep::Finished
    )
}

/// A step in words, its counts left out: what the log compares.
fn way_label(step: &PursuitStep) -> String {
    match step {
        PursuitStep::Hunt {
            creature, place, ..
        } => format!("hunt {creature} in {place}"),
        PursuitStep::Buy { name, vendor, .. } => format!("buy {name} from {vendor}"),
        other => other.words(0),
    }
}

/// The rows whose reward gives `item`, in any alternative, in the data's order.
fn rows_giving(item: u32) -> &'static [&'static Row] {
    static BY_ITEM: OnceLock<BTreeMap<u32, Vec<&'static Row>>> = OnceLock::new();
    let by = BY_ITEM.get_or_init(|| {
        let mut m: BTreeMap<u32, Vec<&'static Row>> = BTreeMap::new();
        for r in ac_world::quests::all() {
            let mut gives: Vec<u32> = r.gives.iter().map(|g| g.0).collect();
            gives.sort_unstable();
            gives.dedup();
            for g in gives {
                m.entry(g).or_default().push(r);
            }
        }
        m
    });
    by.get(&item).map_or(&[], Vec::as_slice)
}

/// How many of `item` one solve of `r` gives, a random reply's alternatives taken as alike.
fn yield_of(r: &Row, item: u32) -> f32 {
    let of = |alt: &str| -> f32 {
        alt.split(';')
            .filter_map(|g| g.split_once(':'))
            .filter(|(w, _)| w.trim().parse() == Ok(item))
            .map(|(_, n)| n.trim().parse::<f32>().unwrap_or(1.0))
            .sum()
    };
    if r.gives_column.is_empty() {
        let n: u32 = r.gives.iter().filter(|g| g.0 == item).map(|g| g.1).sum();
        return n as f32;
    }
    let alts: Vec<&str> = r.gives_column.split('|').collect();
    alts.iter().map(|a| of(a)).sum::<f32>() / alts.len() as f32
}

/// Seconds after a solve before `r` may be solved again; a once-only row's next never comes,
/// reckoned as [`NO_WAY_SECS`].
fn repeat_secs(r: &Row) -> f32 {
    match r.repeat {
        Repeat::Any => 0.0,
        Repeat::After(s) => s as f32,
        Repeat::Once => NO_WAY_SECS,
    }
}
