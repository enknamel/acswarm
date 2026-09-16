//! The one place a name becomes a thing: a spell, an item in the pack, a ware
//! on a counter, an object in view, the corpse of the last target, a place on
//! the map. Every front end resolves here, so that a name means the same thing
//! whoever typed it.

use crate::Client;

impl Client {
    /// The id of a known spell whose name starts with `name`, preferring
    /// the highest level learnt (the last in the spellbook order).
    pub fn spell_by_name(&self, name: &str) -> Option<u32> {
        let want = name.trim().to_lowercase();
        if want.is_empty() {
            return None;
        }
        let table = self.assets.spell_table().ok();
        // Of the family, the strongest that can be cast right now: a
        // name like "Heal Self" means the best Heal Self we can manage,
        // not the best in the book. Failing any castable, the strongest
        // known, so the reason it cannot be cast can be reported.
        let mut best_castable: Option<(u32, u32)> = None;
        let mut best_known: Option<(u32, u32)> = None;
        for id in &self.world.stats.spells {
            let sp = table.as_ref().and_then(|t| t.get(*id));
            let full = sp
                .map(|s| s.name.clone())
                .or_else(|| self.known_spells.get(id).cloned())
                .unwrap_or_default()
                .to_lowercase();
            if !full.starts_with(&want) && !full.contains(&want) {
                continue;
            }
            // Power orders a family; a spell the table lacks ranks by id.
            let power = sp.map(|s| s.power).unwrap_or(*id);
            let castable = matches!(
                self.can_cast(*id),
                crate::magic::CastCheck::Ok | crate::magic::CastCheck::NoCaster
            );
            if castable && best_castable.is_none_or(|(_, p)| power > p) {
                best_castable = Some((*id, power));
            }
            if best_known.is_none_or(|(_, p)| power > p) {
                best_known = Some((*id, power));
            }
        }
        best_castable.or(best_known).map(|(id, _)| id)
    }
}
