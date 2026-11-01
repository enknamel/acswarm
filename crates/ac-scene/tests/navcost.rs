//! What the walking graph costs (docs/movement-phase1-design.md §7): each measured block built
//! whole, its nodes and edges, and the plan time over seeded pairs of its nodes. Prints `COST`
//! and `PLAN` lines and asserts nothing about time; run it before and after a change to edges.

use std::time::Instant;

use ac_scene::nav::EdgeKind;
use ac_scene::wayfind::{self, Search, WalkQuery};
use ac_scene::{collision::Capsule, navarea::Area, Assets};

/// The blocks measured: Holtburg, Oolutanga's Refuge, the Facility Hub, the Hea dungeon.
const BLOCKS: [u32; 4] = [0xA9B4_0000, 0xF682_0000, 0x8A02_0000, 0x0286_0000];
/// Seeded node pairs planned between per block.
const PAIRS: usize = 20;

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

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_graph_of_each_measured_block_is_built_and_planned_on() {
    let assets = Assets::open(ac_dat::test_data_dir()).unwrap();
    let cap = Capsule::default();
    for block in BLOCKS {
        let mut area = Area::build(&assets, &[block], &cap, block).unwrap();
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
            "COST {block:#010x} nodes {} edges {} walk/drop/doorway {walk}/{drop}/{doorway} chunks {} ms {ms:.0}",
            nav.len(),
            nav.edge_count(),
            nav.chunk_count()
        );
        assert!(!nav.is_empty(), "{block:#010x} has no graph");
        let mut rng = Seeded(0x9E37_79B9_7F4A_7C15 ^ block as u64);
        let (mut found, mut total) = (0, 0.0);
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
            };
            let t = Instant::now();
            let search = area.with_ground(|ground, nav| wayfind::search_building(ground, nav, &q));
            found += usize::from(matches!(search, Search::Found(_)));
            total += t.elapsed().as_secs_f64() * 1e3;
        }
        println!("PLAN {block:#010x} pairs {PAIRS} routed {found} ms {total:.1}");
    }
}
