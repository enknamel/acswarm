use std::time::Instant;

use ac_world::elements::Element;

use crate::autoplay::{Autoplay, Doing};
use crate::{Client, Stance};

/// The Magic Defense skill id, which Magic Yield lowers.
const MAGIC_DEFENSE: u32 = 15;

impl Client {
    /// Whether anyone on the team, this character included, has
    /// softened `guid`.
    pub(crate) fn softened_by_anyone(&self, guid: u32) -> bool {
        self.autoplay.debuffed.contains(&guid)
            || self
                .autoplay
                .team
                .mates
                .iter()
                .any(|m| m.debuffed.contains(&guid))
    }

    /// This character's Life Magic as it stands (skill 33).
    pub fn life_magic(&self) -> u32 {
        self.skill_now(33)
    }

    /// Knows a vulnerability or an imperil it could cast right now (a
    /// wand not in hand does not count against it).
    pub fn can_soften(&self) -> bool {
        let castable = |id: &u32| {
            matches!(
                self.can_cast_well(*id),
                crate::magic::CastCheck::Ok | crate::magic::CastCheck::NoCaster
            )
        };
        self.imperil_spells().iter().any(castable)
            || ac_world::elements::ALL
                .into_iter()
                .flat_map(ac_world::elements::vulnerabilities)
                .filter(|id| self.world.stats.spells.contains(id))
                .any(|id| castable(&id))
    }

    /// Whether this character fights with a weapon: an imperil lowers armour, which spells ignore.
    /// Told to fight with whatever suits, a war mage is one with attack spells and no weapon in hand.
    pub fn hits_with_weapons(&self) -> bool {
        match self.autoplay.config.fight.style {
            crate::autoplay::Style::Melee | crate::autoplay::Style::Missile => true,
            crate::autoplay::Style::Magic => false,
            crate::autoplay::Style::Auto => {
                self.combat_stance() != Stance::Magic || self.attack_spells_known().is_empty()
            }
        }
    }

    /// The imperils known: spells cast on another that lower its
    /// armour. Found by effect, so level eight's "Incantation of
    /// Imperil Other" counts without its name being read.
    pub fn imperil_spells(&self) -> Vec<u32> {
        let Ok(table) = self.assets.spell_table() else {
            return Vec::new();
        };
        self.world
            .stats
            .spells
            .iter()
            .copied()
            .filter(|id| {
                ac_world::buffs::effect(*id)
                    .is_some_and(|e| e.kind() == ac_world::buffs::kind::BODY_ARMOR && e.value < 0.0)
            })
            .filter(|id| table.get(*id).is_some_and(|s| s.needs_target()))
            .collect()
    }

    /// The Magic Yields known: spells cast on another that lower its Magic Defense (skill 15),
    /// found by effect, so Futility and the Incantation count without their names being read.
    pub(crate) fn magic_yield_spells(&self) -> Vec<u32> {
        let Ok(table) = self.assets.spell_table() else {
            return Vec::new();
        };
        self.world
            .stats
            .spells
            .iter()
            .copied()
            .filter(|id| {
                ac_world::buffs::effect(*id)
                    .is_some_and(|e| e.skill() == Some(MAGIC_DEFENSE) && e.value < 0.0)
            })
            .filter(|id| table.get(*id).is_some_and(|s| s.needs_target()))
            .collect()
    }

    /// Of `ids`, the strongest this character knows, can cast well, and
    /// that takes a target. Levels come from power, never from names.
    pub(crate) fn strongest_castable(&self, ids: &[u32]) -> Option<u32> {
        let table = self.assets.spell_table().ok()?;
        let mut known: Vec<(u32, u32)> = ids
            .iter()
            .copied()
            .filter(|id| self.world.stats.spells.contains(id))
            .filter_map(|id| {
                table
                    .get(id)
                    .filter(|s| s.needs_target())
                    .map(|s| (id, s.power))
            })
            .collect();
        // Strongest first, so the packs are walked for components only until one will do.
        known.sort_by_key(|(_, power)| std::cmp::Reverse(*power));
        known.into_iter().map(|(id, _)| id).find(|id| {
            matches!(
                self.can_cast_well(*id),
                crate::magic::CastCheck::Ok | crate::magic::CastCheck::NoCaster
            )
        })
    }

    /// The strongest vulnerability for `e` known, and whether it can be cast well: its chance and
    /// multiplier, or what stops it (`cast_problem`); or that none is known.
    fn vuln_standing(&self, e: Element) -> String {
        let table = self.assets.spell_table().ok();
        let spell = |id: u32| table.as_ref().and_then(|t| t.get(id));
        let strongest = crate::weapons::vulnerability_spells(e)
            .into_iter()
            .filter(|id| self.world.stats.spells.contains(id))
            .filter(|id| spell(*id).is_some_and(|s| s.needs_target()))
            .max_by_key(|id| spell(*id).map_or(0, |s| s.power));
        let Some(id) = strongest else {
            return format!("no {} vulnerability known", e.name());
        };
        let name = spell(id).map(|s| s.name.clone()).unwrap_or_default();
        let chance = self.cast_chance(id) * 100.0;
        match self.can_cast_well(id) {
            crate::magic::CastCheck::Ok => format!(
                "{name} (x{}, lands {chance:.0}%)",
                ac_world::elements::vuln_mod(id).unwrap_or(1.0)
            ),
            check => format!(
                "{name} lands {chance:.0}%, {}",
                crate::autoplay::cast_problem(&check)
            ),
        }
    }

