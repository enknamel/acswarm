//! Item goals (`Pursuit::count`): so many of an item, come by the fastest way the data has --
//! itself hunted, bought or fetched, or handed in for to a quest that gives it -- until carried.

use ac_world::quests::{Handing, Kind, Row, Want};

use super::{stuck, Ask, Hunting, PursuitLine, PursuitStep};
use crate::autoplay::Pursuit;
use crate::quest_log::QuestLine;

/// An item goal's way that fetches rather than hunts, ranked as this many seconds an item: a
/// walk and a word, beaten only by a hunt faster than a minute an item.
const GOAL_FETCH_SECS: f32 = 60.0;

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
            let way = self.goal_way(item, goal.count - have, &name);
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

    /// The fastest way to `more` of `item`: come by itself, or handed in for by a quest giving it,
    /// ranked by seconds an item at its ground (camped, the journey there left out). A way whose
    /// step is for a part further down its chain is not ranked: it is had only when no way is.
    fn goal_way(&self, item: u32, more: u32, name: &str) -> (PursuitStep, Option<Hunting>) {
        let secs = |got: &(PursuitStep, Option<Hunting>), target: u32, per: f32| -> Option<f32> {
            let pace = match &got.0 {
                PursuitStep::HandIn { .. } => return Some(0.0),
                PursuitStep::Hunt { .. } => {
                    let h = got.1.as_ref()?;
                    if !h.items.contains(&target) {
                        return None;
                    }
                    h.paces.first()?.pace.soonest()
                }
                step if step_item(step) == Some(target) => GOAL_FETCH_SECS / step_chance(step),
                _ => return None,
            };
            pace.is_finite().then_some(pace / per)
        };
        let usable = |got: &(PursuitStep, Option<Hunting>)| {
            !matches!(
                got.0,
                PursuitStep::Stuck { .. } | PursuitStep::Waiting { .. } | PursuitStep::Finished
            )
        };
        let direct = self.obtain(&[Want::Item(item)], more, name, 0);
        let mut best = secs(&direct, item, 1.0).map(|s| (s, direct.clone()));
        let mut fallback = usable(&direct).then(|| direct.clone());
        let gives = |r: &&Row| {
            r.kind == Kind::Item
                && r.hand == Handing::Give
                && r.want != Want::Item(item)
                && r.gives.iter().any(|(w, _)| *w == item)
        };
        for r in ac_world::quests::all().iter().filter(gives) {
            let Want::Item(input) = r.want else {
                continue;
            };
            let Some(at) = r.places.iter().find(|p| !(self.asking.refused)(r, p)) else {
                continue;
            };
            let count = r.count.max(1);
            let per = r.gives.iter().filter(|(w, _)| *w == item);
            let per = per.map(|(_, n)| *n).sum::<u32>().max(1) as f32 / count as f32;
            let got = if (self.asking.carried)(input) >= count {
                (PursuitStep::HandIn { row: r, at: *at }, None)
            } else {
                let of = r.item.as_ref().map_or("", |i| i.name);
                self.obtain(std::slice::from_ref(&r.want), count, of, 1)
            };
            match secs(&got, input, per) {
                Some(s) if best.as_ref().is_none_or(|b| s < b.0) => best = Some((s, got)),
                Some(_) => {}
                None if fallback.is_none() && usable(&got) => fallback = Some(got),
                None => {}
            }
        }
        best.map(|b| b.1).or(fallback).unwrap_or(direct)
    }
}

/// The item a fetch step comes by: picked up or bought, or what its source yields.
fn step_item(step: &PursuitStep) -> Option<u32> {
    let source = match step {
        PursuitStep::PickUp { wcid, .. } | PursuitStep::Buy { wcid, .. } => return Some(*wcid),
        PursuitStep::OpenChest { chest: s, .. }
        | PursuitStep::UseOn { source: s, .. }
        | PursuitStep::AskFor { source: s, .. }
        | PursuitStep::GiveTo { source: s, .. }
        | PursuitStep::Combine { source: s, .. } => s,
        _ => return None,
    };
    match source.want {
        Want::Item(w) => Some(w),
        _ => None,
    }
}

/// A fetch step's chance a try yields: its source's, clamped as [`super::fetch::try_cost`]
/// does; a pickup or a buy is sure.
fn step_chance(step: &PursuitStep) -> f32 {
    match step {
        PursuitStep::OpenChest { chest: s, .. }
        | PursuitStep::UseOn { source: s, .. }
        | PursuitStep::AskFor { source: s, .. }
        | PursuitStep::GiveTo { source: s, .. }
        | PursuitStep::Combine { source: s, .. } => s.chance.clamp(0.001, 1.0),
        _ => 1.0,
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
