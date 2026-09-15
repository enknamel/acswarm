//! Loot: what an item is worth having for, and emptying the corpse it lies on.
//!
//! [`items`] is the vocabulary and search language, [`profile`] the player's rules, [`run`] empties
//! one corpse ([`Run::step`]), [`ledger`] remembers what each item was taken for, [`sale`] decides
//! what goes over a counter, [`weapons`] picks what to fight with. No socket; callers pass the time.

pub mod corpse;
pub mod items;
pub mod ledger;
pub mod profile;
pub mod run;
pub mod sale;
pub mod weapons;

pub use corpse::{Lying, Open, Verdict, REACH};
pub use items::{ItemStats, NumKey, Op, Query, Term, Tier};
pub use ledger::{Ledger, Took};
pub use profile::{Library, LootAction, Profile, Rule, Verdict as RuleVerdict};
pub use run::{Act, Next, Run, Tally};
pub use sale::{fate, offer_to_vendor, Fate};
pub use weapons::{Stance, Wielder};
