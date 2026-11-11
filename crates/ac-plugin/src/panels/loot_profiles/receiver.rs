//! A give rule's receiver in the profile editor: the nearest NPC that takes anything, an NPC by
//! name (handed over on a town run) or a player by name (only in view and in reach).

use super::text_field;
use crate::egui;
use crate::panels::caption;
use ac_client::autoplay::LootAction;
use ac_client::profile::{GiveTo, Rule};

/// The "give to" line under a give rule; nothing for any other action.
pub(super) fn row(ui: &mut egui::Ui, salt: &str, rule: &mut Rule) {
    if rule.action != LootAction::Give {
        return;
    }
    ui.horizontal(|ui| {
        caption(ui, "give to");
        give_to_box(ui, &format!("{salt}.give_to"), &mut rule.give_to);
    });
}

fn give_to_box(ui: &mut egui::Ui, salt: &str, give_to: &mut Option<GiveTo>) {
    let kinds = ["nobody", "nearest taker", "NPC named", "player named"];
    let mut kind = match give_to {
        None => 0,
        Some(GiveTo::Taker) => 1,
        Some(GiveTo::Npc(_)) => 2,
        Some(GiveTo::Player(_)) => 3,
    };
    let was = kind;
    egui::ComboBox::from_id_salt(format!("{salt}.kind"))
        .selected_text(kinds[kind])
        .width(100.0)
        .show_ui(ui, |ui| {
            for (i, k) in kinds.iter().enumerate() {
                ui.selectable_value(&mut kind, i, *k);
            }
        });
    if kind != was {
        *give_to = match kind {
            1 => Some(GiveTo::Taker),
            2 => Some(GiveTo::Npc(String::new())),
            3 => Some(GiveTo::Player(String::new())),
            _ => None,
        };
    }
    match give_to {
        Some(GiveTo::Npc(name)) => {
            text_field(ui, &format!("{salt}.npc"), name, "Terese", 140.0);
            ui.weak("handed over on a town run calling in its town");
        }
        Some(GiveTo::Player(name)) => {
            text_field(ui, &format!("{salt}.player"), name, "Mule", 140.0);
            ui.weak("handed over only when in view and in reach");
        }
        Some(GiveTo::Taker) => {
            ui.weak("a Town Crier, Garbage Barrel or the like, on a town run");
        }
        None => {
            ui.weak("kept until a receiver is named");
        }
    }
}
