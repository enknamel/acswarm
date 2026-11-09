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

/// A quest wanting ten Piles of Gearknight Parts (timed, so ten and no more), the only quest on.
const PARTS_QUEST: &str = "gearknightpartscollectionwait_0513";
const PARTS: u32 = 51266;

/// A level-150 character (the parts' easiest drop is 135) carrying `piles` Piles of Gearknight
/// Parts, its profile on its own shelf the quest rule above "the rest, sold", [`PARTS_QUEST`] on.
fn carrying_parts(piles: u32, shelf_name: &str) -> Client {
    let mut c = character_of_level(crate::testkit::no_data(), 150);
    c.world.stats.name = "Bryn".into();
    let library = shelf(
        shelf_name,
        vec![
            crate::profile::quest_rule(),
            asks("the rest", "value>=0", LootAction::Sell),
        ],
    );
    c.profiles = std::sync::Arc::new(library);
    c.autoplay.config.loot.profile = "test".into();
    c.autoplay.config.quests.on = true;
    c.autoplay.config.quests.off = ac_world::quests::all()
        .iter()
        .map(|r| r.id.to_string())
        .filter(|id| id != PARTS_QUEST)
        .collect();
    c.autoplay.config.quests.off.dedup();
    c.load_saved();
    let me = c.world.player_guid.unwrap();
    for i in 0..piles {
        let guid = 0x8000_0100 + i;
        let pile = ac_world::WorldObject {
            guid,
            name: "Pile of Gearknight Parts".into(),
            weenie_class_id: PARTS,
            item_type: ac_world::item_type::MISC,
            stack_size: 1,
            container: Some(me),
            ..Default::default()
        };
        c.world.objects.insert(guid, pile);
    }
    c
}

/// Each carried pile's tag and the rule that wrote it, oldest first.
fn tags_of(c: &Client) -> Vec<(Option<LootAction>, Option<String>)> {
    let mut guids: Vec<u32> = c.world.inventory().map(|o| o.guid).collect();
    guids.sort_unstable();
    guids
        .into_iter()
        .map(|g| {
            let stats = c.stats_of(g).unwrap();
            (
                c.autoplay.ledger.of(&stats),
                c.autoplay.ledger.why(g).map(str::to_string),
            )
        })
        .collect()
}

#[test]
fn the_quest_rule_keeps_what_the_quest_needs_and_the_rest_goes_to_the_rules_below() {
    let mut c = carrying_parts(12, "quest-retag");
    let t0 = Instant::now();
    c.tick_retag(t0);
    assert!(
        tags_of(&c).iter().all(|(a, _)| a.is_none()),
        "no board yet, nothing judged"
    );
    c.refresh_quests(t0);
    c.tick_retag(t0);
    assert_eq!(
        c.quest_wants,
        [ac_loot::QuestWant {
            wcid: PARTS,
            need: Some(10)
        }]
    );
    let keep = (Some(LootAction::Keep), Some("quest items".to_string()));
    let sell = (Some(LootAction::Sell), Some("the rest".to_string()));
    let tags = tags_of(&c);
    assert_eq!(
        tags[..10],
        vec![keep; 10][..],
        "the first ten, in arrival order"
    );
    assert_eq!(
        tags[10..],
        vec![sell.clone(); 2][..],
        "enough carried: the rest are sold"
    );
    assert_eq!(c.wielder().quest_wants, c.quest_wants);

    // Switched off, the quest wants nothing, and its items fall to the rules below.
    c.autoplay.config.quests.off.push(PARTS_QUEST.into());
    let t1 = t0 + crate::quest_log::QUEST_BOARD_EVERY;
    c.refresh_quests(t1);
    c.tick_retag(t1);
    assert!(c.quest_wants.is_empty());
    assert_eq!(tags_of(&c), vec![sell; 12]);
    let _ = std::fs::remove_dir_all(c.profiles.dir());
}

#[test]
fn a_pack_is_not_judged_on_a_board_worked_out_before_the_character_was_known() {
    let mut c = carrying_parts(12, "quest-retag-lobby");
    let (name, level) = (std::mem::take(&mut c.world.stats.name), c.world.stats.level);
    c.world.stats.level = 0;
    c.ledger_loaded = false;
    let t0 = Instant::now();
    // In the lobby: a board for nobody.
    c.tick_retag(t0);
    c.refresh_quests(t0);
    // The name and level come; the saved log is read on the next tick.
    (c.world.stats.name, c.world.stats.level) = (name, level);
    c.tick_retag(t0);
    c.refresh_quests(t0);
    assert!(
        tags_of(&c).iter().all(|(a, _)| a.is_none()),
        "judged on the lobby's board"
    );
    c.load_saved();
    c.tick_retag(t0);
    c.refresh_quests(t0);
    c.tick_retag(t0);
    let keep = (Some(LootAction::Keep), Some("quest items".to_string()));
    assert_eq!(tags_of(&c)[..10], vec![keep; 10][..]);
    let _ = std::fs::remove_dir_all(c.profiles.dir());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn mana_junk_is_only_what_came_off_a_corpse() {
    // The Seasoned Explorer Baton, a quest reward no rule named, was tagged mana junk where it
    // was carried and drained into a stone: the stone destroys what it drains.
    let mut c = character_of_level(game_data(), 20);
    c.world.stats.name = "Bryn".into();
    c.profiles = std::sync::Arc::new(shelf("looted-junk", vec![]));
    c.autoplay.config.loot.profile = "test".into();
    c.autoplay.config.quests.on = false;
    let me = c.world.player_guid.unwrap();
    let t0 = Instant::now();
    c.autoplay_tag_arrivals(t0);
    let wand = |c: &mut Client, guid: u32| {
        c.world.objects.insert(
            guid,
            ac_world::WorldObject {
                guid,
                name: "Baton".into(),
                weenie_class_id: 5000,
                item_type: ac_world::item_type::CASTER,
                value: 500,
                stack_size: 1,
                container: Some(me),
                ..Default::default()
            },
        );
        c.appraisals.insert(
            guid,
            ac_net::messages::Appraisal {
                guid,
                success: true,
                ints: vec![(107, 480), (108, 600)],
                ..Default::default()
            },
        );
    };
    let (reward, loot) = (0x8000_0011, 0x8000_0012);
    wand(&mut c, reward);
    c.autoplay_tag_arrivals(t0);
    assert_eq!(
        c.autoplay.ledger.by_guid(reward),
        None,
        "a reward made junk"
    );
    wand(&mut c, loot);
    c.autoplay.off_corpses.push((loot, LootAction::Drain, t0));
    c.autoplay_tag_arrivals(t0);
    let stats = c.stats_of(loot).unwrap();
    assert_eq!(c.autoplay.ledger.of(&stats), Some(LootAction::Drain));
    assert!(
        c.autoplay.ledger.is_looted(&stats),
        "where it came from was lost"
    );

    // A tag written before the flag: the re-judge forgets the reward's, keeps the loot's.
    let stats = c.stats_of(reward).unwrap();
    c.autoplay
        .tag_why(&stats, LootAction::Drain, Some(crate::profile::MANA_JUNK));
    c.tick_retag(t0);
    c.tick_retag(t0 + TAG_TIMEOUT);
    assert_eq!(
        c.autoplay.ledger.by_guid(reward),
        None,
        "the reward still junk"
    );
    assert_eq!(c.autoplay.ledger.by_guid(loot), Some(LootAction::Drain));
    let _ = std::fs::remove_dir_all(c.profiles.dir());
}
