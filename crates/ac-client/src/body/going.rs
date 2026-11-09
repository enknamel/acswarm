//! The slot: the one walk the body follows, asked for by one owner ([`Client::go`]) and answered
//! truthfully. A new walk is planned whole on the asking frame and handed to the follower, and a
//! refusal is kept per owner and goal; a step's walk ends the tick its step stops asking;
//! [`GIVE_UP_AFTER`] is the one give-up. A journey's stretch is walked here while no other is.

use std::time::{Duration, Instant};

use ac_nav::routing::{self, Planned};
use ac_nav::steering::REPLAN_DISTANCE;
use ac_scene::navarea;
use ac_scene::wayfind::NoWay;
use ac_world::Place;
use glam::{Vec2, Vec3};

use super::server_walk::reached;
use crate::did::Did;
use crate::Client;

/// A walk making no progress along any route for this long is given up: the one give-up, where
/// the loot walk, counter, explore, fight approach, quartermaster, academy and visit kept clocks.
pub const GIVE_UP_AFTER: Duration = Duration::from_secs(20);

/// The refusals kept (`Client.refusals`): more than the goals one tick asks for, a mate each.
const REFUSALS_KEPT: usize = 16;

/// A goal answered no way for an owner, from where the body stood and where the goal was then.
#[derive(Clone, Debug)]
pub(crate) struct Refusal {
    owner: Owner,
    goal: Goal,
    from: Vec3,
    there: Vec3,
    why: String,
    /// The planner's answer, when it was the planner that refused.
    no_way: Option<NoWay>,
    /// A journey's refusal: what the trip planner had then (`journey_means`); a walk's has none.
    means: Option<u64>,
}

/// Who asked for a walk: a STEPS row, or something outside the step table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// The row running when it asked (`steps::Step::name`); its walk ends when it stops asking.
    Step(&'static str),
    /// The stretch of a journey no STEPS row owns (`Travel`), asked by `walk_journey`.
    Journey,
    /// A visit's last stretch (`visit.rs`).
    Visit,
    /// A script's `walk_to`.
    Script,
    /// A front end or a test: the map, a panel, the route harness.
    Hand,
}

impl Owner {
    /// The owner's name in the log and telemetry: the row's, else what it is.
    pub fn label(self) -> &'static str {
        match self {
            Owner::Step(row) => row,
            Owner::Journey => "journey",
            Owner::Visit => "visit",
            Owner::Script => "script",
            Owner::Hand => "hand",
        }
    }
}

/// Where a walk is to: a place, or something in the world wherever it stands now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Goal {
    /// A place on its floor, to within `within` metres (3D).
    At { place: Place, within: f32 },
    /// An object where it stands, to within `within` metres (3D).
    Near { guid: u32, within: f32 },
}

impl Goal {
    pub(crate) fn within(self) -> f32 {
        match self {
            Goal::At { within, .. } | Goal::Near { within, .. } => within,
        }
    }

    /// The place of an `At` goal.
    pub fn place(self) -> Option<Place> {
        match self {
            Goal::At { place, .. } => Some(place),
            Goal::Near { .. } => None,
        }
    }

    /// The same walk: a place moved less than [`REPLAN_DISTANCE`], or the same object, as near.
    pub(crate) fn is_same(self, other: Goal) -> bool {
        match (self, other) {
            (
                Goal::At {
                    place: a,
                    within: x,
                },
                Goal::At {
                    place: b,
                    within: y,
                },
            ) => a.at.distance(b.at) <= REPLAN_DISTANCE && x == y,
            (Goal::Near { guid: a, within: x }, Goal::Near { guid: b, within: y }) => {
                a == b && x == y
            }
            _ => false,
        }
    }
}

/// The walk the slot holds (`Client.going`).
#[derive(Clone, Debug)]
pub struct Going {
    pub owner: Owner,
    pub goal: Goal,
    /// The journey's stretch: let go by the journey, not by its owner's asking.
    pub journey: bool,
    /// The autoplay tick (`Autoplay.tick`) of the owner's last ask.
    asked: u64,
    /// When the walk last made progress along any of its routes, or began.
    progressed: Instant,
    /// Why the walk was answered no way (kept in `Client.refusals` too): the body stands.
    refused: Option<String>,
    /// Given up ([`GIVE_UP_AFTER`]): the body stands.
    given_up: bool,
}

impl Client {
    /// Walk to `goal` for `owner`: the one way anything asks the body to move, ending a visit unless
    /// the visit asks. Past the region it is a journey; asked again, the same goal plans nothing.
    pub fn go(&mut self, owner: Owner, goal: Goal, now: Instant) -> Did {
        self.ask_slot(owner, goal, now, false)
    }

