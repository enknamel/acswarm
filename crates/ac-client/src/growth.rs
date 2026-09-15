//! Keeping a character that plays on its own going for hours: the
//! rules of `crate::autoplay` fight, loot and buff, and these make sure
//! there is always something worth fighting, a pack to put the loot in,
//! and a character that grows with the experience it earns.
//!
//! Three rules. The first runs every tick as housekeeping (see
//! [`Client::autoplay_spend_xp`]); the other two when nothing more
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
use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;

use crate::autoplay::{Doing, LootAction, CORPSE_LIFE};
use crate::items::ItemStats;
use crate::logistics::{self, Stage, Supplies};
use crate::Client;
use ac_world::{equip, item_type, object_desc_flags};

/// One raise -- a rank, or a batch of them -- is sent at most this often.
const RAISE_EVERY: Duration = Duration::from_millis(700);
/// After a raise is sent, nothing more is spent until the server has
/// answered with the new pool, or this long has passed.
const RAISE_SETTLE: Duration = Duration::from_secs(3);
/// A pool worth fewer than this many of the chosen rank buys that one
/// rank, as it always did; a larger one buys several to a message (see
/// [`batch_raise`]).
const BATCH_FROM: i64 = 10;
/// When nothing was affordable, the pool is looked at again this often
/// (or as soon as it changes).
const XP_CHECK_EVERY: Duration = Duration::from_secs(20);
/// Standing this close to a ground's middle counts as being there.
const GROUND_REACH: f32 = 30.0;
/// A ground the character has just left is not gone back to for this
/// long; one it could not reach, the same.
const SKIP_GROUND_FOR: Duration = Duration::from_secs(20 * 60);
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
/// A rank the server would not sell (the pool did not move) is not
/// asked for again for this long.
const SULK_FOR: Duration = Duration::from_secs(10 * 60);
/// What the character is short of is worked out at most this often
/// while it waits for something to do.
const NEEDS_EVERY: Duration = Duration::from_secs(5);
/// At a ground with nothing in sight, the character walks this far to
/// look, this many times, before it moves on: the spawns of a block
/// are spread over it and the fight rules only look a short way.
const ROAM: f32 = 70.0;
const ROAMS: u32 = 3;
/// The longest a character inside a hunting area stands on a quiet spot
/// before walking to another part of it, seconds.
///
/// Short, because moving on inside an area costs only the walk: the
/// area is still the ground, the way is already known, and the fights
/// are wherever the character is not. Nine characters given a
/// 197 x 192 m field worked a 40 x 32 m corner of it for ten minutes,
/// keeping one spawn point busy, because the wait was the minute a
/// character picking a whole new ground is given.
const PATROL_AFTER: f32 = 10.0;
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
/// A walk to a vendor, or to a hunting ground, that has taken longer
/// than this is stuck somewhere: the place is given up on.
const WALK_TIMEOUT: Duration = Duration::from_secs(4 * 60);
/// After a journey that could not be planned, the next place is not
/// tried for this long.
const RETRY_AFTER: Duration = Duration::from_secs(30);
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

/// Growing the character and keeping it supplied.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Growth {
    /// Spend unassigned experience on skills, attributes and vitals.
    pub auto_xp: bool,
    /// Go to a hunting ground that suits the level when nothing is about.
    pub hunt_grounds: bool,
    /// How many levels either side of the character's a ground may be
    /// (see `ac_world::hunting::Ground::suits`).
    pub level_margin: u32,
    /// Hunt this landblock and no other, 0 to pick whatever suits.
    /// A party follows its leader's choice.
    #[serde(default)]
    pub hunt_at: u32,
    /// How to hunt the ground once there.
    #[serde(default)]
    pub tactic: ac_world::hunting::Tactic,
    /// Seconds with nothing to fight before moving on.
    pub idle_before_move: f32,
    /// Walk to a vendor when the pack is full or supplies are short.
    pub town_runs: bool,
    /// How much ammunition to carry for a bow or crossbow. A run to
    /// town is made when a quarter of this is left and none can be
    /// made from what is carried.
    pub ammo_keep: u32,
    /// Quest flags this character has earned, for the counters that
    /// ask for one (the Rossu Morta and Whispering Blade chapter
    /// houses, the Academy stores, and so on -- see
    /// `ac_world::shops::Gate`).
    ///
    /// The server never tells a client which quests it has done, so
    /// this is the player's word for it. Empty means those doors are
    /// treated as shut, which costs a walk to the next counter rather
    /// than a walk to a door that will not open. Society membership is
    /// not listed here: the character carries that itself.
    pub gates_open: Vec<String>,
    /// Loot taken for a counter is reason enough for a run to town on
    /// its own -- the pack need not be full nor a supply short -- once
    /// it is worth this much at face value, in pyreals. 0 turns the
    /// rule off. See [`worth_a_sale_run`].
    pub sell_run_value: u32,
    /// The same, once this many things are being carried for a
    /// counter, whatever they are worth: they are slots as well as
    /// money. 0 turns the rule off.
    pub sell_run_count: u32,
    /// The same, once anything has been carried for a counter this
    /// long, in seconds, however little it is: one Lead Pea is not
    /// worth a trip, but it is not worth carrying about all afternoon
    /// either. 0 turns the rule off.
    pub sell_run_patience: f32,
}

impl Default for Growth {
    fn default() -> Self {
        Growth {
            auto_xp: true,
            hunt_grounds: true,
            level_margin: 8,
            hunt_at: 0,
            tactic: ac_world::hunting::Tactic::default(),
            idle_before_move: 60.0,
            town_runs: true,
            ammo_keep: 250,
            gates_open: Vec::new(),
            // Five thousand at face is a few thousand in hand at any
            // counter's rate: minutes of hunting at the levels a
            // character first carries loot, and worth the minutes the
            // walk costs. An Iron Pea and a Lead Pea, three thousand
            // between them, are not -- they go on the count or the
            // patience.
            sell_run_value: 5_000,
            // Eight things for a counter is an armful: slots are going,
            // and it is well short of the pack filling by itself.
            sell_run_count: 8,
            // A quarter of an hour, near twice the least wait between
            // runs (`RUN_EVERY`): a character with one pea to sell goes
            // at most once in that while, and is not still carrying it
            // at dinner.
            sell_run_patience: 15.0 * 60.0,
        }
    }
}

/// Whether what the pack holds for a counter is reason enough for a
/// run to town, and the reason if it is.
///
/// A run used to be made for a full pack, a heavy one or a supply run
/// short, and for nothing else: what the loot rules had tagged for a
/// counter never came into it, so two peas taken to sell sat in a
/// roomy pack for ever and the character never went. The ledger says
/// why each thing was taken; this is where "to sell" is acted on.
///
/// `sale` is what the selling rules would let go today (see
/// [`Client::salables`]), and `carried_for` how long any of it has
/// been in the pack. Three rules, any one enough: it is worth
/// `sell_run_value` at face, there are `sell_run_count` things, or it
/// has been carried for `sell_run_patience`. Each is off at 0.
///
/// This is only the reason. The waits between runs and after a futile
/// one are the caller's ([`Client::grow_town_run`]), which is what
/// keeps a lone character from wearing a path to town for one pea.
fn worth_a_sale_run(
    sale: &[Salable],
    carried_for: Option<Duration>,
    cfg: &Growth,
) -> Option<String> {
    if sale.is_empty() {
        return None;
    }
    let worth: u32 = sale.iter().fold(0u32, |sum, s| sum.saturating_add(s.value));
    let count = sale.len() as u32;
    if cfg.sell_run_value > 0 && worth >= cfg.sell_run_value {
        return Some(format!(
            "carrying {worth} pyreals' worth for a counter ({count} thing(s))"
        ));
    }
    if cfg.sell_run_count > 0 && count >= cfg.sell_run_count {
        return Some(format!("carrying {count} things for a counter"));
    }
    // The config is hand-edited JSON: a patience no Duration can hold
    // (1e20, say) reads as the rule being off, not as a panic on every
    // frame a run could start.
    if let Some(patience) = Some(cfg.sell_run_patience)
        .filter(|p| *p > 0.0)
        .and_then(|p| Duration::try_from_secs_f32(p).ok())
    {
        if let Some(d) = carried_for.filter(|d| *d >= patience) {
            return Some(format!(
                "{count} thing(s) for a counter carried {} min",
                d.as_secs() / 60
            ));
        }
    }
    None
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

/// How long a spot with nothing on it is given before the character
/// looks elsewhere, seconds. See [`PATROL_AFTER`] for why a hunting
/// area is given less; a setting shorter than that is still obeyed.
fn quiet_before_move(cfg: &Growth, in_an_area: bool) -> f32 {
    if in_an_area {
        cfg.idle_before_move.min(PATROL_AFTER)
    } else {
        cfg.idle_before_move
    }
}

/// One thing experience can be spent on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Raise {
    Skill(u32),
    /// Index into `advance::ATTRIBUTE_NAMES`.
    Attribute(usize),
    /// Index into `advance::VITAL_NAMES`.
    Vital(usize),
}

impl Raise {
    /// What the stat is called.
    pub fn name(self) -> &'static str {
        use crate::advance::{ATTRIBUTE_NAMES, VITAL_NAMES};
        match self {
            Raise::Skill(id) => ac_world::stats::skill_name(id),
            Raise::Attribute(i) => ATTRIBUTE_NAMES.get(i).copied().unwrap_or("an attribute"),
            Raise::Vital(i) => VITAL_NAMES.get(i).copied().unwrap_or("a vital"),
        }
    }
}

/// A rank on offer: what it is, what it costs, and how much it matters
/// (1 for the skill the character fights with, less for the rest).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Offer {
    pub raise: Raise,
    pub cost: u32,
    pub weight: f32,
}

/// The better buy of two ranks, each a cost and a weight: the cheaper for
/// what it is worth, and of two as good, the cheaper outright.
fn by_value(a: (u32, f32), b: (u32, f32)) -> std::cmp::Ordering {
    (a.0 as f32 / a.1)
        .total_cmp(&(b.0 as f32 / b.1))
        .then(a.0.cmp(&b.0))
}

/// Of ranks each a cost and a weight, the best buy (see [`by_value`]),
/// with what it costs. A rank that matters not at all is never one.
fn best_buy<T>(ranks: impl Iterator<Item = (T, u32, f32)>) -> Option<(T, u32)> {
    ranks
        .filter(|r| r.2 > 0.0)
        .min_by(|a, b| by_value((a.1, a.2), (b.1, b.2)))
        .map(|(what, cost, _)| (what, cost))
}

/// The rank worth buying with `xp` to spend: the cheapest for what it is
/// worth, and nothing while the pool is short of it. A cheap rank of a
/// minor skill is bought before a dear rank of the main one, and the main
/// one catches up as the minor ones get dear.
///
/// It catches up only because the pool saves for it. Of the affordable
/// ranks alone, a character that had spent a large pool bought a minor
/// rank the moment the kills after it covered one: ten billion left
/// War Magic's next rank at 110 million and Arcane Lore's at 17 million,
/// worth about the same for what they are, and two billion more a kill at
/// a time went on Arcane Lore, Jump, Loyalty and the like, ten ranks each,
/// while War, Life, Focus, Self and Health got none.
pub fn choose_raise(offers: &[Offer], xp: i64) -> Option<Raise> {
    best_buy(offers.iter().map(|o| (o.raise, o.cost, o.weight)))
        .filter(|&(_, cost)| i64::from(cost) <= xp)
        .map(|(raise, _)| raise)
}

/// A stat in the running for the pool: the whole of its ladder, not only
/// its next rank, how much a rank of it matters, and whether it may be
/// bought now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Climb<'a> {
    pub raise: Raise,
    pub ladder: crate::advance::Ladder<'a>,
    pub weight: f32,
    /// Not to be bought for now -- a maximum in the middle of a fight, or
    /// a stat whose last raise the server has not answered -- but its
    /// share of the pool is kept for it all the same.
    pub held: bool,
}

/// The rank each stat of `field` is at once `xp` is spent on paper a rank
/// at a time, as [`choose_raise`] spends it: the best buy each time, until
/// the best buy costs more than is left.
///
/// Where it stops is the only thing the pool decides. The order of the
/// ranks is set by their prices and weights alone, so a pool spent in
/// parts -- some of the stats bought, the rest still to come, or answers
/// that have not arrived yet -- comes to the same ranks as the whole of it
/// spent at once, and the parts never add up to more than the pool.
fn plan(field: &[Climb], xp: i64) -> Vec<u32> {
    let mut at: Vec<u32> = field.iter().map(|c| c.ladder.ranks).collect();
    let mut left = xp;
    loop {
        let best = best_buy(
            field
                .iter()
                .zip(&at)
                .enumerate()
                .filter_map(|(i, (c, &rank))| Some((i, c.ladder.step(rank)?, c.weight))),
        );
        match best {
            Some((i, cost)) if i64::from(cost) <= left => {
                left -= i64::from(cost);
                at[i] += 1;
            }
            _ => return at,
        }
    }
}

/// Ranks of one stat bought in one message, and the experience they cost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Batch {
    pub ranks: u32,
    pub xp: u32,
}

/// A raise sent to the server and not yet answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pending {
    raise: Raise,
    batch: Batch,
    /// Where the stat stood when it was sent (see [`Raise::standing`]).
    /// The answer moves it. The pool alone says nothing: a kill moves that
    /// too, and a stat sized again from a record the answer has not
    /// reached yet is bought twice.
    was: Option<(u32, u32)>,
    /// When the message went out, which is not when it was queued: it
    /// goes on the wire at the top of the next tick, and a round of nine
    /// headless characters took four and a half seconds on the local
    /// server. Stamped by the first look at it after that.
    since: Option<Instant>,
}

impl Raise {
    /// The stat's ranks and the experience spent on it, as the server last
    /// said. None for a skill not on the sheet.
    fn standing(self, stats: &ac_world::stats::PlayerStats) -> Option<(u32, u32)> {
        match self {
            Raise::Skill(id) => stats.skill(id).map(|s| (u32::from(s.ranks), s.xp)),
            Raise::Attribute(i) => stats.attributes.get(i).map(|a| (a.ranks, a.xp)),
            Raise::Vital(i) => stats.vitals.get(i).map(|v| (v.ranks, v.xp)),
        }
    }
}

/// What to raise in one message, and by how many ranks, with `xp` to spend
/// and `field` every stat in the running for it.
///
/// ACE takes the experience for any number of ranks in one message and
/// puts all of it on the stat, and a rank to a message is hopeless for a
/// large pool: nine characters granted ten to fifty billion each on the
/// local server bought about a rank a second, and the one given ten
/// billion spent a thousandth of a percent of it in five minutes. But a
/// large pool must still be spread the way [`choose_raise`] spreads it,
/// over the skills and bars that matter, not poured into whichever stat
/// happened to be the best buy when it came in.
///
/// So the whole pool is spent on paper exactly as it would be a rank at a
/// time (see [`plan`]), and the best buy of the stats that paper run
/// raises is given the ranks it gives it, no more. No stat goes past the
/// rank buying one at a time would have left it at; each of the others is
/// bought up to its own in a message of its own when its turn comes. A
/// pool of any size is spread as it always was, and spent in about as
/// many messages as there are stats: ten billion in two dozen. Two
/// simpler rules were tried on the real XpTable and left behind. Buying
/// the best buy only while it stays the best value is a rank a message
/// again as soon as the stats are level, and spent nothing of ten billion
/// in a minute; capping a message at a share of the pool ran the first
/// stats bought sixty ranks ahead of the rest.
///
/// A stat held back for now (see [`Climb::held`]) is spent on in the paper
/// run like any other, and never sent. Its share waits for it, and a stat
/// the run gives nothing more is not sold a rank anyway: a caster given
/// ten billion in the middle of a fight was, on paper, a rank a message at
/// up to a hundred million, until Self, Health and Mana came out of the
/// fight ninety ranks short.
///
/// A pool worth fewer than [`BATCH_FROM`] of the chosen stat's next rank
/// buys the one rank, as it always did: what a kill brings in goes a rank
/// a message. A batch never goes past the top rank (ACE refuses an amount
/// past the experience left to the top outright, rather than trimming
/// it), never costs more than the pool, and so never more than the `u32`
/// a message carries, which is what the table's entries are. The paper
/// run is a step per rank: a few thousand at most, over a few dozen
/// stats, once a message.
///
/// None when the paper run raises nothing that may be bought now.
pub fn batch_raise(field: &[Climb], xp: i64) -> Option<(Raise, Batch)> {
    let planned = plan(field, xp);
    let (c, to) = best_buy(
        field
            .iter()
            .zip(planned)
            .filter(|(c, to)| !c.held && *to > c.ladder.ranks)
            .filter_map(|(c, to)| Some(((c, to), c.ladder.step(c.ladder.ranks)?, c.weight))),
    )
    .map(|((c, to), first)| {
        let to = if xp < i64::from(first) * BATCH_FROM {
            c.ladder.ranks + 1
        } else {
            to
        };
        (c, to)
    })?;
    Some((
        c.raise,
        Batch {
            ranks: to - c.ladder.ranks,
            xp: c.ladder.cost_to(to)?,
        },
    ))
}

/// Whether a rank raises the maximum of a vital: Health, Stamina or Mana
/// itself, or Endurance or Self, which the maximums are built from.
///
/// Such a rank leaves what is left of the vital where it was (ACE spends
/// the experience on the ranks and never touches the current value), so
/// the fraction left drops with every one. That fraction is what the
/// heal, Revitalize and mana rules read, and what the team reads to
/// heal a mate.
fn raises_a_maximum(raise: Raise) -> bool {
    const ENDURANCE: usize = 1;
    const SELF: usize = 5;
    matches!(raise, Raise::Vital(_) | Raise::Attribute(ENDURANCE | SELF))
}

/// How much a skill matters to a character fighting with `weapon_skill`
/// (0 for a caster's wand), given the schools it casts with.
fn skill_weight(skill: u32, weapon_skill: Option<u32>, caster: bool) -> f32 {
    use ac_world::stats::skill;
    const CREATURE_ENCHANTMENT: u32 = 31;
    const ITEM_ENCHANTMENT: u32 = 32;
    const VOID_MAGIC: u32 = 43;
    if Some(skill) == weapon_skill {
        return 1.0;
    }
    match skill {
        skill::WAR_MAGIC | VOID_MAGIC => {
            if caster {
                1.0
            } else {
                0.15
            }
        }
        skill::LIFE_MAGIC => {
            if caster {
                0.9
            } else {
                0.6
            }
        }
        skill::MELEE_DEFENSE | skill::MISSILE_DEFENSE => 0.7,
        15 => 0.6, // Magic Defense
        skill::HEALING => 0.5,
        CREATURE_ENCHANTMENT | ITEM_ENCHANTMENT => 0.5,
        skill::MANA_CONVERSION => {
            if caster {
                0.6
            } else {
                0.2
            }
        }
        skill::FLETCHING => {
            if weapon_skill == Some(47) {
                0.5
            } else {
                0.1
            }
        }
        skill::RUN => 0.3,
        // Another weapon skill: not the one in hand.
        41 | 44..=50 => 0.1,
        _ => 0.15,
    }
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
    wont_merge: crate::did::Patience<(u32, u32)>,
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

/// Why no stack was poured into another (see `Client::pour_next`).
///
/// Four answers rather than one, because the callers do different
/// things with them: a town run stays where it is while a pour is in
/// the air, and walks on to the counter when the pack is tight or the
/// character too laden; the housekeeping says the laden one out loud
/// once and then leaves the pack alone for a while.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Unpoured {
    /// The pack is as tight as it goes.
    Tight,
    /// There is something to pour and no room to be handed it. The
    /// server weighs a pour as though the source were being picked up
    /// for the first time.
    Laden,
    /// The last pour has not been answered yet.
    Wait(crate::did::Did),
    /// This pour was turned down, by the server or by our own rules.
    Turned(crate::did::Did),
}

/// Something the character is short of.
#[derive(Clone, Debug, PartialEq)]
struct Need {
    /// What it is, for the log.
    name: String,
    /// How many more to buy.
    want: u32,
    /// How many are carried, and how many the rules ask for. The
    /// shortfall alone does not say how close to empty a line is, and
    /// that is what decides whether the party stops hunting.
    have: u32,
    keep: u32,
    /// Whether it is short enough to be worth a run to town.
    urgent: bool,
    /// Whether enough of the world sells it to go and buy it. What is
    /// not is farmed instead: the Void components, and in practice the
    /// Diamond Scarab, whose one seller is a curiosity shop. A line
    /// like that must never drive a trip and must never be counted
    /// against the character's supplies, or a caster is out of stock
    /// for ever and the party restocks for ever.
    buyable: bool,
    /// The counter the player named for this line, if they named one.
    ///
    /// A want that says where it comes from is filled there and nowhere
    /// else: fletching supplies come in levels and elements that one
    /// bowyer carries and the next does not, and healing kits come in
    /// levels. Without this the field was editable, saved to the
    /// profile and read by nothing.
    from: Option<String>,
    kind: NeedKind,
}

