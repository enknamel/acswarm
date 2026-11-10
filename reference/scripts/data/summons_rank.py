#!/usr/bin/env python3
"""What each summoning essence's creature does in a fight, as summons.csv (stdlib only).

Usage: summons_rank.py DUMP_DIR PORTAL_DAT ACDAT_CMD..., DUMP_DIR holding summons.sh's tables.
ACE citations are under reference/ext/ACE/Source/ACE.Server/ unless they name another project.

An attack, as a CombatPet makes one (Monster_Melee.cs:45-159, 164-314):
- each monster tick (0.2 s, Monster_Tick.cs:10) it rolls an attack height, uniform over its
  combat table's (1-3, from 2 when the table has not three: Monster_Melee.cs:202-204); the
  attack type is Punch above Low and Kick at Low, or its weapon's (Monster_Melee.cs:222-232);
  of that height's maneuvers of that type one is picked at random (Monster_Melee.cs:267-279);
- a maneuver its motion table cannot play from Ready ends the try with nothing done, and the
  next tick rolls again (Monster_Melee.cs:282-308, 64-66; Monster_Combat.cs:248-268);
- one it can plays for the animation's length over its speed, 1 / (1 - Quickness/300 + 40/150)
  within 0.5-2 (Creature_Combat.cs:503-522; a monster's weapon speed is always the default 40,
  WorldObject_Weapon.cs:265-274), and strikes once per attack hook in it, every strike with the
  body part the first hook names, or any part that does damage when none has that key
  (Monster_Melee.cs:73-99, 590-620; DamageEvent.cs:437-450); a wielded weapon's damage stands
  in for the part's (Monster_Melee.cs:385-397);
- the next attack waits a random 0 to PowerupTime (default 1 s) after it (Monster_Melee.cs:154-156).
A strike does max * (1 - variance/2) on average, times 1 + (Strength - 55) * 0.011
(SkillFormula.cs:18-23, Creature_Combat.cs:412-420); a critical does max (DamageEvent.cs:287).
"""
import json
import os
import subprocess
import sys

TICK = 0.2
READY = 0x41000003
HAND_COMBAT, SWORD_COMBAT = 0x8000003C, 0x8000003E
# CombatStyle.OneHanded (ACE.Entity/Enum/CombatStyle.cs), played in SwordCombat
# (Creature_Combat.cs:315-316).
ONE_HANDED = 2
PUNCH, KICK = 0x1, 0x8
LOW = 3
BREATH = 22
WEAPON_SPEED = 40


def rows(dump, name):
    path = os.path.join(dump, name + ".tsv")
    if not os.path.exists(path):
        return
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            if line.strip():
                yield line.rstrip("\n").split("\t")


class Dat:
    def __init__(self, dat, cmd):
        self.dat, self.cmd, self.cache = dat, cmd, {}

    def run(self, verb, fid):
        args = self.cmd + [self.dat, verb, "0x%08X" % fid]
        return subprocess.run(args, check=True, capture_output=True).stdout

    def decode(self, fid):
        if fid not in self.cache:
            self.cache[fid] = json.loads(self.run("decode", fid))
        return self.cache[fid]

    def combat_table(self, fid):
        """CombatManeuverTable (0x30): id, count, then (style, height, type, min skill, motion)."""
        if fid not in self.cache:
            b = self.run("cat", fid)
            word = lambda i: int.from_bytes(b[i * 4:i * 4 + 4], "little")
            self.cache[fid] = [tuple(word(2 + i * 5 + j) for j in range(5)) for i in range(word(1))]
        return self.cache[fid]


def links(table, key):
    for k, inner in table["links"]:
        if k == key:
            return {m: d for m, d in inner}
    return {}


def anim_data(table, stance, motion):
    """MotionTable.GetAnimData (ACE.DatLoader/FileTypes/MotionTable.cs:130-149), from Ready."""
    low = stance & 0xFFFF
    data = links(table, (low << 16) | (READY & 0xFFFFF)).get(motion)
    if data is None:
        data = links(table, low << 16).get(motion)
    return data["anims"] if data else []


def play(dat, table, stance, motion):
    """(seconds at speed 1, attack hooks' part indices) of one maneuver."""
    secs, hooks = 0.0, []
    for a in anim_data(table, stance, motion):
        anim = dat.decode(a["anim_id"])
        n = anim["num_frames"]
        high = n if a["high_frame"] == -1 or a["high_frame"] > n else a["high_frame"]
        # MotionTable.cs:162-182 (ACE.DatLoader): frames over the framerate.
        secs += (high - a["low_frame"]) / abs(a["framerate"])
        for frame in anim["part_frames"]:
            for h in frame["hooks"]:
                if isinstance(h["data"], dict) and "Attack" in h["data"]:
                    hooks.append(h["data"]["Attack"]["part_index"])
    return secs, hooks


