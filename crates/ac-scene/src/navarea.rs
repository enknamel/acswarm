//! The region the one planner plans on, an [`Area`]: a dungeon whole, or the blocks round a walk
//! ([`blocks_for`], at most [`MAX_SIDE`] a side) with their buildings, as one collision world with
//! one terrain over all, so a route leaves its block and comes back (a gate past a city wall).
//! Assembling nine blocks costs tens of ms, a long search a hundred more: built off the frame and
//! shared by every session (`ac-client` `pathfinder.rs`), its graph filling chunk by chunk.

use std::collections::HashMap;

use glam::{Vec2, Vec3};

use crate::collision::{Capsule, CollisionWorld};
use crate::nav::{Ground, NavGraph, INDOOR_SPACING, OUTDOOR_SPACING};
use crate::terrain::TerrainMesh;
use crate::worldroute::{is_water_block, water_types};
use crate::{lbid, Assets, Result, BLOCK_SIZE};

/// Most blocks on a side of an area. Nine blocks is about 600 m across,
/// which covers any city and more than one travel leg.
pub const MAX_SIDE: u32 = 3;

/// The landblock a world position falls in.
pub fn block_at(p: Vec3) -> u32 {
    let x = (p.x / BLOCK_SIZE).floor().clamp(0.0, 254.0) as u32;
    let y = (p.y / BLOCK_SIZE).floor().clamp(0.0, 254.0) as u32;
    lbid::from_xy(x, y)
}

/// The blocks an area should cover to hold a walk from `from` to `to`:
/// the bounding box of the two, grown by one block, and clamped to
/// [`MAX_SIDE`] blocks a side around `from`.
pub fn blocks_for(from: Vec3, to: Vec3) -> Vec<u32> {
    let (a, b) = (block_at(from), block_at(to));
    let (ax, ay) = (lbid::block_x(a) as i32, lbid::block_y(a) as i32);
    let (bx, by) = (lbid::block_x(b) as i32, lbid::block_y(b) as i32);
    let half = (MAX_SIDE as i32 - 1) / 2;
    let span = |a: i32, b: i32| {
        let (lo, hi) = (a.min(b) - 1, a.max(b) + 1);
        // Keep the character's own block in the middle when the goal is
        // far enough that the box would be bigger than we allow.
        let (lo, hi) = if hi - lo + 1 > MAX_SIDE as i32 {
            (a - half, a + half)
        } else {
            (lo, hi)
        };
        (lo.max(0), hi.min(254))
    };
    let (x0, x1) = span(ax, bx);
    let (y0, y1) = span(ay, by);
    let mut out = Vec::new();
    for x in x0..=x1 {
        for y in y0..=y1 {
            out.push(lbid::from_xy(x as u32, y as u32));
        }
    }
    out
}

/// Several landblocks assembled into one thing paths can be planned on.
pub struct Area {
    /// The blocks covered, sorted.
    pub blocks: Vec<u32>,
    /// Their static geometry, merged.
    pub collision: CollisionWorld,
    /// Each block's terrain, by block id: plain data, so an area can be shared across threads.
    terrain: HashMap<u32, TerrainMesh>,
    /// The middle of every opening between interior cells, in world
    /// space: the graph puts a node in each (see `nav::Ground`).
    pub doorways: Vec<Vec3>,
    pub nav: NavGraph,
    /// A dungeon area: no terrain under it, fine lattice.
    pub dungeon: bool,
    /// Which of the region's terrain types are water, by index: a block water at every vertex
    /// is the invisible wall in the sea ([`is_water_block`]). Empty when the region is unread.
    water_types: Vec<bool>,
}

