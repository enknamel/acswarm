use std::time::Instant;

use super::*;

#[test]
fn a_pour_refused_while_a_take_is_in_the_air_is_not_the_takes_refusal() {
    // The tidying pours in the gaps between takes, so both can be
    // out at once. A refusal that names a stack in the pack is
    // about that stack, and the take goes on waiting for its own
    // answer rather than being written off.
    let (in_the_pack, on_the_corpse) = (0x8000_7001, 0x8000_7002);
    assert_eq!(
        refused_item(in_the_pack, 0, Some(on_the_corpse)),
        Some(in_the_pack)
    );
}

#[test]
fn a_daily_limit_is_a_wait_and_not_a_grudge() {
    use crate::did::{Because, Did, Patience};
    let t0 = Instant::now();
    let mut kinds: Patience<u32> = Patience::new();

    // YouHaveSolvedThisQuestTooRecently is what gates a once-a-day
    // drop. It is a wait: the thing comes back, and a session can
    // run for days.
    let too_recently = Did::Blocked(Because::server(0x043E));
    assert_eq!(too_recently.because().and_then(|b| b.code), Some(0x043E));
    kinds.note(7299, &too_recently, t0);
    assert!(kinds.held(&7299, t0), "left alone for now");
    // Not for ever, though: a day later it is asked about again.
    assert!(
        !kinds.held(&7299, t0 + Duration::from_secs(24 * 60 * 60)),
        "a day later it is worth another ask"
    );
    // Nor is "too many times", which a raised cap can lift.
    kinds.note(7299, &Did::Blocked(Because::server(0x043F)), t0);
    assert!(!kinds.held(&7299, t0 + Duration::from_secs(24 * 60 * 60)));
    // Only a thing no counter will ever take is for ever, and that
    // is a different answer entirely.
    kinds.note(1, &Did::refused("no vendor will take it"), t0);
    assert!(kinds.held(&1, t0 + Duration::from_secs(24 * 60 * 60)));
}
