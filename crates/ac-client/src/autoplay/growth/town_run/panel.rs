use std::time::{Duration, Instant};

use glam::Vec2;

use super::counter::counter_asked;
use super::vendor::Stop;
use super::{Driver, Errand, Phase, Run, Turn, VENDOR_OPEN_TIMEOUT, VENDOR_REACH};
use crate::autoplay::growth::about;
use crate::autoplay::Doing;
use crate::Client;

/// Who steps a run to town: the panel when it asked for the run, or
/// when autoplay is not running town runs and so would never step it
/// (a run left over from autoplay being switched off part-way); autoplay
/// otherwise.
fn driver(by_hand: bool, autoplay_drives: bool) -> Driver {
    if by_hand || !autoplay_drives {
        Driver::Hand
    } else {
        Driver::Autoplay
    }
}

/// How long the panel can leave a run before it counts as having let
/// go of it. The panel steps a run it is driving every frame, so a
/// second without a step is a run held on purpose -- a Step that has
/// had its act, or a panel closed on one.
const HAND_HOLD: Duration = Duration::from_secs(1);

/// Whether the panel has let go of the run it was driving: nothing has
/// stepped it for [`HAND_HOLD`]. A run left like that with autoplay
/// running town runs is autoplay's again, and with them off it stands
/// where it is and the status line says why. Without this a Step
/// pressed once and the panel closed stood the character at the counter
/// for good, autoplay claiming every tick for a run nothing was
/// stepping, with nothing on the status line to say so.
pub(crate) fn hand_has_let_go(stepped: Option<Instant>, now: Instant) -> bool {
    stepped.is_none_or(|t| now.saturating_duration_since(t) >= HAND_HOLD)
}

/// Whether a window opened for `vendor` is one a run stopped waiting
/// for: it was asked for at `asked` by a run that ended before it came,
/// and it has come within the time the run would have waited. Later
/// than that it is more likely the player's own doing, and is left be.
fn window_is_unwanted(unwanted: Option<(u32, Instant)>, vendor: u32, now: Instant) -> bool {
    unwanted.is_some_and(|(guid, asked)| {
        guid == vendor && now.saturating_duration_since(asked) <= VENDOR_OPEN_TIMEOUT
    })
}

/// A run to town as the vendoring panel shows it: read, not moved on.
#[derive(Clone, Debug, PartialEq)]
pub struct TownRunView {
    pub vendor: String,
    /// Why the run was made.
    pub reason: String,
    /// Which counter of the run this is, counting from one.
    pub stop: u32,
    pub phase: String,
    /// What the shopping rules would do next, once selling: asked of
    /// a copy of the rules, so the run itself does not move.
    pub next: Option<ac_vendor::Act>,
    /// How the run puts what it is doing or waiting on, whatever the
    /// phase.
    pub saying: String,
    /// Sold on the whole run, this counter included.
    pub sold: u32,
    /// Handed over at this counter and not yet answered for.
    pub waiting: usize,
    pub driver: Driver,
}

impl Client {
    /// Whether autoplay's tick is the thing that steps runs to town:
    /// autoplay on, with town runs on. When it is not, the vendoring
    /// panel is (see [`Driver`]).
    pub fn autoplay_drives_town_runs(&self) -> bool {
        self.autoplay.config.enabled && self.autoplay.config.growth.town_runs
    }

    /// Who steps the run to town in progress, if there is one.
    pub fn town_run_driver(&self) -> Option<Driver> {
        let st = &self.autoplay.growth;
        st.run.as_ref()?;
        Some(driver(st.by_hand, self.autoplay_drives_town_runs()))
    }

