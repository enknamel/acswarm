//! Where to stand in a dungeon fight with more than one creature coming to swing: the spot near
//! by with the fewest sides open to a melee attacker (a corner, a hallway, a wall at the back),
//! walked to once and fought from. Entry: `Client::autoplay_take_stand`. A caster also backs off
//! to a few metres from its attackers (`autoplay_keep_off`) and steps aside between its own casts
//! (`autoplay_jiggle`), so a bolt aimed where it stood at launch lands where it no longer is.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use ac_nav::steering::REPLAN_DISTANCE;
use ac_world::ring_casters::widest_ring;
use ac_world::Place;
use glam::{Vec2, Vec3};

use crate::autoplay::Doing;
use crate::did::Did;
use crate::dodge::rings::{ring_reach, PAST};
use crate::dodge::{How, WITHIN};
use crate::{aim, Client, Goal, SAME_FLOOR};

/// Directions round a spot a melee attacker may stand in, 45 degrees apart.
/// How far round a hurting spot a way off it is looked for (metres, flat): past an acid pool's 6.7 m.
const OFF_TRAP_LOOK: f32 = 12.0;
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
/// A caster backs off from an attacker nearer than this (metres): from 1-4 m bolts landed 0-110 ms after seen.
const STANDOFF_NEAR: f32 = 6.0;
/// The spot backed off to has its nearest attacker at most this far (metres).
const STANDOFF_FAR: f32 = 8.0;
/// Spots looked at for a back-off within this of the character (metres, flat).
const STANDOFF_LOOK: f32 = 8.0;
/// Spots asked whether the spell gets to the target from them, nearest first.
const STANDOFF_LOOKS: usize = 48;
/// How long a back-off walk is given.
const STANDOFF_LASTS: Duration = Duration::from_secs(3);
/// After a back-off, or none found, the next waits this long: a swinging creature follows.
const STANDOFF_AGAIN: Duration = Duration::from_secs(8);
/// A step between casts goes this far aside of the spot kept to (metres): `dodge::MISS_BY` off its aim.
const JIGGLE: f32 = 1.2;
/// At the step's end within this (metres, 3D); a standing spot this near the ideal one will do.
const AT_JIGGLE: f32 = 0.4;
/// How long a step between casts is given before the cast goes out anyway.
const JIGGLE_LASTS: Duration = Duration::from_millis(900);

/// The stand in a fight and the spots refused, until fewer than two attackers are left.
#[derive(Debug, Default, Clone)]
pub(crate) struct Stands {
    pub(crate) held: Option<Stand>,
    /// Spots whose walk the slot refused: not chosen again in this fight.
    pub(crate) refused: Vec<Vec3>,
    /// The creature waited for at the stand, and since when (see [`HOLD_FOR`]).
    pub(crate) waiting: Option<(u32, Instant)>,
}

/// A caster's back-off walk and its steps aside between casts, for one target.
#[derive(Debug, Clone)]
pub(crate) struct KeepOff {
    /// The target these are for: another starts afresh.
    pub(crate) fought: Option<u32>,
    /// The back-off spot being walked to, and since when.
    pub(crate) walk: Option<(Place, Instant)>,
    /// When the last back-off ended or found no spot (see [`STANDOFF_AGAIN`]).
    pub(crate) last: Option<Instant>,
    /// A cast went up since the last step aside: one is due before the next cast.
    pub(crate) due: bool,
    /// The step aside being walked, and since when.
    pub(crate) step: Option<(Place, Instant)>,
    /// The target and the spot the steps aside keep to.
    pub(crate) anchor: Option<(u32, Vec3)>,
    /// Which side the next step goes to, 1 or -1 across the line to the target.
    pub(crate) side: f32,
}

impl Default for KeepOff {
    fn default() -> Self {
        Self {
            fought: None,
            walk: None,
            last: None,
            due: false,
            step: None,
            anchor: None,
            side: 1.0,
        }
    }
}

