# Bugs

Worked top down. Each entry says how it shows (a telemetry or log line, a `tools/scenarios.sh`
check) and is closed by a commit whose scenario run passes. Add new reports at the bottom of their
priority; a live report starts from the telemetry (`tools/telemetry.py`, `--marks` for F9 presses).

## 1. Stops the character playing

- **The same unreachable creature, again and again.** Scn Mage on the lower floor of the Mite
  Sentry building (0xBDAF0100) picked one Mite Sentry six times in four minutes, each time "getting
  Mite Sentry in sight" then "giving up: no damage in a while" after 20 s, with three roams from
  indoors cut short between and the ground's quiet minute waited out indoors (scenario idle2).
- **Wedged at the end of a long walk to a ground.** Scn Blade, 4.4 km on foot to hunt Banderling
  Guard, stood 81 s then 40 s at B2A10022 (34273.5 30958.9 90.0), 59 m and then 14 m from the goal,
  wedged 24/40 and 20/20 samples, no "no way" (scenario scn-town2).
- **A walk back that cannot close its last point loops.** Scn Seller in the ACB5 mine stood 390 s
  "back the way it came" 0.25 m from a trail point it never reached (RETRACE_ARRIVE 0.2); each
  stuck replan found no path and retraced to the same point (scenario scn-ghost; the mine itself is
  reachable now, but the loop is not bounded).
- **Held 0.42 m from a corner waypoint.** A new character on a platform in 0xA9B2 (78.2 85.9 97.5)
  stood 124 s "getting Black Rabbit in sight", 0.42 m from its route's first waypoint: a corner is
  held until the body stands on it (`Route::target`, ON_THE_SPOT) and it could not. Offline over the
  same static geometry the walk arrives, so something the map lacks was in the way (scn-shots).

## 2. Plays badly

- **Experience goes to weapon skills with no weapon to use them** (Scn Blade raised Heavy Weapons
  to 149 holding only the Academy's training bow). Found because the fixtures had no weapons; a
  character that has lost its weapon plays the same way.
- **Tapers counted as two needs** (Named and Component). Shows as: "short of Prismatic Taper, Lead
  Scarab, Prismatic Taper" in the status line.
- **The Academy tutorial fallback cannot finish for a bow soldier**: no damage to the Olthoi,
  then out of arrows.
- **The corpse choice flips each frame between two.** Scn Mage at ACB5 alternated "walking to
  Corpse of Small Fledgling Mukkir (19 m)" and "walking to Corpse of Drudge Slinker (13 m)" every
  tick for 2 s (scn-nav3, 15:25:00).
- **Our collision has a wall the server does not.** A new character's Lightning Bolt flew 44.3 m in
  0xABB4 through a wall our collision puts 28.1 m out (scn-shots, "shot:" notes): the shot test
  refuses shots the server would let through.

## 3. Tooling

- **"wedged" counts only fully blocked steps.** A step the walls slide back to where it began
  reads as progress, so a body going nowhere for 12 min showed wedged false in every sample; the
  steering's own no-progress check (`STUCK_AFTER`) is what saw it.
- **`--create` with no template makes a crippled character** (60 attribute points, 18 HP at
  level 5); use a template, as `tools/scenarios.sh` does.

## Fixed

- **A mine at ACB5 had no way in** (550b8d7): Mukkirs on a floor 17 m under the hill were fought
  from above, a corpse there waited on 36 s, a seller stood 390 s inside. The graph now leaves out
  overlay cells no portal leads into and joins indoor hops the body's own step walks; offline the
  walk goes down to the floor and back (a_mine_is_walked_down_to_its_floor).
- **Two town-run planners** (this merge): ac-vendor's whole-trip `errand::plan` was dead code, and
  the first stop and the next were chosen by different rules (and a 300 m reach). One rule now,
  `errands_for` through `choose_stop`, for every stop and the panel's button.
- **Autoplay's walk outlived autoplay; a ghost target cast at** (57ee6d4): see that commit.
- **Our spell taken for a fellow's arrow** (this merge): in a huddle, a fellow's arrow beside us was
  taken for our Frost Arc, its speed (24.9 against 40 m/s) feeding the arc aim; ours is now the one
  flying at the target.
- **Ran at a bookcase for 12 minutes** (759744a, e2890ea): Blargerton, squeezed between a row of
  bookcases and a chest 0.75 m apart in the Holtburg Dungeon (0x01F60233), was routed from the node
  nearest him, a lattice point snapped out to the bookcases' far side, on every replan. A route now
  starts at the nearest node a straight walk reaches. Offline on the real dungeon: still there
  after 30 s before; round the end of the bookcases to the corpse after. `find_path` cost unchanged
  (ACB5 185 -> 182 us, Holtburg 115 -> 117, the dungeon 985 -> 999). Scenarios: one run all passed;
  two had only the ACB5 ledge corpse (the entry above) over 30 s.
- **Never went to town when low on supplies** (95e6782, f372476): a due run waited 8 min after
  any run, waited out a walk to a ground as "busy", ranked under every fight (only the last step
  started one), stopped at 600 m for loot, and a component sized from a small taper line (two
  scarabs beside 100 tapers) was never urgent. Starting a run is now its own step over starting a
  fight; a supply run waits only after a futile one (a sale-only run keeps 8 min, or the fighters
  spent a third of their time selling), and buy lines ask the character's skills (kits need
  Healing). +Scn Taper, no tapers at the Mosswart ground, main -> branch: set off 20 s -> 0 s,
  tapers in the pack 150 -> 130 s. Scenarios (scn-town2 against idle2 on main): kills Mage 8 -> 14,
  Blade 12 -> 12, Bow 1 -> 6; in town 0-24% before, 0-23% after.
