# Autoplay conflicts: plan

Status: approved 2026-11-09 (all seven phases; urgent top-ups keep running while a journey walks;
docs/movement-plan.md unchanged unless the post-phase-3 count calls for it).

## The problem in one paragraph

Each autoplay system decides on its own, every tick, from its own picture of the world. Some of what one system sends to the server is invisible to the rest: a recall cast, a gem use, the counter Use. Some of what one system writes reaches the others late: the quest board, the needs cache, a purchase. So two systems act on the same server slot or the same journey, or one acts on a picture the other has already changed. Today's nine incidents were each fixed by a new predicate for one row to read. This plan fixes the few shared places that all those cases pass through instead.

## Where the conflicts come from

**A. Hops and some uses never take the server's busy slot.** The recall cast stamps only `travel.recall_cast` (crates/ac-client/src/travel.rs:1283-1288). The gem uses (travel.rs:813, 888) and the summoned-portal USE (travel.rs:823-824) mark nothing. `cast_in_flight`/`casting` read only `cast_sent` (autoplay/cast.rs:36-64), so buffs, vitals, mana stones and wields send over a hop. The 0x4EB "in the air" hold never starts for a recall (cast.rs:79). Gems guard themselves with a guessed 6 s window (gem_room.rs:24-26, 141-148). Explains incidents 1 (stones over gem and counter acts) and 4, and the 0x1D after every recall in four logs.

**B. A hop already sent is not a commitment.** `set_off` plans from where the body stands and overwrites the trip (travel.rs:281-299). It never asks whether a recall is still in the air. A gem step returns from its branch (travel.rs:1103-1180) before the "carried off" replan (1185-1202), so it uses the gem wherever an earlier recall dropped the body. Explains incident 4, and 6 (state from one journey leaking into the next).

**C. Reflexes win every slot that frees up.** Reflexes run before goals (autoplay/mod.rs:679-689). Between fights the urgent pass also tops up, "journey or not" (vitals/buffs.rs:455-466). The only yield signal, `hands_wanted`, is written only at pursuit/fetch.rs:352 and read only at buffs.rs:459-463. The counter hold does not cover walking up to the counter: `counter_asked` returns None for `Phase::Going` (town_run/counter.rs:89). Explains incident 9 (31-55 s counter and gem waits).

**D. Handoffs read state at different ages.** `refresh_quests` runs after `tick_autoplay` (session/apply.rs:575 vs 583), every 2 s. Its fingerprint leaves out the pack; only a gem arriving triggers it (quest_log.rs:853-866). `needs_seen` is a 5 s cache whose only writer is needs.rs:193; nothing clears it. grow treats a pursuit Buy step as no pursuit (pursuit.rs:976 `_ => None`). A quest buy is not exempt from the 300 s between-runs wait (town_run/mod.rs:405 vs 431-440). A want that drops out of the snapshot counts as the purchase arriving (crates/ac-vendor/src/run.rs:191-197). Explains incidents 2, 3 and 4.

**E. "No way" answers from a passing state are kept, and the kept ones are not read.** Walks read the kept refusals (body/going.rs:154-163). Journeys do not (travel.rs:267-277), so a refused journey is planned again every tick: 3583 "travel: cancelled" lines in 2 min in the 51154 log. Only a trap crossing stops planning from mid-air (going.rs:223). One refused portal becomes a 20-minute ground skip (hunt.rs:270). The pursuit reads that skip as "want the used gem again". Explains incident 5 and the walk-away from Ithaenc right after the gem, seen three times today.

**F. A commitment carries no weight in the weighing.** A run under way is worth 15 (autoplay/steps.rs:304), below five other rows, so each higher row has to veto itself (for example explore.rs:185-196). Explains incident 8.

## Principles

