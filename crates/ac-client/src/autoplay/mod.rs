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

use serde::{Deserialize, Serialize};

#[cfg(test)]
use crate::refusals::refused;
use crate::refusals::{self, RecruitRefusal, Refusal};
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
use loot::choose::WILL_NOT_SPLIT;
pub(crate) use loot::choose::{CORPSE_LIFE, CORPSE_URGENT};
pub use loot::judge::{
    called_to, claim_on, judge_loot, skills_asked_of, Calling, Claim, Left, Taker,
};
use loot::judge::{takers_with, would_take, Fellow};
use loot::owed::{corpse_within_reach, killed_by_us, whose_to_ask, Whose, KILL_SPOT};
pub(crate) use loot::room::Room;
pub(crate) use loot::take::refused_item;
use loot::take::{REACH_GIVE_UP, REACH_PROGRESS};
pub(crate) use loot::walk::CorpseWalk;
use team::view::rival_leader;
pub use team::view::{Mate, TeamView};
#[cfg(doc)]
use tidy::TIDY_LADEN_WAIT;
use vitals::buffs::BUFF_EVERY;
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
/// Close enough to hand something over (ACE's use radius, with room).
const GIVE_REACH: f32 = 2.0;
/// Least time between two attack orders.
const ATTACK_EVERY: Duration = Duration::from_millis(1200);
/// A leader further off than twice the following distance (and at least
/// this) is followed before anything else, a fight included; nearer,
/// the fight comes first. Following is the follower's job.
const FOLLOW_BREAK: f32 = 10.0;

/// The character options a teammate keeps on, by the name the option
/// table knows them by (see `crate::options`). Every one of them is
/// something the server checks before it will let the party work as
/// one; the reasons are with the code that turns them on.
const TEAM_OPTIONS: [&str; 4] = [
    "accept fellowship",
    "automatically accept fellowship",
    "let other players give you items",
    "share fellowship loot",
];

/// The shortest gap between two fellowship invitations. Nine characters
/// arriving together are nine invitations, and they used to go out one
/// every five seconds -- forty-one seconds before the last one was in,
/// which is the very stretch in which nobody may loot anyone else's
/// kill. ACE has no rate limit on recruiting that I can find: the only
/// refusal is against a busy member (`Entity/Fellowship.cs`,
/// `fellow_busy_no_recruit`). That is read from the source and not
/// tested against a live server, so a small gap is kept rather than
/// firing nine invitations into one frame.
const RECRUIT_FLOOR: Duration = Duration::from_millis(500);

/// How long the leader waits for the server to answer a founding
/// before asking for one again. The answer is the fellowship itself,
/// arriving as a full update; until it comes there is nothing to say
/// whether the ask was heard, and asking twice would be two
/// fellowships.
const FOUNDING_WAIT: Duration = Duration::from_secs(5);

/// How far off a mate may stand and still be asked in, metres. The
/// server sets no distance on recruiting; this is about asking the ones
/// that are here rather than one still walking in from the last town.
const RECRUIT_RANGE: f32 = 25.0;

/// How long one invitee waits before being asked again. Nothing comes
/// back from an invitation that was turned down for a busy member, and
/// the news that one was accepted comes the long way round -- the mate
/// tells the board it is in a fellowship -- so a wait is the only way
/// to tell "not yet" from "never". It is the invitee that waits, not
/// the leader: the others are asked meanwhile.
const RECRUIT_AGAIN: Duration = Duration::from_secs(5);

/// How long an invitee the server turned down is left alone before it
/// is asked again: the table's wait for a refused recruit (see
/// `refusals::answer`). Until the server's words were read, two mates
/// were asked ten times each every [`RECRUIT_AGAIN`] and never came.
const HELD_OFF_FIRST: Duration = refusals::RECRUIT_HELD_OFF;

/// How long the team's rightful leader must be seen in a fellowship of
/// its own before this character gives up the one it founded. The
/// board's word on a mate is up to a round old and the world's on a
/// fellowship comes in its own time, so for a moment after that mate
/// quits this fellowship its row still says it is in one and the world
/// says it is not: a few rounds tell that from a second fellowship.
const YIELD_AFTER: Duration = Duration::from_secs(3);

/// How many a fellowship holds, the leader counted (ACE
/// `Entity/Fellowship.cs`, `MaxFellows`). A team of ten is one too
/// many, and the tenth is not asked.
const MAX_FELLOWS: usize = 9;

/// WeenieError for an invitation into a full fellowship (ACE
/// `WeenieError.YourFellowshipIsFull`).
pub(crate) const FELLOWSHIP_FULL: u32 = 0x041e;

/// Who to ask into the fellowship next, out of the mates standing by:
/// the nearest one that has not been asked in the last
/// [`RECRUIT_AGAIN`] and is not being held off after a refusal in words
/// (`held_off`, see [`HELD_OFF_FIRST`]). `None` when there is nobody to
/// ask, or when the last invitation went out inside [`RECRUIT_FLOOR`].
///
/// `waiting` is (guid, metres away), `asked` is when each invitee was
/// last sent an invitation.
fn next_invitee(
    waiting: &[(u32, f32)],
    asked: &[(u32, Instant)],
    held_off: &crate::did::Patience<u32>,
    last_sent: Option<Instant>,
    now: Instant,
) -> Option<u32> {
    if last_sent.is_some_and(|t| now.duration_since(t) < RECRUIT_FLOOR) {
        return None;
    }
    waiting
        .iter()
        .filter(|(guid, _)| {
            !held_off.held(guid, now)
                && !asked
                    .iter()
                    .any(|(g, t)| g == guid && now.duration_since(*t) < RECRUIT_AGAIN)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(guid, _)| *guid)
}

/// How often one character hands something to another. The server
/// takes one give at a time and answers in its own time.
const GIVE_EVERY: Duration = Duration::from_millis(700);

/// Swung at this recently, the character is in a fight whether or not
/// it chose one.
const UNDER_ATTACK: Duration = Duration::from_secs(4);

/// How long a mate's claim on a body is believed (see
/// [`TeamView::working`]). A claimant that stalled or was dragged into
/// a fight must not hold a body for the whole five minutes it lies
/// there.
///
/// Longer than the loot rules keep at one body (`ac_loot::run::KEEP_AT_IT`,
/// forty-five seconds), and by enough to cover a walk back to one that
/// drifted out of reach. At twenty seconds a character working a body
/// full of loot lost its claim while the body was still open on the
/// server, and the other eight converged on a container the server had
/// already handed out and were refused one by one.
const CLAIM_STALE: Duration = Duration::from_secs(75);

/// Two claims made within this of each other are the same moment.
///
/// Each session ages its own claim on its own clock and hears the
/// others' up to a board round late (`ac_plugin::team::SAY_EVERY`, half
/// a second), so nothing finer can be told apart and two sessions asked
/// which of them claimed a body first would both answer "I did". Inside
/// this window the lower player guid settles it instead, which is an
/// answer both of them reach (see [`TeamView::outranks_our_claim`]).
const SAME_MOMENT: Duration = Duration::from_millis(1500);

/// How long a newly fallen body is settled by the tie-break rather than
/// by the claims (see [`TeamView::opens_first`]).
///
/// A claim is said every half second (`ac_plugin::team::SAY_EVERY`) and
/// heard a frame later; a body is chosen within a tick of falling. A
/// second covers the round trip, and after it the claims are the truth.
const CLAIM_SETTLE: Duration = Duration::from_secs(1);

/// How long a body shut as emptied goes on being said on the board (see
/// [`Autoplay::shuts_to_say`]). A mate takes the word in for good the
/// first time it hears it, a board round after it is said
/// (`ac_plugin::team::SAY_EVERY`, half a second), so this only has to
/// outlast a mate that missed a few rounds: one quiet for six seconds is
/// dropped from the roster anyway.
const SHUT_SAID_FOR: Duration = Duration::from_secs(20);
/// How many bodies shut as emptied are said at once, the newest. A
/// character in a party of nine shuts one a minute or so.
const SHUTS_SAID: usize = 16;

/// How long a body opened first counts as a turn at the bodies (see
/// [`TeamView::opens_first`]). Long enough to hold a round of turns in a
/// party of nine killing eight a minute; short enough that one just come
/// to the ground, with no turns yet, is not dealt every body until it has
/// caught up with the rest.
const DEAL_WINDOW: Duration = Duration::from_secs(120);
/// How many shuts heard of are remembered (see [`Autoplay::has_shut`]),
/// the oldest bodies forgotten first. The bodies are forgotten as they go
/// (see [`Autoplay::forget_corpses_gone`]); this only bounds a character
/// that never looks.
const SHUT_BY_KEPT: usize = 512;
/// How many fellows done with a body are remembered (see
/// [`Autoplay::done_with`]): a party of nine, each done with as many
/// bodies as there are shuts remembered.
const DONE_WITH_KEPT: usize = SHUT_BY_KEPT * 9;

/// How long a character stands by a body for the fellow something on it
/// was left for (see [`StandBy`]), at most. That fellow may be fighting,
/// or have other bodies left for it first; one that has not come by then
/// is not coming, and what was left for it is taken by whoever else wants
/// it, with the whole of a body's five minutes still to spare.
const STAND_BY_FOR: Duration = Duration::from_secs(60);

/// How long a body emptied is remembered as emptied (see
/// [`Autoplay::looted`]). No monster's body lasts anywhere near an hour
/// ([`CORPSE_LIFE`]), and ACE hands a released guid out again once it has
/// been free for six hours (`GuidManager`, `recycleTime`): remembered for
/// the whole of a long session, a new body that came with an old one's
/// guid was taken as emptied and never opened.
const EMPTIED_KEPT: Duration = Duration::from_secs(60 * 60);

/// How far from its leader a follower keeping `keep` metres may stray
/// before following comes before everything else.
pub fn follow_break(keep: f32) -> f32 {
    (2.0 * keep).max(FOLLOW_BREAK)
}
/// Up to this far the follower walks straight for the leader, letting
/// the steering find the way; further (the leader took a portal) a
/// journey is planned.
const FOLLOW_WALK: f32 = 120.0;

/// What a character that shut a body as emptied says about it to the
/// others, each by player guid (see [`judge_shut`]).
///
/// Every body shut as emptied is said, whomever it is done for, so that
/// the others know who has shut it: nothing on it is sent back to one of
/// those (see `Autoplay::may_be_sent`), and going back to it is no turn.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shut {
    /// The body.
    pub body: u32,
    /// Which of the shutter's shuts this is, counting up from its start.
    /// The same shut is said on every row for a while; a second shut of
    /// the same body, once it has been opened again, is a new word.
    pub n: u32,
    /// Those that would take nothing still on it, read by their own skills
    /// with the shared rules: they write it off for good.
    pub done_for: Vec<u32>,
    /// Those that would take only what is left on it for one of
    /// `left_for`: they leave it to those first (see [`StandBy`]).
    pub stand_by: Vec<u32>,
    /// Those something on it was left for in particular, the best at what
    /// a rule asks (see [`called_to`]).
    pub left_for: Vec<u32>,
}

