//! Keeping a character that plays on its own going for hours: the
//! rules of `crate::autoplay` fight, loot and buff, and these make sure
//! there is always something worth fighting, a pack to put the loot in,
//! and a character that grows with the experience it earns.
//!
//! Three rules. The first runs every tick as housekeeping (see
//! `Client::autoplay_spend_xp`); the other two when nothing more
//! pressing is going on (see [`Client::autoplay_grow`]):
//!
//! 1. **Spend experience.** The unassigned pool is spent the way a
//!    player would: the skills it fights with first (the weapon in hand,
//!    the defences, the magic it casts), the attributes those skills are
//!    built from next, then the rest. Each candidate rank is priced from
//!    the XpTable and weighed by how much it matters, and the best value
//!    is bought -- a rank to a message for what a kill brings in, and for
//!    a large pool as many ranks of it as buying a rank at a time would
//!    have given it, so the pool is spread as far in a few messages. When
//!    the best buy costs more than the pool holds, the pool saves for it
//!    rather than going on a worse buy that fits. One message at a time,
//!    no more often than the server can answer, in the middle of a walk or
//!    a fight -- except a rank that raises a maximum, which waits for the
//!    fight to be over with its share of the pool kept for it.
//! 2. **Go where the monsters are.** When nothing has been worth fighting
//!    for a while, a hunting ground that suits the character's level is
//!    picked from `ac_world::hunting` (the nearest, leaving out the one
//!    just hunted out and any that could not be reached) and travelled
//!    to. Fights on the way are fought; the journey is picked up again
//!    after each.
//! 3. **Run to town.** When the pack is full, or something the character
//!    lives on is short and cannot be made from what it carries
//!    (healing kits, arrows, spell components), it walks to the nearest
//!    vendor, sells the loot it has no use for, buys what it is short of,
//!    and goes back to its hunting ground. A run visits up to a few
//!    vendors of the same town, since no one shop buys everything or
//!    sells everything.
//!
//! What is sold is decided by searches in the inventory's own language,
//! the way loot is chosen (`Growth::sell`), with the things a character
//! cannot do without kept back whatever the searches say: what it
//! wears and wields, money, packs, components, ammunition, tools, the
//! items it keeps stocked, and weapons it could wield. A weapon it
//! cannot wield is loot.

use std::time::Instant;

use ac_agent::recent::Recent;
use glam::Vec2;

use crate::autoplay::Doing;
use crate::Client;

pub(crate) mod burden;
mod config;
mod hunt;
mod needs;
mod policy;
mod raise;
mod road;
mod sale;
mod supplies;
mod town_run;
mod xp;

pub use config::Growth;
use needs::Need;
use raise::Pending;
pub use raise::{batch_raise, choose_raise, Batch, Climb, Offer, Raise};
#[cfg(doc)]
use road::on_the_way;
#[cfg(doc)]
use sale::worth_a_sale_run;
pub use sale::Salable;
#[cfg(doc)]
use town_run::counter::on_opening;
use town_run::panel::hand_has_let_go;
pub use town_run::panel::TownRunView;
pub use town_run::vendor::Forecast;
pub(crate) use town_run::COUNTER_REACH;
pub use town_run::{Driver, Turn};
use town_run::{Phase, Run, BUSY_ASKS};

/// A distance for a status line, in steps of fifty metres, so the line
/// changes (and is logged) now and then rather than every frame.
fn about(distance: f32) -> String {
    format!("{} m", ((distance / 50.0).ceil() * 50.0) as u32)
}