impl Area {
    /// Assemble `blocks`. A dungeon is assembled alone whatever else was
    /// asked for: the blocks around it on the lattice are unrelated
    /// pieces of the surface, and there is no terrain under it to walk.
    pub fn build(assets: &Assets, blocks: &[u32], cap: &Capsule, centre: u32) -> Result<Area> {
        let centre = centre & 0xFFFF_0000;
        let centre_scene = crate::landblock::load(assets, centre)?;
        let dungeon = centre_scene.is_dungeon;
        let wanted: Vec<u32> = if dungeon {
            vec![centre]
        } else {
            let mut v: Vec<u32> = blocks.iter().map(|b| b & 0xFFFF_0000).collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        let mut terrain: HashMap<u32, TerrainMesh> = HashMap::new();
        let mut indoors = false;
        let mut doorways: Vec<Vec3> = Vec::new();
        let mut collision = CollisionWorld::default();
        for &id in &wanted {
            let scene = if id == centre {
                centre_scene.clone()
            } else {
                match crate::landblock::load(assets, id) {
                    Ok(s) => s,
                    // A block off the edge of the world or unreadable:
                    // the area is simply smaller.
                    Err(_) => continue,
                }
            };
            // Never mix a dungeon's geometry into the surface: it sits
            // at the same coordinates as the ground above it.
            if scene.is_dungeon && id != centre {
                continue;
            }
            if let Ok(w) = CollisionWorld::from_scene(assets, &scene) {
                collision.absorb(&w);
            }
            indoors |= !scene.cells.is_empty();
            // Every opening the cell data names, so the graph can put a
            // node in each rather than hope the lattice lands on one.
            doorways.extend(scene.cells.iter().flat_map(|c| c.doorways.iter().copied()));
            terrain.insert(id, scene.terrain.clone());
        }
        let blocks: Vec<u32> = {
            let mut v: Vec<u32> = terrain.keys().copied().collect();
            v.sort_unstable();
            v
        };
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for &id in &blocks {
            let o = lbid::world_origin(id);
            lo = lo.min(Vec2::new(o.x, o.y));
            hi = hi.max(Vec2::new(o.x + BLOCK_SIZE, o.y + BLOCK_SIZE));
        }
        if dungeon {
            if let Some((clo, chi)) = collision.bounds() {
                lo = lo.min(Vec2::new(clo.x, clo.y));
                hi = hi.max(Vec2::new(chi.x, chi.y));
            }
        }
        // The fine lattice wherever there are interiors to walk, the coarse one over open country:
        // a 4 m lattice puts about one node in a room, so a storey has a node or two, unjoined.
        let spacing = if dungeon || indoors {
            INDOOR_SPACING
        } else {
            OUTDOOR_SPACING
        };
        let nav = NavGraph::new(lo, hi, spacing, cap);
        let water_types = assets.region().map(|r| water_types(&r)).unwrap_or_default();
        Ok(Area {
            blocks,
            collision,
            terrain,
            doorways,
            nav,
            dungeon,
            water_types,
        })
    }

    /// Open sea at a world `(x, y)`: its landblock is water at every vertex ([`is_water_block`]).
    pub fn is_sea(&self, x: f32, y: f32) -> bool {
        !self.dungeon && seas(&self.terrain, &self.water_types)(x, y)
    }

    /// Run something against the area's ground: its merged collision,
    /// the terrain of whichever block a point falls in, and where the
    /// sea is. The graph is handed over at the same time because every
    /// caller needs both and they borrow different fields.
    pub fn with_ground<R>(&mut self, f: impl FnOnce(&Ground, &mut NavGraph) -> R) -> R {
        let Area {
            collision,
            terrain,
            doorways,
            nav,
            dungeon,
            water_types,
            ..
        } = self;
        let (height, sea) = (heights(terrain), seas(terrain, water_types));
        let ground = Ground {
            collision,
            terrain: (!*dungeon).then_some(&height),
            sea: (!*dungeon).then_some(&sea),
            doorways,
        };
        f(&ground, nav)
    }

    /// [`with_ground`](Self::with_ground) with the graph only read, for an area shared.
    pub fn read_ground<R>(&self, f: impl FnOnce(&Ground, &NavGraph) -> R) -> R {
        let (height, sea) = (
            heights(&self.terrain),
            seas(&self.terrain, &self.water_types),
        );
        let ground = Ground {
            collision: &self.collision,
            terrain: (!self.dungeon).then_some(&height),
            sea: (!self.dungeon).then_some(&sea),
            doorways: &self.doorways,
        };
        f(&ground, &self.nav)
    }

    /// The capsule the graph was built for.
    pub fn capsule(&self) -> Capsule {
        self.nav.capsule
    }

    /// Height of the ground at a world `(x, y)`, from whichever block it
    /// falls in. `None` in a dungeon, and outside the area.
    pub fn terrain_at(&self, x: f32, y: f32) -> Option<f32> {
        if self.dungeon {
            return None;
        }
        heights(&self.terrain)(x, y)
    }

    /// Build every chunk of the graph now. For tools and tests: paths
    /// build only what they cross.
    pub fn build_everything(&mut self) {
        self.with_ground(|ground, nav| nav.build_all(ground));
    }
}

/// The terrain height at a world `(x, y)`, from whichever block of `terrain` it falls in.
fn heights(terrain: &HashMap<u32, TerrainMesh>) -> impl Fn(f32, f32) -> Option<f32> + '_ {
    move |x, y| {
        let id = block_at(Vec3::new(x, y, 0.0));
        let o = lbid::world_origin(id);
        terrain
            .get(&id)?
            .height_at(Vec3::new(x - o.x, y - o.y, 0.0))
    }
}

