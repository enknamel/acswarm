use std::time::Instant;

use super::*;

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
