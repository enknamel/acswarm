use super::{CORPSE_LIFE, CORPSE_URGENT};
use std::time::Duration;

#[test]
fn a_corpse_lasts_five_minutes() {
    // ACE gives an unlooted monster corpse no timer until its first
    // heartbeat, when it takes the default of five minutes. That is
    // the whole window, so it is what the rules plan against.
    assert_eq!(CORPSE_LIFE, Duration::from_secs(300));
}

#[test]
fn breaking_off_a_fight_is_reserved_for_a_corpse_about_to_go() {
    // Loot keeps for minutes; the thing hitting you does not. The
    // urgency window has to be small enough that a fight is not
    // interrupted for a corpse with plenty of time left, and big
    // enough to actually reach one.
    assert!(CORPSE_URGENT < CORPSE_LIFE / 3, "too eager to break off");
    assert!(
        CORPSE_URGENT >= Duration::from_secs(30),
        "no time to get there"
    );
}
