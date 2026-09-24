use std::time::{Duration, Instant};

use crate::autoplay::hands::weapon::change_of_hands_waits;
use crate::autoplay::{cast_problem, Buffs, Doing};
use crate::{Client, Stance};

/// Least time between two casts of the same buff.
pub(crate) const BUFF_EVERY: Duration = Duration::from_millis(1500);

/// How near to running out a buff must be, in seconds, before this pass
/// will put it back.
///
/// The quiet pass tops up anything within the wide `top_up_within`. The
/// urgent one runs as a reflex, ahead of loot and ahead of the fight,
/// and puts back whatever is under `never_below` -- but it ignored
/// `out_of_combat_only` outright, which is the player saying the buff
/// pass must keep its hands off mid-fight. A buff with a minute still
/// on it was enough to put the sword away and reach for a wand in the
/// middle of a swing.
///
/// What that setting is protecting is the weapon, so that is what this
/// asks about. With a wand already in hand (`wand_in_hand`) a recast
/// costs a cast and nothing else, and `never_below` holds as it always
/// did: a protection going down in the middle of a fight is how a
/// character dies, and waiting for the buff to lapse before putting it
/// back is exactly the window `never_below` exists to close. It is only
/// when the recast would cost the character its weapon that an engaged
/// character with the setting on waits for a buff to actually lapse --
/// nought seconds left, which is what a buff that is not up at all
/// reads as -- and the rest wait for the fight to end.
fn buff_within(cfg: &Buffs, urgent: bool, fighting: bool, wand_in_hand: bool) -> f32 {
    if !urgent {
        return cfg.top_up_within;
    }
    if fighting && cfg.out_of_combat_only && !wand_in_hand {
        return 0.0;
    }
    cfg.never_below
}

/// How often each buff pass, urgent and top-up, and the rearm work out what is due.
pub(crate) const BUFF_CHECK_EVERY: Duration = Duration::from_millis(1000);

impl Client {
    /// Seconds left on the enchantment of a spell family, if any is up.
    fn buff_left(&self, spell: u32) -> Option<f32> {
        let table = self.assets.spell_table().ok();
        let sp = table.as_ref().and_then(|t| t.get(spell));
        match sp {
            Some(sp) => self.category_left(sp.category, sp.power),
            None => self.spell_left(spell),
        }
    }

    /// Seconds left on this exact spell: `None` when it is not up,
    /// infinity when it never runs out.
    fn spell_left(&self, spell: u32) -> Option<f32> {
        self.longest_left(|e| e.spell_id as u32 == spell)
    }

    /// Seconds left on any enchantment of this category at least as
    /// strong as `power`: Strength Self VI already up means Strength
    /// Self IV is not wanted, and the other way round it is.
    fn category_left(&self, category: u32, power: u32) -> Option<f32> {
        self.longest_left(|e| e.category as u32 == category && e.power >= power)
    }

    /// The longest any enchantment passing `keep` has left. A quest or
    /// item enchantment with no end has a duration below zero and is
    /// worth infinity here: treating it as run out had a character
    /// recasting a permanent buff every two seconds. Without the
    /// server's clock nothing can be said, and nothing is due.
    fn longest_left(&self, keep: impl Fn(&ac_world::stats::Enchantment) -> bool) -> Option<f32> {
        let now = self.session.server_time()?;
        self.world
            .stats
            .enchantments
            .iter()
            .filter(|e| keep(e))
            .map(|e| match e.remaining(now) {
                Some(left) => left as f32,
                None => f32::INFINITY,
            })
            .fold(None, |acc: Option<f32>, left| {
                Some(acc.map_or(left, |a| a.max(left)))
            })
    }

    /// Seconds left on an enchantment we put on an item, by category,
    /// from when we cast it and how long the spell lasts.
    fn item_buff_left(&self, item: u32, category: u32, now: Instant) -> Option<f32> {
        self.autoplay
            .item_buffs
            .iter()
            .filter(|(g, c, _, _)| *g == item && *c == category)
            .map(|(_, _, when, lasts)| lasts - now.duration_since(*when).as_secs_f32())
            .fold(None, |acc: Option<f32>, left| {
                Some(acc.map_or(left, |a| a.max(left)))
            })
    }

