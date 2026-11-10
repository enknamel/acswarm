#!/usr/bin/env python3
"""Walk ACE's emote sets into quests.csv and quest_sources.csv (stdlib only), and simulate the
generators for how fast each hunted creature comes (the sources' `rates`, seeded).

Usage: quests_walk.py DUMP_DIR OUT_DIR, DUMP_DIR holding the tables quests.sh reads out.
docs/quests-design.md §1.4 says what a path is and which branches the client can know.
ACE citations are under reference/ext/ACE/Source/ACE.Server/.
"""
import heapq
import os
import random
import sys
from collections import defaultdict

# EmoteCategory (ACE.Entity/Enum/EmoteCategory.cs).
REFUSE, GIVE, USE = 1, 6, 7
QUEST_OK, QUEST_NO, TEST_OK, TEST_NO, GOTO_SET = 12, 13, 22, 23, 32
# EmoteType (ACE.Entity/Enum/EmoteType.cs).
AWARD_XP, GIVE_ITEM, INQ_QUEST, STAMP, INQ_SOLVES = 2, 3, 21, 22, 30
INQ_INT, LEVEL_XP, NO_SHARE_XP, GOTO, SET_SOLVES, TAKE, INQ_OWNS = 36, 49, 62, 67, 70, 74, 76
# Every branching type (EmoteManager.EmoteIsBranchingType); the ones not read above end a path.
BRANCHING = {20, 21, 30, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 51, 58, 59, 60, 67, 71,
             75, 76, 79, 80, 82, 89, 102, 103, 104, 105, 114, 121}
LEVEL_STAT = 25
# ACE's top level (Player_Xp.cs GetMaxLevel): a band reaching it is open above.
MAX_LEVEL = 275
DEPTH = 8
# A root whose paths pass this many is not a quest anyone wrote by hand.
MAX_PATHS = 4096
# WeenieType Chest and Container (ACE.Entity/Enum/WeenieType.cs).
CHESTS = (20, 21)
# Generators of generators looked through for where an item lies.
GEN_DEPTH = 3
# Keys of keys looked through for what a chest's item needs.
KEY_DEPTH = 3


def rows(dump, name):
    with open(os.path.join(dump, name + ".tsv"), encoding="utf-8", errors="replace") as f:
        for line in f:
            yield line.rstrip("\n").split("\t")


def corner(cells):
    """Of a landblock's encounter `(count, cell_X, cell_Y)`s, the busiest corner (the one nearest
    the middle of a tie) as metres into the landblock, clamped as ACE does (Landblock.cs:281-282)."""
    _, x, y = max(cells, key=lambda c: (c[0], -abs(c[1] - 4) - abs(c[2] - 4)))
    return tuple(min(max(v * 24.0, 0.5), 191.5) for v in (x, y))


def num(s, default=0.0):
    try:
        return float(s)
    except ValueError:
        return default


def qname(message):
    """The quest a message names, without its @comment (QuestManager.GetQuestName)."""
    return message.split("@", 1)[0].lower()


