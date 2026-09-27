use super::*;
use crate::autoplay::LootAction;
use crate::testkit::{a_loot_profile, character_of_level, game_data};

#[test]
fn the_best_salvager_has_an_ust_and_the_highest_skill() {
    let mate = |name: &str, guid: u32, salvaging: u32, has_ust: bool| Mate {
        name: name.into(),
        guid,
        salvaging,
        has_ust,
        ..Default::default()
    };
    let team = [
        mate("Zed", 1, 300, true),
        mate("Amy", 2, 300, true),
        mate("Bob", 3, 400, false),
        mate("Cal", 4, 200, true),
    ];
    // Bob's skill is highest but he has no Ust; Amy and Zed tie and
    // the name that sorts first wins.
    assert_eq!(best_salvager(team.iter()), Some(("Amy".into(), 2)));
    assert_eq!(best_salvager(team[3..].iter()), Some(("Cal".into(), 4)));
    assert_eq!(best_salvager(team[2..3].iter()), None);
    assert_eq!(best_salvager(std::iter::empty()), None);
    // Someone not yet in the world (guid 0) cannot be handed anything.
    assert_eq!(best_salvager([mate("Nobody", 0, 999, true)].iter()), None);
}

const STEEL: u32 = 0x40;
const IRON: u32 = 0x3D;

/// An item of `material` and `workmanship` tagged by a rule combining `bands`, refused `refused` times.
fn waiting(guid: u32, material: u32, workmanship: f32, bands: &str, refused: u8) -> Waiting {
    Waiting {
        guid,
        material,
        workmanship,
        bands: ac_loot::bands::parse(bands),
        refused,
    }
}

/// A salvage bag of `material` holding `units` of 100, of average `workmanship`.
fn partial(guid: u32, material: u32, workmanship: f32, units: u32) -> Partial {
    Partial {
        guid,
        material,
        workmanship,
        units,
        holds: 100,
    }
}

/// Every call sent for `items`, the server taking each before the next is chosen.
fn salvages(items: &[Waiting]) -> Vec<Vec<u32>> {
    let mut left = items.to_vec();
    let mut sent = Vec::new();
    while let Some(batch) = next_salvage_batch(&left, &[]) {
        left.retain(|w| !batch.items.contains(&w.guid));
        sent.push(batch.items);
    }
    sent
}

#[test]
fn with_no_bands_one_material_goes_in_one_salvage() {
    // "If no ranges specified then 1-10": everything of a material together.
    let items = [
        waiting(1, STEEL, 1.0, "", 0),
        waiting(2, STEEL, 6.0, "", 0),
        waiting(3, STEEL, 9.0, "", 0),
        waiting(4, STEEL, 10.0, "", 0),
    ];
    assert_eq!(salvages(&items), vec![vec![1, 2, 3, 4]]);
}

#[test]
fn bands_keep_apart_what_they_name_the_best_first() {
    // "Only combine workmanship 1-7, 8, 9, 10."
    let b = "1-7, 8, 9, 10";
    let items = [
        waiting(1, STEEL, 9.0, b, 0),
        waiting(2, STEEL, 10.0, b, 0),
        waiting(3, STEEL, 6.0, b, 0),
        waiting(4, STEEL, 8.0, b, 0),
        waiting(5, STEEL, 10.0, b, 0),
        waiting(6, STEEL, 3.0, b, 0),
    ];
    assert_eq!(
        salvages(&items),
        vec![vec![2, 5], vec![1], vec![4], vec![3, 6]]
    );
    let batch = next_salvage_batch(&items, &[]).unwrap();
    assert_eq!((batch.material, batch.band), (STEEL, (10, 10)));
}

#[test]
fn two_materials_never_share_a_salvage() {
    let items = [waiting(1, STEEL, 6.0, "", 0), waiting(2, IRON, 6.0, "", 0)];
    assert_eq!(salvages(&items).len(), 2);
}

#[test]
fn a_salvage_that_came_to_nothing_waits_behind_those_not_yet_tried() {
    // ACE skips a Retained item without a word. Chosen as the best band every time, a 10 like
    // that went out alone after each timeout, and everything below it waited behind all three.
    let b = "1-8, 9, 10";
    let (ten, nine, six) = (1, 2, 3);
    let items = [
        waiting(ten, IRON, 10.0, b, 1),
        waiting(nine, IRON, 9.0, b, 0),
        waiting(six, IRON, 6.0, b, 0),
    ];
    assert_eq!(next_salvage_batch(&items, &[]).unwrap().items, vec![nine]);
    assert_eq!(
        next_salvage_batch(&items[..1], &[]).unwrap().items,
        vec![ten]
    );
}

