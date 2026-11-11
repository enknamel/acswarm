//! A flag only a portal stamps, gone for: a standing portal walked to and gone through, or the
//! Colosseum's rooms run to the one revealing it (`ac_world::arenas`): a fellowship, a ticket to
//! the Master Arbitrator (InqFellowQuest: none, and it is handed back, EmoteManager.cs:500-517),
//! the arena NPC's teleport, then each room's bell rung, its waves fought and its portal taken.

use std::time::{Duration, Instant};

use ac_world::arenas::Way;
use ac_world::quests::{Source, Want, Yield};
use ac_world::Place;
use glam::Vec3;

use super::fetch::{ANSWER_WAIT, UNLOCK_TRIES};
use super::{stuck, Ask, Hunting, PursuitLine, PursuitStep, OBTAIN_DEPTH};
use crate::autoplay::team::view::Mate;
use crate::autoplay::Doing;
use crate::Client;

/// The walk to what is acted on ends this near it (metres, 3D), as a fetch's.
const ACT_STOP: f32 = 1.5;

/// One step of a run through the arena's rooms.
#[derive(Clone, Debug, PartialEq)]
pub enum Stage {
    /// Found a fellowship: the ticket's giver takes it only from a fellow.
    Fellowship,
    /// Hand the ticket (`item`) to `giver` (weenie class `wcid`) at `at`.
    HandTicket {
        giver: String,
        wcid: u32,
        item: u32,
        at: Place,
    },
    /// Use `npc` (`wcid`) at `at`: it sends the fellowship into its arena.
    Enter { npc: String, wcid: u32, at: Place },
    /// Ring room `room`'s bell (`wcid`) at `at`, which starts its waves.
    Ring { room: usize, wcid: u32, at: Place },
    /// Fight room `room`'s waves by its bell at `at` till its portal comes.
    Clear { room: usize, at: Place },
    /// Go through room `room`'s portal (`wcid`) at `at`, into the next room.
    Leave { room: usize, wcid: u32, at: Place },
}

impl Stage {
    /// The stage in words, for `flag`.
    pub fn words(&self, flag: &str) -> String {
        match self {
            Stage::Fellowship => format!("found a fellowship for the arena (the {flag} flag)"),
            Stage::HandTicket { giver, .. } => format!("hand the ticket to {giver}"),
            Stage::Enter { npc, .. } => format!("go into the arena by {npc}"),
            Stage::Ring { room, .. } => format!("ring arena room {room}'s bell"),
            Stage::Clear { room, .. } => format!("clear arena room {room}'s waves"),
            Stage::Leave { room, .. } => format!("go through arena room {room}'s portal"),
        }
    }
}

/// What the arena stages are worked out from, read off the client.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ArenaView {
    pub(crate) fellowship: bool,
    /// On a team whose fellowship is switched off: it would go in without its mates.
    pub(crate) team_unfellowed: bool,
    /// A ticket handed in within the access quest's wait (`Ticket::lasts`).
    pub(crate) access: bool,
    /// The landblock and world spot the character stands at.
    pub(crate) at: Option<(u32, Vec3)>,
    /// The bells this character rang in the arena it is in.
    pub(crate) rung: Vec<u32>,
    /// The rooms' portals in view, by weenie class.
    pub(crate) portals: Vec<(u32, Place)>,
}

/// What was done to an arena object, awaiting its answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Deed {
    /// The ticket (weenie class) handed over: taken if it stays gone.
    Give(u32),
    /// A portal or NPC used: answered by a teleport.
    Enter,
    /// A bell rung: answered by its UseDone.
    Ring,
}

/// The arena deed out and when, on which object class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Deeding {
    pub(crate) deed: Deed,
    pub(crate) wcid: u32,
    /// The item handed, for a give.
    pub(crate) item: u32,
    pub(crate) at: Instant,
    pub(crate) tries: u32,
}

/// The flag a room's portal (weenie class) stamps on whoever goes through it, if any.
fn stamped_by(portal: u32) -> Option<String> {
    let rooms = &ac_world::arenas::all().rooms;
    rooms
        .iter()
        .find(|r| r.portal == portal)?
        .flags
        .first()
        .cloned()
}

