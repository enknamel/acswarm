use std::time::Instant;

use crate::autoplay::{cast_problem, Autoplay, Doing, Fight, Release};
use crate::{Client, Stance};

impl Autoplay {
    /// The creature spells are being thrown at, if any: the magic
    /// fighter's counterpart to `Client::attack_target`.
    pub fn casting_at(&self) -> Option<u32> {
        self.casting_at
    }
}

impl Client {
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
                .is_some_and(|o| o.health.unwrap_or(1.0) > 0.0)
        };
        // The leader's plan has this character on another, and the one
        // being cast at is not hitting us: let it go for the other.
        if let Some(g) = self.autoplay.casting_at {
            if self.ordered_elsewhere(g, cfg, now) {
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
        // all any more, it is let go (see `fight_target_gone`).
        let underground = self.underground();
        let casting_at = self.autoplay.casting_at;
        let kept = casting_at.filter(|g| {
            alive(self, *g)
                && !self.fight_target_gone(*g, underground)
                && !self.left_behind_on_the_road(*g, now)
        });
        let target = match kept {
            Some(g) => Some(g),
            None => {
                self.let_go(Release::Cast);
                if self.waits_for_a_corpse() {
                    None
                } else {
                    // What the plan says first, else what is nearest.
                    self.ordered_target(cfg, now)
                        .map(|(g, _)| g)
                        .or_else(|| self.pick_target(cfg))
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
        // Behind the throttle, so choosing a wand costs no more than one
        // look per cast. Wielding takes a moment, so this cast still
        // goes out with the old one and the next with the new.
        self.arm_for(guid, Stance::Magic, cfg);
        self.select(Some(guid));
        if self.autoplay_plan_hard(guid, &name, now) {
            return true;
        }
        // Something with a lot of health is worth softening first: one
        // vulnerability for the element it is weakest to, then throw
        // that element at it for the rest of the fight.
        if self.autoplay_make_vulnerable(guid, &name, now) {
            return true;
        }
        // Of the spells that could be thrown this moment, the one this
        // creature is hurt most by. An Ice Golem takes nothing at all
        // from cold and full damage from fire, so the difference between
        // choosing well and throwing the first spell on the list is the
        // difference between a fight and a stalemate.
        let table = self.assets.spell_table().ok();
        let ready: Vec<(u32, String)> = self
            .offered_spells(cfg)
            .into_iter()
            .filter(|(id, _)| matches!(self.can_cast(*id), crate::magic::CastCheck::Ok))
            .collect();
        let ids: Vec<u32> = ready.iter().map(|(id, _)| *id).collect();
        let wcid = self
            .world
            .objects
            .get(&guid)
            .map(|o| o.weenie_class_id)
            .unwrap_or(0);
        if let Some((spell, worth)) = ac_world::elements::best_spell(wcid, &name, &ids) {
            let spell_name = ready
                .iter()
                .find(|(id, _)| *id == spell)
                .map(|(_, n)| n.clone())
                .unwrap_or_default();
            // From too far off the server refuses the cast outright, and
            // from behind a wall or a rise the bolt strikes that instead:
            // close in first (see `crate::dodge`, `crate::aim`).
            if self.autoplay_approach(guid, &name, crate::dodge::How::Spell(spell)) {
                return true;
            }
            self.cast_paced(spell, now);
            self.note_fired(spell, now);
            self.autoplay.attack_spell = Some(spell);
            self.throw_at(guid, now);
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
            .map(|(n, id)| format!("{n}: {}", cast_problem(&self.can_cast(id))))
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

    /// The spell this character would throw, before there is a target to
    /// throw it at: the first on offer it can cast this moment, else the
    /// first it would try.
    ///
    /// Which one the fight throws is chosen against the creature
    /// (`ac_world::elements::best_spell`), and that keeps the first of
    /// the ready ones wherever the element table has nothing to say
    /// about it -- so this is what the target pick asks about too.
    pub(super) fn spell_to_throw(&self, cfg: &Fight) -> Option<u32> {
        let offered = self.offered_spells(cfg);
        offered
            .iter()
            .find(|(id, _)| matches!(self.can_cast(*id), crate::magic::CastCheck::Ok))
            .or_else(|| offered.first())
            .map(|(id, _)| *id)
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
