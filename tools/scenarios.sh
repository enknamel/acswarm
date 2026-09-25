#!/usr/bin/env bash
# Does autoplay still play? Run before merging an autoplay change. Five level-20 characters start in
# Holtburg and one new one in the Academy, every rule on, for MINUTES on the local ACE; then
# tools/scenarios.py says pass or fail for each and exits 1 on a failure.
#
#   ACSWARM_TEST_PASSWORD=... tools/scenarios.sh [MINUTES] [--out DIR]
#
#   fight    Scn Mage, Scn Blade, Scn Bow hunt: kills, corpses opened, buffs, no deaths
#   restock  Scn Taper starts each run with no tapers at the Mosswart ground, 2.5 km out: it buys some
#   sell     Scn Seller starts each run with ten rings: a town run sells them
#   academy  a new character each run leaves the Training Academy
#   (all)    no stall: never stands still for long while meaning to walk
#
# Accounts scn01..05 are made on the first run and given developer access in ace_auth (for @grantxp,
# @teleloc and @ci), so their characters keep their level, gear and loot from run to run. Each run
# starts them in the same places, so two runs measure the code and not where the last one ended.
set -euo pipefail
minutes=10
out=""
while [ $# -gt 0 ]; do
  case "$1" in
    --out) out="$2"; shift 2 ;;
    *) minutes="$1"; shift ;;
  esac
done
: "${ACSWARM_TEST_PASSWORD:?set ACSWARM_TEST_PASSWORD to the password of the local test accounts}"
repo=$(cd "$(dirname "$0")/.." && pwd)
out=${out:-$(mktemp -d "${TMPDIR:-/tmp}/scenarios.XXXXXX")}
mkdir -p "${out}/cfg/profiles" "${out}/scripts"
# The check profile, keeping the fixtures' own weapons as a player's profile keeps theirs: an
# unwielded weapon is loot to the Check profile, and a counter took the Battle Axe from Scn Blade.
python3 - "${repo}/tools/autoplay-check-profile.json" "${out}/cfg/profiles/Check.json" <<'PY2'
import json, sys
profile = json.load(open(sys.argv[1]))
keep = [{"name": f"our {w}", "on": True, "action": "keep", "all": [{"Item": {"Word": w}}], "keep_up_to": None}
        for w in ("Battle Axe", "Longbow", "Arrow")]
