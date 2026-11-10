use super::*;
use crate::testkit;
use ac_world::object::Position;
use glam::{Quat, Vec3};

/// A dungeon room in Blargerton's dungeon, and two rooms: one it sees, one it does not.
const ROOM: u32 = 0x5A48_01D5;
const SEEN: u32 = 0x5A48_01D6;
const HIDDEN: u32 = 0x5A48_0200;

fn placed(mut o: ac_world::WorldObject, cell: u32) -> ac_world::WorldObject {
    o.position = Some(Position {
        cell,
        local: Vec3::new(10.0, 10.0, 0.0),
        rotation: Quat::IDENTITY,
    });
    o
}

/// A character in `ROOM`, which sees `SEEN` and not `HIDDEN`, with a body in each.
fn in_the_dungeon() -> Client {
    let mut c = testkit::offline_client();
    testkit::stand(&mut c, ROOM, Vec3::new(10.0, 10.0, 0.0));
    let view = View::from_cell(ROOM, |_| Some((vec![0x01D6], false)));
    c.sight.view = Some((ROOM, view));
    for (guid, cell) in [(0x8000_0b01, SEEN), (0x8000_0b02, HIDDEN)] {
        let body = placed(testkit::corpse(guid, "Corpse of Tusker Guard"), cell);
        c.world.objects.insert(guid, body);
    }
    c
}

/// Judge the view each second from `t` to `t + secs`, and say where that left the clock.
fn judge_for(c: &mut Client, t: Instant, secs: u64) -> Instant {
    for s in 0..=secs {
        c.tick_sight(t + Duration::from_secs(s));
    }
    t + Duration::from_secs(secs)
}

#[test]
fn a_dungeon_room_sees_itself_and_the_rooms_listed_from_it_only() {
    let view = View::from_cell(ROOM, |_| Some((vec![0x01D6], false)));
    assert!(view.sees(ROOM) && view.sees(SEEN));
    assert!(!view.sees(HIDDEN), "a room the data does not list");
    assert!(!view.sees(0x5A49_01D6), "the same room number next door");
    assert!(!view.sees(0x5A48_0001), "the landblock's outdoors");
}

#[test]
fn outdoors_the_nine_landblocks_around_are_in_view() {
    let view = View::from_cell(0xA9B4_0019, |_| None);
    assert!(view.sees(0xA9B4_0001) && view.sees(0xAAB5_0040) && view.sees(0xA8B3_0010));
    assert!(
        view.sees(0xA9B4_0105),
        "a room under the same ground: kept, never let go sooner"
    );
    assert!(!view.sees(0xABB4_0019), "two landblocks off");
    // A cell the data does not have: nothing is let go.
    assert!(View::from_cell(ROOM, |_| None).sees(HIDDEN));
}

#[test]
fn what_is_out_of_view_longer_than_the_server_keeps_it_is_set_aside() {
    // ACE stops tracking it 25 s out of view and sends no delete when it rots (ObjectMaint.cs:38-43):
    // Blargerton kept asking at dungeon bodies long gone.
    let mut c = in_the_dungeon();
    let t0 = Instant::now();
    let t = judge_for(&mut c, t0, OUT_OF_VIEW_FOR.as_secs() - 1);
    assert!(
        c.world.objects.contains_key(&0x8000_0b02),
        "let go before the server had"
    );
    judge_for(&mut c, t, 1);
    assert!(
        !c.world.objects.contains_key(&0x8000_0b02),
        "out of view and kept"
    );
    assert!(
        c.world.objects.contains_key(&0x8000_0b01),
        "the one in view went too"
    );
    assert!(
        c.world.out_of_view().any(|o| o.guid == 0x8000_0b02),
        "set aside, not forgotten"
    );
}

#[test]
fn a_moment_in_view_starts_the_clock_again() {
    let mut c = in_the_dungeon();
    let t0 = Instant::now();
    let t = judge_for(&mut c, t0, 30);
    // Walked where it shows, then away again.
    let into = View::from_cell(ROOM, |_| Some((vec![0x01D6, 0x0200], false)));
    c.sight.view = Some((ROOM, into));
    let t = judge_for(&mut c, t + Duration::from_secs(1), 0);
    c.sight.view = Some((ROOM, View::from_cell(ROOM, |_| Some((vec![0x01D6], false)))));
    judge_for(
        &mut c,
        t + Duration::from_secs(1),
        OUT_OF_VIEW_FOR.as_secs() - 2,
    );
    assert!(c.world.objects.contains_key(&0x8000_0b02));
}

#[test]
fn what_was_set_aside_comes_back_by_the_server_s_word_or_goes_by_it() {
    let mut c = in_the_dungeon();
    c.world.objects.insert(
        0x8000_0b03,
        placed(testkit::creature(0x8000_0b03, "Tusker Guard"), HIDDEN),
    );
    let t = judge_for(&mut c, Instant::now(), OUT_OF_VIEW_FOR.as_secs());
    assert_eq!(c.world.out_of_view().count(), 2);
    // Back in view and nothing said of either: asked about, after a moment.
    let both = View::from_cell(ROOM, |_| Some((vec![0x01D6, 0x0200], false)));
    c.sight.view = Some((ROOM, both));
    let sent = c.session.actions_sent();
    let t = judge_for(
        &mut c,
        t + Duration::from_secs(1),
        BACK_IN_VIEW_ASK.as_secs() - 1,
    );
    assert_eq!(
        c.session.actions_sent(),
        sent,
        "asked before the server had its moment"
    );
    judge_for(&mut c, t, 1);
    assert_eq!(c.session.actions_sent(), sent + 2, "each asked about once");
    let answer = |guid: u32, ints: &[(u32, i32)]| {
        let mut w = ac_net::wire::Writer::new();
        let flags = if ints.is_empty() {
            0
        } else {
            ac_net::messages::Appraisal::FLAG_INT
        };
        w.u32(guid).u32(flags).u32(u32::from(!ints.is_empty()));
        if !ints.is_empty() {
            w.u16(ints.len() as u16).u16(16);
            for (k, v) in ints {
                w.u32(*k).i32(*v);
            }
        }
        w.finish()
    };
    // The body is gone; the Tusker Guard is still there.
    c.appraisal(&answer(0x8000_0b02, &[]));
    c.appraisal(&answer(0x8000_0b03, &[(25, 40)]));
    assert!(!c.world.objects.contains_key(&0x8000_0b02));
    assert_eq!(c.world.out_of_view().count(), 0, "neither left set aside");
    assert!(
        c.world.objects.contains_key(&0x8000_0b03),
        "a real one is never lost"
    );
}
