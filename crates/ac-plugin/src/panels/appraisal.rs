//! The appraisal window (key bindable from the menu): what the server told us about the last
//! object we assessed. It never opens by itself, because it used to
//! appear under the pointer on the first click of a double-click and
//! swallow the second; its key opens and closes it, and while it is open
//! it follows whatever is assessed next. The one-line summary always goes
//! to the chat log, and the inventory shows an item's stats on hover.
//! Items show value,
//! burden, workmanship and material, armor level with per-damage
//! protections, weapon damage, speed, skill and bonuses, spells with
//! spellcraft and mana, wield requirements, tinkering and the flags
//! (bonded, attuned, retained); creatures show level, vitals,
//! attributes and armor by location. Close hides it until the next
//! appraisal.

use super::{caption, title_bar, window, Source};
use crate::{egui, Client, Ctx, Plugin, Settings};
use ac_net::messages::Appraisal;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AppraisalView {
    pub name: String,
    /// (label, value) lines in display order.
    pub lines: Vec<(String, String)>,
    /// Free text: descriptions, use, inscription.
    pub texts: Vec<String>,
    /// Spell names on the item.
    pub spells: Vec<String>,
    pub failed: bool,
    /// The object appraised.
    pub guid: u32,
    /// Its inscription, when it may be written: inscribable, and carried or worn.
    pub inscription: Option<String>,
}

/// Imbue bits (ImbuedEffectType.cs) and their names.
const IMBUES: [(u32, &str); 17] = [
    (0x1, "Critical Strike"),
    (0x2, "Crippling Blow"),
    (0x4, "Armor Rending"),
    (0x8, "Slash Rending"),
    (0x10, "Pierce Rending"),
    (0x20, "Bludgeon Rending"),
    (0x40, "Acid Rending"),
    (0x80, "Cold Rending"),
    (0x100, "Electric Rending"),
    (0x200, "Fire Rending"),
    (0x400, "Melee Defense"),
    (0x800, "Missile Defense"),
    (0x1000, "Magic Defense"),
    (0x4000, "Nether Rending"),
    (0x2000_0000, "Ignores some magic projectile damage"),
    (0x4000_0000, "Always critical"),
    (0x8000_0000, "Ignores all armor"),
];

/// Gear rating ints (PropertyInt.cs 370-389) and their names.
const RATINGS: [(u32, &str); 14] = [
    (370, "Damage"),
    (371, "Damage resist"),
    (372, "Crit"),
    (373, "Crit resist"),
    (374, "Crit damage"),
    (375, "Crit damage resist"),
    (376, "Healing boost"),
    (377, "Nether resist"),
    (378, "Life resist"),
    (379, "Max health"),
    (383, "PK damage"),
    (384, "PK damage resist"),
    (388, "Overpower"),
    (389, "Overpower resist"),
];

/// `+5%` from an added fraction such as 0.05 (mana conversion).
fn percent_added(f: f64) -> String {
    format!("{:+}%", (f * 100.0).round() as i64)
}

