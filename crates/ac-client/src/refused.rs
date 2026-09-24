//! The one place the server's words are read for a refusal.
//!
//! Every line of chat the server sends on its own account -- a
//! transient string, a Broadcast line, a weenie error put into words
//! -- comes through [`Client::hear_refusal`], is matched once against
//! the table in `ac_agent::refusals`, and is handed to whichever system
//! made the request it answers. Those systems never see the English:
//! they are told what was refused, by name, and why, and act on the
//! decision the table makes for that kind.
//!
//! A refusal of something no system here is waiting on is still read:
//! it goes to the log as a refusal, so a run can be grepped for what
//! the server would not do, and the row is there to be wired to when
//! something starts waiting on it. What is not in the table at all is
//! listed in `docs/agent.md`, "What the server says in words".

use std::time::Instant;

use crate::refusals::{refused, Refusal};
use crate::Client;

impl Client {
    /// A line from the server. Read against the table of refusals and,
    /// when it is one, acted on by the system that was waiting for the
    /// answer.
    pub(crate) fn hear_refusal(&mut self, text: &str, now: Instant) {
        let Some(refusal) = refused(text) else {
            return;
        };
        match refusal {
            // A body that will not open: whether someone is in it, it is
            // not ours yet, or it never will be, is a different wait
            // (see `autoplay::Autoplay::corpse_refused`).
            Refusal::Open { name, why } => self.hear_corpse_refusal(name, why, now),
            // A take the pack it named had no room for: the refusal
            // that follows carries no reason, and these words are the
            // reason (see `autoplay::Client::hear_put_refusal`).
            Refusal::Put { item, .. } => self.hear_put_refusal(item),
            // An invitation into the fellowship that came to nothing:
            // the mate named is held off for the table's wait (see
            // `autoplay::Autoplay::hear_recruit_refusal`).
            Refusal::Recruit { name, why } => {
                self.autoplay.hear_recruit_refusal(name, why, now);
            }
            // A target the server will not let us fight: given up at
            // once, rather than when the stall clock runs out (see
            // `autoplay::Client::hear_attack_refusal`).
            Refusal::Attack { name } => self.hear_attack_refusal(name, now),
            // An essence turned away: for good, or for the creature
            // still out (see `summoning::Client::hear_summoning`).
            Refusal::Summon { why } => self.hear_summoning(why, text, now),
            // A purchase the counter turned down leaves the pack as it
            // was, so the rules are told, or they ask again at once
            // (see `ac_vendor::Run::buy_refused`).
            Refusal::Buy { .. } => {
                tracing::info!("the server refused ({refusal:?}): {text}");
                let answer = crate::refusals::answer(&refusal);
                self.autoplay.growth.shop.buy_refused(answer, now);
            }
            // Read, and waited on by nothing here yet. The item stays
            // where it lies and the loot rules pass it over
            // (`Carry`, `PickUpFirst`); a sale is judged by the pack
            // afterwards (`Sell`); a cast or a use the server will not
            // make is not asked of it on a clock (`Cast`, `Use`,
            // `UseWith`).
            Refusal::Carry
            | Refusal::PickUpFirst { .. }
            | Refusal::Cast { .. }
            | Refusal::Use { .. }
            | Refusal::UseWith { .. }
            | Refusal::Sell { .. } => {
                tracing::info!("the server refused ({refusal:?}): {text}");
            }
        }
    }
}
