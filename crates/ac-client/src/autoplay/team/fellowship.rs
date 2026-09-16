use std::time::{Duration, Instant};

use super::view::rival_leader;
use crate::autoplay::{Autoplay, Doing};
#[cfg(test)]
use crate::refusals::refused;
use crate::refusals::{self, RecruitRefusal, Refusal};
use crate::Client;

/// The character options a teammate keeps on, by the name the option
/// table knows them by (see `crate::options`). Every one of them is
/// something the server checks before it will let the party work as
/// one; the reasons are with the code that turns them on.
const TEAM_OPTIONS: [&str; 4] = [
    "accept fellowship",
    "automatically accept fellowship",
    "let other players give you items",
    "share fellowship loot",
];

/// The shortest gap between two fellowship invitations. Nine characters
/// arriving together are nine invitations, and they used to go out one
/// every five seconds -- forty-one seconds before the last one was in,
/// which is the very stretch in which nobody may loot anyone else's
/// kill. ACE has no rate limit on recruiting that I can find: the only
/// refusal is against a busy member (`Entity/Fellowship.cs`,
/// `fellow_busy_no_recruit`). That is read from the source and not
/// tested against a live server, so a small gap is kept rather than
/// firing nine invitations into one frame.
pub(crate) const RECRUIT_FLOOR: Duration = Duration::from_millis(500);

/// How long the leader waits for the server to answer a founding
/// before asking for one again. The answer is the fellowship itself,
/// arriving as a full update; until it comes there is nothing to say
/// whether the ask was heard, and asking twice would be two
/// fellowships.
pub(crate) const FOUNDING_WAIT: Duration = Duration::from_secs(5);

/// How far off a mate may stand and still be asked in, metres. The
/// server sets no distance on recruiting; this is about asking the ones
/// that are here rather than one still walking in from the last town.
const RECRUIT_RANGE: f32 = 25.0;

/// How long one invitee waits before being asked again. Nothing comes
/// back from an invitation that was turned down for a busy member, and
/// the news that one was accepted comes the long way round -- the mate
/// tells the board it is in a fellowship -- so a wait is the only way
/// to tell "not yet" from "never". It is the invitee that waits, not
/// the leader: the others are asked meanwhile.
pub(crate) const RECRUIT_AGAIN: Duration = Duration::from_secs(5);

/// How long an invitee the server turned down is left alone before it
/// is asked again: the table's wait for a refused recruit (see
/// `refusals::answer`). Until the server's words were read, two mates
/// were asked ten times each every [`RECRUIT_AGAIN`] and never came.
pub(crate) const HELD_OFF_FIRST: Duration = refusals::RECRUIT_HELD_OFF;

/// How long the team's rightful leader must be seen in a fellowship of
/// its own before this character gives up the one it founded. The
/// board's word on a mate is up to a round old and the world's on a
/// fellowship comes in its own time, so for a moment after that mate
/// quits this fellowship its row still says it is in one and the world
/// says it is not: a few rounds tell that from a second fellowship.
pub(crate) const YIELD_AFTER: Duration = Duration::from_secs(3);

/// How many a fellowship holds, the leader counted (ACE
/// `Entity/Fellowship.cs`, `MaxFellows`). A team of ten is one too
/// many, and the tenth is not asked.
const MAX_FELLOWS: usize = 9;

/// WeenieError for an invitation into a full fellowship (ACE
/// `WeenieError.YourFellowshipIsFull`).
pub(crate) const FELLOWSHIP_FULL: u32 = 0x041e;

