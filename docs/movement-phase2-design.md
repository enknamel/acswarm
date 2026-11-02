# Movement phase 2 design: the follower

Status: design, 2026-11-01, for phase 2 of `docs/movement-plan.md` ("The follower"). Nothing here is
built yet. Line anchors are on `92a9d9a` (branch `phase2`, phase 1 plus its review).

Phase 2 makes steering a follower of one planned route, and deletes every straight fallback. It
does not add `Client::go` or owners (phase 3), and does not touch journeys' legs (phase 4).
Callers keep `head_for` and the `follow` slot. What they ask is walked only along a plan, and a
"no way" reaches them as a refusal, never as a push.

The bar is phase 1's harness:

| trials | arrived | refused | gave up | still pushing | AGREE A, B, C, D |
|---|---|---|---|---|---|
| stalls (9) | 8 | 1 | 0 | 0 | 8, 0, 0, 1 |
| random (120) | 108 | 5 | 0 | 7 | 106, 0, 2, 12 |

Phase 2's target:
- **0 still pushing.** The 7 are the old steering walking at goals the planner refused; each must
  become refused.
- **Arrived no lower than 108**, except for a pair that arrived before and now answers "no way".
  Each such pair is a graph gap, fixed for every place of its kind (step 3), never by a straight
  line.
- **The invariant holds:** a pair the planner calls reachable arrives (B = 0). It becomes an
  assertion.

## 1. What the follower holds

### Types (ac-nav, `crates/ac-nav/src/steering.rs`)

`Course` (today `steering.rs:41-99`) becomes the plan being followed.

```rust
/// The route being followed: the planner's, whole, and how far along it the body is.
pub struct Course {
    /// What was asked: the goal and how near is there (metres, 3D).
    pub goal: Place,
    pub within: f32,
    /// Where the plan was made from: the start of its first leg.
    pub start: Vec3,
    /// The planner's waypoints, each with its place and the kind of edge into it.
    pub waypoints: Vec<Waypoint>,   // ac_nav::routing::Waypoint
    /// Index of the waypoint being walked at.
    pub next: usize,
    pub planned: Instant,
    /// The route still to walk (metres, 3D) at the last progress, and when that was.
    best: f32,
    since: Instant,
}
```

`Steering` (today `steering.rs:148-167`) keeps only what following needs.

```rust
pub struct Steering {
    pub route: Option<Course>,
    /// The walk asked of the planner and not answered yet: its goal, and why it was asked.
    asking: Option<(Place, f32, Replan)>,
    /// Where earlier walks to this goal stalled, and the waypoint each was aimed at.
    stalls: Vec<Stall>,
    /// The refusal standing for a goal (see `unreachable`).
    refused: Option<(Vec3, Unreachable)>,
    /// The last steer stood still for the planner.
    waiting: bool,
}
enum Replan { Start, Moved, OffCorridor, NoProgress }
struct Stall { at: Vec3, aimed: Vec3 }
/// Why the steering stands with no way: the planner's answer, or the same stall twice.
pub enum Unreachable { Planner(NoWay), Stalled }
```

- `Unreachable` is a new type name. Run `grep -rn "Unreachable\b" crates bins` first. It is not
  "Refusal", because the glossary keeps "refusal" for the server's no.

`Aim` (`steering.rs:136-144`) gains `Wait`:
- `Go(Vec3)`: walk at this point.
- `Wait`: stand. The plan has not come back yet.
- `NoWay`: stand. There is no way, and the caller is told.

Why `Wait` is needed: today a wait answers `Aim::Go(me)` (`steering.rs:409`). `player_tick`
then sets `step_cap = Some(0.0)` and `forward = 1.0`. `Player::update` and `Player::step` both
drop a cap of 0 (`player.rs:1250`, `:1407`, `.filter(|c| *c > 0.0)`), so the body walks on at its
old heading. A "wait" is really a lean. Offline every plan builds on the asking thread, so
`WAITED` was 0 and the harness never saw this.

### The `Ground` trait (`steering.rs:103-134`)

```rust
pub trait Ground {
    fn at(&self) -> Vec3;
    fn cell(&self) -> u32;
    fn is_airborne(&self) -> bool;
    /// Have the planner plan the whole walk from where the body stands to within `within` of `goal`.
    fn ask(&mut self, goal: Place, within: f32, now: Instant);
    /// The planner's answer once it is back: the place it was planned from, and the plan.
    fn answer(&mut self, goal: Place) -> Option<(Place, Planned)>;
    /// The walk test (`nav::Ground::can_walk`) from `from` to `to` for this body, on the region.
    fn can_walk(&mut self, from: Vec3, to: Vec3) -> bool;
}
```

Deleted from the trait: `block`, `line_blocked`, `line_drops`, `find_path`, `ask_wide`,
`take_wide` and `is_planning`.

`Standing` (`crates/ac-client/src/body/standing.rs`) answers the new trait:
- `ask` calls `Pathfinder::ask`, from the body's own `Place`.
- `answer` calls `Pathfinder::take`.
- `can_walk` calls `Pathfinder::can_walk`.
- `is_airborne` calls `Player::is_airborne`.

