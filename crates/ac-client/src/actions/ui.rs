use std::time::Instant;

use crate::{emotes, Client};

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

    /// A chat line starting with `/`: the retail client's own commands
    /// become their game actions; anything else goes to the server as an
    /// `@command` (ACE runs its command manager on those and answers
    /// "Unknown command" for the rest). Returns false for an empty line.
    pub fn slash_command(&mut self, line: &str) -> bool {
        use ac_net::messages::action;
        let body = line.trim_start_matches(['/', '@']).trim();
        if body.is_empty() {
            return false;
        }
        let (name, args) = body
            .split_once(char::is_whitespace)
            .map(|(n, a)| (n, a.trim()))
            .unwrap_or((body, ""));
        let name = name.to_ascii_lowercase();
        // A logout the player asked for is a session ending on purpose,
        // however the server ends up ending it: never reconnected (see
        // [`reconnect`]). The command itself is passed on as a server
        // command like any other.
        if matches!(name.as_str(), "logout" | "logoff" | "quit" | "exit") {
            self.quitting = true;
        }
        let mut w = ac_net::wire::Writer::new();
        match name.as_str() {
            "lifestone" | "ls" => self.session.send_action(action::TELE_TO_LIFESTONE, &[]),
            "die" => self.session.send_action(action::DIE, &[]),
            "house" | "home" => self.session.send_action(action::TELE_TO_HOUSE, &[]),
            "mansion" | "mp" => self.session.send_action(action::TELE_TO_MANSION, &[]),
            "hometown" => self
                .session
                .send_action(action::RECALL_ALLEGIANCE_HOMETOWN, &[]),
            "marketplace" | "mkt" => self.session.send_action(action::TELE_TO_MARKETPLACE, &[]),
            "pklite" => self.session.send_action(action::ENTER_PK_LITE, &[]),
            "afk" => {
                if !args.is_empty() {
                    w.string16(args);
                    self.session
                        .send_action(action::SET_AFK_MESSAGE, &w.finish());
                    w = ac_net::wire::Writer::new();
                }
                w.u32(1);
                self.session.send_action(action::SET_AFK_MODE, &w.finish());
            }
            "back" => {
                w.u32(0);
                self.session.send_action(action::SET_AFK_MODE, &w.finish());
            }
            "tell" | "t" => {
                // `/tell Name, message` or `/tell Name message`.
                let (target, msg) = match args.split_once(',') {
                    Some((t, m)) => (t.trim(), m.trim()),
                    None => args
                        .split_once(char::is_whitespace)
                        .map(|(t, m)| (t.trim(), m.trim()))
                        .unwrap_or((args, "")),
                };
                if target.is_empty() || msg.is_empty() {
                    return false;
                }
                w.string16(msg).string16(target);
                self.session.send_action(action::TELL, &w.finish());
            }
            "emote" | "e" | "me" => {
                w.string16(args);
                self.session.send_action(action::EMOTE, &w.finish());
            }
            // `/wave`, `/bow`...: the soul emotes (retail typed them as
            // `*wave*`; both work).
            n if emotes::lookup(n).is_some() && args.is_empty() => {
                self.emote(n);
            }
            // `/g`, `/trade`, `/lfg`, `/rp`, `/a`: the Turbine chat rooms.
            n if ac_net::messages::turbine::from_prefix(n).is_some() => {
                if args.is_empty() {
                    return false;
                }
                let room = ac_net::messages::turbine::from_prefix(n).unwrap_or(0);
                return self.turbine_say(room, args);
            }
            // `/v`, `/p`, `/m`, `/c`, `/f`: vassals, patron, monarch,
            // co-vassals and fellowship group chat.
            n if ac_net::messages::channel::from_prefix(n).is_some() => {
                if args.is_empty() {
                    return false;
                }
                let channel = ac_net::messages::channel::from_prefix(n).unwrap_or(0);
                self.chat_channel(channel, args);
            }
            _ => {
                // The server's own commands (@acehelp, @myquests, admin
                // commands...): ACE reads them from Talk with an @ prefix.
                w.string16(&format!("@{body}"));
                self.session.send_action(action::TALK, &w.finish());
            }
        }
        true
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
