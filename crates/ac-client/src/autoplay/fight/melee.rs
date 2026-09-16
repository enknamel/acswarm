use std::time::{Duration, Instant};

use super::road_fight_over;
#[cfg(doc)]
use crate::autoplay::hear::arrived_unharmed;
use crate::autoplay::{Autoplay, Doing, Fight, Release};
use crate::refusals;
use crate::{Client, Stance};

/// A target that takes no damage for this long is let go.
const STALL_AFTER: Duration = Duration::from_secs(20);
/// Walking up to a target is working on it while each stretch brings the
/// character this much nearer than it has been (see [`came_nearer`]).
const APPROACH_PROGRESS: f32 = 1.0;
/// And left alone for this long afterwards.
pub(super) const GIVE_UP_FOR: Duration = refusals::ATTACK_AGAIN;
/// How far off a creature that has stopped attacking can stand before a
/// character on the road lets it go and walks on (see
/// [`road_fight_over`]): beyond a swing's reach, so one still being hit
/// is finished rather than left at half health.
pub(super) const ROAD_REACH: f32 = 8.0;
/// Casting or shooting this long from one spot with nothing landing:
/// the spot is no good, and the character closes in rather than going on.
pub(crate) const CLOSE_IN_AFTER: Duration = Duration::from_secs(8);
/// Nearer than this, closing in again achieves nothing: give up instead.
const MIN_STAND_OFF: f32 = 6.0;

/// Whether what has been thrown at a target has had long enough to get
/// there and has not: the first shot since anything last arrived went
/// out at `thrown`, more than [`CLOSE_IN_AFTER`] ago. Nothing thrown is
/// nothing missed.
pub(crate) fn nothing_arrived(thrown: Option<Instant>, now: Instant) -> bool {
    thrown.is_some_and(|t| now.duration_since(t) > CLOSE_IN_AFTER)
}

/// Whether the walk up to `guid`, now `distance` off, has come nearer
/// than the nearest yet (`best`, for whichever target it was) by
/// [`APPROACH_PROGRESS`].
fn came_nearer(best: Option<(u32, f32)>, guid: u32, distance: f32) -> bool {
    best.is_none_or(|(g, d)| g != guid || distance < d - APPROACH_PROGRESS)
}

/// How near to fight from after nothing has landed from `distance`: half
/// as far, never nearer than [`MIN_STAND_OFF`]. `None` when already that
/// close, and there is nowhere nearer worth trying.
fn closer_stand_off(distance: f32) -> Option<f32> {
    (distance > MIN_STAND_OFF + 1.0).then(|| (distance * 0.5).max(MIN_STAND_OFF))
}

/// Least time between two attack orders.
const ATTACK_EVERY: Duration = Duration::from_millis(1200);

impl Autoplay {
    /// How near to fight `guid` from, when it has not been hurt from
    /// further off (see `Client::stalled_on`).
    pub(crate) fn closing_on(&self, guid: u32) -> Option<f32> {
        self.closing.filter(|(g, _)| *g == guid).map(|(_, cap)| cap)
    }
}

impl Client {
    /// Pick something to fight and attack it. True when fighting.
    pub(crate) fn autoplay_fight(&mut self, now: Instant) -> bool {
        let cfg = self.autoplay.config.fight.clone();
        self.autoplay_fight_as(now, &cfg)
    }

