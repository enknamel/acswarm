use super::{choose_heal, restores_health, SelfHeal, Survive, CRITICAL_HEALTH};
use ac_world::vitals::{transfers_between, vital, Transfer};

/// A Heal Self: a fixed number of points, drawing on nothing, and
/// well enough learnt to land nine casts in ten.
fn heal(spell: u32, gain: u32, mana: u32) -> SelfHeal {
    SelfHeal {
        spell,
        gain,
        mana,
        chance: 0.9,
        seconds: 2.0,
        leaves: None,
    }
}

/// A transfer into health: what it gives has already been worked
/// out against the bar it draws on, and `left` is the fraction of
/// that bar the cast would leave behind.
fn transfer(spell: u32, gain: u32, mana: u32, from: u32, left: f32) -> SelfHeal {
    SelfHeal {
        spell,
        gain,
        mana,
        chance: 0.9,
        seconds: 2.0,
        leaves: Some((from, left)),
    }
}

/// Heal Self I through VI: every level costs more and gives more.
fn every_level() -> Vec<SelfHeal> {
    vec![
        heal(1, 25, 10),
        heal(2, 50, 20),
        heal(3, 80, 30),
        heal(4, 110, 40),
        heal(5, 160, 55),
        heal(6, 220, 75),
    ]
}

#[test]
fn a_scratch_is_not_healed_with_the_biggest_spell_in_the_book() {
    let cfg = Survive::default();
    // Twenty points off a full bar: the cheapest level that covers
    // it, not the two hundred and twenty point heal and its mana.
    assert_eq!(choose_heal(&every_level(), 20, 0.9, &cfg), Some(1));
    // A serious wound walks up the book, and stops at the first
    // level that covers it rather than going to the top.
    assert_eq!(choose_heal(&every_level(), 90, 0.5, &cfg), Some(4));
}

#[test]
fn a_wound_nothing_covers_takes_the_biggest_there_is() {
    let cfg = Survive::default();
    // Three hundred missing and nothing in the book reaches it:
    // the most health one cast can give goes out.
    assert_eq!(choose_heal(&every_level(), 300, 0.4, &cfg), Some(6));
}

#[test]
fn a_character_about_to_die_reaches_for_the_biggest_at_once() {
    let cfg = Survive::default();
    // Under a quarter of the bar and three hundred missing: the most one cast gives, mana or not.
    let health = CRITICAL_HEALTH - 0.05;
    assert_eq!(choose_heal(&every_level(), 300, health, &cfg), Some(6));
}

/// `h` taking `seconds` from the cast to the health.
fn taking(mut h: SelfHeal, seconds: f32) -> SelfHeal {
    h.seconds = seconds;
    h
}

/// A mage's book as the data has it: Heal Self I and VI, and Stamina to Health Self IV to VI
/// on `stamina` points of a 400 bar, each timed by its scarab's windup.
fn mage_book(stamina: u32) -> Vec<SelfHeal> {
    let left = |drained: u32| (stamina - drained) as f32 / 400.0;
    let mut book = vec![
        taking(heal(6, 17, 15), 0.9),
        taking(heal(1161, 87, 70), 3.12),
    ];
    for (spell, mana, seconds) in [(1667, 40, 2.33), (1668, 50, 2.73), (1669, 60, 3.12)] {
        let t = ac_world::vitals::transfer(spell).expect("in the table");
        let gain = t.gain(stamina);
        book.push(taking(
            transfer(spell, gain, mana, vital::STAMINA, left(t.drain(stamina))),
            seconds,
        ));
    }
    book
}

#[test]
fn at_a_third_of_health_a_full_stamina_bar_closes_the_wound_fastest() {
    let cfg = Survive::default();
    // 90 of 300 health and a full stamina bar: Stamina to Health IV (240) closes the 210 missing
    // in 2.3 s, before V and VI wind up and long before Heal Self VI's 87 lands.
    assert_eq!(choose_heal(&mage_book(400), 210, 0.3, &cfg), Some(1667));
    // A wound IV and V leave open takes VI: no small transfer on a near-death wound.
    assert_eq!(choose_heal(&mage_book(400), 290, 0.27, &cfg), Some(1669));
}

#[test]
fn a_scratch_still_takes_the_cheap_heal_self() {
    let cfg = Survive::default();
    // A full stamina bar is no reason to pour it into a 15-point scratch.
    assert_eq!(choose_heal(&mage_book(400), 15, 0.95, &cfg), Some(6));
}

