//! A quest's chain in the data's terms: who starts it, the flags it needs and who stamps them,
//! where its item or kills come from and the way in, who rewards it; a place in words; and
//! one card per quest for a list's filters.

use crate::portals::Portal;
use crate::quests::{Kind, Repeat, Row, Source, Want};
use crate::unlocks::Unlock;
use crate::Place;

/// Who stamps a flag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FlagBy {
    /// A talk to an unlock NPC at its level (`crate::unlocks`).
    Unlock(&'static Unlock),
    /// A quest row's reward or talk (`crate::quests::stamped_by`).
    Quest(&'static Row),
}

/// One place what a quest wants comes from.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceGround {
    pub landblock: u32,
    pub source: &'static Source,
    /// The source's spawn entries there.
    pub entries: u32,
    /// Where a journey to hunt there ends; `None` when no ground or portal is known.
    pub place: Option<Place>,
    /// The portal in, when only a portal reaches it: its level and flag gate the hunt.
    pub way_in: Option<&'static Portal>,
}

/// One link of a quest's chain, in the order a player does them.
#[derive(Clone, Debug, PartialEq)]
pub enum ChainLink {
    /// Talk to a giver who starts it (a `Kind::Start` row, at any of its placements).
    TakeUp(&'static Row),
    /// A flag held first, who stamps it (empty: nobody known), and the portal asking it, if one.
    Flag {
        flag: String,
        by: Vec<FlagBy>,
        portal: Option<&'static Portal>,
    },
    /// Kill or loot it, best place first.
    Hunt(Vec<SourceGround>),
    /// An NPC hands the item out.
    Fetch(&'static Source),
    /// Hand it in to any of these rows' givers.
    HandIn(Vec<&'static Row>),
}

/// One quest's facets for the Quests window's filters, built once from the data.
#[derive(Clone, Debug, PartialEq)]
pub struct QuestCard {
    pub id: String,
    /// The title, or the wanted item's name when the title is internal.
    pub title: String,
    pub kind: Kind,
    pub item: Option<String>,
    /// Giver names with their places in words, one per placement.
    pub givers: Vec<(String, String)>,
    /// Source creatures or NPCs: name, level, chance per kill.
    pub creatures: Vec<(String, u32, f32)>,
    /// Source places in words.
    pub places: Vec<String>,
    /// The giver band, 0 for open.
    pub levels: (u32, u32),
    /// The easiest source's level.
    pub easiest: Option<u32>,
    /// The way-in portal's level, 0 for none.
    pub entry: u32,
    /// The fixed XP (the board gives it at the character's level).
    pub xp: u64,
    /// Reward items in words ("Trade Note (75,000) x1", "item 31606"); a random reply's are
    /// alternatives.
    pub gives: Vec<String>,
    pub repeat: Repeat,
    /// Flags it needs: its rows' needs and its way in's.
    pub flags: Vec<String>,
    /// What its rows do, in the data's words (`xp`, `item`, `flag`, `luminance`...).
    pub does: Vec<String>,
    /// What its rows ask that the client cannot check, in words.
    pub unread: Vec<String>,
}

/// Working portals into `landblock` from another, lowest level first, flagged ones included.
pub fn ways_in(landblock: u32) -> Vec<&'static Portal> {
    let _ = landblock;
    Vec::new()
}

/// Who stamps `flag`: unlock NPCs, then quest rows; empty when nobody known.
pub fn flag_givers(flag: &str) -> Vec<FlagBy> {
    let _ = flag;
    Vec::new()
}

/// Where `want` comes from, best first (spawn entries x chance, then the lower way in).
pub fn source_grounds(want: &Want) -> Vec<SourceGround> {
    let _ = want;
    Vec::new()
}

/// Quest `id`'s chain, in order.
pub fn chain_of(id: &str) -> Vec<ChainLink> {
    let _ = id;
    Vec::new()
}

/// A cell's place in words: its dungeon, the town by its portal, near a town, else map coordinates.
pub fn place_name(cell: u32) -> String {
    format!("{cell:08X}")
}

/// An item's name from the quest and shop data, by weenie class.
pub fn reward_name(wcid: u32) -> Option<&'static str> {
    let _ = wcid;
    None
}

/// One card per quest id, in the data's order, built on first use.
pub fn quest_cards() -> &'static [QuestCard] {
    &[]
}
