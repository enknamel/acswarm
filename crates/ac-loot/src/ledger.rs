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
//! * **Stacks split and merge.** Splitting a stack makes a *new*
//!   object with a new id, and merging destroys one. A split half
//!   therefore arrives with no entry and is judged afresh, which is
//!   right; a merge is the one case where two entries have to become
//!   one, and [`Ledger::merged`] settles it before the stacks are
//!   poured together.
//!
//! Stackables were once left out of here for that reason -- a taper is
//! settled by being a taper, so nothing was lost by asking again. What
//! that cost was the *record*: a search across every character's packs
//! could say a character holds four thousand tapers but not what they
//! are for, which is most of the reason to keep a ledger anybody can
//! read (`ac_client::holdings`). So everything held is written down
//! now, and the churn is handled where it happens rather than avoided.
//!
//! # Which way it fails
//!
//! An item with no entry is never sold. A ledger that is lost, stale or
//! unreadable therefore costs a full pack, never a sold heirloom, and
//! that is the only acceptable direction for this to fail in.

use crate::items::ItemStats;
use crate::profile::LootAction;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Failed {
    pub why: String,
    /// Unix seconds. Unix rather than an `Instant` because it outlives
    /// the process that wrote it.
    pub at: u64,
}

impl<'de> Deserialize<'de> for Failed {
    /// Reads what is on disk today and what was on disk before a
    /// failure carried the hour it happened.
    ///
    /// The first shape of this field was a bare string. A file holding
    /// one must still read: failing to parse the ledger loses every
    /// decision the character has made, and the next run to town judges
    /// the whole pack afresh -- which is the heirloom sale this is all
    /// here to prevent.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Either {
            Now { why: String, at: u64 },
            Before(String),
        }
        Ok(match Either::deserialize(d)? {
            Either::Now { why, at } => Failed { why, at },
            // No hour, so it is taken as long past: forgiving, which is
            // the right way round for a thing that is only a wait.
            Either::Before(why) => Failed { why, at: 0 },
        })
    }
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

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Ledger {
    #[serde(default)]
    took: BTreeMap<u32, Took>,
    /// Changed since it was last written. Not part of the file.
    #[serde(skip)]
    dirty: bool,
    /// Bumped on every change, so a reader that has to notice one can
    /// do it by comparing a number rather than by walking the entries.
    ///
    /// What needs this is the holdings snapshot: what a thing is *for*
    /// travels with it, so a decision changing has to make the snapshot
    /// stale exactly as moving the thing would. Not part of the file --
    /// a ledger read back from disk starts again at zero, which is
    /// right, because nothing has compared against it yet.
    #[serde(skip)]
    version: u64,
    /// A fingerprint of the rules these decisions were made under.
    ///
    /// Written out with them, so that a profile edited while the client
    /// was off is noticed when it comes back. `None` in a file from
    /// before this was recorded, which reads as "unknown" and is left
    /// alone rather than re-judged on a guess.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rules: Option<u64>,
    /// A file was there and would not be read.
    ///
    /// This is not the same as having nothing written down, and the
    /// difference decides whether anything may be sold. An empty ledger
    /// says "no decisions", every item falls through to the rules, and
    /// a profile with a broad sell rule sells the keepers -- so a
    /// ledger that *lost* its decisions must not be mistaken for one
    /// that never had any.
    #[serde(skip)]
    unreadable: bool,
}

impl Ledger {
    pub fn new() -> Self {
        Ledger::default()
    }

    /// Something changed: it needs writing out, and anyone watching
    /// needs to notice.
    fn changed(&mut self) {
        self.dirty = true;
        self.version = self.version.wrapping_add(1);
    }

    /// A number that changes whenever any decision here does.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// The fingerprint of the rules these decisions were made under, or
    /// `None` when it was never recorded.
    pub fn rules(&self) -> Option<u64> {
        self.rules
    }

    /// Record which rules the decisions now standing were made under.
    pub fn judged_under(&mut self, rules: u64) {
        if self.rules != Some(rules) {
            self.rules = Some(rules);
            self.changed();
        }
    }

