//! Pathfinding through the Training Academy (landblock 8602), on a one-block area. Needs
//! AC_DATA_DIR.

use ac_scene::{
    collision::Capsule,
    landblock, lbid,
    nav::line_clear,
    navarea::Area,
    wayfind::{self, Search, WalkQuery},
    Assets,
};
use glam::Vec3;

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn academy_start_room_to_the_far_end() {
    let dir = ac_dat::test_data_dir();
    let assets = Assets::open(dir).unwrap();
    let block = 0x8602_0000;
    let scene = landblock::load(&assets, block).unwrap();
    let cap = Capsule::default();
    let origin = lbid::world_origin(block);
    let start = origin + Vec3::new(12.3, -28.5, 0.0);

    // The whole block, for the numbers.
    let mut whole = Area::build(&assets, &[block], &cap, block).unwrap();
    whole.build_everything();
    eprintln!(
        "academy nav: {} nodes, {} edges, {} chunks, built in {:.0} ms",
        whole.nav.len(),
        whole.nav.edge_count(),
        whole.nav.chunk_count(),
        whole.nav.build_time.as_secs_f64() * 1e3
    );
    assert!(whole.nav.len() > 5000, "{} nodes", whole.nav.len());

    for target in [0x8602_0215, 0x8602_026D] {
        assert!(
            scene.cells.iter().any(|c| c.cell_id == target),
            "cell {target:#010x} missing"
        );
        let goal = whole
            .nav
            .nodes
            .iter()
            .find(|n| n.cell == target)
            .unwrap_or_else(|| panic!("no walkable node in {target:#010x}"))
            .pos;
        assert!(
            !line_clear(&whole.collision, start, goal),
            "the straight line to {target:#010x} is clear"
        );
        // A fresh area builds only the chunks the search reaches.
        let mut area = Area::build(&assets, &[block], &cap, block).unwrap();
        let t = std::time::Instant::now();
        area.with_ground(|ground, graph| {
            let q = WalkQuery {
                from: (start, wayfind::locate(ground, start)),
                goal: (goal, target),
                within: 0.0,
                avoid: &[],
                berth: 0.0,
                indoors_too: true,
                drops: false,
            };
            let path: Vec<Vec3> = match wayfind::search_building(ground, graph, &q) {
                Search::Found(p) => p.into_iter().map(|(n, _)| n.pos).collect(),
                other => panic!("no path to {target:#010x}: {other:?}"),
            };
            eprintln!(
                "to {target:#010x}: {} waypoints in {:.1} ms; built {} nodes in {} chunks ({:.0} ms)",
                path.len(),
                t.elapsed().as_secs_f64() * 1e3,
                graph.len(),
                graph.chunk_count(),
                graph.build_time.as_secs_f64() * 1e3
            );
            assert_eq!(*path.last().unwrap(), goal);
            assert!(path.len() >= 2, "{path:?}");
            // The spawn point sits inside a thin marker post; the planner
            // steps out of it first, as the player does on its first move.
            let collision = ground.collision;
            let mut from = collision.resolve_above(start, cap.radius, cap.height, cap.step_up);
            assert!(from.distance(start) < 0.5, "start pushed to {from}");
            for &p in &path {
                assert!(
                    line_clear(collision, from, p),
                    "{from} -> {p} crosses a wall"
                );
                // By the body's own step, as every edge is (`Ground::hop`).
                let cell = |q: Vec3| ground.surface_at(q, &cap).map_or(0, |f| f.1);
                assert!(
                    ground.body_reaches(from, cell(from), p, &cap),
                    "{from} -> {p} cannot be walked"
                );
                assert!(from.distance(p) > 0.1, "degenerate step at {p}");
                from = p;
            }
            // Every waypoint stands on an Academy floor.
            for &p in &path {
                let (_, cell) = ground.surface_at(p, &cap).expect("floor under waypoint");
                assert_eq!(cell & 0xFFFF_0000, block, "{p} in {cell:#010x}");
            }
        });
    }
}
