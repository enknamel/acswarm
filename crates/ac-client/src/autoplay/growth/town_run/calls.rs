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
use crate::quest_log::{worth_detour, QuestState, SHORT_LEG};
use crate::Client;

/// A giver no journey reached is planned to again after this (a Warden's wait, hunt.rs).
const GIVER_RETRY: Duration = Duration::from_secs(30 * 60);
/// A give not answered in this long counts as unanswered (design §5.1, guess).
const GIVE_PAUSE: Duration = Duration::from_secs(3);
/// Unanswered gives before the call ends: ACE turns a give away while the giver emotes
/// (Player_Inventory.cs:3422-3427), and the visit's own use sets one off.
const GIVE_STALLS: u32 = 3;
/// Visits begun to one giver on one call before it is given up.
const GIVER_TRIES: u32 = 3;
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

/// A ready hand-in a run may take: the giver's row and the XP of every hand-in carried.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HandIn {
    pub(super) row: &'static Row,
    pub(super) xp: u64,
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
/// best XP a second of detour first, each tried on the tour as it stands, to [`EVERY_ORDER`] calls.
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
        let block = hand_ins[g - n].row.place.cell >> 16;
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
    tried.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (g, _) in tried {
        if taken.len() >= EVERY_ORDER {
            break;
        }
        let with = [taken.as_slice(), &[g]].concat();
        let Some(w) = secs(&with) else {
            continue;
        };
        let detour = base.map_or(w, |b| (w - b).max(0.0));
        if on_the_round(g, detour) || worth_detour(hand_ins[g - n].xp, detour, xp_per_hour) {
            taken = with;
            base = Some(w);
        }
    }
    let order = legs.tour(&taken).map_or(taken, |(o, _)| o);
    CallPlan { order, refused }
}

impl Client {
    /// The hand-ins ready on the board, each at the giver nearest the first of `near` it can be
    /// asked at (level band, open door, not lately refused), nearest first, at most `most`.
    pub(super) fn ready_hand_ins(&self, near: &[Vec2], most: usize, now: Instant) -> Vec<HandIn> {
        let level = self.world.stats.level.max(0) as u32;
        let held = &self.travel.quests_done;
        let refused = &self.autoplay.growth.givers_refused;
        let away = |r: &Row| {
            near.iter()
                .map(|p| p.distance(r.place.at.truncate()))
                .fold(f32::MAX, f32::min)
        };
        let mut out: Vec<(f32, HandIn)> = Vec::new();
        for line in &self.quest_board {
            if !line.on || line.state != QuestState::Ready || line.kind == Kind::Start {
                continue;
            }
            let row = ac_world::quests::rows_of(&line.id)
                .filter(|r| r.kind == line.kind && r.fits_level(level))
                .filter(|r| match r.want {
                    Want::Item(w) => self.carried_count(w) >= r.count.max(1),
                    _ => true,
                })
                .filter(|r| r.locked_by(held).is_none())
                .filter(|r| !refused.within(&(r.giver_wcid, r.place.cell), now, GIVER_RETRY))
                .min_by(|a, b| away(a).total_cmp(&away(b)));
            let Some(row) = row else {
                continue;
            };
            // A hand-in that repeats any time pays for each one carried (the Collectors).
            let times = match (&row.want, row.repeat) {
                (Want::Item(wcid), Repeat::Any) => {
                    (self.carried_count(*wcid) / row.count.max(1)).max(1)
                }
                _ => 1,
            };
            out.push((
                away(row),
                HandIn {
                    row,
                    xp: line.xp * u64::from(times),
                },
            ));
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
            let row = hand_ins[k].row;
            self.refuse_giver(row, &format!("no way to {}", row.giver), now);
        }
    }

    fn refuse_giver(&mut self, row: &Row, why: &str, now: Instant) {
        // Not asked after all: no answer is waited for.
        if self
            .quest_asked
            .as_ref()
            .is_some_and(|a| std::ptr::eq(a.row, row))
        {
            self.quest_asked = None;
        }
        let key = (row.giver_wcid, row.place.cell);
        self.autoplay.growth.givers_refused.mark(key, now);
        self.autoplay.note(format!("{why} for {}", row.title), now);
    }