/// The item stats past damage and armor: imbues, defenses, caster and crit mods, cleaving,
/// slayer (`creature_type` names it), ratings and item level, each only when appraised.
fn item_extras(
    a: &Appraisal,
    creature_type: &dyn Fn(u32) -> String,
    lines: &mut Vec<(String, String)>,
) {
    let mut push = |k: &str, v: String| lines.push((k.into(), v));
    let imbued = [179, 303, 304, 305, 306]
        .iter()
        .filter_map(|k| a.int(*k))
        .fold(0u32, |b, v| b | v as u32);
    let imbues: Vec<&str> = IMBUES
        .iter()
        .filter(|(b, _)| imbued & b != 0)
        .map(|(_, n)| *n)
        .collect();
    if !imbues.is_empty() {
        push("Imbued", imbues.join(", "));
    }
    let bonus = |k: u32| a.float(k).filter(|m| (m - 1.0).abs() > 1e-6);
    if let Some(m) = bonus(149) {
        push("Missile defense bonus", percent_bonus(m));
    }
    if let Some(m) = bonus(150) {
        push("Magic defense bonus", percent_bonus(m));
    }
    if let Some(f) = a.float(144).filter(|f| f.abs() > 1e-6) {
        push("Mana conversion", percent_added(f));
    }
    if let Some(m) = bonus(152) {
        push("Elemental damage", percent_bonus(m));
    }
    if let Some(w) = a.weapon.as_ref().filter(|w| w.max_velocity > 0.0) {
        if (w.damage_mod - 1.0).abs() > 1e-6 {
            push("Damage modifier", percent_bonus(w.damage_mod));
        }
    }
    if let Some(f) = a.float(147) {
        push("Critical chance", format!("{:.0}%", f * 100.0));
    }
    if let Some(m) = a.float(136) {
        push("Critical damage", format!("x{m:.2}"));
    }
    if let Some(f) = a.float(155) {
        push(
            "Armor cleaving",
            format!("ignores {:.0}% of armor", f * 100.0),
        );
    }
    if let (Some(m), Some(t)) = (a.float(157), a.int(263)) {
        let kind = damage_type_name(t as u32);
        push("Resistance cleaving", format!("{kind} x{m:.2}"));
    }
    if let Some(f) = a.float(159) {
        push("Absorbs magic damage", format!("{:.0}%", f * 100.0));
    }
    if let Some(n) = a.int(292).filter(|n| *n > 0) {
        push("Cleave", format!("{n} targets"));
    }
    if let Some(t) = a.int(166).filter(|t| *t > 0) {
        let name = creature_type(t as u32).replace('_', " ");
        push("Slayer", format!("{name} Slayer"));
    }
    let ratings: Vec<String> = RATINGS
        .iter()
        .filter_map(|(k, n)| Some((n, a.int(*k).filter(|v| *v != 0)?)))
        .map(|(n, v)| format!("{n} {v}"))
        .collect();
    if !ratings.is_empty() {
        push("Ratings", ratings.join(", "));
    }
    if let Some(max) = a.int(319).filter(|m| *m > 0) {
        let xp = a.int64s.iter().find(|(k, _)| *k == 4).map(|(_, x)| *x);
        let xp = xp.map(|x| format!(", {x} XP")).unwrap_or_default();
        push("Item level", format!("max {max}{xp}"));
    }
    if let Some(m) = bonus(100) {
        push("Healing bonus", percent_bonus(m));
    }
    if let Some(s) = a.float(167).filter(|s| *s > 0.0) {
        push("Cooldown", format!("{s:.0} s"));
    }
}

/// The damage type bits as words.
pub(crate) fn damage_type_name(bits: u32) -> String {
    let names = [
        (0x1, "Slashing"),
        (0x2, "Piercing"),
        (0x4, "Bludgeoning"),
        (0x8, "Cold"),
        (0x10, "Fire"),
        (0x20, "Acid"),
        (0x40, "Electric"),
        (0x400, "Nether"),
    ];
    let parts: Vec<&str> = names
        .iter()
        .filter(|(b, _)| bits & b != 0)
        .map(|(_, n)| *n)
        .collect();
    if parts.is_empty() {
        "?".into()
    } else {
        parts.join(", ")
    }
}

/// `+5%` / `-3%` from a multiplier such as 1.05.
pub(crate) fn percent_bonus(mult: f64) -> String {
    let pct = ((mult - 1.0) * 100.0).round() as i64;
    format!("{pct:+}%")
}

