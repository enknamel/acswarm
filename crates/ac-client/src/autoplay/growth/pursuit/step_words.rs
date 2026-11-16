//! What a pursued quest's step is about: the landblock a fetch works in, whether autoplay can act
//! on it now, and the step in words.

use super::{events, PursuitStep};

impl PursuitStep {
    /// The landblock of the thing this step fetches from where it lies: an item, chest or NPC.
    pub(crate) fn fetch_landblock(&self) -> Option<u32> {
        match self {
            PursuitStep::PickUp { at, .. }
            | PursuitStep::OpenChest { at, .. }
            | PursuitStep::UseOn { at, .. }
            | PursuitStep::AskFor { at, .. }
            | PursuitStep::GiveTo { at, .. } => Some(at.cell >> 16),
            _ => None,
        }
    }

    /// Whether autoplay can act on it now.
    pub fn can_progress(&self) -> bool {
        matches!(
            self,
            PursuitStep::TakeUp { .. }
                | PursuitStep::EarnFlag { .. }
                | PursuitStep::Hunt { .. }
                | PursuitStep::Buy { .. }
                | PursuitStep::PickUp { .. }
                | PursuitStep::OpenChest { .. }
                | PursuitStep::UseOn { .. }
                | PursuitStep::AskFor { .. }
                | PursuitStep::GiveTo { .. }
                | PursuitStep::Combine { .. }
                | PursuitStep::HandIn { .. }
                | PursuitStep::Read { .. }
                | PursuitStep::GoThrough { .. }
                | PursuitStep::Arena { .. }
        )
    }

    /// The step in words, `now` in Unix seconds for a wait ("hunt Insatiable Eater in ...").
    pub fn words(&self, now: u64) -> String {
        match self {
            PursuitStep::TakeUp { row, .. } => format!("take it up from {}", row.giver),
            PursuitStep::EarnFlag { flag, from } => {
                format!("earn the {flag} flag from {}", from.npc)
            }
            PursuitStep::Hunt {
                creature,
                place,
                more,
                ..
            } => format!("hunt {creature} in {place} ({more} to go)"),
            PursuitStep::Buy {
                name, vendor, more, ..
            } => format!("buy {name} from {vendor} ({more} to go)"),
            PursuitStep::PickUp { name, at, .. } => format!(
                "pick up {name} in {}",
                ac_world::quest_chain::place_name(at.cell)
            ),
            PursuitStep::OpenChest { chest, at, key } => {
                let with = key
                    .and_then(|k| chest.keys.iter().find(|c| c.0 == k))
                    .map_or(String::new(), |k| format!(" with {}", k.1));
                let place = ac_world::quest_chain::place_name(at.cell);
                format!("open {}{with} in {place}", chest.name)
            }
            PursuitStep::UseOn { source, at, with } => {
                let used = source.keys.iter().find(|k| k.0 == *with);
                let used = used.map_or_else(|| format!("item {with}"), |k| k.1.clone());
                let place = ac_world::quest_chain::place_name(at.cell);
                format!("use {used} on {} in {place}", source.name)
            }
            PursuitStep::AskFor { source, at } => {
                let place = ac_world::quest_chain::place_name(at.cell);
                events::ask_words(source, &place)
            }
            PursuitStep::GiveTo { source, at, item } => {
                let given = source.keys.iter().find(|k| k.0 == *item);
                let given = given.map_or_else(|| format!("item {item}"), |k| k.1.clone());
                let place = ac_world::quest_chain::place_name(at.cell);
                format!("give {given} to {} in {place}", source.name)
            }
            PursuitStep::Combine { source, with, on } => {
                let name = |w: &u32| source.keys.iter().find(|k| k.0 == *w).map(|k| k.1.clone());
                let (used, onto) = (name(with), name(on));
                format!(
                    "use {} on {}",
                    used.unwrap_or_else(|| format!("item {with}")),
                    onto.unwrap_or_else(|| format!("item {on}"))
                )
            }
            PursuitStep::HandIn { row, .. } => format!("hand it in to {}", row.giver),
            PursuitStep::GoThrough { name, at, flag, .. } => {
                let place = ac_world::quest_chain::place_name(at.cell);
                format!("go through {name} in {place} for the {flag} flag")
            }
            PursuitStep::Arena { flag, step } => step.words(flag),
            PursuitStep::Read { name, .. } => format!("read {name}"),
            PursuitStep::Waiting { until } => {
                let left = until.saturating_sub(now);
                format!("waits {}h {}m", left / 3600, left % 3600 / 60)
            }
            PursuitStep::Finished => "finished".to_string(),
            PursuitStep::Stuck { why } => format!("passed over: {why}"),
        }
    }
}
