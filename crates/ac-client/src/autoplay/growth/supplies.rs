use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use super::Growth;
use crate::Client;
use ac_world::item_type;

/// How long a [`Client::spells_cast`] answer is kept while the spellbook stays the same size.
const SPELLS_CAST_EVERY: Duration = Duration::from_secs(1);

impl Client {
    // ---- town runs ------------------------------------------------

    /// How many of a named thing are carried (name contains, as the
    /// team's `keep_stocked` counts).
    pub(crate) fn carried_named(&self, name: &str) -> u32 {
        let want = name.trim().to_lowercase();
        self.world
            .inventory()
            .filter(|o| o.name.to_lowercase().contains(&want))
            .map(|o| o.stack_size.max(1))
            .sum()
    }

    /// Pyreals carried.
    pub fn purse(&self) -> u32 {
        self.world
            .inventory()
            .filter(|o| o.item_type & item_type::MONEY != 0)
            .map(|o| o.stack_size.max(1))
            .sum()
    }

    /// What the character can spend. Coin and trade notes both: a note
    /// is money in a lighter form, and a vendor takes either.
    pub fn spendable(&self) -> u32 {
        let notes: u32 = self
            .world
            .inventory()
            .filter(|o| o.item_type & item_type::PROMISSORY_NOTE != 0)
            .map(|o| o.value.saturating_mul(o.stack_size.max(1)))
            .sum();
        self.purse().saturating_add(notes)
    }

    /// Every spell autoplay casts, in one list: what the restock list
    /// stocks components for, and what the counter's component guard
    /// keeps.
    ///
    /// All of them, not only the buffs and the bolts. The list once
    /// held those two alone, and a caster's heal, its vulnerabilities
    /// and imperils, the team's debuffs, its recalls and the casts
    /// that keep its stamina and mana up were stocked for nothing: each
    /// carries its own herb, powder, potion and talisman that no buff
    /// or bolt shares (Heal Self V burns 7, 26, 41 and 61; Flame Bolt
    /// I burns 15, 34, 46 and 55), so when the pack ran out the heal
    /// stopped casting and nobody went to town for it.
    ///
    /// Known, not castable. Whether a spell can be cast this moment is
    /// mostly a question of whether its components are in the pack,
    /// and that is the question this list is asked in order to answer.
    /// A buff is asked for with that check left out for the same
    /// reason (`wanted_buffs_if`).
    pub(crate) fn spells_cast(&self) -> Vec<u32> {
        // It walks the whole spellbook and the buffs worth wearing, and the planners ask it several
        // times a tick; what it answers moves with the book, the skills and the settings, over minutes.
        let book = self.world.stats.spells.len();
        if let Some((at, n, spells)) = self.autoplay.spells_cast_memo.borrow().as_ref() {
            if *n == book && at.elapsed() < SPELLS_CAST_EVERY {
                return spells.clone();
            }
        }
        let spells = self.spells_cast_now();
        *self.autoplay.spells_cast_memo.borrow_mut() = Some((Instant::now(), book, spells.clone()));
        spells
    }

    /// [`Self::spells_cast`], worked out afresh.
    fn spells_cast_now(&self) -> Vec<u32> {
        use ac_world::vitals::{boosts_of, transfers_between, vital};
        let known = |id: &u32| self.world.stats.spells.contains(id);
        let table = self.assets.spell_table().ok();
        let at_another = |id: &u32| {
            table
                .as_ref()
                .and_then(|t| t.get(*id))
                .is_some_and(|s| s.needs_target())
        };
        let by_name = |names: &[String]| -> Vec<u32> {
            names.iter().filter_map(|n| self.spell_by_name(n)).collect()
        };
        let cfg = &self.autoplay.config;
        let mut out: Vec<u32> = Vec::new();
        // The buffs it should be wearing, castable or not, and the
        // ones the player named.
        let wearable = |id: u32| {
            !matches!(
                self.can_cast(id),
                crate::magic::CastCheck::NotKnown | crate::magic::CastCheck::TooHard { .. }
            )
        };
        out.extend(self.wanted_buffs_if(wearable).iter().map(|w| w.spell));
        out.extend(by_name(&cfg.buffs.spells));
        // The attack: every bolt in the book, or the ones named.
        if cfg.fight.spells.is_empty() {
            out.extend(self.attack_spells_known());
        } else {
            out.extend(by_name(&cfg.fight.spells));
        }
        // Softening: a vulnerability for each element, and the
        // imperils (see `autoplay_make_vulnerable`, `autoplay_soften`).
        for element in ac_world::elements::ALL {
            out.extend(
                crate::weapons::vulnerability_spells(element)
                    .into_iter()
                    .filter(known)
                    .filter(at_another),
            );
        }
        out.extend(self.imperil_spells());
        // The team's work: its debuffs by name, and the healer's heal.
        out.extend(by_name(&cfg.team.debuffs));
        if cfg.team.role == crate::autoplay::Role::Healer {
            out.extend(self.spell_by_name("Heal Other"));
        }
        // Its own heals (see `choose_heal`), and the casts that keep
        // stamina and mana up (see `autoplay_vitals`).
        out.extend(
            boosts_of(vital::HEALTH)
                .iter()
                .map(|b| b.spell)
                .filter(known),
        );
        for from in [vital::STAMINA, vital::MANA] {
            out.extend(
                transfers_between(from, vital::HEALTH)
                    .iter()
                    .map(|t| t.spell)
                    .filter(known),
            );
        }
        if cfg.survive.manage_mana {
            out.extend(
                boosts_of(vital::STAMINA)
                    .iter()
                    .map(|b| b.spell)
                    .filter(known),
            );
            out.extend(
                transfers_between(vital::STAMINA, vital::MANA)
                    .iter()
                    .map(|t| t.spell)
                    .filter(known),
            );
        }
        // The recalls a journey is planned over (see `castable_recalls`).
        out.extend(
            self.world
                .stats
                .spells
                .iter()
                .copied()
                .filter(|id| ac_world::recalls::is_recall(*id)),
        );
        out.sort_unstable();
        out.dedup();
        out
    }