    /// The buffs this character should be wearing right now, worked out
    /// from what it is (see `crate::buffs::wanted`).
    pub fn wanted_buffs(&self) -> Vec<crate::buffs::Want> {
        // With the components and mana to try: a wand not yet in hand
        // is the one lack that does not count, since wielding one is
        // the first thing done. The pack is counted once, not per spell.
        let carried = self.components();
        self.wanted_buffs_if(|id| {
            matches!(
                self.can_cast_from(id, Some(&carried)),
                crate::magic::CastCheck::Ok | crate::magic::CastCheck::NoCaster
            )
        })
    }

    /// The same, with `ready` saying which spells may be counted on
    /// besides being likely enough to land. The buffing asks for what
    /// can be cast this moment; the restock list asks for what would
    /// be cast once the pack were filled (`growth::spells_cast`), which
    /// is a different question with the same answer otherwise.
    pub(crate) fn wanted_buffs_if(&self, ready: impl Fn(u32) -> bool) -> Vec<crate::buffs::Want> {
        let Ok(table) = self.assets.spell_table() else {
            return Vec::new();
        };
        let trained = crate::buffs::trained_skills(&self.world.stats.skills);
        let least = self.autoplay.config.buffs.least_chance;
        // Likely enough to land, and ready by the caller's measure.
        let usable = |id: u32| self.cast_chance(id) >= least && ready(id);
        // Anything the armour spells could harden: armour, clothing or
        // a shield, since a cast at ourselves lands on all of it.
        let wears_armour = self.world.wielded().any(|o| {
            o.item_type & (ac_world::item_type::ARMOR | ac_world::item_type::CLOTHING) != 0
                || o.valid_locations & ac_world::equip::SHIELD != 0
        });
        // The weapon in hand decides which weapon skill is worth a buff.
        let weapon_skill = match self.combat_stance() {
            Stance::Magic => Some(0),
            _ => self
                .world
                .wielded()
                .find(|o| {
                    o.item_type
                        & (ac_world::item_type::MELEE_WEAPON | ac_world::item_type::MISSILE_WEAPON)
                        != 0
                })
                .and_then(|o| self.stats_of(o.guid))
                .map(|i| i.weapon_skill_id)
                .filter(|id| *id != 0),
        };
        let me = crate::buffs::Character {
            known: &self.world.stats.spells,
            trained: &trained,
            stance: self.combat_stance(),
            guid: self.world.player_guid.unwrap_or(0),
            wears_armour,
            usable: &usable,
            weapon_skill,
        };
        crate::buffs::wanted(&table, &me)
    }

    /// Whether a target is being fought, by weapon or by spell: a fight, to the buff passes.
    fn has_target(&self) -> bool {
        self.attack_target.is_some() || self.autoplay.casting_at.is_some()
    }

    /// `buff_within` for this moment, the fight and the hands read as the passes read them.
    fn buff_within_now(&self, urgent: bool) -> f32 {
        let wand_in_hand = self.combat_stance() == Stance::Magic;
        buff_within(
            &self.autoplay.config.buffs,
            urgent,
            self.has_target(),
            wand_in_hand,
        )
    }

    /// Whether the urgent pass would put a buff back now, by its own `buff_within`: a
    /// weapon put down for one waits on nothing that pass would leave down.
    pub(crate) fn has_urgent_buff_due(&self, now: Instant) -> bool {
        self.due_buff(self.buff_within_now(true), now).is_some()
    }

