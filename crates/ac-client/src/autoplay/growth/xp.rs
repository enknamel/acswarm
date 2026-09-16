use std::time::{Duration, Instant};

use super::raise::{batch_raise, raises_a_maximum, skill_weight, Climb, Offer, Pending, Raise};
use crate::Client;
use ac_world::{equip, item_type};

/// One raise -- a rank, or a batch of them -- is sent at most this often.
const RAISE_EVERY: Duration = Duration::from_millis(700);

/// After a raise is sent, nothing more is spent until the server has
/// answered with the new pool, or this long has passed.
const RAISE_SETTLE: Duration = Duration::from_secs(3);

/// A pool worth fewer than this many of the chosen rank buys that one
/// rank, as it always did; a larger one buys several to a message (see
/// [`batch_raise`]).
pub(super) const BATCH_FROM: i64 = 10;

/// When nothing was affordable, the pool is looked at again this often
/// (or as soon as it changes).
const XP_CHECK_EVERY: Duration = Duration::from_secs(20);

/// A rank the server would not sell (the pool did not move) is not
/// asked for again for this long.
const SULK_FOR: Duration = Duration::from_secs(10 * 60);

impl Client {
    // ---- experience -----------------------------------------------

    /// The weapon in hand (not the ammunition), if any.
    fn weapon_in_hand(&self) -> Option<u32> {
        self.world
            .wielded()
            .find(|o| {
                o.item_type & (item_type::MELEE_WEAPON | item_type::MISSILE_WEAPON) != 0
                    && o.valid_locations & equip::MISSILE_AMMO == 0
            })
            .map(|o| o.guid)
    }

    /// The weapon skill the character fights with, from its hands:
    /// `Some(0)` for a caster. An unappraised weapon says nothing about
    /// its skill, so the stance stands in: a bow is Missile Weapons,
    /// and a melee weapon whichever melee skill is trained highest.
    fn fighting_skill(&self) -> Option<u32> {
        use ac_world::stats::sac;
        let stance = self.combat_stance();
        if stance == crate::Stance::Magic {
            return Some(0);
        }
        let weapon = self.weapon_in_hand()?;
        if let Some(id) = self
            .stats_of(weapon)
            .map(|i| i.weapon_skill_id)
            .filter(|id| *id != 0)
        {
            return Some(id);
        }
        if stance == crate::Stance::Missile {
            return Some(47);
        }
        self.world
            .stats
            .skills
            .iter()
            .filter(|s| [41, 44, 45, 46].contains(&s.id) && s.advancement >= sac::TRAINED)
            .max_by_key(|s| (s.advancement, s.ranks))
            .map(|s| s.id)
    }

    /// Everything experience could go on right now, priced and weighed.
    pub fn raise_offers(&self) -> Vec<Offer> {
        use ac_world::stats::sac;
        let weapon_skill = self.fighting_skill();
        let caster = weapon_skill == Some(0);
        let table = self.assets.skill_table().ok();
        let mut offers = Vec::new();
        // The attributes the fighting skill is built from matter as
        // much as a defence; the others little, Endurance apart.
        let mut attr_weight = [0.15f32; 6];
        attr_weight[1] = 0.45;
        let feeds = |skill: u32| -> Vec<u32> {
            table
                .as_ref()
                .and_then(|t| t.get(skill))
                .map(|b| {
                    [b.formula.attr1, b.formula.attr2]
                        .into_iter()
                        .filter(|a| (1..=6).contains(a))
                        .collect()
                })
                .unwrap_or_default()
        };
        let main: Vec<u32> = match weapon_skill {
            Some(0) => vec![
                ac_world::stats::skill::WAR_MAGIC,
                ac_world::stats::skill::LIFE_MAGIC,
            ],
            Some(s) => vec![s],
            None => Vec::new(),
        };
        for s in &main {
            for a in feeds(*s) {
                attr_weight[a as usize - 1] = attr_weight[a as usize - 1].max(0.6);
            }
        }
        for sk in &self.world.stats.skills {
            if sk.advancement < sac::TRAINED {
                continue;
            }
            let weight = skill_weight(sk.id, weapon_skill, caster);
            if let Some(cost) = self.skill_raise_cost(sk.id).xp() {
                offers.push(Offer {
                    raise: Raise::Skill(sk.id),
                    cost,
                    weight,
                });
            }
        }
        for (i, w) in attr_weight.iter().enumerate() {
            if let Some(cost) = self.attribute_raise_cost(i).xp() {
                offers.push(Offer {
                    raise: Raise::Attribute(i),
                    cost,
                    weight: *w,
                });
            }
        }
        let vital_weight = [0.5, 0.2, if caster { 0.5 } else { 0.15 }];
        for (i, w) in vital_weight.iter().enumerate() {
            if let Some(cost) = self.vital_raise_cost(i).xp() {
                offers.push(Offer {
                    raise: Raise::Vital(i),
                    cost,
                    weight: *w,
                });
            }
        }
        offers
    }