    /// [`Client::go`], or with `journey` the journey's stretch for its owner: planned the same,
    /// it leaves the visit and the journey alone and is never a journey of its own.
    fn ask_slot(&mut self, owner: Owner, goal: Goal, now: Instant, journey: bool) -> Did {
        let Some(me) = self.player.as_ref().map(|p| Place {
            at: p.world_position(),
            cell: p.cell,
        }) else {
            return Did::waiting("not in the world yet");
        };
        // A walk sent mid-hop is lost to the teleport; only the journey's own stretch goes.
        if !journey && self.autoplay.is_hopping(now) {
            return Did::waiting("a hop is in the air");
        }
        if !journey && !matches!(owner, Owner::Visit | Owner::Journey) {
            self.drop_visit(owner.label());
        }
        if let Some((_, by)) = self.taken.filter(|(was, _)| *was == owner && !journey) {
            self.taken = None;
            return Did::waiting(format!("taken by {}", by.label()));
        }
        let Some(place) = self.goal_place(goal) else {
            return Did::blocked("not in view");
        };
        if let Some(why) = self.kept_refusal(owner, goal, (me.at, place.at), false) {
            return Did::blocked(why);
        }
        let tick = self.autoplay.tick;
        if let Some(g) = self.going.as_mut().filter(|g| {
            g.owner == owner && g.journey == journey && g.goal.is_same(goal) && g.refused.is_none()
        }) {
            g.goal = goal;
            g.asked = tick;
            return self.answer_going(me.at, place, now);
        }
        self.start_going(owner, goal, (me, place), now, journey)
    }

    /// Why `owner`'s `goal`, at `there` now, stands refused from `me` (with `journeys`, only a
    /// journey's refusal): one stands while the body and the goal stand where they were.
    pub(crate) fn kept_refusal(
        &mut self,
        owner: Owner,
        goal: Goal,
        (me, there): (Vec3, Vec3),
        journeys: bool,
    ) -> Option<String> {
        self.refusals
            .retain(|r| r.from.distance(me) <= REPLAN_DISTANCE);
        let at = self.refusals.iter().position(|r| {
            r.owner == owner
                && (r.means.is_some() || !journeys)
                && r.goal.is_same(goal)
                && r.there.distance(there) <= REPLAN_DISTANCE
        })?;
        // A gem come into the pack, a recall castable again, a level: the journey is planned anew.
        if self.refusals[at]
            .means
            .is_some_and(|m| m != self.journey_means())
        {
            self.refusals.remove(at);
            return None;
        }
        Some(self.refusals[at].why.clone())
    }

    /// Forget `owner`'s refusals: its journey is flown now, past what refused it on foot.
    pub(crate) fn forget_refusals(&mut self, owner: Owner) {
        self.refusals.retain(|r| r.owner != owner);
    }

    /// Keep `owner`'s journey to `goal` refused (`Client.refusals`), in place of an older refusal.
    pub(crate) fn refuse(&mut self, owner: Owner, goal: Goal, from: Vec3, there: Vec3, why: &str) {
        let means = Some(self.journey_means());
        self.keep_refusal(owner, goal, (from, there), why, (None, means));
    }

    /// Keep `owner`'s walk to `goal` refused, with the planner's answer when it was the planner's.
    fn refuse_for(
        &mut self,
        owner: Owner,
        goal: Goal,
        ends: (Vec3, Vec3),
        why: &str,
        no_way: Option<NoWay>,
    ) {
        self.keep_refusal(owner, goal, ends, why, (no_way, None));
    }

    /// Keep a refusal of `owner`'s `goal` from `from` to `there`, a journey's or a walk's.
    fn keep_refusal(
        &mut self,
        owner: Owner,
        goal: Goal,
        (from, there): (Vec3, Vec3),
        why: &str,
        (no_way, means): (Option<NoWay>, Option<u64>),
    ) {
        self.refusals
            .retain(|r| !(r.owner == owner && r.goal.is_same(goal)));
        if self.refusals.len() >= REFUSALS_KEPT {
            self.refusals.remove(0);
        }
        self.refusals.push(Refusal {
            owner,
            goal,
            from,
            there,
            why: why.to_string(),
            no_way,
            means,
        });
    }

    /// Whether a walk to `there` was refused because no floor of its cell holds it.
    pub(crate) fn is_refused_off_floor(&self, there: Vec3) -> bool {
        self.refusals.iter().any(|r| {
            r.no_way == Some(NoWay::OffFloor) && r.there.distance(there) <= REPLAN_DISTANCE
        })
    }

