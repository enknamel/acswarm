#!/bin/sh
# Regenerate crates/ac-world/data/spawns.csv.gz: what spawns where, outdoors
# and in every dungeon room, for the map to show.
#
# Monsters are server-side: a landblock holds generator objects, each of
# which spawns creatures from its list (weenie_properties_generator), and
# a generator can spawn other generators, so the list is walked to a
# depth of four (as reference/scripts/data/hunting.sh does). A creature
# counts when it is attackable and has a level.
#
# One row per cell and creature (weenie class): outdoors the cell is the
# outdoor cell the generators stand in, indoors the dungeon room. x/y/z are
# the average position of those generators, local to the landblock, and
# count is how many generator entries spawn that creature there. Two kinds
# can share a name (the Catacombs of Ithaenc's key-carrying Putrid Moarsman,
# 8866, and its thirteen others, 4248), so the class is kept beside it.
#
# Roughly a hundred thousand rows, so the table is kept gzipped and read
# once when first asked for.
#
# This reads the local ACE world database (the community's reconstruction
# of retail's server data) running in Docker, and is only needed when that
# data changes.
#
# Usage: reference/scripts/data/spawns.sh > crates/ac-world/data/spawns.csv.gz
set -e

SQL='set session max_execution_time=20000; with recursive spawn(inst, wcid, depth) as (
  select li.guid, li.weenie_Class_Id, 0 from landblock_instance li
  union all
  select s.inst, g.weenie_Class_Id, s.depth + 1 from spawn s
  join weenie_properties_generator g on g.object_Id = s.wcid
  where s.depth < 4
    -- an entry put in a container, a hand or a shop (where_Create Contain, Wield, Shop:
    -- RegenLocationType.cs:12-16) puts nothing in the world, as the guardians a chest holds
    and (g.where_Create & 56) = 0
),
mon as (
  select distinct s.inst, s.wcid, li.obj_Cell_Id cell,
    li.origin_X x, li.origin_Y y, li.origin_Z z, nm.value nm, lv.value lv
  from spawn s
  join landblock_instance li on li.guid = s.inst
  join weenie c on c.class_Id = s.wcid and c.type = 10
  join weenie_properties_int lv on lv.object_Id = c.class_Id and lv.type = 25
  join weenie_properties_string nm on nm.object_Id = c.class_Id and nm.type = 1
  left join weenie_properties_bool b on b.object_Id = c.class_Id and b.type = 19
  where coalesce(b.value, 1) = 1
)
select hex(cell), round(avg(x), 1), round(avg(y), 1), round(avg(z), 1),
  lv, count(*), wcid, nm
from mon group by cell, wcid, nm, lv;'

{
  cat <<'HEADER'
# What spawns where in Dereth, outdoors and in dungeon rooms.
# cell,x,y,z,level,count,wcid,name
# The cell is hex; x/y/z are the average generator position, local to the
# cell's landblock; count is how many generator entries spawn that creature
# there. Regenerate with reference/scripts/data/spawns.sh (reads the ACE
# world database, the community's reconstruction of retail's server data;
# the client has no copy of its own).
HEADER

  docker exec ace-db sh -c \
    "mysql -uroot -p\"\$MYSQL_ROOT_PASSWORD\" -N --batch ace_world -e '$SQL'" \
    2>/dev/null |
    awk -F'\t' 'NF == 8 {
      gsub(/,/, ";", $8)
      printf "%s,%s,%s,%s,%s,%s,%s,%s\n", $1, $2, $3, $4, $5, $6, $7, $8
    }' |
    LC_ALL=C sort -t, -k1,1 -k8,8 -k7,7n
} | gzip -9 -n
