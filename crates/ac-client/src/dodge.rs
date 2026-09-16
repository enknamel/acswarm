//! Getting out of a projectile's way.
//!
//! A war spell is not a hit roll: the server makes a projectile
//! (`SpellProjectile` in ACE), aims it at where the target stands and
//! flies it there with its physics, and the damage is done when it
//! collides with the target's capsule. Arrows, bolts and thrown weapons
//! are launched the same way (`Creature_Missile`), as the ammunition
//! itself. The client is told once, on the projectile's creation, where
//! it is and how fast it moves, and hears nothing more until it lands;
//! the projectile's whole path is that one line, or that one arc when
//! the `Gravity` physics bit says it falls. So a character that steps a
//! couple of metres sideways before the bolt arrives is simply not hit,
//! and does not have to run from a caster the way it would from a
//! swordsman.
//!
//! The spell shapes all come down to single projectiles on the wire:
//!
//! - a **bolt** or **streak** is one object flying a straight line at
//!   the target (a streak is only faster and lower);
//! - an **arc** is lobbed: it carries the `Gravity` bit and its path is
//!   ballistic, which changes where it is at any moment but not the
//!   ground track, so the sidestep is the same;
//! - a **blast** or **volley** is a fan of objects, a **wall** a row of
//!   them side by side, a **ring** one from the caster in every
//!   direction: each arrives as its own object with its own velocity,
//!   and only the ones whose lines pass through us are threats;
//! - a **strike** starts beside the target and flies at it;
//! - an **arrow** or **thrown weapon** flies like a bolt, under gravity.
//!
//! Each tick `Client::autoplay_dodge` looks at every missile in view,
//! works out where it is now from where it started and how long ago,
//! and asks [`threat`] whether it will pass within [`MISS_BY`] of us in
//! the next [`HORIZON`]. For the soonest one it measures the room to
//! either side of the projectile's path with the walker's own
//! collision, picks a side with [`sidestep`], and hands the step to
//! `Client::tick_player` as `dodge_to`, which outranks every other
//! movement goal until the step is done. Everything else the character
//! was doing (casting, swinging) resumes after.
//!
//! Our own projectiles start beside us and fly away, so they never
//! come toward us and are never dodged. A fellow's are another matter:
//! nine characters standing a metre apart and shooting the same
//! creature put bolts across each other all afternoon, and a fifth of
//! one run's dodges were for a teammate's spell.
//!
//! Those are demoted rather than dropped, because neither half of the
//! obvious reading is true. A fellow's bolt cannot hurt us -- ACE
//! refuses the damage in `Player.CheckPKStatusVsTarget`, where a
//! non-PK on either side ends it -- but it is not harmless: the
//! collision is resolved first (`SpellProjectile.OnCollideObject` calls
//! `ProjectileImpact` before it asks about PK), so standing in a
//! fellow's line destroys his spell and nobody is any better off.
//! Stepping aside genuinely saves it. What is wrong is the price: a
//! dodge takes the legs and the tick with them, ahead of the healing
//! and everything else, and nothing that cannot hurt us is worth an
//! interrupted cast or a broken swing. So [`can_hurt_us`] is read at
//! intake and a fellow's bolt is stepped out of the way of only when
//! the moment is going spare (see `Client::free_to_step_aside`).
//!
//! A projectile is dodged whether or not it could reach us: the wire
//! names its weenie, not its spell, and the DAT spell table carries no
//! projectile class, so a bolt cannot be tied to its spell's range;
//! the server lets one fly until it hits something or times out.
//!
//! The second half of this module is the other side of range: how far
//! our own attacks reach, and closing in before one is thrown.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use glam::{Vec2, Vec3};

use crate::aim::{self, Shot};
use crate::autoplay::Doing;
use crate::Client;

/// A projectile arriving later than this is left for a later tick: it
/// may yet be for someone else, and there is time.
pub const HORIZON: f32 = 1.5;
/// How far a sidestep goes at most. A bolt has to miss the capsule by
/// its own radius and ours, about a metre; a bit over twice that leaves
/// room for a late start.
pub const STEP: f32 = 2.5;
/// A side with less room than this is not stepped toward.
pub const LEAST_ROOM: f32 = 1.0;
/// How far out the room on each side is measured.
pub const LOOK: f32 = 3.0;
/// How long a sidestep is given before the other goals get the legs
/// back.
pub const LASTS: Duration = Duration::from_millis(600);
/// Close enough to the sidestep's end to call it done.
pub const STOP: f32 = 0.3;
/// How far a projectile's path has to pass from us to count as a miss:
/// our capsule's radius (0.4), the projectile's (up to 0.5), and a
/// margin for the server's view of where we stand.
pub const MISS_BY: f32 = 1.2;
/// The server's gravity (ACE `PhysicsGlobals.Gravity`), metres a
/// second squared, for things carrying the `Gravity` bit.
pub const GRAVITY: f32 = -9.8;
/// A creature this close to where a projectile started is taken to
/// have cast it.
const CASTER_WITHIN: f32 = 3.0;

/// A projectile in flight, as first seen.
#[derive(Debug, Clone, Copy)]
pub struct Track {
    /// Where it was when it appeared.
    pub origin: Vec3,
    /// Metres a second, world axes.
    pub velocity: Vec3,
    /// Downward acceleration: [`GRAVITY`] for an arc or an arrow, 0
    /// for a bolt.
    pub gravity: f32,
    /// When it appeared.
    pub seen: Instant,
    /// Who it looks to have come from (see `Client::fired_by`): a
    /// guess from where it appeared, not the wire's word.
    pub from: Option<u32>,
    /// Whether it could hurt this character if it landed (see
    /// [`can_hurt_us`]), read once when it appeared. A fellow's spell
    /// cannot, and is worth a great deal less of the character's time.
    pub can_hurt: bool,
    /// A sidestep was taken for it: one is enough.
    pub dodged: bool,
}

/// Whether a projectile from a creature whose description flags are
/// `caster` can hurt a character whose flags are `mine` (both
/// `ac_world::object_desc_flags`).
///
/// A monster's can. Another player's cannot, unless both sides are
/// player killers: ACE's `Player.CheckPKStatusVsTarget` refuses the
/// damage the moment either side is a non-PK, and the fellowship a
/// character hunts with is nine non-PKs shooting past each other.
///
/// Free is the exception, and it runs the other way: ACE short-circuits
/// to "allowed" the moment *either* side is Free
/// (`Player_Combat.CheckPKStatusVsTarget`, which returns no error at
/// all for it), so a Free player's spell lands on an ordinary character
/// for full damage. Either side carrying that bit is a threat.
///
/// Read the other way round when there is any doubt. The caster is a
/// guess (see `Client::fired_by`), and the two PK statuses the server
/// keeps are finer than these bits -- a PK and a PK Lite cannot touch
/// each other either -- so anything that looks at all like a threat is
/// taken for one and dodged as it always was.
pub fn can_hurt_us(caster: u32, mine: u32) -> bool {
    use ac_world::object_desc_flags as flags;
    if caster & flags::PLAYER == 0 {
        return true;
    }
    if (caster | mine) & flags::FREE_PK_STATUS != 0 {
        return true;
    }
    let pk = |f: u32| f & (flags::PLAYER_KILLER | flags::PK_LITE_STATUS) != 0;
    pk(caster) && pk(mine)
}

