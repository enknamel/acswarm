use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::shuts::DEAL_WINDOW;
#[cfg(doc)]
use super::shuts::{judge_shut, StandBy};
#[cfg(doc)]
use super::view::{Mate, TeamView};
#[cfg(doc)]
use crate::autoplay::called_to;
use crate::autoplay::{Autoplay, Room};
use crate::Client;

/// How long a mate's claim on a body is believed (see
/// [`TeamView::working`]). A claimant that stalled or was dragged into
/// a fight must not hold a body for the whole five minutes it lies
/// there.
///
/// Longer than the loot rules keep at one body (`ac_loot::run::KEEP_AT_IT`,
/// forty-five seconds), and by enough to cover a walk back to one that
/// drifted out of reach. At twenty seconds a character working a body
/// full of loot lost its claim while the body was still open on the
/// server, and the other eight converged on a container the server had
/// already handed out and were refused one by one.
pub(crate) const CLAIM_STALE: Duration = Duration::from_secs(75);

/// Two claims made within this of each other are the same moment.
///
/// Each session ages its own claim on its own clock and hears the
/// others' up to a board round late (`ac_plugin::team::SAY_EVERY`, half
/// a second), so nothing finer can be told apart and two sessions asked
/// which of them claimed a body first would both answer "I did". Inside
/// this window the lower player guid settles it instead, which is an
/// answer both of them reach (see [`TeamView::outranks_our_claim`]).
pub(crate) const SAME_MOMENT: Duration = Duration::from_millis(1500);

/// How long a newly fallen body is settled by the tie-break rather than
/// by the claims (see [`TeamView::opens_first`]).
///
/// A claim is said every half second (`ac_plugin::team::SAY_EVERY`) and
/// heard a frame later; a body is chosen within a tick of falling. A
/// second covers the round trip, and after it the claims are the truth.
pub(crate) const CLAIM_SETTLE: Duration = Duration::from_secs(1);

/// What a character that shut a body as emptied says about it to the
/// others, each by player guid (see [`judge_shut`]).
///
/// Every body shut as emptied is said, whomever it is done for, so that
/// the others know who has shut it: nothing on it is sent back to one of
/// those (see `Autoplay::may_be_sent`), and going back to it is no turn.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shut {
    /// The body.
    pub body: u32,
    /// Which of the shutter's shuts this is, counting up from its start.
    /// The same shut is said on every row for a while; a second shut of
    /// the same body, once it has been opened again, is a new word.
    pub n: u32,
    /// Those that would take nothing still on it, read by their own skills
    /// with the shared rules: they write it off for good.
    pub done_for: Vec<u32>,
    /// Those that would take only what is left on it for one of
    /// `left_for`: they leave it to those first (see [`StandBy`]).
    pub stand_by: Vec<u32>,
    /// Those something on it was left for in particular, the best at what
    /// a rule asks (see [`called_to`]).
    pub left_for: Vec<u32>,
}

/// A character as the turns at a newly fallen body read it (see
/// [`TeamView::opens_first`]): each mate from its row on the board, and
/// this character from the same word about itself, so that every session
/// reaches the same answer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Turn {
    pub guid: u32,
    pub world: glam::Vec3,
    /// It is at another body, or on its way to one.
    pub looting: bool,
    /// It is fighting something.
    pub fighting: bool,
    /// Its pack is neither full nor laden (see `logistics::Supplies`).
    pub room: bool,
    /// How many bodies it opened first lately ([`Mate::opened_first`]).
    pub opened: u16,
}