impl KeepOff {
    /// Drops the back-off and step aside under way and the spot kept to: the character was moved off them.
    pub(crate) fn break_off(&mut self) {
        self.walk = None;
        self.step = None;
        self.anchor = None;
    }
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
    /// Given up for a shot ([`Client::leave_stand`]): kept so it is not chosen again, never walked to.
    pub(crate) left: bool,
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
            Some(s) if s.left => return Did::Done,
            // Left (run out of a ring): walked back to once no ring spreads over it, else waited out.
            Some(s) if s.reached && is_kept(&s) && !is_at_stand(&s, me.at) => {
                if is_in_rings(&self.spreading_rings(me.at), s.spot.at) {
                    return Did::Done;
                }
                let back = Stand {
                    reached: false,
                    ..s
                };
                self.autoplay.stands.held = Some(back);
                back
            }
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
        // Not walked into a ring spreading over the spot: the walk waits it out.
        if is_in_rings(&self.spreading_rings(me.at), stand.spot.at) {
            return Did::Done;
        }
        // Walking, a swing or shot in hand would have the server walk us at its target instead.
        if self.attack_target.is_some() {
            self.cancel_attack();
        }
        self.autoplay.keep_off.break_off();
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
            .is_some_and(|s| s.reached && s.walked && !s.left)
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

