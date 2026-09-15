//! The wire between characters playing together.
//!
//! `ac_client::autoplay` has rules for a team: fight what the leader is
//! fighting, land the debuffs on it, hand a teammate what it is short
//! of, heal whoever is worst hurt, bring everyone into one fellowship.
//! Those rules read a [`TeamView`], and until now nothing filled it in.
//! The word each session gives about itself carries its Salvaging and
//! whether it has an Ust, so everyone agrees on who salvages. It carries
//! the skills its loot rules ask about and the bodies it emptied, with
//! the others it found nothing left on each for, so a body one of them
//! finished is not opened again by the rest.
//!
//! This plugin does. Every session with the team rules on says who it
//! is and what it is doing, a few times a second, on the blackboard:
//! to the other sessions in this process directly, and to every other
//! process through the bus when one is attached (`--bus`). Every
//! session gathers what the others said into a roster, drops anyone who
//! has gone quiet, and hands the roster to its own rules.
//!
//! The leader is not elected, it is chosen: the character whose name
//! sorts first among those on the team. Every session reaches the same
//! answer from the same roster, so there is no vote and nothing to
//! agree on, and a leader who logs off is replaced the moment the
//! others stop hearing from it.
//!
//! The same roster, that is, once everyone has been heard. A session
//! that has just come onto the team has heard nobody and leads its
//! roster of one, and nine sessions coming on in the same tick led
//! nine such rosters for a frame: two of them founded a fellowship
//! each before the first board round had carried a word, the fleet
//! split between the two, and members of one had no right to the
//! other's bodies. So a view says whether it has *settled* (see
//! [`Settling`]): the leader flag is acted on -- a fellowship founded
//! or given up -- only once the roster has stood unchanged for a full
//! board round.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use ac_client::autoplay::{Mate, TeamView};
use ac_client::plan::Plan;
use serde_json::Value;

use crate::{Ctx, Plugin};

/// The topic a session's word about itself goes out on.
pub const MATE_TOPIC: &str = "autoplay.mate";
/// The topic the fleet view asks another process's session to do
/// something on: `{"process", "session", "name", "action"}`, where
/// `action` is a [`Request`] word. The team plugin in the process named
/// applies it to the session named (see [`Request::apply`]).
pub const REQUEST_TOPIC: &str = "fleet.request";
/// The topic the leader's plan for the party goes out on, once a board
/// round: an `ac_client::plan::Plan`. Every session on the team takes its
/// orders from the plan of the leader it sees, and from nobody else's.
pub const PLAN_TOPIC: &str = "autoplay.plan";
/// How often each session speaks.
const SAY_EVERY: Duration = Duration::from_millis(500);
/// A mate not heard from for this long has gone.
const FORGET_AFTER: Duration = Duration::from_secs(6);
/// How long a session's roster must stand unchanged before its view is
/// settled: one full board round, in which everyone already on the
/// team has said its piece, and a little over for a word crossing the
/// bus from another process at the very end of the round.
const SETTLE_AFTER: Duration = Duration::from_millis(750);

/// A mate as last heard, and when.
#[derive(Clone, Debug)]
struct Heard {
    mate: Mate,
    at: Instant,
}

/// Everyone on the team, keyed by where they spoke from: the process
/// (this one or a name on the bus) and the session within it.
#[derive(Debug, Default)]
pub struct Roster {
    heard: BTreeMap<(String, usize), Heard>,
}

impl Roster {
    /// Take in one word from a mate.
    pub fn hear(&mut self, process: &str, session: usize, mate: Mate, now: Instant) {
        self.heard
            .insert((process.to_string(), session), Heard { mate, at: now });
    }

    /// Forget whoever has gone quiet.
    pub fn forget_quiet(&mut self, now: Instant) {
        self.heard
            .retain(|_, h| now.duration_since(h.at) < FORGET_AFTER);
    }

