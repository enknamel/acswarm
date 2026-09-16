//! Every character's inventory, on every account, in one place: the
//! "Items" window and the `holdings_search` call look here to find which
//! character holds what.
//!
//! A session takes a [`CharacterHoldings`] snapshot of its own items
//! ([`Client::holdings_snapshot`]) and hands it to the process-wide
//! [`HoldingsStore`] ([`store`]); the plugin that drives this
//! (`ac_plugin::panels::holdings`) also publishes it on the bus as a
//! `set` under [`bus_key`] so every other process gets it, and writes it
//! to `<cache dir>/holdings/<server>/<account>/<character>.json` so a
//! character that is not logged in stays searchable. A store merges what
//! it hears by `taken_at`: the newer snapshot wins.
//!
//! A character is named by **server, account and character**, all three.
//! The account alone is not enough -- the same account holds characters
//! on more than one world, and two worlds can hold the same name -- and
//! searching across worlds would be answering a question nobody can act
//! on, because there is no way to move an item between them. So every
//! search and every view is scoped to the world the asking character is
//! logged in to (see [`HoldingsStore::on_server`]).
//!
//! An item in a snapshot is a [`HoldingRecord`], the owned, serialisable
//! twin of [`ItemStats`]; [`HoldingRecord::to_stats`] turns it back so
//! the same [`Query`] that searches the inventory searches every
//! character.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::items::{kind_name, ItemStats, Query};
use crate::Client;

/// Bus keys a snapshot is `set` under start with this.
pub const BUS_PREFIX: &str = "holdings.";
/// The bus topic a window posts on to ask every online session to
/// appraise its unappraised items and publish again. The value is
/// `{"all": true}` or `{"account": .., "character": ..}` for one.
pub const REQUEST_TOPIC: &str = "holdings.request";
/// A snapshot that says `online` counts as online for this long after it
/// was taken: sessions publish at least this often while they live, so
/// a process that died without saying goodbye fades out.
pub const ONLINE_FOR: u64 = 90;

/// The bus key of one character's snapshot:
/// `holdings.<server>/<account>/<character>`.
pub fn bus_key(server: &str, account: &str, character: &str) -> String {
    format!(
        "{BUS_PREFIX}{}/{}/{character}",
        ac_store::file_safe(&server.to_ascii_lowercase()),
        account.to_ascii_lowercase()
    )
}

/// The server, account and character a bus key names, if it is one of
/// ours. A key from before worlds were told apart has two parts, and is
/// read as a character on no particular world -- which no scoped search
/// will match, and which its own session replaces on its next login.
pub fn parse_bus_key(key: &str) -> Option<(&str, &str, &str)> {
    let rest = key.strip_prefix(BUS_PREFIX)?;
    let mut parts = rest.split('/');
    let a = parts.next()?;
    let b = parts.next()?;
    match parts.next() {
        Some(character) => Some((a, b, character)),
        None => Some(("", a, b)),
    }
}

/// What names one character for good: the world, the account and the
/// name. Two of the three is not enough -- one account plays several
/// worlds, and two worlds can hold the same name.
pub type Key = (String, String, String);

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}
fn is_zero_f32(v: &f32) -> bool {
    *v == 0.0
}
fn is_zero_f64(v: &f64) -> bool {
    *v == 0.0
}
fn is_false(v: &bool) -> bool {
    !*v
}

/// One item as kept in a snapshot: everything [`ItemStats`] holds, owned.
/// Fields at their default are left out of the JSON, so an unappraised
/// item is a short line.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HoldingRecord {
    pub guid: u32,
    pub name: String,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub wcid: u32,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub item_type: u32,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub stack: u32,
    #[serde(skip_serializing_if = "is_false")]
    pub wielded: bool,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub valid_locations: u32,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub container: u32,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub value: u32,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub burden: u32,
    #[serde(skip_serializing_if = "is_zero_f32")]
    pub workmanship: f32,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub material: String,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub structure: u32,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub max_structure: u32,
    #[serde(skip_serializing_if = "is_false")]
    pub appraised: bool,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub damage_low: u32,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub damage_high: u32,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub damage_type: String,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub damage_type_bits: u32,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub imbued: u32,
    #[serde(skip_serializing_if = "is_zero_f32")]
    pub elemental_damage: f32,
    #[serde(skip_serializing_if = "is_zero_f32")]
    pub crit_frequency: f32,
    #[serde(skip_serializing_if = "is_zero_f32")]
    pub crit_multiplier: f32,
    #[serde(skip_serializing_if = "is_zero_f32")]
    pub damage_mod: f32,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub ammo_type: u32,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub combat_use: u32,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub speed: u32,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub weapon_skill: String,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub weapon_skill_id: u32,
    #[serde(skip_serializing_if = "is_zero_f64")]
    pub attack_bonus: f64,
    #[serde(skip_serializing_if = "is_zero_f64")]
    pub defense_bonus: f64,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub armor_level: u32,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub shield: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub spells: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub wield_skill: String,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub wield_level: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub wield_reqs: Vec<(u32, u32, u32)>,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub mana: u32,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub max_mana: u32,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub spellcraft: u32,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub tinks: u32,
    #[serde(skip_serializing_if = "is_false")]
    pub bonded: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub attuned: bool,
    /// What this was picked up for, when the character that holds it
    /// wrote it down (`ac_loot::Ledger`). Absent for anything bought,
    /// traded, or in the pack from before there was a profile.
    ///
    /// It travels with the snapshot so the Items window can answer
    /// "what is my mule carrying, and what is it carrying it *for*"
    /// without that character being logged in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub took: Option<ac_loot::LootAction>,
}

