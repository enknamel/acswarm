use std::time::Instant;

use crate::{attack_unanswered, magic, Client, Stance};

impl Client {
    /// Cast a spell (see [`Client::try_cast`]); the outcome is logged.
    pub fn cast(&mut self, spell: u32) {
        let _ = self.try_cast(spell);
    }

    /// Cast a spell on the selected target (or ourselves), switching to
    /// magic mode first when needed. `CastCheck::Ok` once the cast was
    /// sent. Without a wielded caster nothing is sent (`NoCaster`: the
    /// server would only drop us back to peace mode); missing components
    /// refuse the cast only with `require_components` set. An unknown
    /// spell or low mana is logged and sent anyway, the server being the
    /// judge (Mana Conversion can make a cast the estimate rejects).
    pub fn try_cast(&mut self, spell: u32) -> magic::CastCheck {
        use ac_net::messages::action;
        use magic::CastCheck;
        let check = self.can_cast(spell);
        match &check {
            CastCheck::Ok => {}
            CastCheck::NoCaster => {
                tracing::warn!("cannot cast {spell}: no magic caster wielded");
                return check;
            }
            CastCheck::MissingComponents(missing) if self.require_components => {
                tracing::warn!("cannot cast {spell}: missing components {missing:?}");
                return check;
            }
            other => tracing::info!("casting {spell} although {other:?}"),
        }
        let table = self.assets.spell_table().ok();
        let entry = table.as_ref().and_then(|t| t.get(spell));
        let name = entry
            .map(|sp| sp.name.clone())
            .or_else(|| self.known_spells.get(&spell).cloned())
            .unwrap_or_default();
        // Spells that need a target go to the selected creature (or
        // ourselves when nothing is selected); the rest are self casts.
        let needs_target = entry
            .map(|sp| sp.needs_target())
            .unwrap_or_else(|| name.contains("Other"));
        let target = if needs_target {
            self.selected.or(self.world.player_guid)
        } else {
            None
        };
        // The selection can go stale between the tick that made it and
        // this one: the creature dies and is replaced by a corpse of
        // another guid. ACE looks the target up before it starts the
        // windup and answers TargetNotAcquired when it is not there
        // (`Player_Magic.cs`, `HandleActionCastTargetedSpell`), so the
        // cast is a wasted message and a wasted UseDone. The swing makes
        // exactly this test before it goes out; the cast never did, and
        // spent 36 of them in one run. Ourselves we never doubt: a self
        // cast is aimed at a character the server has in front of it by
        // definition, and asking the world about our own object would
        // only find new ways to refuse a buff.
        let gone = target
            .filter(|t| Some(*t) != self.world.player_guid)
            .is_some_and(|t| !self.target_still_there(t));
        if gone {
            tracing::debug!("not casting {name}: the target is gone");
            return CastCheck::NoTarget;
        }
        // A caster is wielded (can_cast said so), so entering combat is
        // entering magic mode; there is no separate choice to make.
        self.enter_combat();
        self.note_cast(spell);
        // The next UseDone is the cast's, not the answer to a use.
        self.last_used = None;
        match target {
            Some(t) => {
                tracing::info!("cast {name} ({spell}) on {t:#010x}");
                self.session.send_action(
                    action::CAST_TARGETED_SPELL,
                    &ac_net::messages::cast_targeted(t, spell),
                );
                // Keep the target's health bar moving (see query_health).
                let creature = self
                    .world
                    .objects
                    .get(&t)
                    .is_some_and(|o| o.item_type & ac_world::item_type::CREATURE != 0);
                if creature {
                    self.query_health(t);
                }
            }
            None => {
                tracing::info!("cast {name} ({spell})");
                self.session
                    .send_action(action::CAST_UNTARGETED_SPELL, &spell.to_le_bytes());
            }
        }
        CastCheck::Ok
    }

    /// Cast `spell` on a particular thing rather than on whatever is
    /// selected: an item being enchanted, a creature being softened.
    /// The selection is left as it was.
    pub fn cast_at(&mut self, spell: u32, target: u32) -> magic::CastCheck {
        let was = self.selected;
        self.selected = Some(target);
        let r = self.try_cast(spell);
        self.selected = was;
        r
    }