impl Track {
    /// Where the projectile is now.
    pub fn at(&self, now: Instant) -> Vec3 {
        let t = now.duration_since(self.seen).as_secs_f32();
        self.origin + self.velocity * t + Vec3::new(0.0, 0.0, 0.5 * self.gravity * t * t)
    }
}

/// The dodging rules' state.
#[derive(Debug, Default)]
pub struct State {
    /// Missiles in view, by guid.
    pub tracks: HashMap<u32, Track>,
    /// The target being closed on for an attack (see
    /// `Client::autoplay_approach`), whose `follow` goal is ours to
    /// clear.
    pub approaching: Option<u32>,
    /// How fast this character's own spells have been seen to fly, by
    /// spell: an arc's height depends on it (see `crate::aim`).
    pub shot_speeds: HashMap<u32, f32>,
    /// The attack spell last cast, when, and the missiles already in the
    /// air then, until its own projectile is seen.
    pub fired: Option<(u32, Instant, Vec<u32>)>,
    /// The sidestep in hand is a courtesy to a fellow rather than a
    /// dodge: it saves his spell and nothing of ours, so it is given up
    /// the moment there is something better to do (see
    /// `Client::free_to_step_aside`). Read only while `dodge_to` is
    /// set, and written afresh with every step.
    pub courtesy: bool,
}

/// A projectile leaving this long after a cast is not taken for it.
const FIRED_WITHIN: Duration = Duration::from_secs(5);

// ---- Reach: how far our own attacks go ---------------------------------
//
// The server refuses an attack from too far off (`MissileOutOfRange`)
// and nothing happens, so the fight rules close in first. The limits
// are the server's (ACE):
//
// - a spell reaches `BaseRangeConstant + skill * BaseRangeMod` metres,
//   capped at the outdoor radar range, with `skill` the school's
//   trained level (initial level plus ranks: no attribute formula and
//   no buffs, as the original client's `DetermineSpellRange` had it)
//   (`Player_Magic.VerifySpellRange`);
// - a bow, crossbow or thrown weapon reaches `MaximumVelocity^2 /
//   9.8` metres, capped at 85 yards (`Creature_Missile
//   .GetMaxMissileRange`), the velocity being the launcher's own
//   property with a default when it has none;
// - a melee swing needs the target within `MeleeDistance`, or within
//   `StickyDistance` and in sight, failing which the server walks us
//   there itself (`Player_Melee`): nothing for the rules to do.

/// Furthest any spell reaches (ACE `MaxRadarRange_Outdoors`).
pub const MAX_SPELL_RANGE: f32 = 75.0;
/// Furthest any missile flies, 85 yards (ACE `MissileRangeCap`).
pub const MAX_MISSILE_RANGE: f32 = 85.0 / 1.094;
/// The launch speed assumed for a launcher without one of its own
/// (ACE `DefaultMaxVelocity`).
pub const DEFAULT_MAX_VELOCITY: f32 = 20.0;
/// Appraisal float property carrying a launcher's launch speed (ACE
/// `PropertyFloat.MaximumVelocity`).
pub const MAXIMUM_VELOCITY: u32 = 26;
/// A swing lands within this (ACE `MeleeDistance`).
pub const MELEE_REACH: f32 = 0.6;
/// Or within this when the target is in sight (ACE `StickyDistance`).
pub const STICKY_REACH: f32 = 4.0;
/// An attack goes out only from within this fraction of its reach,
/// so a target edging away does not put the next one out of range.
pub const WITHIN: f32 = 0.85;
/// Without a line of sight the walk goes for the target itself, stopping
/// this close: round the corner it will be in sight long before that.
const NO_SIGHT_STOP: f32 = 2.5;

/// How far a spell with `base_constant` and `base_mod` reaches for a
/// caster whose school skill (trained level, no buffs) is `skill`.
pub fn spell_range(base_constant: f32, base_mod: f32, skill: u32) -> f32 {
    (base_constant + skill as f32 * base_mod).min(MAX_SPELL_RANGE)
}

/// How far a launcher with launch speed `max_velocity` shoots.
pub fn missile_range(max_velocity: f32) -> f32 {
    (max_velocity * max_velocity * 0.102_040_82).min(MAX_MISSILE_RANGE)
}

/// The kinds of attack, for [`Client::attack_range`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum How {
    /// Casting this spell.
    Spell(u32),
    /// Shooting or throwing with the wielded launcher.
    Missile,
    /// Swinging.
    Melee,
}

impl Client {
    /// How far the attack `how` reaches from where we stand, by the
    /// server's rules (see the notes above): the spell's range at our
    /// trained skill, the wielded launcher's range (the default speed
    /// when it has not been appraised), or a swing's sticky reach.
    pub fn attack_range(&self, how: How) -> f32 {
        match how {
            How::Spell(id) => {
                let table = self.assets.spell_table().ok();
                let Some(sp) = table.as_ref().and_then(|t| t.get(id)) else {
                    return MAX_SPELL_RANGE;
                };
                let skill = Client::school_skill(sp.school)
                    .and_then(|s| self.world.stats.skill(s))
                    .map_or(0, |s| s.init_level + s.ranks as u32);
                spell_range(sp.base_range_constant, sp.base_range_mod, skill)
            }
            How::Missile => missile_range(self.launcher_speed()),
            How::Melee => STICKY_REACH,
        }
    }

    /// The wielded launcher's launch speed, the default when it has not
    /// been appraised.
    fn launcher_speed(&self) -> f32 {
        self.wielded_missile_weapon()
            .and_then(|g| self.appraisals.get(&g))
            .and_then(|a| a.float(MAXIMUM_VELOCITY))
            .map_or(DEFAULT_MAX_VELOCITY, |v| v as f32)
    }

    /// How the attack `how` flies (see `crate::aim`). A spell is an arc
    /// when the table says its projectile does not track and it is named
    /// one; every other spell, and a swing's line, flies straight.
    pub fn shot_for(&self, how: How) -> Shot {
        match how {
            How::Missile => Shot::Missile {
                speed: self.launcher_speed(),
            },
            How::Spell(id) => {
                let table = self.assets.spell_table().ok();
                let arc = table.as_ref().and_then(|t| t.get(id)).is_some_and(|sp| {
                    sp.bitfield & ac_formats::spell_table::flags::NON_TRACKING_PROJECTILE != 0
                        && sp.name.split_whitespace().any(|w| w == "Arc")
                });
                if arc {
                    Shot::Arc {
                        speed: self
                            .dodge
                            .shot_speeds
                            .get(&id)
                            .copied()
                            .unwrap_or(aim::ARC_SPEED),
                    }
                } else {
                    Shot::Bolt
                }
            }
            How::Melee => Shot::Bolt,
        }
    }

