pub(super) mod counter;
pub(super) mod panel;
pub(super) mod vendor;

use std::time::{Duration, Instant};

use glam::Vec2;

#[cfg(doc)]
use self::counter::on_opening;
use self::vendor::{nearest_way, spot, Forecast, Stop, SALE_RUN_REACH};
use crate::autoplay::growth::needs::Need;
use crate::autoplay::growth::policy::restocks_as_a_party;
#[cfg(doc)]
use crate::autoplay::growth::road::on_the_way;
use crate::autoplay::growth::road::RETRY_AFTER;
use crate::autoplay::growth::sale::worth_a_sale_run;
use crate::autoplay::growth::{a_few, about, Growth};
use crate::autoplay::Doing;
use crate::logistics::Stage;
use crate::Client;

/// Standing this close to a vendor is close enough to use it: the
/// server walks the character the last stretch itself.
pub(super) const VENDOR_REACH: f32 = 35.0;

/// How close to stand before asking a counter to open. A vendor will
/// not trade with somebody across the room, and says so by telling the
/// character to walk there rather than by refusing.
pub(crate) const COUNTER_REACH: f32 = 3.0;

/// A vendor that does not answer a Use in this long is tried once more,
/// then left.
const VENDOR_OPEN_TIMEOUT: Duration = Duration::from_secs(12);

/// How long after a counter turned the Use away as busy it is asked
/// over, once nothing of ours is in flight. ACE is busy with a cast
/// for its recoil only -- `IsBusy` is set in `FinishCast` and cleared
/// when the Ready motion ends, about a second later (`Player_Magic.cs`)
/// -- and this comfortably outlasts one. The cast slot alone cannot be
/// trusted for it: ACE answers the turned-away Use with a UseDone of
/// its own (`Player_Use.cs`, `TryUseItem`), and that frees the slot
/// here while the recoil that made the character busy is still
/// running.
const BUSY_REASK: Duration = Duration::from_secs(3);

/// How many times a Use turned away as busy is sent over before the
/// refusals are taken for the counter's own. A counter that keeps
/// saying busy with nothing of ours in flight is busy with something
/// this client does not track, and is left to the ordinary timeout.
pub(super) const BUSY_ASKS: u32 = 3;

/// How long to wait for the server to take the items sold, the
/// appraisals to come back, or the purchases to arrive.
const SETTLE: Duration = Duration::from_secs(5);

/// How long to leave it after a trip to town that bought and sold
/// nothing. Long enough that a character which cannot afford what it
/// needs goes back to earning instead of shuttling between counters.
const FUTILE_RUN_WAIT: Duration = Duration::from_secs(300);

/// How often a character that has stopped in town looks to see whether
/// its luck has changed.
const STOPPED_LOOK_EVERY: Duration = Duration::from_secs(5);

/// Least time between two town runs. A run that could sell nothing
/// leaves the pack as full as it found it, and the next is not until
/// this has passed.
const RUN_EVERY: Duration = Duration::from_secs(8 * 60);

/// Most vendors visited in one run.
const STOPS_PER_RUN: u32 = 3;

/// What a run to town is for, which decides which counter it goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Errand {
    /// To buy what the character is short of. A counter is chosen on
    /// what it stocks; what it pays for the loot carried along is a
    /// tiebreak, and with nothing on the list the nearest counter will
    /// do.
    Buy,
    /// To sell what is carried: a full or heavy pack, or loot tagged for
    /// a counter. A counter is chosen on what it pays for that, and one
    /// that buys none of it is never chosen -- that was the walk to a
    /// tailor with a pack of peas, and home again with the peas.
    Sell,
}

impl Errand {
    /// Whether a trip with this forecast does what the errand is for.
    fn served_by(self, look: &Forecast) -> bool {
        match self {
            Errand::Buy => look.worth_going(),
            Errand::Sell => look.selling > 0,
        }
    }
}

/// Where a town run stands.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Phase {
    /// Walking to the vendor.
    Going,
    /// The vendor was used; waiting for its stock. `busy` is when the
    /// counter last turned the Use away for our own doing -- a cast in
    /// the air, not the counter refusing -- and `asked_over` how many
    /// times the Use has gone out again for that (see [`on_opening`]).
    Opening {
        guid: u32,
        tries: u32,
        busy: Option<Instant>,
        asked_over: u32,
    },
    /// The pack's items are being appraised before the sale.
    Appraising,
    /// Selling. `sent` is what has gone over the counter and not yet
    /// left the pack -- nothing else is remembered, because what there
    /// is to sell is asked of the pack afresh every turn.
    Selling { sent: Vec<u32> },
}

