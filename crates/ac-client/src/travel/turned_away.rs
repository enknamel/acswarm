//! The server turning the journey's portal away for a reason no wait cures: the mouth is left out
//! and the journey planned again at once, held or not (test: a_portal_that_turns_us_away_is_left_out).

use crate::Client;

/// Weenie errors a portal answers a use with, past waiting out (Portal.cs:161-256, codes from
/// WeenieError.cs). Teleported or PK-fought too recently (0x04A7, 0x04CC) pass, and are not here.
const TURNED_AWAY: [u32; 14] = [
    0x0446, // QuestSolvedTooLongAgo
    0x045C, // PKsMayNotUsePortal
    0x045D, // NonPKsMayNotUsePortal
    0x0474, // YouMustCompleteQuestToUsePortal
    0x04A8, // YouMustBeAnAdvocateToUsePortal
    0x04AA, // PlayersMayNotUsePortal
    0x04AB, // YouAreNotPowerfulEnoughToUsePortal
    0x04AC, // YouAreTooPowerfulToUsePortal
    0x04F3, // PKLiteMayNotUsePortal
    0x0554, // MustPurchaseThroneOfDestinyToUsePortal
    0x058F, // OnlyOlthoiMayUsePortal
    0x0590, // OlthoiMayNotUsePortal
    0x0591, // YouMayNotUsePortalWithVitae
    0x0592, // YouMustBeTwoWeeksOldToUsePortal
];

impl Client {
    /// Weenie error `code` came in: a portal turning us away leaves the journey's portal out. The
    /// wait at the mouth would too, but a hold resets its clock and never let it run out.
    pub(crate) fn portal_turned_away(&mut self, code: u32) {
        if !TURNED_AWAY.contains(&code) {
            return;
        }
        let Some((name, mouth)) = self.travel_portal() else {
            return;
        };
        if self.travel.refused.iter().any(|r| r.distance(mouth) < 1.0) {
            return;
        }
        tracing::warn!(
            "travel: the portal {name:?} turned us away ({code:#06x}); going another way"
        );
        self.travel.refused.push(mouth);
        self.travel.state.portal_since = None;
        self.travel.replan_due = true;
    }
}
