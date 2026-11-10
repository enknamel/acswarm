use super::*;
use crate::testkit::{self, ME};

/// The Holtburg Dungeon's bookcase room, 0x01F60233: open floor, with the bookcases' narrow
/// gaps a few metres south-west of where the character is set down.
const ROOM: Vec3 = Vec3::new(36.0, -43.5, 0.0);

/// A character of level 30 in the open middle of the bookcase room.
fn in_the_bookcase_room() -> Client {
    let mut c = testkit::standing_in_the_field(30, 0x01F6_0233, ROOM);
    let at = c.my_position().unwrap();
    let (at, cell) = crate::walk_trial::on_floor(&mut c, at);
    testkit::stand(&mut c, cell, at - ac_world::landblock_origin(cell));
    c
}

/// Creatures `guids` coming at the character, each `off` it (metres, world axes).
fn coming_at(c: &mut Client, round: &[(u32, Vec3)]) {
    let pl = c.player.as_ref().unwrap();
    let (cell, me) = (pl.cell, pl.world_position());
    for &(guid, off) in round {
        let mut o = testkit::standing_by(c, guid, "Revenant", 0.0);
        o.position = Some(ac_world::object::Position::new_flat(
            cell,
            me + off - ac_world::landblock_origin(cell),
        ));
        o.walked_at = Some(ME);
        c.world.objects.insert(guid, o);
    }
}

/// Three Revenants a swing off, east, north-west and south-west.
fn three_round(c: &mut Client) {
    coming_at(
        c,
        &[
            (0x8000_0001, Vec3::new(1.5, 0.0, 0.0)),
            (0x8000_0002, Vec3::new(-0.8, 1.3, 0.0)),
            (0x8000_0003, Vec3::new(-0.8, -1.3, 0.0)),
        ],
    );
}

/// Sides open at `at` (see [`Client::autoplay_take_stand`]).
fn sides_at(c: &mut Client, at: Place) -> u32 {
    assert!(c.standable_near(at.at, 1.0).is_some(), "the region built");
    let cap = c.player.as_ref().unwrap().capsule();
    c.pathfinder.open_sides(&[at], RING, SIDES, cap).unwrap()[0].count_ones()
}

/// Ask for the stand until the region round it is built, as a live fight asks each tick.
fn take_stand(c: &mut Client, target: Option<u32>, how: How, now: Instant) -> Did {
    for _ in 0..20 {
        let did = c.autoplay_take_stand(target, how, now);
        if c.autoplay.stands.held.is_some() || did != Did::Done {
            return did;
        }
    }
    Did::Done
}

fn here(c: &Client) -> Place {
    let pl = c.player.as_ref().unwrap();
    Place {
        at: pl.world_position(),
        cell: pl.cell,
    }
}

