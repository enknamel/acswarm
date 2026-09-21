//! Headless mode: run many game sessions in one process with no window
//! and no GPU. What used to be a separate headless mode binary.
//!
//! The sessions themselves are `ac_plugin::Sessions`, the same core the
//! window runs on, so a session dropped mid-run is logged back in here
//! too. The loop ticks each one `--hz` (`--tick-hz`) times a second with
//! no keyboard input (plugins and the server's move-to drive movement),
//! prints what the server says, and runs the plugin host once per session
//! per frame. Lines from `--say` and `--script` are typed one per second
//! after the character is placed, through the same router the chat box
//! uses (`ac_client::action`). Ctrl-C disconnects every session cleanly.

use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use ac_client::creation::{self, CreateSpec};
use ac_client::reconnect::Ending;
use ac_client::Event;
use ac_plugin::console::Console;
use ac_plugin::{Enter, Host, Requests, SessionSpec, Sessions};

/// The lines of a script file: trimmed, without blanks and `#` comments.
pub fn parse_script(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// Lines a session types after it is placed: line `k` goes out once
/// `k + 1` periods have passed (the first a period after placement, so the
/// world has settled), never twice, and in order however late the poll.
#[derive(Debug, Clone)]
pub struct Schedule {
    lines: Vec<String>,
    period: f32,
    sent: usize,
}

impl Schedule {
    pub fn new(lines: Vec<String>, period: f32) -> Self {
        Schedule {
            lines,
            period,
            sent: 0,
        }
    }

    /// How many lines are due `since_placed` seconds after placement.
    pub fn due_by(&self, since_placed: f32) -> usize {
        if since_placed < self.period || self.period <= 0.0 {
            return 0;
        }
        let n = (since_placed / self.period).floor() as usize;
        n.min(self.lines.len())
    }

    /// The lines that became due since the last poll, in order.
    pub fn poll(&mut self, since_placed: f32) -> Vec<String> {
        let due = self.due_by(since_placed);
        let out = self.lines[self.sent..due.max(self.sent)].to_vec();
        self.sent = self.sent.max(due);
        out
    }

    /// Whether every line has gone out. The loop stops on the duration
    /// or on every session ending, so only the tests ask this.
    #[cfg(test)]
    pub fn finished(&self) -> bool {
        self.sent >= self.lines.len()
    }
}

/// What a headless run keeps beside each session: the scripted lines it
/// still has to type, when its character was placed (the lines are timed
/// from that), and the file `/log` is copying its chat to.
pub struct Typing {
    schedule: Schedule,
    placed_at: Option<Instant>,
    chat_file: Option<crate::chat::ChatFile>,
}

impl Typing {
    fn new(lines: Vec<String>) -> Self {
        Typing {
            schedule: Schedule::new(lines, 1.0),
            placed_at: None,
            chat_file: None,
        }
    }
}

/// One line about a session: placed?, cell, health, target.
fn status(s: &ac_plugin::Session<Typing>) -> String {
    let c = &s.client;
    let placed = if c.placed() { "yes" } else { "no" };
    let cell = match c.player.as_ref().map(|p| p.cell) {
        Some(cell) => format!("{cell:08X}"),
        None => "-".to_string(),
    };
    let st = &c.world.stats;
    let health = if st.name.is_empty() {
        "-".to_string()
    } else {
        format!("{}/{}", st.vitals[0].current, st.vital_max_current(0))
    };
    let target = c
        .attack_target
        .or(c.selected)
        .and_then(|g| c.world.objects.get(&g))
        .map(|o| o.name.clone())
        .unwrap_or_else(|| "-".to_string());
    let state = if s.is_finished() {
        " ended"
    } else if s.is_reconnecting() {
        " reconnecting"
    } else {
        ""
    };
    format!(
        "[{}] placed={placed} cell={cell} hp={health} target={target}{state}",
        s.account()
    )
}

/// Print what a plugin's callbacks asked for and apply the rest. A
/// headless run has no window, so a session to show (`activate`) and a
/// folder to pick (`pick_data_dir`) are nothing it can answer; the ask
/// to close it answers here and the sessions to start and stop wait for
/// [`apply_pending`], since a stop moves every session after it down.
fn apply_requests(account: &str, quit: &mut bool, pending: &mut Pending, r: Requests) {
    for (text, _) in r.chat {
        println!("[{account}] {text}");
    }
    if r.quit {
        println!("[{account}] a plugin asked to close; disconnecting");
        *quit = true;
    }
    if !r.start_sessions.is_empty() || !r.stop_sessions.is_empty() {
        pending.push((r.start_sessions, r.stop_sessions));
    }
}

/// Sessions plugins asked to start and stop this frame, applied once no
/// session is being ticked.
type Pending = Vec<(Vec<SessionSpec>, Vec<usize>)>;

/// Start and stop the sessions plugins asked for. Stops go first,
/// highest index first, so each index still means the session the plugin
/// saw; then the starts are appended in order.
fn apply_pending(
    sessions: &mut Sessions<Typing>,
    host: &mut Host,
    connect: &str,
    lines: &[String],
    pending: &mut Pending,
) {
    for (start, stop) in std::mem::take(pending) {
        let mut stop = stop;
        stop.sort_unstable();
        stop.dedup();
        for i in stop.into_iter().rev() {
            match sessions.stop(i) {
                Some(gone) => {
                    println!("[{gone}] session {} stopped", i + 1);
                    host.session_removed(i);
                }
                None => tracing::warn!("a plugin asked to stop session {i}, which is not running"),
            }
        }
        // A plugin (the fleet panel's roster, a script through
        // `fleet.start`) asked for sessions: start them here too.
        for spec in start {
            let account = spec.account.clone();
            match sessions.start(&spec, connect, Enter::First, Typing::new(lines.to_vec())) {
                Ok(i) => println!("[{account}] session {} started", i + 1),
                Err(e) => {
                    println!("[{account}] cannot start: {e}");
                    host.board
                        .set_local(ac_plugin::panels::fleet::error_key(&account), e);
                }
            }
        }
    }
}

pub fn run(cli: crate::Cli) -> Result<()> {
    // The app set up logging before it worked out which mode to run in.
    anyhow::ensure!(cli.tick_hz > 0, "--tick-hz must be at least 1");
    if cli.show_rules {
        let assets = ac_scene::Assets::open(cli.data_dir.as_ref().expect("data dir"))
            .context("opening DAT archives")?;
        let cg = assets.chargen().context("reading the CharGen table")?;
        let heritage = match &cli.heritage {
            Some(h) => ac_scene::chargen::heritage_id(&cg, h)
                .with_context(|| format!("unknown heritage {h:?}"))?,
            None => creation::HERITAGE_ALUVIAN,
        };
        let rules = creation::rules(&assets, heritage).map_err(|e| anyhow::anyhow!("{e}"))?;
        print!("{}", rules.summary());
        return Ok(());
    }
    let connect = cli.connect.clone().context("--connect is required")?;
    let mut lines = cli.say.clone();
    if let Some(path) = &cli.script {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading script {}", path.display()))?;
        lines.extend(parse_script(&text));
    }

    let assets = Rc::new(
        ac_scene::Assets::open(cli.data_dir.as_ref().expect("data dir"))
            .context("opening DAT archives")?,
    );
    // What --create makes on a session whose account lacks the character,
    // for every `--client` that named no creation fields of its own.
    let create: Option<CreateSpec> = cli.create.as_ref().map(|name| CreateSpec {
        name: name.clone(),
        heritage: cli.heritage.clone(),
        sex: cli.gender.clone(),
        template: cli.template.clone(),
        town: cli.start_area.clone(),
    });
    let mut specs: Vec<SessionSpec> = cli
        .clients
        .iter()
        .map(|s| {
            s.parse::<SessionSpec>()
                .map_err(|e| anyhow::anyhow!("--client: {e}"))
        })
        .collect::<Result<_>>()?;
    for spec in &mut specs {
        if spec.create.is_none() {
            spec.create = create.clone();
        }
    }
    // A dropped session is logged back in where it stood; without this a
    // fleet measurement scored a character that dropped as one that did
    // less. `--reconnect-tries 0` turns it off.
    let mut sessions: Sessions<Typing> = Sessions::new(
        cli.data_dir.clone().unwrap_or_default(),
        ac_client::reconnect::Policy {
            tries: cli.reconnect_tries,
            ..Default::default()
        },
    );
    sessions.set_assets(assets);
    for spec in &specs {
        // Nobody is at the keyboard, so a session with no character named
        // enters with the account's first.
        sessions
            .start(spec, &connect, Enter::First, Typing::new(lines.clone()))
            .map_err(|e| anyhow::anyhow!("connecting {} to {connect}: {e}", spec.account))?;
    }

    let mut host = Host::new();
    if let Some(bus) = &cli.bus {
        let name = sessions.get(0).map(|s| s.account().to_string());
        host.join_bus(
            Some(bus),
            &name.unwrap_or_else(|| format!("pid{}", std::process::id())),
        )
        .context("joining the bus")?;
    }
    // The panels draw nothing here but their commands (/bar ...) work.
    for p in ac_plugin::panels::live() {
        host.register(p);
    }
    host.register(Box::new(Console));
    host.register(Box::new(ac_plugin::party::Party::default()));
    host.register(Box::new(ac_plugin::team::Team::default()));
    host.register(Box::new(ac_script::ScriptPlugin::new(
        ac_script::default_dir(),
    )));
    // The rules set up in the window are the rules a windowless session
    // should play by: which ground to hunt, how to hunt it, what to keep
    // stocked. Without this a headless session ran on the defaults and
    // there was no way to tell it otherwise.
    if !cli.no_settings {
        host.load_settings(
            cli.settings
                .clone()
                .unwrap_or_else(ac_plugin::Settings::default_path),
        );
    }
    let period = Duration::from_secs_f64(1.0 / cli.tick_hz as f64);
    println!(
        "headless: {} session(s) to {}, {} Hz ({} ms per tick), {} scripted line(s), {}",
        sessions.len(),
        connect,
        cli.tick_hz,
        period.as_millis(),
        lines.len(),
        if cli.duration == 0 {
            "running until Ctrl-C".to_string()
        } else {
            format!("running for {} s", cli.duration)
        }
    );

    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        ctrlc::set_handler(move || stop.store(true, Ordering::SeqCst))
            .context("installing the Ctrl-C handler")?;
    }

    let start = Instant::now();
    let mut last = start;
    let mut next_tick = start;
    let mut next_status = start + Duration::from_secs(10);
    let mut quit = false;
    let mut pending: Pending = Vec::new();
    loop {
        let now = Instant::now();
        let dt = (now - last).as_secs_f32().min(0.25);
        last = now;

        for i in 0..sessions.len() {
            if sessions[i].is_finished() {
                continue;
            }
            let _frame = sessions[i].client.tick(None, dt, now);
            let events = sessions[i].client.drain_events();
            for ev in &events {
                let account = sessions[i].account().to_string();
                match ev {
                    Event::Chat { text, .. } => {
                        if let Some(f) = sessions[i].extra.chat_file.as_mut() {
                            f.write(&crate::chat::stamp_now(), text);
                        }
                        if cli.log_chat {
                            println!("[{account}] {text}");
                        } else {
                            tracing::debug!("[{account}] chat: {text}");
                        }
                    }
                    // A headless run has no chat window: `/log` still
                    // copies the lines it prints, `/clear` has nothing
                    // to empty.
                    Event::ChatToFile(file) => {
                        let said = match file {
                            None => match sessions[i].extra.chat_file.take() {
                                Some(f) => format!("chat log {} closed", f.path().display()),
                                None => "no chat log to close".to_string(),
                            },
                            Some(name) => match crate::chat::ChatFile::open(name) {
                                Ok(f) => {
                                    let said = format!("copying chat to {}", f.path().display());
                                    sessions[i].extra.chat_file = Some(f);
                                    said
                                }
                                Err(e) => format!("cannot write to {name}: {e}"),
                            },
                        };
                        println!("[{account}] {said}");
                    }
                    Event::ChatClear { .. } => {}
                    Event::Connected => println!("[{account}] connected"),
                    Event::Placed { cell } => {
                        println!("[{account}] placed in cell {cell:08X}");
                        sessions[i].extra.placed_at.get_or_insert(now);
                    }
                    // Whether this is the end of the session or a drop to
                    // come back from is `Sessions::tick_reconnect`'s to
                    // say, off `Client::ending`; both of these are only
                    // reported here.
                    Event::Terminated(reason) => {
                        println!("[{account}] terminated: {reason}");
                    }
                    Event::Refused(op) => {
                        println!("[{account}] refused (opcode {op:#06X})");
                    }
                    Event::Characters(list) => {
                        let names: Vec<&str> = list.iter().map(|c| c.name.as_str()).collect();
                        println!("[{account}] characters: {names:?}");
                        // The client creates a missing --create character
                        // itself (`create_when_missing`); the list comes
                        // here for the record.
                        let c = &sessions[i].client;
                        if let Some(name) = c.creating() {
                            println!(
                                "[{account}] creating {name}{}",
                                create
                                    .as_ref()
                                    .map(|c| {
                                        let s = c.summary();
                                        if s.is_empty() {
                                            s
                                        } else {
                                            format!(" ({s})")
                                        }
                                    })
                                    .unwrap_or_default()
                            );
                        } else if let Some(why) = c.create_error() {
                            println!("[{account}] cannot create: {why}");
                            sessions.ended(i, Ending::Fatal(format!("cannot create: {why}")), now);
                        } else if c.entering.is_none() {
                            println!("[{account}] no character to enter the world with (use --create NAME)");
                            sessions.ended(
                                i,
                                Ending::Fatal("no character to enter the world with".into()),
                                now,
                            );
                        }
                    }
                    Event::CharacterCreated { id, name } => {
                        println!("[{account}] created {name} ({id:#010x}); entering the world");
                    }
                    Event::CharacterCreateFailed(code) => {
                        let why = creation::create_failure_message(*code);
                        println!("[{account}] character creation failed: {why} (code {code})");
                        sessions.ended(
                            i,
                            Ending::Fatal(format!("character creation failed: {why}")),
                            now,
                        );
                    }
                    // Nothing a windowless run can do with these: no
                    // audio device to play a sound on, nothing drawing
                    // the particles of an effect, and a spell coming or
                    // going is already in `client.world`, which is what
                    // the plugins and scripts read. `Autoplay` the host
                    // itself puts on the blackboard for them.
                    Event::Sound { .. }
                    | Event::Effect { .. }
                    | Event::SpellLearned(_)
                    | Event::SpellForgotten(_)
                    | Event::Autoplay { .. } => {}
                }
            }
            let account = sessions[i].account().to_string();
            let r = host.frame(sessions.clients(), i, &events, dt, now);
            apply_requests(&account, &mut quit, &mut pending, r);
            let Some(s) = sessions.get_mut(i) else {
                continue;
            };
            // Only a session actually in the world types: one waiting to
            // be logged back in has nobody to say it to.
            let due = match s.extra.placed_at {
                Some(t) if !s.is_finished() && !s.is_reconnecting() => {
                    s.extra.schedule.poll((now - t).as_secs_f32())
                }
                _ => Vec::new(),
            };
            for line in due {
                let spoken = !line.starts_with(['/', '@']);
                println!("[{account}] {}{line}", if spoken { "> " } else { "" });
                let typed = sessions.typed(&mut host, i, &line);
                if let Some(why) = typed.refused {
                    println!("[{account}] {why}");
                }
                apply_requests(&account, &mut quit, &mut pending, typed.requests);
            }
        }
        host.end_frame();
        // A session that ended without being asked to comes back here, in
        // its own slot, before the starts and stops below can move the
        // slots around.
        let report = sessions.tick_reconnect(now);
        for (_, account, line) in report.notices {
            println!("[{account}] {line}");
        }
        for i in report.reconnected {
            // The world it was placed in is gone; the lines already typed
            // stay typed, and the rest wait for the new placement.
            sessions[i].extra.placed_at = None;
        }
        // Sessions come and go only here, between frames: no session is
        // being ticked and no plugin holds them.
        apply_pending(&mut sessions, &mut host, &connect, &lines, &mut pending);

        if now >= next_status {
            for s in sessions.iter() {
                println!("{}", status(s));
            }
            next_status += Duration::from_secs(10);
        }

        if stop.load(Ordering::SeqCst) {
            println!("headless: interrupted, disconnecting");
            break;
        }
        if quit {
            break;
        }
        if cli.duration > 0 && now - start >= Duration::from_secs(cli.duration) {
            println!("headless: {} s elapsed, disconnecting", cli.duration);
            break;
        }
        if sessions.all_finished() {
            println!("headless: every session ended");
            break;
        }

        next_tick += period;
        let after = Instant::now();
        if next_tick > after {
            std::thread::sleep(next_tick - after);
        } else {
            // Fell behind: don't try to catch up with a burst of ticks.
            next_tick = after;
        }
    }

    let mut clients: Vec<&mut ac_client::Client> = sessions
        .iter_mut()
        .filter(|s| !s.is_finished())
        .map(|s| &mut s.client)
        .collect();
    ac_client::log_off_all(&mut clients, ac_client::LOG_OFF_WAIT);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_skip_blanks_and_comments() {
        let lines =
            parse_script("  /combat \n\n# a comment\nhello there\n   \n@telepoi holtburg\n");
        assert_eq!(lines, vec!["/combat", "hello there", "@telepoi holtburg"]);
        assert!(parse_script("").is_empty());
    }

    fn lines(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("line {i}")).collect()
    }

    #[test]
    fn schedule_is_one_line_per_period() {
        let mut s = Schedule::new(lines(3), 1.0);
        assert!(s.poll(0.0).is_empty());
        assert!(s.poll(0.99).is_empty());
        assert_eq!(s.poll(1.0), vec!["line 0"]);
        assert!(s.poll(1.5).is_empty(), "nothing twice");
        assert_eq!(s.poll(2.0), vec!["line 1"]);
        assert!(!s.finished());
        assert_eq!(s.poll(3.0), vec!["line 2"]);
        assert!(s.finished());
        assert!(s.poll(100.0).is_empty(), "nothing past the end");
    }

    #[test]
    fn schedule_catches_up_in_order_after_a_late_poll() {
        let mut s = Schedule::new(lines(4), 1.0);
        assert_eq!(s.poll(2.5), vec!["line 0", "line 1"]);
        assert_eq!(s.poll(10.0), vec!["line 2", "line 3"]);
        assert!(s.finished());
    }

    #[test]
    fn schedule_period_scales_and_empty_is_finished() {
        let mut s = Schedule::new(lines(2), 0.5);
        assert_eq!(s.due_by(0.4), 0);
        assert_eq!(s.due_by(0.5), 1);
        assert_eq!(s.due_by(1.0), 2);
        assert_eq!(s.due_by(9.0), 2);
        assert_eq!(s.poll(1.0), vec!["line 0", "line 1"]);
        assert!(Schedule::new(Vec::new(), 1.0).finished());
        assert_eq!(Schedule::new(lines(2), 0.0).due_by(5.0), 0);
    }

    /// A line typed after a reconnect is not one already typed: the
    /// schedule keeps count across the drop, and the clock it is timed
    /// from starts again at the new placement, so nothing is said into a
    /// world that has not settled.
    #[test]
    fn a_reconnect_resumes_the_script_rather_than_replaying_it() {
        let mut t = Typing::new(lines(4));
        let placed = Instant::now();
        t.placed_at = Some(placed);
        assert_eq!(t.schedule.poll(2.0), vec!["line 0", "line 1"]);
        // Dropped, then placed again: the clock is the new placement's.
        t.placed_at = None;
        t.placed_at.get_or_insert(placed + Duration::from_secs(300));
        assert!(t.schedule.poll(1.0).is_empty(), "nothing twice");
        assert_eq!(t.schedule.poll(3.0), vec!["line 2"]);
        assert_eq!(t.schedule.poll(4.0), vec!["line 3"]);
        assert!(t.schedule.finished());
    }
}
