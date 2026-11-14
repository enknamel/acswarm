//! Mules. The give row: items tagged Give under a rule naming a player go to that player while it
//! stands in view and in a give's reach, between fights; a refusal holds the items back for the
//! ledger's wait (`Ledger::failed`). Loading a mule a host logged in (`load`), and a mule's own hold.

use std::time::Instant;

use super::Doing;
use crate::giving::GIVE_REACH;
use crate::profile::GiveTo;
use crate::Client;

mod hold;
mod load;

pub(crate) use hold::MULE_REFLEXES;
pub(crate) use load::Muling;
pub use load::{Delivery, Meet};

impl Client {
    /// Hand Give-tagged items to the player their rule names, when in view and in reach; true
    /// while gives of autoplay's own are under way or one was started.
    pub(crate) fn autoplay_give(&mut self, _now: Instant) -> bool {
        if let Some(g) = self.giving.as_ref() {
            // Waits on its own gives to a player; a player's give-all walks, autoplay's never.
            return !g.by_hand
                && self
                    .world
                    .objects
                    .get(&g.to)
                    .is_some_and(|o| o.is_character());
        }
        if self.attack_target.is_some() || self.under_attack() || self.autoplay.corpse.is_some() {
            return false;
        }
        let gifts = self.gifts_tagged(|t| matches!(t, GiveTo::Player(_)));
        let Some(me) = self.my_position() else {
            return false;
        };
        for (_, to) in &gifts {
            let GiveTo::Player(name) = to else {
                continue;
            };
            let Some(mule) = self.mule_in_reach(name, me) else {
                continue;
            };
            let items: Vec<u32> = gifts
                .iter()
                .filter(|(_, t)| t == to)
                .map(|(g, _)| *g)
                .collect();
            let n = items.len();
            if self.give_items(mule, items) {
                let said = format!("handing {n} item(s) to {name}");
                self.autoplay.say(Doing::Helping, said);
                return true;
            }
        }
        false
    }

    /// The player called `name` (a leading `+` either side ignored), in view within a give's reach.
    pub(crate) fn mule_in_reach(&self, name: &str, me: glam::Vec3) -> Option<u32> {
        let bare = |n: &str| n.trim().trim_start_matches('+').to_lowercase();
        let want = bare(name);
        self.world
            .objects
            .values()
            .filter(|o| o.is_character() && Some(o.guid) != self.world.player_guid)
            .filter(|o| bare(&o.name) == want)
            .find(|o| o.world_pos().is_some_and(|p| p.distance(me) <= GIVE_REACH))
            .map(|o| o.guid)
    }
}

#[cfg(test)]
mod tests;
