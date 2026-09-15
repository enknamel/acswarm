use super::ChatLine;
use crate::wire::{Reader, Truncated, Writer};

pub const ALLEGIANCE: u32 = 1;
pub const GENERAL: u32 = 2;
pub const TRADE: u32 = 3;
pub const LFG: u32 = 4;
pub const ROLEPLAY: u32 = 5;
pub const SOCIETY: u32 = 6;
pub const OLTHOI: u32 = 10;
/// The `ChatLine::kind` a room line is tagged with; `sender_id`
/// then holds the room id.
pub const KIND: u32 = 0x2000_0000;

const EVENT_BINARY: u32 = 1;
const REQUEST_BINARY: u32 = 3;
const SEND_TO_ROOM_BY_ID: u32 = 2;

/// The room's name; ids above Olthoi are allegiance rooms.
pub fn name(id: u32) -> &'static str {
    match id {
        ALLEGIANCE => "Allegiance",
        GENERAL => "General",
        TRADE => "Trade",
        LFG => "LFG",
        ROLEPLAY => "Roleplay",
        SOCIETY | 7..=9 => "Society",
        OLTHOI => "Olthoi",
        _ => "Allegiance",
    }
}

/// The room a `/g`, `/trade`, `/lfg`, `/rp` or `/a` prefix means
/// (`ALLEGIANCE` stands for "our allegiance's room").
pub fn from_prefix(p: &str) -> Option<u32> {
    match p {
        "g" | "general" => Some(GENERAL),
        "tr" | "trade" => Some(TRADE),
        "lfg" => Some(LFG),
        "rp" | "roleplay" => Some(ROLEPLAY),
        "a" | "allegiance" => Some(ALLEGIANCE),
        _ => None,
    }
}

/// The ChatType the server expects with a room id.
pub fn chat_type(room: u32) -> u32 {
    match room {
        GENERAL | TRADE | LFG | ROLEPLAY | OLTHOI => room,
        SOCIETY..=9 => SOCIETY,
        _ => ALLEGIANCE,
    }
}

/// A counted UTF-16 string: a length byte (0x80 then the length
/// when it does not fit), then the code units.
fn read_wstring(r: &mut Reader) -> Result<String, Truncated> {
    let mut n = r.u8()? as usize;
    if n & 0x80 != 0 {
        n = r.u8()? as usize;
    }
    let bytes = r.bytes(n * 2)?;
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    Ok(String::from_utf16_lossy(&units))
}

fn write_wstring(w: &mut Writer, s: &str) {
    let units: Vec<u16> = s.encode_utf16().take(255).collect();
    if units.len() < 128 {
        w.u8(units.len() as u8);
    } else {
        w.u8(0x80).u8(units.len() as u8);
    }
    for u in units {
        w.u16(u);
    }
}

/// Decode a Turbine chat message body (after the 0xF7DE opcode).
/// `Ok(None)` for blobs that are not room lines (our own acks).
pub fn parse(body: &[u8]) -> Result<Option<ChatLine>, Truncated> {
    let mut r = Reader::new(body);
    let _size = r.u32()?;
    let blob = r.u32()?;
    let _dispatch = r.u32()?;
    for _ in 0..5 {
        r.u32()?;
    }
    let _payload_size = r.u32()?;
    if blob != EVENT_BINARY {
        return Ok(None);
    }
    let room = r.u32()?;
    let sender = read_wstring(&mut r)?;
    let text = read_wstring(&mut r)?;
    let _extra = r.u32()?;
    let _sender_guid = r.u32()?;
    let _result = r.u32()?;
    let _chat_type = r.u32()?;
    Ok(Some(ChatLine {
        text,
        sender,
        sender_id: room,
        kind: KIND,
    }))
}

/// Encode a line for a room (blob type request, dispatch "send to
/// room by id"): the whole 0xF7DE message including the opcode.
pub fn encode(room: u32, sender: u32, text: &str, context: u32) -> Vec<u8> {
    let mut payload = Writer::new();
    payload.u32(context).u32(2).u32(2).u32(room);
    write_wstring(&mut payload, text);
    payload.u32(0x0C).u32(sender).u32(0).u32(chat_type(room));
    let payload = payload.finish();
    let mut blob = Writer::new();
    blob.u32(REQUEST_BINARY)
        .u32(SEND_TO_ROOM_BY_ID)
        .u32(1)
        .u32(0)
        .u32(0)
        .u32(0)
        .u32(0)
        .u32(payload.len() as u32)
        .bytes(&payload);
    let blob = blob.finish();
    let mut w = Writer::new();
    w.u32(super::opcode::TURBINE_CHAT)
        .u32(blob.len() as u32)
        .bytes(&blob);
    w.finish()
}

#[cfg(test)]
mod tests;
