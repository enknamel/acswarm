//! Corpse looting: the permissions other players give us (`/consent`), the
//! ones we give them (`/permit`), and where our own last corpse fell
//! (`/corpse`, which asks the server nothing).

use ac_net::messages::action;
use ac_net::wire::Writer;
use ac_world::recalls::position_type;

use super::{Outcome, Refused};
use crate::options;
use crate::{Client, Event};

/// Accept corpse-looting permissions, or stop. Retail set the option's bit
/// itself; the server is told the way every other option is.
pub(super) fn accept(c: &mut Client, on: bool) -> Outcome {
    let option = options::option_by_id(options::ACCEPT_LOOT_PERMITS)
        .ok_or_else(|| Refused::ours("no corpse-looting option"))?;
    c.set_option(option, on);
    say(
        c,
        if on {
            "You can now accept corpse looting permissions from other players."
        } else {
            "You are no longer accepting corpse looting permissions from other players."
        },
    );
    Ok(())
}

/// Who has let us loot their corpse; the server answers in chat.
pub(super) fn list(c: &mut Client) -> Outcome {
    c.session
        .send_action(action::DISPLAY_PLAYER_CONSENT_LIST, &[]);
    Ok(())
}

/// Give back every permission we hold.
pub(super) fn clear(c: &mut Client) -> Outcome {
    c.session
        .send_action(action::CLEAR_PLAYER_CONSENT_LIST, &[]);
    Ok(())
}

/// Give back one player's.
pub(super) fn drop_one(c: &mut Client, who: &str) -> Outcome {
    let who = who.trim();
    if who.is_empty() {
        return Err(Refused::ours("a consent list wants a name to drop"));
    }
    let mut w = Writer::new();
    w.string16(who);
    c.session
        .send_action(action::REMOVE_FROM_PLAYER_CONSENT_LIST, &w.finish());
    Ok(())
}

/// Let a player loot our corpse, or take that back. ACE grants an hour, and
/// only to somebody who is online and accepting (Player_Death.cs:736-817).
pub(super) fn permit(c: &mut Client, who: &str, allow: bool) -> Outcome {
    let who = who.trim();
    if who.is_empty() {
        return Err(Refused::ours("a permit wants a name"));
    }
    let mut w = Writer::new();
    w.string16(who);
    let op = if allow {
        action::ADD_PLAYER_PERMISSION
    } else {
        action::REMOVE_PLAYER_PERMISSION
    };
    c.session.send_action(op, &w.finish());
    Ok(())
}

/// Where the character last died outdoors, which the server sends with the
/// rest of its saved positions. Retail rounded to the landcell; these are
/// ACE's own map coordinates and can differ by a tenth.
pub(super) fn corpse_location(c: &mut Client) -> Outcome {
    let line = match c
        .world
        .stats
        .recall_position(position_type::LAST_OUTSIDE_DEATH)
    {
        Some(p) => format!(
            "The last time you died outside, your corpse was located at ({}).",
            ac_world::map_coord_str(&p)
        ),
        None => "We're sorry, but we have no record of your last outside corpse location.".into(),
    };
    say(c, &line);
    Ok(())
}

/// A line for the chat log, as the server's own broadcasts arrive.
fn say(c: &mut Client, text: &str) {
    c.events.push(Event::Chat {
        text: text.to_string(),
        kind: 0,
    });
}

/// The rest of a command line as one name: retail joined the words and took
/// off the `+` an admin character's name is typed with.
pub(super) fn name_of(args: &str) -> String {
    args.trim().trim_start_matches('+').trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    /// The chat lines an action pushed.
    fn lines(c: &mut Client) -> Vec<String> {
        c.drain_events()
            .into_iter()
            .filter_map(|e| match e {
                Event::Chat { text, .. } => Some(text),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn consent_on_sets_the_loot_permits_bit_and_says_so() {
        let mut c = testkit::offline_client();
        let sent = c.session.actions_sent();
        assert_eq!(accept(&mut c, true), Ok(()));
        assert_eq!(c.world.stats.options.options1 & 0x0008_0000, 0x0008_0000);
        assert_eq!(c.session.actions_sent(), sent + 1, "the option went out");
        assert_eq!(lines(&mut c).len(), 1);
        assert_eq!(accept(&mut c, false), Ok(()));
        assert_eq!(c.world.stats.options.options1 & 0x0008_0000, 0);
    }

    #[test]
    fn a_permit_names_the_player_and_nothing_else() {
        let mut c = testkit::offline_client();
        let sent = c.session.actions_sent();
        assert_eq!(permit(&mut c, "Verity", true), Ok(()));
        assert_eq!(c.session.actions_sent(), sent + 1);
        assert!(permit(&mut c, "  ", true).is_err(), "no name, nothing sent");
        assert_eq!(c.session.actions_sent(), sent + 1);
    }

    #[test]
    fn a_name_loses_the_admin_plus() {
        assert_eq!(name_of("  +Verity  "), "Verity");
        assert_eq!(name_of("Fletch of Holtburg"), "Fletch of Holtburg");
        assert_eq!(name_of("   "), "");
    }

    #[test]
    fn the_corpse_line_says_where_or_says_there_is_no_record() {
        let mut c = testkit::offline_client();
        assert_eq!(corpse_location(&mut c), Ok(()));
        assert_eq!(
            lines(&mut c),
            ["We're sorry, but we have no record of your last outside corpse location."]
        );
        c.world.stats.set_recall_position(
            position_type::LAST_OUTSIDE_DEATH,
            ac_world::object::Position::new_flat(0xA9B4_0019, glam::Vec3::new(60.0, 90.0, 0.0)),
        );
        assert_eq!(corpse_location(&mut c), Ok(()));
        let said = lines(&mut c);
        assert!(
            said[0].starts_with("The last time you died outside, your corpse was located at ("),
            "{said:?}"
        );
        assert!(said[0].contains('N') || said[0].contains('S'), "{said:?}");
    }
}
