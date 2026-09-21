//! The chat family: what the character says and to whom -- aloud, to one
//! player, in a Turbine room or on a group channel -- the away message,
//! whom we stop hearing, the chat window the front end keeps, and the
//! commands the server itself reads off a Talk line.

use ac_net::messages::action;
use ac_net::wire::Writer;

use super::{line, Action, Outcome, Refused};
use crate::{Client, Event};

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
    // Only a tell sets whom `/retell` tells again; a reply does not.
    c.last_told = Some(who.to_string());
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

/// Tell the last player who told us (TalkDirect 0x0032: the text, then
/// the target's guid). The server looks that guid up in our own
/// landblock alone (ACE GameActionTalkDirect.cs:21), so a reply across
/// the world is refused where `/tell` by name would have arrived.
pub(super) fn reply(c: &mut Client, text: &str) -> Outcome {
    let text = text.trim();
    if text.is_empty() {
        return Err(Refused::ours("a reply wants something to say"));
    }
    let Some((guid, _)) = c.last_teller.clone() else {
        return Err(Refused::ours("nobody has told us anything yet"));
    };
    let mut w = Writer::new();
    w.string16(text).u32(guid);
    c.session.send_action(action::TALK_DIRECT, &w.finish());
    Ok(())
}

/// Tell the last player we told, by name, the way `/tell` does.
pub(super) fn retell(c: &mut Client, text: &str) -> Outcome {
    let text = text.trim();
    if text.is_empty() {
        return Err(Refused::ours("a retell wants something to say"));
    }
    let Some(who) = c.last_told.clone() else {
        return Err(Refused::ours("we have told nobody yet"));
    };
    tell(c, &who, text)
}

/// Squelch one player, or their whole account. `who` None is retail's
/// `-reply`: the last player who told us.
pub(super) fn squelch(
    c: &mut Client,
    who: Option<&str>,
    account: bool,
    kind: u32,
    on: bool,
) -> Outcome {
    let who = match who {
        Some(w) => w.trim().to_string(),
        None => match c.last_teller.clone() {
            Some((_, name)) => name,
            None => return Err(Refused::ours("nobody has told us anything yet")),
        },
    };
    if who.is_empty() {
        return Err(Refused::ours("a squelch wants somebody to squelch"));
    }
    if account {
        c.squelch_account(&who, on);
    } else {
        c.squelch(0, &who, kind, on);
    }
    Ok(())
}

/// Squelch a whole category from everyone, which is what `/filter`,
/// `/chat` and `/notell` each ask for.
pub(super) fn filter(c: &mut Client, kind: u32, on: bool) -> Outcome {
    c.squelch_global(kind, on);
    Ok(())
}

/// Whom we squelch, or the categories we filter from everyone, as
/// SetSquelchDB last left them.
pub(super) fn show_squelches(c: &mut Client, global: bool) -> Outcome {
    if global {
        let types = mask_words(c.world.squelches.global, "none");
        line(
            c,
            "The following types of messages are currently being filtered globally:",
        );
        line(c, types);
        return Ok(());
    }
    line(
        c,
        "(account) denotes a character whose account has also been squelched.",
    );
    line(c, "Format: Name : List of squelched message types.");
    line(c, "--------");
    let squelched = c.world.squelches.characters.clone();
    if squelched.is_empty() {
        line(c, "none");
        return Ok(());
    }
    for s in squelched {
        let account = if s.account { " (account) " } else { " " };
        let types = mask_words(s.mask, "");
        line(c, format!("  Name: {}{account}{types}", s.name));
    }
    Ok(())
}

/// The categories a squelch or a filter can name.
pub(super) fn show_message_types(c: &mut Client) -> Outcome {
    let named = ac_net::messages::squelch::named_in_mask(u32::MAX).join(", ");
    line(c, "Squelch channels are as follows:");
    line(c, format!("  {named}"));
    Ok(())
}

/// Join a chat channel (AddChannel 0x0145), leave it (RemoveChannel
/// 0x0146), or ask who is on it (ListChannels 0x0148). ACE answers a
/// player who is not an advocate with nothing at all
/// (GameActionAddChannel.cs:10).
pub(super) fn channel_join(c: &mut Client, channel: u32) -> Outcome {
    c.session
        .send_action(action::ADD_CHANNEL, &channel.to_le_bytes());
    Ok(())
}

pub(super) fn channel_leave(c: &mut Client, channel: u32) -> Outcome {
    c.session
        .send_action(action::REMOVE_CHANNEL, &channel.to_le_bytes());
    Ok(())
}

