//! The one place a name becomes a thing: a spell, an item in the pack, a ware
//! on a counter, an object in view, the corpse of the last target, a place on
//! the map. Every front end resolves here, so that a name means the same thing
//! whoever typed it.

use super::{SpellRef, Target};
use crate::autoplay::Role;
use crate::Client;

/// The guid `target` names, among everything in view and carried.
pub fn object(c: &Client, target: &Target) -> Option<u32> {
    match target {
        Target::Guid(g) => c.world.objects.contains_key(g).then_some(*g),
        Target::Me => c.world.player_guid,
        Target::Selected => c.selected.filter(|g| c.world.objects.contains_key(g)),
        Target::LastCorpse => last_corpse(c),
        Target::Name(n) => c.object_by_name(n),
    }
}

/// The guid `target` names in the pack: what is sold, given, split or salvaged
/// is carried, so a name looks there rather than at the floor.
pub fn pack_item(c: &Client, target: &Target) -> Option<u32> {
    match target {
        Target::Name(n) => c.pack_item_by_name(n),
        _ => object(c, target),
    }
}

/// The guid `target` names on the open counter's shelf.
pub fn ware(c: &Client, target: &Target) -> Option<u32> {
    match target {
        Target::Name(n) => c.ware_by_name(n),
        Target::Guid(g) => c
            .world
            .open_vendor
            .as_ref()?
            .items
            .iter()
            .any(|i| i.guid == *g)
            .then_some(*g),
        _ => None,
    }
}

/// The spell `spell` names, by id or by name.
pub fn spell(c: &Client, spell: &SpellRef) -> Option<u32> {
    match spell {
        SpellRef::Id(id) => Some(*id),
        SpellRef::Name(n) => c.spell_by_name(n),
    }
}

/// The corpse of the last creature fought, by the name the server gives it.
pub fn last_corpse(c: &Client) -> Option<u32> {
    if c.last_target_name.is_empty() {
        return None;
    }
    c.object_by_name(&format!("Corpse of {}", c.last_target_name))
}

/// The place `name` means on the map (`ac_world::towns`: case-insensitive,
/// prefix or substring).
pub fn place(name: &str) -> Option<&'static ac_world::towns::Place> {
    ac_world::towns::find(name)
}

/// The team role `name` means ("fighter", "healer", "debuffer").
pub fn role(name: &str) -> Option<Role> {
    match name.trim().to_lowercase().as_str() {
        "fighter" => Some(Role::Fighter),
        "healer" => Some(Role::Healer),
        "debuffer" => Some(Role::Debuffer),
        _ => None,
    }
}

