#!/bin/sh
# Regenerate crates/ac-world/data/unlocks.csv: the quest flags a portal
# asks for that a character earns just by talking to someone at a level.
#
# A portal with a QuestRestriction (PropertyString 37) lets in only a
# character holding that flag (Portal.cs:246-258). Some flags are
# stamped by an NPC whose Use emote checks nothing but the level: an
# InqIntStat on Level (stat 25) whose TestSuccess set, named after the
# check, stamps the flag (StampQuest 22 or SetQuestCompletions 70). The
# Facility Hub Wardens are all of these. A Use set that asks anything
# else first (another quest, an item, a skill) is left out: talking
# alone does not earn that flag.
#
# This reads the local ACE world database running in Docker, and is
# only needed when that data changes.
#
# Usage: reference/scripts/data/unlocks.sh > crates/ac-world/data/unlocks.csv
set -e

SQL='SET SESSION max_execution_time=20000;
select distinct a_s.message, a_u.min, nm.value, hex(li.obj_Cell_Id),
  round(li.origin_X, 2), round(li.origin_Y, 2), round(li.origin_Z, 2)
from weenie_properties_emote e_s
join weenie_properties_emote_action a_s on a_s.emote_Id = e_s.id and a_s.type in (22, 70)
join weenie_properties_emote e_u on e_u.object_Id = e_s.object_Id and e_u.category = 7
join weenie_properties_emote_action a_u on a_u.emote_Id = e_u.id and a_u.type = 36
  and a_u.stat = 25 and a_u.message = e_s.quest
join weenie_properties_string nm on nm.object_Id = e_s.object_Id and nm.type = 1
join landblock_instance li on li.weenie_Class_Id = e_s.object_Id
where e_s.category = 22
  and a_s.message in (select value from weenie_properties_string where type = 37)
  and not exists (
    select 1 from weenie_properties_emote_action x
    where x.emote_Id = e_u.id
      and (x.type in (21, 30, 35, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 51, 58, 59, 71,
                      75, 76, 80, 82, 89, 102, 103, 104, 105, 114, 121)
           or (x.type = 36 and x.stat <> 25)))
order by a_u.min, a_s.message;'

cat <<'HEADER'
# Quest flags earned by talking to someone: the portals that ask for them
# let the character in afterwards.
# flag,min_level,npc,cell,x,y,z
# The cell is hex; x/y/z are local to its landblock. min_level is the
# level the NPC checks before stamping the flag.
# Regenerate with reference/scripts/data/unlocks.sh (reads the ACE world
# database, the community's reconstruction of retail's server data).
HEADER

docker exec ace-db sh -c \
  "mysql -uroot -p\"\$MYSQL_ROOT_PASSWORD\" -N --batch ace_world -e '$SQL'" \
  2>/dev/null |
  awk -F'\t' 'NF == 7 {
    gsub(/,/, ";", $3)
    printf "%s,%s,%s,%s,%s,%s,%s\n", $1, $2, $3, $4, $5, $6, $7
  }'
