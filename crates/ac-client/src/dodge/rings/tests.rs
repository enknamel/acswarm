use super::*;

const FIELD: u32 = 0xA9B4_0019;
/// Virindi Inquisitor: Horizon's Blades and Cassius' Ring of Fire, 4.5 m at 2 m/s.
const INQUISITOR: u32 = 10_814;
/// Patriarch's Inquisitor: Incendiary Ring, 8 m at 2 m/s.
const PATRIARCHS: u32 = 88_305;

/// A character in the open with creature `wcid` `metres` east of it, its forward motion `forward`.
fn beside(wcid: u32, metres: f32, forward: u32) -> (Client, u32) {
    let mut c = crate::testkit::offline_client();
    crate::testkit::stand(&mut c, FIELD, Vec3::new(84.0, 84.0, 94.0));
    let o = crate::testkit::standing_by(&mut c, 0x8000_0001, "Virindi Inquisitor", metres);
    let caster = c.world.objects.get_mut(&o.guid).unwrap();
    caster.weenie_class_id = wcid;
    caster.motion.forward = forward;
    (c, o.guid)
}

fn flat_from(c: &Client, guid: u32, spot: Vec3) -> f32 {
    let at = c.world.objects[&guid].world_pos().unwrap();
    at.truncate().distance(spot.truncate())
}

#[test]
fn a_cast_motion_is_told_from_standing_and_walking() {
    for cast in [
        0xD3,
        0x4000_00D3,
        0x2B,
        0x4000_0039,
        0x6F,
        0x1000_0078,
        0x12B,
        0x134,
    ] {
        assert!(is_cast_motion(cast), "{cast:#x}");
    }
    for other in [0x03, 0x4100_0003, 0x05, 0x07, 0x4400_0007, 0x62, 0xD4] {
        assert!(!is_cast_motion(other), "{other:#x}");
    }
}

#[test]
fn a_ring_beside_us_is_left_at_once_and_a_wide_one_a_few_metres_off_gives_time() {
    let small = Ring {
        spell: 1785,
        range: 4.5,
        speed: 2.0,
    };
    // A metre off, the projectiles are on us as they leave.
    assert!(leave_within(0.0, 1.0, small) < 0.0);
    // An 8 m ring 6 m off: 1.5 s of flight after the windup, less the slack.
    let wide = Ring {
        spell: 3882,
        range: 8.0,
        speed: 2.0,
    };
    let t = leave_within(0.0, 6.0, wide);
    assert!((t - 1.4).abs() < 1e-3, "{t}");
    // A second into the cast, a second less.
    assert!((leave_within(1.0, 6.0, wide) - (t - 1.0)).abs() < 1e-3);
    // Ring of Thorns flies 15 m/s: a run at 4 m/s cannot beat it to its reach, so go now.
    let fast = Ring {
        spell: 6163,
        range: 4.5,
        speed: 15.0,
    };
    assert!(leave_within(0.0, 5.0, fast) < 0.0);
}

#[test]
fn the_spot_is_the_shortest_run_past_the_reach_round_a_wall_or_the_deepest_one_boxed_in() {
    let me = Vec3::new(2.0, 0.0, 10.0);
    let rings = [(Vec2::ZERO, 7.5)];
    // In the open, straight away from the caster: 5.5 m to the reach and half a metre past it.
    let spot = escape_spot(me, &rings, |_| true).unwrap();
    assert!((spot - Vec3::new(8.0, 0.0, 10.0)).length() < 1e-3, "{spot}");
    // A wall a metre east: round it, still out of reach.
    let spot = escape_spot(me, &rings, |to| to.x < me.x + 1.0).unwrap();
    assert!(spot.x < me.x + 1.0);
    assert!(spot.truncate().length() >= 7.5, "{spot}");
    assert_eq!(spot.z, me.z);
    // Boxed in to two metres: as far out as that goes.
    let spot = escape_spot(me, &rings, |to| to.distance(me) <= 2.0).unwrap();
    assert!(spot.distance(me) <= 2.0);
    assert!(spot.truncate().length() > 3.4, "{spot}");
    // Nowhere to go at all.
    assert!(escape_spot(me, &rings, |_| false).is_none());
    // Two rings: out of both.
    let two = [(Vec2::ZERO, 7.5), (Vec2::new(4.0, 6.0), 7.5)];
    let spot = escape_spot(me, &two, |_| true).unwrap();
    assert!(two.iter().all(|(c, r)| spot.truncate().distance(*c) >= *r));
}

#[test]
fn a_ring_caster_starting_a_cast_beside_us_is_run_from_past_its_reach() {
    let now = Instant::now();
    let (mut c, guid) = beside(INQUISITOR, 2.0, 0xD3);
    assert!(c.autoplay_dodge(now));
    let (spot, _) = c.dodge_to.expect("a step out");
    let out = flat_from(&c, guid, spot);
    assert!(out >= 4.5 + RING_MARGIN, "{out:.1} m from the caster");
    assert!(c.autoplay.status.contains("ring"), "{}", c.autoplay.status);
    // One step for one cast.
    c.dodge_to = None;
    assert!(!c.autoplay_dodge(now + Duration::from_millis(100)));
}

