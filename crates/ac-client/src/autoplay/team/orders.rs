use std::time::Instant;

use super::shuts::DEAL_WINDOW;
use super::turns::CLAIM_STALE;
use crate::autoplay::{corpse_within_reach, Autoplay, Doing, Fight};
use crate::Client;

impl Autoplay {
    /// Take in the leader's plan, heard at `now`.
    pub fn take_orders(&mut self, plan: crate::plan::Plan, now: Instant) {
        self.orders = Some(crate::plan::Orders { plan, heard: now });
    }

    /// The plan this character is under, while it is fresh (see
    /// `crate::plan::ORDERS_LAST`) and from whoever leads the team as
    /// this character now sees it. A plan from a leader since replaced,
    /// or from before the team was left, is nobody's to obey.
    pub fn current_plan(&self, now: Instant) -> Option<&crate::plan::Plan> {
        if !self.config.team.enabled {
            return None;
        }
        let plan = self.orders.as_ref()?.current(now)?;
        let leader = if self.team.leader {
            self.team.me.as_ref().map(|m| m.name.as_str())
        } else {
            self.team.leader_mate().map(|m| m.name.as_str())
        };
        (leader == Some(plan.leader.as_str())).then_some(plan)
    }

    /// This character's orders under the plan, `me` being its player guid.
    pub fn order_for(&self, me: u32, now: Instant) -> Option<crate::plan::Order> {
        self.current_plan(now)?.order_for(me)
    }

    /// Whom the plan deals the body `body` to, if it is fresh and deals it.
    pub fn body_dealt_to(&self, body: u32, now: Instant) -> Option<u32> {
        self.current_plan(now)?.body_dealt_to(body)
    }

    /// A body this character, leading, dealt to one of the others and
    /// that still lies there (`there`) unemptied: `(body, whom)`. The
    /// leader's next chosen fight waits on it the way its own kill's body
    /// holds it (see `Client::waits_for_a_corpse`).
    ///
    /// Measured without this: the deal gave each body to one hand, so
    /// the leader owed none and walked off to the next creature the
    /// moment one fell, the party followed it, and the hand dealt the
    /// body was left forty metres behind or gave the body up. The
    /// party walked twice as far and took a quarter of the loot. Only
    /// while the plan is fresh: a leader that has stopped planning holds
    /// nothing for a deal it is no longer making.
    pub fn body_dealt_to_another(
        &self,
        me: u32,
        now: Instant,
        there: impl Fn(u32) -> bool,
    ) -> Option<(u32, u32)> {
        self.current_plan(now)?;
        self.planner
            .deals
            .iter()
            .filter(|(body, to, _)| *to != me && there(*body))
            .filter(|(body, _, _)| !self.looted.contains(body) && !self.shut_by_anyone(*body))
            .map(|(body, to, _)| (*body, *to))
            .min()
    }
}

impl Client {
    /// The party as the planner reads it: this character first, from
    /// what it knows of itself, then each mate from its row on the board
    /// (see `crate::plan::Hand`).
    fn hands_of_team(&self, now: Instant, with_me: bool) -> Vec<crate::plan::Hand> {
        let alive = |g: u32| {
            self.world
                .objects
                .get(&g)
                .is_some_and(|o| o.alive_or_unknown())
        };
        let me = self.world.player_guid.unwrap_or(0);
        let mine = self.my_position();
        let mut hands = Vec::with_capacity(self.autoplay.team.mates.len() + 1);
        if let Some(mine) = mine.filter(|_| me != 0 && with_me) {
            let supplies = self.supplies(&self.autoplay.config.growth, now);
            hands.push(crate::plan::Hand {
                guid: me,
                name: self.world.stats.name.clone(),
                world: mine,
                health: self.health_fraction(),
                fights: self.autoplay.config.enabled,
                target: self
                    .attack_target
                    .or(self.autoplay.casting_at())
                    .filter(|g| alive(*g)),
                looting: self.autoplay.corpse_claim(now).map(|(g, _)| g),
                opens: self.autoplay.config.enabled
                    && self.opens_bodies()
                    && !supplies.pack_full
                    && !supplies.laden,
                following: false,
                turns: self.autoplay.planner.turns_of(me, now, DEAL_WINDOW),
                hit_by: self.attackers_lately(),
            });
        }
        for m in &self.autoplay.team.mates {
            if m.guid == 0 {
                continue;
            }
            hands.push(crate::plan::Hand {
                guid: m.guid,
                name: m.name.clone(),
                world: m.world,
                health: m.health,
                fights: m.autoplay && m.health > 0.0,
                target: m.target,
                looting: m.looting.filter(|_| m.looting_for < CLAIM_STALE),
                opens: m.turn().is_some_and(|t| t.room),
                following: m.following,
                turns: self.autoplay.planner.turns_of(m.guid, now, DEAL_WINDOW),
                hit_by: m.hit_by.clone(),
            });
        }
        hands
    }

