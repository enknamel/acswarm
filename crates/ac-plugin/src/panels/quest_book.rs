//! The Quests window (the menu, or the autoplay window's button): every quest in the data, searched
//! and sorted, one quest's chain in words, and the switches that have autoplay do it. Drawn by the
//! Autoplay plugin, the saved config's one owner; it edits only `Config.quests`. States come from a
//! browse board worked out every 2 s while the window is open (`quest_log::browse_board`).

use std::collections::HashMap;

use ac_client::autoplay::{PursuitLine, Quests, NO_RULE_KEEPS};
use ac_client::quest_log::{Blocked, QuestLine, QuestLog, QuestState};
use ac_world::quest_chain::{ChainLink, QuestCard};
use ac_world::quests::{Kind, Repeat};

use super::quest_list::{colour, count_line, time_left, to_whom, words};
use super::{caption, title, title_bar, window};
use crate::egui;

/// The window's id and its menu action.
pub(crate) const ID: &str = "quests";

mod quest_filter;
mod quest_outcome;
mod quest_samples;
mod quest_steps;

pub(crate) use quest_filter::*;
pub(crate) use quest_outcome::{end_choices, end_tag, end_words};
pub(crate) use quest_samples::{demo, Demo};
pub(crate) use quest_steps::*;

/// What the list was last filtered and sorted for.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ListKey {
    filter: Filter,
    sort: Sort,
    browse: u64,
    cfg: Quests,
    me: Me,
    cards: (usize, usize),
}

/// The window's own state between frames.
#[derive(Debug, Default)]
pub(crate) struct BookState {
    pub filter: Filter,
    pub sort: Sort,
    /// The quest the detail pane shows, by id.
    pub selected: Option<String>,
    pub browse: Browse,
    /// "add the quest rule" was pressed: the plugin puts it in the character's profile.
    pub add_rule: bool,
    listed_for: Option<ListKey>,
    listed: Vec<usize>,
    /// Card index by id, for the cards at `(pointer, length)`.
    index: HashMap<String, usize>,
    indexed: (usize, usize),
    /// The selected quest's steps in words, for that id and browse board.
    steps: Vec<Step>,
    steps_for: Option<(String, u64)>,
}

impl BookState {
    /// Every quest listed and `id` selected: the demo's.
    pub(crate) fn showing(id: &str) -> Self {
        let mut st = BookState::default();
        st.filter.show = Show::All;
        st.selected = Some(id.to_string());
        st
    }

    /// The cards' indices the list shows, in order: filtered and sorted again only when the
    /// filter, the order, the board, the switches or the character changed.
    fn listed(&mut self, v: &BookView) -> &[usize] {
        let cards = (v.cards.as_ptr() as usize, v.cards.len());
        if self.indexed != cards {
            self.index = v
                .cards
                .iter()
                .enumerate()
                .map(|(i, c)| (c.id.clone(), i))
                .collect();
            self.indexed = cards;
        }
        let key = ListKey {
            filter: self.filter.clone(),
            sort: self.sort,
            browse: self.browse.stamp,
            cfg: v.cfg.clone(),
            me: v.me,
            cards,
        };
        if self.listed_for.as_ref() != Some(&key) {
            let terms = parse_search(&self.filter.search);
            let mut rows: Vec<usize> = (0..v.cards.len())
                .filter(|&i| {
                    let card = &v.cards[i];
                    let line = self.browse.line(&card.id);
                    is_listed(card, line, v.cfg, &self.filter, &terms, v.me)
                })
                .collect();
            sort_rows(&mut rows, v.cards, &self.browse, self.sort);
            self.listed = rows;
            self.listed_for = Some(key);
        }
        &self.listed
    }

    pub(crate) fn card<'c>(&self, cards: &'c [QuestCard], id: &str) -> Option<&'c QuestCard> {
        self.index.get(id).and_then(|&i| cards.get(i))
    }
}