profile["rules"] = keep + profile["rules"]
json.dump(profile, open(sys.argv[2], "w"), indent=1)
PY2
cat > "${out}/scripts/scenario.rhai" <<'RHAI'
// Sets each character up for its scenario, asked every twenty seconds: every step looks before it
// acts, so an early or repeated ask costs nothing. The @commands need the account's developer access.
fn command(name, args) {
    if name == "scn_wait" { return true; }
    if name != "scn" { return false; }
    let m = me();
    let who = m.name;
    if who.starts_with("+") { who = who.sub_string(1); }
    if !who.starts_with("Fresh") {
        if (m.cell >> 16) == 0x8602 {
            say("@teleloc 0xA9B40019 84 7.1 94");
            return true;
        }
        if m.level < 20 { say("@grantxp 300000"); }
        if type_of(board_get("scn.set." + who)) == "()" {
            board_set("scn.set." + who, true);
            if who == "Scn Taper" {
                say("@teleloc 0xBAAD0017 65.4 144.0 88.0");
            } else {
                say("@teleloc 0xA9B40019 84 7.1 94");
            }
            // A level-20 fighter owns a real weapon: the Academy's training bow hits for 0-1.
            let coin = 0; let axe = false; let bow = false; let arrows = 0;
            for it in inventory() {
                if it.name == "Pyreal" { coin += it.stack; }
                if it.name == "Battle Axe" { axe = true; }
                if it.name == "Longbow" { bow = true; }
                if it.name == "Arrow" { arrows += it.stack; }
            }
            if coin < 3000 { say("@ci 273 5000"); }
            if who == "Scn Blade" && !axe { say("@ci 301"); }
            if who == "Scn Bow" && !bow { say("@ci 306"); }
            if who == "Scn Bow" && arrows < 200 { say("@ci 300 250"); }
            if who == "Scn Seller" { for i in 0..10 { say("@ci 297"); } }
        }
    }
    // The tapers go on each of the first three asks: a drop sent with the teleport is refused
    // while the character is in portal space, and a town run takes longer than a minute.
    if who == "Scn Taper" {
        let asks = board_get("scn.asks." + who);
        let asks = if type_of(asks) == "()" { 0 } else { asks };
        if asks < 3 {
            for it in inventory() { if it.name.contains("Taper") { drop_item(it.guid); } }
        }
        board_set("scn.asks." + who, asks + 1);
    }
    set_loot_profile("Check");
    growth(true);
    fight(true);
    autoplay(true);
    if type_of(board_get("scn.on." + who)) == "()" {
        board_set("scn.on." + who, true);
        log("SCN on as " + who + " level " + m.level);
    }
    true
}
RHAI
# One ask, then nineteen seconds of waiting, for the whole run.
for i in $(seq 1 $((minutes * 3))); do echo "/scn"; for w in $(seq 1 19); do echo "/scn_wait"; done; done > "${out}/lines.txt"
cargo build --release -p acswarm --manifest-path "${repo}/Cargo.toml" >/dev/null 2>&1
bin="${repo}/target/release/acswarm"
p="${ACSWARM_TEST_PASSWORD}"
# ACE lists a character of a developer account with a plus ("+Scn Mage"), so once the accounts have
# their access the plus is part of the name the client looks for (CharacterHandler.cs:374).
fixtures() {
  local acct name tmpl row
  specs=()
  for row in "scn01|Scn Mage|war" "scn02|Scn Blade|soldier" "scn03|Scn Bow|bow" \
    "scn04|Scn Taper|war" "scn05|Scn Seller|soldier"; do
    IFS='|' read -r acct name tmpl <<<"${row}"
    specs+=(--client "${acct}:${p}:$1${name}:${tmpl}:holtburg")
  done
}
db() { docker exec -i ace-db sh -c 'mysql -uroot -p"$MYSQL_ROOT_PASSWORD" -N --batch ace_auth' 2>/dev/null; }
accounts="'scn01','scn02','scn03','scn04','scn05'"
run() {
  ACSWARM_CONFIG_DIR="${out}/cfg" ACSWARM_SCRIPTS="${out}/scripts" ACSWARM_CACHE_DIR="${out}/cache" \
  AC_DATA_DIR="${AC_DATA_DIR:-${HOME}/Downloads/ac_data}" RUST_LOG="warn,acswarm=info,ac_client=info" \
    "${bin}" --headless --connect 127.0.0.1 "$@" || true
}
if [ "$(echo "SELECT COUNT(*) FROM account WHERE accountName IN (${accounts}) AND accessLevel >= 4;" | db)" != 5 ]; then
  echo "scenarios: first run, making the five characters (one minute)"
  fixtures ""
  run "${specs[@]}" --duration 60 > "${out}/create.log" 2>&1
  echo "UPDATE account SET accessLevel = 4 WHERE accountName IN (${accounts}) AND accessLevel < 4;" | db
  sleep 10
fi
# A new character each run; names are unique on the server and letters only.
stamp=$(date +%s | tr 0-9 a-j)
fresh=(--client "scnnew$(date +%s):${p}:Fresh $(printf '%s' "${stamp:0:1}" | tr a-j A-J)${stamp:1}:war:holtburg")
echo "scenarios: ${minutes} min, logs in ${out}"
fixtures "+"
run "${specs[@]}" "${fresh[@]}" --script "${out}/lines.txt" --duration $((minutes * 60)) > "${out}/run.log" 2>&1
python3 "${repo}/tools/scenarios.py" "${out}"