class Data:
    def __init__(self, dump):
        self.placed = defaultdict(list)
        self.wtype = {}
        for wcid, cell, x, y, z, t in rows(dump, "placed"):
            self.placed[int(wcid)].append((int(cell, 16), x, y, z))
            self.wtype[int(wcid)] = int(t)
        self.sets = {}
        self.by_obj = defaultdict(list)
        for sid, obj, cat, prob, wcid, quest in rows(dump, "emote"):
            s = {"id": int(sid), "obj": int(obj), "cat": int(cat), "p": num(prob, 1.0),
                 "wcid": int(wcid), "quest": quest.lower(), "acts": []}
            self.sets[s["id"]] = s
            self.by_obj[s["obj"]].append(s)
        for f in rows(dump, "action"):
            s = self.sets.get(int(f[0]))
            if s is None:
                continue
            s["acts"].append({"type": int(f[2]), "msg": f[3], "min": f[4], "max": f[5],
                              "amount": f[6], "amount64": f[7], "percent": f[8], "min64": f[9],
                              "max64": f[10], "stat": f[11], "wcid": f[12], "stack": f[13],
                              "spell": int(num(f[14])) if len(f) > 14 else 0})
        self.harm = {int(i) for (i,) in rows(dump, "harm")}
        # The scrolls teaching each spell: spell -> scroll wcids.
        self.scrolls_of = defaultdict(list)
        for wcid, spell in rows(dump, "scrolls"):
            self.scrolls_of[int(spell)].append(int(wcid))
        # Recall spells by id and name, and portal gems (recalls.csv, gems.csv, read in main).
        self.recalls, self.gems = {}, []
        self.quest = {}
        for name, delta, solves, message in rows(dump, "quest"):
            self.quest[name.lower()] = (int(delta), int(solves), message)
        self.counter_of = defaultdict(set)
        for obj, value in rows(dump, "killq"):
            self.counter_of[qname(value)].add(int(obj))
        self.drops = defaultdict(list)
        for obj, wcid, dest, shade, _stack in rows(dump, "drops"):
            treasure = int(dest) & 8 and num(shade) != 0
            self.drops[int(wcid)].append((int(obj), num(shade) if treasure else 1.0))
        self.name = {int(o): v.replace(",", ";") for o, v in rows(dump, "names")}
        self.ints = defaultdict(dict)
        for obj, t, v in rows(dump, "ints"):
            self.ints[int(obj)][int(t)] = int(v)
        self.allow_give = {int(o) for (o,) in rows(dump, "allowgive")}
        # An item's own pickup limit and the flag its pickup asks (Player_Inventory.cs:1207-1262).
        self.pickup, self.restrict = {}, {}
        for obj, t, v in rows(dump, "itemq"):
            (self.pickup if t == "33" else self.restrict)[int(obj)] = qname(v)
        self.spawns = defaultdict(list)
        for wcid, lb, n in rows(dump, "spawns"):
            self.spawns[int(wcid)].append((int(lb, 16), int(n)))
        # The encounter table's, only for a creature nothing placed spawns, and only its
        # ENCOUNTER_BLOCKS landblocks with the most generators: all of a common one's thousands
        # took the table from under 1 MB to 37 MB (a Banished Drudge is on 8,363).
        # Each landblock's spot is its corner with the most, at 24 m a cell (Landblock.cs:281-282):
        # a landblock's middle can be sea (the Mire-Witch's 0x97ED).
        placed = set(self.spawns)
        met = defaultdict(lambda: defaultdict(list))
        for wcid, lb, x, y, n in rows(dump, "encounters"):
            if int(wcid) not in placed:
                met[int(wcid)][int(lb, 16)].append((int(n), int(x), int(y)))
        self.met_at = {}
        for wcid, blocks in met.items():
            counts = sorted(((sum(c[0] for c in cs), lb) for lb, cs in blocks.items()),
                            key=lambda b: (-b[0], b[1]))
            kept = sorted((lb, n) for n, lb in counts[:ENCOUNTER_BLOCKS])
            self.spawns[wcid] = kept
            self.met_at[wcid] = [corner(blocks[lb]) for lb, _ in kept]
        # What lies about: placed objects but creatures and buildings, by wcid.
        # And the placements a generator makes, linked to it: made there with a guid of their own.
        self.ground, self.linked = defaultdict(list), defaultdict(list)
        for wcid, cell, x, y, z, link in rows(dump, "ground"):
            self.ground[int(wcid)].append((int(cell, 16), x, y, z))
            if link == "1":
                self.linked[int(wcid)].append((int(cell, 16), x, y, z))
        self.kind = {int(w): int(t) for w, t in rows(dump, "wtypes")}
        # Generator profiles by the wcid they make (WorldObject_Generators.cs:108-180).
        self.made_by = defaultdict(list)
        last = {}
        for gen, prob, wcid, where, cell, x, y, z in rows(dump, "gens"):
            g, p = int(gen), num(prob, 1.0)
            # -1 always makes; others are cumulative (WorldObject_Generators.cs:159, 184-213).
            was = last.get(g, 0.0)
            chance = 1.0 if p < 0 else max(p - (0.0 if was > p else was), 0.0)
            last[g] = p if p >= 0 else was
            self.made_by[int(wcid)].append((g, min(chance, 1.0), int(where),
                                            (int(cell, 16), x, y, z)))
        # Generators put in the world themselves: placed, or a landblock's encounter.
        self.roots = {int(w) for (w,) in rows(dump, "roots")}
        self.chest_list = defaultdict(set)
        for chest, wcid, dest in rows(dump, "chestlist"):
            if int(dest) & 1:  # DestinationType.Contain
                self.chest_list[int(wcid)].add(int(chest))
        self.lock = {int(o): code for o, code in rows(dump, "locks")}
        self.keys_of = defaultdict(list)
        for key, code in rows(dump, "keys"):
            self.keys_of[code.lower()].append(int(key))
        self.stuck = {int(o) for (o,) in rows(dump, "stuck")}
        # What an item used on a placed object makes: made -> object -> items used on it.
        self.made_on = defaultdict(lambda: defaultdict(set))
        for made, used, on in rows(dump, "recipes"):
            self.made_on[int(made)][int(on)].add(int(used))
        # What one carried item used on another makes: made -> (item used, item it is used on).
        self.combined = defaultdict(set)
        for made, used, on in rows(dump, "combines"):
            self.combined[int(made)].add((int(used), int(on)))
        # Items a shop sells (vendors.csv, joined at load): a source the walk does not write.
        self.sold = set()
        self.spawning = Spawning(dump)

    def ground_places(self, wcid, depth=0):
        """Where `wcid` lies or stands in the world: placed, or made on the ground."""
        return list(self.ground.get(wcid, ())) + self.made_on_ground(wcid, depth)

    def made_on_ground(self, wcid, depth=0):
        """Where a placed generator makes `wcid` on the ground (a profile with no Contain, Wield,
        Shop or Treasure bit: RegenLocationType.cs), at its Specific spot or the generator's own,
        or a placement linked to its generator, at that placement.
        A placed object's guid is static and never picked up (Player_Inventory.cs:831-835)."""
        out = list(self.linked.get(wcid, ()))
        if depth >= GEN_DEPTH:
            return out
        for gen, _, where, spot in self.made_by.get(wcid, ()):
            if where & (8 | 16 | 32 | 64):
                continue
            if where & 4 and spot[0]:
                out.append(spot)
            else:
                out.extend(self.ground_places(gen, depth + 1))
        return out

    def chests_holding(self, wcid):
        """Chests that hold `wcid`, and the chance each fill has it: a Contain profile of
        theirs (not a treasure table's) or their create list."""
        found = defaultdict(float)
        for gen, chance, where, _ in self.made_by.get(wcid, ()):
            if where & 8 and not where & 64 and self.kind.get(gen) in CHESTS:
                found[gen] = min(found[gen] + chance, 1.0)
        for chest in self.chest_list.get(wcid, ()):
            if self.kind.get(chest) in CHESTS:
                found[chest] = 1.0
        return found

    def outcome(self, obj, cat, message):
        """The set a branch runs: one, or 'random' when drawn by chance (EmoteManager.cs:1546)."""
        key = message.lower()
        found = [s for s in self.by_obj[obj] if s["cat"] == cat and s["quest"] == key]
        if not found:
            return None
        if any(s["p"] < 1 for s in found):
            return "random"
        return found[0]

    def outcomes(self, obj, cats, message):
        """Every set of `obj` keyed on `message`, in `cats` (None: any reply category)."""
        key = message.lower()
        return [s for s in self.by_obj[obj] if s["quest"] == key
                and (s["cat"] in cats if cats else s["cat"] not in (REFUSE, GIVE, USE))]

    def level(self, wcid):
        return self.ints[wcid].get(25, 0)

    def spawn_share(self, wcid, depth=0):
        """The best share of a generator roll that puts `wcid` in the world, along its chain from
        a placed or encounter generator (WorldObject_Generators.cs:108-213): 1 a sure spawn, the
        Mosswart Mire-Witch's camp 0.005 of a Marae Lassel shore generator's rolls."""
        best = 1.0 if wcid in self.roots else 0.0
        if depth >= GEN_DEPTH:
            return best
        for gen, chance, where, _ in self.made_by.get(wcid, ()):
            if where & 56:  # Contain, Wield, Shop: RegenLocationType.cs:12-16
                continue
            best = max(best, chance * self.spawn_share(gen, depth + 1))
        return best


# Spawn rates: a Monte Carlo of ACE's generators (WorldObject_Generators.cs, GeneratorProfile.cs),
# for the `rates` column: how many of a creature a landblock holds when it loads, how many it
# makes an hour while the player kills all its generators make, and how many others that takes
# killing when killing it alone fills their slots with others.
# WeenieType Chest and Container: an emptied one is not destroyed (WorldObject_Generators.cs:571).
GEN_CONTAINERS = (20, 21)
# The placeholder wcid a profile never makes (GeneratorProfile.cs:54).
PLACEHOLDER = 3666
# Power-ups averaged for the load, and hours of clearing for the rate, per root generator.
LOAD_RUNS = 60
CLEAR_HOURS = 2.0
# Killed alone, a creature coming at under this share of its rate cleared needs its slots cleared.
ALONE_SHARE = 0.9


class Slot:
    """One generator profile: what it makes, how often and how many, and what it holds now."""
    __slots__ = ("prob", "wcid", "delay", "init", "max", "spawned", "avail")

    def __init__(self, p, delay0):
        self.prob, self.wcid, self.init, self.max = p["prob"], p["wcid"], p["init"], p["max"]
        # A profile with no delay takes profile[0]'s (GeneratorProfile.cs:106-114).
        self.delay = p["delay"] if p["delay"] is not None else (delay0 or 0.0)
        self.spawned = []
        self.avail = -1e18