    pub fn attack(&mut self, guid: u32) {
        self.interrupt_travel("attacking");
        use ac_net::messages::action;
        // A wand does not swing and does not shoot: with a caster in
        // hand the way to attack is to cast.
        if self.combat_stance() == Stance::Magic {
            tracing::info!("cannot attack with a caster wielded; cast instead");
            return;
        }
        if !self.combat {
            return;
        }
        let name = self.world.name_of(guid).unwrap_or_default().to_string();
        tracing::info!(
            "attack {name} ({guid:#010x}){}",
            if self.missile { " with a missile" } else { "" }
        );
        self.last_target_name = name.clone();
        let mut w = ac_net::wire::Writer::new();
        w.u32(guid)
            .u32(self.attack_height)
            .f32(self.attack_power.clamp(0.0, 1.0));
        let opcode = if self.missile {
            action::TARGETED_MISSILE_ATTACK
        } else {
            action::TARGETED_MELEE_ATTACK
        };
        self.session.send_action(opcode, &w.finish());
        self.attack_target = Some(guid);
        self.attack_pending = true;
        self.attacked = Some(guid);
        self.last_attack = Instant::now();
        self.select(Some(guid));
        self.query_health(guid);
    }

    /// Select a creature on the server (QueryHealth 0x01BF): it answers
    /// UpdateHealth now and again every heartbeat (5 s) while the
    /// creature stays selected, which is the only way its health bar
    /// keeps moving between our own blows. Sent by `attack` and by a
    /// targeted cast; `None` clears the selection.
    pub fn query_health(&mut self, guid: impl Into<Option<u32>>) {
        let guid = guid.into().unwrap_or(0);
        self.session
            .send_action(ac_net::messages::action::QUERY_HEALTH, &guid.to_le_bytes());
    }

    /// Stop the attack in progress (CancelAttack 0x01B7): the server
    /// ends the swing chain with AttackDone and cancels its walk toward
    /// the target. Combat mode stays on.
    pub fn cancel_attack(&mut self) {
        if self.attack_target.take().is_some() || self.attack_pending {
            tracing::info!("cancel attack");
        }
        self.attack_pending = false;
        self.session
            .send_action(ac_net::messages::action::CANCEL_ATTACK, &[]);
    }

    /// Whether something we mean to act on is still there to act on: in
    /// view, and not a creature that has died. A dead creature does not
    /// linger under its own guid -- the server replaces it with a corpse
    /// of another -- so the guid simply goes, and anything still aimed
    /// at it is refused.
    pub fn target_still_there(&self, guid: u32) -> bool {
        self.world
            .objects
            .get(&guid)
            .is_some_and(|o| o.alive_or_unknown())
    }

    pub fn tick_combat(&mut self) {
        let Some(target) = self.attack_target else {
            return;
        };
        let alive = self.target_still_there(target);
        // Dead: the server drops us to peace mode and refuses attacks
        // until we stand at the lifestone.
        let dead = !self.world.stats.name.is_empty() && self.world.stats.vitals[0].current == 0;
        if dead && self.combat {
            tracing::info!("dead: leaving combat");
            self.combat = false;
            self.missile = false;
            self.magic = false;
        }
        if !alive || !self.combat {
            tracing::info!("attack target gone");
            self.attack_target = None;
            self.attack_pending = false;
            return;
        }
        // Re-sending while the server walks us to the target would cancel
        // that walk, so wait for it; back off after a refused attack.
        if !self.attack_pending
            && self.move_to.is_none()
            && self.last_attack.elapsed() > self.attack_backoff
        {
            // A change of hands has been waiting for this attack to be
            // answered, and this is the gap between two swings it was
            // waiting for (see [`Client::wait_for_the_swing`]). It has
            // this tick; the next swing goes out on the one after. One
            // tick only, so nothing can hold the character's sword arm.
            if std::mem::take(&mut self.wants_the_hands) {
                return;
            }
            // And a swap already under way holds the swing until it
            // lands: between the put and the wield the hands are empty,
            // and a swing sent into that gap is a punch. The picker
            // asks this before it joins a fight; the fight it is
            // already in comes back through here, so it asks too.
            // Bounded by SWAP_SETTLES, so a swap that never lands
            // cannot stop the character fighting.
            if self.hands_changing(Instant::now()) {
                return;
            }
            self.attack(target);
        }
    }

