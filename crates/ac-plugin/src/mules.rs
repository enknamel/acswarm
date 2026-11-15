//! Mules: characters on accounts of their own holding what a character of this process keeps.
//! A character wanting a mule (`Client::is_mule_wanted`) gets the roster's roomiest, logged in,
//! told it is a mule, its place handed over; a delivery (`deliver`) logs in every mule holding
//! some of it, as many at once as it needs. A mule is logged off once nothing wants it. With none
//! that has room a new account and character are made (ACE makes an account at its first login,
//! AuthenticationHandler.cs:63). The roster, passwords with it, is `mules.json` (owner-only),
//! written the moment it changes: a host that never saves its settings must not lose an account.

use std::hash::{BuildHasher, Hasher};
use std::time::{Duration, Instant};

use ac_client::autoplay::Meet;
use ac_client::creation::CreateSpec;
use ac_client::{Client, Event};
use serde::{Deserialize, Serialize};

use crate::servers::Servers;
use crate::{Ctx, Plugin, Role, SessionSpec};

mod deliver;

pub use deliver::{Request, Wanted, ACCOUNTS_KEY, DELIVER_KEY};

/// The roster's file in the config directory.
const ROSTER_FILE: &str = "mules.json";
/// How often the sessions are looked over.
const LOOK_EVERY: Duration = Duration::from_secs(2);
/// A mule with fewer free slots than this is full.
const ROOM_LEAST: u32 = 10;
/// A mule not in the world this long after its login (a name refused, a login failed) is given up.
const LOGIN_LOST: Duration = Duration::from_secs(300);

/// One mule: where it lives and how much room it had when last logged off.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mule {
    /// The server's `host:port`.
    pub host: String,
    pub account: String,
    /// Made with the account; only in the owner-only roster file and the servers store.
    pub password: String,
    pub character: String,
    /// Free slots over all its packs when last in the world; `None` before its first time.
    pub room: Option<u32>,
    /// Out of the Academy: somewhere a character can meet it.
    pub ready: bool,
    /// Never came into the world (its name refused, its login failed): kept, its account being
    /// made, but never picked.
    pub failed: bool,
}

/// The roster file: the mules, and what they delivered to whom (receiver account to guids),
/// which is never loaded back.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct RosterFile {
    mules: Vec<Mule>,
    delivered: std::collections::BTreeMap<String, Vec<u32>>,
}

/// The mule being loaded, and by whom.
#[derive(Clone, Debug)]
struct Loading {
    /// The mule's account.
    mule: String,
    /// The account of the character loading it.
    hunter: String,
}

/// The mule host plugin.
#[derive(Default)]
pub struct Mules {
    roster: Vec<Mule>,
    /// Items delivered, by receiver account: never loaded back.
    delivered: std::collections::BTreeMap<String, Vec<u32>>,
    /// The roster file as last written, or `None` before it was read.
    written: Option<RosterFile>,
    loading: Option<Loading>,
    deliveries: Vec<deliver::Delivery>,
    /// Mule sessions this host started, and when: only these it logs off.
    started: Vec<(String, Instant)>,
    next_look: Option<Instant>,
    /// A creation the server refused: the roster entry to give up.
    refused: Option<String>,
}

impl Plugin for Mules {
    fn name(&self) -> &str {
        "mules"
    }

    fn on_event(&mut self, cx: &mut Ctx, ev: &Event) {
        if let Event::CharacterCreateFailed(code) = ev {
            let account = cx.client().config.account.clone();
            if self.roster.iter().any(|m| m.account == account) {
                tracing::warn!("mules: making the mule on {account} was refused ({code:#x})");
                self.refused = Some(account);
            }
        }
    }

    fn tick(&mut self, cx: &mut Ctx) {
        if self.next_look.is_some_and(|t| cx.now < t) {
            return;
        }
        self.next_look = Some(cx.now + LOOK_EVERY);
        let path = ac_store::config_dir().join(ROSTER_FILE);
        if self.written.is_none() {
            let file: RosterFile = match ac_store::read_json(&path) {
                Ok(r) => r.unwrap_or_default(),
                Err(e) => {
                    // Unreadable is not empty: nothing is made over a roster that may hold mules.
                    tracing::warn!("mules: reading {}: {e}", path.display());
                    return;
                }
            };
            self.roster = file.mules.clone();
            self.delivered = file.delivered.clone();
            self.written = Some(file);
        }
        self.look(cx);
        let file = RosterFile {
            mules: self.roster.clone(),
            delivered: self.delivered.clone(),
        };
        if self.written.as_ref() != Some(&file) {
            match ac_store::write_json_atomic(&path, &file, ac_store::Visibility::Private) {
                Ok(()) => self.written = Some(file),
                Err(e) => tracing::warn!("mules: writing {}: {e}", path.display()),
            }
        }
    }
}

