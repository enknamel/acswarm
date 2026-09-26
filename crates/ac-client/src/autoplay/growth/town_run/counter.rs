use std::time::{Duration, Instant};

use glam::Vec2;

use super::vendor::spot;
use super::{
    Errand, Phase, Run, Turn, BUSY_ASKS, BUSY_REASK, COUNTER_REACH, SELLING_TIMEOUT, SETTLE,
    STOPS_PER_RUN, VENDOR_OPEN_TIMEOUT, VENDOR_REACH,
};
use crate::autoplay::growth::road::{on_the_way, OnTheWay, WALK_ON_EVERY};
use crate::autoplay::growth::{a_few, about, Growth};
use crate::autoplay::Doing;
use crate::Client;

/// How long a counter whose window bought none of the pack is left out of the choice: a shelf
/// list does not change between runs, and runs follow each other with no wait.
const BUYS_NONE_HOLD: Duration = Duration::from_secs(60 * 60);

/// What a run waiting on a counter's window does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OnOpening {
    /// The window is open: on to the pack.
    Opened,
    /// Nothing yet.
    Wait,
    /// Send the Use over. The last one was turned away for our own
    /// doing, so this is not counted against the counter.
    AskOver,
    /// Ask again after the counter's silence, counted against it.
    AskAgain,
    /// The counter will not open.
    GiveUp,
}

/// What a run waiting on a counter's window does next (see
/// [`Phase::Opening`]).
///
/// ACE will not open a window for a character in the middle of
/// something: `Vendor.ActOnUse` turns the Use away as YoureTooBusy
/// while `IsBusy` is set, and a cast sets it for its recoil, from the
/// last gesture to the Ready stance (`Player_Magic.cs`, `FinishCast`;
/// the gestures before it set only `MagicState.IsCasting`, which the
/// counter does not look at). A refusal like that is ours, not the
/// counter's -- +Vesperi arrived at Archmage Cindrue with eight
/// protections lapsing and had every Use turned away until the run gave
/// the counter up as one that "would not trade", the peas still in the
/// pack. So a Use turned away as busy (`busy`, how long ago) is sent
/// over once nothing of ours is in flight and a recoil's length has
/// passed, without spending the retry or the counter's patience on it.
/// Only silence and the counter's own refusal give it up, and a counter
/// that keeps saying busy past [`BUSY_ASKS`] sends is taken at its word.
pub(crate) fn on_opening(
    open: bool,
    busy: Option<Duration>,
    ours_in_flight: bool,
    asked_over: u32,
    elapsed: Duration,
    tries: u32,
) -> OnOpening {
    if open {
        return OnOpening::Opened;
    }
    if let Some(ago) = busy.filter(|_| asked_over < BUSY_ASKS) {
        return if ours_in_flight || ago < BUSY_REASK {
            OnOpening::Wait
        } else {
            OnOpening::AskOver
        };
    }
    if elapsed <= VENDOR_OPEN_TIMEOUT {
        OnOpening::Wait
    } else if tries < 2 {
        OnOpening::AskAgain
    } else {
        OnOpening::GiveUp
    }
}

/// The counter the character has asked for its window, if any: the
/// one a run is asking, or the one whose window is open -- the run's
/// once it is past the asking, or the player's own with no run on. A
/// window open while a run is still walking is one left over from
/// before it, not a counter at hand. A cast made at one is turned away
/// or turns the counter's answer away (see [`on_opening`]), so the
/// casts that can wait do (see `Client::counter_holding_casts`).
pub(super) fn counter_asked(phase: Option<&Phase>, window: Option<u32>) -> Option<u32> {
    match phase {
        Some(Phase::Going) => None,
        Some(Phase::Opening { guid, .. }) => Some(*guid),
        Some(Phase::Appraising | Phase::Selling { .. }) | None => window,
    }
}

impl Client {
    /// Something was handed to the counter and it has not answered
    /// yet.
    ///
    /// The answer is what paces the shopping: an item leaving the pack,
    /// a purchase arriving, or a refusal. `cast_sent` is cleared by the
    /// server's `UseDone`, which is the same signal for a counter as
    /// for a spell; the clock behind it only stops a run waiting for
    /// ever on an answer that never comes.
    fn vendor_busy(&self, now: Instant) -> bool {
        self.autoplay.cast_in_flight(now)
    }

