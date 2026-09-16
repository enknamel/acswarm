use std::time::Duration;

use serde::{Deserialize, Serialize};

#[cfg(doc)]
use crate::autoplay::{called_to, Autoplay};
use crate::autoplay::{
    corpse_within_reach, deal, skills_asked_of, Role, Shut, Turn, CLAIM_STALE, SAME_MOMENT,
};
use crate::Client;

/// The mate the fleet should be following instead of this character,
/// when this character leads a fellowship it founded and that mate has
/// one of its own: the second fellowship this character gives up (see
/// [`Client::autoplay_yield_fellowship`]).
///
/// A session learns of another fellowship from two words that have to
/// agree. The world's own record says who is in *this* fellowship,
/// first-hand (`members`). The board's row for each mate says whether
/// it is in *a* fellowship (`Mate::in_fellowship`), up to a round old.
/// A mate whose row says it is in one and whom the world does not list
/// is in another. Only the mate the settled roster says leads counts:
/// the leader of the team is the one to gather it, and a leader that
/// let itself be recruited into this fellowship instead is a working
/// party, not a rival. And only one whose session will do the
/// gathering: one playing on its own, or one that asked to lead. A
/// person standing in a fellowship of their own with autoplay off
/// would recruit nobody, and a fleet given up to them would stand
/// unfellowshipped.
pub(crate) fn rival_leader<'a>(view: &'a TeamView, members: &[u32]) -> Option<&'a Mate> {
    if !view.settled || view.leader {
        return None;
    }
    view.leader_mate()
        .filter(|m| m.in_fellowship && !members.contains(&m.guid) && (m.autoplay || m.leads))
}

/// What one of the others has told us about itself. The host fills this
/// in from the bus every frame (see `ac_plugin::team`); the rules here
/// only read it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mate {
    pub name: String,
    pub guid: u32,
    /// Its session index in its own process.
    pub session: usize,
    pub world: glam::Vec3,
    pub health: f32,
    pub role: Role,
    pub target: Option<u32>,
    pub target_name: String,
    pub in_fellowship: bool,
    /// Items it is short of, by name.
    pub wants: Vec<String>,
    /// Targets it has already debuffed.
    pub debuffed: Vec<u32>,
    /// The body it has open or is walking to, and how long it has been
    /// at that one: its claim on it. The others leave a claimed body
    /// alone (see [`TeamView::working`]). The age travels with the
    /// claim because it is the claimant's own clock that says whether
    /// the claim is still good, not the clock of whoever reads it.
    pub looting: Option<u32>,
    pub looting_for: Duration,
    /// True for the one that picks the targets.
    pub leader: bool,
    /// Its Life Magic as it stands, buffs counted: what decides who
    /// softens a hard target.
    pub life_magic: u32,
    /// It knows a vulnerability or an imperil it can cast right now.
    pub can_soften: bool,
    /// It asked to lead (see `Team::lead`).
    pub leads: bool,
    /// It is flying (no-clip); followers fly too.
    pub flying: bool,
    /// The cell it stands in: indoors (a hub, a dungeon) is somewhere
    /// a journey cannot be planned to from outside.
    pub cell: u32,
    /// Level and experience, total and unspent, as the sheet has them
    /// (0 before it arrives). The fleet view works XP an hour out of
    /// the total over time.
    pub level: i32,
    pub total_xp: i64,
    pub available_xp: i64,
    /// Stamina and mana as fractions of their maximum, like `health`.
    pub stamina: f32,
    pub mana: f32,
    /// Its rules are on: it plays on its own.
    pub autoplay: bool,
    /// It follows the leader about (`Team::follow`, and not leading).
    pub following: bool,
    /// Its Salvaging as it stands, buffs counted, and whether it carries
    /// an Ust: what decides who salvages for the team.
    pub salvaging: u32,
    pub has_ust: bool,
    /// How close to empty it is, what it still has to buy and what that
    /// will cost: what the party decides hunting and restocking from.
    pub supplies: crate::logistics::Supplies,
    /// The hunting ground it is on or heading for: landblock, where,
    /// and what it is called. What the party goes back to together
    /// after a trip to town.
    pub ground: Option<(u32, glam::Vec2, String)>,
    /// It is on its way somewhere, or keeping up with a leader that is
    /// (see `Client::on_its_way`). A party on the road walks past what its
    /// leader walks past and stops for what any of it on the road is
    /// fighting, so it neither scatters to fight nor walks off and leaves
    /// one of its own behind.
    pub on_its_way: bool,
    /// Its skills that its loot rules ask about (see
    /// `Profile::skills_asked`), as `(id, base, current, advancement)`:
    /// what another character needs to judge a body on its behalf (see
    /// [`Mate::wielder`]). Empty when its rules ask about none.
    pub skills: Vec<(u32, u32, u32, u32)>,
    /// The bodies it shut lately as emptied, and what each is for the
    /// others (see [`Shut`], [`Autoplay::shuts_to_say`]).
    pub shut: Vec<Shut>,
    /// How many bodies it opened first lately, within `DEAL_WINDOW`:
    /// its turns at the bodies (see [`TeamView::opens_first`]). Going back
    /// for what one of the others shut first and left for it is no turn.
    pub opened_first: u16,
    /// It opens bodies at all: it has loot rules to go by, and its pack is
    /// neither down to the slots kept for a counter's money nor past the
    /// server's wall (see `Client::opens_bodies`). One that does not is
    /// never dealt a body nor left anything on one: an Ust carrier with no
    /// loot profile never came, and the salvage left for it rotted.
    pub opens_bodies: bool,
    /// The names of what has attacked it lately (see
    /// `Autoplay::attacked_by`): what the leader's plan reads to give
    /// it the creature that is on it rather than the party's.
    pub hit_by: Vec<String>,
}