    /// Spend experience, as housekeeping: on every tick, whatever else
    /// the character is doing.
    ///
    /// This used to be the first of the growth rules, and those are the
    /// last goal in the table, run only on a tick nothing else claims.
    /// Something always did. A character granted a hundred billion
    /// experience on the local server spent none of it in three minutes,
    /// because exploring had another room to walk to on every one of
    /// those ticks. A raise needs no tick of its own: it is one message --
    /// a rank, or for a large pool a batch of them (see [`batch_raise`])
    /// -- and the pacing keeps it to one at a time: no more often than
    /// [`RAISE_EVERY`], nothing more until the server has answered or
    /// [`RAISE_SETTLE`] has passed, and a raise it would not sell left
    /// alone for [`SULK_FOR`].
    ///
    /// Nor does it wait for a fight to end. ACE takes a raise with no
    /// busy check, no animation and no movement: it checks the stat, the
    /// pool and the rank's ceiling, spends, and answers with the new
    /// record, a sound and a line of chat. None of that touches an attack
    /// or a cast under way. Strength, Quickness and Run can change the run
    /// rate, which ACE sends out again only for a character already
    /// moving and only with `runrate_add_hooks`, off by default -- and
    /// that is a speed, not an action. The server does not mind a rank
    /// that raises a maximum either, but the client's own heal lines do,
    /// and that rank waits (see [`raises_a_maximum`]).
    pub(crate) fn autoplay_spend_xp(&mut self, now: Instant) {
        if self.autoplay.config.growth.auto_xp {
            self.grow_spend_xp(now);
        }
    }

    /// Buy the rank worth buying -- with a large pool, as many of its
    /// ranks as buying a rank at a time would give it -- in one message.
    /// True when one was sent.
    fn grow_spend_xp(&mut self, now: Instant) -> bool {
        let xp = self.world.stats.available_xp;
        if xp <= 0 || self.world.stats.level <= 0 {
            return false;
        }
        let stats = &self.world.stats;
        let st = &mut self.autoplay.growth;
        // The raise last sent is answered when its stat moves.
        if let Some(p) = st.pending {
            if p.raise.standing(stats) != p.was {
                st.pending = None;
            } else {
                match p.since {
                    // Queued on the last look, and on the wire since the
                    // top of this tick: its time starts now. Started when
                    // it was queued, a round of nine headless characters
                    // that took four and a half seconds gave up on nine
                    // raises in the tick they were sent, and every one of
                    // them landed a third of a second later.
                    None => {
                        st.pending = Some(Pending {
                            since: Some(now),
                            ..p
                        });
                        return false;
                    }
                    Some(since) if now.duration_since(since) < RAISE_SETTLE => return false,
                    // No answer in time: refused, or only late. It is left
                    // alone for a while, and keeps its share of the pool.
                    Some(_) => {
                        tracing::info!(
                            "growth: no answer to a raise of {} ({} ranks for {} xp)",
                            p.raise.name(),
                            p.batch.ranks,
                            p.batch.xp
                        );
                        st.sulking.push((p, now));
                        st.pending = None;
                    }
                }
            }
        }
        // An answer that comes in late ends the wait; the rest wear off.
        // Either can leave something to buy that was not there.
        let sulks = st.sulking.len();
        st.sulking.retain(|(p, since)| {
            let landed = p.raise.standing(stats) != p.was;
            if landed {
                tracing::info!("growth: the raise of {} came in late", p.raise.name());
            }
            !landed && now.duration_since(*since) < SULK_FOR
        });
        if st.sulking.len() < sulks {
            st.nothing_at = None;
        }
        if st
            .last_raise
            .is_some_and(|t| now.duration_since(t) < RAISE_EVERY)
        {
            return false;
        }
        let fighting = self.in_a_fight();
        if let Some((at, was_fighting, when)) = self.autoplay.growth.nothing_at {
            if at == xp && was_fighting == fighting && now.duration_since(when) < XP_CHECK_EVERY {
                return false;
            }
        }
        // The weapon's skill is read off its appraisal.
        if let Some(w) = self.weapon_in_hand() {
            if !self.appraisals.contains_key(&w) {
                self.appraise_many([w]);
            }
        }
        let sulking: Vec<Raise> = self
            .autoplay
            .growth
            .sulking
            .iter()
            .map(|(p, _)| p.raise)
            .collect();
        let Ok(table) = self.assets.xp_table() else {
            self.autoplay.growth.nothing_at = Some((xp, fighting, now));
            return false;
        };
        // A rank that raises a maximum waits for the fight to be over.
        // The fight under way is fought with what is left, which the rank
        // does not raise, and the fraction left drops as surely as it
        // would for a blow: a character with a large pool bought Health
        // between swings until it healed, mid-fight, health it had never
        // lost. Skills and the other attributes count at once, and go on
        // being bought. A stat whose raise went unanswered waits too. Both
        // are held, not left out: the pool is spread over them all the
        // same, and what is theirs waits for them (see [`batch_raise`]).
        // A pool with nothing to buy but what is held is noted as having
        // nothing until it moves or the fight ends.
        let field: Vec<Climb> = self
            .raise_offers()
            .into_iter()
            .filter_map(|o| {
                Some(Climb {
                    raise: o.raise,
                    ladder: self.raise_ladder(&table, o.raise)?,
                    weight: o.weight,
                    held: (fighting && raises_a_maximum(o.raise)) || sulking.contains(&o.raise),
                })
            })
            .collect();
        let Some((pick, batch)) = batch_raise(&field, xp) else {
            self.autoplay.growth.nothing_at = Some((xp, fighting, now));
            return false;
        };
        let was = pick.standing(&self.world.stats);
        if !self.raise_by(pick, batch.xp) {
            self.autoplay.growth.nothing_at = Some((xp, fighting, now));
            return false;
        }
        let st = &mut self.autoplay.growth;
        st.last_raise = Some(now);
        st.pending = Some(Pending {
            raise: pick,
            batch,
            was,
            since: None,
        });
        st.nothing_at = None;
        // A note, not a status. This is housekeeping, and the status line
        // belongs to whatever claims the tick: said here, a character with
        // nothing else to do would go from "waiting" to "spending
        // experience" and back again with every rank.
        let what = match batch.ranks {
            1 => pick.name().to_string(),
            n => format!("{} {n} ranks", pick.name()),
        };
        self.autoplay
            .note(format!("raising {what} ({xp} xp to spend)"), now);
        true
    }
}

#[cfg(test)]
mod tests;
