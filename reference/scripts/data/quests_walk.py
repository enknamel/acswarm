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
DEATH = 3
# StartEvent, and CastSpellInstant (a creature's portal summon at its death).
START_EVENT, CAST_INSTANT = 23, 19
# ReceiveLocalSignal (EmoteCategory.cs), and a watcher's own counter: StampMyQuest and
# InqMyQuestSolves (EmoteType.cs).
RECEIVE_SIGNAL, STAMP_MY, INQ_MY_SOLVES = 37, 81, 82
# LocalSignal, Generate (a generator's profiles made now) and StampFellowQuest (EmoteType.cs).
LOCAL_SIGNAL, GENERATE, STAMP_FELLOW = 88, 72, 61
# CastSpell, and TeleportTarget: the one it is run for moved to its position (EmoteType.cs).
CAST_SPELL, TELEPORT = 14, 99
# GameEventState.On: an event the server starts with (EventManager.cs:55-57).
EVENT_ON = 4
QUEST_OK, QUEST_NO, TEST_OK, TEST_NO, GOTO_SET = 12, 13, 22, 23, 32
# EmoteType (ACE.Entity/Enum/EmoteType.cs).
AWARD_XP, GIVE_ITEM, INQ_QUEST, STAMP, INQ_SOLVES = 2, 3, 21, 22, 30
INQ_INT, LEVEL_XP, NO_SHARE_XP, GOTO, SET_SOLVES, TAKE, INQ_OWNS = 36, 49, 62, 67, 70, 74, 76
AWARD_LUMINANCE = 113
# Every branching type (EmoteManager.EmoteIsBranchingType); the ones not read above end a path.
BRANCHING = {20, 21, 30, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 51, 58, 59, 60, 67, 71,
             75, 76, 79, 80, 82, 89, 102, 103, 104, 105, 114, 121}
LEVEL_STAT = 25
# InqEvent, asking whether a server event runs, and its replies (EmoteType.cs, EmoteCategory.cs).
INQ_EVENT, EVENT_OK, EVENT_NO = 51, 27, 28
EVENT_UNREAD = "a check of type %d" % INQ_EVENT
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
# Events whose starters are looked through for what a gated creature needs: a chain of server
# events (the Aerlinthe eruption runs seven deep, a sluice bar to Lady Aerfalle's chest).
EVENT_DEPTH = 10


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
        # Put out by a placed generator, at it: quests.sh's madenpc.
        for name in ("placed", "madenpc"):
            for wcid, cell, x, y, z, t in rows(dump, name):
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
                              "spell": int(num(f[14])) if len(f) > 14 else 0,
                              "cell": f[15] if len(f) > 18 else "",
                              "at": tuple(f[16:19]) if len(f) > 18 else (),
                              "delay": num(f[19]) if len(f) > 19 else 0.0})
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
        # A creature with no corpse drops a generator on the ground where it died
        # (Creature_Death.cs:431-443): what that makes there is the creature's drop too (the
        # Dull Aetherium Ore Stockpile's nine fragments).
        nocorpse = {int(o) for (o,) in rows(dump, "nocorpse")}
        for item, makers in list(self.made_by.items()):
            for gen, chance, where, _ in makers:
                if not where & (8 | 16 | 32 | 64):
                    self.drops[item].extend((c, p * chance) for c, p in self.drops.get(gen, ())
                                            if c in nocorpse)
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
        self.components = {int(o) for (o,) in rows(dump, "components")}
        # What an item used on a placed object makes: made -> object -> items used on it.
        self.made_on = defaultdict(lambda: defaultdict(set))
        for made, used, on in rows(dump, "recipes"):
            self.made_on[int(made)][int(on)].add(int(used))
        # What one carried item used on another makes: made -> (item used, item it is used on).
        # One used on a stuck object a generator places (a Deposit of Chorizite Ore, struck with a
        # Lugian Pick Axe) is a use on that object where it stands, as never carried.
        self.combined = defaultdict(set)
        for made, used, on in rows(dump, "combines"):
            if int(on) in self.stuck:
                self.made_on[int(made)][int(on)].add(int(used))
            else:
                self.combined[int(made)].add((int(used), int(on)))
        # Items a shop sells (vendors.csv, joined at load): a source the walk does not write.
        self.sold = set()
        self.spawning = Spawning(dump)
        # The same with every event on: how fast a creature an event gates comes once started.
        self.spawning_on = Spawning(dump, events_on=True)
        # Each generator's event and that event's state at server start.
        self.gen_event = {int(g): (name, int(state)) for g, name, state in rows(dump, "genevent")}
        # The events on at server start (GameEventState On, 4).
        self.event_on = {name.lower() for name, state in rows(dump, "events") if int(state) == 4}
        self.summon_link = {int(i): int(link) for i, link in rows(dump, "summonspell")}
        self.link_portal = {}
        for obj, t, portal, cell, x, y, z, lo, hi, quest in rows(dump, "linkportal"):
            self.link_portal[(int(obj), int(t))] = (int(portal), int(cell, 16), x, y, z,
                                                    int(lo), int(hi), quest)
        self.event_name = {}
        self.start_needs = defaultdict(set)
        self.starts, self.summons, self.stamps = event_starts(self)
        # The events a generator portal waits on (event_portal_rows): their starters are written.
        self.portal_events = set()
        # Who sends each signal as it dies (lower case: a set's key matches it whatever its case,
        # EmoteManager.cs:1526), the landblocks each placed object stands in, and what hears it.
        self.death_signals = defaultdict(set)
        for obj, message in rows(dump, "deathsignals"):
            self.death_signals[message.lower()].add(int(obj))
        self.inst_blocks = defaultdict(set)
        for _, wcid, cell in rows(dump, "inst"):
            self.inst_blocks[int(wcid)].add(int(cell, 16) >> 16)
        self.signal_starts = signal_starts(self)
        # Each flag a portal stamps as it is gone through, and where each such portal stands.
        self.portal_stamps, self.portal_placed = defaultdict(set), defaultdict(list)
        for portal, flag, cell, x, y, z in rows(dump, "portalstamps"):
            self.portal_stamps[qname(flag)].add(int(portal))
            if cell:
                self.portal_placed[int(portal)].append((int(cell, 16), x, y, z))
        # Teleport spells' destinations, portals' relative destinations, and hearing radii.
        self.spell_pos = {int(i): (int(c, 16), x, y, z) for i, c, x, y, z in rows(dump, "spellpos")}
        self.rel_dest = {int(o): (num(x), num(y), num(z)) for o, x, y, z in rows(dump, "reldest")}
        self.hearing = {int(o): num(v) for o, v in rows(dump, "hearing")}

    def gate_of(self, wcid, depth=0):
        """(made, event): whether anything puts `wcid` in the world, and when every way that does
        runs through an Off event's generator, that event (lower case); placed is never gated."""
        if wcid in self.spawning.placed:
            return True, None
        if depth > GEN_DEPTH + 2:
            return False, None
        made, gated = False, None
        makers = [(g, 0) for g, _, where, _ in self.made_by.get(wcid, ()) if not where & 56]
        makers += [(g, 0) for g in self.spawning.link_parent.get(wcid, ())]
        for gen, _ in makers:
            name, state = self.gen_event.get(gen, ("", EVENT_ON))
            if name and state != EVENT_ON:
                made = True
                gated = gated or name.lower()
                self.event_name.setdefault(name.lower(), name)
                continue
            up, ev = self.gate_of(gen, depth + 1)
            if up and ev is None:
                return True, None
            if up:
                made, gated = True, gated or ev
        return made, gated

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
# Power-ups averaged for what a generator a generator makes puts out at once.
FILL_RUNS = 20
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
        if not self.stops(now):
            self.pick(now, rng.uniform(0.0, self.total(now)), out)

    def ways(self, now):
        """Each way one roll can go now: a roll inside it, and its share of rolls. The picks change
        only at a threshold, so a roll between two stands for all between them."""
        if self.stops(now):
            return []
        total = self.total(now)
        if total <= 0.0:
            return [(0.0, 1.0)]
        cuts, tot, last = {0.0, total}, 0.0, 0.0
        for s in self.slots:
            if s.prob == -1:
                continue
            if not self.maxed(s) and now > s.avail:
                if last > s.prob:
                    last = 0.0
                tot += s.prob - last
                if 0.0 < tot < total:
                    cuts.add(tot)
            last = s.prob
        cuts = sorted(cuts)
        return [((a + b) / 2, (b - a) / total) for a, b in zip(cuts, cuts[1:])]

    def pick(self, now, roll, out):
        """The picks of a roll of `roll` (SelectProfiles' loop)."""
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


