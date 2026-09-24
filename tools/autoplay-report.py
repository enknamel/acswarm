#!/usr/bin/env python3
"""Score an autoplay run's log: tools/autoplay-report.py RUN.log

One column per character (the log's s{a=ACCOUNT} span), one row per thing a player expects autoplay
to do, and a verdict. Reads a log written with RUST_LOG=warn,acswarm=info,ac_client=info, as
tools/autoplay-check.sh writes it. Kills are the server's own kill shouts, matched by the templates
tools/fleet-score.py reads from the ACE checkout.
"""
import importlib.util
import re
import sys

sys.dont_write_bytecode = True  # importing fleet-score.py would leave a __pycache__ in tools/
from collections import Counter, defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("fleet_score", HERE / "fleet-score.py")
fleet = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fleet)

ANSI = re.compile(r"\x1b\[[0-9;]*m")
SPAN = re.compile(r"s\{a=([\w-]+)\}")
STATUS = re.compile(r"^\[([\w-]+)\] placed=(\w+) cell=([0-9A-F]{8}) hp=(\d+)/(\d+)")

# (row, pattern on the line after the span, "chat" when it is a chat line's text)
SIGNALS = [
    ("left the Academy", r"academy: .*(out of the Academy|left the Academy|through the way out|outside)", None),
    ("buffs cast", r"autoplay: casting .+ on ", None),
    ("spells at creatures", r"autoplay: casting .+ at ", None),
    ("heals", r"autoplay: (healing|heal) ", None),
    ("corpses opened", r"use Corpse of ", None),
    ("items looted", r"(actions::items: take .+ \(0x[0-9a-f]+\)|emptied a corpse)", None),
    ("town runs begun", r"autoplay: .*going to .*\(\d+ m\)", None),
    ("counters asked", r"autoplay: at .*; looking over", None),
    ("sales sent", r"trade_vendor: sell \d+ item", None),
    ("experience spent", r"autoplay: raising ", None),
]
PROBLEMS = [
    ("server refusals", r"the server refused \((\w+)"),
    ("given up on", r"autoplay: (giving up on|gave up on|cannot reach)"),
    ("counters that would not trade", r"would not trade"),
    ("no way there", r"(no way on to|could not be got to|No way there|no way to )"),
    ("walks too long", r"too long a walk|taking too long"),
]


def score(path):
    per = defaultdict(Counter)
    names = {}
    first_cell, last_cell, died = {}, {}, Counter()
    refusals = defaultdict(Counter)
    lines = [ANSI.sub("", l.rstrip("\n")) for l in open(path, encoding="utf-8", errors="replace")]
    for line in lines:
        m = re.search(r"CHECK autoplay on as (.+)$", line)
        if m:
            acct = re.match(r"^\[([\w-]+)\]", line)
            if acct:
                names[acct.group(1)] = m.group(1).strip()
    for line in lines:
        st = STATUS.match(line)
        if st and st.group(2) == "yes":
            a = st.group(1)
            first_cell.setdefault(a, st.group(3))
            last_cell[a] = st.group(3)
            continue
        sp = SPAN.search(line)
        if not sp:
            continue
        a = sp.group(1)
        rest = line[sp.end():]
        me = re.escape(names.get(a, "\x00"))
        chat = fleet.CHAT.search(line)
        text = chat.group(1) if chat else None
        for row, pat, kind in SIGNALS:
            p = pat.replace("{me}", me)
            if kind == "chat":
                if text is not None and re.search(p, text):
                    per[a][row] += 1
            elif re.search(p, rest):
                per[a][row] += 1
        if text is not None:
            for rx in fleet.KILLER:
                if rx.search(text):
                    per[a]["kills"] += 1
                    break
            for rx in fleet.VICTIM:
                if rx.search(text) and "You" in text:
                    died[a] += 1
                    break
        for row, pat in PROBLEMS:
            m = re.search(pat, rest)
            if m:
                per[a][row] += 1
                if row == "server refusals":
                    refusals[a][m.group(1)] += 1
    accounts = sorted(set(per) | set(last_cell))
    if not accounts:
        raise SystemExit("no s{a=...} lines: was the log written with the per-session span?")
    rows = [r for r, _, _ in SIGNALS[:2]] + ["kills"] + [r for r, _, _ in SIGNALS[2:]]
    w = 30
    print(f"{'':{w}}" + "".join(f"{names.get(a, a)[:18]:>20}" for a in accounts))
    for a in accounts:
        out = last_cell.get(a, "?")[:4] != "8602"
        per[a]["left the Academy"] = max(per[a]["left the Academy"], int(out))
    for row in rows:
        print(f"{row:{w}}" + "".join(f"{per[a][row]:>20}" for a in accounts))
    print(f"{'deaths':{w}}" + "".join(f"{died[a]:>20}" for a in accounts))
    print(f"{'where it ended':{w}}" + "".join(f"{last_cell.get(a, '?'):>20}" for a in accounts))
    print()
    for row, _ in PROBLEMS:
        print(f"{row:{w}}" + "".join(f"{per[a][row]:>20}" for a in accounts))
    for a in accounts:
        if refusals[a]:
            print(f"  {names.get(a, a)} refusals: " + ", ".join(f"{k} x{n}" for k, n in refusals[a].most_common()))
    print()
    expect = ["left the Academy", "kills", "corpses opened", "town runs begun", "sales sent", "experience spent"]
    for a in accounts:
        missing = [r for r in expect if per[a][r] == 0]
        verdict = "all seen" if not missing else "never seen: " + ", ".join(missing)
        print(f"{names.get(a, a)}: {verdict}")


if __name__ == "__main__":
    for p in sys.argv[1:]:
        score(p)
