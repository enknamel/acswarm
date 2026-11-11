#!/bin/sh
# Regenerate crates/ac-world/data/spell_speeds.csv: how fast each damage
# spell's projectile flies.
#
# A spell projectile is launched at its weenie's MaximumVelocity
# (PropertyFloat 26), read by GetProjectileSpeed (WorldObject_Magic.cs:1898-1921)
# for the velocity CalculateProjectileVelocity aims (WorldObject_Magic.cs:1695-1756).
# The projectile's weenie is the spell row's Wcid. The client is told none of
# this: the DAT SpellTable carries no projectile class, and the speed is server data.
#
# Usage: reference/scripts/data/spell_speeds.sh > crates/ac-world/data/spell_speeds.csv
set -e

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

cat >"$TMP/q.sql" <<'SQL'
SET SESSION max_execution_time=20000;
select s.id, v.value, s.name
from spell s
join weenie_properties_float v on v.object_Id = s.wcid and v.type = 26
where s.base_Intensity is not null and s.base_Intensity > 0
order by 1;
SQL

cat <<'HEADER'
# How fast each damage spell's projectile flies.
# spell_id,speed,name
# speed is the projectile weenie's MaximumVelocity in m/s (the spell row's Wcid,
# PropertyFloat 26), the speed it is launched at (WorldObject_Magic.cs:1695-1756,
# 1898-1921). The name is there to read by; the client matches on the id.
# Regenerate with reference/scripts/data/spell_speeds.sh (reads the ACE world
# database; the client's SpellTable carries no projectile).
HEADER

docker exec -i ace-db sh -c \
  'mysql -uroot -p"$MYSQL_ROOT_PASSWORD" -N --batch ace_world 2>/dev/null' \
  <"$TMP/q.sql" |
  awk -F'\t' 'NF == 3 {
    gsub(/,/, ";", $3)
    printf "%s,%g,%s\n", $1, $2, $3
  }' |
  LC_ALL=C sort -t, -k1,1n
