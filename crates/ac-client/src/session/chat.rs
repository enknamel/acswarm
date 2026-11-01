use std::time::{Duration, Instant};

use crate::{
    attack_ended_walk, autoplay, salvage_text, weenie_errors, Client, Event, YOURE_TOO_BUSY,
};

impl Client {
    pub fn play_sound(&mut self, body: &[u8]) {
        use ac_net::messages::parse_sound;
        let Ok((guid, kind, volume)) = parse_sound(body) else {
            return;
        };
        let (name, table_id) = match self.world.objects.get(&guid) {
            Some(o) => (
                o.name.clone(),
                if o.sound_table_id != 0 {
                    o.sound_table_id
                } else if o.is_player
                    || o.object_desc_flags & ac_world::object_desc_flags::PLAYER != 0
                {
                    0x2000_0001
                } else {
                    0
                },
            ),
            None => return,
        };
        if table_id == 0 {
            return;
        }
        let assets = &self.assets;
        let table = self
            .sound_tables
            .entry(table_id)
            .or_insert_with(|| {
                assets
                    .portal
                    .read(table_id)
                    .ok()
                    .and_then(|b| ac_formats::sound_table::SoundTable::parse(table_id, &b).ok())
                    .map(std::rc::Rc::new)
            })
            .clone();
        let Some(table) = table else { return };
        let Some(wave_id) = ac_formats::sound_table::sound_for(&table, kind) else {
            return;
        };
        let wave = self
            .waves
            .entry(wave_id)
            .or_insert_with(|| {
                assets
                    .portal
                    .read(wave_id)
                    .ok()
                    .and_then(|b| ac_formats::wave::Wave::parse(wave_id, &b).ok())
                    .map(std::rc::Rc::new)
            })
            .clone();
        let Some(wave) = wave else { return };
        tracing::debug!("sound {kind:#x} from {name}: wave {wave_id:#010x} vol {volume:.2}");
        self.events.push(Event::Sound {
            wave,
            volume: volume.clamp(0.0, 1.0),
        });
    }

    /// Remember whom to `/reply` to. Retail took the sender from a tell
    /// only when it was a player's (guid 0x50000001..=0x6FFFFFFF), so an
    /// NPC's does not become the reply target (FUN_00572370:147-151); it
    /// also checked the tell was addressed to us, which ACE guarantees by
    /// sending GameEventTell to the target alone (GameActionTalkDirect.cs:46).
    pub(crate) fn hear_tell(&mut self, sender_id: u32, sender: &str) {
        if (0x5000_0001..=0x6FFF_FFFF).contains(&sender_id) && !sender.is_empty() {
            self.last_teller = Some((sender_id, sender.to_string()));
        }
    }