    /// A new walk for `owner`, planned here and handed to the follower; arrived or refused, the slot
    /// is left as it was. A walk ends a journey without breaking it off (`interrupt_travel` does).
    fn start_going(
        &mut self,
        owner: Owner,
        goal: Goal,
        (me, place): (Place, Place),
        now: Instant,
        journey: bool,
    ) -> Did {
        // Nothing is planned from the air, which has no way: the walk is asked again once the
        // body is down (a crossing over a trap plate lands first, `tick_player`).
        if self.is_aloft() {
            return Did::waiting("in the air");
        }
        // A journey's stretch is the planner's to refuse; any other goal past the region is a
        // journey, whose stretch takes the slot.
        if !journey && !self.is_in_region(place) {
            let xy = place.at.truncate();
            // Acting while it travels to this goal, taken over for `owner`; one elsewhere is replaced.
            let bound = self.traveling()
                && self
                    .travel_goal_xy()
                    .is_some_and(|g| g.distance(xy) <= REPLAN_DISTANCE);
            if bound {
                if self.going.as_ref().is_some_and(|g| !g.journey) {
                    self.replace_walk(owner, me.at);
                }
                self.take_journey(owner);
                return Did::Acting;
            }
            let within = goal.within();
            let did = self.set_off(owner, Goal::At { place, within }, now);
            if let Did::Blocked(why) = &did {
                tracing::info!(
                    "travel: no way for {} to {:?} from here: {}",
                    owner.label(),
                    place.at,
                    why.what
                );
                self.refuse(owner, goal, me.at, place.at, &why.what);
            }
            return did;
        }
        let within = goal.within();
        if reached(me.at, place.at, within) {
            return Did::Done;
        }
        // Flown, the body goes straight there (`tick_player`): nothing is planned. A journey with
        // no way left but flying flies its stretch across whatever lies between (`Travel.fly`).
        let fly = journey && self.is_journey_flown();
        let planned = match fly {
            true => Some(Planned::Route(ac_nav::routing::Route::flown_across(
                me, place,
            ))),
            false => (!self.flies_freely()).then(|| self.plan_walk(place, within)),
        };
        if let Some(Planned::NoWay(no)) = planned {
            let why = format!(
                "no way there: {:?}",
                ac_nav::steering::Unreachable::Planner(no)
            );
            self.refuse_for(owner, goal, (me.at, place.at), &why, Some(no));
            return Did::blocked(why);
        }
        self.replace_walk(owner, me.at);
        // The journey's own owner wants something else; another row's journey is held meanwhile.
        let ends = self.journey_row().is_none() || self.journey_owner() == owner;
        if !journey && self.traveling() && ends {
            self.end_trip();
        }
        self.going = Some(Going {
            owner,
            goal,
            journey,
            asked: self.autoplay.tick,
            progressed: now,
            refused: None,
            given_up: false,
        });
        // The follower takes the plan now: told first whether this walk may be flown across.
        self.steering.fly_gaps = fly;
        // While the server walks, the follower is its; the slot's walk is steered once it is done.
        if let Some(planned) = planned.filter(|_| self.move_to.is_none()) {
            self.steering.adopt(me.at, me, place, within, planned, now);
        }
        self.answer_going(me.at, place, now)
    }

    /// What the walk in the slot answers from state alone: arrived, refused, given up, or acting.
    fn answer_going(&mut self, me: Vec3, place: Place, now: Instant) -> Did {
        let unreachable = self.steering.unreachable(place.at);
        let progress = self.steering.last_progress();
        let flown = self.flies_freely();
        let Some(g) = self.going.as_mut() else {
            return Did::Acting;
        };
        if reached(me, place.at, g.goal.within()) {
            return Did::Done;
        }
        // Flown, there is no route to refuse or to make progress along.
        if flown {
            return Did::Acting;
        }
        let (owner, goal) = (g.owner, g.goal);
        if let Some(no) = unreachable {
            let why = format!("no way there: {no:?}");
            g.refused = Some(why.clone());
            let planner = match no {
                ac_nav::steering::Unreachable::Planner(p) => Some(p),
                ac_nav::steering::Unreachable::Stalled => None,
            };
            self.refuse_for(owner, goal, (me, place.at), &why, planner);
            return Did::blocked(why);
        }
        g.progressed = progress.map_or(g.progressed, |p| p.max(g.progressed));
        if now.saturating_duration_since(g.progressed) >= GIVE_UP_AFTER {
            tracing::info!(
                "walk: {} given up, no progress for {GIVE_UP_AFTER:?}",
                owner.label()
            );
            let why = "no way there: no progress";
            g.given_up = true;
            g.refused = Some(why.into());
            self.refuse_for(owner, goal, (me, place.at), why, None);
            return Did::blocked(why);
        }
        Did::Acting
    }

