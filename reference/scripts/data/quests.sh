#!/bin/sh
# Regenerate crates/ac-world/data/quests.csv, quest_givers.csv and
# quest_sources.csv: every quest a placed NPC holds, XP or not (each path
# that does anything for the character), what each asks and does, what
# the client cannot check, where its givers stand, and where what it asks
# comes from.
#
# Quest logic is server-side, in emote sets (weenie_properties_emote) and
# their actions (weenie_properties_emote_action); a branching action runs
# the set of its outcome whose quest key is the action's message
# (GetEmoteSet, WorldObjects/Managers/EmoteManager.cs:1512-1556). The
# walk through those sets is a graph search, done by quests_walk.py
# (stdlib Python) on tables read out here: placed objects with a Refuse,
# Give or Use set, their sets and actions, the quest table, kill counters
# on creatures (PropertyString 45, 49, 50), a wanted item's pickup limit and
# the flag its pickup asks (33, 37), who takes a gift (AllowGive,
# PropertyBool 8: Player_Inventory.cs:3439), corpse create lists (Contain,
# or Treasure without Wield: Creature_Death.cs:688-693), names, levels,
# item types and values, and spawn entries per creature and landblock
# with hunting.sh's filters (attackable, with a level; a landblock with a
# surface counts its outdoor spawns only); and for each wanted item the
# generators making it on the ground and the chests holding it, with the
# keys a locked one needs, and the placed objects a recipe makes it on from
# an item used on them. docs/quests-design.md §1 says what the
# walk reads and what it leaves out.
#
# This reads the local ACE world database (the community's
# reconstruction of retail's server data) running in Docker, and is only
# needed when that data changes. Every query is capped at 20 s.
#
# Usage: reference/scripts/data/quests.sh crates/ac-world/data
set -e

OUT=${1:?usage: quests.sh OUT_DIR}
HERE=$(cd "$(dirname "$0")" && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

q() {
  docker exec ace-db sh -c \
    "mysql -uroot -p\"\$MYSQL_ROOT_PASSWORD\" -N --batch ace_world -e 'SET SESSION max_execution_time=20000; $2'" \
    2>/dev/null >"$TMP/$1.tsv"
}

q placed 'select li.weenie_Class_Id, hex(li.obj_Cell_Id), round(li.origin_X, 2),
  round(li.origin_Y, 2), round(li.origin_Z, 2), w.type
from landblock_instance li join weenie w on w.class_Id = li.weenie_Class_Id
where li.weenie_Class_Id in
  (select object_Id from weenie_properties_emote where category in (1, 6, 7));'

q emote 'select id, object_Id, category, probability, coalesce(weenie_Class_Id, 0),
  coalesce(quest, "")
from weenie_properties_emote
where object_Id in (select weenie_Class_Id from landblock_instance)
order by object_Id, id;'

q action 'select a.emote_Id, a.order, a.type, coalesce(a.message, ""), coalesce(a.min, ""),
  coalesce(a.max, ""), coalesce(a.amount, ""), coalesce(a.amount_64, ""),
  coalesce(a.percent, ""), coalesce(a.min_64, ""), coalesce(a.max_64, ""),
  coalesce(a.stat, ""), coalesce(a.weenie_Class_Id, ""), coalesce(a.stack_Size, "")
from weenie_properties_emote_action a
where a.emote_Id in (select id from weenie_properties_emote
  where object_Id in (select weenie_Class_Id from landblock_instance))
order by a.emote_Id, a.order;'

q quest 'select name, min_Delta, max_Solves, coalesce(message, "") from quest;'

q killq 'select object_Id, value from weenie_properties_string where type in (45, 49, 50);'

q drops 'select cl.object_Id, cl.weenie_Class_Id, cl.destination_Type, cl.shade, cl.stack_Size
from weenie_properties_create_list cl
join weenie w on w.class_Id = cl.object_Id and w.type = 10
where cl.weenie_Class_Id <> 0
  and ((cl.destination_Type & 1) <> 0
    or ((cl.destination_Type & 8) <> 0 and (cl.destination_Type & 2) = 0))
order by cl.object_Id, cl.id;'

q names 'select object_Id, value from weenie_properties_string where type = 1;'

# An item's own pickup limit (Quest 33) and the flag picking it up asks (QuestRestriction 37):
# Player_Inventory.cs:1207-1211, 1249-1262.
q itemq 'select object_Id, type, value from weenie_properties_string where type in (33, 37);'

q allowgive 'select object_Id from weenie_properties_bool where type = 8 and value = 1;'

q ints 'select object_Id, type, value from weenie_properties_int where type in (1, 19, 25);'

q spawns 'with recursive spawn(inst, wcid, depth) as (
  select li.guid, li.weenie_Class_Id, 0 from landblock_instance li
  union all
  select s.inst, g.weenie_Class_Id, s.depth + 1 from spawn s
  join weenie_properties_generator g on g.object_Id = s.wcid
  where s.depth < 4
    -- an entry put in a container, a hand or a shop (where_Create Contain, Wield, Shop:
    -- RegenLocationType.cs:12-16) puts nothing in the world, as the guardians a chest holds
    and (g.where_Create & 56) = 0
),
surface as (
  select distinct obj_Cell_Id >> 16 lb from landblock_instance
  where (obj_Cell_Id & 65535) < 256
),
mon as (
  select distinct s.inst, s.wcid, li.obj_Cell_Id >> 16 lb,
    if((li.obj_Cell_Id & 65535) < 256, "o", "d") kind
  from spawn s
  join landblock_instance li on li.guid = s.inst
  join weenie c on c.class_Id = s.wcid and c.type = 10
  join weenie_properties_int lv on lv.object_Id = c.class_Id and lv.type = 25
  left join weenie_properties_bool b on b.object_Id = c.class_Id and b.type = 19
  where coalesce(b.value, 1) = 1
)
select m.wcid, hex(m.lb), count(*) from mon m
where m.kind = "o" or m.lb not in (select lb from surface)
group by m.wcid, m.lb;'