    pub fn chat_message(&mut self, op: u32, body: &[u8]) {
        use ac_net::messages::{event, opcode, ChatLine};
        if op == opcode::SOUND {
            self.play_sound(body);
            return;
        }
        let line = match op {
            opcode::TURBINE_CHAT => match ac_net::messages::turbine::parse(body) {
                Ok(Some(l)) => Ok(l),
                Ok(None) => return,
                Err(e) => Err(e),
            },
            opcode::HEAR_SPEECH => ChatLine::parse_hear_speech(body),
            opcode::HEAR_RANGED_SPEECH => ChatLine::parse_hear_ranged_speech(body),
            opcode::SERVER_MESSAGE => ChatLine::parse_server_message(body),
            opcode::EMOTE_TEXT | opcode::SOUL_EMOTE => ChatLine::parse_emote_text(body),
            // "X was killed by Y" for players in view; the world has
            // already zeroed the victim's health.
            opcode::PLAYER_KILLED => match ac_net::messages::parse_player_killed(body) {
                Ok((text, _, _)) => Ok(ChatLine {
                    text,
                    sender: String::new(),
                    sender_id: 0,
                    kind: 0x1F,
                }),
                Err(e) => Err(e),
            },
            opcode::GAME_EVENT => match ac_net::messages::split_game_event(body) {
                Some((_, _, event::TELL, rest)) => {
                    let line = ChatLine::parse_tell(rest);
                    if let Ok(l) = &line {
                        self.hear_tell(l.sender_id, &l.sender);
                    }
                    line
                }
                Some((_, _, event::CHANNEL_BROADCAST, rest)) => {
                    ChatLine::parse_channel_broadcast(rest)
                }
                Some((_, _, event::BOOK_DATA_RESPONSE, rest)) => {
                    match ac_net::messages::BookData::parse(rest) {
                        Ok(b) => {
                            tracing::info!(
                                "book {:#010x} \"{}\": {} pages",
                                b.guid,
                                b.inscription,
                                b.pages.len()
                            );
                            let first_missing = b.pages.first().is_some_and(|p| p.text.is_none());
                            let guid = b.guid;
                            self.book = Some(b);
                            self.book_seq += 1;
                            if first_missing {
                                self.read_page(guid, 0);
                            }
                            return;
                        }
                        Err(e) => {
                            tracing::warn!("book data: {e}");
                            return;
                        }
                    }
                }
                Some((_, _, event::BOOK_PAGE_DATA_RESPONSE, rest)) => {
                    if let Ok((guid, index, page)) = ac_net::messages::BookData::parse_page(rest) {
                        if let Some(b) = self.book.as_mut().filter(|b| b.guid == guid) {
                            if let Some(slot) = b.pages.get_mut(index as usize) {
                                *slot = page;
                            } else if index as usize == b.pages.len() {
                                b.pages.push(page);
                            }
                            self.book_seq += 1;
                        }
                    }
                    return;
                }
                Some((_, _, event::SALVAGE_OPERATIONS_RESULT, rest)) => {
                    match ac_net::messages::SalvageResult::parse(rest) {
                        Ok(res) => Ok(ChatLine {
                            text: salvage_text(&res),
                            sender: String::new(),
                            sender_id: 0,
                            kind: 0,
                        }),
                        Err(e) => Err(e),
                    }
                }
                Some((_, _, event::IDENTIFY_OBJECT_RESPONSE, rest)) => {
                    self.appraisal(rest);
                    return;
                }
                Some((_, _, event::ATTACK_DONE, rest)) => {
                    let err = rest
                        .get(..4)
                        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                        .unwrap_or(0);
                    {
                        self.attack_pending = false;
                        // ACE always reports ActionCancelled (0x36) here: it
                        // just means the swing sequence ended.
                        tracing::debug!("attack done ({err:#x})");
                        self.attack_backoff = Duration::from_millis(300);
                        if attack_ended_walk(self.move_to, self.attacked) {
                            tracing::debug!("server move-to over: the attack it was for is done");
                            self.move_to = None;
                        }
                    }
                    return;
                }
                Some((_, _, event::ATTACKER_NOTIFICATION, rest)) => {
                    // Our killing blow: its body is owed from now, before
                    // the corpse appears, and wherever it fell (see
                    // `Client::owes_a_corpse`).
                    if let Ok(n) = ac_net::messages::AttackNotice::parse_attacker(rest) {
                        self.blows.dealt += 1;
                        self.blows.dealt_points += u64::from(n.damage);
                        if n.percent >= 0.999 {
                            let now = Instant::now();
                            self.autoplay.last_kill = Some(now);
                            if let Some(at) = self.killed_at(&n.name) {
                                self.autoplay.kill_spots.push((at, now));
                            }
                        }
                    }
                    match ac_net::messages::AttackNotice::parse_attacker(rest) {
                        Ok(n) => Ok(ChatLine {
                            text: format!(
                                "You {} {} for {} points{}.",
                                if n.critical { "critically hit" } else { "hit" },
                                n.name,
                                n.damage,
                                if n.percent >= 0.999 {
                                    ", killing it"
                                } else {
                                    ""
                                }
                            ),
                            sender: String::new(),
                            sender_id: 0,
                            kind: 5,
                        }),
                        Err(e) => Err(e),
                    }
                }
                Some((_, _, event::DEFENDER_NOTIFICATION, rest)) => {
                    // Something hit us: that fight comes before any loot.
                    self.autoplay.last_hit_us = Some(Instant::now());
                    match ac_net::messages::AttackNotice::parse_defender(rest) {
                        Ok(n) => Ok({
                            self.autoplay.attacked_by(&n.name, Instant::now());
                            self.blows.taken += 1;
                            self.blows.taken_points += u64::from(n.damage);
                            ChatLine {
                                text: format!(
                                    "{} {} you for {} points.",
                                    n.name,
                                    if n.critical {
                                        "critically hits"
                                    } else {
                                        "hits"
                                    },
                                    n.damage
                                ),
                                sender: String::new(),
                                sender_id: 0,
                                kind: 6,
                            }
                        }),
                        Err(e) => Err(e),
                    }
                }
                Some((_, _, event::EVASION_ATTACKER_NOTIFICATION, rest)) => {
                    self.blows.missed += 1;
                    match ac_net::wire::Reader::new(rest).string16() {
                        Ok(n) => Ok(ChatLine {
                            text: format!("{n} evades your attack."),
                            sender: String::new(),
                            sender_id: 0,
                            kind: 5,
                        }),
                        Err(e) => Err(e),
                    }
                }
                Some((_, _, event::EVASION_DEFENDER_NOTIFICATION, rest)) => {
                    // A swing that missed is still a swing: the creature
                    // has picked this fight as surely as one that landed
                    // a blow, and every "but fight back when it attacks
                    // you" carve-out reads these two fields. Counting
                    // only hits let a critter stand there missing while
                    // the character walked past it.
                    self.autoplay.last_hit_us = Some(Instant::now());
                    self.blows.evaded += 1;
                    match ac_net::wire::Reader::new(rest).string16() {
                        Ok(n) => Ok({
                            self.autoplay.attacked_by(&n, Instant::now());
                            ChatLine {
                                text: format!("You evade {n}'s attack."),
                                sender: String::new(),
                                sender_id: 0,
                                kind: 6,
                            }
                        }),
                        Err(e) => Err(e),
                    }
                }
                Some((
                    _,
                    _,
                    kind @ (event::VICTIM_NOTIFICATION | event::KILLER_NOTIFICATION),
                    rest,
                )) => match ac_net::wire::Reader::new(rest).string16() {
                    Ok(t) => {
                        // Our kill, however it was made: a spell's killing
                        // blow comes with no attacker notice, only this, and
                        // its body is owed all the same.
                        if kind == event::KILLER_NOTIFICATION {
                            let now = Instant::now();
                            self.autoplay.last_kill = Some(now);
                            if let Some(at) = self.killed_in(&t) {
                                self.autoplay.kill_spots.push((at, now));
                            }
                        }
                        Ok(ChatLine {
                            text: t,
                            sender: String::new(),
                            sender_id: 0,
                            kind: 0,
                        })
                    }
                    Err(e) => Err(e),
                },
                Some((_, _, event::TRANSIENT_STRING, rest)) => {
                    match ac_net::wire::Reader::new(rest).string16() {
                        Ok(t) => Ok(ChatLine {
                            text: t,
                            sender: String::new(),
                            sender_id: 0,
                            kind: 7,
                        }),
                        Err(e) => Err(e),
                    }
                }
                Some((_, _, event::WEENIE_ERROR, rest)) => {
                    let code = rest
                        .get(..4)
                        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                        .unwrap_or(0);
                    tracing::info!("weenie error {code:#x}");
                    self.recall_error(code);
                    // A counter turns a Use away as too busy while a
                    // cast of ours is in the air: ours to wait out,
                    // not the counter refusing (see `growth::on_opening`).
                    // Food, gems, scrolls and a swing are turned away
                    // with the same word, and one of those heard while
                    // the counter is being asked is taken for the
                    // Use's refusal too: the cost is one re-ask three
                    // seconds on, and the asks are capped (`BUSY_ASKS`).
                    if code == YOURE_TOO_BUSY {
                        self.autoplay.growth.counter_said_busy(Instant::now());
                    }
                    // A full fellowship is refused by code, and names
                    // nobody: it is about whoever was invited last.
                    if code == autoplay::FELLOWSHIP_FULL {
                        self.autoplay.hear_fellowship_full(Instant::now());
                    }
                    // The informational ones (teleported, turbine chat) stay
                    // in the log; refusals reach the chat.
                    match weenie_errors::text(code) {
                        Some(t) if !matches!(code, 0x3c | 0x51d) => Ok(ChatLine {
                            text: t.to_string(),
                            sender: String::new(),
                            sender_id: 0,
                            kind: 7,
                        }),
                        _ => return,
                    }
                }
                Some((_, _, event::WEENIE_ERROR_WITH_STRING, rest)) => {
                    let mut r = ac_net::wire::Reader::new(rest);
                    match (r.u32(), r.string16()) {
                        (Ok(code), Ok(param)) => {
                            tracing::info!("weenie error {code:#x} ({param})");
                            Ok(ChatLine {
                                text: weenie_errors::text_with(code, &param)
                                    .unwrap_or_else(|| format!("{param}: error {code:#x}")),
                                sender: String::new(),
                                sender_id: 0,
                                kind: 7,
                            })
                        }
                        _ => return,
                    }
                }
                Some((_, _, event::POPUP_STRING, rest)) => {
                    match ac_net::wire::Reader::new(rest).string16() {
                        Ok(t) => Ok(ChatLine {
                            text: t,
                            sender: String::new(),
                            sender_id: 0,
                            kind: 0,
                        }),
                        Err(e) => Err(e),
                    }
                }
                _ => return,
            },
            _ => return,
        };
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!("chat message {op:#06x}: {e}");
                return;
            }
        };
        // The academy rule listens to what the agents say.
        self.autoplay
            .academy
            .hear(&line.sender, &line.text, Instant::now());
        // A resist or an evasion: the shot got there.
        self.hear_arrival(&line.text);
        // The server's own word on a death (never a player's): nothing
        // dropped, nothing to go back for.
        if line.sender.is_empty() {
            self.autoplay.recovery.heard(&line.text, Instant::now());
            // And on anything it would not do -- open a body, put a
            // take away, recruit a mate, summon, fight -- which it says
            // in words and nothing else: read once against the table
            // of refusals (see `refused`, and `refusals` for the table).
            self.hear_refusal(&line.text, Instant::now());
            // And on a spell cast at the character: a caster attacks with
            // no notification, only this line (see
            // `autoplay::spell_attacker`).
            self.hear_spell_attack(&line.text, Instant::now());
        }
        let text = match (op, line.sender.is_empty()) {
            _ if line.kind == ac_net::messages::turbine::KIND => {
                let room = ac_net::messages::turbine::name(line.sender_id);
                format!("[{room}] {}: {}", line.sender, line.text)
            }
            _ if line.kind == ac_net::messages::channel::KIND => {
                let channel = ac_net::messages::channel::name(line.sender_id);
                if line.sender.is_empty() {
                    format!("[{channel}] You say, \"{}\"", line.text)
                } else {
                    format!("[{channel}] {} says, \"{}\"", line.sender, line.text)
                }
            }
            (_, true) => line.text.clone(),
            (opcode::EMOTE_TEXT | opcode::SOUL_EMOTE, _) => {
                format!("{} {}", line.sender, line.text)
            }
            (opcode::GAME_EVENT, _) => format!("{} tells you, \"{}\"", line.sender, line.text),
            _ => format!("{} says, \"{}\"", line.sender, line.text),
        };
        tracing::info!("chat: {text}");
        self.recall_notice(&text);
        self.events.push(Event::Chat {
            text,
            kind: line.kind,
        });
    }
}