    /// The leader's plan for the party (see `crate::plan`): who fights
    /// what, whose turn each body is, and whom the leader is waiting
    /// for. Made from what the leader sees of the field and what the
    /// others have said on the board, and kept as the leader's own orders
    /// too. The team plugin asks for it once a board round and posts it.
    pub fn plan_for_team(&mut self, now: Instant) -> crate::plan::Plan {
        let cfg = self.autoplay.config.fight.clone();
        let hands = self.hands_of_team(now, true);
        let me = self.my_position();
        let underground = self.underground();
        // The creatures the party would fight, within the leader's own
        // radius and the one a follower fights within of its leader
        // (`Team::fight_radius`): further off, an order would draw a
        // follower away just as picking for itself would have.
        let within = cfg
            .radius
            .min(self.autoplay.config.team.fight_radius.max(1.0));
        let named: Vec<(u32, String, glam::Vec3)> = self
            .world
            .objects
            .values()
            .filter(|o| self.would_fight(o, &cfg, underground, now))
            .filter_map(|o| {
                let at = o.world_pos()?;
                (me.is_none_or(|m| at.distance(m) <= within)).then(|| (o.guid, o.name.clone(), at))
            })
            .collect();
        let by_name: Vec<(u32, &str, glam::Vec3)> = named
            .iter()
            .map(|(g, n, at)| (*g, n.as_str(), *at))
            .collect();
        let hunted: Vec<(u32, Vec<u32>)> = hands
            .iter()
            .map(|h| (h.guid, crate::plan::hunted_by(&by_name, h)))
            .collect();
        let foes: Vec<crate::plan::Foe> = named
            .iter()
            .map(|(guid, _, at)| {
                let mut after: Vec<u32> = self
                    .world
                    .objects
                    .get(guid)
                    .and_then(|o| o.walked_at)
                    .filter(|w| hands.iter().any(|h| h.guid == *w))
                    .into_iter()
                    .collect();
                after.extend(
                    hunted
                        .iter()
                        .filter(|(_, foes)| foes.contains(guid))
                        .map(|(h, _)| *h),
                );
                after.sort_unstable();
                after.dedup();
                crate::plan::Foe {
                    guid: *guid,
                    world: *at,
                    hard: self.is_hard_fight(*guid),
                    after,
                }
            })
            .collect();
        // The bodies nobody has opened: not emptied by this character,
        // not shut by any of the others, and no player's remains.
        let bodies: Vec<crate::plan::Body> = self
            .world
            .objects
            .values()
            .filter(|o| o.object_desc_flags & ac_world::object_desc_flags::CORPSE != 0)
            .filter(|o| !self.corpse_is_someone_elses(&o.name))
            .filter(|o| !self.autoplay.looted.contains(&o.guid))
            .filter(|o| !self.autoplay.shut_by_anyone(o.guid))
            .filter_map(|o| {
                Some(crate::plan::Body {
                    guid: o.guid,
                    world: o.world_pos()?,
                })
            })
            .collect();
        // What each was last told to fight, so that nobody is moved for
        // nothing (see `plan::assign_targets`).
        let prior: std::collections::BTreeMap<u32, u32> = self
            .autoplay
            .current_plan(now)
            .map(|p| {
                p.orders
                    .iter()
                    .filter_map(|(who, o)| Some((*who, o.target?)))
                    .collect()
            })
            .unwrap_or_default();
        let targets = crate::plan::assign_targets(&hands, &foes, &prior);
        let standing = self.autoplay.planner.standing(now);
        let passed = self.autoplay.planner.passed();
        let dealt =
            crate::plan::deal_bodies(&hands, &bodies, &standing, &passed, corpse_within_reach);
        for (body, to) in &dealt.lapsed {
            let who = hands
                .iter()
                .find(|h| h.guid == *to)
                .map(|h| h.name.clone())
                .unwrap_or_default();
            self.autoplay.note(
                format!("{who} did not come for the body {body:#010x}; dealing it on"),
                now,
            );
        }
        self.autoplay.planner.dealt(&dealt, now, DEAL_WINDOW);
        let deals = dealt.deals;
        self.autoplay.planner.n = self.autoplay.planner.n.wrapping_add(1);
        let mut orders: std::collections::BTreeMap<u32, crate::plan::Order> =
            std::collections::BTreeMap::new();
        for h in &hands {
            let order = crate::plan::Order {
                target: targets.get(&h.guid).copied(),
                body: deals
                    .iter()
                    .find(|(_, to)| *to == h.guid)
                    .map(|(body, _)| *body),
            };
            if order != crate::plan::Order::default() {
                orders.insert(h.guid, order);
            }
        }
        let waiting_for = match (self.autoplay.straggling_since, me) {
            (Some(_), Some(mine)) => {
                crate::plan::stragglers(mine, self.autoplay.config.team.follow_distance, &hands)
                    .into_iter()
                    .map(|s| s.name)
                    .collect()
            }
            _ => Vec::new(),
        };
        let plan = crate::plan::Plan {
            leader: self.world.stats.name.clone(),
            n: self.autoplay.planner.n,
            orders,
            waiting_for,
        };
        self.autoplay.take_orders(plan.clone(), now);
        plan
    }

