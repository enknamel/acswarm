#!/usr/bin/env python3
"""Read acswarm telemetry (~/.cache/acswarm/telemetry/*.jsonl, written by ac_plugin::telemetry).

  tools/telemetry.py [FILE ...]          what each character did: time by activity, kills, deaths,
                                         xp, town runs, stalls, refusals, marks (default: newest file)
  tools/telemetry.py --compare A B       two runs side by side, per hour, to measure a change
  tools/telemetry.py --marks [FILE]      each F9 mark with the status lines around it
  tools/telemetry.py --setup [FILE]      each character's autoplay settings, as last written

A stall is a stretch of samples, SAMPLE_GAP apart, where the character meant to move (walking,
going, following, a town run) and got less than a metre from where it was, for STALL_AFTER or more.
Kills and deaths are the server's own words, matched by the templates tools/fleet-score.py reads
from the ACE checkout (kills show as "?" without it).
"""
import importlib.util
import json
import re
import sys
from collections import Counter, defaultdict
from pathlib import Path

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
DIR = Path.home() / ".cache" / "acswarm" / "telemetry"
SAMPLE_GAP = 2.0  # seconds between samples (SAMPLE_EVERY in crates/ac-plugin/src/telemetry.rs)
STALL_AFTER = 10.0  # seconds without a metre of progress while meaning to move
MOVING = re.compile(r"going to|walking|on the way|heading|following|back to|travel|approach", re.I)

try:
    spec = importlib.util.spec_from_file_location("fleet_score", HERE / "fleet-score.py")
    fleet = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(fleet)
    KILLER, VICTIM = fleet.KILLER, fleet.VICTIM
except SystemExit:
    KILLER, VICTIM = None, None


def records(path):
    for line in open(path, encoding="utf-8", errors="replace"):
        try:
            yield json.loads(line)
        except json.JSONDecodeError:
            continue


def newest():
    files = sorted(DIR.glob("acswarm-*.jsonl"))
    if not files:
        raise SystemExit(f"no telemetry in {DIR}")
    return files[-1]


def summarise(path):
    """Per character: a dict of measures, and the stalls and marks behind them."""
    by = defaultdict(lambda: {"samples": [], "chat": [], "status": [], "refused": 0, "marks": [], "setup": None})
    for r in records(path):
        who = (r.get("a", "?"), r.get("c", "?"))
        k = r.get("k")
        if k == "sample":
            by[who]["samples"].append(r)
        elif k == "chat":
            by[who]["chat"].append(r)
        elif k == "status":
            by[who]["status"].append(r)
        elif k == "refused":
            by[who]["refused"] += 1
        elif k == "mark":
            by[who]["marks"].append(r)
        elif k == "setup":
            by[who]["setup"] = r
    out = {}
    for who, d in by.items():
        s = d["samples"]
        if not s:
            continue
        hours = max((s[-1]["t"] - s[0]["t"]) / 3.6e6, 1e-9)
        doing = Counter(x.get("doing") or "?" for x in s)
        chat = [c["text"] for c in d["chat"]]
        kills = deaths = None
        if KILLER is not None:
            kills = sum(1 for t in chat if any(rx.search(t) for rx in KILLER))
            deaths = sum(1 for t in chat if "You" in t and any(rx.search(t) for rx in VICTIM))
        texts = [x.get("text", "") for x in d["status"]]
        runs = sum(1 for t in texts if t.startswith("town run done"))
        sold = sum(int(m.group(1)) for t in texts for m in [re.match(r"town run done: sold (\d+)", t)] if m)
        bought = sum(int(m.group(1)) for t in texts for m in [re.match(r"buying (\d+) ", t)] if m)
        gave_up = sum(1 for t in texts if re.search(r"giving up|gave up|could not get to|would not trade", t))
        stalls = find_stalls(s)
        out[who] = {
            "hours": hours,
            "level": (s[0].get("lvl"), s[-1].get("lvl")),
            "xp_per_h": ((s[-1].get("xp") or 0) - (s[0].get("xp") or 0)) / hours,
            "coin": (s[0].get("coin"), s[-1].get("coin")),
            "doing": {k: v / len(s) for k, v in doing.most_common()},
            "kills": kills,
            "deaths": deaths,
            "town_runs": runs,
            "sold": sold,
            "bought": bought,
            "gave_up": gave_up,
            "refused": d["refused"],
            "stall_s": sum(x["secs"] for x in stalls),
            "stalls": stalls,
            "marks": d["marks"],
            "setup": d["setup"],
        }
    return out


def find_stalls(samples):
    stalls, run = [], []

    def close():
        if run and (run[-1]["t"] - run[0]["t"]) / 1000 + SAMPLE_GAP >= STALL_AFTER:
            stalls.append({
                "secs": (run[-1]["t"] - run[0]["t"]) / 1000 + SAMPLE_GAP,
                "cell": run[0].get("cell"),
                "status": run[0].get("status"),
                "t": run[0]["t"],
            })

    for a, b in zip(samples, samples[1:]):
        pa, pb = a.get("pos"), b.get("pos")
        meaning = MOVING.search(b.get("status") or "") is not None
        still = pa and pb and sum((x - y) ** 2 for x, y in zip(pa, pb)) ** 0.5 < 1.0
        if meaning and still:
            if not run:
                run.append(a)
            run.append(b)
        else:
            close()
            run = []
    close()
    return sorted(stalls, key=lambda x: -x["secs"])