    /// Wield the carried item whose name starts with `name` (the first
    /// exact match wins); nothing happens when it is wielded already.
    /// Returns false when no such item is carried.
    pub fn wield_by_name(&mut self, name: &str) -> bool {
        use ac_net::messages::action;
        let me = self.world.player_guid;
        let Some((guid, locations, wielded)) = self
            .world
            .objects
            .values()
            .filter(|o| self.world.is_carried(o.guid) || o.wielder == me)
            .filter(|o| o.name.starts_with(name))
            .min_by_key(|o| if o.name == name { 0 } else { 1 })
            .map(|o| (o.guid, o.valid_locations, o.wielder == me))
        else {
            return false;
        };
        if !wielded {
            let mut w = ac_net::wire::Writer::new();
            w.u32(guid).u32(locations);
            self.session
                .send_action(action::GET_AND_WIELD_ITEM, &w.finish());
        }
        true
    }

    /// What this character can bring to bear, for judging what it may
    /// wield and how well (see `crate::weapons`).
    pub fn wielder(&self) -> crate::weapons::Wielder {
        let stats = &self.world.stats;
        let table = self.assets.skill_table().ok();
        let skills = stats
            .skills
            .iter()
            .map(|s| {
                let base = table.as_ref().and_then(|t| t.get(s.id));
                (
                    s.id,
                    stats.skill_value(s, base),
                    stats.skill_current(s, base),
                    s.advancement,
                )
            })
            .collect();
        let mut attributes = [0u32; 6];
        let mut attributes_current = [0u32; 6];
        for (i, a) in stats.attributes.iter().enumerate() {
            attributes[i] = a.value();
            attributes_current[i] = stats.attribute_current(i as u32 + 1);
        }
        let mut vitals = [0u32; 3];
        for (i, v) in vitals.iter_mut().enumerate() {
            *v = stats.vital_max_current(i);
        }
        crate::weapons::Wielder {
            level: stats.level.max(0) as u32,
            skills,
            attributes,
            attributes_current,
            vitals,
        }
    }

    /// Wield a carried item by guid, in whatever slot it goes in. True
    /// when the item is in hand or on its way there.
    ///
    /// Nothing is sent for an item already wielded, nor for one asked
    /// for so recently that the answer may still be in flight (see
    /// `Autoplay::wield_in_flight`) -- both of those the server
    /// answers with "You must remove your Slashing Sceptre to wield
    /// Slashing Sceptre" and a refusal that then backs the weapon off.
    /// Nothing is sent either while the item is inside a refusal's wait
    /// or while the server has us busy (see [`Client::wield_must_wait`]),
    /// and those two return false: the caller keeps the errand and asks
    /// again on a free tick.
    pub fn wield_guid(&mut self, guid: u32) -> bool {
        use ac_net::messages::action;
        let now = Instant::now();
        let me = self.world.player_guid;
        // Already in hand. Asking again is not a wasted message but a
        // harmful one: the refusal it earns backs the weapon off for
        // three seconds, then six, then twelve, and takes the wand the
        // buff pass needs with it.
        if self
            .world
            .objects
            .get(&guid)
            .is_some_and(|o| o.wielder == me)
        {
            return true;
        }
        if self.autoplay.wield_in_flight(guid, now) {
            return true;
        }
        let Some(locations) = self
            .world
            .objects
            .get(&guid)
            .filter(|o| self.world.is_carried(o.guid))
            .map(|o| o.valid_locations)
        else {
            return false;
        };
        if self.wield_must_wait(guid, now) {
            return false;
        }
        let mut w = ac_net::wire::Writer::new();
        w.u32(guid).u32(locations);
        self.session
            .send_action(action::GET_AND_WIELD_ITEM, &w.finish());
        self.autoplay.wield_asked = Some((guid, now));
        true
    }

    /// The server has refused to move `item`. When that is the answer to
    /// a wield we asked for, decide what the refusal is worth.
    ///
    /// A refusal for something now in our own hands is the answer to an
    /// ask the server had already granted: the client asked twice before
    /// the first answer came back, and the second ask was refused for
    /// the first one having worked ("You must remove your Slashing
    /// Sceptre to wield Slashing Sceptre"). Backing the weapon off for
    /// that is worse than useless. [`Client::hold_off_wield`] clears
    /// `wield_asked`, and with it the check in
    /// [`Client::autoplay_pending_wield`] that would have forgotten the
    /// wait once the item was seen wielded -- so the wait only ever
    /// doubles, and the wand the buff pass needs is never taken up
    /// again. Nine characters went a whole run without casting Prodigal
    /// Strength once, behind a back-off against a wield that had worked.
    pub(crate) fn wield_refused(&mut self, item: u32, now: Instant) {
        if !self.autoplay.wield_asked.is_some_and(|(g, _)| g == item) {
            return;
        }
        let me = self.world.player_guid;
        if self
            .world
            .objects
            .get(&item)
            .is_some_and(|o| o.wielder == me)
        {
            self.autoplay.wield_asked = None;
            self.autoplay.wield_refused.forget(&item);
        } else {
            self.hold_off_wield(item, now);
        }
    }