class Gen:
    """A generator in the sim: its profiles, and linked placements as profiles of their own with
    profile[0]'s figures (WorldObject_Generators.cs:589-625)."""

    def __init__(self, sp, wcid, links=(), parent=None):
        gp = sp.genprops.get(wcid, (1, 0.0, 0, 0, 0, "0"))
        self.wcid, self.parent, self.alive, self.powering = wcid, parent, True, True
        kind, self.regen, self.init, self.max, self.ad, event = gp
        self.container = kind in GEN_CONTAINERS
        # Started with no regeneration, only a container (not a creature) or an event generator
        # fills at once (WorldObject_Generators.cs:487-491); any other waits on a use or an emote.
        self.fills = self.regen > 0 or self.container or event == "1"
        # One whose event the table has and not On is disabled (WorldObject_Generators.cs:398-460).
        if event == "off":
            self.regen, self.init, self.max, self.fills = 0.0, 0, 0, False
        profs = sp.profiles.get(wcid, [])
        delay0 = profs[0]["delay"] if profs else 0.0
        # Contain, Wield and Shop put nothing in the world (RegenLocationType.cs:12-16).
        self.slots = [Slot(p, delay0) for p in profs if not p["where"] & 56]
        if links and profs:
            t = profs[0]
            for w in links:
                self.slots.append(Slot(dict(t, wcid=w), delay0))
                if t["prob"] == -1:
                    self.init += t["init"]
                    self.max += t["max"]
        # Max under Init is raised to Init (WorldObject_Generators.cs:61-65).
        if (self.max <= 0 or self.max < self.init) and self.init > 0:
            self.max = self.init

    def cur(self):
        return sum(len(s.spawned) for s in self.slots)

    def maxed(self, s):
        return s.max != -1 and len(s.spawned) >= s.max

    def stops(self, now):
        """GenStopSelectProfileConditions (WorldObject_Generators.cs:319-338)."""
        c = self.cur()
        if c >= self.max or (self.powering and c >= self.init):
            return True
        live = [s for s in self.slots if s.wcid != PLACEHOLDER]
        return not any(now > s.avail for s in live) or all(self.maxed(s) for s in live)

    def total(self, now):
        """The share a roll is drawn over: cumulative thresholds of the slots free now, a -1 one
        always making (WorldObject_Generators.cs:184-214)."""
        tot, last = 0.0, 0.0
        for s in self.slots:
            if s.prob == -1:
                if not self.maxed(s) and now > s.avail:
                    return 1.0
                continue
            if not self.maxed(s) and now > s.avail:
                if last > s.prob:
                    last = 0.0
                tot += s.prob - last
            last = s.prob
        return tot

    def count(self, s):
        """How many one pick makes, capped by the free room (WorldObject_Generators.cs:281-314)."""
        n = 1 if (s.init == -1 or s.max == -1) else s.init
        n = min(n, self.max - self.cur())
        if s.max != -1:
            n = min(n, s.max - len(s.spawned))
        return max(n, 0)

    def select(self, now, rng, out):
        """SelectProfiles (WorldObject_Generators.cs:108-178, 238-276): the picks of one roll."""
        if self.stops(now):
            return
        roll = rng.uniform(0.0, self.total(now))
        tot, last = 0.0, 0.0
        for s in self.slots:
            # A placeholder's band counts toward the thresholds after it; it is only never made
            # (GetAdjustedProbability, :238-276, does not skip it; SelectProfiles :127 does).
            free = not self.maxed(s) and now > s.avail
            if s.prob != -1:
                if free:
                    if last > s.prob:
                        last = 0.0
                    tot += s.prob - last
                last = s.prob
            if not free or s.wcid == PLACEHOLDER:
                continue
            if s.prob == -1 or roll < tot:
                out.extend([s] * self.count(s))
                if s.prob != -1:
                    break
                if self.stops(now):
                    return


class Sim:
    """One root generator's tree in time; `kills(wcid)` says whether the player kills one at once."""

    def __init__(self, sp, root, links, rng, kills):
        self.sp, self.rng, self.kills = sp, rng, kills
        self.heap, self.seq, self.pending = [], 0, []
        self.made, self.alive = defaultdict(int), defaultdict(int)
        self.push(0.0, Gen(sp, root, links))

    def push(self, t, g):
        self.seq += 1
        heapq.heappush(self.heap, (t, self.seq, g))

    def generate(self, g, now):
        """Generator_Generate: power-up rolls until Init, then one roll a tick
        (WorldObject_Generators.cs:652-682)."""
        if g.powering:
            n = 0
            while not g.stops(now) and n < 1000:
                made = []
                g.select(now, self.rng, made)
                self.place(g, made, now)
                n += 1
            g.powering = False
        else:
            made = []
            g.select(now, self.rng, made)
            self.place(g, made, now)

    def place(self, g, made, now):
        sp = self.sp
        for s in made:
            w = s.wcid
            if w in sp.profiles and w not in sp.attackable:
                h = Gen(sp, w, parent=(g, s))
                s.spawned.append(h)
                self.push(now, h)
            elif sp.attackable.get(w):
                tok = ("c", w)
                s.spawned.append(tok)
                self.made[w] += 1
                self.alive[w] += 1
                if self.kills(w):
                    self.pending.append((g, s, tok))
            else:
                s.spawned.append(("o", w))  # an item, or a creature nobody kills: holds its slot

    def die(self, g, s, tok, now):
        """A death frees the slot after its delay (GeneratorProfile.cs:526-564); a generator a
        generator made goes once all it made is dead (WorldObject_Generators.cs:562-584)."""
        s.spawned.remove(tok)
        self.alive[tok[1]] -= 1
        s.avail = now + s.delay
        if g.init > 0 and g.cur() == 0 and (g.parent is not None and (not g.container or g.ad)
                                            or g.parent is None and g.ad):
            g.alive = False
            if g.parent is not None:
                _, ps = g.parent
                if g in ps.spawned:
                    ps.spawned.remove(g)
                ps.avail = now + ps.delay

    def run(self, until):
        """Regeneration ticks every RegenerationInterval s (WorldObject_Tick.cs:121-136); none at
        0 (:77-78). The player's kills land after the generator's own work each tick."""
        while self.heap and self.heap[0][0] <= until:
            t, _, g = heapq.heappop(self.heap)
            if not g.alive or not g.fills:
                continue
            self.generate(g, t)
            kills, self.pending = self.pending, []
            for kg, ks, tok in kills:
                self.die(kg, ks, tok, t)
            if g.regen > 0:
                self.push(t + g.regen, g)


