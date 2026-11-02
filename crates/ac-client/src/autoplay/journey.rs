use super::Doing;
use crate::Client;

impl Client {
    /// A swing cancels the journey (the move-to and the trip cannot both
    /// steer). Note where it was going, so it is taken up again once
    /// the fight is over; a row's journey is held for its row instead.
    pub(crate) fn remember_journey(&mut self) {
        // Not following's own journey (following plans it again), nor any while led: a copy would
        // outlive a stop (`a_stop_leaves_no_old_aim_behind_when_a_fight_broke_the_journey_off`).
        let no_row = self.journey_row().is_none();
        if self.traveling() && no_row && !self.is_follow_journey() && !self.is_led() {
            if let Some(goal) = self.journey_goal() {
                self.autoplay.resume_trip = Some(goal);
                self.autoplay.resume_about_the_ground = self.travel_about_the_ground();
            }
        }
    }

    /// Pick the journey up again after a fight, once there is nothing
    /// else to do.
    pub(crate) fn autoplay_resume_journey(&mut self) -> bool {
        // A follower's way is its leader's: one left from before (its own town run's) is let go.
        if self.is_led() {
            self.autoplay.resume_trip = None;
            return false;
        }
        // Picked up, it is this row's to walk until it ends.
        if self.is_own_journey() {
            self.autoplay.say(Doing::Traveling, "back on the road");
            return true;
        }
        let Some(goal) = self.autoplay.resume_trip else {
            return false;
        };
        if self.traveling() || self.attack_target.is_some() || self.autoplay.casting_at.is_some() {
            return false;
        }
        self.autoplay.resume_trip = None;
        let resumed = if self.autoplay.resume_about_the_ground {
            self.travel_about(goal)
        } else {
            goal.place()
                .is_some_and(|p| self.journey_to(p, goal.within()))
        };
        if resumed {
            self.autoplay.say(Doing::Traveling, "back on the road");
            return true;
        }
        false
    }
}
