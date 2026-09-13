//! Whether a shot will get there.
//!
//! A spell or an arrow is a projectile the server launches and flies
//! with its physics (see `crate::dodge`): it strikes the first thing in
//! its way, and a wall, a fence or the brow of a hill between caster and
//! target takes the damage instead. Sight from the eyes to the chest is
//! not the path it takes. It starts in front of the caster at a height
//! set by the caster's own, aims at a height set by the target's, and an
//! arc or an arrow falls on the way. So the flight is worked out as the
//! server works it out (ACE `WorldObject_Magic.CalculateProjectileOrigins`
//! and `CalculateProjectileVelocity`, `Creature_Missile
//! .GetProjectileVelocity`) and followed through the world, before the
//! shot is spent.
//!
//! - a **bolt** (and a streak, a blast's middle, a volley's) spawns at
//!   two thirds of the caster's height and flies a straight line to two
//!   thirds of the target's;
//! - an **arc** spawns at the caster's full height and is lobbed at five
//!   sixths of the target's with a fixed ground speed, gravity deciding
//!   how high it goes: the slower it is, the higher the arc;
//! - an **arrow** spawns at 0.8454 of the archer's height and is shot at
//!   half the target's height at the launcher's speed, on the low one of
//!   the two arcs that get there -- and past its reach there is none.
//!
//! Each starts beside the shooter, its own radius and the shooter's out
//! in front, twice over; the stretch from the shooter's middle to there
//! counts as part of the flight. The path comes back as points no
//! further apart than [`PIECE`], so that the collision of the landblocks
//! under each pair covers the stretch between them and the ground is
//! looked at every few steps.

use glam::Vec3;

/// A spell projectile's start and aim, as a fraction of the caster's and
/// the target's height (ACE `ProjHeight`).
pub const SPELL_HEIGHT: f32 = 2.0 / 3.0;
/// An arc's aim, as a fraction of the target's height (ACE
/// `ProjHeightArc`). It starts at the caster's full height.
pub const ARC_AIM: f32 = 5.0 / 6.0;
/// An arrow's start, as a fraction of the archer's height (ACE
/// `ProjSpawnHeight`).
pub const MISSILE_SPAWN: f32 = 0.8454;
/// An arrow's aim, as a fraction of the target's height (one over ACE
/// `GetAimHeight`).
pub const MISSILE_AIM: f32 = 0.5;
/// A projectile's radius, when its own is not known: the wire names its
/// weenie, and the DAT spell table carries no projectile class.
pub const PROJECTILE_RADIUS: f32 = 0.15;
/// An arc's ground speed until one of this character's own has been
/// seen to fly (see `Client::learn_shot_speeds`).
pub const ARC_SPEED: f32 = 15.0;
/// The server's gravity for arcs and arrows, metres a second squared,
/// downward.
pub const FALL: f32 = 9.8;
/// Longest stretch of the path between two points.
pub const PIECE: f32 = 2.0;
/// How far below the ground surface a point has to be before the ground
/// is taken to have stopped it: the terrain is a plane between samples,
/// and a shot grazing a slope is not a shot into it.
pub const GROUND_GRAZE: f32 = 0.05;
/// Height taken for something whose shape is not known.
pub const UNKNOWN_HEIGHT: f32 = 1.8;
/// Radius taken for something whose shape is not known.
pub const UNKNOWN_RADIUS: f32 = 0.4;

/// Whether a spell of `meta_spell_type` is thrown as a projectile, and
/// so can strike something on the way and miss (ACE `Spell.IsProjectile`).
/// Every other spell lands, or is resisted, where it is cast.
pub fn flies(meta_spell_type: u32) -> bool {
    use ac_formats::spell_table::spell_type::*;
    matches!(
        meta_spell_type,
        PROJECTILE | LIFE_PROJECTILE | ENCHANTMENT_PROJECTILE
    )
}

/// The kinds of flight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shot {
    /// Straight, no gravity.
    Bolt,
    /// Lobbed at this ground speed.
    Arc { speed: f32 },
    /// Shot at this speed, falling.
    Missile { speed: f32 },
}

/// Something standing: where its feet are, how tall and how wide.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Body {
    pub feet: Vec3,
    pub height: f32,
    pub radius: f32,
}

impl Body {
    /// With the unknown filled in: a shape of nothing is a person's.
    pub fn new(feet: Vec3, height: f32, radius: f32) -> Self {
        Body {
            feet,
            height: if height > 0.1 { height } else { UNKNOWN_HEIGHT },
            radius: if radius > 0.01 {
                radius
            } else {
                UNKNOWN_RADIUS
            },
        }
    }
}