class Spawning:
    """Each landblock's root generators, and per root what it holds at load and makes an hour."""

    def __init__(self, dump):
        self.profiles = defaultdict(list)
        for gen, prob, wcid, delay, init, mx, where in rows(dump, "profiles"):
            self.profiles[int(gen)].append(dict(
                prob=num(prob), wcid=int(wcid), delay=None if delay == "" else num(delay),
                init=int(init), max=int(mx), where=int(where)))
        self.genprops = {}
        for wcid, kind, regen, init, mx, ad, event in rows(dump, "genprops"):
            self.genprops[int(wcid)] = (int(kind), num(regen), int(init), int(mx), int(ad),
                                        event)
        # Creatures, by whether one can be attacked (PropertyBool 19, default true).
        self.attackable = {int(w): a != "0" for w, a in rows(dump, "crit")}
        links, children = defaultdict(list), set()
        for parent, child, wcid in rows(dump, "links"):
            links[int(parent)].append(int(wcid))
            children.add(int(child))
        # Roots: placed generators and creatures, not a linked placement (WorldObjectFactory.cs:280),
        # and the encounter table's generators, outdoors on a surface (Landblock.cs:269-290).
        self.roots = defaultdict(list)
        for guid, wcid, cell in rows(dump, "inst"):
            g, w, c = int(guid), int(wcid), int(cell, 16)
            if g not in children:
                self.roots[c >> 16].append((w, tuple(sorted(links.get(g, ()))), c & 0xFFFF < 0x100, 1))
        for lb, wcid, n in rows(dump, "encroots"):
            self.roots[int(lb, 16)].append((int(wcid), (), True, int(n)))
        self.surface = {lb for lb, rs in self.roots.items() if any(r[2] for r in rs)}
        self.reach_of, self.per_root, self.per_alone = {}, {}, {}

    def reaches(self, w, depth=0):
        """Every wcid generator `w` can put in the world, five deep."""
        if w in self.reach_of:
            return self.reach_of[w]
        out = set()
        if depth < 5:
            for p in self.profiles.get(w, ()):
                if p["where"] & 56:
                    continue
                out.add(p["wcid"])
                if p["wcid"] in self.profiles and p["wcid"] not in self.attackable:
                    out |= self.reaches(p["wcid"], depth + 1)
        self.reach_of[w] = out
        return out

    def root_figures(self, w, links):
        """(load, per hour) by creature of one root, seeded by the root so the file is the same
        whatever order roots are met in."""
        key = (w, links)
        if key not in self.per_root:
            if w in self.attackable and w not in self.profiles:
                self.per_root[key] = ({w: 1.0}, {})
                return self.per_root[key]
            rng = random.Random("%d:%s" % key)
            load = defaultdict(float)
            for _ in range(LOAD_RUNS):
                s = Sim(self, w, links, rng, lambda c: False)
                s.run(0.0)
                for c, n in s.alive.items():
                    load[c] += n / LOAD_RUNS
            s = Sim(self, w, links, rng, lambda c: True)
            s.run(0.0)
            base = dict(s.made)
            s.run(CLEAR_HOURS * 3600)
            rate = {c: (n - base.get(c, 0)) / CLEAR_HOURS for c, n in s.made.items()}
            # A placed creature that is a generator too stands there itself.
            if self.attackable.get(w):
                load[w] += 1.0
            self.per_root[key] = (dict(load), rate)
        return self.per_root[key]

    def alone(self, w, links, wcid):
        """Creature `wcid` an hour from one root when the player kills it and nothing else."""
        key = (w, links, wcid)
        if key not in self.per_alone:
            s = Sim(self, w, links, random.Random("%d:%s:%d" % key), lambda c: c == wcid)
            s.run(0.0)
            base = s.made.get(wcid, 0)
            s.run(CLEAR_HOURS * 3600)
            self.per_alone[key] = (s.made.get(wcid, 0) - base) / CLEAR_HOURS
        return self.per_alone[key]

    def rates(self, wcid, lb):
        """(load, per hour, others an hour) of creature `wcid` on landblock `lb`: a surface
        landblock's outdoor roots only, as the spawns query counts. Others are what clearing its
        slots kills, counted from the roots it comes from only so (a share of a region roll)."""
        load = rate = others = 0.0
        for w, links, outdoors, n in self.roots.get(lb, ()):
            if lb in self.surface and not outdoors:
                continue
            if not (w == wcid or wcid in links or wcid in self.reaches(w)
                    or any(wcid in self.reaches(c) for c in links)):
                continue
            ld, rt = self.root_figures(w, links)
            load += n * ld.get(wcid, 0.0)
            rate += n * rt.get(wcid, 0.0)
            if rt.get(wcid, 0.0) > 0 and self.alone(w, links, wcid) < ALONE_SHARE * rt[wcid]:
                others += n * (sum(rt.values()) - rt[wcid])
        return load, rate, others


def walk(data, root):
    """Every path from `root`: (branches, effects); a branch the client cannot read is (t, why, a)."""
    out = []
    obj = root["obj"]

    def go(stack, branches, effects, seen):
        if len(out) >= MAX_PATHS:
            return
        while stack:
            s, i = stack[-1]
            if i >= len(s["acts"]):
                stack = stack[:-1]
                continue
            a = s["acts"][i]
            stack = stack[:-1] + [(s, i + 1)]
            t = a["type"]
            if t == GOTO:
                nxt = data.outcome(obj, GOTO_SET, a["msg"])
                if nxt == "random":
                    for alt in data.outcomes(obj, (GOTO_SET,), a["msg"]):
                        if alt["id"] not in seen and len(stack) < DEPTH:
                            go(stack + [(alt, 0)], branches + [(t, "a random reply", a)],
                               effects, seen | {alt["id"]})
                    return
                if nxt is not None and nxt["id"] not in seen and len(stack) < DEPTH:
                    stack = stack + [(nxt, 0)]
                    seen = seen | {nxt["id"]}
                continue
            if t in BRANCHING:
                if t in (INQ_QUEST, INQ_SOLVES):
                    cats = ((QUEST_OK, True), (QUEST_NO, False))
                elif t in (INQ_INT, INQ_OWNS):
                    cats = ((TEST_OK, True), (TEST_NO, False))
                else:
                    # A check the client cannot read: every reply keyed on it is a path.
                    alts = [x for x in data.outcomes(obj, None, a["msg"]) if x["id"] not in seen]
                    why = "a check of type %d" % t
                    if not alts or len(stack) >= DEPTH:
                        go(stack, branches + [(t, why, a)], effects, seen)
                    for alt in alts if len(stack) < DEPTH else []:
                        go(stack + [(alt, 0)], branches + [(t, why, a)], effects,
                           seen | {alt["id"]})
                    return
                for cat, ok in cats:
                    nxt = data.outcome(obj, cat, a["msg"])
                    if nxt == "random":
                        for alt in data.outcomes(obj, (cat,), a["msg"]):
                            if alt["id"] not in seen and len(stack) < DEPTH:
                                go(stack + [(alt, 0)],
                                   branches + [(t, ok, a), (t, "a random reply", a)],
                                   effects, seen | {alt["id"]})
                        continue
                    b = branches + [(t, ok, a)]
                    if nxt is None or nxt["id"] in seen or len(stack) >= DEPTH:
                        go(stack, b, effects, seen)
                    else:
                        go(stack + [(nxt, 0)], b, effects, seen | {nxt["id"]})
                return
            effects = effects + [a]
        out.append((branches, effects))

    go([(root, 0)], [], [], {root["id"]})
    return out


def xp_of(effects):
    """(fixed, percent, min, max, share) of a path's awards (EmoteManager.cs:126-184)."""
    fixed, scaled, share = 0, None, None
    for a in effects:
        if a["type"] in (AWARD_XP, NO_SHARE_XP):
            amt = int(num(a["amount64"] or a["amount"]))
            if amt > 0:
                fixed += amt
                share = share or ("all" if a["type"] == AWARD_XP else "none")
        elif a["type"] == LEVEL_XP and num(a["percent"]) > 0 and scaled is None:
            lo = int(num(a["min64"] or a["min"]))
            hi = int(num(a["max64"] or a["max"]))
            scaled = (num(a["percent"]), lo, hi)
            share = share or "allegiance"
    return fixed, scaled, share


def stamps_of(effects):
    out = []
    for a in effects:
        if a["type"] in (STAMP, SET_SOLVES) and a["msg"]:
            q = qname(a["msg"])
            if q not in out:
                out.append(q)
    return out


def open_max(v, at_least=2147483647):
    return v == "" or num(v) >= at_least


