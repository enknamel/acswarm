//! The follower of one planned route ([`Course`]): it aims only at the next waypoint, and asks the
//! planner again when pushed off the corridor, when the goal moves, or when no progress is made
//! along the route. Entry: [`Steering::steer`] over [`Ground`]. Server-placed objects are walked
//! round inside the corridor by [`crate::obstacles::detour`].

use std::time::{Duration, Instant};

use ac_scene::nav::{EdgeKind, BODY_ARRIVE, DROP_SPEED, INDOOR_SPACING, OUTDOOR_SPACING};
use ac_scene::wayfind::END_FLOOR;
use ac_world::Place;
use glam::{Vec2, Vec3};

use crate::routing::{NoWay, Planned, Route, Waypoint};

/// A waypoint counts as reached within this distance (metres, flat).
pub const ARRIVE: f32 = 0.7;
/// Plan again when the goal has moved this far (metres) from the planned one.
pub const REPLAN_DISTANCE: f32 = 2.0;
/// No progress along the route for this long plans the walk again from where the body stands.
pub const NO_PROGRESS: Duration = Duration::from_millis(1500);
/// Less than this (metres, 3D) closed along the route is not progress.
const PROGRESS: f32 = 0.1;

fn flat(v: Vec3) -> Vec2 {
    Vec2::new(v.x, v.y)
}

/// The route being followed: the planner's, whole, and how far along it the body is.
#[derive(Debug, Clone)]
pub struct Course {
    /// What was asked: the goal and how near is there (metres, 3D).
    pub goal: Place,
    pub within: f32,
    /// Where the plan was made from: the start of its first leg.
    pub start: Vec3,
    /// The planner's waypoints, each with its place and the kind of edge into it.
    pub waypoints: Vec<Waypoint>,
    /// Index of the waypoint being walked at.
    pub next: usize,
    pub planned: Instant,
    /// The route still to walk (metres, 3D) at the last progress, and when that was.
    best: f32,
    since: Instant,
}

impl Course {
    pub fn new(from: Place, goal: Place, within: f32, route: Route, now: Instant) -> Self {
        Course {
            goal,
            within,
            start: from.at,
            waypoints: route.waypoints,
            next: 0,
            planned: now,
            best: f32::INFINITY,
            since: now,
        }
    }

    /// The waypoint being walked at, or the start of a route with none.
    fn aim(&self) -> Vec3 {
        self.waypoints
            .get(self.next)
            .map_or(self.start, |w| w.place.at)
    }

    /// How far `p` lies off leg `i` (into waypoint `i`) on the flat, when it is within the leg's
    /// half-width (the graph's spacing at its end) and on the floor walked (a drop's: either end's).
    fn off_leg(&self, i: usize, p: Vec3) -> Option<f32> {
        let w = self.waypoints.get(i)?;
        let a = if i == 0 {
            self.start
        } else {
            self.waypoints[i - 1].place.at
        };
        let b = w.place.at;
        let (ab, ap) = (flat(b - a), flat(p - a));
        let t = if ab.length_squared() > 1e-6 {
            (ap.dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let off = (ap - ab * t).length();
        let half = if w.place.is_indoors() {
            INDOOR_SPACING
        } else {
            OUTDOOR_SPACING
        };
        let on_floor = match w.kind {
            EdgeKind::Drop => p.z >= a.z.min(b.z) - END_FLOOR && p.z <= a.z.max(b.z) + END_FLOOR,
            _ => (p.z - w.floor_at(a, t)).abs() <= END_FLOOR,
        };
        (off <= half && on_floor).then_some(off)
    }

    /// Whether `p` lies in the corridor: near the leg into the next waypoint or the leg out of it.
    pub fn is_in_corridor(&self, p: Vec3) -> bool {
        (self.next..self.next + 2).any(|i| self.off_leg(i, p).is_some())
    }

    /// Walk on from the leg nearest `me` on its floor; `false` when no leg's corridor holds `me`.
    pub fn join(&mut self, me: Vec3) -> bool {
        let nearest = (0..self.waypoints.len())
            .filter_map(|i| self.off_leg(i, me).map(|d| (i, d)))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((i, _)) = nearest {
            self.next = i;
            self.best = self.remaining(me);
        }
        nearest.is_some()
    }

    /// The waypoint to walk at from `me`, passing each within [`ARRIVE`] on its floor but never the
    /// last; a corner is cut only where `can_walk(me, next)` passes, and never into a drop.
    pub fn target(&mut self, me: Vec3, mut can_walk: impl FnMut(Vec3, Vec3) -> bool) -> Vec3 {
        while self.next + 1 < self.waypoints.len() {
            let w = self.waypoints[self.next].place.at;
            let d = flat(w - me).length();
            if d > ARRIVE || (w.z - me.z).abs() > END_FLOOR {
                break;
            }
            // Held until the walk test's own arrival, the leg out of it is the leg tested.
            let on = self.waypoints[self.next + 1];
            if d > BODY_ARRIVE && (on.kind == EdgeKind::Drop || !can_walk(me, on.place.at)) {
                break;
            }
            self.next += 1;
        }
        self.aim()
    }

    /// Distance (metres, 3D) still to walk from `me` through the waypoints left.
    pub fn remaining(&self, me: Vec3) -> f32 {
        let mut from = me;
        let mut total = 0.0;
        for w in &self.waypoints[self.next.min(self.waypoints.len())..] {
            total += w.place.at.distance(from);
            from = w.place.at;
        }
        total
    }

    /// The pace (m/s) the leg being walked is held to: a drop's, as the graph simulated it.
    pub fn pace(&self) -> Option<f32> {
        let w = self.waypoints.get(self.next)?;
        (w.kind == EdgeKind::Drop).then_some(DROP_SPEED)
    }

    /// Whether `me` stands at the route's end and the goal has moved since it was planned, out of
    /// reach of that end: walked whole, the route brings the body no nearer.
    fn is_spent(&self, me: Vec3, goal: Vec3, within: f32) -> bool {
        self.waypoints.last().is_some_and(|w| {
            let end = w.place.at;
            is_same_spot(me, end) && goal != self.goal.at && !is_in_reach(end, goal, within)
        })
    }

    /// The waypoints still to walk.
    fn left(&self) -> Vec<Vec3> {
        let from = self.next.min(self.waypoints.len());
        self.waypoints[from..].iter().map(|w| w.place.at).collect()
    }

    /// Whether the route still to walk from `me` has not fallen by [`PROGRESS`] for [`NO_PROGRESS`].
    fn is_stalled(&mut self, me: Vec3, now: Instant) -> bool {
        let left = self.remaining(me);
        if left < self.best - PROGRESS {
            self.best = left;
            self.since = now;
        }
        now.duration_since(self.since) >= NO_PROGRESS
    }
}

/// What the steering needs of the world and of the body in it: physics and the region planner in
/// the client, a few answers by hand in a test, so each navigation fault is a unit test.
pub trait Ground {
    /// Where the body stands (world space, metres).
    fn at(&self) -> Vec3;
    /// The cell it stands in (indoors when the low word is 0x100 or more).
    fn cell(&self) -> u32;
    /// Off its feet: on a drop or a fall.
    fn is_airborne(&self) -> bool;
    /// Have the planner plan the whole walk from where the body stands to within `within` of `goal`.
    fn ask(&mut self, goal: Place, within: f32, now: Instant);
    /// The planner's answer once it is back: the place it was planned from, and the plan.
    fn answer(&mut self, goal: Place) -> Option<(Place, Planned)>;
    /// The walk test every edge passed, from `from` to `to`, for this body on the region.
    fn can_walk(&mut self, from: Vec3, to: Vec3) -> bool;
}

/// Where to head this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Aim {
    /// Walk at this point: the next waypoint of the route.
    Go(Vec3),
    /// Stand: the plan has not come back yet. Not "no way", and never a lean at the old heading.
    Wait,
    /// No way there from here: stand still and let whoever set the goal choose another; heading for
    /// the goal anyway never arrives (no_route_to_somewhere_far_is_refused_rather_than_walked_at).
    NoWay,
}

/// Why the walk is asked of the planner again.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Replan {
    Start,
    Moved,
    OffCorridor,
    NoProgress,
}

