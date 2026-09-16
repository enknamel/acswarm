//! Playing the character on its own: keeping it alive, keeping its buffs
//! up, fighting what it is told to fight and taking the loot worth
//! taking.
//!
//! This is a set of rules, not a script: [`Config`] says what to do and
//! [`Client::tick_autoplay`] does the highest-priority thing that needs
//! doing this moment. In order:
//!
//! 1. **Stay alive**: below a fraction of health, use a healing kit or
//!    cast a healing spell; below a lower fraction, break off the fight.
//! 2. **Urgent buffs**: one about to run out goes back up before
//!    anything but healing -- a fight, a corpse, a journey. A lapsed buff
//!    can kill a character outright; a fight or a corpse can wait a cast.
//! 3. **Loot**: a corpse of something we killed is opened, the items
//!    the rules say to take are taken, and it is closed again.
//! 4. **Salvage**: items the rules tagged for salvage are salvaged by
//!    the team's best salvager, and carried to it by everyone else.
//! 5. **Fight**: pick the nearest creature that passes the name rules
//!    and attack it, with the weapon that suits it best.
//! 6. **Top up buffs**: in a quiet moment, recast anything that has
//!    run out or will soon.
//!
//! Loot is judged by the character's loot profile and nothing else
//! (`crate::profile`): the first rule that claims an item decides it,
//! and items are appraised first when a rule needs numbers
//! ([`judge_loot`]). A character whose profile is missing takes
//! nothing, which is the right way round for a client with no rules to
//! read. The two name lists here -- `Loot::always` and `Loot::never` --
//! are the player's own word and are read before any rule.
//!
//! Everything a rule keeps, salvages or sells is picked up, and what it
//! was taken for is written down against its guid (`Autoplay::tag`,
//! `crate::profile::Ledger`) for the salvage pass, the run to town, the
//! UI and scripts.
//!
//! Nothing here talks to the UI: the panel edits a [`Config`] and reads
//! [`Autoplay::status`].

use std::time::{Duration, Instant};

use crate::refusals;
use crate::{Client, Stance};
// The rule vocabulary lives in ac-loot; this file still speaks it.
pub use ac_loot::profile::LootAction;

// Autoplay's own rules, each in its own file below this one.
mod cast;
mod config;
mod fight;
pub mod growth;
mod hands;
mod hear;
mod journey;
mod ledger;
mod loot;
pub mod steps;
pub mod summoning;
mod team;
mod tidy;
mod vitals;

pub use cast::cast_problem;
pub use config::{Buffs, Config, Fight, Loot, Role, Style, Survive, Team};
pub use fight::critter::{critter, Critter, Hint, Seen};
pub use hands::ammo::choose_recipe;
#[cfg(doc)]
use hear::{arrived_unharmed, spell_attacker};
pub use ledger::retag::arrival_tag;
#[cfg(doc)]
use ledger::retag::{RETAG_EVERY, TAG_TIMEOUT};
pub use ledger::salvage::best_salvager;
#[cfg(doc)]
use loot::choose::answered_with_nothing;
pub(crate) use loot::choose::{CORPSE_LIFE, CORPSE_URGENT};
pub use loot::judge::{
    called_to, claim_on, judge_loot, skills_asked_of, Calling, Claim, Left, Taker,
};
use loot::owed::{corpse_within_reach, killed_by_us, whose_to_ask, Whose, KILL_SPOT};
pub(crate) use loot::room::Room;
pub(crate) use loot::take::refused_item;
pub(crate) use loot::walk::CorpseWalk;
pub(crate) use team::fellowship::FELLOWSHIP_FULL;
#[cfg(doc)]
use team::fellowship::{FOUNDING_WAIT, HELD_OFF_FIRST, RECRUIT_AGAIN, RECRUIT_FLOOR, YIELD_AFTER};
pub use team::follow::follow_break;
pub(crate) use team::quartermaster::{GIVE_EVERY, GIVE_REACH};
pub(crate) use team::shuts::Emptied;
pub use team::shuts::{
    judge_shut, shut_line, standing_by_line, taken_in_line, ShutFor, StandBy, TakenIn,
};
pub(crate) use team::turns::CLAIM_SETTLE;
pub use team::turns::{deal, Shut, Turn};
#[cfg(doc)]
use team::view::rival_leader;
pub use team::view::{Mate, TeamView};
#[cfg(doc)]
use tidy::TIDY_LADEN_WAIT;
pub use vitals::heal::SelfHeal;

/// A target that takes no damage for this long is let go.
const STALL_AFTER: Duration = Duration::from_secs(20);
/// Walking up to a target is working on it while each stretch brings the
/// character this much nearer than it has been (see [`came_nearer`]).
const APPROACH_PROGRESS: f32 = 1.0;
/// And left alone for this long afterwards.
const GIVE_UP_FOR: Duration = refusals::ATTACK_AGAIN;
/// How far off a creature that has stopped attacking can stand before a
/// character on the road lets it go and walks on (see
/// [`road_fight_over`]): beyond a swing's reach, so one still being hit
/// is finished rather than left at half health.
const ROAD_REACH: f32 = 8.0;
/// Casting or shooting this long from one spot with nothing landing:
/// the spot is no good, and the character closes in rather than going on.
const CLOSE_IN_AFTER: Duration = Duration::from_secs(8);
/// Nearer than this, closing in again achieves nothing: give up instead.
const MIN_STAND_OFF: f32 = 6.0;

/// Whether what has been thrown at a target has had long enough to get
/// there and has not: the first shot since anything last arrived went
/// out at `thrown`, more than [`CLOSE_IN_AFTER`] ago. Nothing thrown is
/// nothing missed.
fn nothing_arrived(thrown: Option<Instant>, now: Instant) -> bool {
    thrown.is_some_and(|t| now.duration_since(t) > CLOSE_IN_AFTER)
}

/// Whether the walk up to `guid`, now `distance` off, has come nearer
/// than the nearest yet (`best`, for whichever target it was) by
/// [`APPROACH_PROGRESS`].
fn came_nearer(best: Option<(u32, f32)>, guid: u32, distance: f32) -> bool {
    best.is_none_or(|(g, d)| g != guid || distance < d - APPROACH_PROGRESS)
}

/// How near to fight from after nothing has landed from `distance`: half
/// as far, never nearer than [`MIN_STAND_OFF`]. `None` when already that
/// close, and there is nowhere nearer worth trying.
fn closer_stand_off(distance: f32) -> Option<f32> {
    (distance > MIN_STAND_OFF + 1.0).then(|| (distance * 0.5).max(MIN_STAND_OFF))
}
/// The same note is not logged again within this.
const NOTE_EVERY: Duration = Duration::from_secs(5);
/// Least time between two attack orders.
const ATTACK_EVERY: Duration = Duration::from_millis(1200);

/// Swung at this recently, the character is in a fight whether or not
/// it chose one.
const UNDER_ATTACK: Duration = Duration::from_secs(4);

/// Whether `name` contains any of `list`, case-insensitively. An empty
/// list matches nothing.
pub fn name_matches(name: &str, list: &[String]) -> bool {
    let name = name.to_lowercase();
    list.iter()
        .any(|w| !w.trim().is_empty() && name.contains(&w.trim().to_lowercase()))
}

/// Whether a creature called `name` is one to fight.
pub fn wanted_target(name: &str, f: &Fight) -> bool {
    if name_matches(name, &f.avoid) {
        return false;
    }
    f.only.iter().all(|w| w.trim().is_empty()) || name_matches(name, &f.only)
}

/// Whether a fight taken on the road is over because the creature has
/// dropped out of it: the character is on its way somewhere, the
/// creature has not attacked it lately, is not walking at it or a mate,
/// nobody on the road is fighting it, and it stands `away` metres off,
/// beyond a swing's reach (`ROAD_REACH`).
///
/// On the road a character takes on only what attacks it (see
/// `Client::passing_by`), and a creature that swung once and then fell
/// behind -- the party outran it, it lost interest, it was never going
/// to keep up -- was chased until it died or until twenty seconds
/// without a hit gave it up (`STALL_AFTER`), the road forgotten
/// meanwhile. The fight ends when the creature stops following. One
/// still in reach is finished: leaving a creature at half health a
/// swing away is a chase the other way round.
pub fn road_fight_over(
    on_the_road: bool,
    attacking_us: bool,
    coming_at_us: bool,
    a_mate_is_on_it: bool,
    away: f32,
) -> bool {
    on_the_road && !attacking_us && !coming_at_us && !a_mate_is_on_it && away > ROAD_REACH
}

/// What the character is doing on its own right now.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Doing {
    #[default]
    Idle,
    Healing,
    Fighting,
    Looting,
    /// Salvaging, or carrying salvage to whoever does.
    Salvaging,
    Buffing,
    Debuffing,
    Helping,
    Following,
    /// Dead, or coming back from it (see `crate::recovery`).
    Recovering,
    /// On the way to a hunting ground.
    Traveling,
    /// On a run to town.
    Shopping,
    /// Stepping out of a spell's way (see `crate::dodge`).
    Dodging,
    /// Doing the Training Academy tutorial (see `crate::academy`).
    Training,
    /// Pouring loose stacks together (see `crate::pack`).
    Tidying,
}

impl Doing {
    pub fn label(self) -> &'static str {
        match self {
            Doing::Tidying => "tidying the pack",
            Doing::Idle => "waiting",
            Doing::Healing => "healing",
            Doing::Fighting => "fighting",
            Doing::Looting => "looting",
            Doing::Salvaging => "salvaging",
            Doing::Buffing => "buffing",
            Doing::Debuffing => "debuffing",
            Doing::Helping => "helping the team",
            Doing::Following => "following the leader",
            Doing::Recovering => "recovering from death",
            Doing::Traveling => "travelling",
            Doing::Shopping => "in town",
            Doing::Dodging => "dodging",
            Doing::Training => "in the Training Academy",
        }
    }
}

