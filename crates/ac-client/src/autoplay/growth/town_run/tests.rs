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
    assert!(c.follow.is_none());
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
    assert!(c.follow.is_none());
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
fn peas_for_a_counter_send_a_roomy_pack_to_town_once_the_waits_are_up() {
    // The user's report: two peas tagged to sell, room in the pack,
    // nothing short, and no run was ever made. Now the peas are the
    // reason, with no wait but the one after a futile run.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 50);
    pea_in_the_pack(&mut c, 0x8000_0010, "Iron Pea", 8328, 2_500);
    pea_in_the_pack(&mut c, 0x8000_0011, "Iron Pea", 8328, 2_500);
    let cfg = c.autoplay.config.growth.clone();
    assert!(!c.pack_low_on_room() && !c.laden(&cfg), "the old reasons");
    let now = Instant::now();

    // Just back from a futile run: its wait holds.
    c.autoplay.growth.last_run = Some(now);
    c.autoplay.growth.run_was_futile = true;
    let soon = now + FUTILE_RUN_WAIT - Duration::from_secs(1);
    assert!(!c.grow_town_run(soon, &cfg));
    assert!(
        c.autoplay
            .growth
            .held_back
            .contains("to wait since the last run"),
        "{}",
        c.autoplay.growth.held_back
    );
    // Back from one that sold something, a sale that merely adds up still waits its while.
    c.autoplay.growth.run_was_futile = false;
    assert!(!c.grow_town_run(soon, &cfg));
    assert!(
        c.autoplay.growth.held_back.contains("a sale waits"),
        "{}",
        c.autoplay.growth.held_back
    );
    // Then off, five thousand at face in the pack, to the one counter in town that buys peas.
    let later = now + SALE_RUN_EVERY;
    assert!(
        c.grow_town_run(later, &cfg),
        "{}",
        c.autoplay.growth.held_back
    );
    let run = c.autoplay.growth.run.as_ref().expect("no run");
    assert_eq!(run.errand, Errand::Sell);
    assert!(run.reason.contains("5000 pyreals' worth"), "{}", run.reason);
    assert_eq!(run.vendor, "Archmage Cindrue");
    assert!(c.traveling());

    // One Lead Pea is not worth the trip on its own...
    c.town_run_stop(later);
    c.world.objects.remove(&0x8000_0010);
    c.world.objects.remove(&0x8000_0011);
    pea_in_the_pack(&mut c, 0x8000_0012, "Lead Pea", 8329, 500);
    let again = later + FUTILE_RUN_WAIT;
    assert!(!c.grow_town_run(again, &cfg));
    assert!(
        c.autoplay
            .growth
            .held_back
            .contains("1 thing(s) for a counter, not yet worth the trip"),
        "{}",
        c.autoplay.growth.held_back
    );
    // ...until it has been carried a quarter of an hour.
    let patience = Duration::from_secs_f32(cfg.sell_run_patience);
    assert!(!c.grow_town_run(again + patience / 2, &cfg));
    assert!(
        c.grow_town_run(again + patience, &cfg),
        "{}",
        c.autoplay.growth.held_back
    );
    let run = c.autoplay.growth.run.as_ref().expect("no run");
    assert!(run.reason.contains("carried 15 min"), "{}", run.reason);
    assert_eq!(run.vendor, "Archmage Cindrue");
    // Setting off put the clock back: what comes home unsold is
    // counted afresh.
    assert_eq!(c.autoplay.growth.sale_since, None);
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
    // Five thousand at face in peas, on the walk out to a hunting ground: that walk held the run
    // back as "busy" until the ground was reached. Only a fight in hand does now.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.player_guid = Some(crate::testkit::ME);
    c.world.stats.level = 20;
    with_a_pack(&mut c, 50);
    pea_in_the_pack(&mut c, 0x8000_0010, "Iron Pea", 8328, 2_500);
    pea_in_the_pack(&mut c, 0x8000_0011, "Iron Pea", 8328, 2_500);
    let cfg = c.autoplay.config.growth.clone();
    let now = Instant::now();
    let me = c.player.as_ref().unwrap().world_position();
    let ground = Vec2::new(me.x + 200.0, me.y);
    assert!(c.grow_travel(ground, now), "no walk to the ground");
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
    // One Lead Pea, carried a quarter of an hour, with nobody in the town buying it: a recall
    // makes any trip, so the run goes to a counter that does.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 50);
    pea_in_the_pack(&mut c, 0x8000_0010, "Lead Pea", 8329, 500);
    with_a_buy_list(&mut c, &[]);
    let cfg = c.autoplay.config.growth.clone();
    let now = Instant::now();
    let town = 600.0;
    assert!(nobody_near_buys_peas(&mut c, town, now) > 0);
    let patience = Duration::from_secs_f32(cfg.sell_run_patience);
    c.autoplay.growth.sale_since = Some(now - patience);
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
    // What it tells the party: loot enough for a trip.
    assert!(c.supplies(&cfg, now).sale);
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
