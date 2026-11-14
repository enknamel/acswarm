use super::*;
use crate::autoplay::LootAction;
use crate::testkit::{a_loot_profile, character_of_level, no_data, stand, thing_in_the_pack};

const HOLTBURG: u32 = 0xA9B4_0019;

#[test]
fn a_mule_out_of_the_academy_only_holds_what_it_is_given() {
    let now = Instant::now();
    let mut c = character_of_level(no_data(), 1);
    crate::testkit::with_a_pack(&mut c, 2);
    stand(&mut c, HOLTBURG, glam::Vec3::new(84.0, 7.1, 94.0));
    a_loot_profile(&mut c, "mule-hold-test");
    c.autoplay.config.enabled = true;
    c.autoplay.config.growth.town_runs = true;
    // A full pack of things its profile would sell and salvage.
    for (g, tag) in [
        (0x8000_0001, LootAction::Sell),
        (0x8000_0002, LootAction::Salvage),
    ] {
        thing_in_the_pack(&mut c, g, "Rusty Mace", 1);
        let stats = c.stats_of(g).unwrap();
        c.autoplay.ledger.remember(&stats, tag);
    }
    c.autoplay.config.mule = true;
    assert!(c.is_mule_holding());
    c.tick_autoplay(now);
    assert_eq!(c.autoplay.step, None, "no row ran");
    assert!(c.autoplay.status.starts_with("holding 2 item(s)"));
    assert!(
        c.going.is_none() && !c.traveling(),
        "it stays where it stands"
    );
    let allow = crate::options::option_by_name("let other players give you items").unwrap();
    assert!(c.option_enabled(allow), "gifts are let in");
}

#[test]
fn a_mule_in_the_academy_does_the_academy_first() {
    let mut c = character_of_level(no_data(), 1);
    stand(&mut c, 0x8602_0100, glam::Vec3::new(60.0, -60.0, 0.0));
    c.autoplay.config.mule = true;
    assert!(!c.is_mule_holding());
}
