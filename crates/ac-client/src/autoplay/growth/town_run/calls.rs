//! A town run's calls: its counter and the quest givers with a hand-in ready, put in one order
//! when the run sets off (`ac_world::tour`), each leg priced by the journey planner. A giver joins
//! when it stands in a town the run calls at or a short leg from one, or its XP beats the detour
//! at the character's XP per hour; else its items wait, kept, for a run that passes (design §5).

use std::time::{Duration, Instant};

use ac_world::quests::{Handing, Kind, Repeat, Row, Want};
use ac_world::tour::{Legs, Stop, EVERY_ORDER};
use ac_world::Place;
use glam::Vec2;

use super::vendor::counter_place;
use super::{Errand, Phase, Run, Turn, VENDOR_REACH};
use crate::autoplay::growth::Growth;
use crate::autoplay::Doing;
use crate::quest_log::{worth_detour, QuestLine, QuestState, SHORT_LEG};
use crate::Client;

/// A giver no journey reached is planned to again after this (a Warden's wait, hunt.rs).
pub(crate) const GIVER_RETRY: Duration = Duration::from_secs(30 * 60);
/// A talk to a giver for a quest is not made again for this long, whatever the board says
/// (guess): a board wrong about it would otherwise call on the giver every tick.
pub(crate) const TALK_AGAIN: Duration = Duration::from_secs(5 * 60);
/// A give not answered in this long counts as unanswered (design §5.1, guess).
const GIVE_PAUSE: Duration = Duration::from_secs(3);
/// Unanswered gives before the call ends: ACE turns a give away while the giver emotes
/// (Player_Inventory.cs:3422-3427), and the visit's own use sets one off.
const GIVE_STALLS: u32 = 3;
/// Visits begun to one giver on one call before it is given up.
const GIVER_TRIES: u32 = 3;
/// A giver used for a talk is waited on this long for a line of its own (guess): left at once,
/// the journey on cancelled ACE's walk to it and Shoichi never spoke.
const TALK_WAIT: Duration = Duration::from_secs(5);
/// A giver this near (metres, 3D) is in reach of a give: the visit's talking reach, with slack.
const GIVER_REACH: f32 = crate::visit::REACH + 1.0;

/// One stop of a town run.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Call {
    pub(super) at: Place,
    pub(super) on: CallOn,
}

/// What a call is for.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CallOn {
    /// A counter, for what the run was made for.
    Counter { errand: Errand, vendor: String },
    /// A quest giver, for a hand-in ready.
    Giver { row: &'static Row },
}

/// A ready hand-in a run may take: the giver's row, the placement called at, and the XP of
/// every hand-in carried.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HandIn {
    pub(super) row: &'static Row,
    pub(super) at: Place,
    pub(super) xp: u64,
    /// For a pursued quest: taken on any run, whatever the detour.
    pub(super) pursued: bool,
}

/// What a giver's placement is chosen on, as one character stands.
pub(crate) struct Asking<'a> {
    pub(crate) level: u32,
    /// Quest flags held: a door shut behind one not held keeps a giver out of reach.
    pub(crate) held: &'a [String],
    /// Items carried, by weenie class, worn ones left out.
    pub(crate) carried: &'a dyn Fn(u32) -> u32,
    /// The giver at that placement was refused lately.
    pub(crate) refused: &'a dyn Fn(&Row, &Place) -> bool,
    /// How far a placement is, by the caller's measure: the nearest is asked.
    pub(crate) away: &'a dyn Fn(&Place) -> f32,
}

impl Asking<'_> {
    /// `row`'s nearest placement it can be asked at: an open door, not lately refused.
    fn open_at(&self, row: &'static Row) -> Option<(f32, &'static Row, Place)> {
        row.places
            .iter()
            .filter(|p| ac_world::quests::locked_at(p, self.held).is_none())
            .filter(|p| !(self.refused)(row, p))
            .map(|p| ((self.away)(p), row, *p))
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }

    /// The nearest talk with `from` in the level band stamping `flag`, and how far: matched by
    /// the flag too, as givers share names.
    pub(crate) fn take_up_at(&self, from: &str, flag: &str) -> Option<(f32, &'static Row, Place)> {
        let stamps = |r: &Row| r.stamps.iter().any(|s| s.eq_ignore_ascii_case(flag));
        ac_world::quests::all()
            .iter()
            .filter(|r| r.kind == Kind::Start && r.giver == from && stamps(r))
            .filter(|r| r.fits_level(self.level))
            .filter_map(|r| self.open_at(r))
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }

    /// The nearest way to hand `line`'s quest in now, its items carried, and how far.
    pub(crate) fn hand_in_at(&self, line: &QuestLine) -> Option<(f32, &'static Row, Place)> {
        crate::quest_log::worked_rows(line)
            .filter(|r| r.kind == line.kind && r.fits_level(self.level))
            .filter(|r| match r.want {
                Want::Item(w) => (self.carried)(w) >= r.count.max(1),
                _ => true,
            })
            .filter_map(|r| self.open_at(r))
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }
}