/// "20h", "7d", "any time", "once".
pub(crate) fn repeat_words(r: Repeat) -> String {
    match r {
        Repeat::Any => "any time".into(),
        Repeat::Once => "once".into(),
        Repeat::After(s) if s % DAY == 0 => format!("{}d", s / DAY),
        Repeat::After(s) if s % 3_600 == 0 => format!("{}h", s / 3_600),
        Repeat::After(s) => time_left(u64::from(s)),
    }
}

/// "20-50", "80+", "L80" (the easiest source), or nothing.
fn levels_words(card: &QuestCard) -> String {
    match (card.levels, card.easiest) {
        ((0, 0), Some(e)) if e > 0 => format!("L{e}"),
        ((0, 0), _) => String::new(),
        ((min, 0), _) => format!("{min}+"),
        ((min, max), _) => format!("{min}-{max}"),
    }
}

/// "2.5M", "75k", "900".
fn short_xp(xp: u64) -> String {
    match xp {
        0 => String::new(),
        x if x >= 1_000_000 => format!("{:.1}M", x as f64 / 1e6),
        x if x >= 1_000 => format!("{}k", x / 1_000),
        x => x.to_string(),
    }
}

/// A state in a word or two, for the list's column.
fn short_state(state: &QuestState, now: u64) -> String {
    match state {
        QuestState::Off => "off".into(),
        QuestState::Ready => "ready".into(),
        QuestState::Needs { more, .. } => format!("needs {more}"),
        QuestState::NotTaken { .. } => "take up".into(),
        QuestState::WontComplete { .. } => "won't complete".into(),
        QuestState::CantYet(Blocked::Level { .. }) => "level".into(),
        QuestState::CantYet(Blocked::Locked { .. }) => "locked".into(),
        QuestState::CantYet(Blocked::NeedsFirst { .. }) => "needs another".into(),
        QuestState::Waiting { until } => format!("in {}", time_left(until.saturating_sub(now))),
        QuestState::Done => "done".into(),
    }
}

/// The autoplay window's Quests section: "do quests", the count, the quest pursued and the button
/// that opens the window. True when the button was pressed.
pub(crate) fn summary(
    ui: &mut egui::Ui,
    cfg: &mut Quests,
    lines: &[QuestLine],
    pursuit: &[PursuitLine],
    now: u64,
) -> bool {
    title(ui, "Quests");
    ui.checkbox(&mut cfg.on, "do quests").on_hover_text(
        "Keep what the quests switched on want, and hand it in. Off, the quest rule wants \
         nothing and its items go to the rules below it; a pursued quest is still done",
    );
    caption(ui, count_line(lines));
    caption(
        ui,
        pursuit_summary(pursuit, cfg, now, &ac_world::quest_chain::place_name),
    );
    ui.button("Open the Quests window")
        .on_hover_text("Every quest: search them, switch them on, and have autoplay do one")
        .clicked()
}

/// What the window draws, borrowed from the session (or the demo) for one frame.
pub(crate) struct BookView<'a> {
    pub cards: &'a [QuestCard],
    pub cfg: &'a Quests,
    /// The acting board (`Client.quest_board`): what autoplay does now.
    pub acting: &'a [QuestLine],
    pub pursuit: &'a [PursuitLine],
    pub me: Me,
    pub held: &'a [String],
    pub log: &'a QuestLog,
    /// Unix seconds.
    pub now: u64,
    pub chain: &'a dyn Fn(&str) -> Vec<ChainLink>,
    pub place: &'a dyn Fn(u32) -> String,
}

/// A clicked-name or button the frame asked for, applied after drawing.
#[derive(Default)]
struct Asks {
    search: Option<String>,
    select: Option<String>,
    sort: Option<Column>,
}

/// Hover help for the search box.
const SEARCH_HELP: &str = "Words match a quest's title, item, creatures, givers, places, \
rewards and flags; every word must match. A prefix narrows a word to one of them:\n\
item:jaw  creature:eater  at:westwatch  giver:clemenza  reward:\"trade note\"  flag:eaterpit\n\
Quote a phrase: at:\"remote empyrean\". A name in the quest's details puts itself here.";

