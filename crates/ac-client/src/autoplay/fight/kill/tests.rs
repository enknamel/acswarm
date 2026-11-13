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
    let kill = |seconds: Option<f32>, per_second: f32, vuln: bool| Kill {
        wand: wand(0),
        attack: 1,
        takes: 1.0,
        vuln: vuln.then_some((Element::Fire, 2, 2.5, 1.0)),
        magic_yield: None,
        casts: seconds,
        seconds,
        per_cast: per_second,
        per_second,
    };
    let best = fastest(vec![kill(None, 100.0, false), kill(None, 250.0, true)]);
    assert_eq!(best.map(|k| k.per_second), Some(100.0));
    let best = fastest(vec![
        kill(Some(2.5), 100.0, false),
        kill(Some(2.2), 250.0, true),
    ]);
    assert!(best.is_some_and(|k| k.vuln.is_some()), "fewest seconds");
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
fn a_vulnerability_is_charged_its_own_cast_and_a_tusker_guard_needs_none() {
    // Blargerton threw Frost Bolt VI behind Cold Vulnerability IV at guards resisting cold: the
    // vulnerability's own cast was never charged, so it looked better than a bolt alone. Timed,
    // Piercing VI's 3.7 s try (Ebony's gesture, 72% to land) saves no more than it takes.
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
    let ways = c.kills_said(0x8000_0103, &cfg, 8);
    assert_eq!(kill.attack, ids[0], "Flame Bolt VI alone: {ways}");
    assert_eq!(kill.vuln, None, "{ways}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_rending_wand_at_the_vulns_strength_makes_its_cast_a_waste() {
    // A Pierce Rending wand at base 360 is a level VI vulnerability built in, and the larger of the
    // two counts (Creature_Properties.cs:133-134): no vuln for it. Cleaving at 1.5 still wants one
    // that lands surely.
    let (mut c, ids) = at_a_tusker_guard(&["Force Bolt VI", "Piercing Vulnerability Other VI"]);
    crate::testkit::trained_in(&mut c, ac_world::stats::skill::WAR_MAGIC, 360);
    let ready = [ids[0]];
    let kill =
        fastest(c.kills(0x8000_0103, &ready, &[wand(imbue::PIERCE_RENDING)])).expect("a kill");
    assert_eq!(kill.vuln, None, "rent to 2.5 already: {kill:?}");
    // Cleaving at 1.5 on a Banished Tusker (575): at Life 300 Piercing VI lands 62% and its tries
    // cost more than the bolts they save; at 400 it lands nearly always and pays its cast.
    let (mut c, ids) = at_a_tusker_guard(&["Force Bolt VI", "Piercing Vulnerability Other VI"]);
    crate::testkit::trained_in(&mut c, ac_world::stats::skill::LIFE_MAGIC, 400);
    let ready = [ids[0]];
    if let Some(it) = c.world.objects.get_mut(&0x8000_0103) {
        it.weenie_class_id = 30883;
        it.name = "Banished Tusker".into();
    }
    let mut weak = wand(0);
    weak.cleave = Some((Element::Pierce as u32, 1.5));
    let all = c.kills(0x8000_0103, &ready, &[weak]);
    let said: Vec<String> = all.iter().map(|k| k.said(|id| id.to_string())).collect();
    let kill = fastest(all).expect("a kill");
    assert_eq!(kill.vuln.map(|(_, id, ..)| id), Some(ids[1]), "{said:?}");
}

#[test]
fn a_resist_is_the_servers_roll_of_skill_against_magic_defence() {
    // GetSkillChance at 0.03 (WorldObject_Magic.cs:107): even at par, 1 / (1 + e^-1.8) 60 over.
    assert_eq!(unresisted(300, Some(300.0)), 0.5);
    assert!((unresisted(304, Some(244.0)) - 0.858).abs() < 1e-3);
    assert_eq!(
        unresisted(10, None),
        1.0,
        "no defence known, no resist reckoned"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn magic_yield_goes_first_only_when_it_saves_casts() {
    let names = [
        "Force Bolt VI",
        "Piercing Vulnerability Other VI",
        "Futility",
    ];
    let skilled = |c: &mut crate::Client| {
        // Creature Enchantment, Life and War Magic.
        for skill in [31, 33, 34] {
            crate::testkit::trained_in(c, skill, 350);
        }
    };
    // A Tusker Guard (244) dies in two bolts that are seldom resisted: Futility's cast is a waste.
    let (mut c, ids) = at_a_tusker_guard(&names);
    skilled(&mut c);
    let cfg = c.autoplay.config.fight.clone();
    let kill = c
        .best_kill(0x8000_0103, &cfg, Instant::now())
        .expect("a kill");
    assert_eq!(kill.magic_yield, None, "{kill:?}");
    assert_eq!(kill.vuln.map(|(_, id, ..)| id), Some(ids[1]), "{kill:?}");
    // A Titanium Armoredillo (7083: 5000 health, 362) resists more than half the bolts: lowered by
    // 40 first, every one of thirty-odd bolts lands more often.
    let (mut c, ids) = at_a_tusker_guard(&names);
    skilled(&mut c);
    if let Some(it) = c.world.objects.get_mut(&0x8000_0103) {
        it.weenie_class_id = 7083;
        it.name = "Titanium Armoredillo".into();
    }
    let kill = c
        .best_kill(0x8000_0103, &cfg, Instant::now())
        .expect("a kill");
    assert_eq!(
        kill.magic_yield.map(|(id, by, _)| (id, by)),
        Some((ids[2], 40.0)),
        "{kill:?}"
    );
    assert_eq!(
        kill.vuln.map(|(_, id, ..)| id),
        Some(ids[1]),
        "and its vulnerability after"
    );
    // Once on, it is not cast again.
    c.autoplay.mark_yielded(0x8000_0103, 40.0);
    *c.autoplay.kill_kept.borrow_mut() = None;
    let kill = c
        .best_kill(0x8000_0103, &cfg, Instant::now())
        .expect("a kill");
    assert_eq!(kill.magic_yield, None, "{kill:?}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_lower_vulnerability_that_lands_surer_is_cast_when_it_kills_faster() {
    let names = [
        "Flame Bolt VI",
        "Fire Vulnerability Other V",
        "Fire Vulnerability Other VI",
    ];
    let skilled = |c: &mut crate::Client, life: u32| {
        crate::testkit::trained_in(c, 33, life);
        crate::testkit::trained_in(c, 34, 282);
    };
    // Life and War Magic 282, as Blargerton's: VI (power 275) casts 62% of the time and is resisted
    // on top, so V (225), x2 instead of x2.5 but nearly always cast, kills a Banished Tusker (575)
    // sooner.
    let banished = |c: &mut crate::Client| {
        if let Some(it) = c.world.objects.get_mut(&0x8000_0103) {
            it.weenie_class_id = 30883;
            it.name = "Banished Tusker".into();
        }
    };
    let (mut c, ids) = at_a_tusker_guard(&names);
    banished(&mut c);
    skilled(&mut c, 282);
    let cfg = c.autoplay.config.fight.clone();
    let kill = c
        .best_kill(0x8000_0103, &cfg, Instant::now())
        .expect("a kill");
    let ways = c.kills_said(0x8000_0103, &cfg, 8);
    assert_eq!(kill.vuln.map(|(_, id, ..)| id), Some(ids[1]), "{ways}");
    // With skill to spare VI casts as surely, and its x2.5 wins.
    let (mut c, ids) = at_a_tusker_guard(&names);
    banished(&mut c);
    skilled(&mut c, 400);
    let kill = c
        .best_kill(0x8000_0103, &cfg, Instant::now())
        .expect("a kill");
    let ways = c.kills_said(0x8000_0103, &cfg, 8);
    assert_eq!(kill.vuln.map(|(_, id, ..)| id), Some(ids[2]), "{ways}");
}

/// [`at_a_tusker_guard`], the guard `metres` north of the caster with `health` left.
fn guard_off(names: &[&str], metres: f32, health: f32) -> (crate::Client, Vec<u32>) {
    let (mut c, ids) = at_a_tusker_guard(names);
    let me = c.my_position().expect("standing");
    let cell = c.player.as_ref().map(|p| p.cell).expect("a body");
    if let Some(it) = c.world.objects.get_mut(&0x8000_0103) {
        it.position = crate::testkit::placed(cell, me + glam::Vec3::Y * metres);
        it.health = Some(health / 260.0);
    }
    (c, ids)
}

/// The fastest way at the guard with the spells `ready`, and every way as the log says it.
fn fastest_at_guard(c: &crate::Client, ready: &[u32]) -> (Kill, String) {
    let cfg = c.autoplay.config.fight.clone();
    let all = c.kills(0x8000_0103, ready, &c.wands_to_hand(&cfg));
    let name = |id: u32| c.spell(id).map(|s| s.name).unwrap_or_default();
    let said: Vec<String> = all.iter().map(|k| k.said(name)).collect();
    (fastest(all).expect("a kill"), said.join("; "))
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_streak_with_no_windup_is_thrown_when_it_kills_sooner_than_the_bolt() {
    // A Tusker Guard 20 m off: Flame Bolt VI kills in fewer casts, but each holds the hands 3.3 s
    // (Pyreal windup) and flies at 15 m/s; Flame Streak VI is FastCast, 1.5 s a try, 45 m/s.
    let (c, ids) = guard_off(&["Flame Bolt VI", "Flame Streak VI"], 20.0, 260.0);
    let (kill, said) = fastest_at_guard(&c, &ids);
    assert_eq!(kill.attack, ids[1], "{said}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_bolt_is_kept_where_its_damage_kills_sooner_than_streaks() {
    // A Fire Crystal (45747: 3000 health) beside the caster: no flight to speak of, and Flame Bolt
    // VI's 126 a try over 3.3 s outpaces the streak's 54 over 1.5 s, cast after cast.
    let (mut c, ids) = guard_off(&["Flame Bolt VI", "Flame Streak VI"], 2.0, 260.0);
    if let Some(it) = c.world.objects.get_mut(&0x8000_0103) {
        it.weenie_class_id = 45747;
        it.name = "Fire Crystal".into();
    }
    let (kill, said) = fastest_at_guard(&c, &ids);
    assert_eq!(kill.attack, ids[0], "{said}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_streak_is_passed_over_when_winning_back_its_mana_makes_it_slower() {
    // The guard of the first test, but mana is kept up by Stamina to Mana and Revitalize: the
    // streak's 60 mana a try costs the round's casts, twice the bolt's 30.
    let names = [
        "Flame Bolt VI",
        "Flame Streak VI",
        "Stamina to Mana Self VI",
        "Revitalize Self VI",
    ];
    let (mut c, ids) = guard_off(&names, 20.0, 260.0);
    c.world.stats.vitals[1].base = 300;
    c.world.stats.vitals[1].current = 300;
    assert!(c.seconds_per_mana() > 0.0, "a mana round");
    let (kill, said) = fastest_at_guard(&c, &ids[..2]);
    assert_eq!(kill.attack, ids[0], "{said}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_wounded_creature_is_finished_with_the_spell_that_flies_first() {
    // 30 health left: Flame Bolt I and VI both kill in one, and Bolt I's Lead Scarab has no windup.
    let (c, ids) = guard_off(&["Flame Bolt VI", "Flame Bolt I"], 10.0, 30.0);
    let (kill, said) = fastest_at_guard(&c, &ids);
    assert_eq!(kill.attack, ids[1], "{said}");
}

/// A caster knowing `names` in the camp's halls under 0x8852 at local `me` in `cell`, a Tusker
/// Guard at `at` in `its`.
fn in_the_halls(
    names: &[&str],
    cell: u32,
    me: glam::Vec3,
    its: u32,
    at: glam::Vec3,
) -> (crate::Client, Vec<u32>) {
    let (mut c, ids) = at_a_tusker_guard(names);
    crate::testkit::stand(&mut c, cell, me);
    if let Some(it) = c.world.objects.get_mut(&0x8000_0103) {
        it.position = Some(ac_world::object::Position::new_flat(its, at));
    }
    (c, ids)
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_bolt_in_a_hall_below_the_grass_flies() {
    // Where Blargerton stood under the Aetherium camp, 23 m below its grass, the trooper 1.8 m
    // off: every flight was taken for one into the ground, so no shot or spot to cast from cleared.
    let me = glam::Vec3::new(84.7, 60.4, -23.6);
    let at = glam::Vec3::new(83.7, 58.9, -23.6);
    let (mut c, ids) = in_the_halls(&["Acid Stream VI"], 0x8852_02D0, me, 0x8852_02CF, at);
    assert!(c.shot_clears(0x8000_0103, crate::dodge::How::Spell(ids[0])));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn an_arc_that_would_strike_the_ceiling_gives_way_to_a_bolt_that_flies_clear() {
    // Blargerton threw Acid Arc VI in the camp's halls ten times running from a spot it burst at
    // as it left; Acid Stream VI does as much and flies flat under the ceiling an arc rises into.
    let me = glam::Vec3::new(72.0, 176.0, -23.6);
    let at = glam::Vec3::new(76.0, 176.0, -23.6);
    let names = ["Acid Arc VI", "Acid Stream VI"];
    let (mut c, ids) = in_the_halls(&names, 0x8852_02AC, me, 0x8852_02AC, at);
    let cfg = c.autoplay.config.fight.clone();
    let now = Instant::now();
    let best = c.best_kill(0x8000_0103, &cfg, now).expect("a kill");
    let ways = c.kills_said(0x8000_0103, &cfg, 4);
    assert_eq!(best.attack, ids[0], "the case: the arc is fastest, {ways}");
    let how = |id| crate::dodge::How::Spell(id);
    assert!(
        !c.shot_clears(0x8000_0103, how(ids[0])),
        "and strikes on the way"
    );
    assert!(
        c.shot_clears(0x8000_0103, how(ids[1])),
        "the bolt flies clear"
    );
    let kill = c.kill_from_here(0x8000_0103, &cfg, now).expect("a kill");
    assert_eq!(kill.attack, ids[1], "{ways}");
}