/// Every word [`kind_name`] can answer with, so a stored kind maps back
/// to the same `&'static str` without leaking anything.
const KINDS: &[&str] = &[
    "weapon",
    "missile",
    "caster",
    "armor",
    "clothing",
    "jewelry",
    "pack",
    "food",
    "money",
    "gem",
    "key",
    "manastone",
    "comps",
    "writable",
    "portal",
    "salvage",
    "tool",
    "note",
    "service",
    "lifestone",
    "gameboard",
    "trinket",
    "misc",
    "junk",
];

/// The static kind word for a stored one: the table's own word when it
/// is one, else what the item type says.
pub fn kind_static(kind: &str, item_type: u32) -> &'static str {
    KINDS
        .iter()
        .copied()
        .find(|k| *k == kind)
        .unwrap_or_else(|| kind_name(item_type))
}

/// The static material name for a stored one, by looking through the
/// material table; `""` for none and `"?"` for a name it lacks.
pub fn material_static(material: &str) -> &'static str {
    if material.is_empty() {
        return "";
    }
    (1..=0x100u32)
        .map(ac_world::material::name)
        .find(|m| *m == material)
        .unwrap_or("?")
}

impl From<&ItemStats> for HoldingRecord {
    fn from(s: &ItemStats) -> Self {
        HoldingRecord {
            guid: s.guid,
            name: s.name.clone(),
            wcid: s.wcid,
            item_type: s.item_type,
            kind: s.kind.to_string(),
            stack: s.stack,
            wielded: s.wielded,
            valid_locations: s.valid_locations,
            container: s.container,
            value: s.value,
            burden: s.burden,
            workmanship: s.workmanship,
            material: s.material.to_string(),
            structure: s.structure,
            max_structure: s.max_structure,
            appraised: s.appraised,
            damage_low: s.damage_low,
            damage_high: s.damage_high,
            damage_type: s.damage_type.clone(),
            damage_type_bits: s.damage_type_bits,
            imbued: s.imbued,
            elemental_damage: s.elemental_damage,
            crit_frequency: s.crit_frequency,
            crit_multiplier: s.crit_multiplier,
            damage_mod: s.damage_mod,
            ammo_type: s.ammo_type,
            combat_use: s.combat_use,
            speed: s.speed,
            weapon_skill: s.weapon_skill.clone(),
            weapon_skill_id: s.weapon_skill_id,
            attack_bonus: s.attack_bonus,
            defense_bonus: s.defense_bonus,
            armor_level: s.armor_level,
            shield: s.shield,
            spells: s.spells.clone(),
            wield_skill: s.wield_skill.clone(),
            wield_level: s.wield_level,
            wield_reqs: s.wield_reqs.clone(),
            mana: s.mana,
            max_mana: s.max_mana,
            spellcraft: s.spellcraft,
            tinks: s.tinks,
            bonded: s.bonded,
            attuned: s.attuned,
            took: None,
        }
    }
}

