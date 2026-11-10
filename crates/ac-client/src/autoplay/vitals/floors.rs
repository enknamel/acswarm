//! The health, stamina and mana sliders (`Survive`) as floors. No optional cast takes mana under
//! its floor (`can_spare_mana`); a vital under one is restored before the next fight
//! (`autoplay_vitals`); one nothing restores where the character stands breaks the fight off: walk
//! away from the attackers and rest till each is back above its floor (`autoplay_fall_back`).

use std::time::{Duration, Instant};

use ac_world::Place;
use glam::{Vec2, Vec3};

use super::heal::restores_health;
use crate::autoplay::{Doing, Release, Style, Survive};
use crate::did::Did;
use crate::{Client, Goal, Stance};

/// Above its floor by this fraction of the bar before a fall back ends (guess: a few casts' room).
const RESTED_MARGIN: f32 = 0.15;
/// How far one leg of a fall back aims from the character, away from the attackers (metres, flat).
const LEG: f32 = 30.0;
/// Open ground is looked for within this of a leg's aim (metres, flat).
const LOOK_ROUND: f32 = 8.0;
/// At a leg's spot within this (metres, 3D).
const AT_SPOT: f32 = 2.0;
/// Legs walked with creatures still on the character before it is cornered: four legs pass ACE's
/// 96 m chase (`MaxChaseRange`, Monster_Navigation.cs:21, :244).
const LEGS: u32 = 4;
/// A creature coming at the character within this is fled (metres); one hitting it, at any range.
const ATTACKERS_WITHIN: f32 = 30.0;
/// A creature that attacked within this is still on the character: a few of its rounds, monster
/// spells going every 3 s and more (`AiUseMagicDelay`, Monster_Combat.cs:63).
const HIT_LATELY: Duration = Duration::from_secs(10);
/// A cast that can never leave the mana floor goes from a bar within this fraction of the top,
/// the restore's own cast leaving it short of full (guess).
const NEARLY_FULL: f32 = 0.05;
/// Turns off straight away from the attackers tried for a leg, in degrees.
const TURNS: [f32; 5] = [0.0, 45.0, -45.0, 90.0, -90.0];
/// The vitals by their slot, for the log.
const NAMES: [&str; 3] = ["health", "stamina", "mana"];

/// A fall back under way (`Autoplay.falling_back`).
#[derive(Clone, Debug, Default)]
pub(crate) struct FallBack {
    /// Mana points the cast that started it wanted with the floor kept, 0 for none.
    mana_for: u32,
    /// The open ground the leg under way walks to.
    spot: Option<Place>,
    /// Legs walked with creatures still on the character.
    legs: u32,
    /// Spots whose walk the slot refused.
    refused: Vec<Vec3>,
    /// Nowhere left to walk with creatures on it: those are fought till the floors are back.
    cornered: bool,
}

/// The first vital (0 health, 1 stamina, 2 mana) under its floor plus `margin`, from `bars` as
/// fractions; mana only when `mana` binds. A floor of 0 is off.
pub(crate) fn short_bar(bars: [f32; 3], cfg: &Survive, mana: bool, margin: f32) -> Option<usize> {
    let floors = [cfg.heal_below, cfg.stamina_below, cfg.mana_below];
    (0..3).find(|&i| (i < 2 || mana) && floors[i] > 0.0 && bars[i] < (floors[i] + margin).min(1.0))
}

/// Mana points a cast of `cost` wants first, of a bar of `most` with its floor at `below`: the
/// floor's and its own, or a nearly full bar for one that can never leave the floor ([`NEARLY_FULL`]).
pub(crate) fn mana_wanted(cost: u32, most: u32, below: f32) -> u32 {
    let floor = (below * most as f32).ceil() as u32;
    let full = most - (most as f32 * NEARLY_FULL) as u32;
    if floor + cost <= most {
        floor + cost
    } else {
        full
    }
}

/// The flat way from `me` straight away from the attackers' middle (east when stood on it).
fn away_from(me: Vec3, attackers: &[Vec3]) -> Vec2 {
    let middle = attackers.iter().sum::<Vec3>() / attackers.len().max(1) as f32;
    (me - middle).truncate().try_normalize().unwrap_or(Vec2::X)
}

impl Client {
    /// Each vital as a fraction of its most (0 health, 1 stamina, 2 mana), 1.0 when not known.
    pub(crate) fn bars(&self) -> [f32; 3] {
        [0, 1, 2].map(|i| match self.vital_now(i) {
            (_, 0) => 1.0,
            (have, most) => have as f32 / most as f32,
        })
    }

    /// Whether casting `spell` leaves mana at or above its floor (`Survive::mana_below`), or one
    /// that never can goes from a nearly full bar ([`mana_wanted`]).
    pub(crate) fn can_spare_mana(&self, spell: u32) -> bool {
        let below = self.autoplay.config.survive.mana_below;
        let (have, most) = self.vital_now(2);
        most == 0 || below <= 0.0 || have >= self.mana_for(spell)
    }