    /// Whether spell `id` is thrown as a projectile (see [`aim::flies`]);
    /// one the table does not have is taken to be.
    pub fn spell_flies(&self, id: u32) -> bool {
        self.assets
            .spell_table()
            .ok()
            .and_then(|t| t.get(id).map(|sp| aim::flies(sp.meta_spell_type)))
            .unwrap_or(true)
    }

    /// The shape of object `guid` standing at `feet`: its Setup's height
    /// and radius at its scale, a person's when that is not known.
    fn body_of(&self, guid: u32, feet: Vec3) -> aim::Body {
        let (height, radius) = self
            .world
            .objects
            .get(&guid)
            .and_then(|o| {
                let s = self.assets.setup(o.setup_id).ok()?;
                Some((s.height * o.scale, s.radius * o.scale))
            })
            .unwrap_or((0.0, 0.0));
        aim::Body::new(feet, height, radius)
    }

    /// Whether the attack `how`, thrown from where this character stands,
    /// gets to `target` without striking a wall or the ground on the way
    /// (see `crate::aim`). With nothing to go on it is taken to.
    pub(crate) fn shot_clears(&mut self, target: u32, how: How) -> bool {
        let Some(me) = self.my_position() else {
            return true;
        };
        let Some(at) = self.world.objects.get(&target).and_then(|o| o.world_pos()) else {
            return true;
        };
        let mine = self.world.player().map_or(0, |o| o.guid);
        let (from, to) = (self.body_of(mine, me), self.body_of(target, at));
        let Some(path) = aim::flight(self.shot_for(how), from, to) else {
            return false;
        };
        let assets = self.assets.clone();
        self.player
            .as_mut()
            .is_none_or(|pl| pl.flies_clear(&assets, &path))
    }

    /// An attack spell has just been cast: its projectile, when it
    /// leaves, tells how fast that spell flies.
    pub(crate) fn note_fired(&mut self, spell: u32, now: Instant) {
        let before = self
            .world
            .objects
            .values()
            .filter(|o| o.is_missile())
            .map(|o| o.guid)
            .collect();
        self.dodge.fired = Some((spell, now, before));
    }

    /// Read the speed of the spell last cast off the projectile that has
    /// just left this character: new since the cast, beside us, and
    /// moving away. The spell table does not say how fast a spell flies,
    /// and an arc's height depends on it.
    fn learn_shot_speeds(&mut self, now: Instant) {
        let Some((spell, at, before)) = self.dodge.fired.clone() else {
            return;
        };
        if now.duration_since(at) > FIRED_WITHIN {
            self.dodge.fired = None;
            return;
        }
        let Some(me) = self.my_position() else {
            return;
        };
        let speed = self
            .world
            .objects
            .values()
            .filter(|o| o.is_missile() && o.parent.is_none() && !before.contains(&o.guid))
            .find_map(|o| {
                let p = o.world_pos()?;
                let flat = o.velocity.truncate();
                (p.distance(me) <= CASTER_WITHIN
                    && flat.length() > 0.5
                    && flat.dot((p - me).truncate()) > 0.0)
                    .then(|| flat.length())
            });
        if let Some(speed) = speed {
            if self.dodge.shot_speeds.insert(spell, speed).is_none() {
                tracing::info!("aim: spell {spell} flies at {speed:.1} m/s");
            }
            self.dodge.fired = None;
        }
    }

    /// Close on `target` until it is within [`WITHIN`] of the reach of
    /// `how` and the shot is clear, walking after it like a leader. True
    /// while still too far, when the attack has to wait; false once in
    /// reach (or with nowhere to go), the walk called off.
    pub(crate) fn autoplay_approach(&mut self, target: u32, name: &str, how: How) -> bool {
        let Some((me, cell)) = self.player.as_ref().map(|p| (p.world_position(), p.cell)) else {
            return false;
        };
        let Some(at) = self.world.objects.get(&target).and_then(|o| o.world_pos()) else {
            self.stop_approaching();
            return false;
        };
        // Nearer than its reach, when nothing landed from further off.
        let range = self.attack_range(how);
        let range = self
            .autoplay
            .closing_on(target)
            .map_or(range, |cap| range.min(cap));
        // Within reach is not enough: the spell or the arrow has to get
        // there, and one that would strike a wall or the ground on the
        // way is not thrown (see `crate::aim`). Walk round (the steering
        // finds the way) until the shot is clear and in reach.
        let seen = self.shot_clears(target, how);
        let stop = if seen { range * WITHIN } else { NO_SIGHT_STOP };
        if at.distance(me) <= stop && seen {
            self.stop_approaching();
            return false;
        }
        if self.dodge.approaching != Some(target) {
            // Where we stand as well as how far off it is: a walk that
            // goes wrong from here can then be put on the map.
            if seen {
                tracing::info!(
                    "range: {name} is {:.1} m off, reach {range:.1} m: closing to {stop:.1} m, from {:.1} {:.1} {:.1} in {cell:#010x}",
                    at.distance(me),
                    me.x,
                    me.y,
                    me.z
                );
            } else {
                tracing::info!(
                    "range: no clear shot at {name} ({:.1} m off) from {:.1} {:.1} {:.1} in {cell:#010x}: moving",
                    at.distance(me),
                    me.x,
                    me.y,
                    me.z
                );
            }
            self.interrupt_travel("closing on a target");
            self.steering.reset();
        }
        // Nothing to hit is ever a journey away.
        //
        // A fight is with what is in front of the character. Something
        // named as a target from half a world away is not there at all:
        // it is an object left over from somewhere the character has
        // since left -- the Academy, most often, whose creatures stay
        // in the world model after the way back has closed -- and a
        // character that sets off for one spends its life planning a
        // trip it can never make.
        if at.distance(me) > crate::travel::WALKABLE {
            tracing::info!(
                "range: {name} is not here any more ({:.0} m off)",
                at.distance(me)
            );
            self.stop_approaching();
            return false;
        }
        self.dodge.approaching = Some(target);
        // Closing on something to hit it is a goal like any other, so
        // it is named to the travel system and not steered by hand.
        self.head_for(at, stop, name);
        self.autoplay.say(
            Doing::Fighting,
            if seen {
                format!("closing on {name}")
            } else {
                format!("getting {name} in sight")
            },
        );
        true
    }

    /// Done closing on a target: the walk it set is taken back.
    fn stop_approaching(&mut self) {
        if self.dodge.approaching.take().is_some() {
            self.follow = None;
            self.steering.reset();
        }
    }
}