/// Draw the window; the edited switches when something was clicked.
pub(crate) fn draw(egui: &egui::Context, v: &BookView, st: &mut BookState) -> Option<Quests> {
    let mut cfg = v.cfg.clone();
    let mut asks = Asks::default();
    let w = egui.viewport_rect().width();
    let h = egui.viewport_rect().height();
    let width = 980.0_f32.min(w - 16.0);
    let height = 600.0_f32.min(h - 16.0);
    window(
        ID,
        egui::pos2((w - width) * 0.5, 30.0),
        egui::vec2(width, height),
        245,
        8,
    )
    .show(egui, |ui| {
        ui.set_min_size(egui::vec2(width - 16.0, height - 16.0));
        title_bar(ui, ID, "Quests");
        let listed = st.listed(v).len();
        ui.horizontal(|ui| {
            ui.checkbox(&mut cfg.on, "do quests").on_hover_text(
                "Autoplay keeps what the quests switched on want and hands it in; a pursued \
                 quest is done first",
            );
            if ui
                .button("Uncheck all")
                .on_hover_text(
                    "Switch every quest off: from now on only the ones you tick are on, new \
                     ones in later data too. Pursued quests are still done",
                )
                .clicked()
            {
                cfg.uncheck_all();
            }
            if ui
                .button("Check all XP quests")
                .on_hover_text("Every quest paying XP on again, unless switched off one by one")
                .clicked()
            {
                cfg.check_all();
            }
            caption(
                ui,
                format!(
                    "{listed} of {} quests listed; {}",
                    v.cards.len(),
                    count_line(v.acting)
                ),
            );
        });
        pursued_strip(ui, v, &mut cfg, st, &mut asks);
        filter_bar(ui, v.me, &mut st.filter);
        ui.separator();
        let pane = ui.available_height();
        let left = (ui.available_width() * 0.62).floor();
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(left, pane),
                egui::Layout::top_down(egui::Align::Min),
                |ui| list_pane(ui, v, &mut cfg, st, &mut asks, left),
            );
            ui.separator();
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), pane),
                egui::Layout::top_down(egui::Align::Min),
                |ui| detail_pane(ui, v, &mut cfg, st, &mut asks),
            );
        });
    });
    if let Some(search) = asks.search {
        st.filter.search = search;
    }
    if let Some(id) = asks.select {
        st.selected = (st.selected.as_deref() != Some(id.as_str())).then_some(id);
    }
    if let Some(col) = asks.sort {
        st.sort = if st.sort.col == col {
            Sort {
                col,
                descending: !st.sort.descending,
            }
        } else {
            Sort {
                col,
                descending: matches!(col, Column::Xp | Column::Levels),
            }
        };
    }
    (cfg != *v.cfg).then_some(cfg)
}

/// One line per pursued quest, first first, with its step and up, down and remove.
fn pursued_strip(
    ui: &mut egui::Ui,
    v: &BookView,
    cfg: &mut Quests,
    st: &BookState,
    asks: &mut Asks,
) {
    if v.cfg.pursue.is_empty() {
        return;
    }
    ui.add_space(2.0);
    caption(
        ui,
        "Pursued, first first: autoplay works the first that can go on",
    );
    for (i, p) in v.cfg.pursue.iter().enumerate() {
        let title = st
            .card(v.cards, &p.id)
            .map_or(p.id.as_str(), |c| c.title.as_str());
        let said = v.pursuit.iter().find(|l| l.id == p.id).map_or_else(
            || "not worked out yet".to_string(),
            |l| line_words(l, v.now, v.place),
        );
        let again = if p.again { " (again)" } else { "" };
        let end = match end_tag(&p.outcome, &p.id) {
            tag if tag.is_empty() => String::new(),
            tag => format!(" [{tag}]"),
        };
        ui.horizontal(|ui| {
            if ui
                .add_enabled(i > 0, egui::Button::new("up").small())
                .on_hover_text("Worked sooner")
                .clicked()
            {
                cfg.move_pursuit(&p.id, true);
            }
            if ui
                .add_enabled(
                    i + 1 < v.cfg.pursue.len(),
                    egui::Button::new("down").small(),
                )
                .on_hover_text("Worked later")
                .clicked()
            {
                cfg.move_pursuit(&p.id, false);
            }
            if ui
                .small_button("x")
                .on_hover_text("Stop pursuing it")
                .clicked()
            {
                cfg.stop_pursuing(&p.id);
            }
            let text =
                egui::RichText::new(format!("{}. {title}{again}{end} - {said}", i + 1)).small();
            if ui
                .add(
                    egui::Label::new(text)
                        .truncate()
                        .sense(egui::Sense::click()),
                )
                .on_hover_text("Show it")
                .clicked()
            {
                asks.select = Some(p.id.clone());
            }
        });
    }
}

