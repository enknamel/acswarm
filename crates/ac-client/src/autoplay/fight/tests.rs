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

/// An attack notice as the server sends it: ours landing on `name` (0x01B1), or `name`'s on us
/// (0x01B2, which carries the hit location before the critical flag).
fn blow(name: &str, damage: u32, on_us: bool) -> Vec<u8> {
    let mut w = ac_net::wire::Writer::new();
    let event = if on_us {
        ac_net::messages::event::DEFENDER_NOTIFICATION
    } else {
        ac_net::messages::event::ATTACKER_NOTIFICATION
    };
    w.u32(0x5000_0001)
        .u32(0)
        .u32(event)
        .string16(name)
        .u32(1)
        .f64(0.1)
        .u32(damage);
    if on_us {
        w.u32(0);
    }
    w.u32(0);
    w.finish()
}

#[test]
fn blows_are_counted_off_the_attack_notices() {
    let mut c = character_of_level(no_data(), 20);
    let event = ac_net::messages::opcode::GAME_EVENT;
    c.chat_message(event, &blow("Drudge Skulker", 7, false));
    c.chat_message(event, &blow("Drudge Skulker", 5, false));
    c.chat_message(event, &blow("Drudge Skulker", 3, true));
    c.chat_message(event, &evaded(0x8000_0001, "Drudge Skulker"));
    assert_eq!(
        c.blows,
        crate::tally::Blows {
            dealt: 2,
            dealt_points: 12,
            taken: 1,
            taken_points: 3,
            missed: 0,
            evaded: 1,
        }
    );
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
    c.autoplay.hit_by = Default::default();
    c.chat_message(
        op,
        &magic_line("You resist the spell cast by Drudge Shaman"),
    );
    assert!(c.hit_lately_by("Drudge Shaman"));

    // A fellow's heal is not.
    c.autoplay.last_hit_us = None;
    c.autoplay.hit_by = Default::default();
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
    // Hits are judged by the wall's clock, and setting off plans the first walk: seconds offline.
    let now = Instant::now();
    assert!(c.on_its_way());
    assert!(c.left_behind_on_the_road(guid, now), "it fell behind");
    c.autoplay.attacked_by("Drudge Skulker", now);
    assert!(
        !c.left_behind_on_the_road(guid, now),
        "it is still swinging"
    );
    c.autoplay.hit_by = Default::default();
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

/// A caster on the Tusker ground's floor (0x5C49) with Flame Bolt I, a Female Tusker on its ledge
/// 6 m up the wall (no walk reaches it) and another on the floor a little further off.
fn tuskers_on_the_ledge_and_the_floor(now: Instant) -> (Client, u32, u32) {
    let t = crate::walk_trial::named(
        20,
        "5C49 Tusker on a ledge over the room",
        glam::Vec3::new(17_744.8, 13_910.7, 0.0),
        0x5C49_012A,
        glam::Vec3::new(17_754.0, 13_929.0, 6.0),
    );
    let (mut c, _) = crate::testkit::a_caster_knowing(now, &["Flame Bolt I"]);
    let origin = ac_world::landblock_origin(t.start_cell);
    crate::testkit::stand(&mut c, t.start_cell, t.start - origin);
    let me = c.my_position().expect("standing");
    let ledge_away = me.distance(t.goal);
    let floor = c
        .standable_near(me, 25.0)
        .expect("built offline")
        .into_iter()
        .filter(|p| (p.at.z - me.z).abs() < 0.5)
        .filter(|p| (ledge_away + 1.0..24.0).contains(&p.at.distance(me)))
        .find(|p| matches!(c.plan_walk(*p, 1.0), ac_nav::routing::Planned::Route(_)))
        .expect("a floor spot further off than the ledge");
    let (on_ledge, on_floor) = (0x8000_0501, 0x8000_0502);
    for (guid, place) in [
        (
            on_ledge,
            ac_world::Place {
                at: t.goal,
                cell: t.goal_cell,
            },
        ),
        (on_floor, floor),
    ] {
        let mut tusker = crate::testkit::creature(guid, "Female Tusker");
        tusker.object_desc_flags |= ac_world::object_desc_flags::ATTACKABLE;
        tusker.position = crate::testkit::placed(place.cell, place.at);
        c.world.objects.insert(guid, tusker);
        appraised_as(&mut c, guid, Some(10), 50);
    }
    c.autoplay.config.fight.enabled = true;
    (c, on_ledge, on_floor)
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_target_on_a_ledge_nothing_walks_to_is_given_up_at_once() {
    let t0 = Instant::now();
    let (mut c, on_ledge, on_floor) = tuskers_on_the_ledge_and_the_floor(t0);
    let cfg = c.autoplay.config.fight.clone();
    // The fight is on the ledge one (it was the one in sight, or hitting us).
    c.autoplay.casting_at = Some(on_ledge);
    let dt = 0.05;
    let mut now = t0;
    let mut given_up_at = None;
    while now < t0 + Duration::from_secs(3) {
        c.autoplay_fight_with_spells(now, &cfg);
        c.tick_player(crate::player::Input::default(), dt, now);
        if given_up_at.is_none() && c.autoplay.given_up.since(&on_ledge).is_some() {
            given_up_at = Some(now - t0);
        }
        now += Duration::from_secs_f32(dt);
    }
    let given_up_at = given_up_at.expect("the ledge one was never given up");
    assert!(
        given_up_at <= Duration::from_secs(1),
        "stood {given_up_at:?} at it"
    );
    assert_eq!(
        c.autoplay.casting_at,
        Some(on_floor),
        "the floor one fought"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_target_given_up_that_comes_down_and_attacks_is_fought() {
    // The ledge one is given up; it drops beside the character and hits it: it is fought.
    let now = Instant::now();
    let (mut c, on_ledge, on_floor) = tuskers_on_the_ledge_and_the_floor(now);
    let cfg = c.autoplay.config.fight.clone();
    c.give_up_target(on_ledge, "no way to it", now);
    assert_eq!(c.pick_target(&cfg, now), Some(on_floor));
    let me = c.my_position().expect("standing");
    let cell = c.player.as_ref().map(|p| p.cell).expect("standing");
    if let Some(t) = c.world.objects.get_mut(&on_ledge) {
        t.position = crate::testkit::placed(cell, me + glam::Vec3::X);
    }
    c.autoplay.attacked_by("Female Tusker", Instant::now());
    assert!(c.can_take_on(on_ledge, &cfg, true, now));
    c.world.objects.remove(&on_floor);
    assert_eq!(c.pick_target(&cfg, now), Some(on_ledge));
    // Still up on its ledge, hit by another of its name: still given up.
    let (mut c, on_ledge, on_floor) = tuskers_on_the_ledge_and_the_floor(now);
    c.give_up_target(on_ledge, "no way to it", now);
    c.autoplay.attacked_by("Female Tusker", Instant::now());
    assert!(!c.can_take_on(on_ledge, &cfg, true, now));
    assert_eq!(c.pick_target(&cfg, now), Some(on_floor));
}

/// [`tuskers_on_the_ledge_and_the_floor`] for a character with a mace in hand.
fn with_a_mace(c: &mut Client) {
    let me = 0x5000_0001;
    c.world.player_guid = Some(me);
    for o in c.world.objects.values_mut() {
        if o.item_type & ac_world::item_type::CASTER != 0 {
            o.wielder = None;
            o.container = Some(me);
        }
    }
    crate::testkit::a_weapon(
        c,
        0x8000_0601,
        ac_world::item_type::MELEE_WEAPON,
        "Mace",
        true,
    );
}

/// Fight rounds at 20 Hz for `secs`, the body walking: whether a swing went out at `guid`.
fn swings_at(c: &mut Client, guid: u32, t0: Instant, secs: u64) -> bool {
    let dt = 0.05;
    let mut now = t0;
    let mut swung = false;
    while now < t0 + Duration::from_secs(secs) {
        c.autoplay_fight(now);
        c.tick_player(crate::player::Input::default(), dt, now);
        swung |= c.attacked == Some(guid);
        now += Duration::from_secs_f32(dt);
    }
    swung
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_swing_is_not_sent_at_a_creature_no_walk_reaches() {
    let t0 = Instant::now();
    let (mut c, on_ledge, on_floor) = tuskers_on_the_ledge_and_the_floor(t0);
    with_a_mace(&mut c);
    assert_eq!(c.combat_stance(), crate::Stance::Melee, "the case: a swing");
    // With both: the ledge one is never swung at, the floor one is.
    assert!(!swings_at(&mut c, on_ledge, t0, 6), "swung at the ledge");
    assert_eq!(c.attack_target, Some(on_floor), "the floor one fought");

    // Alone with the ledge one: given up at once, and the character stands.
    let (mut c, on_ledge, on_floor) = tuskers_on_the_ledge_and_the_floor(t0);
    with_a_mace(&mut c);
    c.world.objects.remove(&on_floor);
    let me = c.my_position().expect("standing");
    assert!(!swings_at(&mut c, on_ledge, t0, 2), "swung at the ledge");
    assert!(
        c.autoplay.given_up.since(&on_ledge).is_some(),
        "not given up"
    );
    let moved = c.my_position().expect("standing").distance(me);
    assert!(moved < 0.5, "walked {moved:.1} m");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_caster_closes_on_its_target_down_a_corridor_of_trap_plates() {
    // Blargerton, after the Lich Overseer down the Mines of Despair's corridor of plates: stood
    // inside one plate's berth, every walk flew a 0.1 m crossing, and from the air the shot
    // cleared, the walk was let go, and he sank out of sight again; 45 s bobbing on the spot.
    let now0 = std::time::Instant::now();
    let (mut c, _) = crate::testkit::a_caster_knowing(now0, &["Blade Arc VI"]);
    crate::testkit::stand(&mut c, 0x0188_0113, glam::Vec3::new(75.0, -100.4, -24.0));
    c.autoplay.config.enabled = true;
    c.autoplay.config.fight.enabled = true;
    c.autoplay.config.fight.radius = 60.0;
    let lich = 0x8000_0103;
    c.world.objects.insert(
        lich,
        ac_world::WorldObject {
            guid: lich,
            weenie_class_id: 8592,
            name: "Lich Overseer".into(),
            item_type: ac_world::item_type::CREATURE,
            object_desc_flags: ac_world::object_desc_flags::ATTACKABLE,
            health: Some(1.0),
            position: Some(ac_world::object::Position::new_flat(
                0x0188_010D,
                glam::Vec3::new(41.1, -95.8, -24.0),
            )),
            ..Default::default()
        },
    );
    let start = c.my_position().unwrap();
    let plates: Vec<_> = ac_world::traps::in_landblock(0x0188_0000).collect();
    let mut furthest = 0.0f32;
    for frame in 0..30 * 20 {
        let now = now0 + std::time::Duration::from_secs_f32(frame as f32 * 0.05);
        c.tick_autoplay(now);
        c.tick_player(crate::player::Input::default(), 0.05, now);
        c.pathfinder.settle(std::time::Duration::from_secs(5));
        let me = c.my_position().unwrap();
        furthest = furthest.max(me.distance(start));
        for p in &plates {
            let (centre, r) = p.sphere();
            let axis = glam::Vec3::new(me.x, me.y, centre.z.clamp(me.z + 0.48, me.z + 1.5));
            assert!(axis.distance(centre) >= r + 0.48, "on a plate at {me:?}");
        }
    }
    assert!(furthest >= 15.0, "came {furthest:.1} m down the corridor");
}
