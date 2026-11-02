# Movement: plan the whole route, then follow it

Status: proposed 2026-11-01; phases 0-3 merged (main `0db2f2f`); phase 4 built on branch `phase4`,
not merged.
Replaces spot fixes to walking with one design.

## The rule

Nothing moves the character toward anything without a complete route first.

- A request names where to be: a place (position with its floor and cell), within some distance.
- The planner answers with every waypoint from where the character stands to there: the cell and
  height of each one, and the kind of step it is (walk, drop, portal, recall, gem). Otherwise it
  answers "no way", before a single step is taken.
- The body follows that route and nothing else. When it cannot (off the route, no progress), the
  route is planned once more from where it stands; the same answer again is "no way" to whoever
  asked.
- There is never a straight line "to see if it works".

## Why: what the review found

Five read-only reviews (data layer, steering, journeys and body, every caller, history) on 1d791eb.

**Steering goes straight first, and pushes when there is no path.**
- `Steering::steer` aims straight whenever a chest-height line looks clear, dropping a good route
  to do it (`ac-nav/src/steering.rs:343-350`).
- It looks for a path only on one landblock's graph. With none found, it walks straight at the
  goal on purpose (`steering.rs:429-431`, test `no_route_and_a_blocked_line_nearby_still_tries`).
- It also aims straight in three other places (`:273-283`, `:444`, and when no collision is
  loaded).
- Nothing in steering ever gives up: lean, stuck 1.5 s, lean again.

**Most walks have no route at all.**
- `head_for` turns any goal under 192 m into the `follow` slot (`ac-client/src/travel.rs:314`).
- That slot has no plan and no cell, and always answers `Acting`, so a caller cannot tell "no
  way" from "walking".
- About 11 callers use it: corpses, counters, explore, fight approach, follow, keep_to_area,
  salvage, quartermaster, academy.

**Points are flat.**
- `Step::Walk(Vec2)` carries no cell and no height (`ac-world/src/trip.rs:75`).
- An indoor or dungeon journey step is `[me, target]`, at the character's own height
  (`travel.rs:763`, `:1418`).
- `resume_trip` keeps only x,y (`autoplay/journey.rs:12`). So does keep_to_area's goal
  (`growth/hunt.rs:269`).
- Arrival is judged on the flat (`travel.rs:1409`).
- Eleven past fixes were "wrong floor", the latest being 86f9f70.

**Three planners, three opinions.**
- `worldroute` knows terrain only.
- The per-block `NavGraph` sees one block.
- `navarea::Area` covers 3x3 blocks, asynchronously.
- They treat goal height, dungeon cells and block edges differently.
- `NavGraph::find_path` takes no cell, snaps to a node up to 12 m and 2 m of height away, then
  appends the goal unchecked (`ac-scene/src/nav.rs:848-858`).

**The graph and the body disagree on what is walkable.**

| rule | graph | body |
|---|---|---|
| floor tracking | ±1.5 m | +0.6 / −1.5 m |
| door frames and lips | refused | walked |
| low overhangs | refused | pushed aside |
| collision blocks seen | one block | two blocks |
| triangles of normal.z 0.5-0.6 | both floor and wall | both floor and wall |

Portal berths are baked into the cached graph by whichever walk built the chunk (`nav.rs:97`,
`blockcache.rs:53-61`).

**Nobody owns the goal or the decision to give up.**
- About 14 call sites, 5 movers, 12 different stuck and arrival rules, 4 with none at all.
- Systems clear each other's `follow` slot.
- A near walk silently ends a journey.

**The history.**
- About 40 movement fixes since September, mostly spot patches.
- The walk out of a building was fixed four times and two cases are still open.
- The bookcase spot was fixed, cut back for cost, then came back through explore.
- The "goal on the ground" rule (143a153) caused the vendor-upstairs and roof bugs.

All the data a full route needs is already loaded: physics polygons, cell BSPs, which cells
connect through which portals, and floors by cell. What is missing is one query contract, one
graph scope, and one meaning of "walkable".

## The design

### A place has a floor (ac-nav vocabulary)

- `Place { at: Vec3, cell: u32 }` is the only way to say where.
  - Every request, `Step::Walk`, `resume_trip`, a corpse spot, a counter or a ground carries one.
  - The cell is an interior cell (`>= 0x100`), or the outdoor cell under the point.
