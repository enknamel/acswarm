//! Steering a move-to: straight at the goal while the line is clear, else along a route from the
//! landblock's graph or the neighbourhood planner. Entry: [`Steering::steer`] over [`Ground`].
//! Server-placed objects are walked round afterwards by [`crate::obstacles::detour`].

use std::time::{Duration, Instant};

use glam::Vec3;

/// A waypoint counts as reached within this distance (metres, flat).
pub const ARRIVE: f32 = 0.7;
/// Height gap (metres) between waypoint and character that means another floor, not a step or doorsill.
const A_STOREY: f32 = 2.0;
/// Within this (metres, flat) a waypoint is passed even when the next cannot be walked to.
const ON_THE_SPOT: f32 = 0.25;
/// Re-plan when the goal has moved this far (metres) from the planned one.
pub const REPLAN_DISTANCE: f32 = 2.0;
/// Re-plan (or re-check the straight line) at least this often.
pub const REPLAN_AFTER: Duration = Duration::from_secs(2);
/// How long a neighbourhood route is kept: longer, as it cost more and already sees past the block.
pub const WIDE_REPLAN_AFTER: Duration = Duration::from_secs(6);
/// How often the straight line is re-tested while no route is needed.
const LINE_CHECK: Duration = Duration::from_millis(500);

/// The route being followed.
#[derive(Debug, Clone)]
pub struct Route {
    /// Goal the route was planned for.
    pub goal: Vec3,
    /// Waypoints in order; the last one is the goal.
    pub waypoints: Vec<Vec3>,
    /// Index of the waypoint being steered at.
    pub next: usize,
    pub planned: Instant,
}

impl Route {
    pub fn new(goal: Vec3, waypoints: Vec<Vec3>, now: Instant) -> Self {
        Route {
            goal,
            waypoints,
            next: 0,
            planned: now,
        }
    }

    /// The goal moved or the plan is old.
    pub fn stale(&self, goal: Vec3, now: Instant) -> bool {
        goal.distance(self.goal) > REPLAN_DISTANCE
            || now.duration_since(self.planned) >= REPLAN_AFTER
    }

    /// The next waypoint from `me`, passing those within [`ARRIVE`] but never the last: the caller decides arrival.
    /// A waypoint whose successor `clear(from, to)` denies is held until `ON_THE_SPOT`, so corners are not cut.
    pub fn target(&mut self, me: Vec3, mut clear: impl FnMut(Vec3, Vec3) -> bool) -> Vec3 {
        while self.next + 1 < self.waypoints.len() {
            let w = self.waypoints[self.next];
            let d = glam::Vec2::new(w.x - me.x, w.y - me.y).length();
            // Height counts: judged flat, a waypoint a storey up is "reached" from the floor below
            // (a_waypoint_a_storey_above_is_not_reached_from_below).
            if d > ARRIVE || (w.z - me.z).abs() > A_STOREY {
                break;
            }
            if d > ON_THE_SPOT && !clear(me, self.waypoints[self.next + 1]) {
                break;
            }
            self.next += 1;
        }
        self.waypoints.get(self.next).copied().unwrap_or(self.goal)
    }

    /// Flat distance (metres) still to walk from `me` through the waypoints left.
    pub fn remaining(&self, me: Vec3) -> f32 {
        let mut from = me;
        let mut total = 0.0;
        for w in &self.waypoints[self.next.min(self.waypoints.len())..] {
            total += glam::Vec2::new(w.x - from.x, w.y - from.y).length();
            from = *w;
        }
        total
    }
}

