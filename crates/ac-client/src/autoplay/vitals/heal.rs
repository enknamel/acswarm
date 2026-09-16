use std::time::Instant;

use crate::autoplay::{cast_problem, Doing, Survive};
use crate::Client;

/// Health left below which there is no time to be careful: the biggest
/// heal goes out at once, whatever it costs and whatever it drains.
const CRITICAL_HEALTH: f32 = 0.25;

/// A heal picked to cover a known wound has to be likely to land. Half
/// is where the school's skill equals the spell's power; under that the
/// mana is as likely to be thrown away as spent.
const RELIABLE_CAST: f32 = 0.5;

/// One self heal the character could cast this moment, weighed for what
/// it would give *this* character.
///
/// A boost restores the same however hurt it is; a transfer's gain is a
/// share of the bar it draws from, so the same Stamina to Health is
/// worth two hundred points on a full bar and nothing on an empty one.
/// The chooser cannot know that from the spell alone, so it is worked
/// out before it gets here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelfHeal {
    pub spell: u32,
    /// Health points it would restore, now.
    pub gain: u32,
    /// What the cast costs in mana.
    pub mana: u32,
    /// How likely it is to land rather than fizzle.
    pub chance: f32,
    /// For a transfer, the vital it draws on and the fraction of that
    /// bar left afterwards. `None` for a boost, which draws on nothing.
    pub leaves: Option<(u32, f32)>,
}

impl SelfHeal {
    /// Health this cast is worth on average: what it gives, times how
    /// often it lands.
    fn worth(&self) -> f32 {
        self.gain as f32 * self.chance
    }
}

/// Which of the heals that can be cast right now to cast, for a
/// character `missing` health points with `health` of its bar left.
///
/// A scratch should not be healed with the biggest spell in the book:
/// the mana that goes into it is the mana the next real wound needs.
/// So the cheapest heal that covers what is actually missing wins, and
/// only when nothing covers it -- or there is no time left to be
/// careful -- does the biggest go out.
fn choose_heal(heals: &[SelfHeal], missing: u32, health: f32, cfg: &Survive) -> Option<u32> {
    use ac_world::vitals::vital;
    // Dying beats saving a bar: at a quarter of the bar the next hit is
    // the last one, so the most health one cast can give goes out --
    // out of everything castable, floors and all. The floors below are
    // what the stamina and the mana were being kept for, and a dead
    // character has no later to keep them for.
    if health <= CRITICAL_HEALTH {
        let everything: Vec<&SelfHeal> = heals.iter().collect();
        return biggest_heal(&everything).map(|h| h.spell);
    }
    // A transfer that empties the bar it draws on is a trade, not a
    // heal: it buys health with the mana the next heal needs or the
    // stamina that carries the fight. Left out of the reckoning while
    // anything else will do, and only put back when nothing else will.
    let floor = |v: u32| match v {
        vital::STAMINA => cfg.stamina_below,
        vital::MANA => cfg.mana_below,
        _ => 0.0,
    };
    let sparing: Vec<&SelfHeal> = heals
        .iter()
        .filter(|h| h.leaves.is_none_or(|(v, left)| left >= floor(v)))
        .collect();
    let pool: Vec<&SelfHeal> = if sparing.is_empty() {
        heals.iter().collect()
    } else {
        sparing
    };
    pool.iter()
        .copied()
        .filter(|h| h.chance >= RELIABLE_CAST && h.worth() >= missing as f32)
        // Cheapest in mana, then the smallest of the ones that cover:
        // the rest of the heal is spilt on a full bar.
        .min_by_key(|h| (h.mana, h.gain, h.spell))
        .or_else(|| biggest_heal(&pool))
        .map(|h| h.spell)
}

/// The most health there is to be had from a single cast; ties to the
/// cheaper spell.
fn biggest_heal<'a>(pool: &[&'a SelfHeal]) -> Option<&'a SelfHeal> {
    pool.iter().copied().max_by(|a, b| {
        a.worth()
            .total_cmp(&b.worth())
            .then(b.mana.cmp(&a.mana))
            .then(b.spell.cmp(&a.spell))
    })
}

