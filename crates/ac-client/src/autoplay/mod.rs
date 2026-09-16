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

use ac_agent::recent::Recent;

use crate::Client;
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
#[cfg(any(test, feature = "testkit"))]
pub(crate) use fight::critter::CREATURE_LEVEL;
pub use fight::critter::{critter, Critter, Hint, Seen};
pub use fight::road_fight_over;
pub use fight::target::{name_matches, wanted_target, Release};
pub use hands::ammo::choose_recipe;
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

/// The same note is not logged again within this.
const NOTE_EVERY: Duration = Duration::from_secs(5);

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
    noted: Recent<String>,
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
    given_up: Recent<u32>,
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
    recruited: Recent<u32>,
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
    pub(crate) academy_doors: Recent<u32>,
    /// When the academy rule last asked for a weapon to be wielded.
    pub(crate) academy_armed: Option<Instant>,
}

impl Autoplay {
    /// Something worth knowing that is not what the character is doing:
    /// logged, at most every few seconds for the same words, and the
    /// status left as it was. Said every tick it would drown the log
    /// and flip the status back and forth with whatever else is going on.
    pub(crate) fn note(&mut self, text: impl Into<String>, now: Instant) {
        let text = text.into();
        self.noted.expire(now, NOTE_EVERY);
        if self.noted.since(&text).is_none() {
            tracing::info!("autoplay: {text}");
            self.noted.mark(text, now);
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

    /// A skill as it stands right now, 0 when the sheet lacks it.
    fn skill_now(&self, id: u32) -> u32 {
        let stats = &self.world.stats;
        let Some(sk) = stats.skill(id) else {
            return 0;
        };
        let table = self.assets.skill_table().ok();
        stats.skill_current(sk, table.as_ref().and_then(|t| t.get(id)))
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
}