/// When a projectile at `pos` moving at `velocity`, falling at
/// `gravity` (0 for one that flies straight), will pass within
/// `radius` of `me` horizontally and at something like body height:
/// seconds from now until it is closest, or `None` when it will miss,
/// is already past, or is moving away. Gravity bends the path only in
/// z, so the ground track and the moment of closest approach are the
/// same as for the straight line; it decides whether the thing is at
/// our height when it gets here.
pub fn threat(pos: Vec3, velocity: Vec3, gravity: f32, me: Vec3, radius: f32) -> Option<f32> {
    let v = Vec2::new(velocity.x, velocity.y);
    let speed2 = v.length_squared();
    if speed2 < 0.25 {
        return None;
    }
    let rel = Vec2::new(me.x - pos.x, me.y - pos.y);
    let t = rel.dot(v) / speed2;
    if t <= 0.0 {
        return None;
    }
    let closest = Vec2::new(pos.x, pos.y) + v * t;
    if closest.distance(Vec2::new(me.x, me.y)) > radius {
        return None;
    }
    // Aimed at two thirds of the target's height; anything from the
    // knees to well over the head counts, a bolt at the feet is a
    // miss the server also sees.
    let z = pos.z + velocity.z * t + 0.5 * gravity * t * t;
    if z < me.z - 1.0 || z > me.z + 4.0 {
        return None;
    }
    Some(t)
}

/// Unit vectors to the left and to the right of a projectile's path,
/// flat.
pub fn sides(velocity: Vec3) -> (Vec3, Vec3) {
    let d = Vec2::new(velocity.x, velocity.y).normalize_or(Vec2::Y);
    let left = Vec3::new(-d.y, d.x, 0.0);
    (left, -left)
}

/// How far `me` already stands to the left of the path through `pos`
/// along `velocity` (negative: to the right).
pub fn lean(pos: Vec3, velocity: Vec3, me: Vec3) -> f32 {
    let (left, _) = sides(velocity);
    let rel = me - pos;
    rel.x * left.x + rel.y * left.y
}

/// Where to step to get out of a projectile's way: sideways to its
/// path, on the side we already lean to when it has [`LEAST_ROOM`]
/// (stepping across the path would put us on it as the bolt arrives),
/// else on the roomier side; as far as [`STEP`] or the room, whichever
/// is less. `room_left` and `room_right` are the clear metres to
/// either side, `lean` our present offset from the path (see [`lean`]).
/// `None` when neither side has room enough.
pub fn sidestep(
    me: Vec3,
    velocity: Vec3,
    room_left: f32,
    room_right: f32,
    lean: f32,
) -> Option<Vec3> {
    let (left, right) = sides(velocity);
    // Square on the path, the roomier side is the near one; off it by
    // a hand's breadth or more, the side we are already on is.
    let leaning_left = if lean.abs() < 0.2 {
        room_left >= room_right
    } else {
        lean > 0.0
    };
    let (near, near_room, far, far_room) = if leaning_left {
        (left, room_left, right, room_right)
    } else {
        (right, room_right, left, room_left)
    };
    let (dir, room) = if near_room >= LEAST_ROOM {
        (near, near_room)
    } else if far_room >= LEAST_ROOM {
        (far, far_room)
    } else {
        return None;
    };
    Some(me + dir * room.min(STEP))
}

impl Client {
    /// Step out of the way of any projectile about to hit us. True
    /// while a sidestep is under way, when nothing else should take
    /// the legs. Runs before everything else in `tick_autoplay`.
    pub(crate) fn autoplay_dodge(&mut self, now: Instant) -> bool {
        // A target being closed on that has died or gone is not walked
        // after any further (the fight rules only speak up about one
        // they are still fighting).
        if let Some(g) = self.dodge.approaching {
            let alive = self
                .world
                .objects
                .get(&g)
                .is_some_and(|o| o.health.unwrap_or(1.0) > 0.0);
            if !alive {
                self.stop_approaching();
            }
        }
        self.learn_shot_speeds(now);
        if !self.autoplay.config.survive.dodge {
            return false;
        }
        let Some((me, cell)) = self.player.as_ref().map(|p| (p.world_position(), p.cell)) else {
            return false;
        };
        // Flying, there is no floor to step along.
        if self.noclip() {
            return false;
        }
        // Forget projectiles that have landed or gone, and note the
        // ones that have just appeared.
        let world = &self.world;
        self.dodge
            .tracks
            .retain(|g, _| world.objects.get(g).is_some_and(|o| !o.no_draw));
        // Who fired it is worked out here, once, while the projectile
        // is still sitting at its caster's feet: a moment later the
        // caster has moved and the huddle around it has changed, and
        // the answer decides how much of the character's time the
        // thing is worth.
        let fresh: Vec<u32> = self
            .world
            .objects
            .values()
            .filter(|o| o.is_missile() && !o.no_draw && o.parent.is_none())
            .filter(|o| o.velocity.length_squared() >= 0.25)
            .filter(|o| !self.dodge.tracks.contains_key(&o.guid))
            .map(|o| o.guid)
            .collect();
        for guid in fresh {
            let Some((origin, velocity, falls, name, wcid, state)) =
                self.world.objects.get(&guid).and_then(|o| {
                    Some((
                        o.world_pos()?,
                        o.velocity,
                        o.physics_state & ac_world::object::PHYSICS_STATE_GRAVITY != 0,
                        o.name.clone(),
                        o.weenie_class_id,
                        o.physics_state,
                    ))
                })
            else {
                continue;
            };
            let from = self.fired_by(origin);
            let can_hurt = self.can_be_hurt_by(from);
            let whose = from
                .and_then(|g| self.world.objects.get(&g))
                .map(|o| format!(", from {}", o.name))
                .unwrap_or_default();
            tracing::info!(
                "dodge: projectile {name} ({wcid}, {guid:#010x}) at {:.1} {:.1} {:.1} moving {:.1} {:.1} {:.1} ({:.1} m/s{}), state {state:#x}, {:.1} m off{whose}{}",
                origin.x,
                origin.y,
                origin.z,
                velocity.x,
                velocity.y,
                velocity.z,
                velocity.length(),
                if falls { ", falling" } else { "" },
                origin.distance(me),
                if can_hurt { "" } else { " (cannot hurt us)" },
            );
            self.dodge.tracks.insert(
                guid,
                Track {
                    origin,
                    velocity,
                    gravity: if falls { GRAVITY } else { 0.0 },
                    seen: now,
                    from,
                    can_hurt,
                    dodged: false,
                },
            );
        }
        // A sidestep under way is kept up, barring two things.
        if let Some((goal, _)) = self.dodge_to {
            // It was only a courtesy to a fellow and the moment it was
            // taken in has stopped being spare: a heal that has come
            // due, or a cast gone out, must not wait out somebody
            // else's bolt.
            if self.dodge.courtesy && !self.free_to_step_aside(now) {
                self.dodge_to = None;
                self.dodge.courtesy = false;
                return false;
            }
            // Or its end is itself in the path of something that can
            // hurt us. Only of something that can: starting again
            // because a fellow's bolt crosses the spot would be the
            // same mistake one level down.
            let in_the_way = self.dodge.tracks.values().any(|t| {
                t.can_hurt
                    && threat(t.at(now), t.velocity, t.gravity, goal, MISS_BY)
                        .is_some_and(|eta| eta <= HORIZON)
            });
            if !in_the_way {
                return true;
            }
            self.dodge_to = None;
        }
        if self.dodge.tracks.is_empty() {
            return false;
        }
        // The soonest projectile on its way to us that can hurt us --
        // and only failing that, and only with a moment to spare, the
        // soonest of a fellow's. A spell that would land on us always
        // outranks one that would merely die on us, however much
        // sooner the fellow's arrives.
        let soonest = self.soonest(now, me, true).or_else(|| {
            self.free_to_step_aside(now)
                .then(|| self.soonest(now, me, false))
                .flatten()
        });
        let Some((guid, eta)) = soonest else {
            return false;
        };
        let track = self.dodge.tracks[&guid];
        let name = self
            .world
            .objects
            .get(&guid)
            .map(|o| o.name.clone())
            .unwrap_or_else(|| "a spell".into());
        let caster = track
            .from
            .and_then(|g| self.world.name_of(g).map(str::to_string));
        // Room to either side, measured with the walker's own collision
        // half a metre at a time.
        let (left, right) = sides(track.velocity);
        let assets = &self.assets;
        let Some(pl) = self.player.as_mut() else {
            return false;
        };
        let mut room = |dir: Vec3| -> f32 {
            let mut clear = 0.0f32;
            let mut k = 0.5f32;
            while k <= LOOK + 1e-3 {
                if pl.line_blocked(assets, cell, me, me + dir * k) {
                    break;
                }
                clear = k;
                k += 0.5;
            }
            clear
        };
        let mut room_left = room(left);
        let mut room_right = room(right);
        // Nothing clear on either side, not even half a metre, is not
        // a corridor: it is the walk test failing where we stand (a
        // character just placed by the server, its feet not yet
        // settled on the ground it was given). A step taken blind
        // costs nothing if a wall stops it; standing still costs the
        // hit.
        let blind = room_left < 0.5 && room_right < 0.5;
        if blind {
            room_left = LOOK;
            room_right = LOOK;
        }
        let lean = lean(track.at(now), track.velocity, me);
        if let Some(t) = self.dodge.tracks.get_mut(&guid) {
            t.dodged = true;
        }
        let from = caster
            .as_ref()
            .map(|c| format!(" from {c}"))
            .unwrap_or_default();
        let Some(goal) = sidestep(me, track.velocity, room_left, room_right, lean) else {
            self.autoplay.note(
                format!(
                    "no room to dodge {name}{from}: {room_left:.1} m left, {room_right:.1} m right"
                ),
                now,
            );
            return false;
        };
        tracing::info!(
            "dodge: {name}{from}{} arrives in {eta:.2} s, lean {lean:.2}, room {room_left:.1} left {room_right:.1} right{}, stepping to {:.1} {:.1}",
            if track.can_hurt { "" } else { ", which cannot hurt us," },
            if blind { " (blind: the walk test failed both ways)" } else { "" },
            goal.x,
            goal.y
        );
        self.dodge_to = Some((goal, now + LASTS));
        self.dodge.courtesy = !track.can_hurt;
        self.steering.reset();
        // Said differently for a fellow's spell, so that a run's log
        // tells a reflex that kept the character alive from a courtesy
        // that only saved a teammate's cast.
        let doing = match (track.can_hurt, caster.as_ref()) {
            (true, _) => format!("dodging {name}{from}"),
            (false, Some(c)) => format!("stepping out of {c}'s way"),
            (false, None) => format!("stepping out of the way of {name}"),
        };
        self.autoplay.say(Doing::Dodging, doing);
        true
    }