    /// The run that just set off for its counter takes the ready hand-ins that pay, every call
    /// put in the fastest order; the first may be a giver, the counter then waiting its turn.
    pub(super) fn add_hand_ins(&mut self, now: Instant) {
        if !self.autoplay.config.quests.on {
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
        places.extend(hand_ins.iter().map(|h| h.row.place));
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
                g => giver_call(hand_ins[g - 1].row),
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
                CallOn::Giver { row } => Some(row.giver.as_str()),
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
                run.vendor = row.giver.clone();
                run.phase = Phase::ToGiver { row, tries: 1 };
                let ok = self.visit_giver(row);
                if ok {
                    self.autoplay.say(
                        Doing::Shopping,
                        format!("{}: to {} for {}", run.reason, row.giver, row.title),
                    );
                } else {
                    self.refuse_giver(row, &format!("no way to {}", row.giver), now);
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
            Phase::ToGiver { row, tries } => {
                if self.visits.current.is_some() {
                    // Asked again each frame in reach, so the XP before the visit's use is kept.
                    if self.giver_in_reach(row).is_some() {
                        self.ask_giver(row, now);
                    }
                    self.autoplay.say(
                        Doing::Shopping,
                        format!("{}: going to {} for {}", run.reason, row.giver, row.title),
                    );
                    self.autoplay.growth.run = Some(run);
                    return Turn::Waited;
                }
                if let Some(giver) = self.giver_in_reach(row).filter(|_| self.visits.finished) {
                    if row.hand == Handing::Talk {
                        // The visit's use was the hand-in: kills done, a talk, items it takes.
                        if !self
                            .quest_asked
                            .as_ref()
                            .is_some_and(|a| std::ptr::eq(a.row, row))
                        {
                            self.ask_giver(row, now);
                        }
                        self.autoplay
                            .note(format!("talked to {} for {}", row.giver, row.title), now);
                        return self.giver_done(run, now, cfg);
                    }
                    run.phase = Phase::HandingIn {
                        row,
                        giver,
                        taken: 0,
                        last: None,
                        stalls: 0,
                    };
                    run.since = now;
                    self.autoplay.growth.run = Some(run);
                    return Turn::Waited;
                }
                // A fight on the way broke the visit off: the run picks it up after.
                if self.attack_target.is_some() || self.autoplay.casting_at().is_some() {
                    self.autoplay.growth.run = Some(run);
                    return Turn::Waited;
                }
                if tries >= GIVER_TRIES || !self.visit_giver(row) {
                    self.refuse_giver(row, &format!("could not get to {}", row.giver), now);
                    return self.giver_done(run, now, cfg);
                }
                run.phase = Phase::ToGiver {
                    row,
                    tries: tries + 1,
                };
                self.autoplay.growth.run = Some(run);
                Turn::Acted
            }
            Phase::HandingIn {
                row,
                giver,
                taken,
                last,
                stalls,
            } => self.hand_in(run, (row, giver, taken, last, stalls), now, cfg),
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
        state: (&'static Row, u32, u32, Option<(Instant, u32)>, u32),
        now: Instant,
        cfg: &Growth,
    ) -> Turn {
        let (row, giver, mut taken, last, mut stalls) = state;
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
                .wait_until(&row.timer, crate::holdings::unix_now())
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
                self.refuse_giver(row, &format!("{} would not take it", row.giver), now);
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
        self.ask_giver(row, now);
        self.autoplay.say(
            Doing::Shopping,
            format!("handing {} to {}", item_name(row), row.giver),
        );
        run.phase = Phase::HandingIn {
            row,
            giver,
            taken,
            last: Some((now, carried)),
            stalls,
        };
        self.autoplay.growth.run = Some(run);
        Turn::Acted
    }
}

/// A call on `row`'s giver, at its place and cell.
fn giver_call(row: &'static Row) -> Call {
    Call {
        at: row.place,
        on: CallOn::Giver { row },
    }
}

fn item_name(row: &Row) -> &str {
    row.item.as_ref().map_or("the item", |i| i.name.as_str())
}

#[cfg(test)]
mod tests;
