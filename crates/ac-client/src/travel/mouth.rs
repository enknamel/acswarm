//! The portal step at its mouth: waited in, jumped into, and stepped back from when the server
//! turns the journey away as teleported too recently.

use std::time::{Duration, Instant};

use glam::Vec2;

use super::{PORTAL_COOLDOWN, PORTAL_GIVE_UP, PORTAL_STEP_BACK, TELEPORTED_TOO_RECENTLY};
use crate::Client;

impl Client {
    /// A portal turned the journey away as teleported too recently: step back off its mouth, to
    /// where the walk in last stood, and walk in again once the server's wait is over, the
    /// give-up clock started afresh.
    pub(crate) fn portal_too_soon(&mut self, code: u32, now: Instant) {
        if code != TELEPORTED_TOO_RECENTLY || self.travel_portal().is_none() {
            return;
        }
        let Some((me, forward)) = self
            .player
            .as_ref()
            .map(|pl| (pl.world_position(), pl.forward().truncate()))
        else {
            return;
        };
        let trail = &self.travel.state.trail;
        let walked = trail
            .iter()
            .rev()
            .find(|p| p.at.distance(me) >= PORTAL_STEP_BACK);
        let back = match walked
            .or(trail.first())
            .filter(|p| p.at.distance(me) >= 1.0)
        {
            Some(p) => *p,
            None => {
                let spot = me.truncate() - forward.normalize_or(Vec2::Y) * PORTAL_STEP_BACK;
                let assets = self.assets.clone();
                let floor = self
                    .player
                    .as_mut()
                    .and_then(|pl| pl.ground_height(&assets, spot.x, spot.y, me.z));
                self.place_at(floor.map_or(me, |z| spot.extend(z)))
            }
        };
        tracing::info!("travel: teleported too recently; stepping back to try again");
        self.travel.state.too_soon = Some((now + PORTAL_COOLDOWN, back));
        self.travel.state.portal_since = None;
        self.walk_afresh();
    }

    /// Drop the stretch's walk and the steering's memory of it: the step back and the walk in
    /// again are walks of their own, no stall or refusal of one counted against the other.
    pub(super) fn walk_afresh(&mut self) {
        if self.going.as_ref().is_some_and(|g| g.journey) {
            self.going = None;
        }
        self.steering.reset();
    }

    /// Standing at the portal's mouth and still here: jump into it after a while, and after
    /// [`PORTAL_GIVE_UP`] leave it out. False once it is left out.
    pub(super) fn wait_at_mouth(&mut self, now: Instant) -> bool {
        let since = *self.travel.state.portal_since.get_or_insert(now);
        if now.duration_since(since) > PORTAL_GIVE_UP {
            if let Some((name, mouth)) = self.travel_portal() {
                let level = self.world.stats.level.max(1) as u32;
                let why = ac_world::portals::near(mouth, 3.0)
                    .first()
                    .and_then(|p| p.refusal(level))
                    .unwrap_or_else(|| "would not take us".into());
                tracing::warn!("travel: the portal {name:?} {why}; going another way");
                self.travel.refused.push(mouth);
            }
            self.travel.replan_due = true;
            return false;
        }
        // A few portals sit above the ground and have to be jumped into; try that while we wait.
        let hop_due = self
            .travel
            .state
            .last_hop
            .is_none_or(|t| now.duration_since(t) > Duration::from_secs(2));
        if now.duration_since(since) > Duration::from_secs(3) && hop_due {
            self.travel.state.last_hop = Some(now);
            tracing::info!("travel: jumping into the portal");
            self.jump(1.0);
        }
        true
    }
}
