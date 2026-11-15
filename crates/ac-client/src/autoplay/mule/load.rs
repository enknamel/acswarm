//! Loading a mule: what the character keeps and does not use goes to a mule on another account,
//! met where it stands once a mule host (`ac_plugin::mules`) has logged it in. Asked for once the
//! pack runs low with something to load; nothing tinkered, inscribed or attuned ever goes. A
//! delivery the other way: the character goes to the mule and waits while it hands things over.

use std::time::Instant;

use ac_world::item_type as kind;

use super::Doing;
use crate::giving::GIVE_REACH;
use crate::items::{NumKey, Op, Term};
use crate::profile::{Ask, LootAction, QUESTS};
use crate::Client;

/// A full salvage bag: 100 units (ACE `Salvage.cs` caps a bag at 100).
const FULL_BAG: u32 = 100;
/// The kinds that go to a mule; tools, supplies, money, keys and scrolls stay.
const LOADED_KINDS: u32 = kind::MELEE_WEAPON
    | kind::ARMOR
    | kind::CLOTHING
    | kind::JEWELRY
    | kind::MISSILE_WEAPON
    | kind::CASTER
    | kind::GEM
    | kind::MISC
    | kind::USELESS
    | kind::CRAFT_COOKING_BASE
    | kind::CRAFT_ALCHEMY_BASE
    | kind::CRAFT_FLETCHING_BASE
    | kind::CRAFT_ALCHEMY_INTERMEDIATE
    | kind::CRAFT_FLETCHING_INTERMEDIATE
    | kind::TINKERING_MATERIAL;

/// The mule being loaded: met at `place` once its host says where it stands.
#[derive(Clone, Debug, PartialEq)]
pub struct Meet {
    /// The mule's character name.
    pub name: String,
    pub place: ac_world::Place,
}

/// The loading of a mule, from the pack running low until all is given or the mule is full.
#[derive(Clone, Debug, Default)]
pub(crate) struct Muling {
    /// A mule is wanted: set when the pack ran low with something to load, until it is given.
    pub(crate) loading: bool,
    /// Where the mule its host logged in stands.
    pub(crate) meet: Option<Meet>,
    /// The last mule's packs filled with things still to load: its host picks another.
    pub(crate) mule_full: bool,
    /// A mule with a delivery for this character, and where it stands.
    pub(crate) fetch: Option<Meet>,
    /// As a mule: what to hand over, and to whom.
    pub(crate) deliver: Option<Delivery>,
    /// Delivered from a mule because the player asked for them: never loaded back.
    pub(crate) kept: Vec<u32>,
    /// Gives the server refused this session (attuned, no room): not asked again of this mule.
    pub(crate) refused: Vec<u32>,
}

/// What a mule hands to one character: items by guid, only those, only to it.
#[derive(Clone, Debug, PartialEq)]
pub struct Delivery {
    /// The receiving character's name.
    pub to: String,
    pub items: Vec<u32>,
}

impl Client {
    /// Whether a mule is wanted: true from the pack running low with something to load until it
    /// is all given; a mule host calls it every few seconds and logs a mule in while it holds.
    pub fn is_mule_wanted(&mut self) -> bool {
        if !self.autoplay.config.growth.mule_runs || self.autoplay.config.mule {
            self.autoplay.muling.loading = false;
            return false;
        }
        // Short of slots, or of weight as a town run counts it: salvage bags weigh 100 each, and a
        // pack of them sent him to town with 42 slots free and never called the mule.
        let cfg = self.autoplay.config.growth.clone();
        let full = self.pack_low_on_room() || self.laden(&cfg);
        if !self.autoplay.muling.loading && full {
            // Only an appraisal tells a tinkered or inscribed thing: asked before the load is read.
            let unknown = self.unappraised_load();
            if !unknown.is_empty() {
                self.appraise_many(unknown);
            }
        }
        if !self.autoplay.muling.loading && full && !self.mule_load().is_empty() {
            tracing::info!("muling: the pack is low or heavy, a mule is wanted");
            self.autoplay.muling.loading = true;
            self.autoplay.muling.mule_full = false;
            // Another mule may take what the last refused.
            self.autoplay.muling.refused.clear();
        }
        self.autoplay.muling.loading
    }