- Fix the shared place every case passes through: the busy slot, `set_off`, the planning entrance, the board refresh. Never add another "X is under way" predicate.
- Everything autoplay sends that the server answers takes the one busy slot. No private busy clocks.
- A hop sent to the server is not replaced until it lands or is refused.
- A goal waiting on the slot gets it next. Only survive, dodge and a buff that really lapses go ahead of it.
- A plan asked while the body is airborne or mid-hop gets a wait, never a kept refusal.
- Refresh on change, before the readers run. Timers stay only as backstops.
- Each phase: an offline test that fails on main, the gates, `tools/scenarios.sh`, and a live A/B counted from the logs.

## Phases

**1. Hops take the busy slot (S).**
- Changes:
  - The recall's `CastCheck::Ok` arm (travel.rs:1283-1288), both gem uses (813, 888) and the portal USE (823-824) set `autoplay.cast_sent`. The portal USE also checks `is_gem_held` first; today it is sent with no check.
  - Add `Autoplay.hop_until`, read by `casting()` (cast.rs:43-64), so stones, refills, wields (`server_busy`, actions/combat.rs:402-404) and buffs wait until the body lands.
  - The teleport (apply.rs:288), the arrival and carried-off checks (travel.rs:1109-1116, 1185-1202) and `recall_error` clear it. A 0x1D answered to the recall recasts it once the slot is free instead of waiting out RECALL_GIVE_UP.
  - Delete gem_room's guessed RECALL_LANDS.
- Removes: the 0x1D after a recall, 0x4EB arriving with no hold, a stone 100 ms after a gem use, and a recall lost to a busy answer (49547 19:17:03-18). Prevents incident 1's stone-over-hop overlaps, part of 4, and the 0x1D half of 9.
- Test: tick with a recall just sent and an urgent buff and a stone due. Assert nothing is sent and `wield_must_wait` is true. On main all of them send.
- Risk: low. First measure in the logs whether the recall's UseDone(0) comes before the teleport; `hop_until` covers that gap.

**2. A hop in the air is not replaced (S).**
- Changes:
  - `set_off`, and `ask_slot` for every owner that is not the journey, answer `Did::waiting` while `hop_until` runs. Nothing is cached.
  - `interrupt_travel` and `settle_journey`'s put-down (travel.rs:549-561, 645-650) keep the step state while a hop is in the air.
  - `step_goal` checks "carried off" before the gem branch.
- Removes: the journey replaced while its recall flies (53261 19:46:43 and 19:50:02), and a gem used from the recall's landing spot. Prevents incident 4.
- Test: send grow's recall, then ask a journey from the town run. Assert Waiting, an unchanged goal, and a plan from the landing once the teleport arrives.
- Risk: low; RECALL_GIVE_UP is the backstop.

**3. The goal waiting on the slot gets it next (S).**
- Changes:
  - Move `hands_busy` (fetch.rs:348-354) into `autoplay/cast.rs`. Call it from the counter walk-up (counter.rs:255), the recall wait (travel.rs:1267), the gem's `is_gem_held` wait, and the giver `give` (calls.rs:694, which has no busy check today; ACE Player_Inventory.cs:3201-3202).
  - Add the existing `hands_wanted` read to vitals (heal.rs:270-290), mana stones (mana_stones.rs:68) and the essence refill (summoning.rs:373).
- Removes: 35 "waiting for the cast to land" lines today, recall and gem steps starved 30-55 s, and stones between counter buys. Prevents incident 9's starvation.
- Test: the run in `Phase::Going` at the counter, a top-up due, the slot just freed. Assert the Use goes out. On main the buff wins.
- Risk: low; top-ups are delayed by at most one act.