/// Where a walk to the goal stalled, at which waypoint, and the waypoints it still had.
#[derive(Debug, Clone)]
struct Stall {
    at: Vec3,
    aimed: Vec3,
    goal: Vec3,
    left: Vec<Vec3>,
}

/// Within [`ARRIVE`] on the flat and on the same floor: two points the follower cannot tell apart.
fn is_same_spot(a: Vec3, b: Vec3) -> bool {
    flat(a - b).length() <= ARRIVE && (a.z - b.z).abs() <= END_FLOOR
}

/// Within `within` of `goal` on the flat and on its floor: as near as the planner's last leg comes.
fn is_in_reach(at: Vec3, goal: Vec3, within: f32) -> bool {
    flat(goal - at).length() <= within && (goal.z - at.z).abs() <= END_FLOOR
}

impl Stall {
    /// Whether `c`, joined, walks the waypoints this stall still had, its end where the goal has
    /// carried the stalled one: a goal that moves moves the end, not the way.
    fn is_route(&self, c: &Course) -> bool {
        let fresh = c.left();
        let n = fresh.len();
        n == self.left.len()
            && fresh.iter().zip(&self.left).enumerate().all(|(i, (a, b))| {
                if i + 1 == n {
                    is_same_spot(*a - c.goal.at, *b - self.goal)
                } else {
                    is_same_spot(*a, *b)
                }
            })
    }
}

/// Why the steering stands with no way: the planner's answer, or the same stall twice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Unreachable {
    Planner(NoWay),
    Stalled,
}

/// One body's route, the plan asked for, and the refusals standing.
#[derive(Debug, Clone, Default)]
pub struct Steering {
    pub route: Option<Course>,
    /// The walk asked of the planner and not answered yet, and why it was asked.
    asking: Option<Replan>,
    /// Where earlier walks to this goal stalled, and the waypoint each was aimed at.
    stalls: Vec<Stall>,
    /// The refusal standing for a goal (see [`unreachable`](Self::unreachable)).
    refused: Option<(Vec3, Unreachable)>,
    /// The last steer stood still for the planner.
    waiting: bool,
    /// When the route left last fell by `PROGRESS` (see [`last_progress`](Self::last_progress)).
    progressed: Option<Instant>,
}

impl Steering {
    /// Whether a refusal stands: ask before calling a character slow, since that walk will not
    /// improve and a recall, portal or other shop beats a timeout.
    pub fn no_way(&self) -> bool {
        self.refused.is_some()
    }

    /// The refusal standing for a goal within [`REPLAN_DISTANCE`] of `goal`.
    pub fn unreachable(&self, goal: Vec3) -> Option<Unreachable> {
        self.refused
            .filter(|(at, _)| at.distance(goal) <= REPLAN_DISTANCE)
            .map(|(_, why)| why)
    }

    /// Whether the last steer stood still while the planner had not answered.
    pub fn is_waiting(&self) -> bool {
        self.waiting
    }

    /// The pace (m/s) the body is held to on the leg being walked (see [`Course::pace`]).
    pub fn pace(&self) -> Option<f32> {
        self.route.as_ref().and_then(Course::pace)
    }