    /// The stand held is no shot at the target: given up till the attackers change, not walked
    /// back to, and a walked one not chosen again (a_stand_walked_off_for_a_shot_is_not_walked_back_to).
    pub(crate) fn leave_stand(&mut self) {
        let st = &mut self.autoplay.stands;
        let Some(s) = st.held.as_mut().filter(|s| !s.left) else {
            return;
        };
        s.left = true;
        if s.walked {
            st.refused.push(s.spot.at);
        }
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
        let leader = self.leader_bound();
        let thrown = !matches!(how, How::Melee);
        let refused = &self.autoplay.stands.refused;
        let rings = self.spreading_rings(me.at);
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
                    && !is_in_rings(&rings, p.at)
                    && leader.is_none_or(|(l, r)| p.at.distance(l) <= r)
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
            left: false,
        };
        Some(match best {
            Some((spot, o)) if is_material(exposure(o, n), exposure(here, n)) => Stand {
                spot,
                against: n,
                sides: o.count_ones(),
                reached: false,
                walked: true,
                left: false,
            },
            _ => stay,
        })
    }

    /// The leader followed and the team's fight radius round it: a spot to fight from keeps inside.
    fn leader_bound(&self) -> Option<(Vec3, f32)> {
        let leader_at = self
            .followed_leader()
            .filter(|_| !self.autoplay.academy.active)
            .map(|m| m.world)?;
        Some((leader_at, self.autoplay.config.team.fight_radius.max(1.0)))
    }

    /// The stand a caster keeps to rather than backing off: walked to, or a corner or wall it stood
    /// at already (fewer than [`SIDES`] open).
    fn stand_kept(&self) -> Option<Stand> {
        let me = self.my_position()?;
        self.autoplay
            .stands
            .held
            .filter(|s| !s.left && is_kept(s) && is_at_stand(s, me))
    }

    /// The back-off and steps aside for `target`, started afresh if they were for another.
    fn keep_off_for(&mut self, target: u32) {
        if self.autoplay.keep_off.fought != Some(target) {
            self.autoplay.keep_off = KeepOff {
                fought: Some(target),
                ..KeepOff::default()
            };
        }
    }

    /// Ends the slot's walk to `spot` if this step holds it: never another row's walk.
    fn stop_walk_to(&mut self, spot: Place, why: &str) {
        let owner = self.step_owner();
        let mine = self
            .going
            .as_ref()
            .is_some_and(|g| g.owner == owner && g.goal.place() == Some(spot));
        if mine {
            self.stop_going(Some(owner), why);
        }
    }

    /// How near each of `attackers` a caster keeps (metres, flat): [`STANDOFF_NEAR`], or past a
    /// ring caster's reach by [`PAST`].
    fn keeps_of(&self, attackers: &[(u32, Vec3)]) -> Vec<f32> {
        attackers
            .iter()
            .map(|(g, _)| {
                let ring = self
                    .world
                    .objects
                    .get(g)
                    .and_then(|o| widest_ring(o.weenie_class_id));
                ring.map_or(STANDOFF_NEAR, |r| STANDOFF_NEAR.max(ring_reach(r) + PAST))
            })
            .collect()
    }

    /// A caster nearer an attacker than it keeps ([`Client::keeps_of`]) backs off to a `standoff_spot`:
    /// `Acting` while walking, else `Done`. A stand kept outranks it; so does no room.
    pub(crate) fn autoplay_keep_off(&mut self, target: u32, how: How, now: Instant) -> Did {
        if !matches!(how, How::Spell(_)) || self.stand_kept().is_some() {
            return Did::Done;
        }
        self.keep_off_for(target);
        let Some(me) = self.my_position() else {
            return Did::Done;
        };
        let rings = self.spreading_rings(me);
        if let Some((spot, since)) = self.autoplay.keep_off.walk {
            let ringed = is_in_rings(&rings, spot.at);
            let did = if ringed {
                Did::blocked("a ring spreads over the spot")
            } else if now.saturating_duration_since(since) < STANDOFF_LASTS {
                let goal = Goal::At {
                    place: spot,
                    within: AT_STAND,
                };
                self.go(self.step_owner(), goal, now)
            } else {
                Did::blocked("took too long")
            };
            if matches!(did, Did::Acting | Did::Waiting(_)) {
                self.autoplay
                    .say(Doing::Fighting, "backing off to cast from further away");
                return Did::Acting;
            }
            if let Did::Blocked(why) | Did::Refused(why) = &did {
                let why = format!("backing off: {}", why.what);
                self.stop_walk_to(spot, &why);
            }
            self.autoplay.keep_off.walk = None;
            // Out from under a ring, another spot is looked for at once.
            if !ringed {
                self.autoplay.keep_off.last = Some(now);
            }
            return Did::Done;
        }
        let again = self.autoplay.keep_off.last;
        if again.is_some_and(|t| now.saturating_duration_since(t) < STANDOFF_AGAIN) {
            return Did::Done;
        }
        let attackers = self.melee_attackers(me);
        let keeps = self.keeps_of(&attackers);
        if clearance(&attackers, &keeps, me) >= 0.0 {
            return Did::Done;
        }
        // Looked round first: that builds the region the walk reads; built next tick.
        let wider = keeps.iter().fold(0.0f32, |w, k| w.max(k - STANDOFF_NEAR));
        let Some(places) = self.standable_near(me, STANDOFF_LOOK + wider) else {
            return Did::Done;
        };
        let around = Around {
            attackers: &attackers,
            keeps: &keeps,
            rings: &rings,
        };
        let Some(spot) = self.standoff_spot(places, me, around, target, how) else {
            self.autoplay.keep_off.last = Some(now);
            return Did::Done;
        };
        tracing::info!(
            "stand: backing off {:.1} m to {:.1} {:.1} {:.1}, {:.1} m from the nearest attacker",
            spot.at.distance(me),
            spot.at.x,
            spot.at.y,
            spot.at.z,
            nearest_of(&attackers, spot.at)
        );
        self.autoplay.keep_off.walk = Some((spot, now));
        self.autoplay_keep_off(target, how, now)
    }

    /// Of `places`, the nearest to `me` on this floor no nearer any attacker than kept and at most
    /// [`STANDOFF_FAR`] less [`STANDOFF_NEAR`] past that, out of every ring, the spell reaching `target`.
    fn standoff_spot(
        &mut self,
        places: Vec<Place>,
        me: Vec3,
        around: Around,
        target: u32,
        how: How,
    ) -> Option<Place> {
        let reach = self.reach_at(target, how);
        let target_at = self.world.objects.get(&target).and_then(|o| o.world_pos());
        let leader = self.leader_bound();
        let band = 0.0..=STANDOFF_FAR - STANDOFF_NEAR;
        let mut spots: Vec<Place> = places
            .into_iter()
            .filter(|p| {
                (p.at.z - me.z).abs() <= SAME_FLOOR
                    && band.contains(&clearance(around.attackers, around.keeps, p.at))
                    && !is_in_rings(around.rings, p.at)
                    && target_at.is_none_or(|t| p.at.distance(t) <= reach)
                    && leader.is_none_or(|(l, r)| p.at.distance(l) <= r)
                    && !self.is_dud_spot(p.at)
            })
            .collect();
        spots.sort_by(|a, b| a.at.distance(me).total_cmp(&b.at.distance(me)));
        spots.truncate(STANDOFF_LOOKS);
        spots
            .into_iter()
            .find(|p| self.throws_from(p.at, target, how))
    }

    /// How far off `target` the attack `how` goes out from: its reach, or nearer when nothing
    /// landed from further, times [`WITHIN`] (see `Client::autoplay_approach`).
    fn reach_at(&self, target: u32, how: How) -> f32 {
        let range = self.attack_range(how);
        let range = self
            .autoplay
            .closing_on(target)
            .map_or(range, |cap| range.min(cap));
        range * WITHIN
    }

    /// After each cast a caster steps [`JIGGLE`] aside of the spot it keeps to, or back to it:
    /// `Acting` while stepping. Never mid-cast: the caller asks only with the cast slot free.
    pub(crate) fn autoplay_jiggle(&mut self, target: u32, how: How, now: Instant) -> Did {
        if !matches!(how, How::Spell(_)) || self.mid_attack() {
            return Did::Done;
        }
        self.keep_off_for(target);
        if let Some((spot, since)) = self.autoplay.keep_off.step {
            let rings = self
                .my_position()
                .map(|me| self.spreading_rings(me))
                .unwrap_or_default();
            let did = if is_in_rings(&rings, spot.at) {
                Did::blocked("a ring spreads over the step")
            } else if now.saturating_duration_since(since) < JIGGLE_LASTS {
                let goal = Goal::At {
                    place: spot,
                    within: AT_JIGGLE,
                };
                self.go(self.step_owner(), goal, now)
            } else {
                Did::blocked("took too long")
            };
            if matches!(did, Did::Acting | Did::Waiting(_)) {
                self.autoplay
                    .say(Doing::Fighting, "stepping aside between casts");
                return Did::Acting;
            }
            if let Did::Blocked(why) | Did::Refused(why) = &did {
                let why = format!("the step aside: {}", why.what);
                self.stop_walk_to(spot, &why);
            }
            self.autoplay.keep_off.step = None;
            return Did::Done;
        }
        if !std::mem::take(&mut self.autoplay.keep_off.due) {
            return Did::Done;
        }
        let Some(spot) = self.jiggle_spot(target, how) else {
            return Did::Done;
        };
        self.autoplay.keep_off.step = Some((spot, now));
        self.autoplay_jiggle(target, how, now)
    }

    /// The next step across the line to `target`: a side of the spot kept to (a stand kept near) not
    /// stood on, else back to it; walked straight, the spell reaching, no nearer attackers, no ring.
    fn jiggle_spot(&mut self, target: u32, how: How) -> Option<Place> {
        let (here, cap) = self.player.as_ref().map(|p| {
            let here = Place {
                at: p.world_position(),
                cell: p.cell,
            };
            (here, p.capsule())
        })?;
        let me = here.at;
        let at = self
            .world
            .objects
            .get(&target)
            .and_then(|o| o.world_pos())?;
        let near = |a: Vec3| a.distance(me) <= JIGGLE + AT_STAND;
        let stand = self.stand_kept().filter(|s| s.reached && near(s.spot.at));
        let anchor = match (stand, self.autoplay.keep_off.anchor) {
            (Some(s), _) => s.spot.at,
            (None, Some((g, a))) if g == target && near(a) => a,
            _ => me,
        };
        self.autoplay.keep_off.anchor = Some((target, anchor));
        let across = (at - anchor).truncate().perp().normalize_or_zero();
        if across == Vec2::ZERO {
            return None;
        }
        let attackers = self.melee_attackers(me);
        let floor = nearest_of(&attackers, anchor).clamp(RING, STANDOFF_NEAR);
        // A ring caster's reach the spot kept to is out of, a step aside keeps out of too.
        let reaches: Vec<(Vec2, f32)> = attackers
            .iter()
            .filter_map(|(g, a)| {
                let ring = widest_ring(self.world.objects.get(g)?.weenie_class_id)?;
                let off = a.truncate().distance(anchor.truncate());
                Some((a.truncate(), ring_reach(ring).min(off)))
            })
            .collect();
        let rings = self.spreading_rings(me);
        let reach = self.reach_at(target, how);
        let side = self.autoplay.keep_off.side;
        let mut why: Vec<&str> = Vec::new();
        for s in [side, -side, 0.0] {
            let ideal = anchor + (across * JIGGLE * s).extend(0.0);
            if ideal.truncate().distance(me.truncate()) <= AT_JIGGLE {
                why.push("stood on");
                continue;
            }
            let home = s == 0.0;
            let place = self.place_at(ideal);
            let Ok(Some(p)) = self.pathfinder.put_on_floor(here, place, cap) else {
                why.push("no floor");
                continue;
            };
            let off = if p.at.truncate().distance(ideal.truncate()) > AT_JIGGLE
                || (p.at.z - anchor.z).abs() > SAME_FLOOR
            {
                Some("off the floor")
            } else if p.at.distance(at) > reach {
                Some("out of reach")
            } else if !home && (nearest_of(&attackers, p.at) < floor || is_in_rings(&reaches, p.at))
            {
                Some("nearer an attacker")
            } else if is_in_rings(&rings, p.at) || self.is_dud_spot(p.at) {
                Some("a ring or a dud spot")
            } else if ac_nav::fly_over::is_on_trap(p, cap.radius) {
                Some("a trap or hurting ground")
            } else if !self.pathfinder.can_walk(here, p.at, cap) {
                Some("no walk")
            } else if !self.throws_from(p.at, target, how) {
                Some("no shot")
            } else {
                None
            };
            if let Some(w) = off {
                why.push(w);
                continue;
            }
            if !home {
                self.autoplay.keep_off.side = -s;
            }
            return Some(p);
        }
        tracing::info!("stand: no step aside: {}", why.join(", "));
        None
    }

    /// Standing within a trap's reach (an acid pool, fire, a plate): walked to the nearest spot clear
    /// of every one, a fight or none, the player's rule (a_character_in_acid_walks_out_of_it).
    pub(crate) fn autoplay_leave_trap(&mut self, now: Instant) -> bool {
        let Some((here, cap)) = self.player.as_ref().map(|p| {
            let here = Place {
                at: p.world_position(),
                cell: p.cell,
            };
            (here, p.capsule())
        }) else {
            return false;
        };
        if self.is_aloft() || !ac_nav::fly_over::is_on_trap(here, cap.radius) {
            return false;
        }
        // Spots on a trap are never standable (`routing::standable_near`): the nearest is off it.
        let spots = self
            .standable_near(here.at, OFF_TRAP_LOOK)
            .unwrap_or_default();
        let near = |p: &Place| p.at.distance(here.at);
        let Some(spot) = spots.into_iter().min_by(|a, b| near(a).total_cmp(&near(b))) else {
            return false;
        };
        let goal = Goal::At {
            place: spot,
            within: AT_STAND,
        };
        if matches!(self.go(self.step_owner(), goal, now), Did::Blocked(_)) {
            return false;
        }
        self.autoplay
            .say(Doing::Dodging, "stepping off hurting ground");
        true
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

/// The attackers round a back-off, how near each a caster keeps, and the rings spreading.
#[derive(Clone, Copy)]
struct Around<'a> {
    attackers: &'a [(u32, Vec3)],
    keeps: &'a [f32],
    rings: &'a [(Vec2, f32)],
}

