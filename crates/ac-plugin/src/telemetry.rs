//! What every session did, as JSON lines for `tools/telemetry.py`: a sample of each session every
//! two seconds, and a line for each status change, chat line worth keeping, refusal, session
//! change and mark, and the autoplay settings whenever they change. One file per process under the
//! cache's `telemetry` folder, capped, newest kept. F9 marks the moment, in the log and here.

use std::collections::BTreeMap;
use std::io::Write;
use std::time::{Duration, Instant};

use ac_client::{Client, Event};
use serde_json::{json, Value};

use crate::{Ctx, Plugin};

/// How often each session is sampled.
const SAMPLE_EVERY: Duration = Duration::from_secs(2);

/// Telemetry files kept, this process's included.
const KEEP_FILES: usize = 20;

/// Most one process writes, in bytes; past it the file stops and play goes on.
const FILE_CAP: u64 = 256 * 1024 * 1024;

/// Chat lines too many and too alike to keep: every swing's hit, miss and evade.
const SPAM: &[&str] = &[
    "evades your",
    "You evade",
    " misses you",
    "You miss",
    "You hit ",
    " hits you",
    " says, \"",
];

/// Writes the telemetry file; a built-in plugin, so every host has it.
pub struct Telemetry {
    out: Option<std::io::BufWriter<std::fs::File>>,
    left: u64,
    sampled: BTreeMap<usize, Instant>,
    /// How each session's walk was last steered, so a change of way is written as it happens.
    steered: BTreeMap<usize, Steered>,
    /// The autoplay settings each session last wrote, so a change is written once.
    setup: BTreeMap<usize, ac_client::autoplay::Config>,
}

impl Default for Telemetry {
    fn default() -> Self {
        let dir = ac_store::cache_dir().join("telemetry");
        let out = match ac_store::fresh_file(&dir, "acswarm", "jsonl", KEEP_FILES) {
            Ok((file, path)) => {
                tracing::info!("telemetry to {}", path.display());
                Some(std::io::BufWriter::new(file))
            }
            Err(e) => {
                tracing::warn!("no telemetry file: {e}");
                None
            }
        };
        Telemetry {
            out,
            left: FILE_CAP,
            sampled: BTreeMap::new(),
            steered: BTreeMap::new(),
            setup: BTreeMap::new(),
        }
    }
}

impl Telemetry {
    fn write(&mut self, mut record: Value, client: &Client) {
        let Some(out) = self.out.as_mut() else {
            return;
        };
        if let Some(o) = record.as_object_mut() {
            o.insert("t".into(), json!(unix_ms()));
            o.insert("a".into(), json!(client.config.account));
            o.insert("c".into(), json!(client.world.stats.name));
        }
        let mut line = record.to_string();
        line.push('\n');
        if line.len() as u64 > self.left {
            self.left = 0;
            return;
        }
        self.left -= line.len() as u64;
        let _ = out.write_all(line.as_bytes());
    }
}

/// How a walk is steered, for spotting the frame it changes: a goal (to the metre), and whether the
/// steering walks the straight line, a route, or finds no way.
#[derive(Clone, Copy, PartialEq)]
struct Steered {
    goal: [i32; 3],
    way: &'static str,
}

fn steered(w: &ac_client::tally::WalkFrame) -> Steered {
    let way = match (w.aim, w.route) {
        (None, _) => "no way",
        (Some(_), Some(_)) => "route",
        (Some(_), None) => "straight",
    };
    Steered {
        goal: w.goal.to_array().map(|v| v.round() as i32),
        way,
    }
}