    /// Start a run to town now, from the vendoring panel: the same run
    /// autoplay makes -- the counter chosen the same way, the walk, the
    /// window, the appraisals, the selling and buying -- without
    /// autoplay's throttles, since the player asked for it. At a counter
    /// the player has already opened, the run is made there: that is
    /// the counter the panel has been showing the selling at, and
    /// walking off to another was the one case the old panel got right
    /// going wrong. Fine when a run is already under way: that run is
    /// the one the panel then shows, whoever is driving it, and a second
    /// is not started.
    ///
    /// Who steps it afterwards depends on whether autoplay runs town
    /// runs. When it does, the run is handed to it and goes at its
    /// pace; when it does not -- autoplay off, or town runs off -- the
    /// panel steps it ([`Self::town_run_step_by_hand`]) and autoplay
    /// keeps its hands off.
    pub fn town_run_by_hand(&mut self, now: Instant) -> Result<(), String> {
        if self.autoplay.growth.run.is_some() {
            // A run nobody else will step is the panel's now, and the
            // panel is about to step it.
            if self.town_run_driver() == Some(Driver::Hand) {
                self.autoplay.growth.by_hand = true;
                self.autoplay.growth.hand_stepped = Some(now);
            }
            return Ok(());
        }
        if self.attack_target.is_some() || self.autoplay.casting_at().is_some() {
            return Err("busy fighting".into());
        }
        // The same refusal autoplay makes before a run: no counter can
        // pay into a pack with no slot, and buying needs one too, so
        // the trip would sell nothing and count as futile afterwards.
        if self.room_anywhere() == 0 {
            return Err("no free slot for a counter's money".into());
        }
        // Whatever journey was under way, the run plans its own; and a
        // ground autoplay was bound for is let go, or autoplay would
        // hold the character to it over the run.
        if self.traveling() {
            self.cancel_travel();
        }
        self.autoplay.growth.bound = None;
        self.autoplay.growth.bound_since = None;
        let cfg = self.autoplay.config.growth.clone();
        let needs = self.needs_now(now, &cfg);
        let reason = "asked for from the panel".to_string();
        match self.open_counter_at_hand() {
            Some((guid, vendor, at)) => {
                // Opening with the window already open goes straight
                // to the appraising, as autoplay's own run does when
                // the window comes.
                let st = &mut self.autoplay.growth;
                st.needs = needs;
                st.window_unwanted = None;
                st.run = Some(Run {
                    vendor: vendor.clone(),
                    at,
                    phase: Phase::Opening {
                        guid,
                        tries: 1,
                        busy: None,
                        asked_over: 0,
                    },
                    since: now,
                    last_sell: None,
                    town: at,
                    stops: 1,
                    sold: 0,
                    reason: reason.clone(),
                    // This counter, whatever it takes: the player opened
                    // it, and a run made to sell would walk off from a
                    // counter that buys none of the pack.
                    errand: Errand::Buy,
                    visited: vec![at],
                    walked_on: None,
                    walk_limit: crate::autoplay::growth::road::WALK_TIMEOUT,
                });
                self.autoplay
                    .say(Doing::Shopping, format!("{reason}: at {vendor}"));
            }
            None => {
                // The button is pressed to see the shopping happen: with
                // anything on the list the run goes where the list is
                // and the loot goes along as a tiebreak (and to a second
                // counter after, see [`Self::grow_run_next`]); with
                // nothing to buy and something to sell it goes to the
                // counter that pays, however far. Made to sell
                // whenever the pack held a pea, the run walked past the
                // bowyer with the arrows on the list and came home
                // short of them.
                let to_buy = needs.iter().any(|n| n.want > 0 && n.buyable);
                let errand = if !to_buy && !self.salables(&cfg).is_empty() {
                    Errand::Sell
                } else {
                    Errand::Buy
                };
                let first = Stop {
                    errand,
                    within: None,
                    visited: &[],
                };
                let started =
                    self.start_town_run(now, &cfg, needs.clone(), reason.clone(), first, true);
                match (started, errand) {
                    // Nobody buys what is carried: the player still
                    // asked for a run, and a run to buy has a counter to
                    // fall back on -- the nearest.
                    (Err(_), Errand::Sell) => {
                        let any = Stop {
                            errand: Errand::Buy,
                            within: None,
                            visited: &[],
                        };
                        self.start_town_run(now, &cfg, needs, reason, any, true)?
                    }
                    (started, _) => started?,
                }
            }
        }
        let by_hand = !self.autoplay_drives_town_runs();
        self.autoplay.growth.by_hand = by_hand;
        self.autoplay.growth.hand_stepped = by_hand.then_some(now);
        Ok(())
    }