/// Whether a world `(x, y)` is open sea: its block's terrain is water at every vertex
/// ([`is_water_block`]); never when the water types are unknown.
fn seas<'a>(
    terrain: &'a HashMap<u32, TerrainMesh>,
    water_types: &'a [bool],
) -> impl Fn(f32, f32) -> bool + 'a {
    move |x, y| {
        let id = block_at(Vec3::new(x, y, 0.0));
        terrain.get(&id).is_some_and(|t| {
            let types = t.vertices.iter().map(|v| v.terrain_type);
            is_water_block(types, water_types)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_area_covers_the_walk_and_stays_within_its_size() {
        let o = |id: u32| lbid::world_origin(id) + Vec3::new(96.0, 96.0, 0.0);
        // Neighbours: the box around both, grown by one, is 3x3 already.
        let here = lbid::from_xy(0x33, 0xD9);
        let next = lbid::from_xy(0x34, 0xD9);
        let blocks = blocks_for(o(here), o(next));
        assert_eq!(blocks.len(), (MAX_SIDE * MAX_SIDE) as usize);
        assert!(blocks.contains(&here) && blocks.contains(&next));
        // A distant goal: the box is centred on where we are instead.
        let far = lbid::from_xy(0x50, 0xD9);
        let blocks = blocks_for(o(here), o(far));
        assert_eq!(blocks.len(), (MAX_SIDE * MAX_SIDE) as usize);
        assert!(blocks.contains(&here));
        assert!(!blocks.contains(&far));
        // The world's corner: clamped, so fewer blocks.
        let corner = lbid::from_xy(0, 0);
        let blocks = blocks_for(o(corner), o(corner));
        assert_eq!(blocks.len(), 4);
        assert!(blocks.contains(&corner));
    }

    #[test]
    fn an_area_is_shared_across_threads() {
        fn is_shared<T: Send + Sync>() {}
        is_shared::<Area>();
    }

    #[test]
    fn positions_map_to_the_block_they_fall_in() {
        let id = lbid::from_xy(0x33, 0xD9);
        let o = lbid::world_origin(id);
        assert_eq!(block_at(o), id);
        assert_eq!(block_at(o + Vec3::new(191.0, 191.0, 0.0)), id);
        assert_eq!(
            block_at(o + Vec3::new(192.0, 0.0, 0.0)),
            lbid::from_xy(0x34, 0xD9)
        );
        // Off the map: clamped rather than wrapped into another block.
        assert_eq!(block_at(Vec3::new(-10.0, -10.0, 0.0)), lbid::from_xy(0, 0));
    }
}
