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

use std::time::{Duration, Instant};

use glam::Vec2;

use crate::autoplay::Doing;
use crate::logistics::Stage;
use crate::Client;
use ac_world::{item_type, object_desc_flags};

pub(crate) mod burden;
mod config;
mod hunt;
mod needs;
mod policy;
mod raise;
mod road;
mod sale;
mod supplies;
mod xp;

pub use config::Growth;
use needs::{Need, NeedKind};
use policy::restocks_as_a_party;
use raise::Pending;
pub use raise::{batch_raise, choose_raise, Batch, Climb, Offer, Raise};
use road::{on_the_way, OnTheWay, RETRY_AFTER, WALK_ON_EVERY, WALK_TIMEOUT};
use sale::worth_a_sale_run;
pub use sale::Salable;

/// Standing this close to a vendor is close enough to use it: the
/// server walks the character the last stretch itself.
const VENDOR_REACH: f32 = 35.0;
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
const BUSY_ASKS: u32 = 3;
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
/// How near a counter must stand to a *way out* -- a gem's exit, a
/// recall's landing, the character's own feet -- to be worth stopping
/// at on a run.
///
/// This used to be measured from the first counter of the run, which
/// meant one town and no further. But a run is not a walk around a
/// town: a mid-level character uses a Town Network gem, takes the
/// portal it summons, sells at the broker outside Cragstone, uses an
/// Archmage gem, takes that portal, restocks its components there, and
/// recalls back to where it was hunting. Every one of those counters is
/// a long way from the last one and a few paces from a way out, which
/// is the distance that actually costs anything.
const NEAR_A_WAY_OUT: f32 = 300.0;
/// How far from a way out the first counter of a run made for loot
/// that merely adds up (see [`worth_a_sale_run`]) may stand: the town
/// the character is in, or the one its gem lands in. A pack that
/// cannot hunt on is walked anywhere; a few peas are not, or one Lead
/// Pea carried a quarter of an hour is a walk to an archmage three
/// towns over, and the same again for the next.
const SALE_RUN_REACH: f32 = VENDOR_RINGS[0];

/// A distance for a status line, in steps of fifty metres, so the line
/// changes (and is logged) now and then rather than every frame.
fn about(distance: f32) -> String {
    format!("{} m", ((distance / 50.0).ceil() * 50.0) as u32)
}

/// What a run to town is for, which decides which counter it goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Errand {
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

/// What the next counter of a run is chosen for, and among which (see
/// [`Client::pick_vendor`]).
#[derive(Clone, Copy)]
struct Stop<'a> {
    errand: Errand,
    /// No further than this from a way out, once a run is already out.
    within: Option<f32>,
    /// The counters this run has already called at, by position.
    visited: &'a [Vec2],
}