impl HoldingRecord {
    /// Back to the searchable form. Fields `ItemStats` grows that a
    /// record does not keep take their default, hence the update
    /// syntax even while there are none.
    #[allow(clippy::needless_update)]
    pub fn to_stats(&self) -> ItemStats {
        ItemStats {
            guid: self.guid,
            name: self.name.clone(),
            wcid: self.wcid,
            item_type: self.item_type,
            kind: kind_static(&self.kind, self.item_type),
            stack: self.stack,
            wielded: self.wielded,
            valid_locations: self.valid_locations,
            container: self.container,
            value: self.value,
            burden: self.burden,
            workmanship: self.workmanship,
            material: material_static(&self.material),
            structure: self.structure,
            max_structure: self.max_structure,
            appraised: self.appraised,
            damage_low: self.damage_low,
            damage_high: self.damage_high,
            damage_type: self.damage_type.clone(),
            damage_type_bits: self.damage_type_bits,
            imbued: self.imbued,
            elemental_damage: self.elemental_damage,
            crit_frequency: self.crit_frequency,
            crit_multiplier: self.crit_multiplier,
            damage_mod: self.damage_mod,
            ammo_type: self.ammo_type,
            combat_use: self.combat_use,
            speed: self.speed,
            weapon_skill: self.weapon_skill.clone(),
            weapon_skill_id: self.weapon_skill_id,
            attack_bonus: self.attack_bonus,
            defense_bonus: self.defense_bonus,
            armor_level: self.armor_level,
            shield: self.shield,
            spells: self.spells.clone(),
            wield_skill: self.wield_skill.clone(),
            wield_level: self.wield_level,
            wield_reqs: self.wield_reqs.clone(),
            mana: self.mana,
            max_mana: self.max_mana,
            spellcraft: self.spellcraft,
            tinks: self.tinks,
            bonded: self.bonded,
            attuned: self.attuned,
            ..Default::default()
        }
    }
}

/// One character's items as last seen.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CharacterHoldings {
    /// The world it is on: `Config::host`. Empty in a file written
    /// before worlds were told apart.
    #[serde(default)]
    pub server: String,
    pub account: String,
    pub character: String,
    /// The character's guid, 0 when unknown.
    pub guid: u32,
    /// When the snapshot was taken, unix seconds.
    pub taken_at: u64,
    /// The session was in the world when it was taken (see
    /// [`CharacterHoldings::online_at`]).
    pub online: bool,
    pub items: Vec<HoldingRecord>,
}

/// Unix seconds now.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl CharacterHoldings {
    /// The store's key: the account in lower case, the character as is.
    pub fn key(&self) -> Key {
        (
            self.server.to_ascii_lowercase(),
            self.account.to_ascii_lowercase(),
            self.character.clone(),
        )
    }

    pub fn bus_key(&self) -> String {
        bus_key(&self.server, &self.account, &self.character)
    }

    /// Whether this character is on `server`, case-insensitively.
    pub fn on(&self, server: &str) -> bool {
        self.server.eq_ignore_ascii_case(server)
    }

    /// Online as of `now` (unix seconds): said so, and said so recently
    /// enough ([`ONLINE_FOR`]).
    pub fn online_at(&self, now: u64) -> bool {
        self.online && now.saturating_sub(self.taken_at) < ONLINE_FOR
    }

    pub fn unappraised(&self) -> usize {
        self.items.iter().filter(|i| !i.appraised).count()
    }

    /// Where an item sits, in words: "worn", "pack" (the main pack) or
    /// the side pack's name.
    pub fn place_of(&self, item: &HoldingRecord) -> String {
        if item.wielded {
            return "worn".into();
        }
        if item.container == 0 || item.container == self.guid {
            return "pack".into();
        }
        match self.items.iter().find(|i| i.guid == item.container) {
            Some(pack) => pack.name.clone(),
            None => "pack".into(),
        }
    }
}

/// One search result: who holds the item, and the item.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub server: String,
    pub account: String,
    pub character: String,
    pub online: bool,
    pub taken_at: u64,
    /// "worn", "pack", or a side pack's name.
    pub place: String,
    pub stats: ItemStats,
    /// What the holder wrote down that this was picked up for
    /// (`HoldingRecord::took`). `None` for anything no rule claimed, or
    /// bought or traded before a rule could.
    pub took: Option<ac_loot::LootAction>,
}

/// Every snapshot known to this process, by (account, character).
#[derive(Debug, Default)]
pub struct HoldingsStore {
    chars: BTreeMap<Key, CharacterHoldings>,
    /// Where snapshots are written; `None` keeps them in memory only.
    dir: Option<PathBuf>,
}

impl HoldingsStore {
    /// An empty store that writes nothing.
    pub const fn new() -> Self {
        HoldingsStore {
            chars: BTreeMap::new(),
            dir: None,
        }
    }

    /// The default directory: `<cache dir>/holdings`, the cache dir
    /// being `$ACSWARM_CACHE_DIR` or `~/.cache/acswarm`.
    pub fn default_dir() -> PathBuf {
        ac_scene::worldgrid::WorldGrid::cache_dir().join("holdings")
    }

