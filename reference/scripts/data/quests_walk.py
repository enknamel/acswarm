#!/usr/bin/env python3
"""Walk ACE's emote sets into quests.csv and quest_sources.csv (stdlib only).

Usage: quests_walk.py DUMP_DIR OUT_DIR, DUMP_DIR holding the tables quests.sh reads out.
docs/quests-design.md §1.4 says what a path is and which branches the client can know.
ACE citations are under reference/ext/ACE/Source/ACE.Server/.
"""
import os
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


def rows(dump, name):
    with open(os.path.join(dump, name + ".tsv"), encoding="utf-8", errors="replace") as f:
        for line in f:
            yield line.rstrip("\n").split("\t")


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
                              "max64": f[10], "stat": f[11], "wcid": f[12], "stack": f[13]})
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
# What a path does, for the `does` column: effect types by word.
DOES = (("xp", {2, 49, 62}), ("item", {3, 56}), ("flag", {20, 22, 31, 32, 33, 60, 61, 70,
        106, 107, 108, 109}), ("spell", {14, 19, 27, 73}), ("skill", {28, 29, 47, 50, 110, 111}),
        ("luminance", {112, 113}), ("title", {34}), ("teleport", {99}), ("takes", {74}),
        ("stat", {53, 54, 55, 69, 115, 118, 57, 63, 90, 48}), ("contract", {119, 120}),
        ("enlightenment", {9001}))


def does_of(root, effects):
    types = {a["type"] for a in effects}
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
                does = does_of(root, effects)
                if not does:
                    continue  # talk only: no quest
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
    kind_order = {"start": 0, "talk": 1, "kill": 2, "item": 3}
    rows_out.sort(key=lambda r: (r[0], kind_order[r[1]], int(r[3]), int(r[7])))
    givers_out = sorted({(int(r[3]), cell, x, y, z) for r in rows_out
                         for cell, x, y, z in data.placed[int(r[3])]})
    givers_out = [(str(w), data.name.get(w, ""), "%08X" % cell, x, y, z)
                  for w, cell, x, y, z in givers_out]

    src = []
    for want in sorted({r[5] for r in rows_out if r[1] in ("kill", "item")}):
        if want.isdigit():
            w = int(want)
            chance = defaultdict(float)
            for c, p in data.drops.get(w, []):
                chance[c] = min(chance[c] + p, 1.0)
            for c in sorted(chance):
                if data.spawns.get(c):
                    src.append((want, "drop", c, chance[c]))
            for n in sorted(npc_gives.get(w, ())):
                src.append((want, "npc", n, 1.0))
        else:
            for c in sorted(data.counter_of[want]):
                if data.spawns.get(c):
                    src.append((want, "kill", c, 1.0))
    src_out = []
    for want, kind, c, p in src:
        if kind == "npc":
            where = ";".join("%08X:1" % cell for cell, _, _, _ in data.placed[c])
        else:
            where = ";".join("%X:%d" % lb for lb in sorted(data.spawns[c]))
        src_out.append((want, kind, str(c), data.name.get(c, ""), str(data.level(c)),
                        "%g" % round(p, 4), where))
    return rows_out, givers_out, src_out, reasons


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


def main():
    dump, out_dir = sys.argv[1], sys.argv[2]
    data = Data(dump)
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
    by_kind = defaultdict(set)
    for r in rows_out:
        by_kind[r[1]].add(r[0])
    print("quests: %d rows, %d giver placements; %s; out: %s" % (
        len(rows_out), len(givers_out), ", ".join("%s %d" % (k, len(v)) for k, v in sorted(by_kind.items())),
        dict(sorted(reasons.items()))), file=sys.stderr)


QUESTS_HEADER = """\
# Every quest a placed NPC holds: each Use, Give or Refuse path that does anything for the
# character, XP or not, one row per way to complete one (giver and level band); the giver's
# placements are in quest_givers.csv.
# id,kind,title,giver_wcid,hand,want,count,min_level,max_level,xp,xp_level,xp_min,xp_max,share,gives,stamps,repeat,timer,needs,item,item_type,item_value,does,unread
# Rows sharing an id are alternatives. kind: kill, item, talk, or start (the talk that takes a
# quest up: it stamps what the quest's rows need). hand: give (hand the item over) or talk.
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
# Where what a quest wants comes from.
# want,kind,wcid,name,level,chance,landblocks
# want: an item wcid or a kill counter, as quests.csv names it. kind: drop (a corpse drop,
# chance per kill), kill (a creature counting for the counter) or npc (a placed NPC whose talk
# gives the item). landblocks: hex landblock:spawn entries;... (for an npc, its hex cell:1).
# Regenerate with reference/scripts/data/quests.sh.
"""

if __name__ == "__main__":
    main()
