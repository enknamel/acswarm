//! Doorways and thrown attacks. A spell or an arrow is never thrown through a doorway, open door or
//! shut: the frame stops it. The caster goes into the target's room instead, and a target standing
//! in a doorway is waited for a few metres back on the caster's side until it comes through.
//! A closed door also stops a swing's line; an open one does not (walks go straight through).

use std::time::{Duration, Instant};

use glam::Vec3;

use crate::aim;
use crate::autoplay::Doing;
use crate::Client;

/// Doors further than this (metres) from every point of a flight are not looked at: a point is
/// within [`aim::PIECE`] of the next and a doorway is some three metres across.
const DOOR_NEAR: f32 = 4.0;
/// A doorway's box is widened by this (metres) for the projectile's own size.
const SPELL_PAD: f32 = 0.3;
/// A target in a doorway is waited for at least this far back from it (metres).
const PULL_BACK: f32 = 6.0;
/// Waiting spots are looked for up to this much further out than [`PULL_BACK`].
const PULL_BAND: f32 = 4.0;
/// Waiting spots tried for a path, nearest the character first.
const PULL_LOOKS: usize = 24;
/// A target that stays in its doorway this long is fought as any other.
const PULL_WAIT: Duration = Duration::from_secs(15);
/// Within this of the waiting spot is standing at it (metres).
const AT_SPOT: f32 = 1.0;

/// A door by a flight: where it stands, its turn and scale, and its Setup's box (the doorway).
#[derive(Debug, Clone, Copy)]
pub(crate) struct DoorBox {
    at: Vec3,
    turn: glam::Quat,
    scale: f32,
    lo: Vec3,
    hi: Vec3,
}

impl DoorBox {
    fn local(&self, q: Vec3) -> Vec3 {
        self.turn.inverse() * (q - self.at) / self.scale
    }

    fn padded(&self) -> (Vec3, Vec3) {
        let pad = Vec3::splat(SPELL_PAD / self.scale);
        (self.lo - pad, self.hi + pad)
    }

    /// Where the segment `a`-`b` first enters the doorway, as a fraction along it.
    fn entered(&self, a: Vec3, b: Vec3) -> Option<f32> {
        let (lo, hi) = self.padded();
        aim::segment_box(self.local(a), self.local(b), lo, hi)
    }

    /// Whether `p` stands in the doorway.
    fn holds(&self, p: Vec3) -> bool {
        let (lo, hi) = self.padded();
        let l = self.local(p);
        l.cmpge(lo).all() && l.cmple(hi).all()
    }

    /// Which side of the doorway `p` is on: across the box's thin way.
    fn side(&self, p: Vec3) -> bool {
        let (l, mid, d) = (self.local(p), (self.lo + self.hi) * 0.5, self.hi - self.lo);
        if d.x < d.y {
            l.x > mid.x
        } else {
            l.y > mid.y
        }
    }
}

impl Client {
    /// The doors within [`DOOR_NEAR`] of `path`: shut ones only, or open ones too (`open`). The
    /// server flies a spell into a shut one, though it lets a walk through
    /// (`ac_nav::obstacles::in_the_way`). A door fills its Setup's box, the doorway: its leaf's
    /// closed place is an animation frame the default placement lacks.
    fn doors_near(&mut self, path: &[Vec3], open: bool) -> Vec<DoorBox> {
        use ac_world::{object, object_desc_flags};
        let near: Vec<(u32, Vec3, glam::Quat, f32)> = self
            .world
            .objects
            .values()
            .filter(|o| o.object_desc_flags & object_desc_flags::DOOR != 0 && o.setup_id != 0)
            .filter(|o| open || o.physics_state & object::PHYSICS_STATE_ETHEREAL == 0)
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

    /// Where `path` first enters a door's doorway, and the door: shut doors only, or open ones
    /// too (`open`, for a spell or an arrow).
    pub(crate) fn door_across(&mut self, path: &[Vec3], open: bool) -> Option<(Vec3, DoorBox)> {
        let doors = self.doors_near(path, open);
        path.windows(2).find_map(|w| {
            doors
                .iter()
                .filter_map(|d| d.entered(w[0], w[1]).map(|t| (t, *d)))
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(t, d)| (w[0] + (w[1] - w[0]) * t, d))
        })
    }

    /// Of `spots` to throw from, those on the far side of the doorway the shot from `me` crosses:
    /// in the target's room, where the shot need not cross it.
    pub(crate) fn keep_past_doorway(&mut self, door: &DoorBox, me: Vec3, spots: &mut Vec<Vec3>) {
        let mine = door.side(me);
        spots.retain(|s| door.side(*s) != mine && !door.holds(*s));
    }

    /// A target standing in a doorway at `at` is not shot through its frame: walk back to a spot
    /// on our side, [`PULL_BACK`] out, and wait there for it to come through. True while doing
    /// so; false when it is not in a doorway, has stayed in one past [`PULL_WAIT`], or there is
    /// nowhere to wait (`a_target_in_a_doorway_is_waited_for_out_of_it`).
    pub(crate) fn pull_from_doorway(&mut self, target: u32, name: &str, at: Vec3) -> bool {
        let now = Instant::now();
        let Some(door) = self
            .doors_near(&[at], true)
            .into_iter()
            .find(|d| d.holds(at))
        else {
            self.dodge.pulling = None;
            return false;
        };
        let Some((me, block)) = self
            .player
            .as_ref()
            .map(|p| (p.world_position(), p.cell & 0xFFFF_0000))
        else {
            return false;
        };
        let spot = match self.dodge.pulling {
            Some((g, since, spot)) if g == target => {
                if now.duration_since(since) > PULL_WAIT {
                    return false;
                }
                spot
            }
            _ => {
                let spot = self.waiting_spot(&door, me, block);
                tracing::info!(
                    "range: {name} stands in a doorway; waiting for it {}",
                    spot.map_or("nowhere: no spot back from it".into(), |s| format!(
                        "at {:.1} {:.1} {:.1}, {:.1} m off",
                        s.x,
                        s.y,
                        s.z,
                        s.distance(me)
                    ))
                );
                self.dodge.pulling = Some((target, now, spot));
                spot
            }
        };
        let Some(spot) = spot else {
            return false;
        };
        self.dodge.approaching = Some(target);
        if spot.distance(me) <= AT_SPOT {
            self.follow = None;
            self.autoplay.say(
                Doing::Fighting,
                format!("waiting for {name} to come out of the doorway"),
            );
        } else {
            self.head_for(spot, 0.3, name);
            self.autoplay.say(
                Doing::Fighting,
                format!("backing away for {name} to come out of the doorway"),
            );
        }
        true
    }

    /// The nearest place on our side of `door`, [`PULL_BACK`] out or more, that a path leads to.
    fn waiting_spot(&mut self, door: &DoorBox, me: Vec3, block: u32) -> Option<Vec3> {
        let assets = self.assets.clone();
        let pl = self.player.as_mut()?;
        let mine = door.side(me);
        let mut spots = pl.standable_near(&assets, block, door.at, PULL_BACK + PULL_BAND);
        spots.retain(|s| door.side(*s) == mine && s.distance(door.at) >= PULL_BACK);
        spots.sort_by(|a, b| a.distance(me).total_cmp(&b.distance(me)));
        spots.into_iter().take(PULL_LOOKS).find(|s| {
            s.distance(me) <= AT_SPOT || pl.find_path(&assets, block, me, *s, block).is_some()
        })
    }
}

#[cfg(test)]
mod tests;