/// What the steering needs of the world and of the character in it: physics, triangles and a
/// planner thread in the client, a few rectangles in a test, so each navigation fault is a unit test.
pub trait Ground {
    /// Where the character is (world space, metres).
    fn at(&self) -> Vec3;
    /// The cell it stands in (indoors when the low word is 0x100 or more).
    fn cell(&self) -> u32;
    /// The landblock it stands in.
    fn block(&self) -> u32;
    /// Whether anything stands between these two points.
    fn line_blocked(&mut self, block: u32, from: Vec3, to: Vec3) -> bool;
    /// Whether the straight walk runs off an edge over nothing before anything solid stops it, or keeps
    /// a floor and ends a storey under or over `to`; a ledge over a floor is no drop (it lands, walks on).
    /// Unlike `line_blocked`, a wall or a ledge over a floor is safe to lean on; the void is not.
    fn line_drops(&mut self, block: u32, from: Vec3, to: Vec3) -> bool;
    /// A walkable route within one landblock, or `None` when the graph knows of none.
    fn find_path(&mut self, block: u32, from: Vec3, to: Vec3, goal_cell: u32) -> Option<Vec<Vec3>>;
    /// Ask the neighbourhood planner for a route that may leave this block; it answers via `take_wide`.
    fn ask_wide(&mut self, from: Vec3, to: Vec3, block: u32, outdoors: bool, exact_to: bool);
    /// A neighbourhood route that has come back, if one has.
    fn take_wide(&mut self, from: Vec3, to: Vec3) -> Option<Vec<Vec3>>;
}

/// Where to head this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Aim {
    /// Walk at this point: the goal or the next waypoint.
    Go(Vec3),
    /// No way there from here: stand still and let whoever set the goal choose another; heading for
    /// the goal anyway never arrives (no_route_to_somewhere_far_is_refused_rather_than_walked_at).
    NoWay,
}

/// One character's route and the throttles and stuck detection around it.
#[derive(Debug, Clone)]
pub struct Steering {
    pub route: Option<Route>,
    /// Next time the straight line is re-tested while no route exists.
    next_check: Instant,
    /// Where the character last made progress, and when.
    last_pos: Option<Vec3>,
    last_progress: Instant,
    /// The straight line is not trusted before this: its line test passed but the walk stuck.
    straight_blocked_until: Instant,
    /// The route came from the neighbourhood planner; a single-block re-plan should not swap in a worse one.
    route_is_wide: bool,
    /// The last steer answered [`Aim::NoWay`].
    no_way: bool,
}

impl Steering {
    /// Whether the last steer found no way at all to its goal: ask before calling a character slow,
    /// since that walk will not improve and a recall, portal or other shop beats a timeout.
    pub fn no_way(&self) -> bool {
        self.no_way
    }
}

/// One landblock across (metres): a goal farther off is never reached by leaning on what is in the way.
const A_BLOCK: f32 = 192.0;

/// No progress for this long while steering counts as stuck.
const STUCK_AFTER: Duration = Duration::from_millis(1500);
/// Movement below this (metres, flat) is not progress.
const PROGRESS: f32 = 0.1;
/// After sticking on the straight line, route on the graph this long before trusting the line again.
const AVOID_STRAIGHT: Duration = Duration::from_secs(8);

impl Steering {
    pub fn new(now: Instant) -> Self {
        Steering {
            route: None,
            next_check: now,
            last_pos: None,
            last_progress: now,
            straight_blocked_until: now,
            route_is_wide: false,
            no_way: false,
        }
    }

    /// Forget the route and progress history, when the goal goes away or the user takes over.
    pub fn reset(&mut self) {
        self.route = None;
        self.last_pos = None;
        self.route_is_wide = false;
        self.no_way = false;
    }

