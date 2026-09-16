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

/// `items` (guid and workmanship), none of them refused yet.
fn never_refused(items: &[(u32, f32)]) -> Vec<(u32, f32, u8)> {
    items.iter().map(|(g, w)| (*g, *w, 0)).collect()
}

/// Every salvage sent for `items` (guid and workmanship), the server
/// taking each batch before the next is chosen.
fn salvages(items: &[(u32, f32)]) -> Vec<Vec<u32>> {
    let mut left = items.to_vec();
    let mut sent = Vec::new();
    while let Some((_, batch)) = next_salvage_batch(never_refused(&left)) {
        left.retain(|(g, _)| !batch.contains(g));
        sent.push(batch);
    }
    sent
}

#[test]
fn a_salvage_that_came_to_nothing_waits_behind_the_grades_not_yet_tried() {
    // ACE skips a Retained item without a word. Chosen as the best
    // grade every time, a 10 like that went out alone after each
    // timeout, and everything below it waited behind all three.
    let (ten, nine, six, five) = (1, 2, 3, 4);
    assert_eq!(
        next_salvage_batch([(ten, 10.0, 1), (nine, 9.0, 0), (six, 6.0, 0)]),
        Some((SalvageGrade::Nine, vec![nine]))
    );
    assert_eq!(
        next_salvage_batch([(ten, 10.0, 1), (six, 6.0, 0)]),
        Some((SalvageGrade::Common, vec![six]))
    );
    // Once the rest are gone it is asked for again, still on its own.
    assert_eq!(
        next_salvage_batch([(ten, 10.0, 1)]),
        Some((SalvageGrade::Ten, vec![ten]))
    );
    // Refused alike, the grades still keep apart.
    assert_eq!(
        next_salvage_batch([(six, 6.0, 1), (ten, 10.0, 1), (five, 5.0, 1)]),
        Some((SalvageGrade::Ten, vec![ten]))
    );
    // And the one refused least goes first.
    assert_eq!(
        next_salvage_batch([(six, 6.0, 2), (five, 5.0, 1)]),
        Some((SalvageGrade::Common, vec![five]))
    );
}

#[test]
fn a_workmanship_10_iron_mace_is_not_salvaged_with_a_6() {
    // In one salvage both go into the same bag of Iron, and the bag
    // comes out a workmanship 8.
    let (six, ten) = (0x8000_0001, 0x8000_0002);
    assert_eq!(
        salvages(&[(six, 6.0), (ten, 10.0)]),
        vec![vec![ten], vec![six]]
    );
}

#[test]
fn nines_and_tens_never_share_a_salvage() {
    let items = [(1, 9.0), (2, 10.0), (3, 6.0), (4, 9.0), (5, 10.0), (6, 3.0)];
    assert_eq!(
        next_salvage_batch(never_refused(&items)),
        Some((SalvageGrade::Ten, vec![2, 5]))
    );
    // The best first, each grade alone, in the order they were given.
    assert_eq!(salvages(&items), vec![vec![2, 5], vec![1, 4], vec![3, 6]]);
}

#[test]
fn everything_below_nine_goes_in_one_salvage() {
    let items = [(1, 1.0), (2, 8.0), (3, 5.0), (4, 8.0)];
    assert_eq!(
        next_salvage_batch(never_refused(&items)),
        Some((SalvageGrade::Common, vec![1, 2, 3, 4]))
    );
    assert_eq!(salvages(&items), vec![vec![1, 2, 3, 4]]);
}

#[test]
fn a_grade_with_nothing_in_it_sends_no_salvage() {
    assert_eq!(next_salvage_batch([]), None);
    assert_eq!(salvages(&[]), Vec::<Vec<u32>>::new());
    // No 9s: the 10 and the rest, and no empty salvage between.
    assert_eq!(salvages(&[(1, 6.0), (2, 10.0)]), vec![vec![2], vec![1]]);
    // Only 9s: one salvage.
    assert_eq!(salvages(&[(1, 9.0), (2, 9.0)]), vec![vec![1, 2]]);
}

/// A level 20 character that salvages for itself: an Ust in the pack,
/// and a loot profile of its own called `profile`.
fn a_salvager(assets: std::rc::Rc<ac_scene::Assets>, profile: &str) -> Client {
    let mut c = character_of_level(assets, 20);
    a_loot_profile(&mut c, profile);
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

/// An Iron mace of `workmanship` in `container`, tagged for salvage.
fn a_mace_to_salvage(c: &mut Client, guid: u32, workmanship: f32, container: Option<u32>) {
    c.world.objects.insert(
        guid,
        ac_world::WorldObject {
            guid,
            name: "Iron Mace".into(),
            material: 0x3D,
            workmanship,
            container,
            ..Default::default()
        },
    );
    let stats = c.stats_of(guid).expect("carried");
    c.autoplay.tag(&stats, LootAction::Salvage);
}

/// The salvage the salvager is waiting on.
fn salvage_on_its_way(c: &Client) -> Option<Vec<u32>> {
    c.autoplay.salvaging.as_ref().map(|(g, _)| g.clone())
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn what_the_team_hands_the_salvager_is_salvaged_a_grade_at_a_time() {
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
fn a_ten_the_server_skips_holds_up_none_of_the_grades_below_it() {
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