def drawn_or_expected(drawn, expected):
    """Each creature's figure as the runs drew it, or what their rolls make on average where they
    never drew it: a refill of a camp a rare pick made is counted only once it is drawn, so the
    average alone reads it low (a Mire-Witch shore's 3 an hour as 0.2)."""
    out = {c: n for c, n in expected.items() if n > 0}
    out.update({c: n for c, n in drawn.items() if n > 0})
    return out


class Sim:
    """One root generator's tree in time; `kills(wcid)` says whether the player kills one at once."""

    def __init__(self, sp, root, links, rng, kills):
        self.sp, self.rng, self.kills = sp, rng, kills
        self.heap, self.seq, self.pending = [], 0, []
        self.made, self.alive = defaultdict(int), defaultdict(int)
        # What each roll makes on average, over every way it could go: a 0.2% pick (Aun Ralirea)
        # counts at its share instead of at 0 in a run that never draws it (`drawn_or_expected`).
        self.expect = defaultdict(float)
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
                self.roll(g, now)
                n += 1
            g.powering = False
        else:
            self.roll(g, now)

    def roll(self, g, now):
        """One roll: what it makes on average counted, then one way it goes drawn. A generator a
        roll makes was counted whole by its parent's roll ([`Spawning.first_fill`]), so its own
        power-up is not counted again."""
        if not (g.powering and g.parent is not None):
            for roll, share in g.ways(now):
                picks = []
                g.pick(now, roll, picks)
                for s in picks:
                    for c, n in self.sp.yields(s.wcid).items():
                        self.expect[c] += share * n
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

    def __init__(self, dump, events_on=False, counted=()):
        self.profiles = defaultdict(list)
        for gen, prob, wcid, delay, init, mx, where in rows(dump, "profiles"):
            self.profiles[int(gen)].append(dict(
                prob=num(prob), wcid=int(wcid), delay=None if delay == "" else num(delay),
                init=int(init), max=int(mx), where=int(where)))
        self.genprops = {}
        for wcid, kind, regen, init, mx, ad, event in rows(dump, "genprops"):
            if events_on and event == "off":
                event = "1"
            self.genprops[int(wcid)] = (int(kind), num(regen), int(init), int(mx), int(ad),
                                        event)
        # Creatures, by whether one can be attacked (PropertyBool 19, default true).
        self.attackable = {int(w): a != "0" for w, a in rows(dump, "crit")}
        # And what else is counted as one is: a portal a camp makes, taken as it comes.
        self.attackable.update((w, True) for w in counted)
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
        # Put out by the landblock itself, and a linked placement's generator by its wcid.
        self.placed = {r[0] for rs in self.roots.values() for r in rs}
        self.link_parent = defaultdict(set)
        guid_wcid = {int(g): int(w) for g, w, _ in rows(dump, "inst")}
        for parent, child, wcid in rows(dump, "links"):
            if int(parent) in guid_wcid:
                self.link_parent[int(wcid)].add(guid_wcid[int(parent)])
        self.reach_of, self.per_root, self.per_alone = {}, {}, {}
        self.filled = {}

    def yields(self, w):
        """What one `w` a roll picks puts in the world on average, by creature: itself, a
        generator's first fill, or nothing (an item, or a creature nobody can attack)."""
        if w in self.profiles and w not in self.attackable:
            return self.first_fill(w)
        return {w: 1.0} if self.attackable.get(w) else {}

    def first_fill(self, w):
        """Creatures generator `w` puts out on average as it powers up, once made."""
        if w not in self.filled:
            self.filled[w] = {}  # a generator making itself counts nothing more
            out = defaultdict(float)
            rng = random.Random("fill:%d" % w)
            for _ in range(FILL_RUNS):
                s = Sim(self, w, (), rng, lambda c: False)
                s.run(0.0)
                for c, n in s.expect.items():
                    out[c] += n / FILL_RUNS
            self.filled[w] = dict(out)
        return self.filled[w]

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
            seen, expect = defaultdict(float), defaultdict(float)
            for _ in range(LOAD_RUNS):
                s = Sim(self, w, links, rng, lambda c: False)
                s.run(0.0)
                for c, n in s.alive.items():
                    seen[c] += n / LOAD_RUNS
                for c, n in s.expect.items():
                    expect[c] += n / LOAD_RUNS
            load.update(drawn_or_expected(seen, expect))
            s = Sim(self, w, links, rng, lambda c: True)
            s.run(0.0)
            made, expect = dict(s.made), dict(s.expect)
            s.run(CLEAR_HOURS * 3600)
            rate = drawn_or_expected(
                {c: (n - made.get(c, 0)) / CLEAR_HOURS for c, n in s.made.items()},
                {c: (n - expect.get(c, 0)) / CLEAR_HOURS for c, n in s.expect.items()})
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
            made, expect = s.made.get(wcid, 0), s.expect.get(wcid, 0)
            s.run(CLEAR_HOURS * 3600)
            self.per_alone[key] = drawn_or_expected(
                {wcid: (s.made.get(wcid, 0) - made) / CLEAR_HOURS},
                {wcid: (s.expect.get(wcid, 0) - expect) / CLEAR_HOURS}).get(wcid, 0.0)
        return self.per_alone[key]

    def rates(self, wcid, lb):
        """(load, per hour, others an hour) of creature `wcid` on landblock `lb`: a surface
        landblock's outdoor roots only, its dungeon's when none outdoors put it out, as the spawns
        query counts. Others are what clearing its slots kills, counted from the roots it comes
        from only so (a share of a region roll)."""
        load = rate = others = 0.0
        def puts_out(w, links):
            return (w == wcid or wcid in links or wcid in self.reaches(w)
                    or any(wcid in self.reaches(c) for c in links))
        roots = [r for r in self.roots.get(lb, ()) if puts_out(r[0], r[1])]
        if lb in self.surface and any(r[2] for r in roots):
            roots = [r for r in roots if r[2]]
        for w, links, _, n in roots:
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
                elif t == INQ_EVENT:
                    cats = ((EVENT_OK, True), (EVENT_NO, False))
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


