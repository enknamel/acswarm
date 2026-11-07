use std::time::{Duration, Instant};

#[cfg(doc)]
use super::choose::LOOT_NEAR;
use super::choose::{CORPSE_REACH, LOOT_TIMEOUT};
use super::take::LOOT_WALK;
use crate::autoplay::{Autoplay, Doing};
use crate::did::Did;
use crate::{Client, Goal};

/// How long to allow a corpse `away` metres off to open.
///
/// Opening one asks the server to walk us there, and that walk is not
/// instant: a flat six seconds was enough for a corpse at our feet and
/// not for one across a room, so the far ones were written off unopened.
pub(crate) fn loot_wait(away: f32) -> Duration {
    LOOT_TIMEOUT + Duration::from_secs_f32((away.max(0.0) / LOOT_WALK).min(30.0))
}

/// A walk to a corpse that the looting set going ([`Client::go`]'s walk holds the rest).
#[derive(Clone, Copy, Debug)]
pub(crate) struct CorpseWalk {
    /// The corpse being walked to.
    pub(crate) guid: u32,
    /// When the walk began, for the log and the claim.
    pub(super) started: Instant,
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
    /// Let go of a walk toward a corpse, wherever that corpse has been let go of: left running,
    /// it would carry the character off to a body it has finished with.
    pub(crate) fn stop_walking_to_loot(&mut self) {
        let Some(w) = self.autoplay.walking_to.take() else {
            return;
        };
        let owner = self
            .going
            .as_ref()
            .filter(|g| matches!(g.goal, Goal::Near { guid, .. } if guid == w.guid))
            .map(|g| g.owner);
        if owner.is_some() {
            self.stop_going(owner, "done with the corpse");
        }
    }

    /// Walk to the corpse `guid`, called `name` and `away` metres off, to well inside
    /// [`CORPSE_REACH`], the reach the use is sent from. What the slot answers is the caller's.
    pub(crate) fn walk_to_corpse(&mut self, guid: u32, name: &str, away: f32, now: Instant) -> Did {
        // A detour the character comes back from: a town run picks its walk to the counter up
        // again once the body is dealt with (`Client::journey_broken_off`).
        self.remember_journey();
        self.interrupt_travel("walking to a corpse");
        let goal = Goal::Near {
            guid,
            within: CORPSE_REACH / 2.0,
        };
        let did = self.go(self.step_owner(), goal, now);
        // Said once for the walk, not once a frame.
        if self.autoplay.walking_to.map(|w| w.guid) != Some(guid) {
            self.autoplay.walking_to = Some(CorpseWalk { guid, started: now });
            self.autoplay.say(
                Doing::Looting,
                format!("walking to {name} ({} m)", away.round()),
            );
        }
        did
    }
}

#[cfg(test)]
mod tests;
