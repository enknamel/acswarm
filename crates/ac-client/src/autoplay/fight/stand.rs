//! Where to stand in a dungeon fight with more than one creature coming to swing: the spot near
//! by with the fewest sides open to a melee attacker (a corner, a hallway, a wall at the back),
//! walked to once and fought from. Entry: `Client::autoplay_take_stand`.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use ac_nav::steering::REPLAN_DISTANCE;
use ac_world::Place;
use glam::Vec3;

use crate::autoplay::Doing;
use crate::did::Did;
use crate::dodge::{How, WITHIN};
use crate::{aim, Client, Goal, SAME_FLOOR};

/// Directions round a spot a melee attacker may stand in, 45 degrees apart.
const SIDES: usize = 8;
/// Centre to centre (metres) a creature stands to swing: under two bodies' radii (0.48 m each)
/// and ACE's `MaxMeleeRange` 0.75 m between cylinders (`Monster_Navigation.cs:28`, `:215`).
const RING: f32 = 1.2;
/// Spots looked at within this of the character (metres, flat).
const LOOK_WITHIN: f32 = 6.0;
/// Spots scored at most, nearest first: each costs [`SIDES`] walk tests.
const SPOTS_SCORED: usize = 64;
/// A creature fighting or coming at the character within this (metres) is counted.
const ATTACKERS_WITHIN: f32 = 20.0;
/// At the spot within this (metres, 3D).
const AT_STAND: f32 = 0.5;
/// With as many attackers in reach either way, a spot must close this many sides more to be walked to.
const MATERIAL_SIDES: u32 = 2;
/// How long a stand is held for one coming creature before the swing closes on it as before.
const HOLD_FOR: Duration = Duration::from_secs(8);

/// The stand in a fight and the spots refused, until fewer than two attackers are left.
#[derive(Debug, Default, Clone)]
pub(crate) struct Stands {
    pub(crate) held: Option<Stand>,
    /// Spots whose walk the slot refused: not chosen again in this fight.
    pub(crate) refused: Vec<Vec3>,
    /// The creature waited for at the stand, and since when (see [`HOLD_FOR`]).
    pub(crate) waiting: Option<(u32, Instant)>,
}

/// A spot to fight from, chosen against `against` melee attackers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Stand {
    pub(crate) spot: Place,
    pub(crate) against: usize,
    /// Sides open to a swing there, of [`SIDES`].
    pub(crate) sides: u32,
    /// Stood on, or chosen where the character stood: never walked back to.
    pub(crate) reached: bool,
    /// Walked to for fewer sides: only such a stand is held for a creature coming.
    pub(crate) walked: bool,
}

/// How many attackers reach at once from a spot with `open` sides, then the sides: less is better.
fn exposure(open: u32, attackers: usize) -> (u32, u32) {
    let sides = open.count_ones();
    (sides.min(attackers as u32), sides)
}

/// Whether `there` is worth walking to from `here` (both [`exposure`]): fewer in reach at once,
/// or as many with [`MATERIAL_SIDES`] fewer sides open.
fn is_material(there: (u32, u32), here: (u32, u32)) -> bool {
    there.0 < here.0 || (there.0 == here.0 && there.1 + MATERIAL_SIDES <= here.1)
}