/// What planning a run's calls came to.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct CallPlan {
    /// Calls in the order made: the counters' indices, then each hand-in's at counters + its own.
    pub(super) order: Vec<usize>,
    /// Hand-ins no journey from the start or a counter reaches, by index.
    pub(super) refused: Vec<usize>,
}

/// Which hand-ins join a run calling at `counters` (the legs' first calls), and every call's order:
/// pursued then best XP a second of detour first, each tried on the tour, to [`EVERY_ORDER`] calls.
pub(crate) fn plan_calls(
    legs: &Legs,
    counters: &[Place],
    hand_ins: &[HandIn],
    xp_per_hour: Option<f32>,
) -> CallPlan {
    let n = counters.len();
    let leg = |a: Stop, b: Stop| legs.leg(a, b);
    let reached = |g: usize| {
        leg(Stop::Start, Stop::Call(g)).is_some()
            || (0..n).any(|c| leg(Stop::Call(c), Stop::Call(g)).is_some())
    };
    let short = |s: Option<f32>| s.is_some_and(|s| s <= SHORT_LEG);
    // In a counter's town or a short leg out from one, and no more than there and back out of
    // the way: a cheap recall home makes no giver near.
    let on_the_round = |g: usize, detour: f32| {
        let block = hand_ins[g - n].at.cell >> 16;
        detour <= 2.0 * SHORT_LEG
            && counters
                .iter()
                .enumerate()
                .any(|(c, p)| p.cell >> 16 == block || short(leg(Stop::Call(c), Stop::Call(g))))
    };
    let secs = |calls: &[usize]| legs.tour(calls).map(|(_, s)| s);
    let mut taken: Vec<usize> = (0..n).collect();
    let mut base = secs(&taken);
    let mut refused = Vec::new();
    let mut tried: Vec<(usize, f32)> = Vec::new();
    for (k, h) in hand_ins.iter().enumerate() {
        let g = n + k;
        if !reached(g) {
            refused.push(k);
            continue;
        }
        let with = [taken.as_slice(), &[g]].concat();
        let Some(w) = secs(&with) else {
            continue;
        };
        let detour = base.map_or(w, |b| (w - b).max(0.0));
        tried.push((g, h.xp as f32 / detour.max(1.0)));
    }
    // A pursued quest's hand-in goes first, whatever it pays: the player asked for it.
    let pursued = |g: usize| hand_ins[g - n].pursued;
    tried.sort_by(|a, b| pursued(b.0).cmp(&pursued(a.0)).then(b.1.total_cmp(&a.1)));
    for (g, _) in tried {
        if taken.len() >= EVERY_ORDER {
            break;
        }
        let with = [taken.as_slice(), &[g]].concat();
        let Some(w) = secs(&with) else {
            continue;
        };
        let detour = base.map_or(w, |b| (w - b).max(0.0));
        let worth = worth_detour(hand_ins[g - n].xp, detour, xp_per_hour);
        if pursued(g) || on_the_round(g, detour) || worth {
            taken = with;
            base = Some(w);
        }
    }
    let order = legs.tour(&taken).map_or(taken, |(o, _)| o);
    CallPlan { order, refused }
}

impl Client {
    /// Whether `row`'s giver at `p` is let be: refused or unreached lately, or, for a hand-in by
    /// talk, that talk made lately.
    pub(crate) fn is_giver_refused(&self, row: &Row, p: &Place, now: Instant) -> bool {
        let st = &self.autoplay.growth;
        st.givers_refused
            .within(&(row.giver_wcid, p.cell), now, GIVER_RETRY)
            || (row.hand == Handing::Talk && st.talked.within(&talk_key(row), now, TALK_AGAIN))
    }

