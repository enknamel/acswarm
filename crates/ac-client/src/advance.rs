//! Character advancement (see `docs/game/mechanics.md`, section 3):
//! unassigned experience is spent on attributes, vitals and trained or
//! specialized skills rank by rank, at costs from the XpTable
//! (0x0E000018), and skill credits train new skills at the SkillTable's
//! price. The server (ACE `Player_Skills`, `Player_Attributes`,
//! `Player_Vitals`) takes an XP amount and adds all of it to the stat,
//! whose rank is then the highest the experience spent on it reaches:
//! sending exactly the cost of the next rank raises it by one, the cost of
//! several raises it by several in the one message, and an amount short
//! of a rank is kept towards it. An amount past the pool, or past the
//! experience left to the top rank, is refused outright rather than
//! trimmed. It answers with the updated record and the new unassigned
//! total.

use ac_formats::xp_table::XpTable;

use crate::growth::Raise;
use crate::Client;

/// Attribute indices in `stats.attributes`, also the wire ids minus one
/// (PropertyAttribute: Strength 1, Endurance 2, Quickness 3,
/// Coordination 4, Focus 5, Self 6).
pub const ATTRIBUTE_NAMES: [&str; 6] = [
    "Strength",
    "Endurance",
    "Quickness",
    "Coordination",
    "Focus",
    "Self",
];
/// Vital indices in `stats.vitals`; wire ids are MaxHealth 1,
/// MaxStamina 3, MaxMana 5.
pub const VITAL_NAMES: [&str; 3] = ["Health", "Stamina", "Mana"];
const VITAL_WIRE: [u32; 3] = [1, 3, 5];

/// What one more rank costs, or why it cannot be bought.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaiseCost {
    /// XP for the next rank (the caller checks the unassigned pool).
    Xp(u32),
    /// At the table's last rank.
    Maxed,
    /// Not trained (skills), or unknown.
    Unavailable,
}

impl RaiseCost {
    pub fn xp(self) -> Option<u32> {
        match self {
            RaiseCost::Xp(x) => Some(x),
            _ => None,
        }
    }
}

fn next_cost(table: &[u32], ranks: u32, spent: u32) -> RaiseCost {
    match table.get(ranks as usize + 1) {
        Some(&total) => RaiseCost::Xp(total.saturating_sub(spent).max(1)),
        None => RaiseCost::Maxed,
    }
}

/// Where a stat stands on its column of the XpTable.
///
/// The column is cumulative: the entry at a rank is all the experience it
/// takes to reach that rank from nothing, and the last entry is the top
/// rank. What has been spent can lie between two entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ladder<'a> {
    pub table: &'a [u32],
    pub ranks: u32,
    pub spent: u32,
}

impl Ladder<'_> {
    /// The top rank.
    pub fn top(&self) -> u32 {
        (self.table.len() as u32).saturating_sub(1)
    }

    /// What the next rank costs.
    pub fn next_cost(&self) -> RaiseCost {
        next_cost(self.table, self.ranks, self.spent)
    }

    /// The experience that takes the stat from where it stands to `rank`
    /// in one message: the table's entry less what was spent. None for a
    /// rank it already has or one past the top. `cost_to(top())` is all
    /// the server will take (ACE's `ExperienceLeft`).
    pub fn cost_to(&self, rank: u32) -> Option<u32> {
        if rank <= self.ranks {
            return None;
        }
        let total = *self.table.get(rank as usize)?;
        Some(total.saturating_sub(self.spent).max(1))
    }

    /// What the rank after `rank` costs on its own, `rank` being the one
    /// the stat has or one above it. None at the top.
    pub fn step(&self, rank: u32) -> Option<u32> {
        let next = *self.table.get(rank as usize + 1)?;
        let paid = self.table[rank as usize].max(self.spent);
        Some(next.saturating_sub(paid).max(1))
    }

    /// The rank the stat is at once `xp` more is spent on it: the highest
    /// whose entry the experience spent reaches, as ACE works it out.
    pub fn rank_after(&self, xp: u32) -> u32 {
        let spent = u64::from(self.spent) + u64::from(xp);
        let reached = self.table.partition_point(|&t| u64::from(t) <= spent);
        (reached as u32).saturating_sub(1)
    }
}

