#!/usr/bin/env python3
"""Score a six-character fleet run: tools/fleet-score.py RUN.log [RUN.log ...]

This is how a behaviour change is judged. Run the arms interleaved (A-1, B-1,
A-2, B-2) so spawn depletion and machine load fall on both alike, and never on
a loaded machine: kills per run are timing-sensitive.

It reads the client's own log, so it depends on tracing targets, which are
module paths and therefore change whenever a module moves. It once matched
`ac_client: chat: ` literally; a module move made that `ac_client::session::chat`
and every run afterwards scored zero kills, silently, for hours. Hence the
sanity check below: telemetry with no chat at all is a stale pattern, not a
quiet run. If you move chat again, this is the file that breaks.

Clean signals (the god characters' packs are full, so loot counts are
confounded):
- kills: chat kill shouts. ACE sends the killer, and only the killer, the
  first string of a DeathMessage (Strings.cs); the templates are read from
  the ACE reference checkout at run time, victims starting with '+'
  (players) excluded.
- deaths: chat lines matching a DeathMessage's victim string.
- opens per corpse: `ac_client: use Corpse of X (0x...)` lines per guid.
- "already in use" refusals: "is already in use by someone else".
- stalls: from the 1 s AREA lines, per character, a run of samples with the
  same cell and local x/y (to 0.1 m) while meaning to move, lasting >= 6 s.
  "Meaning to move" is navscore.py's rule: traveling=true or the status
  holds "on the way", "walking", "looking", "in sight" or "getting".
- time shares: AREA samples by status kind.
- INSTR counts, when the binary carries the instrumentation.
"""
import pathlib
import re
import sys
from collections import Counter, defaultdict

ANSI = re.compile(r"\x1b\[[0-9;]*m")
# ACE's own death-message templates, so the kill shouts are never guessed.
# The checkout is gitignored (reference/ext/), so this fails clearly when it is absent.
REPO = pathlib.Path(__file__).resolve().parent.parent
STRINGS = REPO / "reference/ext/ACE/Source/ACE.Server/Entity/Strings.cs"
LIT = r'"((?:[^"\\]|\\.)*)"'
DM = re.compile(r"new DeathMessage\(\s*" + LIT + r",\s*" + LIT + r",\s*" + LIT + r"\)")
AREA = re.compile(
    r"AREA t=(\d+) who=(\S+) cell=(\S+) local=\(([-\d.]+),([-\d.]+),([-\d.]+)\) "
    r"hp=(-?\d+)/(-?\d+) traveling=(\w+) ap=(.*)$"
)
FLEET = re.compile(r"FLEET t=(\d+) who=(\S+) xp=(\d+) hp=(\d+)/(\d+)")
USE = re.compile(r"ac_client(?:::[\w:]+)?: use Corpse of .+? \((0x[0-9a-f]+)\)")
# Wave 1 moved chat to its own module, so the tracing target went from
# `ac_client` to `ac_client::session::chat`. Accept any ac_client target,
# or this reads every post-wave-1 run as zero kills.
CHAT = re.compile(r"ac_client(?:::[\w:]+)?: chat: (.*)$")
INSTR = re.compile(r"INSTR (second reflex acted|melee pick differs|melee pick agrees|unknown health judged dead): (.*)$")
MOVING = ("on the way", "walking", "looking", "in sight", "getting")
# The fleet's own statuses that also mean "going somewhere": a follower,
# a fighter joining the team's target, one catching up or heading back.
MOVING_BROAD = MOVING + ("following", "catching up", "joining on", "back to", "going to",
                         "stepping out of")


def stall_runs(rows, rule):
    """Runs of >= 6 s at one spot while `rule(trav, ap)` says it means to move."""
    found = []
    run = None
    for t, key, trav, ap in rows:
        moving = rule(trav, ap)
        if run and run[0] == key and moving:
            run[2] = t
        else:
            if run and run[2] - run[1] >= 6:
                found.append((run[1], run[2] - run[1], run[3]))
            run = [key, t, t, ap[:60]] if moving else None
    if run and run[2] - run[1] >= 6:
        found.append((run[1], run[2] - run[1], run[3]))
    return found


def template_regex(t):
    out = ""
    for part in re.split(r"(\{[01]\})", t):
        if part == "{0}":
            out += r"(?P<victim>.+?)"
        elif part == "{1}":
            out += r"(?P<killer>.+?)"
        else:
            out += re.escape(part)
    return re.compile("^" + out + "$")


