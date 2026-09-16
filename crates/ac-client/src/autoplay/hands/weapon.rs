use std::time::{Duration, Instant};

use crate::autoplay::{Autoplay, Fight, Style};
use crate::{Client, Stance};

/// Whether a change of hands must wait for the swing in flight.
///
/// True only when both hold: the hands do not already give the stance
/// wanted, so something would have to be wielded or put away; and a
/// swing or a charge is out unanswered. ACE turns every combat-mode
/// change into a cancelled attack, and putting the weapon in hand away
/// to reach for a wand is one -- so a buff pass that wants a wand waits
/// for the swing to land rather than taking the charge down with it. A
/// pass that already holds a wand changes nothing and casts at once.
///
/// The wait costs a second of a buff's life. +Verity's cost her the
/// fight: she reached for her wand every second and a half for a minute,
/// and the Drudge Servant she was charging was never once reached.
pub(crate) fn change_of_hands_waits(have: Stance, want: Stance, mid_attack: bool) -> bool {
    have != want && mid_attack
}

/// A change of weapon is asked for at most this often.
const REWIELD_EVERY: Duration = Duration::from_millis(1000);

/// How long an item the server refused to wield is left alone the first
/// time. It doubles with every refusal after that. Short to begin with
/// on purpose: most refusals are about what else is in the hands, and
/// that changes within a few seconds.
const WIELD_AGAIN: Duration = Duration::from_secs(3);

/// How long a wield already asked for is given to be answered before it
/// is worth asking again.
///
/// The client thinks at 8 Hz and the server's word takes a few hundred
/// milliseconds to come back, so without this the same weapon goes out
/// two or three times over and every ask after the first is refused --
/// for the first one having worked. Nine characters sent 81 wields in
/// ten minutes and were refused 71 of them, every refusal reading "You
/// must remove your Slashing Sceptre to wield Slashing Sceptre".
pub(crate) const WIELD_ANSWERS_IN: Duration = Duration::from_millis(1500);

/// How long a weapon swap is given to land before the character gives
/// up waiting and fights with whatever is in its hands. A put and a
/// wield are a tick or two; anything longer means the swap is stuck.
pub(crate) const SWAP_SETTLES: Duration = Duration::from_millis(1500);

/// How long dropping to peace mode takes on the server, which will not
/// craft in any other stance.
pub(super) const STANCE_CHANGE: Duration = Duration::from_millis(1000);

impl Autoplay {
    /// Whether this item was asked for a moment ago and the server's
    /// word could still be on its way (see [`WIELD_ANSWERS_IN`]).
    ///
    /// Asking twice for one weapon is not a wasted message but a
    /// harmful one: the second ask is refused because the first worked,
    /// and the refusal backs the item off for three seconds and then
    /// six and then twelve (see `Client::hold_off_wield`).
    pub(crate) fn wield_in_flight(&self, guid: u32, now: Instant) -> bool {
        self.wield_asked
            .is_some_and(|(g, t)| g == guid && now.duration_since(t) < WIELD_ANSWERS_IN)
    }
}

impl Client {
    /// A weapon waiting for empty hands is taken up as soon as they
    /// are: a bow cannot be drawn with a shield up, and a two-handed
    /// weapon needs both. Runs every tick and never claims one.
    ///
    /// This is also where a wield that did land forgets whatever wait a
    /// refusal earned it: the hands have changed, so whatever the server
    /// was objecting to has gone.
    pub(crate) fn autoplay_pending_wield(&mut self, now: Instant) {
        if let Some((g, _)) = self.autoplay.wield_asked {
            let me = self.world.player_guid;
            if self.world.objects.get(&g).is_some_and(|o| o.wielder == me) {
                self.autoplay.wield_refused.forget(&g);
                self.autoplay.wield_asked = None;
            }
        }
        if let Some(g) = self.autoplay.pending_wield {
            // A shield in the off hand counts as a full hand for a
            // weapon that cannot be held with one.
            let offhand_matters = self
                .stats_of(g)
                .is_some_and(|i| crate::weapons::needs_free_offhand(&i));
            let hands_full = self.world.wielded().any(|o| {
                (o.item_type
                    & (ac_world::item_type::MELEE_WEAPON
                        | ac_world::item_type::MISSILE_WEAPON
                        | ac_world::item_type::CASTER)
                    != 0
                    && o.valid_locations & ac_world::equip::MISSILE_AMMO == 0)
                    || (offhand_matters && o.valid_locations & ac_world::equip::SHIELD != 0)
            });
            if !hands_full {
                // Inside the wait a refusal earned it, or with the
                // server busy swinging, the errand keeps rather than
                // being dropped: giving up here would leave the weapon
                // in the pack and the character bare-handed with
                // nothing left to ask again.
                if self.world.is_carried(g) && self.wield_must_wait(g, now) {
                    return;
                }
                self.autoplay.pending_wield = None;
                if self.world.is_carried(g) {
                    self.wield_guid(g);
                }
            } else if !self.world.is_carried(g) && !self.world.objects.contains_key(&g) {
                self.autoplay.pending_wield = None;
            }
        }
    }