/// Who to ask into the fellowship next, out of the mates standing by:
/// the nearest one that has not been asked in the last
/// [`RECRUIT_AGAIN`] and is not being held off after a refusal in words
/// (`held_off`, see [`HELD_OFF_FIRST`]). `None` when there is nobody to
/// ask, or when the last invitation went out inside [`RECRUIT_FLOOR`].
///
/// `waiting` is (guid, metres away), `asked` is when each invitee was
/// last sent an invitation.
fn next_invitee(
    waiting: &[(u32, f32)],
    asked: &[(u32, Instant)],
    held_off: &crate::did::Patience<u32>,
    last_sent: Option<Instant>,
    now: Instant,
) -> Option<u32> {
    if last_sent.is_some_and(|t| now.duration_since(t) < RECRUIT_FLOOR) {
        return None;
    }
    waiting
        .iter()
        .filter(|(guid, _)| {
            !held_off.held(guid, now)
                && !asked
                    .iter()
                    .any(|(g, t)| g == guid && now.duration_since(*t) < RECRUIT_AGAIN)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(guid, _)| *guid)
}

impl Autoplay {
    /// The server's words on an invitation that came to nothing (see
    /// `refusals::Refusal::Recruit`): the mate named is held off
    /// recruiting for the table's wait, and the log says so once. Only
    /// a mate this character has invited is read this way: "{Name} is
    /// busy." is also what the server says of a patron who cannot take
    /// an oath just now (ACE `Player_Allegiance.cs`).
    pub(crate) fn hear_recruit_refusal(&mut self, name: &str, why: RecruitRefusal, now: Instant) {
        let Some(guid) = self
            .team
            .mates
            .iter()
            .find(|m| m.name == name)
            .map(|m| m.guid)
            .filter(|g| self.asked_lately(*g, now))
        else {
            return;
        };
        self.refuse_recruit(guid, why, now);
    }

    /// [`Autoplay::hear_recruit_refusal`] from the server's own words,
    /// for the tests, which are written in them.
    #[cfg(test)]
    pub(crate) fn hear_recruit_words(&mut self, text: &str, now: Instant) {
        if let Some(Refusal::Recruit { name, why }) = refused(text) {
            self.hear_recruit_refusal(name, why, now);
        }
    }

    /// The server's code on an invitation that came to nothing: the
    /// fellowship is full (WeenieError 0x041E, ACE `Fellowship.cs`,
    /// `AddFellowshipMember`), which is about whoever was asked last.
    /// The recruiting stops by the count before this is ever heard;
    /// this is for a count the server disagrees with.
    pub(crate) fn hear_fellowship_full(&mut self, now: Instant) {
        let Some(guid) = self
            .recruited
            .iter()
            .filter(|(g, _)| self.asked_lately(*g, now))
            .max_by_key(|(_, t)| *t)
            .map(|(g, _)| *g)
        else {
            return;
        };
        self.refuse_recruit(guid, RecruitRefusal::Full, now);
    }

    /// Whether this mate was invited within the last [`RECRUIT_AGAIN`]:
    /// what makes a refusal the answer to that invitation. The list of
    /// the invited is pruned only on the next invitation, so without
    /// the age a "+X is busy." ten minutes later -- a patron who could
    /// not take an oath -- held X off the fellowship.
    fn asked_lately(&self, guid: u32, now: Instant) -> bool {
        self.recruited
            .iter()
            .any(|(g, t)| *g == guid && now.duration_since(*t) < RECRUIT_AGAIN)
    }

    /// Hold the mate off recruiting after a refusal, as the table
    /// decides (see `refusals::answer`), and say so once. "Busy" is
    /// over in the seconds a use or a cast takes, so it is the same
    /// short wait every time: doubling, a mage that happened to be
    /// casting at each of eight asks was left out for hours. The others
    /// double, since what they wait on is slower to change.
    fn refuse_recruit(&mut self, guid: u32, why: RecruitRefusal, now: Instant) {
        let name = self
            .team
            .mates
            .iter()
            .find(|m| m.guid == guid)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| format!("{guid:#010x}"));
        let said = match why {
            RecruitRefusal::AlreadyAMember => "is already in a fellowship",
            RecruitRefusal::Busy => "is busy",
            RecruitRefusal::NotAccepting => "is not accepting fellowship requests",
            RecruitRefusal::Declined => "declined",
            RecruitRefusal::Full => "would not fit: the fellowship is full",
        };
        refusals::answer(&Refusal::Recruit { name: &name, why }).hold(
            &mut self.held_off,
            guid,
            said,
            now,
        );
        let wait = self.held_off.waited(&guid).unwrap_or(HELD_OFF_FIRST);
        let why = said;
        self.note(
            format!(
                "{name} {why}: not asked into the fellowship again for {} s",
                wait.as_secs()
            ),
            now,
        );
    }
}

impl Client {
    /// The fellowship's members by player guid, while this character is
    /// one of them.
    pub(super) fn fellows(&self) -> Option<Vec<u32>> {
        let me = self.world.player_guid.unwrap_or(0);
        self.world
            .fellowship
            .as_ref()
            .map(|f| f.members.iter().map(|m| m.guid).collect::<Vec<u32>>())
            .filter(|f| f.contains(&me))
    }

    /// A fellowship invitation from anyone is accepted while on a team:
    /// the leader sends them, and the leader is trusted.
    pub(crate) fn autoplay_accept_invites(&mut self) {
        const FELLOWSHIP: u32 = 4;
        // The server asks the character before recruiting it only when
        // its options allow: with "accept fellowship requests" off the
        // leader's invitation is refused outright, and with "automatically
        // accept" on it never has to be answered. A teammate keeps both on,
        // and lets the others give it items: that is how salvage reaches
        // whoever salvages, and the server refuses a gift to anyone with
        // the option off (ACE `CharacterOptions1.AllowGive`).
        //
        // Loot sharing is the fourth, and it is the leader's option that
        // counts: ACE takes it off whoever founds the fellowship, once,
        // in the `Fellowship` constructor, and `Corpse.HasPermission`
        // lets a fellow open a fresh body through that clause alone.
        // With it off, nine characters hunting together told each other
        // "You do not yet have the right to loot" 278 times in ten
        // minutes -- only the killer could open its own kill, for the
        // first two minutes of the body's life. Every teammate keeps it
        // on, not just the one leading today, because the fellowship's
        // leader changes with whoever is about.
        for name in TEAM_OPTIONS {
            if let Some(o) = crate::options::option_by_name(name) {
                if !self.option_enabled(o) {
                    self.set_option(o, true);
                }
            }
        }
        let invites: Vec<(u32, u32)> = self
            .world
            .confirmations
            .iter()
            .filter(|c| c.kind == FELLOWSHIP)
            .map(|c| (c.kind, c.context))
            .collect();
        for (kind, context) in invites {
            self.confirm(kind, context, true);
        }
    }

