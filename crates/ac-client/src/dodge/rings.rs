//! Stepping out of a ring before it reaches us. A monster's cast is shown to all in view as its
//! forward motion, `CastSpell` at speed 2 (`Monster_Magic.cs:210-223`) or, on human animations,
//! the spell's gestures (`Monster_Magic.cs:231-263`); its projectiles leave once that has played
//! (`Monster_Magic.cs:160-173`). The spell is not named, so a creature with a ring in its book
//! (`ac_world::ring_casters`) is taken to be casting its widest.

use std::time::{Duration, Instant};

use ac_world::ring_casters::{widest_ring, Ring};
use glam::{Vec2, Vec3};

use crate::autoplay::Doing;
use crate::Client;

/// Seconds from a cast motion to the launch, least: 0.23 s, CastSpell link of table 0x09000194 at speed 2.
pub const WINDUP_LEAST: f32 = 0.2;
/// The same, most: 3.04 s (0x09000001, stance 0x80000041); 0.87-0.96 s for most combat stances.
pub const WINDUP_MOST: f32 = 3.1;
/// Metres a ring reaches past its range: spawned 1.2 x (1 m caster + 0.5 m projectile) out, plus both radii (`WorldObject_Magic.cs:1576-1594`).
pub const RING_MARGIN: f32 = 3.0;
/// The slowest run, m/s: run rate 1 (`player::run_rate(0)`) times the 4 m/s run cycle.
pub const RUN_LEAST: f32 = 4.0;
/// Seconds kept in hand before the ring could reach us: a tick or two, and the turn.
pub const LEAVE_SLACK: f32 = 0.3;
/// How long a cast of ours takes from its send to its answer, at most (guess: 1.5-2 s live).
pub const OWN_CAST: Duration = Duration::from_secs(2);
/// Metres past the reach the spot stands.
pub const PAST: f32 = 0.5;
/// Metres run at most to get out.
pub const LONGEST: f32 = 20.0;
/// Directions tried round the character.
pub const DIRECTIONS: usize = 16;
/// A partial step must gain this much, metres, on standing still.
pub const LEAST_GAIN: f32 = 0.5;

/// A creature seen starting a cast that may be a ring.
#[derive(Debug, Clone, Copy)]
pub struct RingCast {
    /// When its cast motion was first seen.
    pub seen: Instant,
    /// A step out was taken (or found no room) for this cast: one is enough.
    pub stepped: bool,
}

/// Whether a forward motion (low 16 bits) is a monster casting: `CastSpell`, the Magic* cast
/// gestures, or the MagicPowerUp windups (`MotionCommand.cs:50-64, 118-127, 218, 307-316`).
pub fn is_cast_motion(forward: u32) -> bool {
    matches!(forward & 0xFFFF, 0xD3 | 0x2B..=0x39 | 0x6F..=0x78 | 0x12B..=0x134)
}

/// Seconds left, `elapsed` after the cast motion was seen `d` metres flat off its caster, before a
/// run at [`RUN_LEAST`] no longer clears `ring`: before it reaches us or catches us on the way out.
pub fn leave_within(elapsed: f32, d: f32, ring: Ring) -> f32 {
    let launch = WINDUP_LEAST - elapsed;
    let speed = ring.speed.max(0.1);
    let arrives = launch + (d - RING_MARGIN).max(0.0) / speed;
    let reach = ring.range + RING_MARGIN;
    let at_reach = launch + ring.range / speed;
    arrives.min(at_reach - (reach - d).max(0.0) / RUN_LEAST) - LEAVE_SLACK
}

/// Clear of `rings` (flat centre, reach): of [`DIRECTIONS`] runs from `me` that `clear` passes, the
/// shortest past every reach; else a shortened run's end deepest out, gaining [`LEAST_GAIN`].
pub fn escape_spot(
    me: Vec3,
    rings: &[(Vec2, f32)],
    mut clear: impl FnMut(Vec3) -> bool,
) -> Option<Vec3> {
    let flat = me.truncate();
    let worst = |p: Vec2| {
        rings
            .iter()
            .map(|(c, reach)| p.distance(*c) - reach)
            .fold(f32::INFINITY, f32::min)
    };
    let mut out: Option<(f32, Vec3)> = None;
    let mut deepest: Option<(f32, Vec3)> = None;
    for i in 0..DIRECTIONS {
        let a = i as f32 / DIRECTIONS as f32 * std::f32::consts::TAU;
        let dir = Vec2::new(a.cos(), a.sin());
        // How far along `dir` the run leaves each ring: the far root of |flat + dir k - c| = reach.
        let need = rings
            .iter()
            .map(|(c, reach)| {
                let off = flat - *c;
                let b = dir.dot(off);
                let q = off.length_squared() - reach * reach;
                (-b + (b * b - q).max(0.0).sqrt()).max(0.0)
            })
            .fold(0.0f32, f32::max)
            + PAST;
        for k in [need, need * 0.75, need * 0.5, need * 0.25] {
            if k > LONGEST || k < 0.5 {
                continue;
            }
            let spot = me + (dir * k).extend(0.0);
            if !clear(spot) {
                continue;
            }
            let margin = worst(spot.truncate());
            if margin >= 0.0 {
                if out.is_none_or(|(best, _)| k < best) {
                    out = Some((k, spot));
                }
            } else if deepest.is_none_or(|(best, _)| margin > best) {
                deepest = Some((margin, spot));
            }
            break;
        }
    }
    out.map(|(_, s)| s).or_else(|| {
        deepest
            .filter(|(m, _)| *m >= worst(flat) + LEAST_GAIN)
            .map(|(_, s)| s)
    })
}