impl Ask<'_> {
    /// The step for `flag`, stamped going through portal source `s`: walked through when it
    /// stands, the arena run when a room reveals it; else why not.
    pub(super) fn portal_step(
        &self,
        flag: &str,
        s: &Source,
        depth: usize,
    ) -> (PursuitStep, Option<Hunting>) {
        if s.kind == Yield::Portal {
            if let Some(at) = s.nearest(self.facts.me) {
                let (wcid, name, flag) = (s.wcid, s.name.clone(), flag.to_string());
                return (
                    PursuitStep::GoThrough {
                        wcid,
                        name,
                        at,
                        flag,
                    },
                    None,
                );
            }
        }
        if ac_world::arenas::all().reveals(flag) {
            return self.arena_step(flag, depth);
        }
        let place = ac_world::quest_chain::place_name(s.places.first().map_or(0, |p| p.cell));
        let why = format!(
            "the {flag} flag is stamped going through {} in {place}, which only something there reveals",
            s.name
        );
        (stuck(why), None)
    }

    /// The arena run's step for `flag`: in an arena whose rooms come to its portal, this room's;
    /// else a fellowship, the ticket (had as any item), its hand-in and the way in.
    fn arena_step(&self, flag: &str, depth: usize) -> (PursuitStep, Option<Hunting>) {
        let a = ac_world::arenas::all();
        let level = self.facts.level;
        let ways: Vec<&Way> = a.ways_to(flag, level).collect();
        let stage = |step| {
            let flag = flag.to_string();
            (PursuitStep::Arena { flag, step }, None)
        };
        let Some(first) = ways.first() else {
            return (
                stuck(format!(
                    "no way into the arena reaches the {flag} portal at level {level}"
                )),
                None,
            );
        };
        let v = self.arena;
        if let Some((lb, me)) =
            v.at.filter(|(lb, _)| ways.iter().any(|w| w.landblock == *lb))
        {
            let way = ways.iter().find(|w| w.landblock == lb).unwrap_or(first);
            let rooms = a.rooms_to(way, flag).unwrap_or_default();
            let here = a.room_at(lb, me);
            let Some(i) = here.and_then(|r| rooms.iter().position(|x| *x == r)) else {
                return (
                    stuck(format!(
                        "in the arena past the room revealing the {flag} portal"
                    )),
                    None,
                );
            };
            let room = rooms[i];
            if let Some((_, at)) = v.portals.iter().find(|p| p.0 == room.portal) {
                return stage(Stage::Leave {
                    room: i + 1,
                    wcid: room.portal,
                    at: *at,
                });
            }
            if !v.rung.contains(&room.bell) {
                return stage(Stage::Ring {
                    room: i + 1,
                    wcid: room.bell,
                    at: room.bell_at,
                });
            }
            return stage(Stage::Clear {
                room: i + 1,
                at: room.bell_at,
            });
        }
        if v.team_unfellowed {
            return (
                stuck("the arena takes a fellowship, and the team's is switched off".into()),
                None,
            );
        }
        if !v.fellowship {
            return stage(Stage::Fellowship);
        }
        if v.access {
            let (npc, wcid, at) = (first.name.clone(), first.npc, first.at);
            return stage(Stage::Enter { npc, wcid, at });
        }
        let Some(t) = a
            .tickets
            .iter()
            .find(|t| ways.iter().any(|w| w.access == t.access))
        else {
            return (
                stuck(format!(
                    "nobody known lets a fellowship into the arena for {flag}"
                )),
                None,
            );
        };
        if (self.asking.carried)(t.item) > 0 {
            let (giver, wcid, item, at) = (t.name.clone(), t.giver, t.item, t.at);
            return stage(Stage::HandTicket {
                giver,
                wcid,
                item,
                at,
            });
        }
        let name = ac_world::quests::all()
            .iter()
            .filter(|r| r.want == Want::Item(t.item))
            .find_map(|r| r.item.as_ref().map(|i| i.name.to_string()))
            .unwrap_or_else(|| format!("item {}", t.item));
        if depth >= OBTAIN_DEPTH {
            return (stuck(format!("{name} is too many links down")), None);
        }
        // Handed for by its vendor's Give set (two Trade Notes, ColoTicketPayment) ahead of the
        // Colosseum Coin Collector, whose AlternateCurrency (DID 57) the shop data does not carry.
        let want = [Want::Item(t.item)];
        if let Some(got) = self.reward_step(&want, depth + 1) {
            if !matches!(got.0, PursuitStep::Stuck { .. }) {
                return got;
            }
        }
        self.obtain(&want, 1, &name, depth + 1)
    }
}

