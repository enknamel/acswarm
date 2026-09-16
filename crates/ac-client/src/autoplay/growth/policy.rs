use std::time::Instant;

use super::sale::worth_a_sale_run;
use super::Growth;
use crate::logistics::{self, Stage, Supplies};
use crate::Client;

/// Whether the party's mode decides when this character goes to town,
/// rather than its own pack and supplies. `mates` is how many others are
/// on the team as it was last heard.
///
/// Restocking together needs somebody to restock with. Blargerton had the
/// team rules on, restocking together on, and nobody else on the team: a
/// party of one only ever decided to go for supplies or a pack with no
/// slot, never for weight, so he hunted on carrying all he meant to and
/// left everything else on the bodies.
pub(super) fn restocks_as_a_party(team: &crate::autoplay::Team, mates: usize) -> bool {
    team.enabled && team.restock.together && mates > 0
}

impl Client {
    /// Work out what the party is doing and remember it.
    ///
    /// Every session runs this on the same roster and reaches the same
    /// answer, so there is nothing to agree on: the party changes mode
    /// together without a message being sent about it. A character
    /// playing alone -- nobody else on the team, whatever its settings
    /// say -- or one whose team rules are off, is always hunting: its
    /// own pack and supplies send it to town, by the older rule (see
    /// [`restocks_as_a_party`]).
    pub(super) fn grow_mode(&mut self, now: Instant, cfg: &Growth) -> crate::logistics::GroupMode {
        use crate::logistics::{decide, GroupMode};
        let team = &self.autoplay.config.team;
        if !restocks_as_a_party(team, self.autoplay.team.mates.len()) {
            self.autoplay.growth.mode = GroupMode::Hunting;
            return GroupMode::Hunting;
        }
        let policy = team.restock.clone();
        let mut party: Vec<Supplies> = self
            .autoplay
            .team
            .mates
            .iter()
            .map(|m| m.supplies.clone())
            .collect();
        party.push(self.supplies(cfg, now));
        let was = self.autoplay.growth.mode;
        // A trip that has dragged on has failed at something no rule
        // here can see: a vendor out of tapers, a purse that ran dry, a
        // character that died on the way. Waiting at the hunting ground
        // for ever is worse than hunting undersupplied, so the party
        // gives up and goes back to it.
        let stalled = !was.hunting()
            && policy.give_up_after > 0.0
            && self
                .autoplay
                .growth
                .mode_since
                .is_some_and(|t| now.duration_since(t).as_secs_f32() > policy.give_up_after);
        if stalled {
            let st = &mut self.autoplay.growth;
            st.mode = GroupMode::Hunting;
            st.mode_since = Some(now);
            st.mode_because = "the trip to town took too long".to_string();
            st.handed_over = false;
            st.round = 0;
            self.autoplay.note(
                "giving up on the trip to town and going back to hunting",
                now,
            );
            return GroupMode::Hunting;
        }
        if let Some(switch) = decide(was, &party, &policy, self.autoplay.growth.round) {
            let st = &mut self.autoplay.growth;
            // Back to the start of a trip is another round; going home
            // starts the count again.
            st.round = match switch.mode {
                GroupMode::Hunting => 0,
                GroupMode::Restocking(Stage::HandOver) if !was.hunting() => st.round + 1,
                GroupMode::Restocking(_) => st.round,
            };
            st.mode = switch.mode;
            st.mode_since = Some(now);
            st.mode_because = switch.because.clone();
            // A new stage is a new set of errands; nothing carries over.
            st.handed_over = false;
            self.autoplay.note(
                format!("the party is {}: {}", switch.mode, switch.because),
                now,
            );
        }
        self.autoplay.growth.mode
    }

    /// What this character says about itself for the party to decide
    /// with: how close to empty it is, what it still has to buy, and
    /// what that will cost.
    ///
    /// The level is the worst supply line, not the average. A mage with
    /// a full load of scarabs and no tapers cannot cast, and averaging
    /// the two would hide that.
    pub fn supplies(&self, cfg: &Growth, now: Instant) -> Supplies {
        let needs = self.grow_needs(cfg);
        // Loot for a counter worth a trip by this character's own rules
        // (see [`worth_a_sale_run`]), said to the party as one word.
        let salables = self.salables(cfg);
        let carried_for = self
            .autoplay
            .growth
            .sale_since
            .map(|t| now.duration_since(t));
        let sale = worth_a_sale_run(&salables, carried_for, cfg).is_some();
        // The worst line the party can do anything about. A Void mage
        // is permanently out of Nightshade -- no counter in Dereth
        // sells it -- and counting that would hold the level at nought
        // for ever, which reads as "always restocking, never stocked".
        let level = needs
            .iter()
            .filter(|n| n.buyable)
            .map(|n| logistics::line_level(n.have, n.keep))
            .fold(1.0f32, f32::min);
        let policy = self.autoplay.config.team.restock.sane();
        Supplies {
            name: self
                .world
                .player()
                .map(|p| p.name.clone())
                .unwrap_or_default(),
            level,
            pack_full: self.pack_full(),
            laden: self.laden(cfg),
            // Ready to go back: stocked up, and not still mid-errand.
            stocked: level >= policy.full_at && self.autoplay.growth.run.is_none(),
            handed_over: self.autoplay.growth.handed_over,
            holding_orders: self.holding_orders(),
            bill: needs.iter().map(|n| self.rough_cost(n)).sum(),
            purse: self.spendable(),
            // Short of something the trip cannot supply: either nothing
            // left to pay with, or a trip that came back with nothing,
            // which says the same thing about this town. Either way the
            // character is done shopping and goes back to earning.
            broke: !needs.is_empty()
                && (self.spendable() == 0 || self.autoplay.growth.run_was_futile),
            // What it can be handed: a gift is created in the pack by
            // the server, which spills into the side packs.
            free_space: self.room_anywhere(),
            sale,
            order: needs
                .iter()
                .filter(|n| n.want > 0 && n.buyable)
                .map(|n| (n.name.clone(), n.want))
                .collect(),
        }
    }

    /// The party as everyone has last described itself, this character
    /// included. What every shared decision is worked out from.
    pub fn party_supplies(&self, cfg: &Growth, now: Instant) -> Vec<Supplies> {
        let mut party: Vec<Supplies> = self
            .autoplay
            .team
            .mates
            .iter()
            .map(|m| m.supplies.clone())
            .collect();
        party.push(self.supplies(cfg, now));
        party
    }
}

#[cfg(test)]
mod tests;
