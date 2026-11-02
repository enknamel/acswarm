# Movement phase 4 design: journeys

Status: design, 2026-11-02, for phase 4 of `docs/movement-plan.md` ("Journeys plan every walk
too"). Line anchors are on `0db2f2f` (main, phases 0-3 merged). Nothing here is built yet.

The trip planner keeps choosing portals, recalls and gems. What changes is how a journey walks:
each step carries the `Place` it walks to, the walk through each region is planned whole by the one
region planner before a step is taken, a step with no walk is left out and the journey planned again
at once, and the slot's give-up is the only one. A journey's walk belongs to the STEPS row that set
it off, which stays acting while it runs.

Rules this design keeps:
- Every walk is planned whole, 3D and cell-aware, by `ac_nav::routing::plan`, before a step.
- The follower (`ac_nav::steering`) only follows; nothing here aims at a point.
- No spot patches: no special case for one place, no threshold tuned to one trial, no straight
  line, no ring of guesses round a target. Every constant below names its source.
- Doors are walked straight through.
- Arrival and reach are 3D (`server_walk::reached`, `SAME_FLOOR`).

## 1. The journey engine today

`crates/ac-client/src/travel.rs` (2,414 lines; 1,634 production, ceiling 1,700):

| part | where | what it does |
|---|---|---|
| set off | `plan_trip_in` `:308-504` | plans the trip on xy and a cell; checks only the first portal by `worldroute` (`travel_can_reach` `:511`), three tries |
| step start | `travel_start_step` `:520-742` | aims a walk: `[me, target]` indoors (`:674`), else a `worldroute` route over terrain with a ring of 24 guesses round the target (`:696-704`); guesses `step_z` from the portal table (`:617-620`) or `floor_in_cell` (`:621-629`) and the block by `steering_block` (`:643-646`) |
| per frame | `travel_goal` `:1143-1538` | teleport checks; `STEP_GIVE_UP` 30 s (`:1265-1310`); the indoor leg (`:1311-1347`); the waypoint loop: skip a waypoint after `STUCK_AFTER` 10 s (`:1487-1516`), cut a `LEG` of 60 m at the block edge (`leg_end` `:1603`), height by `leg_height` (`:1581`) |
| into the slot | `walk_leg` (`body/going.rs:333-351`) | each leg is `go(Owner::Journey, At{leg, within})`; `leg_refused` feeds the clocks above |

What is wrong with it, against the plan:
- **Points are guessed.** A leg ends 60 m on or at a block edge, at the grid's height or the
  character's own floor, in the block under it. The trip's `Step::Walk(Vec2)` has no cell or
  height (`ac-world/src/trip.rs:75`); the goal's cell rides beside it in `Travel.goal_cell`, and
  is lost whenever a caller plans by xy (`plan_trip` `:294-302` keeps it only for the same goal).
- **Nothing is planned whole.** Only the first portal is checked before setting off, by terrain.
  A leg the slot refuses is waited on by three clocks: `STUCK_AFTER` skips a waypoint,
  `STEP_GIVE_UP` replans, `REPLANS` gives up (`:72-84`). docs/bugs.md: "No aim at a block edge on
  the coast ... about 40 s (10 s plus three replans)"; "No walk out of the ACB5 mine" (77 s on a
  step "walk inside" to a point 63 m outdoors); "Stepping out of a counter's building at Nanto"
  (124 s aimed straight at a point 120 m outdoors).
- **The walk out of a building is a second journey.** `grow_travel_in`
  (`autoplay/growth/road.rs:169-196`) first travels to a spot it last stood outdoors (kept every
  `CLEAR_OF_DOOR` 5 m, `growth/mod.rs:436-449`), then `grow_travel_on` (`:200-212`) the real one.
- **Nobody owns a journey.** Its legs walk as `Owner::Journey` whenever the slot is free
  (`going.rs:340`), whatever row is acting. A row that set it off and then stood aside leaves the
  status "waiting" while the body walks on (15-20% of samples in the phase 3 runs).

## 2. The Qalaba'r case, traced

