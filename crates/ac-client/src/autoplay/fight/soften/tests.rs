use super::*;

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_leader_bolts_the_guard_it_made_vulnerable_and_does_not_move_on_to_the_next() {
    // Blargerton led with focus fire on: once Piercing Vulnerability V was on a Tusker Guard, the
    // party plan sent him to the next guard not yet vulned, 21 vulns and no bolt in two minutes.
    let now = Instant::now();
    let (mut c, ids) = crate::testkit::a_caster_knowing(
        now,
        &["Force Bolt VI", "Piercing Vulnerability Other VI"],
    );
    c.autoplay.config.enabled = true;
    c.autoplay.config.fight.enabled = true;
    c.autoplay.config.team.enabled = true;
    c.autoplay.config.team.focus_fire = true;
    c.autoplay.team.leader = true;
    let (first, second) = (0x8000_0103, 0x8000_0104);
    for (guid, metres) in [(first, 6.0), (second, 8.0)] {
        crate::testkit::standing_by(&mut c, guid, "Tusker Guard", metres);
        if let Some(o) = c.world.objects.get_mut(&guid) {
            o.weenie_class_id = 1629;
        }
    }
    c.autoplay.casting_at = Some(first);
    c.autoplay.mark_vulned(first, Some((Element::Pierce, 2.1)));
    let plan = c.plan_for_team(now);
    let me = c.world.player_guid.expect("a player");
    assert_eq!(
        plan.orders.get(&me).and_then(|o| o.target),
        Some(first),
        "{plan:?}"
    );
    let cfg = c.autoplay.config.fight.clone();
    c.autoplay_fight_with_spells(now, &cfg);
    assert_eq!(c.autoplay.casting_at, Some(first));
    assert_eq!(
        c.autoplay.attack_spell,
        Some(ids[0]),
        "Force Bolt VI thrown"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn magic_yield_goes_on_before_the_vulnerability_and_a_resist_has_it_cast_again() {
    let now = Instant::now();
    let (mut c, _) = crate::testkit::a_caster_knowing(
        now,
        &[
            "Force Bolt VI",
            "Piercing Vulnerability Other VI",
            "Futility",
        ],
    );
    // Creature Enchantment, Life and War Magic.
    for skill in [31, 33, 34] {
        crate::testkit::trained_in(&mut c, skill, 350);
    }
    c.autoplay.config.enabled = true;
    c.autoplay.config.fight.enabled = true;
    let it = 0x8000_0103;
    crate::testkit::standing_by(&mut c, it, "Titanium Armoredillo", 6.0);
    if let Some(o) = c.world.objects.get_mut(&it) {
        o.weenie_class_id = 7083;
    }
    let cfg = c.autoplay.config.fight.clone();
    c.autoplay_fight_with_spells(now, &cfg);
    assert_eq!(c.autoplay.yielded_by(it), 40.0, "Futility first");
    assert!(!c.autoplay.is_vulned(it));
    // Resisted: cast again.
    c.hear_arrival("Titanium Armoredillo resists your spell");
    assert_eq!(c.autoplay.yielded_by(it), 0.0);
    c.autoplay.cast_sent = None;
    c.autoplay_fight_with_spells(now, &cfg);
    assert_eq!(c.autoplay.yielded_by(it), 40.0, "Futility again");
    // Landed: the vulnerability next.
    c.autoplay.cast_sent = None;
    c.autoplay_fight_with_spells(now, &cfg);
    assert!(c.autoplay.is_vulned(it), "then Piercing Vulnerability");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_magic_yield_and_a_vulnerability_are_cast_again_before_they_run_out() {
    // In a long fight the debuffs ran out and were never cast again; a Magic Yield let lapse
    // is resisted at the creature's whole magic defence.
    let now = Instant::now();
    let (mut c, _) = crate::testkit::a_caster_knowing(
        now,
        &[
            "Force Bolt VI",
            "Piercing Vulnerability Other VI",
            "Futility",
        ],
    );
    for skill in [31, 33, 34] {
        crate::testkit::trained_in(&mut c, skill, 350);
    }
    c.autoplay.config.enabled = true;
    c.autoplay.config.fight.enabled = true;
    let it = 0x8000_0103;
    crate::testkit::standing_by(&mut c, it, "Titanium Armoredillo", 6.0);
    if let Some(o) = c.world.objects.get_mut(&it) {
        o.weenie_class_id = 7083;
    }
    let cfg = c.autoplay.config.fight.clone();
    c.autoplay_fight_with_spells(now, &cfg);
    c.autoplay.cast_sent = None;
    c.autoplay_fight_with_spells(now, &cfg);
    assert!(
        c.autoplay.yielded_by(it) > 0.0 && c.autoplay.is_vulned(it),
        "both on"
    );
    let futility = c.spell_by_name("Futility").unwrap();
    let lasts = c.spell(futility).and_then(|sp| sp.duration()).unwrap();
    let lasts = std::time::Duration::from_secs_f64(lasts);
    // Still on short of the renewal.
    let early = now + lasts - super::RENEW_BEFORE - std::time::Duration::from_secs(1);
    c.lapse_debuffs(early);
    assert!(c.autoplay.yielded_by(it) > 0.0, "kept while it lasts");
    // Due again before it ends: cast again.
    let due = now + lasts - super::RENEW_BEFORE;
    assert!(due < now + lasts);
    c.lapse_debuffs(due);
    assert_eq!(c.autoplay.yielded_by(it), 0.0, "Futility due again");
    // The fight's choice puts it on again (a stood fight minutes on reads as stalled offline).
    let kill = c.best_kill(it, &cfg, due).expect("a kill");
    assert!(kill.magic_yield.is_some(), "Futility first again: {kill:?}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_vulnerability_is_cast_from_within_its_reach() {
    // Blargerton, 60 m from a Tusker Slave a storey down, sent Fire Vulnerability Other VI seven
    // times a second: each came back out of range (0x550), was cast again, and he never moved.
    let now = Instant::now();
    let (mut c, _) = crate::testkit::a_caster_knowing(
        now,
        &["Force Bolt VI", "Piercing Vulnerability Other VI"],
    );
    c.autoplay.config.enabled = true;
    c.autoplay.config.fight.enabled = true;
    c.autoplay.config.fight.radius = 100.0;
    let it = 0x8000_0103;
    crate::testkit::standing_by(&mut c, it, "Tusker Guard", 60.0);
    if let Some(o) = c.world.objects.get_mut(&it) {
        o.weenie_class_id = 1629;
    }
    c.autoplay.casting_at = Some(it);
    let cfg = c.autoplay.config.fight.clone();
    c.autoplay_fight_with_spells(now, &cfg);
    assert!(
        c.autoplay.debuff_sent.is_none() && !c.autoplay.is_vulned(it),
        "a vulnerability sent from 60 m: {}",
        c.autoplay.status
    );
}
