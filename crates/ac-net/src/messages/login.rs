use super::opcode;
use crate::wire::{Reader, Truncated, Writer};

/// Client version string the end-of-retail client reports.
pub const CLIENT_VERSION: &str = "1802";

/// Body of the LoginRequest packet (sent as the optional header of a packet
/// flagged `LOGIN_REQUEST`, not as a fragment).
pub fn login_request(account: &str, password: &str, timestamp: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.string16(CLIENT_VERSION);
    // Everything after the length dword.
    let mut rest = Writer::new();
    rest.u32(2); // NetAuthType::AccountPassword
    rest.u32(0); // AuthFlags::None
    rest.u32(timestamp);
    rest.string16(account);
    rest.string16(""); // account to log in as (admin override)
                       // "String32L": u32 byte length of what follows, which is a one-byte
                       // (two-byte above 255) length prefix and the bytes. ACE skips the
                       // prefix bytes and reads the rest as the password.
    let mut pw = Writer::new();
    if password.len() > 255 {
        pw.u16(password.len() as u16);
    } else {
        pw.u8(password.len() as u8);
    }
    pw.bytes(password.as_bytes());
    rest.u32(pw.buf.len() as u32);
    rest.bytes(&pw.buf);
    w.u32(rest.buf.len() as u32);
    w.bytes(&rest.buf);
    w.finish()
}

/// Message body for CharacterEnterWorldRequest (0xF7C8): opcode only.
pub fn enter_world_request() -> Vec<u8> {
    Writer::new()
        .u32(opcode::CHARACTER_ENTER_WORLD_REQUEST)
        .clone()
        .finish()
}

/// Message body for CharacterLogOff (0xF653): opcode only. The game's own
/// logout, as against the connection simply going away.
pub fn log_off() -> Vec<u8> {
    Writer::new()
        .u32(opcode::CHARACTER_LOG_OFF)
        .clone()
        .finish()
}

/// Message body for CharacterEnterWorld (0xF657).
pub fn enter_world(character_id: u32, account: &str) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(opcode::CHARACTER_ENTER_WORLD)
        .u32(character_id)
        .string16(account);
    w.finish()
}

/// Message body for CharacterDelete (0xF655): the account and the
/// character's slot (its index in the last CharacterList). ACE marks the
/// character for deletion, echoes 0xF655 and sends a fresh CharacterList.
pub fn character_delete(account: &str, slot: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(opcode::CHARACTER_DELETE).string16(account).u32(slot);
    w.finish()
}

/// Message body for CharacterRestore (0xF7D9): the character id. ACE
/// answers with a CharacterCreateResponse-shaped 0xF643 (1, id, name,
/// seconds greyed out) or a failure code (name in use, corrupt).
pub fn character_restore(character_id: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(opcode::CHARACTER_RESTORE).u32(character_id);
    w.finish()
}

/// One archive's iteration state for the DDD interrogation response.
#[derive(Debug, Clone, Copy)]
pub struct DatIteration {
    /// 1 = portal/highres, 2 = cell, 3 = language.
    pub dat_file_id: i32,
    /// 0 for the normal archive; the "HiFi" tag for client_highres.dat.
    pub dat_file_type: i32,
    pub iterations: i32,
}

/// DDD_InterrogationResponse (0xF7E6): tells the server which DAT
/// iterations we have. With matching iterations ACE replies DDD_EndDDD.
pub fn ddd_interrogation_response(dats: &[DatIteration]) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(opcode::DDD_INTERROGATION_RESPONSE);
    w.u32(1); // language: English
    w.i32(dats.len() as i32);
    for d in dats {
        w.i32(d.dat_file_type).i32(d.dat_file_id);
        // CMostlyConsecutiveIntSet: total, then a single run "-(n+1)"...
        // The client encodes a run of n consecutive iterations starting
        // at 1 as [n, -n?]; ACE only sums runs, so encode one negative run
        // covering all iterations: x < 0 contributes |x| - 1.
        w.i32(d.iterations);
        if d.iterations > 0 {
            w.i32(-(d.iterations + 1));
        }
    }
    w.i32(0); // iterations without keys: none
    w.u32(0); // flags
    w.finish()
}

/// Appearance choices for character creation (indices into the CharGen
/// option lists, hues in 0..1).
#[derive(Debug, Clone, Default)]
pub struct Appearance {
    pub eyes: u32,
    pub nose: u32,
    pub mouth: u32,
    pub hair_color: u32,
    pub eye_color: u32,
    pub hair_style: u32,
    /// `u32::MAX` = no headgear.
    pub headgear_style: u32,
    pub headgear_color: u32,
    pub shirt_style: u32,
    pub shirt_color: u32,
    pub pants_style: u32,
    pub pants_color: u32,
    pub footwear_style: u32,
    pub footwear_color: u32,
    pub skin_hue: f64,
    pub hair_hue: f64,
    pub headgear_hue: f64,
    pub shirt_hue: f64,
    pub pants_hue: f64,
    pub footwear_hue: f64,
}

