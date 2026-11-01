#!/bin/sh
# Regenerate crates/ac-world/data/spell_damage.csv: how much each damage
# spell does before anything multiplies it.
#
# The client's own SpellTable says a spell's school, power and mana but
# not its damage, so a client that wants to know how many casts a
# creature will take has to be told. The server knows: ACE's spell row
# has a base and a variance, MinDamage = base and MaxDamage = base +
# variance (SpellProperties.cs:262-271), and each cast rolls between the
# two (SpellProjectile.cs:537).
#
# Spell ids are the client's own, so they are looked up directly against
# the SpellTable.
#
# Usage: reference/scripts/data/spell_damage.sh > crates/ac-world/data/spell_damage.csv
set -e

SQL='select id, base_Intensity, base_Intensity + coalesce(variance, 0), name
from spell
  where base_Intensity is not null and base_Intensity > 0
order by 1;'

cat <<'HEADER'
# How much each damage spell does before anything multiplies it.
# spell_id,min,max,name
# A cast rolls from min to max (ACE MinDamage and MaxDamage,
# SpellProperties.cs:262-271; rolled at SpellProjectile.cs:537). The
# name is there to read by; the client matches on the id.
# Regenerate with reference/scripts/data/spell_damage.sh (reads the
# ACE world database, the community's reconstruction of retail's server
# data; the client's SpellTable does not carry damage).
HEADER

docker exec ace-db sh -c \
  "mysql -uroot -p\"\$MYSQL_ROOT_PASSWORD\" -N --batch ace_world -e '$SQL'" \
  2>/dev/null |
  awk -F'\t' 'NF == 4 {
    gsub(/,/, ";", $4)
    printf "%s,%s,%s,%s\n", $1, $2, $3, $4
  }' |
  LC_ALL=C sort -t, -k1,1n