    /// One turn of the run in progress: what it came to, for the panel's
    /// Step; whether it goes on, for autoplay (see [`Turn`]).
    pub(super) fn grow_run_step(&mut self, now: Instant, cfg: &Growth) -> Turn {
        let Some(mut run) = self.autoplay.growth.run.take() else {
            return Turn::Over;
        };
        let Some(me) = self.my_position() else {
            self.autoplay.growth.run = Some(run);
            return Turn::Waited;
        };
        let me = Vec2::new(me.x, me.y);
        let elapsed = now.duration_since(run.since);
        match run.phase.clone() {
            Phase::Going => {
                // Out of time only while still short of it: one that got there is not walked away.
                if elapsed > run.walk_limit && run.at.distance(me) > VENDOR_REACH {
                    self.autoplay.note(
                        format!("the walk to {} is taking too long", run.vendor),
                        now,
                    );
                    self.cancel_travel();
                    return Turn::after_stop(self.grow_run_next(
                        run,
                        now,
                        cfg,
                        Some("was too long a walk"),
                    ));
                }
                if self.traveling() || self.grow_travel_on() {
                    self.autoplay.say(
                        Doing::Shopping,
                        format!(
                            "{}: going to {} ({})",
                            run.reason,
                            run.vendor,
                            about(run.at.distance(me))
                        ),
                    );
                    self.autoplay.growth.run = Some(run);
                    return Turn::Waited;
                }
                // A corpse, a fight or a dodge on the way ends the journey
                // without it having got anywhere, and the run picks it up
                // again rather than taking it for a walk that could not.
                let away = run.at.distance(me);
                let busy = self.attack_target.is_some() || self.autoplay.casting_at().is_some();
                let lately = run
                    .walked_on
                    .is_some_and(|t| now.duration_since(t) < WALK_ON_EVERY);
                match on_the_way(
                    away <= VENDOR_REACH,
                    self.journey_broken_off(),
                    busy,
                    lately,
                ) {
                    OnTheWay::There => {}
                    OnTheWay::Wait => {
                        self.autoplay.growth.run = Some(run);
                        return Turn::Waited;
                    }
                    OnTheWay::WalkOn => {
                        if !self.grow_travel(run.at, now) {
                            self.autoplay.note(
                                format!(
                                    "no way on to {} from here ({away:.0} m short)",
                                    run.vendor
                                ),
                                now,
                            );
                            return Turn::after_stop(self.grow_run_next(
                                run,
                                now,
                                cfg,
                                Some("could not be got to"),
                            ));
                        }
                        self.autoplay.note(
                            format!("on the way to {} again ({away:.0} m)", run.vendor),
                            now,
                        );
                        run.walked_on = Some(now);
                        self.autoplay.growth.run = Some(run);
                        return Turn::Acted;
                    }
                    OnTheWay::Short => {
                        self.autoplay.note(
                            format!("could not get to {} ({away:.0} m short)", run.vendor),
                            now,
                        );
                        return Turn::after_stop(self.grow_run_next(
                            run,
                            now,
                            cfg,
                            Some("could not be got to"),
                        ));
                    }
                }
                match self.vendor_object(&run.vendor, run.at) {
                    Some(guid) => {
                        // Stand at the counter before asking it to
                        // open. The journey gets the character within
                        // thirty-five metres of where the vendor is
                        // listed, and a vendor will not trade with
                        // somebody across the room: the server answers
                        // by telling us to walk there, and a client
                        // that does not walk waits for a window that
                        // never opens. The same thing that left
                        // characters standing under corpses.
                        let where_it_is = self.world.objects.get(&guid).and_then(|o| o.world_pos());
                        if let Some(spot) = where_it_is {
                            if Vec2::new(spot.x, spot.y).distance(me) > COUNTER_REACH {
                                self.head_for(spot, COUNTER_REACH / 2.0, &run.vendor);
                                self.autoplay
                                    .say(Doing::Shopping, format!("walking up to {}", run.vendor));
                                self.autoplay.growth.run = Some(run);
                                return Turn::Acted;
                            }
                        }
                        // A cast of ours still in the air -- a
                        // protection put back on the walk in, a heal
                        // -- lands before the counter is asked. The
                        // buffs wait once the Use is out (see
                        // `Client::counter_holding_casts`), but a Use
                        // thrown over a cast already going up meets
                        // its recoil as often as not, and is turned
                        // away for it (see `on_opening`): a second's
                        // wait here against a re-ask three seconds
                        // later.
                        if self.autoplay.cast_in_flight(now) {
                            self.autoplay.say(
                                Doing::Shopping,
                                format!(
                                    "waiting for the cast to land before talking to {}",
                                    run.vendor
                                ),
                            );
                            self.autoplay.growth.run = Some(run);
                            return Turn::Waited;
                        }
                        if self.follow.take().is_some() {
                            self.steering.reset();
                        }
                        self.use_object(guid);
                        run.phase = Phase::Opening {
                            guid,
                            tries: 1,
                            busy: None,
                            asked_over: 0,
                        };
                        run.since = now;
                        self.autoplay
                            .say(Doing::Shopping, format!("talking to {}", run.vendor));
                        self.autoplay.growth.run = Some(run);
                        Turn::Acted
                    }
                    None => {
                        self.autoplay.note(
                            format!("{} is not here (indoors, or gone)", run.vendor),
                            now,
                        );
                        Turn::after_stop(self.grow_run_next(run, now, cfg, Some("is not here")))
                    }
                }
            }
            Phase::Opening {
                guid,
                tries,
                busy,
                asked_over,
            } => {
                let open = self
                    .world
                    .open_vendor
                    .as_ref()
                    .is_some_and(|v| v.vendor == guid);
                // A swing in the air is not ours to wait out: ACE is
                // never busy for one (`IsBusy` is set by casts, uses
                // and the like, never in `Player_Melee`), so the
                // counter did not turn the Use away for it -- and a
                // swing's answer can go missing (see
                // `attack_unanswered`). Read raw, one lost AttackDone
                // parked the run here with no clock at all.
                let ours_in_flight = self.autoplay.cast_in_flight(now);
                let next = on_opening(
                    open,
                    busy.map(|t| now.saturating_duration_since(t)),
                    ours_in_flight,
                    asked_over,
                    elapsed,
                    tries,
                );
                match next {
                    OnOpening::Opened => {
                        // Appraise what might be sold, so weapons can
                        // be judged.
                        let candidates: Vec<u32> = self
                            .world
                            .inventory()
                            .filter(|o| !o.wielder.is_some())
                            .filter(|o| o.value > 0)
                            .filter(|o| !self.appraisals.contains_key(&o.guid))
                            .map(|o| o.guid)
                            .collect();
                        let n = self.appraise_many(candidates);
                        run.phase = Phase::Appraising;
                        run.since = now;
                        self.autoplay.say(
                            Doing::Shopping,
                            format!("at {}; looking over {n} item(s)", run.vendor),
                        );
                        self.autoplay.growth.run = Some(run);
                        Turn::Acted
                    }
                    OnOpening::AskOver => {
                        self.use_object(guid);
                        run.phase = Phase::Opening {
                            guid,
                            tries,
                            busy: None,
                            asked_over: asked_over + 1,
                        };
                        // The counter's time starts over: the wait so
                        // far was ours.
                        run.since = now;
                        self.autoplay.note(
                            format!("asking {} again now the cast has landed", run.vendor),
                            now,
                        );
                        self.autoplay.growth.run = Some(run);
                        Turn::Acted
                    }
                    OnOpening::AskAgain => {
                        self.use_object(guid);
                        run.phase = Phase::Opening {
                            guid,
                            tries: tries + 1,
                            busy: None,
                            asked_over,
                        };
                        run.since = now;
                        self.autoplay.growth.run = Some(run);
                        Turn::Acted
                    }
                    OnOpening::GiveUp => {
                        self.autoplay
                            .note(format!("{} would not trade", run.vendor), now);
                        Turn::after_stop(self.grow_run_next(run, now, cfg, Some("would not trade")))
                    }
                    OnOpening::Wait => {
                        if busy.is_some() {
                            self.autoplay.say(
                                Doing::Shopping,
                                format!(
                                    "{} turned the Use away while a cast was in the air; asking again once it lands",
                                    run.vendor
                                ),
                            );
                        }
                        self.autoplay.growth.run = Some(run);
                        Turn::Waited
                    }
                }
            }
            Phase::Appraising => {
                let waiting = self
                    .world
                    .inventory()
                    .filter(|o| o.value > 0)
                    .any(|o| !self.appraisals.contains_key(&o.guid));
                if waiting && elapsed < SETTLE * 2 {
                    self.autoplay.growth.run = Some(run);
                    return Turn::Waited;
                }
                // Compress the pack before selling any of it.
                //
                // Loose change and part stacks waste slots, and slots
                // are what a sale runs out of. Doing it here rather
                // than during the selling matters: a merge makes one
                // stack vanish into another, and doing that while a
                // sale is in flight pulls the ground from under it.
                //
                // It stays here until there is nothing left to pour.
                // Falling through after a single pour left the pack
                // almost as loose as it started.
                // Acting or waiting on a pour keeps the tick. Done
                // means the pack is as tight as it goes; blocked means
                // it cannot be tightened until something is sold, and
                // selling is the next thing either way.
                match self.compress(now) {
                    crate::did::Did::Acting => {
                        self.autoplay.growth.run = Some(run);
                        return Turn::Acted;
                    }
                    crate::did::Did::Waiting(_) => {
                        self.autoplay.growth.run = Some(run);
                        return Turn::Waited;
                    }
                    _ => {}
                }
                let n = self.sale_list(cfg).len();
                // The window says what this counter takes, and when that
                // is none of what the pack holds for a counter, say so
                // plainly -- "sold 0 item(s)" told nobody why. A run
                // made to sell does not stand here buying: it goes on
                // to a counter that will take the loot, and this one is
                // left alone for a while.
                if n == 0 {
                    let carrying = self.salables(cfg).len();
                    if carrying > 0 {
                        let why = format!(
                            "{} buys none of this ({carrying} thing(s) for a counter)",
                            run.vendor
                        );
                        self.autoplay.note(why.clone(), now);
                        // The window is the server's own word on what
                        // this counter takes, and it is not going to
                        // change by the next run. Left on the half
                        // minute a blocked counter gets, and tidied
                        // away at the end of the run, this counter was
                        // the best-paying choice again on the next run.
                        self.autoplay
                            .growth
                            .skip_vendors
                            .hold(spot(run.at), BUYS_NONE_HOLD, now);
                        if run.errand == Errand::Sell {
                            self.autoplay.say(Doing::Shopping, why);
                            return Turn::after_stop(self.grow_run_next(
                                run,
                                now,
                                cfg,
                                Some("buys none of this"),
                            ));
                        }
                    }
                }
                run.phase = Phase::Selling { sent: Vec::new() };
                run.since = now;
                // The shopping rules start this counter fresh. They
                // remember what was offered, what was refused and how
                // far the trip got, and none of that is this counter's:
                // a trip they had already finished -- or one that never
                // found a window and finished itself for that -- would
                // close this counter at once, without selling a thing.
                self.autoplay.growth.shop = ac_vendor::Run::new();
                self.autoplay.say(
                    Doing::Shopping,
                    format!("selling {n} item(s) to {}", run.vendor),
                );
                self.autoplay.growth.run = Some(run);
                // Nothing went out: the first thing the rules ask for
                // is the next turn's.
                Turn::Waited
            }
            Phase::Selling { .. } => {
                // The shopping itself is decided in `ac-vendor`, which
                // has no idea what a socket is: it is handed a plain
                // description of the pack, the purse and the counter
                // and answers with one thing to do. This side reads the
                // world into that description and carries the answer
                // out. The same rules run in the panel and in
                // `cargo run -p ac-vendor --example trip`, so a trip
                // can be argued about without a server being awake.
                // Wait on the counter's answer, not on a clock. The
                // server says when it has finished with what it was
                // handed -- the goods leave the pack, the shelf
                // changes, an error comes back -- and the next thing
                // goes out then. A fixed pause is either slower than
                // the counter or quicker, and quicker is how a run
                // sells a stack it has already merged away.
                //
                // The phase's clock runs from the first act here, not
                // from the last: an act turned down without a word the
                // rules can read is asked again on every answer, and
                // this is where that ends.
                if elapsed > SELLING_TIMEOUT {
                    self.autoplay.note(
                        format!("{} is taking too long to trade with", run.vendor),
                        now,
                    );
                    return Turn::after_stop(self.leave_counter(
                        run,
                        now,
                        cfg,
                        Some("took too long to trade with"),
                    ));
                }
                if self.vendor_busy(now) {
                    self.autoplay.growth.run = Some(run);
                    return Turn::Waited;
                }
                let snap = self.vendor_snapshot(cfg);
                let next = self.autoplay.growth.shop.step(&snap, now);
                self.autoplay.growth.last_saying = next.saying.clone();
                match next.act {
                    Some(ac_vendor::Act::Close) | None => {
                        // Said, so a visit that ends with nothing sold says why.
                        if !next.saying.is_empty() {
                            self.autoplay
                                .note(format!("{}: {}", run.vendor, next.saying), now);
                        }
                        Turn::after_stop(self.leave_counter(run, now, cfg, None))
                    }
                    Some(act) => {
                        // A refusal on this side is an answer too: the
                        // rules are told, so that they stop asking
                        // rather than spend the afternoon on it. Nothing
                        // went out, so the turn is a wait, not an act,
                        // and a Step goes on to what the rules ask next.
                        if !self.do_vendor_act(&act, &next.saying) {
                            self.autoplay
                                .note(format!("could not {}: {act:?}", next.saying), now);
                            self.autoplay.growth.shop.refused(&act, now);
                            self.autoplay.growth.run = Some(run);
                            return Turn::Waited;
                        }
                        run.last_sell = Some(now);
                        run.phase = Phase::Selling { sent: Vec::new() };
                        self.autoplay.growth.run = Some(run);
                        Turn::Acted
                    }
                }
            }
        }
    }

