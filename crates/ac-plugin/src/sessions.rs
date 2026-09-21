//! The sessions a host runs: starting them, stopping them, putting a
//! dropped one back, and typing a line into one.
//!
//! A window and a headless run differ in what they keep beside a session,
//! not in how a session lives, so that state rides along as `extra` and
//! the living is here once. Reconnection in particular is the same job
//! either way, and a fleet measurement is a headless run.

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use ac_client::reconnect::{Action, Carry, Ending, Policy, Reconnect, State, Stopped};
use ac_client::{Client, Config};
use ac_scene::Assets;

use crate::{CreateSpec, Host, Requests, SessionSpec};

/// How a session opens its connection: [`Client::connect`] in a host
/// that talks to a server, something with no socket in a test.
pub type Open = Box<dyn Fn(Config, Rc<Assets>) -> std::io::Result<Client>>;

/// Whether a new session walks into the world by itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Enter {
    /// Enter with the named character, else the account's first one.
    /// The command line's sessions: nobody is at the keyboard to choose.
    First,
    /// Enter only when the spec names a character or creates one, else
    /// stop at the character-select screen (the connect screen's path).
    Named,
}

/// One live session and what the host keeps beside it.
pub struct Session<E> {
    pub client: Client,
    /// The server this session was started against, so a dropped session
    /// comes back to the same one. Who it was comes from the session
    /// itself ([`Carry`]), which knows the character actually in the
    /// world rather than the one asked for.
    pub host: String,
    /// When to come back after a drop (see `ac_client::reconnect`).
    pub reconnect: Reconnect,
    /// This client's ending has been handed to `reconnect`. Cleared when
    /// a fresh client replaces it, so the next ending is heard too.
    ending_reported: bool,
    /// What only this host keeps per session: GPU state in the window,
    /// a line schedule in a headless run.
    pub extra: E,
}

impl<E> Session<E> {
    pub fn account(&self) -> &str {
        &self.client.config.account
    }

    /// Why it gave up coming back, while it has.
    pub fn stopped(&self) -> Option<&Stopped> {
        self.reconnect.stopped()
    }

    /// The connection is gone and nothing will bring it back: the player
    /// quit, the retries ran out, or the server refused for good. A
    /// session merely waiting for its next attempt is not finished.
    pub fn is_finished(&self) -> bool {
        self.reconnect.stopped().is_some()
    }

    /// Coming back is under way.
    pub fn is_reconnecting(&self) -> bool {
        self.reconnect.busy()
    }
}

/// What a reconnection tick has to tell the host.
#[derive(Debug, Default)]
pub struct Report {
    /// Session index, account, and the line to show.
    pub notices: Vec<(usize, String, String)>,
    /// Accounts whose session is back in the world.
    pub back: Vec<String>,
    /// Sessions whose `Client` was just replaced by a fresh connection.
    /// Their `extra` describes a world that is gone.
    pub reconnected: Vec<usize>,
    /// Every account that has given up, and why. Reported each tick, not
    /// once, so a host that shows it can set it wherever nothing else has.
    pub gave_up: Vec<(String, String)>,
}

/// What typing a line into a session produced.
#[derive(Default)]
pub struct Typed {
    /// What the plugins that saw the line asked the host for.
    pub requests: Requests,
    /// Why the client would not do it, when it refused.
    pub refused: Option<ac_client::did::Because>,
    /// It was a command the retail table has no row for, so the plugins
    /// and scripts were offered it first.
    pub offered: bool,
}

/// Every session in one process, in the order they started.
pub struct Sessions<E> {
    list: Vec<Session<E>>,
    /// The archives every session shares, opened once by the first start.
    assets: Option<Rc<Assets>>,
    /// Where to open them when the host has not opened them already.
    data_dir: PathBuf,
    /// How hard a dropped session tries to come back.
    pub policy: Policy,
    open: Open,
}

impl<E> Sessions<E> {
    pub fn new(data_dir: PathBuf, policy: Policy) -> Self {
        Sessions {
            list: Vec::new(),
            assets: None,
            data_dir,
            policy,
            open: Box::new(Client::connect),
        }
    }

    /// Open sessions with `open` rather than a real socket. A test hands
    /// out `Client::offline` clients this way.
    pub fn open_with(&mut self, open: Open) {
        self.open = open;
    }

