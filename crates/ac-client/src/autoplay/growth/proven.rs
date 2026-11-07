//! The loot tier this character is known to survive: the highest tier of creature it has killed,
//! raised on each kill of ours, seeded from the quest log's kill counts, kept between sessions.
//! A hunting ground is chosen up to one tier past it (`best_ground`); a death there moves it on.

use crate::Client;

impl Client {
    /// The highest loot tier killed: this session's and earlier ones', or a kill task's creature.
    pub(crate) fn proven_tier(&self) -> u32 {
        let counted = self
            .quest_log
            .kills
            .iter()
            .filter(|(_, n)| **n > 0)
            .flat_map(|(counter, _)| {
                ac_world::quests::sources_of(&ac_world::quests::Want::Kill(counter.clone()))
                    .filter_map(|s| ac_world::elements::known(s.wcid, &s.name))
                    .map(|c| c.tier)
                    .collect::<Vec<_>>()
            })
            .max()
            .unwrap_or(0);
        self.autoplay.growth.proven_tier.max(counted)
    }

    /// A kill of ours named in `text`: the creature fought, by its name, raises the tier proven.
    pub(crate) fn note_kill_tier(&mut self, text: &str) {
        let tier = [self.attack_target, self.autoplay.casting_at()]
            .into_iter()
            .flatten()
            .filter_map(|g| self.world.objects.get(&g))
            .find(|o| !o.name.is_empty() && text.contains(o.name.as_str()))
            .and_then(|o| ac_world::elements::known(o.weenie_class_id, &o.name))
            .map(|c| c.tier)
            .unwrap_or(0);
        if tier <= self.autoplay.growth.proven_tier {
            return;
        }
        tracing::info!(
            "ground: killed a tier {tier} creature; grounds up to tier {} now",
            tier + 1
        );
        self.autoplay.growth.proven_tier = tier;
        let Some(path) = self.quest_file(".tier.json") else {
            return;
        };
        if let Err(e) = ac_store::write_json_atomic(&path, &tier, ac_store::Visibility::Normal) {
            tracing::warn!("ground: writing {}: {e}", path.display());
        }
    }

    /// Read back the tier proven in earlier sessions, at login.
    pub(crate) fn load_proven_tier(&mut self) {
        let Some(path) = self.quest_file(".tier.json") else {
            return;
        };
        match ac_store::read_json::<u32>(&path) {
            Ok(Some(t)) => self.autoplay.growth.proven_tier = t,
            Ok(None) => {}
            Err(e) => tracing::warn!("ground: reading {}: {e}", path.display()),
        }
    }
}

#[cfg(test)]
mod tests;