/// The search, Show, levels, then the chips.
fn filter_bar(ui: &mut egui::Ui, me: Me, f: &mut Filter) {
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut f.search)
                .id_salt("quests.search")
                .hint_text("search, or item: creature: at: giver: reward: flag:")
                .desired_width(300.0),
        );
        if ui
            .add_enabled(!f.search.is_empty(), egui::Button::new("x").small())
            .clicked()
        {
            f.search.clear();
        }
        ui.label("?").on_hover_text(SEARCH_HELP);
        egui::ComboBox::from_id_salt("quests.show")
            .selected_text(f.show.label())
            .show_ui(ui, |ui| {
                for show in Show::ALL {
                    ui.selectable_value(&mut f.show, show, show.label());
                }
            });
        ui.label("levels");
        ui.add(egui::DragValue::new(&mut f.levels.0).range(0..=999))
            .on_hover_text("Lowest quest level, 0 for any");
        ui.label("to");
        ui.add(egui::DragValue::new(&mut f.levels.1).range(0..=999))
            .on_hover_text("Highest quest level, 0 for any");
        if ui
            .small_button("my level")
            .on_hover_text("From your level less the margin to twice the margin above it")
            .clicked()
        {
            f.levels = (me.level.saturating_sub(me.margin), me.level + 2 * me.margin);
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        let chip = |ui: &mut egui::Ui, on: bool, label: &str| {
            ui.selectable_label(on, egui::RichText::new(label).small())
                .clicked()
        };
        for (kind, label) in [
            (None, "any kind"),
            (Some(Kind::Item), "item"),
            (Some(Kind::Kill), "kill"),
            (Some(Kind::Talk), "talk"),
        ] {
            if chip(ui, f.kind == kind, label) {
                f.kind = kind;
            }
        }
        ui.separator();
        for r in RepeatChip::ALL {
            if chip(ui, f.repeat == r, r.label()) {
                f.repeat = r;
            }
        }
        ui.separator();
        egui::ComboBox::from_id_salt("quests.state")
            .selected_text(f.state.label())
            .show_ui(ui, |ui| {
                for s in StateChip::ALL {
                    ui.selectable_value(&mut f.state, s, s.label());
                }
            });
        let does = f
            .does
            .and_then(|w| DOES.iter().find(|d| d.0 == w))
            .map_or("does anything", |d| d.1);
        egui::ComboBox::from_id_salt("quests.does")
            .selected_text(does)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut f.does, None, "does anything");
                for (word, label) in DOES {
                    ui.selectable_value(&mut f.does, Some(*word), *label);
                }
            });
    });
}

/// A fixed-width, truncated, clickable cell.
fn cell(ui: &mut egui::Ui, width: f32, text: egui::RichText) -> egui::Response {
    ui.allocate_ui_with_layout(
        egui::vec2(width, 16.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.set_min_width(width);
            ui.add(
                egui::Label::new(text)
                    .truncate()
                    .sense(egui::Sense::click()),
            )
        },
    )
    .inner
}

/// Each column's width, the free ones sharing what the fixed ones leave of `width`.
fn widths(width: f32) -> [f32; 9] {
    let fixed = 18.0 + 34.0 + 46.0 + 50.0 + 44.0 + 78.0 + 9.0 * 4.0;
    let free = (width - fixed).max(150.0);
    [
        18.0,
        free * 0.45,
        34.0,
        free * 0.28,
        free * 0.27,
        46.0,
        50.0,
        44.0,
        78.0,
    ]
}

