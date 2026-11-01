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

#[test]
fn a_target_three_casts_from_dead_is_worth_a_vulnerability() {
    assert!(
        is_vuln_worth(Some(61.0), Some(30.0), 0, 2000),
        "three casts, whatever the floor"
    );
}

#[test]
fn a_target_two_casts_kill_is_not_worth_one() {
    assert!(
        !is_vuln_worth(Some(60.0), Some(30.0), 5000, 200),
        "two casts, though its full health is over the floor"
    );
    assert!(!is_vuln_worth(Some(12.0), Some(30.0), 0, 200), "one cast");
}

#[test]
fn with_no_damage_to_judge_by_the_health_floor_decides() {
    // A spell with no damage row, or a target whose health is not known.
    assert!(is_vuln_worth(Some(500.0), None, 2500, 2000));
    assert!(!is_vuln_worth(Some(500.0), None, 1500, 2000));
    assert!(
        is_vuln_worth(None, Some(30.0), 2500, 2000),
        "health unknown"
    );
    assert!(
        !is_vuln_worth(None, None, 2500, 0),
        "a floor of 0 never softens"
    );
    assert!(
        is_vuln_worth(Some(90.0), Some(0.0), 2500, 2000),
        "a spell it takes nothing from"
    );
}

#[test]
fn force_arc_iii_needs_four_casts_on_two_hundred_health_and_two_on_a_hundred() {
    // Force Arc III rolls 42 to 84 (data/spell_damage.csv): 63 a cast with no skill over its power.
    let per_cast = cast_damage(42, 84, 0, 200, 1.0, 1.0);
    assert_eq!(per_cast, 63.0);
    assert!(
        is_vuln_worth(Some(200.0), Some(per_cast), 0, 2000),
        "four casts"
    );
    assert!(
        !is_vuln_worth(Some(100.0), Some(per_cast), 0, 2000),
        "two casts"
    );
}

#[test]
fn skill_over_the_spell_power_the_wand_and_the_target_all_count() {
    // 150 points of War Magic over the spell's power add 15% of the roll's minimum.
    assert!((cast_damage(42, 84, 350, 200, 1.0, 1.0) - 69.3).abs() < 1e-4);
    // Skill under the power adds nothing; a 10% wand and a target taking half scale the rest.
    assert!((cast_damage(42, 84, 100, 200, 1.1, 0.5) - 34.65).abs() < 1e-4);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_cast_is_worth_what_the_spell_table_says_it_rolls() {
    let c = crate::testkit::character_of_level(game_data(), 30);
    // No War Magic and no wand: the mean of Force Arc III's roll, and nothing for an unknown spell.
    assert_eq!(c.damage_per_cast(2720, None), Some(63.0));
    assert_eq!(c.damage_per_cast(24, None), None, "Armor Self I deals none");
    // An Ice Golem takes nothing from cold (data/creatures.csv, weenie 196).
    let golem = ac_world::elements::creature_by_id(196);
    assert_eq!(c.damage_per_cast(2727, golem), Some(0.0), "Frost Arc III");
}