def event_starts(data):
    """What starts each event, {event: {(kind, obj, item)}} (lower case): give (`item` handed to
    `obj`), kill (`obj`'s death) or use; what a creature's death summons, {creature: (portal,
    cell, x, y, z)} (its CastSpellInstant of a portal summon: Relic Watchman's Aerfalle Keep);
    and whose death stamps each flag on its killer, {flag: {creature}} (lower case: the Guardian
    of the Singularity's VirindiIsland; EmoteManager.cs:1998-2008 runs it for the last damager)."""
    starts, summons, stamps = defaultdict(set), {}, defaultdict(set)
    kinds = {GIVE: "give", DEATH: "kill", USE: "use"}
    objs = {s["obj"] for s in data.sets.values()
            if any(a["type"] in (START_EVENT, CAST_INSTANT) for a in s["acts"])
            or s["cat"] == DEATH and any(a["type"] == STAMP for a in s["acts"])}
    for obj in sorted(objs):
        for s in data.by_obj[obj]:
            kind = kinds.get(s["cat"])
            if kind is None or (kind == "give" and not s["wcid"]):
                continue
            for branches, effects in walk(data, s):
                for a in effects:
                    if a["type"] == START_EVENT and a["msg"]:
                        data.event_name.setdefault(a["msg"].lower(), a["msg"])
                        item = s["wcid"] if kind == "give" else 0
                        starts[a["msg"].lower()].add((kind, obj, item))
                        # The events a use asks running on its way there (the Font Guardian's
                        # four heat events, each a Sealed Valve's): started first.
                        if kind == "use":
                            asks = {b[2]["msg"].lower() for b in branches
                                    if b[0] == INQ_EVENT and b[1] is True and b[2]["msg"]}
                            data.start_needs[(a["msg"].lower(), obj)] |= asks
                    elif (a["type"] == CAST_INSTANT and kind == "kill"
                          and a["spell"] in data.summon_link):
                        did = 31 if data.summon_link[a["spell"]] <= 1 else 48
                        if (obj, did) in data.link_portal:
                            summons[obj] = data.link_portal[(obj, did)]
                    elif a["type"] == STAMP and kind == "kill" and a["msg"]:
                        stamps[a["msg"].lower()].add(obj)
    return starts, summons, stamps


def reached(data, obj, s, depth=0):
    """The actions set `s` runs whatever happens: its own and those of the GotoSet sets its Gotos
    run, DEPTH deep (a branch's outcome is not followed)."""
    out = []
    for a in s["acts"]:
        out.append(a)
        if a["type"] == GOTO and depth < DEPTH:
            nxt = data.outcome(obj, GOTO_SET, a["msg"])
            if isinstance(nxt, dict):
                out += reached(data, obj, nxt, depth + 1)
    return out


def signal_starts(data):
    """Each event a watcher starts on hearing a signal, {event: {(watcher, signal, count)}} (lower
    case): its ReceiveLocalSignal set (OnLocalSignal, EmoteManager.cs:2055-2058) starts it outright
    (count 1), or stamps its own counter (StampMyQuest, :1265-1278) and asks InqMyQuestSolves for
    `count` of it (:666-676), whose QuestSuccess set starts it (StartEvent, :1287-1290): the Stone
    Fists Event Watcher's eight golems."""
    out = defaultdict(set)

    def start(obj, s, acts, count):
        for a in acts:
            if a["type"] == START_EVENT and a["msg"]:
                data.event_name.setdefault(a["msg"].lower(), a["msg"])
                out[a["msg"].lower()].add((obj, s["quest"], count))

    for obj in sorted(data.by_obj):
        for s in data.by_obj[obj]:
            if s["cat"] != RECEIVE_SIGNAL or not s["quest"]:
                continue
            acts = reached(data, obj, s)
            start(obj, s, acts, 1)
            counted = {qname(a["msg"]) for a in acts if a["type"] == STAMP_MY and a["msg"]}
            for a in acts:
                if a["type"] != INQ_MY_SOLVES or qname(a["msg"]) not in counted:
                    continue
                ok = data.outcome(obj, QUEST_OK, a["msg"])
                count = int(num(a["min"]))
                if isinstance(ok, dict) and count >= 1:
                    start(obj, s, reached(data, obj, ok), count)
    return out


def way_of(data, obj, effects):
    """Where a path's effects send whoever it runs for, and the seconds of its emotes' delays till
    then (each before its action: EmoteManager.cs): a portal its owner summons ((portal, cell, x,
    y, z, min level, max level, flag), secs) or a teleport ((0, cell, x, y, z, 0, 0, ""), secs)."""
    secs = 0.0
    for a in effects:
        secs += a["delay"]
        if a["type"] == TELEPORT and a["cell"]:
            return (0, int(a["cell"], 16)) + a["at"] + (0, 0, ""), secs
        if a["type"] in (CAST_SPELL, CAST_INSTANT) and a["spell"] in data.summon_link:
            did = 31 if data.summon_link[a["spell"]] <= 1 else 48
            if (obj, did) in data.link_portal:
                return data.link_portal[(obj, did)], secs
        # A sending cast on whoever it runs for (EmoteManager.cs:223-233) carries them to the
        # spell's position (WorldObject_Magic.cs:323-325): Qin Xikit's temple Guardian.
        if a["type"] in (CAST_SPELL, CAST_INSTANT) and a["spell"] in data.spell_pos:
            cell, x, y, z = data.spell_pos[a["spell"]]
            return (0, cell, x, y, z, 0, 0, ""), secs
    return None


def event_starters(data, events, cell):
    """For each event a way asks running, a placed object in `cell`'s landblock whose use starts
    it (the Eyes of the Watcher fuelling Browerk's Portal Device), as `wcid:CELL:x:y:z`; None when
    one has none."""
    out = set()
    for ev in sorted(set(events)):
        at = sorted((o, pc, px, py, pz) for kind, o, _ in data.starts.get(ev, ()) if kind == "use"
                    for pc, px, py, pz in data.placed.get(o, ()) if pc >> 16 == cell >> 16)
        if not at:
            return None
        o, pc, px, py, pz = at[0]
        out.add("%d:%08X:%s:%s:%s" % (o, pc, px, py, pz))
    return sorted(out)


