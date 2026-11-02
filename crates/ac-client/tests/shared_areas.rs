//! The region graphs every session plans on: built once by the builder thread, shared, and
//! planned on without waiting for it. Each test walks the same dungeon with a capsule of its own,
//! so their areas are their own. Needs AC_DATA_DIR.

use std::time::{Duration, Instant};

use ac_client::pathfinder::Pathfinder;
use ac_nav::routing::Planned;
use ac_scene::collision::Capsule;
use ac_scene::Assets;
use ac_world::Place;
use glam::Vec3;

/// Long enough for the builder to assemble the dungeon and build the walk's chunks (debug).
const BUILT_WITHIN: Duration = Duration::from_secs(60);

/// The route harness's "0286 corpse on the floor below": down a storey in a small dungeon.
fn walk() -> (Place, Place, f32) {
    let from = Place {
        at: Vec3::new(419.98, 25_673.98, 0.0),
        cell: 0x0286_02BF,
    };
    let goal = Place {
        at: Vec3::new(386.725, 25_627.996, -6.0),
        cell: 0x0286_0000,
    };
    (from, goal, 1.5)
}

/// The default body, a little taller by `taller` (metres): a shape no other test plans for.
fn capsule(taller: f32) -> Capsule {
    let d = Capsule::default();
    Capsule {
        height: d.height + taller,
        ..d
    }
}

fn assets() -> Assets {
    Assets::open(ac_dat::test_data_dir()).unwrap()
}

/// Plan until the answer is not `Building`, letting the builder work between asks.
fn planned(p: &mut Pathfinder, cap: Capsule) -> Planned {
    let (from, goal, within) = walk();
    let until = Instant::now() + BUILT_WITHIN;
    loop {
        let answer = p.plan(from, goal, within, cap);
        if !matches!(answer, Planned::Building(_)) || Instant::now() > until {
            return answer;
        }
        p.settle(Duration::from_secs(1));
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_plan_on_an_unbuilt_region_answers_building_then_a_route() {
    let assets = assets();
    let cap = capsule(0.01);
    let mut p = Pathfinder::new(&assets);
    let (from, goal, within) = walk();
    let first = p.plan(from, goal, within, cap);
    assert!(
        matches!(first, Planned::Building(_)),
        "nothing is built yet, so no answer: {first:?}"
    );
    match planned(&mut p, cap) {
        Planned::Route(r) => {
            let end = r.waypoints.last().unwrap().place.at;
            assert!(end.distance(goal.at) <= within + 0.5, "ends at {end}");
        }
        other => panic!("built, the walk has a route: {other:?}"),
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn two_sessions_in_one_area_share_one_region_built_once() {
    let assets = assets();
    let cap = capsule(0.02);
    let mut first = Pathfinder::new(&assets);
    let route = match planned(&mut first, cap) {
        Planned::Route(r) => r,
        other => panic!("built, the walk has a route: {other:?}"),
    };
    // Another session's handle, never having asked: the region is there and built for it.
    let mut second = Pathfinder::new(&assets);
    let (from, goal, within) = walk();
    match second.plan(from, goal, within, cap) {
        Planned::Route(r) => assert_eq!(r, route, "the same graph gives the same route"),
        other => panic!("the second session plans on the first's region: {other:?}"),
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_plan_never_waits_on_the_builder() {
    let assets = assets();
    let cap = capsule(0.03);
    let mut p = Pathfinder::new(&assets);
    let (from, goal, within) = walk();
    // A goal outside the dungeon: the area is looked up or queued and no search starts, so what
    // is timed is only what the frame waits for while the builder holds the area.
    let away = Place {
        at: goal.at,
        cell: 0x0287_0000,
    };
    let _ = p.plan(from, goal, within, cap);
    let until = Instant::now() + BUILT_WITHIN;
    let (mut slowest, mut calls) = (Duration::ZERO, 0);
    let mut other = Pathfinder::new(&assets);
    while Instant::now() < until {
        let started = Instant::now();
        let answer = other.plan(from, away, within, cap);
        slowest = slowest.max(started.elapsed());
        calls += 1;
        assert!(!matches!(answer, Planned::Route(_)), "{answer:?}");
        if !p.busy() && matches!(p.plan(from, goal, within, cap), Planned::Route(_)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(calls > 1, "the builder finished before it could be timed");
    assert!(
        slowest < Duration::from_millis(5),
        "a plan waited {slowest:?} while the builder built"
    );
}
