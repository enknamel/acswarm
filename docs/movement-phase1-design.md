# Movement phase 1 design: the graph

Status: design, 2026-11-01, for phase 1 of `docs/movement-plan.md`. Nothing here is built yet.
Line anchors are on `df4433b` (branch `phase1`).

Phase 1 delivers one planner and one graph, and makes the old steering ask it. Steering logic is
not changed (phase 2 does that). The route harness (`crates/ac-client/tests/route_harness.rs`) is
the measure. The baseline is: stalls 7/9 arrived (1 refused, 1 still pushing); random 108/120
arrived (5 refused, 7 still pushing). No step may be worse on either line, and no step may add a
"still pushing" that was not there before.

Rules this design keeps:
- Every walk is planned as a whole route, 3D and cell-aware, through one planner.
- No spot patches, no threshold tuned to make one trial pass, no straight "try it" in the new code.
- Arriving and reaching are measured in 3D.
- Doors are walked straight through.

## 1. `Place` and the query contract

### Where the types live

Crate layering (`Cargo.toml`):
- ac-scene uses dat, formats and store.
- ac-world uses formats and net.
- ac-nav uses formats, scene and world.
- Neither ac-scene nor ac-world can see the other.

Phase 4 puts a `Place` into `ac_world::trip::Step::Walk`, so `Place` must live in ac-world or
below it. The planner's machinery needs collision, so it lives in ac-scene. The contract that
joins the two lives in ac-nav, the one crate that sees both.

| item | crate, file | why there |
|---|---|---|
| `Place` | ac-world, `src/lib.rs`, beside `landblock_origin` and `outdoor_cell` (:361-383) | trip, nav and client all name it |
| graph, walk test, search | ac-scene: `nav.rs`, new `stride.rs`, new `wayfind.rs` | they need collision and cells |
| `plan`, `Route`, `Waypoint` | ac-nav, new `src/routing.rs` | it converts `Place` to and from the graph's `(Vec3, u32)` |

The new leaf module names (`stride`, `wayfind`, `routing`) are used nowhere else in the workspace.

### Names that collide today, renamed first

These are separate rename commits, made before step 1's code commit. Each commit message lists
old -> new.

| today | becomes | where | why |
|---|---|---|---|
| `ac_world::towns::Place` | `ac_world::towns::Town` | `towns.rs:15`, `action/resolve.rs:64`, ac-plugin `fleet.rs`, `map.rs` (`PLACES` stays) | the name is taken by the new type |
| `ac_world::spawns::Place<'a>` | `ac_world::spawns::SpawnCell<'a>` | `spawns.rs:50,56,149` | the same |
| `ac_nav::steering::Route` | `ac_nav::steering::Course` | `steering.rs:41`; readers `bins/acswarm/src/route_marks.rs:94`, `body/player_tick.rs:209`, `autoplay/loot/walk.rs:189` | the planner's `Route` means "planned", this one means "being followed"; phase 2 replaces it |

### Types

```rust
// ac-world, src/lib.rs
/// Where something stands: a world position and the cell it is in (interior >= 0x100, else outdoor).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place { pub at: Vec3, pub cell: u32 }
impl Place {
    pub fn is_indoors(&self) -> bool;         // cell & 0xFFFF >= 0x100
    pub fn is_located(&self) -> bool;         // cell & 0xFFFF != 0: a real cell, not just a block
    pub fn of_position(p: &Position) -> Place; // landblock_origin(p.cell) + p.local
}

// ac-scene, src/nav.rs
pub enum EdgeKind { Walk, Drop, Doorway }
pub struct NavEdge { pub to: u32, pub kind: EdgeKind }   // replaces the bare u32 in `edges`

// ac-scene, src/wayfind.rs
pub enum NoWay { OffFloor, Stranded, Apart, Unreached, OutOfRegion }
pub struct Query<'a> {
    pub from: (Vec3, u32), pub goal: (Vec3, u32), pub within: f32,
    pub avoid: &'a [Vec3], pub berth: f32, pub indoors_too: bool,
}
pub enum Search { Found(Vec<(NavNode, EdgeKind)>), NoWay(NoWay), Unbuilt(Vec<(i32, i32)>) }

// ac-nav, src/routing.rs
pub struct Waypoint { pub place: Place, pub kind: EdgeKind }  // kind: the edge that leads here
pub struct Route { pub waypoints: Vec<Waypoint> }              // from the first step to the end
pub use ac_scene::wayfind::NoWay;
pub fn plan(area: &mut Area, from: Place, goal: Place, within: f32) -> Result<Route, NoWay>;
pub enum Planned { Route(Route), NoWay(NoWay), Building(Vec<(i32, i32)>) }   // step 3
pub fn plan_built(area: &Area, from: Place, goal: Place, within: f32) -> Planned;  // step 3
```

`NoWay` reasons:
- **`OffFloor`**: the goal stands on no floor of its own cell.
- **`Stranded`**: no node is a checked walk from the start, even after the start is pushed out of
  the wall.
- **`Apart`**: the region is fully built and its component labels differ.
- **`Unreached`**: the search ran out.
- **`OutOfRegion`**: an end lies outside the region.

### The contract

`plan` returns every waypoint from the first step to the last. Each has a place (position and
cell) and the kind of edge that leads to it. Otherwise it returns a `NoWay` reason.

- Every consecutive pair, the start to the first waypoint included, has passed the walk test (§4).
- The last waypoint is within `within` of the goal, in 3D.
- Nothing in the route has gone unchecked.
- `plan` builds whatever chunks it needs. `plan_built` builds nothing: when it needs an unbuilt
  chunk it says `Building(chunks)`, never a partial route and never a guess.