fn list_pane(
    ui: &mut egui::Ui,
    v: &BookView,
    cfg: &mut Quests,
    st: &mut BookState,
    asks: &mut Asks,
    width: f32,
) {
    let w = widths(width);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.add_space(w[0]);
        for (i, col) in Column::ALL.into_iter().enumerate() {
            let mark = match (st.sort.col == col, st.sort.descending) {
                (false, _) => "",
                (true, true) => " v",
                (true, false) => " ^",
            };
            let text = egui::RichText::new(format!("{}{mark}", col.label()))
                .color(egui::Color32::from_gray(170))
                .small();
            if cell(ui, w[i + 1], text)
                .on_hover_text("Click to sort")
                .clicked()
            {
                asks.sort = Some(col);
            }
        }
    });
    let listed = st.listed(v).to_vec();
    if listed.is_empty() {
        caption(
            ui,
            if v.cards.is_empty() {
                "no quest data"
            } else {
                "no quest matches"
            },
        );
        return;
    }
    let row = ui.spacing().interact_size.y;
    egui::ScrollArea::vertical()
        .id_salt("quests.list")
        .auto_shrink([false, false])
        .max_height(ui.available_height())
        .show_rows(ui, row, listed.len(), |ui, range| {
            for &i in &listed[range] {
                let card = &v.cards[i];
                let line = st.browse.line(&card.id);
                quest_row(ui, v, cfg, st.selected.as_deref(), card, line, asks, &w);
            }
        });
}

#[allow(clippy::too_many_arguments)] // one row's cells read the view, the switches and the asks
fn quest_row(
    ui: &mut egui::Ui,
    v: &BookView,
    cfg: &mut Quests,
    selected: Option<&str>,
    card: &QuestCard,
    line: Option<&QuestLine>,
    asks: &mut Asks,
    w: &[f32; 9],
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let mut on = is_switched_on(cfg, &card.id);
        if ui
            .add_sized([w[0], 16.0], egui::Checkbox::without_text(&mut on))
            .on_hover_text("Autoplay considers it")
            .changed()
        {
            set_switch(cfg, &card.id, on);
        }
        let mark = cfg
            .pursue
            .iter()
            .position(|p| p.id == card.id)
            .map_or(String::new(), |n| format!(">{} ", n + 1));
        let is_selected = selected == Some(card.id.as_str());
        let title_colour = if is_selected {
            egui::Color32::from_rgb(255, 220, 120)
        } else {
            egui::Color32::from_gray(225)
        };
        let small = |s: String| egui::RichText::new(s).small();
        let (giver, at) = giver_of(card, line);
        let state = line.map(|l| &l.state);
        let state_text = state.map_or(String::new(), |s| short_state(s, v.now));
        let state_colour = state.map_or(egui::Color32::from_gray(140), colour);
        let cells = [
            egui::RichText::new(format!("{mark}{}", card.title))
                .small()
                .color(title_colour),
            small(kind_word(card.kind).into()),
            small(giver.into()),
            small(at.into()),
            small(levels_words(card)),
            small(short_xp(xp_of(card, line))),
            small(repeat_words(card.repeat)),
            small(state_text).color(state_colour),
        ];
        let mut clicked = false;
        for (j, text) in cells.into_iter().enumerate() {
            clicked |= cell(ui, w[j + 1], text).clicked();
        }
        if clicked {
            asks.select = Some(card.id.clone());
        }
    });
}

