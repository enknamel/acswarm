//! A mule: a character on another account that only holds what it is given. Out of the Academy
//! it buys its packs (a town run on its own profile's one buy line), lets players give it items
//! and stands where it is; no row that salvages, gives away, drains or drops runs for it
//! (`run_steps`), only the reflexes in `MULE_REFLEXES`, and it offers nothing for sale. It gives
//! only a delivery its host set: those items, to that character.

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

    /// The mule's own tick: gifts let in (ACE refuses one with the option off,
    /// Player_Inventory.cs:3306), a delivery handed over, its packs bought, and standing still.
    pub(crate) fn autoplay_mule_hold(&mut self, now: Instant) -> bool {
        if let Some(o) = crate::options::option_by_name("let other players give you items") {
            if !self.option_enabled(o) {
                self.set_option(o, true);
            }
        }
        // A receiver on its way finds the mule where it said it was, not off buying packs.
        if self.deliver_from_mule() || self.autoplay_town_run(now) {
            return true;
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

impl Client {
    /// The delivery set: its items still carried handed to its receiver once in reach; done when
    /// none is left that the server has not refused.
    fn deliver_from_mule(&mut self) -> bool {
        let Some(d) = self.autoplay.muling.deliver.clone() else {
            return false;
        };
        if let Some(g) = self.giving.as_ref() {
            let said = format!("handing a delivery to {}", d.to);
            self.autoplay.say(Doing::Helping, said);
            return !g.by_hand;
        }
        let now = crate::holdings::unix_now();
        let items: Vec<u32> = d
            .items
            .iter()
            .copied()
            .filter(|g| self.world.is_carried(*g))
            .filter(|g| self.autoplay.ledger.why_failed(*g, now).is_none())
            .filter(|g| !self.is_give_refused(*g))
            .collect();
        if items.is_empty() {
            self.autoplay.muling.deliver = None;
            return false;
        }
        let Some(me) = self.my_position() else {
            return false;
        };
        let Some(to) = self.mule_in_reach(&d.to, me) else {
            let said = format!("holding {} item(s) for {}", items.len(), d.to);
            self.autoplay.say(Doing::Idle, said);
            return true;
        };
        let n = items.len();
        if self.give_items(to, items) {
            let said = format!("handing {n} item(s) to {}", d.to);
            self.autoplay.say(Doing::Helping, said);
        }
        true
    }
}

#[cfg(test)]
mod tests;
