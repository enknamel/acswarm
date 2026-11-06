use std::time::Instant;

use ac_world::elements::Element;

use crate::autoplay::{Autoplay, Doing};
use crate::{Client, Stance};

impl Client {
    /// A hard fight, for the team to soften: one whose fastest kill with the wand in hand puts a
    /// vulnerability on first (see `kill`). With no team, the fight vulns it alone.
    pub(crate) fn is_hard_fight(&self, guid: u32) -> bool {
        if !self.autoplay.config.team.enabled {
            return false;
        }
        let cfg = self.autoplay.config.fight.clone();
        let ready: Vec<u32> = self.ready_spells(&cfg).iter().map(|(id, _)| *id).collect();
        self.kill_in_hand(guid, &ready)
            .is_some_and(|k| k.vuln.is_some())
    }

    /// The name of whoever should soften a hard target: the one with
    /// the highest Life Magic of those who can, this character
    /// included. Every session works this out from the same roster,
    /// so they agree without a word.
    fn softener(&self) -> Option<String> {
        let mine = (
            self.world.stats.name.clone(),
            self.life_magic(),
            self.can_soften(),
        );
        let best = self
            .autoplay
            .team
            .mates
            .iter()
            .map(|m| (m.name.clone(), m.life_magic, m.can_soften))
            .chain(std::iter::once(mine))
            .filter(|(_, _, can)| *can)
            .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)));
        best.map(|(name, _, _)| name)
    }

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

    /// Whether an imperil on `guid` helps anyone: this character or a mate on it hits with a weapon.
    fn imperil_helps(&self, guid: u32) -> bool {
        self.hits_with_weapons()
            || self
                .autoplay
                .team
                .mates
                .iter()
                .any(|m| m.hits_with_weapons && (m.target == Some(guid) || m.in_fellowship))
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

    /// The team's plan for a hard target. True when this tick was spent
    /// on it: either softening it, because that is this character's
    /// job, or holding fire while a teammate does. False when the fight
    /// may go ahead: an ordinary target, or a hard one already softened.
    pub(super) fn autoplay_plan_hard(&mut self, guid: u32, name: &str, now: Instant) -> bool {
        if !self.is_hard_fight(guid) || self.softened_by_anyone(guid) {
            return false;
        }
        let me = self.world.stats.name.clone();
        match self.softener() {
            Some(who) if who == me => {
                // Ours to soften: a vulnerability for its weakest element
                // and an imperil, then it is marked and the others go.
                if self.autoplay_soften(guid, name, now) {
                    return true;
                }
                // Nothing castable right now: do not hold everyone up.
                if !self.autoplay.debuffed.contains(&guid) {
                    self.autoplay.debuffed.push(guid);
                }
                false
            }
            Some(who) => {
                if !self.autoplay.config.team.wait_for_debuff {
                    return false;
                }
                self.autoplay.say(
                    Doing::Helping,
                    format!("waiting for {who} to soften {name}"),
                );
                true
            }
            // Nobody can: fight it as it is.
            None => false,
        }
    }

    /// Land the vulnerability and the imperil on `guid`, one cast a
    /// tick, and mark it softened when both are on (or neither can be
    /// cast). True while there is still one to cast.
    fn autoplay_soften(&mut self, guid: u32, name: &str, now: Instant) -> bool {
        // Waiting on the server's answer, not on a clock.
        if self.autoplay.cast_in_flight(now) {
            return true;
        }
        let stage = self
            .autoplay
            .softening
            .iter()
            .find(|(g, _)| *g == guid)
            .map(|(_, s)| *s)
            .unwrap_or(0);
        // Stage 0: the vulnerability. Stage 1: the imperil.
        let element = (stage == 0).then(|| self.vuln_element(guid)).flatten();
        let candidates: Vec<u32> = if stage == 0 {
            element
                .map(ac_world::elements::vulnerabilities)
                .unwrap_or_default()
        } else if self.imperil_helps(guid) {
            self.imperil_spells()
        } else {
            Vec::new()
        };
        let spell = self.strongest_castable(&candidates);
        let advance = |this: &mut Self| {
            this.autoplay.softening.retain(|(g, _)| *g != guid);
            if stage == 0 {
                this.autoplay.softening.push((guid, 1));
            } else if !this.autoplay.debuffed.contains(&guid) {
                this.autoplay.debuffed.push(guid);
            }
        };
        match spell {
            Some(spell) => {
                if self.combat_stance() != Stance::Magic {
                    // Not with a swing out: putting the weapon away to
                    // reach for a wand cancels it. The softening keeps;
                    // hit the thing meanwhile.
                    if self.mid_attack() {
                        self.wait_for_the_swing();
                        return false;
                    }
                    // A wand for the casting; the arming code sorts the
                    // hands out again when the fight proper begins.
                    if !self.wield_for(Stance::Magic) {
                        // No caster to be had -- none carried, or the
                        // server keeps refusing the one there is. Held
                        // fire every tick this went on for ever and the
                        // target was never hit. Mark it and fight it as
                        // it is.
                        advance(self);
                        return false;
                    }
                    return true;
                }
                self.select(Some(guid));
                if self.cast_paced(spell, now) {
                    self.autoplay.debuff_sent = Some((guid, stage));
                }
                let what = if stage == 0 {
                    "vulnerability"
                } else {
                    "imperil"
                };
                self.autoplay
                    .say(Doing::Debuffing, format!("softening {name}: {what}"));
                if stage == 0 {
                    let cast = element.zip(ac_world::elements::vuln_mod(spell));
                    self.autoplay.mark_vulned(guid, cast);
                }
                advance(self);
                stage == 0
            }
            None => {
                // Nothing for this stage: on to the next, or done.
                advance(self);
                stage == 0 && self.imperil_helps(guid) && !self.imperil_spells().is_empty()
            }
        }
    }

    /// The element a vulnerability on `guid` should be for: a caster's, that of the fastest kill
    /// ([`Self::best_kill`]); a weapon's, what the creature is weakest to.
    fn vuln_element(&self, guid: u32) -> Option<Element> {
        let known = self.creature_known(guid);
        if self.hits_with_weapons() {
            return known.and_then(|c| c.weakest_to());
        }
        let cfg = self.autoplay.config.fight.clone();
        let kill = self.best_kill(guid, &cfg, Instant::now())?;
        kill.vuln
            .map(|(e, ..)| e)
            .or_else(|| ac_world::elements::spell_element(kill.attack))
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

    /// Cast the vulnerability the fastest kill of `guid` puts on first, if it has one. True when a
    /// spell went out this tick; a resist or a fizzle has it cast again (`hear_debuff_resisted`).
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
        let Some((element, vuln, m, _)) = kill.vuln else {
            return false;
        };
        let level = self.spell(vuln).map_or(0, |sp| sp.level());
        if self.cast_paced(vuln, now) {
            self.autoplay.debuff_sent = Some((guid, 0));
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