    /// Read every `<dir>/<account>/<character>.json` into a store that
    /// writes back to `dir`.
    pub fn load_all(dir: &Path) -> Self {
        let mut store = HoldingsStore::new();
        store.dir = Some(dir.to_path_buf());
        store.load_dir(dir);
        store
    }

    /// Where snapshots are written, once set.
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    pub fn set_dir(&mut self, dir: Option<PathBuf>) {
        self.dir = dir;
    }

    /// Merge every snapshot file under `dir`; returns how many were read.
    /// Read every snapshot under `dir` into a store.
    ///
    /// Files live at `<server>/<account>/<character>.json`. Files from
    /// before worlds were told apart are two deep instead of three;
    /// they are read, so nothing disappears, but they carry no world
    /// and so match no scoped search. Each is replaced by its own
    /// session on its next login, which is why this is not worth a
    /// migration.
    pub fn load_dir(&mut self, dir: &Path) -> usize {
        let mut n = 0;
        let mut orphans = 0;
        let mut read = |store: &mut Self, path: &Path| {
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                return;
            }
            match ac_store::read_json::<CharacterHoldings>(path) {
                Ok(Some(h)) => {
                    if h.server.is_empty() {
                        orphans += 1;
                    }
                    store.merge(h);
                    n += 1;
                }
                // Gone between listing the directory and reading it.
                Ok(None) => {}
                Err(e) => tracing::warn!("holdings: cannot read {}: {e}", path.display()),
            }
        };
        let Ok(top) = std::fs::read_dir(dir) else {
            return 0;
        };
        for entry in top.flatten() {
            let Ok(mid) = std::fs::read_dir(entry.path()) else {
                continue;
            };
            for m in mid.flatten() {
                let path = m.path();
                if path.is_dir() {
                    // <server>/<account>/<character>.json
                    if let Ok(files) = std::fs::read_dir(&path) {
                        for f in files.flatten() {
                            read(self, &f.path());
                        }
                    }
                } else {
                    // <account>/<character>.json, from before worlds
                    // were told apart.
                    read(self, &path);
                }
            }
        }
        if orphans > 0 {
            tracing::info!(
                "holdings: {orphans} snapshot(s) name no world; each is replaced on its \
                 character's next login"
            );
        }
        n
    }

    /// The file a character's snapshot lives in under `dir`.
    pub fn path_for(dir: &Path, server: &str, account: &str, character: &str) -> PathBuf {
        dir.join(ac_store::file_safe(&server.to_ascii_lowercase()))
            .join(ac_store::file_safe(&account.to_ascii_lowercase()))
            .join(format!("{}.json", ac_store::file_safe(character)))
    }

    /// Write one snapshot under `dir`.
    pub fn save_to(dir: &Path, h: &CharacterHoldings) -> std::io::Result<PathBuf> {
        let path = Self::path_for(dir, &h.server, &h.account, &h.character);
        let text = serde_json::to_string(h).map_err(std::io::Error::other)?;
        ac_store::write_atomic(&path, text.as_bytes(), ac_store::Visibility::Normal)?;
        Ok(path)
    }

    /// Write one snapshot to the store's directory (nothing without one).
    pub fn save(&self, h: &CharacterHoldings) -> std::io::Result<Option<PathBuf>> {
        match &self.dir {
            Some(dir) => Self::save_to(dir, h).map(Some),
            None => Ok(None),
        }
    }

    /// Take a snapshot in unless one at least as new is already held.
    /// Returns whether it was taken.
    pub fn merge(&mut self, h: CharacterHoldings) -> bool {
        if h.character.is_empty() {
            return false;
        }
        let key = h.key();
        if self
            .chars
            .get(&key)
            .is_some_and(|have| have.taken_at > h.taken_at)
        {
            return false;
        }
        self.chars.insert(key, h);
        true
    }

    /// Take in a bus value (the JSON of a [`CharacterHoldings`]).
    pub fn merge_value(&mut self, value: &Value) -> bool {
        match serde_json::from_value::<CharacterHoldings>(value.clone()) {
            Ok(h) => self.merge(h),
            Err(_) => false,
        }
    }

    /// Put the snapshot in whatever its age (this session's own word).
    pub fn put(&mut self, h: CharacterHoldings) {
        if !h.character.is_empty() {
            self.chars.insert(h.key(), h);
        }
    }

    pub fn get(&self, server: &str, account: &str, character: &str) -> Option<&CharacterHoldings> {
        self.chars.get(&(
            server.to_ascii_lowercase(),
            account.to_ascii_lowercase(),
            character.to_string(),
        ))
    }

    /// Mark a character offline as of now; the snapshot as it is now.
    pub fn set_offline(
        &mut self,
        server: &str,
        account: &str,
        character: &str,
    ) -> Option<CharacterHoldings> {
        let h = self.chars.get_mut(&(
            server.to_ascii_lowercase(),
            account.to_ascii_lowercase(),
            character.to_string(),
        ))?;
        h.online = false;
        h.taken_at = h.taken_at.max(unix_now());
        Some(h.clone())
    }

    pub fn iter(&self) -> impl Iterator<Item = &CharacterHoldings> {
        self.chars.values()
    }

    pub fn len(&self) -> usize {
        self.chars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// Characters, items and unappraised items over the whole store.
    /// Characters, items and unappraised items on one world.
    pub fn counts(&self, server: &str) -> (usize, usize, usize) {
        let items = self.on_server(server).map(|h| h.items.len()).sum();
        let unappraised = self.on_server(server).map(|h| h.unappraised()).sum();
        (self.on_server(server).count(), items, unappraised)
    }

    /// Every character known on one world. Nothing from another world: there is no way to move an
    /// item between them, so an answer from one would be a fact nobody
    /// can act on.
    pub fn on_server<'a>(&'a self, server: &'a str) -> impl Iterator<Item = &'a CharacterHoldings> {
        self.chars.values().filter(move |h| h.on(server))
    }

    /// What matches `q` among the characters on `server`.
    pub fn search(&self, server: &str, q: &Query, now: u64) -> Vec<Hit> {
        let mut out = Vec::new();
        for h in self.on_server(server) {
            let online = h.online_at(now);
            for item in &h.items {
                let stats = item.to_stats();
                if stats.matches(q) {
                    out.push(Hit {
                        server: h.server.clone(),
                        account: h.account.clone(),
                        character: h.character.clone(),
                        online,
                        taken_at: h.taken_at,
                        place: h.place_of(item),
                        stats,
                        took: item.took,
                    });
                }
            }
        }
        out
    }
}

