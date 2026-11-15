//! Fealty: an allegiance raised under a character of this process to the rank asked, from
//! characters made for it. ACE's rank (AllegianceNode.cs:69-88) is the greater of one more than
//! the lower of a patron's top two vassals' ranks and the higher one, so rank X is a full binary
//! tree of 2^X - 1 whose root swears to the target last, and X+1 a new bottom row under its leaves.
//! The roster, passwords with it, is `fealty.json` (owner-only); work runs while the target plays.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use ac_client::creation::CreateSpec;
use ac_client::{Client, Event};
use serde::{Deserialize, Serialize};

use crate::servers::Servers;
use crate::{Blackboard, Ctx, Plugin, Role, SessionSpec};

/// Blackboard key (this process): a list of [`Request`]s, taken when read.
pub const BUILD_KEY: &str = "fealty.build";
/// Blackboard key (this process): each target's progress line, by character name.
pub const STATUS_KEY: &str = "fealty.status";

/// The roster's file in the config directory.
const ROSTER_FILE: &str = "fealty.json";
/// How often the sessions are looked over.
const LOOK_EVERY: Duration = Duration::from_secs(1);
/// Characters an account holds: ACE's `max_chars_per_account` default (PropertyManager.cs:626).
const PER_ACCOUNT: usize = 11;
/// Member sessions logged in at once.
const MAX_ONLINE: usize = 8;
/// The highest rank (AllegianceNode.cs:87).
pub const MAX_RANK: u32 = 10;
/// Metres from the patron a swear is sent at; the server walks the rest, to 2 m
/// (Player_Allegiance.cs:75, 291).
const SWEAR_NEAR: f32 = 8.0;
/// A swear unanswered this long is sent again: the offer times out at 30 s (ConfirmationManager.cs:20).
const SWEAR_WAIT: Duration = Duration::from_secs(40);
/// A member not in the world this long after its login is logged off and tried again later.
const LOGIN_LOST: Duration = Duration::from_secs(300);
/// Between one session on an account ending and the next starting: the server lets the first go.
const ACCOUNT_REST: Duration = Duration::from_secs(10);

/// Raise `target`, a character logged in to this process, to allegiance rank `rank`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub target: String,
    pub rank: u32,
}

/// One allegiance under construction.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tree {
    /// The server's `host:port`.
    pub host: String,
    /// The character the allegiance is raised under.
    pub target: String,
    /// The rank asked for.
    pub rank: u32,
    pub accounts: Vec<Account>,
    pub members: Vec<Member>,
    /// Character names handed out so far: a refused one is never tried again.
    pub named: u32,
}

impl Tree {
    /// A member for every slot below the rank that has none, each on an account of its depth's
    /// parity with room, else on a new one; answers the accounts made.
    pub fn fill(&mut self) -> Vec<Account> {
        let mut made = Vec::new();
        let last = (1u32 << self.rank.min(MAX_RANK)) - 1;
        for slot in 1..=last {
            if self.members.iter().any(|m| m.slot == slot) {
                continue;
            }
            let side = depth(slot) % 2;
            let held = |a: &Account| self.members.iter().filter(|m| m.account == a.name).count();
            let account = match self
                .accounts
                .iter()
                .find(|a| a.side == side && held(a) < PER_ACCOUNT)
            {
                Some(a) => a.name.clone(),
                None => {
                    let a = Account {
                        name: account_name(&self.target, self.accounts.len() as u32 + 1),
                        password: crate::mules::password(),
                        side,
                    };
                    self.accounts.push(a.clone());
                    made.push(a.clone());
                    a.name
                }
            };
            self.named += 1;
            let character = character_name(&self.target, self.named);
            tracing::info!("fealty: {character} is to be made for {}", self.target);
            self.members.push(Member {
                slot,
                account,
                character,
                ..Default::default()
            });
        }
        made
    }
}

/// An account made for a tree.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Account {
    pub name: String,
    /// Made with the account; only in the owner-only roster file and the servers store.
    pub password: String,
    /// Depth parity of its characters: a patron and its vassal are never on one account.
    pub side: u32,
}

/// One character of the tree, at `slot`: the root is 1, slot n's vassals are 2n and 2n + 1.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Member {
    pub slot: u32,
    pub account: String,
    pub character: String,
    /// Came into the world once: the account holds it.
    pub made: bool,
    /// Out of the Academy: somewhere its patron and vassals can meet it.
    pub ready: bool,
    /// Sworn to its patron (the root: to the target).
    pub sworn: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct RosterFile {
    trees: Vec<Tree>,
}

