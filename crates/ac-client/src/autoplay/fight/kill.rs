//! A caster's way to a kill: of the wands to hand, the attack spells ready, per element each
//! vulnerability level castable well and Magic Yield before it, the one taking fewest casts on
//! average, every damage step ACE takes counted (SpellProjectile.cs:494-557): roll, skill bonus,
//! crits, the wand's element bonus, resistance times the larger of vulnerability and rend; and
//! each spell's resist roll against the creature's magic defence (WorldObject_Magic.cs:104).

use std::time::{Duration, Instant};

use ac_world::elements::{imbue, Element};

use crate::autoplay::Fight;
use crate::Client;

/// Crit chance with nothing better, for magic (WorldObject_Weapon.cs:324).
const BASE_CRIT: f32 = 0.05;
/// Imbues read base skill up to this (WorldObject_Weapon.cs:822).
const IMBUE_SKILL_CAP: u32 = 360;
/// Rending's divisor and cap: `clamp(base / 144, 1, 2.5)` (WorldObject_Weapon.cs:731, 735).
const REND_PER: f32 = 144.0;
const REND_CAP: f32 = 2.5;
/// A wand changed for costs the hands about a cast's time: put away, taken up (`take_up`).
const SWAP_CASTS: f32 = 1.0;
/// How long the fight's choice for its target is kept before it is worked out again.
const KILL_KEEPS: Duration = Duration::from_millis(500);
/// The resist roll's factor: `GetSkillChance`'s 0.03, not casting's 0.07 (WorldObject_Magic.cs:107).
const RESIST_FACTOR: f32 = 0.03;
/// PropertyInt ResistanceModifierType and PropertyFloat ResistanceModifier: a quest wand's cleave.
const CLEAVE_TYPE: u32 = 263;
const CLEAVE_MOD: u32 = 157;

/// What a wand brings to a spell (WorldObject_Weapon.cs:335-485).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Wand {
    pub guid: u32,
    pub name: String,
    pub held: bool,
    /// Its damage type bits (`W_DamageType`) and the elemental damage bonus on that element.
    pub element: u32,
    pub elemental: f32,
    /// Imbue bits (`ac_world::elements::imbue`).
    pub imbued: u32,
    /// Resistance cleaving: the element's bits and `1 + ResistanceModifier` (:469-470).
    pub cleave: Option<(u32, f32)>,
    /// Its own crit chance and crit damage multiplier, 0 when it has none.
    pub crit_frequency: f32,
    pub crit_multiplier: f32,
}

impl Wand {
    /// Bare hands: nothing added.
    fn none() -> Wand {
        Wand {
            guid: 0,
            name: String::new(),
            held: true,
            element: 0,
            elemental: 1.0,
            imbued: 0,
            cleave: None,
            crit_frequency: 0.0,
            crit_multiplier: 0.0,
        }
    }

    /// Multiplier on `e`'s damage from the wand's elemental bonus: on its own element only (:384-395).
    pub(crate) fn elemental_for(&self, e: Element) -> f32 {
        if self.element == e as u32 && self.elemental > 0.0 {
            self.elemental
        } else {
            1.0
        }
    }

    /// The wand's own resistance multiplier on `e` at `base` skill: rending or cleaving, the larger
    /// (:461-485). A vulnerability on the target does not stack with it: the larger counts.
    pub(crate) fn rend_for(&self, e: Element, base: u32) -> f32 {
        let cleave = self
            .cleave
            .filter(|(bits, _)| *bits == e as u32)
            .map_or(1.0, |(_, m)| m);
        if self.imbued & e.rending() == 0 {
            return cleave;
        }
        let rend = (base.min(IMBUE_SKILL_CAP) as f32 / REND_PER).clamp(1.0, REND_CAP);
        cleave.max(rend)
    }

