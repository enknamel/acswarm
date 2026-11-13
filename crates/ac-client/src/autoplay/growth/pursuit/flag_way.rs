//! A quest flag had first when only a portal asking it leads on to a pursued place: the island
//! over the Singularity Caul's dungeons is reached past VirindiIsland alone, which the Guardian of
//! the Singularity's death stamps on its killer (a `flag:` source). Held from the killing blow:
//! the Death set runs for the last damager (EmoteManager.cs:1998-2008).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use ac_world::quest_chain::FlagBy;
use ac_world::quests::{Row, Want, Yield};
use ac_world::trip::Step;
use ac_world::Place;

use super::super::State;
use super::{stuck, Ask, Hunting, PursuitStep, OBTAIN_DEPTH};
use crate::Client;

/// A portal past a flag not held, reckoned in seconds: a day, more than any journey's walking,
/// so the way past the fewest flags is found (VirindiIsland's, not a shorter one past two).
const FLAG_SECONDS: f32 = 86_400.0;

/// Each creature whose death stamps flags on its killer, and those flags.
fn stamped_by_kill() -> &'static BTreeMap<u32, Vec<String>> {
    static KILLS: OnceLock<BTreeMap<u32, Vec<String>>> = OnceLock::new();
    KILLS.get_or_init(|| {
        let mut m: BTreeMap<u32, Vec<String>> = BTreeMap::new();
        for s in ac_world::quests::sources() {
            if let (Want::Flag(f), Yield::Kill) = (&s.want, s.kind) {
                m.entry(s.wcid).or_default().push(f.clone());
            }
        }
        m
    })
}

impl Client {
    /// The flags our killing blow at `at` earned: a creature lying there whose death stamps one.
    /// A flag come to hand opens ways: every ground passed over for want of one is tried again.
    pub(crate) fn note_flag_kill(&mut self, at: glam::Vec3) {
        let kills = stamped_by_kill();
        let earned: Vec<String> = self
            .world
            .objects
            .values()
            .filter(|o| o.world_pos().is_some_and(|p| p.distance(at) < 1.0))
            .filter_map(|o| kills.get(&o.weenie_class_id))
            .flatten()
            .cloned()
            .collect();
        if earned.iter().all(|f| self.holds_quest_flag(f)) {
            return;
        }
        self.hold_quest_flags(&earned);
        self.autoplay.growth.skip.retain(|_| false);
    }

    /// The flag, not held, of a portal on the journey to one of `grounds` past the fewest such
    /// flags, soonest of those: the one to have first; `None` when no flag would open a way.
    pub(super) fn flag_way(
        &mut self,
        grounds: &[(u32, Place)],
        quests: &[String],
    ) -> Option<String> {
        let held = |f: &str| quests.iter().any(|q| q.eq_ignore_ascii_case(f));
        let trip = grounds
            .iter()
            .filter_map(|g| self.journey_plan_with(g.1, quests, Some(FLAG_SECONDS)))
            .min_by(|a, b| a.seconds.total_cmp(&b.seconds))?;
        trip.steps.iter().find_map(|s| {
            let Step::Portal { mouth, exit, .. } = s else {
                return None;
            };
            ac_world::portals::all()
                .iter()
                .filter(|p| p.from_cell == mouth.cell && p.to_cell == exit.cell)
                .find(|p| !p.quest.is_empty() && !held(&p.quest))
                .map(|p| p.quest.clone())
        })
    }
}

