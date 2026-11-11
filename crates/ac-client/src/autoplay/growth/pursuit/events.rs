//! Server events a pursued source needs ([`Source::event`]): a creature or NPC only an event's
//! generator puts out comes once something starts the event (EventManager.cs:40-60). The client
//! cannot read an event: it takes one for on once it started it (its item handed over, its starter
//! killed) or sees what only the event puts out, until [`belief`] passes with none of that seen or
//! the gated NPC is not where it stands.

use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use ac_world::quests::{Source, Want, Yield};

use crate::Client;

/// A hunted creature an event gates is waited for this many times its spawn wait by the data's
/// rate before the event is taken for over (guess).
const WAITS: f32 = 2.0;

/// Each weenie class only an event puts out, and that event: seeing one says it is on.
fn gated() -> &'static BTreeMap<u32, String> {
    static GATED: OnceLock<BTreeMap<u32, String>> = OnceLock::new();
    GATED.get_or_init(|| {
        let sources = ac_world::quests::sources().iter();
        sources
            .filter_map(|s| Some((s.wcid, s.event.clone()?)))
            .collect()
    })
}

/// Each creature whose death starts an event, and those events.
fn started_by_kill() -> &'static BTreeMap<u32, Vec<String>> {
    static KILLS: OnceLock<BTreeMap<u32, Vec<String>>> = OnceLock::new();
    KILLS.get_or_init(|| {
        let mut m: BTreeMap<u32, Vec<String>> = BTreeMap::new();
        for s in ac_world::quests::sources() {
            if let (Want::Event(e), Yield::Kill) = (&s.want, s.kind) {
                m.entry(s.wcid).or_default().push(e.clone());
            }
        }
        m
    })
}

/// How long event `e` is taken for on with nothing of it seen: [`WAITS`] times the spawn wait of
/// the hunted creatures it gates, the fastest; `None` when it gates only NPCs (on till one is not
/// where it stands).
pub(crate) fn belief(e: &str) -> Option<Duration> {
    let hunted = ac_world::quests::sources()
        .iter()
        .filter(|s| s.kind.is_hunted() && s.event.as_deref() == Some(e));
    let fastest = hunted
        .flat_map(|s| s.rates.iter().map(|r| r.per_hour))
        .fold(0.0f32, f32::max);
    (fastest > 0.0).then(|| Duration::from_secs_f32(WAITS * 3600.0 / fastest))
}

impl Client {
    /// Whether event `e` is taken for on at `now`: started or seen within its [`belief`].
    pub(crate) fn is_event_on(&self, e: &str, now: Instant) -> bool {
        let Some(at) = self.autoplay.growth.events_on.since(&e.to_string()) else {
            return false;
        };
        belief(e).is_none_or(|d| now.saturating_duration_since(at) < d)
    }

    /// Event `e` started by this character, or its work seen, at `now`.
    pub(crate) fn note_event(&mut self, e: &str, now: Instant) {
        if self
            .autoplay
            .growth
            .events_on
            .since(&e.to_string())
            .is_none()
        {
            tracing::info!("pursuit: the {e} event is on");
        }
        self.autoplay.growth.events_on.mark(e.to_string(), now);
    }

    /// Event `e` taken for over: what it gates was not where it stands.
    pub(crate) fn forget_event(&mut self, e: &str) {
        tracing::info!("pursuit: the {e} event is taken for over");
        self.autoplay.growth.events_on.forget(&e.to_string());
    }

    /// Every event something in view says is on: a creature or NPC alive that only it puts out.
    pub(super) fn see_events(&mut self, now: Instant) {
        let gated = gated();
        let seen: Vec<String> = self
            .world
            .objects
            .values()
            .filter(|o| o.alive_or_unknown())
            .filter_map(|o| gated.get(&o.weenie_class_id).cloned())
            .collect();
        for e in seen {
            self.note_event(&e, now);
        }
    }

    /// The events a kill at `at` started: a creature lying there whose death starts one, once its
    /// watcher has heard as many of our kills as it counts (`quests::deaths_heard`).
    pub(crate) fn note_event_kill(&mut self, at: glam::Vec3, now: Instant) {
        let kills = started_by_kill();
        let heard: Vec<(String, u32)> = self
            .world
            .objects
            .values()
            .filter(|o| o.world_pos().is_some_and(|p| p.distance(at) < 1.0))
            .filter_map(|o| Some((o.weenie_class_id, kills.get(&o.weenie_class_id)?)))
            .flat_map(|(w, es)| es.iter().map(move |e| (e.clone(), w)))
            .collect();
        for (e, wcid) in heard {
            let deaths = self
                .autoplay
                .growth
                .event_deaths
                .entry(e.clone())
                .or_default();
            *deaths += 1;
            if *deaths >= ac_world::quests::deaths_heard(&e, wcid) {
                self.autoplay.growth.event_deaths.remove(&e);
                self.note_event(&e, now);
            }
        }
    }

    /// The event the give `s` starts, handed its item.
    pub(super) fn note_given(&mut self, s: &Source, now: Instant) {
        if let Want::Event(e) = &s.want {
            self.note_event(e, now);
        }
    }
}