/// A character as the turns at a newly fallen body read it (see
/// [`TeamView::opens_first`]): each mate from its row on the board, and
/// this character from the same word about itself, so that every session
/// reaches the same answer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Turn {
    pub guid: u32,
    pub world: glam::Vec3,
    /// It is at another body, or on its way to one.
    pub looting: bool,
    /// It is fighting something.
    pub fighting: bool,
    /// Its pack is neither full nor laden (see `logistics::Supplies`).
    pub room: bool,
    /// How many bodies it opened first lately ([`Mate::opened_first`]).
    pub opened: u16,
}

/// Where the character `who` comes in the deal for the body `body`, among
/// characters that have had as many turns lately (see
/// [`TeamView::opens_first`]).
///
/// Every session works the same number out of the same two guids, so the
/// order is agreed without a word said; and it is a different order for
/// every body, so bodies falling together go to different characters. The
/// mix is written out by hand (splitmix64's finish) rather than taken from
/// a std hasher, which may change from build to build and is seeded afresh
/// in every process.
pub fn deal(body: u32, who: u32) -> u64 {
    let mut z = ((u64::from(body) << 32) | u64::from(who)).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// What a body shut as emptied is for this character and for the fellows
/// judged at the shut (see [`judge_shut`], [`Shut`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShutFor {
    /// Where the body lies, which whoever stands by for a fellow measures
    /// that fellow's reach from.
    pub at: glam::Vec3,
    pub done_for: Vec<u32>,
    pub stand_by: Vec<u32>,
    pub left_for: Vec<u32>,
    /// This character left on it something its own rules would take, for
    /// one of `left_for`: it stands by for them too, rather than writing
    /// the body off.
    pub waits: bool,
}

/// What a body this character is shutting as emptied is for it and for
/// each of the `judged` (see [`Shut`]), judged on the things `lying` on it
/// (`None` for one not described yet) by the rules the party shares, read
/// with what each fellow said about itself.
///
/// A thing is left for one fellow in particular when the rules mean it for
/// the best at what they ask (see [`called_to`]) among this character
/// (`me`) and the `sendable`, the judged that things may be sent to. Then:
/// - a fellow that would take nothing still on the body is done with it,
///   whatever was left for whom;
/// - one that would take only things left for others stands by for them;
/// - the rest open it by their own lights, the ones things were left for
///   among them.
///
/// Each fellow is judged as carrying none of anything, its pack not being
/// known here, so a rule that stops at a count takes: at worst that costs
/// it an open that finds nothing.
///
/// A fellow that would take a thing left for another is never told the
/// body is done. Told it was, it wrote the body off for good; and when the
/// one the thing was left for carried its fill already, or died, or
/// followed its leader away, the thing lay there until the body rotted.
pub fn judge_shut(
    lying: &[Option<Left>],
    profile: &crate::profile::Profile,
    me: Taker,
    judged: &[Fellow],
    sendable: &[Fellow],
    salvager: Option<u32>,
) -> ShutFor {
    let meant: Vec<Option<u32>> = lying
        .iter()
        .map(|thing| {
            let left = thing.as_ref()?;
            if sendable.is_empty() {
                return None;
            }
            let me = Taker {
                held: left.held,
                ..me
            };
            let takers = takers_with(me, sendable);
            called_to(&left.stats, left.id, profile, &takers, salvager).filter(|to| *to != me.guid)
        })
        .collect();
    let mut shut = ShutFor::default();
    for to in meant.iter().flatten() {
        if !shut.left_for.contains(to) {
            shut.left_for.push(*to);
        }
    }
    for (guid, name, sheet) in judged {
        let (mut takes, mut takes_its_own) = (false, false);
        for (thing, to) in lying.iter().zip(&meant) {
            let wanted = thing
                .as_ref()
                .is_none_or(|left| would_take(left, profile, sheet, name, 0));
            if wanted {
                takes = true;
                takes_its_own |= to.is_none_or(|to| to == *guid);
            }
        }
        if !takes {
            shut.done_for.push(*guid);
        } else if !takes_its_own {
            shut.stand_by.push(*guid);
        }
    }
    shut.waits = lying.iter().zip(&meant).any(|(thing, to)| {
        to.is_some()
            && thing
                .as_ref()
                .is_some_and(|left| would_take(left, profile, me.sheet, me.name, left.held))
    });
    shut
}

/// What the log says when the rules shut a body as emptied: who shut it
/// (`who`), how many things it took, which of the others it is done for,
/// which it left things for, and which stand by for those.
///
/// The log of a nine-character run named nobody: "use Corpse of Biaka"
/// five times inside 140 milliseconds, and no telling who opened it
/// first or who came back to it for nothing.
pub fn shut_line(
    who: &str,
    corpse: &str,
    guid: u32,
    took: u32,
    done_for: &[String],
    left_for: &[String],
    stand_by: &[String],
) -> String {
    let mut line = format!("autoplay: {who} shut {corpse} ({guid:#010x}), took {took}");
    if !done_for.is_empty() {
        line += &format!("; done for {}", done_for.join(", "));
    }
    if !left_for.is_empty() {
        line += &format!("; left for {}", left_for.join(", "));
    }
    if !stand_by.is_empty() {
        line += &format!("; {} standing by", stand_by.join(", "));
    }
    line
}

/// What the log says when a body one of the others shut (`by`) is taken
/// as emptied for this character (`me`), which then leaves it alone.
pub fn taken_in_line(me: &str, corpse: &str, guid: u32, by: &str) -> String {
    format!("autoplay: {me}: {corpse} ({guid:#010x}) done for me by {by}")
}

/// What the log says when this character (`me`) stands by a body one of
/// the others shut (`by`) for the fellows something on it was left for
/// (`on`).
pub fn standing_by_line(me: &str, corpse: &str, guid: u32, by: &str, on: &[String]) -> String {
    format!(
        "autoplay: {me}: {corpse} ({guid:#010x}) left for {} by {by}; standing by",
        on.join(", ")
    )
}

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

/// A body this character leaves to the fellows something on it was left
/// for, before it takes what it wants off it itself (see
/// `Autoplay::stands_by`).
#[derive(Clone, Debug, PartialEq)]
pub struct StandBy {
    /// Where the body lies, which those fellows' reach is measured from.
    pub at: glam::Vec3,
    /// The fellows things on it were left for ([`Shut::left_for`]).
    pub on: Vec<u32>,
    /// Since when.
    pub since: Instant,
}

/// The bodies emptied, and when each was written off (see
/// [`Autoplay::looted`]).
#[derive(Clone, Debug, Default)]
pub(crate) struct Emptied(Vec<(u32, Instant)>);

impl Emptied {
    /// Whether the body `guid` has been emptied.
    pub(crate) fn contains(&self, guid: &u32) -> bool {
        self.0.iter().any(|(g, _)| g == guid)
    }

    /// Write the body `guid` off as emptied at `now`, once.
    pub(crate) fn push(&mut self, guid: u32, now: Instant) {
        if !self.contains(&guid) {
            self.0.push((guid, now));
        }
    }

    /// Forget the bodies written off [`EMPTIED_KEPT`] or longer before
    /// `now`, long rotted, before their guids can come back on others.
    pub(crate) fn forget_old(&mut self, now: Instant) {
        self.0
            .retain(|(_, when)| now.saturating_duration_since(*when) < EMPTIED_KEPT);
    }

    /// How many bodies are remembered as emptied.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// What a character took in from a shut one of the others said (see
/// `Autoplay::take_in_shuts`), for the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TakenIn {
    /// The body `body` is done for this character, by the word of `by`.
    Done { body: u32, by: String },
    /// This character stands by the body `body`, by the word of `by`, for
    /// the fellows `on`.
    StandBy { body: u32, by: String, on: Vec<u32> },
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

    /// What becomes of a corpse the loot rules have shut, by what they
    /// said about it. Only one they finished with is done with. One
    /// that would not give up its contents is set aside and tried again
    /// later, as the rules meant: marking every shut corpse looted wrote
    /// those off for good, with their loot still on them.
    ///
    /// One shut for want of room to carry the rest (`left_for_weight`, the
    /// lightest thing left on it) waits on room rather than on a clock
    /// (see `Autoplay::left_for_weight`).
    ///
    /// One finished with says on the board what it is for the others
    /// (`shut`, see `Client::shut_for`). Only one finished with: a body set
    /// aside, or left for its weight, still has on it what somebody wanted.
    ///
    /// And one finished with is done with here, unless something on it
    /// that this character's rules would take was left for a fellow better
    /// at what they ask: then it stands by for that fellow (see
    /// [`Autoplay::stands_by`]), and takes the thing itself if the fellow
    /// never comes.
    fn corpse_shut(
        &mut self,
        guid: u32,
        did: &crate::did::Did,
        left_for_weight: Option<u32>,
        shut: ShutFor,
        now: Instant,
    ) {
        match (did, left_for_weight) {
            (crate::did::Did::Done, _) => {
                self.left_for_weight.remove(&guid);
                if shut.waits && !shut.left_for.is_empty() {
                    self.standing_by.insert(
                        guid,
                        StandBy {
                            at: shut.at,
                            on: shut.left_for.clone(),
                            since: now,
                        },
                    );
                } else {
                    self.standing_by.remove(&guid);
                    self.looted.push(guid, now);
                }
                // A turn at the bodies, unless one of the others shut it
                // first, or this one stood by it for another: then it was
                // left for this one, not dealt to it.
                let shut_before = self
                    .shut_by
                    .range((guid, 0, 0)..=(guid, u32::MAX, u32::MAX))
                    .next()
                    .is_some()
                    || self
                        .done_with
                        .range((guid, 0)..=(guid, u32::MAX))
                        .next()
                        .is_some();
                if !shut_before {
                    self.first_opens
                        .retain(|t| now.saturating_duration_since(*t) < DEAL_WINDOW);
                    self.first_opens.push(now);
                }
                self.say_shut(guid, shut, now);
            }
            (_, Some(burden)) => {
                self.left_for_weight.insert(guid, burden);
            }
            (_, None) => {
                self.left_for_weight.remove(&guid);
                self.shelved.note(guid, did, now);
            }
        }
        self.let_go_of_corpse();
    }