/// The fealty plugin.
#[derive(Default)]
pub struct Fealty {
    trees: Vec<Tree>,
    /// The roster file as last written, or `None` before it was read.
    written: Option<RosterFile>,
    next_look: Option<Instant>,
    /// Member sessions this host started (account, when): only these it logs off.
    started: Vec<(String, Instant)>,
    /// Member sessions logging off (account, since when).
    leaving: Vec<(String, Instant)>,
    /// When each account's last session ended.
    ended: BTreeMap<String, Instant>,
    /// Swears sent, by vassal character.
    swore: BTreeMap<String, Instant>,
    /// Member sessions asked for their allegiance since they came in, by account.
    asked: Vec<String>,
    /// Accounts whose character creation the server refused.
    refused: Vec<String>,
    /// Each target's progress line as last said.
    said: BTreeMap<String, String>,
}

/// Ask for `target` to be raised to `rank` (a panel, a script).
pub fn ask(board: &mut Blackboard, target: &str, rank: u32) {
    let mut asked: Vec<Request> = board
        .get(BUILD_KEY)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    asked.push(Request {
        target: target.trim().to_string(),
        rank,
    });
    if let Ok(v) = serde_json::to_value(asked) {
        board.set_local(BUILD_KEY, v);
    }
}

impl Plugin for Fealty {
    fn name(&self) -> &str {
        "fealty"
    }

    fn on_event(&mut self, cx: &mut Ctx, ev: &Event) {
        if let Event::CharacterCreateFailed(code) = ev {
            let account = cx.client().config.account.clone();
            if self.member_on(&account).is_some() {
                tracing::warn!("fealty: making a character on {account} was refused ({code:#x})");
                self.refused.push(account);
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
                    // Unreadable is not empty: nothing is made over a roster that may hold accounts.
                    tracing::warn!("fealty: reading {}: {e}", path.display());
                    return;
                }
            };
            self.trees = file.trees.clone();
            self.written = Some(file);
        }
        self.take_requests(cx);
        self.look(cx);
        let file = RosterFile {
            trees: self.trees.clone(),
        };
        if self.written.as_ref() != Some(&file) {
            match ac_store::write_json_atomic(&path, &file, ac_store::Visibility::Private) {
                Ok(()) => self.written = Some(file),
                Err(e) => tracing::warn!("fealty: writing {}: {e}", path.display()),
            }
        }
    }
}

impl Fealty {
    /// New requests off the blackboard: a tree for each target, its rank set.
    fn take_requests(&mut self, cx: &mut Ctx) {
        let Some(v) = cx.board.get(BUILD_KEY).cloned() else {
            return;
        };
        cx.board.set_local(BUILD_KEY, serde_json::Value::Null);
        let Ok(asked) = serde_json::from_value::<Vec<Request>>(v) else {
            return;
        };
        for r in asked {
            let Some(t) = cx.clients.iter().position(|c| is_named(c, &r.target)) else {
                tracing::warn!("fealty: {} is not logged in here", r.target);
                continue;
            };
            let host = cx.clients[t].config.host.clone();
            let rank = r.rank.clamp(1, MAX_RANK);
            let i = match self.tree_of(&host, &r.target) {
                Some(i) => i,
                None => {
                    self.trees.push(Tree {
                        host,
                        target: r.target.trim().to_string(),
                        ..Default::default()
                    });
                    self.trees.len() - 1
                }
            };
            tracing::info!("fealty: {} is to be raised to rank {rank}", r.target);
            self.trees[i].rank = rank;
        }
    }

    fn tree_of(&self, host: &str, target: &str) -> Option<usize> {
        self.trees
            .iter()
            .position(|t| t.host == host && same_name(&t.target, target))
    }

    /// The tree and member index of the member on `account`.
    fn member_on(&self, account: &str) -> Option<(usize, usize)> {
        self.trees.iter().enumerate().find_map(|(t, tree)| {
            let i = tree.members.iter().position(|m| m.account == account)?;
            Some((t, i))
        })
    }