def death_templates():
    if not STRINGS.exists():
        raise SystemExit(f"no ACE checkout at {STRINGS}: clone it to score kill shouts")
    text = STRINGS.read_text()
    killer, victim = [], []
    for m in DM.finditer(text):
        killer.append(template_regex(m.group(1)))
        victim.append(template_regex(m.group(2)))
    return killer, victim


KILLER, VICTIM = death_templates()


def kind(ap):
    ap = ap.strip()
    if not ap or ap == "waiting":
        return "idle"
    for prefix, label in (
        ("fighting", "fighting"), ("attacking", "fighting"), ("joining on", "fighting"),
        ("shooting", "fighting"), ("changing weapon", "fighting"), ("making ", "fighting"),
        ("opening", "looting"), ("looting", "looting"), ("taking", "looting"),
        ("emptying", "looting"), ("walking to Corpse", "looting"), ("going to Corpse", "looting"),
        ("pack full, leaving", "looting"), ("waiting for", "waiting on a mate"),
        ("dodging", "dodging"), ("following", "following"), ("catching up", "following"),
        ("back to", "keeping to area"), ("casting", "casting"), ("healing", "healing"),
    ):
        if ap.startswith(prefix):
            return label
    return "other"


def measure(path):
    corpses = defaultdict(int)
    kills = deaths = in_use = lines = kills_died = 0
    kill_names = Counter()
    death_lines = Counter()
    instr = Counter()
    instr_guids = defaultdict(set)
    unknown_sites = Counter()
    unknown_sites_nonplayer = Counter()
    shares = Counter()
    per_char = defaultdict(list)
    xp = defaultdict(list)
    exit_line = ""
    for raw in open(path, errors="replace"):
        line = ANSI.sub("", raw).rstrip("\n")
        lines += 1
        if line.startswith("EXIT"):
            exit_line = line
        a = AREA.search(line)
        if a:
            t, who, cell, x, y, _z, _hp, _hpm, trav, ap = a.groups()
            per_char[who].append((int(t), (cell, round(float(x), 1), round(float(y), 1)), trav, ap))
            shares[kind(ap)] += 1
            continue
        f = FLEET.search(line)
        if f:
            xp[f.group(2)].append(int(f.group(3)))
            continue
        m = USE.search(line)
        if m:
            corpses[m.group(1)] += 1
        if "is already in use by someone else" in line:
            in_use += 1
        c = CHAT.search(line)
        if c:
            text = c.group(1).strip()
            for rx in KILLER:
                k = rx.match(text)
                if k and not k.group("victim").startswith("+"):
                    kills += 1
                    # "{0} died!" is also ACE's broadcast form: watch it.
                    kills_died += text.endswith(" died!")
                    kill_names[k.group("victim")] += 1
                    break
            else:
                for rx in VICTIM:
                    if rx.match(text):
                        deaths += 1
                        death_lines[text] += 1
                        break
        i = INSTR.search(line)
        if i:
            what, rest = i.groups()
            instr[what] += 1
            g = re.search(r"0x[0-9a-f]+", rest)
            if what == "second reflex acted":
                instr["second reflex acted: " + rest.strip()] += 1
            if g:
                instr_guids[what].add(g.group(0))
            if what == "unknown health judged dead":
                site = re.search(r"site=(\S+)", rest).group(1)
                unknown_sites[site] += 1
                if "player=false" in rest:
                    unknown_sites_nonplayer[site] += 1
    def narrow(trav, ap):
        return trav == "true" or any(k in ap for k in MOVING)

    def broad(trav, ap):
        return trav == "true" or any(k in ap for k in MOVING_BROAD)

    stalls, stalls_broad = [], []
    for who, rows in per_char.items():
        stalls += [(who,) + s for s in stall_runs(rows, narrow)]
        stalls_broad += [(who,) + s for s in stall_runs(rows, broad)]
    samples = sum(shares.values())
    xp_gain = sum(v[-1] - v[0] for v in xp.values() if v)
    return dict(
        path=path.split("/")[-1], lines=lines, exit=exit_line, characters=len(per_char),
        samples=samples, kills=kills, kill_names=kill_names, deaths=deaths,
        death_lines=death_lines, corpses=len(corpses), opens=sum(corpses.values()),
        per_corpse=sum(corpses.values()) / max(len(corpses), 1), in_use=in_use,
        stalls=stalls, stall_s=sum(s[2] for s in stalls), shares=shares, xp=xp_gain,
        stalls_broad=stalls_broad, stall_broad_s=sum(s[2] for s in stalls_broad),
        kills_died=kills_died,
        instr=instr, instr_guids=instr_guids, unknown_sites=unknown_sites,
        unknown_sites_nonplayer=unknown_sites_nonplayer,
    )