/// How far `at` stands outside the nearest of the distances `keeps` from `attackers` (metres, flat;
/// below 0 inside one); infinite with none.
fn clearance(attackers: &[(u32, Vec3)], keeps: &[f32], at: Vec3) -> f32 {
    attackers
        .iter()
        .zip(keeps)
        .map(|((_, a), k)| a.truncate().distance(at.truncate()) - k)
        .fold(f32::INFINITY, f32::min)
}

/// Whether `s` is held rather than backed off from: walked to, or a corner or wall stood at.
fn is_kept(s: &Stand) -> bool {
    s.walked || s.sides < SIDES as u32
}

/// Whether `at` is at the stand `s`: within a step aside of its spot (flat).
fn is_at_stand(s: &Stand, at: Vec3) -> bool {
    s.spot.at.truncate().distance(at.truncate()) <= JIGGLE + AT_STAND
}

/// Whether `at` stands inside any of `rings` (flat centre, reach).
fn is_in_rings(rings: &[(Vec2, f32)], at: Vec3) -> bool {
    rings.iter().any(|(c, r)| at.truncate().distance(*c) < *r)
}

/// How far the nearest of `attackers` stands from `at` (metres, flat); infinite with none.
fn nearest_of(attackers: &[(u32, Vec3)], at: Vec3) -> f32 {
    attackers
        .iter()
        .map(|(_, a)| a.truncate().distance(at.truncate()))
        .fold(f32::INFINITY, f32::min)
}

#[cfg(test)]
mod tests;
