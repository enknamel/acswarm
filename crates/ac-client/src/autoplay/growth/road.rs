//! The road: whether the character is on its way somewhere (`Client::on_its_way`), where only
//! what attacks is fought, and what an errand's walk to a counter or a ground does when no journey
//! of its own is under way (`on_the_way`). A journey ends arrived or refused; no clock times it.

use std::time::Duration;

use ac_world::Place;

use crate::{Client, Goal};

/// After a journey that could not be planned, the next place is not tried for this long.
pub(super) const RETRY_AFTER: Duration = Duration::from_secs(30);

/// What an errand on its way to a counter or a ground does with no journey of its own under way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OnTheWay {
    /// Near enough already: up to the counter, or on the ground.
    There,
    /// Something else broke the journey off and still has the character.
    Wait,
    /// Something else broke the journey off and is done: set off again.
    WalkOn,
    /// The journey ended by itself short of the counter or the ground.
    Short,
}

/// Least time between two plans of an errand's broken-off walk: one ended as soon as planned
/// would otherwise be a route search every tick or two (see [`on_the_way`]).
pub(super) const WALK_ON_EVERY: Duration = Duration::from_secs(5);

/// A walk broken off by something else (`broken_off`) walks on once that is done (`busy`) and it
/// was not planned within [`WALK_ON_EVERY`] (`lately`); one that ended by itself is short.
pub(super) fn on_the_way(there: bool, broken_off: bool, busy: bool, lately: bool) -> OnTheWay {
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

/// A road outside the errands: `trip` the journey under way, `resumed` the one kept for a fight's
/// end, each as whether it is about the ground (only a roam, a patrol, a follower's catching up).
fn road_under_way(trip: Option<bool>, resumed: Option<bool>) -> bool {
    trip.is_some_and(|about_the_ground| !about_the_ground)
        || resumed.is_some_and(|about_the_ground| !about_the_ground)
}

impl Client {
    /// On its way: an errand's journey under way or broken off (`bound` stays till grow, the last
    /// goal, looks again), any other road, or a leader followed on its way: a party stops as one.
    pub fn on_its_way(&self) -> bool {
        let st = &self.autoplay.growth;
        let walking = st.has_an_errand() && (self.traveling() || self.journey_broken_off());
        walking
            || road_under_way(
                self.traveling().then_some(self.travel_about_the_ground()),
                self.autoplay
                    .resume_trip
                    .map(|_| self.autoplay.resume_about_the_ground),
            )
            || self.followed_leader().is_some_and(|m| m.on_its_way)
    }

    // ---- journeys -------------------------------------------------

    /// Whether the journey under way is an errand's walk to `goal`: the running row's own, one to
    /// that goal (taken over), or one no row owns, which walks on by itself and is waited out.
    pub(super) fn is_errand_journey(&mut self, goal: Goal) -> bool {
        self.traveling()
            && (self.is_own_journey() || self.keep_journey_to(goal) || self.journey_row().is_none())
    }

    /// Set off for `goal`, to within `within`, for the running row; from inside a building the
    /// journey's first stretch walks out of the door. False when no way was found.
    pub(super) fn grow_travel(&mut self, goal: Place, within: f32) -> bool {
        self.journey_to(goal, within)
    }
}

#[cfg(test)]
mod tests;