    /// End the walk in the slot for `owner`'s new one. A walk still followed is told, once, who
    /// took it (logged); one arrived, refused or given up has nothing left to lose.
    pub(crate) fn replace_walk(&mut self, owner: Owner, me: Vec3) {
        let followed = self.going.as_ref().is_some_and(|g| self.is_followed(g, me));
        let Some(old) = self.going.take() else {
            return;
        };
        self.steering.reset();
        // A journey's stretch asks again only once the slot is free: nothing to tell it.
        if old.owner != owner && !old.journey && followed {
            tracing::info!(
                "walk: {} takes the walk from {}",
                owner.label(),
                old.owner.label()
            );
            self.taken = Some((old.owner, owner));
        }
    }

    /// Whether `g` is still being followed from `me`: not refused, given up or arrived.
    fn is_followed(&self, g: &Going, me: Vec3) -> bool {
        g.refused.is_none()
            && !g.given_up
            && self
                .goal_place(g.goal)
                .is_some_and(|p| !reached(me, p.at, g.goal.within()))
    }

    /// The journey's stretch asked of the slot each frame (`tick_player`) for the journey's owner
    /// while the slot is free or the journey's; a step left out is planned again first, and with
    /// nothing to walk the stretch ends.
    pub(crate) fn walk_journey(&mut self, now: Instant) -> Option<Did> {
        if self.is_journey_held() {
            self.stop_stretch("held");
            return None;
        }
        if std::mem::take(&mut self.travel.replan_due) {
            return Some(self.plan_again(now));
        }
        // A corridor stretch arrived at asks the next on the same frame, once.
        let mut answered = None;
        for _ in 0..2 {
            if !self.traveling() {
                self.stop_stretch("no journey");
                return answered;
            }
            let Some(goal) = self.step_goal(now) else {
                self.stop_stretch("no stretch to walk");
                return answered;
            };
            if self.holds_walk_besides_journey() {
                return answered;
            }
            let owner = self.journey_owner();
            let did = self.ask_slot(owner, goal, now, true);
            let again = self.stretch_answered(&did);
            answered = Some(did);
            if !again {
                break;
            }
        }
        answered
    }

    /// End `owner`'s own walk, if the slot holds it (any walk with `None`), saying `why`; a
    /// journey's stretch is the journey's to end.
    pub fn stop_going(&mut self, owner: Option<Owner>, why: &str) {
        let mine = |g: &mut Going| owner.is_none_or(|o| g.owner == o && !g.journey);
        if let Some(g) = self.going.take_if(mine) {
            self.steering.reset();
            tracing::debug!("walk: {}'s let go: {why}", g.owner.label());
        }
    }

    /// End the journey's stretch, if the slot holds it, saying `why`.
    pub(crate) fn stop_stretch(&mut self, why: &str) {
        if let Some(g) = self.going.take_if(|g| g.journey) {
            self.steering.reset();
            tracing::debug!("walk: {}'s stretch let go: {why}", g.owner.label());
        }
    }

    /// Whether the slot holds a walk other than the journey's stretch still being followed.
    pub(crate) fn holds_walk_besides_journey(&self) -> bool {
        let Some(me) = self.my_position() else {
            return false;
        };
        self.going
            .as_ref()
            .is_some_and(|g| !g.journey && self.is_followed(g, me))
    }

    /// Whether the slot holds a walk being followed: neither refused nor given up.
    pub(crate) fn is_walking(&self) -> bool {
        self.going
            .as_ref()
            .is_some_and(|g| g.refused.is_none() && !g.given_up)
    }

    /// The running STEPS row as an owner (`Autoplay.running`); outside a step, `Owner::Hand`.
    pub(crate) fn step_owner(&self) -> Owner {
        self.autoplay.running.map_or(Owner::Hand, Owner::Step)
    }

    /// [`Client::go`] for `owner` to within `within` of the point `at`, on the frame's clock: the
    /// point named by its block ([`Client::place_at`]) once, then kept while it is the same walk.
    pub fn go_to(&mut self, owner: Owner, at: Vec3, within: f32) -> Did {
        let held = self
            .going
            .as_ref()
            .filter(|g| g.owner == owner)
            .and_then(|g| g.goal.place());
        let place = match held {
            Some(p) if p.at.distance(at) <= REPLAN_DISTANCE => Place { at, ..p },
            _ => self.place_at(at),
        };
        self.go(owner, Goal::At { place, within }, self.frame_at)
    }

