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
/// An event portal not in view this near its generator's spot (metres) is not there (guess: in
/// view from 25 m at the least).
const PORTAL_SEEN_NEAR: f32 = 15.0;
/// What its starter's use is given to put an event's portal out before it is taken for shut (guess).
const PORTAL_COMES: Duration = Duration::from_secs(20);
/// A used starter's script plays before its StartEvent (EmoteManager.cs:1632): the Sealed Valve's
/// event came 6.2 s after its use (measured live); what asks the event waits this long.
const EVENT_LANDS: Duration = Duration::from_secs(8);

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

/// Each watcher's source: an event its heartbeat starts once the events it asks run.
fn watches() -> &'static [&'static Source] {
    static WATCHES: OnceLock<Vec<&'static Source>> = OnceLock::new();
    WATCHES.get_or_init(|| {
        let sources = ac_world::quests::sources().iter();
        sources.filter(|s| s.kind == Yield::Watch).collect()
    })
}

/// An NPC asked for an item, or an object used to start an event, `place` in words.
pub(super) fn ask_words(source: &Source, place: &str) -> String {
    match (&source.want, source.kind) {
        (Want::Event(e), Yield::Start) => {
            format!("use the {} in {place} to start {e}", source.name)
        }
        _ => format!("ask {} for it in {place}", source.name),
    }
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
        // Its watcher's count is over: what it put out is about (the boss before our eighth golem).
        self.autoplay.growth.event_deaths.remove(e);
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

    /// Every event a watcher has started: each of its events taken for on. Its heartbeat stops
    /// them as it starts its own (weenie 26575's EventSuccess set: StopEvent of the four summons
    /// and the feeder), so they are let go: a second run starts them again.
    pub(super) fn see_watches(&mut self, now: Instant) {
        let fired: Vec<&'static Source> = watches()
            .iter()
            .copied()
            .filter(|s| !s.needs.is_empty() && s.needs.iter().all(|n| self.is_event_on(n, now)))
            .collect();
        for s in fired {
            let Want::Event(e) = &s.want else { continue };
            tracing::info!("pursuit: {} has seen {:?} on", s.name, s.needs);
            self.note_event(e, now);
            for n in &s.needs {
                self.forget_event(n);
            }
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

    /// The event to start first for a journey to one of `grounds`: one past the fewest flags goes
    /// through a portal only that event puts out, not taken for on (Qin Xikit's Hidden Crown).
    pub(super) fn event_way(
        &mut self,
        grounds: &[(u32, ac_world::Place)],
        quests: &[String],
        now: Instant,
    ) -> Option<String> {
        let trip = grounds
            .iter()
            .filter_map(|g| {
                self.journey_plan_with(g.1, quests, Some(super::flag_way::FLAG_SECONDS))
            })
            .min_by(|a, b| a.seconds.total_cmp(&b.seconds))?;
        let shut = trip.steps.iter().find_map(|s| {
            let ac_world::trip::Step::Portal { mouth, exit, .. } = s else {
                return None;
            };
            ac_world::portals::event_portals()
                .iter()
                .find(|e| e.way.from_cell == mouth.cell && e.way.to_cell == exit.cell)
        })?;
        (!self.is_event_on(&shut.event, now)).then(|| shut.event.clone())
    }

    /// An event portal taken for out but not standing where its generator puts it, once its
    /// starter has had [`PORTAL_COMES`]: over, and so are the events its starter's use asked,
    /// which are started again (the heat events, put out by the Font Browerk Stopgap).
    pub(super) fn forget_unseen_portals(&mut self, now: Instant) {
        let Some(me) = self.my_position() else {
            return;
        };
        let shut: Vec<&'static ac_world::portals::EventPortal> = ac_world::portals::event_portals()
            .iter()
            .filter(|e| {
                let since = self.autoplay.growth.events_on.since(&e.event);
                since.is_some_and(|t| now.saturating_duration_since(t) >= PORTAL_COMES)
            })
            .filter(|e| me.distance(e.way.from) < PORTAL_SEEN_NEAR)
            .filter(|e| !self.is_class_in_view(e.wcid))
            .collect();
        for e in shut {
            self.forget_event(&e.event);
            let want = Want::Event(e.event.clone());
            let asked: Vec<String> = ac_world::quests::sources_of(&want)
                .flat_map(|s| s.needs.iter().cloned())
                .collect();
            for n in asked {
                self.forget_event(&n);
            }
        }
    }

    /// The events pursued quests start first, by id, less those taken for on: the rest kept.
    pub(super) fn event_ways(&mut self, now: Instant) -> BTreeMap<String, String> {
        let mut events = std::mem::take(&mut self.autoplay.growth.pursuit_events);
        events.retain(|_, e| !self.is_event_on(e, now));
        self.autoplay.growth.pursuit_events = events.clone();
        events
    }

    /// An event start `s` asks that was started less than [`EVENT_LANDS`] ago: its use waits.
    pub(super) fn unsettled_need<'a>(&self, s: &'a Source, now: Instant) -> Option<&'a str> {
        let on = &self.autoplay.growth.events_on;
        let fresh = |n: &String| {
            on.since(n)
                .is_some_and(|t| now.saturating_duration_since(t) < EVENT_LANDS)
        };
        s.needs.iter().find(|n| fresh(n)).map(String::as_str)
    }

    /// The event the give `s` starts, handed its item.
    pub(super) fn note_given(&mut self, s: &Source, now: Instant) {
        if let Want::Event(e) = &s.want {
            self.note_event(e, now);
        }
    }
}

impl super::Ask<'_> {
    /// Quest `id`'s step when only a portal an event puts out leads on and the event is not taken
    /// for on: the event obtained, its starters used ([`Client::event_way`]).
    pub(super) fn event_step(
        &self,
        id: &str,
    ) -> Option<(super::PursuitStep, Option<super::Hunting>)> {
        let e = self.event_ways.get(id).filter(|e| !(self.event_on)(e))?;
        let name = format!("the {e} event");
        Some(self.obtain(&[Want::Event(e.clone())], 1, &name, 0))
    }
}