#[derive(Debug, Clone)]
pub struct CharacterCreate {
    pub account: String,
    pub name: String,
    /// 1 = Aluvian, 2 = Gharu'ndim, 3 = Sho, ...
    pub heritage: u32,
    /// 1 = male, 2 = female.
    pub gender: u32,
    pub appearance: Appearance,
    /// Index into the heritage's template list.
    pub template: i32,
    pub strength: u32,
    pub endurance: u32,
    pub coordination: u32,
    pub quickness: u32,
    pub focus: u32,
    pub self_: u32,
    pub slot: u32,
    /// Advancement class per skill id, 55 entries: 0 inactive, 1 untrained,
    /// 2 trained, 3 specialized.
    pub skills: Vec<u32>,
    /// Index into CharGen starter areas.
    pub start_area: u32,
}

impl CharacterCreate {
    /// Message body for CharacterCreate (0xF656).
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u32(opcode::CHARACTER_CREATE).string16(&self.account);
        w.u32(1).u32(self.heritage).u32(self.gender);
        let a = &self.appearance;
        w.u32(a.eyes)
            .u32(a.nose)
            .u32(a.mouth)
            .u32(a.hair_color)
            .u32(a.eye_color)
            .u32(a.hair_style)
            .u32(a.headgear_style)
            .u32(a.headgear_color)
            .u32(a.shirt_style)
            .u32(a.shirt_color)
            .u32(a.pants_style)
            .u32(a.pants_color)
            .u32(a.footwear_style)
            .u32(a.footwear_color)
            .f64(a.skin_hue)
            .f64(a.hair_hue)
            .f64(a.headgear_hue)
            .f64(a.shirt_hue)
            .f64(a.pants_hue)
            .f64(a.footwear_hue);
        w.i32(self.template)
            .u32(self.strength)
            .u32(self.endurance)
            .u32(self.coordination)
            .u32(self.quickness)
            .u32(self.focus)
            .u32(self.self_)
            .u32(self.slot)
            .u32(0); // class id
        w.u32(self.skills.len() as u32);
        for &s in &self.skills {
            w.u32(s);
        }
        w.string16(&self.name).u32(self.start_area).u32(0).u32(0);
        w.finish()
    }
}

/// CharacterCreateResponse (0xF643) body.
#[derive(Debug, Clone, PartialEq)]
pub struct CharacterCreateResponse {
    /// 1 = ok, 2 = pending, 3 = name in use, 4 = name banned, 5 = corrupt,
    /// 6 = database down, 7 = admin privilege denied.
    pub response: u32,
    pub guid: u32,
    pub name: String,
}

impl CharacterCreateResponse {
    pub fn parse(b: &[u8]) -> Result<Self, Truncated> {
        let mut r = Reader::new(b);
        let response = r.u32()?;
        if response == 1 {
            let guid = r.u32()?;
            let name = r.string16()?;
            Ok(CharacterCreateResponse {
                response,
                guid,
                name,
            })
        } else {
            Ok(CharacterCreateResponse {
                response,
                guid: 0,
                name: String::new(),
            })
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CharacterEntry {
    pub id: u32,
    pub name: String,
    pub seconds_until_deleted: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CharacterList {
    pub characters: Vec<CharacterEntry>,
    pub slot_count: u32,
    pub account: String,
    pub use_turbine_chat: bool,
    pub has_throne_of_destiny: bool,
}

impl CharacterList {
    /// Parse the body after the opcode.
    pub fn parse(b: &[u8]) -> Result<Self, Truncated> {
        let mut r = Reader::new(b);
        let _ = r.u32()?;
        let n = r.u32()?;
        let mut characters = Vec::with_capacity(n.min(64) as usize);
        for _ in 0..n {
            characters.push(CharacterEntry {
                id: r.u32()?,
                name: r.string16()?,
                seconds_until_deleted: r.u32()?,
            });
        }
        let _ = r.u32()?;
        let slot_count = r.u32()?;
        let account = r.string16()?;
        let use_turbine_chat = r.u32()? != 0;
        let has_throne_of_destiny = r.u32()? != 0;
        Ok(CharacterList {
            characters,
            slot_count,
            account,
            use_turbine_chat,
            has_throne_of_destiny,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServerName {
    pub current_connections: u32,
    pub max_connections: i32,
    pub name: String,
}

impl ServerName {
    pub fn parse(b: &[u8]) -> Result<Self, Truncated> {
        let mut r = Reader::new(b);
        Ok(ServerName {
            current_connections: r.u32()?,
            max_connections: r.i32()?,
            name: r.string16()?,
        })
    }
}

/// AccountBanned (0xF7C1): seconds until the ban ends and the reason
/// (empty when the server gave none).
pub fn parse_account_banned(body: &[u8]) -> Result<(u32, String), Truncated> {
    let mut r = Reader::new(body);
    let secs = r.u32()?;
    let reason = r.string16().unwrap_or_default();
    Ok((secs, reason))
}

#[cfg(test)]
mod tests;