impl Client {
    /// In a dungeon with two or more melee attackers, walk to the stand chosen for them before
    /// fighting: `Acting` while walking, else `Done`. Chosen again only when their number changes.
    pub(crate) fn autoplay_take_stand(
        &mut self,
        target: Option<u32>,
        how: How,
        now: Instant,
    ) -> Did {
        let Some(me) = self.player.as_ref().map(|p| Place {
            at: p.world_position(),
            cell: p.cell,
        }) else {
            return Did::Done;
        };
        let attackers = if self.underground() {
            self.melee_attackers(me.at)
        } else {
            Vec::new()
        };
        if attackers.len() < 2 {
            self.drop_stand();
            return Did::Done;
        }
        let held = self
            .autoplay
            .stands
            .held
            .filter(|s| s.against == attackers.len());
        let stand = match held {
            Some(s) => s,
            None => {
                let Some(s) = self.choose_stand(me, &attackers, target, how) else {
                    return Did::Done;
                };
                if !s.reached {
                    tracing::info!(
                        "stand: {} attackers; {} sides open at {:.1} {:.1} {:.1}, {:.1} m off",
                        attackers.len(),
                        s.sides,
                        s.spot.at.x,
                        s.spot.at.y,
                        s.spot.at.z,
                        s.spot.at.distance(me.at)
                    );
                }
                self.autoplay.stands.held = Some(s);
                s
            }
        };
        if stand.reached {
            // A swing in hand at a creature still coming would have the server walk us out to it.
            if let Some(t) = self.attack_target.filter(|_| matches!(how, How::Melee)) {
                if self.holds_stand_against(t, now) {
                    self.cancel_attack();
                }
            }
            return Did::Done;
        }
        // Walking, a swing or shot in hand would have the server walk us at its target instead.
        if self.attack_target.is_some() {
            self.cancel_attack();
        }
        let goal = Goal::At {
            place: stand.spot,
            within: AT_STAND,
        };
        match self.go(self.step_owner(), goal, now) {
            Did::Done => {
                if let Some(s) = self.autoplay.stands.held.as_mut() {
                    s.reached = true;
                }
                Did::Done
            }
            Did::Blocked(why) | Did::Refused(why) => {
                tracing::info!("stand: no way to the spot: {}", why.what);
                self.autoplay.stands.refused.push(stand.spot.at);
                self.autoplay.stands.held = None;
                Did::Done
            }
            Did::Acting | Did::Waiting(_) => {
                self.autoplay.say(
                    Doing::Fighting,
                    format!(
                        "taking a stand with {} sides open against {}",
                        stand.sides,
                        attackers.len()
                    ),
                );
                Did::Acting
            }
        }
    }

    /// Whether the swing at `guid` waits at the stand walked to: it is coming at the character, out
    /// of a swing's reach, and has been waited for less than [`HOLD_FOR`].
    pub(crate) fn holds_stand_against(&mut self, guid: u32, now: Instant) -> bool {
        if !self
            .autoplay
            .stands
            .held
            .is_some_and(|s| s.reached && s.walked)
        {
            return false;
        }
        let Some(me) = self.my_position() else {
            return false;
        };
        let mine = self.world.player_guid;
        let Some(o) = self.world.objects.get(&guid) else {
            return false;
        };
        let coming = (mine.is_some() && o.walked_at == mine) || self.hit_lately_by(&o.name);
        let out_of_reach = o
            .world_pos()
            .is_some_and(|at| at.distance(me) > self.attack_range(How::Melee) * WITHIN);
        if !(coming && out_of_reach) {
            return false;
        }
        let since = match self.autoplay.stands.waiting {
            Some((g, t)) if g == guid => t,
            _ => {
                self.autoplay.stands.waiting = Some((guid, now));
                now
            }
        };
        now.saturating_duration_since(since) < HOLD_FOR
    }

    /// Done with stands: fewer than two attackers, or out of the dungeon.
    fn drop_stand(&mut self) {
        self.autoplay.stands = Stands::default();
    }

    /// The creatures that fight the character by swinging, within [`ATTACKERS_WITHIN`]: coming
    /// at it, hitting it, or fought by it. One wielding a bow never needs a spot beside it
    /// (`Monster_Missile.cs:22`); a caster swings when its spell roll fails (`Monster_Combat.cs:108`).
    fn melee_attackers(&self, me: Vec3) -> Vec<(u32, Vec3)> {
        use ac_world::{item_type, object_desc_flags};
        let mine = self.world.player_guid;
        let archers: HashSet<u32> = self
            .world
            .objects
            .values()
            .filter(|o| o.item_type & item_type::MISSILE_WEAPON != 0)
            .filter_map(|o| o.wielder)
            .collect();
        let fought = [self.attack_target, self.autoplay.casting_at];
        self.world
            .objects
            .values()
            .filter(|o| {
                o.item_type & item_type::CREATURE != 0
                    && o.object_desc_flags & object_desc_flags::PLAYER == 0
                    && !o.is_player
                    && o.pet_owner == 0
                    && o.alive_or_unknown()
                    && !archers.contains(&o.guid)
            })
            .filter(|o| {
                (mine.is_some() && o.walked_at == mine)
                    || self.hit_lately_by(&o.name)
                    || fought.contains(&Some(o.guid))
            })
            .filter_map(|o| Some((o.guid, o.world_pos()?)))
            .filter(|(_, at)| at.distance(me) <= ATTACKERS_WITHIN)
            .collect()
    }

