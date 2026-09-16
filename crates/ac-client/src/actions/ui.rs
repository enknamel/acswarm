use std::time::Instant;

use crate::Client;

impl Client {
    pub fn interact(&mut self, guid: u32) {
        use ac_net::messages::action;
        use ac_world::{item_type, object_desc_flags};
        let me = self.world.player_guid;
        // Using something in the world walks us to it: stop any journey
        // so it does not walk us away again afterwards.
        let in_the_world = self
            .world
            .objects
            .get(&guid)
            .is_some_and(|o| o.container.is_none() && o.wielder.is_none());
        if in_the_world {
            self.interrupt_travel("using something");
        }
        let Some(o) = self.world.objects.get(&guid) else {
            return;
        };
        let stuck = o.object_desc_flags
            & (object_desc_flags::STUCK
                | object_desc_flags::PLAYER
                | object_desc_flags::DOOR
                | object_desc_flags::VENDOR
                | object_desc_flags::PORTAL
                | object_desc_flags::CORPSE)
            != 0
            || o.item_type & item_type::CREATURE != 0;
        let carried = me.is_some() && (o.container == me || o.wielder == me);
        let name = o.name.clone();
        let attackable = o.object_desc_flags & object_desc_flags::ATTACKABLE != 0
            && o.object_desc_flags & object_desc_flags::PLAYER == 0
            && o.item_type & item_type::CREATURE != 0;
        let mut w = ac_net::wire::Writer::new();
        if self.combat && attackable {
            self.attack(guid);
            return;
        }
        if o.object_desc_flags & object_desc_flags::PLAYER != 0 && Some(guid) != me {
            // Using another player opens a secure trade (the retail client
            // sent this itself; the server ignores Use on players).
            self.open_trade(guid);
            return;
        }
        if carried && o.spell_id != 0 {
            if let Some(spell) = name.strip_prefix("Scroll of ") {
                self.known_spells.insert(o.spell_id, spell.to_string());
            }
        }
        // A kit, a mana stone or a key used with something selected is
        // applied to it (retail: select the target, then use the item).
        // With nothing selected an item that may be used on its owner (a
        // kit, a stone) goes on ourselves; the server has no plain Use
        // for those.
        if carried && ac_world::usable::needs_target(o.usable) {
            let target = self
                .use_target(guid)
                .or(if ac_world::usable::on_self(o.usable) {
                    me
                } else {
                    None
                });
            if let Some(target) = target {
                self.use_on(guid, target);
                return;
            }
        }
        if carried && o.weenie_class_id == ac_world::material::UST_WCID {
            // The Ust is the salvaging tool: using it opens the salvage
            // window (the retail client did this itself; the server only
            // hears the final list).
            self.salvage_open = !self.salvage_open;
            return;
        }
        if carried && o.wielder != me && o.item_type & item_type::WIELDABLE != 0 {
            tracing::info!("wield {name} ({guid:#010x}) at {:#x}", o.valid_locations);
            w.u32(guid).u32(o.valid_locations);
            self.session
                .send_action(action::GET_AND_WIELD_ITEM, &w.finish());
        } else if carried && o.wielder == me {
            tracing::info!("take off {name} ({guid:#010x})");
            w.u32(guid).u32(me.unwrap_or(0)).u32(0);
            self.session
                .send_action(action::PUT_ITEM_IN_CONTAINER, &w.finish());
        } else if !carried && !stuck && o.position.is_some() {
            // Double-click on a loose item picks it up, as retail did; the
            // "use selected" key (`use_object`) reads or activates it in
            // place instead.
            tracing::info!("pick up {name} ({guid:#010x})");
            self.last_used = Some(guid);
            w.u32(guid).u32(me.unwrap_or(0)).u32(0);
            self.session
                .send_action(action::PUT_ITEM_IN_CONTAINER, &w.finish());
        } else {
            tracing::info!("use {name} ({guid:#010x})");
            // Stopped first, on the wire: a stop reported after the use
            // cancels the walk the server starts for it, and the use with
            // it (ACE `GameActionMoveToState`).
            if in_the_world {
                if let Some(pl) = self.player.as_mut() {
                    pl.report_stopped(&mut self.session, self.held_run);
                    if let Some((cell, local)) = pl.take_sent() {
                        self.world.player_reported(cell, local);
                    }
                }
                // What a server walk that runs out was walking to.
                self.visits.used(guid, Instant::now());
            }
            self.last_used = Some(guid);
            self.session.send_action(action::USE, &guid.to_le_bytes());
        }
    }

    /// Select an object (what the target bar and appraisal refer to).
    pub fn select(&mut self, guid: Option<u32>) {
        if guid != self.selected {
            self.previous_selected = self.selected;
        }
        self.selected = guid;
    }

    /// The thing a targetable item is applied to when used: the most
    /// recently selected object other than the item itself (selecting
    /// the item to use it must not lose the target picked before).
    pub fn use_target(&self, item: u32) -> Option<u32> {
        [self.selected, self.previous_selected]
            .into_iter()
            .flatten()
            .find(|t| *t != item && self.world.objects.contains_key(t))
    }
}
