use super::*;
use crate::autoplay::growth::tests::need;
use crate::testkit::salable;

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
fn the_counter_named_is_asked_even_with_another_vendor_nearer() {
    let mut c = Client::offline(crate::testkit::no_data());
    let vendor = |guid: u32, name: &str, x: f32| {
        let mut o = crate::testkit::creature(guid, name);
        o.object_desc_flags = ac_world::object_desc_flags::VENDOR;
        o.position = Some(ac_world::Position {
            cell: 0xA9B2_0001,
            local: glam::Vec3::new(x, 90.0, 94.0),
            rotation: glam::Quat::IDENTITY,
        });
        o
    };
    // Boddry's listed spot, with the Pawn Shopkeep standing nearer to it than Boddry does.
    c.world.objects.insert(1, vendor(1, "Pawn Shopkeep", 91.0));
    c.world
        .objects
        .insert(2, vendor(2, "Boddry the Chancy", 96.0));
    let at = ac_world::landblock_origin(0xA9B2_0001).truncate() + glam::Vec2::new(90.6, 90.0);
    assert_eq!(c.vendor_object("Boddry the Chancy", at), Some(2));
    // With nobody by that name about, the nearest vendor still answers.
    assert_eq!(c.vendor_object("Gone Missing", at), Some(1));
}
