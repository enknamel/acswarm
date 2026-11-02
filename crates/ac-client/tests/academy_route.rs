//! The Training Academy tutorial's legs are walkable on the academy
//! landblock's own area (see `ac_client::academy`). Needs AC_DATA_DIR.

use ac_scene::{
    collision::Capsule,
    navarea::Area,
    wayfind::{self, Search, WalkQuery},
    Assets,
};
use glam::Vec3;

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn academy_legs_are_walkable() {
    let dir = ac_dat::test_data_dir();
    let assets = Assets::open(dir).unwrap();
    let block = ac_client::academy::LANDBLOCK;
    let cap = Capsule::default();
    let mut area = Area::build(&assets, &[block], &cap, block).unwrap();
    let origin = ac_world::landblock_origin(block);
    // Every walk within one leg of the tutorial (a leg ends at a
    // portal, which the graph cannot follow) must find a path from the
    // previous spot.
    let mut from: Option<Vec3> = None;
    let mut failures = Vec::new();
    for step in ac_client::academy::steps() {
        let Some(local) = step.at else {
            continue;
        };
        let here = origin + local;
        // A ramp the graph cannot climb is walked straight up by the
        // physics; the table says so in the step's name.
        let ramp = step.kind == ac_client::academy::Kind::Walk && step.target.contains("ramp top");
        if let Some(prev) = from.filter(|_| !ramp) {
            let t = std::time::Instant::now();
            let found = area.with_ground(|ground, graph| {
                let q = WalkQuery {
                    from: (prev, wayfind::locate(ground, prev)),
                    goal: (here, wayfind::locate(ground, here)),
                    within: 0.5,
                    avoid: &[],
                    berth: 0.0,
                    indoors_too: true,
                    outdoors_first: false,
                    drops: false,
                };
                wayfind::search_building(ground, graph, &q)
            });
            match found {
                Search::Found(path) => eprintln!(
                    "{:?} {}: {} waypoints in {:.0} ms",
                    step.kind,
                    step.target,
                    path.len(),
                    t.elapsed().as_secs_f64() * 1e3
                ),
                found => failures.push(format!(
                    "{:?} {}: no path from {:?} to {:?}: {found:?}",
                    step.kind,
                    step.target,
                    prev - origin,
                    local
                )),
            }
        }
        // A portal ends the leg where it drops the character.
        from = match step.kind {
            ac_client::academy::Kind::Portal => step.lands.map(|l| origin + l),
            _ => Some(here),
        };
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
