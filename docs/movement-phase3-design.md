# Movement phase 3 design: the slot

Status: built, 2026-11-02 (branch `phase3`, steps 0-4), for phase 3 of `docs/movement-plan.md`
("One movement slot with one owner"). Line anchors are on `432d294` (main, phases 0-2 merged).
Where the build departed from this design:
- `go` takes the caller's clock (`go(owner, goal, now)`); callers with none use `go_to` on the
  frame's (`Client.frame_at`). `step_owner` has no `debug_assert`: scripts and tests ask outside a step.
- A refusal is kept in `Going` (where the body stood, why); a give-up stands the body
  (`going_to` is `None`); the same owner asking within 2 m follows a moving goal without a plan.
- The loot walk picks the next corpse in the same tick; the approach returns a `Did`, and a refused
  vantage goes in `State.vantage_refused`; the academy hears a refusal by `State::walk_refused()`,
  not a `View.walk_refused` flag; the counter keeps `Run.walking_up` as a bool.
- Following runs as the rows "catch up" and "follow" (`FOLLOW_ROWS`), and keeps `FOLLOW_WALK`
  as its own walk-or-journey rate. A journey's leg is asked by `walk_leg` while the slot is free
  or the journey's, and a journey set off (`travel_to`, `travel_to_in`) frees the slot.
- `drop_stale_walk` is folded into `going_to`; `has_way_to` stays for the journey engine's dungeon
  portal check (phase 5); ac-nav's unused `how_to_get_there` went with `WALKABLE`.

Phase 3 makes every walk a request from a named owner, answered truthfully. `Client::go(owner,
goal) -> Did` replaces `head_for`, `head_toward`, the `follow` slot, `walk_toward` and the 192 m
split as the way to walk. Callers move over one subsystem per commit and delete their walk clocks
as they move. Journeys keep their engine (phase 4), but their legs are walked by the slot.

Rules this design keeps:
- Every walk is planned whole, 3D and cell-aware, by the one region planner, before a step.
- The follower (`ac_nav::steering`, phase 2) only follows; nothing here aims at a goal.
- No spot patches, no threshold tuned to one trial, no straight fallback.
- Doors are walked straight through; nothing here stops at one.

## 1. Step 0: the Tusker ground (0x5C49), measured

`areas()` in `crates/ac-client/tests/route_harness.rs` gains `5C49 Tusker ground`, start
(17744.8, 13910.7, 0.0) in 0x5C49012A, seed 9 (appended, so the other areas keep theirs). `known()`
gains `5C49 Tusker on a ledge over the room`, from that start to (17754.0, 13929.0, 6.0), with
the data test `a_ledge_opening_only_onto_air_is_refused_and_dropped_from`.

