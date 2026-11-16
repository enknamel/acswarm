//! The inventory panel's equipment view: every slot ACE has (EquipMask.cs:12-42), what is worn in
//! it, the armour areas left bare, and each slot a drop target that wears the item dropped on it.

use ac_world::equip as e;

use super::{Actions, InventoryView};
use crate::egui;
use crate::panels::{caption, ItemDrag};

/// One slot as shown: its name and the `ac_world::equip` bits it stands for.
pub(crate) struct Slot {
    pub name: &'static str,
    pub mask: u32,
}

const fn slot(name: &'static str, mask: u32) -> Slot {
    Slot { name, mask }
}

/// The outer layer by body area, head, hands and feet included: what armour covers.
pub(crate) const ARMOUR: [Slot; 9] = [
    slot("Head", e::HEAD_WEAR),
    slot("Chest", e::CHEST_ARMOR),
    slot("Abdomen", e::ABDOMEN_ARMOR),
    slot("Upper arms", e::UPPER_ARM_ARMOR),
    slot("Lower arms", e::LOWER_ARM_ARMOR),
    slot("Hands", e::HAND_WEAR),
    slot("Upper legs", e::UPPER_LEG_ARMOR),
    slot("Lower legs", e::LOWER_LEG_ARMOR),
    slot("Feet", e::FOOT_WEAR),
];

/// The under layer: shirts and trousers.
const UNDER: [Slot; 6] = [
    slot("Chest", e::CHEST_WEAR),
    slot("Abdomen", e::ABDOMEN_WEAR),
    slot("Upper arms", e::UPPER_ARM_WEAR),
    slot("Lower arms", e::LOWER_ARM_WEAR),
    slot("Upper legs", e::UPPER_LEG_WEAR),
    slot("Lower legs", e::LOWER_LEG_WEAR),
];

const JEWELLERY: [Slot; 7] = [
    slot("Neck", e::NECK),
    slot("Left wrist", e::WRIST_LEFT),
    slot("Right wrist", e::WRIST_RIGHT),
    slot("Left finger", e::FINGER_LEFT),
    slot("Right finger", e::FINGER_RIGHT),
    slot("Trinket", e::TRINKET),
    slot("Cloak", e::CLOAK),
];

const HANDS: [Slot; 3] = [
    slot(
        "Weapon",
        e::MELEE_WEAPON | e::MISSILE_WEAPON | e::HELD | e::TWO_HANDED,
    ),
    slot("Shield", e::SHIELD),
    slot("Ammunition", e::MISSILE_AMMO),
];

const SIGILS: [Slot; 3] = [
    slot("Sigil one", e::SIGIL_ONE),
    slot("Sigil two", e::SIGIL_TWO),
    slot("Sigil three", e::SIGIL_THREE),
];

const GROUPS: [(&str, &[Slot]); 5] = [
    ("Armour", &ARMOUR),
    ("Under clothes", &UNDER),
    ("Jewellery", &JEWELLERY),
    ("Hands", &HANDS),
    ("Aetheria", &SIGILS),
];

/// Where to ask for an item of `valid` locations dropped on slot `mask`: clothing and armour take
/// every area they cover; anything else the slot's own bit (a ring on the finger it was dropped on).
/// None when it cannot go there.
pub(crate) fn wear_location(valid: u32, mask: u32) -> Option<u32> {
    if valid & mask == 0 {
        return None;
    }
    Some(if mask & e::COVERAGE != 0 {
        valid & e::COVERAGE
    } else {
        valid & mask
    })
}

/// What is worn in slot `mask`, by guid.
fn worn_in(worn: &[(u32, u32)], mask: u32) -> Vec<u32> {
    worn.iter()
        .filter(|(_, at)| at & mask != 0)
        .map(|(g, _)| *g)
        .collect()
}

/// The armour areas nothing covers, by name.
pub(crate) fn bare(worn: &[(u32, u32)]) -> Vec<&'static str> {
    ARMOUR
        .iter()
        .filter(|s| worn_in(worn, s.mask).is_empty())
        .map(|s| s.name)
        .collect()
}