impl Mules {
    /// The sessions looked over: mules kept mules, the loading and deliveries tended, and the mule
    /// sessions made to match what is wanted of them.
    fn look(&mut self, cx: &mut Ctx) {
        self.take_requests(cx);
        let accounts: Vec<_> = self
            .roster
            .iter()
            .map(|m| serde_json::json!({ "host": m.host, "account": m.account, "character": m.character }))
            .collect();
        cx.board
            .set_local(ACCOUNTS_KEY, serde_json::Value::Array(accounts));
        for c in cx.clients.iter_mut() {
            if let Some(kept) = self.delivered.get(&c.config.account) {
                c.set_mule_kept(kept.clone());
            }
            let Some(i) = self.roster_of(c) else { continue };
            mule_rules(c);
            if c.is_mule_holding() {
                self.roster[i].ready = true;
                self.roster[i].room = Some(c.room_anywhere());
            }
        }
        if let Some(account) = self.refused.take() {
            self.give_up(cx, &account, "its making was refused");
        }
        match self.loading.clone() {
            Some(l) => self.tend_loading(cx, &l),
            None => self.send_for(cx),
        }
        self.tend_deliveries(cx);
        self.keep_sessions(cx);
    }

    /// The roster entry of session `c`, by its server and account.
    fn roster_of(&self, c: &Client) -> Option<usize> {
        self.roster
            .iter()
            .position(|m| m.host == c.config.host && m.account == c.config.account)
    }

    /// The mule accounts something wants logged in: the one being loaded, those with a delivery.
    fn wanted(&self) -> Vec<String> {
        let mut out: Vec<String> = self.loading.iter().map(|l| l.mule.clone()).collect();
        for d in &self.deliveries {
            for (m, _) in &d.items {
                if !out.contains(m) {
                    out.push(m.clone());
                }
            }
        }
        out
    }

    /// Every wanted mule logged in, as many at once as there are; one this host started and
    /// nothing wants logged off with its room noted; one that never came given up.
    fn keep_sessions(&mut self, cx: &mut Ctx) {
        let wanted = self.wanted();
        for account in &wanted {
            let Some(m) = self.roster.iter().find(|m| &m.account == account).cloned() else {
                continue;
            };
            let running = session_of(cx, &m.host, account).is_some();
            if running || self.started.iter().any(|(a, _)| a == account) {
                continue;
            }
            tracing::info!("mules: {} logs in", m.character);
            cx.start_session(SessionSpec {
                account: m.account.clone(),
                password: m.password.clone(),
                character: Some(m.character.clone()),
                // Made where the account lacks it: a new mule's first login.
                create: (!m.ready).then(|| CreateSpec::named(m.character.clone())),
                // Off the team: a follower would join the party's supplies and be shopped for.
                role: Role::Manual,
            });
            self.started.push((account.clone(), cx.now));
        }
        for (account, since) in self.started.clone() {
            let Some(i) = self.roster.iter().position(|m| m.account == account) else {
                continue;
            };
            let at = session_of(cx, &self.roster[i].host, &account);
            if !wanted.contains(&account) {
                if let Some(s) = at {
                    if self.roster[i].ready {
                        self.roster[i].room = Some(cx.clients[s].room_anywhere());
                    }
                    tracing::info!("mules: {} logs off", self.roster[i].character);
                    cx.stop_session(s);
                }
                self.started.retain(|(a, _)| a != &account);
                continue;
            }
            let in_world = at.is_some_and(|s| cx.clients[s].world.player_guid.is_some());
            if !in_world && cx.now.duration_since(since) > LOGIN_LOST {
                self.give_up(cx, &account, "it never came into the world");
            }
        }
    }

    /// Where the mule on `account` can be met: in the world, out of the Academy, not shopping.
    fn place_of(&self, cx: &Ctx, account: &str) -> Option<Meet> {
        let m = self.roster.iter().find(|m| m.account == account)?;
        let c = &cx.clients[session_of(cx, &m.host, account)?];
        let pl = c.player.as_ref().filter(|_| c.is_mule_parked())?;
        Some(Meet {
            name: c.world.stats.name.trim().to_string(),
            place: ac_world::Place {
                at: pl.world_position(),
                cell: pl.cell,
            },
        })
    }

