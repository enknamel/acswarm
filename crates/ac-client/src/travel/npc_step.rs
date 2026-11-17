//! A journey's NPC step: walked to, handed its item or used, then through the portal it summons
//! in front of it or carried off (a teleport): Faladha the Emissary's portal to Aerlinthe for a
//! 10,000 note (`portals::npc_ways`). Objects it asks used first are walked to and used before it,
//! in order (Browerk's Eyes; Kivik Lir's, whose stamps hold 30 s). Retries and give-up are a gem's
//! (`gem_room::gem_next`), after its emotes.

use std::time::{Duration, Instant};

use ac_world::trip::Step;
use ac_world::Place;

use crate::gem_room::{gem_next, GemNext};
use crate::{Client, Goal};

/// The walk to the NPC ends this near it (metres, 3D): inside the server's use reach.
const NPC_STOP: f32 = 1.5;
/// The NPC is looked for this near its spot in the data (metres): it may wander a little.
const NPC_NEAR: f32 = 20.0;
/// An object used before the NPC is walked to this near its spot in the data (metres, 3D),
/// there to be seen; then into its own use reach ([`Client::use_reach`]).
const FIRST_REACH: f32 = 2.0;
/// ACE's use radius when an object sends none (metres, edge to edge; WorldObject_Use.cs:50).
const DEFAULT_USE_RADIUS: f32 = 0.6;
/// The character's radius when its Setup is unknown (metres, under a human's).
const BODY_RADIUS: f32 = 0.3;
/// A use counts only stood this much past its reach at its UseDone (metres, 3D): the server's
/// walk the rest of the way fails silently, flying (a_use_the_server_walked_short_of_is_sent_again).
const USED_SLACK: f32 = 0.5;
/// The NPC is waited for this long stood at its spot before it counts as not there (guess: the
/// server's view of a flown body catching up, ObjectMaint's cell visibility).
pub(super) const NPC_LOOK: Duration = Duration::from_secs(8);
/// A use of an object before the NPC is answered within this, else sent again (guess): a walk
/// off before its UseDone cancels the server's walk to it (Player_Move2.cs:172).
const FIRST_ANSWER: Duration = Duration::from_secs(10);

/// The NPC used after its objects has carried us off within this, else they are used again
/// (guess): its teleport lands in under a second, and a use cut short has no answer of its own.
pub(super) const FIRSTS_AGAIN: Duration = Duration::from_secs(4);
/// Rounds of the objects asked first before the NPC's way is given up (guess).
pub(super) const FIRST_ROUNDS: u32 = 3;

/// What an NPC step is: its NPC, what it is handed, whether it summons, and its emotes' delay.
struct NpcStep<'a> {
    npc: u32,
    name: &'a str,
    item: Option<u32>,
    summons: bool,
    secs: f32,
    first: &'a [(u32, Place)],
    mouth: Place,
}

impl Client {
    /// The NPC step stood at: walked to the NPC, then acted on and waited out as a gem is.
    pub(super) fn travel_npc_wait(&mut self, step: &Step, now: Instant) -> Option<Goal> {
        let Step::Npc {
            npc,
            name,
            item,
            portal,
            secs,
            first,
            mouth,
            ..
        } = step
        else {
            return None;
        };
        let step = NpcStep {
            npc: *npc,
            name,
            item: *item,
            summons: portal.is_some(),
            secs: *secs,
            first,
            mouth: *mouth,
        };
        self.npc_step(&step, now)
    }