    /// Whether the server has the character busy, so that a take sent
    /// now would be thrown away.
    ///
    /// ACE refuses a take made while `IsBusy` is set and answers with
    /// YoureTooBusy plus a wasted InventoryServerSaveFailed
    /// (`Player_Inventory.cs`, `HandleActionPutItemInContainer_Verify`).
    /// What sets `IsBusy` is what the server owes us a UseDone for: a
    /// spell being wound up (`MagicState.OnCastStart`), a recipe, a
    /// counter's sale. Nine characters bought eighty-nine "You're too
    /// busy" pairs in ten minutes taking from a body with a spell in the
    /// air.
    ///
    /// A swing is not one of them, whatever this used to say: nothing in
    /// ACE's melee or missile path sets `IsBusy`, and the wield handler
    /// does not read it at all -- the check in
    /// `HandleActionGetAndWieldItem` is commented out. The reason not to
    /// change hands mid-swing is a different one and is stated where it
    /// belongs (see [`Client::wield_must_wait`]).
    pub fn server_busy(&self, now: Instant) -> bool {
        self.autoplay.cast_in_flight(now)
    }

    /// Whether a wield of `guid` sent now would be wasted: the item is
    /// inside the wait a refusal earned it, the server has the character
    /// busy, or there is a swing in the air. Either way the errand keeps
    /// and goes out later.
    ///
    /// The swing is not the server refusing anything -- it would make
    /// the wield, and cancel the attack doing it (see
    /// `autoplay::change_of_hands_waits`). A weapon is not worth a cancelled
    /// swing when the gap between two of them is a few hundred
    /// milliseconds away.
    pub fn wield_must_wait(&self, guid: u32, now: Instant) -> bool {
        self.wield_held_off(guid)
            || self.server_busy(now)
            || attack_unanswered(self.attack_pending, self.last_attack, now)
    }

    /// Whether this item is inside the wait a refused wield earned it.
    ///
    /// The server refuses a wield it will not make with no error code
    /// at all, so there is nothing to read in the refusal and nothing to
    /// do but wait and try again later. Without this the buff pass asked
    /// for +Verity's Training Wand a hundred and fifty times in a
    /// minute, and was refused every one of them.
    pub fn wield_held_off(&self, guid: u32) -> bool {
        self.autoplay.wield_refused.held(&guid, Instant::now())
    }

    /// The arrows, bolts or quarrels wielded, if any. A bow shoots
    /// nothing without them, so autoplay refills the slot from the pack.
    pub fn wielded_ammo(&self) -> Option<u32> {
        self.world
            .wielded()
            .find(|o| o.valid_locations & ac_world::equip::MISSILE_AMMO != 0)
            .map(|o| o.guid)
    }

    /// Wield ammunition from the packs, the largest stack first. False
    /// when none is carried, or some is wielded already. Thrown weapons
    /// are their own ammunition and need none, so this does nothing for
    /// them.
    pub fn wield_ammo(&mut self) -> bool {
        use ac_net::messages::action;
        if self.wielded_ammo().is_some() {
            return false;
        }
        let Some((guid, locations, name)) = self
            .world
            .objects
            .values()
            .filter(|o| self.world.is_carried(o.guid))
            .filter(|o| o.valid_locations & ac_world::equip::MISSILE_AMMO != 0)
            .max_by_key(|o| o.stack_size.max(1))
            .map(|o| (o.guid, o.valid_locations, o.name.clone()))
        else {
            return false;
        };
        tracing::info!("wielding ammunition: {name} ({guid:#010x})");
        let mut w = ac_net::wire::Writer::new();
        w.u32(guid).u32(locations & ac_world::equip::MISSILE_AMMO);
        self.session
            .send_action(action::GET_AND_WIELD_ITEM, &w.finish());
        true
    }