/// The path `shot` from `from` at `to` flies, as points in order from
/// launch to arrival, or `None` when it cannot get there at all (an
/// arrow past its reach).
pub fn flight(shot: Shot, from: Body, to: Body) -> Option<Vec<Vec3>> {
    let (launch, aim) = match shot {
        Shot::Bolt => (SPELL_HEIGHT, SPELL_HEIGHT),
        Shot::Arc { .. } => (1.0, ARC_AIM),
        Shot::Missile { .. } => (MISSILE_SPAWN, MISSILE_AIM),
    };
    let flat = (to.feet - from.feet).truncate();
    let ahead = flat.try_normalize().unwrap_or(glam::Vec2::Y);
    let out = 2.0 * (from.radius + PROJECTILE_RADIUS);
    let origin = from.feet + ahead.extend(0.0) * out + Vec3::Z * (from.height * launch);
    let end = to.feet + Vec3::Z * (to.height * aim);
    let points = match shot {
        Shot::Bolt => pieces(origin, end),
        Shot::Arc { speed } => {
            let x = (end - origin).truncate().length();
            if x < 1e-3 || speed <= 0.0 {
                return Some(pieces(origin, end));
            }
            // A fixed ground speed: the time is set by the distance, and
            // the climb is whatever falls back onto the aim in that time.
            let t = x / speed;
            let vz = (end.z - origin.z) / t + 0.5 * FALL * t;
            let v = ahead.extend(0.0) * speed + Vec3::Z * vz;
            falling(origin, v, t, x)
        }
        Shot::Missile { speed } => {
            let d = end - origin;
            let x = d.truncate().length();
            if x < 1e-3 || speed <= 0.0 {
                return Some(pieces(origin, end));
            }
            // Of the two angles that land it, the low one.
            let v2 = speed * speed;
            let root = v2 * v2 - FALL * (FALL * x * x + 2.0 * d.z * v2);
            if root < 0.0 {
                return None;
            }
            let angle = (v2 - root.sqrt()).atan2(FALL * x);
            let t = x / (angle.cos() * speed);
            let v = ahead.extend(0.0) * angle.cos() * speed + Vec3::Z * angle.sin() * speed;
            falling(origin, v, t, x)
        }
    };
    // From the shooter's middle: one standing against a wall spawns the
    // shot beyond it, and does not get to throw through it.
    let mut path = vec![from.feet + Vec3::Z * (from.height * launch)];
    path.extend(points);
    Some(path)
}

/// A straight line from `a` to `b` in pieces no longer than [`PIECE`].
fn pieces(a: Vec3, b: Vec3) -> Vec<Vec3> {
    let n = (a.distance(b) / PIECE).ceil().max(1.0) as usize;
    (0..=n).map(|i| a.lerp(b, i as f32 / n as f32)).collect()
}

/// A falling flight from `origin` at `v` for `t` seconds covering `x`
/// metres of ground, sampled no further apart than [`PIECE`] along it.
fn falling(origin: Vec3, v: Vec3, t: f32, x: f32) -> Vec<Vec3> {
    // The climb makes the path longer than the ground it covers.
    let along = x.max((v * t).length());
    let n = (along / PIECE).ceil().max(1.0) as usize;
    (0..=n)
        .map(|i| {
            let s = t * i as f32 / n as f32;
            origin + v * s - Vec3::Z * (0.5 * FALL * s * s)
        })
        .collect()
}

