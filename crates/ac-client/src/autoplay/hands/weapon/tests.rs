use super::*;

#[test]
fn a_buff_pass_that_wants_a_wand_waits_rather_than_disarming_mid_charge() {
    use crate::Stance;
    // +Verity's buffing reached for her Training Wand every second
    // and a half while she was charging a Drudge Servant, and every
    // reach put her mace away and cancelled the charge with it.
    assert!(change_of_hands_waits(Stance::Melee, Stance::Magic, true));
    assert!(change_of_hands_waits(Stance::Missile, Stance::Magic, true));

    // Answered, the gap between two swings is hers: the wand goes in
    // then, and nothing is cancelled.
    assert!(!change_of_hands_waits(Stance::Melee, Stance::Magic, false));

    // A pass that already holds a wand changes nothing, so there is
    // nothing to wait for and the buff goes up mid-fight as before.
    assert!(!change_of_hands_waits(Stance::Magic, Stance::Magic, true));

    // The rule is about hands, not about wands: a character told to
    // fight with a bow waits for the swing just the same.
    assert!(change_of_hands_waits(Stance::Melee, Stance::Missile, true));
}

#[test]
fn a_refused_wield_is_left_alone_for_longer_every_time() {
    use crate::did::Patience;
    // ACE refuses a wield it will not make with no error code at all
    // -- a caster cannot go in while a shield is up, and it says so
    // with WeenieError.None -- so there is nothing to read and
    // nothing to do but wait. +Verity asked 150 times in a minute.
    const WAND: u32 = 0x8000_00C3;
    let t0 = Instant::now();
    let mut held: Patience<u32> = Patience::new();

    held.hold(WAND, WIELD_AGAIN, t0);
    assert!(held.held(&WAND, t0), "not asked for again at once");
    assert!(
        held.held(&WAND, t0 + WIELD_AGAIN - Duration::from_millis(1)),
        "nor a moment before the wait is up"
    );
    assert!(
        !held.held(&WAND, t0 + WIELD_AGAIN),
        "asked again once the wait is up"
    );

    // Refused again: the wait doubles, so an item the server will
    // never wield in this state costs a handful of asks rather than
    // one every buff pass.
    let second = t0 + WIELD_AGAIN;
    held.hold(WAND, WIELD_AGAIN, second);
    assert_eq!(held.waited(&WAND), Some(WIELD_AGAIN * 2));
    assert!(held.held(&WAND, second + WIELD_AGAIN));
    assert!(!held.held(&WAND, second + WIELD_AGAIN * 2));

    // A wield that lands forgets the wait: the hands have changed,
    // so whatever the server was objecting to has gone.
    held.forget(&WAND);
    assert!(!held.held(&WAND, second));
    assert_eq!(held.waited(&WAND), None);
}