    /// Crit chance and crit damage multiplier at `base` skill: Critical Strike raises the chance to
    /// `(base - 60) / 600`, Crippling Blow the multiplier to `base / 60` (:335-370, :630, :703).
    pub(crate) fn crits(&self, base: u32) -> (f32, f32) {
        let b = base.min(IMBUE_SKILL_CAP) as f32;
        let mut chance = if self.crit_frequency > 0.0 {
            self.crit_frequency
        } else {
            BASE_CRIT
        };
        if self.imbued & imbue::CRITICAL_STRIKE != 0 {
            chance = chance.max(((b - 60.0) / 600.0).max(BASE_CRIT));
        }
        let mut mult = if self.crit_multiplier > 0.0 {
            self.crit_multiplier
        } else {
            1.0
        };
        if self.imbued & imbue::CRIPPLING_BLOW != 0 {
            mult = mult.max((b / 60.0).max(1.0));
        }
        (chance.min(1.0), mult)
    }
}

/// A vulnerability castable well: its spell, multiplier and chance to land.
type Vuln = (u32, f32, f32);

/// What is already on a target, and whether more may go on: a vulnerability (element and
/// multiplier), whether one may be cast (nobody else has softened it), the magic defence yielded.
#[derive(Clone, Copy)]
struct Softened {
    on: Option<(Element, f32)>,
    may_vuln: bool,
    yielded: f32,
}
/// Magic Yield cast first: its spell, the magic defence it takes off, its chance to land.
pub(crate) type Yield = (u32, f32, f32);
/// A kill as last said: target, wand, attack, vulnerability, Magic Yield.
pub(crate) type KillSaid = (u32, u32, u32, Option<u32>, Option<u32>);

/// The chance a spell cast at `skill` is not resisted by `defence` (WorldObject_Magic.cs:104-113);
/// sure with no defence known.
pub(crate) fn unresisted(skill: u32, defence: Option<f32>) -> f32 {
    defence.map_or(1.0, |d| {
        let x = RESIST_FACTOR * (skill as f32 - d);
        (1.0 - 1.0 / (1.0 + x.exp())).clamp(0.0, 1.0)
    })
}

/// One way to kill a target: the wand, the attack and the vulnerability cast first if any.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Kill {
    pub wand: Wand,
    pub attack: u32,
    /// What the target takes from the attack: its resistance times the larger of vulnerability
    /// and rend, times the wand's element bonus.
    pub takes: f32,
    /// The vulnerability to cast first: element, spell, multiplier, chance to land.
    pub vuln: Option<(Element, u32, f32, f32)>,
    /// Magic Yield cast before it: spell, magic defence taken off, chance to land.
    pub magic_yield: Option<Yield>,
    /// Casts it takes on average, the swap and the vuln's tries counted; `None` with no health known.
    pub casts: Option<f32>,
    /// Damage a cast does on average, fizzles and resists counted.
    pub per_cast: f32,
}

impl Kill {
    /// The kill as the log says it.
    pub(crate) fn said(&self, name: impl Fn(u32) -> String) -> String {
        let mut s = name(self.attack);
        if let Some((_, id, m, chance)) = self.vuln {
            s = format!("{} (x{m}, lands {:.0}%) + {s}", name(id), chance * 100.0);
        }
        if let Some((id, by, chance)) = self.magic_yield {
            s = format!(
                "{} (-{by:.0}, lands {:.0}%) + {s}",
                name(id),
                chance * 100.0
            );
        }
        if !self.wand.name.is_empty() {
            s = format!("{s} from {}", self.wand.name);
        }
        match self.casts {
            Some(c) => format!("{s}: {c:.1} casts"),
            None => format!("{s}: {:.0} a cast", self.per_cast),
        }
    }
}

