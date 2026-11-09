use super::*;

/// A Nullify All Magic Other plate of the data: the trap that strips every buff.
fn nullifier() -> &'static Trap {
    ac_world::traps::all()
        .iter()
        .find(|t| t.spells.contains("Nullify All Magic Other") && t.scale == 1.0)
        .expect("a nullifier in the data")
}

/// A straight walk along x through `t`'s middle, from 10 m before it to 10 m past.
fn through(t: &Trap) -> (Vec3, Route) {
    let at = |dx: f32| Place {
        at: t.at + Vec3::new(dx, 0.0, 0.0),
        cell: t.cell,
    };
    let walk = |p: Place| Waypoint {
        place: p,
        kind: EdgeKind::Walk,
        floor: None,
    };
    let route = Route {
        waypoints: vec![walk(at(-3.0)), walk(at(0.0)), walk(at(4.0)), walk(at(10.0))],
    };
    (at(-10.0).at, route)
}

#[test]
fn a_walk_over_a_buff_stripping_plate_is_flown_over_it() {
    let t = nullifier();
    let (c, r) = t.sphere();
    let (start, route) = through(t);
    let flown = over_traps(route, start, 0.48);
    let reach = r + 0.48 + TRAP_BERTH;
    let kinds: Vec<EdgeKind> = flown.waypoints.iter().map(|w| w.kind).collect();
    use EdgeKind::{Fly, Walk};
    assert_eq!(kinds, [Walk, Walk, Fly, Fly, Fly, Walk, Walk], "{flown:?}");
    // The plate's middle is flown past, not walked to.
    assert!(!flown.waypoints.iter().any(|w| w.place.at == t.at));
    let w = &flown.waypoints[1..];
    // Walked to the edge of its reach, risen there, crossed above its top, landed past it.
    assert!((w[0].place.at.truncate().distance(c.truncate()) - reach).abs() < 0.01);
    assert!(w[1].place.at.truncate().distance(w[0].place.at.truncate()) < 0.01);
    assert!(w[1].place.at.z >= t.top() + TRAP_BERTH - 1e-3);
    assert_eq!(w[1].place.at.z, w[2].place.at.z, "the crossing is level");
    assert!(w[3].place.at.truncate().distance(c.truncate()) >= reach - 0.01);
    assert!(
        (w[3].place.at.z - t.at.z).abs() < 1e-3,
        "landed on the floor"
    );
    let w = &flown.waypoints;
    // No walked leg comes within its reach.
    let mut from = start;
    for x in w {
        if x.kind != EdgeKind::Fly {
            // Its ends stand on the edge of the reach: a centimetre inside is the arithmetic.
            let near = within_circle(
                from.truncate(),
                x.place.at.truncate(),
                c.truncate(),
                reach - 0.01,
            );
            assert!(near.is_none(), "walked into its reach to {:?}", x.place.at);
        }
        from = x.place.at;
    }
}

#[test]
fn a_walk_nowhere_near_a_plate_is_left_as_it_was() {
    let t = nullifier();
    let (start, route) = through(t);
    let away = Route {
        waypoints: route
            .waypoints
            .iter()
            .map(|w| Waypoint {
                place: Place {
                    at: w.place.at + Vec3::new(0.0, 30.0, 0.0),
                    ..w.place
                },
                ..*w
            })
            .collect(),
    };
    let left = over_traps(away.clone(), start + Vec3::new(0.0, 30.0, 0.0), 0.48);
    assert_eq!(left, away);
    // On the floor below, a storey down: under it, not through it.
    let below = Route {
        waypoints: route
            .waypoints
            .iter()
            .map(|w| Waypoint {
                place: Place {
                    at: w.place.at - Vec3::new(0.0, 0.0, 6.0),
                    ..w.place
                },
                ..*w
            })
            .collect(),
    };
    let left = over_traps(below.clone(), start - Vec3::new(0.0, 0.0, 6.0), 0.48);
    assert_eq!(left, below);
}

#[test]
fn a_body_that_may_not_fly_walks_the_route_flown_for_another() {
    let t = nullifier();
    let (start, route) = through(t);
    let flown = over_traps(route, start, 0.48);
    assert!(flown.is_flown());
    let walked = flown.clone().walked();
    assert!(!walked.is_flown());
    assert_eq!(
        walked.waypoints.len(),
        flown.waypoints.len() - 2,
        "{walked:?}"
    );
    assert_eq!(walked.waypoints.last(), flown.waypoints.last());
}

#[test]
fn the_follower_rises_all_the_way_before_crossing_and_lands_before_walking_on() {
    use crate::steering::Course;
    let t = nullifier();
    let (start, route) = through(t);
    let flown = over_traps(route, start, 0.48);
    let w: Vec<Vec3> = flown.waypoints.iter().map(|w| w.place.at).collect();
    let from = Place {
        at: start,
        cell: t.cell,
    };
    let goal = *flown.waypoints.last().map(|w| &w.place).unwrap();
    let mut c = Course::new(from, goal, 0.5, flown, std::time::Instant::now());
    let walk = |_: Vec3, _: Vec3| true;
    // Walked to the edge of its reach, the next aim is straight up.
    assert_eq!(c.target(w[0], walk), w[1]);
    assert!(!c.is_flying(w[0]));
    assert_eq!(c.target(w[1], walk), w[2]);
    assert!(c.is_flying(w[1]));
    // Half way up is not up: no corner is cut through the plate's reach.
    let half = w[1].lerp(w[2], 0.5);
    assert_eq!(c.target(half, walk), w[2]);
    // Up, the crossing; across, the landing; landed, the walk on.
    assert_eq!(c.target(w[2], walk), w[3]);
    assert_eq!(c.target(w[3], walk), w[4]);
    assert!(c.is_flying(w[3]), "flying until it lands");
    assert_eq!(c.target(w[4], walk), w[5]);
    assert!(!c.is_flying(w[4]));
}
