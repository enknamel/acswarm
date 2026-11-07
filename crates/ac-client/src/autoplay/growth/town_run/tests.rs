use super::*;
use crate::autoplay::growth::needs::NeedKind;
use crate::autoplay::growth::tests::{need, run_to};
use crate::logistics;
use crate::testkit::{
    coin_in_the_pack, pea_in_the_pack, shop_named, standing_at, thing_in_the_pack, with_a_buy_list,
    with_a_pack,
};

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_town_run_between_journeys_is_not_taken_exploring_or_back_to_the_area() {
    // Underground, a corpse on the way ends the run's journey, and
    // exploring ranks above the run: a room chosen then kept the tick
    // for good, so the run never planned its walk again and its clock
    // was never read. Keeping to a hunting area ranks above the run as
    // well, and would have walked the character back to it.
    let renald = Vec2::new(32_587.2, 34_578.3);
    let now = Instant::now();

    // Where the Holtburg Dungeon's portal drops a character.
    let mut c = standing_at(0x01F6_0289, glam::Vec3::new(96.7, -10.0, 0.0));
    c.autoplay.growth.run = Some(run_to(renald, now));
    assert!(!c.traveling());
    assert!(!c.autoplay_explore(now), "went exploring on a run to town");
    assert!(c.going.is_none());
    // With no run, the same dungeon is explored.
    c.autoplay.growth.run = None;
    assert!(c.autoplay_explore(now));

    // Outdoors by the Holtburg lifestone, a field to hunt 40 m off.
    let mut c = standing_at(0xA9B4_0019, glam::Vec3::new(84.0, 7.1, 94.0));
    let me = c.player.as_ref().unwrap().world_position();
    let (x, y) = (me.x + 40.0, me.y);
    let fight = &mut c.autoplay.config.fight;
    fight.enabled = true;
    fight.area = Some(crate::hunt::HuntArea {
        name: "the field".into(),
        shape: crate::hunt::Shape::Outline {
            points: vec![[x, y], [x + 30.0, y], [x + 30.0, y + 30.0], [x, y + 30.0]],
        },
    });
    c.autoplay.growth.run = Some(run_to(renald, now));
    assert!(
        !c.autoplay_keep_to_area(now),
        "walked back to the area on a run to town"
    );
    assert!(c.going.is_none());
    // With no run, outside the field, it goes back to it.
    c.autoplay.growth.run = None;
    assert!(c.autoplay_keep_to_area(now));
}