def picks(cmt, stance, attack_type):
    """[(chance, motion)] of every maneuver a try can roll."""
    own = [r for r in cmt if r[0] == stance]
    heights = sorted({r[1] for r in own})
    start = 1 if len(heights) == 3 else 2
    span = list(range(start, 4))
    out = []
    for h in span:
        kind = attack_type or (KICK if h == LOW else PUNCH)
        found = [r[4] for r in own if r[1] == h and r[2] == kind]
        if not found and kind in (PUNCH, KICK) and not attack_type:
            swap = PUNCH if kind == KICK else KICK
            if kind == KICK or h == LOW:
                found = [r[4] for r in own if r[1] == h and r[2] == swap]
        for m in found:
            out.append((1.0 / len(span) / len(found), m))
        if not found:
            out.append((1.0 / len(span), None))
    return out


def figures(dat, pet, parts, weapon):
    """(element bit, hit, top, secs) of one creature, or None when it never strikes."""
    _, level, strength, quick, mt, ct, powerup = pet
    table, cmt = dat.decode(mt), dat.combat_table(ct)
    stance, attack_type = HAND_COMBAT, 0
    if weapon:
        if weapon["style"] != ONE_HANDED or bin(weapon["attack"]).count("1") != 1:
            sys.exit("summons_rank.py: weapon of %d not worked out: %r" % (pet[0], weapon))
        stance, attack_type = SWORD_COMBAT, weapon["attack"]
    ready = links(table, ((stance & 0xFFFF) << 16) | (READY & 0xFFFFFF))
    speed = min(max(1.0 / (1.0 - quick / 300.0 + WEAPON_SPEED / 150.0), 0.5), 2.0)
    attr = max(1.0 + (strength - 55) * 0.011, 1.0)
    q = hit = top = secs = 0.0
    kinds = set()
    for chance, motion in picks(cmt, stance, attack_type):
        if motion is None or motion not in ready:
            continue
        length, hooks = play(dat, table, stance, motion)
        strikes = max(len(hooks), 1)
        if weapon:
            used = [(weapon["type"], weapon["damage"], weapon["variance"])]
        else:
            used = [p for k, p in parts.items() if hooks and k == hooks[0]]
            if not used:
                used = [p for k, p in parts.items() if p[1] != 0 and k != BREATH]
        if not used:
            continue
        kinds.update(p[0] for p in used if p[1] != 0)
        q += chance
        hit += chance * strikes * sum(p[1] * (1 - p[2] / 2) for p in used) / len(used)
        top += chance * strikes * sum(p[1] for p in used) / len(used)
        secs += chance * length / speed
    if q == 0 or len(kinds) != 1:
        return None
    wait = (1.0 / q - 1.0) * TICK + TICK / 2 + (float(powerup) if powerup else 1.0) / 2
    return kinds.pop(), hit / q * attr, top / q * attr, secs / q + wait


def main():
    dump, portal, cmd = sys.argv[1], sys.argv[2], sys.argv[3:]
    dat = Dat(portal, cmd)
    pets = {int(r[0]): (int(r[0]), int(r[1]), int(r[2]), int(r[3]), int(r[4]), int(r[5]), r[6])
            for r in rows(dump, "pets")}
    parts = {}
    for r in rows(dump, "parts"):
        parts.setdefault(int(r[0]), {})[int(r[1])] = (int(r[2]), int(r[3]), float(r[4]))
    weapons = {}
    for r in rows(dump, "weapons"):
        weapons[int(r[0])] = {"damage": int(r[1]), "type": int(r[2]), "style": int(r[3]),
                              "attack": int(r[4]), "variance": float(r[5])}
    print("# Summoning essences: what each one's creature does in a fight.")
    print("# essence,name,pet,level,skill,mastery,element,hit,top,secs")
    print("# pet is the creature's wcid and level its level; skill is the Summoning the essence")
    print("# needs and mastery its SummoningMastery (1 Primalist, 2 Necromancer, 3 Naturalist,")
    print("# 0 any). element is the DamageType bit it hits with. hit is the damage of one attack")
    print("# on average and top a critical's before its doubling, both before ratings and the")
    print("# target's armour and resistances; secs is the mean time from one attack to the next,")
    print("# the tries its motion table cannot play included. A creature that never strikes has")
    print("# 0 for all three.")
    print("# Regenerate with reference/scripts/data/summons.sh (reads the ACE world database,")
    print("# the community's reconstruction of retail's server data, and client_portal.dat).")
    out = []
    for r in rows(dump, "essences"):
        wcid, name, pet, skill, mastery = int(r[0]), r[1], int(r[2]), int(r[3]), int(r[4])
        if pet not in pets:
            continue
        got = figures(dat, pets[pet], parts.get(pet, {}), weapons.get(pet))
        element, hit, top, secs = got if got else (0, 0.0, 0.0, 0.0)
        out.append((wcid, name.replace(",", ";"), pet, pets[pet][1], skill, mastery, element,
                    "%.1f" % hit, "%.1f" % top, "%.2f" % secs))
    for row in sorted(out):
        print(",".join(str(v) for v in row))


if __name__ == "__main__":
    main()
