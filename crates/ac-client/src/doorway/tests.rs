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
    let did = c.autoplay_approach(rat.guid, "Swamp Rat", How::Spell(spell));
    assert_eq!(did, Did::Acting);
    let spot = c
        .dodge
        .vantage
        .and_then(|(_, _, s)| s)
        .expect("somewhere to throw from")
        .at;
    assert!(!west_of_the_door(spot), "in the rat's room: {spot:?}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_target_in_a_doorway_is_waited_for_out_of_it() {
    let (mut c, spell) = by_the_open_door();
    let rat = crate::testkit::standing_by(&mut c, 0x8000_0001, "Swamp Rat", 3.75);
    let how = How::Spell(spell);
    assert!(!c.shot_clears(rat.guid, how), "it stands in the frame");
    assert_eq!(
        c.autoplay_approach(rat.guid, "Swamp Rat", how),
        Did::Acting,
        "waiting for it"
    );
    let (_, _, spot) = c.dodge.pulling.expect("waiting for it");
    let spot = spot.expect("a spot to wait at").at;
    let door = ac_world::landblock_origin(0x01F6_0000) + Vec3::new(94.75, -60.0, -6.0);
    assert!(west_of_the_door(spot), "on our own side: {spot:?}");
    assert!(
        spot.distance(door) >= PULL_BACK - 0.01,
        "back from it: {spot:?}"
    );
    assert_eq!(
        c.going.as_ref().and_then(|g| g.goal.place()).map(|p| p.at),
        Some(spot),
        "walking there"
    );
    // Come through to our side, it is shot at.
    let me = c.my_position().unwrap();
    let out = me + Vec3::new(-2.0, 0.0, 0.0) - ac_world::landblock_origin(0x01F6_01FA);
    c.world.objects.get_mut(&rat.guid).unwrap().position =
        Some(ac_world::object::Position::new_flat(0x01F6_01FA, out));
    assert!(c.shot_clears(rat.guid, how), "out of the doorway");
    assert!(!c.clear_doorway(rat.guid, "Swamp Rat", me + Vec3::new(-2.0, 0.0, 0.0), how));
}

/// The door 0x028602F6 in dungeon 0x0286, open, where 15 of Blargerton's casts went nowhere: he
/// stands at `me_y` across it (local y; the door is at y -95.25, x 70), a Shore Armoredillo at
/// `it_y`.
fn in_the_0286_doorway(me_y: f32, it_y: f32) -> (Client, u32, u32) {
    let origin = ac_world::landblock_origin(0x0286_0000);
    let rooms = crate::testkit::game_data()
        .block_collision(0x0286_0000)
        .unwrap();
    let floor = |y: f32| {
        let p = Vec3::new(70.0, y, 0.0);
        let (_, cell) = rooms
            .world
            .floor_at(origin + p + Vec3::Z, 2.0, 3.0)
            .expect("a floor");
        (p, cell)
    };
    let (me, cell) = floor(me_y);
    let mut c = crate::testkit::standing_in_the_field(37, cell, me);
    let door = ac_world::WorldObject {
        guid: 0x7028_62F6,
        name: "Door".into(),
        setup_id: 0x0200_024F,
        object_desc_flags: ac_world::object_desc_flags::DOOR,
        physics_state: 0x10018 | ac_world::object::PHYSICS_STATE_ETHEREAL,
        position: Some(ac_world::object::Position {
            cell: 0x0286_02F6,
            local: Vec3::new(70.0, -95.25, 0.0),
            rotation: glam::Quat::from_xyzw(0.0, 0.0, 0.0, -1.0),
        }),
        scale: 1.0,
        ..Default::default()
    };
    c.world.objects.insert(door.guid, door);
    let (it, it_cell) = floor(it_y);
    let mut o = crate::testkit::standing_by(&mut c, 0x8000_0001, "Shore Armoredillo", 0.0);
    o.position = Some(ac_world::object::Position::new_flat(it_cell, it));
    c.world.objects.insert(o.guid, o);
    let spell = c
        .assets
        .spell_table()
        .unwrap()
        .find_by_name("Force Arc III")
        .map(|(id, _)| id)
        .expect("Force Arc III");
    (c, spell, 0x8000_0001)
}

