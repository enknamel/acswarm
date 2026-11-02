//! What the walking graph costs (docs/movement-phase1-design.md §7): each measured block built
//! whole, its nodes and edges, and the plan time over seeded pairs of its nodes, by answer.
//! Prints `COST` and `PLAN` lines (`PLAN9` on nine blocks) and asserts nothing about time.

use std::time::Instant;

use ac_scene::nav::EdgeKind;
use ac_scene::navarea::blocks_for;
use ac_scene::wayfind::{self, NoWay, Search, WalkQuery};
use ac_scene::{collision::Capsule, lbid, navarea::Area, Assets};
use glam::Vec3;

/// The blocks measured: Holtburg, Oolutanga's Refuge, the Facility Hub, the Hea dungeon.
const BLOCKS: [u32; 4] = [0xA9B4_0000, 0xF682_0000, 0x8A02_0000, 0x0286_0000];
/// The towns also measured on the nine blocks round them, as the builder assembles them.
const REGIONS: [u32; 2] = [0xA9B4_0000, 0xF682_0000];
/// Seeded node pairs planned between per area.
const PAIRS: usize = 40;

/// A seeded xorshift, so the pairs are the same every run.
struct Seeded(u64);

impl Seeded {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

/// Build `area` whole, print its `COST` line, and plan `PAIRS` seeded pairs on what is built, as
/// a frame plans: one `tag` line of answers and times.
fn measure(tag: &str, block: u32, area: &mut Area) {
    let started = Instant::now();
    area.build_everything();
    let ms = started.elapsed().as_secs_f64() * 1e3;
    let nav = &area.nav;
    let (mut walk, mut drop, mut doorway) = (0, 0, 0);
    for i in 0..nav.len() as u32 {
        for e in nav.neighbours(i) {
            match e.kind {
                EdgeKind::Walk => walk += 1,
                EdgeKind::Drop => drop += 1,
                EdgeKind::Doorway => doorway += 1,
            }
        }
    }
    println!(
        "COST {tag} {block:#010x} nodes {} edges {} walk/drop/doorway {walk}/{drop}/{doorway} chunks {} ms {ms:.0}",
        nav.len(),
        nav.edge_count(),
        nav.chunk_count()
    );
    assert!(!nav.is_empty(), "{block:#010x} has no graph");
    let mut rng = Seeded(0x9E37_79B9_7F4A_7C15 ^ block as u64);
    let (mut routed, mut apart, mut unreached) = (0, 0, 0);
    let (mut times, mut unreached_ms) = (Vec::new(), 0.0);
    for _ in 0..PAIRS {
        let n = area.nav.len();
        let (a, b) = (rng.below(n), rng.below(n));
        let (from, to) = (area.nav.nodes[a], area.nav.nodes[b]);
        let q = WalkQuery {
            from: (from.pos, from.cell),
            goal: (to.pos, to.cell),
            within: 0.5,
            avoid: &[],
            berth: 0.0,
            indoors_too: true,
            outdoors_first: !area.dungeon && from.cell == 0 && to.cell == 0,
            drops: false,
        };
        let t = Instant::now();
        let search = area.read_ground(|ground, nav| wayfind::search(ground, nav, &q));
        let took = t.elapsed().as_secs_f64() * 1e3;
        times.push(took);
        match search {
            Search::Found(_) => routed += 1,
            Search::NoWay(NoWay::Apart) => apart += 1,
            Search::NoWay(NoWay::Unreached) => {
                unreached += 1;
                unreached_ms += took;
            }
            _ => {}
        }
    }
    let total: f64 = times.iter().sum();
    times.sort_by(f64::total_cmp);
    let at = |f: f64| times[((times.len() - 1) as f64 * f).round() as usize];
    println!(
        "{tag} {block:#010x} pairs {PAIRS} routed {routed} apart {apart} unreached {unreached} ms {total:.1} median {:.2} p95 {:.1} max {:.1} unreached ms {unreached_ms:.1}",
        at(0.5),
        at(0.95),
        at(1.0)
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_graph_of_each_measured_block_is_built_and_planned_on() {
    let assets = Assets::open(ac_dat::test_data_dir()).unwrap();
    let cap = Capsule::default();
    for block in BLOCKS {
        let mut area = Area::build(&assets, &[block], &cap, block).unwrap();
        measure("PLAN", block, &mut area);
    }
    for block in REGIONS {
        let middle = lbid::world_origin(block) + Vec3::new(96.0, 96.0, 0.0);
        let blocks = blocks_for(middle, middle);
        let mut area = Area::build(&assets, &blocks, &cap, block).unwrap();
        measure("PLAN9", block, &mut area);
    }
}
