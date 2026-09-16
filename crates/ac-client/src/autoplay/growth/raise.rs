use std::time::Instant;

use super::BATCH_FROM;

/// One thing experience can be spent on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Raise {
    Skill(u32),
    /// Index into `advance::ATTRIBUTE_NAMES`.
    Attribute(usize),
    /// Index into `advance::VITAL_NAMES`.
    Vital(usize),
}

impl Raise {
    /// What the stat is called.
    pub fn name(self) -> &'static str {
        use crate::advance::{ATTRIBUTE_NAMES, VITAL_NAMES};
        match self {
            Raise::Skill(id) => ac_world::stats::skill_name(id),
            Raise::Attribute(i) => ATTRIBUTE_NAMES.get(i).copied().unwrap_or("an attribute"),
            Raise::Vital(i) => VITAL_NAMES.get(i).copied().unwrap_or("a vital"),
        }
    }
}

/// A rank on offer: what it is, what it costs, and how much it matters
/// (1 for the skill the character fights with, less for the rest).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Offer {
    pub raise: Raise,
    pub cost: u32,
    pub weight: f32,
}

/// The better buy of two ranks, each a cost and a weight: the cheaper for
/// what it is worth, and of two as good, the cheaper outright.
fn by_value(a: (u32, f32), b: (u32, f32)) -> std::cmp::Ordering {
    (a.0 as f32 / a.1)
        .total_cmp(&(b.0 as f32 / b.1))
        .then(a.0.cmp(&b.0))
}

/// Of ranks each a cost and a weight, the best buy (see [`by_value`]),
/// with what it costs. A rank that matters not at all is never one.
fn best_buy<T>(ranks: impl Iterator<Item = (T, u32, f32)>) -> Option<(T, u32)> {
    ranks
        .filter(|r| r.2 > 0.0)
        .min_by(|a, b| by_value((a.1, a.2), (b.1, b.2)))
        .map(|(what, cost, _)| (what, cost))
}

/// The rank worth buying with `xp` to spend: the cheapest for what it is
/// worth, and nothing while the pool is short of it. A cheap rank of a
/// minor skill is bought before a dear rank of the main one, and the main
/// one catches up as the minor ones get dear.
///
/// It catches up only because the pool saves for it. Of the affordable
/// ranks alone, a character that had spent a large pool bought a minor
/// rank the moment the kills after it covered one: ten billion left
/// War Magic's next rank at 110 million and Arcane Lore's at 17 million,
/// worth about the same for what they are, and two billion more a kill at
/// a time went on Arcane Lore, Jump, Loyalty and the like, ten ranks each,
/// while War, Life, Focus, Self and Health got none.
pub fn choose_raise(offers: &[Offer], xp: i64) -> Option<Raise> {
    best_buy(offers.iter().map(|o| (o.raise, o.cost, o.weight)))
        .filter(|&(_, cost)| i64::from(cost) <= xp)
        .map(|(raise, _)| raise)
}

/// A stat in the running for the pool: the whole of its ladder, not only
/// its next rank, how much a rank of it matters, and whether it may be
/// bought now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Climb<'a> {
    pub raise: Raise,
    pub ladder: crate::advance::Ladder<'a>,
    pub weight: f32,
    /// Not to be bought for now -- a maximum in the middle of a fight, or
    /// a stat whose last raise the server has not answered -- but its
    /// share of the pool is kept for it all the same.
    pub held: bool,
}

/// The rank each stat of `field` is at once `xp` is spent on paper a rank
/// at a time, as [`choose_raise`] spends it: the best buy each time, until
/// the best buy costs more than is left.
///
/// Where it stops is the only thing the pool decides. The order of the
/// ranks is set by their prices and weights alone, so a pool spent in
/// parts -- some of the stats bought, the rest still to come, or answers
/// that have not arrived yet -- comes to the same ranks as the whole of it
/// spent at once, and the parts never add up to more than the pool.
fn plan(field: &[Climb], xp: i64) -> Vec<u32> {
    let mut at: Vec<u32> = field.iter().map(|c| c.ladder.ranks).collect();
    let mut left = xp;
    loop {
        let best = best_buy(
            field
                .iter()
                .zip(&at)
                .enumerate()
                .filter_map(|(i, (c, &rank))| Some((i, c.ladder.step(rank)?, c.weight))),
        );
        match best {
            Some((i, cost)) if i64::from(cost) <= left => {
                left -= i64::from(cost);
                at[i] += 1;
            }
            _ => return at,
        }
    }
}

/// Ranks of one stat bought in one message, and the experience they cost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Batch {
    pub ranks: u32,
    pub xp: u32,
}

/// A raise sent to the server and not yet answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Pending {
    pub(super) raise: Raise,
    pub(super) batch: Batch,
    /// Where the stat stood when it was sent (see [`Raise::standing`]).
    /// The answer moves it. The pool alone says nothing: a kill moves that
    /// too, and a stat sized again from a record the answer has not
    /// reached yet is bought twice.
    pub(super) was: Option<(u32, u32)>,
    /// When the message went out, which is not when it was queued: it
    /// goes on the wire at the top of the next tick, and a round of nine
    /// headless characters took four and a half seconds on the local
    /// server. Stamped by the first look at it after that.
    pub(super) since: Option<Instant>,
}