    /// Whether a swing or a charge is out and unanswered (see
    /// `attack_unanswered`). While one is, the rules leave the
    /// character's hands and its combat mode alone: every one of those
    /// changes cancels the attack in flight.
    ///
    /// The few changes worth a cancelled attack say so themselves --
    /// dropping to peace to loot a body, fleeing, coming back from a
    /// death -- and they do not ask.
    pub fn mid_attack(&self) -> bool {
        attack_unanswered(self.attack_pending, self.last_attack, Instant::now())
    }

    /// Book the gap after the swing in flight for whatever wanted the
    /// character's hands and found them busy.
    ///
    /// Without this the wait would never end: the client swings again
    /// the moment AttackDone arrives, so the rules would find an attack
    /// in flight every time they looked. Booked, the next swing is held
    /// back one tick and the change goes in between two of them, which
    /// is where the server wanted it all along.
    pub fn wait_for_the_swing(&mut self) {
        self.wants_the_hands = true;
    }

    /// The way this character fights right now, which is decided by
    /// what is in its hands and by nothing else: a wand, orb or staff
    /// means magic, a bow, crossbow or thrown weapon means missile, and
    /// anything else -- a sword, a mace, bare fists -- means melee.
    /// Entering combat enters the stance the hands imply; to fight
    /// another way, wield another weapon.
    pub fn combat_stance(&self) -> Stance {
        if self.wielded_caster().is_some() {
            Stance::Magic
        } else if self.wielded_missile_weapon().is_some() {
            Stance::Missile
        } else {
            Stance::Melee
        }
    }

    /// Enter combat, in whichever stance the wielded weapon gives.
    /// Nothing is sent when we are already in it.
    pub fn enter_combat(&mut self) {
        use ac_net::messages::{action, combat_mode};
        let stance = self.combat_stance();
        let already = match stance {
            Stance::Magic => self.magic,
            Stance::Missile => self.combat && self.missile,
            Stance::Melee => self.combat && !self.missile,
        };
        if already {
            return;
        }
        self.combat = stance != Stance::Magic;
        self.magic = stance == Stance::Magic;
        self.missile = stance == Stance::Missile;
        let mode = match stance {
            Stance::Melee => combat_mode::MELEE,
            Stance::Missile => combat_mode::MISSILE,
            Stance::Magic => combat_mode::MAGIC,
        };
        tracing::info!("combat mode {}", stance.label());
        self.session
            .send_action(action::CHANGE_COMBAT_MODE, &mode.to_le_bytes());
    }

    /// Send the combat mode the character already holds. True when
    /// something went out: in peace there is no mode to send. What it is
    /// for is the way through the server's handler, not the mode itself.
    ///
    /// `HandleActionChangeCombatMode_Inner` fails a cast still being
    /// wound up before it looks at the mode asked for
    /// (Player_Combat.cs:787-788), while `SetCombatMode` returns at once
    /// with no animation for a stance the character is already in
    /// (Creature_Combat.cs:67-71). So this costs one message and changes
    /// nothing else, where `leave_combat` would drop the stance, the
    /// swing's target and the pending attack as well.
    pub(crate) fn resend_combat_mode(&mut self) -> bool {
        use ac_net::messages::{action, combat_mode};
        let mode = if self.magic {
            combat_mode::MAGIC
        } else if self.combat && self.missile {
            combat_mode::MISSILE
        } else if self.combat {
            combat_mode::MELEE
        } else {
            return false;
        };
        self.session
            .send_action(action::CHANGE_COMBAT_MODE, &mode.to_le_bytes());
        true
    }

