//! How a pursued quest ends, in the Quests window's words: the picker's entries (any giver, each
//! way to complete it by giver and reward, keeping its item) and the pursued list's short tag.

use ac_client::autoplay::{quest_ends, QuestEnd};
use ac_world::quests::{Kind, Row, Want};

use super::short_xp;

/// Every end quest `id` offers, each with the picker's words, as [`quest_ends`] lists them.
pub(crate) fn end_choices(id: &str) -> Vec<(QuestEnd, String)> {
    quest_ends(id)
        .into_iter()
        .map(|end| {
            let words = end_words(&end, id);
            (end, words)
        })
        .collect()
}

/// "Hand in", "Hand in to Le-Ai Rea for 35% of a level's XP", "Keep Focusing Stone".
pub(crate) fn end_words(end: &QuestEnd, id: &str) -> String {
    match end {
        QuestEnd::HandIn => "Hand in".into(),
        QuestEnd::HandInTo { .. } => match chosen_row(end, id) {
            Some(row) => format!("Hand in to {} for {}", row.giver, reward_words(row)),
            None => "Hand in to a giver the data no longer has".into(),
        },
        QuestEnd::Keep => format!("Keep {}", kept_name(id)),
    }
}

/// The pursued list's tag for `end`: nothing for any giver, else "to Le-Ai Rea", "keep ...".
pub(crate) fn end_tag(end: &QuestEnd, id: &str) -> String {
    match end {
        QuestEnd::HandIn => String::new(),
        QuestEnd::HandInTo { .. } => chosen_row(end, id).map_or_else(
            || "to a giver gone".into(),
            |row| format!("to {}", row.giver),
        ),
        QuestEnd::Keep => format!("keep {}", kept_name(id)),
    }
}

/// The first row of `id` that `end` hands in by.
fn chosen_row(end: &QuestEnd, id: &str) -> Option<&'static Row> {
    ac_world::quests::rows_of(id).find(|r| r.kind != Kind::Start && end.can_hand_in(r))
}

/// What quest `id` keeps, by name: its first wanted item.
fn kept_name(id: &str) -> String {
    let rows = ac_world::quests::rows_of(id).filter(|r| matches!(r.want, Want::Item(_)));
    let mut names = rows.map(|r| match (&r.item, &r.want) {
        (Some(i), _) => i.name.trim().to_string(),
        (None, Want::Item(w)) => format!("item {w}"),
        _ => String::new(),
    });
    names.next().unwrap_or_else(|| "its item".into())
}

/// One row's reward: its items ("Trade Note (75,000) x1"), then its XP, fixed or a level's share.
pub(crate) fn reward_words(row: &Row) -> String {
    let mut parts: Vec<String> = row
        .gives
        .iter()
        .map(|&(wcid, n)| {
            let name = ac_world::quest_chain::reward_name(wcid);
            format!(
                "{} x{n}",
                name.map_or_else(|| format!("item {wcid}"), str::to_string)
            )
        })
        .collect();
    if row.random && parts.len() > 1 {
        parts = vec![format!("one of {}", parts.join(" or "))];
    }
    if row.reward.xp > 0 {
        parts.push(format!("{} XP", short_xp(row.reward.xp)));
    }
    if row.reward.xp_level > 0.0 {
        let share = (row.reward.xp_level * 100.0).round();
        parts.push(format!("{share}% of a level's XP"));
    }
    if parts.is_empty() {
        return "no reward listed".into();
    }
    parts.join(", ")
}
