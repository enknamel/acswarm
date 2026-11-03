//! Getting a character from where it is to where it wants to be. Entry: [`routing::plan`] (a
//! whole walk planned first), [`Steering::steer`] (the follower of that route),
//! [`obstacles::detour`] (round server objects), [`explore::next_step`] (next dungeon doorway).
//! A goal judged unreachable is answered [`Aim::NoWay`], never walked at; the caller picks
//! another. No socket, no character: [`Ground`] is asked; geometry lives in `ac-scene`.

pub mod explore;
pub mod obstacles;
pub mod routing;
pub mod steering;

pub use obstacles::{Clutter, Cylinder};
pub use steering::{Aim, Ground, Steering};