/// One step's words, a name a link that fills the search.
fn step_row(ui: &mut egui::Ui, n: usize, step: &Step, asks: &mut Asks) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let colour = if step.done {
            egui::Color32::from_rgb(150, 210, 150)
        } else {
            egui::Color32::from_gray(215)
        };
        let lead = if step.done {
            format!("{n}. done: ")
        } else {
            format!("{n}. ")
        };
        ui.label(egui::RichText::new(lead).small().color(colour));
        for piece in &step.pieces {
            match piece {
                Piece::Text(t) => {
                    ui.label(egui::RichText::new(t).small().color(colour));
                }
                Piece::Link(facet, name) => {
                    if ui
                        .link(egui::RichText::new(name).small())
                        .on_hover_text("List the quests with this")
                        .clicked()
                    {
                        asks.search = Some(search_word(*facet, name));
                    }
                }
            }
        }
    });
}

/// Names as links in a row after a label ("Rewards: ...").
fn links_row(ui: &mut egui::Ui, label: &str, facet: Facet, names: &[String], asks: &mut Asks) {
    if names.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(egui::RichText::new(label).small());
        for name in names {
            if ui
                .link(egui::RichText::new(name).small())
                .on_hover_text("List the quests with this")
                .clicked()
            {
                asks.search = Some(search_word(facet, name));
            }
        }
    });
}

fn detail_pane(
    ui: &mut egui::Ui,
    v: &BookView,
    cfg: &mut Quests,
    st: &mut BookState,
    asks: &mut Asks,
) {
    let Some(card) = st.selected.as_deref().and_then(|id| st.card(v.cards, id)) else {
        caption(
            ui,
            "Click a quest to see its chain and to have autoplay do it",
        );
        return;
    };
    let line = st.browse.line(&card.id);
    let acting = v.acting.iter().find(|l| l.id == card.id);
    egui::ScrollArea::vertical()
        .id_salt("quests.detail")
        .auto_shrink([false, false])
        .max_height(ui.available_height())
        .show(ui, |ui| {
            title(ui, &card.title);
            let xp = match xp_of(card, line) {
                0 => "no XP".to_string(),
                xp => format!("{} XP at your level", super::skills::thousands(xp as i64)),
            };
            caption(
                ui,
                format!(
                    "{} · {} · {xp} · {}",
                    card.id,
                    kind_word(card.kind),
                    repeat_words(card.repeat),
                ),
            );
            state_lines(ui, v, cfg, card, line, acting, &mut st.add_rule);
            controls(ui, v, cfg, card);
            ui.separator();
            title(ui, "What to do");
            // Worked out on a new selection or board, not every frame.
            let stamp = (card.id.clone(), st.browse.stamp);
            if st.steps_for.as_ref() != Some(&stamp) {
                let k = Known {
                    held: v.held,
                    log: v.log,
                    me: v.me,
                    place: v.place,
                };
                let links = (v.chain)(&card.id);
                st.steps = if links.is_empty() {
                    card_steps(card, &k)
                } else {
                    chain_words(&links, &k)
                };
                st.steps_for = Some(stamp);
            }
            for (i, step) in st.steps.iter().enumerate() {
                step_row(ui, i + 1, step, asks);
            }
            ui.add_space(4.0);
            let xp = xp_of(card, line);
            if xp > 0 {
                caption(
                    ui,
                    format!("Reward: {} XP", super::skills::thousands(xp as i64)),
                );
            }
            let random = card.unread.iter().any(|u| u == "a random reply");
            let gives = if random { "Gives one of:" } else { "Gives:" };
            links_row(ui, gives, Facet::Reward, &card.gives, asks);
            links_row(ui, "Needs flags:", Facet::Flag, &card.flags, asks);
            if !card.does.is_empty() {
                caption(ui, format!("Does: {}", card.does.join(", ")));
            }
            if !card.unread.is_empty() {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(
                        "The server also checks what autoplay can't read, so it works this \
                         quest that way only when you switch it on or pursue it:",
                    )
                    .small()
                    .color(egui::Color32::from_rgb(230, 160, 80)),
                );
                for u in &card.unread {
                    caption(ui, format!("- {u}"));
                }
            }
        });
}

