//! What each item was picked up for, remembered.
//!
//! The decision about an item is made once, when it is taken off a
//! corpse, and it stands. Asking again later is how something taken to
//! keep gets sold on the next run to town: the rules that judged it the
//! first time are judged again against a pack that has changed, a cap
//! that has filled, an appraisal that has since landed.
//!
//! # Why a guid is enough, and where it is not
//!
//! A server object's id is freed only when the object is destroyed
//! (ACE's `WorldObject.Destroy` is the only caller of
//! `RecycleDynamicGuid` that matters), so an item sitting in a pack
//! keeps its id across a relog, a crash and a server restart. It is a
//! good key. Two things spoil it, and both are handled here rather than
//! worried about at every call site:
//!
//! * **A freed id comes back.** Six hours later from the recycle queue,
//!   or immediately after a restart out of the sequence gaps. So an
//!   entry is kept only while the thing is still held ([`forget_gone`])
//!   and only applies to an item of the same kind ([`Ledger::of`]).
//! * **Stacks split and merge.** Splitting a stack makes a *new* object
//!   with a new id, and merging destroys one. So stackables are not
//!   remembered at all: what to do with a taper is settled by its kind,
//!   which is the same answer every time, and asking the rules afresh
//!   costs nothing.
//!
//! What is left is unique items -- a piece of armour, a weapon, a jewel
//! -- which is exactly the set whose decision is about *that* item and
//! cannot be recovered from its kind.
//!
//! # Which way it fails
//!
//! An item with no entry is never sold. A ledger that is lost, stale or
//! unreadable therefore costs a full pack, never a sold heirloom, and
//! that is the only acceptable direction for this to fail in.

use crate::items::ItemStats;
use crate::profile::LootAction;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What one item was taken for, and enough about it to know it again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Took {
    pub action: LootAction,
    /// The kind of thing it was when the decision was made. An id that
    /// has been recycled onto something else will not match, and the
    /// entry is ignored rather than applied to a stranger.
    pub wcid: u32,
    /// For the log and the panel: which ring it was.
    #[serde(default)]
    pub name: String,
    /// Set when what was decided could not be carried out -- a counter
    /// that would not take it, a salvage that was refused. It is not a
    /// new decision, which is why it does not overwrite one: an item
    /// that cannot be salvaged today is still an item meant for
    /// salvage.
    ///
    /// And it is a wait, not a grudge. Nothing is given up on for good
    /// (`docs/agent.md`): a session runs for days, the salvager who
    /// would not take it logs back in, the counter that would not have
    /// it is not the next counter. So it carries the hour it happened
    /// and stops counting after [`TRY_AGAIN_AFTER`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<Failed>,
}

/// What went wrong, and when, so that it stops mattering.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failed {
    pub why: String,
    /// Unix seconds. Unix rather than an `Instant` because it outlives
    /// the process that wrote it.
    pub at: u64,
}

/// How long a failure is held against an item.
///
/// Long enough that a character does not spend an afternoon offering
/// the same ring to the same counter, short enough that a run tomorrow
/// tries again.
pub const TRY_AGAIN_AFTER: u64 = 60 * 60;

impl Failed {
    /// Whether this is still worth holding against the item.
    pub fn holds(&self, now: u64) -> bool {
        now.saturating_sub(self.at) < TRY_AGAIN_AFTER
    }
}

