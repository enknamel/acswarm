//! Dereth's landmasses: the [`WorldGrid`] lattice's walkable vertices (all but blocks all water, as
//! [`worldroute`](crate::worldroute) walks them) labelled by the piece of land they join, so a
//! trip planner never takes a walk to an island the sea cuts off; and labelled again with a rise
//! steeper than `worldroute` climbs parting them too: the plateau of Corcima Castle's wards.

use ac_formats::region::Region;
use glam::Vec2;

use crate::worldgrid::{WorldGrid, SIDE, SPACING};
use crate::worldroute::{is_sunk_block, water_kind, Water, MAX_GRADE};

/// How far (vertices) a spot in the sea looks for the land it stands off: a portal on a jetty.
const SNAP_RADIUS: i32 = 3;

/// Two labels per lattice vertex, 0 for the sea and blocks the archive lacks.
#[derive(Clone, Debug)]
pub struct Landmasses {
    /// The land the sea parts.
    sea: Vec<u16>,
    /// The ground the sea or a cliff parts, as [`worldroute`](crate::worldroute) walks it.
    ground: Vec<u16>,
}

impl Landmasses {
    /// Label every walkable vertex by flood fill, 4-connected: two pieces meeting only at a
    /// corner are apart, the side `worldroute`'s diagonal rule takes too.
    pub fn of_world(grid: &WorldGrid, region: &Region) -> Landmasses {
        let water: Vec<Water> = (0..32u16).map(|t| water_kind(region, t)).collect();
        let kind = |t: u16| water.get(t as usize).copied().unwrap_or(Water::Sea);
        let mut sunk = vec![false; 255 * 255];
        for (i, s) in sunk.iter_mut().enumerate() {
            *s = is_sunk_block(grid, &kind, i / 255, i % 255);
        }
        let walkable = |i: usize| {
            let b = (((i / SIDE) / 8).min(254), ((i % SIDE) / 8).min(254));
            grid.has_block(b.0, b.1) && !sunk[b.0 * 255 + b.1]
        };
        let height = |i: usize| grid.height(i / SIDE, i % SIDE);
        let climbs = |i: usize, j: usize| (height(i) - height(j)).abs() <= MAX_GRADE * SPACING;
        Landmasses {
            sea: flood(&walkable, &|_, _| true),
            ground: flood(&walkable, &climbs),
        }
    }

    /// The landmass under world xy `at`, or the nearest within 3 vertices, as (the sea's label,
    /// the ground's); `None` at sea.
    pub fn at(&self, at: Vec2) -> Option<(u32, u32)> {
        let (gx, gy) = WorldGrid::nearest_vertex(at);
        let (gx, gy) = (gx as i32, gy as i32);
        let i = (0..=SNAP_RADIUS).find_map(|r| {
            let ring = (-r..=r).flat_map(|dx| (-r..=r).map(move |dy| (dx, dy)));
            ring.filter(|(dx, dy)| dx.abs().max(dy.abs()) == r)
                .filter_map(|(dx, dy)| {
                    let (x, y) = (gx + dx, gy + dy);
                    let inside = (0..SIDE as i32).contains(&x) && (0..SIDE as i32).contains(&y);
                    inside.then_some(x as usize * SIDE + y as usize)
                })
                .find(|i| self.sea[*i] != 0)
        })?;
        Some((u32::from(self.sea[i]), u32::from(self.ground[i])))
    }
}

/// Each `walkable` lattice index labelled by flood fill over the steps between 4-neighbours
/// `joins` takes; 0 off it.
fn flood(walkable: &dyn Fn(usize) -> bool, joins: &dyn Fn(usize, usize) -> bool) -> Vec<u16> {
    let mut labels = vec![0u16; SIDE * SIDE];
    let mut next = 0u16;
    let mut stack = Vec::new();
    for start in 0..SIDE * SIDE {
        if labels[start] != 0 || !walkable(start) {
            continue;
        }
        // About 7,500 grounds in Dereth: a u16 halves the 16 MB a u32 label grid takes.
        next = next.checked_add(1).expect("fewer than 65,536 landmasses");
        labels[start] = next;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (gx, gy) = (i / SIDE, i % SIDE);
            let near = [
                (gx.wrapping_sub(1), gy),
                (gx + 1, gy),
                (gx, gy.wrapping_sub(1)),
                (gx, gy + 1),
            ];
            for (nx, ny) in near {
                if nx >= SIDE || ny >= SIDE {
                    continue;
                }
                let j = nx * SIDE + ny;
                if labels[j] == 0 && walkable(j) && joins(i, j) {
                    labels[j] = next;
                    stack.push(j);
                }
            }
        }
    }
    labels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs AC_DATA_DIR"]
    fn ithaenc_is_an_island_and_corcimas_ward_a_plateau_off_the_mainland() {
        let assets = crate::Assets::open(ac_dat::test_data_dir()).unwrap();
        let region = assets.region().unwrap();
        let cache = std::env::temp_dir().join("acswarm-test-cache");
        let grid = WorldGrid::load_cached(&assets, &cache).unwrap();
        let t0 = std::time::Instant::now();
        let land = Landmasses::of_world(&grid, &region);
        eprintln!("labelled in {:?}", t0.elapsed());
        let map = |ns: f32, ew: f32| Vec2::new((ew + 102.0) * 240.0, (ns + 102.0) * 240.0);
        let in_block = |cell: u32, x: f32, y: f32| {
            land.at(crate::lbid::world_origin(cell).truncate() + Vec2::new(x, y))
        };
        let sea = |l: Option<(u32, u32)>| l.map(|l| l.0);
        let ground = |l: Option<(u32, u32)>| l.map(|l| l.1);
        let holtburg = land.at(map(42.1, 33.6));
        let arwic = land.at(map(33.6, 56.8));
        let nanto = in_block(0xE63E_0000, 100.0, 100.0);
        // The Catacombs of Ithaenc's portal, on the island; the invitation's exit beside it.
        let ithaenc = in_block(0xF418_0000, 100.0, 100.0);
        let invitation = in_block(0xF618_0000, 40.0, 89.0);
        assert!(holtburg.is_some() && ithaenc.is_some());
        assert_eq!(holtburg, arwic, "the mainland in two");
        assert_eq!(sea(holtburg), sea(nanto), "Nanto off the mainland");
        assert_ne!(sea(nanto), sea(ithaenc), "Ithaenc walked to from Nanto");
        assert_eq!(
            sea(invitation),
            sea(ithaenc),
            "the invitation lands off the island"
        );
        // The ruins' rise between: a walk there is flown (`ac_client::travel::flown`).
        assert_ne!(ground(invitation), ground(ithaenc), "the ruins walked over");
        // Corcima's Silver Ward: the Castellan's plateau, 160 m sheer over the land the Plains
        // of Ruin portal lands on, which the sea parts from the mainland.
        let ward = in_block(0x41E7_0000, 81.3, 105.3);
        let ruin = in_block(0x3FF0_0000, 60.0, 132.0);
        assert_eq!(sea(ward), sea(ruin), "the ward over the sea");
        assert_ne!(
            ground(ward),
            ground(ruin),
            "the ward walked up to from the Plains of Ruin"
        );
    }
}