**4. The pursuit's Buy is handed over fresh (M).**
- Changes:
  - Run `watch_quest_answer`, `settle_server_quests` and `refresh_quests` before `tick_autoplay` (apply.rs:575-583).
  - Add the carried counts of pursuit gems and quest items to `BoardFor`.
  - `keep_quest_section` clears `needs_seen` when the buy lines change.
  - `PursuitStep::Buy` returns `Some(true)` ("waiting for the town run") while a run can start. It falls through to grow's ground pick only if the run is held back for a reason other than time.
  - A quest buy skips the between-runs wait, the futile wait and the urgency gate, as `pursuit_call` does.
  - The Quests-section buy lines count only once `quests_synced` is true and a board has been built this session.
- Removes: the 335 ms stale buy line (incident 2), the town run deciding at login (3), and grow's recall while the buy waited on the 5 s cache (4).
- Tests:
  - The pursuit wants a gem: the run sets off that tick and grow sets off nothing.
  - A gem used: the section loses its buy line that tick.
  - Login: no run before the first board is built.
  - A run held back for want of coin: grow still picks a ground.
- Risk: medium (frame order). Check that `read_quest_wants` (retag.rs:226-235) still sees a consistent board, and measure the cost with `tick_meter`.

**5. A purchase is done when it reaches the pack (S).**
- Changes: the Snapshot carries the pack count of each want's wcid. `run.rs:191-197` clears `bought` only when that count rises, or after PURCHASE_LANDS.
- Removes: "town run done" 0.7 s before the Invitation arrived (53261 19:50:01-02). Prevents incident 4's trigger.
- Test (ac-vendor): the want leaves the snapshot and the count is unchanged, so the Run is not done.
- Risk: low.

**6. Refusals are read by journeys and never written from the air (S/M).**
- Changes:
  - `journey_to_goal`/`set_off_or_fly` check `Client.refusals` the same way `ask_slot` does.
  - Widen going.rs:223 to any airborne body, and add the same check at the top of `set_off`: answer a wait, so nothing reaches refusals, `steering.refused`, `growth.skip` or `rooms_shut`. This is a rule about when the planner may be asked, written into docs/movement-plan.md first.
  - `fly_journey` clears only its own owner's refusals (travel/flown.rs:74).
  - `end_trip` clears `fly_to` and `about_the_ground`.
- Removes: the refusal storm, nine grounds marked in 2 s, and a leftover flight marker. Prevents incidents 5 and 6.
- Tests: 60 ticks with the body unmoved plan the journey once. An airborne body leaves `skip` and the refusals empty. Plus one whole-loop test at the trap plate.
- Risk: low to medium. Confirm `is_airborne` does not read true while standing on a slope.

**7. A fight's urgent buffs leave the heal its mana (S).**
- Changes: while `in_a_fight()`, the reserve is the full `keep_mana` (buffs.rs:494-495), and the queue widening (449-452) does not pull more banes in.
- Removes: the 52019 death (four banes, mana 0, 33 "cannot heal" lines).
- Test: fighting at 30% mana with banes lapsed, no buff is cast.
- Risk: low.

## Not doing (and why)

- **Commitment weight in `steps::weigh` (root cause F, incident 8).** It changes who wins in many situations and carries the most risk. The veto at explore.rs:185-196 already covers incident 8.
- **Keeping the journey's stretch while a cast is held (the "held / acts again" log churn).** That changes the movement plan's documented rule. Re-count it after phases 1 and 3 first.
- **Tying each UseDone to the request it answers.** UseDone carries no id. Once phase 1 puts every act in the one slot, only one is in flight at a time, so the answer is credited correctly.
- **A general news struct, a resource-lease framework or a scheduler.** Too heavy for the evidence. Phases 4-6 cover the freshness gaps directly.
- **Splitting `growth.skip` into kinds, and explore's room memory.** Phases 4 and 6 remove what fed them. Revisit only if the A/B still shows the post-gem walk-away.
- **Appraisal age, the trade-note split storm, a grace period after the player lets go of the keys, and the profile overwritten across processes.** Separate, low-severity issues; each goes to docs/bugs.md.
- **Incident 7.** Already fixed in ebb3582; phase 2 also stops planning mid-hop.