/// The running state of the growth rules.
#[derive(Default)]
pub struct State {
    /// What the party is doing, as this session last worked it out from
    /// the roster. Every session reaches the same answer, so this is a
    /// cache of a shared decision rather than a vote of its own.
    pub mode: crate::logistics::GroupMode,
    /// When the mode last changed, and why, for the log and the UI.
    pub mode_since: Option<Instant>,
    pub mode_because: String,
    /// This character has given the quartermaster its sale loot and its
    /// order. Cleared whenever the mode changes.
    pub handed_over: bool,
    /// Something was actually bought on the run in progress.
    pub bought_anything: bool,
    /// How many trips the quartermaster has made this time out.
    pub round: u32,
    /// The character came home from a counter too laden to be handed
    /// anything. Nothing at a shop changes that, so it does not go
    /// back until it has sold or used something.
    pub too_heavy: bool,
    /// Stacks the server would not join, so the tidying does not ask
    /// for ever. Prismatic Tapers at a hundred and two hundred should
    /// merge and the asking should stop when they do not.
    pub(super) wont_merge: crate::did::Patience<(u32, u32)>,
    /// The shopping, as decided by `ac-vendor`. It holds what a
    /// snapshot cannot show: which phase the trip is in, what has been
    /// handed over and not yet answered for, and what has been refused.
    pub shop: ac_vendor::Run,
    /// The last thing the shopping rules said they were doing, for the
    /// log and the panel.
    pub last_saying: String,
    /// The character is underground. Worked out on the tick (it needs
    /// the block's collision, which is loaded lazily) and read by the
    /// rules that must not plan a walk out of a dungeon.
    pub in_dungeon: bool,
    /// The last run to town ended without buying or selling anything.
    /// Another one straight away would do the same, so it waits.
    pub run_was_futile: bool,
    /// The character has given up, in town. Out of money, with nothing
    /// left to sell and too short of supplies to go on hunting, it
    /// stops where the shops are -- which is where the money and the
    /// goods are, and where whoever is watching can put it right --
    /// rather than walk back to a hunting ground it cannot work.
    pub stopped_in_town: bool,
    /// When it last looked to see whether that had changed. Counting
    /// the pack is not free and it is standing still.
    stopped_looked: Option<Instant>,
    /// Why no run to town was started, as last logged. A run is decided
    /// once a frame and the answer is usually the same one; this keeps
    /// the log to the moments it changes.
    held_back: String,
    /// Since when the pack has held loot for a counter, `None` while
    /// it holds none -- the clock the patience in [`worth_a_sale_run`]
    /// reads. Read on the frames a run could start, so it starts once
    /// the waits after the last run are up, and it is put back when a
    /// run sets off: what a counter would not take is counted afresh
    /// from then, not carried over into the next run's reason.
    sale_since: Option<Instant>,
    last_raise: Option<Instant>,
    /// The raise last sent, until the server answers it or it is given up
    /// on.
    pending: Option<Pending>,
    /// The pool at which nothing was worth buying, whether the character
    /// was in a fight then, and when that was seen.
    nothing_at: Option<(i64, bool, Instant)>,
    /// Raises the server did not answer in time, and when they were given
    /// up on: refused, or only late.
    sulking: Vec<(Pending, Instant)>,
    /// What the character was last found short of, and when.
    needs_seen: Option<(Instant, Vec<Need>)>,
    /// How many times the character has walked about the ground it is
    /// on looking for something to fight.
    roams: u32,
    /// Since when there has been nothing here to fight, `None` while
    /// there is (see `Client::autoplay_watch_the_ground`).
    quiet_since: Option<Instant>,
    /// The ground being travelled to, and since when.
    bound: Option<(u32, Vec2, String)>,
    bound_since: Option<Instant>,
    /// How long that walk may take before it counts as stuck (see `road::walk_limit`).
    bound_limit: Option<std::time::Duration>,
    /// When the walk to that ground was last planned again after
    /// something else broke it off (see [`on_the_way`]).
    bound_walked_on: Option<Instant>,
    /// The ground the character hunts on, once it has arrived.
    pub hunting_at: Option<u32>,
    /// Grounds not to go to for a while, and since when.
    skip: Recent<u32>,
    run: Option<Run>,
    /// The run in progress was asked for from the vendoring panel and
    /// is stepped from there, not by autoplay's tick (see
    /// [`Client::town_run_by_hand`]). Autoplay still counts it as the
    /// character being busy, and does not let it go for town runs being
    /// off: the panel is the one thing that runs the shopping with the
    /// rest of autoplay off. Left held for a while with autoplay
    /// running town runs, it is autoplay's again (see
    /// [`hand_has_let_go`]).
    by_hand: bool,
    /// When the panel last stepped the run.
    hand_stepped: Option<Instant>,
    /// A counter asked to open its window by a run that then ended
    /// before it did, and when. The window is closed if it comes within
    /// the time the run would have waited for it: opened after the run
    /// it was for, it would stand between the tidying and every pour,
    /// and the panel would show a counter open with nobody at it.
    window_unwanted: Option<(u32, Instant)>,
    last_run: Option<Instant>,
    /// Counters not to walk to for a while, keyed by where they stand
    /// rounded to the metre. A counter that could not be reached, or
    /// that came to nothing, is left alone and tried again later; each
    /// further disappointment doubles the wait.
    skip_vendors: crate::did::Patience<(i32, i32)>,
    /// Items no vendor will take. Refusals reach us after the item's
    /// kind and worth have already been checked against the counter,
    /// so what is left is the item saying no for itself -- the Academy
    /// bread, a quest token -- and no other counter will take it
    /// either. Remembered for the whole session, or the character
    /// offers the same loaf in every town.
    /// No run is started before this: a vendor that could not be
    /// reached is not tried again at once.
    next_run: Option<Instant>,
    /// No ground is chosen before this, for the same reason.
    next_hunt: Option<Instant>,
    /// Where the character last stood outdoors. A journey out of a
    /// building begins with a walk back to this spot, since the
    /// planner cannot see out of a shop.
    last_outdoors: Option<Vec2>,
    /// The journey to make once outside.
    after_out: Option<Vec2>,
    /// What the character was short of when the run began, for the
    /// stops after the first.
    needs: Vec<Need>,
}

