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

    def level(self, wcid):
        return self.ints[wcid].get(25, 0)


def walk(data, root):
    """Every path from `root`: (branches, effects), or (None, why) for one the client cannot know."""
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
                    out.append((None, "random"))
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
                    out.append((None, "other branches"))
                    return
                for cat, ok in cats:
                    nxt = data.outcome(obj, cat, a["msg"])
                    if nxt == "random":
                        out.append((None, "random"))
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


def read_path(data, root, branches, effects):
    """A path read into a quest's terms (design §1.4's table), or a string saying why it is out."""
    stamped = stamps_of(effects)
    r = {"band": [0, 0], "timer": "", "repeat": 0, "kill": None, "items": [], "needs": [],
         "start": None}
    if root["cat"] in (GIVE, REFUSE):
        r["items"].append((root["wcid"], 1))
    for t, ok, a in branches:
        q = qname(a["msg"])
        quest = data.quest.get(q)
        counter = q in data.counter_of
        if t == INQ_INT:
            if num(a["stat"]) != LEVEL_STAT or not ok:
                return "other branches"
            lo, hi = int(num(a["min"])), int(num(a["max"], MAX_LEVEL))
            r["band"] = [max(r["band"][0], lo), min(r["band"][1] or MAX_LEVEL, hi)]
        elif t == INQ_OWNS:
            if not ok:
                return "other branches"
            r["items"].append((int(num(a["wcid"])), max(int(num(a["stack"], 1)), 1)))
        elif t == INQ_QUEST and counter:
            if quest is None or quest[1] < 1:
                return "counters not fed by kills"
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
                return "solve-count ranges"
        elif t == INQ_QUEST and quest is not None and quest[0] > 0:
            if ok:
                return "another quest's wait"
            if r["timer"] and r["timer"] != q:
                return "another quest's wait"
            r["timer"], r["repeat"] = q, quest[0]
        elif t in (INQ_QUEST, INQ_SOLVES):
            # A flag: held or not (InqQuest: QuestManager.cs:266-280, 334-363).
            lo = int(num(a["min"], 1)) if t == INQ_SOLVES else 1
            if t == INQ_SOLVES and (lo < 1 or not open_max(a["max"])):
                return "solve-count ranges"
            if t == INQ_QUEST and quest is not None and quest[1] == -1:
                if ok:
                    return "other branches"  # never held and unsolvable: no path
                continue  # always solvable: the branch asks nothing
            if t == INQ_QUEST and quest is not None and quest[1] > 1:
                return "solve-count ranges"
            if not ok and q in stamped:
                if not r["timer"]:
                    r["timer"], r["repeat"] = q, -1
                elif r["repeat"] == 0:
                    r["repeat"] = -1
            elif ok and q not in stamped:
                r["needs"].append(q)
            else:
                return "needs a quest or flag not in scope"
        else:
            return "other branches"
    if r["band"][1] >= MAX_LEVEL:
        r["band"][1] = 0
    return r


def write_rows(f, out):
    for row in out:
        assert not any("," in v or "\n" in v for v in row), row
        f.write(",".join(row) + "\n")


