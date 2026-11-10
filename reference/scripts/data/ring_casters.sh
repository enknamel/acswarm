#!/bin/sh
# Regenerate crates/ac-world/data/ring_casters.csv: every creature whose
# spell book holds a ring, with each ring's reach and pace.
#
# A ring is a spell of more than one projectile whose category is
# AcidRing..SlashingRing or whose SpreadAngle is 360
# (SpellProjectile.cs:136-150): projectiles leave the caster in every
# direction, and each bursts once it is past the spell's
# BaseRangeConstant from where it spawned (WorldObject_Tick.cs:351-362).
# Their speed is the projectile weenie's MaximumVelocity
# (WorldObject_Magic.cs:1898-1918). The client is told none of this: a
# monster's spell book is server data, and so is the projectile speed;
# the category and BaseRangeConstant are the client's own SpellTable,
# read here with acdat.
#
# Usage: AC_DATA_DIR=... reference/scripts/data/ring_casters.sh > crates/ac-world/data/ring_casters.csv
set -e

DATA=${AC_DATA_DIR:?set AC_DATA_DIR to the folder holding client_portal.dat}
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../.." && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

cat >"$TMP/q.sql" <<'SQL'
SET SESSION max_execution_time=20000;
select b.object_Id, coalesce(n.value, ''), s.id, coalesce(s.spread_Angle, 0),
  coalesce(v.value, 0), s.name
from weenie_properties_spell_book b
join weenie w on w.class_Id = b.object_Id and w.type = 10
join spell s on s.id = b.spell and s.num_Projectiles > 1
left join weenie_properties_string n on n.object_Id = b.object_Id and n.type = 1
left join weenie_properties_float v on v.object_Id = s.wcid and v.type = 26;
SQL
docker exec -i ace-db sh -c \
  'mysql -uroot -p"$MYSQL_ROOT_PASSWORD" -N --batch ace_world 2>/dev/null' \
  <"$TMP/q.sql" >"$TMP/book.tsv"

(cd "$ROOT" && cargo run -q -p acdat -- "$DATA/client_portal.dat" decode 0x0E00000E) \
  >"$TMP/spells.json"

cat <<'HEADER'
# Creatures whose spell book holds a ring spell, one row per creature and ring.
# wcid,spell_id,range,speed,creature,spell
# range is the ring's BaseRangeConstant in metres: each projectile bursts once it is that far
# from where it spawned, round the caster (WorldObject_Tick.cs:351-362). speed is the
# projectile's MaximumVelocity, m/s. Names are there to read by; the client matches on the ids.
# Regenerate with reference/scripts/data/ring_casters.sh (the ACE world database and the
# client's SpellTable).
HEADER

python3 -I - "$TMP/book.tsv" "$TMP/spells.json" <<'PY' | LC_ALL=C sort -t, -k1,1n -k2,2n
import json, sys
book, table = sys.argv[1], sys.argv[2]
spells = {i: s for i, s in json.load(open(table))["spells"]}
ACID_RING, SLASHING_RING = 222, 228
for line in open(book):
    f = line.rstrip("\n").split("\t")
    if len(f) != 6:
        continue
    wcid, creature, spell, spread, speed, name = f
    s = spells.get(int(spell))
    if s is None:
        continue
    if not (ACID_RING <= s["category"] <= SLASHING_RING or float(spread) == 360):
        continue
    clean = lambda t: t.replace(",", ";").strip()
    print(f"{wcid},{spell},{s['base_range_constant']:g},{float(speed):g},{clean(creature)},{clean(name)}")
PY