/// The running state of the rules.
#[derive(Default)]
pub struct Autoplay {
    pub config: Config,
    pub doing: Doing,
    /// Rooms of the dungeon we are in that have been stood in, and the
    /// one being walked to with the threshold aimed at (see
    /// `crate::explore`). Cleared when the dungeon has been walked, so
    /// that it is walked again rather than left.
    pub rooms_seen: std::collections::HashSet<u32>,
    /// Rooms that would not let us in: walked round, not through, until
    /// the dungeon is walked and everything is given another chance.
    pub rooms_shut: std::collections::HashSet<u32>,
    /// The room we set out from, the one we are walking into, and
    /// the point past its threshold we are aiming at.
    pub room_bound: Option<crate::explore::RoomWalk>,
    pub room_since: Option<Instant>,
    /// A line for the panel: "fighting Drudge Skulker".
    pub status: String,
    /// `Event::Autoplay`s not yet handed out: one per change of `doing`
    /// or `status`, taken by `Client::drain_events`.
    pub announced: Vec<crate::Event>,
    last_heal: Option<Instant>,
    last_attack: Option<Instant>,
    /// When a cast, or a sale or purchase at a counter, was sent and the
    /// server has not yet said it is done. Cleared by its answer
    /// (`UseDone`), which is what paces the next one.
    pub(crate) cast_sent: Option<Instant>,
    /// What the attack spells are being thrown at: a spell keeps no
    /// `attack_target` of its own the way a swing does, so the engine
    /// remembers what it is working on.
    casting_at: Option<u32>,
    /// The target the weapon in hand was chosen for, so it is chosen
    /// once a fight and not once a frame.
    pub(crate) armed_for: Option<u32>,
    /// Coming back from a death (see `crate::recovery`).
    pub recovery: crate::recovery::Recovery,
    /// Targets already made vulnerable this fight.
    vulned: Vec<u32>,
    /// Hard targets being softened, and how far along: 0 the
    /// vulnerability still to cast, 1 the imperil.
    softening: Vec<(u32, u8)>,
    last_vuln: Option<Instant>,
    last_buff: Option<Instant>,
    /// The buff pass has said it is waiting for the counter this
    /// visit, so it is not said again every tick (see
    /// `Client::autoplay_buff`).
    pub(crate) buffs_held_at_counter: bool,
    /// Enchantments put on items: `(item, category, when, seconds it
    /// lasts)`. An item's enchantments are not reported the way the
    /// character's own are, so the cast is remembered instead.
    item_buffs: Vec<(u32, u32, Instant, f32)>,
    /// The last note logged and when (see `note`).
    /// Notes said lately and when, so that two alternating notes are
    /// each said once per `NOTE_EVERY` rather than every frame.
    noted: Vec<(String, Instant)>,
    /// When the buffs were last gone through. The urgent pass runs
    /// every tick, and working out what is due walks the whole
    /// spellbook, so it is only done once a second.
    buffs_checked: Option<Instant>,
    /// The weapon put down to cast an urgent buff mid-fight, to be taken
    /// up again the moment the buffing is done.
    put_down: Option<u32>,
    /// The ammunition chosen for the target, to be wielded with the bow.
    wanted_ammo: Option<u32>,
    /// The shield to put on once a one-handed weapon is in hand.
    wanted_shield: Option<u32>,
    /// When the hands were last asked to change weapon.
    pub(crate) last_rewield: Option<Instant>,
    /// A weapon to wield as soon as the hands are empty.
    pub(crate) pending_wield: Option<u32>,
    /// The last item the server was asked to put in the hands and when,
    /// so that a refusal can be told from an answer to something else,
    /// and so the same ask is not sent again while the first is still
    /// in flight (see [`Autoplay::wield_in_flight`]).
    pub(crate) wield_asked: Option<(u32, Instant)>,
    /// Weapons the server has refused to wield, and for how long to
    /// leave each one alone (see `Client::hold_off_wield`).
    pub(crate) wield_refused: crate::did::Patience<u32>,
    /// A journey put down for a fight, to be picked up again after it.
    pub(crate) resume_trip: Option<glam::Vec2>,
    /// Whether that journey was a walk about the character's own ground
    /// rather than a road (see `Client::travel_about`): it is picked up
    /// again as what it was, and the walk past the road reads it while
    /// it waits.
    pub(crate) resume_about_the_ground: bool,
    /// The target being worked on, since when, and its health when
    /// last seen to drop: a target that takes no damage for a while is
    /// out of reach, and is let go.
    engaged: Option<(u32, Instant, f32)>,
    /// The nearest the walk up to the engaged target has come to it.
    approach_best: Option<(u32, f32)>,
    /// The summoning rules' own state (see `crate::summoning`).
    pub summoning: crate::summoning::State,
    /// Who has attacked the character lately, by name, and when each
    /// last did -- a blow landed or one evaded, since a creature that
    /// misses is attacking all the same, or a spell cast at it, landed
    /// or resisted. Fought wherever they stand, hunting area or not.
    /// All of them: kept as the last one only, a second attacker
    /// silenced the first, which was then walked past between its
    /// swings (see [`Autoplay::attacked_by`]).
    pub(crate) hit_by: Vec<(String, Instant)>,
    /// What the leader remembers between plans, when this character
    /// leads: the bodies it has dealt and the turns each of the others
    /// has had (see `crate::plan`).
    pub(crate) planner: crate::plan::Planner,
    /// The leader's latest plan as heard, and when: what this character
    /// fights and which body it opens, while the plan is fresh (see
    /// [`Autoplay::current_plan`]). A leader holds its own.
    pub(crate) orders: Option<crate::plan::Orders>,
    /// Since when the leader has been waiting for stragglers before
    /// moving the party on (see `Client::waits_for_stragglers`).
    pub(crate) straggling_since: Option<Instant>,
    /// The first shot thrown at a target since anything last got to it,
    /// and when: damage, a resist or an evasion clears it. The time spent
    /// walking to a clear shot, with nothing thrown, is not a miss.
    pub(crate) thrown: Option<(u32, Instant)>,
    /// The attack spell last cast, to tell whether what is being thrown
    /// can miss at all: only a projectile can.
    pub(crate) attack_spell: Option<u32>,
    /// A target not being hurt from where the character stands, and how
    /// near to fight it from now: a spell or an arrow the sight check
    /// lets through and something on the way stops.
    closing: Option<(u32, f32)>,
    /// Targets let go, and when, so they are left alone for a while.
    given_up: Vec<(u32, Instant)>,
    /// When ammunition was last made.
    last_craft: Option<Instant>,
    /// Bundles waiting to be used on each other once the character has
    /// dropped to peace mode: `(heads, shafts, when peace was asked)`.
    crafting: Option<(u32, u32, Instant)>,
    /// When stamina was last poured into mana or Revitalize cast.
    last_vital: Option<Instant>,
    /// The corpse being looted and when we started.
    /// The corpse being opened: which, since when, how long to allow
    /// (the server walks us to it, so a far one is slower), and how
    /// many times we have asked.
    corpse: Option<(u32, Instant, Duration, u32)>,
    /// How many of the asks to open that corpse came back with nothing
    /// at all (see [`answered_with_nothing`]). Kept across asking again,
    /// and started afresh with each corpse taken up.
    quiet_answers: u32,
    /// Which step claimed this tick, for the log and the panel.
    pub(crate) step: Option<&'static str>,
    /// The last "stood aside" said, so the same one is not said twice.
    aside_said: Option<String>,
    /// Corpses already emptied, and when each was written off: by this
    /// character, or by one of the others that found nothing on the body
    /// this one would take (see [`Autoplay::take_in_shuts`]).
    pub(crate) looted: Emptied,
    /// The bodies this character shut lately as emptied, what each is for
    /// the others, and when: said on the board (see
    /// [`Autoplay::shuts_to_say`]).
    shut_lately: Vec<(Shut, Instant)>,
    /// How many bodies this character has shut as emptied on the team:
    /// the number the next is said with ([`Shut::n`]).
    shuts_made: u32,
    /// When this character shut, as emptied, each body it opened first --
    /// one none of the others had said they shut -- lately: its turns at
    /// the bodies (see [`Autoplay::opened_first`]).
    first_opens: Vec<Instant>,
    /// Which of the others have said they shut which body, as `(body,
    /// player guid, which of its shuts)` (see [`Autoplay::take_in_shuts`]).
    /// A body one of them shut first is no turn of this character's, and
    /// nothing on it is left for one that has shut it already (see
    /// [`Autoplay::may_be_sent`]).
    shut_by: std::collections::BTreeSet<(u32, u32, u32)>,
    /// Fellows done with a body, as `(body, player guid)`: told by a shut
    /// that nothing on it is anything they would take, or stood by for
    /// here and never come (see [`Autoplay::stop_standing_by`]). Nothing on
    /// that body is left for them.
    done_with: std::collections::BTreeSet<(u32, u32)>,
    /// The bodies this character leaves to the fellows something on them
    /// was left for, before taking what it wants itself (see
    /// [`Autoplay::stands_by`]).
    standing_by: std::collections::BTreeMap<u32, StandBy>,
    /// When the character last landed a killing blow. Its body is owed
    /// from that moment, not from when the corpse turns up a second or
    /// two later: in that gap the next target used to be picked, and a
    /// busy spot never gave the loot a turn.
    pub(crate) last_kill: Option<Instant>,
    /// Where the character's kills fell, and when. A corpse that turns
    /// up at one is the character's to loot however far off, within the
    /// fight radius: a caster kills from forty metres, and looking only
    /// close by left every body it made at range on the ground.
    pub(crate) kill_spots: Vec<(glam::Vec3, Instant)>,
    /// Far corpses asked about to learn whose kill each was, while a
    /// creature this character summoned is about: its kills send no
    /// word (see `Client::autoplay_claim_pet_kills`).
    pub(crate) whose: Whose,
    /// When something last attacked the character, whether or not the
    /// blow or the spell landed. A fight that has come to it is fought
    /// first, body owed or not.
    pub(crate) last_hit_us: Option<Instant>,
    /// Corpses that would not open, and how long to leave each. A
    /// corpse is locked to the group that killed it until it has rotted
    /// a while, so this is usually a "later", not a "never", and the
    /// wait grows if it keeps saying no. How long a "later" is, when the
    /// server said why, is [`Autoplay::corpse_refused`]'s to say.
    pub(crate) shelved: crate::did::Patience<u32>,
    /// Corpses set aside for want of room to carry what is left on them,
    /// and what the lightest of that weighs. Such a body waits on room,
    /// not on a clock. Set aside for half a minute instead, a laden
    /// character went back to each one as its wait ran out, opened it,
    /// took nothing, and held the next fight for it every time.
    pub(crate) left_for_weight: std::collections::BTreeMap<u32, u32>,
    /// Weenie classes the server has refused to hand over because they
    /// can only be had so often. See `Client::loot_refused`.
    pub(crate) refused_kinds: crate::did::Patience<u32>,
    /// The walk to a corpse, if the looting set one going.
    /// A follower is walking after its leader with the same machinery,
    /// and that walk is not ours to cancel.
    pub(crate) walking_to: Option<CorpseWalk>,
    /// Corpse items we asked the server about.
    appraising: bool,
    /// What the rules said about each carried item taken as loot (or
    /// found in the pack afterwards), by guid: the salvage pass, the UI
    /// and scripts read it.
    /// What each item was picked up for, remembered across restarts
    /// (see `ac_loot::ledger`). The decision is made once, when the
    /// thing is taken, and this is where it is kept.
    pub ledger: ac_loot::Ledger,
    /// Carried items already looked at by the salvage pass, so an item
    /// is judged once, when it arrives. Empty until the first pass,
    /// which takes what is carried then as the baseline (nothing owned
    /// before the rules ran is salvaged behind the player's back).
    seen: std::collections::BTreeSet<u32>,
    baselined: bool,
    /// Arrivals waiting for their appraisal before the rules judge
    /// them, and since when.
    pending_tags: Vec<(u32, Instant)>,
    /// The profile the ledger's decisions were made under, by identity
    /// (the shelf replaces the whole thing on every edit). `None` before
    /// the first look; `Some(None)` for a character with no profile.
    judged_under: Option<Option<std::sync::Arc<crate::profile::Profile>>>,
    /// A re-judge of the whole pack is under way, since when.
    ///
    /// It is not one pass: an item a rule cannot judge until it is
    /// appraised has to wait for the server, so the pass runs again
    /// until nothing is waiting or it gives up ([`TAG_TIMEOUT`]).
    retagging: Option<Instant>,
    /// When the next of those passes is due ([`RETAG_EVERY`]).
    retag_due: Option<Instant>,
    /// The salvage batch sent, and when; refused batches and hand-offs
    /// are counted per item so a stubborn one is given up on.
    salvaging: Option<(Vec<u32>, Instant)>,
    handing: Option<(u32, Instant)>,
    refused: std::collections::BTreeMap<u32, u8>,
    last_salvage: Option<Instant>,
    /// The other characters being played, as the host last saw them.
    pub team: TeamView,
    /// Targets this character has landed its debuffs on.
    pub debuffed: Vec<u32>,
    /// What this character is short of, for the others to hand over.
    pub wants: Vec<String>,
    last_debuff: Option<Instant>,
    last_give: Option<Instant>,
    /// Hand-overs that have not gone through, by what was being handed
    /// over. Counting the money out into its own stack takes a moment
    /// and the server answers in its own time; this is what stops the
    /// asking from running away.
    give_tries: crate::did::Patience<u32>,
    /// The spot being walked to, how close it has been got to, and when
    /// that last improved. See `Client::reaching_too_long`.
    reaching: Option<(glam::Vec3, f32, Instant)>,
    pub(crate) last_merge: Option<Instant>,
    /// The pour asked for and not yet answered, and when it went out.
    /// Until the server has said, the counts the next pour would be
    /// chosen from are the ones from before this one (see
    /// `crate::pack::pour_answer`).
    pub(crate) pour: Option<(crate::pack::PourSent, Instant)>,
    /// When the pack was last looked over for a pour. Working out what
    /// is worth pouring copies every stack's name, and the answer
    /// cannot change faster than the server answers.
    pub(crate) tidy_looked: Option<Instant>,
    /// Tidying left alone until then, because weight is all that stands
    /// in its way ([`TIDY_LADEN_WAIT`]).
    pub(crate) tidy_laden_until: Option<Instant>,
    /// Emptying the corpse in front of us, as `ac-loot` sees it: what
    /// has been asked for, what will not come, how many have been
    /// taken. Started afresh for each body.
    loot_run: ac_loot::Run,
    /// Every body opened and thing taken since the client started, for
    /// the panel. Blargerton's log said "emptied" whether he took
    /// something or nothing, and a count tells a character with nothing
    /// worth taking from one that does not loot.
    pub loot_tally: ac_loot::Tally,
    /// When each corpse was first seen, so the ones about to rot can be
    /// emptied first. A corpse we never saw appear is taken as fresh.
    pub(crate) corpse_seen: Vec<(u32, Instant)>,
    /// When the last fellowship invitation went out, whoever it was to
    /// (see [`RECRUIT_FLOOR`]).
    last_recruit: Option<Instant>,
    /// When each mate was last asked into the fellowship, so that one
    /// that has not answered waits its turn while the others are asked
    /// (see [`RECRUIT_AGAIN`]).
    recruited: Vec<(u32, Instant)>,
    /// When this character asked for the fellowship it leads to be
    /// founded. It asks only once loot sharing has taken, so a
    /// fellowship it founded is the one fellowship it can vouch for:
    /// ACE reads the leader's "share fellowship loot" option once, in
    /// the `Fellowship` constructor, nothing re-reads it, and nothing
    /// on the wire says afterwards whether a fellowship shares loot.
    /// It is kept as a time because the server answers a founding with
    /// a fellowship, in its own time, and one ask is enough until then
    /// (see [`FOUNDING_WAIT`]).
    founded: Option<Instant>,
    /// Whether the leader has already said that the fellowship it is in
    /// may not share loot, so it says it once rather than every note.
    said_not_sharing: bool,
    /// Mates the server turned down an invitation to in words, by guid,
    /// each held off recruiting for a doubling wait (see
    /// [`HELD_OFF_FIRST`]). Forgotten the moment the mate is in.
    held_off: crate::did::Patience<u32>,
    /// Since when the team's rightful leader has been seen in a
    /// fellowship of its own while this character leads one it founded
    /// (see [`rival_leader`], [`YIELD_AFTER`]).
    outled_since: Option<Instant>,
    /// Where the journey after a far-off leader was bound, to plan
    /// again once it has moved on.
    follow_trip: Option<glam::Vec2>,
    /// No journey after the leader is planned before this: planning
    /// costs a search, and one that found no way is not tried again for
    /// a while.
    next_follow_plan: Option<Instant>,
    /// The growth rules' own state (see `crate::growth`).
    pub growth: crate::growth::State,
    /// The academy rule's own state (see `crate::academy`).
    pub academy: crate::academy::State,
    /// The corpse the academy rule is emptying, and since when.
    pub(crate) academy_corpse: Option<(u32, Instant)>,
    /// Doors the academy rule opened lately, and when.
    pub(crate) academy_doors: Vec<(u32, Instant)>,
    /// When the academy rule last asked for a weapon to be wielded.
    pub(crate) academy_armed: Option<Instant>,
}

