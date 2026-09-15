# ac-client code map

One `Client` per game session: connection, `World`, body, the manual actions a UI or script calls,
and autoplay. Bare file names are under `src/`; ranges say where a system lives today (grep the fn
when they drift). `tests/code_map.rs` checks the paths, entry fns, steps and modules named here.

## Tick path

`Client::tick` (`lib.rs:873-1400`), once a frame per session:
1. drain the socket and `World::apply` each message; server chat goes to `chat_message()`
   (`lib.rs:1798-2147`), which hands refusals in words to `hear_refusal()` (`refused.rs`)
2. manual-play timers: `tick_combat()`, `tick_loot()`, `tick_store()`, `tick_appraise()`
3. `tick_autoplay()` (`autoplay.rs:4273-4368`), then `tick_retag()` and `save_ledger()`
4. `tick_player()` moves the body (`player.rs`) and runs `tick_visit()` (`visit.rs`)

`tick_autoplay()`, in order:
- with the team on, even with autoplay off: `autoplay_accept_invites()`, and `autoplay_fellowship()`
  for a leader played by hand; always `autoplay_close_unwanted_window()`
- returns if autoplay is off or no character is in the world
- `autoplay_watch_the_ground()`; then `autoplay_dodge()`, `autoplay_survive()`, `autoplay_recover()`,
  `autoplay_academy()` called directly, and any that acts ends the tick
- every `steps::HOUSEKEEPING` row; `steps::reflexes()` in table order; goals from `steps::weigh()`,
  best first. The first step that acts sets `Autoplay.step` and ends the tick.
- The four direct calls run again as the first reflex rows, in another order (survive before dodge).

## Steps

`steps::STEPS` (`steps.rs:312-520`). A goal with no scorer is worth `(STEPS.len() - place) * 10`,
so adding, removing or moving a row re-scores every goal below it against `LOOT_AT_REST` (45) and
`WALK_TO_A_FIGHT` (40).

| # | step | layer | worth | runs |
|---|---|---|---|---|
| 1 | survive | reflex | - | `autoplay_survive` |
| 2 | dodge | reflex | - | `autoplay_dodge` |
| 3 | recover | reflex | - | `autoplay_recover` |
| 4 | academy | reflex | - | `autoplay_academy` |
| 5 | urgent buffs | reflex | - | `autoplay_buff` (urgent) |
| 6 | vitals | reflex | - | `autoplay_vitals` |
| 7 | loot | goal | `worth_looting` | `autoplay_loot` |
| 8 | catch up | goal | place | `autoplay_follow` (urgent) |
| 9 | team | goal | place | `autoplay_team` |
| 10 | salvage | goal | place | `autoplay_salvage` |
| 11 | summon | goal | `worth_fighting` | `autoplay_summon` |
| 12 | fight | goal | `worth_fighting` | `autoplay_fight` |
| 13 | keep to the area | goal | place | `autoplay_keep_to_area` |
| 14 | buffs | goal | place | `autoplay_buff` |
| 15 | follow | goal | place | `autoplay_follow` |
| 16 | resume the journey | goal | place | `autoplay_resume_journey` |
| 17 | tidy | goal | never weighed | nothing: the row only holds the scores below it |
| 18 | explore | goal | place | `autoplay_explore` |
| 19 | grow | goal | place | `autoplay_grow` |

Housekeeping, every tick and never claiming it: `autoplay_pending_wield`, `autoplay_shield`,
`autoplay_rearm`, `autoplay_stock`, `autoplay_claim_pet_kills`, `autoplay_tidy`, `autoplay_spend_xp`.

## Systems

Tests: `cargo test -p ac-client WORD` with the tests column's word. Log target: `ac_client::` plus
the column (the module path; see the root map for `RUST_LOG`).

