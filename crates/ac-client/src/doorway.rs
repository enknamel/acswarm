//! Doorways and thrown attacks. A spell or an arrow is never thrown through, from or into a
//! doorway, open door or shut: the frame stops it. The caster steps out of one into the target's
//! room, throws from the target's room rather than across one, and waits a few metres back on its
//! own side for a target standing in one to come through. A swing goes through doors.

use std::time::{Duration, Instant};

use glam::Vec3;

use ac_nav::routing::Planned;
use ac_world::Place;

use crate::aim;
use crate::autoplay::Doing;
use crate::did::Did;
use crate::{Client, Goal};

/// Doors further than this (metres) from every point of a flight are not looked at: a point is
/// within [`aim::PIECE`] of the next and a doorway is some three metres across.
const DOOR_NEAR: f32 = 4.0;
/// A doorway's box is widened by this (metres) for the projectile's own size.
const SPELL_PAD: f32 = 0.3;
/// A doorway reaches this far (metres) either side of its door: Blargerton's casts from within 2 m
/// of a door in 0x0286 landed 4 times in 40, from further out 50 in 79 (live, 2026-11-01).
const DOORWAY_DEPTH: f32 = 2.0;
/// A target in a doorway is waited for at least this far back from it (metres).
const PULL_BACK: f32 = 6.0;
/// Waiting spots are looked for up to this much further out than [`PULL_BACK`]; stepping-out
/// spots up to this much past the doorway's edge.
const PULL_BAND: f32 = 4.0;
/// Spots tried for a path, nearest the character first.
const PULL_LOOKS: usize = 24;
/// A target that stays in its doorway this long is fought as any other.
const PULL_WAIT: Duration = Duration::from_secs(15);
/// Within this of the waiting spot is standing at it (metres).
const AT_SPOT: f32 = 1.0;
/// A walk to a spot by a doorway stops this near it (metres).
const AT_SPOT_STOP: f32 = 0.3;

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

    /// Whether the box's thin way, across the doorway, is its local x (else y).
    fn thin_x(&self) -> bool {
        let d = self.hi - self.lo;
        d.x < d.y
    }

    /// The doorway: the door's box, [`DOORWAY_DEPTH`] deep either side and padded for the
    /// projectile, in the door's own frame.
    fn padded(&self) -> (Vec3, Vec3) {
        let (pad, deep) = (SPELL_PAD / self.scale, DOORWAY_DEPTH / self.scale);
        let p = if self.thin_x() {
            Vec3::new(deep, pad, pad)
        } else {
            Vec3::new(pad, deep, pad)
        };
        (self.lo - p, self.hi + p)
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
        let (l, mid) = (self.local(p), (self.lo + self.hi) * 0.5);
        if self.thin_x() {
            l.x > mid.x
        } else {
            l.y > mid.y
        }
    }
}

impl Client {
    /// The doors, open or shut, within [`DOOR_NEAR`] of `path`. The server flies a spell into a
    /// doorway's frame, though it lets a walk through (`ac_nav::obstacles::in_the_way`). A door
    /// fills its Setup's box: its leaf's closed place is an animation frame the default lacks.
    fn doors_near(&mut self, path: &[Vec3]) -> Vec<DoorBox> {
        use ac_world::object_desc_flags;
        let near: Vec<(u32, Vec3, glam::Quat, f32)> = self
            .world
            .objects
            .values()
            .filter(|o| o.object_desc_flags & object_desc_flags::DOOR != 0 && o.setup_id != 0)
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

    /// Where `path` first enters a doorway, and its door.
    pub(crate) fn door_across(&mut self, path: &[Vec3]) -> Option<(Vec3, DoorBox)> {
        let doors = self.doors_near(path);
        path.windows(2).find_map(|w| {
            doors
                .iter()
                .filter_map(|d| d.entered(w[0], w[1]).map(|t| (t, *d)))
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(t, d)| (w[0] + (w[1] - w[0]) * t, d))
        })
    }

    /// Of `spots` to throw from, those on the far side of the doorway the shot from `me` crosses
    /// and out of it: in the target's room, where the shot need not cross it.
    pub(crate) fn keep_past_doorway(&mut self, door: &DoorBox, me: Vec3, spots: &mut Vec<Vec3>) {
        let mine = door.side(me);
        spots.retain(|s| door.side(*s) != mine && !door.holds(*s));
    }

    /// A shot at `target` (at `at`) kept from flying by a doorway either stands in. The target in
    /// one: walk back to a spot on our side, [`PULL_BACK`] out, and wait for it to come through
    /// (`a_target_in_a_doorway_is_waited_for_out_of_it`). We in one: step out into the target's
    /// room (`a_caster_in_a_doorway_steps_out_into_the_target_s_room`). True while doing either;
    /// false when neither stands in one, the target has stayed in one past [`PULL_WAIT`], or
    /// there is nowhere to go.
    pub(crate) fn clear_doorway(
        &mut self,
        target: u32,
        name: &str,
        at: Vec3,
        how: crate::dodge::How,
    ) -> bool {
        let now = Instant::now();
        let Some(me) = self.my_position() else {
            return false;
        };
        let doors = self.doors_near(&[me, at]);
        if let Some(door) = doors.iter().find(|d| d.holds(at)).copied() {
            return self.pull_from_doorway(target, name, &door, me, now);
        }
        self.dodge.pulling = None;
        let Some(door) = doors.iter().find(|d| d.holds(me)).copied() else {
            return false;
        };
        let Some(spot) = self.out_of_doorway(&door, me, (target, at), how) else {
            tracing::info!("range: standing in a doorway, with nowhere out of it toward {name}");
            return false;
        };
        if !self.walk_to_spot(spot) {
            return false;
        }
        self.dodge.approaching = Some(target);
        self.autoplay.say(
            Doing::Fighting,
            format!("stepping out of the doorway to cast at {name}"),
        );
        true
    }

