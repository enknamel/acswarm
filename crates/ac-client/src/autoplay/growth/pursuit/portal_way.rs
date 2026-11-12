//! A portal a camp puts out had first when only its exit leads on to a pursued place: the Royal
//! Chest's hive (0x028A) is reached by a Small Hive Portal alone, a rare pick of the Marae Lassel
//! plateau's camps (`portals::spawned`). Its landblocks are hunted as a rare creature's are, and
//! once it stands in view the trip planner takes it (`standing_ways`): the board's own step again.

use std::time::Instant;

use ac_world::quest_chain::ROTATE_SECS;
use ac_world::trip::Prefs;
use ac_world::Place;

use super::{pace, stuck, Ask, Hunting, PursuitStep, PACED_GROUNDS, SWEEP_GROUNDS};
use crate::Client;

/// Seconds to walk into a portal come in view, as a kill's in a hunt's pace (guess).
const GO_THROUGH_SECS: f32 = 10.0;

/// The camp-made portal `quest`'s pursuit hunts first, since only its exit leads on.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PortalWanted {
    pub quest: String,
    pub wcid: u32,
    pub name: String,
    /// Its exit's landblock: stood in, the portal was gone through.
    pub exit_block: u32,
}

impl Client {
    /// The camp-made portal letting this character through whose exit a journey goes on from to
    /// one of `grounds`, the one whose landblocks put it out fastest; `None` when none does.
    fn portal_way(&mut self, grounds: &[(u32, Place)], quests: &[String]) -> Option<PortalWanted> {
        let level = self.world.stats.level.max(1) as u32;
        let goals: Vec<Place> = grounds.iter().map(|g| self.planning_place(g.1)).collect();
        let mut best: Option<(f32, PortalWanted)> = None;
        let mut tried: Vec<u32> = Vec::new();
        for s in ac_world::portals::spawned() {
            if tried.contains(&s.wcid) {
                continue;
            }
            tried.push(s.wcid);
            if !s.way.usable_by(level, quests) {
                continue;
            }
            let exit = Place {
                at: s.way.to,
                cell: s.way.to_cell,
            };
            let prefs: Prefs = self.travel_prefs().in_dungeon(exit.is_indoors());
            let on = goals.iter().any(|goal| {
                let plan = ac_world::trip::plan_with_recalls_and_gems(
                    exit,
                    *goal,
                    level,
                    quests,
                    &[],
                    &[],
                    &[],
                    &[],
                    prefs,
                );
                plan.is_some()
            });
            if !on {
                continue;
            }
            let rows = ac_world::portals::spawned().iter();
            let per_sec: f32 = rows
                .filter(|r| r.wcid == s.wcid)
                .map(|r| (r.rate.per_hour / 3600.0).max(r.rate.load / ROTATE_SECS))
                .sum();
            if best.as_ref().is_some_and(|(b, _)| *b >= per_sec) {
                continue;
            }
            let wanted = PortalWanted {
                quest: String::new(),
                wcid: s.wcid,
                name: s.way.name.clone(),
                exit_block: s.way.to_cell >> 16,
            };
            best = Some((per_sec, wanted));
        }
        best.map(|(_, p)| p)
    }

    /// With no journey to `grounds` (`what` in words), have quest `id` (`title`) hunt a
    /// camp-made portal whose exit leads on first; whether one was wanted. Once a quest's: the
    /// portal's own camps out of reach are not looked past again.
    pub(super) fn want_portal_way(
        &mut self,
        (id, title): (&str, &str),
        what: &str,
        grounds: &[(u32, Place)],
        quests: &[String],
        now: Instant,
    ) -> bool {
        if self
            .autoplay
            .growth
            .pursuit_portals
            .iter()
            .any(|p| p.quest == id)
        {
            return false;
        }
        let Some(portal) = self.portal_way(grounds, quests) else {
            return false;
        };
        let why = format!(
            "for {title}: a {} first, to reach {what}: hunted where camps put it out",
            portal.name
        );
        tracing::info!("pursuit: {id}: {why}");
        self.autoplay.note(why, now);
        let st = &mut self.autoplay.growth;
        st.pursuit_portals.push(PortalWanted {
            quest: id.to_string(),
            ..portal
        });
        // Passed over for want of a way: through the portal they are tried again.
        for (lb, _) in grounds {
            st.skip.forget(lb);
        }
        self.quest_board_at = None;
        true
    }

