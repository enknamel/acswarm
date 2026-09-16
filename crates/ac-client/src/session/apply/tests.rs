use super::*;

#[test]
fn a_busy_refusal_does_not_free_the_cast_slot() {
    // Finished, or refused for any other reason: the next may go.
    assert!(frees_the_cast_slot(0));
    assert!(frees_the_cast_slot(0x0402)); // a fizzle
                                          // Too busy: whatever went before is still going on.
    assert!(!frees_the_cast_slot(YOURE_TOO_BUSY));
}