/// A run to town in progress.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Run {
    pub(super) vendor: String,
    pub(super) at: Vec2,
    pub(super) phase: Phase,
    /// When the phase began.
    pub(super) since: Instant,
    pub(super) last_sell: Option<Instant>,
    /// Where the run started. Later stops are looked for from every way
    /// out the character has; this is only the fallback for when its
    /// own position is not known.
    pub(super) town: Vec2,
    pub(super) stops: u32,
    /// How many items were sold at all the stops so far.
    pub(super) sold: u32,
    /// Why the run was made.
    pub(super) reason: String,
    /// What it is for: to sell, or to buy (see [`Errand`]).
    pub(super) errand: Errand,
    /// The vendors already called on this run, by position.
    pub(super) visited: Vec<Vec2>,
    /// When the walk to this counter was last planned again after
    /// something broke it off (see [`on_the_way`]).
    pub(super) walked_on: Option<Instant>,
}

/// What one turn of a run to town came to.
///
/// Autoplay only asks whether the run goes on. The vendoring panel's
/// Step asks more: it carries the run on until one act has gone out
/// and then holds, so a trip can be read a line at a time. An act is
/// something sent to the world -- a walk begun or planned again, a
/// counter used, the pack put up for appraisal, a pour, an armful, a
/// purchase -- or a walk on to the next counter. Waiting on the walk,
/// on the window, on the appraisals, on the counter's answer to the
/// last thing it was handed, and moving from one phase to the next
/// without sending anything, are not: a Step that lands on a wait
/// waits it out, and takes the act that follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turn {
    /// Something went out.
    Acted,
    /// Nothing did; the world has not answered yet.
    Waited,
    /// The run is over.
    Over,
}

impl Turn {
    /// True while the run goes on, which is all autoplay's tick asks.
    pub fn goes_on(self) -> bool {
        self != Turn::Over
    }

    /// What leaving a counter came to: a walk to the next one begun, or
    /// the run over (see [`Client::grow_run_next`]).
    fn after_stop(goes_on: bool) -> Turn {
        if goes_on {
            Turn::Acted
        } else {
            Turn::Over
        }
    }
}

/// Who steps the run to town in progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Driver {
    /// Autoplay's own tick, at its own pace.
    Autoplay,
    /// The vendoring panel, one act or one frame at a time.
    Hand,
}

/// How long after one run the next waits: `party_restocking` when the
/// party has agreed to shop, `futile` when the last run bought and sold
/// nothing -- which a run that never reached its counter always has.
fn wait_between_runs(party_restocking: bool, futile: bool) -> Duration {
    if !party_restocking {
        RUN_EVERY
    } else if futile {
        FUTILE_RUN_WAIT
    } else {
        Duration::ZERO
    }
}

impl Client {
    /// Whether the character is stuck: short of something it needs to
    /// go on hunting, with no money to buy it and nothing left to sell.
    ///
    /// This is the one thing it gives up over, and giving up means
    /// stopping in town rather than going back to the hunt: without
    /// supplies there is nothing to earn out there, and the shops,
    /// the party and the player are all here.
    fn stranded(&self, needs: &[Need], cfg: &Growth) -> bool {
        needs.iter().any(|n| n.urgent) && self.spendable() == 0 && self.salables(cfg).is_empty()
    }

    /// Say, once, why no run to town is being started, and answer that
    /// none is. There is nothing to see when a character stands about
    /// doing nothing, so this is how it explains itself.
    fn held_back(&mut self, why: impl Into<String>) -> bool {
        let why = why.into();
        if self.autoplay.growth.held_back != why {
            tracing::debug!("no town run: {why}");
            self.autoplay.growth.held_back = why;
        }
        false
    }