    /// Where to head this frame for `goal` (world space; `goal_block` its landblock, cell in the low word).
    /// The block's graph answers at once but is blind past the edge; the neighbourhood planner (its own
    /// thread, what finds a city gate) is asked whenever the line is blocked or the goal leaves the block.
    /// No progress for `STUCK_AFTER` drops the route and distrusts the line for `AVOID_STRAIGHT`.
    pub fn steer(
        &mut self,
        ground: &mut impl Ground,
        goal: Vec3,
        goal_block: u32,
        now: Instant,
    ) -> Aim {
        let me = ground.at();
        let block = ground.block();
        let far_goal = goal;
        // A goal in another block is aimed at where the line leaves this one, so this block's graph
        // still routes round what is in the way.
        let leaves_block = goal_block & 0xFFFF_0000 != block;
        let following_wide = self.route_is_wide && self.route.is_some();
        let goal = match plan_goal(me, goal, block, leaves_block, following_wide) {
            Some(g) => g,
            // Outside our own block's square, as in a dungeon (its cells lie off the block's side, so the
            // block comes from the cell, not the position): nothing to plan on. Near is tried, farther
            // refused (standing_off_the_edge_of_your_own_block_is_not_a_reason_to_charge).
            None => {
                self.route = None;
                self.route_is_wide = false;
                if glam::Vec2::new(goal.x - me.x, goal.y - me.y).length() > A_BLOCK {
                    tracing::debug!("route: {goal:?} is not in reach and there is no way to plan");
                    self.no_way = true;
                    return Aim::NoWay;
                }
                self.no_way = false;
                return Aim::Go(goal);
            }
        };
        match self.last_pos {
            Some(p) if glam::Vec2::new(me.x - p.x, me.y - p.y).length() < PROGRESS => {
                if now.duration_since(self.last_progress) >= STUCK_AFTER {
                    tracing::debug!(
                        "route: stuck at {:?}, re-planning",
                        me - ac_world::landblock_origin(block)
                    );
                    self.route = None;
                    self.route_is_wide = false;
                    self.next_check = now;
                    self.straight_blocked_until = now + AVOID_STRAIGHT;
                    self.last_progress = now;
                }
            }
            _ => {
                self.last_pos = Some(me);
                self.last_progress = now;
            }
        }
        // A route from the neighbourhood planner, if one has come back.
        if let Some(waypoints) = ground.take_wide(me, far_goal) {
            self.route = Some(Route::new(far_goal, waypoints, now));
            self.route_is_wide = true;
            self.next_check = now + REPLAN_AFTER;
        }
        let replan = match &self.route {
            None => now >= self.next_check,
            // A wide route stands while it leads to `far_goal`: the block planner cannot beat it, and
            // running out of waypoints is not staleness (the last is the goal; the caller decides arrival).
            Some(r) if self.route_is_wide => {
                far_goal.distance(r.goal) > REPLAN_DISTANCE
                    || now.duration_since(r.planned) >= WIDE_REPLAN_AFTER
            }
            Some(r) => r.stale(goal, now),
        };
        if replan {
            self.next_check = now + LINE_CHECK;
            let straight_ok =
                now >= self.straight_blocked_until && !ground.line_blocked(block, me, goal);
            // Blocked, or a goal past the block edge: the way round may leave the block entirely.
            if !straight_ok || leaves_block {
                ground.ask_wide(
                    me,
                    far_goal,
                    block,
                    ground.cell() & 0xFFFF < 0x100 && goal_block & 0xFFFF < 0x100,
                    // An indoor goal's height is where something stands, not a guess to drop to the ground.
                    goal_block & 0xFFFF >= 0x100,
                );
            }
            if straight_ok {
                if self.route.take().is_some() {
                    tracing::debug!("route: straight line clear again");
                }
                self.route_is_wide = false;
                self.no_way = false;
                return Aim::Go(goal);
            }
            match ground.find_path(block, me, goal, goal_block) {
                Some(waypoints) => {
                    let origin = ac_world::landblock_origin(block);
                    let local: Vec<[f32; 3]> = waypoints
                        .iter()
                        .map(|w| {
                            let l = *w - origin;
                            [
                                (l.x * 10.0).round() / 10.0,
                                (l.y * 10.0).round() / 10.0,
                                (l.z * 10.0).round() / 10.0,
                            ]
                        })
                        .collect();
                    tracing::debug!(
                        "route: {} waypoints to {:?} in {block:#010x}: {local:?}",
                        waypoints.len(),
                        goal - origin
                    );
                    self.route = Some(Route::new(goal, waypoints, now));
                    self.route_is_wide = false;
                    self.no_way = false;
                }
                None if following_wide => {
                    // The block's graph cannot see the way on (over a slope, through the next block), but
                    // the wide route stands and a fresh one is asked for: keep walking it, not at the goal.
                    tracing::debug!("route: no path in this block; keeping the wide route");
                    if let Some(r) = self.route.as_mut() {
                        r.planned = now;
                    }
                }
                None => {
                    // No path and a blocked line. In the block, leaning on the obstacle often works as the
                    // graph is coarser than the world (no_route_and_a_blocked_line_nearby_still_tries); out
                    // of it never (no_route_to_somewhere_far_is_refused_rather_than_walked_at).
                    self.route = None;
                    self.route_is_wide = false;
                    self.next_check = now + REPLAN_AFTER;
                    // Distance as well as the cell: indoors the caller passes our own block whatever the goal,
                    // since a dungeon's cells lie outside its square and nothing would plan otherwise.
                    let far = glam::Vec2::new(goal.x - me.x, goal.y - me.y).length() > A_BLOCK;
                    if leaves_block || far {
                        tracing::debug!("route: no way to {goal:?}, and it is not within reach");
                        self.no_way = true;
                        return Aim::NoWay;
                    }
                    // Leaning on an edge steps off it: fine over a floor (often the only way down the graph
                    // lacks), refused over the void outside a dungeon's rooms or under a goal a storey up
                    // (no_route_and_an_edge_over_nothing_is_refused_rather_than_walked_off).
                    if ground.line_drops(block, me, goal) {
                        tracing::debug!(
                            "route: no path to {goal:?}, and straight there runs off an edge over nothing"
                        );
                        self.no_way = true;
                        return Aim::NoWay;
                    }
                    tracing::debug!("route: no path to {goal:?}, going straight");
                    self.no_way = false;
                    return Aim::Go(goal);
                }
            }
        }
        match self.route.take() {
            Some(mut r) => {
                let aim = r.target(me, |from, to| !ground.line_blocked(block, from, to));
                self.route = Some(r);
                Aim::Go(aim)
            }
            // A refusal stands until the next look, not walked at in between
            // (a_refusal_stands_until_the_next_look).
            None if self.no_way => Aim::NoWay,
            None => Aim::Go(goal),
        }
    }

