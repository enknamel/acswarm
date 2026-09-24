use std::time::{Duration, Instant};

use glam::Vec2;

use super::road::{on_the_way, OnTheWay, RETRY_AFTER, WALK_ON_EVERY, WALK_TIMEOUT};
use super::{about, Growth};
use crate::autoplay::{Doing, CORPSE_LIFE};
use crate::Client;
use ac_world::object_desc_flags;

/// Standing this close to a ground's middle counts as being there.
const GROUND_REACH: f32 = 30.0;

/// A ground the character has just left is not gone back to for this
/// long; one it could not reach, the same.
const SKIP_GROUND_FOR: Duration = Duration::from_secs(20 * 60);

/// At a ground with nothing in sight, the character walks this far to
/// look, this many times, before it moves on: the spawns of a block
/// are spread over it and the fight rules only look a short way.
const ROAM: f32 = 70.0;

const ROAMS: u32 = 3;

/// The longest a character inside a hunting area stands on a quiet spot
/// before walking to another part of it, seconds.
///
/// Short, because moving on inside an area costs only the walk: the
/// area is still the ground, the way is already known, and the fights
/// are wherever the character is not. Nine characters given a
/// 197 x 192 m field worked a 40 x 32 m corner of it for ten minutes,
/// keeping one spawn point busy, because the wait was the minute a
/// character picking a whole new ground is given.
const PATROL_AFTER: f32 = 10.0;

/// How long a spot with nothing on it is given before the character
/// looks elsewhere, seconds. See [`PATROL_AFTER`] for why a hunting
/// area is given less; a setting shorter than that is still obeyed.
fn quiet_before_move(cfg: &Growth, in_an_area: bool) -> f32 {
    if in_an_area {
        cfg.idle_before_move.min(PATROL_AFTER)
    } else {
        cfg.idle_before_move
    }
}

impl Client {
    // ---- hunting grounds ------------------------------------------

    /// The hunting ground this character is on or heading for.
    pub fn hunting_ground(&self) -> Option<(u32, Vec2, String)> {
        let st = &self.autoplay.growth;
        st.bound.clone().or_else(|| {
            let lb = st.hunting_at?;
            let g = ac_world::hunting::all()
                .iter()
                .find(|g| g.landblock == lb)?;
            Some((lb, g.at, g.name.clone()))
        })
    }

    /// The ground the party is on, for a character that does not choose
    /// its own.
    ///
    /// A party that walks out of town each picking the nearest ground
    /// to whichever shop it finished at ends up in four different
    /// places. The leader's ground is the party's, so everyone comes
    /// back to the same one.
    fn party_ground(&self) -> Option<(u32, Vec2, String)> {
        let team = &self.autoplay.config.team;
        if !team.enabled || team.lead {
            return None;
        }
        self.autoplay
            .team
            .leader_mate()
            .and_then(|m| m.ground.clone())
    }