impl Mate {
    /// The mate as the turns at a newly fallen body read it; `None` for
    /// one that would not open a body at all: played by hand, dead, not
    /// in the world yet, or not opening bodies ([`Mate::opens_bodies`]).
    pub fn turn(&self) -> Option<Turn> {
        (self.autoplay && self.health > 0.0 && self.guid != 0 && self.opens_bodies).then(|| Turn {
            guid: self.guid,
            world: self.world,
            looting: self.looting.is_some(),
            fighting: self.target.is_some(),
            room: !self.supplies.pack_full && !self.supplies.laden,
            opened: self.opened_first,
        })
    }

    /// Whether the mate could come for something left for it on a body at
    /// `at` (see [`called_to`]): it would open a body at all, it stands
    /// within reach of this one, and it has room to carry what is left.
    /// One at another body or fighting still could, once it is done. One
    /// dead, across the field, with no loot rules or a pack its own looting
    /// would not take from could not, and nothing is left waiting on it;
    /// nor on one gone from the board, which is not asked about at all.
    pub fn could_come_for(&self, at: glam::Vec3) -> bool {
        self.turn()
            .is_some_and(|t| t.room && corpse_within_reach(t.world, at))
    }

    /// The mate as a loot rule sees the character reading it: its level
    /// and the skills it said its rules ask about. A rule asks nothing
    /// else of the character but its name (see `ac_loot::profile::Mine`).
    pub fn wielder(&self) -> crate::weapons::Wielder {
        crate::weapons::Wielder {
            level: self.level.max(0) as u32,
            skills: self.skills.clone(),
            ..Default::default()
        }
    }
}

