//! The allegiance family: retail's `@allegiance` subcommand tree, the two
//! top-level names that share its handlers (`@motd`, `@ah`), and the rooms a
//! character listens to (`@join`, `@leave`, `@allegiance chat on|off`), which
//! are a character option each. Every word a command of this family reads is
//! matched case-insensitively, as retail's `_stricmp` dispatcher does.

use ac_net::messages::{action, channel, turbine};
use ac_net::wire::Writer;

use super::{Action, HouseAccess, Lock, Manage, Outcome, Refused};
use crate::{options, Client};

/// What the one put out of allegiance chat is told when the line named no
/// reason (retail's own default, FUN_00576a80).
const NO_REASON: &str = "No reason given.";

/// Send one `@allegiance` subcommand. Nothing is held back here: the server
/// answers each of these itself, refusing the ranks it wants (ACE
/// `Player_Allegiance.cs:1478-1515`).
pub(super) fn manage(c: &mut Client, what: &Manage) -> Outcome {
    let (op, body) = payload(what);
    c.session.send_action(op, &body);
    Ok(())
}

/// Hear a chat room, or stop hearing it. One character option each, which
/// retail sets only when the bit changes (FUN_005d4570); ACE answers "You
/// have entered the X channel." (`Player_Networking.cs:162-210`).
pub(super) fn listen(c: &mut Client, room: u32, on: bool) -> Outcome {
    let Some(o) = options::listen_option(room) else {
        return Err(Refused::ours("no chat channel of that name"));
    };
    if c.option_enabled(o) != on {
        c.set_option(o, on);
    }
    Ok(())
}

/// The game action one subcommand sends and its body, in retail's own order,
/// which is the order ACE reads (name then level for an officer, level then
/// title for a title: `GameActionSetAllegianceOfficer.cs:10-11`,
/// `GameActionSetAllegianceOfficerTitle.cs:10-11`).
fn payload(what: &Manage) -> (u32, Vec<u8>) {
    let named = |op, name: &str| (op, body(|w| w.string16(name)));
    match what {
        Manage::Boot { name, account } => (
            action::BREAK_ALLEGIANCE_BOOT,
            body(|w| w.string16(name).u32(u32::from(*account))),
        ),
        Manage::Info(name) => named(action::ALLEGIANCE_INFO_REQUEST, name),
        Manage::ChatBoot { name, reason } => (
            action::ALLEGIANCE_CHAT_BOOT,
            body(|w| w.string16(name).string16(reason)),
        ),
        Manage::Gag { name, gagged } => (
            action::ALLEGIANCE_CHAT_GAG,
            body(|w| w.string16(name).u32(u32::from(*gagged))),
        ),
        Manage::BanList => (action::LIST_ALLEGIANCE_BANS, Vec::new()),
        Manage::Ban { name, banned } => named(
            if *banned {
                action::ADD_ALLEGIANCE_BAN
            } else {
                action::REMOVE_ALLEGIANCE_BAN
            },
            name,
        ),
        Manage::OfficerList => (action::LIST_ALLEGIANCE_OFFICERS, Vec::new()),
        Manage::OfficerSet { name, level } => (
            action::SET_ALLEGIANCE_OFFICER,
            body(|w| w.string16(name).u32(*level)),
        ),
        Manage::OfficerRemove(name) => named(action::REMOVE_ALLEGIANCE_OFFICER, name),
        Manage::OfficerClear => (action::CLEAR_ALLEGIANCE_OFFICERS, Vec::new()),
        Manage::TitleList => (action::LIST_ALLEGIANCE_OFFICER_TITLES, Vec::new()),
        Manage::TitleSet { level, title } => (
            action::SET_ALLEGIANCE_OFFICER_TITLE,
            body(|w| w.u32(*level).string16(title)),
        ),
        Manage::TitleClear => (action::CLEAR_ALLEGIANCE_OFFICER_TITLES, Vec::new()),
        Manage::Motd => (action::QUERY_MOTD, Vec::new()),
        Manage::SetMotd(text) => named(action::SET_MOTD, text),
        Manage::ClearMotd => (action::CLEAR_MOTD, Vec::new()),
        Manage::Name => (action::QUERY_ALLEGIANCE_NAME, Vec::new()),
        Manage::SetName(text) => named(action::SET_ALLEGIANCE_NAME, text),
        Manage::ClearName => (action::CLEAR_ALLEGIANCE_NAME, Vec::new()),
        Manage::Lock(lock) => (
            action::DO_ALLEGIANCE_LOCK_ACTION,
            body(|w| w.u32(lock_code(lock))),
        ),
        Manage::ApproveVassal(name) => named(action::SET_ALLEGIANCE_APPROVED_VASSAL, name),
        Manage::House(access) => (
            action::DO_ALLEGIANCE_HOUSE_ACTION,
            body(|w| w.u32(house_code(access))),
        ),
    }
}

