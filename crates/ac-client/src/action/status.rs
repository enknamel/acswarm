//! The status and who family: what the client can answer by itself --
//! where the body stands, the client's version, the frame-rate display,
//! the daylight switch and what endurance is for, and what only the
//! server knows: how old this character is and when it was made.
//!
//! The lines these print are retail's own words, because they are the
//! command's whole answer; a refusal is in acswarm's voice, as in every
//! other family.

use ac_net::messages::{action, opcode, queue};
use ac_net::wire::{Reader, Writer};

use super::{Outcome, Refused};
use crate::{options, Client, Event};

/// Always-daylight outdoors, the option `/day` flips (ACE
/// `CharacterOption.AlwaysDaylightOutdoors`, `CharacterOptions2` bit 0x1).
/// Not in [`options::OPTIONS`]: it is drawn by this client, not asked for
/// on the options panel.
const ALWAYS_DAYLIGHT: options::CharacterOption = options::CharacterOption {
    id: 0x05,
    word: options::Word::Two,
    bit: 0x0000_0001,
    label: "Always daylight outdoors",
    inverted: false,
};

/// What endurance does for a character, in the words retail's client kept
/// (its string at 0x7df318), a line for each of its paragraphs.
const ENDURANCE: &[&str] = &[
    "The endurance attribute has a number of abilities tied to it.",
    "First, some combination of strength and endurance (with endurance being more important) now \
     allows one to regenerate hit points at a faster rate the higher one's endurance is.  This \
     bonus is in addition to any regeneration spells one may have placed upon themselves.  This \
     endurance regeneration bonus caps at around 110%.",
    "Second, the higher a player's Endurance, the less stamina one uses while attacking.  This \
     benefit is tied to Endurance only, and it caps out at around 50% less stamina used per \
     attack.  The minimum stamina used per attack remains one.",
    "Third, the higher a player's Endurance, the more likely they are not to use a point of \
     stamina to successfully evade a missile or melee attack.  A player is required to have Melee \
     Defense for melee attacks or Missile Defense for missile attacks trained or specialized in \
     order for this specific ability to work.  This benefit is tied to Endurance only, and it caps \
     out at around a 75% chance to avoid losing a point of stamina per successful evasion.",
    "Fourth, some combination of strength and endurance (the two are roughly of equivalent \
     importance) now allows one to partially resist drain and harm attacks, up to a maximum of \
     roughly 50%.",
    "Fifth, some combination of strength and endurance (the two are roughly of equivalent \
     importance) now allows one to have a level of \"natural resistances\" to the 7 damage types, \
     the same as a certain level of life protections.  This caps out at a 50% resistance (the \
     equivalent to level 5 life prots) to these damage types.  This resistance is not additive to \
     life protections: higher level life protections will overwrite these natural resistances, \
     although life vulns will take these natural resistances into account, if the player does not \
     have a higher level life protection cast upon him.",
    "The natural resistances, drain resistances, and regeneration rate info are now visible on the \
     Character Information Panel, in what was once the Burden panel.  This panel now displays the \
     above three Endurance benefits, the burden info, as well as information about your age, birth \
     date, and number of deaths.",
    "The 5 categories for the endurance benefits are, in order from lowest benefit to highest: \
     Poor, Mediocre, Hardy, Resilient, and Indomitable, with each range of benefits divided up \
     equally amongst the 5 (e.g. Poor describes having anywhere from 1-10% resistance against \
     drain health attacks, etc.).",
];

/// A line of ours for the chat log, in the type retail printed these
/// answers in (0, the server's Broadcast).
fn line(c: &mut Client, text: impl Into<String>) {
    c.events.push(Event::Chat {
        text: text.into(),
        kind: 0,
    });
}

/// How long this character has been played (QueryAge 0x01C2: the target's
/// name, empty for ourselves -- only an admin named another).
pub(super) fn age(c: &mut Client) -> Outcome {
    let mut w = Writer::new();
    w.string16("");
    c.session.send_action(action::QUERY_AGE, &w.finish());
    Ok(())
}

/// When this character was made (QueryBirth 0x01C4); ACE answers in
/// ordinary system chat (`GameActionQueryBirth.cs:17`).
pub(super) fn birth(c: &mut Client) -> Outcome {
    let mut w = Writer::new();
    w.string16("");
    c.session.send_action(action::QUERY_BIRTH, &w.finish());
    Ok(())
}

