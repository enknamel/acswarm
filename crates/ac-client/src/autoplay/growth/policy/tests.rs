use super::*;

#[test]
fn a_character_with_nobody_to_restock_with_goes_to_town_on_its_own() {
    // Blargerton: team rules on, restocking together on, no party.
    // The party's mode decided his trips, a party of one never left
    // for weight, and he hunted on laden with every body left full.
    let team = crate::autoplay::Team {
        enabled: true,
        restock: crate::logistics::Restock {
            together: true,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(!restocks_as_a_party(&team, 0), "alone is alone");
    assert!(restocks_as_a_party(&team, 1));
    // Either setting off, it is alone whoever else is about.
    let apart = crate::autoplay::Team {
        restock: crate::logistics::Restock {
            together: false,
            ..Default::default()
        },
        ..team.clone()
    };
    assert!(!restocks_as_a_party(&apart, 3));
    let off = crate::autoplay::Team {
        enabled: false,
        ..team
    };
    assert!(!restocks_as_a_party(&off, 3));
}