/// Whether a spell puts health back: a health boost (Harm Self is a
/// boost below zero and is not one) or a transfer into health.
fn restores_health(spell: u32) -> bool {
    use ac_world::vitals::vital;
    ac_world::vitals::boost(spell).is_some_and(|b| b.vital == vital::HEALTH && b.restores())
        || ac_world::vitals::transfer(spell).is_some_and(|t| t.to == vital::HEALTH)
}

/// Where a vital's current and maximum sit on the stats block: health
/// 0, stamina 1, mana 2.
fn vital_slot(vital: u32) -> usize {
    match vital {
        ac_world::vitals::vital::STAMINA => 1,
        ac_world::vitals::vital::MANA => 2,
        _ => 0,
    }
}

impl Client {
    /// Health as a fraction of its maximum, 1.0 when unknown.
    pub fn health_fraction(&self) -> f32 {
        let stats = &self.world.stats;
        let max = stats.vital_max_current(0);
        if max == 0 {
            return 1.0;
        }
        stats.vitals[0].current as f32 / max as f32
    }

    /// The strongest known boost of a vital that can be cast right now,
    /// whatever it is called: Heal Self VI and Adja's Intervention are
    /// both health boosts, and the table says so where a name would not.
    fn best_boost(&self, vital: u32) -> Option<u32> {
        let table = self.assets.spell_table().ok()?;
        ac_world::vitals::boosts_of(vital)
            .into_iter()
            .filter(|b| self.world.stats.spells.contains(&b.spell))
            .filter(|b| table.get(b.spell).is_some_and(|s| s.is_self_targeted()))
            .filter(|b| {
                matches!(
                    self.can_cast(b.spell),
                    crate::magic::CastCheck::Ok | crate::magic::CastCheck::NoCaster
                )
            })
            .max_by_key(|b| table.get(b.spell).map(|s| s.power).unwrap_or(0))
            .map(|b| b.spell)
    }

    /// The strongest known self transfer from one vital into another
    /// that can be cast right now.
    fn best_transfer(&self, from: u32, to: u32) -> Option<u32> {
        let table = self.assets.spell_table().ok()?;
        ac_world::vitals::transfers_between(from, to)
            .into_iter()
            .filter(|t| self.world.stats.spells.contains(&t.spell))
            .filter(|t| table.get(t.spell).is_some_and(|s| s.is_self_targeted()))
            .filter(|t| {
                matches!(
                    self.can_cast(t.spell),
                    crate::magic::CastCheck::Ok | crate::magic::CastCheck::NoCaster
                )
            })
            .max_by_key(|t| table.get(t.spell).map(|s| s.power).unwrap_or(0))
            .map(|t| t.spell)
    }

    /// Every self heal the character could cast this moment, each
    /// weighed for what it would give it now (see [`SelfHeal`]).
    ///
    /// A mage has more ways out of an emergency than Heal Self, and
    /// they are not the same size. Heal Self restores a fixed number of
    /// points however hurt it is; Stamina to Health takes half the
    /// stamina bar, which on a character with a full bar is far more,
    /// and is why a caster in trouble reaches for it rather than a kit.
    /// Which is worth more depends on the moment, so all of them are
    /// worked out and [`choose_heal`] picks between them.
    fn self_heals(&self) -> Vec<SelfHeal> {
        use ac_world::vitals::vital;
        let mut out: Vec<SelfHeal> = ac_world::vitals::boosts_of(vital::HEALTH)
            .into_iter()
            .filter_map(|b| self.self_heal(b.spell, ((b.low + b.high) / 2).max(0) as u32, None))
            .collect();
        // Both transfers into health, not only the stamina one: a mage
        // out of stamina with mana to spare has Mana to Health, and a
        // character that never looked at it stood there and died.
        for from in [vital::STAMINA, vital::MANA] {
            let have = self.world.stats.vitals[vital_slot(from)].current;
            for t in ac_world::vitals::transfers_between(from, vital::HEALTH) {
                if let Some(h) = self.self_heal(t.spell, t.gain(have), Some((from, t.drain(have))))
                {
                    out.push(h);
                }
            }
        }
        out
    }

