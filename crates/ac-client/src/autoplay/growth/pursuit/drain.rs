//! Starters whose use drains their user ([`Source::drain`]): the Eye of the Watcher cuts every
//! skill and attribute to 1% for 60 s, so no heal lands, and drains 35 health each 10.5 s for
//! 292 s wherever its user goes (two at once killed Blargerton in 33 s). One is used at a time,
//! at full health, once the last one's drain has run out.

use std::time::{Duration, Instant};

use ac_world::quests::Source;

use crate::Client;

/// A drain runs this much longer than its emotes' delays: each cast's motion adds to them (11.2 s
/// a cast live against 10.5 s of delays).
const DRAIN_SLACK: f32 = 1.25;
/// How long a wait for full health asks the heals to fill the bar, renewed each tick it waits.
const HEAL_ASKED: Duration = Duration::from_secs(2);

impl Client {
    /// Why the use of `s` waits at `now`: a drain of ours still running, or health not full
    /// (the heals asked to fill it); `None` when it may be used.
    pub(super) fn drain_wait(&mut self, s: &Source, now: Instant) -> Option<String> {
        s.drain?;
        if let Some(left) = self
            .autoplay
            .growth
            .drained_until
            .and_then(|t| t.checked_duration_since(now))
        {
            return Some(format!(
                "waiting out the last drain before {}, {} s",
                s.name,
                left.as_secs()
            ));
        }
        let (have, most) = self.vital_now(0);
        if have >= most {
            return None;
        }
        self.autoplay.heal_full_until = Some(now + HEAL_ASKED);
        Some(format!("healing to full before {}", s.name))
    }

    /// The use of `s` done at `now`: its drain runs on us till it is out.
    pub(super) fn note_drain(&mut self, s: &Source, now: Instant) {
        let Some(secs) = s.drain else {
            return;
        };
        let runs = Duration::from_secs_f32(secs * DRAIN_SLACK);
        tracing::info!(
            "pursuit: {} drains us for about {} s",
            s.name,
            runs.as_secs()
        );
        self.autoplay.growth.drained_until = Some(now + runs);
    }
}
