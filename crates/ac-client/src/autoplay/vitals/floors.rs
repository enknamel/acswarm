//! The health, stamina and mana sliders (`Survive`) as the line no optional cast starts under: an
//! attack, debuff or buff waits while a bar is under its slider and the heal, Revitalize or mana
//! round restores it. What a cast leaves is not weighed; the restores go whatever the bars.

use std::time::Instant;

use crate::Client;

impl Client {
    /// Each vital as a fraction of its most (0 health, 1 stamina, 2 mana), 1.0 when not known.
    pub(crate) fn bars(&self) -> [f32; 3] {
        [0, 1, 2].map(|i| match self.vital_now(i) {
            (_, 0) => 1.0,
            (have, most) => have as f32 / most as f32,
        })
    }

    /// The first bar under its slider ("health", "stamina", "mana"), if any: no optional cast
    /// starts then. A cast that ends under one is fine (an_attack_that_ends_under_the_mana_slider_still_goes).
    pub(crate) fn bar_under_slider(&self) -> Option<&'static str> {
        let cfg = &self.autoplay.config.survive;
        let sliders = [cfg.heal_below, cfg.stamina_below, cfg.mana_below];
        let bars = self.bars();
        (0..3)
            .find(|&i| sliders[i] > 0.0 && bars[i] < sliders[i])
            .map(|i| ["health", "stamina", "mana"][i])
    }

    /// Whether the fight's next cast waits for a bar under its slider: Revitalize or the mana
    /// round goes first ([`Self::autoplay_vitals`]; heals run ahead of every row), else it waits.
    pub(crate) fn hold_for_vitals(&mut self, now: Instant) -> bool {
        let Some(bar) = self.bar_under_slider() else {
            return false;
        };
        if self.autoplay_vitals(now) {
            return true;
        }
        let said = format!("{bar} under its slider: holding the attack till it is back");
        self.autoplay.note(said, now);
        true
    }
}

#[cfg(test)]
mod tests;