/// Whether `chat_message` takes a message: the speech opcodes, and for
/// GameEvent (`ev` is the event type) the notices it turns into chat
/// lines. Keeps the unhandled-message log honest.
pub(super) fn chat_handles(op: u32, ev: u32) -> bool {
    use ac_net::messages::{event, opcode};
    match op {
        opcode::SOUND
        | opcode::TURBINE_CHAT
        | opcode::HEAR_SPEECH
        | opcode::HEAR_RANGED_SPEECH
        | opcode::SERVER_MESSAGE
        | opcode::EMOTE_TEXT
        | opcode::SOUL_EMOTE
        | opcode::PLAYER_KILLED => true,
        opcode::GAME_EVENT => matches!(
            ev,
            event::TELL
                | event::CHANNEL_BROADCAST
                | event::BOOK_DATA_RESPONSE
                | event::BOOK_PAGE_DATA_RESPONSE
                | event::SALVAGE_OPERATIONS_RESULT
                | event::IDENTIFY_OBJECT_RESPONSE
                | event::ATTACK_DONE
                | event::ATTACKER_NOTIFICATION
                | event::DEFENDER_NOTIFICATION
                | event::EVASION_ATTACKER_NOTIFICATION
                | event::EVASION_DEFENDER_NOTIFICATION
                | event::VICTIM_NOTIFICATION
                | event::KILLER_NOTIFICATION
                | event::TRANSIENT_STRING
                | event::WEENIE_ERROR
                | event::WEENIE_ERROR_WITH_STRING
                | event::POPUP_STRING
        ),
        _ => false,
    }
}
