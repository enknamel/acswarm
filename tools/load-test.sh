#!/usr/bin/env bash
# Many headless sessions against the local ACE server, with autoplay on, sampled every 5 s,
# then one table of what they cost. How to read it: docs/performance.md, "Measuring many sessions".
#
#   ACSWARM_LOAD_PASSWORD=... tools/load-test.sh SESSIONS SECONDS [--procs P] [--prefix load]
#                                                [--out DIR] [--bin PATH]
#
# Accounts are PREFIX01, PREFIX02, ... (ACE creates one on its first login), each with one
# character named from its number ("Load Zero One"), created when missing. The password is
# read from ACSWARM_LOAD_PASSWORD and never printed or written; it does reach acswarm's
# command line, as every --client does. Only 127.0.0.1 is ever contacted.
set -euo pipefail

usage() {
  sed -n '2,6p' "$0" | sed 's/^# \{0,1\}//' >&2
  exit 2
}

root="$(cd "$(dirname "$0")/.." && pwd)"
sessions=""
duration=""
procs=1
prefix=load
out=""
bin=""
while (($#)); do
  case "$1" in
    --procs) procs="${2:?--procs needs a number}"; shift 2 ;;
    --prefix) prefix="${2:?--prefix needs a name}"; shift 2 ;;
    --out) out="${2:?--out needs a directory}"; shift 2 ;;
    --bin) bin="${2:?--bin needs a path}"; shift 2 ;;
    -h | --help) usage ;;
    -*) echo "load-test: unknown option $1" >&2; usage ;;
    *)
      if [[ -z "${sessions}" ]]; then sessions="$1"
      elif [[ -z "${duration}" ]]; then duration="$1"
      else usage
      fi
      shift ;;
  esac
done

die() { echo "load-test: $*" >&2; exit 1; }

[[ "${sessions}" =~ ^[0-9]+$ ]] && ((sessions >= 1 && sessions <= 999)) || die "SESSIONS must be 1..999"
# The local ACE admits 128 authenticated sessions in all, everyone's (MaximumAllowedSessions in
# reference/ace-run/Config/Config.js, NetworkManager.cs:101-104); past that a login is refused as
# the logon server is full, which acswarm counts as a drop and retries.
((sessions <= 128)) ||
  echo "load-test: warning: the local ACE admits 128 sessions (MaximumAllowedSessions); the rest will retry" >&2
[[ "${duration}" =~ ^[0-9]+$ ]] && ((duration >= 1)) || die "SECONDS must be a whole number of seconds"
[[ "${procs}" =~ ^[0-9]+$ ]] && ((procs >= 1 && procs <= sessions)) || die "--procs must be 1..SESSIONS"
# Letters only: the character names are made from it, and the server takes letters,
# spaces, hyphens and apostrophes in a name (crates/ac-client/src/creation.rs, valid_name).
[[ "${prefix}" =~ ^[a-z]{1,12}$ ]] || die "--prefix must be 1..12 lowercase letters"
pw="${ACSWARM_LOAD_PASSWORD:-}"
[[ -n "${pw}" ]] || die "set ACSWARM_LOAD_PASSWORD to the load accounts' password"
[[ "${pw}" != *:* ]] || die "the password may not contain a colon (--client splits on them)"

# The machine must have room for the logs and whatever else is building.
free_gb=$(df -g / 2>/dev/null | awk 'NR == 2 { print $4 }') ||
  free_gb=$(df -BG / | awk 'NR == 2 { sub("G", "", $4); print $4 }')
((free_gb >= 15)) || die "only ${free_gb} GB free on /; stopping below 15 GB"