/// Build the lines from an appraisal; `skill_name`, `spell_name` and `creature_type` resolve
/// ids through the DAT tables.
pub(crate) fn build(
    name: &str,
    a: &Appraisal,
    material_name: &dyn Fn(u32) -> &'static str,
    skill_name: &dyn Fn(u32) -> String,
    spell_name: &dyn Fn(u32) -> String,
    creature_type: &dyn Fn(u32) -> String,
) -> AppraisalView {
    let mut lines: Vec<(String, String)> = Vec::new();
    let mut texts = Vec::new();
    for id in [15u32, 16, 14] {
        if let Some(t) = a.string(id) {
            if !t.is_empty() {
                texts.push(t.to_string());
            }
        }
    }
    if let Some(t) = a.string(7) {
        if !t.is_empty() {
            let by = a.string(8).unwrap_or("");
            texts.push(if by.is_empty() {
                format!("\"{t}\"")
            } else {
                format!("\"{t}\" -- {by}")
            });
        }
    }
    if let Some(c) = &a.creature {
        if let Some(l) = a.int(25) {
            lines.push(("Level".into(), l.to_string()));
        }
        lines.push(("Health".into(), format!("{} / {}", c.health, c.health_max)));
        if c.stamina_max > 0 || c.mana_max > 0 {
            lines.push((
                "Stamina".into(),
                format!("{} / {}", c.stamina, c.stamina_max),
            ));
            lines.push(("Mana".into(), format!("{} / {}", c.mana, c.mana_max)));
        }
        if let Some(at) = c.attributes {
            lines.push((
                "Attributes".into(),
                format!(
                    "Str {} End {} Qui {} Coo {} Foc {} Self {}",
                    at[0], at[1], at[2], at[3], at[4], at[5]
                ),
            ));
        }
        if let Some(l) = a.armor_levels {
            lines.push((
                "Armor".into(),
                format!(
                    "head {} chest {} abdomen {} arms {}/{} hands {} legs {}/{} feet {}",
                    l[0], l[1], l[2], l[3], l[4], l[5], l[6], l[7], l[8]
                ),
            ));
        }
    } else {
        if let Some(v) = a.int(19) {
            lines.push(("Value".into(), format!("{v} pyreals")));
        }
        if let Some(b) = a.int(5) {
            lines.push(("Burden".into(), format!("{b} bu")));
        }
        match (a.int(105), a.int(131)) {
            (Some(w), Some(m)) => lines.push((
                "Workmanship".into(),
                format!("{w} ({})", material_name(m as u32)),
            )),
            (Some(w), None) => lines.push(("Workmanship".into(), w.to_string())),
            (None, Some(m)) => lines.push(("Material".into(), material_name(m as u32).into())),
            _ => {}
        }
        if let Some(al) = a.int(28) {
            lines.push(("Armor level".into(), al.to_string()));
        }
        if let Some(p) = &a.armor {
            let al = a.int(28).unwrap_or(0) as f32;
            let show = |m: f32| format!("{:.0}", al * m);
            lines.push((
                "Protection".into(),
                format!(
                    "slash {} pierce {} bludgeon {} cold {} fire {} acid {} electric {} nether {}",
                    show(p.slash),
                    show(p.pierce),
                    show(p.bludgeon),
                    show(p.cold),
                    show(p.fire),
                    show(p.acid),
                    show(p.electric),
                    show(p.nether)
                ),
            ));
        }
        if let Some(w) = &a.weapon {
            let low = (w.damage as f64 * (1.0 - w.variance)).round() as u32;
            lines.push((
                "Damage".into(),
                format!("{low}-{} {}", w.damage, damage_type_name(w.damage_type)),
            ));
            lines.push(("Speed".into(), w.speed.to_string()));
            lines.push(("Skill".into(), skill_name(w.skill)));
            if (w.offense - 1.0).abs() > 1e-6 {
                lines.push(("Attack bonus".into(), percent_bonus(w.offense)));
            }
            if let Some(d) = a.float(29) {
                if (d - 1.0).abs() > 1e-6 {
                    lines.push(("Defense bonus".into(), percent_bonus(d)));
                }
            }
            if let Some(e) = a.int(204) {
                if e != 0 {
                    lines.push(("Elemental bonus".into(), e.to_string()));
                }
            }
            if w.max_velocity > 0.0 {
                lines.push(("Range".into(), format!("{:.0} m", w.max_velocity_estimated)));
            }
        }
        item_extras(a, creature_type, &mut lines);
        if let Some(s) = a.int(56) {
            lines.push(("Shield".into(), s.to_string()));
        }
        if let (Some(skill), Some(level)) = (a.int(159), a.int(160)) {
            lines.push((
                "Requires".into(),
                format!("{} {level}", skill_name(skill as u32)),
            ));
        }
        if let Some(c) = a.int(106) {
            lines.push(("Spellcraft".into(), c.to_string()));
        }
        if let Some((_, cost)) = a.int64s.iter().find(|(k, _)| *k == 3) {
            lines.push(("Augmentation cost".into(), format!("{cost} XP")));
        }
        if let (Some(cur), Some(max)) = (a.int(107), a.int(108)) {
            let rate = a
                .float(5)
                .map(|r| format!(" ({:.2}/s)", -r))
                .unwrap_or_default();
            lines.push(("Mana".into(), format!("{cur} / {max}{rate}")));
        }
        if let Some(d) = a.int(109) {
            lines.push(("Difficulty".into(), d.to_string()));
        }
        if let Some(t) = a.int(171) {
            if t > 0 {
                let by = a.string(39).unwrap_or("");
                lines.push((
                    "Tinkered".into(),
                    if by.is_empty() {
                        format!("{t} times")
                    } else {
                        format!("{t} times by {by}")
                    },
                ));
            }
        }
        if let (Some(s), Some(m)) = (a.int(92), a.int(91)) {
            lines.push(("Uses".into(), format!("{s} / {m}")));
        }
        let mut flags = Vec::new();
        if a.int(33).unwrap_or(0) != 0 {
            flags.push("bonded");
        }
        if a.int(114).unwrap_or(0) != 0 {
            flags.push("attuned");
        }
        if a.bool(91).unwrap_or(false) {
            flags.push("retained");
        }
        if a.bool(22).unwrap_or(false) {
            flags.push("inscribable");
        }
        if !flags.is_empty() {
            lines.push(("Flags".into(), flags.join(", ")));
        }
    }
    AppraisalView {
        name: name.to_string(),
        lines,
        texts,
        spells: a.spells.iter().map(|s| spell_name(*s)).collect(),
        failed: !a.success,
        guid: a.guid,
        inscription: None,
    }
}