impl Client {
    /// Where `raise` stands on its column of `xp`. None for a skill that
    /// is not trained or not on the sheet.
    pub fn raise_ladder<'t>(&self, xp: &'t XpTable, raise: Raise) -> Option<Ladder<'t>> {
        use ac_world::stats::sac;
        let stats = &self.world.stats;
        match raise {
            Raise::Attribute(i) => stats.attributes.get(i).map(|a| Ladder {
                table: &xp.attribute,
                ranks: a.ranks,
                spent: a.xp,
            }),
            Raise::Vital(i) => stats.vitals.get(i).map(|v| Ladder {
                table: &xp.vital,
                ranks: v.ranks,
                spent: v.xp,
            }),
            Raise::Skill(id) => {
                let s = stats.skill(id)?;
                let table = match s.advancement {
                    sac::TRAINED => &xp.trained_skill,
                    sac::SPECIALIZED => &xp.specialized_skill,
                    _ => return None,
                };
                Some(Ladder {
                    table,
                    ranks: u32::from(s.ranks),
                    spent: s.xp,
                })
            }
        }
    }

    /// XP for the next rank of `raise`.
    pub fn raise_cost(&self, raise: Raise) -> RaiseCost {
        let Ok(xp) = self.assets.xp_table() else {
            return RaiseCost::Unavailable;
        };
        self.raise_ladder(&xp, raise)
            .map_or(RaiseCost::Unavailable, |l| l.next_cost())
    }

    /// XP for the next point of an attribute (index into
    /// [`ATTRIBUTE_NAMES`]).
    pub fn attribute_raise_cost(&self, index: usize) -> RaiseCost {
        self.raise_cost(Raise::Attribute(index))
    }

    /// XP for the next point of a vital (index into [`VITAL_NAMES`]).
    pub fn vital_raise_cost(&self, index: usize) -> RaiseCost {
        self.raise_cost(Raise::Vital(index))
    }

    /// XP for the next rank of a trained or specialized skill.
    pub fn skill_raise_cost(&self, skill: u32) -> RaiseCost {
        self.raise_cost(Raise::Skill(skill))
    }

    /// Skill credits to train an untrained skill; None when it is
    /// already trained or not on the sheet.
    pub fn skill_train_cost(&self, skill: u32) -> Option<u32> {
        use ac_world::stats::sac;
        // A skill the sheet lacks a record for is untrained (the server
        // creates the record when it is trained).
        if self
            .world
            .stats
            .skill(skill)
            .is_some_and(|s| s.advancement >= sac::TRAINED)
        {
            return None;
        }
        let table = self.assets.skill_table().ok()?;
        let base = table.get(skill)?;
        Some(base.trained_cost.max(0) as u32)
    }

    /// Spend `xp` on `raise` in one message (RaiseAttribute 0x0045,
    /// RaiseVital 0x0044, RaiseSkill 0x0046): the cost of the next rank,
    /// or of several (see [`Ladder::cost_to`]). False, with nothing sent,
    /// when the pool does not hold it or it goes past the top rank, which
    /// the server would refuse.
    pub fn raise_by(&mut self, raise: Raise, xp: u32) -> bool {
        use ac_net::messages::action;
        let Ok(table) = self.assets.xp_table() else {
            return false;
        };
        let Some(ladder) = self.raise_ladder(&table, raise) else {
            return false;
        };
        let within_the_top = ladder.cost_to(ladder.top()).is_some_and(|left| xp <= left);
        if xp == 0 || i64::from(xp) > self.world.stats.available_xp || !within_the_top {
            return false;
        }
        let (id, message) = match raise {
            Raise::Attribute(i) => (i as u32 + 1, action::RAISE_ATTRIBUTE),
            Raise::Vital(i) => match VITAL_WIRE.get(i) {
                Some(&wire) => (wire, action::RAISE_VITAL),
                None => return false,
            },
            Raise::Skill(id) => (id, action::RAISE_SKILL),
        };
        let ranks = ladder.rank_after(xp).saturating_sub(ladder.ranks);
        if ranks > 1 {
            tracing::info!("raise {} for {xp} xp ({ranks} ranks)", raise.name());
        } else {
            tracing::info!("raise {} for {xp} xp", raise.name());
        }
        let mut w = ac_net::wire::Writer::new();
        w.u32(id).u32(xp);
        self.session.send_action(message, &w.finish());
        true
    }

    /// One rank of `raise` at its price. False when unaffordable or maxed.
    fn raise_one(&mut self, raise: Raise) -> bool {
        match self.raise_cost(raise).xp() {
            Some(xp) => self.raise_by(raise, xp),
            None => false,
        }
    }

    /// Spend unassigned XP on one attribute point (RaiseAttribute 0x0045).
    /// False when unaffordable or maxed.
    pub fn raise_attribute(&mut self, index: usize) -> bool {
        self.raise_one(Raise::Attribute(index))
    }

    /// Spend unassigned XP on one vital point (RaiseVital 0x0044).
    pub fn raise_vital(&mut self, index: usize) -> bool {
        self.raise_one(Raise::Vital(index))
    }

    /// Spend unassigned XP on one skill rank (RaiseSkill 0x0046).
    pub fn raise_skill(&mut self, skill: u32) -> bool {
        self.raise_one(Raise::Skill(skill))
    }

    /// Train an untrained skill with skill credits (TrainSkill 0x0047).
    pub fn train_skill(&mut self, skill: u32) -> bool {
        use ac_net::messages::action;
        let Some(credits) = self.skill_train_cost(skill) else {
            tracing::debug!(
                "train {}: no cost (already trained? {:?})",
                ac_world::stats::skill_name(skill),
                self.world.stats.skill(skill).map(|s| s.advancement)
            );
            return false;
        };
        if self.world.stats.skill_credits < credits as i32 {
            tracing::debug!(
                "train {}: costs {credits}, have {} credits",
                ac_world::stats::skill_name(skill),
                self.world.stats.skill_credits
            );
            return false;
        }
        tracing::info!(
            "train {} for {credits} credits",
            ac_world::stats::skill_name(skill)
        );
        let mut w = ac_net::wire::Writer::new();
        w.u32(skill).u32(credits);
        self.session.send_action(action::TRAIN_SKILL, &w.finish());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_rank_is_the_table_step_minus_what_was_spent() {
        let table = [0u32, 23, 56, 97];
        assert_eq!(next_cost(&table, 0, 0), RaiseCost::Xp(23));
        assert_eq!(next_cost(&table, 1, 23), RaiseCost::Xp(33));
        // Partly paid ranks cost the remainder.
        assert_eq!(next_cost(&table, 1, 40), RaiseCost::Xp(16));
        assert_eq!(next_cost(&table, 3, 97), RaiseCost::Maxed);
    }

    #[test]
    fn several_ranks_cost_the_table_entry_less_what_was_spent() {
        let table = [0u32, 23, 56, 97];
        let partly = Ladder {
            table: &table,
            ranks: 1,
            spent: 40,
        };
        assert_eq!(partly.top(), 3);
        assert_eq!(partly.cost_to(1), None, "a rank it has");
        assert_eq!(partly.cost_to(2), Some(16));
        assert_eq!(partly.cost_to(3), Some(57));
        assert_eq!(partly.cost_to(4), None, "past the top");
        // Rank by rank comes to the same.
        assert_eq!(partly.step(1), Some(16));
        assert_eq!(partly.step(2), Some(41));
        assert_eq!(partly.step(3), None);
        // The server's rank is the highest entry the experience reaches;
        // what falls short of the next is kept towards it.
        assert_eq!(partly.rank_after(15), 1);
        assert_eq!(partly.rank_after(16), 2);
        assert_eq!(partly.rank_after(56), 2);
        assert_eq!(partly.rank_after(57), 3);
        assert_eq!(partly.rank_after(u32::MAX), 3);
    }
}