# Accounts and characters: two digits up to 99 sessions, three past that, so an account keeps
# its number and its character. ACE refuses a name only when taboo, a creature's or in use
# (CharacterHandler.cs:51-70), and taboo patterns match word by word (TabooTableEntry.cs:43-50),
# so a name is the prefix and ordinary number words.
width=2
((sessions > 99)) && width=3
digit_words=(Zero One Two Three Four Five Six Seven Eight Nine)
title="$(printf '%s' "${prefix:0:1}" | tr 'a-z' 'A-Z')${prefix:1}"
accounts=()
names=()
for ((i = 1; i <= sessions; i++)); do
  num=$(printf "%0${width}d" "${i}")
  name="${title}"
  for ((d = 0; d < ${#num}; d++)); do
    name="${name} ${digit_words[${num:d:1}]}"
  done
  accounts+=("${prefix}${num}")
  names+=("${name}")
done

# One session per account: a process already holding one would be refused or would drop. Only
# command lines are read (--client, or -a as aclauncher passes it), not a window's fleet panel.
for acct in "${accounts[@]}"; do
  if running=$(pgrep -f -- "(--client[ =]${acct}:|(-a|--account)[ =]${acct}( |$))" 2>/dev/null); then
    die "account ${acct} is already in use by pid(s) $(echo ${running}); stop it first"
  fi
done

# Without --bin, HEAD's release acswarm, built every time (nothing to do when it is current), in
# the target folder cargo itself reports, which follows CARGO_TARGET_DIR relative to the repo.
if [[ -z "${bin}" ]]; then
  echo "load-test: building the release acswarm" >&2
  (cd "${root}" && cargo build --release -p acswarm)
  target=$(cd "${root}" && cargo metadata --no-deps --format-version 1 |
    sed -E -n 's/.*"target_directory":"([^"]*)".*/\1/p')
  bin="${target}/release/acswarm"
fi
[[ -x "${bin}" ]] || die "no acswarm binary at ${bin}"
# A piece of the `perf run:` line's format (TickMeter's summary) that the summary below reads:
# an older binary would log every account in and leave the tick rows empty.
grep -a -q -F ' ms, plugins ' "${bin}" ||
  die "${bin} predates the perf run line this script reads; rebuild it or drop --bin"

data_dir="${AC_DATA_DIR:-}"
if [[ -z "${data_dir}" ]]; then
  saved="${ACSWARM_CONFIG_DIR:-${HOME}/.config/acswarm}/data-dir"
  [[ -f "${saved}" ]] && data_dir="$(cat "${saved}")"
fi
[[ -f "${data_dir}/client_portal.dat" ]] || die "set AC_DATA_DIR to the folder holding client_portal.dat"

# The absolute, symlink-free form of a folder whose last parts may not exist yet.
resolve() {
  local path="$1" rest=""
  [[ "${path}" == /* ]] || path="${PWD}/${path}"
  while [[ "${path}" == */ && "${path}" != / ]]; do path="${path%/}"; done
  while [[ ! -d "${path}" ]]; do
    rest="/${path##*/}${rest}"
    path="${path%/*}"
    [[ -n "${path}" ]] || path=/
  done
  printf '%s%s\n' "$(cd "${path}" && pwd -P)" "${rest}"
}

# Logs stay out of the repo: session logs never enter it. Checked before anything is made.
repo="$(cd "${root}" && pwd -P)"
common="$(git -C "${root}" rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true)"
is_in_repo() {
  local inside
  for inside in "${repo}" "${common%/.git}"; do
    [[ -n "${inside}" && "$1/" == "${inside}/"* ]] && return 0
  done
  return 1
}
if [[ -n "${out}" ]]; then
  out="$(resolve "${out}")"
  [[ "${out}/" != */../* && "${out}/" != */./* ]] ||
    die "--out ${out} climbs through folders that do not exist yet; name it without . or .."
  is_in_repo "${out}" && die "--out ${out} is inside the repository"
  # A folder of its own: an earlier run's proc logs and samples would join this run's summary.
  [[ ! -d "${out}" || -z "$(ls -A "${out}")" ]] ||
    die "--out ${out} already holds files; name a new or empty folder"
  mkdir -p "${out}"
else
  out="$(mktemp -d "${TMPDIR:-/tmp}/acswarm-load-$(date +%Y%m%d-%H%M%S).XXXX")"
  out="$(cd "${out}" && pwd -P)"
  if is_in_repo "${out}"; then rmdir "${out}"; die "TMPDIR is inside the repository; pass --out"; fi
fi

# A config and cache of their own, so the run plays by the default rules and its characters
# stay out of the everyday settings, ledger and holdings; the world grid is copied in.
user_cache="${ACSWARM_CACHE_DIR:-${HOME}/.cache/acswarm}"
mkdir -p "${out}/cache" "${out}/scripts"
if [[ -f "${user_cache}/worldgrid.bin" ]]; then
  cp -c "${user_cache}/worldgrid.bin" "${out}/cache/" 2>/dev/null ||
    cp "${user_cache}/worldgrid.bin" "${out}/cache/"
fi
cat >"${out}/scripts/load_autoplay.rhai" <<'RHAI'
// Written by tools/load-test.sh: the scripted `/load_autoplay` turns autoplay on once placed.
fn command(name, args) {
    if name != "load_autoplay" { return false; }
    autoplay(true);
    log("load test: autoplay on");
    true
}
RHAI
printf '%s\n' "/load_autoplay" >"${out}/lines.txt"

pids=()
sampler=""

# Every child goes on exit: acswarm logs off on SIGINT, SIGTERM or SIGHUP (up to 10 s,
# ac_client::LOG_OFF_WAIT), so it gets that long before SIGKILL; a killed session holds its
# account on the server for about a minute.
# SIGINT goes again every second: bash starts a background child with SIGINT ignored, and until
# acswarm's handler is in (about a second into startup) one would be lost; after, it is harmless.
cleanup() {
  local code=$? live=() pid tries any
  trap - EXIT INT TERM HUP
  [[ -z "${sampler}" ]] || { kill "${sampler}" && wait "${sampler}"; } 2>/dev/null || true
  for pid in "${pids[@]+"${pids[@]}"}"; do
    kill -0 "${pid}" 2>/dev/null && live+=("${pid}")
  done
  if ((${#live[@]})); then
    echo "load-test: stopping ${#live[@]} acswarm process(es); they log off first" >&2
    for ((tries = 0; tries < 15; tries++)); do
      any=0
      for pid in "${live[@]}"; do kill -0 "${pid}" 2>/dev/null && any=1; done
      ((any)) || break
      kill -INT "${live[@]}" 2>/dev/null || true
      sleep 1
    done
    for pid in "${live[@]}"; do kill -KILL "${pid}" 2>/dev/null || true; done
  fi
  exit "${code}"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

# The first Ctrl-C once the sessions run logs them off and still prints the table (the wait
# below sends SIGINT on every second); a second stops the script, which gives them the same
# 15 s as on any exit.
interrupted=0
interrupt() {
  if ((interrupted)); then exit 130; fi
  interrupted=1
  echo "load-test: interrupted; the sessions log off (Ctrl-C again to stop harder)" >&2
}
trap interrupt INT

echo "load-test: ${sessions} session(s) in ${procs} process(es) for ${duration} s; logs in ${out}"
start=0
for ((p = 0; p < procs && !interrupted; p++)); do
  count=$((sessions / procs + (p < sessions % procs ? 1 : 0)))
  args=(--headless --data-dir "${data_dir}" --connect 127.0.0.1 --duration "${duration}"
    --script "${out}/lines.txt")
  for ((i = start; i < start + count; i++)); do
    # ACCOUNT:PASSWORD:CHARACTER: with the empty fourth field: create it when missing.
    args+=(--client "${accounts[i]}:${pw}:${names[i]}:")
    echo "load-test: proc$((p + 1)) ${accounts[i]} as ${names[i]}"
  done
  start=$((start + count))
  mkdir -p "${out}/proc$((p + 1))-config"
  ACSWARM_CONFIG_DIR="${out}/proc$((p + 1))-config" ACSWARM_CACHE_DIR="${out}/cache" \
    ACSWARM_SCRIPTS="${out}/scripts" RUST_LOG="warn,acswarm=info" NO_COLOR=1 \
    "${bin}" "${args[@]}" >"${out}/proc$((p + 1)).log" 2>&1 &
  pids+=($!)
done
unset args
((${#pids[@]})) || exit 130 # interrupted before the first launch

# Every 5 s: each process's RSS (KB), ps %CPU and CPU time, and where it is: 0 starting, 1 in
# its loop (after acswarm's `headless: N session(s) to` line), 2 past it (a `perf run:` line);
# and the server container's CPU and memory. One tab-separated row each.
sample() {
  local next now p pid row any ace log phase
  next=$(date +%s)
  while :; do
    now=$(date +%s)
    any=0
    for p in "${!pids[@]}"; do
      pid=${pids[p]}
      row=$(ps -o rss=,%cpu=,time= -p "${pid}" 2>/dev/null) || continue
      any=1
      log="${out}/proc$((p + 1)).log"
      phase=0
      grep -q -F 'session(s) to ' "${log}" && phase=1
      grep -q -F 'perf run: ' "${log}" && phase=2
      # shellcheck disable=SC2086 # the row is split into its three columns on purpose
      printf '%s\tproc%d\t%s\t%s\t%s\t%s\t%s\n' "${now}" $((p + 1)) "${pid}" ${row} "${phase}"
    done
    if command -v docker >/dev/null 2>&1 &&
      ace=$(docker stats --no-stream --format '{{.CPUPerc}}\t{{.MemUsage}}' ace-server 2>/dev/null); then
      printf '%s\tace\t-\t%s\n' "${now}" "${ace}"
    fi
    ((any)) || break
    next=$((next + 5))
    now=$(date +%s)
    if ((next > now)); then sleep $((next - now)); else next=${now}; fi
  done >"${out}/samples.tsv"
}
sample &
sampler=$!

# Polled rather than waited on, so that after a Ctrl-C SIGINT goes again every second (see
# cleanup); bash keeps each exit status for the one wait after.
failed=()
for p in "${!pids[@]}"; do
  pid=${pids[p]}
  while kill -0 "${pid}" 2>/dev/null; do
    if ((interrupted)); then kill -INT "${pids[@]}" 2>/dev/null || true; fi
    sleep 1
  done
  status=0
  wait "${pid}" 2>/dev/null || status=$?
  ((status == 0)) || failed+=("proc$((p + 1)) exited with status ${status}")
done
kill "${sampler}" 2>/dev/null || true
wait "${sampler}" 2>/dev/null || true
sampler=""

# ---- the summary ------------------------------------------------------------------------
logs=()
for ((p = 1; p <= ${#pids[@]}; p++)); do logs+=("${out}/proc${p}.log"); done
strip() { sed -E $'s/\x1b\\[[0-9;]*m//g' "$@"; }
placed=$(strip "${logs[@]}" | sed -E -n 's/^\[([^]]+)\] placed in cell .*/\1/p' | sort -u | wc -l | tr -d ' ')
autoplay=$(strip "${logs[@]}" | grep -c 'load test: autoplay on' || true)
refused=$(strip "${logs[@]}" | grep -c 'already logged on' || true)
unmade=$(strip "${logs[@]}" | grep -c 'character creation failed\|cannot create' || true)
full=$(strip "${logs[@]}" | grep -c 'the logon server is full' || true)
denied=$(strip "${logs[@]}" | grep -c 'not accepted\|booted this account' || true)

echo
echo "load-test: each process's whole run"
run_lines=()
for log in "${logs[@]}"; do
  line=$(strip "${log}" | sed -n 's/.*\(perf run: .*\)/\1/p' | tail -1)
  echo "  $(basename "${log}" .log): ${line:-no perf run line (it crashed or was killed)}"
  [[ -z "${line}" ]] || run_lines+=("${line}")
done

# The run lines' numbers, worst of the processes column by column: the lowest rate, the Hz,
# the highest work p50, p95 and max, the period, the highest late p95, per session, plugins and
# overruns per minute, and seconds measured and of wall from the process measured least, then
# how many lines had the shape pinned by `the_run_line_has_the_shape_the_harness_reads`
# (bins/acswarm/src/tick_meter.rs) and how many did not. A field is `-` when nothing was timed.
ticks=$(printf '%s\n' "${run_lines[@]+"${run_lines[@]}"}" | awk '
  BEGIN {
    v = "([0-9.]+|-)"
    shape = "^perf run: [0-9]+ sessions, [0-9]+ ticks at " v " of [0-9]+ Hz, work p50 " v \
      " ms p95 " v " max " v " of [0-9.]+ ms, over [0-9]+, behind [0-9]+, late p95 " v \
      " ms, per session " v " ms, plugins " v " ms, costliest .*, [0-9.]+ overruns/min, " \
      "[0-9]+ s measured of [0-9]+ s wall, [0-9]+ sleep windows excluded \\([0-9]+ s asleep\\)$"
  }
  function most(i, x) { if (x != "-" && (!(i in w) || x + 0 > w[i])) w[i] = x + 0 }
  function f(i, fmt) { return (i in w) ? sprintf(fmt, w[i]) : "-" }
  NF == 0 { next }
  $0 !~ shape { bad++; next }
  {
    n++; k = split($0, t, " ")
    if (t[8] != "-" && (!("rate" in w) || t[8] + 0 < w["rate"])) w["rate"] = t[8] + 0
    hz = t[10]; period = t[21]
    most("p50", t[14]); most("p95", t[17]); most("max", t[19]); most("late", t[29])
    most("per", t[33]); most("plugins", t[36]); most("over", t[k - 15])
    if (!n0 || t[k - 13] + 0 < measured) { n0 = 1; measured = t[k - 13] + 0; wall = t[k - 9] + 0 }
  }
  END {
    if (!n) { printf "-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t-\t0\t%d\n", bad; exit }
    printf "%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%d\t%d\t%d\t%d\n", f("rate", "%.1f"), hz,
      f("p50", "%.1f"), f("p95", "%.1f"), f("max", "%.1f"), period, f("late", "%.1f"),
      f("per", "%.2f"), f("plugins", "%.2f"), f("over", "%.1f"), measured, wall, n, bad
  }')
IFS=$'\t' read -r rate hz p50 p95 pmax period late per_session plugins over measured wall \
  parsed unparsed <<<"${ticks}"
sleeps=$(printf '%s\n' "${run_lines[@]+"${run_lines[@]}"}" |
  sed -E -n 's/.*, ([0-9]+) sleep windows excluded.*/\1/p' |
  awk 'BEGIN { m = 0 } $1 > m { m = $1 } END { print m }')

# The samples: peak total RSS; mean CPU from CPU time over the pairs of samples inside the loop
# at most 10 s apart (a wider gap is a machine sleep), and ps %CPU, a decaying average, for the
# peak beside it; and the server's CPU and memory.
res=$(awk -F'\t' '
  function secs(t, n, a, d, s, i) {
    d = 0; if (index(t, "-")) { d = substr(t, 1, index(t, "-") - 1); t = substr(t, index(t, "-") + 1) }
    n = split(t, a, ":"); s = 0
    for (i = 1; i <= n; i++) s = s * 60 + a[i]
    return d * 86400 + s
  }
  function mb(m, v) {
    split(m, v, " "); m = v[1]
    if (m ~ /GiB$/) return m * 1024; if (m ~ /MiB$/) return m + 0
    if (m ~ /KiB$/) return m / 1024; if (m ~ /kB$/) return m / 1000
    if (m ~ /GB$/) return m * 1000; if (m ~ /MB$/) return m + 0
    return m / 1048576
  }
  $2 ~ /^proc/ && $4 > 0 {
    rss[$1] += $4
    c = secs($6)
    if ($7 == 1) {
      pcpu[$1] += $5
      if (pp[$3] == 1 && $1 - pt[$3] <= 10) { dc[$3] += c - pc[$3]; dt[$3] += $1 - pt[$3] }
    }
    pt[$3] = $1; pc[$3] = c; pp[$3] = $7
    samples++
  }
  $2 == "ace" {
    sub("%", "", $4); acpu += $4; an++; if ($4 + 0 > amax) amax = $4 + 0
    m = mb($5); if (m > amem) amem = m
  }
  END {
    for (t in rss) { if (rss[t] > peak) peak = rss[t]; if (pcpu[t] > ppeak) ppeak = pcpu[t] }
    for (p in dt) if (dt[p] > 0) cpu += dc[p] / dt[p] * 100
    printf "%.1f\t%.1f\t%.1f\t%s\t%s\t%s\t%d\n", peak / 1024, cpu, ppeak,
      an ? sprintf("%.1f", acpu / an) : "-", an ? sprintf("%.1f", amax) : "-",
      an ? sprintf("%.0f", amem) : "-", samples
  }' "${out}/samples.tsv" 2>/dev/null || printf '0\t0\t0\t-\t-\t-\t0\n')
IFS=$'\t' read -r rss_mb cpu_mean cpu_peak ace_cpu ace_peak ace_mem nsamples <<<"${res}"
per_session_mb=$(awk -v t="${rss_mb}" -v n="${sessions}" 'BEGIN { printf "%.1f", t / n }')
worst=""
((procs > 1)) && worst=" (worst of ${procs} processes)"

{
  echo
  echo "| measure | value |"
  echo "|---|---|"
  echo "| sessions | ${sessions} asked, ${placed} placed, ${autoplay} with autoplay on |"
  echo "| processes | ${procs} |"
  echo "| RSS | ${rss_mb} MB peak total, ${per_session_mb} MB per session (process base included) |"
  echo "| acswarm CPU | ${cpu_mean}% mean, ${cpu_peak}% peak ps sample, in the loop (all processes; one core = 100%) |"
  echo "| ACE CPU | ${ace_cpu}% mean, ${ace_peak}% peak; ${ace_mem} MB peak memory |"
  echo "| tick rate | ${rate} of ${hz} Hz${worst} |"
  echo "| tick work | p50 ${p50} ms, p95 ${p95} ms, max ${pmax} ms of ${period} ms${worst} |"
  echo "| overruns | ${over} per minute${worst} |"
  echo "| late | p95 ${late} ms${worst} |"
  echo "| per session tick | ${per_session} ms mean, the session's own work${worst} |"
  echo "| plugins | ${plugins} ms per tick, every session's calls together${worst} |"
  echo "| measured | ${measured} s of ${wall} s wall, the process measured least; ${nsamples} process samples |"
  echo "| sleep windows excluded | ${sleeps} |"
} | tee "${out}/summary.md"
((refused)) && echo "load-test: ${refused} line(s) say an account was still logged on: wait a minute and rerun"
((unmade)) && echo "load-test: ${unmade} character creation(s) failed; the proc logs say why"
((full)) && echo "load-test: ${full} login(s) refused as the logon server is full: ACE admits 128 sessions"
((denied)) && echo "load-test: ${denied} login(s) refused: ACE boots a wrong password for an existing account (AuthenticationHandler.cs:168-175); check ACSWARM_LOAD_PASSWORD"
echo "load-test: logs, samples.tsv and summary.md in ${out}"

problems=("${failed[@]+"${failed[@]}"}")
((placed >= sessions)) || problems+=("only ${placed} of ${sessions} sessions were placed")
((parsed >= procs)) || problems+=("only ${parsed} of ${procs} processes left a perf run line this script can read (${unparsed} of another shape); the tick rows leave the others out")
for problem in "${problems[@]+"${problems[@]}"}"; do echo "load-test: ${problem}" >&2; done
((interrupted)) && exit 130
exit $((${#problems[@]} ? 1 : 0))
