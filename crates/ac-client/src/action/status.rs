//! The status and who family: what the client can answer by itself --
//! where the body stands, the client's version, the frame-rate display,
//! the daylight switch and what endurance is for -- what only the server
//! knows (age, birth, the dwellings for sale), and the friends list.
//!
//! The lines these print are retail's own words, because they are the
//! command's whole answer; a refusal is in acswarm's voice, as in every
//! other family.

use serde::{Deserialize, Serialize};

use ac_net::messages::{action, opcode, queue};
use ac_net::wire::{Reader, Writer};

use super::{line, Outcome, Refused};
use crate::{options, Client};

/// What `/friends` was asked for. `/friends_add` and `/friends_remove`
/// are retail's own names for two of these.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Friends {
    /// The whole list.
    List,
    /// Only those online.
    Online,
    Add(String),
    Remove(String),
    /// Everyone at once (`/friends remove -all`).
    RemoveAll,
    /// The list as the client before the friends window asked for it.
    Old,
}

/// A kind of dwelling, by the server's own numbers (ACE
/// `ACE.Entity/Enum/HouseType.cs:6-10`, which retail's `@hslist` matched).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HouseKind {
    Cottage,
    Villa,
    Mansion,
    Apartment,
}

impl HouseKind {
    /// The number the wire carries.
    pub fn id(self) -> u32 {
        match self {
            HouseKind::Cottage => 1,
            HouseKind::Villa => 2,
            HouseKind::Mansion => 3,
            HouseKind::Apartment => 4,
        }
    }

    fn of_id(id: u32) -> Option<HouseKind> {
        Some(match id {
            1 => HouseKind::Cottage,
            2 => HouseKind::Villa,
            3 => HouseKind::Mansion,
            4 => HouseKind::Apartment,
            _ => return None,
        })
    }

    /// How the count line names several of them.
    fn plural(self) -> &'static str {
        match self {
            HouseKind::Cottage => "cottages",
            HouseKind::Villa => "villas",
            HouseKind::Mansion => "mansions",
            HouseKind::Apartment => "apartments",
        }
    }
}

/// Retail's cap on the friends list (client string 0x561); the server
/// keeps none (ACE `WorldObjects/Player_Character.cs:141-176`).
const MOST_FRIENDS: usize = 50;

/// Locations past this many are the server's own, not sent (retail's
/// note in `FUN_00586bd0:46-48`).
const HOUSES_LISTED: u32 = 400;

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

/// `/friends [online | add NAME | remove NAME | remove -all | old]`; the
/// first word says which, whatever its case (retail `LAB_0057c9d0`).
pub(super) fn friends_args(args: &str) -> Option<Friends> {
    let (word, rest) = args
        .split_once(char::is_whitespace)
        .map(|(w, r)| (w, r.trim()))
        .unwrap_or((args, ""));
    Some(match word.to_ascii_lowercase().as_str() {
        "" => Friends::List,
        "online" => Friends::Online,
        "old" => Friends::Old,
        "add" => add_args(rest)?,
        "remove" => remove_args(rest)?,
        _ => return None,
    })
}

/// `/friends_add NAME`, and the same words after `/friends add`.
pub(super) fn add_args(name: &str) -> Option<Friends> {
    let name = name.trim();
    (!name.is_empty()).then(|| Friends::Add(name.to_string()))
}

/// `/friends_remove NAME | -all`, and the same words after
/// `/friends remove`.
pub(super) fn remove_args(name: &str) -> Option<Friends> {
    let name = name.trim();
    if name.eq_ignore_ascii_case("-all") {
        return Some(Friends::RemoveAll);
    }
    (!name.is_empty()).then(|| Friends::Remove(name.to_string()))
}