    /// Leave an item the server has just refused to wield alone for a
    /// while, and say so once rather than every pass.
    ///
    /// A refused wield carries no error code worth reading -- ACE sends
    /// InventoryServerSaveFailed with WeenieError.None -- so there is
    /// nothing to act on and nothing to do but wait. The wait doubles
    /// each time, so an item the server will never wield in this state
    /// costs a handful of messages rather than one every buff pass:
    /// +Verity asked for her Training Wand a hundred and fifty times in
    /// a minute, because a shield in her off hand made the wield
    /// impossible and the refusal said nothing about it.
    pub(crate) fn hold_off_wield(&mut self, item: u32, now: Instant) {
        self.autoplay.wield_asked = None;
        self.autoplay.wield_refused.hold(item, WIELD_AGAIN, now);
        let name = self.world.name_or_hex(item);
        let waited = self
            .autoplay
            .wield_refused
            .waited(&item)
            .unwrap_or(WIELD_AGAIN);
        tracing::info!(
            "the server will not wield {name}; leaving it for {} s",
            waited.as_secs().max(1)
        );
    }

    /// The stance the rules want this character in, and the weapon that
    /// gives it wielded if one is carried.
    ///
    /// Which way a character fights is not a setting: it follows what
    /// is in its hands, so [`Style::Auto`] simply reads them. The other
    /// three ask for a weapon of that kind to be wielded, and if none is
    /// carried the character fights with what it has and the rules say
    /// so rather than pretending.
    pub(crate) fn fighting_stance_as(&mut self, style: Style) -> Stance {
        let want = match style {
            Style::Auto => return self.combat_stance(),
            Style::Melee => Stance::Melee,
            Style::Missile => Stance::Missile,
            Style::Magic => Stance::Magic,
        };
        // Not with a swing out: changing weapon cancels it, so the
        // stance the hands give now is the honest answer until the
        // attack has been answered (see `change_of_hands_waits`).
        if change_of_hands_waits(self.combat_stance(), want, self.mid_attack()) {
            self.wait_for_the_swing();
        } else if self.combat_stance() != want {
            // Wielding takes a moment; until the server confirms it, the
            // hands still say what they said. Asked once a second, not
            // once a tick: the server answers each ask, and refuses the
            // ones it cannot yet do.
            let now = Instant::now();
            if self
                .autoplay
                .last_rewield
                .is_none_or(|t| now.duration_since(t) >= REWIELD_EVERY)
            {
                self.autoplay.last_rewield = Some(now);
                self.wield_for(want);
            }
        }
        self.combat_stance()
    }

