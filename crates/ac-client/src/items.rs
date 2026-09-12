//! Driving the server side of an appraisal.
//!
//! The vocabulary itself -- [`ItemStats`], the search language, slots
//! and tiers -- lives in `ac_loot::items`, because every rule about an
//! item is a predicate over it and the rules are not the client's
//! business. What is left here is the half that needs a server: asking
//! for an identify, and reading one back.

pub use ac_loot::items::*;

use crate::Client;

/// How many appraisals may be waiting on the server at once.
const AT_ONCE: usize = 8;
/// How long an unanswered appraisal is waited on before its place is
/// given to another.
const APPRAISAL_LOST: std::time::Duration = std::time::Duration::from_secs(2);

impl Client {
    /// Every carried and worn item as [`ItemStats`], appraised where the
    /// server has answered.
    pub fn item_stats(&self) -> Vec<ItemStats> {
        let me = self.world.player_guid;
        let skills = self.assets.skill_table().ok();
        let spells = self.assets.spell_table().ok();
        let skill_name = |id: u32| {
            skills
                .as_ref()
                .and_then(|t| t.get(id).map(|s| s.name.clone()))
                .unwrap_or_else(|| format!("skill {id}"))
        };
        let spell_name = |id: u32| {
            spells
                .as_ref()
                .and_then(|t| t.get(id).map(|s| s.name.clone()))
                .or_else(|| self.known_spells.get(&id).cloned())
                .unwrap_or_else(|| format!("spell {id}"))
        };
        self.world
            .wielded()
            .chain(self.world.inventory())
            .map(|o| {
                let s = ItemStats::of(o, me);
                match self.appraisals.get(&o.guid) {
                    Some(a) => s.with_appraisal(a, &skill_name, &spell_name),
                    None => s,
                }
            })
            .collect()
    }

    /// Carried items matching a search line (see [`Query`]).
    pub fn find_items(&self, line: &str) -> Vec<ItemStats> {
        let q = Query::parse(line);
        self.item_stats()
            .into_iter()
            .filter(|s| s.matches(&q))
            .collect()
    }

    /// Ask the server about every carried item we have not appraised,
    /// one at a time (see `tick_appraise`). Returns how many were queued.
    pub fn appraise_all(&mut self) -> usize {
        let todo: Vec<u32> = self
            .world
            .wielded()
            .chain(self.world.inventory())
            .map(|o| o.guid)
            .filter(|g| {
                !self.appraisals.contains_key(g)
                    && !self.appraise_queue.contains(g)
                    && !self.appraise_inflight.iter().any(|(i, _)| i == g)
            })
            .collect();
        let n = todo.len();
        self.appraise_queue.extend(todo);
        n
    }

    /// How many carried items still lack an appraisal.
    pub fn unappraised_count(&self) -> usize {
        self.world
            .wielded()
            .chain(self.world.inventory())
            .filter(|o| !self.appraisals.contains_key(&o.guid))
            .count()
    }

    /// Send the next queued appraisal once the previous one has answered
    /// (or gone stale).
    pub fn tick_appraise(&mut self) {
        // Answered, or long enough gone to be lost.
        let appraisals = &self.appraisals;
        self.appraise_inflight.retain(|(guid, since)| {
            !appraisals.contains_key(guid) && since.elapsed() <= APPRAISAL_LOST
        });
        // Several in the air at once. The server answers each on its
        // own, so a corpse full of things need not be read one item per
        // round trip -- which is what made standing over a body take
        // seconds. Capped so a big pack does not become a flood.
        while self.appraise_inflight.len() < AT_ONCE {
            let Some(guid) = self.appraise_queue.pop_front() else {
                break;
            };
            if self.appraisals.contains_key(&guid) || !self.world.objects.contains_key(&guid) {
                continue;
            }
            self.appraise(guid);
            self.appraise_inflight
                .push((guid, std::time::Instant::now()));
        }
    }
}

impl Client {
    /// The stats of any object we know about: a carried item, a loot item
    /// in the open corpse or chest, a vendor's stock line, something on
    /// the ground or offered in a trade. Appraised where the cache has
    /// the answer; before that the object description's own spell (a
    /// scroll's, a wand's) still shows. `None` for an unknown guid.
    pub fn stats_of(&self, guid: u32) -> Option<ItemStats> {
        let me = self.world.player_guid;
        let (mut stats, own_spell) = match self.world.objects.get(&guid) {
            Some(o) => (ItemStats::of(o, me), o.spell_id),
            None => {
                let it = self
                    .world
                    .open_vendor
                    .as_ref()?
                    .items
                    .iter()
                    .find(|it| it.guid == guid)?;
                (ItemStats::of_desc(guid, &it.desc), it.desc.spell_id)
            }
        };
        let skills = self.assets.skill_table().ok();
        let spells = self.assets.spell_table().ok();
        let skill_name = |id: u32| {
            skills
                .as_ref()
                .and_then(|t| t.get(id).map(|s| s.name.clone()))
                .unwrap_or_else(|| format!("skill {id}"))
        };
        let spell_name = |id: u32| {
            spells
                .as_ref()
                .and_then(|t| t.get(id).map(|s| s.name.clone()))
                .or_else(|| self.known_spells.get(&id).cloned())
                .unwrap_or_else(|| format!("spell {id}"))
        };
        match self.appraisals.get(&guid) {
            Some(a) => stats = stats.with_appraisal(a, &skill_name, &spell_name),
            None if own_spell != 0 => stats.spells.push(spell_name(own_spell)),
            None => {}
        }
        Some(stats)
    }

    /// Queue background appraisals (see `tick_appraise`) for the given
    /// objects that are not appraised, queued or in flight: the items of
    /// an open corpse, a trade offer, anything in `world.objects`. Guids
    /// the world does not hold (a vendor's stock lines, which the queue
    /// cannot serve; a click on one appraises it) are skipped. Returns
    /// how many were queued.
    pub fn appraise_many(&mut self, guids: impl IntoIterator<Item = u32>) -> usize {
        let mut n = 0;
        for g in guids {
            let known = self.world.objects.contains_key(&g);
            let pending = self.appraisals.contains_key(&g)
                || self.appraise_queue.contains(&g)
                || self.appraise_inflight.iter().any(|(i, _)| *i == g);
            if known && !pending {
                self.appraise_queue.push_back(g);
                n += 1;
            }
        }
        n
    }
}
