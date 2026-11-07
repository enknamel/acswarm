//! The route harness's trials (docs/movement-plan.md, phase 0): the stalls seen live and seeded
//! random walks in the areas they came from, run offline through the real walk. It reports how
//! each ended and what the planner answered first; `ROUTE_TRIALS=name` runs those named so.

use ac_client::walk_trial::{
    journey, named, plan_first, random_trials, run, run_on, run_with_waits, standing_at, Outcome,
    Spread, Trial,
};
use ac_nav::routing::Planned;
use ac_scene::nav::EdgeKind;
use ac_world::Place;
use glam::Vec3;

const LEVEL: i32 = 30;

/// The stalls seen live, start and goal.
fn known() -> Vec<Trial> {
    vec![
        named(
            LEVEL,
            "DA3B Shoichi indoors in Lin from the road east",
            Vec3::new(42_239.613, 11_472.488, 20.0),
            0xDA3B_0116,
            Vec3::new(41_966.64, 11_405.92, 6.8),
        ),
        named(
            LEVEL,
            "0286 corpse on the floor below",
            Vec3::new(419.98, 25_673.98, 0.0),
            0x0286_02BF,
            Vec3::new(386.725, 25_627.996, -6.0),
        ),
        named(
            LEVEL,
            "F682 Jeweler on a deck",
            Vec3::new(47_313.9, 24_988.56, 51.93),
            0xF682_001A,
            Vec3::new(47_313.904, 24_988.559, 63.4465),
        ),
        named(
            LEVEL,
            "F682 Mage on a deck",
            Vec3::new(47_406.61, 25_004.78, 58.0),
            0xF682_003A,
            Vec3::new(47_408.62, 25_004.1, 63.605),
        ),
        named(
            LEVEL,
            "9722 Jeweler from inside a building",
            Vec3::new(29_071.6, 6_613.4, 101.2),
            0x9722_012E,
            Vec3::new(29_094.9, 6_634.8, 102.0),
        ),
        named(
            LEVEL,
            "A21E Kifandal up Linvak Tukal from the landing",
            Vec3::new(31_188.98, 5_798.57, 560.34),
            0xA21E_001A,
            Vec3::new(31_160.1, 5_864.8, 639.5),
        ),
        named(
            LEVEL,
            "A21E Kifandal from the stair landing",
            Vec3::new(31_232.52, 5_862.49, 567.93),
            0xA21E_002D,
            Vec3::new(31_160.1, 5_864.8, 639.5),
        ),
        named(
            LEVEL,
            "A21E Kifandal from where the landing replanned",
            Vec3::new(31_232.057, 5_862.313, 567.914),
            0xA21E_002D,
            Vec3::new(31_160.1, 5_864.8, 639.5),
        ),
        named(
            LEVEL,
            "A21E Kifandal from below the landing",
            Vec3::new(31_229.68, 5_861.95, 561.51),
            0xA21E_002D,
            Vec3::new(31_160.1, 5_864.8, 639.5),
        ),
        named(
            LEVEL,
            "A21E Kifandal from the stair foot",
            Vec3::new(31_231.11, 5_833.52, 560.06),
            0xA21E_002C,
            Vec3::new(31_160.1, 5_864.8, 639.5),
        ),
        named(
            LEVEL,
            "8A02 Hub stair foot to a Warden",
            Vec3::new(26_559.613, 342.481, -6.0),
            0x8A02_015E,
            Vec3::new(26_572.25, 312.46, 0.0),
        ),
        named(
            LEVEL,
            "8A02 Hub stair foot to the lower floor",
            Vec3::new(26_552.4, 342.5, -6.0),
            0x8A02_015E,
            Vec3::new(26_546.0, 344.0, -12.0),
        ),
        named(
            LEVEL,
            "01F6 Holtburg Dungeon corpse 1",
            Vec3::new(288.7, 47_222.0, 0.0),
            0x01F6_0289,
            Vec3::new(220.832, 47_190.047, 0.0),
        ),
        named(
            LEVEL,
            "01F6 Holtburg Dungeon corpse 2",
            Vec3::new(288.7, 47_222.0, 0.0),
            0x01F6_0289,
            Vec3::new(239.049, 47_221.76, 0.0),
        ),
        named(
            LEVEL,
            "C6A8 out of Magus Guthima's shop",
            Vec3::new(38_083.2, 32_344.1, 42.0),
            0xC6A8_0101,
            Vec3::new(38_088.0, 32_352.0, 42.0),
        ),
        named(
            LEVEL,
            "F985 from a four-block corner",
            Vec3::new(47_808.0, 25_536.05, 64.0),
            0xF985_0001,
            Vec3::new(47_808.0, 25_476.05, 63.0),
        ),
        named(
            LEVEL,
            "2CB5 coast block edge",
            Vec3::new(8_448.6, 34_823.9, 0.0),
            0x2CB5_0003,
            Vec3::new(8_472.6, 34_823.9, 0.0),
        ),
        named(
            LEVEL,
            "5C49 Tusker on a ledge over the room",
            Vec3::new(17_744.8, 13_910.7, 0.0),
            0x5C49_012A,
            Vec3::new(17_754.0, 13_929.0, 6.0),
        ),
    ]
}

