use super::*;

/// Seconds between points on a line, at one metre a second.
fn line(a: f32, b: f32) -> Option<f32> {
    Some((a - b).abs())
}

/// Every order of `calls`, by brute force, for checking the search.
fn orders(calls: &[usize]) -> Vec<Vec<usize>> {
    if calls.is_empty() {
        return vec![Vec::new()];
    }
    let mut out = Vec::new();
    for (k, &c) in calls.iter().enumerate() {
        let mut rest = calls.to_vec();
        rest.remove(k);
        for mut o in orders(&rest) {
            o.insert(0, c);
            out.push(o);
        }
    }
    out
}

#[test]
fn calls_on_a_line_are_made_in_the_order_they_lie() {
    let (order, secs) = tour(0.0, &[5.0, 1.0, 3.0], Some(10.0), line).unwrap();
    assert_eq!(order, [1, 2, 0]);
    assert_eq!(secs, 10.0);
    // With no ground to go back to, the run ends at its last call.
    let (order, secs) = tour(4.0, &[5.0, 1.0], None, line).unwrap();
    assert_eq!((order, secs), (vec![0, 1], 5.0));
}

#[test]
fn every_order_of_five_calls_is_tried_and_the_fastest_kept() {
    // A lopsided table: no rule of thumb finds its best order.
    let cost = |a: usize, b: usize| Some(((a * 7 + b * 13) % 11) as f32 + 1.0);
    let calls = [1, 2, 3, 4, 5];
    let (order, secs) = tour(0, &calls, Some(6), cost).unwrap();
    let price = |o: &[usize]| {
        let mut at = 0;
        let mut total = 0.0;
        for &i in o {
            total += cost(at, calls[i]).unwrap();
            at = calls[i];
        }
        total + cost(at, 6).unwrap()
    };
    let best = orders(&[0, 1, 2, 3, 4])
        .iter()
        .map(|o| price(o))
        .fold(f32::MAX, f32::min);
    assert_eq!(orders(&[0, 1, 2, 3, 4]).len(), 120);
    assert_eq!(secs, best);
    assert_eq!(price(&order), secs);
}

#[test]
fn a_call_no_journey_reaches_leaves_no_order_and_the_rest_still_go() {
    // Call 2 stands where nothing goes: no leg into it from anywhere.
    let cost = |a: f32, b: f32| (b != 99.0).then(|| (a - b).abs());
    let legs = Legs::plan(0.0, &[5.0, 1.0, 99.0], Some(10.0), cost);
    assert_eq!(legs.leg(Stop::Start, Stop::Call(2)), None);
    assert_eq!(legs.tour(&[0, 1, 2]), None);
    assert_eq!(legs.tour(&[0, 1]), Some((vec![1, 0], 10.0)));
    assert_eq!(legs.tour(&[]), Some((Vec::new(), 10.0)), "straight home");
}

#[test]
fn legs_are_planned_once_each_and_past_six_calls_the_nearest_goes_next() {
    let mut planned = 0;
    let calls: Vec<f32> = (1..=6).map(|i| i as f32 * 10.0).collect();
    let legs = Legs::plan(0.0, &calls, Some(0.0), |a, b| {
        planned += 1;
        line(a, b)
    });
    assert_eq!(
        planned,
        6 * 6 + 6 + 1,
        "start, call to call, home, start to home"
    );
    assert_eq!(legs.calls(), 6);
    let calls: Vec<f32> = [7.0, 3.0, 1.0, 6.0, 2.0, 5.0, 4.0].to_vec();
    let (order, secs) = tour(0.0, &calls, None, line).unwrap();
    assert_eq!(order, [2, 4, 1, 6, 5, 3, 0]);
    assert_eq!(secs, 7.0);
}