    /// Keep the two clocks that are read off the ground rather than off
    /// what the character is doing: how long this spot has been quiet,
    /// and when each body lying on it first came into sight.
    ///
    /// The quiet clock starts when there is nothing here the character
    /// would fight and stops the moment something is (see
    /// [`Client::a_fight_in_sight`]). It reads the world rather than
    /// the status line: read off the status line it measured how busy
    /// the character was instead of how quiet the ground was, so
    /// re-asking a corpse that would not open counted as having
    /// something to do and put the clock back to nothing, and nine
    /// characters queueing at one body restarted it every couple of
    /// seconds and it never reached its minute once in ten.
    ///
    /// Both clocks are wound here, before anything can claim the tick,
    /// for the same reason: the steps that read them are not reached on
    /// every tick, and a clock only wound where it is read stands still
    /// exactly when it is most needed. The hunting step is the last goal
    /// in the table, so a character with a body to open never reaches
    /// it. Worse, the bodies used to be noted inside the looting: a
    /// character the claim tie-break told to stand off scores that body
    /// nothing, so its loot goal is never run, so it never notes the
    /// body, so the body stays for ever "newly fallen" and the tie-break
    /// governs for the whole five minutes it lies there. One mate that
    /// could not loot -- a full pack, no profile, looting turned off --
    /// then locked every body within twenty metres of it away from the
    /// other eight for good.
    pub(crate) fn autoplay_watch_the_ground(&mut self, now: Instant) {
        let quiet = !self.a_fight_in_sight(now);
        let fresh: Vec<u32> = self
            .world
            .objects
            .values()
            .filter(|o| o.object_desc_flags & object_desc_flags::CORPSE != 0)
            .map(|o| o.guid)
            .filter(|g| self.autoplay.corpse_seen.since(g).is_none())
            .collect();
        let st = &mut self.autoplay;
        for g in fresh {
            st.corpse_seen.mark(g, now);
        }
        // Forgotten once emptied, so the list stays the size of what is
        // on the ground.
        let looted = &st.looted;
        st.corpse_seen.expire(now, CORPSE_LIFE * 2);
        st.corpse_seen.retain(|g| !looted.contains(g));
        let st = &mut st.growth;
        if quiet {
            st.quiet_since.get_or_insert(now);
        } else {
            st.quiet_since = None;
        }
    }