/// A world position `local` in `cell`'s block, with that cell.
fn at(cell: u32, local: Vec3) -> (Vec3, u32) {
    (ac_world::landblock_origin(cell) + local, cell)
}

/// Open ground at world `xy` (the builders' pick, on the terrain), in the outdoor cell under it.
fn open_ground(x: f32, y: f32) -> (Vec3, u32) {
    let p = Place::on_map(glam::Vec2::new(x, y));
    (p.at, p.cell)
}

/// Journeys from one region to another, through portals or overland (docs/movement-phase4-design.md
/// §8): the Warden visit seen live, a walk into a building and upstairs, two walks out of one.
fn journeys() -> Vec<Trial> {
    let rooms = ac_client::testkit::game_data()
        .block_collision(0x01F6_0000)
        .expect("the Holtburg Dungeon");
    let ledge = Vec3::new(262.0, 47_182.0, 9.5);
    let (_, ledge_cell) = rooms.world.floor_at(ledge, 0.6, 1.5).expect("the ledge");
    let lifestone = at(0xA9B4_0019, Vec3::new(84.0, 7.1, 94.0));
    vec![
        journey(
            LEVEL,
            "A9B4 Holtburg to a Facility Hub Warden",
            (Vec3::new(32_532.0, 34_567.1, 94.0), 0xA9B4_0019),
            at(0x8A02_01F7, Vec3::new(108.34, -46.2, 0.0)),
        ),
        journey(
            LEVEL,
            "A9B2 platform to Archmage Cindrue upstairs",
            (Vec3::new(32_526.2, 34_261.9, 97.5), 0xA9B2_0000),
            at(0xA9B4_011B, Vec3::new(152.3, 132.5, 69.0)),
        ),
        journey(
            LEVEL,
            "E632 out of Archmage Yuzai Zhen's",
            at(0xE632_0103, Vec3::new(112.1, 131.0, 30.1)),
            open_ground(44_272.1, 9_331.0),
        ),
        journey(
            LEVEL,
            "ACB5 mine floor to the open ground",
            at(0xACB5_01E2, Vec3::new(125.5, 134.1, 40.4)),
            open_ground(33_549.5, 34_886.1),
        ),
        journey(
            LEVEL,
            "A9B4 Holtburg to the 01F6 ledge",
            lifestone,
            (ledge, ledge_cell),
        ),
        journey(
            LEVEL,
            "0101 Murk Warrens arrival to its Surface Portal",
            (Vec3::new(232.0, -68.0, 0.0), 0x0101_0121),
            (Vec3::new(202.0, -38.0, 24.0), 0x0101_0465),
        ),
    ]
}

