//! Every body of ours kept between sessions, one file per character under its server, so a body
//! left by a crash or the death limit's log-off is still gone back for; and that log-off itself.

use std::time::Instant;

use super::Body;
use crate::autoplay::Doing;
use crate::Client;

/// The least a body of ours lasts, seconds: ACE rots it after max(3600, level x 300) (`Corpse.cs:110`).
const ROT_LEAST: u64 = 3600;
/// Seconds a body lasts per level of the character (`Corpse.cs:110`).
const ROT_PER_LEVEL: u64 = 300;

/// Unix seconds now.
pub(super) fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Seconds a body of a character of `level` lasts before ACE rots it.
pub(super) fn rot_secs(level: i32) -> u64 {
    (u64::try_from(level).unwrap_or(1) * ROT_PER_LEVEL).max(ROT_LEAST)
}

impl Client {
    /// Where this character's bodies are written: beside its quest log.
    fn bodies_path(&self) -> Option<std::path::PathBuf> {
        self.quest_file(".bodies.json")
    }

    /// Read back the bodies left in earlier sessions, at login; those rotted since are dropped.
    pub(crate) fn load_bodies(&mut self) {
        let Some(path) = self.bodies_path() else {
            return;
        };
        match ac_store::read_json::<Vec<Body>>(&path) {
            Ok(Some(mut bodies)) => {
                let now = unix_now();
                bodies.retain(|b| b.rots > now);
                if !bodies.is_empty() {
                    tracing::info!("recovery: {} body(ies) still out from before", bodies.len());
                }
                let rec = &mut self.autoplay.recovery;
                rec.bodies = bodies;
                rec.bodies_changed = true;
            }
            Ok(None) => {}
            Err(e) => tracing::warn!("recovery: reading {}: {e}", path.display()),
        }
    }

    /// Write the bodies out when they have changed: a crash must not lose one.
    pub(crate) fn save_bodies(&mut self) {
        if !self.autoplay.recovery.bodies_changed {
            return;
        }
        let Some(path) = self.bodies_path() else {
            return;
        };
        self.autoplay.recovery.bodies_changed = false;
        let bodies = &self.autoplay.recovery.bodies;
        if let Err(e) = ac_store::write_json_atomic(&path, bodies, ac_store::Visibility::Normal) {
            tracing::warn!("recovery: writing {}: {e}", path.display());
        }
    }

    /// The death limit reached: log off at the lifestone, the bodies kept for next time, and
    /// disconnect for good once the server has done it (never reconnected: `Ending::Quit`).
    pub(crate) fn log_off_for_deaths(&mut self, deaths: u32, now: Instant) {
        if self.quit_after_log_off {
            return;
        }
        let cfg = &self.autoplay.config.survive;
        let why = format!(
            "{deaths} deaths in {:.0} minutes: logging off",
            cfg.death_limit_minutes
        );
        tracing::warn!("recovery: {why}");
        self.autoplay.say(Doing::Recovering, why.clone());
        self.autoplay.note(why, now);
        self.save_bodies();
        self.quit_after_log_off = true;
        self.log_off(now);
    }

    /// Once the server has logged the character off for the death limit, end the session.
    pub(crate) fn tick_death_quit(&mut self, now: Instant) {
        if self.quit_after_log_off && self.logged_off && self.ending().is_none() {
            self.disconnect(now);
        }
    }
}
