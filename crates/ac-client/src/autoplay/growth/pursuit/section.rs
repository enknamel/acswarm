//! The loot profile's Quests section, kept by the pursuit: while a quest is pursued, a Keep rule
//! for each item its chain needs (a key, or an item used on an object, only while what it opens
//! to or makes is wanted) and a buy line for one a shop sells, which the town run buys as any
//! other; gone once the quest is done or dropped. Each entry carries its quest and asks for this
//! character's name; nothing else is touched.

use ac_world::quests::{Kind, Want, Yield};

use super::fetch::ware_name;
use super::{Chains, PursuitLine, PursuitStep};
use crate::items::{NumKey, Op, Term};
use crate::profile::{Ask, Buy, LootAction, Mine, Rule, QUESTS};
use crate::quest_log::{QuestLine, QuestState};
use crate::Client;

impl Client {
    /// Make the profile's Quests section hold what the pursuit asks now, written back through
    /// the profiles store when it changed; from `refresh_pursuit`.
    pub(crate) fn keep_quest_section(&mut self, chains: &dyn Chains) {
        let who = self.world.stats.name.trim().to_string();
        if who.is_empty() {
            return;
        }
        let Some(profile) = self.loot_profile() else {
            return;
        };
        let (rules, buy) = quest_entries(&self.pursuit, &self.quest_board, &who, chains);
        let mut profile = (*profile).clone();
        let (r, b) = (rules.len(), buy.len());
        if !profile.set_quest_entries(&who, rules, buy) {
            return;
        }
        tracing::info!(
            "pursuit: the Quests section of {} now {r} rules, {b} buys",
            profile.name
        );
        if let Err(e) = self.profiles.put(profile) {
            tracing::warn!("writing the Quests section: {e}");
        }
    }
}

/// The entries `who`'s pursuit `lines` ask, each quest's rows and state read off the `board`.
pub(crate) fn quest_entries(
    lines: &[PursuitLine],
    board: &[QuestLine],
    who: &str,
    chains: &dyn Chains,
) -> (Vec<Rule>, Vec<Buy>) {
    let (mut rules, mut buy) = (Vec::new(), Vec::new());
    for line in lines {
        if matches!(
            line.step,
            PursuitStep::Waiting { .. } | PursuitStep::Finished
        ) {
            continue;
        }
        let Some(quest) = board.iter().find(|q| q.id == line.id) else {
            continue;
        };
        if quest.kind != Kind::Item {
            continue;
        }
        let (r, b) = entries_for(quest, who, chains);
        rules.extend(r);
        buy.extend(b);
    }
    (rules, buy)
}

/// One quest's entries: its items, then the keys to the chests holding them and the items used to
/// make them while one is still wanted; a buy line for the first of those a shop sells, sized to what the hand-in takes.
fn entries_for(quest: &QuestLine, who: &str, chains: &dyn Chains) -> (Vec<Rule>, Option<Buy>) {
    let rows: Vec<_> = crate::quest_log::worked_rows(quest)
        .filter(|r| r.kind == Kind::Item)
        .collect();
    // (wcid, name, how many the hand-in takes)
    let mut items: Vec<(u32, String, u32)> = Vec::new();
    for r in &rows {
        let Want::Item(wcid) = r.want else { continue };
        if let Some(i) = items.iter_mut().find(|i| i.0 == wcid) {
            i.2 = i.2.max(r.count.max(1));
            continue;
        }
        let name = r
            .item
            .as_ref()
            .map_or_else(|| format!("item {wcid}"), |i| i.name.to_string());
        items.push((wcid, name, r.count.max(1)));
    }
    if matches!(quest.state, QuestState::Needs { .. }) {
        let wanted: Vec<u32> = items.iter().map(|i| i.0).collect();
        for key in wanted.into_iter().flat_map(crate::quest_log::keys_to) {
            if !items.iter().any(|i| i.0 == key) {
                items.push((key, key_name(key), 1));
            }
        }
    }
    let rules = items
        .iter()
        .map(|(wcid, name, count)| Rule {
            name: format!("{}: {name}", quest.title.trim()),
            group: QUESTS.into(),
            action: LootAction::Keep,
            all: vec![
                Ask::Me(Mine::Name(who.to_string())),
                Ask::Item(Term::Num(NumKey::Class, Op::Eq, *wcid as f64)),
            ],
            keep_up_to: Some(*count),
            quest: quest.id.clone(),
            ..Default::default()
        })
        .collect();
    let buy = items
        .iter()
        .find_map(|(wcid, name, count)| buy_line(*wcid, name, *count, chains))
        .map(|b| Buy {
            when: vec![Mine::Name(who.to_string())],
            group: QUESTS.into(),
            quest: quest.id.clone(),
            ..b
        });
    (rules, buy)
}

/// A key's name, as a chest it opens lists it.
fn key_name(key: u32) -> String {
    ac_world::quests::sources()
        .iter()
        .flat_map(|s| s.keys.iter())
        .find(|k| k.0 == key)
        .map_or_else(|| format!("item {key}"), |k| k.1.clone())
}

/// The line buying `count` of `wcid` when a shop sells it: urgent as soon as one is short, so a
/// run goes for it; the shop named when only one sells it.
fn buy_line(wcid: u32, name: &str, count: u32, chains: &dyn Chains) -> Option<Buy> {
    let shops: Vec<_> = chains
        .sources(&Want::Item(wcid))
        .into_iter()
        .filter(|s| s.kind == Yield::Vendor)
        .collect();
    let first = shops.first()?;
    let one = shops.iter().all(|s| s.name == first.name);
    Some(Buy {
        what: ware_name(first, wcid).unwrap_or_else(|| name.to_string()),
        keep: count,
        restock_at: Some(count - 1),
        from: one.then(|| first.name.clone()),
        on: true,
        ..Default::default()
    })
}
