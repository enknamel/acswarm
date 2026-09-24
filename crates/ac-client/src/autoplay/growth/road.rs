use std::time::{Duration, Instant};

use glam::Vec2;

use crate::Client;

/// A walk to a vendor, or to a hunting ground, that has taken longer
/// than this is stuck somewhere: the place is given up on.
pub(super) const WALK_TIMEOUT: Duration = Duration::from_secs(4 * 60);

/// How long a walk may take before it counts as stuck: [`WALK_TIMEOUT`], or twice the journey
/// planned (`Trip::seconds`) when that is longer, as the last-resort walk home from a far ground is.
pub(super) fn walk_limit(planned_seconds: Option<f32>) -> Duration {
    planned_seconds
        .filter(|s| s.is_finite() && *s > 0.0)
        .map_or(WALK_TIMEOUT, |s| {
            Duration::from_secs_f32(2.0 * s).max(WALK_TIMEOUT)
        })
}

impl Client {
    /// [`walk_limit`] for the journey just planned.
    pub(super) fn planned_walk_limit(&self) -> Duration {
        walk_limit(self.travel_trip().map(|t| t.seconds))
    }
}

/// After a journey that could not be planned, the next place is not
/// tried for this long.
pub(super) const RETRY_AFTER: Duration = Duration::from_secs(30);

/// What a run on its way to a counter does with no journey under way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OnTheWay {
    /// Near enough to go up to the counter.
    There,
    /// Something else broke the journey off and still has the character.
    Wait,
    /// Something else broke the journey off and is done: set off again.
    WalkOn,
    /// The journey ended by itself short of the counter.
    Short,
}

/// How soon after planning the walk to the counter again a run may plan
/// it once more (see [`on_the_way`]).
pub(super) const WALK_ON_EVERY: Duration = Duration::from_secs(5);

/// What a walk to a counter or to a hunting ground does with no journey
/// under way. `there` says it is near enough already, `broken_off` that
/// the last journey was ended by something else the character went to do
/// (see `Client::journey_broken_off`), `busy` that it is still doing it,
/// and `lately` that the walk was planned again less than
/// [`WALK_ON_EVERY`] ago. How long the walk may take is the errand's own
/// clock, and is asked before this.
///
/// Any journey not under way used to be one that could not get there.
/// +Verity set off to sell, a corpse took her a second later -- walking
/// to one ends a journey -- and the run gave up 224 m short, marked the
/// counter no use and sold nothing. A walk that ended by itself short of
/// the counter is still one that cannot get there. The walk to a hunting
/// ground did the same, and worse: it put the ground on the skip list and
/// set off for another.
///
/// Planning the walk is a route search, and something that ends the walk
/// as soon as it is planned would have it planned every tick or two until
/// the run's clock ran out: a follower pulled back to its leader was, each
/// time it closed to the following distance. So it waits a moment first.
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

/// Whether a journey that is not one of the growth rules' errands still
/// has the character on its way somewhere: a road is under way, or one
/// put down for a fight is waiting to be picked up again. `trip` is the
/// journey under way, if any, and whether it is about the character's
/// own ground; `resumed` the same for the journey remembered for after
/// the fight (see `Client::on_its_way`).
///
/// A walk about the ground -- a roam, a patrol -- is never a road: what
/// stands about the ground is what the character came for.
fn road_under_way(trip: Option<bool>, resumed: Option<bool>) -> bool {
    trip.is_some_and(|about_the_ground| !about_the_ground)
        || resumed.is_some_and(|about_the_ground| !about_the_ground)
}