    /// The counter whose window the player has open, when the
    /// character is still near enough to trade at it: its guid, its
    /// name and where it stands. A window left open from across the
    /// town is not a counter at hand.
    fn open_counter_at_hand(&self) -> Option<(u32, String, Vec2)> {
        let guid = self.world.open_vendor.as_ref()?.vendor;
        let (name, at) = self.counter_in_reach(guid)?;
        Some((guid, name, at))
    }

    /// The counter `guid` when the character is still near enough to
    /// trade at it: its name and where it stands.
    fn counter_in_reach(&self, guid: u32) -> Option<(String, Vec2)> {
        let me = self.my_position()?;
        let o = self.world.objects.get(&guid)?;
        let p = o.world_pos()?;
        let at = Vec2::new(p.x, p.y);
        (at.distance(Vec2::new(me.x, me.y)) <= VENDOR_REACH).then(|| (o.name.clone(), at))
    }

    /// The counter the character stands at, by name: one it has asked
    /// for its window (see [`counter_asked`]) and is still near enough
    /// to trade at. The reach is what makes it a counter *at hand*. A
    /// run remembers its counter through a death, and a window is ours
    /// and closes only when we close it, so read off those alone a
    /// character waking at the lifestone, or one that walked off from
    /// a window it opened by hand, stood at a counter for the rest of
    /// the session -- and every buff waited for it.
    pub(crate) fn counter_at_hand(&self) -> Option<String> {
        let phase = self.autoplay.growth.run.as_ref().map(|r| &r.phase);
        let window = self.world.open_vendor.as_ref().map(|v| v.vendor);
        let guid = counter_asked(phase, window)?;
        self.counter_in_reach(guid).map(|(name, _)| name)
    }

    /// The character has died at a counter: it wakes at the lifestone,
    /// and the visit is over. A run past its walk is stopped -- it
    /// cannot go on from there, and autoplay sets off again when the
    /// need still stands -- and a window, the run's or the player's
    /// own, is closed: the server keeps none and a dead character
    /// trades at nothing. Left as they were, the run stood at its
    /// counter through the death and the buffs waited for it, while
    /// recovery, which puts the buffs back before the walk to the
    /// corpse, waited on the buffs.
    pub(crate) fn counter_left_behind(&mut self, now: Instant) {
        let past_the_walk = self
            .autoplay
            .growth
            .run
            .as_ref()
            .is_some_and(|r| !matches!(r.phase, Phase::Going));
        if past_the_walk {
            self.town_run_stop(now);
        }
        if self.world.open_vendor.is_some() {
            self.close_vendor();
        }
    }

    /// One turn of the run in progress, from the vendoring panel: the
    /// same turn autoplay's tick takes, with the same waits -- on the
    /// walk, the window, the appraisals, and the counter's answer to the
    /// last thing it was handed. Call it once a frame; the panel's Step
    /// calls it until a turn comes to an act and then holds (see
    /// [`Turn`]).
    ///
    /// The run is the panel's from here on, whoever started it.
    pub fn town_run_step_by_hand(&mut self, now: Instant) -> Turn {
        if self.autoplay.growth.run.is_none() {
            return Turn::Over;
        }
        self.autoplay.growth.by_hand = true;
        self.autoplay.growth.hand_stepped = Some(now);
        let cfg = self.autoplay.config.growth.clone();
        self.grow_run_step(now, &cfg)
    }

