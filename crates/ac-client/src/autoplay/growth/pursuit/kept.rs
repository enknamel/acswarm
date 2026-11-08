//! A pursued quest kept (`QuestEnd::Keep`): once what a hand-in takes is carried, the pursuit ends
//! with no hand-in, the quest is switched off so no town run hands it in, and the player's own
//! rules gain a Keep rule for the item, which outlives the Quests section.

use std::time::Instant;

use ac_world::quests::{Kind, Want};

use crate::autoplay::QuestEnd;
use crate::items::{NumKey, Op, Term};
use crate::profile::{Ask, LootAction, Rule, QUESTS};
use crate::Client;

/// What item quest `id` keeps once `carried` holds it: the first hand-in carried in full, by weenie
/// class, name and count.
pub(crate) fn kept_item(id: &str, carried: &dyn Fn(u32) -> u32) -> Option<(u32, String, u32)> {
    let rows = ac_world::quests::rows_of(id).filter(|r| r.kind == Kind::Item);
    rows.into_iter().find_map(|r| {
        let Want::Item(wcid) = r.want else {
            return None;
        };
        let name = r
            .item
            .as_ref()
            .map_or_else(|| format!("item {wcid}"), |i| i.name.trim().to_string());
        let count = r.count.max(1);
        (carried(wcid) >= count).then_some((wcid, name, count))
    })
}

/// The player's own rule keeping `count` of `wcid` (`name`), a quest's item kept.
pub(crate) fn kept_rule(wcid: u32, name: &str, count: u32) -> Rule {
    Rule {
        name: format!("Kept: {name}"),
        action: LootAction::Keep,
        all: vec![Ask::Item(Term::Num(NumKey::Class, Op::Eq, f64::from(wcid)))],
        keep_up_to: Some(count),
        ..Default::default()
    }
}

impl Client {
    /// End each kept pursuit whose item is carried: off the list and switched off, said, and its
    /// [`kept_rule`] put in the profile; from `finish_pursuits`, before the board is worked out.
    pub(crate) fn finish_kept(&mut self, now: Instant) {
        let carried = |w| self.carried_count(w);
        let kept: Vec<(String, (u32, String, u32))> = self
            .autoplay
            .config
            .quests
            .pursue
            .iter()
            .filter(|p| p.outcome == QuestEnd::Keep)
            .filter_map(|p| kept_item(&p.id, &carried).map(|k| (p.id.clone(), k)))
            .collect();
        for (id, (wcid, name, count)) in kept {
            let title = self.pursuit.iter().find(|l| l.id == id).map_or_else(
                || {
                    ac_world::quests::rows_of(&id)
                        .next()
                        .map_or(id.clone(), |r| r.title.trim().into())
                },
                |l| l.title.clone(),
            );
            tracing::info!("pursuit: {id} kept {name} ({wcid})");
            self.autoplay.note(format!("kept {name} from {title}"), now);
            let quests = &mut self.autoplay.config.quests;
            quests.stop_pursuing(&id);
            quests.switch_off(&id);
            self.add_kept_rule(kept_rule(wcid, &name, count));
        }
    }

    /// Put `rule` after the profile's Quests section (else first), written back through the
    /// profiles store; nothing when a rule already keeps the same.
    fn add_kept_rule(&mut self, rule: Rule) {
        let Some(profile) = self.loot_profile() else {
            return;
        };
        let same = |r: &Rule| r.action == rule.action && r.all == rule.all;
        if profile.rules.iter().any(same) {
            return;
        }
        let mut profile = (*profile).clone();
        let at = profile.rules.iter().rposition(|r| r.group == QUESTS);
        profile.rules.insert(at.map_or(0, |i| i + 1), rule);
        if let Err(e) = self.profiles.put(profile) {
            tracing::warn!("writing a kept quest item's rule: {e}");
        }
    }
}
