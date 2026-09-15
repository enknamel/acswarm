# acswarm

A Rust Asheron's Call client for the ACE server emulator: DAT reader, renderer, wire protocol, and
characters that play on their own, many per process. Before changing `crates/ac-client`, read
`crates/ac-client/CLAUDE.md`; `crates/ac-client/tests/code_map.rs` fails when a map goes stale.

## Build, test, run

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                                      # what CI runs; data tests show as ignored
AC_DATA_DIR=$HOME/Downloads/ac_data cargo test-data         # every test, the data tests too
cargo test -p ac-client critter                             # one crate, filtered by test name
cargo run -p xtask -- same-code BASE                        # comment-only proof (`facts BASE`: removed facts)
cargo build --release -p acswarm
```

Data tests are `#[ignore = "needs AC_DATA_DIR"]` (folder: `ac_dat::test_data_dir()`, builders:
`ac_client::testkit`); run `cargo test-data` before calling a change done. CI also gates intra-doc
links (`rustdoc` job). Golden files: `tests/golden` (`tools/regen-golden.sh`).

Local ACE server in Docker (needs the ACE source cloned to reference/ext/ACE and `AC_DATA_DIR`
set; login on udp/9000; the first login creates its account):

```sh
tools/ace/up.sh      # build and start
tools/ace/logs.sh    # follow the server log
tools/ace/down.sh    # stop
validate/run.sh      # live acceptance suite (validate/suite.rhai) with the release acswarm
cargo run --release -p acswarm -- --headless --connect 127.0.0.1 \
  --client ACCOUNT:PASSWORD[:CHARACTER] --duration 60 --log-chat   # --create NAME, --script FILE, --bus
```

Parallel worktrees: each its own `CARGO_TARGET_DIR` (target/wt-NAME) and `CARGO_INCREMENTAL=0` (a shared
dir gives false compile errors); check `df -g /` first, stop below 15 GB free, delete the dir when done.

## Crates (libraries in dependency order; "uses" drops the ac- prefix)

| crate | role | entry | uses |
|---|---|---|---|
| ac-dat | DAT container: block chains, B-tree directory, file by id; knows nothing of contents | `DatArchive` | - |
| ac-formats | one decoder module per asset type; every `parse` consumes its bytes exactly | `gfxobj`, `setup`, `spell_table` | dat |
| ac-scene | GPU-free assembly: memoizing loader, terrain, landblocks, interiors, collision, nav graph, particles, chargen | `Assets` | dat, formats |
| ac-audio | plays decoded waves on kira; sound-table lookup | `Audio` | dat, formats |
| ac-net | wire protocol, sans-IO: packets, ISAAC, messages, session state machine; the caller owns the socket | `session::Session` | - |
| ac-world | object table and character sheet built from server messages; world data (shops, spawns, portals, trips) | `World::apply` | formats, net |
| ac-agent | vocabulary every system shares: outcomes and retry, weenie errors, refusals in words, pack arithmetic | `did::Did`, `did::Patience`, `refusals::refused` | - |
| ac-nav | getting somewhere: steering per landblock, walk or portals, obstacles, dungeon doorways; answers `Aim::NoWay` instead of walking at a wall | `Steering`, `how_to_get_there` | formats, scene, world |
| ac-loot | item vocabulary and search language, loot profiles, ledger, emptying a corpse, sale fate, weapon choice | `Profile`, `Run::step`, `Ledger` | agent, net, world |
| ac-vendor | the town run at a counter, decided from a snapshot with no server | `Run::step`, `Snapshot` | agent, loot |
| ac-client | one headless game session: socket, world, body, manual actions, autoplay | `Client::tick` | agent, formats, loot, nav, net, scene, vendor, world |
| ac-bus | cross-process bus: JSON lines over loopback TCP, hosted by the first process | `BusServer`, `BusClient` | - |
| ac-plugin | plugin trait and host, blackboard, settings, built-in panels, team and fleet, servers store | `Plugin`, `Host`, `Ctx` | bus, client, formats, net, scene, vendor, world |
| ac-script | Rhai scripts as a plugin, hot-reloaded; `crates/ac-script/src/api.rs` lists every script call | `ScriptPlugin`, `Api` | client, net, plugin, world |