/// Where a town run stands.
#[derive(Clone, Debug, PartialEq)]
enum Phase {
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
struct Run {
    vendor: String,
    at: Vec2,
    phase: Phase,
    /// When the phase began.
    since: Instant,
    last_sell: Option<Instant>,
    /// Where the run started. Later stops are looked for from every way
    /// out the character has; this is only the fallback for when its
    /// own position is not known.
    town: Vec2,
    stops: u32,
    /// How many items were sold at all the stops so far.
    sold: u32,
    /// Why the run was made.
    reason: String,
    /// What it is for: to sell, or to buy (see [`Errand`]).
    errand: Errand,
    /// The vendors already called on this run, by position.
    visited: Vec<Vec2>,
    /// When the walk to this counter was last planned again after
    /// something broke it off (see [`on_the_way`]).
    walked_on: Option<Instant>,
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

/// What a run waiting on a counter's window does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OnOpening {
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
fn on_opening(
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
fn counter_asked(phase: Option<&Phase>, window: Option<u32>) -> Option<u32> {
    match phase {
        Some(Phase::Going) => None,
        Some(Phase::Opening { guid, .. }) => Some(*guid),
        Some(Phase::Appraising | Phase::Selling { .. }) | None => window,
    }
}

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
fn hand_has_let_go(stepped: Option<Instant>, now: Instant) -> bool {
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
    /// When the walk to that ground was last planned again after
    /// something else broke it off (see [`on_the_way`]).
    bound_walked_on: Option<Instant>,
    /// The ground the character hunts on, once it has arrived.
    pub hunting_at: Option<u32>,
    /// Grounds not to go to for a while, and since when.
    skip: Vec<(u32, Instant)>,
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

/// Where each carried gem can put the character, as ways out.
///
/// A gem that lands outdoors lands where it lands. One that lands
/// indoors -- the Town Network gem comes out in the hub -- is worth more
/// than its landing: nothing is sold in a hub and nobody can walk out of
/// one, but every portal standing in it is a few steps away and each of
/// those comes out somewhere. Judging the gem by the hub alone kept the
/// broker outside Cragstone off every run, gem in the pack or not.
///
/// The landing still counts too: a gem that comes out inside a building
/// leaves the rest of its landblock a walk away.
fn gem_ways(gems: &[ac_world::trip::Gem]) -> Vec<(Vec2, String)> {
    let mut out = Vec::new();
    for g in gems {
        out.push((g.exit, g.name.clone()));
        if g.exit_cell & 0xFFFF < 0x100 {
            continue;
        }
        for p in ac_world::portals::out_of(g.exit_cell) {
            out.push((p.to_xy(), format!("{}, then {}", g.name, p.name)));
        }
    }
    out
}

/// Which way out leaves the character nearest `at`, how far that is,
/// and what it is called.
fn nearest_way(ways: &[(Vec2, String)], at: Vec2) -> (Vec2, f32, String) {
    ways.iter()
        .map(|(w, name)| (*w, at.distance(*w), name.clone()))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap_or((at, 0.0, "here".to_string()))
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

/// What a trip to one shop is expected to achieve, worked out from the
/// shop lists before the character sets off.
///
/// A run to town costs minutes of hunting, so it is worth knowing in
/// advance whether the counter has what the character came for and
/// whether there is money enough to pay for it. Without it the party
/// does what it used to: walk to a shop, find nothing it can afford,
/// and walk to the next one, for ever.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Forecast {
    /// The shop, for the log.
    pub shop: String,
    /// Needs it has on the shelf, and needs it has not.
    pub stocks: Vec<String>,
    pub missing: Vec<String>,
    /// What the needs it stocks would cost in full at its prices.
    pub bill: u32,
    /// The least one of anything wanted costs here; 0 when it stocks
    /// nothing that is wanted.
    pub cheapest: u32,
    /// Coin and notes in hand.
    pub purse: u32,
    /// What it would pay for what the character means to sell, and how
    /// many things that is.
    pub takings: u32,
    pub selling: usize,
}

impl Forecast {
    /// What there is to spend once the selling is done.
    pub fn funds(&self) -> u32 {
        self.purse.saturating_add(self.takings)
    }

    /// Why this shop was the one chosen, in a few words.
    ///
    /// The choice is a ring search outwards, taking the best shop in
    /// the first ring with anything worth the walk, ordered by whether
    /// it fills the whole order, then by how many lines of it it
    /// stocks, then by how near it is. So the answer is always some
    /// mixture of those three, and worth saying out loud: "it went
    /// there" is a bug report, "it went there because it was the only
    /// one stocking quarrels" is an explanation.
    pub fn why_it_was_picked(&self) -> String {
        if self.stocks.is_empty() {
            return match self.selling {
                0 => "nothing wanted is sold here; nearest counter".to_string(),
                n => format!("nowhere sells what is wanted; {n} thing(s) to sell"),
            };
        }
        let what = a_few(&self.stocks.iter().map(|s| s.as_str()).collect::<Vec<_>>());
        let covers = if self.covers_it() {
            "the whole order".to_string()
        } else if self.missing.is_empty() {
            format!("{what}, but short of coin")
        } else {
            format!(
                "{what} (not {})",
                a_few(&self.missing.iter().map(|s| s.as_str()).collect::<Vec<_>>())
            )
        };
        match self.selling {
            0 => covers,
            n => format!("{covers}; {n} thing(s) to sell here"),
        }
    }

    /// Whether the trip fills the whole order.
    pub fn covers_it(&self) -> bool {
        self.missing.is_empty() && !self.stocks.is_empty() && self.funds() >= self.bill
    }

    /// Whether the trip achieves anything at all: something to sell, or
    /// one thing it wants that it can pay for. Buying part of an order
    /// counts -- a mage that can afford half its tapers is a mage that
    /// can keep casting -- but buying none of it does not.
    pub fn worth_going(&self) -> bool {
        self.selling > 0 || (self.cheapest > 0 && self.funds() >= self.cheapest)
    }

    /// Said aloud, so whoever is watching knows why the party did or
    /// did not set off.
    pub fn tell(&self) -> String {
        let asked = self.stocks.len() + self.missing.len();
        let mut out = format!("{} of {asked} on the shelf", self.stocks.len());
        if self.bill > 0 {
            out.push_str(&format!(", {} to buy", self.bill));
        }
        out.push_str(&format!(", {} in hand", self.purse));
        if self.selling > 0 {
            out.push_str(&format!(", {} for {} item(s)", self.takings, self.selling));
        }
        if !self.missing.is_empty() {
            let missing: Vec<&str> = self.missing.iter().map(String::as_str).collect();
            out.push_str(&format!("; no {}", a_few(&missing)));
        }
        out
    }
}

/// The cheapest ware on this shelf that answers the need, if any.
fn shop_ware<'a>(
    shop: &'a ac_world::shops::Shop,
    need: &Need,
    needle: &str,
) -> Option<&'a ac_world::shops::Ware> {
    shop.sells
        .iter()
        .filter(|w| match &need.kind {
            NeedKind::Named(_) => contains_fold(&w.name, needle),
            NeedKind::Ammo(kind) => ammo_stock(&w.name, *kind),
            NeedKind::Component(wcid) => w.wcid == *wcid,
        })
        .min_by_key(|w| w.value)
}

/// What a trip to `shop` would buy, cost and fetch. `wants` is the
/// needs paired with their names already folded, since this is asked of
/// every counter in range.
fn forecast(
    shop: &ac_world::shops::Shop,
    wants: &[(&Need, String)],
    purse: u32,
    salables: &[Salable],
) -> Forecast {
    let mut f = Forecast {
        shop: shop.name.clone(),
        purse,
        ..Default::default()
    };
    for (need, needle) in wants {
        // A line that names its counter is only filled there. To every
        // other shop it reads as a line they do not carry, which is the
        // truth as far as this character is concerned, and the ranking
        // then sends it to the one that does.
        if !need.may_buy_at(&shop.name) {
            f.missing.push(need.name.clone());
            continue;
        }
        match shop_ware(shop, need, needle) {
            Some(w) => {
                let unit = shop.charges_for(w).max(1);
                f.bill = f.bill.saturating_add(unit.saturating_mul(need.want));
                f.cheapest = if f.cheapest == 0 {
                    unit
                } else {
                    f.cheapest.min(unit)
                };
                f.stocks.push(need.name.clone());
            }
            None => f.missing.push(need.name.clone()),
        }
    }
    for it in salables
        .iter()
        .filter(|it| it.taken_by(shop.buys, shop.min_value, shop.max_value))
    {
        // A counter's limits are about one of a thing, and its payment
        // is for all of them. Judging a stack by its total refuses a
        // hundred Pyreal Peas outright -- five million against a
        // ceiling of one -- and then reckons the character has nothing
        // to sell, which is how it came to walk thirty kilometres past
        // a counter that would have bought them twenty at a time.
        if let Some(paid) = shop.pays_for(it.item_type, it.each()) {
            f.selling += 1;
            f.takings = f
                .takings
                .saturating_add(paid.saturating_mul(it.stack.max(1)));
        }
    }
    f
}

/// Where a counter stands, to the metre: what a skipped vendor is
/// remembered by. Two vendors a metre apart are the same counter for
/// this purpose, which is the same slack the old comparison allowed.
fn spot(at: Vec2) -> (i32, i32) {
    (at.x.round() as i32, at.y.round() as i32)
}

/// Whether a stock line is plain ammunition of `kind`: an "Arrow", not
/// a "Bundle of Arrowheads" or a "Fire Arrow" (the plain kind is what
/// is cheap and always there).
fn ammo_stock(name: &str, kind: u32) -> bool {
    use ac_world::fletching::ammo_type;
    let plain = match kind {
        ammo_type::ARROW => "arrow",
        ammo_type::BOLT => "quarrel",
        ammo_type::ATLATL => "atlatl dart",
        _ => return false,
    };
    name.trim().to_lowercase() == plain
}

/// What the vendor charges for an item of `value` (ACE's SellPrice), at
/// least a pyreal.
/// Whether a name on the keep-stocked list is worth buying. A healing
/// kit does nothing for a character that has not trained Healing -- it
/// restores next to nothing untrained -- so buying one wastes the money
/// and the trip. Such a character heals with a spell instead.
fn worth_stocking(name: &str, heals_with_kits: bool) -> bool {
    if heals_with_kits {
        return true;
    }
    !name.to_lowercase().contains("healing kit")
}

/// How far to look for a shop before settling for a nearer one with
/// less on its shelves: the town we are in, the towns around it, then a
/// long walk, then anywhere at all.
const VENDOR_RINGS: [f32; 3] = [600.0, 3_000.0, 15_000.0];

/// The distances to search, widening, never past `within`.
///
/// The last one is `within` itself (or everything), so a character with
/// nothing nearby still finds a shop rather than standing still.
fn vendor_rings(within: Option<f32>) -> Vec<f32> {
    let cap = within.unwrap_or(f32::INFINITY);
    let mut out: Vec<f32> = VENDOR_RINGS.iter().copied().filter(|r| *r < cap).collect();
    out.push(cap);
    out
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

/// Which of two counters is worth walking to, better first.
///
/// The whole order in one stop beats part of it, then more of the order
/// beats less, then what the counter *pays* -- which used not to be
/// asked at all. What a shop gives for an item is `value * buy_rate`
/// and the spread is nearly half: around Cragstone the Scriveners
/// eighty metres off pay 0.5 and the Arcanum Broker three hundred
/// metres further on pays 0.95, and ranking on distance alone walked
/// past the broker every time. Distance settles what is left.
///
/// That is the order for a trip to buy. A trip to sell turns it round:
/// what the counter pays for the loot comes first, then how much of
/// the loot it takes, and what it stocks only settles ties. Ranked the
/// buying way, a run made to empty a pack of peas went to whichever
/// counter had the most of the shopping list on its shelf, and the
/// peas came home.
fn better_counter(errand: Errand, a: (&Forecast, f32), b: (&Forecast, f32)) -> std::cmp::Ordering {
    let (fa, near_a) = a;
    let (fb, near_b) = b;
    let to_buy = || {
        fb.covers_it()
            .cmp(&fa.covers_it())
            .then_with(|| fb.stocks.len().cmp(&fa.stocks.len()))
            .then_with(|| fb.takings.cmp(&fa.takings))
    };
    match errand {
        Errand::Buy => to_buy(),
        Errand::Sell => fb
            .takings
            .cmp(&fa.takings)
            .then_with(|| fb.selling.cmp(&fa.selling))
            .then_with(to_buy),
    }
    .then_with(|| near_a.total_cmp(&near_b))
}

/// The best of these counters for the errand, each with how far off it
/// is, and the forecast it was chosen on; `None` when none of them
/// serves it. The heart of [`Client::pick_vendor`], with the walking
/// of the shop list and the character's own circumstances left to it,
/// so that the choice can be tested on its own.
fn choose_counter<'a>(
    counters: impl Iterator<Item = (&'a ac_world::shops::Shop, f32)>,
    errand: Errand,
    wants: &[(&Need, String)],
    purse: u32,
    salables: &[Salable],
) -> Option<(&'a ac_world::shops::Shop, Forecast)> {
    counters
        .map(|(s, reach)| (s, reach, forecast(s, wants, purse, salables)))
        .filter(|(_, _, f)| errand.served_by(f))
        .min_by(|(_, ra, fa), (_, rb, fb)| better_counter(errand, (fa, *ra), (fb, *rb)))
        .map(|(s, _, f)| (s, f))
}

impl State {
    /// Let go of any journey the growth rules were part-way through.
    /// Used when the player takes the character back (see
    /// `Client::stop_moving_by_itself`).
    pub fn let_go(&mut self) {
        self.bound = None;
        self.bound_since = None;
        self.after_out = None;
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

    /// The best vendor to make for, given every way the character has
    /// of being somewhere else: not one being avoided, not one in
    /// `visited`, within `within` metres of a way out when given, and
    /// -- this is the point of it -- one the trip is known in advance
    /// to achieve something at.
    ///
    /// `ways` is where the character can cheaply be (see
    /// [`Self::ways_out`]); a shop is judged by its distance to the
    /// nearest of them.
    ///
    /// Comes back with the forecast it was chosen on, so the caller can
    /// decide whether to set off at all and can say why.
    ///
    /// `stop` says what the trip is for and where it may go. A trip to
    /// sell goes to a counter that pays for what is carried and to
    /// nothing else: the shops are ranked on what they pay, and there
    /// is no "any counter will do" when none of them buys any of it
    /// (see [`Errand`]).
    fn pick_vendor(
        &self,
        cfg: &Growth,
        needs: &[Need],
        ways: &[(Vec2, String)],
        stop: Stop<'_>,
        now: Instant,
    ) -> Option<(String, Vec2, Forecast)> {
        let Stop {
            errand,
            within,
            visited,
        } = stop;
        // How far a shop is: from the nearest way out, not from the
        // feet.
        let reach = |at: Vec2| {
            ways.iter()
                .map(|(w, _)| at.distance(*w))
                .fold(f32::MAX, f32::min)
        };
        // Counters this character can actually trade at. A society's
        // archmage is a very good archmage and no use at all to
        // somebody else's society, and a chapter house behind a quest
        // portal is a journey to a door that will not open.
        let society = self.society();
        let quests = cfg.gates_open.clone();
        let skip = &self.autoplay.growth.skip_vendors;
        let allowed = |at: Vec2| {
            within.is_none_or(|w| reach(at) <= w)
                && !visited.iter().any(|p| p.distance(at) < 1.0)
                && !skip.held(&spot(at), now)
        };
        // Worked out once for the whole search rather than per shop:
        // this walks every counter in range.
        let wants: Vec<(&Need, String)> = needs
            .iter()
            .filter(|n| n.want > 0 && n.buyable)
            .map(|n| {
                let needle = match &n.kind {
                    NeedKind::Named(t) => t.trim().to_lowercase(),
                    _ => String::new(),
                };
                (n, needle)
            })
            .collect();
        let purse = self.spendable();
        let salables = self.salables(cfg);

        // A shop that has what the character came for is worth a longer
        // walk than one that does not: an archer out of quarrels is not
        // helped by the archmage next door. But only so much longer.
        // Ranking on what a shop stocks alone sent a character twenty-
        // five kilometres to a counter with one more line on the shelf,
        // so the search widens in rings and takes the best shop in the
        // first ring that has anything.
        // The counter the profile names, when it names one. A player
        // who has said where to sell has said it; the rings below are
        // for finding one when nobody has.
        let named = self.sell_to_named();
        if let Some(want) = named.as_deref() {
            // Named, but not exempt, and not the answer to every
            // question.
            //
            // `allowed` is what remembers the counters this run has
            // already emptied its pack at and the ones lately found to
            // be no use; without it a run spent every one of its stops
            // walking back to the same counter, because the name matches
            // just as well the second time.
            //
            // And it is only taken when the trip to it is worth making.
            // This is where a character goes to *sell*; returning it
            // whatever the forecast said made it the answer for buying
            // too, and the ring search -- the thing that finds an
            // archmage -- never ran. A mage out of tapers, with coin in
            // its purse and nothing in its pack worth selling, was sent
            // to a counter that stocks no components, told the trip was
            // not worth making, and sent there again on every run after.
            // It never bought tapers again.
            if let Some(found) = ac_world::shops::all()
                .iter()
                .filter(|s| s.open_to(society, &quests) && allowed(s.xy()))
                .find(|s| s.name.eq_ignore_ascii_case(want))
            {
                let f = forecast(found, &wants, purse, &salables);
                if errand.served_by(&f) {
                    return Some((found.name.clone(), found.xy(), f));
                }
            }
        }
        for ring in vendor_rings(within) {
            let counters = ac_world::shops::all()
                .iter()
                .filter(|s| allowed(s.xy()) && reach(s.xy()) <= ring)
                .filter(|s| s.open_to(society, &quests))
                .map(|s| (s, reach(s.xy())));
            if let Some((s, f)) = choose_counter(counters, errand, &wants, purse, &salables) {
                return Some((s.name.clone(), s.xy(), f));
            }
        }
        // A trip to sell has nowhere to go: no counter in reach buys any
        // of what is carried. Walking to one that does not is the trip
        // there and the trip back with the same pack, and there is no
        // nearest-counter fallback for it.
        if errand == Errand::Sell {
            return None;
        }
        // Nothing sells what is wanted, nothing is wanted at all, or
        // there is no money for any of it: any counter will do, which is
        // the case when the trip is to stop in town rather than to buy
        // something. The forecast comes back saying as much, and the
        // caller decides whether that is reason enough to walk.
        ac_world::landmarks::all()
            .iter()
            .filter(|l| l.kind == ac_world::landmarks::Kind::Vendor)
            .filter(|l| allowed(l.xy()))
            .min_by(|a, b| reach(a.xy()).total_cmp(&reach(b.xy())))
            .map(|l| {
                let look = Forecast {
                    shop: l.name.clone(),
                    missing: wants.iter().map(|(n, _)| n.name.clone()).collect(),
                    purse,
                    ..Default::default()
                };
                (l.name.clone(), l.xy(), look)
            })
    }

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

    /// Everywhere the character can get to cheaply, and what takes it
    /// there.
    ///
    /// A journey is not measured from the feet. A character carries
    /// ways of being somewhere else -- a lifestone recall, the two
    /// portal recalls, whatever gems are in the pack -- and each lands
    /// it at a known spot for the price of one cast. The counter worth
    /// going to is the one nearest *any* of those, not the one nearest
    /// where it happens to be standing.
    ///
    /// It matters most underground, where the feet are the one place
    /// that leads nowhere: a dungeon lies under the landblock it
    /// belongs to, so the nearest counter to a character in Holtburg
    /// Dungeon is one in Holtburg, a hundred metres up through rock.
    /// But it is just as true on the surface -- a recall to Arwic beats
    /// a two-kilometre walk to the shop over the hill.
    fn ways_out(&self, me: Vec2) -> Vec<(Vec2, String)> {
        let mut out: Vec<(Vec2, String)> = Vec::new();
        // Where it stands, unless where it stands leads nowhere.
        if !self.autoplay.growth.in_dungeon {
            out.push((me, "on foot".to_string()));
        }
        let world_xy = |p: ac_world::Position| {
            let o = ac_world::landblock_origin(p.cell);
            Vec2::new(o.x + p.local.x, o.y + p.local.y)
        };
        for r in self.castable_recalls() {
            if let Some(p) = self.recall_destination(r.spell) {
                out.push((world_xy(p), r.name.clone()));
            }
        }
        // A gem needs no skill and no components: carrying one is the
        // whole requirement, which often makes it the cheapest way out
        // a character has.
        out.extend(gem_ways(&self.carried_gems()));
        // And the dungeon's own door. Every portal's mouth and its
        // landing are known, so a character underground can say which
        // dungeon it is in and where the way out comes up -- and be
        // judged on the shops near *that*, which is what walking out
        // would actually achieve.
        if self.autoplay.growth.in_dungeon {
            if let Some(pl) = self.player.as_ref() {
                let block = pl.cell & 0xFFFF_0000;
                for p in ac_world::portals::out_of(block) {
                    out.push((p.to_xy(), format!("{} (the way out)", p.name)));
                }
            }
        }
        // Nothing to hand: the feet are all there is, wherever they are.
        if out.is_empty() {
            out.push((me, "on foot".to_string()));
        }
        out
    }

    /// Which society this character belongs to, as `Faction1Bits`: 1
    /// the Celestial Hand, 2 the Eldrytch Web, 4 the Radiant Blood, 0
    /// none. The server sends it with the rest of the character's
    /// properties, so this is knowledge rather than the player's word
    /// for it -- and it is what the societies' own counters ask for.
    pub fn society(&self) -> u32 {
        const FACTION1_BITS: u32 = 281;
        self.world
            .stats
            .ints
            .iter()
            .find(|(k, _)| *k == FACTION1_BITS)
            .map(|(_, v)| (*v).max(0) as u32)
            .unwrap_or(0)
    }

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
    fn grow_town_run(&mut self, now: Instant, cfg: &Growth) -> bool {
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
                // counter that pays, in the town it is in. Made to sell
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
                    within: (errand == Errand::Sell).then_some(SALE_RUN_REACH),
                    visited: &[],
                };
                let started =
                    self.start_town_run(now, &cfg, needs.clone(), reason.clone(), first, true);
                match (started, errand) {
                    // Nobody near buys what is carried: the player still
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
        let me = self.player.as_ref()?.world_position();
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
    fn window_no_longer_wanted(&mut self, run: &Run) {
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
        let me = self
            .player
            .as_ref()
            .map(|p| p.world_position())
            .map(|p| Vec2::new(p.x, p.y));
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

    /// The vendor object nearest `at`, by name when one matches.
    fn vendor_object(&self, name: &str, at: Vec2) -> Option<u32> {
        let vendors: Vec<(f32, bool, u32)> = self
            .world
            .objects
            .values()
            .filter(|o| {
                o.object_desc_flags & object_desc_flags::VENDOR != 0
                    || (o.item_type & item_type::CREATURE != 0
                        && o.object_desc_flags & object_desc_flags::ATTACKABLE == 0
                        && !o.is_player
                        && o.name == name)
            })
            .filter_map(|o| {
                let p = o.world_pos()?;
                let d = Vec2::new(p.x, p.y).distance(at);
                (d <= VENDOR_REACH + 10.0).then_some((d, o.name == name, o.guid))
            })
            .collect();
        vendors
            .iter()
            .filter(|(_, named, _)| *named)
            .chain(vendors.iter())
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, _, g)| *g)
    }

    /// One turn of the run in progress: what it came to, for the panel's
    /// Step; whether it goes on, for autoplay (see [`Turn`]).
    fn grow_run_step(&mut self, now: Instant, cfg: &Growth) -> Turn {
        let Some(mut run) = self.autoplay.growth.run.take() else {
            return Turn::Over;
        };
        let Some(me) = self.player.as_ref().map(|p| p.world_position()) else {
            self.autoplay.growth.run = Some(run);
            return Turn::Waited;
        };
        let me = Vec2::new(me.x, me.y);
        let elapsed = now.duration_since(run.since);
        match run.phase.clone() {
            Phase::Going => {
                if elapsed > WALK_TIMEOUT {
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
                        // the best-paying choice again every RUN_EVERY.
                        self.autoplay
                            .growth
                            .skip_vendors
                            .hold(spot(run.at), RUN_EVERY, now);
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
                if self.vendor_busy(now) {
                    self.autoplay.growth.run = Some(run);
                    return Turn::Waited;
                }
                let snap = self.vendor_snapshot(cfg);
                let next = self.autoplay.growth.shop.step(&snap, now);
                self.autoplay.growth.last_saying = next.saying.clone();
                match next.act {
                    Some(ac_vendor::Act::Close) | None => {
                        // Added to, not set: the run's count is for all
                        // its counters, and the rules count one at a
                        // time.
                        run.sold += self.autoplay.growth.shop.sold;
                        self.close_vendor();
                        self.autoplay.growth.shop = ac_vendor::Run::new();
                        Turn::after_stop(self.grow_run_next(run, now, cfg, None))
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
                        run.since = now;
                        run.phase = Phase::Selling { sent: Vec::new() };
                        self.autoplay.growth.run = Some(run);
                        Turn::Acted
                    }
                }
            }
        }
    }

    /// The run is done with this vendor: on to the next of the town
    /// when something is still wanted, else home. `left` says the
    /// vendor was no use -- what it did, as a predicate on its name:
    /// "would not trade", "buys none of this" -- and is to be avoided
    /// for a while; it is said in the status with where the run goes
    /// next, so that a counter walked past is a counter walked past
    /// for a reason. True while the run goes on.
    fn grow_run_next(&mut self, run: Run, now: Instant, cfg: &Growth, left: Option<&str>) -> bool {
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
            let me = self.player.as_ref().map(|p| p.world_position());
            let ways = match me {
                Some(p) => self.ways_out(Vec2::new(p.x, p.y)),
                None => vec![(run.town, "in town".to_string())],
            };
            // The selling first, so that what it fetches is there to
            // spend; and when nobody in reach buys any of it, the
            // shopping is still worth the stop.
            let to_buy = needs.iter().any(|n| n.urgent) || still_full || more_to_buy;
            let errands: &[Errand] = match (more_to_sell, to_buy) {
                (true, true) => &[Errand::Sell, Errand::Buy],
                (true, false) => &[Errand::Sell],
                (false, _) => &[Errand::Buy],
            };
            let picked = errands.iter().find_map(|&errand| {
                let next = Stop {
                    errand,
                    within: Some(NEAR_A_WAY_OUT),
                    visited: &run.visited,
                };
                self.pick_vendor(cfg, &needs, &ways, next, now)
                    .map(|(vendor, at, look)| (errand, vendor, at, look))
            });
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
