//! A pursued hunt's ground with its creature absent: its spawn spots gone round (or, none known,
//! looked over), then the soonest of waiting there and clearing what holds its generators' slots,
//! leaving for it to unload and coming back to a fresh load, or the next ground (`pace`); a
//! rotating hunt's grounds are walked on through, never waited at. A landblock nobody stands in,
//! or outdoors beside, unloads after 5 minutes and rolls its generators afresh when it loads
//! (Landblock.cs:118, 269-303, 1111-1124).

use std::time::{Duration, Instant};

use ac_world::Place;

use super::pace::{Missed, Pace, MISSED_FOR};
use super::HUNT_GROUNDS;
use crate::autoplay::fight::melee::GIVE_UP_FOR;
use crate::autoplay::Doing;
use crate::Client;

/// How long a ground with no spawn spot known is looked over from its place: a generator's
/// heartbeat and the creature's arrival (guess).
const LOOK_FOR: Duration = Duration::from_secs(15);
/// The look is made this near the ground's place, m: a creature is first sent within 112.5 m
/// (ObjectMaint.cs:421), and a landblock is 192 m across.
const LOOK_FROM: f32 = 40.0;
/// A landblock left is gone back to this long after the character was last in or beside it:
/// ACE's 5-minute unload (Landblock.cs:118) and a minute for its check to come round.
const SWEEP_AGAIN: Duration = Duration::from_secs(6 * 60);
/// A choice to wait at a ground is held at least this long before it is weighed again (guess).
const STAY_LEAST: Duration = Duration::from_secs(60);
/// And at most this long: a wait that is taking longer is weighed against leaving again.
const STAY_MOST: Duration = Duration::from_secs(10 * 60);

/// What a ground with its creature absent is left to.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Absent {
    /// Waited at, everything holding a generator's slot cleared.
    Stay,
    /// Left for it to unload, waited out at this spot out of its reach.
    Rotate(Place),
    /// Left for the next ground.
    Move,
}

/// Whether landblock `lb` is `here` or one of the eight round it.
fn is_beside(lb: u32, here: u32) -> bool {
    let (x, y) = ((lb >> 8) as i32, (lb & 0xFF) as i32);
    let (hx, hy) = ((here >> 8) as i32, (here & 0xFF) as i32);
    (x - hx).abs() <= 1 && (y - hy).abs() <= 1
}

impl Client {
    /// Whether the working hunt's best ground comes only by rotating (a rare camp's): its grounds
    /// are swept, nearest first, and its creature chased when seen from the road between them.
    pub(super) fn is_rotating_hunt(&self) -> bool {
        let st = &self.autoplay.growth;
        let best = st.pursuit_paces.first();
        !st.pursuit_sought.is_empty() && best.is_some_and(|g| g.pace.camp > g.pace.rotate)
    }

    /// Whether `lb` is a pursued ground cleared while hunted: camping beats rotating there, and
    /// other creatures hold its droppers' generator slots (a linked placement's are its own).
    pub(super) fn is_cleared_ground(&self, lb: u32, now: Instant) -> bool {
        let held = self
            .pursued_pace(lb)
            .is_some_and(|g| g.droppers.iter().any(|d| d.rate.others > 0.0));
        held && self
            .pace_now(lb, now)
            .is_some_and(|p| p.camp.is_finite() && p.camp <= p.rotate)
    }

    /// What the pursued ground `lb` came to nothing at within [`MISSED_FOR`].
    pub(super) fn ground_missed(&self, lb: u32, now: Instant) -> Missed {
        let st = &self.autoplay.growth;
        Missed {
            stay: st.missed_stay.within(&lb, now, MISSED_FOR),
            back: st.missed_back.within(&lb, now, MISSED_FOR),
        }
    }

    /// The pace of the pursued ground `lb` as things stand: its misses counted.
    pub(super) fn pace_now(&self, lb: u32, now: Instant) -> Option<Pace> {
        let missed = self.ground_missed(lb, now);
        self.pursued_pace(lb).map(|g| g.pace.missed(missed))
    }

