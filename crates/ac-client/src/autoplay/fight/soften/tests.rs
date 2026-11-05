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
fn a_vulnerability_is_worth_its_cast_only_when_it_saves_casts() {
    // Three casts of 30, whatever the floor: level VI (x2.5) makes it one; level II (x1.25) two, a draw.
    assert!(is_vuln_worth(
        Some(61.0),
        Some((30.0, 30.0)),
        2.5,
        1.0,
        0,
        2000
    ));
    assert!(!is_vuln_worth(
        Some(61.0),
        Some((30.0, 30.0)),
        1.25,
        1.0,
        0,
        2000
    ));
    assert!(
        !is_vuln_worth(Some(60.0), Some((30.0, 30.0)), 2.5, 1.0, 5000, 200),
        "two casts, though its full health is over the floor"
    );
    assert!(
        !is_vuln_worth(Some(12.0), Some((30.0, 30.0)), 2.5, 1.0, 0, 200),
        "one cast"
    );
}

#[test]
fn a_tusker_guard_two_bolts_kill_half_the_time_is_worth_piercing_vulnerability_vi() {
    // Blargerton's Flame Bolt VI rolled about 88 to 172 on a Tusker Guard's 260: two kill only
    // when they sum over it, half the time. The means (130 twice) said two always did.
    assert!((casts_to_kill(260.0, 88.0, 172.0) - 2.5).abs() < 0.01);
    assert!(is_vuln_worth(
        Some(260.0),
        Some((88.0, 172.0)),
        2.5,
        1.0,
        0,
        2000
    ));
    assert!(
        !is_vuln_worth(Some(260.0), Some((88.0, 172.0)), 1.25, 1.0, 0, 2000),
        "Fire Vulnerability II saves less than its cast"
    );
    // Landing 60% of its casts, Piercing VI takes 1.7 tries of its own: no longer worth it.
    assert!(!is_vuln_worth(
        Some(260.0),
        Some((88.0, 172.0)),
        2.5,
        0.6,
        0,
        2000
    ));
}

#[test]
fn casts_to_kill_count_every_roll_that_leaves_it_standing() {
    assert_eq!(casts_to_kill(50.0, 60.0, 80.0), 1.0, "one always does");
    assert_eq!(casts_to_kill(100.0, 50.0, 50.0), 2.0, "an even roll");
    // Two of 0 to 1 sum under 1 half the time (Irwin-Hall).
    assert!(
        (casts_to_kill(1.0, 0.0, 1.0) - std::f32::consts::E).abs() < 0.01,
        "e, for rolls on 0..1"
    );
    // Far past a dozen casts, the renewal estimate.
    let many = casts_to_kill(2000.0, 40.0, 80.0);
    assert!((many - (2000.0 / 60.0 + 0.5 + 1600.0 / (24.0 * 3600.0))).abs() < 1e-3);
}

#[test]
fn with_no_damage_to_judge_by_the_health_floor_decides() {
    // A spell with no damage row, or a target whose health is not known.
    assert!(is_vuln_worth(Some(500.0), None, 2.5, 1.0, 2500, 2000));
    assert!(!is_vuln_worth(Some(500.0), None, 2.5, 1.0, 1500, 2000));
    assert!(
        is_vuln_worth(None, Some((30.0, 30.0)), 2.5, 1.0, 2500, 2000),
        "health unknown"
    );
    assert!(
        !is_vuln_worth(None, None, 2.5, 1.0, 2500, 0),
        "a floor of 0 never softens"
    );
    assert!(
        is_vuln_worth(Some(90.0), Some((0.0, 0.0)), 2.5, 1.0, 2500, 2000),
        "a spell it takes nothing from"
    );
}

#[test]
fn force_arc_iii_pays_a_vulnerability_on_two_hundred_health_and_not_on_eighty() {
    // Force Arc III rolls 42 to 84 (data/spell_damage.csv) with no skill over its power.
    let roll = cast_roll(42, 84, 0, 200, 1.0, 1.0);
    assert_eq!(roll, (42.0, 84.0));
    assert!(is_vuln_worth(Some(200.0), Some(roll), 2.5, 1.0, 0, 2000));
    assert!(
        !is_vuln_worth(Some(80.0), Some(roll), 2.5, 1.0, 0, 2000),
        "two casts at most"
    );
}

