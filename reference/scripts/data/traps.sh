#!/bin/sh
# Regenerate crates/ac-world/data/traps.csv: every pressure plate placed in
# the world whose linked object casts a spell on whoever trips it (Nullify
# All Magic Other, Mana Blight, Mana Drain, the bolts and streaks).
#
# A plate fires when the server's sweep of the player touches it
# (ACE PressurePlate.cs, Player.cs:370-386); the client is never told what a
# plate is linked to, so it needs this copy to fly over the ones that hurt.
# Plates that open doors or call guardians are left out: they are walked on.
#
# Usage: reference/scripts/data/traps.sh > crates/ac-world/data/traps.csv
set -e

SQL='SET SESSION max_execution_time=20000;
select hex(p.obj_Cell_Id), round(p.origin_X,2), round(p.origin_Y,2), round(p.origin_Z,2),
  hex(s.value), coalesce(sc.value, 1), group_concat(distinct sn.name separator ";")
from landblock_instance p
join weenie pw on pw.class_Id = p.weenie_Class_Id and pw.type = 24
join landblock_instance_link l on l.child_Guid = p.guid
join landblock_instance t on t.guid = l.parent_Guid
join weenie_properties_d_i_d sp on sp.object_Id = t.weenie_Class_Id and sp.type = 28
join spell sn on sn.id = sp.value
join weenie_properties_d_i_d s on s.object_Id = p.weenie_Class_Id and s.type = 1
left join weenie_properties_float sc on sc.object_Id = p.weenie_Class_Id and sc.type = 39
group by p.guid, p.obj_Cell_Id, p.origin_X, p.origin_Y, p.origin_Z, s.value, sc.value;'

cat <<'HEADER'
# Trap plates of Dereth: pressure plates whose linked object casts a spell on whoever trips one.
# cell,x,y,z,setup,scale,spells
# Cells and setups are hex; x/y/z local to the cell's landblock; the setup's spheres times scale
# are what the server's sweep must not touch. Regenerate with reference/scripts/data/traps.sh.
HEADER

docker exec ace-db sh -c \
  "mysql -uroot -p\"\$MYSQL_ROOT_PASSWORD\" -N --batch ace_world -e '$SQL'" \
  2>/dev/null |
  awk -F'\t' 'NF == 7 {
    gsub(/,/, " ", $7)
    printf "%s,%s,%s,%s,%s,%s,%s\n", $1, $2, $3, $4, $5, $6, $7
  }' |
  LC_ALL=C sort -t, -k1,1 -k2,2n -k3,3n
