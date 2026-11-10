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
/// A landblock looked over is looked at again this long after the character was last in or beside
/// it: twice ACE's 5-minute unload (Landblock.cs:118, 548).
const SWEEP_AGAIN: Duration = Duration::from_secs(10 * 60);

/// Whether landblock `lb` is `here` or one of the eight round it.
fn is_beside(lb: u32, here: u32) -> bool {
    let (x, y) = ((lb >> 8) as i32, (lb & 0xFF) as i32);
    let (hx, hy) = ((here >> 8) as i32, (here & 0xFF) as i32);
    (x - hx).abs() <= 1 && (y - hy).abs() <= 1
}

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
        // Met anywhere, on a ground or between two: the fight rule only sees its own radius.
        if let Some(answer) = self.chase_in_sight(title, creature, now) {
            return Some(answer);
        }
        let outdoors = cell & 0xFFFF < 0x100;
        // Stood in or beside, a landblock stays loaded (Landblock.cs:1111-1123): its wait for a
        // fresh roll starts once the character is out of reach of it.
        if outdoors {
            let swept = &mut self.autoplay.growth.swept;
            let near: Vec<u32> = swept
                .iter()
                .map(|(lb, _)| *lb)
                .filter(|lb| is_beside(*lb, here))
                .collect();
            for lb in near {
                swept.mark(lb, now);
            }
        }
        self.autoplay.growth.swept.expire(now, SWEEP_AGAIN);
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

    /// A sought creature in sight walked to (or fought, `Some(None)`); `None` when none is. A
    /// namesake is noted: the common Mosswart Mire-Witch (7102) is not Martine's (9163).
    pub(super) fn chase_in_sight(
        &mut self,
        title: &str,
        creature: &str,
        now: Instant,
    ) -> Option<Option<bool>> {
        let me = self.my_position()?;
        let sought = &self.autoplay.growth.pursuit_sought;
        let alive = self.world.objects.values().filter(|o| o.alive_or_unknown());
        let (mut seen, mut namesake): (Option<glam::Vec3>, Option<u32>) = (None, None);
        for o in alive {
            let Some(at) = o.world_pos() else { continue };
            if sought.contains(&o.weenie_class_id) {
                if seen.is_none_or(|s| at.distance(me) < s.distance(me)) {
                    seen = Some(at);
                }
            } else if o.name == creature {
                namesake = Some(o.weenie_class_id);
            }
        }
        if let (None, Some(wcid)) = (seen, namesake) {
            let why = format!("a {creature} ({wcid}) in sight, not the one {title} wants");
            self.autoplay.note(why, now);
        }
        let it = seen?;
        self.autoplay.growth.looking = None;
        let radius = self.autoplay.config.fight.radius;
        if it.distance(me) > radius * 0.8 && self.grow_travel(self.place_at(it), radius * 0.5) {
            let why = format!("for {title}: to the {creature} in sight");
            self.autoplay.say(Doing::Traveling, why);
            return Some(Some(true));
        }
        Some(None)
    }

    /// Look ground `here` over from its place `at`: none of its sought creatures in sight (asked
    /// first, `chase_in_sight`) after [`LOOK_FOR`], it is marked swept and `None` says on.
    fn look_over(
        &mut self,
        here: u32,
        at: Place,
        title: &str,
        creature: &str,
        now: Instant,
    ) -> Option<Option<bool>> {
        let me = self.my_position()?;
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