    /// Tell the character where the mule stands (`None`: logged off, or none could be had).
    pub fn set_mule_meet(&mut self, meet: Option<Meet>) {
        if meet.is_none() && self.autoplay.muling.meet.is_some() {
            self.autoplay.muling.loading = false;
        }
        self.autoplay.muling.meet = meet;
    }

    /// Go to a mule with a delivery for this character (`None`: all of it has come).
    pub fn set_mule_fetch(&mut self, meet: Option<Meet>) {
        self.autoplay.muling.fetch = meet;
    }

    /// Items a mule delivered, asked for by the player: none of them is loaded onto a mule again.
    pub fn set_mule_kept(&mut self, items: Vec<u32>) {
        self.autoplay.muling.kept = items;
    }

    /// As a mule: hand `delivery` over when its receiver is in reach (`None`: hand nothing).
    pub fn set_mule_delivery(&mut self, delivery: Option<Delivery>) {
        self.autoplay.muling.deliver = delivery;
    }

    /// Whether the server refused a give of `guid` this session.
    pub fn is_give_refused(&self, guid: u32) -> bool {
        self.autoplay.muling.refused.contains(&guid)
    }

    /// A mule being met: loaded, or fetched from.
    pub(crate) fn is_muling(&self) -> bool {
        let m = &self.autoplay.muling;
        (m.loading && m.meet.is_some()) || m.fetch.is_some()
    }

    /// The last mule filled before everything was given to it.
    pub fn is_mule_full(&self) -> bool {
        self.autoplay.muling.mule_full
    }

    /// What goes to a mule: what the loot profile kept, of the kinds a mule holds, and every full
    /// salvage bag; never what is worn, the best weapon of each stance, a tool or an essence, the
    /// Quests section's items, the buy list's, what a mule delivered (`set_mule_kept`), nor
    /// anything an appraisal showed tinkered, inscribed or attuned (not yet appraised: not yet).
    pub fn mule_load(&self) -> Vec<u32> {
        let stats = self.item_stats();
        let Some(profile) = self.loot_profile() else {
            return Vec::new();
        };
        let quest_items: Vec<u32> = profile
            .rules
            .iter()
            .filter(|r| r.group == QUESTS)
            .flat_map(|r| r.all.iter())
            .filter_map(|a| match a {
                Ask::Item(Term::Num(NumKey::Class, Op::Eq, v)) => Some(*v as u32),
                _ => None,
            })
            .collect();
        let bought: Vec<String> = profile
            .buy
            .iter()
            .map(|b| b.what.trim().to_lowercase())
            .collect();
        let wielder = self.wielder();
        let in_use: Vec<u32> = [
            crate::weapons::Stance::Melee,
            crate::weapons::Stance::Missile,
            crate::weapons::Stance::Magic,
        ]
        .into_iter()
        .filter_map(|s| crate::weapons::best(&stats, s, None, &wielder))
        .map(|c| c.guid)
        .collect();
        stats
            .iter()
            .filter(|s| !s.wielded && s.item_type & LOADED_KINDS != 0)
            .filter(|s| !ac_world::pack_slot::used_by(s.wcid, s.item_type & kind::CONTAINER != 0))
            .filter(|s| s.appraised && s.tinks == 0 && !s.inscribed && !s.attuned)
            // A kit, a lockpick, a stone: what is used on something is a tool kept. Salvage is
            // used on an item too (Usable 0x80008) but is the load.
            .filter(|s| s.item_type & kind::TINKERING_MATERIAL != 0 || !self.is_tool(s.guid))
            // An essence is a supply the summon row burns.
            .filter(|s| !crate::autoplay::summoning::is_essence(&s.name, s.max_structure))
            .filter(|s| !quest_items.contains(&s.wcid) && !in_use.contains(&s.guid))
            .filter(|s| !self.autoplay.muling.kept.contains(&s.guid))
            .filter(|s| !bought.contains(&s.name.trim().to_lowercase()))
            .filter(|s| {
                if s.item_type & kind::TINKERING_MATERIAL != 0 {
                    s.structure >= FULL_BAG
                } else {
                    self.autoplay.ledger.of(s) == Some(LootAction::Keep)
                }
            })
            .map(|s| s.guid)
            .collect()
    }