def npc_ways(data, never):
    """Each way a placed NPC opens for whoever hands it an item or uses it: a portal it summons
    (Faladha the Emissary's to Aerlinthe for a 10,000 note) or a teleport. Rows: npc, name, cell,
    x, y, z, item handed (0: a use), flags held first, level band (0: open), portal (0: a teleport),
    to cell, x, y, z, seconds of emote delays first. Paths asking what the client cannot read, a
    wait or a kill count are out."""
    out = set()
    for obj, places in sorted(data.placed.items()):
        for root in data.by_obj.get(obj, ()):
            if root["cat"] not in (GIVE, USE) or (root["cat"] == GIVE and not root["wcid"]):
                continue
            for branches, effects in walk(data, root):
                found = way_of(data, obj, effects)
                if found is None:
                    continue
                way, secs = found
                r = read_path(data, root, branches, effects, never)
                unread = [u for u in r["unread"] if u != EVENT_UNREAD]
                if unread or r["kill"] or r["timer"] or r["start"] or r["unneeds"]:
                    continue
                if r.get("waiting") or r.get("event_off") or len(r["items"]) > (root["cat"] == GIVE):
                    continue
                portal, cell, x, y, z, lo, hi, flag = way
                band_lo = max(r["band"][0], lo)
                band_hi = min(h for h in (r["band"][1], hi, MAX_LEVEL) if h)
                if band_hi >= MAX_LEVEL:
                    band_hi = 0
                needs = sorted(set(r["needs"]) | ({flag.lower()} if flag else set()))
                item = root["wcid"] if root["cat"] == GIVE else 0
                for pc, px, py, pz in places:
                    first = event_starters(data, r["events"], pc)
                    if first is None:
                        continue
                    out.add((str(obj), data.name.get(obj, ""), "%08X" % pc, px, py, pz, str(item),
                             ";".join(needs), str(band_lo), str(band_hi), str(portal),
                             "%08X" % cell, x, y, z, "%g" % round(secs, 1),
                             ";".join(first)))
    return sorted(out)


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


def luminance_of(effects):
    """Luminance a path awards (AwardLuminance: Amount64 ?? HeroXP64, EmoteManager.cs:135-140)."""
    return sum(int(num(a["amount64"])) for a in effects if a["type"] == AWARD_LUMINANCE)


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
         "start": None, "unneeds": [], "unread": [], "events": []}
    if root["cat"] in (GIVE, REFUSE):
        r["items"].append((root["wcid"], 1))
    for t, ok, a in branches:
        if isinstance(ok, str):
            r["unread"].append(ok)
            continue
        q = qname(a["msg"])
        quest = data.quest.get(q)
        counter = q in data.counter_of
        if t == INQ_EVENT:
            # A server event the client cannot see running: unread for a quest, while a way its
            # starters' uses can open first keeps the events it asks running (`event_starters`).
            # Not running is read for an event off at server start (the Watcher of the Dead's
            # OnBrowerkQuest stamp, EventBrowerkStopgap): its starter's use is rare and timed.
            if ok or a["msg"].lower() in data.event_on:
                r["unread"].append(EVENT_UNREAD)
            if not ok:
                r["event_off"] = True
            elif a["msg"]:
                r["events"].append(a["msg"].lower())
            continue
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
            if ok and stamped and q not in stamped:
                # Held inside its wait, stamping another flag: a chain's step asking it done
                # lately (EmoteManager.cs:627-631): the Fir Tree's HeartofInnocence, OswaldLair.
                r["needs"].append(q)
                continue
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
SAME_PAY = ("obj", "kind", "want", "count", "fixed", "scaled", "luminance", "timer", "repeat")


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
    # What a generator puts on the ground and gives by its Use is a source, not a giver: His
    # Master's Voice's Tomb Rubble (80066, Use: GiveItem 70024).
    made = {o for o in list(data.by_obj) if o not in data.placed
            and data.kind.get(o) in (10, 12) and data.made_on_ground(o)
            and any(s["cat"] == USE for s in data.by_obj[o])}
    for obj in sorted(set(data.placed) | made):
        if data.kind.get(obj) not in (10, 12):
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
                         luminance=luminance_of(effects),
                         stamps=stamps_of(effects), does=does,
                         gives=[(int(num(a["wcid"])), max(int(num(a["stack"], 1)), 1))
                                for a in effects if a["type"] == GIVE_ITEM and a["wcid"]])
                if root["cat"] == USE and not r["items"]:
                    for wcid, _ in r["gives"]:
                        npc_gives[wcid].add(obj)
                if obj in made:
                    continue
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
                                               stamps=[flag], fixed=0, scaled=None, luminance=0,
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
               it_name, it_type, it_value, str(q["luminance"]), ";".join(q["does"]),
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
    # A kill heard by an event's watcher: the landblocks it is heard in, and the deaths it needs.
    heard = {}
    # Every recall spell's scroll is had and read (a quest row below); every portal gem is had
    # as a way in. Both are wants whose sources are written.
    learnt = {w: spell for spell in data.recalls for w in data.scrolls_of.get(spell, ())}
    wants = sorted({r[5] for r in rows_out if r[1] in ("kill", "item")}
                   | {str(w) for w in learnt} | {str(w) for w in data.gems}
                   | {"flag:" + f for f in data.stamps}
                   | {"flag:" + f for f in data.portal_stamps}
                   | {"event:" + e for e in data.portal_events})
    # And the spell components no shop sells: what an item goal is set for (Chorizite).
    rewards = reward_items(rows_out)
    wants = sorted(set(wants) | {str(c) for c in data.components - data.sold})
    seen = set(wants)
    todo = [(w, 0) for w in wants]
    while todo:
        want, depth = todo.pop(0)
        before = len(src)
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
        elif want.startswith("event:"):
            # Started by handing an item to an NPC or by a creature's death (EmoteType 23).
            for kind, obj, item in sorted(data.starts.get(want[6:], ())):
                if kind == "give":
                    keys = "%d=%s" % (item, data.name.get(item, "").replace("=", "-"))
                    src.append((want, "give", obj, 1.0, keys))
                    if str(item) not in seen and depth < EVENT_DEPTH:
                        seen.add(str(item))
                        todo.append((str(item), depth + 1))
                elif kind == "kill" and data.spawns.get(obj):
                    src.append((want, "kill", obj, 1.0, ""))
                elif kind == "use" and (data.placed.get(obj) or data.ground_places(obj)):
                    src.append((want, "start", obj, 1.0, ""))
                    for ev in sorted(data.start_needs.get((want[6:], obj), ())):
                        if "event:" + ev not in seen and depth < EVENT_DEPTH:
                            seen.add("event:" + ev)
                            todo.append(("event:" + ev, depth + 1))
            # Or by a watcher that hears `count` deaths: of what signals it as it dies, where the
            # watcher stands (a signal is heard nearby only: Landblock.cs:985-999).
            for watcher, signal, count in sorted(data.signal_starts.get(want[6:], ())):
                near = data.inst_blocks.get(watcher, set())
                for c in sorted(data.death_signals.get(signal, ())):
                    blocks = {lb for lb, _ in data.spawns.get(c, ()) if lb in near}
                    if blocks and (want, c) not in heard:
                        src.append((want, "kill", c, 1.0, ""))
                        heard[(want, c)] = (blocks, count)
        elif want.startswith("flag:"):
            # Stamped on whoever deals a creature its last blow (EmoteType 22 in its Death set).
            for c in sorted(data.stamps.get(want[5:], ())):
                if data.spawns.get(c):
                    src.append((want, "kill", c, 1.0, ""))
            # Or on whoever goes through a portal (its Portal set): one placed, or one a
            # generator makes when something reveals it (the Colosseum's Arena 3).
            for p in sorted(data.portal_stamps.get(want[5:], ())):
                if placed_at(data, p):
                    kind = "portal" if standing(data, p) else "revealed"
                    src.append((want, kind, p, 1.0, ""))
        else:
            for c in sorted(data.counter_of[want]):
                if data.spawns.get(c):
                    src.append((want, "kill", c, 1.0, ""))
        # A creature or NPC only an event puts out needs the event started first.
        for r in src[before:]:
            if r[1] in ("drop", "kill", "npc", "give"):
                ev = data.gate_of(r[2])[1]
                if ev and "event:" + ev not in seen and depth < EVENT_DEPTH:
                    seen.add("event:" + ev)
                    todo.append(("event:" + ev, depth + 1))
    def gate(r):
        return data.gate_of(r[2])[1] if r[1] in ("drop", "kill", "npc", "give") else None

    def needs(r):
        return data.start_needs.get((r[0][6:], r[2]), ()) if r[1] == "start" else ()

    src = kept_uses(src, wants, data.sold | rewards, gate, needs)
    had = {r[0] for r in src} | {str(w) for w in data.sold}
    rows_out = rows_out + learn_rows(data, learnt, had)
    rows_out.sort(key=lambda r: (r[0], kind_order[r[1]], int(r[3]), int(r[7])))
    src_out = []
    for want, kind, c, p, keys in src:
        at, blocks, count = "", [], 1
        if kind == "combine":
            where = ""
        elif kind in ("npc", "start", "floor", "container", "use", "give", "portal", "revealed"):
            if kind == "start":
                spots = data.placed.get(c) or sorted(set(data.ground_places(c)))
            elif kind == "npc":
                spots = data.placed.get(c) or sorted(set(data.made_on_ground(c)))
            elif kind == "give" and data.placed.get(c):
                spots = data.placed.get(c, [])
            elif kind == "give":
                spots = sorted(set(data.made_on_ground(c)))
            elif kind in ("portal", "revealed"):
                spots = placed_at(data, c)
            else:
                spots = sorted(set(data.made_on_ground(c) if kind == "floor"
                                   else data.ground_places(c)))
            where = ";".join("%08X:1" % cell for cell, _, _, _ in spots)
            at = ";".join("%s %s %s" % (x, y, z) for _, x, y, z in spots)
        else:
            # A kill a watcher hears counts only where the watcher stands.
            near, count = heard.get((want, c), (None, 1))
            blocks = [b for b in sorted(data.spawns[c]) if near is None or b[0] in near]
            where = ";".join("%X:%d" % lb for lb in blocks)
            # A creature the encounter table alone puts out: where on each landblock.
            if c in data.met_at:
                kept = [xy for b, xy in zip(sorted(data.spawns[c]), data.met_at[c]) if b in blocks]
                at = ";".join("%g %g 0" % xy for xy in kept)
        # No chain known (an emote's summons, a linked placement): no figure, not a rare one.
        share = data.spawn_share(c) if kind in ("drop", "kill") else 0.0
        spawn = "%g" % round(share, 4) if share > 0 else ""
        rates = ""
        ev = gate((want, kind, c))
        if kind == "start":
            ev = ";".join(sorted(data.start_needs.get((want[6:], c), ()))) or None
        if kind in ("drop", "kill"):
            # One an event gates, as it comes once the event is started.
            spawning = data.spawning_on if ev else data.spawning
            figures = (spawning.rates(c, lb) for lb, _ in blocks)
            rates = ";".join("%s:%s%s" % (figure(n, 2), figure(h, 1),
                                          ":%g" % round(o) if round(o) else "")
                             for n, h, o in figures)
        back = respawn_of(data, c) if kind == "floor" else None
        row = (want, kind, str(c), data.name.get(c, ""), str(data.level(c)),
               "%g" % round(p, 4), where, at, keys, spawn, rates,
               ev or "", "" if back is None else "%g" % round(back))
        # The deaths a watcher must hear, only where more than one: no other row has the column.
        src_out.append(row + (str(count),) if count > 1 else row)
    return rows_out, givers_out, src_out, reasons


