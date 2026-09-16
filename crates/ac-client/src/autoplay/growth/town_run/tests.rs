use super::*;
use crate::autoplay::growth::tests::run_to;
use crate::logistics;
use crate::testkit::{
    coin_in_the_pack, pea_in_the_pack, shop_named, standing_at, with_a_buy_list, with_a_pack,
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
    for party_restocking in [false, true] {
        let wait = wait_between_runs(party_restocking, true);
        assert!(wait > Duration::ZERO, "straight back to the same counter");
        assert!(wait <= RUN_EVERY);
        assert!(
            !skip.held(&renald, t0 + wait),
            "the counter is still skipped when the next run is due"
        );
    }
    // A party that did buy or sell something may go again at once.
    assert_eq!(wait_between_runs(true, false), Duration::ZERO);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn peas_for_a_counter_send_a_roomy_pack_to_town_once_the_waits_are_up() {
    // The user's report: two peas tagged to sell, room in the pack,
    // nothing short, and no run was ever made. Now the peas are the
    // reason -- once the waits between runs are up, as for any
    // other reason, so one pea does not wear a path to town.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 50);
    pea_in_the_pack(&mut c, 0x8000_0010, "Iron Pea", 8328, 2_500);
    pea_in_the_pack(&mut c, 0x8000_0011, "Iron Pea", 8328, 2_500);
    let cfg = c.autoplay.config.growth.clone();
    assert!(!c.pack_low_on_room() && !c.laden(&cfg), "the old reasons");
    let now = Instant::now();

    // Just back from a run: the wait between runs holds.
    c.autoplay.growth.last_run = Some(now);
    assert!(!c.grow_town_run(now, &cfg));
    assert!(
        c.autoplay
            .growth
            .held_back
            .contains("to wait since the last run"),
        "{}",
        c.autoplay.growth.held_back
    );
    // A futile one holds for its own while.
    c.autoplay.growth.run_was_futile = true;
    let soon = now + FUTILE_RUN_WAIT - Duration::from_secs(1);
    assert!(!c.grow_town_run(soon, &cfg));
    c.autoplay.growth.run_was_futile = false;
    // Waits up, five thousand at face in the pack: off to the one
    // counter in town that buys peas.
    let later = now + RUN_EVERY;
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
    let again = later + RUN_EVERY;
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
    // Done at the bowyer: on to the one counter in town that takes
    // the peas, on the same run.
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
fn a_run_to_sell_for_a_light_reason_stays_within_the_town() {
    // One Lead Pea, carried a quarter of an hour, with nobody in
    // the town buying it: not a walk to an archmage three towns
    // over. A pack that cannot hunt on is worth a walk anywhere; a
    // pea is worth the town the character is in.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 50);
    pea_in_the_pack(&mut c, 0x8000_0010, "Lead Pea", 8329, 500);
    with_a_buy_list(&mut c, &[]);
    let cfg = c.autoplay.config.growth.clone();
    let now = Instant::now();
    assert!(nobody_near_buys_peas(&mut c, SALE_RUN_REACH, now) > 0);
    let patience = Duration::from_secs_f32(cfg.sell_run_patience);
    c.autoplay.growth.sale_since = Some(now - patience);
    assert!(!c.grow_town_run(now, &cfg));
    assert!(c.autoplay.growth.run.is_none());
    assert!(
        c.autoplay
            .growth
            .held_back
            .contains("nobody within 600 m of a way out buys any of the 1 thing(s)"),
        "{}",
        c.autoplay.growth.held_back
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
