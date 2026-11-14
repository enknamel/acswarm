//! A mule: a character on another account that only holds what it is given. Out of the Academy
//! it buys its packs (a town run on its own profile's one buy line), lets players give it items
//! and stands where it is; no row that salvages, gives away, drains or drops runs for it
//! (`run_steps`), only the reflexes in `MULE_REFLEXES`, and it offers nothing for sale.

use std::time::Instant;

use super::Doing;
use crate::Client;

/// The reflexes a holding mule keeps: staying alive and getting back from a death.
pub(crate) const MULE_REFLEXES: [&str; 2] = ["survive", "recover"];

impl Client {
    /// A mule out of the Academy: only `MULE_REFLEXES` and the hold run.
    pub fn is_mule_holding(&self) -> bool {
        self.autoplay.config.mule
            && !self.autoplay.academy.active
            && self
                .player
                .as_ref()
                .is_some_and(|p| p.landblock() != crate::academy::LANDBLOCK)
    }

    /// A holding mule standing where it is: somewhere to be met, not off buying its packs.
    pub fn is_mule_parked(&self) -> bool {
        self.is_mule_holding() && !self.autoplay.growth.town_run_under_way()
    }

    /// The mule's own tick: its packs bought, gifts let in (ACE refuses one with the option off,
    /// Player_Inventory.cs:3306), and standing where it is.
    pub(crate) fn autoplay_mule_hold(&mut self, now: Instant) -> bool {
        if self.autoplay_town_run(now) {
            return true;
        }
        if let Some(o) = crate::options::option_by_name("let other players give you items") {
            if !self.option_enabled(o) {
                self.set_option(o, true);
            }
        }
        let held = self.world.inventory().count();
        let room = self.room_anywhere();
        self.autoplay.say(
            Doing::Idle,
            format!("holding {held} item(s), room for {room}"),
        );
        true
    }
}

#[cfg(test)]
mod tests;