pub(crate) fn view(c: &Client) -> Option<AppraisalView> {
    let guid = c.last_appraisal?;
    let a = c.appraisals.get(&guid)?;
    let name = c.world.name_or_hex(guid);
    let skills = c.assets.skill_table().ok();
    let spells = c.assets.spell_table().ok();
    let creatures = c.assets.creature_types().ok();
    let me = c.world.player_guid;
    let held = c.world.is_carried(guid)
        || c.world
            .objects
            .get(&guid)
            .is_some_and(|o| o.wielder.is_some() && o.wielder == me);
    let inscription =
        (held && a.bool(22).unwrap_or(false)).then(|| a.string(7).unwrap_or("").to_string());
    let mut v = build(
        &name,
        a,
        &ac_world::material::name,
        &|id| {
            skills
                .as_ref()
                .and_then(|t| t.get(id).map(|s| s.name.clone()))
                .unwrap_or_else(|| format!("skill {id}"))
        },
        &|id| {
            spells
                .as_ref()
                .and_then(|t| t.get(id).map(|s| s.name.clone()))
                .unwrap_or_else(|| format!("spell {id}"))
        },
        &|id| {
            creatures
                .as_ref()
                .and_then(|t| t.get(id).map(str::to_string))
                .unwrap_or_else(|| format!("creature type {id}"))
        },
    );
    v.inscription = inscription;
    Some(v)
}

/// What the window was asked this frame.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Drawn {
    /// Its title bar's close button, or Escape.
    pub closed: bool,
    /// Write was pressed: inscribe the draft.
    pub write: bool,
}

