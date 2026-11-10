//! How soon a pursued hunt's ground gives its item: each dropper's kill by the fight's own offline
//! estimate (`casts_on_kind`), against how fast it comes there (`SourceGround::rate`). Grounds are
//! ranked by the journey plus these seconds: a dungeon of 2% droppers dying in a cast or two
//! beats a far ground of rare 80% ones.

use std::cell::RefCell;
use std::collections::HashMap;

use ac_world::quest_chain::ROTATE_SECS;
use ac_world::quests::SpawnRate;

use crate::autoplay::fight::kill::Wand;
use crate::Client;

/// Seconds a cast takes, wind-up to landing (guess; Blargerton's telemetry: about 14 s a kill).
pub(crate) const CAST_SECS: f32 = 2.5;
/// Seconds between kills: the walk to the next and the looting (guess, as [`CAST_SECS`]).
pub(crate) const KILL_WALK_SECS: f32 = 8.0;
/// A kill with no offline estimate, a swing's or a shot's: Blargerton's telemetry, 14 s a kill.
pub(crate) const KILL_SECS: f32 = 14.0;

/// How soon one ground gives the item, in seconds there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Pace {
    /// Camped: its droppers killed as its generators make them again.
    pub(crate) camp: f32,
    /// Left for it to unload and come back to what stands at load (a round: `round` of
    /// [`ground_pace`]).
    pub(crate) rotate: f32,
}

impl Pace {
    /// The sooner of camping and rotating: a visit does one of them, the ground's rank.
    pub(crate) fn soonest(&self) -> f32 {
        self.camp.min(self.rotate)
    }
}

/// One dropper on a ground: its chance a kill, seconds a kill, and how fast it comes there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Dropper {
    pub(crate) chance: f32,
    pub(crate) kill: f32,
    pub(crate) rate: SpawnRate,
}

/// A dropper camped: its spawns a second while all its generators make is killed, and the
/// seconds each costs, the others killed to free its slot counted (each a kill like its own).
fn camped(d: &Dropper) -> (f32, f32) {
    let per_sec = d.rate.per_hour / 3600.0;
    if per_sec <= 0.0 {
        return (0.0, d.kill);
    }
    (per_sec, d.kill * (1.0 + d.rate.others / d.rate.per_hour))
}

/// A dropper's spawns a second come to after each unload, `round` seconds away a time: its load,
/// killed, every round; and the seconds each costs.
fn rotated(d: &Dropper, round: f32) -> (f32, f32) {
    if d.rate.load <= 0.0 {
        return (0.0, d.kill);
    }
    (d.rate.load / (d.rate.load * d.kill + round), d.kill)
}

/// Items a second from `droppers`, each coming at `spawn`'s rate a second and costing its
/// seconds: one kill at a time, so the time is shared, best chance a second first.
fn items_per_sec(droppers: &[Dropper], spawn: impl Fn(&Dropper) -> (f32, f32)) -> f32 {
    let mut by_worth: Vec<(f32, f32, f32)> = droppers
        .iter()
        .map(|d| {
            let (rate, kill) = spawn(d);
            (d.chance, rate, kill)
        })
        .collect();
    by_worth.sort_by(|a, b| (b.0 / b.2).total_cmp(&(a.0 / a.2)));
    let (mut budget, mut items) = (1.0_f32, 0.0_f32);
    for (chance, rate, kill) in by_worth {
        let r = rate.min(budget / kill);
        budget -= r * kill;
        items += chance * r;
        if budget <= 0.0 {
            break;
        }
    }
    items
}

/// The pace of a ground with `droppers` on it, each round away from it taking `round` seconds
/// ([`ac_world::quest_chain::ROTATE_SECS`] to unload and come back); `None` when none comes.
pub(crate) fn ground_pace(droppers: &[Dropper], round: f32) -> Option<Pace> {
    let secs = |items: f32| {
        if items > 0.0 {
            1.0 / items
        } else {
            f32::INFINITY
        }
    };
    let pace = Pace {
        camp: secs(items_per_sec(droppers, camped)),
        rotate: secs(items_per_sec(droppers, |d| rotated(d, round))),
    };
    pace.soonest().is_finite().then_some(pace)
}

