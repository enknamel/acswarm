//! A journey's NPC step: walked to, handed its item or used, then through the portal it summons
//! in front of it or carried off (a teleport): Faladha the Emissary's portal to Aerlinthe for a
//! 10,000 note (`portals::npc_ways`). Objects it asks used first are walked to and used before it
//! (Browerk's Eyes). Retries and give-up are a gem's (`gem_room::gem_next`), after its emotes.

use std::time::{Duration, Instant};

use ac_world::trip::Step;
use ac_world::Place;

use crate::gem_room::{gem_next, GemNext};
use crate::{Client, Goal};

/// The walk to the NPC ends this near it (metres, 3D): inside the server's use reach.
const NPC_STOP: f32 = 1.5;
/// The NPC is looked for this near its spot in the data (metres): it may wander a little.
const NPC_NEAR: f32 = 20.0;

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
            self.travel_npc_refused(step, "is not where it stands");
            return None;
        };
        if first {
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

    /// The step's next object to use before its NPC (`NpcStep::first`): walked to, then used once
    /// (each starts a server event, so a use more is harmless); `None` once all are used.
    fn npc_first_use(
        &mut self,
        step: &NpcStep,
        me: glam::Vec3,
        now: Instant,
    ) -> Option<Option<Goal>> {
        let used = &self.travel.state.firsts_used;
        let &(wcid, place) = step.first.iter().find(|(w, _)| !used.contains(w))?;
        if me.distance(place.at) > crate::visit::REACH {
            if !self.is_in_region(place) {
                return Some(self.overland_goal(place.at.truncate()));
            }
            return Some(Some(Goal::At {
                place,
                within: NPC_STOP,
            }));
        }
        let Some(guid) = self.npc_near(wcid, place) else {
            self.travel_npc_refused(step, "has something to use first that is not there");
            return Some(None);
        };
        if self.hands_busy(now) {
            return Some(None);
        }
        tracing::info!("travel: using {wcid} before {}", step.name);
        self.session
            .send_action(ac_net::messages::action::USE, &guid.to_le_bytes());
        self.autoplay.cast_sent = Some(now);
        self.travel.state.firsts_used.push(wcid);
        Some(None)
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