    /// Write down what an item was taken for.
    pub fn remember(&mut self, item: &ItemStats, action: LootAction) {
        self.changed();
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
            self.changed();
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
    ///
    /// Runs every tick on every session, so it walks the pack once into
    /// a set rather than searching it once per entry.
    pub fn forget_gone(&mut self, held: &[u32]) -> usize {
        let held: BTreeSet<u32> = held.iter().copied().collect();
        let before = self.took.len();
        self.took.retain(|guid, _| held.contains(guid));
        let gone = before - self.took.len();
        if gone > 0 {
            self.changed();
        }
        gone
    }

    pub fn forget(&mut self, guid: u32) {
        if self.took.remove(&guid).is_some() {
            self.changed();
        }
    }

    /// Two stacks are about to be poured together: settle what the one
    /// that survives was taken for.
    ///
    /// The halves can honestly disagree -- a rule that keeps up to a
    /// cap and a later rule that sells the rest will tag two stacks of
    /// the same thing differently -- and only one answer survives the
    /// merge. The cautious one wins ([`LootAction::safer_of`]), because
    /// a stack still in the pack can be sold tomorrow and a sold one
    /// cannot be got back.
    ///
    /// Which is also why the client never chooses such a pour: the
    /// cautious answer is a Sell overruled, and the tidy that ran
    /// before every sale was quietly settling the player's loot as
    /// kept. The pack tidy and the counter's compress both leave two
    /// stacks apart when their words differ, so this is reached only
    /// with agreeing entries, or after a pour somebody else made.
    ///
    /// Called once the pour has landed, while both entries still exist
    /// -- the source's is left for [`Ledger::forget_gone`] to clear when
    /// the server confirms the object is gone. Called when the merge
    /// went out instead, a pour the server refused still settled the
    /// target: a stack meant for a counter became one to keep, and was
    /// never sold.
    pub fn merged(&mut self, from: u32, to: u32) {
        let Some(source) = self.took.get(&from).map(|t| t.action) else {
            return;
        };
        if let Some(target) = self.took.get_mut(&to) {
            let settled = target.action.safer_of(source);
            if settled != target.action {
                target.action = settled;
                self.changed();
            }
        }
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

    /// Read one back.
    ///
    /// No file is an empty ledger: a character that has never written
    /// anything down has nothing to lose. A file that will not read is
    /// something else, and says so ([`Ledger::trusted`]), because its
    /// decisions are gone rather than absent.
    pub fn load(path: &Path) -> Ledger {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Ledger::default();
        };
        match serde_json::from_str::<Ledger>(&text) {
            Ok(l) => l,
            Err(e) => {
                tracing::error!(
                    path = %path.display(),
                    "the loot ledger will not read ({e}); nothing will be sold until it is \
                     sorted out, because what this character meant to keep is in there"
                );
                Ledger {
                    unreadable: true,
                    ..Default::default()
                }
            }
        }
    }

    /// Whether what it says can be acted on.
    ///
    /// False when a file was there and would not read. Nothing is sold
    /// while this is false: the cost of waiting is a full pack, and the
    /// cost of guessing is somebody's armour.
    pub fn trusted(&self) -> bool {
        !self.unreadable
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
        // Never write over a file that would not read: it is the only
        // copy of what this character meant to keep, and a person can
        // still repair it.
        if !self.dirty || self.unreadable {
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
    fn the_rules_a_decision_was_made_under_are_written_down_with_it() {
        let mut l = Ledger::new();
        // Unknown, not "no rules": a file from before this was recorded
        // must not read as though it were judged under nothing.
        assert_eq!(l.rules(), None);
        l.judged_under(42);
        assert_eq!(l.rules(), Some(42));
        assert!(l.unsaved(), "it goes to the disk with the decisions");
        // Recording the same rules again changes nothing.
        let was = l.version();
        l.judged_under(42);
        assert_eq!(l.version(), was);
        l.judged_under(43);
        assert_ne!(l.version(), was);
        // And it survives the trip through the file.
        let text = serde_json::to_string(&l).unwrap();
        let back: Ledger = serde_json::from_str(&text).unwrap();
        assert_eq!(back.rules(), Some(43));
        // A file written before the field existed still reads.
        let old: Ledger =
            serde_json::from_str(r#"{"took":{"1":{"action":"sell","wcid":9,"name":"Nail"}}}"#)
                .unwrap();
        assert_eq!(old.rules(), None);
        assert_eq!(old.len(), 1);
    }

    #[test]
    fn a_stack_is_written_down_like_anything_else() {
        // What a stack is for is worth recording even though its kind
        // would settle it again: a search across every character's
        // packs shows it, and that is most of the point of a ledger
        // anyone can read.
        let mut l = Ledger::new();
        l.remember(&pile(7, 691, "Prismatic Taper"), LootAction::Keep);
        assert_eq!(
            l.of(&pile(7, 691, "Prismatic Taper")),
            Some(LootAction::Keep)
        );
        // And a recycled id is caught here exactly as it is for a ring.
        assert_eq!(l.of(&pile(7, 999, "Lead Scarab")), None);
    }

    #[test]
    fn pouring_two_stacks_together_keeps_the_cautious_answer() {
        // A rule that keeps up to a cap and a later rule that sells the
        // rest tag two stacks of the same thing differently. Only one
        // answer survives the merge, and it is the one that does not
        // give anything away.
        let mut l = Ledger::new();
        l.remember(&pile(1, 691, "Prismatic Taper"), LootAction::Sell);
        l.remember(&pile(2, 691, "Prismatic Taper"), LootAction::Keep);
        // Either way round, and whichever stack survives.
        l.merged(2, 1);
        assert_eq!(
            l.of(&pile(1, 691, "Prismatic Taper")),
            Some(LootAction::Keep)
        );
        let mut l = Ledger::new();
        l.remember(&pile(1, 691, "Prismatic Taper"), LootAction::Keep);
        l.remember(&pile(2, 691, "Prismatic Taper"), LootAction::Sell);
        l.merged(2, 1);
        assert_eq!(
            l.of(&pile(1, 691, "Prismatic Taper")),
            Some(LootAction::Keep)
        );
        // Salvage outranks sale, and an unknown source changes nothing.
        let mut l = Ledger::new();
        l.remember(&pile(1, 691, "Prismatic Taper"), LootAction::Sell);
        l.remember(&pile(2, 691, "Prismatic Taper"), LootAction::Salvage);
        l.merged(2, 1);
        assert_eq!(
            l.of(&pile(1, 691, "Prismatic Taper")),
            Some(LootAction::Salvage)
        );
        l.merged(99, 1);
        assert_eq!(
            l.of(&pile(1, 691, "Prismatic Taper")),
            Some(LootAction::Salvage)
        );
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

    fn temp_dir(what: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("acswarm-ledger-{what}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_ledger_is_read_back_from_the_file_it_was_written_to() {
        // The restart itself, not a round trip through a string in
        // memory: `save`, `load` and the path between them are what make
        // a decision survive a crash, and nothing ran them.
        let dir = temp_dir("restart");
        let path = Ledger::path_of(&dir, "play.coldeve.ac", "+Blargerton");
        let mut l = Ledger::new();
        l.remember(&thing(1, 500, "Ornate Ring"), LootAction::Keep);
        l.remember(&thing(2, 501, "Jewel"), LootAction::Sell);
        assert!(l.unsaved(), "there is something to write");
        l.save_if_changed(&path);
        assert!(!l.unsaved(), "and it has been written");
        assert!(path.is_file(), "at {}", path.display());

        let back = Ledger::load(&path);
        assert!(back.trusted());
        assert_eq!(
            back.of(&thing(1, 500, "Ornate Ring")),
            Some(LootAction::Keep),
            "the keeper is still a keeper after a restart"
        );
        assert_eq!(back.for_sale(0), vec![2]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_ledger_written_before_failures_carried_an_hour_still_reads() {
        // The first shape of `failed` was a bare string. A character
        // running since then has one on disk, and a file that will not
        // parse loses every decision it holds.
        let text = r#"{"took":{"1":{"action":"salvage","wcid":700,"name":"Greaves","failed":"could not be salvaged"}}}"#;
        let l: Ledger = serde_json::from_str(text).expect("the old shape still reads");
        let greaves = thing(1, 700, "Greaves");
        assert_eq!(l.of(&greaves), Some(LootAction::Salvage));
        // With no hour in it, it is taken as long past rather than as
        // having just happened -- against a real clock, which is the
        // only one this is ever asked with.
        let now = 1_700_000_000;
        assert_eq!(l.for_salvage(now), vec![1], "not held against it");
    }

    #[test]
    fn a_ledger_that_will_not_read_sells_nothing_rather_than_guessing() {
        // An empty ledger and a lost one are not the same thing. Empty
        // means "nothing decided", every item falls through to the
        // rules, and a profile with a broad sell rule sells the keepers.
        // Lost has to say so.
        let dir = temp_dir("broken");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("nonsense.json");
        std::fs::write(&path, "{ this is not json").unwrap();

        let mut l = Ledger::load(&path);
        assert!(!l.trusted(), "it knows its decisions are gone");
        assert!(
            Ledger::load(&dir.join("never-written.json")).trusted(),
            "no file at all is an honest empty one"
        );

        // And it does not write over the only copy, which a person may
        // still be able to repair.
        l.remember(&thing(1, 500, "Ornate Ring"), LootAction::Keep);
        l.save_if_changed(&path);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{ this is not json",
            "the unreadable file is left alone"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_ledger_belongs_to_one_character_on_one_world() {
        let dir = std::path::Path::new("/c");
        let a = Ledger::path_of(dir, "play.coldeve.ac", "Blargerton");
        assert_ne!(
            a,
            Ledger::path_of(dir, "127.0.0.1", "Blargerton"),
            "two worlds"
        );
        assert_ne!(
            a,
            Ledger::path_of(dir, "play.coldeve.ac", "Bryn"),
            "two characters"
        );
        // A name is a file name, not a path: one that climbed out would
        // put one character's decisions where another reads them.
        let climber = Ledger::path_of(dir, "play.coldeve.ac", "../Bryn");
        assert!(
            climber.starts_with("/c/taken/play_coldeve_ac"),
            "{}",
            climber.display()
        );
    }

    #[test]
    fn one_ledger_per_server_and_character() {
        let dir = Path::new("/tmp/x");
        let a = Ledger::path_of(dir, "play.coldeve.ac", "Blargerton");
        let b = Ledger::path_of(dir, "127.0.0.1", "Blargerton");
        assert_ne!(a, b, "two worlds share no ids");
    }
}