    /// The soonest tracked projectile on its way to `me` within
    /// [`HORIZON`], among those that `can_hurt` us or (false) among
    /// those that cannot.
    fn soonest(&self, now: Instant, me: Vec3, can_hurt: bool) -> Option<(u32, f32)> {
        let mut best: Option<(u32, f32)> = None;
        for (g, t) in &self.dodge.tracks {
            if t.dodged || t.can_hurt != can_hurt {
                continue;
            }
            if let Some(eta) = threat(t.at(now), t.velocity, t.gravity, me, MISS_BY) {
                if eta <= HORIZON && best.is_none_or(|(_, s)| eta < s) {
                    best = Some((*g, eta));
                }
            }
        }
        best
    }

    /// Whether there is a moment going spare for a projectile that
    /// cannot hurt this character.
    ///
    /// A dodge takes the legs for [`LASTS`] and claims the tick, ahead
    /// of the healing and of everything else the character might do. A
    /// spell about to land on it is worth that. A fellow's, which can
    /// only ever cost him his own spell, is worth the step and nothing
    /// more -- so it waits for a tick with no cast in the air, no swing
    /// unanswered and no wound waiting on a heal, and otherwise is left
    /// to land.
    fn free_to_step_aside(&self, now: Instant) -> bool {
        !self.autoplay.cast_in_flight(now)
            && !self.mid_attack()
            && self.health_fraction() >= self.autoplay.config.survive.heal_below
    }