    /// The hand-ins ready on the board, and the talks that take a switched-on quest up, each at the
    /// giver nearest the first of `near` it can be asked at (level band, open door, not lately
    /// refused), nearest first, at most `most`.
    pub(super) fn ready_hand_ins(&self, near: &[Vec2], most: usize, now: Instant) -> Vec<HandIn> {
        let carried = |w| self.carried_count(w);
        let refused = |r: &Row, p: &Place| self.is_giver_refused(r, p, now);
        let away = |p: &Place| {
            near.iter()
                .map(|n| n.distance(p.at.truncate()))
                .fold(f32::MAX, f32::min)
        };
        let asking = Asking {
            level: self.world.stats.level.max(0) as u32,
            held: &self.travel.quests_done,
            carried: &carried,
            refused: &refused,
            away: &away,
        };
        let pursued = |id: &str| self.pursuit.iter().any(|l| l.id == id);
        let mut out: Vec<(f32, HandIn)> = Vec::new();
        for line in &self.quest_board {
            // Not taken up: a talk to whoever starts it, the quest's own XP its worth.
            if let (true, QuestState::NotTaken { from, flag }) = (line.on, &line.state) {
                let start = asking.take_up_at(from, flag);
                if let Some((d, row, at)) =
                    start.filter(|(_, r, _)| !out.iter().any(|(_, h)| std::ptr::eq(h.row, *r)))
                {
                    let hand_in = HandIn {
                        row,
                        at,
                        xp: line.xp,
                        pursued: pursued(&line.id),
                    };
                    out.push((d, hand_in));
                }
                continue;
            }
            if !line.on || line.state != QuestState::Ready || line.kind == Kind::Start {
                continue;
            }
            let Some((d, row, at)) = asking.hand_in_at(line) else {
                continue;
            };
            // A hand-in that repeats any time pays for each one carried (the Collectors).
            let times = match (&row.want, row.repeat) {
                (Want::Item(wcid), Repeat::Any) => {
                    (self.carried_count(*wcid) / row.count.max(1)).max(1)
                }
                _ => 1,
            };
            let hand_in = HandIn {
                row,
                at,
                xp: line.xp * u64::from(times),
                pursued: pursued(&line.id),
            };
            out.push((d, hand_in));
        }
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out.into_iter().take(most).map(|(_, h)| h).collect()
    }

    /// Where the run sets off from, as the journey planner is told it.
    fn start_place(&mut self) -> Option<Place> {
        let pl = self.player.as_ref()?;
        let (at, cell) = (pl.world_position(), pl.cell);
        let underground = self.underground();
        Some(Place {
            at,
            cell: crate::travel::planning_cell(cell, at, underground),
        })
    }

    /// The seconds of a journey from `from` to `to` (both as the planner is told them), planned
    /// as `set_off` plans one and not set off on; `None` with no way.
    fn leg_seconds(&self, from: Place, to: Place) -> Option<f32> {
        let prefs = self.travel_prefs().in_dungeon(from.is_indoors());
        let level = self.world.stats.level.max(1) as u32;
        let gems = self.carried_gems();
        let recalls = if prefs.use_recalls {
            self.castable_recalls()
        } else {
            Vec::new()
        };
        let quests = &self.travel.quests_done;
        let plan = |p| {
            ac_world::trip::plan_with_recalls_and_gems(
                from,
                to,
                level,
                quests,
                &[],
                &[],
                &recalls,
                &gems,
                p,
            )
        };
        let far = ac_world::trip::Prefs::far();
        plan(prefs)
            .or_else(|| {
                plan(ac_world::trip::Prefs {
                    walk_reach: far.walk_reach,
                    portal_reach: far.portal_reach,
                    ..prefs
                })
            })
            .map(|t| t.seconds)
    }

    /// Price the legs between the start, `calls` and the ground the run goes back to.
    fn plan_legs(&mut self, calls: &[Place]) -> Option<Legs> {
        self.tell_building_cells();
        let from = self.start_place()?;
        let calls: Vec<Place> = calls.iter().map(|&p| self.planning_place(p)).collect();
        let end = self
            .autoplay
            .growth
            .hunting_at
            .and_then(ac_world::hunting::at)
            .map(|g| g.place());
        let end = end.map(|p| self.planning_place(p));
        Some(Legs::plan(from, &calls, end, |a, b| self.leg_seconds(a, b)))
    }

    /// Givers no journey reaches, refused when the run is planned: not asked again for a while.
    fn refuse_givers(&mut self, hand_ins: &[HandIn], refused: &[usize], now: Instant) {
        for &k in refused {
            let HandIn { row, at, .. } = hand_ins[k];
            self.refuse_giver(row, at, &format!("no way to {}", row.giver), now);
        }
    }