    /// Put a buff back up. True when it cast one.
    ///
    /// Two passes share this. The urgent one runs before anything else
    /// each tick and puts back whatever is under `never_below`, in a
    /// fight or out of one, swapping to a wand if the hands hold
    /// something else: a buff is never allowed to run out. The other
    /// runs last, in quiet moments, and tops up whatever is under the
    /// much wider `top_up_within`, a cast or two at a time, so the set
    /// is refreshed a little at every lull rather than all at once.
    pub(crate) fn autoplay_buff(&mut self, now: Instant, urgent: bool) -> bool {
        let cfg = self.autoplay.config.buffs.clone();
        if cfg.spells.is_empty() && !cfg.auto {
            return false;
        }
        let fighting = self.has_target();
        // At a counter every cast waits, urgent or not, for the reason
        // the top-ups wait on a journey and one more: a cast roots the
        // character where it stands, and here it also has the counter
        // turn us away. ACE will not open a window for, sell for or
        // buy for a character in a cast's recoil (`Vendor.ActOnUse`,
        // `Player_Commerce.cs`: IsBusy is YoureTooBusy), and the answer
        // reads as the counter's own refusal. +Vesperi arrived at
        // Archmage Cindrue with eight protections lapsing, cast them
        // one after another, and the run gave the counter up as one
        // that "would not trade" with every pea still in the pack. A
        // cast does not close the window -- the server keeps none, and
        // `open_vendor` is ours -- but a sale sent during one is thrown
        // away the same way, so nothing goes up between the acts
        // either: the visit is short, and what lapses in it goes back
        // up the moment the window closes. A fight at the counter is
        // the one exception (see `counter_holding_casts`).
        //
        // Judged before the other holds so that every call not made at
        // a counter lets go of the note's flag, whatever else keeps the
        // pass from casting: cleared only on the way past the hold, it
        // outlived a visit that the fight hold ended, and the next
        // visit's wait went unsaid.
        if let Some(counter) = self.counter_holding_casts() {
            if !self.autoplay.buffs_held_at_counter {
                self.autoplay.buffs_held_at_counter = true;
                self.autoplay.note(
                    format!("buffs wait for the counter: a cast would have {counter} turn us away"),
                    now,
                );
            }
            return false;
        }
        self.autoplay.buffs_held_at_counter = false;
        if !urgent && cfg.out_of_combat_only && fighting {
            return false;
        }
        // On a journey the top-ups wait: every cast roots the character
        // where it stands, and a character with a hundred buffs to put
        // back would never leave town. What is about to run out still
        // goes back up on the way.
        if !urgent && self.traveling() {
            return false;
        }
        if self
            .autoplay
            .last_buff
            .is_some_and(|t| now.duration_since(t) < BUFF_EVERY)
        {
            return false;
        }
        // Each pass looks once a BUFF_CHECK_EVERY on its own clock, so a
        // top-up that holds off cannot stop the urgent one looking. A top-up
        // paced out in a shorter lull waits for a later one, perhaps a fight
        // on: top_up_within, far wider than never_below, leaves that room.
        let checked = if urgent {
            self.autoplay.urgent_buffs_checked
        } else {
            self.autoplay.top_ups_checked
        };
        if checked.is_some_and(|t| now.duration_since(t) < BUFF_CHECK_EVERY) {
            return false;
        }
        // A buff is never worth a cancelled swing: it goes back up in
        // the gap between two of them instead (see
        // `change_of_hands_waits`).
        if change_of_hands_waits(self.combat_stance(), Stance::Magic, self.mid_attack()) {
            self.wait_for_the_swing();
            self.autoplay.note(
                "waiting for the swing to land before reaching for a wand",
                now,
            );
            return false;
        }
        if urgent {
            self.autoplay.urgent_buffs_checked = Some(now);
        } else {
            self.autoplay.top_ups_checked = Some(now);
        }
        let within = self.buff_within_now(urgent);
        // Find what is due before touching the hands: the urgent pass
        // runs every tick and must cost nothing when nothing is due.
        // Worked out once, for the pick and for saying why none is due.
        let wants = self.auto_buffs();
        let Some((spell, target, category, name, lasts)) = self.due_buff_among(&wants, within, now)
        else {
            if !urgent {
                self.autoplay_explain_buffs(&wants, now);
            }
            return false;
        };
        // Mana is kept back for healing and fighting: a top-up waits
        // until there is that much to spare, and even an urgent recast
        // leaves half of it.
        let reserve = self.vital_max_of(2) as f32 * cfg.keep_mana * if urgent { 0.5 } else { 1.0 };
        let cost = self
            .assets
            .spell_table()
            .ok()
            .and_then(|t| t.get(spell).map(|s| self.mana_cost(s)))
            .unwrap_or(0) as f32;
        let have = self.world.stats.vitals[2].current as f32;
        if have - cost < reserve {
            self.autoplay
                .note(format!("holding off {name} to keep mana back"), now);
            return false;
        }
        // A wand is needed to cast. Out of a fight the arming code sorts
        // the hands out afterwards; in one, remember what was put down
        // so it is taken up again the moment the buffing is done.
        if self.combat_stance() != Stance::Magic {
            let held = self
                .world
                .wielded()
                .find(|o| {
                    o.item_type
                        & (ac_world::item_type::MELEE_WEAPON | ac_world::item_type::MISSILE_WEAPON)
                        != 0
                })
                .map(|o| o.guid);
            if !self.wield_for(Stance::Magic) {
                // A wand that is carried but held off after a refusal is
                // a wait, not a want: saying "no wand" for it sent an
                // earlier reader looking through the pack for one.
                //
                // Noted, not said: this rule is standing aside, and the
                // step that does claim the tick writes the status line
                // next. Setting it here only made the two flap, which a
                // live run turned into 667 identical lines in five
                // minutes -- twice a second, enough to bury anything
                // worth reading. `note` says it once a window.
                self.autoplay.note(
                    if self.carries_a_caster() {
                        format!("waiting to take a wand up to cast {name}")
                    } else {
                        format!("no wand to cast {name} with")
                    },
                    now,
                );
                return false;
            }
            if fighting && self.autoplay.put_down.is_none() {
                self.autoplay.put_down = held;
            }
            // The wield takes a moment; cast next tick.
            self.autoplay.last_buff = Some(now);
            return true;
        }
        if !matches!(self.can_cast(spell), crate::magic::CastCheck::Ok) {
            return false;
        }
        // Casting a low level of something we know a better version of
        // is worth saying once: it is nearly always components for the
        // higher formula, and a character quietly buffing at level one
        // looks like a bug rather than an empty pack.
        if let Some((better, why)) = self.better_buff_blocked(spell) {
            self.autoplay
                .note(format!("buffing with a weaker spell: {better} {why}"), now);
        }
        use crate::buffs::Target;
        match target {
            Target::Me => {
                self.cast(spell);
                self.autoplay.say(Doing::Buffing, format!("casting {name}"));
            }
            Target::Item(g) => {
                self.cast_at(spell, g);
                self.autoplay
                    .item_buffs
                    .retain(|(i, c, _, _)| !(*i == g && *c == category));
                self.autoplay.item_buffs.push((g, category, now, lasts));
                let on = self.world.name_of(g).unwrap_or_default().to_string();
                self.autoplay
                    .say(Doing::Buffing, format!("casting {name} on {on}"));
            }
        }
        // A buff is a cast like any other as far as the pacing goes:
        // an attack thrown over it would be dropped.
        self.autoplay.last_buff = Some(now);
        self.autoplay.cast_sent = Some(now);
        true
    }