    /// Wield the best weapon carried for `target`, if a better one than
    /// the one in hand is carried. Done once per target: swapping
    /// weapons mid-swing is worse than a slightly wrong weapon.
    ///
    /// Told to fight with whatever suits, this looks across all three
    /// kinds of weapon at once, so a character skilled with a wand and
    /// poor with a sword reaches for the wand. Changing weapon changes
    /// the stance, which the next tick reads back out of its hands.
    pub(crate) fn arm_for(&mut self, target: u32, stance: Stance, cfg: &Fight) {
        if !cfg.pick_weapon {
            return;
        }
        if self.autoplay.armed_for == Some(target) {
            return;
        }
        // Not with a swing or a charge out: putting the weapon in hand
        // away cancels it (see `Client::mid_attack`). The choice keeps,
        // and is made in the gap after the attack is answered -- a
        // fraction of a second with the wrong weapon beats a charge that
        // never lands.
        if self.mid_attack() {
            self.wait_for_the_swing();
            return;
        }
        self.autoplay.armed_for = Some(target);
        let Some(known) = self.creature_known(target) else {
            tracing::debug!("arm: nothing known about {target:#010x}, keeping what is held");
            return;
        };
        let carried: Vec<crate::items::ItemStats> = self.item_stats();
        // A weapon nobody has looked at has no element, no imbue and no
        // requirement to read, so it can neither be judged nor safely
        // reached for. Ask about the ones we are carrying; the answers
        // come back over the next few seconds and the choice improves
        // with them.
        let unknown: Vec<u32> = carried
            .iter()
            .filter(|i| !i.appraised && crate::weapons::stance_of(i).is_some())
            .map(|i| i.guid)
            .collect();
        if !unknown.is_empty() {
            self.appraise_many(unknown);
            // Come back to the choice once the answers are in.
            self.autoplay.armed_for = None;
        }
        let wielder = self.wielder();
        let free_choice = cfg.style == Style::Auto;
        let picked = if free_choice {
            crate::weapons::best_any(&carried, Some(known), &wielder).map(|(_, c)| c)
        } else {
            crate::weapons::best(&carried, stance, Some(known), &wielder)
        };
        // A bow's choice comes with the arrows to shoot from it.
        self.autoplay.wanted_ammo = picked
            .as_ref()
            .filter(|p| {
                carried
                    .iter()
                    .any(|i| i.guid == p.guid && crate::weapons::is_launcher(i))
            })
            .and_then(|_| crate::weapons::best_missile(&carried, Some(known), &wielder))
            .and_then(|(_, ammo)| ammo.map(|a| a.guid));
        let Some(pick) = picked else {
            tracing::debug!("arm: nothing to pick from for {}", known.name);
            return;
        };
        // What is in hand now, whichever kind it is when the choice is
        // free, so the comparison is between the two real options.
        let held = carried.iter().find(|i| {
            i.wielded
                && match crate::weapons::stance_of(i) {
                    Some(s) => free_choice || s == stance,
                    None => false,
                }
        });
        // Swapping costs nothing worth counting, so take the best there
        // is: anything better than what is in hand wins. Equal keeps
        // what is held, so a tie cannot set it swapping back and forth.
        let now_worth = held
            .map(|i| crate::weapons::score(i, Some(known), &wielder))
            .unwrap_or(0.0);
        tracing::debug!(
            "arm: {} vs {} -> best {} {:.3} (held {:.3})",
            known.name,
            held.map(|i| i.name.as_str()).unwrap_or("nothing"),
            pick.name,
            pick.score,
            now_worth
        );
        if held.map(|i| i.guid) == Some(pick.guid) || pick.score <= now_worth {
            return;
        }
        tracing::info!(
            "autoplay: wielding {} against {} ({})",
            pick.name,
            known.name,
            pick.why
        );
        let picked_stats = carried.iter().find(|i| i.guid == pick.guid);
        // A one-handed melee weapon leaves the off hand for a shield,
        // which is put on once the weapon is in hand (see
        // `autoplay_shield`); anything else wants that hand empty.
        let free_offhand = picked_stats.is_some_and(crate::weapons::needs_free_offhand);
        self.autoplay.wanted_shield = if free_offhand {
            None
        } else {
            crate::weapons::best_shield(&carried, &wielder).map(|s| s.guid)
        };
        // The server will not put a second weapon in full hands: the
        // old one goes back in the pack first, the shield too when the
        // new weapon cannot be held with one, and the new one is
        // wielded once the hands are empty.
        let mut sent = self.put_weapons_away();
        if free_offhand {
            let me = self.world.player_guid;
            let shield = self.wielded_shield();
            if let (Some(me), Some(shield)) = (me, shield) {
                sent |= self.put_in_container(shield, me);
            }
        }
        // When the swap started, so the fight waits for the hands to
        // settle rather than swinging into the moment they are empty.
        self.autoplay.last_rewield = Some(Instant::now());
        // A wield that could not go out this tick -- the item is inside
        // a refusal's wait, or the server has us mid-swing -- becomes an
        // errand rather than being forgotten, so the weapon is taken up
        // on the first free tick instead of being left in the pack.
        if sent || !self.wield_guid(pick.guid) {
            self.autoplay.pending_wield = Some(pick.guid);
        }
    }

    /// Whether a wand, orb or staff is carried at all, in hand or in
    /// the pack. Not the same question as whether one can be taken up
    /// right now, which is a wait rather than a want.
    pub(crate) fn carries_a_caster(&self) -> bool {
        self.world.objects.values().any(|o| {
            self.world.is_carried(o.guid) && o.item_type & ac_world::item_type::CASTER != 0
        })
    }

