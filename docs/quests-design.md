# Quests design

Status: designed, 2026-11-02 (branch `quests`, from main `8b7665d`), for all four steps of
`docs/quests-plan.md`. Line anchors are on `8b7665d`. Every count below was measured on the local
ACE world database (`version`: base v0.8.8, patch v0.9.296, 2026-10-08) with the queries in §1.3
and a prototype of the walk in §1.4; ACE source citations are under
`reference/ext/ACE/Source/ACE.Server/` unless they name `ACE.Entity` or `ACE.Common`.

The plan's rules this design keeps:
- A quest is completable only if its items get looted, so the quest list and the loot profile are
  one idea: the profile keeps what an active quest wants, by an ordinary rule.
- The profile decides. The quest rule is one rule among the others: moved, turned off, or
  overruled by a rule above it. No guard anywhere skips the profile for a quest item.
- The data ships with acswarm, generated from ACE's world database like portals and hunting.
- Players switch quests on and off (a quest paying XP starts on, any other only when chosen by
  hand); autoplay considers only quests that are on and not blocked,
  and a blocked one says why (the rule to move, the level, the locked area).
- Turn-ins may be anywhere: every giver is a `Place` with its cell, and the run that hands in is
  the town run with more calls, planned on phase 4's journeys.

Phase 1 (Warden portal flags, `crates/ac-world/src/unlocks.rs`, `crates/ac-client/src/quest_flags.rs`)
is built. Quests build on it: a quest's stamped flags join `Travel::quests_done` through the same
save, and `quests.csv` follows the `unlocks.csv` conventions (hex cell, landblock-local x/y/z,
`landblock_origin(cell) + local`). Nothing in phase 1 is duplicated.

## 1. The data: `quests.csv` and `quest_sources.csv`

### 1.1 What ACE does (the meanings the generator reads)

Quest logic lives in emote sets (`weenie_properties_emote`: object, category, probability, the
item's wcid for Give/Refuse, a `quest` key) and their actions in order
(`weenie_properties_emote_action`). A branching action runs the set of its outcome category whose
`quest` equals the action's `message` (`GetEmoteSet`, WorldObjects/Managers/EmoteManager.cs:1512-1556;
case-insensitive, `@comment` included). The player's registry is per quest name with
`NumTimesCompleted` and `LastTimeCompleted`; the `quest` table gives `min_Delta` (seconds before
another solve) and `max_Solves` (-1 none).

| emote / action (type) | meaning | ACE |
|---|---|---|
| category Use (7) | the root when the NPC is used (talked to) | ACE.Entity/Enum/EmoteCategory.cs:19 |
| category Give (6), keyed by item wcid | the NPC takes ONE of the item handed, then runs the set | WorldObjects/Player_Inventory.cs:3437-3459 (one from a stack, :3441-3443) |
| category Refuse (1), keyed by item wcid | the NPC examines the item and hands it back (WeenieError 0x04CD), then runs the set; checked before Give | Player_Inventory.cs:3461-3468; WorldObjects/WorldObject.cs:265-282 |
| probability < 1 | the set is drawn at random | EmoteManager.cs:1546-1551 |
| InqQuest (21) | QuestSuccess (12) when the quest is held AND cannot be solved now (on its wait, or at `max_Solves`); else QuestFailure (13) | EmoteManager.cs:620-635 |
| InqQuestSolves (30) | QuestSuccess when solves are in [min, max], 0 when not held | EmoteManager.cs:667-678; Managers/QuestManager.cs:507-523 |
| InqIntStat (36), stat 25 = Level | TestSuccess (22) when the level is in [min, max] | EmoteManager.cs:553-578 |
| InqOwnsItems (76) | TestSuccess when pack plus worn hold `stack_Size` of the wcid | EmoteManager.cs:580-594 |
| Goto (67) | runs GotoSet (32) of that name | EmoteManager.cs:397-403 |
| StampQuest (22), UpdateQuest (20) | one more solve, stamping the time; capped at `max_Solves` | EmoteManager.cs:1265-1279, :1462-1488; QuestManager.cs:172-214 |
| SetQuestCompletions (70) | creates or sets the registry entry: how a kill task is taken up (amount 0) | EmoteManager.cs:1226-1233; QuestManager.cs:220-262 |
| EraseQuest (31) | forgets it: a kill counter is erased when handed in | EmoteManager.cs:345 |
| AwardXP (2) | fixed XP, shared with the fellowship (`ShareType.All`) | EmoteManager.cs:170-184 |
| AwardNoShareXP (62) | fixed XP, not shared | EmoteManager.cs:142-147 |
| AwardLevelProportionalXP (49) | `round(percent * XP(level -> level+1))`, then `min(.., max)` if max > 0, `max(.., min)` if min > 0; shared with the allegiance only | EmoteManager.cs:126-133; WorldObjects/Player_Xp.cs:441-455, :254-266 |
| Give (3) | the reward item, `stack_Size` of it | EmoteManager.cs:375-393 |
| TakeItems (74) | takes `stack_Size` of the wcid (-1 all) from pack or worn | EmoteManager.cs:1297-1325 |
| PropertyString KillQuest 45, 49, 50 on a creature | its death counts for that counter quest, if the player holds it (started), for every damager and fellows in range, capped at `max_Solves` | WorldObjects/Creature_Death.cs:44-49, :209-299; QuestManager.cs:583-627 |
| create list, destination Contain (1) or Treasure (8) without Wield (2) | a corpse drop; with Treasure and `shade` > 0 the chance is `shade`, in sets of chances summing to 1 | Creature_Death.cs:688-693; WorldObjects/Creature_Equipment.cs:636-680 |
| `%tqt` in a message | replaced by the time left, `"1d 2h 3m 4s"`; `name@%tqt` becomes "You may complete this quest again in %tqt." | EmoteManager.cs:1820-1835; ACE.Common/Extensions/TimeSpanExtensions.cs:8-33 |
| wait measured | `LastTimeCompleted + min_Delta * quest_mindelta_rate` (1 unless the server sets it; ColoArena never scaled) | QuestManager.cs:323-363 |