    /// Say on the board what the body `guid`, shut as emptied, is for the
    /// others (see [`Autoplay::shuts_to_say`]). Every one, whomever it is
    /// done for: one said only when it was done for somebody left the rest
    /// not knowing who had shut it, so going back to it counted as a turn
    /// and things on it were left for the one that had shut it already. A
    /// body shut again is said afresh, not twice. Off the team there is
    /// nobody to say it to.
    fn say_shut(&mut self, guid: u32, shut: ShutFor, now: Instant) {
        self.shut_lately.retain(|(said, when)| {
            said.body != guid && now.saturating_duration_since(*when) < SHUT_SAID_FOR
        });
        if !self.config.team.enabled {
            return;
        }
        let n = self.shuts_made;
        self.shuts_made = n.wrapping_add(1);
        self.shut_lately.push((
            Shut {
                body: guid,
                n,
                done_for: shut.done_for,
                stand_by: shut.stand_by,
                left_for: shut.left_for,
            },
            now,
        ));
        let over = self.shut_lately.len().saturating_sub(SHUTS_SAID);
        self.shut_lately.drain(..over);
    }

    /// The bodies this character shut lately as emptied, and what each is
    /// for the others: what it says about itself on the board (see
    /// [`Mate::shut`]). The newest `SHUTS_SAID`, each for
    /// `SHUT_SAID_FOR`.
    pub fn shuts_to_say(&self, now: Instant) -> Vec<Shut> {
        self.shut_lately
            .iter()
            .filter(|(_, when)| now.saturating_duration_since(*when) < SHUT_SAID_FOR)
            .map(|(said, _)| said.clone())
            .collect()
    }

    /// Take in what the others said about the bodies they shut, each shut
    /// once (see [`Shut`]), `me` being this character's player guid and
    /// `at_of` where a body lies, if it is in sight. Returns what was taken
    /// in, for the log.
    ///
    /// - Whoever shut it is noted: going back to it is no turn, and nothing
    ///   on it is left for that one (see [`Autoplay::may_be_sent`]). So are
    ///   the fellows it is done for.
    /// - A body done for this character is written off for good.
    /// - One this character is to stand by is left to the fellows things
    ///   on it were left for, for as long as one of them could still come
    ///   (see [`Autoplay::stands_by`]).
    /// - Otherwise the body is this character's to open by its own lights,
    ///   whatever an older word said: the newest shut of a body says what
    ///   is left on it.
    ///
    /// Nine characters opened 83 bodies 697 times, and 42% of those opens
    /// took nothing: the others opened a body one of them had emptied, to
    /// find it so. Written into `looted`, the word reaches everything that
    /// asks whether a body is done with (see
    /// [`Autoplay::corpse_waiting`]), and it outlasts the row it came on,
    /// which goes once its mate has been quiet a while.
    ///
    /// Off the team nothing is taken in: the rules there see nobody. With
    /// its own rules off, a character only notes who shut what: a body
    /// written off then, by a word about a character played by hand, was
    /// walked past once the player turned the rules back on beside it.
    pub(crate) fn take_in_shuts(
        &mut self,
        me: u32,
        now: Instant,
        at_of: impl Fn(u32) -> Option<glam::Vec3>,
    ) -> Vec<TakenIn> {
        if !self.config.team.enabled || me == 0 {
            return Vec::new();
        }
        let said: Vec<(u32, String, glam::Vec3, Shut)> = self
            .team
            .mates
            .iter()
            .flat_map(|m| {
                m.shut
                    .iter()
                    .map(|s| (m.guid, m.name.clone(), m.world, s.clone()))
            })
            .collect();
        let mut heard = Vec::new();
        for (by, name, where_it_was, shut) in said {
            if !self.shut_by.insert((shut.body, by, shut.n)) {
                continue;
            }
            self.done_with
                .extend(shut.done_for.iter().map(|g| (shut.body, *g)));
            if !self.config.enabled || self.looted.contains(&shut.body) {
                continue;
            }
            if shut.done_for.contains(&me) {
                // As its own emptying would: nothing waits on room for it.
                self.standing_by.remove(&shut.body);
                self.left_for_weight.remove(&shut.body);
                self.looted.push(shut.body, now);
                heard.push(TakenIn::Done {
                    body: shut.body,
                    by: name,
                });
            } else if shut.stand_by.contains(&me) {
                // The shutter stood over it a moment ago.
                let at = at_of(shut.body).unwrap_or(where_it_was);
                self.standing_by.insert(
                    shut.body,
                    StandBy {
                        at,
                        on: shut.left_for.clone(),
                        since: now,
                    },
                );
                heard.push(TakenIn::StandBy {
                    body: shut.body,
                    by: name,
                    on: shut.left_for,
                });
            } else {
                self.standing_by.remove(&shut.body);
            }
        }
        // The server hands out rising ids: the lowest body is oldest.
        while self.shut_by.len() > SHUT_BY_KEPT {
            self.shut_by.pop_first();
        }
        while self.done_with.len() > DONE_WITH_KEPT {
            self.done_with.pop_first();
        }
        heard
    }

    /// Whether this character leaves the body `guid` to the fellows
    /// something on it was left for (see [`StandBy`]): for at most
    /// [`STAND_BY_FOR`], and only while one of them could still come for
    /// it (see [`Mate::could_come_for`]) and has not shut it since. One
    /// dead, gone from the board, out of reach, without room or not
    /// looting is not waited on.
    pub(crate) fn stands_by(&self, guid: u32, now: Instant) -> bool {
        self.standing_by.get(&guid).is_some_and(|s| {
            now.saturating_duration_since(s.since) < STAND_BY_FOR
                && s.on.iter().any(|g| {
                    !self.has_shut(guid, *g)
                        && self
                            .team
                            .mates
                            .iter()
                            .any(|m| m.guid == *g && m.could_come_for(s.at))
                })
        })
    }

    /// Stop standing by the bodies no fellow is coming for any more (see
    /// [`Autoplay::stands_by`]). Each is this character's to open again,
    /// and nothing on it is left for the fellows it stood by for: one that
    /// looked able and never came would be left it again, and stood by
    /// for again, until the body rotted.
    pub(crate) fn stop_standing_by(&mut self, now: Instant) {
        let over: Vec<u32> = self
            .standing_by
            .keys()
            .copied()
            .filter(|g| !self.stands_by(*g, now))
            .collect();
        for body in over {
            if let Some(stood) = self.standing_by.remove(&body) {
                self.done_with.extend(stood.on.iter().map(|g| (body, *g)));
            }
        }
    }

    /// Whether the mate `m` is judged at the shut of the body `corpse`,
    /// lying at `at` (see [`judge_shut`]): it could come for something on
    /// it ([`Mate::could_come_for`]) and is not done with it already. One
    /// played by hand, dead, far off, without room or not looting is told
    /// nothing, and opens the body or not by its own lights, as before;
    /// nor is anything said to be left for one that is not coming.
    pub(crate) fn may_judge(&self, corpse: u32, m: &Mate, at: glam::Vec3) -> bool {
        m.could_come_for(at) && !self.done_with.contains(&(corpse, m.guid))
    }

    /// Whether something on the body `corpse`, lying at `at`, may be left
    /// for the mate `m` (see [`called_to`]): it is judged at the body's
    /// shut ([`Autoplay::may_judge`]) and has not shut it already. Nothing
    /// goes to one that has shut it, was told it is done with it, or was
    /// stood by for and never came: none of those opens it again for it.
    pub(crate) fn may_be_sent(&self, corpse: u32, m: &Mate, at: glam::Vec3) -> bool {
        self.may_judge(corpse, m, at) && !self.has_shut(corpse, m.guid)
    }

    /// This character as the turns at a newly fallen body read it (see
    /// [`TeamView::opens_first`]), `me` being its player guid: all of it as
    /// it last said about itself on the board (`TeamView::me`), which is
    /// what the others read it by. Read as it is now instead, where it
    /// stands, whether it is at another body and how many turns it has had
    /// ran up to half a second ahead of the row the others had: a session
    /// that had just shut a body dealt the next to someone else while the
    /// rest dealt it to that session, and nobody opened it for a second.
    ///
    /// With nothing said yet (`mine` being where it stands), it is read as
    /// it is now.
    fn my_turn(&self, me: u32, mine: glam::Vec3, now: Instant) -> Turn {
        match self.team.me.as_ref() {
            Some(said) => Turn {
                guid: me,
                world: said.world,
                looting: said.looting.is_some(),
                fighting: said.target.is_some(),
                // Not opening bodies at all, it is dealt none.
                room: said.turn().is_some_and(|t| t.room),
                opened: said.opened_first,
            },
            None => Turn {
                guid: me,
                world: mine,
                looting: self.corpse_claim(now).is_some(),
                fighting: false,
                room: true,
                opened: self.opened_first(now),
            },
        }
    }

    /// The name of the one of the others whose turn the body `guid`, lying
    /// at `at`, is, when that turn is what keeps this character (`me`,
    /// standing at `mine`) off it (see [`Autoplay::ours_to_open`]).
    pub(crate) fn whose_turn(
        &self,
        guid: u32,
        at: glam::Vec3,
        me: u32,
        mine: glam::Vec3,
        now: Instant,
    ) -> Option<&str> {
        let settled =
            now.saturating_duration_since(self.corpse_first_seen(guid, now)) >= CLAIM_SETTLE;
        if settled
            || self.team.mates.is_empty()
            || self.corpse_claim(now).is_some_and(|(g, _)| g == guid)
            || self.team.working(guid)
        {
            return None;
        }
        let dealt = self
            .body_dealt_to(guid, now)
            .unwrap_or_else(|| self.team.opens_first(guid, at, self.my_turn(me, mine, now)));
        self.team
            .mates
            .iter()
            .find(|m| m.guid == dealt)
            .map(|m| m.name.as_str())
    }

    /// Take in the leader's plan, heard at `now`.
    pub fn take_orders(&mut self, plan: crate::plan::Plan, now: Instant) {
        self.orders = Some(crate::plan::Orders { plan, heard: now });
    }

    /// The plan this character is under, while it is fresh (see
    /// `crate::plan::ORDERS_LAST`) and from whoever leads the team as
    /// this character now sees it. A plan from a leader since replaced,
    /// or from before the team was left, is nobody's to obey.
    pub fn current_plan(&self, now: Instant) -> Option<&crate::plan::Plan> {
        if !self.config.team.enabled {
            return None;
        }
        let plan = self.orders.as_ref()?.current(now)?;
        let leader = if self.team.leader {
            self.team.me.as_ref().map(|m| m.name.as_str())
        } else {
            self.team.leader_mate().map(|m| m.name.as_str())
        };
        (leader == Some(plan.leader.as_str())).then_some(plan)
    }