def read_path(data, root, branches, effects, never):
    """A path read into a quest's terms (design §1.4's table), or a string saying why it is out.
    `never`: flags no row of the file stamps, so never held; None on the first pass."""
    stamped = stamps_of(effects)
    r = {"band": [0, 0], "timer": "", "repeat": 0, "kill": None, "items": [], "needs": [],
         "start": None, "unneeds": [], "unread": []}
    if root["cat"] in (GIVE, REFUSE):
        r["items"].append((root["wcid"], 1))
    for t, ok, a in branches:
        if isinstance(ok, str):
            r["unread"].append(ok)
            continue
        q = qname(a["msg"])
        quest = data.quest.get(q)
        counter = q in data.counter_of
        if t == INQ_INT:
            if num(a["stat"]) != LEVEL_STAT:
                r["unread"].append("a check of stat %d" % int(num(a["stat"])))
                continue
            lo, hi = int(num(a["min"])), int(num(a["max"], MAX_LEVEL))
            at_lo, at_hi = r["band"][0], r["band"][1] or MAX_LEVEL
            if ok:
                at_lo, at_hi = max(at_lo, lo), min(at_hi, hi)
            elif lo <= max(at_lo, 1):
                at_lo = max(at_lo, hi + 1)  # a ladder: failing "1-49" is 50 and up
            elif hi >= at_hi:
                at_hi = min(at_hi, lo - 1)
            else:
                r["unread"].append("two level bands")  # outside a band in the middle
                continue
            if at_lo > at_hi:
                r["unread"].append("an empty level band")
                continue
            r["band"] = [at_lo, at_hi]
        elif t == INQ_OWNS:
            if not ok:
                r["unread"].append("not carrying %s" % data.name.get(int(num(a["wcid"])), a["wcid"]))
                continue
            r["items"].append((int(num(a["wcid"])), max(int(num(a["stack"], 1)), 1)))
        elif t == INQ_QUEST and counter:
            if quest is None or quest[1] < 1:
                r["unread"].append("counters not fed by kills")
                continue
            if ok:
                r["kill"] = (q, quest[1])  # held at max_Solves: the count reached
            # else not reached yet: the way to the start talk or the "keep going" reply
        elif t == INQ_SOLVES and counter:
            lo = int(num(a["min"]))
            if ok and lo >= 2:
                r["kill"] = (q, lo)
            elif ok and lo == 1 and open_max(a["max"], 9999):
                pass  # taken up: the log's kill count says so
            elif not ok and lo == 1:
                r["start"] = q
            elif not ok and lo >= 2:
                pass  # not reached yet
            else:
                r["unread"].append("solve-count ranges")
                continue
        elif t == INQ_QUEST and quest is not None and quest[0] > 0:
            if ok:
                r["waiting"] = True  # the giver's reply during the wait: the same quest
                continue
            if r["timer"] and r["timer"] != q:
                r["unread"].append("another quest's wait")
                continue
            r["timer"], r["repeat"] = q, quest[0]
        elif t in (INQ_QUEST, INQ_SOLVES):
            # A flag: held or not (InqQuest: QuestManager.cs:266-280, 334-363).
            lo = int(num(a["min"], 1)) if t == INQ_SOLVES else 1
            if t == INQ_SOLVES and (lo < 1 or not open_max(a["max"])):
                r["unread"].append("solve-count ranges")
                continue
            if t == INQ_QUEST and quest is not None and quest[1] == -1:
                if ok:
                    r["unread"].append("a flag never held: %s" % q)  # unsolvable: never held
                    continue
                continue  # always solvable: the branch asks nothing
            if t == INQ_QUEST and quest is not None and quest[1] > 1:
                r["unread"].append("solve-count ranges")
                continue
            if not ok and q in stamped:
                if not r["timer"]:
                    r["timer"], r["repeat"] = q, -1
                elif r["repeat"] == 0:
                    r["repeat"] = -1
            elif ok and q not in stamped:
                r["needs"].append(q)
            elif not ok and never is not None and q in never:
                continue  # no row stamps it: never held, so the branch asks nothing
            elif not ok:
                r["unneeds"].append(q)  # kept only where holding it pays the same (main)
            else:
                r["unread"].append("asks a flag it stamps itself: %s" % q)
                continue
        else:
            r["unread"].append("a check of type %d" % t)
            continue
    if r["band"][1] >= MAX_LEVEL:
        r["band"][1] = 0
    return r


def write_rows(f, out):
    for row in out:
        assert not any("," in v or "\n" in v for v in row), row
        f.write(",".join(row) + "\n")


# What must match for a path asking a flag not held to pay as one asking it held: then the flag
# does not matter (Buckminster pays the same with ToTheBarkeep or without).
SAME_PAY = ("obj", "kind", "want", "count", "fixed", "scaled", "timer", "repeat")


def same_pay(q):
    return (tuple(q[k] for k in SAME_PAY), q["root"]["id"], tuple(q["band"]),
            tuple(sorted(q["gives"])), tuple(sorted(q["stamps"])))


# Effect types that change nothing for the character: talk, motion, sound, looks, broadcasts
# (EmoteType.cs). A path with any other effect, or one handing an item over, is a quest row.
COSMETIC = {1, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 15, 16, 17, 18, 23, 24, 25, 26, 52, 64, 65, 68,
            72, 77, 78, 87, 88, 91, 92, 93, 94, 95, 96, 97, 98, 116, 117}
# Effect types casting a spell on the character (EmoteType.cs: CastSpell, CastSpellInstant...).
SPELL_TYPES = {14, 19, 27, 73}
# What a path does, for the `does` column: effect types by word.
DOES = (("xp", {2, 49, 62}), ("item", {3, 56}), ("flag", {20, 22, 31, 32, 33, 60, 61, 70,
        106, 107, 108, 109}), ("spell", SPELL_TYPES), ("skill", {28, 29, 47, 50, 110, 111}),
        ("luminance", {112, 113}), ("title", {34}), ("teleport", {99}), ("takes", {74}),
        ("stat", {53, 54, 55, 69, 115, 118, 57, 63, 90, 48}), ("contract", {119, 120}),
        ("enlightenment", {9001}))


def is_harm(data, a):
    """Whether effect `a` casts a spell that hurts whoever it lands on."""
    return a["type"] in SPELL_TYPES and a["spell"] in data.harm


def does_of(data, root, effects):
    """What a path does; a cast that hurts is no reward, so it is left out."""
    types = {a["type"] for a in effects if not is_harm(data, a)}
    out = [w for w, ts in DOES if types & ts]
    if root["cat"] in (GIVE, REFUSE) and "takes" not in out:
        out.append("takes")
    other = sorted(t for t in types if t not in COSMETIC and not any(t in ts for _, ts in DOES))
    return out + ["type %d" % t for t in other]