#[test]
fn a_partial_bag_of_the_band_is_topped_up_first() {
    let b = "1-7, 8, 9, 10";
    let items = [waiting(1, STEEL, 5.0, b, 0), waiting(2, STEEL, 7.0, b, 0)];
    let bags = [
        partial(10, STEEL, 6.4, 40),
        partial(11, STEEL, 9.0, 30),  // another band
        partial(12, IRON, 6.0, 20),   // another material
        partial(13, STEEL, 6.0, 100), // full
    ];
    let batch = next_salvage_batch(&items, &bags).unwrap();
    assert_eq!(batch.bags, vec![10]);
    assert_eq!(batch.units, 40);
    assert_eq!(
        batch.guids(),
        vec![10, 1, 2],
        "the bag first, or its excess is lost"
    );
}

#[test]
fn bags_topped_up_together_fit_in_one_bag() {
    // A bag put in past a bag's worth loses the excess (Player_Crafting.cs:283-289).
    let items = [waiting(1, STEEL, 5.0, "", 0)];
    let bags = [
        partial(10, STEEL, 5.0, 50),
        partial(11, STEEL, 5.0, 60),
        partial(12, STEEL, 5.0, 30),
    ];
    let batch = next_salvage_batch(&items, &bags).unwrap();
    assert_eq!(batch.bags, vec![11, 12], "the fullest that fit: 60 and 30");
    assert_eq!(batch.units, 90);
}

#[test]
fn nothing_tagged_sends_no_salvage() {
    assert_eq!(
        next_salvage_batch(&[], &[partial(10, STEEL, 5.0, 50)]),
        None
    );
}

/// A level 20 character that salvages for itself: an Ust in the pack,
/// and a loot profile of its own called `profile`.
fn a_salvager(assets: std::rc::Rc<ac_scene::Assets>, profile: &str) -> Client {
    let mut c = character_of_level(assets, 20);
    a_loot_profile(&mut c, profile);
    // The maces' rule: 1-8 together, 9 and 10 each alone.
    let mut p = (*c.loot_profile().unwrap()).clone();
    p.rules.insert(
        0,
        crate::profile::Rule {
            name: MACES.into(),
            action: LootAction::Salvage,
            combine: "1-8, 9, 10".into(),
            ..Default::default()
        },
    );
    c.profiles.put(p).expect("saved");
    let ust = 0x8000_0100;
    c.world.objects.insert(
        ust,
        ac_world::WorldObject {
            guid: ust,
            weenie_class_id: ac_world::material::UST_WCID,
            name: "Ust".into(),
            container: c.world.player_guid,
            ..Default::default()
        },
    );
    c
}

/// The salvage rule the test maces were tagged by.
const MACES: &str = "salvage maces";

/// An Iron mace of `workmanship` in `container`, appraised (not inscribed) and tagged by [`MACES`].
fn a_mace_to_salvage(c: &mut Client, guid: u32, workmanship: f32, container: Option<u32>) {
    c.world.objects.insert(
        guid,
        ac_world::WorldObject {
            guid,
            name: "Iron Mace".into(),
            material: IRON,
            workmanship,
            container,
            ..Default::default()
        },
    );
    c.appraisals.insert(
        guid,
        ac_net::messages::Appraisal {
            guid,
            success: true,
            ..Default::default()
        },
    );
    let stats = c.stats_of(guid).expect("carried");
    c.autoplay.tag_why(&stats, LootAction::Salvage, Some(MACES));
}