    /// The route's distance left from `me`, its goal, and when it was planned (a new plan measures
    /// afresh); `None` while heading straight for the goal.
    pub fn remaining(&self, me: Vec3) -> Option<(f32, Vec3, Instant)> {
        self.route
            .as_ref()
            .map(|r| (r.remaining(me), r.goal, r.planned))
    }
}

/// The goal to plan on: `goal` while it lies in `block`, else where the line to it leaves the block,
/// so the block's graph can still steer round what is in the way; `None` when nothing is left to plan on.
/// Never clipped while a neighbourhood route is followed, since that route crosses blocks (regression: e2a7c47).
pub fn plan_goal(
    me: Vec3,
    goal: Vec3,
    block: u32,
    leaves_block: bool,
    following_wide: bool,
) -> Option<Vec3> {
    if !leaves_block || following_wide {
        return Some(goal);
    }
    clip_to_block(me, goal, block)
}

/// Where the line from `me` to `goal` leaves `block`, pulled `INSIDE` back in; `None` when `me` is
/// outside the block, the goal is inside it, or that point is not a stride ahead.
pub fn clip_to_block(me: Vec3, goal: Vec3, block: u32) -> Option<Vec3> {
    const SIDE: f32 = 192.0;
    /// Metres inside the edge, so the graph has somewhere to stand.
    const INSIDE: f32 = 3.0;
    let origin = ac_world::landblock_origin(block);
    let (lo, hi) = (origin, origin + Vec3::new(SIDE, SIDE, 0.0));
    if me.x < lo.x || me.y < lo.y || me.x > hi.x || me.y > hi.y {
        return None;
    }
    let d = goal - me;
    let mut t = 1.0f32;
    for axis in 0..2 {
        let (p, v) = (me[axis], d[axis]);
        if v > 1e-6 {
            t = t.min((hi[axis] - p) / v);
        } else if v < -1e-6 {
            t = t.min((lo[axis] - p) / v);
        }
    }
    if t >= 1.0 {
        return None;
    }
    let at = me + d * t;
    let dir = (goal - me).normalize_or_zero();
    let edge = at - dir * INSIDE;
    // Only a point over a metre ahead is worth walking to, else the goal itself is: one underfoot or
    // behind has a walker near the edge step back to it for ever (regression: 075989f).
    let ahead = (edge - me).dot(dir);
    (ahead > 1.0).then_some(edge)
}