/// Draw the view, folded behind its header; drops and double-clicks go into `actions`.
pub(crate) fn draw(ui: &mut egui::Ui, v: &InventoryView, folded: &mut bool, actions: &mut Actions) {
    let bare = bare(&v.worn);
    let fold = if *folded { "▸" } else { "▾" };
    let summary = if bare.is_empty() {
        "armour on everywhere".to_string()
    } else {
        format!("bare: {}", bare.join(", "))
    };
    let head = ui.add(
        egui::Label::new(
            egui::RichText::new(format!("{fold} Equipment ({summary})"))
                .color(egui::Color32::from_rgb(180, 230, 180))
                .small(),
        )
        .sense(egui::Sense::click()),
    );
    if head.clicked() {
        *folded = !*folded;
    }
    if *folded {
        return;
    }
    let name = |g: u32| {
        v.rows
            .iter()
            .find(|r| r.item.guid == g)
            .map_or_else(|| format!("{g:#010x}"), |r| r.item.name.clone())
    };
    for (title, slots) in GROUPS {
        caption(ui, title);
        egui::Grid::new(("equipment", title))
            .num_columns(2)
            .spacing([8.0, 1.0])
            .show(ui, |ui| {
                for s in slots {
                    ui.label(egui::RichText::new(s.name).small());
                    let here = worn_in(&v.worn, s.mask);
                    let frame = egui::Frame::new().inner_margin(1);
                    let (_, dropped) = ui.dnd_drop_zone::<ItemDrag, _>(frame, |ui| {
                        if here.is_empty() {
                            let empty = if s.mask & e::COVERAGE != 0 && title == "Armour" {
                                egui::Color32::from_rgb(230, 140, 120)
                            } else {
                                egui::Color32::GRAY
                            };
                            ui.label(egui::RichText::new("empty").small().color(empty));
                        }
                        for g in &here {
                            let r = ui.add(
                                egui::Label::new(egui::RichText::new(name(*g)).small())
                                    .sense(egui::Sense::click_and_drag()),
                            );
                            if r.drag_started() {
                                r.dnd_set_drag_payload(ItemDrag(*g));
                            }
                            // Double-click takes it off, as on the list.
                            if r.double_clicked() {
                                actions.activated.push(*g);
                            }
                        }
                    });
                    if let Some(p) = dropped {
                        let valid = v
                            .rows
                            .iter()
                            .find(|r| r.item.guid == p.0)
                            .map_or(0, |r| r.stats.valid_locations);
                        if let Some(at) = wear_location(valid, s.mask) {
                            actions.wear.push((p.0, at));
                        }
                    }
                    ui.end_row();
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dropped_item_goes_where_it_can_and_covers_all_it_covers() {
        // A ring dropped on the left finger goes on the left one.
        let ring = e::FINGER_LEFT | e::FINGER_RIGHT;
        assert_eq!(wear_location(ring, e::FINGER_LEFT), Some(e::FINGER_LEFT));
        // A coat dropped on its chest takes its arms too.
        let coat = e::CHEST_ARMOR | e::UPPER_ARM_ARMOR | e::LOWER_ARM_ARMOR;
        assert_eq!(wear_location(coat, e::CHEST_ARMOR), Some(coat));
        // A sword on the weapon slot is a sword; on the feet, nothing.
        assert_eq!(
            wear_location(e::MELEE_WEAPON, HANDS[0].mask),
            Some(e::MELEE_WEAPON)
        );
        assert_eq!(wear_location(e::MELEE_WEAPON, e::FOOT_WEAR), None);
    }

    #[test]
    fn the_armour_left_bare_is_named() {
        let coat = e::CHEST_ARMOR | e::UPPER_ARM_ARMOR | e::LOWER_ARM_ARMOR;
        let shirt = e::CHEST_WEAR | e::UPPER_ARM_WEAR;
        let worn = [(1, coat), (2, shirt), (3, e::HEAD_WEAR)];
        assert_eq!(
            bare(&worn),
            vec!["Abdomen", "Hands", "Upper legs", "Lower legs", "Feet"]
        );
    }
}
