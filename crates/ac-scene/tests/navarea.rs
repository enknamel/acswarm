//! Walking into Sanamar past its city walls, and climbing a dungeon's
//! stairs. Needs AC_DATA_DIR.
//!
//! Sanamar (landblock 33D9) is the case that prompted the neighbourhood
//! planner: a character walking in from outside the walls would run at
//! them and stop, because the gate is in the next landblock and a route
//! planned on one block at a time could not see it.

use ac_scene::{
    collision::Capsule,
    lbid,
    nav::{Ground, NavGraph},
    navarea::{blocks_for, Area},
    wayfind::{self, Search, WalkQuery},
    Assets,
};
use glam::Vec3;

/// The walk from `from` to within `within` of `to` on `nav`, both ends located by the cells.
fn route(
    ground: &Ground,
    nav: &mut NavGraph,
    from: Vec3,
    to: Vec3,
    within: f32,
) -> Option<Vec<Vec3>> {
    let q = WalkQuery {
        from: (from, wayfind::locate(ground, from)),
        goal: (to, wayfind::locate(ground, to)),
        within,
        avoid: &[],
        berth: 0.0,
        indoors_too: true,
        outdoors_first: false,
        drops: false,
    };
    match wayfind::search_building(ground, nav, &q) {
        Search::Found(p) => Some(p.into_iter().map(|(n, _)| n.pos).collect()),
        _ => None,
    }
}