    /// Forget the route, the plan asked, the stalls and the refusal: the goal went away or the user
    /// took over. The next steer asks at once.
    pub fn reset(&mut self) {
        *self = Steering::default();
    }

    /// Where to head this frame for `goal`, to within `within` (metres, 3D): along the planned route
    /// only, standing while it is asked and refusing when the planner or the same stall says no way.
    pub fn steer(
        &mut self,
        ground: &mut impl Ground,
        goal: Place,
        within: f32,
        now: Instant,
    ) -> Aim {
        self.waiting = false;
        let me = ground.at();
        if let Some((at, _)) = self.refused {
            if at.distance(goal.at) <= REPLAN_DISTANCE {
                return Aim::NoWay;
            }
            self.refused = None;
        }
        self.stalls
            .retain(|s| s.goal.distance(goal.at) <= REPLAN_DISTANCE);
        if self.notice(goal, within, me, ground.is_airborne(), now) {
            return self.refuse(goal, Unreachable::Stalled);
        }
        if let Some(why) = self.asking {
            ground.ask(goal, within, now);
            if let Some((from, planned)) = ground.answer(goal) {
                if self.answered(why, me, from, goal, within, planned, now) == Aim::NoWay {
                    return Aim::NoWay;
                }
            }
        }
        match self.route.as_mut() {
            Some(c) if ground.is_airborne() => Aim::Go(c.aim()),
            Some(c) => Aim::Go(c.target(me, |a, b| ground.can_walk(a, b))),
            None => {
                self.waiting = true;
                Aim::Wait
            }
        }
    }

    /// Follow a walk the caller planned itself from `from` (`me` now), as an answer [`steer`](Self::steer)
    /// asked for: a route is walked and a no way stands, neither asked again; `Building` is asked.
    pub fn adopt(
        &mut self,
        me: Vec3,
        from: Place,
        goal: Place,
        within: f32,
        planned: Planned,
        now: Instant,
    ) -> Aim {
        self.reset();
        self.asking = Some(Replan::Start);
        self.answered(Replan::Start, me, from, goal, within, planned, now)
    }

    /// When the route left last fell by `PROGRESS` (0.1 m) on a course to this goal, across replans.
    pub fn last_progress(&self) -> Option<Instant> {
        self.progressed
    }

    /// Take the planner's answer to a walk asked for `why`: the course joined where `me` stands,
    /// the refusal, or (`Wait`) still to be asked.
    #[allow(clippy::too_many_arguments)] // reason: the answer and the walk it answers, in full
    fn answered(
        &mut self,
        why: Replan,
        me: Vec3,
        from: Place,
        goal: Place,
        within: f32,
        planned: Planned,
        now: Instant,
    ) -> Aim {
        match planned {
            Planned::Route(r) => {
                let mut c = Course::new(from, goal, within, r, now);
                if !c.join(me) {
                    tracing::debug!("route: the plan from {:?} does not pass here", from.at);
                    self.route = None;
                    self.asking = Some(Replan::OffCorridor);
                    return Aim::Wait;
                }
                if why == Replan::NoProgress && self.stalls.last().is_some_and(|s| s.is_route(&c)) {
                    return self.refuse(goal, Unreachable::Stalled);
                }
                // Planned again for a moved goal the clock runs on: chasing is no progress.
                if let (Replan::Moved, Some(old)) = (why, self.route.as_ref()) {
                    c.since = old.since;
                }
                tracing::debug!(
                    "route: {} waypoints to {:?} ({why:?})",
                    c.waypoints.len(),
                    goal.at
                );
                let aim = c.aim();
                self.route = Some(c);
                self.asking = None;
                Aim::Go(aim)
            }
            Planned::NoWay(no) => self.refuse(goal, Unreachable::Planner(no)),
            Planned::Building(_) => Aim::Wait,
        }
    }

    /// Ask for a first plan; again when the goal moved (the course kept), or off the corridor, at
    /// an end out of reach or stalled (the course dropped). `true` for a stall seen before.
    fn notice(&mut self, goal: Place, within: f32, me: Vec3, airborne: bool, now: Instant) -> bool {
        let Some(c) = self.route.as_mut() else {
            self.asking.get_or_insert(Replan::Start);
            return false;
        };
        let best = c.best;
        let stalled = c.is_stalled(me, now);
        // Nearer along the course but carried off it is not progress.
        let gained = c.best < best && best.is_finite();
        if gained && (airborne || c.is_in_corridor(me)) {
            self.progressed = Some(now);
        }
        if airborne {
            return false;
        }
        if !c.is_in_corridor(me) {
            tracing::debug!("route: off the corridor at {me:?}; planning again");
            self.route = None;
            self.asking = Some(Replan::OffCorridor);
            return false;
        }
        if c.is_spent(me, goal.at, within) {
            tracing::debug!(
                "route: walked to its end, out of reach of {:?}; planning again",
                goal.at
            );
            self.route = None;
            self.asking = Some(Replan::Moved);
            return false;
        }
        if stalled {
            tracing::debug!("route: no progress at {me:?}; planning again");
            let stall = Stall {
                at: me,
                aimed: c.aim(),
                goal: goal.at,
                left: c.left(),
            };
            self.route = None;
            self.asking = Some(Replan::NoProgress);
            let seen = self
                .stalls
                .iter()
                .any(|s| is_same_spot(s.at, stall.at) && is_same_spot(s.aimed, stall.aimed));
            self.stalls.push(stall);
            return seen;
        }
        if self.asking.is_none() && goal.at.distance(c.goal.at) > REPLAN_DISTANCE {
            self.asking = Some(Replan::Moved);
        }
        false
    }