    /// Wield a carried weapon that gives the stance `want`, so the
    /// character fights that way. True when something was sent. Nothing
    /// happens when the hands already give that stance, or when no such
    /// weapon is carried: a character can only fight with what it has.
    pub fn wield_for(&mut self, want: Stance) -> bool {
        use ac_net::messages::action;
        use ac_world::item_type;
        if self.combat_stance() == want {
            return false;
        }
        let mask = match want {
            Stance::Magic => item_type::CASTER,
            Stance::Missile => item_type::MISSILE_WEAPON,
            Stance::Melee => item_type::MELEE_WEAPON,
        };
        let Some((guid, locations, name)) = self
            .world
            .objects
            .values()
            .filter(|o| self.world.is_carried(o.guid))
            .filter(|o| o.item_type & mask != 0)
            // A stack of arrows is a missile weapon by type and worth
            // more than a training bow; the launcher is wanted here.
            .filter(|o| o.valid_locations & ac_world::equip::MISSILE_AMMO == 0)
            // The dearest one is usually the best one, and it is the
            // only ordering the client has before appraising.
            .max_by_key(|o| o.value)
            .map(|o| (o.guid, o.valid_locations, o.name.clone()))
        else {
            return false;
        };
        let now = Instant::now();
        // The same weapon, asked for a tick or two ago and not answered
        // yet. The swap is under way; asking again only buys "You must
        // remove your Slashing Sceptre to wield Slashing Sceptre" and a
        // back-off against a wield that had in fact worked -- which is
        // how nine characters went ten minutes without casting Prodigal
        // Strength once.
        if self.autoplay.wield_in_flight(guid, now) {
            return true;
        }
        if self.wield_must_wait(guid, now) {
            return false;
        }
        // The server will not put a bow in the hands that hold a wand:
        // whatever weapon is held goes back in the pack first, and the
        // new one is wielded on a later tick once it is there.
        let mut sent = self.put_weapons_away();
        // Nor will it put a wand in the hand of a character whose off
        // hand holds a shield -- it refuses the wield outright, with no
        // error to read. The shield comes off with the weapon, the way
        // the arming code already does it.
        let free_offhand = self
            .stats_of(guid)
            .is_some_and(|i| crate::weapons::needs_free_offhand(&i));
        if free_offhand {
            if let (Some(me), Some(shield)) = (self.world.player_guid, self.wielded_shield()) {
                sent |= self.put_in_container(shield, me);
            }
        }
        // When the swap started, so the fight waits for the hands to
        // settle rather than swinging into the moment they are empty
        // (see `Client::hands_changing`). The arming code stamps this
        // for its own swaps; without it here, every swap the buff pass
        // and the softening start was invisible to the fight, which is
        // how +Verity came to punch a Spikey Armoredillo.
        self.autoplay.last_rewield = Some(now);
        if sent {
            // Taken up by the housekeeping the moment the hands are
            // empty, rather than whenever the caller next happens to ask.
            self.autoplay.pending_wield = Some(guid);
            return true;
        }
        tracing::info!("wielding {name} to fight {}", want.label());
        let mut w = ac_net::wire::Writer::new();
        w.u32(guid).u32(locations);
        self.session
            .send_action(action::GET_AND_WIELD_ITEM, &w.finish());
        self.autoplay.wield_asked = Some((guid, now));
        true
    }

    /// Put every weapon in hand back in the pack. True when something
    /// was sent; the hands are empty on a later tick.
    pub fn put_weapons_away(&mut self) -> bool {
        use ac_world::item_type;
        let Some(me) = self.world.player_guid else {
            return false;
        };
        let held: Vec<u32> = self
            .world
            .wielded()
            .filter(|o| {
                o.item_type
                    & (item_type::MELEE_WEAPON | item_type::MISSILE_WEAPON | item_type::CASTER)
                    != 0
                    && o.valid_locations & ac_world::equip::MISSILE_AMMO == 0
            })
            .map(|o| o.guid)
            .collect();
        let mut sent = false;
        for g in held {
            sent |= self.put_in_container(g, me);
        }
        sent
    }

    /// Drop back to peace mode.
    pub fn leave_combat(&mut self) {
        use ac_net::messages::{action, combat_mode};
        if !self.combat && !self.magic {
            return;
        }
        self.combat = false;
        self.magic = false;
        self.missile = false;
        tracing::info!("combat mode peace");
        self.session.send_action(
            action::CHANGE_COMBAT_MODE,
            &combat_mode::NON_COMBAT.to_le_bytes(),
        );
        self.attack_target = None;
        self.attack_pending = false;
    }

    /// The wielded bow, crossbow or thrown weapon, if any.
    pub fn wielded_missile_weapon(&self) -> Option<u32> {
        // Arrows are missile weapons by type too; the launcher is what
        // is wanted here.
        self.world
            .wielded()
            .find(|o| {
                o.item_type & ac_world::item_type::MISSILE_WEAPON != 0
                    && o.valid_locations & ac_world::equip::MISSILE_AMMO == 0
            })
            .map(|o| o.guid)
    }

    /// Enter or leave combat. Which stance it enters is not a choice:
    /// it is whatever the wielded weapon gives (see `combat_stance`).
    pub fn toggle_combat(&mut self) {
        if self.combat || self.magic {
            self.leave_combat();
        } else {
            self.enter_combat();
        }
    }
}