    /// The counter a cast that can wait is waiting for, by name: the
    /// one the character stands at (see `Client::counter_at_hand`),
    /// unless a fight is on there. A fight at the counter is fought as
    /// anywhere: the run is not stepped while the fight claims the
    /// tick, so a cast in it meets no Use of ours, and one that does
    /// costs a busy re-ask (see `growth::on_opening`) -- while a
    /// protection that lapsed in it, held for a window nobody is
    /// trading at, would not go back up until the fight was over.
    pub(crate) fn counter_holding_casts(&self) -> Option<String> {
        if self.in_a_fight() {
            return None;
        }
        self.counter_at_hand()
    }

    /// A stronger spell than `spell` that we know, do the same thing
    /// with, and cannot cast: its name and why. `None` when the one we
    /// are about to cast is already the best we know.
    fn better_buff_blocked(&self, spell: u32) -> Option<(String, String)> {
        use crate::magic::CastCheck;
        let table = self.assets.spell_table().ok()?;
        let mine = table.get(spell)?;
        let mut best: Option<(u32, String, String)> = None;
        for &id in &self.world.stats.spells {
            let Some(sp) = table.get(id) else { continue };
            // The same buff, only stronger: the category is the family
            // and the power is the level within it.
            if sp.category != mine.category || sp.power <= mine.power {
                continue;
            }
            let check = self.can_cast(id);
            if matches!(check, CastCheck::Ok) {
                continue;
            }
            let why = cast_problem(&check);
            if best.as_ref().is_none_or(|b| sp.power > b.0) {
                best = Some((sp.power, sp.name.clone(), why));
            }
        }
        best.map(|(_, name, why)| (name, why))
    }

