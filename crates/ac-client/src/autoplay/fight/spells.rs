use std::time::Instant;

use crate::autoplay::{cast_problem, Autoplay, Doing, Fight, Release};
use crate::Client;

/// ACE's `TargetNotAcquired`: the cast's target is nowhere the server looks (`Player_Magic.cs:137`).
pub(crate) const TARGET_NOT_ACQUIRED: u32 = 0x042C;

impl Autoplay {
    /// The creature spells are being thrown at, if any: the magic
    /// fighter's counterpart to `Client::attack_target`.
    pub fn casting_at(&self) -> Option<u32> {
        self.casting_at
    }
}

impl Client {
    /// The server's answer `err` to the fight's last cast. Any refusal (`0x1D` too busy among them)
    /// means a debuff just sent never went on, so it is cast again
    /// (test: a_vulnerability_the_server_turned_down_is_cast_again). A target it cannot find, here or
    /// in the blocks around (`Landblock.cs:1043`), is gone and this client missed the word: cast at,
    /// it was refused three times a second for up to 17 s (test: a_target_the_server_cannot_find_is_forgotten).
    pub(crate) fn hear_cast_refused(&mut self, err: u32, now: Instant) {
        if err != 0 {
            self.rearm_debuff("was turned down");
        }
        if err != TARGET_NOT_ACQUIRED {
            return;
        }
        if let Some((guid, _)) = self.autoplay.fight_cast {
            self.give_up_target(guid, "the server has no such creature", now);
            self.world.forget(guid);
        }
    }

    /// Fighting with spells: pick a target the same way, then throw the
    /// first spell in the list that can be cast right now.
    ///
    /// A swing sets `attack_target` and the server keeps swinging; a
    /// spell does not, so this keeps its own target and re-casts on the
    /// pace of a cast rather than of a frame. The target is selected
    /// first because that is what `try_cast` throws at.
    pub(super) fn autoplay_fight_with_spells(&mut self, now: Instant, cfg: &Fight) -> bool {
        if cfg.spells.is_empty() && self.attack_spells_known().is_empty() {
            self.autoplay.say(Doing::Idle, "no attack spells known");
            return false;
        }
        if self.wielded_caster().is_none() {
            self.autoplay.say(Doing::Idle, "no caster wielded");
            return false;
        }
        let alive = |c: &Client, g: u32| {
            c.world
                .objects
                .get(&g)
                .is_some_and(|o| o.alive_or_unknown())
        };
        let underground = self.underground();
        // The leader's plan has this character on another, and the one
        // being cast at is not hitting us: let it go for the other.
        if let Some(g) = self.autoplay.casting_at {
            if self.ordered_elsewhere(g, cfg, underground, now) {
                self.let_go(Release::Cast);
            }
        }
        // Stay on the one already being fought while it lives, and is
        // taking damage.
        if let Some(g) = self.autoplay.casting_at {
            if self.stalled_on(g, now) {
                return false;
            }
        }
        // Out of the hunting area and not hitting us, or not here at
        // all any more, it is let go (see `can_keep_target`, which the
        // swing asks of what it holds too).
        let casting_at = self.autoplay.casting_at;
        let kept = casting_at
            .filter(|g| alive(self, *g))
            .filter(|g| self.can_keep_target(*g, underground, now));
        let target = match kept {
            Some(g) => Some(g),
            None => {
                self.let_go(Release::Cast);
                if self.waits_for_a_corpse() {
                    None
                } else {
                    // What the plan says first, if this character's own
                    // rules would take it on (`can_take_on`), else what is nearest.
                    self.ordered_target(cfg, now)
                        .map(|(g, _)| g)
                        .filter(|g| self.can_take_on(*g, cfg, underground, now))
                        .or_else(|| self.pick_target(cfg, now))
                }
            }
        };
        let Some(guid) = target else {
            return false;
        };
        let name = self.world.name_of(guid).unwrap_or_default().to_string();
        self.autoplay.casting_at = Some(guid);
        // Paced to the casting, buffs included: the server queues one
        // spell sent over another and drops the next, so an attack
        // thrown on the heels of a buff would be the one dropped.
        if self.autoplay.cast_in_flight(now) {
            self.autoplay
                .say(Doing::Fighting, format!("fighting {name}"));
            return true;
        }
        self.select(Some(guid));
        // The fastest kill with what is to hand (see `kill`): its wand taken up first, behind the
        // throttle so the choice costs a look per cast.
        let kill = self.best_kill(guid, cfg, now);
        if let Some(wand) = kill
            .as_ref()
            .map(|k| k.wand.clone())
            .filter(|w| !w.held && w.guid != 0)
        {
            if self.hands_changing(now) {
                return true;
            }
            if self.mid_attack() {
                self.wait_for_the_swing();
                return true;
            }
            let (carried, wielder) = (self.item_stats(), self.wielder());
            tracing::info!("autoplay: taking up {} for {name}", wand.name);
            self.take_up(wand.guid, &carried, &wielder);
            *self.autoplay.kill_kept.borrow_mut() = None;
            self.autoplay.say(
                Doing::Fighting,
                format!("taking up {} for {name}", wand.name),
            );
            return true;
        }
        // Of the spells that could be thrown this moment, the one this
        // creature is hurt most by. An Ice Golem takes nothing at all
        // from cold and full damage from fire, so the difference between
        // choosing well and throwing the first spell on the list is the
        // difference between a fight and a stalemate.
        let table = self.assets.spell_table().ok();
        let ready = self.ready_spells(cfg);
        // Two or more swinging at us underground: to a spot the attack still gets out of first.
        if let Some(spell) = kill.as_ref().map(|k| k.attack) {
            let how = crate::dodge::How::Spell(spell);
            if self.autoplay_take_stand(Some(guid), how, now) != crate::did::Did::Done {
                return true;
            }
        }
        // Its vulnerability first, then its attack for the rest.
        if self.autoplay_make_vulnerable(guid, &name, kill.as_ref(), now) {
            return true;
        }
        let best = kill.map(|k| (k.attack, k.takes));
        if let Some((spell, worth)) = best {
            let spell_name = ready
                .iter()
                .find(|(id, _)| *id == spell)
                .map(|(_, n)| n.clone())
                .unwrap_or_default();
            // From too far off the server refuses the cast outright, and
            // from behind a wall or a rise the bolt strikes that instead:
            // close in first (see `crate::dodge`, `crate::aim`).
            if self.autoplay_approach(guid, &name, crate::dodge::How::Spell(spell))
                != crate::did::Did::Done
            {
                return true;
            }
            // Only what went out is recorded: a cast the client
            // declined leaves no projectile to time and no shot to
            // wait on (see `Client::cast_fight_spell`; `try_cast` notes it fired).
            if self.cast_fight_spell(spell, guid, now) {
                self.autoplay.attack_spell = Some(spell);
                self.throw_at(guid, now);
            }
            let element = ac_world::elements::spell_element(spell)
                .map(|e| e.name())
                .unwrap_or("");
            let said = if element.is_empty() || (0.99..=1.01).contains(&worth) {
                format!("casting {spell_name} at {name}")
            } else {
                format!("casting {spell_name} at {name} ({element} x{worth:.2})")
            };
            self.autoplay.say(Doing::Fighting, said);
            return true;
        }
        // Nothing castable: out of mana, out of components, or the
        // spells are not learnt. Say which, for the first of them.
        let first: Option<(String, u32)> = if cfg.spells.is_empty() {
            self.attack_spells_known().first().map(|id| {
                let name = table
                    .as_ref()
                    .and_then(|t| t.get(*id).map(|s| s.name.clone()))
                    .unwrap_or_default();
                (name, *id)
            })
        } else {
            cfg.spells
                .iter()
                .filter_map(|n| self.spell_by_name(n).map(|id| (n.clone(), id)))
                .next()
        };
        let why = first
            .map(|(n, id)| format!("{n}: {}", cast_problem(&self.can_cast_well(id))))
            .unwrap_or_else(|| "none of the attack spells is known".into());
        self.autoplay
            .say(Doing::Fighting, format!("cannot cast at {name} ({why})"));
        true
    }