/// One payload, written by what the caller puts in it.
fn body(write: impl FnOnce(&mut Writer) -> &mut Writer) -> Vec<u8> {
    let mut w = Writer::new();
    write(&mut w);
    w.finish()
}

/// ACE `AllegianceLockAction.cs:5-11`.
fn lock_code(lock: &Lock) -> u32 {
    match lock {
        Lock::Off => 1,
        Lock::On => 2,
        Lock::Toggle => 3,
        Lock::Check => 4,
        Lock::Approved => 5,
        Lock::ClearApproved => 6,
    }
}

/// ACE `AllegianceHouseAction.cs:8-14`.
fn house_code(access: &HouseAccess) -> u32 {
    match access {
        HouseAccess::Check => 1,
        HouseAccess::GuestOpen => 2,
        HouseAccess::GuestClose => 3,
        HouseAccess::StorageOpen => 4,
        HouseAccess::StorageClose => 5,
    }
}

/// What a `/allegiance ...` line asks for, and `None` where retail's own
/// handler answered instead of sending (its dispatcher's "Please see @help
/// Allegiance", or one of the "Please specify ..." lines). Three subcommands
/// are not allegiance actions at all: `hometown` recalls, `broadcast` speaks
/// on the broadcast channel and `chat on|off` is the listening option.
pub(super) fn subcommand(args: &str) -> Option<Action> {
    let (sub, rest) = word(args);
    let manage = match sub.as_str() {
        "boot" => boot(rest)?,
        "info" => Manage::Info(some_name(rest)?),
        "chat" | "ch" => return chat(rest),
        "broadcast" | "br" => return broadcast(rest),
        "ban" => ban(rest)?,
        "officer" => officer(rest)?,
        "title" => title(rest)?,
        // Retail ignores whatever follows here (its `@ah` handler does not).
        "hometown" | "ho" => return Some(Action::RecallHometown),
        "motd" => motd(rest)?,
        "name" => name(rest)?,
        "lock" => return lock(rest),
        "house" => house(rest)?,
        _ => return None,
    };
    Some(Action::Allegiance(manage))
}

/// What `@motd` alone means: retail gives it the handler `@allegiance motd`
/// has (FUN_00577f20).
pub(super) fn motd(args: &str) -> Option<Manage> {
    let (verb, rest) = word(args);
    match verb.as_str() {
        "" => Some(Manage::Motd),
        // Retail sends the words as they stand, an empty text included; only
        // `clear` clears the message of the day.
        "set" => Some(Manage::SetMotd(words(rest))),
        "clear" => Some(Manage::ClearMotd),
        _ => None,
    }
}

/// The room `@join` and `@leave` name, from the first word of the line and
/// no other (retail takes Allegiance, General, Trade, LFG, Roleplay and
/// Society, and not Olthoi: LAB_005702e0).
pub(super) fn room_wanted(args: &str) -> Option<u32> {
    match word(args).0.as_str() {
        "allegiance" => Some(turbine::ALLEGIANCE),
        "general" => Some(turbine::GENERAL),
        "trade" => Some(turbine::TRADE),
        "lfg" => Some(turbine::LFG),
        "roleplay" => Some(turbine::ROLEPLAY),
        "society" | "soc" => Some(turbine::SOCIETY),
        _ => None,
    }
}

