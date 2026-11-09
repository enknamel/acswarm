//! A journey flown as the last resort, when the body may fly: one that hops (a portal, recall or
//! gem) with no way left on foot, flown to its next hop over what the walk graph cannot cross
//! (Ithaenc's ruins), then walked; and one within the dungeon stood in, clipped through whatever
//! wall or shaft the graph has no way past (the Catacombs of Ithaenc's key room).

use std::time::Instant;

use ac_world::trip::Step;

use crate::did::Did;
use crate::{Client, Goal, Owner};

impl Client {
    /// [`set_off`](Self::set_off), and with no way on foot to a place in the dungeon stood in,
    /// flown there; refused, kept refused for `owner` and answered so till the body moves.
    pub(super) fn set_off_or_fly(&mut self, owner: Owner, goal: Goal, now: Instant) -> Did {
        let ends = self.my_position().zip(goal.place().map(|p| p.at));
        if let Some(why) = ends.and_then(|e| self.kept_refusal(owner, goal, e, true)) {
            return Did::blocked(why);
        }
        let mut did = self.set_off(owner, goal, now);
        if matches!(did, Did::Blocked(_)) && self.is_in_this_dungeon(goal) {
            did = self.fly_journey(owner, goal, now);
        }
        if let (Did::Blocked(why), Some((me, there))) = (&did, ends) {
            self.refuse(owner, goal, me, there, &why.what);
        }
        did
    }

    /// Whether `goal` lies in the dungeon the body stands in.
    pub(super) fn is_in_this_dungeon(&mut self, goal: Goal) -> bool {
        let here = self.player.as_ref().map_or(0, |pl| pl.cell >> 16);
        let there = goal
            .place()
            .filter(|p| p.is_indoors())
            .map(|p| p.cell >> 16);
        there == Some(here) && self.underground()
    }

    /// As after a journey to `goal` flown to its hop and carried through it, still its goal.
    #[cfg(test)]
    pub(crate) fn flown_to_a_hop(&mut self, goal: Goal) {
        self.travel.fly_to = goal.place().map(|p| p.at.truncate());
        self.travel.goal = Some(goal);
        self.travel.hopped = true;
    }

    /// Whether the journey takes a portal, recall or gem: places the trip planner joined.
    pub(crate) fn is_hopping(&self) -> bool {
        let trip = self.travel.trip.as_ref();
        trip.is_some_and(|t| t.steps.iter().any(|s| !matches!(s, Step::Walk(_))))
    }

    /// Whether the journey under way flies its stretches to its destination: till its first hop,
    /// the flight set off afresh (`hopped` cleared).
    pub(crate) fn is_journey_flown(&self) -> bool {
        let to = self.travel_goal_xy();
        self.movement_rules.limits().noclip
            && !self.travel.hopped
            && self
                .travel
                .fly_to
                .is_some_and(|f| to.is_some_and(|g| g.distance(f) <= 1.0))
    }

    /// The journey to `goal` set off flown, `owner`'s refusals forgotten; blocked when
    /// the body may not fly, it is flown already, or the trip planner still has no way. A flight
    /// to the same goal before a hop is no bar: the ruins flown, the catacombs are flown afresh.
    pub(super) fn fly_journey(&mut self, owner: Owner, goal: Goal, now: Instant) -> Did {
        let Some(to) = goal.place().map(|p| p.at.truncate()) else {
            return Did::blocked("no place to fly to");
        };
        if !self.movement_rules.limits().noclip || self.is_journey_flown() {
            return Did::blocked("no way left, and no flying it");
        }
        tracing::info!("travel: no way left on foot; the journey is flown");
        self.cancel_travel();
        self.travel.fly_to = Some(to);
        // The walks refused on foot are flown now: theirs and the journey's refusals no longer
        // stand; another owner's still do. A journey of no row walks as `Owner::Journey`.
        self.forget_refusals(owner);
        if !matches!(owner, Owner::Step(_)) {
            self.forget_refusals(Owner::Journey);
        }
        self.set_off(owner, goal, now)
    }
}
