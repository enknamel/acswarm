#!/bin/sh
# Regenerate crates/ac-world/data/hunting.csv: the landblocks where
# monsters spawn, outdoors and in dungeons, with the levels of what
# spawns there and what a kill of it is worth.
#
# Monsters are server-side: a landblock holds generator objects, each of
# which spawns creatures from its list (weenie_properties_generator),
# and a generator can spawn other generators, so the list is walked to
# a depth of four. A creature counts when it is attackable and has a
# level. Per landblock this gives how many spawn entries there are, the
# lowest and highest level among them, the level and name of the
# creature that appears most often, the average XP a kill awards
# (XpOverride, Creature_Death.cs:193) and the average health to take
# off, divided by the creature's weakest resistance (health is the
# Health vital plus half of Endurance, as the vital formula has it),
# both weighted by how many each generator keeps up (max_Create, the
# product down a chain of generators).
#
# An outdoor row ("o") is the open ground of a landblock: x/y are the
# average spawn point. A dungeon row ("d") is a landblock with nothing
# outdoors at all; its way in is read off the portals at run time.
#
# This reads the local ACE world database (the community's
# reconstruction of retail's server data) running in Docker, and is only
# needed when that data changes.
#
# Usage: reference/scripts/data/hunting.sh > crates/ac-world/data/hunting.csv
set -e

SQL='with recursive spawn(inst, wcid, depth, n) as (
  select li.guid, li.weenie_Class_Id, 0, 1 from landblock_instance li
  union all
  select s.inst, g.weenie_Class_Id, s.depth + 1, s.n * greatest(g.max_Create, 1) from spawn s
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
  select s.inst, s.wcid, li.obj_Cell_Id >> 16 lb,
    if((li.obj_Cell_Id & 65535) < 256, "o", "d") kind,
    li.origin_X x, li.origin_Y y, lv.value lv, nm.value nm, max(s.n) n
  from spawn s
  join landblock_instance li on li.guid = s.inst
  join weenie c on c.class_Id = s.wcid and c.type = 10
  join weenie_properties_int lv on lv.object_Id = c.class_Id and lv.type = 25
  join weenie_properties_string nm on nm.object_Id = c.class_Id and nm.type = 1
  left join weenie_properties_bool b on b.object_Id = c.class_Id and b.type = 19
  where coalesce(b.value, 1) = 1
  group by s.inst, s.wcid, li.obj_Cell_Id, li.origin_X, li.origin_Y, lv.value, nm.value
),
kept as (
  select m.* from mon m
  where m.kind = "o" or m.lb not in (select lb from surface)
),
worth as (
  select w.class_Id wcid,
    coalesce((select value from weenie_properties_int where object_Id = w.class_Id and type = 146), 0) xp,
    (coalesce((select init_Level from weenie_properties_attribute_2nd where object_Id = w.class_Id and type = 1), 0)
      + round(coalesce((select init_Level from weenie_properties_attribute where object_Id = w.class_Id and type = 2), 0) / 2))
    / greatest(
        coalesce((select max(value) from weenie_properties_float where object_Id = w.class_Id and type between 64 and 70), 1),
        if((select count(*) from weenie_properties_float where object_Id = w.class_Id and type between 64 and 70) < 7, 1, 0),
        0.1) hp
  from weenie w where w.class_Id in (select distinct wcid from kept)
),
bynm as (select lb, kind, nm, lv, count(*) n from kept group by lb, kind, nm, lv)
select hex(m.lb), round(avg(m.x), 0), round(avg(m.y), 0), count(*),
  min(m.lv), max(m.lv),
  (select b.lv from bynm b where b.lb = m.lb and b.kind = m.kind order by b.n desc, b.lv, b.nm limit 1),
  (select b.nm from bynm b where b.lb = m.lb and b.kind = m.kind order by b.n desc, b.lv, b.nm limit 1),
  round(sum(w.xp * m.n) / sum(m.n), 0), greatest(round(sum(w.hp * m.n) / sum(m.n), 0), 1),
  sum(m.n), m.kind
from kept m join worth w on w.wcid = m.wcid
group by m.lb, m.kind
having m.kind = "o" or (sum(m.n) >= 10 and sum(w.xp * m.n) > 0);'

cat <<'HEADER'
# Hunting grounds of Dereth: the landblocks where monsters spawn.
# landblock,x,y,count,min_level,max_level,level,name,xp,health,creatures,kind
# The landblock is hex; x/y are the average spawn point, local to it.
# count is the number of spawn entries, min/max the level range among
# them, and level/name the creature that spawns most often there. xp
# is the average a kill awards, health the average health to take off
# (over the weakest resistance), creatures how many the generators keep
# up at once (their max spawns). kind is o for open ground, d for a
# dungeon (a landblock with nothing outdoors; its way in is a portal).
# Regenerate with reference/scripts/data/hunting.sh (reads the ACE
# world database, the community's reconstruction of retail's server
# data; the client has no copy of its own).
HEADER

docker exec ace-db sh -c \
  "mysql -uroot -p\"\$MYSQL_ROOT_PASSWORD\" -N --batch ace_world -e '$SQL'" \
  2>/dev/null |
  awk -F'\t' 'NF == 12 {
    gsub(/,/, ";", $8)
    printf "%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s,%s\n", $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12
  }' |
  LC_ALL=C sort -t, -k1,1
