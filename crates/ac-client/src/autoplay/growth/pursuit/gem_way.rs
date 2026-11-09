//! A portal gem bought when it is the only way to a pursued ground: Ithaenc's island has no
//! portal from the mainland, and the Wedding Planner's invitation puts a character on it. The
//! Quests section keeps and buys it; carried, the trip planner takes it like any other gem.

use ac_world::trip::Prefs;
use ac_world::Place;

use super::super::State;
use crate::items::{NumKey, Op, Term};
use crate::profile::{Ask, Buy, LootAction, Mine, Rule, QUESTS};
use crate::Client;

/// A gem `quest`'s pursuit buys at `vendor`, since only its exit leads on to the ground.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GemWanted {
    pub quest: String,
    pub wcid: u32,
    pub name: String,
    pub vendor: String,
}

impl Client {
    /// The gem a shop this character reaches sells whose exit a journey goes on from to one of
    /// `grounds`, the one landing nearest in time; `None` when none does.
    pub(super) fn gem_way(
        &mut self,
        grounds: &[(u32, Place)],
        quests: &[String],
    ) -> Option<GemWanted> {
        let here = self.my_position()?.truncate();
        let level = self.world.stats.level.max(1) as u32;
        let goals: Vec<Place> = grounds.iter().map(|g| self.planning_place(g.1)).collect();
        let mut best: Option<(f32, GemWanted)> = None;
        for gem in ac_world::gems::all() {
            let Some(shop) = ac_world::shops::selling_wcid(gem.wcid, here)
                .into_iter()
                .next()
            else {
                continue;
            };
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
            if best.as_ref().is_some_and(|(b, _)| *b <= secs) {
                continue;
            }
            let counter = Place {
                at: shop.at,
                cell: shop.cell,
            };
            if self.journey_plan(counter, quests).is_none() {
                continue;
            }
            let wanted = GemWanted {
                quest: String::new(),
                wcid: gem.wcid,
                name: gem.name.clone(),
                vendor: shop.name.clone(),
            };
            best = Some((secs, wanted));
        }
        best.map(|(_, g)| g)
    }
}

impl State {
    /// Have `quest`'s pursuit buy `gem`, in place of any gem it wanted before.
    pub(super) fn want_gem(&mut self, quest: &str, gem: GemWanted) {
        self.pursuit_gems.retain(|g| g.quest != quest);
        self.pursuit_gems.push(GemWanted {
            quest: quest.to_string(),
            ..gem
        });
    }
}

/// `who`'s Quests section entries for `gem`: kept, one at a time, and bought when none is.
pub(super) fn gem_entries(gem: &GemWanted, who: &str) -> (Rule, Buy) {
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
    let buy = Buy {
        what: gem.name.clone(),
        keep: 1,
        restock_at: Some(0),
        from: Some(gem.vendor.clone()),
        on: true,
        when: vec![Mine::Name(who.to_string())],
        group: QUESTS.into(),
        quest: gem.quest.clone(),
    };
    (rule, buy)
}