def build(data, never):
    """The rows and sources of both files, and counts of what the walk could not read; `never`
    as for read_path. Every path that does something for the character is a row."""
    reasons = defaultdict(int)
    paths, starts, npc_gives = [], [], defaultdict(set)
    for obj in sorted(data.placed):
        if data.wtype[obj] not in (10, 12):
            continue
        sets = data.by_obj[obj]
        roots = []
        uses = [s for s in sets if s["cat"] == USE]
        if uses:
            chance = any(s["p"] < 1 for s in uses)
            roots += [(s, chance) for s in (uses if chance else uses[:1])]
        for wcid in sorted({s["wcid"] for s in sets if s["cat"] in (GIVE, REFUSE)}):
            for cat in (REFUSE, GIVE):
                if cat == GIVE and obj not in data.allow_give:
                    break  # Player_Inventory.cs:3439
                keyed = [s for s in sets if s["cat"] == cat and s["wcid"] == wcid]
                if keyed:
                    chance = any(s["p"] < 1 for s in keyed)
                    roots += [(s, chance) for s in (keyed if chance else keyed[:1])]
                    break
        for root, chance in roots:
            found = walk(data, root)
            if len(found) >= MAX_PATHS:
                reasons["roots past %d paths (the rest unwalked)" % MAX_PATHS] += 1
            for branches, effects in found:
                does = does_of(data, root, effects)
                if not does:
                    continue  # talk only: no quest
                if does == ["takes"] and any(is_harm(data, a) for a in effects):
                    # The gift taken and the giver casting harm: a wrong answer punished.
                    reasons["a gift punished with harm"] += 1
                    continue
                fixed, scaled, share = xp_of(effects)
                r = read_path(data, root, branches, effects, never)
                if r.get("waiting"):
                    continue  # the same quest's reply during its wait
                if chance:
                    r["unread"].insert(0, "a random reply")
                r.update(obj=obj, root=root, fixed=fixed, scaled=scaled, share=share,
                         stamps=stamps_of(effects), does=does,
                         gives=[(int(num(a["wcid"])), max(int(num(a["stack"], 1)), 1))
                                for a in effects if a["type"] == GIVE_ITEM and a["wcid"]])
                if root["cat"] == USE and not r["items"]:
                    for wcid, _ in r["gives"]:
                        npc_gives[wcid].add(obj)
                if (root["cat"] == USE and does == ["flag"] and r["stamps"] and not r["items"]
                        and not r["kill"] and not r["unread"]):
                    starts.append(r)
                else:
                    paths.append(r)
                for u in r["unread"]:
                    reasons[u.split(":")[0]] += 1

    def quest_of(r, kind_hint=None):
        items = sorted(set(r["items"]))
        unread = list(r["unread"])
        for w, _ in items[1:]:
            if w != items[0][0]:
                unread.append("also asks for %s" % data.name.get(w, w))
        item = None
        if items:
            wcid = items[0][0]
            item = (wcid, max(n for w, n in items if w == wcid))
        if r["kill"]:
            if item:
                unread.append("also asks for %s" % data.name.get(item[0], item[0]))
                item = None
            kind, want, count = "kill", r["kill"][0], r["kill"][1]
            needs = r["needs"] + [r["kill"][0]]
            qid = r["kill"][0]
        elif item:
            kind, want, count = "item", str(item[0]), item[1]
            needs = r["needs"]
            qid = r["timer"] or (r["stamps"][0] if r["stamps"] else "give:%d" % item[0])
        else:
            kind, want, count = "talk", "", 0
            needs = r["needs"]
            qid = r["timer"] or (r["stamps"][0] if r["stamps"] else "talk:%d" % r["obj"])
        if r["start"]:
            unread.append("a kill task not taken up: %s" % r["start"])
        timer, repeat = r["timer"], r["repeat"]
        if item:
            # A pickup limit on the item limits the quest: one looted per wait, or one ever.
            delta, solves, _ = data.quest.get(data.pickup.get(item[0], ""), (0, 0, ""))
            if repeat != -1 and delta > 0:
                timer, repeat = timer or data.pickup[item[0]], max(repeat, delta)
            elif repeat != -1 and solves == 1:
                timer, repeat = timer or data.pickup[item[0]], -1
            if item[0] in data.restrict:
                needs = needs + [data.restrict[item[0]]]
        return dict(r, id=qid, kind=kind, want=want, count=count, needs=needs, item=item,
                    timer=timer, repeat=repeat, unread=unread)

    quests = [quest_of(r) for r in paths]

    # A path asking a flag not held is the same row as the path asking it held when both pay
    # the same; else it stays, saying which flag it asks to be without.
    by_pay = defaultdict(list)
    for q in quests:
        if not q["unneeds"]:
            by_pay[same_pay(q)].append(q)
    plain = []
    for q in quests:
        if not q["unneeds"]:
            plain.append(q)
            continue
        held = sorted(q["needs"] + q["unneeds"])
        twin = [o for o in by_pay[same_pay(q)] if sorted(o["needs"]) == held]
        for o in twin:
            o["needs"] = list(q["needs"])
        if not twin:
            q["unread"] = q["unread"] + ["only without %s" % f for f in q["unneeds"]]
            plain.append(q)
    quests = plain

    # Start talks: a talk stamping only what another row needs (a kill task's counter) is that
    # quest's start row; any other is a talk quest of its own.
    stamped = {s for q in quests for s in q["stamps"]}
    needed = {n for q in quests for n in q["needs"]}
    start_rows, used = [], set()
    for i, s in enumerate(starts):
        for flag in s["stamps"]:
            if flag in needed and flag not in stamped:
                for q in quests:
                    if flag in q["needs"]:
                        # The start talk's own wait is the quest's when its reward asks none.
                        if s["repeat"] > 0 and not q["timer"]:
                            q["timer"], q["repeat"] = s["timer"], s["repeat"]
                        start_rows.append(dict(s, id=q["id"], kind="start", want="",
                                               count=0, needs=[n for n in s["needs"]
                                                               if n != flag],
                                               item=None, timer="", repeat=0,
                                               stamps=[flag], fixed=0, scaled=None,
                                               share="none", gives=[]))
                        used.add(i)
                        break
    quests += [quest_of(s) for i, s in enumerate(starts) if i not in used]
    kept = quests

    rows_out = []
    seen = {}
    for q in kept + start_rows:
        title = data.quest.get(q["id"], (0, 0, ""))[2]
        if q["id"].startswith("give:"):
            title = data.name.get(int(q["id"][5:]), q["id"])
        elif q["id"].startswith("talk:"):
            title = "Talk to %s" % data.name.get(q["obj"], q["id"])
        title = (title or q["id"]).replace(",", ";")
        pct, lo, hi = q["scaled"] or (0, 0, 0)
        hand = "give" if q["root"]["cat"] in (GIVE, REFUSE) else "talk"
        item = q["item"]
        it_name = data.name.get(item[0], "") if item else ""
        it_type = "%X" % (data.ints[item[0]].get(1, 0) & 0xFFFFFFFF) if item else ""
        it_value = str(data.ints[item[0]].get(19, 0)) if item else ""
        row = (q["id"], q["kind"], title, str(q["obj"]), hand, q["want"], str(q["count"]),
               str(q["band"][0]), str(q["band"][1]), str(q["fixed"]), "%g" % pct, str(lo),
               str(hi), q["share"] or "none", ";".join("%d:%d" % g for g in q["gives"]),
               ";".join(q["stamps"]), str(q["repeat"]), q["timer"], ";".join(q["needs"]),
               it_name, it_type, it_value, ";".join(q["does"]),
               "|".join(u.replace(",", ";") for u in dict.fromkeys(q["unread"])))
        key = row[:-1]  # rows differing only in what was unread are one row
        if key not in seen:
            seen[key] = len(rows_out)
            rows_out.append(row)
        elif row[-1]:
            i = seen[key]
            both = [u for u in rows_out[i][-1].split("|") if u] + row[-1].split("|")
            rows_out[i] = key + ("|".join(dict.fromkeys(both)),)
    rows_out = merge_random(rows_out)
    rows_out = split_by_reward(rows_out, data)
    kind_order = {"start": 0, "talk": 1, "kill": 2, "item": 3}
    rows_out.sort(key=lambda r: (r[0], kind_order[r[1]], int(r[3]), int(r[7])))
    givers_out = sorted({(int(r[3]), cell, x, y, z) for r in rows_out
                         for cell, x, y, z in data.placed[int(r[3])]})
    givers_out = [(str(w), data.name.get(w, ""), "%08X" % cell, x, y, z)
                  for w, cell, x, y, z in givers_out]

    src = []
    # Every recall spell's scroll is had and read (a quest row below); every portal gem is had
    # as a way in. Both are wants whose sources are written.
    learnt = {w: spell for spell in data.recalls for w in data.scrolls_of.get(spell, ())}
    wants = sorted({r[5] for r in rows_out if r[1] in ("kill", "item")}
                   | {str(w) for w in learnt} | {str(w) for w in data.gems})
    seen = set(wants)
    todo = [(w, 0) for w in wants]
    while todo:
        want, depth = todo.pop(0)
        if want.isdigit():
            w = int(want)
            chance = defaultdict(float)
            for c, p in data.drops.get(w, []):
                chance[c] = min(chance[c] + p, 1.0)
            for c in sorted(chance):
                if data.spawns.get(c):
                    src.append((want, "drop", c, chance[c], ""))
            for n in sorted(npc_gives.get(w, ())):
                src.append((want, "npc", n, 1.0, ""))
            if w not in data.stuck and data.made_on_ground(w):
                src.append((want, "floor", w, 1.0, ""))
            for chest, p in sorted(data.chests_holding(w).items()):
                if not data.ground_places(chest):
                    continue
                keys = []
                if chest in data.lock:
                    # A key opens the lock whose LockCode its KeyCode is (Lock.cs:248-280).
                    keys = sorted(data.keys_of.get(data.lock[chest].lower(), ()))
                    if not keys:
                        continue  # a lockpick's lock: nothing autoplay can open
                src.append((want, "container", chest, p,
                            ";".join("%d=%s" % (k, data.name.get(k, "").replace("=", "-"))
                                     for k in keys)))
                for k in keys:
                    if str(k) not in seen and depth < KEY_DEPTH:
                        seen.add(str(k))
                        todo.append((str(k), depth + 1))
            for on, used in sorted(data.made_on.get(w, {}).items()):
                # A world object, never picked up, standing somewhere: the item is used on it.
                if on not in data.stuck or not data.ground_places(on):
                    continue
                keys = sorted(used)
                src.append((want, "use", on, 1.0,
                            ";".join("%d=%s" % (k, data.name.get(k, "").replace("=", "-"))
                                     for k in keys)))
                for k in keys:
                    if str(k) not in seen and depth < KEY_DEPTH:
                        seen.add(str(k))
                        todo.append((str(k), depth + 1))
            for used, on in sorted(data.combined.get(w, ())):
                # Two items carried, the first used on the second: both are had first.
                keys = [used, on]
                src.append((want, "combine", on, 1.0,
                            ";".join("%d=%s" % (k, data.name.get(k, "").replace("=", "-"))
                                     for k in keys)))
                for k in keys:
                    if str(k) not in seen and depth < KEY_DEPTH:
                        seen.add(str(k))
                        todo.append((str(k), depth + 1))
        else:
            for c in sorted(data.counter_of[want]):
                if data.spawns.get(c):
                    src.append((want, "kill", c, 1.0, ""))
    src = kept_uses(src, wants, data.sold)
    had = {r[0] for r in src} | {str(w) for w in data.sold}
    rows_out = rows_out + learn_rows(data, learnt, had)
    rows_out.sort(key=lambda r: (r[0], kind_order[r[1]], int(r[3]), int(r[7])))
    src_out = []
    for want, kind, c, p, keys in src:
        at = ""
        if kind == "combine":
            where = ""
        elif kind in ("npc", "floor", "container", "use"):
            if kind == "npc":
                spots = data.placed[c]
            else:
                spots = sorted(set(data.made_on_ground(c) if kind == "floor"
                                   else data.ground_places(c)))
            where = ";".join("%08X:1" % cell for cell, _, _, _ in spots)
            at = ";".join("%s %s %s" % (x, y, z) for _, x, y, z in spots)
        else:
            where = ";".join("%X:%d" % lb for lb in sorted(data.spawns[c]))
            # A creature the encounter table alone puts out: where on each landblock.
            if c in data.met_at:
                at = ";".join("%g %g 0" % xy for xy in data.met_at[c])
        # No chain known (an emote's summons, a linked placement): no figure, not a rare one.
        share = data.spawn_share(c) if kind in ("drop", "kill") else 0.0
        spawn = "%g" % round(share, 4) if share > 0 else ""
        rates = ""
        if kind in ("drop", "kill"):
            figures = (data.spawning.rates(c, lb) for lb, _ in sorted(data.spawns[c]))
            rates = ";".join("%g:%g%s" % (round(n, 2), round(h, 1),
                                          ":%g" % round(o) if round(o) else "")
                             for n, h, o in figures)
        src_out.append((want, kind, str(c), data.name.get(c, ""), str(data.level(c)),
                        "%g" % round(p, 4), where, at, keys, spawn, rates))
    return rows_out, givers_out, src_out, reasons


