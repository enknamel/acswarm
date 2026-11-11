//! A town run's calls on the NPCs its give rules name (`LootAction::Give`): the nearest taker
//! (`ac_world::takers`) or an NPC by name. One joins a run setting off for a counter when the NPC
//! stands in a counter's landblock or a short leg from one, as a giver does (`plan_calls`); no run
//! starts for one. There the items tagged to it are handed over by `Client::give_items`.

use std::time::Instant;

use ac_world::landmarks::{Kind, Landmark};
use ac_world::Place;
use glam::Vec2;

use super::calls::{Call, CallOn, GIVER_TRIES};
use super::{Phase, Run, Turn};
use crate::autoplay::growth::Growth;
use crate::autoplay::Doing;
use crate::giving::GIVE_REACH;
use crate::profile::GiveTo;
use crate::quest_log::SHORT_LEG;
use crate::Client;

/// What a gift call is for: the rule's receiver, and the NPC chosen for it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GiftCall {
    pub(crate) to: GiveTo,
    /// The NPC's name as the data lists it.
    pub(crate) name: String,
    /// A taker's weenie class, what it is known by in view; `None` for an NPC named by the player.
    pub(crate) wcid: Option<u32>,
}

/// The NPC `to` names, at its placement nearest `from` (world xy): a taker from
/// `ac_world::takers`, a named NPC from the landmarks; none for a player.
pub(crate) fn receiver_place(to: &GiveTo, from: Vec2) -> Option<(GiftCall, Place)> {
    match to {
        GiveTo::Taker => ac_world::takers::nearest(from, |_| true).map(|t| {
            let gift = GiftCall {
                to: to.clone(),
                name: t.name.clone(),
                wcid: Some(t.wcid),
            };
            (gift, t.place)
        }),
        GiveTo::Npc(name) => {
            let l = ac_world::landmarks::named(name, from).filter(|l| l.kind == Kind::Npc)?;
            let gift = GiftCall {
                to: to.clone(),
                name: l.name.clone(),
                wcid: None,
            };
            Some((
                gift,
                Place {
                    at: l.at,
                    cell: l.cell,
                },
            ))
        }
        GiveTo::Player(_) => None,
    }
}

impl Client {
    /// The run that just set off for its counter takes a call on each NPC its Give-tagged items
    /// name, when that NPC is on its round; the rest wait, carried, for a run that passes.
    pub(super) fn add_gift_calls(&mut self, now: Instant) {
        let Some(run) = self.autoplay.growth.run.as_ref().filter(|r| !r.for_quest) else {
            return;
        };
        let mut counters = vec![super::vendor::counter_place(run.at)];
        counters.extend(run.calls.iter().filter_map(|c| match c.on {
            CallOn::Counter { .. } => Some(c.at),
            _ => None,
        }));
        let from = run.at;
        let mut receivers: Vec<GiveTo> = Vec::new();
        for (_, to) in self.gifts_tagged(|t| !matches!(t, GiveTo::Player(_))) {
            if !receivers.contains(&to) {
                receivers.push(to);
            }
        }
        let mut calls = Vec::new();
        for to in receivers {
            let Some((gift, at)) = receiver_place(&to, from) else {
                self.autoplay
                    .note(format!("no {} is known to give to", to.tell()), now);
                continue;
            };
            if !self.is_on_round(&counters, at) {
                let why = format!("gifts for {} wait for a run that passes", gift.name);
                self.autoplay.note(why, now);
                continue;
            }
            calls.push(Call {
                at,
                on: CallOn::Gift(gift),
            });
        }
        if calls.is_empty() {
            return;
        }
        let names: Vec<String> = calls
            .iter()
            .filter_map(|c| match &c.on {
                CallOn::Gift(g) => Some(g.name.clone()),
                _ => None,
            })
            .collect();
        if let Some(run) = self.autoplay.growth.run.as_mut() {
            run.calls.extend(calls);
            let said = format!("the run gives to {} too", names.join(", "));
            self.autoplay.note(said, now);
        }
    }

    /// Whether `at` stands in a counter's landblock, or a short leg from one by the planner.
    fn is_on_round(&mut self, counters: &[Place], at: Place) -> bool {
        if counters.iter().any(|c| c.cell >> 16 == at.cell >> 16) {
            return true;
        }
        let to = self.planning_place(at);
        counters.iter().any(|&c| {
            let from = self.planning_place(c);
            self.leg_seconds(from, to).is_some_and(|s| s <= SHORT_LEG)
        })
    }