static STORE: Mutex<HoldingsStore> = Mutex::new(HoldingsStore::new());

/// The process-wide store every session publishes into and every window
/// reads. Empty until something loads or merges into it (the holdings
/// plugin reads the cache directory at its first tick).
pub fn store() -> MutexGuard<'static, HoldingsStore> {
    STORE.lock().unwrap_or_else(|e| e.into_inner())
}

impl Client {
    /// This character's items as a snapshot, `None` before the sheet has
    /// arrived (no name to file it under).
    pub fn holdings_snapshot(&self, online: bool) -> Option<CharacterHoldings> {
        let character = self.world.stats.name.clone();
        if character.is_empty() {
            return None;
        }
        Some(CharacterHoldings {
            server: self.config.host.clone(),
            account: self.config.account.clone(),
            character,
            guid: self.world.player_guid.unwrap_or(0),
            taken_at: unix_now(),
            online,
            items: self
                .item_stats()
                .iter()
                .map(|s| {
                    let mut r = HoldingRecord::from(s);
                    r.took = self.autoplay.ledger.of(s);
                    r
                })
                .collect(),
        })
    }

    /// A number that changes when the inventory does: an item added,
    /// removed, moved, wielded, stacked or appraised -- or when what
    /// one of them is *for* changes, since that travels with the
    /// snapshot too and a reader of it would otherwise go on seeing the
    /// old answer. Cheap to take every frame; the publisher snapshots
    /// when it moves.
    pub fn holdings_fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut items: Vec<(u32, u32, u32, u32, bool)> = self
            .world
            .wielded()
            .chain(self.world.inventory())
            .map(|o| {
                (
                    o.guid,
                    o.container.unwrap_or(0),
                    o.wielder.unwrap_or(0),
                    o.stack_size,
                    self.appraisals.contains_key(&o.guid),
                )
            })
            .collect();
        items.sort_unstable();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        items.hash(&mut h);
        self.autoplay.ledger.version().hash(&mut h);
        h.finish()
    }

    /// Search the items of every character on **this** world known to
    /// this process (see [`store`]), with a search line in the
    /// inventory's language.
    ///
    /// This world and no other: there is no way to move an item between
    /// worlds, so a hit on another one is a fact nobody can act on.
    pub fn holdings_search(&self, query: &str) -> Vec<Hit> {
        let q = Query::parse(query);
        store().search(&self.config.host, &q, unix_now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_world::item_type;

    fn sword() -> ItemStats {
        ItemStats {
            guid: 0x8000_0001,
            name: "Fine Sword".into(),
            wcid: 21,
            item_type: item_type::MELEE_WEAPON,
            kind: "weapon",
            stack: 1,
            wielded: true,
            valid_locations: 0x0010_0000,
            container: 0,
            value: 1200,
            burden: 300,
            workmanship: 6.0,
            material: "Iron",
            appraised: true,
            damage_low: 8,
            damage_high: 14,
            damage_type: "Slashing".into(),
            damage_type_bits: 1,
            speed: 40,
            weapon_skill: "Sword".into(),
            weapon_skill_id: 9,
            attack_bonus: 1.05,
            defense_bonus: 1.0,
            spells: vec!["Blood Drinker IV".into(), "Epic Life Magic Aptitude".into()],
            wield_skill: "Sword".into(),
            wield_level: 250,
            wield_reqs: vec![(2, 9, 250)],
            mana: 500,
            max_mana: 800,
            spellcraft: 300,
            tinks: 2,
            bonded: true,
            ..Default::default()
        }
    }

    fn tunic(container: u32) -> ItemStats {
        ItemStats {
            guid: 0x8000_0002,
            name: "Leather Tunic".into(),
            item_type: item_type::ARMOR,
            kind: "armor",
            stack: 1,
            container,
            value: 300,
            burden: 500,
            ..Default::default()
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("acswarm-holdings-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn records_round_trip_through_json_and_back_to_stats() {
        let s = sword();
        let r = HoldingRecord::from(&s);
        let text = serde_json::to_string(&r).unwrap();
        // Defaults are left out: nothing about armor on a sword.
        assert!(!text.contains("armor_level"), "{text}");
        assert!(text.contains("\"material\":\"Iron\""), "{text}");
        let back: HoldingRecord = serde_json::from_str(&text).unwrap();
        assert_eq!(back, r);
        assert_eq!(back.to_stats(), s);
        // Statics come from the tables, not from leaked strings.
        assert_eq!(kind_static("armor", 0), "armor");
        assert_eq!(kind_static("nonsense", item_type::CASTER), "caster");
        assert_eq!(material_static(""), "");
        assert_eq!(material_static("Teak"), "Teak");
        assert_eq!(material_static("Unobtainium"), "?");
        // Every kind the item table can answer with is in the list.
        for bit in 0..32 {
            let k = kind_name(1 << bit);
            assert!(KINDS.contains(&k), "kind {k} missing from KINDS");
        }
        // An unappraised item is a short line that still reads back.
        let t = HoldingRecord::from(&tunic(0));
        let text = serde_json::to_string(&t).unwrap();
        assert!(text.len() < 160, "{text}");
        assert_eq!(serde_json::from_str::<HoldingRecord>(&text).unwrap(), t);
        // Older or newer files with fields missing still parse.
        let old: HoldingRecord = serde_json::from_str(r#"{"guid":5,"name":"Apple"}"#).unwrap();
        assert_eq!(old.name, "Apple");
        assert_eq!(old.to_stats().kind, "misc");
    }

    #[test]
    fn a_snapshot_written_before_worlds_were_told_apart_is_still_read() {
        // Those files are two deep, not three. Nothing must vanish from
        // the folder when the layout changes -- but they name no world,
        // so they match no scoped search, and each is replaced by its
        // own session on that character's next login.
        let dir = temp_dir("legacy");
        let now = unix_now();
        std::fs::create_dir_all(dir.join("accone")).unwrap();
        let old = CharacterHoldings {
            server: String::new(),
            account: "accone".into(),
            character: "Alice".into(),
            guid: 1,
            taken_at: now,
            online: false,
            items: vec![HoldingRecord {
                guid: 10,
                name: "Old Dagger".into(),
                ..Default::default()
            }],
        };
        std::fs::write(
            dir.join("accone").join("Alice.json"),
            serde_json::to_string(&old).unwrap(),
        )
        .unwrap();

        // And one written since, three deep, beside it.
        let now_shape = CharacterHoldings {
            server: "one.example".into(),
            character: "Bob".into(),
            items: vec![HoldingRecord {
                guid: 11,
                name: "New Dagger".into(),
                ..Default::default()
            }],
            ..old.clone()
        };
        HoldingsStore::save_to(&dir, &now_shape).unwrap();

        let mut store = HoldingsStore::new();
        assert_eq!(store.load_dir(&dir), 2, "both shapes were read");
        assert!(
            store.get("", "accone", "Alice").is_some(),
            "the old one is not lost"
        );
        assert!(store.get("one.example", "accone", "Bob").is_some());

        let q = Query::parse("dagger");
        let hits = store.search("one.example", &q, now);
        assert_eq!(hits.len(), 1, "only the one that names this world");
        assert_eq!(hits[0].character, "Bob");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_world_at_a_time() {
        // Three parts name a character, because one account plays
        // several worlds and two worlds can hold the same name. And
        // nothing crosses: there is no way to move an item between
        // worlds, so a hit from another one is a fact nobody can act on.
        let now = unix_now();
        let here = CharacterHoldings {
            server: "play.coldeve.ac".into(),
            account: "acc".into(),
            character: "Blargerton".into(),
            guid: 1,
            taken_at: now,
            online: false,
            items: vec![HoldingRecord {
                guid: 10,
                name: "Diamond Scarab".into(),
                ..Default::default()
            }],
        };
        // The same account, the same character name, another world.
        let there = CharacterHoldings {
            server: "127.0.0.1".into(),
            items: vec![HoldingRecord {
                guid: 11,
                name: "Diamond Scarab".into(),
                ..Default::default()
            }],
            ..here.clone()
        };
        let mut store = HoldingsStore::new();
        store.put(here.clone());
        store.put(there.clone());
        assert_eq!(store.len(), 2, "the same name on two worlds is two");

        let q = Query::parse("scarab");
        assert_eq!(store.search("play.coldeve.ac", &q, now).len(), 1);
        assert_eq!(store.search("127.0.0.1", &q, now).len(), 1);
        assert_eq!(store.counts("play.coldeve.ac"), (1, 1, 1));
        assert!(
            store.search("nowhere.example", &q, now).is_empty(),
            "a world with nobody on it holds nothing"
        );
        assert!(
            store.get("play.coldeve.ac", "acc", "Blargerton").is_some(),
            "found by all three"
        );
    }

    #[test]
    fn what_a_character_carries_it_for_travels_with_the_snapshot() {
        // So the Items window can answer "what is the mule carrying, and
        // what is it carrying it for" without logging the mule in.
        let mut r = HoldingRecord::from(&sword());
        assert_eq!(r.took, None, "nothing decided");
        r.took = Some(ac_loot::LootAction::Sell);
        let text = serde_json::to_string(&r).unwrap();
        assert!(text.contains("\"took\":\"sell\""), "{text}");
        let back: HoldingRecord = serde_json::from_str(&text).unwrap();
        assert_eq!(back.took, Some(ac_loot::LootAction::Sell));
        // A record written before there were dispositions still reads.
        let old: HoldingRecord = serde_json::from_str(r#"{"guid":5,"name":"Apple"}"#).unwrap();
        assert_eq!(old.took, None);
    }

    #[test]
    fn keys_and_names() {
        assert_eq!(
            bus_key("Play.Coldeve.AC", "FleetBot1", "Fleetbot One"),
            "holdings.play.coldeve.ac/fleetbot1/Fleetbot One"
        );
        assert_eq!(
            parse_bus_key("holdings.play.coldeve.ac/fleetbot1/Fleetbot One"),
            Some(("play.coldeve.ac", "fleetbot1", "Fleetbot One"))
        );
        // A key from before worlds were told apart names no world, and
        // so matches no scoped search.
        assert_eq!(
            parse_bus_key("holdings.fleetbot1/Fleetbot One"),
            Some(("", "fleetbot1", "Fleetbot One"))
        );
        assert_eq!(parse_bus_key("autoplay.mate"), None);
        // The snapshots on disk keep the names they were written under.
        assert_eq!(ac_store::file_safe("+Fletch"), "+Fletch");
        assert_eq!(
            ac_store::file_safe("Al'Arqas of Yaraq"),
            "Al'Arqas of Yaraq"
        );
        assert_eq!(ac_store::file_safe("../x/y"), "_x_y");
        assert_eq!(ac_store::file_safe("  .. "), "_");
        assert_eq!(
            HoldingsStore::path_for(Path::new("/c"), "A.Server", "Acc", "Ch/ar"),
            PathBuf::from("/c/a.server/acc/Ch_ar.json")
        );
    }

    #[test]
    fn a_search_says_what_each_hit_is_held_for() {
        // The record has carried this all along; the search dropped it
        // on the floor, so no window could show it.
        let mut store = HoldingsStore::new();
        let mut taper = HoldingRecord::from(&ItemStats {
            guid: 0x8000_0021,
            name: "Prismatic Taper".into(),
            wcid: 691,
            kind: "comps",
            stack: 4059,
            max_stack: 5000,
            ..Default::default()
        });
        taper.took = Some(ac_loot::LootAction::Keep);
        store.put(CharacterHoldings {
            server: "one.example".into(),
            account: "accone".into(),
            character: "Alice".into(),
            guid: 0x5000_0001,
            taken_at: unix_now(),
            online: true,
            items: vec![taper, HoldingRecord::from(&sword())],
        });
        let hits = store.search("one.example", &Query::parse("taper"), unix_now());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].stats.stack, 4059);
        assert_eq!(hits[0].took, Some(ac_loot::LootAction::Keep));
        // The sword was never written down, which is not a decision to
        // leave it: it is no decision at all.
        let hits = store.search("one.example", &Query::parse("sword"), unix_now());
        assert_eq!(hits[0].took, None);
    }

    #[test]
    fn store_saves_loads_merges_and_searches() {
        let dir = temp_dir("store");
        let now = unix_now();
        let alice = CharacterHoldings {
            server: "one.example".into(),
            account: "AccOne".into(),
            character: "Alice".into(),
            guid: 0x5000_0001,
            taken_at: now,
            online: true,
            items: vec![
                HoldingRecord::from(&sword()),
                HoldingRecord {
                    guid: 0x8000_0010,
                    name: "Pack".into(),
                    item_type: item_type::CONTAINER,
                    kind: "pack".into(),
                    container: 0x5000_0001,
                    ..Default::default()
                },
                HoldingRecord::from(&tunic(0x8000_0010)),
            ],
        };
        let bob = CharacterHoldings {
            server: "one.example".into(),
            account: "acctwo".into(),
            character: "Bob".into(),
            guid: 0x5000_0002,
            taken_at: now - 1000,
            online: true,
            items: vec![HoldingRecord::from(&tunic(0x5000_0002))],
        };
        HoldingsStore::save_to(&dir, &alice).unwrap();
        HoldingsStore::save_to(&dir, &bob).unwrap();
        assert!(dir
            .join("one.example")
            .join("accone")
            .join("Alice.json")
            .is_file());

        let store = HoldingsStore::load_all(&dir);
        assert_eq!(store.len(), 2);
        assert_eq!(store.dir(), Some(dir.as_path()));
        assert_eq!(store.counts("one.example"), (2, 4, 3));
        // Bob's word is old: not online any more, whatever it says.
        assert!(store
            .get("one.example", "ACCONE", "Alice")
            .unwrap()
            .online_at(now));
        assert!(!store
            .get("one.example", "acctwo", "Bob")
            .unwrap()
            .online_at(now));

        let hits = store.search("one.example", &Query::parse("tunic"), now);
        assert_eq!(hits.len(), 2);
        let places: Vec<(&str, &str)> = hits
            .iter()
            .map(|h| (h.character.as_str(), h.place.as_str()))
            .collect();
        assert_eq!(places, [("Alice", "Pack"), ("Bob", "pack")]);
        let hits = store.search("one.example", &Query::parse("spell:epic wielded"), now);
        assert_eq!(hits.len(), 1);
        assert_eq!(
            (hits[0].account.as_str(), hits[0].place.as_str()),
            ("AccOne", "worn")
        );
        assert!(hits[0].online);
        assert!(store
            .search("one.example", &Query::parse("dmg>20"), now)
            .is_empty());
        assert_eq!(store.search("one.example", &Query::parse(""), now).len(), 4);

        // Merging: the newer snapshot wins, an older one is dropped.
        let mut store = store;
        let mut older = alice.clone();
        older.taken_at = now - 5;
        older.items.clear();
        assert!(!store.merge(older));
        assert_eq!(
            store
                .get("one.example", "accone", "Alice")
                .unwrap()
                .items
                .len(),
            3
        );
        let mut newer = alice.clone();
        newer.taken_at = now + 5;
        newer.items.truncate(1);
        assert!(store.merge_value(&serde_json::to_value(&newer).unwrap()));
        assert_eq!(
            store
                .get("one.example", "accone", "Alice")
                .unwrap()
                .items
                .len(),
            1
        );
        assert!(!store.merge_value(&serde_json::json!("nonsense")));
        // Offline keeps the items and the file follows.
        let off = store.set_offline("one.example", "accone", "Alice").unwrap();
        assert!(!off.online);
        store.save(&off).unwrap();
        let again = HoldingsStore::load_all(&dir);
        assert!(!again.get("one.example", "accone", "Alice").unwrap().online);
        assert_eq!(
            again
                .get("one.example", "accone", "Alice")
                .unwrap()
                .items
                .len(),
            1
        );
        let _ = std::fs::remove_dir_all(&dir);
        // A missing directory is an empty store.
        assert!(HoldingsStore::load_all(&dir).is_empty());
    }
}
