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

#[test]
fn a_walk_with_no_way_on_foot_rises_crosses_above_both_ends_and_comes_in_level() {
    let from = Place {
        at: Vec3::new(10.0, 10.0, 180.0),
        cell: 0xF618_000C,
    };
    let goal = Place {
        at: Vec3::new(-370.0, 0.0, 195.0),
        cell: 0xF418_000C,
    };
    let r = Route::flown_across(from, goal);
    let w: Vec<_> = r.waypoints.iter().map(|w| (w.place.at, w.kind)).collect();
    let high = 195.0 + GAP_CLEARANCE;
    // Down short of the goal, and into it level: a portal's mouth is moved into, never sunk onto.
    let short = goal.at.truncate() - (goal.at - from.at).truncate().normalize() * LEVEL_IN;
    assert_eq!(
        w,
        [
            (Vec3::new(10.0, 10.0, high), EdgeKind::Fly),
            (short.extend(high), EdgeKind::Fly),
            (short.extend(195.0), EdgeKind::Fly),
            (goal.at, EdgeKind::Fly),
        ]
    );
    assert!(r.is_flown());
    // A hop that short is flown straight at the goal.
    let near = Place {
        at: goal.at + Vec3::new(3.0, 0.0, 0.0),
        ..goal
    };
    let r = Route::flown_across(near, goal);
    assert_eq!(r.points(), [goal.at]);
}

#[test]
fn a_flight_replanned_in_the_air_climbs_no_higher() {
    // Each replan of a dungeon flight rose 5 m over where the body was, and Blargerton climbed
    // to 5.7 km in 0x01A60325 over an hour, never crossing.
    let goal = Place {
        at: Vec3::new(221.6, 31794.9, 12.0),
        cell: 0x01A6_02B6,
    };
    let mut at = Vec3::new(247.2, 31792.8, 24.0);
    for _ in 0..10 {
        let r = Route::flown_across(
            Place {
                at,
                cell: 0x01A6_0325,
            },
            goal,
        );
        at = r.points()[0];
    }
    assert_eq!(at.z, 24.0, "climbed");
}

#[test]
fn an_acid_pool_is_never_stood_in_and_a_walk_across_it_is_flown() {
    // Blargerton died 18 times standing in Matron Hive East's acid, 0x614601C3 (53, -122, -18):
    // a pool 6.7 m across and 1.5 m deep (ACE Hotspot.cs:39-80), now a trap.
    let o = ac_world::landblock_origin(0x6146_01C3);
    let spot = Place {
        at: o + Vec3::new(53.0, -122.0, -18.0),
        cell: 0x6146_01C3,
    };
    assert!(is_on_trap(spot, 0.48), "stood in the acid");
    let over = Place {
        at: spot.at + Vec3::Z * 2.5,
        ..spot
    };
    assert!(!is_on_trap(over, 0.48), "a flight 2.5 m up touches it");
    let pool = ac_world::traps::in_landblock(0x6146_0000)
        .find(|t| t.spells.contains("Acid") && t.reach() > 6.0)
        .expect("an acid pool in the hive");
    let (start, route) = through(pool);
    let flown = over_traps(route, start, 0.48);
    let crossing: Vec<&Waypoint> = flown
        .waypoints
        .iter()
        .filter(|w| w.kind == EdgeKind::Fly)
        .collect();
    assert!(!crossing.is_empty(), "walked through the acid: {flown:?}");
    // Crossed just over its 1.5 m depth, not a sphere's 7 m.
    let high = crossing
        .iter()
        .map(|w| w.place.at.z - pool.at.z)
        .fold(0.0, f32::max);
    assert!((1.9..3.0).contains(&high), "{high}");
    // A hotspot that hurts nobody is stood in: the mana fields are no traps.
    assert!(ac_world::traps::all()
        .iter()
        .all(|t| !t.spells.contains("Mana Field")));
}
