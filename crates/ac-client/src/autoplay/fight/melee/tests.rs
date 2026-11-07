use super::*;
use crate::testkit::{in_view, standing_in_the_field};

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_target_left_half_a_world_away_is_not_fought_by_spell_either() {
    // Brynvor, 2026-09-15: teleported from the Holtburg Dungeon to
    // the town above it with a Swamp Rat as the casting target, and
    // stood there ten minutes casting at it from 34 km. The swing
    // already let such a target go; the spell now does too.
    let mut c = standing_in_the_field(999, 0xA9B4_002E, glam::Vec3::new(125.0, 132.0, 67.0));
    let rat = in_view(&mut c, 0x8000_20AD, 0, "Swamp Rat");
    let put = |c: &mut Client, cell: u32, local: glam::Vec3| {
        c.world.objects.get_mut(&rat.guid).unwrap().position = Some(ac_world::Position {
            cell,
            local,
            rotation: glam::Quat::IDENTITY,
        });
    };
    put(&mut c, 0x01F6_01FA, glam::Vec3::new(94.0, -63.0, -6.0));
    assert!(
        c.fight_target_gone(rat.guid, false),
        "not a walk away, so not here"
    );
    put(&mut c, 0xA9B4_002E, glam::Vec3::new(130.0, 132.0, 67.0));
    assert!(
        !c.fight_target_gone(rat.guid, false),
        "beside us, it is a fight"
    );
}

#[test]
fn a_ranged_attacker_closes_in_before_it_gives_up() {
    // Nothing landing from forty metres: try from twenty, then ten,
    // then as near as is worth being -- and only then give up.
    assert_eq!(closer_stand_off(40.0), Some(20.0));
    assert_eq!(closer_stand_off(20.0), Some(10.0));
    assert_eq!(closer_stand_off(10.0), Some(MIN_STAND_OFF));
    assert_eq!(closer_stand_off(MIN_STAND_OFF), None);
    assert_eq!(closer_stand_off(MIN_STAND_OFF + 0.5), None);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_spot_nothing_lands_from_at_arms_length_is_left_for_another() {
    // Blargerton cast twenty Frost Bolts at a Tumerok Standard Bearer 1.8 m off, none landing,
    // gave up and picked it again each time it hit him: nowhere nearer to stand, so he never moved.
    let t0 = Instant::now();
    let (mut c, ids) = crate::testkit::a_caster_knowing(t0, &["Frost Bolt VI"]);
    let foe = crate::testkit::standing_by(&mut c, 0x8000_0103, "Tumerok Standard Bearer", 1.8);
    c.autoplay.casting_at = Some(foe.guid);
    c.autoplay.attack_spell = Some(ids[0]);
    c.autoplay.engaged = Some((foe.guid, t0, 1.0));
    c.autoplay.thrown = Some((foe.guid, t0));
    let me = c.my_position().unwrap();
    let now = t0 + CLOSE_IN_AFTER + Duration::from_secs(1);
    assert!(!c.stalled_on(foe.guid, now), "not given up");
    assert!(c.is_dud_spot(me), "this spot is left for another");
    assert_eq!(c.autoplay.casting_at, Some(foe.guid));
}