def placed_at(data, wcid, depth=0):
    """Where `wcid` stands: placed, or made by a generator where that stands, a Specific profile
    with no cell at the generator's spot plus its offset (Spawn_Specific, GeneratorProfile.cs:320-344;
    the cell stays the generator's)."""
    out = list(data.ground.get(wcid, ())) + list(data.portal_placed.get(wcid, ()))
    if depth < GEN_DEPTH:
        for gen, _, where, spot in data.made_by.get(wcid, ()):
            if where & (8 | 16 | 32 | 64):
                continue
            if where & 4 and spot[0]:
                out.append(spot)
                continue
            off = [num(v) for v in spot[1:]] if where & 4 else [0.0, 0.0, 0.0]
            for cell, *xyz in placed_at(data, gen, depth + 1):
                out.append((cell,) + tuple("%g" % round(num(a) + b, 2) for a, b in zip(xyz, off)))
    return sorted(set(out))


def standing(data, wcid, depth=0):
    """Whether `wcid` is in the world with nothing revealing it: placed, or made by a generator
    that fills at load (regenerating, a container, or its event On: WorldObject_Generators.cs:
    398-460, 487-491) and stands itself."""
    if data.ground.get(wcid) or data.portal_placed.get(wcid):
        return True
    for gen, _, where, _ in data.made_by.get(wcid, ()) if depth < GEN_DEPTH else ():
        kind, regen, _, _, _, event = data.spawning.genprops.get(gen, (0, 0.0, 0, 0, 0, "0"))
        fills = event != "off" and (regen > 0 or kind in GEN_CONTAINERS or event == "1")
        if not where & (8 | 16 | 32 | 64) and fills and standing(data, gen, depth + 1):
            return True
    return False


def reach_all(data, obj, s):
    """Each (set, action) set `s` of `obj` can come to, whatever its branches answer: its own and
    those of every set its Gotos and branches run, keyed on their message (EmoteManager.cs:1512-1556)."""
    seen, todo, out = {s["id"]}, [s], []
    while todo:
        cur = todo.pop()
        for a in cur["acts"]:
            out.append((cur, a))
            if (a["type"] == GOTO or a["type"] in BRANCHING) and a["msg"]:
                for nxt in data.outcomes(obj, None, a["msg"]):
                    if nxt["id"] not in seen:
                        seen.add(nxt["id"])
                        todo.append(nxt)
    return out


