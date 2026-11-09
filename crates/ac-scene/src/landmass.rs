//! Dereth's landmasses: the [`WorldGrid`] lattice's walkable vertices (land and fresh water, as
//! [`worldroute`](crate::worldroute) walks them) labelled by the piece of land they join, so a
//! trip planner never takes a walk from the mainland to an island the sea cuts off.

use ac_formats::region::Region;
use glam::Vec2;

use crate::worldgrid::{WorldGrid, SIDE};
use crate::worldroute::{water_kind, Water};

/// How far (vertices) a spot in the sea looks for the land it stands off: a portal on a jetty.
const SNAP_RADIUS: i32 = 3;

/// A label per lattice vertex, 0 for the sea and blocks the archive lacks.
#[derive(Clone, Debug)]
pub struct Landmasses {
    labels: Vec<u32>,
}

impl Landmasses {
    /// Label every walkable vertex by flood fill, 4-connected: two pieces meeting only at a
    /// corner are apart, the side `worldroute`'s diagonal rule takes too.
    pub fn of_world(grid: &WorldGrid, region: &Region) -> Landmasses {
        let water: Vec<Water> = (0..32u16).map(|t| water_kind(region, t)).collect();
        let walkable = |gx: usize, gy: usize| {
            grid.has_block((gx / 8).min(254), (gy / 8).min(254))
                && !matches!(
                    water.get(grid.terrain_type(gx, gy) as usize),
                    Some(Water::Sea) | None
                )
        };
        let mut labels = vec![0u32; SIDE * SIDE];
        let mut next = 0;
        let mut stack = Vec::new();
        for start in 0..SIDE * SIDE {
            if labels[start] != 0 || !walkable(start / SIDE, start % SIDE) {
                continue;
            }
            next += 1;
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
                    if labels[j] == 0 && walkable(nx, ny) {
                        labels[j] = next;
                        stack.push(j);
                    }
                }
            }
        }
        Landmasses { labels }
    }

    /// The landmass under world xy `at`, or the nearest within 3 vertices; `None` at sea.
    pub fn at(&self, at: Vec2) -> Option<u32> {
        let (gx, gy) = WorldGrid::nearest_vertex(at);
        let (gx, gy) = (gx as i32, gy as i32);
        (0..=SNAP_RADIUS).find_map(|r| {
            let ring = (-r..=r).flat_map(|dx| (-r..=r).map(move |dy| (dx, dy)));
            ring.filter(|(dx, dy)| dx.abs().max(dy.abs()) == r)
                .filter_map(|(dx, dy)| {
                    let (x, y) = (gx + dx, gy + dy);
                    let inside = (0..SIDE as i32).contains(&x) && (0..SIDE as i32).contains(&y);
                    inside.then(|| self.labels[x as usize * SIDE + y as usize])
                })
                .find(|l| *l != 0)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs AC_DATA_DIR"]
    fn ithaenc_is_an_island_and_holtburg_and_arwic_share_the_mainland() {
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
        let holtburg = land.at(map(42.1, 33.6));
        let arwic = land.at(map(33.6, 56.8));
        let nanto = in_block(0xE63E_0000, 100.0, 100.0);
        // The Catacombs of Ithaenc's portal, on the island; the invitation's exit beside it.
        let ithaenc = in_block(0xF418_0000, 100.0, 100.0);
        let invitation = in_block(0xF618_0000, 40.0, 89.0);
        assert!(holtburg.is_some() && ithaenc.is_some());
        assert_eq!(holtburg, arwic, "the mainland in two");
        assert_eq!(holtburg, nanto, "Nanto off the mainland");
        assert_ne!(nanto, ithaenc, "Ithaenc walked to from Nanto");
        assert_eq!(invitation, ithaenc, "the invitation lands off the island");
    }
}
