//! The chat family: what the character says and to whom -- aloud, to one
//! player, in a Turbine room or on a group channel -- the away message, and
//! the commands the server itself reads off a Talk line.

use ac_net::messages::action;
use ac_net::wire::Writer;

use super::{Outcome, Refused};
use crate::Client;

pub(super) fn say(c: &mut Client, text: &str) -> Outcome {
    let text = text.trim();
    if text.is_empty() {
        return Err(Refused::ours("nothing to say"));
    }
    c.say(text);
    Ok(())
}

pub(super) fn tell(c: &mut Client, who: &str, text: &str) -> Outcome {
    let (who, text) = (who.trim(), text.trim());
    if who.is_empty() || text.is_empty() {
        return Err(Refused::ours("a tell wants a name and something to say"));
    }
    let mut w = Writer::new();
    w.string16(text).string16(who);
    c.session.send_action(action::TELL, &w.finish());
    Ok(())
}

pub(super) fn emote(c: &mut Client, text: &str) -> Outcome {
    let text = text.trim();
    if text.is_empty() {
        return Err(Refused::ours("an emote wants something to act out"));
    }
    let mut w = Writer::new();
    w.string16(text);
    c.session.send_action(action::EMOTE, &w.finish());
    Ok(())
}

pub(super) fn soul_emote(c: &mut Client, words: &str) -> Outcome {
    if c.emote(words) {
        Ok(())
    } else {
        Err(Refused::ours(format!("no emote called {words:?}")))
    }
}

pub(super) fn room(c: &mut Client, room: u32, text: &str) -> Outcome {
    if c.turbine_say(room, text) {
        Ok(())
    } else {
        Err(Refused::ours("no room to say that in"))
    }
}

pub(super) fn channel(c: &mut Client, channel: u32, text: &str) -> Outcome {
    if text.trim().is_empty() {
        return Err(Refused::ours("nothing to say"));
    }
    c.chat_channel(channel, text);
    Ok(())
}

/// Away from keyboard: the message others get when they tell us (SetAfkMessage
/// 0x0010), then the state itself (SetAfkMode 0x000F).
pub(super) fn afk(c: &mut Client, away: bool, message: &str) -> Outcome {
    let message = message.trim();
    if away && !message.is_empty() {
        let mut w = Writer::new();
        w.string16(message);
        c.session.send_action(action::SET_AFK_MESSAGE, &w.finish());
    }
    let mut w = Writer::new();
    w.u32(u32::from(away));
    c.session.send_action(action::SET_AFK_MODE, &w.finish());
    Ok(())
}

/// A command for the server, sent the way ACE reads them: a Talk line with an
/// `@` in front (`GameActionTalk`; it answers "Unknown command" for the rest).
pub(super) fn server_command(c: &mut Client, line: &str) -> Outcome {
    let line = line.trim_start_matches(['/', '@']).trim();
    if line.is_empty() {
        return Err(Refused::ours("no command"));
    }
    // A logout the player asked for is a session ending on purpose, however
    // the server ends up ending it: never reconnected (see `crate::reconnect`).
    let word = line.split_whitespace().next().unwrap_or_default();
    if matches!(word, "logout" | "logoff" | "quit" | "exit") {
        c.quitting = true;
    }
    let mut w = Writer::new();
    w.string16(&format!("@{line}"));
    c.session.send_action(action::TALK, &w.finish());
    Ok(())
}