def main():
    dump, out_dir = sys.argv[1], sys.argv[2]
    data = Data(dump)
    reasons = defaultdict(int)
    xp_paths, starts, npc_gives = [], [], defaultdict(set)
    for obj in sorted(data.placed):
        if data.wtype[obj] not in (10, 12):
            continue
        sets = data.by_obj[obj]
        roots = []
        uses = [s for s in sets if s["cat"] == USE]
        if uses:
            roots.append("random" if any(s["p"] < 1 for s in uses) else uses[0])
        for wcid in sorted({s["wcid"] for s in sets if s["cat"] in (GIVE, REFUSE)}):
            for cat in (REFUSE, GIVE):
                if cat == GIVE and obj not in data.allow_give:
                    break  # Player_Inventory.cs:3439
                keyed = [s for s in sets if s["cat"] == cat and s["wcid"] == wcid]
                if keyed:
                    roots.append("random" if any(s["p"] < 1 for s in keyed) else keyed[0])
                    break
        for root in roots:
            if root == "random":
                continue
            for branches, effects in walk(data, root):
                if branches is None:
                    continue
                fixed, scaled, share = xp_of(effects)
                r = read_path(data, root, branches, effects)
                if isinstance(r, str):
                    if fixed or scaled:
                        reasons[r] += 1
                    continue
                r.update(obj=obj, root=root, fixed=fixed, scaled=scaled, share=share,
                         stamps=stamps_of(effects),
                         gives=[(int(num(a["wcid"])), max(int(num(a["stack"], 1)), 1))
                                for a in effects if a["type"] == GIVE_ITEM and a["wcid"]])
                if root["cat"] == USE and not r["items"]:
                    for wcid, _ in r["gives"]:
                        npc_gives[wcid].add(obj)
                if fixed or scaled:
                    xp_paths.append(r)
                elif root["cat"] == USE and r["stamps"] and not r["items"] and not r["kill"]:
                    starts.append(r)

    quests = []
    for r in xp_paths:
        items = sorted(set(r["items"]))
        if len({w for w, _ in items}) > 1:
            reasons["two items"] += 1
            continue
        if r["kill"] and items:
            reasons["two items"] += 1
            continue
        if r["start"]:
            reasons["other branches"] += 1
            continue
        item = None
        if items:
            wcid = items[0][0]
            count = max(n for _, n in items)
            item = (wcid, count)
        if r["kill"]:
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
        quests.append(dict(r, id=qid, kind=kind, want=want, count=count, needs=needs,
                           item=item))

    def sources_of(q):
        if q["kind"] == "kill":
            return [w for w in data.counter_of[q["want"]] if data.spawns.get(w)]
        if q["kind"] == "item":
            w = q["item"][0]
            return [c for c, _ in data.drops.get(w, []) if data.spawns.get(c)] + \
                sorted(npc_gives.get(w, ()))
        return [True]

    kept = []
    for q in quests:
        if not sources_of(q):
            reasons["nothing drops or gives it" if q["kind"] == "item" else "nothing spawns"] += 1
            continue
        kept.append(q)

    # Start talks: a no-XP talk stamping what a kept quest needs (a kill task's counter).
    while True:
        stamped = {s for q in kept for s in q["stamps"]}
        needed = {n for q in kept for n in q["needs"]}
        start_rows = []
        for s in starts:
            for flag in s["stamps"]:
                if flag in needed and flag not in stamped:
                    for q in kept:
                        if flag in q["needs"]:
                            start_rows.append(dict(s, id=q["id"], kind="start", want="",
                                                   count=0, needs=[n for n in s["needs"]
                                                                   if n != flag],
                                                   item=None, timer="", repeat=0,
                                                   stamps=[flag], fixed=0, scaled=None,
                                                   share="none", gives=[]))
                            break
        both = kept + start_rows
        have = {f for q in both for f in q["stamps"]}
        left = [q for q in both if all(n in have for n in q["needs"])]
        if len(left) == len(both):
            break
        reasons["needs a quest or flag not in scope"] += len(kept) - len(
            [q for q in left if q["kind"] != "start"])
        kept = [q for q in left if q["kind"] != "start"]
        starts = [s for s in starts if all(n in have for n in s["needs"])]

    rows_out = []
    seen = set()
    for q in kept + start_rows:
        title = data.quest.get(q["id"], (0, 0, ""))[2]
        if q["id"].startswith("give:"):
            title = data.name.get(q["item"][0], q["id"])
        title = (title or q["id"]).replace(",", ";")
        pct, lo, hi = q["scaled"] or (0, 0, 0)
        hand = "give" if q["root"]["cat"] in (GIVE, REFUSE) else "talk"
        item = q["item"]
        it_name = data.name.get(item[0], "") if item else ""
        it_type = "%X" % (data.ints[item[0]].get(1, 0) & 0xFFFFFFFF) if item else ""
        it_value = str(data.ints[item[0]].get(19, 0)) if item else ""
        for cell, x, y, z in data.placed[q["obj"]]:
            row = (q["id"], q["kind"], title, str(q["obj"]), data.name.get(q["obj"], ""),
                   "%08X" % cell, x, y, z, hand, q["want"], str(q["count"]),
                   str(q["band"][0]), str(q["band"][1]), str(q["fixed"]), "%g" % pct, str(lo),
                   str(hi), q["share"] or "none",
                   ";".join("%d:%d" % g for g in q["gives"]), ";".join(q["stamps"]),
                   str(q["repeat"]), q["timer"], ";".join(q["needs"]), it_name, it_type,
                   it_value)
            if row not in seen:
                seen.add(row)
                rows_out.append(row)
    kind_order = {"start": 0, "talk": 1, "kill": 2, "item": 3}
    rows_out.sort(key=lambda r: (r[0], kind_order[r[1]], int(r[3]), r[5], r[12]))

    src = []
    for want in sorted({r[10] for r in rows_out if r[1] in ("kill", "item")}):
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

    with open(os.path.join(out_dir, "quests.csv"), "w", encoding="utf-8", newline="") as f:
        f.write(QUESTS_HEADER)
        write_rows(f, rows_out)
    with open(os.path.join(out_dir, "quest_sources.csv"), "w", encoding="utf-8",
              newline="") as f:
        f.write(SOURCES_HEADER)
        write_rows(f, src_out)
    by_kind = defaultdict(set)
    for r in rows_out:
        by_kind[r[1]].add(r[0])
    print("quests: %d rows; %s; out: %s" % (
        len(rows_out), ", ".join("%s %d" % (k, len(v)) for k, v in sorted(by_kind.items())),
        dict(sorted(reasons.items()))), file=sys.stderr)


QUESTS_HEADER = """\
# Quests an NPC rewards with XP: one row per way to complete one (giver placement x level band).
# id,kind,title,giver_wcid,giver,cell,x,y,z,hand,want,count,min_level,max_level,xp,xp_level,xp_min,xp_max,share,gives,stamps,repeat,timer,needs,item,item_type,item_value
# Rows sharing an id are alternatives. kind: kill, item, talk, or start (the talk that takes a
# quest up: it stamps what the quest's rows need). cell is hex, x/y/z local to its landblock.
# hand: give (hand the item over) or talk. want: the item wcid, or the kill counter. count: items
# one hand-in takes, or kills. Levels 0 are open. xp is fixed; xp_level is a share of the XP to
# the next level, clamped to xp_min/xp_max when not 0. share: all, none or allegiance. gives:
# wcid:count;... stamps and needs: quest flags. repeat: seconds before another solve, 0 any time,
# -1 once; timer: the quest its wait is kept under. item, item_type (hex), item_value: the wanted
# item. Regenerate with reference/scripts/data/quests.sh (reads the ACE world database, the
# community's reconstruction of retail's server data).
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