def learn_rows(data, learnt, had):
    """A quest row per recall spell's scroll something gives: had, then read (hand `read`, no
    giver), done once the spell is known; its id names the spell (`learn:2023`)."""
    out = []
    for wcid, spell in sorted(learnt.items()):
        if str(wcid) not in had:
            continue
        ints = data.ints[wcid]
        out.append(("learn:%d" % spell, "item", data.recalls[spell].replace(",", ";"), "0",
                    "read", str(wcid), "1", "0", "0", "0", "0", "0", "0", "none", "", "", "-1",
                    "", "", data.name.get(wcid, ""), "%X" % (ints.get(1, 0) & 0xFFFFFFFF),
                    str(ints.get(19, 0)), "spell", ""))
    return out


def kept_uses(src, wants, sold):
    """`src` less each use whose items used on it have no source, and less the rows of an item
    then wanted by nothing kept."""
    while True:
        had = {r[0] for r in src} | {str(w) for w in sold}
        items = lambda r: [k.split("=", 1)[0] for k in r[4].split(";")]
        keep = [r for r in src if r[1] not in ("use", "combine")
                or (r[1] == "use" and any(k in had for k in items(r)))
                or (r[1] == "combine" and all(k in had for k in items(r)))]
        asked = set(wants) | {k.split("=", 1)[0] for r in keep for k in r[4].split(";") if k}
        keep = [r for r in keep if r[0] in asked]
        if len(keep) == len(src):
            return keep
        src = keep


# Landblocks kept per creature the encounter table alone spawns: where it is likeliest.
ENCOUNTER_BLOCKS = 64

# Columns of a quests.csv row a random reply's alternatives may differ in and still be one row.
COL_GIVES, COL_DOES, COL_UNREAD = 14, 22, 23


def merge_random(rows_out):
    """Rows of one random reply differing only in what it gives are one row listing every
    alternative, |-separated (an empty one gives nothing), and what any of them does."""
    out, at = [], {}
    for row in rows_out:
        if "a random reply" not in row[COL_UNREAD].split("|"):
            out.append(row)
            continue
        key = row[:COL_GIVES] + row[COL_GIVES + 1:COL_DOES] + row[COL_DOES + 1:]
        if key not in at:
            at[key] = len(out)
            out.append(row)
            continue
        was = out[at[key]]
        gives = "|".join(dict.fromkeys(was[COL_GIVES].split("|") + [row[COL_GIVES]]))
        words = was[COL_DOES].split(";") + row[COL_DOES].split(";")
        does = ";".join(dict.fromkeys(w for w in words if w))
        out[at[key]] = (was[:COL_GIVES] + (gives,) + was[COL_GIVES + 1:COL_DOES] + (does,)
                        + was[COL_DOES + 1:])
    return out