    /// The team as one session sees it: everyone else on the roster,
    /// and whether this session's character leads. The leader is the
    /// character that asked to lead (the one played by hand), or else
    /// the one whose name sorts first, this one included. What the
    /// session said about itself goes with it, so that the turns at a
    /// newly fallen body read it as the others read it.
    pub fn view_for(&self, me: &Mate) -> TeamView {
        let mut mates: Vec<Mate> = self
            .heard
            .values()
            .map(|h| h.mate.clone())
            .filter(|m| !(m.name == me.name && m.guid == me.guid))
            .collect();
        let first = leader_name(mates.iter().chain(std::iter::once(me)));
        let leader = first.as_deref() == Some(me.name.as_str());
        for m in &mut mates {
            m.leader = first.as_deref() == Some(m.name.as_str());
        }
        TeamView {
            mates,
            leader,
            me: Some(me.clone()),
            // Whether the roster has stood a round is the plugin's to say
            // (see `Settling`); a view read straight off the roster has not.
            settled: false,
        }
    }

    /// Everyone heard, with where each spoke from (process, session).
    pub fn iter(&self) -> impl Iterator<Item = (&str, usize, &Mate)> {
        self.heard
            .iter()
            .map(|((p, s), h)| (p.as_str(), *s, &h.mate))
    }

    pub fn len(&self) -> usize {
        self.heard.len()
    }

    pub fn is_empty(&self) -> bool {
        self.heard.is_empty()
    }
}

/// Whether one session's roster has stood still long enough to be acted
/// on (see [`TeamView::settled`]).
///
/// A session that has just come onto the team leads a roster of one
/// until the others' words arrive, a frame later in this process and a
/// hop later from another. Acting on that view founded two fellowships
/// for one fleet. The roster is settled once the names on it have not
/// changed for [`SETTLE_AFTER`]: a full board round, by the end of
/// which everyone who was on the team when this session joined has
/// spoken. Anyone joining during the round speaks the moment it joins,
/// and its word starts the round again.
#[derive(Clone, Debug, Default)]
pub struct Settling {
    /// The names heard, sorted, and since when they have been these.
    names: Vec<String>,
    since: Option<Instant>,
}

impl Settling {
    /// Note the roster as this session sees it at `now`, and say whether
    /// it has stood for a full round. The first look starts the round;
    /// a roster with different names on it starts it again.
    pub fn settle(&mut self, mut names: Vec<String>, now: Instant) -> bool {
        names.sort_unstable();
        if self.since.is_none() || names != self.names {
            self.names = names;
            self.since = Some(now);
            return false;
        }
        self.since
            .is_some_and(|t| now.duration_since(t) >= SETTLE_AFTER)
    }
}

/// Who leads among `mates`: the first by name of those that asked to,
/// else the first by name of all. `None` when nobody has a name.
pub fn leader_name<'a>(mates: impl Iterator<Item = &'a Mate> + Clone) -> Option<String> {
    let named = |lead_only: bool| {
        mates
            .clone()
            .filter(|m| !lead_only || m.leads)
            .map(|m| m.name.as_str())
            .filter(|n| !n.is_empty())
            .min()
            .map(str::to_string)
    };
    named(true).or_else(|| named(false))
}

/// What the fleet view can ask of a session, its own or another
/// process's (over [`REQUEST_TOPIC`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// Play on its own.
    AutoplayOn,
    AutoplayOff,
    /// Follow the leader about (turns the team rules on too).
    FollowOn,
    FollowOff,
    /// Come to the leader now: follow on, and whatever was being
    /// fought is let go.
    Regroup,
    /// Autoplay off, the journey and the fight cancelled.
    Stop,
}

impl Request {
    pub const ALL: [Request; 6] = [
        Request::AutoplayOn,
        Request::AutoplayOff,
        Request::FollowOn,
        Request::FollowOff,
        Request::Regroup,
        Request::Stop,
    ];