#[test]
fn a_ring_casters_next_cast_two_and_a_half_seconds_on_is_run_from_too() {
    // ACE casts again 2-3 s after the last (`Monster_Magic.cs:160-186`), well inside one ring's
    // span; the Ready between the two casts (`PostCastMotion`) tells them apart.
    let t0 = Instant::now();
    let (mut c, guid) = beside(INQUISITOR, 2.0, 0xD3);
    let set =
        |c: &mut Client, forward| c.world.objects.get_mut(&guid).unwrap().motion.forward = forward;
    assert!(c.autoplay_dodge(t0));
    // Still standing 2 m off when it casts again.
    c.dodge_to = None;
    set(&mut c, 0x4100_0003);
    assert!(!c.autoplay_dodge(t0 + Duration::from_millis(1_000)));
    set(&mut c, 0xD3);
    let again = t0 + Duration::from_millis(2_500);
    assert!(c.autoplay_dodge(again), "the second cast was swallowed");
    let (spot, _) = c.dodge_to.expect("a step out");
    assert!(flat_from(&c, guid, spot) >= 4.5 + RING_MARGIN);
    // Its motion held over the next ticks is still the one cast.
    c.dodge_to = None;
    assert!(!c.autoplay_dodge(again + Duration::from_millis(100)));
}

#[test]
fn a_ring_caster_walking_or_already_far_enough_off_is_left_alone() {
    let now = Instant::now();
    let (mut c, _) = beside(INQUISITOR, 2.0, 0x07);
    assert!(!c.autoplay_dodge(now));
    assert!(c.dodge_to.is_none());
    let (mut c, _) = beside(INQUISITOR, 8.0, 0xD3);
    assert!(!c.autoplay_dodge(now));
    assert!(c.dodge_to.is_none());
    // A Drudge Skulker casting has no ring.
    let (mut c, _) = beside(8592, 2.0, 0xD3);
    assert!(!c.autoplay_dodge(now));
    assert!(c.dodge_to.is_none());
}