    /// Stand with no way to `goal`, told to whoever asks until the goal moves.
    fn refuse(&mut self, goal: Place, why: Unreachable) -> Aim {
        tracing::debug!("route: no way to {:?}: {why:?}", goal.at);
        self.route = None;
        self.asking = None;
        self.refused = Some((goal.at, why));
        Aim::NoWay
    }

    /// The route's distance left from `me` (metres, 3D), its goal, and when it was planned (a new
    /// plan measures afresh); `None` while there is no route.
    pub fn remaining(&self, me: Vec3) -> Option<(f32, Vec3, Instant)> {
        self.route
            .as_ref()
            .map(|r| (r.remaining(me), r.goal.at, r.planned))
    }
}

#[cfg(test)]
mod route_tests {
    use super::*;
    use ac_scene::nav::LEG_SAMPLES;

    fn at(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3::new(x, y, z)
    }

    /// A waypoint walked into, outdoors.
    fn walk(p: Vec3) -> Waypoint {
        Waypoint {
            place: Place { at: p, cell: 0 },
            kind: EdgeKind::Walk,
            floor: None,
        }
    }

    /// A course from `start` through `points`.
    fn course(start: Vec3, points: &[Vec3]) -> Course {
        let route = Route {
            waypoints: points.iter().copied().map(walk).collect(),
        };
        let from = Place { at: start, cell: 0 };
        let goal = Place {
            at: *points.last().unwrap(),
            cell: 0,
        };
        Course::new(from, goal, 0.5, route, Instant::now())
    }

    /// Up a staircase: two paces of floor, three waypoints climbing, the vendor on the landing.
    fn stairs() -> Course {
        course(
            at(0.0, 0.0, 0.0),
            &[
                at(0.0, 1.0, 0.0),
                at(0.0, 2.0, 0.3),
                at(0.0, 3.0, 1.3),
                at(0.0, 4.0, 2.5),
                at(0.0, 6.0, 3.0),
            ],
        )
    }

    #[test]
    fn a_waypoint_a_storey_above_is_not_reached_from_below() {
        // At the foot of the stairs, under the landing: judged flat, every waypoint above is "here"
        // and the route would be thrown away in one go.
        let mut r = stairs();
        let aim = r.target(at(0.0, 6.0, 0.0), |_, _| true);
        assert_eq!(aim, at(0.0, 1.0, 0.0), "the first step, not the landing");
        assert_eq!(r.next, 0, "nothing was counted as reached");
    }

    #[test]
    fn waypoints_on_our_own_floor_are_passed() {
        let mut r = stairs();
        // On the first waypoint, on its floor: the next one is the aim.
        let aim = r.target(at(0.0, 1.0, 0.0), |_, _| true);
        assert_eq!(aim, at(0.0, 2.0, 0.3), "moved on to the next");
    }

    #[test]
    fn climbing_advances_one_step_at_a_time() {
        let mut r = stairs();
        let mut me = at(0.0, 0.0, 0.0);
        let mut seen = Vec::new();
        for _ in 0..5 {
            let aim = r.target(me, |_, _| true);
            seen.push(aim);
            me = aim;
        }
        assert_eq!(
            seen.last().copied(),
            Some(at(0.0, 6.0, 3.0)),
            "ends on the landing"
        );
        assert!(
            seen.contains(&at(0.0, 3.0, 1.3)),
            "and went up the stairs to get there: {seen:?}"
        );
    }

    #[test]
    fn a_corner_is_cut_only_where_the_walk_test_passes() {
        let corner = at(10.0, 0.0, 0.0);
        let on = at(10.0, 10.0, 0.0);
        let near = at(9.6, 0.1, 0.0);
        let mut r = course(at(0.0, 0.0, 0.0), &[corner, on]);
        assert_eq!(r.target(near, |_, _| false), corner, "held at the corner");
        assert_eq!(r.target(near, |_, _| true), on, "cut where it walks");
        // At the walk test's own arrival it is passed whatever the test says.
        let mut r = course(at(0.0, 0.0, 0.0), &[corner, on]);
        assert_eq!(r.target(at(9.9, 0.0, 0.0), |_, _| false), on);
    }

    #[test]
    fn a_leg_whose_floor_falls_ahead_of_the_line_holds_a_body_on_it() {
        // 26 m falling 5 m, the ground dropping early: two thirds of the way down by a third along.
        let mut r = course(at(0.0, 0.0, 5.0), &[at(26.0, 0.0, 0.0)]);
        let mut floor = [0.0; LEG_SAMPLES];
        for (k, z) in floor.iter_mut().enumerate() {
            *z = 5.0 * (1.0 - (k as f32 * 2.0 / (LEG_SAMPLES - 1) as f32).min(1.0)).powi(2);
        }
        r.waypoints[0].floor = Some(floor);
        let on = at(
            8.0,
            0.0,
            r.waypoints[0].floor_at(at(0.0, 0.0, 5.0), 8.0 / 26.0),
        );
        assert!(
            on.z < 5.0 - 8.0 / 26.0 * 5.0 - END_FLOOR,
            "off the line: {on:?}"
        );
        assert!(r.is_in_corridor(on), "on the floor walked");
        assert!(
            !r.is_in_corridor(on + at(0.0, 0.0, 3.0)),
            "a storey over it"
        );
        assert!(r.join(on));
    }

