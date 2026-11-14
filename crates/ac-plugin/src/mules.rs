//! Mules: characters on accounts of their own holding what a character of this process keeps.
//! When one wants a mule (`Client::is_mule_wanted`) the roster's mule with the most room is logged
//! in, told it is a mule, and its place handed over; it is logged off once loaded. With none that
//! has room a new account and character are made (ACE makes an account at its first login,
//! AuthenticationHandler.cs:63); the password goes only to the servers store's logins.

use std::hash::{BuildHasher, Hasher};
use std::time::{Duration, Instant};

use ac_client::autoplay::Meet;
use ac_client::creation::CreateSpec;
use ac_client::{Client, Event};
use serde::{Deserialize, Serialize};

use crate::servers::Servers;
use crate::{Ctx, Plugin, Role, SessionSpec, Settings};

/// Settings key of the roster.
const ROSTER_KEY: &str = "mules.roster";
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
    pub character: String,
    /// Free slots over all its packs when last in the world; `None` before its first time.
    pub room: Option<u32>,
    /// Out of the Academy: somewhere a character can meet it.
    pub ready: bool,
}

/// The mule logged in now, and for whom.
#[derive(Clone, Debug)]
struct Out {
    /// Roster index.
    mule: usize,
    /// The account of the character loading it.
    hunter: String,
    since: Instant,
}

/// The mule host plugin.
#[derive(Default)]
pub struct Mules {
    roster: Vec<Mule>,
    out: Option<Out>,
    next_look: Option<Instant>,
    /// A creation the server refused: the roster entry to drop.
    refused: Option<String>,
}

impl Plugin for Mules {
    fn name(&self) -> &str {
        "mules"
    }

    fn load(&mut self, settings: &Settings) {
        self.roster = settings.get(ROSTER_KEY).unwrap_or_default();
    }

    fn save(&self, settings: &mut Settings) {
        settings.set(ROSTER_KEY, &self.roster);
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
        self.look(cx);
        cx.settings.set(ROSTER_KEY, &self.roster);
    }
}

