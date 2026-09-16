use std::time::{Duration, Instant};

#[cfg(doc)]
use super::choose::LOOT_NEAR;
use super::choose::{CORPSE_REACH, LOOT_TIMEOUT};
use super::take::{LOOT_WALK, REACH_GIVE_UP, REACH_PROGRESS};
use crate::autoplay::{Autoplay, Doing};
use crate::Client;

/// How long to allow a corpse `away` metres off to open.
///
/// Opening one asks the server to walk us there, and that walk is not
/// instant: a flat six seconds was enough for a corpse at our feet and
/// not for one across a room, so the far ones were written off unopened.
pub(super) fn loot_wait(away: f32) -> Duration {
    LOOT_TIMEOUT + Duration::from_secs_f32((away.max(0.0) / LOOT_WALK).min(30.0))
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
    pub(super) started: Instant,
    /// The nearest it has been got to, and when that last improved.
    best: f32,
    since: Instant,
    /// Since when the steering has found no way there, without a break.
    no_way_since: Option<Instant>,
    /// When the walk was last pressed on.
    last: Instant,
}

impl CorpseWalk {
    pub(crate) fn new(guid: u32, away: f32, now: Instant) -> Self {
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
pub(super) fn still_holding_at(away: f32) -> bool {
    away <= HOLD_ON_WITHIN
}

impl Autoplay {
    /// Set aside a corpse the character cannot walk to. It is left
    /// alone for as long as anything blocked is, so the looting and the
    /// next fight both pass it by for that long. It is not written off:
    /// the way may be clear from wherever the character stands next.
    pub(super) fn set_aside_out_of_reach(&mut self, guid: u32, now: Instant) {
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
}

impl Client {
    /// Let go of a walk toward a corpse, wherever that corpse has been
    /// let go of. Nothing else is steering here, so leaving the walk
    /// running would carry the character off to a body it has already
    /// finished with.
    pub(crate) fn stop_walking_to_loot(&mut self) {
        if self.autoplay.walking_to.take().is_some() && self.follow.take().is_some() {
            self.steering.reset();
        }
    }

    /// Press on with a walk to the corpse `guid`, called `name`, lying
    /// at `spot` and `away` metres off. True while the walk is getting
    /// somewhere; false once it cannot arrive -- what becomes of the
    /// body then is for the caller to say, because a body being walked
    /// to and a body already in hand end differently.
    pub(crate) fn walk_to_corpse(
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
        // gave up 224 m short of Shopkeeper Renald the Elder. A road
        // that is nobody's errand -- a script's, a hunting area's --
        // has only the remembered journey to bring it back, so it is
        // remembered as a fight remembers it.
        self.remember_journey();
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
}

#[cfg(test)]
mod tests;
