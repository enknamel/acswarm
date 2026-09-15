//! Vocabulary every system shares: what came of trying a thing, and how long before asking again.
//! [`did`]: outcomes and the one retry policy (`Patience`); [`recent`]: fixed windows, never doubling.
//! [`weenie_errors`]: server refusal codes as text; [`pack`]: which stacks to pour together.
//! [`room`]: room pack by pack and how to take a loose thing; [`refusals`]: server words, one table.
//! Plain data and arithmetic with no server, drawing or character state, so dependents test without.

pub mod did;
pub mod pack;
pub mod recent;
pub mod refusals;
pub mod room;
pub mod weenie_errors;