#[cfg(test)]
mod route_tests {
    use super::*;

    fn at(x: f32, y: f32, z: f32) -> Vec3 {
        Vec3::new(x, y, z)
    }

    /// Up a staircase: two paces of floor, three waypoints climbing, the vendor on the landing.
    fn stairs() -> Route {
        Route::new(
            at(0.0, 6.0, 3.0),
            vec![
                at(0.0, 1.0, 0.0),
                at(0.0, 2.0, 0.3),
                at(0.0, 3.0, 1.3),
                at(0.0, 4.0, 2.5),
                at(0.0, 6.0, 3.0),
            ],
            Instant::now(),
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
    fn waypoints_on_our_own_floor_are_passed_as_before() {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `Ground` answered by hand: what is blocked, what routes exist, where the character stands.
    #[derive(Default)]
    struct Fake {
        at: Vec3,
        cell: u32,
        block: u32,
        /// The straight line is blocked.
        blocked: bool,
        /// The straight line runs off an edge.
        drops: bool,
        /// What the landblock's graph answers with.
        path: Option<Vec<Vec3>>,
        /// What the neighbourhood planner answers with.
        wide: Option<Vec<Vec3>>,
        asked_wide: bool,
    }

    impl Ground for Fake {
        fn at(&self) -> Vec3 {
            self.at
        }
        fn cell(&self) -> u32 {
            self.cell
        }
        fn block(&self) -> u32 {
            self.block
        }
        fn line_blocked(&mut self, _block: u32, _from: Vec3, _to: Vec3) -> bool {
            self.blocked
        }
        fn line_drops(&mut self, _block: u32, _from: Vec3, _to: Vec3) -> bool {
            self.drops
        }
        fn find_path(
            &mut self,
            _block: u32,
            _from: Vec3,
            _to: Vec3,
            _goal_cell: u32,
        ) -> Option<Vec<Vec3>> {
            self.path.clone()
        }
        fn ask_wide(&mut self, _f: Vec3, _t: Vec3, _b: u32, _o: bool, _e: bool) {
            self.asked_wide = true;
        }
        fn take_wide(&mut self, _f: Vec3, _t: Vec3) -> Option<Vec<Vec3>> {
            self.wide.take()
        }
    }

    fn near(a: Vec3, b: Vec3) -> bool {
        a.distance(b) < 0.5
    }

    #[test]
    fn a_clear_line_is_walked_straight() {
        let mut st = Steering::new(Instant::now());
        let mut g = Fake {
            at: Vec3::new(10.0, 10.0, 0.0),
            ..Default::default()
        };
        let goal = Vec3::new(20.0, 10.0, 0.0);
        match st.steer(&mut g, goal, 0, Instant::now()) {
            Aim::Go(aim) => assert!(near(aim, goal), "{aim:?}"),
            Aim::NoWay => panic!("refused a clear walk"),
        }
    }

    #[test]
    fn a_blocked_line_with_a_route_follows_the_route() {
        let mut st = Steering::new(Instant::now());
        let corner = Vec3::new(10.0, 20.0, 0.0);
        let goal = Vec3::new(20.0, 20.0, 0.0);
        let mut g = Fake {
            at: Vec3::new(10.0, 10.0, 0.0),
            blocked: true,
            path: Some(vec![corner, goal]),
            ..Default::default()
        };
        match st.steer(&mut g, goal, 0, Instant::now()) {
            // The first waypoint, not the goal through the wall.
            Aim::Go(aim) => assert!(!near(aim, goal), "walked at the goal through a wall"),
            Aim::NoWay => panic!("refused a walk it had a route for"),
        }
    }

    #[test]
    fn no_route_and_a_blocked_line_nearby_still_tries() {
        // In the block, leaning on the obstacle often works as the graph is coarser than the world;
        // refusing it stopped a ten-metre walk across Holtburg dead.
        let mut st = Steering::new(Instant::now());
        let goal = Vec3::new(20.0, 10.0, 0.0);
        let mut g = Fake {
            at: Vec3::new(10.0, 10.0, 0.0),
            blocked: true,
            path: None,
            ..Default::default()
        };
        assert_eq!(st.steer(&mut g, goal, 0, Instant::now()), Aim::Go(goal));
        assert!(!st.no_way());
    }

    #[test]
    fn no_route_to_somewhere_far_is_refused_rather_than_walked_at() {
        // Blocked line, no route, goal a landblock or more off and often through rock: never walked at.
        let mut st = Steering::new(Instant::now());
        let mut g = Fake {
            at: Vec3::new(277.0, 47217.0, 0.0),
            cell: 0x01F6_0285,
            block: 0x01F6_0000,
            blocked: true,
            path: None,
            ..Default::default()
        };
        // Thelnoth Cort the Healer, in Holtburg, thirty kilometres off.
        let goal = Vec3::new(32478.0, 34719.0, 42.0);
        assert_eq!(
            st.steer(&mut g, goal, 0xA9B4_0111, Instant::now()),
            Aim::NoWay
        );
        assert!(st.no_way(), "the caller must be able to ask");
    }

    #[test]
    fn a_goal_out_of_the_block_asks_the_wider_planner() {
        let mut st = Steering::new(Instant::now());
        let mut g = Fake {
            at: Vec3::new(32_500.0, 34_600.0, 42.0),
            block: 0xA9B4_0000,
            blocked: true,
            ..Default::default()
        };
        // The next block over: a city wall's gate is usually not in the block you stand in.
        st.steer(
            &mut g,
            Vec3::new(32_700.0, 34_600.0, 42.0),
            0xABB3_0000,
            Instant::now(),
        );
        assert!(g.asked_wide, "never asked for a route out of the block");
    }

    #[test]
    fn a_way_found_again_clears_the_refusal() {
        let mut st = Steering::new(Instant::now());
        // In Holtburg's square, goal across the street: blocked with no route but near, so leaned on.
        let goal = Vec3::new(32_520.0, 34_600.0, 42.0);
        let mut g = Fake {
            at: Vec3::new(32_500.0, 34_600.0, 42.0),
            block: 0xA9B4_0000,
            blocked: true,
            path: None,
            ..Default::default()
        };
        assert!(matches!(
            st.steer(&mut g, goal, 0, Instant::now()),
            Aim::Go(_)
        ));
        assert!(!st.no_way());
        // The line comes clear.
        g.blocked = false;
        st.reset();
        assert!(matches!(
            st.steer(&mut g, goal, 0, Instant::now()),
            Aim::Go(_)
        ));
        assert!(!st.no_way());
    }

    #[test]
    fn standing_off_the_edge_of_your_own_block_is_not_a_reason_to_charge() {
        // A dungeon's cells lie off their block's side, so there is no edge to aim at: a town 30 km off
        // through rock is refused, something in the dungeon with it still walked to.
        let mut st = Steering::new(Instant::now());
        let mut g = Fake {
            at: Vec3::new(277.0, 47_217.0, 0.0),
            cell: 0x01F6_0285,
            block: 0x01F6_0000,
            blocked: false,
            ..Default::default()
        };
        let town = Vec3::new(32_478.0, 34_719.0, 42.0);
        assert_eq!(
            st.steer(&mut g, town, 0xA9B4_0111, Instant::now()),
            Aim::NoWay
        );
        let near_by = Vec3::new(290.0, 47_210.0, 0.0);
        st.reset();
        assert!(matches!(
            st.steer(&mut g, near_by, 0x01F6_0290, Instant::now()),
            Aim::Go(_)
        ));
    }

    /// On a Holtburg Dungeon ledge with no way down and a goal off its edge; `drops` says whether
    /// that way runs off into nothing.
    fn on_a_ledge(drops: bool) -> (Fake, Vec3) {
        let g = Fake {
            at: Vec3::new(225.0, 47_166.0, 7.2),
            cell: 0x01F6_0291,
            block: 0x01F6_0000,
            blocked: true,
            drops,
            path: None,
            ..Default::default()
        };
        (g, Vec3::new(218.0, 47_159.0, 0.0))
    }

    #[test]
    fn no_route_and_an_edge_over_nothing_is_refused_rather_than_walked_off() {
        // Leaning is for walls and ledges over a floor; this edge is over nothing, into the void.
        let mut st = Steering::new(Instant::now());
        let (mut g, goal) = on_a_ledge(true);
        assert_eq!(
            st.steer(&mut g, goal, 0x01F6_0000, Instant::now()),
            Aim::NoWay
        );
        assert!(st.no_way(), "the caller must be able to ask");
    }

    #[test]
    fn a_wall_or_a_ledge_over_a_floor_is_still_leaned_on() {
        // Same place, blocked line, no route, but a wall or a floor under the ledge: leaning is right.
        let mut st = Steering::new(Instant::now());
        let (mut g, goal) = on_a_ledge(false);
        assert_eq!(
            st.steer(&mut g, goal, 0x01F6_0000, Instant::now()),
            Aim::Go(goal)
        );
        assert!(!st.no_way());
    }

    #[test]
    fn stairs_down_to_the_goal_are_walked_not_refused() {
        // Stair corridor 0x01F6029F -> 0x01F602A3 -> 0x01F6028E, over a storey top to foot but a step
        // at a time: not a drop, and with no path leaned on like any floor.
        let top = Vec3::new(276.0, 47_152.0, 6.0);
        let foot = Vec3::new(296.0, 47_152.0, -5.9);
        let mut g = Fake {
            at: top,
            cell: 0x01F6_029F,
            block: 0x01F6_0000,
            ..Default::default()
        };
        let mut st = Steering::new(Instant::now());
        assert_eq!(
            st.steer(&mut g, foot, 0x01F6_0000, Instant::now()),
            Aim::Go(foot),
            "a clear flight is walked straight"
        );
        g.blocked = true;
        st.reset();
        assert_eq!(
            st.steer(&mut g, foot, 0x01F6_0000, Instant::now()),
            Aim::Go(foot)
        );
        assert!(!st.no_way());
    }

    #[test]
    fn a_refusal_stands_until_the_next_look() {
        // Between looks the steering answers from its last decision; a one-frame refusal had the
        // character walking at the edge for the two seconds until the next look.
        let start = Instant::now();
        let mut st = Steering::new(start);
        let (mut g, goal) = on_a_ledge(true);
        assert_eq!(st.steer(&mut g, goal, 0x01F6_0000, start), Aim::NoWay);
        let soon = start + Duration::from_millis(100);
        assert_eq!(
            st.steer(&mut g, goal, 0x01F6_0000, soon),
            Aim::NoWay,
            "walked at the edge between looks"
        );
        g.drops = false;
        let later = start + REPLAN_AFTER + Duration::from_millis(100);
        assert_eq!(st.steer(&mut g, goal, 0x01F6_0000, later), Aim::Go(goal));
    }
}