    /// Whether the character hunts the pursued ground it stands on: not on a town run, nor on
    /// its way anywhere but round that ground's own spawn spots.
    fn is_at_pursued_ground(&self) -> bool {
        let Some(here) = self.player.as_ref().map(|p| p.cell >> 16) else {
            return false;
        };
        let st = &self.autoplay.growth;
        let own_walk = st.bound_landblock() == Some(here);
        self.is_pursued_hunt()
            && self.pursued_place(here).is_some()
            && st.run.is_none()
            && (own_walk || !self.on_its_way())
    }

    /// Whether the landblock stood in is a cleared pursued ground being hunted: everything on it
    /// is fought, so the generators holding a slot make way for the creature sought.
    pub(crate) fn is_clearing_ground(&self) -> bool {
        let here = self.player.as_ref().map(|p| p.cell >> 16);
        here.is_some() && self.autoplay.growth.clearing == here && self.is_at_pursued_ground()
    }

    /// The hunt's `grounds` sweep: a sought creature in sight chased, the grounds left lately kept
    /// marked while in or beside them, and `grounds` cut to those not left lately (all of them
    /// when every one was), a rotating hunt's to the nearest [`HUNT_GROUNDS`]. `Some` is
    /// `grow_pursuit`'s answer while chasing.
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
        let on_ground = grounds.iter().any(|(lb, _)| *lb == here);
        if self.is_rotating_hunt() || on_ground {
            let sought = self.autoplay.growth.pursuit_sought.clone();
            if let Some(answer) = self.chase_in_sight(title, creature, &sought, now) {
                if on_ground {
                    // It comes here after all: no miss of this ground stands.
                    let st = &mut self.autoplay.growth;
                    st.found_empty.forget(&here);
                    st.missed_stay.forget(&here);
                    st.missed_back.forget(&here);
                    st.staying = None;
                }
                return Some(answer);
            }
        }
        // Stood in or, outdoors, beside, a landblock stays loaded (Landblock.cs:1111-1123): its
        // wait for a fresh load starts once the character is out of reach of it. A dungeon keeps
        // no neighbour (:1116).
        let outdoors = cell & 0xFFFF < 0x100;
        let swept = &mut self.autoplay.growth.swept;
        let near: Vec<u32> = swept
            .iter()
            .map(|(lb, _)| *lb)
            .filter(|lb| *lb == here || (outdoors && is_beside(*lb, here)))
            .collect();
        for lb in near {
            swept.mark(lb, now);
        }
        self.autoplay.growth.swept.expire(now, SWEEP_AGAIN);
        let swept = &self.autoplay.growth.swept;
        let left: Vec<(u32, Place)> = grounds
            .iter()
            .filter(|(lb, _)| swept.since(lb).is_none())
            .copied()
            .collect();
        if !left.is_empty() {
            *grounds = left;
        } else if self.is_rotating_hunt() {
            *grounds = self.grounds_left_longest(here)?;
            return None;
        }
        if self.is_rotating_hunt() && grounds.len() > HUNT_GROUNDS {
            let me = self.my_position()?.truncate();
            grounds.sort_by(|a, b| {
                let away = |p: &Place| p.at.truncate().distance(me);
                away(&a.1).total_cmp(&away(&b.1))
            });
            grounds.truncate(HUNT_GROUNDS);
        }
        None
    }

    /// A rotating hunt's grounds with every one swept lately: the nearest few of those left
    /// longest ago, the likeliest unloaded, never `here` while another is had.
    pub(super) fn grounds_left_longest(&self, here: u32) -> Option<Vec<(u32, Place)>> {
        let me = self.my_position()?.truncate();
        let st = &self.autoplay.growth;
        let mut grounds = st.pursuit_grounds.clone();
        if grounds.len() > 1 {
            grounds.retain(|(lb, _)| *lb != here);
        }
        let away = |p: &Place| p.at.truncate().distance(me);
        grounds.sort_by(|a, b| {
            let left = st.swept.since(&a.0).cmp(&st.swept.since(&b.0));
            left.then(away(&a.1).total_cmp(&away(&b.1)))
        });
        grounds.truncate(HUNT_GROUNDS);
        Some(grounds)
    }

    /// Hunt the pursued ground `here` stands on for `creature`: its spawn spots gone round,
    /// nearest first, or with none known looked over from its place; then, nothing found, the
    /// soonest way to the item ([`Absent`]). `Some` is `grow_pursuit`'s answer; `None` once the
    /// ground is left, for the next.
    pub(super) fn hunt_ground(
        &mut self,
        title: &str,
        creature: &str,
        here: u32,
        now: Instant,
    ) -> Option<Option<bool>> {
        let place = self.pursued_place(here)?;
        if let Some(spot) = self.next_spawn_spot(here) {
            if self.grow_travel(spot, super::super::hunt::ROOM_REACH) {
                let st = &mut self.autoplay.growth;
                st.bound = Some((here, spot.at.truncate(), creature.to_string()));
                st.quiet_since = None;
                let what = if spot.is_indoors() {
                    "room"
                } else {
                    "spawn spot"
                };
                let why = format!("for {title}: to a {creature}'s {what}");
                self.autoplay.say(Doing::Traveling, why);
                return Some(Some(true));
            }
            // No way to it: passed over this time round.
            self.autoplay.growth.spawns_walked.push(spot.cell);
            return Some(Some(false));
        }
        let spots = self.hunts_spawn_spots(here);
        // A dungeon with no spawn of its creature known is the explorer's to walk.
        if !spots && place.is_indoors() {
            return Some(None);
        }
        if !spots {
            if let Some(answer) = self.look_over(here, place, title, creature, now) {
                return Some(answer);
            }
        }
        // A rare spawn's grounds are swept, never waited at: on to the next (Aun Ralirea's).
        if self.is_rotating_hunt() {
            self.leave_ground(here, now);
            return None;
        }
        let st = &mut self.autoplay.growth;
        let choice = match st.staying.filter(|(lb, _)| *lb == here) {
            Some((_, until)) if now < until => Absent::Stay,
            // Waited out with nothing come: camping it counts as never coming a while.
            Some(_) => {
                st.missed_stay.expire(now, MISSED_FOR);
                st.missed_stay.mark(here, now);
                self.choose_when_absent(here, place, now)
            }
            // Absent again on coming back: its load counts as none a while.
            None => {
                if st.found_empty.within(&here, now, MISSED_FOR) {
                    st.missed_back.expire(now, MISSED_FOR);
                    st.missed_back.mark(here, now);
                }
                self.choose_when_absent(here, place, now)
            }
        };
        match choice {
            Absent::Stay => {
                self.start_spawn_round(here);
                let why = format!("for {title}: waiting for the {creature}");
                self.autoplay.say(Doing::Idle, why);
                Some(Some(true))
            }
            Absent::Rotate(spot) => {
                self.leave_ground(here, now);
                self.autoplay.growth.rotating = Some((here, spot));
                self.wait_out_unload(title)
            }
            Absent::Move => {
                self.leave_ground(here, now);
                None
            }
        }
    }

    /// Leave the pursued ground `here`: not gone back to till it has unloaded, nothing cleared.
    fn leave_ground(&mut self, here: u32, now: Instant) {
        let st = &mut self.autoplay.growth;
        st.swept.mark(here, now);
        st.clearing = None;
        st.staying = None;
        st.looking = None;
        st.spawns_walked.clear();
    }

    /// The soonest way to the item from the pursued ground `here` (at `place`) with its creature
    /// absent: waiting, its unload waited out of reach, or the best of the nearest few others
    /// by journey ([`super::pace::Ground::swept_secs`]). A wait is held a while.
    fn choose_when_absent(&mut self, here: u32, place: Place, now: Instant) -> Absent {
        let st = &mut self.autoplay.growth;
        st.found_empty.expire(now, MISSED_FOR);
        st.found_empty.mark(here, now);
        // No pace known: hunted till the quest has what it asks.
        let Some(pace) = self.pace_now(here, now) else {
            self.autoplay.growth.staying = Some((here, now + STAY_MOST));
            return Absent::Stay;
        };
        let me = self.my_position().map(|m| m.truncate());
        let swept = &self.autoplay.growth.swept;
        let mut others: Vec<(u32, Place)> = self
            .autoplay
            .growth
            .pursuit_grounds
            .iter()
            .filter(|(lb, _)| *lb != here && swept.since(lb).is_none())
            .copied()
            .collect();
        others.sort_by(|a, b| {
            let away = |p: &Place| me.map_or(0.0, |m| p.at.truncate().distance(m));
            away(&a.1).total_cmp(&away(&b.1))
        });
        others.truncate(HUNT_GROUNDS);
        let quests = self.travel.quests_done.clone();
        let mut move_secs = f32::INFINITY;
        for (lb, at) in others {
            let Some(t) = self.journey_plan(at, &quests) else {
                continue;
            };
            let missed = self.ground_missed(lb, now);
            if let Some(g) = self.pursued_pace(lb) {
                move_secs = move_secs.min(g.swept_secs(t.seconds, missed));
            }
        }
        let (stay, rotate) = (pace.camp, pace.rotate);
        let spot = if rotate < stay.min(move_secs) {
            self.out_of_reach(here, place)
        } else {
            None
        };
        let to = ac_world::quest_chain::place_name(place.cell);
        tracing::info!(
            "pursuit: nothing in {to}: wait {stay:.0} s, unload {rotate:.0} s, next {move_secs:.0} s"
        );
        match spot {
            Some(spot) => Absent::Rotate(spot),
            None if move_secs < stay => Absent::Move,
            // Nowhere else and no waiting it out: a wait here is all there is.
            None => {
                let hold = Duration::from_secs_f32(stay.clamp(0.0, STAY_MOST.as_secs_f32()));
                let until = now + hold.max(STAY_LEAST);
                self.autoplay.growth.staying = Some((here, until));
                Absent::Stay
            }
        }
    }

    /// A spot out of reach of landblock `lb` (its ground at `place`), so it unloads: a dungeon's
    /// way in from outdoors, else the nearest landblock two away on land.
    fn out_of_reach(&mut self, lb: u32, place: Place) -> Option<Place> {
        if place.is_indoors() {
            return ac_world::quest_chain::ways_in(lb)
                .into_iter()
                .find(|p| p.mouth_outdoors() && p.from_cell >> 16 != lb)
                .map(|p| Place {
                    at: p.from,
                    cell: p.from_cell,
                });
        }
        let me = self.my_position()?.truncate();
        let (x, y) = ((lb >> 8) as i32, (lb & 0xFF) as i32);
        let mut ring: Vec<glam::Vec2> = Vec::new();
        for (dx, dy) in (-2..=2).flat_map(|dx| (-2..=2).map(move |dy| (dx, dy))) {
            let (bx, by) = (x + dx, y + dy);
            if dx.abs().max(dy.abs()) != 2 || !(0..0xFF).contains(&bx) || !(0..0xFF).contains(&by) {
                continue;
            }
            let block = (bx as u32) << 8 | by as u32;
            let middle = ac_world::landblock_origin(block << 16) + glam::Vec3::new(96.0, 96.0, 0.0);
            ring.push(middle.truncate());
        }
        ring.sort_by(|a, b| a.distance(me).total_cmp(&b.distance(me)));
        let on_land = ring.into_iter().find(|at| !self.is_at_sea(*at))?;
        Some(Place::on_map(on_land))
    }

    /// A ground left to unload, waited out at a spot out of its reach till it may be gone back
    /// to; `None` once it may (or none is being waited out).
    pub(super) fn wait_out_unload(&mut self, title: &str) -> Option<Option<bool>> {
        let (lb, spot) = self.autoplay.growth.rotating?;
        if self.autoplay.growth.swept.since(&lb).is_none() {
            self.autoplay.growth.rotating = None;
            return None;
        }
        let to = self
            .pursued_place(lb)
            .map_or_else(String::new, |p| ac_world::quest_chain::place_name(p.cell));
        let me = self.my_position()?;
        if spot.at.truncate().distance(me.truncate()) > super::GROUND_REACH {
            if self.grow_travel(spot, super::GROUND_REACH * 0.5) {
                let why = format!("for {title}: out of reach of {to} for it to unload");
                self.autoplay.say(Doing::Traveling, why);
                return Some(Some(true));
            }
            // No way out of its reach: gone back to at once.
            self.autoplay.growth.rotating = None;
            self.autoplay.growth.swept.forget(&lb);
            return None;
        }
        let why = format!("for {title}: waiting for {to} to unload");
        self.autoplay.say(Doing::Idle, why);
        Some(Some(true))
    }

    /// A creature of the classes `sought` in sight walked to (or fought, `Some(None)`); `None`
    /// when none is. One with no way to it is given up on a while, like the fight's, and the
    /// hunt goes on. A namesake is noted: the common Mosswart Mire-Witch (7102) is not Martine's
    /// (9163).
    pub(super) fn chase_in_sight(
        &mut self,
        title: &str,
        creature: &str,
        sought: &[u32],
        now: Instant,
    ) -> Option<Option<bool>> {
        let me = self.my_position()?;
        let given_up = &self.autoplay.given_up;
        let alive = self.world.objects.values().filter(|o| o.alive_or_unknown());
        let (mut seen, mut namesake): (Option<(u32, glam::Vec3)>, Option<u32>) = (None, None);
        for o in alive {
            let Some(at) = o.world_pos() else { continue };
            if sought.contains(&o.weenie_class_id) {
                if given_up.within(&o.guid, now, GIVE_UP_FOR) {
                    continue;
                }
                if seen.is_none_or(|(_, s)| at.distance(me) < s.distance(me)) {
                    seen = Some((o.guid, at));
                }
            } else if o.name == creature {
                namesake = Some(o.weenie_class_id);
            }
        }
        if let (None, Some(wcid)) = (seen, namesake) {
            let why = format!("a {creature} ({wcid}) in sight, not the one {title} wants");
            self.autoplay.note(why, now);
        }
        let (guid, it) = seen?;
        let radius = self.autoplay.config.fight.radius;
        let near = it.distance(me) <= radius * 0.8;
        if near || self.grow_travel(self.place_at(it), radius * 0.5) {
            self.autoplay.growth.looking = None;
            self.autoplay.growth.rotating = None;
            if near {
                return Some(None);
            }
            let why = format!("for {title}: to the {creature} in sight");
            self.autoplay.say(Doing::Traveling, why);
            return Some(Some(true));
        }
        let why = format!("no way to the {creature} in sight: hunting on");
        self.autoplay.note(why, now);
        self.autoplay.given_up.expire(now, GIVE_UP_FOR);
        self.autoplay.given_up.mark(guid, now);
        None
    }

    /// Look ground `here` over from its place `at` for [`LOOK_FOR`]; `None` once looked over.
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
        let since = match self.autoplay.growth.looking {
            Some((lb, t)) if lb == here => t,
            _ if at.at.truncate().distance(me.truncate()) > LOOK_FROM => {
                if self.grow_travel(at, LOOK_FROM * 0.5) {
                    let why = format!("for {title}: to look for {creature} in {to}");
                    self.autoplay.say(Doing::Traveling, why);
                    return Some(Some(true));
                }
                return None;
            }
            _ => {
                self.autoplay.growth.looking = Some((here, now));
                now
            }
        };
        // A rotating hunt's look is the walk there: its sweep never stands.
        let look_for = match self.is_rotating_hunt() {
            true => Duration::ZERO,
            false => LOOK_FOR,
        };
        if now.duration_since(since) < look_for {
            let why = format!("for {title}: looking for {creature} in {to}");
            self.autoplay.say(Doing::Idle, why);
            return Some(Some(true));
        }
        None
    }
}