    /// Let each wanted portal go once its exit's landblock is stood in: gone through.
    pub(super) fn keep_portal_ways(&mut self) {
        let Some(here) = self.player.as_ref().map(|p| p.cell >> 16) else {
            return;
        };
        self.autoplay.growth.pursuit_portals.retain(|p| {
            let through = p.exit_block == here;
            if through {
                tracing::info!("pursuit: {}: through the {}", p.quest, p.name);
            }
            !through
        });
    }

    /// The wanted portals the board hunts, by quest: those not standing in view, which the trip
    /// planner takes as they stand.
    pub(super) fn portal_hunts(&self) -> std::collections::BTreeMap<String, (u32, String)> {
        self.autoplay
            .growth
            .pursuit_portals
            .iter()
            .filter(|p| !self.is_portal_in_view(p.wcid))
            .map(|p| (p.quest.clone(), (p.wcid, p.name.clone())))
            .collect()
    }

    /// Whether a wanted portal hunted now stands in view: the board is worked out again, and
    /// its step's journey goes through it.
    pub(super) fn is_sought_portal_seen(&self) -> bool {
        let hunted = self.autoplay.growth.pursuit_sought.iter();
        let wanted = &self.autoplay.growth.pursuit_portals;
        hunted
            .filter(|w| wanted.iter().any(|p| p.wcid == **w))
            .any(|w| self.is_portal_in_view(*w))
    }

    /// Whether a portal of class `wcid` stands in view.
    fn is_portal_in_view(&self, wcid: u32) -> bool {
        let objects = self.world.objects.values();
        objects
            .filter(|o| o.weenie_class_id == wcid)
            .any(|o| o.position.is_some())
    }
}

impl Ask<'_> {
    /// The hunt for the camp-made portal of class `wcid` (`name`): the landblocks putting it out,
    /// soonest first, swept and left for a fresh load as a rare creature's grounds are.
    pub(super) fn portal_hunt(&self, wcid: u32, name: &str) -> (PursuitStep, Option<Hunting>) {
        let rows = ac_world::portals::spawned()
            .iter()
            .filter(|s| s.wcid == wcid);
        let mut paced: Vec<(u32, Place, pace::Ground)> = rows
            .filter_map(|s| {
                let d = pace::Dropper {
                    chance: 1.0,
                    kill: GO_THROUGH_SECS,
                    rate: s.rate,
                };
                Some((s.landblock(), s.place(), pace::Ground::of(vec![d])?))
            })
            .collect();
        paced.sort_by(|a, b| a.2.pace.soonest().total_cmp(&b.2.pace.soonest()));
        let rotating = paced
            .first()
            .is_some_and(|p| p.2.pace.camp > p.2.pace.rotate);
        paced.truncate(if rotating {
            SWEEP_GROUNDS
        } else {
            PACED_GROUNDS
        });
        let Some(&(landblock, place, _)) = paced.first() else {
            return (stuck(format!("no camp known puts out a {name}")), None);
        };
        let step = PursuitStep::Hunt {
            landblock,
            creature: name.to_string(),
            place: ac_world::quest_chain::place_name(place.cell),
            more: 1,
        };
        let hunting = Hunting {
            grounds: paced.iter().map(|p| (p.0, p.1)).collect(),
            paces: paced.into_iter().map(|p| p.2).collect(),
            sought: vec![wcid],
            droppers: Vec::new(),
            items: Vec::new(),
        };
        (step, Some(hunting))
    }
}