    /// Every tree looked after: its members made up to its rank, the sessions it wants logged in
    /// and the rest logged off, and each oath under way tended.
    fn look(&mut self, cx: &mut Ctx) {
        for account in std::mem::take(&mut self.refused) {
            self.rename_refused(cx, &account);
        }
        let mut wanted_members: Vec<(usize, u32)> = Vec::new();
        for t in 0..self.trees.len() {
            let here = target_session(cx, &self.trees[t]);
            if here.is_none() {
                continue;
            }
            self.grow(cx, t);
            self.note_members(cx, t);
            let online: Vec<u32> = self.trees[t]
                .members
                .iter()
                .filter(|m| session_of(cx, &self.trees[t].host, &m.account, &m.character).is_some())
                .map(|m| m.slot)
                .collect();
            let plan = wanted(&self.trees[t], &online);
            for slot in &plan.online {
                wanted_members.push((t, *slot));
            }
            self.tend_oaths(cx, t, &plan.oaths);
            self.say(cx, t);
        }
        self.keep_sessions(cx, &wanted_members);
    }

    /// The tree filled up to its rank; a new account is offered on the connect screen too, for
    /// logging in by hand.
    fn grow(&mut self, cx: &mut Ctx, t: usize) {
        let made = self.trees[t].fill();
        if made.is_empty() {
            return;
        }
        let mut servers = Servers::load(cx.settings);
        for a in made {
            servers.remember(&self.trees[t].host, &a.name, &a.password, "");
        }
        servers.save(cx.settings);
    }

    /// What each member's session shows: made, out of the Academy, sworn; and its rules.
    fn note_members(&mut self, cx: &mut Ctx, t: usize) {
        let host = self.trees[t].host.clone();
        let target = self.trees[t].target.clone();
        for i in 0..self.trees[t].members.len() {
            let m = self.trees[t].members[i].clone();
            let Some(s) = session_of(cx, &host, &m.account, &m.character) else {
                continue;
            };
            let patron = match m.slot {
                1 => target.clone(),
                n => self.trees[t]
                    .members
                    .iter()
                    .find(|p| p.slot == n / 2)
                    .map(|p| p.character.clone())
                    .unwrap_or_default(),
            };
            let c = &mut cx.clients[s];
            if !self.leaving.iter().any(|(a, _)| a == &m.account) {
                member_rules(c);
            }
            if c.world.player_guid.is_none() || c.player.is_none() {
                continue;
            }
            if !self.asked.contains(&m.account) {
                // The server's word on its patron, for an oath sworn before the roster heard of it.
                c.allegiance_update_request(false);
                self.asked.push(m.account.clone());
            }
            let member = &mut self.trees[t].members[i];
            member.made = true;
            member.ready |= is_out_of_academy(c);
            let sworn = c
                .world
                .allegiance
                .as_ref()
                .and_then(|a| a.patron.as_ref())
                .is_some_and(|p| same_name(&p.name, &patron));
            if sworn && !member.sworn {
                tracing::info!("fealty: {} is sworn to {patron}", m.character);
                member.sworn = true;
                if let Some(ts) = target_session(cx, &self.trees[t]) {
                    cx.clients[ts].allegiance_update_request(false);
                }
            }
        }
    }

    /// Each oath planned (`oaths`, by vassal slot) whose two sides are logged in: the vassal goes
    /// to its patron and swears, and the patron says yes to that vassal only.
    fn tend_oaths(&mut self, cx: &mut Ctx, t: usize, oaths: &[u32]) {
        let tree = self.trees[t].clone();
        let planned = |m: &&Member| m.ready && !m.sworn && oaths.contains(&m.slot);
        for m in tree.members.iter().filter(planned) {
            if self.leaving.iter().any(|(a, _)| a == &m.account) {
                continue;
            }
            let Some(v) = session_of(cx, &tree.host, &m.account, &m.character) else {
                continue;
            };
            let p = match m.slot {
                1 => target_session(cx, &tree),
                n => tree
                    .members
                    .iter()
                    .find(|p| p.slot == n / 2 && p.ready)
                    .and_then(|p| session_of(cx, &tree.host, &p.account, &p.character)),
            };
            let Some(p) = p.filter(|p| !cx.clients[*p].quitting) else {
                continue;
            };
            let (Some(guid), Some(pl)) = (cx.clients[p].world.player_guid, &cx.clients[p].player)
            else {
                continue;
            };
            let place = ac_world::Place {
                at: pl.world_position(),
                cell: pl.cell,
            };
            let patron_name = cx.clients[p].world.stats.name.trim().to_string();
            cx.clients[p].accept_oath_from(std::slice::from_ref(&m.character));
            let vassal = &mut cx.clients[v];
            if vassal.player.is_none() {
                continue;
            }
            let near = vassal.my_position().is_some_and(|me| {
                vassal
                    .world
                    .objects
                    .get(&guid)
                    .and_then(|o| o.world_pos())
                    .is_some_and(|at| at.distance(me) <= SWEAR_NEAR)
            });
            if !near {
                if !vassal.traveling() {
                    tracing::info!("fealty: {} goes to {patron_name}", m.character);
                    vassal.journey_to(place, 1.5);
                }
                continue;
            }
            let due = self
                .swore
                .get(&m.character)
                .is_none_or(|at| cx.now.duration_since(*at) > SWEAR_WAIT);
            if due && vassal.swear_allegiance(guid) {
                tracing::info!("fealty: {} swears to {patron_name}", m.character);
                self.swore.insert(m.character.clone(), cx.now);
            }
        }
    }

