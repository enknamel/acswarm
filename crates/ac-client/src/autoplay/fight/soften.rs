use std::time::Instant;

use ac_world::elements::Element;

use crate::autoplay::{Autoplay, Doing};
use crate::{Client, Stance};

impl Client {
    /// A hard fight: the creature has at least the team's threshold of
    /// health. With no team, nothing is hard in this sense (the solo
    /// rules soften by their own threshold).
    pub(crate) fn is_hard_fight(&self, guid: u32) -> bool {
        let team = &self.autoplay.config.team;
        if !team.enabled || team.hard_fight_health == 0 {
            return false;
        }
        self.creature_known(guid)
            .is_some_and(|c| c.health >= team.hard_fight_health)
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
    fn softened_by_anyone(&self, guid: u32) -> bool {
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

    /// The element a vulnerability on `guid` should be for: a caster's, that of the spell it will
    /// throw ([`Self::best_attack`], as the fight picks it); a weapon's, what the creature is weakest to.
    fn vuln_element(&self, guid: u32) -> Option<Element> {
        let known = self.creature_known(guid);
        if self.hits_with_weapons() {
            return known.and_then(|c| c.weakest_to());
        }
        let cfg = self.autoplay.config.fight.clone();
        let ids: Vec<u32> = self.ready_spells(&cfg).iter().map(|(id, _)| *id).collect();
        let vulns = self.vuln_strengths(&ids);
        let attack = self
            .best_attack(guid, &ids, &vulns)
            .and_then(|(id, _)| ac_world::elements::spell_element(id));
        crate::weapons::vulnerability_for(known, attack, 0)
    }

    /// Of `ids`, the strongest this character knows, can cast, and
    /// that takes a target. Levels come from power, never from names.
    fn strongest_castable(&self, ids: &[u32]) -> Option<u32> {
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

    /// The strongest vulnerability this character could cast for each element the spells `ready`
    /// deal: what it multiplies by and its chance to land; an element with none castable is left out.
    pub(crate) fn vuln_strengths(&self, ready: &[u32]) -> Vec<(Element, f32, f32)> {
        let mut out: Vec<(Element, f32, f32)> = Vec::new();
        for e in ready
            .iter()
            .filter_map(|id| ac_world::elements::spell_element(*id))
        {
            if out.iter().any(|(had, ..)| *had == e) {
                continue;
            }
            let strongest = self.strongest_castable(&ac_world::elements::vulnerabilities(e));
            if let Some(id) = strongest {
                if let Some(m) = ac_world::elements::vuln_mod(id) {
                    out.push((e, m, self.cast_chance(id)));
                }
            }
        }
        out
    }

    /// Of the attack spells `ready`, the one expected to take most off `guid` a cast, and what the
    /// creature takes from its element: the roll there times the chance it lands, times the
    /// vulnerability for that element put on it or to be ([`Self::vuln_boost`]). A spell with no
    /// damage row is worth its element's multiplier alone, under any that has one. Ties keep `ready`'s order.
    pub(crate) fn best_attack(
        &self,
        guid: u32,
        ready: &[u32],
        vulns: &[(Element, f32, f32)],
    ) -> Option<(u32, f32)> {
        let known = self.creature_known(guid);
        ready
            .iter()
            .map(|&id| {
                let element = ac_world::elements::spell_element(id);
                let takes = match (known, element) {
                    (Some(c), Some(e)) => c.takes_from(e),
                    _ => 1.0,
                };
                let roll = self.damage_roll(id, known);
                let boost = element.map_or(1.0, |e| self.vuln_boost(guid, e, roll, vulns));
                let per_cast = roll.map(|(lo, hi)| (lo + hi) / 2.0);
                let worth = per_cast.map_or(takes, |d| d * self.cast_chance(id)) * boost;
                (id, takes, worth)
            })
            .reduce(|best, next| if next.2 > best.2 { next } else { best })
            .map(|(id, takes, _)| (id, takes))
    }

    /// What a vulnerability multiplies `element` by on `guid`: the one this character cast there,
    /// else the strongest in `vulns` when the rules would cast it; 1 when neither.
    fn vuln_boost(
        &self,
        guid: u32,
        element: Element,
        roll: Option<(f32, f32)>,
        vulns: &[(Element, f32, f32)],
    ) -> f32 {
        if let Some((_, cast)) = self.autoplay.vulned.iter().find(|(g, _)| *g == guid) {
            return cast.filter(|(e, _)| *e == element).map_or(1.0, |(_, m)| m);
        }
        let Some(&(_, m, chance)) = vulns.iter().find(|(e, ..)| *e == element) else {
            return 1.0;
        };
        if self.would_vuln(guid, roll, m, chance) {
            m
        } else {
            1.0
        }
    }

    /// Whether this character would put a vulnerability multiplying by `m` on `guid`, each cast
    /// each cast of the attack rolling `roll`: as the team's softener of a hard fight, or when it pays.
    fn would_vuln(&self, guid: u32, roll: Option<(f32, f32)>, m: f32, chance: f32) -> bool {
        if self.softened_by_anyone(guid) {
            return false;
        }
        let me = self.world.stats.name.clone();
        if self.is_hard_fight(guid) && self.softener().is_some_and(|who| who == me) {
            return true;
        }
        let least = self.autoplay.config.fight.vuln_above_health;
        let full = self.creature_known(guid).map_or(0, |c| c.health);
        least != 0 && is_vuln_worth(self.health_now(guid), roll, m, chance, full, least)
    }

    /// Why no vulnerability goes on: none for `element` known or likely enough to land, or the one
    /// `cast` (element, spell, multiplier) not saving casts on `health` with the attack's `roll`.
    fn why_no_vuln(
        &self,
        element: Option<Element>,
        cast: Option<(Element, u32, f32)>,
        health: Option<f32>,
        roll: Option<(f32, f32)>,
    ) -> String {
        let table = self.assets.spell_table().ok();
        let spell = |id: u32| table.as_ref().and_then(|t| t.get(id));
        let name = |id: u32| spell(id).map(|s| s.name.clone()).unwrap_or_default();
        let Some(e) = element else {
            return "the attack has no element it takes".into();
        };
        let Some((_, id, m)) = cast else {
            return self.vuln_standing(e);
        };
        let chance = self.cast_chance(id);
        match (health, roll) {
            (Some(h), Some((lo, hi))) => format!(
                "{} (x{m}, lands {:.0}%) takes {:.1} casts with it, {:.1} without",
                name(id),
                chance * 100.0,
                1.0 / chance.max(0.01) + casts_to_kill(h, lo * m, hi * m),
                casts_to_kill(h, lo, hi)
            ),
            _ => format!("{} is not worth it on this one", name(id)),
        }
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

    /// Cast a vulnerability for the element of `spell`, the attack the fight would throw next, once
    /// per target and only when it pays (see [`is_vuln_worth`]). True when a spell went out this tick.
    pub(super) fn autoplay_make_vulnerable(
        &mut self,
        guid: u32,
        name: &str,
        spell: Option<u32>,
        now: Instant,
    ) -> bool {
        let least = self.autoplay.config.fight.vuln_above_health;
        if least == 0 || self.autoplay.is_vulned(guid) || self.softened_by_anyone(guid) {
            return false;
        }
        let known = self.creature_known(guid);
        let attack = spell.and_then(ac_world::elements::spell_element);
        // The strongest one we know and can pay for, cast on someone else (the self versions make
        // us take more). Levels come from the spell's power: level VII has names of its own.
        let element = crate::weapons::vulnerability_for(known, attack, 0);
        let cast = element.and_then(|e| {
            let id = self.strongest_castable(&crate::weapons::vulnerability_spells(e))?;
            Some((e, id, ac_world::elements::vuln_mod(id)?))
        });
        let roll = spell.and_then(|s| self.damage_roll(s, known));
        let full = known.map_or(0, |c| c.health);
        let health = self.health_now(guid);
        let worth = |(_, id, m): &(Element, u32, f32)| {
            is_vuln_worth(health, roll, *m, self.cast_chance(*id), full, least)
        };
        let Some((element, vuln, m)) = cast.filter(worth) else {
            // Nothing castable, or not worth its cast: say why, and do not ask again for this one.
            let why = self.why_no_vuln(element, cast, health, roll);
            // And the other elements on offer, so a choice between them can be read off the note.
            let cfg = self.autoplay.config.fight.clone();
            let mut others: Vec<Element> = Vec::new();
            for (id, _) in self.ready_spells(&cfg) {
                if let Some(e) = ac_world::elements::spell_element(id) {
                    if Some(e) != element && !others.contains(&e) {
                        others.push(e);
                    }
                }
            }
            let others: Vec<String> = others
                .into_iter()
                .map(|e| format!("{}: {}", e.name(), self.vuln_standing(e)))
                .collect();
            let also = if others.is_empty() {
                String::new()
            } else {
                format!("; {}", others.join("; "))
            };
            self.autoplay
                .note(format!("no vulnerability on {name}: {why}{also}"), now);
            self.autoplay.mark_vulned(guid, None);
            return false;
        };
        let level = self
            .assets
            .spell_table()
            .ok()
            .and_then(|t| t.get(vuln).map(|sp| sp.level()))
            .unwrap_or(0);
        if self.cast_paced(vuln, now) {
            self.autoplay.debuff_sent = Some((guid, 0));
        }
        self.autoplay.mark_vulned(guid, Some((element, m)));
        self.autoplay.last_vuln = Some(now);
        let said = format!(
            "making {name} vulnerable to {} (level {level})",
            element.name()
        );
        self.autoplay.say(Doing::Debuffing, said);
        true
    }
}

/// Whether a vulnerability multiplying the attack by `m`, landing `chance` of its casts, pays on a
/// target with `health` left that each cast takes `roll` (lo, hi) off: when it saves casts on
/// average, its own tries counted. With either unknown (no health known, no damage row), by `least`,
/// the floor on its `full` health.
pub(crate) fn is_vuln_worth(
    health: Option<f32>,
    roll: Option<(f32, f32)>,
    m: f32,
    chance: f32,
    full: u32,
    least: u32,
) -> bool {
    match (health, roll) {
        (Some(h), Some((lo, hi))) if hi > 0.0 => {
            let tries = 1.0 / chance.max(0.01);
            casts_to_kill(h, lo, hi) > tries + casts_to_kill(h, lo * m, hi * m)
        }
        _ => least > 0 && full >= least,
    }
}

/// Casts taken whole, up to which [`casts_to_kill`] sums the chances exactly.
const EXACT_CASTS: u32 = 12;

/// The casts it takes on average to take `health` off, each rolling evenly from `lo` to `hi`: the
/// sum over k of the chance k rolls leave it standing (Irwin-Hall); past [`EXACT_CASTS`] casts'
/// worth, the renewal estimate `h/mean + 1/2 + width²/(24 mean²)`.
pub(crate) fn casts_to_kill(health: f32, lo: f32, hi: f32) -> f32 {
    let mean = (lo + hi) / 2.0;
    if mean <= 0.0 {
        return f32::INFINITY;
    }
    let width = (hi - lo).max(0.0);
    if health / mean > EXACT_CASTS as f32 {
        return health / mean + 0.5 + width * width / (24.0 * mean * mean);
    }
    if width < 1e-3 {
        return (health / mean).ceil().max(1.0);
    }
    // No cast at all leaves it standing.
    let mut casts = 1.0;
    for k in 1..=2 * EXACT_CASTS {
        let x = (health - k as f32 * lo) / width;
        if x <= 0.0 {
            break;
        }
        casts += irwin_hall(k, f64::from(x)) as f32;
    }
    casts
}

/// The chance that `n` rolls each even on 0 to 1 sum under `x`.
fn irwin_hall(n: u32, x: f64) -> f64 {
    if x >= f64::from(n) {
        return 1.0;
    }
    let (mut sum, mut choose) = (0.0, 1.0);
    for j in 0..=(x.floor() as u32) {
        let sign = if j % 2 == 0 { 1.0 } else { -1.0 };
        sum += sign * choose * (x - f64::from(j)).powi(n as i32);
        choose *= f64::from(n - j) / f64::from(j + 1);
    }
    let factorial: f64 = (1..=n).map(f64::from).product();
    (sum / factorial).clamp(0.0, 1.0)
}

impl Autoplay {
    /// Whether a vulnerability on `guid` is dealt with: cast, or judged not worth it.
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

/// What one cast takes off a target, crits aside (ACE SpellProjectile.cs:528-557), lowest and
/// highest: a roll from `min` to `max` (both ends, ThreadSafeRandom.cs:28-31), plus `min` times a
/// thousandth of each point `skill` stands over the spell's `power`, times the wand's bonus and `takes`.
pub(crate) fn cast_roll(
    min: u32,
    max: u32,
    skill: u32,
    power: u32,
    wand: f32,
    takes: f32,
) -> (f32, f32) {
    let bonus = min as f32 * skill.saturating_sub(power) as f32 / 1000.0;
    let k = wand * takes;
    ((min as f32 + bonus) * k, (max as f32 + bonus) * k)
}

impl Client {
    /// The least and most a cast of `spell` takes off `creature` (see [`cast_roll`]); `None` for
    /// a spell with no damage row. The wand's bonus counts only on its own element
    /// (WorldObject_Weapon.cs:382-400); enchantments on it are left out.
    pub(crate) fn damage_roll(
        &self,
        spell: u32,
        creature: Option<&ac_world::elements::Creature>,
    ) -> Option<(f32, f32)> {
        let (min, max) = ac_world::elements::spell_damage(spell)?;
        let table = self.assets.spell_table().ok();
        let power = table
            .as_ref()
            .and_then(|t| t.get(spell))
            .map_or(0, |sp| sp.power);
        let element = ac_world::elements::spell_element(spell);
        let wand = self
            .wielded_caster()
            .and_then(|g| self.stats_of(g))
            .filter(|w| element.is_some_and(|e| w.damage_type_bits == e as u32))
            .map(|w| w.elemental_damage)
            .filter(|m| *m > 0.0)
            .unwrap_or(1.0);
        let takes = match (creature, element) {
            (Some(c), Some(e)) => c.takes_from(e),
            _ => 1.0,
        };
        Some(cast_roll(
            min,
            max,
            self.casting_skill(spell),
            power,
            wand,
            takes,
        ))
    }

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
