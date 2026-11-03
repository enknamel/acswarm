//! Quests the player asked for, worked before hunting for XP: each pursued quest's step for this
//! character, worked out with the board; the town run calls on givers, `grow_hunt` hunts sources.

use ac_world::quests::Row;
use ac_world::unlocks::Unlock;
use ac_world::Place;

/// The step a pursued quest is at for this character.
#[derive(Clone, Debug, PartialEq)]
pub enum PursuitStep {
    /// Talk to `row`'s giver at `at`, who starts it.
    TakeUp { row: &'static Row, at: Place },
    /// Earn `flag` from an unlock NPC first: a way on asks it.
    EarnFlag { flag: String, from: &'static Unlock },
    /// Hunt `creature` in `landblock` (`place` in words), `more` to go.
    Hunt {
        landblock: u32,
        creature: String,
        place: String,
        more: u32,
    },
    /// Hand in to `row`'s giver at `at`.
    HandIn { row: &'static Row, at: Place },
    /// Done; may be done again at this Unix second.
    Waiting { until: u64 },
    /// Done since asked for and not to be done again.
    Finished,
    /// Cannot go on, in words ("nobody known gives the eaterpit flag").
    Stuck { why: String },
}

impl PursuitStep {
    /// Whether autoplay can act on it now.
    pub fn can_progress(&self) -> bool {
        matches!(
            self,
            PursuitStep::TakeUp { .. }
                | PursuitStep::EarnFlag { .. }
                | PursuitStep::Hunt { .. }
                | PursuitStep::HandIn { .. }
        )
    }
}

/// One pursued quest as the Quests window shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct PursuitLine {
    pub id: String,
    pub title: String,
    pub step: PursuitStep,
    /// The one autoplay works now: the first that can progress.
    pub working: bool,
    /// The pursued quest this one is worked first for (it stamps a flag that one needs).
    pub via: Option<String>,
}
