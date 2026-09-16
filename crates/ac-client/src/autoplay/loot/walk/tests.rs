use super::*;
use crate::autoplay::loot::choose::LOOT_NEAR;
use crate::autoplay::team::turns::SAME_MOMENT;
use crate::autoplay::{Autoplay, Room};
use crate::testkit::{a_loot_profile, looter, standing_in_the_field};

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
fn a_walk_to_a_corpse_that_cannot_arrive_is_given_up_and_the_corpse_set_aside() {
    // Blargerton walked every tick at bodies through a floor or
    // behind a wall. Nothing timed the walk, so the looting, and the
    // fight waiting on the body, held until it rotted.
    let t0 = Instant::now();
    let s = Duration::from_secs;
    let guid = 0x8000_3001;

    // A long walk that keeps getting nearer goes on however long it
    // takes.
    let mut walk = CorpseWalk::new(guid, 40.0, t0);
    for i in 1..=30 {
        assert!(walk.goes_on(40.0 - i as f32 * 1.2, false, t0 + s(i)));
    }

    // Pressed against a wall and no nearer: given up once that has
    // gone on too long, and not before.
    let mut walk = CorpseWalk::new(guid, 12.0, t0);
    for i in 1..=REACH_GIVE_UP.as_secs() {
        assert!(walk.goes_on(11.5, false, t0 + s(i)), "gave up at {i} s");
    }
    let gave_up = t0 + REACH_GIVE_UP + s(1);
    assert!(!walk.goes_on(11.5, false, gave_up));

    // The steering finding no way there is given up on sooner, but
    // not at the first word of it.
    let mut walk = CorpseWalk::new(guid, 12.0, t0);
    assert!(walk.goes_on(12.0, true, t0 + s(1)));
    assert!(walk.goes_on(12.0, false, t0 + s(2)), "a way was found");
    let said = t0 + s(3);
    for i in 3..3 + NO_WAY_FOR.as_secs() {
        assert!(walk.goes_on(12.0, true, t0 + s(i)), "gave up at {i} s");
    }
    assert!(!walk.goes_on(12.0, true, said + NO_WAY_FOR));

    // A walk broken off for a fight is a new walk when it is taken
    // up again.
    let mut walk = CorpseWalk::new(guid, 12.0, t0);
    assert!(walk.goes_on(12.0, false, t0 + s(1)));
    assert!(walk.goes_on(12.0, false, t0 + s(40)));

    // What `autoplay_loot` does with a walk given up: the body is set
    // aside, not written off, and neither the looting nor the next
    // fight waits on it meanwhile.
    let mut ap = Autoplay::default();
    let (me, at) = (glam::Vec3::ZERO, glam::Vec3::new(12.0, 0.0, 0.0));
    let room = Room::PLENTY;
    assert!(ap.corpse_owed(guid, at, me, gave_up, room));
    ap.set_aside_out_of_reach(guid, gave_up);
    assert!(!ap.looted.contains(&guid), "written off for good");
    assert!(!ap.corpse_owed(guid, at, me, gave_up, room), "still owed");
    assert!(
        ap.corpse_owed(guid, at, me, gave_up + s(60), room),
        "never tried again"
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
    c.autoplay.corpse_seen = vec![(body, now)];
    // Walking to it, seven metres off, and a mate says it claimed
    // the same body well before we did.
    c.autoplay.walking_to = Some(CorpseWalk::new(body, 7.0, now));
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