    /// Whether a weapon swap asked for a moment ago has yet to land.
    ///
    /// Between the put and the wield the hands are empty, and a swing
    /// sent into that gap is a punch: +Verity put her Flaming Takuba
    /// away for a wand and attacked a Spikey Armoredillo bare-handed in
    /// the same tick. Bounded by [`SWAP_SETTLES`] so a swap the server
    /// never finishes cannot stop the character fighting.
    pub(crate) fn hands_changing(&self, now: Instant) -> bool {
        self.autoplay.pending_wield.is_some()
            && self
                .autoplay
                .last_rewield
                .is_some_and(|t| now.duration_since(t) < SWAP_SETTLES)
    }

    /// The shield on the off hand, if any.
    pub(crate) fn wielded_shield(&self) -> Option<u32> {
        self.world
            .wielded()
            .find(|o| o.valid_locations & ac_world::equip::SHIELD != 0)
            .map(|o| o.guid)
    }

    /// Put the shield chosen with a one-handed weapon on, once that
    /// weapon is in hand and the off hand is free.
    pub(crate) fn autoplay_shield(&mut self, now: Instant) {
        let Some(shield) = self.autoplay.wanted_shield else {
            return;
        };
        if self.autoplay.pending_wield.is_some() {
            return;
        }
        if !self.world.is_carried(shield) || self.wielded_shield().is_some() {
            self.autoplay.wanted_shield = None;
            return;
        }
        // Not with a swing out. ACE shuffles the stance on every
        // successful equip, a shield included (TryShuffleStance ->
        // HandleActionChangeCombatMode), and a combat-mode change
        // cancels the attack in flight. The shield keeps; it goes on in
        // the gap after the swing is answered.
        if self.mid_attack() {
            self.wait_for_the_swing();
            return;
        }
        // Inside the wait a refusal earned it, or with the server busy
        // with a spell, the errand keeps: asking now sends nothing, and
        // clearing it would leave the shield in the pack with nothing
        // left to ask again.
        if self.wield_must_wait(shield, now) {
            return;
        }
        // Only with a one-handed melee weapon actually in hand: the
        // weapon may still be on its way, or have turned out to be
        // something a shield cannot go with.
        let held: Vec<crate::items::ItemStats> = self
            .world
            .wielded()
            .filter(|o| crate::weapons::stance_of(&crate::items::ItemStats::of(o, None)).is_some())
            .map(|o| {
                self.stats_of(o.guid)
                    .unwrap_or_else(|| crate::items::ItemStats::of(o, None))
            })
            .collect();
        let Some(weapon) = held.first() else {
            return;
        };
        if crate::weapons::needs_free_offhand(weapon) {
            self.autoplay.wanted_shield = None;
            return;
        }
        if self
            .autoplay
            .last_rewield
            .is_some_and(|t| now.duration_since(t) < REWIELD_EVERY)
        {
            return;
        }
        self.autoplay.last_rewield = Some(now);
        self.autoplay.wanted_shield = None;
        tracing::info!("autoplay: putting the shield on with the weapon");
        self.wield_guid(shield);
    }

    /// Take up again the weapon put down for an urgent buff, once no
    /// buff is due any more.
    pub(crate) fn autoplay_rearm(&mut self) {
        let Some(weapon) = self.autoplay.put_down else {
            return;
        };
        let never_below = self.autoplay.config.buffs.never_below;
        if self.due_buff(never_below, Instant::now()).is_some() {
            return;
        }
        if !self
            .world
            .objects
            .get(&weapon)
            .is_some_and(|o| o.container == self.world.player_guid)
        {
            // Sold, given away, or in hand already: no errand left.
            self.autoplay.put_down = None;
            return;
        }
        // Inside the wait a refusal earned it, or with the server busy,
        // the errand keeps, the way a pending wield's does: dropping it
        // here would leave the weapon in the pack and the character
        // fighting with the wand.
        if self.wield_must_wait(weapon, Instant::now()) {
            return;
        }
        self.autoplay.put_down = None;
        tracing::info!("autoplay: taking the weapon up again after buffing");
        // The wand is still in the hand, and ACE will not put a sword
        // in a hand that holds a caster -- CheckWeaponCollision refuses
        // it outright, with no error to read. So the wand goes back in
        // the pack and the housekeeping takes the weapon up once the
        // hands are empty, the same two steps the arming uses.
        if self.put_weapons_away() {
            self.autoplay.last_rewield = Some(Instant::now());
            self.autoplay.pending_wield = Some(weapon);
        } else {
            self.wield_guid(weapon);
        }
    }
}

#[cfg(test)]
mod tests;