/// `boot [-account] <name>`: retail takes the flag out of the text wherever
/// it stands and boots what is left (FUN_0057bcc0).
fn boot(rest: &str) -> Option<Manage> {
    let account = |w: &str| w.eq_ignore_ascii_case("-account");
    let kept: Vec<&str> = rest.split_whitespace().filter(|w| !account(w)).collect();
    let name = without_plus(&kept.join(" "));
    (!name.is_empty()).then_some(Manage::Boot {
        name,
        account: rest.split_whitespace().any(account),
    })
}

fn chat(rest: &str) -> Option<Action> {
    let (verb, rest) = word(rest);
    match verb.as_str() {
        "on" | "off" => Some(Action::Listen {
            room: turbine::ALLEGIANCE,
            on: verb == "on",
        }),
        // `kick <name>[, <reason>]`: the name stands before the comma and the
        // reason after it. Retail checks neither for emptiness; an empty name
        // earns nothing but the server's "not found", so it is refused here
        // with the rest of them.
        "kick" => {
            let text = words(rest);
            let (name, reason) = match text.split_once(',') {
                Some((n, r)) => (n.to_string(), r.trim().to_string()),
                None => (text, String::new()),
            };
            (!name.is_empty()).then(|| {
                Action::Allegiance(Manage::ChatBoot {
                    reason: if reason.is_empty() {
                        NO_REASON.to_string()
                    } else {
                        reason
                    },
                    name,
                })
            })
        }
        // A gagged name is joined and no more: retail strips no '+' here.
        "gag" | "ungag" => {
            let name = words(rest);
            (!name.is_empty()).then(|| {
                Action::Allegiance(Manage::Gag {
                    name,
                    gagged: verb == "gag",
                })
            })
        }
        _ => None,
    }
}

fn broadcast(rest: &str) -> Option<Action> {
    let text = words(rest);
    (!text.is_empty()).then_some(Action::Channel {
        channel: channel::ALLEGIANCE_BROADCAST,
        text,
    })
}

/// `ban list | ban add <name> | ban remove <name>`: retail reads the name
/// before the verb, so a `ban` with no name at all is refused first
/// (FUN_00577070).
fn ban(rest: &str) -> Option<Manage> {
    let (verb, rest) = word(rest);
    if verb == "list" {
        return Some(Manage::BanList);
    }
    let name = some_name(rest)?;
    match verb.as_str() {
        "add" => Some(Manage::Ban { name, banned: true }),
        "remove" => Some(Manage::Ban {
            name,
            banned: false,
        }),
        _ => None,
    }
}

fn officer(rest: &str) -> Option<Manage> {
    let (verb, rest) = word(rest);
    match verb.as_str() {
        "" | "list" => Some(Manage::OfficerList),
        "clear" => Some(Manage::OfficerClear),
        "remove" => Some(Manage::OfficerRemove(some_name(rest)?)),
        "set" | "add" => {
            let (level, rest) = word(rest);
            Some(Manage::OfficerSet {
                level: officer_level(&level)?,
                name: some_name(rest)?,
            })
        }
        _ => None,
    }
}

fn title(rest: &str) -> Option<Manage> {
    let (verb, rest) = word(rest);
    match verb.as_str() {
        "" | "list" => Some(Manage::TitleList),
        "clear" => Some(Manage::TitleClear),
        // Retail makes no empty check on the title itself.
        "set" => {
            let (level, rest) = word(rest);
            Some(Manage::TitleSet {
                level: officer_level(&level)?,
                title: without_plus(&words(rest)),
            })
        }
        _ => None,
    }
}

fn name(rest: &str) -> Option<Manage> {
    let (verb, rest) = word(rest);
    match verb.as_str() {
        "" => Some(Manage::Name),
        // As with the motd, retail sends the words as they stand.
        "set" => Some(Manage::SetName(words(rest))),
        "clear" => Some(Manage::ClearName),
        _ => None,
    }
}