pub(super) fn channel_list(c: &mut Client, channel: u32) -> Outcome {
    c.session
        .send_action(action::LIST_CHANNELS, &channel.to_le_bytes());
    Ok(())
}

/// Which channels we may join (IndexChannels 0x0149, no body).
pub(super) fn channel_index(c: &mut Client) -> Outcome {
    c.session.send_action(action::INDEX_CHANNELS, &[]);
    Ok(())
}

/// Copy the chat to a file from now on, or stop with `None`. The file is
/// the front end's: it holds the chat log and knows what it has shown.
pub(super) fn chat_to_file(c: &mut Client, file: Option<&str>) -> Outcome {
    c.events
        .push(Event::ChatToFile(file.map(|f| f.trim().to_string())));
    Ok(())
}

pub(super) fn chat_clear(c: &mut Client, all: bool) -> Outcome {
    c.events.push(Event::ChatClear { all });
    Ok(())
}

/// Retail renamed the popup chat window the line was typed in, and
/// refused the main ones (window 1 and 8). This client has one chat
/// window with tabs and no popups, so every line meets that refusal.
pub(super) fn chat_title(_c: &mut Client, _name: &str) -> Outcome {
    Err(Refused::ours(
        "this command must be issued from a popup chat window",
    ))
}

/// Something to say in a Turbine chat room, for the row of the room it
/// is said in.
pub(super) fn room_line(room: u32, args: &str) -> Option<Action> {
    (!args.is_empty()).then(|| Action::Room {
        room,
        text: args.to_string(),
    })
}

/// The one word `@chat` and `@notell` take. `strict` is `@chat`, which
/// wants "on" or "off"; `@notell` reads any other word as on.
pub(super) fn on_or_off(args: &str, strict: bool) -> Option<bool> {
    let mut words = args.split_whitespace();
    let word = words.next()?;
    if words.next().is_some() {
        return None;
    }
    if word.eq_ignore_ascii_case("off") {
        Some(false)
    } else if !strict || word.eq_ignore_ascii_case("on") {
        Some(true)
    } else {
        None
    }
}

/// `@squelch [-account] [-CATEGORY]... NAME` and `@squelch -reply ...`,
/// or the list with no arguments. The flags come first and the last
/// category named wins, as retail's parser left them (FUN_0057aa00).
pub(super) fn squelch_line(args: &str, on: bool) -> Option<Action> {
    if args.is_empty() {
        return Some(Action::ShowSquelches { global: false });
    }
    let mut account = false;
    let mut to_the_teller = false;
    let mut kind = ac_net::messages::squelch::ALL;
    let mut words = args.split_whitespace().peekable();
    while let Some(flag) = words.peek().and_then(|w| w.strip_prefix('-')) {
        match flag.to_ascii_lowercase().as_str() {
            "reply" => to_the_teller = true,
            "account" => account = true,
            category => kind = ac_net::messages::squelch::from_name(category)?,
        }
        words.next();
    }
    let who: Vec<&str> = words.collect();
    // `-reply` names the last teller, so any words after it are retail's
    // to ignore.
    if !to_the_teller && who.is_empty() {
        return None;
    }
    Some(Action::Squelch {
        who: (!to_the_teller).then(|| who.join(" ")),
        account,
        kind,
        on,
    })
}

/// `@filter -CATEGORY` and `@unfilter -CATEGORY`, or the list with no
/// arguments. One category, never a name and never `-account` or
/// `-reply` (FUN_0057d0a0).
pub(super) fn filter_line(args: &str, on: bool) -> Option<Action> {
    if args.is_empty() {
        return Some(Action::ShowSquelches { global: true });
    }
    let mut words = args.split_whitespace();
    let category = words.next()?.strip_prefix('-')?;
    if words.next().is_some() {
        return None;
    }
    Some(Action::Filter {
        kind: ac_net::messages::squelch::from_name(category)?,
        on,
    })
}

/// What a squelch mask covers, in retail's words; `empty` is the answer
/// for a mask with nothing listed in it.
fn mask_words(mask: u32, empty: &str) -> String {
    if mask == u32::MAX {
        return "All message types".to_string();
    }
    let named = ac_net::messages::squelch::named_in_mask(mask);
    if named.is_empty() {
        empty.to_string()
    } else {
        named.join(", ")
    }
}

#[cfg(test)]
mod tests;

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