| system | entry fns | files | state | tests | log target | term |
|---|---|---|---|---|---|---|
| steps and tick | `tick_autoplay`, `weigh`, `reflexes` | `autoplay.rs:4273-4368`, `steps.rs` | `Autoplay.step`, `Autoplay.doing`, `Autoplay.status` | steps:: | autoplay | step |
| target choice | `pick_target`, `would_fight`, `ordered_target`, `a_fight_in_sight` | `autoplay.rs:7585-7677`, `autoplay.rs:8712-8751` | `Client.attack_target`, `Autoplay.config.fight` | target | autoplay | target |
| melee | `autoplay_fight`, `autoplay_fight_as`, `give_up_target`, `stalled_on` | `autoplay.rs:7678-7870`, `autoplay.rs:8293-8532` | `Client.attack_target`, `Autoplay.engaged`, `Autoplay.given_up` | target | autoplay | fight |
| spells in a fight | `autoplay_fight_with_spells`, `autoplay_soften`, `autoplay_make_vulnerable` | `autoplay.rs:7871-8259`, `aim.rs` | `Autoplay.casting_at`, `Autoplay.softening`, `Autoplay.vulned` | spell | autoplay | fight |
| critter | `critter`, `a_critter`, `ask_about_strangers` | `autoplay.rs:2194-2229`, `autoplay.rs:2246-2410`, `autoplay.rs:7341-7508` | `Fight.skip_critters`, `Client.appraisals` | critter | autoplay | critter |
| road | `on_its_way`, `passing_by`, `road_fight_over` | `growth.rs:1780-1846`, `autoplay.rs:2230-2245`, `autoplay.rs:7509-7584` | `Fight.walk_past_on_the_way` | road | growth | road |
| looting | `autoplay_loot`, `corpse_for_us`, `walk_to_corpse`, `worth_looting` | `autoplay.rs:4794-5104`, `autoplay.rs:5180-5572`, `autoplay.rs:7128-7340`, `crates/ac-loot/src/run.rs` | `Autoplay.corpse`, `Autoplay.looted`, `Autoplay.walking_to`, `Autoplay.loot_run` | corpse | autoplay | corpse |
| corpse turns and shuts | `whose_turn`, `judge_shut`, `take_in_shuts`, `claim_on` | `autoplay.rs:1592-1919`, `autoplay.rs:2976-3700`, `autoplay.rs:6194-6364` | `Autoplay.standing_by`, `Autoplay.shut_by`, `Autoplay.whose` | body | autoplay | turn, shut |
| ledger | `tick_retag`, `autoplay_tag_arrivals`, `judge_loot`, `loot_action` | `autoplay.rs:2430-2529`, `autoplay.rs:4536-4793`, `autoplay.rs:6365-6436`, `crates/ac-loot/src/ledger.rs` | `Autoplay.ledger`, `Autoplay.pending_tags` | (`-p ac-loot ledger`) | autoplay | tag |
| salvage | `autoplay_salvage`, `salvage_tagged`, `next_salvage_batch` | `autoplay.rs:1920-1974`, `autoplay.rs:6437-6670` | `Autoplay.salvaging`, `Autoplay.last_salvage` | salvage | autoplay | salvage |
| pack tidy | `autoplay_tidy`, `settle_pour`, `pour_next`, `compress` | `autoplay.rs:4369-4464`, `growth.rs:3783-3977` | `Autoplay.pour`, `Autoplay.tidy_looked`, `growth::State.wont_merge` | pour | autoplay | tidy, pour |
| weapons | `autoplay_pending_wield`, `arm_for`, `autoplay_shield`, `autoplay_rearm` | `autoplay.rs:5105-5179`, `autoplay.rs:5573-5830`, `autoplay.rs:9896-9938`, `crates/ac-loot/src/weapons.rs` | `Autoplay.pending_wield`, `Autoplay.wield_refused`, `Autoplay.armed_for` | wield | autoplay | wield |
| ammo | `autoplay_stock`, `autoplay_craft_ammo`, `ready_ammo`, `choose_recipe` | `autoplay.rs:2411-2429`, `autoplay.rs:6717-6865`, `autoplay.rs:8752-8794`, `growth.rs:2660-2692` | `Autoplay.crafting`, `Autoplay.wanted_ammo` | arrow | autoplay | ammo |
| healing | `autoplay_survive`, `choose_heal`, `self_heals`, `autoplay_vitals` | `autoplay.rs:1478-1577`, `autoplay.rs:3907-4154`, `autoplay.rs:4465-4545` | `Autoplay.config.survive`, `Autoplay.last_heal`, `Autoplay.last_vital` | heal | autoplay | heal, vitals |
| buffs | `autoplay_buff`, `wanted_buffs`, `due_buff`, `wanted` | `autoplay.rs:583-662`, `autoplay.rs:4155-4272`, `autoplay.rs:9633-9895`, `autoplay.rs:9939-10008`, `buffs.rs` | `Autoplay.config.buffs`, `Autoplay.last_buff`, `Autoplay.item_buffs` | buff | autoplay | buff |
| recruiting | `autoplay_fellowship`, `autoplay_accept_invites`, `next_invitee`, `hear_recruit_refusal` | `autoplay.rs:1055-1156`, `autoplay.rs:3789-3882`, `autoplay.rs:8795-9073` | `Autoplay.recruited`, `Autoplay.held_off`, `Team.fellowship` | invit | autoplay | recruit |
| team board | `autoplay_team`, `leader_mate`, `worst_hurt`, `rival_leader` | `autoplay.rs:910-1477`, `autoplay.rs:1975-2149`, `autoplay.rs:9518-9632`, `crates/ac-plugin/src/team.rs` | `Autoplay.team` (`TeamView.mates`), `Config.team` | leader | autoplay | mate |
| fellowship planner | `plan_for_team`, `assign_targets`, `deal_bodies`, `stragglers`, `take_orders` | `plan.rs`, `autoplay.rs:3585-3658`, `autoplay.rs:5959-6193` | `Autoplay.planner`, `Autoplay.orders` | plan:: | autoplay | plan, order |
| follow | `autoplay_follow`, `followed_leader`, `follow_break` | `autoplay.rs:1157-1243`, `autoplay.rs:9074-9183` | `Autoplay.follow_trip`, `Team.follow`, `Client.follow` | follow | autoplay | follow |
| quartermaster | `autoplay_quartermaster`, `decide`, `quartermaster`, `hand_out` | `autoplay.rs:9184-9517`, `logistics.rs`, `growth.rs:2978-3063`, `growth.rs:3128-3431` | `growth::State.mode`, `Team.restock` | quartermaster | autoplay | quartermaster |
| town run | `grow_town_run`, `start_town_run`, `grow_run_step`, `grow_run_next`, `pick_vendor` | `growth.rs:633-866`, `growth.rs:3607-3736`, `growth.rs:4225-5548`, `shopping.rs`, `crates/ac-vendor/src/run.rs` | `growth::State.run`, `growth::State.shop` | counter | growth | town_run, counter |
| supplies and sale | `grow_needs_with`, `supplies`, `sell_policy`, `offers_for_sale`, `burns` | `growth.rs:1006-1690`, `growth.rs:2798-2977`, `growth.rs:3128-3606`, `growth.rs:3737-3782`, `crates/ac-loot/src/sale.rs` | `growth::State.needs`, `Growth.ammo_keep` | counter | growth | need, sale |
| XP spending | `autoplay_spend_xp`, `grow_spend_xp`, `raise_offers`, `batch_raise` | `growth.rs:355-632`, `growth.rs:1983-2230`, `advance.rs` | `growth::State.pending`, `growth::State.sulking` | experience | growth | raise |
| hunting ground and area | `autoplay_grow`, `grow_hunt`, `autoplay_watch_the_ground`, `autoplay_keep_to_area` | `growth.rs:1847-1941`, `growth.rs:2231-2659`, `hunt.rs` | `growth::State.bound`, `growth::State.quiet_since`, `Fight.area` | ground | growth | ground, area |
| travel | `travel_to`, `travel_about`, `plan_trip`, `autoplay_resume_journey` | `travel.rs`, `pathfinder.rs`, `recalls.rs`, `autoplay.rs:8260-8292`, `crates/ac-nav/src/steering.rs` | `Client.travel`, `Client.steering`, `Autoplay.resume_trip` | travel | travel | journey |
| visits | `visit_landmark`, `tick_visit`, `visit_person` | `visit.rs` | `Client.visits` | visit | visit | visit |
| dungeon explore | `autoplay_explore`, `aim_on_floor`, `past_the_door` | `explore.rs`, `crates/ac-nav/src/explore.rs` | `Autoplay.room_bound`, `Autoplay.rooms_seen` | explore | explore | explore |
| dodge | `autoplay_dodge`, `autoplay_approach`, `sidestep`, `threat` | `dodge.rs`, `aim.rs` | `Client.dodge`, `Client.dodge_to` | dodge | dodge | dodge |
| recovery | `autoplay_recover`, `recovery_view`, `is_dead` | `recovery.rs` | `Autoplay.recovery` | recovery:: | recovery | recovery |
| academy | `autoplay_academy`, `academy_open_doors` | `academy.rs`, `tests/academy_route.rs` | `Autoplay.academy`, `Autoplay.academy_corpse`, `Autoplay.academy_doors`, `Autoplay.academy_armed` | academy | academy | academy |
| summoning | `autoplay_summon`, `autoplay_claim_pet_kills`, `hear_summoning` | `summoning.rs`, `autoplay.rs:8639-8711` | `Autoplay.summoning` | summon | summoning | summon, pet |
| refusals | `hear_refusal`, `refused`, `answer` | `refused.rs`, `crates/ac-agent/src/refusals.rs` | the waiting system's own wait, e.g. `Autoplay.shelved` | refus | refused | refusal |
| server chat | `chat_message`, `hear_arrival`, `hear_spell_attack` | `lib.rs:1798-2147`, `autoplay.rs:475-557`, `autoplay.rs:8533-8638` | `Autoplay.hit_by` | arriv | (crate root) | heard |