/// The team as the host last saw it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TeamView {
    pub mates: Vec<Mate>,
    /// Whether this character is the one picking targets.
    pub leader: bool,
    /// What this character said about itself as the view was made (see
    /// `ac_plugin::team::describe`): what the others read it by, and so
    /// what the turns at a newly fallen body read it by too (see
    /// `Autoplay::ours_to_open`). `None` in a view nobody said anything
    /// into.
    pub me: Option<Mate>,
    /// The roster behind this view has stood unchanged for a full board
    /// round (see `ac_plugin::team::Settling`). Until it has, `leader`
    /// is only what this session knows so far: a session that has just
    /// come onto the team has heard nobody and leads a roster of one.
    /// The fellowship rules found or give up a fellowship on the leader
    /// flag only once the view has settled; everything else reads the
    /// flag as it comes.
    pub settled: bool,
}

impl TeamView {
    /// The one leading, if it is one of the others.
    pub fn leader_mate(&self) -> Option<&Mate> {
        self.mates.iter().find(|m| m.leader)
    }

    /// The target the team is on: the leader's, else the first anyone has.
    pub fn target(&self) -> Option<(u32, String)> {
        let leader = self
            .mates
            .iter()
            .find(|m| m.leader)
            .and_then(|m| m.target.map(|t| (t, m.target_name.clone())));
        leader.or_else(|| {
            self.mates
                .iter()
                .find_map(|m| m.target.map(|t| (t, m.target_name.clone())))
        })
    }

    /// Whether anyone has already landed the debuffs on `target`.
    pub fn debuffed(&self, target: u32) -> bool {
        self.mates.iter().any(|m| m.debuffed.contains(&target))
    }

    /// Whether one of the others has the body `guid` open or is on its
    /// way to it.
    ///
    /// The server hands a container to one viewer and refuses everyone
    /// else outright (ACE `Container.CheckUseRequirements`), so a body
    /// two characters want is a body one of them empties and the other
    /// asks about until it rots. Nine characters standing on one tile
    /// see the same bodies and rank them by the same rule, so without
    /// this they all open the same one: a nine-character run opened 41
    /// bodies 1,386 times, against 2.75 times each for one character
    /// hunting alone.
    ///
    /// A claim goes stale (`CLAIM_STALE`): one that never did would
    /// let a claimant that stalled lock a body for its whole life. One
    /// that has died is dropped at once rather than waited out: its
    /// client keeps saying what it was working, and the same board row
    /// that says so already says its health is nothing.
    pub fn working(&self, guid: u32) -> bool {
        self.mates
            .iter()
            .any(|m| m.health > 0.0 && m.looting == Some(guid) && m.looting_for < CLAIM_STALE)
    }

    /// Whether one of the others has a better claim on `guid` than this
    /// character's own, which has stood for `ours`; `me` is this
    /// character's player guid.
    ///
    /// Better is older. Two claims made within a board round of each
    /// other are the same moment -- each session ages its own claim on
    /// its own clock and hears the others' a board round late, so
    /// nothing finer than that can be told apart -- and there the lower
    /// player guid wins, which is an answer both sides reach.
    ///
    /// Without this a body two characters chose in the same tick was
    /// opened by neither: each heard the other's claim half a second
    /// later, each read it as "someone else has it", and each stood
    /// off, while the walk both had started went on being published as
    /// a claim until every claim in it aged out at once.
    pub fn outranks_our_claim(&self, guid: u32, ours: Duration, me: u32) -> bool {
        self.mates.iter().any(|m| {
            m.health > 0.0
                && m.looting == Some(guid)
                && m.looting_for < CLAIM_STALE
                && match m.looting_for.checked_sub(ours) {
                    Some(older) if older > SAME_MOMENT => true,
                    _ => ours.saturating_sub(m.looting_for) <= SAME_MOMENT && m.guid < me,
                }
        })
    }