    /// Mana points the cast `spell` wants first ([`mana_wanted`]).
    fn mana_for(&self, spell: u32) -> u32 {
        let cost = self.spell(spell).map_or(0, |sp| self.mana_cost(&sp));
        let (_, most) = self.vital_now(2);
        mana_wanted(cost, most, self.autoplay.config.survive.mana_below)
    }

    /// Whether the fight's next cast `spell` waits for the mana floor: the mana round goes first
    /// while it can, else the fight is broken off ([`Self::autoplay_fall_back`]). Cornered, never.
    pub(crate) fn hold_for_mana(&mut self, spell: u32, now: Instant) -> bool {
        if self.can_spare_mana(spell) || self.is_cornered() {
            return false;
        }
        let name = self
            .spell(spell)
            .map(|sp| sp.name.clone())
            .unwrap_or_default();
        if self.restore_mana(now) {
            let said = format!("holding {name} for the mana floor: restoring mana first");
            self.autoplay.note(said, now);
            return true;
        }
        let said = format!("holding {name} for the mana floor: nothing restores mana here");
        self.autoplay.note(said, now);
        let mana_for = self.mana_for(spell);
        let held = self
            .autoplay
            .falling_back
            .get_or_insert_with(FallBack::default);
        held.mana_for = held.mana_for.max(mana_for);
        true
    }

    /// Whether a fall back has nowhere left to walk with creatures on the character.
    pub(crate) fn is_cornered(&self) -> bool {
        self.autoplay
            .falling_back
            .as_ref()
            .is_some_and(|f| f.cornered)
    }

    /// Whether fights are put off for a fall back that is not cornered.
    pub(crate) fn is_falling_back(&self) -> bool {
        self.autoplay
            .falling_back
            .as_ref()
            .is_some_and(|f| !f.cornered)
    }

    /// Whether mana is what the character fights or heals with, so that its floor binds.
    fn has_mana_floor(&self) -> bool {
        let style = self.autoplay.config.fight.style;
        let wand = style == Style::Magic
            || (style == Style::Auto && self.combat_stance() == Stance::Magic);
        wand || self.world.stats.spells.iter().any(|s| restores_health(*s))
    }

    /// Each vital under its floor that nothing restores where the character stands, in words:
    /// health with no heal or kit to hand, stamina with no Revitalize, mana with no mana round.
    fn unrestorable_floors(&self) -> Option<String> {
        use ac_world::vitals::vital;
        let cfg = &self.autoplay.config.survive;
        let bars = self.bars();
        if bars[0] <= 0.0 {
            return None;
        }
        let floors = [cfg.heal_below, cfg.stamina_below, cfg.mana_below];
        let mut out = Vec::new();
        for i in 0..3 {
            if bars[i] >= floors[i] {
                continue;
            }
            let restored = match i {
                0 => self.can_heal_here(),
                1 => {
                    cfg.manage_mana
                        && self.best_boost(vital::STAMINA).is_some_and(|s| {
                            matches!(self.can_cast_well(s), crate::magic::CastCheck::Ok)
                        })
                }
                _ => !self.has_mana_floor() || self.mana_round().is_some(),
            };
            if !restored {
                out.push(format!(
                    "{} {:.0}% under its {:.0}% floor",
                    NAMES[i],
                    bars[i] * 100.0,
                    floors[i] * 100.0
                ));
            }
        }
        (!out.is_empty()).then(|| out.join(", "))
    }

    /// Whether every vital is back above its floor and [`RESTED_MARGIN`], with the mana a held
    /// cast wanted.
    fn is_rested(&self) -> bool {
        let cfg = &self.autoplay.config.survive;
        let mana_for = self
            .autoplay
            .falling_back
            .as_ref()
            .map_or(0, |f| f.mana_for);
        let rested = short_bar(self.bars(), cfg, self.has_mana_floor(), RESTED_MARGIN).is_none();
        rested && self.vital_now(2).0 >= mana_for
    }

