//! What each item was picked up for ([`Ledger`]): decided once, when taken, and final, since a
//! second judgement meets a changed pack, a filled cap or an appraisal that has landed.
//! Keyed by guid, which ACE frees only on destroy (WorldObject.cs:895), so it survives relog, crash
//! and restart; a freed one returns 360 min later (GuidManager.cs:126) or, after a restart, out of
//! the sequence gaps, which [`Ledger::of`] and [`Ledger::forget_gone`] are there to cover.

use crate::items::ItemStats;
use crate::profile::LootAction;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// What one item was taken for, and enough about it to know it again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Took {
    pub action: LootAction,
    /// Weenie class when the decision was made: a recycled guid on another kind will not match.
    pub wcid: u32,
    /// For the log and the panel: which ring it was.
    #[serde(default)]
    pub name: String,
    /// Why what was decided could not be carried out; the decision itself stands, unchanged.
    /// A wait, not a grudge (`docs/agent.md`): it lapses after [`TRY_AGAIN_AFTER`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<Failed>,
}

/// What went wrong, and when, so that it stops mattering.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Failed {
    pub why: String,
    /// Unix seconds, not an `Instant`: it outlives the process that wrote it.
    pub at: u64,
}

impl<'de> Deserialize<'de> for Failed {
    /// Reads both shapes on disk: `{why, at}` and the older bare string.
    /// A ledger that will not parse loses every decision it holds, so an old file must still read.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Either {
            Now { why: String, at: u64 },
            Before(String),
        }
        Ok(match Either::deserialize(d)? {
            Either::Now { why, at } => Failed { why, at },
            // No hour: taken as long past, the forgiving side for what is only a wait.
            Either::Before(why) => Failed { why, at: 0 },
        })
    }
}

