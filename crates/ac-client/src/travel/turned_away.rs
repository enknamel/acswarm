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
    /// The character as a portal's bitmask reads it, now: a quest that makes it a player killer
    /// opens those routes at the next plan. Never sought: no route makes one a player killer.
    pub(crate) fn traveller(&self) -> ac_world::portals::Traveller {
        ac_world::portals::Traveller {
            pk_status: self.pk_status(),
            vitae: self.world.stats.vitae() < 1.0,
        }
    }

    /// Our `PlayerKillerStatus`: the last update, else the description flags we came in with
    /// (ACE sends no update to a character that has always been NPK); 0 before either.
    pub(crate) fn pk_status(&self) -> u32 {
        use ac_world::object::pk_status;
        use ac_world::object_desc_flags as flags;
        let Some(me) = self.world.player() else {
            return 0;
        };
        if me.pk_status != 0 {
            return me.pk_status;
        }
        let f = me.object_desc_flags;
        if f & flags::PLAYER_KILLER != 0 {
            pk_status::PK
        } else if f & flags::PK_LITE_STATUS != 0 {
            pk_status::PK_LITE
        } else if f & flags::FREE_PK_STATUS != 0 {
            pk_status::FREE
        } else if f & flags::PLAYER != 0 {
            pk_status::NPK
        } else {
            0
        }
    }

    /// The portal whose mouth the character stands in: the nearest within [`super::ARRIVE`], in 3D,
    /// since the Facility Hub stacks its floors. What a portal's refusal is about, whatever the
    /// journey's step is: logged in standing in a quest portal, each of its refusals was laid on
    /// the next portal planned, and the Hub's own were left out one by one.
    pub(crate) fn portal_at_hand(&self) -> Option<&'static ac_world::portals::Portal> {
        let me = self.my_position()?;
        ac_world::portals::all()
            .iter()
            .filter(|p| p.from.distance(me) <= super::ARRIVE)
            .min_by(|a, b| a.from.distance(me).total_cmp(&b.from.distance(me)))
    }

    /// Weenie error `code` came in: the portal at hand turning us away is left out of every plan,
    /// and a journey that takes it is planned again. The wait at the mouth would too, but a hold
    /// resets its clock and never let it run out.
    pub(crate) fn portal_turned_away(&mut self, code: u32) {
        if !TURNED_AWAY.contains(&code) {
            return;
        }
        let Some(portal) = self.portal_at_hand() else {
            return;
        };
        let mouth = portal.from_xy();
        if !self.travel.refused.iter().any(|r| r.distance(mouth) < 1.0) {
            tracing::warn!(
                "travel: the portal {:?} turned us away ({code:#06x}); going another way",
                portal.name
            );
            self.travel.refused.push(mouth);
        }
        let taken = self.travel.trip.as_ref().is_some_and(|t| {
            t.steps.iter().any(|s| {
                matches!(s, ac_world::trip::Step::Portal { mouth: m, .. }
                    if m.at.truncate().distance(mouth) < 1.0)
            })
        });
        if taken {
            self.travel.state.portal_since = None;
            self.travel.replan_due = true;
        }
    }
}