impl Autoplay {
    /// The creature spells are being thrown at, if any: the magic
    /// fighter's counterpart to `Client::attack_target`.
    pub fn casting_at(&self) -> Option<u32> {
        self.casting_at
    }

    /// Something called `who` attacked the character at `now`: a blow,
    /// a miss or a spell (see [`Autoplay::hit_by`]). Each name is kept
    /// once, at its latest, and what has not attacked for a while is
    /// let go.
    pub(crate) fn attacked_by(&mut self, who: &str, now: Instant) {
        self.last_hit_us = Some(now);
        self.hit_by
            .retain(|(name, when)| name != who && now.duration_since(*when) < UNDER_ATTACK);
        self.hit_by.push((who.to_string(), now));
    }

    /// Let go of whatever is being fought: the spells' target and the
    /// engagement (the fleet view's "regroup" and "stop"; the caller
    /// clears `Client::attack_target` itself).
    pub fn drop_target(&mut self) {
        self.casting_at = None;
        self.engaged = None;
        self.closing = None;
    }

    /// How near to fight `guid` from, when it has not been hurt from
    /// further off (see `Client::stalled_on`).
    pub(crate) fn closing_on(&self, guid: u32) -> Option<f32> {
        self.closing.filter(|(g, _)| *g == guid).map(|(_, cap)| cap)
    }

    /// Something worth knowing that is not what the character is doing:
    /// logged, at most every few seconds for the same words, and the
    /// status left as it was. Said every tick it would drown the log
    /// and flip the status back and forth with whatever else is going on.
    pub(crate) fn note(&mut self, text: impl Into<String>, now: Instant) {
        let text = text.into();
        self.noted
            .retain(|(_, when)| now.duration_since(*when) < NOTE_EVERY);
        if !self.noted.iter().any(|(t, _)| *t == text) {
            tracing::info!("autoplay: {text}");
            self.noted.push((text, now));
        }
    }

    pub(crate) fn say(&mut self, doing: Doing, status: impl Into<String>) {
        let status = status.into();
        if self.doing != doing || self.status != status {
            tracing::info!("autoplay: {status}");
            self.announced.push(crate::Event::Autoplay {
                doing: format!("{doing:?}").to_lowercase(),
                text: status.clone(),
            });
        }
        self.doing = doing;
        self.status = status;
    }
}

