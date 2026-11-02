//! Per-landblock collision, shared by every character in the process: a block's static geometry
//! as a [`CollisionWorld`] (about 1.5 MB for a town block) is the same whoever walks it.
//! [`Assets`] owns one [`BlockCache`]; characters keep `Rc`s to what they stand on and let go of
//! blocks they have left, so the cache's bound (the most recent [`KEEP`] blocks) is the process's.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;

use crate::collision::CollisionWorld;
use crate::{Assets, Result};
use glam::Vec3;

/// How many blocks' collision the cache keeps; the least recently
/// asked for goes first. Characters spread over more than this many
/// blocks rebuild the ones that fell out, at ~0.5 s each.
pub const KEEP: usize = 48;

/// One block's static geometry.
pub struct BlockCollision {
    /// The landblock id (`xxyy0000`).
    pub block: u32,
    pub world: CollisionWorld,
    /// A dungeon block: no terrain to walk on.
    pub dungeon: bool,
    /// The middle of every opening between this block's interior cells,
    /// at the threshold, in world space. The navigation graph stands a
    /// node in each: see `nav::Ground::doorways`.
    pub doorways: Vec<Vec3>,
}

impl std::ops::Deref for BlockCollision {
    type Target = CollisionWorld;
    fn deref(&self) -> &CollisionWorld {
        &self.world
    }
}

/// The process's collision worlds, most recently used [`KEEP`] blocks.
#[derive(Default)]
pub struct BlockCache {
    blocks: RefCell<HashMap<u32, Rc<BlockCollision>>>,
    order: RefCell<VecDeque<u32>>,
}

impl BlockCache {
    /// The collision of landblock `block` (low 16 bits ignored), built
    /// from the assembled scene on first use.
    pub fn collision(&self, assets: &Assets, block: u32) -> Result<Rc<BlockCollision>> {
        let block = block & 0xFFFF_0000;
        if let Some(b) = self.blocks.borrow().get(&block) {
            self.touch(block);
            return Ok(b.clone());
        }
        let scene = assets.landblock(block)?;
        let world = CollisionWorld::from_scene(assets, &scene)?;
        let b = Rc::new(BlockCollision {
            block,
            world,
            dungeon: scene.is_dungeon,
            doorways: scene
                .cells
                .iter()
                .flat_map(|c| c.doorways.iter().copied())
                .collect(),
        });
        let mut blocks = self.blocks.borrow_mut();
        let mut order = self.order.borrow_mut();
        while order.len() >= KEEP {
            if let Some(old) = order.pop_front() {
                blocks.remove(&old);
            }
        }
        blocks.insert(block, b.clone());
        order.push_back(block);
        Ok(b)
    }

    /// The collision of `block` only if it is already built.
    pub fn cached(&self, block: u32) -> Option<Rc<BlockCollision>> {
        self.blocks.borrow().get(&(block & 0xFFFF_0000)).cloned()
    }

    /// How many blocks are held.
    pub fn len(&self) -> usize {
        self.blocks.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.borrow().is_empty()
    }

    fn touch(&self, block: u32) {
        let mut order = self.order.borrow_mut();
        if order.back() == Some(&block) {
            return;
        }
        if let Some(i) = order.iter().position(|&b| b == block) {
            order.remove(i);
            order.push_back(block);
        }
    }
}