def arena_rows(data):
    """The Colosseum's arenas as their emote tables run them (docs in ARENAS_HEADER): each room's
    bell, whose use starts the coordinator it makes, whose waves passed signal an exit generator in
    hearing to make the room's portal; each way in, a teleport an NPC casts for a fellowship
    holding its access quest; and who stamps that quest on the fellowship for an item handed."""
    out = []
    xyz = lambda p: " ".join("%g" % num(v) for v in p[1:])
    away = lambda a, b: sum((num(x) - num(y)) ** 2 for x, y in zip(a[1:], b[1:])) ** 0.5
    sends = lambda o, sig: any(a["type"] == LOCAL_SIGNAL and a["msg"].lower() == sig
                               for s in data.by_obj.get(o, ()) for a in s["acts"])
    for portal in sorted(p for p in data.rel_dest if data.kind.get(p) == 7):
        flags = ";".join(sorted(f for f, ps in data.portal_stamps.items() if portal in ps))
        for gen, _, _, _ in data.made_by.get(portal, ()):
            reveals = {s["quest"] for s in data.by_obj.get(gen, ())
                       if s["cat"] == RECEIVE_SIGNAL and any(a["type"] == GENERATE for a in s["acts"])}
            coords = [o for o in sorted(data.by_obj) for r in reveals if sends(o, r)]
            for coord in coords:
                starts = {s["quest"] for s in data.by_obj[coord] if s["cat"] == RECEIVE_SIGNAL}
                for bell, _, where, spot in data.made_by.get(coord, ()):
                    rings = any(a["type"] == LOCAL_SIGNAL and a["msg"].lower() in starts
                                for root in data.by_obj.get(bell, ()) if root["cat"] == USE
                                for _, a in reach_all(data, bell, root))
                    if not rings:
                        continue
                    off = [num(v) for v in spot[1:]] if where & 4 and not spot[0] else [0.0] * 3
                    for pg in placed_at(data, gen):
                        for pb in placed_at(data, bell):
                            pc = (pb[0],) + tuple(num(a) + b for a, b in zip(pb[1:], off))
                            if pb[0] >> 16 != pg[0] >> 16 or away(pc, pg) > data.hearing.get(gen, 0):
                                continue
                            to = (pg[0],) + tuple(num(a) + b for a, b in
                                                  zip(pg[1:], data.rel_dest[portal]))
                            out.append(("room", "%X" % (pg[0] >> 16), str(bell),
                                        data.name.get(bell, ""), "%08X" % pb[0], xyz(pb),
                                        str(portal), "%08X" % pg[0], xyz(pg), flags, xyz(to)))
    arenas = {r[1] for r in out}
    access = {}
    for obj in sorted(data.placed):
        for root in (s for s in data.by_obj[obj] if s["cat"] == USE):
            for s, a in reach_all(data, obj, root):
                land = data.spell_pos.get(a["spell"]) if a["type"] == CAST_INSTANT else None
                if land is None or "%X" % (land[0] >> 16) not in arenas or not s["quest"]:
                    continue
                access["%X" % (land[0] >> 16)] = qname(s["quest"])
                # The lowest level its use lets in (InqIntStat Level: the Advanced Arena's 80).
                lowest = max([int(num(b["min"])) for _, b in reach_all(data, obj, root)
                              if b["type"] == INQ_INT and num(b["stat"]) == LEVEL_STAT] or [0])
                for cell, *at in data.placed[obj]:
                    out.append(("way", "%X" % (land[0] >> 16), str(obj), data.name.get(obj, ""),
                                "%08X" % cell, xyz((cell,) + tuple(at)), str(a["spell"]),
                                "%08X" % land[0], xyz(land), qname(s["quest"]), str(lowest)))
    for obj in sorted(data.placed):
        for root in (s for s in data.by_obj[obj] if s["cat"] == GIVE and s["wcid"]):
            stamped = {qname(a["msg"]) for _, a in reach_all(data, obj, root)
                       if a["type"] == STAMP_FELLOW and a["msg"]}
            for lb, quest in sorted(access.items()):
                if quest not in stamped:
                    continue
                for cell, *at in data.placed[obj]:
                    out.append(("ticket", lb, str(obj), data.name.get(obj, ""), "%08X" % cell,
                                xyz((cell,) + tuple(at)), str(root["wcid"]), "", "", quest,
                                str(data.quest.get(quest, (0, 0, ""))[0])))
    return sorted(set(out))


def respawn_of(data, wcid):
    """Seconds a generator takes to put `wcid` back on the ground once it is taken: its profile's
    delay (a linked placement's is profile[0]'s: WorldObject_Generators.cs:589-625) and a
    regeneration interval for its tick to come round (GeneratorProfile.cs:526-564); the soonest
    of its generators, None when none is known."""
    sp = data.spawning
    out = []
    gens = [(g, None) for g in sp.link_parent.get(wcid, ())]
    gens += [(g, wcid) for g, _, where, _ in data.made_by.get(wcid, ()) if not where & 120]
    for g, made in gens:
        profs = sp.profiles.get(g, [])
        if not profs or g not in sp.genprops:
            continue
        mine = [p for p in profs if made is None or p["wcid"] == made] or profs
        delay = mine[0]["delay"] if mine[0]["delay"] is not None else profs[0]["delay"]
        out.append((delay or 0.0) + sp.genprops[g][1])
    return min(out) if out else None


def figure(x, places):
    """`x` to `places` decimals, but under 1 to two significant figures: a rare spawn's 0.012 an
    hour is not 0."""
    return "%.2g" % x if 0 < x < 1 else "%g" % round(x, places)


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
                    str(ints.get(19, 0)), "0", "spell", ""))
    return out


def reward_items(rows_out):
    """Every item a quest row gives (its gives column, `wcid:count;...`, `|` between a random
    reply's alternatives)."""
    out = set()
    for r in rows_out:
        for alt in r[COL_GIVES].split("|"):
            for g in alt.split(";"):
                if g and g.split(":")[0].isdigit():
                    out.add(int(g.split(":")[0]))
    return out


def kept_uses(src, wants, sold, gate=lambda r: None, needs=lambda r: ()):
    """`src` less each use whose items used on it have no source, and less the rows of an item
    then wanted by nothing kept; an event a kept row's creature needs (`gate`), or a kept start's
    use asks running (`needs`), is wanted."""
    while True:
        had = {r[0] for r in src} | {str(w) for w in sold}
        items = lambda r: [k.split("=", 1)[0] for k in r[4].split(";")]
        keep = [r for r in src if r[1] not in ("use", "combine")
                or (r[1] == "use" and any(k in had for k in items(r)))
                or (r[1] == "combine" and all(k in had for k in items(r)))]
        asked = set(wants) | {k.split("=", 1)[0] for r in keep for k in r[4].split(";") if k}
        asked |= {"event:" + g for g in map(gate, keep) if g}
        asked |= {"event:" + n for r in keep for n in needs(r)}
        keep = [r for r in keep if r[0] in asked]
        if len(keep) == len(src):
            return keep
        src = keep


# Landblocks kept per creature the encounter table alone spawns: where it is likeliest.
ENCOUNTER_BLOCKS = 64
# Seconds to leave a landblock and come back to a fresh load (ac_world::quest_chain::ROTATE_SECS).
ROTATE_SECS = 420.0