A kill task, as the data has it (San Ming, below): Use asks InqQuest on the task's wait; off its
wait, InqQuestSolves on the counter; with no solves, a Tell and SetQuestCompletions(counter, 0)
(taken up); with the count reached (InqQuestSolves min = N, or InqQuest on `counter@...` at
`max_Solves`), the reward set stamps the wait, erases the counter and awards. Every kill in between
sends "You have killed {n} {plural}! You must kill {N} to complete your task." or "... Your task is
complete!" (QuestManager.cs:619-624).

### 1.2 Scope: every quest a placed NPC holds

A quest is in `quests.csv` when its giver is placed (a `landblock_instance` row of weenie type
Creature (10) or Vendor (12), ACE.Entity/Enum/WeenieType.cs; switches, chests and portals are out)
and a path walked from its Use, Give or Refuse set (§1.4) does anything for the character: XP, an
item, a flag, a spell, a skill, luminance, a title, a teleport, an item taken, a stat, a contract,
enlightenment (the `does` column). A path that only talks or moves is no quest. Nothing else is
dropped:
- a path paying no XP stays (Fiun Rehlyun's jaw for `AugmentationProvingGroundAccess`, the Blank
  Augmentation Gem trainers, Sir Ginazio's `eaterpit` stamp);
- a branch the client cannot know (stats, society rank, quest bits, events, another quest's wait,
  solve-count ranges, two items asked, a random reply) stays, named in the `unread` column;
  autoplay hands such a row in only for a quest the player switched on by hand or pursues;
- a want nothing known drops still stays; it has no `quest_sources.csv` row.

Rows that differ only in the giver's placement are one row; the placements are
`quest_givers.csv`. A random reply's sets (probability < 1) that differ only in what they give are
one row whose `gives` lists every alternative.

Measured (2026-11-03, on the database below):

| what | count |
|---|---|
| quests (ids): item / talk / kill | 5,185: 4,172 / 864 / 149 |
| of them paying XP on some row | 1,301 |
| rows: item / talk / start / kill | 16,023: 13,533 / 2,104 / 199 / 187 |
| rows with an `unread` condition; of them a random reply; rows with alternative gives | 5,904; 1,562; 634 |
| rows by `does`: takes / item / flag / xp / spell / title / luminance / skill / stat / teleport | 13,565 / 8,660 / 5,206 / 3,005 / 842 / 603 / 400 / 223 / 164 / 158 |
| givers (placements; indoors) | 2,179 NPCs (3,058; 2,103) |
| repeat, by row: any time / 20 h / once / 6 d | 11,635 / 3,136 / 700 / 217 |
| `quest_sources.csv` rows: drop / kill / npc | 5,493: 4,718 / 430 / 345 |
| file size, plain text | quests 2.7 MB, givers 147 KB, sources 418 KB |

### 1.3 Generating it

`reference/scripts/data/quests.sh` (one script for both files, as `hunting.sh` is for one), usage
`reference/scripts/data/quests.sh crates/ac-world/data` writing `quests.csv`, `quest_givers.csv`
and `quest_sources.csv` there. It runs the bounded SELECTs below through
`docker exec ace-db sh -c 'mysql -uroot -p"$MYSQL_ROOT_PASSWORD" -N --batch ace_world -e "..."'`
into a `mktemp -d` directory, each beginning `SET SESSION max_execution_time=20000;`, and pipes
them to `python3 -I reference/scripts/data/quests_walk.py DIR OUT` (stdlib only; the walk of §1.4
is a graph search SQL and awk do badly; `reference/scripts/index/` already holds Python). No query
joins the emote tables without the placed-objects `where`; the largest returns 127k rows.

```sql
-- placed: the objects with a Refuse, Give or Use set, and where they stand
select li.weenie_Class_Id, hex(li.obj_Cell_Id), round(li.origin_X,2), round(li.origin_Y,2),
  round(li.origin_Z,2), n.value, w.type
from landblock_instance li join weenie w on w.class_Id = li.weenie_Class_Id
join weenie_properties_string n on n.object_Id = li.weenie_Class_Id and n.type = 1
where li.weenie_Class_Id in (select object_Id from weenie_properties_emote where category in (1,6,7));
-- sets and actions of placed objects only (57k and 127k rows)
select id, object_Id, category, probability, coalesce(weenie_Class_Id,0), coalesce(quest,'')
from weenie_properties_emote where object_Id in (select weenie_Class_Id from landblock_instance);
select a.emote_Id, a.order, a.type, <message with tabs and newlines replaced>, a.min, a.max,
  a.amount, a.amount_64, a.percent, a.min_64, a.max_64, a.stat, a.weenie_Class_Id, a.stack_Size
from weenie_properties_emote_action a where a.emote_Id in
  (select id from weenie_properties_emote where object_Id in (select weenie_Class_Id from landblock_instance));
-- the quest table, and the kill counters creatures carry
select name, min_Delta, max_Solves, message from quest;
select object_Id, type, value from weenie_properties_string where type in (45, 49, 50);
-- corpse drops: creature create lists, Contain or Treasure and not Wield (11k rows)
select cl.object_Id, cl.weenie_Class_Id, cl.destination_Type, cl.shade, cl.stack_Size
from weenie_properties_create_list cl join weenie w on w.class_Id = cl.object_Id and w.type = 10
where (cl.destination_Type & 2) = 0 and (cl.destination_Type & 9) <> 0 and cl.weenie_Class_Id <> 0;
-- names, levels (int 25), item type (int 1) and value (int 19), weenie types
select object_Id, value from weenie_properties_string where type = 1;
select object_Id, type, value from weenie_properties_int where type in (1, 19, 25);
select class_Id, type from weenie;
-- spawn entries per creature and landblock, the walk and filters of hunting.sh (20k rows)
with recursive spawn(inst, wcid, depth) as (
  select li.guid, li.weenie_Class_Id, 0 from landblock_instance li
  union all
  select s.inst, g.weenie_Class_Id, s.depth + 1 from spawn s
  join weenie_properties_generator g on g.object_Id = s.wcid where s.depth < 4)
select s.wcid, hex(li.obj_Cell_Id >> 16) lb, count(distinct s.inst)
from spawn s join landblock_instance li on li.guid = s.inst
join weenie c on c.class_Id = s.wcid and c.type = 10
group by s.wcid, lb;
```

The last query must use `hunting.sh`'s attackable and level filters and its rule that a landblock
with a surface counts only its outdoor spawns, so a quest's share of a ground (§4.1) is a fraction
of like with like (`Ground::count` counts the same entries).

### 1.4 The walk

For every placed giver and every root set (Use; Give and Refuse per item wcid), follow actions in
order to depth 8, never re-entering a set: a branching action (21, 30, 36, 76, 67, and the other
Inq types) continues once into its success set and once into its failure set, carrying the branch
taken; other actions are the path's effects. ACE runs the actions after a branch too; the 24 sets
of placed objects that have any are walked as if they belonged to both outcomes. Each path that
does anything (§1.2) is read:

| branch on the path | read as |
|---|---|
| InqIntStat Level, success | the row's `min_level`/`max_level` |
| InqIntStat Level, failure, on a band's low or high end | the band past it: failing 1-49 is 50 and up (a ladder) |
| InqQuest on a quest with `min_Delta` > 0, failure | the quest's wait (`timer`, `repeat` = its `min_Delta`) |
| the same, success | the wait reply: not a reward path |
| InqQuest `counter@...` success, or InqQuestSolves on a counter with min >= 2 | kill N: N = that min, else the counter's `max_Solves` |
| InqQuestSolves on a counter, min 1, failure | not taken up yet: the start talk's path (a `start` row, no XP) |
| InqQuest or InqQuestSolves (min >= 1, open max) on a flag the same path stamps, failure | once (`repeat` = -1, `timer` = the flag) |
| the same on a flag another row stamps, success | `needs` that flag |
| the same, failure (not held) | kept only where the held path pays the same: then the flag is not a need |
| the same on a flag no row of the file stamps, failure | asks nothing (never held; the walk's second pass) |
| the wanted item's own `Quest` string (PropertyString 33) | the quest repeats no sooner than that quest's `min_Delta`, or once at `max_Solves` 1 (Player_Inventory.cs:1249-1262) |
| the wanted item's `QuestRestriction` (37) | `needs` that flag (Player_Inventory.cs:1207-1211) |
| a start talk's wait (InqQuest on a timed quest, failure) | the quest's `timer` and `repeat` when its reward asks none |
| InqOwnsItems success | the item and count asked (with TakeItems on the path) |
| anything else | the row stays, the branch named in `unread` |

Paths are grouped into quests by `id`: the kill counter, else the wait's quest, else the once-flag,
else the first stamped flag, all lowercased; a hand-in with none of these (the trophy collectors)
is `give:<item wcid>`, so one quest is "hand in a Red Rat Tail to any of the twelve Collectors".
A quest's rows are alternatives: any one completes it. The `title` is the `quest` table's `message`
("Blood Shreth Kill Task", "Sent to the Barkeep"), or the item's name for a `give:` quest.

### 1.5 `quests.csv` columns

One row per way to complete a quest: giver and level band. Commas in names become `;` (as
`vendors.sh`).

| # | column | meaning |
|---|---|---|
| 1 | `id` | the quest (§1.4); rows sharing it are alternatives |
| 2 | `kind` | `kill`, `item`, `talk`, or `start` (the talk that takes a quest up: stamps what a row `needs`) |
| 3 | `title` | what the list shows |
| 4 | `giver_wcid` | the NPC; its name and placements are in `quest_givers.csv` |
| 5 | `hand` | `give`: hand the item over; `talk`: use the giver |
| 6 | `want` | item wcid (`item`), the counter (`kill`), empty |
| 7 | `count` | items one hand-in takes (a Give takes one per item handed), or kills |
| 8-9 | `min_level`, `max_level` | the giver's Level band, 0 for open |
| 10 | `xp` | fixed XP, 0 when scaled |
| 11-13 | `xp_level`, `xp_min`, `xp_max` | AwardLevelProportionalXP's percent (of the XP to the next level) and clamps, 0 for none |
| 14 | `share` | `all` (fellowship), `none`, `allegiance` |
| 15 | `gives` | reward items, `wcid:count;...`; a random reply's alternatives `\|`-separated, an empty one giving nothing |
| 16 | `stamps` | flags the reward stamps, `;`-joined |
| 17 | `repeat` | seconds before it may be done again; 0 any time; -1 once |
| 18 | `timer` | the quest-table name its wait or once-flag is kept under |
| 19 | `needs` | flags held first, `;`-joined; a kill row needs its own counter, which its start row stamps |
| 20-22 | `item`, `item_type`, `item_value` | the wanted item's name, hex ItemType and value: what the loot rules read |
| 23 | `does` | what the path does: `xp`, `item`, `flag`, `spell`, `skill`, `luminance`, `title`, `teleport`, `takes`, `stat`, `contract`, `enlightenment`, `type N` |
| 24 | `unread` | conditions the client cannot check, `\|`-separated (`a random reply` among them) |

`quest_givers.csv`: `giver_wcid,giver,cell,x,y,z`, one row per placement: hex cell, x/y/z local to
its landblock (`unlocks.csv`'s convention).

`quest_sources.csv`: `want,kind,wcid,name,level,chance,landblocks`. `want` is an item wcid or a
counter; `kind` is `drop` (corpse create list, `chance` per kill), `kill` (a creature carrying the
counter, chance 1) or `npc` (a placed NPC whose talk path gives the item; `landblocks` is its cell);
`landblocks` is `LB:entries;...`, hex landblocks with spawn entries counted as in §1.3.

### 1.6 Three worked examples (rows as generated)

**A kill task: Blood Shreth Kill Task (San Ming, Shoushi).** San Ming (31641) stands in DA55001D.
Use: InqQuest `KilltaskBloodShrethWait` (min_Delta 72,000 s); off it, InqQuestSolves
`KilltaskBloodShreth@10` (min 10), then `@1-9` (1-9); with no solves he tells the task and
SetQuestCompletions(`KilltaskBloodShreth`, 0). Blood Shreth (4110, level 8) carries KillQuest
`KilltaskBloodShreth`, whose `max_Solves` is 10. At 10: Tell, StampQuest the wait, EraseQuest the
counter, AwardLevelProportionalXP 1.33 capped at 120,000, title 25, Give 31617 (Blood Shreth
Butcher Plaque).
```
killtaskbloodshreth,start,Blood Shreth Kill Task,31641,San Ming,DA55001D,73.58,98.23,20,talk,,0,0,0,0,0,0,0,none,,killtaskbloodshreth,0,,,,,
killtaskbloodshreth,kill,Blood Shreth Kill Task,31641,San Ming,DA55001D,73.58,98.23,20,talk,killtaskbloodshreth,10,0,0,0,1.33,0,120000,allegiance,31617:1,killtaskbloodshrethwait,72000,killtaskbloodshrethwait,killtaskbloodshreth,,,
```
```
killtaskbloodshreth,kill,4110,Blood Shreth,8,1,154:1;161:4;162:17;1ED:3;5756:2;...;CE4E:6;DB54:1
```

**An item hand-in: Red Rat Tail (any Collector).** Twelve Collectors (Yukara Miko in 2B110112,
the Collectors of 3EA4, 7ACA, A3A4, 9058, DA3B, A260, B470, 33D9, Black Hill...) each have a Give
set keyed 3684 with no branch: AwardNoShareXP 1,000 and Give 10 Stamina Elixirs (2470), every
time. Red Rat (949, level 15) drops it with shade 0.15.
```
give:3684,item,Red Rat Tail,24573,Yukara Miko the Collector,2B110112,39.96,163.46,48.01,give,3684,1,0,0,1000,0,0,0,none,2470:10,,0,,,Red Rat Tail,80,2
```
(and eleven more rows, one per Collector placement)
```
3684,drop,949,Red Rat,15,0.15,118:6;161:2;1C4:17;1D3:5;1D9:4;1DD:4;1F9:2;3FE:4;7D62:1;8C44:5;D756:3
```

**A talk chain: Sent to the Barkeep, then Sent to the Pathwarden (Holtburg).** Alcott (44895,
A9B40019) on Use: InqQuest `ToTheBarkeep` (`max_Solves` 1); not held: Tells, StampQuest
`LifestoneFound1111`, AddContract, AwardNoShareXP 7,000, StampQuest `ToTheBarkeep`. Buckminster
(44891, A9B40163): InqQuest `ToTheBarkeep`, held (62063, then 62068) or not (62066, then 62067),
then `ToPathwarden@BarkeepFlagCheck` failure: StampQuest `BartenderFound1111`, AwardNoShareXP
9,300, Give 500 pyreals, StampQuest `ToPathwarden`. Both ways pay the same, so the row needs nothing.
The same pair stands in each starter town (Rufina and Fiorella 33D9, Ghaziyah 7D64, Eiichi and
Benjirou DA55).
```
tothebarkeep,talk,Sent to the Barkeep,44895,Alcott,A9B40019,83.3,14.04,94,talk,,0,0,0,7000,0,0,0,none,,lifestonefound1111;tothebarkeep,-1,tothebarkeep,,,,
topathwarden,talk,Sent to the Pathwarden,44891,Buckminster,A9B40163,104.41,35.19,94,talk,,0,0,0,9300,0,0,0,none,273:500,bartenderfound1111;topathwarden,-1,topathwarden,,,,
```

### 1.7 `ac_world::quests`

`crates/ac-world/src/quests.rs` (leaf name unique), `include_str!` of both files, parsed once
(`OnceLock`) like `unlocks.rs`:
- `Row { id, kind: Kind, title, giver_wcid, giver, place: Place, hand: Handing, want: Want, count,
  min_level, max_level, reward: Reward, gives: Vec<(u32, u32)>, stamps, repeat: Repeat, timer,
  needs, item: Option<Item> }`; `Want::{Item(u32), Kill(String), Nothing}`;
  `Repeat::{Any, After(u32), Once}`; `Item { wcid, name, item_type, value }`.
- `Reward { xp, xp_level, xp_min, xp_max, share }` with `fn xp_at(&self, to_next: u64) -> u64`:
  Player_Xp.cs:441-455 exactly, `to_next` from `XpTable::xp_to_next_level` (ac-formats; the caller
  passes it, so ac-world needs no table).
- `Source { want, kind, wcid, name, level, chance, landblocks: Vec<(u32, u32)> }`.
- `all() -> &'static [Row]`, `sources() -> &'static [Source]`, `rows_of(id)`, `sources_of(&Want)`,
  `stamped_by(flag)` (the rows that stamp it), and `Row::locked_by(held) -> Option<&str>`: the
  giver's cell is in a dungeon landblock whose every working portal in asks a flag not held (the
  test `hunting::with_entrances` makes).
- Tests: the three examples parse to the rows above; every `needs` is stamped by some row; every
  kill row has a start row and a source; every item row has a source; giver cells' landblocks are
  real (`landblock_origin`).

## 2. Step 1: the quest list and its states (no behaviour change)

### 2.1 The saved config

`ac_client::autoplay::Config` gains `pub quests: Quests` (the struct is `#[serde(default)]`, so
every saved config loads):

```rust
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Quests {
    /// Autoplay considers quests at all; off, the quest rule wants nothing.
    pub on: bool,
    /// Quests switched off, by `ac_world::quests` id; one paying XP the data adds later starts on.
    pub off: Vec<String>,
    /// Quests switched on by hand: every way to complete one is worked, XP or not, unread or not.
    pub chosen: Vec<String>,
}
```
Off by default, so a saved config and a new one keep today's behaviour until the player ticks it.
The list is the shared autoplay config (every character reads the same on/off), as every autoplay
setting is. Saved as part of `autoplay.config`; no rename, so no alias.

### 2.2 The character's quest log

The server never sends the registry (`quest_flags.rs`'s doc), so the client keeps its own:
`crates/ac-client/src/quest_log.rs` (leaf unique), `QuestLog { kills: BTreeMap<String, u32>,
done: BTreeMap<String, Solved { at: u64, times: u32 }>, waits: BTreeMap<String, u64> }` keyed by
quest id or timer, Unix seconds. Saved beside the phase 1 flags as
`quests/<host>/<character>.log.json` (`ac_store::write_json_atomic`, Visibility::Normal), read at
login with `load_quest_flags`. Flags a quest stamps go to `Travel::quests_done` through
quest_flags.rs's own save (one list of held flags). In step 1 only build step 1d writes it (kills);
steps 3-4 write the rest. A character played by hand before has an empty log: the giver's answer
corrects it the first time it is asked (§4.3).

### 2.3 States

```rust
pub enum QuestState {
    Off,                                     // switched off, or quests off
    Ready,                                   // hand-in now: items carried, kills done, a talk open
    Needs { more: u32, of: String },         // "needs 3 more Red Rat Tail" / "7 more Blood Shreth"
    NotTaken { from: String },               // not taken up yet: talk to San Ming first
    WontComplete { rule: String, does: LootAction }, // step 2: a rule above the quest rule
    CantYet(Blocked),                        // can't do yet, and why
    Waiting { until: u64 },                  // done; may be done again then
    Done,                                    // a once-quest done
}
pub enum Blocked { Level { min: u32, max: u32 }, TooHard { level: u32 },
                   Locked { flag: String }, NeedsFirst { title: String } }
```
`QuestLine { id, title, kind, state, xp: u64 (at the character's level), givers: usize, giver:
String (nearest by straight distance), on: bool }`. Read, for one character:
- `Off` unless `config.quests.on` and the id is not in `off`.
- `CantYet`: no row's Level band holds the level (`Level`); the easiest source creature is above
  `level + 2 * growth.level_margin` (`TooHard`, the bound `Ground::suits` uses for a stray); every
  row's giver is `locked_by` the held flags (`Locked`); a `needs` flag is not held and is stamped
  only by a quest that is not a start row (`NeedsFirst`, naming it).
- `Done` / `Waiting` from the log: a once-quest done; `waits[timer]` in the future.
- `NotTaken`: a `needs` flag a start row stamps is not held (a kill task's counter is not in
  `log.kills`), naming the nearest start giver; else kill: `Needs` or `Ready` from the count.
- item: `already_carried(wcid)` (autoplay/loot/judge.rs:242) against `count`; talk: `Ready`.

States are worked out by a pure fn, `quest_board(rows, &Snapshot) -> Vec<QuestLine>`, whose
`Snapshot` holds level, XP to the next level, held flags, the log, carried counts by wcid, margin,
config (and in step 2 the profile): testable offline with no `Client`. `Client::refresh_quests(now)`
runs it into `Client.quest_board` every `QUEST_BOARD_EVERY` (2 s) and at once when
`config.quests`, the level or the log changes, called from `Client::tick` beside `tick_retag()`
(tick path item 3). It sends nothing and decides nothing.

### 2.4 The panel

A **Quests** section in the autoplay window (`crates/ac-plugin/src/panels/autoplay.rs`), between
"Loot" (:460) and "Team" (:509), drawn by a new `crates/ac-plugin/src/panels/quest_list.rs` so
autoplay.rs stays short. `AutoplayView` gains `quests: Vec<QuestLine>` (from `c.quest_board`).
- "do quests" checkbox (`cfg.quests.on`), hover: keep what the quests switched on want and hand it in.
- A search box (title, giver, item) and a show-picker: "for my level" (default: not `CantYet` by
  level or too hard), "switched on", "all".
- One line per quest in a scroll area (about 220 px): its on/off box (writes `cfg.quests.off`), the
  title, "to San Ming" or "any of 12 Collectors", the XP now, and the state in words, coloured:
  ready green, won't complete orange (the rule named), can't do yet and waiting grey.
- A count line: "40 on, 3 ready, 1 won't complete".
- Demo view (`Autoplay::demo`) shows a few lines of each state. Tests in autoplay.rs's module:
  the quests settings survive a restart; a settings file without `quests` loads with them off.

### 2.5 Build steps of step 1

Each its own commit, gates green (fmt, clippy, targeted tests; before 1d's commit the full
`cargo test-data` and `RUSTDOCFLAGS=-Dwarnings cargo doc --workspace --no-deps`).
1. **1a, data.** `quests.sh`, `quests_walk.py`, both CSVs, `ac_world::quests` with its tests;
   `lib.rs` `pub mod quests`; the root map's ac-world row says "quests". `cargo test -p ac-world quests`.
2. **1b, config, log and states.** `autoplay::Quests` in config.rs, `quest_log.rs` (log, load,
   save, `quest_board`, `refresh_quests`), the call in `Client::tick`. Tests: each state from a
   built snapshot (the three examples); an old config loads. `cargo test -p ac-client quest`.
3. **1c, the panel.** quest_list.rs, the section, `AutoplayView.quests`, demo. `cargo test -p ac-plugin autoplay`.
4. **1d, the kill count.** `ac_agent::refusals::kill_count(text) -> Option<KillCount { done, of:
   Option<u32>, plural }>` beside `refused` (QuestManager.cs:619-624; the module doc gains a line:
   the one progress line, kept with the refusals so server English is in one file), a test in the
   exact words, and its hand-off from `chat_message` (session/chat.rs) to `QuestLog::kills`: the
   started kill task whose source creatures' names the plural begins with (the singular less its
   last letter, for "Harpies"), else the only one started. Observing only.

Maps: `quest_log.rs` joins the `recalls.rs`, `quest_flags.rs` row of ac-client's "Other modules";
the glossary gains one row, `quest, giver, hand_in` (a quest in `ac_world::quests`, its rows
sharing an id; the NPC who rewards it; giving it the items, or talking, for the reward | task,
npc, turn-in). Both maps are at their budgets (`ROOT_LINES` 121, `CLIENT_LINES` 200,
tests/code_map.rs): the line is paid for by merging the `options.rs, emotes.rs` and `position.rs`
rows. The code map row edits go in the commit of the file they name (code_map.rs reads them).

## 3. Step 2: the loot rule, the default rule, the cross-check

### 3.1 The condition

Conditions are `ac_loot::profile::Ask` (profile.rs:200), evaluated by `holds()` (:427) against
`ItemStats`, an optional appraisal, the `Wielder` (the character's snapshot: weapons.rs:31) and
its name; the search language (`items::Query`) reads the item alone, so the condition is an `Ask`,
not a search `Term`. It stays a pure function of what is handed in:
- `crates/ac-loot/src/quest_wants.rs` (leaf unique): `pub struct QuestWant { pub wcid: u32,
  pub need: Option<u32> }` (`None`: every one is wanted) and `pub fn wanted(wants, wcid, held) ->
  bool`: some want has the wcid and `need` is `None` or `held < need`.
- `Wielder` gains `pub quest_wants: Vec<QuestWant>` (default empty).
- `Ask::Quest`, a unit variant ("wanted by an active quest"): `holds` is
  `wanted(&me.quest_wants, item.wcid, held)`; `needs_id` false (the wcid comes with the item);
  `tell` "wanted by an active quest". `held` is already what `judge` passes for capped rules, so
  `holds` and `cheap_half_holds` take `held` too.
- The wants come from the board: every item quest whose state is `Ready`, `Needs` or
  `WontComplete` (on, not blocked, not waiting or done), one want per item its rows take. `need` =
  `count` for a timed or once quest, `count * ANY_HAND_INS` (5) for `Repeat::Any`: each trophy is a
  slot and a giver is met only on a run that passes. Summed per wcid. A mate is told the wants less
  what is carried (`Client::quest_wants_left`).
  `Client::wielder()` (actions/combat.rs:262) fills it from `Client.quest_board`.
- A quest switched off, or quests off, wants nothing: its items fall to the rules below.
- Old profiles carry no `Quest` ask and load unchanged; a profile saved with one does not load in
  an older acswarm.

The ledger: tags are final when written (ledger.rs), and `tick_retag` re-judges the pack when the
rules' fingerprint changes. `rules_fingerprint` (autoplay/ledger/retag.rs:195) hashes the sorted
wants too, so switching a quest, quests on or off, a once-quest done or a wait starting re-judges
what is carried. `in_arrival_order` already hands each item the count before it, so a want of 5
keeps the first five and the rest go to the rules below, as a `keep_up_to` does.

### 3.2 The default rule

`Profile::starter()` (profile.rs:851) gains, first:
```rust
Rule { name: "quest items".into(), group: "Quests".into(), action: LootAction::Keep,
       all: vec![Ask::Quest], ..Default::default() }
```
First, because "nothing cheap" (Skip, value <= 250) would skip every trophy (a Red Rat Tail is
worth 2). An ordinary rule: the player moves it, turns it off, or puts a rule above it. Saved
profiles are not edited; for them the cross-check says "no rule keeps it" and the Quests section
offers "add the quest rule at the top", which puts this rule first through `Library::put`. The
editor (`crates/ac-plugin/src/panels/loot_profiles.rs`, `AskKind`, :351-389) gains a "quest" kind
with no fields, inside the file's 1,800-line ceiling.

### 3.3 The cross-check: "won't complete: rule X"

For each item quest wanted (§3.1), the board judges the item as the looting would, in
`judge_loot`'s order (autoplay/loot/judge.rs:191): an `ItemStats` built from the row (`wcid`,
`name`, `item_type`, `kind_name`, `value`, stack 1, `appraised: true` with no appraisal, so a rule
that needs one does not claim it), the character's `Wielder` with the wants, held = carried.
- the never list names it: `WontComplete { rule: "never take these", does: Skip }`;
- decided by a rule with `Ask::Quest` in it, or by any Keep rule: fine (the line says which);
- decided by a Sell, Salvage or Skip rule: `WontComplete { rule, does }`, the line says
  "won't complete: 'nothing cheap' skips it; move the quest rule above it";
- no rule claims it: `WontComplete { rule: "no rule keeps it", does: Skip }`.
A `WontComplete` quest still wants its item (the rule may be moved), so its line can name the rule.

### 3.4 Party looting

A mate who opens the body may not have the quest. `Mate` (autoplay/team/view.rs) gains
`quest_wants: Vec<QuestWant>` (`#[serde(default)]`), set from the board each board round;
`TeamView::wielder` (:162) carries it; `claim_on` (autoplay/loot/judge.rs:39) gives a rule with
`Ask::Quest` a `Calling::Quest`, and `called_to` sends the item to the first taker (by name) whose
wants hold it, the way a skill rule goes to the best at its skill.

### 3.5 Build steps of step 2

1. **2a, ac-loot.** quest_wants.rs, `Wielder.quest_wants`, `Ask::Quest` (holds, needs_id, tell),
   the starter's rule. Tests (`cargo test -p ac-loot quest`): wanted while fewer are held than the
   need; every one for `None`; a rule above overrules it (the profile decides); nothing wanted, the
   rules below decide; the starter keeps a Red Rat Tail and still skips a cheap rag.
2. **2b, ac-client and the panel.** Wants from the board into `wielder()`, the fingerprint, the
   cross-check state, the editor's kind, the "add the quest rule" button. Tests: a quest switched
   off re-tags its carried items to the rules below; a Skip rule above names itself; a mule without
   the rule reads "no rule keeps it".
3. **2c, the party.** `Mate.quest_wants`, `Calling::Quest`. Test: a tail on a body a mate opens is
   called to the character with the quest. Full `cargo test-data` and docs before this commit.

## 4. Step 3: kill and talk quests

### 4.1 The reward in the ground picker

The picker ranks by `Ground::worth()` (hunting.rs:196): XP per point of health taken off, scaled
by how busy the ground is, and `best_ground` (autoplay/growth/hunt.rs:505) takes the best a
journey reaches. A quest adds XP to the ground's kills, in the same units:
- per kill of a kill task's creature: `xp_at(level) / N`, while kills remain (taken up or ready to
  be taken up, not waiting);
- per kill of an item's source: `chance * xp_at(level) / count`, while the want holds;
- a ground's bonus is the sum over quests of that per-kill XP times the quest creatures' share of
  the ground: their spawn entries in the landblock (`Source::landblocks`) over `Ground::count`.
`Ground::worth_with(extra: f32)` = `(xp + extra) / health * c / (c + HALF_BUSY)`, and
`ranked_for` takes the bonus as `impl Fn(&Ground) -> f32` (0 keeps today's order). The client
works the bonus out per landblock from the board, once per board round.

### 4.2 The giver as a stop

`grow_hunt` already visits a Warden before choosing a ground (`unlock_to_earn`, hunt.rs:454). The
quest calls go in the same place, the same way (`visit_landmark` with an Npc `Landmark`, as
`visit_unlock` builds one): a start talk for a kill task that is on and whose creatures suit the
level, a talk quest `Ready`, a kill task `Ready`. One at a time, the nearest by
`journey_seconds` (hunt.rs:526), and only when it pays (§5.3's rule: a short leg, or its XP beats
the journey at the character's XP per hour; a start counts the task's XP). A giver no journey
reaches is skipped for `UNLOCK_RETRY` like a Warden, and its line says "no way to San Ming".
Visits keep their cells (`Visit.cell`, phase 4's journeys), so a giver in a dungeon is planned to
like a counter.

### 4.3 What the use came to

Built as `Client::watch_quest_answer`. The use is sent by the visit; `quest_log` watches
`XP_WATCH` (10 s, guess) for the answer (the start talk and the words are not built):
- the total XP rises: done; `done[id]`, `waits[timer] = now + repeat` (timed), the row's `stamps`
  into `quests_done`; a kill counter is forgotten (ACE erased it);
- a start talk: the counter is taken up (`kills[counter] = 0`), and the first kill line confirms it;
- "You may complete this quest again in 19h 59m." (§5.4): `waits[timer]` from the words;
- nothing: the giver gave its "done" reply in its own words (65 of 149 timed quests), so a once
  quest is `Done` and a timed one waits its whole `repeat` from now (the safe side); an item the
  giver did not take, or an any-time quest unpaid, leaves the giver alone for `GIVER_RETRY`.

## 5. Step 4: item hand-ins and the run with many calls

### 5.1 Handing in

`Client::give(target, item, amount)` (actions/items.rs:448) hands an item over. At the giver
(within `visit::REACH`), the hand-in gives the wanted item one at a time (ACE takes one per give:
Player_Inventory.cs:3441-3443), the next once the last has left the pack and the XP came, or after
`GIVE_PAUSE` (3 s). A Refuse-keyed row (`count` > 1) is handed one item, which comes back with
WeenieError 0x04CD (expected) as the NPC takes `count` by TakeItems. WeenieErrorWithString 0x04CE
(busy with an emote) resends after the pause; 0x03EF (not accepting gifts) ends the call. Done
when the want is met or the items are gone; §4.3's watch reads each.

### 5.2 Calls

The town run (`autoplay/growth/town_run/mod.rs`) visits up to `STOPS_PER_RUN` (3) counters, each
chosen on arrival at the last (`choose_stop`, :273; `grow_run_next`, counter.rs:574), and a counter
is a `Vec2`. It becomes a list of **calls** planned when the run starts: `Call { at: Place, on:
CallOn }`, `CallOn::Counter { errand, vendor }` or `CallOn::Giver { quest, giver_wcid }`. The code
keeps the name town run: the glossary forbids "errand" for it, and the plan's "errand run" is this
run with givers among its calls. Counter calls are chosen up front from the shop lists as
`pick_vendor` forecasts them now (a forecast needs no walk), with their cells from `vendors.csv`.
`Run` gains `calls: Vec<Call>` and `next: usize`; `Phase` gains `HandingIn { giver, sent }`.

### 5.3 Order and the detour rule

`crates/ac-world/src/tour.rs` (leaf unique): `tour(from, calls, end, cost) -> Option<(Vec<usize>,
f32)>`, where `cost(a, b) -> Option<f32>` is a journey's seconds and `end` is the ground to go back
to. With at most 6 calls every order is tried (720) and the fastest kept; past 6, nearest first.
The client fills a cost matrix once per run start, each pair planned as `journey_seconds` plans
(`trip::plan_with_recalls_and_gems`: portals, recalls, `/lifestone`, gems, the town network), at
most 42 plans. A pair no plan joins is no edge; a giver no order reaches is refused when the run is
planned, its line saying "no way to <giver>", and tried again after `UNLOCK_RETRY`.

A ready hand-in joins the run when either:
- its giver is in a landblock the run calls at anyway, or `cost(call, giver) <= SHORT_LEG` (60 s,
  guess) from one; or
- `xp_at(level) * hand-ins carried >= detour * xp_per_hour / 3600`, where `detour` is the tour
  with it less the tour without it.
Hand-ins are tried best `xp / detour` first, each against the tour as it stands, until 6 calls.
One left out waits: its items stay kept (the want holds), and the next run that passes takes it.
Hand-ins never start a run, except a pursued quest's own call (`start_quest_call`): a town run is
made only for a full pack, too much weight or supplies to buy, and a run that sold and bought
nothing waits `FUTILE_RUN_WAIT` whatever it handed in (a pursued quest's call never does).
Until the rate is measured, only the first rule applies, and "a short leg" also needs the detour
to be at most `2 * SHORT_LEG`, so a cheap recall back makes no far giver near.

`xp_per_hour`: `growth::State` keeps the total XP sampled every minute for the last 30 minutes
(the fleet window works its rate the same way, `panels/fleet.rs:666`); none until 10 minutes are
in.

### 5.4 The "you must wait" refusal

A row in `ac_agent::refusals`, with a test in the exact words:
```
// Managers/QuestManager.cs:575, WorldObjects/Managers/EmoteManager.cs:1825 (the `%tqt` template)
//   $"You may complete this quest again in {remainStr}."   (TimeSpanExtensions.cs:8-33: "1d 2h 3m 4s")
// Managers/QuestManager.cs:565
//   "You have solved this quest too many times!"
```
`Refusal::Quest { why: QuestRefusal::TooSoon { left: Duration } }` and `QuestRefusal::Never`;
`answer()` gives `Again(left)` and `Never`. `hear_refusal` (refused.rs:26) hands it to the quest
log, which applies it to the giver of the call or visit in flight: `waits[timer] = now + left`, or
`Done`. The words name no quest, so a line with no call or visit in flight is dropped. The same
words come when a quest-stamped item on someone else's corpse cannot be picked up yet
(Player_Inventory.cs:1249-1262, with WeenieError 0x043E); the loot run's take refusal already
leaves such an item, and the hand-off ignores the line when nothing is handed in.

### 5.5 Building steps 3 and 4

Step 3: the bonus (`worth_with`, `ranked_for`'s bonus, the client's per-landblock sum), the quest
calls in `grow_hunt`, §4.3's watch and the refusal row. Tests offline: a ground where a kill task's
creature spawns outranks a slightly better one; a start talk is visited before the ground; the
wait words set the wait. Step 4: `Call`, `tour.rs` and its tests (every order of 5 calls against a
matrix; a giver no edge reaches is refused), the hand-in phase, the detour rule with `xp_per_hour`,
`worth_town_run`'s term. A systems row (`quests`) joins the ac-client map in step 3; the town run
row names `tour.rs` in step 4. Neither touches movement code; if a build does (the visit's reach),
the route harness bar of `docs/movement-plan.md` applies.

## 6. Left for later

Items picked up off the ground (chests, generators' items: 749 hand-ins wait for it); the 4
two-item quests; yes/no confirmations (InqYesNo, 2 quests); quests gated by quest bits, society
rank or skills; a Rhai call reading the board (it would migrate scripts, validate and the plugin
template, and none is needed for steps 1-4).

## 7. Seams (built on branch `quests` before the four builders start)

The foundation (`675944a` data, `de9a2de` seams) built what more than one builder reads, so no
builder needs another's code. Builders: **list** (step 1: 1b states, 1c panel, 1d kill line),
**loot** (step 2), **hunt** (step 3), **errand** (step 4). The generated data differs a little from
§1.2's prototype counts: 1,961 rows (1,779 item, 79 kill, 19 talk, 84 start), 471 quests (386
item, 75 kill, 10 talk), 264 givers; `quest_sources.csv` 1,932 rows (1,620 drop, 301 kill, 11 npc).
Those were the XP-only files; §1.2 now counts every quest. `quests_walk.py` prints its counts and
what it could not read on every run.

| seam | file | state | filled by |
|---|---|---|---|
| `Row`, `Kind`, `Handing`, `Want`, `Repeat`, `Share`, `Reward::xp_at`, `Item`, `Source`, `Yield`, `all`, `sources`, `rows_of`, `sources_of`, `stamped_by`, `Row::fits_level`, `Row::locked_by` | `crates/ac-world/src/quests.rs` | built, tested | - (read by all) |
| `autoplay::Quests { on, off }`, `Quests::is_on`, `Config.quests` | `crates/ac-client/src/autoplay/config.rs` | built, tested | - |
| `QuestWant { wcid, need }`, `wanted()` | `crates/ac-loot/src/quest_wants.rs` | built, tested | loot adds `Wielder.quest_wants`, `Ask::Quest`, the starter rule |
| `QuestLog` (`note_taken`, `note_kills`, `note_done`, `note_wait`, `wait_until`), `Solved`; `load_quest_log`, `Client::save_quest_log` | `crates/ac-client/src/quest_log.rs` | built, tested | list (1d) writes kills; hunt and errand write done and waits (§4.3) |
| `QuestState`, `Blocked`, `QuestLine`, `QuestFacts`, `quest_board()` | `crates/ac-client/src/quest_log.rs` | Off, Ready, Needs, NotTaken, Waiting, Done | list: `CantYet` (level band, too hard, locked, needs first), refresh on a change (§2.3) |
| `Client::quest_facts`, `Client::refresh_quests` (every `QUEST_BOARD_EVERY` from `Client::tick`), `Client.quest_board`, `Client.quest_log` | `crates/ac-client/src/quest_log.rs`, `crates/ac-client/src/session/apply.rs` | built | list (refresh triggers), loot (profile into the facts is there already) |
| `Client::quests_on()`: the ids autoplay considers | `crates/ac-client/src/quest_log.rs` | built | - |
| `quest_item_blocked(profile, item, me, name, held) -> Option<(rule, LootAction)>`: the cross-check, called by `quest_board` for every item quest on | `crates/ac-client/src/autoplay/loot/judge.rs` | returns `None` | loot (§3.3) |
| `quest_wants(board) -> Vec<QuestWant>`: what the board wants carried (§3.1) | `crates/ac-client/src/quest_log.rs` | built, tested | loot wires it into `wielder()`, `rules_fingerprint`, `Mate` |
| `Client::visit_giver(&Row)`: a visit to a giver, journey and all | `crates/ac-client/src/quest_log.rs` | built | hunt (start talks, ready talks and kill tasks), errand (hand-in calls) |
| `Client.quest_asked: Option<Asked { row, at, xp_before, carried }>`: the giver asked, awaiting its answer | `crates/ac-client/src/lib.rs`, `crates/ac-client/src/quest_log.rs` | built: `ask_giver`, `watch_quest_answer` | the town run's calls set it in reach of a giver and at each give |
| `Client::xp_per_hour()` | `crates/ac-client/src/quest_log.rs` | returns `None` | hunt (§5.3's 30-minute samples) |
| `worth_detour(xp, detour, xp_per_hour)`, `SHORT_LEG` | `crates/ac-client/src/quest_log.rs` | built, tested | - (hunt §4.2, errand §5.3) |
| "you must wait" refusal row and its hand-off to `quest_asked` | `crates/ac-agent/src/refusals.rs`, `crates/ac-client/src/refused.rs` | not built | hunt (§5.4); errand relies on it |
| kill line `kill_count()` | `crates/ac-agent/src/refusals.rs`, `crates/ac-client/src/session/chat.rs` | not built | list (1d) |
| Quests section, `AutoplayView.quests`, demo lines | `crates/ac-plugin/src/panels/quest_list.rs`, `crates/ac-plugin/src/panels/autoplay.rs` | not built | list (1c) |
| `Ground::worth_with`, `ranked_for`'s bonus, the per-landblock sum | `crates/ac-world/src/hunting.rs`, `crates/ac-client/src/autoplay/growth/hunt.rs` | not built | hunt |
| `tour()`, `Call`, `CallOn`, `Phase::HandingIn` | `crates/ac-world/src/tour.rs`, `crates/ac-client/src/autoplay/growth/town_run/mod.rs` | not built | errand |

Maps: the client map already has the `quests` systems row, `quest_log.rs` in the
`recalls.rs`, `quest_flags.rs` row and the `quest, giver, hand_in` glossary row; the root map's
ac-world and ac-loot rows say quests. A builder adding a file adds only its own row (the town run
row names `tour.rs`; ac-plugin has no module map).
