use super::channel;
use crate::wire::{Reader, Truncated};

/// A line of chat from any of the speech-carrying messages.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatLine {
    pub text: String,
    pub sender: String,
    pub sender_id: u32,
    /// ChatMessageType (0 broadcast, 2 speech, 3 tell, 0x1F emote...).
    pub kind: u32,
}

impl ChatLine {
    /// HearSpeech 0x02BB: text, sender, sender id, type.
    pub fn parse_hear_speech(body: &[u8]) -> Result<Self, Truncated> {
        let mut r = Reader::new(body);
        let text = r.string16()?;
        let sender = r.string16()?;
        let sender_id = r.u32()?;
        let kind = r.u32()?;
        Ok(ChatLine {
            text,
            sender,
            sender_id,
            kind,
        })
    }
    /// HearRangedSpeech 0x02BC: text, sender, sender id, range, type.
    pub fn parse_hear_ranged_speech(body: &[u8]) -> Result<Self, Truncated> {
        let mut r = Reader::new(body);
        let text = r.string16()?;
        let sender = r.string16()?;
        let sender_id = r.u32()?;
        let _range = r.f32()?;
        let kind = r.u32()?;
        Ok(ChatLine {
            text,
            sender,
            sender_id,
            kind,
        })
    }
    /// ServerMessage 0xF7E0: text, type.
    pub fn parse_server_message(body: &[u8]) -> Result<Self, Truncated> {
        let mut r = Reader::new(body);
        let text = r.string16()?;
        let kind = r.i32()? as u32;
        Ok(ChatLine {
            text,
            sender: String::new(),
            sender_id: 0,
            kind,
        })
    }
    /// EmoteText 0x01E0: sender id, sender, text.
    pub fn parse_emote_text(body: &[u8]) -> Result<Self, Truncated> {
        let mut r = Reader::new(body);
        let sender_id = r.u32()?;
        let sender = r.string16()?;
        let text = r.string16()?;
        Ok(ChatLine {
            text,
            sender,
            sender_id,
            kind: 0x1F,
        })
    }
    /// GameEvent Tell 0x02BD: text, sender, sender id, target id, type.
    pub fn parse_tell(body: &[u8]) -> Result<Self, Truncated> {
        let mut r = Reader::new(body);
        let text = r.string16()?;
        let sender = r.string16()?;
        let sender_id = r.u32()?;
        let _target = r.u32()?;
        let kind = r.u32()?;
        Ok(ChatLine {
            text,
            sender,
            sender_id,
            kind,
        })
    }

    /// ChannelBroadcast event 0x0147: channel id, sender ("" for our
    /// own line), text. The channel id is kept in `sender_id` and the
    /// kind is `channel::KIND`.
    pub fn parse_channel_broadcast(body: &[u8]) -> Result<Self, Truncated> {
        let mut r = Reader::new(body);
        let channel = r.u32()?;
        let sender = r.string16()?;
        let text = r.string16()?;
        Ok(ChatLine {
            text,
            sender,
            sender_id: channel,
            kind: channel::KIND,
        })
    }
}

#[cfg(test)]
mod tests;
