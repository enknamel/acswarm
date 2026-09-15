use crate::wire::{Reader, Truncated};

/// Sound (0xF750): an object plays a sound-table entry: `(guid, sound type, volume)`.
pub fn parse_sound(body: &[u8]) -> Result<(u32, u32, f32), Truncated> {
    let mut r = Reader::new(body);
    Ok((r.u32()?, r.u32()?, r.f32()?))
}

/// PlayEffect (0xF755): an object plays a particle script: `(guid,
/// PlayScript id, speed)`.
pub fn parse_play_effect(body: &[u8]) -> Result<(u32, u32, f32), Truncated> {
    let mut r = Reader::new(body);
    Ok((r.u32()?, r.u32()?, r.f32()?))
}
