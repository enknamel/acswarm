//! The profile editor's rule list: rules filed under folding group headings, and a search line.
//! A group is only a heading (`Rule::group`): the rules are still read in list order, numbered so.

use super::{action_box, id_marker, rule_editor, text_field, unused_name, Editor, DIM};
use crate::egui;
use crate::panels::caption;
use ac_client::profile::{Profile, Rule, QUESTS};

/// One heading of the rule list: a group and the rules shown under it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Section {
    /// The group's name; empty is the rules filed under none.
    pub group: String,
    /// Indices into the profile's rules, in reading order.
    pub rules: Vec<usize>,
}

/// Whether the search line finds `rule` in its name, group, action or conditions, in any case.
/// `needle` is lower case already; an empty one finds every rule.
pub(super) fn is_found(rule: &Rule, needle: &str) -> bool {
    needle.is_empty()
        || [
            rule.name.as_str(),
            rule.group.as_str(),
            rule.action.label(),
            &rule.tell(),
        ]
        .iter()
        .any(|text| text.to_lowercase().contains(needle))
}

/// The rules cut into sections, each placed where its first rule is read; with a search line,
/// only the rules it finds, and no section left empty.
pub(super) fn sections(rules: &[Rule], search: &str) -> Vec<Section> {
    let needle = search.trim().to_lowercase();
    let mut out: Vec<Section> = Vec::new();
    for (i, rule) in rules.iter().enumerate() {
        let at = match out.iter().position(|s| s.group == rule.group) {
            Some(at) => at,
            None => {
                out.push(Section {
                    group: rule.group.clone(),
                    rules: Vec::new(),
                });
                out.len() - 1
            }
        };
        if is_found(rule, &needle) {
            out[at].rules.push(i);
        }
    }
    out.retain(|s| !s.rules.is_empty());
    out
}

/// Every group the rules are filed under, each once, in reading order; the ungrouped left out.
pub(super) fn group_names(rules: &[Rule]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for rule in rules {
        if !rule.group.is_empty() && !names.contains(&rule.group) {
            names.push(rule.group.clone());
        }
    }
    names
}

/// Files every rule of group `from` under `to` instead; an existing `to` takes them in.
pub(super) fn rename_group(rules: &mut [Rule], from: &str, to: &str) {
    for rule in rules.iter_mut().filter(|r| r.group == from) {
        rule.group = to.trim().to_string();
    }
}

/// The nearest rule of rule `i`'s own group above or below it.
fn neighbour(rules: &[Rule], i: usize, up: bool) -> Option<usize> {
    let group = &rules.get(i)?.group;
    if up {
        (0..i).rev().find(|&j| rules[j].group == *group)
    } else {
        (i + 1..rules.len()).find(|&j| rules[j].group == *group)
    }
}

/// Moves rule `i` past the nearest rule of its own group, up or down: the place it lands at.
/// Rules of other groups in between are read after it going up, before it going down.
pub(super) fn move_in_group(rules: &mut Vec<Rule>, i: usize, up: bool) -> Option<usize> {
    let to = neighbour(rules, i, up)?;
    let rule = rules.remove(i);
    rules.insert(to, rule);
    Some(to)
}

/// Where rule `at` sits once the rule at `from` has been taken out and put back at `to`.
fn shifted(at: usize, from: usize, to: usize) -> usize {
    if at == from {
        to
    } else if from < at && at <= to {
        at - 1
    } else if to <= at && at < from {
        at + 1
    } else {
        at
    }
}

/// What a frame of the list asked for, done once the list is drawn.
enum Change {
    Drop(usize),
    Copy(usize),
    Move(usize, bool),
    /// A new rule in this group, read after its last; in none, read last of all.
    Add(String),
}

/// The rule list with its search line above and the buttons that add a rule or a group below.
pub(super) fn list(ui: &mut egui::Ui, p: &mut Profile, editor: &mut Editor) {
    ui.horizontal(|ui| {
        caption(
            ui,
            "rules, in order: the first that claims an item decides it",
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !editor.rule_search.is_empty()
                && ui
                    .add(egui::Button::new("x").small())
                    .on_hover_text("Clear the search")
                    .clicked()
            {
                editor.rule_search.clear();
            }
            text_field(
                ui,
                "loot_profiles.rule_search",
                &mut editor.rule_search,
                "search rules",
                130.0,
            );
        });
    });
    let mut change = None;
    egui::ScrollArea::vertical()
        .id_salt("loot_profiles.rules")
        .max_height(240.0)
        .show(ui, |ui| {
            ui.set_min_width(440.0);
            if p.rules.is_empty() {
                caption(ui, "(no rules: nothing is taken)");
            }
            change = sections_ui(ui, p, editor);
        });
    ui.horizontal(|ui| {
        if ui
            .add(egui::Button::new("Add a rule").small())
            .on_hover_text("A new rule at the end, read last, in no group")
            .clicked()
        {
            change = Some(Change::Add(String::new()));
        }
        ui.add_space(12.0);
        text_field(
            ui,
            "loot_profiles.new_group",
            &mut editor.new_group,
            "new group, e.g. rending salvage",
            200.0,
        );
        let name = editor.new_group.trim().to_string();
        if ui
            .add_enabled(!name.is_empty(), egui::Button::new("Add a group").small())
            .on_hover_text("A new rule filed under this group, read last")
            .clicked()
        {
            change = Some(Change::Add(name));
            editor.new_group.clear();
        }
    });
    if let Some(change) = change {
        apply_change(p, editor, change);
    }
}