### The planner side (step 1)

All of this is in `crates/ac-client/src/pathfinder.rs` and `crates/ac-nav/src/routing.rs`.

- **`Pathfinder::take`** (`pathfinder.rs:436`) becomes
  `take(&mut self, me: Vec3, goal: Vec3) -> Option<(Place, Planned)>`. It returns the place asked
  from and the whole `Planned`, a `NoWay` with its reason included. It never flattens the answer
  to points. The `ANSWER_GOAL`/`ANSWER_REACH` drops (`:444-454`) stay.
- **`Pathfinder::ask`** (`:399`), when `builds_itself`, answers at once: `asked.answer =
  Some(self.plan(..))`.
  - So an offline walk never depends on the builder thread's timing. This is the same reason
    as `build_when_planning` (phase 1 §7).
  - The `ASK_EVERY` throttle now holds back only a repeat of the same walk: the goal within
    `ANSWER_GOAL` of the last ask's, the start within `steering::ARRIVE` of it, and sent within
    `ASK_EVERY`. A new walk, or the same goal from somewhere else, is sent at once.
  - Without this, a replan (§3) or a `reset` within 1.5 s of the last ask would stand the body
    for the rest of the window.
- **`Pathfinder::can_walk(&mut self, from: Place, to: Vec3, capsule) -> bool`** reads the region
  that holds the walk. It uses `try_read`, or `read` when `builds_itself`, and calls a new
  `routing::can_walk(area, from, to) -> bool`. That function is `graph_end` on both ends, then
  `nav::Ground::can_walk` (`crates/ac-scene/src/nav.rs:235`), the walk test every edge passed.
  The answer is `false` when there is no region or the lock is busy: then no corner is cut and no
  detour is taken. The answer is never a guess.
- **`nav::DROP_SPEED`** (`nav.rs:46`) becomes `pub`, so the follower can walk a drop at the pace
  the graph simulated it at.
- **Drops in every plan (step 2).** `routing::query_for` sets `drops: true` (`routing.rs:114`).
  `Apart` then judges on `component`, not `walk_component` (`wayfind.rs:345-348` already does this
  when `q.drops`). Every caller of the one planner (`has_way_to`, doorway, dodge, the follower)
  sees the same graph.

## 2. Following

`Steering::steer(&mut self, ground, goal: Place, within: f32, now) -> Aim` replaces
`steer(ground, goal: Vec3, goal_block: u32, stop, now)` (`steering.rs:276`). Callers already have
the cell: `player_tick.rs:162` passes `goal_cell`, and the test drivers pass the block or cell.

Each frame, in this order:
1. **A refusal stands.** If `refused` names a goal within `REPLAN_DISTANCE` of this one, the
   answer is `NoWay` and nothing is asked. A goal moved further off clears the refusal and the
   stalls.
2. **The triggers (§3).** One of them drops the course or keeps it, and sets `asking`.
3. **Asking.** While `asking` is set, call `ground.ask(goal, within, now)` every frame. The
   pathfinder sends it, or holds back an identical repeat.
4. **The answer.** Take `ground.answer(goal)`:
   - **`Route`**: `Course::new(from, goal, within, route, now)`, then `join(me)`. If the joined
     route is the stalled one (§4), the answer is `NoWay(Stalled)`.
   - **`NoWay(why)`**: `refused = Some((goal.at, Planner(why)))`, and the answer is `NoWay`.
   - **`Building`**: keep asking.
5. **The aim.**
   - With a course: `Go(course.target(me, |a, b| ground.can_walk(a, b)))`.
   - Without one: `waiting = true` and `Wait`.

### Joining a route

`Course::join(me)` sets `next` past the leg whose segment lies nearest `me`, on that leg's floor
(see "On its floor" below).
- A route asked from where the body stood joins at its first leg.
- An answer planned a few metres back, while the old route was walked meanwhile (the `Moved`
  trigger), joins where the body has got to.
- If no leg is within the corridor, the course is dropped and `OffCorridor` asks again from here.

### Waypoints passed in 3D

`Course::target` (today `steering.rs:72-87`) passes waypoint `w = waypoints[next]` when the body
is within reach on the flat and on the same floor:

- **Within reach** means flat distance `<= ARRIVE` (0.7, kept).
- **On the same floor** means `|w.z - me.z| <= wayfind::END_FLOOR` (1.5 m, the capsule's step
  down: a point further off a floor is not standing on it). This replaces `A_STOREY` (2.0,
  `steering.rs:13`). A storey is about 3 m, so the stairs test
  `a_waypoint_a_storey_above_is_not_reached_from_below` keeps its meaning.
- **The last waypoint is never passed.** The caller decides arrival: `reached` in
  `body/server_walk.rs:73` is flat `stop` and `SAME_FLOOR`.
- **Airborne**, nothing is passed. The body is on a drop or a fall, and the landing is judged
  when it lands.

### Corner cutting inside the corridor

Between `ARRIVE` and the waypoint itself, `w` is passed early (the corner is cut) only when two
things hold:
- the walk test passes from `me` to `waypoints[next + 1]`;
- `waypoints[next + 1]` is not a `Drop`.