    /// Done at this counter's window: what it sold goes on the run's
    /// count, the shopping rules start afresh for the next, and the run
    /// moves on as [`Self::grow_run_next`] decides, `left` as there.
    fn leave_counter(
        &mut self,
        mut run: Run,
        now: Instant,
        cfg: &Growth,
        left: Option<&str>,
    ) -> bool {
        // Added to, not set: the run's count is for all its counters,
        // and the rules count one at a time.
        run.sold += self.autoplay.growth.shop.sold;
        self.close_vendor();
        self.autoplay.growth.shop = ac_vendor::Run::new();
        self.grow_run_next(run, now, cfg, left)
    }

    /// The run is done with this vendor: on to the next of the town
    /// when something is still wanted, else home. `left` says the
    /// vendor was no use -- what it did, as a predicate on its name:
    /// "would not trade", "buys none of this" -- and is to be avoided
    /// for a while; it is said in the status with where the run goes
    /// next, so that a counter walked past is a counter walked past
    /// for a reason. True while the run goes on.
    pub(super) fn grow_run_next(
        &mut self,
        run: Run,
        now: Instant,
        cfg: &Growth,
        left: Option<&str>,
    ) -> bool {
        self.window_no_longer_wanted(&run);
        if self.world.open_vendor.is_some() {
            self.close_vendor();
        }
        let left = left.map(|why| format!("{} {why}", run.vendor));
        if let Some(why) = left.as_deref() {
            self.autoplay.growth.skip_vendors.note(
                spot(run.at),
                &crate::did::Did::blocked(why),
                now,
            );
        }
        let needs = self.grow_needs(cfg);
        let still_full = self.pack_low_on_room();
        // A run in town goes on while there is loot for a counter left
        // and a counter in reach that takes it, whatever the run set
        // off for. This is how a counter that bought none of it is
        // walked past rather than home from, how the peas a general
        // store would not look at reach the archmage next door -- and
        // how a run made for arrows sells the peas at the archmage a
        // hundred metres on, rather than carrying them home and back
        // for them on a run of their own.
        let more_to_sell = !self.salables(cfg).is_empty();
        // And the other way about: a run made to sell went to the
        // counter that pays, not to the one with the list, so what is
        // wanted and can be paid for -- urgent or not -- is bought on
        // the way home. Ranked the buying way, the old run made those
        // top-ups in passing at its one counter.
        let more_to_buy = run.errand == Errand::Sell
            && needs.iter().any(|n| n.want > 0 && n.buyable)
            && self.spendable() > 0;
        // With no slot at all the next counter could not pay out either.
        let wanting = self.room_anywhere() > 0
            && (needs.iter().any(|n| n.urgent) || still_full || more_to_sell || more_to_buy);
        if wanting && run.stops < STOPS_PER_RUN {
            // The next counter is chosen by what is still on the list,
            // not by what is closest -- and only when there is reason to
            // think it can help. A full pack is reason enough on its
            // own: that stop is to empty it.
            //
            // It is looked for from every way out the character has, the
            // same as the first stop was, and not from the town this one
            // is standing in. A gem in the pack makes a counter on the
            // other side of the world two actions away, and judging the
            // second stop by how far it is from the first is what kept a
            // run inside one town.
            let me = self.my_position();
            let ways = match me {
                Some(p) => self.ways_out(Vec2::new(p.x, p.y)),
                None => vec![(run.town, "in town".to_string())],
            };
            // By the one rule every stop is chosen by (`errands_for`), from anywhere: a recall
            // makes the trip, and a second stop held to the town it was in walked past a counter
            // that took the loot.
            let errands = self.errands_for(cfg, &needs);
            let picked = self.choose_stop(cfg, &needs, &ways, &errands, &run.visited, now);
            if let Some((errand, vendor, at, look)) = picked {
                if (still_full || look.worth_going()) && self.grow_travel(at, now) {
                    let what = if still_full || more_to_sell {
                        "the rest of the loot"
                    } else {
                        "the rest"
                    };
                    let leaving = left
                        .as_deref()
                        .map_or(String::new(), |why| format!("{why}; "));
                    self.autoplay.say(
                        Doing::Shopping,
                        format!("{leaving}on to {vendor} for {what} -- {}", look.tell()),
                    );
                    let mut visited = run.visited;
                    visited.push(at);
                    self.autoplay.growth.run = Some(Run {
                        vendor,
                        at,
                        phase: Phase::Going,
                        since: now,
                        last_sell: None,
                        town: run.town,
                        stops: run.stops + 1,
                        sold: run.sold,
                        reason: run.reason,
                        errand,
                        visited,
                        walked_on: None,
                        walk_limit: self.planned_walk_limit(),
                    });
                    return true;
                }
                // Either no journey could be planned to it or it was
                // judged no help; either way, leave it alone for a
                // while rather than choose it again next frame.
                self.autoplay
                    .note(format!("skipping {vendor}: {}", look.tell()), now);
                self.autoplay.growth.skip_vendors.note(
                    spot(at),
                    &crate::did::Did::blocked("could not get there"),
                    now,
                );
            }
        }
        // A trip that neither bought nor sold anything achieved
        // nothing, and starting another at once achieves nothing again:
        // that is the running back and forth between vendors for ever.
        let futile = run.sold == 0 && !self.autoplay.growth.bought_anything;
        let st = &mut self.autoplay.growth;
        st.run_was_futile = futile;
        st.bought_anything = false;
        st.last_run = Some(now);
        st.run = None;
        st.needs.clear();
        st.skip_vendors.tidy(now);
        let by_hand = std::mem::take(&mut st.by_hand);
        let sold = run.sold;
        let full = if still_full { ", pack still full" } else { "" };
        let leaving = left.map_or(String::new(), |why| format!("; {why}"));
        self.autoplay.note(
            format!("town run done: sold {sold} item(s){full}{leaving}"),
            now,
        );
        // A run the panel asked for ends where the last counter was.
        // Whoever is watching it asked to see the shopping, not the
        // walk back to a hunting ground; and giving up in town is a
        // judgement about the hunting, which is autoplay's to make when
        // it is the one hunting.
        if by_hand {
            return false;
        }
        // The one thing worth giving up over, and this is what giving
        // up looks like: stay in town. Walking back to the hunting
        // ground without the supplies to work it would only mean
        // walking straight back again.
        if self.stranded(&needs, cfg) {
            let short: Vec<&str> = needs
                .iter()
                .filter(|n| n.urgent)
                .map(|n| n.name.as_str())
                .collect();
            let short = a_few(&short);
            self.autoplay.growth.stopped_in_town = true;
            self.autoplay.growth.stopped_looked = Some(now);
            self.autoplay.growth.bound = None;
            self.autoplay.growth.bound_since = None;
            self.autoplay.say(
                Doing::Shopping,
                format!("out of money and short of {short} -- stopping in town"),
            );
            return false;
        }
        // Back to the hunting ground.
        if let Some(lb) = self.autoplay.growth.hunting_at {
            if let Some(g) = ac_world::hunting::at(lb) {
                let (at, name) = (g.at, g.name.clone());
                if self.grow_travel(at, now) {
                    self.autoplay.growth.bound = Some((lb, at, name.clone()));
                    self.autoplay.growth.bound_since = Some(now);
                    self.autoplay
                        .say(Doing::Traveling, format!("back to hunt {name}"));
                    return true;
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests;