impl Client {
    /// Run the rules for this moment. Call it once a frame; it does at
    /// most one thing.
    pub fn tick_autoplay(&mut self, now: Instant) {
        // The team's housekeeping runs whether or not the character
        // plays on its own: a leader played by hand still gathers the
        // fellowship, and everyone answers its invitations.
        if self.autoplay.config.team.enabled && self.world.player_guid.is_some() {
            self.autoplay_accept_invites();
            if self.autoplay.config.team.lead && !self.autoplay.config.enabled {
                self.autoplay_fellowship(now);
            }
        }
        // A window a run stopped waiting for is closed when it comes,
        // autoplay on or off (see `Client::autoplay_close_unwanted_window`).
        self.autoplay_close_unwanted_window(now);
        if !self.autoplay.config.enabled || self.world.player_guid.is_none() {
            // The status goes with autoplay -- unless the vendoring
            // panel is running a town run with autoplay off, whose turns
            // say what they are doing. Cleared here, every turn's line
            // was new again: a log line, an event and a bus post a
            // frame for the length of the walk.
            if !self.autoplay.status.is_empty()
                && !self.autoplay.config.enabled
                && !self.autoplay.growth.run_by_hand()
            {
                self.autoplay.doing = Doing::Idle;
                self.autoplay.status.clear();
            }
            return;
        }
        // The clocks read off the ground are wound before anything can
        // claim the tick. The four reflexes below return without
        // reaching the housekeeping, so a character that healed, dodged
        // or died wound neither of them for as long as that went on:
        // it came back from a death with a quiet clock from before the
        // death still running, and bodies that fell while it was busy
        // stayed for ever newly fallen (see
        // [`Client::autoplay_watch_the_ground`]).
        self.autoplay_watch_the_ground(now);
        // A spell on its way to us is stepped out of before anything
        // else, healing included (see `crate::dodge`).
        if self.autoplay_dodge(now) {
            return;
        }
        if self.autoplay_survive(now) {
            return;
        }
        // Dead, or on the way back from it: nothing else until the
        // corpse is dealt with (see `crate::recovery`).
        if self.autoplay_recover(now) {
            return;
        }
        // A new character in the Training Academy does the tutorial
        // before anything else (see `crate::academy`).
        if self.autoplay_academy(now) {
            return;
        }
        // Everything from here is a table rather than a chain, so the
        // order can be read, logged and tested rather than only obeyed
        // (see `crate::steps` and `docs/agent.md`).
        for chore in crate::steps::HOUSEKEEPING {
            chore.run(self, now);
        }
        // The reflexes run in their order, always. Nothing is weighed
        // against a spell already in the air.
        for step in crate::steps::reflexes() {
            let did = step.run(self, now);
            if did.acting() {
                self.autoplay.step = Some(step.name);
                return;
            }
            self.aside(step.name, &did, now);
        }
        // The goals are weighed. Most say nothing and take their place
        // in the table, which is the order they had; the ones with an
        // opinion can say that a corpse about to rot is worth more than
        // the next fight (see `crate::steps`).
        let mut goals = crate::steps::weigh(self, now);
        while let Some((step, _)) = goals.next(self, now) {
            let did = step.run(self, now);
            if did.acting() {
                self.autoplay.step = Some(step.name);
                return;
            }
            self.aside(step.name, &did, now);
        }
        self.autoplay.step = None;
        let doing = self.autoplay.doing;
        if doing != Doing::Idle {
            self.autoplay.say(Doing::Idle, "waiting");
        }
    }

    /// Say why a step stood aside, when it is worth saying.
    ///
    /// A character doing nothing is the hardest thing to account for
    /// from outside: it stands there and nobody can see what it is
    /// waiting on. `Waiting` is not worth a word -- it is the ordinary
    /// business of a tick -- but a step that is blocked or has given up
    /// is something the player wants to know, and once is enough: the
    /// reason is the same on every tick until it changes.
    fn aside(&mut self, step: &'static str, did: &crate::did::Did, now: Instant) {
        use crate::did::Did;
        let because = match did {
            Did::Blocked(b) | Did::Refused(b) => b,
            Did::Acting | Did::Done | Did::Waiting(_) => return,
        };
        let said = format!("{step}: {because}");
        if self.autoplay.aside_said.as_deref() == Some(said.as_str()) {
            return;
        }
        self.autoplay.aside_said = Some(said.clone());
        self.autoplay.note(said, now);
    }

    /// A hard fight: the creature has at least the team's threshold of
    /// health. With no team, nothing is hard in this sense (the solo
    /// rules soften by their own threshold).
    fn is_hard_fight(&self, guid: u32) -> bool {
        let team = &self.autoplay.config.team;
        if !team.enabled || team.hard_fight_health == 0 {
            return false;
        }
        self.creature_known(guid)
            .is_some_and(|c| c.health >= team.hard_fight_health)
    }