    fn npc_step(&mut self, step: &NpcStep, now: Instant) -> Option<Goal> {
        let me = self.my_position()?;
        let first = self.travel.state.gem_used.is_none();
        if first {
            if let Some(next) = self.npc_first_use(step, me, now) {
                return next;
            }
        }
        if first && me.distance(step.mouth.at) > crate::visit::REACH {
            if !self.is_in_region(step.mouth) {
                return self.overland_goal(step.mouth.at.truncate());
            }
            return Some(Goal::At {
                place: step.mouth,
                within: NPC_STOP,
            });
        }
        let Some(guid) = self.npc_near(step.npc, step.mouth) else {
            // The server sends an object only once its cell is in view: a body flown through the
            // walls stands there first (Kivik Lir's Guardian, given up 3.4 s into the Halls).
            let since = *self.travel.state.npc_unseen.get_or_insert(now);
            if now.duration_since(since) >= NPC_LOOK {
                self.travel_npc_refused(step, "is not where it stands");
            }
            return None;
        };
        self.travel.state.npc_unseen = None;
        if first {
            if let Some(goal) = self.approach_use(guid) {
                return Some(goal);
            }
            if self.hands_busy(now) {
                return None;
            }
            tracing::info!("travel: to {} for its way on", step.name);
            self.travel.state.portals_before = self.portals_in_reach();
            self.travel.state.portals_at = self.my_position().map(|p| p.truncate());
            self.travel.state.gem_used = Some(now);
            self.travel.state.gem_uses = 1;
            self.travel.state.gem_stack = step.item.map_or(0, |i| self.carried_count(i));
            self.npc_act(guid, step.item, now);
            return None;
        }
        let used = self.travel.state.gem_used.unwrap_or(now);
        let since_use = now.duration_since(used);
        // Its emotes run their delays first: they are no part of a wait for the way.
        let since_way = since_use.saturating_sub(Duration::from_secs_f32(step.secs.max(0.0)));
        let before = self.travel.state.portals_before.clone();
        let portal = step
            .summons
            .then(|| self.summoned_portal(&before))
            .flatten();
        let since_portal = self
            .travel
            .state
            .summoned_used
            .map(|t| now.duration_since(t));
        let kept = self.travel.state.gem_stack.max(1);
        let untaken = step.item.is_some_and(|i| self.carried_count(i) >= kept);
        let next = gem_next(
            untaken,
            step.summons,
            if untaken { since_use } else { since_way },
            self.travel.state.gem_uses,
            portal,
            since_portal,
            self.travel.state.summoned_uses,
        );
        // Not carried off: an object used first may not have taken (the Portal Device "too weak",
        // an Eye's walk cut short), so each is used again (a_device_still_unfuelled_has_its_eyes_used_again).
        let teleports = !step.summons && step.item.is_none() && !step.first.is_empty();
        if teleports && since_way >= FIRSTS_AGAIN && self.travel.state.first_rounds < FIRST_ROUNDS {
            tracing::info!(
                "travel: {} opened no way; what it asks used first again",
                step.name
            );
            self.travel.state.first_rounds += 1;
            self.travel.state.firsts_used.clear();
            self.travel.state.gem_used = None;
            return None;
        }
        match next {
            GemNext::Wait => {}
            GemNext::UseGem | GemNext::UsePortal(_) if self.hands_busy(now) => {}
            GemNext::UseGem => {
                tracing::info!("travel: {} did not take it; again", step.name);
                self.travel.state.gem_uses += 1;
                self.travel.state.gem_used = Some(now);
                self.npc_act(guid, step.item, now);
            }
            GemNext::UsePortal(portal) => {
                tracing::info!("travel: using the portal {} summoned", step.name);
                self.travel.state.summoned_uses += 1;
                self.travel.state.summoned_used = Some(now);
                self.session
                    .send_action(ac_net::messages::action::USE, &portal.to_le_bytes());
                self.hop_sent(now, crate::autoplay::cast::HOP_LANDS);
            }
            GemNext::GiveUp => self.travel_npc_refused(step, "opened no way"),
        }
        None
    }

    /// The step's next object to use before its NPC (`NpcStep::first`): walked to, used, and stood
    /// at till the server answers (each starts a server event or stamps a flag the NPC asks held,
    /// so a use more is harmless); `None` once all are used.
    fn npc_first_use(
        &mut self,
        step: &NpcStep,
        me: glam::Vec3,
        now: Instant,
    ) -> Option<Option<Goal>> {
        if let Some((sent, guid)) = self.travel.state.first_sent {
            // UseDone comes whether the server's walk to it got there or not (Player_Use.cs:223-238):
            // a cast or a sidestep since cut that walk, so the use is sent again.
            let cut = self.last_used != Some(guid) || self.stepped_aside.is_some_and(|t| t >= sent);
            // Nor does it say whether the server's walk got there: a use is had only standing at it.
            let reach = self.use_reach(guid) + USED_SLACK;
            let away = (self.world.objects.get(&guid))
                .and_then(|o| o.world_pos())
                .is_none_or(|at| me.distance(at) > reach);
            match self.use_done.filter(|(_, at)| *at >= sent) {
                Some((err, _)) => {
                    self.travel.state.first_sent = None;
                    if cut || err != 0 || away {
                        self.travel.state.firsts_used.pop();
                    }
                }
                None if now.duration_since(sent) < FIRST_ANSWER => return Some(None),
                None => {
                    self.travel.state.first_sent = None;
                    self.travel.state.firsts_used.pop();
                }
            }
        }
        let used = &self.travel.state.firsts_used;
        let &(wcid, place) = step.first.iter().find(|(w, _)| !used.contains(w))?;
        if me.distance(place.at) > FIRST_REACH {
            if !self.is_in_region(place) {
                return Some(self.overland_goal(place.at.truncate()));
            }
            return Some(Some(Goal::At {
                place,
                within: NPC_STOP,
            }));
        }
        let Some(guid) = self.npc_near(wcid, place) else {
            // Not sent yet, as the NPC may not be (`NPC_LOOK`).
            let since = *self.travel.state.npc_unseen.get_or_insert(now);
            if now.duration_since(since) >= NPC_LOOK {
                self.travel_npc_refused(step, "has something to use first that is not there");
            }
            return Some(None);
        };
        self.travel.state.npc_unseen = None;
        if let Some(goal) = self.approach_use(guid) {
            return Some(Some(goal));
        }
        if self.hands_busy(now) {
            return Some(None);
        }
        tracing::info!("travel: using {wcid} before {}", step.name);
        self.session
            .send_action(ac_net::messages::action::USE, &guid.to_le_bytes());
        self.autoplay.cast_sent = Some(now);
        self.last_used = Some(guid);
        self.travel.state.firsts_used.push(wcid);
        self.travel.state.first_sent = Some((now, guid));
        Some(None)
    }

