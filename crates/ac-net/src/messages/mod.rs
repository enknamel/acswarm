//! Message opcodes and the bodies needed for login and entering the world.
//! Everything else is delivered to the caller as `(opcode, bytes)`.

use crate::wire::{Reader, Writer};

/// GameAction type ids (inside a 0xF7B1 message).
pub mod action;
/// Group chat channel ids (ACE `Channel`), for ChatChannel 0x0147 both
/// ways.
pub mod channel;
/// GameEvent (0xF7B0) sub-opcodes.
pub mod event;
/// Motion commands and stances used for basic movement.
pub mod motion;
pub mod opcode;
/// Message queues (fragment `queue` field).
pub mod queue;
/// Turbine chat (message 0xF7DE): the rooms every player can join
/// (General, Trade, LFG, Roleplay), the society rooms, and each
/// allegiance's own room (its id is the allegiance's biota id, sent in
/// SetTurbineChatChannels). A message is a "net blob": size, blob type
/// (1 event = a line from the room, 3 request = ours going out, 5
/// response = the server's ack), dispatch type, two (kind, id) pairs, a
/// cookie, then a sized payload. See ACE `GameMessageTurbineChat` and
/// `TurbineChatHandler`.
pub mod turbine;

mod appraise_info;
mod attack;
mod book_data;
mod containers;
mod effects;
mod fellowship_panel;
mod login;
mod movement;
mod property_updates;
mod salvage_result;
mod speech;
mod vendor_trade;

pub use appraise_info::{Appraisal, ArmorProfile, CreatureProfile, WeaponProfile};
pub use attack::{
    cast_targeted, combat_mode, parse_player_killed, parse_update_health, AttackNotice,
};
pub use book_data::{BookData, BookPage};
pub use containers::parse_view_contents;
pub use effects::{parse_play_effect, parse_sound};
pub use fellowship_panel::fellowship_update_request;
pub use login::{
    character_delete, character_restore, ddd_interrogation_response, enter_world,
    enter_world_request, log_off, login_request, parse_account_banned, Appearance, CharacterCreate,
    CharacterCreateResponse, CharacterEntry, CharacterList, DatIteration, ServerName,
    CLIENT_VERSION,
};
pub use movement::{autonomous_position, jump, move_to_state, RawMotion, WirePosition};
pub use property_updates::{parse_set_stack_size, property_int, property_string};
pub use salvage_result::{SalvageResult, SalvageYield};
pub use speech::ChatLine;
pub use vendor_trade::trade;

/// Split a message into opcode and body.
pub fn split(msg: &[u8]) -> Option<(u32, &[u8])> {
    if msg.len() < 4 {
        return None;
    }
    Some((
        u32::from_le_bytes([msg[0], msg[1], msg[2], msg[3]]),
        &msg[4..],
    ))
}

/// GameEvent (0xF7B0) header: object guid, sequence, event type; body follows.
pub fn split_game_event(body: &[u8]) -> Option<(u32, u32, u32, &[u8])> {
    let mut r = Reader::new(body);
    let guid = r.u32().ok()?;
    let seq = r.u32().ok()?;
    let ev = r.u32().ok()?;
    Some((guid, seq, ev, r.remaining()))
}

/// Build a GameAction (0xF7B1) message: opcode, sequence, action type, body.
pub fn game_action(sequence: u32, action: u32, body: &[u8]) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(opcode::GAME_ACTION)
        .u32(sequence)
        .u32(action)
        .bytes(body);
    w.finish()
}

#[cfg(test)]
mod tests;