Two things are decided inside `plan`, so its contract stays at three arguments:
- **Berths.** `plan` fills `Query.avoid` itself from `portal_mouths_to_avoid`, which moves from
  `crates/ac-client/src/pathfinder.rs:378` to `routing.rs`, together with `PORTAL_BERTH`.
- **Indoors or out.** When both ends are outdoors, it searches first with interior nodes filtered
  out. Only if that gives `Unreached` does it search again with them in. This keeps today's
  `outdoors_only` preference (`navarea.rs:95`, `pathfinder.rs:356-363`) without rebuilding the
  graph.

An unlocated place (`is_located()` false: a caller that knows only the block, as
`body/player_tick.rs:103-110` passes today) gets its cell from `Area::locate(at)`. That is a point
in cell test, then outdoors. It is never the nearest node. It disappears in phases 3 and 4, as
callers come to carry places.

## 2. One graph per region

`navarea::Area` becomes the region. It already has both shapes:
- **A dungeon** is assembled alone, whatever it is asked for, with all of its cells. Its bounds
  come from the collision hull, not the block square (`navarea.rs:109-160`).
- **Outdoors**, it is the 3x3 blocks of `blocks_for` (`navarea.rs:46-73`), with the interior of
  every building in them.

What goes:
- `outdoors_only` leaves `nav::Ground` (`nav.rs:85`) and `Area` (`navarea.rs:95`). It becomes the
  query filter described in §1.
- `Area::path`, `path_to` and `grounded` (`navarea.rs:286-350`) and the `exact_to` flag
  (`pathfinder.rs` `Ends`) go. Both ends are resolved by cell and floor (§3), so there is no
  height to believe or distrust.

### Folding three planners into one

| today | phase 1 |
|---|---|
| per-block `NavGraph` on the frame thread (`BlockCollision::nav`, `blockcache.rs:53-61`; `Player::on_nav`/`find_path`, `player.rs:1195-1329`) | deleted in step 4. Its callers ask the region. |
| `navarea::Area` on the `pathfinder` thread, queried there and answered a frame or more later | the region graph, shared as `Arc<RwLock<Area>>` from step 3 |
| the `pathfinder` thread as a query service | stays, as the region builder: it assembles areas and builds chunks |
| `worldroute` | unchanged: a corridor over terrain, for journeys (phase 4) |

### No frame stalls (step 3)

1. **`Area` becomes `Send + Sync`.**
   - `CellIndex`'s `IndexedCell.environment: Rc<Environment>` (`interior.rs:257`) is replaced by
     `bsp: Arc<BspNode>`, a clone of the cell structure's `cell_bsp`. `BspNode` is `Clone`
     (`ac-formats/src/gfxobj.rs:144`).
   - `Area.scenes: HashMap<u32, Rc<LandblockScene>>` (`navarea.rs:80`) is replaced by
     `terrain: HashMap<u32, TerrainMesh>`, cloned at build. `TerrainMesh` is plain data.
   - A test pins it: `fn _is_shared<T: Send + Sync>() {}` called on `Area`.
2. **A process registry in `pathfinder.rs`.** `static AREAS: Mutex<Vec<Arc<RwLock<Area>>>>`,
   most recently used last, at most `AREAS` (4) entries. The frame side and the thread share it.
3. **Building in two halves.** `NavGraph::ensure_chunk` (`nav.rs:516`) is split:
   - `fn build_chunk(&self, ground: &Ground, c: (i32, i32)) -> ChunkBuilt` runs under a read lock.
     It produces the new nodes, the new `(from, NavEdge)` edges and the drop edges still waiting.
   - `fn apply(&mut self, b: ChunkBuilt)` runs under a write lock and only appends, so it takes
     microseconds.
   - The frame's `try_read()` fails only during an `apply`. When it fails, the answer is
     `Building`.
4. **The thread's jobs.**
   - **`Assemble { blocks, centre, capsule }`**: `Area::build`, then put it in the registry.
   - **`Build { area, chunks }`**: build exactly those chunks.
   - **Idle**: finish the most recently used area chunk by chunk, so its component labels become
     final (§4).
   - Only one builder exists, so chunks are built in sequence and node ids are the same for every
     reader.
5. **The frame handle.** It replaces `ask`/`take`. It locks the registry for a moment, finds the
   area that `holds(from, goal)` for this capsule, `try_read()`s it and runs `plan_built`.
   - A `Building(chunks)` answer, or no area at all, sends `Build` or `Assemble` and answers
     `Building`.
   - The function is `Pathfinder::plan(&mut self, from: Place, goal: Place, within: f32) -> Planned`.
   - The 1.5 s `ASK_EVERY` throttle (`pathfinder.rs:33`) is kept for sending jobs only. Queries
     on a built graph are synchronous and cost only the search.

A query is a complete route, a `NoWay`, or `Building`. `Building` is not an answer: the caller
still has no route.

### What happens to the synchronous helpers