impl Raise {
    /// The stat's ranks and the experience spent on it, as the server last
    /// said. None for a skill not on the sheet.
    pub(super) fn standing(self, stats: &ac_world::stats::PlayerStats) -> Option<(u32, u32)> {
        match self {
            Raise::Skill(id) => stats.skill(id).map(|s| (u32::from(s.ranks), s.xp)),
            Raise::Attribute(i) => stats.attributes.get(i).map(|a| (a.ranks, a.xp)),
            Raise::Vital(i) => stats.vitals.get(i).map(|v| (v.ranks, v.xp)),
        }
    }
}

/// What to raise in one message, and by how many ranks, with `xp` to spend
/// and `field` every stat in the running for it.
///
/// ACE takes the experience for any number of ranks in one message and
/// puts all of it on the stat, and a rank to a message is hopeless for a
/// large pool: nine characters granted ten to fifty billion each on the
/// local server bought about a rank a second, and the one given ten
/// billion spent a thousandth of a percent of it in five minutes. But a
/// large pool must still be spread the way [`choose_raise`] spreads it,
/// over the skills and bars that matter, not poured into whichever stat
/// happened to be the best buy when it came in.
///
/// So the whole pool is spent on paper exactly as it would be a rank at a
/// time (see `plan`), and the best buy of the stats that paper run
/// raises is given the ranks it gives it, no more. No stat goes past the
/// rank buying one at a time would have left it at; each of the others is
/// bought up to its own in a message of its own when its turn comes. A
/// pool of any size is spread as it always was, and spent in about as
/// many messages as there are stats: ten billion in two dozen. Two
/// simpler rules were tried on the real XpTable and left behind. Buying
/// the best buy only while it stays the best value is a rank a message
/// again as soon as the stats are level, and spent nothing of ten billion
/// in a minute; capping a message at a share of the pool ran the first
/// stats bought sixty ranks ahead of the rest.
///
/// A stat held back for now (see [`Climb::held`]) is spent on in the paper
/// run like any other, and never sent. Its share waits for it, and a stat
/// the run gives nothing more is not sold a rank anyway: a caster given
/// ten billion in the middle of a fight was, on paper, a rank a message at
/// up to a hundred million, until Self, Health and Mana came out of the
/// fight ninety ranks short.
///
/// A pool worth fewer than `BATCH_FROM` of the chosen stat's next rank
/// buys the one rank, as it always did: what a kill brings in goes a rank
/// a message. A batch never goes past the top rank (ACE refuses an amount
/// past the experience left to the top outright, rather than trimming
/// it), never costs more than the pool, and so never more than the `u32`
/// a message carries, which is what the table's entries are. The paper
/// run is a step per rank: a few thousand at most, over a few dozen
/// stats, once a message.
///
/// None when the paper run raises nothing that may be bought now.
pub fn batch_raise(field: &[Climb], xp: i64) -> Option<(Raise, Batch)> {
    let planned = plan(field, xp);
    let (c, to) = best_buy(
        field
            .iter()
            .zip(planned)
            .filter(|(c, to)| !c.held && *to > c.ladder.ranks)
            .filter_map(|(c, to)| Some(((c, to), c.ladder.step(c.ladder.ranks)?, c.weight))),
    )
    .map(|((c, to), first)| {
        let to = if xp < i64::from(first) * BATCH_FROM {
            c.ladder.ranks + 1
        } else {
            to
        };
        (c, to)
    })?;
    Some((
        c.raise,
        Batch {
            ranks: to - c.ladder.ranks,
            xp: c.ladder.cost_to(to)?,
        },
    ))
}

/// Whether a rank raises the maximum of a vital: Health, Stamina or Mana
/// itself, or Endurance or Self, which the maximums are built from.
///
/// Such a rank leaves what is left of the vital where it was (ACE spends
/// the experience on the ranks and never touches the current value), so
/// the fraction left drops with every one. That fraction is what the
/// heal, Revitalize and mana rules read, and what the team reads to
/// heal a mate.
pub(super) fn raises_a_maximum(raise: Raise) -> bool {
    const ENDURANCE: usize = 1;
    const SELF: usize = 5;
    matches!(raise, Raise::Vital(_) | Raise::Attribute(ENDURANCE | SELF))
}

/// How much a skill matters to a character fighting with `weapon_skill`
/// (0 for a caster's wand), given the schools it casts with.
pub(super) fn skill_weight(skill: u32, weapon_skill: Option<u32>, caster: bool) -> f32 {
    use ac_world::stats::skill;
    const CREATURE_ENCHANTMENT: u32 = 31;
    const ITEM_ENCHANTMENT: u32 = 32;
    const VOID_MAGIC: u32 = 43;
    if Some(skill) == weapon_skill {
        return 1.0;
    }
    match skill {
        skill::WAR_MAGIC | VOID_MAGIC => {
            if caster {
                1.0
            } else {
                0.15
            }
        }
        skill::LIFE_MAGIC => {
            if caster {
                0.9
            } else {
                0.6
            }
        }
        skill::MELEE_DEFENSE | skill::MISSILE_DEFENSE => 0.7,
        15 => 0.6, // Magic Defense
        skill::HEALING => 0.5,
        CREATURE_ENCHANTMENT | ITEM_ENCHANTMENT => 0.5,
        skill::MANA_CONVERSION => {
            if caster {
                0.6
            } else {
                0.2
            }
        }
        skill::FLETCHING => {
            if weapon_skill == Some(47) {
                0.5
            } else {
                0.1
            }
        }
        skill::RUN => 0.3,
        // Another weapon skill: not the one in hand.
        41 | 44..=50 => 0.1,
        _ => 0.15,
    }
}

#[cfg(test)]
mod tests;