    /// One heal, if the character knows it, can aim it at itself, can
    /// cast it this moment and would get anything out of it. `draws` is
    /// the vital a transfer takes from and how many points it takes.
    fn self_heal(&self, spell: u32, gain: u32, draws: Option<(u32, u32)>) -> Option<SelfHeal> {
        use ac_world::vitals::vital;
        if gain == 0 || !self.world.stats.spells.contains(&spell) {
            return None;
        }
        let sp = self.spell(spell)?;
        if !sp.is_self_targeted() || !matches!(self.can_cast(spell), crate::magic::CastCheck::Ok) {
            return None;
        }
        let mana = self.mana_cost(&sp);
        let leaves = draws.map(|(v, taken)| {
            let slot = vital_slot(v);
            let have = self.world.stats.vitals[slot].current;
            let max = self.world.stats.vital_max_current(slot).max(1);
            // Mana to Health is paid for out of the bar it drains, so
            // the cast's own cost comes off as well; a reckoning that
            // left it out called a transfer safe that ends with nothing
            // to cast the next heal with.
            let taken = taken + if v == vital::MANA { mana } else { 0 };
            (v, have.saturating_sub(taken) as f32 / max as f32)
        });
        Some(SelfHeal {
            spell,
            gain,
            mana,
            chance: self.cast_chance(spell),
            leaves,
        })
    }

    /// Why not one heal in the book can be cast, in a few words for the
    /// log. The easiest one known answers it best: the strongest
    /// usually complains only that its own components have run out,
    /// which says nothing about the rest of the book.
    fn why_no_heal(&self) -> Option<String> {
        let table = self.assets.spell_table().ok()?;
        let (spell, name) = self
            .world
            .stats
            .spells
            .iter()
            .filter(|id| restores_health(**id))
            .filter_map(|id| table.get(*id).map(|s| (*id, s)))
            .filter(|(_, s)| s.is_self_targeted())
            .min_by_key(|(_, s)| s.power)
            .map(|(id, s)| (id, s.name.clone()))?;
        Some(format!("{name} {}", cast_problem(&self.can_cast(spell))))
    }

    /// Keep mana and stamina up the way a caster does: stamina poured
    /// into mana when mana runs low, Revitalize when stamina does. True
    /// when a spell went out.
    pub(crate) fn autoplay_vitals(&mut self, now: Instant) -> bool {
        use ac_world::vitals::vital;
        let cfg = self.autoplay.config.survive.clone();
        if !cfg.manage_mana {
            return false;
        }
        // Same again: the next draught or cast waits on the server
        // answering for the last, not on a clock.
        if self.autoplay.cast_in_flight(now) {
            return false;
        }
        // A top-up is a cast that can wait, and at a counter it does,
        // with the buffs (see `counter_holding_casts`). This is the
        // pass that runs every tick and casts the moment the last cast
        // lands, so a transfer thrown from the counter never left it a
        // gap to open its window in: the same failure as the buffs,
        // one reflex over.
        if self.counter_holding_casts().is_some() {
            return false;
        }
        let frac = |i: usize| {
            let max = self.world.stats.vital_max_current(i).max(1) as f32;
            self.world.stats.vitals[i].current as f32 / max
        };
        let (stamina, mana) = (frac(1), frac(2));
        // Stamina first: it is what mana is made from, and Revitalize is
        // cheap next to what a transfer of a full bar returns.
        if stamina < cfg.stamina_below {
            if let Some(spell) = self.best_boost(vital::STAMINA) {
                if matches!(self.can_cast(spell), crate::magic::CastCheck::Ok) {
                    self.cast_paced(spell, now);
                    self.autoplay.last_vital = Some(now);
                    self.autoplay.say(
                        Doing::Buffing,
                        format!("restoring stamina at {:.0}%", stamina * 100.0),
                    );
                    return true;
                }
            }
        }
        if mana < cfg.mana_below && stamina >= cfg.stamina_below.max(0.5) {
            if let Some(spell) = self.best_transfer(vital::STAMINA, vital::MANA) {
                if matches!(self.can_cast(spell), crate::magic::CastCheck::Ok) {
                    self.cast_paced(spell, now);
                    self.autoplay.last_vital = Some(now);
                    self.autoplay.say(
                        Doing::Buffing,
                        format!("pouring stamina into mana at {:.0}%", mana * 100.0),
                    );
                    return true;
                }
            }
        }
        false
    }