/// The quest's state for the character, what autoplay does with it now, and its pursuit line.
fn state_lines(
    ui: &mut egui::Ui,
    v: &BookView,
    cfg: &Quests,
    card: &QuestCard,
    line: Option<&QuestLine>,
    acting: Option<&QuestLine>,
    add_rule: &mut bool,
) {
    if let Some(line) = line {
        let said = words(&line.state, true, v.now);
        ui.label(
            egui::RichText::new(format!("{said} ({})", to_whom(line)))
                .small()
                .color(colour(&line.state)),
        );
        let unkept =
            matches!(&line.state, QuestState::WontComplete { rule, .. } if rule == NO_RULE_KEEPS);
        if unkept
            && ui
                .button("Add the quest rule at the top")
                .on_hover_text(
                    "The profile has no rule keeping quest items: put \"quest items\" (Keep, \
                     wanted by an active quest) first in it",
                )
                .clicked()
        {
            *add_rule = true;
        }
    }
    let now = match acting {
        Some(a) if a.on => format!("autoplay: {}", words(&a.state, cfg.on, v.now)),
        _ if !cfg.on => "autoplay: do quests is off".into(),
        _ if is_switched_on(cfg, &card.id) || cfg.is_pursued(&card.id) => {
            "autoplay: switched on; worked out within 2 s".into()
        }
        _ => "autoplay: switched off".into(),
    };
    caption(ui, now);
    if let Some(p) = v.pursuit.iter().find(|l| l.id == card.id) {
        caption(ui, format!("pursuit: {}", line_words(p, v.now, v.place)));
    }
}

/// The on switch and the pursuit buttons.
fn controls(ui: &mut egui::Ui, v: &BookView, cfg: &mut Quests, card: &QuestCard) {
    let id = card.id.as_str();
    let mut on = is_switched_on(cfg, id);
    let hover = if cfg.only_chosen {
        "Only the quests ticked here are on (Uncheck all): tick it to have autoplay do it"
    } else if ac_client::quest_log::is_automatic_quest(id) {
        "It pays XP with nothing autoplay can't read: on unless switched off here"
    } else {
        "It pays no XP or asks what autoplay can't read: worked only when switched on here"
    };
    if ui
        .checkbox(&mut on, "autoplay considers it")
        .on_hover_text(hover)
        .changed()
    {
        set_switch(cfg, id, on);
    }
    ui.horizontal(|ui| match cfg.pursue.iter().position(|p| p.id == id) {
        Some(i) => {
            let stop = ui
                .button("Stop")
                .on_hover_text("Take it off the pursued list")
                .clicked();
            ui.checkbox(&mut cfg.pursue[i].again, "again whenever it's ready")
                .on_hover_text("Off, it is done once more and then left");
            // Taken off after its checkbox drew: the list is one shorter once it goes.
            if stop {
                cfg.stop_pursuing(id);
            } else {
                end_picker(ui, cfg, id);
            }
        }
        None => {
            if ui
                .button("Do this quest")
                .on_hover_text("Autoplay does it first, ahead of hunting for XP, once")
                .clicked()
            {
                cfg.pursue_quest(id, false, v.now);
            }
            if ui
                .button("Do it whenever it's ready")
                .on_hover_text("A standing order, last on the list: done again after each wait")
                .clicked()
            {
                cfg.pursue_quest(id, true, v.now);
            }
        }
    });
}

/// The pursued quest `id`'s end, picked from what it offers: worked to that end with no more asking.
fn end_picker(ui: &mut egui::Ui, cfg: &mut Quests, id: &str) {
    let choices = end_choices(id);
    if choices.len() < 2 {
        return;
    }
    let mut end = cfg.outcome_of(id).clone();
    egui::ComboBox::from_id_salt("quests.outcome")
        .selected_text(end_words(&end, id))
        .show_ui(ui, |ui| {
            for (choice, words) in choices {
                ui.selectable_value(&mut end, choice, words);
            }
        })
        .response
        .on_hover_text(
            "How it ends: handed in to any giver, to the one picked, or the item kept (a Keep \
             rule for it joins your loot rules)",
        );
    if end != *cfg.outcome_of(id) {
        cfg.set_outcome(id, end);
    }
}

#[cfg(test)]
mod tests;