- A `Goal` is one of:
  - `At { place, within }`;
  - `Near { guid, within }`, resolved to a place each time it is planned;
  - later `InSight { guid, range }`, with the vantage search moved in from `dodge.rs:842`.

### One graph per region (ac-scene)

- **Regions.**
  - A dungeon is a region: all of its cells, wherever they lie relative to the landblock square.
  - Outdoors, a region is the merged area around the character (today's `navarea::Area`),
    including the inside of every building in it.
  - This replaces both the per-block `NavGraph` as a planner and the separate async wide planner.
    There is one planner, and it is the one that sees the most.
- **Ends are resolved by cell.**
  - A place is resolved to a graph node on that cell's own floor (point in cell, then the floor
    of that cell at that x,y), never to the nearest node by distance.
  - A start that resolves to nothing is pushed out to the nearest node that is a checked walk away.
  - A goal is a set of nodes within `within` of the place on its floor. The route ends at one of
    them, and the last leg is checked like every other.
  - Nothing unchecked is appended.
- **One walk test.**
  - Every edge is accepted by the body's own step: `CollisionWorld::walk` with the character's
    capsule, the same function and thresholds the body uses each frame. A route the graph gives
    is then a route the body can walk.
  - The graph's `walkable` sampler and the indoor-only `body_reaches` exception go.
  - The floor and wall slope thresholds are made distinct.
- **Edge kinds.**
  - Walk, drop (a ledge the body can step off; `drops_along` and `landing_under` already find the
    landing) and doorway.
  - Doorways are walked straight through, per the project rule.
- **Asked when walking.**
  - Portal berths and objects in the way become a filter on each query, not baked into cached
    nodes.
- **Reachable or not, before moving.**
  - The nodes of a built region are labelled with connected components. "Is that reachable from
    here" is a lookup, and "no way" is known before the first step.
  - Chunks still build lazily. A query that needs an unbuilt chunk builds it.

### Journeys plan every walk too (ac-world::trip with ac-nav)

- The trip planner keeps its job: which portals, recalls and gems join the regions.
- Each step ends at a `Place` with its cell, so the portal's exit cell and the goal's cell are
  carried through.
- When a journey starts, the walk through the current region is planned in full, start to step
  end. A journey whose first walk has no route is refused then, not after 30 s against a wall.
- Each later region's walk is planned in full when the character arrives there. Those regions'
  collision is not loaded until then. If one has no route, the journey is planned again from
  there with that step left out.
- Long overland walks keep `worldroute` as a corridor across the map. The stretch inside the
  loaded area is planned on the area graph, through buildings, fences and bridges, to the
  corridor point where it leaves the area, then planned on again.
- This replaces legs (`leg_end`), `steering_block`, `leg_height`, `step_z`, `step_cell`,
  `floor_in_cell`, the waypoint skip and the ring of points around a target.

### The follower (ac-nav steering, rewritten)

- It holds a plan and aims only at the plan's next waypoint.
- A waypoint is passed in 3D: within reach on the flat and on the same floor.
- It may cut corners only where the walk test says the shortcut is inside the route's corridor.
- Objects not in the graph (server clutter, people) get the existing per-frame detour, kept
  inside the corridor.
- **It replans when:**
  - the character is pushed off the corridor (a dodge, a server correction, carried off);
  - a `Near` goal has moved more than 2 m;
  - there has been no progress along the route for `NO_PROGRESS`.
- **It answers "no way" when:** a replan from the current spot gives the same route and the same
  stall, or the planner says no way.
- **It deletes:**
  - straight-when-clear, lean-when-blocked, aim-straight-off-the-block, and no-collision-means-clear;
  - the retrace trail, `AVOID_STRAIGHT` and the `fresh` patch;
  - the `line_drops` lean guard, and the per-block versus wide switching.

### One movement slot with one owner (ac-client)

- `Client::go(owner, goal) -> Did` replaces `head_for`, `head_toward`, the `follow` slot,
  `travel_to`/`travel_to_in`/`travel_about` as walking entry points, and the 192 m split.
  - `owner` is the STEPS row asking.
  - `Did` is truthful: `Acting` while the route is being followed; `blocked("no way ...")` when
    the planner says so, at once; `Done` on arrival in 3D.
  - Asking again with the same goal is cheap and does not replan.