fn lock(rest: &str) -> Option<Action> {
    let (verb, rest) = word(rest);
    let lock = match verb.as_str() {
        "" | "check" => Lock::Check,
        "off" => Lock::Off,
        "on" => Lock::On,
        "toggle" => Lock::Toggle,
        // `bypass` alone lists the approved vassals, `bypass clear` empties
        // the list, and any other word is a vassal to approve.
        "bypass" => {
            return match word(rest).0.as_str() {
                "" => Some(Action::Allegiance(Manage::Lock(Lock::Approved))),
                "clear" => Some(Action::Allegiance(Manage::Lock(Lock::ClearApproved))),
                _ => Some(Action::Allegiance(Manage::ApproveVassal(some_name(rest)?))),
            }
        }
        _ => return None,
    };
    Some(Action::Allegiance(Manage::Lock(lock)))
}

/// `house [guest|storage open|close]`: retail asks what the access is with
/// fewer than two words and reads no more than two (FUN_0056fd40).
fn house(rest: &str) -> Option<Manage> {
    let said: Vec<String> = rest
        .split_whitespace()
        .map(|w| w.to_ascii_lowercase())
        .collect();
    let access = match said.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] | [_] => HouseAccess::Check,
        ["guest", "open", ..] => HouseAccess::GuestOpen,
        ["guest", "close", ..] => HouseAccess::GuestClose,
        ["storage", "open", ..] => HouseAccess::StorageOpen,
        ["storage", "close", ..] => HouseAccess::StorageClose,
        _ => return None,
    };
    Some(Manage::House(access))
}

/// The officer level a line names: retail reads it with `strtol` and wants
/// 1 to 3, else it says so and sends nothing.
fn officer_level(word: &str) -> Option<u32> {
    word.parse::<u32>().ok().filter(|l| (1..=3).contains(l))
}

/// The first word, lowercased for retail's case-insensitive compares, and
/// what follows it.
fn word(args: &str) -> (String, &str) {
    let (first, rest) = args
        .trim_start()
        .split_once(char::is_whitespace)
        .unwrap_or((args.trim(), ""));
    (first.to_ascii_lowercase(), rest.trim())
}

