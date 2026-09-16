use crate::{player, Client};

/// Where a follower is heading and how close it stops (see
/// `Client::follow`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Follow {
    pub target: glam::Vec3,
    pub stop: f32,
}

/// How far above or below a goal still counts as standing at it.
///
/// A goal is a point on a floor, and arriving was judged on the flat:
/// how far away it was on the map, height ignored. Sent to Asenala, who
/// keeps a shop on the upper floor of a house in Holtburg, a character
/// walked a hundred and forty metres, stopped six millimetres from her
/// on the map and three metres below her on the ground floor, and stood
/// there. It had arrived, by the only test it had.
///
/// A doorsill, a slope or a step puts a pace of height between two
/// places on the same floor, so the tolerance has to allow that; a
/// storey is three metres and must not pass.
pub(crate) const SAME_FLOOR: f32 = 2.0;

impl Client {
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