    /// Start a run when one is due, or carry the current one on. True
    /// while a run is in progress.
    pub(super) fn grow_town_run(&mut self, now: Instant, cfg: &Growth) -> bool {
        if self.autoplay.growth.run.is_some() {
            // A run the panel is driving never gets here: it keeps the
            // tick in [`Client::autoplay_grow`] without being stepped.
            return self.grow_run_step(now, cfg).goes_on();
        }
        // Stopped in town for want of money: it stays put. Something
        // has to change from outside -- coin or goods handed over, a
        // quartermaster's delivery -- and when it does it picks the
        // shopping straight back up.
        if self.autoplay.growth.stopped_in_town {
            if self
                .autoplay
                .growth
                .stopped_looked
                .is_some_and(|t| now.duration_since(t) < STOPPED_LOOK_EVERY)
            {
                return false;
            }
            self.autoplay.growth.stopped_looked = Some(now);
            let needs = self.needs_now(now, cfg);
            if self.stranded(&needs, cfg) {
                return false;
            }
            let st = &mut self.autoplay.growth;
            st.stopped_in_town = false;
            st.stopped_looked = None;
            st.run_was_futile = false;
            st.last_run = None;
            self.autoplay
                .note("something to spend again: shopping", now);
        }
        // Too laden to be handed anything: a counter cannot help, so
        // it does not go to one. Selling, using or handing something
        // over is what lifts this, and all three happen elsewhere.
        if self.autoplay.growth.too_heavy {
            let needs = self.needs_now(now, cfg);
            let room = self.burden_room();
            // Room enough for the lightest thing it still wants is
            // room enough to be worth the walk.
            if room == 0 && !needs.is_empty() {
                let (carrying, capacity) = self.burden();
                return self.held_back(format!(
                    "too heavy to buy anything ({carrying} of {capacity}, three times over)"
                ));
            }
            self.autoplay.growth.too_heavy = false;
        }
        // Not while anything else is going on.
        if self.attack_target.is_some()
            || self.autoplay.casting_at().is_some()
            || self.traveling()
            || self.autoplay.growth.bound.is_some()
        {
            return self.held_back("busy with something else");
        }
        // The party has already decided to shop, so the throttle that
        // stops a lone character wearing a path to the vendor does not
        // apply: it would leave the rest waiting at the hunting ground
        // for nothing. The retry delay still holds, since it means a
        // vendor could not be reached at all.
        // The party having agreed to shop lifts the throttle that stops
        // a lone character wearing a path to the vendor -- but not when
        // the last trip came back with nothing. Without that a party
        // that cannot buy what it needs walks between counters for ever.
        let party_restocking = !self.autoplay.growth.mode.hunting();
        let wait = wait_between_runs(party_restocking, self.autoplay.growth.run_was_futile);
        if let Some(t) = self
            .autoplay
            .growth
            .last_run
            .filter(|t| now.duration_since(*t) < wait)
        {
            let left = wait.saturating_sub(now.duration_since(t));
            return self.held_back(format!("{} s to wait since the last run", left.as_secs()));
        }
        if self.autoplay.growth.next_run.is_some_and(|t| now < t) {
            return self.held_back("waiting to try a vendor again");
        }
        // Low on room, not out of it: a run that waited for the last slot
        // arrived with nowhere for the money to go.
        let full = self.pack_low_on_room();
        if self.room_anywhere() == 0 {
            // No counter anywhere can pay out into a pack with no slot, and
            // buying needs one too: walking to one achieves nothing.
            self.autoplay.note(
                "no free slot in the pack: a counter has nowhere to put the money, \
                 so a slot has to be freed before anything can be sold",
                now,
            );
            return self.held_back("no free slot for a counter's money");
        }
        // A pack runs out of room two ways, and weight is the one that
        // creeps up unnoticed: slots stay free while the character
        // grows too heavy to lift anything, tidy anything, or move at
        // any speed. It is as good a reason to go and sell as a pack
        // with no slots left.
        //
        // Laden means the loot has filled the working limit, or has taken
        // the character to twice its capacity, or the server's wall
        // leaves no room for any more -- not merely that the character
        // is heavy. What it wears and keeps does not count towards the
        // limit: a counter cannot lighten that, and counting it sent a
        // character whose own gear filled the limit off to sell with
        // nothing to sell.
        let laden = self.laden(cfg);
        let needs = self.needs_now(now, cfg);
        // What the loot rules tagged for a counter, and since when. The
        // ledger says why each thing was taken, and "to sell" is a
        // reason to go and sell it: without this the list was read only
        // once a counter was open, and a pack with room in it never
        // got one open.
        let salables = self.salables(cfg);
        let st = &mut self.autoplay.growth;
        if salables.is_empty() {
            st.sale_since = None;
        } else if st.sale_since.is_none() {
            st.sale_since = Some(now);
        }
        let carried_for = st.sale_since.map(|t| now.duration_since(t));
        // On a team that restocks together, the party's mode decides:
        // one character does not walk off to a vendor while the rest
        // are fighting, and none of them stays behind when the party
        // has agreed to go. Alone, the older rule stands -- something
        // urgent, a pack with no room left, as much loot as it means
        // to carry, or loot enough tagged for a counter. Alone includes
        // a character set to restock together with nobody else on the
        // team, which is how Blargerton never went to sell.
        let party_mode = self.autoplay.growth.mode;
        let together =
            restocks_as_a_party(&self.autoplay.config.team, self.autoplay.team.mates.len());
        let urgent: Vec<&Need> = needs.iter().filter(|n| n.urgent).collect();
        let sale = worth_a_sale_run(&salables, carried_for, cfg);
        // What this character's own pack makes the first stop for,
        // whatever the party decided. A party's trip is to restock,
        // but a member whose pack is full of peas goes to the counter
        // that buys them and shops after (see [`Self::grow_run_next`]);
        // ranked the buying way its first stop was the tailor with the
        // list, and the peas came home.
        let own_errand = if full || laden || sale.is_some() {
            Errand::Sell
        } else {
            Errand::Buy
        };
        // The reason, the errand, and how far the first counter may be
        // from a way out. A pack that cannot hunt on -- full, heavy,
        // or a supply run out -- is worth a walk anywhere; loot that
        // merely adds up is worth the town the character is in, and no
        // further. Without a reach one Lead Pea carried a quarter of an
        // hour was a walk to an archmage three towns over, and the
        // same again for the next pea.
        let (reason, errand, within) = if together {
            match party_mode.stage() {
                None => return self.held_back("the party is hunting"),
                // Only the runner walks to town; the rest hold their
                // place at the hunting ground and wait for it.
                Some(Stage::HandOver | Stage::Away | Stage::HandOut)
                    if self.autoplay.config.team.restock.plan
                        == crate::logistics::Plan::Quartermaster
                        && !self.is_quartermaster(cfg, now) =>
                {
                    return self.held_back("the quartermaster is doing the shopping")
                }
                Some(_) => {
                    let because = self.autoplay.growth.mode_because.clone();
                    let reason = if because.is_empty() {
                        "the party is restocking".to_string()
                    } else {
                        because
                    };
                    (reason, own_errand, None)
                }
            }
        } else if full {
            ("the pack is full".to_string(), Errand::Sell, None)
        } else if laden {
            (
                "carrying as much as it means to".to_string(),
                Errand::Sell,
                None,
            )
        } else if !urgent.is_empty() {
            // A supply run out comes before loot that adds up: the
            // counter with the arrows may stand further from a way out
            // than a second stop is allowed, and the loot is sold on
            // the way at whichever counter takes it (see
            // [`Self::grow_run_next`]).
            let short: Vec<&str> = urgent
                .iter()
                .filter(|n| n.buyable)
                .map(|n| n.name.as_str())
                .collect();
            (format!("short of {}", a_few(&short)), Errand::Buy, None)
        } else if let Some(why) = sale {
            (why, Errand::Sell, Some(SALE_RUN_REACH))
        } else {
            let short: Vec<&str> = needs.iter().map(|n| n.name.as_str()).collect();
            let sale = match salables.len() {
                0 => String::new(),
                n => format!("; {n} thing(s) for a counter, not yet worth the trip"),
            };
            return self.held_back(format!("nothing urgent (short of {}{sale})", a_few(&short)));
        };
        // A pack that is full or heavy with nothing in it a counter
        // takes is not emptied by any counter. The trip is then
        // whatever one can do for it -- and if that is nothing, town is
        // where the character stops (see [`Self::stranded`]).
        let errand = if salables.is_empty() {
            Errand::Buy
        } else {
            errand
        };
        // Know before setting off whether the trip can achieve anything.
        // A counter with nothing the character needs, or nothing it can
        // pay for, is a walk to town and back for its own sake -- and
        // repeated, it is the party pacing between vendors for ever.
        //
        // Two reasons override it. A full pack is its own errand: that
        // trip is to empty it, not to buy. And a character with nothing
        // to spend and nothing to sell goes anyway, because town is
        // where it stops: see [`Self::stranded`].
        let go_anyway = full || self.stranded(&needs, cfg);
        let first = Stop {
            errand,
            within,
            visited: &[],
        };
        match self.start_town_run(now, cfg, needs, reason, first, go_anyway) {
            Ok(()) => true,
            Err(why) => self.held_back(why),
        }
    }

