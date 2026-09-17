//! The allegiance family: the rooms a character listens to (`@join`,
//! `@leave`), which are a character option each. Every word a command of this
//! family reads is matched case-insensitively, as retail's `_stricmp`
//! dispatcher does.

use ac_net::messages::turbine;

use super::{Outcome, Refused};
use crate::{options, Client};

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

/// The first word, lowercased for retail's case-insensitive compares, and
/// what follows it.
fn word(args: &str) -> (String, &str) {
    let (first, rest) = args
        .trim_start()
        .split_once(char::is_whitespace)
        .unwrap_or((args.trim(), ""));
    (first.to_ascii_lowercase(), rest.trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    #[test]
    fn the_rooms_are_the_six_retail_joins() {
        assert_eq!(room_wanted("Allegiance"), Some(turbine::ALLEGIANCE));
        assert_eq!(room_wanted("general"), Some(turbine::GENERAL));
        assert_eq!(room_wanted("trade"), Some(turbine::TRADE));
        assert_eq!(room_wanted("lfg"), Some(turbine::LFG));
        assert_eq!(room_wanted("roleplay"), Some(turbine::ROLEPLAY));
        assert_eq!(room_wanted("soc"), Some(turbine::SOCIETY));
        assert_eq!(room_wanted("society and more"), Some(turbine::SOCIETY));
        // Retail's list has no Olthoi room and no room at all is not one.
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
