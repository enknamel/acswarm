use std::time::Instant;

use ac_world::elements::{imbue, Element};

use super::*;
use crate::testkit::game_data;

fn wand(imbued: u32) -> Wand {
    Wand {
        guid: 0x8000_0900,
        name: "Wand".into(),
        held: true,
        element: 0,
        elemental: 1.0,
        imbued,
        cleave: None,
        crit_frequency: 0.0,
        crit_multiplier: 0.0,
    }
}

#[test]
fn casts_to_kill_count_every_roll_that_leaves_it_standing() {
    assert_eq!(
        casts_to_kill_crits(50.0, 60.0, 80.0, 0.0, 0.0),
        1.0,
        "one always does"
    );
    assert_eq!(
        casts_to_kill_crits(100.0, 50.0, 50.0, 0.0, 0.0),
        2.0,
        "an even roll"
    );
    // Two of 0 to 1 sum under 1 half the time (Irwin-Hall).
    assert!((casts_to_kill_crits(1.0, 0.0, 1.0, 0.0, 0.0) - std::f32::consts::E).abs() < 0.01);
    // Blargerton's Flame Bolt VI, 88 to 172, on a Tusker Guard's 260: two kill half the time.
    assert!((casts_to_kill_crits(260.0, 88.0, 172.0, 0.0, 0.0) - 2.5).abs() < 0.01);
    // Far past a dozen casts, the renewal estimate.
    let many = casts_to_kill_crits(2000.0, 40.0, 80.0, 0.0, 0.0);
    assert!((many - (2000.0 / 60.0 + 0.5 + 1600.0 / (24.0 * 3600.0))).abs() < 1e-3);
}

#[test]
fn a_crit_is_counted_in_every_cast_it_may_land_on() {
    // 50 a cast and a crit of 50 on every one: one cast takes 100.
    assert_eq!(casts_to_kill_crits(100.0, 50.0, 50.0, 1.0, 50.0), 1.0);
    // Half the time: the first cast kills half the time, the second always.
    assert!((casts_to_kill_crits(100.0, 50.0, 50.0, 0.5, 50.0) - 1.5).abs() < 1e-4);
}

#[test]
fn a_wand_rends_crits_and_adds_as_ace_reckons_it() {
    // Rending: base skill over 144, from 1 to 2.5 (WorldObject_Weapon.cs:731).
    let rend = wand(imbue::PIERCE_RENDING);
    assert_eq!(rend.rend_for(Element::Pierce, 288), 2.0);
    assert_eq!(rend.rend_for(Element::Pierce, 500), 2.5, "capped");
    assert_eq!(
        rend.rend_for(Element::Fire, 288),
        1.0,
        "its own element only"
    );
    // A quest wand's cleave.
    let mut cleave = wand(0);
    cleave.cleave = Some((Element::Fire as u32, 2.0));
    assert_eq!(cleave.rend_for(Element::Fire, 100), 2.0);
    // Crits: 5% and no bonus by default; Critical Strike and Crippling Blow by base skill.
    assert_eq!(wand(0).crits(300), (0.05, 1.0));
    assert_eq!(wand(imbue::CRITICAL_STRIKE).crits(360), (0.5, 1.0));
    assert_eq!(
        wand(imbue::CRIPPLING_BLOW).crits(400),
        (0.05, 6.0),
        "capped at 360"
    );
    // The element bonus on the wand's own element.
    let mut fire = wand(0);
    fire.element = Element::Fire as u32;
    fire.elemental = 1.1;
    assert_eq!(fire.elemental_for(Element::Fire), 1.1);
    assert_eq!(fire.elemental_for(Element::Pierce), 1.0);
}

#[test]
fn with_no_health_known_the_most_a_cast_wins_and_no_vuln_is_cast() {
    let kill = |casts: Option<f32>, per_cast: f32, vuln: bool| Kill {
        wand: wand(0),
        attack: 1,
        takes: 1.0,
        vuln: vuln.then_some((Element::Fire, 2, 2.5, 1.0)),
        casts,
        per_cast,
    };
    let best = fastest(vec![kill(None, 100.0, false), kill(None, 250.0, true)]);
    assert_eq!(best.map(|k| k.per_cast), Some(100.0));
    let best = fastest(vec![
        kill(Some(2.5), 100.0, false),
        kill(Some(2.2), 250.0, true),
    ]);
    assert!(best.is_some_and(|k| k.vuln.is_some()), "fewest casts");
}

/// A caster knowing `names`, facing a Tusker Guard (wcid 1629: 260 health, pierce and fire alike).
fn at_a_tusker_guard(names: &[&str]) -> (crate::Client, Vec<u32>) {
    let (mut c, ids) = crate::testkit::a_caster_knowing(Instant::now(), names);
    let mut it = crate::testkit::creature(0x8000_0103, "Tusker Guard");
    it.weenie_class_id = 1629;
    c.world.objects.insert(it.guid, it);
    (c, ids)
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn piercing_vi_and_force_bolt_vi_kill_a_tusker_guard_fastest() {
    // Blargerton threw Frost Bolt VI behind Cold Vulnerability IV at guards resisting cold: the
    // vulnerability's own cast was never charged, so it looked better than a bolt alone.
    let _ = game_data();
    let names = [
        "Flame Bolt VI",
        "Force Bolt VI",
        "Frost Bolt VI",
        "Fire Vulnerability Other II",
        "Piercing Vulnerability Other VI",
        "Cold Vulnerability Other IV",
    ];
    let (c, ids) = at_a_tusker_guard(&names);
    let cfg = c.autoplay.config.fight.clone();
    let kill = c
        .best_kill(0x8000_0103, &cfg, Instant::now())
        .expect("a kill");
    assert_eq!(kill.attack, ids[1], "Force Bolt VI: {kill:?}");
    assert_eq!(
        kill.vuln.map(|(_, id, ..)| id),
        Some(ids[4]),
        "Piercing VI first"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_rending_wand_at_the_vulns_strength_makes_its_cast_a_waste() {
    // A Pierce Rending wand at base 360 is a level VI vulnerability built in, and the larger of the
    // two counts (Creature_Properties.cs:133-134): no vuln for it. Cleaving at 1.5 still wants one.
    let (mut c, ids) = at_a_tusker_guard(&["Force Bolt VI", "Piercing Vulnerability Other VI"]);
    crate::testkit::trained_in(&mut c, ac_world::stats::skill::WAR_MAGIC, 360);
    let ready = [ids[0]];
    let kill =
        fastest(c.kills(0x8000_0103, &ready, &[wand(imbue::PIERCE_RENDING)])).expect("a kill");
    assert_eq!(kill.vuln, None, "rent to 2.5 already: {kill:?}");
    // Cleaving at 1.5, with ordinary skill, on a Banished Tusker (575): Piercing VI pays its cast.
    let (mut c, ids) = at_a_tusker_guard(&["Force Bolt VI", "Piercing Vulnerability Other VI"]);
    let ready = [ids[0]];
    if let Some(it) = c.world.objects.get_mut(&0x8000_0103) {
        it.weenie_class_id = 30883;
        it.name = "Banished Tusker".into();
    }
    let mut weak = wand(0);
    weak.cleave = Some((Element::Pierce as u32, 1.5));
    let kill = fastest(c.kills(0x8000_0103, &ready, &[weak])).expect("a kill");
    assert_eq!(kill.vuln.map(|(_, id, ..)| id), Some(ids[1]), "{kill:?}");
}