    /// The archives, when they are open.
    pub fn assets(&self) -> Option<&Rc<Assets>> {
        self.assets.as_ref()
    }

    /// Share archives the host opened for itself; a host that opens none
    /// leaves the first start to open them.
    pub fn set_assets(&mut self, assets: Rc<Assets>) {
        self.assets = Some(assets);
    }

    fn assets_or_open(&mut self) -> Result<Rc<Assets>, String> {
        if let Some(a) = &self.assets {
            return Ok(a.clone());
        }
        let a =
            Assets::open(&self.data_dir).map_err(|e| format!("opening the DAT archives: {e}"))?;
        let a = Rc::new(a);
        self.assets = Some(a.clone());
        Ok(a)
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn get(&self, i: usize) -> Option<&Session<E>> {
        self.list.get(i)
    }

    pub fn get_mut(&mut self, i: usize) -> Option<&mut Session<E>> {
        self.list.get_mut(i)
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Session<E>> {
        self.list.iter()
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, Session<E>> {
        self.list.iter_mut()
    }

    /// Every session's client, for the plugin host's callbacks.
    pub fn clients(&mut self) -> Vec<&mut Client> {
        self.list.iter_mut().map(|s| &mut s.client).collect()
    }

    /// The session logged in as `account`, if one is.
    pub fn index_of(&self, account: &str) -> Option<usize> {
        self.list
            .iter()
            .position(|s| s.account().eq_ignore_ascii_case(account))
    }

    /// Log `spec` in against `host` as another session of this process.
    /// Answers the new session's index, or why it could not start.
    pub fn start(
        &mut self,
        spec: &SessionSpec,
        host: &str,
        enter: Enter,
        extra: E,
    ) -> Result<usize, String> {
        if self.index_of(&spec.account).is_some() {
            return Err("already running".into());
        }
        let cfg = Config {
            host: host.to_string(),
            account: spec.account.clone(),
            password: spec.password.clone(),
            character: spec.character_name().map(str::to_string),
            auto_enter: match enter {
                Enter::First => true,
                Enter::Named => false,
            },
        };
        let i = self.start_config(cfg, spec.create.clone(), extra)?;
        // A named character is still the one to enter with; the create
        // only fills in for it when the account lacks it.
        if spec.create.is_some() {
            if let Some(name) = &spec.character {
                self.list[i].client.config.character = Some(name.clone());
            }
        }
        Ok(i)
    }

    /// Open a connection from a config the host built itself, and add it.
    /// Answers the new session's index, or why it could not start.
    pub fn start_config(
        &mut self,
        cfg: Config,
        create: Option<CreateSpec>,
        extra: E,
    ) -> Result<usize, String> {
        if self.index_of(&cfg.account).is_some() {
            return Err("already running".into());
        }
        let assets = self.assets_or_open()?;
        let host = cfg.host.clone();
        let mut client = (self.open)(cfg, assets).map_err(|e| e.to_string())?;
        if let Some(create) = create {
            client.create_when_missing(create);
        }
        self.list.push(Session {
            client,
            host,
            reconnect: Reconnect::new(self.policy.clone()),
            ending_reported: false,
            extra,
        });
        Ok(self.list.len() - 1)
    }

    /// Log session `i` off as far as a session about to be dropped can
    /// be -- there is no frame left to wait for the server in -- and
    /// drop it. The sessions after it move down one index. Answers the
    /// account it was, so the host can say so and tell its plugins.
    pub fn stop(&mut self, i: usize) -> Option<String> {
        if i >= self.list.len() {
            return None;
        }
        let mut s = self.list.remove(i);
        let now = Instant::now();
        s.client.log_off(now);
        s.client.disconnect(now);
        Some(s.client.config.account.clone())
    }

    /// Ask every character to log off; the frames run on until
    /// [`Client::logged_off`] says each has.
    pub fn log_off_all(&mut self, now: Instant) {
        for s in &mut self.list {
            s.client.log_off(now);
        }
    }

    /// Every character has logged off, or was never asked to.
    pub fn all_logged_off(&self) -> bool {
        self.list.iter().all(|s| s.client.logged_off())
    }

    /// Every session has ended for good (see [`Session::is_finished`]).
    pub fn all_finished(&self) -> bool {
        self.list.iter().all(|s| s.is_finished())
    }

    /// This session ended for a reason the connection itself does not
    /// show: a character the account has not got and cannot create, a
    /// refusal the host read. A `Fatal` stops the retries for good.
    pub fn ended(&mut self, i: usize, ending: Ending, now: Instant) {
        if let Some(s) = self.list.get_mut(i) {
            s.ending_reported = true;
            s.reconnect.ended(ending, now);
        }
    }

    /// Notice the sessions that have ended, and put the dropped ones back.
    ///
    /// A session being reconnected keeps its place in the list: the
    /// window goes on showing the world it was last in, the plugins keep
    /// the per-session state they index by that slot, and the fleet panel
    /// keeps its row. Only the `Client` inside is replaced, so what
    /// survives a reconnect is what [`Carry`] names and nothing else.
    pub fn tick_reconnect(&mut self, now: Instant) -> Report {
        let mut report = Report::default();
        if self.list.is_empty() {
            return report;
        }
        let mut attempts: Vec<usize> = Vec::new();
        for (i, s) in self.list.iter_mut().enumerate() {
            // In the world (or back at the select screen we were dropped
            // from): the attempt worked. `Client::placed` stays true on a
            // dropped client, whose scene is still up, so this only
            // counts while an attempt is actually in flight.
            let back = s.client.placed()
                || (s.client.config.character.is_none() && s.client.characters_known);
            if back && matches!(s.reconnect.state(), State::Trying { .. }) {
                s.reconnect.placed();
                report.back.push(s.client.config.account.clone());
            }
            if let Some(ending) = s.client.ending() {
                if !s.ending_reported {
                    s.ending_reported = true;
                    tracing::warn!(
                        "session {} ({}) ended: {}",
                        i + 1,
                        s.client.config.account,
                        ending.describe()
                    );
                    s.reconnect.ended(ending, now);
                }
            }
            if s.reconnect.poll(now) == Action::Connect {
                attempts.push(i);
            }
            let account = s.client.config.account.clone();
            while let Some(line) = s.reconnect.take_notice() {
                report.notices.push((i, account.clone(), line));
            }
        }
        for i in attempts {
            if self.reconnect_session(i, now) {
                report.reconnected.push(i);
            }
        }
        for s in &self.list {
            let why = match s.reconnect.stopped() {
                None | Some(Stopped::Quit) => continue,
                Some(Stopped::Exhausted) => "dropped, and out of retries".to_string(),
                Some(Stopped::Fatal(why)) => why.clone(),
            };
            report.gave_up.push((s.client.config.account.clone(), why));
        }
        report
    }

    /// Log a dropped session back in where it stood: the same slot, the
    /// same account against the same server, and the same character, so
    /// it lands back in the world rather than at the character-select
    /// screen. True when a fresh client took the old one's place.
    fn reconnect_session(&mut self, i: usize, now: Instant) -> bool {
        let Some(s) = self.list.get(i) else {
            return false;
        };
        let carry = Carry::of(&s.client);
        let cfg = Config {
            host: s.host.clone(),
            account: s.client.config.account.clone(),
            password: s.client.config.password.clone(),
            character: carry.character.clone(),
            // Straight back into the world when we know who we were; a
            // session dropped at the select screen comes back to it.
            auto_enter: carry.character.is_some(),
        };
        let account = cfg.account.clone();
        let assets = match self.assets.clone() {
            Some(a) => a,
            None => {
                self.list[i]
                    .reconnect
                    .ended(Ending::Fatal("the DAT archives are not open".into()), now);
                return false;
            }
        };
        tracing::info!(
            "reconnecting session {} ({account}) as {}",
            i + 1,
            carry.character.as_deref().unwrap_or("<character select>")
        );
        match (self.open)(cfg, assets) {
            Ok(mut client) => {
                carry.apply(&mut client);
                let s = &mut self.list[i];
                // Tell the server to let the old session go, in case it
                // is only half dead (an attempt that timed out).
                s.client.disconnect(now);
                s.client = client;
                s.ending_reported = false;
                true
            }
            Err(e) => {
                // The socket would not open (no route, DNS gone): this
                // attempt is spent, and the rule schedules the next.
                tracing::warn!("reconnecting {account}: {e}");
                self.list[i]
                    .reconnect
                    .ended(Ending::Dropped(e.to_string()), now);
                false
            }
        }
    }

    /// Type `line` into session `i`. One router decides what it means
    /// (`ac_client::action`): a retail command acts here and words are
    /// said; a command the table has no row for is the plugins' and the
    /// scripts' first, and the server's only if none takes it.
    pub fn typed(&mut self, host: &mut Host, i: usize, line: &str) -> Typed {
        use ac_client::action::Line;
        let Some(s) = self.list.get_mut(i) else {
            return Typed::default();
        };
        match s.client.chat_line(line) {
            Line::Acted(Ok(())) => Typed::default(),
            Line::Acted(Err(why)) => Typed {
                refused: Some(why),
                ..Typed::default()
            },
            Line::Offer { unclaimed, .. } => {
                let requests = host.command(self.clients(), i, line);
                let mut refused = None;
                if !requests.consumed {
                    if let Some(s) = self.list.get_mut(i) {
                        if let Err(why) = s.client.act(unclaimed) {
                            refused = Some(why);
                        }
                    }
                }
                Typed {
                    requests,
                    refused,
                    offered: true,
                }
            }
        }
    }
}

impl<E> std::ops::Index<usize> for Sessions<E> {
    type Output = Session<E>;

    fn index(&self, i: usize) -> &Session<E> {
        &self.list[i]
    }
}

impl<E> std::ops::IndexMut<usize> for Sessions<E> {
    fn index_mut(&mut self, i: usize) -> &mut Session<E> {
        &mut self.list[i]
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::time::Duration;

    use super::*;

    /// Sessions with no socket: every connection is an offline client,
    /// and the accounts opened are counted.
    fn offline_sessions(opened: Rc<RefCell<Vec<String>>>) -> Sessions<u32> {
        let mut s = Sessions::new(PathBuf::from("/nowhere"), Policy::default());
        s.set_assets(Rc::new(Assets::empty()));
        s.open_with(Box::new(move |cfg: Config, assets: Rc<Assets>| {
            opened.borrow_mut().push(cfg.account.clone());
            let mut c = Client::offline(assets);
            c.config = cfg;
            Ok(c)
        }));
        s
    }

    fn spec(account: &str, character: Option<&str>) -> SessionSpec {
        SessionSpec {
            account: account.into(),
            password: "secret".into(),
            character: character.map(str::to_string),
            create: None,
            role: crate::Role::default(),
        }
    }

    /// The connection dies without a clean quit: the host hands the
    /// ending to the core, which is where a socket that stopped
    /// answering and a refusal the host read both arrive.
    fn drop_the_connection(s: &mut Sessions<u32>, i: usize, now: Instant) {
        s.ended(i, Ending::Dropped("the link died".into()), now);
    }

    #[test]
    fn a_dropped_session_is_logged_back_in_where_it_stood() {
        let opened = Rc::new(RefCell::new(Vec::new()));
        let mut s = offline_sessions(opened.clone());
        let i = s
            .start(&spec("bob", Some("Reborn")), "127.0.0.1", Enter::First, 7)
            .expect("started");
        assert_eq!(*opened.borrow(), vec!["bob"]);

        let t0 = Instant::now();
        drop_the_connection(&mut s, i, t0);
        let r = s.tick_reconnect(t0);
        assert!(r.reconnected.is_empty(), "nothing yet: it waits first");
        assert!(s[i].is_reconnecting());
        assert!(
            !s[i].is_finished(),
            "a session waiting to come back is live"
        );

        // The first attempt is due after `first_delay`.
        let t1 = t0 + Policy::default().first_delay;
        let r = s.tick_reconnect(t1);
        assert_eq!(r.reconnected, vec![i]);
        assert_eq!(*opened.borrow(), vec!["bob", "bob"]);
        // The same slot, the same server, and the character it was.
        assert_eq!(s.len(), 1);
        assert_eq!(s[i].host, "127.0.0.1");
        assert_eq!(s[i].client.config.character.as_deref(), Some("Reborn"));
        assert_eq!(s[i].extra, 7, "what the host keeps rides along");
        let said: Vec<&str> = r.notices.iter().map(|(_, _, l)| l.as_str()).collect();
        assert!(
            said.iter().any(|l| l.starts_with("Reconnecting (1 of")),
            "{said:?}"
        );
    }

    #[test]
    fn a_session_the_server_never_answers_runs_out_of_tries() {
        let opened = Rc::new(RefCell::new(Vec::new()));
        let mut s = offline_sessions(opened.clone());
        s.policy = Policy {
            tries: 2,
            ..Policy::default()
        };
        let i = s
            .start(&spec("bob", Some("Reborn")), "127.0.0.1", Enter::First, 0)
            .expect("started");
        let mut t = Instant::now();
        drop_the_connection(&mut s, i, t);
        // Each attempt connects and is never placed, so `attempt_timeout`
        // fails it and the rule schedules the next: the whole loop, with
        // nothing standing in for it.
        for _ in 0..6 {
            t += Duration::from_secs(60);
            s.tick_reconnect(t);
        }
        assert_eq!(s[i].stopped(), Some(&Stopped::Exhausted));
        assert!(s[i].is_finished());
        assert!(s.all_finished());
        let r = s.tick_reconnect(t + Duration::from_secs(600));
        assert_eq!(
            r.gave_up,
            vec![("bob".to_string(), "dropped, and out of retries".to_string())]
        );
        // Two attempts, on top of the first login.
        assert_eq!(opened.borrow().len(), 3, "{:?}", opened.borrow());
    }

    #[test]
    fn a_session_the_player_quit_is_never_reconnected() {
        let opened = Rc::new(RefCell::new(Vec::new()));
        let mut s = offline_sessions(opened.clone());
        let i = s
            .start(&spec("bob", Some("Reborn")), "127.0.0.1", Enter::First, 0)
            .expect("started");
        let t = Instant::now();
        s.ended(i, Ending::Quit, t);
        let r = s.tick_reconnect(t + Duration::from_secs(600));
        assert!(r.reconnected.is_empty());
        assert!(r.gave_up.is_empty(), "a quit is nobody's error");
        assert_eq!(s[i].stopped(), Some(&Stopped::Quit));
        assert_eq!(opened.borrow().len(), 1);
    }

    #[test]
    fn a_fatal_refusal_stops_the_retries_for_good() {
        let opened = Rc::new(RefCell::new(Vec::new()));
        let mut s = offline_sessions(opened.clone());
        let i = s
            .start(&spec("bob", None), "127.0.0.1", Enter::First, 0)
            .expect("started");
        let t = Instant::now();
        s.ended(i, Ending::Fatal("no such account".into()), t);
        let r = s.tick_reconnect(t + Duration::from_secs(600));
        assert!(r.reconnected.is_empty());
        assert_eq!(
            r.gave_up,
            vec![("bob".to_string(), "no such account".to_string())]
        );
        assert_eq!(opened.borrow().len(), 1);
    }

    #[test]
    fn one_account_runs_once_per_process() {
        let opened = Rc::new(RefCell::new(Vec::new()));
        let mut s = offline_sessions(opened.clone());
        s.start(&spec("bob", None), "127.0.0.1", Enter::First, 0)
            .expect("started");
        assert_eq!(
            s.start(&spec("BOB", None), "127.0.0.1", Enter::First, 0),
            Err("already running".into())
        );
        assert_eq!(s.len(), 1);
        assert_eq!(s.index_of("bob"), Some(0));
    }

    #[test]
    fn stopping_a_session_moves_the_rest_down() {
        let opened = Rc::new(RefCell::new(Vec::new()));
        let mut s = offline_sessions(opened.clone());
        for a in ["one", "two", "three"] {
            s.start(&spec(a, None), "127.0.0.1", Enter::First, 0)
                .expect("started");
        }
        assert_eq!(s.stop(0), Some("one".into()));
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].account(), "two");
        assert_eq!(s.index_of("one"), None);
        assert_eq!(s.stop(9), None);
    }

    #[test]
    fn entering_by_itself_is_the_command_lines_rule_only() {
        let opened = Rc::new(RefCell::new(Vec::new()));
        let mut s = offline_sessions(opened.clone());
        s.start(&spec("cli", None), "127.0.0.1", Enter::First, 0)
            .expect("started");
        assert!(s[0].client.config.auto_enter, "no character named, goes in");
        s.start(&spec("screen", None), "127.0.0.1", Enter::Named, 0)
            .expect("started");
        assert!(
            !s[1].client.config.auto_enter,
            "the connect screen stops to ask"
        );
    }
}