    /// When buffs are wanted but none can be cast, say why for the
    /// first of them, so a character standing unbuffed is not a mystery.
    /// `wants` is `wanted_buffs()` as the pass that found none due saw it.
    fn autoplay_explain_buffs(&mut self, wants: &[crate::buffs::Want], now: Instant) {
        use crate::buffs::Target;
        use crate::magic::CastCheck;
        if !self.autoplay.config.buffs.auto {
            return;
        }
        let top_up = self.autoplay.config.buffs.top_up_within;
        for want in wants {
            let left = match want.target {
                Target::Me => self.category_left(want.category, want.power),
                Target::Item(g) => self.item_buff_left(g, want.category, now),
            };
            if left.is_some_and(|l| l > top_up) {
                continue;
            }
            let check = self.can_cast(want.spell);
            if matches!(check, CastCheck::Ok | CastCheck::NoCaster) {
                continue;
            }
            let why = cast_problem(&check);
            let name = self
                .assets
                .spell_table()
                .ok()
                .and_then(|t| t.get(want.spell).map(|s| s.name.clone()))
                .unwrap_or_default();
            self.autoplay
                .note(format!("cannot buff: {name}, {why}"), now);
            return;
        }
    }

    /// The buff with the least time left of those under `within`
    /// seconds, castable or not: the most pressing one is put back
    /// first. `(spell, target, category, name, seconds it lasts)`.
    pub(crate) fn due_buff(
        &self,
        within: f32,
        now: Instant,
    ) -> Option<(u32, crate::buffs::Target, u32, String, f32)> {
        self.due_buff_among(&self.auto_buffs(), within, now)
    }

    /// `wanted_buffs()` when the config's `auto` is on, else none.
    fn auto_buffs(&self) -> Vec<crate::buffs::Want> {
        if self.autoplay.config.buffs.auto {
            self.wanted_buffs()
        } else {
            Vec::new()
        }
    }

    /// The same as `due_buff`, from `wants` as `auto_buffs` gave them.
    fn due_buff_among(
        &self,
        wants: &[crate::buffs::Want],
        within: f32,
        now: Instant,
    ) -> Option<(u32, crate::buffs::Target, u32, String, f32)> {
        use crate::buffs::Target;
        let cfg = &self.autoplay.config.buffs;
        let table = self.assets.spell_table().ok();
        // (rank, left, ...): creature magic goes first. Its buffs raise
        // the skills and attributes the other schools cast from, so a
        // character that buffs them first can land higher levels of
        // everything after.
        let mut due: Option<(u8, f32, u32, Target, u32, String, f32)> = None;
        let clock = self.session.server_time().is_some();
        let creature_first = |spell: u32| -> u8 {
            let school = table.as_ref().and_then(|t| t.get(spell)).map(|s| s.school);
            u8::from(school != Some(ac_formats::spell_table::school::CREATURE))
        };
        let mut offer = |left: Option<f32>, spell: u32, target: Target, category: u32| {
            // Not up at all is due now; but until the server's clock is
            // known nothing can be told apart, so nothing is due.
            let left = match left {
                Some(l) => l,
                None if clock => 0.0,
                None => return,
            };
            if left > within {
                return;
            }
            // One that cannot be cast must not stand in front of the
            // rest: short of components or mana it is passed over. No
            // wand is different, since wielding one is the cure.
            match self.can_cast(spell) {
                crate::magic::CastCheck::Ok | crate::magic::CastCheck::NoCaster => {}
                _ => return,
            }
            let rank = creature_first(spell);
            if due.as_ref().is_some_and(|d| (d.0, d.1) <= (rank, left)) {
                return;
            }
            let sp = table.as_ref().and_then(|t| t.get(spell));
            let name = sp.map(|s| s.name.clone()).unwrap_or_default();
            let lasts = sp.and_then(|s| s.duration()).unwrap_or(1800.0) as f32;
            due = Some((rank, left, spell, target, category, name, lasts));
        };
        for want in wants {
            let left = match want.target {
                Target::Me => self.category_left(want.category, want.power),
                Target::Item(g) => self.item_buff_left(g, want.category, now),
            };
            offer(left, want.spell, want.target, want.category);
        }
        for name in &cfg.spells {
            let Some(spell) = self.spell_by_name(name) else {
                continue;
            };
            let category = table
                .as_ref()
                .and_then(|t| t.get(spell))
                .map(|s| s.category)
                .unwrap_or(0);
            offer(self.buff_left(spell), spell, Target::Me, category);
        }
        due.map(|(_, _, spell, target, category, name, lasts)| {
            (spell, target, category, name, lasts)
        })
    }
}

#[cfg(test)]
mod tests;