| helper | phase 1 (step 4) |
|---|---|
| `Player::find_path` (`player.rs:1270`) | deleted. Steering asks through `Standing` (§5); doorway and dodge ask `Client::plan_walk`, which wraps `Pathfinder::plan`. |
| `Player::on_nav`, `Block.nav` (`player.rs:77,1195`), `BlockCollision::nav`/`navs`/`nav_count` (`blockcache.rs:44-66`) | deleted |
| `Client::has_way_to` (`body/movement.rs:32`) | `Pathfinder::plan(me, Place{at, cell}, within)`: `Route` gives true, `NoWay` false, and `Building` true (today's answer for "not known", `movement.rs:40-42`, until phase 3 makes it truthful). Its own last-leg check (`movement.rs:47-58`) goes, because the planner now checks the last leg. |
| `Player::standable_near` (`player.rs:1252`), `NavGraph::standable_near` (`nav.rs:772`) | `Area::standable_near(from: Place, around: Place, radius) -> Vec<Place>`: the nodes within `radius` of `around`, and, once the labels are final, only those in `from`'s component. Through `Pathfinder::standable_near`; `Building` gives an empty list. Callers: `doorway.rs:267`, `dodge.rs:787`. |
| `Client::floor_in_cell` (`movement.rs:62`) | kept for travel until phase 4; it becomes a call to `wayfind::floor_of` |
| `Player::line_blocked`, `walkable`, `line_drops`, `on_ground` (`player.rs:1112-1190`) | kept unchanged: they are the old steering's straight checks, deleted in phase 2 |

## 3. Ends resolved by cell and floor

These are free functions in `wayfind.rs`, over `nav::Ground` and `&NavGraph`.

**`locate(ground, at) -> u32`.** The cell a point is in.
- It runs `CollisionWorld::cell_at(at + 0.1 z)`, which is new and built on `CellIndex::cell_at`.
- To get there, `IndexedCell` (`interior.rs:251`) gains `id` and `walked` (the `is_walked_into`
  rule, `collision.rs:69-74`), so an overlay cell is never the answer.
- If several cells hold the point, the smallest by volume wins. If none does, the answer is the
  outdoor cell, `0` in graph terms.

**`floor_of(ground, at, cell) -> Option<Vec3>`.** That cell's own floor at that x,y.
- **Interior cell:** among `floors_at_xy(x, y)` (`collision.rs:447`) with `f.1 == cell`, the one
  nearest `at.z` within `END_FLOOR` (1.5 m, the capsule's `step_down`: a point further off its
  floor is not standing on it). If there is none, the same is tried at the eight `footing` points
  (`collision.rs:464`, radius `0.6 * r`), because a body stands on a stair edge its middle
  overhangs.
- **Outdoor cell:** cell-0 floors and the terrain together, the nearest within `END_FLOOR`. If
  none is that close, the terrain under x,y. Outdoors the terrain is the floor of last resort, as
  it is for the body's fall (`player.rs:1798-1805`). This replaces both `Area::grounded` and the
  `GOAL_ON_FLOOR` rule in `Player::find_path` (`player.rs:1300-1316`).
- Otherwise `None`. For a goal that is `NoWay::OffFloor`.

**The start.** `S = floor_of(from)`.
- The candidates are the nodes within `NEAREST_REACH` (2) columns on `S`'s floor (same interior
  cell, or both outdoors), nearest in 3D first, at most `JOIN_TRIES` (8).
- A candidate is kept when the walk test (§4) carries the body from `S` to it. That walk is the
  route's first leg, and it is checked.
- If none passes, the same is tried out to `STRANDED_REACH` (8). Then once more after
  `resolve_above` has pushed the start out of the wall, as `find_path` does today (`nav.rs:867`).
  If that fails too, the answer is `NoWay::Stranded`.
- Every kept candidate is an A* source, its cost the walk's length. This replaces `nearest_joined`
  (`nav.rs:844`), which falls back to the nearest node unchecked.

**The goal.** `G = floor_of(goal)`, else `NoWay::OffFloor`. The goal set is the union of:
- **(a)** nodes within `within` of `G` in 3D, with a clear chest line from node to `G`
  (`line_clear`, `nav.rs:395`), so nothing is reached through a wall. The route ends at that
  node.
- **(b)** nodes within `NEAREST_REACH` columns from which the walk test reaches `G`, stopping
  within `within` (the stride form of `body_comes_within`, `nav.rs:255`). The route ends at the
  point where the simulated walk stopped: where the body actually gets to, never the raw goal.

This is how `within` 0.25 to a spot on a bookcase gives `NoWay`
(`travel.rs:2206`), and how a counter is reached across the counter.

**The search.** A* from all the sources to the goal set.
- The heuristic is the 3D distance to `G` minus `within`, which never overestimates.
- `Query.avoid`/`berth` and `indoors_too` filter nodes and edges as they are expanded (§4). They
  are never baked into the graph.
- `search_building(&mut NavGraph)` builds a node's neighbouring chunks as the search reaches them,
  as `ensure_neighbours` (`nav.rs:696`) does now. `search(&NavGraph)` builds nothing: it
  notes each unbuilt chunk it needed and goes on. When it finds no route and has noted some, it
  answers `Unbuilt(chunks)`.
- **Up front:** when `nav.is_complete()` and no source shares a component root with any goal node,
  the answer is `NoWay::Apart` before any search.

**Smoothing.** String-pulling as in `smooth` (`nav.rs:946`), with one change: a shortcut is taken
only when the walk test (§4) passes on it with `SMOOTH_MARGIN` and the berth filter.
- A shortcut never spans a `Drop` edge.
- A waypoint whose cell differs from the one before it is kind `Doorway`, whether or not the
  shortcut kept the lattice's doorway node.

**What goes.** `points.push(goal)` (`nav.rs:874`) and `NavGraph::find_path`, `nearest`,
`nearest_within` and `nearest_joined`. `near` stays as the candidate scan, and nothing else picks
an end.

## 4. One walk test

### The step the body takes

The body's grounded step (`player.rs:1593-1750`) and its airborne step (`player.rs:1751-1846`,
everything except `fall_is_lost` and `back_to_footing`) move into `crates/ac-scene/src/stride.rs`
as pure functions over a trait:

```rust
pub trait Underfoot {
    fn walk(&self, from: Vec3, to: Vec3, cap: &Capsule) -> Walk;        // body: its 1-2 blocks in turn
    fn floor_over_feet(&self, at: Vec3, cap: &Capsule) -> bool;         // player.rs:1443
    fn floor_below(&self, at: Vec3, cap: &Capsule, depth: f32) -> Option<(f32, u32)>;
    fn terrain_at(&self, x: f32, y: f32) -> Option<f32>;
    fn is_dungeon(&self) -> bool;
    fn vertical(&self, from: Vec3, dz: f32, cap: &Capsule) -> Vertical;
    fn resolve_falling(&self, from: Vec3, to: Vec3, cap: &Capsule) -> Vec3;
    fn fits_falling(&self, at: Vec3, cap: &Capsule) -> bool;
}
pub enum Stride { Stand { at: Vec3, cell: u32 }, Blocked, Falls { at: Vec3 } }
pub fn stride(u: &impl Underfoot, from: Vec3, cell: u32, to: Vec3, cap: &Capsule) -> Stride;
pub enum Air { Flying { at: Vec3, vz: f32 }, Landed { at: Vec3, cell: u32 } }
pub fn fall_step(u: &impl Underfoot, at: Vec3, vel: Vec3, vz: f32, dt: f32, cell: u32, cap: &Capsule) -> Air;
```

There are two implementations:
- **`nav::Ground`**: one merged world and its terrain, for the graph.
- **`BodyFloor`, private in `player.rs`**: the `Rc<BlockCollision>`s that `Player::step` already
  gathers (`player.rs:1585-1588`), the `TerrainSampler` and the dungeon flag.

`Player::step` then calls `stride`/`fall_step`. The wedge counting (`player.rs:1641-1658`) stays
in the body, because it is body state, not geometry. Moving it out takes about 200 lines out of
`player.rs`, which is at 2,073 production lines against its 2,100 ceiling in `file_length.rs`.

### Edges

`Ground::hop(a, b, cap) -> (Option<EdgeKind>, Option<EdgeKind>)` replaces `walkable` + `joins` +
`body_reaches` (`nav.rs:184-300`) as the edge test in `build_chunk`.

- **Refusals first, cheap and certain.**
  - `|a.z - b.z| > MAX_EDGE_RISE`.
  - The chest line is blocked: a body never walks straight through a chest-high wall. The slide
    along it is the neighbouring edges. This is today's rule, at `nav.rs:234`.
- **Then, each way, the body's step itself.** `stride` is called every `BODY_STEP` (0.2 m, a
  frame's run at 20 Hz), always closing on the far node by at least `BODY_CLOSING`.
  - It must end within `BODY_ARRIVE` flat and within `step_up` in height. `Blocked` refuses.
    `Falls` refuses as a walk; it is the drop edges' input.
  - The kind is `Doorway` when the cells differ, otherwise `Walk`.