/// A few names, and how many more there are. A caster short of every
/// component in the book has thirty-one of them, and a line of the log
/// is not the place to read all thirty-one.
fn a_few(names: &[&str]) -> String {
    const MOST: usize = 4;
    if names.is_empty() {
        return "nothing".to_string();
    }
    if names.len() <= MOST {
        return names.join(", ");
    }
    format!(
        "{} and {} more",
        names[..MOST].join(", "),
        names.len() - MOST
    )
}

/// `haystack` contains `needle`, ignoring ASCII case; `needle` is
/// already lowercase.
///
/// Written out rather than lowercasing both sides, which allocates:
/// choosing which shop to walk to compares every need against every
/// ware of a thousand counters.
fn contains_fold(haystack: &str, needle: &str) -> bool {
    let (h, n) = (haystack.as_bytes(), needle.as_bytes());
    if n.is_empty() || n.len() > h.len() {
        return false;
    }
    h.windows(n.len())
        .any(|w| w.iter().zip(n).all(|(a, b)| a.eq_ignore_ascii_case(b)))
}

/// Whether an item is loot to sell. `ammo` says whether it goes in the
/// ammunition slot, which the stats do not carry.
///
/// The order matters, and it is: what cannot be sold, then what the
/// player said, then what the loot rules tagged, then the standing
/// searches. Anything the searches do not name is kept -- a character
/// that empties its pack into a vendor loses the suit it was building.
/// Things that never go over a counter, whatever any rule, profile or
/// tag says.
///
/// A rule is a policy and this is not: it is the difference between a
/// character that can be trusted with its own pack and one that has to
/// be watched. Tinkering is spent on an item and cannot be got back;
/// an inscription is somebody's writing; equipped gear is what the
/// character fights and lives in; and an item the server has flagged
/// unsellable will be refused anyway, so offering it is a wasted round
/// trip and, worse, a chance to get stuck on it.
pub use ac_loot::sale::{
    fate, never_sell, never_sell_because, never_sell_carried, offer_to_vendor, Fate,
};

impl State {
    /// Let go of any journey the growth rules were part-way through.
    /// Used when the player takes the character back (see
    /// `Client::stop_moving_by_itself`).
    pub fn let_go(&mut self) {
        self.bound = None;
        self.bound_since = None;
        self.bound_limit = None;
        self.after_out = None;
    }

    /// Let go of the walk to a ground, and of the step outside before it unless a town run under
    /// way is taking that step: [`State::let_go`] less what would strand a run.
    pub(crate) fn drop_ground_walk(&mut self) {
        self.bound = None;
        self.bound_since = None;
        self.bound_limit = None;
        if self.run.is_none() {
            self.after_out = None;
        }
    }

    /// Whether a run to town is under way, from setting off to the last
    /// counter.
    pub(crate) fn town_run_under_way(&self) -> bool {
        self.run.is_some()
    }

    /// The server turned something away because the character was
    /// busy. Heard while a counter is being asked for its window, that
    /// is the Use being turned away for a cast of our own, and the run
    /// waits for the cast rather than for the counter (see
    /// [`on_opening`]). Past the asks a busy refusal earns, the
    /// refusals are the counter's own and the run is on the ordinary
    /// clock: kept, the refusal had the status promise an ask "once
    /// the cast lands" that was never coming.
    pub(crate) fn counter_said_busy(&mut self, now: Instant) {
        if let Some(Run {
            phase: Phase::Opening {
                busy, asked_over, ..
            },
            ..
        }) = self.run.as_mut()
        {
            if *asked_over < BUSY_ASKS {
                *busy = Some(now);
            }
        }
    }

    /// Whether the run under way is the vendoring panel's to step.
    pub(crate) fn run_by_hand(&self) -> bool {
        self.by_hand && self.run.is_some()
    }

    /// Whether the growth rules have somewhere to be: a hunting ground to
    /// reach (`bound`) or counters to go round (`run`). That says why the
    /// character would be travelling, and nothing about whether it still
    /// is (see [`Client::on_its_way`]).
    fn has_an_errand(&self) -> bool {
        self.bound.is_some() || self.run.is_some()
    }

