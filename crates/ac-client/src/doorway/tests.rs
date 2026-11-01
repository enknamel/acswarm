use super::*;
use crate::dodge::How;

/// The Holtburg Dungeon's door 0x701F6057 in 0x01F601FD, open, with a caster in the room on its
/// west side, 3.75 m from it. Its doorway runs north-south at local x 94.75.
fn by_the_open_door() -> (Client, u32) {
    let mut c =
        crate::testkit::standing_in_the_field(27, 0x01F6_01FA, Vec3::new(91.0, -60.0, -6.0));
    let door = ac_world::WorldObject {
        guid: 0x701F_6057,
        name: "Door".into(),
        setup_id: 0x0200_024F,
        object_desc_flags: ac_world::object_desc_flags::DOOR,
        physics_state: 0x10018 | ac_world::object::PHYSICS_STATE_ETHEREAL,
        position: Some(ac_world::object::Position {
            cell: 0x01F6_01FD,
            local: Vec3::new(94.75, -60.0, -6.0),
            rotation: glam::Quat::from_xyzw(
                0.0,
                0.0,
                -std::f32::consts::FRAC_1_SQRT_2,
                std::f32::consts::FRAC_1_SQRT_2,
            ),
        }),
        scale: 1.0,
        ..Default::default()
    };
    c.world.objects.insert(door.guid, door);
    let spell = c
        .assets
        .spell_table()
        .unwrap()
        .find_by_name("Shock Wave I")
        .map(|(id, _)| id)
        .expect("Shock Wave I");
    (c, spell)
}

fn west_of_the_door(at: Vec3) -> bool {
    at.x - ac_world::landblock_origin(0x01F6_0000).x < 94.75
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_spell_is_not_thrown_through_an_open_doorway_but_a_swing_goes_through() {
    let (mut c, spell) = by_the_open_door();
    let rat = crate::testkit::standing_by(&mut c, 0x8000_0001, "Swamp Rat", 7.0);
    assert!(
        !c.shot_clears(rat.guid, How::Spell(spell)),
        "through the doorway"
    );
    assert!(c.shot_clears(rat.guid, How::Melee), "a walk goes through");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_target_past_a_doorway_is_shot_at_from_its_own_room() {
    let (mut c, spell) = by_the_open_door();
    let rat = crate::testkit::standing_by(&mut c, 0x8000_0001, "Swamp Rat", 7.0);
    assert!(c.autoplay_approach(rat.guid, "Swamp Rat", How::Spell(spell)));
    let spot = c
        .dodge
        .vantage
        .and_then(|(_, _, s)| s)
        .expect("somewhere to throw from");
    assert!(!west_of_the_door(spot), "in the rat's room: {spot:?}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_target_in_a_doorway_is_waited_for_out_of_it() {
    let (mut c, spell) = by_the_open_door();
    let rat = crate::testkit::standing_by(&mut c, 0x8000_0001, "Swamp Rat", 3.75);
    let how = How::Spell(spell);
    assert!(!c.shot_clears(rat.guid, how), "it stands in the frame");
    assert!(
        c.autoplay_approach(rat.guid, "Swamp Rat", how),
        "waiting for it"
    );
    let (_, _, spot) = c.dodge.pulling.expect("waiting for it");
    let spot = spot.expect("a spot to wait at");
    let door = ac_world::landblock_origin(0x01F6_0000) + Vec3::new(94.75, -60.0, -6.0);
    assert!(west_of_the_door(spot), "on our own side: {spot:?}");
    assert!(
        spot.distance(door) >= PULL_BACK - 0.01,
        "back from it: {spot:?}"
    );
    assert_eq!(c.follow.map(|f| f.target), Some(spot), "walking there");
    // Come through to our side, it is shot at.
    let me = c.my_position().unwrap();
    let out = me + Vec3::new(1.5, 0.0, 0.0) - ac_world::landblock_origin(0x01F6_01FA);
    c.world.objects.get_mut(&rat.guid).unwrap().position =
        Some(ac_world::object::Position::new_flat(0x01F6_01FA, out));
    assert!(c.shot_clears(rat.guid, how), "out of the doorway");
    assert!(!c.pull_from_doorway(rat.guid, "Swamp Rat", me + Vec3::new(1.5, 0.0, 0.0)));
}
