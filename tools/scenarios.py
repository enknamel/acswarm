#!/usr/bin/env python3
"""Pass or fail for each character of a tools/scenarios.sh run: tools/scenarios.py RUN_DIR

Reads the telemetry in RUN_DIR/cache/telemetry (samples, status lines, noted lines and chat) and
exits 1 when a check fails, so a merge can wait on it.
"""
import importlib.util
import re
import sys
from collections import defaultdict
from pathlib import Path

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("telemetry", HERE / "telemetry.py")
tel = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tel)

FIGHTERS = ("Scn Mage", "Scn Blade", "Scn Bow")
MIN_KILLS = 3
MAX_STALL = 30.0  # seconds a character may mean to walk and stand still
MAX_IDLE = 90.0  # seconds a character with autoplay on may stand "waiting" at a stretch


def per_character(path):
    """Each character's lines (status and noted) and its samples with autoplay on."""
    lines, samples = defaultdict(list), defaultdict(list)
    for r in tel.records(path):
        who = (r.get("c") or "").lstrip("+")
        if r.get("k") in ("status", "note"):
            lines[who].append(r.get("text", ""))
        elif r.get("k") == "sample" and r.get("on"):
            samples[who].append(r)
    return lines, samples


def longest_idle(samples):
    longest, since = 0.0, None
    for s in samples:
        if s.get("doing") == "waiting":
            since = since or s["t"]
            longest = max(longest, (s["t"] - since) / 1000 + tel.SAMPLE_GAP)
        else:
            since = None
    return longest


def checks(name, m, lines, samples):
    """(check, passed, measured) for one character."""
    count = lambda rx: sum(1 for t in lines if re.search(rx, t))
    out = []
    if name in FIGHTERS:
        corpses = count(r"^(emptied|nothing on) Corpse of")
        out += [
            (f"kills >= {MIN_KILLS}", (m["kills"] or 0) >= MIN_KILLS, m["kills"]),
            ("corpses opened", corpses > 0, corpses),
            ("no deaths", (m["deaths"] or 0) == 0, m["deaths"]),
        ]
        if name == "Scn Mage":
            buffs = sum(1 for t in lines if t.startswith("casting ") and " at " not in t)
            out.append(("buffs cast", buffs > 0, buffs))
    elif name == "Scn Taper":
        tapers = sum(int(g.group(1)) for t in lines for g in [re.match(r"buying (\d+) .*Taper", t)] if g)
        out += [("tapers bought", tapers > 0, tapers), ("town run done", m["town_runs"] > 0, m["town_runs"])]
    elif name == "Scn Seller":
        out.append(("ten rings sold", m["sold"] >= 10, m["sold"]))
    elif name.startswith("Fresh"):
        cell = samples[-1].get("cell", "?") if samples else "?"
        out.append(("left the Academy", not cell.startswith("8602"), cell))
    longest = m["stalls"][0]["secs"] if m["stalls"] else 0.0
    where = f"{longest:.0f} s at {m['stalls'][0]['cell']}" if m["stalls"] else "0 s"
    out.append((f"no stall over {MAX_STALL:.0f} s", longest <= MAX_STALL, where))
    idle = longest_idle(samples)
    out.append((f"not idle over {MAX_IDLE:.0f} s", idle <= MAX_IDLE, f"{idle:.0f} s"))
    return out


def main(run):
    files = sorted((Path(run) / "cache" / "telemetry").glob("acswarm-*.jsonl"))
    if not files:
        raise SystemExit(f"no telemetry under {run}/cache/telemetry")
    path = files[-1]
    lines, samples = per_character(path)
    failed = 0
    for (acct, name), m in sorted(tel.summarise(path).items(), key=lambda kv: kv[0][0]):
        # A developer account's characters carry ACE's plus: "+Scn Mage".
        plain = name.lstrip("+")
        print(f"{name} ({acct}): level {m['level'][0]} -> {m['level'][1]}, {m['hours'] * 60:.0f} min, "
              f"time: " + ", ".join(f"{k} {v:.0%}" for k, v in list(m["doing"].items())[:4]))
        for what, ok, measured in checks(plain, m, lines[plain], samples[plain]):
            failed += not ok
            print(f"  {'pass' if ok else 'FAIL'}  {what:22} {measured}")
    print(f"\n{'all passed' if not failed else f'{failed} check(s) failed'}  (telemetry {path})")
    return 1 if failed else 0


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    sys.exit(main(sys.argv[1]))