fn past_the_0286_door(at: Vec3) -> f32 {
    at.y - ac_world::landblock_origin(0x0286_0000).y + 95.25
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_caster_in_a_doorway_steps_out_into_the_target_s_room() {
    // In the frame, the armoredillo 1.8 m off on the far side: it was thrown, and went nowhere.
    let (mut c, spell, it) = in_the_0286_doorway(-95.55, -93.75);
    let how = How::Spell(spell);
    assert!(!c.shot_clears(it, how), "from the frame");
    assert_eq!(
        c.autoplay_approach(it, "Shore Armoredillo", how),
        Did::Acting,
        "thrown from the frame"
    );
    let to = c
        .going
        .as_ref()
        .and_then(|g| g.goal.place())
        .map(|p| p.at)
        .expect("walking out of it");
    assert!(
        past_the_0286_door(to) > DOORWAY_DEPTH,
        "into its room: {to:?}"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_spot_to_throw_from_is_out_of_the_doorway() {
    // Just short of the frame, the armoredillo just past it: not thrown from beside the frame.
    let (mut c, spell, it) = in_the_0286_doorway(-96.25, -94.45);
    let how = How::Spell(spell);
    assert_eq!(
        c.autoplay_approach(it, "Shore Armoredillo", how),
        Did::Acting
    );
    let to = c
        .going
        .as_ref()
        .and_then(|g| g.goal.place())
        .map(|p| p.at)
        .expect("walking");
    assert!(
        past_the_0286_door(to).abs() > DOORWAY_DEPTH,
        "beside the frame: {:.2} m from the door",
        past_the_0286_door(to)
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_caster_in_a_doorway_with_its_target_past_the_walls_is_not_held_at_the_door() {
    // Blargerton by Archmage Philomon's door in 0x95D6, Relic Bones 59 m off outside: he stepped
    // "out of the doorway" to a spot still in no sight of it, walked back for sight, and flipped
    // between the two every frame until the server dropped him (live, 2026-11-05).
    let origin = ac_world::landblock_origin(0x95D6_0000);
    let mut c =
        crate::testkit::standing_in_the_field(42, 0x95D6_0132, Vec3::new(85.875, 100.617, 100.0));
    let door = ac_world::WorldObject {
        guid: 0x795D_6001,
        name: "Door".into(),
        setup_id: 0x0200_19FF,
        object_desc_flags: ac_world::object_desc_flags::DOOR,
        physics_state: 0x10018,
        position: crate::testkit::placed(0x95D6_0138, origin + Vec3::new(87.36, 99.2242, 100.002)),
        scale: 1.0,
        ..Default::default()
    };
    c.world.objects.insert(door.guid, door);
    const BONES: u32 = 0x8000_0002;
    let mut bones = crate::testkit::creature(BONES, "Relic Bones");
    bones.object_desc_flags |= ac_world::object_desc_flags::ATTACKABLE;
    bones.position =
        crate::testkit::placed(0x95D6_0027, origin + Vec3::new(119.375, 148.39, 91.27));
    c.world.objects.insert(BONES, bones);
    let spell = c
        .assets
        .spell_table()
        .unwrap()
        .find_by_name("Flame Bolt VI")
        .map(|(id, _)| id)
        .expect("Flame Bolt VI");
    let how = How::Spell(spell);
    let (t0, dt) = (Instant::now(), 0.033);
    let mut now = t0;
    let mut flips = 0;
    let mut said = String::new();
    while now < t0 + Duration::from_secs(20) {
        if c.autoplay_approach(BONES, "Relic Bones", how) == Did::Done {
            break;
        }
        if c.autoplay.status != said {
            flips += 1;
            said = c.autoplay.status.clone();
        }
        c.tick_player(crate::player::Input::default(), dt, now);
        now += Duration::from_secs_f32(dt);
    }
    assert!(
        c.shot_clears(BONES, how),
        "no shot after {:?}, at {:?}, saying {said:?}",
        now - t0,
        c.my_position().map(|p| p - origin)
    );
    assert!(flips <= 3, "{flips} changes of mind on the way");
}