/// A pursued hunt's ground: the droppers on it this character can kill, and its pace.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Ground {
    pub(crate) droppers: Vec<Dropper>,
    /// With [`ROTATE_SECS`] a round: what the grounds are ranked by.
    pub(crate) pace: Pace,
}

impl Ground {
    /// Its droppers' pace; `None` when none of them comes.
    pub(crate) fn of(droppers: Vec<Dropper>) -> Option<Ground> {
        let pace = ground_pace(&droppers, ROTATE_SECS)?;
        Some(Ground { droppers, pace })
    }

    /// Seconds to the item come to fresh at the end of a `journey`, swept between it and grounds
    /// like it whenever camping is slower: the walk to the next is the round, no unload waited out.
    pub(crate) fn swept_secs(&self, journey: f32) -> f32 {
        let swept = ground_pace(&self.droppers, journey.max(1.0));
        journey + swept.map_or(f32::INFINITY, |p| p.soonest())
    }
}

/// Of `droppers` (class, chance a kill, seconds a kill), those not `sought` whose kill met on the
/// road saves more than it costs: its chance of the plan's `plan` seconds over its kill's. A 2%
/// Gotrok Tiatus at 25 s is fought only while the plan is over 1250 s away.
pub(crate) fn road_droppers(droppers: &[(u32, f32, f32)], sought: &[u32], plan: f32) -> Vec<u32> {
    droppers
        .iter()
        .filter(|(w, chance, kill)| !sought.contains(w) && chance * plan > *kill)
        .map(|d| d.0)
        .collect()
}

/// Seconds a kill of each creature takes this character, by class, worked out once each; `None`
/// for one it cannot kill reliably: past a loot tier above the highest it killed (`proven_tier`),
/// or a caster's spells cannot hurt it (immune, none ready).
pub(crate) struct Killing<'a> {
    client: &'a Client,
    ready: Vec<u32>,
    wands: Vec<Wand>,
    survived: u32,
    seen: RefCell<HashMap<u32, Option<f32>>>,
}

impl Killing<'_> {
    /// Seconds a kill of creature `wcid` takes, the walk to the next counted.
    pub(crate) fn secs(&self, wcid: u32) -> Option<f32> {
        if let Some(s) = self.seen.borrow().get(&wcid) {
            return *s;
        }
        let s = self.work_out(wcid);
        self.seen.borrow_mut().insert(wcid, s);
        s
    }

    fn work_out(&self, wcid: u32) -> Option<f32> {
        // By class: two Mosswart Mire-Witches, 7102 and 9163.
        let Some(known) = ac_world::elements::creature_by_id(wcid) else {
            return Some(KILL_SECS);
        };
        if known.tier > self.survived {
            return None;
        }
        // No spell ready or no health known: no estimate, the telemetry's kill.
        if self.ready.is_empty() || known.health == 0 {
            return Some(KILL_SECS);
        }
        let casts = self
            .client
            .casts_on_kind(known, &self.ready, &self.wands)
            .filter(|c| c.is_finite() && *c > 0.0)?;
        Some(casts * CAST_SECS + KILL_WALK_SECS)
    }
}