    fn refuse_giver(&mut self, row: &Row, at: Place, why: &str, now: Instant) {
        // Not asked after all: no answer is waited for.
        if self
            .quest_asked
            .as_ref()
            .is_some_and(|a| std::ptr::eq(a.row, row))
        {
            self.quest_asked = None;
        }
        let key = (row.giver_wcid, at.cell);
        self.autoplay.growth.givers_refused.mark(key, now);
        self.autoplay.note(format!("{why} for {}", row.title), now);
    }

    /// The run that just set off for its counter takes the ready hand-ins that pay, every call
    /// put in the fastest order; the first may be a giver, the counter then waiting its turn.
    pub(super) fn add_hand_ins(&mut self, now: Instant) {
        let quests = &self.autoplay.config.quests;
        if !quests.on && quests.pursue.is_empty() {
            return;
        }
        let Some(run) = self.autoplay.growth.run.as_ref() else {
            return;
        };
        let counter = Call {
            at: counter_place(run.at),
            on: CallOn::Counter {
                errand: run.errand,
                vendor: run.vendor.clone(),
            },
        };
        let me = self.my_position().map_or(run.at, |p| p.truncate());
        let hand_ins = self.ready_hand_ins(&[run.at, me], EVERY_ORDER - 1, now);
        if hand_ins.is_empty() {
            return;
        }
        let mut places = vec![counter.at];
        places.extend(hand_ins.iter().map(|h| h.at));
        let Some(legs) = self.plan_legs(&places) else {
            return;
        };
        let planned = plan_calls(&legs, &[counter.at], &hand_ins, self.xp_per_hour());
        self.refuse_givers(&hand_ins, &planned.refused, now);
        let mut calls: Vec<Call> = planned
            .order
            .iter()
            .map(|&i| match i {
                0 => counter.clone(),
                g => giver_call(hand_ins[g - 1].row, hand_ins[g - 1].at),
            })
            .collect();
        if calls.len() <= 1 {
            return;
        }
        let Some(mut run) = self.autoplay.growth.run.take() else {
            return;
        };
        let first = calls.remove(0);
        run.calls = calls;
        let givers: Vec<&str> = std::iter::once(&first)
            .chain(&run.calls)
            .filter_map(|c| match &c.on {
                CallOn::Giver { row } => Some(row.giver),
                CallOn::Counter { .. } => None,
            })
            .collect();
        self.autoplay.note(
            format!("the run hands in to {} too", givers.join(", ")),
            now,
        );
        if first == counter || self.make_call(&mut run, first, now) {
            self.autoplay.growth.run = Some(run);
            return;
        }
        // No way to the first giver after all: on to the next call, the counter among them.
        if let Some(run) = self.next_call(run, now) {
            let cfg = self.autoplay.config.growth.clone();
            let needs = self.grow_needs(&cfg);
            let full = self.pack_low_on_room();
            self.end_run(run, now, &cfg, &needs, full, None);
        }
    }

    /// A run of one call on `row`'s giver at `at`, for a pursued quest: started at once, town runs
    /// on or off, and never futile (`Run::for_quest`). False when there is no way to the giver.
    pub(crate) fn start_quest_call(&mut self, row: &'static Row, at: Place, now: Instant) -> bool {
        let spot = at.at.truncate();
        let mut run = Run {
            vendor: row.giver.to_string(),
            at: spot,
            phase: Phase::Going,
            since: now,
            last_sell: None,
            town: spot,
            stops: 0,
            sold: 0,
            reason: format!("for {}", row.title.trim()),
            errand: Errand::Sell,
            visited: Vec::new(),
            walked_on: None,
            walking_up: false,
            calls: Vec::new(),
            for_quest: true,
            short_with: None,
            looked_from: None,
        };
        if !self.make_call(&mut run, giver_call(row, at), now) {
            return false;
        }
        let st = &mut self.autoplay.growth;
        st.drop_ground_walk();
        st.needs.clear();
        st.by_hand = false;
        st.window_unwanted = None;
        st.run = Some(run);
        true
    }

