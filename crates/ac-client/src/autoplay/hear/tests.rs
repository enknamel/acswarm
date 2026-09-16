use super::*;
use crate::autoplay::fight::melee::{nothing_arrived, CLOSE_IN_AFTER};

#[test]
fn a_spell_cast_at_the_character_names_its_caster() {
    // The lines ACE sends the target of a spell, and nothing else is
    // sent: SpellProjectile's damage and drain, WorldObject_Magic's
    // drain, transfer and resist.
    for (line, who) in [
        (
            "Drudge Shaman blasts you for 12 points with Flame Bolt I.",
            "Drudge Shaman",
        ),
        (
            "Critical hit! Sneak Attack! Drudge Shaman scorches you for 40 points \
             with Flame Bolt II.",
            "Drudge Shaman",
        ),
        (
            "Overpower! Mite hits you for 3 points with Acid Stream I. Your \
             augmentation allows you to avoid a critical hit!",
            "Mite",
        ),
        (
            "Drudge Shaman casts Harm Other I and drains 9 points of your health.",
            "Drudge Shaman",
        ),
        (
            "You lose 20 points of mana due to Drudge Shaman casting Mana to \
             Health Other I on you",
            "Drudge Shaman",
        ),
        (
            "You resist the spell cast by Drudge Shaman",
            "Drudge Shaman",
        ),
    ] {
        assert_eq!(spell_attacker(line), Some(who), "{line}");
    }
    // The character's own spells, and a fellow's help, are no attack.
    for line in [
        "You blast Drudge Shaman for 12 points with Flame Bolt I.",
        "Drudge Shaman resists your spell",
        "With Harm Other I you drain 9 points of health from Drudge Shaman.",
        "Aldric casts Heal Other I and restores 30 points of your health.",
        "Aldric cast Strength Other I on you",
        "You gain 20 points of health due to Aldric casting Stamina to Health \
         Other I on you",
        "You lose 50 points of stamina due to casting Stamina to Mana Other I \
         on Aldric",
        "Drudge Skulker hits you for 5 points.",
    ] {
        assert_eq!(spell_attacker(line), None, "{line}");
    }
}

#[test]
fn a_resist_or_an_evasion_got_there() {
    assert_eq!(
        arrived_unharmed("Drudge Skulker resists your spell"),
        Some("Drudge Skulker")
    );
    assert_eq!(
        arrived_unharmed("Mite Scion evades your attack."),
        Some("Mite Scion")
    );
    // Ours, not theirs.
    assert_eq!(
        arrived_unharmed("You resist the spell cast by Drudge Skulker"),
        None
    );
    let now = Instant::now();
    let long_ago = now.checked_sub(CLOSE_IN_AFTER * 2).unwrap();
    // Thrown a while ago, and nothing has got there since.
    assert!(nothing_arrived(Some(long_ago), now));
    // Only just thrown: still on its way.
    assert!(!nothing_arrived(Some(now), now));
    // Nothing thrown yet -- still walking to a clear shot -- is no miss.
    assert!(!nothing_arrived(None, now));
}