    /// The stand against `attackers` from `me`: the nearest spot leaving fewest in reach at once,
    /// on this floor, short of the attackers, by the leader, with `target` throwable for a shot;
    /// where it stands unless one is [`is_material`]. `None` while the region is built.
    fn choose_stand(
        &mut self,
        me: Place,
        attackers: &[(u32, Vec3)],
        target: Option<u32>,
        how: How,
    ) -> Option<Stand> {
        let cap = self.player.as_ref()?.capsule();
        let n = attackers.len();
        // Looked round first: that builds the region the walk tests read.
        let places = self.standable_near(me.at, LOOK_WITHIN)?;
        let here = self.pathfinder.open_sides(&[me], RING, SIDES, cap)?[0];
        let flat = |v: Vec3| v.truncate();
        let middle = attackers.iter().map(|(_, a)| *a).sum::<Vec3>() / n as f32;
        let toward = flat(middle - me.at);
        let leader_at = self
            .followed_leader()
            .filter(|_| !self.autoplay.academy.active)
            .map(|m| m.world);
        let fight_radius = self.autoplay.config.team.fight_radius.max(1.0);
        let thrown = !matches!(how, How::Melee);
        let refused = &self.autoplay.stands.refused;
        let mut spots: Vec<Place> = places
            .into_iter()
            .filter(|p| {
                let off = flat(p.at - me.at);
                off.length() > AT_STAND
                    && (p.at.z - me.at.z).abs() <= SAME_FLOOR
                    // Short of the attackers' middle: never walked through them.
                    && (toward.length() <= RING
                        || off.dot(toward.normalize()) <= toward.length() - RING)
                    && attackers.iter().all(|(_, a)| flat(*a - p.at).length() > RING)
                    && refused.iter().all(|r| r.distance(p.at) > REPLAN_DISTANCE)
                    && leader_at.is_none_or(|l| p.at.distance(l) <= fight_radius)
            })
            .collect();
        if thrown {
            spots.retain(|p| !self.is_dud_spot(p.at));
        }
        spots.sort_by(|a, b| a.at.distance(me.at).total_cmp(&b.at.distance(me.at)));
        spots.truncate(SPOTS_SCORED);
        let aimed = target.or_else(|| {
            attackers
                .iter()
                .min_by(|a, b| a.1.distance(me.at).total_cmp(&b.1.distance(me.at)))
                .map(|(g, _)| *g)
        });
        if let Some(t) = aimed.filter(|_| thrown) {
            spots.retain(|p| self.throws_from(p.at, t, how));
        }
        let open = self.pathfinder.open_sides(&spots, RING, SIDES, cap)?;
        let mut best: Option<(Place, u32)> = None;
        for (p, o) in spots.into_iter().zip(open) {
            if best.is_none_or(|(_, b)| exposure(o, n) < exposure(b, n)) {
                best = Some((p, o));
            }
        }
        let stay = Stand {
            spot: me,
            against: n,
            sides: here.count_ones(),
            reached: true,
            walked: false,
        };
        Some(match best {
            Some((spot, o)) if is_material(exposure(o, n), exposure(here, n)) => Stand {
                spot,
                against: n,
                sides: o.count_ones(),
                reached: false,
                walked: true,
            },
            _ => stay,
        })
    }

    /// Whether the attack `how` thrown from feet at `from` gets to `target` (see `crate::aim`).
    fn throws_from(&mut self, from: Vec3, target: u32, how: How) -> bool {
        let Some(at) = self.world.objects.get(&target).and_then(|o| o.world_pos()) else {
            return true;
        };
        let mine = self.world.player().map_or(0, |o| o.guid);
        let (a, b) = (self.body_of(mine, from), self.body_of(target, at));
        match aim::flight(self.shot_for(how), a, b) {
            Some(path) => self.shot_flies(&path, how),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests;
