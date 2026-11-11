//! A step or journey with no way on foot: a portal's left out and the journey planned again, a
//! walk carried into a region left out from where it starts, or the journey refused.

use ac_world::trip::{NoWalk, Step};

use crate::Client;

impl Client {
    /// The step's walk has no way: a portal is left out and the journey planned again next frame,
    /// and so is the walk to its end once carried into this region; else the journey is refused.
    pub(super) fn refuse_step(&mut self, why: &str) {
        let n = self.travel.step;
        match self.travel_step().cloned() {
            Some(Step::Portal { name, mouth, .. }) => {
                tracing::warn!("travel: step {n} (portal {name:?}): {why}; going another way");
                self.travel.refused.push(mouth.at.truncate());
                self.travel.replan_due = true;
            }
            // An end no floor of its cell holds is the goal's fault, not the way's: no plan helps.
            Some(Step::Walk(to)) if self.is_refused_off_floor(to.at) => self.refuse_journey(why),
            Some(Step::Walk(to)) if self.travel.hopped => {
                let Some((from, _)) = self.planning_from() else {
                    return self.refuse_journey(why);
                };
                tracing::warn!(
                    "travel: step {n} (walk to {:?}): {why}; going another way",
                    to.at
                );
                let to = self.planning_place(to);
                self.travel.refused_walks.push(NoWalk { from, to });
                // And from where the hop came out, where a plan's walk there starts: refused only
                // where it stood, Portal Recall and the same walk were planned again for ever
                // (a_walk_refused_after_a_recall_is_not_planned_again_from_its_landing).
                let landed = n.checked_sub(1).and_then(|i| {
                    let trip = self.travel.trip.as_ref()?;
                    trip.steps.get(i).and_then(Step::exit)
                });
                if let Some(landed) = landed {
                    let from = self.planning_place(landed);
                    self.travel.refused_walks.push(NoWalk { from, to });
                }
                self.travel.replan_due = true;
            }
            _ => self.refuse_journey(why),
        }
    }

    /// No way to the journey's end from here: it ends, kept refused for whoever set it off.
    pub(super) fn refuse_journey(&mut self, why: &str) {
        tracing::warn!(
            "travel: step {}: {why}; the journey is refused",
            self.travel.step
        );
        let (goal, owner, me) = (self.travel.goal, self.travel.owner, self.my_position());
        if let (Some(goal), Some(owner), Some(me)) = (goal, owner, me) {
            if let Some(p) = goal.place() {
                self.refuse(owner, goal, me, p.at, why);
            }
        }
        self.end_trip();
        self.travel.why_refused = Some(why.to_string());
    }
}