#[test]
fn a_run_that_could_not_get_there_is_tried_again_once_its_wait_is_up() {
    // A run that really cannot reach its counter comes home with
    // nothing, so it is futile, and the counter is left alone for a
    // while. Neither is for good: the next run is due once the wait
    // between runs is up, and the counter can be chosen again by then.
    let t0 = Instant::now();
    let renald = spot(Vec2::new(32_587.2, 34_578.3));
    let mut skip = crate::did::Patience::new();
    skip.note(
        renald,
        &crate::did::Did::blocked("that counter was no use"),
        t0,
    );
    let wait = wait_between_runs(true);
    assert!(wait > Duration::ZERO, "straight back to the same counter");
    assert!(
        !skip.held(&renald, t0 + wait),
        "the counter is still skipped when the next run is due"
    );
    // One that did buy or sell something may go again at once: a recall makes the trip.
    assert_eq!(wait_between_runs(false), Duration::ZERO);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn loot_for_a_counter_alone_never_sends_a_roomy_pack_to_town() {
    // The user's rule: a run is for a full pack, too much weight or supplies to buy. Ten
    // thousand pyreals of peas in a roomy pack, nothing short, an hour on: still hunting.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 50);
    for (k, guid) in (0x8000_0010..0x8000_0014).enumerate() {
        pea_in_the_pack(&mut c, guid, "Iron Pea", 8328, 2_500 + k as u32);
    }
    let cfg = c.autoplay.config.growth.clone();
    assert!(!c.pack_low_on_room() && !c.laden(&cfg));
    let now = Instant::now();
    for later in [now, now + Duration::from_secs(60 * 60)] {
        assert!(!c.grow_town_run(later, &cfg));
        assert!(
            c.autoplay
                .growth
                .held_back
                .contains("4 thing(s) for a counter, sold on the next run"),
            "{}",
            c.autoplay.growth.held_back
        );
    }
    // The pack running low on room is a reason, and the peas go with it.
    let keep = c.autoplay.config.team.restock.money_slots;
    with_a_pack(&mut c, 4 + keep);
    assert!(c.pack_low_on_room());
    assert!(
        c.grow_town_run(now, &cfg),
        "{}",
        c.autoplay.growth.held_back
    );
    let run = c.autoplay.growth.run.as_ref().expect("no run");
    assert_eq!(run.errand, Errand::Sell);
    assert_eq!(run.reason, "the pack is full");
    assert_eq!(run.vendor, "Archmage Cindrue");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_pack_full_of_what_it_keeps_is_no_reason_to_go_to_town() {
    // The user's rule: sell when there is something to sell, buy when something is needed.
    // Blargerton's pack filled with scrolls and armour the profile keeps; every corpse put it back
    // under the line and he went to town for one 399-pyreal thing, or for nothing.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    let keep = c.autoplay.config.team.restock.money_slots;
    with_a_pack(&mut c, 4 + keep);
    for guid in 0x8000_0010..0x8000_0014 {
        crate::testkit::thing_in_the_pack(&mut c, guid, "Pathwarden Helm", 1);
    }
    let cfg = c.autoplay.config.growth.clone();
    assert!(c.pack_low_on_room());
    assert!(
        !c.grow_town_run(Instant::now(), &cfg),
        "went to town to sell nothing"
    );
    assert!(c.autoplay.growth.run.is_none());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn an_urgent_need_comes_before_the_loot_and_the_loot_is_sold_on_the_way() {
    // An archer out of arrows with an armful of cheap peas. Ranked
    // for the sale first, the run went to the archmage and the
    // arrows waited on a second stop that may not reach the bowyer;
    // the supply comes first, and the peas are sold on the way at
    // the counter a hundred metres on, not carried home for a run
    // of their own.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 50);
    for i in 0..8 {
        pea_in_the_pack(&mut c, 0x8000_0010 + i, "Lead Pea", 8329, 100);
    }
    coin_in_the_pack(&mut c, 0x8000_0030, 1_000);
    // Arrowshafts by their full name: a want called "Arrow" is
    // answered by the archmage's Yarrow.
    with_a_buy_list(&mut c, &[("Bundle of Arrowshafts", 10, 5)]);
    let cfg = c.autoplay.config.growth.clone();
    assert_eq!(c.salables(&cfg).len(), 8, "an armful for a counter");
    let now = Instant::now();
    assert!(
        c.grow_town_run(now, &cfg),
        "{}",
        c.autoplay.growth.held_back
    );
    let run = c.autoplay.growth.run.take().expect("no run");
    assert_eq!(run.errand, Errand::Buy);
    assert!(
        run.reason.contains("short of Bundle of Arrowshafts"),
        "{}",
        run.reason
    );
    assert!(
        shop_named(&run.vendor)
            .stocks("Bundle of Arrowshafts")
            .is_some(),
        "{} has no arrowshafts",
        run.vendor
    );
    assert_ne!(run.vendor, "Archmage Cindrue");
    // Done at the bowyer, the arrows bought: on to the one counter in town that takes the peas,
    // on the same run.
    thing_in_the_pack(&mut c, 0x8000_0040, "Bundle of Arrowshafts", 10);
    c.cancel_travel();
    assert!(c.grow_run_next(run, now, &cfg, None), "went home");
    let on = c
        .autoplay
        .growth
        .run
        .as_ref()
        .expect("went home with the peas");
    assert_eq!(on.vendor, "Archmage Cindrue");
    assert_eq!(on.errand, Errand::Sell);
    assert_eq!(on.stops, 2);
}

/// Every counter within `reach` of the character that would pay for
/// a pea, held off.
pub(super) fn nobody_near_buys_peas(c: &mut Client, reach: f32, now: Instant) -> usize {
    use ac_world::item_type::SPELL_COMPONENTS;
    let me = c.player.as_ref().unwrap().world_position();
    let me = Vec2::new(me.x, me.y);
    let buyers: Vec<Vec2> = ac_world::shops::all()
        .iter()
        .filter(|s| s.xy().distance(me) <= reach)
        .filter(|s| s.pays_for(SPELL_COMPONENTS, 500).is_some())
        .map(|s| s.xy())
        .collect();
    for at in &buyers {
        c.autoplay
            .growth
            .skip_vendors
            .hold(spot(*at), Duration::from_secs(60 * 60), now);
    }
    buyers.len()
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn every_stop_is_chosen_by_one_rule() {
    // The first stop and the ones after were chosen by two sets of rules, and the log read as two
    // planners disagreeing: "nowhere sells what is wanted", then "2 of 2 on the shelf".
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 50);
    let cfg = c.autoplay.config.growth.clone();
    let urgent = need(NeedKind::Named("Prismatic Taper".into()), 100);
    let mut topping_up = urgent.clone();
    topping_up.urgent = false;
    // Nothing to do: the nearest counter, as the panel's button always had it.
    assert_eq!(c.errands_for(&cfg, &[]), vec![Errand::Buy]);
    assert_eq!(
        c.errands_for(&cfg, std::slice::from_ref(&urgent)),
        vec![Errand::Buy]
    );
    pea_in_the_pack(&mut c, 0x8000_0010, "Iron Pea", 8328, 2_500);
    // A supply run out first, the loot sold on the way.
    assert_eq!(
        c.errands_for(&cfg, &[urgent]),
        vec![Errand::Buy, Errand::Sell]
    );
    // Otherwise the loot pays for the shopping.
    assert_eq!(
        c.errands_for(&cfg, &[topping_up]),
        vec![Errand::Sell, Errand::Buy]
    );
    assert_eq!(c.errands_for(&cfg, &[]), vec![Errand::Sell]);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_supply_run_goes_straight_after_the_last_one() {
    // Out of tapers just after a run that sold something: no wait, a recall makes the trip.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.player_guid = Some(crate::testkit::ME);
    c.world.stats.level = 20;
    with_a_pack(&mut c, 50);
    coin_in_the_pack(&mut c, 0x8000_0030, 5_000);
    with_a_buy_list(&mut c, &[("Prismatic Taper", 100, 5)]);
    let cfg = c.autoplay.config.growth.clone();
    let now = Instant::now();
    c.autoplay.growth.last_run = Some(now);
    assert!(
        c.grow_town_run(now, &cfg),
        "{}",
        c.autoplay.growth.held_back
    );
    let run = c.autoplay.growth.run.as_ref().expect("no run");
    assert!(
        run.reason.contains("short of Prismatic Taper"),
        "{}",
        run.reason
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_due_run_leaves_the_walk_to_a_ground_for_town() {
    // A pack low on room, on the walk out to a hunting ground: that walk held the run back as
    // "busy" until the ground was reached. Only a fight in hand does now.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.player_guid = Some(crate::testkit::ME);
    c.world.stats.level = 20;
    let keep = c.autoplay.config.team.restock.money_slots;
    with_a_pack(&mut c, 2 + keep);
    pea_in_the_pack(&mut c, 0x8000_0010, "Iron Pea", 8328, 2_500);
    pea_in_the_pack(&mut c, 0x8000_0011, "Iron Pea", 8328, 2_500);
    let cfg = c.autoplay.config.growth.clone();
    let now = Instant::now();
    let me = c.player.as_ref().unwrap().world_position();
    let ground = Vec2::new(me.x + 200.0, me.y);
    assert!(
        c.grow_travel(ac_world::Place::on_map(ground), crate::travel::ARRIVE),
        "no walk to the ground"
    );
    c.autoplay.growth.bound = Some((holtburg, ground, "the field".into()));

    let it = crate::testkit::standing_by(&mut c, 0x8000_0001, "Drudge Skulker", 3.0);
    c.attack_target = Some(it.guid);
    assert!(!c.grow_town_run(now, &cfg));
    assert_eq!(c.autoplay.growth.held_back, "in a fight");

    c.attack_target = None;
    assert!(
        c.grow_town_run(now, &cfg),
        "{}",
        c.autoplay.growth.held_back
    );
    assert!(
        c.autoplay.growth.bound.is_none(),
        "the walk to the ground is let go"
    );
    let run = c.autoplay.growth.run.as_ref().expect("no run");
    assert_eq!(run.vendor, "Archmage Cindrue");
    assert!(c.traveling());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_run_to_sell_goes_to_whoever_buys_however_far() {
    // A pack low on room, its loot a Lead Pea nobody in the town buys: a recall makes any trip,
    // so the run goes to a counter that does.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    let keep = c.autoplay.config.team.restock.money_slots;
    with_a_pack(&mut c, 1 + keep);
    pea_in_the_pack(&mut c, 0x8000_0010, "Lead Pea", 8329, 500);
    with_a_buy_list(&mut c, &[]);
    let cfg = c.autoplay.config.growth.clone();
    let now = Instant::now();
    let town = 600.0;
    assert!(nobody_near_buys_peas(&mut c, town, now) > 0);
    assert!(
        c.grow_town_run(now, &cfg),
        "{}",
        c.autoplay.growth.held_back
    );
    let run = c.autoplay.growth.run.as_ref().expect("no run");
    assert_eq!(run.errand, Errand::Sell);
    let me = c.player.as_ref().unwrap().world_position();
    assert!(
        run.at.distance(Vec2::new(me.x, me.y)) > town,
        "{}",
        run.vendor
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_party_run_sells_what_this_character_carries_first() {
    // Two on a team that restocks together, the party gone shopping
    // for a mate's full pack, and this one carrying two Iron Peas.
    // The party's run was always a run to buy, so its first stop
    // was whichever counter had the most of the list, with the
    // peas along for the walk; its own pack decides its errand.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 50);
    pea_in_the_pack(&mut c, 0x8000_0010, "Iron Pea", 8328, 2_500);
    pea_in_the_pack(&mut c, 0x8000_0011, "Iron Pea", 8328, 2_500);
    with_a_buy_list(&mut c, &[]);
    c.autoplay.config.team.enabled = true;
    c.autoplay.config.team.restock.together = true;
    c.autoplay.team.mates = vec![crate::autoplay::Mate {
        name: "+Brynith".into(),
        guid: 0x5000_0002,
        ..Default::default()
    }];
    let cfg = c.autoplay.config.growth.clone();
    let now = Instant::now();
    // Loot alone is nothing to tell the party: the mate's full pack is the reason.
    assert!(!c.supplies(&cfg).pack_full);
    c.autoplay.growth.mode = logistics::GroupMode::Restocking(Stage::Shopping);
    c.autoplay.growth.mode_because = "+Brynith's pack is full".into();
    assert!(
        c.grow_town_run(now, &cfg),
        "{}",
        c.autoplay.growth.held_back
    );
    let run = c.autoplay.growth.run.as_ref().expect("no run");
    assert_eq!(run.reason, "+Brynith's pack is full");
    assert_eq!(run.errand, Errand::Sell);
    assert_eq!(run.vendor, "Archmage Cindrue");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_run_short_of_coin_goes_hunting_not_round_the_shops() {
    // A fresh mage with 12 coins and Lead to Copper Scarabs on its list bought one Lead Scarab
    // at an archmage, then walked 12 km to the next for the rest, and the next: it died twice.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 5;
    with_a_pack(&mut c, 50);
    coin_in_the_pack(&mut c, 0x8000_0030, 12);
    with_a_buy_list(
        &mut c,
        &[("Lead Scarab", 50, 10), ("Copper Scarab", 50, 10)],
    );
    let cfg = c.autoplay.config.growth.clone();
    let now = Instant::now();
    assert!(
        c.grow_town_run(now, &cfg),
        "{}",
        c.autoplay.growth.held_back
    );
    let mut run = c.autoplay.growth.run.take().expect("no run");
    // The counter had both and the purse paid for neither in full.
    run.short_with = Some(12);
    c.cancel_travel();
    c.grow_run_next(run.clone(), now, &cfg, None);
    assert!(c.autoplay.growth.run.is_none(), "went on round the shops");
    assert!(
        c.autoplay.growth.run_was_futile,
        "the next run went at once"
    );
    // With more coin since (loot sold on the way), the buying goes on.
    coin_in_the_pack(&mut c, 0x8000_0031, 5_000);
    c.autoplay.growth.run_was_futile = false;
    c.cancel_travel();
    assert!(
        c.grow_run_next(run, now, &cfg, None),
        "went home with coin to spend"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn coin_given_after_a_futile_run_ends_the_wait() {
    // Blargerton, with 1 coin and nothing to sell, found the trip not worth it and waited; handed
    // 181,000 coins, he still walked off to hunt with no Copper Scarabs, waiting out the clock.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 5;
    with_a_pack(&mut c, 50);
    coin_in_the_pack(&mut c, 0x8000_0030, 1);
    with_a_buy_list(&mut c, &[("Copper Scarab", 50, 10)]);
    let cfg = c.autoplay.config.growth.clone();
    let now = Instant::now();
    assert!(!c.grow_town_run(now, &cfg), "set off with 1 coin");
    assert!(
        c.autoplay.growth.run_was_futile,
        "no wait after a futile look"
    );
    let later = now + Duration::from_secs(10);
    assert!(!c.grow_town_run(later, &cfg), "the wait did not hold");
    coin_in_the_pack(&mut c, 0x8000_0031, 181_000);
    assert!(
        c.grow_town_run(later, &cfg),
        "{}",
        c.autoplay.growth.held_back
    );
}
