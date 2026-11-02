use ac_nav::routing::{NoWay, Planned};
use ac_world::Place;
use glam::{Vec2, Vec3};

use crate::{player, Client};

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

    /// Where the character can stand within `radius` (metres, flat) of `around`, by the region
    /// planner (see [`Pathfinder::standable_near`](crate::pathfinder::Pathfinder::standable_near)).
    pub(crate) fn standable_near(&mut self, around: Vec3, radius: f32) -> Vec<Place> {
        let Some(pl) = self.player.as_ref() else {
            return Vec::new();
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

    /// Whether a walk can get from the character to within `within` metres of `at` (standing in
    /// `cell`), by the region planner, whose route checks its last leg too: false only on its "no
    /// way"; one past the region is the journey planner's, and one still being built is let be.
    pub(crate) fn has_way_to(&mut self, at: Vec3, cell: u32, within: f32) -> bool {
        match self.plan_walk(Place { at, cell }, within) {
            Planned::Route(_) | Planned::Building(_) | Planned::NoWay(NoWay::OutOfRegion) => true,
            Planned::NoWay(why) => {
                tracing::debug!("travel: no way to {at:?}: {why:?}");
                false
            }
        }
    }

    /// The height of `cell`'s floor at `at`: a dungeon's levels stack, so a spot on the map alone
    /// names no floor (a_walk_that_gets_nowhere_is_given_up). `None` when the cell has none there.
    pub(crate) fn floor_in_cell(&self, at: Vec2, cell: u32) -> Option<f32> {
        let rooms = self.assets.block_collision(cell & 0xFFFF_0000).ok()?;
        let floors = rooms.world.floors_at_xy(at.x, at.y);
        floors.into_iter().find(|f| f.1 == cell).map(|f| f.0)
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

    /// A walk whose goal a teleport, recall or portal left out of the region is let go: walked at,
    /// it was a dungeon spot 34 km from the Holtburg lifestone (a_walk_left_behind_by_a_recall_is_let_go).
    pub(crate) fn drop_stale_walk(&mut self) {
        let Some(place) = self.going.as_ref().and_then(|g| g.goal.place()) else {
            return;
        };
        if self.player.is_some() && !self.is_in_region(place) {
            self.stop_going(None, "left out of the region");
            tracing::info!("travel: the walk's goal was left behind; letting go of it");
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
