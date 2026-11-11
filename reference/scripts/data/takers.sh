#!/bin/sh
# Regenerate crates/ac-world/data/takers.csv: the placed NPCs that take any
# item handed to them (AiAcceptEverything, PropertyBool 79), the Town Criers
# among them, and where each stands.
#
# GiveObjectToNPC takes anything such an NPC is handed, the whole stack, but a
# sticky-attuned item (Player_Inventory.cs:3435-3443). An attuned item can be
# neither dropped (:1389) nor given to a player (:3280), so for an unsellable
# one these NPCs are the only way out of the pack. The client never sees bool
# 79 (not an assessment property), hence this table.
#
# Reads the local ACE world database (the community's reconstruction of
# retail's server data) running in Docker; only needed when that changes.
# The query is capped at 20 s.
#
# Usage: reference/scripts/data/takers.sh > crates/ac-world/data/takers.csv
set -e

SQL='SET SESSION max_execution_time=20000;
select w.class_Id, replace(n.value, ",", ";"), hex(li.obj_Cell_Id),
       round(li.origin_X, 1), round(li.origin_Y, 1), round(li.origin_Z, 1)
from weenie w
join weenie_properties_bool b on b.object_Id = w.class_Id and b.type = 79 and b.value = 1
join weenie_properties_string n on n.object_Id = w.class_Id and n.type = 1
join landblock_instance li on li.weenie_Class_Id = w.class_Id;'

cat <<'HEADER'
# NPCs that take anything handed to them but a sticky-attuned item
# (AiAcceptEverything; Player_Inventory.cs:3435-3443), one row per placement.
# wcid,name,cell,x,y,z
# Cells are hex; the x/y/z are local to the cell's landblock. Regenerate with
# reference/scripts/data/takers.sh (reads the ACE world database, the
# community's reconstruction of retail's server data).
HEADER

docker exec ace-db sh -c "mysql -uroot -p\"\$MYSQL_ROOT_PASSWORD\" -N --batch ace_world -e '$SQL'" \
  | awk -F'\t' '{printf "%s,%s,%s,%s,%s,%s\n", $1, $2, $3, $4, $5, $6}' \
  | LC_ALL=C sort -t, -k2,2 -k3,3
