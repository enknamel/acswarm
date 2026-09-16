use super::*;
use crate::logistics;
use crate::testkit::{
    pea_in_the_pack, salable, standing_at, vendor_beside, window_of, with_a_buy_list, with_a_pack,
};
use ac_world::equip;

/// A run on its way to a counter at `at`, set off at `now`.
pub(super) fn run_to(at: Vec2, now: Instant) -> Run {
    Run {
        vendor: "Shopkeeper Renald the Elder".into(),
        at,
        phase: Phase::Going,
        since: now,
        last_sell: None,
        town: at,
        stops: 1,
        sold: 0,
        reason: "carrying as much as it means to".into(),
        errand: Errand::Sell,
        visited: vec![at],
        walked_on: None,
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_follower_on_its_own_town_run_is_not_pulled_back_to_its_leader() {
    // A party restocking with everyone going: each follower makes its
    // own run while the leader goes on leading. Following ranks above
    // the run, so each time the follower closed to its following
    // distance the walk after the leader ended the run's journey, the
    // run planned it again, and the walk after the leader ended it
    // again, for the run's four minutes.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    let me = c.player.as_ref().unwrap().world_position();
    let team = &mut c.autoplay.config.team;
    team.enabled = true;
    team.follow = true;
    team.lead = false;
    team.follow_distance = 4.0;
    let leader_off = |metres: f32| crate::autoplay::Mate {
        name: "Leader".into(),
        leader: true,
        leads: true,
        world: me + glam::Vec3::new(metres, 0.0, 0.0),
        cell: holtburg,
        ..Default::default()
    };
    let cfg = c.autoplay.config.growth.clone();
    let now = Instant::now();
    let counter = Vec2::new(me.x + 250.0, me.y);
    assert!(c.grow_travel(counter, now), "no way to the counter");
    c.autoplay.growth.run = Some(run_to(counter, now));

    // The leader walks off and stops, and walks off again: the
    // follower keeps to its own walk.
    for (tick, metres) in [12.0, 3.0, 12.0, 3.0, 30.0, 3.0].into_iter().enumerate() {
        c.autoplay.team.mates = vec![leader_off(metres)];
        assert!(!c.autoplay_follow(now, true), "caught up at tick {tick}");
        assert!(!c.autoplay_follow(now, false), "followed at tick {tick}");
        assert!(
            c.traveling(),
            "its walk to the counter ended at tick {tick}"
        );
        assert!(c.grow_run_step(now, &cfg).goes_on());
        assert!(c.traveling());
    }

    // A corpse on the way does break the walk off, and the run walks
    // on once it is dealt with. Broken off again straight away, the run
    // waits a moment before planning it once more.
    c.interrupt_travel("walking to a corpse");
    assert!(c.journey_broken_off());
    assert!(c.grow_run_step(now, &cfg).goes_on());
    assert!(c.traveling(), "the run did not walk on after a corpse");
    c.interrupt_travel("walking to a corpse");
    let soon = now + Duration::from_secs(1);
    assert!(c.grow_run_step(soon, &cfg).goes_on());
    assert!(!c.traveling(), "planned again straight away");
    assert!(c.grow_run_step(now + WALK_ON_EVERY, &cfg).goes_on());
    assert!(c.traveling());

    // With no run of its own the leader is followed. The walk after it
    // ends the journey without breaking it off: nothing is to pick that
    // journey up again.
    c.autoplay.growth.run = None;
    c.autoplay.team.mates = vec![leader_off(12.0)];
    assert!(c.autoplay_follow(now, false));
    assert!(!c.traveling());
    assert!(!c.journey_broken_off());
}

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
fn a_gem_into_the_hub_puts_the_towns_within_reach() {
    let gem = |g: &ac_world::gems::Gem| ac_world::trip::Gem {
        guid: 1,
        name: g.name.clone(),
        exit: g.xy(),
        exit_cell: g.cell,
        summons: true,
    };
    let named = |n: &str| {
        ac_world::gems::all()
            .iter()
            .find(|g| g.name == n)
            .unwrap_or_else(|| panic!("no {n}"))
    };
    // A gem that lands outdoors is its landing and nothing more.
    let farms = named("Cragstone Farms Portal Gem");
    assert_eq!(gem_ways(&[gem(farms)]).len(), 1);

    // The Town Network gem comes out in the hub, indoors...
    let network = named("Town Network Portal Gem");
    assert!(network.cell & 0xFFFF >= 0x100, "the hub is indoors");
    let ways = gem_ways(&[gem(network)]);
    assert!(ways.len() > 30, "{} ways", ways.len());
    // ...and the counter a mid-level character sells at, the
    // Arcanum Broker outside Cragstone, is a short walk from where
    // one of the hub's portals comes out.
    let broker = ac_world::shops::all()
        .iter()
        .find(|s| s.name == "Arcanum Broker" && s.cell & 0xFFFF_0000 == 0xBB9F_0000)
        .expect("the broker outside Cragstone");
    let (far, via) = ways
        .iter()
        .map(|(w, n)| (broker.xy().distance(*w), n.as_str()))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .unwrap();
    assert!(far <= NEAR_A_WAY_OUT, "{far} m from {via}");
    assert_eq!(via, "Town Network Portal Gem, then Portal to Cragstone");
    // Judged by the hub alone it was out of reach: the fault.
    assert!(broker.xy().distance(network.xy()) > NEAR_A_WAY_OUT);
}

pub(super) fn need(kind: NeedKind, want: u32) -> Need {
    Need {
        name: "x".into(),
        want,
        have: 0,
        keep: want,
        urgent: true,
        buyable: true,
        from: None,
        kind,
    }
}

#[test]
fn a_long_list_is_cut_short() {
    assert_eq!(a_few(&[]), "nothing");
    assert_eq!(a_few(&["Myrrh"]), "Myrrh");
    assert_eq!(a_few(&["a", "b", "c", "d"]), "a, b, c, d");
    assert_eq!(a_few(&["a", "b", "c", "d", "e"]), "a, b, c, d and 1 more");
}

fn ware(wcid: u32, name: &str, value: u32) -> ac_world::shops::Ware {
    ac_world::shops::Ware {
        wcid,
        name: name.into(),
        item_type: ac_world::item_type::SPELL_COMPONENTS,
        value,
    }
}

fn shop(name: &str, sells: Vec<ac_world::shops::Ware>) -> ac_world::shops::Shop {
    ac_world::shops::Shop {
        wcid: 1,
        name: name.into(),
        cell: 0x0001_0001,
        at: glam::Vec3::ZERO,
        sells,
        buys: ac_world::item_type::GEM,
        min_value: 10,
        max_value: 0,
        buy_rate: 0.5,
        sell_rate: 2.0,
        gate: None,
    }
}

#[test]
fn a_counter_behind_a_door_is_not_planned_around() {
    use ac_world::shops::Gate;
    let mut hall = shop(
        "Vermilia the Archmage",
        vec![ware(37155, "Mana Scarab", 15_000)],
    );
    hall.gate = Some(Gate::Society(4));
    // A very good archmage, and no use at all to anyone else's
    // society or to nobody's.
    assert!(hall.open_to(4, &[]));
    assert!(!hall.open_to(2, &[]));
    assert!(!hall.open_to(0, &[]));
    let mut chapter = shop("Rossu Morta Quartermaster", vec![]);
    chapter.gate = Some(Gate::Quest("RossuMortaChapterhouse_Flag".into()));
    assert!(!chapter.open_to(4, &[]), "no society opens a quest door");
    assert!(chapter.open_to(0, &["rossumortachapterhouse_flag".to_string()]));
}

#[test]
fn case_folds_without_allocating() {
    assert!(contains_fold("Prismatic Taper", "taper"));
    assert!(contains_fold("PRISMATIC TAPER", "prismatic"));
    assert!(!contains_fold("Lead Scarab", "taper"));
    assert!(!contains_fold("Tap", "taper"));
    assert!(!contains_fold("anything", ""));
}

#[test]
fn the_counter_that_pays_best_wins_when_neither_has_the_order() {
    // Around Cragstone the Scriveners are eighty metres away and pay
    // half; the Arcanum Broker is three hundred metres further on
    // and pays 0.95. Neither stocks what a hunting character came to
    // buy, so the order is a tie -- and the old ranking fell through
    // to distance, walked past the broker every time, and took half
    // price on every sale.
    let look = |takings: u32, stocks: usize| Forecast {
        takings,
        stocks: vec!["something".into(); stocks],
        ..Default::default()
    };
    let near = look(500, 0);
    let far = look(950, 0);
    assert_eq!(
        better_counter(Errand::Buy, (&far, 380.0), (&near, 80.0)),
        std::cmp::Ordering::Less,
        "the broker is worth the extra three hundred metres"
    );

    // But only when the order is a tie: a counter that has what the
    // character came for still beats a richer one that does not.
    let stocked_but_poor = look(0, 2);
    assert_eq!(
        better_counter(Errand::Buy, (&stocked_but_poor, 380.0), (&far, 80.0)),
        std::cmp::Ordering::Less,
        "what it came to buy comes first"
    );
}

#[test]
fn distance_only_settles_what_pay_and_stock_leave_even() {
    let same = Forecast {
        takings: 100,
        ..Default::default()
    };
    assert_eq!(
        better_counter(Errand::Buy, (&same, 50.0), (&same, 900.0)),
        std::cmp::Ordering::Less,
        "all else equal, the nearer one"
    );
}

#[test]
fn a_trip_is_judged_before_it_is_walked() {
    let mut tapers = need(NeedKind::Component(691), 100);
    tapers.name = "Prismatic Taper".into();
    let mut scarabs = need(NeedKind::Component(690), 20);
    scarabs.name = "Lead Scarab".into();
    let needs = [tapers, scarabs];
    let wants: Vec<(&Need, String)> = needs.iter().map(|n| (n, String::new())).collect();

    let mage = shop(
        "Archmage",
        vec![
            ware(691, "Prismatic Taper", 5),
            ware(690, "Lead Scarab", 30),
        ],
    );
    let smith = shop("Blacksmith", vec![ware(20630, "Trade Note", 250)]);

    // Money in hand, everything on the shelf: the whole order in one
    // stop, and the trip is worth walking.
    let rich = forecast(&mage, &wants, 10_000, &[]);
    assert_eq!(rich.bill, 100 * 10 + 20 * 60);
    assert_eq!(rich.cheapest, 10);
    assert!(rich.missing.is_empty());
    assert!(rich.covers_it());
    assert!(rich.worth_going());

    // Enough for some of it is still worth walking: a mage that can
    // afford half its tapers is a mage that can keep casting.
    let thin = forecast(&mage, &wants, 50, &[]);
    assert!(!thin.covers_it());
    assert!(thin.worth_going());

    // Not a copper, and nothing to sell: the trip buys nothing and
    // is not made.
    let broke = forecast(&mage, &wants, 0, &[]);
    assert_eq!(broke.funds(), 0);
    assert!(!broke.worth_going());

    // Unless there is something to sell, which pays for the rest.
    //
    // Three gems, twelve hundred the lot: a stack's `value` is the
    // whole stack's, which is how the server reckons it and what a
    // counter's limits are measured against. Four hundred each, the
    // shop pays half, so six hundred for the three.
    let loot = [salable(9, ac_world::item_type::GEM, 1_200, 3)];
    let selling = forecast(&mage, &wants, 0, &loot);
    assert_eq!(selling.takings, 200 * 3);
    assert_eq!(selling.selling, 1);
    assert!(selling.worth_going());
    // What it will not buy does not count towards the trip.
    let armour = [salable(9, ac_world::item_type::ARMOR, 400, 1)];
    assert_eq!(forecast(&mage, &wants, 0, &armour).takings, 0);
    // Nor does what is beneath its notice -- and "beneath" is
    // about one of them, not the pile.
    let trinket = [salable(9, ac_world::item_type::GEM, 5, 1)];
    assert_eq!(forecast(&mage, &wants, 0, &trinket).takings, 0);

    // A counter that stocks none of it is no help however rich the
    // character is.
    let wrong = forecast(&smith, &wants, 100_000, &[]);
    assert_eq!(wrong.cheapest, 0);
    assert_eq!(wrong.missing.len(), 2);
    assert!(!wrong.covers_it());
    assert!(!wrong.worth_going());
}

#[test]
fn the_search_widens_rather_than_crossing_the_world() {
    // The bug: ranking on what a shop stocks alone sent a character
    // twenty-five kilometres to a counter with one more line on the
    // shelf. The rings mean a good enough shop in this town wins.
    let r = vendor_rings(None);
    assert_eq!(r.first().copied(), Some(600.0), "the town first");
    assert!(r.windows(2).all(|w| w[0] < w[1]), "{r:?} does not widen");
    assert_eq!(r.last().copied(), Some(f32::INFINITY), "and then anywhere");
}

#[test]
fn a_capped_search_never_looks_past_the_cap() {
    // The next stop of a run stays in the same town.
    let r = vendor_rings(Some(500.0));
    assert_eq!(r, vec![500.0]);
    assert!(r.iter().all(|x| *x <= 500.0));
    // A cap between rings keeps the ones below it.
    let mid = vendor_rings(Some(1_000.0));
    assert_eq!(mid, vec![600.0, 1_000.0]);
}

#[test]
fn there_is_always_a_last_ring_to_fall_back_on() {
    // Whatever the cap, the search ends somewhere rather than
    // leaving the character with nowhere to go.
    for cap in [1.0f32, 600.0, 3_000.0, 15_000.0, 100_000.0] {
        let r = vendor_rings(Some(cap));
        assert!(!r.is_empty(), "cap {cap}");
        assert_eq!(r.last().copied(), Some(cap));
    }
}

#[test]
fn a_healing_kit_is_not_bought_for_someone_who_cannot_use_one() {
    // Untrained Healing: a kit restores next to nothing, so it is
    // not worth the money or the trip to a vendor.
    assert!(!worth_stocking("Healing Kit", false));
    assert!(!worth_stocking("Excellent Healing Kit", false));
    // Trained, it is worth having again.
    assert!(worth_stocking("Healing Kit", true));
    // Everything else is judged on its own, either way.
    assert!(worth_stocking("Prismatic Taper", false));
    assert!(worth_stocking("Mana Stone", false));
}

#[test]
fn ammunition_is_the_plain_kind() {
    use ac_world::fletching::ammo_type;
    assert!(ammo_stock("Arrow", ammo_type::ARROW));
    assert!(ammo_stock("arrow ", ammo_type::ARROW));
    assert!(!ammo_stock("Fire Arrow", ammo_type::ARROW));
    assert!(!ammo_stock("Arrow", ammo_type::BOLT));
    assert!(ammo_stock("Quarrel", ammo_type::BOLT));
    assert!(ammo_stock("Atlatl Dart", ammo_type::ATLATL));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn an_errand_whose_setting_is_turned_off_is_let_go() {
    // Nothing carries a run on with town runs turned off, nor a walk
    // to a ground with grounds off, so neither was ever let go. The
    // errand read as under way for the rest of the session.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let now = Instant::now();
    let to = Vec2::new(32_500.0, 34_500.0);
    let growth = &mut c.autoplay.config.growth;
    growth.auto_xp = false;
    growth.town_runs = false;
    growth.hunt_grounds = false;
    c.autoplay.growth.run = Some(run_to(to, now));
    c.autoplay.growth.bound = Some((0xA9B2, to, "Drudge".into()));
    c.autoplay_grow(now);
    assert!(!c.autoplay.growth.town_run_under_way(), "the run was kept");
    assert_eq!(c.autoplay.growth.bound, None, "the walk was kept");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_run_asked_for_from_the_panel_chooses_a_counter_and_sets_off() {
    // The panel's Step and Run once put the shopping rules straight
    // to the character where it stood. With no window open the
    // rules answered "no counter" and called the trip done, so the
    // buttons did nothing -- and having called it done, went on
    // doing nothing once a window was opened. Now they start the
    // run autoplay would start: a counter is chosen and the walk
    // to it begins, with autoplay off and none of its throttles.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let now = Instant::now();
    assert!(!c.autoplay.config.enabled);
    assert_eq!(c.town_run_driver(), None);
    c.town_run_by_hand(now).expect("no run was started");
    assert_eq!(c.town_run_driver(), Some(Driver::Hand));
    assert!(c.traveling(), "chose a counter and did not set off");
    let v = c.town_run_view(now).expect("no run to show");
    assert_eq!(v.phase, "Going");
    assert_eq!(v.driver, Driver::Hand);
    assert_eq!(v.next, None, "nothing is decided on the walk");
    // The walk is a wait, and a Step waits it out.
    assert_eq!(c.town_run_step_by_hand(now), Turn::Waited);
    assert!(c.traveling());
    // Pressed again, the run under way is the one shown; a second
    // is not started.
    c.town_run_by_hand(now).unwrap();
    assert_eq!(c.town_run_view(now).unwrap().vendor, v.vendor);
    assert_eq!(c.autoplay.growth.run.as_ref().unwrap().since, now);

    // Autoplay turned on meanwhile leaves the run to the panel --
    // it neither steps it nor, with town runs off, lets it go --
    // but counts the character as busy with it.
    c.autoplay.config.enabled = true;
    c.autoplay.config.growth.town_runs = false;
    assert_eq!(c.town_run_driver(), Some(Driver::Hand));
    c.autoplay_grow(now);
    assert!(
        c.autoplay.growth.town_run_under_way(),
        "let go for town runs being off"
    );
    c.autoplay.config.growth.town_runs = true;
    assert!(c.autoplay_grow(now), "the run did not keep the tick");
    assert_eq!(c.town_run_driver(), Some(Driver::Hand));
    assert!(c.traveling(), "autoplay stepped the panel's run");

    // Stop leaves the character as it stands: no run, no walk,
    // nothing to walk up to, no window.
    c.town_run_stop(now);
    assert_eq!(c.town_run_driver(), None);
    assert!(!c.traveling());
    assert!(c.follow.is_none());
    assert!(c.world.open_vendor.is_none());
    assert_eq!(c.autoplay.growth.shop.phase, ac_vendor::Phase::Walking);
    // And autoplay, now running town runs, does not set straight
    // off on one of its own.
    assert!(!c.autoplay_grow(now), "autoplay started a run at once");
    assert_eq!(c.town_run_driver(), None);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_run_started_under_autoplay_is_autoplays_to_step() {
    // With autoplay on and running town runs, Run from the panel
    // starts the run now, and autoplay's tick takes it from there:
    // the panel shows it and does not step it too.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    c.autoplay.config.enabled = true;
    let now = Instant::now();
    c.town_run_by_hand(now).expect("no run was started");
    assert_eq!(c.town_run_driver(), Some(Driver::Autoplay));
    assert!(c.autoplay_grow(now));
    assert!(c.traveling());
    // Until the panel steps it itself, which makes it the panel's.
    assert_eq!(c.town_run_step_by_hand(now), Turn::Waited);
    assert_eq!(c.town_run_driver(), Some(Driver::Hand));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_shopping_rules_start_every_counter_fresh() {
    // The rules remember what a trip offered, what was refused and
    // whether it is done, and were made afresh only when a trip
    // closed its counter. Asked once with no window open they call
    // the trip done, and a run dropped part-way leaves them where
    // they were; either way the next counter opened was closed at
    // once, with nothing sold. Now they are started fresh as each
    // counter's selling begins.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.player_guid = Some(0x5000_0001);
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let cfg = c.autoplay.config.growth.clone();
    // Done, and with a count from some earlier counter.
    let snap = c.vendor_snapshot(&cfg);
    c.autoplay.growth.shop.step(&snap, now);
    assert_eq!(c.autoplay.growth.shop.phase, ac_vendor::Phase::Done);
    c.autoplay.growth.shop.sold = 7;

    // A run at the counter with the pack looked over. Nothing goes
    // out on the way from there to selling, so a Step goes on
    // through it to the first act of the selling.
    let mut run = run_to(Vec2::new(me.x, me.y), now);
    run.phase = Phase::Appraising;
    run.since = now - SETTLE * 3;
    c.autoplay.growth.run = Some(run);
    assert_eq!(c.grow_run_step(now, &cfg), Turn::Waited);
    assert!(matches!(
        c.autoplay.growth.run.as_ref().unwrap().phase,
        Phase::Selling { .. }
    ));
    assert_eq!(
        c.autoplay.growth.shop.phase,
        ac_vendor::Phase::Walking,
        "the rules were not started fresh"
    );
    assert_eq!(c.autoplay.growth.shop.sold, 0);
    assert_eq!(c.town_run_view(now).unwrap().sold, 0);

    // Waiting for a window is a wait; asking again after the
    // counter's time is up is an act.
    let mut run = run_to(Vec2::new(me.x, me.y), now);
    run.phase = Phase::Opening {
        guid: 0x8000_0001,
        tries: 1,
        busy: None,
        asked_over: 0,
    };
    c.autoplay.growth.run = Some(run);
    assert_eq!(c.grow_run_step(now, &cfg), Turn::Waited);
    assert_eq!(c.town_run_view(now).unwrap().phase, "Opening");
    let later = now + VENDOR_OPEN_TIMEOUT + Duration::from_secs(1);
    assert_eq!(c.grow_run_step(later, &cfg), Turn::Acted);
    assert!(matches!(
        c.autoplay.growth.run.as_ref().unwrap().phase,
        Phase::Opening { tries: 2, .. }
    ));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_hand_run_left_held_is_autoplays_again_once_it_runs_town_runs() {
    // Step once with autoplay off, close the panel, turn autoplay
    // on: the run was the panel's for good, autoplay claimed every
    // tick for it and stepped nothing, and the character stood on
    // the road with a blank status line. A run nothing has stepped
    // for a moment is autoplay's to carry on, when it runs town
    // runs.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let now = Instant::now();
    c.town_run_by_hand(now).expect("no run was started");
    assert_eq!(c.town_run_driver(), Some(Driver::Hand));
    c.autoplay.config.enabled = true;
    c.autoplay.config.growth.town_runs = true;
    // Just pressed: still the panel's, and not stepped by autoplay.
    assert!(c.autoplay_grow(now));
    assert_eq!(c.town_run_driver(), Some(Driver::Hand));
    // Left alone: autoplay's, and stepped on.
    let later = now + HAND_HOLD;
    assert!(c.autoplay_grow(later));
    assert_eq!(c.town_run_driver(), Some(Driver::Autoplay));
    assert!(c.autoplay.growth.town_run_under_way());
    assert!(c.traveling());
    // The panel stepping it again takes it back, for as long as it
    // keeps stepping.
    assert_eq!(c.town_run_step_by_hand(later), Turn::Waited);
    assert_eq!(c.town_run_driver(), Some(Driver::Hand));
    assert!(c.autoplay_grow(later));
    assert_eq!(c.town_run_driver(), Some(Driver::Hand));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_hand_run_with_town_runs_off_keeps_the_hunting_off_the_counter() {
    // Autoplay on with town runs off is the panel's to drive, and
    // the run is kept for it -- but with town runs off the run was
    // never looked at by autoplay's tick, so nothing claimed the
    // tick for it and the hunting, finding nothing in sight, walked
    // the character off the open window to look about the ground.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    c.autoplay.config.enabled = true;
    c.autoplay.config.growth.town_runs = false;
    c.autoplay.config.growth.hunt_grounds = true;
    c.autoplay.config.growth.tactic = ac_world::hunting::Tactic::Patrol;
    // On its hunting ground, quiet long enough to move on.
    c.autoplay.growth.hunting_at = Some(holtburg >> 16);
    c.autoplay.growth.quiet_since = Some(now - Duration::from_secs(600));
    // At the counter, waiting on its window, held by the panel.
    let mut run = run_to(Vec2::new(me.x, me.y), now);
    run.phase = Phase::Opening {
        guid: 0x8000_0001,
        tries: 1,
        busy: None,
        asked_over: 0,
    };
    c.autoplay.growth.run = Some(run);
    c.autoplay.growth.by_hand = true;
    assert!(c.autoplay_grow(now), "the run did not keep the tick");
    assert!(!c.traveling(), "the hunting walked off the counter");
    assert_eq!(c.autoplay.growth.bound, None);
    assert!(c.autoplay.growth.town_run_under_way());
    assert!(
        c.autoplay.status.contains("held by the vendoring panel"),
        "nothing said why the character stands still: {:?}",
        c.autoplay.status
    );
    // Without the run the same tick goes looking about the ground,
    // which is what the run was keeping it from.
    c.autoplay.growth.by_hand = false;
    assert!(c.autoplay_grow(now));
    assert!(
        !c.autoplay.growth.town_run_under_way(),
        "kept with town runs off"
    );
    assert!(c.traveling(), "the hunting did not move");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn run_pressed_at_a_counter_the_player_opened_sells_there() {
    // The panel shows what the rules would do at a window the
    // player opened by hand; Run then chose a counter of its own
    // and walked away from the one it had just been showing. At an
    // open window within reach the run is made there, and its
    // first turn is the appraising.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let now = Instant::now();
    let rakk = 0x8000_0002;
    vendor_beside(
        &mut c,
        rakk,
        "Rakk the Peddler",
        glam::Vec3::new(1.0, 0.0, 0.0),
    );
    c.world.open_vendor = Some(window_of(rakk));
    c.town_run_by_hand(now).expect("no run was started");
    let v = c.town_run_view(now).expect("no run to show");
    assert_eq!(v.vendor, "Rakk the Peddler");
    assert_eq!(v.phase, "Opening");
    assert!(!c.traveling(), "walked off to another counter");
    assert_eq!(c.town_run_step_by_hand(now), Turn::Acted);
    assert_eq!(c.town_run_view(now).unwrap().phase, "Appraising");
    assert!(c.world.open_vendor.is_some(), "the window was closed");

    // A window left open from across the town is not a counter at
    // hand: the run chooses one and sets off, as it always did.
    c.town_run_stop(now);
    vendor_beside(
        &mut c,
        rakk,
        "Rakk the Peddler",
        glam::Vec3::new(60.0, 0.0, 0.0),
    );
    c.world.open_vendor = Some(window_of(rakk));
    c.town_run_by_hand(now).expect("no run was started");
    assert_eq!(c.town_run_view(now).unwrap().phase, "Going");
    assert!(c.traveling());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_hand_run_is_refused_with_no_slot_for_the_money() {
    // Autoplay's own refusal lived only in its tick, so the panel's
    // Run walked a pack with no free slot to town, sold nothing
    // there, and came home futile -- which held autoplay's next run
    // back for the longer while.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    let me = 0x5000_0001;
    c.world.player_guid = Some(me);
    c.world.objects.insert(
        me,
        ac_world::WorldObject {
            guid: me,
            name: "Verity".into(),
            is_player: true,
            items_capacity: 1,
            ..Default::default()
        },
    );
    c.world.objects.insert(
        0x8000_0003,
        ac_world::WorldObject {
            guid: 0x8000_0003,
            name: "Dagger".into(),
            value: 50,
            container: Some(me),
            ..Default::default()
        },
    );
    assert_eq!(c.free_space(), 0);
    let now = Instant::now();
    assert_eq!(
        c.town_run_by_hand(now),
        Err("no free slot for a counter's money".into())
    );
    assert_eq!(c.town_run_driver(), None);
    assert!(!c.traveling());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_window_that_opens_after_the_run_stopped_is_closed() {
    // Stop pressed while the counter was being asked for its window
    // left the window to arrive afterwards, and nothing closed it:
    // the tidying refused every pour for "a counter is open" and
    // the panel showed a counter open with nobody at it.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.player_guid = Some(0x5000_0001);
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let rakk = 0x8000_0002;
    let asking = |c: &mut Client| {
        let mut run = run_to(Vec2::new(me.x, me.y), now);
        run.phase = Phase::Opening {
            guid: rakk,
            tries: 1,
            busy: None,
            asked_over: 0,
        };
        c.autoplay.growth.run = Some(run);
    };
    asking(&mut c);
    c.town_run_stop(now);
    assert!(c.world.open_vendor.is_none());
    // The window comes a moment later, and goes.
    c.world.open_vendor = Some(window_of(rakk));
    c.autoplay_close_unwanted_window(now + Duration::from_secs(1));
    assert!(c.world.open_vendor.is_none(), "the window was left open");
    // Another counter's window is not the one that was asked for.
    asking(&mut c);
    c.town_run_stop(now);
    c.world.open_vendor = Some(window_of(0x8000_0009));
    c.autoplay_close_unwanted_window(now + Duration::from_secs(1));
    assert!(
        c.world.open_vendor.is_some(),
        "somebody else's window was closed"
    );
    c.world.open_vendor = None;
    // Long after the run would have given up on it, a window for
    // that counter is the player's own doing.
    asking(&mut c);
    c.town_run_stop(now);
    let late = now + VENDOR_OPEN_TIMEOUT + Duration::from_secs(1);
    c.autoplay_close_unwanted_window(late);
    c.world.open_vendor = Some(window_of(rakk));
    c.autoplay_close_unwanted_window(late);
    assert!(
        c.world.open_vendor.is_some(),
        "the player's window was closed"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_hand_runs_status_stays_with_autoplay_off() {
    // With autoplay off its tick cleared the status every frame,
    // and the run's next turn said it again: a log line, an event
    // and a bus post a frame for the length of the walk. The run
    // the panel is driving keeps its line.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.player_guid = Some(0x5000_0001);
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    c.autoplay.config.enabled = false;
    c.autoplay.growth.run = Some(run_to(Vec2::new(me.x, me.y), now));
    c.autoplay.growth.by_hand = true;
    c.autoplay.say(Doing::Shopping, "going to the counter");
    let said = c.autoplay.announced.len();
    c.tick_autoplay(now);
    assert_eq!(
        c.autoplay.status, "going to the counter",
        "the status was cleared"
    );
    c.autoplay.say(Doing::Shopping, "going to the counter");
    assert_eq!(c.autoplay.announced.len(), said, "said again");
    // A run nobody is driving does not keep autoplay's line up.
    c.autoplay.growth.by_hand = false;
    c.tick_autoplay(now);
    assert!(c.autoplay.status.is_empty());
}

#[test]
fn a_sale_is_ranked_on_what_the_counter_pays() {
    // A tailor with the shopping list on the shelf and no use for
    // the pack, and an archmage with the pack's worth in her purse
    // and nothing on the list. A trip to buy goes to the tailor; a
    // trip to sell goes to the archmage, and the tailor is not so
    // much as a candidate for it.
    let look = |takings: u32, selling: usize, stocks: usize| Forecast {
        takings,
        selling,
        stocks: vec!["something".into(); stocks],
        cheapest: if stocks > 0 { 10 } else { 0 },
        purse: 100,
        ..Default::default()
    };
    let tailor = look(0, 0, 2);
    let archmage = look(2_700, 2, 0);
    assert!(Errand::Buy.served_by(&tailor));
    assert!(!Errand::Sell.served_by(&tailor), "buys none of it");
    assert!(Errand::Sell.served_by(&archmage));
    assert_eq!(
        better_counter(Errand::Buy, (&tailor, 300.0), (&archmage, 20.0)),
        std::cmp::Ordering::Less
    );
    assert_eq!(
        better_counter(Errand::Sell, (&archmage, 300.0), (&tailor, 20.0)),
        std::cmp::Ordering::Less
    );
    // Between two that buy: what they pay, then how much of the
    // pack they take, then the shelf, then the walk.
    let scriveners = look(500, 2, 0);
    let broker = look(950, 2, 0);
    assert_eq!(
        better_counter(Errand::Sell, (&broker, 380.0), (&scriveners, 80.0)),
        std::cmp::Ordering::Less
    );
    let takes_more = look(950, 3, 0);
    assert_eq!(
        better_counter(Errand::Sell, (&takes_more, 380.0), (&broker, 80.0)),
        std::cmp::Ordering::Less
    );
    let stocked_too = look(950, 3, 1);
    assert_eq!(
        better_counter(Errand::Sell, (&stocked_too, 380.0), (&takes_more, 80.0)),
        std::cmp::Ordering::Less
    );
    assert_eq!(
        better_counter(Errand::Sell, (&broker, 80.0), (&broker, 380.0)),
        std::cmp::Ordering::Less
    );
}

#[test]
fn a_run_to_sell_goes_to_a_counter_that_buys_what_it_carries() {
    // Holtburg, with two peas in the pack. Fourteen counters stand
    // in the town and one of them, the archmage, buys spell
    // components; the shops that do so at a worse rate are a few
    // hundred metres out. Ranked the buying way the choice fell to
    // whoever had the most of the shopping list, and the peas went
    // to a counter that would not look at them.
    use ac_world::item_type::{LIFESTONE, SPELL_COMPONENTS};
    let from = ac_world::towns::find("Holtburg").unwrap().world_xy();
    let in_town = || {
        ac_world::shops::all()
            .iter()
            .filter(|s| s.gate.is_none())
            .map(|s| (s, s.xy().distance(from)))
            .filter(|(_, d)| *d <= VENDOR_RINGS[0])
    };
    let peas = [
        salable(1, SPELL_COMPONENTS, 2_500, 1),
        salable(2, SPELL_COMPONENTS, 500, 1),
    ];
    let (shop, look) =
        choose_counter(in_town(), Errand::Sell, &[], 0, &peas).expect("nobody buys peas");
    assert_eq!(shop.name, "Archmage Cindrue");
    assert_eq!(look.selling, 2);
    assert_eq!(look.takings, 2_250 + 450, "nine tenths of both");
    assert!(shop.pays_for(SPELL_COMPONENTS, 2_500).is_some());

    // With a healing kit on the list, the trip to buy goes to a
    // counter with kits -- the pawn shop out on the road, as it
    // happens, which also takes peas at eight tenths...
    let mut kits = need(NeedKind::Named("Healing Kit".into()), 1);
    kits.name = "Healing Kit".into();
    let wants = [(&kits, "healing kit".to_string())];
    let (to_buy, look) =
        choose_counter(in_town(), Errand::Buy, &wants, 10_000, &peas).expect("no kits");
    assert!(to_buy.stocks("Healing Kit").is_some());
    assert!(look.covers_it());
    assert_ne!(to_buy.name, "Archmage Cindrue");
    // ...while the trip to sell, list and all, goes where the peas
    // fetch most.
    let (to_sell, look) = choose_counter(in_town(), Errand::Sell, &wants, 10_000, &peas).unwrap();
    assert_eq!(to_sell.name, "Archmage Cindrue");
    assert!(look.takings > 2_400, "{}", look.takings);

    // Nothing in town buys a lifestone: no counter, and no falling
    // back on the nearest one.
    let odd = [salable(3, LIFESTONE, 1_000, 1)];
    assert!(choose_counter(in_town(), Errand::Sell, &[], 0, &odd).is_none());
    // Nothing wanted and nothing anyone buys is no trip to buy
    // either; the nearest-counter fallback is `pick_vendor`'s own.
    assert!(choose_counter(in_town(), Errand::Buy, &[], 0, &odd).is_none());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_counter_that_buys_none_of_the_loot_is_walked_past_on_a_run_to_sell() {
    // At the counter, with the window open, the pack looked over
    // and nothing on the sale list: the run used to stand there and
    // sell nothing, say "sold 0 item(s)", and walk home with the
    // peas. It says who would not buy them and goes on to who will.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 20);
    pea_in_the_pack(&mut c, 0x8000_0010, "Iron Pea", 8328, 2_500);
    pea_in_the_pack(&mut c, 0x8000_0011, "Lead Pea", 8329, 500);
    let cfg = c.autoplay.config.growth.clone();
    assert_eq!(c.salables(&cfg).len(), 2, "the peas are for a counter");
    let me = c.player.as_ref().unwrap().world_position();
    let me = Vec2::new(me.x, me.y);
    let now = Instant::now();
    let rakk = 0x8000_0002;
    vendor_beside(
        &mut c,
        rakk,
        "Rakk the Peddler",
        glam::Vec3::new(1.0, 0.0, 0.0),
    );
    // A tailor's window: armour and clothing, no components.
    let mut window = window_of(rakk);
    window.item_types = item_type::ARMOR | item_type::CLOTHING;
    c.world.open_vendor = Some(window);
    assert!(c.sale_list(&cfg).is_empty(), "Rakk buys peas");

    let mut run = run_to(me, now);
    run.vendor = "Rakk the Peddler".into();
    run.phase = Phase::Appraising;
    run.since = now - SETTLE * 3;
    c.autoplay.growth.run = Some(run);
    c.grow_run_step(now, &cfg);
    assert!(
        c.autoplay
            .status
            .contains("Rakk the Peddler buys none of this"),
        "{}",
        c.autoplay.status
    );
    assert!(c.world.open_vendor.is_none(), "the window was left open");
    assert!(
        c.autoplay.growth.skip_vendors.held(&spot(me), now),
        "Rakk is not left alone"
    );
    // And left alone for at least a run's wait: on the half minute
    // a blocked counter gets, tidied away at the end of the run,
    // Rakk was the best-paying choice again every RUN_EVERY.
    assert!(
        c.autoplay
            .growth
            .skip_vendors
            .held(&spot(me), now + RUN_EVERY - Duration::from_secs(1)),
        "Rakk is the next run's counter"
    );
    // Not standing at Rakk's counter selling nothing: on to a
    // counter that buys peas, of which Holtburg has one.
    let on = c
        .autoplay
        .growth
        .run
        .as_ref()
        .expect("walked home with the peas");
    assert_eq!(on.vendor, "Archmage Cindrue");
    assert_eq!(on.stops, 2);
    assert_eq!(on.errand, Errand::Sell);
    assert!(c.traveling());

    // A run made to buy stays and shops: the counter is the one the
    // player opened, or the one with the tapers on the shelf.
    c.town_run_stop(now);
    c.world.open_vendor = Some({
        let mut w = window_of(rakk);
        w.item_types = item_type::ARMOR;
        w
    });
    let mut run = run_to(me, now);
    run.vendor = "Rakk the Peddler".into();
    run.errand = Errand::Buy;
    run.phase = Phase::Appraising;
    run.since = now - SETTLE * 3;
    c.autoplay.growth.run = Some(run);
    c.grow_run_step(now, &cfg);
    let stayed = c.autoplay.growth.run.as_ref().expect("the run ended");
    assert_eq!(stayed.vendor, "Rakk the Peddler");
    assert!(matches!(stayed.phase, Phase::Selling { .. }));
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

/// Coin in the pack.
fn coin_in_the_pack(c: &mut Client, guid: u32, amount: u32) {
    let me = c.world.player_guid.unwrap();
    c.world.objects.insert(
        guid,
        ac_world::WorldObject {
            guid,
            name: "Pyreal".into(),
            item_type: item_type::MONEY,
            value: 1,
            stack_size: amount,
            max_stack_size: 25_000,
            container: Some(me),
            ..Default::default()
        },
    );
}

/// Something in the pack by name, `stack` of it, kept.
fn thing_in_the_pack(c: &mut Client, guid: u32, name: &str, stack: u32) {
    let me = c.world.player_guid.unwrap();
    c.world.objects.insert(
        guid,
        ac_world::WorldObject {
            guid,
            name: name.into(),
            value: 1,
            stack_size: stack,
            max_stack_size: 1_000,
            container: Some(me),
            ..Default::default()
        },
    );
}

/// The shop of this name.
fn shop_named(name: &str) -> &'static ac_world::shops::Shop {
    ac_world::shops::all()
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no shop {name}"))
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

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_panel_run_with_a_list_and_a_pea_goes_to_the_counter_with_the_list() {
    // The archer has 180 of 250 arrows -- short, not urgent -- and
    // a pea. The panel's button, made a run to sell whenever the
    // pack held anything for a counter, went to the archmage, sold
    // the pea and ended in town with the arrows unbought. It goes
    // where the list is, and the pea to the archmage after.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 50);
    pea_in_the_pack(&mut c, 0x8000_0010, "Lead Pea", 8329, 500);
    thing_in_the_pack(&mut c, 0x8000_0020, "Bundle of Arrowshafts", 18);
    coin_in_the_pack(&mut c, 0x8000_0030, 1_000);
    with_a_buy_list(&mut c, &[("Bundle of Arrowshafts", 25, 6)]);
    let cfg = c.autoplay.config.growth.clone();
    let now = Instant::now();
    let needs = c.needs_now(now, &cfg);
    assert!(
        needs
            .iter()
            .any(|n| n.name == "Bundle of Arrowshafts" && !n.urgent && n.want == 7),
        "{needs:?}"
    );
    c.town_run_by_hand(now).expect("no run");
    let run = c.autoplay.growth.run.take().expect("no run");
    assert_eq!(run.errand, Errand::Buy);
    assert!(
        shop_named(&run.vendor)
            .stocks("Bundle of Arrowshafts")
            .is_some(),
        "{}",
        run.vendor
    );
    assert_ne!(run.vendor, "Archmage Cindrue");
    c.cancel_travel();
    assert!(
        c.grow_run_next(run, now, &cfg, None),
        "went home with the pea"
    );
    let on = c.autoplay.growth.run.as_ref().unwrap();
    assert_eq!(on.vendor, "Archmage Cindrue");
    assert_eq!(on.errand, Errand::Sell);
}

/// Every counter within `reach` of the character that would pay for
/// a pea, held off.
fn nobody_near_buys_peas(c: &mut Client, reach: f32, now: Instant) -> usize {
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
fn the_panel_run_starts_even_when_nobody_buys_the_pea() {
    // Nothing on the list, a pea in the pack, and no counter near
    // that takes it: the button still starts a run, to the nearest
    // counter, as it always did.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 50);
    pea_in_the_pack(&mut c, 0x8000_0010, "Lead Pea", 8329, 500);
    with_a_buy_list(&mut c, &[]);
    let now = Instant::now();
    assert!(nobody_near_buys_peas(&mut c, SALE_RUN_REACH, now) > 0);
    c.town_run_by_hand(now).expect("no run");
    let run = c.autoplay.growth.run.as_ref().expect("no run");
    assert_eq!(run.errand, Errand::Buy);
    assert_ne!(run.vendor, "Archmage Cindrue");
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

#[test]
fn a_use_turned_away_for_our_own_cast_is_sent_over_not_given_up() {
    use OnOpening::*;
    let (open, shut) = (true, false);
    let (flying, landed) = (true, false);
    let soon = Duration::from_secs(1);
    let long = VENDOR_OPEN_TIMEOUT + Duration::from_secs(1);
    // An open window ends the wait whatever else is going on.
    assert_eq!(on_opening(open, Some(soon), flying, 0, long, 2), Opened);
    // Turned away as busy: wait for our own cast, then a cast's
    // length more (the counter's UseDone frees the slot early), then
    // ask over -- and the retry is not spent, however long it took.
    assert_eq!(on_opening(shut, Some(soon), flying, 0, soon, 1), Wait);
    assert_eq!(on_opening(shut, Some(soon), landed, 0, soon, 1), Wait);
    assert_eq!(on_opening(shut, Some(soon), flying, 0, long, 1), Wait);
    assert_eq!(
        on_opening(shut, Some(BUSY_REASK), landed, 0, long, 1),
        AskOver
    );
    assert_eq!(
        on_opening(shut, Some(BUSY_REASK), landed, 0, long, 2),
        AskOver
    );
    // A counter that keeps saying busy with nothing of ours in the
    // air is taken at its word: the ordinary silence rules apply.
    assert_eq!(
        on_opening(shut, Some(BUSY_REASK), landed, BUSY_ASKS, soon, 1),
        Wait
    );
    assert_eq!(
        on_opening(shut, Some(BUSY_REASK), landed, BUSY_ASKS, long, 1),
        AskAgain
    );
    assert_eq!(
        on_opening(shut, Some(BUSY_REASK), landed, BUSY_ASKS, long, 2),
        GiveUp
    );
    // Silence: one more ask, then the counter is given up.
    assert_eq!(on_opening(shut, None, landed, 0, soon, 1), Wait);
    assert_eq!(on_opening(shut, None, landed, 0, long, 1), AskAgain);
    assert_eq!(on_opening(shut, None, landed, 0, long, 2), GiveUp);
}

#[test]
fn the_counter_is_stood_at_from_the_use_to_the_window_closing() {
    let opening = Phase::Opening {
        guid: 1,
        tries: 1,
        busy: None,
        asked_over: 0,
    };
    let selling = Phase::Selling { sent: Vec::new() };
    assert_eq!(
        counter_asked(Some(&Phase::Going), Some(2)),
        None,
        "still walking: a window open now is one left over"
    );
    assert_eq!(counter_asked(Some(&opening), None), Some(1));
    assert_eq!(counter_asked(Some(&Phase::Appraising), Some(1)), Some(1));
    assert_eq!(counter_asked(Some(&selling), Some(1)), Some(1));
    assert_eq!(
        counter_asked(None, Some(2)),
        Some(2),
        "a window open by hand"
    );
    assert_eq!(counter_asked(None, None), None);
}

/// A weenie error as the server sends it, as a game event body.
fn weenie_error(code: u32) -> Vec<u8> {
    let mut w = ac_net::wire::Writer::new();
    w.u32(0x5000_0001)
        .u32(0)
        .u32(ac_net::messages::event::WEENIE_ERROR)
        .u32(code);
    w.finish()
}

/// A counter in view at `at`.
fn a_counter(c: &mut Client, guid: u32, name: &str, at: glam::Vec3) {
    let mut o = ac_world::WorldObject {
        guid,
        name: name.into(),
        item_type: item_type::CREATURE,
        object_desc_flags: object_desc_flags::VENDOR,
        ..Default::default()
    };
    o.position = Some(ac_world::object::Position {
        cell: 0xA9B4_0019,
        local: at,
        rotation: glam::Quat::IDENTITY,
    });
    c.world.objects.insert(guid, o);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_counter_that_said_busy_is_asked_again_once_the_cast_lands() {
    // +Vesperi's Use of Archmage Cindrue went out with a protection
    // still going up, ACE turned it away as YoureTooBusy, and the
    // run read the counter's silence as a counter that would not
    // trade: one retry spent on the same refusal, then the counter
    // held off as no use, the peas still in the pack.
    let holtburg = 0xA9B4_0019;
    let here = glam::Vec3::new(84.0, 7.1, 94.0);
    let mut c = standing_at(holtburg, here);
    c.world.player_guid = Some(0x5000_0001);
    let me = c.player.as_ref().unwrap().world_position();
    let cfg = Growth::default();
    let now = Instant::now();
    let cindrue = 0x7a9b_4033;
    a_counter(&mut c, cindrue, "Archmage Cindrue", here);
    let mut run = run_to(Vec2::new(me.x, me.y), now);
    run.vendor = "Archmage Cindrue".into();
    run.phase = Phase::Opening {
        guid: cindrue,
        tries: 1,
        busy: None,
        asked_over: 0,
    };
    c.autoplay.growth.run = Some(run);
    // A cast is in the air, and the counter says so.
    c.autoplay.cast_sent = Some(now);
    c.chat_message(
        ac_net::messages::opcode::GAME_EVENT,
        &weenie_error(crate::YOURE_TOO_BUSY),
    );
    assert!(
        matches!(
            c.autoplay.growth.run.as_ref().unwrap().phase,
            Phase::Opening { busy: Some(_), .. }
        ),
        "the refusal was not heard as ours"
    );
    let sent = c.session.actions_sent();
    assert_eq!(c.grow_run_step(now, &cfg), Turn::Waited);
    assert!(
        c.town_run_view(now)
            .unwrap()
            .saying
            .contains("turned the Use away"),
        "the wait is not said"
    );
    // The counter's own UseDone frees the slot a moment after the
    // refusal, with the cast still going up: the ask waits a cast's
    // length after the refusal as well as for the slot.
    c.autoplay.cast_sent = None;
    let soon = now + Duration::from_secs(1);
    assert_eq!(c.grow_run_step(soon, &cfg), Turn::Waited);
    assert_eq!(
        c.session.actions_sent(),
        sent,
        "the Use went out into the cast"
    );
    // Long past the counter's time, with a cast in the air again:
    // no retry is spent and nothing is given up.
    let late = now + VENDOR_OPEN_TIMEOUT + Duration::from_secs(1);
    c.autoplay.cast_sent = Some(late);
    assert_eq!(c.grow_run_step(late, &cfg), Turn::Waited);
    assert_eq!(
        c.session.actions_sent(),
        sent,
        "the Use went out into the cast"
    );
    // The cast lands: the Use goes out again.
    c.autoplay.cast_sent = None;
    let landed = late;
    assert_eq!(c.grow_run_step(landed, &cfg), Turn::Acted);
    assert_eq!(
        c.session.actions_sent(),
        sent + 1,
        "the Use was not sent over"
    );
    let run = c.autoplay.growth.run.as_ref().expect("the run goes on");
    assert!(
        matches!(
            run.phase,
            Phase::Opening {
                tries: 1,
                busy: None,
                asked_over: 1,
                ..
            }
        ),
        "the retry was spent, or the refusal kept: {:?}",
        run.phase
    );
    assert_eq!(run.since, landed, "the counter's time did not start over");
    assert!(
        !c.autoplay.growth.skip_vendors.held(&spot(run.at), landed),
        "the counter was held off as no use"
    );
    // Silence from here on is the counter's own: the ordinary
    // retry, then given up.
    let quiet = landed + VENDOR_OPEN_TIMEOUT + Duration::from_secs(1);
    assert_eq!(c.grow_run_step(quiet, &cfg), Turn::Acted);
    assert!(matches!(
        c.autoplay.growth.run.as_ref().unwrap().phase,
        Phase::Opening { tries: 2, .. }
    ));
    let quieter = quiet + VENDOR_OPEN_TIMEOUT + Duration::from_secs(1);
    c.grow_run_step(quieter, &cfg);
    assert!(
        c.autoplay
            .growth
            .skip_vendors
            .held(&spot(Vec2::new(me.x, me.y)), quieter),
        "a counter that never answered was not held off"
    );
}

/// A caster standing in Holtburg with a wand in hand, the
/// components, the skill and the mana for each of `spells`, and
/// the server's clock known. The spells, in the order asked for.
fn a_caster_knowing(now: Instant, spells: &[&str]) -> (Client, Vec<u32>) {
    let holtburg = 0xA9B4_0019;
    let here = glam::Vec3::new(84.0, 7.1, 94.0);
    let mut c = standing_at(holtburg, here);
    let me = 0x5000_0001;
    c.world.player_guid = Some(me);
    // The server's clock, without which nothing is ever due.
    let clock = ac_net::packet::build(
        ac_net::packet::Header {
            flags: ac_net::packet::flags::TIME_SYNC,
            ..Default::default()
        },
        &1000.0f64.to_le_bytes(),
        &[],
        0,
    );
    c.session.receive(&clock, now);
    assert!(c.session.server_time().is_some(), "the clock was not taken");
    let table = c.assets.spell_table().expect("the spell table");
    let mut known = Vec::new();
    for name in spells {
        let (spell, sp) = table
            .spells
            .iter()
            .find(|(_, sp)| sp.name == *name)
            .map(|(id, sp)| (*id, sp.clone()))
            .unwrap_or_else(|| panic!("the spell table knows {name}"));
        c.world.stats.spells.push(spell);
        let skill = Client::school_skill(sp.school).expect("a school with a skill");
        if !c.world.stats.skills.iter().any(|s| s.id == skill) {
            c.world.stats.skills.push(ac_world::stats::Skill {
                id: skill,
                advancement: ac_world::stats::sac::TRAINED,
                init_level: 300,
                ..Default::default()
            });
        }
        known.push(spell);
    }
    c.world.stats.vitals[2].current = 500;
    const WAND: u32 = 0x8000_0102;
    c.world.objects.insert(
        WAND,
        ac_world::WorldObject {
            guid: WAND,
            name: "Training Wand".into(),
            item_type: item_type::CASTER,
            valid_locations: equip::HELD,
            wielder: Some(me),
            ..Default::default()
        },
    );
    let mapper = c.assets.spell_component_ids().expect("the component ids");
    let mut guid = 0x8000_0200;
    for spell in &known {
        for component in c.current_formula(*spell) {
            let wcid = mapper
                .component_wcid(component)
                .expect("a component with a weenie");
            c.world.objects.insert(
                guid,
                ac_world::WorldObject {
                    guid,
                    name: format!("Component {component}"),
                    weenie_class_id: wcid,
                    stack_size: 20,
                    container: Some(me),
                    ..Default::default()
                },
            );
            guid += 1;
        }
    }
    for spell in &known {
        assert!(
            matches!(c.can_cast(*spell), crate::magic::CastCheck::Ok),
            "the caster cannot cast {spell}: {:?}",
            c.can_cast(*spell)
        );
    }
    (c, known)
}

/// A caster standing in Holtburg with a wand in hand, the
/// components and mana for Blade Protection Self, the server's
/// clock known and no protection up: one buff due, urgent or not.
fn a_caster_with_a_buff_due(now: Instant) -> (Client, u32) {
    let (mut c, known) = a_caster_knowing(now, &["Blade Protection Self I"]);
    c.autoplay.config.buffs.auto = false;
    c.autoplay.config.buffs.spells = vec!["Blade Protection Self".into()];
    (c, known[0])
}

/// A run standing at Archmage Cindrue's counter, the Use just
/// gone out: the counter in view where the character stands, and
/// the run in `phase` there.
fn a_run_at_cindrue(c: &mut Client, phase: Phase, now: Instant) -> u32 {
    let me = c.player.as_ref().unwrap().world_position();
    let cindrue = 0x7a9b_4033;
    a_counter(
        c,
        cindrue,
        "Archmage Cindrue",
        glam::Vec3::new(84.0, 7.1, 94.0),
    );
    let mut run = run_to(Vec2::new(me.x, me.y), now);
    run.vendor = "Archmage Cindrue".into();
    run.phase = phase;
    c.autoplay.growth.run = Some(run);
    cindrue
}

/// The Opening phase as a run enters it.
fn opening(guid: u32) -> Phase {
    Phase::Opening {
        guid,
        tries: 1,
        busy: None,
        asked_over: 0,
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn buffs_wait_at_the_counter_and_go_up_on_the_walk_and_after() {
    // +Vesperi arrived at Archmage Cindrue with eight protections
    // lapsing and cast them one after another from the counter; the
    // buff pass held for a journey and for a fight, and standing at
    // a counter was neither.
    let now = Instant::now();
    let (mut c, _) = a_caster_with_a_buff_due(now);
    let me = c.player.as_ref().unwrap().world_position();
    let cindrue = 0x7a9b_4033;
    a_counter(
        &mut c,
        cindrue,
        "Archmage Cindrue",
        glam::Vec3::new(84.0, 7.1, 94.0),
    );
    // On the walk to town the urgent pass casts as it always did.
    let mut run = run_to(Vec2::new(me.x, me.y), now);
    run.vendor = "Archmage Cindrue".into();
    c.autoplay.growth.run = Some(run.clone());
    let sent = c.session.actions_sent();
    assert!(c.autoplay_buff(now, true), "no cast on the walk");
    assert!(c.session.actions_sent() > sent, "the cast did not go out");
    assert!(!c.autoplay.buffs_held_at_counter);
    // At the counter, from the Use going out, neither pass casts.
    let at_counter = |phase: Phase| {
        let mut run = run.clone();
        run.phase = phase;
        run
    };
    let mut then = now;
    for phase in [
        Phase::Opening {
            guid: cindrue,
            tries: 1,
            busy: None,
            asked_over: 0,
        },
        Phase::Appraising,
        Phase::Selling { sent: Vec::new() },
    ] {
        then += Duration::from_secs(2);
        c.autoplay.growth.run = Some(at_counter(phase.clone()));
        // The window is open once the counter has answered the Use.
        c.world.open_vendor = (!matches!(phase, Phase::Opening { .. })).then(|| window_of(cindrue));
        c.autoplay.cast_sent = None;
        let sent = c.session.actions_sent();
        assert!(!c.autoplay_buff(then, true), "an urgent cast at {phase:?}");
        assert!(!c.autoplay_buff(then, false), "a cast at {phase:?}");
        assert_eq!(
            c.session.actions_sent(),
            sent,
            "something went out at {phase:?}"
        );
        assert!(c.autoplay.buffs_held_at_counter, "the hold was not said");
    }
    // A window the player opened by hand holds the pass the same way.
    c.autoplay.growth.run = None;
    c.world.open_vendor = Some(window_of(cindrue));
    then += Duration::from_secs(2);
    assert!(!c.autoplay_buff(then, true), "a cast into an open window");
    // The window closed, the buff goes up.
    c.world.open_vendor = None;
    then += Duration::from_secs(2);
    let sent = c.session.actions_sent();
    assert!(
        c.autoplay_buff(then, true),
        "no cast once the window closed"
    );
    assert!(c.session.actions_sent() > sent, "the cast did not go out");
    assert!(
        !c.autoplay.buffs_held_at_counter,
        "the hold outlived the counter"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn urgent_buffs_go_up_in_a_fight_at_the_counter() {
    // A wandering monster catches the caster at the counter. The
    // fight outranks the run, which stays in its counter phase
    // throughout, and a hold that read the phase alone kept every
    // protection down for a window nobody was trading at until
    // the fight was over.
    let now = Instant::now();
    let (mut c, _) = a_caster_with_a_buff_due(now);
    let cindrue = 0x7a9b_4033;
    a_run_at_cindrue(&mut c, opening(cindrue), now);
    assert!(
        !c.autoplay_buff(now, true),
        "a cast at the counter with no fight on"
    );
    // Something is hitting the character.
    c.autoplay.last_hit_us = Some(Instant::now());
    let sent = c.session.actions_sent();
    assert!(
        c.autoplay_buff(now, true),
        "the urgent buff waited for the counter through the fight"
    );
    assert!(c.session.actions_sent() > sent, "the cast did not go out");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_window_left_open_from_across_the_town_holds_no_buff() {
    // Nothing closes a window but this client, since the server
    // keeps none. One the player opened and walked away from read
    // as a counter at hand for the rest of the session, and no
    // buff went up again.
    let now = Instant::now();
    let (mut c, _) = a_caster_with_a_buff_due(now);
    let cindrue = 0x7a9b_4033;
    // The counter is a hundred metres off.
    a_counter(
        &mut c,
        cindrue,
        "Archmage Cindrue",
        glam::Vec3::new(184.0, 7.1, 94.0),
    );
    c.world.open_vendor = Some(window_of(cindrue));
    assert_eq!(c.counter_at_hand(), None);
    let sent = c.session.actions_sent();
    assert!(
        c.autoplay_buff(now, true),
        "held for a window left open across the town"
    );
    assert!(c.session.actions_sent() > sent, "the cast did not go out");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn town_runs_switched_off_at_the_counter_close_its_window() {
    // A run let go for town runs being switched off left its
    // window open, and with town runs off no later run would ever
    // close it: a counter at hand for the rest of the session.
    let now = Instant::now();
    let mut c = standing_at(0xA9B4_0019, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.player_guid = Some(0x5000_0001);
    let cindrue = a_run_at_cindrue(&mut c, Phase::Selling { sent: Vec::new() }, now);
    c.world.open_vendor = Some(window_of(cindrue));
    assert_eq!(c.counter_at_hand().as_deref(), Some("Archmage Cindrue"));
    c.autoplay.config.growth.town_runs = false;
    c.autoplay_grow(now);
    assert!(
        !c.autoplay.growth.town_run_under_way(),
        "the run was not let go"
    );
    assert!(
        c.world.open_vendor.is_none(),
        "the run's window was left open"
    );
    assert_eq!(c.counter_at_hand(), None, "still at a counter");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn dying_at_the_counter_ends_the_visit() {
    // The run stood at its counter through the death, the window
    // stayed open, and recovery -- which puts the buffs back before
    // the walk to the corpse -- waited on buffs that waited on the
    // counter.
    let now = Instant::now();
    let mut c = standing_at(0xA9B4_0019, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.player_guid = Some(0x5000_0001);
    let cindrue = a_run_at_cindrue(&mut c, Phase::Selling { sent: Vec::new() }, now);
    c.world.open_vendor = Some(window_of(cindrue));
    // Dead: health at zero on a sheet with a name and an Endurance
    // to draw a maximum from.
    c.world.stats.name = "Vesperi".into();
    c.world.stats.attributes[1].base = 100;
    c.world.stats.vitals[0].current = 0;
    assert!(c.is_dead(), "not dead");
    c.autoplay_recover(now);
    assert!(
        !c.autoplay.growth.town_run_under_way(),
        "the run stood at the counter through the death"
    );
    assert!(c.world.open_vendor.is_none(), "the window was left open");
    assert_eq!(c.counter_at_hand(), None);
    // A run still on its walk is kept: the journey is resumed after.
    let me = c.player.as_ref().unwrap().world_position();
    c.autoplay.growth.run = Some(run_to(Vec2::new(me.x, me.y), now));
    c.counter_left_behind(now);
    assert!(
        c.autoplay.growth.town_run_under_way(),
        "a run on its walk was stopped for a death"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn vitals_wait_at_the_counter() {
    // Only the buffs were held. The vitals pass runs every tick
    // and casts the moment the last cast lands, so a Revitalize
    // thrown from the counter never left it a gap to open its
    // window in: the measured failure, one reflex over.
    let now = Instant::now();
    let (mut c, _) = a_caster_knowing(now, &["Revitalize Self I"]);
    c.autoplay.config.survive.manage_mana = true;
    // The stamina is gone.
    c.world.stats.vitals[1].current = 0;
    // Away from any counter the top-up goes out.
    let sent = c.session.actions_sent();
    assert!(
        c.autoplay_vitals(now),
        "no Revitalize with the stamina gone"
    );
    assert!(c.session.actions_sent() > sent, "the cast did not go out");
    c.autoplay.cast_sent = None;
    // At the counter it waits.
    let cindrue = 0x7a9b_4033;
    a_run_at_cindrue(&mut c, opening(cindrue), now);
    let sent = c.session.actions_sent();
    assert!(!c.autoplay_vitals(now), "a top-up cast at the counter");
    assert_eq!(c.session.actions_sent(), sent, "something went out");
    // Unless a fight has come to it there.
    c.autoplay.last_hit_us = Some(Instant::now());
    assert!(
        c.autoplay_vitals(now),
        "the top-up waited for the counter through a fight"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_use_waits_for_a_cast_on_the_walk_in_to_land() {
    // The first Use went out the moment the character stood at
    // the counter, whatever was in the air: a protection put back
    // on the walk in met it with its recoil, and the counter turned
    // the Use away for it.
    let now = Instant::now();
    let mut c = standing_at(0xA9B4_0019, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.player_guid = Some(0x5000_0001);
    let cfg = Growth::default();
    a_run_at_cindrue(&mut c, Phase::Going, now);
    c.autoplay.cast_sent = Some(now);
    let sent = c.session.actions_sent();
    assert_eq!(c.grow_run_step(now, &cfg), Turn::Waited);
    assert_eq!(
        c.session.actions_sent(),
        sent,
        "the Use went out into the cast"
    );
    assert!(matches!(
        c.autoplay.growth.run.as_ref().unwrap().phase,
        Phase::Going
    ));
    // The cast lands: the counter is asked.
    c.autoplay.cast_sent = None;
    assert_eq!(c.grow_run_step(now, &cfg), Turn::Acted);
    assert_eq!(c.session.actions_sent(), sent + 1, "the Use was not sent");
    assert!(matches!(
        c.autoplay.growth.run.as_ref().unwrap().phase,
        Phase::Opening { .. }
    ));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_refusal_past_the_busy_asks_is_the_counters_own() {
    // Past the asks a busy refusal earns the run is on the ordinary
    // clock, but a later refusal was still kept, and the status
    // promised an ask "once the cast lands" that was never coming.
    let now = Instant::now();
    let mut c = standing_at(0xA9B4_0019, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.player_guid = Some(0x5000_0001);
    let cindrue = 0x7a9b_4033;
    a_run_at_cindrue(
        &mut c,
        Phase::Opening {
            guid: cindrue,
            tries: 1,
            busy: None,
            asked_over: BUSY_ASKS,
        },
        now,
    );
    c.autoplay.growth.counter_said_busy(now);
    assert!(
        matches!(
            c.autoplay.growth.run.as_ref().unwrap().phase,
            Phase::Opening { busy: None, .. }
        ),
        "a refusal past the cap was kept"
    );
    let saying = c.town_run_view(now).unwrap().saying;
    assert!(
        saying.contains("waiting for"),
        "the status still promises an ask: {saying}"
    );
    // Under the cap it is heard.
    a_run_at_cindrue(
        &mut c,
        Phase::Opening {
            guid: cindrue,
            tries: 1,
            busy: None,
            asked_over: BUSY_ASKS - 1,
        },
        now,
    );
    c.autoplay.growth.counter_said_busy(now);
    assert!(matches!(
        c.autoplay.growth.run.as_ref().unwrap().phase,
        Phase::Opening { busy: Some(_), .. }
    ));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_swing_in_the_air_does_not_hold_the_ask() {
    // ACE is never busy for a swing, and a swing's answer can go
    // missing: read raw, one lost AttackDone parked a busy wait in
    // Opening with no clock at all.
    let now = Instant::now();
    let mut c = standing_at(0xA9B4_0019, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.player_guid = Some(0x5000_0001);
    let cfg = Growth::default();
    let cindrue = 0x7a9b_4033;
    a_run_at_cindrue(
        &mut c,
        Phase::Opening {
            guid: cindrue,
            tries: 1,
            busy: Some(now),
            asked_over: 0,
        },
        now,
    );
    c.attack_pending = true;
    c.autoplay.cast_sent = None;
    let sent = c.session.actions_sent();
    let later = now + BUSY_REASK;
    assert_eq!(
        c.grow_run_step(later, &cfg),
        Turn::Acted,
        "the ask waited on a swing"
    );
    assert_eq!(
        c.session.actions_sent(),
        sent + 1,
        "the Use was not sent over"
    );
    assert!(matches!(
        c.autoplay.growth.run.as_ref().unwrap().phase,
        Phase::Opening {
            busy: None,
            asked_over: 1,
            ..
        }
    ));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_hold_is_let_go_by_a_pass_that_left_early() {
    // The note's flag was cleared only on the way past the hold. A
    // pass that turned back before it -- the top-ups, in a fight,
    // out of combat only -- left the flag set after the counter,
    // and the next visit's wait went unsaid.
    let now = Instant::now();
    let (mut c, _) = a_caster_with_a_buff_due(now);
    let cindrue = 0x7a9b_4033;
    a_run_at_cindrue(&mut c, opening(cindrue), now);
    assert!(!c.autoplay_buff(now, true));
    assert!(c.autoplay.buffs_held_at_counter, "the hold was not said");
    // Off the counter and in a fight, the top-up pass turns back
    // at once.
    c.autoplay.growth.run = None;
    c.autoplay.config.buffs.out_of_combat_only = true;
    c.attack_target = Some(0xdead);
    assert!(!c.autoplay_buff(now, false));
    assert!(
        !c.autoplay.buffs_held_at_counter,
        "the flag outlived the counter"
    );
}
