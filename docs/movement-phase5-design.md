# Movement phase 5 design: clean-up

Status: design, 2026-11-02, for phase 5 of `docs/movement-plan.md` ("Clean-up"). Nothing here is
built yet. Line anchors are on `da6468f` (main, phases 0-4 merged).

Phase 5 deletes what nothing calls, removes the walk patches whose cause phases 1-4 took away,
and brings the code maps up to date. No behaviour changes except a patch going, and each patch
goes in its own commit with the route harness run on it.

Rules this design keeps:
- Every walk is planned whole, 3D and cell-aware, by `ac_nav::routing`, before a step.
- The follower (`ac_nav::steering`) only follows the plan.
- No spot patches: nothing special-cased for one place, no threshold tuned to one trial, no
  straight-line fallback, no ring of guessed points.
- Arrival and reach are 3D: `server_walk::reached`, the slot's `Did::Done`.
- Doors are walked straight through.

## 0. The bar

The harness command is `cargo test --release -p ac-client --test route_harness -- --ignored
--nocapture --test-threads 1`. On `da6468f` it gives:

| suite | trials | arrived | refused | gave up | still pushing |
|---|---|---|---|---|---|
| `the_stalls_seen_live` | 12 | 10 | 2 (2CB5 coast edge, 5C49 ledge) | 0 | 0 |
| `journeys_across_regions` | 5 | 4 | 0 | 1 (01F6 ledge) | 0 |
| `random_walks_in_each_area` | 135 | 122 | 13 | 0 | 0 |

- INVARIANT planned-must-arrive holds (10 of 10, 122 of 122), and WAITED is 0.
- The bar is no worse **trial for trial**: every trial ends the same way as on main, and 0 still
  pushing, always.
- An arrival time that changes is reported in the commit message, not judged.
- The harness exit code is the gate. Check it directly, never through a pipe's tail.
- Each step's builder first runs the harness on its base and keeps the per-trial lines
  (`--nocapture` output) in scratch. Every later run is diffed against those lines.

## 1. What the plan's list names is already gone

The plan's phase 5 list was written before phases 1-4. All five of its items are gone:

| plan item | where it went | proof on `da6468f` |
|---|---|---|
| the per-block planner entry | phase 1 step 4: `Player::find_path`, `on_nav`, `Block.nav`, `BlockCollision::nav`, `NavGraph::for_scene` deleted | no match for `find_path`, `for_scene`, `on_nav` |
| the wide pathfinder thread | phase 1 step 3: the thread is now the region builder (`pathfinder.rs:576`, `build_forever`); `ask_wide`/`take_wide` are gone | `Pathfinder::ask`/`take` (`pathfinder.rs:450`, `:510`) are the follower's replans, planned off the frame |
| retrace | phase 2: the trail, `RETRACE_*`, `retracing()` and `walked()` deleted | no match for `retrace` |
| `floor_in_cell` | phase 4 | no match |
| `has_way_to` as a separate check | phase 4 | no match |

`Pathfinder::ask`/`take` stays. A replan on the frame cost 21 ms at p95 (phase 1 §7, step 3),
which is past the 5 ms budget, so it is planned on the builder thread.

## 2. Dead code

How it was found:
- A token count over every `.rs` file in the workspace, splitting production code from tests
  (`tests/`, `tests.rs`, `mod tests`, `examples/`).
- A caller grep for each movement item with no production use.
- `pub(crate)` dead code is already caught by clippy `-D warnings` with `--all-targets`. What is
  left is `pub` items, trait methods, and items only a test reads.

### Deleted in step 1