/// Seconds a failure is held against an item (guess): past an afternoon of retries, short of tomorrow.
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
    /// Bumped on every change, so the holdings snapshot goes stale when a decision does.
    /// Not part of the file: a ledger read back starts at zero, having been compared with nothing.
    #[serde(skip)]
    version: u64,
    /// Fingerprint of the rules these decisions were made under, saved with them so a profile
    /// edited while the client was off is noticed. `None` reads as unknown: left alone, not judged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rules: Option<u64>,
    /// A file was there and would not read, which is not the same as nothing written down.
    /// Empty sends every item to the rules, where a broad sell rule would sell the lost keepers.
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

    /// The rules fingerprint, or `None` when it was never recorded.
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
    /// A split stack is a new object with a new id: its half arrives with no entry, judged afresh.
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

    /// What this item was taken for, when an entry for its guid still describes its kind.
    pub fn of(&self, item: &ItemStats) -> Option<LootAction> {
        let took = self.took.get(&item.guid)?;
        // A recycled id wearing someone else's clothes.
        (took.wcid == item.wcid).then_some(took.action)
    }

    /// [`Ledger::of`] for a caller holding only an id; prefer that, as this cannot spot a recycled one.
    pub fn by_guid(&self, guid: u32) -> Option<LootAction> {
        self.took.get(&guid).map(|t| t.action)
    }

    /// Record that the decision could not be carried out, `now` in Unix seconds; it still stands.
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

    /// Forget every entry whose item has left the pack, so a recycled id finds none to misapply.
    /// Runs every tick on every session, hence one pass of the pack into a set.
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

    /// Settle what the surviving stack `to` was taken for: the cautious tag wins
    /// ([`LootAction::safer_of`]), since an unsold stack can still sell tomorrow.
    /// The client's own tidy and compress leave stacks whose tags differ apart, so the answer here
    /// only overrules a Sell after somebody else's pour.
    /// Call once the pour has landed, while both entries exist: a refused pour must settle nothing.
    /// `from`'s entry is left for [`Ledger::forget_gone`], once the server confirms it is gone.
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

    /// Guids tagged Sell, less any refused inside [`TRY_AGAIN_AFTER`]; `now` in Unix seconds.
    pub fn for_sale(&self, now: u64) -> Vec<u32> {
        self.with(LootAction::Sell, now)
    }

    /// The same for the salvage bag.
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

    /// Every decision by guid, for a reader that wants a plain map.
    pub fn actions(&self) -> BTreeMap<u32, LootAction> {
        self.took.iter().map(|(g, t)| (*g, t.action)).collect()
    }

    pub fn len(&self) -> usize {
        self.took.len()
    }

    pub fn is_empty(&self) -> bool {
        self.took.is_empty()
    }

    /// Where one character's ledger lives: per server and character, as guids are a server's own.
    pub fn path_of(dir: &Path, server: &str, character: &str) -> PathBuf {
        // Not `ac_store::file_safe`: ASCII letters and digits only, and renaming these directories
        // now would hide the ledgers players already have -- a ledger read as empty sells keepers.
        let tidy = |s: &str| {
            s.chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect::<String>()
        };
        dir.join("taken")
            .join(tidy(server))
            .join(format!("{}.json", tidy(character)))
    }

    /// Read one back. No file is an honest empty ledger; a file that will not read says so
    /// ([`Ledger::trusted`]), its decisions being lost rather than absent.
    pub fn load(path: &Path) -> Ledger {
        match ac_store::read_json::<Ledger>(path) {
            Ok(Some(l)) => l,
            Ok(None) => Ledger::default(),
            // Only a file that opens and will not parse says its decisions are lost; one that will
            // not open at all reads as none.
            Err(e) if e.kind() != std::io::ErrorKind::InvalidData => Ledger::default(),
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

    /// Whether what it says can be acted on; false when a file was there and would not read.
    /// Nothing is sold while it is false: waiting costs a full pack, guessing somebody's armour.
    pub fn trusted(&self) -> bool {
        !self.unreadable
    }

    /// Write it out, on every change rather than at exit: a crash is what it defends against.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).unwrap_or_default();
        ac_store::write_atomic(path, text.as_bytes(), ac_store::Visibility::Normal)
    }

    /// Write it out if anything changed. Called every tick, so it costs a bool most of the time.
    pub fn save_if_changed(&mut self, path: &Path) {
        // Never write over a file that would not read: it is the only copy, and can be repaired.
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
        // The id was freed when the ring was sold and handed to a sword 6 h later; the entry must
        // not decide the sword's fate.
        let mut l = Ledger::new();
        l.remember(&thing(1, 500, "Ornate Ring"), LootAction::Sell);
        let sword = thing(1, 999, "Shou-jen Sword");
        assert_eq!(l.of(&sword), None);
    }

    #[test]
    fn the_rules_a_decision_was_made_under_are_written_down_with_it() {
        let mut l = Ledger::new();
        // Unknown, not "no rules": an older file must not read as judged under nothing.
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
        // A stack's kind would settle it again, but only the entry shows a holdings search across
        // every character's packs what it is for.
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
        // A cap rule and a later sell rule tag two stacks of one kind differently; the answer that
        // gives nothing away survives the merge.
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
        // A counter that will not take it does not make it a keeper: the next counter may.
        let mut l = Ledger::new();
        let ring = thing(1, 500, "Ornate Ring");
        let now = 1_000_000;
        l.remember(&ring, LootAction::Sell);
        l.failed(1, "no vendor will take it", now);
        assert_eq!(l.of(&ring), Some(LootAction::Sell), "still meant to go");
        assert!(l.for_sale(now).is_empty(), "but not offered again now");
        assert_eq!(l.why_failed(1, now), Some("no vendor will take it"));
        // A wait, not a grudge: a session runs for days.
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
        // The restart itself, not a round trip through a string: `save`, `load` and `path_of` are
        // what make a decision survive a crash.
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
        // The older `failed` on disk is a bare string, and a file that will not parse loses every
        // decision it holds.
        let text = r#"{"took":{"1":{"action":"salvage","wcid":700,"name":"Greaves","failed":"could not be salvaged"}}}"#;
        let l: Ledger = serde_json::from_str(text).expect("the old shape still reads");
        let greaves = thing(1, 700, "Greaves");
        assert_eq!(l.of(&greaves), Some(LootAction::Salvage));
        // With no hour it reads as long past, against the real clock this is only ever asked with.
        let now = 1_700_000_000;
        assert_eq!(l.for_salvage(now), vec![1], "not held against it");
    }

    #[test]
    fn a_ledger_that_will_not_read_sells_nothing_rather_than_guessing() {
        // Empty means "nothing decided", so every item falls to the rules and a broad sell rule
        // sells the keepers. A lost ledger has to say so instead.
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

        // Nor does it write over the only copy, which a person may still repair.
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
        // A name is a file name, not a path: one that climbed out would put a character's decisions
        // where another reads them.
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