impl Ask<'_> {
    /// The step for a flag a way on asks: earned from an unlock NPC the level earns, else by a
    /// kill that stamps it ([`OBTAIN_DEPTH`] deep, as a key), else why not (a portal's, named).
    pub(super) fn flag_step(&self, flag: &str, depth: usize) -> (PursuitStep, Option<Hunting>) {
        self.flag_step_by(flag, depth, false)
    }

    /// [`Self::flag_step`], a quest row stamping it worked with `work_rows`: for a flag wanted as a
    /// way in alone; one a board line asks has that quest chained as a line of its own
    /// (a_quest_needing_another_works_that_one_first).
    pub(super) fn flag_step_by(
        &self,
        flag: &str,
        depth: usize,
        work_rows: bool,
    ) -> (PursuitStep, Option<Hunting>) {
        let by = self.chains.givers(flag);
        let unlocks = || {
            by.iter().filter_map(|b| match b {
                FlagBy::Unlock(u) => Some(*u),
                _ => None,
            })
        };
        if let Some(u) = unlocks().find(|u| u.min_level <= self.facts.level) {
            if (self.tried)(u) {
                let why = format!("{} was set off to lately for the {flag} flag", u.npc);
                return (stuck(why), None);
            }
            let step = PursuitStep::EarnFlag {
                flag: u.flag.clone(),
                from: u,
            };
            return (step, None);
        }
        if let Some(u) = unlocks().next() {
            let why = format!("{} gives the {flag} flag from level {}", u.npc, u.min_level);
            return (stuck(why), None);
        }
        let mut killed = None;
        if by.iter().any(|b| matches!(b, FlagBy::Kill(_))) && depth < OBTAIN_DEPTH {
            let want = Want::Flag(flag.to_ascii_lowercase());
            let got = self.obtain(&[want], 1, &format!("the {flag} flag"), depth + 1);
            if !matches!(got.0, PursuitStep::Stuck { .. }) {
                return got;
            }
            killed = Some(got);
        }
        let quest = by.iter().find_map(|b| match b {
            FlagBy::Quest(row) => Some(*row),
            _ => None,
        });
        let portal = by.iter().find_map(|b| match b {
            FlagBy::Portal(s) => Some(*s),
            _ => None,
        });
        // A portal stamping it is gone through, or the arena revealing it run.
        if let Some(s) = portal.filter(|_| killed.is_none()) {
            let got = self.portal_step(flag, s, depth);
            if !matches!(got.0, PursuitStep::Stuck { .. }) || quest.is_none() {
                return got;
            }
        }
        // A quest row stamping it (the Fir Tree's HeartofInnocence): worked as a step, the first
        // row that leads anywhere.
        let rows = by.iter().filter_map(|b| match b {
            FlagBy::Quest(row) => Some(*row),
            _ => None,
        });
        let mut first_stuck = None;
        for row in rows.filter(|_| work_rows && depth < OBTAIN_DEPTH) {
            let got = self.row_step(row, depth + 1);
            if !matches!(got.0, PursuitStep::Stuck { .. }) {
                return got;
            }
            first_stuck.get_or_insert(got);
        }
        match (quest, killed, portal) {
            (Some(row), _, _) => first_stuck.unwrap_or_else(|| {
                let why = format!(
                    "the {flag} flag comes first, from {} ({})",
                    row.giver,
                    row.title.trim()
                );
                (stuck(why), None)
            }),
            (None, Some(got), _) => got,
            (None, None, _) => (stuck(format!("nobody known gives the {flag} flag")), None),
        }
    }
}

impl Ask<'_> {
    /// Quest row `row` worked for the flag it stamps: each flag it needs first (a flag step), then
    /// what it takes come by, then its giver used or handed it (Oswald's Lair, Ulgrim to the tree).
    fn row_step(&self, row: &'static Row, depth: usize) -> (PursuitStep, Option<Hunting>) {
        let held = |f: &String| self.facts.held.iter().any(|h| h.eq_ignore_ascii_case(f));
        if let Some(need) = row.needs.iter().find(|n| !held(n)) {
            return self.flag_step_by(need, depth, true);
        }
        let Some((_, row, at)) = self.asking.open_at(row) else {
            return (stuck(format!("{} cannot be asked now", row.giver)), None);
        };
        if let Want::Item(item) = row.want {
            let count = row.count.max(1);
            if (self.asking.carried)(item) < count {
                let name = row.item.as_ref().map_or("", |i| i.name);
                return self.obtain(std::slice::from_ref(&row.want), count, name, depth);
            }
        }
        (PursuitStep::HandIn { row, at }, None)
    }
}

impl State {
    /// Have `quest`'s pursuit get `flag` first, in place of any flag it wanted before.
    pub(super) fn want_flag(&mut self, quest: &str, flag: String) {
        self.pursuit_flags.insert(quest.to_string(), flag);
    }
}
