//! A portal gem had first when only its exit leads on to a pursued ground: Ithaenc's island has
//! no portal from the mainland (the Wedding Planner sells the invitation), and Corcima Castle's
//! wards are reached only by the gems their captains drop. The gem is a wanted item like any
//! other: bought, hunted or fetched; carried, the trip planner takes it.

use ac_world::quests::{Want, Yield};
use ac_world::trip::Prefs;
use ac_world::Place;

use super::super::State;
use super::section::buy_line;
use super::ChainData;
use crate::items::{NumKey, Op, Term};
use crate::profile::{Ask, Buy, LootAction, Mine, Rule, QUESTS};
use crate::Client;

/// The gem `quest`'s pursuit has first, since only its exit leads on to the ground.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GemWanted {
    pub quest: String,
    pub wcid: u32,
    pub name: String,
    /// Seen carried: once gone from the pack it was used, and is wanted no more.
    pub had: bool,
}

impl Client {
    /// The gem, not carried, whose exit a journey goes on from to one of `grounds` and that
    /// something gives: one a shop this character reaches sells first, then one hunted or
    /// fetched, each the one landing nearest in time; `None` when none does.
    pub(super) fn gem_way(
        &mut self,
        grounds: &[(u32, Place)],
        quests: &[String],
    ) -> Option<GemWanted> {
        let here = self.my_position()?.truncate();
        let level = self.world.stats.level.max(1) as u32;
        let goals: Vec<Place> = grounds.iter().map(|g| self.planning_place(g.1)).collect();
        // (hunted or fetched, seconds on from the exit), the gem
        let mut best: Option<((bool, f32), GemWanted)> = None;
        for gem in ac_world::gems::all() {
            if self.carried_count(gem.wcid) > 0 {
                continue;
            }
            let want = Want::Item(gem.wcid);
            let sold = ac_world::shops::selling_wcid(gem.wcid, here)
                .into_iter()
                .next();
            let given = ac_world::quests::sources_of(&want).any(|s| s.kind != Yield::Vendor);
            if sold.is_none() && !given {
                continue;
            }
            let exit = Place {
                at: gem.at,
                cell: gem.cell,
            };
            let prefs: Prefs = self.travel_prefs().in_dungeon(exit.is_indoors());
            let on = goals
                .iter()
                .filter_map(|goal| {
                    ac_world::trip::plan_with_recalls_and_gems(
                        exit,
                        *goal,
                        level,
                        quests,
                        &[],
                        &[],
                        &[],
                        &[],
                        prefs,
                    )
                })
                .map(|t| t.seconds)
                .min_by(f32::total_cmp);
            let Some(secs) = on else { continue };
            // A shop reached buys it on a run; else it is hunted or fetched.
            let bought = sold.is_some_and(|shop| {
                let counter = Place {
                    at: shop.at,
                    cell: shop.cell,
                };
                self.journey_plan(counter, quests).is_some()
            });
            if !bought && !given {
                continue;
            }
            let key = (!bought, secs);
            if best
                .as_ref()
                .is_some_and(|(b, _)| (b.0, b.1) <= (key.0, key.1))
            {
                continue;
            }
            let wanted = GemWanted {
                quest: String::new(),
                wcid: gem.wcid,
                name: gem.name.clone(),
                had: false,
            };
            best = Some((key, wanted));
        }
        best.map(|(_, g)| g)
    }

    /// Mark each wanted gem carried as had, and let one had and gone from the pack go: used. A
    /// gem come to hand opens ways: every ground passed over for want of one is tried again.
    pub(super) fn keep_gem_ways(&mut self) {
        let carried: Vec<bool> = self
            .autoplay
            .growth
            .pursuit_gems
            .iter()
            .map(|g| self.carried_count(g.wcid) > 0)
            .collect();
        let st = &mut self.autoplay.growth;
        let mut came = false;
        let mut i = 0;
        st.pursuit_gems.retain_mut(|g| {
            let now_carried = carried[i];
            i += 1;
            if g.had && !now_carried {
                tracing::info!("pursuit: {}: {} used", g.quest, g.name);
                return false;
            }
            came |= now_carried && !g.had;
            g.had |= now_carried;
            true
        });
        if came {
            st.skip.retain(|_| false);
        }
    }
}

impl State {
    /// Have `quest`'s pursuit get `gem` first, in place of any gem it wanted before.
    pub(super) fn want_gem(&mut self, quest: &str, gem: GemWanted) {
        self.pursuit_gems.retain(|g| g.quest != quest);
        self.pursuit_gems.push(GemWanted {
            quest: quest.to_string(),
            ..gem
        });
    }
}

/// `who`'s Quests section entries for `gem`: kept, one at a time, and bought when a shop sells it.
pub(super) fn gem_entries(gem: &GemWanted, who: &str) -> (Rule, Option<Buy>) {
    let rule = Rule {
        name: format!("{}: {}", gem.quest, gem.name),
        group: QUESTS.into(),
        action: LootAction::Keep,
        all: vec![
            Ask::Me(Mine::Name(who.to_string())),
            Ask::Item(Term::Num(NumKey::Class, Op::Eq, gem.wcid as f64)),
        ],
        keep_up_to: Some(1),
        quest: gem.quest.clone(),
        ..Default::default()
    };
    let buy = buy_line(gem.wcid, &gem.name, 1, &ChainData).map(|b| Buy {
        when: vec![Mine::Name(who.to_string())],
        group: QUESTS.into(),
        quest: gem.quest.clone(),
        ..b
    });
    (rule, buy)
}