def per_hour(v, hours):
    return "?" if v is None else f"{v / hours:.1f}"


def show(path):
    print(f"== {path}")
    for (acct, name), m in summarise(path).items():
        h = m["hours"]
        print(f"\n{name} ({acct}): {h * 60:.0f} min, level {m['level'][0]} -> {m['level'][1]}, "
              f"coin {m['coin'][0]} -> {m['coin'][1]}")
        print(f"  per hour: kills {per_hour(m['kills'], h)}, deaths {per_hour(m['deaths'], h)}, "
              f"xp {m['xp_per_h']:,.0f}, town runs {m['town_runs'] / h:.1f}")
        print("  time: " + ", ".join(f"{k} {v:.0%}" for k, v in list(m["doing"].items())[:7]))
        print(f"  sold {m['sold']}, bought {m['bought']}, gave up {m['gave_up']}, refused {m['refused']}, "
              f"marks {len(m['marks'])}")
        if m["setup"]:
            print("  setup: " + brief(m["setup"]))
        if m["stalls"]:
            print(f"  stalls: {len(m['stalls'])}, {m['stall_s']:.0f} s in all; longest:")
            for st in m["stalls"][:5]:
                print(f"    {st['secs']:5.0f} s at {st['cell']}  {st['status']}")


def brief(setup):
    c = setup.get("config", {})
    f, g, t = c.get("fight", {}), c.get("growth", {}), c.get("team", {})
    on = lambda b: "on" if b else "off"
    return (f"autoplay {on(c.get('enabled'))}, {setup.get('sessions')} session(s), profile "
            f"{c.get('loot', {}).get('profile')!r}, fight {on(f.get('enabled'))} {f.get('style')} "
            f"r{f.get('radius')}, town runs {on(g.get('town_runs'))}, grounds {on(g.get('hunt_grounds'))}, "
            f"xp {on(g.get('auto_xp'))}, team {on(t.get('enabled'))} {t.get('role')}"
            + (f", area {f['area'].get('name', 'set')}" if f.get("area") else ""))


def setups(path):
    last = {}
    for r in records(path):
        if r.get("k") == "setup":
            last[(r.get("a"), r.get("c"))] = r
    for (acct, name), r in last.items():
        print(f"== {name} ({acct})")
        print(json.dumps(r.get("config"), indent=1))


def compare(a, b):
    sa, sb = summarise(a), summarise(b)
    print(f"{'':28}{'A':>14}{'B':>14}   A={Path(a).name}  B={Path(b).name}")

    def total(s, key):
        hours = sum(m["hours"] for m in s.values()) or 1e-9
        vals = [m[key] for m in s.values()]
        if any(v is None for v in vals):
            return "?"
        return f"{sum(vals) / hours:.1f}"

    for label, key in [("kills / h", "kills"), ("deaths / h", "deaths"), ("town runs / h", "town_runs"),
                       ("sold / h", "sold"), ("gave up / h", "gave_up"), ("refused / h", "refused"),
                       ("stall seconds / h", "stall_s")]:
        print(f"{label:28}{total(sa, key):>14}{total(sb, key):>14}")
    for side, s in (("A", sa), ("B", sb)):
        xp = sum(m["xp_per_h"] * m["hours"] for m in s.values()) / (sum(m["hours"] for m in s.values()) or 1)
        print(f"{'xp / h (' + side + ')':28}{xp:>14,.0f}")


def marks(path):
    rows = list(records(path))
    for i, r in enumerate(rows):
        if r.get("k") != "mark":
            continue
        who = (r.get("a"), r.get("c"))
        print(f"\n== MARK {r.get('c')} at {r.get('cell')} doing {r.get('doing')}: {r.get('status')}")
        print(f"   hp {r.get('hp')} st {r.get('st')} mp {r.get('mp')} slots {r.get('slots')} "
              f"burden {r.get('burden')} coin {r.get('coin')} target {r.get('target')!r}")
        near = [x for x in rows[max(0, i - 400):i + 40]
                if (x.get("a"), x.get("c")) == who and x.get("k") in ("status", "chat", "refused")]
        for x in near[-25:]:
            print(f"   {x.get('k'):7} {x.get('text') or x.get('code')}")


if __name__ == "__main__":
    args = sys.argv[1:]
    if args[:1] == ["--compare"] and len(args) == 3:
        compare(args[1], args[2])
    elif args[:1] == ["--setup"]:
        setups(args[1] if len(args) > 1 else newest())
    elif args[:1] == ["--marks"]:
        marks(args[1] if len(args) > 1 else newest())
    else:
        for p in args or [newest()]:
            show(p)
