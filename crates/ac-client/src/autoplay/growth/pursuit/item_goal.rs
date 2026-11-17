//! Item goals (`Pursuit::count`): so many of an item, come by the fastest way the data has --
//! itself hunted, bought or fetched, or handed in for to a quest that gives it (`fastest`) --
//! until carried.

use ac_world::quests::{Handing, Want};

use super::{stuck, Ask, Hunting, PursuitLine, PursuitStep};
use crate::autoplay::Pursuit;
use crate::quest_log::QuestLine;

impl Ask<'_> {
    /// An item goal's line: `item` come by however the data has it (`obtain`) until the goal's
    /// count is carried, then finished.
    pub(super) fn goal_line(&self, goal: &Pursuit, item: u32) -> (PursuitLine, Option<Hunting>) {
        let name = goal_name(item);
        let have = self.facts.carried.get(&item).copied().unwrap_or(0);
        let (step, hunt) = if let Some(leader) = self.led {
            (stuck(format!("following {leader}; waiting")), None)
        } else if let Some(why) = self.held.get(&goal.id) {
            (stuck(why.clone()), None)
        } else if have >= goal.count {
            (PursuitStep::Finished, None)
        } else {
            let way = self.obtain(&[Want::Item(item)], goal.count - have, &name, 0);
            self.way_first(&goal.id, way)
        };
        let line = PursuitLine {
            id: goal.id.clone(),
            title: format!("{name} {have}/{}", goal.count),
            step,
            working: false,
            via: None,
        };
        (line, hunt)
    }
}

/// Item `wcid`'s name from the quest and shop data, else its class in words.
pub(crate) fn goal_name(wcid: u32) -> String {
    ac_world::quest_chain::reward_name(wcid).map_or_else(|| format!("item {wcid}"), str::to_string)
}

/// The scroll `line`'s quest is done by reading, and the spell it teaches: a `learn:` row's.
pub(super) fn line_reads(line: &QuestLine) -> Option<(u32, String, u32)> {
    let row = crate::quest_log::worked_rows(line).find(|r| r.hand == Handing::Read)?;
    let Want::Item(wcid) = row.want else {
        return None;
    };
    let name = row
        .item
        .as_ref()
        .map_or_else(|| format!("item {wcid}"), |i| i.name.to_string());
    Some((wcid, name, row.teaches()?))
}