/// Areas walked about at random, by a start in each.
fn areas() -> Vec<(&'static str, Vec3, u32)> {
    vec![
        (
            "A9B4 Holtburg",
            Vec3::new(32_532.0, 34_567.1, 94.0),
            0xA9B4_0019,
        ),
        (
            "01F6 Holtburg Dungeon",
            Vec3::new(288.7, 47_222.0, 0.0),
            0x01F6_0289,
        ),
        (
            "8A02 Facility Hub",
            Vec3::new(26_552.4, 342.5, -6.0),
            0x8A02_015E,
        ),
        (
            "0286 Hea dungeon",
            Vec3::new(419.98, 25_673.98, 0.0),
            0x0286_02BF,
        ),
        (
            "F682 Oolutanga's Refuge",
            Vec3::new(47_313.9, 24_988.56, 51.93),
            0xF682_001A,
        ),
        (
            "9722 Qalaba'r",
            Vec3::new(29_071.6, 6_613.4, 101.2),
            0x9722_012E,
        ),
        (
            "B2A1 end of a long walk",
            Vec3::new(34_273.5, 30_958.9, 90.0),
            0xB2A1_0022,
        ),
        (
            "A9B2 platform",
            Vec3::new(32_526.2, 34_261.9, 97.5),
            0xA9B2_0000,
        ),
        (
            "5C49 Tusker ground",
            Vec3::new(17_744.8, 13_910.7, 0.0),
            0x5C49_012A,
        ),
    ]
}

/// Run and print each trial: the names of those the planner called reachable that did not
/// arrive, and of those still pushing at the limit.
fn report(trials: &[Trial]) -> (Vec<String>, Vec<String>) {
    let only = std::env::var("ROUTE_TRIALS").ok();
    let mut out = Vec::new();
    let mut agree = [0usize; 4];
    let mut waited = 0;
    let mut broken = Vec::new();
    for t in trials
        .iter()
        .filter(|t| only.as_ref().is_none_or(|o| t.name.contains(o.as_str())))
    {
        let mut c = standing_at(LEVEL, t);
        let (planned, took) = plan_first(&mut c, t);
        let (o, waits) = run_with_waits(&mut c, t);
        waited += waits;
        let plan = match &planned {
            Ok(n) => format!("route {n}"),
            Err(why) => format!("no way {why:?}"),
        };
        let arrived = o.kind() == "arrived";
        agree[usize::from(planned.is_err()) * 2 + usize::from(!arrived)] += 1;
        if planned.is_ok() && !arrived {
            broken.push(t.name.clone());
        }
        println!(
            "TRIAL {:<44} {:>6.1} m  {:<13} PLAN {plan} {:.0} ms  WAITED {waits}  {:?}",
            t.name,
            t.start.distance(t.goal),
            o.kind(),
            took.as_secs_f64() * 1e3,
            o
        );
        out.push((t.name.clone(), o));
    }
    let pushing: Vec<String> = out
        .iter()
        .filter(|(_, o)| o.kind() == "still pushing")
        .map(|(n, _)| n.clone())
        .collect();
    let count = |k: &str| out.iter().filter(|(_, o)| o.kind() == k).count();
    println!(
        "SUMMARY {} trials: {} arrived, {} refused, {} gave up, {} still pushing",
        out.len(),
        count("arrived"),
        count("refused"),
        count("gave up"),
        count("still pushing")
    );
    println!(
        "AGREE planned+arrived {}, planned+failed {}, no-way+arrived {}, no-way+failed {}",
        agree[0], agree[1], agree[2], agree[3]
    );
    println!("WAITED {waited} frames standing for the planner to build");
    println!(
        "INVARIANT planned-must-arrive: {} of {} arrived; failed: {broken:?}",
        agree[0],
        agree[0] + agree[1]
    );
    (broken, pushing)
}

