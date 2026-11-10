#!/bin/sh
# Regenerate crates/ac-world/data/summons.csv: what each summoning
# essence's creature does in a fight, for choosing the essence that hurts
# the target most.
#
# An essence (PetDevice, WeenieType 70) names its creature in PetClass
# (PropertyInt 266) and summons it (PetDevice.cs:82-84); the creature is a
# CombatPet (WeenieType 71) that fights with its body parts' damage, or a
# wielded weapon's (Monster_Melee.cs:385-404). How often it attacks comes
# from the client's own files: the maneuvers its combat table offers, the
# ones its motion table can play, and how long each plays. summons_rank.py
# (stdlib Python) reads both and works the figures out; its header says how.
#
# The essence's own damage and critical ratings are rolled per item when
# it drops (LootGenerationFactory_PetDevice.cs:9-44) and copied onto the
# creature (CombatPet.cs:46-52), so they are not here: the client reads
# them from the item's appraisal.
#
# Reads the local ACE world database (the community's reconstruction of
# retail's server data) running in Docker, every query capped at 20 s,
# and client_portal.dat through acdat. Only needed when either changes.
#
# Usage: AC_DATA_DIR=... reference/scripts/data/summons.sh > crates/ac-world/data/summons.csv
set -e

: "${AC_DATA_DIR:?set AC_DATA_DIR to the folder holding client_portal.dat}"
HERE=$(cd "$(dirname "$0")" && pwd)
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

q() {
  docker exec ace-db sh -c \
    "mysql -uroot -p\"\$MYSQL_ROOT_PASSWORD\" -N --batch ace_world -e 'SET SESSION max_execution_time=20000; $2'" \
    2>/dev/null >"$TMP/$1.tsv"
}

# Every essence whose creature fights: wcid, name, creature, the Summoning it needs
# (UseRequiresSkillLevel 367), its mastery (SummoningMastery 362), the character level it
# needs (UseRequiresLevel 369) and whether Summoning must be specialized (UseRequiresSkillSpec
# 368), all checked on use (WorldObject_Use.cs:232-296, PetDevice.cs:112-117).
q essences 'select e.class_Id, n.value, p.value, coalesce(s.value, 0), coalesce(m.value, 0),
  coalesce(ul.value, 0), if(sp.value is null, 0, 1)
from weenie e
join weenie_properties_int p on p.object_Id = e.class_Id and p.type = 266
join weenie c on c.class_Id = p.value and c.type = 71
join weenie_properties_string n on n.object_Id = e.class_Id and n.type = 1
left join weenie_properties_int s on s.object_Id = e.class_Id and s.type = 367
left join weenie_properties_int m on m.object_Id = e.class_Id and m.type = 362
left join weenie_properties_int ul on ul.object_Id = e.class_Id and ul.type = 369
left join weenie_properties_int sp on sp.object_Id = e.class_Id and sp.type = 368
where e.type = 70;'

# Each creature: level (25), Strength (1) and Quickness (3), motion table (DID 2), combat
# table (DID 4) and PowerupTime (PropertyFloat 34).
q pets 'select c.class_Id, coalesce(l.value, 0), coalesce(st.init_Level, 0),
  coalesce(qu.init_Level, 0), coalesce(mt.value, 0), coalesce(ct.value, 0), coalesce(pu.value, "")
from weenie c
left join weenie_properties_int l on l.object_Id = c.class_Id and l.type = 25
left join weenie_properties_attribute st on st.object_Id = c.class_Id and st.type = 1
left join weenie_properties_attribute qu on qu.object_Id = c.class_Id and qu.type = 3
left join weenie_properties_d_i_d mt on mt.object_Id = c.class_Id and mt.type = 2
left join weenie_properties_d_i_d ct on ct.object_Id = c.class_Id and ct.type = 4
left join weenie_properties_float pu on pu.object_Id = c.class_Id and pu.type = 34
where c.type = 71;'

q parts 'select b.object_Id, b.key, b.d_Type, b.d_Val, b.d_Var
from weenie_properties_body_part b join weenie c on c.class_Id = b.object_Id and c.type = 71;'

# A wielded weapon (create list destination Wield, 2): Damage 44, DamageType 45,
# DefaultCombatStyle 46, AttackType 47, DamageVariance (PropertyFloat 22).
q weapons 'select cl.object_Id, coalesce(d.value, 0), coalesce(t.value, 0), coalesce(s.value, 0),
  coalesce(a.value, 0), coalesce(v.value, 0)
from weenie_properties_create_list cl
join weenie c on c.class_Id = cl.object_Id and c.type = 71
join weenie w on w.class_Id = cl.weenie_Class_Id and w.type = 6
left join weenie_properties_int d on d.object_Id = w.class_Id and d.type = 44
left join weenie_properties_int t on t.object_Id = w.class_Id and t.type = 45
left join weenie_properties_int s on s.object_Id = w.class_Id and s.type = 46
left join weenie_properties_int a on a.object_Id = w.class_Id and a.type = 47
left join weenie_properties_float v on v.object_Id = w.class_Id and v.type = 22
where (cl.destination_Type & 2) <> 0;'

for t in essences pets parts; do
  [ -s "$TMP/$t.tsv" ] || { echo "summons.sh: the $t query returned nothing" >&2; exit 1; }
done

cargo build -q -p acdat
python3 -I "$HERE/summons_rank.py" "$TMP" "$AC_DATA_DIR/client_portal.dat" cargo run -q -p acdat --