    /// What the leader's plan has this character fight, while the plan
    /// is fresh and the creature is alive and not one this character is
    /// walking past on its way somewhere (see [`Self::joins_the_team_on`]).
    /// Only with focus fire on: that is the switch for fighting as a
    /// party rather than each for itself.
    pub(crate) fn ordered_target(&self, cfg: &Fight, now: Instant) -> Option<(u32, String)> {
        let team = &self.autoplay.config.team;
        if !team.enabled || !team.focus_fire {
            return None;
        }
        let me = self.world.player_guid?;
        let guid = self.autoplay.order_for(me, now)?.target?;
        let o = self.world.objects.get(&guid)?;
        (o.alive_or_unknown() && !self.passing_by(o, cfg)).then(|| (guid, o.name.clone()))
    }

    /// Whether an order to fight something other than `current` is to be
    /// followed now: it names a live creature, and the one in hand is not
    /// hitting this character. What is hitting us is fought to the end
    /// whatever the plan says, since that fight is already happening.
    pub(crate) fn ordered_elsewhere(&self, current: u32, cfg: &Fight, now: Instant) -> bool {
        let Some((ordered, _)) = self.ordered_target(cfg, now) else {
            return false;
        };
        if ordered == current {
            return false;
        }
        let hitting_us = self
            .world
            .objects
            .get(&current)
            .is_some_and(|o| self.hit_lately_by(&o.name));
        !hitting_us
    }

    /// Whether the leader holds the party where it is, for a follower
    /// still fighting or too far behind (see `crate::plan::stragglers`),
    /// and says so. Up to `crate::plan::STRAGGLE_PATIENCE`; past that the
    /// party moves and the following brings the straggler along.
    pub(crate) fn waits_for_stragglers(&mut self, now: Instant) -> bool {
        let team = &self.autoplay.config.team;
        if !team.enabled || !self.autoplay.team.leader || self.autoplay.team.mates.is_empty() {
            self.autoplay.straggling_since = None;
            return false;
        }
        let Some(me) = self.my_position() else {
            self.autoplay.straggling_since = None;
            return false;
        };
        let keep = team.follow_distance;
        let hands = self.hands_of_team(now, false);
        let behind = crate::plan::stragglers(me, keep, &hands);
        if behind.is_empty() {
            self.autoplay.straggling_since = None;
            return false;
        }
        let since = *self.autoplay.straggling_since.get_or_insert(now);
        if now.duration_since(since) >= crate::plan::STRAGGLE_PATIENCE {
            let names: Vec<&str> = behind.iter().map(|s| s.name.as_str()).collect();
            self.autoplay.note(
                format!("waited long enough for {}; moving on", names.join(", ")),
                now,
            );
            self.autoplay.straggling_since = None;
            return false;
        }
        self.autoplay
            .say(Doing::Idle, crate::plan::waiting_line(&behind));
        true
    }
}

#[cfg(test)]
mod tests;