impl Client {
    /// Whether the character is on its way somewhere it decided to go:
    /// out to a hunting ground, back to one from town, or to a counter.
    ///
    /// Two halves, and it takes both.
    ///
    /// Why it is walking comes from the growth rules' errands, not from
    /// [`Client::traveling`], which says a journey is under way and nothing
    /// about why. The patrol and the roam (`Client::grow_hunt`) travel
    /// too, about the very ground the character came for, so a rule that
    /// read `traveling` alone would have a character stand in its own
    /// hunting ground refusing to fight. Both set off and return before
    /// `bound` is ever set.
    ///
    /// That it is still walking comes from the journey, because the errand
    /// outlives the walk. `bound` is let go only when the hunting step next
    /// looks, and that is the last goal in the table: a body at the
    /// character's feet keeps it from running, and a party restocking does
    /// not call it at all. Read off `bound` alone, a leader home from town
    /// stood on its ground walking past everything until the slowest of the
    /// party had shopped, and one arriving beside a body looted it before
    /// the monster standing over it. `run` spans the counters as well,
    /// where there is no road. A walk broken off by a corpse or a fight is
    /// still the walk, since its errand takes it up again (see
    /// `Client::journey_broken_off`), and so is stepping out of a shop
    /// before it.
    ///
    /// A follower is on its leader's way. It keeps up through the follow
    /// step, which sets no errand of its own, so read off its own errands
    /// alone it stopped to fight what its leader walked past, fell behind,
    /// was fetched back and turned to fight again, and the party came apart
    /// on the road. Each character says on the board whether it is on its
    /// way (`Mate::on_its_way`), which is also how the party stops together
    /// for what attacks any of it (see `Client::passing_by`).
    ///
    /// And a road is a road whoever planned it. The errands are the growth
    /// rules' own, so a journey a script asked for, or the walk to the
    /// portal into a hunting area, was not "on its way" at all, and a
    /// party sent down the Singularity Caul by script fought every Biaka,
    /// Hellion and Carenzi between the drop and the far end: fourteen
    /// fights in a hundred and forty seconds, most of them picked by the
    /// party, on a walk of forty. Every journey says whether it is a road
    /// or a walk about the ground the character is hunting (see
    /// `Client::travel_about`), and the patrol, the roam and a
    /// follower's own catching up are the only walks about the ground.
    pub fn on_its_way(&self) -> bool {
        let st = &self.autoplay.growth;
        let walking = st.has_an_errand()
            && (self.traveling() || self.journey_broken_off() || st.after_out.is_some());
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

    /// Set off for `goal`. From inside a building the planner sees no
    /// way out but a portal, so the walk begins with the spot the
    /// character last stood outdoors, and the journey proper follows
    /// (see [`Self::grow_travel_on`]). False when no way was found.
    pub(super) fn grow_travel(&mut self, goal: Vec2, now: Instant) -> bool {
        self.autoplay.growth.after_out = None;
        let Some(pl) = self.player.as_ref() else {
            return false;
        };
        // The shortcut below is for buildings. A dungeon has no door to
        // step out of -- the way out is its exit portal or a recall --
        // and walking at the last outdoor spot from underground is
        // walking at rock.
        if pl.is_indoors() && !self.autoplay.growth.in_dungeon {
            let me = pl.world_position();
            let block = pl.cell & 0xFFFF_0000;
            let same_block =
                |p: Vec2| (((p.x / 192.0) as u32) << 24) | (((p.y / 192.0) as u32) << 16) == block;
            if let Some(out) = self.autoplay.growth.last_outdoors {
                if same_block(out)
                    && out.distance(Vec2::new(me.x, me.y)) < 150.0
                    && self.travel_to(out)
                {
                    self.autoplay.growth.after_out = Some(goal);
                    self.autoplay.note("stepping outside first", now);
                    return true;
                }
            }
        }
        self.travel_to(goal)
    }

    /// The second leg of a journey begun indoors: once the walk out has
    /// ended, the journey proper. True when one was started.
    pub(super) fn grow_travel_on(&mut self) -> bool {
        if self.traveling() {
            return false;
        }
        let Some(goal) = self.autoplay.growth.after_out.take() else {
            return false;
        };
        if self.travel_to(goal) {
            return true;
        }
        tracing::info!("growth: no way on to {goal:?} from outside either");
        false
    }
}

#[cfg(test)]
mod tests;
