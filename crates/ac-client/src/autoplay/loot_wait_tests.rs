use super::{loot_wait, LOOT_TIMEOUT};

#[test]
fn a_corpse_underfoot_gets_the_plain_wait() {
    assert_eq!(loot_wait(0.0), LOOT_TIMEOUT);
    // A negative distance cannot happen, but must not panic or
    // shorten the wait.
    assert_eq!(loot_wait(-5.0), LOOT_TIMEOUT);
}

#[test]
fn a_corpse_across_a_room_is_given_time_to_walk_to() {
    // The bug: a flat six seconds covered a corpse at our feet and
    // not one twenty metres off, so the far ones were written off
    // unopened.
    let near = loot_wait(2.0);
    let far = loot_wait(20.0);
    assert!(far > near, "{far:?} is not longer than {near:?}");
    assert!(far > LOOT_TIMEOUT * 2, "twenty metres barely added time");
}

#[test]
fn the_wait_does_not_run_away_with_itself() {
    // Whatever distance arrives, the character does not sit on a
    // corpse for ever.
    let silly = loot_wait(100_000.0);
    assert!(silly <= LOOT_TIMEOUT + std::time::Duration::from_secs(30));
}