/// Whether this item is one the ledger remembers.
///
/// Stackables are not: their ids churn as stacks split and merge, and
/// their kind settles them anyway.
pub fn remembered(item: &ItemStats) -> bool {
    item.max_stack <= 1
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Ledger {
    #[serde(default)]
    took: BTreeMap<u32, Took>,
    /// Changed since it was last written. Not part of the file.
    #[serde(skip)]
    dirty: bool,
}

impl Ledger {
    pub fn new() -> Self {
        Ledger::default()
    }

    /// Write down what an item was taken for.
    ///
    /// Stackables are ignored: see [`remembered`].
    pub fn remember(&mut self, item: &ItemStats, action: LootAction) {
        if !remembered(item) {
            return;
        }
        self.dirty = true;
        self.took.insert(
            item.guid,
            Took {
                action,
                wcid: item.wcid,
                name: item.name.clone(),
                failed: None,
            },
        );
    }

    /// What this item was taken for, if it is a thing we remember and
    /// the entry still describes it.
    pub fn of(&self, item: &ItemStats) -> Option<LootAction> {
        let took = self.took.get(&item.guid)?;
        // A recycled id wearing someone else's clothes.
        (took.wcid == item.wcid).then_some(took.action)
    }

    /// The same, by id alone, for a caller that has nothing but an id.
    /// Prefer [`Ledger::of`]: this one cannot tell a recycled id apart.
    pub fn by_guid(&self, guid: u32) -> Option<LootAction> {
        self.took.get(&guid).map(|t| t.action)
    }

    /// Note that what was decided could not be done. The decision
    /// stands; only the record of the attempt is added.
    pub fn failed(&mut self, guid: u32, why: impl Into<String>, now: u64) {
        if let Some(t) = self.took.get_mut(&guid) {
            t.failed = Some(Failed {
                why: why.into(),
                at: now,
            });
            self.dirty = true;
        }
    }

    pub fn why_failed(&self, guid: u32, now: u64) -> Option<&str> {
        let f = self.took.get(&guid)?.failed.as_ref()?;
        f.holds(now).then_some(f.why.as_str())
    }

    /// Forget the things no longer held.
    ///
    /// This is what keeps a recycled id from ever mattering: an entry
    /// only survives while its item does, so there is nothing left to
    /// misapply when the id is handed to something new.
    pub fn forget_gone(&mut self, held: &[u32]) -> usize {
        let before = self.took.len();
        self.took.retain(|guid, _| held.contains(guid));
        let gone = before - self.took.len();
        self.dirty |= gone > 0;
        gone
    }

    pub fn forget(&mut self, guid: u32) {
        self.dirty |= self.took.remove(&guid).is_some();
    }

    /// Whether it has changed since it was last written out.
    pub fn unsaved(&self) -> bool {
        self.dirty
    }

    /// Everything written down as meant for a counter and not lately
    /// refused by one.
    pub fn for_sale(&self, now: u64) -> Vec<u32> {
        self.with(LootAction::Sell, now)
    }

    /// Everything written down as meant for the salvage bag and not
    /// lately refused.
    pub fn for_salvage(&self, now: u64) -> Vec<u32> {
        self.with(LootAction::Salvage, now)
    }

    fn with(&self, action: LootAction, now: u64) -> Vec<u32> {
        self.took
            .iter()
            .filter(|(_, t)| t.action == action)
            .filter(|(_, t)| !t.failed.as_ref().is_some_and(|f| f.holds(now)))
            .map(|(g, _)| *g)
            .collect()
    }

    /// Every decision, by id, in the shape the older readers expect.
    pub fn actions(&self) -> BTreeMap<u32, LootAction> {
        self.took.iter().map(|(g, t)| (*g, t.action)).collect()
    }

    pub fn len(&self) -> usize {
        self.took.len()
    }

    pub fn is_empty(&self) -> bool {
        self.took.is_empty()
    }

    /// Where one character's ledger lives. A server and a character,
    /// because ids are a server's own numbering and two worlds share
    /// none of them.
    pub fn path_of(dir: &Path, server: &str, character: &str) -> PathBuf {
        let tidy = |s: &str| {
            s.chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect::<String>()
        };
        dir.join("taken")
            .join(tidy(server))
            .join(format!("{}.json", tidy(character)))
    }

    /// Read one back. A ledger that will not read is an empty one,
    /// which sells nothing -- see the note at the top of this file.
    pub fn load(path: &Path) -> Ledger {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// Write it out. Called when it changes rather than when the client
    /// closes, because a crash is the thing it is defending against.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(self).unwrap_or_default();
        std::fs::write(path, text)
    }

    /// Write it out if anything has changed, and note that it is
    /// written. Called every tick; costs a bool most of the time.
    pub fn save_if_changed(&mut self, path: &Path) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        if let Err(e) = self.save(path) {
            tracing::warn!(path = %path.display(), "cannot write the loot ledger: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thing(guid: u32, wcid: u32, name: &str) -> ItemStats {
        ItemStats {
            guid,
            wcid,
            name: name.into(),
            max_stack: 1,
            ..Default::default()
        }
    }

    fn pile(guid: u32, wcid: u32, name: &str) -> ItemStats {
        ItemStats {
            max_stack: 100,
            ..thing(guid, wcid, name)
        }
    }

    #[test]
    fn what_it_was_taken_for_is_what_it_is_still_for() {
        let mut l = Ledger::new();
        let ring = thing(1, 500, "Ornate Ring");
        l.remember(&ring, LootAction::Keep);
        assert_eq!(l.of(&ring), Some(LootAction::Keep));
        assert!(l.for_sale(0).is_empty(), "a keeper is never for sale");
    }

    #[test]
    fn a_recycled_id_wearing_someone_elses_clothes_is_ignored() {
        // The id was freed when the ring was sold and the server handed
        // it to a sword six hours later. The entry must not decide the
        // sword's fate.
        let mut l = Ledger::new();
        l.remember(&thing(1, 500, "Ornate Ring"), LootAction::Sell);
        let sword = thing(1, 999, "Shou-jen Sword");
        assert_eq!(l.of(&sword), None);
    }

    #[test]
    fn stacks_are_not_remembered_at_all() {
        // Splitting a stack makes a new object and merging destroys
        // one, so an id means nothing here. A taper is settled by being
        // a taper.
        let mut l = Ledger::new();
        l.remember(&pile(7, 691, "Prismatic Taper"), LootAction::Keep);
        assert!(l.is_empty(), "nothing written down");
        assert_eq!(l.of(&pile(7, 691, "Prismatic Taper")), None);
    }

    #[test]
    fn an_entry_lives_only_as_long_as_the_thing_does() {
        let mut l = Ledger::new();
        let ring = thing(1, 500, "Ornate Ring");
        let jewel = thing(2, 501, "Jewel");
        l.remember(&ring, LootAction::Keep);
        l.remember(&jewel, LootAction::Sell);
        assert_eq!(l.forget_gone(&[1]), 1, "the jewel is gone");
        assert_eq!(l.of(&ring), Some(LootAction::Keep));
        assert_eq!(l.of(&jewel), None);
    }

    #[test]
    fn a_refusal_is_not_a_new_decision() {
        // A counter that will not take it does not make it a keeper: it
        // is still meant for the counter, and the next counter may.
        let mut l = Ledger::new();
        let ring = thing(1, 500, "Ornate Ring");
        let now = 1_000_000;
        l.remember(&ring, LootAction::Sell);
        l.failed(1, "no vendor will take it", now);
        assert_eq!(l.of(&ring), Some(LootAction::Sell), "still meant to go");
        assert!(l.for_sale(now).is_empty(), "but not offered again now");
        assert_eq!(l.why_failed(1, now), Some("no vendor will take it"));
        // A wait, not a grudge: the counter that would not have it is
        // not the next counter, and a session runs for days.
        let later = now + TRY_AGAIN_AFTER + 1;
        assert_eq!(l.for_sale(later), vec![1], "tried again later");
        assert_eq!(l.why_failed(1, later), None);
    }

    #[test]
    fn salvage_waits_until_the_looting_is_done() {
        let mut l = Ledger::new();
        let plate = thing(3, 700, "Platemail Greaves");
        l.remember(&plate, LootAction::Salvage);
        assert_eq!(l.for_salvage(0), vec![3]);
        assert!(l.for_sale(0).is_empty(), "salvage is not for sale");
    }

    #[test]
    fn it_survives_being_written_out_and_read_back() {
        let mut l = Ledger::new();
        l.remember(&thing(1, 500, "Ornate Ring"), LootAction::Keep);
        l.remember(&thing(2, 501, "Jewel"), LootAction::Sell);
        let text = serde_json::to_string(&l).unwrap();
        let back: Ledger = serde_json::from_str(&text).unwrap();
        assert_eq!(
            back.of(&thing(1, 500, "Ornate Ring")),
            Some(LootAction::Keep)
        );
        assert_eq!(back.for_sale(0), vec![2]);
    }

    #[test]
    fn one_ledger_per_server_and_character() {
        let dir = Path::new("/tmp/x");
        let a = Ledger::path_of(dir, "play.coldeve.ac", "Blargerton");
        let b = Ledger::path_of(dir, "127.0.0.1", "Blargerton");
        assert_ne!(a, b, "two worlds share no ids");
    }
}