    /// The fight rule with the rules given (the academy points it at the
    /// creatures a task names).
    pub(crate) fn autoplay_fight_as(&mut self, now: Instant, cfg: &Fight) -> bool {
        let cfg = cfg.clone();
        if !cfg.enabled {
            return false;
        }
        let stance = self.fighting_stance_as(cfg.style);
        if stance == Stance::Magic {
            return self.autoplay_fight_with_spells(now, &cfg);
        }
        // Already on one that is still alive -- unless the leader's plan
        // has this character on another, and this one is not hitting us
        // (see `Client::ordered_elsewhere`).
        if let Some(t) = self.attack_target {
            if self.ordered_elsewhere(t, &cfg, now) {
                self.let_go(Release::Fight);
            }
        }
        if let Some(t) = self.attack_target {
            if self.stalled_on(t, now) {
                return false;
            }
            // And still here (see `fight_target_gone`).
            let underground = self.underground();
            let gone =
                self.fight_target_gone(t, underground) || self.left_behind_on_the_road(t, now);
            if gone {
                self.let_go(Release::Targets);
            }
            if let Some(o) = self.world.objects.get(&t).filter(|_| !gone) {
                if o.health.unwrap_or(1.0) > 0.0 {
                    let name = o.name.clone();
                    // A weapon choice put off for a swing in the air, or
                    // waiting on an appraisal, is made here. Nothing else
                    // asks again once the fight is joined, so what the
                    // buff pass left in the character's hands was what it
                    // fought the whole creature with.
                    if self.autoplay.armed_for != Some(t) {
                        self.arm_for(t, stance, &cfg);
                        if self.hands_changing(now) {
                            self.autoplay
                                .say(Doing::Fighting, format!("changing weapon for {name}"));
                            return true;
                        }
                    }
                    self.autoplay
                        .say(Doing::Fighting, format!("fighting {name}"));
                    // A bow with an empty ammunition slot shoots
                    // nothing, and the slot empties mid-fight.
                    if stance == Stance::Missile
                        && !self.ready_ammo()
                        && self.autoplay_craft_ammo(now)
                    {
                        return true;
                    }
                    return true;
                }
            }
        }
        // Finish what it killed before setting off after the next one --
        // and, leading, what the party killed: the next fight is not
        // walked off to while a body dealt to one of the others still
        // lies here (see `Self::party_owed_a_body`).
        if self.waits_for_a_corpse() {
            if let Some((who, what)) = self.party_owed_a_body(now) {
                self.autoplay
                    .say(Doing::Idle, format!("waiting for {who} to empty {what}"));
            }
            return false;
        }
        if self
            .autoplay
            .last_attack
            .is_some_and(|t| now.duration_since(t) < ATTACK_EVERY)
        {
            return false;
        }
        // Fighting at range: the bow needs something to shoot, and
        // when there is nothing to shoot, something is made.
        let missile = stance == Stance::Missile;
        if missile && !self.ready_ammo() && self.autoplay_craft_ammo(now) {
            return true;
        }
        let me = self.player.as_ref().map(|p| p.world_position());
        let Some(me) = me else { return false };
        // Hunting together: hit what the leader's plan says, and with no
        // plan what the team is hitting, unless it is something this
        // character is walking past on its way somewhere. The leader
        // takes its own orders; without a plan it picks for itself.
        let team = &self.autoplay.config.team;
        if team.enabled && team.focus_fire {
            let joined = self.ordered_target(&cfg, now).or_else(|| {
                if self.autoplay.team.leader {
                    return None;
                }
                self.autoplay
                    .team
                    .target()
                    .filter(|(guid, _)| self.joins_the_team_on(*guid, &cfg))
            });
            if let Some((guid, name)) = joined {
                if self.autoplay_plan_hard(guid, &name, now) {
                    return true;
                }
                self.remember_journey();
                self.arm_for(guid, stance, &cfg);
                if self.hands_changing(now) {
                    self.autoplay
                        .say(Doing::Fighting, format!("changing weapon for {name}"));
                    return true;
                }
                if missile && self.autoplay_approach(guid, &name, crate::dodge::How::Missile) {
                    return true;
                }
                self.enter_combat();
                self.attack(guid);
                self.autoplay.last_attack = Some(now);
                if missile {
                    self.throw_at(guid, now);
                }
                self.autoplay
                    .say(Doing::Fighting, format!("joining on {name}"));
                return true;
            }
        }
        // What cannot be judged yet is asked about, not attacked.
        self.ask_about_strangers(me, &cfg);
        let underground = self.underground();
        let target = self
            .world
            .objects
            .values()
            .filter(|o| self.would_fight(o, &cfg, underground, now))
            .filter_map(|o| {
                let p = o.world_pos()?;
                let d = p.distance(me);
                (d <= cfg.radius).then_some((d, o.guid, o.name.clone()))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let Some((_, guid, name)) = target else {
            self.note_walked_past(me, &cfg, underground, now);
            return false;
        };
        if self.autoplay_plan_hard(guid, &name, now) {
            return true;
        }
        self.remember_journey();
        self.arm_for(guid, stance, &cfg);
        // The hands are empty between the put and the wield, and a swing
        // sent into that gap is a punch (see `hands_changing`).
        if self.hands_changing(now) {
            self.autoplay
                .say(Doing::Fighting, format!("changing weapon for {name}"));
            return true;
        }
        // A bow refused for range shoots nothing: close in first (see
        // `crate::dodge`). A swing from too far the server walks us
        // in for.
        if missile && self.autoplay_approach(guid, &name, crate::dodge::How::Missile) {
            return true;
        }
        self.enter_combat();
        self.attack(guid);
        self.autoplay.last_attack = Some(now);
        if missile {
            self.throw_at(guid, now);
        }
        self.autoplay.say(
            Doing::Fighting,
            if missile {
                format!("shooting {name}")
            } else {
                format!("attacking {name}")
            },
        );
        true
    }

    /// Note the target being worked on. True when it has taken no
    /// damage for too long and should be let go: it is out of reach,
    /// behind something, or not what it seems.
    pub(super) fn stalled_on(&mut self, guid: u32, now: Instant) -> bool {
        let health = self
            .world
            .objects
            .get(&guid)
            .and_then(|o| o.health)
            .unwrap_or(1.0);
        match self.autoplay.engaged {
            Some((g, since, last)) if g == guid => {
                if health < last - 0.001 {
                    self.autoplay.engaged = Some((guid, now, health));
                    self.autoplay.thrown = None;
                    false
                } else if self.dodge.approaching == Some(guid) && self.walking_nearer(guid) {
                    // Walking up to it, and getting nearer: not stalled.
                    // The clock ran through the walk, and a target across a
                    // few of a dungeon's rooms was given up before the first
                    // spell went at it.
                    self.autoplay.engaged = Some((guid, now, health));
                    false
                } else if nothing_arrived(
                    self.autoplay
                        .thrown
                        .filter(|(g, _)| *g == guid)
                        .map(|(_, t)| t),
                    now,
                ) && self.close_in_on(guid)
                {
                    // A new spot to fight from gets its own chance.
                    self.autoplay.engaged = Some((guid, now, health));
                    self.autoplay.thrown = None;
                    false
                } else if now.duration_since(since) > STALL_AFTER {
                    self.give_up_target(guid, "no damage in a while", now);
                    true
                } else {
                    false
                }
            }
            _ => {
                self.autoplay.engaged = Some((guid, now, health));
                self.autoplay.closing = self.autoplay.closing.filter(|(g, _)| *g == guid);
                self.autoplay.approach_best = None;
                false
            }
        }
    }

    /// Whether `guid`, the creature being fought, is not there to fight
    /// any more.
    ///
    /// A target is chosen from within the fight radius, but nothing
    /// checked it was still within it afterwards -- so a creature the
    /// character walked away from, or left behind in the Academy,
    /// stayed its target for ever while it planned a journey to the
    /// other side of the world to swing at it. Anything further off than
    /// a walk is an object left over from somewhere the character has
    /// since left. The swing had this rule and the spell did not:
    /// teleported out of the Holtburg Dungeon mid-fight, a caster stood
    /// in town for ten minutes throwing Flame Arc at a Swamp Rat
    /// thirty-four kilometres away, "closing" on it by halves.
    ///
    /// Or gone out of the hunting area, and not hitting us: let it go.
    pub(crate) fn fight_target_gone(&self, guid: u32, underground: bool) -> bool {
        self.world
            .objects
            .get(&guid)
            .and_then(|o| o.world_pos())
            .zip(self.player.as_ref().map(|p| p.world_position()))
            .is_some_and(|(at, me)| at.distance(me) > crate::travel::WALKABLE)
            || !self.area_allows_guid(guid, underground)
    }

    /// Whether `guid`, the creature being fought, has dropped out of a
    /// fight taken on the road (see [`road_fight_over`]), and say so
    /// when it has. Asked of the fight in hand each tick, beside
    /// [`Self::fight_target_gone`].
    pub(super) fn left_behind_on_the_road(&mut self, guid: u32, now: Instant) -> bool {
        let Some(o) = self.world.objects.get(&guid) else {
            return false;
        };
        let Some(away) = o
            .world_pos()
            .zip(self.player.as_ref().map(|p| p.world_position()))
            .map(|(at, me)| at.distance(me))
        else {
            return false;
        };
        let mates = &self.autoplay.team.mates;
        let ours = |g: u32| self.world.player_guid == Some(g) || mates.iter().any(|m| m.guid == g);
        let over = road_fight_over(
            self.on_its_way(),
            self.hit_lately_by(&o.name),
            o.walked_at.is_some_and(ours),
            self.a_mate_on_the_road_is_on(guid),
            away,
        );
        if over {
            let name = o.name.clone();
            self.autoplay.note(
                format!("letting {name} go: it stopped following on the road ({away:.0} m)"),
                now,
            );
        }
        over
    }

    /// Whether the walk up to `guid` has brought the character nearer to
    /// it than it has been in this fight (see [`came_nearer`]).
    fn walking_nearer(&mut self, guid: u32) -> bool {
        let Some(me) = self.player.as_ref().map(|p| p.world_position()) else {
            return false;
        };
        let Some(at) = self.world.objects.get(&guid).and_then(|o| o.world_pos()) else {
            return false;
        };
        let distance = me.distance(at);
        if !came_nearer(self.autoplay.approach_best, guid, distance) {
            return false;
        }
        self.autoplay.approach_best = Some((guid, distance));
        true
    }

    /// Nothing is landing on `guid` from where a ranged attacker stands:
    /// fight it from nearer. True when a nearer stand-off was set.
    ///
    /// The flight is worked out before a shot is thrown (see `crate::aim`),
    /// but only through the world that stands still: another creature in
    /// the way, a door, a target on the move can still take it -- and a
    /// caster that went on casting from the same spot for twenty seconds,
    /// spending components, and then gave up, never tried a step closer.
    ///
    /// Only something thrown can miss this way: a projectile spell or an
    /// arrow. Any other spell lands or is resisted where it is cast, and a
    /// resist or an evasion got there all the same (see
    /// [`arrived_unharmed`]). A melee attacker is walked in by the server
    /// already.
    fn close_in_on(&mut self, guid: u32) -> bool {
        let thrown = self.missile
            || (self.autoplay.casting_at == Some(guid)
                && self
                    .autoplay
                    .attack_spell
                    .is_some_and(|s| self.spell_flies(s)));
        if !thrown {
            return false;
        }
        let (Some(me), Some(at)) = (
            self.player.as_ref().map(|p| p.world_position()),
            self.world.objects.get(&guid).and_then(|o| o.world_pos()),
        ) else {
            return false;
        };
        let away = at.distance(me);
        let Some(cap) = closer_stand_off(away) else {
            return false;
        };
        let name = self
            .world
            .objects
            .get(&guid)
            .map(|o| o.name.clone())
            .unwrap_or_default();
        self.autoplay.note(
            format!("nothing landing on {name} from {away:.0} m; closing to {cap:.0} m"),
            Instant::now(),
        );
        self.autoplay.closing = Some((guid, cap));
        true
    }

    /// Where the creature just killed was standing: the one being fought
    /// when it has that name, else the nearest creature by that name.
    pub(crate) fn killed_at(&self, name: &str) -> Option<glam::Vec3> {
        let me = self.player.as_ref()?.world_position();
        let fought = [self.attack_target, self.autoplay.casting_at]
            .into_iter()
            .flatten()
            .filter_map(|g| self.world.objects.get(&g))
            .find(|o| o.name == name)
            .and_then(|o| o.world_pos());
        fought.or_else(|| {
            self.world
                .objects
                .values()
                .filter(|o| o.name == name && o.item_type & ac_world::item_type::CREATURE != 0)
                .filter_map(|o| o.world_pos())
                .min_by(|a, b| a.distance(me).total_cmp(&b.distance(me)))
        })
    }
}

#[cfg(test)]
mod tests;