/// Draw the window; `draft` is the inscription being written, shown when `v` may be inscribed.
pub(crate) fn draw(egui: &egui::Context, v: &AppraisalView, draft: &mut String) -> Drawn {
    let mut drawn = Drawn::default();
    let w = egui.viewport_rect().width();
    window(
        "appraisal",
        egui::pos2(w - 720.0, 2.0 * super::radar::RADIUS + 300.0),
        egui::vec2(440.0, 280.0),
        190,
        8,
    )
    .show(egui, |ui| {
        ui.set_min_size(egui::vec2(424.0, 264.0));
        title_bar(ui, "appraisal", &v.name);
        if v.failed {
            caption(ui, "(appraisal failed)");
        }
        egui::ScrollArea::vertical()
            .max_height(230.0)
            .show(ui, |ui| {
                ui.set_min_width(410.0);
                for t in &v.texts {
                    ui.label(
                        egui::RichText::new(t)
                            .color(egui::Color32::from_rgb(220, 210, 180))
                            .italics(),
                    );
                }
                egui::Grid::new("appraisal_lines")
                    .num_columns(2)
                    .spacing([12.0, 2.0])
                    .show(ui, |ui| {
                        for (k, val) in &v.lines {
                            caption(ui, k);
                            ui.label(egui::RichText::new(val).color(egui::Color32::WHITE));
                            ui.end_row();
                        }
                    });
                if !v.spells.is_empty() {
                    caption(ui, "Spells");
                    for s in &v.spells {
                        ui.label(
                            egui::RichText::new(s).color(egui::Color32::from_rgb(160, 190, 255)),
                        );
                    }
                }
            });
        if v.inscription.is_some() {
            ui.horizontal(|ui| {
                caption(ui, "Inscription");
                ui.add(egui::TextEdit::singleline(draft).desired_width(280.0));
                if ui.button("Write").clicked() {
                    drawn.write = true;
                }
            });
        }
    });
    drawn.closed = super::closed("appraisal");
    drawn
}

#[derive(Default)]
pub(crate) struct AppraisalPanel {
    source: Source<AppraisalView>,
    /// Open (its key toggles it). Starts closed.
    pub show: bool,
    /// The inscription being written, and the object it is for.
    draft: (u32, String),
}

impl AppraisalPanel {
    pub(crate) fn demo() -> Self {
        AppraisalPanel {
            source: Source::Demo(AppraisalView {
                name: "Demo Dagger".into(),
                lines: vec![
                    ("Value".into(), "150 pyreals".into()),
                    ("Burden".into(), "90 bu".into()),
                    ("Workmanship".into(), "5 (Iron)".into()),
                    ("Damage".into(), "6-12 Piercing".into()),
                    ("Speed".into(), "20".into()),
                    ("Skill".into(), "Dagger".into()),
                    ("Attack bonus".into(), "+5%".into()),
                    ("Requires".into(), "Dagger 200".into()),
                    ("Spellcraft".into(), "150".into()),
                    ("Mana".into(), "300 / 300 (0.02/s)".into()),
                ],
                texts: vec!["A well-made dagger.".into()],
                spells: vec!["Blood Drinker III".into(), "Swift Killer III".into()],
                failed: false,
                guid: 1,
                inscription: Some(String::new()),
            }),
            show: true,
            draft: (1, String::new()),
        }
    }
}

impl Plugin for AppraisalPanel {
    fn name(&self) -> &str {
        "appraisal"
    }

    fn ui(&mut self, cx: &mut Ctx, egui: &egui::Context) {
        if let Some(ask) = super::take_open(cx.board, "appraisal") {
            self.show = ask.apply(self.show);
        }
        if !self.show {
            return;
        }
        let v = match &self.source {
            Source::Demo(d) => Some(d.clone()),
            Source::Live => {
                let Some(c) = cx.try_client() else { return };
                view(c)
            }
        };
        let Some(v) = v else { return };
        if self.draft.0 != v.guid {
            self.draft = (v.guid, v.inscription.clone().unwrap_or_default());
        }
        let drawn = draw(egui, &v, &mut self.draft.1);
        if drawn.write {
            if let (Source::Live, Some(c)) = (&self.source, cx.try_client()) {
                c.inscribe(v.guid, &self.draft.1);
            }
        }
        if drawn.closed || super::closed("appraisal") {
            self.show = false;
        }
    }

