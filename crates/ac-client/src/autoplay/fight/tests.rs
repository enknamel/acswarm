use super::*;
use crate::testkit::{
    appraised_as, character_of_level, evaded, in_view, no_data, standing_in_the_field, STRANGER,
};

#[test]
fn an_evaded_swing_counts_as_being_attacked() {
    let mut c = character_of_level(no_data(), 20);
    assert!(!c.under_attack(), "nothing has happened yet");
    c.chat_message(
        ac_net::messages::opcode::GAME_EVENT,
        &evaded(0x8000_0001, "Drudge Skulker"),
    );
    assert!(c.under_attack(), "a miss is an attack all the same");
    assert_eq!(
        c.autoplay
            .hit_by
            .iter()
            .map(|(who, _)| who.as_str())
            .collect::<Vec<_>>(),
        ["Drudge Skulker"],
        "and the server named who swung"
    );
    assert!(c.hit_lately_by("Drudge Skulker"));
    assert!(!c.hit_lately_by("Drudge Robber"), "the one standing by");
}

/// A line of system chat as the server sends it (ServerMessage 0xF7E0):
/// the text, and its ChatMessageType, Magic here.
fn magic_line(text: &str) -> Vec<u8> {
    let mut w = ac_net::wire::Writer::new();
    w.string16(text).u32(7);
    w.finish()
}

#[test]
fn a_caster_that_only_casts_is_attacking_the_character() {
    // A shaman turns and casts from its spell range and never closes
    // in to swing, so neither notification a swing brings ever came:
    // `under_attack` stayed false while it worked the character over,
    // and every "but fight back when it attacks you" carve-out walked
    // on past it.
    let mut c = character_of_level(no_data(), 20);
    let op = ac_net::messages::opcode::SERVER_MESSAGE;
    assert!(!c.under_attack(), "nothing has happened yet");
    c.chat_message(
        op,
        &magic_line("Drudge Shaman blasts you for 12 points with Flame Bolt I."),
    );
    assert!(c.under_attack(), "a bolt that landed is an attack");
    assert!(c.hit_lately_by("Drudge Shaman"));
    assert!(!c.hit_lately_by("Drudge Skulker"), "the one standing by");

    // Resisted, it was still cast at the character.
    c.autoplay.last_hit_us = None;
    c.autoplay.hit_by.clear();
    c.chat_message(
        op,
        &magic_line("You resist the spell cast by Drudge Shaman"),
    );
    assert!(c.hit_lately_by("Drudge Shaman"));

    // A fellow's heal is not.
    c.autoplay.last_hit_us = None;
    c.autoplay.hit_by.clear();
    c.chat_message(
        op,
        &magic_line("Aldric casts Heal Other I and restores 30 points of your health."),
    );
    assert!(!c.under_attack());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_fight_on_the_road_ends_when_the_creature_stops_following() {
    // Off the road the fight is the fight, however far it has got.
    assert!(!road_fight_over(false, false, false, false, 20.0));
    // On the road, a creature that has stopped attacking and fallen
    // behind is let go.
    assert!(road_fight_over(true, false, false, false, 20.0));
    // Not while it is still attacking, walking at us, or being fought
    // by one of the party on the road.
    assert!(!road_fight_over(true, true, false, false, 20.0));
    assert!(!road_fight_over(true, false, true, false, 20.0));
    assert!(!road_fight_over(true, false, false, true, 20.0));
    // And one a swing away is finished, not left at half health.
    assert!(!road_fight_over(
        true,
        false,
        false,
        false,
        ROAD_REACH - 1.0
    ));

    // The same, read off the world: a Drudge that swung once on the
    // road and fell twenty metres behind.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_in_the_field(20, holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let guid = 0x8000_0001;
    let place = |c: &mut Client, metres: f32| {
        let mut o = in_view(c, guid, 0, "Drudge Skulker");
        o.position = Some(ac_world::object::Position::new_flat(
            holtburg,
            me + glam::Vec3::new(metres, 0.0, 0.0) - ac_world::landblock_origin(holtburg),
        ));
        c.world.objects.insert(guid, o);
    };
    place(&mut c, 20.0);
    assert!(!c.left_behind_on_the_road(guid, now), "not on a road");
    assert!(
        c.travel_to(glam::Vec2::new(me.x + 250.0, me.y)),
        "no way there"
    );
    assert!(c.on_its_way());
    assert!(c.left_behind_on_the_road(guid, now), "it fell behind");
    c.autoplay.attacked_by("Drudge Skulker", now);
    assert!(
        !c.left_behind_on_the_road(guid, now),
        "it is still swinging"
    );
    c.autoplay.hit_by.clear();
    c.autoplay.last_hit_us = None;
    c.world.objects.get_mut(&guid).unwrap().walked_at = Some(0x5000_0001);
    assert!(!c.left_behind_on_the_road(guid, now), "it is coming at us");
    place(&mut c, 3.0);
    assert!(
        !c.left_behind_on_the_road(guid, now),
        "a swing away: finished"
    );
}

#[test]
fn a_second_attacker_does_not_silence_the_first() {
    // Two creatures on the character, swinging in turn: the last
    // name alone had the other walked past between its swings, and
    // the target moved off something still in melee with the
    // character.
    let mut c = character_of_level(no_data(), 20);
    let cfg = Fight::default();
    let now = Instant::now();
    let cow = in_view(&mut c, 0x8000_0031, STRANGER, "Cow");
    appraised_as(&mut c, cow.guid, Some(8), 20);
    assert!(c.a_critter(&cow, &cfg));
    c.autoplay.attacked_by("Cow", now);
    c.autoplay
        .attacked_by("Drudge Skulker", now + Duration::from_millis(500));
    assert!(!c.a_critter(&cow, &cfg), "still hitting the character");
    assert!(c.hit_lately_by("Drudge Skulker"));
    assert_eq!(c.autoplay.hit_by.len(), 2);
    // The same name again is the same attacker, at its latest.
    c.autoplay.attacked_by("Cow", now + Duration::from_secs(1));
    assert_eq!(c.autoplay.hit_by.len(), 2);
    // One that has not swung for a while is let go of.
    c.autoplay
        .attacked_by("Drudge Robber", now + UNDER_ATTACK + Duration::from_secs(2));
    assert_eq!(
        c.autoplay
            .hit_by
            .iter()
            .map(|(who, _)| who.as_str())
            .collect::<Vec<_>>(),
        ["Drudge Robber"]
    );
}