impl Client {
    /// What a kill of each creature costs this character as it stands: its spells ready and wands,
    /// as `best_ground` weighs a ground (a melee character's kills are [`KILL_SECS`] each).
    pub(crate) fn killing(&self) -> Killing<'_> {
        let fight = crate::autoplay::Fight {
            pick_weapon: false,
            ..self.autoplay.config.fight.clone()
        };
        let ready: Vec<u32> = if self.hits_with_weapons() {
            Vec::new()
        } else {
            let ready = self.ready_spells(&fight).into_iter();
            ready.map(|(id, _)| id).collect()
        };
        Killing {
            client: self,
            wands: self.wands_to_hand(&fight),
            ready,
            survived: self.proven_tier() + 1,
            seen: RefCell::new(HashMap::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dropper(chance: f32, kill: f32, load: f32, per_hour: f32) -> Dropper {
        Dropper {
            chance,
            kill,
            rate: SpawnRate {
                load,
                per_hour,
                others: 0.0,
            },
        }
    }

    #[test]
    fn a_dropper_on_the_road_is_worth_a_fight_only_against_a_long_plan() {
        let gotrok = [(24497, 0.02, 25.0)];
        assert!(road_droppers(&gotrok, &[], 1000.0).is_empty());
        assert_eq!(road_droppers(&gotrok, &[], 1500.0), [24497]);
        assert!(
            road_droppers(&gotrok, &[24497], 1500.0).is_empty(),
            "sought already"
        );
    }

    #[test]
    fn a_dungeon_of_sure_spawns_is_kill_limited() {
        // The Lost Distillery's K'nath: 20%, a cast or so, nine at load, and with its kin more
        // an hour than are killed.
        let p = ground_pace(&[dropper(0.2, 10.5, 9.0, 900.0)], ROTATE_SECS).unwrap();
        assert!((p.camp - 52.5).abs() < 0.1, "{p:?}");
        assert_eq!(p.soonest(), p.camp);
    }

    #[test]
    fn a_rare_camp_comes_only_by_rotating() {
        // A camp at 0.2 at load and none an hour: never camped, and rotated slowly.
        let p = ground_pace(&[dropper(0.8, 20.0, 0.2, 0.0)], ROTATE_SECS).unwrap();
        assert!(p.camp.is_infinite());
        assert!(p.rotate > 2000.0 && p.rotate == p.soonest(), "{p:?}");
        assert_eq!(
            ground_pace(&[dropper(0.8, 20.0, 0.0, 0.0)], ROTATE_SECS),
            None
        );
        // Swept with the next a minute off, it comes sooner than waiting out an unload here.
        let g = Ground::of(vec![dropper(0.8, 20.0, 0.2, 0.0)]).unwrap();
        assert!(g.swept_secs(60.0) < p.rotate, "{}", g.swept_secs(60.0));
    }

    #[test]
    fn droppers_on_one_ground_share_the_kill_time() {
        // Two droppers each spawning faster than they can be killed: the better one takes the time.
        let both = [
            dropper(0.2, 10.0, 9.0, 900.0),
            dropper(0.02, 10.0, 9.0, 900.0),
        ];
        let alone = ground_pace(&both[..1], ROTATE_SECS).unwrap();
        let both = ground_pace(&both, ROTATE_SECS).unwrap();
        assert_eq!(both.camp, alone.camp);
    }

    #[test]
    fn a_camp_cleared_slower_than_its_others_come_comes_slower() {
        // 3 an hour while 840 others are killed at 10 s each: 281 kills a spawn, 2810 s an item.
        let mut d = dropper(1.0, 10.0, 0.0, 3.0);
        d.rate.others = 840.0;
        let p = ground_pace(&[d], ROTATE_SECS).unwrap();
        assert!((p.camp - 2810.0).abs() < 1.0, "{p:?}");
    }

    #[test]
    fn the_time_clearing_takes_is_not_spent_on_the_droppers_too() {
        // 270 an hour at 10 s a kill takes three quarters of the time; as many others cleared
        // take a kill each too, so only one a 20 s comes: 40 s an item, not 26.7.
        let mut d = dropper(0.5, 10.0, 0.0, 270.0);
        let alone = ground_pace(&[d], ROTATE_SECS).unwrap();
        d.rate.others = 270.0;
        let cleared = ground_pace(&[d], ROTATE_SECS).unwrap();
        assert!((alone.camp - 26.67).abs() < 0.1, "{alone:?}");
        assert!((cleared.camp - 40.0).abs() < 0.1, "{cleared:?}");
    }

    #[test]
    fn a_ground_ranks_by_the_one_way_a_visit_takes_it() {
        // One dropper only camped (none at load), another only rotated (none an hour): a visit
        // does one, so the ground is no sooner than the better of the two alone.
        let camped = dropper(0.5, 10.0, 0.0, 36.0);
        let rotated = dropper(0.5, 10.0, 4.0, 0.0);
        let p = ground_pace(&[camped, rotated], ROTATE_SECS).unwrap();
        let a = ground_pace(&[camped], ROTATE_SECS).unwrap().soonest();
        let b = ground_pace(&[rotated], ROTATE_SECS).unwrap().soonest();
        assert_eq!(p.soonest(), a.min(b), "{p:?}");
    }
}
