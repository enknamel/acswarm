use super::*;
use crate::autoplay::vitals::buffs::BUFF_CHECK_EVERY;
use crate::autoplay::Style;
use crate::testkit;

/// Health, stamina and mana bars of `most` points each, at these fractions.
fn bars_at(c: &mut Client, most: u32, health: f32, stamina: f32, mana: f32) {
    for (i, f) in [health, stamina, mana].into_iter().enumerate() {
        c.world.stats.vitals[i].base = most;
        c.world.stats.vitals[i].current = (most as f32 * f) as u32;
    }
}

/// A wand fighter in Holtburg knowing `spells`, its bars at full.
fn mage_knowing(now: Instant, spells: &[&str]) -> (Client, Vec<u32>) {
    let (mut c, known) = testkit::a_caster_knowing(now, spells);
    c.autoplay.config.fight.style = Style::Magic;
    bars_at(&mut c, 200, 1.0, 1.0, 1.0);
    (c, known)
}

#[test]
fn a_cast_may_start_at_each_slider_and_not_under_it() {
    let mut c = testkit::offline_client();
    // The 40% mana slider of 200: 80 starts a cast, 79 does not.
    bars_at(&mut c, 200, 1.0, 1.0, 0.4);
    assert_eq!(c.bar_under_slider(), None);
    c.world.stats.vitals[2].current = 79;
    assert_eq!(c.bar_under_slider(), Some("mana"));
    // Stamina under its 30% with mana full: held as well (the banes cast on a spent bar).
    bars_at(&mut c, 200, 1.0, 0.25, 1.0);
    assert_eq!(c.bar_under_slider(), Some("stamina"));
    bars_at(&mut c, 200, 0.5, 1.0, 1.0);
    assert_eq!(c.bar_under_slider(), Some("health"));
    c.autoplay.config.survive.heal_below = 0.0;
    assert_eq!(c.bar_under_slider(), None, "a slider at 0 held a cast");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn an_attack_that_ends_under_the_mana_slider_still_goes() {
    let now = Instant::now();
    let (mut c, known) = mage_knowing(now, &["Flame Bolt I", "Stamina to Mana Self I"]);
    let bolt = known[0];
    let cost = c.spell(bolt).map(|sp| c.mana_cost(&sp)).unwrap();
    assert!(cost > 0);
    // A bar of 1000 at 400: the slider exactly, the bolt takes it under.
    bars_at(&mut c, 1000, 1.0, 1.0, 0.4);
    assert!(!c.hold_for_vitals(now), "held a bolt started at the slider");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn under_the_mana_slider_stamina_is_poured_into_mana_before_the_attack() {
    let now = Instant::now();
    let (mut c, _) = mage_knowing(now, &["Flame Bolt I", "Stamina to Mana Self I"]);
    bars_at(&mut c, 200, 1.0, 1.0, 0.3);
    assert!(c.hold_for_vitals(now), "the bolt went under the slider");
    assert!(
        c.autoplay.status.contains("pouring stamina into mana"),
        "{}",
        c.autoplay.status
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_transfer_goes_at_the_stamina_slider_whatever_it_leaves() {
    let now = Instant::now();
    let (mut c, _) = mage_knowing(now, &["Stamina to Mana Self I"]);
    // Half the stamina bar pours to 25%, under the 30% slider: the transfer goes anyway.
    bars_at(&mut c, 200, 1.0, 0.5, 0.2);
    assert!(c.autoplay_vitals(now), "the transfer waited");
    assert!(
        c.autoplay.status.contains("pouring stamina into mana"),
        "{}",
        c.autoplay.status
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn under_both_sliders_revitalize_goes_first_whatever_the_mana() {
    let now = Instant::now();
    let (mut c, _) = mage_knowing(
        now,
        &[
            "Flame Bolt I",
            "Stamina to Mana Self I",
            "Revitalize Self I",
        ],
    );
    bars_at(&mut c, 200, 1.0, 0.1, 0.3);
    assert!(c.hold_for_vitals(now), "the bolt went under the slider");
    assert!(
        c.autoplay.status.contains("restoring stamina"),
        "{}",
        c.autoplay.status
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn with_nothing_to_restore_mana_the_attack_is_held() {
    let now = Instant::now();
    let (mut c, _) = mage_knowing(now, &["Flame Bolt I"]);
    bars_at(&mut c, 200, 1.0, 1.0, 0.3);
    assert!(c.hold_for_vitals(now), "the bolt went under the slider");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_buff_waits_under_the_stamina_slider() {
    // Stamina to Mana left stamina at 25%: the banes went on, Revitalize waited 50 s behind them.
    let now = Instant::now();
    let (mut c, _) = testkit::a_caster_with_a_buff_due(now);
    c.world.stats.vitals[1].base = 1000;
    c.world.stats.vitals[1].current = 250;
    assert!(
        !c.autoplay_buff(now, false),
        "a buff went on a spent stamina bar"
    );
    c.world.stats.vitals[1].current = 1000;
    assert!(
        c.autoplay_buff(now + BUFF_CHECK_EVERY, false),
        "the buff waited with stamina back"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_buff_waits_under_the_mana_slider_only() {
    let now = Instant::now();
    let (mut c, _) = testkit::a_caster_with_a_buff_due(now);
    c.world.stats.vitals[2].base = 1000;
    // Under the 40% slider.
    c.world.stats.vitals[2].current = 399;
    assert!(
        !c.autoplay_buff(now, false),
        "the buff went under the mana slider"
    );
    c.world.stats.vitals[2].current = 1000;
    assert!(
        c.autoplay_buff(now + BUFF_CHECK_EVERY, false),
        "the buff waited with mana to spare"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_healer_heals_a_fellow_under_its_own_mana_slider() {
    let now = Instant::now();
    let (mut c, _) = testkit::a_caster_knowing(now, &["Heal Other I"]);
    bars_at(&mut c, 200, 1.0, 1.0, 0.1);
    let team = &mut c.autoplay.config.team;
    team.enabled = true;
    team.role = crate::autoplay::Role::Healer;
    team.fellowship = false;
    team.share_supplies = false;
    let mut aldric = testkit::mate(0x5000_0002, "Aldric");
    aldric.health = 0.3;
    c.autoplay.team.mates.push(aldric);
    assert!(c.autoplay_team(now), "no heal: {}", c.autoplay.status);
    assert!(
        c.autoplay.status.contains("healing Aldric"),
        "{}",
        c.autoplay.status
    );
}
