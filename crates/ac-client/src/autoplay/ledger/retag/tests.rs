use super::*;
use crate::autoplay::Doing;
use crate::items::ItemStats;
use crate::testkit::{asks, character_of_level, game_data, item, judged, name_lists, shelf};

#[test]
fn a_re_judged_pack_counts_each_kind_as_it_goes() {
    // Three rings and two piles of tapers, in the order they were
    // come by. Each ring is told how many rings were held before
    // it -- none, one, two -- not that three are carried, so a rule
    // that keeps up to two claims the first two and not the third.
    // Told instead that it was the second of two, the second ring
    // sat over the cap and only one was kept.
    let mut carried = vec![
        (30, 500, 1),   // third ring
        (10, 500, 1),   // first ring
        (25, 691, 300), // second pile of tapers
        (20, 500, 1),   // second ring
        (15, 691, 120), // first pile of tapers
    ];
    assert_eq!(
        in_arrival_order(&mut carried),
        vec![(10, 0), (15, 0), (20, 1), (25, 120), (30, 2)]
    );
    // A stack counts for what it holds, not for one.
    let mut two = vec![(7, 691, 4059), (8, 691, 1)];
    assert_eq!(in_arrival_order(&mut two), vec![(7, 0), (8, 4059)]);
    assert!(in_arrival_order(&mut []).is_empty());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn what_arrives_is_judged_against_what_was_held_before_it() {
    // "Keep up to four healing kits", three in the pack, a fourth
    // bought. The arrival pass once counted the arrival itself, so
    // the fourth was the fourth of four, over the cap, and "the
    // rest, to the counter" tagged it to sell: the kit just bought
    // for the line went back over the counter. Held is what was
    // held before it, on this path as on the corpse's, and a fifth
    // is the one over the cap.
    let mut c = character_of_level(game_data(), 20);
    let dir = std::env::temp_dir().join("acswarm-test-arrival-profiles");
    std::fs::create_dir_all(&dir).ok();
    let shelf = std::sync::Arc::new(crate::profile::Library::default());
    shelf.open(&dir);
    let mut kits = crate::profile::Rule {
        name: "kits".into(),
        action: LootAction::Keep,
        all: vec![crate::profile::Ask::Item(crate::items::Term::Word(
            "healing kit".into(),
        ))],
        ..Default::default()
    };
    kits.keep_up_to = Some(4);
    shelf
        .put(crate::profile::Profile {
            name: "four-kits".into(),
            rules: vec![kits, asks("the rest", "value>=0", LootAction::Sell)],
            ..Default::default()
        })
        .unwrap();
    c.profiles = shelf;
    c.autoplay.config.loot.profile = "four-kits".into();
    let me = c.world.player_guid.unwrap();
    let kit = |guid: u32| ac_world::WorldObject {
        guid,
        name: "Healing Kit".into(),
        weenie_class_id: 4000,
        item_type: ac_world::item_type::MISC,
        value: 100,
        stack_size: 1,
        container: Some(me),
        ..Default::default()
    };
    for guid in [0x8000_0001, 0x8000_0002, 0x8000_0003] {
        c.world.objects.insert(guid, kit(guid));
    }
    let t0 = Instant::now();
    // The first pass only notes what is carried.
    c.autoplay_tag_arrivals(t0);
    c.world.objects.insert(0x8000_0004, kit(0x8000_0004));
    c.autoplay_tag_arrivals(t0);
    assert_eq!(
        c.autoplay.ledger.by_guid(0x8000_0004),
        Some(LootAction::Keep),
        "the fourth of four is under the cap"
    );
    c.world.objects.insert(0x8000_0005, kit(0x8000_0005));
    c.autoplay_tag_arrivals(t0);
    assert_eq!(
        c.autoplay.ledger.by_guid(0x8000_0005),
        Some(LootAction::Sell),
        "the fifth is over it"
    );
}

#[test]
fn what_arrives_in_the_pack_is_judged_like_what_lies_on_a_corpse() {
    // A bundle of arrowheads is a bundle of arrowheads whether it
    // came off a drudge or over a counter.
    let library = shelf(
        "arrival",
        vec![
            asks("keepers", "value>250", LootAction::Keep),
            asks("plate", "type:armor al>=200", LootAction::Sell),
        ],
    );
    name_lists(&library, &[], &[]);
    let me = crate::weapons::Wielder::default();
    let tag =
        |s: &ItemStats| arrival_tag(s, None, library.get("test").as_deref(), &me, "Aldric", 0);
    assert_eq!(tag(&item("Ornate Ring", 900, 0)), Some(LootAction::Keep));
    assert_eq!(tag(&item("Platemail", 100, 240)), Some(LootAction::Sell));
    // Nothing claimed it, so there is nothing to write down...
    assert_eq!(tag(&item("Rusty Nail", 3, 0)), None);
    // ...and neither has a skip, which is a decision to leave it.
    name_lists(&library, &[], &["ornate"]);
    assert_eq!(tag(&item("Ornate Ring", 900, 0)), None);
    // An item that cannot be judged until it is appraised is not
    // written down either: the answer is not in yet.
    name_lists(&library, &[], &[]);
    let unread = ItemStats {
        appraised: false,
        ..item("Platemail", 100, 240)
    };
    assert!(matches!(
        judged(&unread, &library, "test"),
        crate::profile::Verdict::NeedsId(_)
    ));
    assert_eq!(tag(&unread), None);
    assert_eq!(LootAction::parse("Salvage"), Some(LootAction::Salvage));
    assert_eq!(LootAction::parse("burn"), None);
    assert!(!LootAction::Skip.takes());
    let _ = std::fs::remove_dir_all(library.dir());
}

#[test]
fn what_an_item_was_taken_for_is_written_down() {
    let mut ap = Autoplay::default();
    let ring = item("Ornate Ring", 900, 0);
    assert_eq!(ap.tags().get(&ring.guid), None);
    ap.tag(&ring, LootAction::Salvage);
    assert_eq!(ap.tags().get(&ring.guid), Some(&LootAction::Salvage));
    assert_eq!(Doing::Salvaging.label(), "salvaging");
}