impl Need {
    /// Whether this counter is one this line may be bought at. A line
    /// that names none is bought wherever it is sold.
    fn may_buy_at(&self, shop: &str) -> bool {
        match &self.from {
            None => true,
            Some(want) => shop.eq_ignore_ascii_case(want.trim()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum NeedKind {
    /// Stock whose name contains this.
    Named(String),
    /// Ammunition of this kind (`ac_world::fletching::ammo_type`).
    Ammo(u32),
    /// A spell component, by the weenie class of the item.
    Component(u32),
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

/// Twice its capacity is as much as burden can take from a character.
/// The server scales Melee and Missile Defense, and what the Run skill
/// adds to its speed, by two less its burden in multiples of capacity:
/// all of them at one, half at one and a half, none at all at two.
const DEFENSELESS_AT: f32 = 2.0;

/// How much more loot a character will take on before it has had
/// enough. `carried` is everything, gear and all; `loot` is the part of
/// it a counter would take.
///
/// Three things stop it, and the room is what the nearest of them
/// leaves:
///
/// * the working limit, `carry_up_to` times its capacity, measured on
///   the loot alone;
/// * twice its capacity, measured on everything, where it has no
///   defense left (or the working limit's own multiple, when the
///   player has set it higher than that);
/// * the server's wall at three times, measured on everything, past
///   which it hands over nothing at all.
///
/// The limit counts only loot because nothing else can be sold off. It
/// used to count everything, and that stopped Blargerton looting
/// altogether: Strength 60 gives a capacity of 9000 and a limit of
/// 13500 at one and a half times, and he carried 13866, nearly all of
/// it things he keeps -- his plate alone is 6540, then four foci and a
/// stack of Prismatic Tapers. No room at all, so every corpse he opened
/// was shut again at once and marked looted, and he took nothing; the
/// town run read the same zero as laden and could send him to sell with
/// nothing to sell. Leaving out only what he wore was not enough: at a
/// limit of 0.8 the foci and the tapers alone filled it.
///
/// Leaving what it keeps out of the limit is also what let a character
/// in plate hunt on past twice its capacity, with no defense, before it
/// was laden -- hence the second line. That line gives way when what
/// the character keeps weighs that much on its own: loot cannot slow it
/// any further, and a line no sale can bring it back under would leave
/// every corpse untouched again. The wall never gives way.
fn loot_room(carried: u32, loot: u32, capacity: u32, carry_up_to: f32) -> u32 {
    let up_to = carry_up_to.max(0.0);
    // The server's figure is the one to trust: a count of loot that has
    // got ahead of it is all loot, and nothing is kept.
    let loot = loot.min(carried);
    let kept = carried - loot;
    let limit = (capacity as f32 * up_to) as u32;
    let line = (capacity as f32 * up_to.max(DEFENSELESS_AT)) as u32;
    let wall = capacity.saturating_mul(3);
    let room = limit.saturating_sub(loot).min(wall.saturating_sub(carried));
    if kept < line {
        room.min(line.saturating_sub(carried))
    } else {
        room
    }
}

/// Whether a character with `room` for more loot has had enough and
/// should go and sell: no room at all, or less than the lightest thing
/// the looting left on a body for its weight (`left`) -- as long as
/// selling what it carries would make room for that thing (`sold` is the
/// room it would have then).
///
/// Room used to have to be exactly nothing, and it almost never is. The
/// loot rules take only what fits, so the room settles a little above
/// nothing and below whatever is still lying there: a character forty
/// short of a mace shut every body with one on it as too laden, hunted
/// on, and never went to sell. A thing no sale could make room for is no
/// reason to go.
fn had_enough(room: u32, left: Option<u32>, sold: u32) -> bool {
    room == 0 || left.is_some_and(|burden| room < burden && burden <= sold)
}

/// Whether the party's mode decides when this character goes to town,
/// rather than its own pack and supplies. `mates` is how many others are
/// on the team as it was last heard.
///
/// Restocking together needs somebody to restock with. Blargerton had the
/// team rules on, restocking together on, and nobody else on the team: a
/// party of one only ever decided to go for supplies or a pack with no
/// slot, never for weight, so he hunted on carrying all he meant to and
/// left everything else on the bodies.
fn restocks_as_a_party(team: &crate::autoplay::Team, mates: usize) -> bool {
    team.enabled && team.restock.together && mates > 0
}

/// Whether a character carrying `carried` is past the point where the
/// server hands it anything at all: three times its `capacity`, and
/// weight has nothing to do with it there. A Pyreal weighs nothing, and
/// +Verity at 36462 of a 7500 capacity walked off her way to town for
/// one, was told "You are too encumbered to carry that!", and went back
/// for it twice more. With no capacity known yet nothing is past it.
fn past_the_wall(carried: u32, capacity: u32) -> bool {
    capacity > 0 && carried > capacity.saturating_mul(3)
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

/// What a run on its way to a counter does with no journey under way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OnTheWay {
    /// Near enough to go up to the counter.
    There,
    /// Something else broke the journey off and still has the character.
    Wait,
    /// Something else broke the journey off and is done: set off again.
    WalkOn,
    /// The journey ended by itself short of the counter.
    Short,
}

/// How soon after planning the walk to the counter again a run may plan
/// it once more (see [`on_the_way`]).
const WALK_ON_EVERY: Duration = Duration::from_secs(5);

/// What a walk to a counter or to a hunting ground does with no journey
/// under way. `there` says it is near enough already, `broken_off` that
/// the last journey was ended by something else the character went to do
/// (see `Client::journey_broken_off`), `busy` that it is still doing it,
/// and `lately` that the walk was planned again less than
/// [`WALK_ON_EVERY`] ago. How long the walk may take is the errand's own
/// clock, and is asked before this.
///
/// Any journey not under way used to be one that could not get there.
/// +Verity set off to sell, a corpse took her a second later -- walking
/// to one ends a journey -- and the run gave up 224 m short, marked the
/// counter no use and sold nothing. A walk that ended by itself short of
/// the counter is still one that cannot get there. The walk to a hunting
/// ground did the same, and worse: it put the ground on the skip list and
/// set off for another.
///
/// Planning the walk is a route search, and something that ends the walk
/// as soon as it is planned would have it planned every tick or two until
/// the run's clock ran out: a follower pulled back to its leader was, each
/// time it closed to the following distance. So it waits a moment first.
fn on_the_way(there: bool, broken_off: bool, busy: bool, lately: bool) -> OnTheWay {
    if there {
        OnTheWay::There
    } else if !broken_off {
        OnTheWay::Short
    } else if busy || lately {
        OnTheWay::Wait
    } else {
        OnTheWay::WalkOn
    }
}

/// Something in the pack the rules allow to be sold, before any one
/// counter's tastes are applied to it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Salable {
    pub guid: u32,
    /// `ac_world::item_type` bits.
    pub item_type: u32,
    /// What the whole stack is worth. The server counts a stack's value
    /// as the lot, not as one of them, and a counter's limit is on that
    /// figure -- which is why a hundred Pyreal Peas worth five million
    /// are refused by a counter that will not look at anything over a
    /// million.
    pub value: u32,
    pub stack: u32,
}

impl Salable {
    /// What one of them is worth.
    pub fn each(&self) -> u32 {
        self.value / self.stack.max(1)
    }

    /// How many of them a counter with this ceiling will take at once.
    /// Zero when it will not take even one.
    ///
    /// A stack worth more than the ceiling is not refused for good: it
    /// is split and sold a handful at a time, which is what a player
    /// does and what the counter's limit is for.
    pub fn at_once(&self, max_value: u32) -> u32 {
        if max_value == 0 {
            return self.stack.max(1);
        }
        let each = self.each().max(1);
        (max_value / each).min(self.stack.max(1))
    }

    /// Whether a counter that buys `item_types` between `min_value` and
    /// `max_value` (a maximum of 0 being no maximum) would take it,
    /// whole or in pieces.
    fn taken_by(&self, item_types: u32, min_value: u32, max_value: u32) -> bool {
        self.item_type & item_types != 0 && self.each() >= min_value && self.at_once(max_value) > 0
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

/// What is on its way out of the pack (see [`Client::leaving_of`]),
/// counted: how many of each kind, so that the rest of a kind can
/// still be short.
///
/// Counted, not merely noted. This once held only which kinds were
/// leaving, and a line or a component was dropped from the restock
/// list whenever any stack of it was: one stack of scarabs tagged to
/// sell hid the kept stack's shortfall, a Sell-tagged "Lead Scarab"
/// muted a buy line for "Scarab", and the party heard a different
/// count than the character used. Taking the leaving stacks off what
/// is held answers all three.
#[derive(Default)]
pub(crate) struct Leaving {
    /// How many of each weenie class are leaving.
    by_wcid: BTreeMap<u32, u32>,
    /// Each leaving stack's name, lower-cased, with its count, for the
    /// buy list's lines, which name a thing the way `carried_named`
    /// counts it: by what its name contains.
    names: Vec<(String, u32)>,
    /// How many rounds of ammunition are leaving.
    ammo: u32,
}

impl Leaving {
    /// How many of this weenie class are leaving.
    fn of(&self, wcid: u32) -> u32 {
        self.by_wcid.get(&wcid).copied().unwrap_or(0)
    }

    /// How many of what a buy-list line names are leaving.
    pub(crate) fn named(&self, line: &str) -> u32 {
        let want = line.trim().to_lowercase();
        if want.is_empty() {
            return 0;
        }
        self.names
            .iter()
            .filter(|(n, _)| n.contains(&want))
            .map(|(_, count)| *count)
            .sum()
    }
}

/// What the sale decision needs, gathered once (see
/// [`Client::sell_policy`]).
pub(crate) struct SellPolicy {
    burns: Vec<u32>,
    keep: Vec<String>,
    profile: Option<std::sync::Arc<crate::profile::Profile>>,
    wielder: crate::weapons::Wielder,
    me: String,
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

/// Whether a journey that is not one of the growth rules' errands still
/// has the character on its way somewhere: a road is under way, or one
/// put down for a fight is waiting to be picked up again. `trip` is the
/// journey under way, if any, and whether it is about the character's
/// own ground; `resumed` the same for the journey remembered for after
/// the fight (see `Client::on_its_way`).
///
/// A walk about the ground -- a roam, a patrol -- is never a road: what
/// stands about the ground is what the character came for.
fn road_under_way(trip: Option<bool>, resumed: Option<bool>) -> bool {
    trip.is_some_and(|about_the_ground| !about_the_ground)
        || resumed.is_some_and(|about_the_ground| !about_the_ground)
}

impl Client {
    /// Whether the character is on its way somewhere it decided to go:
    /// out to a hunting ground, back to one from town, or to a counter.
    ///
    /// Two halves, and it takes both.
    ///
    /// Why it is walking comes from the growth rules' errands, not from
    /// [`Client::traveling`], which says a journey is under way and nothing
    /// about why. The patrol and the roam ([`Client::grow_hunt`]) travel
    /// too, about the very ground the character came for, so a rule that
    /// read `traveling` alone would have a character stand in its own
    /// hunting ground refusing to fight. Both set off and return before
    /// `bound` is ever set.
    ///
    /// That it is still walking comes from the journey, because the errand
    /// outlives the walk. `bound` is let go only when the hunting step next
    /// looks, and that is the last goal in the table: a body at the
    /// character's feet keeps it from running, and a party restocking does
    /// not call it at all. Read off `bound` alone, a leader home from town
    /// stood on its ground walking past everything until the slowest of the
    /// party had shopped, and one arriving beside a body looted it before
    /// the monster standing over it. `run` spans the counters as well,
    /// where there is no road. A walk broken off by a corpse or a fight is
    /// still the walk, since its errand takes it up again (see
    /// [`Client::journey_broken_off`]), and so is stepping out of a shop
    /// before it.
    ///
    /// A follower is on its leader's way. It keeps up through the follow
    /// step, which sets no errand of its own, so read off its own errands
    /// alone it stopped to fight what its leader walked past, fell behind,
    /// was fetched back and turned to fight again, and the party came apart
    /// on the road. Each character says on the board whether it is on its
    /// way (`Mate::on_its_way`), which is also how the party stops together
    /// for what attacks any of it (see [`Client::passing_by`]).
    ///
    /// And a road is a road whoever planned it. The errands are the growth
    /// rules' own, so a journey a script asked for, or the walk to the
    /// portal into a hunting area, was not "on its way" at all, and a
    /// party sent down the Singularity Caul by script fought every Biaka,
    /// Hellion and Carenzi between the drop and the far end: fourteen
    /// fights in a hundred and forty seconds, most of them picked by the
    /// party, on a walk of forty. Every journey says whether it is a road
    /// or a walk about the ground the character is hunting (see
    /// [`Client::travel_about`]), and the patrol, the roam and a
    /// follower's own catching up are the only walks about the ground.
    pub fn on_its_way(&self) -> bool {
        let st = &self.autoplay.growth;
        let walking = st.has_an_errand()
            && (self.traveling() || self.journey_broken_off() || st.after_out.is_some());
        walking
            || road_under_way(
                self.traveling().then_some(self.travel_about_the_ground()),
                self.autoplay
                    .resume_trip
                    .map(|_| self.autoplay.resume_about_the_ground),
            )
            || self.followed_leader().is_some_and(|m| m.on_its_way)
    }

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

    // ---- experience -----------------------------------------------

    /// The weapon in hand (not the ammunition), if any.
    fn weapon_in_hand(&self) -> Option<u32> {
        self.world
            .wielded()
            .find(|o| {
                o.item_type & (item_type::MELEE_WEAPON | item_type::MISSILE_WEAPON) != 0
                    && o.valid_locations & equip::MISSILE_AMMO == 0
            })
            .map(|o| o.guid)
    }

    /// The weapon skill the character fights with, from its hands:
    /// `Some(0)` for a caster. An unappraised weapon says nothing about
    /// its skill, so the stance stands in: a bow is Missile Weapons,
    /// and a melee weapon whichever melee skill is trained highest.
    fn fighting_skill(&self) -> Option<u32> {
        use ac_world::stats::sac;
        let stance = self.combat_stance();
        if stance == crate::Stance::Magic {
            return Some(0);
        }
        let weapon = self.weapon_in_hand()?;
        if let Some(id) = self
            .stats_of(weapon)
            .map(|i| i.weapon_skill_id)
            .filter(|id| *id != 0)
        {
            return Some(id);
        }
        if stance == crate::Stance::Missile {
            return Some(47);
        }
        self.world
            .stats
            .skills
            .iter()
            .filter(|s| [41, 44, 45, 46].contains(&s.id) && s.advancement >= sac::TRAINED)
            .max_by_key(|s| (s.advancement, s.ranks))
            .map(|s| s.id)
    }

    /// Everything experience could go on right now, priced and weighed.
    pub fn raise_offers(&self) -> Vec<Offer> {
        use ac_world::stats::sac;
        let weapon_skill = self.fighting_skill();
        let caster = weapon_skill == Some(0);
        let table = self.assets.skill_table().ok();
        let mut offers = Vec::new();
        // The attributes the fighting skill is built from matter as
        // much as a defence; the others little, Endurance apart.
        let mut attr_weight = [0.15f32; 6];
        attr_weight[1] = 0.45;
        let feeds = |skill: u32| -> Vec<u32> {
            table
                .as_ref()
                .and_then(|t| t.get(skill))
                .map(|b| {
                    [b.formula.attr1, b.formula.attr2]
                        .into_iter()
                        .filter(|a| (1..=6).contains(a))
                        .collect()
                })
                .unwrap_or_default()
        };
        let main: Vec<u32> = match weapon_skill {
            Some(0) => vec![
                ac_world::stats::skill::WAR_MAGIC,
                ac_world::stats::skill::LIFE_MAGIC,
            ],
            Some(s) => vec![s],
            None => Vec::new(),
        };
        for s in &main {
            for a in feeds(*s) {
                attr_weight[a as usize - 1] = attr_weight[a as usize - 1].max(0.6);
            }
        }
        for sk in &self.world.stats.skills {
            if sk.advancement < sac::TRAINED {
                continue;
            }
            let weight = skill_weight(sk.id, weapon_skill, caster);
            if let Some(cost) = self.skill_raise_cost(sk.id).xp() {
                offers.push(Offer {
                    raise: Raise::Skill(sk.id),
                    cost,
                    weight,
                });
            }
        }
        for (i, w) in attr_weight.iter().enumerate() {
            if let Some(cost) = self.attribute_raise_cost(i).xp() {
                offers.push(Offer {
                    raise: Raise::Attribute(i),
                    cost,
                    weight: *w,
                });
            }
        }
        let vital_weight = [0.5, 0.2, if caster { 0.5 } else { 0.15 }];
        for (i, w) in vital_weight.iter().enumerate() {
            if let Some(cost) = self.vital_raise_cost(i).xp() {
                offers.push(Offer {
                    raise: Raise::Vital(i),
                    cost,
                    weight: *w,
                });
            }
        }
        offers
    }

    /// Spend experience, as housekeeping: on every tick, whatever else
    /// the character is doing.
    ///
    /// This used to be the first of the growth rules, and those are the
    /// last goal in the table, run only on a tick nothing else claims.
    /// Something always did. A character granted a hundred billion
    /// experience on the local server spent none of it in three minutes,
    /// because exploring had another room to walk to on every one of
    /// those ticks. A raise needs no tick of its own: it is one message --
    /// a rank, or for a large pool a batch of them (see [`batch_raise`])
    /// -- and the pacing keeps it to one at a time: no more often than
    /// [`RAISE_EVERY`], nothing more until the server has answered or
    /// [`RAISE_SETTLE`] has passed, and a raise it would not sell left
    /// alone for [`SULK_FOR`].
    ///
    /// Nor does it wait for a fight to end. ACE takes a raise with no
    /// busy check, no animation and no movement: it checks the stat, the
    /// pool and the rank's ceiling, spends, and answers with the new
    /// record, a sound and a line of chat. None of that touches an attack
    /// or a cast under way. Strength, Quickness and Run can change the run
    /// rate, which ACE sends out again only for a character already
    /// moving and only with `runrate_add_hooks`, off by default -- and
    /// that is a speed, not an action. The server does not mind a rank
    /// that raises a maximum either, but the client's own heal lines do,
    /// and that rank waits (see [`raises_a_maximum`]).
    pub(crate) fn autoplay_spend_xp(&mut self, now: Instant) {
        if self.autoplay.config.growth.auto_xp {
            self.grow_spend_xp(now);
        }
    }

    /// Buy the rank worth buying -- with a large pool, as many of its
    /// ranks as buying a rank at a time would give it -- in one message.
    /// True when one was sent.
    fn grow_spend_xp(&mut self, now: Instant) -> bool {
        let xp = self.world.stats.available_xp;
        if xp <= 0 || self.world.stats.level <= 0 {
            return false;
        }
        let stats = &self.world.stats;
        let st = &mut self.autoplay.growth;
        // The raise last sent is answered when its stat moves.
        if let Some(p) = st.pending {
            if p.raise.standing(stats) != p.was {
                st.pending = None;
            } else {
                match p.since {
                    // Queued on the last look, and on the wire since the
                    // top of this tick: its time starts now. Started when
                    // it was queued, a round of nine headless characters
                    // that took four and a half seconds gave up on nine
                    // raises in the tick they were sent, and every one of
                    // them landed a third of a second later.
                    None => {
                        st.pending = Some(Pending {
                            since: Some(now),
                            ..p
                        });
                        return false;
                    }
                    Some(since) if now.duration_since(since) < RAISE_SETTLE => return false,
                    // No answer in time: refused, or only late. It is left
                    // alone for a while, and keeps its share of the pool.
                    Some(_) => {
                        tracing::info!(
                            "growth: no answer to a raise of {} ({} ranks for {} xp)",
                            p.raise.name(),
                            p.batch.ranks,
                            p.batch.xp
                        );
                        st.sulking.push((p, now));
                        st.pending = None;
                    }
                }
            }
        }
        // An answer that comes in late ends the wait; the rest wear off.
        // Either can leave something to buy that was not there.
        let sulks = st.sulking.len();
        st.sulking.retain(|(p, since)| {
            let landed = p.raise.standing(stats) != p.was;
            if landed {
                tracing::info!("growth: the raise of {} came in late", p.raise.name());
            }
            !landed && now.duration_since(*since) < SULK_FOR
        });
        if st.sulking.len() < sulks {
            st.nothing_at = None;
        }
        if st
            .last_raise
            .is_some_and(|t| now.duration_since(t) < RAISE_EVERY)
        {
            return false;
        }
        let fighting = self.in_a_fight();
        if let Some((at, was_fighting, when)) = self.autoplay.growth.nothing_at {
            if at == xp && was_fighting == fighting && now.duration_since(when) < XP_CHECK_EVERY {
                return false;
            }
        }
        // The weapon's skill is read off its appraisal.
        if let Some(w) = self.weapon_in_hand() {
            if !self.appraisals.contains_key(&w) {
                self.appraise_many([w]);
            }
        }
        let sulking: Vec<Raise> = self
            .autoplay
            .growth
            .sulking
            .iter()
            .map(|(p, _)| p.raise)
            .collect();
        let Ok(table) = self.assets.xp_table() else {
            self.autoplay.growth.nothing_at = Some((xp, fighting, now));
            return false;
        };
        // A rank that raises a maximum waits for the fight to be over.
        // The fight under way is fought with what is left, which the rank
        // does not raise, and the fraction left drops as surely as it
        // would for a blow: a character with a large pool bought Health
        // between swings until it healed, mid-fight, health it had never
        // lost. Skills and the other attributes count at once, and go on
        // being bought. A stat whose raise went unanswered waits too. Both
        // are held, not left out: the pool is spread over them all the
        // same, and what is theirs waits for them (see [`batch_raise`]).
        // A pool with nothing to buy but what is held is noted as having
        // nothing until it moves or the fight ends.
        let field: Vec<Climb> = self
            .raise_offers()
            .into_iter()
            .filter_map(|o| {
                Some(Climb {
                    raise: o.raise,
                    ladder: self.raise_ladder(&table, o.raise)?,
                    weight: o.weight,
                    held: (fighting && raises_a_maximum(o.raise)) || sulking.contains(&o.raise),
                })
            })
            .collect();
        let Some((pick, batch)) = batch_raise(&field, xp) else {
            self.autoplay.growth.nothing_at = Some((xp, fighting, now));
            return false;
        };
        let was = pick.standing(&self.world.stats);
        if !self.raise_by(pick, batch.xp) {
            self.autoplay.growth.nothing_at = Some((xp, fighting, now));
            return false;
        }
        let st = &mut self.autoplay.growth;
        st.last_raise = Some(now);
        st.pending = Some(Pending {
            raise: pick,
            batch,
            was,
            since: None,
        });
        st.nothing_at = None;
        // A note, not a status. This is housekeeping, and the status line
        // belongs to whatever claims the tick: said here, a character with
        // nothing else to do would go from "waiting" to "spending
        // experience" and back again with every rank.
        let what = match batch.ranks {
            1 => pick.name().to_string(),
            n => format!("{} {n} ranks", pick.name()),
        };
        self.autoplay
            .note(format!("raising {what} ({xp} xp to spend)"), now);
        true
    }

    // ---- journeys -------------------------------------------------

    /// Set off for `goal`. From inside a building the planner sees no
    /// way out but a portal, so the walk begins with the spot the
    /// character last stood outdoors, and the journey proper follows
    /// (see [`Self::grow_travel_on`]). False when no way was found.
    fn grow_travel(&mut self, goal: Vec2, now: Instant) -> bool {
        self.autoplay.growth.after_out = None;
        let Some(pl) = self.player.as_ref() else {
            return false;
        };
        // The shortcut below is for buildings. A dungeon has no door to
        // step out of -- the way out is its exit portal or a recall --
        // and walking at the last outdoor spot from underground is
        // walking at rock.
        if pl.is_indoors() && !self.autoplay.growth.in_dungeon {
            let me = pl.world_position();
            let block = pl.cell & 0xFFFF_0000;
            let same_block =
                |p: Vec2| (((p.x / 192.0) as u32) << 24) | (((p.y / 192.0) as u32) << 16) == block;
            if let Some(out) = self.autoplay.growth.last_outdoors {
                if same_block(out)
                    && out.distance(Vec2::new(me.x, me.y)) < 150.0
                    && self.travel_to(out)
                {
                    self.autoplay.growth.after_out = Some(goal);
                    self.autoplay.note("stepping outside first", now);
                    return true;
                }
            }
        }
        self.travel_to(goal)
    }

    /// The second leg of a journey begun indoors: once the walk out has
    /// ended, the journey proper. True when one was started.
    fn grow_travel_on(&mut self) -> bool {
        if self.traveling() {
            return false;
        }
        let Some(goal) = self.autoplay.growth.after_out.take() else {
            return false;
        };
        if self.travel_to(goal) {
            return true;
        }
        tracing::info!("growth: no way on to {goal:?} from outside either");
        false
    }

    // ---- hunting grounds ------------------------------------------

    /// The hunting ground this character is on or heading for.
    pub fn hunting_ground(&self) -> Option<(u32, Vec2, String)> {
        let st = &self.autoplay.growth;
        st.bound.clone().or_else(|| {
            let lb = st.hunting_at?;
            let g = ac_world::hunting::all()
                .iter()
                .find(|g| g.landblock == lb)?;
            Some((lb, g.at, g.name.clone()))
        })
    }

    /// The ground the party is on, for a character that does not choose
    /// its own.
    ///
    /// A party that walks out of town each picking the nearest ground
    /// to whichever shop it finished at ends up in four different
    /// places. The leader's ground is the party's, so everyone comes
    /// back to the same one.
    fn party_ground(&self) -> Option<(u32, Vec2, String)> {
        let team = &self.autoplay.config.team;
        if !team.enabled || team.lead {
            return None;
        }
        self.autoplay
            .team
            .leader_mate()
            .and_then(|m| m.ground.clone())
    }

    /// Keep the two clocks that are read off the ground rather than off
    /// what the character is doing: how long this spot has been quiet,
    /// and when each body lying on it first came into sight.
    ///
    /// The quiet clock starts when there is nothing here the character
    /// would fight and stops the moment something is (see
    /// [`Client::a_fight_in_sight`]). It reads the world rather than
    /// the status line: read off the status line it measured how busy
    /// the character was instead of how quiet the ground was, so
    /// re-asking a corpse that would not open counted as having
    /// something to do and put the clock back to nothing, and nine
    /// characters queueing at one body restarted it every couple of
    /// seconds and it never reached its minute once in ten.
    ///
    /// Both clocks are wound here, before anything can claim the tick,
    /// for the same reason: the steps that read them are not reached on
    /// every tick, and a clock only wound where it is read stands still
    /// exactly when it is most needed. The hunting step is the last goal
    /// in the table, so a character with a body to open never reaches
    /// it. Worse, the bodies used to be noted inside the looting: a
    /// character the claim tie-break told to stand off scores that body
    /// nothing, so its loot goal is never run, so it never notes the
    /// body, so the body stays for ever "newly fallen" and the tie-break
    /// governs for the whole five minutes it lies there. One mate that
    /// could not loot -- a full pack, no profile, looting turned off --
    /// then locked every body within twenty metres of it away from the
    /// other eight for good.
    pub(crate) fn autoplay_watch_the_ground(&mut self, now: Instant) {
        let quiet = !self.a_fight_in_sight(now);
        let fresh: Vec<u32> = self
            .world
            .objects
            .values()
            .filter(|o| o.object_desc_flags & object_desc_flags::CORPSE != 0)
            .map(|o| o.guid)
            .filter(|g| !self.autoplay.corpse_seen.iter().any(|(seen, _)| seen == g))
            .collect();
        let st = &mut self.autoplay;
        st.corpse_seen.extend(fresh.into_iter().map(|g| (g, now)));
        // Forgotten once emptied, so the list stays the size of what is
        // on the ground.
        let looted = &st.looted;
        st.corpse_seen
            .retain(|(g, t)| now.duration_since(*t) < CORPSE_LIFE * 2 && !looted.contains(g));
        let st = &mut st.growth;
        if quiet {
            st.quiet_since.get_or_insert(now);
        } else {
            st.quiet_since = None;
        }
    }

    /// Go somewhere with monsters when there have been none for a
    /// while. True while it is on the way.
    fn grow_hunt(&mut self, now: Instant, cfg: &Growth) -> bool {
        let Some((me, cell)) = self.player.as_ref().map(|p| (p.world_position(), p.cell)) else {
            return false;
        };
        let me = Vec2::new(me.x, me.y);
        let here = cell >> 16;
        // On the way: keep going, and notice arriving.
        if let Some((lb, at, name)) = self.autoplay.growth.bound.clone() {
            let too_long = self
                .autoplay
                .growth
                .bound_since
                .is_some_and(|t| now.duration_since(t) > WALK_TIMEOUT);
            if too_long {
                self.autoplay.note(
                    format!("the walk to the {name} ground is taking too long"),
                    now,
                );
                self.cancel_travel();
            } else if self.traveling() || self.grow_travel_on() {
                self.autoplay.say(
                    Doing::Traveling,
                    format!("going to hunt {name} ({})", about(at.distance(me))),
                );
                return true;
            }
            // A corpse or a fight on the road ends the walk without it
            // having got anywhere. That is a walk to take up again once the
            // character is free, as the run takes up its walk to the
            // counter, and not one that could not get there: read that way,
            // every body looted on the road put the ground on the skip list
            // and sent the character off to another. A walk that took too
            // long was cancelled above, which is not breaking it off.
            //
            // Busy is a target still alive. `attack_target` can go on naming
            // a creature for a moment after it dies (see
            // `steps::worth_fighting`), and a body is no reason to wait.
            let away = at.distance(me);
            let there = here == lb || away <= GROUND_REACH;
            let busy = [self.attack_target, self.autoplay.casting_at()]
                .into_iter()
                .flatten()
                .any(|g| {
                    self.world
                        .objects
                        .get(&g)
                        .is_some_and(|o| o.health.unwrap_or(1.0) > 0.0)
                });
            let lately = self
                .autoplay
                .growth
                .bound_walked_on
                .is_some_and(|t| now.duration_since(t) < WALK_ON_EVERY);
            match on_the_way(there, self.journey_broken_off(), busy, lately) {
                OnTheWay::Wait => return true,
                OnTheWay::WalkOn => {
                    if self.grow_travel(at, now) {
                        self.autoplay.growth.bound_walked_on = Some(now);
                        self.autoplay.note(
                            format!("on the way to the {name} ground again ({away:.0} m)"),
                            now,
                        );
                        return true;
                    }
                }
                OnTheWay::There | OnTheWay::Short => {}
            }
            let st = &mut self.autoplay.growth;
            st.bound = None;
            if there {
                st.hunting_at = Some(lb);
                st.quiet_since = None;
                st.roams = 0;
                self.autoplay
                    .note(format!("at the hunting ground: {name}"), now);
            } else {
                st.skip.push((lb, now));
                self.autoplay
                    .note(format!("could not reach the {name} ground; another"), now);
            }
            return false;
        }
        let level = self.world.stats.level;
        if level <= 0 {
            return false;
        }
        // How long this spot has had nothing on it is kept by the
        // housekeeping, off the world (see
        // `Client::autoplay_watch_the_ground`). It used to be read off
        // the status line here, and so could never reach its hour: a
        // corpse asked about again counted as having something to do and
        // put the clock back to nothing, so nine characters queueing at
        // one body restarted it every couple of seconds and never once
        // walked the two hundred metres they had been given.
        let Some(since) = self.autoplay.growth.quiet_since else {
            return false;
        };
        // A walk already under way is the looking about: it is not cut
        // short to start another.
        if self.traveling() {
            return false;
        }
        let area = self.autoplay.config.fight.area.clone();
        if now.saturating_duration_since(since).as_secs_f32()
            < quiet_before_move(cfg, area.is_some())
        {
            return false;
        }
        if self.autoplay.growth.next_hunt.is_some_and(|t| now < t) {
            return false;
        }
        // In a party only the leader goes looking. The others keep to it
        // (see `Client::autoplay_follow`), and nine characters each
        // picking their own corner scatter the party across the ground
        // instead of moving it.
        let goes_looking = self.followed_leader().is_none();
        // A leader moves the party, not itself: it waits for a follower
        // still fighting or left behind before it goes anywhere (see
        // `Client::waits_for_stragglers`).
        if goes_looking && self.waits_for_stragglers(now) {
            return false;
        }
        // A hunting area is where the hunting is: no other ground is gone
        // to. An outline is walked about, corner by corner, well inside
        // it; a dungeon is the explorer's to walk.
        if let Some(area) = area {
            if !goes_looking {
                return false;
            }
            let Some(corners) = crate::hunt::corners(&area) else {
                return false;
            };
            if corners.len() < crate::hunt::LEAST_CORNERS {
                return false;
            }
            let n = self.autoplay.growth.roams;
            self.autoplay.growth.roams = n.wrapping_add(1);
            self.autoplay.growth.quiet_since = Some(now);
            let goal = crate::hunt::patrol_point(&corners, n);
            if self.travel_about(goal) {
                self.autoplay.say(
                    Doing::Traveling,
                    format!("nothing in sight; looking about {}", area.name),
                );
                return true;
            }
            return false;
        }
        // What to do on a ground with nothing in sight depends on the
        // ground. A few spawn hard enough that standing still is never
        // idle and walking away only leaves the fight; most need
        // covering on foot; a thin one is worth leaving once it is
        // quiet.
        let tactic = ac_world::hunting::tactic_for(cfg.tactic, ac_world::hunting::at(here));
        if tactic == ac_world::hunting::Tactic::Camp
            && self.autoplay.growth.hunting_at == Some(here)
        {
            // Hold the spot. Saying so once is enough; repeating it
            // every frame would drown the log.
            if self.autoplay.doing != Doing::Idle {
                self.autoplay
                    .say(Doing::Idle, "holding the spot; they come to us");
            }
            self.autoplay.growth.quiet_since = Some(now);
            return false;
        }
        // Patrol keeps walking the ground; only Sweep gives up on it.
        let roams_allowed = if tactic == ac_world::hunting::Tactic::Patrol {
            u32::MAX
        } else {
            ROAMS
        };
        if goes_looking
            && self.autoplay.growth.hunting_at == Some(here)
            && self.autoplay.growth.roams < roams_allowed
        {
            let n = self.autoplay.growth.roams % ROAMS;
            let angle = (n as f32 + 0.5) * std::f32::consts::TAU / ROAMS as f32;
            let origin = ac_world::landblock_origin(cell);
            let goal = (me + Vec2::new(angle.cos(), angle.sin()) * ROAM).clamp(
                Vec2::new(origin.x + 10.0, origin.y + 10.0),
                Vec2::new(origin.x + 182.0, origin.y + 182.0),
            );
            self.autoplay.growth.roams += 1;
            self.autoplay.growth.quiet_since = Some(now);
            if self.travel_about(goal) {
                self.autoplay.say(
                    Doing::Traveling,
                    "nothing in sight; looking about the ground",
                );
                return true;
            }
        }
        let st = &mut self.autoplay.growth;
        st.skip
            .retain(|(_, t)| now.duration_since(*t) < SKIP_GROUND_FOR);
        let mut skip: Vec<u32> = st.skip.iter().map(|(g, _)| *g).collect();
        skip.push(here);
        if let Some(h) = st.hunting_at {
            skip.push(h);
        }
        // A ground the player named is where the party hunts. Not the
        // nearest one that suits its level: a place is hunted for its
        // loot, its money, its trophies, and none of that is the level
        // table's business. A follower takes its leader's ground the
        // same way, so naming one on the leader moves everybody.
        let pinned = (cfg.hunt_at != 0)
            .then(|| ac_world::hunting::at(cfg.hunt_at))
            .flatten()
            .map(|g| (g.landblock, g.at, g.name.clone()));
        let named = pinned.is_some();
        if let Some((lb, at, name)) = pinned.or_else(|| self.party_ground()) {
            if self.autoplay.growth.hunting_at != Some(lb) {
                if self.grow_travel(at, now) {
                    let st = &mut self.autoplay.growth;
                    st.bound = Some((lb, at, name.clone()));
                    st.bound_since = Some(now);
                    st.quiet_since = None;
                    self.autoplay
                        .say(Doing::Traveling, format!("on the way to {name}"));
                    return true;
                }
                if named {
                    // Asked for somewhere it cannot plan a way to. Say
                    // so and try again later; picking somewhere else
                    // would be answering a question nobody asked.
                    self.autoplay.note(
                        format!("cannot find a way to {name} from here; will try again"),
                        now,
                    );
                    self.autoplay.growth.quiet_since = Some(now);
                    self.autoplay.growth.next_hunt = Some(now + RETRY_AFTER);
                    return false;
                }
            } else if named {
                // Standing on the named ground: this is where we hunt,
                // and nothing below gets to move us on.
                self.autoplay.growth.hunting_at = Some(lb);
            }
        }
        if named {
            // The player named this ground. Whatever the tactic makes
            // of a quiet spell, it does not get to go somewhere else.
            return false;
        }
        // Everything below picks a ground of its own, which is the
        // leader's to do. A follower has just skipped the roam for the
        // same reason, and falling through to here from that made the
        // scattering the roam's guard was added to stop: the follower
        // left for a landblock of its own the first time its ground went
        // quiet, instead of staying with the party.
        if !goes_looking {
            return false;
        }
        let Some(g) = ac_world::hunting::nearest_for(level as u32, cfg.level_margin, me, &skip)
        else {
            self.autoplay.note(
                format!(
                    "no hunting ground suits level {level} within {} levels",
                    cfg.level_margin
                ),
                now,
            );
            self.autoplay.growth.quiet_since = Some(now);
            return false;
        };
        let (lb, at, name) = (g.landblock, g.at, g.name.clone());
        let (glo, ghi) = (g.min_level, g.max_level);
        if self.grow_travel(at, now) {
            let st = &mut self.autoplay.growth;
            st.bound = Some((lb, at, name.clone()));
            st.bound_since = Some(now);
            st.quiet_since = None;
            if let Some(h) = st.hunting_at.take() {
                st.skip.push((h, now));
            }
            self.autoplay.say(
                Doing::Traveling,
                format!(
                    "nothing about; going to hunt {name} (levels {glo}-{ghi}, {})",
                    about(at.distance(me))
                ),
            );
            true
        } else {
            let st = &mut self.autoplay.growth;
            st.skip.push((lb, now));
            st.quiet_since = None;
            st.next_hunt = Some(now + RETRY_AFTER);
            self.autoplay
                .note(format!("no way to the {name} ground from here"), now);
            false
        }
    }

    // ---- town runs ------------------------------------------------

    /// Ammunition carried, wielded and in the packs, for a bow or
    /// crossbow in hand: `(kind, count)`.
    fn ammo_carried(&self) -> Option<(u32, u32)> {
        let launcher = self
            .wielded_missile_weapon()
            .and_then(|g| self.stats_of(g))
            .filter(crate::weapons::is_launcher)?;
        if launcher.ammo_type == 0 {
            return None;
        }
        let me = self.world.player_guid;
        let count: u32 = self
            .world
            .objects
            .values()
            .filter(|o| o.valid_locations & equip::MISSILE_AMMO != 0)
            .filter(|o| o.wielder == me || self.world.is_carried(o.guid))
            .map(|o| o.stack_size.max(1))
            .sum();
        Some((launcher.ammo_type, count))
    }

    /// Whether more ammunition of `kind` could be made from what is
    /// carried (see `autoplay::Fight::craft_ammo`).
    fn can_craft_ammo(&self, kind: u32) -> bool {
        if !self.autoplay.config.fight.craft_ammo {
            return false;
        }
        let carried: Vec<u32> = self.world.inventory().map(|o| o.weenie_class_id).collect();
        ac_world::fletching::making(kind)
            .any(|r| carried.contains(&r.source) && carried.contains(&r.target))
    }

    /// How many of a named thing are carried (name contains, as the
    /// team's `keep_stocked` counts).
    pub(crate) fn carried_named(&self, name: &str) -> u32 {
        let want = name.trim().to_lowercase();
        self.world
            .inventory()
            .filter(|o| o.name.to_lowercase().contains(&want))
            .map(|o| o.stack_size.max(1))
            .sum()
    }

    /// What is on its way out of the pack: every carried thing the
    /// profile said to sell and the server will let go (see
    /// [`ac_loot::sale::fate`]), counted by weenie class and by name.
    ///
    /// What is held is counted less this before anything is called
    /// short. A thing that is leaving is not stock, whatever else says
    /// it is -- the buy list, the component table, the formulas of the
    /// spells this character casts -- because the player decided it.
    /// It is not the whole of the answer, though: what leaves is
    /// forgotten with it, and whether more of the kind should then be
    /// bought is a question about the kind, which the profile answers
    /// (see [`Client::bought_to_sell`]).
    pub(crate) fn leaving_of(&self, stats: &[ItemStats]) -> Leaving {
        let mut out = Leaving::default();
        for s in stats {
            if fate(self.autoplay.ledger.of(s), never_sell(s)) != Fate::Leaving {
                continue;
            }
            let count = s.stack.max(1);
            *out.by_wcid.entry(s.wcid).or_insert(0) += count;
            out.names.push((s.name.to_lowercase(), count));
            if s.valid_locations & equip::MISSILE_AMMO != 0 {
                out.ammo += count;
            }
        }
        out
    }

    /// Whether `count` of this spell component, bought at a counter,
    /// would be written down as loot to sell the moment they arrived
    /// -- in which case buying them is a round trip at the counter's
    /// markup, and they are not stock.
    ///
    /// Asked of the profile the way the arrival pass will ask it
    /// (`autoplay_tag_arrivals`): the same rules, over the thing a
    /// purchase arrives as. A verdict about the *kind* rather than
    /// about a stack in the pack, which is what the restock list
    /// needs: the moment a tagged stack goes over the counter its tag
    /// is forgotten with it, and a list that only asked "is any of
    /// this leaving?" bought the scarabs straight back in the same
    /// visit, the arrival pass tagged them to sell, and the next trip
    /// sold them again.
    ///
    /// `held` is what will be held when the purchase arrives, less
    /// anything leaving: the count a `keep_up_to` rule, or the buy
    /// list's own line, is judged against.
    fn bought_to_sell(&self, wcid: u32, name: &str, count: u32, held: u32) -> bool {
        let Some(profile) = self.loot_profile() else {
            return false;
        };
        // Worth what a counter lists it at: the one in front of the
        // character, else the world's shops. A rule that sells by value
        // is asked about the stack a purchase makes.
        let each = self
            .world
            .open_vendor
            .as_ref()
            .and_then(|v| {
                v.items
                    .iter()
                    .find(|w| w.desc.weenie_class_id == wcid)
                    .map(|w| w.desc.value)
            })
            .or_else(|| {
                ac_world::shops::all()
                    .iter()
                    .flat_map(|s| &s.sells)
                    .find(|w| w.wcid == wcid)
                    .map(|w| w.value)
            })
            .unwrap_or(0);
        let count = count.max(1);
        let arrives = ItemStats {
            name: name.to_string(),
            wcid,
            item_type: item_type::SPELL_COMPONENTS,
            kind: crate::items::kind_name(item_type::SPELL_COMPONENTS),
            stack: count,
            max_stack: 1_000,
            value: each.saturating_mul(count),
            ..Default::default()
        };
        matches!(
            crate::autoplay::judge_loot(
                &arrives,
                None,
                Some(&profile),
                &self.wielder(),
                &self.world.stats.name,
                held,
            ),
            crate::profile::Verdict::Decided(LootAction::Sell, _)
        )
    }

    /// What the character is short of.
    fn grow_needs(&self, cfg: &Growth) -> Vec<Need> {
        self.grow_needs_with(cfg, &self.item_stats())
    }

    /// The same, over a pack already read: the counter's snapshot reads
    /// it once for the sale and once more here, every tick of the
    /// shopping loop, and the second reading is the same pack.
    fn grow_needs_with(&self, cfg: &Growth, stats: &[ItemStats]) -> Vec<Need> {
        let mut needs = Vec::new();
        let leaving = self.leaving_of(stats);
        // What is held as stock: what is carried, less what is on its
        // way out. Both the character and the party read this count
        // (see `autoplay_stock`), so they agree on what is short.
        let stock = |what: &str| self.carried_named(what).saturating_sub(leaving.named(what));
        // The profile's buy list first: it is where a player says what
        // to keep stocked now, and it is the same list that makes those
        // things unsellable. `keep_stocked` is what it grew out of and
        // is still read, so nobody's settings go quiet.
        // The profile's buy list, through the profile's own reckoning
        // of what is short. `grow_needs` used to re-implement that
        // filter, which is two statements of one rule and the way they
        // come to disagree.
        let named: Vec<(String, u32, u32, Option<String>, bool)> = self
            .profiles
            .get(&self.autoplay.config.loot.profile)
            .map(|p| {
                p.shortfall(stock)
                    .into_iter()
                    .map(|s| {
                        (
                            s.want.what.clone(),
                            s.have,
                            s.want.keep,
                            s.want.from.clone(),
                            s.urgent,
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (name, have, least, from, urgent) in named {
            if name.trim().is_empty() || least == 0 {
                continue;
            }
            if !worth_stocking(&name, self.heals_with_kits()) {
                continue;
            }
            if have < least
                && !needs
                    .iter()
                    .any(|n: &Need| n.kind == NeedKind::Named(name.clone()))
            {
                needs.push(Need {
                    name: name.clone(),
                    want: least - have,
                    have,
                    keep: least,
                    // Being one short is not a reason to walk to town:
                    // the line says how low it may get first.
                    urgent,
                    buyable: true,
                    from,
                    kind: NeedKind::Named(name),
                });
            }
        }
        if let Some((kind, carried)) = self.ammo_carried() {
            // Less the rounds the player said to sell: ammunition the
            // profile is selling out of is loot in the ammunition
            // slot, not stock, and it goes over the counter. The
            // launcher is still short by what is left after it does.
            let have = carried.saturating_sub(leaving.ammo);
            if have < cfg.ammo_keep {
                needs.push(Need {
                    name: ac_world::fletching::ammo_type::name(kind).to_string(),
                    want: cfg.ammo_keep - have,
                    have,
                    keep: cfg.ammo_keep,
                    urgent: have < cfg.ammo_keep / 4 && !self.can_craft_ammo(kind),
                    buyable: true,
                    from: None,
                    kind: NeedKind::Ammo(kind),
                });
            }
        }
        // Components: for a character with spells and a wand. Those it
        // carries, and those its buffs ask for that it has run out of.
        // How many tapers to carry, from the buy list. Everything else
        // a caster burns is scaled to it (see `component_targets`),
        // which is why one number buys forty kinds of thing.
        let tapers = self
            .profiles
            .get(&self.autoplay.config.loot.profile)
            .map_or(0, |p| p.stocked_count("Prismatic Taper"));
        if tapers > 0 && !self.world.stats.spells.is_empty() {
            let has_wand = self.wielded_caster().is_some()
                || self
                    .world
                    .inventory()
                    .any(|o| o.item_type & item_type::CASTER != 0);
            if has_wand {
                if let Ok(mapper) = self.assets.spell_component_ids() {
                    let targets = self.component_targets(tapers);
                    // Every component the spells this character casts
                    // will burn, and nothing else.
                    //
                    // The targets are what matters. Asking only what is
                    // in the pack, as this once did, leaves a caster
                    // that has run right out of something unable to
                    // notice: with none of it carried there is nothing
                    // to count, so nothing is short, so it never goes
                    // to town for more.
                    //
                    // And what is carried is not asked at all. It used
                    // to be added to the targets, at the taper count
                    // for want of a burn rate, and the peas a mage
                    // loots sit in the component table too (seventy-odd
                    // of them, at 113-186, 189 and 191): each became a
                    // need for ninety-nine more. A thing no cast burns
                    // is not stock, whatever table it is in, and the
                    // formulas already say which things are burnt. The
                    // spells autoplay casts, not every spell in the
                    // book, so nothing is bought for a spell that is
                    // never cast.
                    //
                    // That is a rule about what to stock, and it is not
                    // what keeps the player's loot off this list. The
                    // profile does: a stack tagged to sell is not
                    // counted as stock even when a spell burns it, and
                    // a kind the rules would tag to sell the moment it
                    // was bought is not bought, because a need for it
                    // would only buy back at markup what the counter
                    // was just handed, to be sold again next trip.
                    let carried = self.components();
                    for (&id, &keep) in &targets {
                        let Some(wcid) = mapper.component_wcid(id) else {
                            continue;
                        };
                        // What this one burns at, not what a taper does.
                        let c = carried.iter().find(|c| c.component_id == id);
                        let have = c.map_or(0, |c| c.count).saturating_sub(leaving.of(wcid));
                        let buyable = ac_world::shops::sold_anywhere(wcid);
                        if have < keep {
                            // What it is called in the client's own
                            // component table, which is what a vendor
                            // and a person both call it. The enum name
                            // is the last resort: "LeadScarab" is a
                            // symbol, not a thing you can ask for.
                            let name = c
                                .map(|c| c.name.clone())
                                .or_else(|| {
                                    self.assets
                                        .spell_components()
                                        .ok()
                                        .and_then(|t| t.get(id).map(|c| c.name.clone()))
                                })
                                .or_else(|| mapper.name_of(id).map(str::to_string))
                                .unwrap_or_else(|| format!("component {id}"));
                            if self.bought_to_sell(wcid, &name, keep - have, have) {
                                continue;
                            }
                            needs.push(Need {
                                name,
                                want: keep - have,
                                have,
                                keep,
                                urgent: have < keep / 4 && buyable,
                                buyable,
                                from: None,
                                kind: NeedKind::Component(wcid),
                            });
                        }
                    }
                }
            }
        }
        needs
    }

    /// Pyreals carried.
    pub(crate) fn purse(&self) -> u32 {
        self.world
            .inventory()
            .filter(|o| o.item_type & item_type::MONEY != 0)
            .map(|o| o.stack_size.max(1))
            .sum()
    }

    /// Work out what the party is doing and remember it.
    ///
    /// Every session runs this on the same roster and reaches the same
    /// answer, so there is nothing to agree on: the party changes mode
    /// together without a message being sent about it. A character
    /// playing alone -- nobody else on the team, whatever its settings
    /// say -- or one whose team rules are off, is always hunting: its
    /// own pack and supplies send it to town, by the older rule (see
    /// [`restocks_as_a_party`]).
    fn grow_mode(&mut self, now: Instant, cfg: &Growth) -> crate::logistics::GroupMode {
        use crate::logistics::{decide, GroupMode};
        let team = &self.autoplay.config.team;
        if !restocks_as_a_party(team, self.autoplay.team.mates.len()) {
            self.autoplay.growth.mode = GroupMode::Hunting;
            return GroupMode::Hunting;
        }
        let policy = team.restock.clone();
        let mut party: Vec<Supplies> = self
            .autoplay
            .team
            .mates
            .iter()
            .map(|m| m.supplies.clone())
            .collect();
        party.push(self.supplies(cfg, now));
        let was = self.autoplay.growth.mode;
        // A trip that has dragged on has failed at something no rule
        // here can see: a vendor out of tapers, a purse that ran dry, a
        // character that died on the way. Waiting at the hunting ground
        // for ever is worse than hunting undersupplied, so the party
        // gives up and goes back to it.
        let stalled = !was.hunting()
            && policy.give_up_after > 0.0
            && self
                .autoplay
                .growth
                .mode_since
                .is_some_and(|t| now.duration_since(t).as_secs_f32() > policy.give_up_after);
        if stalled {
            let st = &mut self.autoplay.growth;
            st.mode = GroupMode::Hunting;
            st.mode_since = Some(now);
            st.mode_because = "the trip to town took too long".to_string();
            st.handed_over = false;
            st.round = 0;
            self.autoplay.note(
                "giving up on the trip to town and going back to hunting",
                now,
            );
            return GroupMode::Hunting;
        }
        if let Some(switch) = decide(was, &party, &policy, self.autoplay.growth.round) {
            let st = &mut self.autoplay.growth;
            // Back to the start of a trip is another round; going home
            // starts the count again.
            st.round = match switch.mode {
                GroupMode::Hunting => 0,
                GroupMode::Restocking(Stage::HandOver) if !was.hunting() => st.round + 1,
                GroupMode::Restocking(_) => st.round,
            };
            st.mode = switch.mode;
            st.mode_since = Some(now);
            st.mode_because = switch.because.clone();
            // A new stage is a new set of errands; nothing carries over.
            st.handed_over = false;
            self.autoplay.note(
                format!("the party is {}: {}", switch.mode, switch.because),
                now,
            );
        }
        self.autoplay.growth.mode
    }

    /// Free item slots as one take can use them: the most room any one
    /// pack has (see `room::Packs::for_a_take`). A take goes into the
    /// pack it names, so the slots of the main pack and each side pack
    /// are not added: added, a character with every pack down to its
    /// last slot read as having room for a dozen takes.
    pub fn free_space(&self) -> u32 {
        self.packs().for_a_take()
    }

    /// Free item slots as the server spreads what it creates over them:
    /// every pack's room together (see `room::Packs::anywhere`). A
    /// counter's payout, a purchase and a gift are created in the main
    /// pack and spill into the side packs, and a sale is judged
    /// against this same total.
    pub fn room_anywhere(&self) -> u32 {
        self.packs().anywhere()
    }

    /// No pack has a slot for a take, or the packs together are down to
    /// the slots kept free for a counter's money (`restock.keep_slots`):
    /// time to sell, while a sale can still be paid for. The server
    /// finds room for the coin before it takes the goods, so a pack with
    /// no slot at all cannot be sold out of -- and it finds that room
    /// in any pack, so the slots kept are counted over all of them.
    /// Counted in the one pack a take could use, a character with two
    /// slots in the main pack and two in the sack went to town with
    /// room for its money twice over.
    pub fn pack_low_on_room(&self) -> bool {
        let packs = self.packs();
        packs.for_a_take() == 0 || packs.anywhere() <= self.autoplay.config.team.restock.keep_slots
    }

    /// What the character has room for, as a corpse waiting on it sees
    /// it (see `Autoplay::corpse_waiting`).
    pub(crate) fn room_for_loot(&self) -> crate::autoplay::Room {
        let (carried, capacity) = self.burden();
        crate::autoplay::Room {
            pack_low: self.pack_low_on_room(),
            past_the_wall: past_the_wall(carried, capacity),
            // Weighed only while a body is waiting on it: what the loot
            // weighs is judged item by item, and this is asked on every
            // tick a corpse lies about.
            carry: if self.autoplay.left_for_weight.is_empty() {
                u32::MAX
            } else {
                self.carry_room(&self.autoplay.config.growth)
            },
        }
    }

    /// What the character can spend. Coin and trade notes both: a note
    /// is money in a lighter form, and a vendor takes either.
    pub fn spendable(&self) -> u32 {
        let notes: u32 = self
            .world
            .inventory()
            .filter(|o| o.item_type & item_type::PROMISSORY_NOTE != 0)
            .map(|o| o.value.saturating_mul(o.stack_size.max(1)))
            .sum();
        self.purse().saturating_add(notes)
    }

    /// What this character says about itself for the party to decide
    /// with: how close to empty it is, what it still has to buy, and
    /// what that will cost.
    ///
    /// The level is the worst supply line, not the average. A mage with
    /// a full load of scarabs and no tapers cannot cast, and averaging
    /// the two would hide that.
    pub fn supplies(&self, cfg: &Growth, now: Instant) -> Supplies {
        let needs = self.grow_needs(cfg);
        // Loot for a counter worth a trip by this character's own rules
        // (see [`worth_a_sale_run`]), said to the party as one word.
        let salables = self.salables(cfg);
        let carried_for = self
            .autoplay
            .growth
            .sale_since
            .map(|t| now.duration_since(t));
        let sale = worth_a_sale_run(&salables, carried_for, cfg).is_some();
        // The worst line the party can do anything about. A Void mage
        // is permanently out of Nightshade -- no counter in Dereth
        // sells it -- and counting that would hold the level at nought
        // for ever, which reads as "always restocking, never stocked".
        let level = needs
            .iter()
            .filter(|n| n.buyable)
            .map(|n| logistics::line_level(n.have, n.keep))
            .fold(1.0f32, f32::min);
        let policy = self.autoplay.config.team.restock.sane();
        Supplies {
            name: self
                .world
                .player()
                .map(|p| p.name.clone())
                .unwrap_or_default(),
            level,
            pack_full: self.pack_full(),
            laden: self.laden(cfg),
            // Ready to go back: stocked up, and not still mid-errand.
            stocked: level >= policy.full_at && self.autoplay.growth.run.is_none(),
            handed_over: self.autoplay.growth.handed_over,
            holding_orders: self.holding_orders(),
            bill: needs.iter().map(|n| self.rough_cost(n)).sum(),
            purse: self.spendable(),
            // Short of something the trip cannot supply: either nothing
            // left to pay with, or a trip that came back with nothing,
            // which says the same thing about this town. Either way the
            // character is done shopping and goes back to earning.
            broke: !needs.is_empty()
                && (self.spendable() == 0 || self.autoplay.growth.run_was_futile),
            // What it can be handed: a gift is created in the pack by
            // the server, which spills into the side packs.
            free_space: self.room_anywhere(),
            sale,
            order: needs
                .iter()
                .filter(|n| n.want > 0 && n.buyable)
                .map(|n| (n.name.clone(), n.want))
                .collect(),
        }
    }

    /// Roughly what filling a need will cost, for working out who has
    /// to be handed money before the party can shop. A vendor sells
    /// above an item's own value, so this is an underestimate rather
    /// than a promise; it is only ever compared against a purse.
    fn rough_cost(&self, need: &Need) -> u32 {
        let unit = self
            .world
            .inventory()
            .find(|o| o.name.eq_ignore_ascii_case(&need.name) && o.value > 0)
            .map(|o| o.value)
            .unwrap_or(0);
        unit.saturating_mul(need.want)
    }

    /// Whether this character is carrying something another character
    /// asked for. On a quartermaster run that is how the party knows
    /// the runner still has goods to hand out; on any other it is
    /// simply false, since nobody has asked it for anything.
    ///
    /// Worked out from the others' orders alone, never from who the
    /// runner is: the runner is chosen from these reports, so asking
    /// would be circular.
    fn holding_orders(&self) -> bool {
        let me = self.world.stats.name.as_str();
        let wanted: Vec<&str> = self
            .autoplay
            .team
            .mates
            .iter()
            .filter(|m| m.name != me)
            .flat_map(|m| m.supplies.order.iter())
            .filter(|(_, count)| *count > 0)
            .map(|(name, _)| name.as_str())
            .collect();
        if wanted.is_empty() {
            return false;
        }
        self.world
            .inventory()
            .any(|o| wanted.iter().any(|w| o.name.eq_ignore_ascii_case(w)))
    }

    /// Every spell autoplay casts, in one list: what the restock list
    /// stocks components for, and what the counter's component guard
    /// keeps.
    ///
    /// All of them, not only the buffs and the bolts. The list once
    /// held those two alone, and a caster's heal, its vulnerabilities
    /// and imperils, the team's debuffs, its recalls and the casts
    /// that keep its stamina and mana up were stocked for nothing: each
    /// carries its own herb, powder, potion and talisman that no buff
    /// or bolt shares (Heal Self V burns 7, 26, 41 and 61; Flame Bolt
    /// I burns 15, 34, 46 and 55), so when the pack ran out the heal
    /// stopped casting and nobody went to town for it.
    ///
    /// Known, not castable. Whether a spell can be cast this moment is
    /// mostly a question of whether its components are in the pack,
    /// and that is the question this list is asked in order to answer.
    /// A buff is asked for with that check left out for the same
    /// reason (`wanted_buffs_if`).
    pub(crate) fn spells_cast(&self) -> Vec<u32> {
        use ac_world::vitals::{boosts_of, transfers_between, vital};
        let known = |id: &u32| self.world.stats.spells.contains(id);
        let table = self.assets.spell_table().ok();
        let at_another = |id: &u32| {
            table
                .as_ref()
                .and_then(|t| t.get(*id))
                .is_some_and(|s| s.needs_target())
        };
        let by_name = |names: &[String]| -> Vec<u32> {
            names.iter().filter_map(|n| self.spell_by_name(n)).collect()
        };
        let cfg = &self.autoplay.config;
        let mut out: Vec<u32> = Vec::new();
        // The buffs it should be wearing, castable or not, and the
        // ones the player named.
        let wearable = |id: u32| {
            !matches!(
                self.can_cast(id),
                crate::magic::CastCheck::NotKnown | crate::magic::CastCheck::TooHard { .. }
            )
        };
        out.extend(self.wanted_buffs_if(wearable).iter().map(|w| w.spell));
        out.extend(by_name(&cfg.buffs.spells));
        // The attack: every bolt in the book, or the ones named.
        if cfg.fight.spells.is_empty() {
            out.extend(self.attack_spells_known());
        } else {
            out.extend(by_name(&cfg.fight.spells));
        }
        // Softening: a vulnerability for each element, and the
        // imperils (see `autoplay_make_vulnerable`, `autoplay_soften`).
        for element in ac_world::elements::ALL {
            out.extend(
                crate::weapons::vulnerability_spells(element)
                    .into_iter()
                    .filter(known)
                    .filter(at_another),
            );
        }
        out.extend(self.imperil_spells());
        // The team's work: its debuffs by name, and the healer's heal.
        out.extend(by_name(&cfg.team.debuffs));
        if cfg.team.role == crate::autoplay::Role::Healer {
            out.extend(self.spell_by_name("Heal Other"));
        }
        // Its own heals (see `choose_heal`), and the casts that keep
        // stamina and mana up (see `autoplay_vitals`).
        out.extend(
            boosts_of(vital::HEALTH)
                .iter()
                .map(|b| b.spell)
                .filter(known),
        );
        for from in [vital::STAMINA, vital::MANA] {
            out.extend(
                transfers_between(from, vital::HEALTH)
                    .iter()
                    .map(|t| t.spell)
                    .filter(known),
            );
        }
        if cfg.survive.manage_mana {
            out.extend(
                boosts_of(vital::STAMINA)
                    .iter()
                    .map(|b| b.spell)
                    .filter(known),
            );
            out.extend(
                transfers_between(vital::STAMINA, vital::MANA)
                    .iter()
                    .map(|t| t.spell)
                    .filter(known),
            );
        }
        // The recalls a journey is planned over (see `castable_recalls`).
        out.extend(
            self.world
                .stats
                .spells
                .iter()
                .copied()
                .filter(|id| ac_world::recalls::is_recall(*id)),
        );
        out.sort_unstable();
        out.dedup();
        out
    }

    /// How many of each spell component to carry, keyed by component id.
    ///
    /// A taper is the yardstick: it is what a caster runs out of, and
    /// it is the one number a player sets. Everything else is scaled to
    /// it by how fast it burns relative to a taper, which the client
    /// can work out rather than guess, because both halves are in its
    /// own tables. ACE rolls each component of a formula separately
    /// (`Spell.TryBurnComponents`):
    ///
    /// ```text
    /// burn = spell.ComponentLoss * component.CDM * min(1, power / skill)
    /// ```
    ///
    /// The skill term cancels when one component is divided by another
    /// of the same spell, so the ratio is just the loss and the CDMs,
    /// and a component used by several spells is stocked for the one
    /// that burns it fastest. With foci a top-level cast burns about
    /// 0.4 of a taper against 0.003 of a scarab, so a thousand tapers
    /// comes out at a handful of scarabs rather than a thousand.
    pub fn component_targets(&self, tapers_keep: u32) -> BTreeMap<u32, u32> {
        let mut out: BTreeMap<u32, u32> = BTreeMap::new();
        if tapers_keep == 0 {
            return out;
        }
        let (Ok(table), Ok(comps)) = (self.assets.spell_table(), self.assets.spell_components())
        else {
            return out;
        };
        // The fastest rate each component burns at, across the spells
        // this character actually casts.
        let mut fastest: BTreeMap<u32, f32> = BTreeMap::new();
        for spell in self.spells_cast() {
            let Some(sp) = table.get(spell) else { continue };
            for id in self.current_formula(spell) {
                let Some(c) = comps.get(id) else { continue };
                let rate = sp.component_loss * c.cdm;
                let e = fastest.entry(id).or_insert(0.0);
                if rate > *e {
                    *e = rate;
                }
            }
        }
        let taper = fastest
            .get(&crate::magic::PRISMATIC_TAPER)
            .copied()
            .filter(|r| *r > 0.0);
        // A floor under everything: twenty to the thousand. The burn
        // rates say a scarab would only need eight, and that is cutting
        // it far too fine for something bought once a trip -- being
        // over-provisioned on the cheap, light things costs a slot and
        // saves a walk. Anything that genuinely burns faster than the
        // floor keeps its own number, so a caster without foci still
        // stocks its herbs properly.
        let floor = (tapers_keep / 50).max(1);
        for (id, rate) in &fastest {
            let want = match taper {
                // Scaled to the taper by how fast it burns.
                Some(t) => ((tapers_keep as f32) * rate / t).round() as u32,
                // No taper in any formula (no foci, or an odd build):
                // the configured number stands for everything.
                None => tapers_keep,
            };
            out.insert(*id, want.max(floor));
        }
        out
    }

    /// The party as everyone has last described itself, this character
    /// included. What every shared decision is worked out from.
    pub fn party_supplies(&self, cfg: &Growth, now: Instant) -> Vec<Supplies> {
        let mut party: Vec<Supplies> = self
            .autoplay
            .team
            .mates
            .iter()
            .map(|m| m.supplies.clone())
            .collect();
        party.push(self.supplies(cfg, now));
        party
    }

    /// Who is doing the party's shopping, when it sends one character
    /// rather than all going.
    pub fn quartermaster_name(&self, cfg: &Growth, now: Instant) -> Option<String> {
        if self.autoplay.config.team.restock.plan != crate::logistics::Plan::Quartermaster {
            return None;
        }
        crate::logistics::quartermaster(&self.party_supplies(cfg, now)).map(|m| m.name.clone())
    }

    /// Whether this character is the one doing the shopping.
    pub fn is_quartermaster(&self, cfg: &Growth, now: Instant) -> bool {
        let me = self.world.stats.name.as_str();
        !me.is_empty() && self.quartermaster_name(cfg, now).as_deref() == Some(me)
    }

    /// Everything the character is carrying that the rules would sell,
    /// whether or not a vendor is open. What the party hands its
    /// quartermaster before it leaves.
    pub fn loot_for_sale(&self, cfg: &Growth) -> Vec<u32> {
        // The same judgement a counter is handed. This was a third one:
        // it asked `sellable`, which knows nothing of the buy list and
        // has no rule for a character with no profile -- so the
        // quartermaster was handed exactly the two things the rest of
        // this was written to hold back, and sold them on the party's
        // behalf.
        let policy = self.sell_policy(cfg);
        self.world
            .inventory()
            .filter_map(|o| {
                let stats = self.stats_of(o.guid)?;
                let ammo = o.valid_locations & equip::MISSILE_AMMO != 0;
                self.offers_for_sale(&policy, &stats, ammo)
                    .then_some(o.guid)
            })
            .collect()
    }

    /// Every name that is never sold: the player's own list, whatever
    /// is kept stocked, and whatever the loot rules always keep.
    pub(crate) fn keep_names(&self) -> Vec<String> {
        // The player's own word, and nothing else: what the character
        // keeps stocked is barred from sale by the buy list itself
        // (`Profile::stocks`, passed separately), which used to be said
        // here a second time.
        self.loot_profile()
            .map(|p| p.looting.always.clone())
            .unwrap_or_default()
    }

    /// The pack items to sell to the open vendor, which takes only some
    /// kinds of thing. Weapons are only sold once appraised and found
    /// beyond the character.
    fn sale_list(&self, cfg: &Growth) -> Vec<u32> {
        let Some(v) = self.world.open_vendor.as_ref() else {
            return Vec::new();
        };
        // The vendor buys some kinds of thing, within a range of values
        // (a range of 0 is no range at all).
        self.salables(cfg)
            .into_iter()
            .filter(|it| it.taken_by(v.item_types, v.min_value, v.max_value))
            .map(|it| it.guid)
            .collect()
    }

    /// Everything the sale decision needs, worked out once for a whole
    /// pack rather than per item.
    pub(crate) fn sell_policy(&self, cfg: &Growth) -> SellPolicy {
        SellPolicy {
            burns: self.burns(cfg),
            keep: self.keep_names(),
            profile: self.profiles.get(&self.autoplay.config.loot.profile),
            wielder: self.wielder(),
            me: self.world.stats.name.clone(),
        }
    }

    /// Whether this carried thing goes over a counter.
    ///
    /// One answer, asked both by the forecast before the character sets
    /// off and by the snapshot the counter in front of it is handed.
    /// They were two, and they disagreed: the snapshot's had no
    /// fallback for a character with no profile, so the forecast said
    /// the trip was worth making and then nothing was offered.
    pub(crate) fn offers_for_sale(&self, p: &SellPolicy, s: &ItemStats, ammo: bool) -> bool {
        offer_to_vendor(
            s,
            ammo,
            &p.burns,
            &p.keep,
            p.profile.as_ref().is_some_and(|x| x.stocks(&s.name)),
            self.autoplay.ledger.of(s),
            // The profile is the source of truth. A character nobody
            // has given one to sells nothing, which is the right way for
            // this to be empty-handed: the alternative is a client
            // deciding on its own what somebody's things are worth.
            || match &p.profile {
                Some(pr) => matches!(
                    pr.judge(s, self.appraisals.get(&s.guid), &p.wielder, &p.me, 0),
                    crate::profile::Verdict::Decided(LootAction::Sell, _)
                ),
                None => false,
            },
        )
    }

    /// The counter the loot profile names for selling, if it names
    /// one. The policy is [`Profile::sell_to_named`]; this is the
    /// lookup.
    fn sell_to_named(&self) -> Option<String> {
        let loot = &self.autoplay.config.loot;
        let p = self.profiles.get(&loot.profile)?;
        p.sell_to_named().map(str::to_string)
    }

    /// Everything in the pack the selling rules allow to go, before any
    /// one counter's tastes are applied to it.
    ///
    /// [`Self::sale_list`] narrows this to the vendor standing in front
    /// of the character; a forecast narrows it to a shop the character
    /// has not walked to yet, which is how a trip is judged before it is
    /// started.
    fn salables(&self, cfg: &Growth) -> Vec<Salable> {
        self.for_sale(cfg)
            .into_iter()
            .map(|o| Salable {
                guid: o.guid,
                item_type: o.item_type,
                value: o.value,
                stack: o.stack_size.max(1),
            })
            .collect()
    }

    /// The carried things a counter would be offered, as they lie in the
    /// pack: what the selling rules let go, less what no counter takes.
    fn for_sale(&self, cfg: &Growth) -> Vec<&ac_world::WorldObject> {
        // The same judgement the counter is handed (see
        // [`Client::offers_for_sale`]). It used to be a second one, and
        // the two disagreed.
        let policy = self.sell_policy(cfg);
        // A pack with things in it is not loot to be sold; the server
        // refuses it, and it holds the character's belongings. Which
        // packs hold anything is worked out once, not once per item:
        // this is asked on every turn spent over a corpse (see
        // [`Self::loot_burden`]).
        let holders: std::collections::BTreeSet<u32> = self
            .world
            .objects
            .values()
            .filter_map(|o| o.container)
            .collect();
        self.world
            .inventory()
            .filter(|o| {
                let Some(stats) = self.stats_of(o.guid) else {
                    return false;
                };
                let ammo = o.valid_locations & equip::MISSILE_AMMO != 0;
                !never_sell_carried(&stats, holders.contains(&o.guid))
                    && self.offers_for_sale(&policy, &stats, ammo)
            })
            .collect()
    }

    /// What the loot being carried weighs: everything a counter would be
    /// offered. A stack's burden is the whole stack's already, and a
    /// pack with anything in it is never offered, so nothing is counted
    /// twice.
    fn loot_burden(&self, cfg: &Growth) -> u32 {
        self.for_sale(cfg)
            .iter()
            .fold(0u32, |sum, o| sum.saturating_add(o.burden))
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

    /// What the character is short of, worked out afresh at most once
    /// every [`NEEDS_EVERY`]. Counting the pack is not free and the
    /// answer does not change between frames.
    fn needs_now(&mut self, now: Instant, cfg: &Growth) -> Vec<Need> {
        if let Some(n) = self
            .autoplay
            .growth
            .needs_seen
            .as_ref()
            .filter(|(t, _)| now.duration_since(*t) < NEEDS_EVERY)
            .map(|(_, n)| n.clone())
        {
            return n;
        }
        let n = self.grow_needs(cfg);
        self.autoplay.growth.needs_seen = Some((now, n.clone()));
        n
    }

    /// The spell components this character's spells burn, by weenie
    /// class: what it goes to town to buy, as against what it goes to
    /// town to sell.
    pub fn burns(&self, _cfg: &Growth) -> Vec<u32> {
        let Ok(mapper) = self.assets.spell_component_ids() else {
            return Vec::new();
        };
        // Which components, not how many: this is the guard that stops
        // a caster selling what its own spells burn, and it must not be
        // switchable off by editing a shopping list. Any positive scale
        // gives the same set of keys, so it is given one of its own
        // rather than the number of tapers the player happens to keep.
        //
        // A guard, not a verdict. It answers for a component the
        // profile said nothing about; one a loot rule tagged to sell
        // is the player's word, which is not a shopping list, and goes
        // (`ac_loot::sale::offer_to_vendor` reads the tag first).
        const ENOUGH_TO_NAME_THEM: u32 = 1_000;
        self.component_targets(ENOUGH_TO_NAME_THEM)
            .keys()
            .filter_map(|id| mapper.component_wcid(*id))
            .collect()
    }

    /// Read the server's answer to the pour in the air, if there is one.
    ///
    /// `None` means there is nothing to wait for: no pour was sent, or
    /// the one sent is over. `Some(Waiting)` means it is still out
    /// there and the counts a new choice would be made from are stale.
    /// Anything else is what the server said about it.
    pub(crate) fn settle_pour(&mut self, now: Instant) -> Option<crate::did::Did> {
        use crate::did::{Because, Did};
        let (sent, at) = self.autoplay.pour.clone()?;
        let (from, to) = (sent.merge.from, sent.merge.to);
        let target_now = self.world.objects.get(&to).map(|o| o.stack_size.max(1));
        let refusal = crate::pack::refusal_of(
            at,
            self.move_refused.get(&from).copied(),
            self.move_refused.get(&to).copied(),
        );
        let waited = now.saturating_duration_since(at);
        match crate::pack::pour_answer(&sent, target_now, refusal, waited) {
            crate::pack::PourAnswer::InAir => {
                Some(Did::waiting("the last pour has not landed yet"))
            }
            crate::pack::PourAnswer::Landed => {
                // What the surviving stack was taken for is settled
                // here rather than when the pour was sent: a refused
                // pour used to raise the target's tag anyway, so a
                // stack meant for a counter quietly became one to keep
                // and was never sold.
                self.autoplay.ledger.merged(from, to);
                self.autoplay
                    .growth
                    .wont_merge
                    .note((from, to), &Did::Done, now);
                self.autoplay.pour = None;
                tracing::debug!(
                    "pour landed: {} {} into {to:#010x}",
                    sent.merge.amount,
                    sent.merge.name
                );
                None
            }
            crate::pack::PourAnswer::Refused(code) => {
                // Spent: the refusal was about this pour, and leaving
                // it behind would answer the next one too.
                for g in [from, to] {
                    if self
                        .move_refused
                        .get(&g)
                        .is_some_and(|(_, when)| *when >= at)
                    {
                        self.move_refused.remove(&g);
                    }
                }
                let did = Did::Blocked(match code {
                    0 => Because::ours("the server would not put those two together"),
                    code => Because::server(code),
                });
                self.autoplay.growth.wont_merge.note((from, to), &did, now);
                self.autoplay.pour = None;
                Some(did)
            }
            crate::pack::PourAnswer::Lost => {
                // No word at all. Not a refusal -- the pair is left a
                // moment and offered again -- but the pack has to be
                // read afresh before anything else is asked for.
                self.autoplay.growth.wont_merge.note(
                    (from, to),
                    &Did::waiting("no word on the pour"),
                    now,
                );
                self.autoplay.pour = None;
                None
            }
        }
    }

    /// Choose the next pour, send it, and remember it until the server
    /// answers ([`Unpoured`] says why one was not sent).
    ///
    /// One at a time, because the server answers in its own time and
    /// the counts a second choice would be made from are the ones from
    /// before the first. What a pour must not touch is settled here: a
    /// pair the server has turned down, a stack another errand is
    /// holding, and weight the server will not hand the character.
    pub(crate) fn pour_next(&mut self, now: Instant) -> Result<crate::pack::Merge, Unpoured> {
        use crate::did::Did;
        // Nothing is chosen over counts the server has not settled.
        if let Some(did) = self.settle_pour(now) {
            return Err(match did {
                Did::Waiting(_) => Unpoured::Wait(did),
                did => Unpoured::Turned(did),
            });
        }
        let may_carry = self.burden_room();
        let stacks = self.pack_stacks();
        let errands = self.autoplay.held_by_an_errand();
        // A pair the server has turned down waits its turn out, and a
        // stack another errand is holding is left where it is. Without
        // the first, one stubborn pair was the only answer ever given
        // and nothing else in the pack was ever poured.
        //
        // And two stacks whose words disagree are never poured
        // together. A pour makes one stack of two, and one stack can
        // carry one word: a stack the player said to sell poured into
        // one they said to keep was settled as kept (the ledger takes
        // the cautious answer), and poured into one nothing was
        // decided about it was settled as nothing -- either way the
        // Sell was gone before the character set off for town. Kept
        // apart, each stack goes where its own word says.
        let ledger = &self.autoplay.ledger;
        let skip = |from: u32, to: u32| {
            self.autoplay.growth.wont_merge.held(&(from, to), now)
                || errands.iter().flatten().any(|g| *g == from || *g == to)
                || ledger.by_guid(from) != ledger.by_guid(to)
        };
        let Some(m) = crate::pack::next_merge_unless(&stacks, may_carry, skip) else {
            // Nothing to pour -- or nothing light enough. The two are
            // worth telling apart: one is a tidy pack, the other is a
            // character that must sell something first. Asked with the
            // same skips, so a pack whose only pours are held is not
            // called too laden.
            return Err(
                if crate::pack::next_merge_unless(&stacks, u32::MAX, skip).is_some() {
                    Unpoured::Laden
                } else {
                    Unpoured::Tight
                },
            );
        };
        if self
            .autoplay
            .last_merge
            .is_some_and(|t| now.duration_since(t) < crate::autoplay::MERGE_EVERY)
        {
            return Err(Unpoured::Wait(Did::waiting(
                "the last pour has not landed yet",
            )));
        }
        let to_before = self
            .world
            .objects
            .get(&m.to)
            .map(|o| o.stack_size.max(1))
            .unwrap_or(0);
        self.autoplay.last_merge = Some(now);
        if !self.send_merge(m.from, m.to, Some(m.amount)) {
            // Our own rules turned it down: not both carried, not the
            // same weenie, or the target does not stack at all.
            let did = Did::refused("those two will never join");
            self.autoplay
                .growth
                .wont_merge
                .note((m.from, m.to), &did, now);
            return Err(Unpoured::Turned(did));
        }
        self.autoplay.pour = Some((
            crate::pack::PourSent {
                merge: m.clone(),
                to_before,
            },
            now,
        ));
        Ok(m)
    }

    /// Pour one loose stack into another, and say what came of it.
    ///
    /// `Did::Done` means the pack is as tight as it goes. `Blocked`
    /// means nothing can be poured until the character is lighter:
    /// the server weighs a pour as though the source were being picked
    /// up for the first time, so a character near its ceiling is
    /// refused a move that changes its burden by nothing at all. The
    /// refusal carries no message and no code, which is how a tidier
    /// that could not read it spent whole afternoons asking.
    ///
    /// This is the town run's way in, where tidying is the step and is
    /// worth a status line of its own. Everywhere else the pack is
    /// tidied as housekeeping (`Client::autoplay_tidy`).
    pub(crate) fn compress(&mut self, now: Instant) -> crate::did::Did {
        use crate::did::{Because, Did};
        match self.pour_next(now) {
            Ok(m) => {
                self.autoplay.say(
                    Doing::Tidying,
                    format!("putting {} {} with the rest", m.amount, m.name),
                );
                Did::Acting
            }
            Err(Unpoured::Tight) => Did::Done,
            Err(Unpoured::Laden) => Did::Blocked(Because::ours("too laden to put stacks together")),
            Err(Unpoured::Wait(did) | Unpoured::Turned(did)) => did,
        }
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

    /// What the character is short of, as the shopping rules want it:
    /// a weenie class, a name and how many more to buy.
    ///
    /// Only lines that can actually be bought. What no counter stocks
    /// is farmed instead, and a want nobody can fill would hold a trip
    /// open for ever.
    #[cfg(test)]
    pub(crate) fn vendor_shortfall(&self, cfg: &Growth) -> Vec<ac_vendor::counter::Want> {
        self.vendor_shortfall_with(cfg, &self.item_stats())
    }

    /// The same, over a pack already read (see [`Client::grow_needs_with`]).
    pub(crate) fn vendor_shortfall_with(
        &self,
        cfg: &Growth,
        stats: &[ItemStats],
    ) -> Vec<ac_vendor::counter::Want> {
        self.grow_needs_with(cfg, stats)
            .into_iter()
            .filter(|n| n.buyable && n.want > 0)
            .filter_map(|n| {
                let wcid = match n.kind {
                    NeedKind::Component(wcid) => wcid,
                    // Ammunition and named stock are matched on the
                    // shelf by name rather than by class, so they are
                    // left to the older path for now.
                    _ => return None,
                };
                Some(ac_vendor::counter::Want {
                    wcid,
                    name: n.name,
                    short: n.want,
                    urgent: n.urgent,
                })
            })
            .collect()
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

    /// What the character is carrying, in burden units, and the most
    /// it may carry.
    ///
    /// The capacity is a hundred and fifty times Strength, plus thirty
    /// more per rank of the carrying-capacity augmentation; the server
    /// refuses to hand over anything that would take the character past
    /// three times that. The comfortable place to work is well under
    /// it: a character at twice its capacity is slow and has no Melee or
    /// Missile Defense left, and a character at three times it cannot
    /// pick up what it kills.
    pub fn burden(&self) -> (u32, u32) {
        const ENCUMBRANCE_VAL: u32 = 5;
        const CARRY_AUGMENTATION: u32 = 230;
        let int_of = |k: u32| {
            self.world
                .stats
                .ints
                .iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| (*v).max(0) as u32)
        };
        // What the server says we are carrying, or the pack counted up
        // when it has not said.
        //
        // Counted the way the server counts it, which is not the
        // obvious way. A stack's weight is the whole stack's already,
        // so multiplying by its size counts it twice over; and a side
        // pack's weight is its own plus everything inside it, so adding
        // the contents as well counts those twice too. Only what hangs
        // directly off the character is added: the main pack's items,
        // side packs included as the single lumps they weigh, and
        // whatever is being worn or held.
        let now = int_of(ENCUMBRANCE_VAL).unwrap_or_else(|| {
            self.world
                .main_pack()
                .chain(self.world.wielded())
                .map(|o| o.burden)
                .sum()
        });
        let strength = self.wielder().attributes_current[0];
        let augmented = 150 + 30 * int_of(CARRY_AUGMENTATION).unwrap_or(0);
        (now, strength.saturating_mul(augmented))
    }

    /// How much more the character may be handed before the server
    /// starts refusing: three times its capacity, less what it carries.
    pub fn burden_room(&self) -> u32 {
        let (now, capacity) = self.burden();
        capacity.saturating_mul(3).saturating_sub(now)
    }

    /// How much more loot the character will take on before it stops
    /// hunting and goes to sell. Zero means it has had enough.
    ///
    /// The loot profile's `carry_up_to` is measured on the loot alone:
    /// what the selling rules would hand a counter. What the character
    /// wears and wields, and what the rules keep -- foci, the components
    /// its spells burn, what it keeps stocked, what it took to keep or
    /// to salvage -- is left out, because no trip to town takes it off.
    /// Counted, a character in heavy plate had no room before it picked
    /// up a thing, and left every corpse untouched. Everything still
    /// counts towards twice its capacity, where it has no defense left,
    /// and towards the server's wall at three times (`loot_room` has the
    /// arithmetic and the whole story).
    ///
    /// This is the working limit, not the server's. [`burden_room`] is
    /// the wall -- what the server will still accept -- and a character
    /// that hunts up to the wall cannot loot, cannot merge stacks and
    /// can barely walk.
    pub fn carry_room(&self, cfg: &Growth) -> u32 {
        let (now, capacity) = self.burden();
        let up_to = self
            .loot_profile()
            .map_or(crate::profile::Looting::default().carry_up_to, |p| {
                p.looting.carry_up_to
            });
        loot_room(now, self.loot_burden(cfg), capacity, up_to)
    }

    /// Carrying as much loot as it means to: time to go and sell.
    ///
    /// Not before its strength is known. With no capacity there is no
    /// room either, and a party told a member was laden the moment it
    /// logged in would turn round for town before a fight.
    ///
    /// Having had enough is not only having no room at all: it is having
    /// less room than the lightest thing the looting last left on a body
    /// still lying about (see [`had_enough`]).
    pub fn laden(&self, cfg: &Growth) -> bool {
        let (carried, capacity) = self.burden();
        if capacity == 0 {
            return false;
        }
        let up_to = self
            .loot_profile()
            .map_or(crate::profile::Looting::default().carry_up_to, |p| {
                p.looting.carry_up_to
            });
        let loot = self.loot_burden(cfg).min(carried);
        let room = loot_room(carried, loot, capacity, up_to);
        // Sold down to what it keeps: the room a trip to town would give.
        let sold = loot_room(carried - loot, 0, capacity, up_to);
        let objects = &self.world.objects;
        let left = self
            .autoplay
            .lightest_left_for_weight(|g| objects.contains_key(&g));
        had_enough(room, left, sold)
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
mod tests {
    use super::*;

    fn offer(raise: Raise, cost: u32, weight: f32) -> Offer {
        Offer {
            raise,
            cost,
            weight,
        }
    }

    #[test]
    fn blargerton_has_room_to_loot_with_everything_he_keeps_on_him() {
        // Strength 60: capacity 9000, a limit of 13500 at 1.5, twice his
        // capacity at 18000 and the wall at 27000. He carried 13866 and
        // had taken nothing, so all of it was his own: plate 6540, four
        // foci 1600, a stack of tapers 1374, then weapons and the rest.
        let (carried, capacity) = (13_866, 9_000);
        // Counting all of it as loot left him nothing: the fault.
        assert_eq!(loot_room(carried, carried, capacity, 1.5), 0);
        // None of it is loot. He takes on loot until he reaches twice
        // his capacity, which comes before the limit does.
        assert_eq!(loot_room(carried, 0, capacity, 1.5), 18_000 - 13_866);
    }

    #[test]
    fn kept_things_in_the_pack_do_not_fill_a_low_limit() {
        // Leaving out only what he wore still counted his foci, tapers
        // and weapons as loot: 7326 of it, over a limit of 7200 at 0.8,
        // and no room again.
        let (carried, worn, capacity) = (13_866, 6_540, 9_000);
        assert_eq!(loot_room(carried, carried - worn, capacity, 0.8), 0);
        // Measured on what a counter would take, there is room at every
        // setting the slider allows.
        for up_to in [0.5, 0.8, 1.0, 1.5, 2.0, 2.9] {
            assert!(loot_room(carried, 0, capacity, up_to) > 0, "at {up_to}");
        }
        // A lighter character at a low setting is stopped by the limit
        // itself: 4500 of loot at 0.5, whatever it keeps.
        assert_eq!(loot_room(3_000, 0, 9_000, 0.5), 4_500);
        assert_eq!(loot_room(3_000 + 4_500, 4_500, 9_000, 0.5), 0);
    }

    #[test]
    fn loot_does_not_take_a_character_in_plate_past_twice_its_capacity() {
        // Plate 6540 and 11460 of loot is 18000, twice his capacity, and
        // no Melee or Missile Defense left. The limit alone would have
        // let him take 2040 more and fight on to 20040.
        assert_eq!(loot_room(18_000, 11_460, 9_000, 1.5), 0);
        // A thousand short of it, a thousand is all the room there is.
        assert_eq!(loot_room(17_000, 10_460, 9_000, 1.5), 1_000);
        // A player who asks for more than twice gets it: at 2.5 the line
        // is the limit's own, 22500.
        assert_eq!(loot_room(6_540, 0, 9_000, 2.5), 22_500 - 6_540);
        assert_eq!(loot_room(22_500, 15_960, 9_000, 2.5), 0);
    }

    #[test]
    fn what_is_kept_past_twice_capacity_leaves_the_limit_and_the_wall() {
        // What it keeps is at twice its capacity on its own: loot cannot
        // slow it further and no sale gets it back under the line, so
        // the line gives way rather than leave every corpse untouched.
        assert_eq!(loot_room(18_000, 0, 9_000, 1.5), 9_000);
        assert_eq!(loot_room(20_000, 0, 9_000, 1.5), 7_000);
        assert_eq!(loot_room(20_000 + 5_000, 5_000, 9_000, 1.5), 2_000);
        // At a low setting the limit still stops it first.
        assert_eq!(loot_room(18_000, 0, 9_000, 0.5), 4_500);
        assert_eq!(loot_room(18_000 + 4_500, 4_500, 9_000, 0.5), 0);
    }

    #[test]
    fn the_servers_wall_still_caps_the_room() {
        // A limit set by hand past three times: the limit and the line
        // both allow more than the server will hand over.
        assert_eq!(loot_room(20_000, 8_000, 9_000, 3.5), 27_000 - 20_000);
        // At the wall there is no room, however light the loot.
        assert_eq!(loot_room(27_000, 7_000, 9_000, 1.5), 0);
        assert_eq!(loot_room(30_000, 0, 9_000, 1.5), 0);
    }

    #[test]
    fn the_room_never_passes_a_line_and_a_sale_always_makes_some() {
        let capacity = 9_000u32;
        let wall = 27_000u32;
        for up_to in [0.5f32, 0.8, 1.0, 1.5, 2.0, 2.5, 2.9] {
            let limit = (capacity as f32 * up_to) as u32;
            let line = (capacity as f32 * up_to.max(2.0)) as u32;
            for kept in (0..=30_000u32).step_by(250) {
                for loot in (0..=30_000u32).step_by(250) {
                    let carried = kept + loot;
                    let room = loot_room(carried, loot, capacity, up_to);
                    let at = format!("up to {up_to}, kept {kept}, loot {loot}: room {room}");
                    if room == 0 {
                        continue;
                    }
                    // Never past the limit on loot, nor the wall on
                    // everything.
                    assert!(loot + room <= limit, "limit: {at}");
                    assert!(carried + room <= wall, "wall: {at}");
                    // Never past twice capacity while what it keeps is
                    // under it.
                    if kept < line {
                        assert!(carried + room <= line, "line: {at}");
                    }
                }
                // Sold down to what it keeps, it has room again -- unless
                // that is already at the wall, which no sale can help.
                let sold = loot_room(kept, 0, capacity, up_to);
                assert_eq!(
                    sold == 0,
                    kept >= wall,
                    "sold down: up to {up_to}, kept {kept}"
                );
            }
        }
    }

    #[test]
    fn no_capacity_or_no_limit_is_no_room() {
        // Strength not heard yet: capacity 0, so nothing is claimed.
        assert_eq!(loot_room(0, 0, 0, 1.5), 0);
        assert_eq!(loot_room(500, 0, 0, 1.5), 0);
        // A limit of nothing, less, or not a number is nothing.
        assert_eq!(loot_room(0, 0, 9_000, 0.0), 0);
        assert_eq!(loot_room(0, 0, 9_000, -1.0), 0);
        assert_eq!(loot_room(0, 0, 9_000, f32::NAN), 0);
        // A count of loot ahead of the server's total: all of it is loot.
        assert_eq!(loot_room(1_000, 1_500, 9_000, 1.5), 13_500 - 1_000);
    }

    #[test]
    fn a_character_with_room_for_nothing_it_wants_goes_to_sell() {
        // Laden needed no room at all. The loot rules take only what fits,
        // so the room settled a little above nothing: forty short of a
        // mace, every body with one on it was shut as too laden, and the
        // character hunted on and never went to sell.
        let sold = 18_000 - 13_866;
        assert!(!had_enough(40, None, sold), "nothing left behind yet");
        assert!(had_enough(40, Some(300), sold), "forty short of a mace");
        // Room for it again -- tapers burnt, or a sale -- and it is not.
        assert!(!had_enough(300, Some(300), sold));
        assert!(!had_enough(sold, Some(300), sold));
        // No room at all is enough, as it always was.
        assert!(had_enough(0, None, sold));
        // A thing no sale could make room for is no reason to go: an anvil
        // heavier than the room left with every bit of loot sold.
        assert!(!had_enough(40, Some(9_000), sold));
    }

    #[test]
    fn a_character_with_nobody_to_restock_with_goes_to_town_on_its_own() {
        // Blargerton: team rules on, restocking together on, no party.
        // The party's mode decided his trips, a party of one never left
        // for weight, and he hunted on laden with every body left full.
        let team = crate::autoplay::Team {
            enabled: true,
            restock: crate::logistics::Restock {
                together: true,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(!restocks_as_a_party(&team, 0), "alone is alone");
        assert!(restocks_as_a_party(&team, 1));
        // Either setting off, it is alone whoever else is about.
        let apart = crate::autoplay::Team {
            restock: crate::logistics::Restock {
                together: false,
                ..Default::default()
            },
            ..team.clone()
        };
        assert!(!restocks_as_a_party(&apart, 3));
        let off = crate::autoplay::Team {
            enabled: false,
            ..team
        };
        assert!(!restocks_as_a_party(&off, 3));
    }

    /// Whether a run standing `away` metres from its counter is there.
    fn there(away: f32) -> bool {
        away <= VENDOR_REACH
    }

    #[test]
    fn a_walk_to_a_counter_broken_off_by_a_corpse_is_walked_on() {
        // +Verity, carrying as much as she meant to, set off for Shopkeeper
        // Renald the Elder 250 m away. A second later the looting walked
        // her to a fresh corpse, which ends a journey, and the run found no
        // journey under way 224 m short: "could not get to", sold nothing.
        assert_eq!(
            on_the_way(there(224.0), true, false, false),
            OnTheWay::WalkOn
        );
        // Not while what broke it off still has her: a fight on the way.
        assert_eq!(on_the_way(there(224.0), true, true, false), OnTheWay::Wait);
        // A journey that ended by itself short of the counter -- it gave
        // up, or arrived somewhere else -- still could not get there.
        assert_eq!(
            on_the_way(there(224.0), false, false, false),
            OnTheWay::Short
        );
        assert_eq!(
            on_the_way(there(224.0), false, true, false),
            OnTheWay::Short
        );
        // Near enough, she goes up to the counter however it ended.
        for (broken_off, busy) in [(false, false), (true, false), (true, true)] {
            assert_eq!(
                on_the_way(there(VENDOR_REACH), broken_off, busy, false),
                OnTheWay::There
            );
        }
    }

    #[test]
    fn a_walk_broken_off_again_as_soon_as_it_is_planned_waits_before_the_next_plan() {
        // Something that ends the walk to the counter the moment it is
        // planned -- a follower pulled back to its leader each time it
        // closed to the following distance -- had it planned again every
        // tick or two for the run's four minutes, a route search each time.
        assert_eq!(on_the_way(there(224.0), true, false, true), OnTheWay::Wait);
        // Once the moment is up it is planned again.
        assert_eq!(
            on_the_way(there(224.0), true, false, false),
            OnTheWay::WalkOn
        );
        // Neither giving up nor going up to the counter waits on it.
        assert_eq!(
            on_the_way(there(224.0), false, false, true),
            OnTheWay::Short
        );
        assert_eq!(
            on_the_way(there(VENDOR_REACH), true, false, true),
            OnTheWay::There
        );
    }

    /// Offline session over the real archives: nothing calls `tick`, so
    /// no packet is ever sent.
    fn offline_client(assets: std::rc::Rc<ac_scene::Assets>) -> Client {
        Client::connect(
            crate::Config {
                host: "127.0.0.1:1".into(),
                account: "acreborn".into(),
                password: "x".into(),
                character: None,
                auto_enter: true,
            },
            assets,
        )
        .unwrap()
    }

    /// A character standing in `cell` at `local`, offline, when the
    /// archives are there to be read.
    fn standing_at(cell: u32, local: glam::Vec3) -> Option<Client> {
        let Some(dir) = std::env::var_os("AC_DATA_DIR") else {
            eprintln!("AC_DATA_DIR unset; skipping");
            return None;
        };
        let assets = std::rc::Rc::new(ac_scene::Assets::open(dir).unwrap());
        let mut c = offline_client(assets.clone());
        let mut pl = crate::player::Player::new(&assets, cell, local, glam::Quat::IDENTITY);
        pl.set_motion_table(&assets, 0x0200_0001, 0x0900_0001);
        c.player = Some(pl);
        Some(c)
    }

    /// A run on its way to a counter at `at`, set off at `now`.
    fn run_to(at: Vec2, now: Instant) -> Run {
        Run {
            vendor: "Shopkeeper Renald the Elder".into(),
            at,
            phase: Phase::Going,
            since: now,
            last_sell: None,
            town: at,
            stops: 1,
            sold: 0,
            reason: "carrying as much as it means to".into(),
            errand: Errand::Sell,
            visited: vec![at],
            walked_on: None,
        }
    }

    #[test]
    fn a_follower_on_its_own_town_run_is_not_pulled_back_to_its_leader() {
        // A party restocking with everyone going: each follower makes its
        // own run while the leader goes on leading. Following ranks above
        // the run, so each time the follower closed to its following
        // distance the walk after the leader ended the run's journey, the
        // run planned it again, and the walk after the leader ended it
        // again, for the run's four minutes.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        let me = c.player.as_ref().unwrap().world_position();
        let team = &mut c.autoplay.config.team;
        team.enabled = true;
        team.follow = true;
        team.lead = false;
        team.follow_distance = 4.0;
        let leader_off = |metres: f32| crate::autoplay::Mate {
            name: "Leader".into(),
            leader: true,
            leads: true,
            world: me + glam::Vec3::new(metres, 0.0, 0.0),
            cell: holtburg,
            ..Default::default()
        };
        let cfg = c.autoplay.config.growth.clone();
        let now = Instant::now();
        let counter = Vec2::new(me.x + 250.0, me.y);
        assert!(c.grow_travel(counter, now), "no way to the counter");
        c.autoplay.growth.run = Some(run_to(counter, now));

        // The leader walks off and stops, and walks off again: the
        // follower keeps to its own walk.
        for (tick, metres) in [12.0, 3.0, 12.0, 3.0, 30.0, 3.0].into_iter().enumerate() {
            c.autoplay.team.mates = vec![leader_off(metres)];
            assert!(!c.autoplay_follow(now, true), "caught up at tick {tick}");
            assert!(!c.autoplay_follow(now, false), "followed at tick {tick}");
            assert!(
                c.traveling(),
                "its walk to the counter ended at tick {tick}"
            );
            assert!(c.grow_run_step(now, &cfg).goes_on());
            assert!(c.traveling());
        }

        // A corpse on the way does break the walk off, and the run walks
        // on once it is dealt with. Broken off again straight away, the run
        // waits a moment before planning it once more.
        c.interrupt_travel("walking to a corpse");
        assert!(c.journey_broken_off());
        assert!(c.grow_run_step(now, &cfg).goes_on());
        assert!(c.traveling(), "the run did not walk on after a corpse");
        c.interrupt_travel("walking to a corpse");
        let soon = now + Duration::from_secs(1);
        assert!(c.grow_run_step(soon, &cfg).goes_on());
        assert!(!c.traveling(), "planned again straight away");
        assert!(c.grow_run_step(now + WALK_ON_EVERY, &cfg).goes_on());
        assert!(c.traveling());

        // With no run of its own the leader is followed. The walk after it
        // ends the journey without breaking it off: nothing is to pick that
        // journey up again.
        c.autoplay.growth.run = None;
        c.autoplay.team.mates = vec![leader_off(12.0)];
        assert!(c.autoplay_follow(now, false));
        assert!(!c.traveling());
        assert!(!c.journey_broken_off());
    }

    #[test]
    fn a_town_run_between_journeys_is_not_taken_exploring_or_back_to_the_area() {
        // Underground, a corpse on the way ends the run's journey, and
        // exploring ranks above the run: a room chosen then kept the tick
        // for good, so the run never planned its walk again and its clock
        // was never read. Keeping to a hunting area ranks above the run as
        // well, and would have walked the character back to it.
        let renald = Vec2::new(32_587.2, 34_578.3);
        let now = Instant::now();

        // Where the Holtburg Dungeon's portal drops a character.
        let Some(mut c) = standing_at(0x01F6_0289, glam::Vec3::new(96.7, -10.0, 0.0)) else {
            return;
        };
        c.autoplay.growth.run = Some(run_to(renald, now));
        assert!(!c.traveling());
        assert!(!c.autoplay_explore(now), "went exploring on a run to town");
        assert!(c.follow.is_none());
        // With no run, the same dungeon is explored.
        c.autoplay.growth.run = None;
        assert!(c.autoplay_explore(now));

        // Outdoors by the Holtburg lifestone, a field to hunt 40 m off.
        let Some(mut c) = standing_at(0xA9B4_0019, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        let me = c.player.as_ref().unwrap().world_position();
        let (x, y) = (me.x + 40.0, me.y);
        let fight = &mut c.autoplay.config.fight;
        fight.enabled = true;
        fight.area = Some(crate::hunt::HuntArea {
            name: "the field".into(),
            shape: crate::hunt::Shape::Outline {
                points: vec![[x, y], [x + 30.0, y], [x + 30.0, y + 30.0], [x, y + 30.0]],
            },
        });
        c.autoplay.growth.run = Some(run_to(renald, now));
        assert!(
            !c.autoplay_keep_to_area(now),
            "walked back to the area on a run to town"
        );
        assert!(c.follow.is_none());
        // With no run, outside the field, it goes back to it.
        c.autoplay.growth.run = None;
        assert!(c.autoplay_keep_to_area(now));
    }

    /// A level 10 character playing on its own where the Holtburg
    /// Dungeon's portal drops it -- a dungeon, so exploring always has a
    /// room to walk to -- with `xp` to spend and nothing spent yet.
    fn with_experience_to_spend(xp: i64) -> Option<Client> {
        let mut c = standing_at(0x01F6_0289, glam::Vec3::new(96.7, -10.0, 0.0))?;
        c.world.player_guid = Some(0x5000_0001);
        c.world.stats.level = 10;
        c.world.stats.available_xp = xp;
        c.autoplay.config.enabled = true;
        Some(c)
    }

    #[test]
    fn experience_is_spent_while_exploring_claims_every_tick() {
        // On the local server a character granted a hundred billion
        // experience spent none of it in three minutes. Spending was part
        // of the last goal, and exploring claimed every tick before it.
        let Some(mut c) = with_experience_to_spend(1_000_000) else {
            return;
        };
        let start = Instant::now();
        let mut raised: Vec<Instant> = Vec::new();
        let mut t = start;
        while t < start + RAISE_EVERY * 5 {
            c.tick_autoplay(t);
            assert_eq!(
                c.autoplay.step,
                Some("explore"),
                "exploring did not claim the tick at {:?}",
                t - start
            );
            if let Some(at) = c.autoplay.growth.last_raise {
                if !raised.contains(&at) {
                    raised.push(at);
                    // The server takes it.
                    let (pick, batch) = sent(&c);
                    server_takes(&mut c, pick, batch);
                }
            }
            t += Duration::from_millis(100);
        }
        assert!(
            raised.first().is_some_and(|at| *at - start <= RAISE_EVERY),
            "nothing was raised within {RAISE_EVERY:?}"
        );
        // And it goes on, a message at a time.
        assert!(raised.len() >= 4, "only {} ranks", raised.len());
        assert!(
            raised.windows(2).all(|w| w[1] - w[0] >= RAISE_EVERY),
            "ranks closer together than {RAISE_EVERY:?}"
        );
    }

    #[test]
    fn nothing_is_sent_with_auto_xp_off_or_nothing_to_spend() {
        let Some(mut c) = with_experience_to_spend(1_000_000) else {
            return;
        };
        let start = Instant::now();
        let at = |s: u64| start + Duration::from_secs(s);

        c.autoplay.config.growth.auto_xp = false;
        for s in 0..30 {
            c.autoplay_spend_xp(at(s));
        }
        assert_eq!(c.session.actions_sent(), 0, "spent with auto_xp off");

        // On, with an empty pool, or one short of the cheapest rank.
        c.autoplay.config.growth.auto_xp = true;
        for (s, pool) in [0, -5, 1].into_iter().enumerate() {
            c.world.stats.available_xp = pool;
            c.autoplay_spend_xp(at(30 + s as u64));
        }
        assert_eq!(c.session.actions_sent(), 0, "spent with nothing to spend");

        // With something in the pool, a rank goes out.
        c.world.stats.available_xp = 1_000_000;
        c.autoplay_spend_xp(at(40));
        assert_eq!(c.session.actions_sent(), 1);
    }

    #[test]
    fn a_rank_the_server_refuses_is_not_asked_for_again_while_it_sulks() {
        let Some(mut c) = with_experience_to_spend(1_000_000) else {
            return;
        };
        let start = Instant::now();
        assert!(c.grow_spend_xp(start));
        let (refused, _) = sent(&c);

        // Nothing comes back. Nothing more is asked for until the server
        // has had time to answer, from the look after it went out.
        let out = start + Duration::from_millis(100);
        assert!(!c.grow_spend_xp(out));
        assert!(!c.grow_spend_xp(out + RAISE_SETTLE / 2));
        assert_eq!(c.session.actions_sent(), 1);

        // Then it is taken as refused, and the rest are bought instead for
        // as long as the refusal holds, the server taking each.
        let sulked = out + RAISE_SETTLE;
        let mut t = sulked;
        let mut bought = 0;
        while t < sulked + SULK_FOR {
            if c.grow_spend_xp(t) {
                let (pick, batch) = sent(&c);
                assert_ne!(
                    pick,
                    refused,
                    "asked again {:?} after it was refused",
                    t - start
                );
                server_takes(&mut c, pick, batch);
                bought += 1;
            }
            t += Duration::from_secs(5);
        }
        assert!(bought > 0, "nothing else was bought");

        // Once the sulk is over it is the best buy again, and what is left
        // of the pool was kept for it.
        assert!(c.grow_spend_xp(sulked + SULK_FOR + Duration::from_secs(5)));
        assert_eq!(sent(&c).0, refused);
    }

    #[test]
    fn a_raise_is_not_given_up_on_before_it_has_gone_out() {
        // A round of nine headless characters took four and a half seconds
        // on the local server. A raise queued in it goes on the wire at the
        // top of the next tick, and was given up on in that same tick,
        // before any answer could come: nine of them, and all nine landed a
        // third of a second later.
        let Some(mut c) = with_experience_to_spend(10_000_000_000) else {
            return;
        };
        let start = Instant::now();
        let round = Duration::from_millis(4_600);
        assert!(c.grow_spend_xp(start));
        let (pick, batch) = sent(&c);
        assert!(!c.grow_spend_xp(start + round));
        assert!(
            c.autoplay.growth.sulking.is_empty(),
            "{pick:?} was given up on as it went out"
        );
        // The answer comes, and the next raise goes out on the round after.
        server_takes(&mut c, pick, batch);
        assert!(c.grow_spend_xp(start + round * 2));
        assert!(c.autoplay.growth.sulking.is_empty());
        assert_eq!(c.session.actions_sent(), 2);
    }

    #[test]
    fn an_answer_that_comes_late_ends_the_wait_and_finds_its_share_kept() {
        // When the local server stopped answering for forty seconds, nine
        // characters gave up on ninety raises, and nearly all of them
        // landed afterwards. A stat given up on must not lose its share of
        // the pool to the others meanwhile, nor stay shut out once its
        // answer is in.
        let pool = 10_000_000_000;
        let (Some(mut slow), Some(mut calm)) = (
            with_experience_to_spend(pool),
            with_experience_to_spend(pool),
        ) else {
            return;
        };
        let step = Duration::from_millis(100);
        let start = Instant::now();
        let spend_until = |c: &mut Client, from: Instant, until: Instant| {
            let mut t = from;
            while t < until {
                if c.grow_spend_xp(t) {
                    let (pick, batch) = sent(c);
                    server_takes(c, pick, batch);
                }
                t += step;
            }
        };
        spend_until(&mut calm, start, start + Duration::from_secs(120));

        // The first raise goes unanswered, and is given up on.
        assert!(slow.grow_spend_xp(start));
        let (late, late_batch) = sent(&slow);
        let mut t = start + step;
        loop {
            assert!(t < start + Duration::from_secs(5), "never given up on");
            let went = slow.grow_spend_xp(t);
            t += step;
            if slow.autoplay.growth.sulking.is_empty() {
                assert!(!went, "another raise went out before it was given up on");
                continue;
            }
            // Given up on; the next raise goes out in the same look.
            if went {
                let (pick, batch) = sent(&slow);
                assert_ne!(pick, late, "sized again before its answer came");
                server_takes(&mut slow, pick, batch);
            }
            break;
        }
        // The rest go out and are answered at once, and none of them is
        // the late one sized again.
        let mut others = 0;
        while t < start + Duration::from_secs(20) {
            if slow.grow_spend_xp(t) {
                let (pick, batch) = sent(&slow);
                assert_ne!(pick, late, "sized again before its answer came");
                server_takes(&mut slow, pick, batch);
                others += 1;
            }
            t += step;
        }
        assert!(others > 0, "nothing else went out");
        // Twenty seconds on, the answer comes. The pool still holds it.
        server_takes(&mut slow, late, late_batch);
        spend_until(&mut slow, t, t + Duration::from_secs(100));
        assert!(
            slow.autoplay.growth.sulking.is_empty(),
            "still waiting on an answer that came"
        );
        // And it all comes to what a server that answers at once gives.
        let (s, c) = (&slow.world.stats, &calm.world.stats);
        assert_eq!(s.available_xp, c.available_xp);
        assert_eq!(s.attributes, c.attributes);
        assert_eq!(s.vitals, c.vitals);
    }

    #[test]
    fn a_stat_is_not_sized_again_before_its_answer_comes() {
        // A kill's experience moves the pool too. Taken for the answer, it
        // let the stat be sized again from the record the answer had not
        // reached yet, and bought twice.
        let Some(mut c) = with_experience_to_spend(10_000_000_000) else {
            return;
        };
        let start = Instant::now();
        assert!(c.grow_spend_xp(start));
        let (pick, batch) = sent(&c);
        c.world.stats.available_xp += 2_000_000;
        for n in 1..=5 {
            assert!(
                !c.grow_spend_xp(start + RAISE_EVERY * n),
                "a raise went out {:?} after {pick:?}, before its answer",
                RAISE_EVERY * n
            );
        }
        server_takes(&mut c, pick, batch);
        assert!(c.grow_spend_xp(start + RAISE_EVERY * 6));
        let (next, next_batch) = sent(&c);
        server_takes(&mut c, next, next_batch);
        assert_eq!(c.session.actions_sent(), 2);
    }

    /// The raise last sent, still waiting on its answer.
    fn sent(c: &Client) -> (Raise, Batch) {
        let p = c.autoplay.growth.pending.expect("a raise was sent");
        (p.raise, p.batch)
    }

    /// The server taking a raise as ACE does. It would refuse one past the
    /// pool or past the experience left to the top, so neither may ever be
    /// sent; otherwise the pool pays, the stat takes all of it, and its
    /// rank is the highest the experience spent on it reaches.
    fn server_takes(c: &mut Client, pick: Raise, batch: Batch) {
        let table = c.assets.xp_table().expect("the XpTable");
        let ladder = c.raise_ladder(&table, pick).expect("on the sheet");
        let left = ladder.cost_to(ladder.top()).expect("not at the top");
        assert!(
            batch.xp <= left,
            "{pick:?}: {} xp is past the {left} left to the top",
            batch.xp
        );
        assert!(
            i64::from(batch.xp) <= c.world.stats.available_xp,
            "{pick:?}: {} xp is more than the pool of {}",
            batch.xp,
            c.world.stats.available_xp
        );
        let ranks = ladder.rank_after(batch.xp);
        assert_eq!(
            ranks,
            ladder.ranks + batch.ranks,
            "{pick:?}: the server's rank is not the one it was sized for"
        );
        let stats = &mut c.world.stats;
        stats.available_xp -= i64::from(batch.xp);
        match pick {
            Raise::Skill(id) => {
                let s = stats
                    .skills
                    .iter_mut()
                    .find(|s| s.id == id)
                    .expect("on the sheet");
                s.ranks = ranks as u16;
                s.xp += batch.xp;
            }
            Raise::Attribute(i) => {
                stats.attributes[i].ranks = ranks;
                stats.attributes[i].xp += batch.xp;
            }
            Raise::Vital(i) => {
                stats.vitals[i].ranks = ranks;
                stats.vitals[i].xp += batch.xp;
            }
        }
    }

    /// Up to `messages` raises, a `RAISE_EVERY` apart from `from`, the
    /// server taking each one.
    fn buy_ranks(c: &mut Client, from: Instant, messages: u32) -> Vec<Raise> {
        let mut bought = Vec::new();
        for n in 0..messages {
            if !c.grow_spend_xp(from + RAISE_EVERY * n) {
                continue;
            }
            let (pick, batch) = sent(c);
            server_takes(c, pick, batch);
            bought.push(pick);
        }
        bought
    }

    /// Spending looked at every 100 ms for `how_long` from `from`, the
    /// server taking each raise as it goes out. When it stopped.
    fn spend_for(c: &mut Client, from: Instant, how_long: Duration) -> Instant {
        let mut t = from;
        while t < from + how_long {
            if c.grow_spend_xp(t) {
                let (pick, batch) = sent(c);
                server_takes(c, pick, batch);
            }
            t += Duration::from_millis(100);
        }
        t
    }

    #[test]
    fn raises_sent_while_the_server_is_not_answering_come_to_no_more_than_the_pool() {
        // For forty seconds the local server answered nothing. Each raise
        // was given up on after RAISE_SETTLE and the next one sized on a
        // pool not yet charged for those before it, and without the stats
        // given up on, so each was given their shares too. On paper, on the
        // real XpTable, a caster given ten billion queued eighteen raises
        // for thirty-six billion, and the server refuses all it cannot pay.
        let pool = 10_000_000_000;
        let (Some(mut stalled), Some(mut calm)) = (
            with_experience_to_spend(pool),
            with_experience_to_spend(pool),
        ) else {
            return;
        };
        let start = Instant::now();
        spend_for(&mut calm, start, Duration::from_secs(120));

        let mut queued = Vec::new();
        let mut t = start;
        while t < start + Duration::from_secs(90) {
            if stalled.grow_spend_xp(t) {
                queued.push(sent(&stalled));
            }
            t += Duration::from_millis(100);
        }
        assert!(queued.len() > 1, "only {queued:?} went out");
        let total: i64 = queued.iter().map(|(_, b)| i64::from(b.xp)).sum();
        assert!(
            total <= pool,
            "{total} xp queued against a pool of {pool}: {queued:?}"
        );
        // The server wakes and takes them in order, refusing none, and the
        // rest of the pool goes as it would have gone.
        for (pick, batch) in queued {
            server_takes(&mut stalled, pick, batch);
        }
        spend_for(&mut stalled, t, Duration::from_secs(120));
        let (s, c) = (&stalled.world.stats, &calm.world.stats);
        assert_eq!(s.available_xp, c.available_xp);
        assert_eq!(s.attributes, c.attributes);
        assert_eq!(s.vitals, c.vitals);
    }

    #[test]
    fn a_fight_does_not_sell_off_the_share_kept_for_the_maximums() {
        // On paper, on the real XpTable: a caster given ten billion in the
        // middle of a fight had every other stat bought to its share in
        // thirteen messages, and then sold a rank a message more of them
        // out of the share kept for the maximums. Fifty-six seconds in, Self,
        // Health and Mana came out of the fight ninety ranks short.
        let pool = 10_000_000_000;
        let (Some(mut fighting), Some(mut calm)) = (
            with_experience_to_spend(pool),
            with_experience_to_spend(pool),
        ) else {
            return;
        };
        let start = Instant::now();
        spend_for(&mut calm, start, Duration::from_secs(120));

        let foe = standing_by(&mut fighting, 0x8000_0301, "Revenant", 3.0).guid;
        fighting.attack_target = Some(foe);
        let bought = buy_ranks(&mut fighting, start, 80);
        assert!(!bought.is_empty(), "spending stopped for the fight");
        assert!(
            !bought.iter().copied().any(raises_a_maximum),
            "a maximum was raised mid-fight: {bought:?}"
        );
        use crate::advance::ATTRIBUTE_NAMES;
        let (f, c) = (&fighting.world.stats, &calm.world.stats);
        for (i, name) in ATTRIBUTE_NAMES.iter().enumerate() {
            let (f, c) = (f.attributes[i].ranks, c.attributes[i].ranks);
            assert!(f <= c, "{name} ran ahead to {f} in the fight, against {c}");
        }

        // The fight is over, and what was kept for the maximums buys them.
        fighting
            .world
            .objects
            .get_mut(&foe)
            .expect("the Revenant")
            .health = Some(0.0);
        spend_for(
            &mut fighting,
            start + RAISE_EVERY * 80,
            Duration::from_secs(120),
        );
        let (f, c) = (&fighting.world.stats, &calm.world.stats);
        assert_eq!(f.available_xp, c.available_xp);
        assert_eq!(f.attributes, c.attributes);
        assert_eq!(f.vitals, c.vitals);
    }

    #[test]
    fn a_rank_that_raises_a_maximum_waits_for_the_fight_to_be_over() {
        // A character with a large pool bought Health between swings. Its
        // maximum rose and what was left of it did not, until the fraction
        // left fell under the heal line and it healed, mid-fight, health
        // it had never lost.
        let pool = 100_000_000_000;
        let (Some(mut calm), Some(mut fighting)) = (
            with_experience_to_spend(pool),
            with_experience_to_spend(pool),
        ) else {
            return;
        };
        let start = Instant::now();
        // With nothing to fight, the maximums are among the best buys.
        let bought = buy_ranks(&mut calm, start, 20);
        assert!(
            bought.iter().copied().any(raises_a_maximum),
            "no maximum is worth buying here, so this proves nothing: {bought:?}"
        );

        let foe = standing_by(&mut fighting, 0x8000_0301, "Revenant", 3.0).guid;
        fighting.attack_target = Some(foe);
        let bought = buy_ranks(&mut fighting, start, 20);
        assert!(
            !bought.iter().copied().any(raises_a_maximum),
            "a maximum was raised mid-fight: {bought:?}"
        );
        // Spending did not stop for the fight: a pool this size takes every
        // attribute that raises no maximum to the top, a message each, and
        // leaves the ones that do where they were.
        let top = fighting
            .assets
            .xp_table()
            .expect("the XpTable")
            .attribute
            .len() as u32
            - 1;
        for (i, a) in fighting.world.stats.attributes.iter().enumerate() {
            let held = raises_a_maximum(Raise::Attribute(i));
            assert_eq!(
                a.ranks,
                if held { 0 } else { top },
                "{} after spending through the fight: {bought:?}",
                crate::advance::ATTRIBUTE_NAMES[i]
            );
        }

        // The Revenant is dead, its experience comes in, and the maximums
        // are bought again.
        fighting
            .world
            .objects
            .get_mut(&foe)
            .expect("the Revenant")
            .health = Some(0.0);
        fighting.world.stats.available_xp += 1_000;
        let bought = buy_ranks(&mut fighting, start + RAISE_EVERY * 20, 20);
        assert!(bought.iter().copied().any(raises_a_maximum), "{bought:?}");
    }

    #[test]
    fn a_pool_worth_hundreds_of_ranks_is_spent_within_a_minute() {
        // Nine characters granted billions on the local server bought
        // about a rank a second each: a large pool would have taken days.
        let pool = 1_000_000;
        let (Some(mut batched), Some(mut one_at_a_time)) = (
            with_experience_to_spend(pool),
            with_experience_to_spend(pool),
        ) else {
            return;
        };
        // A rank a message, as it was, to the end of the pool.
        let mut ranks = 0;
        while let Some(pick) = choose_raise(
            &one_at_a_time.raise_offers(),
            one_at_a_time.world.stats.available_xp,
        ) {
            let xp = one_at_a_time.raise_cost(pick).xp().expect("a price");
            server_takes(&mut one_at_a_time, pick, Batch { ranks: 1, xp });
            ranks += 1;
        }
        assert!(
            ranks >= 300,
            "the pool is worth only {ranks} ranks, so this proves nothing"
        );

        let start = Instant::now();
        let (mut t, mut messages) = (start, 0);
        while t < start + Duration::from_secs(60) {
            if batched.grow_spend_xp(t) {
                let (pick, batch) = sent(&batched);
                server_takes(&mut batched, pick, batch);
                messages += 1;
            }
            t += Duration::from_millis(100);
        }
        // All of it that a rank at a time spends: what is left is saved for
        // the next best buy.
        let left = batched.world.stats.available_xp;
        assert_eq!(
            left, one_at_a_time.world.stats.available_xp,
            "{left} of {pool} left after a minute and {messages} messages"
        );
        assert!(
            messages * 5 < ranks,
            "{messages} messages for {ranks} ranks"
        );
        // Spread over the attributes and vitals as a rank at a time
        // spreads it.
        use crate::advance::{ATTRIBUTE_NAMES, VITAL_NAMES};
        let (b, o) = (&batched.world.stats, &one_at_a_time.world.stats);
        for (i, name) in ATTRIBUTE_NAMES.iter().enumerate() {
            let (b, o) = (b.attributes[i].ranks, o.attributes[i].ranks);
            assert!(b.abs_diff(o) <= 1, "{name}: {b} against {o}");
        }
        for (i, name) in VITAL_NAMES.iter().enumerate() {
            let (b, o) = (b.vitals[i].ranks, o.vitals[i].ranks);
            assert!(b.abs_diff(o) <= 1, "{name}: {b} against {o}");
        }
    }

    #[test]
    fn a_kills_worth_of_experience_still_goes_a_rank_a_message() {
        let Some(mut c) = with_experience_to_spend(400) else {
            return;
        };
        let start = Instant::now();
        let (mut t, mut messages) = (start, 0);
        while t < start + Duration::from_secs(30) {
            if c.grow_spend_xp(t) {
                let (pick, batch) = sent(&c);
                let one = c.raise_cost(pick).xp().map(|xp| Batch { ranks: 1, xp });
                assert_eq!(Some(batch), one, "{pick:?}");
                server_takes(&mut c, pick, batch);
                messages += 1;
            }
            t += Duration::from_millis(100);
        }
        assert!(messages >= 2, "only {messages} ranks");
    }

    #[test]
    fn a_run_that_could_not_get_there_is_tried_again_once_its_wait_is_up() {
        // A run that really cannot reach its counter comes home with
        // nothing, so it is futile, and the counter is left alone for a
        // while. Neither is for good: the next run is due once the wait
        // between runs is up, and the counter can be chosen again by then.
        let t0 = Instant::now();
        let renald = spot(Vec2::new(32_587.2, 34_578.3));
        let mut skip = crate::did::Patience::new();
        skip.note(
            renald,
            &crate::did::Did::blocked("that counter was no use"),
            t0,
        );
        for party_restocking in [false, true] {
            let wait = wait_between_runs(party_restocking, true);
            assert!(wait > Duration::ZERO, "straight back to the same counter");
            assert!(wait <= RUN_EVERY);
            assert!(
                !skip.held(&renald, t0 + wait),
                "the counter is still skipped when the next run is due"
            );
        }
        // A party that did buy or sell something may go again at once.
        assert_eq!(wait_between_runs(true, false), Duration::ZERO);
    }

    #[test]
    fn past_the_servers_wall_not_even_a_coin_comes_off() {
        // +Verity: 36462 carried, 7500 capacity. The server hands nothing
        // to a character past three times its capacity, whatever it weighs.
        assert!(past_the_wall(36_462, 7_500));
        // At the wall a coin still fits, and under it so do light things.
        assert!(!past_the_wall(22_500, 7_500));
        assert!(!past_the_wall(13_866, 9_000));
        // Strength not heard yet is no reason to leave every body alone.
        assert!(!past_the_wall(13_866, 0));
    }

    #[test]
    fn a_gem_into_the_hub_puts_the_towns_within_reach() {
        let gem = |g: &ac_world::gems::Gem| ac_world::trip::Gem {
            guid: 1,
            name: g.name.clone(),
            exit: g.xy(),
            exit_cell: g.cell,
            summons: true,
        };
        let named = |n: &str| {
            ac_world::gems::all()
                .iter()
                .find(|g| g.name == n)
                .unwrap_or_else(|| panic!("no {n}"))
        };
        // A gem that lands outdoors is its landing and nothing more.
        let farms = named("Cragstone Farms Portal Gem");
        assert_eq!(gem_ways(&[gem(farms)]).len(), 1);

        // The Town Network gem comes out in the hub, indoors...
        let network = named("Town Network Portal Gem");
        assert!(network.cell & 0xFFFF >= 0x100, "the hub is indoors");
        let ways = gem_ways(&[gem(network)]);
        assert!(ways.len() > 30, "{} ways", ways.len());
        // ...and the counter a mid-level character sells at, the
        // Arcanum Broker outside Cragstone, is a short walk from where
        // one of the hub's portals comes out.
        let broker = ac_world::shops::all()
            .iter()
            .find(|s| s.name == "Arcanum Broker" && s.cell & 0xFFFF_0000 == 0xBB9F_0000)
            .expect("the broker outside Cragstone");
        let (far, via) = ways
            .iter()
            .map(|(w, n)| (broker.xy().distance(*w), n.as_str()))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .unwrap();
        assert!(far <= NEAR_A_WAY_OUT, "{far} m from {via}");
        assert_eq!(via, "Town Network Portal Gem, then Portal to Cragstone");
        // Judged by the hub alone it was out of reach: the fault.
        assert!(broker.xy().distance(network.xy()) > NEAR_A_WAY_OUT);
    }

    #[test]
    fn the_best_value_rank_is_bought_and_none_beyond_the_pool() {
        let offers = [
            offer(Raise::Skill(47), 1000, 1.0),
            offer(Raise::Skill(24), 300, 0.3),
            offer(Raise::Attribute(3), 500, 0.6),
            offer(Raise::Vital(0), 400, 0.5),
        ];
        // Attribute: 500/0.6 = 833 beats 1000, 1000 and 800? No: health
        // 400/0.5 = 800 is the best value.
        assert_eq!(choose_raise(&offers, 5000), Some(Raise::Vital(0)));
        // With less to spend than the best buy costs, nothing: the pool
        // saves for it rather than going on a worse buy that fits.
        assert_eq!(choose_raise(&offers, 399), None);
        assert_eq!(choose_raise(&offers, 350), None);
        assert_eq!(choose_raise(&offers, 400), Some(Raise::Vital(0)));
        assert_eq!(choose_raise(&[], 100), None);
        // The main skill wins over a minor one at the same price.
        let tie = [
            offer(Raise::Skill(24), 300, 0.3),
            offer(Raise::Skill(47), 300, 1.0),
        ];
        assert_eq!(choose_raise(&tie, 300), Some(Raise::Skill(47)));
        // A zero weight is never bought, nor saved for.
        assert_eq!(choose_raise(&[offer(Raise::Skill(1), 1, 0.0)], 10), None);
        let free = [
            offer(Raise::Skill(1), 1, 0.0),
            offer(Raise::Skill(24), 300, 0.3),
        ];
        assert_eq!(choose_raise(&free, 300), Some(Raise::Skill(24)));
    }

    /// A column shaped like the XpTable's: the first rank costs `first`,
    /// each after it a twentieth more, up to rank `top`.
    fn column(first: u32, top: usize) -> Vec<u32> {
        let mut table = vec![0u32];
        let (mut total, mut step) = (0.0f64, f64::from(first));
        for _ in 0..top {
            total += step.round();
            table.push(total as u32);
            step *= 1.05;
        }
        table
    }

    /// A stat standing at `ranks` of `table`, nothing spent past them, and
    /// not held.
    fn climb(raise: Raise, table: &[u32], ranks: u32, weight: f32) -> Climb<'_> {
        Climb {
            raise,
            ladder: crate::advance::Ladder {
                table,
                ranks,
                spent: table[ranks as usize],
            },
            weight,
            held: false,
        }
    }

    /// Spend `pool` over `field` the way `grow_spend_xp` does, a message at
    /// a time until nothing more is affordable, and take each message as
    /// the server does: batched, or a rank a message as before batches.
    /// `each` looks at the field after every message. How many messages
    /// it took, and what is left of the pool.
    fn spend_on_paper(
        field: &mut [Climb],
        mut pool: i64,
        batched: bool,
        mut each: impl FnMut(&[Climb]),
    ) -> (u32, i64) {
        let mut messages = 0;
        loop {
            let offers: Vec<Offer> = field
                .iter()
                .filter_map(|c| Some(offer(c.raise, c.ladder.next_cost().xp()?, c.weight)))
                .collect();
            let Some(pick) = choose_raise(&offers, pool) else {
                return (messages, pool);
            };
            let batch = if batched {
                let (raise, batch) = batch_raise(field, pool).expect("the best buy is affordable");
                assert_eq!(raise, pick, "sent for another stat than the best buy");
                batch
            } else {
                let o = offers.iter().find(|o| o.raise == pick).expect("on offer");
                Batch {
                    ranks: 1,
                    xp: o.cost,
                }
            };
            let c = field
                .iter_mut()
                .find(|c| c.raise == pick)
                .expect("in the field");
            let left = c.ladder.cost_to(c.ladder.top()).expect("not at the top");
            assert!(
                batch.xp <= left,
                "{pick:?}: {} is past the {left} left to the top",
                batch.xp
            );
            assert!(
                i64::from(batch.xp) <= pool,
                "{pick:?}: {} is more than the pool of {pool}",
                batch.xp
            );
            let ranks = c.ladder.rank_after(batch.xp);
            assert_eq!(ranks, c.ladder.ranks + batch.ranks, "{pick:?}: sized wrong");
            pool -= i64::from(batch.xp);
            c.ladder.ranks = ranks;
            c.ladder.spent += batch.xp;
            messages += 1;
            each(field);
        }
    }

    #[test]
    fn a_large_pool_goes_out_a_message_a_stat_and_spread_as_a_rank_at_a_time_spreads_it() {
        let (attribute, vital, skill) = (column(110, 190), column(73, 196), column(23, 226));
        let start = [
            climb(Raise::Skill(47), &skill, 40, 1.0),
            climb(Raise::Skill(6), &skill, 20, 0.5),
            climb(Raise::Attribute(3), &attribute, 30, 0.6),
            climb(Raise::Attribute(0), &attribute, 10, 0.15),
            climb(Raise::Vital(0), &vital, 5, 0.5),
            climb(Raise::Vital(2), &vital, 0, 0.15),
        ];
        for pool in [500_000, 5_000_000] {
            // A rank a message, as it was: hundreds of messages.
            let mut one_at_a_time = start;
            let (slow, left) = spend_on_paper(&mut one_at_a_time, pool, false, |_| {});
            assert!(
                slow >= 300,
                "a pool of {pool} is worth only {slow} ranks, so this proves nothing"
            );

            // Batched: about a message a stat, and no stat is ever taken
            // past the rank a rank at a time leaves it at.
            let mut batched = start;
            let (fast, batched_left) = spend_on_paper(&mut batched, pool, true, |field| {
                for (now, end) in field.iter().zip(&one_at_a_time) {
                    assert!(
                        now.ladder.ranks <= end.ladder.ranks,
                        "{:?} ran ahead to {} of the {} it ends at",
                        now.raise,
                        now.ladder.ranks,
                        end.ladder.ranks
                    );
                }
            });
            assert!(
                fast <= 2 * start.len() as u32,
                "{fast} messages for {} stats, against {slow} a rank at a time",
                start.len()
            );
            // And it ends where a rank at a time ends, rank for rank.
            assert_eq!(batched_left, left);
            for (b, o) in batched.iter().zip(&one_at_a_time) {
                assert_eq!(b.ladder, o.ladder, "{:?} with a pool of {pool}", b.raise);
            }
        }
    }

    #[test]
    fn a_kills_worth_of_experience_buys_the_one_rank_as_it_always_did() {
        let skill = column(23, 226);
        let field = [
            climb(Raise::Skill(47), &skill, 40, 1.0),
            climb(Raise::Skill(6), &skill, 40, 0.5),
        ];
        let rank = field[0].ladder.step(40).expect("a rank above");
        let one = Some((Raise::Skill(47), Batch { ranks: 1, xp: rank }));
        for pool in [rank, 3 * rank, BATCH_FROM as u32 * rank - 1] {
            assert_eq!(
                batch_raise(&field, i64::from(pool)),
                one,
                "a pool of {pool}"
            );
        }
        // Short of the rank, nothing; worth many of it, several.
        assert_eq!(batch_raise(&field, i64::from(rank) - 1), None);
        let (_, many) = batch_raise(&field, 100 * i64::from(rank)).expect("a batch");
        assert!(many.ranks > 1, "{many:?}");
        // Nothing while every stat is held.
        let held = field.map(|c| Climb { held: true, ..c });
        assert_eq!(batch_raise(&held, 1_000_000), None);
    }

    #[test]
    fn a_trickle_of_kills_buys_what_the_same_experience_in_one_lump_buys() {
        // On paper, on the real XpTable: after a caster spent ten billion,
        // two billion more a kill at a time went only on ranks that cost
        // little -- Arcane Lore, Jump, Loyalty, Salvaging, Strength,
        // Coordination and Quickness ten each -- and War, Life, Focus,
        // Self and Health none, where the same two billion at once gave War
        // three and the others two. A kill never filled the pool to the
        // best buy before a cheap rank took it.
        let (attribute, vital, skill) = (column(110, 190), column(73, 196), column(23, 226));
        let mut spent = [
            climb(Raise::Skill(47), &skill, 0, 1.0),
            climb(Raise::Skill(6), &skill, 0, 0.6),
            climb(Raise::Skill(22), &skill, 0, 0.15),
            climb(Raise::Attribute(4), &attribute, 0, 0.6),
            climb(Raise::Attribute(0), &attribute, 0, 0.15),
            climb(Raise::Vital(0), &vital, 0, 0.5),
        ];
        let (_, left) = spend_on_paper(&mut spent, 5_000_000, true, |_| {});
        let (kill, kills): (i64, i64) = (2_000, 1_000);
        // The cheapest rank now costs several kills, and the best buy many
        // more.
        let next = |c: &Climb| c.ladder.step(c.ladder.ranks).expect("below the top");
        assert!(
            spent.iter().all(|c| i64::from(next(c)) > 4 * kill),
            "{spent:?}"
        );

        let mut lump = spent;
        let (_, lump_left) = spend_on_paper(&mut lump, left + kill * kills, true, |_| {});
        let mut trickle = spent;
        let mut pool = left;
        for _ in 0..kills {
            (_, pool) = spend_on_paper(&mut trickle, pool + kill, true, |_| {});
        }
        assert_eq!(pool, lump_left);
        for ((t, l), s) in trickle.iter().zip(&lump).zip(&spent) {
            assert_eq!(t.ladder, l.ladder, "{:?}", t.raise);
            assert!(
                t.ladder.ranks > s.ladder.ranks,
                "{:?} got nothing of {} kills",
                t.raise,
                kills
            );
        }
    }

    #[test]
    fn a_batch_stops_at_the_top_rank_whatever_the_pool() {
        // A stat partway to its next rank, a column whose top is nearly all
        // a u32 holds, and a pool of a hundred billion.
        let table = [0u32, 1_000, 3_000, 4_000_000_000, u32::MAX - 5];
        let partway = Climb {
            raise: Raise::Attribute(0),
            ladder: crate::advance::Ladder {
                table: &table,
                ranks: 1,
                spent: 2_500,
            },
            weight: 1.0,
            held: false,
        };
        let batch = batch_raise(&[partway], 100_000_000_000).expect("a batch");
        // To the top and no further: exactly the experience left to it,
        // which is all the server takes and fits the u32 a message carries.
        assert_eq!(
            batch,
            (
                partway.raise,
                Batch {
                    ranks: 3,
                    xp: u32::MAX - 5 - 2_500
                }
            )
        );
        assert_eq!(
            Some(batch.1.xp),
            partway.ladder.cost_to(partway.ladder.top())
        );
        // At the top, nothing.
        let topped = Climb {
            ladder: crate::advance::Ladder {
                ranks: 4,
                spent: u32::MAX - 5,
                ..partway.ladder
            },
            ..partway
        };
        assert_eq!(batch_raise(&[topped], 100_000_000_000), None);
    }

    #[test]
    fn a_maximum_held_for_the_fight_keeps_its_share_of_the_pool() {
        let (skill, vital) = (column(23, 226), column(73, 196));
        let weapon = climb(Raise::Skill(47), &skill, 50, 1.0);
        let health = climb(Raise::Vital(0), &vital, 10, 0.5);
        let held = Climb {
            held: true,
            ..health
        };
        let pool = 1_000_000;
        // With nothing else in the running, the weapon's skill takes the
        // pool.
        let (_, alone) = batch_raise(&[weapon], pool).expect("a batch");
        assert!(i64::from(alone.xp) > pool * 9 / 10, "{alone:?}");
        // With Health held for the fight but still in the running, only
        // its own share, and Health nothing.
        let (raise, sharing) = batch_raise(&[weapon, held], pool).expect("a batch");
        assert_eq!(raise, weapon.raise);
        assert!(
            sharing.ranks < alone.ranks && sharing.xp < alone.xp,
            "{sharing:?} against {alone:?}"
        );

        // Nor is a rank of the weapon's sold out of Health's share when the
        // pool runs short. Health's next three ranks are better buys than
        // the weapon's next, and the one after them is not.
        let (h, w) = (
            |r| health.ladder.step(r).expect("below the top"),
            weapon.ladder.step(50).expect("below the top"),
        );
        let worth = |cost: u32, weight: f32| cost as f32 / weight;
        assert!(worth(h(12), 0.5) < worth(w, 1.0) && worth(h(13), 0.5) > worth(w, 1.0));
        let kept = h(10) + h(11) + h(12);
        // A pool one short of those and the weapon's rank: the weapon waits.
        assert_eq!(batch_raise(&[weapon, held], i64::from(kept + w - 1)), None);
        // Enough for both: the weapon's one rank, and no more.
        assert_eq!(
            batch_raise(&[weapon, held], i64::from(kept + w)),
            Some((weapon.raise, Batch { ranks: 1, xp: w }))
        );
        // Not held, Health is bought first.
        assert_eq!(
            batch_raise(&[weapon, health], i64::from(kept + w - 1)).map(|(r, _)| r),
            Some(health.raise)
        );
    }

    #[test]
    fn skills_are_weighed_by_how_the_character_fights() {
        // An archer: bows first, fletching worth something, war magic not.
        assert_eq!(skill_weight(47, Some(47), false), 1.0);
        assert!(skill_weight(37, Some(47), false) > skill_weight(37, Some(44), false));
        assert!(skill_weight(34, Some(47), false) < 0.5);
        // A caster: war magic first, life magic close behind.
        assert_eq!(skill_weight(34, Some(0), true), 1.0);
        assert!(skill_weight(33, Some(0), true) > 0.8);
        // Another weapon skill is nearly worthless.
        assert!(skill_weight(44, Some(47), false) < 0.2);
    }

    #[test]
    fn a_stack_worth_more_than_the_counter_allows_is_sold_in_pieces() {
        // A hundred Pyreal Peas: fifty thousand each and five million
        // the stack, and the server reckons a stack's value as the lot.
        // A counter that will not look at anything over a million
        // refuses all hundred -- so it takes twenty at a time.
        let peas = Salable {
            guid: 1,
            item_type: item_type::SPELL_COMPONENTS,
            value: 5_000_000,
            stack: 100,
        };
        assert_eq!(peas.each(), 50_000);
        assert_eq!(peas.at_once(1_000_000), 20);
        assert!(
            peas.taken_by(item_type::SPELL_COMPONENTS, 0, 1_000_000),
            "in pieces, but taken"
        );

        // A counter with no ceiling takes the lot in one go.
        assert_eq!(peas.at_once(0), 100);

        // One too dear even singly is not taken at all, and saying so
        // is better than splitting a stack down to nothing.
        let jewel = Salable {
            guid: 2,
            item_type: item_type::JEWELRY,
            value: 4_000_000,
            stack: 1,
        };
        assert_eq!(jewel.at_once(1_000_000), 0);
        assert!(!jewel.taken_by(item_type::JEWELRY, 0, 1_000_000));

        // And the counter's floor is about one of them, not the pile:
        // a stack of cheap things is not made sellable by being big.
        let chaff = Salable {
            guid: 3,
            item_type: item_type::MISC,
            value: 10_000,
            stack: 1_000,
        };
        assert_eq!(chaff.each(), 10);
        assert!(
            !chaff.taken_by(item_type::MISC, 100, 0),
            "ten is under the floor"
        );
    }

    fn need(kind: NeedKind, want: u32) -> Need {
        Need {
            name: "x".into(),
            want,
            have: 0,
            keep: want,
            urgent: true,
            buyable: true,
            from: None,
            kind,
        }
    }

    #[test]
    fn a_want_that_names_its_counter_is_only_filled_there() {
        // Fletching supplies come in levels and elements that one bowyer
        // carries and the next does not, so a player who says where a
        // line comes from means it. The field was editable and saved and
        // read by nothing, so every counter that name-matched would do.
        let mut named = need(NeedKind::Named("Acid Arrowhead".into()), 500);
        named.from = Some("Thimrin Woodsetter".into());
        assert!(named.may_buy_at("Thimrin Woodsetter"));
        assert!(
            named.may_buy_at("thimrin woodsetter"),
            "however it is typed"
        );
        assert!(!named.may_buy_at("Scildith Dyrson the Bowyer"));

        // A line that names nobody is bought wherever it is sold, which
        // is every line a character has unless it says otherwise.
        let anywhere = need(NeedKind::Component(691), 1000);
        assert!(anywhere.may_buy_at("Anyone At All"));
    }

    #[test]
    fn a_long_list_is_cut_short() {
        assert_eq!(a_few(&[]), "nothing");
        assert_eq!(a_few(&["Myrrh"]), "Myrrh");
        assert_eq!(a_few(&["a", "b", "c", "d"]), "a, b, c, d");
        assert_eq!(a_few(&["a", "b", "c", "d", "e"]), "a, b, c, d and 1 more");
    }

    fn ware(wcid: u32, name: &str, value: u32) -> ac_world::shops::Ware {
        ac_world::shops::Ware {
            wcid,
            name: name.into(),
            item_type: ac_world::item_type::SPELL_COMPONENTS,
            value,
        }
    }

    fn shop(name: &str, sells: Vec<ac_world::shops::Ware>) -> ac_world::shops::Shop {
        ac_world::shops::Shop {
            wcid: 1,
            name: name.into(),
            cell: 0x0001_0001,
            at: glam::Vec3::ZERO,
            sells,
            buys: ac_world::item_type::GEM,
            min_value: 10,
            max_value: 0,
            buy_rate: 0.5,
            sell_rate: 2.0,
            gate: None,
        }
    }

    fn salable(guid: u32, item_type: u32, value: u32, stack: u32) -> Salable {
        Salable {
            guid,
            item_type,
            value,
            stack,
        }
    }

    #[test]
    fn a_counter_behind_a_door_is_not_planned_around() {
        use ac_world::shops::Gate;
        let mut hall = shop(
            "Vermilia the Archmage",
            vec![ware(37155, "Mana Scarab", 15_000)],
        );
        hall.gate = Some(Gate::Society(4));
        // A very good archmage, and no use at all to anyone else's
        // society or to nobody's.
        assert!(hall.open_to(4, &[]));
        assert!(!hall.open_to(2, &[]));
        assert!(!hall.open_to(0, &[]));
        let mut chapter = shop("Rossu Morta Quartermaster", vec![]);
        chapter.gate = Some(Gate::Quest("RossuMortaChapterhouse_Flag".into()));
        assert!(!chapter.open_to(4, &[]), "no society opens a quest door");
        assert!(chapter.open_to(0, &["rossumortachapterhouse_flag".to_string()]));
    }

    #[test]
    fn case_folds_without_allocating() {
        assert!(contains_fold("Prismatic Taper", "taper"));
        assert!(contains_fold("PRISMATIC TAPER", "prismatic"));
        assert!(!contains_fold("Lead Scarab", "taper"));
        assert!(!contains_fold("Tap", "taper"));
        assert!(!contains_fold("anything", ""));
    }

    #[test]
    fn the_counter_that_pays_best_wins_when_neither_has_the_order() {
        // Around Cragstone the Scriveners are eighty metres away and pay
        // half; the Arcanum Broker is three hundred metres further on
        // and pays 0.95. Neither stocks what a hunting character came to
        // buy, so the order is a tie -- and the old ranking fell through
        // to distance, walked past the broker every time, and took half
        // price on every sale.
        let look = |takings: u32, stocks: usize| Forecast {
            takings,
            stocks: vec!["something".into(); stocks],
            ..Default::default()
        };
        let near = look(500, 0);
        let far = look(950, 0);
        assert_eq!(
            better_counter(Errand::Buy, (&far, 380.0), (&near, 80.0)),
            std::cmp::Ordering::Less,
            "the broker is worth the extra three hundred metres"
        );

        // But only when the order is a tie: a counter that has what the
        // character came for still beats a richer one that does not.
        let stocked_but_poor = look(0, 2);
        assert_eq!(
            better_counter(Errand::Buy, (&stocked_but_poor, 380.0), (&far, 80.0)),
            std::cmp::Ordering::Less,
            "what it came to buy comes first"
        );
    }

    #[test]
    fn distance_only_settles_what_pay_and_stock_leave_even() {
        let same = Forecast {
            takings: 100,
            ..Default::default()
        };
        assert_eq!(
            better_counter(Errand::Buy, (&same, 50.0), (&same, 900.0)),
            std::cmp::Ordering::Less,
            "all else equal, the nearer one"
        );
    }

    #[test]
    fn a_trip_is_judged_before_it_is_walked() {
        let mut tapers = need(NeedKind::Component(691), 100);
        tapers.name = "Prismatic Taper".into();
        let mut scarabs = need(NeedKind::Component(690), 20);
        scarabs.name = "Lead Scarab".into();
        let needs = [tapers, scarabs];
        let wants: Vec<(&Need, String)> = needs.iter().map(|n| (n, String::new())).collect();

        let mage = shop(
            "Archmage",
            vec![
                ware(691, "Prismatic Taper", 5),
                ware(690, "Lead Scarab", 30),
            ],
        );
        let smith = shop("Blacksmith", vec![ware(20630, "Trade Note", 250)]);

        // Money in hand, everything on the shelf: the whole order in one
        // stop, and the trip is worth walking.
        let rich = forecast(&mage, &wants, 10_000, &[]);
        assert_eq!(rich.bill, 100 * 10 + 20 * 60);
        assert_eq!(rich.cheapest, 10);
        assert!(rich.missing.is_empty());
        assert!(rich.covers_it());
        assert!(rich.worth_going());

        // Enough for some of it is still worth walking: a mage that can
        // afford half its tapers is a mage that can keep casting.
        let thin = forecast(&mage, &wants, 50, &[]);
        assert!(!thin.covers_it());
        assert!(thin.worth_going());

        // Not a copper, and nothing to sell: the trip buys nothing and
        // is not made.
        let broke = forecast(&mage, &wants, 0, &[]);
        assert_eq!(broke.funds(), 0);
        assert!(!broke.worth_going());

        // Unless there is something to sell, which pays for the rest.
        //
        // Three gems, twelve hundred the lot: a stack's `value` is the
        // whole stack's, which is how the server reckons it and what a
        // counter's limits are measured against. Four hundred each, the
        // shop pays half, so six hundred for the three.
        let loot = [salable(9, ac_world::item_type::GEM, 1_200, 3)];
        let selling = forecast(&mage, &wants, 0, &loot);
        assert_eq!(selling.takings, 200 * 3);
        assert_eq!(selling.selling, 1);
        assert!(selling.worth_going());
        // What it will not buy does not count towards the trip.
        let armour = [salable(9, ac_world::item_type::ARMOR, 400, 1)];
        assert_eq!(forecast(&mage, &wants, 0, &armour).takings, 0);
        // Nor does what is beneath its notice -- and "beneath" is
        // about one of them, not the pile.
        let trinket = [salable(9, ac_world::item_type::GEM, 5, 1)];
        assert_eq!(forecast(&mage, &wants, 0, &trinket).takings, 0);

        // A counter that stocks none of it is no help however rich the
        // character is.
        let wrong = forecast(&smith, &wants, 100_000, &[]);
        assert_eq!(wrong.cheapest, 0);
        assert_eq!(wrong.missing.len(), 2);
        assert!(!wrong.covers_it());
        assert!(!wrong.worth_going());
    }

    #[test]
    fn a_named_ground_is_not_judged_on_level() {
        // Hunting a place is about its loot, its money, its trophies.
        // A ground the player named must not be refused because the
        // level table disapproves, and `suits` is the only thing that
        // would refuse it.
        let ground = ac_world::hunting::all()
            .iter()
            .find(|g| g.max_level < 50)
            .expect("a low-level ground");
        // Far too low for a level 275 character by the usual rule...
        assert!(!ground.suits(275, 8));
        // ...but naming it is a landblock id, and looking one up asks
        // nothing about levels.
        assert_eq!(
            ac_world::hunting::at(ground.landblock).map(|g| g.landblock),
            Some(ground.landblock)
        );
    }

    #[test]
    fn the_search_widens_rather_than_crossing_the_world() {
        // The bug: ranking on what a shop stocks alone sent a character
        // twenty-five kilometres to a counter with one more line on the
        // shelf. The rings mean a good enough shop in this town wins.
        let r = vendor_rings(None);
        assert_eq!(r.first().copied(), Some(600.0), "the town first");
        assert!(r.windows(2).all(|w| w[0] < w[1]), "{r:?} does not widen");
        assert_eq!(r.last().copied(), Some(f32::INFINITY), "and then anywhere");
    }

    #[test]
    fn a_capped_search_never_looks_past_the_cap() {
        // The next stop of a run stays in the same town.
        let r = vendor_rings(Some(500.0));
        assert_eq!(r, vec![500.0]);
        assert!(r.iter().all(|x| *x <= 500.0));
        // A cap between rings keeps the ones below it.
        let mid = vendor_rings(Some(1_000.0));
        assert_eq!(mid, vec![600.0, 1_000.0]);
    }

    #[test]
    fn there_is_always_a_last_ring_to_fall_back_on() {
        // Whatever the cap, the search ends somewhere rather than
        // leaving the character with nowhere to go.
        for cap in [1.0f32, 600.0, 3_000.0, 15_000.0, 100_000.0] {
            let r = vendor_rings(Some(cap));
            assert!(!r.is_empty(), "cap {cap}");
            assert_eq!(r.last().copied(), Some(cap));
        }
    }

    #[test]
    fn a_healing_kit_is_not_bought_for_someone_who_cannot_use_one() {
        // Untrained Healing: a kit restores next to nothing, so it is
        // not worth the money or the trip to a vendor.
        assert!(!worth_stocking("Healing Kit", false));
        assert!(!worth_stocking("Excellent Healing Kit", false));
        // Trained, it is worth having again.
        assert!(worth_stocking("Healing Kit", true));
        // Everything else is judged on its own, either way.
        assert!(worth_stocking("Prismatic Taper", false));
        assert!(worth_stocking("Mana Stone", false));
    }

    #[test]
    fn ammunition_is_the_plain_kind() {
        use ac_world::fletching::ammo_type;
        assert!(ammo_stock("Arrow", ammo_type::ARROW));
        assert!(ammo_stock("arrow ", ammo_type::ARROW));
        assert!(!ammo_stock("Fire Arrow", ammo_type::ARROW));
        assert!(!ammo_stock("Arrow", ammo_type::BOLT));
        assert!(ammo_stock("Quarrel", ammo_type::BOLT));
        assert!(ammo_stock("Atlatl Dart", ammo_type::ATLATL));
    }

    /// The character whose pack these tests are about.
    const TIDIER: u32 = 0x5000_0001;

    /// A character carrying `stacks` of `(guid, wcid, count, max)`,
    /// offline, when the archives are there to be read.
    fn carrying(stacks: &[(u32, u32, u32, u32)]) -> Option<Client> {
        let Some(dir) = std::env::var_os("AC_DATA_DIR") else {
            eprintln!("AC_DATA_DIR unset; skipping");
            return None;
        };
        let assets = std::rc::Rc::new(ac_scene::Assets::open(dir).unwrap());
        let mut c = offline_client(assets);
        c.world.player_guid = Some(TIDIER);
        for &(guid, wcid, count, max) in stacks {
            c.world.objects.insert(
                guid,
                ac_world::WorldObject {
                    guid,
                    weenie_class_id: wcid,
                    name: format!("thing {wcid}"),
                    stack_size: count,
                    max_stack_size: max,
                    container: Some(TIDIER),
                    parent: Some(TIDIER),
                    ..Default::default()
                },
            );
        }
        Some(c)
    }

    /// The server's answer to a whole pour: the source forgotten, the
    /// target's new size.
    fn poured(c: &mut Client, from: u32, to: u32, now: u32) {
        c.world.objects.remove(&from);
        if let Some(o) = c.world.objects.get_mut(&to) {
            o.stack_size = now;
        }
    }

    #[test]
    fn lead_peas_taken_in_two_stacks_are_poured_together_without_a_tick_of_their_own() {
        // The report: peas looted off one corpse after another sit in
        // stacks of their own. Nothing here waits for a quiet moment --
        // there is a fight on -- because the server makes a pack-to-pack
        // merge on the spot.
        let Some(mut c) = carrying(&[(1, 8329, 40, 100), (2, 8329, 5, 100)]) else {
            return;
        };
        c.attack_target = Some(0x8000_30F2);
        let t0 = Instant::now();
        c.autoplay_tidy(t0);
        let sent = c.autoplay.pour.clone().expect("a pour went out").0;
        assert_eq!(
            (sent.merge.from, sent.merge.to, sent.merge.amount),
            (2, 1, 5)
        );
        assert_eq!(sent.to_before, 40);
        // Nothing else is asked for while the server has not answered:
        // the counts a second choice would be made from are stale.
        c.autoplay_tidy(t0 + Duration::from_millis(100));
        assert_eq!(
            c.autoplay.pour.as_ref().map(|(p, _)| p.merge.clone()),
            Some(sent.merge.clone())
        );
        // The answer, read off the target's new size.
        poured(&mut c, 2, 1, 45);
        c.autoplay_tidy(t0 + Duration::from_millis(700));
        assert!(c.autoplay.pour.is_none(), "the pour landed");
    }

    #[test]
    fn a_pair_the_server_turns_down_does_not_stop_the_rest_being_tidied() {
        // One stubborn pair used to be the only answer ever offered, so
        // it was asked for every 600 ms and nothing else in the pack was
        // ever poured together.
        let Some(mut c) = carrying(&[
            (1, 273, 5000, 25000),
            (2, 273, 900, 25000),
            (3, 8329, 40, 100),
            (4, 8329, 5, 100),
        ]) else {
            return;
        };
        let t0 = Instant::now();
        c.autoplay_tidy(t0);
        let first = c.autoplay.pour.clone().expect("a pour went out").0.merge;
        assert_eq!((first.from, first.to), (2, 1));
        // InventoryServerSaveFailed naming the source, with no reason
        // at all, which is half of them.
        c.move_refused
            .insert(2, (0, t0 + Duration::from_millis(50)));
        c.autoplay_tidy(t0 + Duration::from_millis(700));
        assert!(
            !c.move_refused.contains_key(&2),
            "the refusal was this pour's and is spent"
        );
        let next = c.autoplay.pour.clone().expect("the next pair").0.merge;
        assert_eq!((next.from, next.to), (4, 3), "the peas go in instead");
    }

    #[test]
    fn a_refusal_naming_the_target_is_read_as_this_pours_answer() {
        // A stack that is stuck, or being traded, is refused under the
        // target's guid. Read only under the source's, these were never
        // seen at all and the same pair was offered every 600 ms.
        let Some(mut c) = carrying(&[(1, 8329, 40, 100), (2, 8329, 5, 100)]) else {
            return;
        };
        let t0 = Instant::now();
        c.autoplay_tidy(t0);
        c.move_refused
            .insert(1, (0x29, t0 + Duration::from_millis(50)));
        c.autoplay_tidy(t0 + Duration::from_millis(700));
        assert!(c.autoplay.pour.is_none(), "the pour is over");
        assert!(!c.move_refused.contains_key(&1));
        // And the pair is left alone for a while rather than asked again.
        assert!(c
            .autoplay
            .growth
            .wont_merge
            .held(&(2, 1), t0 + Duration::from_millis(800)));
    }

    #[test]
    fn stacks_whose_words_disagree_are_never_poured_together() {
        use ac_loot::LootAction;
        let stats = |guid: u32| crate::items::ItemStats {
            guid,
            wcid: 8329,
            name: "Lead Pea".into(),
            ..Default::default()
        };
        // The big stack is meant for a counter, the small one is kept.
        // Pouring the small one in would settle the survivor as kept
        // (the ledger takes the cautious answer), and forty peas the
        // player said to sell would stay in the pack for good: the
        // player's Sell overruled by a tidy. So nothing is poured.
        let Some(mut c) = carrying(&[(1, 8329, 40, 100), (2, 8329, 5, 100)]) else {
            return;
        };
        c.autoplay.ledger.remember(&stats(1), LootAction::Sell);
        c.autoplay.ledger.remember(&stats(2), LootAction::Keep);
        let t0 = Instant::now();
        c.autoplay_tidy(t0);
        assert!(
            c.autoplay.pour.is_none(),
            "poured across words: {:?}",
            c.autoplay.pour
        );
        assert_eq!(c.autoplay.ledger.by_guid(1), Some(LootAction::Sell));

        // Nor into a stack nothing was decided about. "Undecided" is
        // not "sell": the guards answer for the survivor, and a Sell
        // poured into it is a Sell lost.
        let Some(mut c) = carrying(&[(1, 8329, 40, 100), (2, 8329, 5, 100)]) else {
            return;
        };
        c.autoplay.ledger.remember(&stats(2), LootAction::Sell);
        c.autoplay_tidy(t0);
        assert!(c.autoplay.pour.is_none(), "{:?}", c.autoplay.pour);

        // Two stacks with one word are poured, and the word survives
        // the pour.
        let Some(mut c) = carrying(&[(1, 8329, 40, 100), (2, 8329, 5, 100)]) else {
            return;
        };
        c.autoplay.ledger.remember(&stats(1), LootAction::Sell);
        c.autoplay.ledger.remember(&stats(2), LootAction::Sell);
        c.autoplay_tidy(t0);
        let sent = c.autoplay.pour.clone().expect("a pour went out").0;
        assert_eq!((sent.merge.from, sent.merge.to), (2, 1));
        poured(&mut c, 2, 1, 45);
        c.autoplay_tidy(t0 + Duration::from_millis(700));
        assert_eq!(c.autoplay.ledger.by_guid(1), Some(LootAction::Sell));
    }

    #[test]
    fn a_pour_with_no_word_at_all_is_given_up_on_and_the_pack_read_afresh() {
        let Some(mut c) = carrying(&[(1, 8329, 40, 100), (2, 8329, 5, 100)]) else {
            return;
        };
        let t0 = Instant::now();
        c.autoplay_tidy(t0);
        assert!(c.autoplay.pour.is_some());
        // Nothing comes back: no refusal, no new counts.
        c.autoplay_tidy(t0 + crate::pack::POUR_LOST);
        assert!(c.autoplay.pour.is_none(), "given up as lost");
    }

    #[test]
    fn a_quiet_ground_is_read_off_the_world_and_not_off_the_status_line() {
        // The roam clock counted only the frames where the engine had
        // found nothing to do, and put itself back to nothing the moment
        // anything ran. Asking a corpse that would not open is something
        // running, so nine characters queueing at one body restarted it
        // every couple of seconds: in ten minutes it never once reached
        // its minute, and they worked a 40 x 32 m corner of a
        // 197 x 192 m field.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 84.0, 94.0)) else {
            return;
        };
        let s = Duration::from_secs;
        let now = Instant::now();
        c.world.stats.level = 20;

        // Nothing about: the clock starts.
        c.autoplay_watch_the_ground(now);
        assert_eq!(c.autoplay.growth.quiet_since, Some(now));

        // And keeps running through everything the character does
        // meanwhile. This is the whole of the fix.
        c.autoplay
            .say(crate::autoplay::Doing::Looting, "opening a corpse");
        c.autoplay_watch_the_ground(now + s(5));
        assert_eq!(
            c.autoplay.growth.quiet_since,
            Some(now),
            "a corpse put the clock back to nothing"
        );

        // Something worth fighting inside the radius stops it.
        let guid = 0x8000_0001;
        let me = c.player.as_ref().unwrap().world_position();
        c.world.objects.insert(
            guid,
            ac_world::WorldObject {
                guid,
                weenie_class_id: 8592,
                name: "Revenant".into(),
                item_type: ac_world::item_type::CREATURE,
                object_desc_flags: ac_world::object_desc_flags::ATTACKABLE,
                health: Some(1.0),
                position: Some(ac_world::object::Position::new_flat(
                    holtburg,
                    me + glam::Vec3::new(5.0, 0.0, 0.0) - ac_world::landblock_origin(holtburg),
                )),
                ..Default::default()
            },
        );
        c.autoplay_watch_the_ground(now + s(6));
        assert_eq!(c.autoplay.growth.quiet_since, None);

        // A dead one is no fight, and the clock starts again from there.
        c.world.objects.get_mut(&guid).unwrap().health = Some(0.0);
        c.autoplay_watch_the_ground(now + s(7));
        assert_eq!(c.autoplay.growth.quiet_since, Some(now + s(7)));
    }

    #[test]
    fn a_body_is_noted_when_it_appears_and_not_when_the_looting_gets_round_to_it() {
        // The bodies used to be noted inside the looting. A character
        // the claim tie-break tells to stand off scores that body
        // nothing, so its loot goal is never run, so it never notes the
        // body -- and a body it never noted is for ever "newly fallen",
        // which is exactly when the tie-break governs. One mate that
        // could not loot locked every body near it away from the other
        // eight for good.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 84.0, 94.0)) else {
            return;
        };
        let me = c.player.as_ref().unwrap().world_position();
        let s = Duration::from_secs;
        let now = Instant::now();
        let body = 0x8000_0001;
        c.world.objects.insert(
            body,
            ac_world::WorldObject {
                guid: body,
                name: "Corpse of Drudge Slave".into(),
                object_desc_flags: object_desc_flags::CORPSE,
                position: Some(ac_world::object::Position::new_flat(
                    holtburg,
                    me - ac_world::landblock_origin(holtburg),
                )),
                ..Default::default()
            },
        );
        // No loot profile, so nothing this character does will ever
        // reach past the first line of the looting step.
        c.autoplay.config.loot.profile = String::new();
        c.autoplay_watch_the_ground(now);
        assert_eq!(
            c.autoplay.corpse_seen,
            vec![(body, now)],
            "a body nobody looted was never noted"
        );
        // Noted once, not restamped every tick: the age is what the
        // tie-break and the rotting order both read.
        c.autoplay_watch_the_ground(now + s(5));
        assert_eq!(c.autoplay.corpse_seen, vec![(body, now)]);
        // Forgotten once emptied, so the list stays the size of what is
        // on the ground.
        c.autoplay.looted.push(body, now + s(5));
        c.autoplay_watch_the_ground(now + s(6));
        assert!(c.autoplay.corpse_seen.is_empty());
    }

    #[test]
    fn a_hunting_area_is_walked_about_sooner_than_a_whole_new_ground_is_chosen() {
        // Moving on inside an area costs only the walk; moving on without
        // one means picking a whole new ground and travelling to it,
        // which is worth being slow about.
        let mut cfg = Growth::default();
        assert_eq!(quiet_before_move(&cfg, false), cfg.idle_before_move);
        assert_eq!(quiet_before_move(&cfg, true), PATROL_AFTER);
        assert!(PATROL_AFTER < cfg.idle_before_move);
        // A player who asked for less still gets less.
        cfg.idle_before_move = 4.0;
        assert_eq!(quiet_before_move(&cfg, true), 4.0);
        assert_eq!(quiet_before_move(&cfg, false), 4.0);
    }

    #[test]
    fn only_the_one_leading_walks_the_area_looking_for_a_fight() {
        // Nine characters each picking their own corner of an outline
        // scatter the party across two hundred metres instead of moving
        // it. The followers keep to their leader and it takes them.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        let me = c.player.as_ref().unwrap().world_position();
        let now = Instant::now();
        let cfg = Growth::default();
        c.world.stats.level = 20;
        c.autoplay.config.fight.area = Some(crate::hunt::HuntArea {
            name: "test".into(),
            shape: crate::hunt::Shape::Outline {
                points: vec![
                    [me.x - 40.0, me.y - 5.0],
                    [me.x + 40.0, me.y - 5.0],
                    [me.x + 40.0, me.y + 60.0],
                    [me.x - 40.0, me.y + 60.0],
                ],
            },
        });
        let team = &mut c.autoplay.config.team;
        team.enabled = true;
        team.follow = true;
        team.lead = false;
        c.autoplay.team.mates = vec![crate::autoplay::Mate {
            name: "Leader".into(),
            leader: true,
            leads: true,
            world: me,
            cell: holtburg,
            ..Default::default()
        }];
        // Quiet for long enough that the patrol would go.
        c.autoplay.growth.quiet_since = Some(now - Duration::from_secs(60));
        assert!(!c.grow_hunt(now, &cfg), "a follower wandered off");
        assert!(!c.traveling());

        // The one leading walks it.
        c.autoplay.team.mates.clear();
        c.autoplay.team.leader = true;
        assert!(c.grow_hunt(now, &cfg), "the leader stayed put");
        assert!(c.traveling());
    }

    #[test]
    fn a_follower_with_no_hunting_area_does_not_go_looking_for_a_ground_of_its_own() {
        // The guard that keeps a follower from walking an area of its
        // own covered the roam as well, but not the tail below it: a
        // party with no area configured had its followers fail the roam
        // on the first quiet minute and fall straight through to picking
        // a landblock and travelling to it -- which is the scattering
        // the guard was added to stop, only sooner than before.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        let me = c.player.as_ref().unwrap().world_position();
        let now = Instant::now();
        let cfg = Growth::default();
        c.world.stats.level = 20;
        let team = &mut c.autoplay.config.team;
        team.enabled = true;
        team.follow = true;
        team.lead = false;
        // A leader with no ground of its own to take, so nothing below
        // the roam can be following anybody.
        c.autoplay.team.mates = vec![crate::autoplay::Mate {
            name: "Leader".into(),
            leader: true,
            leads: true,
            world: me,
            cell: holtburg,
            ..Default::default()
        }];
        // Standing on the ground it is hunting, quiet long enough to
        // move on, and past its roams so the roam itself is refused.
        c.autoplay.growth.hunting_at = Some(holtburg >> 16);
        c.autoplay.growth.roams = ROAMS;
        c.autoplay.growth.quiet_since = Some(now - Duration::from_secs(600));
        assert!(!c.grow_hunt(now, &cfg), "a follower went hunting alone");
        assert!(!c.traveling());
        assert_eq!(
            c.autoplay.growth.bound, None,
            "bound for a ground of its own"
        );

        // The one leading still moves the party on.
        c.autoplay.team.mates.clear();
        c.autoplay.team.leader = true;
        assert!(c.grow_hunt(now, &cfg), "the leader stayed put");
    }

    #[test]
    fn a_walk_to_a_ground_broken_off_by_a_corpse_is_walked_on() {
        // A body on the road -- a fellow's kill, shared -- took the
        // character off its walk to the ground, and walking to a corpse
        // ends a journey. Once the body was looted the hunting step found
        // no journey under way, took that for a walk that could not get
        // there, put the ground on the skip list and set off for another.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        c.world.player_guid = Some(0x5000_0001);
        let me = c.player.as_ref().unwrap().world_position();
        let now = Instant::now();
        let cfg = Growth::default();
        let at = Vec2::new(me.x + 250.0, me.y);
        let lb = (((at.x / 192.0) as u32) << 8) | (at.y / 192.0) as u32;
        assert!(c.grow_travel(at, now), "no way to the ground");
        c.autoplay.growth.bound = Some((lb, at, "Drudge".into()));
        c.autoplay.growth.bound_since = Some(now);

        c.interrupt_travel("walking to a corpse");
        assert!(c.grow_hunt(now, &cfg), "gave the walk up after a corpse");
        assert!(c.traveling(), "and did not walk on");
        assert!(c.autoplay.growth.bound.is_some());
        assert!(c.autoplay.growth.skip.is_empty(), "the ground was skipped");

        // Broken off again as soon as it is planned, it waits a moment
        // before planning the walk once more.
        c.interrupt_travel("walking to a corpse");
        assert!(c.grow_hunt(now + Duration::from_secs(1), &cfg));
        assert!(!c.traveling(), "planned again straight away");
        assert!(c.grow_hunt(now + WALK_ON_EVERY, &cfg));
        assert!(c.traveling());

        // A walk that ended by itself short of the ground still could not
        // get there.
        c.cancel_travel();
        assert!(!c.grow_hunt(now + WALK_ON_EVERY, &cfg));
        assert_eq!(c.autoplay.growth.bound, None);
        assert!(c.autoplay.growth.skip.iter().any(|(g, _)| *g == lb));
    }

    /// A Revenant called `name` standing `metres` east of the character:
    /// a real fight at level 20, not a critter walked past for its own
    /// sake.
    fn standing_by(c: &mut Client, guid: u32, name: &str, metres: f32) -> ac_world::WorldObject {
        let pl = c.player.as_ref().unwrap();
        let (cell, me) = (pl.cell, pl.world_position());
        let o = ac_world::WorldObject {
            guid,
            weenie_class_id: 8592,
            name: name.into(),
            item_type: ac_world::item_type::CREATURE,
            object_desc_flags: ac_world::object_desc_flags::ATTACKABLE,
            health: Some(1.0),
            position: Some(ac_world::object::Position::new_flat(
                cell,
                me + glam::Vec3::new(metres, 0.0, 0.0) - ac_world::landblock_origin(cell),
            )),
            ..Default::default()
        };
        c.world.objects.insert(guid, o.clone());
        o
    }

    #[test]
    fn what_stands_on_the_road_is_walked_past_and_what_swings_at_us_is_not() {
        // "Traveling to the hunting ground shouldn't have much fighting,
        // more ignoring the monsters on the way so you can get to the
        // hunting ground" -- the player, and the reason for the rule.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        c.world.player_guid = Some(0x5000_0001);
        let me = c.player.as_ref().unwrap().world_position();
        let now = Instant::now();
        let fight = c.autoplay.config.fight.clone();
        let it = standing_by(&mut c, 0x8000_0001, "Revenant", 5.0);

        // Standing on its ground with nowhere to be: a fight.
        assert!(!c.passing_by(&it, &fight));
        assert!(c.would_fight(&it, &fight, false, now));

        // Bound for a hunting ground and walking there: walked past, and
        // the fight rules have nothing to pick.
        let ground = (0xA9B2, Vec2::new(32_580.0, 34_570.0), "Drudge".to_string());
        assert!(c.grow_travel(Vec2::new(me.x + 250.0, me.y), now));
        c.autoplay.growth.bound = Some(ground.clone());
        assert!(c.passing_by(&it, &fight));
        assert!(!c.would_fight(&it, &fight, false, now));

        // Unless it swings: the road does not get to decide that.
        c.autoplay.attacked_by("Revenant", now);
        assert!(!c.passing_by(&it, &fight));
        assert!(c.would_fight(&it, &fight, false, now));
        // Which says nothing about the one standing next to it.
        let other = standing_by(&mut c, 0x8000_0002, "Drudge Skulker", 6.0);
        assert!(c.passing_by(&other, &fight));
        c.autoplay.last_hit_us = None;
        c.autoplay.hit_by.clear();

        // Named in "only these", it is still the road. The list says which
        // kind to hunt at the far end, and every creature the fight could
        // pick already matches it, so as an override it switched the rule
        // off for anyone who kept one.
        let only = crate::autoplay::Fight {
            only: vec!["revenant".into()],
            ..fight.clone()
        };
        assert!(c.passing_by(&it, &only));
        assert!(!c.would_fight(&it, &only, false, now));

        // A creature of ours on it is no reason to stop either. A summoned
        // creature picks its own fights, the nearest monster it can see,
        // and following its lead stopped the character for each in turn.
        c.world.objects.insert(
            0x8000_0003,
            ac_world::WorldObject {
                guid: 0x8000_0003,
                name: "Fire Elemental".into(),
                item_type: ac_world::item_type::CREATURE,
                pet_owner: 0x5000_0001,
                walked_at: Some(it.guid),
                ..Default::default()
            },
        );
        assert!(c.passing_by(&it, &fight), "its pet went for it");
        assert!(!c.would_fight(&it, &fight, false, now));
        c.world.objects.remove(&0x8000_0003);

        // With the setting off, everything on the road is fought again.
        let all = crate::autoplay::Fight {
            walk_past_on_the_way: false,
            ..fight.clone()
        };
        assert!(!c.passing_by(&it, &all));
        assert!(c.would_fight(&it, &all, false, now));

        // A town run is the same errand: the counters are the point of
        // it, not whatever stands between here and them.
        c.autoplay.growth.bound = None;
        c.autoplay.growth.run = Some(run_to(Vec2::new(32_500.0, 34_500.0), now));
        assert!(c.passing_by(&it, &fight));

        // The errand ending does not end the walking past: the journey
        // still under way is a road whoever planned it (see
        // `a_road_is_a_road_whoever_planned_it`). The journey ending does.
        c.autoplay.growth.run = None;
        assert!(c.passing_by(&it, &fight), "the journey is still under way");
        c.cancel_travel();
        assert!(!c.passing_by(&it, &fight), "nowhere to be again");
        assert!(c.would_fight(&it, &fight, false, now));
    }

    #[test]
    fn an_errand_is_on_its_way_only_while_its_walk_is() {
        // The errand outlives its walk. `bound` is let go only when the
        // hunting step next looks, and a party restocking does not call
        // it: a leader home from town stood on its own ground walking past
        // everything that had not swung at it until the slowest of the
        // party had shopped. And a body at its feet kept the grow step
        // from running at all, so the body beat the monster beside it.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        c.world.player_guid = Some(0x5000_0001);
        let me = c.player.as_ref().unwrap().world_position();
        let now = Instant::now();
        let fight = c.autoplay.config.fight.clone();
        let it = standing_by(&mut c, 0x8000_0001, "Revenant", 5.0);
        let at = Vec2::new(me.x + 250.0, me.y);
        let lb = (((at.x / 192.0) as u32) << 8) | (at.y / 192.0) as u32;

        // Bound, and walking there.
        assert!(c.grow_travel(at, now), "no way to the ground");
        c.autoplay.growth.bound = Some((lb, at, "Drudge".into()));
        assert!(c.on_its_way());
        assert!(!c.would_fight(&it, &fight, false, now));

        // A body on the road breaks the walk off, and it is still the walk.
        c.interrupt_travel("walking to a corpse");
        assert!(c.on_its_way(), "a walk broken off is still the walk");
        assert!(!c.would_fight(&it, &fight, false, now));

        // Arrived: fighting again that tick, whatever `bound` still says.
        assert!(c.grow_travel(at, now));
        c.end_trip();
        assert!(c.autoplay.growth.bound.is_some(), "not noticed yet");
        assert!(!c.on_its_way(), "arrived, and still walking past");
        assert!(c.would_fight(&it, &fight, false, now));

        // Cancelled -- the player took the keys -- is no walk either.
        assert!(c.grow_travel(at, now));
        c.cancel_travel();
        assert!(!c.on_its_way());

        // Stepping out of a shop first is the start of the walk.
        c.autoplay.growth.after_out = Some(at);
        assert!(c.on_its_way());
    }

    #[test]
    fn a_party_on_the_road_walks_past_together_and_stops_together() {
        // The leader, bound for a new ground, walked past a Drudge. Its
        // followers keep up through the follow step, which sets no errand,
        // so they stopped and fought it; the leader walked on, they were
        // fetched after it and turned on the Drudge again each time they
        // closed, and the leader never helped.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        c.world.player_guid = Some(0x5000_0001);
        let me = c.player.as_ref().unwrap().world_position();
        let now = Instant::now();
        let fight = c.autoplay.config.fight.clone();
        let drudge = standing_by(&mut c, 0x8000_0001, "Drudge Skulker", 5.0);
        let other = standing_by(&mut c, 0x8000_0002, "Drudge Slinker", 6.0);
        let team = &mut c.autoplay.config.team;
        team.enabled = true;
        team.follow = true;
        team.lead = false;
        let leader = |on_its_way: bool, target: Option<u32>| crate::autoplay::Mate {
            name: "Leader".into(),
            leader: true,
            leads: true,
            world: me,
            cell: holtburg,
            on_its_way,
            target,
            ..Default::default()
        };

        // A leader going nowhere: its follower fights what is about.
        c.autoplay.team.mates = vec![leader(false, None)];
        assert!(!c.on_its_way());
        assert!(c.would_fight(&drudge, &fight, false, now));

        // A leader on its way: so is the follower keeping up with it.
        c.autoplay.team.mates = vec![leader(true, None)];
        assert!(c.on_its_way(), "a follower is on its leader's way");
        assert!(!c.would_fight(&drudge, &fight, false, now));
        assert!(!c.joins_the_team_on(drudge.guid, &fight));

        // Something attacks the leader on the road and it turns to fight:
        // the follower turns with it and joins on it, but not on the one
        // standing by.
        c.autoplay.team.mates = vec![leader(true, Some(drudge.guid))];
        assert!(c.would_fight(&drudge, &fight, false, now));
        assert!(c.joins_the_team_on(drudge.guid, &fight));
        assert!(!c.would_fight(&other, &fight, false, now));
        assert!(!c.joins_the_team_on(other.guid, &fight));

        // On a run of its own while the party back at the ground fights:
        // that fight is not the road's, and neither focus fire nor the
        // debuffer turns the character round for it.
        c.autoplay.config.team.follow = false;
        let counter = Vec2::new(me.x + 250.0, me.y);
        assert!(c.grow_travel(counter, now));
        c.autoplay.growth.run = Some(run_to(counter, now));
        c.autoplay.team.mates = vec![leader(false, Some(drudge.guid))];
        assert!(c.on_its_way());
        assert!(
            !c.joins_the_team_on(drudge.guid, &fight),
            "turned back for the party"
        );
        // Until the Drudge attacks the character itself.
        c.autoplay.attacked_by("Drudge Skulker", Instant::now());
        assert!(c.joins_the_team_on(drudge.guid, &fight));
    }

    #[test]
    fn a_road_is_under_way_unless_it_is_a_walk_about_the_ground() {
        // No journey, nothing remembered: not on a road.
        assert!(!road_under_way(None, None));
        // A journey to somewhere else, or one put down for a fight.
        assert!(road_under_way(Some(false), None));
        assert!(road_under_way(None, Some(false)));
        // A roam, a patrol: about the ground, so never a road.
        assert!(!road_under_way(Some(true), None));
        assert!(!road_under_way(None, Some(true)));
    }

    #[test]
    fn a_road_is_a_road_whoever_planned_it() {
        // The walk past the road read only the growth rules' errands, so
        // a journey a script asked for was not "on its way": a party sent
        // down the Singularity Caul by script fought everything between
        // the drop and the far end, fourteen fights on a walk of forty
        // seconds.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        c.world.player_guid = Some(0x5000_0001);
        let me = c.player.as_ref().unwrap().world_position();
        let now = Instant::now();
        let fight = c.autoplay.config.fight.clone();
        let it = standing_by(&mut c, 0x8000_0001, "Revenant", 5.0);
        let at = Vec2::new(me.x + 250.0, me.y);
        assert!(c.autoplay.growth.bound.is_none() && c.autoplay.growth.run.is_none());

        // A script's journey: on its way, and walking past.
        assert!(c.travel_to(at), "no way there");
        assert!(c.on_its_way());
        assert!(!c.would_fight(&it, &fight, false, now));

        // Put down for a fight that came to it: still the road, and the
        // road again once the fight is over.
        c.remember_journey();
        c.interrupt_travel("attacking");
        assert!(!c.traveling());
        assert!(
            c.on_its_way(),
            "a road put down for a fight is still the road"
        );
        assert!(c.autoplay_resume_journey());
        assert!(c.traveling() && c.on_its_way());

        // Cancelled -- the player took the keys -- is no road.
        c.cancel_travel();
        assert!(!c.on_its_way());
        assert!(c.would_fight(&it, &fight, false, now));

        // A roam about the ground is not a road, and is not one after a
        // fight has put it down either: what stands about the ground is
        // what the character came for.
        assert!(c.travel_about(at));
        assert!(!c.on_its_way(), "a roam walked past the ground");
        assert!(c.would_fight(&it, &fight, false, now));
        c.remember_journey();
        c.interrupt_travel("attacking");
        assert!(!c.on_its_way());
        assert!(c.autoplay_resume_journey());
        assert!(c.traveling());
        assert!(!c.on_its_way(), "a roam picked up again became a road");

        // A road put down for a body on it is remembered like one put
        // down for a fight, since nothing else would bring it back.
        c.cancel_travel();
        assert!(c.travel_to(at));
        c.autoplay.resume_trip = None;
        let spot = glam::Vec3::new(me.x + 30.0, me.y, me.z);
        c.walk_to_corpse(0x8000_0002, "Corpse of Drudge", spot, 30.0, now);
        assert!(!c.traveling());
        assert_eq!(c.autoplay.resume_trip, Some(at));
        assert!(c.on_its_way(), "a road put down for a body was lost");
    }

    #[test]
    fn an_errand_whose_setting_is_turned_off_is_let_go() {
        // Nothing carries a run on with town runs turned off, nor a walk
        // to a ground with grounds off, so neither was ever let go. The
        // errand read as under way for the rest of the session.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        c.world.player_guid = Some(0x5000_0001);
        let now = Instant::now();
        let to = Vec2::new(32_500.0, 34_500.0);
        let growth = &mut c.autoplay.config.growth;
        growth.auto_xp = false;
        growth.town_runs = false;
        growth.hunt_grounds = false;
        c.autoplay.growth.run = Some(run_to(to, now));
        c.autoplay.growth.bound = Some((0xA9B2, to, "Drudge".into()));
        c.autoplay_grow(now);
        assert!(!c.autoplay.growth.town_run_under_way(), "the run was kept");
        assert_eq!(c.autoplay.growth.bound, None, "the walk was kept");
    }

    #[test]
    fn a_run_asked_for_from_the_panel_chooses_a_counter_and_sets_off() {
        // The panel's Step and Run once put the shopping rules straight
        // to the character where it stood. With no window open the
        // rules answered "no counter" and called the trip done, so the
        // buttons did nothing -- and having called it done, went on
        // doing nothing once a window was opened. Now they start the
        // run autoplay would start: a counter is chosen and the walk
        // to it begins, with autoplay off and none of its throttles.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        c.world.player_guid = Some(0x5000_0001);
        let now = Instant::now();
        assert!(!c.autoplay.config.enabled);
        assert_eq!(c.town_run_driver(), None);
        c.town_run_by_hand(now).expect("no run was started");
        assert_eq!(c.town_run_driver(), Some(Driver::Hand));
        assert!(c.traveling(), "chose a counter and did not set off");
        let v = c.town_run_view(now).expect("no run to show");
        assert_eq!(v.phase, "Going");
        assert_eq!(v.driver, Driver::Hand);
        assert_eq!(v.next, None, "nothing is decided on the walk");
        // The walk is a wait, and a Step waits it out.
        assert_eq!(c.town_run_step_by_hand(now), Turn::Waited);
        assert!(c.traveling());
        // Pressed again, the run under way is the one shown; a second
        // is not started.
        c.town_run_by_hand(now).unwrap();
        assert_eq!(c.town_run_view(now).unwrap().vendor, v.vendor);
        assert_eq!(c.autoplay.growth.run.as_ref().unwrap().since, now);

        // Autoplay turned on meanwhile leaves the run to the panel --
        // it neither steps it nor, with town runs off, lets it go --
        // but counts the character as busy with it.
        c.autoplay.config.enabled = true;
        c.autoplay.config.growth.town_runs = false;
        assert_eq!(c.town_run_driver(), Some(Driver::Hand));
        c.autoplay_grow(now);
        assert!(
            c.autoplay.growth.town_run_under_way(),
            "let go for town runs being off"
        );
        c.autoplay.config.growth.town_runs = true;
        assert!(c.autoplay_grow(now), "the run did not keep the tick");
        assert_eq!(c.town_run_driver(), Some(Driver::Hand));
        assert!(c.traveling(), "autoplay stepped the panel's run");

        // Stop leaves the character as it stands: no run, no walk,
        // nothing to walk up to, no window.
        c.town_run_stop(now);
        assert_eq!(c.town_run_driver(), None);
        assert!(!c.traveling());
        assert!(c.follow.is_none());
        assert!(c.world.open_vendor.is_none());
        assert_eq!(c.autoplay.growth.shop.phase, ac_vendor::Phase::Walking);
        // And autoplay, now running town runs, does not set straight
        // off on one of its own.
        assert!(!c.autoplay_grow(now), "autoplay started a run at once");
        assert_eq!(c.town_run_driver(), None);
    }

    #[test]
    fn a_run_started_under_autoplay_is_autoplays_to_step() {
        // With autoplay on and running town runs, Run from the panel
        // starts the run now, and autoplay's tick takes it from there:
        // the panel shows it and does not step it too.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        c.world.player_guid = Some(0x5000_0001);
        c.autoplay.config.enabled = true;
        let now = Instant::now();
        c.town_run_by_hand(now).expect("no run was started");
        assert_eq!(c.town_run_driver(), Some(Driver::Autoplay));
        assert!(c.autoplay_grow(now));
        assert!(c.traveling());
        // Until the panel steps it itself, which makes it the panel's.
        assert_eq!(c.town_run_step_by_hand(now), Turn::Waited);
        assert_eq!(c.town_run_driver(), Some(Driver::Hand));
    }

    #[test]
    fn the_shopping_rules_start_every_counter_fresh() {
        // The rules remember what a trip offered, what was refused and
        // whether it is done, and were made afresh only when a trip
        // closed its counter. Asked once with no window open they call
        // the trip done, and a run dropped part-way leaves them where
        // they were; either way the next counter opened was closed at
        // once, with nothing sold. Now they are started fresh as each
        // counter's selling begins.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.player_guid = Some(0x5000_0001);
        let me = c.player.as_ref().unwrap().world_position();
        let now = Instant::now();
        let cfg = c.autoplay.config.growth.clone();
        // Done, and with a count from some earlier counter.
        let snap = c.vendor_snapshot(&cfg);
        c.autoplay.growth.shop.step(&snap, now);
        assert_eq!(c.autoplay.growth.shop.phase, ac_vendor::Phase::Done);
        c.autoplay.growth.shop.sold = 7;

        // A run at the counter with the pack looked over. Nothing goes
        // out on the way from there to selling, so a Step goes on
        // through it to the first act of the selling.
        let mut run = run_to(Vec2::new(me.x, me.y), now);
        run.phase = Phase::Appraising;
        run.since = now - SETTLE * 3;
        c.autoplay.growth.run = Some(run);
        assert_eq!(c.grow_run_step(now, &cfg), Turn::Waited);
        assert!(matches!(
            c.autoplay.growth.run.as_ref().unwrap().phase,
            Phase::Selling { .. }
        ));
        assert_eq!(
            c.autoplay.growth.shop.phase,
            ac_vendor::Phase::Walking,
            "the rules were not started fresh"
        );
        assert_eq!(c.autoplay.growth.shop.sold, 0);
        assert_eq!(c.town_run_view(now).unwrap().sold, 0);

        // Waiting for a window is a wait; asking again after the
        // counter's time is up is an act.
        let mut run = run_to(Vec2::new(me.x, me.y), now);
        run.phase = Phase::Opening {
            guid: 0x8000_0001,
            tries: 1,
            busy: None,
            asked_over: 0,
        };
        c.autoplay.growth.run = Some(run);
        assert_eq!(c.grow_run_step(now, &cfg), Turn::Waited);
        assert_eq!(c.town_run_view(now).unwrap().phase, "Opening");
        let later = now + VENDOR_OPEN_TIMEOUT + Duration::from_secs(1);
        assert_eq!(c.grow_run_step(later, &cfg), Turn::Acted);
        assert!(matches!(
            c.autoplay.growth.run.as_ref().unwrap().phase,
            Phase::Opening { tries: 2, .. }
        ));
    }

    /// A vendor's window as the server sends it, with nothing on the
    /// shelf.
    fn window_of(vendor: u32) -> ac_world::object::ApproachVendor {
        ac_world::object::ApproachVendor {
            vendor,
            item_types: 0,
            min_value: 0,
            max_value: 0,
            magical: false,
            buy_rate: 1.0,
            sell_rate: 1.0,
            alt_currency: 0,
            alt_amount: 0,
            alt_name: String::new(),
            items: Vec::new(),
        }
    }

    /// A vendor standing `off` from the character, in view.
    fn vendor_beside(c: &mut Client, guid: u32, name: &str, off: glam::Vec3) {
        let holtburg = 0xA9B4_0019;
        let me = c.player.as_ref().unwrap().world_position();
        c.world.objects.insert(
            guid,
            ac_world::WorldObject {
                guid,
                name: name.into(),
                item_type: ac_world::item_type::CREATURE,
                object_desc_flags: object_desc_flags::VENDOR,
                position: Some(ac_world::object::Position::new_flat(
                    holtburg,
                    me + off - ac_world::landblock_origin(holtburg),
                )),
                ..Default::default()
            },
        );
    }

    #[test]
    fn a_hand_run_left_held_is_autoplays_again_once_it_runs_town_runs() {
        // Step once with autoplay off, close the panel, turn autoplay
        // on: the run was the panel's for good, autoplay claimed every
        // tick for it and stepped nothing, and the character stood on
        // the road with a blank status line. A run nothing has stepped
        // for a moment is autoplay's to carry on, when it runs town
        // runs.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        c.world.player_guid = Some(0x5000_0001);
        let now = Instant::now();
        c.town_run_by_hand(now).expect("no run was started");
        assert_eq!(c.town_run_driver(), Some(Driver::Hand));
        c.autoplay.config.enabled = true;
        c.autoplay.config.growth.town_runs = true;
        // Just pressed: still the panel's, and not stepped by autoplay.
        assert!(c.autoplay_grow(now));
        assert_eq!(c.town_run_driver(), Some(Driver::Hand));
        // Left alone: autoplay's, and stepped on.
        let later = now + HAND_HOLD;
        assert!(c.autoplay_grow(later));
        assert_eq!(c.town_run_driver(), Some(Driver::Autoplay));
        assert!(c.autoplay.growth.town_run_under_way());
        assert!(c.traveling());
        // The panel stepping it again takes it back, for as long as it
        // keeps stepping.
        assert_eq!(c.town_run_step_by_hand(later), Turn::Waited);
        assert_eq!(c.town_run_driver(), Some(Driver::Hand));
        assert!(c.autoplay_grow(later));
        assert_eq!(c.town_run_driver(), Some(Driver::Hand));
    }

    #[test]
    fn a_hand_run_with_town_runs_off_keeps_the_hunting_off_the_counter() {
        // Autoplay on with town runs off is the panel's to drive, and
        // the run is kept for it -- but with town runs off the run was
        // never looked at by autoplay's tick, so nothing claimed the
        // tick for it and the hunting, finding nothing in sight, walked
        // the character off the open window to look about the ground.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        c.world.player_guid = Some(0x5000_0001);
        let me = c.player.as_ref().unwrap().world_position();
        let now = Instant::now();
        c.autoplay.config.enabled = true;
        c.autoplay.config.growth.town_runs = false;
        c.autoplay.config.growth.hunt_grounds = true;
        c.autoplay.config.growth.tactic = ac_world::hunting::Tactic::Patrol;
        // On its hunting ground, quiet long enough to move on.
        c.autoplay.growth.hunting_at = Some(holtburg >> 16);
        c.autoplay.growth.quiet_since = Some(now - Duration::from_secs(600));
        // At the counter, waiting on its window, held by the panel.
        let mut run = run_to(Vec2::new(me.x, me.y), now);
        run.phase = Phase::Opening {
            guid: 0x8000_0001,
            tries: 1,
            busy: None,
            asked_over: 0,
        };
        c.autoplay.growth.run = Some(run);
        c.autoplay.growth.by_hand = true;
        assert!(c.autoplay_grow(now), "the run did not keep the tick");
        assert!(!c.traveling(), "the hunting walked off the counter");
        assert_eq!(c.autoplay.growth.bound, None);
        assert!(c.autoplay.growth.town_run_under_way());
        assert!(
            c.autoplay.status.contains("held by the vendoring panel"),
            "nothing said why the character stands still: {:?}",
            c.autoplay.status
        );
        // Without the run the same tick goes looking about the ground,
        // which is what the run was keeping it from.
        c.autoplay.growth.by_hand = false;
        assert!(c.autoplay_grow(now));
        assert!(
            !c.autoplay.growth.town_run_under_way(),
            "kept with town runs off"
        );
        assert!(c.traveling(), "the hunting did not move");
    }

    #[test]
    fn run_pressed_at_a_counter_the_player_opened_sells_there() {
        // The panel shows what the rules would do at a window the
        // player opened by hand; Run then chose a counter of its own
        // and walked away from the one it had just been showing. At an
        // open window within reach the run is made there, and its
        // first turn is the appraising.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        c.world.player_guid = Some(0x5000_0001);
        let now = Instant::now();
        let rakk = 0x8000_0002;
        vendor_beside(
            &mut c,
            rakk,
            "Rakk the Peddler",
            glam::Vec3::new(1.0, 0.0, 0.0),
        );
        c.world.open_vendor = Some(window_of(rakk));
        c.town_run_by_hand(now).expect("no run was started");
        let v = c.town_run_view(now).expect("no run to show");
        assert_eq!(v.vendor, "Rakk the Peddler");
        assert_eq!(v.phase, "Opening");
        assert!(!c.traveling(), "walked off to another counter");
        assert_eq!(c.town_run_step_by_hand(now), Turn::Acted);
        assert_eq!(c.town_run_view(now).unwrap().phase, "Appraising");
        assert!(c.world.open_vendor.is_some(), "the window was closed");

        // A window left open from across the town is not a counter at
        // hand: the run chooses one and sets off, as it always did.
        c.town_run_stop(now);
        vendor_beside(
            &mut c,
            rakk,
            "Rakk the Peddler",
            glam::Vec3::new(60.0, 0.0, 0.0),
        );
        c.world.open_vendor = Some(window_of(rakk));
        c.town_run_by_hand(now).expect("no run was started");
        assert_eq!(c.town_run_view(now).unwrap().phase, "Going");
        assert!(c.traveling());
    }

    #[test]
    fn a_hand_run_is_refused_with_no_slot_for_the_money() {
        // Autoplay's own refusal lived only in its tick, so the panel's
        // Run walked a pack with no free slot to town, sold nothing
        // there, and came home futile -- which held autoplay's next run
        // back for the longer while.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        let me = 0x5000_0001;
        c.world.player_guid = Some(me);
        c.world.objects.insert(
            me,
            ac_world::WorldObject {
                guid: me,
                name: "Verity".into(),
                is_player: true,
                items_capacity: 1,
                ..Default::default()
            },
        );
        c.world.objects.insert(
            0x8000_0003,
            ac_world::WorldObject {
                guid: 0x8000_0003,
                name: "Dagger".into(),
                value: 50,
                container: Some(me),
                ..Default::default()
            },
        );
        assert_eq!(c.free_space(), 0);
        let now = Instant::now();
        assert_eq!(
            c.town_run_by_hand(now),
            Err("no free slot for a counter's money".into())
        );
        assert_eq!(c.town_run_driver(), None);
        assert!(!c.traveling());
    }

    #[test]
    fn a_window_that_opens_after_the_run_stopped_is_closed() {
        // Stop pressed while the counter was being asked for its window
        // left the window to arrive afterwards, and nothing closed it:
        // the tidying refused every pour for "a counter is open" and
        // the panel showed a counter open with nobody at it.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.player_guid = Some(0x5000_0001);
        let me = c.player.as_ref().unwrap().world_position();
        let now = Instant::now();
        let rakk = 0x8000_0002;
        let asking = |c: &mut Client| {
            let mut run = run_to(Vec2::new(me.x, me.y), now);
            run.phase = Phase::Opening {
                guid: rakk,
                tries: 1,
                busy: None,
                asked_over: 0,
            };
            c.autoplay.growth.run = Some(run);
        };
        asking(&mut c);
        c.town_run_stop(now);
        assert!(c.world.open_vendor.is_none());
        // The window comes a moment later, and goes.
        c.world.open_vendor = Some(window_of(rakk));
        c.autoplay_close_unwanted_window(now + Duration::from_secs(1));
        assert!(c.world.open_vendor.is_none(), "the window was left open");
        // Another counter's window is not the one that was asked for.
        asking(&mut c);
        c.town_run_stop(now);
        c.world.open_vendor = Some(window_of(0x8000_0009));
        c.autoplay_close_unwanted_window(now + Duration::from_secs(1));
        assert!(
            c.world.open_vendor.is_some(),
            "somebody else's window was closed"
        );
        c.world.open_vendor = None;
        // Long after the run would have given up on it, a window for
        // that counter is the player's own doing.
        asking(&mut c);
        c.town_run_stop(now);
        let late = now + VENDOR_OPEN_TIMEOUT + Duration::from_secs(1);
        c.autoplay_close_unwanted_window(late);
        c.world.open_vendor = Some(window_of(rakk));
        c.autoplay_close_unwanted_window(late);
        assert!(
            c.world.open_vendor.is_some(),
            "the player's window was closed"
        );
    }

    #[test]
    fn a_hand_runs_status_stays_with_autoplay_off() {
        // With autoplay off its tick cleared the status every frame,
        // and the run's next turn said it again: a log line, an event
        // and a bus post a frame for the length of the walk. The run
        // the panel is driving keeps its line.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.player_guid = Some(0x5000_0001);
        let me = c.player.as_ref().unwrap().world_position();
        let now = Instant::now();
        c.autoplay.config.enabled = false;
        c.autoplay.growth.run = Some(run_to(Vec2::new(me.x, me.y), now));
        c.autoplay.growth.by_hand = true;
        c.autoplay.say(Doing::Shopping, "going to the counter");
        let said = c.autoplay.announced.len();
        c.tick_autoplay(now);
        assert_eq!(
            c.autoplay.status, "going to the counter",
            "the status was cleared"
        );
        c.autoplay.say(Doing::Shopping, "going to the counter");
        assert_eq!(c.autoplay.announced.len(), said, "said again");
        // A run nobody is driving does not keep autoplay's line up.
        c.autoplay.growth.by_hand = false;
        c.tick_autoplay(now);
        assert!(c.autoplay.status.is_empty());
    }

    #[test]
    fn walking_the_hunting_ground_is_not_being_on_the_way_to_it() {
        // The trap this rule had to be kept out of. A patrol of the
        // hunting area, and the roam around a ground, both travel --
        // `traveling()` is true for a character doing the very thing it
        // came here for. Keyed on that, a character would have stood in
        // its own hunting ground refusing to fight. Both paths set off
        // and return before `bound` is ever set, and that is the whole
        // of the difference.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        c.world.player_guid = Some(0x5000_0001);
        let me = c.player.as_ref().unwrap().world_position();
        let now = Instant::now();
        let cfg = Growth::default();
        c.autoplay.config.fight.area = Some(crate::hunt::HuntArea {
            name: "test".into(),
            shape: crate::hunt::Shape::Outline {
                points: vec![
                    [me.x - 40.0, me.y - 5.0],
                    [me.x + 40.0, me.y - 5.0],
                    [me.x + 40.0, me.y + 60.0],
                    [me.x - 40.0, me.y + 60.0],
                ],
            },
        });
        let fight = c.autoplay.config.fight.clone();
        let it = standing_by(&mut c, 0x8000_0001, "Revenant", 5.0);

        // Quiet long enough to walk the area, and it walks it.
        c.autoplay.growth.quiet_since = Some(now - Duration::from_secs(600));
        assert!(c.grow_hunt(now, &cfg), "the patrol stayed put");
        assert!(c.traveling(), "the patrol is a journey like any other");
        assert_eq!(c.autoplay.growth.bound, None, "the patrol is not an errand");
        assert!(!c.on_its_way());

        // So the creature it walked up to is still a fight.
        assert!(!c.passing_by(&it, &fight));
        assert!(c.would_fight(&it, &fight, false, now));
    }

    #[test]
    fn growth_config_has_defaults_and_round_trips() {
        let g: Growth = serde_json::from_str("{}").unwrap();
        assert_eq!(g, Growth::default());
        assert!(g.auto_xp && g.hunt_grounds && g.town_runs);
        let text = serde_json::to_string(&g).unwrap();
        let back: Growth = serde_json::from_str(&text).unwrap();
        assert_eq!(back, g);
        let partial: Growth =
            serde_json::from_str(r#"{"auto_xp":false,"level_margin":3}"#).unwrap();
        assert!(!partial.auto_xp);
        assert_eq!(partial.level_margin, 3);
        assert!(partial.town_runs);
        // A config saved before the sale rules had thresholds reads
        // with the defaults, and the defaults are on.
        assert_eq!(partial.sell_run_value, 5_000);
        assert_eq!(partial.sell_run_count, 8);
        assert_eq!(partial.sell_run_patience, 15.0 * 60.0);
        let set: Growth =
            serde_json::from_str(r#"{"sell_run_value":0,"sell_run_count":3}"#).unwrap();
        assert_eq!(set.sell_run_value, 0);
        assert_eq!(set.sell_run_count, 3);
    }

    #[test]
    fn loot_for_a_counter_is_reason_enough_for_a_run() {
        // An Iron Pea and a Lead Pea, taken to sell, in a pack with room
        // to spare. The ledger said what they were for and nothing read
        // it: a run was made for a full pack, a heavy one or a supply
        // short, and the peas were carried about for ever.
        use ac_world::item_type::{ARMOR, GEM, SPELL_COMPONENTS};
        let cfg = Growth::default();
        let minutes = |m: u64| Some(Duration::from_secs(m * 60));
        let two_peas = [
            salable(1, SPELL_COMPONENTS, 2_500, 1),
            salable(2, SPELL_COMPONENTS, 500, 1),
        ];
        // Three thousand at face is not worth the walk on its own...
        assert_eq!(worth_a_sale_run(&two_peas, None, &cfg), None);
        assert_eq!(worth_a_sale_run(&two_peas, minutes(5), &cfg), None);
        // ...but it is not carried about all afternoon either.
        let why = worth_a_sale_run(&two_peas, minutes(15), &cfg).expect("a quarter of an hour");
        assert!(why.contains("carried 15 min"), "{why}");
        // Worth enough is reason at once, and a stack is worth the lot.
        let gem = [salable(3, GEM, 5_000, 1)];
        let why = worth_a_sale_run(&gem, None, &cfg).expect("five thousand");
        assert!(why.contains("5000 pyreals"), "{why}");
        let peas = [salable(4, SPELL_COMPONENTS, 10 * 500, 10)];
        assert!(worth_a_sale_run(&peas, None, &cfg).is_some());
        // So is an armful, however cheap: the slots are going.
        let junk: Vec<Salable> = (0..8).map(|i| salable(10 + i, ARMOR, 50, 1)).collect();
        let why = worth_a_sale_run(&junk, None, &cfg).expect("an armful");
        assert!(why.contains("8 things"), "{why}");
        assert_eq!(worth_a_sale_run(&junk[..7], None, &cfg), None);
        // The count is of stacks -- the slots going -- not of things: a
        // stack of eight cheap things is one, and seven singles and a
        // stack are eight.
        assert_eq!(
            worth_a_sale_run(&[salable(5, ARMOR, 400, 8)], None, &cfg),
            None
        );
        let mut seven_and_a_stack = junk[..7].to_vec();
        seven_and_a_stack.push(salable(20, ARMOR, 50, 10));
        assert!(worth_a_sale_run(&seven_and_a_stack, None, &cfg).is_some());
        // Nothing for a counter is no reason, however long since.
        assert_eq!(worth_a_sale_run(&[], minutes(60), &cfg), None);
        // Each rule is off at zero.
        let off = Growth {
            sell_run_value: 0,
            sell_run_count: 0,
            sell_run_patience: 0.0,
            ..cfg
        };
        assert_eq!(worth_a_sale_run(&junk, minutes(60), &off), None);
        assert_eq!(worth_a_sale_run(&gem, minutes(60), &off), None);
    }

    #[test]
    fn a_sale_is_ranked_on_what_the_counter_pays() {
        // A tailor with the shopping list on the shelf and no use for
        // the pack, and an archmage with the pack's worth in her purse
        // and nothing on the list. A trip to buy goes to the tailor; a
        // trip to sell goes to the archmage, and the tailor is not so
        // much as a candidate for it.
        let look = |takings: u32, selling: usize, stocks: usize| Forecast {
            takings,
            selling,
            stocks: vec!["something".into(); stocks],
            cheapest: if stocks > 0 { 10 } else { 0 },
            purse: 100,
            ..Default::default()
        };
        let tailor = look(0, 0, 2);
        let archmage = look(2_700, 2, 0);
        assert!(Errand::Buy.served_by(&tailor));
        assert!(!Errand::Sell.served_by(&tailor), "buys none of it");
        assert!(Errand::Sell.served_by(&archmage));
        assert_eq!(
            better_counter(Errand::Buy, (&tailor, 300.0), (&archmage, 20.0)),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            better_counter(Errand::Sell, (&archmage, 300.0), (&tailor, 20.0)),
            std::cmp::Ordering::Less
        );
        // Between two that buy: what they pay, then how much of the
        // pack they take, then the shelf, then the walk.
        let scriveners = look(500, 2, 0);
        let broker = look(950, 2, 0);
        assert_eq!(
            better_counter(Errand::Sell, (&broker, 380.0), (&scriveners, 80.0)),
            std::cmp::Ordering::Less
        );
        let takes_more = look(950, 3, 0);
        assert_eq!(
            better_counter(Errand::Sell, (&takes_more, 380.0), (&broker, 80.0)),
            std::cmp::Ordering::Less
        );
        let stocked_too = look(950, 3, 1);
        assert_eq!(
            better_counter(Errand::Sell, (&stocked_too, 380.0), (&takes_more, 80.0)),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            better_counter(Errand::Sell, (&broker, 80.0), (&broker, 380.0)),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn a_run_to_sell_goes_to_a_counter_that_buys_what_it_carries() {
        // Holtburg, with two peas in the pack. Fourteen counters stand
        // in the town and one of them, the archmage, buys spell
        // components; the shops that do so at a worse rate are a few
        // hundred metres out. Ranked the buying way the choice fell to
        // whoever had the most of the shopping list, and the peas went
        // to a counter that would not look at them.
        use ac_world::item_type::{LIFESTONE, SPELL_COMPONENTS};
        let from = ac_world::towns::find("Holtburg").unwrap().world_xy();
        let in_town = || {
            ac_world::shops::all()
                .iter()
                .filter(|s| s.gate.is_none())
                .map(|s| (s, s.xy().distance(from)))
                .filter(|(_, d)| *d <= VENDOR_RINGS[0])
        };
        let peas = [
            salable(1, SPELL_COMPONENTS, 2_500, 1),
            salable(2, SPELL_COMPONENTS, 500, 1),
        ];
        let (shop, look) =
            choose_counter(in_town(), Errand::Sell, &[], 0, &peas).expect("nobody buys peas");
        assert_eq!(shop.name, "Archmage Cindrue");
        assert_eq!(look.selling, 2);
        assert_eq!(look.takings, 2_250 + 450, "nine tenths of both");
        assert!(shop.pays_for(SPELL_COMPONENTS, 2_500).is_some());

        // With a healing kit on the list, the trip to buy goes to a
        // counter with kits -- the pawn shop out on the road, as it
        // happens, which also takes peas at eight tenths...
        let mut kits = need(NeedKind::Named("Healing Kit".into()), 1);
        kits.name = "Healing Kit".into();
        let wants = [(&kits, "healing kit".to_string())];
        let (to_buy, look) =
            choose_counter(in_town(), Errand::Buy, &wants, 10_000, &peas).expect("no kits");
        assert!(to_buy.stocks("Healing Kit").is_some());
        assert!(look.covers_it());
        assert_ne!(to_buy.name, "Archmage Cindrue");
        // ...while the trip to sell, list and all, goes where the peas
        // fetch most.
        let (to_sell, look) =
            choose_counter(in_town(), Errand::Sell, &wants, 10_000, &peas).unwrap();
        assert_eq!(to_sell.name, "Archmage Cindrue");
        assert!(look.takings > 2_400, "{}", look.takings);

        // Nothing in town buys a lifestone: no counter, and no falling
        // back on the nearest one.
        let odd = [salable(3, LIFESTONE, 1_000, 1)];
        assert!(choose_counter(in_town(), Errand::Sell, &[], 0, &odd).is_none());
        // Nothing wanted and nothing anyone buys is no trip to buy
        // either; the nearest-counter fallback is `pick_vendor`'s own.
        assert!(choose_counter(in_town(), Errand::Buy, &[], 0, &odd).is_none());
    }

    /// The character, with `capacity` slots in its main pack.
    fn with_a_pack(c: &mut Client, capacity: u32) {
        let me = 0x5000_0001;
        c.world.player_guid = Some(me);
        c.world.objects.insert(
            me,
            ac_world::WorldObject {
                guid: me,
                name: "Verity".into(),
                is_player: true,
                items_capacity: capacity,
                ..Default::default()
            },
        );
    }

    /// A pea in the pack, taken to sell.
    fn pea_in_the_pack(c: &mut Client, guid: u32, name: &str, wcid: u32, value: u32) {
        let me = c.world.player_guid.unwrap();
        c.world.objects.insert(
            guid,
            ac_world::WorldObject {
                guid,
                name: name.into(),
                weenie_class_id: wcid,
                item_type: item_type::SPELL_COMPONENTS,
                value,
                stack_size: 1,
                max_stack_size: 100,
                container: Some(me),
                ..Default::default()
            },
        );
        let stats = c.stats_of(guid).unwrap();
        c.autoplay.tag(&stats, LootAction::Sell);
    }

    /// A wand in hand, a bolt in the book and the Foci of Strife in the
    /// pack: a war mage, whose every cast burns a scarab and a prismatic
    /// taper.
    fn as_a_war_mage(c: &mut Client) {
        const FLAME_BOLT_I: u32 = 27;
        const FOCI_OF_STRIFE: u32 = 15271;
        let me = c.world.player_guid.unwrap();
        c.world.stats.spells = vec![FLAME_BOLT_I];
        c.world.objects.insert(
            0x8000_0040,
            ac_world::WorldObject {
                guid: 0x8000_0040,
                name: "Wand".into(),
                item_type: item_type::CASTER,
                value: 100,
                wielder: Some(me),
                parent: Some(me),
                ..Default::default()
            },
        );
        c.world.objects.insert(
            0x8000_0041,
            ac_world::WorldObject {
                guid: 0x8000_0041,
                name: "Foci of Strife".into(),
                weenie_class_id: FOCI_OF_STRIFE,
                value: 100,
                container: Some(me),
                ..Default::default()
            },
        );
    }

    /// `stack` prismatic tapers in the pack.
    fn tapers_in_the_pack(c: &mut Client, guid: u32, stack: u32) {
        let me = c.world.player_guid.unwrap();
        c.world.objects.insert(
            guid,
            ac_world::WorldObject {
                guid,
                name: "Prismatic Taper".into(),
                weenie_class_id: 20631,
                item_type: item_type::SPELL_COMPONENTS,
                value: stack,
                stack_size: stack,
                max_stack_size: 1_000,
                container: Some(me),
                ..Default::default()
            },
        );
    }

    #[test]
    fn a_casters_peas_are_loot_to_sell_and_not_stock_to_buy() {
        // The report: "autovendoring doesn't seem to sell at all", from
        // a caster with a spellbook and a wand. The peas it looted sit
        // in the component table beside the scarabs, and every carried
        // component went on the restock list at the taper count for
        // want of a burn rate: two peas became two wants for ninety-nine
        // more, the counter skipped them as what the character came to
        // buy, and a counter with peas on the shelf would have bought
        // them at markup. The character that passed a live proof knew
        // spells and wielded nothing, so this never opened for it.
        use ac_vendor::Act;
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        with_a_pack(&mut c, 20);
        as_a_war_mage(&mut c);
        pea_in_the_pack(&mut c, 0x8000_0010, "Iron Pea", 8328, 2_500);
        pea_in_the_pack(&mut c, 0x8000_0011, "Lead Pea", 8329, 500);
        tapers_in_the_pack(&mut c, 0x8000_0012, 78);
        with_a_buy_list(&mut c, &[("Prismatic Taper", 100, 25)]);
        let cfg = c.autoplay.config.growth.clone();

        // The gate is open: the tapers are stock, scaled from the line.
        let needs = c.grow_needs(&cfg);
        let taper = needs
            .iter()
            .find(|n| n.kind == NeedKind::Component(20631))
            .expect("the tapers it is short of");
        assert_eq!((taper.have, taper.keep, taper.want), (78, 100, 22));
        // And a pea is not: no cast burns one.
        let pea = needs.iter().find(|n| n.name.contains("Pea"));
        assert!(pea.is_none(), "a pea is stock: {pea:?}");
        let wants = c.vendor_shortfall(&cfg);
        assert!(
            wants.iter().all(|w| w.wcid != 8328 && w.wcid != 8329),
            "a pea on the shopping list: {wants:?}"
        );
        assert!(wants.iter().any(|w| w.wcid == 20631), "{wants:?}");

        // At a counter that buys components, the peas go and the
        // tapers stay.
        let cindrue = 0x8000_0002;
        vendor_beside(
            &mut c,
            cindrue,
            "Archmage Cindrue",
            glam::Vec3::new(1.0, 0.0, 0.0),
        );
        let mut window = window_of(cindrue);
        window.item_types = item_type::SPELL_COMPONENTS;
        c.world.open_vendor = Some(window);
        let snap = c.vendor_snapshot(&cfg);
        let mut offered: Vec<u32> = snap
            .items
            .iter()
            .filter(|i| snap.offers(i))
            .map(|i| i.guid)
            .collect();
        offered.sort_unstable();
        assert_eq!(offered, [0x8000_0010, 0x8000_0011]);
        assert!(
            snap.items
                .iter()
                .any(|i| i.guid == 0x8000_0010 && i.to_sell()),
            "the ledger's word travels with the pea"
        );
        let next = ac_vendor::Run::new().step(&snap, Instant::now());
        assert_eq!(
            next.act,
            Some(Act::Sell {
                items: vec![0x8000_0010, 0x8000_0011]
            }),
            "{}",
            next.saying
        );
    }

    /// A stack of `stack` `name` in the pack, a spell component, with
    /// nothing written down about it.
    fn component_in_the_pack(c: &mut Client, guid: u32, name: &str, wcid: u32, stack: u32) {
        let me = c.world.player_guid.unwrap();
        c.world.objects.insert(
            guid,
            ac_world::WorldObject {
                guid,
                name: name.into(),
                weenie_class_id: wcid,
                item_type: item_type::SPELL_COMPONENTS,
                value: 5 * stack,
                stack_size: stack,
                max_stack_size: 1_000,
                container: Some(me),
                ..Default::default()
            },
        );
    }

    /// The weenie class the archives give a component of this name.
    fn component_named(c: &Client, name: &str) -> u32 {
        let table = c.assets.spell_components().unwrap();
        let id = table.find_by_name(name).expect(name);
        c.assets
            .spell_component_ids()
            .unwrap()
            .component_wcid(id)
            .expect(name)
    }

    /// Write down what the profile makes of this item now, as the
    /// arrival pass does: the item is judged by the rules once, when it
    /// is taken, and the answer travels with it.
    fn tagged_by_the_profile(c: &mut Client, guid: u32) -> LootAction {
        let stats = c.stats_of(guid).unwrap();
        let action = c.loot_action(&stats).expect("a rule claims it");
        c.autoplay.tag(&stats, action);
        action
    }

    /// The war mage in front of Cindrue's open window, which buys
    /// components: what she is offered, and what the run does first.
    fn at_cindrues_counter(c: &mut Client, cfg: &Growth) -> (Vec<u32>, ac_vendor::Next) {
        let cindrue = 0x8000_0002;
        vendor_beside(
            c,
            cindrue,
            "Archmage Cindrue",
            glam::Vec3::new(1.0, 0.0, 0.0),
        );
        let mut window = window_of(cindrue);
        window.item_types = item_type::SPELL_COMPONENTS;
        c.world.open_vendor = Some(window);
        let snap = c.vendor_snapshot(cfg);
        let mut offered: Vec<u32> = snap
            .items
            .iter()
            .filter(|i| snap.offers(i))
            .map(|i| i.guid)
            .collect();
        offered.sort_unstable();
        let next = ac_vendor::Run::new().step(&snap, Instant::now());
        (offered, next)
    }

    #[test]
    fn a_scarab_the_player_said_to_sell_goes_though_its_own_spells_burn_it() {
        // The principle, in the player's words: follow the loot
        // profile. A rule that says "sell lead scarabs" is the player's
        // decision about every lead scarab the character picks up, and
        // it was being overruled three ways at once: the component
        // guard kept it from the counter because Flame Bolt burns it,
        // the restock list wanted more of it for the same reason, and
        // the counter skipped it as what the character came to buy.
        use ac_vendor::Act;
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        with_a_pack(&mut c, 20);
        as_a_war_mage(&mut c);
        let scarab = component_named(&c, "Lead Scarab");
        tapers_in_the_pack(&mut c, 0x8000_0012, 78);
        component_in_the_pack(&mut c, 0x8000_0013, "Lead Scarab", scarab, 3);
        with_a_profile(
            &mut c,
            "sells-scarabs",
            vec![
                word_rule("lead scarabs to sell", "lead scarab", LootAction::Sell),
                word_rule("components", "taper", LootAction::Keep),
            ],
            &[("Prismatic Taper", 100, 25)],
        );
        assert_eq!(tagged_by_the_profile(&mut c, 0x8000_0013), LootAction::Sell);
        let cfg = c.autoplay.config.growth.clone();
        assert!(
            c.burns(&cfg).contains(&scarab),
            "the spells do burn it; that is the point"
        );

        // Not stock: leaving, so never a need and never a want. The
        // tapers, which the player keeps, still are.
        let needs = c.grow_needs(&cfg);
        assert!(
            !needs.iter().any(|n| n.kind == NeedKind::Component(scarab)),
            "a need for what is being sold: {needs:?}"
        );
        assert!(needs.iter().any(|n| n.kind == NeedKind::Component(20631)));
        let wants = c.vendor_shortfall(&cfg);
        assert!(wants.iter().all(|w| w.wcid != scarab), "{wants:?}");

        // Offered, and sold, at a counter that buys components.
        let (offered, next) = at_cindrues_counter(&mut c, &cfg);
        assert_eq!(offered, [0x8000_0013]);
        assert_eq!(
            next.act,
            Some(Act::Sell {
                items: vec![0x8000_0013]
            }),
            "{}",
            next.saying
        );
    }

    #[test]
    fn a_scarab_nothing_was_decided_about_is_kept_by_the_guard() {
        // The guards still answer for what the profile did not decide.
        // A scarab with no entry in the ledger, burnt by the spells
        // this mage casts, stays out of the counter's hands: "the rest,
        // to the counter" would sell it today, and the guard says no.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        with_a_pack(&mut c, 20);
        as_a_war_mage(&mut c);
        let scarab = component_named(&c, "Lead Scarab");
        tapers_in_the_pack(&mut c, 0x8000_0012, 78);
        component_in_the_pack(&mut c, 0x8000_0013, "Lead Scarab", scarab, 1);
        with_a_profile(
            &mut c,
            "sells-the-rest",
            vec![the_rest_to_the_counter()],
            &[("Prismatic Taper", 100, 25)],
        );
        assert_eq!(c.autoplay.ledger.by_guid(0x8000_0013), None);
        let cfg = c.autoplay.config.growth.clone();
        let (offered, next) = at_cindrues_counter(&mut c, &cfg);
        assert!(offered.is_empty(), "{offered:x?}");
        assert!(
            !matches!(next.act, Some(ac_vendor::Act::Sell { .. })),
            "{}",
            next.saying
        );
    }

    #[test]
    fn a_kind_the_rules_would_sell_on_arrival_is_not_bought() {
        // Whether to buy more of a thing is a question about the kind,
        // and the profile answers it the way it will answer for the
        // purchase when it arrives. Under "the rest, to the counter"
        // a bought scarab is tagged to sell as it lands and sold on
        // the next trip, so it is not bought, however low the pack is
        // and whatever the spells burn. Under the starter's rules,
        // which keep components, it is.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        with_a_pack(&mut c, 20);
        as_a_war_mage(&mut c);
        let scarab = component_named(&c, "Lead Scarab");
        tapers_in_the_pack(&mut c, 0x8000_0012, 78);
        component_in_the_pack(&mut c, 0x8000_0013, "Lead Scarab", scarab, 1);
        with_a_profile(
            &mut c,
            "sells-the-rest",
            vec![the_rest_to_the_counter()],
            &[("Prismatic Taper", 100, 25)],
        );
        let cfg = c.autoplay.config.growth.clone();
        let needs = c.grow_needs(&cfg);
        assert!(
            !needs.iter().any(|n| n.kind == NeedKind::Component(scarab)),
            "bought to be sold: {needs:?}"
        );
        // The tapers are on the buy list, which is the player's word
        // that they are stock: still a need, whatever the rules say.
        assert!(needs.iter().any(|n| n.kind == NeedKind::Component(20631)));

        with_a_buy_list(&mut c, &[("Prismatic Taper", 100, 25)]);
        let needs = c.grow_needs(&cfg);
        let need = needs
            .iter()
            .find(|n| n.kind == NeedKind::Component(scarab))
            .unwrap_or_else(|| panic!("the scarabs it is short of: {needs:?}"));
        assert_eq!(need.have, 1);
        assert!(need.want > 0);
        assert!(c.vendor_shortfall(&cfg).iter().any(|w| w.wcid == scarab));
    }

    #[test]
    fn tapers_taken_to_keep_are_neither_sold_nor_wanted() {
        // On the buy list and written down as kept: the counter is not
        // offered them, and a pack holding its full line has nothing
        // to buy. A Keep says "do not sell this"; it does not say "do
        // not buy more", so a short line is still filled -- the buy
        // list is the player's word too.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        with_a_pack(&mut c, 20);
        as_a_war_mage(&mut c);
        tapers_in_the_pack(&mut c, 0x8000_0012, 100);
        with_a_profile(
            &mut c,
            "keeps-tapers",
            vec![
                word_rule("components", "taper", LootAction::Keep),
                the_rest_to_the_counter(),
            ],
            &[("Prismatic Taper", 100, 25)],
        );
        assert_eq!(tagged_by_the_profile(&mut c, 0x8000_0012), LootAction::Keep);
        let cfg = c.autoplay.config.growth.clone();

        let wants = c.vendor_shortfall(&cfg);
        assert!(wants.iter().all(|w| w.wcid != 20631), "{wants:?}");
        let (offered, next) = at_cindrues_counter(&mut c, &cfg);
        assert!(offered.is_empty(), "{offered:x?}");
        assert!(
            !matches!(next.act, Some(ac_vendor::Act::Sell { .. })),
            "{}",
            next.saying
        );

        // Short of the line, still kept, and still bought.
        c.world.objects.get_mut(&0x8000_0012).unwrap().stack_size = 78;
        let wants = c.vendor_shortfall(&cfg);
        let taper = wants
            .iter()
            .find(|w| w.wcid == 20631)
            .unwrap_or_else(|| panic!("the tapers it is short of: {wants:?}"));
        assert_eq!(taper.short, 22);
        let snap = c.vendor_snapshot(&cfg);
        assert!(snap.items.iter().filter(|i| snap.offers(i)).count() == 0);
    }

    #[test]
    fn what_the_server_will_not_take_stays_whatever_the_profile_said() {
        // The one word ahead of the profile's is the server's own. A
        // dagger in hand and a tinkered ring, both written down as
        // meant for a counter, are not offered: a sale the server will
        // not make is not a decision anybody gets to take.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        with_a_pack(&mut c, 20);
        let me = c.world.player_guid.unwrap();
        c.world.objects.insert(
            0x8000_0020,
            ac_world::WorldObject {
                guid: 0x8000_0020,
                name: "Dagger".into(),
                weenie_class_id: 300,
                item_type: item_type::MELEE_WEAPON,
                value: 900,
                wielder: Some(me),
                parent: Some(me),
                ..Default::default()
            },
        );
        c.world.objects.insert(
            0x8000_0021,
            ac_world::WorldObject {
                guid: 0x8000_0021,
                name: "Ornate Ring".into(),
                weenie_class_id: 301,
                item_type: item_type::JEWELRY,
                value: 900,
                container: Some(me),
                ..Default::default()
            },
        );
        // Tinkered twice, by the server's appraisal (int 171).
        c.appraisals.insert(
            0x8000_0021,
            ac_net::messages::Appraisal {
                guid: 0x8000_0021,
                success: true,
                ints: vec![(171, 2)],
                ..Default::default()
            },
        );
        with_a_profile(
            &mut c,
            "sells-the-rest",
            vec![the_rest_to_the_counter()],
            &[],
        );
        for guid in [0x8000_0020, 0x8000_0021] {
            let stats = c.stats_of(guid).unwrap();
            c.autoplay.tag(&stats, LootAction::Sell);
        }
        let cfg = c.autoplay.config.growth.clone();
        assert!(c.for_sale(&cfg).is_empty());
        let (offered, next) = at_cindrues_counter(&mut c, &cfg);
        assert!(offered.is_empty(), "{offered:x?}");
        assert!(
            !matches!(next.act, Some(ac_vendor::Act::Sell { .. })),
            "{}",
            next.saying
        );
    }

    #[test]
    fn what_was_taken_to_keep_is_not_swept_up_by_the_rest_to_the_counter() {
        // The decision is made once, when the item is taken. A ring
        // taken under "keep ornate rings" stays kept when the rules are
        // later just "the rest, to the counter": asking again at the
        // counter is how a thing taken to keep gets sold on the next
        // run to town.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        with_a_pack(&mut c, 20);
        let me = c.world.player_guid.unwrap();
        c.world.objects.insert(
            0x8000_0021,
            ac_world::WorldObject {
                guid: 0x8000_0021,
                name: "Ornate Ring".into(),
                weenie_class_id: 301,
                item_type: item_type::JEWELRY,
                value: 900,
                container: Some(me),
                ..Default::default()
            },
        );
        with_a_profile(
            &mut c,
            "keeps-rings",
            vec![
                word_rule("ornate rings", "ornate", LootAction::Keep),
                the_rest_to_the_counter(),
            ],
            &[],
        );
        assert_eq!(tagged_by_the_profile(&mut c, 0x8000_0021), LootAction::Keep);
        // The rules change under it: today they would sell it.
        with_a_profile(
            &mut c,
            "sells-the-rest",
            vec![the_rest_to_the_counter()],
            &[],
        );
        let stats = c.stats_of(0x8000_0021).unwrap();
        let policy = c.sell_policy(&c.autoplay.config.growth.clone());
        assert!(
            policy.profile.as_ref().is_some_and(|p| matches!(
                p.judge(&stats, None, &policy.wielder, &policy.me, 0),
                crate::profile::Verdict::Decided(LootAction::Sell, _)
            )),
            "the rules as they read now would sell it"
        );
        let cfg = c.autoplay.config.growth.clone();
        assert!(c.for_sale(&cfg).is_empty(), "but it was taken to keep");
        let (offered, _) = at_cindrues_counter(&mut c, &cfg);
        assert!(offered.is_empty(), "{offered:x?}");
    }

    #[test]
    fn a_stack_to_sell_beside_one_to_keep_goes_whole_and_is_not_poured_into_it() {
        // Two stacks of scarabs, one word each: ten the player said to
        // sell and ninety-five they said to keep. The tidy that runs
        // before every sale used to pour the ten into the ninety-five,
        // the ledger settled the lot as kept, and the counter was
        // offered nothing. The ten go over the counter whole.
        use ac_vendor::Act;
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        with_a_pack(&mut c, 20);
        as_a_war_mage(&mut c);
        let scarab = component_named(&c, "Lead Scarab");
        tapers_in_the_pack(&mut c, 0x8000_0012, 100);
        component_in_the_pack(&mut c, 0x8000_0013, "Lead Scarab", scarab, 10);
        component_in_the_pack(&mut c, 0x8000_0014, "Lead Scarab", scarab, 95);
        with_a_profile(
            &mut c,
            "keeps-some",
            vec![word_rule("components", "taper", LootAction::Keep)],
            &[("Prismatic Taper", 100, 25)],
        );
        for (guid, word) in [
            (0x8000_0013, LootAction::Sell),
            (0x8000_0014, LootAction::Keep),
        ] {
            let stats = c.stats_of(guid).unwrap();
            c.autoplay.tag(&stats, word);
        }
        let cfg = c.autoplay.config.growth.clone();
        // The client's own tidy leaves them apart too.
        assert!(
            matches!(c.pour_next(Instant::now()), Err(Unpoured::Tight)),
            "the tidy poured across words"
        );
        let (offered, next) = at_cindrues_counter(&mut c, &cfg);
        assert_eq!(offered, [0x8000_0013]);
        assert_eq!(
            next.act,
            Some(Act::Sell {
                items: vec![0x8000_0013]
            }),
            "{}",
            next.saying
        );
    }

    #[test]
    fn a_stack_on_its_way_out_does_not_hide_the_shortfall_of_the_one_that_stays() {
        // Seventy-eight tapers kept and five hundred tagged to sell,
        // against a line of a hundred. The five hundred are not stock:
        // the line is twenty-two short, and the party hears the same.
        // The restock list once dropped the whole line while any of
        // the kind was leaving, and the party's broadcast counted the
        // leaving stack as stock, so a mate handed tapers over while
        // the character's own list said it wanted none.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        with_a_pack(&mut c, 20);
        as_a_war_mage(&mut c);
        tapers_in_the_pack(&mut c, 0x8000_0012, 78);
        tapers_in_the_pack(&mut c, 0x8000_0015, 500);
        with_a_profile(
            &mut c,
            "sells-the-rest",
            vec![the_rest_to_the_counter()],
            &[("Prismatic Taper", 100, 25)],
        );
        for (guid, word) in [
            (0x8000_0012, LootAction::Keep),
            (0x8000_0015, LootAction::Sell),
        ] {
            let stats = c.stats_of(guid).unwrap();
            c.autoplay.tag(&stats, word);
        }
        let cfg = c.autoplay.config.growth.clone();
        let needs = c.grow_needs(&cfg);
        let taper = needs
            .iter()
            .find(|n| n.kind == NeedKind::Component(20631))
            .unwrap_or_else(|| panic!("the tapers it is short of: {needs:?}"));
        assert_eq!((taper.have, taper.want), (78, 22));
        let line = needs
            .iter()
            .find(|n| n.kind == NeedKind::Named("Prismatic Taper".into()))
            .unwrap_or_else(|| panic!("the line it is short of: {needs:?}"));
        assert_eq!((line.have, line.want), (78, 22));
        // And the party is told the same.
        c.autoplay.config.team.enabled = true;
        c.autoplay_stock();
        assert_eq!(c.autoplay.wants, vec!["Prismatic Taper".to_string()]);
        // With the five hundred sold, nothing changes but the count.
        c.world.objects.remove(&0x8000_0015);
        c.autoplay.ledger.forget(0x8000_0015);
        let needs = c.grow_needs(&cfg);
        let taper = needs
            .iter()
            .find(|n| n.kind == NeedKind::Component(20631))
            .unwrap();
        assert_eq!((taper.have, taper.want), (78, 22));
    }

    #[test]
    fn a_scarab_sold_at_the_counter_is_not_bought_straight_back() {
        // The round trip the restock list is there to avoid, in one
        // visit: the scarab the player said to sell goes over the
        // counter, its tag is forgotten with it, and the same counter
        // has scarabs on the shelf. Asking only "is any of this
        // leaving?" said no the moment it had gone, and the run bought
        // it back at markup for the arrival pass to tag to sell again.
        use ac_vendor::Act;
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        with_a_pack(&mut c, 20);
        as_a_war_mage(&mut c);
        let scarab = component_named(&c, "Lead Scarab");
        tapers_in_the_pack(&mut c, 0x8000_0012, 100);
        component_in_the_pack(&mut c, 0x8000_0013, "Lead Scarab", scarab, 3);
        let me = c.world.player_guid.unwrap();
        c.world.objects.insert(
            0x8000_0016,
            ac_world::WorldObject {
                guid: 0x8000_0016,
                name: "Pyreal".into(),
                weenie_class_id: 273,
                item_type: item_type::MONEY,
                value: 5_000,
                stack_size: 5_000,
                max_stack_size: 25_000,
                container: Some(me),
                ..Default::default()
            },
        );
        with_a_profile(
            &mut c,
            "sells-scarabs",
            vec![
                word_rule("lead scarabs to sell", "lead scarab", LootAction::Sell),
                word_rule("components", "taper", LootAction::Keep),
            ],
            &[("Prismatic Taper", 100, 25)],
        );
        assert_eq!(tagged_by_the_profile(&mut c, 0x8000_0013), LootAction::Sell);
        let cfg = c.autoplay.config.growth.clone();

        let cindrue = 0x8000_0002;
        vendor_beside(
            &mut c,
            cindrue,
            "Archmage Cindrue",
            glam::Vec3::new(1.0, 0.0, 0.0),
        );
        let mut window = window_of(cindrue);
        window.item_types = item_type::SPELL_COMPONENTS;
        window.items.push(ac_world::object::VendorItem {
            guid: 0x9000_0001,
            stack: 100,
            desc: ac_world::object::WeenieDesc {
                name: "Lead Scarab".into(),
                weenie_class_id: scarab,
                item_type: item_type::SPELL_COMPONENTS,
                value: 5,
                ..Default::default()
            },
        });
        c.world.open_vendor = Some(window);
        let mut run = ac_vendor::Run::new();
        let snap = c.vendor_snapshot(&cfg);
        assert!(
            snap.wants.iter().all(|w| w.wcid != scarab),
            "{:?}",
            snap.wants
        );
        let next = run.step(&snap, Instant::now());
        assert_eq!(
            next.act,
            Some(Act::Sell {
                items: vec![0x8000_0013]
            }),
            "{}",
            next.saying
        );
        // Sold: the server takes it, and the ledger forgets it.
        c.world.objects.remove(&0x8000_0013);
        c.autoplay.ledger.forget(0x8000_0013);
        let snap = c.vendor_snapshot(&cfg);
        assert!(
            snap.wants.iter().all(|w| w.wcid != scarab),
            "wanted back the moment it was gone: {:?}",
            snap.wants
        );
        let next = run.step(&snap, Instant::now());
        assert!(
            !matches!(next.act, Some(Act::Buy { wcid, .. }) if wcid == scarab),
            "bought straight back: {} ({:?})",
            next.saying,
            next.act
        );
    }

    #[test]
    fn a_casters_heal_is_stocked_for_though_no_bolt_shares_its_herb() {
        // Heal Self V burns an herb, a powder, a potion and a talisman
        // (7, 26, 41, 61 in the dat) that no bolt or buff burns, and a
        // caster without a Life focus needs every one of them. The
        // restock list once stocked for the buffs and the bolts alone,
        // so when the pack ran out the heal stopped casting and nobody
        // went to town for it.
        const HEAL_SELF_V: u32 = 1160;
        const HERB: u32 = 7;
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        with_a_pack(&mut c, 20);
        as_a_war_mage(&mut c);
        tapers_in_the_pack(&mut c, 0x8000_0012, 100);
        with_a_buy_list(&mut c, &[("Prismatic Taper", 100, 25)]);
        let herb = c
            .assets
            .spell_component_ids()
            .unwrap()
            .component_wcid(HERB)
            .expect("the herb's weenie");
        let cfg = c.autoplay.config.growth.clone();
        let needs = c.grow_needs(&cfg);
        assert!(
            !needs.iter().any(|n| n.kind == NeedKind::Component(herb)),
            "no spell of a war mage's burns the herb: {needs:?}"
        );

        c.world.stats.spells.push(HEAL_SELF_V);
        assert!(c.spells_cast().contains(&HEAL_SELF_V));
        let needs = c.grow_needs(&cfg);
        let need = needs
            .iter()
            .find(|n| n.kind == NeedKind::Component(herb))
            .unwrap_or_else(|| panic!("the heal's herb: {needs:?}"));
        assert_eq!(need.have, 0);
        assert!(need.want > 0);
    }

    #[test]
    fn arrows_the_player_said_to_sell_are_not_counted_as_the_launchers_stock() {
        // Three hundred arrows in the pack and a bow in hand. Tagged to
        // sell, they are loot in the ammunition slot, not stock: the
        // launcher is short by the whole of what it keeps, and a
        // forecast that counted the arrows as stock would set off to
        // town for what it was about to sell -- or not set off at all.
        use ac_world::fletching::ammo_type;
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        with_a_pack(&mut c, 20);
        let me = c.world.player_guid.unwrap();
        c.world.objects.insert(
            0x8000_0030,
            ac_world::WorldObject {
                guid: 0x8000_0030,
                name: "Yumi".into(),
                item_type: item_type::MISSILE_WEAPON,
                ammo_type: ammo_type::ARROW,
                value: 500,
                wielder: Some(me),
                parent: Some(me),
                ..Default::default()
            },
        );
        c.world.objects.insert(
            0x8000_0031,
            ac_world::WorldObject {
                guid: 0x8000_0031,
                name: "Arrow".into(),
                weenie_class_id: 300,
                item_type: item_type::MISSILE_WEAPON,
                valid_locations: equip::MISSILE_AMMO,
                value: 300,
                stack_size: 300,
                max_stack_size: 1_000,
                container: Some(me),
                ..Default::default()
            },
        );
        let mut cfg = c.autoplay.config.growth.clone();
        cfg.ammo_keep = 250;
        let needs = c.grow_needs(&cfg);
        assert!(
            !needs
                .iter()
                .any(|n| n.kind == NeedKind::Ammo(ammo_type::ARROW)),
            "three hundred in the pack: {needs:?}"
        );
        let stats = c.stats_of(0x8000_0031).unwrap();
        c.autoplay.tag(&stats, LootAction::Sell);
        let needs = c.grow_needs(&cfg);
        let arrows = needs
            .iter()
            .find(|n| n.kind == NeedKind::Ammo(ammo_type::ARROW))
            .unwrap_or_else(|| panic!("the arrows it will be short of: {needs:?}"));
        assert_eq!((arrows.have, arrows.want), (0, 250));
    }

    #[test]
    fn a_counter_that_buys_none_of_the_loot_is_walked_past_on_a_run_to_sell() {
        // At the counter, with the window open, the pack looked over
        // and nothing on the sale list: the run used to stand there and
        // sell nothing, say "sold 0 item(s)", and walk home with the
        // peas. It says who would not buy them and goes on to who will.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        with_a_pack(&mut c, 20);
        pea_in_the_pack(&mut c, 0x8000_0010, "Iron Pea", 8328, 2_500);
        pea_in_the_pack(&mut c, 0x8000_0011, "Lead Pea", 8329, 500);
        let cfg = c.autoplay.config.growth.clone();
        assert_eq!(c.salables(&cfg).len(), 2, "the peas are for a counter");
        let me = c.player.as_ref().unwrap().world_position();
        let me = Vec2::new(me.x, me.y);
        let now = Instant::now();
        let rakk = 0x8000_0002;
        vendor_beside(
            &mut c,
            rakk,
            "Rakk the Peddler",
            glam::Vec3::new(1.0, 0.0, 0.0),
        );
        // A tailor's window: armour and clothing, no components.
        let mut window = window_of(rakk);
        window.item_types = item_type::ARMOR | item_type::CLOTHING;
        c.world.open_vendor = Some(window);
        assert!(c.sale_list(&cfg).is_empty(), "Rakk buys peas");

        let mut run = run_to(me, now);
        run.vendor = "Rakk the Peddler".into();
        run.phase = Phase::Appraising;
        run.since = now - SETTLE * 3;
        c.autoplay.growth.run = Some(run);
        c.grow_run_step(now, &cfg);
        assert!(
            c.autoplay
                .status
                .contains("Rakk the Peddler buys none of this"),
            "{}",
            c.autoplay.status
        );
        assert!(c.world.open_vendor.is_none(), "the window was left open");
        assert!(
            c.autoplay.growth.skip_vendors.held(&spot(me), now),
            "Rakk is not left alone"
        );
        // And left alone for at least a run's wait: on the half minute
        // a blocked counter gets, tidied away at the end of the run,
        // Rakk was the best-paying choice again every RUN_EVERY.
        assert!(
            c.autoplay
                .growth
                .skip_vendors
                .held(&spot(me), now + RUN_EVERY - Duration::from_secs(1)),
            "Rakk is the next run's counter"
        );
        // Not standing at Rakk's counter selling nothing: on to a
        // counter that buys peas, of which Holtburg has one.
        let on = c
            .autoplay
            .growth
            .run
            .as_ref()
            .expect("walked home with the peas");
        assert_eq!(on.vendor, "Archmage Cindrue");
        assert_eq!(on.stops, 2);
        assert_eq!(on.errand, Errand::Sell);
        assert!(c.traveling());

        // A run made to buy stays and shops: the counter is the one the
        // player opened, or the one with the tapers on the shelf.
        c.town_run_stop(now);
        c.world.open_vendor = Some({
            let mut w = window_of(rakk);
            w.item_types = item_type::ARMOR;
            w
        });
        let mut run = run_to(me, now);
        run.vendor = "Rakk the Peddler".into();
        run.errand = Errand::Buy;
        run.phase = Phase::Appraising;
        run.since = now - SETTLE * 3;
        c.autoplay.growth.run = Some(run);
        c.grow_run_step(now, &cfg);
        let stayed = c.autoplay.growth.run.as_ref().expect("the run ended");
        assert_eq!(stayed.vendor, "Rakk the Peddler");
        assert!(matches!(stayed.phase, Phase::Selling { .. }));
    }

    #[test]
    fn peas_for_a_counter_send_a_roomy_pack_to_town_once_the_waits_are_up() {
        // The user's report: two peas tagged to sell, room in the pack,
        // nothing short, and no run was ever made. Now the peas are the
        // reason -- once the waits between runs are up, as for any
        // other reason, so one pea does not wear a path to town.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        with_a_pack(&mut c, 50);
        pea_in_the_pack(&mut c, 0x8000_0010, "Iron Pea", 8328, 2_500);
        pea_in_the_pack(&mut c, 0x8000_0011, "Iron Pea", 8328, 2_500);
        let cfg = c.autoplay.config.growth.clone();
        assert!(!c.pack_low_on_room() && !c.laden(&cfg), "the old reasons");
        let now = Instant::now();

        // Just back from a run: the wait between runs holds.
        c.autoplay.growth.last_run = Some(now);
        assert!(!c.grow_town_run(now, &cfg));
        assert!(
            c.autoplay
                .growth
                .held_back
                .contains("to wait since the last run"),
            "{}",
            c.autoplay.growth.held_back
        );
        // A futile one holds for its own while.
        c.autoplay.growth.run_was_futile = true;
        let soon = now + FUTILE_RUN_WAIT - Duration::from_secs(1);
        assert!(!c.grow_town_run(soon, &cfg));
        c.autoplay.growth.run_was_futile = false;
        // Waits up, five thousand at face in the pack: off to the one
        // counter in town that buys peas.
        let later = now + RUN_EVERY;
        assert!(
            c.grow_town_run(later, &cfg),
            "{}",
            c.autoplay.growth.held_back
        );
        let run = c.autoplay.growth.run.as_ref().expect("no run");
        assert_eq!(run.errand, Errand::Sell);
        assert!(run.reason.contains("5000 pyreals' worth"), "{}", run.reason);
        assert_eq!(run.vendor, "Archmage Cindrue");
        assert!(c.traveling());

        // One Lead Pea is not worth the trip on its own...
        c.town_run_stop(later);
        c.world.objects.remove(&0x8000_0010);
        c.world.objects.remove(&0x8000_0011);
        pea_in_the_pack(&mut c, 0x8000_0012, "Lead Pea", 8329, 500);
        let again = later + RUN_EVERY;
        assert!(!c.grow_town_run(again, &cfg));
        assert!(
            c.autoplay
                .growth
                .held_back
                .contains("1 thing(s) for a counter, not yet worth the trip"),
            "{}",
            c.autoplay.growth.held_back
        );
        // ...until it has been carried a quarter of an hour.
        let patience = Duration::from_secs_f32(cfg.sell_run_patience);
        assert!(!c.grow_town_run(again + patience / 2, &cfg));
        assert!(
            c.grow_town_run(again + patience, &cfg),
            "{}",
            c.autoplay.growth.held_back
        );
        let run = c.autoplay.growth.run.as_ref().expect("no run");
        assert!(run.reason.contains("carried 15 min"), "{}", run.reason);
        assert_eq!(run.vendor, "Archmage Cindrue");
        // Setting off put the clock back: what comes home unsold is
        // counted afresh.
        assert_eq!(c.autoplay.growth.sale_since, None);
    }

    #[test]
    fn a_patience_too_large_for_a_duration_turns_the_rule_off() {
        // The config is hand-edited JSON, and 1e20 is a finite f32 that
        // no Duration holds: it used to panic on the first frame a run
        // could start with anything for a counter in the pack.
        use ac_world::item_type::SPELL_COMPONENTS;
        let cfg = Growth {
            sell_run_patience: 1e20,
            ..Growth::default()
        };
        let pea = [salable(1, SPELL_COMPONENTS, 500, 1)];
        assert_eq!(
            worth_a_sale_run(&pea, Some(Duration::from_secs(60 * 60)), &cfg),
            None
        );
    }

    /// Coin in the pack.
    fn coin_in_the_pack(c: &mut Client, guid: u32, amount: u32) {
        let me = c.world.player_guid.unwrap();
        c.world.objects.insert(
            guid,
            ac_world::WorldObject {
                guid,
                name: "Pyreal".into(),
                item_type: item_type::MONEY,
                value: 1,
                stack_size: amount,
                max_stack_size: 25_000,
                container: Some(me),
                ..Default::default()
            },
        );
    }

    /// Something in the pack by name, `stack` of it, kept.
    fn thing_in_the_pack(c: &mut Client, guid: u32, name: &str, stack: u32) {
        let me = c.world.player_guid.unwrap();
        c.world.objects.insert(
            guid,
            ac_world::WorldObject {
                guid,
                name: name.into(),
                value: 1,
                stack_size: stack,
                max_stack_size: 1_000,
                container: Some(me),
                ..Default::default()
            },
        );
    }

    /// The character's buy list, and nothing else on it: `what`, `keep`
    /// of them, urgent at `restock_at` or fewer. The starter's rules.
    fn with_a_buy_list(c: &mut Client, lines: &[(&str, u32, u32)]) {
        with_a_profile(c, "wants", crate::profile::Profile::starter().rules, lines);
    }

    /// A profile of the character's own, `name`, with these `rules` in
    /// this order and this buy list. A shelf of its own, so the one
    /// every session shares is not touched.
    fn with_a_profile(
        c: &mut Client,
        name: &str,
        rules: Vec<crate::profile::Rule>,
        lines: &[(&str, u32, u32)],
    ) {
        let dir = std::env::temp_dir().join("acswarm-test-growth-profiles");
        std::fs::create_dir_all(&dir).ok();
        let shelf = std::sync::Arc::new(crate::profile::Library::default());
        shelf.open(&dir);
        let mut p = crate::profile::Profile::starter();
        p.name = name.into();
        p.rules = rules;
        p.buy.clear();
        for (what, keep, restock_at) in lines {
            p.buy.push(crate::profile::Buy {
                what: (*what).into(),
                keep: *keep,
                restock_at: Some(*restock_at),
                on: true,
                ..Default::default()
            });
        }
        shelf.put(p).ok();
        c.profiles = shelf;
        c.autoplay.config.loot.profile = name.into();
    }

    /// One rule: `action` for anything whose name contains `word`.
    fn word_rule(name: &str, word: &str, action: LootAction) -> crate::profile::Rule {
        crate::profile::Rule {
            name: name.into(),
            action,
            all: vec![crate::profile::Ask::Item(crate::items::Term::Word(
                word.into(),
            ))],
            ..Default::default()
        }
    }

    /// "The rest, to the counter": a rule that claims anything at all.
    fn the_rest_to_the_counter() -> crate::profile::Rule {
        crate::profile::Rule {
            name: "the rest".into(),
            action: LootAction::Sell,
            all: vec![crate::profile::Ask::Item(crate::items::Term::Num(
                crate::items::NumKey::Value,
                crate::items::Op::Ge,
                0.0,
            ))],
            ..Default::default()
        }
    }

    /// The shop of this name.
    fn shop_named(name: &str) -> &'static ac_world::shops::Shop {
        ac_world::shops::all()
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("no shop {name}"))
    }

    #[test]
    fn an_urgent_need_comes_before_the_loot_and_the_loot_is_sold_on_the_way() {
        // An archer out of arrows with an armful of cheap peas. Ranked
        // for the sale first, the run went to the archmage and the
        // arrows waited on a second stop that may not reach the bowyer;
        // the supply comes first, and the peas are sold on the way at
        // the counter a hundred metres on, not carried home for a run
        // of their own.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        with_a_pack(&mut c, 50);
        for i in 0..8 {
            pea_in_the_pack(&mut c, 0x8000_0010 + i, "Lead Pea", 8329, 100);
        }
        coin_in_the_pack(&mut c, 0x8000_0030, 1_000);
        // Arrowshafts by their full name: a want called "Arrow" is
        // answered by the archmage's Yarrow.
        with_a_buy_list(&mut c, &[("Bundle of Arrowshafts", 10, 5)]);
        let cfg = c.autoplay.config.growth.clone();
        assert_eq!(c.salables(&cfg).len(), 8, "an armful for a counter");
        let now = Instant::now();
        assert!(
            c.grow_town_run(now, &cfg),
            "{}",
            c.autoplay.growth.held_back
        );
        let run = c.autoplay.growth.run.take().expect("no run");
        assert_eq!(run.errand, Errand::Buy);
        assert!(
            run.reason.contains("short of Bundle of Arrowshafts"),
            "{}",
            run.reason
        );
        assert!(
            shop_named(&run.vendor)
                .stocks("Bundle of Arrowshafts")
                .is_some(),
            "{} has no arrowshafts",
            run.vendor
        );
        assert_ne!(run.vendor, "Archmage Cindrue");
        // Done at the bowyer: on to the one counter in town that takes
        // the peas, on the same run.
        c.cancel_travel();
        assert!(c.grow_run_next(run, now, &cfg, None), "went home");
        let on = c
            .autoplay
            .growth
            .run
            .as_ref()
            .expect("went home with the peas");
        assert_eq!(on.vendor, "Archmage Cindrue");
        assert_eq!(on.errand, Errand::Sell);
        assert_eq!(on.stops, 2);
    }

    #[test]
    fn the_panel_run_with_a_list_and_a_pea_goes_to_the_counter_with_the_list() {
        // The archer has 180 of 250 arrows -- short, not urgent -- and
        // a pea. The panel's button, made a run to sell whenever the
        // pack held anything for a counter, went to the archmage, sold
        // the pea and ended in town with the arrows unbought. It goes
        // where the list is, and the pea to the archmage after.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        with_a_pack(&mut c, 50);
        pea_in_the_pack(&mut c, 0x8000_0010, "Lead Pea", 8329, 500);
        thing_in_the_pack(&mut c, 0x8000_0020, "Bundle of Arrowshafts", 18);
        coin_in_the_pack(&mut c, 0x8000_0030, 1_000);
        with_a_buy_list(&mut c, &[("Bundle of Arrowshafts", 25, 6)]);
        let cfg = c.autoplay.config.growth.clone();
        let now = Instant::now();
        let needs = c.needs_now(now, &cfg);
        assert!(
            needs
                .iter()
                .any(|n| n.name == "Bundle of Arrowshafts" && !n.urgent && n.want == 7),
            "{needs:?}"
        );
        c.town_run_by_hand(now).expect("no run");
        let run = c.autoplay.growth.run.take().expect("no run");
        assert_eq!(run.errand, Errand::Buy);
        assert!(
            shop_named(&run.vendor)
                .stocks("Bundle of Arrowshafts")
                .is_some(),
            "{}",
            run.vendor
        );
        assert_ne!(run.vendor, "Archmage Cindrue");
        c.cancel_travel();
        assert!(
            c.grow_run_next(run, now, &cfg, None),
            "went home with the pea"
        );
        let on = c.autoplay.growth.run.as_ref().unwrap();
        assert_eq!(on.vendor, "Archmage Cindrue");
        assert_eq!(on.errand, Errand::Sell);
    }

    /// Every counter within `reach` of the character that would pay for
    /// a pea, held off.
    fn nobody_near_buys_peas(c: &mut Client, reach: f32, now: Instant) -> usize {
        use ac_world::item_type::SPELL_COMPONENTS;
        let me = c.player.as_ref().unwrap().world_position();
        let me = Vec2::new(me.x, me.y);
        let buyers: Vec<Vec2> = ac_world::shops::all()
            .iter()
            .filter(|s| s.xy().distance(me) <= reach)
            .filter(|s| s.pays_for(SPELL_COMPONENTS, 500).is_some())
            .map(|s| s.xy())
            .collect();
        for at in &buyers {
            c.autoplay
                .growth
                .skip_vendors
                .hold(spot(*at), Duration::from_secs(60 * 60), now);
        }
        buyers.len()
    }

    #[test]
    fn the_panel_run_starts_even_when_nobody_buys_the_pea() {
        // Nothing on the list, a pea in the pack, and no counter near
        // that takes it: the button still starts a run, to the nearest
        // counter, as it always did.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        with_a_pack(&mut c, 50);
        pea_in_the_pack(&mut c, 0x8000_0010, "Lead Pea", 8329, 500);
        with_a_buy_list(&mut c, &[]);
        let now = Instant::now();
        assert!(nobody_near_buys_peas(&mut c, SALE_RUN_REACH, now) > 0);
        c.town_run_by_hand(now).expect("no run");
        let run = c.autoplay.growth.run.as_ref().expect("no run");
        assert_eq!(run.errand, Errand::Buy);
        assert_ne!(run.vendor, "Archmage Cindrue");
    }

    #[test]
    fn a_run_to_sell_for_a_light_reason_stays_within_the_town() {
        // One Lead Pea, carried a quarter of an hour, with nobody in
        // the town buying it: not a walk to an archmage three towns
        // over. A pack that cannot hunt on is worth a walk anywhere; a
        // pea is worth the town the character is in.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        with_a_pack(&mut c, 50);
        pea_in_the_pack(&mut c, 0x8000_0010, "Lead Pea", 8329, 500);
        with_a_buy_list(&mut c, &[]);
        let cfg = c.autoplay.config.growth.clone();
        let now = Instant::now();
        assert!(nobody_near_buys_peas(&mut c, SALE_RUN_REACH, now) > 0);
        let patience = Duration::from_secs_f32(cfg.sell_run_patience);
        c.autoplay.growth.sale_since = Some(now - patience);
        assert!(!c.grow_town_run(now, &cfg));
        assert!(c.autoplay.growth.run.is_none());
        assert!(
            c.autoplay
                .growth
                .held_back
                .contains("nobody within 600 m of a way out buys any of the 1 thing(s)"),
            "{}",
            c.autoplay.growth.held_back
        );
    }

    #[test]
    fn a_party_run_sells_what_this_character_carries_first() {
        // Two on a team that restocks together, the party gone shopping
        // for a mate's full pack, and this one carrying two Iron Peas.
        // The party's run was always a run to buy, so its first stop
        // was whichever counter had the most of the list, with the
        // peas along for the walk; its own pack decides its errand.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.stats.level = 20;
        with_a_pack(&mut c, 50);
        pea_in_the_pack(&mut c, 0x8000_0010, "Iron Pea", 8328, 2_500);
        pea_in_the_pack(&mut c, 0x8000_0011, "Iron Pea", 8328, 2_500);
        with_a_buy_list(&mut c, &[]);
        c.autoplay.config.team.enabled = true;
        c.autoplay.config.team.restock.together = true;
        c.autoplay.team.mates = vec![crate::autoplay::Mate {
            name: "+Brynith".into(),
            guid: 0x5000_0002,
            ..Default::default()
        }];
        let cfg = c.autoplay.config.growth.clone();
        let now = Instant::now();
        // What it tells the party: loot enough for a trip.
        assert!(c.supplies(&cfg, now).sale);
        c.autoplay.growth.mode = logistics::GroupMode::Restocking(Stage::Shopping);
        c.autoplay.growth.mode_because = "+Brynith's pack is full".into();
        assert!(
            c.grow_town_run(now, &cfg),
            "{}",
            c.autoplay.growth.held_back
        );
        let run = c.autoplay.growth.run.as_ref().expect("no run");
        assert_eq!(run.reason, "+Brynith's pack is full");
        assert_eq!(run.errand, Errand::Sell);
        assert_eq!(run.vendor, "Archmage Cindrue");
    }

    #[test]
    fn a_use_turned_away_for_our_own_cast_is_sent_over_not_given_up() {
        use OnOpening::*;
        let (open, shut) = (true, false);
        let (flying, landed) = (true, false);
        let soon = Duration::from_secs(1);
        let long = VENDOR_OPEN_TIMEOUT + Duration::from_secs(1);
        // An open window ends the wait whatever else is going on.
        assert_eq!(on_opening(open, Some(soon), flying, 0, long, 2), Opened);
        // Turned away as busy: wait for our own cast, then a cast's
        // length more (the counter's UseDone frees the slot early), then
        // ask over -- and the retry is not spent, however long it took.
        assert_eq!(on_opening(shut, Some(soon), flying, 0, soon, 1), Wait);
        assert_eq!(on_opening(shut, Some(soon), landed, 0, soon, 1), Wait);
        assert_eq!(on_opening(shut, Some(soon), flying, 0, long, 1), Wait);
        assert_eq!(
            on_opening(shut, Some(BUSY_REASK), landed, 0, long, 1),
            AskOver
        );
        assert_eq!(
            on_opening(shut, Some(BUSY_REASK), landed, 0, long, 2),
            AskOver
        );
        // A counter that keeps saying busy with nothing of ours in the
        // air is taken at its word: the ordinary silence rules apply.
        assert_eq!(
            on_opening(shut, Some(BUSY_REASK), landed, BUSY_ASKS, soon, 1),
            Wait
        );
        assert_eq!(
            on_opening(shut, Some(BUSY_REASK), landed, BUSY_ASKS, long, 1),
            AskAgain
        );
        assert_eq!(
            on_opening(shut, Some(BUSY_REASK), landed, BUSY_ASKS, long, 2),
            GiveUp
        );
        // Silence: one more ask, then the counter is given up.
        assert_eq!(on_opening(shut, None, landed, 0, soon, 1), Wait);
        assert_eq!(on_opening(shut, None, landed, 0, long, 1), AskAgain);
        assert_eq!(on_opening(shut, None, landed, 0, long, 2), GiveUp);
    }

    #[test]
    fn the_counter_is_stood_at_from_the_use_to_the_window_closing() {
        let opening = Phase::Opening {
            guid: 1,
            tries: 1,
            busy: None,
            asked_over: 0,
        };
        let selling = Phase::Selling { sent: Vec::new() };
        assert_eq!(
            counter_asked(Some(&Phase::Going), Some(2)),
            None,
            "still walking: a window open now is one left over"
        );
        assert_eq!(counter_asked(Some(&opening), None), Some(1));
        assert_eq!(counter_asked(Some(&Phase::Appraising), Some(1)), Some(1));
        assert_eq!(counter_asked(Some(&selling), Some(1)), Some(1));
        assert_eq!(
            counter_asked(None, Some(2)),
            Some(2),
            "a window open by hand"
        );
        assert_eq!(counter_asked(None, None), None);
    }

    /// A weenie error as the server sends it, as a game event body.
    fn weenie_error(code: u32) -> Vec<u8> {
        let mut w = ac_net::wire::Writer::new();
        w.u32(0x5000_0001)
            .u32(0)
            .u32(ac_net::messages::event::WEENIE_ERROR)
            .u32(code);
        w.finish()
    }

    /// A counter in view at `at`.
    fn a_counter(c: &mut Client, guid: u32, name: &str, at: glam::Vec3) {
        let mut o = ac_world::WorldObject {
            guid,
            name: name.into(),
            item_type: item_type::CREATURE,
            object_desc_flags: object_desc_flags::VENDOR,
            ..Default::default()
        };
        o.position = Some(ac_world::object::Position {
            cell: 0xA9B4_0019,
            local: at,
            rotation: glam::Quat::IDENTITY,
        });
        c.world.objects.insert(guid, o);
    }

    #[test]
    fn a_counter_that_said_busy_is_asked_again_once_the_cast_lands() {
        // +Vesperi's Use of Archmage Cindrue went out with a protection
        // still going up, ACE turned it away as YoureTooBusy, and the
        // run read the counter's silence as a counter that would not
        // trade: one retry spent on the same refusal, then the counter
        // held off as no use, the peas still in the pack.
        let holtburg = 0xA9B4_0019;
        let here = glam::Vec3::new(84.0, 7.1, 94.0);
        let Some(mut c) = standing_at(holtburg, here) else {
            return;
        };
        c.world.player_guid = Some(0x5000_0001);
        let me = c.player.as_ref().unwrap().world_position();
        let cfg = Growth::default();
        let now = Instant::now();
        let cindrue = 0x7a9b_4033;
        a_counter(&mut c, cindrue, "Archmage Cindrue", here);
        let mut run = run_to(Vec2::new(me.x, me.y), now);
        run.vendor = "Archmage Cindrue".into();
        run.phase = Phase::Opening {
            guid: cindrue,
            tries: 1,
            busy: None,
            asked_over: 0,
        };
        c.autoplay.growth.run = Some(run);
        // A cast is in the air, and the counter says so.
        c.autoplay.cast_sent = Some(now);
        c.chat_message(
            ac_net::messages::opcode::GAME_EVENT,
            &weenie_error(crate::YOURE_TOO_BUSY),
        );
        assert!(
            matches!(
                c.autoplay.growth.run.as_ref().unwrap().phase,
                Phase::Opening { busy: Some(_), .. }
            ),
            "the refusal was not heard as ours"
        );
        let sent = c.session.actions_sent();
        assert_eq!(c.grow_run_step(now, &cfg), Turn::Waited);
        assert!(
            c.town_run_view(now)
                .unwrap()
                .saying
                .contains("turned the Use away"),
            "the wait is not said"
        );
        // The counter's own UseDone frees the slot a moment after the
        // refusal, with the cast still going up: the ask waits a cast's
        // length after the refusal as well as for the slot.
        c.autoplay.cast_sent = None;
        let soon = now + Duration::from_secs(1);
        assert_eq!(c.grow_run_step(soon, &cfg), Turn::Waited);
        assert_eq!(
            c.session.actions_sent(),
            sent,
            "the Use went out into the cast"
        );
        // Long past the counter's time, with a cast in the air again:
        // no retry is spent and nothing is given up.
        let late = now + VENDOR_OPEN_TIMEOUT + Duration::from_secs(1);
        c.autoplay.cast_sent = Some(late);
        assert_eq!(c.grow_run_step(late, &cfg), Turn::Waited);
        assert_eq!(
            c.session.actions_sent(),
            sent,
            "the Use went out into the cast"
        );
        // The cast lands: the Use goes out again.
        c.autoplay.cast_sent = None;
        let landed = late;
        assert_eq!(c.grow_run_step(landed, &cfg), Turn::Acted);
        assert_eq!(
            c.session.actions_sent(),
            sent + 1,
            "the Use was not sent over"
        );
        let run = c.autoplay.growth.run.as_ref().expect("the run goes on");
        assert!(
            matches!(
                run.phase,
                Phase::Opening {
                    tries: 1,
                    busy: None,
                    asked_over: 1,
                    ..
                }
            ),
            "the retry was spent, or the refusal kept: {:?}",
            run.phase
        );
        assert_eq!(run.since, landed, "the counter's time did not start over");
        assert!(
            !c.autoplay.growth.skip_vendors.held(&spot(run.at), landed),
            "the counter was held off as no use"
        );
        // Silence from here on is the counter's own: the ordinary
        // retry, then given up.
        let quiet = landed + VENDOR_OPEN_TIMEOUT + Duration::from_secs(1);
        assert_eq!(c.grow_run_step(quiet, &cfg), Turn::Acted);
        assert!(matches!(
            c.autoplay.growth.run.as_ref().unwrap().phase,
            Phase::Opening { tries: 2, .. }
        ));
        let quieter = quiet + VENDOR_OPEN_TIMEOUT + Duration::from_secs(1);
        c.grow_run_step(quieter, &cfg);
        assert!(
            c.autoplay
                .growth
                .skip_vendors
                .held(&spot(Vec2::new(me.x, me.y)), quieter),
            "a counter that never answered was not held off"
        );
    }

    /// A caster standing in Holtburg with a wand in hand, the
    /// components, the skill and the mana for each of `spells`, and
    /// the server's clock known. The spells, in the order asked for.
    fn a_caster_knowing(now: Instant, spells: &[&str]) -> Option<(Client, Vec<u32>)> {
        let holtburg = 0xA9B4_0019;
        let here = glam::Vec3::new(84.0, 7.1, 94.0);
        let mut c = standing_at(holtburg, here)?;
        let me = 0x5000_0001;
        c.world.player_guid = Some(me);
        // The server's clock, without which nothing is ever due.
        let clock = ac_net::packet::build(
            ac_net::packet::Header {
                flags: ac_net::packet::flags::TIME_SYNC,
                ..Default::default()
            },
            &1000.0f64.to_le_bytes(),
            &[],
            0,
        );
        c.session.receive(&clock, now);
        assert!(c.session.server_time().is_some(), "the clock was not taken");
        let table = c.assets.spell_table().ok()?;
        let mut known = Vec::new();
        for name in spells {
            let (spell, sp) = table
                .spells
                .iter()
                .find(|(_, sp)| sp.name == *name)
                .map(|(id, sp)| (*id, sp.clone()))
                .unwrap_or_else(|| panic!("the spell table knows {name}"));
            c.world.stats.spells.push(spell);
            let skill = Client::school_skill(sp.school).expect("a school with a skill");
            if !c.world.stats.skills.iter().any(|s| s.id == skill) {
                c.world.stats.skills.push(ac_world::stats::Skill {
                    id: skill,
                    advancement: ac_world::stats::sac::TRAINED,
                    init_level: 300,
                    ..Default::default()
                });
            }
            known.push(spell);
        }
        c.world.stats.vitals[2].current = 500;
        const WAND: u32 = 0x8000_0102;
        c.world.objects.insert(
            WAND,
            ac_world::WorldObject {
                guid: WAND,
                name: "Training Wand".into(),
                item_type: item_type::CASTER,
                valid_locations: equip::HELD,
                wielder: Some(me),
                ..Default::default()
            },
        );
        let mapper = c.assets.spell_component_ids().ok()?;
        let mut guid = 0x8000_0200;
        for spell in &known {
            for component in c.current_formula(*spell) {
                let wcid = mapper
                    .component_wcid(component)
                    .expect("a component with a weenie");
                c.world.objects.insert(
                    guid,
                    ac_world::WorldObject {
                        guid,
                        name: format!("Component {component}"),
                        weenie_class_id: wcid,
                        stack_size: 20,
                        container: Some(me),
                        ..Default::default()
                    },
                );
                guid += 1;
            }
        }
        for spell in &known {
            assert!(
                matches!(c.can_cast(*spell), crate::magic::CastCheck::Ok),
                "the caster cannot cast {spell}: {:?}",
                c.can_cast(*spell)
            );
        }
        Some((c, known))
    }

    /// A caster standing in Holtburg with a wand in hand, the
    /// components and mana for Blade Protection Self, the server's
    /// clock known and no protection up: one buff due, urgent or not.
    fn a_caster_with_a_buff_due(now: Instant) -> Option<(Client, u32)> {
        let (mut c, known) = a_caster_knowing(now, &["Blade Protection Self I"])?;
        c.autoplay.config.buffs.auto = false;
        c.autoplay.config.buffs.spells = vec!["Blade Protection Self".into()];
        Some((c, known[0]))
    }

    /// A run standing at Archmage Cindrue's counter, the Use just
    /// gone out: the counter in view where the character stands, and
    /// the run in `phase` there.
    fn a_run_at_cindrue(c: &mut Client, phase: Phase, now: Instant) -> u32 {
        let me = c.player.as_ref().unwrap().world_position();
        let cindrue = 0x7a9b_4033;
        a_counter(
            c,
            cindrue,
            "Archmage Cindrue",
            glam::Vec3::new(84.0, 7.1, 94.0),
        );
        let mut run = run_to(Vec2::new(me.x, me.y), now);
        run.vendor = "Archmage Cindrue".into();
        run.phase = phase;
        c.autoplay.growth.run = Some(run);
        cindrue
    }

    /// The Opening phase as a run enters it.
    fn opening(guid: u32) -> Phase {
        Phase::Opening {
            guid,
            tries: 1,
            busy: None,
            asked_over: 0,
        }
    }

    #[test]
    fn buffs_wait_at_the_counter_and_go_up_on_the_walk_and_after() {
        // +Vesperi arrived at Archmage Cindrue with eight protections
        // lapsing and cast them one after another from the counter; the
        // buff pass held for a journey and for a fight, and standing at
        // a counter was neither.
        let now = Instant::now();
        let Some((mut c, _)) = a_caster_with_a_buff_due(now) else {
            return;
        };
        let me = c.player.as_ref().unwrap().world_position();
        let cindrue = 0x7a9b_4033;
        a_counter(
            &mut c,
            cindrue,
            "Archmage Cindrue",
            glam::Vec3::new(84.0, 7.1, 94.0),
        );
        // On the walk to town the urgent pass casts as it always did.
        let mut run = run_to(Vec2::new(me.x, me.y), now);
        run.vendor = "Archmage Cindrue".into();
        c.autoplay.growth.run = Some(run.clone());
        let sent = c.session.actions_sent();
        assert!(c.autoplay_buff(now, true), "no cast on the walk");
        assert!(c.session.actions_sent() > sent, "the cast did not go out");
        assert!(!c.autoplay.buffs_held_at_counter);
        // At the counter, from the Use going out, neither pass casts.
        let at_counter = |phase: Phase| {
            let mut run = run.clone();
            run.phase = phase;
            run
        };
        let mut then = now;
        for phase in [
            Phase::Opening {
                guid: cindrue,
                tries: 1,
                busy: None,
                asked_over: 0,
            },
            Phase::Appraising,
            Phase::Selling { sent: Vec::new() },
        ] {
            then += Duration::from_secs(2);
            c.autoplay.growth.run = Some(at_counter(phase.clone()));
            // The window is open once the counter has answered the Use.
            c.world.open_vendor =
                (!matches!(phase, Phase::Opening { .. })).then(|| window_of(cindrue));
            c.autoplay.cast_sent = None;
            let sent = c.session.actions_sent();
            assert!(!c.autoplay_buff(then, true), "an urgent cast at {phase:?}");
            assert!(!c.autoplay_buff(then, false), "a cast at {phase:?}");
            assert_eq!(
                c.session.actions_sent(),
                sent,
                "something went out at {phase:?}"
            );
            assert!(c.autoplay.buffs_held_at_counter, "the hold was not said");
        }
        // A window the player opened by hand holds the pass the same way.
        c.autoplay.growth.run = None;
        c.world.open_vendor = Some(window_of(cindrue));
        then += Duration::from_secs(2);
        assert!(!c.autoplay_buff(then, true), "a cast into an open window");
        // The window closed, the buff goes up.
        c.world.open_vendor = None;
        then += Duration::from_secs(2);
        let sent = c.session.actions_sent();
        assert!(
            c.autoplay_buff(then, true),
            "no cast once the window closed"
        );
        assert!(c.session.actions_sent() > sent, "the cast did not go out");
        assert!(
            !c.autoplay.buffs_held_at_counter,
            "the hold outlived the counter"
        );
    }

    #[test]
    fn urgent_buffs_go_up_in_a_fight_at_the_counter() {
        // A wandering monster catches the caster at the counter. The
        // fight outranks the run, which stays in its counter phase
        // throughout, and a hold that read the phase alone kept every
        // protection down for a window nobody was trading at until
        // the fight was over.
        let now = Instant::now();
        let Some((mut c, _)) = a_caster_with_a_buff_due(now) else {
            return;
        };
        let cindrue = 0x7a9b_4033;
        a_run_at_cindrue(&mut c, opening(cindrue), now);
        assert!(
            !c.autoplay_buff(now, true),
            "a cast at the counter with no fight on"
        );
        // Something is hitting the character.
        c.autoplay.last_hit_us = Some(Instant::now());
        let sent = c.session.actions_sent();
        assert!(
            c.autoplay_buff(now, true),
            "the urgent buff waited for the counter through the fight"
        );
        assert!(c.session.actions_sent() > sent, "the cast did not go out");
    }

    #[test]
    fn a_window_left_open_from_across_the_town_holds_no_buff() {
        // Nothing closes a window but this client, since the server
        // keeps none. One the player opened and walked away from read
        // as a counter at hand for the rest of the session, and no
        // buff went up again.
        let now = Instant::now();
        let Some((mut c, _)) = a_caster_with_a_buff_due(now) else {
            return;
        };
        let cindrue = 0x7a9b_4033;
        // The counter is a hundred metres off.
        a_counter(
            &mut c,
            cindrue,
            "Archmage Cindrue",
            glam::Vec3::new(184.0, 7.1, 94.0),
        );
        c.world.open_vendor = Some(window_of(cindrue));
        assert_eq!(c.counter_at_hand(), None);
        let sent = c.session.actions_sent();
        assert!(
            c.autoplay_buff(now, true),
            "held for a window left open across the town"
        );
        assert!(c.session.actions_sent() > sent, "the cast did not go out");
    }

    #[test]
    fn town_runs_switched_off_at_the_counter_close_its_window() {
        // A run let go for town runs being switched off left its
        // window open, and with town runs off no later run would ever
        // close it: a counter at hand for the rest of the session.
        let now = Instant::now();
        let Some(mut c) = standing_at(0xA9B4_0019, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.player_guid = Some(0x5000_0001);
        let cindrue = a_run_at_cindrue(&mut c, Phase::Selling { sent: Vec::new() }, now);
        c.world.open_vendor = Some(window_of(cindrue));
        assert_eq!(c.counter_at_hand().as_deref(), Some("Archmage Cindrue"));
        c.autoplay.config.growth.town_runs = false;
        c.autoplay_grow(now);
        assert!(
            !c.autoplay.growth.town_run_under_way(),
            "the run was not let go"
        );
        assert!(
            c.world.open_vendor.is_none(),
            "the run's window was left open"
        );
        assert_eq!(c.counter_at_hand(), None, "still at a counter");
    }

    #[test]
    fn dying_at_the_counter_ends_the_visit() {
        // The run stood at its counter through the death, the window
        // stayed open, and recovery -- which puts the buffs back before
        // the walk to the corpse -- waited on buffs that waited on the
        // counter.
        let now = Instant::now();
        let Some(mut c) = standing_at(0xA9B4_0019, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.player_guid = Some(0x5000_0001);
        let cindrue = a_run_at_cindrue(&mut c, Phase::Selling { sent: Vec::new() }, now);
        c.world.open_vendor = Some(window_of(cindrue));
        // Dead: health at zero on a sheet with a name and an Endurance
        // to draw a maximum from.
        c.world.stats.name = "Vesperi".into();
        c.world.stats.attributes[1].base = 100;
        c.world.stats.vitals[0].current = 0;
        assert!(c.is_dead(), "not dead");
        c.autoplay_recover(now);
        assert!(
            !c.autoplay.growth.town_run_under_way(),
            "the run stood at the counter through the death"
        );
        assert!(c.world.open_vendor.is_none(), "the window was left open");
        assert_eq!(c.counter_at_hand(), None);
        // A run still on its walk is kept: the journey is resumed after.
        let me = c.player.as_ref().unwrap().world_position();
        c.autoplay.growth.run = Some(run_to(Vec2::new(me.x, me.y), now));
        c.counter_left_behind(now);
        assert!(
            c.autoplay.growth.town_run_under_way(),
            "a run on its walk was stopped for a death"
        );
    }

    #[test]
    fn vitals_wait_at_the_counter() {
        // Only the buffs were held. The vitals pass runs every tick
        // and casts the moment the last cast lands, so a Revitalize
        // thrown from the counter never left it a gap to open its
        // window in: the measured failure, one reflex over.
        let now = Instant::now();
        let Some((mut c, _)) = a_caster_knowing(now, &["Revitalize Self I"]) else {
            return;
        };
        c.autoplay.config.survive.manage_mana = true;
        // The stamina is gone.
        c.world.stats.vitals[1].current = 0;
        // Away from any counter the top-up goes out.
        let sent = c.session.actions_sent();
        assert!(
            c.autoplay_vitals(now),
            "no Revitalize with the stamina gone"
        );
        assert!(c.session.actions_sent() > sent, "the cast did not go out");
        c.autoplay.cast_sent = None;
        // At the counter it waits.
        let cindrue = 0x7a9b_4033;
        a_run_at_cindrue(&mut c, opening(cindrue), now);
        let sent = c.session.actions_sent();
        assert!(!c.autoplay_vitals(now), "a top-up cast at the counter");
        assert_eq!(c.session.actions_sent(), sent, "something went out");
        // Unless a fight has come to it there.
        c.autoplay.last_hit_us = Some(Instant::now());
        assert!(
            c.autoplay_vitals(now),
            "the top-up waited for the counter through a fight"
        );
    }

    #[test]
    fn the_use_waits_for_a_cast_on_the_walk_in_to_land() {
        // The first Use went out the moment the character stood at
        // the counter, whatever was in the air: a protection put back
        // on the walk in met it with its recoil, and the counter turned
        // the Use away for it.
        let now = Instant::now();
        let Some(mut c) = standing_at(0xA9B4_0019, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.player_guid = Some(0x5000_0001);
        let cfg = Growth::default();
        a_run_at_cindrue(&mut c, Phase::Going, now);
        c.autoplay.cast_sent = Some(now);
        let sent = c.session.actions_sent();
        assert_eq!(c.grow_run_step(now, &cfg), Turn::Waited);
        assert_eq!(
            c.session.actions_sent(),
            sent,
            "the Use went out into the cast"
        );
        assert!(matches!(
            c.autoplay.growth.run.as_ref().unwrap().phase,
            Phase::Going
        ));
        // The cast lands: the counter is asked.
        c.autoplay.cast_sent = None;
        assert_eq!(c.grow_run_step(now, &cfg), Turn::Acted);
        assert_eq!(c.session.actions_sent(), sent + 1, "the Use was not sent");
        assert!(matches!(
            c.autoplay.growth.run.as_ref().unwrap().phase,
            Phase::Opening { .. }
        ));
    }

    #[test]
    fn a_refusal_past_the_busy_asks_is_the_counters_own() {
        // Past the asks a busy refusal earns the run is on the ordinary
        // clock, but a later refusal was still kept, and the status
        // promised an ask "once the cast lands" that was never coming.
        let now = Instant::now();
        let Some(mut c) = standing_at(0xA9B4_0019, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.player_guid = Some(0x5000_0001);
        let cindrue = 0x7a9b_4033;
        a_run_at_cindrue(
            &mut c,
            Phase::Opening {
                guid: cindrue,
                tries: 1,
                busy: None,
                asked_over: BUSY_ASKS,
            },
            now,
        );
        c.autoplay.growth.counter_said_busy(now);
        assert!(
            matches!(
                c.autoplay.growth.run.as_ref().unwrap().phase,
                Phase::Opening { busy: None, .. }
            ),
            "a refusal past the cap was kept"
        );
        let saying = c.town_run_view(now).unwrap().saying;
        assert!(
            saying.contains("waiting for"),
            "the status still promises an ask: {saying}"
        );
        // Under the cap it is heard.
        a_run_at_cindrue(
            &mut c,
            Phase::Opening {
                guid: cindrue,
                tries: 1,
                busy: None,
                asked_over: BUSY_ASKS - 1,
            },
            now,
        );
        c.autoplay.growth.counter_said_busy(now);
        assert!(matches!(
            c.autoplay.growth.run.as_ref().unwrap().phase,
            Phase::Opening { busy: Some(_), .. }
        ));
    }

    #[test]
    fn a_swing_in_the_air_does_not_hold_the_ask() {
        // ACE is never busy for a swing, and a swing's answer can go
        // missing: read raw, one lost AttackDone parked a busy wait in
        // Opening with no clock at all.
        let now = Instant::now();
        let Some(mut c) = standing_at(0xA9B4_0019, glam::Vec3::new(84.0, 7.1, 94.0)) else {
            return;
        };
        c.world.player_guid = Some(0x5000_0001);
        let cfg = Growth::default();
        let cindrue = 0x7a9b_4033;
        a_run_at_cindrue(
            &mut c,
            Phase::Opening {
                guid: cindrue,
                tries: 1,
                busy: Some(now),
                asked_over: 0,
            },
            now,
        );
        c.attack_pending = true;
        c.autoplay.cast_sent = None;
        let sent = c.session.actions_sent();
        let later = now + BUSY_REASK;
        assert_eq!(
            c.grow_run_step(later, &cfg),
            Turn::Acted,
            "the ask waited on a swing"
        );
        assert_eq!(
            c.session.actions_sent(),
            sent + 1,
            "the Use was not sent over"
        );
        assert!(matches!(
            c.autoplay.growth.run.as_ref().unwrap().phase,
            Phase::Opening {
                busy: None,
                asked_over: 1,
                ..
            }
        ));
    }

    #[test]
    fn the_hold_is_let_go_by_a_pass_that_left_early() {
        // The note's flag was cleared only on the way past the hold. A
        // pass that turned back before it -- the top-ups, in a fight,
        // out of combat only -- left the flag set after the counter,
        // and the next visit's wait went unsaid.
        let now = Instant::now();
        let Some((mut c, _)) = a_caster_with_a_buff_due(now) else {
            return;
        };
        let cindrue = 0x7a9b_4033;
        a_run_at_cindrue(&mut c, opening(cindrue), now);
        assert!(!c.autoplay_buff(now, true));
        assert!(c.autoplay.buffs_held_at_counter, "the hold was not said");
        // Off the counter and in a fight, the top-up pass turns back
        // at once.
        c.autoplay.growth.run = None;
        c.autoplay.config.buffs.out_of_combat_only = true;
        c.attack_target = Some(0xdead);
        assert!(!c.autoplay_buff(now, false));
        assert!(
            !c.autoplay.buffs_held_at_counter,
            "the flag outlived the counter"
        );
    }
}