    /// Make `call` the run's call in hand and set off for it. False when there is no way.
    fn make_call(&mut self, run: &mut Run, call: Call, now: Instant) -> bool {
        run.since = now;
        run.at = call.at.at.truncate();
        run.walked_on = None;
        run.walking_up = false;
        match call.on {
            CallOn::Counter { errand, vendor } => {
                run.vendor = vendor;
                run.errand = errand;
                run.phase = Phase::Going;
                if !run.visited.iter().any(|v| v.distance(run.at) < 1.0) {
                    run.visited.push(run.at);
                }
                let ok = self.grow_travel(call.at, VENDOR_REACH);
                if ok {
                    self.autoplay.say(
                        Doing::Shopping,
                        format!("{}: on to {}", run.reason, run.vendor),
                    );
                }
                ok
            }
            CallOn::Giver { row } => {
                let at = call.at;
                run.vendor = row.giver.to_string();
                run.phase = Phase::ToGiver { row, at, tries: 1 };
                let ok = self.visit_giver(row, at);
                if ok {
                    self.autoplay.say(
                        Doing::Shopping,
                        format!("{}: to {} for {}", run.reason, row.giver, row.title),
                    );
                } else {
                    self.refuse_giver(row, at, &format!("no way to {}", row.giver), now);
                }
                ok
            }
        }
    }

    /// On to the run's next call that can be set off for; the run back when no call is left.
    pub(super) fn next_call(&mut self, mut run: Run, now: Instant) -> Option<Run> {
        while !run.calls.is_empty() {
            let call = run.calls.remove(0);
            if self.make_call(&mut run, call, now) {
                self.autoplay.growth.run = Some(run);
                return None;
            }
        }
        Some(run)
    }

    /// Done with a giver: on to the next call, or the run is over.
    fn giver_done(&mut self, run: Run, now: Instant, cfg: &Growth) -> Turn {
        let Some(run) = self.next_call(run, now) else {
            return Turn::Acted;
        };
        let needs = self.grow_needs(cfg);
        let full = self.pack_low_on_room();
        Turn::after_stop(self.end_run(run, now, cfg, &needs, full, None))
    }

