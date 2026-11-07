# ac-client code map

One `Client` per game session: connection, `World`, body, the manual actions a UI or script calls,
and autoplay. File names are under `src/`; grep an entry fn for where its system sits in one.
`tests/code_map.rs` checks the paths, entry fns, steps and modules named here, and that each
system's entry fns are written in the files its own row names.

## Tick path

`Client::tick` (`session/apply.rs`), once a frame per session:
1. drain the socket and `World::apply` each message; server chat goes to `chat_message()`
   (`session/chat.rs`), which hands refusals in words to `hear_refusal()` (`refused.rs`)
2. manual-play timers: `tick_combat()`, `tick_loot()`, `tick_store()`, `tick_appraise()`
3. `tick_autoplay()` (`autoplay/mod.rs`), then `tick_retag()` (`autoplay/ledger/retag.rs`) and
   `save_ledger()` (`autoplay/ledger/store.rs`)
4. `tick_player()` (`body/player_tick.rs`) runs `tick_visit()` (`visit.rs`), walks the journey's
   stretch (`walk_journey()`), steers the slot's walk along its planned route (`body/standing.rs`,
   `ac_nav::Steering`) and moves the body (`player.rs`)

`tick_autoplay()`, in order:
- once no leader is followed, whatever switched it off: `stop_following()`, which lets go of only
  the walk and the journey following planted; on the tick one is, its own plans for the road go
- with the team on, even with autoplay off: `autoplay_accept_invites()`, and `autoplay_fellowship()`
  for a leader played by hand; always `autoplay_close_unwanted_window()`
- with autoplay off, `autoplay_by_hand()`: the team rules a player keeps, and then it returns, as
  it does with no character in the world
- `autoplay_watch_the_ground()`, whose clocks are wound before any step can claim the tick
- every `steps::HOUSEKEEPING` row; `steps::reflexes()` in table order, each once and only from the
  table; goals from `steps::weigh()`, best first. The first step that acts sets `Autoplay.step`.
- then `settle_going()`: a row's walk it did not ask for ends, and `settle_journey()` holds a row's
  journey while another row acts and puts it down (broken off) once its row stands aside

## Steps

`steps::STEPS` (`autoplay/steps.rs`). A goal with no scorer is worth its `base`, written in the
table, so a row may be added or dropped without re-scoring the rest. `weigh` takes goals only, most
worth first, ties going to the earlier row; reflexes run in table order and their `base` is not
read. The fixed curves are placed against those numbers: `LOOT_AT_REST` (45) sits under `follow`
(50), and `WALK_TO_A_FIGHT` (40) ties `resume the journey`, which the tie rule puts second.

| # | step | layer | worth | runs |
|---|---|---|---|---|
| 1 | survive | reflex | - | `autoplay_survive` |
| 2 | dodge | reflex | - | `autoplay_dodge` |
| 3 | recover | reflex | - | `autoplay_recover` |
| 4 | academy | reflex | - | `autoplay_academy` |
| 5 | urgent buffs | reflex | - | `autoplay_buff` (urgent) |
| 6 | vitals | reflex | - | `autoplay_vitals` |
| 7 | loot | goal | `worth_looting` | `autoplay_loot` |
| 8 | catch up | goal | 120 | `autoplay_follow` (urgent) |
| 9 | team | goal | 110 | `autoplay_team` |
| 10 | salvage | goal | 100 | `autoplay_salvage` |
| 11 | town run | goal | `worth_town_run` | `autoplay_town_run` |
| 12 | summon | goal | `worth_fighting` | `autoplay_summon` |
| 13 | fight | goal | `worth_fighting` | `autoplay_fight` |
| 14 | keep to the area | goal | 70 | `autoplay_keep_to_area` |
| 15 | buffs | goal | 60 | `autoplay_buff` |
| 16 | follow | goal | 50 | `autoplay_follow` |
| 17 | resume the journey | goal | 40 | `autoplay_resume_journey` |
| 18 | explore | goal | 20 | `autoplay_explore` |
| 19 | grow | goal | 10 | `autoplay_grow` |