impl Client {
    /// What the arena stages are worked out from now; a character out of every arena has rung no
    /// bell there.
    pub(super) fn arena_view(&mut self, now: Instant) -> ArenaView {
        let a = ac_world::arenas::all();
        let at = self
            .player
            .as_ref()
            .map(|p| (p.cell >> 16, p.world_position()));
        if !at.is_some_and(|(lb, _)| a.is_arena(lb)) {
            self.autoplay.growth.arena_rung.clear();
        }
        let lasts = a.tickets.iter().map(|t| t.lasts).max().unwrap_or(0);
        let team = &self.autoplay.config.team;
        let portals = self
            .world
            .objects
            .values()
            .filter(|o| a.rooms.iter().any(|r| r.portal == o.weenie_class_id))
            .filter_map(|o| {
                let cell = o.position?.cell;
                Some((
                    o.weenie_class_id,
                    Place {
                        at: o.world_pos()?,
                        cell,
                    },
                ))
            })
            .collect();
        ArenaView {
            fellowship: self.world.fellowship.is_some(),
            team_unfellowed: team.enabled && !team.fellowship,
            access: self.autoplay.growth.arena_access.is_some_and(|t| {
                now.saturating_duration_since(t) < Duration::from_secs(lasts.into())
            }),
            at,
            rung: self.autoplay.growth.arena_rung.clone(),
            portals,
        }
    }

    /// The working pursuit's portal or arena step, from `grow_pursuit`.
    pub(super) fn grow_arena(&mut self, line: &PursuitLine, now: Instant) -> Option<bool> {
        let said = format!("for {}: {}", line.title, line.step.words(0));
        match &line.step {
            PursuitStep::GoThrough { wcid, at, flag, .. } => {
                let flag = flag.clone();
                self.arena_deed(
                    Some(&line.id),
                    *wcid,
                    *at,
                    Deed::Enter,
                    Some(flag),
                    &said,
                    now,
                )
            }
            PursuitStep::Arena { step, .. } => match step {
                Stage::Fellowship => Some(self.found_fellowship(&said, now)),
                Stage::HandTicket { wcid, at, item, .. } => self.arena_deed(
                    Some(&line.id),
                    *wcid,
                    *at,
                    Deed::Give(*item),
                    None,
                    &said,
                    now,
                ),
                Stage::Enter { wcid, at, .. } => {
                    self.arena_deed(Some(&line.id), *wcid, *at, Deed::Enter, None, &said, now)
                }
                Stage::Leave { wcid, at, .. } => {
                    let flag = stamped_by(*wcid);
                    self.arena_deed(Some(&line.id), *wcid, *at, Deed::Enter, flag, &said, now)
                }
                Stage::Ring { wcid, at, .. } => {
                    self.arena_deed(Some(&line.id), *wcid, *at, Deed::Ring, None, &said, now)
                }
                // The fight step takes on the wave; this keeps the character in its room.
                Stage::Clear { at, .. } => {
                    let me = self.my_position()?;
                    if me.distance(at.at) > crate::visit::REACH && self.journey_to(*at, ACT_STOP) {
                        self.autoplay.say(Doing::Traveling, said);
                        return Some(true);
                    }
                    self.autoplay.say(Doing::Fighting, said);
                    Some(true)
                }
            },
            _ => None,
        }
    }

    /// A fellowship founded for the arena: by the team's own rule when it leads mates, else one of
    /// this character alone. True while it waits for it.
    fn found_fellowship(&mut self, said: &str, now: Instant) -> bool {
        let team = self.autoplay.config.team.clone();
        let gathering = team.enabled && team.fellowship && !self.autoplay.team.mates.is_empty();
        let asked = self.autoplay.founded.is_some_and(|t| {
            now.duration_since(t) < crate::autoplay::team::fellowship::FOUNDING_WAIT
        });
        if !gathering && !asked {
            self.fellowship_create(&team.fellowship_name, true);
            self.autoplay.founded = Some(now);
        }
        self.autoplay.say(Doing::Helping, said.to_string());
        true
    }

