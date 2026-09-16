use std::time::Instant;

use super::*;
use crate::testkit::{no_data, with_packs, SACK};

#[test]
fn no_body_waits_on_a_character_the_server_will_hand_nothing() {
    // +Verity, 36462 carried of a 7500 capacity, on her way to sell:
    // the looting walked her to a corpse for a Pyreal, the server said
    // "You are too encumbered to carry that!", and the walk to town
    // was lost. Past the wall no body is owed, so none is walked to.
    let t0 = Instant::now();
    let ap = Autoplay::default();
    let (me, at) = (glam::Vec3::ZERO, glam::Vec3::new(5.0, 0.0, 0.0));
    let body = 0x8000_9001;
    let walled = Room {
        past_the_wall: true,
        ..Room::PLENTY
    };
    assert!(ap.corpse_owed(body, at, me, t0, Room::PLENTY));
    assert!(!ap.corpse_waiting(body, t0, walled));
    assert!(!ap.corpse_owed(body, at, me, t0, walled));
    // Short of it, a character with no room left for loot still goes
    // to a body: coins weigh nothing, and light things may fit.
    let laden = Room {
        carry: 0,
        ..Room::PLENTY
    };
    assert!(ap.corpse_owed(body, at, me, t0, laden));
}

#[test]
fn free_space_is_what_one_take_can_use_and_not_the_sum_over_the_packs() {
    // Main pack 2 of 4, Sack 19 of 24: seventeen free in all, and
    // five for any one take.
    let c = with_packs(no_data(), 4, 2, 24, 19);
    let me = c.world.player_guid.unwrap();
    assert_eq!(c.free_space(), 5);
    assert_eq!(c.room_anywhere(), 7);
    assert!(!c.pack_full());
    let packs = c.packs();
    assert_eq!((packs.main.capacity, packs.main.used), (4, 2));
    assert_eq!(packs.side.len(), 1);
    assert_eq!((packs.side[0].guid, packs.side[0].used), (SACK, 19));
    // The Sack sits in a pack slot, not an item slot.
    assert_eq!(packs.container_for_a_take(), Some(me));
    // Every pack down to its last slot: room for one take, however
    // many packs there are.
    let mut c = with_packs(no_data(), 4, 3, 24, 23);
    assert_eq!(c.free_space(), 1);
    assert_eq!(c.room_anywhere(), 2);
    // A character with an empty main pack reads as it always did.
    c.world
        .objects
        .retain(|_, o| o.name != "Dagger" && o.guid != SACK);
    assert_eq!(c.free_space(), 4);
    assert_eq!(c.room_anywhere(), 4);
}