Blargerton (account rawrtherton, `~/.cache/acswarm/logs/acswarm-1793558125-56899.log`,
2026-11-01 18:53:31-18:53:57; the same at 16:58, 17:50 and 18:36 in that day's logs):

```
travel: planning to Vec2(26604.34, 337.8) from 0xa9b40019, underground false
travel: no shorter way; the long walk, 1820 s
travel: 2 portals, about 30 min (3 steps)
travel: step 1 (portal "Qalaba'r Portal") inside
travel: through the portal
travel: step 2 (walk): no route to Vec2(26604.34, 337.8), giving up
visit: the journey to Warden ended short of it
... (the next Warden, by Land Bridge Villas, the same) ...
visit: no way to Warden from here
travel: /lifestone, then 4 portals, about 6 min        (growth's next ground)
```

Through the code:
1. `grow_hunt` (`growth/hunt.rs:443-450`) picks the unlock `fachubeasthamportals_flag`
   (`ac-world/data/unlocks.csv`: Warden, cell 0x8A0201F7, local (108.34, -46.2, 0)) and calls
   `visit_unlock` (`quest_flags.rs:106`) -> `visit_landmark` with that cell and position.
2. `tick_visit` (`visit.rs:378-389`): the region does not hold the goal, so `Next::Journey`:
   `self.plan_trip(v.goal.truncate())` (`:382`). The cell is dropped here.
3. `plan_trip` (`travel.rs:294-302`): no journey to this goal yet, so `cell = 0`; `plan_trip_in`
   plans to the bare xy.
4. The xy (26604.34, 337.8) has local y = -46.2: it lies in block 0x8A01, not 0x8A02 (the Facility
   Hub's rooms run south of its square). With cell 0 the trip planner (`trip.rs:196-213`) takes it
   for open ground in 0x8A01, finds no believable walk, and `Prefs::far` (`travel.rs:418-439`)
   plans Town Network -> Qalaba'r -> a 6 km walk off the coast.
5. At Qalaba'r `travel_start_step` (`:696-741`) finds no terrain route, nor round the ring, and
   ends the trip; the visit sees `Next::GiveUp` (`visit.rs:390`). Growth goes elsewhere.

Checked offline with the trip planner on this commit (a scratch test, not kept): from the Holtburg
lifestone to that position, cell 0 gives "2 portals, about 30 min" (Town Network, Qalaba'r, walk);
cell 0x8A0201F7 gives "2 portals, about 1 min": Portal to Town Network, then Portal to Facility
Hub (0x00070126 -> 0x8A020212), then the walk.

**The fix.** Every journey entry takes a `Place` (§3), so the visit's cell reaches the trip
planner, which plans Town Network -> Facility Hub -> `Walk(Place { warden, 0x8A0201F7 })`. Each
region's walk is planned on arrival there (§4): the Holtburg walk to the Town Network portal, the
hub's walk to the Facility Hub portal, the Facility Hub's walk to the Warden's cell. The Facility
Hub is a dungeon region (`is_in_region` by cells), so the visit's own last stretch
(`Owner::Visit`) takes over and arrives in the Warden's cell. Tests:
`a_warden_in_the_facility_hub_is_planned_through_its_portal` (ac-world, step 1),
`a_visit_to_a_warden_is_planned_through_the_facility_hub` (step 1), the harness trial "A9B4
Holtburg to a Facility Hub Warden" (step 2).

## 3. A step carries its place

### Types (ac-world, `src/trip.rs`)

```rust
pub enum Step {
    /// Walk to this place: the journey's end.
    Walk(Place),
    /// Walk into this portal's mouth and come out at its exit.
    Portal { name: String, mouth: Place, exit: Place },
    /// Stand still, cast this spell, and come out at its destination.
    Recall { spell: u32, name: String, exit: Place },
    /// Use a carried gem and come out at its exit (most summon a portal to walk into).
    Gem { guid: u32, name: String, exit: Place, summons: bool },
}
impl Step {
    pub fn end(&self) -> Place;                // where the step leaves the character
    pub fn exit(&self) -> Option<Place>;       // a portal's, recall's or gem's exit
    pub fn walk_end(&self) -> Option<Place>;   // where it walks to: a Walk's place, a mouth
}
pub struct Recall { pub spell: u32, pub name: String, pub exit: Place }
pub struct Gem { pub guid: u32, pub name: String, pub exit: Place, pub summons: bool }
pub fn plan_with_recalls_and_gems(from: Place, goal: Place, level, quests, avoid, recalls,
    gems, prefs) -> Option<Trip>;              // and plan, plan_for, plan_with, plan_with_recalls
```

- The search is unchanged: it reads `place.at.truncate()` and `place.cell` where it read xy and a
  cell. A goal "known by position only" is `Place::on_map` (below), never cell 0.
- `Place::on_map(xy: Vec2) -> Place` (ac-world `lib.rs`, beside `Place::of_position`): the outdoor
  cell under `xy` (`outdoor_cell`), height 0. Its height is never read before its region puts it
  on its floor (§4).
- The portal step keeps walking to its own mouth; there is no separate `Walk(mouth)`. Trip counts,
  `summary`, the map and the quest-flag tests read steps unchanged.

### Where each place comes from

| place | source | cell |
|---|---|---|
| portal mouth, exit | `portals::Portal.from`, `.to` (data `from_z`, `to_z`) | `from_cell`, `to_cell` |
| recall exit | `recalls::fixed` (`recalls.csv` x, y, z) or the saved position (`Position`) | its cell |
| gem exit | `gems::Gem` (`gems.csv`) | its cell |
| the goal | the caller (table below) | the caller's |

| who sets off | goal today | goal from phase 4 | within |
|---|---|---|---|
| visit (`visit.rs:382`) | `v.goal.truncate()` | `Place { v.goal, v.cell }` | `visit::REACH` |
| `go` past the region (`going.rs:201-232`) | xy, the cell only indoors | the goal's place | the goal's |
| script `travel_to`, map, examples | `travel_to(xy)` | `Place::on_map(xy)` | `travel::ARRIVE` |
| growth ground (`hunt.rs:223,394,464`) | `at`, `ground_cell(lb)` | `Ground::place()` (new, ac-world): a dungeon's entrance portal's exit (`portals` by `to_cell`), else `Place::on_map(at)` | `GROUND_REACH` |
| town run counter (`counter.rs:170,647,744`; `town_run/mod.rs:589`) | `run.at` xy | the vendor's `Vec3` and cell (`shops`) | `VENDOR_REACH` |
| follow (`follow.rs:159`) | leader's xy | the leader's place (`leader.world`, `leader.cell`) | `keep` |
| recovery (`recovery.rs:699-706`) | `way_back` (cell only underground, else ground under) | `Recovery.death: Place`, the body's place at death in any cell | `NEAR` |
| resume (`journey.rs:13,35-37`) | `resume_trip: Vec2` | `resume_trip: Option<Goal>`, the journey's own goal | the original's |

- `Travel.goal: Option<Vec2>` + `goal_cell` (`travel.rs:161-163`) become `Travel.goal: Option<Goal>`
  (always `Goal::At`); `Client::journey_goal() -> Option<Goal>`. `travel_goal_xy()` stays, derived.
- `recovery::View.trip`, `Action::Finish { trip }`, `Recovery.trip` (`recovery.rs:130,157,176`)
  become `Option<Goal>`; `Recovery.death_cell` (`:170`, set only underground, `:286`) becomes
  `death: Option<Place>`, set from the body's place at death; `death_xy` stays for the state
  machine's flat tests. `way_back` (`:786-810`) is `Goal::At { death, NEAR }` when the corpse is not
  in view: its `floor_in_cell` and `ground_height` guesses go.

## 4. A step's walk: stretches, each planned whole

A **stretch** is the part of a step's walk the region around the body holds: from where the body
stands to the step's walk end when the region holds it, else to where the overland corridor leaves
the region. Each stretch is one `go` of the slot, planned whole on the asking frame (phase 3, §3
there) and followed by the follower. Nothing else walks.

### 4.1 The end of a step's walk

- `Step::walk_end()`: a `Walk`'s place, or a `Portal`'s mouth. Recall and gem steps walk nowhere
  (a gem's step to open ground, §4.6, aside).
- The final walk's place is put on its floor once a region holds it, by the planner's own end
  rule (`wayfind::floor_of`, which `graph_end` and the goal set already use):
  `Pathfinder::put_on_floor(from: Place, place: Place, cap) -> Result<Option<Place>, NoWay>`
  (`None` while its area builds; `NoWay::OffFloor` when its cell has no floor there). That is how
  a map point's height is known, and `reached` then measures arrival in 3D against a real floor.
  A mouth keeps the portal's own height (the planner resolves it as it does today, phase 3 §8).
- Within: the final walk, the journey's `within`; a mouth, 0.0 (walked into: the server takes a
  body that touches it). "At the mouth" for the portal clocks is `reached(me, mouth, ARRIVE)`.

### 4.2 The region holds the end: one walk

`is_in_region(end)` (`going.rs:476`: a dungeon by cells, outdoors the blocks round both): the
stretch is the whole walk, `go(owner, Goal::At { end, within })`. Inside a building, the Town
Network hub, a dungeon or a mine, or from inside a shop to a portal across town, this is the walk;
the `[me, target]` leg (`travel.rs:674`), `step_cell`, `step_z` and `floor_in_cell` go.

### 4.3 It does not: the overland corridor

New file `crates/ac-client/src/travel/overland.rs` (leaf name unused in the workspace).

```rust
/// A walk end's overland corridor (`worldroute`), walked a region at a time.
pub(crate) struct Corridor {
    /// The terrain route from where it was planned to the walk end (world xy).
    points: Vec<Vec2>,
    /// Stretch ends the planner refused: the next corridor keeps clear of them.
    refused: Vec<Vec2>,
}
/// A stretch end is planned to within this (metres, 3D): a corridor point is known only to its
/// lattice's spacing (`worldgrid::SPACING`, 24 m).
pub(crate) const EXIT_REACH: f32 = SPACING / 2.0;
```

- **Planned once per step**, when the step's walk end is past the region:
  `worldroute::find_avoiding` from the body to the end, avoiding every outdoor portal mouth but the
  step's own (the avoid list `travel.rs:687-695` builds today, kept). No route: the step has no
  walk (§4.5). No ring of points.
- **Where a stretch ends** (`Corridor::exit_from(me) -> Option<Vec2>`): the corridor resampled every
  `SPACING` from its point nearest the body; the square is the 3x3 blocks round the body's block
  (`navarea::blocks_for`'s far case, `MAX_SIDE`) shrunk by `SPACING`; the exit is the last sample
  before the corridor first leaves that square. `None` when the corridor's nearest point lies
  outside the square (the body was drawn off it): the corridor is planned again from the body.
- **The stretch** is `go(owner, At { exit, EXIT_REACH })`, the exit `Place::on_map(exit)` put on its
  floor (§4.1). The planner routes through buildings, fences and bridges to a node within 12 m of
  the corridor; the slot's `Done` is the same 12 m.
- **Planned on again**: on `Done`, the next stretch is asked on the same frame from where the body
  stands, along the same corridor; once the region holds the walk end, the last stretch is §4.2.
- **A stretch refused** (`Blocked`): its exit joins `Corridor.refused`, and the corridor is planned
  again from the body with those exits avoided too (`AVOID_RADIUS`, `worldroute.rs:91`). If no
  corridor is found, or its new exit lies within `AVOID_RADIUS` of a refused one, the step has no
  walk (§4.5). Each refusal adds an exit, so this ends without a count.
- `Client::travel_route()` returns the corridor's points (the map and `route_marks.rs:98` draw it
  as the overland route, unchanged in type).

### 4.4 Setting off: the first walk planned before a step

`Client::set_off(goal: Goal) -> Did` replaces `plan_trip`/`plan_trip_in` (`travel.rs:294-504`):
1. Plan the trip from the body's place (`planning_cell` kept, `:1550`) to the goal, with this
   goal's refused mouths, recalls and gems. No trip: why, as today (`:440-485`), and `Blocked`.
2. Start step 0 (`StepState::default()`, §4.7) and, if it walks, ask its first stretch of the slot
   on this frame (`walk_journey`, §4.8).
3. That stretch refused: the step has no walk (§4.5). A final walk with no walk is the journey
   refused at once: `Blocked`, `!traveling()`, the body never moved
   (`a_journey_whose_first_walk_has_no_way_is_refused_at_once`). A portal step with no walk is left
   out and the journey planned again on the next frame: `Acting`.
4. `Building` (the area not assembled yet, live): `Acting`; the follower's answer comes back to the
   slot and is handled as any refusal is.

`travel_can_reach` and its three tries (`:361-414`, `:511-516`) go: the first walk is planned, not
guessed by terrain. `Travel.last_seen` is set to the body's place at set-off: a stale one made the
first frame of the traced journey "carried off ... planning again" (§2's log, 18:53:31.720).

### 4.5 A step with no walk is left out

One rule for every refusal of a step's walk (the planner's no way, the follower's `Stalled`,
`GIVE_UP_AFTER`, a refused corridor, `OffFloor` from `put_on_floor`):
- a `Portal` step: its mouth joins `Travel.refused` (as `:729`, `:1292`, `:1425` do today), and
  `Travel.replan_due` is set: the next frame plans the trip again from where the body stands,
  keeping this goal's refusals. `cancel_travel_keeping_refusals` + `plan_trip`, five copies today
  (`:669-670`, `:730-732`, `:919-923`, `:1082-1086`, `:1427-1431`; the recall and gem refusals
  among them), become `plan_again(why)`. Each round refuses one more mouth, recall or gem, so
  planning again ends without `REPLANS`;
- the final `Walk`: no other way exists from here, so the journey is refused: it ends, and
  `refusals` (phase 3) keeps the goal refused for the owner that asked, so its next `go` answers
  `Blocked` without planning (`going.rs:150-159`).

**Arriving in a new region** (through a portal, a recall or a gem): the next step starts and its
first stretch is planned on that frame, on that region, whose collision is loaded only now. A
refusal there leaves the step out and plans the trip again from there: a portal whose mouth the
arrival region has no walk to is replaced by another way, or the journey refused if none
(`a_journey_that_loses_a_step_is_planned_again_without_it`).

### 4.6 Steps that do not walk

- **Recall**: unchanged (`:1203-1247`): settle, cast, cast again, refuse after `RECALL_TRIES`. The
  journey asks the slot for nothing, so the body stands.
- **Portal at its mouth**: `PORTAL_GIVE_UP` from first being at the mouth, the jump after 3 s,
  carried-off and arrival checks (`:1150-1202`, `:1405-1450`) are kept: they time a portal that
  will not take us, not a walk.
- **Gem**: the use, the summoned portal and their clocks are kept. The step to face open ground
  (`travel_gem_first_use` `:1001-1026`) becomes a slot walk to `Place { spot on the floor
  room_to_summon found, place_at(spot).cell }` within 0.3 m; `Done` or `Blocked` both end it and the
  gem is used, as `FACE_GIVE_UP` did after 4 s. `FACE_GIVE_UP` (`:69`) goes.

### 4.7 The step's state is replaced whole

`travel_start_step` reset fourteen fields by hand (`:546-559`, `:577-595`, `:610-616`) and
`travel_next_step`/`end_trip` others (`:746-758`, `:870-888`). The per-step fields move into one
`StepState` (portal, recall, gem and summoned clocks, `corridor`, `end`) replaced by
`StepState::default()` when a step begins, so nothing from one step reaches the next.

### 4.8 Per frame: `walk_journey`

`Client::walk_journey(now) -> Option<Did>` replaces `walk_leg` (`going.rs:333`) and the walking
half of `travel_goal`, renamed `step_goal(now) -> Option<Goal>` (the teleport, recall, gem and
portal-mouth checks, then the stretch to walk). In `tick_player` (`player_tick.rs:38`) as now:
1. `replan_due`: plan again (§4.5); next frame walks it.
2. No journey, or held (§5): let go of the journey's walk in the slot.
3. `step_goal`; none (a recall, standing for a cast): let go of the walk.
4. Another owner's walk still followed in the slot (`holds_walk_besides`): wait.
5. `go(journey_owner(), goal, now)`: `Done` at a corridor exit asks the next stretch;
   `Done` at the final place ends the journey arrived; `Blocked` is §4.3's or §4.5's rule.

The one give-up is the slot's (`GIVE_UP_AFTER`, `going.rs:20`): a stretch making no progress for
20 s is `Blocked`, which leaves the step out. No clock in travel times a walk.

## 5. A journey's walk is its row's

### 5.1 Who owns it

- `Travel.row: Option<&'static str>`: the STEPS row running when the journey set off
  (`Autoplay.running`), else none (a script, the map, a panel, a visit's journey from `tick_visit`,
  a vendoring-panel run, an example).
- `journey_owner()`: `Owner::Step(row)`, else `Owner::Journey`. Every stretch is asked of the slot
  as that owner, so telemetry's `source` names the row ("grow", "town run") while it walks.
- `Owner::Journey` stays only for journeys no row owns; its doc says so. `Going` gains
  `journey: bool`: this walk is the journey's stretch, settled by §5.2's rule, not by `asked`.

### 5.2 The row keeps it walking

- At the end of `tick_autoplay` (`settle_going`, `autoplay/mod.rs:553`): a journey whose row did not
  act this tick (`Autoplay.step != Some(row)`, read only after `run_steps` set it this tick; a
  row's journey is already gone when autoplay is off, `drop_walk` at `:577`) is **held**: its
  stretch leaves the slot (`stop_going`), the body stands, the trip and its step are kept
  (`Travel.held`). On a tick its row acts it is let go on, and the next frame plans its stretch
  afresh from where the body stands; the trip is not planned again. This is phase 3's "a walk ends
  when its owner stops asking", for a journey (`a_journey_stands_while_its_row_stands_aside`).
- A row carrying a journey another row set off takes it over with `keep_journey()` (the journey's
  row becomes the running one): following's two rows ("catch up", "follow", `FOLLOW_ROWS`) and keep
  to the area. A `go` for the journey's own goal (within `REPLAN_DISTANCE`) from another owner takes
  it over the same way, with no new plan.
- A row says it is travelling only for its own journey, `is_own_journey()`. Audit of every
  `traveling()` read inside a row (step 3 changes each):

| row | today | phase 4 |
|---|---|---|
| keep to the area (`hunt.rs:259`) | any journey or visit: `true` | its own: `true`; another row's: `keep_journey()`, `true`; a visit's: `true` |
| grow, bound (`growth/hunt.rs:183`) | `traveling() \|\| grow_travel_on()`: `true` | own journey: `true` (set off by grow) |
| grow, roam from indoors (`growth/hunt.rs:273`) | any journey: `false` | own journey: `true`, "looking about"; another's: `false` |
| town run (`counter.rs:136`) | `traveling() \|\| grow_travel_on()`: `Waited` | own journey: `Waited` (acting) |
| follow, catch up (`follow.rs:113-178`) | follow journey: `true` | `keep_journey()` while `is_follow_journey()` |
| resume the journey (`journey.rs:21-44`) | `true` only on the tick it resumes | `true` while its own journey runs, "back on the road" |
| recover (`recovery.rs:700`) | `go` each tick | unchanged: `go` for its own goal is acting |
| explore (`explore.rs:233`), buffs (`buffs.rs:250`), town run start (`town_run/mod.rs:366`) | stand aside while travelling | unchanged |

- With no row acting and a journey of no row walking, the idle line (`autoplay/mod.rs:664-666`)
  says "travelling to ..." (`Doing::Traveling`), not "waiting".

### 5.3 Walks, uses and journeys

- **A walk by the journey's own owner** (`go` with a goal in the region) ends its journey: the
  owner wants something else (`going.rs:248-250`, kept for that case).
- **A walk by another owner** leaves a row's journey alone (it is held at settle, since its row did
  not act). A journey of no row is ended by it, as phase 3 does.
- **`interrupt_travel`** (`travel.rs:813`: a swing, a use, a trade, a corpse): ends a journey of no
  row or of the running row (as today, `broken_off` set); another row's journey is left to be held.
  Outside any step (a hand's use) it ends every journey, as today.
- **`remember_journey`** (`journey.rs:8`) keeps only journeys of no row: a row's journey is held,
  and its row picks it up by acting. It keeps the `Goal` (place and within). The resume row sets it
  off again with `set_off(goal)` (`a_journey_put_down_for_a_fight_arrives_at_its_place`).
- **Ends**: `cancel_travel`, `stop_moving_by_itself`, `drop_walk` (autoplay off), a death
  (`recovery.rs:641`), a new journey set off, its own owner's other walk. A held journey is kept
  through any number of ticks; its row's next act walks it on.

### 5.4 The errands' walk clocks and the walk out of a building

- `grow_travel_in`'s walk out (`road.rs:178-194`), `grow_travel_on` (`:200-212`), `after_out`,
  `last_outdoors`, `outdoors_mark`, `outdoors_back` (`growth/mod.rs:217-223`, `:289-291`,
  `:311-314`, `:436-449`), `CLEAR_OF_DOOR` (`road.rs:28-30`) and `on_its_way`'s `after_out` term
  (`road.rs:147`) go: from inside a shop the first stretch is planned from the body's cell through
  the door (§4.2), and from upstairs down the stairs. `grow_travel(goal: Place, within, now)`
  sets off; the dungeon test (`road.rs:174-178`) is the trip planner's `in_dungeon`.
- `WALK_TIMEOUT`, `walk_limit`, `planned_walk_limit` (`road.rs:7-26`), `growth::State.bound_limit`
  (`growth/mod.rs:168-169`, `hunt.rs:174-182`, `:395-399`, `:465-469`), `town_run::Run.walk_limit`
  (`town_run/mod.rs:152-153`, `:604`, `:626`; `counter.rs:121-134`, `:675`; `panel.rs:162`) go:
  they timed a walk. A journey now ends arrived, or refused at the step that has no walk (§4.5),
  and the errand reads that as it reads a journey ended short today (`OnTheWay::Short`).
- `on_the_way`, `journey_broken_off`, `WALK_ON_EVERY` stay: a no-row journey (a vendoring-panel
  run) is still broken off and walked on.

## 6. Deleted, by name

| item | where (`0db2f2f`) | why it goes |
|---|---|---|
| `LEG`, `EDGE_MARGIN` | `travel.rs:33,37` | legs |
| `STEP_GIVE_UP`, `STEP_PROGRESS`, `STUCK_AFTER`, `REPLANS`, `PROGRESS` | `travel.rs:72-84` | walk clocks; §4.5 |
| `FACE_GIVE_UP` | `travel.rs:69` | walk clock; §4.6 |
| `Travel.{step_block, step_cell, step_z, step_since, step_best, step_target}` | `travel.rs:99-115` | guesses and clocks |
| `Travel.{route, next, leg, leg_refused, best, last_progress, measured_on, replans}` | `travel.rs:164-182` | legs, the waypoint loop |
| `Travel.goal_cell` | `travel.rs:163` | in `Travel.goal: Goal` |
| `Travel::restart_waypoint` | `travel.rs:196-203` | legs |
| `travel_can_reach`, the first-portal loop | `travel.rs:361-414`, `:511-516` | §4.4 |
| `plan_trip`, `plan_trip_in`, `travel_to_in` | `travel.rs:278-504` | `set_off`, `journey_to` |
| the walk half of `travel_start_step` | `travel.rs:598-741` | §4.2, §4.3 |
| the end-cell floor guess | `travel.rs:621-629` | §4.1 |
| the ring of points round a target | `travel.rs:696-704` | §4.3 |
| `travel_displaced` | `travel.rs:1135-1137`; caller `session/apply.rs:113` | no leg to forget; the follower replans off its corridor |
| step clocks, indoor leg | `travel.rs:1260-1347` | §4.2, §4.5 |
| the waypoint loop and its skip | `travel.rs:1385-1537` | §4.3 |
| `steering_block`, `leg_height`, `portal_mouth_height`, `leg_end` | `travel.rs:1569-1633` | places carry cell and height |
| `walk_leg` | `body/going.rs:331-351` | `walk_journey` |
| `has_way_to` | `body/movement.rs:54-65` | its one caller (`travel.rs:662`) goes; clippy would flag it |
| `floor_in_cell` | `body/movement.rs:67-73` | its callers (`travel.rs:626`, `recovery.rs:796`) go |
| `Recovery.death_cell`, `way_back`'s ground guess | `recovery.rs:170,286,793-805` | `Recovery.death: Place` |
| walk-out and errand clocks | §5.4 | §5.4 |
| `examples/walksim.rs` | `crates/ac-client/examples/` | it walks legs (`leg_end`, `STUCK_AFTER`); the route harness replaces it |
| tests of the above | `travel.rs` tests: `a_walk_underground_is_steered_in_the_dungeon_it_is_in`, `legs_are_short_and_stay_in_the_block`, `a_walk_that_gets_nowhere_is_given_up` (a goal with no cell no longer exists) | rewritten or replaced, §8 |

Phase 5 is left: `place_at`'s block-only places (`going.rs:452`), `routing::is_held` holding an
indoor goal by position from outdoors, `next_follow_plan`, `on_the_way` for no-row journeys, and
assembling the next region before a corridor reaches it, should live runs show stands there
(today's legs wait the same way at each new 3x3).

## 7. Rhai and plugin surface

- No script call changes. `travel_to(destination)` keeps its signature and `bool`; a `Place`
  destination is `Client::travel_to(xy)`, now `journey_to(Place::on_map(xy), ARRIVE)`.
  `traveling()`, `cancel_travel()` unchanged. No validate or plugin-template change.
- ac-plugin: `travel::ARRIVE` (`map.rs:1607`) stays, now "a map point, or a stretch, is reached
  within this (metres, 3D)"; `travel_route()` (`map.rs:376`) is the corridor; `travel_progress()`
  (telemetry `trip`) unchanged; `cancel_travel()` (`team.rs:255`) unchanged.
- `pub` items removed from ac-client: `travel::{LEG, STUCK_AFTER, STEP_GIVE_UP, leg_end}`,
  `Client::travel_to_in` (only walksim and ac-client used them).
- `docs/game/mechanics.md:966` (walksim) points at the route harness instead.

### Risks

- **A held journey stands while rows above its own act** (vitals, buffs, a fight). That is one row
  acting per tick, said truthfully in the status; the scenarios show whether any row now holds a
  journey for long.
- **Two rows asking different far goals in turn** would each plan a trip on its tick. A row keeps
  the tick while its journey runs (§5.2), so this needs two rows acting alternately; each takeover
  is logged ("takes the journey from"), which the scenarios' logs are read for.
- **Stands at a stretch's end** while the next region assembles (up to ~1.4 s for a town, phase 1
  §2's measures): the same wait today's legs have at each new 3x3. Not prefetched until measured.
- **A ground's spot** is the average of its spawns and may sit in a building: `GROUND_REACH` (30 m)
  as `within` lets the final walk end on any floor node near it, which is what "at the ground"
  already meant to the grow row (`growth/hunt.rs:202-205`).

## 8. Tests

Whole-loop tests run the real slot, follower and body at 20 Hz through `walk_journey` and
`tick_player` (the `walk_the_journey` driver, `travel.rs` tests `:1768`, renamed callers), data
tests.

| test | step | shows |
|---|---|---|
| `a_warden_in_the_facility_hub_is_planned_through_its_portal` (ac-world, no data) | 1 | with its cell: Town Network, Facility Hub, `Walk(Place{.., 0x8A0201F7})`; on the map alone the 30 min Qalaba'r way |
| `a_visit_to_a_warden_is_planned_through_the_facility_hub` (no data) | 1 | `visit_landmark` from Holtburg plans 2 portals and keeps the cell |
| `a_journey_put_down_for_a_fight_is_resumed_to_its_place` (no data) | 1 | `remember_journey` then the resume row: `journey_goal()` has the place and cell |
| `a_death_upstairs_is_walked_back_to_upstairs` (no data) | 1 | `way_back` is the death place, cell and height |
| `a_journey_to_an_indoor_goal_arrives_in_its_cell` | 2 | A9B2 platform to Archmage Cindrue (0xA9B4011B): a corridor stretch, then into the house and upstairs; ends in that cell, `reached` in 3D |
| `a_visit_to_a_warden_arrives_at_the_warden` | 2 | from the Facility Hub portal's exit (0x8A020212) the visit walks to 0x8A0201F7 and uses the Warden |
| `a_journey_whose_first_walk_has_no_way_is_refused_at_once` | 2 | 01F6 room to the 9.5 m ledge (`a_journey_leg_with_no_way_is_planned_again_at_once`, rewritten): `journey_to` false, not travelling, not a frame walked |
| `a_journey_that_loses_a_step_is_planned_again_without_it` | 2 | `testkit::set_out_on` hands the engine a trip whose next portal's mouth is the 01F6 ledge, as if just arrived: within a frame the mouth is refused, the new trip has no step at it, and the journey arrives at its goal |
| `an_overland_walk_is_planned_a_region_at_a_time` | 2 | the BAAD ground toward Holtburg (`a_ground_reached_through_a_one_way_portal_has_a_way_home_on_foot`): each stretch ends inside the 3x3 round where it began, is routed before a step, and the body keeps moving for 90 s |
| `a_portal_in_a_hall_is_walked_to_on_its_own_floor` | 2 | replaces `a_portal_in_a_hall_is_aimed_at_on_its_own_floor`: the stretch's place is the mouth at 96.0 in 0xA9B4017A |
| `the_planner_answers_from_the_region` | 2 | `has_way_to_answers_from_the_region` on `plan_walk` alone |
| `a_journey_stands_while_its_row_stands_aside` | 3 | grow's journey; the fight row acts 20 ticks: no walk in the slot, the body stands; grow acts: it walks on, `Travel.trip` and its step unchanged, one stretch planned |
| `a_journey_carried_by_another_row_is_claimed` | 3 | catch up sets off, follow acts: the stretch's owner is "follow", the body walks |
| `a_journey_of_no_row_says_travelling` | 3 | a script's `travel_to` with autoplay on: status "travelling to ...", not "waiting" |
| `a_walk_out_of_a_shop_is_the_journeys_first_walk` | 3 | `grow_travel` from Cindrue's upstairs to a ground 400 m off: the first stretch begins in 0xA9B4011B and leaves by the door; replaces the `after_out` tests in `road/tests.rs` |
| `a_counter_no_journey_reaches_is_passed_at_once`, `a_ground_no_journey_reaches_is_skipped_at_once` | 3 | replace the `walk_limit` tests (`counter/tests.rs:736,833`, `growth/tests.rs:175-185`) |
| `a_journey_put_down_for_a_fight_arrives_at_its_place` | 3 | whole loop: a script's journey to Cindrue, a swing breaks it off 5 s in, the resume row walks it to 0xA9B4011B |

Kept green unloosened:
- `the_facility_hub_is_walked_down_to_its_lower_portals` (reads `step_goal`/`going_to`, not
  `travel_goal`'s leg), `the_stairs_up_to_a_scrivener_are_walked_down_too`;
- `a_walk_out_of_a_shop_to_a_spot_outside_the_door_reaches_the_next_counter`,
  `a_corpse_in_the_holtburg_dungeon_is_walked_to_from_its_portal`,
  `a_corpse_on_a_floor_below_in_a_dungeon_is_walked_to`,
  `a_lifestone_known_is_the_way_to_a_far_counter`;
- the visit, follow, recovery, road, hunt and counter tests, `tests/academy_route.rs`,
  `dungeon_edges`.

### The route harness: journey trials

The harness walks only within a region. Step 2 adds, before the engine changes:
- `walk_trial::walk` carries a body through a portal as the server would: while the journey's step
  is a portal and the body stands within `PORTAL_TOUCH` (1.0 m flat, `SAME_FLOOR`; a harness
  constant, the server's collision with the mouth) of its mouth, it is set down at the exit place.
- `route_harness.rs` gains `journeys()` and the test `journeys_across_regions` (limit 300 s each),
  reported as the others are (arrived, refused, gave up, still pushing, and seconds):

| trial | start | goal | kind |
|---|---|---|---|
| A9B4 Holtburg to a Facility Hub Warden | (32532.0, 34567.1, 94.0) 0xA9B40019 | Warden, 0x8A0201F7 (108.34, -46.2, 0) | 2 portals, 3 regions (the §2 case) |
| A9B2 platform to Archmage Cindrue upstairs | (32526.2, 34261.9, 97.5) 0xA9B20000 | 0xA9B4011B (152.3, 132.5, 69) | corridor, a building, a floor up |
| E632 out of Archmage Yuzai Zhen's | 0xE6320103 (112.1, 131, 30.1) | open ground about 400 m off (the builder's pick, on a floor, recorded) | bugs.md "Stepping out ... at Nanto" |
| ACB5 mine floor to the open ground | 0xACB501E2 (125.5, 134.1, 40.4) | open ground about 400 m off (recorded) | bugs.md "No walk out of the ACB5 mine" |
| A9B4 Holtburg to the 01F6 ledge | the Holtburg lifestone | (262.0, 47182.0, 9.5) on its cell | a portal, then refused on arrival |

The bar: run on step 1's engine (legs) when added, then after each code commit of steps 2 and 3.
Never worse, and never "still pushing". The walks keep the main bar: stalls 12 trials, 10 arrived,
2 refused (2CB5 coast edge, 5C49 ledge); random 135 trials, 122 arrived, 13 refused; 0 still
pushing; INVARIANT planned-must-arrive holds. The ledge trial is expected to be refused within a
few seconds of arriving in the dungeon, where legs stood out their clocks.

## 9. Names, files and gates

- **New fns** (four words or fewer, glossary nouns): `set_off`, `journey_to`, `plan_again`,
  `walk_journey`, `step_goal`, `journey_goal`, `journey_owner`, `keep_journey`, `is_own_journey`,
  `put_on_floor`, `Place::on_map`, `Step::walk_end`, `Ground::place`, `Corridor::exit_from`,
  `testkit::set_out_on`. New types `Corridor`, `StepState` (`grep -rn "struct Corridor\|struct
  StepState"`: none today).
- **Renames** (listed old -> new in their commits): `Step::Portal { mouth: Vec2, mouth_cell, exit:
  Vec2, exit_cell }` -> `{ mouth: Place, exit: Place }`; `Step::Walk(Vec2)` -> `Walk(Place)`;
  `Recall`/`Gem` `{ exit: Vec2, exit_cell }` -> `{ exit: Place }`; `Step::end() -> Vec2` ->
  `-> Place`; `travel_to_in` -> `journey_to`; `plan_trip`/`plan_trip_in` -> `set_off`; `walk_leg`
  -> `walk_journey`; `travel_goal` (the leg) -> `step_goal`; `Autoplay.resume_trip: Vec2` ->
  `Option<Goal>`; `Recovery.death_cell` -> `death`. No saved config field is renamed.
- **Files.** The tests of `travel.rs` move to `travel/tests.rs` first, a pure move; the corridor is
  `travel/overland.rs`. `travel.rs` should fall under 1,500 production lines: its `LONG_FILES` row
  (`tests/file_length.rs:34-38`) then goes, in the same commit.
- **Code maps** (`crates/ac-client/CLAUDE.md` is at its 200-line ceiling: rows edited in place):
  the travel row's entry fns `travel_to`, `travel_about`, `walk_journey`, `autoplay_resume_journey`
  and files with `travel/overland.rs`; the glossary's journey row: "a planned trip of walks,
  portals and recalls (`Travel`), owned by the row that set it off; each walk a stretch the slot
  follows"; the tick-path line on `settle_going` gains "holds a journey whose row did not act".
- **Comments.** Each new item two lines at most, module docs five; the deleted clocks' stories go
  with them. `travel.rs`'s 17-line module doc becomes five, in a comment-only commit.

## 10. The steps

Each builder step: fmt, clippy and targeted tests green on every commit; before its last commit the
full `cargo test-data`, `RUSTDOCFLAGS=-Dwarnings cargo doc --workspace --no-deps`, and the route
harness, each exit code checked itself, never through a pipe; SUMMARY, AGREE, WAITED and INVARIANT
lines and the journeys' outcomes quoted in the step's last commit. Comment-only edits, pure moves
and code in separate commits, staged by path. `tools/scenarios.sh` is the coordinator's.

### Step 1: places through the journey (one fix of behaviour: the cell is kept)

1. **ac-world: steps carry places** (§3 types, `Place::on_map`, `Ground::place`). All `plan_*` take
   `from: Place, goal: Place`; `examples/plan_trip.rs`; the client adapted where it reads steps
   (`recalls.rs:83-143`, `town_run/vendor.rs:32`, `growth/hunt.rs:531-551`, `travel.rs`'s reads of
   `mouth_cell`/`exit_cell`; `step_z` from `mouth.at.z`, so `portal_mouth_height` goes). Tests:
   the trip tests on places, `a_warden_in_the_facility_hub_is_planned_through_its_portal`.
2. **ac-client: a journey's goal is a `Goal`** (§3's table): `Travel.goal`, `journey_goal`,
   `journey_to`, `travel_to` via `Place::on_map`, `go`'s far branch, the visit's journey, follow,
   growth and town run goals, `resume_trip`, recovery's `trip` and `death`. The legs still walk.
   Tests: `a_visit_to_a_warden_is_planned_through_the_facility_hub`,
   `a_journey_put_down_for_a_fight_is_resumed_to_its_place`,
   `a_death_upstairs_is_walked_back_to_upstairs`.
3. Harness: outcome for outcome as main.

Targeted: `-p ac-world trip`, `-p ac-client travel::`, `visit`, `recovery`, `journey`, `follow`,
`growth`, `going`.

### Step 2: each walk planned whole; legs go

1. Pure move: `travel.rs`'s tests to `travel/tests.rs`.
2. Harness journeys (§8) and `walk_trial`'s portal carry; `Pathfinder::put_on_floor` with a unit
   test (a map point on the Holtburg terrain; a ledge cell's floor). Run: the journeys' bar on legs.
3. Code: regions that hold the end (§4.1, §4.2, §4.4-§4.8): `set_off`, `walk_journey`,
   `step_goal`, `StepState`, `plan_again`, `replan_due`; the gem's face step on the slot.
   Deleted: the indoor leg, `step_cell`, `step_z`, `step_block`, the end-cell guess,
   `has_way_to`, `floor_in_cell` (recovery's use went in step 1), `STEP_GIVE_UP`,
   `STEP_PROGRESS`, `REPLANS`, `FACE_GIVE_UP`, `travel_can_reach`, `walk_leg`. Overland steps
   still walk the waypoint loop through `step_goal`, its end-of-step replan (`:1493-1508`) now
   §4.5's rule. Tests:
   `a_journey_whose_first_walk_has_no_way_is_refused_at_once`,
   `a_journey_that_loses_a_step_is_planned_again_without_it`,
   `a_visit_to_a_warden_arrives_at_the_warden`, `a_portal_in_a_hall_is_walked_to_on_its_own_floor`,
   `the_planner_answers_from_the_region`.
4. Code: the overland corridor (§4.3, `travel/overland.rs`). Deleted: the waypoint loop and skip,
   `leg_end`, `leg_height`, `steering_block`, `LEG`, `EDGE_MARGIN`, `STUCK_AFTER`, `PROGRESS`,
   the ring, `route`,
   `next`, `leg`, `leg_refused`, `best`, `last_progress`, `measured_on`, `restart_waypoint`,
   `travel_displaced`, `examples/walksim.rs`; the `LONG_FILES` row if under the limit. Tests:
   `a_journey_to_an_indoor_goal_arrives_in_its_cell`,
   `an_overland_walk_is_planned_a_region_at_a_time`.
5. Comment-only: module docs of `travel.rs`, `travel/overland.rs`, `body/going.rs` (walk_journey),
   `docs/game/mechanics.md:966`.

Targeted: `-p ac-client travel::`, `going`, `visit`, `gem_room`, `pathfinder`, `--test
route_harness` with `ROUTE_TRIALS` for the journeys, `--test dungeon_edges`.

### Step 3: a journey's walk is its row's

1. Code: `Travel.row`, `journey_owner`, `keep_journey`, `is_own_journey`, `Going.journey`; the rows
   of §5.2's audit changed to read their own journey or claim it; journeys still walk whenever
   free, so behaviour is phase 3's with the row as the walk's label. Test:
   `a_journey_carried_by_another_row_is_claimed`.
2. Code: the walk out of a building is the first stretch (§5.4 first item). Test:
   `a_walk_out_of_a_shop_is_the_journeys_first_walk`; `road/tests.rs` off `after_out`.
3. Code: the errands' walk clocks go (§5.4 second item). Tests: the counter and ground tests of
   §8.
4. Code: holding (§5.2), §5.3's rules for walks, `interrupt_travel` and `remember_journey`, the
   idle line. Tests: `a_journey_stands_while_its_row_stands_aside`,
   `a_journey_of_no_row_says_travelling`, `a_journey_put_down_for_a_fight_arrives_at_its_place`;
   the follow, recovery, town run and growth tests green.

Targeted: `-p ac-client growth`, `town_run`, `follow`, `journey`, `recovery`, `going`, `travel::`,
`hunt`, `steps`.

### Step 4: maps and docs

1. Comment-only: `Owner::Journey`'s doc, `autoplay/journey.rs`, `growth/road.rs` module and item
   docs; the code maps (§9).
2. Docs: `docs/movement-plan.md` gets the "Phase 4" row (both SUMMARY lines, AGREE, WAITED,
   INVARIANT, the journeys' outcomes) and its status line; this file's status block with where the
   build departed. `docs/bugs.md` is left for the coordinator, whose scenario runs close entries.

What the scenarios should show (the coordinator's run): no "waiting" sample while a journey with a
row is under way; no stand over 20 s on a journey; a refused journey refused within seconds, not
30 s; Blargerton's Warden visit through the Facility Hub, arriving in 0x8A0201F7.