    /// The wanted member sessions logged in, one an account at a time; the rest this host
    /// started logged off; one that never came into the world given up for now.
    fn keep_sessions(&mut self, cx: &mut Ctx, members: &[(usize, u32)]) {
        // Logging off: stopped once the server let the character go, or the wait is up.
        for (account, since) in self.leaving.clone() {
            let s = cx.clients.iter().position(|c| c.config.account == account);
            let done = s.is_none_or(|s| {
                cx.clients[s].logged_off() || cx.now.duration_since(since) > ac_client::LOG_OFF_WAIT
            });
            if done {
                if let Some(s) = s {
                    cx.stop_session(s);
                }
                self.leaving.retain(|(a, _)| a != &account);
                self.started.retain(|(a, _)| a != &account);
                self.asked.retain(|a| a != &account);
                self.ended.insert(account, cx.now);
            }
        }
        for (account, since) in self.started.clone() {
            if self.leaving.iter().any(|(a, _)| a == &account) {
                continue;
            }
            let Some(s) = cx.clients.iter().position(|c| c.config.account == account) else {
                self.started.retain(|(a, _)| a != &account);
                self.asked.retain(|a| a != &account);
                self.ended.insert(account, cx.now);
                continue;
            };
            let c = &mut cx.clients[s];
            let character = c.config.character.clone().unwrap_or_default();
            let still = members.iter().any(|(t, slot)| {
                self.trees[*t]
                    .members
                    .iter()
                    .any(|m| m.slot == *slot && m.account == account && m.character == character)
            });
            let lost = c.world.player_guid.is_none() && cx.now.duration_since(since) > LOGIN_LOST;
            if !still || lost {
                if lost {
                    tracing::warn!("fealty: {character} never came into the world");
                }
                tracing::info!("fealty: {character} logs off");
                c.log_off(cx.now);
                self.leaving.push((account, cx.now));
            }
        }
        for (t, slot) in members {
            let tree = &self.trees[*t];
            let Some(m) = tree.members.iter().find(|m| m.slot == *slot) else {
                continue;
            };
            let busy = cx.clients.iter().any(|c| c.config.account == m.account)
                || self.started.iter().any(|(a, _)| a == &m.account);
            let resting = self
                .ended
                .get(&m.account)
                .is_some_and(|at| cx.now.duration_since(*at) < ACCOUNT_REST);
            if busy || resting {
                continue;
            }
            let Some(a) = tree.accounts.iter().find(|a| a.name == m.account) else {
                continue;
            };
            tracing::info!("fealty: {} logs in", m.character);
            cx.start_session(SessionSpec {
                account: a.name.clone(),
                password: a.password.clone(),
                character: Some(m.character.clone()),
                // Made where the account lacks it: the member's first login.
                create: (!m.made).then(|| CreateSpec::named(m.character.clone())),
                role: Role::Manual,
            });
            self.started.push((m.account.clone(), cx.now));
        }
    }

    /// A character whose making the server refused gets the next name, and its session goes.
    fn rename_refused(&mut self, cx: &mut Ctx, account: &str) {
        let Some((t, _)) = self.member_on(account) else {
            return;
        };
        let s = cx.clients.iter().position(|c| c.config.account == account);
        let character = s.and_then(|s| cx.clients[s].config.character.clone());
        let tree = &mut self.trees[t];
        let Some(m) = tree
            .members
            .iter_mut()
            .find(|m| m.account == account && Some(&m.character) == character.as_ref())
        else {
            return;
        };
        if m.made {
            return;
        }
        tree.named += 1;
        let name = character_name(&tree.target, tree.named);
        tracing::warn!(
            "fealty: {} was refused, {name} is made instead",
            m.character
        );
        m.character = name;
        if let Some(s) = s {
            cx.clients[s].log_off(cx.now);
            self.leaving.push((account.to_string(), cx.now));
        }
    }