/// The session as it stands: where, how hurt, doing what, carrying what.
fn sample(client: &Client) -> Value {
    let (hp, st, mp) = (
        client.vital_now(0),
        client.vital_now(1),
        client.vital_now(2),
    );
    let pos = client.my_position().map(|p| [p.x, p.y, p.z]);
    let cell = client
        .player
        .as_ref()
        .map(|p| format!("{:08X}", p.cell))
        .unwrap_or_default();
    let target = client
        .attack_target
        .and_then(|g| client.world.name_of(g))
        .unwrap_or("");
    let (carried, capacity) = client.burden();
    // A walk frame older than a second is a walk that has ended.
    let walk = client
        .walk_frame
        .filter(|w| w.at.elapsed() < Duration::from_secs(1))
        .map(|w| {
            json!({
                "goal": w.goal.to_array(),
                "aim": w.aim.map(|a| a.to_array()),
                "detour": w.detoured,
                "route": w.route,
                "wedged": w.wedged,
            })
        });
    let hands = ac_world::item_type::MELEE_WEAPON
        | ac_world::item_type::MISSILE_WEAPON
        | ac_world::item_type::CASTER;
    let wield: Vec<&str> = client
        .world
        .wielded()
        .filter(|o| o.item_type & hands != 0)
        .map(|o| o.name.as_str())
        .collect();
    let b = client.blows;
    json!({
        "lvl": client.world.stats.level,
        "xp": client.world.stats.total_xp,
        "hp": [hp.0, hp.1], "st": [st.0, st.1], "mp": [mp.0, mp.1],
        "cell": cell, "pos": pos,
        "on": client.autoplay.config.enabled,
        "doing": client.autoplay.doing.label(),
        "step": client.autoplay.step_name(),
        "status": client.autoplay.status,
        "target": target,
        "slots": client.room_anywhere(),
        "burden": [carried, capacity],
        "coin": client.purse(),
        "walk": walk,
        "trip": client.travel_progress(),
        "wield": wield,
        // Totals this session: dealt, points dealt, taken, points taken, our misses, our evades.
        "blows": [b.dealt, b.dealt_points, b.taken, b.taken_points, b.missed, b.evaded],
    })
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

impl Plugin for Telemetry {
    fn name(&self) -> &str {
        "telemetry"
    }

    fn tick(&mut self, cx: &mut Ctx) {
        let (index, now, sessions) = (cx.index, cx.now, cx.clients.len());
        let due = self
            .sampled
            .get(&index)
            .is_none_or(|t| now.duration_since(*t) >= SAMPLE_EVERY);
        let Some(client) = cx.try_client() else {
            return;
        };
        // A walk's way, whenever it changes: the samples are two seconds apart, and a walk can go
        // wrong in its first frame (a character straight into a pocket the graph cannot leave).
        let way = client.walk_frame.as_ref().map(steered);
        if way.is_some() && way != self.steered.get(&index).copied() {
            let w = client.walk_frame.expect("way is some");
            let record = json!({
                "k": "steer", "way": way.map(|s| s.way), "goal": w.goal.to_array(),
                "aim": w.aim.map(|a| a.to_array()), "route": w.route, "wedged": w.wedged,
                "pos": client.my_position().map(|p| [p.x, p.y, p.z]),
                "cell": client.player.as_ref().map(|p| format!("{:08X}", p.cell)),
            });
            self.write(record, client);
        }
        match way {
            Some(s) => {
                self.steered.insert(index, s);
            }
            None => {
                self.steered.remove(&index);
            }
        }
        if !due || client.player.is_none() {
            return;
        }
        self.sampled.insert(index, now);
        if self.setup.get(&index) != Some(&client.autoplay.config) {
            self.setup.insert(index, client.autoplay.config.clone());
            let record =
                json!({"k": "setup", "sessions": sessions, "config": client.autoplay.config});
            self.write(record, client);
        }
        let mut record = sample(client);
        record["k"] = json!("sample");
        self.write(record, client);
        // Flushed with each round of samples, so a file read while play goes on is current.
        if let Some(out) = self.out.as_mut() {
            let _ = out.flush();
        }
    }

    fn on_event(&mut self, cx: &mut Ctx, ev: &Event) {
        let Some(client) = cx.try_client() else {
            return;
        };
        let record = match ev {
            Event::Autoplay { doing, text } => json!({"k": "status", "doing": doing, "text": text}),
            Event::Noted(text) => json!({"k": "note", "text": text}),
            Event::Chat { text, kind } => {
                if SPAM.iter().any(|s| text.contains(s)) {
                    return;
                }
                json!({"k": "chat", "text": text, "kind": kind})
            }
            Event::Refused(code) => json!({"k": "refused", "code": format!("{code:#x}")}),
            Event::Placed { .. } => json!({"k": "placed"}),
            Event::Connected => json!({"k": "connected"}),
            Event::Terminated(why) => json!({"k": "ended", "why": why}),
            _ => return,
        };
        self.write(record, client);
    }

    fn key(&mut self, cx: &mut Ctx, key: egui::Key, pressed: bool) -> bool {
        if key != egui::Key::F9 || !pressed {
            return false;
        }
        let Some(client) = cx.try_client() else {
            return true;
        };
        let mut record = sample(client);
        tracing::warn!(
            "MARK {} ({}): {} at {} -- {}",
            client.world.stats.name,
            client.config.account,
            client.autoplay.doing.label(),
            record["cell"].as_str().unwrap_or("?"),
            client.autoplay.status
        );
        record["k"] = json!("mark");
        self.write(record, client);
        if let Some(out) = self.out.as_mut() {
            let _ = out.flush();
        }
        true
    }

    fn session_removed(&mut self, index: usize) {
        crate::shift_removed(&mut self.sampled, index);
        crate::shift_removed(&mut self.setup, index);
        crate::shift_removed(&mut self.steered, index);
    }
}