# Columns of a quests.csv row a random reply's alternatives may differ in and still be one row.
COL_GIVES, COL_DOES, COL_UNREAD = 14, 23, 24


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
    events_out = event_portal_rows(data, dump)
    data.portal_events = {r[-1] for r in events_out}
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
    # The components an item goal can be set for: those the walk found a way to.
    found = {int(r[0]) for r in src_out if r[0].isdigit()}
    goals = sorted(c for c in data.components - data.sold if c in found)
    with open(os.path.join(out_dir, "goal_items.csv"), "w", encoding="utf-8", newline="") as f:
        f.write(GOAL_ITEMS_HEADER)
        write_rows(f, [(str(c), data.name.get(c, "")) for c in goals])
    with open(os.path.join(out_dir, "reward_items.csv"), "w", encoding="utf-8", newline="") as f:
        f.write(REWARD_ITEMS_HEADER)
        write_rows(f, [(str(c), data.name[c]) for c in sorted(reward_items(rows_out))
                       if data.name.get(c)])
    ways = npc_ways(data, asked - stamped)
    summoned = {(c,) + w for c, w in data.summons.items()}
    summoned |= {(int(r[0]),) + data.link_portal[(int(r[0]), d)]
                 for r in ways if r[10] != "0" for d in (31, 48)
                 if data.link_portal.get((int(r[0]), d), (0,))[0] == int(r[10])}
    with open(os.path.join(out_dir, "summoned_portals.csv"), "w", encoding="utf-8",
              newline="") as f:
        f.write(SUMMONS_HEADER)
        write_rows(f, [(str(c), data.name.get(c, ""), str(p), data.name.get(p, ""),
                        "%08X" % cell, x, y, z, str(lo), str(hi), quest)
                       for c, p, cell, x, y, z, lo, hi, quest in sorted(summoned)])
    with open(os.path.join(out_dir, "spawned_portals.csv"), "w", encoding="utf-8",
              newline="") as f:
        f.write(SPAWNED_HEADER)
        write_rows(f, spawned_portal_rows(data, dump))
    with open(os.path.join(out_dir, "event_portals.csv"), "w", encoding="utf-8", newline="") as f:
        f.write(EVENT_PORTALS_HEADER)
        write_rows(f, events_out)
    with open(os.path.join(out_dir, "npc_ways.csv"), "w", encoding="utf-8", newline="") as f:
        f.write(NPC_WAYS_HEADER)
        write_rows(f, ways)
    with open(os.path.join(out_dir, "arenas.csv"), "w", encoding="utf-8", newline="") as f:
        f.write(ARENAS_HEADER)
        write_rows(f, arena_rows(data))
    with open(os.path.join(out_dir, "pickups.csv"), "w", encoding="utf-8", newline="") as f:
        f.write(PICKUPS_HEADER)
        write_rows(f, pickup_rows(data))
    by_kind = defaultdict(set)
    for r in rows_out:
        by_kind[r[1]].add(r[0])
    print("quests: %d rows, %d giver placements; %s; out: %s" % (
        len(rows_out), len(givers_out), ", ".join("%s %d" % (k, len(v)) for k, v in sorted(by_kind.items())),
        dict(sorted(reasons.items()))), file=sys.stderr)


def event_portal_rows(data, dump):
    """Each portal a generator puts out only while an Off event runs, where something's use starts
    it: where it comes, where it leads, its level band and flag, and the event (Qin Xikit's Hidden
    Crown, the only way into 0x00F2, while the Font Guardian's EventFontBrowerkStopgap runs)."""
    out = []
    for w, c, x, y, z, lo, hi, q in rows(dump, "genportals"):
        ev = data.gate_of(int(w))[1]
        if not ev or not any(k == "use" for k, _, _ in data.starts.get(ev, ())):
            continue
        for cell, px, py, pz in sorted(set(data.made_on_ground(int(w)))):
            out.append((w, data.name.get(int(w), ""), "%08X" % cell, px, py, pz,
                        "%08X" % int(c, 16), x, y, z, lo, hi, q, ev))
    return out


