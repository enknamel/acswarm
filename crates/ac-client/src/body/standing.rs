use crate::{pathfinder, player};

/// The client answering what the steering asks of the world.
///
/// The steering itself is in `ac-nav` and knows nothing of packets,
/// physics or landblocks; this is the half that does. Seven questions,
/// which in a test are answered with a few rectangles and here with
/// the character's own collision and the planner on its thread.
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
    ) -> Option<Vec<glam::Vec3>> {
        self.player
            .find_path(self.assets, block, from, to, goal_cell)
    }

    fn ask_wide(
        &mut self,
        from: glam::Vec3,
        to: glam::Vec3,
        block: u32,
        outdoors: bool,
        exact_to: bool,
    ) {
        self.wide.ask(
            from,
            to,
            self.player.capsule(),
            block,
            pathfinder::Ends { outdoors, exact_to },
            std::time::Instant::now(),
        );
    }

    fn take_wide(&mut self, from: glam::Vec3, to: glam::Vec3) -> Option<Vec<glam::Vec3>> {
        self.wide.take(from, to)
    }
}