    fn pull_from_doorway(
        &mut self,
        target: u32,
        name: &str,
        door: &DoorBox,
        me: Vec3,
        now: Instant,
    ) -> bool {
        let spot = match self.dodge.pulling {
            Some((g, since, spot)) if g == target => {
                if now.duration_since(since) > PULL_WAIT {
                    return false;
                }
                spot
            }
            _ => {
                let spot = self.waiting_spot(door, me);
                tracing::info!(
                    "range: {name} stands in a doorway; waiting for it {}",
                    spot.map_or("nowhere: no spot back from it".into(), |s| format!(
                        "at {:.1} {:.1} {:.1}, {:.1} m off",
                        s.at.x,
                        s.at.y,
                        s.at.z,
                        s.at.distance(me)
                    ))
                );
                self.dodge.pulling = Some((target, now, spot));
                spot
            }
        };
        let Some(spot) = spot else {
            return false;
        };
        if spot.at.distance(me) <= AT_SPOT {
            self.dodge.approaching = Some(target);
            self.autoplay.say(
                Doing::Fighting,
                format!("waiting for {name} to come out of the doorway"),
            );
        } else {
            if !self.walk_to_spot(spot) {
                return false;
            }
            self.dodge.approaching = Some(target);
            self.autoplay.say(
                Doing::Fighting,
                format!("backing away for {name} to come out of the doorway"),
            );
        }
        true
    }

    /// Walk to `spot` by the slot for the step running: false when no walk gets there.
    fn walk_to_spot(&mut self, spot: Place) -> bool {
        let goal = Goal::At {
            place: spot,
            within: AT_SPOT_STOP,
        };
        !matches!(
            self.go(self.step_owner(), goal, self.frame_at),
            Did::Blocked(_)
        )
    }

    /// The nearest place on our side of `door`, [`PULL_BACK`] out or more, that a path leads to.
    fn waiting_spot(&mut self, door: &DoorBox, me: Vec3) -> Option<Place> {
        let mine = door.side(me);
        self.nearest_reached(door.at, PULL_BACK + PULL_BAND, me, |s| {
            door.side(s) == mine && s.distance(door.at) >= PULL_BACK
        })
    }

    /// The nearest place out of `door`'s doorway on the side `target` (at `at`) is on, that the
    /// shot `how` flies from and a path leads to: from none, the doorway is not what stops it
    /// (a_caster_in_a_doorway_with_its_target_past_the_walls_is_not_held_at_the_door).
    fn out_of_doorway(
        &mut self,
        door: &DoorBox,
        me: Vec3,
        (target, at): (u32, Vec3),
        how: crate::dodge::How,
    ) -> Option<Place> {
        let theirs = door.side(at);
        let mut spots = self
            .standable_near(door.at, DOORWAY_DEPTH + PULL_BAND)
            .unwrap_or_default();
        // Out of it where the walk stops, AT_SPOT_STOP short: a spot 0.29 m off, just past the
        // frame, was "reached" from inside it and held Blargerton there (a_caster_just_inside_a_shop_door_steps_out).
        let stops_out = |s: Vec3| {
            let short = s + (me - s).normalize_or_zero() * AT_SPOT_STOP;
            s.distance(me) > AT_SPOT_STOP && !door.holds(s) && !door.holds(short)
        };
        spots.retain(|s| door.side(s.at) == theirs && stops_out(s.at));
        spots.sort_by(|a, b| a.at.distance(me).total_cmp(&b.at.distance(me)));
        let mine = self.world.player().map_or(0, |o| o.guid);
        let (shot, to) = (self.shot_for(how), self.body_of(target, at));
        for s in spots.into_iter().take(PULL_LOOKS) {
            let Some(path) = aim::flight(shot, self.body_of(mine, s.at), to) else {
                continue;
            };
            if !self.shot_flies(&path, how) {
                continue;
            }
            if s.at.distance(me) <= AT_SPOT
                || matches!(self.plan_walk(s, AT_SPOT), Planned::Route(_))
            {
                return Some(s);
            }
        }
        None
    }

    /// The standing place within `radius` of `around` that passes `keep`, nearest `me` of the
    /// first [`PULL_LOOKS`], that a path from `me` leads to.
    fn nearest_reached(
        &mut self,
        around: Vec3,
        radius: f32,
        me: Vec3,
        keep: impl Fn(Vec3) -> bool,
    ) -> Option<Place> {
        let mut spots = self.standable_near(around, radius).unwrap_or_default();
        spots.retain(|s| keep(s.at));
        spots.sort_by(|a, b| a.at.distance(me).total_cmp(&b.at.distance(me)));
        spots.into_iter().take(PULL_LOOKS).find(|s| {
            s.at.distance(me) <= AT_SPOT || matches!(self.plan_walk(*s, AT_SPOT), Planned::Route(_))
        })
    }
}

#[cfg(test)]
mod tests;
