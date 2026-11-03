//! The autoplay window's Quests section: "do quests", and each quest with its switch and its
//! state in words, read off the character's board (`ac_client::quest_log`). It edits only
//! `Config.quests`; what autoplay does with them is the rules' business.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use ac_client::autoplay::{LootAction, Quests, NO_RULE_KEEPS};
use ac_client::quest_log::{Blocked, QuestLine, QuestState};

use super::{caption, title};
use crate::egui;

/// How tall the list may grow before it scrolls (points).
const LIST_HEIGHT: f32 = 220.0;

/// Which quests the list shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Show {
    /// Those whose band and creatures suit the character's level.
    #[default]
    ForLevel,
    On,
    All,
}

impl Show {
    fn label(self) -> &'static str {
        match self {
            Show::ForLevel => "for my level",
            Show::On => "switched on",
            Show::All => "all",
        }
    }
}

/// The list's search and picker, kept by the window between frames.
#[derive(Clone, Debug, Default)]
pub(crate) struct QuestFilter {
    pub search: String,
    pub show: Show,
    /// "add the quest rule" was pressed: the window puts it in the character's profile.
    pub add_rule: bool,
}

/// The wanted item's name by quest id, for the search: a line names only its title and giver.
fn item_names() -> &'static BTreeMap<&'static str, &'static str> {
    static NAMES: OnceLock<BTreeMap<&'static str, &'static str>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let rows = ac_world::quests::all().iter();
        rows.filter_map(|r| Some((r.id, r.item.as_ref()?.name)))
            .collect()
    })
}

/// Whether `line` is one the filter lets through.
pub(crate) fn is_shown(
    line: &QuestLine,
    cfg: &Quests,
    unsuited: &BTreeSet<String>,
    f: &QuestFilter,
) -> bool {
    let shown = match f.show {
        Show::ForLevel => !unsuited.contains(&line.id),
        Show::On => !cfg.off.contains(&line.id),
        Show::All => true,
    };
    let needle = f.search.trim().to_lowercase();
    let item = item_names().get(line.id.as_str()).copied().unwrap_or("");
    shown
        && (needle.is_empty()
            || [line.title.as_str(), line.giver.as_str(), item]
                .iter()
                .any(|s| s.to_lowercase().contains(&needle)))
}

/// "3h 20m" for a wait of `secs`.
fn time_left(secs: u64) -> String {
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
    }
}

/// A quest's state in words; `quests_on` tells "do quests" off from the quest switched off.
pub(crate) fn words(state: &QuestState, quests_on: bool, now: u64) -> String {
    match state {
        QuestState::Off if quests_on => "switched off".into(),
        QuestState::Off => "do quests is off".into(),
        QuestState::Ready => "ready to hand in".into(),
        QuestState::Needs { more, of } => format!("needs {more} more {of}"),
        QuestState::NotTaken { from } => format!("not taken up: talk to {from} first"),
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
                Blocked::TooHard { level } => format!("its easiest creature is level {level}"),
                Blocked::Locked { flag } => format!("its giver is behind a portal asking {flag}"),
                Blocked::NeedsFirst { title } => format!("do '{title}' first"),
            }
        ),
        QuestState::Waiting { until } => {
            format!("done; again in {}", time_left(until.saturating_sub(now)))
        }
        QuestState::Done => "done".into(),
    }
}

