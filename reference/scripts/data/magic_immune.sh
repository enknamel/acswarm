#!/bin/sh
# Regenerate crates/ac-world/data/magic_immune.csv: the creatures flagged
# NonProjectileMagicImmune (PropertyBool 103). No spell but a projectile
# takes hold on one: a vulnerability, a Magic Yield or an imperil is
# answered "You fail to affect" (ACE Player_Magic.cs:1109).
#
# Usage: reference/scripts/data/magic_immune.sh > crates/ac-world/data/magic_immune.csv
set -e

SQL='SET SESSION max_execution_time=20000;
select b.object_Id, coalesce(n.value, "")
from weenie_properties_bool b
join weenie w on w.class_Id = b.object_Id and w.type = 10
left join weenie_properties_string n on n.object_Id = b.object_Id and n.type = 1
where b.type = 103 and b.value = 1
order by b.object_Id;'

cat <<'HEADER'
# Creatures no spell but a projectile affects (NonProjectileMagicImmune).
# wcid,name
# Regenerate with reference/scripts/data/magic_immune.sh (reads the ACE world database).
HEADER

docker exec ace-db sh -c \
  "mysql -uroot -p\"\$MYSQL_ROOT_PASSWORD\" -N --batch ace_world -e '$SQL'" \
  2>/dev/null |
  awk -F'\t' 'NF == 2 { gsub(/,/, ";", $2); printf "%s,%s\n", $1, $2 }'
