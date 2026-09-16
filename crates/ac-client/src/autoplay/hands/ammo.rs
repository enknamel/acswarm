use std::time::{Duration, Instant};

use crate::autoplay::hands::weapon::STANCE_CHANGE;
use crate::autoplay::Doing;
use crate::Client;
use ac_world::equip;

/// Ammunition is made at most this often: a use takes a moment and
/// the bundles need to answer.
const CRAFT_EVERY: Duration = Duration::from_secs(4);

/// The Fletching skill.
const FLETCHING: u32 = 37;

/// Whether an item is worth taking.
/// The ammunition to make for a launcher that takes `fits` (see
/// `ac_world::fletching::ammo_type`), from what is carried as `(wcid,
/// guid)` pairs, within a Fletching of `fletching`: `(recipe, heads,
/// shafts)`. The element `weakest` (the target's weakest, when known)
/// comes first, then whatever is hardest to make, which is the better
/// arrow. `None` when no pair of bundles carried makes anything the
/// launcher shoots.
pub fn choose_recipe(
    fits: u32,
    fletching: u32,
    carried: &[(u32, u32)],
    weakest: Option<ac_world::elements::Element>,
) -> Option<(&'static ac_world::fletching::Recipe, u32, u32)> {
    let held = |wcid: u32| carried.iter().find(|(w, _)| *w == wcid).map(|(_, g)| *g);
    ac_world::fletching::making(fits)
        .filter(|r| r.difficulty <= fletching)
        .filter_map(|r| Some((r, held(r.source)?, held(r.target)?)))
        .max_by_key(|(r, _, _)| {
            let hits = weakest.is_some_and(|w| r.element() == Some(w));
            (hits, r.difficulty)
        })
}

impl Client {
    /// See that the bow has something to shoot: the ammunition chosen
    /// for the target if it is still carried, else whatever fits. True
    /// when there is something to shoot -- in the slot, or on its way
    /// into it.
    ///
    /// Not "did a wield go out this tick": the one caller reads a false
    /// as "no ammunition" and goes off to fletch some (see
    /// [`Client::autoplay_craft_ammo`]). A wield held back because the
    /// server is busy is a tick's wait, not an empty quiver, and reading
    /// it as one dropped an archer with a full quiver into peace stance
    /// to make arrows it was already carrying.
    pub(crate) fn ready_ammo(&mut self) -> bool {
        if self.wielded_ammo().is_some() {
            // The chosen kind, if it is not the one in the slot.
            if let Some(want) = self.autoplay.wanted_ammo {
                if self.wielded_ammo() != Some(want) && self.world.is_carried(want) {
                    self.wield_guid(want);
                }
            }
            return true;
        }
        if let Some(want) = self.autoplay.wanted_ammo {
            if self.world.is_carried(want) {
                // Sent, or waiting on a busy tick: either way there is
                // something to shoot and nothing to make. Only a stack
                // the server keeps refusing to wield is an answer of
                // "not this kind", and then another stack is tried.
                if self.wield_guid(want) || !self.wield_held_off(want) {
                    return true;
                }
            }
        }
        self.wield_ammo()
    }

