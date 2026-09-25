# Bugs

Worked top down. Each entry says how it shows (a telemetry or log line, a `tools/scenarios.sh`
check) and is closed by a commit whose scenario run passes. Add new reports at the bottom of their
priority; a live report starts from the telemetry (`tools/telemetry.py`, `--marks` for F9 presses).

## 1. Stops the character playing

- **Walked into a pocket at the Academy spawn** (sometimes). In scenario runs A and B the new
  character went straight from the spawn (12.3, -28.5) to (14.5, -25.5) in 0x860201AD and stood
  53 s: from there the block's graph finds no path anywhere, so the steering leaned on the prop;
  Jonathan never answered a use from 10 m, and the full tutorial got it out instead. From the
  spawn the graph routes round (16.5, -28.5), (21.0, -22.5), and a lone character did that and
  left by Jonathan in 4 s, so what differs in a busy process's first frames is not yet known: the
  "steer" records (7eb71fe) catch the first frame next time.
- **Leaning for a minute on what the line test calls clear.** Scn Mage at the Mosswart ground
  (0xBAAD0017, 35773.4 33364.8) "getting Gotrok Lithos in sight" aimed straight at a spot 18 m off
  (follow, block 0xBAAD0000, straight line chosen) and did not move for 61 s, then 20 s more
  walking to its corpse (run E). Nothing gives up a target the body cannot get to. Shows as: a
  stall whose walk detail says straight line, not wedged, outdoors.
- **The two town-run planners disagree.** The first plan said "Boddry the Chancy it is: nowhere
  sells what is wanted", the next-stop plan found Cindrue with "2 of 2 on the shelf", so the run
  walked to the wrong counter. Shows as: two stops for one need in the status lines.

## 2. Plays badly

- **The counter spends every coin on buy-list lines** (470 tapers, nothing left for healing kits;
  scenarios run 2: Scn Seller, a soldier, bought 336 tapers with its 10,000 and kept 20). Shows as:
  `coin` near 0 in the samples after a town run.
- **Experience goes to weapon skills with no weapon to use them** (Scn Blade raised Heavy Weapons
  to 149 holding only the Academy's training bow). Found because the fixtures had no weapons; a
  character that has lost its weapon plays the same way.
- **Tapers counted as two needs** (Named and Component). Shows as: "short of Prismatic Taper, Lead
  Scarab, Prismatic Taper" in the status line.
- **The Academy tutorial fallback cannot finish for a bow soldier**: no damage to the Olthoi,
  then out of arrows.

## 3. Tooling

- **Launcher "Launch headless" is not headless** (it only adds `--mute`) and passes the password
  in argv (`-v`), visible to `ps`.
- **`--create` with no template makes a crippled character** (60 attribute points, 18 HP at
  level 5); use a template, as `tools/scenarios.sh` does.

## Fixed

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
