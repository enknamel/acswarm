//! The vocabulary a character's systems share.
//!
//! Vendoring, looting, fighting and travelling are separate problems
//! with separate rules, but they all answer the same two questions over
//! and over: *what came of trying that*, and *how long before asking
//! again*. Answering them differently in each place is how a client
//! ends up with sixty-two flags meaning almost the same thing, and how
//! "keeps trying for ever" becomes the default rather than a bug
//! somebody wrote.
//!
//! So they are answered once, here, below every system that needs them:
//!
//! - [`did`] -- what happened ([`did::Did`]), why not ([`did::Because`]),
//!   and the one retry policy that reads them ([`did::Patience`]).
//! - [`recent`] -- when each thing was last seen, forgotten after a fixed
//!   window that, unlike [`did::Patience`], never doubles.
//! - [`weenie_errors`] -- what the server's own refusal codes mean, so a
//!   reason can be told apart by its number rather than by its English.
//! - [`pack`] -- the arithmetic of a pack full of stacks: which two to
//!   pour together, and what it is worth.
//! - [`room`] -- where the room is, pack by pack, and how a thing lying
//!   loose is to be taken into it.
//! - [`refusals`] -- what the server says in words when it will not do
//!   a thing, read once against one table, and what to do about each
//!   kind, decided once.
//!
//! Nothing here talks to a server, draws anything, or knows what a
//! character is doing. It is all plain data and arithmetic, which is
//! what makes the systems built on it testable without either.

pub mod did;
pub mod pack;
pub mod recent;
pub mod refusals;
pub mod room;
pub mod weenie_errors;
