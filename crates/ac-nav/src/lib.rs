//! Getting a character from where it is to where it wants to be. Entry: [`Steering::steer`]
//! (route within a landblock), [`routing::plan`] (a whole walk planned first), [`how_to_get_there`]
//! (walk or journey), [`explore::next_step`] (next dungeon doorway), [`obstacles::detour`] (round
//! server objects). A goal judged unreachable is answered [`Aim::NoWay`], never walked at; the
//! caller picks another. No socket, no character: [`Ground`] is asked; geometry lives in `ac-scene`.

pub mod explore;
pub mod means;
pub mod obstacles;
pub mod routing;
pub mod steering;

pub use means::{how_to_get_there, Means, WALKABLE};
pub use obstacles::{Clutter, Cylinder};
pub use steering::{Aim, Ground, Steering};
