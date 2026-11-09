//! Worn things kept charged with mana stones, and the stones charged from mana junk: an empty
//! stone used on an item takes its mana and destroys it; a charged one used on the character
//! shares its mana round what is worn and goes empty, or breaks (ManaStone.cs:64-150, 235-250).

use std::time::{Duration, Instant};

use ac_loot::profile::LootAction;
use serde::{Deserialize, Serialize};

use crate::Client;

/// PropertyInt ItemCurMana: a stone's charge, absent when it is empty (ManaStone.cs:64).
const ITEM_CUR_MANA: u32 = 107;
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
    /// The worn things asked about again and not yet answered: their old answers are not read.
    awaiting: Vec<u32>,
    /// The stone last used and when, until the server answers and it is looked at again.
    sent: Option<(u32, Instant)>,
    /// The stone and the junk of the drain sent, until its answer: junk still there was not taken.
    draining: Option<(u32, u32)>,
    /// Stone and junk pairs a drain left as they were: not tried again this session.
    refused: Vec<(u32, u32)>,
}

/// A carried stone as the rules see it, once appraised.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Stone {
    Charged(u32),
    Empty(u32),
}

impl Client {
    /// Recharge what is worn when any one thing runs low, and fill each empty stone from mana junk
    /// between fights. One use at a time, each answered.
    pub(crate) fn autoplay_mana_stones(&mut self, now: Instant) {
        let cfg = self.autoplay.config.mana.clone();
        if !cfg.on || self.autoplay.cast_in_flight(now) || self.autoplay.is_slot_wanted(now) {
            return;
        }
        if let Some((stone, at)) = self.autoplay.mana_stones.sent {
            if now.duration_since(at) < ANSWER_WAIT && self.world.objects.contains_key(&stone) {
                return;
            }
            self.autoplay.mana_stones.sent = None;
        }
        let st = &mut self.autoplay.mana_stones;
        if let Some((stone, junk)) = st.draining.take() {
            if self.world.objects.contains_key(&junk) {
                st.refused.push((stone, junk));
                let name = self.world.name_of(junk).unwrap_or_default().to_string();
                self.autoplay
                    .note(format!("{name} would not drain into the stone"), now);
            }
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
        // Asked about again (after a charge, or a minute on): the old figures say "low" until the
        // new ones come, and a stone went in every five seconds through "fully charged".
        let pending = |g: &u32| {
            self.appraise_queue.contains(g) || self.appraise_inflight.iter().any(|(i, _)| i == g)
        };
        let awaiting: Vec<u32> = self
            .autoplay
            .mana_stones
            .awaiting
            .iter()
            .copied()
            .filter(|g| pending(g))
            .collect();
        let fresh = awaiting.is_empty();
        self.autoplay.mana_stones.awaiting = awaiting;
        let low = fresh
            .then(|| self.low_on_mana(cfg.recharge_below))
            .flatten();
        if let (Some(&stone), Some(low)) = (charged.first(), low) {
            let me = self.world.player_guid.unwrap_or_default();
            self.use_stone(
                stone,
                me,
                format!("charging what is worn ({low} is low)"),
                now,
            );
            return;
        }
        if self.in_a_fight() {
            return;
        }
        let empty = stones.iter().find_map(|s| match s {
            Stone::Empty(g) => Some(*g),
            Stone::Charged(_) => None,
        });
        if let Some((stone, (junk, name))) = empty.and_then(|g| Some((g, self.mana_junk(g)?))) {
            self.autoplay.mana_stones.draining = Some((stone, junk));
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
        self.appraise_again(worn.iter().copied());
        self.autoplay.mana_stones.awaiting = worn;
    }

    /// How many more pieces of mana junk are worth taking: one per empty stone, less the junk
    /// carried or on its way from a corpse.
    pub(crate) fn junk_room(&self) -> usize {
        if !self.autoplay.config.mana.on {
            return 0;
        }
        let empty = self
            .stones()
            .iter()
            .filter(|s| matches!(s, Stone::Empty(_)))
            .count();
        let carried = self
            .world
            .inventory()
            .filter(|o| o.item_type & ac_world::item_type::MANA_STONE == 0)
            .filter_map(|o| self.stats_of(o.guid))
            .filter(|s| self.is_mana_junk(s))
            .count();
        let coming = self
            .autoplay
            .off_corpses
            .iter()
            .filter(|(g, action, _)| {
                *action == LootAction::Drain && !self.world.inventory().any(|o| o.guid == *g)
            })
            .count();
        empty.saturating_sub(carried + coming)
    }

    /// Whether `item` is to be drained: tagged so, and off a corpse if the built-in rule said it.
    fn is_mana_junk(&self, item: &ac_loot::items::ItemStats) -> bool {
        let ledger = &self.autoplay.ledger;
        ledger.of(item) == Some(LootAction::Drain)
            && (ledger.why(item.guid) != Some(ac_loot::profile::MANA_JUNK)
                || ledger.is_looted(item))
    }

    /// How many stones are worth carrying: the backup charged, and one more to fill.
    fn stones_wanted(&self) -> usize {
        self.autoplay.config.mana.backup as usize + 1
    }

    /// Whether a stone on a corpse is worth taking: fewer carried than [`Self::stones_wanted`].
    pub(crate) fn wants_mana_stone(&self) -> bool {
        let carried = self
            .world
            .inventory()
            .filter(|o| o.item_type & ac_world::item_type::MANA_STONE != 0)
            .count();
        self.autoplay.config.mana.on && carried < self.stones_wanted()
    }

    /// The carried stones whose appraisal is in: charged with mana, or empty.
    fn stones(&self) -> Vec<Stone> {
        self.world
            .inventory()
            .filter(|o| o.item_type & ac_world::item_type::MANA_STONE != 0)
            .filter_map(|o| {
                // Its charge alone, ItemCurMana: a stone has no ItemMaxMana, and read through
                // `ItemStats` (which wants both) a charged one poured into the junk it was to drain.
                let a = self.appraisals.get(&o.guid).filter(|a| a.success)?;
                Some(if a.int(ITEM_CUR_MANA).is_some_and(|m| m > 0) {
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
    fn mana_junk(&self, stone: u32) -> Option<(u32, String)> {
        let refused = &self.autoplay.mana_stones.refused;
        self.world
            .inventory()
            .filter(|o| o.item_type & ac_world::item_type::MANA_STONE == 0)
            .filter(|o| !refused.contains(&(stone, o.guid)))
            .find_map(|o| {
                let s = self
                    .stats_of(o.guid)
                    .filter(|s| s.appraised && s.mana > 0)?;
                self.is_mana_junk(&s).then(|| (o.guid, o.name.clone()))
            })
    }
}

#[cfg(test)]
mod tests;
