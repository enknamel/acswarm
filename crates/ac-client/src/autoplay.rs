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

use crate::{Client, Stance, SAME_FLOOR};
// The rule vocabulary lives in ac-loot; this file still speaks it.
pub use ac_loot::profile::LootAction;

/// How long to wait for a corpse to open before asking again, when it
/// is right under our feet. A corpse further off is given time for the
/// walk as well (see [`loot_wait`]).
///
/// Short, because the character now walks to the corpse itself (see
/// [`CORPSE_REACH`]) and asks from on top of it. The first ask is
/// nonetheless often ignored -- the server still has us moving -- so
/// what matters is how quickly the second one follows.
const LOOT_TIMEOUT: Duration = Duration::from_millis(2500);
/// How many times to ask before leaving a corpse alone. Asking is
/// cheap now that it is asked from arm's length.
const LOOT_TRIES: u32 = 3;

/// A wait that has doubled this far means the stack has been asked to
/// come apart half a dozen times and has not. The party gets on
/// without that hand-over.
const WILL_NOT_SPLIT: Duration = Duration::from_secs(8);
/// Coming this much closer (metres) counts as getting somewhere.
const REACH_PROGRESS: f32 = 1.0;
/// Walking towards something for this long without getting closer is
/// not walking towards it any more.
const REACH_GIVE_UP: Duration = Duration::from_secs(20);
/// A pessimistic walking speed for pricing that walk, metres a second:
/// the way round a dungeon corner is longer than the line to it.
const LOOT_WALK: f32 = 2.5;

/// How long to allow a corpse `away` metres off to open.
///
/// Opening one asks the server to walk us there, and that walk is not
/// instant: a flat six seconds was enough for a corpse at our feet and
/// not for one across a room, so the far ones were written off unopened.
fn loot_wait(away: f32) -> Duration {
    LOOT_TIMEOUT + Duration::from_secs_f32((away.max(0.0) / LOOT_WALK).min(30.0))
}

/// Whether a refusal's code is one that gates a drop that can only be had
/// so often: YouHaveSolvedThisQuestTooRecently or TooManyTimes.
pub(crate) fn only_so_often(code: u32) -> bool {
    matches!(code, 0x043E | 0x043F)
}

/// Which item an inventory refusal (`InventoryServerSaveFailed`, `item`
/// and `err` as it came) is about, given the take in flight (`inflight`).
///
/// Usually the one it names. ACE turns a drop that can only be had so
/// often down naming no item at all, with only the quest's reason
/// (`QuestManager.HandleSolveError`). Read as a refusal of nothing, the
/// take was never known to be refused and was asked for again until the
/// corpse was given up on.
pub(crate) fn refused_item(item: u32, err: u32, inflight: Option<u32>) -> Option<u32> {
    match item {
        0 if only_so_often(err) => inflight,
        0 => None,
        item => Some(item),
    }
}

/// Everything that can stand between the pack and a tidier pack, as
/// `Client::autoplay_tidy` finds it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TidyGate {
    /// The loot profile says this character does not tidy.
    pub(crate) tidy_pack_off: bool,
    /// A vendor's window is open.
    pub(crate) counter_open: bool,
    /// Loading or unloading the party's quartermaster.
    pub(crate) quartermaster: bool,
    /// Two bundles are waiting to be made into ammunition.
    pub(crate) crafting: bool,
    /// Something was handed to a teammate a moment ago.
    pub(crate) gave_lately: bool,
    /// A take is queued or on its way from a corpse.
    pub(crate) take_in_air: bool,
}

/// Why the pack is being left alone, or `None` to go ahead.
///
/// Beyond the profile's own say-so, every one of these is a moment when
/// something else is counting on a guid staying where it is. A merge
/// makes one stack vanish into another, and whatever was holding it --
/// a sale list, the money counted out for a runner, a take the server
/// has not answered -- is then waiting on something that does not exist.
fn why_not_tidy(g: TidyGate) -> Option<&'static str> {
    if g.tidy_pack_off {
        // Nothing stops a player tidying by hand; this is only the
        // rules keeping their hands off a pack they were told to.
        Some("this profile leaves the pack as it is")
    } else if g.counter_open {
        // A run to town holds what it has sent the vendor by guid, and
        // a merge makes one of those vanish mid-sale. Whatever was
        // bought is tidied the moment the window closes.
        Some("a counter is open")
    } else if g.quartermaster {
        // Money counted out for the runner is a stack of its own, and
        // tidying poured it straight back into the pile it came from:
        // count out, merge back, count out again, a hundred and twenty
        // six times in one watched run.
        Some("the quartermaster is being loaded or unloaded")
    } else if g.crafting {
        // The heads and the shafts are about to be used on each other.
        Some("ammunition is being made")
    } else if g.gave_lately {
        // A hand-over is a split and a give, and the server is still
        // working through it.
        Some("something was just handed to a teammate")
    } else if g.take_in_air {
        // The thing coming off the corpse may be the very stack a pour
        // would empty, and the server answers one at a time.
        Some("a take is queued or in the air")
    } else {
        None
    }
}

/// What the character has room for, as a corpse waiting on it sees it
/// (see `Client::room_for_loot`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Room {
    /// The pack is down to the slots kept for a counter's money.
    pub(crate) pack_low: bool,
    /// Carrying more than three times its capacity: the server hands it
    /// nothing at all, not even a coin.
    pub(crate) past_the_wall: bool,
    /// How much more loot it means to carry (see `Client::carry_room`).
    pub(crate) carry: u32,
}

impl Room {
    /// Room for anything.
    #[cfg(test)]
    pub(crate) const PLENTY: Room = Room {
        pack_low: false,
        past_the_wall: false,
        carry: u32::MAX,
    };
}

/// Whether an ask to open a corpse `away` metres off, sent on the tick
/// `asked`, came back the way the server answers for a thing it does not
/// have: a `UseDone` with no error (`done`), and not a word (`told`) since.
///
/// Blargerton, in the Holtburg Dungeon: ACE lets an object go once it
/// has been out of the character's sight for twenty-five seconds, and a
/// body that rots after that sends its delete only to the players who
/// still know it. The client kept such bodies for the session. It walked
/// back to one, asked, set it aside, came back to ask again, and held the
/// next fight for it meanwhile. A corpse that is there and within reach
/// opens, or says why it will not ("You do not yet have the right to
/// loot"); only one that is not there says nothing at all. From further
/// off the server walks the character over first, and a walk it cannot
/// finish ends just as quietly.
///
/// A quiet answer can also be an earlier ask's walk cut short by this
/// one, so one is not enough: every ask at a body has to come back so
/// (see `Autoplay::nothing_came_back`). Answers are stamped with the tick
/// they came in on and the ask with the tick it went out on, so an
/// answer that came in on that same tick was to an earlier ask.
fn answered_with_nothing(
    away: f32,
    asked: Instant,
    done: Option<(u32, Instant)>,
    told: Option<Instant>,
) -> bool {
    away <= CORPSE_REACH
        && done.is_some_and(|(err, at)| err == 0 && at > asked)
        && told.is_none_or(|at| at <= asked)
}

/// Why the server would not open a body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CorpseRefusal {
    /// Someone has it open this moment. The server hands a container to
    /// one viewer at a time and turns the rest away outright
    /// (`Container.InUseMessage`); that viewer is usually done with it
    /// in a moment.
    InUse,
    /// It is the killer's for now (`Corpse.Open`). A monster's body
    /// opens to everyone once it is half rotted.
    NotYetOurs,
    /// It is the killer's for good: a body that made a rare, or a
    /// player killer's doing. Neither is ever shared, however long it
    /// lies there.
    NeverOurs,
}

/// Which body the server's words named, and why it would not open it.
///
/// Matched on the English because that is all there is to match on:
/// every one of these arrives as a transient string, which carries no
/// error code. They are what ACE answers an open with and nothing else,
/// so a line that reads this way was an answer to an open -- of a
/// container, at least. That it was *our* body's open is for the caller
/// to say (see [`Autoplay::corpse_refused`]); the in-use words are sent
/// for any container, a chest as readily as a corpse.
fn corpse_refusal(text: &str) -> Option<(&str, CorpseRefusal)> {
    // "The Corpse of Hellion is already in use by someone else!" -- or
    // by a name, on a server that tells you whose (`container_opener_name`).
    if let Some((name, _who)) = text
        .strip_prefix("The ")
        .and_then(|s| s.strip_suffix('!'))
        .and_then(|s| s.split_once(" is already in use by "))
    {
        return Some((name, CorpseRefusal::InUse));
    }
    // "You do not yet have the right to loot the Corpse of Hellion."
    if let Some(name) = text.strip_prefix("You do not yet have the right to loot the ") {
        return Some((
            name.strip_suffix('.').unwrap_or(name),
            CorpseRefusal::NotYetOurs,
        ));
    }
    // "You may not loot the Corpse of Hellion because ..." -- the body
    // made a rare, or the death was a player killer's doing.
    if let Some((name, _why)) = text
        .strip_prefix("You may not loot the ")
        .and_then(|s| s.split_once(" because "))
    {
        return Some((name, CorpseRefusal::NeverOurs));
    }
    None
}
/// Least time between two casts of the same buff.
const BUFF_EVERY: Duration = Duration::from_millis(1500);
/// A target that takes no damage for this long is let go.
const STALL_AFTER: Duration = Duration::from_secs(20);
/// Walking up to a target is working on it while each stretch brings the
/// character this much nearer than it has been (see [`came_nearer`]).
const APPROACH_PROGRESS: f32 = 1.0;
/// And left alone for this long afterwards.
const GIVE_UP_FOR: Duration = Duration::from_secs(90);
/// Casting or shooting this long from one spot with nothing landing:
/// the spot is no good, and the character closes in rather than going on.
const CLOSE_IN_AFTER: Duration = Duration::from_secs(8);
/// Nearer than this, closing in again achieves nothing: give up instead.
const MIN_STAND_OFF: f32 = 6.0;
/// A corpse within this of where a kill fell is that kill's body.
const KILL_SPOT: f32 = 6.0;
/// Appraisal int: a creature's level (ACE `Level`), for the creatures
/// the table has no level for.
const CREATURE_LEVEL: u32 = 25;

/// Whether a corpse at `at` lies where one of the character's kills fell.
fn near_a_kill(at: glam::Vec3, spots: &[(glam::Vec3, Instant)]) -> bool {
    spots
        .iter()
        .any(|(k, _)| k.truncate().distance(at.truncate()) <= KILL_SPOT)
}

/// Whether a corpse lying at `at` is this character's to empty, seen from
/// where it stands (`me`): close by on its own floor, or where one of its
/// kills fell within the fight radius. The next fight waits on exactly
/// the corpses the looting takes: waiting on one it will not take is
/// waiting for good.
///
/// Close by is measured on the map and on the same floor, the way
/// arriving is (see `visit::arrived`). Measured as a straight line, a
/// body in the Holtburg Dungeon's stacked rooms, a few metres off on the
/// map and a storey above or below, counted as close. Blargerton walked
/// at it through the floor, and the next fight waited on it. A body
/// where one of his own kills fell is still his whatever the floor (it
/// may have been shot from a ledge). Whether it can be walked to is for
/// the walk to find out (see [`CorpseWalk`]).
fn corpse_is_ours(
    me: glam::Vec3,
    at: glam::Vec3,
    fight_radius: f32,
    spots: &[(glam::Vec3, Instant)],
) -> bool {
    corpse_within_reach(me, at) || (me.distance(at) <= fight_radius && near_a_kill(at, spots))
}

/// Whether a body at `at` lies close enough to `me` to be emptied from
/// where it stands: near on the map and on the same floor. See
/// [`corpse_is_ours`], where the two measures are explained.
fn corpse_within_reach(me: glam::Vec3, at: glam::Vec3) -> bool {
    me.truncate().distance(at.truncate()) <= LOOT_NEAR && (me.z - at.z).abs() <= SAME_FLOOR
}

/// How long the next fight waits on an answer about whose kill a far
/// corpse was. An appraisal comes back in well under a second; one not
/// back in this long is not coming.
const WHOSE_WAIT: Duration = Duration::from_secs(3);
/// A summoned creature last seen out this long ago may still have left
/// bodies about: it can die, or its time run out, with its last kill
/// still falling, and the character comes within reach of bodies it made
/// at the edge of the fight radius.
const PET_KILLS_FOR: Duration = Duration::from_secs(60);

/// Whether a corpse's description (`long_desc`) names this character
/// (`me`), or a creature it summoned, as its killer.
///
/// The server tells a player of a kill only when the player landed the
/// last blow. Blargerton's Mud Golem fought beside him in the Holtburg
/// Dungeon, and the bodies it finished told him nothing: no kill spot was
/// noted, so every one past twenty metres was passed by and the next
/// fight went ahead. The corpse is still his to open, and says so:
/// "Killed by Blargerton." when he did the most damage, "Killed by
/// Blargerton's Mud Golem." when his creature did. The server leaves off
/// a leading '+', and adds to the line when a rare was found on the body.
fn killed_by_us(long_desc: &str, me: &str) -> bool {
    let me = me.trim_start_matches('+');
    let Some(by) = long_desc.strip_prefix("Killed by ") else {
        return false;
    };
    let by = by.trim_start_matches('+');
    let named = by
        .get(..me.len())
        .is_some_and(|n| !me.is_empty() && n.eq_ignore_ascii_case(me));
    if !named {
        return false;
    }
    let rest = &by[me.len()..];
    rest.starts_with('.') || rest.starts_with("'s ")
}

/// Whether a corpse lying at `at` is worth asking about, to learn whose
/// kill it was: within the fight radius, and not already this
/// character's by lying close by or where one of its kills fell (see
/// [`corpse_is_ours`]).
fn whose_to_ask(
    me: glam::Vec3,
    at: glam::Vec3,
    fight_radius: f32,
    spots: &[(glam::Vec3, Instant)],
) -> bool {
    me.distance(at) <= fight_radius && !corpse_is_ours(me, at, fight_radius, spots)
}

/// Far corpses asked about while a creature this character summoned was
/// about, to learn whose kill each was (see [`killed_by_us`]).
#[derive(Debug, Default)]
pub(crate) struct Whose {
    /// Each corpse asked about, when, and whether the answer is in.
    asked: Vec<(u32, Instant, bool)>,
    /// When a creature of this character's was last seen out.
    pet_seen: Option<Instant>,
}

impl Whose {
    /// A creature of this character's is out.
    fn pet_out(&mut self, now: Instant) {
        self.pet_seen = Some(now);
    }

    /// One of its creatures has been out lately enough to have left
    /// bodies about.
    fn pet_lately(&self, now: Instant) -> bool {
        self.pet_seen
            .is_some_and(|t| now.duration_since(t) < PET_KILLS_FOR)
    }

    /// Whether `guid` has been asked about. Each corpse is asked about
    /// once: the answer does not change.
    fn has_asked(&self, guid: u32) -> bool {
        self.asked.iter().any(|(g, _, _)| *g == guid)
    }

    /// `guid` has been asked about, just now.
    fn ask(&mut self, guid: u32, now: Instant) {
        if !self.has_asked(guid) {
            self.asked.push((guid, now, false));
        }
    }

    /// The corpses asked about whose answer is not in.
    fn out(&self) -> impl Iterator<Item = u32> + '_ {
        self.asked
            .iter()
            .filter(|(_, _, done)| !done)
            .map(|(g, _, _)| *g)
    }

    /// The answer about `guid` is in.
    fn answered(&mut self, guid: u32) {
        for (g, _, done) in &mut self.asked {
            if *g == guid {
                *done = true;
            }
        }
    }

    /// An answer is on its way and has not been long about it. The next
    /// fight waits for it: it was picked in the moment before the answer
    /// came, past the body the creature had just made.
    fn waiting(&self, now: Instant) -> bool {
        self.asked
            .iter()
            .any(|(_, t, done)| !done && now.duration_since(*t) < WHOSE_WAIT)
    }

    /// Forget the corpses no longer there.
    fn tidy(&mut self, there: impl Fn(u32) -> bool) {
        self.asked.retain(|(g, _, _)| there(*g));
    }
}

/// A walk to a corpse on which the steering has found no way there for
/// this long is given up. Not at the first word of it: the steering
/// can say so for a moment before it has planned again for a new goal,
/// or while the neighbourhood planner is still working out a route on
/// its own thread.
const NO_WAY_FOR: Duration = Duration::from_secs(5);
/// A walk to a corpse that has not been pressed on for this long is a
/// new walk when it is taken up again. Another step had the ticks (a
/// fight, most likely), and time spent fighting is not the walk getting
/// nowhere.
const WALK_PAUSED: Duration = Duration::from_secs(3);

/// A walk to a corpse that the looting set going, and how it is getting on.
///
/// Blargerton, in the Holtburg Dungeon, walked every tick towards bodies
/// lying through a floor or behind a wall. Nothing timed that walk,
/// because the opening's clocks only start once the corpse is used. So
/// the looting held him, and the next fight waited on the body, until
/// it rotted.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CorpseWalk {
    /// The corpse being walked to.
    pub(crate) guid: u32,
    /// When the walk began, for the log.
    started: Instant,
    /// The nearest it has been got to, and when that last improved.
    best: f32,
    since: Instant,
    /// Since when the steering has found no way there, without a break.
    no_way_since: Option<Instant>,
    /// When the walk was last pressed on.
    last: Instant,
}

impl CorpseWalk {
    fn new(guid: u32, away: f32, now: Instant) -> Self {
        CorpseWalk {
            guid,
            started: now,
            best: away,
            since: now,
            no_way_since: None,
            last: now,
        }
    }

    /// Walk on, `away` metres from the corpse, with the steering having
    /// found `no_way` there or not. False once the walk cannot arrive:
    /// no way there for [`NO_WAY_FOR`], or no nearer for
    /// [`REACH_GIVE_UP`].
    ///
    /// That is the rule of `Client::reaching_too_long`, kept on a clock of
    /// its own. The hand-overs share that one, and it outlives the walk
    /// it timed, so a second walk to the same corpse would have started
    /// out of time. Nearer is measured as a straight line, not on the
    /// map, so that going down a stair to a body on the floor below
    /// counts as getting somewhere.
    fn goes_on(&mut self, away: f32, no_way: bool, now: Instant) -> bool {
        if now.duration_since(self.last) > WALK_PAUSED {
            *self = CorpseWalk::new(self.guid, away, now);
        }
        self.last = now;
        if away < self.best - REACH_PROGRESS {
            self.best = away;
            self.since = now;
        }
        self.no_way_since = no_way.then(|| self.no_way_since.unwrap_or(now));
        let no_way = self
            .no_way_since
            .is_some_and(|t| now.duration_since(t) >= NO_WAY_FOR);
        !no_way && now.duration_since(self.since) <= REACH_GIVE_UP
    }
}

/// The creature a line says was reached and not hurt: "X resists your
/// spell" (ACE `TryResistSpell`, projectile or not) or "X evades your
/// attack.". Either way the shot got there.
fn arrived_unharmed(text: &str) -> Option<&str> {
    text.strip_suffix(" resists your spell")
        .or_else(|| text.strip_suffix(" evades your attack."))
}

/// Who a line says has just cast a spell at this character to hurt it: a
/// war spell that landed ("Drudge Shaman blasts you for 12 points with
/// Flame Bolt I."), a drain ("Drudge Shaman casts Harm Other I and drains
/// 9 points of your health."), a vital taken ("You lose 20 points of mana
/// due to Drudge Shaman casting Mana to Health Other I on you"), or a
/// spell resisted ("You resist the spell cast by Drudge Shaman").
///
/// ACE tells a character it was hit by a swing with a notification, and
/// by a spell with nothing but one of these lines (`SpellProjectile`,
/// `WorldObject_Magic`). A caster working on the character from twenty
/// metres never closes in to swing, so without these it never counted as
/// attacking it at all.
///
/// "X cast Y on you" is left out on purpose: a fellow's buff reads the
/// same as a monster's curse, and a buff is not a fight.
fn spell_attacker(text: &str) -> Option<&str> {
    if let Some(who) = text.strip_prefix("You resist the spell cast by ") {
        return Some(who);
    }
    if let Some(rest) = text.strip_prefix("You lose ") {
        let (_, by) = rest.split_once(" due to ")?;
        let (who, _) = by.strip_suffix(" on you")?.split_once(" casting ")?;
        return Some(who);
    }
    // A bolt that landed can come with any of these in front of it.
    let mut line = text;
    while let Some(rest) = ["Critical hit! ", "Overpower! ", "Sneak Attack! "]
        .into_iter()
        .find_map(|p| line.strip_prefix(p))
    {
        line = rest;
    }
    if let Some((before, after)) = line.split_once(" you for ") {
        // The verb is one word, and says how hard: "blasts", "singes".
        return after
            .contains(" points with ")
            .then(|| before.rsplit_once(' ').map(|(who, _)| who))
            .flatten()
            .filter(|who| !who.is_empty());
    }
    let (who, rest) = line.split_once(" casts ")?;
    (rest.contains(" and drains ") && rest.contains(" points of your ")).then_some(who)
}

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
/// Whether a change of hands must wait for the swing in flight.
///
/// True only when both hold: the hands do not already give the stance
/// wanted, so something would have to be wielded or put away; and a
/// swing or a charge is out unanswered. ACE turns every combat-mode
/// change into a cancelled attack, and putting the weapon in hand away
/// to reach for a wand is one -- so a buff pass that wants a wand waits
/// for the swing to land rather than taking the charge down with it. A
/// pass that already holds a wand changes nothing and casts at once.
///
/// The wait costs a second of a buff's life. +Verity's cost her the
/// fight: she reached for her wand every second and a half for a minute,
/// and the Drudge Servant she was charging was never once reached.
fn change_of_hands_waits(have: Stance, want: Stance, mid_attack: bool) -> bool {
    have != want && mid_attack
}

/// How near to running out a buff must be, in seconds, before this pass
/// will put it back.
///
/// The quiet pass tops up anything within the wide `top_up_within`. The
/// urgent one runs as a reflex, ahead of loot and ahead of the fight,
/// and puts back whatever is under `never_below` -- but it ignored
/// `out_of_combat_only` outright, which is the player saying the buff
/// pass must keep its hands off mid-fight. A buff with a minute still
/// on it was enough to put the sword away and reach for a wand in the
/// middle of a swing.
///
/// What that setting is protecting is the weapon, so that is what this
/// asks about. With a wand already in hand (`wand_in_hand`) a recast
/// costs a cast and nothing else, and `never_below` holds as it always
/// did: a protection going down in the middle of a fight is how a
/// character dies, and waiting for the buff to lapse before putting it
/// back is exactly the window `never_below` exists to close. It is only
/// when the recast would cost the character its weapon that an engaged
/// character with the setting on waits for a buff to actually lapse --
/// nought seconds left, which is what a buff that is not up at all
/// reads as -- and the rest wait for the fight to end.
fn buff_within(cfg: &Buffs, urgent: bool, fighting: bool, wand_in_hand: bool) -> f32 {
    if !urgent {
        return cfg.top_up_within;
    }
    if fighting && cfg.out_of_combat_only && !wand_in_hand {
        return 0.0;
    }
    cfg.never_below
}

/// A change of weapon is asked for at most this often.
const REWIELD_EVERY: Duration = Duration::from_millis(1000);
/// How long an item the server refused to wield is left alone the first
/// time. It doubles with every refusal after that. Short to begin with
/// on purpose: most refusals are about what else is in the hands, and
/// that changes within a few seconds.
const WIELD_AGAIN: Duration = Duration::from_secs(3);
/// How long a wield already asked for is given to be answered before it
/// is worth asking again.
///
/// The client thinks at 8 Hz and the server's word takes a few hundred
/// milliseconds to come back, so without this the same weapon goes out
/// two or three times over and every ask after the first is refused --
/// for the first one having worked. Nine characters sent 81 wields in
/// ten minutes and were refused 71 of them, every refusal reading "You
/// must remove your Slashing Sceptre to wield Slashing Sceptre".
const WIELD_ANSWERS_IN: Duration = Duration::from_millis(1500);
/// How long a weapon swap is given to land before the character gives
/// up waiting and fights with whatever is in its hands. A put and a
/// wield are a tick or two; anything longer means the swap is stuck.
const SWAP_SETTLES: Duration = Duration::from_millis(1500);
/// Ammunition is made at most this often: a use takes a moment and
/// the bundles need to answer.
const CRAFT_EVERY: Duration = Duration::from_secs(4);
/// How long dropping to peace mode takes on the server, which will not
/// craft in any other stance.
const STANCE_CHANGE: Duration = Duration::from_millis(1000);
/// The Fletching skill.
const FLETCHING: u32 = 37;
/// The same note is not logged again within this.
const NOTE_EVERY: Duration = Duration::from_secs(5);
/// Least time between two hand-offs, and between two salvage batches
/// when the first has not been seen to go: a batch whose items have
/// left the pack is followed by the next at once.
const SALVAGE_EVERY: Duration = Duration::from_secs(3);
/// How long a salvage batch or a hand-off is given to take effect (the
/// items leaving the pack) before it counts as refused.
const SALVAGE_TIMEOUT: Duration = Duration::from_secs(5);
/// A batch or an item refused this many times is left alone.
const SALVAGE_TRIES: u8 = 3;
/// The salvager is walked to when within this; further off, the salvage
/// waits for the team to come together.
const HAND_OFF_RANGE: f32 = 30.0;
/// Close enough to hand something over (ACE's use radius, with room).
const GIVE_REACH: f32 = 2.0;
/// How long an item that arrived in the pack waits for its appraisal
/// before the rules judge it as it is.
const TAG_TIMEOUT: Duration = Duration::from_secs(15);
/// How often the pack is judged afresh while a re-judge is waiting on
/// appraisals.
///
/// The answers cannot change faster than the server sends them, so
/// there is nothing to be had from asking every frame -- and a hundred
/// and fifty items judged sixty times a second for the fifteen seconds
/// an appraisal may take is real work for no answer.
const RETAG_EVERY: Duration = Duration::from_millis(250);
/// The Salvaging skill.
const SALVAGING: u32 = 40;
/// How often the buffs are gone through to see what is due.
const BUFF_CHECK_EVERY: Duration = Duration::from_millis(1000);
/// Least time between two attack orders.
const ATTACK_EVERY: Duration = Duration::from_millis(1200);
/// How long to wait on a cast the server never answers for. Casting is
/// paced by its answer, not by a clock; this only stops a character
/// waiting for ever on one that went astray.
const CAST_LOST: Duration = Duration::from_secs(6);

/// Staying alive.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Survive {
    /// Heal when health falls below this fraction of its maximum.
    pub heal_below: f32,
    /// Stop fighting below this fraction (0 to keep fighting).
    /// Use a carried healing kit.
    pub use_kits: bool,
    /// Keep mana up by pouring stamina into it, and stamina up with
    /// Revitalize, the way a caster does: the transfer gives more mana
    /// than the Revitalize costs, so the round is a gain.
    pub manage_mana: bool,
    /// Pour stamina into mana when mana is under this fraction.
    pub mana_below: f32,
    /// Revitalize when stamina is under this fraction.
    pub stamina_below: f32,
    /// After a death, go back for the corpse and take the gear off it
    /// (see `crate::recovery`).
    pub recover_corpse: bool,
    /// Give up on the corpse when it has not been reached in this many
    /// minutes.
    pub corpse_minutes: f32,
    /// While the vitae penalty is at least `vitae_above`, leave the
    /// hard fights and whatever killed us alone.
    pub vitae_wait: bool,
    /// The vitae penalty, as a fraction, from which the fights are
    /// picked with care: 0.25 is five deaths' worth.
    pub vitae_above: f32,
    /// Step out of the way of spells flying at us instead of standing
    /// in them (see `crate::dodge`).
    pub dodge: bool,
}

impl Default for Survive {
    fn default() -> Self {
        Survive {
            heal_below: 0.6,
            use_kits: true,
            manage_mana: true,
            mana_below: 0.4,
            stamina_below: 0.3,
            recover_corpse: true,
            corpse_minutes: 10.0,
            vitae_wait: true,
            vitae_above: 0.25,
            dodge: true,
        }
    }
}

/// Buffs to keep up.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Buffs {
    /// Work the buffs out from the character itself: every Life and
    /// Creature self-enchantment it knows for the skills it has trained,
    /// the Item auras for the way it fights, and the armour spells on
    /// each piece worn; the highest level of each (see `crate::buffs`).
    pub auto: bool,
    /// The lowest chance of a cast landing that is still worth the
    /// mana: a level that fizzles more often than this is passed over
    /// for the one below it. Half is the point where the school's skill
    /// equals the spell's power.
    pub least_chance: f32,
    /// Spell names to keep on the character as well, by hand.
    pub spells: Vec<String>,
    /// In a quiet moment, put back any buff with this many seconds or
    /// fewer left. Wide on purpose: refreshing a few at every lull
    /// spreads the work out, so the set never all runs out at once and
    /// the character is never stood still for twenty casts in a row.
    pub top_up_within: f32,
    /// A buff with this many seconds or fewer left is put back at once,
    /// fight or no fight, swapping to a wand for it if need be. A buff
    /// must never be allowed to run out: the protections going down in
    /// the middle of a fight is how a character dies.
    pub never_below: f32,
    /// Only top up out of combat. This is about the weapon: with a wand
    /// already in hand `never_below` holds mid-fight as it always did,
    /// and otherwise a buff that has actually run out still goes back
    /// up while one with time left on it waits for the fight to end
    /// rather than costing the character its weapon for a tick (see
    /// [`buff_within`]).
    pub out_of_combat_only: bool,
    /// Keep this fraction of mana back from buffing, for healing and
    /// fighting. A character that spends its last point on Quickness
    /// Self cannot heal, and buffs are the one thing that can wait.
    pub keep_mana: f32,
}

impl Default for Buffs {
    fn default() -> Self {
        Buffs {
            auto: true,
            least_chance: 0.5,
            spells: Vec::new(),
            top_up_within: 300.0,
            never_below: 60.0,
            out_of_combat_only: true,
            keep_mana: 0.35,
        }
    }
}

/// What to fight.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fight {
    pub enabled: bool,
    /// How to fight: with a weapon in hand, at range, or with spells.
    pub style: Style,
    /// Attack spells to throw, by name. Empty means every attack spell
    /// in the spellbook: of the ones that can be cast right now, the one
    /// the target is weakest to is used. Only read when fighting with
    /// magic.
    pub spells: Vec<String>,
    /// Wield the best weapon carried for whatever is being fought: the
    /// one whose element it takes most damage from, rending and
    /// criticals counted (see `crate::weapons`).
    pub pick_weapon: bool,
    /// Cast a vulnerability for the target's weakest element before
    /// fighting anything with at least this much health. 0 never does.
    pub vuln_above_health: u32,
    /// Only attack creatures whose name contains one of these; empty
    /// means anything that can be attacked.
    pub only: Vec<String>,
    /// Never attack creatures whose name contains one of these.
    pub avoid: Vec<String>,
    /// Walk past a creature the server will not let start a fight when
    /// one swing would end it and the character has outgrown it: a
    /// Rabbit, a Chicken, a Bunny (see [`beneath_fighting`]). Never a
    /// reason not to hit back.
    ///
    /// Defaulted by name, because serde fills a missing field from its
    /// type and a settings file written before this existed would
    /// otherwise turn it off.
    #[serde(default = "yes")]
    pub skip_critters: bool,
    /// Walk past what stands about while the character is on its way
    /// somewhere it decided to go: out to a hunting ground, back from
    /// town, round the counters. Getting there is the errand; the
    /// fighting is what the ground at the far end is for. Never a
    /// reason not to hit back.
    ///
    /// Defaulted by name for the same reason as `skip_critters`.
    #[serde(default = "yes")]
    pub walk_past_on_the_way: bool,
    /// Farthest creature to pick, metres.
    pub radius: f32,
    /// Make more ammunition when out, from a bundle of heads and a
    /// bundle of shafts carried, if Fletching is up to it.
    pub craft_ammo: bool,
    /// Summon a creature from an essence carried to fight beside the
    /// character (see `crate::summoning`).
    pub summon: bool,
    /// Hunt only here (see `crate::hunt`): fight what stands inside it,
    /// let what leaves it go, and come back to it. `None` hunts wherever
    /// the character is.
    pub area: Option<crate::hunt::HuntArea>,
}

impl Default for Fight {
    fn default() -> Self {
        Fight {
            enabled: true,
            style: Style::Auto,
            spells: Vec::new(),
            pick_weapon: true,
            vuln_above_health: crate::weapons::LONG_FIGHT_HEALTH,
            craft_ammo: true,
            summon: true,
            area: None,
            only: Vec::new(),
            avoid: Vec::new(),
            skip_critters: true,
            walk_past_on_the_way: true,
            radius: 25.0,
        }
    }
}

/// What a bool setting defaults to when a settings file leaves it out.
fn yes() -> bool {
    true
}

/// Which weapon the character should be holding.
///
/// How a character fights is not a setting: it follows what is in its
/// hands. A wand, orb or staff casts, a bow or crossbow shoots, a sword
/// swings. So this does not choose a stance, it chooses a weapon:
/// [`Style::Auto`] fights with whatever is already held, and the other
/// three wield a weapon of that kind first, for a character that
/// carries more than one and should only use the one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Style {
    #[default]
    Auto,
    Melee,
    Missile,
    Magic,
}

impl Style {
    pub fn label(self) -> &'static str {
        match self {
            Style::Auto => "whatever is held",
            Style::Melee => "a melee weapon",
            Style::Missile => "a bow or thrown weapon",
            Style::Magic => "a wand or staff",
        }
    }

    pub const ALL: [Style; 4] = [Style::Auto, Style::Melee, Style::Missile, Style::Magic];
}

/// Which loot profile this character reads.
///
/// Everything about looting -- what to take, what to do with it, when a
/// body outranks the next fight, whether to salvage -- is the profile's
/// (`crate::profile::Looting`). A character without one does not loot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Loot {
    /// The profile's name. None, or a name nothing on the shelf answers
    /// to, means nothing is looted at all.
    ///
    /// Defaulted by name rather than by `Default::default`, because a
    /// settings file that mentions `loot` at all and leaves this out
    /// would otherwise get an empty string -- serde fills a missing
    /// field from its type, not from the struct's own default -- and a
    /// character would quietly stop reading its profile.
    #[serde(default = "starter")]
    pub profile: String,
}

/// The profile the shelf seeds itself with, which is what a character
/// reads when nobody has said otherwise.
fn starter() -> String {
    "Starter".to_string()
}

impl Default for Loot {
    fn default() -> Self {
        Loot { profile: starter() }
    }
}

/// What this character does for the others playing alongside it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// Attacks the team's target.
    #[default]
    Fighter,
    /// Lands the debuffs on the team's target before the others hit it.
    Debuffer,
    /// Heals whoever is worst off, and fights only when everyone is
    /// healthy.
    Healer,
}

impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Role::Fighter => "fighter",
            Role::Debuffer => "debuffer",
            Role::Healer => "healer",
        }
    }
}

/// Hunting with the other characters being played.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Team {
    pub enabled: bool,
    pub role: Role,
    /// Attack whatever the team is attacking rather than picking alone.
    pub focus_fire: bool,
    /// Form a fellowship and recruit the others.
    pub fellowship: bool,
    /// The fellowship's name.
    pub fellowship_name: String,
    /// Spells a debuffer lands on the team's target, in order
    /// ("Imperil", "Magic Yield Other").
    pub debuffs: Vec<String>,
    /// Hand a teammate standing next to us what they are short of.
    pub share_supplies: bool,
    /// Ask for more when fewer than this many are carried (by name).
    pub keep_stocked: Vec<(String, u32)>,
    /// A creature with at least this much health is a hard fight, and
    /// hard fights are planned: the teammate with the highest Life
    /// Magic softens it with a vulnerability and an imperil, and the
    /// rest hold their fire until it has. 0 plans nothing.
    pub hard_fight_health: u32,
    /// Whether the rest wait for the softening before opening fire on
    /// a hard target. Off by default: the shots and spells spent before
    /// the vulnerability lands cost next to nothing, and every second
    /// the target is not being hit is a second it is hitting someone.
    pub wait_for_debuff: bool,
    /// This character leads: the others come to it, follow it about and
    /// fly when it flies. The one played by hand, usually. Without one
    /// the leader is whoever's name sorts first, and nobody follows.
    pub lead: bool,
    /// Follow the leader about (a character that leads never does).
    pub follow: bool,
    /// How close to keep to the leader, metres.
    pub follow_distance: f32,
    /// A follower fights only what stands within this of its leader,
    /// metres: further off, a monster would draw it away.
    pub fight_radius: f32,
    /// When the party stops hunting to restock, how it makes the trip,
    /// and what it does about money. How much of each thing to carry
    /// lives in `growth::Growth`.
    pub restock: crate::logistics::Restock,
}

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

/// How long an invitee the server turned down in words is left alone
/// before it is asked again, doubling with every refusal after. The
/// server answers a recruit of someone already in a fellowship, or busy
/// with something, in plain chat (ACE `Entity/Fellowship.cs`,
/// `AddFellowshipMember`), and until those words were read two mates
/// were asked ten times each every [`RECRUIT_AGAIN`] and never came.
/// "Busy" passes (a use finishes, a cast lands), so the wait starts
/// short; "already a member" is settled by that mate's own row on the
/// board, which says so within a round, and by the rightful leader
/// taking the fleet's fellowships apart (see [`rival_leader`]).
const HELD_OFF_FIRST: Duration = Duration::from_secs(10);

/// How long the team's rightful leader must be seen in a fellowship of
/// its own before this character gives up the one it founded. The
/// board's word on a mate is up to a round old and the world's on a
/// fellowship comes in its own time, so for a moment after that mate
/// quits this fellowship its row still says it is in one and the world
/// says it is not: a few rounds tell that from a second fellowship.
const YIELD_AFTER: Duration = Duration::from_secs(3);

/// Why the server would not recruit somebody, in its own words (see
/// [`recruit_refusal`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecruitRefusal {
    /// "{Name} is already a member of a Fellowship."
    AlreadyAMember,
    /// "{Name} is busy.": the server's `fellow_busy_no_recruit` rule, or
    /// a confirmation it could not put to that character.
    Busy,
    /// WeenieError 0x041E, "Your fellowship is full": the one code a
    /// recruit meets that names nobody, so it is about the last asked.
    Full,
}

/// How many a fellowship holds, the leader counted (ACE
/// `Entity/Fellowship.cs`, `MaxFellows`). A team of ten is one too
/// many, and the tenth is not asked.
const MAX_FELLOWS: usize = 9;

/// WeenieError for an invitation into a full fellowship (ACE
/// `WeenieError.YourFellowshipIsFull`).
pub(crate) const FELLOWSHIP_FULL: u32 = 0x041e;

/// The mate a recruiting refusal is about and why, from a chat line.
/// ACE answers the two refusals a recruit can meet with plain Broadcast
/// chat, not a WeenieError (`Entity/Fellowship.cs`,
/// `AddFellowshipMember`), so nothing on the wire but these words says
/// the invitation came to nothing. The name is the server's for the
/// character, `+` and all, as the board has it.
pub(crate) fn recruit_refusal(text: &str) -> Option<(&str, RecruitRefusal)> {
    let text = text.trim();
    if let Some(name) = text.strip_suffix(" is already a member of a Fellowship.") {
        return Some((name, RecruitRefusal::AlreadyAMember));
    }
    if let Some(name) = text.strip_suffix(" is busy.") {
        return Some((name, RecruitRefusal::Busy));
    }
    None
}

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

/// The mate the fleet should be following instead of this character,
/// when this character leads a fellowship it founded and that mate has
/// one of its own: the second fellowship this character gives up (see
/// [`Client::autoplay_yield_fellowship`]).
///
/// A session learns of another fellowship from two words that have to
/// agree. The world's own record says who is in *this* fellowship,
/// first-hand (`members`). The board's row for each mate says whether
/// it is in *a* fellowship (`Mate::in_fellowship`), up to a round old.
/// A mate whose row says it is in one and whom the world does not list
/// is in another. Only the mate the settled roster says leads counts:
/// the leader of the team is the one to gather it, and a leader that
/// let itself be recruited into this fellowship instead is a working
/// party, not a rival. And only one whose session will do the
/// gathering: one playing on its own, or one that asked to lead. A
/// person standing in a fellowship of their own with autoplay off
/// would recruit nobody, and a fleet given up to them would stand
/// unfellowshipped.
pub(crate) fn rival_leader<'a>(view: &'a TeamView, members: &[u32]) -> Option<&'a Mate> {
    if !view.settled || view.leader {
        return None;
    }
    view.leader_mate()
        .filter(|m| m.in_fellowship && !members.contains(&m.guid) && (m.autoplay || m.leads))
}

/// How often one character hands something to another. The server
/// takes one give at a time and answers in its own time.
const GIVE_EVERY: Duration = Duration::from_millis(700);

/// How often a stack is poured into another. The server takes one merge
/// at a time and answers in its own time.
pub(crate) const MERGE_EVERY: Duration = Duration::from_millis(600);
/// How long the tidying leaves the pack alone once weight is the only
/// thing standing between it and a tighter pack. Nothing the character
/// can do about that comes quickly: it has to sell, or hand something
/// over, and looking every tick only burns the time.
const TIDY_LADEN_WAIT: Duration = Duration::from_secs(30);
/// A corpse this close is looted, wherever it came from.
const LOOT_NEAR: f32 = 20.0;
/// How long after a killing blow its corpse is taken to be on the way.
/// The server makes the body a moment after the creature dies.
const CORPSE_APPEARS: Duration = Duration::from_secs(3);
/// Swung at this recently, the character is in a fight whether or not
/// it chose one.
const UNDER_ATTACK: Duration = Duration::from_secs(4);

/// How long a monster's corpse lasts before it rots away. ACE gives an
/// unlooted corpse no timer at all until its first heartbeat, when it
/// takes the default of five minutes and counts down from there
/// (`WorldObject_Decay`), so this is the whole window there is.
pub(crate) const CORPSE_LIFE: Duration = Duration::from_secs(300);

/// How long to leave a body someone else has open. Long enough that the
/// two of us are not asking over each other, short enough to have it the
/// moment they are done: emptying one takes a few seconds. It doubles
/// from there like any other wait.
const CORPSE_IN_USE_AGAIN: Duration = Duration::from_secs(3);

/// How long to leave a body the server says is not ours yet.
///
/// Half decay is the slowest way a body opens up, not the usual one:
/// ACE marks a corpse looted the moment anyone closes it, and a looted
/// corpse is everyone's (`Corpse.Close` sets `IsLooted`, which
/// `Corpse.HasPermission` answers on before it ever looks at the clock).
/// So the ordinary course -- the killer opens it, empties it, closes it
/// -- makes a body public within seconds of the refusal, and writing it
/// off until it had half rotted left its loot on the floor for two
/// minutes. Short, and doubling like any other wait, so a body that
/// really is locked to its killer is asked about a handful of times
/// rather than every half minute.
const CORPSE_NOT_OURS_AGAIN: Duration = Duration::from_secs(5);

/// How close to rotting a corpse has to be before it is worth breaking
/// off for. Inside this there is no second chance.
pub(crate) const CORPSE_URGENT: Duration = Duration::from_secs(75);

/// How close the character has to stand before a corpse will open.
///
/// The server will not hand over a container we are not standing at: a
/// use from across the room is answered by being told to walk there,
/// and it waits for us to arrive. A client that never walks waits for
/// ever, which is what every corpse that "did not open" turned out to
/// be. Two and a half metres is inside the use radius of everything
/// that leaves a body.
const CORPSE_REACH: f32 = 2.5;

/// How far a character may drift from a body it already has open and
/// still walk back to it holding it (see `Client::autoplay_loot`).
///
/// Generous, because the drift is not the character's doing: a dodge, a
/// knock-back or a fight that came to it moves it several metres in a
/// frame, and the body is still well inside the twenty the looting calls
/// its own ([`LOOT_NEAR`]). Further off than this it has really gone,
/// and the body is shut and walked back to like any other.
const HOLD_ON_WITHIN: f32 = CORPSE_REACH * 4.0;

/// Whether a character `away` metres from the body it has open walks
/// back to it still holding it, rather than shutting it and starting
/// again (see [`HOLD_ON_WITHIN`]).
fn still_holding_at(away: f32) -> bool {
    away <= HOLD_ON_WITHIN
}

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

impl Default for Team {
    fn default() -> Self {
        Team {
            enabled: false,
            role: Role::Fighter,
            focus_fire: true,
            fellowship: true,
            fellowship_name: "acswarm".into(),
            debuffs: Vec::new(),
            share_supplies: true,
            keep_stocked: vec![("Healing Kit".into(), 1)],
            hard_fight_health: 400,
            wait_for_debuff: false,
            lead: false,
            follow: true,
            follow_distance: 4.0,
            fight_radius: 25.0,
            restock: crate::logistics::Restock::default(),
        }
    }
}

/// What one of the others has told us about itself. The host fills this
/// in from the bus every frame (see `ac_plugin::team`); the rules here
/// only read it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mate {
    pub name: String,
    pub guid: u32,
    /// Its session index in its own process.
    pub session: usize,
    pub world: glam::Vec3,
    pub health: f32,
    pub role: Role,
    pub target: Option<u32>,
    pub target_name: String,
    pub in_fellowship: bool,
    /// Items it is short of, by name.
    pub wants: Vec<String>,
    /// Targets it has already debuffed.
    pub debuffed: Vec<u32>,
    /// The body it has open or is walking to, and how long it has been
    /// at that one: its claim on it. The others leave a claimed body
    /// alone (see [`TeamView::working`]). The age travels with the
    /// claim because it is the claimant's own clock that says whether
    /// the claim is still good, not the clock of whoever reads it.
    pub looting: Option<u32>,
    pub looting_for: Duration,
    /// True for the one that picks the targets.
    pub leader: bool,
    /// Its Life Magic as it stands, buffs counted: what decides who
    /// softens a hard target.
    pub life_magic: u32,
    /// It knows a vulnerability or an imperil it can cast right now.
    pub can_soften: bool,
    /// It asked to lead (see `Team::lead`).
    pub leads: bool,
    /// It is flying (no-clip); followers fly too.
    pub flying: bool,
    /// The cell it stands in: indoors (a hub, a dungeon) is somewhere
    /// a journey cannot be planned to from outside.
    pub cell: u32,
    /// Level and experience, total and unspent, as the sheet has them
    /// (0 before it arrives). The fleet view works XP an hour out of
    /// the total over time.
    pub level: i32,
    pub total_xp: i64,
    pub available_xp: i64,
    /// Stamina and mana as fractions of their maximum, like `health`.
    pub stamina: f32,
    pub mana: f32,
    /// Its rules are on: it plays on its own.
    pub autoplay: bool,
    /// It follows the leader about (`Team::follow`, and not leading).
    pub following: bool,
    /// Its Salvaging as it stands, buffs counted, and whether it carries
    /// an Ust: what decides who salvages for the team.
    pub salvaging: u32,
    pub has_ust: bool,
    /// How close to empty it is, what it still has to buy and what that
    /// will cost: what the party decides hunting and restocking from.
    pub supplies: crate::logistics::Supplies,
    /// The hunting ground it is on or heading for: landblock, where,
    /// and what it is called. What the party goes back to together
    /// after a trip to town.
    pub ground: Option<(u32, glam::Vec2, String)>,
    /// It is on its way somewhere, or keeping up with a leader that is
    /// (see `Client::on_its_way`). A party on the road walks past what its
    /// leader walks past and stops for what any of it on the road is
    /// fighting, so it neither scatters to fight nor walks off and leaves
    /// one of its own behind.
    pub on_its_way: bool,
    /// Its skills that its loot rules ask about (see
    /// `Profile::skills_asked`), as `(id, base, current, advancement)`:
    /// what another character needs to judge a body on its behalf (see
    /// [`Mate::wielder`]). Empty when its rules ask about none.
    pub skills: Vec<(u32, u32, u32, u32)>,
    /// The bodies it shut lately as emptied, and what each is for the
    /// others (see [`Shut`], [`Autoplay::shuts_to_say`]).
    pub shut: Vec<Shut>,
    /// How many bodies it opened first lately, within [`DEAL_WINDOW`]:
    /// its turns at the bodies (see [`TeamView::opens_first`]). Going back
    /// for what one of the others shut first and left for it is no turn.
    pub opened_first: u16,
    /// It opens bodies at all: it has loot rules to go by, and its pack is
    /// neither down to the slots kept for a counter's money nor past the
    /// server's wall (see `Client::opens_bodies`). One that does not is
    /// never dealt a body nor left anything on one: an Ust carrier with no
    /// loot profile never came, and the salvage left for it rotted.
    pub opens_bodies: bool,
}

/// What a character that shut a body as emptied says about it to the
/// others, each by player guid (see [`judge_shut`]).
///
/// Every body shut as emptied is said, whomever it is done for, so that
/// the others know who has shut it: nothing on it is sent back to one of
/// those (see [`Autoplay::may_be_sent`]), and going back to it is no turn.
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

impl Mate {
    /// The mate as the turns at a newly fallen body read it; `None` for
    /// one that would not open a body at all: played by hand, dead, not
    /// in the world yet, or not opening bodies ([`Mate::opens_bodies`]).
    pub fn turn(&self) -> Option<Turn> {
        (self.autoplay && self.health > 0.0 && self.guid != 0 && self.opens_bodies).then(|| Turn {
            guid: self.guid,
            world: self.world,
            looting: self.looting.is_some(),
            fighting: self.target.is_some(),
            room: !self.supplies.pack_full && !self.supplies.laden,
            opened: self.opened_first,
        })
    }

    /// Whether the mate could come for something left for it on a body at
    /// `at` (see [`called_to`]): it would open a body at all, it stands
    /// within reach of this one, and it has room to carry what is left.
    /// One at another body or fighting still could, once it is done. One
    /// dead, across the field, with no loot rules or a pack its own looting
    /// would not take from could not, and nothing is left waiting on it;
    /// nor on one gone from the board, which is not asked about at all.
    pub fn could_come_for(&self, at: glam::Vec3) -> bool {
        self.turn()
            .is_some_and(|t| t.room && corpse_within_reach(t.world, at))
    }

    /// The mate as a loot rule sees the character reading it: its level
    /// and the skills it said its rules ask about. A rule asks nothing
    /// else of the character but its name (see `ac_loot::profile::Mine`).
    pub fn wielder(&self) -> crate::weapons::Wielder {
        crate::weapons::Wielder {
            level: self.level.max(0) as u32,
            skills: self.skills.clone(),
            ..Default::default()
        }
    }
}

/// Health left below which there is no time to be careful: the biggest
/// heal goes out at once, whatever it costs and whatever it drains.
const CRITICAL_HEALTH: f32 = 0.25;
/// A heal picked to cover a known wound has to be likely to land. Half
/// is where the school's skill equals the spell's power; under that the
/// mana is as likely to be thrown away as spent.
const RELIABLE_CAST: f32 = 0.5;

/// One self heal the character could cast this moment, weighed for what
/// it would give *this* character.
///
/// A boost restores the same however hurt it is; a transfer's gain is a
/// share of the bar it draws from, so the same Stamina to Health is
/// worth two hundred points on a full bar and nothing on an empty one.
/// The chooser cannot know that from the spell alone, so it is worked
/// out before it gets here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelfHeal {
    pub spell: u32,
    /// Health points it would restore, now.
    pub gain: u32,
    /// What the cast costs in mana.
    pub mana: u32,
    /// How likely it is to land rather than fizzle.
    pub chance: f32,
    /// For a transfer, the vital it draws on and the fraction of that
    /// bar left afterwards. `None` for a boost, which draws on nothing.
    pub leaves: Option<(u32, f32)>,
}

impl SelfHeal {
    /// Health this cast is worth on average: what it gives, times how
    /// often it lands.
    fn worth(&self) -> f32 {
        self.gain as f32 * self.chance
    }
}

/// Which of the heals that can be cast right now to cast, for a
/// character `missing` health points with `health` of its bar left.
///
/// A scratch should not be healed with the biggest spell in the book:
/// the mana that goes into it is the mana the next real wound needs.
/// So the cheapest heal that covers what is actually missing wins, and
/// only when nothing covers it -- or there is no time left to be
/// careful -- does the biggest go out.
fn choose_heal(heals: &[SelfHeal], missing: u32, health: f32, cfg: &Survive) -> Option<u32> {
    use ac_world::vitals::vital;
    // Dying beats saving a bar: at a quarter of the bar the next hit is
    // the last one, so the most health one cast can give goes out --
    // out of everything castable, floors and all. The floors below are
    // what the stamina and the mana were being kept for, and a dead
    // character has no later to keep them for.
    if health <= CRITICAL_HEALTH {
        let everything: Vec<&SelfHeal> = heals.iter().collect();
        return biggest_heal(&everything).map(|h| h.spell);
    }
    // A transfer that empties the bar it draws on is a trade, not a
    // heal: it buys health with the mana the next heal needs or the
    // stamina that carries the fight. Left out of the reckoning while
    // anything else will do, and only put back when nothing else will.
    let floor = |v: u32| match v {
        vital::STAMINA => cfg.stamina_below,
        vital::MANA => cfg.mana_below,
        _ => 0.0,
    };
    let sparing: Vec<&SelfHeal> = heals
        .iter()
        .filter(|h| h.leaves.is_none_or(|(v, left)| left >= floor(v)))
        .collect();
    let pool: Vec<&SelfHeal> = if sparing.is_empty() {
        heals.iter().collect()
    } else {
        sparing
    };
    pool.iter()
        .copied()
        .filter(|h| h.chance >= RELIABLE_CAST && h.worth() >= missing as f32)
        // Cheapest in mana, then the smallest of the ones that cover:
        // the rest of the heal is spilt on a full bar.
        .min_by_key(|h| (h.mana, h.gain, h.spell))
        .or_else(|| biggest_heal(&pool))
        .map(|h| h.spell)
}

/// The most health there is to be had from a single cast; ties to the
/// cheaper spell.
fn biggest_heal<'a>(pool: &[&'a SelfHeal]) -> Option<&'a SelfHeal> {
    pool.iter().copied().max_by(|a, b| {
        a.worth()
            .total_cmp(&b.worth())
            .then(b.mana.cmp(&a.mana))
            .then(b.spell.cmp(&a.spell))
    })
}

/// Whether a spell puts health back: a health boost (Harm Self is a
/// boost below zero and is not one) or a transfer into health.
fn restores_health(spell: u32) -> bool {
    use ac_world::vitals::vital;
    ac_world::vitals::boost(spell).is_some_and(|b| b.vital == vital::HEALTH && b.restores())
        || ac_world::vitals::transfer(spell).is_some_and(|t| t.to == vital::HEALTH)
}

/// Where a vital's current and maximum sit on the stats block: health
/// 0, stamina 1, mana 2.
fn vital_slot(vital: u32) -> usize {
    match vital {
        ac_world::vitals::vital::STAMINA => 1,
        ac_world::vitals::vital::MANA => 2,
        _ => 0,
    }
}

/// Who salvages for the team, out of `mates` (the caller includes
/// itself): the highest Salvaging among those with an Ust, ties to the
/// name that sorts first. `None` when nobody carries an Ust.
pub fn best_salvager<'a>(mates: impl Iterator<Item = &'a Mate>) -> Option<(String, u32)> {
    mates
        .filter(|m| m.has_ust && m.guid != 0)
        .max_by(|a, b| {
            a.salvaging
                .cmp(&b.salvaging)
                .then_with(|| b.name.cmp(&a.name))
        })
        .map(|m| (m.name.clone(), m.guid))
}

/// What means a thing on a body for one of a party in particular, rather
/// than for whoever has the body open (see [`called_to`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Calling {
    /// The rule that takes it asks about this skill: it is for the highest
    /// of that skill, buffs counted, as the rule itself reads it.
    Skill(u32),
    /// The rules would salvage it: it is for whoever salvages for the team
    /// (see [`best_salvager`]). Everyone has Salvaging, so a salvage rule
    /// asks about it without saying so.
    Salvage,
}

/// One rule's claim on a thing for one character (see [`claim_on`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Claim {
    /// Where the claim is read in the profile: 0 for the always list,
    /// which is read before any rule, and each rule's place after that.
    pub order: usize,
    /// What means what it claims for one of the party in particular, if
    /// anything does.
    pub calling: Option<Calling>,
}

/// The claim the loot rules make on a thing for the character `me`, called
/// `name` and already carrying `held` of it: `None` when they would not
/// take it, or cannot say until it is appraised.
///
/// Read as [`judge_loot`] reads it, the player's never and always lists
/// first. What those lists take is nobody's in particular, nor is what a
/// rule takes that asks nothing of the character. A salvage rule calls for
/// the salvager while the salvager salvages (`Looting::salvage`); any other
/// rule that asks about a skill calls for the best at the first it names.
pub fn claim_on(
    stats: &crate::items::ItemStats,
    id: Option<&ac_net::messages::Appraisal>,
    profile: &crate::profile::Profile,
    me: &crate::weapons::Wielder,
    name: &str,
    held: u32,
) -> Option<Claim> {
    if name_matches(&stats.name, &profile.looting.never) {
        return None;
    }
    if name_matches(&stats.name, &profile.looting.always) {
        return Some(Claim {
            order: 0,
            calling: None,
        });
    }
    let (at, rule) = profile.decided_by(stats, id, me, name, held)?;
    if !rule.action.takes() {
        return None;
    }
    let calling = if rule.action == LootAction::Salvage && profile.looting.salvage {
        Some(Calling::Salvage)
    } else {
        rule.skill_asked().map(Calling::Skill)
    };
    Some(Claim {
        order: at + 1,
        calling,
    })
}

/// One of a party a thing on a body could be for, as the loot rules read
/// it: its player guid, its name, its sheet, and how many of the thing it
/// carries (not known of a mate, which is taken to carry none).
#[derive(Clone, Copy, Debug)]
pub struct Taker<'a> {
    pub guid: u32,
    pub name: &'a str,
    pub sheet: &'a crate::weapons::Wielder,
    pub held: u32,
}

/// A fellow a thing on a body could be meant for, as its row says it:
/// player guid, name and sheet (see `Client::callable_to`).
type Fellow = (u32, String, crate::weapons::Wielder);

/// This character (`me`) and the `fellows` a thing could be meant for
/// instead, as [`called_to`] reads them.
fn takers_with<'a>(me: Taker<'a>, fellows: &'a [Fellow]) -> Vec<Taker<'a>> {
    std::iter::once(me)
        .chain(fellows.iter().map(|(guid, name, sheet)| Taker {
            guid: *guid,
            name,
            sheet,
            held: 0,
        }))
        .collect()
}

/// Which of `takers` a thing on a body is meant for, when the rules mean
/// it for one of the party in particular: `None` when they do not, and
/// then whoever has the body open takes it or leaves it by its own reading.
///
/// "If an item has a skill requirement in the loot settings, it should go
/// to whoever has the highest skill." Each taker's reading of the thing is
/// one rule's claim (see [`claim_on`]), and the first claim in the
/// profile's order that calls for someone decides, the way a character's
/// own first matching rule decides for it. It is for one of the takers
/// that same rule claims it for:
/// - a skill: the highest of that skill, buffs counted, which is what the
///   rule reads; ties to the name that sorts first, as the salvager's are
///   broken (see [`best_salvager`]);
/// - salvage: `salvager`, the team's, when it is one of them. When it is
///   not (out of reach, say), the thing is nobody's in particular, and the
///   hand-off carries it to the salvager afterwards as it always has.
pub fn called_to(
    stats: &crate::items::ItemStats,
    id: Option<&ac_net::messages::Appraisal>,
    profile: &crate::profile::Profile,
    takers: &[Taker],
    salvager: Option<u32>,
) -> Option<u32> {
    let claims: Vec<(&Taker, Claim)> = takers
        .iter()
        .filter_map(|t| Some((t, claim_on(stats, id, profile, t.sheet, t.name, t.held)?)))
        .collect();
    let (first, calling) = claims
        .iter()
        .filter_map(|(_, c)| Some((c.order, c.calling?)))
        .min_by_key(|(order, _)| *order)?;
    let mut claimed_for = claims
        .iter()
        .filter(|(_, c)| c.order == first)
        .map(|(t, _)| *t);
    match calling {
        Calling::Salvage => salvager.filter(|g| claimed_for.any(|t| t.guid == *g)),
        Calling::Skill(skill) => claimed_for
            .max_by(|a, b| {
                a.sheet
                    .skill(skill)
                    .cmp(&b.sheet.skill(skill))
                    .then_with(|| b.name.cmp(a.name))
            })
            .map(|t| t.guid),
    }
}

/// Of `me`'s skills, the ones in `asked`: what a character puts on its
/// board row for the others to judge a body by (see [`Mate::skills`]).
pub fn skills_asked_of(me: &crate::weapons::Wielder, asked: &[u32]) -> Vec<(u32, u32, u32, u32)> {
    me.skills
        .iter()
        .filter(|(id, ..)| asked.contains(id))
        .copied()
        .collect()
}

/// A thing still on a body a shut is judged on: what it is, its appraisal
/// if one came back, and how many of it this character carries already.
#[derive(Clone, Debug)]
pub struct Left<'a> {
    pub stats: crate::items::ItemStats,
    pub id: Option<&'a ac_net::messages::Appraisal>,
    pub held: u32,
}

/// Whether the rules would have the character with `sheet`, called `name`
/// and carrying `held` of it already, take `left` off a body.
///
/// Read the way the looting reads a body it stands over (see
/// `Client::corpse_now`): wanted is what a rule takes, and a thing a rule
/// could claim once appraised is wanted while appraising is allowed,
/// because the character would ask.
fn would_take(
    left: &Left,
    profile: &crate::profile::Profile,
    sheet: &crate::weapons::Wielder,
    name: &str,
    held: u32,
) -> bool {
    use crate::profile::Verdict;
    match judge_loot(&left.stats, left.id, Some(profile), sheet, name, held) {
        Verdict::Decided(action, _) => action.takes(),
        Verdict::NeedsId(_) => profile.looting.appraise,
        Verdict::None => false,
    }
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

/// Where an item stands for salvaging, by its workmanship.
///
/// The server puts everything of one material salvaged in one go into
/// the same bag, and the bag's workmanship is the average of what went
/// in (ACE `TryAddSalvage`); a bag already carried is never added to.
/// So a workmanship 10 Iron mace salvaged beside a workmanship 6 one
/// makes a bag of 8, and the 10 is wasted. Skill cannot make up for it:
/// it decides how many units come out, never their workmanship. Below 9
/// nobody minds the averaging and everything goes in together; a 9 is
/// salvaged only with 9s, and a 10 only with 10s.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum SalvageGrade {
    /// Below workmanship 9.
    Common,
    Nine,
    Ten,
}

impl SalvageGrade {
    fn of(workmanship: f32) -> Self {
        if workmanship >= 10.0 {
            Self::Ten
        } else if workmanship >= 9.0 {
            Self::Nine
        } else {
            Self::Common
        }
    }
}

/// One salvage's worth out of `items` (guid, workmanship, and how many
/// times a salvage of it has come to nothing, in the order they are to
/// go), and the grade they share. `None` with nothing to salvage.
///
/// What has come to nothing least goes first, and of that every 10 when
/// there is one, else every 9, else the rest. The best go first, so that
/// ordinary loot turning up between batches never keeps them waiting;
/// each grade is a batch, and so a turn, of its own.
///
/// The refusals come before the grade because ACE skips some items
/// without a word -- a Retained one, say. Chosen as the best grade every
/// time, a 10 like that went out alone again after each timeout, and the
/// 9s and everything below waited behind three of them, where a single
/// salvage of everything used to take the rest at once.
fn next_salvage_batch(
    items: impl IntoIterator<Item = (u32, f32, u8)>,
) -> Option<(SalvageGrade, Vec<u32>)> {
    use std::cmp::Reverse;
    let turns: Vec<(u32, (Reverse<u8>, SalvageGrade))> = items
        .into_iter()
        .map(|(guid, workmanship, refused)| {
            (guid, (Reverse(refused), SalvageGrade::of(workmanship)))
        })
        .collect();
    let first = turns.iter().map(|(_, turn)| *turn).max()?;
    let batch = turns
        .into_iter()
        .filter(|(_, turn)| *turn == first)
        .map(|(guid, _)| guid)
        .collect();
    Some((first.1, batch))
}

/// The team as the host last saw it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TeamView {
    pub mates: Vec<Mate>,
    /// Whether this character is the one picking targets.
    pub leader: bool,
    /// What this character said about itself as the view was made (see
    /// `ac_plugin::team::describe`): what the others read it by, and so
    /// what the turns at a newly fallen body read it by too (see
    /// [`Autoplay::ours_to_open`]). `None` in a view nobody said anything
    /// into.
    pub me: Option<Mate>,
    /// The roster behind this view has stood unchanged for a full board
    /// round (see `ac_plugin::team::Settling`). Until it has, `leader`
    /// is only what this session knows so far: a session that has just
    /// come onto the team has heard nobody and leads a roster of one.
    /// The fellowship rules found or give up a fellowship on the leader
    /// flag only once the view has settled; everything else reads the
    /// flag as it comes.
    pub settled: bool,
}

impl TeamView {
    /// The one leading, if it is one of the others.
    pub fn leader_mate(&self) -> Option<&Mate> {
        self.mates.iter().find(|m| m.leader)
    }

    /// The target the team is on: the leader's, else the first anyone has.
    pub fn target(&self) -> Option<(u32, String)> {
        let leader = self
            .mates
            .iter()
            .find(|m| m.leader)
            .and_then(|m| m.target.map(|t| (t, m.target_name.clone())));
        leader.or_else(|| {
            self.mates
                .iter()
                .find_map(|m| m.target.map(|t| (t, m.target_name.clone())))
        })
    }

    /// Whether anyone has already landed the debuffs on `target`.
    pub fn debuffed(&self, target: u32) -> bool {
        self.mates.iter().any(|m| m.debuffed.contains(&target))
    }

    /// Whether one of the others has the body `guid` open or is on its
    /// way to it.
    ///
    /// The server hands a container to one viewer and refuses everyone
    /// else outright (ACE `Container.CheckUseRequirements`), so a body
    /// two characters want is a body one of them empties and the other
    /// asks about until it rots. Nine characters standing on one tile
    /// see the same bodies and rank them by the same rule, so without
    /// this they all open the same one: a nine-character run opened 41
    /// bodies 1,386 times, against 2.75 times each for one character
    /// hunting alone.
    ///
    /// A claim goes stale ([`CLAIM_STALE`]): one that never did would
    /// let a claimant that stalled lock a body for its whole life. One
    /// that has died is dropped at once rather than waited out: its
    /// client keeps saying what it was working, and the same board row
    /// that says so already says its health is nothing.
    pub fn working(&self, guid: u32) -> bool {
        self.mates
            .iter()
            .any(|m| m.health > 0.0 && m.looting == Some(guid) && m.looting_for < CLAIM_STALE)
    }

    /// Whether one of the others has a better claim on `guid` than this
    /// character's own, which has stood for `ours`; `me` is this
    /// character's player guid.
    ///
    /// Better is older. Two claims made within a board round of each
    /// other are the same moment -- each session ages its own claim on
    /// its own clock and hears the others' a board round late, so
    /// nothing finer than that can be told apart -- and there the lower
    /// player guid wins, which is an answer both sides reach.
    ///
    /// Without this a body two characters chose in the same tick was
    /// opened by neither: each heard the other's claim half a second
    /// later, each read it as "someone else has it", and each stood
    /// off, while the walk both had started went on being published as
    /// a claim until every claim in it aged out at once.
    pub fn outranks_our_claim(&self, guid: u32, ours: Duration, me: u32) -> bool {
        self.mates.iter().any(|m| {
            m.health > 0.0
                && m.looting == Some(guid)
                && m.looting_for < CLAIM_STALE
                && match m.looting_for.checked_sub(ours) {
                    Some(older) if older > SAME_MOMENT => true,
                    _ => ours.saturating_sub(m.looting_for) <= SAME_MOMENT && m.guid < me,
                }
        })
    }

    /// Whose turn it is to open the body `guid`, lying at `at`, `me` being
    /// this character as the others read it: the player guid of one of
    /// those standing over it.
    ///
    /// This is only for the moment before a claim can have reached the
    /// board -- the word goes out every half second and a body is
    /// chosen within a tick of falling -- and in that moment the fleet
    /// needs an answer every session reaches on its own. All of them work
    /// this one out of the same roster, so there is nothing to negotiate
    /// and nothing to vote on, which is how the leader is settled too
    /// (`ac_plugin::team`).
    ///
    /// Turns go round, a body a turn: the one that has opened the fewest
    /// bodies first lately ([`Mate::opened_first`]), and among those the
    /// first in the deal for this body ([`deal`]), so bodies falling
    /// together go to different characters. It used to be the lowest
    /// guid, which opened every body that fell while it stood over one
    /// and carried the whole party's loot.
    ///
    /// Best effort. Only one free to open it is dealt a turn: playing on
    /// its own, alive, in the world, standing over it, not at another body,
    /// not fighting, and with room in its pack. With nobody free in reach
    /// it is ours to walk to, as it is everyone's, since standing off from
    /// a body nobody will open leaves it lying. And the turn only holds
    /// for the first second ([`CLAIM_SETTLE`]): one dealt a body that does
    /// not claim it by then loses it to whoever is free first.
    ///
    /// That only holds while every session judges the same candidates,
    /// so this character passes the same tests as the others, reach among
    /// them. A body is owed to a character out to the fight radius when
    /// one of its own kills fell there, which is further than
    /// [`LOOT_NEAR`]: without the test on ourselves, a caster twenty-two
    /// metres off called a body its own while every mate's roster had it
    /// too far away to count, and two of them opened it in the same second.
    pub fn opens_first(&self, guid: u32, at: glam::Vec3, me: Turn) -> u32 {
        self.mates
            .iter()
            .filter_map(Mate::turn)
            .chain(std::iter::once(me))
            .filter(|t| !t.looting && !t.fighting && t.room)
            .filter(|t| corpse_within_reach(t.world, at))
            .min_by_key(|t| (t.opened, deal(guid, t.guid)))
            .map_or(me.guid, |t| t.guid)
    }

    /// The others a body this character empties is judged for as it is
    /// shut (see `Client::shut_for`): everyone in the world, or, while
    /// this character is in a fellowship (`fellows`, its members' player
    /// guids), its fellows only. One outside it is left to open the body
    /// or not by its own lights, as before.
    pub fn judged_at_a_shut<'a>(
        &'a self,
        fellows: Option<&'a [u32]>,
    ) -> impl Iterator<Item = &'a Mate> {
        self.mates
            .iter()
            .filter(move |m| m.guid != 0 && fellows.is_none_or(|f| f.contains(&m.guid)))
    }

    /// The mate nearest `me` that is short of something we could hand
    /// over, within `radius` metres.
    pub fn wanting(&self, me: glam::Vec3, radius: f32) -> Option<&Mate> {
        self.mates
            .iter()
            .filter(|m| !m.wants.is_empty() && m.world.distance(me) <= radius)
            .min_by(|a, b| a.world.distance(me).total_cmp(&b.world.distance(me)))
    }

    /// The mate in the worst shape, for a healer.
    pub fn worst_hurt(&self) -> Option<&Mate> {
        self.mates
            .iter()
            .filter(|m| m.health > 0.0 && m.health < 1.0)
            .min_by(|a, b| a.health.total_cmp(&b.health))
    }
}

/// Everything the character does on its own.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub enabled: bool,
    pub survive: Survive,
    pub buffs: Buffs,
    pub fight: Fight,
    pub loot: Loot,
    pub team: Team,
    /// Growing and keeping supplied over the hours (see `crate::growth`).
    pub growth: crate::growth::Growth,
    /// Getting a new character through the Training Academy (see
    /// `crate::academy`).
    pub academy: crate::academy::Academy,
}

/// Why a cast is refused, in a few words for a status line.
pub fn cast_problem(check: &crate::magic::CastCheck) -> String {
    use crate::magic::CastCheck;
    match check {
        CastCheck::Ok => "fine".into(),
        CastCheck::NotKnown => "not known".into(),
        CastCheck::NoCaster => "no wand wielded".into(),
        CastCheck::NoTarget => "the target is gone".into(),
        CastCheck::MissingComponents(m) => format!("short of {} components", m.len()),
        CastCheck::NotEnoughMana { need, have } => format!("mana {have}/{need}"),
        CastCheck::TooHard { power, skill } => format!("power {power} over skill {skill}"),
    }
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

/// The most health a creature can have and still be a critter. A Black
/// Rabbit has five, a Chicken three, a Bunny three; the smallest thing
/// in a hunting field that is really a monster -- a Gnawer Shrethlet --
/// has eight, a Gnawer Shreth fifteen, a Drudge Skulker forty-two. One
/// swing ends anything under this line, and there is nothing in it for
/// a character that can swing.
const CRITTER_HEALTH: u32 = 5;

/// Whether a creature is beneath fighting: the server will not let it
/// start a fight, it dies to a single swing, and the character has long
/// since outgrown it. A Rabbit, a Chicken, a Bunny.
///
/// Passive on its own is not the answer. A Revenant stands there until
/// it is hit, and so do Cursed Bones and a Silver Tusker: half of what
/// a hunting ground is for waits to be provoked, and those are worth
/// sixty levels or more.
///
/// Nor is passive and far below by level, which is what this asked at
/// first and what emptied the fields it was meant to tidy. ACE gives
/// the newbie-field spawns Retaliate so they do not come at a new
/// player -- generators 2007 and 5150 around Holtburg put out nothing
/// but Drudge Skulkers, Gnawer Shreths, Mites and Mosswarts, every one
/// of them passive and level 8 -- so twice the level left a level 16
/// character with nothing in reach to attack anywhere in Holtburg, and
/// no reason to walk anywhere either. The level is a poor separator in
/// any case: a Black Rabbit is level 4 and a Gnawer Shrethlet level 2.
///
/// What does separate them is how much they can take, and that is the
/// figure this leans on for anything that will fight back.
///
/// The one flag that stands on its own is "never attacks anything":
/// that is not a creature with a health bar, it is scenery with one --
/// an Egg, a Totem, a Pillar, a Reinforced Door -- and hitting it is a
/// chore, not a fight, however much health it has.
///
/// A creature with no level recorded is fought, and so is one with no
/// health recorded that could fight back: nothing is walked past on a
/// guess.
pub fn beneath_fighting(tolerance: u32, health: u32, level: Option<u32>, mine: i32) -> bool {
    use ac_world::elements::tolerance as flag;
    let Some(level) = level else {
        return false;
    };
    if tolerance & flag::PASSIVE == 0 || i64::from(level) * 2 > i64::from(mine) {
        return false;
    }
    if tolerance & flag::NO_ATTACK != 0 {
        return true;
    }
    health > 0 && health <= CRITTER_HEALTH
}

/// Whether an item is worth taking.
/// The ammunition to make for a launcher that takes `fits` (see
/// `ac_world::fletching::ammo_type`), from what is carried as `(wcid,
/// guid)` pairs, within a Fletching of `fletching`: `(recipe, heads,
/// shafts)`. The element `weakest` (the target's weakest, when known)
/// comes first, then whatever is hardest to make, which is the better
/// arrow. `None` when no pair of bundles carried makes anything the
/// launcher shoots.
pub fn choose_recipe(
    fits: u32,
    fletching: u32,
    carried: &[(u32, u32)],
    weakest: Option<ac_world::elements::Element>,
) -> Option<(&'static ac_world::fletching::Recipe, u32, u32)> {
    let held = |wcid: u32| carried.iter().find(|(w, _)| *w == wcid).map(|(_, g)| *g);
    ac_world::fletching::making(fits)
        .filter(|r| r.difficulty <= fletching)
        .filter_map(|r| Some((r, held(r.source)?, held(r.target)?)))
        .max_by_key(|(r, _, _)| {
            let hits = weakest.is_some_and(|w| r.element() == Some(w));
            (hits, r.difficulty)
        })
}

/// What the loot rules make of an item, and whether they can say yet.
///
/// The character's loot profile decides; with none, nothing is taken.
pub fn judge_loot(
    stats: &crate::items::ItemStats,
    id: Option<&ac_net::messages::Appraisal>,
    profile: Option<&crate::profile::Profile>,
    me: &crate::weapons::Wielder,
    my_name: &str,
    held: u32,
) -> crate::profile::Verdict {
    use crate::profile::Verdict;
    // No profile, nothing decided. The profile is where a player says
    // what their things are worth, and a client with nothing to read
    // should take nothing rather than guess.
    let Some(p) = profile else {
        return Verdict::None;
    };
    // The player's own word comes first, whatever any rule says.
    if name_matches(&stats.name, &p.looting.never) {
        return Verdict::Decided(LootAction::Skip, "never take these".into());
    }
    if name_matches(&stats.name, &p.looting.always) {
        return Verdict::Decided(LootAction::Keep, "always take these".into());
    }
    // The buy list is the player's word too: a line on it says "keep
    // this many of these stocked", and up to that many of the thing
    // are kept, whatever the rules below would make of it. Without
    // this the tapers a character had just bought for its line were
    // judged by "the rest, to the counter" as they arrived, tagged to
    // sell, sold on the next trip and bought again at markup, for as
    // long as the profile stood. Beyond the line the rules answer: a
    // stack over what the line asks for is loot like any other, and
    // "sell the rest" sells it. Judged here, once, when the thing is
    // taken -- the list does not answer back to a tag already written.
    let stocked = p.stocked_count(&stats.name);
    if stocked > 0 && held < stocked {
        return Verdict::Decided(LootAction::Keep, "kept stocked".into());
    }
    p.judge(stats, id, me, my_name, held)
}

/// Each carried thing paired with how many of its kind came before
/// it, oldest first.
///
/// `carried` is `(guid, wcid, stack)`. The server hands out rising ids,
/// so sorting by guid is the order the character came by the things in,
/// and the running count *before* each is what a rule with a
/// `keep_up_to` on it was answered with when they arrived one at a
/// time: `held` is what was already in the pack when the thing was
/// judged, on the corpse path and everywhere else, and a rule with a
/// cap keeps while `held` is under it.
///
/// The whole point is not to hand every item the pack's total. A rule
/// that keeps up to two rings, asked about three rings and told three
/// times that three are carried, claims none of them -- and a profile
/// edit would turn a set of keepers into a set of vendor trash in one
/// pass. Nor the count with the item itself in it, which this once
/// was: told it was the second of two, the second ring was over a cap
/// of two, and the pass kept one ring fewer than the cap.
fn in_arrival_order(carried: &mut [(u32, u32, u32)]) -> Vec<(u32, u32)> {
    carried.sort_unstable();
    let mut seen_of: std::collections::BTreeMap<u32, u32> = std::collections::BTreeMap::new();
    carried
        .iter()
        .map(|(guid, wcid, stack)| {
            let n = seen_of.entry(*wcid).or_insert(0);
            let before = *n;
            *n += stack;
            (*guid, before)
        })
        .collect()
}

/// What to write down about something that turned up in the pack
/// (given, bought, made) rather than off a corpse.
///
/// The same judgement as a corpse item's, and by the same profile: a
/// bundle of arrowheads is a bundle of arrowheads whether it came off a
/// drudge or over a counter, and it used to be judged by two different
/// sets of rules depending on which. `Keep` is written down too, where
/// this once answered only Salvage or Sell -- "the character means to
/// keep this" is exactly what the vendor side needs to hear, and
/// silence let it be sold.
pub fn arrival_tag(
    stats: &crate::items::ItemStats,
    id: Option<&ac_net::messages::Appraisal>,
    profile: Option<&crate::profile::Profile>,
    me: &crate::weapons::Wielder,
    my_name: &str,
    held: u32,
) -> Option<LootAction> {
    match judge_loot(stats, id, profile, me, my_name, held) {
        crate::profile::Verdict::Decided(LootAction::Skip, _) => None,
        crate::profile::Verdict::Decided(a, _) => Some(a),
        // Not judgeable yet, or nothing claimed it: nothing to write
        // down, and the pack keeps it either way.
        crate::profile::Verdict::NeedsId(_) | crate::profile::Verdict::None => None,
    }
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
/// [`Autoplay::stands_by`]).
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
/// [`Autoplay::take_in_shuts`]), for the log.
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
    pub room_bound: Option<(u32, u32, glam::Vec3)>,
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
    /// The target being worked on, since when, and its health when
    /// last seen to drop: a target that takes no damage for a while is
    /// out of reach, and is let go.
    engaged: Option<(u32, Instant, f32)>,
    /// The nearest the walk up to the engaged target has come to it.
    approach_best: Option<(u32, f32)>,
    /// The summoning rules' own state (see `crate::summoning`).
    pub summoning: crate::summoning::State,
    /// Who last attacked the character, and when -- a blow landed or one
    /// evaded, since a creature that misses is attacking all the same,
    /// or a spell cast at it, landed or resisted. Fought wherever it
    /// stands, hunting area or not.
    pub(crate) hit_by: Option<(String, Instant)>,
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

    /// Write down what an item was taken for (the loot pass, a script,
    /// the inventory panel): what the salvage and vendor passes do with
    /// it from now on.
    pub fn tag(&mut self, stats: &crate::items::ItemStats, action: LootAction) {
        self.ledger.remember(stats, action);
        self.seen.insert(stats.guid);
    }

    /// What was decided could not be done -- a counter that would not
    /// take it, a salvage refused. Not a new decision: the thing is
    /// still meant for what it was meant for.
    pub fn tag_failed(&mut self, guid: u32, why: impl Into<String>) {
        self.ledger.failed(guid, why, crate::holdings::unix_now());
    }

    /// What each item was taken for, by guid.
    pub fn tags(&self) -> std::collections::BTreeMap<u32, LootAction> {
        self.ledger.actions()
    }

    /// Start on a corpse: asked to open just now, with `allow` for it
    /// to do so. The loot rules start afresh with it.
    pub(crate) fn take_up_corpse(&mut self, guid: u32, now: Instant, allow: Duration) {
        self.fresh_loot_run();
        self.corpse = Some((guid, now, allow, 0));
        self.quiet_answers = 0;
    }

    /// Start the loot rules afresh, first counting what the body before
    /// came to. Every run ends here however its corpse was let go, so
    /// each body opened and each thing taken is counted once.
    fn fresh_loot_run(&mut self) {
        let run = std::mem::take(&mut self.loot_run);
        self.loot_tally.count(&run);
    }

    /// The ask to open the corpse in hand has had its wait and it has not
    /// opened: note whether that ask came back with nothing (`quiet`, see
    /// [`answered_with_nothing`]), and say whether every ask at this body
    /// so far has.
    fn nothing_came_back(&mut self, quiet: bool) -> bool {
        let Some((_, _, _, tries)) = self.corpse else {
            return false;
        };
        self.quiet_answers += u32::from(quiet);
        // One ask to start with, and one more for each try.
        self.quiet_answers > tries
    }

    /// Put down the corpse in hand, however it ended: emptied, given up
    /// on, not opening, or out of reach. Everything about emptying it
    /// goes with it. The loot rules' clock once outlived a corpse let go
    /// any way but a shut, and the next body, opened more than
    /// forty-five seconds after the first, was shut on its first step.
    fn let_go_of_corpse(&mut self) {
        self.corpse = None;
        self.appraising = false;
        self.fresh_loot_run();
    }

    /// Words from the server in answer to the ask to open the body in
    /// hand, `in_hand` being that body's name as the world has it: let
    /// the body go and leave it alone for as long as the words are
    /// worth. True if the words were about it.
    ///
    /// The server says why it will not open a body, and the client used
    /// to throw the words away and wait out [`loot_wait`] instead, then
    /// ask again three more times and be refused in the same words. Nine
    /// characters hunting one spot sent nine hundred and thirty such
    /// asks against a thousand refusals that had already arrived, each
    /// about a third of a second after the ask.
    ///
    /// Guarded as tightly as a quiet answer is (see
    /// [`answered_with_nothing`]): only words the server stamped after
    /// this ask went out -- `told` is when it last put anything in words
    /// -- and only words naming the body in hand. Words about a chest,
    /// or about a body one of the others is working, change nothing.
    pub(crate) fn corpse_refused(
        &mut self,
        text: &str,
        in_hand: &str,
        told: Option<Instant>,
        now: Instant,
    ) -> bool {
        let Some((guid, asked, ..)) = self.corpse else {
            return false;
        };
        let Some((named, why)) = corpse_refusal(text) else {
            return false;
        };
        if named != in_hand || told.is_none_or(|at| at <= asked) {
            return false;
        }
        match why {
            CorpseRefusal::InUse => self.shelved.hold(guid, CORPSE_IN_USE_AGAIN, now),
            // Not ours yet; it becomes everyone's the moment whoever
            // has it closes it, which is usually within seconds (see
            // [`CORPSE_NOT_OURS_AGAIN`]). A short wait, doubling.
            CorpseRefusal::NotYetOurs => self.shelved.hold(guid, CORPSE_NOT_OURS_AGAIN, now),
            // Nobody but the killer will ever open it. Left for good
            // rather than waited on: it still lies there, and every wait
            // that runs out is another walk back to it.
            CorpseRefusal::NeverOurs => self.shelved.note(
                guid,
                &crate::did::Did::refused("it is the killer's alone"),
                now,
            ),
        }
        match (why, self.shelved.waited(&guid)) {
            (CorpseRefusal::NeverOurs, _) | (_, None) => {
                tracing::info!("autoplay: {in_hand} ({guid:#010x}): {text} -- leaving it");
            }
            (_, Some(wait)) => tracing::info!(
                "autoplay: {in_hand} ({guid:#010x}): {text} -- trying again in {} s",
                wait.as_secs().max(1)
            ),
        }
        self.let_go_of_corpse();
        true
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
    /// [`Mate::shut`]). The newest [`SHUTS_SAID`], each for
    /// [`SHUT_SAID_FOR`].
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

    /// Whether the corpse `guid` is still waiting to be emptied: not
    /// emptied already, and not set aside for now. What looting is worth
    /// counts only these (see `crate::steps`).
    ///
    /// And only while the character has `room` for what is on it. A pack
    /// down to the slots kept for a counter's money takes nothing off a
    /// corpse, so no corpse waits on it: a character with such a pack
    /// walked to every body in reach, shut each on the spot as done with
    /// -- writing it off -- and held up the town run that makes room.
    ///
    /// Nor does a body left for its weight wait on a character still
    /// without room for what is left on it.
    ///
    /// Nor does any body wait on a character past the server's wall, which
    /// takes nothing from one. +Verity, at nearly five times her capacity,
    /// broke off her walk to town for a Pyreal the server would not hand
    /// her, and went back for it twice more. Short of the wall a body still
    /// waits however laden the character is: coins weigh nothing, and the
    /// loot rules take the light things that fit.
    ///
    /// Nor does a body this character stands by for a fellow (see
    /// [`Autoplay::stands_by`]), while that fellow could still come for it.
    pub(crate) fn corpse_waiting(&self, guid: u32, now: Instant, room: Room) -> bool {
        !room.pack_low
            && !room.past_the_wall
            && !self.looted.contains(&guid)
            && !self.shelved.held(&guid, now)
            && !self.stands_by(guid, now)
            && self
                .left_for_weight
                .get(&guid)
                .is_none_or(|burden| room.carry >= *burden)
    }

    /// Whether the corpse `guid`, lying at `at`, is still this
    /// character's to empty from where it stands (`me`). That means
    /// waiting to be emptied (see [`Autoplay::corpse_waiting`]), and its
    /// own (see [`corpse_is_ours`]). The looting chooses among these and
    /// the next fight waits on these, so the two cannot come to disagree.
    pub(crate) fn corpse_owed(
        &self,
        guid: u32,
        at: glam::Vec3,
        me: glam::Vec3,
        now: Instant,
        room: Room,
    ) -> bool {
        self.corpse_waiting(guid, now, room)
            && corpse_is_ours(me, at, self.config.fight.radius, &self.kill_spots)
    }

    /// The body this character is working and how long it has been at
    /// it: the one it has open, else the one it is walking to. This is
    /// what it tells the others so they leave that body alone (see
    /// [`TeamView::working`]).
    ///
    /// The walk counts, not just the open. Nine characters that all
    /// walk to one body and race at the far end of the walk have wasted
    /// the walk as well as the open.
    pub fn corpse_claim(&self, now: Instant) -> Option<(u32, Duration)> {
        self.corpse
            .map(|(guid, since, ..)| (guid, since))
            .or_else(|| self.walking_to.map(|w| (w.guid, w.started)))
            .map(|(guid, since)| (guid, now.saturating_duration_since(since)))
    }

    /// When the body `guid` first came into sight, or `now` for one
    /// never noted (see [`Autoplay::corpse_seen`]).
    fn corpse_first_seen(&self, guid: u32, now: Instant) -> Instant {
        self.corpse_seen
            .iter()
            .find(|(g, _)| *g == guid)
            .map_or(now, |(_, t)| *t)
    }

    /// Whether the body `guid`, lying at `at`, is this character's to
    /// open or one of the others' -- `me` being this character's player
    /// guid. Asked of every body the character is owed, by the looting
    /// and by what holds the next fight alike, so the two cannot come
    /// to disagree (see [`Autoplay::corpse_owed`]).
    ///
    /// A character alone answers yes to everything it is owed: with
    /// nobody on the board there is nothing to divide, and a body its
    /// pet killed is still its own.
    ///
    /// `mine` is where this character stands, which the turns need to
    /// judge it by the same rule as the others.
    pub(crate) fn ours_to_open(
        &self,
        guid: u32,
        at: glam::Vec3,
        me: u32,
        mine: glam::Vec3,
        now: Instant,
    ) -> bool {
        if self.team.mates.is_empty() {
            return true;
        }
        // Already committed to this body -- holding it, or walking to
        // it. A standing claim wins a tie rather than losing it: asking
        // only whether somebody claims the body made two characters
        // that chose it in the same tick both stand off, each for the
        // other, and neither ever opened it. Yield only to a claim that
        // outranks ours.
        if let Some(ours) = self
            .corpse_claim(now)
            .filter(|(g, _)| *g == guid)
            .map(|(_, held)| held)
        {
            return !self.team.outranks_our_claim(guid, ours, me);
        }
        if self.team.working(guid) {
            return false;
        }
        // Newly fallen, so nobody's claim can have reached the board
        // yet: the turns say whose it is until it has.
        let seen = self.corpse_first_seen(guid, now);
        now.saturating_duration_since(seen) >= CLAIM_SETTLE
            || self.team.opens_first(guid, at, self.my_turn(me, mine, now)) == me
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
        let dealt = self.team.opens_first(guid, at, self.my_turn(me, mine, now));
        self.team
            .mates
            .iter()
            .find(|m| m.guid == dealt)
            .map(|m| m.name.as_str())
    }

    /// How many bodies this character opened first lately, within
    /// [`DEAL_WINDOW`]: its turns, as it says them on the board (see
    /// [`Mate::opened_first`]).
    pub fn opened_first(&self, now: Instant) -> u16 {
        let lately = self
            .first_opens
            .iter()
            .filter(|t| now.saturating_duration_since(**t) < DEAL_WINDOW)
            .count();
        u16::try_from(lately).unwrap_or(u16::MAX)
    }

    /// Whether the mate `who` has said it shut the body `corpse` (see
    /// [`Autoplay::take_in_shuts`]).
    pub(crate) fn has_shut(&self, corpse: u32, who: u32) -> bool {
        self.shut_by
            .range((corpse, who, 0)..=(corpse, who, u32::MAX))
            .next()
            .is_some()
    }

    /// Set aside a corpse the character cannot walk to. It is left
    /// alone for as long as anything blocked is, so the looting and the
    /// next fight both pass it by for that long. It is not written off:
    /// the way may be clear from wherever the character stands next.
    fn set_aside_out_of_reach(&mut self, guid: u32, now: Instant) {
        self.shelved
            .note(guid, &crate::did::Did::blocked("cannot reach it"), now);
    }

    /// Forget the corpses set aside that are no longer there (`there`
    /// says which are).
    ///
    /// Only those. A wait that is up is kept, so a corpse that says no
    /// again waits twice as long. Waits used to be tidied away as they
    /// ran out, which is the moment the corpse is chosen again, so every
    /// refusal set a fresh thirty seconds: a body locked to its killer,
    /// behind a wall or holding only what was too heavy was walked back
    /// to every half minute until it rotted, and the fight broke off for
    /// it each time.
    pub(crate) fn forget_corpses_gone(&mut self, there: impl Fn(u32) -> bool) {
        self.shelved.retain(|g| there(*g));
        self.left_for_weight.retain(|g, _| there(*g));
        self.shut_by.retain(|(body, ..)| there(*body));
        self.done_with.retain(|(body, _)| there(*body));
        self.standing_by.retain(|body, _| there(*body));
    }

    /// What the lightest thing left for its weight on a body still lying
    /// about weighs (`there` says which bodies are), if anything was.
    pub(crate) fn lightest_left_for_weight(&self, there: impl Fn(u32) -> bool) -> Option<u32> {
        self.left_for_weight
            .iter()
            .filter(|(g, _)| there(**g))
            .map(|(_, burden)| *burden)
            .min()
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

    /// A spell of any kind went out less than a cast ago, so another
    /// sent now would queue behind it or be dropped.
    pub(crate) fn cast_in_flight(&self, now: Instant) -> bool {
        // The server says when a cast is finished, so that is what is
        // waited on -- not a guess at how long spells take. A heal sent
        // the moment the last one lands is the difference between
        // living and dying, and no fixed interval can be both quick
        // enough for that and slow enough never to have the next spell
        // dropped for arriving early.
        //
        // The clock that remains is a backstop, not the pacing: if the
        // server never answers at all, the character must not wait for
        // ever.
        match self.cast_sent {
            Some(t) => now.duration_since(t) < CAST_LOST,
            None => false,
        }
    }

    /// Whether this item was asked for a moment ago and the server's
    /// word could still be on its way (see [`WIELD_ANSWERS_IN`]).
    ///
    /// Asking twice for one weapon is not a wasted message but a
    /// harmful one: the second ask is refused because the first worked,
    /// and the refusal backs the item off for three seconds and then
    /// six and then twelve (see `Client::hold_off_wield`).
    pub(crate) fn wield_in_flight(&self, guid: u32, now: Instant) -> bool {
        self.wield_asked
            .is_some_and(|(g, t)| g == guid && now.duration_since(t) < WIELD_ANSWERS_IN)
    }

    /// Every guid some other errand is holding on to across ticks.
    ///
    /// Each of these is a thing another part of the rules has written
    /// down and will come back to: a weapon to wield once the hands are
    /// free, the arrows chosen for this target, the two bundles waiting
    /// to be made into ammunition, the stack counted out for a teammate.
    /// Pouring one of those into another stack makes its guid vanish
    /// under the errand, which then waits on something that no longer
    /// exists. The money one was watched: counted out, poured straight
    /// back, counted out again.
    pub(crate) fn held_by_an_errand(&self) -> [Option<u32>; 6] {
        let (craft_from, craft_to) = match self.crafting {
            Some((from, to, _)) => (Some(from), Some(to)),
            None => (None, None),
        };
        [
            self.pending_wield,
            self.wanted_ammo,
            self.put_down,
            craft_from,
            craft_to,
            self.handing.map(|(item, _)| item),
        ]
    }

    /// The server's words on an invitation that came to nothing (see
    /// [`recruit_refusal`]): the mate named is held off recruiting for a
    /// doubling wait, and the log says so once. Only a mate this
    /// character has invited is read this way: "{Name} is busy." is also
    /// what the server says of a patron who cannot take an oath just
    /// now (ACE `Player_Allegiance.cs`).
    pub(crate) fn hear_recruit_refusal(&mut self, text: &str, now: Instant) {
        let Some((name, why)) = recruit_refusal(text) else {
            return;
        };
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

    /// Hold the mate off recruiting after a refusal, and say so once.
    /// "Busy" is over in the seconds a use or a cast takes, so it is
    /// the same short wait every time: doubling, a mage that happened
    /// to be casting at each of eight asks was left out for hours. The
    /// others double, since what they wait on is slower to change.
    fn refuse_recruit(&mut self, guid: u32, why: RecruitRefusal, now: Instant) {
        if why == RecruitRefusal::Busy {
            self.held_off.forget(&guid);
        }
        self.held_off.hold(guid, HELD_OFF_FIRST, now);
        let wait = self.held_off.waited(&guid).unwrap_or(HELD_OFF_FIRST);
        let name = self
            .team
            .mates
            .iter()
            .find(|m| m.guid == guid)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| format!("{guid:#010x}"));
        let why = match why {
            RecruitRefusal::AlreadyAMember => "is already in a fellowship",
            RecruitRefusal::Busy => "is busy",
            RecruitRefusal::Full => "would not fit: the fellowship is full",
        };
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
    /// Health as a fraction of its maximum, 1.0 when unknown.
    pub fn health_fraction(&self) -> f32 {
        let stats = &self.world.stats;
        let max = stats.vital_max_current(0);
        if max == 0 {
            return 1.0;
        }
        stats.vitals[0].current as f32 / max as f32
    }

    /// The id of a known spell whose name starts with `name`, preferring
    /// the highest level learnt (the last in the spellbook order).
    pub fn spell_by_name(&self, name: &str) -> Option<u32> {
        let want = name.trim().to_lowercase();
        if want.is_empty() {
            return None;
        }
        let table = self.assets.spell_table().ok();
        // Of the family, the strongest that can be cast right now: a
        // name like "Heal Self" means the best Heal Self we can manage,
        // not the best in the book. Failing any castable, the strongest
        // known, so the reason it cannot be cast can be reported.
        let mut best_castable: Option<(u32, u32)> = None;
        let mut best_known: Option<(u32, u32)> = None;
        for id in &self.world.stats.spells {
            let sp = table.as_ref().and_then(|t| t.get(*id));
            let full = sp
                .map(|s| s.name.clone())
                .or_else(|| self.known_spells.get(id).cloned())
                .unwrap_or_default()
                .to_lowercase();
            if !full.starts_with(&want) && !full.contains(&want) {
                continue;
            }
            // Power orders a family; a spell the table lacks ranks by id.
            let power = sp.map(|s| s.power).unwrap_or(*id);
            let castable = matches!(
                self.can_cast(*id),
                crate::magic::CastCheck::Ok | crate::magic::CastCheck::NoCaster
            );
            if castable && best_castable.is_none_or(|(_, p)| power > p) {
                best_castable = Some((*id, power));
            }
            if best_known.is_none_or(|(_, p)| power > p) {
                best_known = Some((*id, power));
            }
        }
        best_castable.or(best_known).map(|(id, _)| id)
    }

    /// The strongest known boost of a vital that can be cast right now,
    /// whatever it is called: Heal Self VI and Adja's Intervention are
    /// both health boosts, and the table says so where a name would not.
    fn best_boost(&self, vital: u32) -> Option<u32> {
        let table = self.assets.spell_table().ok()?;
        ac_world::vitals::boosts_of(vital)
            .into_iter()
            .filter(|b| self.world.stats.spells.contains(&b.spell))
            .filter(|b| table.get(b.spell).is_some_and(|s| s.is_self_targeted()))
            .filter(|b| {
                matches!(
                    self.can_cast(b.spell),
                    crate::magic::CastCheck::Ok | crate::magic::CastCheck::NoCaster
                )
            })
            .max_by_key(|b| table.get(b.spell).map(|s| s.power).unwrap_or(0))
            .map(|b| b.spell)
    }

    /// The strongest known self transfer from one vital into another
    /// that can be cast right now.
    fn best_transfer(&self, from: u32, to: u32) -> Option<u32> {
        let table = self.assets.spell_table().ok()?;
        ac_world::vitals::transfers_between(from, to)
            .into_iter()
            .filter(|t| self.world.stats.spells.contains(&t.spell))
            .filter(|t| table.get(t.spell).is_some_and(|s| s.is_self_targeted()))
            .filter(|t| {
                matches!(
                    self.can_cast(t.spell),
                    crate::magic::CastCheck::Ok | crate::magic::CastCheck::NoCaster
                )
            })
            .max_by_key(|t| table.get(t.spell).map(|s| s.power).unwrap_or(0))
            .map(|t| t.spell)
    }

    /// Every self heal the character could cast this moment, each
    /// weighed for what it would give it now (see [`SelfHeal`]).
    ///
    /// A mage has more ways out of an emergency than Heal Self, and
    /// they are not the same size. Heal Self restores a fixed number of
    /// points however hurt it is; Stamina to Health takes half the
    /// stamina bar, which on a character with a full bar is far more,
    /// and is why a caster in trouble reaches for it rather than a kit.
    /// Which is worth more depends on the moment, so all of them are
    /// worked out and [`choose_heal`] picks between them.
    fn self_heals(&self) -> Vec<SelfHeal> {
        use ac_world::vitals::vital;
        let mut out: Vec<SelfHeal> = ac_world::vitals::boosts_of(vital::HEALTH)
            .into_iter()
            .filter_map(|b| self.self_heal(b.spell, ((b.low + b.high) / 2).max(0) as u32, None))
            .collect();
        // Both transfers into health, not only the stamina one: a mage
        // out of stamina with mana to spare has Mana to Health, and a
        // character that never looked at it stood there and died.
        for from in [vital::STAMINA, vital::MANA] {
            let have = self.world.stats.vitals[vital_slot(from)].current;
            for t in ac_world::vitals::transfers_between(from, vital::HEALTH) {
                if let Some(h) = self.self_heal(t.spell, t.gain(have), Some((from, t.drain(have))))
                {
                    out.push(h);
                }
            }
        }
        out
    }

    /// One heal, if the character knows it, can aim it at itself, can
    /// cast it this moment and would get anything out of it. `draws` is
    /// the vital a transfer takes from and how many points it takes.
    fn self_heal(&self, spell: u32, gain: u32, draws: Option<(u32, u32)>) -> Option<SelfHeal> {
        use ac_world::vitals::vital;
        if gain == 0 || !self.world.stats.spells.contains(&spell) {
            return None;
        }
        let sp = self.spell(spell)?;
        if !sp.is_self_targeted() || !matches!(self.can_cast(spell), crate::magic::CastCheck::Ok) {
            return None;
        }
        let mana = self.mana_cost(&sp);
        let leaves = draws.map(|(v, taken)| {
            let slot = vital_slot(v);
            let have = self.world.stats.vitals[slot].current;
            let max = self.world.stats.vital_max_current(slot).max(1);
            // Mana to Health is paid for out of the bar it drains, so
            // the cast's own cost comes off as well; a reckoning that
            // left it out called a transfer safe that ends with nothing
            // to cast the next heal with.
            let taken = taken + if v == vital::MANA { mana } else { 0 };
            (v, have.saturating_sub(taken) as f32 / max as f32)
        });
        Some(SelfHeal {
            spell,
            gain,
            mana,
            chance: self.cast_chance(spell),
            leaves,
        })
    }

    /// Why not one heal in the book can be cast, in a few words for the
    /// log. The easiest one known answers it best: the strongest
    /// usually complains only that its own components have run out,
    /// which says nothing about the rest of the book.
    fn why_no_heal(&self) -> Option<String> {
        let table = self.assets.spell_table().ok()?;
        let (spell, name) = self
            .world
            .stats
            .spells
            .iter()
            .filter(|id| restores_health(**id))
            .filter_map(|id| table.get(*id).map(|s| (*id, s)))
            .filter(|(_, s)| s.is_self_targeted())
            .min_by_key(|(_, s)| s.power)
            .map(|(id, s)| (id, s.name.clone()))?;
        Some(format!("{name} {}", cast_problem(&self.can_cast(spell))))
    }

    /// Cast `spell` and hold the next cast until the server answers for
    /// this one (see [`Autoplay::cast_in_flight`]). Every cast autoplay
    /// sends goes through here or sets the same clock itself: the heal
    /// once did neither, and a character at 16% health sent Heal Self
    /// every frame, forty times in under two seconds, until the first
    /// one went up.
    ///
    /// A cast the client declines to send earns no wait. Nothing is
    /// coming back to end one, so the whole six-second backstop would be
    /// spent standing still -- and now that the takes and the wields
    /// wait on this clock too, standing still over a corpse.
    pub(crate) fn cast_paced(&mut self, spell: u32, now: Instant) {
        if matches!(self.try_cast(spell), crate::magic::CastCheck::Ok) {
            self.autoplay.cast_sent = Some(now);
        }
    }

    /// Keep mana and stamina up the way a caster does: stamina poured
    /// into mana when mana runs low, Revitalize when stamina does. True
    /// when a spell went out.
    pub(crate) fn autoplay_vitals(&mut self, now: Instant) -> bool {
        use ac_world::vitals::vital;
        let cfg = self.autoplay.config.survive.clone();
        if !cfg.manage_mana {
            return false;
        }
        // Same again: the next draught or cast waits on the server
        // answering for the last, not on a clock.
        if self.autoplay.cast_in_flight(now) {
            return false;
        }
        let frac = |i: usize| {
            let max = self.world.stats.vital_max_current(i).max(1) as f32;
            self.world.stats.vitals[i].current as f32 / max
        };
        let (stamina, mana) = (frac(1), frac(2));
        // Stamina first: it is what mana is made from, and Revitalize is
        // cheap next to what a transfer of a full bar returns.
        if stamina < cfg.stamina_below {
            if let Some(spell) = self.best_boost(vital::STAMINA) {
                if matches!(self.can_cast(spell), crate::magic::CastCheck::Ok) {
                    self.cast_paced(spell, now);
                    self.autoplay.last_vital = Some(now);
                    self.autoplay.say(
                        Doing::Buffing,
                        format!("restoring stamina at {:.0}%", stamina * 100.0),
                    );
                    return true;
                }
            }
        }
        if mana < cfg.mana_below && stamina >= cfg.stamina_below.max(0.5) {
            if let Some(spell) = self.best_transfer(vital::STAMINA, vital::MANA) {
                if matches!(self.can_cast(spell), crate::magic::CastCheck::Ok) {
                    self.cast_paced(spell, now);
                    self.autoplay.last_vital = Some(now);
                    self.autoplay.say(
                        Doing::Buffing,
                        format!("pouring stamina into mana at {:.0}%", mana * 100.0),
                    );
                    return true;
                }
            }
        }
        false
    }

    /// Seconds left on the enchantment of a spell family, if any is up.
    fn buff_left(&self, spell: u32) -> Option<f32> {
        let table = self.assets.spell_table().ok();
        let sp = table.as_ref().and_then(|t| t.get(spell));
        match sp {
            Some(sp) => self.category_left(sp.category, sp.power),
            None => self.spell_left(spell),
        }
    }

    /// Seconds left on this exact spell: `None` when it is not up,
    /// infinity when it never runs out.
    fn spell_left(&self, spell: u32) -> Option<f32> {
        self.longest_left(|e| e.spell_id as u32 == spell)
    }

    /// Seconds left on any enchantment of this category at least as
    /// strong as `power`: Strength Self VI already up means Strength
    /// Self IV is not wanted, and the other way round it is.
    fn category_left(&self, category: u32, power: u32) -> Option<f32> {
        self.longest_left(|e| e.category as u32 == category && e.power >= power)
    }

    /// The longest any enchantment passing `keep` has left. A quest or
    /// item enchantment with no end has a duration below zero and is
    /// worth infinity here: treating it as run out had a character
    /// recasting a permanent buff every two seconds. Without the
    /// server's clock nothing can be said, and nothing is due.
    fn longest_left(&self, keep: impl Fn(&ac_world::stats::Enchantment) -> bool) -> Option<f32> {
        let now = self.session.server_time()?;
        self.world
            .stats
            .enchantments
            .iter()
            .filter(|e| keep(e))
            .map(|e| match e.remaining(now) {
                Some(left) => left as f32,
                None => f32::INFINITY,
            })
            .fold(None, |acc: Option<f32>, left| {
                Some(acc.map_or(left, |a| a.max(left)))
            })
    }

    /// Seconds left on an enchantment we put on an item, by category,
    /// from when we cast it and how long the spell lasts.
    fn item_buff_left(&self, item: u32, category: u32, now: Instant) -> Option<f32> {
        self.autoplay
            .item_buffs
            .iter()
            .filter(|(g, c, _, _)| *g == item && *c == category)
            .map(|(_, _, when, lasts)| lasts - now.duration_since(*when).as_secs_f32())
            .fold(None, |acc: Option<f32>, left| {
                Some(acc.map_or(left, |a| a.max(left)))
            })
    }

    /// The buffs this character should be wearing right now, worked out
    /// from what it is (see `crate::buffs::wanted`).
    pub fn wanted_buffs(&self) -> Vec<crate::buffs::Want> {
        // With the components and mana to try: a wand not yet in hand
        // is the one lack that does not count, since wielding one is
        // the first thing done.
        self.wanted_buffs_if(|id| {
            matches!(
                self.can_cast(id),
                crate::magic::CastCheck::Ok | crate::magic::CastCheck::NoCaster
            )
        })
    }

    /// The same, with `ready` saying which spells may be counted on
    /// besides being likely enough to land. The buffing asks for what
    /// can be cast this moment; the restock list asks for what would
    /// be cast once the pack were filled (`growth::spells_cast`), which
    /// is a different question with the same answer otherwise.
    pub(crate) fn wanted_buffs_if(&self, ready: impl Fn(u32) -> bool) -> Vec<crate::buffs::Want> {
        let Ok(table) = self.assets.spell_table() else {
            return Vec::new();
        };
        let trained = crate::buffs::trained_skills(&self.world.stats.skills);
        let least = self.autoplay.config.buffs.least_chance;
        // Likely enough to land, and ready by the caller's measure.
        let usable = |id: u32| self.cast_chance(id) >= least && ready(id);
        // Anything the armour spells could harden: armour, clothing or
        // a shield, since a cast at ourselves lands on all of it.
        let wears_armour = self.world.wielded().any(|o| {
            o.item_type & (ac_world::item_type::ARMOR | ac_world::item_type::CLOTHING) != 0
                || o.valid_locations & ac_world::equip::SHIELD != 0
        });
        // The weapon in hand decides which weapon skill is worth a buff.
        let weapon_skill = match self.combat_stance() {
            Stance::Magic => Some(0),
            _ => self
                .world
                .wielded()
                .find(|o| {
                    o.item_type
                        & (ac_world::item_type::MELEE_WEAPON | ac_world::item_type::MISSILE_WEAPON)
                        != 0
                })
                .and_then(|o| self.stats_of(o.guid))
                .map(|i| i.weapon_skill_id)
                .filter(|id| *id != 0),
        };
        let me = crate::buffs::Character {
            known: &self.world.stats.spells,
            trained: &trained,
            stance: self.combat_stance(),
            guid: self.world.player_guid.unwrap_or(0),
            wears_armour,
            usable: &usable,
            weapon_skill,
        };
        crate::buffs::wanted(&table, &me)
    }

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

    /// Every stack in the packs, as the compactor sees them.
    ///
    /// What the character is holding is here too: a quiver of arrows
    /// can be topped up from the pack, and is worth topping up, but is
    /// never the stack poured away.
    pub fn pack_stacks(&self) -> Vec<crate::pack::Stack> {
        let me = self.world.player_guid;
        let describe = |o: &ac_world::WorldObject, wielded: bool| crate::pack::Stack {
            guid: o.guid,
            wcid: o.weenie_class_id,
            name: o.name.clone(),
            count: o.stack_size.max(1),
            max: o.max_stack_size,
            wielded,
            burden: o.burden,
        };
        self.world
            .inventory()
            .map(|o| describe(o, false))
            .chain(
                self.world
                    .objects
                    .values()
                    .filter(|o| me.is_some() && o.wielder == me)
                    .map(|o| describe(o, true)),
            )
            .collect()
    }

    /// Pour loose stacks together, a pour at a time.
    ///
    /// Slots are the scarce thing, not weight, and nothing warns a
    /// player that a purchase landed beside a pile of the same, or that
    /// the peas taken off four corpses are sitting in four stacks. This
    /// is housekeeping rather than a goal because it must not have to
    /// win a tick to happen: a character that fights, loots and walks
    /// all afternoon never has a quiet one, and tidying that waits for
    /// one never runs. It costs nothing to let it go first -- the server
    /// makes a pack-to-pack merge on the spot, with no walk, no animation
    /// and no busy check, so a pour cannot get in the way of a take, a
    /// cast or a walk.
    pub(crate) fn autoplay_tidy(&mut self, now: Instant) {
        // Read the last pour's answer before any gate. A counter opened
        // or a fight started in the meantime is no reason to go on
        // believing a pour is still in the air.
        match self.settle_pour(now) {
            Some(crate::did::Did::Waiting(_)) => return,
            Some(did) => self.aside("tidy the pack", &did, now),
            None => {}
        }
        let gate = TidyGate {
            // With no profile the pack is still tidied: it is not looting.
            tidy_pack_off: !self.loot_profile().is_none_or(|p| p.looting.tidy_pack),
            counter_open: self.world.open_vendor.is_some(),
            quartermaster: matches!(
                self.autoplay.growth.mode.stage(),
                Some(crate::logistics::Stage::HandOver | crate::logistics::Stage::HandOut)
            ),
            crafting: self.autoplay.crafting.is_some(),
            gave_lately: self
                .autoplay
                .last_give
                .is_some_and(|t| now.duration_since(t) < GIVE_EVERY),
            take_in_air: self.loot_inflight.is_some() || !self.loot_queue.is_empty(),
        };
        if let Some(why) = why_not_tidy(gate) {
            tracing::trace!("not tidying the pack: {why}");
            return;
        }
        if self.autoplay.tidy_laden_until.is_some_and(|t| now < t) {
            return;
        }
        // Deciding what to pour copies the name of every stack carried,
        // and no answer can change faster than the server gives one.
        if self
            .autoplay
            .tidy_looked
            .is_some_and(|t| now.duration_since(t) < MERGE_EVERY)
        {
            return;
        }
        self.autoplay.tidy_looked = Some(now);
        match self.pour_next(now) {
            // A note, not a `say`: this is not what the character is
            // doing. Said as a status it would flicker against the
            // fighting or looting that is.
            Ok(m) => self.autoplay.note(
                format!("putting {} {} with the rest", m.amount, m.name),
                now,
            ),
            Err(crate::growth::Unpoured::Laden) => {
                self.autoplay.tidy_laden_until = Some(now + TIDY_LADEN_WAIT);
                let did = crate::did::Did::blocked("too laden to put stacks together");
                self.aside("tidy the pack", &did, now);
            }
            Err(crate::growth::Unpoured::Turned(did)) => self.aside("tidy the pack", &did, now),
            Err(crate::growth::Unpoured::Tight | crate::growth::Unpoured::Wait(_)) => {}
        }
    }

    /// Heal, and break off a losing fight. True when it acted.
    pub(crate) fn autoplay_survive(&mut self, now: Instant) -> bool {
        let cfg = self.autoplay.config.survive.clone();
        let health = self.health_fraction();
        if health >= cfg.heal_below || health <= 0.0 {
            return false;
        }
        // Paced by the server, not by a clock: the next heal goes out
        // as soon as the last one is answered for. A fixed interval is
        // either too slow to save a character or quick enough to have
        // the spell dropped for arriving over the last.
        if self.autoplay.cast_in_flight(now) {
            // Waiting for the next heal is not a reason to stand
            // still. Everything below this in the list -- looting,
            // walking, tidying -- carries on; it is only the fighting
            // that must not go first, and the fight rule sees to that
            // itself (`too_hurt_to_fight`). Holding the tick here
            // instead left the character idle between heals.
            return false;
        }
        // A kit is quicker and cheaper than a spell -- but only to
        // someone who has trained Healing. Untrained it restores next to
        // nothing, so a caster is far better off with Heal Self, and
        // reaching for a kit only wastes the moment it takes.
        if cfg.use_kits && self.heals_with_kits() {
            let kit = self
                .world
                .inventory()
                .filter(|o| ac_world::usable::on_self(o.usable) && o.name.contains("Healing Kit"))
                .map(|o| o.guid)
                .next();
            if let Some(kit) = kit {
                let me = self.world.player_guid.unwrap_or(0);
                self.remember_journey();
                self.use_on(kit, me);
                // A kit is answered like a cast, and waited on like one.
                self.autoplay.cast_sent = Some(now);
                self.autoplay.last_heal = Some(now);
                self.autoplay
                    .say(Doing::Healing, format!("healing at {:.0}%", health * 100.0));
                return true;
            }
        }
        // Which heal is the right one is not a setting: it is this
        // moment's answer, and it changes with every point of damage.
        // A scratch takes the cheapest spell that covers it; a wound
        // that will kill takes the biggest in the book (see
        // [`choose_heal`]).
        let missing = self
            .world
            .stats
            .vital_max_current(0)
            .saturating_sub(self.world.stats.vitals[0].current);
        if let Some(spell) = choose_heal(&self.self_heals(), missing, health, &cfg) {
            self.cast_paced(spell, now);
            self.autoplay.last_heal = Some(now);
            self.autoplay
                .say(Doing::Healing, format!("healing at {:.0}%", health * 100.0));
            return true;
        }
        // Hurt and unable to heal is worth saying out loud.
        if let Some(why) = self.why_no_heal() {
            self.autoplay
                .note(format!("cannot heal at {:.0}%: {why}", health * 100.0), now);
        }
        false
    }

    /// Whether a healing kit is worth using: Healing has to be trained
    /// or specialised for one to restore anything much. A character
    /// without it heals with a spell instead, and does not want kits
    /// bought for it either (see `grow_needs`).
    pub fn heals_with_kits(&self) -> bool {
        use ac_world::stats::{sac, skill};
        self.world
            .stats
            .skill(skill::HEALING)
            .is_some_and(|s| s.advancement >= sac::TRAINED)
    }

    /// How many of the same weenie are already carried, for the rules
    /// that stop at a number.
    fn already_carried(&self, wcid: u32) -> u32 {
        self.world
            .inventory()
            .filter(|o| o.weenie_class_id == wcid)
            .map(|o| o.stack_size.max(1))
            .sum()
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

    /// The same, for a thing already in the pack: how many of its kind
    /// are carried besides it, which is what was held when it arrived.
    fn carried_besides(&self, stats: &crate::items::ItemStats) -> u32 {
        let own = self
            .world
            .objects
            .get(&stats.guid)
            .map_or(0, |o| o.stack_size.max(1));
        self.already_carried(stats.wcid).saturating_sub(own)
    }

    /// What the rules say to do with an item, now.
    /// The corpse in front of the character, as the looting rules need
    /// to see it.
    ///
    /// The judging stays here -- it needs the profile, the character's
    /// own skills, what is already carried, and whether this is the
    /// character's own body -- and arrives in `ac-loot` as a verdict
    /// already reached. What that crate decides is the *order*: what to
    /// ask about, what to take, when to stop, and when to shut it.
    /// What is to be done with an item: what it was taken for if that
    /// was written down, else what the profile makes of it now.
    ///
    /// The ledger alone is not the answer. It only knows what was
    /// decided about things the character has held since; something
    /// bought, traded or split off a stack a moment ago carries no
    /// entry yet, and a ledger-only reading would call that "nothing
    /// decided", which reads as "skip".
    pub fn loot_action(&self, stats: &crate::items::ItemStats) -> Option<LootAction> {
        if let Some(a) = self.autoplay.ledger.of(stats) {
            return Some(a);
        }
        match judge_loot(
            stats,
            self.appraisals.get(&stats.guid),
            self.loot_profile().as_deref(),
            &self.wielder(),
            &self.world.stats.name,
            self.already_carried(stats.wcid),
        ) {
            crate::profile::Verdict::Decided(a, _) => Some(a),
            crate::profile::Verdict::NeedsId(_) | crate::profile::Verdict::None => None,
        }
    }

    /// The loot profile this character reads, if the shelf has it. None
    /// means it does not loot.
    pub fn loot_profile(&self) -> Option<std::sync::Arc<crate::profile::Profile>> {
        self.profiles.get(&self.autoplay.config.loot.profile)
    }

    /// Notice the rules changing and re-judge what is already carried.
    ///
    /// A decision stands for as long as the rules that made it do. It
    /// has to: a character that judged its pack afresh every time it
    /// looked would sell the ring it kept the moment the pack filled,
    /// which is the whole reason the ledger exists. But the *rules*
    /// changing is the one thing that should reach back. The player has
    /// just said what their things are worth, and a decision written
    /// down under the old rules is an answer to a question nobody is
    /// asking any more.
    ///
    /// Runs whatever else the character is doing, autoplay off
    /// included: the ledger is what the Items window reads, and it
    /// should not be telling somebody their mule is holding things for
    /// a rule they deleted.
    pub(crate) fn tick_retag(&mut self, now: Instant) {
        let profile = self.loot_profile();
        // The cheap half, run every frame: has the shelf handed out a
        // different profile? It replaces the whole profile on every edit,
        // name lists and all, so identity answers it without reading a
        // rule.
        let untouched = match &self.autoplay.judged_under {
            Some(was) => match (was, &profile) {
                (None, None) => true,
                (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
                _ => false,
            },
            None => false,
        };
        if !untouched {
            // A character that has not entered the world yet has no
            // pack to judge and no name to file a ledger under. Leave
            // the rules unrecorded so this runs again once it has.
            if self.world.stats.name.trim().is_empty() {
                return;
            }
            let rules = self.rules_fingerprint(profile.as_deref());
            self.autoplay.judged_under = Some(profile);
            // Something was handed out, but were the rules themselves
            // any different? Typing in a profile's note replaces it
            // without changing a single answer, and neither does
            // opening a client that has been shut since the last edit.
            //
            // This is also what keeps a decision from being re-made on
            // every login. Judging the pack afresh each time is the one
            // thing the ledger exists to prevent: a rule that keeps up
            // to a number reads the pack it is in, and a pack that has
            // since filled turns yesterday's keepers into today's
            // vendor trash.
            if self.autoplay.ledger.rules() == Some(rules) {
                return;
            }
            self.autoplay.ledger.judged_under(rules);
            self.autoplay.retagging = Some(now);
            self.autoplay.retag_due = Some(now);
        }
        let Some(since) = self.autoplay.retagging else {
            return;
        };
        if self.autoplay.retag_due.is_some_and(|due| now < due) {
            return;
        }
        self.autoplay.retag_due = Some(now + RETAG_EVERY);
        if self.retag_pack(now.duration_since(since) >= TAG_TIMEOUT) {
            self.autoplay.retagging = None;
            self.autoplay.retag_due = None;
        }
    }

    /// Everything that decides an item, as one number (see
    /// `Profile::fingerprint`). A character reading no profile still has
    /// a fingerprint, so being given one counts as a change.
    fn rules_fingerprint(&self, profile: Option<&crate::profile::Profile>) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        profile.map(|p| p.fingerprint()).hash(&mut h);
        h.finish()
    }

    /// Judge everything carried against the rules as they stand now and
    /// write the answers down. True when nothing is left waiting on an
    /// appraisal, so the caller can stop.
    ///
    /// `settle` says to take the answer as it is rather than go on
    /// waiting for the server.
    ///
    /// Items are gone through oldest first and each is told how many of
    /// its kind come at or before it, rather than how many are carried
    /// altogether. That is what a rule with a `keep_up_to` on it was
    /// answered with when the things arrived one at a time, and telling
    /// all three rings that three rings are carried would put every one
    /// of them over a cap of two -- turning a set of keepers into a set
    /// of vendor trash in one pass.
    fn retag_pack(&mut self, settle: bool) -> bool {
        let profile = self.loot_profile();
        let wielder = self.wielder();
        let who = self.world.stats.name.clone();
        let mut carried: Vec<(u32, u32, u32)> = self
            .world
            .inventory()
            .chain(self.world.wielded())
            .map(|o| (o.guid, o.weenie_class_id, o.stack_size.max(1)))
            .collect();
        let mut waiting = Vec::new();
        for (guid, held) in in_arrival_order(&mut carried) {
            let Some(stats) = self.stats_of(guid) else {
                continue;
            };
            match judge_loot(
                &stats,
                self.appraisals.get(&guid),
                profile.as_deref(),
                &wielder,
                &who,
                held,
            ) {
                crate::profile::Verdict::Decided(LootAction::Skip, _)
                | crate::profile::Verdict::None => {
                    // Nothing claims it any more. That is not a decision
                    // to be rid of it -- it is no decision at all, and
                    // an item with no entry is never sold.
                    self.autoplay.ledger.forget(guid);
                }
                crate::profile::Verdict::Decided(action, _) => {
                    if self.autoplay.ledger.of(&stats) != Some(action) {
                        self.autoplay.tag(&stats, action);
                    }
                }
                // A rule wants it but cannot say so until the server has
                // identified it. Yesterday's answer stands in the
                // meantime rather than being thrown away over a question
                // that has not been answered.
                crate::profile::Verdict::NeedsId(_) if !settle && !stats.appraised => {
                    waiting.push(guid);
                }
                crate::profile::Verdict::NeedsId(_) => {}
            }
        }
        if waiting.is_empty() {
            return true;
        }
        self.appraise_many(waiting);
        false
    }

    /// Judge a carried item by this character's profile and write down
    /// what it is for, the way the arrival pass does for something that
    /// turns up in the pack ([`arrival_tag`]).
    ///
    /// `None` when nothing claimed it, and then nothing is written:
    /// silence is not a decision to leave it, and an item with no entry
    /// is judged afresh next time.
    pub fn tag_loot(&mut self, guid: u32) -> Option<LootAction> {
        let stats = self.stats_of(guid)?;
        let action = arrival_tag(
            &stats,
            self.appraisals.get(&guid),
            self.loot_profile().as_deref(),
            &self.wielder(),
            &self.world.stats.name.clone(),
            self.already_carried(stats.wcid),
        )?;
        self.autoplay.tag(&stats, action);
        Some(action)
    }

    /// The corpse `guid` holding `items`, asked to open at `asked`, and
    /// the character standing over it, as the loot rules see them.
    fn corpse_now(
        &mut self,
        guid: u32,
        items: &[u32],
        profile: &crate::profile::Profile,
        asked: Instant,
        now: Instant,
    ) -> ac_loot::Open {
        use crate::profile::Verdict as Judged;
        let away = match (
            self.player.as_ref().map(|p| p.world_position()),
            self.world.objects.get(&guid).and_then(|o| o.world_pos()),
        ) {
            (Some(me), Some(at)) => at.distance(me),
            _ => 0.0,
        };
        let name = self
            .world
            .objects
            .get(&guid)
            .map(|o| o.name.clone())
            .unwrap_or_else(|| "the corpse".to_string());
        // Our own body: everything on it is ours, and the wand and the
        // components are what the character needs to fight again.
        let mine = {
            let me = self.world.stats.name.to_lowercase();
            !me.is_empty() && name.to_lowercase() == format!("corpse of {me}")
        };
        let wielder = self.wielder();
        let who = self.world.stats.name.clone();
        let my_guid = self.world.player_guid.unwrap_or(0);
        // The fellows a thing here may be meant for instead of this
        // character (see `called_to`): nobody off its own body, or under
        // rules that mean nothing for anyone in particular.
        let fellows = if mine || !profile.sends_to_the_best() {
            Vec::new()
        } else {
            self.callable_to(guid)
        };
        let salvager = if fellows.is_empty() {
            None
        } else {
            self.best_salvager().map(|(_, g)| g)
        };
        // What has already been spoken for off this body counts towards
        // a cap (see `ac_loot::Claimed`).
        let mut claimed = ac_loot::corpse::Claimed::default();
        // Listed but not described yet: the server sends the items a
        // pass after the list. Dropped silently, they made a full body
        // read as empty (see `ac_loot::Open::arriving`).
        let mut arriving = Vec::new();
        // Where a take can go, as the rules must see it: a thing that
        // pours onto a carried pile needs no slot. Judged here from the
        // same room the take is sent from (`Client::how_to_take`), so
        // the two cannot disagree. The stacks carried are copied by
        // name, and only when something on the body could pour.
        let packs = self.packs();
        let carried = if items
            .iter()
            .filter_map(|g| self.world.objects.get(g))
            .any(|o| Self::loose(o).pours)
        {
            self.pack_stacks()
        } else {
            Vec::new()
        };
        let lying: Vec<ac_loot::Lying> = items
            .iter()
            .filter_map(|g| {
                let Some(stats) = self.stats_of(*g) else {
                    arriving.push(*g);
                    return None;
                };
                let needs_no_slot = self.world.objects.get(g).is_some_and(|o| {
                    let loose = Self::loose(o);
                    loose.pours
                        && matches!(
                            crate::room::how_to_take(&loose, &carried, &packs),
                            Some(crate::room::Take::Merge { .. })
                        )
                });
                // A kind the server has lately said cannot be had yet
                // is left alone for a while (see `loot_refused`).
                if self.refused_lately(stats.wcid, now) {
                    return None;
                }
                let held = self.already_carried(stats.wcid) + claimed.of(stats.wcid);
                // Judged once, here, whoever the body belongs to.
                let judged = judge_loot(
                    &stats,
                    self.appraisals.get(g),
                    Some(profile),
                    &wielder,
                    &who,
                    held,
                );
                let verdict = if mine {
                    // Our own body: everything on it comes back, but
                    // what each thing is for is still what the rules
                    // said (see `ac_loot::corpse::recovered`).
                    let took = match judged {
                        Judged::Decided(action, _) => Some(action),
                        Judged::NeedsId(_) | Judged::None => None,
                    };
                    ac_loot::Verdict::Take(ac_loot::corpse::recovered(took))
                } else {
                    match judged {
                        Judged::Decided(action, _) if action.takes() => {
                            // Meant for a fellow better at what the rule
                            // asks: left for that one, and the shut says
                            // so (see `Client::shut_for`).
                            let meant_for = if fellows.is_empty() {
                                None
                            } else {
                                let me = Taker {
                                    guid: my_guid,
                                    name: &who,
                                    sheet: &wielder,
                                    held,
                                };
                                let takers = takers_with(me, &fellows);
                                called_to(
                                    &stats,
                                    self.appraisals.get(g),
                                    profile,
                                    &takers,
                                    salvager,
                                )
                            };
                            if meant_for.is_some_and(|to| to != my_guid) {
                                ac_loot::Verdict::Leave
                            } else {
                                ac_loot::Verdict::Take(action)
                            }
                        }
                        Judged::Decided(_, _) => ac_loot::Verdict::Leave,
                        // Only worth asking about when asking is allowed
                        // and might answer.
                        Judged::NeedsId(_) if profile.looting.appraise => ac_loot::Verdict::MustAsk,
                        Judged::NeedsId(_) | Judged::None => ac_loot::Verdict::Leave,
                    }
                };
                if matches!(verdict, ac_loot::Verdict::Take(_)) {
                    claimed.take(stats.wcid, stats.stack.max(1));
                }
                Some(ac_loot::Lying {
                    guid: *g,
                    name: stats.name.clone(),
                    burden: stats.burden,
                    verdict,
                    needs_no_slot,
                })
            })
            .collect();
        ac_loot::Open {
            guid,
            name,
            away,
            open: true,
            items: lying,
            slots_free: self.free_space(),
            room_anywhere: self.room_anywhere(),
            keep_free: self.autoplay.config.team.restock.keep_slots,
            carry_room: self.carry_room(&self.autoplay.config.growth),
            may_ask: profile.looting.appraise,
            asking: self.appraise_inflight.iter().map(|(g, _)| *g).collect(),
            // What the server has turned down since the corpse was asked
            // to open (see `ac_loot::Open::refused`). One turned down
            // before is not held against this opening.
            refused: items
                .iter()
                .filter(|g| {
                    self.move_refused
                        .get(g)
                        .is_some_and(|(_, when)| *when >= asked)
                })
                .copied()
                .collect(),
            arriving,
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

    /// The server has refused something with `code`. When the reason is
    /// one that will not change today -- a thing that can only be had
    /// so many times a day -- the *kind* of thing is remembered, not
    /// the one on this corpse: the next corpse's copy would be refused
    /// for the same reason, and asking again is a round trip spent to
    /// be told no twice.
    ///
    /// `guid` is the item turned down, as the refusal named it or as the
    /// take in flight (see [`refused_item`]). This used to look up the
    /// front of a take queue that nothing filled any more, so no kind was
    /// ever remembered.
    pub(crate) fn loot_refused(&mut self, guid: u32, code: u32) {
        // Both are waits -- the first will certainly lift, and a solve
        // cap can be raised -- so neither is a `Refused`, which would
        // mean never.
        if !only_so_often(code) {
            return;
        }
        let Some(o) = self.world.objects.get(&guid) else {
            return;
        };
        let (wcid, name) = (o.weenie_class_id, o.name.clone());
        if wcid != 0 {
            let now = Instant::now();
            // The wait doubles with each refusal, so a cooldown of an
            // hour is picked up within the hour and a daily one costs a
            // handful of wasted asks a day -- nothing against missing
            // the thing for a day.
            self.autoplay.refused_kinds.note(
                wcid,
                &crate::did::Did::Blocked(crate::did::Because::server(code)),
                now,
            );
            tracing::info!("autoplay: {name} cannot be had yet; leaving its kind for a while");
        }
    }

    /// Whether this kind of thing is still inside the wait a refusal
    /// put on it.
    fn refused_lately(&self, wcid: u32, now: Instant) -> bool {
        self.autoplay.refused_kinds.held(&wcid, now)
    }

    /// Let go of a walk toward a corpse, wherever that corpse has been
    /// let go of. Nothing else is steering here, so leaving the walk
    /// running would carry the character off to a body it has already
    /// finished with.
    fn stop_walking_to_loot(&mut self) {
        if self.autoplay.walking_to.take().is_some() && self.follow.take().is_some() {
            self.steering.reset();
        }
    }

    /// Press on with a walk to the corpse `guid`, called `name`, lying
    /// at `spot` and `away` metres off. True while the walk is getting
    /// somewhere; false once it cannot arrive -- what becomes of the
    /// body then is for the caller to say, because a body being walked
    /// to and a body already in hand end differently.
    fn walk_to_corpse(
        &mut self,
        guid: u32,
        name: &str,
        spot: glam::Vec3,
        away: f32,
        now: Instant,
    ) -> bool {
        // The walk ends any journey under way, and it is a detour the
        // character comes back from: a town run picks its walk to the
        // counter up again once the body is dealt with (see
        // `Client::journey_broken_off`). +Verity's run did not, and
        // gave up 224 m short of Shopkeeper Renald the Elder.
        self.interrupt_travel("walking to a corpse");
        // Well inside the radius rather than on its edge: the last
        // metre of a walk wanders, and stopping on the line means
        // stepping back off it again.
        let did = self.head_for(spot, CORPSE_REACH / 2.0, "the corpse");
        // Said once for the walk, not once a frame: the distance
        // changes every tick and the log is not a tape measure.
        if self.autoplay.walking_to.map(|w| w.guid) != Some(guid) {
            self.autoplay.walking_to = Some(CorpseWalk::new(guid, away, now));
            self.autoplay.say(
                Doing::Looting,
                format!("walking to {name} ({} m)", away.round()),
            );
        }
        // A route being followed is a way there, whatever the steering
        // said before it had one.
        let no_way = self.steering.no_way() && self.steering.route.is_none();
        did.fine()
            && self
                .autoplay
                .walking_to
                .as_mut()
                .is_some_and(|w| w.goes_on(away, no_way, now))
    }

    /// A weapon waiting for empty hands is taken up as soon as they
    /// are: a bow cannot be drawn with a shield up, and a two-handed
    /// weapon needs both. Runs every tick and never claims one.
    ///
    /// This is also where a wield that did land forgets whatever wait a
    /// refusal earned it: the hands have changed, so whatever the server
    /// was objecting to has gone.
    pub(crate) fn autoplay_pending_wield(&mut self, now: Instant) {
        if let Some((g, _)) = self.autoplay.wield_asked {
            let me = self.world.player_guid;
            if self.world.objects.get(&g).is_some_and(|o| o.wielder == me) {
                self.autoplay.wield_refused.forget(&g);
                self.autoplay.wield_asked = None;
            }
        }
        if let Some(g) = self.autoplay.pending_wield {
            // A shield in the off hand counts as a full hand for a
            // weapon that cannot be held with one.
            let offhand_matters = self
                .stats_of(g)
                .is_some_and(|i| crate::weapons::needs_free_offhand(&i));
            let hands_full = self.world.wielded().any(|o| {
                (o.item_type
                    & (ac_world::item_type::MELEE_WEAPON
                        | ac_world::item_type::MISSILE_WEAPON
                        | ac_world::item_type::CASTER)
                    != 0
                    && o.valid_locations & ac_world::equip::MISSILE_AMMO == 0)
                    || (offhand_matters && o.valid_locations & ac_world::equip::SHIELD != 0)
            });
            if !hands_full {
                // Inside the wait a refusal earned it, or with the
                // server busy swinging, the errand keeps rather than
                // being dropped: giving up here would leave the weapon
                // in the pack and the character bare-handed with
                // nothing left to ask again.
                if self.world.is_carried(g) && self.wield_must_wait(g, now) {
                    return;
                }
                self.autoplay.pending_wield = None;
                if self.world.is_carried(g) {
                    self.wield_guid(g);
                }
            } else if !self.world.is_carried(g) && !self.world.objects.contains_key(&g) {
                self.autoplay.pending_wield = None;
            }
        }
    }

    /// Leave an item the server has just refused to wield alone for a
    /// while, and say so once rather than every pass.
    ///
    /// A refused wield carries no error code worth reading -- ACE sends
    /// InventoryServerSaveFailed with WeenieError.None -- so there is
    /// nothing to act on and nothing to do but wait. The wait doubles
    /// each time, so an item the server will never wield in this state
    /// costs a handful of messages rather than one every buff pass:
    /// +Verity asked for her Training Wand a hundred and fifty times in
    /// a minute, because a shield in her off hand made the wield
    /// impossible and the refusal said nothing about it.
    pub(crate) fn hold_off_wield(&mut self, item: u32, now: Instant) {
        self.autoplay.wield_asked = None;
        self.autoplay.wield_refused.hold(item, WIELD_AGAIN, now);
        let name = self
            .world
            .objects
            .get(&item)
            .map(|o| o.name.clone())
            .unwrap_or_else(|| format!("{item:#010x}"));
        let waited = self
            .autoplay
            .wield_refused
            .waited(&item)
            .unwrap_or(WIELD_AGAIN);
        tracing::info!(
            "the server will not wield {name}; leaving it for {} s",
            waited.as_secs().max(1)
        );
    }

    /// Open the corpse of something we killed and take what is worth
    /// taking. True while looting.
    pub(crate) fn autoplay_loot(&mut self, now: Instant) -> bool {
        // No profile, no looting: it is what says what to take.
        let Some(profile) = self.loot_profile() else {
            return false;
        };
        // Already at one: wait for its contents, then empty it.
        if let Some((guid, since, allow, tries)) = self.autoplay.corpse {
            // The clock is on the opening, not on the emptying. Once
            // the corpse is open the character is taking from it an
            // item at a time, and asking the server to open it again
            // in the middle of that pulls the container out from under
            // the take in flight -- which is what "Source item not
            // found!" is.
            let opened = self
                .world
                .open_container
                .as_ref()
                .is_some_and(|(g, _)| *g == guid);
            // How long to keep at an open corpse is the loot rules' to say
            // (`ac_loot::run::KEEP_AT_IT`), counting only the time spent
            // at it, and they set a body that will not empty aside. This
            // used to give up first, forty-five seconds after the open was
            // asked for -- a Drudge pack fought off in between counted
            // too -- and wrote the body off for good with its loot still
            // on it, so the rules' own close could never be reached.
            if !opened && now.duration_since(since) > allow && self.autoplay.cast_in_flight(now) {
                // A spell went out meanwhile, and the use was most likely
                // turned away as too busy: that is not the corpse refusing.
                // Wait for the cast, and do not count it as a try.
                self.autoplay.corpse = Some((guid, now, allow, tries));
                return true;
            }
            if !opened && now.duration_since(since) > allow {
                // How this ask came back, from where the character stands
                // now: nothing walks it while nothing comes back.
                let me = self.player.as_ref().map(|p| p.world_position());
                let away = self
                    .world
                    .objects
                    .get(&guid)
                    .and_then(|o| o.world_pos())
                    .zip(me)
                    .map_or(f32::INFINITY, |(at, me)| at.distance(me));
                let quiet = answered_with_nothing(away, since, self.use_done, self.told);
                let nothing_there = self.autoplay.nothing_came_back(quiet);
                self.autoplay.let_go_of_corpse();
                self.stop_walking_to_loot();
                // Opening a corpse asks the server to walk us to it,
                // and indoors that walk goes round corners. Giving up
                // once and never asking again left loot on the floor,
                // so ask again before writing it off.
                if tries < LOOT_TRIES {
                    tracing::info!(
                        "autoplay: corpse {guid:#010x} did not open; asking again ({tries})"
                    );
                    self.interact(guid);
                    self.autoplay.corpse = Some((guid, now, allow, tries + 1));
                    return true;
                }
                // Every ask, from within reach, came back with nothing: the
                // server has no such body. It let it go while the
                // character was out of sight and never said so (see
                // [`answered_with_nothing`]). Setting it aside only brought
                // the character back to ask again, and held the next fight
                // for it meanwhile, so it is forgotten the way its delete
                // would have had it. One the server still has but cannot
                // walk the character to answers the same way, and is no
                // more to be had; should the server describe it again, it
                // is back.
                if nothing_there {
                    tracing::info!(
                        "autoplay: corpse {guid:#010x} is not there any more; forgetting it"
                    );
                    self.forget_kill_spot(guid);
                    self.world.forget(guid);
                    self.autoplay.corpse_seen.retain(|(g, _)| *g != guid);
                    return false;
                }
                // Not "done with": set aside. A corpse belongs to
                // whoever killed it until it has rotted a while, so one
                // that will not open now may well open later, and
                // writing it off for good leaves a boss's loot on the
                // floor. It is tried again while it is still there.
                tracing::info!(
                    "autoplay: corpse {guid:#010x} will not open yet; trying again later"
                );
                self.autoplay.shelved.note(
                    guid,
                    &crate::did::Did::blocked("it will not open yet"),
                    now,
                );
                return false;
            }
            let open = self.world.open_container.clone();
            let Some((open_guid, items)) = open else {
                self.autoplay.say(Doing::Looting, "opening a corpse");
                return true;
            };
            if open_guid != guid {
                return true;
            }
            // What to ask about, what to take, in what order and when
            // to stop is decided in `ac-loot`, which knows nothing of
            // sockets or packs: it is handed the body and the character
            // standing over it and answers with one thing to do. The
            // judging stays here, where the profile and the character's
            // own skills are (see `corpse_now`).
            let at = self.corpse_now(guid, &items, &profile, since, now);
            let next = self.autoplay.loot_run.step(&at, now);
            // Any walk back to this body is over the moment the rules
            // stop asking for one (see the branch below).
            if !matches!(
                next.act,
                Some(ac_loot::Act::Approach) | Some(ac_loot::Act::Open)
            ) {
                self.stop_walking_to_loot();
            }
            match next.act {
                Some(ac_loot::Act::Approach) | Some(ac_loot::Act::Open) => {
                    // Out of reach of a body it is holding: the
                    // character drifted while it worked, or a fight came
                    // to it and moved it.
                    //
                    // The body is still open on the server, whatever the
                    // distance. ACE shuts a container when its viewer
                    // stops viewing it, when that viewer uses another,
                    // or on the container's own reset -- and a corpse
                    // has no reset interval. Shutting it here therefore
                    // gave a body the character was already holding back
                    // to everyone else standing over it, and the walk
                    // back had to win it again: fifty times in one
                    // nine-character run.
                    //
                    // So walk back still holding it, and put it down
                    // only once the character has really gone
                    // ([`HOLD_ON_WITHIN`]) or the walk cannot arrive.
                    let spot = self.world.objects.get(&guid).and_then(|o| o.world_pos());
                    let setting_off = self.autoplay.walking_to.map(|w| w.guid) != Some(guid);
                    let going_back = still_holding_at(at.away)
                        && spot.is_some_and(|spot| {
                            self.walk_to_corpse(guid, &at.name, spot, at.away, now)
                        });
                    if going_back {
                        if setting_off {
                            tracing::info!(
                                "autoplay: corpse {guid:#010x} is {} m off; walking back to it \
                                 without shutting it",
                                at.away.round()
                            );
                        }
                        return true;
                    }
                    tracing::info!(
                        "autoplay: corpse {guid:#010x} is out of reach for good; letting it go"
                    );
                    self.close_container();
                    self.autoplay.let_go_of_corpse();
                    self.stop_walking_to_loot();
                    self.autoplay.say(Doing::Looting, next.saying);
                    return true;
                }
                Some(ac_loot::Act::Ask(ids)) => {
                    let n = ids.len();
                    self.appraise_many(ids);
                    self.autoplay.appraising = true;
                    self.autoplay.say(
                        Doing::Looting,
                        format!("looking over {n} of {} item(s)", items.len()),
                    );
                    return true;
                }
                Some(ac_loot::Act::Take(g)) => {
                    // What it is being taken for was settled when the
                    // lid came up; it is not asked again here, because
                    // asking again is how the two answers came to
                    // differ.
                    let took = at.items.iter().find(|i| i.guid == g).and_then(|i| i.took());
                    if let (Some(action), Some(stats)) = (took, self.stats_of(g)) {
                        self.autoplay.tag(&stats, action);
                    }
                    self.take(g);
                    self.autoplay.say(Doing::Looting, next.saying);
                    return true;
                }
                // Nothing to do yet: the rules are waiting on appraisals,
                // or on the things the corpse lists to be described. The
                // corpse stays open and in hand, and the rules' own clock
                // is still the limit. Reading this as done
                // closed a corpse the moment its items went off to be
                // appraised, marked it looted, and sent the character to the
                // next body -- which closed the first on the server and left
                // both, and everything worth taking on them, behind.
                None => {
                    self.autoplay.say(Doing::Looting, next.saying);
                    return true;
                }
                Some(ac_loot::Act::Close) => {
                    let done = matches!(next.did, crate::did::Did::Done);
                    // What it is for the others is judged on what is still
                    // on it, so before the lid goes down, and only for a
                    // body finished with (see `Autoplay::corpse_shut`).
                    let shut = if done {
                        self.shut_for(guid, &items, &profile)
                    } else {
                        ShutFor::default()
                    };
                    self.close_container();
                    // A body set aside is still this character's to come
                    // back to, however far off it fell.
                    if done {
                        self.forget_kill_spot(guid);
                        tracing::info!(
                            "{}",
                            shut_line(
                                &self.world.stats.name,
                                &at.name,
                                guid,
                                self.autoplay.loot_run.taken,
                                &self.names_of(&shut.done_for),
                                &self.names_of(&shut.left_for),
                                &self.names_of(&shut.stand_by),
                            )
                        );
                    } else {
                        tracing::info!(
                            "autoplay: corpse {guid:#010x} set aside; trying again later"
                        );
                    }
                    let left_for = self.names_of(&shut.left_for);
                    self.autoplay
                        .corpse_shut(guid, &next.did, next.left_for_weight, shut, now);
                    self.stop_walking_to_loot();
                    // Whom something is left for says why the others go on
                    // standing by a body this one has shut.
                    let saying = if left_for.is_empty() {
                        next.saying
                    } else {
                        format!("{}; left for {}", next.saying, left_for.join(", "))
                    };
                    self.autoplay.say(Doing::Looting, saying);
                    return true;
                }
            }
        }
        // Look for one nearby that we have not emptied.
        //
        // Mid-fight the loot waits: a corpse keeps for five minutes and
        // the thing hitting us does not. The exception is a corpse
        // about to rot, which is worth breaking off for because there
        // is no second chance at it.
        //
        // A cast in progress is different from a fight in progress: a
        // spell half thrown is wasted, and the corpse keeps for the
        // second it takes to finish. Whether the fight itself is worth
        // breaking off is not decided here any more -- the worth of
        // looting says that, and it rises as bodies age and pile up
        // (see `crate::steps`).
        // Only a cast at something still alive: the fight forgets a dead
        // target when it next runs, and while it waits for this very
        // body it does not run.
        let pressed = self
            .autoplay
            .casting_at()
            .and_then(|g| self.world.objects.get(&g))
            .is_some_and(|o| o.health.unwrap_or(1.0) > 0.0);
        let me = self.player.as_ref().map(|p| p.world_position());
        let Some(me) = me else { return false };
        // Kills go stale with the bodies they leave.
        self.autoplay
            .kill_spots
            .retain(|(_, t)| now.duration_since(*t) < CORPSE_LIFE);
        // A corpse set aside is tried again once its wait is up, and its
        // wait is remembered until the corpse is gone (see
        // `Autoplay::forget_corpses_gone`).
        let objects = &self.world.objects;
        self.autoplay
            .forget_corpses_gone(|g| objects.contains_key(&g));
        // A body stood by for a fellow that is not coming is this
        // character's to open again, and one emptied long ago is forgotten.
        self.autoplay.stop_standing_by(now);
        self.autoplay.looted.forget_old(now);
        // When each corpse turned up, so the ones running out can be
        // emptied first. Noted by the housekeeping rather than here (see
        // [`Client::autoplay_watch_the_ground`]): a body this character
        // is standing off from never reaches this step, and a body it
        // never noted is for ever newly fallen.
        let seen_at: std::collections::BTreeMap<u32, Instant> =
            self.autoplay.corpse_seen.iter().copied().collect();
        let room = self.room_for_loot();
        let corpse = self
            .world
            .objects
            .values()
            // Not emptied, not set aside, something there is room for,
            // close by on this floor or where one of this character's
            // kills fell, not one of the others' to open, and not another
            // player's remains. The one rule every part of this asks, so
            // what the looting goes to and what the next fight waits on
            // cannot come apart (see [`Client::corpse_for_us`]).
            .filter(|o| self.corpse_for_us(o, me, now, room))
            .filter_map(|o| Some((o.world_pos()?.distance(me), o)))
            .map(|(d, o)| (d, o.guid, o.name.clone()))
            .map(|(d, guid, name)| {
                let seen = seen_at.get(&guid).copied().unwrap_or(now);
                let left = CORPSE_LIFE.saturating_sub(now.duration_since(seen));
                (left, d, guid, name)
            })
            .min_by(|a, b| {
                // The one closest to rotting first, and among the ones
                // in no danger, the nearest.
                let urgent = |l: Duration| l <= CORPSE_URGENT;
                urgent(b.0)
                    .cmp(&urgent(a.0))
                    .then_with(|| a.1.total_cmp(&b.1))
            });
        let Some((left, away, guid, name)) = corpse else {
            // Nothing left to go to, so any walk that was going to one
            // is over. Left standing, that walk went on being said as a
            // claim on a body this character had just given up on, and
            // it kept the looting's own worth up (see `crate::steps`)
            // for a body it would never open.
            self.stop_walking_to_loot();
            // A body stood off from only because it is one of the others'
            // turn says so: one character opens it while the rest stand
            // by, and without a word that reads as a hang.
            if let Some((corpse, who)) = self.turn_of_another(me, now, room) {
                self.autoplay.note(format!("{corpse} is {who}'s turn"), now);
            }
            return false;
        };
        // Never mid-cast, unless the body will not be there when the
        // spell lands.
        if pressed && left > CORPSE_URGENT {
            return false;
        }
        // Breaking off a fight means breaking it off. The server marks
        // a character swinging or shooting as busy and refuses to open
        // anything for it, so letting the attack run while walking to a
        // corpse buys "You're too busy" and nothing else -- the target
        // goes first, then the combat stance.
        self.attack_target = None;
        self.autoplay.casting_at = None;
        self.autoplay.armed_for = None;
        if self.combat {
            self.toggle_combat();
        }
        // Stand over it first (see [`CORPSE_REACH`]).
        if away > CORPSE_REACH {
            if let Some(at) = self.world.objects.get(&guid).and_then(|o| o.world_pos()) {
                if !self.walk_to_corpse(guid, &name, at, away, now) {
                    // Through a floor or behind a wall, the walk never
                    // ends by itself, and it held the looting and the
                    // next fight until the body rotted. Set it aside and
                    // get on; it is tried again once the wait is up.
                    let walked = self
                        .autoplay
                        .walking_to
                        .map_or(0, |w| now.duration_since(w.started).as_secs());
                    tracing::info!(
                        "autoplay: cannot reach corpse {guid:#010x} after {walked} s; \
                         trying again later"
                    );
                    self.stop_walking_to_loot();
                    self.autoplay.set_aside_out_of_reach(guid, now);
                    self.autoplay
                        .say(Doing::Looting, format!("cannot reach {name}; leaving it"));
                    return false;
                }
                return true;
            }
        }
        // Not while a spell is on its way: the server turns the use away
        // as too busy.
        if self.autoplay.cast_in_flight(now) {
            return true;
        }
        self.stop_walking_to_loot();
        // Opening a corpse is "using something", which ends a journey.
        self.remember_journey();
        self.interact(guid);
        self.autoplay.take_up_corpse(guid, now, loot_wait(away));
        self.autoplay.say(Doing::Looting, format!("looting {name}"));
        true
    }

    /// The stance the rules want this character in, and the weapon that
    /// gives it wielded if one is carried.
    ///
    /// Which way a character fights is not a setting: it follows what
    /// is in its hands, so [`Style::Auto`] simply reads them. The other
    /// three ask for a weapon of that kind to be wielded, and if none is
    /// carried the character fights with what it has and the rules say
    /// so rather than pretending.
    fn fighting_stance_as(&mut self, style: Style) -> Stance {
        let want = match style {
            Style::Auto => return self.combat_stance(),
            Style::Melee => Stance::Melee,
            Style::Missile => Stance::Missile,
            Style::Magic => Stance::Magic,
        };
        // Not with a swing out: changing weapon cancels it, so the
        // stance the hands give now is the honest answer until the
        // attack has been answered (see `change_of_hands_waits`).
        if change_of_hands_waits(self.combat_stance(), want, self.mid_attack()) {
            self.wait_for_the_swing();
        } else if self.combat_stance() != want {
            // Wielding takes a moment; until the server confirms it, the
            // hands still say what they said. Asked once a second, not
            // once a tick: the server answers each ask, and refuses the
            // ones it cannot yet do.
            let now = Instant::now();
            if self
                .autoplay
                .last_rewield
                .is_none_or(|t| now.duration_since(t) >= REWIELD_EVERY)
            {
                self.autoplay.last_rewield = Some(now);
                self.wield_for(want);
            }
        }
        self.combat_stance()
    }

    /// Wield the best weapon carried for `target`, if a better one than
    /// the one in hand is carried. Done once per target: swapping
    /// weapons mid-swing is worse than a slightly wrong weapon.
    ///
    /// Told to fight with whatever suits, this looks across all three
    /// kinds of weapon at once, so a character skilled with a wand and
    /// poor with a sword reaches for the wand. Changing weapon changes
    /// the stance, which the next tick reads back out of its hands.
    fn arm_for(&mut self, target: u32, stance: Stance, cfg: &Fight) {
        if !cfg.pick_weapon {
            return;
        }
        if self.autoplay.armed_for == Some(target) {
            return;
        }
        // Not with a swing or a charge out: putting the weapon in hand
        // away cancels it (see `Client::mid_attack`). The choice keeps,
        // and is made in the gap after the attack is answered -- a
        // fraction of a second with the wrong weapon beats a charge that
        // never lands.
        if self.mid_attack() {
            self.wait_for_the_swing();
            return;
        }
        self.autoplay.armed_for = Some(target);
        let Some(known) = self.creature_known(target) else {
            tracing::debug!("arm: nothing known about {target:#010x}, keeping what is held");
            return;
        };
        let carried: Vec<crate::items::ItemStats> = self.item_stats();
        // A weapon nobody has looked at has no element, no imbue and no
        // requirement to read, so it can neither be judged nor safely
        // reached for. Ask about the ones we are carrying; the answers
        // come back over the next few seconds and the choice improves
        // with them.
        let unknown: Vec<u32> = carried
            .iter()
            .filter(|i| !i.appraised && crate::weapons::stance_of(i).is_some())
            .map(|i| i.guid)
            .collect();
        if !unknown.is_empty() {
            self.appraise_many(unknown);
            // Come back to the choice once the answers are in.
            self.autoplay.armed_for = None;
        }
        let wielder = self.wielder();
        let free_choice = cfg.style == Style::Auto;
        let picked = if free_choice {
            crate::weapons::best_any(&carried, Some(known), &wielder).map(|(_, c)| c)
        } else {
            crate::weapons::best(&carried, stance, Some(known), &wielder)
        };
        // A bow's choice comes with the arrows to shoot from it.
        self.autoplay.wanted_ammo = picked
            .as_ref()
            .filter(|p| {
                carried
                    .iter()
                    .any(|i| i.guid == p.guid && crate::weapons::is_launcher(i))
            })
            .and_then(|_| crate::weapons::best_missile(&carried, Some(known), &wielder))
            .and_then(|(_, ammo)| ammo.map(|a| a.guid));
        let Some(pick) = picked else {
            tracing::debug!("arm: nothing to pick from for {}", known.name);
            return;
        };
        // What is in hand now, whichever kind it is when the choice is
        // free, so the comparison is between the two real options.
        let held = carried.iter().find(|i| {
            i.wielded
                && match crate::weapons::stance_of(i) {
                    Some(s) => free_choice || s == stance,
                    None => false,
                }
        });
        // Swapping costs nothing worth counting, so take the best there
        // is: anything better than what is in hand wins. Equal keeps
        // what is held, so a tie cannot set it swapping back and forth.
        let now_worth = held
            .map(|i| crate::weapons::score(i, Some(known), &wielder))
            .unwrap_or(0.0);
        tracing::debug!(
            "arm: {} vs {} -> best {} {:.3} (held {:.3})",
            known.name,
            held.map(|i| i.name.as_str()).unwrap_or("nothing"),
            pick.name,
            pick.score,
            now_worth
        );
        if held.map(|i| i.guid) == Some(pick.guid) || pick.score <= now_worth {
            return;
        }
        tracing::info!(
            "autoplay: wielding {} against {} ({})",
            pick.name,
            known.name,
            pick.why
        );
        let picked_stats = carried.iter().find(|i| i.guid == pick.guid);
        // A one-handed melee weapon leaves the off hand for a shield,
        // which is put on once the weapon is in hand (see
        // `autoplay_shield`); anything else wants that hand empty.
        let free_offhand = picked_stats.is_some_and(crate::weapons::needs_free_offhand);
        self.autoplay.wanted_shield = if free_offhand {
            None
        } else {
            crate::weapons::best_shield(&carried, &wielder).map(|s| s.guid)
        };
        // The server will not put a second weapon in full hands: the
        // old one goes back in the pack first, the shield too when the
        // new weapon cannot be held with one, and the new one is
        // wielded once the hands are empty.
        let mut sent = self.put_weapons_away();
        if free_offhand {
            let me = self.world.player_guid;
            let shield = self.wielded_shield();
            if let (Some(me), Some(shield)) = (me, shield) {
                sent |= self.put_in_container(shield, me);
            }
        }
        // When the swap started, so the fight waits for the hands to
        // settle rather than swinging into the moment they are empty.
        self.autoplay.last_rewield = Some(Instant::now());
        // A wield that could not go out this tick -- the item is inside
        // a refusal's wait, or the server has us mid-swing -- becomes an
        // errand rather than being forgotten, so the weapon is taken up
        // on the first free tick instead of being left in the pack.
        if sent || !self.wield_guid(pick.guid) {
            self.autoplay.pending_wield = Some(pick.guid);
        }
    }

    /// Whether a wand, orb or staff is carried at all, in hand or in
    /// the pack. Not the same question as whether one can be taken up
    /// right now, which is a wait rather than a want.
    pub(crate) fn carries_a_caster(&self) -> bool {
        self.world.objects.values().any(|o| {
            self.world.is_carried(o.guid) && o.item_type & ac_world::item_type::CASTER != 0
        })
    }

    /// Whether a weapon swap asked for a moment ago has yet to land.
    ///
    /// Between the put and the wield the hands are empty, and a swing
    /// sent into that gap is a punch: +Verity put her Flaming Takuba
    /// away for a wand and attacked a Spikey Armoredillo bare-handed in
    /// the same tick. Bounded by [`SWAP_SETTLES`] so a swap the server
    /// never finishes cannot stop the character fighting.
    pub(crate) fn hands_changing(&self, now: Instant) -> bool {
        self.autoplay.pending_wield.is_some()
            && self
                .autoplay
                .last_rewield
                .is_some_and(|t| now.duration_since(t) < SWAP_SETTLES)
    }

    /// The shield on the off hand, if any.
    pub(crate) fn wielded_shield(&self) -> Option<u32> {
        self.world
            .wielded()
            .find(|o| o.valid_locations & ac_world::equip::SHIELD != 0)
            .map(|o| o.guid)
    }

    /// Put the shield chosen with a one-handed weapon on, once that
    /// weapon is in hand and the off hand is free.
    pub(crate) fn autoplay_shield(&mut self, now: Instant) {
        let Some(shield) = self.autoplay.wanted_shield else {
            return;
        };
        if self.autoplay.pending_wield.is_some() {
            return;
        }
        if !self.world.is_carried(shield) || self.wielded_shield().is_some() {
            self.autoplay.wanted_shield = None;
            return;
        }
        // Not with a swing out. ACE shuffles the stance on every
        // successful equip, a shield included (TryShuffleStance ->
        // HandleActionChangeCombatMode), and a combat-mode change
        // cancels the attack in flight. The shield keeps; it goes on in
        // the gap after the swing is answered.
        if self.mid_attack() {
            self.wait_for_the_swing();
            return;
        }
        // Inside the wait a refusal earned it, or with the server busy
        // with a spell, the errand keeps: asking now sends nothing, and
        // clearing it would leave the shield in the pack with nothing
        // left to ask again.
        if self.wield_must_wait(shield, now) {
            return;
        }
        // Only with a one-handed melee weapon actually in hand: the
        // weapon may still be on its way, or have turned out to be
        // something a shield cannot go with.
        let held: Vec<crate::items::ItemStats> = self
            .world
            .wielded()
            .filter(|o| crate::weapons::stance_of(&crate::items::ItemStats::of(o, None)).is_some())
            .map(|o| {
                self.stats_of(o.guid)
                    .unwrap_or_else(|| crate::items::ItemStats::of(o, None))
            })
            .collect();
        let Some(weapon) = held.first() else {
            return;
        };
        if crate::weapons::needs_free_offhand(weapon) {
            self.autoplay.wanted_shield = None;
            return;
        }
        if self
            .autoplay
            .last_rewield
            .is_some_and(|t| now.duration_since(t) < REWIELD_EVERY)
        {
            return;
        }
        self.autoplay.last_rewield = Some(now);
        self.autoplay.wanted_shield = None;
        tracing::info!("autoplay: putting the shield on with the weapon");
        self.wield_guid(shield);
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

    /// This character's Salvaging as it stands, buffs counted.
    pub fn salvaging(&self) -> u32 {
        self.skill_now(SALVAGING)
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

    /// Who salvages for the team, this character included: the highest
    /// Salvaging among those carrying an Ust (see [`best_salvager`]).
    /// Off the team it is this character, if it has an Ust.
    pub fn best_salvager(&self) -> Option<(String, u32)> {
        let me = Mate {
            name: self.world.stats.name.clone(),
            guid: self.world.player_guid.unwrap_or(0),
            salvaging: self.salvaging(),
            has_ust: self.salvage_tool().is_some(),
            ..Default::default()
        };
        best_salvager(std::iter::once(&me).chain(self.autoplay.team.mates.iter()))
    }

    /// This character's skills that its loot rules ask about, for the
    /// others to judge a body on its behalf (see [`Mate::skills`]).
    pub fn skills_its_rules_ask_about(&self) -> Vec<(u32, u32, u32, u32)> {
        let asked = self
            .loot_profile()
            .map(|p| p.skills_asked())
            .unwrap_or_default();
        if asked.is_empty() {
            return Vec::new();
        }
        skills_asked_of(&self.wielder(), &asked)
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

    /// Whether this character opens bodies at all: it has loot rules to go
    /// by, and a pack its looting takes from (see
    /// `Autoplay::corpse_waiting`). What it says about itself as
    /// [`Mate::opens_bodies`].
    pub fn opens_bodies(&self) -> bool {
        let room = self.room_for_loot();
        self.loot_profile().is_some() && !room.pack_low && !room.past_the_wall
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
    /// [`Autoplay::take_in_shuts`]). The team plugin calls this as it
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

    /// Look at what has turned up in the pack since last time and tag
    /// what the rules would salvage or sell (see [`arrival_tag`]): the
    /// salvage a teammate handed over, mostly. The first pass only
    /// notes what is carried.
    fn autoplay_tag_arrivals(&mut self, now: Instant) {
        let profile = self.loot_profile();
        let wielder = self.wielder();
        let who = self.world.stats.name.clone();
        let carried: Vec<u32> = self
            .world
            .inventory()
            .chain(self.world.wielded())
            .map(|o| o.guid)
            .collect();
        let ap = &mut self.autoplay;
        if !ap.baselined {
            ap.seen = carried.iter().copied().collect();
            ap.baselined = true;
            return;
        }
        ap.seen.retain(|g| carried.contains(g));
        // An entry lives only as long as the thing does: this is what
        // stops a recycled id ever being mistaken for the item it used
        // to name (see `ac_loot::ledger`).
        ap.ledger.forget_gone(&carried);
        ap.refused.retain(|g, _| carried.contains(g));
        ap.pending_tags.retain(|(g, _)| carried.contains(g));
        for g in &carried {
            if ap.seen.insert(*g) && ap.ledger.by_guid(*g).is_none() {
                ap.pending_tags.push((*g, now));
            }
        }
        // Whether an identify is worth asking for: the profile says,
        // since it is the only thing that judges anything now.
        let needs = profile
            .as_ref()
            .is_some_and(|p| p.looting.appraise && p.needs_id());
        let pending = std::mem::take(&mut self.autoplay.pending_tags);
        for (g, since) in pending {
            let Some(stats) = self.stats_of(g) else {
                continue;
            };
            if needs && !stats.appraised && now.duration_since(since) < TAG_TIMEOUT {
                self.appraise_many([g]);
                self.autoplay.pending_tags.push((g, since));
                continue;
            }
            // What was held before it arrived: the count every other
            // path judges against, and the count a cap is a cap on.
            // With the arrival itself counted, the fourth kit under
            // "keep up to four" was the fourth of four, over the cap,
            // and "the rest, to the counter" had it.
            let held = self.carried_besides(&stats);
            if let Some(action) = arrival_tag(
                &stats,
                self.appraisals.get(&g),
                profile.as_deref(),
                &wielder,
                &who,
                held,
            ) {
                tracing::info!(
                    "autoplay: {} arrived, tagged {}",
                    stats.name,
                    action.label()
                );
                self.autoplay.tag(&stats, action);
            }
        }
    }

    /// Carried items tagged for salvage that can go: not worn, not
    /// wanted by a blank tag. `bags` says whether salvage bags count
    /// (they are handed on, never salvaged again). Each comes with its
    /// name, for the log, and its workmanship, which decides the batch
    /// it is salvaged in (see `SalvageGrade`).
    fn salvage_tagged(&self, bags: bool) -> Vec<(u32, String, f32)> {
        let me = self.world.player_guid;
        let mut items: Vec<(u32, String, f32)> = self
            .autoplay
            .ledger
            .for_salvage(crate::holdings::unix_now())
            .into_iter()
            .filter_map(|g| self.world.objects.get(&g))
            .filter(|o| o.wielder != me && self.world.is_carried(o.guid))
            .filter(|o| {
                let bag = o.name.starts_with("Salvaged ");
                if bag {
                    bags
                } else {
                    o.material != 0 && o.workmanship > 0.0
                }
            })
            .filter(|o| {
                self.autoplay
                    .refused
                    .get(&o.guid)
                    .is_none_or(|n| *n < SALVAGE_TRIES)
            })
            .map(|o| (o.guid, o.name.clone(), o.workmanship))
            .collect();
        items.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
        items
    }

    /// Salvage what the rules tagged, or carry it to whoever salvages
    /// for the team. Runs between fights. True while busy with it.
    pub(crate) fn autoplay_salvage(&mut self, now: Instant) -> bool {
        if self.world.player_guid.is_none() {
            return false;
        }
        self.autoplay_tag_arrivals(now);
        let Some(profile) = self.loot_profile() else {
            return false;
        };
        let cfg = profile.looting.clone();
        if self.attack_target.is_some() || self.autoplay.corpse.is_some() {
            return false;
        }
        // A batch on its way: wait for the items to go, and count a
        // refusal against each when they do not.
        let mut answered = false;
        if let Some((items, since)) = self.autoplay.salvaging.clone() {
            let left: Vec<u32> = items
                .iter()
                .copied()
                .filter(|g| self.world.is_carried(*g))
                .collect();
            if left.is_empty() {
                self.autoplay.salvaging = None;
                answered = true;
                self.autoplay.say(
                    Doing::Salvaging,
                    format!("salvaged {} item(s)", items.len()),
                );
            } else if now.duration_since(since) < SALVAGE_TIMEOUT {
                return true;
            } else {
                self.autoplay.salvaging = None;
                for g in left {
                    let n = self.autoplay.refused.entry(g).or_default();
                    *n += 1;
                    if *n >= SALVAGE_TRIES {
                        let name = self.world.objects.get(&g).map(|o| o.name.clone());
                        // A refusal is not a new decision. Rewriting it
                        // to Keep made the thing eligible for nothing
                        // while it went on holding a slot.
                        self.autoplay.tag_failed(g, "could not be salvaged");
                        self.autoplay.note(
                            format!(
                                "could not salvage {}, setting it aside",
                                name.unwrap_or_default()
                            ),
                            now,
                        );
                    }
                }
            }
        }
        if let Some((item, since)) = self.autoplay.handing {
            if !self.world.is_carried(item) {
                self.autoplay.handing = None;
            } else if now.duration_since(since) < SALVAGE_TIMEOUT {
                return true;
            } else {
                self.autoplay.handing = None;
                let n = self.autoplay.refused.entry(item).or_default();
                *n += 1;
                if *n >= SALVAGE_TRIES {
                    let name = self
                        .world
                        .objects
                        .get(&item)
                        .map(|o| o.name.clone())
                        .unwrap_or_default();
                    self.autoplay
                        .tag_failed(item, "the salvager would not take it");
                    self.autoplay
                        .note(format!("{name} was not taken, setting it aside"), now);
                }
            }
        }
        let Some((who, guid)) = self.best_salvager() else {
            if !self.salvage_tagged(false).is_empty() {
                self.autoplay
                    .note("salvage waiting: nobody on the team carries an Ust", now);
            }
            return false;
        };
        // A batch the server has just salvaged is answered, and the next
        // grade goes at once. Sitting out the gap after it returned the
        // tick to the goals below, the fight among them, and the grades
        // still to come waited out a whole fight and its looting.
        let rate_ok = answered
            || self
                .autoplay
                .last_salvage
                .is_none_or(|t| now.duration_since(t) >= SALVAGE_EVERY);
        if Some(guid) == self.world.player_guid {
            if !cfg.salvage {
                return false;
            }
            let items = self.salvage_tagged(false);
            if items.is_empty() || !rate_ok {
                return false;
            }
            // The server salvages in peace mode only.
            if self.combat {
                self.toggle_combat();
                return true;
            }
            // A 9 or a 10 goes only with its own grade, and the rest wait
            // their turn. What teammates handed over is batched the same
            // way: it was tagged when it arrived, like anything looted.
            let refused = |g: &u32| self.autoplay.refused.get(g).copied().unwrap_or(0);
            let Some((grade, guids)) =
                next_salvage_batch(items.iter().map(|(g, _, w)| (*g, *w, refused(g))))
            else {
                return false;
            };
            if !self.salvage(&guids) {
                return false;
            }
            self.autoplay.salvaging = Some((guids.clone(), now));
            self.autoplay.last_salvage = Some(now);
            let apart = match grade {
                SalvageGrade::Common => "",
                SalvageGrade::Nine => " of workmanship 9, on their own",
                SalvageGrade::Ten => " of workmanship 10, on their own",
            };
            self.autoplay.say(
                Doing::Salvaging,
                format!("salvaging {} item(s){apart}", guids.len()),
            );
            return true;
        }
        if !cfg.hand_off {
            return false;
        }
        let items = self.salvage_tagged(true);
        if items.is_empty() {
            return false;
        }
        let Some(mate) = self
            .autoplay
            .team
            .mates
            .iter()
            .find(|m| m.guid == guid)
            .cloned()
        else {
            return false;
        };
        let Some(me) = self.player.as_ref().map(|p| p.world_position()) else {
            return false;
        };
        // Not while it is fighting, and not from across the map.
        if mate.target.is_some() {
            return false;
        }
        let distance = mate.world.distance(me);
        if distance > HAND_OFF_RANGE {
            self.autoplay.note(
                format!(
                    "salvage waiting: {who} is {distance:.0} m off (salvaging {})",
                    mate.salvaging
                ),
                now,
            );
            return false;
        }
        if distance > GIVE_REACH {
            if self
                .follow
                .is_none_or(|f| f.target.distance(mate.world) > 1.0)
            {
                self.interrupt_travel("taking salvage to the salvager");
                self.steering.reset();
            }
            self.head_for(mate.world, GIVE_REACH * 0.8, &who);
            self.autoplay
                .say(Doing::Salvaging, format!("taking salvage to {who}"));
            return true;
        }
        if self.follow.take().is_some() {
            self.steering.reset();
        }
        if !rate_ok {
            return true;
        }
        // One item at a time, so a hand-off never mixes grades: the
        // salvager batches what arrives like the rest of its own.
        let (item, name, _) = items[0].clone();
        if !self.give(guid, item, None) {
            return false;
        }
        self.autoplay.handing = Some((item, now));
        self.autoplay.last_salvage = Some(now);
        self.autoplay.say(
            Doing::Salvaging,
            format!(
                "giving {name} to {who} to salvage ({} left)",
                items.len() - 1
            ),
        );
        true
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

    /// See that the bow has something to shoot: the ammunition chosen
    /// for the target if it is still carried, else whatever fits. True
    /// when there is something to shoot -- in the slot, or on its way
    /// into it.
    ///
    /// Not "did a wield go out this tick": the one caller reads a false
    /// as "no ammunition" and goes off to fletch some (see
    /// [`Client::autoplay_craft_ammo`]). A wield held back because the
    /// server is busy is a tick's wait, not an empty quiver, and reading
    /// it as one dropped an archer with a full quiver into peace stance
    /// to make arrows it was already carrying.
    fn ready_ammo(&mut self) -> bool {
        if self.wielded_ammo().is_some() {
            // The chosen kind, if it is not the one in the slot.
            if let Some(want) = self.autoplay.wanted_ammo {
                if self.wielded_ammo() != Some(want) && self.world.is_carried(want) {
                    self.wield_guid(want);
                }
            }
            return true;
        }
        if let Some(want) = self.autoplay.wanted_ammo {
            if self.world.is_carried(want) {
                // Sent, or waiting on a busy tick: either way there is
                // something to shoot and nothing to make. Only a stack
                // the server keeps refusing to wield is an answer of
                // "not this kind", and then another stack is tried.
                if self.wield_guid(want) || !self.wield_held_off(want) {
                    return true;
                }
            }
        }
        self.wield_ammo()
    }

    /// Make ammunition for the launcher in hand from a bundle of heads
    /// and a bundle of shafts carried, the recipe within Fletching, for
    /// the element the target is weakest to when there is a choice.
    /// True when this tick went on making some.
    ///
    /// The server's side of it (ACE `RecipeManager::UseObjectOnTarget`):
    /// using the heads on the shafts is refused outright in any combat
    /// stance, and by a character not trained in Fletching, so the
    /// character drops to peace first and the bundles are used once the
    /// stance change has had its moment. The server may then ask, as a
    /// yes/no confirmation, whether the chance of success is good
    /// enough; it is answered yes, and the arrows land in the pack a
    /// clap of the hands later, where the bow's arming picks them up.
    fn autoplay_craft_ammo(&mut self, now: Instant) -> bool {
        if !self.autoplay.config.fight.craft_ammo {
            return false;
        }
        // The chance-of-success question, if the character has that
        // option on: the answer is always yes, the bundles being for
        // nothing else.
        const CRAFT: u32 = 5;
        let asked: Vec<u32> = self
            .world
            .confirmations
            .iter()
            .filter(|c| c.kind == CRAFT)
            .map(|c| c.context)
            .collect();
        if !asked.is_empty() && self.autoplay.last_craft.is_some() {
            for context in asked {
                self.confirm(CRAFT, context, true);
            }
            return true;
        }
        // Waiting for peace mode before the bundles are used.
        if let Some((source, target, since)) = self.autoplay.crafting {
            if now.duration_since(since) < STANCE_CHANGE {
                return true;
            }
            self.autoplay.crafting = None;
            self.autoplay.last_craft = Some(now);
            return self.use_on(source, target);
        }
        if self
            .autoplay
            .last_craft
            .is_some_and(|t| now.duration_since(t) < CRAFT_EVERY)
        {
            return false;
        }
        let launcher = self
            .wielded_missile_weapon()
            .and_then(|g| self.stats_of(g))
            .filter(crate::weapons::is_launcher);
        let Some(launcher) = launcher else {
            return false;
        };
        if launcher.ammo_type == 0 {
            return false;
        }
        // Fletching as it stands, and only if trained: an untrained
        // skill has a number too, but the server will not craft with it.
        let fletching = {
            let stats = &self.world.stats;
            let table = self.assets.skill_table().ok();
            stats
                .skill(FLETCHING)
                .filter(|sk| sk.advancement >= ac_world::stats::sac::TRAINED)
                .map(|sk| stats.skill_current(sk, table.as_ref().and_then(|t| t.get(FLETCHING))))
                .unwrap_or(0)
        };
        let carried: Vec<(u32, u32)> = self
            .world
            .objects
            .values()
            .filter(|o| self.world.is_carried(o.guid))
            .map(|o| (o.weenie_class_id, o.guid))
            .collect();
        let weakest = self
            .autoplay
            .casting_at
            .or(self.attack_target)
            .and_then(|g| self.creature_known(g))
            .and_then(|c| c.weakest_to());
        let Some((recipe, source, target)) =
            choose_recipe(launcher.ammo_type, fletching, &carried, weakest)
        else {
            self.autoplay.note(
                format!(
                    "out of {} and nothing to make more from",
                    ac_world::fletching::ammo_type::name(launcher.ammo_type)
                ),
                now,
            );
            return false;
        };
        self.autoplay.say(
            Doing::Looting,
            format!("making {} from {}", recipe.result_name, recipe.source_name),
        );
        if self.combat || self.magic {
            // Peace first; the use goes out once the stance has changed.
            self.leave_combat();
            self.autoplay.crafting = Some((source, target, now));
            return true;
        }
        self.autoplay.last_craft = Some(now);
        self.use_on(source, target)
    }

    /// The room in the packs, pack by pack: the main pack and each side
    /// pack with its own slots and its own count (see `room::Packs`).
    ///
    /// Counted apart because the server keeps them apart. A take names
    /// the pack it goes into and is refused when that one is full,
    /// however empty the sacks beside it; only what the server creates
    /// itself -- a counter's payout, a purchase -- spills from the main
    /// pack into the side packs. Added into one sum, nine characters
    /// whose main packs had filled offered every body for looting and
    /// were refused seven thousand takes.
    ///
    /// Pack slots are a separate count and are left out: a pack, and
    /// each of the five Foci, sits in one of those in the main pack
    /// (see `ac_world::pack_slot`). What the server has said is full
    /// stays full until something leaves it (`packs_said_full`).
    pub(crate) fn packs(&self) -> crate::room::Packs {
        let me = self.world.player_guid;
        let capacity = self
            .world
            .player()
            .map(|p| p.items_capacity)
            .filter(|c| *c > 0)
            .unwrap_or(102);
        let mut packs = crate::room::Packs {
            main: crate::room::Pack {
                guid: me.unwrap_or(0),
                capacity,
                used: 0,
                said_full: false,
            },
            side: Vec::new(),
        };
        let in_a_pack_slot = |o: &ac_world::WorldObject| {
            o.container == me
                && ac_world::pack_slot::used_by(
                    o.weenie_class_id,
                    o.item_type & ac_world::item_type::CONTAINER != 0,
                )
        };
        // The side packs first, so what is in them has somewhere to be
        // counted; by guid, so the choice among equals does not wander
        // between frames.
        for o in self.world.main_pack() {
            if in_a_pack_slot(o) && o.item_type & ac_world::item_type::CONTAINER != 0 {
                packs.side.push(crate::room::Pack {
                    guid: o.guid,
                    capacity: o.items_capacity,
                    used: 0,
                    said_full: false,
                });
            }
        }
        packs.side.sort_by_key(|p| p.guid);
        for o in self.world.inventory() {
            if in_a_pack_slot(o) {
                continue;
            }
            if o.container == me {
                packs.main.used += 1;
            } else if let Some(p) = packs.side.iter_mut().find(|p| Some(p.guid) == o.container) {
                p.used += 1;
            }
        }
        for p in std::iter::once(&mut packs.main).chain(packs.side.iter_mut()) {
            p.said_full = self
                .packs_said_full
                .get(&p.guid)
                .is_some_and(|held| crate::room::still_full(*held, p.used));
        }
        packs
    }

    /// The thing `guid`, lying loose, as the choice of how to take it
    /// sees it (see `room::Loose`).
    fn loose(o: &ac_world::WorldObject) -> crate::room::Loose {
        crate::room::Loose {
            wcid: o.weenie_class_id,
            count: o.stack_size.max(1),
            // Coin and spell components: what the server never caps or
            // makes unique, so a pour that skips a take's checks skips
            // nothing that matters.
            pours: o.max_stack_size > 1
                && o.item_type
                    & (ac_world::item_type::MONEY | ac_world::item_type::SPELL_COMPONENTS)
                    != 0,
            is_pack: o.item_type & ac_world::item_type::CONTAINER != 0,
        }
    }

    /// How the loose thing `guid` is to be taken -- into which pack, or
    /// poured onto which carried stack -- or `None` when it is unknown
    /// or there is nowhere for it (see `room::how_to_take`).
    pub(crate) fn how_to_take(&self, guid: u32) -> Option<crate::room::Take> {
        let o = self.world.objects.get(&guid)?;
        let item = Self::loose(o);
        // Every stack carried is copied by name; only a thing that can
        // pour needs them.
        let carried = if item.pours {
            self.pack_stacks()
        } else {
            Vec::new()
        };
        crate::room::how_to_take(&item, &carried, &self.packs())
    }

    /// The pack the loose thing `guid` is to be picked up into, or
    /// `None` when it is unknown or no pack has a slot: the choice a
    /// take makes, less the pour. A pack off the ground goes to the
    /// main pack's pack slots whatever the room; named into a sack, the
    /// server turns it down without a word, and a pickup's answer is
    /// not read.
    pub(crate) fn where_to_pick_up(&self, guid: u32) -> Option<u32> {
        let o = self.world.objects.get(&guid)?;
        match crate::room::how_to_take(&Self::loose(o), &[], &self.packs())? {
            crate::room::Take::Put(into) => Some(into),
            crate::room::Take::Merge { .. } => None,
        }
    }

    /// Hold what the server said was full against what each pack holds
    /// now (see `room::full_mark`): the word is kept at the most the
    /// pack has been seen to hold since, so that one thing leaving
    /// lifts it, and lifted once something has.
    pub(crate) fn note_pack_counts(&mut self) {
        if self.packs_said_full.is_empty() {
            return;
        }
        let counts: Vec<(u32, u32)> = self.packs().all().map(|p| (p.guid, p.used)).collect();
        for (guid, used) in counts {
            if let Some(held) = self.packs_said_full.get(&guid).copied() {
                match crate::room::full_mark(held, used) {
                    Some(mark) => {
                        self.packs_said_full.insert(guid, mark);
                    }
                    None => {
                        self.packs_said_full.remove(&guid);
                    }
                }
            }
        }
    }

    /// The server has said "Unable to put {item} into container" (see
    /// `room::unable_to_put`). About the take in flight, when it names
    /// that take's item: the refusal that goes with it says nothing
    /// more, and has usually been read already -- a tick reads its
    /// chat after its events -- so the refusal is judged again now.
    pub(crate) fn hear_put_refusal(&mut self, text: &str) {
        let Some(name) = crate::room::unable_to_put(text) else {
            return;
        };
        let Some(sent) = self.loot_sent.as_mut() else {
            return;
        };
        if self
            .world
            .objects
            .get(&sent.item)
            .is_some_and(|o| o.name == name)
        {
            sent.said_full = true;
            self.judge_take_refusal();
        }
    }

    /// The server has turned down the take `item` with `err`. Kept with
    /// the take and judged (see [`Client::judge_take_refusal`]): the
    /// words that say why, when there are any, are still to come.
    ///
    /// A pour off the body is not read this way: a pour is refused for
    /// weight or for a full stack, never for a slot, and its answer is
    /// read where it was sent (see `Client::tick_loot`).
    pub(crate) fn take_refused(&mut self, item: u32, err: u32) {
        if self.loot_merge.is_some() {
            return;
        }
        let Some(sent) = self.loot_sent.as_mut().filter(|s| s.item == item) else {
            return;
        };
        sent.refused = Some(err);
        self.judge_take_refusal();
    }

    /// Read a refused take as the pack it named being full when the
    /// server said so in words (see `room::unable_to_put`), and only
    /// then: that pack is full until something leaves it, and the take
    /// goes once more into another pack with room. There is no third
    /// try, and a body nothing on it can go into is set aside for room
    /// by the loot rules rather than opened again -- reopened, it was
    /// refused the same take thirty times over.
    ///
    /// The words are the whole of it. A refusal with no reason and no
    /// word is not the pack: it is what the server sends for a second
    /// of a unique, whose explanation comes in the system chat where
    /// nothing here reads it, and for a pack put into a sack. Read as
    /// the pack being full, either marked every pack in turn and sent
    /// the character to town with its slots free.
    fn judge_take_refusal(&mut self) {
        let Some(sent) = self.loot_sent.clone() else {
            return;
        };
        if sent.refused.is_none() || !sent.said_full {
            return;
        }
        let item = sent.item;
        // A pack goes in a pack slot, and a pack slot refused says
        // nothing about the item slots.
        let is_pack = self
            .world
            .objects
            .get(&item)
            .is_some_and(|o| o.item_type & ac_world::item_type::CONTAINER != 0);
        if is_pack {
            return;
        }
        // Judged: whatever else is said of it is not read twice.
        self.loot_sent = None;
        let held = self
            .packs()
            .all()
            .find(|p| p.guid == sent.into)
            .map(|p| p.used)
            .unwrap_or(0);
        self.packs_said_full.insert(sent.into, held);
        let pack = self.pack_name(sent.into);
        if sent.retried {
            tracing::info!("the server says {pack} is full too ({held} items); leaving the rest");
            return;
        }
        match self
            .packs()
            .container_for_a_take()
            .filter(|c| *c != sent.into)
        {
            Some(other) => {
                let name = self
                    .world
                    .objects
                    .get(&item)
                    .map(|o| o.name.clone())
                    .unwrap_or_else(|| format!("{item:#010x}"));
                tracing::info!(
                    "the server says {pack} is full ({held} items); taking {name} into {} instead",
                    self.pack_name(other)
                );
                // The refusal was the pack's, not the item's: left in
                // place, the rules would pass the item over.
                self.move_refused.remove(&item);
                self.loot_queue.push_front(item);
                self.loot_retry = Some(item);
            }
            None => {
                tracing::info!(
                    "the server says {pack} is full ({held} items), and no pack has room"
                );
            }
        }
    }

    /// What to call the pack `guid` in the log: the main pack, or the
    /// side pack's own name.
    fn pack_name(&self, guid: u32) -> String {
        if Some(guid) == self.world.player_guid {
            return "the main pack".into();
        }
        self.world
            .objects
            .get(&guid)
            .map(|o| o.name.clone())
            .unwrap_or_else(|| format!("{guid:#010x}"))
    }

    /// There is no room for another item.
    /// A body within reach that has not been emptied yet.
    ///
    /// What "within reach" means matters: a corpse across the dungeon
    /// is not something the character owes anything to, and waiting on
    /// it would stop the fighting altogether. This is about the one at
    /// its feet that it just made.
    pub fn owes_a_corpse(&self) -> bool {
        // No room to take anything: no body is owed, not even the one on
        // its way, and the fight is not held for it (see
        // `Autoplay::corpse_waiting`).
        let room = self.room_for_loot();
        if room.pack_low {
            return false;
        }
        let now = Instant::now();
        // A far body is being asked about, and may be one its creature
        // killed (see `Client::autoplay_claim_pet_kills`).
        if self.autoplay.whose.waiting(now) {
            return true;
        }
        // A body on the floor that this character will go to. One it
        // will not -- set aside, out of reach, a mate's to open -- is not
        // waited on: waiting on a body it is never going to open would
        // stop the fighting altogether (see [`Client::corpse_for_us`]).
        let on_the_floor = self
            .player
            .as_ref()
            .map(|p| p.world_position())
            .is_some_and(|me| {
                self.world
                    .objects
                    .values()
                    .any(|o| self.corpse_for_us(o, me, now, room))
            });
        // Otherwise the only thing owed is a body of its own still on
        // its way.
        on_the_floor || self.own_body_still_falling(now)
    }

    /// Whether the body `o` is one this character would walk to and
    /// empty from where it stands (`me`): a corpse, waiting and its own
    /// ([`Autoplay::corpse_owed`]), not a mate's to open
    /// ([`Autoplay::ours_to_open`]), and not another player's remains.
    ///
    /// Every question about the bodies on the floor asks this one: which
    /// the looting goes to, which the next fight waits on, and what
    /// looting is worth against the next fight. They did not, and so
    /// came to disagree: what looting was worth counted every body
    /// waiting, ownership and all, so eight bodies another character had
    /// locked lifted this one's loot score past the fight it was in the
    /// middle of, with nothing for the looting to actually do when it
    /// won the tick. Nine characters spent 53% of a run standing over
    /// bodies and 13% fighting.
    pub(crate) fn corpse_for_us(
        &self,
        o: &ac_world::WorldObject,
        me: glam::Vec3,
        now: Instant,
        room: Room,
    ) -> bool {
        if o.object_desc_flags & ac_world::object_desc_flags::CORPSE == 0 {
            return false;
        }
        let Some(at) = o.world_pos() else {
            return false;
        };
        let my_guid = self.world.player_guid.unwrap_or(0);
        self.autoplay.corpse_owed(o.guid, at, me, now, room)
            && self.autoplay.ours_to_open(o.guid, at, my_guid, me, now)
            && !self.corpse_is_someone_elses(&o.name)
    }

    /// Whether this character's own last killing blow has yet to leave a
    /// body: the server makes one a moment after the creature dies (see
    /// [`CORPSE_APPEARS`]), and the next fight waits that moment out
    /// rather than walking off from a body about to appear.
    ///
    /// Its own kill and nobody else's: the server tells the last damager
    /// alone (ACE `Creature_Death.GetDeathMessage`), so `last_kill` is
    /// never a mate's. And only until its own body has turned up,
    /// because from then on the bodies on the floor are the answer and
    /// they are read first -- a mate's claim among them. Held for the
    /// whole three seconds regardless, nine characters killing in one
    /// huddle each held the next fight for a body somebody else was
    /// already opening.
    ///
    /// Its own body, and not simply the next body to be noted: in a
    /// huddle a mate's corpse comes into view within those three seconds
    /// constantly, and each one ended the hold early and sent the
    /// character off after the next fight leaving the body it had just
    /// made to be scored on its own. A kill of this character's leaves a
    /// kill spot at the same instant as `last_kill`, so the body that
    /// ends the hold is one lying at one of those.
    fn own_body_still_falling(&self, now: Instant) -> bool {
        let Some(blow) = self.autoplay.last_kill else {
            return false;
        };
        if now.saturating_duration_since(blow) >= CORPSE_APPEARS {
            return false;
        }
        let ours = |guid: u32| {
            self.world
                .objects
                .get(&guid)
                .and_then(|o| o.world_pos())
                .is_some_and(|at| near_a_kill(at, &self.autoplay.kill_spots))
        };
        !self
            .autoplay
            .corpse_seen
            .iter()
            .any(|(guid, seen)| *seen >= blow && ours(*guid))
    }

    /// Whether the corpse named `corpse` is another player's, and theirs:
    /// named for anyone but this character who is a player in view or on
    /// the team, or for no creature there is.
    fn corpse_is_someone_elses(&self, corpse: &str) -> bool {
        let lower = corpse.to_lowercase();
        let Some(who) = lower.strip_prefix("corpse of ") else {
            return false;
        };
        if who == self.world.stats.name.to_lowercase() {
            return false;
        }
        let a_player = self
            .world
            .objects
            .values()
            .filter(|o| o.is_player)
            .map(|o| o.name.as_str())
            .chain(self.autoplay.team.mates.iter().map(|m| m.name.as_str()))
            .any(|n| n.to_lowercase() == who);
        a_player || ac_world::elements::creature(corpse.get(10..).unwrap_or("")).is_none()
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

    /// Whether what has just attacked the character is called `name`.
    /// The server names the attacker in what it sends, hit, miss or
    /// spell, so this is all there is to go on, and it is enough: it is
    /// what says a creature outside the hunting area, or one otherwise
    /// walked past, is a fight the character is already in.
    ///
    /// A creature's move target being this character is not asked here.
    /// It says something is walking at us, which our own pets, a fellow
    /// and a wandering townsfolk all do, and it carries no time, so it
    /// could not say when the fight began. The two notifications and the
    /// spell lines (see [`spell_attacker`]) say outright that an attack
    /// was aimed at this character, and when.
    pub(crate) fn hit_lately_by(&self, name: &str) -> bool {
        self.under_attack()
            && self
                .autoplay
                .hit_by
                .as_ref()
                .is_some_and(|(who, _)| who == name)
    }

    /// The next fight waits for the body the last one left: every body
    /// is to be emptied first, one is owed, and nothing is hitting the
    /// character meanwhile. A character that does not loot owes nothing,
    /// or the fighting would stop for good.
    pub fn waits_for_a_corpse(&self) -> bool {
        self.loot_profile()
            .is_some_and(|p| p.looting.after_every_fight)
            && self.owes_a_corpse()
            && !self.under_attack()
    }

    /// No pack has a slot for a take.
    pub fn pack_full(&self) -> bool {
        self.packs().for_a_take() == 0
    }

    /// What is known about the kind of creature `guid` is.
    fn creature_known(&self, guid: u32) -> Option<&'static ac_world::elements::Creature> {
        let o = self.world.objects.get(&guid)?;
        ac_world::elements::known(o.weenie_class_id, &o.name)
    }

    /// Whether `o` is a critter to walk past rather than fight (see
    /// [`beneath_fighting`]).
    ///
    /// Three things always outrank the rule, because in each of them
    /// the fight is already happening or was asked for: the creature is
    /// swinging at the character, hit or miss, a creature this one
    /// summoned has taken it on, or the player named it in "only these",
    /// which is a player saying outright what to hunt.
    ///
    /// The table is asked first and the three are only consulted for
    /// something the table would have the character walk past. This runs
    /// for every creature in view every tick -- `worth_fighting` weighs
    /// with it -- and one of the three walks the whole object map.
    pub(crate) fn a_critter(&self, o: &ac_world::WorldObject, cfg: &Fight) -> bool {
        if !cfg.skip_critters {
            return false;
        }
        let Some(kind) = ac_world::elements::known(o.weenie_class_id, &o.name) else {
            return false;
        };
        // A level read off this very creature beats the table's, which
        // is a weenie's -- and may be a weenie found by name rather than
        // by id, so a stronger version of a familiar thing. Not every
        // weenie carries one either, so the appraisal is often the only
        // level there is.
        let level = self
            .appraisals
            .get(&o.guid)
            .and_then(|a| a.int(CREATURE_LEVEL))
            .and_then(|l| u32::try_from(l).ok())
            .or(kind.level);
        if !beneath_fighting(kind.tolerance, kind.health, level, self.world.stats.level) {
            return false;
        }
        !(name_matches(&o.name, &cfg.only)
            || self.hit_lately_by(&o.name)
            || self.a_pet_is_on(o.guid))
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

    /// Whether a creature this character summoned is already walking at
    /// `guid`: its fight, and the character's to finish.
    fn a_pet_is_on(&self, guid: u32) -> bool {
        let Some(me) = self.world.player_guid else {
            return false;
        };
        let on = Some(ac_world::object::MoveTarget::Object(guid));
        self.world
            .objects
            .values()
            .any(|o| o.pet_owner == me && o.target == on)
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
        // Already on one that is still alive.
        if let Some(t) = self.attack_target {
            if self.stalled_on(t, now) {
                return false;
            }
            // And still here. A target is chosen from within the fight
            // radius, but nothing checked it was still within it
            // afterwards -- so a creature the character walked away
            // from, or left behind in the Academy, stayed its target
            // for ever while it planned a journey to the other side of
            // the world to swing at it.
            // Or gone out of the hunting area, and not hitting us: let it go.
            let underground = self.underground();
            let gone = self
                .world
                .objects
                .get(&t)
                .and_then(|o| o.world_pos())
                .zip(self.player.as_ref().map(|p| p.world_position()))
                .is_some_and(|(at, me)| at.distance(me) > crate::travel::WALKABLE)
                || !self.area_allows_guid(t, underground);
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
        // Finish what it killed before setting off after the next one.
        if self.waits_for_a_corpse() {
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
        // Hunting together: hit what the team is hitting, unless it is
        // something this character is walking past on its way somewhere.
        let team = &self.autoplay.config.team;
        if team.enabled && team.focus_fire && !self.autoplay.team.leader {
            let joined = self
                .autoplay
                .team
                .target()
                .filter(|(guid, _)| self.joins_the_team_on(*guid, &cfg));
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
        // Stay on the one already being fought while it lives, and is
        // taking damage.
        if let Some(g) = self.autoplay.casting_at {
            if self.stalled_on(g, now) {
                return false;
            }
        }
        // Out of the hunting area, and not hitting us, it is let go.
        let underground = self.underground();
        let target = match self.autoplay.casting_at {
            Some(g) if alive(self, g) && self.area_allows_guid(g, underground) => Some(g),
            _ => {
                self.autoplay.casting_at = None;
                if self.waits_for_a_corpse() {
                    None
                } else {
                    self.pick_target(cfg)
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

    /// A swing cancels the journey (the move-to and the trip cannot both
    /// steer). Note where it was going, so it is taken up again once
    /// the fight is over.
    fn remember_journey(&mut self) {
        if self.traveling() {
            if let Some(goal) = self.travel_goal_xy() {
                self.autoplay.resume_trip = Some(goal);
            }
        }
    }

    /// Pick the journey up again after a fight, once there is nothing
    /// else to do.
    pub(crate) fn autoplay_resume_journey(&mut self) -> bool {
        let Some(goal) = self.autoplay.resume_trip else {
            return false;
        };
        if self.traveling() || self.attack_target.is_some() || self.autoplay.casting_at.is_some() {
            return false;
        }
        self.autoplay.resume_trip = None;
        if self.travel_to(goal) {
            self.autoplay.say(Doing::Idle, "back on the road");
            return true;
        }
        false
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
                    let name = self
                        .world
                        .objects
                        .get(&guid)
                        .map(|o| o.name.clone())
                        .unwrap_or_else(|| format!("{guid:#010x}"));
                    self.autoplay
                        .note(format!("giving up on {name}: no damage in a while"), now);
                    self.autoplay
                        .given_up
                        .retain(|(_, t)| now.duration_since(*t) < GIVE_UP_FOR);
                    self.autoplay.given_up.push((guid, now));
                    self.autoplay.engaged = None;
                    self.autoplay.closing = None;
                    self.attack_target = None;
                    self.autoplay.casting_at = None;
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

    /// A line from the server while a body is waiting to open. One
    /// refusing that body is acted on at once rather than waited out
    /// (see [`Autoplay::corpse_refused`]), and the walk to it, if there
    /// was one, ends with it.
    pub(crate) fn hear_corpse_refusal(&mut self, text: &str, now: Instant) {
        let Some((guid, ..)) = self.autoplay.corpse else {
            return;
        };
        // What the server calls it. Without the name there is no telling
        // this body's refusal from another's, so nothing is done.
        let Some(name) = self.world.objects.get(&guid).map(|o| o.name.clone()) else {
            return;
        };
        if self.autoplay.corpse_refused(text, &name, self.told, now) {
            self.stop_walking_to_loot();
        }
    }

    /// A line saying something cast a spell at this character to hurt it
    /// (see [`spell_attacker`]), kept in the same two fields a swing sets.
    /// Every "but fight back when it attacks you" carve-out reads those,
    /// and a caster attacks as surely as a creature that swings.
    pub(crate) fn hear_spell_attack(&mut self, text: &str, now: Instant) {
        let Some(who) = spell_attacker(text) else {
            return;
        };
        self.autoplay.last_hit_us = Some(now);
        self.autoplay.hit_by = Some((who.to_string(), now));
    }

    /// A line saying a shot got to something and did not hurt it (see
    /// [`arrived_unharmed`]): noted against the target it names.
    pub(crate) fn hear_arrival(&mut self, text: &str) {
        let Some(name) = arrived_unharmed(text) else {
            return;
        };
        let target = [self.autoplay.casting_at, self.attack_target]
            .into_iter()
            .flatten()
            .find(|g| self.world.objects.get(g).is_some_and(|o| o.name == name));
        if let Some(g) = target {
            self.autoplay.thrown = self.autoplay.thrown.filter(|(t, _)| *t != g);
            // Resisted or evaded, it still got there: the target is in
            // reach and being worked on, whatever its health says. Counting
            // only damage gave a creature that resisted a run of spells up
            // as out of reach.
            if let Some((engaged, _, health)) = self.autoplay.engaged {
                if engaged == g {
                    self.autoplay.engaged = Some((g, Instant::now(), health));
                }
            }
        }
    }

    /// A shot has gone out at `guid`: the clock on it getting there
    /// starts now, unless one is already running for that target.
    fn throw_at(&mut self, guid: u32, now: Instant) {
        if self.autoplay.thrown.is_none_or(|(g, _)| g != guid) {
            self.autoplay.thrown = Some((guid, now));
        }
    }

    /// Where the creature a kill message names was standing: the one
    /// being fought when the message names it, else the nearest creature
    /// it names, the longest name that fits first (a "Mite Scion" is not
    /// a "Mite").
    pub(crate) fn killed_in(&self, text: &str) -> Option<glam::Vec3> {
        let me = self.player.as_ref()?.world_position();
        let named = |name: &str| !name.is_empty() && text.contains(name);
        let fought = [self.attack_target, self.autoplay.casting_at]
            .into_iter()
            .flatten()
            .filter_map(|g| self.world.objects.get(&g))
            .find(|o| named(&o.name))
            .and_then(|o| o.world_pos());
        fought.or_else(|| {
            self.world
                .objects
                .values()
                .filter(|o| o.item_type & ac_world::item_type::CREATURE != 0)
                .filter(|o| named(&o.name))
                .filter_map(|o| Some((o.name.len(), o.world_pos()?)))
                .max_by(|a, b| {
                    a.0.cmp(&b.0)
                        .then(b.1.distance(me).total_cmp(&a.1.distance(me)))
                })
                .map(|(_, at)| at)
        })
    }

    /// A corpse is done with: the kill spot it lay at is too.
    fn forget_kill_spot(&mut self, corpse: u32) {
        if let Some(at) = self.world.objects.get(&corpse).and_then(|o| o.world_pos()) {
            self.autoplay
                .kill_spots
                .retain(|(k, _)| k.truncate().distance(at.truncate()) > KILL_SPOT);
        }
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
                if self.travel_to(goal) {
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

    /// Put a buff back up. True when it cast one.
    ///
    /// Two passes share this. The urgent one runs before anything else
    /// each tick and puts back whatever is under `never_below`, in a
    /// fight or out of one, swapping to a wand if the hands hold
    /// something else: a buff is never allowed to run out. The other
    /// runs last, in quiet moments, and tops up whatever is under the
    /// much wider `top_up_within`, a cast or two at a time, so the set
    /// is refreshed a little at every lull rather than all at once.
    pub(crate) fn autoplay_buff(&mut self, now: Instant, urgent: bool) -> bool {
        let cfg = self.autoplay.config.buffs.clone();
        if cfg.spells.is_empty() && !cfg.auto {
            return false;
        }
        let fighting = self.attack_target.is_some() || self.autoplay.casting_at.is_some();
        if !urgent && cfg.out_of_combat_only && fighting {
            return false;
        }
        // On a journey the top-ups wait: every cast roots the character
        // where it stands, and a character with a hundred buffs to put
        // back would never leave town. What is about to run out still
        // goes back up on the way.
        if !urgent && self.traveling() {
            return false;
        }
        if self
            .autoplay
            .last_buff
            .is_some_and(|t| now.duration_since(t) < BUFF_EVERY)
        {
            return false;
        }
        if urgent
            && self
                .autoplay
                .buffs_checked
                .is_some_and(|t| now.duration_since(t) < BUFF_CHECK_EVERY)
        {
            return false;
        }
        // A buff is never worth a cancelled swing: it goes back up in
        // the gap between two of them instead (see
        // `change_of_hands_waits`).
        if change_of_hands_waits(self.combat_stance(), Stance::Magic, self.mid_attack()) {
            self.wait_for_the_swing();
            self.autoplay.note(
                "waiting for the swing to land before reaching for a wand",
                now,
            );
            return false;
        }
        self.autoplay.buffs_checked = Some(now);
        let within = buff_within(
            &cfg,
            urgent,
            fighting,
            self.combat_stance() == Stance::Magic,
        );
        // Find what is due before touching the hands: the urgent pass
        // runs every tick and must cost nothing when nothing is due.
        let Some((spell, target, category, name, lasts)) = self.due_buff(within, now) else {
            if !urgent {
                self.autoplay_explain_buffs(now);
            }
            return false;
        };
        // Mana is kept back for healing and fighting: a top-up waits
        // until there is that much to spare, and even an urgent recast
        // leaves half of it.
        let reserve = self.vital_max_of(2) as f32 * cfg.keep_mana * if urgent { 0.5 } else { 1.0 };
        let cost = self
            .assets
            .spell_table()
            .ok()
            .and_then(|t| t.get(spell).map(|s| self.mana_cost(s)))
            .unwrap_or(0) as f32;
        let have = self.world.stats.vitals[2].current as f32;
        if have - cost < reserve {
            self.autoplay
                .note(format!("holding off {name} to keep mana back"), now);
            return false;
        }
        // A wand is needed to cast. Out of a fight the arming code sorts
        // the hands out afterwards; in one, remember what was put down
        // so it is taken up again the moment the buffing is done.
        if self.combat_stance() != Stance::Magic {
            let held = self
                .world
                .wielded()
                .find(|o| {
                    o.item_type
                        & (ac_world::item_type::MELEE_WEAPON | ac_world::item_type::MISSILE_WEAPON)
                        != 0
                })
                .map(|o| o.guid);
            if !self.wield_for(Stance::Magic) {
                // A wand that is carried but held off after a refusal is
                // a wait, not a want: saying "no wand" for it sent an
                // earlier reader looking through the pack for one.
                self.autoplay.say(
                    Doing::Buffing,
                    if self.carries_a_caster() {
                        format!("waiting to take a wand up to cast {name}")
                    } else {
                        format!("no wand to cast {name} with")
                    },
                );
                return false;
            }
            if fighting && self.autoplay.put_down.is_none() {
                self.autoplay.put_down = held;
            }
            // The wield takes a moment; cast next tick.
            self.autoplay.last_buff = Some(now);
            return true;
        }
        if !matches!(self.can_cast(spell), crate::magic::CastCheck::Ok) {
            return false;
        }
        // Casting a low level of something we know a better version of
        // is worth saying once: it is nearly always components for the
        // higher formula, and a character quietly buffing at level one
        // looks like a bug rather than an empty pack.
        if let Some((better, why)) = self.better_buff_blocked(spell) {
            self.autoplay
                .note(format!("buffing with a weaker spell: {better} {why}"), now);
        }
        use crate::buffs::Target;
        match target {
            Target::Me => {
                self.cast(spell);
                self.autoplay.say(Doing::Buffing, format!("casting {name}"));
            }
            Target::Item(g) => {
                self.cast_at(spell, g);
                self.autoplay
                    .item_buffs
                    .retain(|(i, c, _, _)| !(*i == g && *c == category));
                self.autoplay.item_buffs.push((g, category, now, lasts));
                let on = self
                    .world
                    .objects
                    .get(&g)
                    .map(|o| o.name.clone())
                    .unwrap_or_default();
                self.autoplay
                    .say(Doing::Buffing, format!("casting {name} on {on}"));
            }
        }
        // A buff is a cast like any other as far as the pacing goes:
        // an attack thrown over it would be dropped.
        self.autoplay.last_buff = Some(now);
        self.autoplay.cast_sent = Some(now);
        true
    }

    /// A stronger spell than `spell` that we know, do the same thing
    /// with, and cannot cast: its name and why. `None` when the one we
    /// are about to cast is already the best we know.
    fn better_buff_blocked(&self, spell: u32) -> Option<(String, String)> {
        use crate::magic::CastCheck;
        let table = self.assets.spell_table().ok()?;
        let mine = table.get(spell)?;
        let mut best: Option<(u32, String, String)> = None;
        for &id in &self.world.stats.spells {
            let Some(sp) = table.get(id) else { continue };
            // The same buff, only stronger: the category is the family
            // and the power is the level within it.
            if sp.category != mine.category || sp.power <= mine.power {
                continue;
            }
            let check = self.can_cast(id);
            if matches!(check, CastCheck::Ok) {
                continue;
            }
            let why = cast_problem(&check);
            if best.as_ref().is_none_or(|b| sp.power > b.0) {
                best = Some((sp.power, sp.name.clone(), why));
            }
        }
        best.map(|(_, name, why)| (name, why))
    }

    /// When buffs are wanted but none can be cast, say why for the
    /// first of them, so a character standing unbuffed is not a mystery.
    fn autoplay_explain_buffs(&mut self, now: Instant) {
        use crate::buffs::Target;
        use crate::magic::CastCheck;
        if !self.autoplay.config.buffs.auto {
            return;
        }
        let top_up = self.autoplay.config.buffs.top_up_within;
        for want in self.wanted_buffs() {
            let left = match want.target {
                Target::Me => self.category_left(want.category, want.power),
                Target::Item(g) => self.item_buff_left(g, want.category, now),
            };
            if left.is_some_and(|l| l > top_up) {
                continue;
            }
            let check = self.can_cast(want.spell);
            if matches!(check, CastCheck::Ok | CastCheck::NoCaster) {
                continue;
            }
            let why = cast_problem(&check);
            let name = self
                .assets
                .spell_table()
                .ok()
                .and_then(|t| t.get(want.spell).map(|s| s.name.clone()))
                .unwrap_or_default();
            self.autoplay
                .note(format!("cannot buff: {name}, {why}"), now);
            return;
        }
    }

    /// The maximum of a vital (0 health, 1 stamina, 2 mana).
    fn vital_max_of(&self, i: usize) -> u32 {
        self.world.stats.vital_max_current(i)
    }

    /// Take up again the weapon put down for an urgent buff, once no
    /// buff is due any more.
    pub(crate) fn autoplay_rearm(&mut self) {
        let Some(weapon) = self.autoplay.put_down else {
            return;
        };
        let never_below = self.autoplay.config.buffs.never_below;
        if self.due_buff(never_below, Instant::now()).is_some() {
            return;
        }
        if !self
            .world
            .objects
            .get(&weapon)
            .is_some_and(|o| o.container == self.world.player_guid)
        {
            // Sold, given away, or in hand already: no errand left.
            self.autoplay.put_down = None;
            return;
        }
        // Inside the wait a refusal earned it, or with the server busy,
        // the errand keeps, the way a pending wield's does: dropping it
        // here would leave the weapon in the pack and the character
        // fighting with the wand.
        if self.wield_must_wait(weapon, Instant::now()) {
            return;
        }
        self.autoplay.put_down = None;
        tracing::info!("autoplay: taking the weapon up again after buffing");
        // The wand is still in the hand, and ACE will not put a sword
        // in a hand that holds a caster -- CheckWeaponCollision refuses
        // it outright, with no error to read. So the wand goes back in
        // the pack and the housekeeping takes the weapon up once the
        // hands are empty, the same two steps the arming uses.
        if self.put_weapons_away() {
            self.autoplay.last_rewield = Some(Instant::now());
            self.autoplay.pending_wield = Some(weapon);
        } else {
            self.wield_guid(weapon);
        }
    }

    /// The buff with the least time left of those under `within`
    /// seconds, castable or not: the most pressing one is put back
    /// first. `(spell, target, category, name, seconds it lasts)`.
    pub(crate) fn due_buff(
        &self,
        within: f32,
        now: Instant,
    ) -> Option<(u32, crate::buffs::Target, u32, String, f32)> {
        use crate::buffs::Target;
        let cfg = &self.autoplay.config.buffs;
        let table = self.assets.spell_table().ok();
        // (rank, left, ...): creature magic goes first. Its buffs raise
        // the skills and attributes the other schools cast from, so a
        // character that buffs them first can land higher levels of
        // everything after.
        let mut due: Option<(u8, f32, u32, Target, u32, String, f32)> = None;
        let clock = self.session.server_time().is_some();
        let creature_first = |spell: u32| -> u8 {
            let school = table.as_ref().and_then(|t| t.get(spell)).map(|s| s.school);
            u8::from(school != Some(ac_formats::spell_table::school::CREATURE))
        };
        let mut offer = |left: Option<f32>, spell: u32, target: Target, category: u32| {
            // Not up at all is due now; but until the server's clock is
            // known nothing can be told apart, so nothing is due.
            let left = match left {
                Some(l) => l,
                None if clock => 0.0,
                None => return,
            };
            if left > within {
                return;
            }
            // One that cannot be cast must not stand in front of the
            // rest: short of components or mana it is passed over. No
            // wand is different, since wielding one is the cure.
            match self.can_cast(spell) {
                crate::magic::CastCheck::Ok | crate::magic::CastCheck::NoCaster => {}
                _ => return,
            }
            let rank = creature_first(spell);
            if due.as_ref().is_some_and(|d| (d.0, d.1) <= (rank, left)) {
                return;
            }
            let sp = table.as_ref().and_then(|t| t.get(spell));
            let name = sp.map(|s| s.name.clone()).unwrap_or_default();
            let lasts = sp.and_then(|s| s.duration()).unwrap_or(1800.0) as f32;
            due = Some((rank, left, spell, target, category, name, lasts));
        };
        if cfg.auto {
            for want in self.wanted_buffs() {
                let left = match want.target {
                    Target::Me => self.category_left(want.category, want.power),
                    Target::Item(g) => self.item_buff_left(g, want.category, now),
                };
                offer(left, want.spell, want.target, want.category);
            }
        }
        for name in &cfg.spells {
            let Some(spell) = self.spell_by_name(name) else {
                continue;
            };
            let category = table
                .as_ref()
                .and_then(|t| t.get(spell))
                .map(|s| s.category)
                .unwrap_or(0);
            offer(self.buff_left(spell), spell, Target::Me, category);
        }
        due.map(|(_, _, spell, target, category, name, lasts)| {
            (spell, target, category, name, lasts)
        })
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_quests_refusal_that_names_no_item_is_about_the_take_in_flight() {
        // ACE turns a drop that can only be had so often down naming item
        // 0, with only "You have solved this quest too recently". Read as
        // a refusal of nothing, the take was asked for again until the
        // corpse was given up on, and the kind was never remembered.
        let (gem, dagger) = (0x8000_7001, 0x8000_7002);
        assert_eq!(refused_item(0, 0x043E, Some(gem)), Some(gem));
        assert_eq!(refused_item(0, 0x043F, Some(gem)), Some(gem));
        // With nothing in flight there is nothing to pin it on.
        assert_eq!(refused_item(0, 0x043E, None), None);
        // A refusal naming no item for any other reason is not a take's.
        assert_eq!(refused_item(0, 0, Some(gem)), None);
        // One that names its item is about that item, whatever is flying.
        assert_eq!(refused_item(dagger, 0, Some(gem)), Some(dagger));
        assert!(only_so_often(0x043E) && only_so_often(0x043F));
        assert!(!only_so_often(0));
    }

    #[test]
    fn a_pour_refused_while_a_take_is_in_the_air_is_not_the_takes_refusal() {
        // The tidying pours in the gaps between takes, so both can be
        // out at once. A refusal that names a stack in the pack is
        // about that stack, and the take goes on waiting for its own
        // answer rather than being written off.
        let (in_the_pack, on_the_corpse) = (0x8000_7001, 0x8000_7002);
        assert_eq!(
            refused_item(in_the_pack, 0, Some(on_the_corpse)),
            Some(in_the_pack)
        );
    }

    #[test]
    fn a_profile_with_tidy_pack_off_is_never_tidied() {
        assert_eq!(
            why_not_tidy(TidyGate {
                tidy_pack_off: true,
                ..TidyGate::default()
            }),
            Some("this profile leaves the pack as it is")
        );
    }

    #[test]
    fn nothing_is_poured_with_a_counter_open() {
        // A sale holds what it has sent the vendor by guid, and a pour
        // makes one of those vanish out from under it.
        assert_eq!(
            why_not_tidy(TidyGate {
                counter_open: true,
                ..TidyGate::default()
            }),
            Some("a counter is open")
        );
    }

    #[test]
    fn nor_while_the_quartermaster_is_loaded_or_unloaded() {
        // Money counted out into its own stack was poured straight back
        // into the pile it came from, a hundred and twenty six times.
        assert_eq!(
            why_not_tidy(TidyGate {
                quartermaster: true,
                ..TidyGate::default()
            }),
            Some("the quartermaster is being loaded or unloaded")
        );
    }

    #[test]
    fn nor_while_ammunition_is_being_made() {
        assert_eq!(
            why_not_tidy(TidyGate {
                crafting: true,
                ..TidyGate::default()
            }),
            Some("ammunition is being made")
        );
    }

    #[test]
    fn nor_just_after_a_hand_over_to_a_teammate() {
        assert_eq!(
            why_not_tidy(TidyGate {
                gave_lately: true,
                ..TidyGate::default()
            }),
            Some("something was just handed to a teammate")
        );
    }

    #[test]
    fn nor_while_a_take_is_queued_or_in_the_air() {
        assert_eq!(
            why_not_tidy(TidyGate {
                take_in_air: true,
                ..TidyGate::default()
            }),
            Some("a take is queued or in the air")
        );
    }

    #[test]
    fn with_nothing_in_the_way_the_pack_is_tidied() {
        assert_eq!(why_not_tidy(TidyGate::default()), None);
        // The first reason that applies is the one given.
        assert_eq!(
            why_not_tidy(TidyGate {
                counter_open: true,
                take_in_air: true,
                ..TidyGate::default()
            }),
            Some("a counter is open")
        );
    }

    #[test]
    fn every_errand_holding_a_stack_is_offered_up() {
        // Whatever another part of the rules is holding across ticks
        // must not be poured away under it.
        let mut ap = Autoplay::default();
        assert_eq!(ap.held_by_an_errand().iter().flatten().count(), 0);
        ap.pending_wield = Some(1);
        ap.wanted_ammo = Some(2);
        ap.put_down = Some(3);
        ap.crafting = Some((4, 5, Instant::now()));
        ap.handing = Some((6, Instant::now()));
        let held: Vec<u32> = ap.held_by_an_errand().iter().flatten().copied().collect();
        assert_eq!(held, vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn a_daily_limit_is_a_wait_and_not_a_grudge() {
        use crate::did::{Because, Did, Patience};
        let t0 = Instant::now();
        let mut kinds: Patience<u32> = Patience::new();

        // YouHaveSolvedThisQuestTooRecently is what gates a once-a-day
        // drop. It is a wait: the thing comes back, and a session can
        // run for days.
        let too_recently = Did::Blocked(Because::server(0x043E));
        assert_eq!(too_recently.because().and_then(|b| b.code), Some(0x043E));
        kinds.note(7299, &too_recently, t0);
        assert!(kinds.held(&7299, t0), "left alone for now");
        // Not for ever, though: a day later it is asked about again.
        assert!(
            !kinds.held(&7299, t0 + Duration::from_secs(24 * 60 * 60)),
            "a day later it is worth another ask"
        );
        // Nor is "too many times", which a raised cap can lift.
        kinds.note(7299, &Did::Blocked(Because::server(0x043F)), t0);
        assert!(!kinds.held(&7299, t0 + Duration::from_secs(24 * 60 * 60)));
        // Only a thing no counter will ever take is for ever, and that
        // is a different answer entirely.
        kinds.note(1, &Did::refused("no vendor will take it"), t0);
        assert!(kinds.held(&1, t0 + Duration::from_secs(24 * 60 * 60)));
    }

    #[test]
    fn a_buff_pass_that_wants_a_wand_waits_rather_than_disarming_mid_charge() {
        use crate::Stance;
        // +Verity's buffing reached for her Training Wand every second
        // and a half while she was charging a Drudge Servant, and every
        // reach put her mace away and cancelled the charge with it.
        assert!(change_of_hands_waits(Stance::Melee, Stance::Magic, true));
        assert!(change_of_hands_waits(Stance::Missile, Stance::Magic, true));

        // Answered, the gap between two swings is hers: the wand goes in
        // then, and nothing is cancelled.
        assert!(!change_of_hands_waits(Stance::Melee, Stance::Magic, false));

        // A pass that already holds a wand changes nothing, so there is
        // nothing to wait for and the buff goes up mid-fight as before.
        assert!(!change_of_hands_waits(Stance::Magic, Stance::Magic, true));

        // The rule is about hands, not about wands: a character told to
        // fight with a bow waits for the swing just the same.
        assert!(change_of_hands_waits(Stance::Melee, Stance::Missile, true));
    }

    #[test]
    fn a_refused_wield_is_left_alone_for_longer_every_time() {
        use crate::did::Patience;
        // ACE refuses a wield it will not make with no error code at all
        // -- a caster cannot go in while a shield is up, and it says so
        // with WeenieError.None -- so there is nothing to read and
        // nothing to do but wait. +Verity asked 150 times in a minute.
        const WAND: u32 = 0x8000_00C3;
        let t0 = Instant::now();
        let mut held: Patience<u32> = Patience::new();

        held.hold(WAND, WIELD_AGAIN, t0);
        assert!(held.held(&WAND, t0), "not asked for again at once");
        assert!(
            held.held(&WAND, t0 + WIELD_AGAIN - Duration::from_millis(1)),
            "nor a moment before the wait is up"
        );
        assert!(
            !held.held(&WAND, t0 + WIELD_AGAIN),
            "asked again once the wait is up"
        );

        // Refused again: the wait doubles, so an item the server will
        // never wield in this state costs a handful of asks rather than
        // one every buff pass.
        let second = t0 + WIELD_AGAIN;
        held.hold(WAND, WIELD_AGAIN, second);
        assert_eq!(held.waited(&WAND), Some(WIELD_AGAIN * 2));
        assert!(held.held(&WAND, second + WIELD_AGAIN));
        assert!(!held.held(&WAND, second + WIELD_AGAIN * 2));

        // A wield that lands forgets the wait: the hands have changed,
        // so whatever the server was objecting to has gone.
        held.forget(&WAND);
        assert!(!held.held(&WAND, second));
        assert_eq!(held.waited(&WAND), None);
    }

    use super::*;
    use crate::items::ItemStats;

    #[test]
    fn ammunition_is_made_for_the_bow_and_the_targets_weakness() {
        use ac_world::elements::Element;
        use ac_world::fletching::ammo_type;
        // Plain arrowheads (4586), fire arrowheads (5341), arrowshafts
        // (4585) and quarrel shafts (5339), by guid.
        let carried = [(4586, 1), (5341, 2), (4585, 3), (5339, 4)];
        // Fletching enough for fire arrows, against something weak to fire.
        let (r, heads, shafts) =
            choose_recipe(ammo_type::ARROW, 100, &carried, Some(Element::Fire)).expect("fire");
        assert_eq!(
            (r.result_name.as_str(), heads, shafts),
            ("Fire Arrow", 2, 3)
        );
        // Weak to cold and no cold heads carried: the hardest recipe
        // that can be made, which is still the fire one.
        let (r, _, _) =
            choose_recipe(ammo_type::ARROW, 100, &carried, Some(Element::Cold)).expect("any");
        assert_eq!(r.result_name, "Fire Arrow");
        // Not skilled enough for fire arrows: plain ones.
        let (r, heads, shafts) =
            choose_recipe(ammo_type::ARROW, 10, &carried, Some(Element::Fire)).expect("plain");
        assert_eq!((r.result_name.as_str(), heads, shafts), ("Arrow", 1, 3));
        // A crossbow wants quarrels, made on the quarrel shafts.
        let (r, _, shafts) = choose_recipe(ammo_type::BOLT, 100, &carried, None).expect("quarrels");
        assert_eq!((r.result_name.as_str(), shafts), ("Fire Quarrel", 4));
        // No dart shafts: nothing for an atlatl.
        assert!(choose_recipe(ammo_type::ATLATL, 100, &carried, None).is_none());
        // Untrained (0): nothing at all.
        assert!(choose_recipe(ammo_type::ARROW, 0, &carried, None).is_none());
    }

    #[test]
    fn a_follower_strays_no_further_than_twice_its_distance() {
        assert_eq!(follow_break(4.0), 10.0);
        assert_eq!(follow_break(8.0), 16.0);
    }

    #[test]
    fn target_rules_read_names() {
        let mut f = Fight::default();
        assert!(wanted_target("Drudge Skulker", &f));
        f.only = vec!["drudge".into()];
        assert!(wanted_target("Drudge Skulker", &f));
        assert!(!wanted_target("Olthoi Grub", &f));
        f.only = vec!["  ".into()];
        assert!(wanted_target("anything", &f), "a blank rule means any");
        f.only = Vec::new();
        f.avoid = vec!["Olthoi".into()];
        assert!(!wanted_target("Olthoi Grub", &f));
        assert!(wanted_target("Drudge Skulker", &f));
        // Avoid wins over only.
        f.only = vec!["olthoi".into()];
        assert!(!wanted_target("Olthoi Grub", &f));
    }

    #[test]
    fn a_creature_is_beneath_fighting_only_when_it_is_all_three() {
        use ac_world::elements::tolerance;
        // A level 4 Rabbit with its five health: fought at 7, walked
        // past from 8 up.
        assert!(!beneath_fighting(tolerance::RETALIATE, 5, Some(4), 5));
        assert!(!beneath_fighting(tolerance::RETALIATE, 5, Some(4), 7));
        assert!(beneath_fighting(tolerance::RETALIATE, 5, Some(4), 8));
        assert!(beneath_fighting(tolerance::RETALIATE, 5, Some(4), 20));
        // A level 61 Revenant is passive and nobody outgrows it: 122
        // is past the level a character can reach.
        assert!(!beneath_fighting(tolerance::RETALIATE, 200, Some(61), 100));
        assert!(!beneath_fighting(tolerance::RETALIATE, 200, Some(61), 121));
        // Something that attacks on sight is fought however small.
        assert!(!beneath_fighting(0, 3, Some(1), 275));
        // And nothing is walked past on a guess: no level, no health.
        assert!(!beneath_fighting(tolerance::RETALIATE, 5, None, 275));
        assert!(!beneath_fighting(tolerance::RETALIATE, 0, Some(4), 275));
        // Every flag that means it leaves a passer-by alone counts.
        for flag in [
            tolerance::NO_ATTACK,
            tolerance::APPRAISE,
            tolerance::PROVOKE,
            tolerance::RETALIATE,
            tolerance::MONSTER,
        ] {
            assert!(beneath_fighting(flag, 5, Some(4), 20), "{flag}");
        }
        // Something that cannot fight back at all is scenery with a
        // health bar, and hitting it is a chore whatever it can take: a
        // Portal Pillar has two thousand health and never swings.
        assert!(beneath_fighting(tolerance::NO_ATTACK, 2001, Some(4), 50));
        // The rest of the flags do fight back once hit, so what they
        // can take is what decides: a Drudge Skulker's forty-two is a
        // fight, a Rabbit's five is not.
        assert!(!beneath_fighting(tolerance::RETALIATE, 42, Some(8), 50));
        // The ones that do not: "only fight back at whoever started it"
        // still starts fights with everyone else.
        assert!(!beneath_fighting(32, 5, Some(4), 20));
        // A character with no level yet fights everything.
        assert!(!beneath_fighting(tolerance::RETALIATE, 5, Some(4), 0));
    }

    #[test]
    fn the_holtburg_fields_are_still_a_hunting_ground_at_any_level() {
        use ac_world::elements::creature_by_id;
        // Every creature the two encounter generators around Holtburg
        // put out (2007 newbietownaluviangen, 5150 harmlessaluviangen),
        // by weenie. ACE gives the first eight Retaliate so they do not
        // come at a new player, and they are all level 8: by level
        // alone a level 16 character had nothing left to attack
        // anywhere in Holtburg.
        let field = [
            19257, // Drudge Skulker
            19258, // Drudge Slinker
            19263, // Gnawer Shreth
            19261, // Creeper Mosswart
            19262, // Young Mosswart
            19256, // Young Banderling
            19260, // Mite Snippet
            19259, // Mite Scion
        ];
        for wcid in field {
            let c = creature_by_id(wcid).expect("in the table");
            for mine in [16, 20, 50, 275] {
                assert!(
                    !beneath_fighting(c.tolerance, c.health, c.level, mine),
                    "{} is what the fields are for, at level {mine}",
                    c.name
                );
            }
        }
        // The two that really are critters are still walked past.
        for wcid in [2566, 24937] {
            let c = creature_by_id(wcid).expect("in the table");
            assert!(
                beneath_fighting(c.tolerance, c.health, c.level, 20),
                "{} is worth nothing to a level 20 character",
                c.name
            );
        }
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

    /// A character of `level` with nothing around it yet.
    fn character_of_level(level: i32) -> Option<Client> {
        let Some(dir) = std::env::var_os("AC_DATA_DIR") else {
            eprintln!("AC_DATA_DIR unset; skipping");
            return None;
        };
        let assets = std::rc::Rc::new(ac_scene::Assets::open(dir).unwrap());
        let mut c = offline_client(assets);
        c.world.player_guid = Some(0x5000_0001);
        c.world.stats.level = level;
        Some(c)
    }

    /// The same character, standing somewhere, for the rules that read
    /// a position.
    fn standing_in_the_field(level: i32, cell: u32, local: glam::Vec3) -> Option<Client> {
        let mut c = character_of_level(level)?;
        let assets = c.assets.clone();
        let mut pl = crate::player::Player::new(&assets, cell, local, glam::Quat::IDENTITY);
        pl.set_motion_table(&assets, 0x0200_0001, 0x0900_0001);
        c.player = Some(pl);
        Some(c)
    }

    /// A shelf of its own holding one starter profile, so the looting
    /// has rules to carry out without touching the one every session
    /// shares.
    fn a_loot_profile(c: &mut Client, name: &str) {
        let dir = std::env::temp_dir().join("acswarm-test-loot-profiles");
        std::fs::create_dir_all(&dir).ok();
        let shelf = std::sync::Arc::new(crate::profile::Library::default());
        shelf.open(&dir);
        let mut p = crate::profile::Profile::starter();
        p.name = name.into();
        shelf.put(p).ok();
        c.profiles = shelf;
        c.autoplay.config.loot.profile = name.into();
        assert!(c.loot_profile().is_some(), "no rules to loot by");
    }

    /// A creature of `wcid` standing in view, and a copy of it to ask
    /// the fight rules about.
    fn in_view(c: &mut Client, guid: u32, wcid: u32, name: &str) -> ac_world::WorldObject {
        let o = ac_world::WorldObject {
            guid,
            weenie_class_id: wcid,
            name: name.into(),
            item_type: ac_world::item_type::CREATURE,
            health: Some(1.0),
            ..Default::default()
        };
        c.world.objects.insert(guid, o.clone());
        o
    }

    #[test]
    fn a_rabbit_is_walked_past_once_it_is_outgrown_and_a_revenant_never_is() {
        // Brown Rabbit 2567 (passive, level 4), Revenant 8592 (passive,
        // level 61), Chicken 35499 (attacks on sight, level 8).
        let Some(mut c) = character_of_level(20) else {
            return;
        };
        let cfg = Fight::default();
        let rabbit = in_view(&mut c, 0x8000_0001, 2567, "Brown Rabbit");
        let revenant = in_view(&mut c, 0x8000_0002, 8592, "Revenant");
        let chicken = in_view(&mut c, 0x8000_0003, 35499, "Chicken");
        assert!(c.a_critter(&rabbit, &cfg));
        assert!(!c.a_critter(&revenant, &cfg), "worth sixty levels");
        assert!(!c.a_critter(&chicken, &cfg), "this one starts fights");

        // A character the Rabbit is still worth something to fights it.
        c.world.stats.level = 5;
        assert!(!c.a_critter(&rabbit, &cfg));
        assert!(!c.a_critter(&revenant, &cfg));

        // Back at 20, and hitting back is never refused.
        c.world.stats.level = 20;
        assert!(c.a_critter(&rabbit, &cfg));
        c.autoplay.last_hit_us = Some(Instant::now());
        c.autoplay.hit_by = Some(("Brown Rabbit".into(), Instant::now()));
        assert!(!c.a_critter(&rabbit, &cfg), "it is hitting the character");
        // Which says nothing about the one next to it.
        let other = in_view(&mut c, 0x8000_0004, 2566, "Black Rabbit");
        assert!(c.a_critter(&other, &cfg));
        c.autoplay.last_hit_us = None;
        c.autoplay.hit_by = None;

        // Named outright, it is what the player asked to hunt.
        let only = Fight {
            only: vec!["rabbit".into()],
            ..Fight::default()
        };
        assert!(!c.a_critter(&rabbit, &only));

        // A creature of ours already on it: its fight, and ours to end.
        c.world.objects.insert(
            0x8000_0005,
            ac_world::WorldObject {
                guid: 0x8000_0005,
                name: "Fire Elemental".into(),
                item_type: ac_world::item_type::CREATURE,
                health: Some(1.0),
                pet_owner: 0x5000_0001,
                target: Some(ac_world::object::MoveTarget::Object(rabbit.guid)),
                ..Default::default()
            },
        );
        assert!(!c.a_critter(&rabbit, &cfg));
        assert!(c.a_critter(&other, &cfg), "the one it is not on");
        c.world.objects.remove(&0x8000_0005);

        // With the setting off, everything is fought.
        let all = Fight {
            skip_critters: false,
            ..Fight::default()
        };
        assert!(!c.a_critter(&rabbit, &all));
        assert!(!c.a_critter(&other, &all));
    }

    /// The GameEvent behind "You evade <name>'s attack.": the server's
    /// word that `name` swung at the character and missed.
    fn evaded(guid: u32, name: &str) -> Vec<u8> {
        let mut w = ac_net::wire::Writer::new();
        w.u32(guid)
            .u32(0)
            .u32(ac_net::messages::event::EVASION_DEFENDER_NOTIFICATION)
            .string16(name);
        w.finish()
    }

    #[test]
    fn an_evaded_swing_counts_as_being_attacked() {
        let Some(mut c) = character_of_level(20) else {
            return;
        };
        assert!(!c.under_attack(), "nothing has happened yet");
        c.chat_message(
            ac_net::messages::opcode::GAME_EVENT,
            &evaded(0x8000_0001, "Drudge Skulker"),
        );
        assert!(c.under_attack(), "a miss is an attack all the same");
        assert_eq!(
            c.autoplay.hit_by.as_ref().map(|(who, _)| who.as_str()),
            Some("Drudge Skulker"),
            "and the server named who swung"
        );
        assert!(c.hit_lately_by("Drudge Skulker"));
        assert!(!c.hit_lately_by("Drudge Robber"), "the one standing by");
    }

    #[test]
    fn a_spell_cast_at_the_character_names_its_caster() {
        // The lines ACE sends the target of a spell, and nothing else is
        // sent: SpellProjectile's damage and drain, WorldObject_Magic's
        // drain, transfer and resist.
        for (line, who) in [
            (
                "Drudge Shaman blasts you for 12 points with Flame Bolt I.",
                "Drudge Shaman",
            ),
            (
                "Critical hit! Sneak Attack! Drudge Shaman scorches you for 40 points \
                 with Flame Bolt II.",
                "Drudge Shaman",
            ),
            (
                "Overpower! Mite hits you for 3 points with Acid Stream I. Your \
                 augmentation allows you to avoid a critical hit!",
                "Mite",
            ),
            (
                "Drudge Shaman casts Harm Other I and drains 9 points of your health.",
                "Drudge Shaman",
            ),
            (
                "You lose 20 points of mana due to Drudge Shaman casting Mana to \
                 Health Other I on you",
                "Drudge Shaman",
            ),
            (
                "You resist the spell cast by Drudge Shaman",
                "Drudge Shaman",
            ),
        ] {
            assert_eq!(spell_attacker(line), Some(who), "{line}");
        }
        // The character's own spells, and a fellow's help, are no attack.
        for line in [
            "You blast Drudge Shaman for 12 points with Flame Bolt I.",
            "Drudge Shaman resists your spell",
            "With Harm Other I you drain 9 points of health from Drudge Shaman.",
            "Aldric casts Heal Other I and restores 30 points of your health.",
            "Aldric cast Strength Other I on you",
            "You gain 20 points of health due to Aldric casting Stamina to Health \
             Other I on you",
            "You lose 50 points of stamina due to casting Stamina to Mana Other I \
             on Aldric",
            "Drudge Skulker hits you for 5 points.",
        ] {
            assert_eq!(spell_attacker(line), None, "{line}");
        }
    }

    /// A line of system chat as the server sends it (ServerMessage 0xF7E0):
    /// the text, and its ChatMessageType, Magic here.
    fn magic_line(text: &str) -> Vec<u8> {
        let mut w = ac_net::wire::Writer::new();
        w.string16(text).u32(7);
        w.finish()
    }

    #[test]
    fn a_caster_that_only_casts_is_attacking_the_character() {
        // A shaman turns and casts from its spell range and never closes
        // in to swing, so neither notification a swing brings ever came:
        // `under_attack` stayed false while it worked the character over,
        // and every "but fight back when it attacks you" carve-out walked
        // on past it.
        let Some(mut c) = character_of_level(20) else {
            return;
        };
        let op = ac_net::messages::opcode::SERVER_MESSAGE;
        assert!(!c.under_attack(), "nothing has happened yet");
        c.chat_message(
            op,
            &magic_line("Drudge Shaman blasts you for 12 points with Flame Bolt I."),
        );
        assert!(c.under_attack(), "a bolt that landed is an attack");
        assert!(c.hit_lately_by("Drudge Shaman"));
        assert!(!c.hit_lately_by("Drudge Skulker"), "the one standing by");

        // Resisted, it was still cast at the character.
        c.autoplay.last_hit_us = None;
        c.autoplay.hit_by = None;
        c.chat_message(
            op,
            &magic_line("You resist the spell cast by Drudge Shaman"),
        );
        assert!(c.hit_lately_by("Drudge Shaman"));

        // A fellow's heal is not.
        c.autoplay.last_hit_us = None;
        c.autoplay.hit_by = None;
        c.chat_message(
            op,
            &magic_line("Aldric casts Heal Other I and restores 30 points of your health."),
        );
        assert!(!c.under_attack());
    }

    #[test]
    fn a_critter_that_swings_and_misses_is_fought_rather_than_walked_past() {
        let Some(mut c) = character_of_level(20) else {
            return;
        };
        let cfg = Fight::default();
        let rabbit = in_view(&mut c, 0x8000_0001, 2567, "Brown Rabbit");
        let other = in_view(&mut c, 0x8000_0002, 2566, "Black Rabbit");
        assert!(
            c.a_critter(&rabbit, &cfg),
            "outgrown, and it has done nothing"
        );

        // It swings and misses, over and over: the character never takes
        // a point of damage, so nothing but this says it is in a fight.
        c.chat_message(
            ac_net::messages::opcode::GAME_EVENT,
            &evaded(rabbit.guid, "Brown Rabbit"),
        );
        assert!(
            !c.a_critter(&rabbit, &cfg),
            "it is swinging at the character"
        );
        assert!(
            c.a_critter(&other, &cfg),
            "which says nothing about its neighbour"
        );
    }

    /// An appraisal of `guid` saying it is `level`.
    fn appraised_at(c: &mut Client, guid: u32, level: i32) {
        c.appraisals.insert(
            guid,
            ac_net::messages::Appraisal {
                guid,
                success: true,
                ints: vec![(CREATURE_LEVEL, level)],
                ..Default::default()
            },
        );
    }

    #[test]
    fn a_level_read_off_the_creature_itself_stands_in_for_the_tables() {
        let Some(mut c) = character_of_level(50) else {
            return;
        };
        let cfg = Fight::default();
        // A Portal Pillar (32522) never attacks anything and the table
        // has no level for it. Nothing is walked past on a guess, so it
        // is fought until an appraisal says what it is worth.
        let pillar = in_view(&mut c, 0x8000_0011, 32522, "Portal Pillar");
        assert_eq!(
            ac_world::elements::creature_by_id(32522).and_then(|k| k.level),
            None
        );
        assert!(!c.a_critter(&pillar, &cfg));
        appraised_at(&mut c, pillar.guid, 4);
        assert!(c.a_critter(&pillar, &cfg));

        // A creature whose weenie is not in the table at all is known
        // only by the end of its name, and that row is some other
        // weenie: a Brown Rabbit is level 4, and this is not one.
        let stronger = in_view(&mut c, 0x8000_0012, 0x00FF_FFFF, "Weakened Brown Rabbit");
        assert!(c.a_critter(&stronger, &cfg), "on the name alone, a Rabbit");
        appraised_at(&mut c, stronger.guid, 40);
        assert!(!c.a_critter(&stronger, &cfg), "not at forty it is not");
    }

    /// A weapon in the character's hand, or in its pack.
    fn a_weapon(c: &mut Client, guid: u32, kind: u32, name: &str, in_hand: bool) {
        let me = c.world.player_guid;
        let locations = if kind == ac_world::item_type::CASTER {
            ac_world::equip::HELD
        } else {
            ac_world::equip::MELEE_WEAPON
        };
        c.world.objects.insert(
            guid,
            ac_world::WorldObject {
                guid,
                name: name.into(),
                item_type: kind,
                value: 100,
                valid_locations: locations,
                container: if in_hand { None } else { me },
                wielder: if in_hand { me } else { None },
                ..Default::default()
            },
        );
    }

    /// A character with a mace in hand, a wand in the pack, and a
    /// creature it is swinging at.
    fn mid_fight() -> Option<(Client, u32)> {
        const MACE: u32 = 0x8000_0101;
        const WAND: u32 = 0x8000_0102;
        const CREATURE: u32 = 0x8000_0103;
        let mut c = character_of_level(20)?;
        a_weapon(
            &mut c,
            MACE,
            ac_world::item_type::MELEE_WEAPON,
            "Mace",
            true,
        );
        a_weapon(
            &mut c,
            WAND,
            ac_world::item_type::CASTER,
            "Training Wand",
            false,
        );
        in_view(&mut c, CREATURE, 19257, "Drudge Skulker");
        c.combat = true;
        c.attack_target = Some(CREATURE);
        c.last_attack = Instant::now() - Duration::from_secs(5);
        Some((c, WAND))
    }

    #[test]
    fn the_swing_waits_for_a_swap_the_buff_pass_started() {
        let Some((mut c, wand)) = mid_fight() else {
            return;
        };
        // The in-fight buffing reaches for the wand. The mace goes back
        // in the pack and the wand waits on empty hands.
        assert!(c.wield_for(Stance::Magic), "the swap went out");
        assert_eq!(c.autoplay.pending_wield, Some(wand));
        // Which the fight can now see. Before this, only the arming
        // code stamped the swap clock, so a swap the buff pass or the
        // softening started was invisible and the next swing went out
        // into the empty hands: +Verity put her Flaming Takuba away for
        // a wand and punched a Spikey Armoredillo.
        assert!(c.hands_changing(Instant::now()), "the swap is under way");
        // The hands still say melee -- the server has not answered the
        // put yet -- so nothing but this holds the swing back.
        assert_eq!(c.combat_stance(), Stance::Melee);
        c.tick_combat();
        assert!(!c.attack_pending, "no swing into the empty hands");
        // And the wait is bounded: a swap the server never finishes
        // cannot stop the character fighting.
        c.autoplay.last_rewield = Some(Instant::now() - SWAP_SETTLES);
        c.tick_combat();
        assert!(c.attack_pending, "swinging again once the swap is stale");
    }

    #[test]
    fn the_weapon_comes_back_out_of_the_pack_after_a_buff() {
        let Some((mut c, wand)) = mid_fight() else {
            return;
        };
        const MACE: u32 = 0x8000_0101;
        // The buff pass put the mace down and took the wand up.
        a_weapon(
            &mut c,
            MACE,
            ac_world::item_type::MELEE_WEAPON,
            "Mace",
            false,
        );
        a_weapon(
            &mut c,
            wand,
            ac_world::item_type::CASTER,
            "Training Wand",
            true,
        );
        c.autoplay.put_down = Some(MACE);
        c.autoplay_rearm();
        // ACE will not put a mace in a hand that holds a caster -- it
        // refuses the wield outright, with no error to read -- so the
        // wand goes back in the pack first and the mace waits on empty
        // hands. Asking straight out was refused every single time.
        assert_eq!(c.autoplay.pending_wield, Some(MACE));
        assert_eq!(c.autoplay.wield_asked, None, "nothing was asked for yet");
        assert_eq!(c.autoplay.put_down, None, "the errand passed on");
    }

    #[test]
    fn an_errand_the_server_is_refusing_is_kept_rather_than_dropped() {
        let Some((mut c, _)) = mid_fight() else {
            return;
        };
        const MACE: u32 = 0x8000_0101;
        const SHIELD: u32 = 0x8000_0104;
        let me = c.world.player_guid;
        a_weapon(
            &mut c,
            MACE,
            ac_world::item_type::MELEE_WEAPON,
            "Mace",
            false,
        );
        c.autoplay.put_down = Some(MACE);
        c.hold_off_wield(MACE, Instant::now());
        c.autoplay_rearm();
        // Asking now sends nothing, so clearing the errand would leave
        // the mace in the pack with nothing left to ask again.
        assert_eq!(c.autoplay.put_down, Some(MACE), "still owed the weapon");
        assert_eq!(c.autoplay.pending_wield, None);

        // The shield keeps its errand the same way.
        c.world.objects.insert(
            SHIELD,
            ac_world::WorldObject {
                guid: SHIELD,
                name: "Buckler".into(),
                valid_locations: ac_world::equip::SHIELD,
                container: me,
                ..Default::default()
            },
        );
        c.autoplay.wanted_shield = Some(SHIELD);
        c.hold_off_wield(SHIELD, Instant::now());
        c.autoplay_shield(Instant::now());
        assert_eq!(c.autoplay.wanted_shield, Some(SHIELD), "still owed it");
    }

    #[test]
    fn the_shield_goes_on_between_swings_and_not_during_one() {
        let Some((mut c, _)) = mid_fight() else {
            return;
        };
        const SHIELD: u32 = 0x8000_0104;
        let me = c.world.player_guid;
        c.world.objects.insert(
            SHIELD,
            ac_world::WorldObject {
                guid: SHIELD,
                name: "Buckler".into(),
                valid_locations: ac_world::equip::SHIELD,
                container: me,
                ..Default::default()
            },
        );
        c.autoplay.wanted_shield = Some(SHIELD);
        // A swing is in the air. ACE shuffles the stance on every
        // successful equip, a shield included, and a combat-mode change
        // cancels the attack -- so the shield waits for the gap.
        c.attack_pending = true;
        c.last_attack = Instant::now();
        assert!(c.mid_attack());
        c.autoplay_shield(Instant::now());
        assert_eq!(c.autoplay.wanted_shield, Some(SHIELD), "still to go on");
        assert_eq!(c.autoplay.wield_asked, None, "nothing sent mid-swing");
        assert!(c.wants_the_hands, "the gap after the swing is booked");
    }

    #[test]
    fn a_weapon_choice_put_off_for_a_swing_is_made_once_the_fight_is_joined() {
        let Some((mut c, _)) = mid_fight() else {
            return;
        };
        let cfg = Fight::default();
        // The choice was put off: a swing was in the air when the target
        // was picked, so `arm_for` booked the gap and returned without
        // recording what it had armed for. The attack went out anyway --
        // the target that owned the swing had left the area or been
        // given up on, which clears `attack_target` and leaves
        // `attack_pending` set.
        assert_eq!(c.autoplay.armed_for, None);
        assert!(c.appraise_queue.is_empty());
        // Nothing in the air now, and the fight is joined.
        assert!(!c.mid_attack());
        assert!(c.autoplay_fight_as(Instant::now(), &cfg), "fighting");
        // Only this branch runs from here on: the picker is never
        // reached again while the target is alive. Without it the
        // character fought the whole creature with whatever the buff
        // pass had left in its hands, and a bow with no arrows chosen.
        assert!(
            !c.appraise_queue.is_empty(),
            "the weapons are being weighed for the choice"
        );
    }

    #[test]
    fn a_softening_with_no_wand_to_be_had_lets_the_fight_go_ahead() {
        let Some((mut c, wand)) = mid_fight() else {
            return;
        };
        const CREATURE: u32 = 0x8000_0103;
        // A Drudge Skulker is weakest to cold, fire and electricity
        // alike; whichever it picks, the character knows the
        // vulnerability for it.
        for element in [
            ac_world::elements::Element::Cold,
            ac_world::elements::Element::Fire,
            ac_world::elements::Element::Electric,
        ] {
            for id in ac_world::elements::vulnerabilities(element) {
                c.world.stats.spells.push(id);
            }
        }
        assert_eq!(c.combat_stance(), Stance::Melee);
        assert!(!c.mid_attack());
        // With a wand to be had, the softening takes the tick to reach
        // for one. This is the setup working, and what makes the second
        // half mean anything.
        assert!(
            c.autoplay_soften(CREATURE, "Drudge Skulker", Instant::now()),
            "reaching for the wand"
        );
        assert_eq!(c.autoplay.pending_wield, Some(wand));

        // Now the server is refusing that wand. Nothing can be sent, so
        // the softening gives the tick back rather than holding fire on
        // the target for ever: every expiry of the wait earned one more
        // refusal and doubled the next, up towards four hours.
        let Some((mut c, wand)) = mid_fight() else {
            return;
        };
        for element in [
            ac_world::elements::Element::Cold,
            ac_world::elements::Element::Fire,
            ac_world::elements::Element::Electric,
        ] {
            for id in ac_world::elements::vulnerabilities(element) {
                c.world.stats.spells.push(id);
            }
        }
        c.hold_off_wield(wand, Instant::now());
        assert!(
            !c.autoplay_soften(CREATURE, "Drudge Skulker", Instant::now()),
            "the fight may go ahead unsoftened"
        );
        assert_eq!(c.autoplay.pending_wield, None, "nothing was sent");
    }

    fn item(name: &str, value: u32, armor: u32) -> ItemStats {
        ItemStats {
            name: name.into(),
            value,
            armor_level: armor,
            appraised: true,
            kind: if armor > 0 { "armor" } else { "misc" },
            ..Default::default()
        }
    }

    /// A shelf holding one profile of `rules`, in a directory of its
    /// own so that two tests never read each other's files.
    fn shelf(named: &str, rules: Vec<crate::profile::Rule>) -> crate::profile::Library {
        let dir = std::env::temp_dir().join(format!("acswarm-{named}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let library = crate::profile::Library::default();
        library.open(&dir);
        library
            .put(crate::profile::Profile {
                name: "test".into(),
                rules,
                ..Default::default()
            })
            .expect("saved");
        library
    }

    /// A rule that claims what `line` matches, in the inventory's own
    /// search language.
    fn asks(name: &str, line: &str, action: LootAction) -> crate::profile::Rule {
        crate::profile::Rule {
            name: name.into(),
            action,
            all: vec![crate::profile::Ask::Search(line.into())],
            ..Default::default()
        }
    }

    fn judged(
        stats: &ItemStats,
        library: &crate::profile::Library,
        profile: &str,
    ) -> crate::profile::Verdict {
        judge_loot(
            stats,
            None,
            library.get(profile).as_deref(),
            &crate::weapons::Wielder::default(),
            "Aldric",
            0,
        )
    }

    /// Set the name lists on the test profile. They are the profile's
    /// now, so they change for everyone reading it.
    fn name_lists(library: &crate::profile::Library, always: &[&str], never: &[&str]) {
        let mut p = (*library.get("test").expect("the test profile")).clone();
        p.looting.always = always.iter().map(|s| s.to_string()).collect();
        p.looting.never = never.iter().map(|s| s.to_string()).collect();
        library.put(p).expect("put");
    }

    #[test]
    fn the_players_own_word_comes_before_the_profile() {
        use crate::profile::Verdict;
        let library = shelf(
            "own-word",
            vec![
                asks("keepers", "value>250", LootAction::Keep),
                asks("trash", "rusty", LootAction::Sell),
            ],
        );
        name_lists(&library, &[], &[]);
        let ring = item("Ornate Ring", 900, 0);
        let nail = item("Rusty Nail", 3, 0);
        assert_eq!(
            judged(&ring, &library, "test"),
            Verdict::Decided(LootAction::Keep, "keepers".into())
        );
        assert_eq!(
            judged(&nail, &library, "test"),
            Verdict::Decided(LootAction::Sell, "trash".into())
        );
        // Never wins over a rule that would have kept it.
        name_lists(&library, &[], &["ornate"]);
        assert_eq!(
            judged(&ring, &library, "test"),
            Verdict::Decided(LootAction::Skip, "never take these".into())
        );
        // Always wins over a rule that would have sold it, and loses to
        // never, which is read first.
        name_lists(&library, &["rusty"], &[]);
        assert_eq!(
            judged(&nail, &library, "test"),
            Verdict::Decided(LootAction::Keep, "always take these".into())
        );
        name_lists(&library, &["rusty"], &["rusty"]);
        assert_eq!(
            judged(&nail, &library, "test"),
            Verdict::Decided(LootAction::Skip, "never take these".into())
        );
        let _ = std::fs::remove_dir_all(library.dir());
    }

    /// Set the buy list on the test profile.
    fn buy_list(library: &crate::profile::Library, lines: &[(&str, u32)]) {
        let mut p = (*library.get("test").expect("the test profile")).clone();
        p.buy = lines
            .iter()
            .map(|(what, keep)| crate::profile::Buy {
                what: what.to_string(),
                keep: *keep,
                restock_at: None,
                from: None,
                on: true,
            })
            .collect();
        library.put(p).expect("put");
    }

    #[test]
    fn the_buy_list_keeps_its_line_and_the_rules_answer_for_the_rest() {
        // A line on the buy list is the player's word that this many
        // are stock. Up to the line, a taper is kept whatever the rules
        // make of it -- "sell the rest" once tagged the tapers just
        // bought for the line, and the next trip sold them and bought
        // them again. Over the line, the rules answer, so a surplus
        // under "sell the rest" still goes.
        use crate::profile::Verdict;
        let library = shelf(
            "stock",
            vec![asks("the rest", "value>=0", LootAction::Sell)],
        );
        buy_list(&library, &[("Prismatic Taper", 100)]);
        let profile = library.get("test");
        let me = crate::weapons::Wielder::default();
        let judge = |stats: &ItemStats, held: u32| {
            judge_loot(stats, None, profile.as_deref(), &me, "Aldric", held)
        };
        let taper = item("Prismatic Taper", 500, 0);
        assert_eq!(
            judge(&taper, 0),
            Verdict::Decided(LootAction::Keep, "kept stocked".into())
        );
        assert_eq!(
            judge(&taper, 99),
            Verdict::Decided(LootAction::Keep, "kept stocked".into()),
            "one short of the line: the stack that fills it is kept"
        );
        assert_eq!(
            judge(&taper, 100),
            Verdict::Decided(LootAction::Sell, "the rest".into()),
            "the line is full: the rules answer"
        );
        // What the list does not name is the rules' from the start.
        assert_eq!(
            judge(&item("Lead Scarab", 5, 0), 0),
            Verdict::Decided(LootAction::Sell, "the rest".into())
        );
        // A line switched off says nothing.
        let mut p = (*library.get("test").unwrap()).clone();
        p.buy[0].on = false;
        library.put(p).unwrap();
        let profile = library.get("test");
        assert_eq!(
            judge_loot(&taper, None, profile.as_deref(), &me, "Aldric", 0),
            Verdict::Decided(LootAction::Sell, "the rest".into())
        );
        let _ = std::fs::remove_dir_all(library.dir());
    }

    #[test]
    fn what_arrives_is_judged_against_what_was_held_before_it() {
        // "Keep up to four healing kits", three in the pack, a fourth
        // bought. The arrival pass once counted the arrival itself, so
        // the fourth was the fourth of four, over the cap, and "the
        // rest, to the counter" tagged it to sell: the kit just bought
        // for the line went back over the counter. Held is what was
        // held before it, on this path as on the corpse's, and a fifth
        // is the one over the cap.
        let Some(mut c) = character_of_level(20) else {
            return;
        };
        let dir = std::env::temp_dir().join("acswarm-test-arrival-profiles");
        std::fs::create_dir_all(&dir).ok();
        let shelf = std::sync::Arc::new(crate::profile::Library::default());
        shelf.open(&dir);
        let mut kits = crate::profile::Rule {
            name: "kits".into(),
            action: LootAction::Keep,
            all: vec![crate::profile::Ask::Item(crate::items::Term::Word(
                "healing kit".into(),
            ))],
            ..Default::default()
        };
        kits.keep_up_to = Some(4);
        shelf
            .put(crate::profile::Profile {
                name: "four-kits".into(),
                rules: vec![kits, asks("the rest", "value>=0", LootAction::Sell)],
                ..Default::default()
            })
            .unwrap();
        c.profiles = shelf;
        c.autoplay.config.loot.profile = "four-kits".into();
        let me = c.world.player_guid.unwrap();
        let kit = |guid: u32| ac_world::WorldObject {
            guid,
            name: "Healing Kit".into(),
            weenie_class_id: 4000,
            item_type: ac_world::item_type::MISC,
            value: 100,
            stack_size: 1,
            container: Some(me),
            ..Default::default()
        };
        for guid in [0x8000_0001, 0x8000_0002, 0x8000_0003] {
            c.world.objects.insert(guid, kit(guid));
        }
        let t0 = Instant::now();
        // The first pass only notes what is carried.
        c.autoplay_tag_arrivals(t0);
        c.world.objects.insert(0x8000_0004, kit(0x8000_0004));
        c.autoplay_tag_arrivals(t0);
        assert_eq!(
            c.autoplay.ledger.by_guid(0x8000_0004),
            Some(LootAction::Keep),
            "the fourth of four is under the cap"
        );
        c.world.objects.insert(0x8000_0005, kit(0x8000_0005));
        c.autoplay_tag_arrivals(t0);
        assert_eq!(
            c.autoplay.ledger.by_guid(0x8000_0005),
            Some(LootAction::Sell),
            "the fifth is over it"
        );
    }

    #[test]
    fn supplies_are_handed_over_from_the_stack_that_is_leaving_anyway() {
        // A mate short of tapers: the stack the player said to sell
        // goes first, and the one they said to keep only when it is
        // the only one.
        let Some(mut c) = character_of_level(20) else {
            return;
        };
        let me = c.world.player_guid.unwrap();
        let tapers = |guid: u32, stack: u32| ac_world::WorldObject {
            guid,
            name: "Prismatic Taper".into(),
            weenie_class_id: 20631,
            item_type: ac_world::item_type::SPELL_COMPONENTS,
            value: stack,
            stack_size: stack,
            max_stack_size: 1_000,
            container: Some(me),
            ..Default::default()
        };
        c.world
            .objects
            .insert(0x8000_0001, tapers(0x8000_0001, 1_000));
        let kept = c.stats_of(0x8000_0001).unwrap();
        c.autoplay.tag(&kept, LootAction::Keep);
        assert_eq!(
            c.spare_for("prismatic taper").map(|(g, _)| g),
            Some(0x8000_0001)
        );
        c.world.objects.insert(0x8000_0002, tapers(0x8000_0002, 40));
        assert_eq!(
            c.spare_for("prismatic taper").map(|(g, _)| g),
            Some(0x8000_0002),
            "nothing decided about it beats a keeper"
        );
        c.world.objects.insert(0x8000_0003, tapers(0x8000_0003, 12));
        let to_sell = c.stats_of(0x8000_0003).unwrap();
        c.autoplay.tag(&to_sell, LootAction::Sell);
        assert_eq!(
            c.spare_for("prismatic taper").map(|(g, _)| g),
            Some(0x8000_0003),
            "leaving anyway"
        );
        assert_eq!(c.spare_for("lead scarab"), None);
    }

    #[test]
    fn nothing_is_decided_without_a_profile() {
        use crate::profile::Verdict;
        let library = shelf(
            "no-profile",
            vec![asks("keepers", "value>250", LootAction::Keep)],
        );
        name_lists(&library, &["ornate"], &[]);
        let ring = item("Ornate Ring", 900, 0);
        // A character reading no profile, or one not on the shelf, takes
        // nothing -- the name lists included, since they are the
        // profile's and not the character's.
        assert_eq!(judged(&ring, &library, ""), Verdict::None);
        assert_eq!(judged(&ring, &library, "missing"), Verdict::None);
        assert_eq!(
            judged(&ring, &library, "test"),
            Verdict::Decided(LootAction::Keep, "always take these".into())
        );
        let _ = std::fs::remove_dir_all(library.dir());
    }

    #[test]
    fn what_arrives_in_the_pack_is_judged_like_what_lies_on_a_corpse() {
        // A bundle of arrowheads is a bundle of arrowheads whether it
        // came off a drudge or over a counter.
        let library = shelf(
            "arrival",
            vec![
                asks("keepers", "value>250", LootAction::Keep),
                asks("plate", "type:armor al>=200", LootAction::Sell),
            ],
        );
        name_lists(&library, &[], &[]);
        let me = crate::weapons::Wielder::default();
        let tag =
            |s: &ItemStats| arrival_tag(s, None, library.get("test").as_deref(), &me, "Aldric", 0);
        assert_eq!(tag(&item("Ornate Ring", 900, 0)), Some(LootAction::Keep));
        assert_eq!(tag(&item("Platemail", 100, 240)), Some(LootAction::Sell));
        // Nothing claimed it, so there is nothing to write down...
        assert_eq!(tag(&item("Rusty Nail", 3, 0)), None);
        // ...and neither has a skip, which is a decision to leave it.
        name_lists(&library, &[], &["ornate"]);
        assert_eq!(tag(&item("Ornate Ring", 900, 0)), None);
        // An item that cannot be judged until it is appraised is not
        // written down either: the answer is not in yet.
        name_lists(&library, &[], &[]);
        let unread = ItemStats {
            appraised: false,
            ..item("Platemail", 100, 240)
        };
        assert!(matches!(
            judged(&unread, &library, "test"),
            crate::profile::Verdict::NeedsId(_)
        ));
        assert_eq!(tag(&unread), None);
        assert_eq!(LootAction::parse("Salvage"), Some(LootAction::Salvage));
        assert_eq!(LootAction::parse("burn"), None);
        assert!(!LootAction::Skip.takes());
        let _ = std::fs::remove_dir_all(library.dir());
    }

    #[test]
    fn the_best_salvager_has_an_ust_and_the_highest_skill() {
        let mate = |name: &str, guid: u32, salvaging: u32, has_ust: bool| Mate {
            name: name.into(),
            guid,
            salvaging,
            has_ust,
            ..Default::default()
        };
        let team = [
            mate("Zed", 1, 300, true),
            mate("Amy", 2, 300, true),
            mate("Bob", 3, 400, false),
            mate("Cal", 4, 200, true),
        ];
        // Bob's skill is highest but he has no Ust; Amy and Zed tie and
        // the name that sorts first wins.
        assert_eq!(best_salvager(team.iter()), Some(("Amy".into(), 2)));
        assert_eq!(best_salvager(team[3..].iter()), Some(("Cal".into(), 4)));
        assert_eq!(best_salvager(team[2..3].iter()), None);
        assert_eq!(best_salvager(std::iter::empty()), None);
        // Someone not yet in the world (guid 0) cannot be handed anything.
        assert_eq!(best_salvager([mate("Nobody", 0, 999, true)].iter()), None);
    }

    /// `items` (guid and workmanship), none of them refused yet.
    fn never_refused(items: &[(u32, f32)]) -> Vec<(u32, f32, u8)> {
        items.iter().map(|(g, w)| (*g, *w, 0)).collect()
    }

    /// Every salvage sent for `items` (guid and workmanship), the server
    /// taking each batch before the next is chosen.
    fn salvages(items: &[(u32, f32)]) -> Vec<Vec<u32>> {
        let mut left = items.to_vec();
        let mut sent = Vec::new();
        while let Some((_, batch)) = next_salvage_batch(never_refused(&left)) {
            left.retain(|(g, _)| !batch.contains(g));
            sent.push(batch);
        }
        sent
    }

    #[test]
    fn a_salvage_that_came_to_nothing_waits_behind_the_grades_not_yet_tried() {
        // ACE skips a Retained item without a word. Chosen as the best
        // grade every time, a 10 like that went out alone after each
        // timeout, and everything below it waited behind all three.
        let (ten, nine, six, five) = (1, 2, 3, 4);
        assert_eq!(
            next_salvage_batch([(ten, 10.0, 1), (nine, 9.0, 0), (six, 6.0, 0)]),
            Some((SalvageGrade::Nine, vec![nine]))
        );
        assert_eq!(
            next_salvage_batch([(ten, 10.0, 1), (six, 6.0, 0)]),
            Some((SalvageGrade::Common, vec![six]))
        );
        // Once the rest are gone it is asked for again, still on its own.
        assert_eq!(
            next_salvage_batch([(ten, 10.0, 1)]),
            Some((SalvageGrade::Ten, vec![ten]))
        );
        // Refused alike, the grades still keep apart.
        assert_eq!(
            next_salvage_batch([(six, 6.0, 1), (ten, 10.0, 1), (five, 5.0, 1)]),
            Some((SalvageGrade::Ten, vec![ten]))
        );
        // And the one refused least goes first.
        assert_eq!(
            next_salvage_batch([(six, 6.0, 2), (five, 5.0, 1)]),
            Some((SalvageGrade::Common, vec![five]))
        );
    }

    #[test]
    fn a_workmanship_10_iron_mace_is_not_salvaged_with_a_6() {
        // In one salvage both go into the same bag of Iron, and the bag
        // comes out a workmanship 8.
        let (six, ten) = (0x8000_0001, 0x8000_0002);
        assert_eq!(
            salvages(&[(six, 6.0), (ten, 10.0)]),
            vec![vec![ten], vec![six]]
        );
    }

    #[test]
    fn nines_and_tens_never_share_a_salvage() {
        let items = [(1, 9.0), (2, 10.0), (3, 6.0), (4, 9.0), (5, 10.0), (6, 3.0)];
        assert_eq!(
            next_salvage_batch(never_refused(&items)),
            Some((SalvageGrade::Ten, vec![2, 5]))
        );
        // The best first, each grade alone, in the order they were given.
        assert_eq!(salvages(&items), vec![vec![2, 5], vec![1, 4], vec![3, 6]]);
    }

    #[test]
    fn everything_below_nine_goes_in_one_salvage() {
        let items = [(1, 1.0), (2, 8.0), (3, 5.0), (4, 8.0)];
        assert_eq!(
            next_salvage_batch(never_refused(&items)),
            Some((SalvageGrade::Common, vec![1, 2, 3, 4]))
        );
        assert_eq!(salvages(&items), vec![vec![1, 2, 3, 4]]);
    }

    #[test]
    fn a_grade_with_nothing_in_it_sends_no_salvage() {
        assert_eq!(next_salvage_batch([]), None);
        assert_eq!(salvages(&[]), Vec::<Vec<u32>>::new());
        // No 9s: the 10 and the rest, and no empty salvage between.
        assert_eq!(salvages(&[(1, 6.0), (2, 10.0)]), vec![vec![2], vec![1]]);
        // Only 9s: one salvage.
        assert_eq!(salvages(&[(1, 9.0), (2, 9.0)]), vec![vec![1, 2]]);
    }

    /// A level 20 character that salvages for itself: an Ust in the pack,
    /// and a loot profile of its own called `profile`.
    fn a_salvager(profile: &str) -> Option<Client> {
        let mut c = character_of_level(20)?;
        a_loot_profile(&mut c, profile);
        let ust = 0x8000_0100;
        c.world.objects.insert(
            ust,
            ac_world::WorldObject {
                guid: ust,
                weenie_class_id: ac_world::material::UST_WCID,
                name: "Ust".into(),
                container: c.world.player_guid,
                ..Default::default()
            },
        );
        Some(c)
    }

    /// An Iron mace of `workmanship` in `container`, tagged for salvage.
    fn a_mace_to_salvage(c: &mut Client, guid: u32, workmanship: f32, container: Option<u32>) {
        c.world.objects.insert(
            guid,
            ac_world::WorldObject {
                guid,
                name: "Iron Mace".into(),
                material: 0x3D,
                workmanship,
                container,
                ..Default::default()
            },
        );
        let stats = c.stats_of(guid).expect("carried");
        c.autoplay.tag(&stats, LootAction::Salvage);
    }

    /// The salvage the salvager is waiting on.
    fn salvage_on_its_way(c: &Client) -> Option<Vec<u32>> {
        c.autoplay.salvaging.as_ref().map(|(g, _)| g.clone())
    }

    #[test]
    fn what_the_team_hands_the_salvager_is_salvaged_a_grade_at_a_time() {
        // Teammates hand salvage over one item at a time, and the
        // salvager salvages it along with its own: that is where a 10
        // one teammate carried would meet another's 6.
        let Some(mut c) = a_salvager("salvage test") else {
            return;
        };
        let me = c.world.player_guid;
        // The 9 is in a side pack: the server finds it there, and so
        // must the salvage.
        let pack = 0x8000_0110;
        c.world.objects.insert(
            pack,
            ac_world::WorldObject {
                guid: pack,
                name: "Pack".into(),
                container: me,
                ..Default::default()
            },
        );
        let (ten, six, nine) = (0x8000_0101, 0x8000_0102, 0x8000_0103);
        for (guid, workmanship, container) in
            [(ten, 10.0, me), (six, 6.0, me), (nine, 9.0, Some(pack))]
        {
            a_mace_to_salvage(&mut c, guid, workmanship, container);
        }
        let mut now = Instant::now();
        assert!(c.autoplay_salvage(now), "salvaging");
        assert_eq!(salvage_on_its_way(&c), Some(vec![ten]));
        for (done, next) in [(ten, nine), (nine, six)] {
            now += Duration::from_millis(100);
            assert!(c.autoplay_salvage(now), "not waited on");
            assert_eq!(salvage_on_its_way(&c), Some(vec![done]));
            // The server takes it, and the next grade goes at once. Sitting
            // out the gap gave the tick to the fight, and the grades still
            // to come waited a whole fight for their turn.
            c.world.objects.remove(&done);
            now += Duration::from_millis(100);
            assert!(c.autoplay_salvage(now), "the next grade waited");
            assert_eq!(salvage_on_its_way(&c), Some(vec![next]));
        }
        c.world.objects.remove(&six);
        now += Duration::from_millis(100);
        assert!(!c.autoplay_salvage(now), "nothing left to salvage");
        assert_eq!(salvage_on_its_way(&c), None);
    }

    #[test]
    fn a_ten_the_server_skips_holds_up_none_of_the_grades_below_it() {
        // ACE skips a Retained item without a word, and its salvage times
        // out. Chosen again as the best grade, the 10 went out alone after
        // every timeout, and the 9 and the 6 waited behind all three.
        let Some(mut c) = a_salvager("salvage skipped") else {
            return;
        };
        let me = c.world.player_guid;
        let (ten, nine, six) = (0x8000_0121, 0x8000_0122, 0x8000_0123);
        for (guid, workmanship) in [(ten, 10.0), (nine, 9.0), (six, 6.0)] {
            a_mace_to_salvage(&mut c, guid, workmanship, me);
        }
        let mut now = Instant::now();
        assert!(c.autoplay_salvage(now));
        assert_eq!(salvage_on_its_way(&c), Some(vec![ten]));
        // Nothing comes of it.
        now += SALVAGE_TIMEOUT;
        assert!(c.autoplay_salvage(now));
        assert_eq!(
            salvage_on_its_way(&c),
            Some(vec![nine]),
            "the 10 went again before the grades not yet tried"
        );
        for (done, next) in [(nine, six), (six, ten)] {
            c.world.objects.remove(&done);
            now += Duration::from_millis(100);
            assert!(c.autoplay_salvage(now));
            assert_eq!(salvage_on_its_way(&c), Some(vec![next]));
        }
        // Still on its own, until the third try sets it aside.
        now += SALVAGE_TIMEOUT;
        assert!(c.autoplay_salvage(now));
        assert_eq!(salvage_on_its_way(&c), Some(vec![ten]));
        now += SALVAGE_TIMEOUT;
        assert!(!c.autoplay_salvage(now), "tried {SALVAGE_TRIES} times");
        assert_eq!(salvage_on_its_way(&c), None);
    }

    #[test]
    fn tidying_the_pack_never_pours_one_salvage_bag_into_another() {
        // Two bags of Iron share a wcid, but only an Ust puts bags
        // together. The server sends a bag with no stack size, which
        // leaves it at 1, and both pours -- the tidy chore and the one
        // at a vendor's counter -- read that to decide what stacks.
        let Some(mut c) = character_of_level(20) else {
            return;
        };
        let me = c.world.player_guid;
        let bag = |guid: u32, workmanship: f32| ac_world::WorldObject {
            guid,
            weenie_class_id: 20986,
            name: "Salvaged Iron".into(),
            item_type: ac_world::item_type::TINKERING_MATERIAL,
            material: 0x3D,
            workmanship,
            structure: 50,
            max_structure: 100,
            stack_size: 1,
            max_stack_size: 1,
            container: me,
            ..Default::default()
        };
        let (tens, sixes) = (0x8000_0201, 0x8000_0202);
        c.world.objects.insert(tens, bag(tens, 10.0));
        c.world.objects.insert(sixes, bag(sixes, 6.0));
        // A pair that does pour, so the bags are not passed over only
        // because nothing is looked at.
        let arrows = |guid: u32, count: u32| ac_world::WorldObject {
            guid,
            weenie_class_id: 300,
            name: "Arrow".into(),
            stack_size: count,
            max_stack_size: 250,
            container: me,
            ..Default::default()
        };
        let (few, many) = (0x8000_0203, 0x8000_0204);
        c.world.objects.insert(few, arrows(few, 5));
        c.world.objects.insert(many, arrows(many, 40));
        let m = crate::pack::next_merge(&c.pack_stacks()).expect("the arrows pour");
        assert_eq!((m.from, m.to), (few, many));
        c.world.objects.remove(&few);
        assert_eq!(crate::pack::next_merge(&c.pack_stacks()), None);
        let counter = c.vendor_snapshot(&crate::growth::Growth::default());
        let bags: Vec<_> = counter
            .items
            .iter()
            .filter(|i| i.guid == tens || i.guid == sixes)
            .collect();
        assert_eq!(bags.len(), 2);
        assert!(bags.iter().all(|i| i.max_stack <= 1), "{bags:?}");
    }

    #[test]
    fn a_body_where_a_kill_fell_is_ours_however_far() {
        let now = Instant::now();
        let spots = vec![(glam::Vec3::new(100.0, 100.0, 50.0), now)];
        // Where it fell, give or take the drift of dying.
        assert!(near_a_kill(glam::Vec3::new(103.0, 98.0, 51.0), &spots));
        // Somebody else's, a street away.
        assert!(!near_a_kill(glam::Vec3::new(120.0, 100.0, 50.0), &spots));
        assert!(!near_a_kill(glam::Vec3::new(100.0, 100.0, 50.0), &[]));
    }

    #[test]
    fn a_resist_or_an_evasion_got_there() {
        assert_eq!(
            arrived_unharmed("Drudge Skulker resists your spell"),
            Some("Drudge Skulker")
        );
        assert_eq!(
            arrived_unharmed("Mite Scion evades your attack."),
            Some("Mite Scion")
        );
        // Ours, not theirs.
        assert_eq!(
            arrived_unharmed("You resist the spell cast by Drudge Skulker"),
            None
        );
        let now = Instant::now();
        let long_ago = now.checked_sub(CLOSE_IN_AFTER * 2).unwrap();
        // Thrown a while ago, and nothing has got there since.
        assert!(nothing_arrived(Some(long_ago), now));
        // Only just thrown: still on its way.
        assert!(!nothing_arrived(Some(now), now));
        // Nothing thrown yet -- still walking to a clear shot -- is no miss.
        assert!(!nothing_arrived(None, now));
    }

    #[test]
    fn walking_up_to_a_target_is_working_on_it_while_it_gets_nearer() {
        // The first stretch of the walk, and a new target, both count.
        assert!(came_nearer(None, 7, 40.0));
        assert!(came_nearer(Some((8, 5.0)), 7, 40.0));
        // Nearer by a pace and more: still closing.
        assert!(came_nearer(Some((7, 40.0)), 7, 38.5));
        // Shuffling on the spot, or backing off round a wall, is not.
        assert!(!came_nearer(Some((7, 40.0)), 7, 39.5));
        assert!(!came_nearer(Some((7, 40.0)), 7, 45.0));
    }

    #[test]
    fn the_fight_waits_only_on_bodies_the_looting_takes() {
        let now = Instant::now();
        let me = glam::Vec3::ZERO;
        let off = glam::Vec3::new(22.0, 0.0, 0.0);
        // Close by: looted, and waited on.
        assert!(corpse_is_ours(
            me,
            glam::Vec3::new(5.0, 0.0, 0.0),
            40.0,
            &[]
        ));
        // Twenty-two metres off where nothing of ours fell: neither. It
        // used to be waited on out to twenty-five and looted only to
        // twenty, and the character stood between the two for good.
        assert!(!corpse_is_ours(me, off, 40.0, &[]));
        // Where a kill of ours fell: both.
        assert!(corpse_is_ours(me, off, 40.0, &[(off, now)]));
        // But not past the fight radius.
        assert!(!corpse_is_ours(me, off, 20.0, &[(off, now)]));

        // Thirty metres off, where the summoned creature landed the last
        // blow: no kill notice came, so no kill spot, and it is not ours
        // as it lies. It is asked about instead.
        let pets = glam::Vec3::new(30.0, 0.0, 0.0);
        assert!(!corpse_is_ours(me, pets, 60.0, &[]));
        assert!(whose_to_ask(me, pets, 60.0, &[]));
        // Its description names the creature, so it is claimed where it
        // lies: looted, and waited on, and not asked about again.
        assert!(killed_by_us(
            "Killed by Blargerton's Mud Golem.",
            "Blargerton"
        ));
        let claimed = [(pets, now)];
        assert!(corpse_is_ours(me, pets, 60.0, &claimed));
        assert!(!whose_to_ask(me, pets, 60.0, &claimed));
        // One close by needs no asking, and one past the fight radius is
        // not asked about.
        assert!(!whose_to_ask(me, glam::Vec3::new(5.0, 0.0, 0.0), 60.0, &[]));
        assert!(!whose_to_ask(
            me,
            glam::Vec3::new(70.0, 0.0, 0.0),
            60.0,
            &[]
        ));
    }

    #[test]
    fn the_moment_held_open_for_a_falling_body_ends_when_one_lands() {
        // The next fight is held for three seconds after a killing blow,
        // because the body comes a moment after the creature dies. Held
        // for the whole three regardless of what landed, nine characters
        // killing in one huddle each spent it standing over a body one
        // of the others was already opening: fighting was 13% of that
        // run and opening a corpse 53%.
        let Some(mut c) = character_of_level(20) else {
            return;
        };
        let s = Duration::from_secs;
        let t0 = Instant::now();
        let spot = glam::Vec3::new(40.0, 40.0, 10.0);
        let body = |guid: u32, at: glam::Vec3| ac_world::WorldObject {
            guid,
            name: "Corpse of Drudge Slave".into(),
            object_desc_flags: ac_world::object_desc_flags::CORPSE,
            position: Some(ac_world::object::Position::new_flat(0, at)),
            ..Default::default()
        };
        assert!(!c.owes_a_corpse(), "owed a body with no kill behind it");

        // Its own killing blow: the body is owed while it is still
        // falling, and no longer.
        c.autoplay.last_kill = Some(t0);
        c.autoplay.kill_spots = vec![(spot, t0)];
        assert!(c.own_body_still_falling(t0));
        assert!(c.own_body_still_falling(t0 + CORPSE_APPEARS - s(1)));
        assert!(!c.own_body_still_falling(t0 + CORPSE_APPEARS));

        // A body noted before the blow is some other kill's and says
        // nothing about this one, wherever it lies.
        let (early, elsewhere, ours) = (0x8000_0001, 0x8000_0002, 0x8000_0003);
        c.world.objects.insert(early, body(early, spot));
        c.autoplay.corpse_seen = vec![(early, t0 - s(1))];
        assert!(c.own_body_still_falling(t0));

        // Nor does a body that landed since the blow somewhere this
        // character killed nothing. Ending the hold on any corpse at
        // all, a huddle of nine had one come into view every couple of
        // seconds, and each one sent a character off after the next
        // fight leaving the body it had just made behind.
        let away = spot + glam::Vec3::new(KILL_SPOT * 2.0, 0.0, 0.0);
        c.world.objects.insert(elsewhere, body(elsewhere, away));
        c.autoplay.corpse_seen.push((elsewhere, t0));
        assert!(
            c.own_body_still_falling(t0),
            "let go of its own body for somebody else's"
        );

        // Its own, landing where its kill fell, ends the wait: from here
        // the bodies on the floor are the answer, a mate's claim among
        // them.
        c.world.objects.insert(ours, body(ours, spot));
        c.autoplay.corpse_seen.push((ours, t0));
        assert!(!c.own_body_still_falling(t0));
        assert!(!c.owes_a_corpse());
    }

    #[test]
    fn a_corpse_is_ours_when_it_names_this_character_or_its_creature_as_the_killer() {
        assert!(killed_by_us("Killed by Blargerton.", "Blargerton"));
        assert!(killed_by_us(
            "Killed by Blargerton's Mud Golem.",
            "Blargerton"
        ));
        // The server leaves off a leading '+'; a line that kept it still
        // counts.
        assert!(killed_by_us("Killed by Fletch.", "+Fletch"));
        assert!(killed_by_us("Killed by Fletch's Mud Golem.", "+Fletch"));
        assert!(killed_by_us("Killed by +Fletch.", "+Fletch"));
        // A rare found on the body adds to the line.
        assert!(killed_by_us(
            "Killed by Blargerton. This corpse generated a rare item!",
            "Blargerton"
        ));
        // A name that only starts like ours is someone else.
        assert!(!killed_by_us("Killed by Blargertonia.", "Blargerton"));
        // Another player, or another player's creature, is not us.
        assert!(!killed_by_us("Killed by Grimble.", "Blargerton"));
        assert!(!killed_by_us(
            "Killed by Grimble's Mud Golem.",
            "Blargerton"
        ));
        assert!(!killed_by_us("Killed by misadventure.", "Blargerton"));
        // Nothing to go on is not ours.
        assert!(!killed_by_us("", "Blargerton"));
        assert!(!killed_by_us("Killed by .", ""));
    }

    #[test]
    fn a_far_corpse_is_asked_about_once_and_the_fight_waits_on_the_answer_briefly() {
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let mut whose = Whose::default();
        // No creature seen out: nothing left bodies about.
        assert!(!whose.pet_lately(t0));
        whose.pet_out(t0);
        assert!(whose.pet_lately(t0 + s(30)));
        assert!(!whose.pet_lately(t0 + PET_KILLS_FOR));

        // Asked about: the fight waits a moment for the answer.
        assert!(!whose.waiting(t0));
        whose.ask(0x8000_0001, t0);
        assert!(whose.has_asked(0x8000_0001));
        assert!(whose.waiting(t0 + s(1)));
        // Asking again changes nothing, and does not restart the wait.
        whose.ask(0x8000_0001, t0 + s(2));
        assert_eq!(whose.out().collect::<Vec<_>>(), vec![0x8000_0001]);
        // An answer that never comes does not hold the fight for good.
        assert!(!whose.waiting(t0 + WHOSE_WAIT));

        // Once the answer is in, nothing is waited on or out, and the
        // corpse is still not asked about again.
        whose.ask(0x8000_0002, t0 + s(4));
        assert!(whose.waiting(t0 + s(5)));
        whose.answered(0x8000_0002);
        whose.answered(0x8000_0001);
        assert!(!whose.waiting(t0 + s(5)));
        assert_eq!(whose.out().count(), 0);
        assert!(whose.has_asked(0x8000_0002));

        // A corpse gone from view is forgotten.
        whose.tidy(|g| g == 0x8000_0002);
        assert!(!whose.has_asked(0x8000_0001));
        assert!(whose.has_asked(0x8000_0002));
    }

    #[test]
    fn a_corpse_a_floor_below_is_not_close_however_near_it_lies_on_the_map() {
        // The Holtburg Dungeon's rooms are stacked: a body three metres
        // off on the map and eight metres down is in another room.
        let now = Instant::now();
        let me = glam::Vec3::new(0.0, 0.0, 8.0);
        let below = glam::Vec3::new(3.0, 0.0, 0.0);
        assert!(!corpse_is_ours(me, below, 60.0, &[]));
        // Nor one a floor above.
        assert!(!corpse_is_ours(below, me, 60.0, &[]));
        // On this floor it is, and a step or a doorsill up is still this
        // floor.
        assert!(corpse_is_ours(
            me,
            glam::Vec3::new(3.0, 0.0, 8.0),
            60.0,
            &[]
        ));
        assert!(corpse_is_ours(
            me,
            glam::Vec3::new(3.0, 0.0, 9.5),
            60.0,
            &[]
        ));
        // Where a kill of its own fell it is still its own: whether it
        // can be walked to is for the walk to find out.
        assert!(corpse_is_ours(me, below, 60.0, &[(below, now)]));
    }

    /// One of the others, standing at `at`, with `looting` in hand for
    /// `held`.
    fn looter(guid: u32, at: glam::Vec3, looting: Option<u32>, held: Duration) -> Mate {
        Mate {
            name: format!("Bryn{guid:02}"),
            guid,
            world: at,
            health: 1.0,
            autoplay: true,
            looting,
            looting_for: held,
            opens_bodies: true,
            ..Default::default()
        }
    }

    #[test]
    fn a_body_one_of_the_others_has_is_left_to_them() {
        // Nine characters in one huddle rank the bodies by the same
        // rule and so choose the same one: 1,386 opens for 41 bodies in
        // one run, because the server hands a container to one viewer
        // and refuses everyone else. Whoever has a body says so, and
        // the rest take another.
        let t0 = Instant::now();
        let (a, b) = (0x8000_0001, 0x8000_0002);
        let (here, there) = (glam::Vec3::ZERO, glam::Vec3::new(4.0, 0.0, 0.0));
        let me = 0x5000_0001;
        // Both fell a while back, so the tie-break has had its moment
        // and the claims are what speak.
        let mut ap = Autoplay {
            corpse_seen: vec![(a, t0), (b, t0)],
            ..Default::default()
        };
        let now = t0 + CLAIM_SETTLE;

        // Alone on the board, nothing changes: both are ours.
        assert!(ap.ours_to_open(a, here, me, here, now));
        assert!(ap.ours_to_open(b, there, me, here, now));

        // One mate is at the first body, another at nothing.
        ap.team.mates = vec![
            looter(2, here, Some(a), Duration::ZERO),
            looter(3, here, None, Duration::ZERO),
        ];
        assert!(
            !ap.ours_to_open(a, here, me, here, now),
            "raced a mate for a body"
        );
        assert!(
            ap.ours_to_open(b, there, me, here, now),
            "left a free body lying"
        );

        // A claim goes stale. A mate that stalled or was dragged into a
        // fight over a body must not hold it for the five minutes it
        // lies there.
        ap.team.mates = vec![looter(2, here, Some(a), CLAIM_STALE)];
        assert!(
            ap.ours_to_open(a, here, me, here, now),
            "a stale claim still held"
        );
        ap.team.mates = vec![looter(
            2,
            here,
            Some(a),
            CLAIM_STALE - Duration::from_millis(1),
        )];
        assert!(!ap.ours_to_open(a, here, me, here, now));

        // A mate that died over the body it had open goes on saying so:
        // its client is still running. The same board row says its
        // health is nothing, and that is read rather than waited out.
        ap.team.mates = vec![Mate {
            health: 0.0,
            ..looter(2, here, Some(a), Duration::ZERO)
        }];
        assert!(
            ap.ours_to_open(a, here, me, here, now),
            "a dead mate held a body for the rest of the claim's life"
        );
    }

    #[test]
    fn a_body_this_character_has_already_claimed_is_not_given_up_for_a_mates() {
        // Two characters that choose one body in the same tick each
        // hear the other's claim half a second later. Asking only
        // "does somebody claim it" made both stand off, each for the
        // other, and the walk each had started was never let go of --
        // so the claim kept going out, the body was opened by nobody,
        // and eight characters stood over it until every claim aged out
        // at once. A standing claim wins the tie instead of losing it.
        let t0 = Instant::now();
        let body = 0x8000_0001;
        let at = glam::Vec3::ZERO;
        let (me, lower, higher) = (0x5000_0005, 0x5000_0001, 0x5000_0009);
        let mut ap = Autoplay {
            corpse_seen: vec![(body, t0)],
            ..Default::default()
        };
        ap.take_up_corpse(body, t0, LOOT_TIMEOUT);
        let now = t0 + CLAIM_SETTLE;

        // A mate that reached for it in the same moment and has the
        // higher guid gives way to us.
        ap.team.mates = vec![looter(higher, at, Some(body), CLAIM_SETTLE)];
        assert!(
            ap.ours_to_open(body, at, me, at, now),
            "gave up a body we were already holding"
        );

        // The lower guid in the same moment has it, and we let go.
        ap.team.mates = vec![looter(lower, at, Some(body), CLAIM_SETTLE)];
        assert!(!ap.ours_to_open(body, at, me, at, now));

        // Plainly older than ours, whatever the guid: theirs.
        ap.team.mates = vec![looter(
            higher,
            at,
            Some(body),
            CLAIM_SETTLE + SAME_MOMENT * 2,
        )];
        assert!(!ap.ours_to_open(body, at, me, at, now));

        // Plainly younger than ours, whatever the guid: ours.
        let later = t0 + SAME_MOMENT * 3;
        ap.team.mates = vec![looter(lower, at, Some(body), Duration::ZERO)];
        assert!(ap.ours_to_open(body, at, me, at, later));
    }

    #[test]
    fn a_walk_to_a_body_this_character_has_given_up_on_is_let_go_of() {
        // The one way out of the looting that did not stop the walk. A
        // character that chose a body, set off for it, and then heard a
        // better claim on it walked on -- and the walk is itself a claim
        // (see `Autoplay::corpse_claim`), so it went on telling the
        // others the body was taken, and went on holding the looting's
        // own worth up, for a body it would never open.
        let holtburg = 0xA9B4_0019;
        let at = glam::Vec3::new(84.0, 84.0, 10.0);
        let Some(mut c) = standing_in_the_field(20, holtburg, at) else {
            return;
        };
        a_loot_profile(&mut c, "walk test");
        let me = c.player.as_ref().unwrap().world_position();
        let now = Instant::now();
        let body = 0x8000_0001;
        c.world.objects.insert(
            body,
            ac_world::WorldObject {
                guid: body,
                name: "Corpse of Drudge Slave".into(),
                object_desc_flags: ac_world::object_desc_flags::CORPSE,
                position: Some(ac_world::object::Position::new_flat(
                    holtburg,
                    me + glam::Vec3::new(7.0, 0.0, 0.0) - ac_world::landblock_origin(holtburg),
                )),
                ..Default::default()
            },
        );
        c.autoplay.corpse_seen = vec![(body, now)];
        // Walking to it, seven metres off, and a mate says it claimed
        // the same body well before we did.
        c.autoplay.walking_to = Some(CorpseWalk::new(body, 7.0, now));
        c.autoplay.team.mates = vec![looter(
            0x5000_0002,
            me,
            Some(body),
            SAME_MOMENT + Duration::from_secs(5),
        )];

        assert!(!c.autoplay_loot(now), "went to a body that is not ours");
        assert_eq!(c.autoplay.walking_to.map(|w| w.guid), None, "still walking");
        assert_eq!(c.autoplay.corpse_claim(now), None, "still claiming it");
    }

    /// A character as the turns read it: standing at `at`, free, with
    /// room, having opened `opened` bodies first lately.
    fn turn_at(guid: u32, at: glam::Vec3, opened: u16) -> Turn {
        Turn {
            guid,
            world: at,
            looting: false,
            fighting: false,
            room: true,
            opened,
        }
    }

    #[test]
    fn every_session_deals_a_newly_fallen_body_to_the_same_character() {
        // A claim is said every half second and a body is chosen within
        // a tick of falling, so for that first moment there is no claim
        // to read and the nine have to agree without one. Every session
        // works the same answer out of the same roster, and the rest
        // stand off.
        let t0 = Instant::now();
        let body = 0x8000_0001;
        let at = glam::Vec3::ZERO;
        let fleet: Vec<u32> = (0..9).map(|i| 0x5000_0010 + i).collect();
        let others = |me: u32| -> Vec<Mate> {
            fleet
                .iter()
                .filter(|g| **g != me)
                .map(|g| looter(*g, at, None, Duration::ZERO))
                .collect()
        };
        // Not the lowest guid, which used to open every body it stood
        // over: the deal for this body.
        let first = 0x5000_0012;
        for &me in &fleet {
            let ap = Autoplay {
                corpse_seen: vec![(body, t0)],
                team: view_of(others(me)),
                ..Default::default()
            };
            assert_eq!(
                ap.team.opens_first(body, at, turn_at(me, at, 0)),
                first,
                "disagreed about whose turn"
            );
            assert_eq!(
                ap.ours_to_open(body, at, me, at, t0),
                me == first,
                "{me:#010x} did not stand off"
            );
            // Once the board has had time to catch up the claims are
            // the truth, and whoever has not been told otherwise goes
            // ahead.
            assert!(ap.ours_to_open(body, at, me, at, t0 + CLAIM_SETTLE));
        }

        // Only the ones free to open it count. A mate played by hand, a
        // dead one, one not in the world, one already at another body,
        // one fighting, one with a full pack, one laden and one across
        // the field all leave it to us, whatever the deal.
        let mine = fleet[8];
        let field_away = glam::Vec3::new(LOOT_NEAR + 5.0, 0.0, 0.0);
        let packed = |pack_full, laden| crate::logistics::Supplies {
            pack_full,
            laden,
            ..Default::default()
        };
        let view = view_of(vec![
            Mate {
                autoplay: false,
                ..looter(fleet[0], at, None, Duration::ZERO)
            },
            Mate {
                health: 0.0,
                ..looter(fleet[1], at, None, Duration::ZERO)
            },
            looter(0, at, None, Duration::ZERO),
            looter(fleet[2], at, Some(0x8000_0002), Duration::ZERO),
            Mate {
                target: Some(0x7000_0001),
                ..looter(fleet[3], at, None, Duration::ZERO)
            },
            Mate {
                supplies: packed(true, false),
                ..looter(fleet[4], at, None, Duration::ZERO)
            },
            Mate {
                supplies: packed(false, true),
                ..looter(fleet[5], at, None, Duration::ZERO)
            },
            looter(fleet[6], field_away, None, Duration::ZERO),
        ]);
        assert_eq!(view.opens_first(body, at, turn_at(mine, at, 0)), mine);

        // And this character is judged by the same tests as the rest. A
        // body is owed to a caster out to the fight radius where one of
        // its kills fell, which is further than a mate has to stand to
        // count: with no reach test on ourselves, the caster called the
        // body its own while every mate's roster ruled the caster out,
        // and the two of them opened it in the same second.
        let over_it = view_of(vec![looter(fleet[0], at, None, Duration::ZERO)]);
        assert_eq!(
            over_it.opens_first(body, at, turn_at(mine, field_away, 0)),
            fleet[0],
            "claimed a body from across the field"
        );
        // Nor is this one dealt a turn while it is at another body, or
        // with no room for what is on this one.
        for busy in [
            Turn {
                looting: true,
                ..turn_at(mine, at, 0)
            },
            Turn {
                room: false,
                ..turn_at(mine, at, 0)
            },
        ] {
            assert_eq!(over_it.opens_first(body, at, busy), fleet[0]);
        }
        // Nobody within reach at all: it is ours to walk to.
        assert_eq!(
            view_of(vec![looter(fleet[0], field_away, None, Duration::ZERO)]).opens_first(
                body,
                at,
                turn_at(mine, field_away, 0)
            ),
            mine,
            "left a body nobody was near"
        );
    }

    #[test]
    fn the_turns_go_round_body_by_body() {
        // Three characters over one spot, all free. Whoever opens a body
        // first says so on its row, and the next body goes to one that
        // has had fewer turns.
        let at = glam::Vec3::ZERO;
        let party = [0x5000_0021u32, 0x5000_0022, 0x5000_0023];
        let mut opened = [0u16; 3];
        let mut who_opened = Vec::new();
        for n in 0..6u32 {
            let body = 0x8000_0100 + n;
            // Each session works it out for itself, from what the others
            // said about their turns.
            let dealt: Vec<u32> = (0..3)
                .map(|i| {
                    let mates = (0..3)
                        .filter(|j| *j != i)
                        .map(|j| Mate {
                            opened_first: opened[j],
                            ..looter(party[j], at, None, Duration::ZERO)
                        })
                        .collect();
                    view_of(mates).opens_first(body, at, turn_at(party[i], at, opened[i]))
                })
                .collect();
            assert!(dealt.iter().all(|g| *g == dealt[0]), "{n}: {dealt:x?}");
            let i = party.iter().position(|g| *g == dealt[0]).unwrap();
            opened[i] += 1;
            who_opened.push(i);
        }
        // Each of the three once in every round of three bodies.
        for round in who_opened.chunks(3) {
            let mut round = round.to_vec();
            round.sort_unstable();
            assert_eq!(round, vec![0, 1, 2], "{who_opened:?}");
        }
    }

    #[test]
    fn bodies_falling_together_go_to_different_characters() {
        // Nine bodies in one tick, before anyone's turn is on the board:
        // the lowest guid used to be dealt every one of them.
        let at = glam::Vec3::ZERO;
        let fleet: Vec<u32> = (0..9).map(|i| 0x5000_0010 + i).collect();
        let view = view_of(
            fleet[1..]
                .iter()
                .map(|g| looter(*g, at, None, Duration::ZERO))
                .collect(),
        );
        let dealt: Vec<u32> = (1..=9)
            .map(|n| view.opens_first(0x8000_0000 + n, at, turn_at(fleet[0], at, 0)))
            .collect();
        let mut to = dealt.clone();
        to.sort_unstable();
        to.dedup();
        assert_eq!(to.len(), 7, "{dealt:x?}");
        for g in &to {
            assert!(dealt.iter().filter(|d| *d == g).count() <= 2);
        }
    }

    #[test]
    fn the_dealing_mix_gives_the_same_numbers_everywhere() {
        // Worked out by hand from splitmix64's finish: no build, process
        // or platform may deal a body differently.
        assert_eq!(deal(0, 0), 0xe220_a839_7b1d_cdaf);
        assert_eq!(deal(0x8000_0001, 0x5000_0010), 0xfe12_cdc0_9d21_d7ba);
        assert_eq!(deal(0x8000_0001, 0x5000_0011), 0xd972_5b91_3475_6d9c);
        assert_eq!(deal(0x8000_198f, 0x5000_0001), 0x2ac7_ac48_90d6_3ca3);
    }

    #[test]
    fn a_busy_fellows_turn_passes_on_and_the_body_is_still_opened() {
        // Best effort: a character fighting when a body falls loses its
        // turn to one that is free, and a turn nobody takes never leaves
        // the body lying.
        let t0 = Instant::now();
        let (body, at, foe) = (0x8000_0001, glam::Vec3::ZERO, 0x7000_0001);
        let party = [0x5000_0031u32, 0x5000_0032, 0x5000_0033];
        let session = |me: u32, fighting: &[u32]| {
            let row = |g: u32| Mate {
                target: fighting.contains(&g).then_some(foe),
                ..looter(g, at, None, Duration::ZERO)
            };
            let mut ap = Autoplay {
                corpse_seen: vec![(body, t0)],
                team: view_of(
                    party
                        .iter()
                        .filter(|g| **g != me)
                        .map(|g| row(*g))
                        .collect(),
                ),
                ..Default::default()
            };
            // Each reads itself off what it said about itself.
            ap.team.me = Some(row(me));
            ap
        };
        let opening = |fighting: &[u32], now: Instant| -> Vec<u32> {
            party
                .iter()
                .copied()
                .filter(|me| session(*me, fighting).ours_to_open(body, at, *me, at, now))
                .collect()
        };
        // All free: one of them has the turn.
        let dealt = opening(&[], t0);
        assert_eq!(dealt.len(), 1, "{dealt:x?}");
        // That one fighting: another, free, has it instead.
        let passed = opening(&dealt, t0);
        assert_eq!(passed.len(), 1, "{passed:x?}");
        assert_ne!(passed, dealt, "the turn waited on one fighting");
        // After the first second the body is anyone's who is free, the
        // one that was fighting too once it is done.
        assert_eq!(opening(&dealt, t0 + CLAIM_SETTLE), party.to_vec());
        // Everyone fighting: nobody stands off for anybody.
        assert_eq!(opening(&party, t0), party.to_vec());
    }

    /// One profile for a whole party: a broken key for whoever can mend
    /// it, and healing kits up to four.
    fn a_party_profile() -> crate::profile::Profile {
        use crate::profile::{Ask, Mine, Profile, Rule};
        Profile {
            name: "party".into(),
            rules: vec![
                Rule {
                    name: "broken keys, if I can mend them".into(),
                    action: LootAction::Keep,
                    all: vec![
                        Ask::Search("broken".into()),
                        Ask::Me(Mine::Skill {
                            skill: ac_world::stats::skill::LOCKPICK,
                            op: crate::items::Op::Ge,
                            level: 250,
                        }),
                    ],
                    ..Default::default()
                },
                Rule {
                    name: "healing kits, a few".into(),
                    action: LootAction::Keep,
                    all: vec![Ask::Search("healing kit".into())],
                    keep_up_to: Some(4),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    /// A mate whose board row says it has `lockpick` Lockpick.
    fn picking(guid: u32, lockpick: u32) -> Mate {
        use ac_world::stats::{sac, skill};
        Mate {
            level: 30,
            skills: vec![(skill::LOCKPICK, lockpick, lockpick, sac::TRAINED)],
            ..looter(guid, glam::Vec3::ZERO, None, Duration::ZERO)
        }
    }

    #[test]
    fn a_shut_body_says_who_shut_it_what_it_took_and_whom_it_is_left_for() {
        // A nine-character run's log named nobody, so nobody could say who
        // opened a body first or who came back to it for nothing.
        let names = |of: &[&str]| of.iter().map(|n| n.to_string()).collect::<Vec<_>>();
        let body = 0x8000_198f;
        assert_eq!(
            shut_line(
                "Bryn01",
                "Corpse of Biaka",
                body,
                3,
                &names(&["Bryn02", "Bryn03"]),
                &names(&["Bryn04"]),
                &names(&["Bryn05"]),
            ),
            "autoplay: Bryn01 shut Corpse of Biaka (0x8000198f), took 3; \
             done for Bryn02, Bryn03; left for Bryn04; Bryn05 standing by"
        );
        // Alone, it says only what it did.
        assert_eq!(
            shut_line("Bryn01", "Corpse of Biaka", body, 0, &[], &[], &[]),
            "autoplay: Bryn01 shut Corpse of Biaka (0x8000198f), took 0"
        );
        assert_eq!(
            shut_line(
                "Bryn01",
                "Corpse of Biaka",
                body,
                1,
                &[],
                &names(&["Bryn04"]),
                &[]
            ),
            "autoplay: Bryn01 shut Corpse of Biaka (0x8000198f), took 1; left for Bryn04"
        );
        // The one that stays away says whose word it took.
        assert_eq!(
            taken_in_line("Bryn02", "Corpse of Biaka", body, "Bryn01"),
            "autoplay: Bryn02: Corpse of Biaka (0x8000198f) done for me by Bryn01"
        );
        assert_eq!(
            standing_by_line(
                "Bryn05",
                "Corpse of Biaka",
                body,
                "Bryn01",
                &names(&["Bryn04"])
            ),
            "autoplay: Bryn05: Corpse of Biaka (0x8000198f) left for Bryn04 by Bryn01; standing by"
        );
    }

    /// A fellow as a shut judges it, off its row.
    fn as_fellow(m: &Mate) -> Fellow {
        (m.guid, m.name.clone(), m.wielder())
    }

    /// A thing lying on a body, this character carrying `held` of it.
    fn lying(stats: ItemStats, held: u32) -> Option<Left<'static>> {
        Some(Left {
            stats,
            id: None,
            held,
        })
    }

    #[test]
    fn a_mate_is_judged_by_the_skills_it_said_about_itself() {
        // One profile for the party, read differently for each of it: the
        // broken key is the lockpicker's to take and not the mage's. Judged
        // by another character, each is read with what its row said.
        use crate::profile::Verdict;
        let profile = a_party_profile();
        let heard = |said: Mate| -> Mate {
            serde_json::from_value(serde_json::to_value(&said).unwrap()).unwrap()
        };
        let (picker, mage) = (heard(picking(2, 300)), heard(picking(3, 5)));
        let key = item("Broken Marble Key", 0, 0);
        let judge = |m: &Mate| judge_loot(&key, None, Some(&profile), &m.wielder(), &m.name, 0);
        assert_eq!(
            judge(&picker),
            Verdict::Decided(LootAction::Keep, "broken keys, if I can mend them".into())
        );
        assert_eq!(judge(&mage), Verdict::None);
        // Shut by one that cannot mend it: left for the lockpicker, done for
        // the mage, and nothing the opener would take left on it.
        let opener = crate::weapons::Wielder::default();
        let me = Taker {
            guid: 1,
            name: "Bryn01",
            sheet: &opener,
            held: 0,
        };
        let fellows = [as_fellow(&picker), as_fellow(&mage)];
        assert_eq!(
            judge_shut(&[lying(key, 0)], &profile, me, &fellows, &fellows, None),
            ShutFor {
                left_for: vec![picker.guid],
                done_for: vec![mage.guid],
                ..Default::default()
            }
        );
    }

    #[test]
    fn a_fellow_that_would_take_what_was_left_for_another_stands_by_and_is_never_told_it_is_done() {
        // "Healing kits, while my Healing is 100 or more, up to four", read
        // by four of a party. The kit goes to the best healer, who turns out
        // to carry four already. Told the body was done, the others wrote it
        // off, and the kit lay there though two of them wanted it.
        use ac_world::stats::{sac, skill};
        let mut profile = a_party_profile();
        profile.rules[1]
            .all
            .push(crate::profile::Ask::Me(crate::profile::Mine::Skill {
                skill: skill::HEALING,
                op: crate::items::Op::Ge,
                level: 100,
            }));
        let healer = |guid: u32, healing: u32| Mate {
            skills: vec![(skill::HEALING, healing, healing, sac::TRAINED)],
            ..picking(guid, 0)
        };
        let (a, b, c, d) = (healer(1, 200), healer(2, 300), healer(3, 150), healer(4, 0));
        let kit = item("Healing Kit", 50, 0);

        // A opens it carrying two.
        let a_sheet = a.wielder();
        let a_opens = Taker {
            guid: a.guid,
            name: &a.name,
            sheet: &a_sheet,
            held: 2,
        };
        let others = [as_fellow(&b), as_fellow(&c), as_fellow(&d)];
        assert_eq!(
            judge_shut(
                &[lying(kit.clone(), 2)],
                &profile,
                a_opens,
                &others,
                &others,
                None
            ),
            ShutFor {
                left_for: vec![b.guid],
                // C would take the kit too: it stands by for B.
                stand_by: vec![c.guid],
                // D cannot heal, and nothing on the body is for it.
                done_for: vec![d.guid],
                // A would take it too, and stands by rather than writing the
                // body off.
                waits: true,
                ..Default::default()
            }
        );

        // B carries four and leaves it. Nothing goes back to A, which shut
        // the body: the kit is C's now, and A stands by for C.
        let b_sheet = b.wielder();
        let b_opens = Taker {
            guid: b.guid,
            name: &b.name,
            sheet: &b_sheet,
            held: 4,
        };
        let judged = [as_fellow(&a), as_fellow(&c), as_fellow(&d)];
        let sendable = [as_fellow(&c), as_fellow(&d)];
        assert_eq!(
            judge_shut(
                &[lying(kit, 4)],
                &profile,
                b_opens,
                &judged,
                &sendable,
                None
            ),
            ShutFor {
                left_for: vec![c.guid],
                stand_by: vec![a.guid],
                done_for: vec![d.guid],
                ..Default::default()
            }
        );
    }

    #[test]
    fn a_body_stood_by_for_a_fellow_that_cannot_come_is_opened_again() {
        // Salvage was left on every body for the one salvager, and the rest
        // wrote each body off: when the salvager died, followed its leader
        // out of reach or filled its pack first, the salvage lay there until
        // the body rotted.
        use crate::did::Did;
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let (body, at) = (0x8000_0001, glam::Vec3::ZERO);
        let salvager = looter(2, at, None, Duration::ZERO);
        let mut ap = Autoplay {
            team: view_of(vec![salvager.clone()]),
            ..Default::default()
        };
        ap.config.team.enabled = true;
        let left = ShutFor {
            at,
            left_for: vec![salvager.guid],
            waits: true,
            ..Default::default()
        };
        ap.corpse_shut(body, &Did::Done, None, left, t0);
        assert!(
            !ap.looted.contains(&body),
            "wrote off what it would take itself"
        );
        assert!(
            !ap.corpse_waiting(body, t0 + s(1), Room::PLENTY),
            "did not leave it to the salvager"
        );
        let packed = crate::logistics::Supplies {
            laden: true,
            ..Default::default()
        };
        for (why, row) in [
            (
                "dead",
                Some(Mate {
                    health: 0.0,
                    ..salvager.clone()
                }),
            ),
            (
                "played by hand",
                Some(Mate {
                    autoplay: false,
                    ..salvager.clone()
                }),
            ),
            (
                "out of reach",
                Some(Mate {
                    world: glam::Vec3::new(LOOT_NEAR + 5.0, 0.0, 0.0),
                    ..salvager.clone()
                }),
            ),
            (
                "laden",
                Some(Mate {
                    supplies: packed.clone(),
                    ..salvager.clone()
                }),
            ),
            (
                "not looting",
                Some(Mate {
                    opens_bodies: false,
                    ..salvager.clone()
                }),
            ),
            ("off the board", None),
        ] {
            ap.team = view_of(row.into_iter().collect());
            assert!(
                ap.corpse_waiting(body, t0 + s(1), Room::PLENTY),
                "waited on a salvager {why}"
            );
        }
        // Able to come, it is waited on, but not for ever.
        ap.team = view_of(vec![salvager.clone()]);
        assert!(!ap.corpse_waiting(body, t0 + STAND_BY_FOR - s(1), Room::PLENTY));
        let later = t0 + STAND_BY_FOR;
        assert!(
            ap.corpse_waiting(body, later, Room::PLENTY),
            "waited for good on one that never came"
        );
        // Given up on, it is left nothing on that body again, however able
        // it looks: left it again, it was stood by for again, and again.
        ap.stop_standing_by(later);
        assert!(!ap.may_be_sent(body, &salvager, at));
        assert!(ap.may_be_sent(0x8000_0002, &salvager, at));
        assert!(ap.corpse_waiting(body, later, Room::PLENTY));
        // Opened again and emptied, it is done with, and going back to it
        // was no turn.
        ap.corpse_shut(body, &Did::Done, None, ShutFor::default(), later);
        assert!(ap.looted.contains(&body));
        assert_eq!(ap.opened_first(later), 1);
    }

    #[test]
    fn a_fellow_standing_by_goes_by_the_newest_word_on_the_body() {
        let t0 = Instant::now();
        let (body, at) = (0x8000_0001, glam::Vec3::ZERO);
        let (a, b, me, mage) = (1, 2, 3, 4);
        let row = |guid: u32, shut: Vec<Shut>| Mate {
            shut,
            ..looter(guid, at, None, Duration::ZERO)
        };
        let mut ap = Autoplay::default();
        ap.config.enabled = true;
        ap.config.team.enabled = true;
        // A shut it and left the kit on it for B. This character would take
        // the kit too, and stands by for B; the mage wants nothing on it.
        let a_shut = Shut {
            body,
            n: 0,
            done_for: vec![mage],
            stand_by: vec![me],
            left_for: vec![b],
        };
        ap.team = view_of(vec![row(a, vec![a_shut.clone()]), row(b, Vec::new())]);
        assert_eq!(
            ap.take_in_shuts(me, t0, |_| Some(at)),
            vec![TakenIn::StandBy {
                body,
                by: "Bryn01".into(),
                on: vec![b],
            }]
        );
        assert!(!ap.corpse_waiting(body, t0, Room::PLENTY));
        // Nothing on it is left for the mage, told it is done with it,
        // whatever a buff makes of its skills later.
        assert!(!ap.may_judge(body, &looter(mage, at, None, Duration::ZERO), at));
        assert!(ap.may_be_sent(body, &looter(b, at, None, Duration::ZERO), at));

        // B carried its fill already, and shut it leaving the kit to anyone
        // that wants it: this character opens it.
        let b_shut = Shut {
            body,
            n: 7,
            done_for: vec![mage],
            ..Default::default()
        };
        ap.team = view_of(vec![
            row(a, vec![a_shut.clone()]),
            row(b, vec![b_shut.clone()]),
        ]);
        assert!(ap.take_in_shuts(me, t0, |_| Some(at)).is_empty());
        assert!(
            ap.corpse_waiting(body, t0, Room::PLENTY),
            "the kit lay there though it was wanted"
        );
        // Nothing on it goes back to either that shut it.
        for shutter in [a, b] {
            assert!(!ap.may_be_sent(body, &looter(shutter, at, None, Duration::ZERO), at));
        }
        // The same shuts heard again change nothing.
        assert!(ap.take_in_shuts(me, t0, |_| Some(at)).is_empty());
        assert!(ap.corpse_waiting(body, t0, Room::PLENTY));
        // A's next shut of it, having opened it again, is new word.
        let again = Shut {
            body,
            n: 1,
            done_for: vec![me, mage],
            ..Default::default()
        };
        ap.team = view_of(vec![row(a, vec![again]), row(b, vec![b_shut])]);
        assert_eq!(
            ap.take_in_shuts(me, t0, |_| Some(at)),
            vec![TakenIn::Done {
                body,
                by: "Bryn01".into()
            }]
        );
        assert!(!ap.corpse_waiting(body, t0, Room::PLENTY));
    }

    #[test]
    fn every_body_shut_as_emptied_is_said_so_the_others_know_who_shut_it() {
        // Said only when it was done for somebody, a shut that left
        // something for every fellow went unheard: going back to the body
        // counted as a turn, and things on it were left for the one that
        // had shut it already, which never came back.
        use crate::did::Did;
        let t0 = Instant::now();
        let (body, at) = (0x8000_0001, glam::Vec3::ZERO);
        let mut opener = Autoplay::default();
        opener.config.team.enabled = true;
        opener.corpse_shut(body, &Did::Done, None, ShutFor::default(), t0);
        let said = opener.shuts_to_say(t0);
        assert_eq!(
            said,
            vec![Shut {
                body,
                ..Default::default()
            }]
        );

        let mut ap = Autoplay {
            team: view_of(vec![Mate {
                shut: said,
                ..looter(1, at, None, Duration::ZERO)
            }]),
            ..Default::default()
        };
        ap.config.enabled = true;
        ap.config.team.enabled = true;
        assert!(ap.take_in_shuts(2, t0, |_| Some(at)).is_empty());
        assert!(ap.corpse_waiting(body, t0, Room::PLENTY));
        assert!(!ap.may_be_sent(body, &looter(1, at, None, Duration::ZERO), at));
        ap.corpse_shut(body, &Did::Done, None, ShutFor::default(), t0);
        assert_eq!(ap.opened_first(t0), 0, "going back to it counted as a turn");
    }

    #[test]
    fn a_fellow_that_does_not_open_bodies_is_never_dealt_one_or_left_anything() {
        // An Ust carrier with no loot profile salvaged best and stood a few
        // metres from the leader, and never opened a body: the salvage left
        // for it rotted. One down to the slots kept for a sale opens none
        // either.
        let (body, at) = (0x8000_0001, glam::Vec3::ZERO);
        let ust = Mate {
            has_ust: true,
            salvaging: 400,
            opens_bodies: false,
            ..looter(2, at, None, Duration::ZERO)
        };
        assert_eq!(ust.turn(), None);
        assert!(!ust.could_come_for(at));
        let ap = Autoplay::default();
        assert!(!ap.may_judge(body, &ust, at));
        assert!(!ap.may_be_sent(body, &ust, at));
        // Not dealt a newly fallen body, though it has had no turns.
        let me = 3;
        assert_eq!(
            view_of(vec![ust.clone()]).opens_first(body, at, turn_at(me, at, 5)),
            me
        );
        // So its salvage is nobody's in particular: whoever opens the body
        // takes it, and the hand-off carries it to the salvager as ever.
        let profile = crate::profile::Profile {
            name: "party".into(),
            rules: vec![asks(
                "platemail to salvage",
                "platemail",
                LootAction::Salvage,
            )],
            ..Default::default()
        };
        let sheet = crate::weapons::Wielder::default();
        let taker = Taker {
            guid: me,
            name: "Bryn03",
            sheet: &sheet,
            held: 0,
        };
        let plate = item("Platemail", 100, 240);
        assert_eq!(
            called_to(&plate, None, &profile, &[taker], Some(ust.guid)),
            None
        );
    }

    #[test]
    fn only_fellows_that_could_open_a_body_are_judged_at_its_shut() {
        // A character played by hand was named in "left for", and a body
        // found done for it was written off while its rules were off: the
        // player turned them on beside the body, and it was walked past.
        let t0 = Instant::now();
        let (body, at) = (0x8000_0001, glam::Vec3::ZERO);
        let ap = Autoplay::default();
        let able = looter(2, at, None, Duration::ZERO);
        assert!(ap.may_judge(body, &able, at));
        for (why, m) in [
            (
                "played by hand",
                Mate {
                    autoplay: false,
                    ..able.clone()
                },
            ),
            (
                "dead",
                Mate {
                    health: 0.0,
                    ..able.clone()
                },
            ),
            (
                "across the field",
                Mate {
                    world: glam::Vec3::new(LOOT_NEAR + 5.0, 0.0, 0.0),
                    ..able.clone()
                },
            ),
            (
                "with a full pack",
                Mate {
                    supplies: crate::logistics::Supplies {
                        pack_full: true,
                        ..Default::default()
                    },
                    ..able.clone()
                },
            ),
        ] {
            assert!(!ap.may_judge(body, &m, at), "judged a fellow {why}");
        }

        // With its own rules off, a character notes who shut the body and
        // writes nothing off.
        let me = 3;
        let mut ap = Autoplay::default();
        ap.config.enabled = false;
        ap.config.team.enabled = true;
        ap.team.mates = vec![Mate {
            shut: vec![Shut {
                body,
                done_for: vec![me],
                ..Default::default()
            }],
            ..looter(1, at, None, Duration::ZERO)
        }];
        assert!(ap.take_in_shuts(me, t0, |_| Some(at)).is_empty());
        assert!(ap.has_shut(body, 1));
        assert!(
            ap.corpse_waiting(body, t0, Room::PLENTY),
            "wrote a body off while played by hand"
        );
    }

    #[test]
    fn a_body_emptied_long_ago_is_forgotten_before_its_guid_can_come_back() {
        // ACE hands a released guid out again once it has been free for six
        // hours. Remembered for a whole long session, a new body that came
        // with an emptied one's guid was never opened.
        let t0 = Instant::now();
        let body = 0x8000_0001;
        let mut ap = Autoplay::default();
        ap.looted.push(body, t0);
        ap.looted
            .forget_old(t0 + EMPTIED_KEPT - Duration::from_secs(1));
        assert!(!ap.corpse_waiting(body, t0 + CORPSE_LIFE, Room::PLENTY));
        ap.looted.forget_old(t0 + EMPTIED_KEPT);
        assert!(
            ap.corpse_waiting(body, t0 + EMPTIED_KEPT, Room::PLENTY),
            "a new body with an old guid was never opened"
        );
    }

    #[test]
    fn a_session_deals_itself_by_what_it_last_said_about_itself() {
        // The others read this character off its row, up to half a second
        // old. Read as it is now -- at another body, three turns in -- it
        // dealt a newly fallen body to a mate while the mate, reading the
        // row, dealt it back, and for a second nobody opened it.
        let t0 = Instant::now();
        let (body, other_body, at) = (0x8000_0001, 0x8000_0002, glam::Vec3::ZERO);
        let me = 0x5000_0011;
        let mut ap = Autoplay {
            corpse_seen: vec![(body, t0)],
            first_opens: vec![t0; 3],
            team: view_of(vec![Mate {
                opened_first: 1,
                ..looter(0x5000_0012, at, None, Duration::ZERO)
            }]),
            ..Default::default()
        };
        ap.take_up_corpse(other_body, t0, LOOT_TIMEOUT);
        // What it last said: free, and no turns yet.
        ap.team.me = Some(looter(me, at, None, Duration::ZERO));
        assert!(
            ap.ours_to_open(body, at, me, at, t0),
            "dealt itself out on what the others had not heard"
        );
        // Once it has said it is at another body, with three turns, the
        // mate has it, as the mate reckons too.
        ap.team.me = Some(Mate {
            opened_first: 3,
            ..looter(me, at, Some(other_body), Duration::ZERO)
        });
        assert!(!ap.ours_to_open(body, at, me, at, t0));
    }

    #[test]
    fn only_the_skills_the_rules_ask_about_go_on_the_row() {
        use ac_world::stats::{sac, skill};
        let me = crate::weapons::Wielder {
            level: 30,
            skills: vec![
                (skill::LOCKPICK, 200, 260, sac::TRAINED),
                (skill::SALVAGING, 100, 100, sac::TRAINED),
                (skill::WAR_MAGIC, 300, 340, sac::SPECIALIZED),
            ],
            ..Default::default()
        };
        // Buffs and all: a rule on a skill reads it as it stands.
        assert_eq!(
            skills_asked_of(&me, &[skill::LOCKPICK]),
            vec![(skill::LOCKPICK, 200, 260, sac::TRAINED)]
        );
        // A skill asked about that the sheet lacks is not made up: off the
        // row it reads as nothing, as it does for the character itself.
        let row = Mate {
            skills: skills_asked_of(&me, &[skill::LOCKPICK, skill::HEALING]),
            ..Default::default()
        };
        assert_eq!(row.skills.len(), 1);
        assert_eq!(
            row.wielder().skill(skill::HEALING),
            me.skill(skill::HEALING)
        );
        assert_eq!(row.wielder().skill(skill::LOCKPICK), 260);
        // Rules that ask about no skill put none on the row.
        assert!(skills_asked_of(&me, &[]).is_empty());
    }

    #[test]
    fn a_body_one_of_the_others_emptied_for_everyone_is_not_opened_again() {
        // Nine characters opened 83 bodies 697 times, and 42% of the opens
        // took nothing: the others opened a body one of them had emptied,
        // to find it so. The one that shuts it says whom it found nothing
        // left on it for, and those leave it alone.
        let t0 = Instant::now();
        let (me, other) = (2, 3);
        let (body, another) = (0x8000_0001, 0x8000_0002);
        let (here, at) = (glam::Vec3::ZERO, glam::Vec3::new(3.0, 0.0, 0.0));
        let mut ap = Autoplay {
            corpse_seen: vec![(body, t0), (another, t0)],
            ..Default::default()
        };
        ap.config.enabled = true;
        ap.config.team.enabled = true;
        ap.left_for_weight.insert(body, 300);
        assert!(ap.corpse_owed(body, at, here, t0, Room::PLENTY));
        ap.team.mates = vec![Mate {
            shut: vec![Shut {
                body,
                done_for: vec![me, other],
                ..Default::default()
            }],
            ..looter(1, here, None, Duration::ZERO)
        }];
        assert_eq!(
            ap.take_in_shuts(me, t0, |_| None),
            vec![TakenIn::Done {
                body,
                by: "Bryn01".into()
            }]
        );
        assert!(
            !ap.corpse_waiting(body, t0, Room::PLENTY),
            "went to open it again"
        );
        assert!(!ap.corpse_owed(body, at, here, t0, Room::PLENTY));
        // Nothing waits on room for it any more.
        assert_eq!(ap.lightest_left_for_weight(|g| g == body), None);
        // The rest of the ground is as it was.
        assert!(ap.corpse_owed(another, at, here, t0, Room::PLENTY));
        // Heard again the next round: taken in, and logged, once.
        assert!(ap.take_in_shuts(me, t0, |_| None).is_empty());
        assert_eq!(ap.looted.len(), 1);
    }

    #[test]
    fn a_body_left_for_a_mate_still_waits_on_that_mate() {
        // One party, one profile, and a broken key left on a body whose
        // opener cannot mend it. The mage is told the body is done with and
        // stays away; the lockpicker is not, and still goes to it.
        use crate::did::Did;
        let Some(mut c) = character_of_level(30) else {
            return;
        };
        let t0 = Instant::now();
        let profile = a_party_profile();
        let (body, key) = (0x8000_3001, 0x8000_3002);
        c.world.objects.insert(
            body,
            ac_world::WorldObject {
                guid: body,
                name: "Corpse of Drudge Slave".into(),
                object_desc_flags: ac_world::object_desc_flags::CORPSE,
                position: Some(ac_world::object::Position::new_flat(
                    0xA9B4_0019,
                    glam::Vec3::new(84.0, 84.0, 0.0),
                )),
                ..Default::default()
            },
        );
        let at = c.world.objects[&body].world_pos().expect("in the world");
        c.world.objects.insert(
            key,
            ac_world::WorldObject {
                guid: key,
                name: "Broken Marble Key".into(),
                container: Some(body),
                ..Default::default()
            },
        );
        let (picker, mage) = (
            Mate {
                world: at,
                ..picking(2, 300)
            },
            Mate {
                world: at,
                ..picking(3, 5)
            },
        );
        c.autoplay.team.mates = vec![picker.clone(), mage.clone()];
        let shut = c.shut_for(body, &[key], &profile);
        assert_eq!(
            shut,
            ShutFor {
                at,
                done_for: vec![mage.guid],
                left_for: vec![picker.guid],
                ..Default::default()
            }
        );

        // Shut, said on the board, and heard by each.
        c.autoplay.config.team.enabled = true;
        c.autoplay.corpse_shut(body, &Did::Done, None, shut, t0);
        let said = Mate {
            shut: c.autoplay.shuts_to_say(t0),
            ..looter(1, at, None, Duration::ZERO)
        };
        let still_waiting_for = |guid: u32| {
            let mut ap = Autoplay {
                team: view_of(vec![said.clone()]),
                ..Default::default()
            };
            ap.config.enabled = true;
            ap.config.team.enabled = true;
            ap.take_in_shuts(guid, t0, |_| Some(at));
            ap.corpse_waiting(body, t0, Room::PLENTY)
        };
        assert!(
            !still_waiting_for(mage.guid),
            "the mage went to open it for nothing"
        );
        assert!(
            still_waiting_for(picker.guid),
            "the key was not waited on for the lockpicker"
        );

        // In a fellowship with only the mage, the lockpicker is not judged
        // at all: it opens the body or not by its own lights, as before.
        let fellow = |guid| ac_world::Fellow {
            guid,
            ..Default::default()
        };
        c.world.fellowship = Some(ac_world::Fellowship {
            members: vec![fellow(0x5000_0001), fellow(mage.guid)],
            ..Default::default()
        });
        assert_eq!(
            c.shut_for(body, &[key], &profile),
            ShutFor {
                at,
                done_for: vec![mage.guid],
                ..Default::default()
            }
        );
    }

    #[test]
    fn the_leader_that_does_not_sort_first_gives_up_the_fellowship_it_founded() {
        // Two fellowships for one fleet, and this character founded the
        // one whose leader does not sort first. Once the rightful leader
        // has been seen in a fellowship of its own for a few seconds,
        // this one is disbanded, so the rightful leader can recruit its
        // members and this character with them.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_in_the_field(20, holtburg, glam::Vec3::new(84.0, 84.0, 10.0))
        else {
            return;
        };
        let t0 = Instant::now();
        let me = c.world.player_guid.unwrap();
        let member = 0x5000_0005;
        c.autoplay.config.team.enabled = true;
        c.autoplay.config.team.fellowship = true;
        let fellow = |guid| ac_world::Fellow {
            guid,
            ..Default::default()
        };
        c.world.fellowship = Some(ac_world::Fellowship {
            name: "acreborn".into(),
            leader: me,
            members: vec![fellow(me), fellow(member)],
            ..Default::default()
        });
        c.autoplay.founded = Some(t0);
        c.autoplay.team = TeamView {
            mates: vec![
                Mate {
                    name: "+Brynith".into(),
                    guid: 0x5000_0002,
                    in_fellowship: true,
                    leader: true,
                    autoplay: true,
                    ..Default::default()
                },
                Mate {
                    name: "+Brynwyn".into(),
                    guid: member,
                    in_fellowship: true,
                    ..Default::default()
                },
            ],
            leader: false,
            settled: true,
            ..Default::default()
        };
        // The first sight starts the clock; the board's word on a mate
        // is up to a round old, so it is given a few rounds to agree
        // with the world's.
        assert!(!c.autoplay_fellowship(t0));
        assert!(!c.autoplay_fellowship(t0 + YIELD_AFTER / 2));
        assert!(c.autoplay_fellowship(t0 + YIELD_AFTER), "not given up");
        assert!(c.autoplay.founded.is_none());
        assert!(
            c.autoplay.status.contains("disbanding"),
            "{}",
            c.autoplay.status
        );
        // One this character did not found is never given up, whoever
        // leads: the note on a fellowship "not founded by me" stands.
        c.autoplay.founded = None;
        c.autoplay.status.clear();
        assert!(!c.autoplay_fellowship(t0 + YIELD_AFTER * 4));
        assert!(!c.autoplay_fellowship(t0 + YIELD_AFTER * 8));
        assert!(!c.autoplay.status.contains("disbanding"));
        // Nor is one this character founded while it leads the team.
        c.autoplay.founded = Some(t0);
        c.autoplay.team.leader = true;
        assert!(!c.autoplay_fellowship(t0 + YIELD_AFTER * 12));
        assert!(c.autoplay.founded.is_some());
    }

    /// A character leading a fellowship it founded, with `others` in
    /// it besides itself, on a settled team.
    fn leading_a_fellowship(c: &mut Client, others: &[u32], t0: Instant) {
        let me = c.world.player_guid.unwrap();
        c.autoplay.config.team.enabled = true;
        c.autoplay.config.team.fellowship = true;
        let fellow = |guid| ac_world::Fellow {
            guid,
            ..Default::default()
        };
        c.world.fellowship = Some(ac_world::Fellowship {
            name: "acreborn".into(),
            leader: me,
            members: std::iter::once(me)
                .chain(others.iter().copied())
                .map(fellow)
                .collect(),
            ..Default::default()
        });
        c.autoplay.founded = Some(t0);
        c.autoplay.team.settled = true;
    }

    #[test]
    fn whoever_leads_the_fellowship_brings_in_the_rightful_leader_standing_outside() {
        // +Brynlyn founded and gathered the seven that came on with it;
        // +Brynith, first by name, came on a few seconds later. Every
        // roster's leader flipped to +Brynith the moment it was heard:
        // +Brynlyn stopped recruiting, since it led the team no longer,
        // and +Brynith, with nobody free to found with, founded
        // nothing. Seven in a fellowship and the team's leader outside
        // it for the life of the run. Whoever leads a fellowship brings
        // the team's mates into it, the rightful leader among them.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_in_the_field(20, holtburg, glam::Vec3::new(84.0, 84.0, 10.0))
        else {
            return;
        };
        let t0 = Instant::now();
        let here = c.player.as_ref().unwrap().world_position();
        let (rightful, member) = (0x5000_0002, 0x5000_0005);
        leading_a_fellowship(&mut c, &[member], t0);
        c.autoplay.team.leader = false;
        c.autoplay.team.mates = vec![
            Mate {
                name: "+Brynith".into(),
                guid: rightful,
                in_fellowship: false,
                leader: true,
                autoplay: true,
                world: here,
                ..Default::default()
            },
            Mate {
                name: "+Brynwyn".into(),
                guid: member,
                in_fellowship: true,
                world: here,
                ..Default::default()
            },
        ];
        assert!(c.autoplay_fellowship(t0), "{}", c.autoplay.status);
        assert!(
            c.autoplay
                .status
                .contains("bringing +Brynith into the fellowship"),
            "{}",
            c.autoplay.status
        );
        assert_eq!(
            c.autoplay
                .recruited
                .iter()
                .map(|(g, _)| *g)
                .collect::<Vec<_>>(),
            vec![rightful]
        );
        // The other way about, nothing: a team leader that let itself
        // be recruited into a mate's fellowship cannot recruit into it
        // (the server answers 0x041D), and leaves the gathering to that
        // mate.
        c.autoplay.recruited.clear();
        c.autoplay.last_recruit = None;
        c.world.fellowship.as_mut().unwrap().leader = member;
        c.autoplay.founded = None;
        c.autoplay.team.leader = true;
        c.autoplay.team.mates[0].leader = false;
        assert!(!c.autoplay_fellowship(t0 + RECRUIT_AGAIN));
        assert!(c.autoplay.recruited.is_empty());
    }

    #[test]
    fn a_full_fellowship_asks_nobody_else() {
        // Nine in, a tenth on the team: the server answered 0x041E to
        // every ask and the tenth was asked every five seconds for the
        // life of the run. The count says so first, once; and the code,
        // should the server's count differ, holds off whoever was asked
        // last.
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_in_the_field(20, holtburg, glam::Vec3::new(84.0, 84.0, 10.0))
        else {
            return;
        };
        let t0 = Instant::now();
        let here = c.player.as_ref().unwrap().world_position();
        let eight: Vec<u32> = (0..8).map(|i| 0x5000_0010 + i).collect();
        leading_a_fellowship(&mut c, &eight, t0);
        c.autoplay.team.leader = true;
        let tenth = 0x5000_0030;
        c.autoplay.team.mates = eight
            .iter()
            .map(|&guid| Mate {
                name: format!("+Bryn{guid:x}"),
                guid,
                in_fellowship: true,
                world: here,
                ..Default::default()
            })
            .chain(std::iter::once(Mate {
                name: "+Brynzed".into(),
                guid: tenth,
                world: here,
                ..Default::default()
            }))
            .collect();
        // A hold on somebody no longer on the team goes with them.
        let gone = 0x5000_0099;
        c.autoplay.held_off.hold(gone, HELD_OFF_FIRST, t0);
        assert!(!c.autoplay_fellowship(t0));
        assert!(c.autoplay.recruited.is_empty(), "the tenth was asked");
        assert!(
            c.autoplay
                .noted
                .iter()
                .any(|(t, _)| t.contains("the fellowship is full at 9: 1 mate(s)")),
            "{:?}",
            c.autoplay.noted
        );
        assert!(!c.autoplay.held_off.held(&gone, t0));
        // The server's own word on it, about the last one asked.
        c.autoplay.recruited = vec![(tenth, t0)];
        let soon = t0 + Duration::from_secs(1);
        c.autoplay.hear_fellowship_full(soon);
        assert!(c.autoplay.held_off.held(&tenth, soon));
        assert!(!c.autoplay.held_off.held(&tenth, soon + HELD_OFF_FIRST));
    }

    #[test]
    fn a_body_emptied_by_a_mate_stays_emptied_after_the_mate_goes_quiet() {
        // The row goes from the board once its mate has been quiet for six
        // seconds, and a shut is said for twenty; what it said stays said.
        let t0 = Instant::now();
        let (me, body) = (2, 0x8000_0001);
        let mut ap = Autoplay::default();
        ap.config.enabled = true;
        ap.config.team.enabled = true;
        ap.team.mates = vec![Mate {
            shut: vec![Shut {
                body,
                done_for: vec![me],
                ..Default::default()
            }],
            ..looter(1, glam::Vec3::ZERO, None, Duration::ZERO)
        }];
        assert_eq!(ap.take_in_shuts(me, t0, |_| None).len(), 1);
        ap.team = TeamView::default();
        assert!(!ap.corpse_waiting(body, t0 + SHUT_SAID_FOR * 3, Room::PLENTY));
        // Nor does the mate that said it saying nothing more undo it.
        ap.team.mates = vec![looter(1, glam::Vec3::ZERO, None, Duration::ZERO)];
        assert!(ap.take_in_shuts(me, t0, |_| None).is_empty());
        assert!(!ap.corpse_waiting(body, t0 + SHUT_SAID_FOR * 3, Room::PLENTY));
    }

    #[test]
    fn a_body_set_aside_or_left_for_its_weight_is_never_done_for_anyone_else() {
        // Shut for want of room, or because something would not come off,
        // a body still has on it what somebody wanted.
        use crate::did::Did;
        let t0 = Instant::now();
        let mut ap = Autoplay::default();
        ap.config.team.enabled = true;
        let (stubborn, heavy, emptied) = (0x8000_4001, 0x8000_4002, 0x8000_4003);
        let for_both = || ShutFor {
            done_for: vec![2, 3],
            ..Default::default()
        };
        ap.corpse_shut(
            stubborn,
            &Did::blocked("it would not give something up"),
            None,
            for_both(),
            t0,
        );
        ap.corpse_shut(
            heavy,
            &Did::blocked("too laden to take the rest"),
            Some(300),
            for_both(),
            t0,
        );
        assert!(
            ap.shuts_to_say(t0).is_empty(),
            "said a body with something on it was emptied"
        );
        ap.corpse_shut(emptied, &Did::Done, None, for_both(), t0);
        assert_eq!(
            ap.shuts_to_say(t0),
            vec![Shut {
                body: emptied,
                n: 0,
                done_for: vec![2, 3],
                ..Default::default()
            }]
        );
    }

    #[test]
    fn a_mate_under_its_cap_is_left_the_healing_kits_the_opener_would_not_take() {
        use crate::profile::Verdict;
        let profile = a_party_profile();
        let kits = ItemStats {
            stack: 5,
            ..item("Healing Kit", 50, 0)
        };
        // The opener carries four already: the rule stops there, and
        // nothing else claims them.
        let opener = picking(1, 0);
        assert_eq!(
            judge_loot(&kits, None, Some(&profile), &opener.wielder(), "Bryn01", 4),
            Verdict::None
        );
        // Nobody knows what a mate carries, so the mate is not done with the
        // body, and opens it by its own lights: the rule asks about no
        // skill, so the kits are nobody's in particular.
        let mate = picking(2, 0);
        let sheet = opener.wielder();
        let me = Taker {
            guid: opener.guid,
            name: &opener.name,
            sheet: &sheet,
            held: 4,
        };
        let fellows = [as_fellow(&mate)];
        assert_eq!(
            judge_shut(&[lying(kits, 4)], &profile, me, &fellows, &fellows, None),
            ShutFor::default()
        );
    }

    #[test]
    fn a_mate_that_would_need_an_appraisal_is_not_counted_done() {
        let mut profile = a_party_profile();
        profile
            .rules
            .push(asks("good armour", "type:armor al>=200", LootAction::Sell));
        let mage = picking(3, 0);
        let sheet = crate::weapons::Wielder::default();
        let me = Taker {
            guid: 1,
            name: "Bryn01",
            sheet: &sheet,
            held: 0,
        };
        let done = |lying: &[Option<Left>], profile: &crate::profile::Profile| {
            judge_shut(lying, profile, me, &[as_fellow(&mage)], &[], None).done_for
                == vec![mage.guid]
        };
        let unread = ItemStats {
            appraised: false,
            ..item("Platemail", 100, 240)
        };
        // Not appraised, it might be good armour: the mate would ask.
        assert!(!done(&[lying(unread.clone(), 0)], &profile));
        // Appraised by the one that shut it, it is judged outright.
        assert!(!done(&[lying(item("Platemail", 100, 240), 0)], &profile));
        assert!(done(&[lying(item("Platemail", 100, 50), 0)], &profile));
        // Where the rules never appraise, the mate would never ask either.
        profile.looting.appraise = false;
        assert!(done(&[lying(unread, 0)], &profile));
        // A thing not described yet cannot be judged, and is waited on.
        assert!(!done(&[None], &profile));
    }

    #[test]
    fn a_mate_outside_the_fellowship_is_never_counted_done_or_left_anything() {
        let at = glam::Vec3::ZERO;
        let view = view_of(vec![
            looter(1, at, None, Duration::ZERO),
            looter(2, at, None, Duration::ZERO),
            // Not in the world yet.
            looter(0, at, None, Duration::ZERO),
        ]);
        let judged = |fellows: Option<&[u32]>| {
            view.judged_at_a_shut(fellows)
                .map(|m| m.guid)
                .collect::<Vec<_>>()
        };
        // Out of a fellowship, everyone in the world.
        assert_eq!(judged(None), vec![1, 2]);
        // In one, its fellows only.
        assert_eq!(judged(Some(&[9, 1])), vec![1]);
    }

    #[test]
    fn what_a_shut_says_is_bounded_and_ages_off() {
        use crate::did::Did;
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let mut ap = Autoplay::default();
        // Off the team there is nobody to say it to.
        ap.corpse_shut(0x8000_0001, &Did::Done, None, ShutFor::default(), t0);
        assert!(ap.shuts_to_say(t0).is_empty());
        ap.config.team.enabled = true;
        // Only the newest are said.
        let first = 0x8000_1000;
        for n in 0..SHUTS_SAID as u32 + 4 {
            let at = t0 + Duration::from_millis(n as u64);
            let shut = ShutFor {
                done_for: vec![2],
                ..Default::default()
            };
            ap.corpse_shut(first + n, &Did::Done, None, shut, at);
        }
        let said = ap.shuts_to_say(t0 + s(1));
        assert_eq!(said.len(), SHUTS_SAID);
        assert_eq!(said.first().map(|shut| shut.body), Some(first + 4));
        // A body shut again is said once, as a new shut.
        let again = ShutFor {
            done_for: vec![2, 3],
            ..Default::default()
        };
        ap.corpse_shut(first + 10, &Did::Done, None, again, t0 + s(2));
        let said = ap.shuts_to_say(t0 + s(2));
        assert_eq!(said.len(), SHUTS_SAID);
        let tenth: Vec<&Shut> = said.iter().filter(|shut| shut.body == first + 10).collect();
        assert_eq!(tenth.len(), 1);
        assert_eq!(tenth[0].n, SHUTS_SAID as u32 + 4);
        // And only for a while.
        assert!(ap.shuts_to_say(t0 + s(2) + SHUT_SAID_FOR).is_empty());
        // Done with here all the same, said or not.
        assert!(ap.looted.contains(&0x8000_0001) && ap.looted.contains(&first));
    }

    #[test]
    fn a_character_alone_loots_exactly_as_it_did() {
        // Nobody on the board: nothing to say, nothing to hear, and every
        // body its own as before.
        use crate::did::Did;
        let t0 = Instant::now();
        let (me, body) = (2, 0x8000_0001);
        let (here, at) = (glam::Vec3::ZERO, glam::Vec3::new(3.0, 0.0, 0.0));
        let mut ap = Autoplay {
            corpse_seen: vec![(body, t0)],
            ..Default::default()
        };
        ap.config.enabled = true;
        for team in [false, true] {
            ap.config.team.enabled = team;
            assert!(ap.ours_to_open(body, at, me, here, t0));
            assert!(ap.corpse_owed(body, at, here, t0, Room::PLENTY));
            assert_eq!(ap.team.judged_at_a_shut(None).count(), 0);
            assert!(ap.take_in_shuts(me, t0, |_| None).is_empty());
            assert!(ap.looted.is_empty());
        }
        ap.config.team.enabled = false;
        ap.take_up_corpse(body, t0, LOOT_TIMEOUT);
        ap.corpse_shut(body, &Did::Done, None, ShutFor::default(), t0);
        assert!(ap.looted.contains(&body) && ap.looted.len() == 1);
        assert!(ap.shuts_to_say(t0).is_empty(), "told nobody about it");
        assert!(!ap.corpse_waiting(body, t0, Room::PLENTY));

        // And a character alone finds nobody to judge a body for.
        let Some(c) = character_of_level(30) else {
            return;
        };
        assert_eq!(
            c.shut_for(body, &[0x8000_0002], &a_party_profile()),
            ShutFor::default()
        );
    }

    #[test]
    fn a_character_off_the_team_takes_in_no_shuts() {
        let t0 = Instant::now();
        let (me, body) = (2, 0x8000_0001);
        let mut ap = Autoplay::default();
        ap.config.enabled = true;
        assert!(!ap.config.team.enabled);
        ap.team.mates = vec![Mate {
            shut: vec![Shut {
                body,
                done_for: vec![me],
                ..Default::default()
            }],
            ..looter(1, glam::Vec3::ZERO, None, Duration::ZERO)
        }];
        assert!(ap.take_in_shuts(me, t0, |_| None).is_empty());
        assert!(ap.looted.is_empty());
        assert!(ap.corpse_waiting(body, t0, Room::PLENTY));
        // Back on the team, the same word is taken in.
        ap.config.team.enabled = true;
        assert_eq!(
            ap.take_in_shuts(me, t0, |_| None),
            vec![TakenIn::Done {
                body,
                by: "Bryn01".into()
            }]
        );
    }

    fn view_of(mates: Vec<Mate>) -> TeamView {
        TeamView {
            mates,
            ..Default::default()
        }
    }

    #[test]
    fn a_body_opened_first_is_a_turn_and_one_left_for_this_character_is_not() {
        use crate::did::Did;
        let t0 = Instant::now();
        let me = 2;
        let mut ap = Autoplay::default();
        ap.config.enabled = true;
        ap.config.team.enabled = true;
        assert_eq!(ap.opened_first(t0), 0);
        ap.corpse_shut(0x8000_0001, &Did::Done, None, ShutFor::default(), t0);
        assert_eq!(ap.opened_first(t0), 1);
        // One of the others shut this one first and left it for us: going
        // back for what it left is no turn.
        let passed_on = 0x8000_0002;
        ap.team.mates = vec![Mate {
            shut: vec![Shut {
                body: passed_on,
                done_for: vec![3],
                ..Default::default()
            }],
            ..looter(1, glam::Vec3::ZERO, None, Duration::ZERO)
        }];
        assert!(
            ap.take_in_shuts(me, t0, |_| None).is_empty(),
            "left for us, not done for us"
        );
        assert!(ap.has_shut(passed_on, 1));
        ap.corpse_shut(passed_on, &Did::Done, None, ShutFor::default(), t0);
        assert_eq!(ap.opened_first(t0), 1);
        // Nor is a body set aside.
        ap.corpse_shut(
            0x8000_0003,
            &Did::blocked("it would not give something up"),
            None,
            ShutFor::default(),
            t0,
        );
        assert_eq!(ap.opened_first(t0), 1);
        // Turns are counted lately, not for ever.
        assert_eq!(ap.opened_first(t0 + DEAL_WINDOW), 0);
        // And what was heard goes with the body.
        ap.forget_corpses_gone(|g| g != passed_on);
        assert!(!ap.has_shut(passed_on, 1));
    }

    #[test]
    fn a_thing_a_skill_rule_takes_is_for_the_highest_of_that_skill_as_it_stands() {
        // "If an item has a skill requirement in the loot settings, it
        // should go to whoever has the highest skill."
        use ac_world::stats::{sac, skill};
        let sheet = |base: u32, now: u32| crate::weapons::Wielder {
            level: 50,
            skills: vec![(skill::LOCKPICK, base, now, sac::TRAINED)],
            ..Default::default()
        };
        let profile = a_party_profile();
        let key = item("Broken Marble Key", 0, 0);
        let (mine, steady, buffed, sapped) = (
            sheet(300, 300),
            sheet(320, 320),
            sheet(280, 360),
            sheet(400, 200),
        );
        let taker = |guid, name, sheet| Taker {
            guid,
            name,
            sheet,
            held: 0,
        };
        let takers = [
            taker(1, "Bryn01", &mine),
            taker(2, "Bryn02", &steady),
            taker(3, "Bryn03", &buffed),
            taker(4, "Bryn04", &sapped),
        ];
        // Buffs counted, as the rule counts them: the highest as it
        // stands. The highest before buffs is drained below what the rule
        // asks, and the rule does not take the key for it at all.
        assert_eq!(called_to(&key, None, &profile, &takers, None), Some(3));
        // Two as high: the name that sorts first, in every session.
        let twin = sheet(360, 360);
        let tied = [takers[2], taker(5, "Aldric", &twin)];
        assert_eq!(called_to(&key, None, &profile, &tied, None), Some(5));
        // Alone, it is this character's, as it always was.
        assert_eq!(called_to(&key, None, &profile, &takers[..1], None), Some(1));
        // What no rule asking about a skill takes is nobody's in particular.
        let kits = item("Healing Kit", 50, 0);
        assert_eq!(called_to(&kits, None, &profile, &takers, Some(2)), None);

        // Salvage is for whoever salvages for the team.
        let mut salvaging = profile.clone();
        salvaging.rules.push(asks(
            "platemail to salvage",
            "platemail",
            LootAction::Salvage,
        ));
        let plate = item("Platemail", 100, 240);
        assert_eq!(
            called_to(&plate, None, &salvaging, &takers, Some(2)),
            Some(2)
        );
        // When it is not one of those that could take it -- out of reach,
        // or nobody carries an Ust -- it is not waited on: the thing is
        // nobody's in particular, and the hand-off takes it there later.
        assert_eq!(called_to(&plate, None, &salvaging, &takers, Some(9)), None);
        assert_eq!(called_to(&plate, None, &salvaging, &takers, None), None);

        // A key two rules would take goes by the first of them in the
        // profile, as each character reads its rules: the lockpick rule,
        // though for the one it does not hold for the salvage rule further
        // down would take it.
        salvaging
            .rules
            .push(asks("keys to salvage", "broken", LootAction::Salvage));
        assert_eq!(called_to(&key, None, &salvaging, &takers, Some(4)), Some(3));
        // And a rule ahead of both that takes it for anyone makes it
        // nobody's in particular.
        salvaging
            .rules
            .insert(0, asks("every key", "key", LootAction::Keep));
        assert_eq!(called_to(&key, None, &salvaging, &takers, Some(4)), None);
    }

    #[test]
    fn nobody_is_sent_anything_when_no_rule_asks_about_a_skill_and_nobody_salvages() {
        let mut profile = crate::profile::Profile {
            name: "plain".into(),
            rules: vec![
                asks("gems", "diamond", LootAction::Sell),
                asks("platemail to salvage", "platemail", LootAction::Salvage),
                asks("kits", "healing kit", LootAction::Keep),
            ],
            ..Default::default()
        };
        profile.looting.salvage = false;
        assert!(!profile.sends_to_the_best());
        let weak = crate::weapons::Wielder::default();
        let strong = crate::weapons::Wielder {
            level: 200,
            skills: vec![(ac_world::stats::skill::SALVAGING, 400, 450, 3)],
            ..Default::default()
        };
        let takers = [
            Taker {
                guid: 1,
                name: "Bryn01",
                sheet: &weak,
                held: 0,
            },
            Taker {
                guid: 2,
                name: "Bryn02",
                sheet: &strong,
                held: 0,
            },
        ];
        for thing in [
            item("Diamond", 5000, 0),
            item("Platemail", 100, 240),
            item("Healing Kit", 50, 0),
        ] {
            assert_eq!(
                called_to(&thing, None, &profile, &takers, Some(2)),
                None,
                "{}",
                thing.name
            );
        }
        // Nor under the starter, while its salvager does not salvage.
        let mut starter = crate::profile::Profile::starter();
        starter.looting.salvage = false;
        assert!(!starter.sends_to_the_best());
    }

    #[test]
    fn a_mate_dead_or_missing_from_the_board_is_never_waited_on() {
        let t0 = Instant::now();
        let (body, at) = (0x8000_0001, glam::Vec3::ZERO);
        let alive = looter(2, at, None, Duration::ZERO);
        assert!(alive.could_come_for(at));
        // At another body or fighting, it comes once it is done.
        assert!(Mate {
            looting: Some(0x8000_0002),
            target: Some(0x7000_0001),
            ..alive.clone()
        }
        .could_come_for(at));
        for gone in [
            Mate {
                health: 0.0,
                ..alive.clone()
            },
            Mate {
                autoplay: false,
                ..alive.clone()
            },
            Mate {
                guid: 0,
                ..alive.clone()
            },
            Mate {
                opens_bodies: false,
                ..alive.clone()
            },
        ] {
            assert_eq!(gone.turn(), None);
            assert!(!gone.could_come_for(at));
        }
        // Nor one across the field, or with no room for what is left.
        assert!(!Mate {
            world: glam::Vec3::new(LOOT_NEAR + 5.0, 0.0, 0.0),
            ..alive.clone()
        }
        .could_come_for(at));
        for (pack_full, laden) in [(true, false), (false, true)] {
            let full = Mate {
                supplies: crate::logistics::Supplies {
                    pack_full,
                    laden,
                    ..Default::default()
                },
                ..alive.clone()
            };
            assert!(!full.could_come_for(at));
        }

        // Nor is a body's turn dealt to one dead, or to one gone quiet and
        // off the board: either way the body is this character's.
        let me = 3;
        let mut ap = Autoplay {
            corpse_seen: vec![(body, t0)],
            first_opens: vec![t0; 3],
            team: view_of(vec![looter(1, at, None, Duration::ZERO)]),
            ..Default::default()
        };
        assert!(!ap.ours_to_open(body, at, me, at, t0), "had more turns");
        // And standing off it, this character says whose turn it is.
        assert_eq!(ap.whose_turn(body, at, me, at, t0), Some("Bryn01"));
        assert_eq!(ap.whose_turn(body, at, me, at, t0 + CLAIM_SETTLE), None);
        ap.team.mates[0].health = 0.0;
        assert!(
            ap.ours_to_open(body, at, me, at, t0),
            "waited on a dead mate's turn"
        );
        assert_eq!(ap.whose_turn(body, at, me, at, t0), None);
        ap.team.mates.clear();
        assert!(ap.ours_to_open(body, at, me, at, t0));
    }

    #[test]
    fn the_salvage_on_an_open_body_is_left_for_the_salvager_standing_by() {
        use crate::logistics::Supplies;
        let holtburg = 0xA9B4_0019;
        let Some(mut c) = standing_in_the_field(20, holtburg, glam::Vec3::new(84.0, 84.0, 10.0))
        else {
            return;
        };
        let me = c.player.as_ref().unwrap().world_position();
        let t0 = Instant::now();
        let (body, plate, gem) = (0x8000_5001, 0x8000_5002, 0x8000_5003);
        c.world.objects.insert(
            body,
            ac_world::WorldObject {
                guid: body,
                name: "Corpse of Drudge Slave".into(),
                object_desc_flags: ac_world::object_desc_flags::CORPSE,
                position: Some(ac_world::object::Position::new_flat(
                    holtburg,
                    me + glam::Vec3::new(2.0, 0.0, 0.0) - ac_world::landblock_origin(holtburg),
                )),
                ..Default::default()
            },
        );
        for (guid, name) in [(plate, "Platemail"), (gem, "Diamond")] {
            c.world.objects.insert(
                guid,
                ac_world::WorldObject {
                    guid,
                    name: name.into(),
                    container: Some(body),
                    ..Default::default()
                },
            );
        }
        let mut profile = crate::profile::Profile {
            name: "party".into(),
            rules: vec![
                asks("platemail to salvage", "platemail", LootAction::Salvage),
                asks("gems", "diamond", LootAction::Sell),
            ],
            ..Default::default()
        };
        // Bryn02 salvages for the party and stands by; Bryn03 does not
        // salvage. This character carries no Ust.
        let salvager = Mate {
            has_ust: true,
            salvaging: 300,
            ..looter(2, me, None, Duration::ZERO)
        };
        let other = looter(3, me, None, Duration::ZERO);
        c.autoplay.team.mates = vec![salvager.clone(), other.clone()];
        let verdicts = |c: &mut Client, profile: &crate::profile::Profile| {
            let open = c.corpse_now(body, &[plate, gem], profile, t0, t0);
            let of = |g| open.items.iter().find(|i| i.guid == g).map(|i| i.verdict);
            (of(plate), of(gem))
        };
        let take = |a| Some(ac_loot::Verdict::Take(a));
        assert_eq!(
            verdicts(&mut c, &profile),
            (Some(ac_loot::Verdict::Leave), take(LootAction::Sell)),
            "took the salvager's salvage, or left the rest"
        );
        // Shut with the platemail on it: left for the salvager. Bryn03 would
        // take it too and stands by for the salvager, and so does this
        // character, rather than either writing the body off.
        let shut = c.shut_for(body, &[plate], &profile);
        assert_eq!(
            (shut.done_for, shut.stand_by, shut.left_for, shut.waits),
            (Vec::<u32>::new(), vec![3], vec![2], true)
        );

        // Never waited on: dead, off the board, with no room, not looting,
        // or having shut this body already. Each time it is taken as ever.
        let salvage = (take(LootAction::Salvage), take(LootAction::Sell));
        c.autoplay.team.mates = vec![
            Mate {
                health: 0.0,
                ..salvager.clone()
            },
            other.clone(),
        ];
        assert_eq!(verdicts(&mut c, &profile), salvage, "dead");
        c.autoplay.team.mates = vec![other.clone()];
        assert_eq!(verdicts(&mut c, &profile), salvage, "off the board");
        c.autoplay.team.mates = vec![
            Mate {
                supplies: Supplies {
                    laden: true,
                    ..Default::default()
                },
                ..salvager.clone()
            },
            other.clone(),
        ];
        assert_eq!(verdicts(&mut c, &profile), salvage, "laden");
        c.autoplay.team.mates = vec![
            Mate {
                opens_bodies: false,
                ..salvager.clone()
            },
            other.clone(),
        ];
        assert_eq!(verdicts(&mut c, &profile), salvage, "not looting");
        c.autoplay.config.team.enabled = true;
        c.autoplay.team.mates = vec![
            Mate {
                shut: vec![Shut {
                    body,
                    done_for: vec![3],
                    ..Default::default()
                }],
                ..salvager.clone()
            },
            other.clone(),
        ];
        c.autoplay.take_in_shuts(0x5000_0001, t0, |_| None);
        assert_eq!(verdicts(&mut c, &profile), salvage, "shut it already");

        // Rules that mean nothing for anyone in particular, and alone.
        c.autoplay.team.mates = vec![salvager, other];
        c.autoplay.shut_by.clear();
        c.autoplay.done_with.clear();
        profile.looting.salvage = false;
        assert_eq!(verdicts(&mut c, &profile), salvage, "nobody salvages");
        profile.looting.salvage = true;
        c.autoplay.team.mates.clear();
        assert_eq!(verdicts(&mut c, &profile), salvage, "alone");
    }

    #[test]
    fn a_body_a_step_or_two_off_is_walked_back_to_and_not_let_go_of() {
        // The server does not mind the distance: a corpse has no reset
        // interval, so it stays open until its viewer shuts it. Shutting
        // one the character was already holding handed it back to the
        // other eight and the walk back had to win it again -- fifty
        // times in one run.
        //
        // A dodge or a knock-back moves a character several metres in a
        // frame, and that is the drift this covers.
        assert!(still_holding_at(CORPSE_REACH + 0.5), "no room to drift");
        assert!(still_holding_at(HOLD_ON_WITHIN));
        // Really gone: shut it and walk back to it like any other body.
        assert!(!still_holding_at(HOLD_ON_WITHIN + 0.5));
        // Never as far as the twenty metres the looting calls its own,
        // or a body held would be one nothing else could ever have.
        assert!(!still_holding_at(LOOT_NEAR));
    }

    #[test]
    fn a_walk_to_a_corpse_that_cannot_arrive_is_given_up_and_the_corpse_set_aside() {
        // Blargerton walked every tick at bodies through a floor or
        // behind a wall. Nothing timed the walk, so the looting, and the
        // fight waiting on the body, held until it rotted.
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let guid = 0x8000_3001;

        // A long walk that keeps getting nearer goes on however long it
        // takes.
        let mut walk = CorpseWalk::new(guid, 40.0, t0);
        for i in 1..=30 {
            assert!(walk.goes_on(40.0 - i as f32 * 1.2, false, t0 + s(i)));
        }

        // Pressed against a wall and no nearer: given up once that has
        // gone on too long, and not before.
        let mut walk = CorpseWalk::new(guid, 12.0, t0);
        for i in 1..=REACH_GIVE_UP.as_secs() {
            assert!(walk.goes_on(11.5, false, t0 + s(i)), "gave up at {i} s");
        }
        let gave_up = t0 + REACH_GIVE_UP + s(1);
        assert!(!walk.goes_on(11.5, false, gave_up));

        // The steering finding no way there is given up on sooner, but
        // not at the first word of it.
        let mut walk = CorpseWalk::new(guid, 12.0, t0);
        assert!(walk.goes_on(12.0, true, t0 + s(1)));
        assert!(walk.goes_on(12.0, false, t0 + s(2)), "a way was found");
        let said = t0 + s(3);
        for i in 3..3 + NO_WAY_FOR.as_secs() {
            assert!(walk.goes_on(12.0, true, t0 + s(i)), "gave up at {i} s");
        }
        assert!(!walk.goes_on(12.0, true, said + NO_WAY_FOR));

        // A walk broken off for a fight is a new walk when it is taken
        // up again.
        let mut walk = CorpseWalk::new(guid, 12.0, t0);
        assert!(walk.goes_on(12.0, false, t0 + s(1)));
        assert!(walk.goes_on(12.0, false, t0 + s(40)));

        // What `autoplay_loot` does with a walk given up: the body is set
        // aside, not written off, and neither the looting nor the next
        // fight waits on it meanwhile.
        let mut ap = Autoplay::default();
        let (me, at) = (glam::Vec3::ZERO, glam::Vec3::new(12.0, 0.0, 0.0));
        let room = Room::PLENTY;
        assert!(ap.corpse_owed(guid, at, me, gave_up, room));
        ap.set_aside_out_of_reach(guid, gave_up);
        assert!(!ap.looted.contains(&guid), "written off for good");
        assert!(!ap.corpse_owed(guid, at, me, gave_up, room), "still owed");
        assert!(
            ap.corpse_owed(guid, at, me, gave_up + s(60), room),
            "never tried again"
        );
    }

    #[test]
    fn a_ranged_attacker_closes_in_before_it_gives_up() {
        // Nothing landing from forty metres: try from twenty, then ten,
        // then as near as is worth being -- and only then give up.
        assert_eq!(closer_stand_off(40.0), Some(20.0));
        assert_eq!(closer_stand_off(20.0), Some(10.0));
        assert_eq!(closer_stand_off(10.0), Some(MIN_STAND_OFF));
        assert_eq!(closer_stand_off(MIN_STAND_OFF), None);
        assert_eq!(closer_stand_off(MIN_STAND_OFF + 0.5), None);
    }

    #[test]
    fn a_re_judged_pack_counts_each_kind_as_it_goes() {
        // Three rings and two piles of tapers, in the order they were
        // come by. Each ring is told how many rings were held before
        // it -- none, one, two -- not that three are carried, so a rule
        // that keeps up to two claims the first two and not the third.
        // Told instead that it was the second of two, the second ring
        // sat over the cap and only one was kept.
        let mut carried = vec![
            (30, 500, 1),   // third ring
            (10, 500, 1),   // first ring
            (25, 691, 300), // second pile of tapers
            (20, 500, 1),   // second ring
            (15, 691, 120), // first pile of tapers
        ];
        assert_eq!(
            in_arrival_order(&mut carried),
            vec![(10, 0), (15, 0), (20, 1), (25, 120), (30, 2)]
        );
        // A stack counts for what it holds, not for one.
        let mut two = vec![(7, 691, 4059), (8, 691, 1)];
        assert_eq!(in_arrival_order(&mut two), vec![(7, 0), (8, 4059)]);
        assert!(in_arrival_order(&mut []).is_empty());
    }

    #[test]
    fn what_an_item_was_taken_for_is_written_down() {
        let mut ap = Autoplay::default();
        let ring = item("Ornate Ring", 900, 0);
        assert_eq!(ap.tags().get(&ring.guid), None);
        ap.tag(&ring, LootAction::Salvage);
        assert_eq!(ap.tags().get(&ring.guid), Some(&LootAction::Salvage));
        assert_eq!(Doing::Salvaging.label(), "salvaging");
    }

    /// A corpse open at the character's feet, as the loot rules see it,
    /// with one thing on it worth taking.
    fn corpse_at_hand(guid: u32, item: u32) -> ac_loot::Open {
        ac_loot::Open {
            guid,
            name: "Corpse of a Drudge Skulker".into(),
            open: true,
            items: vec![ac_loot::Lying {
                guid: item,
                name: "Dagger".into(),
                burden: 10,
                verdict: ac_loot::Verdict::Take(LootAction::Keep),
                needs_no_slot: false,
            }],
            slots_free: 20,
            room_anywhere: 20,
            carry_room: 10_000,
            ..Default::default()
        }
    }

    #[test]
    fn the_corpse_opened_after_one_was_given_up_on_is_not_written_off_on_its_first_step() {
        // Blargerton: a corpse that would not empty was given up on after
        // forty-five seconds, and the loot rules' clock went on running
        // from it. The next body he opened was shut on its first step
        // for having taken too long, and marked looted with everything
        // still on it.
        let t0 = Instant::now();
        let mut ap = Autoplay::default();
        let (first, second) = (0x8000_1001, 0x8000_1002);
        ap.take_up_corpse(first, t0, LOOT_TIMEOUT);
        let next = ap.loot_run.step(&corpse_at_hand(first, 1), t0);
        assert_eq!(next.act, Some(ac_loot::Act::Take(1)), "{}", next.saying);
        // Let go of some way other than a shut by the rules: given up on
        // as it once was, or asked again when it would not open.
        let gave_up = t0 + ac_loot::run::KEEP_AT_IT + Duration::from_secs(1);
        ap.let_go_of_corpse();
        assert_eq!(ap.corpse, None);
        // The next corpse is chosen, and opens a moment later.
        ap.take_up_corpse(second, gave_up, LOOT_TIMEOUT);
        let opened = gave_up + Duration::from_secs(1);
        let next = ap.loot_run.step(&corpse_at_hand(second, 2), opened);
        assert_eq!(next.act, Some(ac_loot::Act::Take(2)), "{}", next.saying);
        assert!(!ap.looted.contains(&second), "written off unlooted");
    }

    #[test]
    fn a_corpse_the_rules_set_aside_is_shelved_and_not_written_off() {
        // The rules shut a corpse that will not give up its contents as
        // Blocked -- "later" -- and the client marked every shut corpse
        // looted, which is "never".
        use crate::did::Did;
        let t0 = Instant::now();
        let mut ap = Autoplay::default();
        let stubborn = 0x8000_2001;
        ap.take_up_corpse(stubborn, t0, LOOT_TIMEOUT);
        ap.corpse_shut(
            stubborn,
            &Did::blocked("it will not give up its contents"),
            None,
            ShutFor::default(),
            t0,
        );
        assert_eq!(ap.corpse, None, "still in hand");
        assert!(!ap.looted.contains(&stubborn), "written off for good");
        assert!(ap.shelved.held(&stubborn, t0), "not left alone for now");
        assert!(
            !ap.shelved.held(&stubborn, t0 + Duration::from_secs(60)),
            "never tried again"
        );
        // One the rules emptied is done with.
        let emptied = 0x8000_2002;
        ap.take_up_corpse(emptied, t0, LOOT_TIMEOUT);
        ap.corpse_shut(emptied, &Did::Done, None, ShutFor::default(), t0);
        assert!(ap.looted.contains(&emptied));
        assert!(!ap.shelved.held(&emptied, t0));
    }

    #[test]
    fn the_server_names_the_body_it_will_not_open() {
        assert_eq!(
            corpse_refusal("The Corpse of Hellion is already in use by someone else!"),
            Some(("Corpse of Hellion", CorpseRefusal::InUse))
        );
        // A server that tells you whose (`container_opener_name`).
        assert_eq!(
            corpse_refusal("The Chest is already in use by +Brynna!"),
            Some(("Chest", CorpseRefusal::InUse))
        );
        assert_eq!(
            corpse_refusal("You do not yet have the right to loot the Corpse of Hellion."),
            Some(("Corpse of Hellion", CorpseRefusal::NotYetOurs))
        );
        assert_eq!(
            corpse_refusal(
                "You may not loot the Corpse of Hellion because the Corpse of Hellion \
                 has generated a rare item."
            ),
            Some(("Corpse of Hellion", CorpseRefusal::NeverOurs))
        );
        assert_eq!(
            corpse_refusal(
                "You may not loot the Corpse of Biaka because the death was caused by \
                 a player killer."
            ),
            Some(("Corpse of Biaka", CorpseRefusal::NeverOurs))
        );
        // Everything else the server says while a body waits to open.
        assert_eq!(corpse_refusal("You're too busy"), None);
        assert_eq!(corpse_refusal("The Corpse of Hellion is open"), None);
        assert_eq!(corpse_refusal("You do not have permission to loot"), None);
    }

    #[test]
    fn the_words_a_body_is_refused_in_say_how_long_to_leave_it() {
        // 1,044 refusals arrived in that run, a third of a second after
        // the ask, and every one of them was thrown away: the take-up
        // ended on a clock instead, and asked again three times more.
        let t0 = Instant::now();
        let name = "Corpse of Hellion";
        let answered = t0 + Duration::from_millis(360);
        let in_use = format!("The {name} is already in use by someone else!");
        let not_ours = format!("You do not yet have the right to loot the {name}.");
        let (held, locked, rare) = (0x8000_1221, 0x8000_1222, 0x8000_1223);
        let mut ap = Autoplay {
            corpse_seen: vec![(held, t0), (locked, t0), (rare, t0)],
            ..Default::default()
        };

        // Someone is inside it: let it go and have it the moment they
        // are done, not in the half minute anything blocked waits.
        ap.take_up_corpse(held, t0, LOOT_TIMEOUT);
        assert!(ap.corpse_refused(&in_use, name, Some(answered), answered));
        assert_eq!(ap.corpse, None, "still holding a body it cannot open");
        assert_eq!(ap.shelved.waited(&held), Some(CORPSE_IN_USE_AGAIN));
        assert!(ap.shelved.held(
            &held,
            answered + CORPSE_IN_USE_AGAIN - Duration::from_millis(1)
        ));
        assert!(!ap.shelved.held(&held, answered + CORPSE_IN_USE_AGAIN));

        // The killer's for now: a short wait, because a corpse becomes
        // everyone's the moment whoever has it closes it and not only
        // when it half rots.
        ap.take_up_corpse(locked, t0, LOOT_TIMEOUT);
        assert!(ap.corpse_refused(&not_ours, name, Some(answered), answered));
        assert_eq!(ap.corpse, None);
        assert_eq!(ap.shelved.waited(&locked), Some(CORPSE_NOT_OURS_AGAIN));
        assert!(ap.shelved.held(
            &locked,
            answered + CORPSE_NOT_OURS_AGAIN - Duration::from_millis(1)
        ));
        assert!(!ap.shelved.held(&locked, answered + CORPSE_NOT_OURS_AGAIN));

        // A body refused twice, in two different sets of words, waits
        // longer the second time -- and the wait it is given is a wait
        // and not a deadline. Handing the shelf an exact "come back in
        // 117 seconds" doubled the three seconds already on the body
        // instead: six, then twelve, then twenty-four, five more asks
        // and five more refusals before it reached the wait that was
        // meant the first time.
        let both = 0x8000_1224;
        ap.corpse_seen.push((both, t0));
        ap.take_up_corpse(both, t0, LOOT_TIMEOUT);
        assert!(ap.corpse_refused(&in_use, name, Some(answered), answered));
        assert_eq!(ap.shelved.waited(&both), Some(CORPSE_IN_USE_AGAIN));
        let again = answered + CORPSE_IN_USE_AGAIN;
        ap.take_up_corpse(both, again, LOOT_TIMEOUT);
        let told = again + Duration::from_millis(360);
        assert!(ap.corpse_refused(&not_ours, name, Some(told), told));
        assert_eq!(
            ap.shelved.waited(&both),
            Some(CORPSE_IN_USE_AGAIN * 2),
            "a doubling wait, not a deadline the shelf then doubled"
        );

        // The killer's for good: not waited on at all.
        ap.take_up_corpse(rare, t0, LOOT_TIMEOUT);
        let words =
            format!("You may not loot the {name} because the {name} has generated a rare item.");
        assert!(ap.corpse_refused(&words, name, Some(answered), answered));
        assert!(ap.shelved.held(&rare, t0 + CORPSE_LIFE));
    }

    #[test]
    fn a_refusal_about_another_body_leaves_the_one_in_hand_alone() {
        let t0 = Instant::now();
        let body = 0x8000_1221;
        let asked = t0 + Duration::from_secs(1);
        let answered = asked + Duration::from_millis(360);
        let mut ap = Autoplay {
            corpse_seen: vec![(body, t0)],
            ..Default::default()
        };
        ap.take_up_corpse(body, asked, LOOT_TIMEOUT);

        // Nine characters stand in one huddle and the server answers all
        // of them: words about a body this character is not working
        // change nothing.
        assert!(!ap.corpse_refused(
            "The Corpse of Drudge Slave is already in use by someone else!",
            "Corpse of Hellion",
            Some(answered),
            answered,
        ));
        // Nor do words about something that is not a refusal.
        assert!(!ap.corpse_refused(
            "You're too busy",
            "Corpse of Hellion",
            Some(answered),
            answered
        ));
        // Nor an answer that came in before this ask went out: it was
        // the ask before it that was refused.
        let words = "You do not yet have the right to loot the Corpse of Hellion.";
        assert!(!ap.corpse_refused(words, "Corpse of Hellion", Some(asked), answered));
        assert!(!ap.corpse_refused(words, "Corpse of Hellion", None, answered));
        assert_eq!(
            ap.corpse.map(|c| c.0),
            Some(body),
            "let a body go for nothing"
        );
        assert!(ap.shelved.is_empty(), "set a body aside for nothing");

        // The same words, stamped after the ask, are this body's.
        assert!(ap.corpse_refused(words, "Corpse of Hellion", Some(answered), answered));
        assert_eq!(ap.corpse, None);
    }

    #[test]
    fn no_body_waits_on_a_character_the_server_will_hand_nothing() {
        // +Verity, 36462 carried of a 7500 capacity, on her way to sell:
        // the looting walked her to a corpse for a Pyreal, the server said
        // "You are too encumbered to carry that!", and the walk to town
        // was lost. Past the wall no body is owed, so none is walked to.
        let t0 = Instant::now();
        let ap = Autoplay::default();
        let (me, at) = (glam::Vec3::ZERO, glam::Vec3::new(5.0, 0.0, 0.0));
        let body = 0x8000_9001;
        let walled = Room {
            past_the_wall: true,
            ..Room::PLENTY
        };
        assert!(ap.corpse_owed(body, at, me, t0, Room::PLENTY));
        assert!(!ap.corpse_waiting(body, t0, walled));
        assert!(!ap.corpse_owed(body, at, me, t0, walled));
        // Short of it, a character with no room left for loot still goes
        // to a body: coins weigh nothing, and light things may fit.
        let laden = Room {
            carry: 0,
            ..Room::PLENTY
        };
        assert!(ap.corpse_owed(body, at, me, t0, laden));
    }

    #[test]
    fn a_body_reached_with_a_full_pack_is_set_aside_and_owed_again_after_the_sale() {
        // A pack down to the slots kept for a counter's money: every body
        // in reach was walked to, shut on the spot as done with -- so
        // written off -- and the town run waited behind them. After the
        // sale none of them was gone back to, with minutes left on them.
        let t0 = Instant::now();
        let mut ap = Autoplay::default();
        let (me, at) = (glam::Vec3::ZERO, glam::Vec3::new(5.0, 0.0, 0.0));
        let body = 0x8000_8001;
        let full = Room {
            pack_low: true,
            ..Room::PLENTY
        };
        // No room, so no body is owed: none is walked to, and the fight is
        // not held for one.
        assert!(ap.corpse_owed(body, at, me, t0, Room::PLENTY));
        assert!(!ap.corpse_owed(body, at, me, t0, full), "owed with no room");
        // One already open when the pack filled is shut and set aside.
        ap.take_up_corpse(body, t0, LOOT_TIMEOUT);
        let mut open = corpse_at_hand(body, 1);
        open.slots_free = 3;
        open.room_anywhere = 3;
        open.keep_free = 3;
        let next = ap.loot_run.step(&open, t0);
        assert_eq!(next.act, Some(ac_loot::Act::Close), "{}", next.saying);
        ap.corpse_shut(
            body,
            &next.did,
            next.left_for_weight,
            ShutFor::default(),
            t0,
        );
        assert!(!ap.looted.contains(&body), "written off for good");
        // Sold down a few minutes later, it is owed again.
        let sold = t0 + Duration::from_secs(4 * 60);
        assert!(
            ap.corpse_owed(body, at, me, sold, Room::PLENTY),
            "never gone back to"
        );
    }

    #[test]
    fn a_corpse_that_says_no_again_waits_twice_as_long() {
        // The looting tidied away each lapsed wait before choosing a
        // corpse, and a corpse is chosen again exactly when its wait is
        // up. So every refusal was the first: thirty seconds, never more,
        // and the character went back every half minute until it rotted.
        use crate::did::Did;
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let mut ap = Autoplay::default();
        let (locked, rotted) = (0x8000_2101, 0x8000_2102);
        let no = Did::blocked("it will not open yet");
        ap.shelved.note(locked, &no, t0);
        ap.shelved.note(rotted, &no, t0);
        // Half a minute on, what `autoplay_loot` does before it chooses:
        // one body still lies there and the other has gone.
        let again = t0 + s(31);
        ap.forget_corpses_gone(|g| g == locked);
        assert_eq!(ap.shelved.len(), 1, "the rotted body is still remembered");
        assert!(
            ap.corpse_waiting(locked, again, Room::PLENTY),
            "never tried again"
        );
        // Chosen, and it says no again: a minute this time.
        ap.shelved.note(locked, &no, again);
        assert!(
            ap.shelved.held(&locked, again + s(59)),
            "back after thirty seconds again"
        );
        assert!(!ap.shelved.held(&locked, again + s(60)));
    }

    #[test]
    fn a_body_left_for_its_weight_waits_on_room_not_on_a_clock() {
        // Laden, a character shut every body with something heavy on it as
        // too laden and set it aside for half a minute. As each wait ran
        // out it went back, opened the body, took nothing and shut it
        // again, holding the next fight for it every time, for as long as
        // the body lay there.
        use crate::did::Did;
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let mut ap = Autoplay::default();
        let (me, at) = (glam::Vec3::ZERO, glam::Vec3::new(5.0, 0.0, 0.0));
        let (body, rotted) = (0x8000_9001, 0x8000_9002);
        let mut open = corpse_at_hand(body, 1);
        open.items[0].burden = 300;
        open.carry_room = 40;
        ap.take_up_corpse(body, t0, LOOT_TIMEOUT);
        let next = ap.loot_run.step(&open, t0);
        assert_eq!(next.act, Some(ac_loot::Act::Close), "{}", next.saying);
        ap.corpse_shut(
            body,
            &next.did,
            next.left_for_weight,
            ShutFor::default(),
            t0,
        );
        assert!(!ap.looted.contains(&body), "written off for good");
        // Still forty short: not owed, however long it has waited.
        let laden = Room {
            carry: 40,
            ..Room::PLENTY
        };
        assert!(!ap.corpse_owed(body, at, me, t0 + s(31), laden));
        assert!(!ap.corpse_owed(body, at, me, t0 + s(4 * 60), laden));
        // It is what the character weighs its room against to count itself
        // laden, but only while the body is still lying there.
        ap.left_for_weight.insert(rotted, 20);
        assert_eq!(ap.lightest_left_for_weight(|g| g == body), Some(300));
        // Sold down: owed again at once.
        let sold = Room {
            carry: 4_000,
            ..Room::PLENTY
        };
        assert!(
            ap.corpse_owed(body, at, me, t0 + s(4 * 60), sold),
            "never gone back to"
        );
        // Emptied then, it is done with, and waits on nothing.
        ap.corpse_shut(body, &Did::Done, None, ShutFor::default(), t0 + s(4 * 60));
        assert_eq!(ap.lightest_left_for_weight(|g| g == body), None);
        // A body that has rotted is forgotten.
        ap.forget_corpses_gone(|g| g == body);
        assert!(ap.left_for_weight.is_empty());
    }

    #[test]
    fn every_body_opened_is_counted_for_the_panel_however_it_was_let_go() {
        // Blargerton's log said "emptied" whether he took something or
        // nothing. The panel's count tells the two apart, and a body given
        // up on still counts for what came off it.
        use crate::did::Did;
        let t0 = Instant::now();
        let mut ap = Autoplay::default();
        // Opened, the dagger asked for, then given up on.
        let first = 0x8000_3001;
        ap.take_up_corpse(first, t0, LOOT_TIMEOUT);
        let next = ap.loot_run.step(&corpse_at_hand(first, 1), t0);
        assert_eq!(next.act, Some(ac_loot::Act::Take(1)), "{}", next.saying);
        ap.let_go_of_corpse();
        // Shut by the rules with nothing worth taking on it.
        let second = 0x8000_3002;
        ap.take_up_corpse(second, t0, LOOT_TIMEOUT);
        let mut bare = corpse_at_hand(second, 2);
        bare.items[0].verdict = ac_loot::Verdict::Leave;
        let next = ap.loot_run.step(&bare, t0);
        assert_eq!(next.did, Did::Done, "{}", next.saying);
        ap.corpse_shut(
            second,
            &next.did,
            next.left_for_weight,
            ShutFor::default(),
            t0,
        );
        // Walked to and never opened, then the next body taken up.
        let third = 0x8000_3003;
        ap.take_up_corpse(third, t0, LOOT_TIMEOUT);
        let mut unopened = corpse_at_hand(third, 3);
        unopened.open = false;
        ap.loot_run.step(&unopened, t0);
        ap.take_up_corpse(0x8000_3004, t0, LOOT_TIMEOUT);
        assert_eq!(
            ap.loot_tally,
            ac_loot::Tally {
                opened: 2,
                taken: 1
            }
        );
    }

    #[test]
    fn an_ask_at_a_corpse_the_server_let_go_comes_back_with_nothing() {
        // Blargerton walked back to bodies ACE had let go while he was out
        // of sight of them, and asked at them for the rest of the session.
        let asked = Instant::now();
        let ms = Duration::from_millis;
        let (underfoot, done) = (CORPSE_REACH / 2.0, Some((0, asked + ms(80))));
        // Not there: done, no error, and not a word.
        assert!(answered_with_nothing(underfoot, asked, done, None));
        // A word from before the ask was about something else.
        assert!(answered_with_nothing(underfoot, asked, done, Some(asked)));
        // Locked to whoever killed it, or open to someone else: the server
        // says so before it is done.
        assert!(!answered_with_nothing(
            underfoot,
            asked,
            done,
            Some(asked + ms(40))
        ));
        // Too busy is a refusal, not an absence.
        assert!(!answered_with_nothing(
            underfoot,
            asked,
            Some((0x1D, asked + ms(80))),
            None
        ));
        // No answer yet, or only one that came in on the tick the ask went
        // out, which was to an earlier ask.
        assert!(!answered_with_nothing(underfoot, asked, None, None));
        assert!(!answered_with_nothing(
            underfoot,
            asked,
            Some((0, asked)),
            None
        ));
        // From across the room the server walks the character over first,
        // and a walk it cannot finish ends just as quietly.
        assert!(!answered_with_nothing(
            CORPSE_REACH * 4.0,
            asked,
            done,
            None
        ));
    }

    #[test]
    fn a_corpse_is_taken_for_gone_only_when_every_ask_at_it_came_back_with_nothing() {
        let t0 = Instant::now();
        let mut ap = Autoplay::default();
        // What `autoplay_loot` does as each ask's wait runs out: note how
        // it came back, then ask again until the tries are used up. What
        // is said after the last ask decides what becomes of the body.
        let ask_until_given_up = |ap: &mut Autoplay, guid: u32, quiet: &[bool]| {
            ap.take_up_corpse(guid, t0, LOOT_TIMEOUT);
            let mut gone = false;
            for (tries, q) in (0..).zip(quiet) {
                gone = ap.nothing_came_back(*q);
                ap.let_go_of_corpse();
                if tries < LOOT_TRIES {
                    ap.corpse = Some((guid, t0, LOOT_TIMEOUT, tries + 1));
                }
            }
            gone
        };
        let every = [true; LOOT_TRIES as usize + 1];
        assert!(
            ask_until_given_up(&mut ap, 0x8000_4001, &every),
            "a body that is not there is set aside to be asked at again"
        );
        // One ask that said something, or had no answer at all, and the
        // body is only set aside, as before. Nor does the last body's
        // count run on into this one.
        let mut once = every;
        once[0] = false;
        assert!(
            !ask_until_given_up(&mut ap, 0x8000_4002, &once),
            "a body that answered once is forgotten"
        );
        // Nothing in hand, nothing to say.
        assert!(!ap.nothing_came_back(true));
    }

    #[test]
    fn config_round_trips_through_json() {
        let mut c = Config {
            enabled: true,
            ..Config::default()
        };
        c.buffs.spells = vec!["Strength Self".into()];
        c.fight.avoid = vec!["Olthoi".into()];
        let text = serde_json::to_string(&c).unwrap();
        let back: Config = serde_json::from_str(&text).unwrap();
        assert_eq!(back, c);
        // Missing fields fall back to the defaults.
        let partial: Config = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert!(partial.enabled);
        assert_eq!(partial.survive.heal_below, Survive::default().heal_below);
        assert_eq!(Doing::Fighting.label(), "fighting");
        // A settings file written when the heal spell was still a
        // setting keeps loading, rest and all: the heal is chosen by
        // the rules now, and a name left in the file is no reason to
        // throw a player's whole configuration away.
        let old: Config =
            serde_json::from_str(r#"{"survive":{"heal_spell":"Heal Self VI","heal_below":0.45}}"#)
                .unwrap();
        assert_eq!(old.survive.heal_below, 0.45);
        assert!(old.survive.use_kits);
        // And one written before critters were walked past walks past
        // them: a bool left out would otherwise read as off. The same
        // goes for walking past what stands on the road.
        let before: Config = serde_json::from_str(r#"{"fight":{"radius":30.0}}"#).unwrap();
        assert!(before.fight.skip_critters);
        assert!(before.fight.walk_past_on_the_way);
    }

    /// A character with a mace in hand and a wand in the pack, and
    /// nothing the server is waiting on: no swing out, no spell in the
    /// air.
    fn hands_free() -> Option<Client> {
        let (mut c, _) = mid_fight()?;
        c.attack_target = None;
        c.attack_pending = false;
        c.autoplay.cast_sent = None;
        assert!(!c.server_busy(Instant::now()), "nothing owed to the server");
        Some(c)
    }

    #[test]
    fn the_weapon_already_in_the_hand_is_not_asked_for_again() {
        // All 71 "You must remove your X to wield Y" lines of a
        // ten-minute run had X and Y the same item: the client asking
        // the server to wield what it was already holding.
        let Some(mut c) = hands_free() else {
            return;
        };
        const MACE: u32 = 0x8000_0101;
        assert!(c.wield_guid(MACE), "the mace is in hand, so the ask stands");
        assert_eq!(c.autoplay.wield_asked, None, "and nothing was sent");
    }

    #[test]
    fn one_wield_goes_out_once_however_often_it_is_asked_for() {
        let Some(mut c) = hands_free() else {
            return;
        };
        const WAND: u32 = 0x8000_0102;
        assert!(c.wield_guid(WAND), "the wand is asked for");
        let first = c.autoplay.wield_asked;
        assert!(matches!(first, Some((WAND, _))));
        // The client thinks at 8 Hz and the answer takes a few hundred
        // milliseconds. Three ticks of asking used to be three sends,
        // and the server refused the last two for the first having
        // worked.
        assert!(c.wield_guid(WAND), "the ask already stands");
        assert!(c.wield_guid(WAND));
        assert_eq!(c.autoplay.wield_asked, first, "nothing else was sent");
        // A wield the server never answers at all is asked for again.
        c.autoplay.wield_asked = Some((WAND, Instant::now() - WIELD_ANSWERS_IN));
        assert!(c.wield_guid(WAND));
        assert_ne!(c.autoplay.wield_asked, first, "asked again once stale");
    }

    #[test]
    fn a_refused_wield_that_worked_forgets_the_wait_rather_than_doubling_it() {
        let Some(mut c) = hands_free() else {
            return;
        };
        const WAND: u32 = 0x8000_0102;
        let now = Instant::now();
        // The wand was asked for twice and taken up once. The second
        // ask comes back refused, and the world already shows the wand
        // in hand.
        a_weapon(&mut c, WAND, ac_world::item_type::CASTER, "Wand", true);
        c.autoplay.wield_asked = Some((WAND, now));
        c.wield_refused(WAND, now);
        assert!(
            !c.wield_held_off(WAND),
            "the wield worked; there is nothing to wait for"
        );
        assert_eq!(c.autoplay.wield_asked, None, "and the ask is answered");

        // A refusal for something still in the pack is a real refusal
        // and still earns its wait.
        a_weapon(&mut c, WAND, ac_world::item_type::CASTER, "Wand", false);
        c.autoplay.wield_asked = Some((WAND, now));
        c.wield_refused(WAND, now);
        assert!(c.wield_held_off(WAND), "left alone for a while");
    }

    #[test]
    fn nothing_is_sent_to_move_an_item_while_the_server_has_us_busy() {
        // ACE refuses a take made while the character is busy and spends
        // two messages saying so -- YoureTooBusy and an
        // InventoryServerSaveFailed with nothing in it. Nine characters
        // bought 89 of those pairs in ten minutes, taking from a body
        // with a spell in the air.
        let Some(mut c) = hands_free() else {
            return;
        };
        const WAND: u32 = 0x8000_0102;
        const LOOT: u32 = 0x8000_0105;
        let now = Instant::now();
        c.autoplay.cast_sent = Some(now);
        assert!(c.server_busy(now));

        assert!(!c.wield_guid(WAND), "the wand waits for a free tick");
        assert_eq!(c.autoplay.wield_asked, None, "nothing sent mid-cast");

        c.loot_queue.push_back(LOOT);
        c.tick_loot(now);
        assert!(c.loot_inflight.is_none(), "the take waits too");
        assert_eq!(
            c.loot_queue.front(),
            Some(&LOOT),
            "and keeps its place in the queue"
        );

        // A swing in the air is a different matter. ACE sets no IsBusy
        // for one -- nothing in its melee or missile path does, and the
        // wield handler does not read it at all -- so the take goes out.
        // The wield still waits, because a wield mid-swing lands and
        // cancels the swing doing it.
        c.autoplay.cast_sent = None;
        c.attack_pending = true;
        c.last_attack = now;
        assert!(!c.server_busy(now), "a swing is not the server being busy");
        assert!(c.wield_must_wait(WAND, now), "not worth a cancelled swing");
        assert!(!c.wield_guid(WAND));
        c.tick_loot(now);
        assert_eq!(
            c.loot_inflight.map(|(g, _)| g),
            Some(LOOT),
            "the take was held back for a rule the server does not have"
        );
        assert!(c.loot_queue.is_empty());

        // The swing lands, and the wand goes out too.
        c.attack_pending = false;
        assert!(c.wield_guid(WAND));
    }

    /// A Sack hanging from the main pack, with `capacity` slots.
    const SACK: u32 = 0x8000_0300;
    /// A body at the character's feet.
    const BODY: u32 = 0x8000_0400;

    /// A character whose main pack has `main_slots` slots, `main_used`
    /// of them taken by daggers, and a Sack of `sack_slots` slots with
    /// `sack_used` daggers in it.
    fn with_packs(
        main_slots: u32,
        main_used: u32,
        sack_slots: u32,
        sack_used: u32,
    ) -> Option<Client> {
        let mut c = character_of_level(20)?;
        let me = c.world.player_guid.unwrap();
        c.world.objects.insert(
            me,
            ac_world::WorldObject {
                guid: me,
                name: "Verity".into(),
                is_player: true,
                items_capacity: main_slots,
                ..Default::default()
            },
        );
        c.world.objects.insert(
            SACK,
            ac_world::WorldObject {
                guid: SACK,
                name: "Sack".into(),
                weenie_class_id: 166,
                item_type: ac_world::item_type::CONTAINER,
                items_capacity: sack_slots,
                container: Some(me),
                ..Default::default()
            },
        );
        let mut next = 0x8000_0500;
        for (holder, n) in [(me, main_used), (SACK, sack_used)] {
            for _ in 0..n {
                c.world.objects.insert(
                    next,
                    ac_world::WorldObject {
                        guid: next,
                        name: "Dagger".into(),
                        container: Some(holder),
                        ..Default::default()
                    },
                );
                next += 1;
            }
        }
        c.world.objects.insert(
            BODY,
            ac_world::WorldObject {
                guid: BODY,
                name: "Corpse of a Drudge Skulker".into(),
                object_desc_flags: ac_world::object_desc_flags::CORPSE,
                ..Default::default()
            },
        );
        // No slots kept back for a counter's money: these are about the
        // packs being full, not low.
        c.autoplay.config.team.restock.keep_slots = 0;
        assert!(!c.server_busy(Instant::now()));
        Some(c)
    }

    /// A thing of `wcid` lying on the body, `count` to the stack.
    fn on_the_body(c: &mut Client, guid: u32, name: &str, wcid: u32, kind: u32, count: u32) {
        c.world.objects.insert(
            guid,
            ac_world::WorldObject {
                guid,
                name: name.into(),
                weenie_class_id: wcid,
                item_type: kind,
                stack_size: count,
                max_stack_size: if count > 1 { 25_000 } else { 1 },
                container: Some(BODY),
                ..Default::default()
            },
        );
        match &mut c.world.open_container {
            Some((body, items)) if *body == BODY => items.push(guid),
            _ => c.world.open_container = Some((BODY, vec![guid])),
        }
    }

    #[test]
    fn free_space_is_what_one_take_can_use_and_not_the_sum_over_the_packs() {
        // Main pack 2 of 4, Sack 19 of 24: seventeen free in all, and
        // five for any one take.
        let Some(c) = with_packs(4, 2, 24, 19) else {
            return;
        };
        let me = c.world.player_guid.unwrap();
        assert_eq!(c.free_space(), 5);
        assert_eq!(c.room_anywhere(), 7);
        assert!(!c.pack_full());
        let packs = c.packs();
        assert_eq!((packs.main.capacity, packs.main.used), (4, 2));
        assert_eq!(packs.side.len(), 1);
        assert_eq!((packs.side[0].guid, packs.side[0].used), (SACK, 19));
        // The Sack sits in a pack slot, not an item slot.
        assert_eq!(packs.container_for_a_take(), Some(me));
        // Every pack down to its last slot: room for one take, however
        // many packs there are.
        let Some(mut c) = with_packs(4, 3, 24, 23) else {
            return;
        };
        assert_eq!(c.free_space(), 1);
        assert_eq!(c.room_anywhere(), 2);
        // A character with an empty main pack reads as it always did.
        c.world
            .objects
            .retain(|_, o| o.name != "Dagger" && o.guid != SACK);
        assert_eq!(c.free_space(), 4);
        assert_eq!(c.room_anywhere(), 4);
    }

    #[test]
    fn a_take_with_the_main_pack_full_goes_into_the_sack_with_room() {
        // Nine characters at 102/102 with a Sack at 7 of 24 aimed every
        // take at the main pack and were refused seven thousand times.
        let Some(mut c) = with_packs(2, 2, 24, 7) else {
            return;
        };
        let me = c.world.player_guid.unwrap();
        const DAGGER: u32 = 0x8000_0401;
        on_the_body(&mut c, DAGGER, "Dagger", 21, 0, 1);
        assert_eq!(c.free_space(), 17);
        assert_eq!(c.how_to_take(DAGGER), Some(crate::room::Take::Put(SACK)));
        let now = Instant::now();
        c.take(DAGGER);
        c.tick_loot(now);
        let sent = c.loot_sent.clone().expect("a take went out");
        assert_eq!((sent.item, sent.into, sent.retried), (DAGGER, SACK, false));
        assert!(c.loot_merge.is_none(), "a put, not a pour");
        // Landed in the Sack: ours, though not in our own container.
        c.world.objects.get_mut(&DAGGER).unwrap().container = Some(SACK);
        c.tick_loot(now + Duration::from_millis(100));
        assert!(c.loot_inflight.is_none(), "the take is over");
        assert!(c.loot_sent.is_some(), "kept until the next take goes out");
        // With a slot in the main pack, that comes first, as ever.
        c.world.objects.get_mut(&me).unwrap().items_capacity = 4;
        assert_eq!(c.how_to_take(DAGGER), Some(crate::room::Take::Put(me)));
    }

    #[test]
    fn with_every_pack_full_the_body_is_set_aside_once_and_not_reopened() {
        let Some(mut c) = with_packs(2, 2, 3, 3) else {
            return;
        };
        const DAGGER: u32 = 0x8000_0401;
        on_the_body(&mut c, DAGGER, "Dagger", 21, 0, 1);
        let t0 = Instant::now();
        assert_eq!(c.free_space(), 0);
        let room = c.room_for_loot();
        assert!(room.pack_low, "no pack has a slot");
        // The body at its feet is not waited on, and the rules shut one
        // already open as full, setting it aside rather than writing
        // it off.
        assert!(!c.autoplay.corpse_waiting(BODY, t0, room));
        let profile = crate::profile::Profile {
            rules: vec![asks("daggers", "dagger", LootAction::Sell)],
            ..Default::default()
        };
        c.autoplay.take_up_corpse(BODY, t0, LOOT_TIMEOUT);
        let open = c.corpse_now(BODY, &[DAGGER], &profile, t0, t0);
        assert_eq!(open.slots_free, 0);
        let next = c.autoplay.loot_run.step(&open, t0);
        assert_eq!(next.act, Some(ac_loot::Act::Close), "{}", next.saying);
        assert!(matches!(&next.did, crate::did::Did::Blocked(b) if b.what == "the pack is full"));
        c.autoplay.corpse_shut(
            BODY,
            &next.did,
            next.left_for_weight,
            ShutFor::default(),
            t0,
        );
        assert!(!c.autoplay.looted.contains(&BODY), "written off for good");
        // Set aside for room: with none, it is not gone back to however
        // long it lies there.
        let later = t0 + Duration::from_secs(10 * 60);
        assert!(!c.autoplay.corpse_waiting(BODY, later, c.room_for_loot()));
        // Room appears -- a dagger sold -- and it waits again.
        c.world.objects.remove(&0x8000_0500);
        let room = c.room_for_loot();
        assert!(!room.pack_low);
        assert!(c.autoplay.corpse_waiting(BODY, later, room));
    }

    #[test]
    fn the_servers_word_that_a_pack_is_full_sends_the_take_elsewhere_and_then_gives_up() {
        // The count said the main pack had two slots; the server said
        // "Unable to put Dagger into container". Believed, the next try
        // names the Sack; refused there too, the body is left for room.
        let Some(mut c) = with_packs(4, 2, 24, 23) else {
            return;
        };
        let me = c.world.player_guid.unwrap();
        const DAGGER: u32 = 0x8000_0401;
        on_the_body(&mut c, DAGGER, "Dagger", 21, 0, 1);
        let t0 = Instant::now();
        c.take(DAGGER);
        c.tick_loot(t0);
        assert_eq!(c.loot_sent.as_ref().map(|s| s.into), Some(me));
        // In the order a tick reads them: the InventoryServerSaveFailed
        // with no reason in it first, with the words noted as said but
        // not yet read; then the words themselves, from the chat.
        let answered = t0 + Duration::from_millis(50);
        c.told = Some(answered);
        c.move_refused.insert(DAGGER, (0, answered));
        c.loot_inflight = None;
        c.take_refused(DAGGER, 0);
        assert!(
            c.packs_said_full.is_empty(),
            "something was said, and not read yet"
        );
        // Words about some other take are not about this one.
        c.hear_put_refusal("Unable to put Pyreal into container");
        assert!(c.packs_said_full.is_empty());
        c.hear_put_refusal("Unable to put Dagger into container");
        assert_eq!(
            c.packs_said_full.get(&me),
            Some(&2),
            "full at two, whatever the count said"
        );
        assert!(c.loot_sent.is_none(), "judged, and not read twice");
        assert_eq!(c.free_space(), 1, "the Sack's slot is all there is");
        assert_eq!(c.packs().container_for_a_take(), Some(SACK));
        assert_eq!(c.loot_queue.front(), Some(&DAGGER), "sent once more");
        assert_eq!(c.loot_retry, Some(DAGGER));
        assert!(
            !c.move_refused.contains_key(&DAGGER),
            "the refusal was the pack's, not the item's"
        );
        c.tick_loot(answered + Duration::from_millis(10));
        let sent = c.loot_sent.clone().expect("the second try");
        assert_eq!((sent.into, sent.retried), (SACK, true));
        // Refused there as well, in the same words.
        let again = answered + Duration::from_millis(60);
        c.move_refused.insert(DAGGER, (0, again));
        c.loot_inflight = None;
        c.take_refused(DAGGER, 0);
        c.hear_put_refusal("Unable to put Dagger into container");
        assert_eq!(c.packs_said_full.get(&SACK), Some(&23));
        assert_eq!(c.free_space(), 0);
        assert!(c.loot_queue.is_empty(), "no third try");
        assert!(
            c.move_refused.contains_key(&DAGGER),
            "the rules read the refusal"
        );
        assert!(c.room_for_loot().pack_low, "the body is set aside for room");
        // Something leaves the main pack, and the server's word on it
        // lapses: the count is believed again until the server says
        // otherwise.
        c.world.objects.remove(&0x8000_0500);
        assert_eq!(c.free_space(), 3);
        assert_eq!(c.packs().container_for_a_take(), Some(me));
        assert!(
            !c.room_for_loot().pack_low,
            "room appeared; bodies wait again"
        );
    }

    #[test]
    fn a_refusal_with_no_reason_and_no_word_at_all_is_not_the_pack() {
        // A second of a unique: the server refuses it with no code, and
        // explains itself in the system chat, which is not a word about
        // the take. Read as the pack being full, it marked the main
        // pack, then the Sack, and sent the character to town with
        // seventeen slots free.
        let Some(mut c) = with_packs(4, 2, 24, 7) else {
            return;
        };
        let me = c.world.player_guid.unwrap();
        const KEY: u32 = 0x8000_0401;
        on_the_body(&mut c, KEY, "Sturdy Iron Key", 9000, 0, 1);
        let t0 = Instant::now();
        c.take(KEY);
        c.tick_loot(t0);
        assert_eq!(c.loot_sent.as_ref().map(|s| s.into), Some(me));
        let answered = t0 + Duration::from_millis(50);
        c.move_refused.insert(KEY, (0, answered));
        c.loot_inflight = None;
        c.take_refused(KEY, 0);
        // And nothing more is said. The next tick finds nothing to send
        // and nothing to mark.
        c.tick_loot(answered);
        assert!(c.packs_said_full.is_empty(), "no word: not the pack");
        assert!(c.loot_queue.is_empty(), "and no second try");
        assert!(c.loot_inflight.is_none(), "no take in the air");
        assert!(
            c.move_refused.contains_key(&KEY),
            "the refusal was the item's, for the rules to read"
        );
        assert_eq!(c.free_space(), 17);
        assert!(!c.room_for_loot().pack_low);
    }

    #[test]
    fn the_servers_words_count_when_they_come_a_packet_behind_the_refusal() {
        // The refusal in one packet, the words in the next: the tick
        // between has read the take as over. What was sent is kept
        // until the next goes out, so the words still find it.
        let Some(mut c) = with_packs(4, 2, 24, 7) else {
            return;
        };
        let me = c.world.player_guid.unwrap();
        const DAGGER: u32 = 0x8000_0401;
        on_the_body(&mut c, DAGGER, "Dagger", 21, 0, 1);
        let t0 = Instant::now();
        c.take(DAGGER);
        c.tick_loot(t0);
        let answered = t0 + Duration::from_millis(50);
        c.move_refused.insert(DAGGER, (0, answered));
        c.loot_inflight = None;
        c.take_refused(DAGGER, 0);
        c.tick_loot(answered);
        assert!(c.loot_inflight.is_none());
        assert!(c.loot_sent.is_some(), "kept for the words");
        c.hear_put_refusal("Unable to put Dagger into container");
        assert_eq!(c.packs_said_full.get(&me), Some(&2));
        assert_eq!(c.loot_queue.front(), Some(&DAGGER), "sent once more");
    }

    #[test]
    fn the_servers_word_lifts_when_one_thing_leaves_however_the_count_climbed() {
        // Called full when the client had counted two; two descriptions
        // still on their way arrive, and the count reads four. One
        // sold: the word held at two would still stand, and the pack
        // would read as full until a third left.
        let Some(mut c) = with_packs(4, 2, 24, 24) else {
            return;
        };
        let me = c.world.player_guid.unwrap();
        c.packs_said_full.insert(me, 2);
        let t0 = Instant::now();
        assert_eq!(c.free_space(), 0);
        for guid in [0x8000_0700, 0x8000_0701] {
            c.world.objects.insert(
                guid,
                ac_world::WorldObject {
                    guid,
                    name: "Dagger".into(),
                    container: Some(me),
                    ..Default::default()
                },
            );
        }
        c.tick_loot(t0);
        assert_eq!(
            c.packs_said_full.get(&me),
            Some(&4),
            "kept at the most seen"
        );
        assert_eq!(c.free_space(), 0);
        c.world.objects.remove(&0x8000_0700);
        c.tick_loot(t0 + Duration::from_millis(125));
        assert!(
            !c.packs_said_full.contains_key(&me),
            "one left: the word lapses"
        );
        assert_eq!(c.free_space(), 1);
    }

    #[test]
    fn a_pack_off_the_ground_is_picked_up_into_the_main_pack_whatever_the_room() {
        // The main pack full and the Sack with room: a dagger on the
        // ground goes into the Sack, and a Pouch into the main pack's
        // pack slots, where the server would refuse it named into the
        // Sack without a word.
        let Some(mut c) = with_packs(2, 2, 24, 7) else {
            return;
        };
        let me = c.world.player_guid.unwrap();
        const POUCH: u32 = 0x8000_0801;
        const DAGGER: u32 = 0x8000_0802;
        c.world.objects.insert(
            POUCH,
            ac_world::WorldObject {
                guid: POUCH,
                name: "Pouch".into(),
                weenie_class_id: 167,
                item_type: ac_world::item_type::CONTAINER,
                items_capacity: 12,
                position: Some(ac_world::object::Position::new_flat(0, Default::default())),
                ..Default::default()
            },
        );
        c.world.objects.insert(
            DAGGER,
            ac_world::WorldObject {
                guid: DAGGER,
                name: "Dagger".into(),
                position: Some(ac_world::object::Position::new_flat(0, Default::default())),
                ..Default::default()
            },
        );
        assert_eq!(c.where_to_pick_up(POUCH), Some(me));
        assert_eq!(c.where_to_pick_up(DAGGER), Some(SACK));
        assert_eq!(c.where_to_pick_up(0x8000_0803), None, "unknown");
    }

    #[test]
    fn a_refusal_with_a_reason_or_other_words_is_not_read_as_a_full_pack() {
        let Some(mut c) = with_packs(4, 2, 24, 7) else {
            return;
        };
        let me = c.world.player_guid.unwrap();
        const DAGGER: u32 = 0x8000_0401;
        on_the_body(&mut c, DAGGER, "Dagger", 21, 0, 1);
        let t0 = Instant::now();
        c.take(DAGGER);
        c.tick_loot(t0);
        // "You are too encumbered to carry that!" and the usual
        // reasonless refusal after it.
        let answered = t0 + Duration::from_millis(50);
        c.told = Some(answered);
        c.move_refused.insert(DAGGER, (0, answered));
        c.loot_inflight = None;
        c.take_refused(DAGGER, 0);
        assert!(c.packs_said_full.is_empty(), "the pack was not the reason");
        assert!(c.loot_queue.is_empty(), "and it is not asked for again");
        assert!(c.move_refused.contains_key(&DAGGER));
        // A quest's cap, with its code.
        c.take(DAGGER);
        c.tick_loot(answered);
        c.loot_inflight = None;
        c.take_refused(DAGGER, 0x043E);
        assert!(c.packs_said_full.is_empty());
        assert_eq!(c.free_space(), 17);
        assert_eq!(c.packs().container_for_a_take(), Some(me));
    }

    #[test]
    fn coin_off_a_body_is_poured_onto_the_pile_carried_rather_than_given_a_slot() {
        // The main pack full but for the pyreals in it: the body's
        // coin joins the pile, spending no slot, and the rules take it
        // off a body they would otherwise shut as full.
        let Some(mut c) = with_packs(2, 1, 0, 0) else {
            return;
        };
        c.world.objects.remove(&SACK);
        let me = c.world.player_guid.unwrap();
        const PILE: u32 = 0x8000_0600;
        const COIN: u32 = 0x8000_0401;
        c.world.objects.insert(
            PILE,
            ac_world::WorldObject {
                guid: PILE,
                name: "Pyreal".into(),
                weenie_class_id: 273,
                item_type: ac_world::item_type::MONEY,
                stack_size: 100,
                max_stack_size: 25_000,
                container: Some(me),
                ..Default::default()
            },
        );
        on_the_body(&mut c, COIN, "Pyreal", 273, ac_world::item_type::MONEY, 50);
        assert_eq!(c.free_space(), 0, "the pile took the last slot");
        assert_eq!(
            c.how_to_take(COIN),
            Some(crate::room::Take::Merge {
                to: PILE,
                amount: 50
            })
        );
        // The rules see it needs no slot and ask for it.
        let t0 = Instant::now();
        let profile = crate::profile::Profile {
            rules: vec![asks("coin", "pyreal", LootAction::Keep)],
            ..Default::default()
        };
        c.autoplay.take_up_corpse(BODY, t0, LOOT_TIMEOUT);
        let open = c.corpse_now(BODY, &[COIN], &profile, t0, t0);
        assert_eq!(open.slots_free, 0);
        assert!(open.items[0].needs_no_slot);
        let next = c.autoplay.loot_run.step(&open, t0);
        assert_eq!(next.act, Some(ac_loot::Act::Take(COIN)), "{}", next.saying);
        // Not while the tidying's own pour is in the air: the pile it
        // is emptying, or filling, may be the one the coin would join.
        c.take(COIN);
        c.autoplay.pour = Some((
            crate::pack::PourSent {
                merge: crate::pack::Merge {
                    from: 0x8000_0601,
                    to: PILE,
                    amount: 10,
                    name: "Pyreal".into(),
                    frees_a_slot: true,
                },
                to_before: 100,
            },
            t0,
        ));
        c.tick_loot(t0);
        assert!(c.loot_sent.is_none(), "held while the tidying pours");
        assert_eq!(c.loot_queue.front(), Some(&COIN));
        c.autoplay.pour = None;
        // The take is a pour off the body, read like a tidying pour.
        c.tick_loot(t0);
        let pour = c.loot_merge.clone().expect("a pour went out");
        assert_eq!(
            (pour.merge.from, pour.merge.to, pour.merge.amount),
            (COIN, PILE, 50)
        );
        assert_eq!(pour.to_before, 100);
        assert_eq!(c.loot_sent.as_ref().map(|s| s.into), Some(PILE));
        // The source goes first; that is no answer yet.
        c.world.objects.remove(&COIN);
        c.tick_loot(t0 + Duration::from_millis(100));
        assert!(c.loot_inflight.is_some(), "the pile has not grown");
        // Nor is the tidying's two seconds without a word: the server
        // walks to the body and stoops for this as for a take, and one
        // take in ten took longer.
        c.tick_loot(t0 + Duration::from_millis(2_500));
        assert!(c.loot_inflight.is_some(), "waited on as a take is");
        // The pile grows: landed.
        c.world.objects.get_mut(&PILE).unwrap().stack_size = 150;
        c.tick_loot(t0 + Duration::from_millis(2_600));
        assert!(c.loot_inflight.is_none());
        assert!(c.loot_merge.is_some(), "kept until the next take goes out");
        // A pour the server turns down -- too heavy, by its reckoning --
        // is over, and the refusal is left for the rules to read: it is
        // not a pack being full, so nothing is marked.
        on_the_body(&mut c, COIN, "Pyreal", 273, ac_world::item_type::MONEY, 50);
        let t1 = t0 + Duration::from_secs(1);
        c.take(COIN);
        c.tick_loot(t1);
        assert!(c.loot_merge.is_some());
        c.move_refused
            .insert(COIN, (0, t1 + Duration::from_millis(50)));
        c.take_refused(COIN, 0);
        assert!(
            c.packs_said_full.is_empty(),
            "a pour refused says nothing of the packs"
        );
        c.tick_loot(t1 + Duration::from_millis(100));
        assert!(c.loot_inflight.is_none(), "the pour is over");
        assert!(c.move_refused.contains_key(&COIN));
        // A stack too big for the pile is put in a slot, when there is
        // one, and is not a thing that needs no slot.
        c.world.objects.get_mut(&COIN).unwrap().stack_size = 25_000;
        assert_eq!(c.how_to_take(COIN), None, "no slot, and no pile it fits");
        let open = c.corpse_now(BODY, &[COIN], &profile, t0, t1);
        assert!(!open.items[0].needs_no_slot);
    }

    #[test]
    fn a_busy_tick_is_not_an_empty_quiver() {
        // The one caller of `ready_ammo` reads a false as "no
        // ammunition" and goes off to fletch some, dropping out of
        // combat stance to do it. With the busy test on every wield, a
        // shot still unanswered or a spell in the air made every tick a
        // false one, and an archer with a full pack was sent to make
        // arrows it was already carrying.
        let Some(mut c) = character_of_level(20) else {
            return;
        };
        const ARROWS: u32 = 0x8000_0201;
        let me = c.world.player_guid;
        c.world.objects.insert(
            ARROWS,
            ac_world::WorldObject {
                guid: ARROWS,
                name: "Arrow".into(),
                stack_size: 100,
                valid_locations: ac_world::equip::MISSILE_AMMO,
                container: me,
                ..Default::default()
            },
        );
        c.autoplay.wanted_ammo = Some(ARROWS);
        assert!(c.wielded_ammo().is_none(), "the slot is empty");

        // A spell in the air: the wield waits, and the quiver is still
        // not empty.
        let now = Instant::now();
        c.autoplay.cast_sent = Some(now);
        assert!(c.server_busy(now));
        assert!(c.ready_ammo(), "a busy tick read as an empty quiver");
        assert_eq!(c.autoplay.wield_asked, None, "nothing sent mid-cast");

        // The spell lands and the arrows go into the slot.
        c.autoplay.cast_sent = None;
        assert!(c.ready_ammo());
        assert_eq!(c.autoplay.wield_asked.map(|(g, _)| g), Some(ARROWS));
    }

    #[test]
    fn no_spell_is_sent_at_a_creature_that_has_already_died() {
        // 36 "Target not acquired" in a run, every one a cast at a
        // creature that had died since the tick that chose it: ACE looks
        // the target up before the windup and answers TargetNotAcquired.
        // The swing has always made this test; the cast never did.
        let Some((mut c, wand)) = mid_fight() else {
            return;
        };
        const MACE: u32 = 0x8000_0101;
        const CREATURE: u32 = 0x8000_0103;
        // Incantation of Lightning Vulnerability Other, one of the
        // softening spells the run cast.
        const VULNERABILITY: u32 = 4483;
        a_weapon(
            &mut c,
            MACE,
            ac_world::item_type::MELEE_WEAPON,
            "Mace",
            false,
        );
        a_weapon(&mut c, wand, ac_world::item_type::CASTER, "Wand", true);
        c.select(Some(CREATURE));
        assert_eq!(
            c.try_cast(VULNERABILITY),
            crate::magic::CastCheck::Ok,
            "it is alive and in view"
        );
        // Dead: the server replaces the creature with a corpse of
        // another guid, so its own simply goes.
        c.world.objects.remove(&CREATURE);
        assert_eq!(c.try_cast(VULNERABILITY), crate::magic::CastCheck::NoTarget);
        // And a cast that never went out earns no wait for an answer.
        // Refused, the server used to answer TargetNotAcquired with a
        // UseDone, which ended the wait; declined here, nothing comes
        // back at all, and the whole backstop would be spent standing
        // over a corpse the character was not allowed to take from.
        c.autoplay.cast_sent = None;
        c.cast_paced(VULNERABILITY, Instant::now());
        assert_eq!(c.autoplay.cast_sent, None, "nothing to wait for");
        assert!(!c.server_busy(Instant::now()));
    }

    #[test]
    fn an_urgent_buff_waits_for_the_fight_only_when_it_costs_the_weapon() {
        // The urgent pass runs as a reflex, ahead of loot and ahead of
        // the fight, and ignored `out_of_combat_only` outright: a buff
        // with a minute still on it was enough to put the sword away
        // mid-swing. Under god mode that cost throughput; for a mortal
        // character it is a fight fought bare-handed.
        let (sword, wand) = (false, true);
        let mut cfg = Buffs {
            never_below: 60.0,
            top_up_within: 300.0,
            out_of_combat_only: true,
            ..Buffs::default()
        };
        assert_eq!(
            buff_within(&cfg, true, true, sword),
            0.0,
            "only what has lapsed"
        );
        // A buff that is not up at all reads as nought seconds left, so
        // it still goes back up in the middle of a fight. That is what
        // "never below" is for.
        assert!(0.0 <= buff_within(&cfg, true, true, sword));
        // With a wand already in hand the recast costs a cast and
        // nothing else, so `never_below` holds. Answering nought here
        // meant a mortal caster's protections were put back only after
        // they had lapsed -- the very window the setting names.
        assert_eq!(
            buff_within(&cfg, true, true, wand),
            60.0,
            "a free recast was still made to wait for the fight"
        );
        // Out of the fight the urgent pass is unchanged, and so is the
        // quiet one either way.
        assert_eq!(buff_within(&cfg, true, false, sword), 60.0);
        assert_eq!(buff_within(&cfg, false, true, sword), 300.0);
        // And a player who has not asked for the restraint keeps the
        // old behaviour: buffs go back up mid-fight.
        cfg.out_of_combat_only = false;
        assert_eq!(buff_within(&cfg, true, true, sword), 60.0);
    }
}
#[cfg(test)]
mod heal_choice_tests {
    use super::{choose_heal, restores_health, SelfHeal, Survive, CRITICAL_HEALTH};
    use ac_world::vitals::{transfers_between, vital, Transfer};

    /// A Heal Self: a fixed number of points, drawing on nothing, and
    /// well enough learnt to land nine casts in ten.
    fn heal(spell: u32, gain: u32, mana: u32) -> SelfHeal {
        SelfHeal {
            spell,
            gain,
            mana,
            chance: 0.9,
            leaves: None,
        }
    }

    /// A transfer into health: what it gives has already been worked
    /// out against the bar it draws on, and `left` is the fraction of
    /// that bar the cast would leave behind.
    fn transfer(spell: u32, gain: u32, mana: u32, from: u32, left: f32) -> SelfHeal {
        SelfHeal {
            spell,
            gain,
            mana,
            chance: 0.9,
            leaves: Some((from, left)),
        }
    }

    /// Heal Self I through VI: every level costs more and gives more.
    fn every_level() -> Vec<SelfHeal> {
        vec![
            heal(1, 25, 10),
            heal(2, 50, 20),
            heal(3, 80, 30),
            heal(4, 110, 40),
            heal(5, 160, 55),
            heal(6, 220, 75),
        ]
    }

    #[test]
    fn a_scratch_is_not_healed_with_the_biggest_spell_in_the_book() {
        let cfg = Survive::default();
        // Twenty points off a full bar: the cheapest level that covers
        // it, not the two hundred and twenty point heal and its mana.
        assert_eq!(choose_heal(&every_level(), 20, 0.9, &cfg), Some(1));
        // A serious wound walks up the book, and stops at the first
        // level that covers it rather than going to the top.
        assert_eq!(choose_heal(&every_level(), 90, 0.5, &cfg), Some(4));
    }

    #[test]
    fn a_wound_nothing_covers_takes_the_biggest_there_is() {
        let cfg = Survive::default();
        // Three hundred missing and nothing in the book reaches it:
        // the most health one cast can give goes out.
        assert_eq!(choose_heal(&every_level(), 300, 0.4, &cfg), Some(6));
    }

    #[test]
    fn a_character_about_to_die_reaches_for_the_biggest_at_once() {
        let cfg = Survive::default();
        // A quarter of the bar left. Even a wound the cheapest heal
        // would cover gets the biggest: there may not be a second cast.
        let health = CRITICAL_HEALTH - 0.05;
        assert_eq!(choose_heal(&every_level(), 20, health, &cfg), Some(6));
    }

    #[test]
    fn a_heal_that_usually_fizzles_is_not_what_a_wound_is_covered_with() {
        let cfg = Survive::default();
        // The big heal is barely learnt and lands three casts in ten.
        let mut shaky = heal(6, 220, 75);
        shaky.chance = 0.3;
        let book = vec![heal(1, 25, 10), shaky];
        assert_eq!(choose_heal(&book, 20, 0.9, &cfg), Some(1));
        // Unless nothing else comes close, and a third of a big heal
        // is still the best there is.
        assert_eq!(choose_heal(&book, 200, 0.4, &cfg), Some(6));
    }

    #[test]
    fn a_transfer_is_not_drained_out_of_a_bar_the_character_needs() {
        let cfg = Survive::default();
        // The transfer is bigger and cheaper, but it would leave
        // stamina at a tenth, under the floor the player set, and a
        // Heal Self covers the wound on its own.
        let book = vec![
            heal(1161, 90, 30),
            transfer(1669, 300, 25, vital::STAMINA, 0.1),
        ];
        assert_eq!(choose_heal(&book, 80, 0.5, &cfg), Some(1161));
        // Mana is guarded the same way: a Mana to Health that would
        // leave nothing to cast with is passed over.
        let book = vec![
            heal(1161, 90, 30),
            transfer(1295, 300, 25, vital::MANA, 0.2),
        ];
        assert_eq!(choose_heal(&book, 80, 0.5, &cfg), Some(1161));
        // Left with room to spare, it is taken like anything else.
        let book = vec![transfer(1669, 120, 25, vital::STAMINA, 0.45)];
        assert_eq!(choose_heal(&book, 100, 0.5, &cfg), Some(1669));
    }

    #[test]
    fn a_character_whose_only_heal_is_a_transfer_still_casts_it() {
        let cfg = Survive::default();
        // Nothing else to reach for: better a bar spent than a
        // character dead, floor or no floor.
        let book = vec![transfer(1669, 300, 25, vital::STAMINA, 0.05)];
        assert_eq!(choose_heal(&book, 200, 0.5, &cfg), Some(1669));
    }

    #[test]
    fn a_dying_character_spends_the_bar_it_was_keeping() {
        let cfg = Survive::default();
        // Eight hundredths of health left and three hundred points
        // missing. The book holds a Heal Self that covers eighty of it
        // and a transfer that covers all of it but would leave stamina
        // at a seventh, under the floor. The floor is what the stamina
        // was being kept for, and there is no next fight to keep it
        // for: the transfer goes out.
        let book = vec![
            heal(3, 80, 30),
            transfer(1669, 300, 25, vital::STAMINA, 0.15),
        ];
        assert_eq!(choose_heal(&book, 300, 0.08, &cfg), Some(1669));
        // Mana is lifted the same way.
        let book = vec![heal(3, 80, 30), transfer(1295, 300, 25, vital::MANA, 0.15)];
        assert_eq!(choose_heal(&book, 300, 0.08, &cfg), Some(1295));
        // Above the line the floor still holds: a wound the Heal Self
        // covers is healed with it and the bar is left alone.
        assert_eq!(choose_heal(&book, 80, 0.5, &cfg), Some(3));
    }

    #[test]
    fn a_character_with_nothing_to_cast_heals_with_nothing() {
        // Out of mana, out of components, or no heal ever learnt: the
        // castable list comes back empty and there is no choice to make.
        assert_eq!(choose_heal(&[], 200, 0.2, &Survive::default()), None);
    }

    #[test]
    fn a_caster_really_does_know_stamina_to_health() {
        // The choice is worth nothing if the table has no such spell.
        let found: Vec<Transfer> = transfers_between(vital::STAMINA, vital::HEALTH);
        assert!(!found.is_empty(), "no stamina to health transfers");
        // The strongest moves half a bar, and on a big bar that beats
        // any fixed heal.
        let best = found.iter().map(|t| t.gain(700)).max().unwrap_or(0);
        assert!(best > 200, "the top transfer only returned {best}");
        // Half the bar goes whatever comes out the other side.
        let top = found.iter().max_by_key(|t| t.gain(700)).expect("one");
        assert_eq!(top.drain(700), 350);
        // Mana to Health is a heal too, and Harm Self is not.
        assert!(!transfers_between(vital::MANA, vital::HEALTH).is_empty());
        assert!(restores_health(1161), "Heal Self VI");
        assert!(restores_health(1669), "Stamina to Health Self VI");
        assert!(restores_health(1295), "Mana to Health Self VI");
        assert!(!restores_health(8), "Harm Self I");
        assert!(
            !restores_health(1182),
            "Revitalize Self VI restores stamina"
        );
    }
}
#[cfg(test)]
mod loot_wait_tests {
    use super::{loot_wait, LOOT_TIMEOUT};

    #[test]
    fn a_corpse_underfoot_gets_the_plain_wait() {
        assert_eq!(loot_wait(0.0), LOOT_TIMEOUT);
        // A negative distance cannot happen, but must not panic or
        // shorten the wait.
        assert_eq!(loot_wait(-5.0), LOOT_TIMEOUT);
    }

    #[test]
    fn a_corpse_across_a_room_is_given_time_to_walk_to() {
        // The bug: a flat six seconds covered a corpse at our feet and
        // not one twenty metres off, so the far ones were written off
        // unopened.
        let near = loot_wait(2.0);
        let far = loot_wait(20.0);
        assert!(far > near, "{far:?} is not longer than {near:?}");
        assert!(far > LOOT_TIMEOUT * 2, "twenty metres barely added time");
    }

    #[test]
    fn the_wait_does_not_run_away_with_itself() {
        // Whatever distance arrives, the character does not sit on a
        // corpse for ever.
        let silly = loot_wait(100_000.0);
        assert!(silly <= LOOT_TIMEOUT + std::time::Duration::from_secs(30));
    }
}

#[cfg(test)]
mod loot_timing_tests {
    use super::{CORPSE_LIFE, CORPSE_URGENT};
    use std::time::Duration;

    #[test]
    fn a_corpse_lasts_five_minutes() {
        // ACE gives an unlooted monster corpse no timer until its first
        // heartbeat, when it takes the default of five minutes. That is
        // the whole window, so it is what the rules plan against.
        assert_eq!(CORPSE_LIFE, Duration::from_secs(300));
    }

    #[test]
    fn breaking_off_a_fight_is_reserved_for_a_corpse_about_to_go() {
        // Loot keeps for minutes; the thing hitting you does not. The
        // urgency window has to be small enough that a fight is not
        // interrupted for a corpse with plenty of time left, and big
        // enough to actually reach one.
        assert!(CORPSE_URGENT < CORPSE_LIFE / 3, "too eager to break off");
        assert!(
            CORPSE_URGENT >= Duration::from_secs(30),
            "no time to get there"
        );
    }
}

#[cfg(test)]
mod fellowship_tests {
    use super::{
        next_invitee, recruit_refusal, rival_leader, Mate, RecruitRefusal, TeamView,
        HELD_OFF_FIRST, RECRUIT_AGAIN, RECRUIT_FLOOR, TEAM_OPTIONS,
    };
    use crate::did::Patience;
    use std::time::Instant;

    /// Nobody held off.
    fn nobody() -> Patience<u32> {
        Patience::new()
    }

    #[test]
    fn the_words_of_a_refusal_name_the_mate() {
        // ACE answers both in plain Broadcast chat, not a WeenieError
        // (`Entity/Fellowship.cs`, `AddFellowshipMember`).
        assert_eq!(
            recruit_refusal("+Brynlyn is already a member of a Fellowship."),
            Some(("+Brynlyn", RecruitRefusal::AlreadyAMember))
        );
        assert_eq!(
            recruit_refusal("+Brynoth is busy."),
            Some(("+Brynoth", RecruitRefusal::Busy))
        );
        assert_eq!(recruit_refusal("You are too busy."), None);
        assert_eq!(recruit_refusal("Brynoth is busy"), None);
    }

    #[test]
    fn a_recruit_answered_already_a_member_is_not_asked_again_within_the_hold() {
        // Two mates were "brought into the fellowship" ten times each and
        // never came: the server's answer was chat the client never
        // read, and `next_invitee` asked again every RECRUIT_AGAIN. Read,
        // the answer holds that mate off for a doubling wait while the
        // others are still asked.
        let start = Instant::now();
        let (lyn, oth) = (0x1001, 0x1002);
        let waiting = [(lyn, 3.0), (oth, 5.0)];
        let mut held = nobody();
        let mut asked = vec![(lyn, start)];
        held.hold(lyn, HELD_OFF_FIRST, start);
        // The other is asked meanwhile.
        assert_eq!(
            next_invitee(&waiting, &asked, &held, Some(start), start + RECRUIT_FLOOR),
            Some(oth)
        );
        asked.push((oth, start + RECRUIT_FLOOR));
        // RECRUIT_AGAIN later the held one would have been asked again;
        // now it waits out its hold.
        let again = start + RECRUIT_AGAIN;
        assert_eq!(
            next_invitee(&waiting, &asked, &nobody(), Some(start), again),
            Some(lyn)
        );
        assert_eq!(
            next_invitee(&waiting, &asked, &held, Some(start), again),
            None
        );
        let free = start + HELD_OFF_FIRST;
        assert_eq!(
            next_invitee(&waiting, &asked, &held, Some(start), free),
            Some(lyn)
        );
        // Refused again: twice the wait. (The other, long since asked,
        // is asked again meanwhile, so only this one is on the list.)
        let only_lyn = [(lyn, 3.0)];
        held.hold(lyn, HELD_OFF_FIRST, free);
        assert_eq!(
            next_invitee(&only_lyn, &asked, &held, Some(start), free + HELD_OFF_FIRST),
            None
        );
        assert_eq!(
            next_invitee(
                &only_lyn,
                &asked,
                &held,
                Some(start),
                free + HELD_OFF_FIRST * 2
            ),
            Some(lyn)
        );
    }

    #[test]
    fn a_refusal_in_words_holds_off_the_mate_it_names_and_only_one_that_was_asked() {
        let t0 = Instant::now();
        let (lyn, oth) = (0x1001, 0x1002);
        let mut ap = super::Autoplay::default();
        ap.team.mates = vec![
            Mate {
                name: "+Brynlyn".into(),
                guid: lyn,
                ..Default::default()
            },
            Mate {
                name: "+Brynoth".into(),
                guid: oth,
                ..Default::default()
            },
        ];
        ap.recruited = vec![(lyn, t0)];
        ap.hear_recruit_refusal("+Brynlyn is busy.", t0);
        assert!(ap.held_off.held(&lyn, t0 + RECRUIT_AGAIN));
        assert!(!ap.held_off.held(&lyn, t0 + HELD_OFF_FIRST));
        // The same words about a mate never invited are about something
        // else -- a patron busy with an oath says "is busy." too.
        ap.hear_recruit_refusal("+Brynoth is busy.", t0);
        assert!(!ap.held_off.held(&oth, t0));
        // And a stranger's name is nobody's.
        ap.hear_recruit_refusal("Ulgrim is already a member of a Fellowship.", t0);
        assert_eq!(ap.held_off.len(), 1);
    }

    /// An autoplay with two mates on its roster, and the first of them
    /// just invited.
    fn having_asked_lyn(t0: Instant) -> (super::Autoplay, u32, u32) {
        let (lyn, oth) = (0x1001, 0x1002);
        let mut ap = super::Autoplay::default();
        ap.team.mates = vec![
            Mate {
                name: "+Brynlyn".into(),
                guid: lyn,
                ..Default::default()
            },
            Mate {
                name: "+Brynoth".into(),
                guid: oth,
                ..Default::default()
            },
        ];
        ap.recruited = vec![(lyn, t0)];
        (ap, lyn, oth)
    }

    #[test]
    fn a_busy_mate_s_hold_does_not_outgrow_ten_seconds() {
        // The server's `fellow_busy_no_recruit` answers "is busy." for
        // a mate mid-use or mid-cast, which is over in seconds. Doubled
        // each time, a mage that happened to be casting at each of
        // eight asks was left out for hours; it is the same ten seconds
        // every time. "Already a member" is slower to change, and
        // doubles.
        let t0 = Instant::now();
        let (mut ap, lyn, _) = having_asked_lyn(t0);
        let mut now = t0;
        for _ in 0..8 {
            ap.recruited = vec![(lyn, now)];
            ap.hear_recruit_refusal("+Brynlyn is busy.", now);
            assert_eq!(ap.held_off.waited(&lyn), Some(HELD_OFF_FIRST));
            now += HELD_OFF_FIRST * 2;
        }
        ap.recruited = vec![(lyn, now)];
        ap.hear_recruit_refusal("+Brynlyn is already a member of a Fellowship.", now);
        assert_eq!(ap.held_off.waited(&lyn), Some(HELD_OFF_FIRST * 2));
    }

    #[test]
    fn a_refusal_long_after_the_invitation_is_about_something_else() {
        // The list of the invited is pruned only on the next invitation.
        // "+Brynlyn is busy." ten minutes after the last invitation is
        // a patron who could not take an oath (ACE
        // `Player_Allegiance.cs`), not an answer to it.
        let t0 = Instant::now();
        let (mut ap, lyn, _) = having_asked_lyn(t0);
        let later = t0 + RECRUIT_AGAIN * 2;
        ap.hear_recruit_refusal("+Brynlyn is busy.", later);
        assert!(!ap.held_off.held(&lyn, later));
        assert_eq!(ap.held_off.len(), 0);
        // Within the wait for an answer, it is the answer.
        let soon = t0 + RECRUIT_AGAIN / 2;
        ap.hear_recruit_refusal("+Brynlyn is busy.", soon);
        assert!(ap.held_off.held(&lyn, soon));
    }

    #[test]
    fn a_second_fellowship_is_given_up_by_the_leader_whose_name_does_not_sort_first() {
        // Two fellowships for one fleet: the one founded by the leader
        // that does not sort first goes, so the rightful leader can
        // recruit its members. The rival is read off two words that
        // have to agree -- the board's row says it is in a fellowship,
        // the world's record says not in this one.
        let me = 0x5000_0009;
        let rival = Mate {
            name: "+Brynith".into(),
            guid: 0x5000_0001,
            in_fellowship: true,
            leader: true,
            autoplay: true,
            ..Default::default()
        };
        let mine = [me, 0x5000_0005];
        let view = |m: Mate, settled: bool| TeamView {
            mates: vec![m],
            leader: false,
            settled,
            ..Default::default()
        };
        assert_eq!(
            rival_leader(&view(rival.clone(), true), &mine).map(|m| m.guid),
            Some(rival.guid)
        );
        // Not on a roster that has yet to settle: the rightful leader
        // may still be a frame away from being heard.
        assert!(rival_leader(&view(rival.clone(), false), &mine).is_none());
        // Nor while this character leads the team itself.
        let mut led = view(rival.clone(), true);
        led.leader = true;
        assert!(rival_leader(&led, &mine).is_none());
        // A rightful leader that let itself be recruited into this one
        // is a working party, not a rival.
        assert!(rival_leader(&view(rival.clone(), true), &[me, rival.guid]).is_none());
        // One in no fellowship at all is recruited, not yielded to.
        let free = Mate {
            in_fellowship: false,
            ..rival.clone()
        };
        assert!(rival_leader(&view(free, true), &mine).is_none());
        // A person standing in a fellowship of their own, with autoplay
        // off and not asking to lead, would recruit nobody: this one is
        // kept rather than given up to them.
        let person = Mate {
            autoplay: false,
            leads: false,
            ..rival.clone()
        };
        assert!(rival_leader(&view(person.clone(), true), &mine).is_none());
        let leads = Mate {
            leads: true,
            ..person
        };
        assert!(rival_leader(&view(leads, true), &mine).is_some());
    }

    #[test]
    fn every_option_a_teammate_keeps_on_is_one_the_table_knows() {
        // The options are named in words and looked up by those words,
        // so a name that no longer matches the table is an option that
        // is silently never set -- which is how nine characters hunted
        // for ten minutes without loot sharing.
        for name in TEAM_OPTIONS {
            assert!(
                crate::options::option_by_name(name).is_some(),
                "no option called {name:?}"
            );
        }
    }

    #[test]
    fn loot_sharing_is_the_option_ace_reads_off_the_founder() {
        // ACE takes `CharacterOption.ShareFellowshipLoot` (0x11, bit
        // 0x0010_0000 of the first word) off the leader in the
        // `Fellowship` constructor, and `Corpse.HasPermission` lets a
        // fellow open a fresh body through that clause alone.
        let o = crate::options::option_by_name("share fellowship loot").expect("no such option");
        assert_eq!(o.id, 0x11);
        assert_eq!(o.bit, 0x0010_0000);
        assert!(!o.inverted, "the bit means share, not ignore");
    }

    #[test]
    fn the_others_are_asked_while_one_invitee_waits() {
        // The bug: one timer for the whole party, so nine characters
        // took forty-one seconds to join, at five seconds apart. Each
        // tick now asks somebody who has not been asked yet.
        let start = Instant::now();
        let waiting = [(0x1001, 3.0), (0x1002, 5.0), (0x1003, 9.0)];
        let first = next_invitee(&waiting, &[], &nobody(), None, start).expect("nobody asked");
        assert_eq!(first, 0x1001, "the nearest is asked first");
        let asked = [(first, start)];
        let then = start + RECRUIT_FLOOR;
        assert_eq!(
            next_invitee(&waiting, &asked, &nobody(), Some(start), then),
            Some(0x1002)
        );
    }

    #[test]
    fn two_invitations_do_not_leave_in_the_same_breath() {
        // No rate limit on recruiting was found in ACE, but that is
        // read from the source and untested against a live server, so
        // the invitations keep a small gap.
        let start = Instant::now();
        let waiting = [(0x1001, 3.0), (0x1002, 5.0)];
        assert_eq!(
            next_invitee(&waiting, &[], &nobody(), Some(start), start),
            None
        );
        let soon = start + RECRUIT_FLOOR / 2;
        assert_eq!(
            next_invitee(&waiting, &[], &nobody(), Some(start), soon),
            None
        );
        assert!(
            next_invitee(&waiting, &[], &nobody(), Some(start), start + RECRUIT_FLOOR).is_some()
        );
    }

    #[test]
    fn an_invitee_that_never_came_is_asked_again_later() {
        // A member the server thought busy is refused with nothing the
        // client can act on, so the only way to tell "not yet" from
        // "never" is to ask again once the others have been asked.
        let start = Instant::now();
        let waiting = [(0x1001, 3.0)];
        let asked = [(0x1001, start)];
        let soon = start + RECRUIT_AGAIN / 2;
        assert_eq!(
            next_invitee(&waiting, &asked, &nobody(), Some(start), soon),
            None
        );
        let later = start + RECRUIT_AGAIN;
        assert_eq!(
            next_invitee(&waiting, &asked, &nobody(), Some(start), later),
            Some(0x1001)
        );
    }

    #[test]
    fn nine_are_all_asked_inside_ten_seconds() {
        // What the party is judged on: everyone in the fellowship soon
        // after they arrive, rather than the last one joining after the
        // first two minutes of every body's life have gone by.
        let start = Instant::now();
        let waiting: Vec<(u32, f32)> = (0..9).map(|i| (0x1000 + i, i as f32)).collect();
        let mut asked: Vec<(u32, Instant)> = Vec::new();
        let mut last = None;
        let mut now = start;
        while asked.len() < waiting.len() {
            match next_invitee(&waiting, &asked, &nobody(), last, now) {
                Some(guid) => {
                    asked.push((guid, now));
                    last = Some(now);
                }
                None => now += RECRUIT_FLOOR / 4,
            }
            assert!(
                now.duration_since(start) < std::time::Duration::from_secs(10),
                "only {} of nine asked in ten seconds",
                asked.len()
            );
        }
    }
}