    /// Whether this character's own options say it shares fellowship
    /// loot. It is the founder's answer to this, at the moment it
    /// founds, that decides whether the fellowship shares loot at all
    /// (ACE `Entity/Fellowship.cs`, the constructor).
    fn shares_fellowship_loot(&self) -> bool {
        crate::options::option_by_name("share fellowship loot")
            .is_some_and(|o| self.option_enabled(o))
    }

    /// The fellowship this character founded is given up when the team's
    /// rightful leader has one of its own (see [`rival_leader`]): the
    /// server refuses no founding (ACE `Player_Fellowship.cs`,
    /// `FellowshipCreate`), so two leaders make two fellowships, the
    /// fleet splits between them, and members of one have no right to
    /// the other's bodies. Disbanded, this one's members are free for
    /// the rightful leader to recruit, and this character with them.
    ///
    /// Only a fellowship this character founded itself, and still leads
    /// (`founded`, the same word that decides whether it can vouch for
    /// the loot sharing): one a person made by hand is never taken
    /// apart on the strength of a roster, as the note on a fellowship
    /// "not founded by me" already has it. True when it was given up.
    fn autoplay_yield_fellowship(&mut self, now: Instant) -> bool {
        let (Some(f), Some(me)) = (self.world.fellowship.as_ref(), self.world.player_guid) else {
            self.autoplay.outled_since = None;
            return false;
        };
        if self.autoplay.founded.is_none() || f.leader != me {
            self.autoplay.outled_since = None;
            return false;
        }
        let fname = f.name.clone();
        let members: Vec<u32> = f.members.iter().map(|m| m.guid).collect();
        let Some(rival) = rival_leader(&self.autoplay.team, &members).map(|m| m.name.clone())
        else {
            self.autoplay.outled_since = None;
            return false;
        };
        let since = *self.autoplay.outled_since.get_or_insert(now);
        if now.duration_since(since) < YIELD_AFTER {
            return false;
        }
        self.fellowship_quit(true);
        self.autoplay.founded = None;
        self.autoplay.outled_since = None;
        self.autoplay.say(
            Doing::Helping,
            format!("disbanding the fellowship {fname}: {rival} leads, and has one of its own"),
        );
        true
    }