- **Standing about for no reason** (0d345d7): a crowded ground whose spot falls inside a building
  was camped from indoors for good (the new character, 134 s), and after a town run the quiet
  minute was waited out in a shop before any ground was chosen (four characters, 38-45 s). Idle
  20 s or more indoors, before -> after: Mage 42, Blade 45 -> 0, Bow 45 -> 0, Seller 38 -> 0, new
  character 164 -> 0 (scenarios idle1, idle2); the Mage's 93 s after is the entry above.

- **Stood in a pocket among props** (91c32c7): from where the body stood the graph found no path
  anywhere and the steering leaned on the prop for good (Mosswart ground 61 s, run E; the Academy
  spawn). The steering keeps a trail of where the body walked and, stuck with no path, walks back
  along it to where the graph finds a way on. Offline on the real map: before, 18.2 m short after
  30 s; after, at the goal in about 6 s.
- **Stood under a creature on a roof** (00e938b): an outdoor goal far above the terrain was
  dropped to it, so the walk to a Mite Sentry 4.3 m overhead ended beneath it; a goal on a floor
  now keeps its height and the path takes the ramp up. A spell or arrow with no clear shot walks to
  the nearest reachable place it clears from. Scn Taper's stall seconds 87 and 77 (runs F, J) -> 0
  (run K, where it reached the roof).

- **Walked into a pocket at the Academy spawn** (6556d76): a steering reset a tenth of a second
  into a routed walk to Jonathan aimed straight at him until the next line check, into a pocket
  among the props no path leaves. Before: stuck 53-55 s (runs A, B, D), 388 s and never out (E),
  the Jonathan shortcut never working. After: route kept, out by Jonathan at 5.9 s, no stall (F, G).

- **Stood in a shop aiming through its wall** (733ed66): the walk out of a building aimed at the
  threshold, within `ARRIVE` of a character just inside, so it ended at once and the next walk
  (112 m to Shopkeeper Renald) leaned on the wall; the block's graph finds no path that far from
  indoors. Now it walks out to a spot 5-10 m outside on the way it came in. Scn Mage: 241 s stalled
  in both runs before; 0 s after (run A: out of Boddry's shop in 2 s); data test replays Cindrue's.
- **A caster in hand and no attack spell: no fight at all** (35e30cf). Scn Blade held a looted
  Staff for whole runs, "no attack spells known" 2,397 times, 0 blows, with a Battle Axe in its
  pack. Now it takes up the axe: in hand from 2.6 s, 25 blows (187 points), 3 kills (run B).
- **A purchase refused in silence for three minutes** (e974d12): sized at 34 a taper where the
  server charges a stack as one item (150 at Magus Guthima's 1.55 cost 5,115 against 5,109), and
  the silent refusal asked again every tick. Silent refusals 1,203 and 1,212 -> 2 (the bow's arrow
  wield, not a buy); 677 tapers and a scarab bought at the first ask, then 32 casts (run C).

- **No way back to town from a far ground** (3b64241): the planner believed a leg on foot only to
  1200 m, so a ground reached through a one-way portal had no way home. Scn Taper, out of tapers
  2.5 km out: before, 17 "no way to" refusals; after, none, a 656 s walk planned, at Magus
  Guthima in 129 s. (Its purchase then failed: see the stack price below; "150 bought" in the
  first report counted asks, not arrivals.)
- **Standing about a quiet ground** (f37ffc3): longest idle 237 s -> 24-63 s.