| item | file:line | callers | action |
|---|---|---|---|
| `Pathfinder::is_building`, field `building` | `ac-client/src/pathfinder.rs:234`, `:188`, `:211`, `:229` | none | delete both; `plan` returns `plan_now`'s answer directly |
| `ac_nav::Ground::cell` | `ac-nav/src/steering.rs:181`; impls `ac-client/src/body/standing.rs:33`, `ac-client/tests/dungeon_edges.rs:98`, the `Fake` at `steering.rs:667` | none: the follower never asks it | delete it from the trait and all three impls; `Standing`'s doc "Six questions" becomes "Five" |
| `Going::is_to` | `ac-client/src/body/going.rs:130` | only `autoplay/team/follow/tests.rs:221` | delete; the test reads `g.goal.place().map(\|p\| p.at)` |
| `Player::blocks_held` | `ac-client/src/player.rs:919` | none | delete |
| `navarea::Area::holds` | `ac-scene/src/navarea.rs:224` | none; `routing::is_held` replaced it in phase 1 | delete |
| `navarea::Area::line_clear` | `ac-scene/src/navarea.rs:253` | only `ac-scene/tests/navarea.rs:83`; its doc ("the test that decides a route is not needed at all") is the old straight-line check | delete; the test calls `ac_scene::nav::line_clear(&area.collision, ..)` |
| `nav::Ground::fits_here` | `ac-scene/src/nav.rs:151` | only the `wayfind.rs` tests (`:1000`, `:1182`) | delete the wrapper; make `fits` (`:156`) `pub(crate)` and have the tests call it |
| `ac_world::trip::ARRIVED` | `ac-world/src/trip.rs:42` | none; phase 4 ends a journey within its goal's `within` | delete |

### Kept: only tests read these, on purpose

These stay. Each is a test surface or names an ACE fact.

- `Area::is_sea`, `terrain_at`, `build_everything` and `NavGraph::edge_count`: the API of the
  ac-scene integration tests (`tests/navarea.rs`, `nav.rs`, `navcost.rs`) and `graph_agrees.rs`.