Otherwise `w` is held until `nav::BODY_ARRIVE` (0.25 m). That is where the walk test itself
counts an edge walked, so the leg out of `w` is exactly the leg that was tested. This replaces
`ON_THE_SPOT` (`steering.rs:15`), the same 0.25 under its own name.

**Why a cut stays inside the corridor.** Take any point `p = (1-t)·me + t·w₊` on the cut
`me → w₊`. The point `q = (1-t)·w + t·w₊` lies on the planned leg `w → w₊`. Then
`|p - q| = (1-t)·|me - w| <= ARRIVE`, and `ARRIVE` is smaller than the corridor's half-width.
So the cut never leaves the corridor, and needs no corridor test of its own. Its walk test is
`can_walk`, the same function the graph's edges and the smoothing shortcuts passed
(`wayfind.rs:721`).

`can_walk` is called only in the few frames when the body is within `ARRIVE` of a waypoint.

### The corridor

`Course::is_in_corridor(p) -> bool`. A point is in the corridor when two things hold:
- **On the flat**, it lies within the half-width of the leg into `next` (from `start`, or from
  `waypoints[next - 1]`) or of the leg out of it.
- **In height**, it lies within `END_FLOOR` of that leg's height, interpolated at the nearest
  point.

The half-width is the graph's own spacing where the leg ends:
- `nav::INDOOR_SPACING` (1.5 m) for an indoor waypoint (`Place::is_indoors`);
- `nav::OUTDOOR_SPACING` (4.0 m) otherwise.

Off by more than that, the body stands where the lattice put other nodes than the route's, and
the planner's resolution can tell it apart from the route. No new number is introduced.

### Objects not in the graph

`ac_nav::obstacles::detour` stays as the per-frame walk round server clutter
(`player_tick.rs:181-194`). Its `clear` closure changes:

- **Today:** `|a, b| !pl.line_blocked(..)`.
- **Phase 2:** `|a, b| course.is_in_corridor(a) && course.is_in_corridor(b) && can_walk(a, b)`.

So a way round a chest is a walk test inside the corridor. If there is none, `detour` returns the
aim unchanged (`obstacles.rs:354-357`), and the body walks through the object, which its physics
allows (`obstacles.rs:2-4`). `Player::line_blocked` stays only for the dodge sidestep's room
check (`dodge.rs:1216`) and `examples/session_cost.rs`. Both are separate by the plan.

### Drops

A leg whose end waypoint has `kind == EdgeKind::Drop` is walked at `nav::DROP_SPEED`.
- **The pace.** `Steering::pace() -> Option<f32>` is `Some(DROP_SPEED)` while `waypoints[next]`
  is a drop.
- **The body's cap.** `Player` gains `pub pace_cap: Option<f32>` (m/s), applied in `Player::pace`
  (`player.rs:1277`) as a `min`. `player_tick` sets it to `None` each frame next to
  `walk_frame = None` (`player_tick.rs:42`), and to `steering.pace()` in the steered `Go` arm.
- **Why the cap.** The body leaves an edge at `ground_velocity = dir * pace` (`player.rs:1353`,
  `:1484`). `step_cap` shortens the step but not the speed, so only a pace cap makes the body
  step off as the graph's `fall_from` simulated it (`nav.rs:1314`).
- **No cuts.** The ledge waypoint before a drop is passed only at `BODY_ARRIVE`, never cut.

## 3. Replanning

There are three triggers, and nothing else replans. No timer replans: `REPLAN_AFTER`,
`WIDE_REPLAN_AFTER` and `LINE_CHECK` go.

