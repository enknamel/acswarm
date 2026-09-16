use std::time::{Duration, Instant};

use super::choose::{CORPSE_APPEARS, LOOT_NEAR};
use super::room::Room;
#[cfg(doc)]
use super::walk::CorpseWalk;
#[cfg(doc)]
use crate::autoplay::TeamView;
use crate::autoplay::{Autoplay, CLAIM_SETTLE};
use crate::{Client, SAME_FLOOR};

/// A corpse within this of where a kill fell is that kill's body.
pub(crate) const KILL_SPOT: f32 = 6.0;

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
pub(crate) fn corpse_within_reach(me: glam::Vec3, at: glam::Vec3) -> bool {
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
pub(crate) fn killed_by_us(long_desc: &str, me: &str) -> bool {
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
pub(crate) fn whose_to_ask(
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
    pub(crate) fn pet_out(&mut self, now: Instant) {
        self.pet_seen = Some(now);
    }

    /// One of its creatures has been out lately enough to have left
    /// bodies about.
    pub(crate) fn pet_lately(&self, now: Instant) -> bool {
        self.pet_seen
            .is_some_and(|t| now.duration_since(t) < PET_KILLS_FOR)
    }

    /// Whether `guid` has been asked about. Each corpse is asked about
    /// once: the answer does not change.
    pub(crate) fn has_asked(&self, guid: u32) -> bool {
        self.asked.iter().any(|(g, _, _)| *g == guid)
    }

    /// `guid` has been asked about, just now.
    pub(crate) fn ask(&mut self, guid: u32, now: Instant) {
        if !self.has_asked(guid) {
            self.asked.push((guid, now, false));
        }
    }

    /// The corpses asked about whose answer is not in.
    pub(crate) fn out(&self) -> impl Iterator<Item = u32> + '_ {
        self.asked
            .iter()
            .filter(|(_, _, done)| !done)
            .map(|(g, _, _)| *g)
    }

    /// The answer about `guid` is in.
    pub(crate) fn answered(&mut self, guid: u32) {
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
    pub(crate) fn tidy(&mut self, there: impl Fn(u32) -> bool) {
        self.asked.retain(|(g, _, _)| there(*g));
    }
}

impl Autoplay {
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
    pub(crate) fn corpse_first_seen(&self, guid: u32, now: Instant) -> Instant {
        self.corpse_seen.since(&guid).unwrap_or(now)
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
        // Dealt by the leader's plan: whoever it was dealt to opens it,
        // and the rest leave it. Only while the plan is fresh; a leader
        // gone quiet leaves the turns below to say (see `crate::plan`).
        if let Some(to) = self.body_dealt_to(guid, now) {
            return to == me;
        }
        // Newly fallen, so nobody's claim can have reached the board
        // yet: the turns say whose it is until it has.
        let seen = self.corpse_first_seen(guid, now);
        now.saturating_duration_since(seen) >= CLAIM_SETTLE
            || self.team.opens_first(guid, at, self.my_turn(me, mine, now)) == me
    }
}

impl Client {
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
        let on_the_floor = self.my_position().is_some_and(|me| {
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
            .any(|(guid, seen)| seen >= blow && ours(*guid))
    }

    /// Whether the corpse named `corpse` is another player's, and theirs:
    /// named for anyone but this character who is a player in view or on
    /// the team, or for no creature there is.
    pub(crate) fn corpse_is_someone_elses(&self, corpse: &str) -> bool {
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

    /// The next fight waits for the body the last one left: every body
    /// is to be emptied first, one is owed, and nothing is hitting the
    /// character meanwhile. A character that does not loot owes nothing,
    /// or the fighting would stop for good.
    pub fn waits_for_a_corpse(&self) -> bool {
        self.loot_profile()
            .is_some_and(|p| p.looting.after_every_fight)
            && (self.owes_a_corpse() || self.party_owed_a_body(Instant::now()).is_some())
            && !self.under_attack()
    }

    /// A body the plan dealt to one of the others, still lying there
    /// within the leader's reach, as `(who, what)` by name (see
    /// [`Autoplay::body_dealt_to_another`]). Within reach: a body across
    /// the field is nothing to hold the party for, as it is nothing for
    /// a character alone (see [`Self::owes_a_corpse`]).
    pub(crate) fn party_owed_a_body(&self, now: Instant) -> Option<(String, String)> {
        if !self.autoplay.team.leader {
            return None;
        }
        let me = self.world.player_guid?;
        let mine = self.my_position()?;
        let objects = &self.world.objects;
        let there = |g: u32| {
            objects
                .get(&g)
                .and_then(|o| o.world_pos())
                .is_some_and(|at| corpse_within_reach(mine, at))
        };
        let (body, to) = self.autoplay.body_dealt_to_another(me, now, there)?;
        let who = self
            .autoplay
            .team
            .mates
            .iter()
            .find(|m| m.guid == to)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| format!("{to:#010x}"));
        let what = self.world.name_or_hex(body);
        Some((who, what))
    }
}

#[cfg(test)]
mod tests;