    /// Go somewhere with monsters when there have been none for a
    /// while. True while it is on the way.
    pub(super) fn grow_hunt(&mut self, now: Instant, cfg: &Growth) -> bool {
        let Some((me, cell)) = self.player.as_ref().map(|p| (p.world_position(), p.cell)) else {
            return false;
        };
        let me = Vec2::new(me.x, me.y);
        let here = cell >> 16;
        // On the way: keep going, and notice arriving.
        if let Some((lb, at, name)) = self.autoplay.growth.bound.clone() {
            let too_long = self
                .autoplay
                .growth
                .bound_since
                .is_some_and(|t| now.duration_since(t) > WALK_TIMEOUT);
            if too_long {
                self.autoplay.note(
                    format!("the walk to the {name} ground is taking too long"),
                    now,
                );
                self.cancel_travel();
            } else if self.traveling() || self.grow_travel_on() {
                self.autoplay.say(
                    Doing::Traveling,
                    format!("going to hunt {name} ({})", about(at.distance(me))),
                );
                return true;
            }
            // A corpse or a fight on the road ends the walk without it
            // having got anywhere. That is a walk to take up again once the
            // character is free, as the run takes up its walk to the
            // counter, and not one that could not get there: read that way,
            // every body looted on the road put the ground on the skip list
            // and sent the character off to another. A walk that took too
            // long was cancelled above, which is not breaking it off.
            //
            // Busy is a target still alive. `attack_target` can go on naming
            // a creature for a moment after it dies (see
            // `steps::worth_fighting`), and a body is no reason to wait.
            let away = at.distance(me);
            let there = here == lb || away <= GROUND_REACH;
            let busy = [self.attack_target, self.autoplay.casting_at()]
                .into_iter()
                .flatten()
                .any(|g| {
                    self.world
                        .objects
                        .get(&g)
                        .is_some_and(|o| o.alive_or_unknown())
                });
            let lately = self
                .autoplay
                .growth
                .bound_walked_on
                .is_some_and(|t| now.duration_since(t) < WALK_ON_EVERY);
            match on_the_way(there, self.journey_broken_off(), busy, lately) {
                OnTheWay::Wait => return true,
                OnTheWay::WalkOn => {
                    if self.grow_travel(at, now) {
                        self.autoplay.growth.bound_walked_on = Some(now);
                        self.autoplay.note(
                            format!("on the way to the {name} ground again ({away:.0} m)"),
                            now,
                        );
                        return true;
                    }
                }
                OnTheWay::There | OnTheWay::Short => {}
            }
            let st = &mut self.autoplay.growth;
            st.bound = None;
            if there {
                st.hunting_at = Some(lb);
                st.quiet_since = None;
                st.roams = 0;
                self.autoplay
                    .note(format!("at the hunting ground: {name}"), now);
            } else {
                st.skip.mark(lb, now);
                self.autoplay
                    .note(format!("could not reach the {name} ground; another"), now);
            }
            return false;
        }
        let level = self.world.stats.level;
        if level <= 0 {
            return false;
        }
        // How long this spot has had nothing on it is kept by the
        // housekeeping, off the world (see
        // `Client::autoplay_watch_the_ground`). It used to be read off
        // the status line here, and so could never reach its hour: a
        // corpse asked about again counted as having something to do and
        // put the clock back to nothing, so nine characters queueing at
        // one body restarted it every couple of seconds and never once
        // walked the two hundred metres they had been given.
        let Some(since) = self.autoplay.growth.quiet_since else {
            return false;
        };
        // A walk already under way is the looking about: it is not cut
        // short to start another.
        if self.traveling() {
            return false;
        }
        let area = self.autoplay.config.fight.area.clone();
        if now.saturating_duration_since(since).as_secs_f32()
            < quiet_before_move(cfg, area.is_some())
        {
            return false;
        }
        if self.autoplay.growth.next_hunt.is_some_and(|t| now < t) {
            return false;
        }
        // In a party only the leader goes looking. The others keep to it
        // (see `Client::autoplay_follow`), and nine characters each
        // picking their own corner scatter the party across the ground
        // instead of moving it.
        let goes_looking = self.followed_leader().is_none();
        // A leader moves the party, not itself: it waits for a follower
        // still fighting or left behind before it goes anywhere (see
        // `Client::waits_for_stragglers`).
        if goes_looking && self.waits_for_stragglers(now) {
            return false;
        }
        // A hunting area is where the hunting is: no other ground is gone
        // to. An outline is walked about, corner by corner, well inside
        // it; a dungeon is the explorer's to walk.
        if let Some(area) = area {
            if !goes_looking {
                return false;
            }
            let Some(corners) = crate::hunt::corners(&area) else {
                return false;
            };
            if corners.len() < crate::hunt::LEAST_CORNERS {
                return false;
            }
            let n = self.autoplay.growth.roams;
            self.autoplay.growth.roams = n.wrapping_add(1);
            self.autoplay.growth.quiet_since = Some(now);
            let goal = crate::hunt::patrol_point(&corners, n);
            if self.travel_about(goal) {
                self.autoplay.say(
                    Doing::Traveling,
                    format!("nothing in sight; looking about {}", area.name),
                );
                return true;
            }
            return false;
        }
        // What to do on a ground with nothing in sight depends on the
        // ground. A few spawn hard enough that standing still is never
        // idle and walking away only leaves the fight; most need
        // covering on foot; a thin one is worth leaving once it is
        // quiet.
        let tactic = ac_world::hunting::tactic_for(cfg.tactic, ac_world::hunting::at(here));
        if tactic == ac_world::hunting::Tactic::Camp
            && self.autoplay.growth.hunting_at == Some(here)
        {
            // Hold the spot. Saying so once is enough; repeating it
            // every frame would drown the log.
            if self.autoplay.doing != Doing::Idle {
                self.autoplay
                    .say(Doing::Idle, "holding the spot; they come to us");
            }
            self.autoplay.growth.quiet_since = Some(now);
            return false;
        }
        // Patrol keeps walking the ground; only Sweep gives up on it.
        let roams_allowed = if tactic == ac_world::hunting::Tactic::Patrol {
            u32::MAX
        } else {
            ROAMS
        };
        if goes_looking
            && self.autoplay.growth.hunting_at == Some(here)
            && self.autoplay.growth.roams < roams_allowed
        {
            let n = self.autoplay.growth.roams % ROAMS;
            let angle = (n as f32 + 0.5) * std::f32::consts::TAU / ROAMS as f32;
            let origin = ac_world::landblock_origin(cell);
            let goal = (me + Vec2::new(angle.cos(), angle.sin()) * ROAM).clamp(
                Vec2::new(origin.x + 10.0, origin.y + 10.0),
                Vec2::new(origin.x + 182.0, origin.y + 182.0),
            );
            self.autoplay.growth.roams += 1;
            self.autoplay.growth.quiet_since = Some(now);
            if self.travel_about(goal) {
                self.autoplay.say(
                    Doing::Traveling,
                    "nothing in sight; looking about the ground",
                );
                return true;
            }
        }
        let st = &mut self.autoplay.growth;
        st.skip.expire(now, SKIP_GROUND_FOR);
        let mut skip: Vec<u32> = st.skip.iter().map(|(g, _)| *g).collect();
        skip.push(here);
        if let Some(h) = st.hunting_at {
            skip.push(h);
        }
        // A ground the player named is where the party hunts. Not the
        // nearest one that suits its level: a place is hunted for its
        // loot, its money, its trophies, and none of that is the level
        // table's business. A mate that does not follow takes its
        // leader's ground the same way, and a follower goes with the
        // leader, so naming one on the leader moves everybody.
        let pinned = (cfg.hunt_at != 0)
            .then(|| ac_world::hunting::at(cfg.hunt_at))
            .flatten()
            .map(|g| (g.landblock, g.at, g.name.clone()));
        let named = pinned.is_some();
        if let Some((lb, at, name)) = pinned.or_else(|| self.party_ground()) {
            if self.autoplay.growth.hunting_at != Some(lb) {
                if self.grow_travel(at, now) {
                    let st = &mut self.autoplay.growth;
                    st.bound = Some((lb, at, name.clone()));
                    st.bound_since = Some(now);
                    st.quiet_since = None;
                    self.autoplay
                        .say(Doing::Traveling, format!("on the way to {name}"));
                    return true;
                }
                if named {
                    // Asked for somewhere it cannot plan a way to. Say
                    // so and try again later; picking somewhere else
                    // would be answering a question nobody asked.
                    self.autoplay.note(
                        format!("cannot find a way to {name} from here; will try again"),
                        now,
                    );
                    self.autoplay.growth.quiet_since = Some(now);
                    self.autoplay.growth.next_hunt = Some(now + RETRY_AFTER);
                    return false;
                }
            } else if named {
                // Standing on the named ground: this is where we hunt,
                // and nothing below gets to move us on.
                self.autoplay.growth.hunting_at = Some(lb);
            }
        }
        if named {
            // The player named this ground. Whatever the tactic makes
            // of a quiet spell, it does not get to go somewhere else.
            return false;
        }
        // Everything below picks a ground of its own, which is the
        // leader's to do. A follower has just skipped the roam for the
        // same reason, and falling through to here from that made the
        // scattering the roam's guard was added to stop: the follower
        // left for a landblock of its own the first time its ground went
        // quiet, instead of staying with the party.
        if !goes_looking {
            return false;
        }
        let Some(g) = ac_world::hunting::nearest_for(level as u32, cfg.level_margin, me, &skip)
        else {
            self.autoplay.note(
                format!(
                    "no hunting ground suits level {level} within {} levels",
                    cfg.level_margin
                ),
                now,
            );
            self.autoplay.growth.quiet_since = Some(now);
            return false;
        };
        let (lb, at, name) = (g.landblock, g.at, g.name.clone());
        let (glo, ghi) = (g.min_level, g.max_level);
        if self.grow_travel(at, now) {
            let st = &mut self.autoplay.growth;
            st.bound = Some((lb, at, name.clone()));
            st.bound_since = Some(now);
            st.quiet_since = None;
            if let Some(h) = st.hunting_at.take() {
                st.skip.mark(h, now);
            }
            self.autoplay.say(
                Doing::Traveling,
                format!(
                    "nothing about; going to hunt {name} (levels {glo}-{ghi}, {})",
                    about(at.distance(me))
                ),
            );
            true
        } else {
            let st = &mut self.autoplay.growth;
            st.skip.mark(lb, now);
            st.quiet_since = None;
            st.next_hunt = Some(now + RETRY_AFTER);
            self.autoplay
                .note(format!("no way to the {name} ground from here"), now);
            false
        }
    }
}

#[cfg(test)]
mod tests;