    /// The giver of `row`, in view within a give's reach of the character.
    fn giver_in_reach(&self, row: &Row) -> Option<u32> {
        let me = self.my_position()?;
        let mine = self.world.player_guid;
        self.world
            .objects
            .values()
            .filter(|o| Some(o.guid) != mine && o.container.is_none() && o.name == row.giver)
            .filter_map(|o| Some((o.guid, o.world_pos()?.distance(me))))
            .filter(|(_, d)| *d <= GIVER_REACH)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(g, _)| g)
    }

    /// One turn of a call on a giver: the visit there, then the hand-in.
    pub(super) fn giver_step(&mut self, mut run: Run, now: Instant, cfg: &Growth) -> Turn {
        match run.phase.clone() {
            Phase::ToGiver { row, at, tries } => {
                if self.visits.current.is_some() {
                    // Asked again each frame in reach, so the XP before the visit's use is kept.
                    if self.giver_in_reach(row).is_some() {
                        self.ask_giver(row, at, now);
                    }
                    self.autoplay.say(
                        Doing::Shopping,
                        format!("{}: going to {} for {}", run.reason, row.giver, row.title),
                    );
                    self.autoplay.growth.run = Some(run);
                    return Turn::Waited;
                }
                if let Some(giver) = self.giver_in_reach(row).filter(|_| self.visits.finished) {
                    // A talk is over when the giver has said something: else it is waited for, used
                    // again, and on the last try taken as heard (a_talk_taking_a_quest_up_is_heard_once_made_and_not_made_again_however_soon_it_is_called_for).
                    let spoke = row.hand != Handing::Talk || self.npc_spoke == Some(giver);
                    let since = match self.autoplay.growth.talk_wait {
                        Some((g, t)) if g == giver => t,
                        _ => {
                            self.npc_spoke = None;
                            self.autoplay.growth.talk_wait = Some((giver, now));
                            now
                        }
                    };
                    if !spoke && now.saturating_duration_since(since) < TALK_WAIT {
                        let said = format!("waiting for {} to answer", row.giver);
                        self.autoplay.say(Doing::Shopping, said);
                        self.autoplay.growth.run = Some(run);
                        return Turn::Waited;
                    }
                    self.autoplay.growth.talk_wait = None;
                    let again = !spoke && tries < GIVER_TRIES;
                    if row.hand == Handing::Talk && !again {
                        // The visit's use was the hand-in: kills done, a talk, items it takes.
                        if !self
                            .quest_asked
                            .as_ref()
                            .is_some_and(|a| std::ptr::eq(a.row, row))
                        {
                            self.ask_giver(row, at, now);
                        }
                        self.autoplay.growth.talked.mark(talk_key(row), now);
                        // A talk taking a quest up pays nothing: there is no XP to wait for.
                        if row.kind == Kind::Start {
                            self.answer_take_up(now);
                        }
                        self.autoplay
                            .note(format!("talked to {} for {}", row.giver, row.title), now);
                        return self.giver_done(run, now, cfg);
                    }
                    if !again {
                        run.phase = Phase::HandingIn {
                            row,
                            at,
                            giver,
                            taken: 0,
                            last: None,
                            stalls: 0,
                        };
                        run.since = now;
                        self.autoplay.growth.run = Some(run);
                        return Turn::Waited;
                    }
                }
                // A fight on the way broke the visit off: the run picks it up after.
                if self.attack_target.is_some() || self.autoplay.casting_at().is_some() {
                    self.autoplay.growth.run = Some(run);
                    return Turn::Waited;
                }
                if tries >= GIVER_TRIES || !self.visit_giver(row, at) {
                    self.refuse_giver(row, at, &format!("could not get to {}", row.giver), now);
                    return self.giver_done(run, now, cfg);
                }
                run.phase = Phase::ToGiver {
                    row,
                    at,
                    tries: tries + 1,
                };
                self.autoplay.growth.run = Some(run);
                Turn::Acted
            }
            Phase::HandingIn {
                row,
                at,
                giver,
                taken,
                last,
                stalls,
            } => self.hand_in(run, (row, at, giver, taken, last, stalls), now, cfg),
            _ => {
                self.autoplay.growth.run = Some(run);
                Turn::Waited
            }
        }
    }

    /// Give the giver the wanted items one at a time: ACE takes one per give whatever the
    /// amount (Player_Inventory.cs:3439-3443). The next goes once the last left the pack.
    fn hand_in(
        &mut self,
        mut run: Run,
        state: (&'static Row, Place, u32, u32, Option<(Instant, u32)>, u32),
        now: Instant,
        cfg: &Growth,
    ) -> Turn {
        let (row, at, giver, mut taken, last, mut stalls) = state;
        let Want::Item(wcid) = row.want else {
            return self.giver_done(run, now, cfg);
        };
        let carried = self.carried_count(wcid);
        if let Some((at, before)) = last {
            if carried < before {
                taken += 1;
                stalls = 0;
            } else if now.duration_since(at) < GIVE_PAUSE {
                self.autoplay.growth.run = Some(run);
                return Turn::Waited;
            } else {
                stalls += 1;
            }
        }
        // A timed quest handed in starts its wait, from the reward or the server's words.
        let waits = !row.timer.is_empty()
            && self
                .quest_log
                .wait_until(row.timer, crate::holdings::unix_now())
                .is_some();
        let more = row.repeat == Repeat::Any || taken == 0;
        let item = self
            .world
            .inventory()
            .find(|o| o.weenie_class_id == wcid && o.wielder.is_none())
            .map(|o| o.guid);
        let ends = !more || waits || stalls >= GIVE_STALLS || carried < row.count.max(1);
        let Some(item) = item.filter(|_| !ends) else {
            if taken == 0 && stalls >= GIVE_STALLS {
                self.refuse_giver(row, at, &format!("{} would not take it", row.giver), now);
            }
            self.autoplay.note(
                format!("handed {taken} {} to {}", item_name(row), row.giver),
                now,
            );
            return self.giver_done(run, now, cfg);
        };
        if !self.give(giver, item, Some(1)) {
            return self.giver_done(run, now, cfg);
        }
        self.ask_giver(row, at, now);
        self.autoplay.say(
            Doing::Shopping,
            format!("handing {} to {}", item_name(row), row.giver),
        );
        run.phase = Phase::HandingIn {
            row,
            at,
            giver,
            taken,
            last: Some((now, carried)),
            stalls,
        };
        self.autoplay.growth.run = Some(run);
        Turn::Acted
    }
}

/// What a talk made is remembered by (`growth::State::talked`).
fn talk_key(row: &Row) -> (String, u32, bool) {
    (row.id.to_string(), row.giver_wcid, row.kind == Kind::Start)
}

/// A call on `row`'s giver, at its placement `at`.
fn giver_call(row: &'static Row, at: Place) -> Call {
    Call {
        at,
        on: CallOn::Giver { row },
    }
}

fn item_name(row: &Row) -> &str {
    row.item.as_ref().map_or("the item", |i| i.name)
}

#[cfg(test)]
mod tests;