Housekeeping, every tick and never claiming it: `autoplay_pending_wield`, `autoplay_shield`,
`autoplay_rearm`, `autoplay_stock`, `autoplay_claim_pet_kills`, `autoplay_tidy`, `autoplay_learn_scrolls`, `autoplay_spend_xp`.

## Systems

Tests: `cargo test -p ac-client WORD` with the tests column's word. Log target: `ac_client::` plus
the log column, which is the module path of the file the system's entry point is written in. A
target covers its submodules, and a rule's own status line (`note`, `say`) comes out under
`ac_client::autoplay` whichever row wrote it (see the root map for `RUST_LOG`).

| system | entry fns | files | state | tests | log target | term |
|---|---|---|---|---|---|---|
| steps and tick | `tick_autoplay`, `weigh`, `reflexes` | `autoplay/mod.rs`, `autoplay/steps.rs`, `autoplay/config.rs` | `Autoplay.step`, `Autoplay.doing`, `Autoplay.status` | steps:: | autoplay | step |
| target choice | `pick_target`, `can_keep_target`, `a_fight_in_sight`, `nearest_fight`, `would_fight`, `can_take_on`, `ordered_target` | `autoplay/fight/target.rs`, `autoplay/fight/mod.rs`, `autoplay/team/orders.rs` | `Client.attack_target`, `Autoplay.config.fight`, `Autoplay.nearest_fight` | target | autoplay::fight::target | target |
| melee | `autoplay_fight`, `autoplay_fight_as`, `stalled_on`, `give_up_target`, `let_go`, `autoplay_take_stand` | `autoplay/fight/melee.rs`, `autoplay/fight/target.rs`, `autoplay/fight/stand.rs` | `Client.attack_target`, `Autoplay.engaged`, `Autoplay.given_up`, `Autoplay.stands` | target, stand | autoplay::fight::melee, autoplay::fight::stand | fight, stand |
| spells in a fight | `autoplay_fight_with_spells`, `autoplay_make_vulnerable` | `autoplay/fight/spells.rs`, `autoplay/fight/soften.rs`, `autoplay/cast.rs`, `aim.rs` | `Autoplay.casting_at`, `Autoplay.softening`, `Autoplay.vulned`, `Autoplay.cast_sent`, `Autoplay.fight_cast` | spell | autoplay::fight::spells | fight |
| critter | `critter`, `a_critter`, `ask_about_strangers` | `autoplay/fight/critter.rs` | `Fight.skip_critters`, `Client.appraisals` | critter | autoplay::fight::critter | critter |
| road | `on_its_way`, `passing_by`, `road_fight_over` | `autoplay/growth/road.rs`, `autoplay/fight/mod.rs` | `Fight.walk_past_on_the_way` | road | autoplay::growth::road | road |
| looting | `autoplay_loot`, `corpse_for_us`, `walk_to_corpse`, `how_to_take`, `room_for_loot`, `worth_looting` | `autoplay/loot/choose.rs`, `autoplay/loot/owed.rs`, `autoplay/loot/walk.rs`, `autoplay/loot/take.rs`, `autoplay/loot/room.rs`, `autoplay/steps.rs`, `crates/ac-loot/src/run.rs` | `Autoplay.corpse`, `Autoplay.looted`, `Autoplay.walking_to`, `Autoplay.loot_run`, `Client.packs_said_full` | corpse | autoplay::loot | corpse |
| corpse turns and shuts | `whose_turn`, `judge_shut`, `take_in_shuts`, `claim_on` | `autoplay/team/turns.rs`, `autoplay/team/shuts.rs`, `autoplay/loot/judge.rs` | `Autoplay.standing_by`, `Autoplay.shut_by`, `Autoplay.whose` | body | autoplay::team::shuts | turn, shut |
| ledger | `tick_retag`, `autoplay_tag_arrivals`, `judge_loot`, `loot_action` | `autoplay/ledger/retag.rs`, `autoplay/ledger/store.rs`, `autoplay/loot/judge.rs`, `crates/ac-loot/src/ledger.rs` | `Autoplay.ledger`, `Autoplay.pending_tags` | (`-p ac-loot ledger`) | autoplay::ledger | tag |
| salvage | `autoplay_salvage`, `salvage_tagged`, `next_salvage_batch` | `autoplay/ledger/salvage.rs` | `Autoplay.salvaging`, `Autoplay.last_salvage` | salvage | autoplay::ledger::salvage | salvage |
| pack tidy | `autoplay_tidy`, `settle_pour`, `pour_next`, `compress` | `autoplay/tidy.rs`, `autoplay/growth/mod.rs` | `Autoplay.pour`, `Autoplay.tidy_looked`, `growth::State.wont_merge` | pour | autoplay::tidy | tidy, pour |
| weapons | `autoplay_pending_wield`, `arm_for`, `autoplay_shield`, `autoplay_rearm` | `autoplay/hands/weapon.rs`, `crates/ac-loot/src/weapons.rs` | `Autoplay.pending_wield`, `Autoplay.wield_refused`, `Autoplay.armed_for` | wield | autoplay::hands::weapon | wield |
| ammo | `ready_ammo`, `autoplay_craft_ammo`, `choose_recipe`, `ammo_carried` | `autoplay/hands/ammo.rs` | `Autoplay.crafting`, `Autoplay.wanted_ammo` | arrow | autoplay::hands::ammo | ammo |
| healing | `autoplay_survive`, `choose_heal`, `self_heals`, `autoplay_vitals` | `autoplay/vitals/heal.rs` | `Autoplay.config.survive`, `Autoplay.last_heal`, `Autoplay.last_vital` | heal | autoplay::vitals::heal | heal, vitals |
| buffs | `autoplay_buff`, `wanted_buffs`, `due_buff`, `wanted` | `autoplay/vitals/buffs.rs`, `buffs.rs` | `Autoplay.config.buffs`, `Autoplay.last_buff`, `Autoplay.item_buffs` | buff | autoplay::vitals::buffs | buff |
| recruiting | `autoplay_fellowship`, `autoplay_accept_invites`, `next_invitee`, `hear_recruit_refusal` | `autoplay/team/fellowship.rs` | `Autoplay.recruited`, `Autoplay.held_off`, `Team.fellowship` | invit | autoplay::team::fellowship | recruit |
| team board | `autoplay_team`, `leader_mate`, `worst_hurt`, `rival_leader` | `autoplay/team/mod.rs`, `autoplay/team/view.rs`, `crates/ac-plugin/src/team.rs` | `Autoplay.team` (`TeamView.mates`), `Config.team` | leader | autoplay::team | mate |
| fellowship planner | `plan_for_team`, `take_orders`, `assign_targets`, `deal_bodies`, `stragglers` | `autoplay/team/orders.rs`, `plan.rs` | `Autoplay.planner`, `Autoplay.orders` | plan:: | autoplay::team::orders | plan, order |
| follow | `autoplay_follow`, `stop_following`, `followed_leader`, `is_led`, `team_leader`, `follow_break` | `autoplay/team/follow.rs` | `Autoplay.follow_trip`, `Autoplay.followed`, `Team.follow`, `Client.going` (`FOLLOW_ROWS`' walks) | follow | autoplay::team::follow | follow |
| played by hand | `autoplay_by_hand`, `autoplay_assist` | `autoplay/team/by_hand.rs` | `Team.follow`, `Team.focus_fire` | by_hand | autoplay::team::by_hand | assist |
| quartermaster | `autoplay_quartermaster`, `autoplay_stock`, `decide`, `quartermaster`, `hand_out` | `autoplay/team/quartermaster.rs`, `logistics.rs`, `autoplay/growth/policy.rs` | `growth::State.mode`, `Team.restock` | quartermaster | autoplay::team::quartermaster | quartermaster |
| town run | `autoplay_town_run`, `grow_town_run`, `start_town_run`, `grow_run_step`, `grow_run_next`, `choose_stop`, `pick_vendor`, `plan_calls`, `giver_step` | `autoplay/growth/town_run/mod.rs`, `autoplay/growth/town_run/counter.rs`, `autoplay/growth/town_run/vendor.rs`, `autoplay/growth/town_run/panel.rs`, `autoplay/growth/town_run/calls.rs`, `crates/ac-world/src/tour.rs`, `shopping.rs`, `crates/ac-vendor/src/run.rs` | `growth::State.run`, `growth::State.shop` | counter | autoplay::growth::town_run | town_run, counter |
| supplies and sale | `grow_needs_with`, `supplies`, `sell_policy`, `offers_for_sale`, `burns` | `autoplay/growth/needs.rs`, `autoplay/growth/policy.rs`, `autoplay/growth/sale.rs`, `autoplay/growth/supplies.rs`, `crates/ac-loot/src/sale.rs` | `growth::State.needs`, `Profile.buy` (an `Ammunition` line: the launcher's rounds) | counter | autoplay::growth | need, sale |
| XP spending | `autoplay_spend_xp`, `grow_spend_xp`, `raise_offers`, `batch_raise` | `autoplay/growth/xp.rs`, `autoplay/growth/raise.rs`, `advance.rs` | `growth::State.pending`, `growth::State.sulking` | experience | autoplay::growth::xp | raise |
| hunting ground and area | `autoplay_grow`, `grow_hunt`, `autoplay_watch_the_ground`, `autoplay_keep_to_area` | `autoplay/growth/mod.rs`, `autoplay/growth/hunt.rs`, `autoplay/growth/quest_bonus.rs`, `hunt.rs` | `growth::State.bound`, `growth::State.quiet_since`, `Fight.area` | ground | autoplay::growth::hunt | ground, area |
| walk | `go`, `go_to`, `stop_going`, `settle_going`, `walk_journey` | `body/going.rs`, `body/player_tick.rs`, `body/standing.rs`, `pathfinder.rs`, `crates/ac-nav/src/steering.rs`, `crates/ac-nav/src/routing.rs` | `Client.going`, `Client.steering`, `Client.refusals` | going | body::going | walk |
| travel | `travel_to`, `travel_about`, `set_off`, `keep_journey`, `settle_journey`, `autoplay_resume_journey` | `travel.rs`, `travel/overland.rs`, `gem_room.rs`, `recalls.rs`, `autoplay/journey.rs` | `Client.travel`, `Autoplay.resume_trip` | travel | travel | journey |
| visits | `visit_landmark`, `tick_visit`, `visit_person` | `visit.rs` | `Client.visits` | visit | visit | visit |
| dungeon explore | `autoplay_explore`, `spot_in_room`, `past_the_door` | `explore.rs`, `crates/ac-nav/src/explore.rs` | `Autoplay.room_bound`, `Autoplay.rooms_seen` | explore | explore | explore |
| dodge | `autoplay_dodge`, `autoplay_approach`, `sidestep`, `threat` | `dodge.rs`, `aim.rs`, `doorway.rs` | `Client.dodge`, `Client.dodge_to` | dodge | dodge | dodge |
| recovery | `autoplay_recover`, `recovery_view`, `is_dead` | `recovery.rs` | `Autoplay.recovery` | recovery:: | recovery | recovery |
| academy | `autoplay_academy` | `academy.rs`, `tests/academy_route.rs` | `Autoplay.academy`, `Autoplay.academy_corpse`, `Autoplay.academy_armed` | academy | academy | academy |
| summoning | `autoplay_summon`, `autoplay_claim_pet_kills`, `hear_summoning` | `autoplay/summoning.rs`, `autoplay/mod.rs` | `Autoplay.summoning` | summon | autoplay::summoning | summon, pet |
| refusals | `hear_refusal`, `refused`, `answer` | `refused.rs`, `crates/ac-agent/src/refusals.rs` | the waiting system's own wait, e.g. `Autoplay.shelved` | refus | refused | refusal |
| server chat | `chat_message`, `hear_arrival`, `hear_spell_attack` | `session/chat.rs`, `autoplay/hear.rs` | `Autoplay.hit_by` | arriv | session::chat | heard |
| quests | `refresh_quests`, `quest_board`, `quest_wants`, `visit_giver`, `watch_quest_answer`, `refresh_pursuit`, `keep_quest_section`, `start_quest_call` | `quest_log.rs`, `crates/ac-world/src/quests.rs`, `autoplay/growth/pursuit.rs`, `autoplay/growth/pursuit/fetch.rs`, `autoplay/growth/pursuit/section.rs`, `autoplay/growth/town_run/calls.rs` | `Client.quest_board`, `Client.quest_log`, `Client.pursuit`, `Autoplay.config.quests` | quest_log | quest_log | quest, pursuit |

## Glossary

| term | means | not |
|---|---|---|
| corpse | a dead creature's container that autoplay opens and empties | body, remains |
| town_run, call | a trip to sell loot and restock at counters, handing in at quest givers on the way; each stop a call | errand, sale run, trip |
| counter | a vendor's shop the town run stands at | shop, store |
| road | a walk on the way somewhere (a ground, a counter, a script's goal); on it only what attacks is fought | way |
| ground | the hunting spot growth chose; walking about it is not a road | spot, field |
| area | a hunting area a player drew on the map (`HuntArea`) | zone |
| walk | the one movement the slot follows (`Going`), planned whole by the region planner, owned by the row that asked it | leg, path, head_for |
| journey | a planned trip of walks, portals and recalls (`Travel`), owned by the row that set it off or took it over; held while another row acts, put down when its row stands aside; each walk a stretch the slot follows (`Going`) | route, travel plan |
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
| assist | hit what the leader is hitting, picking nothing | help, support |
| quest, giver, hand_in, pursuit | a quest in `ac_world::quests`, its rows sharing an id; the NPC who rewards it; giving it the items, or talking, for the reward; a quest the player asked autoplay to work before hunting for XP (`Quests.pursue`) | task, npc, turn-in, goal, order |

## Domain rules

- Critter rule, `critter()` applied by `a_critter()`: a creature is walked past only when it is
  quiet (has not attacked, walks at nobody of ours, fights nobody), the creature table does not
  mark it aggressive, its level is at most half the character's, and its health is at most 5
  (`CRITTER_HEALTH`; 30 for a creature the table does not know). Unknown level or health: appraise
  and leave it alone until the answer. Never loosen it to "passive": ACE's newbie spawns are all passive.
- Collision rule, `add_model()` in crates/ac-scene/src/collision.rs: a placed model collides by its
  parts' physics polygons, else its Setup's cylinder-spheres (else spheres), else not at all; never
  by drawn polygons. Server-placed objects never stall a walk; `cylinders()` only steers round them.
- Doors are walked through, never used: `in_the_way()` never counts a door, explore aims past the
  sill (`past_the_door()`), and the Academy uses only a task's key on one.
- Peas are components in the data: `spell_component_ids()` maps trade peas to spell-component ids,
  so never key restock or sale on "is a component". Only the profile's buy list is bought (`grow_needs`);
  the sale guard keeps what cast spells burn (`burns()`), and a pea tagged Sell is sold (`offer_to_vendor()`).
- The loot profile decides: `fate()` puts a tag ahead of every guard (restock, burns, keep names);
  only the server's refusal overrides it. Quests keep and buy via its Quests section (`keep_quest_section()`).
- One target picker. Swing, shot and spell all call `pick_target()`: the nearest of what
  `would_fight()` allows inside `cfg.radius`, within the leader's `team.fight_radius` while following
  one (not in the academy) unless it is hitting the character (`hit_lately_by()`), and preferring one
  the attack in hand gets to, then a pursued quest's creature (`pursuit_sought`): `attack_kind()` reads the hands, never `Client::missile`, and names the
  spell per creature by the cast's own `kill_in_hand()` (the fastest kill, `autoplay/fight/kill.rs`), so the flight tested is the flight thrown.
  Nothing beside the leader, nothing is picked (`follow` fetches it; past `follow_break()` catch up
  outranks the fight). A team proposal outranks the pick only through `can_take_on()` (less the
  vitae's `shy_of()`); `can_keep_target()` holds the fight in hand, wherever the leader is.
- Going with the leader, `is_led()`: keep to the area (bar into its dungeon after a leader in it),
  resume the journey, explore and a ground step aside, none is put aside; the area limits its fights.
- Played by hand, `autoplay_by_hand()`: with autoplay off only the team's own rules run -- invites,
  a leader's fellowship, following and assisting. Nothing takes the legs or the hands for an errand
  of its own: no town run, no ground, no experience spent, no weapon changed, no target picked.
  The assist asks only `joins_the_team_on()`: the player's rules, not `can_take_on()`.
- Refusals table: `refused()` quotes ACE's words with `File.cs:line` and `answer()` sets the wait;
  `hear_refusal()` hands each to the waiting system. A new one is a row, a test in the exact words,
  an `answer()` arm and a hand-off; never a `strip_prefix` in the system that noticed.

## Other modules

| module | what |
|---|---|
| `lib.rs` | `Client`, `Event`, `Config`, `placed`, `drain_events`; the module declarations, and the re-exports that keep every old path: `ac_agent::{did, pack, refusals, room, weenie_errors}`, `route` (`ac_nav::steering`), `errand` (`ac_vendor::errand`), and the names lifted into `session/`, `body/` and `actions/` |
| `action/mod.rs` | one vocabulary for what a front end asks: `Action`, `Client::act`, the retail command table (`RETAIL`, `PENDING`) with the `chat_line()` router over it, and a file per family (`action/allegiance.rs`, `action/chat.rs`, `action/combat.rs`, `action/consent.rs`, `action/fellow.rs`, `action/help.rs`, `action/item.rs`, `action/magic.rs`, `action/recall.rs`, `action/status.rs`, `action/team.rs`, `action/trade.rs`); `action/resolve.rs` is the one place a name becomes a spell, a pack item, a ware, an object, the last corpse or a place. External intent only: autoplay and every read call `Client` directly |
| `actions/mod.rs` | what a player, script or panel can ask for: `actions/combat.rs` (casting, attacking, what is wielded), `actions/items.rs` (using, taking, packing, splitting, salvaging), `actions/social.rs` (emotes, fellowship, friends, housing, allegiance, chat), `actions/trade_vendor.rs` (trade windows and counters), `actions/ui.rs` (`interact`, selection) |
| `session/mod.rs` | the connection and what arrives on it: `session/connect.rs` (connect, offline, disconnect, log off), `session/net.rs` (sending), `session/apply.rs` (`Client::tick` and the message match), `session/lobby.rs` (the character list), `session/chat.rs` (server chat and sounds) |
| `body/mod.rs` | where the character is and how it gets there; the walk row holds the slot, its follower and `pathfinder.rs` (region graphs built off the frame, planned on from it): `body/movement.rs` (speed, jumps, noclip, `SAME_FLOOR`), `body/server_walk.rs` (walks the server makes for us, `reached` in 3D) |
| `player.rs` | body physics: walking, floors, ledges, falls, jumps, cell tracking |
| `creation.rs` | character creation rules and create-if-missing |
| `magic.rs` | spellbook, spell bars, enchantments, components, cast checks |
| `items.rs` | appraisal requests and answers; the item vocabulary is `ac_loot::items` |
| `profile.rs`, `weapons.rs`, `daytime.rs`, `augmentations.rs` | shims re-exporting `ac_loot::profile` and `ac_loot::weapons`; Dereth's time of day; augmentation gems |
| `holdings.rs` | every character's inventory snapshots, for the Items window |
| `reconnect.rs`, `logoff.rs` | whether and when a dropped session logs back in; logging every session off on exit |
| `options.rs`, `emotes.rs`, `position.rs` | character option bits; soul emotes; `my_position()`: the character's world position |
| `tally.rs` | what the character did lately, for telemetry: this frame's walk (`WalkFrame`), blows traded (`Blows`), supplies short (`SupplyLine`) |
| `advance.rs` | experience costs and raise messages |
| `recalls.rs`, `quest_flags.rs`, `quest_log.rs` | recall spells the journey planner can use; the portal quest flags it may plan through, earned by talking to whoever stamps one; each character's quest log |
| `shopping.rs` | reads the pack into `ac_vendor`'s snapshot and carries out its act |
| `aim.rs` | whether a projectile clears walls and hills on the way to its target |
| `logistics.rs` | the party's mode (hunting or restocking) and money sharing, the same on every session |
| `buffs.rs`, `hunt.rs` | which buffs a character should wear, from its skills and spells; hunting areas drawn on the map |
| `testkit.rs`, `walk_trial.rs` | test-only: `Client::offline` sessions over `no_data()` or `game_data()`, `standing_at()`, `creature()`, `corpse()`, `mate()` builders; the route harness (`walk_trial::run`, `tests/route_harness.rs`) |

Unit tests sit in each file's `mod tests`; those reading the archives are ignored (`cargo test-data`).