/// The salvage the salvager is waiting on.
fn salvage_on_its_way(c: &Client) -> Option<Vec<u32>> {
    c.autoplay.salvaging.as_ref().map(|(g, _)| g.clone())
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn what_the_team_hands_the_salvager_is_salvaged_a_band_at_a_time() {
    // Teammates hand salvage over one item at a time, and the
    // salvager salvages it along with its own: that is where a 10
    // one teammate carried would meet another's 6.
    let mut c = a_salvager(game_data(), "salvage test");
    let me = c.world.player_guid;
    // The 9 is in a side pack: the server finds it there, and so
    // must the salvage.
    let pack = 0x8000_0110;
    c.world.objects.insert(
        pack,
        ac_world::WorldObject {
            guid: pack,
            name: "Pack".into(),
            container: me,
            ..Default::default()
        },
    );
    let (ten, six, nine) = (0x8000_0101, 0x8000_0102, 0x8000_0103);
    for (guid, workmanship, container) in [(ten, 10.0, me), (six, 6.0, me), (nine, 9.0, Some(pack))]
    {
        a_mace_to_salvage(&mut c, guid, workmanship, container);
    }
    let mut now = Instant::now();
    assert!(c.autoplay_salvage(now), "salvaging");
    assert_eq!(salvage_on_its_way(&c), Some(vec![ten]));
    for (done, next) in [(ten, nine), (nine, six)] {
        now += Duration::from_millis(100);
        assert!(c.autoplay_salvage(now), "not waited on");
        assert_eq!(salvage_on_its_way(&c), Some(vec![done]));
        // The server takes it, and the next grade goes at once. Sitting
        // out the gap gave the tick to the fight, and the grades still
        // to come waited a whole fight for their turn.
        c.world.objects.remove(&done);
        now += Duration::from_millis(100);
        assert!(c.autoplay_salvage(now), "the next grade waited");
        assert_eq!(salvage_on_its_way(&c), Some(vec![next]));
    }
    c.world.objects.remove(&six);
    now += Duration::from_millis(100);
    assert!(!c.autoplay_salvage(now), "nothing left to salvage");
    assert_eq!(salvage_on_its_way(&c), None);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_ten_the_server_skips_holds_up_none_of_the_bands_below_it() {
    // ACE skips a Retained item without a word, and its salvage times
    // out. Chosen again as the best grade, the 10 went out alone after
    // every timeout, and the 9 and the 6 waited behind all three.
    let mut c = a_salvager(game_data(), "salvage skipped");
    let me = c.world.player_guid;
    let (ten, nine, six) = (0x8000_0121, 0x8000_0122, 0x8000_0123);
    for (guid, workmanship) in [(ten, 10.0), (nine, 9.0), (six, 6.0)] {
        a_mace_to_salvage(&mut c, guid, workmanship, me);
    }
    let mut now = Instant::now();
    assert!(c.autoplay_salvage(now));
    assert_eq!(salvage_on_its_way(&c), Some(vec![ten]));
    // Nothing comes of it.
    now += SALVAGE_TIMEOUT;
    assert!(c.autoplay_salvage(now));
    assert_eq!(
        salvage_on_its_way(&c),
        Some(vec![nine]),
        "the 10 went again before the grades not yet tried"
    );
    for (done, next) in [(nine, six), (six, ten)] {
        c.world.objects.remove(&done);
        now += Duration::from_millis(100);
        assert!(c.autoplay_salvage(now));
        assert_eq!(salvage_on_its_way(&c), Some(vec![next]));
    }
    // Still on its own, until the third try sets it aside.
    now += SALVAGE_TIMEOUT;
    assert!(c.autoplay_salvage(now));
    assert_eq!(salvage_on_its_way(&c), Some(vec![ten]));
    now += SALVAGE_TIMEOUT;
    assert!(!c.autoplay_salvage(now), "tried {SALVAGE_TRIES} times");
    assert_eq!(salvage_on_its_way(&c), None);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn neither_an_inscribed_item_nor_one_not_yet_looked_at_is_salvaged() {
    // "Never salvage any item that the player is wearing or is inscribed": only an appraisal says.
    let mut c = a_salvager(game_data(), "salvage inscribed");
    let me = c.world.player_guid;
    let (plain, inscribed, unknown) = (0x8000_0131, 0x8000_0132, 0x8000_0133);
    for g in [plain, inscribed, unknown] {
        a_mace_to_salvage(&mut c, g, 6.0, me);
    }
    c.appraisals.get_mut(&inscribed).unwrap().strings = vec![(7, "For my dear Bryn".into())];
    c.appraisals.remove(&unknown);
    assert!(c.autoplay_salvage(Instant::now()));
    assert_eq!(salvage_on_its_way(&c), Some(vec![plain]));
    assert!(
        c.appraise_queue.contains(&unknown),
        "asked about before it goes"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn what_is_worn_is_never_salvaged() {
    let mut c = a_salvager(game_data(), "salvage worn");
    let me = c.world.player_guid;
    let worn = 0x8000_0141;
    a_mace_to_salvage(&mut c, worn, 6.0, me);
    let o = c.world.objects.get_mut(&worn).unwrap();
    o.container = None;
    o.wielder = me;
    assert!(!c.autoplay_salvage(Instant::now()), "nothing to salvage");
    assert!(!c.salvage(&[worn]), "nor by hand");
}