/// Ready green, won't complete orange, can't do yet and waiting grey, the rest plain.
fn colour(state: &QuestState) -> egui::Color32 {
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
fn to_whom(line: &QuestLine) -> String {
    match line.givers {
        0 | 1 => format!("to {}", line.giver),
        n => format!("to {} or {} more", line.giver, n - 1),
    }
}

/// The Quests section, editing `cfg`.
pub(crate) fn section(
    ui: &mut egui::Ui,
    cfg: &mut Quests,
    lines: &[QuestLine],
    unsuited: &BTreeSet<String>,
    filter: &mut QuestFilter,
    now: u64,
) {
    title(ui, "Quests");
    ui.checkbox(&mut cfg.on, "do quests").on_hover_text(
        "Keep what the quests switched on want, and hand it in. Off, the quest rule \
         wants nothing and its items go to the rules below it",
    );
    caption(ui, count_line(lines));
    let unkept = |l: &&QuestLine| matches!(&l.state, QuestState::WontComplete { rule, .. } if rule == NO_RULE_KEEPS);
    if lines.iter().any(|l| unkept(&l))
        && ui
            .button("add the quest rule at the top")
            .on_hover_text(
                "The profile has no rule keeping quest items: put \"quest items\" (Keep, wanted \
                 by an active quest) first in it",
            )
            .clicked()
    {
        filter.add_rule = true;
    }
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut filter.search)
                .hint_text("title, giver or item")
                .desired_width(150.0),
        );
        egui::ComboBox::from_id_salt("autoplay.quests.show")
            .selected_text(filter.show.label())
            .show_ui(ui, |ui| {
                for show in [Show::ForLevel, Show::On, Show::All] {
                    ui.selectable_value(&mut filter.show, show, show.label());
                }
            });
    });
    let shown: Vec<&QuestLine> = lines
        .iter()
        .filter(|l| is_shown(l, cfg, unsuited, filter))
        .collect();
    if shown.is_empty() {
        caption(ui, "no quest here");
        return;
    }
    let small = ui.text_style_height(&egui::TextStyle::Small);
    let row = ui.spacing().interact_size.y + small + ui.spacing().item_spacing.y;
    egui::ScrollArea::vertical()
        .id_salt("autoplay.quests")
        .max_height(LIST_HEIGHT)
        .show_rows(ui, row, shown.len(), |ui, range| {
            for line in &shown[range] {
                let mut on = cfg.is_chosen(&line.id) || !cfg.off.contains(&line.id);
                let said = words(&line.state, cfg.on, now);
                let hover = format!("{}\n{} XP now\n{said}", to_whom(line), line.xp);
                if ui
                    .checkbox(&mut on, &line.title)
                    .on_hover_text(hover)
                    .changed()
                {
                    // Ticked back, a quest paying XP works as it starts; any other is chosen.
                    if !on {
                        cfg.switch_off(&line.id);
                    } else if ac_client::quest_log::is_automatic_quest(&line.id) {
                        cfg.off.retain(|o| o != &line.id);
                    } else {
                        cfg.switch_on(&line.id);
                    }
                }
                let detail = format!(
                    "{} · {} XP · {said}",
                    to_whom(line),
                    super::skills::thousands(line.xp as i64)
                );
                ui.label(
                    egui::RichText::new(detail)
                        .small()
                        .color(colour(&line.state)),
                );
            }
        });
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
    fn the_filter_keeps_the_level_the_switch_and_the_search() {
        let lines = [
            line("give:3684", "Red Rat Tail", QuestState::Ready),
            line(
                "killtaskbloodshreth",
                "Blood Shreth Kill Task",
                QuestState::Off,
            ),
        ];
        let unsuited: BTreeSet<String> = ["killtaskbloodshreth".to_string()].into();
        let mut cfg = Quests {
            on: true,
            off: vec!["give:3684".into()],
            ..Default::default()
        };
        let mut f = QuestFilter::default();
        let shown = |cfg: &Quests, f: &QuestFilter| -> Vec<&str> {
            let lines = lines.iter().filter(|l| is_shown(l, cfg, &unsuited, f));
            lines.map(|l| l.id.as_str()).collect()
        };
        assert_eq!(shown(&cfg, &f), ["give:3684"], "for my level");
        f.show = Show::On;
        assert_eq!(shown(&cfg, &f), ["killtaskbloodshreth"]);
        cfg.off.clear();
        f.show = Show::All;
        f.search = "ming".into();
        assert_eq!(shown(&cfg, &f).len(), 2, "by giver");
        f.search = "rat tail".into();
        assert_eq!(shown(&cfg, &f), ["give:3684"], "by title or item");
        assert_eq!(count_line(&lines), "2 on, 1 ready, 0 won't complete");
    }

    #[test]
    fn a_frame_of_the_section_draws_and_changes_nothing_unclicked() {
        let lines = [
            line("give:3684", "Red Rat Tail", QuestState::Ready),
            line("give:3681", "Black Rat Tail", QuestState::Done),
        ];
        let mut cfg = Quests {
            on: true,
            off: Vec::new(),
            ..Default::default()
        };
        let mut f = QuestFilter::default();
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            section(ui, &mut cfg, &lines, &BTreeSet::new(), &mut f, 0);
        });
        out.textures_delta.clear();
        assert_eq!(
            cfg,
            Quests {
                on: true,
                off: Vec::new(),
                ..Default::default()
            }
        );
    }
}
