use ac_net::messages::{squelch as category, turbine};

use super::*;
use crate::action::{command, Line};
use crate::testkit;

/// What the retail table makes of `/name args`, before anything is sent.
fn parsed(name: &str, args: &str) -> Option<Action> {
    let cmd = command(name).unwrap_or_else(|| panic!("no row answers /{name}"));
    (cmd.action)(args)
}

fn room(room: u32, text: &str) -> Option<Action> {
    Some(Action::Room {
        room,
        text: text.to_string(),
    })
}

/// The chat lines a client has to show, newest last.
fn said(c: &mut Client) -> Vec<String> {
    c.drain_events()
        .into_iter()
        .filter_map(|e| match e {
            Event::Chat { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

#[test]
fn a_general_line_is_what_was_typed_after_the_command() {
    assert_eq!(parsed("cg", "hi there"), room(turbine::GENERAL, "hi there"));
    assert_eq!(parsed("general", "hi there"), parsed("cg", "hi there"));
    // The words are taken as they stand, slashes, commas and all.
    assert_eq!(
        parsed("cg", "wts a/b, 10k"),
        room(turbine::GENERAL, "wts a/b, 10k")
    );
    // Nothing to say is the usage line, not an empty room line.
    assert_eq!(parsed("cg", ""), None);
}

#[test]
fn each_room_has_its_own_names() {
    assert_eq!(parsed("guild", "x"), room(turbine::ALLEGIANCE, "x"));
    assert_eq!(parsed("gu", "x"), room(turbine::ALLEGIANCE, "x"));
    assert_eq!(parsed("ct", "x"), room(turbine::TRADE, "x"));
    assert_eq!(parsed("trade", "x"), room(turbine::TRADE, "x"));
    assert_eq!(parsed("clfg", "x"), room(turbine::LFG, "x"));
    assert_eq!(parsed("crp", "x"), room(turbine::ROLEPLAY, "x"));
    assert_eq!(parsed("soc", "x"), room(turbine::SOCIETY, "x"));
    assert_eq!(parsed("o", "x"), room(turbine::OLTHOI, "x"));
    // `/rp` is the reply retail registered, not the Roleplay room.
    assert_eq!(parsed("rp", "x"), Some(Action::Reply("x".into())));
}

#[test]
fn a_general_line_goes_out_as_a_turbine_message() {
    let mut c = testkit::offline_client();
    assert_eq!(c.chat_line("/cg hi there"), Line::Acted(Ok(())));
    let sent = c
        .session
        .queued()
        .iter()
        .find(|(_, m)| m.starts_with(&ac_net::messages::opcode::TURBINE_CHAT.to_le_bytes()))
        .map(|(_, m)| m.clone())
        .expect("no TurbineChat (0xF7DE) message went out");
    // The room, then the text as UTF-16 inside the blob.
    assert!(sent.windows(4).any(|w| w == turbine::GENERAL.to_le_bytes()));
    let text: Vec<u8> = "hi there"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    assert!(sent.windows(text.len()).any(|w| w == text));
}

#[test]
fn a_room_we_have_none_of_is_refused_here() {
    let mut c = testkit::offline_client();
    // No allegiance and no society until the server names their rooms.
    assert!(matches!(c.chat_line("/gu hello"), Line::Acted(Err(_))));
    assert!(matches!(c.chat_line("/soc hello"), Line::Acted(Err(_))));
    assert_eq!(
        said(&mut c),
        [
            "You are not in an allegiance.",
            "You do not belong to a society."
        ]
    );
}

#[test]
fn the_speech_switches_squelch_and_unsquelch() {
    // "@chat on" is speech we want, so the squelch comes off; "@notell
    // on" is tells we do not, so it goes on.
    let filter = |kind, on| Some(Action::Filter { kind, on });
    assert_eq!(parsed("chat", "on"), filter(category::SPEECH, false));
    assert_eq!(parsed("chat", "OFF"), filter(category::SPEECH, true));
    assert_eq!(parsed("notell", "on"), filter(category::TELL, true));
    assert_eq!(parsed("notell", "off"), filter(category::TELL, false));
    // Retail read any other single word as on for @notell alone.
    assert_eq!(parsed("notell", "yes"), filter(category::TELL, true));
    assert_eq!(parsed("chat", "yes"), None);
    // One word, never two, and never none.
    assert_eq!(parsed("notell", "on off"), None);
    assert_eq!(parsed("chat", ""), None);
}

#[test]
fn a_squelch_names_a_person_a_category_or_the_last_teller() {
    let squelch = |who: Option<&str>, account, kind, on| {
        Some(Action::Squelch {
            who: who.map(str::to_string),
            account,
            kind,
            on,
        })
    };
    assert_eq!(
        parsed("squelch", "Oswald"),
        squelch(Some("Oswald"), false, category::ALL, true)
    );
    // A name of several words is rejoined.
    assert_eq!(
        parsed("squelch", "Ivory Guardian"),
        squelch(Some("Ivory Guardian"), false, category::ALL, true)
    );
    assert_eq!(
        parsed("squelch", "-account -tell Oswald"),
        squelch(Some("Oswald"), true, category::TELL, true)
    );
    // The last category named wins, as retail's parser left it.
    assert_eq!(
        parsed("squelch", "-tell -combat Oswald"),
        squelch(Some("Oswald"), false, 0x06, true)
    );
    // `-reply` is the last teller, and the words after it are ignored.
    assert_eq!(
        parsed("unsquelch", "-reply"),
        squelch(None, false, category::ALL, false)
    );
    assert_eq!(
        parsed("squelch", "-reply -account Oswald"),
        squelch(None, true, category::ALL, true)
    );
    // No arguments lists them; a category that is not one is the usage.
    assert_eq!(
        parsed("squelch", ""),
        Some(Action::ShowSquelches { global: false })
    );
    assert_eq!(parsed("squelch", "-sideways Oswald"), None);
    assert_eq!(parsed("squelch", "-account"), None);
}

#[test]
fn a_filter_names_one_category_and_never_a_person() {
    assert_eq!(
        parsed("filter", "-spellcasting"),
        Some(Action::Filter {
            kind: 0x11,
            on: true
        })
    );
    assert_eq!(
        parsed("unfilter", "-all"),
        Some(Action::Filter {
            kind: category::ALL,
            on: false
        })
    );
    assert_eq!(
        parsed("filter", ""),
        Some(Action::ShowSquelches { global: true })
    );
    // The dash is retail's own check, before the category is looked up.
    assert_eq!(parsed("filter", "spellcasting"), None);
    assert_eq!(parsed("filter", "-tell Oswald"), None);
    assert_eq!(parsed("filter", "-reply"), None);
}

#[test]
fn a_channel_command_wants_a_channel_retail_knew() {
    use ac_net::messages::channel;
    assert_eq!(
        parsed("on", "help"),
        Some(Action::ChannelJoin(channel::HELP))
    );
    assert_eq!(
        parsed("off", "av2"),
        Some(Action::ChannelLeave(channel::ADVOCATE2))
    );
    assert_eq!(
        parsed("clist", "sentinel"),
        Some(Action::ChannelList(channel::SENTINEL))
    );
    assert_eq!(parsed("on", "general"), None);
    assert_eq!(parsed("on", ""), None);
    assert_eq!(parsed("index", "anything"), Some(Action::ChannelIndex));
}

#[test]
fn a_reply_wants_somebody_to_have_told_us_first() {
    let mut c = testkit::offline_client();
    assert!(matches!(c.chat_line("/r hello"), Line::Acted(Err(_))));
    assert!(matches!(c.chat_line("/retell hello"), Line::Acted(Err(_))));
    // A creature's tell is not a reply target; a player's is.
    c.hear_tell(0x8000_0001, "Drudge Skulker");
    assert!(matches!(c.chat_line("/r hello"), Line::Acted(Err(_))));
    c.hear_tell(0x5000_0002, "Verity");
    let sent = c.session.actions_sent();
    assert_eq!(c.chat_line("/reply hello"), Line::Acted(Ok(())));
    assert_eq!(c.session.actions_sent(), sent + 1);
    assert_eq!(c.last_teller, Some((0x5000_0002, "Verity".to_string())));
}

#[test]
fn a_retell_tells_whoever_we_told_last() {
    let mut c = testkit::offline_client();
    assert_eq!(c.chat_line("/tell Verity, hello"), Line::Acted(Ok(())));
    assert_eq!(c.last_told.as_deref(), Some("Verity"));
    let sent = c.session.actions_sent();
    assert_eq!(c.chat_line("/rt again"), Line::Acted(Ok(())));
    assert_eq!(c.session.actions_sent(), sent + 1, "the retell went out");
    // A reply is not a tell, so it does not move the retell target.
    c.hear_tell(0x5000_0002, "Fletch");
    assert_eq!(c.chat_line("/r hello"), Line::Acted(Ok(())));
    assert_eq!(c.last_told.as_deref(), Some("Verity"));
}

#[test]
fn the_squelch_lists_are_the_ones_the_server_sent() {
    let mut c = testkit::offline_client();
    assert_eq!(c.chat_line("/messagetypes"), Line::Acted(Ok(())));
    assert_eq!(
        said(&mut c),
        [
            "Squelch channels are as follows:".to_string(),
            "  Speech, Tell, Combat, Magic, Emote, Appraisal, Spellcasting, Allegiance, \
             Fellowship, Combat_Enemy, Combat_Self, Recall, Craft, Salvaging"
                .to_string(),
        ]
    );
    // Nobody squelched and nothing filtered, until SetSquelchDB says so.
    assert_eq!(c.chat_line("/squelch"), Line::Acted(Ok(())));
    assert_eq!(said(&mut c).last().map(String::as_str), Some("none"));
    assert_eq!(c.chat_line("/filter"), Line::Acted(Ok(())));
    assert_eq!(said(&mut c).last().map(String::as_str), Some("none"));
    c.world.squelches.global = 1 << category::TELL;
    c.world.squelches.characters = vec![ac_world::social::Squelch {
        guid: 0x5000_0002,
        name: "Oswald".into(),
        mask: u32::MAX,
        account: true,
    }];
    assert_eq!(c.chat_line("/unfilter"), Line::Acted(Ok(())));
    assert_eq!(said(&mut c).last().map(String::as_str), Some("Tell"));
    assert_eq!(c.chat_line("/unsquelch"), Line::Acted(Ok(())));
    assert_eq!(
        said(&mut c).last().map(String::as_str),
        Some("  Name: Oswald (account) All message types")
    );
}

#[test]
fn the_chat_window_commands_are_the_front_ends() {
    assert_eq!(
        parsed("log", "AClog.txt"),
        Some(Action::ChatToFile(Some("AClog.txt".into())))
    );
    assert_eq!(parsed("log", ""), Some(Action::ChatToFile(None)));
    assert_eq!(parsed("clear", ""), Some(Action::ChatClear { all: false }));
    assert_eq!(
        parsed("clear", "ALL"),
        Some(Action::ChatClear { all: true })
    );
    // Retail read the first word and ignored the rest.
    assert_eq!(
        parsed("clear", "all of it"),
        Some(Action::ChatClear { all: true })
    );
    assert_eq!(parsed("title", ""), None);
    let mut c = testkit::offline_client();
    let sent = c.session.actions_sent();
    assert_eq!(c.chat_line("/clear all"), Line::Acted(Ok(())));
    assert!(matches!(
        c.drain_events().first(),
        Some(Event::ChatClear { all: true })
    ));
    // One chat window with tabs and no popups: retail's own refusal.
    assert!(matches!(c.chat_line("/title Tells"), Line::Acted(Err(_))));
    assert_eq!(c.session.actions_sent(), sent, "nothing reached the server");
}