    /// This character's orders under the plan, `me` being its player guid.
    pub fn order_for(&self, me: u32, now: Instant) -> Option<crate::plan::Order> {
        self.current_plan(now)?.order_for(me)
    }

    /// Whom the plan deals the body `body` to, if it is fresh and deals it.
    pub fn body_dealt_to(&self, body: u32, now: Instant) -> Option<u32> {
        self.current_plan(now)?.body_dealt_to(body)
    }

    /// A body this character, leading, dealt to one of the others and
    /// that still lies there (`there`) unemptied: `(body, whom)`. The
    /// leader's next chosen fight waits on it the way its own kill's body
    /// holds it (see `Client::waits_for_a_corpse`).
    ///
    /// Measured without this: the deal gave each body to one hand, so
    /// the leader owed none and walked off to the next creature the
    /// moment one fell, the party followed it, and the hand dealt the
    /// body was left forty metres behind or gave the body up. The
    /// party walked twice as far and took a quarter of the loot. Only
    /// while the plan is fresh: a leader that has stopped planning holds
    /// nothing for a deal it is no longer making.
    pub fn body_dealt_to_another(
        &self,
        me: u32,
        now: Instant,
        there: impl Fn(u32) -> bool,
    ) -> Option<(u32, u32)> {
        self.current_plan(now)?;
        self.planner
            .deals
            .iter()
            .filter(|(body, to, _)| *to != me && there(*body))
            .filter(|(body, _, _)| !self.looted.contains(body) && !self.shut_by_anyone(*body))
            .map(|(body, to, _)| (*body, *to))
            .min()
    }

    /// How many bodies this character opened first lately, within
    /// `DEAL_WINDOW`: its turns, as it says them on the board (see
    /// [`Mate::opened_first`]).
    pub fn opened_first(&self, now: Instant) -> u16 {
        let lately = self
            .first_opens
            .iter()
            .filter(|t| now.saturating_duration_since(**t) < DEAL_WINDOW)
            .count();
        u16::try_from(lately).unwrap_or(u16::MAX)
    }

    /// Whether any of the others has said it shut the body `corpse`: one
    /// the leader's plan does not deal, what is still on it being routed
    /// by the shut (see [`Shut`]).
    pub(crate) fn shut_by_anyone(&self, corpse: u32) -> bool {
        self.shut_by
            .range((corpse, 0, 0)..=(corpse, u32::MAX, u32::MAX))
            .next()
            .is_some()
    }