    /// Make `gift` the run's call in hand and set off for it. False when there is no way.
    pub(super) fn make_gift_call(
        &mut self,
        run: &mut Run,
        gift: GiftCall,
        at: Place,
        now: Instant,
    ) -> bool {
        run.vendor = gift.name.clone();
        let ok = self.visit_receiver(&gift, at);
        if ok {
            let said = format!("{}: to {} to give", run.reason, gift.name);
            self.autoplay.say(Doing::Shopping, said);
        } else {
            self.autoplay
                .note(format!("no way to {} to give", gift.name), now);
        }
        run.phase = Phase::Gift {
            gift,
            at,
            tries: 1,
            handing: false,
        };
        ok
    }

    fn visit_receiver(&mut self, gift: &GiftCall, at: Place) -> bool {
        let l = Landmark {
            kind: Kind::Npc,
            name: gift.name.clone(),
            cell: at.cell,
            at: at.at,
        };
        self.visit_landmark(&l)
    }

    /// The NPC of `gift` in view within a give's reach: a taker by weenie class, a named NPC by
    /// the name the player typed.
    fn receiver_in_reach(&self, gift: &GiftCall) -> Option<u32> {
        let me = self.my_position()?;
        let mine = self.world.player_guid;
        self.world
            .objects
            .values()
            .filter(|o| Some(o.guid) != mine && o.container.is_none() && !o.is_player)
            .filter(|o| match gift.wcid {
                Some(w) => o.weenie_class_id == w,
                None => o.name.eq_ignore_ascii_case(&gift.name),
            })
            .filter_map(|o| Some((o.guid, o.world_pos()?.distance(me))))
            .filter(|(_, d)| *d <= GIVE_REACH)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(g, _)| g)
    }

    /// One turn of a gift call: the visit there, then the gives, then on to the next call.
    pub(super) fn gift_step(&mut self, mut run: Run, now: Instant, cfg: &Growth) -> Turn {
        let Phase::Gift {
            gift,
            at,
            tries,
            handing,
        } = run.phase.clone()
        else {
            self.autoplay.growth.run = Some(run);
            return Turn::Waited;
        };
        if handing {
            if self.giving.is_some() {
                let said = format!("{}: giving to {}", run.reason, gift.name);
                self.autoplay.say(Doing::Shopping, said);
                self.autoplay.growth.run = Some(run);
                return Turn::Waited;
            }
            self.gave = None;
            return self.giver_done(run, now, cfg);
        }
        if self.visits.current.is_some() || self.giving.is_some() {
            let said = format!("{}: going to {} to give", run.reason, gift.name);
            self.autoplay.say(Doing::Shopping, said);
            self.autoplay.growth.run = Some(run);
            return Turn::Waited;
        }
        if let Some(npc) = self
            .receiver_in_reach(&gift)
            .filter(|_| self.visits.finished)
        {
            let items: Vec<u32> = self
                .gifts_tagged(|t| *t == gift.to)
                .into_iter()
                .map(|(g, _)| g)
                .collect();
            if !self.give_items(npc, items) {
                let said = format!("nothing left to give {}", gift.name);
                self.autoplay.note(said, now);
                return self.giver_done(run, now, cfg);
            }
            run.phase = Phase::Gift {
                gift,
                at,
                tries,
                handing: true,
            };
            self.autoplay.growth.run = Some(run);
            return Turn::Acted;
        }
        // A fight on the way broke the visit off: the run picks it up after.
        if self.attack_target.is_some() || self.autoplay.casting_at().is_some() {
            self.autoplay.growth.run = Some(run);
            return Turn::Waited;
        }
        if tries >= GIVER_TRIES || !self.visit_receiver(&gift, at) {
            let said = format!("could not get to {} to give", gift.name);
            self.autoplay.note(said, now);
            return self.giver_done(run, now, cfg);
        }
        run.phase = Phase::Gift {
            gift,
            at,
            tries: tries + 1,
            handing: false,
        };
        self.autoplay.growth.run = Some(run);
        Turn::Acted
    }
}

#[cfg(test)]
mod tests;
