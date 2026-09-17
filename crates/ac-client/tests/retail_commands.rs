//! What the chat box does with a line, and which retail commands are answered.
//! Every name the retail client registered is either a row of `action::RETAIL`
//! or on one of the three lists, so the families landing later can read what is
//! left off `action::PENDING`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use ac_client::action::{self, Action, Line, CLIENT_UI_ONLY, PENDING, RETAIL, RETIRED};
use ac_client::testkit;
use ac_net::messages::{channel, turbine};

/// Every command name the retail client registered, from its own command table
/// (131 registrations, `a` twice: the legacy allegiance room and the Turbine
/// one). Read out of `reference/research/retail-slash-commands.json`, which is
/// not in the repo; `the_names_are_the_ones_retail_registered` checks this list
/// against that file wherever it is on disk.
const RETAIL_NAMES: &[&str] = &[
    "?",
    "help",
    "commands",
    "allegiances",
    "allegiance",
    "all",
    "ab",
    "alh",
    "ah",
    "motd",
    "speaker",
    "channels",
    "a",
    "co-vassals",
    "covassals",
    "covassal",
    "c",
    "fellowship",
    "fellows",
    "fellow",
    "f",
    "group",
    "g",
    "party",
    "monarch",
    "m",
    "patron",
    "p",
    "vassals",
    "vassal",
    "v",
    "join",
    "leave",
    "chatting",
    "chat",
    "notell",
    "reply",
    "r",
    "rp",
    "mr",
    "pr",
    "retell",
    "rt",
    "say",
    "s",
    "tell",
    "t",
    "send",
    "whisper",
    "w",
    "afk",
    "death",
    "consent",
    "corpse",
    "cor",
    "die",
    "lifestone",
    "lif",
    "ls",
    "marketplace",
    "mar",
    "mp",
    "permit",
    "pkarena",
    "pka",
    "pklarena",
    "pla",
    "e",
    "em",
    "emote",
    "me",
    "emotes",
    "fillcomps",
    "loadfile",
    "friends",
    "friends_add",
    "friends_remove",
    "house",
    "hou",
    "hslist",
    "hor",
    "hr",
    "hom",
    "hoa",
    "squelch",
    "unsquelch",
    "messagetypes",
    "message_types",
    "msgtypes",
    "msg_types",
    "status",
    "age",
    "birth",
    "day",
    "endurance",
    "framerate",
    "loc",
    "pklite",
    "pkl",
    "render",
    "version",
    "saveui",
    "loadui",
    "saveautoui",
    "loadautoui",
    "lockui",
    "text",
    "clear",
    "filter",
    "unfilter",
    "log",
    "title",
    "index",
    "clist",
    "on",
    "off",
    "guild",
    "gu",
    "general",
    "cg",
    "trade",
    "ct",
    "lfg",
    "clfg",
    "roleplay",
    "crp",
    "society",
    "soc",
    "olthoi",
    "o",
];

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the workspace above crates/ac-client")
        .to_path_buf()
}

/// Where each retail name is answered, or waiting to be.
fn homes() -> BTreeMap<&'static str, Vec<String>> {
    let mut homes: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    for cmd in RETAIL {
        for name in cmd.names {
            homes
                .entry(name)
                .or_default()
                .push(format!("the row /{}", cmd.names[0]));
        }
    }
    for (list, label) in [
        (PENDING, "PENDING"),
        (CLIENT_UI_ONLY, "CLIENT_UI_ONLY"),
        (RETIRED, "RETIRED"),
    ] {
        for name in list {
            homes.entry(name).or_default().push(label.to_string());
        }
    }
    homes
}

#[test]
fn every_retail_command_has_a_row_or_a_list() {
    let homes = homes();
    let mut missing = Vec::new();
    let mut twice = Vec::new();
    for name in RETAIL_NAMES {
        match homes.get(name).map(Vec::as_slice) {
            None => missing.push(*name),
            Some([_]) => {}
            Some(many) => twice.push(format!("/{name}: {}", many.join(" and "))),
        }
    }
    assert!(
        missing.is_empty(),
        "retail commands with no row and on no list: {missing:?}"
    );
    assert!(
        twice.is_empty(),
        "retail commands answered twice: {twice:?}"
    );
}