| bin | role | entry |
|---|---|---|
| acswarm | the client: wgpu window, `--headless` sessions, viewer and `--screenshot` modes; built-in plugins in `bins/acswarm/src/plugins/mod.rs` | `App` in `bins/acswarm/src/main.rs`, `run` in `bins/acswarm/src/headless.rs` |
| aclauncher | desktop launch manager: servers and accounts, one acswarm process per launch | `bins/aclauncher/src/main.rs` |
| acdat | DAT CLI: info, ls, cat, extract, decode, wav, manifest, diff | `bins/acdat/src/main.rs` |
| plugin-template | example plugin to copy (panel, key, setting, command) with an offline runner | `examples/plugin-template` |

## Data flow

```
client_*.dat -> ac-dat -> ac-formats -> ac-scene::Assets   (opened once per process, Rc-shared by sessions)
                                            |-> collision, nav graph -> ac-nav -> ac-client body and walks
                                            '-> acswarm GPU batches          (the active session only)
UDP <-> ac-net::Session <-> ac-client::Client::tick <-> ac-world::World::apply
                                    |-> autoplay rules, deciding through ac-agent, ac-loot, ac-vendor, ac-nav
                                    '-> Vec<Event> -> ac-plugin::Host -> panels, ac-script, ac-bus
```

- `Client::tick` sends what `Session` queued, drains the socket into it and applies each message.
  Wire structs that build world state live in ac-world; framing and non-world messages in ac-net.
- ac-client renders nothing; only the session the window shows holds GPU state. Window, headless
  loop, panels, scripts and the bus all act through `Client` methods and `Event`s.
- `~/.config/acswarm` (`$ACSWARM_CONFIG_DIR`) holds the settings, `~/.acswarm` the launcher config
  and scripts (`$ACSWARM_SCRIPTS`). Passwords live in the servers store (`ac_plugin::servers`).

## Logging

- `RUST_LOG` is a tracing filter (default `warn,acswarm=info`) whose targets are module paths:
  `RUST_LOG=warn,ac_client::travel=debug`, `RUST_LOG=warn,ac_client::autoplay=debug,ac_nav=debug`.
- Explicit targets: `wire` (trace, every packet and message in and out) and `steer` (trace, obstacle
  detours and the walk): `RUST_LOG=warn,wire=trace`. The log panel reloads the filter live
  (`bins/acswarm/src/logging.rs`, list in `ac_plugin::logging::SYSTEMS`); headless adds `--log-chat`.

## Comments

- At most two lines per item: what it answers if the name does not, then the non-obvious why.
  Module `//!` at most five lines; field docs one line; constants one line with unit and source.
- No history ("used to", incident stories): point at the guarding test or `(regression: <sha>)`.
- Never cut, only compress: protocol facts, server behaviour with its ACE `File.cs:line`, retail
  addresses, invariants and ordering, measured thresholds, coordinate frames, cross-session
  agreement, reasons for `unwrap`/`allow`, and the refusals table's wording rows.
- Comment-only edits, pure moves and code changes go in separate commits.

## Names

- Production fns use the glossary's nouns and verbs (`crates/ac-client/CLAUDE.md`), predicates
  `is_`/`has_`/`can_`, no articles or pronouns, at most four words. Test names stay sentences.
- No leaf module name repeats across the workspace; no two types share a name with different meanings.
- A STEPS row is named after its run fn's suffix (`keep_to_area` for `autoplay_keep_to_area`). Rename
  per subsystem, in that subsystem's commit, with old to new listed in the message.
- Saved config fields are renamed only with `#[serde(alias = "old")]` (saved configs live outside the
  repo); Rhai and plugin API renames migrate scripts, validate, the plugin template and docs.

## Project rules

- Server refusals in words are matched only in `ac_agent::refusals` and handed to the waiting
  system by `hear_refusal()` (crates/ac-client/src/refused.rs); no system matches server English.
- Slash commands are only those the retail client registered; never invent one.
- Characters walk straight through doors: navigation never stops at, opens or routes round one
  (`in_the_way()` in crates/ac-nav/src/obstacles.rs).
- The loot profile's Sell/Keep tag is final; only the server's own refusal (wielded, tinkered,
  unsellable, no value) stands ahead of it (`offer_to_vendor()` in crates/ac-loot/src/sale.rs).
- ACE is AGPL: read it and cite `File.cs:line`, never copy it. Game data, passwords and session
  logs never enter the repo.