/// Whether a flight along `path` gets to its end: no stretch of it is
/// struck by what `strikes` says stops a segment, and no point of it is
/// under the ground `ground` gives at that spot (`None` where there is
/// no terrain, as in a dungeon).
pub fn clears(
    path: &[Vec3],
    mut strikes: impl FnMut(Vec3, Vec3) -> bool,
    mut ground: impl FnMut(f32, f32) -> Option<f32>,
) -> bool {
    if path
        .iter()
        .any(|p| ground(p.x, p.y).is_some_and(|z| p.z < z - GROUND_GRAZE))
    {
        return false;
    }
    !path.windows(2).any(|w| strikes(w[0], w[1]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(x: f32, y: f32) -> Body {
        Body::new(Vec3::new(x, y, 0.0), 1.8, 0.4)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.05
    }

    #[test]
    fn a_bolt_flies_straight_from_two_thirds_up_to_two_thirds_up() {
        let path = flight(Shot::Bolt, person(0.0, 0.0), person(0.0, 20.0)).unwrap();
        let (first, last) = (path[1], *path.last().unwrap());
        // Counted from the caster's middle.
        assert!(
            close(path[0].y, 0.0) && close(path[0].z, 1.2),
            "{}",
            path[0]
        );
        assert!(close(first.z, 1.2) && close(last.z, 1.2), "{first} {last}");
        // Out in front of the caster, by its radius and the bolt's twice.
        assert!(close(first.y, 1.1), "{first}");
        assert!(close(last.y, 20.0));
        assert!(path.windows(2).all(|w| w[0].distance(w[1]) <= PIECE + 1e-3));
        assert!(path.iter().all(|p| close(p.z, 1.2) && close(p.x, 0.0)));
    }

    #[test]
    fn an_arc_climbs_and_comes_down_at_five_sixths() {
        let path = flight(
            Shot::Arc { speed: 10.0 },
            person(0.0, 0.0),
            person(20.0, 0.0),
        )
        .unwrap();
        let last = *path.last().unwrap();
        assert!(close(last.z, 1.5) && close(last.x, 20.0), "{last}");
        assert!(close(path[0].z, 1.8));
        let top = path.iter().map(|p| p.z).fold(f32::MIN, f32::max);
        assert!(top > 5.0, "a slow arc goes high: {top}");
        // Faster is flatter.
        let fast = flight(
            Shot::Arc { speed: 30.0 },
            person(0.0, 0.0),
            person(20.0, 0.0),
        )
        .unwrap();
        let fast_top = fast.iter().map(|p| p.z).fold(f32::MIN, f32::max);
        assert!(fast_top < top);
    }

    #[test]
    fn an_arrow_takes_the_low_arc_and_lands_at_half_height() {
        let path = flight(
            Shot::Missile { speed: 20.0 },
            person(0.0, 0.0),
            person(0.0, 30.0),
        )
        .unwrap();
        let last = *path.last().unwrap();
        assert!(close(last.z, 0.9) && close(last.y, 30.0), "{last}");
        assert!(close(path[0].z, 1.8 * MISSILE_SPAWN));
        let top = path.iter().map(|p| p.z).fold(f32::MIN, f32::max);
        // The low arc at 20 m/s over 30 m peaks a few metres up, not
        // the tens the high one would.
        assert!(top > 1.6 && top < 8.0, "{top}");
    }

    #[test]
    fn an_arrow_past_its_reach_gets_nowhere() {
        // 20 m/s carries about 41 m.
        assert!(flight(
            Shot::Missile { speed: 20.0 },
            person(0.0, 0.0),
            person(0.0, 60.0)
        )
        .is_none());
    }

    #[test]
    fn a_wall_or_the_ground_in_the_way_stops_it() {
        let path = flight(Shot::Bolt, person(0.0, 0.0), person(0.0, 20.0)).unwrap();
        let open = |_: f32, _: f32| Some(0.0);
        assert!(clears(&path, |_, _| false, open));
        // A wall across y = 10.
        let wall = |a: Vec3, b: Vec3| a.y < 10.0 && b.y >= 10.0;
        assert!(!clears(&path, wall, open));
        // A rise between, higher than the bolt flies.
        let hill = |_: f32, y: f32| Some(if (8.0..12.0).contains(&y) { 2.0 } else { 0.0 });
        assert!(!clears(&path, |_, _| false, hill));
        // One it passes over.
        let bump = |_: f32, y: f32| Some(if (8.0..12.0).contains(&y) { 1.0 } else { 0.0 });
        assert!(clears(&path, |_, _| false, bump));
        // No terrain at all (a dungeon): only walls count.
        assert!(clears(&path, |_, _| false, |_, _| None));
    }

    #[test]
    fn an_arc_clears_a_wall_a_bolt_does_not() {
        let from = person(0.0, 0.0);
        let to = person(0.0, 20.0);
        // A three-metre wall across y = 10.
        let wall = |a: Vec3, b: Vec3| {
            let crosses = (a.y < 10.0) != (b.y < 10.0);
            crosses && a.lerp(b, (10.0 - a.y) / (b.y - a.y)).z < 3.0
        };
        let flat = |_: f32, _: f32| Some(0.0);
        let bolt = flight(Shot::Bolt, from, to).unwrap();
        assert!(!clears(&bolt, wall, flat));
        let arc = flight(Shot::Arc { speed: 10.0 }, from, to).unwrap();
        assert!(clears(&arc, wall, flat));
    }

    #[test]
    fn only_a_projectile_spell_flies() {
        // Flame Bolt, Martyr's Hecatomb: thrown.
        assert!(flies(2) && flies(10) && flies(15));
        // Imperil Other (enchantment), Harm Other (boost), Drain Health
        // Other (transfer): cast where the target stands.
        assert!(!flies(1) && !flies(3) && !flies(4));
    }

    #[test]
    fn an_unknown_shape_is_a_persons() {
        let b = Body::new(Vec3::ZERO, 0.0, 0.0);
        assert_eq!((b.height, b.radius), (UNKNOWN_HEIGHT, UNKNOWN_RADIUS));
    }
}