/// Every section in reading order, under a heading once any rule has a group.
fn sections_ui(ui: &mut egui::Ui, p: &mut Profile, editor: &mut Editor) -> Option<Change> {
    let searching = !editor.rule_search.trim().is_empty();
    let headed = p.rules.iter().any(|r| !r.group.is_empty());
    let shown = sections(&p.rules, &editor.rule_search);
    if searching && shown.is_empty() {
        caption(ui, "(no rule matches the search)");
    }
    let names = group_names(&p.rules);
    let mut change = None;
    for s in &shown {
        // A search opens every heading it found a rule under.
        let open = searching || !editor.shut.contains(&s.group);
        if !headed {
            for &i in &s.rules {
                rule_row(ui, p, i, &names, editor, &mut change);
            }
            continue;
        }
        heading(ui, p, s, open, searching, editor, &mut change);
        if open {
            ui.indent(format!("loot_profiles.group.{}", s.group), |ui| {
                for &i in &s.rules {
                    rule_row(ui, p, i, &names, editor, &mut change);
                }
            });
        }
    }
    change
}

/// A group's heading: folds it, counts its rules, adds one to it, renames it.
fn heading(
    ui: &mut egui::Ui,
    p: &mut Profile,
    s: &Section,
    open: bool,
    searching: bool,
    editor: &mut Editor,
    change: &mut Option<Change>,
) {
    let total = p.rules.iter().filter(|r| r.group == s.group).count();
    ui.horizontal(|ui| {
        if let Some((from, to)) = &mut editor.group_rename {
            if *from == s.group {
                text_field(ui, "loot_profiles.group_rename", to, "group name", 160.0);
                let to = to.trim().to_string();
                if ui
                    .add_enabled(!to.is_empty(), egui::Button::new("Rename").small())
                    .clicked()
                {
                    rename_group(&mut p.rules, &s.group, &to);
                    if editor.shut.remove(&s.group) {
                        editor.shut.insert(to);
                    }
                    editor.group_rename = None;
                } else if ui.add(egui::Button::new("Cancel").small()).clicked() {
                    editor.group_rename = None;
                }
                return;
            }
        }
        let fold = if open { "▾" } else { "▸" };
        let name = if s.group.is_empty() {
            "no group"
        } else {
            &s.group
        };
        let count = if searching {
            format!("{} of {total}", s.rules.len())
        } else {
            total.to_string()
        };
        let label = egui::RichText::new(format!("{fold} {name}  ({count})")).strong();
        if ui
            .selectable_label(false, label)
            .on_hover_text("Fold or unfold this group; it changes nothing about reading order")
            .clicked()
            && !searching
            && !editor.shut.remove(&s.group)
        {
            editor.shut.insert(s.group.clone());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(egui::Button::new("+").small())
                .on_hover_text("A new rule in this group, read after its last")
                .clicked()
            {
                *change = Some(Change::Add(s.group.clone()));
            }
            // The quest system finds its own rules by this group's name (`profile::QUESTS`).
            let named = !s.group.is_empty() && s.group != QUESTS;
            if ui
                .add_enabled(named, egui::Button::new("rename").small())
                .on_hover_text("Rename this group on every rule filed under it")
                .clicked()
            {
                editor.group_rename = Some((s.group.clone(), s.group.clone()));
            }
        });
    });
}