    /// Heal, and break off a losing fight. True when it acted.
    pub(crate) fn autoplay_survive(&mut self, now: Instant) -> bool {
        let cfg = self.autoplay.config.survive.clone();
        let health = self.health_fraction();
        if health >= cfg.heal_below || health <= 0.0 {
            return false;
        }
        // Paced by the server, not by a clock: the next heal goes out
        // as soon as the last one is answered for. A fixed interval is
        // either too slow to save a character or quick enough to have
        // the spell dropped for arriving over the last.
        if self.autoplay.cast_in_flight(now) {
            // Waiting for the next heal is not a reason to stand
            // still. Everything below this in the list -- looting,
            // walking, tidying -- carries on; it is only the fighting
            // that must not go first, and the fight rule sees to that
            // itself (`too_hurt_to_fight`). Holding the tick here
            // instead left the character idle between heals.
            return false;
        }
        // A kit is quicker and cheaper than a spell -- but only to
        // someone who has trained Healing. Untrained it restores next to
        // nothing, so a caster is far better off with Heal Self, and
        // reaching for a kit only wastes the moment it takes.
        if cfg.use_kits && self.heals_with_kits() {
            let kit = self
                .world
                .inventory()
                .filter(|o| ac_world::usable::on_self(o.usable) && o.name.contains("Healing Kit"))
                .map(|o| o.guid)
                .next();
            if let Some(kit) = kit {
                let me = self.world.player_guid.unwrap_or(0);
                self.remember_journey();
                self.use_on(kit, me);
                // A kit is answered like a cast, and waited on like one.
                self.autoplay.cast_sent = Some(now);
                self.autoplay.last_heal = Some(now);
                self.autoplay
                    .say(Doing::Healing, format!("healing at {:.0}%", health * 100.0));
                return true;
            }
        }
        // Which heal is the right one is not a setting: it is this
        // moment's answer, and it changes with every point of damage.
        // A scratch takes the cheapest spell that covers it; a wound
        // that will kill takes the biggest in the book (see
        // [`choose_heal`]).
        let missing = self
            .world
            .stats
            .vital_max_current(0)
            .saturating_sub(self.world.stats.vitals[0].current);
        if let Some(spell) = choose_heal(&self.self_heals(), missing, health, &cfg) {
            self.cast_paced(spell, now);
            self.autoplay.last_heal = Some(now);
            self.autoplay
                .say(Doing::Healing, format!("healing at {:.0}%", health * 100.0));
            return true;
        }
        // Hurt and unable to heal is worth saying out loud.
        if let Some(why) = self.why_no_heal() {
            self.autoplay
                .note(format!("cannot heal at {:.0}%: {why}", health * 100.0), now);
        }
        false
    }

    /// Whether a healing kit is worth using: Healing has to be trained
    /// or specialised for one to restore anything much. A character
    /// without it heals with a spell instead, and does not want kits
    /// bought for it either (see `grow_needs`).
    pub fn heals_with_kits(&self) -> bool {
        use ac_world::stats::{sac, skill};
        self.world
            .stats
            .skill(skill::HEALING)
            .is_some_and(|s| s.advancement >= sac::TRAINED)
    }

    /// The maximum of a vital (0 health, 1 stamina, 2 mana).
    pub(super) fn vital_max_of(&self, i: usize) -> u32 {
        self.world.stats.vital_max_current(i)
    }
}

#[cfg(test)]
mod tests;
