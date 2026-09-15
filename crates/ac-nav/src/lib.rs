//! Getting a character from where it is to where it wants to be.
//! Entry: [`Steering::steer`] (route within a landblock), [`how_to_get_there`] (walk or journey),
//! [`explore::next_step`] (next dungeon doorway), [`obstacles::detour`] (round server objects).
//! A goal judged unreachable is answered [`Aim::NoWay`], never walked at; the caller picks another.
//! No socket, no character: the world is asked through [`Ground`]; geometry lives in `ac-scene`.

pub mod explore;
pub mod means;
pub mod obstacles;
pub mod steering;

pub use means::{how_to_get_there, Means, WALKABLE};
pub use obstacles::{Clutter, Cylinder};
pub use steering::{Aim, Ground, Steering};