    /// Whether `guid` is applied to something (a kit, a lockpick, a mana stone): ACE `Usable`'s
    /// target bits.
    fn is_tool(&self, guid: u32) -> bool {
        self.world
            .objects
            .get(&guid)
            .is_some_and(|o| o.usable > 0xFFFF)
    }

    /// The muling row: loading the mule its host logged in, else fetching a delivery from one.
    pub(crate) fn autoplay_mule(&mut self, _now: Instant) -> bool {
        if self.autoplay.config.mule {
            return false;
        }
        let m = &self.autoplay.muling;
        if let Some(meet) = m.meet.clone().filter(|_| m.loading) {
            return self.load_mule(meet);
        }
        match self.autoplay.muling.fetch.clone() {
            Some(meet) => self.fetch_from_mule(meet),
            None => false,
        }
    }

    /// To a mule with a delivery, and standing by it while it hands things over.
    fn fetch_from_mule(&mut self, meet: Meet) -> bool {
        let Some(me) = self.my_position() else {
            return false;
        };
        if self.mule_in_reach(&meet.name, me).is_some() {
            let said = format!("taking a delivery from {}", meet.name);
            self.autoplay.say(Doing::Helping, said);
            return true;
        }
        let said = format!("going to the mule {} for a delivery", meet.name);
        self.autoplay.say(Doing::Traveling, said);
        self.journey_to(meet.place, GIVE_REACH * 0.5)
    }

    /// Everything to load handed over in one run of gives; done when nothing is left, or the mule
    /// took no more.
    fn load_mule(&mut self, meet: Meet) -> bool {
        if let Some(g) = self.giving.as_ref() {
            return !g.by_hand;
        }
        let load = self.mule_load();
        let unknown: Vec<u32> = self.unappraised_load();
        if !unknown.is_empty() {
            self.appraise_many(unknown);
        }
        let Some(me) = self.my_position() else {
            return false;
        };
        if let Some(mule) = self.mule_in_reach(&meet.name, me) {
            let (held, load): (Vec<u32>, Vec<u32>) =
                load.into_iter().partition(|g| self.is_give_refused(*g));
            let held_back = held.len();
            if load.is_empty() {
                // Given all it would take: what the server held back stayed for want of room.
                self.autoplay.muling.mule_full = held_back > 0;
                self.autoplay.muling.loading = false;
                tracing::info!(held_back, "muling: done with {}", meet.name);
                return false;
            }
            let n = load.len();
            if self.give_items(mule, load) {
                let said = format!("loading {n} item(s) onto {}", meet.name);
                self.autoplay.say(Doing::Helping, said);
                return true;
            }
            return false;
        }
        self.autoplay
            .say(Doing::Traveling, format!("going to the mule {}", meet.name));
        self.journey_to(meet.place, GIVE_REACH * 0.5)
    }

    /// Carried things of the kinds a mule holds, kept, and not yet appraised.
    fn unappraised_load(&self) -> Vec<u32> {
        self.item_stats()
            .iter()
            .filter(|s| !s.wielded && !s.appraised && s.item_type & LOADED_KINDS != 0)
            .filter(|s| self.autoplay.ledger.of(s) == Some(LootAction::Keep))
            .map(|s| s.guid)
            .collect()
    }
}

#[cfg(test)]
mod tests;
