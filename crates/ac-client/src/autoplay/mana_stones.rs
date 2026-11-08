//! Worn things kept charged with mana stones, and the stones charged from mana junk: an empty
//! stone used on an item takes its mana and destroys it; a charged one used on the character
//! shares its mana round what is worn and goes empty, or breaks (ManaStone.cs:64-150, 235-250).

use std::time::{Duration, Instant};

use ac_loot::profile::LootAction;
use serde::{Deserialize, Serialize};

use crate::Client;

/// How often the worn things' mana is asked for again (guess: a worn item drains over hours).
const LOOK_EVERY: Duration = Duration::from_secs(60);
/// How long a stone's use waits for its answer before the next (ManaStone.cs:232 sends UseDone).
const ANSWER_WAIT: Duration = Duration::from_secs(5);

/// How mana stones are kept up.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mana {
    /// Keep what is worn charged, and stones charged from what the loot rules drain.
    pub on: bool,
    /// Charged stones carried at all times, filled from mana junk.
    pub backup: u32,
    /// Any one worn thing under this share of its mana is recharged.
    pub recharge_below: f32,
}

impl Default for Mana {
    fn default() -> Self {
        Mana {
            on: true,
            backup: 5,
            recharge_below: 0.5,
        }
    }
}

/// What the mana stone rules remember between ticks.
#[derive(Debug, Default)]
pub struct State {
    /// When the worn things were last asked about.
    looked: Option<Instant>,
    /// The stone last used and when, until the server answers and it is looked at again.
    sent: Option<(u32, Instant)>,
}

/// A carried stone as the rules see it, once appraised.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Stone {
    Charged(u32),
    Empty(u32),
}

impl Client {
    /// Recharge what is worn when any one thing runs low, and fill empty stones from mana junk
    /// between fights until [`Mana::backup`] are charged. One use at a time, each answered.
    pub(crate) fn autoplay_mana_stones(&mut self, now: Instant) {
        let cfg = self.autoplay.config.mana.clone();
        if !cfg.on || self.autoplay.cast_in_flight(now) {
            return;
        }
        if let Some((stone, at)) = self.autoplay.mana_stones.sent {
            if now.duration_since(at) < ANSWER_WAIT && self.world.objects.contains_key(&stone) {
                return;
            }
            self.autoplay.mana_stones.sent = None;
        }
        self.look_at_mana(now);
        let stones = self.stones();
        let charged: Vec<u32> = stones
            .iter()
            .filter_map(|s| match s {
                Stone::Charged(g) => Some(*g),
                Stone::Empty(_) => None,
            })
            .collect();
        if let (Some(&stone), Some(low)) = (charged.first(), self.low_on_mana(cfg.recharge_below)) {
            let me = self.world.player_guid.unwrap_or_default();
            self.use_stone(
                stone,
                me,
                format!("charging what is worn ({low} is low)"),
                now,
            );
            return;
        }
        if charged.len() as u32 >= cfg.backup || self.in_a_fight() {
            return;
        }
        let empty = stones.iter().find_map(|s| match s {
            Stone::Empty(g) => Some(*g),
            Stone::Charged(_) => None,
        });
        if let (Some(stone), Some((junk, name))) = (empty, self.mana_junk()) {
            self.use_stone(
                stone,
                junk,
                format!("draining {name} into a mana stone"),
                now,
            );
        }
    }

    /// Use `stone` on `target`, wait for its answer, and look at what it changed afterwards: the
    /// server keeps a stone's and a worn thing's new mana to itself until asked.
    fn use_stone(&mut self, stone: u32, target: u32, why: String, now: Instant) {
        if !self.use_on(stone, target) {
            return;
        }
        self.autoplay.mana_stones.sent = Some((stone, now));
        self.autoplay.cast_sent = Some(now);
        self.appraisals.remove(&stone);
        if Some(target) == self.world.player_guid {
            self.autoplay.mana_stones.looked = None;
        }
        self.autoplay.note(why, now);
    }

    /// Ask about the carried stones not yet appraised, and every [`LOOK_EVERY`] about the worn
    /// things, whose mana drains as they are worn.
    fn look_at_mana(&mut self, now: Instant) {
        let stones: Vec<u32> = self
            .world
            .inventory()
            .filter(|o| o.item_type & ac_world::item_type::MANA_STONE != 0)
            .map(|o| o.guid)
            .collect();
        self.appraise_many(stones);
        let due = self
            .autoplay
            .mana_stones
            .looked
            .is_none_or(|t| now.duration_since(t) >= LOOK_EVERY);
        if !due {
            return;
        }
        self.autoplay.mana_stones.looked = Some(now);
        let worn: Vec<u32> = self
            .world
            .wielded()
            .filter(|o| {
                self.stats_of(o.guid)
                    .is_none_or(|s| !s.appraised || s.max_mana > 0)
            })
            .map(|o| o.guid)
            .collect();
        self.appraise_again(worn);
    }

    /// The carried stones whose appraisal is in: charged with mana, or empty.
    fn stones(&self) -> Vec<Stone> {
        self.world
            .inventory()
            .filter(|o| o.item_type & ac_world::item_type::MANA_STONE != 0)
            .filter_map(|o| {
                let s = self.stats_of(o.guid).filter(|s| s.appraised)?;
                Some(if s.mana > 0 {
                    Stone::Charged(o.guid)
                } else {
                    Stone::Empty(o.guid)
                })
            })
            .collect()
    }

    /// The name of a worn thing under `below` of its mana, if any is.
    fn low_on_mana(&self, below: f32) -> Option<String> {
        self.world.wielded().find_map(|o| {
            let s = self
                .stats_of(o.guid)
                .filter(|s| s.appraised && s.max_mana > 0)?;
            ((s.mana as f32) < below * s.max_mana as f32).then(|| o.name.clone())
        })
    }

    /// A carried thing the loot rules tagged Drain that holds mana: the stone's filling.
    fn mana_junk(&self) -> Option<(u32, String)> {
        self.world
            .inventory()
            .filter(|o| o.item_type & ac_world::item_type::MANA_STONE == 0)
            .find_map(|o| {
                let s = self
                    .stats_of(o.guid)
                    .filter(|s| s.appraised && s.mana > 0)?;
                (self.autoplay.ledger.of(&s) == Some(LootAction::Drain))
                    .then(|| (o.guid, o.name.clone()))
            })
    }
}

#[cfg(test)]
mod tests;
