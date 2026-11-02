use ac_nav::routing::Planned;

use crate::{pathfinder, player};

/// The client answering what the steering asks of the world.
///
/// The steering itself is in `ac-nav` and knows nothing of packets,
/// physics or landblocks; this is the half that does. Eight questions,
/// which in a test are answered with a few rectangles and here with
/// the character's own collision and the region planner.
pub struct Standing<'a> {
    pub player: &'a mut player::Player,
    pub assets: &'a ac_scene::Assets,
    pub wide: &'a mut pathfinder::Pathfinder,
}

impl ac_nav::Ground for Standing<'_> {
    fn at(&self) -> glam::Vec3 {
        self.player.world_position()
    }

    fn cell(&self) -> u32 {
        self.player.cell
    }

    fn block(&self) -> u32 {
        self.player.landblock()
    }

    fn line_blocked(&mut self, block: u32, from: glam::Vec3, to: glam::Vec3) -> bool {
        self.player.line_blocked(self.assets, block, from, to)
    }

    fn line_drops(&mut self, block: u32, from: glam::Vec3, to: glam::Vec3) -> bool {
        self.player.line_drops(self.assets, block, from, to)
    }

    fn find_path(
        &mut self,
        block: u32,
        from: glam::Vec3,
        to: glam::Vec3,
        goal_cell: u32,
        within: f32,
    ) -> Option<Vec<glam::Vec3>> {
        // From where the body stands, in its own cell; from anywhere else, located by the cells.
        let mine = from.distance(self.player.world_position()) < 1e-3;
        let from = ac_world::Place {
            at: from,
            cell: if mine {
                self.player.cell
            } else {
                block & 0xFFFF_0000
            },
        };
        let to = ac_world::Place {
            at: to,
            cell: goal_cell,
        };
        match self.wide.plan(from, to, within, self.player.capsule()) {
            Planned::Route(r) => Some(r.points()),
            Planned::NoWay(why) => {
                tracing::debug!("nav: no way to {:?}: {why:?}", to.at);
                None
            }
            Planned::Building(_) => None,
        }
    }

    fn ask_wide(
        &mut self,
        from: glam::Vec3,
        to: glam::Vec3,
        _block: u32,
        goal_cell: u32,
        within: f32,
    ) {
        // The steering asks from where the body stands, in its own cell (and so its block).
        self.wide.ask(
            ac_world::Place {
                at: from,
                cell: self.player.cell,
            },
            ac_world::Place {
                at: to,
                cell: goal_cell,
            },
            within,
            self.player.capsule(),
            std::time::Instant::now(),
        );
    }

    fn take_wide(&mut self, from: glam::Vec3, to: glam::Vec3) -> Option<Vec<glam::Vec3>> {
        self.wide.take(from, to)
    }

    fn is_planning(&self) -> bool {
        self.wide.is_building()
    }
}