/// The casts it takes on average to take `health` off, each landing cast rolling evenly from `lo`
/// to `hi` and adding `bonus` on a crit (chance `crit`): the sum over k of the chance k casts leave
/// it standing, crits binomial and the rolls Irwin-Hall; past [`EXACT_CASTS`] casts' worth, the
/// renewal estimate `h/mean + 1/2 + variance/(2 mean²)`.
pub(crate) fn casts_to_kill_crits(health: f32, lo: f32, hi: f32, crit: f32, bonus: f32) -> f32 {
    let p = crit.clamp(0.0, 1.0);
    let mean = (lo + hi) / 2.0 + p * bonus;
    if mean <= 0.0 {
        return f32::INFINITY;
    }
    let width = (hi - lo).max(0.0);
    if health / mean > EXACT_CASTS as f32 {
        let var = width * width / 12.0 + p * (1.0 - p) * bonus * bonus;
        return health / mean + 0.5 + var / (2.0 * mean * mean);
    }
    let below = |k: u32, x: f32| -> f64 {
        if width < 1e-3 {
            return if x > k as f32 * lo { 1.0 } else { 0.0 };
        }
        irwin_hall(k, f64::from((x - k as f32 * lo) / width))
    };
    // No cast at all leaves it standing.
    let mut casts = 1.0f64;
    for k in 1..=2 * EXACT_CASTS {
        let mut standing = 0.0f64;
        let mut ways = 1.0f64;
        for j in 0..=k {
            let left = health - j as f32 * bonus;
            if left > k as f32 * lo {
                let pj =
                    ways * f64::from(p).powi(j as i32) * f64::from(1.0 - p).powi((k - j) as i32);
                standing += pj * below(k, left);
            }
            ways *= f64::from(k - j) / f64::from(j + 1);
        }
        if standing < 1e-6 {
            break;
        }
        casts += standing;
    }
    casts as f32
}

/// Casts taken whole, up to which [`casts_to_kill_crits`] sums the chances exactly.
const EXACT_CASTS: u32 = 12;

