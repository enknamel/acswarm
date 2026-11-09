//! A journey flown as the last resort: planned again, a journey that hops (a portal, recall or
//! gem) has no way left on foot, and the body may fly; each stretch to the next hop is then flown
//! straight to its end (`Route::flown_across`) over what the walk graph cannot cross (Ithaenc's
//! ruins). Past the hop it is walked: flown through a dungeon's walls, the server puts it back.

use std::time::Instant;

use ac_world::trip::Step;

use crate::did::Did;
use crate::{Client, Goal, Owner};

impl Client {
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

    /// The journey to `goal` set off flown, its refusals and the slot's forgotten; blocked when
    /// the body may not fly, it is flown already, or the trip planner still has no way.
    pub(super) fn fly_journey(&mut self, owner: Owner, goal: Goal, now: Instant) -> Did {
        let Some(to) = goal.place().map(|p| p.at.truncate()) else {
            return Did::blocked("no place to fly to");
        };
        if !self.movement_rules.limits().noclip || self.travel.fly_to.is_some() {
            return Did::blocked("no way left, and no flying it");
        }
        tracing::info!("travel: no way left on foot; the journey is flown");
        self.cancel_travel();
        self.travel.fly_to = Some(to);
        // The walks refused on foot are flown now: their refusals no longer stand.
        self.refusals.clear();
        self.set_off(owner, goal, now)
    }
}