    /// The mule being loaded: its place to the character loading it, let go once that is done.
    fn tend_loading(&mut self, cx: &mut Ctx, l: &Loading) {
        let Some(i) = self.roster.iter().position(|m| m.account == l.mule) else {
            self.loading = None;
            return;
        };
        let host = self.roster[i].host.clone();
        let Some(h) = session_of(cx, &host, &l.hunter) else {
            tracing::info!(
                "mules: {} is let go, its character left",
                self.roster[i].character
            );
            self.loading = None;
            return;
        };
        let meet = self.place_of(cx, &l.mule);
        let hunter = &mut cx.clients[h];
        if !hunter.is_mule_wanted() {
            if hunter.is_mule_full() {
                self.roster[i].room = Some(0);
            }
            hunter.set_mule_meet(None);
            tracing::info!("mules: {} loaded", self.roster[i].character);
            self.loading = None;
            return;
        }
        if meet.is_some() {
            hunter.set_mule_meet(meet);
        }
    }

    /// A mule that could not be had this time: its session stopped, what it was wanted for let
    /// go, and one never in the world marked failed; one that has been is tried again later.
    fn give_up(&mut self, cx: &mut Ctx, account: &str, why: &str) {
        self.started.retain(|(a, _)| a != account);
        if self.loading.as_ref().is_some_and(|l| l.mule == account) {
            if let Some(l) = self.loading.take() {
                if let Some(h) = cx.clients.iter().position(|c| c.config.account == l.hunter) {
                    cx.clients[h].set_mule_meet(None);
                }
            }
        }
        self.drop_mule_deliveries(cx, account);
        let Some(i) = self.roster.iter().position(|m| m.account == account) else {
            return;
        };
        tracing::warn!("mules: {} given up, {why}", self.roster[i].character);
        if let Some(s) = session_of(cx, &self.roster[i].host, account) {
            cx.stop_session(s);
        }
        if !self.roster[i].ready && self.roster[i].room.is_none() {
            self.roster[i].failed = true;
        }
    }

    /// A character wanting a mule gets the roster's roomiest, or a new one.
    fn send_for(&mut self, cx: &mut Ctx) {
        let wanting = cx
            .clients
            .iter_mut()
            .filter(|c| !c.autoplay.config.mule)
            .find_map(|c| {
                c.is_mule_wanted().then(|| {
                    (
                        c.config.host.clone(),
                        c.config.account.clone(),
                        c.world.stats.name.trim().to_string(),
                    )
                })
            });
        let Some((host, hunter, hunter_name)) = wanting else {
            return;
        };
        // One out for a delivery may be loaded too; one being logged off may not be had.
        let busy: Vec<String> = cx
            .clients
            .iter()
            .map(|c| c.config.account.clone())
            .filter(|a| !self.wanted().contains(a))
            .collect();
        let i = match pick(&self.roster, &host, &busy) {
            Some(i) => i,
            None => {
                // Numbered past every mule ever made here: a dropped one's name may be taken.
                let made = self.made_on(&host);
                let (account, character) = names_for(&hunter, &hunter_name, made + 1);
                let password = password();
                // Also offered on the connect screen, for logging a mule in by hand.
                let mut servers = Servers::load(cx.settings);
                servers.remember(&host, &account, &password, &character);
                servers.save(cx.settings);
                self.roster.push(Mule {
                    host: host.clone(),
                    account,
                    password,
                    character: character.clone(),
                    ..Default::default()
                });
                tracing::info!("mules: making {character} for {hunter_name}");
                self.roster.len() - 1
            }
        };
        tracing::info!(
            "mules: {} is for {hunter_name} to load",
            self.roster[i].character
        );
        self.loading = Some(Loading {
            mule: self.roster[i].account.clone(),
            hunter,
        });
    }
}

/// The mules' loot profile: no rules, and one buy line, its packs.
const MULE_PROFILE: &str = "Mule";
/// What a mule buys to fill its pack slots (ACE weenie 136, 24 slots).
const PACK: &str = "Pack";