    /// Whose turn it is to open the body `guid`, lying at `at`, `me` being
    /// this character as the others read it: the player guid of one of
    /// those standing over it.
    ///
    /// This is only for the moment before a claim can have reached the
    /// board -- the word goes out every half second and a body is
    /// chosen within a tick of falling -- and in that moment the fleet
    /// needs an answer every session reaches on its own. All of them work
    /// this one out of the same roster, so there is nothing to negotiate
    /// and nothing to vote on, which is how the leader is settled too
    /// (`ac_plugin::team`).
    ///
    /// Turns go round, a body a turn: the one that has opened the fewest
    /// bodies first lately ([`Mate::opened_first`]), and among those the
    /// first in the deal for this body ([`deal`]), so bodies falling
    /// together go to different characters. It used to be the lowest
    /// guid, which opened every body that fell while it stood over one
    /// and carried the whole party's loot.
    ///
    /// Best effort. Only one free to open it is dealt a turn: playing on
    /// its own, alive, in the world, standing over it, not at another body,
    /// not fighting, and with room in its pack. With nobody free in reach
    /// it is ours to walk to, as it is everyone's, since standing off from
    /// a body nobody will open leaves it lying. And the turn only holds
    /// for the first second (`CLAIM_SETTLE`): one dealt a body that does
    /// not claim it by then loses it to whoever is free first.
    ///
    /// That only holds while every session judges the same candidates,
    /// so this character passes the same tests as the others, reach among
    /// them. A body is owed to a character out to the fight radius when
    /// one of its own kills fell there, which is further than
    /// `LOOT_NEAR`: without the test on ourselves, a caster twenty-two
    /// metres off called a body its own while every mate's roster had it
    /// too far away to count, and two of them opened it in the same second.
    pub fn opens_first(&self, guid: u32, at: glam::Vec3, me: Turn) -> u32 {
        self.mates
            .iter()
            .filter_map(Mate::turn)
            .chain(std::iter::once(me))
            .filter(|t| !t.looting && !t.fighting && t.room)
            .filter(|t| corpse_within_reach(t.world, at))
            .min_by_key(|t| (t.opened, deal(guid, t.guid)))
            .map_or(me.guid, |t| t.guid)
    }

    /// The others a body this character empties is judged for as it is
    /// shut (see `Client::shut_for`): everyone in the world, or, while
    /// this character is in a fellowship (`fellows`, its members' player
    /// guids), its fellows only. One outside it is left to open the body
    /// or not by its own lights, as before.
    pub fn judged_at_a_shut<'a>(
        &'a self,
        fellows: Option<&'a [u32]>,
    ) -> impl Iterator<Item = &'a Mate> {
        self.mates
            .iter()
            .filter(move |m| m.guid != 0 && fellows.is_none_or(|f| f.contains(&m.guid)))
    }

    /// The mate nearest `me` that is short of something we could hand
    /// over, within `radius` metres.
    pub fn wanting(&self, me: glam::Vec3, radius: f32) -> Option<&Mate> {
        self.mates
            .iter()
            .filter(|m| !m.wants.is_empty() && m.world.distance(me) <= radius)
            .min_by(|a, b| a.world.distance(me).total_cmp(&b.world.distance(me)))
    }

    /// The mate in the worst shape, for a healer.
    pub fn worst_hurt(&self) -> Option<&Mate> {
        self.mates
            .iter()
            .filter(|m| m.health > 0.0 && m.health < 1.0)
            .min_by(|a, b| a.health.total_cmp(&b.health))
    }
}

impl Client {
    /// This character's skills that its loot rules ask about, for the
    /// others to judge a body on its behalf (see [`Mate::skills`]).
    pub fn skills_its_rules_ask_about(&self) -> Vec<(u32, u32, u32, u32)> {
        let asked = self
            .loot_profile()
            .map(|p| p.skills_asked())
            .unwrap_or_default();
        if asked.is_empty() {
            return Vec::new();
        }
        skills_asked_of(&self.wielder(), &asked)
    }

    /// Whether this character opens bodies at all: it has loot rules to go
    /// by, and a pack its looting takes from (see
    /// `Autoplay::corpse_waiting`). What it says about itself as
    /// [`Mate::opens_bodies`].
    pub fn opens_bodies(&self) -> bool {
        let room = self.room_for_loot();
        self.loot_profile().is_some() && !room.pack_low && !room.past_the_wall
    }
}
