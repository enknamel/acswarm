use super::*;
use crate::testkit::{a_loot_profile, character_of_level, no_data, stand, thing_in_the_pack};
use ac_world::item_type as kind;

const HOLTBURG: u32 = 0xA9B4_0019;
const MULE: u32 = 0x5000_0002;

/// A character of a 4-slot pack at Holtburg, judging by a profile of its own.
fn hunter() -> Client {
    let mut c = character_of_level(no_data(), 75);
    crate::testkit::with_a_pack(&mut c, 4);
    c.world.stats.name = "Hunter".into();
    stand(&mut c, HOLTBURG, glam::Vec3::new(84.0, 7.1, 94.0));
    a_loot_profile(&mut c, "mule-load-test");
    c.autoplay.config.growth.mule_runs = true;
    c
}

/// `name` of `item_type` in the pack, appraised, tagged `tag`.
fn carried(c: &mut Client, guid: u32, name: &str, item_type: u32, tag: LootAction) {
    thing_in_the_pack(c, guid, name, 1);
    c.world.objects.get_mut(&guid).unwrap().item_type = item_type;
    c.appraisals.insert(
        guid,
        ac_net::messages::Appraisal {
            guid,
            success: true,
            ..Default::default()
        },
    );
    let stats = c.stats_of(guid).unwrap();
    c.autoplay.ledger.remember(&stats, tag);
}

/// The mule, `metres` east, in view.
fn mule_at(c: &mut Client, metres: f32) -> ac_world::Place {
    let me = c.player.as_ref().unwrap().world_position();
    let at = me + glam::Vec3::X * metres;
    c.world.objects.insert(
        MULE,
        ac_world::WorldObject {
            guid: MULE,
            name: "Mule".into(),
            object_desc_flags: ac_world::object_desc_flags::PLAYER,
            position: Some(ac_world::object::Position::new_flat(
                HOLTBURG,
                at - ac_world::landblock_origin(HOLTBURG),
            )),
            ..Default::default()
        },
    );
    ac_world::Place { at, cell: HOLTBURG }
}

#[test]
fn what_is_kept_goes_to_a_mule_but_never_what_is_tinkered_inscribed_worn_or_a_tool() {
    let mut c = hunter();
    let (kept, tinkered, inscribed, sold, worn, kit) = (
        0x8000_0001,
        0x8000_0002,
        0x8000_0003,
        0x8000_0004,
        0x8000_0005,
        0x8000_0006,
    );
    carried(&mut c, kept, "Breastplate", kind::ARMOR, LootAction::Keep);
    carried(
        &mut c,
        tinkered,
        "Tinkered Gauntlets",
        kind::ARMOR,
        LootAction::Keep,
    );
    c.appraisals.get_mut(&tinkered).unwrap().ints = vec![(171, 3)];
    carried(
        &mut c,
        inscribed,
        "Inscribed Ring",
        kind::JEWELRY,
        LootAction::Keep,
    );
    c.appraisals.get_mut(&inscribed).unwrap().strings = vec![(7, "For Bryn".into())];
    carried(
        &mut c,
        sold,
        "Rusty Mace",
        kind::MELEE_WEAPON,
        LootAction::Sell,
    );
    carried(&mut c, worn, "Worn Helm", kind::ARMOR, LootAction::Keep);
    let w = c.world.objects.get_mut(&worn).unwrap();
    w.container = None;
    w.wielder = c.world.player_guid;
    carried(&mut c, kit, "Odd Kit", kind::MISC, LootAction::Keep);
    c.world.objects.get_mut(&kit).unwrap().usable = 0x0010_0008;
    assert_eq!(c.mule_load(), vec![kept]);
}

#[test]
fn a_full_bag_of_salvage_goes_to_a_mule_and_a_part_bag_stays_to_be_topped_up() {
    let mut c = hunter();
    let (full, part) = (0x8000_0011, 0x8000_0012);
    for (g, units) in [(full, 100), (part, 40)] {
        carried(
            &mut c,
            g,
            "Salvaged Steel",
            kind::TINKERING_MATERIAL,
            LootAction::Salvage,
        );
        c.world.objects.get_mut(&g).unwrap().structure = units;
    }
    assert_eq!(c.mule_load(), vec![full]);
}

#[test]
fn a_low_pack_wants_a_mule_goes_to_it_loads_it_and_lets_it_go() {
    let now = Instant::now();
    let mut c = hunter();
    for g in 0..4 {
        carried(
            &mut c,
            0x8000_0020 + g,
            "Breastplate",
            kind::ARMOR,
            LootAction::Keep,
        );
    }
    assert!(
        !c.autoplay_mule(now),
        "nothing goes before a mule is wanted"
    );
    assert!(
        c.is_mule_wanted(),
        "a full pack of kept things wants a mule"
    );
    assert!(
        !c.autoplay_mule(now),
        "nowhere to go before the mule stands somewhere"
    );
    let place = mule_at(&mut c, 60.0);
    c.set_mule_meet(Some(Meet {
        name: "Mule".into(),
        place,
    }));
    assert!(c.autoplay_mule(now), "set off for the mule");
    assert!(c.giving.is_none(), "nothing given out of reach");
    mule_at(&mut c, 2.0);
    assert!(c.autoplay_mule(now), "loading the mule");
    assert_eq!(c.giving.as_ref().map(|g| g.to), Some(MULE));
    // All of it taken.
    c.giving = None;
    for g in 0..4 {
        c.world.objects.remove(&(0x8000_0020 + g));
    }
    assert!(!c.autoplay_mule(now));
    assert!(!c.is_mule_wanted(), "loaded: the mule may log off");
    assert!(!c.is_mule_full());
}

#[test]
fn a_character_fetching_a_delivery_goes_to_the_mule_and_waits_by_it() {
    let now = Instant::now();
    let mut c = hunter();
    let place = mule_at(&mut c, 60.0);
    c.set_mule_fetch(Some(Meet {
        name: "Mule".into(),
        place,
    }));
    assert!(c.autoplay_mule(now), "set off for the mule");
    assert!(c.autoplay.status.contains("for a delivery"));
    mule_at(&mut c, 2.0);
    assert!(c.autoplay_mule(now), "stands by while it hands things over");
    assert!(c.autoplay.status.contains("taking a delivery from Mule"));
    c.set_mule_fetch(None);
    assert!(!c.autoplay_mule(now), "all come: nothing more to do");
}

#[test]
fn what_a_mule_delivered_is_never_loaded_back() {
    let mut c = hunter();
    carried(
        &mut c,
        0x8000_0031,
        "Breastplate",
        kind::ARMOR,
        LootAction::Keep,
    );
    carried(
        &mut c,
        0x8000_0032,
        "Breastplate",
        kind::ARMOR,
        LootAction::Keep,
    );
    c.set_mule_kept(vec![0x8000_0031]);
    assert_eq!(c.mule_load(), vec![0x8000_0032]);
}