#[test]
fn a_spot_is_walked_to_only_for_fewer_in_reach_or_two_sides_fewer() {
    // Three swinging: a wall at the back (5 open) leaves all three in reach, but closes 3 sides.
    assert!(is_material(exposure(0b1_1111, 3), exposure(0xFF, 3)));
    // A gap with two sides open lets two of three swing at once.
    assert!(is_material(exposure(0b11, 3), exposure(0b1_1111, 3)));
    // One side fewer, as many in reach: not worth the walk.
    assert!(!is_material(exposure(0b111_1111, 3), exposure(0xFF, 3)));
    // Two swinging have two sides wherever there are two: closing sides past that is the tie-break.
    assert_eq!(exposure(0b111, 2), (2, 3));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn three_creatures_round_a_character_in_an_open_room_send_it_to_a_corner() {
    let mut c = in_the_bookcase_room();
    assert!(c.underground());
    let start = here(&c);
    let open_here = sides_at(&mut c, start);
    assert_eq!(open_here, 8, "the case: open floor all round");
    three_round(&mut c);
    let now = Instant::now();
    c.frame_at = now;
    assert_eq!(take_stand(&mut c, None, How::Melee, now), Did::Acting);
    let stand = c.autoplay.stands.held.expect("a stand chosen");
    assert!(!stand.reached);
    let sides = sides_at(&mut c, stand.spot);
    assert_eq!(sides, stand.sides);
    // A corner of the bookcases: three sides open, where the open floor had all eight.
    assert!(
        sides <= 3,
        "{sides} sides open at {:?}",
        stand.spot.at - ac_world::landblock_origin(0x01F6_0000)
    );
    assert!(stand.spot.at.distance(start.at) <= LOOK_WITHIN + 0.5);
    assert_eq!(
        c.going.as_ref().and_then(|g| g.goal.place()),
        Some(stand.spot),
        "walked there by the slot, planned first"
    );
    // Asked again with the same three, nothing is chosen again.
    assert_eq!(take_stand(&mut c, None, How::Melee, now), Did::Acting);
    assert_eq!(c.autoplay.stands.held, Some(stand));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn one_creature_is_fought_where_the_character_stands() {
    let mut c = in_the_bookcase_room();
    coming_at(&mut c, &[(0x8000_0001, Vec3::new(1.5, 0.0, 0.0))]);
    let now = Instant::now();
    assert_eq!(take_stand(&mut c, None, How::Melee, now), Did::Done);
    assert!(c.autoplay.stands.held.is_none());
    assert!(c.going.is_none(), "no walk");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_stand_where_the_character_stands_holds_no_swing_back() {
    // In the bookcase corner already, nothing near is worth the walk: the swing closes as before.
    let mut c = in_the_bookcase_room();
    let corner = ac_world::landblock_origin(0x01F6_0000) + Vec3::new(33.02, -46.48, 0.0);
    let (at, cell) = crate::walk_trial::on_floor(&mut c, corner);
    testkit::stand(&mut c, cell, at - ac_world::landblock_origin(cell));
    coming_at(
        &mut c,
        &[
            (0x8000_0001, Vec3::new(1.5, 1.0, 0.0)),
            (0x8000_0002, Vec3::new(5.0, 1.0, 0.0)),
        ],
    );
    let now = Instant::now();
    assert_eq!(take_stand(&mut c, None, How::Melee, now), Did::Done);
    let stand = c.autoplay.stands.held.expect("a choice");
    assert!(stand.reached && !stand.walked, "{stand:?}");
    assert!(!c.holds_stand_against(0x8000_0002, now));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn three_creatures_on_open_ground_outside_are_fought_where_they_are() {
    let mut c = testkit::standing_in_the_field(30, 0xA9B4_0019, Vec3::new(84.0, 108.0, 94.0));
    three_round(&mut c);
    assert_eq!(
        take_stand(&mut c, None, How::Melee, Instant::now()),
        Did::Done
    );
    assert!(c.going.is_none());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_casters_stand_keeps_its_target_in_sight() {
    let mut c = in_the_bookcase_room();
    three_round(&mut c);
    let (spell, _) = c
        .assets
        .spell_table()
        .unwrap()
        .find_by_name("Frost Bolt III")
        .expect("Frost Bolt III");
    let how = How::Spell(spell);
    let now = Instant::now();
    c.frame_at = now;
    let target = 0x8000_0001;
    let _ = take_stand(&mut c, Some(target), how, now);
    let stand = c.autoplay.stands.held.expect("a stand chosen");
    assert!(!stand.reached, "walked to: {stand:?}");
    assert!(c.throws_from(stand.spot.at, target, how), "{stand:?}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_stand_walked_off_for_a_shot_is_not_walked_back_to() {
    // Blargerton flipped every tick between "taking a stand" and "getting K'nath Ban'ry in sight".
    let mut c = in_the_bookcase_room();
    three_round(&mut c);
    let (spell, _) = c
        .assets
        .spell_table()
        .unwrap()
        .find_by_name("Frost Bolt III")
        .expect("Frost Bolt III");
    let how = How::Spell(spell);
    let now = Instant::now();
    c.frame_at = now;
    let target = 0x8000_0001;
    let _ = take_stand(&mut c, Some(target), how, now);
    let stand = c.autoplay.stands.held.expect("a stand chosen");
    // Stood at it, the target moves off where no shot from it gets (up through the ceiling).
    let me = here(&c);
    c.autoplay.stands.held = Some(Stand {
        spot: me,
        reached: true,
        ..stand
    });
    let o = c.world.objects.get_mut(&target).unwrap();
    o.position = Some(ac_world::object::Position::new_flat(
        me.cell,
        me.at + Vec3::new(0.0, 0.0, 20.0) - ac_world::landblock_origin(me.cell),
    ));
    let _ = c.autoplay_approach(target, "Revenant", how);
    assert!(
        c.autoplay.stands.held.is_some_and(|s| s.left),
        "the stand still held"
    );
    assert_eq!(c.autoplay.stands.refused, vec![me.at]);
    // Asked again where it stands, nothing walks it back or is chosen afresh: the approach has
    // the legs. Each approach tick refused the spot it stood on again, and chose a stand anew.
    for _ in 0..3 {
        assert_eq!(take_stand(&mut c, Some(target), how, now), Did::Done);
        let _ = c.autoplay_approach(target, "Revenant", how);
    }
    assert!(c
        .autoplay
        .stands
        .held
        .is_some_and(|s| s.left && s.spot == me));
    assert_eq!(
        c.autoplay.stands.refused.len(),
        1,
        "refused again each tick"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_followers_stand_keeps_by_its_leader() {
    // The leader two metres north-east, its fight radius three: the bookcase corner, five metres
    // off it, is not a follower's to walk to.
    let mut c = in_the_bookcase_room();
    let me = c.my_position().unwrap();
    c.autoplay.config.team.enabled = true;
    c.autoplay.config.team.follow = true;
    c.autoplay.config.team.fight_radius = 3.0;
    let mut boss = testkit::mate(0x5000_0002, "Fleetbot One");
    boss.leader = true;
    boss.leads = true;
    let leader = me + Vec3::new(1.5, 1.5, 0.0);
    boss.world = leader;
    c.autoplay.team = testkit::view_of(vec![boss]);
    three_round(&mut c);
    let now = Instant::now();
    c.frame_at = now;
    let _ = take_stand(&mut c, None, How::Melee, now);
    let stand = c.autoplay.stands.held.expect("a choice");
    assert!(
        stand.spot.at.distance(leader) <= 3.0,
        "{:?}, {:.1} m from the leader",
        stand.spot.at - ac_world::landblock_origin(0x01F6_0000),
        stand.spot.at.distance(leader)
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_refused_spot_is_passed_for_the_next() {
    let mut c = in_the_bookcase_room();
    three_round(&mut c);
    let now = Instant::now();
    c.frame_at = now;
    let _ = take_stand(&mut c, None, How::Melee, now);
    let first = c.autoplay.stands.held.expect("a stand").spot;
    c.frame_at = now + crate::GIVE_UP_AFTER + Duration::from_secs(1);
    let later = c.frame_at;
    assert_eq!(c.autoplay_take_stand(None, How::Melee, later), Did::Done);
    assert_eq!(c.autoplay.stands.refused, vec![first.at]);
    let _ = take_stand(&mut c, None, How::Melee, later);
    let next = c.autoplay.stands.held.expect("another choice");
    assert!(
        next.spot.at.distance(first.at) > REPLAN_DISTANCE,
        "{next:?}"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn autoplay_and_the_body_take_the_stand_once_and_fight_from_it() {
    // The whole loop offline at 20 Hz. The fight walks to the stand and swings from it at the
    // creature in reach; offline nothing is hurt, so that one is given up (`STALL_AFTER`), the
    // stand is held for one still coming, and once held long enough the swing closes on it.
    let mut c = in_the_bookcase_room();
    c.autoplay.config.enabled = true;
    c.autoplay.config.fight.enabled = true;
    let start = here(&c);
    three_round(&mut c);
    let dt = 0.05;
    let t0 = Instant::now();
    let mut chosen: Vec<Place> = Vec::new();
    let (mut reached_at, mut swung_from_it, mut held_said) = (None, false, false);
    for frame in 0..40 * 20 {
        let now = t0 + Duration::from_secs_f32(frame as f32 * dt);
        c.frame_at = now;
        c.tick_autoplay(now);
        c.tick_player(crate::player::Input::default(), dt, now);
        let me = c.my_position().unwrap();
        if let Some(s) = c.autoplay.stands.held {
            if chosen.last() != Some(&s.spot) {
                chosen.push(s.spot);
            }
            if s.reached && reached_at.is_none() {
                reached_at = Some((frame, me));
            }
        }
        swung_from_it |= reached_at.is_some()
            && c.attack_target.is_some()
            && chosen.last().is_some_and(|s| s.at.distance(me) <= 1.0);
        held_said |= c.autoplay.status.starts_with("holding the stand");
    }
    let origin = ac_world::landblock_origin(0x01F6_0000);
    let walked: Vec<Vec3> = chosen.iter().map(|p| p.at - origin).collect();
    assert_eq!(chosen.len(), 1, "one stand, not hopping: {walked:?}");
    let (frame, at) = reached_at.expect("the stand reached");
    assert!(frame < 5 * 20, "{} s to get there", frame / 20);
    assert!(at.distance(chosen[0].at) <= AT_STAND + 0.2);
    assert!(at.distance(start.at) > 1.0, "it moved");
    assert!(swung_from_it, "no swing from the stand");
    assert!(held_said, "the stand held for a creature coming");
    assert!(
        testkit::went_for(&c).is_some(),
        "fighting at the end: {}",
        c.autoplay.status
    );
}

/// A war mage in the Holtburg field, Frost Bolt III in hand, a Revenant `off` it coming at it.
fn a_caster_with_one_coming(off: Vec3) -> (Client, How, u32) {
    let mut c = testkit::standing_in_the_field(30, 0xA9B4_0019, Vec3::new(84.0, 108.0, 94.0));
    let at = c.my_position().unwrap();
    let (at, cell) = crate::walk_trial::on_floor(&mut c, at);
    testkit::stand(&mut c, cell, at - ac_world::landblock_origin(cell));
    testkit::trained_in(&mut c, ac_world::stats::skill::WAR_MAGIC, 200);
    let (spell, _) = c
        .assets
        .spell_table()
        .unwrap()
        .find_by_name("Frost Bolt III")
        .expect("Frost Bolt III");
    let target = 0x8000_0001;
    coming_at(&mut c, &[(target, off)]);
    (c, How::Spell(spell), target)
}

/// Ask for the back-off until the region round it is built, as a live fight asks each tick.
fn keep_off(c: &mut Client, target: u32, how: How, now: Instant) -> Did {
    for _ in 0..20 {
        let did = c.autoplay_keep_off(target, how, now);
        if did != Did::Done || c.autoplay.keep_off.last.is_some() {
            return did;
        }
    }
    Did::Done
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_caster_backs_off_from_a_creature_beside_it_to_a_few_metres() {
    let (mut c, how, target) = a_caster_with_one_coming(Vec3::new(2.0, 0.0, 0.0));
    let now = Instant::now();
    c.frame_at = now;
    assert_eq!(keep_off(&mut c, target, how, now), Did::Acting);
    let (spot, _) = c.autoplay.keep_off.walk.expect("a spot backed off to");
    let creature = c.world.objects[&target].world_pos().unwrap();
    let off = spot.at.distance(creature);
    assert!(
        (STANDOFF_NEAR..=STANDOFF_FAR).contains(&off),
        "{off:.1} m from it"
    );
    assert!(
        c.throws_from(spot.at, target, how),
        "the bolt still gets there"
    );
    assert_eq!(
        c.going.as_ref().and_then(|g| g.goal.place()),
        Some(spot),
        "walked there by the slot"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_swinging_fighter_and_a_caster_already_off_keep_where_they_are() {
    let (mut c, _, target) = a_caster_with_one_coming(Vec3::new(2.0, 0.0, 0.0));
    let now = Instant::now();
    assert_eq!(keep_off(&mut c, target, How::Melee, now), Did::Done);
    assert!(c.going.is_none() && c.autoplay.keep_off.walk.is_none());
    let (mut c, how, target) = a_caster_with_one_coming(Vec3::new(7.0, 0.0, 0.0));
    assert_eq!(keep_off(&mut c, target, how, now), Did::Done);
    assert!(c.going.is_none() && c.autoplay.keep_off.walk.is_none());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_caster_steps_from_side_to_side_between_casts_and_only_then() {
    // Seven metres off its target, each step aside goes across the line to it and back, within a
    // step of where it stood: never out of the band, never off its spot.
    let (mut c, how, target) = a_caster_with_one_coming(Vec3::new(7.0, 0.0, 0.0));
    let start = c.my_position().unwrap();
    let creature = c.world.objects[&target].world_pos().unwrap();
    let t0 = Instant::now();
    c.frame_at = t0;
    assert_eq!(c.autoplay_jiggle(target, how, t0), Did::Done, "no cast yet");
    assert!(c.going.is_none());
    let dt = 0.05;
    let mut frame = 0;
    let mut ends = Vec::new();
    for _ in 0..3 {
        // A cast went up and was answered.
        c.autoplay.keep_off.due = true;
        let mut stepped = false;
        for _ in 0..40 {
            let now = t0 + Duration::from_secs_f32(frame as f32 * dt);
            c.frame_at = now;
            c.autoplay.tick += 1;
            let did = c.autoplay_jiggle(target, how, now);
            c.settle_going();
            c.tick_player(crate::player::Input::default(), dt, now);
            frame += 1;
            if did == Did::Done && (stepped || !c.autoplay.keep_off.due) {
                break;
            }
            stepped = true;
        }
        assert!(stepped, "a step aside");
        assert!(c.autoplay.keep_off.step.is_none(), "the step ended");
        ends.push(c.my_position().unwrap());
    }
    for (i, at) in ends.iter().enumerate() {
        assert!(
            at.truncate().distance(start.truncate()) <= JIGGLE + AT_JIGGLE + 0.3,
            "step {i} wandered {:.1} m",
            at.distance(start)
        );
        let off = at.distance(creature);
        assert!((6.5..=7.6).contains(&off), "step {i} {off:.1} m off it");
    }
    // Across the line to it, one side and then the other.
    let across = |at: Vec3| (at - start).y;
    assert!(across(ends[0]) * across(ends[1]) < 0.0, "{ends:?}");
    assert!(across(ends[1]) * across(ends[2]) < 0.0, "{ends:?}");
    assert!(ends[0].distance(ends[1]) >= 1.5, "{ends:?}");
    // With no cast since, no step.
    let now = t0 + Duration::from_secs_f32(frame as f32 * dt);
    assert_eq!(c.autoplay_jiggle(target, how, now), Did::Done);
    assert!(c.autoplay.keep_off.step.is_none());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn autoplay_and_the_body_back_off_and_step_aside_between_casts_never_during_one() {
    // The whole loop offline at 20 Hz, the server's answer to each cast 1.5 s after it went up.
    let t0 = Instant::now();
    let (mut c, _) = testkit::a_caster_knowing(t0, &["Frost Bolt III"]);
    let field = ac_world::landblock_origin(0xA9B4_0000) + Vec3::new(84.0, 108.0, 94.0);
    let (at, cell) = crate::walk_trial::on_floor(&mut c, field);
    testkit::stand(&mut c, cell, at - ac_world::landblock_origin(cell));
    c.autoplay.config.enabled = true;
    c.autoplay.config.fight.enabled = true;
    let target = 0x8000_0001;
    coming_at(&mut c, &[(target, Vec3::new(2.0, 0.0, 0.0))]);
    let creature = c.world.objects[&target].world_pos().unwrap();
    let dt = 0.05;
    let mut casts: Vec<Vec3> = Vec::new();
    let (mut moved_mid_cast, mut was) = (0.0f32, c.my_position().unwrap());
    for frame in 0..12 * 20 {
        let now = t0 + Duration::from_secs_f32(frame as f32 * dt);
        c.frame_at = now;
        if c.autoplay
            .cast_sent
            .is_some_and(|t| now.duration_since(t) >= Duration::from_millis(1500))
        {
            // The bolt landed: offline nothing is hurt, and a target unhurt is closed on.
            c.autoplay.cast_answered();
            let o = c.world.objects.get_mut(&target).unwrap();
            o.health = o.health.map(|h| h - 0.01);
        }
        let before = c.autoplay.cast_sent;
        c.tick_autoplay(now);
        c.tick_player(crate::player::Input::default(), dt, now);
        let me = c.my_position().unwrap();
        if c.autoplay.cast_sent.is_some() && before.is_some() {
            moved_mid_cast = moved_mid_cast.max(me.distance(was));
        }
        if c.autoplay.cast_sent.is_some() && c.autoplay.cast_sent != before {
            casts.push(me);
        }
        was = me;
    }
    assert!(
        casts.len() >= 4,
        "{} casts: {}",
        casts.len(),
        c.autoplay.status
    );
    let off: Vec<f32> = casts.iter().map(|m| m.distance(creature)).collect();
    assert!(
        off.iter()
            .all(|d| (STANDOFF_NEAR - 0.5..=STANDOFF_FAR + 0.5).contains(d)),
        "cast from {off:?} m off it"
    );
    let steps: Vec<f32> = casts.windows(2).map(|w| w[0].distance(w[1])).collect();
    assert!(
        steps.iter().all(|s| *s >= 1.0),
        "between casts it moved {steps:?} m"
    );
    assert!(
        moved_mid_cast < 0.2,
        "moved {moved_mid_cast:.2} m in a frame while casting"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_held_cast_still_takes_its_stand() {
    // Mana under its slider and nothing to restore it: the bolt waits, the back-off does not.
    let t0 = Instant::now();
    let (mut c, _) = testkit::a_caster_knowing(t0, &["Frost Bolt III"]);
    let field = ac_world::landblock_origin(0xA9B4_0000) + Vec3::new(84.0, 108.0, 94.0);
    let (at, cell) = crate::walk_trial::on_floor(&mut c, field);
    testkit::stand(&mut c, cell, at - ac_world::landblock_origin(cell));
    c.autoplay.config.enabled = true;
    c.autoplay.config.fight.enabled = true;
    c.world.stats.vitals[2].base = 1000;
    c.world.stats.vitals[2].current = 100;
    let target = 0x8000_0001;
    coming_at(&mut c, &[(target, Vec3::new(2.0, 0.0, 0.0))]);
    let creature = c.world.objects[&target].world_pos().unwrap();
    let dt = 0.05;
    for frame in 0..4 * 20 {
        let now = t0 + Duration::from_secs_f32(frame as f32 * dt);
        c.frame_at = now;
        c.tick_autoplay(now);
        c.tick_player(crate::player::Input::default(), dt, now);
    }
    assert!(
        c.autoplay.cast_sent.is_none(),
        "a bolt went under the mana slider"
    );
    let off = c.my_position().unwrap().distance(creature);
    assert!(
        off >= STANDOFF_NEAR - 0.5,
        "held where it stood, {off:.1} m off: {}",
        c.autoplay.status
    );
}

/// Virindi Inquisitor: Cassius' Ring of Fire, 4.5 m at 2 m/s.
const INQUISITOR: u32 = 10_814;

/// Put creature `guid` at `at`, a ring caster starting a cast and walking at nobody.
fn ring_caster_at(c: &mut Client, guid: u32, at: Vec3) {
    let cell = c.player.as_ref().unwrap().cell;
    let mut o = testkit::standing_by(c, guid, "Virindi Inquisitor", 0.0);
    o.weenie_class_id = INQUISITOR;
    o.position = Some(ac_world::object::Position::new_flat(
        cell,
        at - ac_world::landblock_origin(cell),
    ));
    o.motion.forward = 0xD3;
    c.world.objects.insert(guid, o);
}

/// Flat metres from `a` to `b`.
fn flat(a: Vec3, b: Vec3) -> f32 {
    a.truncate().distance(b.truncate())
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_caster_backs_off_from_a_ring_caster_to_past_its_ring() {
    let (mut c, how, target) = a_caster_with_one_coming(Vec3::new(2.0, 0.0, 0.0));
    c.world.objects.get_mut(&target).unwrap().weenie_class_id = INQUISITOR;
    let now = Instant::now();
    c.frame_at = now;
    assert_eq!(keep_off(&mut c, target, how, now), Did::Acting);
    let (spot, _) = c.autoplay.keep_off.walk.expect("a spot backed off to");
    let off = flat(spot.at, c.world.objects[&target].world_pos().unwrap());
    let reach = 4.5 + crate::dodge::rings::RING_MARGIN;
    assert!(
        (reach..=reach + PAST + STANDOFF_FAR - STANDOFF_NEAR).contains(&off),
        "{off:.1} m from it, its ring reaching {reach} m"
    );
    assert!(c.throws_from(spot.at, target, how));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn no_back_off_or_step_aside_goes_back_under_a_ring_while_it_spreads() {
    let (mut c, how, target) = a_caster_with_one_coming(Vec3::new(2.0, 0.0, 0.0));
    let now = Instant::now();
    c.frame_at = now;
    let me = c.my_position().unwrap();
    assert_eq!(keep_off(&mut c, target, how, now), Did::Acting);
    let (half_walked, _) = c.autoplay.keep_off.walk.expect("a back-off");
    // An Inquisitor two metres past the spot starts a ring over it and over the character.
    let past = half_walked.at + (half_walked.at - me).normalize() * 2.0;
    let inquisitor = 0x8000_0009;
    ring_caster_at(&mut c, inquisitor, past);
    let reach = 4.5 + crate::dodge::rings::RING_MARGIN;
    assert!(flat(me, past) < reach);
    assert!(c.autoplay_dodge(now), "out of the ring");
    let (out, _) = c.dodge_to.expect("a step out");
    assert!(c.autoplay.keep_off.walk.is_none(), "the back-off is let go");
    // Out: the back-off and the steps aside while the ring spreads keep out of it.
    let (at, cell) = crate::walk_trial::on_floor(&mut c, out);
    testkit::stand(&mut c, cell, at - ac_world::landblock_origin(cell));
    c.dodge_to = None;
    let later = now + Duration::from_millis(1_500);
    c.frame_at = later;
    let _ = keep_off(&mut c, target, how, later);
    if let Some((spot, _)) = c.autoplay.keep_off.walk {
        assert!(flat(spot.at, past) >= reach, "backed off under the ring");
    }
    c.autoplay.keep_off.due = true;
    let _ = c.autoplay_jiggle(target, how, later);
    if let Some((spot, _)) = c.autoplay.keep_off.step {
        assert!(flat(spot.at, past) >= reach, "stepped aside under the ring");
    }
    if let Some(goal) = c.going.as_ref().and_then(|g| g.goal.place()) {
        assert!(flat(goal.at, past) >= reach, "walking back under the ring");
    }
}

/// A caster in the bookcase corner, two Revenants coming at it, the stand chosen there.
fn a_caster_in_the_corner() -> (Client, How, u32, Vec3) {
    let mut c = in_the_bookcase_room();
    let corner = ac_world::landblock_origin(0x01F6_0000) + Vec3::new(33.02, -46.48, 0.0);
    let (at, cell) = crate::walk_trial::on_floor(&mut c, corner);
    testkit::stand(&mut c, cell, at - ac_world::landblock_origin(cell));
    testkit::trained_in(&mut c, ac_world::stats::skill::WAR_MAGIC, 200);
    let (spell, _) = c
        .assets
        .spell_table()
        .unwrap()
        .find_by_name("Frost Bolt III")
        .expect("Frost Bolt III");
    let target = 0x8000_0001;
    coming_at(
        &mut c,
        &[
            (target, Vec3::new(3.0, 3.0, 0.0)),
            (0x8000_0002, Vec3::new(5.0, 2.0, 0.0)),
        ],
    );
    (c, How::Spell(spell), target, at)
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_caster_in_a_dungeon_corner_stays_in_it_and_steps_back_to_it() {
    let (mut c, how, target, corner) = a_caster_in_the_corner();
    let t0 = Instant::now();
    c.frame_at = t0;
    assert_eq!(take_stand(&mut c, Some(target), how, t0), Did::Done);
    let stand = c.autoplay.stands.held.expect("a stand");
    assert!(
        stand.reached && !stand.walked,
        "the corner it stands in: {stand:?}"
    );
    assert!(stand.sides < SIDES as u32, "{stand:?}");
    // Not backed off out of it to the open room.
    assert_eq!(keep_off(&mut c, target, how, t0), Did::Done);
    assert!(c.autoplay.keep_off.walk.is_none() && c.going.is_none());
    // Each step aside comes back: never off the corner for good.
    let dt = 0.05;
    let mut frame = 0;
    let mut ends = Vec::new();
    for _ in 0..4 {
        c.autoplay.keep_off.due = true;
        for _ in 0..40 {
            let now = t0 + Duration::from_secs_f32(frame as f32 * dt);
            c.frame_at = now;
            c.autoplay.tick += 1;
            let did = c.autoplay_jiggle(target, how, now);
            c.settle_going();
            c.tick_player(crate::player::Input::default(), dt, now);
            frame += 1;
            if did == Did::Done && !c.autoplay.keep_off.due {
                break;
            }
        }
        ends.push(c.my_position().unwrap());
    }
    let off: Vec<f32> = ends.iter().map(|e| flat(*e, corner)).collect();
    assert!(
        off.iter().all(|d| *d <= JIGGLE + AT_JIGGLE + 0.3),
        "wandered {off:?} m off the corner"
    );
    assert!(off.iter().any(|d| *d > 0.6), "never stepped: {off:?}");
    // Never two steps' ends on the same spot off the corner: out, then back or across.
    for w in ends.windows(2) {
        let stuck = flat(w[0], w[1]) < 0.5 && flat(w[1], corner) > AT_JIGGLE + 0.2;
        assert!(!stuck, "stayed out at {off:?} m off the corner");
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_caster_run_out_of_its_corner_walks_back_once_the_ring_is_past() {
    // Run out of a ring from the corner it held, a caster was left in the open with neither the
    // corner (still marked reached) nor its back-off (the stand still counted as kept).
    let (mut c, how, target, corner) = a_caster_in_the_corner();
    let now = Instant::now();
    c.frame_at = now;
    assert_eq!(take_stand(&mut c, Some(target), how, now), Did::Done);
    let held = c.autoplay.stands.held.expect("the corner held");
    assert!(held.reached && is_kept(&held));
    // Out of the corner, five metres off, with a ring still spreading over it: waited out.
    let off = corner + Vec3::new(-5.0, 0.0, 0.0);
    let (at, cell) = crate::walk_trial::on_floor(&mut c, off);
    testkit::stand(&mut c, cell, at - ac_world::landblock_origin(cell));
    ring_caster_at(&mut c, 0x8000_0009, corner + Vec3::new(1.0, 0.0, 0.0));
    c.watch_ring_casts(now);
    assert!(!c.spreading_rings(at).is_empty(), "the ring not noted");
    assert!(
        c.stand_kept().is_none(),
        "the corner kept from five metres off"
    );
    let _ = take_stand(&mut c, Some(target), how, now);
    assert!(c.going.is_none(), "walked back under the ring");
    // The ring gone: walked back to the corner.
    c.world.objects.remove(&0x8000_0009);
    c.watch_ring_casts(now);
    assert_eq!(take_stand(&mut c, Some(target), how, now), Did::Acting);
    let goal = c
        .going
        .as_ref()
        .and_then(|g| g.goal.place())
        .expect("a walk");
    assert!(
        goal.at.distance(held.spot.at) < 0.1,
        "not back to the corner"
    );
}

#[test]
fn a_back_off_and_step_aside_are_let_go_with_the_target() {
    let mut c = crate::Client::offline(testkit::no_data());
    let spot = Place {
        at: Vec3::new(1.0, 2.0, 3.0),
        cell: 0xA9B4_0019,
    };
    let now = Instant::now();
    let held = KeepOff {
        fought: Some(0x8000_0001),
        walk: Some((spot, now)),
        step: Some((spot, now)),
        due: true,
        ..KeepOff::default()
    };
    // Another target starts afresh.
    c.autoplay.keep_off = held.clone();
    c.keep_off_for(0x8000_0002);
    assert!(c.autoplay.keep_off.walk.is_none() && c.autoplay.keep_off.step.is_none());
    assert!(!c.autoplay.keep_off.due);
    // The same target keeps what it had.
    c.autoplay.keep_off = held.clone();
    c.keep_off_for(0x8000_0001);
    assert!(c.autoplay.keep_off.walk.is_some());
    // The fight let go, nothing is kept.
    c.let_go(crate::autoplay::Release::Fight);
    assert!(c.autoplay.keep_off.fought.is_none() && c.autoplay.keep_off.walk.is_none());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_stale_back_off_ending_leaves_another_walk_alone() {
    let (mut c, how, target) = a_caster_with_one_coming(Vec3::new(7.0, 0.0, 0.0));
    let now = Instant::now();
    c.frame_at = now;
    let me = c.my_position().unwrap();
    let owner = c.step_owner();
    let _ = c.go_to(owner, me + Vec3::new(0.0, 5.0, 0.0), 0.5);
    let walk = c
        .going
        .as_ref()
        .and_then(|g| g.goal.place())
        .expect("a walk");
    let stale = Place {
        at: me + Vec3::new(-3.0, 0.0, 0.0),
        cell: walk.cell,
    };
    c.autoplay.keep_off.fought = Some(target);
    c.autoplay.keep_off.walk = Some((stale, now - Duration::from_secs(30)));
    assert_eq!(c.autoplay_keep_off(target, how, now), Did::Done);
    assert!(c.autoplay.keep_off.walk.is_none());
    assert_eq!(c.going.as_ref().and_then(|g| g.goal.place()), Some(walk));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn autoplay_and_the_body_keep_a_caster_in_its_corner_through_its_casts() {
    // The whole loop offline at 20 Hz, the server's answer to each cast 1.5 s after it went up.
    let t0 = Instant::now();
    let (mut c, _) = testkit::a_caster_knowing(t0, &["Frost Bolt III"]);
    testkit::stand(&mut c, 0x01F6_0233, Vec3::new(33.02, -46.48, 0.0));
    let corner = c.my_position().unwrap();
    let (corner, cell) = crate::walk_trial::on_floor(&mut c, corner);
    testkit::stand(&mut c, cell, corner - ac_world::landblock_origin(cell));
    c.autoplay.config.enabled = true;
    c.autoplay.config.fight.enabled = true;
    let target = 0x8000_0001;
    coming_at(
        &mut c,
        &[
            (target, Vec3::new(3.0, 3.0, 0.0)),
            (0x8000_0002, Vec3::new(5.0, 2.0, 0.0)),
        ],
    );
    let dt = 0.05;
    let (mut casts, mut furthest) = (Vec::new(), 0.0f32);
    for frame in 0..12 * 20 {
        let now = t0 + Duration::from_secs_f32(frame as f32 * dt);
        c.frame_at = now;
        if c.autoplay
            .cast_sent
            .is_some_and(|t| now.duration_since(t) >= Duration::from_millis(1500))
        {
            c.autoplay.cast_answered();
            let o = c.world.objects.get_mut(&target).unwrap();
            o.health = o.health.map(|h| h - 0.01);
        }
        let before = c.autoplay.cast_sent;
        c.tick_autoplay(now);
        c.tick_player(crate::player::Input::default(), dt, now);
        let me = c.my_position().unwrap();
        furthest = furthest.max(flat(me, corner));
        if c.autoplay.cast_sent.is_some() && c.autoplay.cast_sent != before {
            casts.push(me);
        }
    }
    let off: Vec<f32> = casts.iter().map(|m| flat(*m, corner)).collect();
    assert!(off.len() >= 4, "{} casts: {}", off.len(), c.autoplay.status);
    assert!(
        furthest <= JIGGLE + AT_JIGGLE + 0.3,
        "{furthest:.1} m off the corner; cast from {off:?} m off it"
    );
    // Out and back between casts, never stuck out on one spot.
    let steps: Vec<f32> = casts.windows(2).map(|w| flat(w[0], w[1])).collect();
    assert!(
        steps.iter().all(|s| *s >= 0.5),
        "between casts it moved {steps:?} m, cast from {off:?} m off the corner"
    );
}
