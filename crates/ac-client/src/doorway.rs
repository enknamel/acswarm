//! The doors by a shot's flight, and where a flight first enters one (see `Client::shot_clears`).

use glam::Vec3;

use crate::aim;
use crate::Client;

/// Doors further than this (metres) from every point of a flight are not looked at: a point is
/// within [`aim::PIECE`] of the next and a doorway is some three metres across.
const DOOR_NEAR: f32 = 4.0;

/// A closed door by a flight: where it stands, its turn and scale, and its Setup's box.
pub(crate) struct DoorBox {
    at: Vec3,
    turn: glam::Quat,
    scale: f32,
    lo: Vec3,
    hi: Vec3,
}

impl Client {
    /// The closed doors within [`DOOR_NEAR`] of `path`. The server flies a spell into one, though
    /// it lets a walk through (`ac_nav::obstacles::in_the_way`). A door fills its Setup's box, the
    /// doorway: its leaf's closed place is an animation frame the default placement lacks.
    fn doors_near(&mut self, path: &[Vec3]) -> Vec<DoorBox> {
        use ac_world::{object, object_desc_flags};
        let near: Vec<(u32, Vec3, glam::Quat, f32)> = self
            .world
            .objects
            .values()
            .filter(|o| o.object_desc_flags & object_desc_flags::DOOR != 0)
            .filter(|o| o.physics_state & object::PHYSICS_STATE_ETHEREAL == 0 && o.setup_id != 0)
            .filter_map(|o| {
                let p = o.position?;
                let at = ac_world::landblock_origin(p.cell) + p.local;
                path.iter().any(|q| q.distance(at) < DOOR_NEAR).then_some((
                    o.setup_id,
                    at,
                    p.rotation,
                    o.scale.max(1e-3),
                ))
            })
            .collect();
        let assets = self.assets.clone();
        near.into_iter()
            .filter_map(|(setup, at, turn, scale)| {
                let (lo, hi) = (*self.dodge.door_boxes.entry(setup).or_insert_with(|| {
                    ac_scene::collision::from_model(&assets, setup, glam::Mat4::IDENTITY)
                        .ok()?
                        .bounds()
                }))?;
                Some(DoorBox {
                    at,
                    turn,
                    scale,
                    lo,
                    hi,
                })
            })
            .collect()
    }

    /// Where `path` first enters a closed door, if it does.
    pub(crate) fn door_across(&mut self, path: &[Vec3]) -> Option<Vec3> {
        let doors = self.doors_near(path);
        path.windows(2).find_map(|w| {
            doors
                .iter()
                .filter_map(|d| {
                    let local = |q: Vec3| d.turn.inverse() * (q - d.at) / d.scale;
                    aim::segment_box(local(w[0]), local(w[1]), d.lo, d.hi)
                })
                .min_by(|a, b| a.total_cmp(b))
                .map(|t| w[0] + (w[1] - w[0]) * t)
        })
    }
}