    /// The leader founds the fellowship and brings the others into it,
    /// one invitation at a time. True when one went out.
    pub(crate) fn autoplay_fellowship(&mut self, now: Instant) -> bool {
        let team = self.autoplay.config.team.clone();
        if !team.enabled || !team.fellowship {
            return false;
        }
        if self.autoplay_yield_fellowship(now) {
            return true;
        }
        // Who gathers: the team's leader founds the fellowship, and
        // whoever leads a fellowship brings the team's mates into it --
        // the team's leader among them, when it stands outside. It does
        // stand outside: slow to post, it came on after a mate had
        // founded and gathered everyone else, and with nobody free to
        // found with it stayed out for the life of the run, since the
        // founder recruited only while it led the team. The same
        // follows the leader's own disconnect: the server quits it on
        // logout and hands the fellowship to whoever is left (ACE
        // `Fellowship.QuitFellowship`, `AssignNewLeader`), and the
        // leader comes back to a fellowship it is outside of.
        let leads_this = self
            .world
            .fellowship
            .as_ref()
            .is_some_and(|f| Some(f.leader) == self.world.player_guid);
        if !self.autoplay.team.leader && !leads_this {
            return false;
        }
        let Some(me) = self.player.as_ref().map(|p| p.world_position()) else {
            return false;
        };
        // Whoever is already in, first-hand: the server tells the leader
        // who has joined before the mate gets round to telling the board.
        let joined: Vec<u32> = self
            .world
            .fellowship
            .as_ref()
            .map(|f| f.members.iter().map(|m| m.guid).collect())
            .unwrap_or_default();
        // One that is in is no longer held off: the next refusal, if it
        // ever leaves and is asked again, starts a wait of its own. Nor
        // is one off the roster: a mate that left the team, or lost its
        // session, is asked afresh when it is back.
        for guid in &joined {
            self.autoplay.held_off.forget(guid);
        }
        let on_the_team: Vec<u32> = self.autoplay.team.mates.iter().map(|m| m.guid).collect();
        self.autoplay.held_off.retain(|g| on_the_team.contains(g));
        let waiting: Vec<(u32, f32)> = self
            .autoplay
            .team
            .mates
            .iter()
            .filter(|m| !m.in_fellowship && m.guid != 0 && !joined.contains(&m.guid))
            .map(|m| (m.guid, m.world.distance(me)))
            .filter(|(_, away)| *away < RECRUIT_RANGE)
            .collect();
        if self.world.fellowship.is_none() {
            // A founding already asked for and not yet answered: the
            // server makes the fellowship and tells us about it in its
            // own time, and two foundings would be two fellowships.
            if self
                .autoplay
                .founded
                .is_some_and(|t| now.duration_since(t) < FOUNDING_WAIT)
            {
                return false;
            }
            // Nothing came back, or the fellowship we made has gone:
            // either way there is none to vouch for now.
            self.autoplay.founded = None;
            self.autoplay.said_not_sharing = false;
            if waiting.is_empty() {
                return false;
            }
            // Not until the roster has stood a full board round: until
            // then this session may be leading only what it has heard so
            // far, and the one that sorts first among everyone is a frame
            // or a hop away from being heard. Nine sessions coming onto
            // the team in one tick founded two fellowships this way.
            if !self.autoplay.team.settled {
                self.autoplay.note(
                    "waiting for the roster to settle before founding the fellowship",
                    now,
                );
                return false;
            }
            // Nothing re-reads the option once the fellowship exists, so
            // a fellowship founded a moment too early shares no loot for
            // as long as it lives. The option and the founding go out on
            // the one ordered queue of actions and the server works
            // through them in that order, so all this has to wait for is
            // our own asking, which `autoplay_accept_invites` does at the
            // top of the same tick.
            if !self.shares_fellowship_loot() {
                self.autoplay.note(
                    "waiting for loot sharing before founding the fellowship",
                    now,
                );
                return false;
            }
            if self
                .autoplay
                .last_recruit
                .is_some_and(|t| now.duration_since(t) < RECRUIT_FLOOR)
            {
                return false;
            }
            let fname = team.fellowship_name.clone();
            self.fellowship_create(&fname, true);
            self.autoplay.founded = Some(now);
            self.autoplay.last_recruit = Some(now);
            self.autoplay
                .say(Doing::Helping, format!("starting the fellowship {fname}"));
            return true;
        }
        // A fellowship we did not found ourselves is left alone, and
        // said so once. Nothing on the wire says whether one shares
        // loot -- the full update writes the same 0x10 for every fellow
        // whatever the fellowship does -- so disbanding it would be
        // throwing away a working party on a guess, and the guess would
        // be wrong for every fellowship a person made by hand.
        if self.autoplay.founded.is_none() && !self.autoplay.said_not_sharing {
            self.autoplay.said_not_sharing = true;
            self.autoplay.note(
                "this fellowship was not founded by me: it may not share loot, \
                 and only disbanding it would tell",
                now,
            );
        }
        // Only the fellowship's own leader may recruit into it (ACE
        // `Player_Fellowship.cs`, `FellowshipRecruit`: anyone else is
        // answered 0x041D). A team leader that let itself be recruited
        // into a mate's fellowship leaves the gathering to that mate.
        if !leads_this {
            return false;
        }
        // A fellowship holds nine (ACE `Fellowship.MaxFellows`), and the
        // tenth asked is refused (0x041E) as often as it is asked. Said
        // once, and nobody is asked.
        if joined.len() >= MAX_FELLOWS {
            if !waiting.is_empty() {
                self.autoplay.note(
                    format!(
                        "the fellowship is full at {MAX_FELLOWS}: {} mate(s) left outside it",
                        waiting.len()
                    ),
                    now,
                );
            }
            return false;
        }
        let Some(guid) = next_invitee(
            &waiting,
            &self.autoplay.recruited,
            &self.autoplay.held_off,
            self.autoplay.last_recruit,
            now,
        ) else {
            return false;
        };
        self.fellowship_recruit(guid);
        self.autoplay.last_recruit = Some(now);
        self.autoplay
            .recruited
            .retain(|(g, t)| *g != guid && now.duration_since(*t) < RECRUIT_AGAIN);
        self.autoplay.recruited.push((guid, now));
        let name = self
            .autoplay
            .team
            .mates
            .iter()
            .find(|m| m.guid == guid)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| format!("{guid:#010x}"));
        self.autoplay.say(
            Doing::Helping,
            format!("bringing {name} into the fellowship"),
        );
        true
    }
}

#[cfg(test)]
mod tests;