/// The chance that `n` rolls each even on 0 to 1 sum under `x`.
fn irwin_hall(n: u32, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
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

/// Of `kills`, the fastest: fewest casts where the health is known, ties to the earlier (the held
/// wand and no debuff come first); with no health known, the most a cast with no debuff.
pub(crate) fn fastest(kills: Vec<Kill>) -> Option<Kill> {
    if kills.iter().any(|k| k.casts.is_some()) {
        kills
            .into_iter()
            .filter(|k| k.casts.is_some())
            .reduce(|best, next| if next.casts < best.casts { next } else { best })
    } else {
        kills
            .into_iter()
            .filter(|k| k.vuln.is_none() && k.magic_yield.is_none())
            .reduce(|best, next| {
                if next.per_cast > best.per_cast {
                    next
                } else {
                    best
                }
            })
    }
}

impl Client {
    /// The wands to choose from: the one in hand, and with `pick_weapon` every other caster carried
    /// that has been appraised and can be wielded. Bare hands when none is held.
    pub(crate) fn wands_to_hand(&self, cfg: &Fight) -> Vec<Wand> {
        let wielder = self.wielder();
        let mut wands: Vec<Wand> = self
            .item_stats()
            .into_iter()
            .filter(|i| crate::weapons::stance_of(i) == Some(crate::weapons::Stance::Magic))
            .filter(|i| i.wielded || (cfg.pick_weapon && i.appraised && wielder.can_wield(i)))
            .map(|i| {
                let cleave = self.appraisals.get(&i.guid).and_then(|a| {
                    let bits = a.ints.iter().find(|(k, _)| *k == CLEAVE_TYPE)?.1 as u32;
                    let m = a.floats.iter().find(|(k, _)| *k == CLEAVE_MOD)?.1 as f32;
                    (bits != 0).then_some((bits, 1.0 + m))
                });
                Wand {
                    guid: i.guid,
                    name: i.name.clone(),
                    held: i.wielded,
                    element: i.damage_type_bits,
                    elemental: i.elemental_damage,
                    imbued: i.imbued,
                    cleave,
                    crit_frequency: i.crit_frequency,
                    crit_multiplier: i.crit_multiplier,
                }
            })
            .collect();
        if !wands.iter().any(|w| w.held) {
            wands.insert(0, Wand::none());
        }
        // The held one first, so a tie keeps it.
        wands.sort_by_key(|w| !w.held);
        wands
    }

    /// The skill `spell`'s school casts from, without enchantments: what the imbues read (:822).
    fn school_base(&self, spell: u32) -> u32 {
        let stats = &self.world.stats;
        let Some(id) = self
            .spell(spell)
            .and_then(|sp| crate::Client::school_skill(sp.school))
        else {
            return 0;
        };
        let Some(sk) = stats.skill(id) else {
            return 0;
        };
        let table = self.assets.skill_table().ok();
        stats.skill_value(sk, table.as_ref().and_then(|t| t.get(id)))
    }

    /// Every way to kill `guid` from `wands` and the attacks `ready`: each wand, each attack with
    /// the vulnerability already on and with each level castable well for its element first,
    /// each of those with Magic Yield first too when the creature's magic defence is known.
    pub(crate) fn kills(&self, guid: u32, ready: &[u32], wands: &[Wand]) -> Vec<Kill> {
        let on = self
            .autoplay
            .vulned
            .iter()
            .find(|(g, _)| *g == guid)
            .and_then(|(_, cast)| *cast);
        let fight = Softened {
            on,
            may_vuln: !self.softened_by_anyone(guid),
            yielded: self.autoplay.yielded_by(guid),
        };
        let known = self.creature_known(guid);
        self.kills_with(known, self.health_now(guid), fight, ready, wands)
    }

    /// The fastest way to kill one of kind `known` met fresh, at full health, with the wand in
    /// hand: the casts a kill takes, for weighing a hunting ground. `None` with no way or no health.
    pub(crate) fn casts_on_kind(
        &self,
        known: &ac_world::elements::Creature,
        ready: &[u32],
        wands: &[Wand],
    ) -> Option<f32> {
        let health = Some(known.health as f32).filter(|h| *h > 0.0);
        let fresh = Softened {
            on: None,
            may_vuln: true,
            yielded: 0.0,
        };
        fastest(self.kills_with(Some(known), health, fresh, ready, wands))?.casts
    }

    /// [`Self::kills`] for a creature of kind `known` with `health` left, softened as `fight` says.
    fn kills_with(
        &self,
        known: Option<&ac_world::elements::Creature>,
        health: Option<f32>,
        fight: Softened,
        ready: &[u32],
        wands: &[Wand],
    ) -> Vec<Kill> {
        let Softened {
            on,
            may_vuln,
            yielded,
        } = fight;
        // Magic defence as it stands, less the yield already on; a stronger yield replaces it.
        let defence = known
            .and_then(|c| c.magic_defense)
            .map(|d| (d as f32 - yielded).max(0.0));
        let yields: Vec<(Option<Yield>, Option<f32>)> = {
            let mut y = vec![(None, defence)];
            let levels = self
                .castable_levels(&self.magic_yield_spells())
                .into_iter()
                .filter_map(|id| Some((id, -ac_world::buffs::effect(id)?.value)))
                .filter(|(_, by)| *by > yielded && may_vuln);
            for (id, by) in levels {
                let Some(d) = defence else { break };
                let chance = self.cast_chance(id) * unresisted(self.casting_skill(id), Some(d));
                let lowered = (d + yielded - by).max(0.0);
                y.push((Some((id, by - yielded, chance.max(0.01))), Some(lowered)));
            }
            y
        };
        // Per element, once: each vulnerability level castable well, its multiplier and chance.
        let mut vulns: Vec<(Element, Vec<Vuln>)> = Vec::new();
        let mut out = Vec::new();
        for wand in wands {
            let swap = if wand.held { 0.0 } else { SWAP_CASTS };
            for &id in ready {
                let Some(e) = ac_world::elements::spell_element(id) else {
                    continue;
                };
                let Some((min, max)) = ac_world::elements::spell_damage(id) else {
                    continue;
                };
                let power = self.spell(id).map_or(0, |s| s.power);
                let skill = self.casting_skill(id);
                let base = self.school_base(id);
                let bonus_skill = min as f32 * skill.saturating_sub(power) as f32 / 1000.0;
                let (crit, crit_mult) = wand.crits(base);
                let natural = known.map_or(1.0, |c| c.takes_from(e));
                let rend = wand.rend_for(e, base);
                let elemental = wand.elemental_for(e);
                let not_fizzled = self.cast_chance(id);
                let on_m = on.filter(|(oe, _)| *oe == e).map_or(1.0, |(_, m)| m);
                let levels = match vulns.iter().find(|(ve, _)| *ve == e) {
                    Some((_, v)) => v.clone(),
                    None => {
                        let v: Vec<Vuln> = self
                            .castable_levels(&ac_world::elements::vulnerabilities(e))
                            .into_iter()
                            .filter_map(|vid| {
                                ac_world::elements::vuln_mod(vid)
                                    .map(|m| (vid, m, self.cast_chance(vid).max(0.01)))
                            })
                            .collect();
                        vulns.push((e, v.clone()));
                        v
                    }
                };
                let mut way = |vm: f32,
                               first: Option<(Element, u32, f32, f32)>,
                               magic_yield: Option<Yield>,
                               defence: Option<f32>,
                               tries: f32| {
                    let landing = (not_fizzled * unresisted(skill, defence)).max(0.01);
                    let takes = elemental * natural * vm.max(rend);
                    let lo = (min as f32 + bonus_skill) * takes;
                    let hi = (max as f32 + bonus_skill) * takes;
                    let crit_bonus = 0.5 * max as f32 * crit_mult * takes;
                    let per_cast = ((lo + hi) / 2.0 + crit * crit_bonus) * landing;
                    let casts = health.map(|h| {
                        swap + tries + casts_to_kill_crits(h, lo, hi, crit, crit_bonus) / landing
                    });
                    out.push(Kill {
                        wand: wand.clone(),
                        attack: id,
                        takes,
                        vuln: first,
                        magic_yield,
                        casts,
                        per_cast,
                    });
                };
                for &(magic_yield, defence) in &yields {
                    let first = magic_yield.map_or(0.0, |(_, _, chance)| 1.0 / chance);
                    way(on_m, None, magic_yield, defence, first);
                    for &(vid, m, vuln_cast) in levels.iter().filter(|_| may_vuln) {
                        if m > on_m {
                            let chance = (vuln_cast * unresisted(self.casting_skill(vid), defence))
                                .max(0.01);
                            let tries = first + 1.0 / chance;
                            way(m, Some((e, vid, m, chance)), magic_yield, defence, tries);
                        }
                    }
                }
            }
        }
        out
    }

    /// The fastest way to kill `guid` with what is to hand (see [`fastest`]), kept a moment.
    pub(crate) fn best_kill(&self, guid: u32, cfg: &Fight, now: Instant) -> Option<Kill> {
        if let Some((g, at, kill)) = self.autoplay.kill_kept.borrow().as_ref() {
            if *g == guid && now.saturating_duration_since(*at) < KILL_KEEPS {
                return kill.clone();
            }
        }
        let ready: Vec<u32> = self.ready_spells(cfg).iter().map(|(id, _)| *id).collect();
        let wands = self.wands_to_hand(cfg);
        let kill = fastest(self.kills(guid, &ready, &wands));
        *self.autoplay.kill_kept.borrow_mut() = Some((guid, now, kill.clone()));
        kill
    }

    /// The fastest way with the wand in hand only, for judging many targets at once.
    pub(crate) fn kill_in_hand(&self, guid: u32, ready: &[u32]) -> Option<Kill> {
        let cfg = Fight {
            pick_weapon: false,
            ..self.autoplay.config.fight.clone()
        };
        let wands = self.wands_to_hand(&cfg);
        fastest(self.kills(guid, ready, &wands))
    }

    /// The other ways to the kill, fastest first, as the log says them, after the chosen one.
    pub(crate) fn kills_said(&self, guid: u32, cfg: &Fight, most: usize) -> String {
        let ready: Vec<u32> = self.ready_spells(cfg).iter().map(|(id, _)| *id).collect();
        let wands = self.wands_to_hand(cfg);
        let mut all = self.kills(guid, &ready, &wands);
        all.retain(|k| k.casts.is_some());
        all.sort_by(|a, b| {
            a.casts
                .partial_cmp(&b.casts)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let name = |id: u32| self.spell(id).map(|s| s.name).unwrap_or_default();
        all.iter()
            .take(most)
            .map(|k| k.said(name))
            .collect::<Vec<_>>()
            .join("; ")
    }
}

#[cfg(test)]
mod tests;
