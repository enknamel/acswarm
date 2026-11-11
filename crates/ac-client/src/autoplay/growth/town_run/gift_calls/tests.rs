use super::*;
use crate::autoplay::growth::tests::run_to;
use crate::profile::{LootAction, Rule};
use crate::testkit::{a_loot_profile, character_of_level, no_data, stand, thing_in_the_pack};

/// Town Crier 5772's placement in landblock 0x17B2 (`data/takers.csv`).
const CRIER: u32 = 5772;
const JUNK: &str = "junk to a taker";

fn crier_place() -> Place {
    ac_world::takers::all()
        .iter()
        .find(|t| t.wcid == CRIER && t.place.cell >> 16 == 0x17B2)
        .expect("the crier in 0x17B2")
        .place
}

/// A character standing beside the crier, a profile giving junk to the nearest taker, and one
/// item tagged for it.
fn junk_for_a_taker() -> Client {
    let mut c = character_of_level(no_data(), 20);
    crate::testkit::with_a_pack(&mut c, 50);
    let at = crier_place().at;
    let spot = Place::on_map(at.truncate() + Vec2::new(1.0, 0.0));
    stand(
        &mut c,
        spot.cell,
        spot.at - ac_world::landblock_origin(spot.cell) + glam::Vec3::Z * at.z,
    );
    a_loot_profile(&mut c, "gift-calls-test");
    let mut p = (*c.loot_profile().unwrap()).clone();
    p.rules.insert(
        0,
        Rule {
            name: JUNK.into(),
            action: LootAction::Give,
            give_to: Some(GiveTo::Taker),
            ..Default::default()
        },
    );
    c.profiles.put(p).expect("saved");
    thing_in_the_pack(&mut c, 0x8000_0001, "Useless Attuned Token", 1);
    let stats = c.stats_of(0x8000_0001).unwrap();
    c.autoplay
        .ledger
        .remember_why(&stats, LootAction::Give, Some(JUNK));
    c
}

#[test]
fn a_give_rule_naming_a_taker_becomes_a_call_on_a_run_through_its_town() {
    let now = Instant::now();
    let mut c = junk_for_a_taker();
    // A run to a counter in the crier's landblock.
    let counter = crier_place().at.truncate() + Vec2::new(20.0, 0.0);
    c.autoplay.growth.run = Some(run_to(counter, now));
    c.add_gift_calls(now);
    let run = c.autoplay.growth.run.as_ref().unwrap();
    let gifts: Vec<&GiftCall> = run
        .calls
        .iter()
        .filter_map(|k| match &k.on {
            CallOn::Gift(g) => Some(g),
            _ => None,
        })
        .collect();
    assert_eq!(gifts.len(), 1, "{:?}", run.calls);
    assert_eq!(gifts[0].wcid, Some(CRIER));
    assert_eq!(gifts[0].to, GiveTo::Taker);
    // Nothing tagged for a taker: no call.
    c.autoplay.ledger.forget(0x8000_0001);
    c.autoplay.growth.run = Some(run_to(counter, now));
    c.add_gift_calls(now);
    assert!(c.autoplay.growth.run.as_ref().unwrap().calls.is_empty());
}

#[test]
fn at_the_taker_the_tagged_items_are_given_and_the_run_moves_on() {
    let now = Instant::now();
    let mut c = junk_for_a_taker();
    let me = c.my_position().unwrap();
    let cell = c.player.as_ref().unwrap().cell;
    c.world.objects.insert(
        0x7000_0001,
        ac_world::WorldObject {
            guid: 0x7000_0001,
            name: "Town Crier".into(),
            weenie_class_id: CRIER,
            item_type: ac_world::item_type::CREATURE,
            position: Some(ac_world::object::Position::new_flat(
                cell,
                me + glam::Vec3::X - ac_world::landblock_origin(cell),
            )),
            ..Default::default()
        },
    );
    let at = crier_place();
    let gift = GiftCall {
        to: GiveTo::Taker,
        name: "Town Crier".into(),
        wcid: Some(CRIER),
    };
    let mut run = run_to(at.at.truncate(), now);
    run.phase = Phase::Gift {
        gift,
        at,
        tries: 1,
        handing: false,
    };
    c.autoplay.growth.run = Some(run);
    c.visits.finished = true;
    let cfg = c.autoplay.config.growth.clone();
    assert_eq!(c.grow_run_step(now, &cfg), Turn::Acted);
    let g = c.giving.as_ref().expect("gives under way");
    assert_eq!((g.to, g.items.clone()), (0x7000_0001, vec![0x8000_0001]));
    assert_eq!(c.grow_run_step(now, &cfg), Turn::Waited, "on while giving");
    c.tick_giving(now);
    // The taker took it whole.
    c.world.objects.remove(&0x8000_0001);
    c.tick_giving(now);
    assert!(c.giving.is_none());
    assert_eq!(c.grow_run_step(now, &cfg), Turn::Over);
}