    /// Set off for a counter: choose one, plan the walk and open the
    /// run. `first` is what the first counter is chosen for and how far
    /// from a way out it may stand; `go_anyway` takes the trip whether
    /// or not the forecast says it is worth making. The throttles --
    /// how long since the last run, whether anything is urgent, whether
    /// the party agrees -- are the caller's: autoplay's tick applies
    /// them ([`Self::grow_town_run`]) and the vendoring panel does not
    /// ([`Self::town_run_by_hand`]).
    fn start_town_run(
        &mut self,
        now: Instant,
        cfg: &Growth,
        needs: Vec<Need>,
        reason: String,
        first: Stop<'_>,
        go_anyway: bool,
    ) -> Result<(), String> {
        let Stop { errand, within, .. } = first;
        let Some(me) = self.player.as_ref().map(|p| p.world_position()) else {
            return Err("not placed in the world yet".into());
        };
        let me = Vec2::new(me.x, me.y);
        // Underground, "nearest" is measured from where the character
        // will come up, not from where it is standing.
        //
        // A dungeon lies under the landblock it belongs to, so the
        // counter nearest a character in Holtburg Dungeon is one in
        // Holtburg -- a hundred metres away and a hundred metres of
        // rock in between. The way out is a recall, and a recall lands
        // somewhere particular; the shops worth considering are the
        // ones near *that*.
        let ways = self.ways_out(me);
        let Some((vendor, at, look)) = self.pick_vendor(cfg, &needs, &ways, first, now) else {
            let why = match (errand, within) {
                (Errand::Buy, _) => "no vendor to run to".to_string(),
                // Said with the count, so that whoever is watching can
                // tell "nothing to sell" from "nothing anyone buys" --
                // and "nobody near enough" from either.
                (Errand::Sell, Some(reach)) => format!(
                    "nobody within {reach:.0} m of a way out buys any of the {} thing(s) for sale",
                    self.salables(cfg).len()
                ),
                (Errand::Sell, None) => format!(
                    "no counter buys any of the {} thing(s) for sale",
                    self.salables(cfg).len()
                ),
            };
            self.autoplay.note(why.clone(), now);
            self.autoplay.growth.last_run = Some(now);
            return Err(why);
        };
        // Why this counter and not another, in the log. The choice is a
        // ring search outwards from `from`, taking the best shop in the
        // first ring with anything worth the walk, so the answer is
        // always some mixture of what it stocks and how far off it is.
        self.autoplay.note(
            format!(
                "{vendor} it is: {}, {:.0} m by {}",
                look.why_it_was_picked(),
                nearest_way(&ways, at).1,
                nearest_way(&ways, at).2
            ),
            now,
        );
        if !go_anyway && !look.worth_going() {
            let why = format!("not worth a trip to {vendor}: {}", look.tell());
            self.autoplay.note(why.clone(), now);
            let st = &mut self.autoplay.growth;
            st.last_run = Some(now);
            // Nothing to buy and nothing to sell is exactly what a
            // futile run comes home with, and the party reads it the
            // same way: this one is broke, carry on without it.
            st.run_was_futile = true;
            return Err(why);
        }
        if !self.grow_travel(at, now) {
            // Not the next one straight away: a character somewhere no
            // journey can start from would try every vendor in the
            // world, one a frame.
            let st = &mut self.autoplay.growth;
            st.skip_vendors.note(
                spot(at),
                &crate::did::Did::blocked("no way there from here"),
                now,
            );
            st.next_run = Some(now + RETRY_AFTER);
            let why = format!("no way to {vendor} from here");
            self.autoplay.note(why.clone(), now);
            return Err(why);
        }
        let st = &mut self.autoplay.growth;
        st.needs = needs;
        st.by_hand = false;
        st.window_unwanted = None;
        // The loot for a counter is on its way to one: what comes home
        // unsold is counted from when it is next seen.
        st.sale_since = None;
        st.run = Some(Run {
            vendor: vendor.clone(),
            at,
            phase: Phase::Going,
            since: now,
            last_sell: None,
            town: at,
            stops: 1,
            sold: 0,
            reason: reason.clone(),
            errand,
            visited: vec![at],
            walked_on: None,
        });
        self.autoplay.say(
            Doing::Shopping,
            format!(
                "{reason}: going to {vendor} ({}) -- {}",
                about(at.distance(me)),
                look.tell()
            ),
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests;