impl Client {
    /// The spell `name` means: of the family, the strongest that can be cast
    /// right now; by prefix first, then by substring, then a scroll of it.
    pub fn spell_by_name(&self, name: &str) -> Option<u32> {
        let want = name.trim().to_lowercase();
        if want.is_empty() {
            return None;
        }
        let table = self.assets.spell_table().ok();
        // A name like "Heal Self" means the best Heal Self we can manage, not
        // the best in the book. Failing any castable, the strongest known, so
        // the reason it cannot be cast can be reported.
        for by_prefix in [true, false] {
            let mut best_castable: Option<(u32, u32)> = None;
            let mut best_known: Option<(u32, u32)> = None;
            for id in &self.world.stats.spells {
                let sp = table.as_ref().and_then(|t| t.get(*id));
                let full = sp
                    .map(|s| s.name.clone())
                    .or_else(|| self.known_spells.get(id).cloned())
                    .unwrap_or_default()
                    .to_lowercase();
                let hit = if by_prefix {
                    full.starts_with(&want)
                } else {
                    full.contains(&want)
                };
                if !hit {
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
            if let Some((id, _)) = best_castable.or(best_known) {
                return Some(id);
            }
        }
        // Nothing in the book: a Scroll of it in the pack is what was meant,
        // and the scroll carries the spell's own id.
        self.world
            .inventory()
            .find(|o| {
                o.spell_id != 0
                    && o.name
                        .strip_prefix("Scroll of ")
                        .is_some_and(|n| n.to_lowercase().starts_with(&want))
            })
            .map(|o| o.spell_id)
    }

    /// The object `name` names: the nearest in view, what is carried counting
    /// as nearest, and an exact name beating a prefix.
    pub fn object_by_name(&self, name: &str) -> Option<u32> {
        let me = self.my_position();
        let my_guid = self.world.player_guid;
        let mut best: Option<(f32, u32)> = None;
        for o in self.world.objects.values() {
            if !o.name.starts_with(name) {
                continue;
            }
            let carried = my_guid.is_some() && (o.container == my_guid || o.wielder == my_guid);
            let exact = if o.name == name { 0.0 } else { 1000.0 };
            let d = exact
                + if carried {
                    0.0
                } else {
                    let Some(p) = o.display.or(o.position) else {
                        continue;
                    };
                    me.map(|m| (ac_world::landblock_origin(p.cell) + p.local).distance(m))
                        .unwrap_or(0.0)
                };
            if best.map(|(bd, _)| d < bd).unwrap_or(true) {
                best = Some((d, o.guid));
            }
        }
        best.map(|(_, guid)| guid)
    }

    /// The carried item whose name starts with `name`.
    pub fn pack_item_by_name(&self, name: &str) -> Option<u32> {
        self.world
            .inventory()
            .find(|o| o.name.starts_with(name))
            .map(|o| o.guid)
    }

    /// The ware on the open counter's shelf whose name starts with `name`.
    pub fn ware_by_name(&self, name: &str) -> Option<u32> {
        self.world
            .open_vendor
            .as_ref()?
            .items
            .iter()
            .find(|i| i.desc.name.starts_with(name))
            .map(|i| i.guid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    /// A carried item of `name`, as the pack holds it.
    fn carried(guid: u32, name: &str) -> ac_world::WorldObject {
        ac_world::WorldObject {
            guid,
            name: name.into(),
            container: Some(testkit::ME),
            ..Default::default()
        }
    }

    /// Holtburg, where the testkit stands a character.
    const FIELD: u32 = 0xA9B4_0019;

    /// The world point `metres` along the field's diagonal.
    fn at(metres: f32) -> glam::Vec3 {
        ac_world::landblock_origin(FIELD) + glam::Vec3::new(metres, metres, 0.0)
    }

    #[test]
    fn a_name_finds_what_is_carried_before_what_is_on_the_floor() {
        let mut c = testkit::offline_client();
        c.world.player_guid = Some(testkit::ME);
        testkit::stand(&mut c, FIELD, glam::Vec3::new(10.0, 10.0, 0.0));
        c.world
            .objects
            .insert(0x8000_0001, carried(0x8000_0001, "Shield of Rest"));
        let mut floor = testkit::creature(0x8000_0002, "Shield of Rest");
        floor.position = testkit::placed(FIELD, at(60.0));
        c.world.objects.insert(floor.guid, floor);
        assert_eq!(c.object_by_name("Shield"), Some(0x8000_0001));
        assert_eq!(c.pack_item_by_name("Shield"), Some(0x8000_0001));
    }

    #[test]
    fn the_last_corpse_is_the_one_named_after_the_last_target() {
        let mut c = testkit::offline_client();
        c.world.player_guid = Some(testkit::ME);
        assert_eq!(last_corpse(&c), None, "nothing fought yet");
        c.last_target_name = "Drudge Skulker".into();
        let mut corpse = testkit::corpse(0x8000_0003, "Corpse of Drudge Skulker");
        corpse.position = testkit::placed(FIELD, at(12.0));
        c.world.objects.insert(corpse.guid, corpse);
        assert_eq!(
            object(&c, &Target::LastCorpse),
            Some(0x8000_0003),
            "the corpse of the last target"
        );
    }

    #[test]
    fn a_target_that_is_gone_resolves_to_nothing() {
        let mut c = testkit::offline_client();
        c.world.player_guid = Some(testkit::ME);
        c.selected = Some(0x8000_0009);
        assert_eq!(object(&c, &Target::Guid(0x8000_0009)), None);
        assert_eq!(object(&c, &Target::Selected), None);
        assert_eq!(object(&c, &Target::Me), Some(testkit::ME));
    }

    #[test]
    fn a_role_is_named_in_the_words_the_panels_and_scripts_use() {
        assert_eq!(role("Healer"), Some(Role::Healer));
        assert_eq!(role(" debuffer "), Some(Role::Debuffer));
        assert_eq!(role("tank"), None);
    }
}