    /// Whether the mate `who` has said it shut the body `corpse` (see
    /// [`Autoplay::take_in_shuts`]).
    pub(crate) fn has_shut(&self, corpse: u32, who: u32) -> bool {
        self.shut_by
            .range((corpse, who, 0)..=(corpse, who, u32::MAX))
            .next()
            .is_some()
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

    /// The server's words on an invitation that came to nothing (see
    /// `refusals::Refusal::Recruit`): the mate named is held off
    /// recruiting for the table's wait, and the log says so once. Only
    /// a mate this character has invited is read this way: "{Name} is
    /// busy." is also what the server says of a patron who cannot take
    /// an oath just now (ACE `Player_Allegiance.cs`).
    pub(crate) fn hear_recruit_refusal(&mut self, name: &str, why: RecruitRefusal, now: Instant) {
        let Some(guid) = self
            .team
            .mates
            .iter()
            .find(|m| m.name == name)
            .map(|m| m.guid)
            .filter(|g| self.asked_lately(*g, now))
        else {
            return;
        };
        self.refuse_recruit(guid, why, now);
    }

    /// [`Autoplay::hear_recruit_refusal`] from the server's own words,
    /// for the tests, which are written in them.
    #[cfg(test)]
    pub(crate) fn hear_recruit_words(&mut self, text: &str, now: Instant) {
        if let Some(Refusal::Recruit { name, why }) = refused(text) {
            self.hear_recruit_refusal(name, why, now);
        }
    }

    /// The server's code on an invitation that came to nothing: the
    /// fellowship is full (WeenieError 0x041E, ACE `Fellowship.cs`,
    /// `AddFellowshipMember`), which is about whoever was asked last.
    /// The recruiting stops by the count before this is ever heard;
    /// this is for a count the server disagrees with.
    pub(crate) fn hear_fellowship_full(&mut self, now: Instant) {
        let Some(guid) = self
            .recruited
            .iter()
            .filter(|(g, _)| self.asked_lately(*g, now))
            .max_by_key(|(_, t)| *t)
            .map(|(g, _)| *g)
        else {
            return;
        };
        self.refuse_recruit(guid, RecruitRefusal::Full, now);
    }

    /// Whether this mate was invited within the last [`RECRUIT_AGAIN`]:
    /// what makes a refusal the answer to that invitation. The list of
    /// the invited is pruned only on the next invitation, so without
    /// the age a "+X is busy." ten minutes later -- a patron who could
    /// not take an oath -- held X off the fellowship.
    fn asked_lately(&self, guid: u32, now: Instant) -> bool {
        self.recruited
            .iter()
            .any(|(g, t)| *g == guid && now.duration_since(*t) < RECRUIT_AGAIN)
    }

    /// Hold the mate off recruiting after a refusal, as the table
    /// decides (see `refusals::answer`), and say so once. "Busy" is
    /// over in the seconds a use or a cast takes, so it is the same
    /// short wait every time: doubling, a mage that happened to be
    /// casting at each of eight asks was left out for hours. The others
    /// double, since what they wait on is slower to change.
    fn refuse_recruit(&mut self, guid: u32, why: RecruitRefusal, now: Instant) {
        let name = self
            .team
            .mates
            .iter()
            .find(|m| m.guid == guid)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| format!("{guid:#010x}"));
        let said = match why {
            RecruitRefusal::AlreadyAMember => "is already in a fellowship",
            RecruitRefusal::Busy => "is busy",
            RecruitRefusal::NotAccepting => "is not accepting fellowship requests",
            RecruitRefusal::Declined => "declined",
            RecruitRefusal::Full => "would not fit: the fellowship is full",
        };
        refusals::answer(&Refusal::Recruit { name: &name, why }).hold(
            &mut self.held_off,
            guid,
            said,
            now,
        );
        let wait = self.held_off.waited(&guid).unwrap_or(HELD_OFF_FIRST);
        let why = said;
        self.note(
            format!(
                "{name} {why}: not asked into the fellowship again for {} s",
                wait.as_secs()
            ),
            now,
        );
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

    /// The carried stack to hand a teammate short of `want`, by name.
    ///
    /// The stack the player said to sell first, since it is leaving
    /// anyway, and a stack they said to keep only when there is no
    /// other of the kind. The first stack whose name matched used to
    /// go, which was the kept stack as often as not while the one
    /// meant for a counter stayed.
    pub(crate) fn spare_for(&self, want: &str) -> Option<(u32, String)> {
        let want = want.to_lowercase();
        let ledger = &self.autoplay.ledger;
        self.world
            .inventory()
            .filter(|o| o.name.to_lowercase().contains(&want))
            .min_by_key(|o| match ledger.by_guid(o.guid) {
                Some(LootAction::Sell) => 0,
                Some(LootAction::Keep) => 2,
                _ => 1,
            })
            .map(|o| (o.guid, o.name.clone()))
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

    /// The fellowship's members by player guid, while this character is
    /// one of them.
    fn fellows(&self) -> Option<Vec<u32>> {
        let me = self.world.player_guid.unwrap_or(0);
        self.world
            .fellowship
            .as_ref()
            .map(|f| f.members.iter().map(|m| m.guid).collect::<Vec<u32>>())
            .filter(|f| f.contains(&me))
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

    /// The party as the planner reads it: this character first, from
    /// what it knows of itself, then each mate from its row on the board
    /// (see `crate::plan::Hand`).
    fn hands_of_team(&self, now: Instant, with_me: bool) -> Vec<crate::plan::Hand> {
        let alive = |g: u32| {
            self.world
                .objects
                .get(&g)
                .is_some_and(|o| o.health.unwrap_or(1.0) > 0.0)
        };
        let me = self.world.player_guid.unwrap_or(0);
        let mine = self.player.as_ref().map(|p| p.world_position());
        let mut hands = Vec::with_capacity(self.autoplay.team.mates.len() + 1);
        if let Some(mine) = mine.filter(|_| me != 0 && with_me) {
            let supplies = self.supplies(&self.autoplay.config.growth, now);
            hands.push(crate::plan::Hand {
                guid: me,
                name: self.world.stats.name.clone(),
                world: mine,
                health: self.health_fraction(),
                fights: self.autoplay.config.enabled,
                target: self
                    .attack_target
                    .or(self.autoplay.casting_at())
                    .filter(|g| alive(*g)),
                looting: self.autoplay.corpse_claim(now).map(|(g, _)| g),
                opens: self.autoplay.config.enabled
                    && self.opens_bodies()
                    && !supplies.pack_full
                    && !supplies.laden,
                following: false,
                turns: self.autoplay.planner.turns_of(me, now, DEAL_WINDOW),
                hit_by: self.attackers_lately(),
            });
        }
        for m in &self.autoplay.team.mates {
            if m.guid == 0 {
                continue;
            }
            hands.push(crate::plan::Hand {
                guid: m.guid,
                name: m.name.clone(),
                world: m.world,
                health: m.health,
                fights: m.autoplay && m.health > 0.0,
                target: m.target,
                looting: m.looting.filter(|_| m.looting_for < CLAIM_STALE),
                opens: m.turn().is_some_and(|t| t.room),
                following: m.following,
                turns: self.autoplay.planner.turns_of(m.guid, now, DEAL_WINDOW),
                hit_by: m.hit_by.clone(),
            });
        }
        hands
    }

    /// The leader's plan for the party (see `crate::plan`): who fights
    /// what, whose turn each body is, and whom the leader is waiting
    /// for. Made from what the leader sees of the field and what the
    /// others have said on the board, and kept as the leader's own orders
    /// too. The team plugin asks for it once a board round and posts it.
    pub fn plan_for_team(&mut self, now: Instant) -> crate::plan::Plan {
        let cfg = self.autoplay.config.fight.clone();
        let hands = self.hands_of_team(now, true);
        let me = self.player.as_ref().map(|p| p.world_position());
        let underground = self.underground();
        // The creatures the party would fight, within the leader's own
        // radius and the one a follower fights within of its leader
        // (`Team::fight_radius`): further off, an order would draw a
        // follower away just as picking for itself would have.
        let within = cfg
            .radius
            .min(self.autoplay.config.team.fight_radius.max(1.0));
        let named: Vec<(u32, String, glam::Vec3)> = self
            .world
            .objects
            .values()
            .filter(|o| self.would_fight(o, &cfg, underground, now))
            .filter_map(|o| {
                let at = o.world_pos()?;
                (me.is_none_or(|m| at.distance(m) <= within)).then(|| (o.guid, o.name.clone(), at))
            })
            .collect();
        let by_name: Vec<(u32, &str, glam::Vec3)> = named
            .iter()
            .map(|(g, n, at)| (*g, n.as_str(), *at))
            .collect();
        let hunted: Vec<(u32, Vec<u32>)> = hands
            .iter()
            .map(|h| (h.guid, crate::plan::hunted_by(&by_name, h)))
            .collect();
        let foes: Vec<crate::plan::Foe> = named
            .iter()
            .map(|(guid, _, at)| {
                let mut after: Vec<u32> = self
                    .world
                    .objects
                    .get(guid)
                    .and_then(|o| o.walked_at)
                    .filter(|w| hands.iter().any(|h| h.guid == *w))
                    .into_iter()
                    .collect();
                after.extend(
                    hunted
                        .iter()
                        .filter(|(_, foes)| foes.contains(guid))
                        .map(|(h, _)| *h),
                );
                after.sort_unstable();
                after.dedup();
                crate::plan::Foe {
                    guid: *guid,
                    world: *at,
                    hard: self.is_hard_fight(*guid),
                    after,
                }
            })
            .collect();
        // The bodies nobody has opened: not emptied by this character,
        // not shut by any of the others, and no player's remains.
        let bodies: Vec<crate::plan::Body> = self
            .world
            .objects
            .values()
            .filter(|o| o.object_desc_flags & ac_world::object_desc_flags::CORPSE != 0)
            .filter(|o| !self.corpse_is_someone_elses(&o.name))
            .filter(|o| !self.autoplay.looted.contains(&o.guid))
            .filter(|o| !self.autoplay.shut_by_anyone(o.guid))
            .filter_map(|o| {
                Some(crate::plan::Body {
                    guid: o.guid,
                    world: o.world_pos()?,
                })
            })
            .collect();
        // What each was last told to fight, so that nobody is moved for
        // nothing (see `plan::assign_targets`).
        let prior: std::collections::BTreeMap<u32, u32> = self
            .autoplay
            .current_plan(now)
            .map(|p| {
                p.orders
                    .iter()
                    .filter_map(|(who, o)| Some((*who, o.target?)))
                    .collect()
            })
            .unwrap_or_default();
        let targets = crate::plan::assign_targets(&hands, &foes, &prior);
        let standing = self.autoplay.planner.standing(now);
        let passed = self.autoplay.planner.passed();
        let dealt =
            crate::plan::deal_bodies(&hands, &bodies, &standing, &passed, corpse_within_reach);
        for (body, to) in &dealt.lapsed {
            let who = hands
                .iter()
                .find(|h| h.guid == *to)
                .map(|h| h.name.clone())
                .unwrap_or_default();
            self.autoplay.note(
                format!("{who} did not come for the body {body:#010x}; dealing it on"),
                now,
            );
        }
        self.autoplay.planner.dealt(&dealt, now, DEAL_WINDOW);
        let deals = dealt.deals;
        self.autoplay.planner.n = self.autoplay.planner.n.wrapping_add(1);
        let mut orders: std::collections::BTreeMap<u32, crate::plan::Order> =
            std::collections::BTreeMap::new();
        for h in &hands {
            let order = crate::plan::Order {
                target: targets.get(&h.guid).copied(),
                body: deals
                    .iter()
                    .find(|(_, to)| *to == h.guid)
                    .map(|(body, _)| *body),
            };
            if order != crate::plan::Order::default() {
                orders.insert(h.guid, order);
            }
        }
        let waiting_for = match (self.autoplay.straggling_since, me) {
            (Some(_), Some(mine)) => {
                crate::plan::stragglers(mine, self.autoplay.config.team.follow_distance, &hands)
                    .into_iter()
                    .map(|s| s.name)
                    .collect()
            }
            _ => Vec::new(),
        };
        let plan = crate::plan::Plan {
            leader: self.world.stats.name.clone(),
            n: self.autoplay.planner.n,
            orders,
            waiting_for,
        };
        self.autoplay.take_orders(plan.clone(), now);
        plan
    }

    /// What the leader's plan has this character fight, while the plan
    /// is fresh and the creature is alive and not one this character is
    /// walking past on its way somewhere (see [`Self::joins_the_team_on`]).
    /// Only with focus fire on: that is the switch for fighting as a
    /// party rather than each for itself.
    pub(crate) fn ordered_target(&self, cfg: &Fight, now: Instant) -> Option<(u32, String)> {
        let team = &self.autoplay.config.team;
        if !team.enabled || !team.focus_fire {
            return None;
        }
        let me = self.world.player_guid?;
        let guid = self.autoplay.order_for(me, now)?.target?;
        let o = self.world.objects.get(&guid)?;
        (o.health.unwrap_or(1.0) > 0.0 && !self.passing_by(o, cfg)).then(|| (guid, o.name.clone()))
    }

    /// Whether an order to fight something other than `current` is to be
    /// followed now: it names a live creature, and the one in hand is not
    /// hitting this character. What is hitting us is fought to the end
    /// whatever the plan says, since that fight is already happening.
    fn ordered_elsewhere(&self, current: u32, cfg: &Fight, now: Instant) -> bool {
        let Some((ordered, _)) = self.ordered_target(cfg, now) else {
            return false;
        };
        if ordered == current {
            return false;
        }
        let hitting_us = self
            .world
            .objects
            .get(&current)
            .is_some_and(|o| self.hit_lately_by(&o.name));
        !hitting_us
    }

    /// Whether the leader holds the party where it is, for a follower
    /// still fighting or too far behind (see `crate::plan::stragglers`),
    /// and says so. Up to `crate::plan::STRAGGLE_PATIENCE`; past that the
    /// party moves and the following brings the straggler along.
    pub(crate) fn waits_for_stragglers(&mut self, now: Instant) -> bool {
        let team = &self.autoplay.config.team;
        if !team.enabled || !self.autoplay.team.leader || self.autoplay.team.mates.is_empty() {
            self.autoplay.straggling_since = None;
            return false;
        }
        let Some(me) = self.player.as_ref().map(|p| p.world_position()) else {
            self.autoplay.straggling_since = None;
            return false;
        };
        let keep = team.follow_distance;
        let hands = self.hands_of_team(now, false);
        let behind = crate::plan::stragglers(me, keep, &hands);
        if behind.is_empty() {
            self.autoplay.straggling_since = None;
            return false;
        }
        let since = *self.autoplay.straggling_since.get_or_insert(now);
        if now.duration_since(since) >= crate::plan::STRAGGLE_PATIENCE {
            let names: Vec<&str> = behind.iter().map(|s| s.name.as_str()).collect();
            self.autoplay.note(
                format!("waited long enough for {}; moving on", names.join(", ")),
                now,
            );
            self.autoplay.straggling_since = None;
            return false;
        }
        self.autoplay
            .say(Doing::Idle, crate::plan::waiting_line(&behind));
        true
    }

    /// Where the body `corpse` lies, the fellows judged at its shut (see
    /// [`Autoplay::may_judge`]), and those of them things on it may be left
    /// for (see [`Autoplay::may_be_sent`]), each as its row says it. `None`
    /// for a body with no place in the world to measure from.
    fn fellows_at(&self, corpse: u32) -> Option<(glam::Vec3, Vec<Fellow>, Vec<Fellow>)> {
        let at = self
            .world
            .objects
            .get(&corpse)
            .and_then(|o| o.world_pos())?;
        let fellows = self.fellows();
        let (mut judged, mut sendable) = (Vec::new(), Vec::new());
        for m in self.autoplay.team.judged_at_a_shut(fellows.as_deref()) {
            if !self.autoplay.may_judge(corpse, m, at) {
                continue;
            }
            let fellow = (m.guid, m.name.clone(), m.wielder());
            if self.autoplay.may_be_sent(corpse, m, at) {
                sendable.push(fellow.clone());
            }
            judged.push(fellow);
        }
        Some((at, judged, sendable))
    }

    /// The fellows a thing on the body `corpse` could be meant for instead
    /// of this character (see [`called_to`], [`Client::fellows_at`]).
    fn callable_to(&self, corpse: u32) -> Vec<Fellow> {
        self.fellows_at(corpse)
            .map(|(_, _, sendable)| sendable)
            .unwrap_or_default()
    }

    /// What the body `corpse`, being shut as emptied with `items` still on
    /// it, is for this character and for each fellow (see [`judge_shut`]):
    /// judged by this character's rules, which the party shares, with what
    /// each fellow said about itself. Under rules that mean nothing for
    /// anyone in particular, nothing is left for anyone in particular.
    fn shut_for(&self, corpse: u32, items: &[u32], profile: &crate::profile::Profile) -> ShutFor {
        let Some((at, judged, sendable)) = self.fellows_at(corpse) else {
            return ShutFor::default();
        };
        let sendable = if profile.sends_to_the_best() {
            sendable
        } else {
            Vec::new()
        };
        let lying: Vec<Option<Left>> = items
            .iter()
            .map(|g| {
                self.stats_of(*g).map(|stats| Left {
                    held: self.already_carried(stats.wcid),
                    id: self.appraisals.get(g),
                    stats,
                })
            })
            .collect();
        let sheet = self.wielder();
        let me = Taker {
            guid: self.world.player_guid.unwrap_or(0),
            name: &self.world.stats.name,
            sheet: &sheet,
            held: 0,
        };
        let salvager = if sendable.is_empty() {
            None
        } else {
            self.best_salvager().map(|(_, g)| g)
        };
        ShutFor {
            at,
            ..judge_shut(&lying, profile, me, &judged, &sendable, salvager)
        }
    }

    /// The names of the fellows `guids`, as their rows have them.
    fn names_of(&self, guids: &[u32]) -> Vec<String> {
        guids
            .iter()
            .filter_map(|g| self.autoplay.team.mates.iter().find(|m| m.guid == *g))
            .map(|m| m.name.clone())
            .collect()
    }

    /// A body this character is standing off from only because it is one
    /// of the others' turn at it, by name, and whose turn (see
    /// [`Autoplay::whose_turn`]).
    fn turn_of_another(
        &self,
        me: glam::Vec3,
        now: Instant,
        room: Room,
    ) -> Option<(String, String)> {
        let my_guid = self.world.player_guid.unwrap_or(0);
        self.world
            .objects
            .values()
            .filter(|o| o.object_desc_flags & ac_world::object_desc_flags::CORPSE != 0)
            .find_map(|o| {
                let at = o.world_pos()?;
                if !self.autoplay.corpse_owed(o.guid, at, me, now, room)
                    || self.corpse_is_someone_elses(&o.name)
                {
                    return None;
                }
                let who = self.autoplay.whose_turn(o.guid, at, my_guid, me, now)?;
                Some((o.name.clone(), who.to_string()))
            })
    }

    /// Take in the bodies the others said they emptied for this
    /// character, and say so in the log (see
    /// `Autoplay::take_in_shuts`). The team plugin calls this as it
    /// hands the rules what the others said.
    pub fn take_in_shuts(&mut self) {
        let me = self.world.player_guid.unwrap_or(0);
        let objects = &self.world.objects;
        let heard = self.autoplay.take_in_shuts(me, Instant::now(), |g| {
            objects.get(&g).and_then(|o| o.world_pos())
        });
        let who = &self.world.stats.name;
        let corpse = |g: u32| objects.get(&g).map_or("a corpse", |o| o.name.as_str());
        for taken in heard {
            match taken {
                TakenIn::Done { body, by } => {
                    tracing::info!("{}", taken_in_line(who, corpse(body), body, &by));
                }
                TakenIn::StandBy { body, by, on } => tracing::info!(
                    "{}",
                    standing_by_line(who, corpse(body), body, &by, &self.names_of(&on))
                ),
            }
        }
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

    /// The leader this character follows, when it follows one.
    pub(crate) fn followed_leader(&self) -> Option<&Mate> {
        let team = &self.autoplay.config.team;
        if !team.enabled || !team.follow || team.lead || self.autoplay.team.leader {
            return None;
        }
        self.autoplay.team.leader_mate().filter(|m| m.leads)
    }

    /// Note what this character is running short of, so the others can
    /// hand it over.
    pub(crate) fn autoplay_stock(&mut self) {
        if !self.autoplay.config.team.enabled {
            self.autoplay.wants.clear();
            return;
        }
        // What this character is short of, from the same buy list that
        // decides what it shops for and what it will not sell. It used
        // to be a second list on the team settings, tested by hand here
        // with the same arithmetic the profile already does.
        //
        // This is what a teammate reads before handing anything over
        // (`Mate::wants`), so a character with nothing on its buy list
        // asks for nothing -- which is right, and is also why the list
        // matters more than it looks.
        //
        // Counted the way the character's own restock list counts it:
        // what is carried less what the profile is selling out of. The
        // two once differed, and a mate handed scarabs over while the
        // character's own list said it wanted none.
        let profile = self.profiles.get(&self.autoplay.config.loot.profile);
        let leaving = self.leaving_of(&self.item_stats());
        self.autoplay.wants = profile
            .map(|p| {
                p.shortfall(|what| self.carried_named(what).saturating_sub(leaving.named(what)))
                    .into_iter()
                    .map(|s| s.want.what.clone())
                    .collect()
            })
            .unwrap_or_default();
    }

    /// A fellowship invitation from anyone is accepted while on a team:
    /// the leader sends them, and the leader is trusted.
    fn autoplay_accept_invites(&mut self) {
        const FELLOWSHIP: u32 = 4;
        // The server asks the character before recruiting it only when
        // its options allow: with "accept fellowship requests" off the
        // leader's invitation is refused outright, and with "automatically
        // accept" on it never has to be answered. A teammate keeps both on,
        // and lets the others give it items: that is how salvage reaches
        // whoever salvages, and the server refuses a gift to anyone with
        // the option off (ACE `CharacterOptions1.AllowGive`).
        //
        // Loot sharing is the fourth, and it is the leader's option that
        // counts: ACE takes it off whoever founds the fellowship, once,
        // in the `Fellowship` constructor, and `Corpse.HasPermission`
        // lets a fellow open a fresh body through that clause alone.
        // With it off, nine characters hunting together told each other
        // "You do not yet have the right to loot" 278 times in ten
        // minutes -- only the killer could open its own kill, for the
        // first two minutes of the body's life. Every teammate keeps it
        // on, not just the one leading today, because the fellowship's
        // leader changes with whoever is about.
        for name in TEAM_OPTIONS {
            if let Some(o) = crate::options::option_by_name(name) {
                if !self.option_enabled(o) {
                    self.set_option(o, true);
                }
            }
        }
        let invites: Vec<(u32, u32)> = self
            .world
            .confirmations
            .iter()
            .filter(|c| c.kind == FELLOWSHIP)
            .map(|c| (c.kind, c.context))
            .collect();
        for (kind, context) in invites {
            self.confirm(kind, context, true);
        }
    }

    /// Whether this character's own options say it shares fellowship
    /// loot. It is the founder's answer to this, at the moment it
    /// founds, that decides whether the fellowship shares loot at all
    /// (ACE `Entity/Fellowship.cs`, the constructor).
    fn shares_fellowship_loot(&self) -> bool {
        crate::options::option_by_name("share fellowship loot")
            .is_some_and(|o| self.option_enabled(o))
    }

    /// The fellowship this character founded is given up when the team's
    /// rightful leader has one of its own (see [`rival_leader`]): the
    /// server refuses no founding (ACE `Player_Fellowship.cs`,
    /// `FellowshipCreate`), so two leaders make two fellowships, the
    /// fleet splits between them, and members of one have no right to
    /// the other's bodies. Disbanded, this one's members are free for
    /// the rightful leader to recruit, and this character with them.
    ///
    /// Only a fellowship this character founded itself, and still leads
    /// (`founded`, the same word that decides whether it can vouch for
    /// the loot sharing): one a person made by hand is never taken
    /// apart on the strength of a roster, as the note on a fellowship
    /// "not founded by me" already has it. True when it was given up.
    fn autoplay_yield_fellowship(&mut self, now: Instant) -> bool {
        let (Some(f), Some(me)) = (self.world.fellowship.as_ref(), self.world.player_guid) else {
            self.autoplay.outled_since = None;
            return false;
        };
        if self.autoplay.founded.is_none() || f.leader != me {
            self.autoplay.outled_since = None;
            return false;
        }
        let fname = f.name.clone();
        let members: Vec<u32> = f.members.iter().map(|m| m.guid).collect();
        let Some(rival) = rival_leader(&self.autoplay.team, &members).map(|m| m.name.clone())
        else {
            self.autoplay.outled_since = None;
            return false;
        };
        let since = *self.autoplay.outled_since.get_or_insert(now);
        if now.duration_since(since) < YIELD_AFTER {
            return false;
        }
        self.fellowship_quit(true);
        self.autoplay.founded = None;
        self.autoplay.outled_since = None;
        self.autoplay.say(
            Doing::Helping,
            format!("disbanding the fellowship {fname}: {rival} leads, and has one of its own"),
        );
        true
    }

    /// The leader founds the fellowship and brings the others into it,
    /// one invitation at a time. True when one went out.
    fn autoplay_fellowship(&mut self, now: Instant) -> bool {
        let team = self.autoplay.config.team.clone();
        if !team.enabled || !team.fellowship {
            return false;
        }
        if self.autoplay_yield_fellowship(now) {
            return true;
        }
        // Who gathers: the team's leader founds the fellowship, and
        // whoever leads a fellowship brings the team's mates into it --
        // the team's leader among them, when it stands outside. It does
        // stand outside: slow to post, it came on after a mate had
        // founded and gathered everyone else, and with nobody free to
        // found with it stayed out for the life of the run, since the
        // founder recruited only while it led the team. The same
        // follows the leader's own disconnect: the server quits it on
        // logout and hands the fellowship to whoever is left (ACE
        // `Fellowship.QuitFellowship`, `AssignNewLeader`), and the
        // leader comes back to a fellowship it is outside of.
        let leads_this = self
            .world
            .fellowship
            .as_ref()
            .is_some_and(|f| Some(f.leader) == self.world.player_guid);
        if !self.autoplay.team.leader && !leads_this {
            return false;
        }
        let Some(me) = self.player.as_ref().map(|p| p.world_position()) else {
            return false;
        };
        // Whoever is already in, first-hand: the server tells the leader
        // who has joined before the mate gets round to telling the board.
        let joined: Vec<u32> = self
            .world
            .fellowship
            .as_ref()
            .map(|f| f.members.iter().map(|m| m.guid).collect())
            .unwrap_or_default();
        // One that is in is no longer held off: the next refusal, if it
        // ever leaves and is asked again, starts a wait of its own. Nor
        // is one off the roster: a mate that left the team, or lost its
        // session, is asked afresh when it is back.
        for guid in &joined {
            self.autoplay.held_off.forget(guid);
        }
        let on_the_team: Vec<u32> = self.autoplay.team.mates.iter().map(|m| m.guid).collect();
        self.autoplay.held_off.retain(|g| on_the_team.contains(g));
        let waiting: Vec<(u32, f32)> = self
            .autoplay
            .team
            .mates
            .iter()
            .filter(|m| !m.in_fellowship && m.guid != 0 && !joined.contains(&m.guid))
            .map(|m| (m.guid, m.world.distance(me)))
            .filter(|(_, away)| *away < RECRUIT_RANGE)
            .collect();
        if self.world.fellowship.is_none() {
            // A founding already asked for and not yet answered: the
            // server makes the fellowship and tells us about it in its
            // own time, and two foundings would be two fellowships.
            if self
                .autoplay
                .founded
                .is_some_and(|t| now.duration_since(t) < FOUNDING_WAIT)
            {
                return false;
            }
            // Nothing came back, or the fellowship we made has gone:
            // either way there is none to vouch for now.
            self.autoplay.founded = None;
            self.autoplay.said_not_sharing = false;
            if waiting.is_empty() {
                return false;
            }
            // Not until the roster has stood a full board round: until
            // then this session may be leading only what it has heard so
            // far, and the one that sorts first among everyone is a frame
            // or a hop away from being heard. Nine sessions coming onto
            // the team in one tick founded two fellowships this way.
            if !self.autoplay.team.settled {
                self.autoplay.note(
                    "waiting for the roster to settle before founding the fellowship",
                    now,
                );
                return false;
            }
            // Nothing re-reads the option once the fellowship exists, so
            // a fellowship founded a moment too early shares no loot for
            // as long as it lives. The option and the founding go out on
            // the one ordered queue of actions and the server works
            // through them in that order, so all this has to wait for is
            // our own asking, which `autoplay_accept_invites` does at the
            // top of the same tick.
            if !self.shares_fellowship_loot() {
                self.autoplay.note(
                    "waiting for loot sharing before founding the fellowship",
                    now,
                );
                return false;
            }
            if self
                .autoplay
                .last_recruit
                .is_some_and(|t| now.duration_since(t) < RECRUIT_FLOOR)
            {
                return false;
            }
            let fname = team.fellowship_name.clone();
            self.fellowship_create(&fname, true);
            self.autoplay.founded = Some(now);
            self.autoplay.last_recruit = Some(now);
            self.autoplay
                .say(Doing::Helping, format!("starting the fellowship {fname}"));
            return true;
        }
        // A fellowship we did not found ourselves is left alone, and
        // said so once. Nothing on the wire says whether one shares
        // loot -- the full update writes the same 0x10 for every fellow
        // whatever the fellowship does -- so disbanding it would be
        // throwing away a working party on a guess, and the guess would
        // be wrong for every fellowship a person made by hand.
        if self.autoplay.founded.is_none() && !self.autoplay.said_not_sharing {
            self.autoplay.said_not_sharing = true;
            self.autoplay.note(
                "this fellowship was not founded by me: it may not share loot, \
                 and only disbanding it would tell",
                now,
            );
        }
        // Only the fellowship's own leader may recruit into it (ACE
        // `Player_Fellowship.cs`, `FellowshipRecruit`: anyone else is
        // answered 0x041D). A team leader that let itself be recruited
        // into a mate's fellowship leaves the gathering to that mate.
        if !leads_this {
            return false;
        }
        // A fellowship holds nine (ACE `Fellowship.MaxFellows`), and the
        // tenth asked is refused (0x041E) as often as it is asked. Said
        // once, and nobody is asked.
        if joined.len() >= MAX_FELLOWS {
            if !waiting.is_empty() {
                self.autoplay.note(
                    format!(
                        "the fellowship is full at {MAX_FELLOWS}: {} mate(s) left outside it",
                        waiting.len()
                    ),
                    now,
                );
            }
            return false;
        }
        let Some(guid) = next_invitee(
            &waiting,
            &self.autoplay.recruited,
            &self.autoplay.held_off,
            self.autoplay.last_recruit,
            now,
        ) else {
            return false;
        };
        self.fellowship_recruit(guid);
        self.autoplay.last_recruit = Some(now);
        self.autoplay
            .recruited
            .retain(|(g, t)| *g != guid && now.duration_since(*t) < RECRUIT_AGAIN);
        self.autoplay.recruited.push((guid, now));
        let name = self
            .autoplay
            .team
            .mates
            .iter()
            .find(|m| m.guid == guid)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| format!("{guid:#010x}"));
        self.autoplay.say(
            Doing::Helping,
            format!("bringing {name} into the fellowship"),
        );
        true
    }

    /// Keep up with the leader: fly when it flies, walk straight after
    /// it while it is near, plan a journey after it when it has gone
    /// through a portal. With `urgent`, only a leader that has got well
    /// away counts (it is fetched before a fight); otherwise any leader
    /// further than the following distance. True while on the way.
    pub(crate) fn autoplay_follow(&mut self, now: Instant, urgent: bool) -> bool {
        let team = self.autoplay.config.team.clone();
        if !team.enabled || !team.follow || team.lead || self.autoplay.team.leader {
            return false;
        }
        // Not while on a town run of its own. A party that restocks with
        // everyone going shops each for itself, and a follower pulled back
        // to its leader had its walk to its own counter ended each time it
        // closed to the following distance. The journey under way is the
        // run's, not one after the leader.
        if self.autoplay.growth.town_run_under_way() {
            self.autoplay.follow_trip = None;
            return false;
        }
        let Some(leader) = self
            .autoplay
            .team
            .leader_mate()
            .filter(|m| m.leads)
            .cloned()
        else {
            return false;
        };
        let Some(me) = self.player.as_ref().map(|p| p.world_position()) else {
            return false;
        };
        let keep = team.follow_distance.max(1.5);
        // Fly when the leader flies, land when it lands.
        if leader.flying != self.noclip() {
            self.set_noclip(leader.flying);
        }
        let flat = glam::Vec2::new(leader.world.x - me.x, leader.world.y - me.y).length();
        let far = flat > follow_break(keep);
        if urgent && !far {
            return false;
        }
        let level = !leader.flying || (leader.world.z - me.z).abs() < 2.0;
        if flat <= keep && level {
            if self.follow.take().is_some() {
                self.steering.reset();
            }
            if self.autoplay.follow_trip.take().is_some() && self.traveling() {
                self.cancel_travel();
            }
            return false;
        }
        if far {
            // Whatever we were fighting is not worth losing the leader.
            self.autoplay.casting_at = None;
            self.attack_target = None;
        }
        if leader.flying || flat < FOLLOW_WALK {
            // Straight after it: the steering finds the way round
            // walls, and flight has nothing in the way.
            if self.autoplay.follow_trip.take().is_some() && self.traveling() {
                self.cancel_travel();
            }
            self.head_for(leader.world, keep, "the leader");
        } else {
            // Out of sight (through a portal, say): a journey there,
            // planned again once it has moved on. Not while it stands
            // somewhere a journey cannot end -- inside the Town Network
            // hub, or a dungeon -- in another landblock: it will come
            // out, and its last position outside is followed meanwhile.
            self.follow = None;
            let indoors = leader.cell & 0xFFFF >= 0x100;
            let my_block = self.player.as_ref().map(|p| p.landblock());
            if indoors && my_block != Some(leader.cell & 0xFFFF_0000) {
                self.autoplay
                    .say(Doing::Following, "waiting for the leader to come out");
                return false;
            }
            let goal = glam::Vec2::new(leader.world.x, leader.world.y);
            let stale = self
                .autoplay
                .follow_trip
                .is_none_or(|g| g.distance(goal) > 30.0);
            let due = self.autoplay.next_follow_plan.is_none_or(|t| now >= t);
            if (stale || !self.traveling()) && due {
                // On the leader's way, whatever the leader is on (see
                // `Client::on_its_way`): not a road of its own.
                if self.travel_about(goal) {
                    self.autoplay.follow_trip = Some(goal);
                    self.autoplay.next_follow_plan = Some(now + Duration::from_secs(3));
                } else {
                    self.autoplay.follow_trip = None;
                    self.autoplay.next_follow_plan = Some(now + Duration::from_secs(10));
                }
            }
        }
        self.autoplay
            .say(Doing::Following, format!("following {}", leader.name));
        true
    }

    /// How close two characters must stand to hand something over.
    const REACH: f32 = 5.0;

    /// Loading the quartermaster and unloading it again.
    ///
    /// On a quartermaster run one character carries the party's sale
    /// loot to town and its shopping home, so there are two moments
    /// where items change hands: everyone gives it their loot before it
    /// leaves, and it gives everyone their order when it gets back.
    /// Both are the same shape -- walk into reach, hand one thing over,
    /// come back next frame for the next -- because the server takes
    /// one give at a time.
    ///
    /// True when it acted, which stops the rest of the rules for this
    /// frame: nothing else matters while the party is being loaded.
    fn autoplay_quartermaster(&mut self, now: Instant) -> bool {
        use crate::logistics::{Plan, Stage};
        let team = self.autoplay.config.team.clone();
        if !team.enabled || !team.restock.together || team.restock.plan != Plan::Quartermaster {
            return false;
        }
        let Some(stage) = self.autoplay.growth.mode.stage() else {
            return false;
        };
        // Waiting on the last give. The frame is still this errand's:
        // let something else have it and the tidying merges away the
        // money that was just counted out.
        if self
            .autoplay
            .last_give
            .is_some_and(|t| now.duration_since(t) < GIVE_EVERY)
        {
            return matches!(stage, Stage::HandOver | Stage::HandOut);
        }
        let growth = self.autoplay.config.growth.clone();
        let Some(runner) = self.quartermaster_name(&growth, now) else {
            return false;
        };
        let am_runner = runner == self.world.stats.name;
        match stage {
            Stage::HandOver if !am_runner => self.load_the_quartermaster(&runner, &growth, now),
            Stage::HandOut if am_runner => self.unload_the_quartermaster(&growth, now),
            _ => false,
        }
    }

    /// Give the runner this character's sale loot, then say so.
    fn load_the_quartermaster(
        &mut self,
        runner: &str,
        growth: &crate::growth::Growth,
        now: Instant,
    ) -> bool {
        if self.autoplay.growth.handed_over {
            return false;
        }
        let Some(mate) = self
            .autoplay
            .team
            .mates
            .iter()
            .find(|m| m.name == runner)
            .cloned()
        else {
            return false;
        };
        let loot = self.loot_for_sale(growth);
        // Money travels with the loot. The runner is the one standing
        // at the counter, so it is the one that has to be able to pay,
        // and coin changes hands for nothing: it is trade notes that
        // cost to make (see `spare_coin`).
        let coin = self
            .autoplay
            .config
            .team
            .restock
            .share_money
            .then(|| self.spare_coin())
            .flatten();
        if loot.is_empty() && coin.is_none() {
            // Nothing to hand over: this character is loaded already.
            self.autoplay.growth.handed_over = true;
            return false;
        }
        if !self.step_into_reach(mate.world) {
            if self.reaching_too_long(mate.world, now) {
                // Cannot get to them -- a wall, a different building,
                // a floor above. The party is not held up over it.
                self.autoplay.growth.handed_over = true;
                self.autoplay
                    .note(format!("cannot get to {runner} to hand over"), now);
                return false;
            }
            self.autoplay
                .say(Doing::Helping, format!("taking the loot to {runner}"));
            return true;
        }
        if let Some((purse, amount)) = coin {
            // Part of a stack cannot be handed over as it stands: the
            // money is counted out first (see `hand_stack`).
            match self.hand_stack(mate.guid, purse, amount, now) {
                Some(true) => {
                    self.autoplay.say(
                        Doing::Helping,
                        format!("giving {runner} {amount} pyreals to shop with"),
                    );
                    return true;
                }
                None => {
                    self.autoplay.say(
                        Doing::Helping,
                        format!("counting out {amount} pyreals for {runner}"),
                    );
                    return true;
                }
                // The money will not come apart. The loot still can go,
                // and the runner may have enough of its own.
                Some(false) => {}
            }
        }
        let Some(&item) = loot.first() else {
            self.autoplay.growth.handed_over = true;
            return false;
        };
        let name = self
            .world
            .objects
            .get(&item)
            .map(|o| o.name.clone())
            .unwrap_or_default();
        if !self.give(mate.guid, item, None) {
            // The server would not take it; do not jam on this item.
            self.autoplay.growth.handed_over = true;
            return false;
        }
        self.autoplay.last_give = Some(now);
        self.autoplay
            .give_tries
            .note(item, &crate::did::Did::Done, now);
        self.autoplay
            .say(Doing::Helping, format!("giving {name} to {runner} to sell"));
        // Loaded once the last piece has gone.
        self.autoplay.growth.handed_over = loot.len() == 1;
        true
    }

    /// A carried stack of this weenie holding exactly this many, which
    /// is what a split leaves behind: the piece counted out to hand
    /// over. `None` for the weenie matches anything of the right size.
    fn piece_of(&self, wcid: Option<u32>, amount: u32) -> Option<u32> {
        self.world
            .inventory()
            .find(|o| {
                o.stack_size == amount
                    && match wcid {
                        Some(w) => o.weenie_class_id == w,
                        None => o.item_type & ac_world::item_type::MONEY != 0,
                    }
            })
            .map(|o| o.guid)
    }

    /// Hand `amount` out of a carried `stack` to a teammate.
    ///
    /// The server takes whole objects: a give of part of a stack goes
    /// into the void unanswered. So anything short of the whole stack
    /// is counted out into a stack of its own first, and that is what
    /// changes hands. `None` while the counting-out is still going on,
    /// `Some(true)` when a give went out, `Some(false)` when the thing
    /// will not come apart and the party should get on without it.
    fn hand_stack(&mut self, to: u32, stack: u32, amount: u32, now: Instant) -> Option<bool> {
        let (whole, wcid) = self
            .world
            .objects
            .get(&stack)
            .map(|o| (o.stack_size.max(1), o.weenie_class_id))
            .unwrap_or((1, 0));
        let piece = if amount >= whole {
            Some(stack)
        } else {
            self.piece_of(Some(wcid), amount)
        };
        if let Some(g) = piece {
            if self.give(to, g, None) {
                self.autoplay.last_give = Some(now);
                self.autoplay
                    .give_tries
                    .note(stack, &crate::did::Did::Done, now);
                return Some(true);
            }
            return Some(false);
        }
        // The counting-out has been asked for and the piece has not
        // appeared. Waiting rather than blocked: a split is one message
        // and the server answers in its own time, so the first few asks
        // are a quarter of a second apart and double from there.
        if self
            .autoplay
            .give_tries
            .waited(&stack)
            .is_some_and(|w| w > WILL_NOT_SPLIT)
        {
            return Some(false);
        }
        self.autoplay.give_tries.note(
            stack,
            &crate::did::Did::waiting("the money has not come apart yet"),
            now,
        );
        self.split_stack(stack, None, amount);
        self.autoplay.last_give = Some(now);
        None
    }

    /// Give everyone what they ordered.
    fn unload_the_quartermaster(&mut self, growth: &crate::growth::Growth, now: Instant) -> bool {
        let party = self.party_supplies(growth, now);
        // Work the whole party's split out for each thing carried, so
        // that a short run is shared rather than filling the first
        // order and leaving the last character with nothing.
        for (item, _) in crate::logistics::merged_order(&party) {
            let carried: Vec<(u32, u32, String)> = self
                .world
                .inventory()
                .filter(|o| o.name.eq_ignore_ascii_case(&item))
                .map(|o| (o.guid, o.stack_size.max(1), o.name.clone()))
                .collect();
            let brought: u32 = carried.iter().map(|(_, n, _)| n).sum();
            if brought == 0 {
                continue;
            }
            for (who, share) in crate::logistics::hand_out(&party, &item, brought) {
                if who == self.world.stats.name || share == 0 {
                    continue;
                }
                let Some(mate) = self
                    .autoplay
                    .team
                    .mates
                    .iter()
                    .find(|m| m.name == who)
                    .cloned()
                else {
                    continue;
                };
                if !self.step_into_reach(mate.world) {
                    if self.reaching_too_long(mate.world, now) {
                        self.autoplay
                            .note(format!("cannot get to {who} to hand out"), now);
                        continue;
                    }
                    self.autoplay
                        .say(Doing::Helping, format!("taking {who} their supplies"));
                    return true;
                }
                let (guid, stack, name) = carried[0].clone();
                let share = share.min(stack);
                match self.hand_stack(mate.guid, guid, share, now) {
                    Some(true) => {
                        self.autoplay
                            .say(Doing::Helping, format!("giving {share} {name} to {who}"));
                        return true;
                    }
                    None => {
                        self.autoplay.say(
                            Doing::Helping,
                            format!("counting out {share} {name} for {who}"),
                        );
                        return true;
                    }
                    Some(false) => continue,
                }
            }
        }
        false
    }

    /// The coin this character can hand over, as `(stack, amount)`:
    /// what it carries less the float it keeps for itself.
    ///
    /// Coin, not trade notes. A note is the lighter way to carry a
    /// fortune, but the server charges 1.15 times a note's face value
    /// to make one and pays only face value to cash it back, so turning
    /// a purse into notes and back costs the party thirteen percent of
    /// it. Handing over pyreals costs nothing and buys exactly as much.
    fn spare_coin(&self) -> Option<(u32, u32)> {
        let float = self.autoplay.config.team.restock.float;
        let purse = self
            .world
            .inventory()
            .filter(|o| o.item_type & ac_world::item_type::MONEY != 0)
            .map(|o| (o.guid, o.stack_size.max(1)))
            .max_by_key(|(_, n)| *n)?;
        let spare = purse.1.checked_sub(float)?;
        (spare > 0).then_some((purse.0, spare))
    }

    /// Walk towards a spot until close enough to hand something over.
    /// True once in reach.
    fn step_into_reach(&mut self, spot: glam::Vec3) -> bool {
        let Some(me) = self.player.as_ref().map(|p| p.world_position()) else {
            return false;
        };
        if glam::Vec2::new(spot.x - me.x, spot.y - me.y).length() <= Self::REACH {
            self.autoplay.reaching = None;
            if self.follow.take().is_some() {
                self.steering.reset();
            }
            return true;
        }
        self.head_for(spot, Self::REACH * 0.6, "the spot");
        false
    }

    /// Whether walking to `spot` has gone on too long to be walking any
    /// more. Indoors a counter can stand behind a wall the steering
    /// cannot get round, and a teammate can be in the next building;
    /// without this the character presses towards it for ever.
    ///
    /// The clock starts when a new spot is aimed at and is reset by any
    /// real progress towards it, so a long walk is fine and a stopped
    /// one is not.
    fn reaching_too_long(&mut self, spot: glam::Vec3, now: Instant) -> bool {
        let Some(me) = self.player.as_ref().map(|p| p.world_position()) else {
            return false;
        };
        let away = glam::Vec2::new(spot.x - me.x, spot.y - me.y).length();
        match self.autoplay.reaching {
            Some((at, best, since)) if at.distance(spot) < 1.0 => {
                if away < best - REACH_PROGRESS {
                    self.autoplay.reaching = Some((spot, away, now));
                    return false;
                }
                if now.duration_since(since) > REACH_GIVE_UP {
                    self.autoplay.reaching = None;
                    return true;
                }
                false
            }
            _ => {
                self.autoplay.reaching = Some((spot, away, now));
                false
            }
        }
    }

    /// The things done for the team: land the debuffs on its target,
    /// recruit it into a fellowship, hand over what someone is short of,
    /// and heal whoever is worst hurt. True when it acted.
    pub(crate) fn autoplay_team(&mut self, now: Instant) -> bool {
        let team = self.autoplay.config.team.clone();
        if !team.enabled {
            return false;
        }
        let me = self.player.as_ref().map(|p| p.world_position());
        let Some(me) = me else { return false };

        if self.autoplay_fellowship(now) {
            return true;
        }

        // Loading and unloading the quartermaster comes before the rest:
        // nothing else matters while the party is changing hands.
        if self.autoplay_quartermaster(now) {
            return true;
        }

        // Hand over what a teammate is short of.
        if team.share_supplies
            && self
                .autoplay
                .last_give
                .is_none_or(|t| now.duration_since(t) > Duration::from_secs(3))
        {
            let mate = self.autoplay.team.wanting(me, 6.0).cloned();
            if let Some(mate) = mate {
                for want in &mate.wants {
                    if let Some((item, name)) = self.spare_for(want) {
                        self.give(mate.guid, item, None);
                        self.autoplay.last_give = Some(now);
                        self.autoplay
                            .say(Doing::Helping, format!("giving {name} to {}", mate.name));
                        return true;
                    }
                }
            }
        }

        // A healer looks after the others before it fights.
        if team.role == Role::Healer {
            let hurt = self
                .autoplay
                .team
                .worst_hurt()
                .filter(|m| m.health < self.autoplay.config.survive.heal_below)
                .cloned();
            if let Some(hurt) = hurt {
                let spell = self
                    .spell_by_name("Heal Other")
                    .filter(|s| matches!(self.can_cast(*s), crate::magic::CastCheck::Ok));
                if let Some(spell) = spell {
                    self.select(Some(hurt.guid));
                    self.cast(spell);
                    self.autoplay.last_heal = Some(now);
                    self.autoplay
                        .say(Doing::Healing, format!("healing {}", hurt.name));
                    return true;
                }
            }
        }

        // A debuffer softens the team's target before the others hit it.
        if team.role == Role::Debuffer && !team.debuffs.is_empty() {
            if self
                .autoplay
                .last_debuff
                .is_some_and(|t| now.duration_since(t) < BUFF_EVERY)
            {
                return false;
            }
            let target = self.autoplay.team.target().or_else(|| {
                self.attack_target
                    .map(|t| (t, self.last_target_name.clone()))
            });
            if let Some((guid, name)) = target {
                // Not one it is walking past on its way somewhere, any more
                // than the fight would be (see `joins_the_team_on`).
                let fight = &self.autoplay.config.fight;
                if self.joins_the_team_on(guid, fight) && !self.autoplay.debuffed.contains(&guid) {
                    for spell_name in &team.debuffs {
                        let Some(spell) = self.spell_by_name(spell_name) else {
                            continue;
                        };
                        if !matches!(self.can_cast(spell), crate::magic::CastCheck::Ok) {
                            continue;
                        }
                        self.select(Some(guid));
                        self.cast(spell);
                        self.autoplay.last_debuff = Some(now);
                        let spell_name = spell_name.clone();
                        self.autoplay
                            .say(Doing::Debuffing, format!("casting {spell_name} on {name}"));
                        // One family per target: the rest of the team can
                        // stop waiting for us.
                        if team.debuffs.last() == Some(&spell_name) {
                            self.autoplay.debuffed.push(guid);
                        }
                        return true;
                    }
                }
            }
        }
        false
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod fellowship_tests;
