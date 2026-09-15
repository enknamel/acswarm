//! Autovendoring: the town run at a counter, decided from a [`Snapshot`] with no server.
//! [`Run::step`] answers at most one [`Act`] per snapshot; it opens no socket, moves nothing and
//! keeps no clock. [`errand::plan`] works out a whole trip before it is walked.
//! ac-client builds the snapshot and carries out the act. Fixed order: compress the pack, sell
//! until low on room, turn the takings into trade notes, round again, then buy.

pub mod counter;
pub mod errand;
pub mod run;

pub use counter::{Counter, Item, Keep, Rules, Snapshot, Want, Ware};
pub use run::{Act, Next, Phase, Run};
