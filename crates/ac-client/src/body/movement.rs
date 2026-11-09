use ac_nav::routing::{NoWay, Planned};
use ac_world::Place;
use glam::Vec3;

use crate::{player, Client};

/// A goal's floor reaches this far over or under it (metres): a sill, slope or step is within
/// it, a storey (3 m) is not.
pub(crate) const SAME_FLOOR: f32 = 2.0;

impl Client {
    /// The region planner's walk from where the character stands to within `within` (metres, 3D)
    /// of `to` (see [`Pathfinder::plan`](crate::pathfinder::Pathfinder::plan)).
    pub(crate) fn plan_walk(&mut self, to: Place, within: f32) -> Planned {
        let Some(pl) = self.player.as_ref() else {
            return Planned::NoWay(NoWay::Stranded);
        };
        let me = Place {
            at: pl.world_position(),
            cell: pl.cell,
        };
        let cap = pl.capsule();
        self.pathfinder.plan(me, to, within, cap)
    }

    /// Where the character can stand within `radius` (metres, flat) of `around`; `None` while the
    /// region planner builds it (see [`Pathfinder::standable_near`](crate::pathfinder::Pathfinder::standable_near)).
    pub(crate) fn standable_near(&mut self, around: Vec3, radius: f32) -> Option<Vec<Place>> {
        let Some(pl) = self.player.as_ref() else {
            return Some(Vec::new());
        };
        let me = Place {
            at: pl.world_position(),
            cell: pl.cell,
        };
        let around = Place {
            at: around,
            cell: pl.landblock(),
        };
        let cap = pl.capsule();
        self.pathfinder.standable_near(me, around, radius, cap)
    }

    /// Let go of the walk in the slot and the journey under way, saying `why`.
    pub(crate) fn drop_walk(&mut self, why: &str) {
        let walking = self.going.is_some();
        self.stop_going(None, why);
        let journey = self.traveling() || self.travel_goal_xy().is_some();
        if journey {
            self.end_trip();
        }
        self.autoplay.resume_trip = None;
        if walking || journey {
            tracing::info!("travel: {why}; letting go of the walk");
        }
    }

    /// Jump on the next tick with `power` 0..=1 (a script's or bot's
    /// jump; the window charges one by holding the key). Capped by the
    /// stamina left; nothing happens in the air.
    /// Run this many times faster than the Run skill allows. Anything
    /// over 1 is the client's own doing; the server accepts it (it
    /// refuses a move only when it is both more than 50 m from the last
    /// and more than a landblock away), but other players' clients still
    /// animate this character at its proper rate, so past about 2 it
    /// looks like skating to them.
    pub fn set_speed_boost(&mut self, boost: f32) {
        self.speed_boost = boost.clamp(0.25, 4.0);
        if let Some(pl) = self.player.as_mut() {
            pl.speed_boost = self.speed_boost;
        }
    }

    /// A full jump rises this many metres whatever the Jump skill
    /// allows (0 leaves it to the skill), up to the server's tolerance
    /// ([`player::MAX_JUMP_HEIGHT`]).
    pub fn set_jump_height(&mut self, metres: f32) {
        self.jump_height = metres.clamp(0.0, player::MAX_JUMP_HEIGHT);
        if let Some(pl) = self.player.as_mut() {
            pl.jump_height = self.jump_height;
        }
    }

    /// The jump being charged right now (the jump key held), worked out
    /// to its landing spot; `None` when no jump is being charged.
    pub fn jump_preview(&mut self) -> Option<player::JumpPreview> {
        let pl = self.player.as_mut()?;
        let power = pl.jump_charge()?;
        pl.preview_jump(&self.assets, power)
    }

    /// Fly through walls and floors, or stop and drop to the ground; see
    /// [`player::Player::set_noclip`] for what the server allows.
    /// Returns false when there is no character yet, or when the
    /// movement rules refuse to fly here ([`Client::movement_rules`]).
    pub fn set_noclip(&mut self, on: bool) -> bool {
        match self.player.as_mut() {
            Some(pl) => pl.set_noclip(on),
            None => false,
        }
    }

    /// What the movement rules allow against the server we are
    /// connected to.
    pub fn movement_limits(&self) -> player::MovementLimits {
        self.movement_rules.limits()
    }

    pub fn noclip(&self) -> bool {
        self.player.as_ref().is_some_and(|p| p.noclip)
    }

    /// Flying by hand or after a flying leader: straight at the goal, nothing planned. A route's
    /// own flown leg over a trap plate is not this; the route is followed through it.
    pub fn flies_freely(&self) -> bool {
        self.noclip() && !self.steering.flying
    }

    /// Flying was asked for and the movement rules refused it (see
    /// [`player::Player::noclip_refused`]).
    pub fn noclip_refused(&self) -> bool {
        self.player.as_ref().is_some_and(|p| p.noclip_refused())
    }

    /// Nudge the jump being charged (see [`player::Player::adjust_charge`]).
    pub fn adjust_jump_charge(&mut self, delta: f32) {
        if let Some(pl) = self.player.as_mut() {
            pl.adjust_charge(delta);
        }
    }

    pub fn jump(&mut self, power: f32) {
        self.pending_jump = Some(power.clamp(0.0, 1.0));
    }

    /// The jump charge while the key is held, as power 0..=1.
    pub fn jump_charge(&self) -> Option<f32> {
        self.player.as_ref().and_then(|p| p.jump_charge())
    }
}
