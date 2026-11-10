//! A rare spawn is swept for: a creature under [`RARE_SPAWN`] of its generators' rolls is looked
//! for landblock by landblock, the nearest not looked over lately first, never waited for at one
//! (the Mosswart Mire-Witch, 0.49% of a Marae Lassel shore generator's rolls). A landblock nobody
//! stands in or beside unloads after 5 minutes and rolls its encounters again when it loads
//! (Landblock.cs:118, 548; 269-290). Any other hunt camps, and clears all its ground holds.

use std::time::{Duration, Instant};

use ac_world::Place;

use super::HUNT_GROUNDS;
use crate::autoplay::Doing;
use crate::Client;

/// A creature spawning on fewer of its generators' rolls than this is swept for, not camped.
pub(crate) const RARE_SPAWN: f32 = 0.05;
/// How long a landblock is looked over from its middle: a generator's heartbeat and the
/// creature's arrival (guess).
const LOOK_FOR: Duration = Duration::from_secs(15);
/// The look is made this near the ground's place, m: a creature is first sent within 112.5 m
/// (ObjectMaint.cs:421), and a landblock is 192 m across.
const LOOK_FROM: f32 = 40.0;
/// A landblock looked over is looked at again after this: twice ACE's 5-minute unload.
const SWEEP_AGAIN: Duration = Duration::from_secs(10 * 60);

/// Whether creature `wcid` is a rare spawn, swept for.
pub(crate) fn is_rare_spawn(wcid: u32) -> bool {
    ac_world::quests::spawn_share(wcid) < RARE_SPAWN
}

impl Client {
    /// Whether the working hunt's creatures are all rare spawns.
    pub(super) fn is_rare_hunt(&self) -> bool {
        let sought = &self.autoplay.growth.pursuit_sought;
        !sought.is_empty() && sought.iter().all(|w| is_rare_spawn(*w))
    }

    /// Whether `o`'s landblock is a camped pursued ground: everything on it is fought, so the
    /// generators holding a slot make way for the creature sought.
    pub(crate) fn is_clearing_ground(&self) -> bool {
        let Some(here) = self.player.as_ref().map(|p| p.cell >> 16) else {
            return false;
        };
        self.autoplay.growth.clearing == Some(here)
            && self.is_pursued_hunt()
            && self.pursued_place(here).is_some()
    }

    /// The sweep of a rare hunt's `grounds` for `creature`, `title`'s: the ground stood on looked
    /// over, then `grounds` cut to the nearest not looked over lately (all of them when every one
    /// was: then camped). `Some` is `grow_pursuit`'s answer while looking.
    pub(super) fn sweep(
        &mut self,
        title: &str,
        creature: &str,
        grounds: &mut Vec<(u32, Place)>,
        now: Instant,
    ) -> Option<Option<bool>> {
        let cell = self.player.as_ref()?.cell;
        let here = cell >> 16;
        self.autoplay.growth.swept.expire(now, SWEEP_AGAIN);
        let outdoors = cell & 0xFFFF < 0x100;
        let stood = grounds.iter().find(|(lb, _)| *lb == here).map(|g| g.1);
        if let (true, Some(at)) = (outdoors, stood) {
            if self.autoplay.growth.swept.since(&here).is_none() {
                if let Some(answer) = self.look_over(here, at, title, creature, now) {
                    return Some(answer);
                }
            }
        }
        let me = self.my_position()?.truncate();
        let swept = &self.autoplay.growth.swept;
        let left: Vec<(u32, Place)> = grounds
            .iter()
            .filter(|(lb, _)| swept.since(lb).is_none())
            .copied()
            .collect();
        if !left.is_empty() {
            *grounds = left;
        }
        grounds.sort_by(|a, b| {
            let away = |p: &Place| p.at.truncate().distance(me);
            away(&a.1).total_cmp(&away(&b.1))
        });
        grounds.truncate(HUNT_GROUNDS);
        None
    }

    /// Look ground `here` over from its place `at` for a sought creature: one in sight is walked
    /// to (or fought, `Some(None)`); none after [`LOOK_FOR`], the ground is marked swept and `None`
    /// says on to the next.
    fn look_over(
        &mut self,
        here: u32,
        at: Place,
        title: &str,
        creature: &str,
        now: Instant,
    ) -> Option<Option<bool>> {
        let me = self.my_position()?;
        let sought = &self.autoplay.growth.pursuit_sought;
        let seen = self
            .world
            .objects
            .values()
            .filter(|o| sought.contains(&o.weenie_class_id) && o.alive_or_unknown())
            .filter_map(|o| o.world_pos())
            .min_by(|a, b| a.distance(me).total_cmp(&b.distance(me)));
        if let Some(it) = seen {
            self.autoplay.growth.looking = None;
            let radius = self.autoplay.config.fight.radius;
            if it.distance(me) > radius * 0.8 && self.grow_travel(self.place_at(it), radius * 0.5) {
                let why = format!("for {title}: to the {creature} in sight");
                self.autoplay.say(Doing::Traveling, why);
                return Some(Some(true));
            }
            return Some(None);
        }
        let to = ac_world::quest_chain::place_name(at.cell);
        if at.at.truncate().distance(me.truncate()) > LOOK_FROM {
            if self.grow_travel(at, LOOK_FROM * 0.5) {
                let why = format!("for {title}: to look for {creature} in {to}");
                self.autoplay.say(Doing::Traveling, why);
                return Some(Some(true));
            }
            self.autoplay.growth.swept.mark(here, now);
            return None;
        }
        let since = match self.autoplay.growth.looking {
            Some((lb, t)) if lb == here => t,
            _ => {
                self.autoplay.growth.looking = Some((here, now));
                now
            }
        };
        if now.duration_since(since) < LOOK_FOR {
            let why = format!("for {title}: looking for {creature} in {to}");
            self.autoplay.say(Doing::Idle, why);
            return Some(Some(true));
        }
        tracing::info!("pursuit: no {creature} in {to}: on to the next");
        self.autoplay.growth.swept.mark(here, now);
        self.autoplay.growth.looking = None;
        None
    }
}