| trials | arrived | refused | gave up | still pushing | AGREE A, B, C, D |
|---|---|---|---|---|---|
| 5C49 random (15) | 14 | 1 (#4, `Unreached`) | 0 | 0 | 14, 0, 0, 1 |
| stalls (12, with the ledge) | 10 | 2 (2CB5; the 5C49 ledge) | 0 | 0 | 10, 0, 0, 2 |
| random (135) | 122 | 13 | 0 | 0 | 122, 0, 0, 13 |

(The last two rows are this run's full harness; the main bar was 11/10/1 and 120/108/12.)

**What the refusals are.** Every "no way" in the live 5C49 run (telemetry of `scn-phase2c`, 39
steer samples) has its goal at z = 6.0 in one of the cells 0x182, 0x183, 0x187, 0x18A, 0x18B,
0x18E, 0x18F, 0x190: the Female Tuskers' spawn ledges. Random pair #4 ends on one of them (0x18A).
- Each ledge is a small cell with a floor at 6.0 whose only portal opens onto 0x188, 0x189, 0x18C
  or 0x18D. Those cells have no floor polygon at all (12 triangles, none with normal.z >= 0.5):
  they are the upper half of the two-storey rooms (0x125, 0x126, 0x12C) below.
- From the floor no walk reaches a ledge (`Unreached` with `within` up to 12 m); from each ledge
  the plan down is a route with one `Drop`.
- The other 130-odd upper cells of 0x5C49 are walk-joined to the floor; only these eight are not.

**Verdict: no graph gap.** The planner refuses rightly: a ledge 6 m up the wall, opening onto
air, is reached by no walk the body can take. Nothing in ac-scene changes. The live stall is
entirely the callers asking again and again for what was refused:
- "getting Female Tusker in sight": `vantage` found no spot, so `autoplay_approach` walked at the
  creature itself (`dodge.rs:946`), up on its ledge;
- "fighting Female Tusker": the swing's server MoveTo toward the ledge (`player_tick.rs:94`),
  followed by the planner, refused, and stood out for `SERVER_WALK_FOR` then `STALL_AFTER`;
- "opening a corpse", "walking to Corpse of …": in this run their samples are all "waiting for
  its plan" (183 waiting samples in 0x5C49 against 39 no way), not refusals. They are the
  server MoveTo to within 1 m (`player_tick.rs:94`) and the loot walk, standing while the builder
  thread plans; a refused one (the 19 of 20 seen in an earlier run) is the same caller pattern.
  `go` plans a new walk on the asking frame (§3), which also removes most of those waits.

Phase 3 is the fix: each of those callers is told "no way" at once and acts on it (§6).

## 2. Types and the call

New file `crates/ac-client/src/body/going.rs` (leaf name `going` is used nowhere else; `walk` is
taken by `autoplay/loot/walk.rs`, and the type name `Slot` by ac-plugin's inventory).

```rust
/// Who asked for a walk: a STEPS row, or something outside the step table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// The row running when it asked (`steps::Step::name`); its walk ends when it stops asking.
    Step(&'static str),
    /// The leg of the journey under way (`Travel`); ends with the journey (phase 4 reworks it).
    Journey,
    /// A visit's last stretch (`visit.rs`).
    Visit,
    /// A script's `walk_to`.
    Script,
    /// A front end or a test: the map, a panel, the route harness.
    Hand,
}

/// Where a walk is to: a place, or something in the world wherever it stands now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Goal {
    At { place: Place, within: f32 },
    Near { guid: u32, within: f32 },
}

/// The walk the slot holds (`Client.going`).
pub struct Going {
    pub owner: Owner,
    pub goal: Goal,
    /// The autoplay tick (`Autoplay.tick`) of the owner's last ask.
    asked: u64,
    /// When the walk began, and when it last made progress along a route (any route: a replan
    /// keeps the clock), for [`GIVE_UP_AFTER`].
    since: Instant,
    progressed: Instant,
    /// What the last ask answered, for the cheap re-ask.
    answer: Did,
}

/// The owner whose walk was taken, and who took it: told once, on its next ask.
taken: Option<(Owner, Owner)>  // a field of Client beside `going`
```

```rust
impl Client {
    /// Walk to `goal` for `owner`: the one way anything asks the body to move.
    pub fn go(&mut self, owner: Owner, goal: Goal) -> Did;
    /// End `owner`'s walk, if the slot holds it (any owner with `None`), saying `why`.
    pub fn stop_going(&mut self, owner: Option<Owner>, why: &str);
    /// The running STEPS row as an owner (`Autoplay.running`, set round each `Step::run`).
    pub(crate) fn step_owner(&self) -> Owner;
    /// End a step's walk its row did not ask for this tick (end of `tick_autoplay`).
    pub(crate) fn settle_going(&mut self);
    /// Where the slot's walk is to this frame: the place, `within`, its cell (`player_tick`).
    pub(crate) fn going_to(&self) -> Option<(Vec3, f32, u32)>;
}
```

- `Client.follow: Option<Follow>` (`lib.rs:356`) becomes `Client.going: Option<Going>`, and
  `Follow` (`body/movement.rs:8-14`) is deleted. `Client.follow` is a pub field written by
  `ac-plugin/src/team.rs:253` and `ac-script/src/bridge.rs:440,1356`: both become
  `stop_going(None, ..)`. It is not saved config and not the Rhai API, so no alias or migration.
  The rename `follow -> going` is listed in its commit, and also frees "follow" for the team
  system the glossary means by it.
- `Autoplay.running: Option<&'static str>` is set by `tick_autoplay` before each `step.run`
  (`autoplay/mod.rs:635,648`) and cleared after; `Autoplay.tick: u64` counts calls of
  `tick_autoplay`. `step_owner()` is `Owner::Step(running)`; outside a step (housekeeping never
  walks) it is `Owner::Hand`, with a `debug_assert!`.
- Owners are named by what runs, never by a constant a caller types: `autoplay_approach` asked
  from the fight is the fight's walk, from the academy (`autoplay_fight_as`) the academy's.
- `Owner::label() -> &'static str` (the row's name, or "journey", "visit", "script", "hand")
  replaces `WalkFrame.source`'s "follow" (`player_tick.rs:84-87`); telemetry's `source` carries
  it unchanged in type. "server walk" and "dodge" stay as they are.

## 3. What `go` answers (`Did`)

| `Did` | when | the caller |
|---|---|---|
| `Acting` | a route is being followed, or the plan is building | claims the tick |
| `Done` | arrived: `server_walk::reached(me, goal, within)` | does what it walked for (a use) |
| `Waiting("taken by X")` | its walk was replaced by owner X since its last ask (once) | claims the tick and asks again |
| `Blocked("no way there: …")` | the planner's no way, the follower's `Unreachable`, or the give-up | acts on it at once (§6) |
| `Blocked("not in view")` | a `Near` goal whose object is gone | as for no way |

- **Arrival is the one rule the body already stops by.** `reached` (`body/server_walk.rs:73`) is
  within `within` on the flat and on the goal's floor (`SAME_FLOOR`): player_tick stops steering
  on the same test, so `Done` and the body standing agree. A goal a storey above at the same x, y
  is never `Done` (test `a_goal_a_storey_up_is_not_arrived_at_from_below`).
- **At once.** A new walk is planned on the asking frame by `Client::plan_walk`
  (`body/movement.rs:33`, `Pathfinder::plan`: the built region, read only, as `has_way_to` does
  today). Its answer is handed to the follower with a new `Steering::adopt`, so it is planned
  once, not once by the caller's check and again by the steering:
  - `Route` -> the course is installed; `Acting`.
  - `NoWay(why)` -> the follower's refusal is set; `Blocked("no way there: {why:?}")`.
  - `Building` -> `Acting`; the follower asks the builder as in phase 2 (`Aim::Wait`), and the
    first answer back is the walk's.
- **Out of the region is a journey, not a walk.** The 192 m test (`travel.rs:300`) goes. A goal
  the region around the body does not hold (`routing::is_held` on `navarea::blocks_for(me, goal)`,
  a dungeon by cells: the planner's own `OutOfRegion`) is planned as a journey
  (`plan_trip_in(goal.xy, cell)`), exactly as `head_toward` plans one today: `Acting` while it
  travels to this goal, `Blocked("no way there from here")` when none is planned. The journey's
  lifetime is the journey engine's until phase 4.
- **A walk near at hand ends a journey under way** (`travel.rs:299-301`, kept): `go` with an
  in-region goal while travelling calls `end_trip`, unless the caller broke it off first with
  `interrupt_travel`/`remember_journey`, as the corpse and fight paths do.
- `go` from an owner that is not `Owner::Visit` ends a visit being made, as `head_for` did
  (`drop_visit`, `travel.rs:273`).

### The cheap re-ask

Same owner and same goal is answered from the slot with no plan:
- **Same goal:** `At` within `REPLAN_DISTANCE` (2.0 m, `steering.rs:18`) of the walk's place and the
  same `within`; `Near` the same guid and `within` (its movement is the follower's `Moved` trigger,
  replanned while the old course is walked).
- The answer is recomputed from state only: `reached` -> `Done`; `steering.unreachable(goal)` ->
  `Blocked`; the give-up (§5) -> `Blocked`; else `Acting`. `asked` is set to this tick.
- A refusal stands for its goal while the body stands where it was refused: re-asked from more
  than `REPLAN_DISTANCE` away, the same goal is planned again (the follower's refusal is keyed by
  goal only, `steering.rs:278`; `adopt` also records where it was refused).
- Test `asking_again_for_the_same_goal_plans_nothing`: `Pathfinder` gains a `plans: u32` counter
  (test-visible); ten asks, one plan.

## 4. Ownership and the end of a walk

- **Another owner replaces the walk.** `go(B, ..)` while A holds the slot ends A's walk, starts
  B's, logs `info!(target: "ac_client::body::going", "walk: {B} takes the walk from {A}")` and sets
  `taken = Some((A, B))`. A's next ask answers `Waiting("taken by {B}")` once and clears it; an ask
  after that is a request like any other. Waiting, not Blocked: what took it passes by itself, and
  no caller may set a body aside over it.
- **A step's walk ends when its step stops asking.** At the end of `tick_autoplay` (every return
  path: the body moves to `run_steps`, and `tick_autoplay` calls it then `settle_going`), a walk
  owned by `Owner::Step(_)` whose `asked != Autoplay.tick` is ended: `steering.reset()`, the body
  stands, `debug!("walk: {row} stopped asking")`. One step acts per tick and autoplay asks every
  tick, so the walk of a step that lost the tick to another, or stopped wanting it, never outlives
  it. With autoplay switched off no step asks, and the same rule ends their walks
  (`drop_walk("autoplay switched off")`, `autoplay/mod.rs:574`, stays for the journey).
- **A step that wants its walk asks every tick it runs.** Today every walking caller already does,
  except the fight's "fighting {name}" branch (`melee.rs:86-121`), which does not walk at all: the
  swing's walk is the server's (§6, melee).
- **Other owners** (`Journey`, `Visit`, `Script`, `Hand`) keep their walk until it is `Done` and
  the owner lets go, it is replaced, `stop_going` ends it, the user takes the controls
  (`player_tick.rs:26-30`), or a teleport leaves its goal out of the region (`OutOfRegion`, which
  replaces `drop_stale_walk`'s `WALKABLE` test, `body/movement.rs:111`).
- `drop_walk` (`body/movement.rs:95`) becomes `stop_going(None, why)` plus the journey's end.

## 5. One give-up policy, in the slot

A walk is `Blocked` when, and only when:
1. the planner says no way (`Planned::NoWay`, at the first ask or any replan);
2. the follower says `Unreachable::Stalled` (the same route from the same stall, or a stall seen
   before: phase 2 §4), which already ends every walk that pushes without getting anywhere;
3. `GIVE_UP_AFTER` has passed with no progress along any route for this walk.

```rust
/// A walk making no progress along its route for this long is given up (20 s, the walk clock
/// the loot walk, counter, fight approach and quartermaster each kept: `take.rs:10`, ...).
pub const GIVE_UP_AFTER: Duration = Duration::from_secs(20);
```

- **Why (3) and not only (2).** The follower's own clock (`NO_PROGRESS`, 1.5 s) restarts with each
  course. What it cannot see is a walk that never has a course long enough to stall: a plan that
  never comes back (`Building` for ever), or replans on `OffCorridor` that each walk a little and
  are pushed back. Progress is the follower's: `Steering::last_progress() -> Option<Instant>`, the
  last time `Course::remaining` fell by `PROGRESS` (0.1 m) on any course for this goal; kept across
  replans, cleared by `reset`. Waiting for a plan is not progress.
- A `Near` goal walking away is progress while the body walks its course (the route left falls),
  so a follower of a running leader is never given up by it.
- No caller keeps a walk clock after it moves (§7). They keep reasons to stop that are not about
  walking: no damage on a target (`STALL_AFTER`, `melee.rs:11`), a counter's own refusals, a
  doorway's `PULL_WAIT`, a visit's person not appearing (`waits_for_them`).

## 6. Each caller, and what it does on `Blocked`

The owner is `step_owner()` for every autoplay caller below; the goal kind is named per row.

| caller | today (432d294) | goal | on `Done` | on `Blocked` |
|---|---|---|---|---|
| loot walk (`walk_to_corpse`, `loot/walk.rs:156`) | `head_for(spot, CORPSE_REACH/2)` + `CorpseWalk::goes_on` + `has_way_to` first (`choose.rs:796`) | `Near{corpse, CORPSE_REACH/2}` | `interact` (`choose.rs:833`) | `set_aside_out_of_reach` at once; next corpse |
| corpse held, drifted (`choose.rs:575-581`) | `walk_to_corpse` while `still_holding_at` | the same | goes on looting | `close_container`, let go (`choose.rs:592-597`, kept) |
| corpse open, server walk (`choose.rs:532-535`, asked again at `:494`) | waits `loot_wait(away)` | (server MoveTo) | - | the server walk refused (below): set aside at once |
| fight vantage (`dodge.rs:945`) | `head_for(vantage, AT_VANTAGE)` | `At{vantage, AT_VANTAGE}` | shoot from it | that spot is refused: `Dodge.vantage_refused` gets it, `vantage` skips it, the next spot next tick |
| fight at the target, no sight (`dodge.rs:946`) | `head_for(at, NO_SIGHT_STOP)` | `Near{target, NO_SIGHT_STOP}` | - | `give_up_target(guid, "no way to it")` |
| fight closing in sight (`dodge.rs:946`, `seen`) | `head_for(at, range*WITHIN)` | `Near{target, range*WITHIN}` | shoot | within full `range`: shoot from here (`autoplay_approach` returns false); else `give_up_target` |
| doorway (`doorway.rs:177,227`) | `head_for(spot, 0.3)` | `At{spot, 0.3}` | waits for the target (kept) | `clear_doorway` returns false: the approach goes on as if no doorway |
| melee approach (new, before `melee.rs:184,220`) | `attack(guid)`; the server walks | `Near{target, attack_range(Melee)*WITHIN}` | `attack(guid)` | `give_up_target(guid, "no way to it")` |
| melee, server walk refused | stands, `STALL_AFTER` | (server MoveTo to the target) | - | `give_up_target` at once |
| counter (`counter.rs:216-235`) | `head_for` + `run.walking_up` vs `WALK_UP_LIMIT` | `Near{vendor, COUNTER_REACH/2}` | `use_object` (`:266`) | `grow_run_next(run, .., Some("could not be got to"))` |
| shopping act (`shopping.rs:200`) | `head_for(at, COUNTER_REACH/2)` | `Near{guid, ..}` | - | `false`: the vendor rules read a refusal |
| explore (`explore.rs:320,329,342,417,433`) | `has_way_to` then `head_for`; `GIVE_UP_ON_A_ROOM` | `At{aim on the floor, STOP}` | the sill push (`:309-326`, kept: a goal choice) | `rooms_shut.insert(room)`, "no way into"; next room this tick |
| follow (`team/follow.rs:116`) | `head_for(leader.world, keep)`; `follow_walk` | `Near{leader guid, keep}` | stands (`:94-103`) | the far branch's journey (`travel_about`) with its `next_follow_plan` wait; none: "no way to {leader}", returns false |
| keep to the area (`hunt.rs:269`) | `head_for(goal.extend(me.z), BACK_IN)` | `At{way_in on its floor, BACK_IN}` | - | "no way back to {area}" (kept), false |
| salvage (`salvage.rs:518`) | `head_for(mate.world, GIVE_REACH*0.8)` | `Near{mate guid, GIVE_REACH*0.8}` | hands over | "cannot get to {who}", false |
| quartermaster (`quartermaster.rs:376,388`) | `step_into_reach` + `reaching_too_long` | `Near{mate guid, REACH*0.6}` | hands out | "cannot get to {who}", next mate (`:312-316`, kept) |
| academy (`academy.rs:904,1188`) | `head_for`; `AcademyState.stuck` after `STUCK_AFTER` | `At{target, stop}` / `At{centre, REACH}` | its next action | `View.walk_refused`: the state acts from here, as `stuck` did (`academy.rs:330-343`) |
| recovery (`recovery.rs:676-677`, `:685`) | `travel_to_in(spot, cell)`; `interact(guid)` | `At{death place, ..}`; `Near{corpse, CORPSE_REACH/2}` before `Open` | `interact` | `recovery.no_way()` (kept) |
| visit (`visit.rs:440`) | `head_toward`; `Visit::stalled` (`GIVE_UP`, 60 s) | `Owner::Visit`, `At{v.goal, STOP}` | `finish_visit` | `let_go_of(v)`, "no way to {label}"; `Waiting("taken")`: dropped as "something else is steering" (`:431-434`) |
| script `walk_to` (`bridge.rs:1351`) | `head_for(target, stop).fine()` | `Owner::Script`, `At{target in its block, stop}` | | the call returns false |
| harness (`walk_trial.rs:120,136`) | `head_for` | `Owner::Hand`, `At{goal, stop}` | | `Refused`/`GaveUp` as now |

**The 5C49 cases, in detail.**
- **Getting a Tusker on its ledge in sight (missile, spell).** `vantage` (`dodge.rs:780`) already
  returns only spots the planner routes to. With none, the walk at the target is `Blocked` on the
  first ask: `give_up_target(guid, "no way to it")`. The target is held off by `Autoplay.given_up`
  (`GIVE_UP_FOR`, `target.rs:184`), so `pick_target` takes the next creature, one on the floor; with
  none, the fight returns false and the character stands where it is, which is where a Tusker
  coming down (by its drop) is met. If a vantage is chosen and its walk is refused (the planner's
  answer changed since `vantage` asked), that spot alone is refused and the next one is tried; a
  target with no spot left is given up as above.
- **A clear shot from here at a target out of range.** Closing in is refused: the shot is thrown
  from here if within the attack's full reach, which the server allows (`WITHIN` only keeps a
  margin); else the target is given up.
- **Melee at a Tusker on its ledge.** The swing is sent only from within reach: before
  `attack(guid)` the fight goes to `Near{target, attack_range(Melee) * WITHIN}`. `Blocked` gives
  the target up at once (a creature out of a walk's reach is skipped like any other); the fight
  picks a reachable one, or stands. A creature that comes down is reachable again once its
  `given_up` wait is over or it hits the character (`hit_lately_by` in `pick_target` already puts a
  creature hitting us first).
- **The server's own MoveTo** stays the server's (ACE drives it during a use or a swing). What
  changes is its refusal: when the follower answers `NoWay` for a `server walk` goal
  (`player_tick.rs:185` sees the source), `move_to` is dropped (the client stops standing aside,
  and its next MoveToState calls the use off server-side) and `Client.server_walk_refused =
  Some(target)` is set. The corpse open (`choose.rs:471`) and the fight (`stalled_on`) read it for
  their own guid and act at once: the corpse is set aside, the target given up. Walking to use
  range first (`CORPSE_REACH`, inside ACE's use radius) makes such a walk rare.
- **A corpse up on a ledge** is `Blocked` on its first ask and set aside; the next corpse on the
  floor is walked to in the same tick's next pick.

### Before every use, the slot walks to use range

Each autoplay use is sent only on `Done` of a walk to the reach the caller already names:
corpse `CORPSE_REACH` (`choose.rs:89`), counter `COUNTER_REACH`, visit `REACH`
(`visit.rs:46`), academy `REACH` (`academy.rs:59`, its `Approach` precedes its `Use`), give
`GIVE_REACH`/`Self::REACH`. The one use with no walk before it today is recovery's `Open`
(`recovery.rs:685`): it gains the walk. Manual uses (a double-click) keep the server's MoveTo.

## 7. Walk clocks deleted, per caller

| caller | deleted | kept (not about walking) |
|---|---|---|
| loot walk | `NO_WAY_FOR`, `WALK_PAUSED` (`loot/walk.rs:24,30`), `CorpseWalk.{best, since, no_way_since, last}`, `goes_on` (`:76-91`), the `no_way` read (`:189`), the `has_way_to` pre-check (`choose.rs:796-802`); `CorpseWalk` keeps `guid`, `started` (the log) | `LOOT_TIMEOUT`, `loot_wait` (the open's answer), `HOLD_ON_WITHIN` |
| fight approach | `came_nearer`, `walking_nearer`, `Autoplay.approach_best` (`melee.rs:39,348`; `autoplay/mod.rs:257`); the `walking_nearer` branch of `stalled_on` (`melee.rs:252-258`) becomes "the slot is `Acting` for this target" | `STALL_AFTER` (no damage), `close_in_on` (nothing landing) |
| doorway | `self.follow = None` at the spot (`doorway.rs:216`) | `PULL_WAIT` |
| counter | `run.walking_up`, `WALK_UP_LIMIT`, `WALK_UP_PROGRESS` (`town_run/mod.rs:155-164`; `counter.rs:220-231`) | `VENDOR_OPEN_TIMEOUT`, the opening's tries |
| explore | `GIVE_UP_ON_A_ROOM`, `Autoplay.room_since` (`explore.rs:27,300-303`), both `has_way_to` calls (`:320,417`) | `RESTART_EVERY`, `rooms_shut` |
| follow | `Autoplay.follow_walk` (`autoplay/mod.rs:497`) and its `planted` test (`follow.rs:117-119,174-180`): the slot's owner says whose walk it is | `next_follow_plan` (a journey's plan rate) |
| keep to the area | none (it timed nothing) | |
| salvage | the `interrupt_travel`/`steering.reset` on a new target (`salvage.rs:510-516`): `go` does it | `HAND_OFF_RANGE` |
| quartermaster | `step_into_reach`, `reaching_too_long`, `Autoplay.reaching` (`quartermaster.rs:376-405`; `autoplay/mod.rs:432`); its use of `REACH_GIVE_UP`, `REACH_PROGRESS` | |
| academy | `STUCK_AFTER`, `PROGRESS`, `AcademyState.progress` (`academy.rs:79,330-343`); `stuck` stays, set from `View.walk_refused` | `ANSWER_WAIT`, `RIDE_WAIT`, ... |
| recovery | none | `PLAN_EVERY`, `CORPSE_WAIT` |
| visit | `GIVE_UP`, `PROGRESS`, `Visit::stalled`, `Visit.{best, since}` (`visit.rs:56-58,174-185`) | `waits_for_them`, `USE_REMEMBERED` |
| take.rs | `REACH_GIVE_UP`, `REACH_PROGRESS` once nothing reads them (`take.rs:6,10`) | |

## 8. Journeys walk through the slot (no phase-4 rewrite)

- The journey engine is unchanged: `travel_goal(now)` (`travel.rs:1218`) still picks the leg and
  keeps `STUCK_AFTER`, `STEP_GIVE_UP` and `REPLANS` (phase 4 deletes them with the legs).
- In `tick_player`, the leg is handed to the slot instead of straight to the steering:
  `go(Owner::Journey, Goal::At { place: Place { at: leg, cell: leg_cell }, within: leg_stop })`,
  only when the slot is empty or already the journey's. A step's walk in the slot wins (as
  `head_toward` ending the trip did): its `go` ended the trip, or the trip was broken off.
- The source order in `player_tick.rs:84-114` becomes: dodge, then the server walk, then
  `going_to()`. The `follow` arm and its block-only cell guess (`:98-112`) go: `At` carries its
  cell, `Near` reads the object's own `Position.cell`.
- `Travel.leg_refused` (`travel.rs:177`, set at `player_tick.rs:185`) is set from the slot: the
  journey's walk answered `Blocked`. The engine's two `|| leg_refused` edits (`:1353`, `:1564`)
  are then the journey's reaction to the slot's one give-up.
- `head_for`, `head_toward`, `walk_toward` and `WALKABLE` as a walking split are deleted once the
  last caller has moved (step 4). `travel_to`, `travel_to_in`, `travel_about` stay as journey
  entry points for goals given as a map point (the map panel, script `travel_to`, roams, resume);
  `fight_target_gone`'s `WALKABLE` (`melee.rs:308`) and `autoplay_approach`'s (`dodge.rs:933`)
  become "out of the region" (`Client::is_in_region(at, cell)`), the planner's own boundary.

## 9. Rhai and plugin surface

- No script call is renamed or added. `walk_to(x, y, z[, stop])` keeps its signature and `bool`
  (true while `Acting` or `Done`), now `go(Owner::Script, ..)`; `walk_stop()` is
  `stop_going(Some(Owner::Script), "walk_stop")`; `follow(false)` drops only following's walk.
  `travel_to(destination)` is a journey, unchanged.
- ac-plugin's team `Request::Stop` (`team.rs:253`) calls `stop_going(None, "stopped")`.
- `crates/ac-script/src/api.rs:316-319`'s doc for `walk_to` gains one line: false when the planner
  has no way there. No validate or plugin-template change: neither calls `walk_to`.

## 10. Names, files and gates

- **New production fns** (each four words or fewer, no articles): `go`, `stop_going`,
  `step_owner`, `settle_going`, `going_to`, `is_in_region`, `label`, `adopt`, `last_progress`.
- **New types:** `Owner`, `Goal`, `Going` (run `grep -rn "enum Owner\|enum Goal\|struct Going"`
  first: none today). `Goal` is the plan's word (`docs/movement-plan.md` "A place has a floor").
- **Renames** (listed in their commits): `Client.follow -> Client.going`; `Follow` deleted;
  `head_for`, `head_toward`, `walk_toward` deleted (step 4).
- **Line ceilings.** `travel.rs` is at its 1,800 (`tests/file_length.rs`): every edit there is net
  negative (head_for and head_toward leave it). `body/going.rs` is new and short. `player_tick.rs`
  loses the `follow` arm.
- **Code maps.** `crates/ac-client/CLAUDE.md`: the `body/mod.rs` row names `body/going.rs`
  (`Client::go`, `Going`); the follow system's state drops `Autoplay.follow_walk` and
  `Client.follow`; the glossary gains `walk | what the slot follows for one owner (`Going`) | follow`.
  The travel row keeps `travel_to`, `travel_about`. Rows are edited in place (both maps are full).
- **Comments.** Each new item at most two lines, the module `//!` five; the deleted clocks' long
  stories go with them, a guard's reason cut to its rule and test name.

## 11. The steps

Each builder step: fmt, clippy and targeted tests green on every commit; before its last commit
the full `cargo test-data`, `RUSTDOCFLAGS=-Dwarnings cargo doc --workspace --no-deps`, and the
harness (`cargo test --release -p ac-client --test route_harness -- --ignored --nocapture
--test-threads 1`), each exit code checked itself; SUMMARY, AGREE, WAITED and INVARIANT quoted in
the step's last commit. The bar is §1's: stalls 12/10/2, random 135/122/13, 0 still pushing,
B = 0. Comment-only edits, pure moves and code in separate commits, staged by path.

### Step 1: the slot (one caller: the harness)

1. **Follower hooks** (`crates/ac-nav/src/steering.rs`): `Steering::adopt(me, from, goal, within,
   planned, now) -> Aim` (the answer branch of `steer` lifted out, so `steer` calls it too) and
   `Steering::last_progress()`. Unit tests: `an_adopted_route_is_followed_without_asking`,
   `an_adopted_no_way_is_refused_without_asking`, `progress_is_kept_across_a_replan`.
2. **`body/going.rs`**: `Owner`, `Goal`, `Going`, `go`, `stop_going`, `step_owner`,
   `settle_going`, `going_to`, `GIVE_UP_AFTER`; `Client.follow -> going`, `Follow` deleted;
   `Autoplay.{running, tick}`, `run_steps` + `settle_going` in `tick_autoplay`; player_tick reads
   `going_to()`; `drop_walk`/`drop_stale_walk` on the slot; `server_walk_refused`.
   `head_for`/`head_toward` become one-line wrappers `go(self.step_owner(), Goal::At{..})` (the
   place's cell located by the region) until their callers move. `Pathfinder.plans` counter.
   Tests (`body/going.rs` `mod tests`, offline and data):
   - `a_walk_with_no_way_is_blocked_on_its_first_ask` (the 5C49 ledge);
   - `asking_again_for_the_same_goal_plans_nothing`;
   - `a_walk_taken_by_another_owner_says_so_once` (log line and `Waiting`, then replaced);
   - `a_steps_walk_ends_the_tick_it_is_not_asked_for` (whole loop: `tick_autoplay` +
     `tick_player`, a step that asks then stops; the body stands within a frame);
   - `a_walk_without_progress_is_given_up_after_twenty_seconds` (a plan that never builds);
   - `a_goal_a_storey_up_is_not_arrived_at_from_below`;
   - `a_goal_out_of_the_region_is_a_journey` (replaces the 192 m test);
   - `a_refused_server_walk_is_let_go_at_once`.
3. **The harness** asks `go(Owner::Hand, ..)` (`walk_trial.rs:120,136`; its module doc names
   `Client::go`). Outcome for outcome the same as §1.

Targeted: `-p ac-nav steering`, `-p ac-client going`, `pathfinder`, `travel::`, `--test
dungeon_edges`, `--test route_harness` (named trials).

### Step 2: the Tusker ground callers (loot, fight, melee)

One subsystem per commit:
1. **Loot walk** (`autoplay/loot/walk.rs`, `choose.rs`): §6 rows, §7's deletions; the server walk
   refusal sets the body aside. Tests: `a_corpse_up_on_a_ledge_is_set_aside_at_once` (5C49, a
   corpse on 0x18E and one on the floor: the floor one is opened, the ledge one never walked at,
   within 2 s of the first ask), and the existing `loot/walk/tests.rs` rewritten off the clocks.
2. **Fight approach** (`dodge.rs` `autoplay_approach`/`vantage`, `doorway.rs`): vantage refusals,
   give-up on no way, shoot from here in full reach; §7's melee.rs deletions. Tests:
   `a_target_on_a_ledge_nothing_walks_to_is_given_up_at_once` (5C49 archer, a Female Tusker on
   0x18E and one on the floor: the floor one is shot, the character never stands more than a
   second "getting it in sight"), `a_refused_vantage_is_passed_for_the_next`; the roof, doorway
   and `a_caster_beside_its_target_with_nowhere_to_walk_casts` tests stay green unloosened.
3. **Melee approach** (`autoplay/fight/melee.rs`): approach before the swing, give-up on `Blocked`
   and on a refused server walk. Test: `a_swing_is_not_sent_at_a_creature_no_walk_reaches` (5C49
   melee: the ledge Tusker given up, the floor one fought; alone with the ledge one, the character
   stands and sends no swing).

### Step 3: the other walkers

One commit each, in this order: counter (with `shopping.rs`'s `Approach`), explore, follow (with
ac-plugin `team.rs` and ac-script `bridge.rs`'s `follow(false)`), keep to the area, salvage,
quartermaster, academy, recovery (and its walk before `Open`), visit (`Owner::Visit`). Each commit
deletes §7's clocks for that caller and adds or rewrites its tests:
- counter: `a_counter_no_walk_reaches_is_passed_at_once` (the F682 deck jeweler is reachable;
  a vendor behind a sealed counter is refused at the first ask, `grow_run_next` the same tick);
- explore: `exploring_the_holtburg_dungeon_does_not_stand_at_the_bookcases` (kept) and
  `exploring_does_not_pace_between_two_rooms` green without `GIVE_UP_ON_A_ROOM`;
- follow: `a_follower_with_no_walk_to_its_leader_plans_a_journey`; the follow tests' `head_for`
  (`follow/tests.rs:194`) becomes `go`;
- quartermaster and salvage: `a_mate_no_walk_reaches_is_not_walked_at`;
- academy: `tests/academy_route.rs` green; `a_refused_walk_acts_from_where_it_stands`;
- recovery: `no_way_there_finishes` (kept) and `a_corpse_is_walked_to_before_it_is_opened`;
- visit: the visit tests green with `GIVE_UP` gone; `a_visit_taken_by_a_step_is_dropped`.

### Step 4: journeys through the slot; the old entry points go

1. Journey legs through the slot (§8), `leg_refused` from the slot. Tests: the travel tests
   (`walk_the_journey`, `drive`, `a_journey_leg_with_no_way_is_planned_again_at_once`) green.
2. Script `walk_to` and `walk_stop` (§9); `api.rs`'s doc line.
3. Delete `head_for`, `head_toward`, `walk_toward`, `drop_stale_walk`, the walking `WALKABLE`
   split, `has_way_to` where nothing calls it, `REACH_GIVE_UP`/`REACH_PROGRESS`.
4. Comment-only: module docs and the code maps (§10), `walk_trial.rs`'s doc.
5. Docs: `docs/movement-plan.md` gets the "Phase 3" row (both SUMMARY lines, AGREE, WAITED,
   INVARIANT), this file's status line with what departed from it.

Not in this run: `tools/scenarios.sh` (the coordinator runs it before the merge). What it should
show at 0x5C49: no stand over 30 s, the archer and blade with kills, no "no way" asked twice for
one goal by one owner.