    fn key(&mut self, _cx: &mut Ctx, key: egui::Key, pressed: bool) -> bool {
        if pressed && crate::keys::bound("appraisal", key) {
            self.show = !self.show;
            return true;
        }
        false
    }

    fn load(&mut self, settings: &Settings) {
        if let Some(v) = settings.get("appraisal.show") {
            self.show = v;
        }
    }

    fn save(&self, settings: &mut Settings) {
        settings.set("appraisal.show", self.show);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imbues_defenses_caster_mods_and_ratings_are_shown() {
        let a = Appraisal {
            guid: 1,
            success: true,
            ints: vec![
                (179, 0x4 | 0x400),
                (292, 2),
                (370, 2),
                (372, 1),
                (319, 50),
                (166, 24),
            ],
            floats: vec![
                (144, 0.05),
                (149, 1.1),
                (150, 1.08),
                (152, 1.04),
                (147, 0.15),
            ],
            int64s: vec![(4, 1000)],
            ..Default::default()
        };
        let v = build(
            "Wand",
            &a,
            &ac_world::material::name,
            &|id| format!("skill {id}"),
            &|id| format!("spell {id}"),
            &|id| format!("type{id}_name"),
        );
        let line = |k: &str| {
            v.lines
                .iter()
                .find(|(l, _)| l == k)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(line("Imbued"), Some("Armor Rending, Melee Defense"));
        assert_eq!(line("Mana conversion"), Some("+5%"));
        assert_eq!(line("Missile defense bonus"), Some("+10%"));
        assert_eq!(line("Magic defense bonus"), Some("+8%"));
        assert_eq!(line("Elemental damage"), Some("+4%"));
        assert_eq!(line("Critical chance"), Some("15%"));
        assert_eq!(line("Cleave"), Some("2 targets"));
        assert_eq!(line("Ratings"), Some("Damage 2, Crit 1"));
        assert_eq!(line("Item level"), Some("max 50, 1000 XP"));
        assert_eq!(line("Slayer"), Some("type24 name Slayer"));
        assert_eq!(
            v.inscription, None,
            "offered only once the client says it is held"
        );
    }

    #[test]
    fn weapon_lines() {
        let a = Appraisal {
            guid: 1,
            success: true,
            ints: vec![
                (19, 150),
                (5, 90),
                (105, 5),
                (131, 0x3D),
                (159, 1),
                (160, 200),
            ],
            floats: vec![(29, 1.03)],
            strings: vec![(15, "A dagger.".into())],
            weapon: Some(ac_net::messages::WeaponProfile {
                damage_type: 2,
                speed: 20,
                skill: 1,
                damage: 12,
                variance: 0.5,
                damage_mod: 1.0,
                length: 0.3,
                max_velocity: 0.0,
                offense: 1.05,
                max_velocity_estimated: 0,
            }),
            spells: vec![2091],
            ..Default::default()
        };
        let v = build(
            "Dagger",
            &a,
            &ac_world::material::name,
            &|id| format!("skill {id}"),
            &|id| format!("spell {id}"),
            &|id| format!("type{id}_name"),
        );
        let get = |k: &str| v.lines.iter().find(|(a, _)| a == k).map(|(_, b)| b.clone());
        assert_eq!(get("Workmanship").as_deref(), Some("5 (Iron)"));
        assert_eq!(get("Damage").as_deref(), Some("6-12 Piercing"));
        assert_eq!(get("Attack bonus").as_deref(), Some("+5%"));
        assert_eq!(get("Defense bonus").as_deref(), Some("+3%"));
        assert_eq!(get("Requires").as_deref(), Some("skill 1 200"));
        assert_eq!(v.spells, vec!["spell 2091"]);
        assert_eq!(v.texts, vec!["A dagger."]);
        assert_eq!(damage_type_name(0x11), "Slashing, Fire");
    }
}