impl Mules {
    /// How many mules were ever made on `host`, failed ones too: the highest number in a roster
    /// account's name.
    fn made_on(&self, host: &str) -> u32 {
        self.roster
            .iter()
            .filter(|m| m.host == host)
            .filter_map(|m| m.account.rsplit("mule").next()?.parse::<u32>().ok())
            .max()
            .unwrap_or(0)
    }
}

/// A mule's rules: autoplay on, off the team, the Academy done first, and a mule after it, buying
/// only packs.
fn mule_rules(c: &mut Client) {
    let cfg = &mut c.autoplay.config;
    cfg.enabled = true;
    cfg.mule = true;
    cfg.team.enabled = false;
    cfg.academy.enabled = true;
    cfg.growth.mule_runs = false;
    cfg.growth.town_runs = true;
    cfg.quests = Default::default();
    cfg.loot.profile = MULE_PROFILE.into();
    let Some(packs) = packs_wanted(c) else {
        return;
    };
    let line = ac_client::profile::Buy {
        what: PACK.into(),
        keep: packs,
        restock_at: Some(packs.saturating_sub(1)),
        on: packs > 0,
        ..Default::default()
    };
    let have = c.profiles.get(MULE_PROFILE);
    if have
        .as_ref()
        .is_some_and(|p| p.buy == [line.clone()] && p.rules.is_empty())
    {
        return;
    }
    let profile = ac_client::profile::Profile {
        name: MULE_PROFILE.into(),
        note: "A mule's: it keeps all it is given and buys only its packs".into(),
        buy: vec![line],
        ..Default::default()
    };
    if let Err(e) = c.profiles.put(profile) {
        tracing::warn!("mules: writing the {MULE_PROFILE} profile: {e}");
    }
}

/// How many packs a mule in the world should hold: those it has and a pack slot for each free
/// one (ContainersCapacity, 7 on a new character).
fn packs_wanted(c: &Client) -> Option<u32> {
    let me = c.world.player()?;
    let slots = me.containers_capacity;
    if slots == 0 {
        return None;
    }
    let is_pack = |o: &&ac_world::WorldObject| o.item_type & ac_world::item_type::CONTAINER != 0;
    let held = c.world.main_pack().filter(is_pack).count() as u32;
    let packs = c
        .world
        .main_pack()
        .filter(is_pack)
        .filter(|o| o.name == PACK)
        .count() as u32;
    Some(packs + slots.saturating_sub(held))
}

/// The session index of `account` on `host`.
fn session_of(cx: &Ctx, host: &str, account: &str) -> Option<usize> {
    cx.clients
        .iter()
        .position(|c| c.config.account == account && c.config.host == host)
}

/// The roster's mule on `host` with the most room, not logged in (`running`) and not full; one
/// never in the world counts as roomy, a new one about to be made.
fn pick(roster: &[Mule], host: &str, running: &[String]) -> Option<usize> {
    roster
        .iter()
        .enumerate()
        .filter(|(_, m)| m.host == host && !m.failed && !running.contains(&m.account))
        .filter(|(_, m)| m.room.is_none_or(|r| r >= ROOM_LEAST))
        .max_by_key(|(_, m)| m.room.unwrap_or(u32::MAX))
        .map(|(i, _)| i)
}

/// The `n`th mule's account and character: the hunter's account and name with "mule" and a
/// count; a character name takes letters and spaces only (A, B, .. Z, AA).
fn names_for(hunter_account: &str, hunter_name: &str, n: u32) -> (String, String) {
    let base: String = hunter_account
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_lowercase();
    let letters = letters(n);
    let letters_only: String = hunter_name
        .chars()
        .filter(|c| c.is_ascii_alphabetic() || *c == ' ')
        .take(16)
        .collect();
    let first = letters_only
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (format!("{base}mule{n}"), format!("{first} Mule {letters}"))
}

/// The `n`th name in letters, counting from 1: A, B, .. Z, AA.
pub(crate) fn letters(n: u32) -> String {
    let mut out = String::new();
    let mut k = n;
    while k > 0 {
        k -= 1;
        out.insert(0, char::from(b'A' + (k % 26) as u8));
        k /= 26;
    }
    out
}

/// A new account's password: 20 letters and digits off the process's random hasher keys.
pub(crate) fn password() -> String {
    const CHARS: &[u8] = b"abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    (0..20)
        .map(|i| {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u32(i);
            CHARS[(h.finish() % CHARS.len() as u64) as usize] as char
        })
        .collect()
}

#[cfg(test)]
mod tests;
