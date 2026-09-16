use super::*;

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
