//! A quest's state in words and colour, read off the character's board (`ac_client::quest_log`),
//! shared by the autoplay window's Quests section and the Quests window (`super::quest_book`).

use ac_client::autoplay::{LootAction, NO_RULE_KEEPS};
use ac_client::quest_log::{Blocked, QuestLine, QuestState};

use crate::egui;

/// "3h 20m" for a wait of `secs`.
pub(crate) fn time_left(secs: u64) -> String {
    let (d, h, m) = (secs / 86_400, secs / 3_600 % 24, secs / 60 % 60);
    match (d, h) {
        (0, 0) => format!("{}m", m.max(1)),
        (0, _) => format!("{h}h {m}m"),
        _ => format!("{d}d {h}h"),
    }
}

fn does(action: LootAction) -> &'static str {
    match action {
        LootAction::Keep => "keeps",
        LootAction::Salvage => "salvages",
        LootAction::Sell => "sells",
        LootAction::Skip => "skips",
        LootAction::Drain => "drains",
    }
}

/// A quest's state in words; `quests_on` tells "do quests" off from the quest switched off.
pub(crate) fn words(state: &QuestState, quests_on: bool, now: u64) -> String {
    match state {
        QuestState::Off if quests_on => "switched off".into(),
        QuestState::Off => "do quests is off".into(),
        QuestState::Ready => "ready to hand in".into(),
        QuestState::Needs { more, of } => format!("needs {more} more {of}"),
        QuestState::NotTaken { from, .. } => format!("not taken up: talk to {from} first"),
        QuestState::WontComplete { rule, .. } if rule == NO_RULE_KEEPS => {
            format!("won't complete: {NO_RULE_KEEPS}; add the quest rule")
        }
        QuestState::WontComplete { rule, does: d } => format!(
            "won't complete: '{rule}' {} it; move the quest rule above it",
            does(*d)
        ),
        QuestState::CantYet(why) => format!(
            "can't do yet: {}",
            match why {
                Blocked::Level { min, max: 0 } => format!("for level {min} and up"),
                Blocked::Level { min, max } => format!("for levels {min} to {max}"),
                Blocked::Locked { flag } => format!("its giver is behind a portal asking {flag}"),
                Blocked::NeedsFirst { title } => format!("do '{title}' first"),
                Blocked::CantLearn => "its spell is beyond your magic skills".into(),
            }
        ),
        QuestState::Waiting { until } => {
            format!("done; again in {}", time_left(until.saturating_sub(now)))
        }
        QuestState::Done => "done".into(),
    }
}

/// Ready green, won't complete orange, can't do yet and waiting grey, the rest plain.
pub(crate) fn colour(state: &QuestState) -> egui::Color32 {
    match state {
        QuestState::Ready => egui::Color32::from_rgb(150, 210, 150),
        QuestState::WontComplete { .. } => egui::Color32::from_rgb(230, 160, 80),
        QuestState::Needs { .. } | QuestState::NotTaken { .. } => egui::Color32::from_gray(210),
        _ => egui::Color32::from_gray(140),
    }
}

/// "40 on, 3 ready, 1 won't complete".
pub(crate) fn count_line(lines: &[QuestLine]) -> String {
    let count = |f: fn(&QuestLine) -> bool| lines.iter().filter(|l| f(l)).count();
    format!(
        "{} on, {} ready, {} won't complete",
        count(|l| l.on),
        count(|l| l.state == QuestState::Ready),
        count(|l| matches!(l.state, QuestState::WontComplete { .. })),
    )
}

/// "to San Ming", or "to Yukara Miko the Collector or 11 more".
pub(crate) fn to_whom(line: &QuestLine) -> String {
    match line.givers {
        0 | 1 => format!("to {}", line.giver),
        n => format!("to {} or {} more", line.giver, n - 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_world::quests::Kind;

    fn line(id: &str, title: &str, state: QuestState) -> QuestLine {
        QuestLine {
            id: id.into(),
            title: title.into(),
            kind: Kind::Item,
            state,
            xp: 1_000,
            givers: 1,
            giver: "San Ming".into(),
            on: true,
            every_way: false,
        }
    }

    #[test]
    fn each_state_reads_as_the_list_says_it() {
        let now = 1_000;
        let needs = QuestState::Needs {
            more: 3,
            of: "Red Rat Tail".into(),
        };
        assert_eq!(words(&needs, true, now), "needs 3 more Red Rat Tail");
        let skip = QuestState::WontComplete {
            rule: "nothing cheap".into(),
            does: LootAction::Skip,
        };
        assert_eq!(
            words(&skip, true, now),
            "won't complete: 'nothing cheap' skips it; move the quest rule above it"
        );
        let level = QuestState::CantYet(Blocked::Level { min: 20, max: 0 });
        assert_eq!(
            words(&level, true, now),
            "can't do yet: for level 20 and up"
        );
        let wait = QuestState::Waiting {
            until: now + 3 * 3_600 + 20 * 60,
        };
        assert_eq!(words(&wait, true, now), "done; again in 3h 20m");
        assert_eq!(words(&QuestState::Off, false, now), "do quests is off");
        let unkept = QuestState::WontComplete {
            rule: NO_RULE_KEEPS.into(),
            does: LootAction::Skip,
        };
        assert_eq!(
            words(&unkept, true, now),
            "won't complete: no rule keeps it; add the quest rule"
        );
    }

    #[test]
    fn the_count_reads_on_ready_and_unkept_and_a_line_names_its_givers() {
        let mut lines = [
            line("give:3684:2470", "Red Rat Tail", QuestState::Ready),
            line(
                "killtaskbloodshreth",
                "Blood Shreth Kill Task",
                QuestState::Off,
            ),
        ];
        assert_eq!(count_line(&lines), "2 on, 1 ready, 0 won't complete");
        assert_eq!(to_whom(&lines[0]), "to San Ming");
        lines[0].givers = 12;
        assert_eq!(to_whom(&lines[0]), "to San Ming or 11 more");
    }
}
