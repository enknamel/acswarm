pub const ABUSE: u32 = 0x0000_0001;
pub const ADMIN: u32 = 0x0000_0002;
pub const AUDIT: u32 = 0x0000_0004;
pub const ADVOCATE1: u32 = 0x0000_0008;
pub const ADVOCATE2: u32 = 0x0000_0010;
pub const ADVOCATE3: u32 = 0x0000_0020;
pub const DEBUG: u32 = 0x0000_0100;
pub const SENTINEL: u32 = 0x0000_0200;
pub const HELP: u32 = 0x0000_0400;
pub const FELLOW: u32 = 0x0000_0800;
pub const VASSALS: u32 = 0x0000_1000;
pub const PATRON: u32 = 0x0000_2000;
pub const MONARCH: u32 = 0x0000_4000;
pub const CO_VASSALS: u32 = 0x0100_0000;
/// The monarch's broadcast to the whole allegiance (`/ab`,
/// `@allegiance broadcast`); ACE `Channel.cs:127`. Not the allegiance's
/// Turbine room, which `turbine::ALLEGIANCE` is.
pub const ALLEGIANCE_BROADCAST: u32 = 0x0200_0000;
pub const CELESTIAL_HAND: u32 = 0x0800_0000;
pub const ELDRYTCH_WEB: u32 = 0x1000_0000;
pub const RADIANT_BLOOD: u32 = 0x2000_0000;
pub const OLTHOI: u32 = 0x4000_0000;
/// The `ChatLine::kind` a channel line is tagged with; not a
/// ChatMessageType, the line carries the channel id instead.
pub const KIND: u32 = 0x1000_0000;

pub fn name(id: u32) -> &'static str {
    match id {
        ABUSE => "Abuse",
        ADMIN => "Admin",
        AUDIT => "Audit",
        ADVOCATE1 | ADVOCATE2 | ADVOCATE3 => "Advocate",
        DEBUG => "Debug",
        SENTINEL => "Sentinel",
        HELP => "Help",
        FELLOW => "Fellowship",
        VASSALS => "Vassals",
        PATRON => "Patron",
        MONARCH => "Monarch",
        CO_VASSALS => "Co-vassals",
        ALLEGIANCE_BROADCAST => "Allegiance Broadcast",
        CELESTIAL_HAND => "Celestial Hand",
        ELDRYTCH_WEB => "Eldrytch Web",
        RADIANT_BLOOD => "Radiant Blood",
        OLTHOI => "Olthoi",
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

/// The channel `@on`, `@off` and `@clist` name, the retail client's own
/// table (FUN_005d0190), whatever the case. `None` is the client's
/// "That channel doesn't exist."
pub fn from_name(name: &str) -> Option<u32> {
    let name = name.trim().to_ascii_lowercase();
    Some(match name.as_str() {
        "allegiance" | "a" | "ab" => ALLEGIANCE_BROADCAST,
        "co-vassals" | "covassals" | "covassal" | "c" => CO_VASSALS,
        "monarch" | "m" => MONARCH,
        "patron" | "p" => PATRON,
        "vassals" | "vassal" | "v" => VASSALS,
        "fellowship" | "fellows" | "fellow" | "f" | "group" | "g" | "party" => FELLOW,
        "av" | "av1" | "advocate" | "advocate1" => ADVOCATE1,
        "av2" | "advocate2" => ADVOCATE2,
        "av3" | "advocate3" => ADVOCATE3,
        "abuse" => ABUSE,
        "ad" | "admin" => ADMIN,
        "au" | "audit" => AUDIT,
        "help" => HELP,
        "sent" | "sentinel" => SENTINEL,
        "celestialhand" | "celhan" => CELESTIAL_HAND,
        "eldrytchweb" | "eldweb" => ELDRYTCH_WEB,
        "radiantblood" | "radblo" => RADIANT_BLOOD,
        "olthoi" | "ol" => OLTHOI,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_channel_is_named_the_way_retail_named_it() {
        assert_eq!(from_name("Advocate2"), Some(ADVOCATE2));
        assert_eq!(from_name(" ol "), Some(OLTHOI));
        assert_eq!(from_name("party"), Some(FELLOW));
        assert_eq!(from_name("ab"), Some(ALLEGIANCE_BROADCAST));
        assert_eq!(from_name("celhan"), Some(CELESTIAL_HAND));
        // The prefix table is the chat bar's, and answers fewer names.
        assert_eq!(from_name("general"), None);
        assert_eq!(from_prefix("party"), None);
        assert_eq!(name(from_name("sentinel").unwrap()), "Sentinel");
    }
}