    /// Let go of an errand whose setting has been turned off: a run with
    /// town runs off, a walk to a ground with grounds off and no hunting
    /// area. Nothing carries either on once its rule is no longer called,
    /// so left set it said an errand was under way for the rest of the
    /// session -- the character walked past what stood on its own ground
    /// on every patrol, and a run held open kept a follower from its
    /// leader.
    ///
    /// The run let go is handed back: its window, if it had one, is the
    /// client's to close (see [`Client::autoplay_grow`]).
    fn drop_what_is_turned_off(&mut self, town_runs: bool, grounds: bool) -> Option<Run> {
        // A run the panel is driving is not autoplay's to let go: town
        // runs being off is exactly when the panel is the thing running
        // one.
        let mut dropped = None;
        if !town_runs && !self.by_hand {
            dropped = self.run.take();
            if dropped.is_some() {
                self.needs.clear();
            }
        }
        if !grounds {
            self.bound = None;
            self.bound_since = None;
            self.bound_limit = None;
            self.bound_limit = None;
        }
        dropped
    }
}

impl Client {
    /// The growth rules: spend experience, find monsters, run to town.
    /// Run once a frame when nothing more pressing claimed it; true
    /// when it did something.
    pub fn autoplay_grow(&mut self, now: Instant) -> bool {
        let cfg = self.autoplay.config.growth.clone();
        if self.world.player_guid.is_none() {
            return false;
        }
        if let Some(pl) = self.player.as_ref() {
            if !pl.is_indoors() {
                let p = pl.world_position();
                self.autoplay.growth.last_outdoors = Some(Vec2::new(p.x, p.y));
            }
        }
        // Whether we are underground decides what a journey may be made
        // of. Knowing needs the block's collision, which loads lazily,
        // so it is settled here once a tick rather than asked for in
        // the middle of a decision.
        let assets = self.assets.clone();
        self.autoplay.growth.in_dungeon = self
            .player
            .as_mut()
            .map(|pl| pl.is_indoors() && pl.in_dungeon(&assets))
            .unwrap_or(false);
        // An errand whose setting was turned off part-way is carried on by
        // nothing below, so it is let go here.
        let roams_an_area = self.autoplay.config.fight.area.is_some();
        let dropped = self
            .autoplay
            .growth
            .drop_what_is_turned_off(cfg.town_runs, cfg.hunt_grounds || roams_an_area);
        // A run let go at its counter leaves its window behind, and the
        // window is ours: the server keeps none, so nothing closes it
        // but this client, and with town runs off no later run would.
        // Left open it read as a counter at hand for the rest of the
        // session (see [`Client::counter_at_hand`]).
        if let Some(run) = dropped {
            self.window_no_longer_wanted(&run);
            if self.world.open_vendor.is_some() {
                self.close_vendor();
            }
        }
        let mode = self.grow_mode(now, &cfg);
        // A run the vendoring panel is driving is the panel's to step,
        // and has the tick whether or not town runs are on: the
        // character is on a run, and nothing below -- the hunting, with
        // its patrols and roams -- walks it off the counter in the
        // middle of one. Held for a while with town runs on, it is
        // autoplay's again and stepped below; with them off, the panel
        // is the one thing running it, and the status line says so.
        if self.autoplay.growth.run_by_hand() {
            if !hand_has_let_go(self.autoplay.growth.hand_stepped, now) {
                return true;
            }
            if !cfg.town_runs {
                let vendor = self.autoplay.growth.run.as_ref().map(|r| r.vendor.clone());
                self.autoplay.say(
                    Doing::Shopping,
                    format!(
                        "the run to {} is held by the vendoring panel",
                        vendor.unwrap_or_default()
                    ),
                );
                return true;
            }
            self.autoplay.growth.by_hand = false;
            self.autoplay.note(
                "the vendoring panel let go of its run: autoplay takes it on",
                now,
            );
        }
        // Experience is not spent here but as housekeeping (see
        // [`Client::autoplay_spend_xp`]): this is the last goal, and a
        // rank that waited for it waited for good.
        if cfg.town_runs && self.grow_town_run(now, &cfg) {
            return true;
        }
        // A follower goes with its leader, not to a ground of its own (`grow_hunt`): a walk there
        // set meanwhile, as a town run's end sets one, is let go whatever the party's mode.
        if self.is_led() {
            self.autoplay.growth.drop_ground_walk();
            return false;
        }
        // A party that has stopped to restock does not wander off to a
        // new hunting ground in the middle of it, and nor does one that
        // has given up and stopped in town.
        //
        // A hunting area the player chose is walked about whether or not
        // the character is left to find grounds of its own: with that
        // off, a character that had cleared the Holtburg field it was
        // given stood in the middle of it for good.
        if (cfg.hunt_grounds || roams_an_area)
            && mode.hunting()
            && !self.autoplay.growth.stopped_in_town
            && self.grow_hunt(now, &cfg)
        {
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests;
