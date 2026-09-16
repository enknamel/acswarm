use super::*;
use crate::testkit::{character_of_level, game_data};

#[test]
fn a_body_where_a_kill_fell_is_ours_however_far() {
    let now = Instant::now();
    let spots = vec![(glam::Vec3::new(100.0, 100.0, 50.0), now)];
    // Where it fell, give or take the drift of dying.
    assert!(near_a_kill(glam::Vec3::new(103.0, 98.0, 51.0), &spots));
    // Somebody else's, a street away.
    assert!(!near_a_kill(glam::Vec3::new(120.0, 100.0, 50.0), &spots));
    assert!(!near_a_kill(glam::Vec3::new(100.0, 100.0, 50.0), &[]));
}

#[test]
fn the_fight_waits_only_on_bodies_the_looting_takes() {
    let now = Instant::now();
    let me = glam::Vec3::ZERO;
    let off = glam::Vec3::new(22.0, 0.0, 0.0);
    // Close by: looted, and waited on.
    assert!(corpse_is_ours(
        me,
        glam::Vec3::new(5.0, 0.0, 0.0),
        40.0,
        &[]
    ));
    // Twenty-two metres off where nothing of ours fell: neither. It
    // used to be waited on out to twenty-five and looted only to
    // twenty, and the character stood between the two for good.
    assert!(!corpse_is_ours(me, off, 40.0, &[]));
    // Where a kill of ours fell: both.
    assert!(corpse_is_ours(me, off, 40.0, &[(off, now)]));
    // But not past the fight radius.
    assert!(!corpse_is_ours(me, off, 20.0, &[(off, now)]));

    // Thirty metres off, where the summoned creature landed the last
    // blow: no kill notice came, so no kill spot, and it is not ours
    // as it lies. It is asked about instead.
    let pets = glam::Vec3::new(30.0, 0.0, 0.0);
    assert!(!corpse_is_ours(me, pets, 60.0, &[]));
    assert!(whose_to_ask(me, pets, 60.0, &[]));
    // Its description names the creature, so it is claimed where it
    // lies: looted, and waited on, and not asked about again.
    assert!(killed_by_us(
        "Killed by Blargerton's Mud Golem.",
        "Blargerton"
    ));
    let claimed = [(pets, now)];
    assert!(corpse_is_ours(me, pets, 60.0, &claimed));
    assert!(!whose_to_ask(me, pets, 60.0, &claimed));
    // One close by needs no asking, and one past the fight radius is
    // not asked about.
    assert!(!whose_to_ask(me, glam::Vec3::new(5.0, 0.0, 0.0), 60.0, &[]));
    assert!(!whose_to_ask(
        me,
        glam::Vec3::new(70.0, 0.0, 0.0),
        60.0,
        &[]
    ));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_moment_held_open_for_a_falling_body_ends_when_one_lands() {
    // The next fight is held for three seconds after a killing blow,
    // because the body comes a moment after the creature dies. Held
    // for the whole three regardless of what landed, nine characters
    // killing in one huddle each spent it standing over a body one
    // of the others was already opening: fighting was 13% of that
    // run and opening a corpse 53%.
    let mut c = character_of_level(game_data(), 20);
    let s = Duration::from_secs;
    let t0 = Instant::now();
    let spot = glam::Vec3::new(40.0, 40.0, 10.0);
    let body = |guid: u32, at: glam::Vec3| ac_world::WorldObject {
        guid,
        name: "Corpse of Drudge Slave".into(),
        object_desc_flags: ac_world::object_desc_flags::CORPSE,
        position: Some(ac_world::object::Position::new_flat(0, at)),
        ..Default::default()
    };
    assert!(!c.owes_a_corpse(), "owed a body with no kill behind it");

    // Its own killing blow: the body is owed while it is still
    // falling, and no longer.
    c.autoplay.last_kill = Some(t0);
    c.autoplay.kill_spots = vec![(spot, t0)];
    assert!(c.own_body_still_falling(t0));
    assert!(c.own_body_still_falling(t0 + CORPSE_APPEARS - s(1)));
    assert!(!c.own_body_still_falling(t0 + CORPSE_APPEARS));

    // A body noted before the blow is some other kill's and says
    // nothing about this one, wherever it lies.
    let (early, elsewhere, ours) = (0x8000_0001, 0x8000_0002, 0x8000_0003);
    c.world.objects.insert(early, body(early, spot));
    c.autoplay.corpse_seen = vec![(early, t0 - s(1))];
    assert!(c.own_body_still_falling(t0));

    // Nor does a body that landed since the blow somewhere this
    // character killed nothing. Ending the hold on any corpse at
    // all, a huddle of nine had one come into view every couple of
    // seconds, and each one sent a character off after the next
    // fight leaving the body it had just made behind.
    let away = spot + glam::Vec3::new(KILL_SPOT * 2.0, 0.0, 0.0);
    c.world.objects.insert(elsewhere, body(elsewhere, away));
    c.autoplay.corpse_seen.push((elsewhere, t0));
    assert!(
        c.own_body_still_falling(t0),
        "let go of its own body for somebody else's"
    );

    // Its own, landing where its kill fell, ends the wait: from here
    // the bodies on the floor are the answer, a mate's claim among
    // them.
    c.world.objects.insert(ours, body(ours, spot));
    c.autoplay.corpse_seen.push((ours, t0));
    assert!(!c.own_body_still_falling(t0));
    assert!(!c.owes_a_corpse());
}

#[test]
fn a_far_corpse_is_asked_about_once_and_the_fight_waits_on_the_answer_briefly() {
    let t0 = Instant::now();
    let s = Duration::from_secs;
    let mut whose = Whose::default();
    // No creature seen out: nothing left bodies about.
    assert!(!whose.pet_lately(t0));
    whose.pet_out(t0);
    assert!(whose.pet_lately(t0 + s(30)));
    assert!(!whose.pet_lately(t0 + PET_KILLS_FOR));

    // Asked about: the fight waits a moment for the answer.
    assert!(!whose.waiting(t0));
    whose.ask(0x8000_0001, t0);
    assert!(whose.has_asked(0x8000_0001));
    assert!(whose.waiting(t0 + s(1)));
    // Asking again changes nothing, and does not restart the wait.
    whose.ask(0x8000_0001, t0 + s(2));
    assert_eq!(whose.out().collect::<Vec<_>>(), vec![0x8000_0001]);
    // An answer that never comes does not hold the fight for good.
    assert!(!whose.waiting(t0 + WHOSE_WAIT));

    // Once the answer is in, nothing is waited on or out, and the
    // corpse is still not asked about again.
    whose.ask(0x8000_0002, t0 + s(4));
    assert!(whose.waiting(t0 + s(5)));
    whose.answered(0x8000_0002);
    whose.answered(0x8000_0001);
    assert!(!whose.waiting(t0 + s(5)));
    assert_eq!(whose.out().count(), 0);
    assert!(whose.has_asked(0x8000_0002));

    // A corpse gone from view is forgotten.
    whose.tidy(|g| g == 0x8000_0002);
    assert!(!whose.has_asked(0x8000_0001));
    assert!(whose.has_asked(0x8000_0002));
}

#[test]
fn a_corpse_a_floor_below_is_not_close_however_near_it_lies_on_the_map() {
    // The Holtburg Dungeon's rooms are stacked: a body three metres
    // off on the map and eight metres down is in another room.
    let now = Instant::now();
    let me = glam::Vec3::new(0.0, 0.0, 8.0);
    let below = glam::Vec3::new(3.0, 0.0, 0.0);
    assert!(!corpse_is_ours(me, below, 60.0, &[]));
    // Nor one a floor above.
    assert!(!corpse_is_ours(below, me, 60.0, &[]));
    // On this floor it is, and a step or a doorsill up is still this
    // floor.
    assert!(corpse_is_ours(
        me,
        glam::Vec3::new(3.0, 0.0, 8.0),
        60.0,
        &[]
    ));
    assert!(corpse_is_ours(
        me,
        glam::Vec3::new(3.0, 0.0, 9.5),
        60.0,
        &[]
    ));
    // Where a kill of its own fell it is still its own: whether it
    // can be walked to is for the walk to find out.
    assert!(corpse_is_ours(me, below, 60.0, &[(below, now)]));
}