- **Ownership is explicit.**
  - A request from another owner replaces the walk and is logged. The replaced owner's next ask
    says "taken by <owner>".
  - The walk ends when its owner stops asking: the step that asked loses the tick, so a walk can
    no longer outlive its step.
- **One give-up policy**, in the slot. Callers keep their own reasons to stop that are not about
  movement (no damage on a target, a counter's own refusals), but none of them times a walk
  again.
- **Stays separate:**
  - the dodge sidestep, which is time-critical and has its room checked when chosen;
  - manual input;
  - the server's own MoveTo during a use (ACE drives it);
  - noclip flight.
  - Before every use, the slot walks to use range, as the corpse, counter and visit paths already
    do, so the server MoveTo is rarely needed.

## How it is measured, not argued

The general case is tested, not spots.

1. **A route harness** (`ac_client::testkit`, data tests). It drives the real autoplay walk, the
   follower and the body at 20 Hz, offline, from a start place to a goal place. A run's outcome
   is one of:
   - **arrived**;
   - **refused up front**;
   - **gave up**, with how long it took;
   - **still pushing at the time limit**, which is always a failure.
2. **Random pairs.** In a fixed set of areas, run seeded random standable start and goal pairs
   through the harness:
   - areas: Holtburg town and its buildings, the Holtburg Dungeon, the Facility Hub, the ACB5
     mine, dungeon 0x0286, Nanto, the Arwic gate, a coast block edge;
   - scores: the share that arrive, and the share refused where a route did exist;
   - invariants: a pair the planner calls reachable must arrive; never "still pushing".
3. **The known spots.** Every stall in docs/bugs.md and in the history table becomes one case in
   the same harness, as a regression list rather than a fix list.
4. **Live.** `tools/scenarios.sh` before every merge, no worse than main.

The harness runs on main first, so every phase has a baseline to beat.

## Baseline (phase 0, 2026-11-01, main's movement code)

`AC_DATA_DIR=... cargo test --release -p ac-client --test route_harness -- --ignored --nocapture
--test-threads 1` (`ROUTE_TRIALS=name` for some). Each phase is measured against the last.

| trials | arrived | refused | gave up | still pushing |
|---|---|---|---|---|
| the stalls seen live (9) | 7 | 1 (2CB5 coast edge) | 0 | 1 (9722 Jeweler from inside) |
| random walks, 8 areas x 15 (120) | 108 | 5 | 0 | 7 |

- **Facility Hub (8A02):** 1 refused, 4 still pushing.
- **0286:** 3 refused, 2 still pushing.
- **Holtburg Dungeon (01F6):** 1 refused, 1 still pushing.
- **Every outdoor area:** all arrived (Holtburg, Oolutanga's Refuge, Qalaba'r, B2A1, A9B2).

"Refused" is not yet known to be wrong: the points are drawn from collision floors, some of them
sealed off. "Still pushing" is always wrong.

**Phase 1** (branch `phase1`, the old steering asking the region planner, step 4):

| trials | arrived | refused | gave up | still pushing | AGREE planned+arrived, planned+failed, no-way+arrived, no-way+failed |
|---|---|---|---|---|---|
| the stalls seen live (9) | 8 | 1 (2CB5 coast edge) | 0 | 0 | 8, 0, 0, 1 |
| random walks (120) | 108 | 5 | 0 | 7 | 108, 0, 0, 12 |

- The planner and the body never disagree on a route: every planned pair arrives, and every
  pair that fails was answered "no way" before a step.
- The 7 still pushing are all among the 12 no-way pairs: the old steering walks at what the
  planner refused. Phase 2's follower stops there.
- After review (floors of overlapping cells, walk components, smoothing that halves, the old
  steering waiting for its plan; `docs/movement-phase1-design.md` §7): the same, outcome for
  outcome, and 0 frames waited for the planner offline.

**Phase 2** (branch `phase2`, the follower of one planned route, `docs/movement-phase2-design.md`):

| trials | arrived | refused | gave up | still pushing | AGREE planned+arrived, planned+failed, no-way+arrived, no-way+failed |
|---|---|---|---|---|---|
| the stalls seen live (9) | 8 | 1 (2CB5 coast edge) | 0 | 0 | 8, 0, 0, 1 |
| random walks (120) | 108 | 12 | 0 | 0 | 108, 0, 0, 12 |

- WAITED 0 on both; INVARIANT planned-must-arrive 8 of 8 and 108 of 108. The harness asserts the
  invariant and 0 still pushing, so its exit code is the gate.
- The 7 still pushing became refused. No pair that arrived before refuses now: the 12 are the
  pairs phase 1's planner refused, and none arrived under phase 0's lean either.
- The refused pairs, the planner's reason first (`plan_first`, a fresh area by cell), then the
  follower's (the follow slot names the goal by block):

| pair | planner | follower | phase 0 |
|---|---|---|---|
| 01F6 Holtburg Dungeon #6 | Unreached | OffFloor | pushing, 28.8 m left |
| 01F6 Holtburg Dungeon #10 | Stranded | Stranded | refused |
| 8A02 Facility Hub #1 | Unreached | OffFloor | pushing, 51.9 m left |
| 8A02 Facility Hub #3 | Unreached | OffFloor | pushing, 20.2 m left |
| 8A02 Facility Hub #8 | Stranded | Stranded | refused |
| 8A02 Facility Hub #11 | Unreached | OffFloor | pushing, 21.7 m left |
| 8A02 Facility Hub #14 | Unreached | OffFloor | pushing, 35.3 m left |
| 0286 Hea dungeon #0 | Stranded | OffFloor | refused |
| 0286 Hea dungeon #2 | Stranded | Stranded | pushing, 19.9 m left |
| 0286 Hea dungeon #3 | Stranded | Stranded | refused |
| 0286 Hea dungeon #6 | Unreached | Unreached | pushing, 28.7 m left |
| 0286 Hea dungeon #13 | Unreached | OffFloor | refused |
| 2CB5 coast block edge (stalls) | OffFloor | OffFloor | refused |

- A journey's leg the steering refuses is planned again at once (`REPLANS` times, then given up),
  not after `STUCK_AFTER` or `STEP_GIVE_UP` (`a_journey_leg_with_no_way_is_planned_again_at_once`).

**Phase 3** (branch `phase3`, the slot: `Client::go` with owners, `docs/movement-phase3-design.md`;
the stalls gain the 5C49 ledge and the random areas the Tusker ground, 0x5C49):

| trials | arrived | refused | gave up | still pushing | AGREE planned+arrived, planned+failed, no-way+arrived, no-way+failed |
|---|---|---|---|---|---|
| the stalls seen live (12) | 10 | 2 (2CB5 coast edge; the 5C49 ledge) | 0 | 0 | 10, 0, 0, 2 |
| random walks, 9 areas x 15 (135) | 122 | 13 | 0 | 0 | 122, 0, 0, 13 |

- `SUMMARY 12 trials: 10 arrived, 2 refused, 0 gave up, 0 still pushing` and `SUMMARY 135 trials:
  122 arrived, 13 refused, 0 gave up, 0 still pushing`; WAITED 0 on both; INVARIANT
  planned-must-arrive 10 of 10 and 122 of 122.
- Each step's run was outcome for outcome and arrival time for arrival time the same as the one
  before it. 5C49 is 14 of 15 arrived, and #4 is refused, ending on a Tusker ledge (no graph gap:
  §1 of the design).
