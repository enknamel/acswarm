//! What the character did lately, kept for telemetry to read (`ac_plugin::telemetry`): the walk
//! as the steering and the body left it this frame, and the blows traded since the session began.

use std::time::Instant;

use glam::Vec3;

/// One frame of a walk, as the steering and the body left it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WalkFrame {
    pub at: Instant,
    pub goal: Vec3,
    /// The cell or landblock the goal was said to lie in, which decides what the steering plans on.
    pub goal_cell: u32,
    /// Who set the goal: "server walk", "journey" or "follow".
    pub source: &'static str,
    /// Where the steering aimed; `None` when it found no way (`ac_nav::Aim::NoWay`).
    pub aim: Option<Vec3>,
    /// The leg turned aside for a server-placed object on it (`ac_nav::obstacles::detour`).
    pub detoured: bool,
    /// The route's waypoint being walked at and how many it has; `None` on the straight line.
    pub route: Option<(usize, usize)>,
    /// Pressed against geometry and going nowhere ([`crate::player::Player::wedged`]).
    pub wedged: bool,
    /// Walking back the way it came, out of somewhere the graph finds no path from.
    pub retracing: bool,
}

/// Blows traded since the session began, counted off the server's attack notices (0x01B1-0x01B4).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Blows {
    pub dealt: u32,
    pub dealt_points: u64,
    pub taken: u32,
    pub taken_points: u64,
    /// Our attacks the other side evaded.
    pub missed: u32,
    /// Attacks on us we evaded.
    pub evaded: u32,
}