- **The same test everywhere.** Smoothing (§3), the first and last legs, and
  `drops_along` (`nav.rs:319`, still the old steering's `line_drops` until phase 2) all use this
  test. The indoor-only exception in `joins` goes. So does the sampler as an edge test: `walkable`
  remains only behind `Player::line_blocked`, for the old steering, until phase 2.

What this settles, from the plan's table:

| rule | after phase 1 |
|---|---|
| floor tracking | `footing` with +`step_up`/−`step_down`, inside `walk`, for both graph and body |
| door frames and lips | walked by both: the same `resolve_from` slide |
| low overhangs | pushed aside by both: `overhang_escape` |
| collision blocks | the graph sees the merged region (all 9 blocks), the body its 1-2. The region's set contains the body's, so the graph can only be stricter at a block edge. |
| normal.z 0.5-0.6 | one `FLOOR_Z` for floors and walls (step 2, last commit) |

**One slope threshold.** Today floors are `normal.z >= 0.5` (`collision.rs:452,484`) and walls are
`|normal.z| <= 0.6` (`collision.rs:546,635`), so a triangle between 0.5 and 0.6 counts as both.
- The new rule: `const FLOOR_Z: f32 = 0.664_174_15;`, the floor slope cosine from ACE
  `PhysicsGlobals.cs:50`, walkable at `normal.z >= FloorZ` (`PhysicsObj.cs:2863`).
- Floors take `>= FLOOR_Z`, walls `< FLOOR_Z` and above `-FLOOR_Z`, ceilings `<= -FLOOR_Z`.
  `ramp_points`' `RAMP_FLOOR` (`nav.rs:44`) becomes `FLOOR_Z`.
- This is its own commit. If a data test or the harness gets worse, the commit is not made.
  Instead the failing place becomes a harness case and the change waits for phase 2. The constant
  is never moved to pass a test.

### Drop edges

A drop is the body stepping off a ledge it can land from.

1. **Where they come from.** In `build_chunk`, for each node `a` and each of the eight
   directions, a forward `hop` that ended in `Stride::Falls { at }`.
2. **The fall.** `fall_step` runs at 20 Hz from `at`, with the horizontal speed `DROP_SPEED`
   (1.0 m/s, the speed the phase-2 follower steps off at), until `Air::Landed`.
3. **Refused** when the fall is deeper than `DROP_MAX`, or is still in the air after
   `OVER_NOTHING` (2.8 s, `player.rs:242`).
   - `DROP_MAX` is 12.0 m. ACE starts falling damage once the landing speed passes
     `11.25434 + 4.5` m/s (`Player_Move.cs:254-259`), which is about 12.6 m of free fall at 9.8.
4. **The landing.** The landing point `L` is resolved like a start (§3): a node on `L`'s floor
   within one column that the walk test reaches from `L`. The edge `a -> that node` is
   `EdgeKind::Drop`, costing its 3D length. It is one way only.
5. **Landing in an unbuilt chunk.** The landing goes into `NavGraph.drops_waiting: Vec<(u32,
   Vec3, u32)>`, and is joined when that chunk is built.
6. **The old steering.** It runs at run speed off a drop edge, so it can land further out than
   simulated. The harness measures how often; the phase-2 follower walks drops at `DROP_SPEED`.

### Berths per query

- `Ground.no_go` (`nav.rs:81`) goes from graph building: from `surface_at` (`nav.rs:103`), from
  the column loop and from the doorway seeding in `ensure_chunk`.
- A chunk built by one walk is the same chunk for every walk. That ends the baked berths of
  `on_nav` (`player.rs:1231-1241`) and of the thread's `a.avoid` (`pathfinder.rs:352-354`).
- At query time `wayfind` drops two things against `(avoid, berth)` using `nav::in_berth`
  (`nav.rs:62`):
  - a node in a berth;
  - an edge whose segment passes within `berth` (flat, within `BERTH_HEIGHT`) of a mouth.
- The first and last legs, and every smoothing shortcut, are checked against the same filter.

### Component labels

- `NavGraph` gains `roots: Vec<u32>`, a union-find with path halving.
- `apply` unites the ends of every new edge, of any kind. That gives weak components: two nodes in
  different components are certainly apart. Being in the same one is a hint only, because drops
  are one way.
- `NavGraph::is_complete()` is true once every chunk inside `gx`/`gy` is built. Only then is
  `Apart` answered without a search. The builder's idle work (§2) completes the regions that are
  being used.

### Cost of the walk test

The estimate:
- One `stride` on open ground is one `resolve_from` pass, one `footing` and one
  `overhang_escape`: about 1.2 of today's 0.5 m sampler samples.
- A 1.5 m hop is about 3 samples today, against 2 directions × 8 strides.
- So edges cost about 5-6 times as much.
- This agrees with the measured cost of running `body_reaches` on refused hops, which was 92 to
  445 ms for Holtburg (`nav.rs:227-230`).
- Expect about 3 s per block, up from about 0.6 s, before the fast path below.

The budget is the build time of each of the four blocks at no more than 2.0 × the baseline in §7.
The fast path keeps the single test exact:
- A hop whose swept box (segment ± `r + 0.5`) holds no triangle with `|normal.z| < FLOOR_Z` and no
  down-facing triangle within the capsule's height cannot be pushed or blocked. `stride` is then
  `footing` alone, so `hop` calls only that.
- This is the same function's result computed without its no-op parts. It is not a second test.

If the budget is still exceeded, the hop's reverse direction reuses the forward simulation when
that met no wall contact and stayed within `min(step_up, step_down)` at every step. The cost is
measured before and after.

## 5. Wiring the old steering to the new planner

`Steering::steer` (`steering.rs:254-473`) keeps all of its logic. Only what it is handed changes.

**The trait** (`steering.rs:117-121`):
- `find_path(&mut self, block: u32, from: Vec3, to: Vec3, goal_cell: u32, within: f32) -> Option<Vec<Vec3>>`.
- `ask_wide(&mut self, from: Vec3, to: Vec3, block: u32, goal_cell: u32, within: f32)`.
  `outdoors` and `exact_to` go, because the planner decides both from the places.

**`steer` gains `stop: f32`** and passes it on as `within`. Calls that change:
- `body/player_tick.rs:162` (it already has `stop`);
- the test drivers `travel.rs:1948` and `:2094`;
- the `Fake` in `steering.rs:615-650`.

**`Standing`** (`body/standing.rs`) is the seam:
- **Step 1:** `find_path` calls `Player::find_path`, now built on `wayfind::search_building` over
  the per-block graph, with `within`. `ask_wide` sends places to the thread, which answers with
  `routing::plan` on its areas.
- **Step 3:** `ask_wide`/`take_wide` call `Pathfinder::plan` on the shared region. `take_wide`
  returns `route.points()` on `Planned::Route`.
- **Step 4:** `find_path` calls `Pathfinder::plan` too. `Route` gives `Some(points)`; `NoWay` and
  `Building` give `None`.
  - On `None` the old steering does what it does today (`steering.rs:384-431`). That is the
    straight walk phase 2 deletes; phase 1 does not touch it.

**Places for the steering.** The start is `Place { at: player.world_position(), cell: player.cell }`.
The goal is `Place { at: goal, cell: goal_cell }`, located (§1) when only the block is known.

**Other callers moved in step 4:** `has_way_to` (`movement.rs:32`, called from `travel.rs:754`,
`explore.rs:320,416` and `autoplay/loot/choose.rs:796`), `doorway.rs:267-271` and
`dodge.rs:787,815`.

**Telemetry.** `WalkFrame.route` (`player_tick.rs:205-210`) and `route_marks.rs:94` keep reading
`Course.waypoints: Vec<Vec3>`.

## 6. The four steps

Each step:
- is committable on its own, with fmt, clippy, `cargo test-data` and rustdoc green;
- runs the harness (`--release`, `--test-threads 1`) and quotes both SUMMARY lines in its merge
  message, no worse than the last step;
- runs the cost test from step 1, `crates/ac-scene/tests/navcost.rs`, and quotes it;
- puts comment-only edits, pure moves and code changes in separate commits, with `cargo run -p
  xtask -- same-code BASE` on the comment-only ones;
- is staged by path.

### Step 1: places, ends by cell, checked legs, the contract

Commits:
1. Renames (§1): `Town`, `SpawnCell`, `Course`. One commit each, with old -> new in the message.
2. `crates/ac-scene/tests/navcost.rs`, a data test that is ignored by default. For each of 0xA9B4,
   0xF682, 0x8A02 and 0x0286 it builds a one-block `Area::build(&assets, &[b], &cap, b)` and runs
   `build_everything`, then prints `COST block nodes edges walk/drop/doorway chunks ms`. It also
   prints the plan ms over 20 seeded pairs.
   - It records the baseline. No behaviour changes.
3. The code:
   - `ac_world::Place`.
   - `CellIndex::cell_at` and `CollisionWorld::cell_at`: `interior.rs`, `collision.rs`.
   - `NavEdge` and `EdgeKind` in `nav.rs`. The `edges` type changes; `neighbours` returns
     `&[NavEdge]`.
   - `wayfind.rs`: `locate`, `floor_of`, `Query`, `NoWay`, `Search`, `search_building`, `search`,
     and smoothing moved from `nav.rs` with the new rules.
   - `routing.rs`: `plan`, `Route`, `Waypoint`.
   - `Area::path`/`path_to`/`grounded` are deleted, and so are `NavGraph::find_path`/`nearest*`.
   - `Player::find_path` calls `wayfind::search_building`, and its goal-height rule goes.
   - `pathfinder::plan` calls `routing::plan`.
   - The trait and `steer(stop)` change as in §5. `Ends` is deleted.
4. The harness reports the planner:
   - `Trial` gains `goal_cell`. `standing_place` already returns it, and `named` locates it.
   - `walk_trial::plan_first(c, t) -> (Result<usize, NoWay>, Duration)` runs `routing::plan` on a
     fresh `Area` before the walk.
   - Each TRIAL line gains `PLAN route N` or `PLAN no way WHY`, with ms. A third line,
     `AGREE planned+arrived A, planned+failed B, no-way+arrived C, no-way+failed D`, is the
     agreement the plan asks for. B and C are what later steps drive to zero.

Tests:
- **ac-scene `wayfind` unit tests**, on synthetic worlds built like those in `nav.rs`'s tests (from line 1023):
  - `a_goal_upstairs_is_routed_to_the_upper_floor_by_its_cell`: two storeys over the same x,y,
    cells 1 and 2, joined by a ramp.
  - `a_goal_on_no_floor_of_its_cell_is_no_way`.
  - `the_route_ends_where_the_body_gets_to_not_at_the_goal`: a goal behind a thin wall, `within`
    0.25.
  - `a_start_against_a_wall_begins_at_a_node_it_walks_to`: carries over
    `a_character_pressed_against_a_wall_can_still_be_routed`.
  - `every_leg_of_a_route_passes_the_walk_test`.
  - `a_counter_is_reached_across_the_counter`.
  - The existing `nav.rs` tests are moved onto `search_building`, and their assertions keep their
    meaning.
- **ac-nav `routing` data test:** `a_corpse_on_the_floor_below_is_planned_to_on_that_floor`, the
  harness's "0286 corpse on the floor below" pair. The route's last waypoint is within 0.6 m of
  the goal's floor and in its cell.
- **ac-client, whole loop:** `walk_trial::run` on the "0286 corpse" and "F682 Jeweler on a deck"
  pairs gives `Arrived`. These are data tests in `tests/route_harness.rs` that assert and do not
  just print.
- **Kept green without being loosened:**
  - the bookcase assert (`travel.rs:2206`);
  - `a_goal_on_a_roof_is_walked_to_on_the_roof` (`travel.rs:2242`);
  - `a_mine_is_walked_down_to_its_floor` (`travel.rs:2215`);
  - `the_stairs_up_to_a_scrivener_are_walked_down_too` (`travel.rs:2025`);
  - `a_counter_on_a_deck_up_a_ramp_is_walked_up_to`
    (`autoplay/growth/town_run/counter/tests.rs:780`).
- **Moved, not loosened:** `a_goal_standing_on_a_deck_is_planned_to_on_the_deck`
  (`crates/ac-scene/tests/navarea.rs:253`) moves from `Area::path_to` to
  `wayfind::search_building` with a raw `Query`. ac-scene cannot depend on ac-nav.

Expected harness effect: small. The new ends make the old steering's last waypoint a reachable
point instead of the raw goal.

### Step 2: the body's step as the edge test, drops, berths per query, labels

Commits:
1. **Move:** `stride.rs`, holding `stride`, `fall_step` and `Underfoot`. `Player::step` calls
   them through `BodyFloor`. No behaviour changes: `jump.rs`, `hillside.rs`, `dungeon_edges.rs`,
   `berth.rs` and the `player.rs` tests (from 2073) stay green as they are.
2. `Ground::hop` in `build_chunk` and in smoothing. `joins`, `body_reaches` and `walkable` are
   removed as edge tests. `body_comes_within` is rewritten on `stride` and stays in use by
   `has_way_to` until step 4.
3. Drop edges and `drops_waiting`.
4. Berths per query: `no_go` comes out of graph building, the filter goes into `wayfind`, and
   `portal_mouths_to_avoid` moves to `routing.rs`. `outdoors_only` comes out of `Ground`/`Area`
   and becomes the two-pass search.
5. Component labels, `is_complete`, and `Apart`.
6. `FLOOR_Z` (§4), alone and only if the harness and the data tests allow it.

Tests:
- **ac-scene unit:**
  - `a_door_frame_the_body_slides_past_is_an_edge`: today's `two_rooms(Some(1.0))`, refused by
    the sampler.
  - `a_low_overhang_beside_the_way_is_walked_as_the_body_walks_it`.
  - `a_ledge_over_a_floor_is_a_drop_one_way`, `a_ledge_over_nothing_is_no_edge` and
    `a_drop_deeper_than_falling_hurts_is_no_edge`, built on `a_ledge` (`nav.rs:1350`).
  - `a_berth_is_kept_per_walk_not_baked_into_the_graph`: one graph, two queries.
  - `rooms_with_no_door_between_are_apart_once_built`.
  - `the_fast_path_and_the_full_step_agree_on_open_ground`: one hop computed both ways.
- **ac-client, whole loop:** `every_sampled_edge_is_walked_by_the_body`. For 200 seeded edges
  each in 0xA9B4 and 0x0286, put a testkit body on `a`, drive it at `b` at 20 Hz with
  `tick_player` for at most 3 s, and assert it arrives within `BODY_ARRIVE` on `b`'s floor. This
  is the agreement test, graph against body.
- **ac-client, existing:** `a_mine_is_walked_down_to_its_floor`, the scrivener stairs and the
  bookcase tests stay green.

Cost: `navcost.rs` within the 2.0 × budget (§4), quoted in the commit message.

### Step 3: the shared region and the builder thread

Commits:
1. `Area: Send + Sync` (§2.1): `interior.rs` holds `Arc<BspNode>`, and `navarea.rs` holds
   `terrain` instead of `scenes`.
2. `build_chunk`/`apply` split (`nav.rs`), and `search` (no building) in `wayfind.rs`.
3. `pathfinder.rs` rewritten around the builder:
   - the registry, `Assemble`/`Build`/idle;
   - `Pathfinder::plan`, `Pathfinder::standable_near` and `Planned`;
   - `routing::plan_built`;
   - `settle` (testkit) now waits for the builder to go idle on the asked-for chunks.
   - `Standing::ask_wide`/`take_wide` move onto it.
4. Map rows: `crates/ac-client/CLAUDE.md`'s `pathfinder.rs` row becomes "region graphs built off
   the frame thread and planned on from it". The root map's ac-nav row adds
   `routing::plan`. Both maps are at their line limits (121 and 200), so the rows are edited in
   place.

Tests:
- **ac-client:**
  - `two_sessions_in_one_area_share_one_region_built_once`.
  - `a_plan_on_an_unbuilt_region_answers_building_then_a_route`.
  - `a_plan_never_waits_on_the_builder`: `Pathfinder::plan` takes under 5 ms while the builder is
    building, measured.
  - `an_area_is_shared_across_threads`: the compile-time `Send + Sync` pin.
- **Harness:** the wide answers now come from the shared region.

### Step 4: every planned walk on the region; the per-block planner deleted

Commits:
1. `Standing::find_path` calls `Pathfinder::plan`.
2. The rest move to the planner: `has_way_to`, `doorway.rs`, `dodge.rs`, `Client::plan_walk`.
3. Deletions: `Player::find_path`, `on_nav`, `standable_near` and `Block.nav`;
   `BlockCollision::nav`/`navs`/`nav_count`; `NavGraph::for_scene`. Tests that called them move
   to the planner: `travel.rs:2173,2205,2257`, and `crates/ac-scene/tests/nav.rs`, which moves to
   a one-block `Area`.
4. `crates/ac-client/CLAUDE.md`'s `body/standing.rs` row is updated to "the client's answers to
   `ac_nav::Ground`, from the region planner".

Tests:
- **ac-client:**
  - `a_walk_inside_a_building_is_planned_on_the_region_not_the_block`: the "9722 Jeweler from
    inside a building" pair. It asserts the planner's answer (route or `NoWay`); the outcome is
    reported, not asserted, because the old steering still owns the walk.
  - `has_way_to_answers_from_the_region`: the existing `travel.rs:2198,2206,2208` asserts, kept.
  - `a_vantage_is_chosen_only_where_the_planner_has_a_way`, for `dodge.rs`.
- **Harness:** the whole of phase 1, against the baseline. Write the AGREE line into
  `docs/movement-plan.md`'s baseline section as the "phase 1" row.

## 7. Risks and how each step measures cost

**What is measured, at every step and before and after it:**
- `cargo test --release -p ac-scene --test navcost -- --ignored --nocapture --test-threads 1`:
  build ms, nodes and edges by kind for 0xA9B4, 0xF682, 0x8A02 and 0x0286, and plan ms over the
  seeded pairs.
- The route harness: both SUMMARY lines, the AGREE line, and the per-trial `PLAN` ms.
- From step 3 on, `tools/load-test.sh` for memory and frame time, run alone and never in parallel
  with a build.

Baseline (main, release, `Area::build_everything`, one block): 0xA9B4 0.64 s, 0xF682 0.58 s,
0x8A02 0.59 s, 0x0286 0.22 s. Step 1 re-measures these with `navcost.rs` and records them here.

Step 1, measured (`navcost.rs`, release; build ms, then plan ms over 20 node pairs):

| block | before step 1 | after step 1 |
|---|---|---|
| 0xA9B4 | 644 ms, plans 30.3 ms | 643 ms, plans 30.6 ms |
| 0xF682 | 601 ms, plans 55.8 ms | 587 ms, plans 60.1 ms |
| 0x8A02 | 620 ms, plans 42.9 ms | 607 ms, plans 68.3 ms |
| 0x0286 | 231 ms, plans 82.6 ms | 227 ms, plans 94.2 ms |

Step 2, measured the same way. The collision commit (e92d306) made every query cheaper with
the graph unchanged; the columns after it are the step's own commits, each over the last.

| block | collision faster | body's step (hop) | drop edges | berths, labels |
|---|---|---|---|---|
| 0xA9B4 | 374 ms | 1173 ms | 1337 ms, 1305 drops | 1396 ms, plans 56.7 ms |
| 0xF682 | 356 ms | 1116 ms | 1334 ms, 3387 drops | 1352 ms, plans 89.7 ms |
| 0x8A02 | 322 ms | 796 ms | 811 ms, 172 drops | 810 ms, plans 195.5 ms |
| 0x0286 | 157 ms | 285 ms | 362 ms, 88 drops | 360 ms, plans 129.0 ms |

- Against step 1's 643 / 587 / 607 / 227 ms that is 2.2x, 2.3x, 1.3x and 1.6x: the two town
  blocks are over the 2.0x budget. The open-ground fast path is exact and per step (`Swept`).
  Reusing a plain forward walk for the way back saved nothing measurable and was dropped.
- Drops are built but no phase-1 query takes one (`WalkQuery::drops`): the old steering runs off
  a ledge at a run, and with drops in it stepped off a Holtburg upper floor instead of taking the
  stairs (`the_stairs_down_from_an_upper_floor_are_found_and_walked`). The phase-2 follower asks.
- `FLOOR_Z` (commit 6) waits for phase 2. With it the harness was unchanged, but
  `every_sampled_edge_is_walked_by_the_body` found a drop edge the body does not walk:
  0xA9B4 (32526.0, 34603.5, 102.0) to (32527.5, 34603.5, 94.0), a roof's edge the graph's
  0.2 m step falls from and the body's walking step stands on.

Step 3, measured the same way. The graphs are unchanged, node for node and edge for edge, and
building a chunk by `build_chunk` then `apply` costs what building it in place did: 1414 / 1377
/ 819 / 367 ms against 1414 / 1371 / 822 / 367 ms. The harness is unchanged outcome for outcome.

- The steering's wide answers come from the shared region, planned on the builder thread
  (`Pathfinder::ask`). Planned on the frame, as §2 had it, they took 0.4 ms at the median, but
  21 ms at p95 and 74 ms at worst over the harness's 115 asks. That is past the 5 ms the risk
  table allows, so its answer (the slow query moves to the thread) is taken for every ask.
  `Pathfinder::plan` stays synchronous, for step 4's callers.
- A `Build` job carries the walk, not only its chunks. The builder plans on what is built and
  builds what that names until the walk is answered, so an answer takes one job, not one round
  trip per ring of chunks.
- `search` (no building) answers `Unbuilt` whenever it reached an unbuilt chunk, not only when
  it found no route, as §3 had it. A route found round the chunks not yet built is not the
  route the whole graph gives: `the_walk_that_fell_keeps_to_the_floor_at_a_headless_frame`
  (`crates/ac-client/tests/dungeon_edges.rs`) took such a route and never got down. With the
  rule, every harness walk arrives in the same simulated time as at step 2.
- A walk no area could hold answers `OutOfRegion` at once (a dungeon already assembled without
  the goal, or a goal past the blocks `blocks_for` covers), instead of assembling an area that
  cannot hold it every ask.
- `tools/load-test.sh` needs the live server, so it was not run: phase 1 is measured offline.

| risk | where | how it shows | what is done |
|---|---|---|---|
| The walk test makes the graph about 5 × slower to build | step 2 | `navcost.rs` ms over 2.0 × the baseline | the open-ground fast path (exact), then reuse of the reverse direction (§4). The builder thread means a slow build delays the first answer and never stalls a frame (step 3); `PLAN` ms in the harness shows that delay. |
| Ends by cell refuse goals the old code reached (a point between floors, a goal on a stair nosing) | step 1 | the harness `no-way+arrived` count (C) | each one is a harness case. The fix is to the general floor rule (`floor_of`'s footing ring, or `END_FLOOR` taken from the capsule) and never a special case. |
| Drop edges land elsewhere at run speed, because the old steering does not walk drops | step 2 | `planned+failed` (B) on dungeon pairs | measured only. The phase-2 follower walks them at `DROP_SPEED`. |
| The region's merged collision is stricter than the body's two blocks at a block edge | steps 2-4 | B and C at block-edge pairs (the 2CB5 coast edge) | a missing edge costs only a missed route; it never makes a walk unsafe. The harness lists them for phase 2. |
| `FLOOR_Z` turns ramps between 0.5 and 0.66 into walls | step 2, last commit | data tests (the deck ramp `a_counter_on_a_deck_up_a_ramp_is_walked_up_to`) and the harness | the commit is not made. The ramp becomes a harness case and the slope rule waits for phase 2. The constant is never moved. |
| `try_read` contention, or a long frame query | step 3 | `a_plan_never_waits_on_the_builder`; `PLAN` ms over 5 ms at p95 | `apply` only appends. A slow query (long smoothing) moves to the thread and answers the next frame, still a full route or `NoWay`. |
| Memory: shared areas plus the body's block cache | step 3 | `tools/load-test.sh` own footprint, 32 MB per process today | the per-block graphs go in step 4, offsetting it. `AREAS` stays at 4 until measured otherwise. |
| `Building` reads as "no route", so the old steering walks straight for those frames | steps 3-4 | harness frames spent `Building` (counted by `walk_trial`) | accepted for phase 1: it is the old steering's fallback, deleted in phase 2. The harness's `settle` waits for the builder, so the planner itself is measured. |
| `player.rs` past its 2,100-line ceiling | step 2 | `file_length.rs` | the `stride.rs` move takes about 200 lines out before anything is added |
| Name gates (`naming.rs`, one type name with one meaning) | all | the gates | the new production fns (`locate`, `floor_of`, `plan`, `plan_built`, `hop`, `stride`, `fall_step`, `build_chunk`, `apply`, `cell_at`, `is_complete`) have no articles and four words or fewer; the three renames in §1 come first |