- Every walk is a `go` from an owner, a journey's leg included (`Owner::Journey`, `walk_leg`).
  `head_for`, `head_toward`, `walk_toward`, the `follow` slot, `drop_stale_walk` and the 192 m
  `WALKABLE` split are gone; the region the planner holds is the one boundary.

**Phase 4** (branch `phase4`, journeys: `docs/movement-phase4-design.md`; the harness gains
`journeys_across_regions`, five trials from one region into another):

| trials | arrived | refused | gave up | still pushing | AGREE planned+arrived, planned+failed, no-way+arrived, no-way+failed |
|---|---|---|---|---|---|
| the stalls seen live (12) | 10 | 2 (2CB5 coast edge; the 5C49 ledge) | 0 | 0 | 10, 0, 0, 2 |
| random walks, 9 areas x 15 (135) | 122 | 13 | 0 | 0 | 122, 0, 0, 13 |
| journeys (5), on the legs | 3 | 0 | 1 | 1 | - |
| journeys (5), phase 4 | 4 | 0 | 1 | 0 | - |

- `SUMMARY 12 trials: 10 arrived, 2 refused, 0 gave up, 0 still pushing`, `SUMMARY 135 trials:
  122 arrived, 13 refused, 0 gave up, 0 still pushing` and `SUMMARY 5 trials: 4 arrived, 0
  refused, 1 gave up, 0 still pushing`; WAITED 0 on all three (on the legs, 4518 at E632);
  INVARIANT planned-must-arrive 10 of 10 and 122 of 122. Stalls and random walks are outcome for
  outcome and time for time phase 3's at every code commit.