# What lies about and what makes it: placed objects but creatures and buildings (WeenieType
# Portal, Door, Vendor, HotSpot, Creature, LifeStone, House, SlumLord, Hook, Storage, BootSpot,
# HousePortal), every weenie's type, and every generator profile.
# A placement linked to a generator (landblock_instance_link) is never put out itself
# (WorldObjectFactory.cs:280): its generator makes it there (WorldObject_Generators.cs:599-618).
q ground 'select li.weenie_Class_Id, hex(li.obj_Cell_Id), round(li.origin_X, 2),
  round(li.origin_Y, 2), round(li.origin_Z, 2),
  exists(select 1 from landblock_instance_link l where l.child_Guid = li.guid)
from landblock_instance li join weenie w on w.class_Id = li.weenie_Class_Id
where w.type not in (7, 10, 12, 13, 19, 25, 53, 55, 56, 57, 58, 59);'

q wtypes 'select class_Id, type from weenie;'

q gens 'select object_Id, probability, weenie_Class_Id, where_Create, hex(coalesce(obj_Cell_Id, 0)),
  round(coalesce(origin_X, 0), 2), round(coalesce(origin_Y, 0), 2), round(coalesce(origin_Z, 0), 2)
from weenie_properties_generator where weenie_Class_Id <> 0 order by object_Id, id;'

# A chest's create list, its lock (Locked, PropertyBool 3; LockCode, PropertyString 12), every
# key's KeyCode (PropertyString 13) and what is Stuck (PropertyBool 1, never picked up).
q chestlist 'select cl.object_Id, cl.weenie_Class_Id, cl.destination_Type
from weenie_properties_create_list cl
join weenie w on w.class_Id = cl.object_Id and w.type in (20, 21)
where cl.weenie_Class_Id <> 0;'

q locks 'select b.object_Id, coalesce(s.value, "") from weenie_properties_bool b
left join weenie_properties_string s on s.object_Id = b.object_Id and s.type = 12
where b.type = 3 and b.value = 1;'

q keys 'select s.object_Id, s.value from weenie_properties_string s
join weenie w on w.class_Id = s.object_Id and w.type = 22 where s.type = 13;'

q stuck 'select object_Id from weenie_properties_bool where type = 1 and value = 1;'

# What a carried item used on a placed object makes (RecipeManager.UseObjectOnTarget,
# Managers/RecipeManager.cs:43-130): the made item, the item used and the object.
q recipes 'select r.success_W_C_I_D, c.source_W_C_I_D, c.target_W_C_I_D
from recipe r join cook_book c on c.recipe_Id = r.id
where r.success_W_C_I_D <> 0
  and c.target_W_C_I_D in (select weenie_Class_Id from landblock_instance);'

# And what one carried item used on another makes (the same recipes, the target an item carried:
# a town stamp on a Lucky Gold Letter): the made item, the item used and the item it is used on.
q combines 'select r.success_W_C_I_D, c.source_W_C_I_D, c.target_W_C_I_D
from recipe r join cook_book c on c.recipe_Id = r.id
where r.success_W_C_I_D <> 0
  and c.target_W_C_I_D not in (select weenie_Class_Id from landblock_instance);'

for t in placed emote action quest killq itemq drops names allowgive ints spawns ground wtypes \
  gens chestlist locks keys stuck recipes combines; do
  [ -s "$TMP/$t.tsv" ] || { echo "quests.sh: the $t query returned nothing" >&2; exit 1; }
done

python3 -I "$HERE/quests_walk.py" "$TMP" "$OUT"