    /// How near `guid`'s centre a use of it is sent from (metres, 3D): its use radius plus both
    /// bodies' radii (Setup radius × scale), as ACE's cylinder distance (WorldObject_Use.cs:47-60).
    pub(crate) fn use_reach(&self, guid: u32) -> f32 {
        let radius = |g: u32| {
            let o = self.world.objects.get(&g)?;
            let setup = self.assets.setup(o.setup_id).ok()?;
            Some(setup.radius * if o.scale > 0.0 { o.scale } else { 1.0 })
        };
        let me = self
            .world
            .player_guid
            .and_then(radius)
            .unwrap_or(BODY_RADIUS);
        let it = radius(guid).unwrap_or(0.0);
        let use_radius = self.world.objects.get(&guid).and_then(|o| o.use_radius);
        use_radius.unwrap_or(DEFAULT_USE_RADIUS) + me + it
    }

    /// The walk into `guid`'s use reach, landed, before it is used; `None` stood there, on the
    /// floor: the server walks a body no further when it is flying (Xi Ru's Eyes, live).
    fn approach_use(&mut self, guid: u32) -> Option<Goal> {
        let o = self.world.objects.get(&guid)?;
        let (at, cell) = (o.world_pos()?, o.position.as_ref()?.cell);
        let reach = self.use_reach(guid);
        let me = self.my_position()?;
        if me.distance(at) > reach {
            return Some(Goal::At {
                place: Place { at, cell },
                within: reach * 0.8,
            });
        }
        if self.noclip() && self.set_noclip(false) {
            tracing::info!("travel: landing to use {guid:#010x}");
        }
        None
    }

    /// The NPC of weenie class `npc` in view nearest its spot `at`, within [`NPC_NEAR`].
    fn npc_near(&self, npc: u32, at: Place) -> Option<u32> {
        self.world
            .objects
            .values()
            .filter(|o| o.weenie_class_id == npc)
            .filter_map(|o| Some((o.guid, o.world_pos()?.distance(at.at))))
            .filter(|(_, d)| *d <= NPC_NEAR)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(g, _)| g)
    }

    /// Hand the NPC `guid` the carried `item`, or use it with none: sent straight to the server,
    /// as a summoned portal's use is, since this act is the journey.
    fn npc_act(&mut self, guid: u32, item: Option<u32>, now: Instant) {
        match item {
            Some(wcid) => {
                let held = self
                    .world
                    .inventory()
                    .find(|o| o.weenie_class_id == wcid)
                    .map(|o| o.guid);
                if let Some(held) = held {
                    self.give(guid, held, Some(1));
                }
            }
            None => self
                .session
                .send_action(ac_net::messages::action::USE, &guid.to_le_bytes()),
        }
        self.autoplay.cast_sent = Some(now);
    }

    /// The NPC step's way did not open: left out of the journey's next plan.
    fn travel_npc_refused(&mut self, step: &NpcStep, why: &str) {
        tracing::warn!("travel: {} {why}; going another way", step.name);
        self.travel.refused_npcs.push(step.npc);
        self.travel.replan_due = true;
    }

    /// Whether the NPC way of `npc` failed on this journey (`refused_npcs`).
    pub(crate) fn is_npc_way_refused(&self, npc: u32) -> bool {
        self.travel.refused_npcs.contains(&npc)
    }
}
