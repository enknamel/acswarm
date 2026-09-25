# Bugs

Worked top down. Each entry says how it shows (a telemetry or log line, a `tools/scenarios.sh`
check) and is closed by a commit whose scenario run passes. Add new reports at the bottom of their
priority; a live report starts from the telemetry (`tools/telemetry.py`, `--marks` for F9 presses).

## 1. Stops the character playing

- **The Jonathan shortcut out of the Academy fails**: "waiting for the Jonathan to turn up", then
  "nothing to give", and the character does the whole tutorial instead (a war mage left in 6.5
  min, scenarios run 2). Blargerton took 58 s to walk 9 m towards Boddry the Chancy, and props
  near local (14.88, -26.79) in 0x860201AD wedged walks there while `line_blocked` called the
  line clear. Shows as: `no stall over 30 s` fails with a cell in 0x8602 or near a counter.
- **Stands at a counter it has not reached.** Scn Mage stood 240 s 4 m from Shopkeeper Renald the
  Elder (0xA9B40117, 32599.8 34689.8): straight line, aim set, not wedged, no detour, in both runs
  of 2026-09-24. Shows as: `no stall over 30 s` fails with "going to <vendor> (150 m)".
- **A caster in hand and no attack spells: no fight at all.** Scn Blade held a looted Staff, knew no
  attack spells ("no attack spells known" from its first second) and never took up a weapon; 0
  blows in both runs. (The fixture's Battle Axe went to a counter: the Check profile sells anything
  worth 25, so the scenario profile must keep the fixtures' weapons.)
- **A purchase hangs at the counter** (fix under test). The count was sized at the per-item price
  (34 a taper at Guthima's 1.55), but a stack is one item to the server: 150 cost 5,115 against a
  purse of 5,109, refused with no words (Vendor.cs:546-551), and asked again every tick until the
  3-minute timeout: 1,203 and 1,212 silent refusals in two runs, 0 tapers arrived.
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

- **No way back to town from a far ground** (3b64241): the planner believed a leg on foot only to
  1200 m, so a ground reached through a one-way portal had no way home. Scn Taper, out of tapers
  2.5 km out: before, 17 "no way to" refusals; after, none, a 656 s walk planned, at Magus
  Guthima in 129 s. (Its purchase then failed: see the stack price below; "150 bought" in the
  first report counted asks, not arrivals.)
- **Standing about a quiet ground** (f37ffc3): longest idle 237 s -> 24-63 s.
