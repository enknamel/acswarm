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
    crate::testkit::trained_in(&mut c, ac_world::stats::skill::LIFE_MAGIC, 200);
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
    crate::testkit::trained_in(&mut c, ac_world::stats::skill::LIFE_MAGIC, 200);
    c.hold_off_wield(wand, Instant::now());
    assert!(
        !c.autoplay_soften(CREATURE, "Drudge Skulker", Instant::now()),
        "the fight may go ahead unsoftened"
    );
    assert_eq!(c.autoplay.pending_wield, None, "nothing was sent");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_war_mage_softening_alone_casts_no_imperil() {
    // Blargerton softened each Tusker Slave with a vulnerability and then Imperil Other I: an
    // imperil lowers armour, which war spells ignore. It helps only a mate hitting with a weapon.
    let now = Instant::now();
    let (mut c, _) = crate::testkit::a_caster_knowing(now, &["Flame Bolt I", "Imperil Other I"]);
    const CREATURE: u32 = 0x8000_0103;
    let it = crate::testkit::creature(CREATURE, "Tusker Slave");
    c.world.objects.insert(it.guid, it);
    assert_eq!(c.combat_stance(), Stance::Magic);
    assert!(!c.hits_with_weapons());
    c.autoplay.softening = vec![(CREATURE, 1)];
    assert!(!c.autoplay_soften(CREATURE, "Tusker Slave", now));
    assert_eq!(c.autoplay.debuff_sent, None, "an imperil for spells");
    assert!(c.autoplay.debuffed.contains(&CREATURE));

    // A mate on it with a sword: the imperil is worth its cast.
    let mut sword = crate::testkit::mate(0x5000_0009, "Swordsman");
    sword.hits_with_weapons = true;
    sword.target = Some(CREATURE);
    c.autoplay.team.mates.push(sword);
    c.autoplay.debuffed.clear();
    c.autoplay.softening = vec![(CREATURE, 1)];
    c.autoplay_soften(CREATURE, "Tusker Slave", now);
    assert_eq!(
        c.autoplay.debuff_sent,
        Some((CREATURE, 1)),
        "no imperil for the sword"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_casters_softening_makes_the_target_vulnerable_to_the_spell_it_will_throw() {
    // Blargerton softened a Tusker Slave with Fire Vulnerability, its weakest, then threw a
    // piercing spell: the vulnerability is for the element of the attack, not the creature alone.
    let now = Instant::now();
    for (attack, element) in [
        ("Force Bolt I", ac_world::elements::Element::Pierce),
        ("Flame Bolt I", ac_world::elements::Element::Fire),
    ] {
        let (mut c, _) = crate::testkit::a_caster_knowing(now, &[attack]);
        let it = crate::testkit::creature(0x8000_0103, "Tusker Slave");
        let wcid = 1628;
        c.world.objects.insert(it.guid, it);
        c.world
            .objects
            .get_mut(&0x8000_0103)
            .unwrap()
            .weenie_class_id = wcid;
        assert_eq!(
            c.vuln_element(0x8000_0103),
            Some(element),
            "throwing {attack}"
        );
    }
}
