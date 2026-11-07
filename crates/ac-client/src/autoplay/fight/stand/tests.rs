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
