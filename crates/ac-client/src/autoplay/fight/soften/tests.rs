use super::*;
use crate::testkit::{game_data, mid_fight};

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_softening_with_no_wand_to_be_had_lets_the_fight_go_ahead() {
    let (mut c, wand) = mid_fight(game_data());
    const CREATURE: u32 = 0x8000_0103;
    // A Drudge Skulker is weakest to cold, fire and electricity
    // alike; whichever it picks, the character knows the
    // vulnerability for it.
    for element in [
        ac_world::elements::Element::Cold,
        ac_world::elements::Element::Fire,
        ac_world::elements::Element::Electric,
    ] {
        for id in ac_world::elements::vulnerabilities(element) {
            c.world.stats.spells.push(id);
        }
    }
    assert_eq!(c.combat_stance(), Stance::Melee);
    assert!(!c.mid_attack());
    // With a wand to be had, the softening takes the tick to reach
    // for one. This is the setup working, and what makes the second
    // half mean anything.
    assert!(
        c.autoplay_soften(CREATURE, "Drudge Skulker", Instant::now()),
        "reaching for the wand"
    );
    assert_eq!(c.autoplay.pending_wield, Some(wand));

    // Now the server is refusing that wand. Nothing can be sent, so
    // the softening gives the tick back rather than holding fire on
    // the target for ever: every expiry of the wait earned one more
    // refusal and doubled the next, up towards four hours.
    let (mut c, wand) = mid_fight(game_data());
    for element in [
        ac_world::elements::Element::Cold,
        ac_world::elements::Element::Fire,
        ac_world::elements::Element::Electric,
    ] {
        for id in ac_world::elements::vulnerabilities(element) {
            c.world.stats.spells.push(id);
        }
    }
    c.hold_off_wield(wand, Instant::now());
    assert!(
        !c.autoplay_soften(CREATURE, "Drudge Skulker", Instant::now()),
        "the fight may go ahead unsoftened"
    );
    assert_eq!(c.autoplay.pending_wield, None, "nothing was sent");
}
