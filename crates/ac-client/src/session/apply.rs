use std::time::Instant;

use super::chat::chat_handles;
use crate::{
    answers_a_pour, answers_walk, heard_server_walk, player, Client, Event, PlayerFrame, ServerWalk,
};

/// The server's word for a request it refused because the character was
/// already doing something (WeenieError `YoureTooBusy`).
pub(crate) const YOURE_TOO_BUSY: u32 = 0x001D;

/// Whether a `UseDone` answer frees the cast slot: every answer does but a
/// busy refusal, which says the character is still in the middle of
/// something that will end with an answer of its own.
fn frees_the_cast_slot(err: u32) -> bool {
    err != YOURE_TOO_BUSY
}

impl Client {
    /// Pump the network, apply the server's messages, run the gameplay
    /// timers, and once the character is placed run its physics with
    /// `input` (None for a session nobody is steering). Returns what the
    /// renderer needs to know about the character this frame.
    pub fn tick(&mut self, input: Option<player::Input>, dt: f32, now: Instant) -> PlayerFrame {
        use ac_net::messages::{self, opcode, queue};
        // Read back what this character was carrying things for, once
        // the server has said which character it is.
        if !self.ledger_loaded && !self.world.stats.name.trim().is_empty() {
            self.ledger_loaded = true;
            self.load_ledger();
        }
        use ac_net::session::{Event, Port};
        let mut chat_pending: Vec<(u32, Vec<u8>)> = Vec::new();
        for (port, dg) in self.session.outgoing() {
            let to = if port == Port::Primary {
                self.primary
            } else {
                self.secondary
            };
            if let Some(socket) = &self.socket {
                let _ = socket.send_to(&dg, to);
            }
        }
        if let Some(socket) = &self.socket {
            let mut buf = [0u8; 2048];
            while let Ok((n, _)) = socket.recv_from(&mut buf) {
                self.session.receive(&buf[..n], now);
            }
        }
        self.session.poll(now);
        for ev in self.session.events() {
            match ev {
                Event::Connected { client_id } => {
                    tracing::info!("connected, client id {client_id}");
                    self.events.push(self::Event::Connected);
                }
                Event::Terminated(why) => {
                    tracing::warn!("terminated: {why}");
                    self.ended = Some(why.clone());
                    self.events.push(self::Event::Terminated(why));
                }
                Event::Message(msg) => {
                    // Every message the server sends, named, for when the
                    // question is "did it tell us at all?".  Off unless
                    // RUST_LOG asks for it: `wire=trace`.
                    if tracing::enabled!(target: "wire", tracing::Level::TRACE) {
                        if let Some((op, body)) = messages::split(&msg) {
                            let what = messages::opcode::name(op)
                                .map(|n| n.to_string())
                                .unwrap_or_else(|| format!("{op:#06x}"));
                            if op == opcode::GAME_EVENT {
                                let ev = body
                                    .get(4..8)
                                    .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                                    .unwrap_or(0);
                                let sub = messages::event::name(ev)
                                    .map(|n| n.to_string())
                                    .unwrap_or_else(|| format!("{ev:#06x}"));
                                tracing::trace!(target: "wire", "<- GameEvent/{sub} ({} bytes)", msg.len());
                            } else if op == opcode::SERVER_MESSAGE {
                                let text = ac_net::messages::ChatLine::parse_server_message(body)
                                    .map(|l| l.text)
                                    .unwrap_or_default();
                                tracing::trace!(target: "wire", "<- ServerMessage {text:?}");
                            } else {
                                tracing::trace!(target: "wire", "<- {what} ({} bytes)", msg.len());
                            }
                        }
                    }
                    self.forget_the_departed(&msg);
                    match self.world.apply(&msg) {
                        ac_world::Applied::PlayerSet => {
                            // The server ignores our positions until we say we landed.
                            self.session
                                .send_action(ac_net::messages::action::LOGIN_COMPLETE, &[]);
                            continue;
                        }
                        ac_world::Applied::Moved => continue,
                        ac_world::Applied::PlayerMoved => {
                            // A teleport or a server correction: stand where
                            // the server put us and forget the current route.
                            if let (Some(pl), Some(p)) = (
                                self.player.as_mut(),
                                self.world.player().and_then(|o| o.position),
                            ) {
                                pl.cell = p.cell;
                                pl.local = p.local;
                                pl.dirty = true;
                            }
                            self.steering.reset();
                            self.travel_displaced();
                        }
                        ac_world::Applied::PlayerMoveTo | ac_world::Applied::PlayerMotion => {
                            // A MoveTo aimed at us is ours to carry out; a plain
                            // motion state for us means the server is done
                            // walking us (or echoed our own state).
                            let stance = self.world.player().map(|o| o.motion.style);
                            let target = self.world.player_mut().and_then(|o| o.target.take());
                            match heard_server_walk(
                                &mut self.move_to,
                                &mut self.move_to_since,
                                &mut self.move_to_answered,
                                target,
                                Instant::now(),
                            ) {
                                ServerWalk::Began(t) => tracing::debug!("server move-to {t:?}"),
                                ServerWalk::Ended => {
                                    tracing::debug!("server move-to finished");
                                    // Take the server's idea of where we ended up.
                                    if let (Some(pl), Some(p)) = (
                                        self.player.as_mut(),
                                        self.world.player().and_then(|o| o.position),
                                    ) {
                                        pl.cell = p.cell;
                                        pl.local = p.local;
                                        pl.dirty = true;
                                    }
                                }
                                ServerWalk::Unchanged => {}
                            }
                            // Our stance follows the server (combat mode changes).
                            if let (Some(st), Some(pl)) = (stance, self.player.as_mut()) {
                                if st != 0 {
                                    pl.set_stance(&self.assets, 0x8000_0000 | st as u32);
                                }
                            }
                            continue;
                        }
                        ac_world::Applied::Appearance => {
                            // Our own look changed: redraw the character.
                            if let Some(pl) = self.player.as_mut() {
                                pl.dirty = true;
                            }
                            continue;
                        }
                        ac_world::Applied::Spellbook { spell, known } => {
                            tracing::info!(
                                "spell {spell} {}",
                                if known { "learned" } else { "forgotten" }
                            );
                            self.events.push(if known {
                                self::Event::SpellLearned(spell)
                            } else {
                                self::Event::SpellForgotten(spell)
                            });
                            continue;
                        }
                        ac_world::Applied::Enchantments => {
                            // The server sends an enchantment's start
                            // as an offset from now; give the record
                            // the clock so its countdown can run.
                            if let Some(now) = self.session.server_time() {
                                self.world.stats.anchor_enchantments(now);
                            }
                            continue;
                        }
                        ac_world::Applied::Effect { guid, script } => {
                            let speed = messages::split(&msg)
                                .and_then(|(_, b)| messages::parse_play_effect(b).ok())
                                .map(|(_, _, s)| s)
                                .unwrap_or(1.0);
                            self.events.push(self::Event::Effect {
                                guid,
                                script,
                                speed,
                            });
                            continue;
                        }
                        ac_world::Applied::Fellowship => {
                            // ACE sends fellows' vitals only while it
                            // believes our panel is open: say so once
                            // per fellowship.
                            match self.world.fellowship.is_some() {
                                true if !self.fellow_updates => {
                                    self.fellow_updates = true;
                                    self.session.send_action(
                                        ac_net::messages::action::FELLOWSHIP_UPDATE_REQUEST,
                                        &messages::fellowship_update_request(true),
                                    );
                                }
                                false => self.fellow_updates = false,
                                _ => {}
                            }
                            continue;
                        }
                        ac_world::Applied::Created
                        | ac_world::Applied::Deleted
                        | ac_world::Applied::Stats
                        | ac_world::Applied::Health
                        | ac_world::Applied::Vendor
                        | ac_world::Applied::Trade
                        | ac_world::Applied::Allegiance
                        | ac_world::Applied::House
                        | ac_world::Applied::Social
                        | ac_world::Applied::Confirmation
                        | ac_world::Applied::Inventory => continue,
                        ac_world::Applied::Failed => tracing::warn!("failed to apply a message"),
                        ac_world::Applied::Ignored => {}
                    }
                    if let Some((op, body)) = messages::split(&msg) {
                        chat_pending.push((op, body.to_vec()));
                    }
                    let Some((op, body)) = messages::split(&msg) else {
                        continue;
                    };
                    match op {
                        opcode::CHARACTER_LIST => {
                            if let Ok(cl) = messages::CharacterList::parse(body) {
                                tracing::info!(
                                    "characters: {:?}",
                                    cl.characters.iter().map(|c| &c.name).collect::<Vec<_>>()
                                );
                                self.characters = cl.characters;
                                self.characters_known = true;
                                self.lobby_ready();
                            }
                        }
                        opcode::DDD_END_DDD => {
                            self.ddd_done = true;
                            self.lobby_ready();
                        }
                        opcode::CHARACTER_CREATE_RESPONSE => {
                            match messages::CharacterCreateResponse::parse(body) {
                                Ok(r) => self.create_response(r),
                                Err(e) => tracing::warn!("CharacterCreateResponse: {e}"),
                            }
                        }
                        opcode::CHARACTER_DELETE => {
                            // Acknowledged; the refreshed list follows.
                            tracing::info!("character deletion acknowledged");
                        }
                        _ => {}
                    }
                    let Some((op, _)) = messages::split(&msg) else {
                        continue;
                    };
                    match op {
                        opcode::CHARACTER_ENTER_WORLD_SERVER_READY => {
                            let id = self
                                .entering
                                .or_else(|| self.pick_character().map(|c| c.id));
                            if let Some(id) = id {
                                let account = self.config.account.clone();
                                self.session
                                    .send_message(queue::UI, messages::enter_world(id, &account));
                            } else {
                                tracing::warn!("server ready but no character to enter with");
                            }
                        }
                        opcode::PLAYER_TELEPORT => {
                            // A walk the server was making for us (to the portal
                            // just used) ends with the teleport. Left standing,
                            // the client kept out of the server's way for the
                            // rest of its twelve seconds, and the first walk in
                            // the new place -- into the dungeon's next room --
                            // went nowhere and was written off.
                            self.move_to = None;
                            // After a server teleport, take the new position and
                            // tell the server we landed.
                            if let (Some(pl), Some(p)) = (
                                self.player.as_mut(),
                                self.world.player().and_then(|o| o.position),
                            ) {
                                pl.cell = p.cell;
                                pl.local = p.local;
                                pl.dirty = true;
                            }
                            self.session
                                .send_action(ac_net::messages::action::LOGIN_COMPLETE, &[]);
                        }
                        opcode::ACCOUNT_BANNED => {
                            let (secs, reason) =
                                messages::parse_account_banned(body).unwrap_or((0, String::new()));
                            tracing::error!("account banned for {secs} s: {reason}");
                            self.last_refusal = Some((op, 0));
                            self.events.push(self::Event::Refused(op));
                        }
                        opcode::CHARACTER_LOG_OFF => {
                            tracing::info!("server logged the character off");
                            self.logged_off = true;
                        }
                        opcode::CHARACTER_ERROR | opcode::ACCOUNT_BOOT => {
                            let code = body
                                .get(..4)
                                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                                .unwrap_or(0);
                            tracing::error!("server refused: {op:#06x} (code {code:#x})");
                            self.last_refusal = Some((op, code));
                            // A refused enter (not owned, still in world,
                            // pending deletion) may be retried with another
                            // character.
                            if op == opcode::CHARACTER_ERROR && self.scene_block.is_none() {
                                self.enter_requested = false;
                                self.entering = None;
                            }
                            self.events.push(self::Event::Refused(op));
                        }
                        opcode::GAME_EVENT => {
                            if let Some((_, _, ev, rest)) = messages::split_game_event(body) {
                                // Words about a request, stamped with this
                                // tick (see `told`).
                                if matches!(
                                    ev,
                                    messages::event::TRANSIENT_STRING
                                        | messages::event::WEENIE_ERROR
                                        | messages::event::WEENIE_ERROR_WITH_STRING
                                ) {
                                    self.told = Some(now);
                                }
                                if ev == 0x00A0 && rest.len() >= 8 {
                                    let item =
                                        u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]);
                                    let err =
                                        u32::from_le_bytes([rest[4], rest[5], rest[6], rest[7]]);
                                    tracing::warn!(
                                        "inventory action failed for {item:#010x}, error {err:#x}"
                                    );
                                    // This is the server's whole answer
                                    // to a move, a split or a merge it
                                    // will not make, and half the time
                                    // the error is None -- no words, no
                                    // code, nothing. Until this was
                                    // kept, a step could not tell a
                                    // refusal from an answer still on
                                    // its way, so it asked again every
                                    // few hundred milliseconds for as
                                    // long as the session lasted.
                                    // A quest's refusal of a drop names no
                                    // item; it is about the take in flight
                                    // (see `autoplay::refused_item`).
                                    let inflight = self.loot_inflight.map(|(g, _)| g);
                                    let refused =
                                        crate::autoplay::refused_item(item, err, inflight);
                                    // What the refusal is about: the item,
                                    // and whatever holds it (a take from a
                                    // corpse walks to the corpse).
                                    let about: Vec<u32> = [Some(item), refused]
                                        .into_iter()
                                        .flatten()
                                        .flat_map(|g| {
                                            [
                                                Some(g),
                                                self.world
                                                    .objects
                                                    .get(&g)
                                                    .and_then(|o| o.container),
                                            ]
                                        })
                                        .flatten()
                                        .collect();
                                    if let Some(item) = refused {
                                        self.move_refused.insert(item, (err, Instant::now()));
                                        if inflight == Some(item) {
                                            self.loot_inflight = None;
                                        }
                                        self.loot_refused(item, err);
                                        // A take turned down for want of
                                        // room in the pack it named (see
                                        // `Client::take_refused`).
                                        self.take_refused(item, err);
                                    }
                                    self.wield_refused(item, now);
                                    let for_a_pour =
                                        answers_a_pour(self.autoplay.pour.as_ref(), item, now);
                                    // A pickup the server walked us to
                                    // for, refused: an answer like a
                                    // UseDone (see `server_walk_over`).
                                    // Only that pickup's, though: a wield
                                    // refused mid-charge is no answer to
                                    // the charge.
                                    if !for_a_pour
                                        && answers_walk(
                                            self.move_to,
                                            &about,
                                            self.last_used,
                                            self.attacked,
                                        )
                                    {
                                        self.move_to_answered = true;
                                    }
                                } else if ev == ac_net::messages::event::USE_DONE && rest.len() >= 4
                                {
                                    let err =
                                        u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]);
                                    tracing::debug!("use done, error {err:#x}");
                                    self.use_done = Some((err, now));
                                    // The server has finished with what
                                    // it was asked to do -- a cast, a
                                    // use, a counter opening. That is
                                    // the signal to send the next one:
                                    // a heal cast the moment the last
                                    // one lands is the difference
                                    // between living and dying, and a
                                    // clock cannot be both that quick
                                    // and slow enough to never have a
                                    // spell dropped for arriving early.
                                    // Not a busy refusal, though. The server
                                    // turns a cast away as too busy while the
                                    // one before is still going up, and says
                                    // so at once; taking that for the earlier
                                    // cast finishing had a caster throw the
                                    // same spell every 150 ms while a buff was
                                    // still being cast. Whatever made the
                                    // character busy -- a cast, a kit, a use --
                                    // ends with an answer of its own, and that
                                    // is what frees the slot.
                                    if frees_the_cast_slot(err) {
                                        self.autoplay.cast_sent = None;
                                    }
                                    // A use by hand that has been answered
                                    // is not carried on when the walk for
                                    // it runs out (see `visit`).
                                    self.visits.answered();
                                    // Only a refusal ends the walk, and
                                    // only a refusal of what the walk is
                                    // for: a cast turned away as too busy
                                    // mid-charge is no reason to take the
                                    // controls back (see `answers_walk`).
                                    //
                                    // The server answers a use of
                                    // something out of reach by telling
                                    // the client to walk there, and
                                    // acknowledges the request in the
                                    // same breath. Taking that
                                    // acknowledgement for "the walk is
                                    // over" threw the goal away after a
                                    // single tick: the character took
                                    // one stride per attempt and stood
                                    // still between them, four metres
                                    // from a merchant, saying it was
                                    // walking to it. Arriving is what
                                    // ends the walk.
                                    let for_the_walk = answers_walk(
                                        self.move_to,
                                        self.last_used.as_slice(),
                                        self.last_used,
                                        self.attacked,
                                    );
                                    if for_the_walk && err != 0 {
                                        self.move_to = None;
                                    }
                                    // Once there, though, the answer is
                                    // the last word on the walk (see
                                    // `server_walk_over`).
                                    if for_the_walk && self.move_to.is_some() {
                                        self.move_to_answered = true;
                                    }
                                } else if ev == ac_net::messages::event::QUERY_AGE_RESPONSE {
                                    self.hear_age(rest);
                                } else if ev == ac_net::messages::event::AVAILABLE_HOUSES {
                                    self.hear_houses(rest);
                                } else if ev == ac_net::messages::event::SET_TURBINE_CHAT_CHANNELS
                                    && rest.len() >= 4
                                {
                                    // Ten room ids: allegiance, the five
                                    // every player has, then our society's
                                    // and the three fixed society rooms
                                    // (ACE GameEventSetTurbineChatChannels.cs:10-19).
                                    self.allegiance_room =
                                        u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]);
                                    if let Some(id) = rest.get(24..28) {
                                        self.society_room =
                                            u32::from_le_bytes(id.try_into().expect("four bytes"));
                                    }
                                } else if !chat_handles(op, ev) {
                                    // Everything neither World::apply
                                    // nor chat_message takes.
                                    tracing::debug!(
                                        "game event {ev:#06x} {} ({} bytes)",
                                        ac_net::messages::event::name(ev).unwrap_or("unknown"),
                                        rest.len()
                                    );
                                }
                            }
                        }
                        // Taken above (the lobby) or by the session
                        // itself (the DAT interrogation).
                        opcode::CHARACTER_LIST
                        | opcode::DDD_END_DDD
                        | opcode::DDD_INTERROGATION
                        | opcode::DDD_BEGIN_DDD
                        | opcode::SERVER_NAME
                        | opcode::CHARACTER_CREATE_RESPONSE
                        | opcode::CHARACTER_DELETE => {}
                        _ if chat_handles(op, 0) => {}
                        _ => tracing::debug!(
                            "message {op:#06x} {} ({} bytes)",
                            opcode::name(op).unwrap_or("unknown"),
                            body.len()
                        ),
                    }
                }
            }
        }
        for (op, body) in chat_pending {
            self.chat_message(op, &body);
        }
        self.tick_combat();
        self.tick_loot(now);
        self.tick_store();
        self.tick_appraise();
        self.tick_autoplay(now);
        // The rules may have changed under the decisions already made.
        // Before the ledger is written out, not after.
        self.tick_retag(now);
        // What each item was taken for, written out when it changes.
        // Not on the way out: the thing this is defending against is a
        // client that does not get a way out.
        self.save_ledger();
        // Build the static scene once the player is placed.
        if self.scene_block.is_none() {
            if let Some(p) = self.world.player().and_then(|o| o.position) {
                let block = p.landblock();
                tracing::info!(
                    "player at cell {:#010x} local {:?}; loading landblocks",
                    p.cell,
                    p.local
                );
                self.player_setup = self
                    .world
                    .player()
                    .map(|o| o.setup_id)
                    .unwrap_or(0x0200_0001);
                let mut pl = player::Player::new(&self.assets, p.cell, p.local, p.rotation);
                let table_id = self
                    .world
                    .player()
                    .map(|o| o.motion_table_id)
                    .filter(|&t| t != 0)
                    .unwrap_or(0x0900_0001);
                pl.set_motion_table(&self.assets, self.player_setup, table_id);
                self.player = Some(pl);
                self.events.push(self::Event::Placed { cell: p.cell });
                self.scene_block = Some(block);
                if self.world.allegiance.is_none() {
                    self.allegiance_update_request(false);
                }
                if self.world.house.is_none() {
                    self.house_query();
                }
            }
        }
        self.world.tick(dt);
        self.tick_player(input.unwrap_or_default(), dt, now)
    }
}

#[cfg(test)]
mod tests;
