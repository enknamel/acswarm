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
    // A quarter of the bar left. Even a wound the cheapest heal
    // would cover gets the biggest: there may not be a second cast.
    let health = CRITICAL_HEALTH - 0.05;
    assert_eq!(choose_heal(&every_level(), 20, health, &cfg), Some(6));
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