def sanity(path, chat_lines, area_samples):
    """A run with telemetry but no chat at all means the patterns have gone
    stale, not that nothing was said. Wave 1 moved chat's tracing target from
    `ac_client` to `ac_client::session::chat` and this scorer read every run
    after it as zero kills for hours. Fail loudly instead."""
    if area_samples > 100 and chat_lines == 0:
        raise SystemExit(
            f"{path}: {area_samples} AREA samples but no chat lines matched.\n"
            "The CHAT pattern is stale -- check the tracing target in the log."
        )


def main():
    runs = [measure(p) for p in sys.argv[1:]]
    w = max(16, *(len(r["path"]) + 2 for r in runs))
    head = f"{'':<40}" + "".join(f"{r['path']:>{w}}" for r in runs)
    print(head)
    rows = [
        ("characters (AREA)", lambda r: r["characters"]),
        ("AREA samples (1 s, all characters)", lambda r: r["samples"]),
        ("kills (kill shouts)", lambda r: r["kills"]),
        ("deaths (victim lines)", lambda r: r["deaths"]),
        ("corpses opened (distinct guids)", lambda r: r["corpses"]),
        ("opens sent", lambda r: r["opens"]),
        ("opens per corpse", lambda r: f"{r['per_corpse']:.2f}"),
        ("'already in use' refusals", lambda r: r["in_use"]),
        ("  of which '<name> died!' matches", lambda r: r["kills_died"]),
        ("stalls >= 6 s (navscore rule)", lambda r: len(r["stalls"])),
        ("stalled seconds (navscore rule)", lambda r: r["stall_s"]),
        ("stalls >= 6 s (broad rule)", lambda r: len(r["stalls_broad"])),
        ("stalled seconds (broad rule)", lambda r: r["stall_broad_s"]),
        ("xp gained (sum of FLEET deltas)", lambda r: f"{r['xp']:,}"),
    ]
    for label in ("fighting", "looting", "idle", "casting", "following", "dodging",
                  "waiting on a mate", "keeping to area", "healing", "other"):
        rows.append((f"% samples {label}",
                     lambda r, l=label: f"{100 * r['shares'][l] / max(r['samples'], 1):.1f}"))
    for what in ("second reflex acted", "melee pick differs", "melee pick agrees",
                 "unknown health judged dead"):
        rows.append((f"INSTR {what}", lambda r, w_=what: r["instr"][w_]))
        rows.append((f"  distinct guids", lambda r, w_=what: len(r["instr_guids"][w_])))
    for site in ("worth_fighting.engaged", "worth_fighting.nearest", "a_fight_in_sight.joined"):
        rows.append((f"  unknown health {site}", lambda r, s=site: r["unknown_sites"][s]))
        rows.append((f"    of which non-player", lambda r, s=site: r["unknown_sites_nonplayer"][s]))
    for label, fn in rows:
        print(f"{label:<40}" + "".join(f"{str(fn(r)):>{w}}" for r in runs))
    for r in runs:
        print(f"\n== {r['path']} {r['exit']}")
        print("   kills by creature:", dict(r["kill_names"].most_common(10)))
        if r["death_lines"]:
            print("   death lines:", dict(r["death_lines"]))
        reflex = {k: v for k, v in r["instr"].items() if k.startswith("second reflex acted: ")}
        if reflex:
            print("   second reflex by step:", reflex)
        for who, t, secs, ap in sorted(r["stalls"], key=lambda s: -s[2])[:10]:
            print(f"   stall (navscore) {who} t={t} {secs}s: {ap}")
        for who, t, secs, ap in sorted(r["stalls_broad"], key=lambda s: -s[2])[:10]:
            print(f"   stall (broad) {who} t={t} {secs}s: {ap}")


if __name__ == "__main__":
    main()
