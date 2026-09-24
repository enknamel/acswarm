#!/usr/bin/env bash
# Does autoplay play? Fresh characters (war mage, life caster, soldier) start in Holtburg with every
# rule on, for MINUTES, against the local ACE; then tools/autoplay-report.py scores the log for
# each thing a player expects: out of the Academy, buffs, fights, heals, loot, sales, experience.
#
#   ACSWARM_TEST_PASSWORD=... tools/autoplay-check.sh [MINUTES] [--prefix check] [--out DIR]
#
# Accounts PREFIX01..03 (ACE makes each on its first login) and their characters ("Prefix Warmage"...)
# are created when missing, so a second run carries on from where the first left them. Only 127.0.0.1 is contacted.
set -euo pipefail
minutes=25
prefix=check
out=""
while [ $# -gt 0 ]; do
  case "$1" in
    --prefix) prefix="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    *) minutes="$1"; shift ;;
  esac
done
: "${ACSWARM_TEST_PASSWORD:?set ACSWARM_TEST_PASSWORD to the password of the local test accounts}"
repo=$(cd "$(dirname "$0")/.." && pwd)
out=${out:-$(mktemp -d "${TMPDIR:-/tmp}/autoplay-check.XXXXXX")}
mkdir -p "${out}/cfg/profiles" "${out}/scripts"
cp "${repo}/tools/autoplay-check-profile.json" "${out}/cfg/profiles/Check.json"
cat > "${out}/scripts/check.rhai" <<'RHAI'
// Every rule on, the check's loot profile: what a player gets from the Autoplay switch.
fn command(name, args) {
    if name == "check_wait" { return true; }
    if name != "check_on" { return false; }
    set_loot_profile("Check");
    growth(true);
    fight(true);
    autoplay(true);
    log("CHECK autoplay on as " + me().name);
    true
}
RHAI
# Asked again now and then: a character still being created misses the first.
{ for i in 1 2 3 4 5 6; do echo "/check_on"; for w in $(seq 1 20); do echo "/check_wait"; done; done; } > "${out}/lines.txt"
cargo build --release -p acswarm --manifest-path "${repo}/Cargo.toml" >/dev/null 2>&1
bin="${repo}/target/release/acswarm"
p="${ACSWARM_TEST_PASSWORD}"
# Names are unique on the server and letters only, so they follow the prefix: "Check Warmage".
who=$(printf '%s' "${prefix}" | tr -cd 'A-Za-z')
who="$(printf '%s' "${who:0:1}" | tr '[:lower:]' '[:upper:]')${who:1}"
echo "autoplay-check: ${minutes} min, logs in ${out}"
ACSWARM_CONFIG_DIR="${out}/cfg" ACSWARM_SCRIPTS="${out}/scripts" AC_DATA_DIR="${AC_DATA_DIR:-${HOME}/Downloads/ac_data}" \
RUST_LOG="warn,acswarm=info,ac_client=info" "${bin}" --headless --connect 127.0.0.1 \
  --client "${prefix}01:${p}:${who} Warmage:war:holtburg" \
  --client "${prefix}02:${p}:${who} Healer:life:holtburg" \
  --client "${prefix}03:${p}:${who} Soldier:soldier:holtburg" \
  --script "${out}/lines.txt" --duration $((minutes * 60)) > "${out}/run.log" 2>&1 || true
python3 "${repo}/tools/autoplay-report.py" "${out}/run.log"