    /// While a floor nothing restores here is down: break the fight off, walk away from the
    /// attackers a leg at a time, and rest till [`Self::is_rested`]. True while it holds the tick.
    pub(crate) fn autoplay_fall_back(&mut self, now: Instant) -> bool {
        if self.autoplay.falling_back.is_none() {
            let Some(why) = self.unrestorable_floors() else {
                return false;
            };
            self.autoplay
                .note(format!("breaking off to recover: {why}"), now);
            self.autoplay.falling_back = Some(FallBack::default());
        }
        if self.is_rested() {
            self.autoplay.falling_back = None;
            self.autoplay
                .note("vitals back above their floors: fighting again", now);
            return false;
        }
        let Some(me) = self.player.as_ref().map(|p| Place {
            at: p.world_position(),
            cell: p.cell,
        }) else {
            return false;
        };
        let attackers = self.attackers_round(me.at, now);
        if attackers.is_empty() {
            if let Some(held) = self.autoplay.falling_back.as_mut() {
                *held = FallBack {
                    mana_for: held.mana_for,
                    ..FallBack::default()
                };
            }
            // A town run under way picks no fight, and walks on.
            if self.autoplay.growth.town_run_under_way() {
                return false;
            }
            self.break_off(now);
            let said = "resting till the vitals are back above their floors";
            self.autoplay.say(Doing::Resting, said);
            return true;
        }
        let (spot, legs, cornered) = self
            .autoplay
            .falling_back
            .as_ref()
            .map(|f| (f.spot, f.legs, f.cornered))
            .unwrap_or_default();
        if cornered {
            return false;
        }
        self.break_off(now);
        let spot = match spot {
            Some(spot) => spot,
            None if legs >= LEGS => return self.cornered(attackers.len(), now),
            None => match self.fall_back_spot(me, &attackers) {
                None => {
                    let said = "falling back: looking for open ground";
                    self.autoplay.say(Doing::Resting, said);
                    return true;
                }
                Some(None) => return self.cornered(attackers.len(), now),
                Some(Some(spot)) => {
                    if let Some(held) = self.autoplay.falling_back.as_mut() {
                        held.spot = Some(spot);
                        held.legs += 1;
                    }
                    spot
                }
            },
        };
        let goal = Goal::At {
            place: spot,
            within: AT_SPOT,
        };
        match self.go(self.step_owner(), goal, now) {
            Did::Done => self.end_leg(None),
            Did::Blocked(_) | Did::Refused(_) => self.end_leg(Some(spot.at)),
            Did::Acting | Did::Waiting(_) => {
                let said = format!("falling back from {} attackers to recover", attackers.len());
                self.autoplay.say(Doing::Resting, said);
            }
        }
        true
    }

    /// A leg walked, or refused at `refused`: the next is chosen from where the character stands.
    fn end_leg(&mut self, refused: Option<Vec3>) {
        if let Some(held) = self.autoplay.falling_back.as_mut() {
            held.spot = None;
            held.refused.extend(refused);
        }
    }

    /// No open ground left to fall back to from `attackers`: they are fought till the floors are back.
    fn cornered(&mut self, attackers: usize, now: Instant) -> bool {
        if let Some(held) = self.autoplay.falling_back.as_mut() {
            held.cornered = true;
        }
        let said = format!("cornered by {attackers} attackers with a floor down: fighting on");
        self.autoplay.note(said, now);
        false
    }

    /// Stop the fight in hand: the swing, the spell still winding up and the targets.
    fn break_off(&mut self, now: Instant) {
        if self.attack_target.is_some() || self.attack_pending {
            self.cancel_attack();
        }
        self.stop_fight_cast(now);
        self.let_go(Release::Fight);
    }

    /// Where the creatures on the character stand: each that attacked within [`HIT_LATELY`] at any
    /// range (spells to 75 m, Monster_Magic.cs:117), each fought or coming at it within 30 m.
    fn attackers_round(&self, me: Vec3, now: Instant) -> Vec<Vec3> {
        use ac_world::{item_type, object_desc_flags};
        let mine = self.world.player_guid;
        let fought = [self.attack_target, self.autoplay.casting_at];
        self.world
            .objects
            .values()
            .filter(|o| {
                o.item_type & item_type::CREATURE != 0
                    && o.object_desc_flags & object_desc_flags::PLAYER == 0
                    && !o.is_player
                    && o.pet_owner == 0
                    && o.alive_or_unknown()
            })
            .filter_map(|o| {
                let at = o.world_pos()?;
                let hitting = self.hit_lately_from(o.guid, now, HIT_LATELY);
                let coming = (mine.is_some() && (o.walked_at == mine || o.turned_to == mine))
                    || fought.contains(&Some(o.guid));
                (hitting || (coming && at.distance(me) <= ATTACKERS_WITHIN)).then_some(at)
            })
            .collect()
    }

    /// Open ground a [`LEG`] off, straight away from `attackers` or turned off it, farthest from
    /// them; `Some(None)` for none, `None` while the region is built.
    fn fall_back_spot(&mut self, me: Place, attackers: &[Vec3]) -> Option<Option<Place>> {
        let away = away_from(me.at, attackers);
        let middle = attackers.iter().sum::<Vec3>() / attackers.len() as f32;
        let from_them = |p: &Place| (p.at - middle).truncate().length();
        let refused = self
            .autoplay
            .falling_back
            .as_ref()
            .map(|f| f.refused.clone())
            .unwrap_or_default();
        for turn in TURNS {
            let way = Vec2::from_angle(turn.to_radians()).rotate(away);
            let aim = me.at + (way * LEG).extend(0.0);
            let best = self
                .standable_near(aim, LOOK_ROUND)?
                .into_iter()
                .filter(|p| refused.iter().all(|r| r.distance(p.at) > LOOK_ROUND))
                .filter(|p| from_them(p) > from_them(&me) + LEG * 0.5)
                .max_by(|a, b| from_them(a).total_cmp(&from_them(b)));
            if best.is_some() {
                return Some(best);
            }
        }
        Some(None)
    }
}

#[cfg(test)]
mod tests;