def split_by_reward(rows_out, data):
    """A handed item's quest whose givers reward it differently (the augmentation trainers each
    trade a Blank Augmentation Gem for their own gem) is one quest per reward, so the player
    chooses which: id give:ITEM:REWARD, the title naming the reward."""
    rewards, does = defaultdict(set), {}
    for r in rows_out:
        if r[0].startswith("give:") and r[1] != "start":
            rewards[r[0]].add(r[COL_GIVES])
            words = does.setdefault((r[0], r[COL_GIVES]), {})
            words.update(dict.fromkeys(r[COL_DOES].split(";")))
    suffix = {}
    for qid, gives in rewards.items():
        if len(gives) < 2:
            continue
        taken = set()
        for g in sorted(gives):
            first = g.split("|")[0].split(";")[0].split(":")[0] or "none"
            s, n = first, 2
            while s in taken:
                s, n = "%s-%d" % (first, n), n + 1
            taken.add(s)
            if first.isdigit():
                name = data.name.get(int(first), first)
            else:
                # No item: the reward is what the path does ("luminance", "xp").
                words = [{"xp": "XP"}.get(w, w) for w in does[(qid, g)]
                         if w not in ("", "takes")]
                name = " and ".join(words) or "nothing"
            suffix[(qid, g)] = (s, name.replace(",", ";"))
    out = []
    for r in rows_out:
        if r[1] == "start" and len(rewards.get(r[0], ())) > 1:
            # A start talk takes up every reward's quest.
            for (qid, _), (s, name) in sorted(suffix.items()):
                if qid == r[0]:
                    out.append(("%s:%s" % (qid, s), r[1], "%s for %s" % (r[2], name)) + r[3:])
            continue
        s = suffix.get((r[0], r[COL_GIVES]))
        if s is None:
            out.append(r)
            continue
        out.append(("%s:%s" % (r[0], s[0]), r[1], "%s for %s" % (r[2], s[1])) + r[3:])
    return out


def main():
    dump, out_dir = sys.argv[1], sys.argv[2]
    data = Data(dump)
    vendors = os.path.join(out_dir, "vendors.csv")
    if os.path.exists(vendors):
        with open(vendors, encoding="utf-8") as f:
            data.sold = {int(r.split(",")[6]) for r in f if r[:1].isdigit()}
    with open(os.path.join(out_dir, "recalls.csv"), encoding="utf-8") as f:
        for r in f:
            if r[:1].isdigit():
                c = r.rstrip("\n").split(",")
                data.recalls[int(c[0])] = c[1]
    with open(os.path.join(out_dir, "gems.csv"), encoding="utf-8") as f:
        data.gems = sorted({int(r.split(",")[0]) for r in f if r[:1].isdigit()})
    rows_out, _, _, _ = build(data, None)
    stamped = {f for r in rows_out for f in r[15].split(";") if f}
    asked = {qname(a["msg"]) for s in data.sets.values() for a in s["acts"]
             if a["type"] in (INQ_QUEST, INQ_SOLVES) and a["msg"]}
    rows_out, givers_out, src_out, reasons = build(data, asked - stamped)
    with open(os.path.join(out_dir, "quests.csv"), "w", encoding="utf-8", newline="") as f:
        f.write(QUESTS_HEADER)
        write_rows(f, rows_out)
    with open(os.path.join(out_dir, "quest_givers.csv"), "w", encoding="utf-8",
              newline="") as f:
        f.write(GIVERS_HEADER)
        write_rows(f, givers_out)
    with open(os.path.join(out_dir, "quest_sources.csv"), "w", encoding="utf-8",
              newline="") as f:
        f.write(SOURCES_HEADER)
        write_rows(f, src_out)
    with open(os.path.join(out_dir, "pickups.csv"), "w", encoding="utf-8", newline="") as f:
        f.write(PICKUPS_HEADER)
        write_rows(f, pickup_rows(data))
    by_kind = defaultdict(set)
    for r in rows_out:
        by_kind[r[1]].add(r[0])
    print("quests: %d rows, %d giver placements; %s; out: %s" % (
        len(rows_out), len(givers_out), ", ".join("%s %d" % (k, len(v)) for k, v in sorted(by_kind.items())),
        dict(sorted(reasons.items()))), file=sys.stderr)


def pickup_rows(data):
    """Each item with a pickup limit (Quest 33) whose quest is known: wcid, stamp, wait, solves."""
    out = []
    for wcid, stamp in sorted(data.pickup.items()):
        if stamp in data.quest:
            delta, solves, _ = data.quest[stamp]
            out.append([str(wcid), stamp, str(delta), str(solves)])
    return out


PICKUPS_HEADER = """\
# Items whose pickup the server limits (an item's Quest, PropertyString 33): picked up while its
# stamp waits, "You have solved this quest too recently!" (Player_Inventory.cs:1249-1262).
# wcid,stamp,wait,solves: the stamp (lower case, as the quest log keys it), seconds between
# pickups (0 none), and the most pickups ever (-1 no limit). Regenerate with
# reference/scripts/data/quests.sh.
"""


QUESTS_HEADER = """\
# Every quest a placed NPC holds: each Use, Give or Refuse path that does anything for the
# character, XP or not, one row per way to complete one (giver and level band); the giver's
# placements are in quest_givers.csv. And one per recall spell's scroll, had and read.
# id,kind,title,giver_wcid,hand,want,count,min_level,max_level,xp,xp_level,xp_min,xp_max,share,gives,stamps,repeat,timer,needs,item,item_type,item_value,does,unread
# Rows sharing an id are alternatives. kind: kill, item, talk, or start (the talk that takes a
# quest up: it stamps what the quest's rows need). hand: give (hand the item over), talk, or
# read (a recall spell's scroll read: id learn:SPELL, giver_wcid 0, no giver).
# want: the item wcid, or the kill counter. count: items one hand-in takes, or kills. Levels 0
# are open. xp is fixed; xp_level is a share of the XP to the next level, clamped to
# xp_min/xp_max when not 0. share: all, none or allegiance. gives: wcid:count;..., and for a
# random reply every alternative |-separated (an empty one gives nothing). stamps and needs:
# quest flags. repeat: seconds before another solve, 0 any time, -1 once; timer: the quest its
# wait is kept under. item, item_type (hex), item_value: the wanted item. does: what the path
# does (xp, item, flag, spell, skill, luminance, title, teleport, takes, stat, contract,
# enlightenment, type N). unread: conditions the client cannot check, |-separated ("a random
# reply" among them). Regenerate with reference/scripts/data/quests.sh (reads the ACE world
# database, the community's reconstruction of retail's server data).
"""

GIVERS_HEADER = """\
# Where each quest giver in quests.csv stands: one row per placement.
# giver_wcid,giver,cell,x,y,z
# cell is hex, x/y/z local to its landblock. Regenerate with reference/scripts/data/quests.sh.
"""

SOURCES_HEADER = """\
# Where what a quest wants comes from, and what each source itself needs.
# want,kind,wcid,name,level,chance,landblocks,at,keys,spawn,rates
# want: an item wcid or a kill counter, as quests.csv names it, or a key a container or use
# below needs. kind: drop (a corpse drop, chance per kill), kill (a creature counting for the
# counter), npc (a placed NPC whose talk gives the item), floor (made on the ground by a
# generator; wcid is the item's own), container (a chest that holds it, chance per fill), use
# (a placed object a recipe makes it on, from a carried item used on it) or combine (a carried
# item a recipe makes it on; nowhere to go). landblocks: hex landblock:spawn entries;... (for
# an npc, floor, container or use, hex cell:1 per placement, with at the matching local
# "x y z";...; for a drop or kill the encounter table alone spawns, each landblock's busiest
# encounter corner, "x y 0";...). keys: for a locked container, the keys whose KeyCode is its LockCode; for a
# use, the items used on it; for a combine, the item used then the one used on;
# wcid=name;..., each a want of its own rows. A shop's stock is vendors.csv's, joined at load (ac_world::quests::sources).
# spawn: for a drop or kill, the best share of a generator roll that puts the creature out (1 sure;
# blank when no generator chain is known). rates: for a drop or kill, load:per_hour[:others];...
# one per landblocks entry: how many stand there once the landblock loads, how many its
# generators make an hour while everything they make is killed, and when killing it alone leaves
# its slots held, the others an hour that clearing kills; from a seeded simulation of ACE's
# generators.
# Regenerate with reference/scripts/data/quests.sh.
"""

if __name__ == "__main__":
    main()