fn capsule() -> Capsule {
    Capsule {
        radius: 0.5,
        height: 1.8,
        step_up: 0.8,
        step_down: 1.5,
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_walk_into_sanamar_goes_round_the_walls_not_into_them() {
    let dir = ac_dat::test_data_dir();
    let assets = Assets::open(dir).unwrap();
    let cap = capsule();
    let block = lbid::from_xy(0x33, 0xD9);
    let origin = lbid::world_origin(block);
    // The town square, where the portal from the town network lands.
    let square = origin + Vec3::new(59.0, 100.0, 0.0);

    let mut area = Area::build(&assets, &blocks_for(square, square), &cap, block).unwrap();
    assert!(!area.dungeon);
    assert_eq!(area.blocks.len(), 9, "a full neighbourhood around the town");
    assert!(
        area.collision.tris.len() > 30_000,
        "the walls and buildings are there: {} triangles",
        area.collision.tris.len()
    );
    let square = Vec3::new(
        square.x,
        square.y,
        area.terrain_at(square.x, square.y).unwrap(),
    );

    // Approach from the south-west, where the wall stands between us and
    // the square. Straight in is blocked; a route exists all the same.
    let bearing = 225f32.to_radians();
    let from = square + Vec3::new(130.0 * bearing.cos(), 130.0 * bearing.sin(), 0.0);
    let from = Vec3::new(from.x, from.y, area.terrain_at(from.x, from.y).unwrap());
    assert!(
        !area.line_clear(from + Vec3::Z, square + Vec3::Z),
        "the wall should be between us and the square"
    );
    let path = area
        .with_ground(|ground, nav| route(ground, nav, from, square, 0.5))
        .expect("a way in past the wall");
    let end = *path.last().unwrap();
    assert!(
        end.distance(square) < 1.0,
        "the route should reach the square, not stop short at {end:?}"
    );
    // The way round is longer than the way through, and leaves the block
    // the goal is in: that is the whole point of planning wide.
    let mut walked = 0.0;
    let mut prev = from;
    for w in &path {
        walked += prev.distance(*w);
        prev = *w;
    }
    assert!(
        walked > from.distance(square),
        "going round should be further than the straight line"
    );

    // The same walk planned on the goal's landblock alone: the start is
    // outside that block, so the single-block planner has nothing to
    // stand on and cannot answer at all.
    let mut one = Area::build(&assets, &[block], &cap, block).unwrap();
    assert!(
        one.with_ground(|ground, nav| route(ground, nav, from, square, 0.5))
            .is_none(),
        "one landblock alone should not be able to plan this walk"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_dungeon_is_planned_on_its_own_and_its_stairs_connect_its_floors() {
    let dir = ac_dat::test_data_dir();
    let assets = Assets::open(dir).unwrap();
    let cap = capsule();
    // The Old Talisman dungeon: several floors joined by stairs.
    let block = 0x01EA_0000;
    let square = lbid::world_origin(block) + Vec3::new(96.0, 96.0, 0.0);
    // Asking for a neighbourhood still gets just the dungeon: the blocks
    // beside it on the lattice are unrelated pieces of the surface.
    let mut area = Area::build(&assets, &blocks_for(square, square), &cap, block).unwrap();
    assert!(area.dungeon);
    assert_eq!(area.blocks, vec![block]);
    assert_eq!(area.terrain_at(square.x, square.y), None);

    area.build_everything();
    assert!(
        !area.nav.is_empty(),
        "the dungeon should have walkable floors"
    );

    // The biggest group of floors that can all reach each other should
    // span more than one storey: that only happens if the graph climbs.
    let n = area.nav.len();
    let mut seen = vec![false; n];
    let mut best_rise = 0.0f32;
    let mut best_size = 0usize;
    for s in 0..n {
        if seen[s] {
            continue;
        }
        seen[s] = true;
        let mut stack = vec![s as u32];
        let (mut lo, mut hi, mut size) = (f32::MAX, f32::MIN, 0usize);
        while let Some(v) = stack.pop() {
            size += 1;
            let z = area.nav.nodes[v as usize].pos.z;
            lo = lo.min(z);
            hi = hi.max(z);
            for e in area.nav.neighbours(v) {
                if !seen[e.to as usize] {
                    seen[e.to as usize] = true;
                    stack.push(e.to);
                }
            }
        }
        if size > best_size {
            best_size = size;
            best_rise = hi - lo;
        }
    }
    assert!(
        best_size > n / 2,
        "most of the dungeon should be one connected place, got {best_size} of {n}"
    );
    assert!(
        best_rise > 20.0,
        "the connected part should climb several floors, got {best_rise:.1} m"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_route_goes_round_the_ocean_rather_than_across_it() {
    let dir = ac_dat::test_data_dir();
    let assets = Assets::open(dir).unwrap();
    let cap = capsule();
    // Sanamar is a port: about six per cent of its neighbourhood is
    // open sea, so there are bays to walk round.
    let block = lbid::from_xy(0x33, 0xD9);
    let centre = lbid::world_origin(block) + Vec3::new(96.0, 96.0, 0.0);
    let mut area = Area::build(&assets, &blocks_for(centre, centre), &cap, block).unwrap();
    let lo = lbid::world_origin(*area.blocks.first().unwrap());
    let step = 4.0f32;
    let side = 3 * 192 / step as i32;
    let at = |i: i32, j: i32| (lo.x + i as f32 * step, lo.y + j as f32 * step);

    let sea = (0..side)
        .flat_map(|i| (0..side).map(move |j| (i, j)))
        .filter(|&(i, j)| {
            let (x, y) = at(i, j);
            area.is_sea(x, y)
        })
        .count();
    assert!(sea > 500, "Sanamar should have open water: {sea} points");

    // Points on the shore, and pairs of them with water between.
    let shore: Vec<(f32, f32)> = (0..side)
        .flat_map(|i| (0..side).map(move |j| at(i, j)))
        .filter(|&(x, y)| {
            !area.is_sea(x, y)
                && [(step, 0.0), (-step, 0.0), (0.0, step), (0.0, -step)]
                    .iter()
                    .any(|(dx, dy)| area.is_sea(x + dx, y + dy))
        })
        .collect();
    assert!(shore.len() > 20, "a coastline: {} points", shore.len());

    let mut tested = 0;
    for a in shore.iter().step_by(7) {
        for b in shore.iter().step_by(11) {
            let d = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
            if !(60.0..140.0).contains(&d) {
                continue;
            }
            let n = (d / 4.0) as i32;
            let across = (1..n).any(|k| {
                let t = k as f32 / n as f32;
                area.is_sea(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
            });
            if !across {
                continue;
            }
            let (Some(za), Some(zb)) = (area.terrain_at(a.0, a.1), area.terrain_at(b.0, b.1))
            else {
                continue;
            };
            let (pa, pb) = (Vec3::new(a.0, a.1, za), Vec3::new(b.0, b.1, zb));
            let Some(path) = area.with_ground(|ground, nav| route(ground, nav, pa, pb, 1.0)) else {
                continue;
            };
            let mut prev = pa;
            for w in &path {
                let steps = (prev.distance(*w) / 3.0).ceil() as i32;
                for k in 0..=steps {
                    let q = prev.lerp(*w, k as f32 / steps.max(1) as f32);
                    assert!(
                        !area.is_sea(q.x, q.y),
                        "the route walks into the sea at {q:?}"
                    );
                }
                prev = *w;
            }
            tested += 1;
            if tested >= 5 {
                return;
            }
        }
    }
    assert!(
        tested > 0,
        "no pair of shore points with water between them"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_goal_standing_on_a_deck_is_planned_to_on_the_deck() {
    // Oolutanga's Refuge's Jeweler, 11.5 m over the ground on a deck: the goal was grounded to the
    // spot beneath him, a one-step route there, and a character walked on the spot under him.
    let dir = ac_dat::test_data_dir();
    let assets = Assets::open(dir).unwrap();
    let cap = capsule();
    let block = 0xF682_0000;
    let origin = lbid::world_origin(block);
    let under = origin + Vec3::new(81.9, 28.56, 51.93);
    let jeweler = origin + Vec3::new(81.9037, 28.5586, 63.4465);
    let mut area = Area::build(&assets, &[block], &cap, block).unwrap();
    let (on_deck, path) = area.with_ground(|ground, nav| {
        let cell = wayfind::locate(ground, jeweler);
        let on_deck = wayfind::floor_of(ground, jeweler, cell, &cap).map(|f| f.0);
        (on_deck, route(ground, nav, under, jeweler, 0.5))
    });
    let on_deck = on_deck.expect("he stands on a floor");
    assert!(
        (on_deck.z - jeweler.z).abs() < 0.05,
        "he stands on the deck, not {on_deck}"
    );
    let path = path.expect("the way up");
    let end = *path.last().unwrap();
    assert!((end.z - jeweler.z).abs() < 0.6, "ends at {end:?}");
    assert!(path.len() > 3, "straight at him: {path:?}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_stairs_from_an_upper_floor_are_found_without_its_drop() {
    // Upstairs in a Holtburg house, the goal in the room under it 2.5 m down over the edge: a walk
    // that takes drops may step off, and one that takes none still finds the stairs.
    let assets = Assets::open(ac_dat::test_data_dir()).unwrap();
    let cap = Capsule::default();
    let block = 0xA9B4_0000;
    let me = Vec3::new(32_553.0, 34_594.5, 97.5);
    let goal = Vec3::new(32_550.17, 34_597.33, 94.0);
    let mut area = Area::build(&assets, &[block], &cap, block).unwrap();
    let path = area
        .with_ground(|ground, nav| route(ground, nav, me, goal, 0.5))
        .expect("the stairs down");
    assert!(path.len() > 4, "not the stairs: {path:?}");
    let end = *path.last().unwrap();
    assert!((end.z - goal.z).abs() < 0.6, "ends at {end:?}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_hillside_steeper_than_a_stair_is_joined_on_the_outdoor_lattice() {
    // A canyon wall at the corner of F884, F885, F984 and F985: its ground climbs 2.3 m and more
    // between nodes 4 m apart, under ACE's walkable slope; down it and up the hill are both walks.
    let assets = Assets::open(ac_dat::test_data_dir()).unwrap();
    let cap = Capsule::default();
    let corner = Vec3::new(47_808.0, 25_536.05, 63.98);
    for goal in [
        Vec3::new(47_808.0, 25_476.05, 63.03),
        Vec3::new(47_786.5, 25_480.0, 103.88),
        Vec3::new(47_778.2, 25_484.0, 112.27),
    ] {
        let mut area = Area::build(&assets, &blocks_for(corner, goal), &cap, 0xF985_0000).unwrap();
        let path = area.with_ground(|ground, nav| route(ground, nav, corner, goal, 1.5));
        let path = path.unwrap_or_else(|| panic!("no way from the corner to {goal}"));
        let end = *path.last().unwrap();
        assert!(end.distance(goal) <= 1.5 + 0.6, "ends at {end} for {goal}");
    }
}
