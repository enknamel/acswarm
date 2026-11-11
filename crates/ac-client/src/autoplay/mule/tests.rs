use super::*;
use crate::profile::{LootAction, Rule};
use crate::testkit::{a_loot_profile, character_of_level, no_data, stand, thing_in_the_pack};

const HOLTBURG: u32 = 0xA9B4_0019;
const MULE: u32 = 0x5000_0002;
const ARMOUR: &str = "armour to the mule";

/// A character whose profile gives kept armour to "Mule", one breastplate tagged for it.
fn with_armour_for_the_mule() -> Client {
    let mut c = character_of_level(no_data(), 20);
    crate::testkit::with_a_pack(&mut c, 50);
    stand(&mut c, HOLTBURG, glam::Vec3::new(84.0, 7.1, 94.0));
    a_loot_profile(&mut c, "mule-test");
    let mut p = (*c.loot_profile().unwrap()).clone();
    p.rules.insert(
        0,
        Rule {
            name: ARMOUR.into(),
            action: LootAction::Give,
            give_to: Some(GiveTo::Player("Mule".into())),
            ..Default::default()
        },
    );
    c.profiles.put(p).expect("saved");
    thing_in_the_pack(&mut c, 0x8000_0001, "Breastplate", 1);
    let stats = c.stats_of(0x8000_0001).unwrap();
    c.autoplay
        .ledger
        .remember_why(&stats, LootAction::Give, Some(ARMOUR));
    c
}

/// The mule, `metres` east, in view.
fn mule_at(c: &mut Client, metres: f32) {
    let me = c.player.as_ref().unwrap().world_position();
    c.world.objects.insert(
        MULE,
        ac_world::WorldObject {
            guid: MULE,
            name: "+Mule".into(),
            is_player: true,
            position: Some(ac_world::object::Position::new_flat(
                HOLTBURG,
                me + glam::Vec3::X * metres - ac_world::landblock_origin(HOLTBURG),
            )),
            ..Default::default()
        },
    );
}

#[test]
fn a_mule_is_given_its_items_only_in_view_and_in_reach() {
    let now = Instant::now();
    let mut c = with_armour_for_the_mule();
    assert!(!c.autoplay_give(now), "given to a mule not in view");
    mule_at(&mut c, 10.0);
    assert!(!c.autoplay_give(now), "given to a mule out of reach");
    assert!(c.giving.is_none());
    assert!(c.going.is_none(), "walked off to find the mule");
    mule_at(&mut c, 1.0);
    assert!(c.autoplay_give(now));
    let g = c.giving.as_ref().expect("gives under way");
    assert_eq!((g.to, g.items.clone()), (MULE, vec![0x8000_0001]));
    assert!(!g.walk);
    // It waits on its own gives, and stands aside in a fight.
    assert!(c.autoplay_give(now));
    c.giving = None;
    c.attack_target = Some(0x7000_0009);
    assert!(!c.autoplay_give(now));
}

#[test]
fn a_mule_that_refused_is_not_asked_again_for_the_ledgers_wait() {
    let now = Instant::now();
    let mut c = with_armour_for_the_mule();
    mule_at(&mut c, 1.0);
    assert!(c.autoplay_give(now));
    c.tick_giving(now);
    c.hear_refusal("+Mule is not accepting gifts right now", now);
    c.tick_giving(now);
    assert!(c.giving.is_none());
    assert!(!c.autoplay_give(now), "asked again at once");
}