| trigger | condition | constant, source | course meanwhile |
|---|---|---|---|
| `OffCorridor` | `!is_in_corridor(me)`, judged on the ground only (never airborne) | `INDOOR_SPACING`/`OUTDOOR_SPACING` (`nav.rs:15,17`) | dropped: `Wait` until the new plan |
| `Moved` | `goal.at.distance(course.goal.at) > REPLAN_DISTANCE` | `REPLAN_DISTANCE` 2.0 m (`steering.rs:17`, the plan's "a `Near` goal has moved more than 2 m") | kept and walked: it is still a checked route, and a follower of a running leader must not stop every 2 m |
| `NoProgress` | `course.remaining(me)` has not fallen by `PROGRESS` for `NO_PROGRESS` | `NO_PROGRESS` 1.5 s and `PROGRESS` 0.1 m: today's `STUCK_AFTER` and `PROGRESS` (`steering.rs:186,188`), moved and renamed | dropped and recorded as a stall (§4): `Wait` |

More detail on the triggers:
- **What progress is.** It is measured along the route. `Course::remaining` becomes 3D: from
  `me` to `waypoints[next]` and on through the rest. A detour round a chest still counts as
  progress, because it closes on the waypoint.
- **When the clock runs.** `since` restarts when a course is installed. The clock does not run
  while waiting, because there is no course then.
- **A route walked to its end.** The body stands within `BODY_ARRIVE` of the last waypoint but
  short of `reached`. `remaining` cannot fall, so `NoProgress` fires, and §4 decides.
- **The start.** With no course and no refusal, the trigger is `Start`.
- **`reset()`** (`steering.rs:263`) drops the course, `asking`, `stalls` and `refused`. The next
  steer asks at once (`Start`), as `a_steering_reset_mid_walk_plans_at_once_rather_than_walking_blind`
  wants. It never aims at the goal.

## 4. No way

The follower answers `NoWay` (stand, and `no_way()` is true) in three cases.

1. **The planner says so.** A `Planned::NoWay(why)` answer gives `Unreachable::Planner(why)`.
2. **The same route, the same stall.** This applies when the answer to a `NoProgress` replan
   joins (from where the body stands) to the same waypoints the stalled course still had. Same
   means the same count, each within `ARRIVE` of its counterpart: the follower cannot tell such
   waypoints apart. The answer is `Unreachable::Stalled`, at once, without walking it again.
3. **A stall seen before.** A new stall whose `at` and `aimed` are each within `ARRIVE` (and on
   the same floor) of an earlier stall for this goal gives `Unreachable::Stalled`. This bounds a
   cycle through two routes.

The stalls are kept until the goal moves more than `REPLAN_DISTANCE` or the steering is reset. A
refusal stands for its goal until then too
(`a_refusal_stands_until_the_next_look` becomes `a_refusal_stands_until_the_goal_moves`).

**`Building` is never a way.**
- With no course, the follower stands (`Wait`) and asks again. It never leans.
- With a course kept for a `Moved` replan, it walks that course, which is a plan.
- `is_waiting()` stays, for the harness's `WAITED`.

`Steering::unreachable(goal: Vec3) -> Option<Unreachable>` gives the refusal standing for a goal
within `REPLAN_DISTANCE`. `no_way()` stays: `autoplay/loot/walk.rs:189` and `walk_trial.rs:157`
read it.

### How a no way reaches the callers

- **`head_for` and the follow slot.**
  - `travel.rs:320-321` (`self.follow = Some(..); return Did::Acting;`) becomes one line,
    `return self.walk_toward(goal, stop);`.
  - `Client::walk_toward(goal: Vec3, stop: f32) -> Did` is new, in `body/movement.rs`. It sets
    the slot. It answers `Did::blocked(format!("no way there: {why:?}"))` while
    `steering.unreachable(goal)` is `Some`, and `Did::Acting` otherwise.
  - The slot stays set and the body stands. Clearing it would have `player_tick:46-48` reset the
    steering and replan every ask.
  - The first ask of a walk answers `Acting`, because the answer is not back yet. The next ask
    (autoplay asks every tick) gets `Blocked`. "At once" is phase 3's truthful `Did`.
- **Callers that read the `Did`.**
  - The loot walk's `did.fine()` (`autoplay/loot/walk.rs:177-195`) ends the corpse walk.
  - explore's `.acting()` (`explore.rs:342,433`), shopping (`shopping.rs:200`) and the harness
    read it too.
  - Callers that drop it (`counter.rs:235`, `team/follow.rs:116`, `dodge.rs:945`, doorway,
    academy, salvage, quartermaster) keep their own clocks until phase 3. Their body now stands
    instead of pushing.
- **Journeys (step 3).** `travel_goal` hands the steering a leg (`travel.rs:1625`). A leg with
  `steering.no_way()` makes the step's no-progress fire at once, instead of after `STUCK_AFTER`
  (10 s) or `STEP_GIVE_UP` (30 s). Two edits:
  - `travel.rs:1579`: `if now.duration_since(t) >= STUCK_AFTER || self.steering.no_way() {`
  - `travel.rs:1367`: the same `||` on the `STEP_GIVE_UP` test. This is the indoor branch that
    returns before the waypoint loop.

  They skip the waypoint, or plan the journey again, `REPLANS` times. That is the journey's own
  existing refusal path.
- **Stays separate.**
  - The dodge sidestep (`player_tick.rs:53-78`, straight and checked when chosen).
  - Manual input and noclip flight (`:140-153`).
  - doorway.rs and dodge.rs keep asking the planner for spots (`nearest_reached`,
    `doorway.rs:252`; `vantage`, `dodge.rs:778`). They then `head_for` the spot, which is now
    followed.
  - Doors never block a route; the graph walks doorways (`EdgeKind::Doorway`).

## 5. What is deleted

### From `crates/ac-nav/src/steering.rs`

| item | line | why |
|---|---|---|
| `A_STOREY` | 13 | the floor rule is `wayfind::END_FLOOR` |
| `ON_THE_SPOT` | 15 | it is `nav::BODY_ARRIVE`, the walk test's own arrival |
| `REPLAN_AFTER`, `WIDE_REPLAN_AFTER`, `LINE_CHECK` | 19, 21, 23 | timers are not triggers (§3) |
| `TRAIL_STEP`, `TRAIL_POINTS`, `TRAIL_JUMP`, `RETRACE_TRIES`, `RETRACE_STRIDE`, `RETRACE_ARRIVE` | 26-37 | the retrace trail: an unplanned walk back |
| `Course.arrive`, `Course::stale` | 50, 65-68 | used only by the retrace and the timers |
| `Ground::block`, `line_blocked`, `line_drops`, `find_path`, `ask_wide`, `take_wide`, `is_planning` | 109-133 | the straight checks and the per-block vs wide switching |
| `Steering.next_check`, `last_pos`, `last_progress`, `straight_blocked_until`, `route_is_wide`, `no_way`, `trail`, `retracing` | 151-164 | moved into `Course` (progress), `refused` (no way), or deleted |
| `retracing()`, `walked()`, `retrace()` | 209-260 | the retrace trail |
| `A_BLOCK` | 183 | "no way past a block" is the planner's `OutOfRegion` |
| `STUCK_AFTER` | 186 | renamed `NO_PROGRESS` |
| `AVOID_STRAIGHT` | 190 | there is no straight line to distrust |
| `steer` body | 284-472 | rewritten (§2). Gone with it: aim-straight-off-the-block (`:297-306`), the `fresh` patch (`:309-311`), straight-when-clear (`:353-367`), the wide-route hold (`:393-400`), the lean on a blocked line (`:411-458`), the `line_drops` lean guard (`:446-455`), and no-collision-means-clear (a `None` line test was a clear line) |
| `plan_goal`, `leaves_block`, `clip_to_block` | 487-539 | the per-block clip; one region planner sees past the block |

### Elsewhere

| item | where | why |
|---|---|---|
| `Player::line_drops` | `player.rs:1121-1138` | only the lean guard asked it |
| `nav::Ground::drops_along`, then `rises_over` and `landing_under` if nothing else calls them | `nav.rs:360-414`, `:416`, `:427` | the same; drop edges use `fall_from` |
| `Standing::line_blocked`, `line_drops`, `find_path`, `ask_wide`, `take_wide`, `is_planning`, `block` | `body/standing.rs:26-100` | the trait lost them |
| the trail feed `self.steering.walked(..)` | `body/player_tick.rs:271` | no trail |
| `WalkFrame.retracing` | `tally.rs:27`, `player_tick.rs:213`, `ac-plugin/src/telemetry.rs:103` | becomes `waiting: bool`. Telemetry's `way` reads `"waiting for its plan"`; `"straight"` and `"back the way it came"` go |

`Player::line_blocked` and the sampler `nav::Ground::walkable` stay, for the dodge sidestep only.
Phase 5 removes what nothing calls.

## 6. Tests deleted or rewritten, and why

### `steering.rs` unit tests

The `Fake` ground (`:625-682`) is rewritten. It holds:
- `answers: VecDeque<(Place, Planned)>`;
- `asks: Vec<(Vec3, Place)>`;
- `walks: bool` for `can_walk`;
- `airborne: bool`.

Waypoints are built with `routing::Waypoint`.

| test (line) | fate | reason |
|---|---|---|
| `a_goal_on_the_block_edge_is_planned_to` (546) | deleted | it tests `leaves_block`, which is gone; one region holds both blocks |
| `a_waypoint_a_storey_above_is_not_reached_from_below` (580) | kept, on `Waypoint`s and the `END_FLOOR` rule | the 3D pass |
| `waypoints_on_our_own_floor_are_passed_as_before` (590) | renamed `waypoints_on_our_own_floor_are_passed` | "as before" is history |
| `climbing_advances_one_step_at_a_time` (598) | kept | |
| `a_clear_line_is_walked_straight` (689) | rewritten `a_walk_is_aimed_at_its_plans_first_waypoint` | asserted a straight push; now it asserts a plan is asked and `Go(first waypoint)`, never the goal |
| `a_blocked_line_with_a_route_follows_the_route` (703) | folded into the one above | there is no line any more |
| `a_blocked_line_waits_for_the_planner_rather_than_leaning` (721) | rewritten `a_walk_with_no_plan_yet_waits_and_never_leans` | asserts `Aim::Wait` (step 1), then the route |
| `a_steering_reset_mid_walk_plans_at_once_rather_than_walking_blind` (750) | kept in meaning: `a_steering_reset_mid_walk_asks_again_at_once` | after `reset`, a new ask on the next steer, and never `Go(goal)`; the incident comment is cut to the rule |
| `a_character_in_a_pocket_no_path_leaves_goes_back_the_way_it_came` (775) | deleted | the retrace; replaced by `a_stall_plans_again_from_where_it_stands` and `the_same_stall_twice_is_no_way` |
| `no_route_and_a_blocked_line_nearby_still_tries` (815) | rewritten `a_planner_no_way_nearby_is_refused_not_leaned_on` | asserted a straight push; now `NoWay` and `no_way()` |
| `no_route_to_somewhere_far_is_refused_rather_than_walked_at` (834) | kept, on `NoWay::OutOfRegion` | |
| `a_goal_out_of_the_block_asks_the_wider_planner` (855) | deleted | no per-block vs wide; every walk is asked of the one planner (`a_walk_is_aimed_at_its_plans_first_waypoint` checks `asks`) |
| `a_way_found_again_clears_the_refusal` (875) | rewritten `a_goal_moved_past_the_refusal_is_asked_again` | it leaned first; now refused, the goal moves more than 2 m, asked again, a route clears it |
| `standing_off_the_edge_of_your_own_block_is_not_a_reason_to_charge` (902) | deleted | the `plan_goal == None` branch is gone; out of the region is the planner's `OutOfRegion` (kept above) |
| `no_route_and_an_edge_over_nothing_is_refused_rather_than_walked_off` (942) | deleted | its refusal is the planner's `NoWay`, tested above; there is no lean left to guard |
| `a_wall_or_a_ledge_over_a_floor_is_still_leaned_on` (954) | deleted | asserted a lean; the ledge is now a planned `Drop` (`a_drop_leg_is_walked_at_drop_pace_and_never_cut`) |
| `stairs_down_to_the_goal_are_walked_not_refused` (966) | deleted | asserted `Go(foot)` straight; stairs are a route, and the whole-loop `the_stair_corridor_is_walked_not_refused` stays |
| `a_refusal_stands_until_the_next_look` (993) | rewritten `a_refusal_stands_until_the_goal_moves` | no "next look" timer |

New tests:
- `a_corner_is_cut_only_where_the_walk_test_passes`
- `a_body_pushed_off_the_corridor_plans_again_from_where_it_stands`
- `a_near_goal_moved_past_two_metres_is_planned_again_while_the_old_route_is_walked`
- `no_progress_along_the_route_plans_again`
- `a_stall_plans_again_from_where_it_stands`
- `the_same_stall_twice_is_no_way`
- `the_same_route_from_the_stall_is_no_way_at_once`
- `a_drop_leg_is_walked_at_drop_pace_and_never_cut`
- `nothing_is_passed_in_the_air`
- `a_planner_answer_still_building_is_waited_for`

### `crates/ac-client/tests/dungeon_edges.rs`

The test-only `Standing` (`:92-160`) moves to the new trait. Its `take_wide` loop (`:146-158`) is
not needed, because `ask` answers at once offline. `steer()` (`:163`) and `chase()` (`:180`) pass
a `Place`.

| test (line) | fate |
|---|---|
| `a_straight_walk_off_a_ledge_onto_the_floor_below_is_leaned_on` (242) | rewritten `a_ledge_with_no_stairs_down_is_left_by_its_planned_drop`. The plan has a `Drop` waypoint, and the chase lands on `BELOW`'s floor. The `line_blocked`/`line_drops` asserts and "nothing leads down" go: the drop is the way down. |
| `a_straight_walk_from_the_floor_to_a_goal_up_on_a_ledge_is_refused` (268) | rewritten `a_goal_up_on_a_ledge_nothing_climbs_to_is_refused`: the plan is `NoWay`, and the steer is `NoWay` |
| `a_goal_on_the_storey_above_is_climbed_to_by_the_stairs_not_pushed_under` (285) | kept; the `line_blocked`/`line_drops` asserts go |
| `a_walk_across_the_room_is_still_a_walk` (305) | kept; the `line_drops` assert goes |
| `the_stair_corridor_is_walked_not_refused` (317) | kept; the `line_drops` asserts go; the loop handles `Wait` |
| `the_stairs_down_from_an_upper_floor_are_found_and_walked` (679) | **decision, see below** |
| the falls, long frames, server-walk and doorway tests | unchanged, with drivers on the new `steer` |

**The decision on the upper floor.**
- **What happens.** With drops in every plan, A* takes the 2.5 m step off that upper floor
  whenever it is shorter than the stairs. A drop costs its 3D length (phase 1 §4).
- **The recommendation.** Keep that cost, which is no new number. Rewrite the test as
  `the_way_down_from_an_upper_floor_is_planned_and_walked`, with three asserts:
  - the walk arrives on the goal's floor;
  - every airborne frame lies on a leg the route marks `Drop`, so the body falls only where the
    plan said;
  - the stairs are still found. A new ac-scene data test,
    `the_stairs_from_an_upper_floor_are_found_without_its_drop`, makes this check through
    `wayfind::search_building` with a raw `WalkQuery { drops: false, .. }`, as phase 1 moved the
    deck test.
- **If the user would rather characters take stairs.** That is a cost rule for drops. It must be
  stated as a rule for every drop, never tuned to this house.

### `crates/ac-client/src/travel.rs` tests

- `walk_the_journey` (`:1926`) and `drive` (`:2083`) pass a `Place` and handle `Wait`. `drive`
  drops `c.steering.walked` (`:2120`).
- `a_character_in_a_pocket_no_path_leaves_goes_back_the_way_it_came` (`:2126`) is rewritten as
  `a_character_in_a_pocket_among_props_is_planned_out_of_it`, with two asserts:
  - `plan_walk` from inside the pocket is a `Route`, because the start is pushed out by the walk
    test to `STRANDED_REACH`;
  - `drive` arrives.
  If the plan is `NoWay::Stranded`, that is a graph gap (step 2, first commits): the body walked
  in, so a walk-tested start must exist. It is never a retrace.
- The bookcase, mine, roof, scrivener stairs, shop door, Facility Hub and lifestone tests stay
  green, unloosened.

### The harness (`crates/ac-client/tests/route_harness.rs`, `src/walk_trial.rs`)

- **Refused counts the same either way** (step 1). A new `REFUSED_WITHIN: f32 = 1.0` names the
  "since < 1.0" already at `walk_trial.rs:160`.
  - A `head_for` that answers not-acting before `REFUSED_WITHIN` (`walk_trial.rs:134-143`) is
    `Refused`, as a steering no-way inside it already is.
  - Without this, the follower's refusal reaches `head_for` on the second ask (0.3 s), and the
    harness would score it "gave up".
- **The invariant** (step 1). `report` prints
  `INVARIANT planned-must-arrive: A of A+B arrived; failed: [names]`, and returns its counts.
  - `the_stalls_seen_live` and `random_walks_in_each_area` assert `planned+failed == 0`. That
    holds on phase 1 already.
  - From step 2 they also assert `still pushing == 0`.
  - The harness command's exit code is then the gate.
- **No-way pairs that arrived before** (step 2's measurement). Save phase 1's output
  (`--nocapture`) to scratch before step 1, as `phase2-baseline.txt`. The trial names are seeded
  and stable.
  - After the follower, list every trial that was `arrived` and is now `refused`.
  - Each such trial is put in `known()` under a name for its kind ("door lip", "ramp onto a
    deck", ...), so it stays a regression case.
- **No-way that arrives** (AGREE `no-way+arrived`, C) must fall to 0, because the follower cannot
  arrive without a plan. A C above 0 means `plan_first`'s fresh area and the shared region
  disagree. It is printed and investigated, not asserted.

### Other tests

`crates/ac-client/tests/graph_agrees.rs` gains `every_sampled_drop_is_landed_at_drop_pace`. This
is the existing walk with `pace_cap = Some(DROP_SPEED)`, run on the drop edges only, so the
body's fall and the graph's simulated one agree at the pace the follower uses.

## 7. Files, names and gates that bite

- **Line ceilings.**
  - `player.rs` is about 1,885 production lines against its 1,900 ceiling. `pace_cap` adds about
    6 lines in step 1, and step 2 takes about 18 out with `line_drops`.
  - `travel.rs` is at 1,800 against 1,800. Every edit there is net zero or less: `head_toward`
    goes from two lines to one; step 3's two `||` must not make rustfmt add lines. If they do,
    first make a pure-move commit that takes the free fns `clear_heading` (`:1728`) and
    `gem_next` (`:1765`) to a new `gem_room.rs`, with its code-map row.
- **New production fns:** `steer`, `target`, `join`, `is_in_corridor`, `remaining`, `pace`,
  `unreachable`, `walk_toward`, `can_walk`. None has an article, and each has four words or
  fewer. Predicates take `is_`/`can_`. New leaf modules: none, except a possible `gem_room`.
- **Comments.** Each new item gets at most two lines; the module `//!` five. No incident stories:
  the pocket and Academy stories in the old tests are cut to the rule, plus the test name.
- **Map rows.**
  - The root `CLAUDE.md` ac-nav row (`:50`) changes "steering per landblock" to "a follower of
    one planned route".
  - `crates/ac-client/CLAUDE.md` keeps its rows: `body/standing.rs` still answers `ac_nav::Ground`
    from the region planner. Both maps are at their limits, so rows are edited in place.

## 8. The three steps

Each step:
- is committable with fmt, clippy and the targeted tests green on every commit;
- runs, before its last commit, the full `cargo test-data`, rustdoc with
  `-D rustdoc::broken_intra_doc_links -D rustdoc::private_intra_doc_links`, and the harness
  (`cargo test --release -p ac-client --test route_harness -- --ignored --nocapture
  --test-threads 1`). Each exit code is checked itself, and both SUMMARY lines, AGREE, WAITED and
  INVARIANT are quoted in the step's last commit message;
- keeps comment-only edits (`cargo run -p xtask -- same-code BASE`), pure moves and code in
  separate commits, staged by path.

### Step 1: the planner answers a follower (no walk changes)

Commits:
1. **`Aim::Wait`.**
   - The old steering's planning branch (`steering.rs:401-410`) returns `Wait`.
   - `player_tick` stands on `Wait`, as on `NoWay` (`player_tick.rs:229`).
   - Every exhaustive match gains the arm (`dungeon_edges.rs:298,697`). The `if let Go` sites
     (`travel.rs:1948,2109`, `examples/walksim.rs:150`) stand on it as they are.
   - Test: `a_blocked_line_waits_for_the_planner_rather_than_leaning` asserts `Aim::Wait`.
2. **The planner side** (§1):
   - `Pathfinder::take` returns `(Place, Planned)`; old `Standing::take_wide` maps it to points.
   - `ask` answers offline at once, and throttles only a repeat.
   - `Pathfinder::can_walk` and `routing::can_walk`; `nav::DROP_SPEED` made `pub`.
   - Tests (pathfinder tests): `an_offline_ask_is_answered_before_the_next_frame`,
     `a_new_walk_is_asked_at_once_and_a_repeat_waits`.
   - Data test: `can_walk_answers_the_walk_test_on_the_region`, at the bookcase spot: across the
     room is true, onto the bookcase is false.
3. **`Player::pace_cap`** (unused until step 2). Test: `a_pace_cap_holds_the_body_to_it`.
4. **Harness:** `REFUSED_WITHIN`, the INVARIANT line, and the `planned+failed == 0` asserts.

Targeted tests:
- `cargo test-data -p ac-nav steering`
- `-p ac-client pathfinder`
- `-p ac-client --test dungeon_edges`
- `-p ac-client --test route_harness` (one named trial)

The harness must match phase 1 outcome for outcome: 8/1/0/0 and 108/5/0/7, AGREE unchanged.

### Step 2: the follower

Commits:
1. **The gaps the data tests show, if any.**
   - First prototype the follower (uncommitted), and run the arrival tests: §6's travel tests,
     dungeon_edges, the counter deck, and the harness's asserting trials.
   - Each test that now fails because the planner says no way is a graph gap. Fix it in
     ac-scene, its own commit before the follower, for every place of its kind. Each fix has a
     synthetic unit test in `nav.rs`/`wayfind.rs`, and the harness no worse under the old
     steering.
   - Likely candidates: the pocket start (`Stranded`).
   - No test is loosened to let the follower in.
2. **The follower** (one commit, because the trait change ripples):
   - `steering.rs` rewritten (§2-§5) with its tests (§6);
   - `Standing`; `player_tick` (`Wait`, the corridor detour, `pace_cap`, no trail);
   - `walk_toward` with `travel.rs:320-321`; `routing::query_for` `drops: true`;
   - `route_marks.rs:94-96` reads `w.place.at`; `WalkFrame.waiting` and telemetry;
   - the drivers in `dungeon_edges.rs`, `travel.rs` tests and `examples/walksim.rs`;
   - `graph_agrees.rs`'s drop test;
   - the harness's `still pushing == 0` assert.
3. **Deletions:** `Player::line_drops`, `nav::Ground::drops_along` and the private helpers it
   leaves unused, and its tests `a_walk_off_a_ledge_onto_the_floor_below_is_no_drop`,
   `a_walk_off_an_edge_with_nothing_under_it_is_a_drop` and
   `a_ledge_too_tall_for_a_step_is_solid_not_an_edge_over_the_storey_below` (`nav.rs:1536,1549,
   1562`). Also any `drops_along` assert in `a_hillside_too_steep_to_climb_is_not_an_edge`
   (`:1584`), whose `walkable` asserts stay.
4. **Comment-only.**
   - `crates/ac-nav/src/lib.rs:1-5`: "route within a landblock" becomes "the follower of a
     planned route".
   - `wayfind.rs:59`, `WalkQuery.drops`: "not by the old steering" becomes "the follower walks
     them at `DROP_SPEED`".
   - `nav.rs:45` (`DROP_SPEED`) and `nav.rs:163` (`walkable`, "the dodge sidestep's room check").
   - `body/standing.rs:5-10`: the count of questions.
   - The root `CLAUDE.md` ac-nav row.

Targeted tests:
- `cargo test-data -p ac-nav`
- `-p ac-scene nav`
- `-p ac-client --test dungeon_edges`
- `--test graph_agrees`
- `-p ac-client travel::`
- `-p ac-client loot`
- `-p ac-client dodge`
- `-p ac-client doorway`
- `-p ac-client counter`

The harness is no worse when:
- 0 still pushing, and B = 0;
- arrived is at least 108 less the pairs that now refuse;
- each of those pairs is named in the commit message with its `NoWay` reason.

### Step 3: no way where a push used to arrive; journeys told

Commits:
1. **One commit per kind of gap** in the pairs step 2 listed, never per pair. For each pair:
   1. Read its phase-1 walk (`RUST_LOG=ac_client=debug,steer=trace`, the `ROUTE_TRIALS` filter)
      for where the old lean got through.
   2. Find the edge the graph refuses there: a door lip, a ramp's slope, a missing drop, a
      block-edge seam in the merged collision, or a floor rule.
   3. Fix that rule in ac-scene, with a synthetic test of the kind.
   4. Put the pair in `known()`.
   - A pair whose body cannot get out of where it stands is a physics rule (a step taken must be
     undoable), with the pair as its test.
   - A pair the planner rightly refuses (a sealed floor) is left refused, with the reason in the
     commit message.
2. **Journeys** (§4): the two `|| self.steering.no_way()` edits in `travel_goal`. Data test:
   `a_journey_leg_with_no_way_is_planned_again_at_once`, which is the `walk_the_journey` driver
   on a leg ending on a goal the planner refuses.
3. **Docs:**
   - `docs/movement-plan.md` gets the "Phase 2" row: both SUMMARY lines, AGREE, WAITED and the
     refused pairs with their reasons.
   - This file's status line, with what departed from it.

The harness is no worse than step 2; arrived is back to at least 108, or each pair short of it
is answered by a refused-rightly reason.

Not in this run: `tools/scenarios.sh` and `tools/load-test.sh` need the live server. They come
before the merge to main, as the working agreement asks.