impl Client {
    /// Notes each ring caster in view that has started a cast, and forgets one whose ring would
    /// have burst by now.
    pub(crate) fn watch_ring_casts(&mut self, now: Instant) {
        let casting: Vec<u32> = self
            .world
            .objects
            .values()
            .filter(|o| o.alive_or_unknown() && is_cast_motion(o.motion.forward))
            .filter(|o| widest_ring(o.weenie_class_id).is_some())
            .map(|o| o.guid)
            .collect();
        for g in casting {
            self.dodge.ring_casts.entry(g).or_insert(RingCast {
                seen: now,
                stepped: false,
            });
        }
        let world = &self.world;
        self.dodge.ring_casts.retain(|g, cast| {
            let Some(ring) = world
                .objects
                .get(g)
                .filter(|o| o.alive_or_unknown())
                .and_then(|o| widest_ring(o.weenie_class_id))
            else {
                return false;
            };
            let over = WINDUP_MOST + ring.range / ring.speed.max(0.1);
            now.duration_since(cast.seen).as_secs_f32() < over
        });
    }

    /// Out of the reach of every ring being cast that we stand in, by a straight run (the
    /// dodge's `dodge_to`). A cast of ours that lands first is let land.
    pub(crate) fn step_out_of_rings(&mut self, now: Instant, me: Vec3, cell: u32) -> bool {
        let mut rings: Vec<(u32, Vec2, f32, f32)> = Vec::new();
        for (g, cast) in &self.dodge.ring_casts {
            if cast.stepped || !self.can_be_hurt_by(Some(*g)) {
                continue;
            }
            let Some((at, ring)) = self
                .world
                .objects
                .get(g)
                .and_then(|o| Some((o.world_pos()?, widest_ring(o.weenie_class_id)?)))
            else {
                continue;
            };
            let d = at.truncate().distance(me.truncate());
            let reach = ring.range + RING_MARGIN;
            // Rings fly flat from the caster's middle: one a floor away passes over or under.
            if d >= reach || (at.z - me.z).abs() > 4.0 {
                continue;
            }
            let elapsed = now.duration_since(cast.seen).as_secs_f32();
            rings.push((*g, at.truncate(), reach, leave_within(elapsed, d, ring)));
        }
        let Some(&(first, ..)) = rings.iter().min_by(|a, b| a.3.total_cmp(&b.3)) else {
            return false;
        };
        let leave_in = rings.iter().map(|r| r.3).fold(f32::INFINITY, f32::min);
        let name = self
            .world
            .name_of(first)
            .unwrap_or("a creature")
            .to_string();
        // An NPK's cast is not broken by a move (`Player_Magic.cs:871-877, 1338-1346`, PKs only),
        // but the run pulls against the server's turn to the target (`Player_Magic.cs:803-830`).
        if let Some(sent) = self.autoplay.cast_sent {
            let lands_in = (sent + OWN_CAST)
                .saturating_duration_since(now)
                .as_secs_f32();
            if lands_in > 0.0 && lands_in <= leave_in {
                tracing::debug!("dodge: letting our cast land, then out of {name}'s ring");
                return false;
            }
        }
        for r in &rings {
            if let Some(cast) = self.dodge.ring_casts.get_mut(&r.0) {
                cast.stepped = true;
            }
        }
        let circles: Vec<(Vec2, f32)> = rings.iter().map(|r| (r.1, r.2)).collect();
        let assets = &self.assets;
        let Some(pl) = self.player.as_mut() else {
            return false;
        };
        let Some(spot) = escape_spot(me, &circles, |to| !pl.line_blocked(assets, cell, me, to))
        else {
            self.autoplay
                .note(format!("no room to step out of {name}'s ring"), now);
            return false;
        };
        let run = spot.truncate().distance(me.truncate());
        tracing::info!(
            "dodge: {name} is casting, its ring reaches us in {leave_in:.2} s; running {run:.1} m to {:.1} {:.1}",
            spot.x,
            spot.y
        );
        self.dodge_to = Some((spot, now + Duration::from_secs_f32(run / RUN_LEAST + 0.5)));
        self.steering.reset();
        self.autoplay
            .say(Doing::Dodging, format!("stepping out of {name}'s ring"));
        true
    }
}

#[cfg(test)]
mod tests;