#[test]
fn a_cast_of_ours_that_lands_first_is_let_land_and_one_that_would_not_is_left() {
    let now = Instant::now();
    // An 8 m ring 7 m off leaves 1.9 s; our cast, sent a second ago, is answered in one.
    let (mut c, guid) = beside(PATRIARCHS, 7.0, 0xD3);
    c.autoplay.cast_sent = Some(now - Duration::from_secs(1));
    assert!(!c.autoplay_dodge(now));
    assert!(c.dodge_to.is_none());
    // Its answer in, the step is taken.
    c.autoplay.cast_sent = None;
    let later = now + Duration::from_millis(1_000);
    assert!(c.autoplay_dodge(later));
    let (spot, _) = c.dodge_to.unwrap();
    assert!(flat_from(&c, guid, spot) >= 8.0 + RING_MARGIN);
    // A ring beside us lands before a cast just sent: the cast is left.
    let (mut c, _) = beside(INQUISITOR, 1.0, 0xD3);
    c.autoplay.cast_sent = Some(now);
    assert!(c.autoplay_dodge(now));
    assert!(c.dodge_to.is_some());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn autoplay_and_the_body_run_a_fighting_caster_out_of_a_ring_before_it_lands() {
    // The whole loop at 20 Hz in the Holtburg field: an Inquisitor 2 m off starts a cast 2 s in,
    // its ring leaving 0.9 s later (most stances' windup) at 2 m/s out to 4.5 m.
    let t0 = Instant::now();
    let (mut c, _) = crate::testkit::a_caster_knowing(t0, &["Frost Bolt III"]);
    let field = ac_world::landblock_origin(0xA9B4_0000) + Vec3::new(84.0, 108.0, 94.0);
    let (at, cell) = crate::walk_trial::on_floor(&mut c, field);
    crate::testkit::stand(&mut c, cell, at - ac_world::landblock_origin(cell));
    c.autoplay.config.enabled = true;
    c.autoplay.config.fight.enabled = true;
    let guid = 0x8000_0001;
    let mut o = crate::testkit::standing_by(&mut c, guid, "Virindi Inquisitor", 2.0);
    o.weenie_class_id = INQUISITOR;
    o.walked_at = Some(crate::testkit::ME);
    c.world.objects.insert(guid, o);
    let caster = c.world.objects[&guid].world_pos().unwrap().truncate();
    let dt = 0.05;
    let (cast_at, launch_at) = (2.0, 2.9);
    let mut worst = f32::INFINITY;
    for frame in 0..8 * 20 {
        let t = frame as f32 * dt;
        let now = t0 + Duration::from_secs_f32(t);
        c.frame_at = now;
        if c.autoplay
            .cast_sent
            .is_some_and(|s| now.duration_since(s) >= Duration::from_millis(1500))
        {
            c.autoplay.cast_answered();
            let o = c.world.objects.get_mut(&guid).unwrap();
            o.health = o.health.map(|h| h - 0.01);
        }
        let forward = if (cast_at..launch_at).contains(&t) {
            0xD3
        } else {
            0x4100_0003
        };
        c.world.objects.get_mut(&guid).unwrap().motion.forward = forward;
        c.tick_autoplay(now);
        c.tick_player(crate::player::Input::default(), dt, now);
        // The ring's front from its launch: spawned about 1.8 m out, its projectiles 0.5 m round
        // and our capsule 0.4 m, out to 4.5 m of flight.
        let flown = ((t - launch_at) * 2.0).clamp(0.0, 4.5);
        if t >= launch_at && (t - launch_at) * 2.0 <= 4.5 {
            let me = c.my_position().unwrap().truncate();
            worst = worst.min(me.distance(caster) - (1.8 + 0.9 + flown));
        }
    }
    assert!(
        worst > 0.0,
        "the ring's front came within {:.2} m: {}",
        -worst,
        c.autoplay.status
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn autoplay_and_the_body_run_a_caster_out_of_two_rings_cast_two_and_a_half_seconds_apart() {
    // The whole loop at 20 Hz in the Holtburg field: an Inquisitor 2 m off casts at 0.5 s, closes
    // to 2 m again and casts at 3 s, each ring leaving 0.9 s on at 2 m/s out to 4.5 m.
    let t0 = Instant::now();
    let (mut c, _) = crate::testkit::a_caster_knowing(t0, &["Frost Bolt III"]);
    let field = ac_world::landblock_origin(0xA9B4_0000) + Vec3::new(84.0, 108.0, 94.0);
    let (at, cell) = crate::walk_trial::on_floor(&mut c, field);
    crate::testkit::stand(&mut c, cell, at - ac_world::landblock_origin(cell));
    c.autoplay.config.enabled = true;
    c.autoplay.config.fight.enabled = true;
    let guid = 0x8000_0001;
    let mut o = crate::testkit::standing_by(&mut c, guid, "Virindi Inquisitor", 2.0);
    o.weenie_class_id = INQUISITOR;
    o.walked_at = Some(crate::testkit::ME);
    c.world.objects.insert(guid, o);
    let dt = 0.05;
    let casts = [(0.5, 1.4), (3.0, 3.9)];
    let mut worst = [f32::INFINITY; 2];
    let mut centre: [Option<glam::Vec2>; 2] = [None; 2];
    let mut closed = false;
    for frame in 0..7 * 20 {
        let t = frame as f32 * dt;
        let now = t0 + Duration::from_secs_f32(t);
        c.frame_at = now;
        if c.autoplay
            .cast_sent
            .is_some_and(|s| now.duration_since(s) >= Duration::from_millis(1500))
        {
            c.autoplay.cast_answered();
            let o = c.world.objects.get_mut(&guid).unwrap();
            o.health = o.health.map(|h| h - 0.01);
        }
        if t >= 2.95 && !closed {
            // It walks up to us again.
            closed = true;
            let pl = c.player.as_ref().unwrap();
            let (cell, me) = (pl.cell, pl.world_position());
            let o = c.world.objects.get_mut(&guid).unwrap();
            let from = o.world_pos().unwrap();
            let to = me + (from - me).with_z(0.0).normalize() * 2.0;
            o.position = Some(ac_world::object::Position::new_flat(
                cell,
                to - ac_world::landblock_origin(cell),
            ));
        }
        let casting = casts.iter().any(|(s, l)| (*s..*l).contains(&t));
        let forward = if casting { 0xD3 } else { 0x4100_0003 };
        c.world.objects.get_mut(&guid).unwrap().motion.forward = forward;
        c.tick_autoplay(now);
        c.tick_player(crate::player::Input::default(), dt, now);
        let caster = c.world.objects[&guid].world_pos().unwrap().truncate();
        let me = c.my_position().unwrap().truncate();
        for (i, (_, launch)) in casts.iter().enumerate() {
            // Each ring spreads from where its caster stood at its launch.
            let flown = (t - launch) * 2.0;
            let from = *centre[i].get_or_insert(caster);
            if !(0.0..=4.5).contains(&flown) {
                centre[i] = None;
                continue;
            }
            worst[i] = worst[i].min(me.distance(from) - (1.8 + 0.9 + flown));
        }
    }
    assert!(
        worst.iter().all(|w| *w > 0.0),
        "the rings' fronts came within {worst:?} m: {}",
        c.autoplay.status
    );
}