    /// The target's progress on the blackboard, and in the log when it changes.
    fn say(&mut self, cx: &mut Ctx, t: usize) {
        let tree = &self.trees[t];
        let server_rank = target_session(cx, tree)
            .and_then(|s| cx.clients[s].world.allegiance.as_ref().map(|a| a.rank))
            .unwrap_or(0);
        let line = progress(tree, server_rank);
        if self.said.get(&tree.target) == Some(&line) {
            return;
        }
        tracing::info!("fealty: {}: {line}", tree.target);
        self.said.insert(tree.target.clone(), line.clone());
        let mut all = match cx.board.get(STATUS_KEY) {
            Some(serde_json::Value::Object(m)) => m.clone(),
            _ => serde_json::Map::new(),
        };
        all.insert(tree.target.clone(), serde_json::Value::String(line));
        cx.board
            .set_local(STATUS_KEY, serde_json::Value::Object(all));
    }
}

/// A member's rules: autoplay only to get out of the Academy, then standing where it is.
fn member_rules(c: &mut Client) {
    let out = c.player.is_some() && is_out_of_academy(c);
    let cfg = &mut c.autoplay.config;
    cfg.team.enabled = false;
    cfg.academy.enabled = true;
    cfg.growth.mule_runs = false;
    cfg.enabled = c.world.player_guid.is_some() && !out;
}

/// In the world and out of the Academy's landblock.
fn is_out_of_academy(c: &Client) -> bool {
    c.player
        .as_ref()
        .is_some_and(|p| p.landblock() != ac_client::academy::LANDBLOCK)
}

/// The members to have logged in and the oaths to swear now, both by slot (an oath by its
/// vassal's).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Plan {
    online: Vec<u32>,
    oaths: Vec<u32>,
}

/// One still in the Academy keeps its place, then each oath whose sides are both ready, a slot
/// in one oath at a time (a patron holds one offer at a time, ConfirmationManager.cs:36); the
/// root's last, with every other sworn; then one logged in with an oath still to come keeps its
/// place, and the rest not yet ready come in. Never two on one account, at most `MAX_ONLINE`.
fn wanted(tree: &Tree, online: &[u32]) -> Plan {
    let last = (1u32 << tree.rank.min(MAX_RANK)) - 1;
    let member = |slot: u32| tree.members.iter().find(|m| m.slot == slot);
    let in_tree = |m: &&Member| m.slot <= last;
    let mut out: Vec<&Member> = Vec::new();
    let mut oaths: Vec<u32> = Vec::new();
    let mut busy: Vec<u32> = Vec::new();
    for m in tree.members.iter().filter(in_tree) {
        if !m.ready && online.contains(&m.slot) {
            claim(&mut out, &[m]);
        }
    }
    for m in tree.members.iter().filter(in_tree).filter(|m| m.slot > 1) {
        let Some(p) = member(m.slot / 2) else {
            continue;
        };
        if m.sworn || !m.ready || !p.ready || busy.contains(&m.slot) || busy.contains(&p.slot) {
            continue;
        }
        if claim(&mut out, &[m, p]) {
            oaths.push(m.slot);
            busy.extend([m.slot, p.slot]);
        }
    }
    let rest_sworn = tree
        .members
        .iter()
        .filter(in_tree)
        .filter(|m| m.slot > 1)
        .all(|m| m.sworn);
    if let Some(root) = member(1).filter(|r| r.ready && !r.sworn && rest_sworn) {
        if claim(&mut out, &[root]) {
            oaths.push(1);
        }
    }
    for m in tree.members.iter().filter(in_tree).filter(|m| !m.ready) {
        claim(&mut out, &[m]);
    }
    // Kept only for one coming in: holding out for one that cannot would hold accounts in a ring.
    let coming: Vec<u32> = out.iter().map(|m| m.slot).collect();
    for m in tree.members.iter().filter(in_tree) {
        let patron = (m.slot > 1 && !m.sworn).then_some(m.slot / 2);
        let vassals = [m.slot * 2, m.slot * 2 + 1]
            .into_iter()
            .filter(|v| member(*v).is_some_and(|v| v.slot <= last && !v.sworn));
        let owed = patron
            .into_iter()
            .chain(vassals)
            .any(|s| coming.contains(&s));
        if m.ready && online.contains(&m.slot) && owed {
            claim(&mut out, &[m]);
        }
    }
    Plan {
        online: out.iter().map(|m| m.slot).collect(),
        oaths,
    }
}

