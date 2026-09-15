pub const FELLOW: u32 = 0x0000_0800;
pub const VASSALS: u32 = 0x0000_1000;
pub const PATRON: u32 = 0x0000_2000;
pub const MONARCH: u32 = 0x0000_4000;
pub const CO_VASSALS: u32 = 0x0100_0000;
/// The `ChatLine::kind` a channel line is tagged with; not a
/// ChatMessageType, the line carries the channel id instead.
pub const KIND: u32 = 0x1000_0000;

pub fn name(id: u32) -> &'static str {
    match id {
        0x1 => "Abuse",
        0x2 => "Admin",
        0x4 => "Audit",
        0x8 | 0x10 | 0x20 => "Advocate",
        0x100 => "Debug",
        0x200 => "Sentinel",
        0x400 => "Help",
        FELLOW => "Fellowship",
        VASSALS => "Vassals",
        PATRON => "Patron",
        MONARCH => "Monarch",
        CO_VASSALS => "Co-vassals",
        _ => "Channel",
    }
}

/// The channel a `/v`, `/p`, `/m`, `/c` or `/f` chat prefix means.
pub fn from_prefix(p: &str) -> Option<u32> {
    match p {
        "v" | "vassals" => Some(VASSALS),
        "p" | "patron" => Some(PATRON),
        "m" | "monarch" => Some(MONARCH),
        "c" | "covassals" => Some(CO_VASSALS),
        "f" | "fellow" => Some(FELLOW),
        _ => None,
    }
}
