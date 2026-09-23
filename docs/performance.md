# What a session costs

Measured 2026-09-09 on an Apple M3 Pro against a local ACE server.
Numbers here are a baseline to notice regressions against, not a
promise: re-run the commands before trusting them on other hardware.

## The short version

Running many sessions is cheap. The parts that are expensive -- the
world grid, the wide planner, a landblock's collision and nav graph,
the DAT archives -- are loaded once and shared, and the GPU only ever
holds the session the window is showing. What a further session adds is
well under a megabyte and a fraction of a percent of a core.

| | Value |
|---|---|
| Process, warm, no server | 66 MB footprint |
| Process, one live session standing in the world | 29 MB footprint |
| Each further live session | ~0.6 MB footprint |
| Each further idle session (no server) | ~0.2 MB footprint |
| Idle tick | ~2.4 us per session |
| Four live sessions, headless | 0.3% CPU |

## Offline floor

```
AC_DATA_DIR=~/Downloads/ac_data cargo run --release -p ac-client --example session_cost 24
```

One session warms everything shared, then 24 more are added and only
the growth from there is charged to a session. The collision, nav
graph, world grid and pathfinder rows all read 0.0 MB for those 24,
which is the sharing working.

Read the per-phase rows as the price of the *process*, not of a
session: the world grid is 24 MB and the wide planner 13 MB however
many sessions run. An earlier version of this harness divided those by
N and reported them as a per-session cost, which overstated a session
by more than twenty times.

These sessions never hear from a server, so they carry no objects and
no scenery. Treat 0.2 MB as a floor.

## Live cost

```
./target/release/acswarm --headless --data-dir ~/Downloads/ac_data --connect 127.0.0.1 \
  --client acct1:pass --client acct2:pass ... --duration 90
# then, against the pid:
vmmap --summary <pid> | grep 'Physical footprint:'
ps -o %cpu= -p <pid>
```

One session placed in the world: 29.0 MB. Four: 30.7 MB. That is about
0.6 MB for each session after the first, with its objects and its
landblock loaded. Four together held 0.3% of a core, sampled over 24
seconds.

Caveats worth keeping in mind: four is a small sample, the characters
were standing still rather than fighting or autoplaying, and the server
was local, so nothing was waiting on a network.

## Where the cost is not

Not in per-session memory, and not in the headless tick. If a machine
runs short it will be the rendered window or the server, not the
simulation.

Only the active session holds GPU state. Switching sessions drops the
old one's pickables and animation players and re-instances the new one
on the next frame (`bins/acswarm/src/main.rs`, `switch_to`), so the
window costs one session's worth of GPU whatever else is running
alongside it.

`acswarm --headless --tick-hz` sets the pace for every session in a process; 20 is
the game's, and a process of followers gets by on 10.

## Rendering cost (the window)

The goal is speed and low resource use, not fidelity: one rendered
session and up to nine followers in one process must fit on an ordinary
machine. What the renderer does about it, in `bins/acswarm/src/gpu.rs`
and the frame loop in `main.rs`:

* **Only the active session touches the GPU.** Landblocks stream, objects
  are instanced, particles simulate and animations advance for
  `App::active` alone; the other sessions tick their network, physics and
  plugins and keep no GPU state (`Net` holds only their `Client`, `anims`
  and `pickables`, and a switch clears the last two for the session left
  behind). `--render none` goes further for a follower window: nothing of
  the world reaches the GPU, only the sky and the overlay are drawn.
* **Culling.** Every static batch (one per material per landblock) carries
  a world-space box and every instance a bounding sphere; `Gpu::draw`
  tests them against the view frustum each frame. Instances farther than
  the draw distance, or too small on screen to cover a pixel and a half,
  are skipped too. The draw distance is the options panel's "draw
  distance" slider (`options.draw_distance` in the settings, published as
  `render.draw_distance` on the blackboard, 0 = no limit), read by the
  viewer each frame. `ACV_NO_CULL=1` draws everything, for comparisons.
* **Instancing.** Model matrices live in one storage buffer indexed by the
  instance index. Visible instances are sorted by mesh and every copy of a
  mesh (the same GfxObj in the same look) is one `draw_indexed` with an
  instance range; static batches read record 0 (the identity). A bind
  group already set is not set again.
* **Frame pacing.** `--fps` caps the window (default 60) through
  `ControlFlow::WaitUntil`; an unfocused window draws at 10 fps at most, a
  hidden (minimised or covered) one draws nothing, and a frame in which
  neither the camera, the GPU scene (`Gpu::is_dirty`) nor the egui output
  (`Ui::repaint_wanted`: the shapes compared with the last pass) changed
  is skipped. The sessions keep ticking at the paced rate regardless.