    /// Make ammunition for the launcher in hand from a bundle of heads
    /// and a bundle of shafts carried, the recipe within Fletching, for
    /// the element the target is weakest to when there is a choice.
    /// True when this tick went on making some.
    ///
    /// The server's side of it (ACE `RecipeManager::UseObjectOnTarget`):
    /// using the heads on the shafts is refused outright in any combat
    /// stance, and by a character not trained in Fletching, so the
    /// character drops to peace first and the bundles are used once the
    /// stance change has had its moment. The server may then ask, as a
    /// yes/no confirmation, whether the chance of success is good
    /// enough; it is answered yes, and the arrows land in the pack a
    /// clap of the hands later, where the bow's arming picks them up.
    pub(crate) fn autoplay_craft_ammo(&mut self, now: Instant) -> bool {
        if !self.autoplay.config.fight.craft_ammo {
            return false;
        }
        // The chance-of-success question, if the character has that
        // option on: the answer is always yes, the bundles being for
        // nothing else.
        const CRAFT: u32 = 5;
        let asked: Vec<u32> = self
            .world
            .confirmations
            .iter()
            .filter(|c| c.kind == CRAFT)
            .map(|c| c.context)
            .collect();
        if !asked.is_empty() && self.autoplay.last_craft.is_some() {
            for context in asked {
                self.confirm(CRAFT, context, true);
            }
            return true;
        }
        // Waiting for peace mode before the bundles are used.
        if let Some((source, target, since)) = self.autoplay.crafting {
            if now.duration_since(since) < STANCE_CHANGE {
                return true;
            }
            self.autoplay.crafting = None;
            self.autoplay.last_craft = Some(now);
            return self.use_on(source, target);
        }
        if self
            .autoplay
            .last_craft
            .is_some_and(|t| now.duration_since(t) < CRAFT_EVERY)
        {
            return false;
        }
        let launcher = self
            .wielded_missile_weapon()
            .and_then(|g| self.stats_of(g))
            .filter(crate::weapons::is_launcher);
        let Some(launcher) = launcher else {
            return false;
        };
        if launcher.ammo_type == 0 {
            return false;
        }
        // Fletching as it stands, and only if trained: an untrained
        // skill has a number too, but the server will not craft with it.
        let fletching = {
            let stats = &self.world.stats;
            let table = self.assets.skill_table().ok();
            stats
                .skill(FLETCHING)
                .filter(|sk| sk.advancement >= ac_world::stats::sac::TRAINED)
                .map(|sk| stats.skill_current(sk, table.as_ref().and_then(|t| t.get(FLETCHING))))
                .unwrap_or(0)
        };
        let carried: Vec<(u32, u32)> = self
            .world
            .objects
            .values()
            .filter(|o| self.world.is_carried(o.guid))
            .map(|o| (o.weenie_class_id, o.guid))
            .collect();
        let weakest = self
            .autoplay
            .casting_at
            .or(self.attack_target)
            .and_then(|g| self.creature_known(g))
            .and_then(|c| c.weakest_to());
        let Some((recipe, source, target)) =
            choose_recipe(launcher.ammo_type, fletching, &carried, weakest)
        else {
            self.autoplay.note(
                format!(
                    "out of {} and nothing to make more from",
                    ac_world::fletching::ammo_type::name(launcher.ammo_type)
                ),
                now,
            );
            return false;
        };
        self.autoplay.say(
            Doing::Looting,
            format!("making {} from {}", recipe.result_name, recipe.source_name),
        );
        if self.combat || self.magic {
            // Peace first; the use goes out once the stance has changed.
            self.leave_combat();
            self.autoplay.crafting = Some((source, target, now));
            return true;
        }
        self.autoplay.last_craft = Some(now);
        self.use_on(source, target)
    }

    /// Ammunition carried, wielded and in the packs, for a bow or
    /// crossbow in hand: `(kind, count)`.
    pub(crate) fn ammo_carried(&self) -> Option<(u32, u32)> {
        let launcher = self
            .wielded_missile_weapon()
            .and_then(|g| self.stats_of(g))
            .filter(crate::weapons::is_launcher)?;
        if launcher.ammo_type == 0 {
            return None;
        }
        let me = self.world.player_guid;
        let count: u32 = self
            .world
            .objects
            .values()
            .filter(|o| o.valid_locations & equip::MISSILE_AMMO != 0)
            .filter(|o| o.wielder == me || self.world.is_carried(o.guid))
            .map(|o| o.stack_size.max(1))
            .sum();
        Some((launcher.ammo_type, count))
    }

    /// Whether more ammunition of `kind` could be made from what is
    /// carried (see `autoplay::Fight::craft_ammo`).
    pub(crate) fn can_craft_ammo(&self, kind: u32) -> bool {
        if !self.autoplay.config.fight.craft_ammo {
            return false;
        }
        let carried: Vec<u32> = self.world.inventory().map(|o| o.weenie_class_id).collect();
        ac_world::fletching::making(kind)
            .any(|r| carried.contains(&r.source) && carried.contains(&r.target))
    }
}

#[cfg(test)]
mod tests;