#[test]
fn a_near_empty_stamina_bar_leaves_the_emergency_to_heal_self() {
    let cfg = Survive::default();
    // Twenty stamina makes fifteen health at best: Heal Self VI gives more.
    assert_eq!(choose_heal(&mage_book(20), 210, 0.3, &cfg), Some(1161));
}

#[test]
fn an_emergency_spends_stamina_under_its_floor() {
    let cfg = Survive::default();
    // Half a stamina bar: every transfer leaves a quarter, under the floor, and VI still gives 150.
    let book = mage_book(200);
    assert!(book[4]
        .leaves
        .is_some_and(|(_, left)| left < cfg.stamina_below));
    assert_eq!(choose_heal(&book, 210, 0.3, &cfg), Some(1669));
    // Outside an emergency the floor holds and Heal Self VI covers 70.
    assert_eq!(choose_heal(&book, 70, 0.5, &cfg), Some(1161));
}

#[test]
fn an_emergency_keeps_the_mana_floor_until_critical() {
    let cfg = Survive::default();
    let book = vec![
        taking(heal(1161, 87, 70), 3.12),
        taking(transfer(1295, 300, 60, vital::MANA, 0.2), 3.12),
    ];
    // Mana to Health would leave a fifth of the mana the next heal is cast with.
    assert_eq!(choose_heal(&book, 210, 0.3, &cfg), Some(1161));
    assert_eq!(
        choose_heal(&book, 210, CRITICAL_HEALTH - 0.05, &cfg),
        Some(1295)
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_mage_hurt_badly_pours_stamina_into_health() {
    let now = std::time::Instant::now();
    let book = [
        "Heal Self VI",
        "Stamina to Health Self IV",
        "Stamina to Health Self VI",
    ];
    let (mut c, known) = crate::testkit::a_caster_knowing(now, &book);
    // The windup grows with the scarab: level IV is quicker than VI, and a heal and a transfer
    // of one level take the same.
    let secs = |c: &crate::Client, id: u32| c.cast_seconds(&c.spell(id).expect("in the table"));
    let (heal_vi, sth_iv, sth_vi) = (secs(&c, known[0]), secs(&c, known[1]), secs(&c, known[2]));
    assert!((heal_vi - sth_vi).abs() < 1e-3, "{heal_vi} {sth_vi}");
    assert!(
        sth_iv < sth_vi && (2.0..4.0).contains(&sth_vi),
        "{sth_iv} {sth_vi}"
    );
    assert_eq!(c.self_heal_names(), book.map(String::from).to_vec());
    // 90 of 300 health, 400 of 400 stamina: IV's 240 closes the 210 missing soonest.
    c.world.stats.vitals[0].base = 300;
    c.world.stats.vitals[0].current = 90;
    c.world.stats.vitals[1].base = 400;
    c.world.stats.vitals[1].current = 400;
    let missing = 210;
    let cfg = Survive::default();
    let health = c.health_fraction();
    assert_eq!(
        choose_heal(&c.self_heals(), missing, health, &cfg),
        Some(known[1])
    );
}

#[test]
fn a_heal_that_usually_fizzles_is_not_what_a_wound_is_covered_with() {
    let cfg = Survive::default();
    // The big heal is barely learnt and lands three casts in ten.
    let mut shaky = heal(6, 220, 75);
    shaky.chance = 0.3;
    let book = vec![heal(1, 25, 10), shaky];
    assert_eq!(choose_heal(&book, 20, 0.9, &cfg), Some(1));
    // Unless nothing else comes close, and a third of a big heal
    // is still the best there is.
    assert_eq!(choose_heal(&book, 200, 0.4, &cfg), Some(6));
}

#[test]
fn a_transfer_is_not_drained_out_of_a_bar_the_character_needs() {
    let cfg = Survive::default();
    // The transfer is bigger and cheaper, but it would leave
    // stamina at a tenth, under the floor the player set, and a
    // Heal Self covers the wound on its own.
    let book = vec![
        heal(1161, 90, 30),
        transfer(1669, 300, 25, vital::STAMINA, 0.1),
    ];
    assert_eq!(choose_heal(&book, 80, 0.5, &cfg), Some(1161));
    // Mana is guarded the same way: a Mana to Health that would
    // leave nothing to cast with is passed over.
    let book = vec![
        heal(1161, 90, 30),
        transfer(1295, 300, 25, vital::MANA, 0.2),
    ];
    assert_eq!(choose_heal(&book, 80, 0.5, &cfg), Some(1161));
    // Left with room to spare, it is taken like anything else.
    let book = vec![transfer(1669, 120, 25, vital::STAMINA, 0.45)];
    assert_eq!(choose_heal(&book, 100, 0.5, &cfg), Some(1669));
}

#[test]
fn a_character_whose_only_heal_is_a_transfer_still_casts_it() {
    let cfg = Survive::default();
    // Nothing else to reach for: better a bar spent than a
    // character dead, floor or no floor.
    let book = vec![transfer(1669, 300, 25, vital::STAMINA, 0.05)];
    assert_eq!(choose_heal(&book, 200, 0.5, &cfg), Some(1669));
}

#[test]
fn a_dying_character_spends_the_bar_it_was_keeping() {
    let cfg = Survive::default();
    // Eight hundredths of health left and three hundred points
    // missing. The book holds a Heal Self that covers eighty of it
    // and a transfer that covers all of it but would leave stamina
    // at a seventh, under the floor. The floor is what the stamina
    // was being kept for, and there is no next fight to keep it
    // for: the transfer goes out.
    let book = vec![
        heal(3, 80, 30),
        transfer(1669, 300, 25, vital::STAMINA, 0.15),
    ];
    assert_eq!(choose_heal(&book, 300, 0.08, &cfg), Some(1669));
    // Mana is lifted the same way.
    let book = vec![heal(3, 80, 30), transfer(1295, 300, 25, vital::MANA, 0.15)];
    assert_eq!(choose_heal(&book, 300, 0.08, &cfg), Some(1295));
    // Above the line the floor still holds: a wound the Heal Self
    // covers is healed with it and the bar is left alone.
    assert_eq!(choose_heal(&book, 80, 0.5, &cfg), Some(3));
}

#[test]
fn a_character_with_nothing_to_cast_heals_with_nothing() {
    // Out of mana, out of components, or no heal ever learnt: the
    // castable list comes back empty and there is no choice to make.
    assert_eq!(choose_heal(&[], 200, 0.2, &Survive::default()), None);
}

#[test]
fn a_caster_really_does_know_stamina_to_health() {
    // The choice is worth nothing if the table has no such spell.
    let found: Vec<Transfer> = transfers_between(vital::STAMINA, vital::HEALTH);
    assert!(!found.is_empty(), "no stamina to health transfers");
    // The strongest moves half a bar, and on a big bar that beats
    // any fixed heal.
    let best = found.iter().map(|t| t.gain(700)).max().unwrap_or(0);
    assert!(best > 200, "the top transfer only returned {best}");
    // Half the bar goes whatever comes out the other side.
    let top = found.iter().max_by_key(|t| t.gain(700)).expect("one");
    assert_eq!(top.drain(700), 350);
    // Mana to Health is a heal too, and Harm Self is not.
    assert!(!transfers_between(vital::MANA, vital::HEALTH).is_empty());
    assert!(restores_health(1161), "Heal Self VI");
    assert!(restores_health(1669), "Stamina to Health Self VI");
    assert!(restores_health(1295), "Mana to Health Self VI");
    assert!(!restores_health(8), "Harm Self I");
    assert!(
        !restores_health(1182),
        "Revitalize Self VI restores stamina"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_restore_that_mostly_fizzles_gives_way_to_one_that_lands() {
    // Blargerton learned Robustification (Revitalize Self VII) from a scroll and cast it ten
    // times in a row at a Life Magic it fizzles at, with Revitalize Self IV in the book.
    let now = std::time::Instant::now();
    let (mut c, known) =
        crate::testkit::a_caster_knowing(now, &["Revitalize Self IV", "Robustification"]);
    let life = |c: &mut crate::Client, level: u32| {
        let s = c
            .world
            .stats
            .skills
            .iter_mut()
            .find(|s| s.id == 33)
            .unwrap();
        s.init_level = level;
    };
    life(&mut c, 180);
    assert!(c.cast_chance(known[1]) < 0.2, "{}", c.cast_chance(known[1]));
    assert_eq!(c.best_boost(vital::STAMINA), Some(known[0]), "the fizzle");
    life(&mut c, 500);
    assert_eq!(
        c.best_boost(vital::STAMINA),
        Some(known[1]),
        "skill enough for VII"
    );
}