#[test]
fn no_row_or_list_invents_a_command() {
    let retail: BTreeSet<&str> = RETAIL_NAMES.iter().copied().collect();
    let invented: Vec<&str> = homes()
        .into_keys()
        .filter(|n| !retail.contains(n))
        .collect();
    assert!(
        invented.is_empty(),
        "names the retail client never registered: {invented:?} \
         (a command of ours is a key, a panel or a script)"
    );
}

#[test]
fn the_names_are_the_ones_retail_registered() {
    let path = workspace().join("reference/research/retail-slash-commands.json");
    let Ok(text) = fs::read_to_string(&path) else {
        // Research material, not in the repo: the list above stands on its own.
        return;
    };
    let registered: BTreeSet<String> = text
        .split("\"name\": \"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .map(|n| n.to_lowercase())
        .map(|n| n.split(" (").next().unwrap_or_default().trim().to_string())
        .collect();
    let ours: BTreeSet<String> = RETAIL_NAMES.iter().map(|n| n.to_string()).collect();
    // The file holds the registrations and other names beside them (the
    // handlers' own, the wiki's); every name of ours must be one of them.
    let unknown: Vec<&String> = ours.difference(&registered).collect();
    assert!(
        unknown.is_empty(),
        "names in no registration in {}: {unknown:?}",
        path.display()
    );
}

#[test]
fn a_retail_command_never_reaches_the_plugin_hooks() {
    let mut c = testkit::offline_client();
    let sent = c.session.actions_sent();
    assert_eq!(
        c.chat_line("/tell Verity, hello"),
        Line::Acted(Ok(())),
        "a row acts, and is not offered to the plugins"
    );
    assert_eq!(c.session.actions_sent(), sent + 1, "the tell went out");
    assert!(
        matches!(c.chat_line("/emote"), Line::Acted(Err(_))),
        "a row with nothing to act out is refused, not passed on"
    );
}

#[test]
fn a_command_of_ours_is_offered_to_the_plugins_and_scripts() {
    let mut c = testkit::offline_client();
    let sent = c.session.actions_sent();
    // validate/lines.txt paces its suite with /nop, and validate/suite.rhai
    // answers /v_*; both reach the scripts through the hooks.
    let Line::Offer {
        name,
        args,
        unclaimed,
    } = c.chat_line("/v_burden two")
    else {
        panic!("a script's own command must be offered to it");
    };
    assert_eq!((name.as_str(), args.as_str()), ("v_burden", "two"));
    assert_eq!(unclaimed, Action::ServerCommand("v_burden two".into()));
    assert!(matches!(c.chat_line("/nop"), Line::Offer { .. }));
    assert_eq!(
        c.session.actions_sent(),
        sent,
        "nothing goes out until the hooks have had it"
    );
}

#[test]
fn a_server_command_reaches_the_server() {
    let mut c = testkit::offline_client();
    let Line::Offer { unclaimed, .. } = c.chat_line("@ci") else {
        panic!("an unknown command is offered first");
    };
    assert_eq!(unclaimed, Action::ServerCommand("ci".into()));
    let sent = c.session.actions_sent();
    assert_eq!(c.act(unclaimed), Ok(()));
    assert_eq!(
        c.session.actions_sent(),
        sent + 1,
        "@ci goes out as a Talk line"
    );
}

#[test]
fn words_with_no_slash_are_said_aloud() {
    let mut c = testkit::offline_client();
    let sent = c.session.actions_sent();
    assert_eq!(c.chat_line("hello there"), Line::Acted(Ok(())));
    assert_eq!(c.session.actions_sent(), sent + 1);
    assert!(matches!(c.chat_line("   "), Line::Acted(Err(_))));
}

#[test]
fn a_soul_emote_is_what_a_slash_word_falls_back_to() {
    let mut c = testkit::offline_client();
    let Line::Offer { unclaimed, .. } = c.chat_line("/bow") else {
        panic!("no row has /bow: retail read it from the emote table");
    };
    assert_eq!(unclaimed, Action::SoulEmote("bow".into()));
}

/// What a row makes of its argument text, without a session to act on.
fn asked(line: &str) -> Option<Action> {
    let (name, args) = line
        .trim_start_matches('/')
        .split_once(char::is_whitespace)
        .unwrap_or((line.trim_start_matches('/'), ""));
    (action::command(name).expect("a row of its own").action)(args.trim())
}

#[test]
fn the_group_channels_are_the_ones_retail_speaks_on() {
    for (line, channel) in [
        ("/fellowship hello", channel::FELLOW),
        ("/fellows hello", channel::FELLOW),
        ("/f hello", channel::FELLOW),
        // Retail's /group and /g are the fellowship's, not the General room's.
        ("/group hello", channel::FELLOW),
        ("/g hello", channel::FELLOW),
        ("/party hello", channel::FELLOW),
        ("/vassals hello", channel::VASSALS),
        ("/vassal hello", channel::VASSALS),
        ("/v hello", channel::VASSALS),
        ("/patron hello", channel::PATRON),
        ("/p hello", channel::PATRON),
        ("/monarch hello", channel::MONARCH),
        ("/m hello", channel::MONARCH),
        ("/covassals hello", channel::CO_VASSALS),
        ("/co-vassals hello", channel::CO_VASSALS),
        ("/covassal hello", channel::CO_VASSALS),
        ("/c hello", channel::CO_VASSALS),
        ("/ab hello", channel::ALLEGIANCE_BROADCAST),
    ] {
        assert_eq!(
            asked(line),
            Some(Action::Channel {
                channel,
                text: "hello".into()
            }),
            "{line}"
        );
    }
    // Retail replaces the legacy /a when Turbine chat starts, so it speaks in
    // the allegiance's room and only /ab keeps the broadcast channel.
    assert_eq!(
        asked("/a hello"),
        Some(Action::Room {
            room: turbine::ALLEGIANCE,
            text: "hello".into()
        })
    );
}

#[test]
fn a_channel_with_nothing_to_say_is_refused_here() {
    for line in ["/fellowship", "/ab", "/a", "/c"] {
        assert_eq!(asked(line), None, "{line}");
    }
    let mut c = testkit::offline_client();
    let sent = c.session.actions_sent();
    assert!(matches!(c.chat_line("/fellowship"), Line::Acted(Err(_))));
    assert_eq!(c.session.actions_sent(), sent, "nothing went out");
}

#[test]
fn joining_and_leaving_read_the_first_word_only() {
    assert_eq!(
        asked("/join General"),
        Some(Action::Listen {
            room: turbine::GENERAL,
            on: true
        })
    );
    assert_eq!(
        asked("/leave soc please"),
        Some(Action::Listen {
            room: turbine::SOCIETY,
            on: false
        })
    );
    // Retail's own list, and nothing beside it.
    assert_eq!(asked("/join olthoi"), None);
    assert_eq!(asked("/join"), None);
}

#[test]
fn the_table_is_asked_by_name_whatever_the_case() {
    assert_eq!(action::command("LS").map(|c| c.names[0]), Some("lifestone"));
    assert_eq!(
        action::command(" mp ").map(|c| c.names[0]),
        Some("marketplace")
    );
    assert!(
        action::command("hometown").is_none(),
        "retail had no /hometown"
    );
}

/// Every `name == "..."` a `fn command` in a `.rhai` file under `dir` answers.
fn script_commands(dir: &Path, out: &mut BTreeSet<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            script_commands(&path, out);
        } else if path.extension().is_some_and(|e| e == "rhai") {
            let text = fs::read_to_string(&path).unwrap_or_default();
            for rest in text.split("name == \"").skip(1) {
                if let Some(name) = rest.split('"').next() {
                    out.insert(name.to_lowercase());
                }
            }
        }
    }
}

#[test]
fn no_script_command_takes_a_retail_name() {
    let mut names = BTreeSet::new();
    script_commands(&workspace().join("scripts"), &mut names);
    script_commands(&workspace().join("validate"), &mut names);
    assert!(
        !names.is_empty(),
        "no script commands were read; the suites answer /v_* and the examples /hi"
    );
    let retail: BTreeSet<&str> = RETAIL_NAMES.iter().copied().collect();
    let clashing: Vec<&String> = names
        .iter()
        .filter(|n| retail.contains(n.as_str()))
        .collect();
    assert!(
        clashing.is_empty(),
        "script commands with a retail name, which the table would answer first: {clashing:?}"
    );
}
