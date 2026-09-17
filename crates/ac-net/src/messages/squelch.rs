/// Everything a player says (ACE `ChatMessageType.AllChannels`); what a
/// squelch with no category named means.
pub const ALL: u32 = 0x01;
pub const SPEECH: u32 = 0x02;
pub const TELL: u32 = 0x03;

/// Every category name the retail client took, with its ChatMessageType
/// (FUN_006b1460). The display name comes first where two names share an
/// id, since [`name`] answers with the first ("Appraisal", not
/// "Assessment"; FUN_006b0e60).
const NAMES: &[(&str, u32)] = &[
    ("Default", 0x00),
    ("All", ALL),
    ("Speech", SPEECH),
    ("Tell", TELL),
    ("Speech_Direct_Send", 0x04),
    ("System", 0x05),
    ("Combat", 0x06),
    ("Magic", 0x07),
    ("Channel", 0x08),
    ("Channel_Send", 0x09),
    ("Social", 0x0A),
    ("Social_Send", 0x0B),
    ("Emote", 0x0C),
    ("Advancement", 0x0D),
    ("Abuse", 0x0E),
    ("Help", 0x0F),
    ("Appraisal", 0x10),
    ("Assessment", 0x10),
    ("Spellcasting", 0x11),
    ("Allegiance", 0x12),
    ("Fellowship", 0x13),
    ("World_Broadcast", 0x14),
    ("Combat_Enemy", 0x15),
    ("Combat_Self", 0x16),
    ("Recall", 0x17),
    ("Craft", 0x18),
    ("Salvaging", 0x19),
    ("Admin_Tell", 0x1F),
];

/// The categories `@messagetypes` lists, in the order it lists them
/// (FUN_006b0e10); the same set ACE takes per channel
/// (`SquelchManager.IsLegalChannel`, SquelchManager.cs:45).
pub const LISTABLE: &[u32] = &[
    0x02, 0x03, 0x06, 0x07, 0x0C, 0x10, 0x11, 0x12, 0x13, 0x15, 0x16, 0x17, 0x18, 0x19,
];

/// The ChatMessageType `name` stands for, whatever the case.
pub fn from_name(name: &str) -> Option<u32> {
    let name = name.trim();
    NAMES
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, kind)| *kind)
}

/// What the client calls a ChatMessageType.
pub fn name(kind: u32) -> Option<&'static str> {
    NAMES.iter().find(|(_, k)| *k == kind).map(|(n, _)| *n)
}

/// The listed categories a mask of `1 << kind` bits holds, in
/// [`LISTABLE`] order.
pub fn named_in_mask(mask: u32) -> Vec<&'static str> {
    LISTABLE
        .iter()
        .filter(|k| mask & (1 << *k) != 0)
        .filter_map(|k| name(*k))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_category_is_named_the_way_retail_named_it() {
        assert_eq!(from_name("tell"), Some(TELL));
        assert_eq!(from_name("Combat_Enemy"), Some(0x15));
        // Two names, one id, and the first is what we print back.
        assert_eq!(from_name("Assessment"), from_name("Appraisal"));
        assert_eq!(name(0x10), Some("Appraisal"));
        assert_eq!(from_name("channels"), None);
    }

    #[test]
    fn the_listed_categories_are_the_ones_a_squelch_takes() {
        assert_eq!(LISTABLE.len(), 14);
        assert_eq!(
            named_in_mask(u32::MAX).join(", "),
            "Speech, Tell, Combat, Magic, Emote, Appraisal, Spellcasting, Allegiance, \
             Fellowship, Combat_Enemy, Combat_Self, Recall, Craft, Salvaging"
        );
        assert_eq!(named_in_mask(1 << TELL), vec!["Tell"]);
        assert!(named_in_mask(0).is_empty());
    }
}