- `worldroute::find_with`: its unit tests' entry.
- `routing::plan`, the planner that builds chunks as it searches. Its callers are
  `walk_trial::plan_first` (the harness's "planner first" column) and the `berth.rs` and
  `hillside.rs` tests. It is the same search on the same graph (`wayfind::search_building`), not
  a second planner.
- `Steering::remaining` and `no_way`: the follower's state, read by the travel and road tests
  and by `walk_trial`.
- `dodge::MELEE_REACH`: ACE `MeleeDistance`, a fact read by one test.
- `trip::Prefs::without_recalls`: a builder used by a test; not movement.

### Seen, not phase 5's

These have no callers but are not movement code. They are listed here and left for their own
subsystems:
- `actions/*`: `wield_by_name`, `open_book`, `split_to_ground`, `house_clear`,
  `allegiance_info_request`.
- `anim.rs`: `DEAD`, `quat_identity`. `model.rs`: `quat_frame`. `ac-world`: `quat_forward`.
- `ac-world` constants: `CRAFT_*`, `TARGET_*`.
- `creation::CreateOutcome`; `player.rs`: `effective_speed_boost`, `effective_jump_height`.

## 3. Walk patches still standing

A patch is removed when all three hold:
1. its cause was the old straight-line steering, the old leg engine, or flat places;
2. the phase that took the cause away is named;
3. a test covers the case: the harness, or a whole-loop data test where the harness cannot reach.

A removal that changes any harness trial's outcome, or any guarding whole-loop test, is reverted
and the patch stays. The commit message says so.

| # | patch | file:line | what it papered over | cause removed by | verdict | guard |
|---|---|---|---|---|---|---|
| P1 | the wedge rest: after 10 blocked steps the body stops pushing that way for 20 frames (`WEDGED_REST`, `WEDGE_TURN`, `wedged_dir`, `wedged_rest`) | `player.rs:80-94`, `:134-137`, `:1287-1325`, `:1427-1453` | straight-line steering leaning on a wall from "the ten places that set a goal" | phase 2: the follower replans after `NO_PROGRESS` and refuses the same stall twice; phase 3: the slot's one `GIVE_UP_AFTER` | **remove** the rest. Keep `WEDGED_STEPS`, `wedged_for` and `wedged()`: telemetry's `wedged` (`telemetry.rs:144`, `:210`, `tools/telemetry.py:181`) still reports a body pressed against geometry | harness (all three suites); `dungeon_edges`, `jump`, `hillside`; `player.rs` tests |
| P2 | the `walkable` sampler, used by the dodge sidestep's room check (`Player::line_blocked` -> `Player::walkable` -> `nav::Ground::walkable`, samples every `SUBSTEP` 0.5 m) | `nav.rs:169-211`, `player.rs:1104-1111`, `SUBSTEP` `nav.rs:19` | a second meaning of "walkable" next to the body's step (plan, "The graph and the body disagree") | phase 1 §4: one walk test, the body's step (`Ground::can_walk`, `body_reaches`). Phase 2 kept the sampler "for the dodge sidestep only" until phase 5 | **replace**: `line_blocked`'s capsule check becomes the body's own step from the player's cell (`Ground::can_walk((from, cell), (to, 0), cap)`). Delete `walkable`, `SUBSTEP`, and `Player::walkable`; rewrite the nav.rs tests at `:1555`, `:1578` on `can_walk`. `line_blocked` keeps its chest-ray check | dodge tests (`the_sidestep_*`, `no_room_either_side_is_no_step`, the 0x01F6 data tests); `examples/session_cost.rs` compiles; harness unchanged |
| P3 | flat arrivals in callers that decide "there" before asking the slot | `academy.rs:334` (`State::approach`), `academy.rs:1198` (hunt spot); `autoplay/team/follow.rs:86`, `:96` (beside the leader: `level` is true whenever the leader walks); `autoplay/team/quartermaster.rs:378` (`walk_into_reach`); `recovery.rs:411` (`spot.distance(v.xy) <= NEAR`) | flat goals and flat arrival (plan, "Points are flat") | phase 3: the slot answers `Done` in 3D; phase 4: `Recovery.death: Place` | **remove the flat tests**. Each becomes `server_walk::reached(me, at, stop)` (flat within `stop`, \|dz\| <= `SAME_FLOOR`) or is left to the slot's `Did::Done`. In recovery, `death_xy` and `death_block` go: `Action::Travel` carries the `Place`, and `way_back`'s `place_at(spot.extend(me.z))` fallback goes too, since `death` is set with `death_xy` in `note_death` (`:283-285`) and is never `None` there; the recovery tests that set `death_xy` by hand (`:1032`, `:1250`) set `death`. `follow_break` (how far behind is urgent) stays flat: it is a following rule, not an arrival | `tests/academy_route.rs`; follow, quartermaster and recovery tests (`a_death_upstairs_is_walked_back_to_upstairs`, `a_corpse_on_another_floor_is_walked_to`); new `a_leader_on_the_floor_above_is_walked_up_to_not_stood_under` (follow, data); harness unchanged |
| P4 | recovery's replans: `MAX_REPLANS` (4) journeys back "that end short", throttled by `PLAN_EVERY` (3 s) | `recovery.rs:44`, `:63`, `:188-189`, `:416-431` | the leg engine ending a journey short of its goal (plan, "A near walk silently ends a journey") | phase 4: a journey ends arrived within its `within`, refused (`Did::Blocked` -> `no_way()` -> abandoned), or put down for its row; the slot's `GIVE_UP_AFTER` is the one give-up | **remove** both, and `replans`/`last_plan`. The player's own `corpse_minutes` limit (`View.limit`, `:395`) stays: that is the player's rule, not a walk clock | `replans_a_journey_that_ends_short_then_gives_up` is rewritten as `a_journey_back_put_down_is_set_off_again`; `no_way_there_finishes`, `gives_up_when_the_way_back_takes_too_long`, `the_whole_way_back` |
| P5 | explore's aim guesses: `aim_on_floor` tries the aim, then ±45°, then 0.6/0.4/0.25 of the way back to the sill (a ring of guesses); `STAIRS` (3.5 m) keeps the guess near the sill's height; `RoomWalk.pushed` re-aims once at 2x `PAST_THE_DOOR` | `explore.rs:40`, `:162-189`, `:197-212`, `:62-68`, `:287-308` | "a walk at an aim with nothing to stand on jitters against the wall for ten seconds", and "the aim was not far enough in" | phase 2: an aim off any floor is refused `OffFloor` at once, not leaned on; phase 1 §3: ends are resolved by cell, so a goal named in the room's own cell is on the room's floor | **replace** with one rule: the walk's goal is the standable spot in `room`'s own cell nearest the aim past the sill. That is `Client::standable_near(aim, PAST_THE_DOOR)` filtered to `cell == room`, nearest the aim, with `Place.cell = room`. A room with none is shut, as a refused walk is today. Delete `aim_on_floor`, `on_a_floor`, `STAIRS` and `RoomWalk.pushed` with its second aim. `past_the_door` (which side is through) and `PAST_THE_DOOR` stay: the server describes only the room the body stands in. The unit tests `an_aim_in_the_wall_is_turned_to_the_corridor_or_drawn_in` and `:546-625` that test `aim_on_floor` are deleted | `exploring_the_holtburg_dungeon_does_not_stand_at_the_bookcases`, `exploring_does_not_pace_between_two_rooms`, `exploring_climbs_the_stairs`, `a_dungeon_ground_is_arrived_at_underground_and_walked_by_the_explorer`; harness unchanged. **If any of these changes outcome, P5 is reverted and kept** |
| P6 | follow's journey-staleness literal `30.0`, written twice | `follow.rs:121`, `:157` | not a patch; an unnamed threshold | - | **name it** (`FOLLOW_TRIP_MOVED`, metres, "a leader this far from the journey's goal has moved on"). Same value, no behaviour change | follow tests |

### Considered and kept

| item | file:line | why it stays |
|---|---|---|
| the dodge sidestep walked straight at its spot | `body/player_tick.rs:69-80` | the plan keeps it apart: time-critical, with its room checked when chosen (P2 makes that check the body's step) |
| the server's own MoveTo, `SERVER_WALK_FOR` (12 s) | `body/server_walk.rs:7` | the plan keeps it apart; ACE drives it, and the follower walks it on a plan |
| noclip flight, straight | `player_tick.rs:122-136`, `going.rs:261` | the plan keeps it apart |
| the corpse, counter and hand-over margins: `CORPSE_REACH / 2`, `COUNTER_REACH / 2`, `GIVE_REACH * 0.8`, `REACH * 0.6` | `loot/walk.rs:101`, `academy.rs:1150`, `recovery.rs:718`, `:791`, `counter.rs:215`, `salvage.rs:512`, `quartermaster.rs:382` | each sets the walk's end inside the reach the use or hand-over is sent from, so the slot's arrival and the caller's own reach test agree. The comment's reason, "the last metre of a walk wanders" (`loot/walk.rs:93`), is the old steering, and is rewritten to say this (step 3) |
| `HOLD_ON_WITHIN` | `loot/walk.rs:37` | covers drift from a dodge, a knock-back or a fight, not steering |
| `WALK_ON_EVERY` (5 s) and `on_the_way`'s `lately`, for a broken-off errand | `growth/road.rs:29`, `growth/hunt.rs:215`, `counter.rs:146` | a planning-cost throttle: a trip plan searches every portal (`growth/hunt.rs:32`), and a journey broken off for a fight is set off again. No harness trial reaches it; the coordinator's scenarios would be the measure. Phase 4 listed it as left over, and it is kept |
| `next_follow_plan` (3 s, or 10 s after a failed plan) | `follow.rs:158-175` | the same throttle, for a journey after a leader past the region |
| `place_at`'s block-only places | `going.rs:509-529` | a point that carries only its block is located by the planner by the cells' BSPs (`routing::graph_end`, `wayfind::locate`): one rule, not a guess. Its callers (a map point, a script's point, a hunting area's way in, a gem's step) have no cell |
| `routing::is_held` holds an outdoor walk by position | `routing.rs:122` | a building's cells lie in the block under it. Phase 4 measured no case against it |
| assembling the next region before a corridor reaches it | phase 4 §7 risks | needs live evidence (stands at a stretch's end); the coordinator's scenarios |
| `SAME_FLOOR` (2.0 m, client) beside `END_FLOOR` (1.5 m, planner) | `body/movement.rs:19`, `wayfind.rs:17` | two questions: two places at a stop's flat reach, with a slope across it, versus a point over or under a floor. Merging them would move arrivals, and no trial asks for it |
| explore's `RESTART_EVERY` (60 s) | `explore.rs:27` | explore policy (do not go round shut rooms on the spot), not steering |
| doorway constants (`DOORWAY_DEPTH`, `PULL_*`) | `doorway.rs:20-38` | measured spell flight through doorways, not walking |
| `travel.rs` gem room (`SUMMON_*`, `FACE_STEP`, `clear_heading`) | `travel.rs:41-56`, `gem_room.rs` | ACE's room for a summoned portal, not the walk |
| visit's `ARRIVAL_WAIT`, `PORTAL_TAKES`, `CARRIED_OFF` | `visit.rs:58-70` | server facts (objects sent on arrival, a portal's delay, a teleport) |
| `routing::plan`'s building search, `Pathfinder::ask`/`take` | §1, §2 | one planner; the async half is the frame budget |

## 4. Maps and docs

### Root `CLAUDE.md` (121 lines, gate `ROOT_LINES` 121; edit rows in place)

- ac-scene row: "nav graph" becomes "walking graphs of a region and their search (`navarea`,
  `wayfind`, `stride`)".
- ac-nav row: "getting somewhere: one planner's whole walks (`routing::plan`), the follower of a
  planned route, objects in the way, dungeon doorways; answers `Aim::NoWay` instead of walking at
  a wall". Entry: `Steering`, `routing::plan`, `obstacles::detour`. "walk or portals" goes: trips
  are ac-world's.
- Data flow, line 69: `collision, nav graph -> ac-nav` becomes `collision, region graphs (built
  off the frame) -> ac-nav`.

### `crates/ac-client/CLAUDE.md` (200 lines, gate `CLIENT_LINES` 200; net zero)

- Tick path, item 4: `tick_player()` walks the journey's stretch (`walk_journey()`), steers the
  slot's walk along its planned route (`body/standing.rs`, `ac_nav::Steering`), moves the body
  (`player.rs`) and runs `tick_visit()`.
- Systems: a new **walk** row.
  - entry fns: `go`, `go_to`, `stop_going`, `settle_going`
  - files: `body/going.rs`, `body/player_tick.rs`, `body/standing.rs`, `pathfinder.rs`,
    `crates/ac-nav/src/steering.rs`, `crates/ac-nav/src/routing.rs`
  - state: `Client.going`, `Client.steering`, `Client.refusals`
  - tests `going`; log target `body::going`; term `walk`
  - The travel row drops `body/going.rs`, `pathfinder.rs` and `steering.rs`, which this row now
    holds.
  - `tests/code_map.rs` checks that each entry fn is written in the row's own files.
- Glossary: a `walk` row. Means: "the one movement the slot follows (`Going`), planned whole by
  the region planner, owned by the row that asked it". Not: "leg, path, head_for".
- Paying for the two new lines: the `body/mod.rs` row in Other modules loses what the walk row now
  says, and the `profile.rs, weapons.rs` and `daytime.rs, augmentations.rs` rows merge into one
  shim row. The gate (`the_maps_stay_short`) is the judge.

### Module docs and comments (comment-only commits, `same-code` on each)

| file | edit |
|---|---|
| `ac-scene/src/navarea.rs:1-15` | a history story ("so far only ever given it one landblock ... run at a city wall"). Becomes at most 5 lines: an `Area` is the region one planner plans on; a dungeon whole, or the 3x3 blocks round a walk with their buildings; built lazily by chunk, shared by every session through `ac-client` `pathfinder.rs` |
| `ac-nav/src/lib.rs:1-4` | wrap to the line width, and add `obstacles::detour` and `routing::plan` as entries |
| `ac-client/src/body/movement.rs:7-19` (`SAME_FLOOR`) | the Asenala story becomes: "A goal's floor reaches this far over or under it (metres): a sill, slope or step is within it, a storey (3 m) is not." |
| `ac-client/src/body/player_tick.rs:156-166` | the clutter comment, 10 lines, becomes 2: server-placed objects are not in the static geometry, so the follower's aim is walked round the first one on its leg, inside the corridor (`obstacles::detour`) |
| `ac-client/src/tally.rs:17`, `:23` | `source` is the walk's owner label (or "server walk", "dodge"); `route` is `None` with no route (standing for a plan, or refused), not "on the straight line" |
| `ac-client/src/visit.rs:47-49` | `STOP`'s doc says "(flat)"; the walk's `within` is 3D |
| `ac-client/src/autoplay/loot/walk.rs:92-93` | "the last metre of a walk wanders" becomes: inside the reach the use is sent from |
| `ac-client/src/explore.rs:72-88` (`past_the_door`) | the incident story is cut to the rule: through the wall's normal, on the far side from where the body stands, else the way it faces |
| `ac-world/src/trip.rs:34-35` (`WALK_REACH`) | "a single leg" becomes "one walk" |
| the files P1-P5 change | the comment rules on every comment they touch: at most two lines per item, no history; never cut a protocol fact, an ACE `File.cs:line`, a measured threshold or an invariant |

### `docs/movement-plan.md`

- Status line: phases 0-4 merged (main `da6468f`); phase 5 built on branch `phase5`.
- A "Phase 5" measures block after "Phase 4": the three SUMMARY lines, WAITED, INVARIANT, and per
  commit of step 2 whether any trial's outcome or time moved. List the patches removed and the
  patches kept, with reasons (this file's §3 by number).
- Phases list, item 5: rewrite to what was built. Mark the plan's own five items as gone in
  phases 1-4 (§1).

`docs/bugs.md` is left to the coordinator, whose scenario runs close entries.

## 5. Steps

Every commit:
- `cargo fmt --all -- --check`;
- `cargo clippy --workspace --all-targets -- -D warnings`;
- targeted tests: `cargo test -p <crate> --lib <filter> -- --include-ignored`, or `--test <name>`
  for integration tests.

Each step, before its last commit:
- `cargo test-data`;
- `RUSTDOCFLAGS=-Dwarnings cargo doc --workspace --no-deps`;
- the full harness.

Check each exit code itself. Stage files by path, check `git status` for stray `D` lines, and
end every message with the session's attribution lines. Environment: `CARGO_TARGET_DIR=
/Users/danielwilliams/code/acswarm/target/wt-phase5 CARGO_INCREMENTAL=0
AC_DATA_DIR=$HOME/Downloads/ac_data`, with `df -g /` at 15 GB or more before a build.

### Step 1 (builder A): deletions

The harness is run once, at the end of the step. It must be identical trial for trial, times
included: nothing that runs changes.

1. Code, ac-client: `Pathfinder::is_building` and `building`; `Player::blocks_held`;
   `Going::is_to` (the follow test reads `goal.place()`). Targeted: `-p ac-client --lib
   pathfinder`, `follow`.
2. Code, ac-nav and its implementers: `Ground::cell` in the trait, `Standing`, the `dungeon_edges`
   test `Standing` and the `steering.rs` `Fake`. Targeted: `-p ac-nav --lib steering`, `-p
   ac-client --test dungeon_edges -- --include-ignored`.
3. Code, ac-scene: `Area::holds`, `Area::line_clear` (the `tests/navarea.rs:83` assert moves to
   `nav::line_clear`), and `Ground::fits_here` (`fits` becomes `pub(crate)`). Targeted: `-p
   ac-scene --lib wayfind`, `--test navarea -- --include-ignored`.
4. Code, ac-world: `trip::ARRIVED`. Targeted: `-p ac-world --lib trip`.

Before each commit, `cargo run -p xtask -- facts da6468f` (or the step's base). It lists the
comment lines removed that hold facts; none of them may be a protocol fact, an ACE line, or a
measured threshold still true of live code. The commit message names the deleted items.

### Step 2 (builder B): patch removals, one commit each, harness on each

Order: least to most reach, so a revert leaves the rest standing.

1. **P6**: name follow's `30.0`. No behaviour change, so the harness is not needed. Targeted:
   `-p ac-client --lib follow`.
2. **P1**: the wedge rest. Targeted: `-p ac-client --lib player`, `--test dungeon_edges`,
   `--test jump`, `--test hillside` (`--include-ignored`). Then the harness.
3. **P2**: the sidestep's room check on the body's step; `walkable` and `SUBSTEP` deleted.
   Targeted: `-p ac-client --lib dodge -- --include-ignored`, `-p ac-scene --lib nav`. Then the
   harness.
4. **P3**: flat arrivals to 3D, one commit per subsystem in this order: academy, follow (with
   the new data test), quartermaster, recovery (`death_xy`/`death_block` to `death: Place`).
   Targeted: `academy`, `--test academy_route`, `follow`, `quartermaster`, `recovery::`. Then the
   harness after the last of the four.
5. **P4**: recovery's replans. Targeted: `recovery::`. Then the harness.
6. **P5**: explore's aim. Targeted: `-p ac-client --lib explore -- --include-ignored`, `hunt`
   (the explorer test), `visit` (`grows_visit_across_the_facility_hub_is_not_taken_exploring`),
   `follow` (`a_follower_in_a_dungeon_keeps_to_its_leader_rather_than_exploring`). Then the
   harness. If any of these tests or any trial changes outcome, revert, and record P5 as kept
   with the failing case named.

Each commit message has:
- the patch's number and name;
- why its cause is gone (the phase);
- the three SUMMARY lines, and "trial for trial as before" or the times that moved;
- renames, if any, as old -> new.

At the step's end: `cargo test-data`, rustdoc, and the harness against the step's base lines.

### Step 3 (builder C): comments, maps, docs

1. Comment-only: the module docs and comments of §4 that are not in files P1-P5 rewrote, then
   the comment-rule pass on those files. Run `cargo run -p xtask -- same-code <base>` on each
   commit, and `facts <base>` to show nothing factual was cut.
2. Maps: the root `CLAUDE.md` rows and data flow; `crates/ac-client/CLAUDE.md`'s tick path,
   walk row, glossary and line budget. Gate: `cargo test -p ac-client --test code_map`.
3. Docs: `docs/movement-plan.md` (status, Phase 5 block, phases list). This file's status block:
   built, with where the build departed from it, and which patches were kept.

At the step's end: the full gates, `cargo test-data`, rustdoc, and the harness, once, to show the
tip is step 2's tip trial for trial.

## 6. Risks

| risk | where | what is done |
|---|---|---|
| Without the wedge rest the body keeps pushing for up to `NO_PROGRESS` (1.5 s) before the follower replans, instead of resting after 0.5 s | P1 | the harness times show it; an outcome that changes reverts P1 |
| The body's step is stricter or looser than the sampler for a sidestep, so the dodge picks another side or none | P2 | the dodge tests are the judge. The sampler was the second meaning of "walkable" that phase 1 set out to remove; if a dodge test changes, P2 is reverted and kept |
| A 3D arrival walks to a leader or mate on another floor where the flat test said "beside" | P3 | that is the rule (arrival is 3D); the new follow data test pins it. Followers now climb stairs after a leader, and the scenarios will show it |
| A room whose standable spots lie past `PAST_THE_DOOR` of the aim is shut where a guess reached it | P5 | the explore whole-loop tests; revert and keep if any changes |
| The `ac-client` map's 200-line gate | step 3 | rows are edited in place and two shim rows merge; the gate is the judge |
