use super::*;
use crate::autoplay::loot::choose::{CORPSE_LIFE, LOOT_NEAR};
use crate::autoplay::team::turns::SAME_MOMENT;
use crate::autoplay::{Autoplay, Room};
use crate::testkit::{a_loot_profile, corpses_seen, looter, standing_in_the_field};

#[test]
fn a_body_a_step_or_two_off_is_walked_back_to_and_not_let_go_of() {
    // The server does not mind the distance: a corpse has no reset
    // interval, so it stays open until its viewer shuts it. Shutting
    // one the character was already holding handed it back to the
    // other eight and the walk back had to win it again -- fifty
    // times in one run.
    //
    // A dodge or a knock-back moves a character several metres in a
    // frame, and that is the drift this covers.
    assert!(still_holding_at(CORPSE_REACH + 0.5), "no room to drift");
    assert!(still_holding_at(HOLD_ON_WITHIN));
    // Really gone: shut it and walk back to it like any other body.
    assert!(!still_holding_at(HOLD_ON_WITHIN + 0.5));
    // Never as far as the twenty metres the looting calls its own,
    // or a body held would be one nothing else could ever have.
    assert!(!still_holding_at(LOOT_NEAR));
}

#[test]
fn a_corpse_set_aside_is_not_written_off_and_is_tried_again_later() {
    // What `autoplay_loot` does with a corpse no walk reaches: set aside, not written off, and
    // neither the looting nor the next fight waits on it meanwhile.
    let t0 = Instant::now();
    let s = Duration::from_secs;
    let guid = 0x8000_3001;
    let mut ap = Autoplay::default();
    let (me, at) = (glam::Vec3::ZERO, glam::Vec3::new(12.0, 0.0, 0.0));
    let room = Room::PLENTY;
    assert!(ap.corpse_owed(guid, at, me, t0, room));
    ap.set_aside_out_of_reach(guid, t0);
    assert!(!ap.looted.contains(&guid), "written off for good");
    assert!(!ap.corpse_owed(guid, at, me, t0, room), "still owed");
    assert!(
        ap.corpse_owed(guid, at, me, t0 + s(60), room),
        "never tried again"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_corpse_up_on_a_ledge_is_set_aside_at_once() {
    // The Tusker ground (0x5C49): a Female Tusker shot dead on its ledge 6 m up the wall, which
    // no walk reaches, and another dead on the floor. The ledge one is nearer rotting, so first.
    let t = crate::walk_trial::named(
        20,
        "5C49 Tusker on a ledge over the room",
        glam::Vec3::new(17_744.8, 13_910.7, 0.0),
        0x5C49_012A,
        glam::Vec3::new(17_754.0, 13_929.0, 6.0),
    );
    let mut c = crate::walk_trial::standing_at(20, &t);
    c.world.player_guid = Some(crate::testkit::ME);
    c.autoplay.config.enabled = true;
    a_loot_profile(&mut c, "ledge test");
    let me = c.my_position().expect("standing");
    let t0 = Instant::now();
    let (ledge, floor) = (0x8000_0101, 0x8000_0102);
    let on_floor = c
        .standable_near(me, 8.0)
        .expect("built offline")
        .into_iter()
        .find(|p| p.at.distance(me) > 4.0 && (p.at.z - me.z).abs() < 0.5)
        .expect("a floor spot near the start");
    for (guid, place) in [
        (
            ledge,
            ac_world::Place {
                at: t.goal,
                cell: t.goal_cell,
            },
        ),
        (floor, on_floor),
    ] {
        let mut body = crate::testkit::corpse(guid, "Corpse of Female Tusker");
        body.position = crate::testkit::placed(place.cell, place.at);
        c.world.objects.insert(guid, body);
        c.autoplay.kill_spots.push((place.at, t0));
    }
    let long_ago = t0 - (CORPSE_LIFE - Duration::from_secs(30));
    c.autoplay.corpse_seen = corpses_seen([(ledge, long_ago), (floor, t0)]);

    let dt = 0.05;
    let mut now = t0;
    while now < t0 + Duration::from_secs(2) && c.autoplay.corpse.is_none() {
        c.tick_autoplay(now);
        c.tick_player(crate::player::Input::default(), dt, now);
        assert!(
            c.walk_frame
                .as_ref()
                .is_none_or(|w| w.goal.distance(t.goal) > 0.5),
            "walked at the ledge"
        );
        now += Duration::from_secs_f32(dt);
    }
    assert_eq!(
        c.autoplay.corpse.map(|(g, ..)| g),
        Some(floor),
        "the floor one opened"
    );
    assert!(
        c.autoplay.shelved.held(&ledge, now),
        "the ledge one not set aside"
    );
}

#[test]
fn a_corpse_underfoot_gets_the_plain_wait() {
    assert_eq!(loot_wait(0.0), LOOT_TIMEOUT);
    // A negative distance cannot happen, but must not panic or
    // shorten the wait.
    assert_eq!(loot_wait(-5.0), LOOT_TIMEOUT);
}

#[test]
fn a_corpse_across_a_room_is_given_time_to_walk_to() {
    // The bug: a flat six seconds covered a corpse at our feet and
    // not one twenty metres off, so the far ones were written off
    // unopened.
    let near = loot_wait(2.0);
    let far = loot_wait(20.0);
    assert!(far > near, "{far:?} is not longer than {near:?}");
    assert!(far > LOOT_TIMEOUT * 2, "twenty metres barely added time");
}

#[test]
fn the_wait_does_not_run_away_with_itself() {
    // Whatever distance arrives, the character does not sit on a
    // corpse for ever.
    let silly = loot_wait(100_000.0);
    assert!(silly <= LOOT_TIMEOUT + std::time::Duration::from_secs(30));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_walk_to_a_body_this_character_has_given_up_on_is_let_go_of() {
    // The one way out of the looting that did not stop the walk. A
    // character that chose a body, set off for it, and then heard a
    // better claim on it walked on -- and the walk is itself a claim
    // (see `Autoplay::corpse_claim`), so it went on telling the
    // others the body was taken, and went on holding the looting's
    // own worth up, for a body it would never open.
    let holtburg = 0xA9B4_0019;
    let at = glam::Vec3::new(84.0, 84.0, 10.0);
    let mut c = standing_in_the_field(20, holtburg, at);
    a_loot_profile(&mut c, "walk test");
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let body = 0x8000_0001;
    c.world.objects.insert(
        body,
        ac_world::WorldObject {
            guid: body,
            name: "Corpse of Drudge Slave".into(),
            object_desc_flags: ac_world::object_desc_flags::CORPSE,
            position: Some(ac_world::object::Position::new_flat(
                holtburg,
                me + glam::Vec3::new(7.0, 0.0, 0.0) - ac_world::landblock_origin(holtburg),
            )),
            ..Default::default()
        },
    );
    c.autoplay.corpse_seen = corpses_seen([(body, now)]);
    // Walking to it, seven metres off, and a mate says it claimed
    // the same body well before we did.
    c.autoplay.walking_to = Some(CorpseWalk {
        guid: body,
        started: now,
    });
    c.autoplay.team.mates = vec![looter(
        0x5000_0002,
        me,
        Some(body),
        SAME_MOMENT + Duration::from_secs(5),
    )];

    assert!(!c.autoplay_loot(now), "went to a body that is not ours");
    assert_eq!(c.autoplay.walking_to.map(|w| w.guid), None, "still walking");
    assert_eq!(c.autoplay.corpse_claim(now), None, "still claiming it");
}