    /// Walk to the object of class `wcid` near `at` and do `deed` to it, then wait for its answer;
    /// gone through, `flag` is held. True while under way; a refusal passes quest `id` over.
    #[allow(clippy::too_many_arguments)] // one call per stage; the deed's parts are the stage's
    pub(crate) fn arena_deed(
        &mut self,
        id: Option<&str>,
        wcid: u32,
        at: Place,
        deed: Deed,
        flag: Option<String>,
        said: &str,
        now: Instant,
    ) -> Option<bool> {
        let mut tries = 0;
        if let Some(d) = self
            .autoplay
            .growth
            .arena_deeding
            .filter(|d| d.wcid == wcid)
        {
            let waited = now.saturating_duration_since(d.at) >= ANSWER_WAIT;
            let done = match d.deed {
                // Handed back at once when no arena is free (EmoteManager.cs:500-517): it stays out.
                Deed::Give(item) => waited && self.carried_count(item) == 0,
                Deed::Enter => self.teleported_at.is_some_and(|t| t >= d.at),
                Deed::Ring => self.use_done.is_some_and(|(_, t)| t >= d.at),
            };
            if done {
                self.autoplay.growth.arena_deeding = None;
                match d.deed {
                    Deed::Give(_) => self.autoplay.growth.arena_access = Some(now),
                    Deed::Ring => self.autoplay.growth.arena_rung.push(wcid),
                    Deed::Enter => {}
                }
                if let Some(f) = flag {
                    self.hold_quest_flags(&[f]);
                }
                self.quest_board_at = None;
                return Some(true);
            }
            if !waited {
                return Some(true);
            }
            self.autoplay.growth.arena_deeding = None;
            tries = d.tries;
            if tries >= UNLOCK_TRIES || matches!(d.deed, Deed::Give(_)) {
                self.arena_refused(id, format!("{said}: no answer"), now);
                return Some(false);
            }
        }
        let me = self.my_position()?;
        let found = self.lying_near(wcid, at);
        let goal = found.map_or(at, |(_, p)| p);
        if me.distance(goal.at) > crate::visit::REACH {
            if self.journey_to(goal, ACT_STOP) {
                self.autoplay.say(Doing::Traveling, said.to_string());
                return Some(true);
            }
            self.arena_refused(id, format!("{said}: no way there"), now);
            return Some(false);
        }
        let Some((guid, _)) = found else {
            self.arena_refused(id, format!("{said}: it is not there"), now);
            return Some(false);
        };
        if self.traveling() || self.is_walking() {
            self.interrupt_travel("at the arena's object");
            self.stop_going(None, "at the arena's object");
            return Some(true);
        }
        if self.hands_busy(now) {
            return Some(true);
        }
        let sent = match deed {
            Deed::Give(item) => {
                let held = self
                    .world
                    .inventory()
                    .find(|o| o.weenie_class_id == item && o.wielder.is_none())?
                    .guid;
                self.give(guid, held, Some(1))
            }
            Deed::Enter | Deed::Ring => {
                self.interact(guid);
                true
            }
        };
        if sent {
            let item = match deed {
                Deed::Give(item) => item,
                _ => 0,
            };
            let tries = tries + 1;
            self.autoplay.growth.arena_deeding = Some(Deeding {
                deed,
                wcid,
                item,
                at: now,
                tries,
            });
        }
        self.autoplay.say(Doing::Helping, said.to_string());
        Some(true)
    }

    /// A deed that came to nothing: the pursued quest `id` passed over, else noted.
    fn arena_refused(&mut self, id: Option<&str>, why: String, now: Instant) {
        match id {
            Some(id) => self.hold_pursuit(id, why, now),
            None => self.autoplay.note(why, now),
        }
    }

    /// A follower after a `leader` in an arena: in by the way NPC its fellowship's access opens,
    /// and through each room's portal while the leader is in a room further on. `None` when the
    /// leader is in no arena, or in this room.
    pub(crate) fn follow_arena(&mut self, leader: &Mate, now: Instant) -> Option<bool> {
        let a = ac_world::arenas::all();
        let lb = (leader.cell >> 16) & 0xFFFF;
        if !a.is_arena(lb) {
            return None;
        }
        let level = u32::try_from(self.world.stats.level).unwrap_or(0);
        let way = a
            .ways
            .iter()
            .filter(|w| w.landblock == lb && w.min_level <= level)
            .min_by_key(|w| w.min_level)?;
        let said = format!("following {} into the arena", leader.name);
        let (here, me) = self
            .player
            .as_ref()
            .map(|p| (p.cell >> 16, p.world_position()))?;
        if here != lb {
            return self.arena_deed(None, way.npc, way.at, Deed::Enter, None, &said, now);
        }
        let mut rooms = vec![a.room_at(lb, way.land.at)?];
        while let Some(next) = a.room_at(lb, rooms[rooms.len() - 1].to.at) {
            if rooms.contains(&next) {
                break;
            }
            rooms.push(next);
        }
        let index = |at: Vec3| {
            a.room_at(lb, at)
                .and_then(|r| rooms.iter().position(|x| *x == r))
        };
        let (mine, theirs) = (index(me)?, index(leader.world)?);
        if theirs <= mine {
            return None;
        }
        let room = rooms[mine];
        let at = self
            .world
            .objects
            .values()
            .filter(|o| o.weenie_class_id == room.portal)
            .find_map(|o| {
                Some(Place {
                    at: o.world_pos()?,
                    cell: o.position?.cell,
                })
            })?;
        let flag = stamped_by(room.portal);
        self.arena_deed(None, room.portal, at, Deed::Enter, flag, &said, now)
    }
}
