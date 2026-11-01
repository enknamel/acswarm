//! The route harness's trials (docs/movement-plan.md, phase 0): the stalls seen live and seeded
//! random walks in the areas they came from, run offline through the real walk. It reports how
//! each ended and what the planner answered first; `ROUTE_TRIALS=name` runs those named so.

use ac_client::walk_trial::{
    named, plan_first, random_trials, run, run_on, standing_at, Outcome, Spread, Trial,
};
use glam::Vec3;

const LEVEL: i32 = 30;

/// The stalls seen live, start and goal.
fn known() -> Vec<Trial> {
    vec![
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
            "2CB5 coast block edge",
            Vec3::new(8_448.6, 34_823.9, 0.0),
            0x2CB5_0003,
            Vec3::new(8_472.6, 34_823.9, 0.0),
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
    ]
}

fn report(trials: &[Trial]) -> Vec<(String, Outcome)> {
    let only = std::env::var("ROUTE_TRIALS").ok();
    let mut out = Vec::new();
    let mut agree = [0usize; 4];
    for t in trials
        .iter()
        .filter(|t| only.as_ref().is_none_or(|o| t.name.contains(o.as_str())))
    {
        let mut c = standing_at(LEVEL, t);
        let (planned, took) = plan_first(&mut c, t);
        let o = run_on(&mut c, t);
        let plan = match &planned {
            Ok(n) => format!("route {n}"),
            Err(why) => format!("no way {why:?}"),
        };
        let arrived = o.kind() == "arrived";
        agree[usize::from(planned.is_err()) * 2 + usize::from(!arrived)] += 1;
        println!(
            "TRIAL {:<44} {:>6.1} m  {:<13} PLAN {plan} {:.0} ms  {:?}",
            t.name,
            t.start.distance(t.goal),
            o.kind(),
            took.as_secs_f64() * 1e3,
            o
        );
        out.push((t.name.clone(), o));
    }
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
    out
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
    report(&known());
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
    report(&trials);
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
