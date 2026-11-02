use std::time::{Duration, Instant};

use super::view::Mate;
use crate::autoplay::{Doing, Release};
use crate::did::Did;
use crate::travel::WALKABLE;
use crate::{Client, Goal, Owner};

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

/// The STEPS rows that follow a leader, whose walks are following's (`Owner::Step`).
pub(crate) const FOLLOW_ROWS: [&str; 2] = ["catch up", "follow"];

impl Client {
    /// The mate this character takes its lead from: the team is on, this
    /// character is not the one leading, and that mate asked to lead
    /// (`Team::lead`). Only the asking makes a leader to take from: a
    /// roster whose leader is no more than the first name in it is a
    /// party of equals, and nobody goes anywhere for it.
    pub(crate) fn team_leader(&self) -> Option<&Mate> {
        let team = &self.autoplay.config.team;
        if !team.enabled || team.lead || self.autoplay.team.leader {
            return None;
        }
        self.autoplay.team.leader_mate().filter(|m| m.leads)
    }

    /// The leader this character follows, when it follows one.
    pub(crate) fn followed_leader(&self) -> Option<&Mate> {
        if !self.autoplay.config.team.follow {
            return None;
        }
        self.team_leader()
    }

    /// Whether the leader says where this character goes: followed, and not on a town run of its
    /// own, which following waits out. Its own walks step aside; the area limits only its fights.
    pub(crate) fn is_led(&self) -> bool {
        self.followed_leader().is_some() && !self.autoplay.growth.town_run_under_way()
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
        let Some(me) = self.my_position() else {
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
            self.stop_at_leader();
            return false;
        }
        if far {
            // Whatever we were fighting is not worth losing the leader.
            self.let_go(Release::Targets);
        }
        let goal = glam::Vec2::new(leader.world.x, leader.world.y);
        let mut refused = None;
        if flat < FOLLOW_WALK || (leader.flying && flat <= WALKABLE) {
            // A journey after it planned for a refused walk is kept while it is bound for it.
            let on_the_way = self.is_follow_journey()
                && self
                    .autoplay
                    .follow_trip
                    .is_some_and(|g| g.distance(goal) <= 30.0);
            if !on_the_way {
                if self.autoplay.follow_trip.take().is_some() && self.traveling() {
                    self.cancel_travel();
                }
                match self.go(self.step_owner(), self.leader_goal(&leader, keep), now) {
                    Did::Done if level => {
                        self.stop_at_leader();
                        return false;
                    }
                    // Flown, a walk goes straight there whatever the planner says.
                    Did::Blocked(why) if !self.noclip() => refused = Some(why.what),
                    _ => {
                        // Past the region the slot set off on a journey: following's, for a stop.
                        if self.traveling() {
                            self.autoplay.follow_trip = self.travel_goal_xy();
                        }
                    }
                }
            }
        }
        if refused.is_some() || flat >= FOLLOW_WALK && !(leader.flying && flat <= WALKABLE) {
            // Out of sight (through a portal, say), or no walk there: a journey, planned again once
            // it has moved on. Not while it stands where a journey cannot end -- inside the Town
            // Network hub, or a dungeon -- in another landblock: it will come out.
            let indoors = leader.cell & 0xFFFF >= 0x100;
            let my_block = self.player.as_ref().map(|p| p.landblock());
            if indoors && my_block != Some(leader.cell & 0xFFFF_0000) {
                self.stop_going(Some(self.step_owner()), "the leader is indoors elsewhere");
                self.autoplay
                    .say(Doing::Following, "waiting for the leader to come out");
                return false;
            }
            let stale = self
                .autoplay
                .follow_trip
                .is_none_or(|g| g.distance(goal) > 30.0);
            let due = self.autoplay.next_follow_plan.is_none_or(|t| now >= t);
            if (stale || !self.traveling()) && due {
                self.stop_going(Some(self.step_owner()), "a journey after the leader");
                // On the leader's way, whatever the leader is on (see
                // `Client::on_its_way`): not a road of its own.
                let planned = self.travel_about(goal);
                // Recorded by what is under way, not by the answer: a journey can start and still
                // answer no, and a plan that failed leaves the one before it running.
                if self.traveling() && self.travel_goal_xy() == Some(goal) {
                    self.autoplay.follow_trip = Some(goal);
                } else if !self.is_follow_journey() {
                    self.autoplay.follow_trip = None;
                }
                let wait = if planned { 3 } else { 10 };
                self.autoplay.next_follow_plan = Some(now + Duration::from_secs(wait));
            }
            if let Some(why) = refused.filter(|_| !self.traveling()) {
                self.autoplay
                    .note(format!("no way to {} ({why})", leader.name), now);
                return false;
            }
        }
        self.autoplay
            .say(Doing::Following, format!("following {}", leader.name));
        true
    }

    /// Standing beside the leader: following's walk and its journey after it end.
    fn stop_at_leader(&mut self) {
        self.stop_going(Some(self.step_owner()), "beside the leader");
        if self.autoplay.follow_trip.take().is_some() && self.traveling() {
            self.cancel_travel();
        }
    }

    /// The walk to `leader`, to within `keep`: where the world shows it, else where the team says
    /// (its cell, or the block under it when the team names none).
    fn leader_goal(&self, leader: &Mate, keep: f32) -> Goal {
        if self.world.objects.contains_key(&leader.guid) {
            return Goal::Near {
                guid: leader.guid,
                within: keep,
            };
        }
        let place = match leader.cell {
            0 => self.place_at(leader.world),
            cell => ac_world::Place {
                at: leader.world,
                cell,
            },
        };
        Goal::At {
            place,
            within: keep,
        }
    }

    /// Let go of following's walk (its rows' own, `FOLLOW_ROWS`) and the journey it planned while
    /// still following's; what others planted stays.
    pub fn stop_following(&mut self) {
        let mut let_go = false;
        let owner = self.going.as_ref().map(|g| g.owner);
        if let Some(o) = owner.filter(|o| FOLLOW_ROWS.iter().any(|r| *o == Owner::Step(r))) {
            self.stop_going(Some(o), "no leader followed");
            let_go = true;
        }
        // The trip alone: a visit riding on it is its own, and plans
        // again once the trip is gone.
        if self.is_follow_journey() {
            self.end_trip();
            let_go = true;
        }
        // A goal with no journey under way is what a failed replan leaves between ticks
        // (`cancel_travel_keeping_refusals`); whoever wants it plans again, but a death keeps it.
        if !self.traveling() && self.travel_goal_xy().is_some() {
            self.end_trip();
            let_go = true;
        }
        self.autoplay.follow_trip = None;
        self.autoplay.next_follow_plan = None;
        if let_go {
            tracing::info!("follow: no leader followed now; letting go of the way after it");
        }
    }

    /// Whether the journey under way is the one following planned: bound for `Autoplay.follow_trip`
    /// exactly, since the planner keeps the goal it was given.
    pub(crate) fn is_follow_journey(&self) -> bool {
        self.traveling()
            && self.autoplay.follow_trip.is_some()
            && self.travel_goal_xy() == self.autoplay.follow_trip
    }

    /// How close two characters must stand to hand something over.
    pub(super) const REACH: f32 = 5.0;
}

#[cfg(test)]
mod tests;