def spawned_portal_rows(data, dump):
    """Each portal a generator in the world puts out, by landblock it comes on: where (the
    generator's spot, or an encounter's busiest corner), how fast (as a creature's `rates`, the
    portal taken as it comes) and where it leads. The Small Hive Portals come by the Marae Lassel
    plateau's camps alone, and each is the only way into its hive."""
    dest = {int(w): ("%08X" % int(c, 16), x, y, z, lo, hi, q)
            for w, c, x, y, z, lo, hi, q in rows(dump, "genportals")}
    sp = Spawning(dump, counted=set(dest))
    met = defaultdict(lambda: defaultdict(list))
    for w, lb, x, y, n in rows(dump, "portalmet"):
        met[int(w)][int(lb, 16)].append((int(n), int(x), int(y)))
    out = []
    for w in sorted(dest):
        found = []
        spots = {}
        for cell, x, y, z in data.made_on_ground(w):
            if cell:
                spots.setdefault(cell >> 16, (cell, num(x), num(y), num(z)))
        for lb, cells in met[w].items():
            x, y = corner(cells)
            spots.setdefault(lb, ((lb << 16) | (int(x // 24) * 8 + int(y // 24) + 1), x, y, 0.0))
        for lb in sorted(spots):
            load, rate, others = sp.rates(w, lb)
            if load > 0 or rate > 0:
                found.append((max(rate, load * 3600 / ROTATE_SECS), lb, load, rate, others))
        # The ENCOUNTER_BLOCKS soonest, as for a creature: a Destroyed Portal comes on 11,439.
        found = sorted(found, key=lambda f: (-f[0], f[1]))[:ENCOUNTER_BLOCKS]
        for _, lb, load, rate, others in sorted(found, key=lambda f: f[1]):
            cell, x, y, z = spots[lb]
            out.append((str(w), data.name.get(w, ""), "%08X" % cell, "%g" % x, "%g" % y, "%g" % z,
                        figure(load, 2), figure(rate, 1), "%g" % round(others)) + dest[w])
    return out


def pickup_rows(data):
    """Each item with a pickup limit (Quest 33) whose quest is known: wcid, stamp, wait, solves."""
    out = []
    for wcid, stamp in sorted(data.pickup.items()):
        if stamp in data.quest:
            delta, solves, _ = data.quest[stamp]
            out.append([str(wcid), stamp, str(delta), str(solves)])
    return out


ARENAS_HEADER = """\
# The Colosseum's arenas as their emote tables run them (EmoteManager.cs; quests_walk.arena_rows).
# kind,landblock,wcid,name,cell,at,other_wcid,other_cell,other_at,quest,to
# room: an arena room's bell (wcid, at): used, the coordinator it makes starts the room's waves
# (its ReceiveLocalSignal set); once they pass it signals an exit generator in hearing, which makes
# the room's portal (other: wcid, where); quest: the flags going through it stamps (Portal set);
# to: where the portal sends (its RelativeDestination, Portal.cs:57-64). way: the NPC whose use
# casts the teleport into the arena (other: spell, where it lands) for a fellowship holding
# quest (InqFellowQuest, EmoteManager.cs:500-517); to: the lowest level it lets in. ticket: the NPC who, handed the item (other),
# stamps that quest on the fellowship (StampFellowQuest); to: the quest's wait in seconds.
# Cells hex, x y z local to the cell's landblock. Regenerate with reference/scripts/data/quests.sh.
"""


GOAL_ITEMS_HEADER = """\
# Spell components no shop sells that the data has a way to, besides the quests' own items: what an
# item goal can be set for (ac_world::quest_chain::items_named). Their ways are in
# quest_sources.csv. wcid,name. Regenerate with reference/scripts/data/quests.sh.
"""

REWARD_ITEMS_HEADER = """\
# Every item a quest in quests.csv gives, by name: the quest book's rewards
# (ac_world::quest_chain::reward_name). wcid,name. Regenerate with reference/scripts/data/quests.sh.
"""

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
# id,kind,title,giver_wcid,hand,want,count,min_level,max_level,xp,xp_level,xp_min,xp_max,share,gives,stamps,repeat,timer,needs,item,item_type,item_value,luminance,does,unread
# Rows sharing an id are alternatives. kind: kill, item, talk, or start (the talk that takes a
# quest up: it stamps what the quest's rows need). hand: give (hand the item over), talk, or
# read (a recall spell's scroll read: id learn:SPELL, giver_wcid 0, no giver).
# want: the item wcid, or the kill counter. count: items one hand-in takes, or kills. Levels 0
# are open. xp is fixed; xp_level is a share of the XP to the next level, clamped to
# xp_min/xp_max when not 0. share: all, none or allegiance. gives: wcid:count;..., and for a
# random reply every alternative |-separated (an empty one gives nothing). stamps and needs:
# quest flags. repeat: seconds before another solve, 0 any time, -1 once; timer: the quest its
# wait is kept under. item, item_type (hex), item_value: the wanted item. luminance: what the
# path awards. does: what the path does (xp, item, flag, spell, skill, luminance, title,
# teleport, takes, stat, contract, enlightenment, type N). unread: conditions the client cannot
# check, |-separated ("a random reply" among them). Regenerate with
# reference/scripts/data/quests.sh (reads the ACE world database, the community's reconstruction
# of retail's server data).
"""

GIVERS_HEADER = """\
# Where each quest giver in quests.csv stands: one row per placement.
# giver_wcid,giver,cell,x,y,z
# cell is hex, x/y/z local to its landblock. Regenerate with reference/scripts/data/quests.sh.
"""

SOURCES_HEADER = """\
# Where what a quest wants comes from, and what each source itself needs.
# want,kind,wcid,name,level,chance,landblocks,at,keys,spawn,rates,event,respawn[,count]
# want: an item wcid or a kill counter, as quests.csv names it, a key a container or use
# below needs, or event:NAME, a server event a source below needs, or flag:NAME, a flag stamped
# on a creature's killer or a portal's user (both lower case). kind: drop (a corpse drop, chance
# per kill), kill (a creature counting for the counter, or whose death starts the event, itself
# or heard by a watcher there, or stamps the flag), give (an NPC handed its keys starts the
# event), portal (going through it stamps the flag), revealed (as portal, made by a generator
# once something reveals it: the Colosseum's, arenas.csv), npc (a placed NPC, or an object a generator
# puts out, whose talk gives the item), start (an object whose use starts the event, once the
# events in its event column run), floor (made on the ground by a
# generator; wcid is the item's own), container (a chest that holds it, chance per fill), use
# (a placed object a recipe makes it on, from a carried item used on it) or combine (a carried
# item a recipe makes it on; nowhere to go). landblocks: hex landblock:spawn entries;... (for
# an npc, start, give, portal, revealed, floor, container or use, hex cell:1 per placement, with at the matching local
# "x y z";...; for a drop or kill the encounter table alone spawns, each landblock's busiest
# encounter corner, "x y 0";...). keys: for a locked container, the keys whose KeyCode is its LockCode; for a give, the item handed; for a
# use, the items used on it; for a combine, the item used then the one used on;
# wcid=name;..., each a want of its own rows. A shop's stock is vendors.csv's, joined at load (ac_world::quests::sources).
# spawn: for a drop or kill, the best share of a generator roll that puts the creature out (1 sure;
# blank when no generator chain is known). rates: for a drop or kill, load:per_hour[:others];...
# one per landblocks entry: how many stand there once the landblock loads, how many its
# generators make an hour while everything they make is killed, and when killing it alone leaves
# its slots held, the others an hour that clearing kills; from a seeded simulation of ACE's
# generators, with its event on when it has one. event: the Off event (EventManager.cs) whose
# generator alone puts the creature or NPC out: it comes only once that event is started; for a
# start, the events its use asks running first, ;-separated.
# respawn: for a floor item, seconds its generator takes to put it back once taken (profile delay
# and a regeneration interval); blank when unknown. count: for a kill a watcher hears, the deaths
# it must hear before it starts the event (only written when more than 1).
# Regenerate with reference/scripts/data/quests.sh.
"""

EVENT_PORTALS_HEADER = """\
# Portals a generator puts out only while an Off server event runs (EventManager.cs), started by
# a use: event:NAME's start rows in quest_sources.csv (Qin Xikit's Hidden Crown, the way into 0x00F2).
# wcid,name,cell,x,y,z,to_cell,to_x,to_y,to_z,min_level,max_level,quest,event
# cell, x/y/z: where it comes (its generator's spot), local to the landblock; to: where it leads;
# the portal's own level band and flag (0, blank: none); event: lower case.
# Regenerate with reference/scripts/data/quests.sh.
"""

SPAWNED_HEADER = """\
# Portals a generator in the world puts out, one row a landblock each comes on: a camp's rare pick
# (the Small Hive Portals of the Marae Lassel plateau, the only ways into their hives).
# wcid,name,cell,x,y,z,load,per_hour,others,to_cell,to_x,to_y,to_z,min_level,max_level,quest
# cell, x/y/z: where it comes (the generator's spot, or the encounter table's busiest corner
# there), local to the landblock; load, per_hour, others: as quest_sources.csv's rates, the portal
# taken as it comes; to: where it leads; the portal's own level band and flag (0, blank: none).
# Regenerate with reference/scripts/data/quests.sh.
"""

SUMMONS_HEADER = """\
# Portals a creature's death, or an NPC handed an item or used, opens: its emote casts a portal
# summon (WorldObject_Magic.cs:1185-1266) of its linked portal, 3 m in front of it, for the
# spell's lifetime.
# by_wcid,by,portal_wcid,portal,to_cell,to_x,to_y,to_z,min_level,max_level,quest
# (x/y/z local to the cell's landblock; the portal's own level band and flag, 0 and blank: none)
# Regenerate with reference/scripts/data/quests.sh.
"""

NPC_WAYS_HEADER = """\
# Ways a placed NPC opens for whoever hands it an item or uses it: a portal it summons in front
# of it (Faladha the Emissary's to Aerlinthe, for a 10,000 note) or a teleport (EmoteType 99).
# npc_wcid,npc,cell,x,y,z,item,needs,min_level,max_level,portal_wcid,to_cell,to_x,to_y,to_z,secs,first
# item: what is handed, 0 for a use. needs: flags held first, ;-separated. Level band 0: open.
# portal_wcid 0: a teleport (EmoteType 99, or a sending spell cast). x/y/z local to the cell's
# landblock. secs: the NPC's emote delays before the portal is summoned or the teleport made.
# first: placed objects in the NPC's landblock used first, ;-separated wcid:cell:x:y:z, each
# starting a server event the NPC asks running (Browerk's Portal Device, the Eyes of the Watcher).
# Regenerate with reference/scripts/data/quests.sh.
"""

if __name__ == "__main__":
    main()
