use crate::wire::{Reader, Truncated};

/// PropertyInt ids carried by the Public/PrivateUpdatePropertyInt
/// messages (`u8 sequence, [u32 guid], u32 property, i32 value`).
pub mod property_int {
    pub const MAX_STACK_SIZE: u32 = 11;
    pub const STACK_SIZE: u32 = 12;
    pub const VALUE: u32 = 19;
}

/// ACE `PropertyString` ids.
pub mod property_string {
    pub const NAME: u32 = 1;
    pub const TITLE: u32 = 2;
}

/// SetStackSize (0x0197): `u8 sequence, u32 guid, u32 stack size, u32
/// value`, sent for every change of a stack in view (including our own
/// packs, after a spell burns components or a vendor buy merges).
pub fn parse_set_stack_size(body: &[u8]) -> Result<(u32, u32, u32), Truncated> {
    let mut r = Reader::new(body);
    let _seq = r.u8()?;
    Ok((r.u32()?, r.u32()?, r.u32()?))
}