    /// End the run to town in progress and leave the character as it
    /// stands: the counter closed, the walk ended, nothing left waiting
    /// for a next turn. Autoplay waits its usual while before starting
    /// another, as after any run.
    pub fn town_run_stop(&mut self, now: Instant) {
        let st = &mut self.autoplay.growth;
        let Some(run) = st.run.take() else {
            return;
        };
        st.by_hand = false;
        st.needs.clear();
        st.after_out = None;
        st.shop = ac_vendor::Run::new();
        st.last_run = Some(now);
        self.window_no_longer_wanted(&run);
        if self.world.open_vendor.is_some() {
            self.close_vendor();
        }
        self.cancel_travel();
        // The last few metres to a counter are a walk after it, not a
        // journey (see `do_vendor_act`).
        if self.follow.take().is_some() {
            self.steering.reset();
        }
        self.autoplay
            .note(format!("the run to {} was stopped", run.vendor), now);
    }

    /// The run is over while its counter was still being asked for its
    /// window. A window that comes now is nobody's: remember which, so
    /// that it is closed when it does (see [`window_is_unwanted`]).
    pub(crate) fn window_no_longer_wanted(&mut self, run: &Run) {
        if let Phase::Opening { guid, .. } = run.phase {
            if self.world.open_vendor.is_none() {
                self.autoplay.growth.window_unwanted = Some((guid, run.since));
            }
        }
    }

    /// Close a counter's window that opened for a run already over.
    /// Once a frame, whether or not autoplay is on: Stop from the panel
    /// is the usual way a run ends mid-opening, and that is with
    /// autoplay off.
    pub(crate) fn autoplay_close_unwanted_window(&mut self, now: Instant) {
        let unwanted = self.autoplay.growth.window_unwanted;
        let Some((_, asked)) = unwanted else {
            return;
        };
        let open = self.world.open_vendor.as_ref().map(|v| v.vendor);
        match open {
            Some(vendor) if window_is_unwanted(unwanted, vendor, now) => {
                self.autoplay.growth.window_unwanted = None;
                self.close_vendor();
                self.autoplay.note(
                    "closed a counter's window that opened after the run to it ended",
                    now,
                );
            }
            // Not coming: whatever opens from now on is the player's.
            _ if now.saturating_duration_since(asked) > VENDOR_OPEN_TIMEOUT => {
                self.autoplay.growth.window_unwanted = None;
            }
            _ => {}
        }
    }

    /// The run to town in progress as the vendoring panel shows it,
    /// read without moving it on: the phase, and what would happen
    /// next. Once selling, the next act is asked of a copy of the
    /// shopping rules, so that asking does not hand anything over.
    pub fn town_run_view(&self, now: Instant) -> Option<TownRunView> {
        let st = &self.autoplay.growth;
        let run = st.run.as_ref()?;
        let me = self.my_position().map(|p| Vec2::new(p.x, p.y));
        let (phase, next, saying) = match &run.phase {
            Phase::Going => {
                let away = me.map(|me| format!(" ({})", about(run.at.distance(me))));
                (
                    "Going",
                    None,
                    format!("going to {}{}", run.vendor, away.unwrap_or_default()),
                )
            }
            Phase::Opening {
                tries, busy: None, ..
            } => (
                "Opening",
                None,
                format!(
                    "waiting for {} to open its window (try {tries})",
                    run.vendor
                ),
            ),
            Phase::Opening { busy: Some(_), .. } => (
                "Opening",
                None,
                format!(
                    "{} turned the Use away while a cast was in the air; asking again once it lands",
                    run.vendor
                ),
            ),
            Phase::Appraising => (
                "Appraising",
                None,
                "looking over the pack, then putting stacks together".to_string(),
            ),
            Phase::Selling { .. } => {
                let cfg = &self.autoplay.config.growth;
                let snap = self.vendor_snapshot(cfg);
                let mut asking = st.shop.clone();
                let peek = asking.step(&snap, now);
                ("Selling", peek.act, peek.saying)
            }
        };
        Some(TownRunView {
            vendor: run.vendor.clone(),
            reason: run.reason.clone(),
            stop: run.stops,
            phase: phase.to_string(),
            next,
            saying,
            sold: run.sold + st.shop.sold,
            waiting: st.shop.waiting_on(),
            driver: driver(st.by_hand, self.autoplay_drives_town_runs()),
        })
    }
}

#[cfg(test)]
mod tests;