/// What endurance is for: a text the client holds, with nothing asked of
/// the server.
pub(super) fn endurance(c: &mut Client) -> Outcome {
    for paragraph in ENDURANCE {
        line(c, *paragraph);
    }
    Ok(())
}

/// Where the body stands, in the cell id, frame and rotation retail
/// printed (`0x%08X [%f %f %f] %f %f %f %f`, rotation w first).
pub(super) fn location(c: &mut Client) -> Outcome {
    let Some(p) = c.player.as_ref() else {
        return Err(Refused::ours("not in the world"));
    };
    if p.cell == 0 {
        return Err(Refused::ours("not in a valid cell"));
    }
    let (cell, l, q) = (p.cell, p.local, p.rotation());
    line(
        c,
        format!(
            "Your location is: 0x{cell:08X} [{:.6} {:.6} {:.6}] {:.6} {:.6} {:.6} {:.6}",
            l.x, l.y, l.z, q.w, q.x, q.y, q.z
        ),
    );
    Ok(())
}

/// The client's own version, and whether Turbine chat is on. Retail also
/// asked the server for its build, but only from an admin, arch or PSR
/// character (PropertyBool 44, 45, 97), and so does this.
pub(super) fn version(c: &mut Client) -> Outcome {
    if c.allegiance_room != 0 {
        line(c, "Using Turbine Chat.");
    }
    line(
        c,
        concat!("Client version acswarm ", env!("CARGO_PKG_VERSION")),
    );
    if [44, 45, 97]
        .iter()
        .any(|&p| c.world.stats.bool_prop(p) == Some(true))
    {
        let mut w = Writer::new();
        w.u32(opcode::GET_SERVER_VERSION);
        c.session.send_message(queue::WEENIE, w.finish());
    }
    Ok(())
}

/// Show the frame rate, or stop. acswarm's status line has always shown
/// it, so it starts on; retail's own display started off.
pub(super) fn framerate(c: &mut Client) -> Outcome {
    c.show_framerate = !c.show_framerate;
    Ok(())
}

/// Daylight outdoors whatever the hour, or the world's own time again.
/// The option is the server's to keep, so it goes out as one.
pub(super) fn daylight(c: &mut Client) -> Outcome {
    let on = !c.option_enabled(&ALWAYS_DAYLIGHT);
    c.set_option(&ALWAYS_DAYLIGHT, on);
    line(
        c,
        if on {
            "Let there be light!"
        } else {
            "Normality has been restored."
        },
    );
    Ok(())
}

impl Client {
    /// Whether outdoors is drawn in daylight whatever the hour, which
    /// `/day` turns on: the window's question, not the world's clock.
    pub fn is_always_daylight(&self) -> bool {
        self.option_enabled(&ALWAYS_DAYLIGHT)
    }