    /// The word on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Request::AutoplayOn => "autoplay_on",
            Request::AutoplayOff => "autoplay_off",
            Request::FollowOn => "follow_on",
            Request::FollowOff => "follow_off",
            Request::Regroup => "regroup",
            Request::Stop => "stop",
        }
    }

    pub fn parse(word: &str) -> Option<Request> {
        Request::ALL.into_iter().find(|r| r.as_str() == word)
    }

    /// Do it to `client`.
    pub fn apply(self, client: &mut ac_client::Client) {
        let cfg = &mut client.autoplay.config;
        match self {
            Request::AutoplayOn => cfg.enabled = true,
            Request::AutoplayOff => cfg.enabled = false,
            Request::FollowOn => {
                cfg.team.enabled = true;
                cfg.team.follow = true;
            }
            Request::FollowOff => cfg.team.follow = false,
            Request::Regroup => {
                cfg.enabled = true;
                cfg.team.enabled = true;
                cfg.team.follow = true;
                client.autoplay.drop_target();
                client.attack_target = None;
            }
            Request::Stop => {
                cfg.enabled = false;
                client.autoplay.drop_target();
                client.attack_target = None;
                client.follow = None;
                if client.traveling() || client.visiting().is_some() {
                    client.cancel_travel();
                }
            }
        }
    }

    /// The message asking `process`'s session `session` (whose
    /// character is `name`) to do this.
    pub fn message(self, process: &str, session: usize, name: &str) -> Value {
        serde_json::json!({
            "process": process,
            "session": session,
            "name": name,
            "action": self.as_str(),
        })
    }

    /// Read a [`REQUEST_TOPIC`] message: what it asks and of whom, as
    /// `(process, session, name, request)`.
    pub fn from_message(value: &Value) -> Option<(String, usize, String, Request)> {
        let process = value.get("process")?.as_str()?.to_string();
        let session = value.get("session")?.as_u64()? as usize;
        let name = value
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let request = Request::parse(value.get("action")?.as_str()?)?;
        Some((process, session, name, request))
    }
}

/// The plugin: one roster shared by every session in this process, and
/// when each session last spoke.
#[derive(Default)]
pub struct Team {
    roster: Roster,
    /// When each session last said its piece on the board, and what it
    /// said.
    last_said: BTreeMap<usize, (Instant, Mate)>,
    /// How long each session's roster has stood unchanged (see
    /// [`Settling`]). Forgotten when the session leaves the team, so
    /// coming back on starts the round again.
    settling: BTreeMap<usize, Settling>,
}

/// A vital (0 health, 1 stamina, 2 mana) as a fraction of its maximum,
/// 1.0 when the sheet has not arrived.
pub fn vital_fraction(stats: &ac_world::stats::PlayerStats, i: usize) -> f32 {
    let max = stats.vital_max_current(i);
    if max == 0 {
        return 1.0;
    }
    (stats.vitals[i].current as f32 / max as f32).clamp(0.0, 1.0)
}

/// What a session says about itself; `None` before it is in the world
/// with a name. The fleet view builds its rows for this process's
/// sessions from the same word.
pub fn describe(client: &ac_client::Client, session: usize) -> Option<Mate> {
    let player = client.player.as_ref()?;
    let cfg = &client.autoplay.config.team;
    let name = client.world.stats.name.clone();
    if name.is_empty() {
        return None;
    }
    let stats = &client.world.stats;
    let guid = client.world.player_guid.unwrap_or(0);
    let target = client
        .attack_target
        .or(client.autoplay.casting_at())
        .filter(|g| {
            client
                .world
                .objects
                .get(g)
                .is_some_and(|o| o.health.unwrap_or(1.0) > 0.0)
        });
    let target_name = target
        .and_then(|g| client.world.objects.get(&g).map(|o| o.name.clone()))
        .unwrap_or_default();
    let in_fellowship = client
        .world
        .fellowship
        .as_ref()
        .is_some_and(|f| f.members.iter().any(|m| m.guid == guid));
    // The body this one is working, so the others leave it alone. Its
    // age goes with it: a claim is only believed while it is fresh, and
    // it is this character's clock that says how old it is.
    let (looting, looting_for) = client
        .autoplay
        .corpse_claim(Instant::now())
        .map_or((None, Duration::ZERO), |(g, held)| (Some(g), held));
    Some(Mate {
        name,
        guid,
        session,
        world: player.world_position(),
        health: client.health_fraction(),
        role: cfg.role,
        target,
        target_name,
        in_fellowship,
        wants: client.autoplay.wants.clone(),
        debuffed: client.autoplay.debuffed.clone(),
        looting,
        looting_for,
        leader: false,
        life_magic: client.life_magic(),
        can_soften: client.can_soften(),
        leads: cfg.lead,
        flying: client.noclip(),
        cell: player.cell,
        level: stats.level,
        total_xp: stats.total_xp,
        available_xp: stats.available_xp,
        stamina: vital_fraction(stats, 1),
        mana: vital_fraction(stats, 2),
        autoplay: client.autoplay.config.enabled,
        following: cfg.enabled && cfg.follow && !cfg.lead,
        salvaging: client.salvaging(),
        has_ust: client.salvage_tool().is_some(),
        supplies: client.supplies(&client.autoplay.config.growth, Instant::now()),
        ground: client.hunting_ground(),
        on_its_way: client.on_its_way(),
        skills: client.skills_its_rules_ask_about(),
        shut: client.autoplay.shuts_to_say(Instant::now()),
        opened_first: client.autoplay.opened_first(Instant::now()),
        opens_bodies: client.opens_bodies(),
        hit_by: client.attackers_lately(),
    })
}