    /// The attack spells on offer, in the order they are tried and with
    /// the name to say for each: the ones the rules name, else every
    /// attack spell in the book -- the game is a closed system, and the
    /// book says what can be thrown.
    fn offered_spells(&self, cfg: &Fight) -> Vec<(u32, String)> {
        if cfg.spells.is_empty() {
            let table = self.assets.spell_table().ok();
            self.attack_spells_known()
                .into_iter()
                .map(|id| {
                    let name = table
                        .as_ref()
                        .and_then(|t| t.get(id).map(|s| s.name.clone()))
                        .unwrap_or_default();
                    (id, name)
                })
                .collect()
        } else {
            cfg.spells
                .iter()
                .filter_map(|n| self.spell_by_name(n).map(|id| (id, n.clone())))
                .collect()
        }
    }

    /// The spells that could be thrown this moment: [`Self::offered_spells`]
    /// less the ones `can_cast` refuses, in the same order.
    ///
    /// The one to throw is chosen from these against the creature
    /// (`Client::best_kill`), so the target pick reads the
    /// same list and names the same spell (`Client::attack_kind`).
    /// Costly -- `can_cast` walks the packs for components -- so both
    /// callers look once and neither on every tick.
    pub(crate) fn ready_spells(&self, cfg: &Fight) -> Vec<(u32, String)> {
        self.offered_spells(cfg)
            .into_iter()
            .filter(|(id, _)| matches!(self.can_cast_well(*id), crate::magic::CastCheck::Ok))
            .collect()
    }

    /// Every attack spell in the spellbook that is thrown at a target:
    /// the ones the element table knows deal an element, strongest
    /// first. Which of them to throw is decided against the target.
    pub(crate) fn attack_spells_known(&self) -> Vec<u32> {
        let Ok(table) = self.assets.spell_table() else {
            return Vec::new();
        };
        let mut ids: Vec<u32> = self
            .world
            .stats
            .spells
            .iter()
            .copied()
            .filter(|id| ac_world::elements::spell_element(*id).is_some())
            .filter(|id| table.get(*id).is_some_and(|s| s.needs_target()))
            .collect();
        ids.sort_by_key(|id| std::cmp::Reverse(table.get(*id).map(|s| s.power).unwrap_or(0)));
        ids
    }
}

#[cfg(test)]
mod tests;