    /// QueryAgeResponse 0x01C3: the target's name and its age in words
    /// (`GameEventQueryAgeResponse.cs`). The name is empty for ourselves.
    pub(crate) fn hear_age(&mut self, body: &[u8]) {
        let mut r = Reader::new(body);
        let (Ok(who), Ok(age)) = (r.string16(), r.string16()) else {
            return;
        };
        let text = if who.is_empty() {
            format!("You have played for {age}.")
        } else {
            format!("{who} has played for {age}.")
        };
        line(self, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::{Action, Line};
    use crate::testkit;
    use crate::Event;

    /// The lines a command put in the chat log.
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
    fn loc_prints_the_cell_and_the_frame_the_body_is_in() {
        let mut c = testkit::offline_client();
        assert!(
            matches!(c.chat_line("/loc"), Line::Acted(Err(_))),
            "nowhere to stand before the character is placed"
        );
        testkit::stand(&mut c, 0xA9B4_0019, glam::vec3(84.0, 108.5, 94.0));
        assert_eq!(c.chat_line("/loc"), Line::Acted(Ok(())));
        let told = said(&mut c).remove(0);
        assert!(
            told.starts_with(
                "Your location is: 0xA9B40019 [84.000000 108.500000 94.000000] 1.000000"
            ),
            "{told}"
        );
        assert_eq!(
            told.split_whitespace().count(),
            11,
            "the cell, three for the frame and four for the rotation: {told}"
        );
    }

    #[test]
    fn a_client_answered_command_takes_no_arguments() {
        for line in ["/loc here", "/version 2", "/framerate on"] {
            let mut c = testkit::offline_client();
            assert!(
                matches!(c.chat_line(line), Line::Acted(Err(_))),
                "{line} must be refused, as retail refused it"
            );
        }
        // Retail ignored what was typed after these two.
        let mut c = testkit::offline_client();
        assert_eq!(c.chat_line("/endurance please"), Line::Acted(Ok(())));
        assert_eq!(c.chat_line("/day now"), Line::Acted(Ok(())));
    }

    #[test]
    fn version_names_the_client_and_nothing_goes_out() {
        let mut c = testkit::offline_client();
        let sent = c.session.actions_sent();
        assert_eq!(c.chat_line("/version"), Line::Acted(Ok(())));
        let lines = said(&mut c);
        assert_eq!(lines.len(), 1, "no Turbine room, so no Turbine line");
        assert!(lines[0].starts_with("Client version acswarm "), "{lines:?}");
        assert_eq!(
            c.session.actions_sent(),
            sent,
            "the client answers this one"
        );
        // Turbine chat is on once the server has named our allegiance room.
        c.allegiance_room = 7;
        assert_eq!(c.act(Action::Version), Ok(()));
        assert_eq!(said(&mut c)[0], "Using Turbine Chat.");
    }

    #[test]
    fn endurance_says_what_endurance_is_for() {
        let mut c = testkit::offline_client();
        assert_eq!(c.chat_line("/endurance"), Line::Acted(Ok(())));
        let lines = said(&mut c);
        assert_eq!(lines.len(), ENDURANCE.len());
        assert!(lines[0].starts_with("The endurance attribute"));
    }

    #[test]
    fn framerate_flips_the_display_and_says_nothing() {
        let mut c = testkit::offline_client();
        assert!(c.show_framerate, "acswarm's status line starts with it");
        assert_eq!(c.chat_line("/framerate"), Line::Acted(Ok(())));
        assert!(!c.show_framerate);
        assert!(said(&mut c).is_empty(), "retail printed nothing here");
        assert_eq!(c.chat_line("/framerate"), Line::Acted(Ok(())));
        assert!(c.show_framerate);
    }

    #[test]
    fn day_flips_daylight_outdoors_and_tells_the_server() {
        let mut c = testkit::offline_client();
        let sent = c.session.actions_sent();
        assert!(!c.is_always_daylight());
        assert_eq!(c.chat_line("/day"), Line::Acted(Ok(())));
        assert!(c.is_always_daylight());
        assert_eq!(said(&mut c), ["Let there be light!"]);
        assert_eq!(
            c.session.actions_sent(),
            sent + 1,
            "the option is the server's to keep"
        );
        assert_eq!(c.chat_line("/day"), Line::Acted(Ok(())));
        assert!(!c.is_always_daylight());
        assert_eq!(said(&mut c), ["Normality has been restored."]);
    }

    #[test]
    fn age_and_birth_are_asked_of_the_server() {
        let mut c = testkit::offline_client();
        let sent = c.session.actions_sent();
        assert_eq!(c.chat_line("/age"), Line::Acted(Ok(())));
        assert_eq!(c.chat_line("/birth"), Line::Acted(Ok(())));
        assert_eq!(c.session.actions_sent(), sent + 2);
        assert!(said(&mut c).is_empty(), "the answer comes back in chat");
        // Retail ignored what was typed after either of them.
        assert_eq!(c.chat_line("/age now"), Line::Acted(Ok(())));
        assert_eq!(c.chat_line("/birth now"), Line::Acted(Ok(())));
    }

    #[test]
    fn the_age_answer_names_who_was_asked_about() {
        let mut c = testkit::offline_client();
        let mut w = Writer::new();
        w.string16("").string16("5d 8h 52m 25s");
        c.hear_age(&w.finish());
        assert_eq!(said(&mut c), ["You have played for 5d 8h 52m 25s."]);
        let mut w = Writer::new();
        w.string16("Verity").string16("1y 2mo");
        c.hear_age(&w.finish());
        assert_eq!(said(&mut c), ["Verity has played for 1y 2mo."]);
        c.hear_age(&[0, 0]);
        assert!(said(&mut c).is_empty(), "a truncated answer says nothing");
    }
}