/// The stall seen live named `name`.
fn stall(name: &str) -> Trial {
    known()
        .into_iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("no stall named {name}"))
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_stalls_seen_live() {
    let (broken, pushing) = report(&known());
    assert!(broken.is_empty(), "planned and not arrived: {broken:?}");
    assert!(pushing.is_empty(), "still pushing: {pushing:?}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn random_walks_in_each_area() {
    let mut trials = Vec::new();
    for (i, (name, at, cell)) in areas().into_iter().enumerate() {
        let spread = Spread {
            radius: 60.0,
            reach: 60.0,
            least: 10.0,
            n: 15,
            seed: 1 + i as u64,
        };
        trials.extend(random_trials(LEVEL, name, (at, cell), spread));
    }
    let (broken, pushing) = report(&trials);
    assert!(broken.is_empty(), "planned and not arrived: {broken:?}");
    assert!(pushing.is_empty(), "still pushing: {pushing:?}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn journeys_across_regions() {
    let (broken, pushing) = report(&journeys());
    assert!(broken.is_empty(), "planned and not arrived: {broken:?}");
    assert!(pushing.is_empty(), "still pushing: {pushing:?}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_corpse_on_the_floor_below_is_walked_down_to() {
    let o = run(LEVEL, &stall("0286 corpse on the floor below"));
    assert!(matches!(o, Outcome::Arrived { .. }), "{o:?}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_jeweler_on_a_deck_is_walked_up_to() {
    let o = run(LEVEL, &stall("F682 Jeweler on a deck"));
    assert!(matches!(o, Outcome::Arrived { .. }), "{o:?}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_building_whose_sill_meets_the_ground_is_walked_into() {
    // Lin's floor runs past its cell onto ground of the same height: the sill is stood on outdoors.
    let o = run(
        LEVEL,
        &stall("DA3B Shoichi indoors in Lin from the road east"),
    );
    assert!(matches!(o, Outcome::Arrived { .. }), "{o:?}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_ledge_opening_only_onto_air_is_refused_and_dropped_from() {
    // The Tuskers' alcoves 6 m up open only onto the floorless upper half of the room below.
    let t = stall("5C49 Tusker on a ledge over the room");
    let o = run(LEVEL, &t);
    assert!(matches!(o, Outcome::Refused { .. }), "{o:?}");
    let mut c = standing_at(LEVEL, &t);
    let pl = c.player.as_ref().expect("a body");
    let me = Place {
        at: pl.world_position(),
        cell: pl.cell,
    };
    let cap = pl.capsule();
    let ledge = Place {
        at: t.goal,
        cell: t.goal_cell,
    };
    let down = c.pathfinder.plan(ledge, me, t.stop, cap);
    let Planned::Route(r) = down else {
        panic!("no way down: {down:?}")
    };
    assert!(
        r.waypoints.iter().any(|w| w.kind == EdgeKind::Drop),
        "{r:?}"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_walk_inside_a_building_is_planned_on_the_region_not_the_block() {
    // The steering's plan is the region's: a route checked to its end, or why there is none. The
    // walk is reported here; the stalls' trial asserts it.
    let t = stall("9722 Jeweler from inside a building");
    let mut c = standing_at(LEVEL, &t);
    let pl = c.player.as_ref().expect("a body");
    let me = Place {
        at: pl.world_position(),
        cell: pl.cell,
    };
    let cap = pl.capsule();
    let goal = Place {
        at: t.goal,
        cell: t.goal_cell,
    };
    match c.pathfinder.plan(me, goal, t.stop, cap) {
        Planned::Route(r) => {
            let end = r.waypoints.last().expect("a route has an end").place.at;
            assert!(end.distance(t.goal) <= t.stop, "ends at {end:?}");
        }
        Planned::NoWay(why) => println!("PLAN no way {why:?}"),
        Planned::Building(_) => panic!("an offline plan answers once built"),
    }
    println!("WALK {:?}", run_on(&mut c, &t));
}
