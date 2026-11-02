//! The walking graph against the body (docs/movement-phase1-design.md §4): seeded edges of two
//! areas' graphs, each walked by a testkit body from its near end at its far end, frame by frame
//! as the client walks, a drop fallen and walked on from. An edge the body does not walk is a
//! route the follower cannot keep.

use ac_client::player::Player;
use ac_client::testkit::human;
use ac_scene::collision::Capsule;
use ac_scene::nav::{EdgeKind, NavNode, BODY_ARRIVE, DROP_SPEED};
use ac_scene::navarea::Area;
use ac_scene::Assets;
use glam::{Quat, Vec2, Vec3};

/// Edges walked per area.
const EDGES: usize = 200;
/// Frames a second, as the headless client ticks.
const HZ: f32 = 20.0;
/// How long one edge is given (seconds).
const LIMIT: f32 = 3.0;

/// A seeded xorshift, so the edges are the same every run.
struct Seeded(u64);

impl Seeded {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

/// A body standing on node `n`, in its cell or the outdoor cell under it.
fn standing_on(assets: &Assets, n: &NavNode) -> Player {
    let cell = if n.cell != 0 {
        n.cell
    } else {
        let block = ac_scene::navarea::block_at(n.pos);
        ac_world::outdoor_cell(block, n.pos - ac_world::landblock_origin(block))
    };
    human(
        assets,
        cell,
        n.pos - ac_world::landblock_origin(cell),
        Quat::IDENTITY,
    )
}

/// Where a body walked from `a` at `b` for `limit` seconds, facing it each frame and never
/// stepping past it, held to `pace` (m/s) when given, ends.
fn walked(assets: &Assets, a: &NavNode, b: &NavNode, pace: Option<f32>, limit: f32) -> Vec3 {
    let mut pl = standing_on(assets, a);
    pl.pace_cap = pace;
    let input = ac_client::player::Input {
        forward: 1.0,
        ..Default::default()
    };
    for _ in 0..(limit * HZ) as usize {
        let d = b.pos - pl.world_position();
        let left = Vec2::new(d.x, d.y).length();
        // Off a drop it falls where it falls, and walks on to the node once down.
        if pl.is_airborne() {
            pl.update(assets, &ac_client::player::Input::default(), 1.0 / HZ);
            continue;
        }
        // There once within reach on its floor; short of a drop's edge, walking on to it.
        if left <= BODY_ARRIVE && (b.pos.z - pl.world_position().z).abs() <= pl.capsule().step_up {
            break;
        }
        pl.heading = (-d.x).atan2(d.y);
        pl.step_cap = Some(left);
        pl.update(assets, &input, 1.0 / HZ);
    }
    pl.world_position()
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn every_sampled_edge_is_walked_by_the_body() {
    let assets = Assets::open(ac_dat::test_data_dir()).unwrap();
    let cap = Capsule::default();
    let mut missed = Vec::new();
    for block in [0xA9B4_0000u32, 0x0286_0000] {
        let mut area = Area::build(&assets, &[block], &cap, block).unwrap();
        area.build_everything();
        let nav = &area.nav;
        let mut rng = Seeded(0x2545_F491_4F6C_DD1D ^ block as u64);
        let mut tried = 0;
        while tried < EDGES {
            let i = rng.below(nav.len()) as u32;
            let out = nav.neighbours(i);
            if out.is_empty() {
                continue;
            }
            tried += 1;
            let (a, b) = (
                nav.nodes[i as usize],
                nav.nodes[out[rng.below(out.len())].to as usize],
            );
            let end = walked(&assets, &a, &b, None, LIMIT);
            let flat = Vec2::new(end.x - b.pos.x, end.y - b.pos.y).length();
            // Arrived on the far node's floor: within reach of it, and within a step of its height.
            if flat > BODY_ARRIVE + 0.05 || (end.z - b.pos.z).abs() > cap.step_up {
                missed.push(format!(
                    "{block:#010x}: {:?} {:#x} -> {:?} {:#x} ended {end:?}",
                    a.pos, a.cell, b.pos, b.cell
                ));
            }
        }
    }
    assert!(
        missed.is_empty(),
        "{} edges not walked:\n{}",
        missed.len(),
        missed.join("\n")
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn every_sampled_drop_is_landed_at_drop_pace() {
    // The follower steps off a drop at the pace the graph's fall was simulated at: the body
    // walked so must land where the edge does.
    let assets = Assets::open(ac_dat::test_data_dir()).unwrap();
    let cap = Capsule::default();
    let mut missed = Vec::new();
    let mut tried = 0;
    for block in [0xA9B4_0000u32, 0x0286_0000, 0x01F6_0000] {
        let mut area = Area::build(&assets, &[block], &cap, block).unwrap();
        area.build_everything();
        let nav = &area.nav;
        let drops: Vec<(u32, u32)> = (0..nav.len() as u32)
            .flat_map(|i| {
                nav.neighbours(i)
                    .iter()
                    .filter(|e| e.kind == EdgeKind::Drop)
                    .map(move |e| (i, e.to))
            })
            .collect();
        let mut rng = Seeded(0x2545_F491_4F6C_DD1D ^ block as u64);
        for _ in 0..drops.len().min(EDGES) {
            let (i, j) = drops[rng.below(drops.len())];
            let (a, b) = (nav.nodes[i as usize], nav.nodes[j as usize]);
            tried += 1;
            // The leg at the drop's pace, then the edge's own time to fall and walk on.
            let limit = a.pos.distance(b.pos) / DROP_SPEED + LIMIT;
            let end = walked(&assets, &a, &b, Some(DROP_SPEED), limit);
            let flat = Vec2::new(end.x - b.pos.x, end.y - b.pos.y).length();
            if flat > BODY_ARRIVE + 0.05 || (end.z - b.pos.z).abs() > cap.step_up {
                missed.push(format!(
                    "{block:#010x}: {:?} {:#x} -> {:?} {:#x} ended {end:?}",
                    a.pos, a.cell, b.pos, b.cell
                ));
            }
        }
    }
    assert!(tried > 0, "no drops in the areas sampled");
    assert!(
        missed.is_empty(),
        "{} of {tried} drops not landed:\n{}",
        missed.len(),
        missed.join("\n")
    );
}