/// One rule's line, its conditions in words under it, and its editor when open.
fn rule_row(
    ui: &mut egui::Ui,
    p: &mut Profile,
    i: usize,
    names: &[String],
    editor: &mut Editor,
    change: &mut Option<Change>,
) {
    let open = editor.open_rule == Some(i);
    let earlier = neighbour(&p.rules, i, true).is_some();
    let later = neighbour(&p.rules, i, false).is_some();
    ui.horizontal(|ui| {
        // Straight to the library, so a rule switched off here is
        // off everywhere before the next corpse.
        ui.checkbox(&mut p.rules[i].on, "")
            .on_hover_text("Off without being deleted");
        ui.label(egui::RichText::new(format!("{}", i + 1)).small().color(DIM))
            .on_hover_text("Its place in the reading: the first rule that claims an item decides");
        let name = if p.rules[i].name.trim().is_empty() {
            format!("rule {}", i + 1)
        } else {
            p.rules[i].name.clone()
        };
        if ui
            .selectable_label(open, name)
            .on_hover_text("Open its conditions")
            .clicked()
        {
            editor.open_rule = if open { None } else { Some(i) };
        }
        action_box(
            ui,
            &format!("loot_profiles.rule{i}.act"),
            &mut p.rules[i].action,
        );
        if p.rules[i].needs_id() {
            id_marker(ui);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add(egui::Button::new("x").small())
                .on_hover_text("Delete this rule")
                .clicked()
            {
                *change = Some(Change::Drop(i));
            }
            if ui
                .add(egui::Button::new("+").small())
                .on_hover_text("A copy of this rule, under it")
                .clicked()
            {
                *change = Some(Change::Copy(i));
            }
            if ui
                .add_enabled(later, egui::Button::new("v").small())
                .on_hover_text("Read this rule after the next one in its group")
                .clicked()
            {
                *change = Some(Change::Move(i, false));
            }
            if ui
                .add_enabled(earlier, egui::Button::new("^").small())
                .on_hover_text("Read this rule before the one above it in its group")
                .clicked()
            {
                *change = Some(Change::Move(i, true));
            }
        });
    });
    let told = p.rules[i].tell();
    ui.add(egui::Label::new(egui::RichText::new(told).small().color(DIM)).wrap());
    if open {
        egui::Frame::new()
            .inner_margin(4)
            .fill(egui::Color32::from_black_alpha(60))
            .show(ui, |ui| {
                let salt = format!("loot_profiles.rule{i}");
                group_row(ui, &salt, &mut p.rules[i], names, &mut editor.move_to);
                rule_editor(ui, &salt, &mut p.rules[i], editor);
            });
    }
    ui.add_space(2.0);
}

/// The open rule's group: one already in use, none, or a new one typed and filed.
fn group_row(ui: &mut egui::Ui, salt: &str, rule: &mut Rule, names: &[String], draft: &mut String) {
    ui.horizontal(|ui| {
        caption(ui, "group");
        let shown = if rule.group.is_empty() {
            "none".to_string()
        } else {
            rule.group.clone()
        };
        egui::ComboBox::from_id_salt(format!("{salt}.group"))
            .selected_text(shown)
            .width(130.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut rule.group, String::new(), "none");
                for name in names {
                    ui.selectable_value(&mut rule.group, name.clone(), name);
                }
            });
        text_field(
            ui,
            &format!("{salt}.new_group"),
            draft,
            "or a new one",
            130.0,
        );
        let name = draft.trim().to_string();
        if ui
            .add_enabled(!name.is_empty(), egui::Button::new("File").small())
            .on_hover_text("File this rule under the group typed; its reading place stays")
            .clicked()
        {
            rule.group = name;
            draft.clear();
        }
    });
}