## Glossary

| term | means | not |
|---|---|---|
| corpse | a dead creature's container that autoplay opens and empties | body, remains |
| town_run | a trip to sell loot and restock at counters | errand, sale run, trip |
| counter | a vendor's shop the town run stands at | shop, store |
| road | a walk on the way somewhere (a ground, a counter, a script's goal); on it only what attacks is fought | way |
| ground | the hunting spot growth chose; walking about it is not a road | spot, field |
| area | a hunting area a player drew on the map (`HuntArea`) | zone |
| journey | a planned trip of walks, portals and recalls (`Travel`) | route, travel plan |
| mate | another character on the team board (`Mate`), in the fellowship or not | teammate, member |
| fellow | a member of the server's fellowship | - |
| recruit | invite a mate into the fellowship | invite, ask |
| heal | restore health, by kit or spell | cure |
| vitals | stamina and mana kept up | - |
| buff | a beneficial enchantment kept up | - |
| critter | a creature the critter rule walks past | - |
| target | the creature being fought | enemy, foe |
| tag | the loot profile's `LootAction` for an item, kept in the ledger | verdict, mark |
| step | a `steps::STEPS` row, reflex or goal; housekeeping never claims a tick | chore |
| refusal | the server's no, by weenie error or in words | error, rejection |
| plan, order | the leader's plan for the party, and one character's part in it | - |

## Domain rules

- Critter rule, `critter()` applied by `a_critter()`: a creature is walked past only when it is
  quiet (has not attacked, walks at nobody of ours, fights nobody), the creature table does not
  mark it aggressive, its level is at most half the character's, and its health is at most 5
  (`CRITTER_HEALTH`; 30 for a creature the table does not know). Unknown level or health: appraise
  and leave it alone until the answer. Never loosen it to "passive": ACE's newbie spawns are all passive.
- Collision rule, `add_model()` in crates/ac-scene/src/collision.rs: a placed model collides by its
  parts' physics polygons, else its Setup's cylinder-spheres (else spheres), else not at all; never
  by drawn polygons. Server-placed objects never stall a walk; `cylinders()` only steers round them.
- Doors are walked through: `in_the_way()` never counts a door, and explore aims past the sill
  (`past_the_door()`).
- Peas are components in the data: `spell_component_ids()` maps trade peas to spell-component ids,
  so never key restock or sale on "is a component". Restock follows what cast spells burn
  (`component_targets()`, `burns()`), and a pea the profile tagged Sell is sold (`offer_to_vendor()`).
- The loot profile decides: `fate()` puts a tag ahead of every guard (restock, burns, keep names);
  only the server's own refusal overrides it.
- Refusals table: `refused()` quotes ACE's words with `File.cs:line` and `answer()` sets the wait;
  `hear_refusal()` hands each to the waiting system. A new one is a row, a test in the exact words,
  an `answer()` arm and a hand-off; never a `strip_prefix` in the system that noticed.

## Other modules

| module | what |
|---|---|
| `lib.rs` | `Client`, `Event`, `Config`; connect, tick, manual actions (combat, items, trade, fellowship, housing, allegiance, chat). Re-exports `ac_agent::{did, pack, refusals, room, weenie_errors}`, `route` (`ac_nav::steering`), `errand` (`ac_vendor::errand`) |
| `player.rs` | body physics: walking, floors, ledges, falls, jumps, cell tracking |
| `creation.rs` | character creation rules and create-if-missing |
| `magic.rs` | spellbook, spell bars, enchantments, components, cast checks |
| `items.rs` | appraisal requests and answers; the item vocabulary is `ac_loot::items` |
| `profile.rs`, `weapons.rs` | shims re-exporting `ac_loot::profile` and `ac_loot::weapons` |
| `holdings.rs` | every character's inventory snapshots, for the Items window |
| `reconnect.rs` | whether and when a dropped session logs back in |
| `logoff.rs` | logging every session off on exit |
| `options.rs` | character option bits |
| `emotes.rs` | soul emotes |
| `daytime.rs` | Dereth's time of day |
| `augmentations.rs` | augmentation gems |
| `advance.rs` | experience costs and raise messages |
| `recalls.rs` | recall spells the journey planner can use |
| `pathfinder.rs` | neighbourhood route planning off the frame thread |
| `shopping.rs` | reads the pack into `ac_vendor`'s snapshot and carries out its act |
| `aim.rs` | whether a projectile clears walls and hills on the way to its target |
| `logistics.rs` | the party's mode (hunting or restocking) and money sharing, the same on every session |
| `buffs.rs` | which buffs a character should wear, from its skills and spells |
| `hunt.rs` | hunting areas drawn on the map |

Unit tests sit in each file's `mod tests`; tests that need `AC_DATA_DIR` (`tests/` and many unit
tests) return early without it.
