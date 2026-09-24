# Bugs

Worked top down. Each entry says how it shows (a telemetry or log line, a `tools/scenarios.sh`
check) and is closed by a commit whose scenario run passes. Add new reports at the bottom of their
priority; a live report starts from the telemetry (`tools/telemetry.py`, `--marks` for F9 presses).

## 1. Stops the character playing

- **Standing about a quiet ground.** The ground's wait before each of its three looks about was the
  whole `idle_before_move` (60 s), so a cleared ground held a character still for four minutes
  (scenarios run 2: all five fixtures 237 s "waiting" at 0xADAF0020). Fix under test: looking
  about waits 10 s, the setting is for leaving. Shows as: `not idle over 90 s` fails.
- **The Jonathan shortcut out of the Academy fails**: "waiting for the Jonathan to turn up", then
  "nothing to give", and the character does the whole tutorial instead (a war mage left in 6.5
  min, scenarios run 2). Blargerton took 58 s to walk 9 m towards Boddry the Chancy, and props
  near local (14.88, -26.79) in 0x860201AD wedged walks there while `line_blocked` called the
  line clear. Shows as: `no stall over 30 s` fails with a cell in 0x8602 or near a counter.
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