    /// [`Client::go_to`] a map point (world xy), on the floor under it nearest the body's height.
    pub fn go_to_point(&mut self, owner: Owner, xy: Vec2, within: f32) -> Did {
        let assets = self.assets.clone();
        let Some(pl) = self.player.as_mut() else {
            return Did::waiting("not in the world yet");
        };
        let below = pl.world_position().z;
        let z = pl
            .ground_height(&assets, xy.x, xy.y, below)
            .unwrap_or(below);
        self.go_to(owner, xy.extend(z), within)
    }

    /// End a step's walk its row did not ask for this tick (end of `tick_autoplay`); a journey's
    /// stretch is the journey's, settled by [`Client::settle_journey`].
    /// Ask the slot's walk again as it stands, for this tick: false with no walk of a row's own.
    pub(crate) fn keep_walk(&mut self) -> bool {
        let tick = self.autoplay.tick;
        let Some(g) = self.going.as_mut().filter(|g| !g.journey) else {
            return false;
        };
        g.asked = tick;
        true
    }

    pub(crate) fn settle_going(&mut self) {
        let tick = self.autoplay.tick;
        if let Some(g) = self
            .going
            .take_if(|g| matches!(g.owner, Owner::Step(_)) && !g.journey && g.asked != tick)
        {
            self.steering.reset();
            tracing::debug!("walk: {} stopped asking", g.owner.label());
        }
        self.settle_journey();
    }

    /// Where the slot's walk is to this frame: the place, how near, and its cell (`tick_player`);
    /// none once given up, or once a teleport left its place out of the region, which lets it go
    /// (a_walk_left_behind_by_a_recall_is_let_go). Progress is kept from the follower's on the way.
    pub(crate) fn going_to(&mut self) -> Option<(Vec3, f32, u32)> {
        let progress = self.steering.last_progress();
        let flown = self.is_journey_flown();
        let g = self.going.as_mut().filter(|g| !g.given_up)?;
        g.progressed = progress.map_or(g.progressed, |p| p.max(g.progressed));
        let goal = g.goal;
        // A flown journey's stretch is flown past the region on purpose.
        let kept = flown && g.journey;
        if !kept && self.player.is_some() && goal.place().is_some_and(|p| !self.is_in_region(p)) {
            self.stop_going(None, "left out of the region");
            return None;
        }
        let place = self.goal_place(goal)?;
        Some((place.at, goal.within(), place.cell))
    }

    /// The place of a goal now: an object's own position, `None` once it is out of view.
    fn goal_place(&self, goal: Goal) -> Option<Place> {
        match goal {
            Goal::At { place, .. } => Some(place),
            Goal::Near { guid, .. } => {
                let o = self.world.objects.get(&guid)?;
                o.display.or(o.position).as_ref().map(Place::of_position)
            }
        }
    }

    /// The place at `at` as a walk names it by its block: indoors, the block stood in when `at`
    /// lies within its cells' bounds (a dungeon's run past its block's square), else the one under it.
    pub fn place_at(&self, at: Vec3) -> Place {
        let inside = self
            .player
            .as_ref()
            .filter(|p| p.is_indoors())
            .map(|p| p.landblock());
        let mine = inside.filter(|&b| {
            let bounds = self
                .assets
                .block_collision(b)
                .ok()
                .and_then(|bc| bc.world.bounds());
            bounds.is_some_and(|(lo, hi)| {
                (lo.x..=hi.x).contains(&at.x) && (lo.y..=hi.y).contains(&at.y)
            })
        });
        Place {
            at,
            cell: mine.unwrap_or_else(|| crate::player::block_of(at)),
        }
    }

    /// Whether a walk from where the character stands to `goal` lies in one region the planner
    /// covers: a dungeon's by cells, else the blocks round both (`routing::is_held`).
    pub(crate) fn is_in_region(&mut self, goal: Place) -> bool {
        let assets = self.assets.clone();
        let Some(pl) = self.player.as_mut() else {
            return false;
        };
        let me = Place {
            at: pl.world_position(),
            cell: pl.cell,
        };
        let dungeon = pl.is_indoors() && pl.in_dungeon(&assets);
        let blocks = if dungeon {
            vec![pl.landblock()]
        } else {
            navarea::blocks_for(me.at, goal.at)
        };
        routing::is_held(&blocks, dungeon, me, goal)
    }
}

#[cfg(test)]
mod tests;