/// `ms` added to `out` when none of their accounts is there already, nor shared between them, and
/// there is room; true when they were (or already are) all in.
fn claim<'a>(out: &mut Vec<&'a Member>, ms: &[&'a Member]) -> bool {
    let fresh: Vec<&Member> = ms
        .iter()
        .copied()
        .filter(|m| !out.iter().any(|o| o.slot == m.slot))
        .collect();
    let shared = fresh
        .iter()
        .enumerate()
        .any(|(i, a)| fresh[i + 1..].iter().any(|b| b.account == a.account));
    let taken = fresh
        .iter()
        .any(|m| out.iter().any(|o| o.account == m.account));
    if shared || taken || out.len() + fresh.len() > MAX_ONLINE {
        return false;
    }
    out.extend(fresh);
    true
}

/// The rank the roster's sworn members give the root (AllegianceNode.cs:78-87); 0 unmade.
fn rank_at(tree: &Tree, slot: u32) -> u32 {
    let Some(m) = tree.members.iter().find(|m| m.slot == slot) else {
        return 0;
    };
    if !m.made {
        return 0;
    }
    let mut below: Vec<u32> = [slot * 2, slot * 2 + 1]
        .into_iter()
        .filter(|v| tree.members.iter().any(|m| m.slot == *v && m.sworn))
        .map(|v| rank_at(tree, v))
        .collect();
    below.sort_unstable_by(|a, b| b.cmp(a));
    let higher = below.first().copied().unwrap_or(0);
    let lower = below.get(1).copied().unwrap_or(0);
    (lower + 1).max(higher).min(MAX_RANK)
}

/// One line of where a tree stands.
fn progress(tree: &Tree, server_rank: u32) -> String {
    let n = tree.members.len();
    let made = tree.members.iter().filter(|m| m.made).count();
    let ready = tree.members.iter().filter(|m| m.ready).count();
    let sworn = tree.members.iter().filter(|m| m.sworn).count();
    let state = if sworn == n && n > 0 {
        "done"
    } else {
        "building"
    };
    format!(
        "rank {} asked, the tree gives {} ({}); {made}/{n} made, {ready} ready, {sworn} sworn; \
         the server says rank {server_rank}; {} account(s)",
        tree.rank,
        rank_at(tree, 1),
        state,
        tree.accounts.len()
    )
}

/// Depth of `slot` below the root (the root is 0).
fn depth(slot: u32) -> u32 {
    31 - slot.max(1).leading_zeros()
}

/// The session of `account` on `host` entering as `character`.
fn session_of(cx: &Ctx, host: &str, account: &str, character: &str) -> Option<usize> {
    cx.clients.iter().position(|c| {
        c.config.host == host
            && c.config.account == account
            && c.config
                .character
                .as_deref()
                .is_some_and(|n| same_name(n, character))
    })
}

/// The target's session, in the world.
fn target_session(cx: &Ctx, tree: &Tree) -> Option<usize> {
    cx.clients
        .iter()
        .position(|c| c.config.host == tree.host && is_named(c, &tree.target))
        .filter(|s| cx.clients[*s].world.player_guid.is_some())
}

/// Whether session `c` plays the character `name`.
fn is_named(c: &Client, name: &str) -> bool {
    same_name(&c.world.stats.name, name)
}

/// Names compared as ACE shows them: case and an admin's `+` aside.
fn same_name(a: &str, b: &str) -> bool {
    let bare = |n: &str| n.trim().trim_start_matches('+').to_lowercase();
    !a.trim().is_empty() && bare(a) == bare(b)
}

/// The `n`th account made for `target`: its letters and digits, "vassal" and a count.
fn account_name(target: &str, n: u32) -> String {
    let base: String = target
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(16)
        .collect::<String>()
        .to_lowercase();
    format!("{base}vassal{n}")
}

/// The `n`th character made for `target`: a name takes letters and spaces only.
fn character_name(target: &str, n: u32) -> String {
    let first: String = target
        .chars()
        .filter(|c| c.is_ascii_alphabetic() || *c == ' ')
        .take(16)
        .collect();
    let first = first.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("{first} Vassal {}", crate::mules::letters(n))
}

#[cfg(test)]
mod tests;