/// `/hslist apartment | cottage | villa | mansion`: the first word only,
/// whatever its case and never a prefix of one (retail `FUN_005712e0:17-50`).
pub(super) fn house_kind(args: &str) -> Option<HouseKind> {
    let word = args.split_whitespace().next().unwrap_or_default();
    Some(match word.to_ascii_lowercase().as_str() {
        "cottage" => HouseKind::Cottage,
        "villa" => HouseKind::Villa,
        "mansion" => HouseKind::Mansion,
        "apartment" => HouseKind::Apartment,
        _ => return None,
    })
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

/// How many dwellings of a kind are for sale, and where
/// (ListAvailableHouses 0x0270); [`Client::hear_houses`] prints the answer.
pub(super) fn houses_available(c: &mut Client, kind: HouseKind) -> Outcome {
    c.session
        .send_action(action::LIST_AVAILABLE_HOUSES, &kind.id().to_le_bytes());
    Ok(())
}

pub(super) fn friends(c: &mut Client, what: &Friends) -> Outcome {
    match what {
        Friends::List => show_friends(c, false),
        Friends::Online => show_friends(c, true),
        Friends::Add(name) => {
            if c.world.friends.len() >= MOST_FRIENDS {
                return Err(Refused::ours("the friends list is full"));
            }
            c.add_friend(name);
            Ok(())
        }
        Friends::Remove(name) => {
            // RemoveFriend names a guid, so the list is where a typed name
            // is looked up; the server never sees the name.
            let Some(guid) = friend_guid(c, name) else {
                return Err(Refused::ours(format!("no friend named {name:?}")));
            };
            c.remove_friend(Some(guid));
            Ok(())
        }
        Friends::RemoveAll => {
            c.remove_friend(None);
            // ACE clears the list without a word (Player_Character.cs:203-211),
            // so this line is the client's, as it was retail's.
            line(c, "Your friends list has been cleared.");
            Ok(())
        }
        Friends::Old => {
            // FriendsOld 0xF7CD (u32, string16), which ACE answers "That
            // command is not used in the emulator." (FriendsOldHandler.cs).
            let mut w = Writer::new();
            w.u32(opcode::FRIENDS_OLD).u32(0).string16("");
            c.session.send_message(queue::WEENIE, w.finish());
            Ok(())
        }
    }
}

/// The friends list as retail printed it: a heading, then each name
/// indented, with "(Online)" after those who are.
fn show_friends(c: &mut Client, online_only: bool) -> Outcome {
    if c.world.friends.is_empty() {
        line(c, "Your friends list is empty!");
        return Ok(());
    }
    let rows: Vec<String> = c
        .world
        .friends
        .iter()
        .filter(|f| f.online || !online_only)
        .map(|f| {
            if f.online {
                format!("  {} (Online)", f.name)
            } else {
                format!("  {}", f.name)
            }
        })
        .collect();
    line(c, "Your friends:");
    if rows.is_empty() {
        line(c, "  You have no friends that are online.");
    }
    for row in rows {
        line(c, row);
    }
    Ok(())
}

/// The guid of the friend of that name, whatever its case.
fn friend_guid(c: &Client, name: &str) -> Option<u32> {
    c.world
        .friends
        .iter()
        .find(|f| f.name.eq_ignore_ascii_case(name.trim()))
        .map(|f| f.guid)
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

    /// AvailableHouses 0x0271, the answer to `/hslist`: the kind, the cell
    /// of each dwelling for sale and how many there are in all
    /// (`GameEventHouseAvailableHouses.cs`). Apartments have no place on
    /// the map, so retail listed the locations of every other kind only.
    pub(crate) fn hear_houses(&mut self, body: &[u8]) {
        let mut r = Reader::new(body);
        let (Ok(kind), Ok(count)) = (r.u32(), r.u32()) else {
            return;
        };
        let Some(kind) = HouseKind::of_id(kind) else {
            return;
        };
        let mut cells = Vec::new();
        for _ in 0..count {
            let Ok(cell) = r.u32() else { break };
            cells.push(cell);
        }
        let Ok(total) = r.u32() else { return };
        line(
            self,
            format!("There are {total} {} available.", kind.plural()),
        );
        if kind == HouseKind::Apartment {
            return;
        }
        for cell in cells {
            let where_it_is = ac_world::map_coord_str(&ac_world::object::Position::new_flat(
                cell,
                ac_world::outdoor_cell_centre(cell),
            ));
            line(self, format!("     {where_it_is}"));
        }
        if total > HOUSES_LISTED {
            line(
                self,
                "There were too many houses to display all the locations. \
                 Only the first 400 locations are displayed here.",
            );
        }
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

    #[test]
    fn the_friends_list_is_read_off_the_one_the_server_sent() {
        let mut c = testkit::offline_client();
        assert_eq!(c.chat_line("/friends"), Line::Acted(Ok(())));
        assert_eq!(said(&mut c), ["Your friends list is empty!"]);
        c.world.friends = vec![
            ac_world::social::Friend {
                guid: 0x5000_0002,
                name: "Verity".into(),
                online: true,
            },
            ac_world::social::Friend {
                guid: 0x5000_0003,
                name: "Fletch".into(),
                online: false,
            },
        ];
        assert_eq!(c.chat_line("/friends"), Line::Acted(Ok(())));
        assert_eq!(
            said(&mut c),
            ["Your friends:", "  Verity (Online)", "  Fletch"]
        );
        assert_eq!(c.chat_line("/friends online"), Line::Acted(Ok(())));
        assert_eq!(said(&mut c), ["Your friends:", "  Verity (Online)"]);
        c.world.friends[0].online = false;
        assert_eq!(c.chat_line("/friends online"), Line::Acted(Ok(())));
        assert_eq!(
            said(&mut c),
            ["Your friends:", "  You have no friends that are online."]
        );
    }

    #[test]
    fn a_friend_is_added_by_name_and_removed_by_guid() {
        let mut c = testkit::offline_client();
        let sent = c.session.actions_sent();
        assert_eq!(c.chat_line("/friends add Verity"), Line::Acted(Ok(())));
        assert_eq!(c.chat_line("/friends_add Verity"), Line::Acted(Ok(())));
        assert_eq!(c.session.actions_sent(), sent + 2);
        // Removing wants the guid, which only the list we were sent holds.
        assert!(matches!(
            c.chat_line("/friends remove Verity"),
            Line::Acted(Err(_))
        ));
        c.world.friends = vec![ac_world::social::Friend {
            guid: 0x5000_0002,
            name: "Verity".into(),
            online: true,
        }];
        assert_eq!(c.chat_line("/friends_remove verity"), Line::Acted(Ok(())));
        assert_eq!(c.session.actions_sent(), sent + 3);
        assert_eq!(c.chat_line("/friends remove -all"), Line::Acted(Ok(())));
        assert_eq!(said(&mut c), ["Your friends list has been cleared."]);
        assert_eq!(c.chat_line("/friends old"), Line::Acted(Ok(())));
    }

    #[test]
    fn the_friends_grammar_is_retails() {
        assert_eq!(friends_args(""), Some(Friends::List));
        assert_eq!(friends_args("ONLINE"), Some(Friends::Online));
        assert_eq!(
            friends_args("add  Verity"),
            Some(Friends::Add("Verity".into()))
        );
        assert_eq!(friends_args("remove -ALL"), Some(Friends::RemoveAll));
        assert_eq!(
            friends_args("Remove Verity"),
            Some(Friends::Remove("Verity".into()))
        );
        assert_eq!(friends_args("add"), None, "a name is wanted");
        assert_eq!(friends_args("list"), None, "retail knew four words");
        assert_eq!(remove_args("-all"), Some(Friends::RemoveAll));
        assert_eq!(add_args("  "), None);
    }

    #[test]
    fn the_full_friends_list_takes_no_more() {
        let mut c = testkit::offline_client();
        c.world.friends = (0..MOST_FRIENDS)
            .map(|i| ac_world::social::Friend {
                guid: 0x5000_0000 + i as u32,
                name: format!("Friend{i}"),
                online: false,
            })
            .collect();
        assert!(matches!(
            c.chat_line("/friends add Verity"),
            Line::Acted(Err(_))
        ));
    }

    #[test]
    fn hslist_wants_one_of_the_four_kinds() {
        assert_eq!(house_kind("Cottage"), Some(HouseKind::Cottage));
        assert_eq!(house_kind("mansion and more"), Some(HouseKind::Mansion));
        assert_eq!(house_kind("cot"), None, "retail matched the whole word");
        assert_eq!(house_kind(""), None);
        let mut c = testkit::offline_client();
        let sent = c.session.actions_sent();
        assert!(matches!(c.chat_line("/hslist"), Line::Acted(Err(_))));
        assert_eq!(c.session.actions_sent(), sent, "nothing goes out unasked");
        assert_eq!(c.chat_line("/hslist villa"), Line::Acted(Ok(())));
        assert_eq!(c.session.actions_sent(), sent + 1);
    }

    #[test]
    fn the_houses_answer_counts_them_and_says_where() {
        let mut c = testkit::offline_client();
        let mut w = Writer::new();
        w.u32(HouseKind::Cottage.id())
            .u32(2)
            .u32(0xA9B4_0019)
            .u32(0xA9B4_0001)
            .u32(2);
        c.hear_houses(&w.finish());
        let lines = said(&mut c);
        assert_eq!(lines[0], "There are 2 cottages available.");
        assert_eq!(lines.len(), 3, "a line each: {lines:?}");
        assert!(
            lines[1].starts_with("     ") && lines[1].ends_with('E'),
            "{lines:?}"
        );
        // Apartments have no place on the map, so only the count is printed.
        let mut w = Writer::new();
        w.u32(HouseKind::Apartment.id()).u32(0).u32(37);
        c.hear_houses(&w.finish());
        assert_eq!(said(&mut c), ["There are 37 apartments available."]);
    }

    #[test]
    fn the_house_recalls_are_the_ones_the_panel_asks_for() {
        for (line, want) in [
            ("/hor", Action::RecallHouse),
            ("/hr", Action::RecallHouse),
            ("/hom", Action::RecallMansion),
            ("/hoa", Action::RecallMansion),
        ] {
            let mut c = testkit::offline_client();
            let sent = c.session.actions_sent();
            assert_eq!(c.chat_line(line), Line::Acted(Ok(())), "{line}");
            assert_eq!(c.session.actions_sent(), sent + 1, "{line}");
            assert_eq!(
                crate::action::command(line.trim_start_matches('/'))
                    .and_then(|cmd| (cmd.action)("")),
                Some(want),
                "{line}"
            );
        }
        // Retail sent nothing at all when words followed these.
        let mut c = testkit::offline_client();
        assert!(matches!(c.chat_line("/hor now"), Line::Acted(Err(_))));
    }
}
