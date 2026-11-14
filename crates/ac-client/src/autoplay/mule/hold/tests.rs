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

/// A holding mule at Holtburg carrying three things, and `who` (a player) `metres` east.
fn mule_with_three_things(who: &str, metres: f32) -> Client {
    let mut c = character_of_level(no_data(), 1);
    crate::testkit::with_a_pack(&mut c, 50);
    stand(&mut c, HOLTBURG, glam::Vec3::new(84.0, 7.1, 94.0));
    a_loot_profile(&mut c, "mule-deliver-test");
    c.autoplay.config.enabled = true;
    c.autoplay.config.mule = true;
    for g in [0x8000_0001, 0x8000_0002, 0x8000_0003] {
        thing_in_the_pack(&mut c, g, "Salvaged Steel", 1);
    }
    let me = c.player.as_ref().unwrap().world_position();
    c.world.objects.insert(
        0x5000_0009,
        ac_world::WorldObject {
            guid: 0x5000_0009,
            name: who.into(),
            object_desc_flags: ac_world::object_desc_flags::PLAYER,
            position: Some(ac_world::object::Position::new_flat(
                HOLTBURG,
                me + glam::Vec3::X * metres - ac_world::landblock_origin(HOLTBURG),
            )),
            ..Default::default()
        },
    );
    c
}

#[test]
fn a_mule_gives_nothing_to_a_player_beside_it_without_a_delivery() {
    let mut c = mule_with_three_things("Stranger", 1.0);
    c.tick_autoplay(Instant::now());
    assert!(c.giving.is_none());
}

#[test]
fn a_mule_hands_a_delivery_to_its_receiver_alone_and_only_what_was_asked() {
    let now = Instant::now();
    let mut c = mule_with_three_things("Blargerton", 30.0);
    c.set_mule_delivery(Some(crate::autoplay::Delivery {
        to: "Blargerton".into(),
        items: vec![0x8000_0001, 0x8000_0002],
    }));
    c.tick_autoplay(now);
    assert!(c.giving.is_none(), "the receiver is not yet in reach");
    assert!(
        c.autoplay
            .status
            .contains("holding 2 item(s) for Blargerton"),
        "{}",
        c.autoplay.status
    );
    let mut d = mule_with_three_things("Blargerton", 1.0);
    d.set_mule_delivery(Some(crate::autoplay::Delivery {
        to: "Blargerton".into(),
        items: vec![0x8000_0001, 0x8000_0002],
    }));
    d.tick_autoplay(now);
    let g = d.giving.as_ref().expect("handing it over");
    assert_eq!(g.to, 0x5000_0009);
    // All of it gone: the delivery is over.
    d.giving = None;
    d.world.objects.remove(&0x8000_0001);
    d.world.objects.remove(&0x8000_0002);
    d.tick_autoplay(now);
    assert!(d.autoplay.muling.deliver.is_none());
    assert!(d.giving.is_none(), "the third thing stays");
}