* **Memory.** Textures are decoded and mip-mapped once per material and
  shared by every batch, mesh and session in the process. When a landblock
  is unloaded, and every 30 s, materials and GPU meshes nothing references
  any more are dropped (`Gpu::prune_materials`, `scene::prune_gpu_meshes`).
  `--max-texture N` uploads from a smaller mip level (128 quarters the
  texture memory) for a blurrier but lighter world.

`--perf` measures it: the status line shows the last frame's time and
draw counts, and a summary is printed at exit (average, p50, p95 and max
frame time; draw calls, triangles, instances and batches drawn and culled
per frame; materials, texture MB, meshes and buffers held). A windowed run
records every presented frame; a headless `--connect --screenshot --perf`
run draws offscreen at the `--fps` rate and waits for the GPU each frame,
so its frame time is the whole cost of a windowed frame; an offline
`--landblock --screenshot --perf` draws the still scene a hundred times.

Measured on an Apple-silicon laptop at 1280x800 (release build, the
machine otherwise busy, so treat differences under half a millisecond as
noise; `before` is the same build with `ACV_NO_CULL=1` and the old
one-draw-per-instance path):

| Scene | | frame ms avg / p95 | draw calls | triangles | instances drawn | textures |
|---|---|---|---|---|---|---|
| Holtburg town, offline `--landblock A9B4 --radius 1`, overhead view | before | 2.10 / 2.32 | 256 | 154k | 0 | 91 MB |
| | after | 1.49 / 1.53 | 256 | 154k | 0 | 91 MB |
| | after, `--max-texture 128` | 1.38 / 2.07 | 256 | 154k | 0 | 65 MB |
| Open country, offline `--landblock A6B4 --radius 1` | before | 1.21 / 1.96 | 137 | 143k | 0 | 83 MB |
| | after | 1.22 / 1.96 | 133 | 143k | 0 | 83 MB |
| Connected, dungeon (Qalaba'r), 339 instances, 96 emitters, four panels open | before | 4.87 / 7.21 | 731 | 127k | 317 | 44 MB |
| | after, culling | 4.36 / 6.11 | 188 | 71k | 70 | 44 MB |
| | after, culling and instancing | 4.40 / 6.11 | 149 | 70k | 69 | 44 MB |
| | `--render none` (a follower) | 3.60 / 11.04 | 1 | 0 | 0 | 0 MB |

The offline overhead views look down on all nine blocks, so nothing
culls and the frame is the fixed cost of 250 batches. The connected
frame, headless, is about 2 ms of CPU (half of it the egui pass over the
open panels, the rest ticking the session), under 1 ms encoding the draw
calls, and 2 ms of GPU work and pass latency; in a window the GPU part
overlaps the next frame's CPU, and a frame in which nothing moved costs
only the tick and the egui pass. A follower window (`--render none`)
holds no textures, meshes or buffers at all. Pixel-for-pixel, the offline
screenshots are the same before and after (the water animates with the
clock, nothing else differs).

Left to do, in order of likely payoff: compressed textures (the DATs are
RGBA8 at up to 256x256; BC1/ASTC would cut the texture memory four to
eight times), merging the static batches of neighbouring blocks by
material (a third of the draw calls in a town), and an egui pass only
when its input or data changed (it is half the CPU time when idle).

## Measuring many sessions

```
ACSWARM_LOAD_PASSWORD=... tools/load-test.sh 25 300             # 25 sessions, one process, 5 min
ACSWARM_LOAD_PASSWORD=... tools/load-test.sh 25 300 --procs 5   # the same 25 in five processes
```

The harness logs `load01` to `load25` into the local ACE at 127.0.0.1
(`--prefix` changes `load`; ACE creates an account at its first login),
each with one character named from its number, "Load Zero One", made
when missing. Names are letters and spaces because the retail creation
screen allowed nothing else (`valid_name` in
`crates/ac-client/src/creation.rs`); ACE itself refuses only taboo words,
creature names and names in use (`CharacterHandler.cs:51-70`). Once
placed, each session types `/load_autoplay`, a script command hook that
turns autoplay on, so the sessions play rather than stand.

Every 5 s it samples each process's RSS and CPU (`ps`) and the server
container's CPU and memory (`docker stats ace-server`), and at the end
prints one table. It refuses to start with under 15 GB free or when an
`acswarm` already holds one of its accounts. Ctrl-C logs the sessions off
and still prints the table; a second Ctrl-C gives them 15 s to log off
and then kills them. A session killed without logging off holds its
account on the server for about a minute, so a rerun straight after one
is refused as already logged on.

Each run gets its own config and cache directory under `--out` (default
`$TMPDIR/acswarm-load-DATE`), so it plays by the default rules and its
characters stay out of the everyday settings, ledger and holdings. There
too: `procN.log` (all a process printed), `samples.tsv` and `summary.md`.
Take numbers on a quiet machine, with the lid open and nothing building.

### The perf line

Every headless run, not only the harness's, logs one line every 10 s at
INFO on `acswarm::tick_meter` (in `RUST_LOG`'s default filter):

```
perf: N sessions, T ticks at R of 20 Hz, work p50 A ms p95 B max C of 50 ms, over O,
      behind K, late p95 L ms, per session S ms, plugins P ms, costliest NAME M ms
```

* **ticks at R of 20 Hz**: the rate the loop achieved against
  `--tick-hz`. A process that cannot keep up shows it here first.
* **work**: one loop iteration, from its start to just before it sleeps
  (never the sleep), against the period, 1000 / `--tick-hz` ms.
* **over**: iterations whose work alone ran past the period. **behind**:
  iterations after which the loop gave up its schedule and started the
  next one at once; lateness plus work can cause that too, so behind is
  at least over.
* **late p95**: how far past its due time an iteration began. While the
  loop keeps up that is the sleep's overshoot, the operating system's
  timer, which on a Mac measured about 10 ms on a 30 to 50 ms sleep and
  under 1 ms on a 2 ms one, so it falls as the work grows. Once the loop falls behind it is
  how far the iteration before overran, and grows with the load.
* **per session**: the mean time one session's own work takes in an
  iteration: its tick, its events and the lines it types. **plugins**:
  the plugin host's time per iteration, every session's calls together;
  part of it is per session (scripts, the autoplay panel) and part once
  per iteration (the fleet, holdings and lobby panels keep house on the
  first session's call). What is left of work is the loop's own:
  reconnects, starts and stops, the status lines. **costliest**: the
  session whose own work has the highest mean, by character name (by
  account before the character is known, or once it is stopped).

Each session added costs about per session plus its share of plugins.
A process is full when the rate drops below `--tick-hz`, behind climbs,
or work p95 nears the period. Percentiles come from a fixed histogram and
are within 3%; max is exact.

At exit a `perf run:` line covers the whole run and adds overruns per
minute, seconds measured against seconds of wall time, and the sleep
windows excluded. `tools/load-test.sh` parses it, and the test
`the_run_line_has_the_shape_the_harness_reads` pins its shape. The
monotonic clock the loop runs on stops while the machine sleeps (so
`--duration` does too). The boot clock (CLOCK_MONOTONIC on macOS,
CLOCK_BOOTTIME on Linux) runs on and cannot be set. When it gets more
than 2 s ahead of the monotonic clock between two iterations, a WARN
says how long the machine slept, and that window is dropped rather than
reported.

### The table

| row | what it is |
|---|---|
| sessions | asked; placed (a `placed in cell` line); with autoplay on (the script's reply) |
| RSS | peak of the processes' summed RSS over the samples, and that over the sessions |
| acswarm CPU | mean from each process's CPU time over the sampled span, summed over processes; peak is the highest summed `ps` %CPU sample |
| ACE CPU | the server container's mean and peak, and its peak memory; `-` without Docker |
| tick rate, tick work, overruns, late, per session tick, plugins | from the `perf run:` lines; with `--procs`, the worst figure of any process, column by column (the lowest rate) |
| measured | seconds measured and of wall time, both from the process measured least |
| sleep windows excluded | windows dropped for a machine sleep |

RSS counts the resident pages of the memory-mapped DATs, which are clean
and shared, so it reads far above the footprint figures earlier on this
page, and a sum over processes counts those pages once per process.
Compare RSS with RSS. When the server's CPU climbs with the sessions
while the client's tick work stays flat, the server is the bottleneck.

## Not yet measured

* Frame time in the window, with a person playing one session and
  others following. The headless tick is measured
  ([Measuring many sessions](#measuring-many-sessions)); the window's
  frame is `--perf`'s, and nobody has run it with followers.
* A session under load: fighting, casting, looting, autoplay running.
  The harness turns autoplay on in every session; no numbers yet.
* Many sessions across separate processes rather than within one.
  `tools/load-test.sh --procs` splits them; no numbers yet.