#[test]
fn skill_over_the_spell_power_the_wand_and_the_target_all_count() {
    // 150 points of War Magic over the spell's power add 15% of the roll's minimum to both ends.
    let (lo, hi) = cast_roll(42, 84, 350, 200, 1.0, 1.0);
    assert!((lo - 48.3).abs() < 1e-4 && (hi - 90.3).abs() < 1e-4);
    // Skill under the power adds nothing; a 10% wand and a target taking half scale the rest.
    let (lo, hi) = cast_roll(42, 84, 100, 200, 1.1, 0.5);
    assert!((lo - 23.1).abs() < 1e-4 && (hi - 46.2).abs() < 1e-4);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_cast_is_worth_what_the_spell_table_says_it_rolls() {
    let c = crate::testkit::character_of_level(game_data(), 30);
    // No War Magic and no wand: Force Arc III's roll as it stands, and nothing for a spell dealing none.
    assert_eq!(c.damage_roll(2720, None), Some((42.0, 84.0)));
    assert_eq!(c.damage_roll(24, None), None, "Armor Self I deals none");
    // An Ice Golem takes nothing from cold (data/creatures.csv, weenie 196).
    let golem = ac_world::elements::creature_by_id(196);
    assert_eq!(
        c.damage_roll(2727, golem),
        Some((0.0, 0.0)),
        "Frost Arc III"
    );
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

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_attack_is_chosen_with_the_vulnerability_that_goes_with_it() {
    // Blargerton threw Flame Bolt VI behind Fire Vulnerability Other II (x1.25) at Tusker Guards,
    // which take pierce and fire alike, knowing Piercing Vulnerability Other VI (x2.5) and Force Bolt VI.
    let now = Instant::now();
    let names = [
        "Flame Bolt VI",
        "Force Bolt VI",
        "Fire Vulnerability Other II",
        "Piercing Vulnerability Other VI",
    ];
    let at = |wcid: u32, name: &str| {
        let (mut c, ids) = crate::testkit::a_caster_knowing(now, &names);
        let mut it = crate::testkit::creature(0x8000_0103, name);
        it.weenie_class_id = wcid;
        c.world.objects.insert(it.guid, it);
        (c, ids)
    };
    // A Banished Tusker (575 health) takes five bolts of about 130; behind the x2.5 it takes two.
    let (mut c, ids) = at(30883, "Banished Tusker");
    let ready = [ids[0], ids[1]];
    let vulns = c.vuln_strengths(&ready);
    let best = |c: &crate::Client| c.best_attack(0x8000_0103, &ready, &vulns).map(|(id, _)| id);
    assert_eq!(best(&c), Some(ids[1]), "Force Bolt VI");
    assert!(c.autoplay_make_vulnerable(0x8000_0103, "Banished Tusker", Some(ids[1]), now));
    assert_eq!(
        c.autoplay.status,
        "making Banished Tusker vulnerable to pierce (level 6)"
    );
    assert_eq!(
        best(&c),
        Some(ids[1]),
        "once it is on, the bolts stay piercing"
    );

    // A Tusker Guard (260): two bolts kill it half the time, the vuln and one bolt three times in four.
    let (mut c, ids) = at(1629, "Tusker Guard");
    let ready = [ids[0], ids[1]];
    let vulns = c.vuln_strengths(&ready);
    let best = c.best_attack(0x8000_0103, &ready, &vulns).map(|(id, _)| id);
    assert_eq!(best, Some(ids[1]), "Force Bolt VI");
    assert!(c.autoplay_make_vulnerable(0x8000_0103, "Tusker Guard", best, now));
    assert_eq!(
        c.autoplay.status,
        "making Tusker Guard vulnerable to pierce (level 6)"
    );
}