impl Team {
    /// Whether session `session`, describing itself as `me` at `now`, is
    /// due to say so on the board (every [`SAY_EVERY`]). When it is, `me`
    /// is what it last said from then on.
    fn say(&mut self, session: usize, me: &Mate, now: Instant) -> bool {
        let due = self
            .last_said
            .get(&session)
            .is_none_or(|(t, _)| now.duration_since(*t) >= SAY_EVERY);
        if due {
            self.last_said.insert(session, (now, me.clone()));
        }
        due
    }

    /// What session `session` last said about itself on the board: what
    /// the others read it by.
    fn said(&self, session: usize) -> Option<&Mate> {
        self.last_said.get(&session).map(|(_, said)| said)
    }
}

impl Plugin for Team {
    fn name(&self) -> &str {
        "team"
    }

    fn session_removed(&mut self, index: usize) {
        crate::shift_removed(&mut self.last_said, index);
        crate::shift_removed(&mut self.settling, index);
    }

    fn tick(&mut self, cx: &mut Ctx) {
        let now = Instant::now();
        let session = cx.index;
        // Hear everyone who spoke since last frame, other sessions of
        // this process and other processes alike.
        let me_name = cx.board.bus_name().unwrap_or("local").to_string();
        let heard: Vec<(String, usize, Mate)> = cx
            .board
            .messages_on(MATE_TOPIC)
            .filter_map(|m| {
                let mate: Mate = serde_json::from_value(m.value.clone()).ok()?;
                let process = m.origin.clone().unwrap_or_else(|| me_name.clone());
                Some((
                    process,
                    if m.origin.is_some() {
                        mate.session
                    } else {
                        m.from
                    },
                    mate,
                ))
            })
            .collect();
        for (process, from, mate) in heard {
            self.roster.hear(&process, from, mate, now);
        }
        self.roster.forget_quiet(now);
        // The plans heard this frame, whoever made them: which to obey is
        // decided against the view below.
        let plans: Vec<Plan> = cx
            .board
            .messages_on(PLAN_TOPIC)
            .filter_map(|m| serde_json::from_value(m.value.clone()).ok())
            .collect();
        // What another process's fleet view asked of this session. (The
        // fleet view here applies its own asks directly.)
        let asked: Vec<Request> = cx
            .board
            .messages_on(REQUEST_TOPIC)
            .filter(|m| m.is_remote())
            .filter_map(|m| Request::from_message(&m.value))
            .filter(|(process, s, _, _)| process == &me_name && *s == session)
            .map(|(_, _, _, r)| r)
            .collect();

        let Some(client) = cx.try_client() else {
            return;
        };
        for r in asked {
            r.apply(client);
            tracing::info!(request = r.as_str(), session, "fleet request applied");
        }
        if !client.autoplay.config.team.enabled {
            // Off the team: the rules see nobody, and the next time on
            // starts a round of its own.
            self.settling.remove(&session);
            if !client.autoplay.team.mates.is_empty() || client.autoplay.team.leader {
                client.autoplay.team = TeamView::default();
            }
            return;
        }
        let Some(me) = describe(client, session) else {
            return;
        };
        let due = self.say(session, &me, now);
        // Hand the rules what the others said, and what this session last
        // said about itself: the others read it by that, up to a board
        // round old, so the turns at a newly fallen body read it by that
        // too (see `Autoplay::ours_to_open`).
        let mut view = self.roster.view_for(&me);
        view.me = self.said(session).cloned();
        // Whether the roster behind it has stood a full round: what the
        // leader flag is worth to the fellowship rules.
        let names = view.mates.iter().map(|m| m.name.clone()).collect();
        view.settled = self.settling.entry(session).or_default().settle(names, now);
        if client.autoplay.team != view {
            client.autoplay.team = view;
            // A body one of them emptied and found nothing left on for
            // this character is not opened again. Taken in for good the
            // moment it is heard: the row it came on goes once its mate
            // has been quiet a while.
            client.take_in_shuts();
        }
        // The leader plans for the party once a round, and holds its own
        // orders; everyone else takes the orders of the leader it sees.
        // A plan from anyone else -- a leader since replaced, a fleet
        // this one is not on -- is left alone (see `ac_client::plan`).
        let plan = if client.autoplay.team.leader {
            (due && client.autoplay.team.settled && client.autoplay.config.enabled)
                .then(|| client.plan_for_team(now))
        } else {
            let leader = client.autoplay.team.leader_mate().map(|m| m.name.clone());
            for plan in plans {
                if Some(&plan.leader) == leader.as_ref() {
                    client.autoplay.take_orders(plan, now);
                }
            }
            None
        };
        // And say our piece, a few times a second.
        if due {
            if let Ok(value) = serde_json::to_value(&me) {
                cx.post(MATE_TOPIC, value);
            }
        }
        if let Some(plan) = plan {
            if let Ok(value) = serde_json::to_value(&plan) {
                cx.post(PLAN_TOPIC, value);
            }
        }
    }
}