- Journeys, legs then phase 4: the Holtburg to Facility Hub Warden visit (into 0x8A0201F7) 19.3 s
  then 18.6 s; the A9B2 platform to Archmage Cindrue upstairs (0xA9B4011B) 41.9 s then 42.0 s;
  out of Archmage Yuzai Zhen's at Nanto (E632) still pushing, 400.8 m left, then arrived 36.6 s;
  the ACB5 mine floor to open ground 38.3 s then 37.9 s; Holtburg to the 01F6 ledge given up
  at 39.4 s, then refused on arriving in the dungeon at 34.4 s (the harness counts a refusal
  after its first second as "gave up").
- Each step carries its `Place`; each walk is planned whole before a step, a stretch along a
  `worldroute` corridor past the region; a step with no walk is left out and the trip planned
  again; the slot's `GIVE_UP_AFTER` is the one clock. A journey is its row's (a visit's too):
  held while another row acts, put down for its row to walk on when it stands aside or is passed
  over. After a hop a walk with no way is left out and the trip planned again. `floor_in_cell`
  and `has_way_to` went here.
- The review fixes (`d512833`, `fede9ea`, `5e01c0c`) change no trial: the ledge's last walk, left
  out on arriving, is planned again and refused at 34.4 s, nothing else reaching it.

## Phases (each merged alone, each measured against the last)

0. **Baseline.** The harness, random pairs, the known spots, and the numbers on current main.
   No behaviour change.
1. **Graph.**
   - `Place`; region graphs (a dungeon whole, the outdoor area); ends resolved by cell; checked
     last legs.
   - The body's walk test for edges; drop edges; berths filtered per query; component labels.
   - The old steering is unchanged, but asks the new planner. The harness shows how often the
     planner and the body agree.
2. **Follower.** Steering only follows a plan, and every straight fallback is deleted. Measured
   by the harness; any pair that now answers "no way" where it used to arrive is looked into as a
   graph gap and fixed generally.
3. **The slot.** `Client::go` with owners, and a truthful `Did`. Callers move over one subsystem
   per commit: loot walk, counter, explore, fight approach, follow, keep_to_area, salvage,
   quartermaster, academy, recovery, visit. Their walk clocks are deleted as they move.
4. **Journeys.**
   - `Step::Walk(Place)`, `resume_trip` with a place, the current region's walk planned at the
     start of a journey.
   - Legs and their height and cell guesses deleted; one give-up.
5. **Clean-up.**
   - Delete what nothing calls: the per-block planner entry, the wide pathfinder thread, retrace,
     `floor_in_cell`, `has_way_to` as a separate check.
   - Remove the patches whose cause is gone, each removal checked against the known-spots list.
   - Update the code maps in CLAUDE.md.

## Risks

- **Cost.**
  - Checking edges with the body's walk is slower than sampling a line. Built lazily per chunk
    and shared by every session in the process (as graphs are now), the cost is per area, not per
    character. Measured in phase 1 with `tools/load-test.sh` before going further.
  - Planning a whole region on each replan is bounded by how often replans happen (a few per
    walk), not by frames.
- **"No way" where leaning used to work.**
  - Some places will refuse that a push used to get through.
  - The harness lists them, and each is a gap in the graph (a missing drop, a door lip) fixed for
    every place of that kind, never a straight line put back.
- **Body traps.**
  - Spots the body can enter but not leave, like the Facility Hub at z −6.0, are physics bugs,
    not routing ones.
  - The random pairs find them. Each is fixed as a rule of the collision (for example, a step
    that is taken must be undoable) with the pair as its test.
- **Size.** About 2,000 lines change across ac-scene, ac-nav and ac-client. Phases keep each merge
  small enough to measure, and nothing is merged that the harness or the scenarios say is worse.