    /// How many of each spell component to carry, keyed by component id.
    ///
    /// A taper is the yardstick: it is what a caster runs out of, and
    /// it is the one number a player sets. Everything else is scaled to
    /// it by how fast it burns relative to a taper, which the client
    /// can work out rather than guess, because both halves are in its
    /// own tables. ACE rolls each component of a formula separately
    /// (`Spell.TryBurnComponents`):
    ///
    /// ```text
    /// burn = spell.ComponentLoss * component.CDM * min(1, power / skill)
    /// ```
    ///
    /// The skill term cancels when one component is divided by another
    /// of the same spell, so the ratio is just the loss and the CDMs,
    /// and a component used by several spells is stocked for the one
    /// that burns it fastest. With foci a top-level cast burns about
    /// 0.4 of a taper against 0.003 of a scarab, so a thousand tapers
    /// comes out at a handful of scarabs rather than a thousand.
    pub fn component_targets(&self, tapers_keep: u32) -> BTreeMap<u32, u32> {
        let mut out: BTreeMap<u32, u32> = BTreeMap::new();
        if tapers_keep == 0 {
            return out;
        }
        let (Ok(table), Ok(comps)) = (self.assets.spell_table(), self.assets.spell_components())
        else {
            return out;
        };
        // The fastest rate each component burns at, across the spells
        // this character actually casts.
        let mut fastest: BTreeMap<u32, f32> = BTreeMap::new();
        for spell in self.spells_cast() {
            let Some(sp) = table.get(spell) else { continue };
            for id in self.current_formula(spell) {
                let Some(c) = comps.get(id) else { continue };
                let rate = sp.component_loss * c.cdm;
                let e = fastest.entry(id).or_insert(0.0);
                if rate > *e {
                    *e = rate;
                }
            }
        }
        let taper = fastest
            .get(&crate::magic::PRISMATIC_TAPER)
            .copied()
            .filter(|r| *r > 0.0);
        // A floor under everything: twenty to the thousand. The burn
        // rates say a scarab would only need eight, and that is cutting
        // it far too fine for something bought once a trip -- being
        // over-provisioned on the cheap, light things costs a slot and
        // saves a walk. Anything that genuinely burns faster than the
        // floor keeps its own number, so a caster without foci still
        // stocks its herbs properly.
        let floor = (tapers_keep / 50).max(1);
        for (id, rate) in &fastest {
            let want = match taper {
                // Scaled to the taper by how fast it burns.
                Some(t) => ((tapers_keep as f32) * rate / t).round() as u32,
                // No taper in any formula (no foci, or an odd build):
                // the configured number stands for everything.
                None => tapers_keep,
            };
            out.insert(*id, want.max(floor));
        }
        out
    }

    /// The spell components this character's spells burn, by weenie
    /// class: what it goes to town to buy, as against what it goes to
    /// town to sell.
    pub fn burns(&self, _cfg: &Growth) -> Vec<u32> {
        let Ok(mapper) = self.assets.spell_component_ids() else {
            return Vec::new();
        };
        // Which components, not how many: this is the guard that stops
        // a caster selling what its own spells burn, and it must not be
        // switchable off by editing a shopping list. Any positive scale
        // gives the same set of keys, so it is given one of its own
        // rather than the number of tapers the player happens to keep.
        //
        // A guard, not a verdict. It answers for a component the
        // profile said nothing about; one a loot rule tagged to sell
        // is the player's word, which is not a shopping list, and goes
        // (`ac_loot::sale::offer_to_vendor` reads the tag first).
        const ENOUGH_TO_NAME_THEM: u32 = 1_000;
        self.component_targets(ENOUGH_TO_NAME_THEM)
            .keys()
            .filter_map(|id| mapper.component_wcid(*id))
            .collect()
    }
}