    /// Say the fastest kill of `guid` when it changes: the chosen way, the next ones, and each
    /// element's vulnerability as it stands.
    fn say_kill(
        &mut self,
        guid: u32,
        name: &str,
        kill: &crate::autoplay::fight::kill::Kill,
        now: Instant,
    ) {
        let key = (
            guid,
            kill.wand.guid,
            kill.attack,
            kill.vuln.map(|(_, id, ..)| id),
            kill.magic_yield.map(|(id, ..)| id),
        );
        if self.autoplay.kill_said == Some(key) {
            return;
        }
        self.autoplay.kill_said = Some(key);
        let cfg = self.autoplay.config.fight.clone();
        let ways = self.kills_said(guid, &cfg, 4);
        let mut elements: Vec<Element> = Vec::new();
        for (id, _) in self.ready_spells(&cfg) {
            if let Some(e) = ac_world::elements::spell_element(id) {
                if !elements.contains(&e) {
                    elements.push(e);
                }
            }
        }
        let standing: Vec<String> = elements
            .into_iter()
            .map(|e| format!("{}: {}", e.name(), self.vuln_standing(e)))
            .collect();
        let chosen = kill.said(|id| self.spell(id).map(|s| s.name).unwrap_or_default());
        self.autoplay.note(
            format!(
                "fastest kill of {name}: {chosen} (of {ways}); vulnerabilities {}",
                standing.join("; ")
            ),
            now,
        );
    }

    /// Cast the Magic Yield, else the vulnerability, the fastest kill of `guid` puts on first, if
    /// it has one. True when a spell went out this tick; a resist or a fizzle has it cast again
    /// (`hear_debuff_resisted`).
    pub(super) fn autoplay_make_vulnerable(
        &mut self,
        guid: u32,
        name: &str,
        kill: Option<&crate::autoplay::fight::kill::Kill>,
        now: Instant,
    ) -> bool {
        let Some(kill) = kill else {
            return false;
        };
        self.say_kill(guid, name, kill, now);
        if let Some((spell, by, _)) = kill.magic_yield {
            let level = self.spell(spell).map_or(0, |sp| sp.level());
            if self.cast_paced(spell, now) {
                self.autoplay.yield_sent = Some(guid);
            }
            self.autoplay
                .mark_yielded(guid, by + self.autoplay.yielded_by(guid));
            *self.autoplay.kill_kept.borrow_mut() = None;
            let said = format!("lowering {name}'s magic defense by {by:.0} (level {level})");
            self.autoplay.say(Doing::Debuffing, said);
            return true;
        }
        let Some((element, vuln, m, _)) = kill.vuln else {
            return false;
        };
        let level = self.spell(vuln).map_or(0, |sp| sp.level());
        if self.cast_paced(vuln, now) {
            self.autoplay.debuff_sent = Some(guid);
        }
        self.autoplay.mark_vulned(guid, Some((element, m)));
        self.autoplay.last_vuln = Some(now);
        *self.autoplay.kill_kept.borrow_mut() = None;
        let said = format!(
            "making {name} vulnerable to {} (level {level})",
            element.name()
        );
        self.autoplay.say(Doing::Debuffing, said);
        true
    }
}

impl Autoplay {
    /// How much magic defence the Magic Yield cast on `guid` took off, 0 for none.
    pub(crate) fn yielded_by(&self, guid: u32) -> f32 {
        self.yielded
            .iter()
            .find(|(g, _)| *g == guid)
            .map_or(0.0, |(_, by)| *by)
    }

    /// Note Magic Yield cast on `guid`, taking `by` off its magic defence.
    pub(crate) fn mark_yielded(&mut self, guid: u32, by: f32) {
        if self.yielded.len() > 64 {
            self.yielded.drain(..32);
        }
        self.yielded.retain(|(g, _)| *g != guid);
        self.yielded.push((guid, by));
    }

    /// Whether a vulnerability on `guid` is cast and not heard missed.
    #[cfg(test)]
    pub(crate) fn is_vulned(&self, guid: u32) -> bool {
        self.vulned.iter().any(|(g, _)| *g == guid)
    }

    /// Note `guid`'s vulnerability dealt with, and the one cast on it (element, multiplier) if any.
    pub(crate) fn mark_vulned(&mut self, guid: u32, cast: Option<(Element, f32)>) {
        // Only the recent ones are worth remembering, not every creature of a long session.
        if self.vulned.len() > 64 {
            self.vulned.drain(..32);
        }
        self.vulned.retain(|(g, _)| *g != guid);
        self.vulned.push((guid, cast));
    }
}

impl Client {
    /// The health `guid` has left: its full health, appraised or from the creature table, times
    /// the fraction the server last sent (full when none has come). `None` with no full health.
    pub(crate) fn health_now(&self, guid: u32) -> Option<f32> {
        let full = self
            .appraisals
            .get(&guid)
            .and_then(|a| a.creature.as_ref())
            .map(|c| c.health_max)
            .filter(|h| *h > 0)
            .or_else(|| {
                self.creature_known(guid)
                    .map(|c| c.health)
                    .filter(|h| *h > 0)
            })?;
        let fraction = self
            .world
            .objects
            .get(&guid)
            .and_then(|o| o.health)
            .unwrap_or(1.0);
        Some(full as f32 * fraction)
    }
}

#[cfg(test)]
mod tests;