    /// The name of whoever should soften a hard target: the one with
    /// the highest Life Magic of those who can, this character
    /// included. Every session works this out from the same roster,
    /// so they agree without a word.
    fn softener(&self) -> Option<String> {
        let mine = (
            self.world.stats.name.clone(),
            self.life_magic(),
            self.can_soften(),
        );
        let best = self
            .autoplay
            .team
            .mates
            .iter()
            .map(|m| (m.name.clone(), m.life_magic, m.can_soften))
            .chain(std::iter::once(mine))
            .filter(|(_, _, can)| *can)
            .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)));
        best.map(|(name, _, _)| name)
    }

    /// Whether anyone on the team, this character included, has
    /// softened `guid`.
    fn softened_by_anyone(&self, guid: u32) -> bool {
        self.autoplay.debuffed.contains(&guid)
            || self
                .autoplay
                .team
                .mates
                .iter()
                .any(|m| m.debuffed.contains(&guid))
    }

    /// This character's Life Magic as it stands (skill 33).
    pub fn life_magic(&self) -> u32 {
        self.skill_now(33)
    }

    /// A skill as it stands right now, 0 when the sheet lacks it.
    fn skill_now(&self, id: u32) -> u32 {
        let stats = &self.world.stats;
        let Some(sk) = stats.skill(id) else {
            return 0;
        };
        let table = self.assets.skill_table().ok();
        stats.skill_current(sk, table.as_ref().and_then(|t| t.get(id)))
    }

    /// The names of what has attacked this character lately (see
    /// `Autoplay::attacked_by`), as it says them on the board.
    pub fn attackers_lately(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .autoplay
            .hit_by
            .iter()
            .filter(|(_, when)| when.elapsed() < UNDER_ATTACK)
            .map(|(who, _)| who.clone())
            .collect();
        names.sort_unstable();
        names.dedup();
        names
    }

    /// Knows a vulnerability or an imperil it could cast right now (a
    /// wand not in hand does not count against it).
    pub fn can_soften(&self) -> bool {
        let castable = |id: &u32| {
            matches!(
                self.can_cast(*id),
                crate::magic::CastCheck::Ok | crate::magic::CastCheck::NoCaster
            )
        };
        self.imperil_spells().iter().any(castable)
            || ac_world::elements::ALL
                .into_iter()
                .flat_map(ac_world::elements::vulnerabilities)
                .filter(|id| self.world.stats.spells.contains(id))
                .any(|id| castable(&id))
    }

    /// The imperils known: spells cast on another that lower its
    /// armour. Found by effect, so level eight's "Incantation of
    /// Imperil Other" counts without its name being read.
    pub fn imperil_spells(&self) -> Vec<u32> {
        let Ok(table) = self.assets.spell_table() else {
            return Vec::new();
        };
        self.world
            .stats
            .spells
            .iter()
            .copied()
            .filter(|id| {
                ac_world::buffs::effect(*id)
                    .is_some_and(|e| e.kind() == ac_world::buffs::kind::BODY_ARMOR && e.value < 0.0)
            })
            .filter(|id| table.get(*id).is_some_and(|s| s.needs_target()))
            .collect()
    }

    /// Something has attacked the character in the last few seconds --
    /// landed a blow, swung and missed, or cast a spell at it. A creature
    /// that keeps missing is in a fight with this character just as much
    /// as one that hits, and so is one that stands off and casts; the
    /// server says so each time.
    pub fn under_attack(&self) -> bool {
        self.autoplay
            .last_hit_us
            .is_some_and(|t| t.elapsed() < UNDER_ATTACK)
    }

    /// Whether something that has attacked the character lately is
    /// called `name`. The server names the attacker in what it sends,
    /// hit, miss or spell, so this is all there is to go on, and it is
    /// enough: it is what says a creature outside the hunting area, or
    /// one otherwise walked past, is a fight the character is already
    /// in. Two Cows, one of which kicked back, are both fought on it;
    /// the server does not say which.
    ///
    /// A creature's move target being this character is not asked here.
    /// It says something is walking at us, which our own pets, a fellow
    /// and a wandering townsfolk all do, and it carries no time, so it
    /// could not say when the fight began. The two notifications and the
    /// spell lines (see [`spell_attacker`]) say outright that an attack
    /// was aimed at this character, and when.
    pub(crate) fn hit_lately_by(&self, name: &str) -> bool {
        self.autoplay
            .hit_by
            .iter()
            .any(|(who, when)| who == name && when.elapsed() < UNDER_ATTACK)
    }

    /// Whether `o` is a creature to walk past because the character is
    /// on its way somewhere (see [`Self::on_its_way`]).
    ///
    /// Going somewhere is an errand of its own. The fighting is what the
    /// ground at the far end is for, and a character that stops for
    /// every drudge between here and there arrives an hour late or not
    /// at all. It answers what the character is *doing*, which is why it
    /// is a rule of its own rather than another clause in
    /// [`Self::a_critter`]: that one answers what a creature *is*, and
    /// the same Drudge is worth fighting once the walk is over.
    ///
    /// Two things outrank it, and in both the fight is already happening:
    /// the creature is attacking the character, with a swing or a spell,
    /// or one of the party on the road with it is fighting it (see
    /// [`Self::a_mate_on_the_road_is_on`]). The road does not get to decide
    /// either. Nothing has to be undone at the end of the road: the rule
    /// ends with the walk, and the character is fighting again the tick it
    /// arrives.
    ///
    /// `a_critter`'s other overrides are not this rule's. A name in
    /// "only these" says which kind to hunt, not where: every creature the
    /// pickers could choose already matches it, so as an override it
    /// switched the walking past off for anyone who kept the list, and a
    /// Drudge hunter stopped for every Drudge on the road to the Drudge
    /// ground. And a summoned creature picks its own fights. ACE's combat
    /// pet goes for the nearest monster it can see, whether or not that one
    /// is doing anything, so taking its target for a fight already on had
    /// the character stop for each creature its pet went for in turn --
    /// the very walk this rule is for.
    ///
    /// A creature standing in the character's path is not carved out,
    /// and that is a decision rather than an oversight. Nothing this
    /// client walks with can be stopped by one: its own physics collides
    /// with the landblock's static geometry and with nothing else, which
    /// is why a character walks straight through a closed door, and the
    /// steering plans its way round that same geometry. A creature is an
    /// object like the door is. And anything that could get in the way
    /// and matter is aggressive, which means it attacks -- the first
    /// override, and the character turns and fights it. If some ground
    /// proves otherwise, the walk's own four-minute timeout
    /// (`growth::WALK_TIMEOUT`) still ends it and another ground is
    /// chosen.
    pub(crate) fn passing_by(&self, o: &ac_world::WorldObject, cfg: &Fight) -> bool {
        if !cfg.walk_past_on_the_way || !self.on_its_way() {
            return false;
        }
        !(self.hit_lately_by(&o.name) || self.a_mate_on_the_road_is_on(o.guid))
    }

    /// Say what the character is walking past on its way somewhere: the
    /// nearest creature in reach that would be fought were the road not
    /// being walked (see [`Self::passing_by`]). Once per creature every
    /// few seconds, so a road can be read back from the log; nothing is
    /// asked when the character is not on a road.
    fn note_walked_past(&mut self, me: glam::Vec3, cfg: &Fight, underground: bool, now: Instant) {
        if !cfg.walk_past_on_the_way || !self.on_its_way() {
            return;
        }
        let mut standing = cfg.clone();
        standing.walk_past_on_the_way = false;
        let passed = self
            .world
            .objects
            .values()
            .filter(|o| self.would_fight(o, &standing, underground, now))
            .filter_map(|o| {
                let d = o.world_pos()?.distance(me);
                (d <= cfg.radius).then_some((d, o.name.clone()))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, name)) = passed {
            self.autoplay
                .note(format!("walking past {name} on the road"), now);
        }
    }

    /// Whether one of the party, on its way as well, is fighting `guid`.
    ///
    /// A character on the road takes on nothing but what attacks it, so
    /// this is a fight that came to one of its own. Without it the leader
    /// walked on while a follower turned to fight, and the follower was
    /// fetched after it and turned again, alone, each time it closed. A
    /// mate that is not on the road -- hunting back at the ground while
    /// this one goes to town -- is no reason to turn round.
    fn a_mate_on_the_road_is_on(&self, guid: u32) -> bool {
        self.autoplay
            .team
            .mates
            .iter()
            .any(|m| m.on_its_way && m.target == Some(guid))
    }

    /// Whether to go on at `guid` with the rest of the team: it is alive,
    /// and not one this character is walking past on its way somewhere
    /// (see [`Self::passing_by`]).
    ///
    /// Focus fire and the debuffer take the team's target off the board
    /// rather than choosing through [`Self::would_fight`], so they ask
    /// this instead. Without it a character setting off for town turned
    /// round for whatever the party back at the ground was hitting, from
    /// as far off as it could see it.
    pub(crate) fn joins_the_team_on(&self, guid: u32, cfg: &Fight) -> bool {
        self.world
            .objects
            .get(&guid)
            .is_some_and(|o| o.health.unwrap_or(1.0) > 0.0 && !self.passing_by(o, cfg))
    }

    /// Whether `o` is something this character would take on: a live
    /// creature, nobody's summoned pet and no player, inside the hunting
    /// area, of a kind it hunts, not a critter beneath it, not one the
    /// vitae has it keeping clear of, and not one it has given up
    /// reaching. Where it stands is the caller's business.
    ///
    /// Asked by the fight when it picks a target, and by the clock that
    /// says the ground has gone quiet (see
    /// [`Client::a_fight_in_sight`]), so the two agree about what
    /// counts as something to fight.
    pub(crate) fn would_fight(
        &self,
        o: &ac_world::WorldObject,
        cfg: &Fight,
        underground: bool,
        now: Instant,
    ) -> bool {
        o.item_type & ac_world::item_type::CREATURE != 0
            && o.object_desc_flags & ac_world::object_desc_flags::ATTACKABLE != 0
            && o.object_desc_flags & ac_world::object_desc_flags::PLAYER == 0
            && o.health.unwrap_or(1.0) > 0.0
            && !o.is_player
            // A summoned creature is its owner's, ours or anyone's.
            && o.pet_owner == 0
            // Inside the hunting area, or hitting the character.
            && self.area_allows(o, underground)
            && wanted_target(&o.name, cfg)
            // A Rabbit the character has outgrown is walked past.
            && !self.a_critter(o, cfg)
            // And so is whatever stands about while it is on its way
            // somewhere: the trip is the errand, not the road.
            && !self.passing_by(o, cfg)
            // With the vitae high, the hard ones and the killer wait.
            && !self.shy_of(o)
            // And one there is no getting to is not a fight on offer.
            && !self
                .autoplay
                .given_up
                .iter()
                .any(|(g, t)| *g == o.guid && now.duration_since(*t) < GIVE_UP_FOR)
    }

    /// Whether there is a fight to be had where the character stands:
    /// something it would take on within its fight radius, or one it has
    /// already joined.
    ///
    /// This is what says a spot has gone quiet, and it reads the world
    /// rather than the status line (see
    /// `Client::autoplay_watch_the_ground`).
    pub(crate) fn a_fight_in_sight(&mut self, now: Instant) -> bool {
        // Not in the world yet: nothing to say about the ground, and
        // nothing that should start a clock running on it.
        let Some(me) = self.player.as_ref().map(|p| p.world_position()) else {
            return true;
        };
        // A fight already joined counts wherever it has led. A creature
        // chased past the radius is still a fight, and walking off to
        // look for one in the middle of it is not looking for a fight.
        let joined = self
            .attack_target
            .or_else(|| self.autoplay.casting_at())
            .and_then(|g| self.world.objects.get(&g))
            .is_some_and(|o| o.health.unwrap_or(0.0) > 0.0);
        if joined {
            return true;
        }
        // Every session pays for this on every tick, so the cheap
        // question -- is it near enough -- is asked first.
        let underground = self.underground();
        let cfg = &self.autoplay.config.fight;
        self.world
            .objects
            .values()
            .filter(|o| {
                o.world_pos()
                    .is_some_and(|at| at.distance(me) <= cfg.radius)
            })
            .any(|o| self.would_fight(o, cfg, underground, now))
    }

    /// Whether a creature this character summoned is already on `guid`:
    /// its fight, and the character's to finish. Read off what the pet
    /// last walked at, which outlasts the position reports of the walk.
    fn a_pet_is_on(&self, guid: u32) -> bool {
        let Some(me) = self.world.player_guid else {
            return false;
        };
        self.world
            .objects
            .values()
            .any(|o| o.pet_owner == me && o.walked_at == Some(guid))
    }

    /// How this character is fighting right now: what its hands give.
    pub fn fighting_style(&self) -> Style {
        match self.combat_stance() {
            Stance::Melee => Style::Melee,
            Stance::Missile => Style::Missile,
            Stance::Magic => Style::Magic,
        }
    }

    /// Pick something to fight and attack it. True when fighting.
    pub(crate) fn autoplay_fight(&mut self, now: Instant) -> bool {
        let cfg = self.autoplay.config.fight.clone();
        self.autoplay_fight_as(now, &cfg)
    }

    /// The fight rule with the rules given (the academy points it at the
    /// creatures a task names).
    pub(crate) fn autoplay_fight_as(&mut self, now: Instant, cfg: &Fight) -> bool {
        let cfg = cfg.clone();
        if !cfg.enabled {
            return false;
        }
        let stance = self.fighting_stance_as(cfg.style);
        if stance == Stance::Magic {
            return self.autoplay_fight_with_spells(now, &cfg);
        }
        // Already on one that is still alive -- unless the leader's plan
        // has this character on another, and this one is not hitting us
        // (see `Client::ordered_elsewhere`).
        if let Some(t) = self.attack_target {
            if self.ordered_elsewhere(t, &cfg, now) {
                self.attack_target = None;
                self.autoplay.drop_target();
            }
        }
        if let Some(t) = self.attack_target {
            if self.stalled_on(t, now) {
                return false;
            }
            // And still here (see `fight_target_gone`).
            let underground = self.underground();
            let gone =
                self.fight_target_gone(t, underground) || self.left_behind_on_the_road(t, now);
            if gone {
                self.attack_target = None;
                self.autoplay.casting_at = None;
            }
            if let Some(o) = self.world.objects.get(&t).filter(|_| !gone) {
                if o.health.unwrap_or(1.0) > 0.0 {
                    let name = o.name.clone();
                    // A weapon choice put off for a swing in the air, or
                    // waiting on an appraisal, is made here. Nothing else
                    // asks again once the fight is joined, so what the
                    // buff pass left in the character's hands was what it
                    // fought the whole creature with.
                    if self.autoplay.armed_for != Some(t) {
                        self.arm_for(t, stance, &cfg);
                        if self.hands_changing(now) {
                            self.autoplay
                                .say(Doing::Fighting, format!("changing weapon for {name}"));
                            return true;
                        }
                    }
                    self.autoplay
                        .say(Doing::Fighting, format!("fighting {name}"));
                    // A bow with an empty ammunition slot shoots
                    // nothing, and the slot empties mid-fight.
                    if stance == Stance::Missile
                        && !self.ready_ammo()
                        && self.autoplay_craft_ammo(now)
                    {
                        return true;
                    }
                    return true;
                }
            }
        }
        // Finish what it killed before setting off after the next one --
        // and, leading, what the party killed: the next fight is not
        // walked off to while a body dealt to one of the others still
        // lies here (see `Self::party_owed_a_body`).
        if self.waits_for_a_corpse() {
            if let Some((who, what)) = self.party_owed_a_body(now) {
                self.autoplay
                    .say(Doing::Idle, format!("waiting for {who} to empty {what}"));
            }
            return false;
        }
        if self
            .autoplay
            .last_attack
            .is_some_and(|t| now.duration_since(t) < ATTACK_EVERY)
        {
            return false;
        }
        // Fighting at range: the bow needs something to shoot, and
        // when there is nothing to shoot, something is made.
        let missile = stance == Stance::Missile;
        if missile && !self.ready_ammo() && self.autoplay_craft_ammo(now) {
            return true;
        }
        let me = self.player.as_ref().map(|p| p.world_position());
        let Some(me) = me else { return false };
        // Hunting together: hit what the leader's plan says, and with no
        // plan what the team is hitting, unless it is something this
        // character is walking past on its way somewhere. The leader
        // takes its own orders; without a plan it picks for itself.
        let team = &self.autoplay.config.team;
        if team.enabled && team.focus_fire {
            let joined = self.ordered_target(&cfg, now).or_else(|| {
                if self.autoplay.team.leader {
                    return None;
                }
                self.autoplay
                    .team
                    .target()
                    .filter(|(guid, _)| self.joins_the_team_on(*guid, &cfg))
            });
            if let Some((guid, name)) = joined {
                if self.autoplay_plan_hard(guid, &name, now) {
                    return true;
                }
                self.remember_journey();
                self.arm_for(guid, stance, &cfg);
                if self.hands_changing(now) {
                    self.autoplay
                        .say(Doing::Fighting, format!("changing weapon for {name}"));
                    return true;
                }
                if missile && self.autoplay_approach(guid, &name, crate::dodge::How::Missile) {
                    return true;
                }
                self.enter_combat();
                self.attack(guid);
                self.autoplay.last_attack = Some(now);
                if missile {
                    self.throw_at(guid, now);
                }
                self.autoplay
                    .say(Doing::Fighting, format!("joining on {name}"));
                return true;
            }
        }
        // What cannot be judged yet is asked about, not attacked.
        self.ask_about_strangers(me, &cfg);
        let underground = self.underground();
        let target = self
            .world
            .objects
            .values()
            .filter(|o| self.would_fight(o, &cfg, underground, now))
            .filter_map(|o| {
                let p = o.world_pos()?;
                let d = p.distance(me);
                (d <= cfg.radius).then_some((d, o.guid, o.name.clone()))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let Some((_, guid, name)) = target else {
            self.note_walked_past(me, &cfg, underground, now);
            return false;
        };
        if self.autoplay_plan_hard(guid, &name, now) {
            return true;
        }
        self.remember_journey();
        self.arm_for(guid, stance, &cfg);
        // The hands are empty between the put and the wield, and a swing
        // sent into that gap is a punch (see `hands_changing`).
        if self.hands_changing(now) {
            self.autoplay
                .say(Doing::Fighting, format!("changing weapon for {name}"));
            return true;
        }
        // A bow refused for range shoots nothing: close in first (see
        // `crate::dodge`). A swing from too far the server walks us
        // in for.
        if missile && self.autoplay_approach(guid, &name, crate::dodge::How::Missile) {
            return true;
        }
        self.enter_combat();
        self.attack(guid);
        self.autoplay.last_attack = Some(now);
        if missile {
            self.throw_at(guid, now);
        }
        self.autoplay.say(
            Doing::Fighting,
            if missile {
                format!("shooting {name}")
            } else {
                format!("attacking {name}")
            },
        );
        true
    }

    /// Fighting with spells: pick a target the same way, then throw the
    /// first spell in the list that can be cast right now.
    ///
    /// A swing sets `attack_target` and the server keeps swinging; a
    /// spell does not, so this keeps its own target and re-casts on the
    /// pace of a cast rather than of a frame. The target is selected
    /// first because that is what `try_cast` throws at.
    fn autoplay_fight_with_spells(&mut self, now: Instant, cfg: &Fight) -> bool {
        if cfg.spells.is_empty() && self.attack_spells_known().is_empty() {
            self.autoplay.say(Doing::Idle, "no attack spells known");
            return false;
        }
        if self.wielded_caster().is_none() {
            self.autoplay.say(Doing::Idle, "no caster wielded");
            return false;
        }
        let alive = |c: &Client, g: u32| {
            c.world
                .objects
                .get(&g)
                .is_some_and(|o| o.health.unwrap_or(1.0) > 0.0)
        };
        // The leader's plan has this character on another, and the one
        // being cast at is not hitting us: let it go for the other.
        if let Some(g) = self.autoplay.casting_at {
            if self.ordered_elsewhere(g, cfg, now) {
                self.autoplay.casting_at = None;
            }
        }
        // Stay on the one already being fought while it lives, and is
        // taking damage.
        if let Some(g) = self.autoplay.casting_at {
            if self.stalled_on(g, now) {
                return false;
            }
        }
        // Out of the hunting area and not hitting us, or not here at
        // all any more, it is let go (see `fight_target_gone`).
        let underground = self.underground();
        let casting_at = self.autoplay.casting_at;
        let kept = casting_at.filter(|g| {
            alive(self, *g)
                && !self.fight_target_gone(*g, underground)
                && !self.left_behind_on_the_road(*g, now)
        });
        let target = match kept {
            Some(g) => Some(g),
            None => {
                self.autoplay.casting_at = None;
                if self.waits_for_a_corpse() {
                    None
                } else {
                    // What the plan says first, else what is nearest.
                    self.ordered_target(cfg, now)
                        .map(|(g, _)| g)
                        .or_else(|| self.pick_target(cfg))
                }
            }
        };
        let Some(guid) = target else {
            return false;
        };
        let name = self
            .world
            .objects
            .get(&guid)
            .map(|o| o.name.clone())
            .unwrap_or_default();
        self.autoplay.casting_at = Some(guid);
        // Paced to the casting, buffs included: the server queues one
        // spell sent over another and drops the next, so an attack
        // thrown on the heels of a buff would be the one dropped.
        if self.autoplay.cast_in_flight(now) {
            self.autoplay
                .say(Doing::Fighting, format!("fighting {name}"));
            return true;
        }
        // Behind the throttle, so choosing a wand costs no more than one
        // look per cast. Wielding takes a moment, so this cast still
        // goes out with the old one and the next with the new.
        self.arm_for(guid, Stance::Magic, cfg);
        self.select(Some(guid));
        if self.autoplay_plan_hard(guid, &name, now) {
            return true;
        }
        // Something with a lot of health is worth softening first: one
        // vulnerability for the element it is weakest to, then throw
        // that element at it for the rest of the fight.
        if self.autoplay_make_vulnerable(guid, &name, now) {
            return true;
        }
        // Of the spells that could be thrown this moment, the one this
        // creature is hurt most by. An Ice Golem takes nothing at all
        // from cold and full damage from fire, so the difference between
        // choosing well and throwing the first spell on the list is the
        // difference between a fight and a stalemate.
        // The spells on offer: the ones named, or, with none named,
        // every attack spell in the book. The game is a closed system
        // and the book says what can be thrown.
        let table = self.assets.spell_table().ok();
        let offered: Vec<(u32, String)> = if cfg.spells.is_empty() {
            self.attack_spells_known()
                .into_iter()
                .map(|id| {
                    let name = table
                        .as_ref()
                        .and_then(|t| t.get(id).map(|s| s.name.clone()))
                        .unwrap_or_default();
                    (id, name)
                })
                .collect()
        } else {
            cfg.spells
                .iter()
                .filter_map(|n| self.spell_by_name(n).map(|id| (id, n.clone())))
                .collect()
        };
        let ready: Vec<(u32, String)> = offered
            .into_iter()
            .filter(|(id, _)| matches!(self.can_cast(*id), crate::magic::CastCheck::Ok))
            .collect();
        let ids: Vec<u32> = ready.iter().map(|(id, _)| *id).collect();
        let wcid = self
            .world
            .objects
            .get(&guid)
            .map(|o| o.weenie_class_id)
            .unwrap_or(0);
        if let Some((spell, worth)) = ac_world::elements::best_spell(wcid, &name, &ids) {
            let spell_name = ready
                .iter()
                .find(|(id, _)| *id == spell)
                .map(|(_, n)| n.clone())
                .unwrap_or_default();
            // From too far off the server refuses the cast outright, and
            // from behind a wall or a rise the bolt strikes that instead:
            // close in first (see `crate::dodge`, `crate::aim`).
            if self.autoplay_approach(guid, &name, crate::dodge::How::Spell(spell)) {
                return true;
            }
            self.cast_paced(spell, now);
            self.note_fired(spell, now);
            self.autoplay.attack_spell = Some(spell);
            self.throw_at(guid, now);
            let element = ac_world::elements::spell_element(spell)
                .map(|e| e.name())
                .unwrap_or("");
            let said = if element.is_empty() || (0.99..=1.01).contains(&worth) {
                format!("casting {spell_name} at {name}")
            } else {
                format!("casting {spell_name} at {name} ({element} x{worth:.2})")
            };
            self.autoplay.say(Doing::Fighting, said);
            return true;
        }
        // Nothing castable: out of mana, out of components, or the
        // spells are not learnt. Say which, for the first of them.
        let first: Option<(String, u32)> = if cfg.spells.is_empty() {
            self.attack_spells_known().first().map(|id| {
                let name = table
                    .as_ref()
                    .and_then(|t| t.get(*id).map(|s| s.name.clone()))
                    .unwrap_or_default();
                (name, *id)
            })
        } else {
            cfg.spells
                .iter()
                .filter_map(|n| self.spell_by_name(n).map(|id| (n.clone(), id)))
                .next()
        };
        let why = first
            .map(|(n, id)| format!("{n}: {}", cast_problem(&self.can_cast(id))))
            .unwrap_or_else(|| "none of the attack spells is known".into());
        self.autoplay
            .say(Doing::Fighting, format!("cannot cast at {name} ({why})"));
        true
    }

    /// The team's plan for a hard target. True when this tick was spent
    /// on it: either softening it, because that is this character's
    /// job, or holding fire while a teammate does. False when the fight
    /// may go ahead: an ordinary target, or a hard one already softened.
    fn autoplay_plan_hard(&mut self, guid: u32, name: &str, now: Instant) -> bool {
        if !self.is_hard_fight(guid) || self.softened_by_anyone(guid) {
            return false;
        }
        let me = self.world.stats.name.clone();
        match self.softener() {
            Some(who) if who == me => {
                // Ours to soften: a vulnerability for its weakest element
                // and an imperil, then it is marked and the others go.
                if self.autoplay_soften(guid, name, now) {
                    return true;
                }
                // Nothing castable right now: do not hold everyone up.
                if !self.autoplay.debuffed.contains(&guid) {
                    self.autoplay.debuffed.push(guid);
                }
                false
            }
            Some(who) => {
                if !self.autoplay.config.team.wait_for_debuff {
                    return false;
                }
                self.autoplay.say(
                    Doing::Helping,
                    format!("waiting for {who} to soften {name}"),
                );
                true
            }
            // Nobody can: fight it as it is.
            None => false,
        }
    }

    /// Land the vulnerability and the imperil on `guid`, one cast a
    /// tick, and mark it softened when both are on (or neither can be
    /// cast). True while there is still one to cast.
    fn autoplay_soften(&mut self, guid: u32, name: &str, now: Instant) -> bool {
        // Waiting on the server's answer, not on a clock.
        if self.autoplay.cast_in_flight(now) {
            return true;
        }
        let stage = self
            .autoplay
            .softening
            .iter()
            .find(|(g, _)| *g == guid)
            .map(|(_, s)| *s)
            .unwrap_or(0);
        // Stage 0: the vulnerability. Stage 1: the imperil.
        let wcid = self
            .world
            .objects
            .get(&guid)
            .map(|o| o.weenie_class_id)
            .unwrap_or(0);
        let known = ac_world::elements::known(wcid, name);
        let candidates: Vec<u32> = if stage == 0 {
            known
                .and_then(|c| c.weakest_to())
                .map(ac_world::elements::vulnerabilities)
                .unwrap_or_default()
        } else {
            self.imperil_spells()
        };
        let spell = self.strongest_castable(&candidates);
        let advance = |this: &mut Self| {
            this.autoplay.softening.retain(|(g, _)| *g != guid);
            if stage == 0 {
                this.autoplay.softening.push((guid, 1));
            } else if !this.autoplay.debuffed.contains(&guid) {
                this.autoplay.debuffed.push(guid);
            }
        };
        match spell {
            Some(spell) => {
                if self.combat_stance() != Stance::Magic {
                    // Not with a swing out: putting the weapon away to
                    // reach for a wand cancels it. The softening keeps;
                    // hit the thing meanwhile.
                    if self.mid_attack() {
                        self.wait_for_the_swing();
                        return false;
                    }
                    // A wand for the casting; the arming code sorts the
                    // hands out again when the fight proper begins.
                    if !self.wield_for(Stance::Magic) {
                        // No caster to be had -- none carried, or the
                        // server keeps refusing the one there is. Held
                        // fire every tick this went on for ever and the
                        // target was never hit. Mark it and fight it as
                        // it is.
                        advance(self);
                        return false;
                    }
                    return true;
                }
                self.select(Some(guid));
                self.cast_paced(spell, now);
                let what = if stage == 0 {
                    "vulnerability"
                } else {
                    "imperil"
                };
                self.autoplay
                    .say(Doing::Debuffing, format!("softening {name}: {what}"));
                if stage == 0 && !self.autoplay.vulned.contains(&guid) {
                    self.autoplay.vulned.push(guid);
                }
                advance(self);
                stage == 0
            }
            None => {
                // Nothing for this stage: on to the next, or done.
                advance(self);
                stage == 0 && !self.imperil_spells().is_empty()
            }
        }
    }

    /// Of `ids`, the strongest this character knows, can cast, and
    /// that takes a target. Levels come from power, never from names.
    fn strongest_castable(&self, ids: &[u32]) -> Option<u32> {
        let table = self.assets.spell_table().ok()?;
        ids.iter()
            .copied()
            .filter(|id| self.world.stats.spells.contains(id))
            .filter(|id| table.get(*id).is_some_and(|s| s.needs_target()))
            .filter(|id| {
                matches!(
                    self.can_cast(*id),
                    crate::magic::CastCheck::Ok | crate::magic::CastCheck::NoCaster
                )
            })
            .max_by_key(|id| table.get(*id).map(|s| s.power).unwrap_or(0))
    }

    /// Cast a vulnerability for the target's weakest element, once per
    /// target and only on something with health enough for the spell to
    /// pay for itself. True when a spell went out this tick.
    fn autoplay_make_vulnerable(&mut self, guid: u32, name: &str, now: Instant) -> bool {
        let least = self.autoplay.config.fight.vuln_above_health;
        if least == 0 || self.autoplay.vulned.contains(&guid) || self.softened_by_anyone(guid) {
            return false;
        }
        // Only the recent ones are worth remembering; a long session
        // should not keep every creature it has ever softened.
        if self.autoplay.vulned.len() > 64 {
            self.autoplay.vulned.drain(..32);
        }
        let known = self.creature_known(guid);
        let Some(element) = crate::weapons::vulnerability_for(known, least) else {
            // Nothing worth softening: do not ask again for this one.
            self.autoplay.vulned.push(guid);
            return false;
        };
        // The strongest one we know and can pay for. Levels are read
        // from the spell's own power, never from its name: level seven
        // has names of its own ("Curse of the Blades") and level eight
        // mixes the two.
        let table = self.assets.spell_table().ok();
        let mut known_spells: Vec<(u32, u32)> = crate::weapons::vulnerability_spells(element)
            .into_iter()
            .filter(|id| self.world.stats.spells.contains(id))
            .filter_map(|id| {
                let sp = table.as_ref()?.get(id)?;
                // Only the ones cast on someone else: the self versions
                // of these make us take more, not them.
                sp.needs_target().then_some((id, sp.level()))
            })
            .collect();
        known_spells.sort_by_key(|(_, level)| std::cmp::Reverse(*level));
        let castable = known_spells
            .into_iter()
            .find(|(id, _)| matches!(self.can_cast(*id), crate::magic::CastCheck::Ok));
        let Some((spell, level)) = castable else {
            // Cannot do it now; do not keep trying every cast.
            self.autoplay.vulned.push(guid);
            return false;
        };
        self.cast_paced(spell, now);
        self.autoplay.vulned.push(guid);
        self.autoplay.last_vuln = Some(now);
        let said = format!(
            "making {name} vulnerable to {} (level {level})",
            element.name()
        );
        self.autoplay.say(Doing::Debuffing, said);
        true
    }

    /// Every attack spell in the spellbook that is thrown at a target:
    /// the ones the element table knows deal an element, strongest
    /// first. Which of them to throw is decided against the target.
    pub(crate) fn attack_spells_known(&self) -> Vec<u32> {
        let Ok(table) = self.assets.spell_table() else {
            return Vec::new();
        };
        let mut ids: Vec<u32> = self
            .world
            .stats
            .spells
            .iter()
            .copied()
            .filter(|id| ac_world::elements::spell_element(*id).is_some())
            .filter(|id| table.get(*id).is_some_and(|s| s.needs_target()))
            .collect();
        ids.sort_by_key(|id| std::cmp::Reverse(table.get(*id).map(|s| s.power).unwrap_or(0)));
        ids
    }

    /// Let the target go and leave it alone for [`GIVE_UP_FOR`], saying
    /// why once.
    fn give_up_target(&mut self, guid: u32, why: &str, now: Instant) {
        let name = self
            .world
            .objects
            .get(&guid)
            .map(|o| o.name.clone())
            .unwrap_or_else(|| format!("{guid:#010x}"));
        self.autoplay
            .note(format!("giving up on {name}: {why}"), now);
        self.autoplay
            .given_up
            .retain(|(_, t)| now.duration_since(*t) < GIVE_UP_FOR);
        self.autoplay.given_up.push((guid, now));
        self.autoplay.engaged = None;
        self.autoplay.closing = None;
        self.attack_target = None;
        self.autoplay.casting_at = None;
    }

    /// Note the target being worked on. True when it has taken no
    /// damage for too long and should be let go: it is out of reach,
    /// behind something, or not what it seems.
    fn stalled_on(&mut self, guid: u32, now: Instant) -> bool {
        let health = self
            .world
            .objects
            .get(&guid)
            .and_then(|o| o.health)
            .unwrap_or(1.0);
        match self.autoplay.engaged {
            Some((g, since, last)) if g == guid => {
                if health < last - 0.001 {
                    self.autoplay.engaged = Some((guid, now, health));
                    self.autoplay.thrown = None;
                    false
                } else if self.dodge.approaching == Some(guid) && self.walking_nearer(guid) {
                    // Walking up to it, and getting nearer: not stalled.
                    // The clock ran through the walk, and a target across a
                    // few of a dungeon's rooms was given up before the first
                    // spell went at it.
                    self.autoplay.engaged = Some((guid, now, health));
                    false
                } else if nothing_arrived(
                    self.autoplay
                        .thrown
                        .filter(|(g, _)| *g == guid)
                        .map(|(_, t)| t),
                    now,
                ) && self.close_in_on(guid)
                {
                    // A new spot to fight from gets its own chance.
                    self.autoplay.engaged = Some((guid, now, health));
                    self.autoplay.thrown = None;
                    false
                } else if now.duration_since(since) > STALL_AFTER {
                    self.give_up_target(guid, "no damage in a while", now);
                    true
                } else {
                    false
                }
            }
            _ => {
                self.autoplay.engaged = Some((guid, now, health));
                self.autoplay.closing = self.autoplay.closing.filter(|(g, _)| *g == guid);
                self.autoplay.approach_best = None;
                false
            }
        }
    }

    /// Whether `guid`, the creature being fought, is not there to fight
    /// any more.
    ///
    /// A target is chosen from within the fight radius, but nothing
    /// checked it was still within it afterwards -- so a creature the
    /// character walked away from, or left behind in the Academy,
    /// stayed its target for ever while it planned a journey to the
    /// other side of the world to swing at it. Anything further off than
    /// a walk is an object left over from somewhere the character has
    /// since left. The swing had this rule and the spell did not:
    /// teleported out of the Holtburg Dungeon mid-fight, a caster stood
    /// in town for ten minutes throwing Flame Arc at a Swamp Rat
    /// thirty-four kilometres away, "closing" on it by halves.
    ///
    /// Or gone out of the hunting area, and not hitting us: let it go.
    pub(crate) fn fight_target_gone(&self, guid: u32, underground: bool) -> bool {
        self.world
            .objects
            .get(&guid)
            .and_then(|o| o.world_pos())
            .zip(self.player.as_ref().map(|p| p.world_position()))
            .is_some_and(|(at, me)| at.distance(me) > crate::travel::WALKABLE)
            || !self.area_allows_guid(guid, underground)
    }

    /// Whether `guid`, the creature being fought, has dropped out of a
    /// fight taken on the road (see [`road_fight_over`]), and say so
    /// when it has. Asked of the fight in hand each tick, beside
    /// [`Self::fight_target_gone`].
    fn left_behind_on_the_road(&mut self, guid: u32, now: Instant) -> bool {
        let Some(o) = self.world.objects.get(&guid) else {
            return false;
        };
        let Some(away) = o
            .world_pos()
            .zip(self.player.as_ref().map(|p| p.world_position()))
            .map(|(at, me)| at.distance(me))
        else {
            return false;
        };
        let mates = &self.autoplay.team.mates;
        let ours = |g: u32| self.world.player_guid == Some(g) || mates.iter().any(|m| m.guid == g);
        let over = road_fight_over(
            self.on_its_way(),
            self.hit_lately_by(&o.name),
            o.walked_at.is_some_and(ours),
            self.a_mate_on_the_road_is_on(guid),
            away,
        );
        if over {
            let name = o.name.clone();
            self.autoplay.note(
                format!("letting {name} go: it stopped following on the road ({away:.0} m)"),
                now,
            );
        }
        over
    }

    /// Whether the walk up to `guid` has brought the character nearer to
    /// it than it has been in this fight (see [`came_nearer`]).
    fn walking_nearer(&mut self, guid: u32) -> bool {
        let Some(me) = self.player.as_ref().map(|p| p.world_position()) else {
            return false;
        };
        let Some(at) = self.world.objects.get(&guid).and_then(|o| o.world_pos()) else {
            return false;
        };
        let distance = me.distance(at);
        if !came_nearer(self.autoplay.approach_best, guid, distance) {
            return false;
        }
        self.autoplay.approach_best = Some((guid, distance));
        true
    }

    /// Nothing is landing on `guid` from where a ranged attacker stands:
    /// fight it from nearer. True when a nearer stand-off was set.
    ///
    /// The flight is worked out before a shot is thrown (see `crate::aim`),
    /// but only through the world that stands still: another creature in
    /// the way, a door, a target on the move can still take it -- and a
    /// caster that went on casting from the same spot for twenty seconds,
    /// spending components, and then gave up, never tried a step closer.
    ///
    /// Only something thrown can miss this way: a projectile spell or an
    /// arrow. Any other spell lands or is resisted where it is cast, and a
    /// resist or an evasion got there all the same (see
    /// [`arrived_unharmed`]). A melee attacker is walked in by the server
    /// already.
    fn close_in_on(&mut self, guid: u32) -> bool {
        let thrown = self.missile
            || (self.autoplay.casting_at == Some(guid)
                && self
                    .autoplay
                    .attack_spell
                    .is_some_and(|s| self.spell_flies(s)));
        if !thrown {
            return false;
        }
        let (Some(me), Some(at)) = (
            self.player.as_ref().map(|p| p.world_position()),
            self.world.objects.get(&guid).and_then(|o| o.world_pos()),
        ) else {
            return false;
        };
        let away = at.distance(me);
        let Some(cap) = closer_stand_off(away) else {
            return false;
        };
        let name = self
            .world
            .objects
            .get(&guid)
            .map(|o| o.name.clone())
            .unwrap_or_default();
        self.autoplay.note(
            format!("nothing landing on {name} from {away:.0} m; closing to {cap:.0} m"),
            Instant::now(),
        );
        self.autoplay.closing = Some((guid, cap));
        true
    }

    /// Where the creature just killed was standing: the one being fought
    /// when it has that name, else the nearest creature by that name.
    pub(crate) fn killed_at(&self, name: &str) -> Option<glam::Vec3> {
        let me = self.player.as_ref()?.world_position();
        let fought = [self.attack_target, self.autoplay.casting_at]
            .into_iter()
            .flatten()
            .filter_map(|g| self.world.objects.get(&g))
            .find(|o| o.name == name)
            .and_then(|o| o.world_pos());
        fought.or_else(|| {
            self.world
                .objects
                .values()
                .filter(|o| o.name == name && o.item_type & ac_world::item_type::CREATURE != 0)
                .filter_map(|o| o.world_pos())
                .min_by(|a, b| a.distance(me).total_cmp(&b.distance(me)))
        })
    }

    /// Claim the bodies a creature this character summoned killed.
    ///
    /// The server tells a player of a kill only when the player landed
    /// the last blow, so a body its creature finished had no kill spot
    /// and, past twenty metres, was passed by (see [`killed_by_us`]).
    /// While one of its creatures is about, each far corpse within the
    /// fight radius is appraised once, and one whose description names
    /// the character or its creature is claimed as its own kill, where it
    /// lies.
    pub(crate) fn autoplay_claim_pet_kills(&mut self, now: Instant) {
        let Some(me_guid) = self.world.player_guid else {
            return;
        };
        if self.world.objects.values().any(|o| o.pet_owner == me_guid) {
            self.autoplay.whose.pet_out(now);
        }
        let objects = &self.world.objects;
        self.autoplay.whose.tidy(|g| objects.contains_key(&g));
        // The answers that are in, whether or not a creature is still out.
        let me_name = &self.world.stats.name;
        let answers: Vec<(u32, bool)> = self
            .autoplay
            .whose
            .out()
            .filter_map(|g| {
                let desc = self
                    .appraisals
                    .get(&g)?
                    .string(ac_net::messages::Appraisal::STRING_LONG_DESC);
                Some((g, desc.is_some_and(|d| killed_by_us(d, me_name))))
            })
            .collect();
        for (guid, ours) in answers {
            self.autoplay.whose.answered(guid);
            let Some((at, corpse)) = self
                .world
                .objects
                .get(&guid)
                .filter(|_| ours)
                .and_then(|o| Some((o.world_pos()?, o.name.clone())))
            else {
                continue;
            };
            tracing::info!("autoplay: {corpse} ({guid:#010x}) was a kill of ours; looting it");
            self.autoplay.kill_spots.push((at, now));
            self.autoplay.last_kill = Some(now);
        }
        // A character that does not loot owes no body, and one with no
        // creature about has had every kill of its own told to it.
        if !self.autoplay.whose.pet_lately(now) || self.loot_profile().is_none() {
            return;
        }
        let Some(me) = self.player.as_ref().map(|p| p.world_position()) else {
            return;
        };
        let radius = self.autoplay.config.fight.radius;
        let ask: Vec<u32> = self
            .world
            .objects
            .values()
            .filter(|o| o.object_desc_flags & ac_world::object_desc_flags::CORPSE != 0)
            .filter(|o| !self.autoplay.whose.has_asked(o.guid))
            .filter(|o| {
                !self.autoplay.looted.contains(&o.guid) && !self.autoplay.shelved.held(&o.guid, now)
            })
            .filter(|o| {
                o.world_pos()
                    .is_some_and(|at| whose_to_ask(me, at, radius, &self.autoplay.kill_spots))
            })
            .filter(|o| !self.corpse_is_someone_elses(&o.name))
            .map(|o| o.guid)
            .collect();
        if ask.is_empty() {
            return;
        }
        for &g in &ask {
            self.autoplay.whose.ask(g, now);
        }
        self.appraise_many(ask);
    }

    /// The nearest creature the name rules allow, within the radius.
    fn pick_target(&mut self, cfg: &Fight) -> Option<u32> {
        let underground = self.underground();
        let me = self.player.as_ref()?.world_position();
        let now = Instant::now();
        // A follower fights beside its leader, not wherever a monster
        // happens to be.
        let leader_at = self.followed_leader().map(|m| m.world);
        let fight_radius = self.autoplay.config.team.fight_radius.max(1.0);
        // What cannot be judged yet is asked about, not attacked.
        self.ask_about_strangers(me, cfg);
        let candidates: Vec<(u32, glam::Vec3)> = self
            .world
            .objects
            .values()
            .filter(|o| self.would_fight(o, cfg, underground, now))
            .filter_map(|o| {
                let at = o.world_pos()?;
                let near_leader = leader_at.is_none_or(|l| at.distance(l) <= fight_radius);
                (at.distance(me) <= cfg.radius && near_leader).then_some((o.guid, at))
            })
            .collect();
        // The nearest one we can actually hit: one behind a wall is
        // taken only when nothing is in sight, and then the fight rules
        // walk round to it.
        let how = if self.missile {
            crate::dodge::How::Missile
        } else {
            crate::dodge::How::Melee
        };
        candidates
            .into_iter()
            .map(|(guid, at)| {
                let seen = self.shot_clears(guid, how);
                ((!seen) as u8, at.distance(me), guid)
            })
            .min_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)))
            .map(|(_, _, g)| g)
    }
}

#[cfg(test)]
mod tests;