    #[test]
    fn a_body_off_every_leg_is_out_of_the_corridor() {
        let r = course(
            at(0.0, 0.0, 0.0),
            &[at(20.0, 0.0, 0.0), at(20.0, 20.0, 0.0)],
        );
        assert!(r.is_in_corridor(at(10.0, 3.0, 0.0)));
        assert!(!r.is_in_corridor(at(10.0, 6.0, 0.0)), "beyond the spacing");
        assert!(!r.is_in_corridor(at(10.0, 0.0, 3.0)), "a storey up");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    /// A `Ground` answered by hand: where the body stands, what the planner answers, the walk test.
    #[derive(Default)]
    struct Fake {
        at: Vec3,
        cell: u32,
        /// The planner's answers, in order, each taken by one `answer`.
        answers: VecDeque<(Place, Planned)>,
        /// Every ask: where the body stood, and the goal.
        asks: Vec<(Vec3, Place)>,
        /// What the walk test answers.
        walks: bool,
        airborne: bool,
    }

    impl Fake {
        fn standing(at: Vec3) -> Fake {
            Fake {
                at,
                walks: true,
                ..Default::default()
            }
        }

        /// The planner will answer a route from where the body stands through `points`.
        fn route(&mut self, points: &[Vec3]) {
            let from = Place {
                at: self.at,
                cell: self.cell,
            };
            self.answers
                .push_back((from, Planned::Route(route(points))));
        }

        fn no_way(&mut self, why: NoWay) {
            let from = Place {
                at: self.at,
                cell: self.cell,
            };
            self.answers.push_back((from, Planned::NoWay(why)));
        }
    }

    impl Ground for Fake {
        fn at(&self) -> Vec3 {
            self.at
        }
        fn cell(&self) -> u32 {
            self.cell
        }
        fn is_airborne(&self) -> bool {
            self.airborne
        }
        fn ask(&mut self, goal: Place, _within: f32, _now: Instant) {
            self.asks.push((self.at, goal));
        }
        fn answer(&mut self, _goal: Place) -> Option<(Place, Planned)> {
            self.answers.pop_front()
        }
        fn can_walk(&mut self, _from: Vec3, _to: Vec3) -> bool {
            self.walks
        }
    }

    fn route(points: &[Vec3]) -> Route {
        Route {
            waypoints: points
                .iter()
                .map(|p| Waypoint {
                    place: Place { at: *p, cell: 0 },
                    kind: EdgeKind::Walk,
                    floor: None,
                })
                .collect(),
        }
    }

    fn place(at: Vec3) -> Place {
        Place { at, cell: 0 }
    }

    fn secs(t0: Instant, s: f32) -> Instant {
        t0 + Duration::from_secs_f32(s)
    }

    #[test]
    fn a_walk_is_aimed_at_its_plans_first_waypoint() {
        let t0 = Instant::now();
        let mut st = Steering::default();
        let corner = Vec3::new(10.0, 20.0, 0.0);
        let goal = Vec3::new(20.0, 20.0, 0.0);
        let mut g = Fake::standing(Vec3::new(10.0, 10.0, 0.0));
        g.route(&[corner, goal]);
        assert_eq!(st.steer(&mut g, place(goal), 0.5, t0), Aim::Go(corner));
        assert_eq!(g.asks.len(), 1, "the one planner was asked: {:?}", g.asks);
        assert_eq!(g.asks[0].1.at, goal);
        // Walking it asks nothing more.
        assert_eq!(
            st.steer(&mut g, place(goal), 0.5, secs(t0, 0.05)),
            Aim::Go(corner)
        );
        assert_eq!(g.asks.len(), 1);
    }

    #[test]
    fn a_walk_with_no_plan_yet_waits_and_never_leans() {
        let t0 = Instant::now();
        let mut st = Steering::default();
        let corner = Vec3::new(10.0, 20.0, 0.0);
        let goal = Vec3::new(20.0, 20.0, 0.0);
        let mut g = Fake::standing(Vec3::new(10.0, 10.0, 0.0));
        assert_eq!(st.steer(&mut g, place(goal), 0.5, t0), Aim::Wait);
        assert!(st.is_waiting() && !st.no_way());
        // Asked again each frame until it answers, then the route is walked.
        g.route(&[corner, goal]);
        assert_eq!(
            st.steer(&mut g, place(goal), 0.5, secs(t0, 0.05)),
            Aim::Go(corner)
        );
        assert_eq!(g.asks.len(), 2);
        assert!(!st.is_waiting());
    }

    #[test]
    fn a_planner_answer_still_building_is_waited_for() {
        let t0 = Instant::now();
        let mut st = Steering::default();
        let goal = Vec3::new(20.0, 10.0, 0.0);
        let mut g = Fake::standing(Vec3::new(10.0, 10.0, 0.0));
        let from = place(g.at);
        g.answers.push_back((from, Planned::Building(vec![(0, 0)])));
        assert_eq!(st.steer(&mut g, place(goal), 0.5, t0), Aim::Wait);
        assert!(st.is_waiting() && !st.no_way());
        g.route(&[goal]);
        assert_eq!(
            st.steer(&mut g, place(goal), 0.5, secs(t0, 0.05)),
            Aim::Go(goal)
        );
    }

    #[test]
    fn a_steering_reset_mid_walk_asks_again_at_once() {
        let t0 = Instant::now();
        let mut st = Steering::default();
        let corner = Vec3::new(10.0, 20.0, 0.0);
        let goal = Vec3::new(20.0, 20.0, 0.0);
        let mut g = Fake::standing(Vec3::new(10.0, 10.0, 0.0));
        g.route(&[corner, goal]);
        assert_eq!(st.steer(&mut g, place(goal), 0.5, t0), Aim::Go(corner));
        st.reset();
        g.route(&[corner, goal]);
        assert_eq!(
            st.steer(&mut g, place(goal), 0.5, secs(t0, 0.1)),
            Aim::Go(corner),
            "planned again, not straight"
        );
        assert_eq!(g.asks.len(), 2);
    }

    #[test]
    fn a_planner_no_way_nearby_is_refused_not_leaned_on() {
        let mut st = Steering::default();
        let goal = Vec3::new(20.0, 10.0, 0.0);
        let mut g = Fake::standing(Vec3::new(10.0, 10.0, 0.0));
        g.no_way(NoWay::Apart);
        assert_eq!(
            st.steer(&mut g, place(goal), 0.5, Instant::now()),
            Aim::NoWay
        );
        assert!(st.no_way());
        assert_eq!(
            st.unreachable(goal),
            Some(Unreachable::Planner(NoWay::Apart))
        );
    }

    #[test]
    fn no_route_to_somewhere_far_is_refused_rather_than_walked_at() {
        let mut st = Steering::default();
        let mut g = Fake {
            at: Vec3::new(277.0, 47217.0, 0.0),
            cell: 0x01F6_0285,
            ..Default::default()
        };
        g.no_way(NoWay::OutOfRegion);
        // Thelnoth Cort the Healer, in Holtburg, thirty kilometres off.
        let goal = Place {
            at: Vec3::new(32478.0, 34719.0, 42.0),
            cell: 0xA9B4_0111,
        };
        assert_eq!(st.steer(&mut g, goal, 0.5, Instant::now()), Aim::NoWay);
        assert!(st.no_way(), "the caller must be able to ask");
    }

    #[test]
    fn a_refusal_stands_until_the_goal_moves() {
        let t0 = Instant::now();
        let mut st = Steering::default();
        let goal = Vec3::new(20.0, 10.0, 0.0);
        let mut g = Fake::standing(Vec3::new(10.0, 10.0, 0.0));
        g.no_way(NoWay::Unreached);
        assert_eq!(st.steer(&mut g, place(goal), 0.5, t0), Aim::NoWay);
        g.route(&[goal]);
        let nudged = goal + Vec3::new(1.0, 0.0, 0.0);
        assert_eq!(
            st.steer(&mut g, place(nudged), 0.5, secs(t0, 5.0)),
            Aim::NoWay,
            "asked again within two metres"
        );
        assert_eq!(g.asks.len(), 1);
    }

    #[test]
    fn a_goal_moved_past_the_refusal_is_asked_again() {
        let t0 = Instant::now();
        let mut st = Steering::default();
        let goal = Vec3::new(20.0, 10.0, 0.0);
        let mut g = Fake::standing(Vec3::new(10.0, 10.0, 0.0));
        g.no_way(NoWay::Unreached);
        assert_eq!(st.steer(&mut g, place(goal), 0.5, t0), Aim::NoWay);
        let moved = goal + Vec3::new(0.0, 3.0, 0.0);
        g.route(&[moved]);
        assert_eq!(
            st.steer(&mut g, place(moved), 0.5, secs(t0, 0.1)),
            Aim::Go(moved)
        );
        assert!(!st.no_way(), "a route clears it");
    }

    #[test]
    fn a_body_pushed_off_the_corridor_plans_again_from_where_it_stands() {
        let t0 = Instant::now();
        let mut st = Steering::default();
        let goal = Vec3::new(30.0, 10.0, 0.0);
        let mut g = Fake::standing(Vec3::new(10.0, 10.0, 0.0));
        g.route(&[goal]);
        assert_eq!(st.steer(&mut g, place(goal), 0.5, t0), Aim::Go(goal));
        // Carried ten metres aside, outdoors (the corridor is 4 m either side).
        g.at = Vec3::new(15.0, 20.0, 0.0);
        let back = Vec3::new(20.0, 15.0, 0.0);
        g.route(&[back, goal]);
        assert_eq!(
            st.steer(&mut g, place(goal), 0.5, secs(t0, 0.5)),
            Aim::Go(back)
        );
        assert_eq!(g.asks.last().map(|a| a.0), Some(g.at), "asked from here");
    }

    #[test]
    fn a_near_goal_moved_past_two_metres_is_planned_again_while_the_old_route_is_walked() {
        let t0 = Instant::now();
        let mut st = Steering::default();
        let corner = Vec3::new(10.0, 20.0, 0.0);
        let goal = Vec3::new(20.0, 20.0, 0.0);
        let mut g = Fake::standing(Vec3::new(10.0, 10.0, 0.0));
        g.route(&[corner, goal]);
        assert_eq!(st.steer(&mut g, place(goal), 0.5, t0), Aim::Go(corner));
        // The leader moves on 3 m; the planner has not answered yet.
        let moved = goal + Vec3::new(3.0, 0.0, 0.0);
        g.at = Vec3::new(10.0, 11.0, 0.0);
        assert_eq!(
            st.steer(&mut g, place(moved), 0.5, secs(t0, 0.2)),
            Aim::Go(corner),
            "the old route walked meanwhile"
        );
        assert_eq!(g.asks.last().map(|a| a.1.at), Some(moved));
        // Answered, from where it was asked, the new route is joined where the body has got to.
        g.answers.push_back((
            place(Vec3::new(10.0, 10.5, 0.0)),
            Planned::Route(route(&[corner, moved])),
        ));
        g.at = Vec3::new(10.0, 12.0, 0.0);
        assert_eq!(
            st.steer(&mut g, place(moved), 0.5, secs(t0, 0.3)),
            Aim::Go(corner)
        );
        assert_eq!(st.route.as_ref().map(|c| c.goal.at), Some(moved));
    }

    #[test]
    fn a_goal_drifted_out_of_reach_of_the_routes_end_is_planned_again_not_refused() {
        let t0 = Instant::now();
        let mut st = Steering::default();
        let goal = Vec3::new(41.9, 68.2, 0.0);
        let mut g = Fake::standing(Vec3::new(34.0, 68.2, 0.0));
        let end = Vec3::new(40.9, 68.2, 0.0);
        g.route(&[end]);
        assert_eq!(st.steer(&mut g, place(goal), 1.0, t0), Aim::Go(end));
        // Stood at the end in reach, the target steps 1.14 m off: under two metres, out of reach.
        g.at = end;
        let drifted = goal + Vec3::new(1.14, 0.0, 0.0);
        assert_eq!(
            st.steer(&mut g, place(drifted), 1.0, secs(t0, 1.0)),
            Aim::Wait,
            "planned again, not walked at its own feet"
        );
        let nearer = drifted - Vec3::new(0.9, 0.0, 0.0);
        g.route(&[nearer]);
        assert_eq!(
            st.steer(&mut g, place(drifted), 1.0, secs(t0, 1.05)),
            Aim::Go(nearer)
        );
        assert!(!st.no_way());
    }

    #[test]
    fn a_body_held_still_chasing_a_moving_goal_is_refused() {
        let t0 = Instant::now();
        let mut st = Steering::default();
        let mut g = Fake::standing(Vec3::new(10.0, 10.0, 0.0));
        let mut refused = None;
        for frame in 0..80 {
            let t = frame as f32 * 0.05;
            // The goal runs on 3.5 m a second; the body, held, never moves.
            let goal = Vec3::new(20.0 + 3.5 * t.floor(), 10.0, 0.0);
            g.answers.clear();
            g.route(&[goal]);
            if st.steer(&mut g, place(goal), 0.5, secs(t0, t)) == Aim::NoWay {
                refused = Some(t);
                break;
            }
        }
        assert!(refused.is_some_and(|t| t <= 2.0), "refused at {refused:?}");
        assert_eq!(
            st.unreachable(g.asks.last().unwrap().1.at),
            Some(Unreachable::Stalled)
        );
    }

    #[test]
    fn a_body_running_after_a_faster_goal_is_never_refused() {
        let t0 = Instant::now();
        let mut st = Steering::default();
        let mut g = Fake::standing(Vec3::new(10.0, 10.0, 0.0));
        for frame in 0..80 {
            let t = frame as f32 * 0.05;
            let goal = Vec3::new(20.0 + 5.0 * t.floor(), 10.0, 0.0);
            g.answers.clear();
            g.route(&[goal]);
            let aim = st.steer(&mut g, place(goal), 0.5, secs(t0, t));
            assert_ne!(aim, Aim::NoWay, "refused at {t}");
            g.at.x += 0.2;
        }
    }

    #[test]
    fn no_progress_along_the_route_plans_again() {
        let t0 = Instant::now();
        let mut st = Steering::default();
        let goal = Vec3::new(20.0, 10.0, 0.0);
        let mut g = Fake::standing(Vec3::new(10.0, 10.0, 0.0));
        g.route(&[goal]);
        assert_eq!(st.steer(&mut g, place(goal), 0.5, t0), Aim::Go(goal));
        assert_eq!(
            st.steer(&mut g, place(goal), 0.5, secs(t0, 1.0)),
            Aim::Go(goal),
            "not yet"
        );
        assert_eq!(st.steer(&mut g, place(goal), 0.5, secs(t0, 1.6)), Aim::Wait);
        assert_eq!(g.asks.len(), 2, "asked again from where it stands");
    }

    #[test]
    fn a_stall_plans_again_from_where_it_stands() {
        let t0 = Instant::now();
        let mut st = Steering::default();
        let corner = Vec3::new(10.0, 20.0, 0.0);
        let other = Vec3::new(14.0, 12.0, 0.0);
        let goal = Vec3::new(20.0, 20.0, 0.0);
        let mut g = Fake::standing(Vec3::new(10.0, 10.0, 0.0));
        g.route(&[corner, goal]);
        st.steer(&mut g, place(goal), 0.5, t0);
        g.route(&[other, goal]);
        assert_eq!(
            st.steer(&mut g, place(goal), 0.5, secs(t0, 1.6)),
            Aim::Go(other),
            "another way, walked"
        );
        assert!(!st.no_way());
    }

    #[test]
    fn the_same_route_from_the_stall_is_no_way_at_once() {
        let t0 = Instant::now();
        let mut st = Steering::default();
        let corner = Vec3::new(10.0, 20.0, 0.0);
        let goal = Vec3::new(20.0, 20.0, 0.0);
        let mut g = Fake::standing(Vec3::new(10.0, 10.0, 0.0));
        g.route(&[corner, goal]);
        st.steer(&mut g, place(goal), 0.5, t0);
        g.route(&[corner + Vec3::new(0.3, 0.0, 0.0), goal]);
        assert_eq!(
            st.steer(&mut g, place(goal), 0.5, secs(t0, 1.6)),
            Aim::NoWay
        );
        assert_eq!(st.unreachable(goal), Some(Unreachable::Stalled));
    }

    #[test]
    fn the_same_stall_twice_is_no_way() {
        let t0 = Instant::now();
        let mut st = Steering::default();
        let a = Vec3::new(10.0, 20.0, 0.0);
        let b = Vec3::new(14.0, 12.0, 0.0);
        let goal = Vec3::new(20.0, 20.0, 0.0);
        let mut g = Fake::standing(Vec3::new(10.0, 10.0, 0.0));
        // Stalled aimed at `a`, then at `b`, then at `a` again: two routes in a cycle.
        g.route(&[a, goal]);
        st.steer(&mut g, place(goal), 0.5, t0);
        g.route(&[b, goal]);
        assert_eq!(
            st.steer(&mut g, place(goal), 0.5, secs(t0, 1.6)),
            Aim::Go(b)
        );
        g.route(&[a, goal]);
        assert_eq!(
            st.steer(&mut g, place(goal), 0.5, secs(t0, 3.2)),
            Aim::Go(a)
        );
        assert_eq!(
            st.steer(&mut g, place(goal), 0.5, secs(t0, 4.8)),
            Aim::NoWay
        );
        assert_eq!(st.unreachable(goal), Some(Unreachable::Stalled));
    }

    #[test]
    fn an_adopted_route_is_followed_without_asking() {
        let t0 = Instant::now();
        let mut st = Steering::default();
        let corner = Vec3::new(10.0, 20.0, 0.0);
        let goal = Vec3::new(20.0, 20.0, 0.0);
        let mut g = Fake::standing(Vec3::new(10.0, 10.0, 0.0));
        let planned = Planned::Route(route(&[corner, goal]));
        let aim = st.adopt(g.at, place(g.at), place(goal), 0.5, planned, t0);
        assert_eq!(aim, Aim::Go(corner));
        assert_eq!(
            st.steer(&mut g, place(goal), 0.5, secs(t0, 0.05)),
            Aim::Go(corner)
        );
        assert!(
            g.asks.is_empty(),
            "planned once, by the caller: {:?}",
            g.asks
        );
    }

    #[test]
    fn an_adopted_no_way_is_refused_without_asking() {
        let t0 = Instant::now();
        let mut st = Steering::default();
        let goal = Vec3::new(20.0, 10.0, 0.0);
        let mut g = Fake::standing(Vec3::new(10.0, 10.0, 0.0));
        let planned = Planned::NoWay(NoWay::Unreached);
        let aim = st.adopt(g.at, place(g.at), place(goal), 0.5, planned, t0);
        assert_eq!(aim, Aim::NoWay);
        assert_eq!(
            st.steer(&mut g, place(goal), 0.5, secs(t0, 0.05)),
            Aim::NoWay
        );
        assert!(g.asks.is_empty());
        assert_eq!(
            st.unreachable(goal),
            Some(Unreachable::Planner(NoWay::Unreached))
        );
    }

    #[test]
    fn progress_is_kept_across_a_replan() {
        let t0 = Instant::now();
        let mut st = Steering::default();
        let goal = Vec3::new(30.0, 10.0, 0.0);
        let mut g = Fake::standing(Vec3::new(10.0, 10.0, 0.0));
        g.route(&[goal]);
        st.steer(&mut g, place(goal), 0.5, t0);
        assert_eq!(st.last_progress(), None, "a new course is no progress");
        g.at.x += 1.0;
        st.steer(&mut g, place(goal), 0.5, secs(t0, 0.5));
        assert_eq!(st.last_progress(), Some(secs(t0, 0.5)));
        // Carried off the corridor: planned again, and the new course walks nothing yet.
        g.at = Vec3::new(15.0, 20.0, 0.0);
        g.route(&[goal]);
        st.steer(&mut g, place(goal), 0.5, secs(t0, 1.0));
        st.steer(&mut g, place(goal), 0.5, secs(t0, 1.1));
        assert!(st.route.is_some(), "walking the new course");
        assert_eq!(
            st.last_progress(),
            Some(secs(t0, 0.5)),
            "kept, not restarted"
        );
        st.reset();
        assert_eq!(st.last_progress(), None);
    }

    #[test]
    fn a_drop_leg_is_walked_at_drop_pace_and_never_cut() {
        let t0 = Instant::now();
        let mut st = Steering::default();
        let ledge = Vec3::new(10.0, 10.0, 4.0);
        let landing = Vec3::new(11.5, 10.0, 0.0);
        let goal = Vec3::new(16.0, 10.0, 0.0);
        let mut g = Fake::standing(Vec3::new(5.0, 10.0, 4.0));
        let mut r = route(&[ledge, landing, goal]);
        r.waypoints[1].kind = EdgeKind::Drop;
        g.answers.push_back((place(g.at), Planned::Route(r)));
        assert_eq!(st.steer(&mut g, place(goal), 0.5, t0), Aim::Go(ledge));
        assert_eq!(
            st.pace(),
            None,
            "walked at the body's own pace to the ledge"
        );
        // Half a metre short of the ledge, the walk test passing: not cut.
        g.at = Vec3::new(9.5, 10.0, 4.0);
        assert_eq!(
            st.steer(&mut g, place(goal), 0.5, secs(t0, 1.0)),
            Aim::Go(ledge)
        );
        g.at = Vec3::new(9.9, 10.0, 4.0);
        assert_eq!(
            st.steer(&mut g, place(goal), 0.5, secs(t0, 1.1)),
            Aim::Go(landing)
        );
        assert_eq!(st.pace(), Some(DROP_SPEED));
    }

    #[test]
    fn nothing_is_passed_in_the_air() {
        let t0 = Instant::now();
        let mut st = Steering::default();
        let a = Vec3::new(10.0, 10.0, 0.0);
        let goal = Vec3::new(20.0, 10.0, 0.0);
        let mut g = Fake::standing(Vec3::new(5.0, 10.0, 0.0));
        g.route(&[a, goal]);
        st.steer(&mut g, place(goal), 0.5, t0);
        g.at = Vec3::new(10.0, 10.0, 0.5);
        g.airborne = true;
        assert_eq!(
            st.steer(&mut g, place(goal), 0.5, secs(t0, 1.0)),
            Aim::Go(a)
        );
        g.airborne = false;
        g.at = Vec3::new(10.0, 10.0, 0.0);
        assert_eq!(
            st.steer(&mut g, place(goal), 0.5, secs(t0, 1.1)),
            Aim::Go(goal)
        );
    }
}