/// Does what a frame of the list asked, keeping the rule open the same rule.
fn apply_change(p: &mut Profile, editor: &mut Editor, change: Change) {
    let names: Vec<String> = p.rules.iter().map(|r| r.name.clone()).collect();
    match change {
        Change::Drop(i) => {
            p.rules.remove(i);
            editor.open_rule = None;
        }
        Change::Copy(i) => {
            let mut made = p.rules[i].clone();
            // Names are how a tag remembers its rule (`Ledger::why`): never two alike.
            made.name = unused_name(&names, &format!("{} copy", made.name));
            p.rules.insert(i + 1, made);
            editor.open_rule = Some(i + 1);
        }
        Change::Move(i, up) => {
            if let Some(to) = move_in_group(&mut p.rules, i, up) {
                editor.open_rule = editor.open_rule.map(|at| shifted(at, i, to));
            }
        }
        Change::Add(group) => {
            // An ungrouped rule goes last, as before groups; a grouped one after its group's last.
            let at = p
                .rules
                .iter()
                .rposition(|r| !group.is_empty() && r.group == group)
                .map_or(p.rules.len(), |last| last + 1);
            p.rules.insert(
                at,
                Rule {
                    name: unused_name(&names, &format!("rule {}", p.rules.len() + 1)),
                    group: group.clone(),
                    ..Default::default()
                },
            );
            editor.shut.remove(&group);
            editor.open_rule = Some(at);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_client::autoplay::LootAction;

    fn rule(name: &str, group: &str) -> Rule {
        Rule {
            name: name.into(),
            group: group.into(),
            ..Default::default()
        }
    }

    fn rending() -> Vec<Rule> {
        vec![
            rule("pyreals", ""),
            rule("Red Garnet", "Rending salvage"),
            rule("trade notes", ""),
            Rule {
                action: LootAction::Sell,
                ..rule("vendor trash", "Junk")
            },
            rule("White Sapphire", "Rending salvage"),
        ]
    }

    fn shown(s: &[Section]) -> Vec<(&str, Vec<usize>)> {
        s.iter()
            .map(|s| (s.group.as_str(), s.rules.clone()))
            .collect()
    }

    #[test]
    fn sections_follow_reading_order_and_keep_each_rules_place() {
        assert_eq!(
            shown(&sections(&rending(), "")),
            vec![
                ("", vec![0, 2]),
                ("Rending salvage", vec![1, 4]),
                ("Junk", vec![3]),
            ]
        );
        assert!(sections(&[], "").is_empty());
    }

    #[test]
    fn a_search_shows_only_the_rules_it_finds_in_any_case() {
        let rules = rending();
        assert_eq!(
            shown(&sections(&rules, "  sapphire ")),
            vec![("Rending salvage", vec![4])]
        );
        // The group's own name finds every rule filed under it.
        assert_eq!(
            shown(&sections(&rules, "RENDING")),
            vec![("Rending salvage", vec![1, 4])]
        );
        // So does what a rule does, and what it asks in words.
        assert_eq!(shown(&sections(&rules, "sell")), vec![("Junk", vec![3])]);
        assert_eq!(
            shown(&sections(&rules, "anything")).len(),
            3,
            "a rule asking nothing reads as anything"
        );
        assert!(sections(&rules, "Black Garnet").is_empty());
    }

    #[test]
    fn a_rule_moves_past_the_next_of_its_own_group_only() {
        let mut rules = rending();
        assert_eq!(move_in_group(&mut rules, 0, true), None);
        assert_eq!(
            move_in_group(&mut rules, 3, false),
            None,
            "Junk has one rule"
        );
        assert_eq!(move_in_group(&mut rules, 4, true), Some(1));
        let names: Vec<&str> = rules.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "pyreals",
                "White Sapphire",
                "Red Garnet",
                "trade notes",
                "vendor trash"
            ]
        );
        assert_eq!(move_in_group(&mut rules, 0, false), Some(3));
        assert_eq!(rules[3].name, "pyreals");
        assert_eq!(rules[2].name, "trade notes");
    }

    #[test]
    fn the_rule_held_open_is_followed_through_a_move() {
        assert_eq!(shifted(4, 4, 1), 1);
        assert_eq!(shifted(1, 4, 1), 2);
        assert_eq!(shifted(3, 0, 3), 2);
        assert_eq!(shifted(4, 0, 3), 4);
    }

    #[test]
    fn a_rule_added_to_a_group_is_read_after_its_last_and_opens() {
        let mut p = Profile {
            rules: rending(),
            ..Default::default()
        };
        let mut editor = Editor::default();
        editor.shut.insert("Rending salvage".into());
        apply_change(&mut p, &mut editor, Change::Add("Rending salvage".into()));
        assert_eq!(p.rules[5].group, "Rending salvage");
        assert_eq!(editor.open_rule, Some(5));
        assert!(editor.shut.is_empty(), "its group unfolds");
        apply_change(&mut p, &mut editor, Change::Add("Jewels".into()));
        assert_eq!(
            (p.rules[6].group.as_str(), editor.open_rule),
            ("Jewels", Some(6))
        );
        apply_change(&mut p, &mut editor, Change::Add(String::new()));
        assert_eq!((p.rules[7].group.as_str(), editor.open_rule), ("", Some(7)));
        apply_change(&mut p, &mut editor, Change::Add("Junk".into()));
        assert_eq!(
            (p.rules[4].group.as_str(), editor.open_rule),
            ("Junk", Some(4))
        );
        let mut names: Vec<&str> = p.rules.iter().map(|r| r.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), p.rules.len(), "every name new");
    }

    #[test]
    fn a_group_renamed_carries_every_rule_and_names_list_once() {
        let mut rules = rending();
        assert_eq!(group_names(&rules), ["Rending salvage", "Junk"]);
        rename_group(&mut rules, "Rending salvage", " Imbue salvage ");
        assert_eq!(group_names(&rules), ["Imbue salvage", "Junk"]);
        rename_group(&mut rules, "Junk", "Imbue salvage");
        assert_eq!(group_names(&rules), ["Imbue salvage"]);
        assert_eq!(rules[0].group, "", "the ungrouped stay so");
    }
}