impl Mules {
    /// The sessions looked over: mules kept mules, the one out met or let go, a new one asked for.
    fn look(&mut self, cx: &mut Ctx) {
        for c in cx.clients.iter_mut() {
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
        match self.out.clone() {
            Some(out) => self.tend(cx, &out),
            None => self.send_for(cx),
        }
    }

    /// The roster entry of session `c`, by its server and account.
    fn roster_of(&self, c: &Client) -> Option<usize> {
        self.roster
            .iter()
            .position(|m| m.host == c.config.host && m.account == c.config.account)
    }

    /// The mule out: its place to the character loading it, logged off once that is done.
    fn tend(&mut self, cx: &mut Ctx, out: &Out) {
        let mule = self.roster[out.mule].clone();
        let mule_at = session_of(cx, &mule.host, &mule.account);
        let hunter_at = cx
            .clients
            .iter()
            .position(|c| c.config.account == out.hunter && c.config.host == mule.host);
        let Some(h) = hunter_at else {
            return self.let_go(cx, mule_at, "its character left");
        };
        let Some(m) = mule_at else {
            // Not started yet, or dropped: given up once it never came.
            if cx.now.duration_since(out.since) > LOGIN_LOST {
                cx.clients[h].set_mule_meet(None);
                self.give_up(cx, &mule.account, "it never logged in");
            }
            return;
        };
        let place = {
            let c = &cx.clients[m];
            match (c.is_mule_parked(), c.player.as_ref()) {
                (true, Some(pl)) => Some(ac_world::Place {
                    at: pl.world_position(),
                    cell: pl.cell,
                }),
                _ => None,
            }
        };
        if place.is_none()
            && cx.clients[m].world.player_guid.is_none()
            && cx.now.duration_since(out.since) > LOGIN_LOST
        {
            cx.clients[h].set_mule_meet(None);
            return self.give_up(cx, &mule.account, "it never came into the world");
        }
        let hunter = &mut cx.clients[h];
        if !hunter.is_mule_wanted() {
            if hunter.is_mule_full() {
                self.roster[out.mule].room = Some(0);
            }
            hunter.set_mule_meet(None);
            return self.let_go(cx, Some(m), "loaded");
        }
        if let Some(place) = place {
            let name = cx.clients[m].world.stats.name.trim().to_string();
            cx.clients[h].set_mule_meet(Some(Meet { name, place }));
        }
    }

    /// The mule out logged off and its room noted.
    fn let_go(&mut self, cx: &mut Ctx, mule_at: Option<usize>, why: &str) {
        if let (Some(out), Some(m)) = (self.out.as_ref(), mule_at) {
            if self.roster[out.mule].ready {
                let room = cx.clients[m].room_anywhere();
                self.roster[out.mule].room = Some(room);
            }
            tracing::info!("mules: {} logs off, {why}", self.roster[out.mule].character);
            cx.stop_session(m);
        }
        self.out = None;
    }

    /// A mule that could not be had: dropped from the roster, its session stopped.
    fn give_up(&mut self, cx: &mut Ctx, account: &str, why: &str) {
        let Some(i) = self.roster.iter().position(|m| m.account == account) else {
            return;
        };
        tracing::warn!("mules: {} given up, {why}", self.roster[i].character);
        if let Some(s) = session_of(cx, &self.roster[i].host, account) {
            cx.stop_session(s);
        }
        self.roster.remove(i);
        self.out = None;
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
        let running: Vec<String> = cx
            .clients
            .iter()
            .map(|c| c.config.account.clone())
            .collect();
        let mut servers = Servers::load(cx.settings);
        let (i, create) = match pick(&self.roster, &host, &running) {
            Some(i) => (i, None),
            None => {
                let made = self.roster.iter().filter(|m| m.host == host).count() as u32;
                let (account, character) = names_for(&hunter, &hunter_name, made + 1);
                let password = password();
                servers.remember(&host, &account, &password, &character);
                servers.save(cx.settings);
                self.roster.push(Mule {
                    host: host.clone(),
                    account,
                    character: character.clone(),
                    ..Default::default()
                });
                tracing::info!("mules: making {character} for {hunter_name}");
                (self.roster.len() - 1, Some(CreateSpec::named(character)))
            }
        };
        let mule = self.roster[i].clone();
        let Some(password) = servers
            .accounts_for(&host)
            .into_iter()
            .find(|l| l.account == mule.account)
            .map(|l| l.password.clone())
            .filter(|p| !p.is_empty())
        else {
            tracing::warn!("mules: no saved login for {}", mule.character);
            self.roster.remove(i);
            return;
        };
        tracing::info!("mules: {} logs in for {hunter_name}", mule.character);
        cx.start_session(SessionSpec {
            account: mule.account.clone(),
            password,
            character: Some(mule.character.clone()),
            create,
            role: Role::default(),
        });
        self.out = Some(Out {
            mule: i,
            hunter,
            since: cx.now,
        });
    }
}

/// The mules' loot profile: no rules, and one buy line, its packs.
const MULE_PROFILE: &str = "Mule";
/// What a mule buys to fill its pack slots (ACE weenie 136, 24 slots).
const PACK: &str = "Pack";

/// A mule's rules: autoplay on, the Academy done first, and a mule after it, buying only packs.
fn mule_rules(c: &mut Client) {
    let cfg = &mut c.autoplay.config;
    cfg.enabled = true;
    cfg.mule = true;
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
        .filter(|(_, m)| m.host == host && !running.contains(&m.account))
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
    let mut letters = String::new();
    let mut k = n;
    while k > 0 {
        k -= 1;
        letters.insert(0, char::from(b'A' + (k % 26) as u8));
        k /= 26;
    }
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

/// A new account's password: 20 letters and digits off the process's random hasher keys.
fn password() -> String {
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