/// Where the character `who` comes in the deal for the body `body`, among
/// characters that have had as many turns lately (see
/// [`TeamView::opens_first`]).
///
/// Every session works the same number out of the same two guids, so the
/// order is agreed without a word said; and it is a different order for
/// every body, so bodies falling together go to different characters. The
/// mix is written out by hand (splitmix64's finish) rather than taken from
/// a std hasher, which may change from build to build and is seeded afresh
/// in every process.
pub fn deal(body: u32, who: u32) -> u64 {
    let mut z = ((u64::from(body) << 32) | u64::from(who)).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Autoplay {
    /// This character as the turns at a newly fallen body read it (see
    /// [`TeamView::opens_first`]), `me` being its player guid: all of it as
    /// it last said about itself on the board (`TeamView::me`), which is
    /// what the others read it by. Read as it is now instead, where it
    /// stands, whether it is at another body and how many turns it has had
    /// ran up to half a second ahead of the row the others had: a session
    /// that had just shut a body dealt the next to someone else while the
    /// rest dealt it to that session, and nobody opened it for a second.
    ///
    /// With nothing said yet (`mine` being where it stands), it is read as
    /// it is now.
    pub(crate) fn my_turn(&self, me: u32, mine: glam::Vec3, now: Instant) -> Turn {
        match self.team.me.as_ref() {
            Some(said) => Turn {
                guid: me,
                world: said.world,
                looting: said.looting.is_some(),
                fighting: said.target.is_some(),
                // Not opening bodies at all, it is dealt none.
                room: said.turn().is_some_and(|t| t.room),
                opened: said.opened_first,
            },
            None => Turn {
                guid: me,
                world: mine,
                looting: self.corpse_claim(now).is_some(),
                fighting: false,
                room: true,
                opened: self.opened_first(now),
            },
        }
    }

    /// The name of the one of the others whose turn the body `guid`, lying
    /// at `at`, is, when that turn is what keeps this character (`me`,
    /// standing at `mine`) off it (see [`Autoplay::ours_to_open`]).
    pub(crate) fn whose_turn(
        &self,
        guid: u32,
        at: glam::Vec3,
        me: u32,
        mine: glam::Vec3,
        now: Instant,
    ) -> Option<&str> {
        let settled =
            now.saturating_duration_since(self.corpse_first_seen(guid, now)) >= CLAIM_SETTLE;
        if settled
            || self.team.mates.is_empty()
            || self.corpse_claim(now).is_some_and(|(g, _)| g == guid)
            || self.team.working(guid)
        {
            return None;
        }
        let dealt = self
            .body_dealt_to(guid, now)
            .unwrap_or_else(|| self.team.opens_first(guid, at, self.my_turn(me, mine, now)));
        self.team
            .mates
            .iter()
            .find(|m| m.guid == dealt)
            .map(|m| m.name.as_str())
    }

    /// How many bodies this character opened first lately, within
    /// `DEAL_WINDOW`: its turns, as it says them on the board (see
    /// [`Mate::opened_first`]).
    pub fn opened_first(&self, now: Instant) -> u16 {
        let lately = self
            .first_opens
            .iter()
            .filter(|t| now.saturating_duration_since(**t) < DEAL_WINDOW)
            .count();
        u16::try_from(lately).unwrap_or(u16::MAX)
    }
}

impl Client {
    /// A body this character is standing off from only because it is one
    /// of the others' turn at it, by name, and whose turn (see
    /// [`Autoplay::whose_turn`]).
    pub(crate) fn turn_of_another(
        &self,
        me: glam::Vec3,
        now: Instant,
        room: Room,
    ) -> Option<(String, String)> {
        let my_guid = self.world.player_guid.unwrap_or(0);
        self.world
            .objects
            .values()
            .filter(|o| o.object_desc_flags & ac_world::object_desc_flags::CORPSE != 0)
            .find_map(|o| {
                let at = o.world_pos()?;
                if !self.autoplay.corpse_owed(o.guid, at, me, now, room)
                    || self.corpse_is_someone_elses(&o.name)
                {
                    return None;
                }
                let who = self.autoplay.whose_turn(o.guid, at, my_guid, me, now)?;
                Some((o.name.clone(), who.to_string()))
            })
    }
}

#[cfg(test)]
mod tests;