/// For a panel or a script: the roster as JSON, everyone heard.
pub fn roster_json(team: &Team) -> Value {
    Value::Array(
        team.roster
            .heard
            .values()
            .filter_map(|h| serde_json::to_value(&h.mate).ok())
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mate(name: &str, guid: u32) -> Mate {
        Mate {
            name: name.into(),
            guid,
            ..Default::default()
        }
    }

    #[test]
    fn a_mate_that_says_it_has_a_body_is_left_to_it() {
        // The claim travels the same way everything else about a mate
        // does: said on the blackboard, heard into the roster, handed
        // to the rules. Everyone else's looting reads it and takes
        // another body (`TeamView::working`).
        let now = Instant::now();
        let body = 0x8000_0001;
        let me = mate("Reborn", 1);
        let theirs = Mate {
            looting: Some(body),
            looting_for: Duration::from_secs(2),
            health: 1.0,
            ..mate("Brynna", 2)
        };
        // Over the bus it goes as JSON, and it has to come back whole.
        let said = serde_json::to_value(&theirs).expect("a mate is JSON");
        let heard: Mate = serde_json::from_value(said).expect("and comes back");
        assert_eq!(heard.looting, Some(body));
        assert_eq!(heard.looting_for, theirs.looting_for);

        let mut r = Roster::default();
        r.hear("other", 0, heard.clone(), now);
        assert!(r.view_for(&me).working(body));
        assert!(!r.view_for(&me).working(0x8000_0002), "claimed every body");
        // Gone quiet: gone, and the body is anyone's again.
        r.forget_quiet(now + FORGET_AFTER + Duration::from_secs(1));
        assert!(!r.view_for(&me).working(body));

        // Dead over the body it had open: its client goes on saying so,
        // and the same row says its health is nothing. That is read
        // rather than waited out.
        let mut r = Roster::default();
        r.hear(
            "other",
            0,
            Mate {
                health: 0.0,
                ..heard
            },
            now,
        );
        assert!(
            !r.view_for(&me).working(body),
            "a dead claimant held a body"
        );
    }

    #[test]
    fn a_row_from_an_older_build_reads_with_no_skills_and_no_shuts() {
        // A process still on an older build says nothing about the skills
        // its loot rules ask about or the bodies it emptied. Its row still
        // reads: it asks about no skill, and has emptied nothing for
        // anyone.
        let old = serde_json::json!({"name": "Brannoc", "guid": 5, "health": 1.0});
        let heard: Mate = serde_json::from_value(old).expect("an older row still reads");
        assert!(heard.skills.is_empty() && heard.shut.is_empty());
        // Nor is it dealt bodies or left things on them, not having said
        // that it opens any.
        assert!(!heard.opens_bodies);

        // And what a newer one says comes through the roster whole.
        let said = Mate {
            skills: vec![(23, 280, 300, 2)],
            shut: vec![ac_client::autoplay::Shut {
                body: 0x8000_0001,
                n: 4,
                done_for: vec![1],
                stand_by: vec![3],
                left_for: vec![5],
            }],
            opens_bodies: true,
            ..mate("Brynna", 2)
        };
        let json = serde_json::to_value(&said).expect("a mate is JSON");
        let mut r = Roster::default();
        r.hear(
            "other",
            0,
            serde_json::from_value(json).expect("and comes back"),
            Instant::now(),
        );
        let v = r.view_for(&mate("Reborn", 1));
        assert_eq!(v.mates[0].skills, said.skills);
        assert_eq!(v.mates[0].shut, said.shut);
        assert!(v.mates[0].opens_bodies);
    }

    #[test]
    fn a_session_reads_itself_by_what_it_last_said_not_by_this_frame() {
        // The others read a session off the row it last put on the board,
        // up to half a second old. Read off this frame instead, a session
        // that had just shut a body dealt the next to someone else while
        // the rest dealt it to that session, and nobody opened it.
        let t0 = Instant::now();
        let mut team = Team::default();
        let first = Mate {
            opened_first: 2,
            ..mate("Reborn", 1)
        };
        let later = Mate {
            opened_first: 3,
            looting: Some(0x8000_0001),
            ..mate("Reborn", 1)
        };
        assert!(team.say(0, &first, t0), "said nothing at first");
        assert!(!team.say(0, &later, t0 + Duration::from_millis(100)));
        assert_eq!(
            team.said(0),
            Some(&first),
            "read itself ahead of what the others had heard"
        );
        assert!(team.say(0, &later, t0 + SAY_EVERY));
        assert_eq!(team.said(0), Some(&later));
        // Each session by its own word.
        assert_eq!(team.said(1), None);
    }

    #[test]
    fn a_session_takes_its_turn_by_what_it_said_about_itself() {
        // The turns at a newly fallen body read every mate off its row,
        // and a session reads itself off the same word, so that all of
        // them deal the body to the same one.
        let me = Mate {
            opened_first: 3,
            target: Some(0x77),
            ..mate("Reborn", 1)
        };
        let theirs = Mate {
            opened_first: 2,
            ..mate("Brynna", 2)
        };
        let json = serde_json::to_value(&theirs).expect("a mate is JSON");
        let mut r = Roster::default();
        r.hear(
            "other",
            0,
            serde_json::from_value(json).expect("and comes back"),
            Instant::now(),
        );
        let v = r.view_for(&me);
        assert_eq!(v.me.as_ref(), Some(&me));
        assert_eq!(v.mates[0].opened_first, 2);
        // A row from an older build has had no turns.
        let old = serde_json::json!({"name": "Brannoc", "guid": 5, "health": 1.0});
        let heard: Mate = serde_json::from_value(old).expect("an older row still reads");
        assert_eq!(heard.opened_first, 0);
    }

    #[test]
    fn the_first_name_leads_and_the_quiet_are_forgotten() {
        let now = Instant::now();
        let mut r = Roster::default();
        let me = mate("Reborn", 1);
        // Alone: I lead, and see nobody.
        let v = r.view_for(&me);
        assert!(v.leader && v.mates.is_empty());
        // Another process's session speaks, with a name that sorts first.
        r.hear("other", 0, mate("+Admin", 2), now);
        let v = r.view_for(&me);
        assert!(!v.leader);
        assert_eq!(v.mates.len(), 1);
        assert!(v.mates[0].leader, "the other is the leader");
        // My own word coming back is not a mate.
        r.hear("local", 0, me.clone(), now);
        assert_eq!(r.view_for(&me).mates.len(), 1);
        // The same session speaking again replaces, not adds.
        r.hear("other", 0, mate("+Admin", 2), now + Duration::from_secs(1));
        assert_eq!(r.len(), 2);
        // Gone quiet: gone, my own echo included.
        r.forget_quiet(now + FORGET_AFTER + Duration::from_secs(2));
        assert!(r.is_empty());
        assert!(r.view_for(&me).leader, "alone again, so leading again");
    }

    #[test]
    fn nine_sessions_coming_on_in_one_tick_have_one_settled_leader() {
        // The bug, measured: nine sessions ran the team-on line in the
        // same tick, and two of them founded a fellowship each, fourteen
        // milliseconds apart, before the first board round had carried a
        // word. Each led its roster of one. A view is acted on only once
        // it has settled, and then exactly one of the nine leads.
        let t0 = Instant::now();
        let frame = Duration::from_millis(16);
        let names = [
            "+Brynith", "+Brynlyn", "+Brynoth", "+Brynrun", "+Brynuth", "+Brynvor", "+Brynwyn",
            "+Brynna", "+Reborn",
        ];
        let mates: Vec<Mate> = names
            .iter()
            .enumerate()
            .map(|(i, n)| mate(n, 0x5000_0001 + i as u32))
            .collect();
        let mut rosters: Vec<Roster> = (0..9).map(|_| Roster::default()).collect();
        let mut settling: Vec<Settling> = (0..9).map(|_| Settling::default()).collect();
        // What each session's rules would act on this frame: leader, and
        // settled.
        let founders =
            |rosters: &[Roster], settling: &mut [Settling], now: Instant| -> Vec<String> {
                (0..9)
                    .filter(|&i| {
                        let v = rosters[i].view_for(&mates[i]);
                        let names = v.mates.iter().map(|m| m.name.clone()).collect();
                        v.leader && settling[i].settle(names, now)
                    })
                    .map(|i| names[i].to_string())
                    .collect()
            };
        // The tick they all come on: nobody has heard anybody, everyone
        // leads its own roster -- and nobody has settled.
        let leading = (0..9)
            .filter(|&i| rosters[i].view_for(&mates[i]).leader)
            .count();
        assert_eq!(leading, 9, "each led its roster of one");
        assert!(founders(&rosters, &mut settling, t0).is_empty());
        // Next frame everyone hears the eight others (local posts are
        // read at home the next frame). A changed roster starts the
        // round again.
        for (i, r) in rosters.iter_mut().enumerate() {
            for (j, m) in mates.iter().enumerate() {
                if i != j {
                    r.hear("local", j, m.clone(), t0 + frame);
                }
            }
        }
        assert!(founders(&rosters, &mut settling, t0 + frame).is_empty());
        assert!(founders(&rosters, &mut settling, t0 + frame + SAY_EVERY).is_empty());
        // A round later, one leader, the first name.
        assert_eq!(
            founders(&rosters, &mut settling, t0 + frame + SETTLE_AFTER),
            vec!["+Brynith".to_string()]
        );
    }

    #[test]
    fn a_roster_settles_once_its_names_have_stood_a_round() {
        let t0 = Instant::now();
        let mut s = Settling::default();
        // The first look starts the round.
        assert!(!s.settle(vec![], t0));
        assert!(!s.settle(vec![], t0 + SETTLE_AFTER / 2));
        // Alone for a round: settled, and it leads a roster of one for
        // as long as nobody comes.
        assert!(s.settle(vec![], t0 + SETTLE_AFTER));
        // Somebody comes: the round starts again, however they are
        // ordered.
        assert!(!s.settle(vec!["Zed".into(), "Alpha".into()], t0 + SETTLE_AFTER));
        assert!(!s.settle(
            vec!["Alpha".into(), "Zed".into()],
            t0 + SETTLE_AFTER * 2 - Duration::from_millis(1)
        ));
        assert!(s.settle(vec!["Alpha".into(), "Zed".into()], t0 + SETTLE_AFTER * 2));
        // And somebody going quiet starts it again too.
        assert!(!s.settle(vec!["Alpha".into()], t0 + SETTLE_AFTER * 2));
    }

    #[test]
    fn requests_round_trip_through_their_messages() {
        for r in Request::ALL {
            assert_eq!(Request::parse(r.as_str()), Some(r));
            let m = r.message("bob", 2, "Brannoc");
            assert_eq!(
                Request::from_message(&m),
                Some(("bob".to_string(), 2, "Brannoc".to_string(), r))
            );
        }
        assert_eq!(Request::parse("dance"), None);
        assert_eq!(
            Request::from_message(&serde_json::json!({"process": "bob", "action": "stop"})),
            None,
            "a request names its session"
        );
        // The word comes from the roster, whichever process spoke.
        let mut r = Roster::default();
        r.hear("bob", 1, mate("Brannoc", 5), Instant::now());
        let heard: Vec<_> = r.iter().collect();
        assert_eq!(heard.len(), 1);
        assert_eq!(
            (heard[0].0, heard[0].1, heard[0].2.name.as_str()),
            ("bob", 1, "Brannoc")
        );
    }

    #[test]
    fn a_session_takes_the_plan_of_the_leader_it_sees_and_no_other() {
        // The plan crosses the board as JSON like everything else, and
        // is obeyed only while it is fresh and signed by the leader the
        // roster names: a plan from a leader since replaced, or from a
        // leader gone quiet, orders nobody about.
        let t0 = Instant::now();
        let mut plan = Plan {
            leader: "+Admin".into(),
            n: 1,
            ..Default::default()
        };
        plan.orders.insert(
            2,
            ac_client::plan::Order {
                target: Some(0x77),
                body: None,
            },
        );
        let json = serde_json::to_value(&plan).expect("a plan is JSON");
        let back: Plan = serde_json::from_value(json).expect("and comes back");
        assert_eq!(back, plan);

        let mut r = Roster::default();
        let me = mate("Reborn", 2);
        r.hear("other", 0, mate("+Admin", 1), t0);
        r.hear("other", 1, mate("Zed", 3), t0);
        let v = r.view_for(&me);
        assert_eq!(v.leader_mate().map(|m| m.name.as_str()), Some("+Admin"));
        let mut ap = ac_client::autoplay::Autoplay::default();
        ap.config.team.enabled = true;
        ap.team = v;
        ap.take_orders(
            Plan {
                leader: "Zed".into(),
                ..plan.clone()
            },
            t0,
        );
        assert_eq!(ap.order_for(2, t0), None, "a plan from one not leading");
        ap.take_orders(plan, t0);
        assert_eq!(ap.order_for(2, t0).and_then(|o| o.target), Some(0x77));
        assert_eq!(
            ap.order_for(2, t0 + ac_client::plan::ORDERS_LAST),
            None,
            "the leader gone quiet"
        );
    }

    #[test]
    fn the_team_view_follows_the_leaders_target() {
        let now = Instant::now();
        let mut r = Roster::default();
        let me = mate("Zed", 1);
        let mut boss = mate("Alpha", 2);
        boss.target = Some(0x77);
        boss.target_name = "Drudge".into();
        r.hear("p", 0, boss, now);
        let v = r.view_for(&me);
        assert!(!v.leader);
        assert_eq!(v.target(), Some((0x77, "Drudge".into())));
    }
}
