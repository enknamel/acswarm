use std::time::{Duration, Instant};

use super::view::Mate;
use crate::autoplay::{Doing, Release};
use crate::Client;

/// A leader further off than twice the following distance (and at least
/// this) is followed before anything else, a fight included; nearer,
/// the fight comes first. Following is the follower's job.
const FOLLOW_BREAK: f32 = 10.0;

/// How far from its leader a follower keeping `keep` metres may stray
/// before following comes before everything else.
pub fn follow_break(keep: f32) -> f32 {
    (2.0 * keep).max(FOLLOW_BREAK)
}

/// Up to this far the follower walks straight for the leader, letting
/// the steering find the way; further (the leader took a portal) a
/// journey is planned.
const FOLLOW_WALK: f32 = 120.0;

impl Client {
    /// The leader this character follows, when it follows one.
    pub(crate) fn followed_leader(&self) -> Option<&Mate> {
        let team = &self.autoplay.config.team;
        if !team.enabled || !team.follow || team.lead || self.autoplay.team.leader {
            return None;
        }
        self.autoplay.team.leader_mate().filter(|m| m.leads)
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
            self.let_go(Release::Targets);
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
    pub(super) const REACH: f32 = 5.0;
}

#[cfg(test)]
mod tests;