/// Retail joins the rest of the arguments with single spaces (FUN_00576890),
/// which leaves one space between the words whatever was typed.
fn words(rest: &str) -> String {
    rest.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A member's name as retail reads one: the words joined, then a leading '+'
/// dropped, the way an admin's name is shown (FUN_005769e0:11-19). ACE drops
/// one too when it looks a name up (`PlayerManager.cs:521`).
fn without_plus(joined: &str) -> String {
    joined.strip_prefix('+').unwrap_or(joined).to_string()
}

fn some_name(rest: &str) -> Option<String> {
    let name = without_plus(&words(rest));
    (!name.is_empty()).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    /// The action and body one subcommand puts on the wire.
    fn sent(line: &str) -> (u32, Vec<u8>) {
        match subcommand(line) {
            Some(Action::Allegiance(what)) => payload(&what),
            other => panic!("{line:?} is {other:?}, not an allegiance action"),
        }
    }

    fn string16(s: &str) -> Vec<u8> {
        body(|w| w.string16(s))
    }

    #[test]
    fn a_subcommand_that_asks_carries_no_payload() {
        for (line, op) in [
            ("ban list", action::LIST_ALLEGIANCE_BANS),
            ("officer", action::LIST_ALLEGIANCE_OFFICERS),
            ("officer list", action::LIST_ALLEGIANCE_OFFICERS),
            ("officer clear", action::CLEAR_ALLEGIANCE_OFFICERS),
            ("title", action::LIST_ALLEGIANCE_OFFICER_TITLES),
            ("title clear", action::CLEAR_ALLEGIANCE_OFFICER_TITLES),
            ("motd", action::QUERY_MOTD),
            ("motd clear", action::CLEAR_MOTD),
            ("name", action::QUERY_ALLEGIANCE_NAME),
            ("name clear", action::CLEAR_ALLEGIANCE_NAME),
        ] {
            assert_eq!(sent(line), (op, Vec::new()), "{line}");
        }
    }

    #[test]
    fn a_name_is_joined_trimmed_and_loses_its_plus() {
        assert_eq!(
            sent("info   +Verity "),
            (action::ALLEGIANCE_INFO_REQUEST, string16("Verity"))
        );
        assert_eq!(
            sent("ban add  Lady  Jane "),
            (action::ADD_ALLEGIANCE_BAN, string16("Lady Jane"))
        );
        assert_eq!(
            sent("ban remove Jane"),
            (action::REMOVE_ALLEGIANCE_BAN, string16("Jane"))
        );
    }

    #[test]
    fn an_officer_is_named_before_the_level_on_the_wire() {
        let mut want = string16("Verity");
        want.extend(2u32.to_le_bytes());
        assert_eq!(
            sent("officer set 2 Verity"),
            (action::SET_ALLEGIANCE_OFFICER, want)
        );
        assert_eq!(
            sent("OFFICER ADD 2 Verity").0,
            action::SET_ALLEGIANCE_OFFICER
        );
        assert_eq!(
            sent("officer remove Verity"),
            (action::REMOVE_ALLEGIANCE_OFFICER, string16("Verity"))
        );
    }

    #[test]
    fn a_title_is_levelled_before_it_is_named() {
        let mut want = 3u32.to_le_bytes().to_vec();
        want.extend(string16("Castellan of Nowhere"));
        assert_eq!(
            sent("title set 3 Castellan of Nowhere"),
            (action::SET_ALLEGIANCE_OFFICER_TITLE, want)
        );
    }

    #[test]
    fn only_officer_levels_one_to_three_are_sent() {
        for line in [
            "officer set 0 Verity",
            "officer set 4 Verity",
            "officer set two Verity",
            "title set x y",
        ] {
            assert_eq!(subcommand(line), None, "{line}");
        }
    }

    #[test]
    fn booting_an_account_sets_the_flag_after_the_name() {
        let mut want = string16("Verity");
        want.extend(1u32.to_le_bytes());
        assert_eq!(
            sent("boot -account Verity"),
            (action::BREAK_ALLEGIANCE_BOOT, want.clone())
        );
        assert_eq!(
            sent("boot Verity -account").1,
            want,
            "the flag stands where it likes"
        );
        let mut plain = string16("Verity");
        plain.extend(0u32.to_le_bytes());
        assert_eq!(sent("boot Verity"), (action::BREAK_ALLEGIANCE_BOOT, plain));
        assert_eq!(subcommand("boot -account"), None, "no name, nothing sent");
    }

    #[test]
    fn a_kick_names_its_reason_after_the_comma() {
        let mut want = string16("Verity");
        want.extend(string16("too loud"));
        assert_eq!(
            sent("chat kick Verity, too loud"),
            (action::ALLEGIANCE_CHAT_BOOT, want)
        );
        let mut default = string16("Verity");
        default.extend(string16(NO_REASON));
        assert_eq!(
            sent("chat kick Verity"),
            (action::ALLEGIANCE_CHAT_BOOT, default)
        );
        assert_eq!(
            subcommand("chat kick"),
            None,
            "with no name, nothing goes out"
        );
    }

    #[test]
    fn a_gag_keeps_the_name_it_was_given() {
        let mut want = string16("+Verity");
        want.extend(1u32.to_le_bytes());
        assert_eq!(
            sent("chat gag +Verity"),
            (action::ALLEGIANCE_CHAT_GAG, want)
        );
        let mut off = string16("Verity");
        off.extend(0u32.to_le_bytes());
        assert_eq!(
            sent("chat ungag Verity"),
            (action::ALLEGIANCE_CHAT_GAG, off)
        );
    }

    #[test]
    fn the_motd_and_the_name_are_set_as_they_stand() {
        assert_eq!(
            sent("motd set  be  good "),
            (action::SET_MOTD, string16("be good"))
        );
        assert_eq!(
            sent("name set The Hand"),
            (action::SET_ALLEGIANCE_NAME, string16("The Hand"))
        );
        // Retail makes no empty check on either, so both go out as typed.
        assert_eq!(sent("motd set"), (action::SET_MOTD, string16("")));
    }

    #[test]
    fn the_lock_words_are_the_servers_own_numbering() {
        for (line, code) in [
            ("lock", 4u32),
            ("lock check", 4),
            ("lock off", 1),
            ("lock on", 2),
            ("lock toggle", 3),
            ("lock bypass", 5),
            ("lock bypass clear", 6),
        ] {
            assert_eq!(
                sent(line),
                (
                    action::DO_ALLEGIANCE_LOCK_ACTION,
                    code.to_le_bytes().to_vec()
                ),
                "{line}"
            );
        }
        assert_eq!(
            sent("lock bypass +Verity"),
            (action::SET_ALLEGIANCE_APPROVED_VASSAL, string16("Verity"))
        );
    }

    #[test]
    fn the_house_words_are_the_servers_own_numbering() {
        for (line, code) in [
            ("house", 1u32),
            ("house guest", 1),
            ("house guest open", 2),
            ("house guest close", 3),
            ("house storage open", 4),
            ("house storage close", 5),
        ] {
            assert_eq!(
                sent(line),
                (
                    action::DO_ALLEGIANCE_HOUSE_ACTION,
                    code.to_le_bytes().to_vec()
                ),
                "{line}"
            );
        }
        assert_eq!(subcommand("house guest sideways"), None);
    }

    #[test]
    fn three_subcommands_are_not_allegiance_actions_at_all() {
        assert_eq!(subcommand("hometown"), Some(Action::RecallHometown));
        assert_eq!(subcommand("ho anything"), Some(Action::RecallHometown));
        assert_eq!(
            subcommand("br hello all"),
            Some(Action::Channel {
                channel: channel::ALLEGIANCE_BROADCAST,
                text: "hello all".into()
            })
        );
        assert_eq!(
            subcommand("chat off"),
            Some(Action::Listen {
                room: turbine::ALLEGIANCE,
                on: false
            })
        );
    }

    #[test]
    fn a_word_retail_never_dispatched_is_refused() {
        for line in [
            "",
            "officers",
            "kick Verity",
            "chat sideways",
            "motd please",
        ] {
            assert_eq!(subcommand(line), None, "{line}");
        }
    }

    #[test]
    fn a_managing_line_goes_out_as_one_action() {
        let mut c = testkit::offline_client();
        let sent = c.session.actions_sent();
        assert_eq!(manage(&mut c, &Manage::OfficerList), Ok(()));
        assert_eq!(c.session.actions_sent(), sent + 1);
    }

    #[test]
    fn the_rooms_are_the_six_retail_joins() {
        assert_eq!(room_wanted("Allegiance"), Some(turbine::ALLEGIANCE));
        assert_eq!(room_wanted("general"), Some(turbine::GENERAL));
        assert_eq!(room_wanted("trade"), Some(turbine::TRADE));
        assert_eq!(room_wanted("lfg"), Some(turbine::LFG));
        assert_eq!(room_wanted("roleplay"), Some(turbine::ROLEPLAY));
        assert_eq!(room_wanted("soc"), Some(turbine::SOCIETY));
        assert_eq!(room_wanted("society and more"), Some(turbine::SOCIETY));
        // Retail's list has no Olthoi room, and no room at all is not one.
        assert_eq!(room_wanted("olthoi"), None);
        assert_eq!(room_wanted(""), None);
    }

    #[test]
    fn a_room_is_joined_once_and_left_once() {
        let mut c = testkit::offline_client();
        let sent = c.session.actions_sent();
        assert_eq!(listen(&mut c, turbine::GENERAL, true), Ok(()));
        assert_eq!(c.session.actions_sent(), sent + 1, "the option went out");
        assert_eq!(listen(&mut c, turbine::GENERAL, true), Ok(()));
        assert_eq!(
            c.session.actions_sent(),
            sent + 1,
            "a room already heard is not joined again"
        );
        assert_eq!(listen(&mut c, turbine::GENERAL, false), Ok(()));
        assert_eq!(c.session.actions_sent(), sent + 2, "leaving it went out");
    }
}
