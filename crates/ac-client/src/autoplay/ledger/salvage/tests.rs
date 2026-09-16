use super::*;

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
