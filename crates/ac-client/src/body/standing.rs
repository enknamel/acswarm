use ac_nav::routing::Planned;
use ac_world::Place;
use glam::Vec3;

use crate::{pathfinder, player};

/// The client answering what the steering asks of the world.
///
/// The steering itself is in `ac-nav` and knows nothing of packets,
/// physics or landblocks; this is the half that does. Eight questions,
/// which in a test are answered with a few rectangles and here with
/// the character's own collision and the region planner.
pub struct Standing<'a> {
    pub player: &'a mut player::Player,
    pub pathfinder: &'a mut pathfinder::Pathfinder,
}

/// Where a walk test from `at` starts: the body's own place where it stands, else `at` in its
/// block, located by the cells' BSPs.
pub(crate) fn walk_start(pl: &player::Player, at: Vec3) -> Place {
    let mine = at.distance(pl.world_position()) < 1e-3;
    Place {
        at,
        cell: if mine { pl.cell } else { pl.cell & 0xFFFF_0000 },
    }
}

impl ac_nav::Ground for Standing<'_> {
    fn at(&self) -> Vec3 {
        self.player.world_position()
    }

    fn cell(&self) -> u32 {
        self.player.cell
    }

    fn is_airborne(&self) -> bool {
        self.player.is_airborne()
    }

    fn ask(&mut self, goal: Place, within: f32, now: std::time::Instant) {
        let me = Place {
            at: self.player.world_position(),
            cell: self.player.cell,
        };
        let cap = self.player.capsule();
        self.pathfinder.ask(me, goal, within, cap, now);
    }

    fn answer(&mut self, goal: Place) -> Option<(Place, Planned)> {
        self.pathfinder.take(self.player.world_position(), goal.at)
    }

    fn can_walk(&mut self, from: Vec3, to: Vec3) -> bool {
        let start = walk_start(self.player, from);
        self.pathfinder.can_walk(start, to, self.player.capsule())
    }
}