    /// The creature that seems to have fired a projectile that appeared
    /// at `origin`: the nearest one within [`CASTER_WITHIN`], this
    /// character excepted.
    ///
    /// A guess, and named as one. Nothing on the wire says who cast a
    /// spell; what is known is that a projectile is made at its
    /// caster's feet, so the creature standing closest to where it
    /// appeared nearly always fired it. In a huddle of nine characters
    /// a metre apart it can name the fellow beside the one who did.
    /// Good enough to decide what a projectile is worth of the
    /// character's time; not good enough to decide whether to dodge it
    /// at all.
    fn fired_by(&self, origin: Vec3) -> Option<u32> {
        let me = self.world.player_guid;
        self.world
            .objects
            .values()
            .filter(|o| {
                Some(o.guid) != me
                    && o.item_type & ac_world::item_type::CREATURE != 0
                    && o.parent.is_none()
            })
            .filter_map(|o| o.world_pos().map(|p| (p.distance(origin), o.guid)))
            .filter(|(d, _)| *d <= CASTER_WITHIN)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, g)| g)
    }

    /// Whether a projectile fired by `caster` could hurt this character
    /// (see [`can_hurt_us`]). One whose caster is not known is taken for
    /// a threat: the guess is the doubtful part, not the rule.
    fn can_be_hurt_by(&self, caster: Option<u32>) -> bool {
        let Some(caster) = caster.and_then(|g| self.world.objects.get(&g)) else {
            return true;
        };
        let mine = self.world.player().map_or(0, |o| o.object_desc_flags);
        can_hurt_us(caster.object_desc_flags, mine)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: Vec3 = Vec3::new(100.0, 100.0, 10.0);
    /// Where the server aims: two thirds of the way up the target.
    const CHEST: f32 = 1.1;

    /// A projectile launched from `from` (feet) at `speed` straight at
    /// our chest, the way the server aims a bolt.
    fn aimed(from: Vec3, speed: f32) -> (Vec3, Vec3) {
        let origin = from + Vec3::new(0.0, 0.0, CHEST);
        let target = ME + Vec3::new(0.0, 0.0, CHEST);
        (origin, (target - origin).normalize() * speed)
    }

    /// A projectile lobbed from `from` at `speed` (flat) so that,
    /// falling under [`GRAVITY`], it comes down on our chest: the
    /// ballistic solution the server uses for arcs and arrows.
    fn lobbed(from: Vec3, speed: f32) -> (Vec3, Vec3) {
        let origin = from + Vec3::new(0.0, 0.0, CHEST);
        let target = ME + Vec3::new(0.0, 0.0, CHEST);
        let flat = Vec2::new(target.x - origin.x, target.y - origin.y);
        let t = flat.length() / speed;
        let dir = flat.normalize() * speed;
        // z(t) = z0 + vz t + g t^2 / 2 = target z
        let vz = (target.z - origin.z - 0.5 * GRAVITY * t * t) / t;
        (origin, Vec3::new(dir.x, dir.y, vz))
    }

    #[test]
    fn a_bolt_aimed_at_us_is_a_threat_with_its_flight_time() {
        // Ten metres south of us, flying north at 20 m/s.
        let (pos, v) = aimed(Vec3::new(100.0, 90.0, 10.0), 20.0);
        let t = threat(pos, v, 0.0, ME, MISS_BY).expect("a hit");
        assert!((t - 0.5).abs() < 1e-3, "{t}");
    }

    #[test]
    fn a_streak_is_a_bolt_that_gets_here_sooner() {
        // The same ten metres at 45 m/s: under a quarter of a second,
        // but still seen in time, and still on the same line.
        let (pos, v) = aimed(Vec3::new(100.0, 90.0, 10.0), 45.0);
        let t = threat(pos, v, 0.0, ME, MISS_BY).expect("a hit");
        assert!(t < 0.25 && t > 0.2, "{t}");
        let (l, _) = sides(v);
        assert!((l - Vec3::new(-1.0, 0.0, 0.0)).length() < 1e-4);
    }

    #[test]
    fn an_arc_is_lobbed_and_still_comes_down_on_us() {
        // Twelve metres off, thrown up so that it falls on our chest.
        let (pos, v) = lobbed(Vec3::new(100.0, 88.0, 10.0), 12.0);
        assert!(v.z > 3.0, "it goes up first: {v}");
        let t = threat(pos, v, GRAVITY, ME, MISS_BY).expect("a hit");
        assert!((t - 1.0).abs() < 1e-3, "{t}");
        // Read as a straight line it would sail high over our head.
        assert_eq!(threat(pos, v, 0.0, ME, MISS_BY), None);
        // And the track knows where it is halfway: up in the air.
        let seen = Instant::now();
        let track = Track {
            origin: pos,
            velocity: v,
            gravity: GRAVITY,
            seen,
            from: None,
            can_hurt: true,
            dodged: false,
        };
        let mid = track.at(seen + Duration::from_millis(500));
        assert!(mid.z > pos.z + 1.0, "{mid}");
        let end = track.at(seen + Duration::from_millis(1000));
        assert!((end.z - (ME.z + CHEST)).abs() < 0.05, "{end}");
    }

    #[test]
    fn an_arrow_flies_like_a_bolt_under_gravity() {
        // An archer twenty metres off, 30 m/s: a shallow arc.
        let (pos, v) = lobbed(Vec3::new(120.0, 100.0, 10.0), 30.0);
        let t = threat(pos, v, GRAVITY, ME, MISS_BY).expect("a hit");
        assert!((t - 20.0 / 30.0).abs() < 1e-3, "{t}");
        // The sidestep is across its line, north or south.
        let goal = sidestep(ME, v, 3.0, 3.0, 0.0).expect("room");
        assert!(
            (goal.x - ME.x).abs() < 1e-4 && (goal.y - ME.y).abs() > 2.0,
            "{goal}"
        );
    }

    #[test]
    fn only_the_ring_member_coming_our_way_is_a_threat() {
        // A ring: one projectile every 45 degrees from a caster four
        // metres to our south-west, all at 15 m/s.
        let caster = Vec3::new(97.0, 97.0, 10.0);
        let mut hits = 0;
        for i in 0..8 {
            let a = i as f32 * std::f32::consts::FRAC_PI_4;
            let dir = Vec3::new(a.cos(), a.sin(), 0.0);
            let pos = caster + dir * 0.8 + Vec3::new(0.0, 0.0, CHEST);
            if threat(pos, dir * 15.0, 0.0, ME, MISS_BY).is_some() {
                hits += 1;
                // The one heading north-east.
                assert!(dir.x > 0.5 && dir.y > 0.5, "{dir}");
            }
        }
        assert_eq!(hits, 1);
    }

    #[test]
    fn only_the_wall_member_in_line_with_us_is_a_threat() {
        // A wall: five projectiles two metres apart, side by side,
        // all flying north from ten metres south.
        let mut hits = Vec::new();
        for i in -2..=2 {
            let pos = Vec3::new(100.0 + 2.0 * i as f32, 90.0, 10.0 + CHEST);
            if threat(pos, Vec3::new(0.0, 20.0, 0.0), 0.0, ME, MISS_BY).is_some() {
                hits.push(i);
            }
        }
        assert_eq!(hits, vec![0]);
        // A fan (a blast or volley) is the same test with the
        // velocities rotated: the middle one is straight at us, the
        // outer ones swing wide.
        let (pos, v) = aimed(Vec3::new(100.0, 90.0, 10.0), 20.0);
        let mut hits = 0;
        for step in -2..=2 {
            let a = step as f32 * 15f32.to_radians();
            let rv = Vec3::new(
                v.x * a.cos() - v.y * a.sin(),
                v.x * a.sin() + v.y * a.cos(),
                v.z,
            );
            if threat(pos, rv, 0.0, ME, MISS_BY).is_some() {
                hits += 1;
            }
        }
        assert_eq!(hits, 1);
    }

    #[test]
    fn a_bolt_that_will_miss_is_not() {
        // Passing three metres to our east.
        let pos = Vec3::new(103.0, 90.0, 11.0);
        let v = Vec3::new(0.0, 20.0, 0.0);
        assert_eq!(threat(pos, v, 0.0, ME, MISS_BY), None);
        // Grazing within the margin still counts.
        let pos = Vec3::new(100.8, 90.0, 11.0);
        assert!(threat(pos, v, 0.0, ME, MISS_BY).is_some());
    }

    #[test]
    fn a_bolt_going_away_or_already_past_is_not() {
        // Our own: starts a metre in front of us, flies off north.
        let pos = Vec3::new(100.0, 101.0, 11.0);
        let v = Vec3::new(0.0, 20.0, 0.0);
        assert_eq!(threat(pos, v, 0.0, ME, MISS_BY), None);
        // One that has flown through and beyond.
        let pos = Vec3::new(100.0, 105.0, 11.0);
        assert_eq!(threat(pos, v, 0.0, ME, MISS_BY), None);
        // Not moving at all.
        assert_eq!(
            threat(Vec3::new(100.0, 90.0, 11.0), Vec3::ZERO, 0.0, ME, MISS_BY),
            None
        );
    }

    #[test]
    fn a_bolt_far_above_or_below_is_not() {
        let v = Vec3::new(0.0, 20.0, 0.0);
        assert_eq!(
            threat(Vec3::new(100.0, 90.0, 20.0), v, 0.0, ME, MISS_BY),
            None
        );
        assert_eq!(
            threat(Vec3::new(100.0, 90.0, 5.0), v, 0.0, ME, MISS_BY),
            None
        );
    }

    #[test]
    fn a_track_flies_straight_from_where_it_was_seen() {
        let seen = Instant::now();
        let t = Track {
            origin: Vec3::new(0.0, 0.0, 1.0),
            velocity: Vec3::new(10.0, 0.0, 0.0),
            gravity: 0.0,
            seen,
            from: None,
            can_hurt: true,
            dodged: false,
        };
        let p = t.at(seen + Duration::from_millis(500));
        assert!((p - Vec3::new(5.0, 0.0, 1.0)).length() < 1e-3, "{p}");
    }

    #[test]
    fn the_sidestep_is_across_the_path_on_the_roomier_side() {
        // Flying north: left is west (-x), right is east (+x).
        let v = Vec3::new(0.0, 20.0, 0.0);
        let goal = sidestep(ME, v, 3.0, 1.5, 0.0).expect("room");
        assert!(
            (goal - Vec3::new(100.0 - STEP, 100.0, 10.0)).length() < 1e-4,
            "{goal}"
        );
        let goal = sidestep(ME, v, 1.5, 3.0, 0.0).expect("room");
        assert!(
            (goal - Vec3::new(100.0 + STEP, 100.0, 10.0)).length() < 1e-4,
            "{goal}"
        );
        // Only as far as there is room.
        let goal = sidestep(ME, v, 1.5, 1.0, 0.0).expect("room");
        assert!(
            (goal - Vec3::new(98.5, 100.0, 10.0)).length() < 1e-4,
            "{goal}"
        );
    }

    #[test]
    fn the_sidestep_keeps_to_the_side_we_lean_to_when_it_has_room() {
        let v = Vec3::new(0.0, 20.0, 0.0);
        // Already half a metre east of the path: keep going east even
        // though the west has more room.
        let goal = sidestep(ME, v, 3.0, 1.5, -0.5).expect("room");
        assert!(goal.x > ME.x, "{goal}");
        // Unless the east is a wall.
        let goal = sidestep(ME, v, 3.0, 0.5, -0.5).expect("room");
        assert!(goal.x < ME.x, "{goal}");
    }

    #[test]
    fn no_room_either_side_is_no_step() {
        let v = Vec3::new(0.0, 20.0, 0.0);
        assert_eq!(sidestep(ME, v, 0.5, 0.5, 0.0), None);
    }

    #[test]
    fn lean_is_signed_by_side() {
        let v = Vec3::new(0.0, 20.0, 0.0);
        let path = Vec3::new(100.0, 90.0, 10.0);
        assert!(
            lean(path, v, Vec3::new(99.0, 100.0, 10.0)) > 0.0,
            "west is left"
        );
        assert!(
            lean(path, v, Vec3::new(101.0, 100.0, 10.0)) < 0.0,
            "east is right"
        );
        assert!(lean(path, v, ME).abs() < 1e-6);
    }

    #[test]
    fn a_spell_reaches_its_constant_plus_skill_times_mod_up_to_the_radar() {
        // ACE: min(BaseRangeConstant + magicSkill * BaseRangeMod, 75).
        assert!((spell_range(25.0, 0.1, 100) - 35.0).abs() < 1e-5);
        assert!((spell_range(25.0, 0.1, 0) - 25.0).abs() < 1e-5);
        assert_eq!(spell_range(25.0, 0.1, 1000), MAX_SPELL_RANGE);
    }

    #[test]
    fn a_missile_reaches_its_speed_squared_over_gravity_up_to_85_yards() {
        // ACE: min(maxVelocity^2 * 0.1020408, 85 / 1.094): the default
        // launcher speed of 20 gives 40.8 m.
        assert!((missile_range(DEFAULT_MAX_VELOCITY) - 40.816).abs() < 0.01);
        assert!((missile_range(10.0) - 10.204).abs() < 0.01);
        assert!((missile_range(30.0) - 77.697).abs() < 0.01, "capped");
        assert!((MAX_MISSILE_RANGE - 77.697).abs() < 0.01);
    }

    #[test]
    fn a_swing_reaches_the_servers_sticky_distance() {
        assert_eq!(MELEE_REACH, 0.6);
        assert_eq!(STICKY_REACH, 4.0);
    }

    #[test]
    fn sides_are_perpendicular_and_flat() {
        let v = Vec3::new(3.0, 4.0, 1.0);
        let (l, r) = sides(v);
        assert!((l.x * v.x + l.y * v.y).abs() < 1e-6);
        assert!((l + r).length() < 1e-6);
        assert_eq!(l.z, 0.0);
        assert!((l.length() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_monsters_spell_can_hurt_us_and_a_fellows_cannot() {
        use ac_world::object_desc_flags as flags;
        // Nothing flagged a player is a monster, and a monster's spell
        // always lands.
        assert!(can_hurt_us(0, 0));
        assert!(can_hurt_us(flags::ATTACKABLE, flags::PLAYER));
        // Two ordinary players: ACE refuses the damage either way
        // round, so the bolt only dies on us.
        assert!(!can_hurt_us(flags::PLAYER, flags::PLAYER));
        // A player killer's does land -- on another player killer.
        assert!(can_hurt_us(
            flags::PLAYER | flags::PLAYER_KILLER,
            flags::PLAYER | flags::PLAYER_KILLER
        ));
        // But not on someone who is not one, whichever side is which.
        assert!(!can_hurt_us(
            flags::PLAYER | flags::PLAYER_KILLER,
            flags::PLAYER
        ));
        assert!(!can_hurt_us(
            flags::PLAYER,
            flags::PLAYER | flags::PLAYER_KILLER
        ));
        // Free runs the other way: ACE short-circuits to "allowed" the
        // moment either side is Free, so a Free player's spell lands on
        // an ordinary character for full damage. Read as one more PK bit
        // that both sides had to carry, the one projectile that can kill
        // us was the one demoted to a courtesy.
        assert!(can_hurt_us(
            flags::PLAYER | flags::FREE_PK_STATUS,
            flags::PLAYER
        ));
        assert!(can_hurt_us(
            flags::PLAYER,
            flags::PLAYER | flags::FREE_PK_STATUS
        ));
    }

    /// The Holtburg field these tests stand in.
    const HOLTBURG: u32 = 0xA9B4_0019;
    /// This character's own guid in the world model.
    const ME_GUID: u32 = 0x5000_0009;

    /// A creature standing at `at`, described by `flags`.
    fn creature(guid: u32, name: &str, flags: u32, at: Vec3) -> ac_world::WorldObject {
        ac_world::WorldObject {
            object_desc_flags: flags,
            position: crate::testkit::placed(HOLTBURG, at),
            scale: 1.0,
            ..crate::testkit::creature(guid, name)
        }
    }

    /// A bolt made at `from`'s chest and flying at `at`'s, the way the
    /// server aims one.
    fn bolt(guid: u32, from: Vec3, at: Vec3, speed: f32) -> ac_world::WorldObject {
        let origin = from + Vec3::new(0.0, 0.0, CHEST);
        let target = at + Vec3::new(0.0, 0.0, CHEST);
        ac_world::WorldObject {
            guid,
            name: "Lightning Bolt V".into(),
            physics_state: ac_world::object::PHYSICS_STATE_MISSILE,
            velocity: (target - origin).normalize() * speed,
            position: Some(ac_world::object::Position::new_flat(
                HOLTBURG,
                origin - ac_world::landblock_origin(HOLTBURG),
            )),
            scale: 1.0,
            ..Default::default()
        }
    }

    /// A character standing in the Holtburg field, offline, over the
    /// game's archives. The room to either side is
    /// measured with the walker's own collision, so this wants the
    /// real landscape.
    fn in_the_field() -> Client {
        let mut c = crate::testkit::standing_at(HOLTBURG, Vec3::new(84.0, 84.0, 94.0));
        let me = c.player.as_ref().unwrap().world_position();
        c.world.player_guid = Some(ME_GUID);
        // The character's own object: the PK rule reads its flags.
        c.world.objects.insert(
            ME_GUID,
            creature(
                ME_GUID,
                "Blargerton",
                ac_world::object_desc_flags::PLAYER,
                me,
            ),
        );
        c
    }

    #[test]
    #[ignore = "needs AC_DATA_DIR"]
    fn a_fellows_bolt_is_known_at_intake_and_never_takes_the_tick_from_a_cast() {
        let mut c = in_the_field();
        let me = c.player.as_ref().unwrap().world_position();
        let now = Instant::now();
        // A fellow twelve metres off, shooting through where we stand
        // at whatever we are both fighting.
        let (fellow, shot) = (0x5000_0001, 0x8000_0001);
        let stands = me + Vec3::new(0.0, -12.0, 0.0);
        c.world.objects.insert(
            fellow,
            creature(
                fellow,
                "+Brynnu",
                ac_world::object_desc_flags::PLAYER,
                stands,
            ),
        );
        c.world.objects.insert(shot, bolt(shot, stands, me, 15.0));

        // A cast of our own in the air: the bolt is taken up, read for
        // a fellow's, and left to land. It cannot hurt us, and no spell
        // that cannot hurt us is worth a cancelled cast.
        c.autoplay.cast_sent = Some(now);
        assert!(
            !c.autoplay_dodge(now),
            "a fellow's bolt interrupted our own cast"
        );
        let track = c.dodge.tracks[&shot];
        assert_eq!(track.from, Some(fellow), "the caster guess");
        assert!(!track.can_hurt);
        assert_eq!(c.dodge_to, None);

        // The cast answered for, the same bolt is worth the step: it
        // dies on us otherwise, and the fellow has spent his mana for
        // nothing.
        c.autoplay.cast_sent = None;
        assert!(c.autoplay_dodge(now), "a spare moment is worth the step");
        assert!(c.dodge_to.is_some());
        assert!(c.dodge.courtesy, "the step was a courtesy, not a dodge");

        // And it is given back the moment the moment stops being spare:
        // a step for somebody else's spell must not hold the legs while
        // a heal comes due.
        c.autoplay.cast_sent = Some(now);
        assert!(!c.autoplay_dodge(now), "the courtesy outstayed its moment");
        assert_eq!(c.dodge_to, None);
    }

    #[test]
    #[ignore = "needs AC_DATA_DIR"]
    fn a_monsters_bolt_is_dodged_whatever_else_is_in_hand() {
        // The reflex as it was. In the run this change came from it
        // fired 543 times and nothing landed on anyone, and for a
        // character that can be hurt it is what keeps him alive.
        let mut c = in_the_field();
        let me = c.player.as_ref().unwrap().world_position();
        let now = Instant::now();
        let (shaman, shot) = (0x6000_0001, 0x8000_0002);
        let stands = me + Vec3::new(0.0, -12.0, 0.0);
        c.world.objects.insert(
            shaman,
            creature(
                shaman,
                "Drudge Shaman",
                ac_world::object_desc_flags::ATTACKABLE,
                stands,
            ),
        );
        c.world.objects.insert(shot, bolt(shot, stands, me, 15.0));
        // Mid-cast and mid-swing both.
        c.autoplay.cast_sent = Some(now);
        c.attack_pending = true;
        c.last_attack = now;
        assert!(
            c.autoplay_dodge(now),
            "a spell about to land was not dodged"
        );
        assert!(c.dodge.tracks[&shot].can_hurt);
        assert!(c.dodge_to.is_some());
    }

    #[test]
    #[ignore = "needs AC_DATA_DIR"]
    fn a_spell_that_would_land_outranks_a_fellows_that_arrives_sooner() {
        let mut c = in_the_field();
        let me = c.player.as_ref().unwrap().world_position();
        let now = Instant::now();
        // The fellow is nearer, so his bolt gets here first; the
        // shaman's is the one that matters.
        let (fellow, friendly) = (0x5000_0001, 0x8000_0001);
        let beside = me + Vec3::new(0.0, -4.0, 0.0);
        c.world.objects.insert(
            fellow,
            creature(
                fellow,
                "+Brynnu",
                ac_world::object_desc_flags::PLAYER,
                beside,
            ),
        );
        c.world
            .objects
            .insert(friendly, bolt(friendly, beside, me, 15.0));
        let (shaman, harmful) = (0x6000_0001, 0x8000_0002);
        let across = me + Vec3::new(12.0, 0.0, 0.0);
        c.world.objects.insert(
            shaman,
            creature(
                shaman,
                "Drudge Shaman",
                ac_world::object_desc_flags::ATTACKABLE,
                across,
            ),
        );
        c.world
            .objects
            .insert(harmful, bolt(harmful, across, me, 15.0));
        // Nothing else in hand, so both are eligible.
        assert!(c.autoplay_dodge(now));
        assert!(
            c.dodge.tracks[&harmful].dodged,
            "the step was taken for the fellow's bolt"
        );
        assert!(!c.dodge.tracks[&friendly].dodged);
    }
}
