#[cfg(doc)]
use crate::autoplay::{best_salvager, Mate};
use crate::autoplay::{name_matches, LootAction};
use crate::Client;

/// What means a thing on a body for one of a party in particular, rather
/// than for whoever has the body open (see [`called_to`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Calling {
    /// The rule that takes it asks about this skill: it is for the highest
    /// of that skill, buffs counted, as the rule itself reads it.
    Skill(u32),
    /// The rules would salvage it: it is for whoever salvages for the team
    /// (see [`best_salvager`]). Everyone has Salvaging, so a salvage rule
    /// asks about it without saying so.
    Salvage,
}

/// One rule's claim on a thing for one character (see [`claim_on`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Claim {
    /// Where the claim is read in the profile: 0 for the always list,
    /// which is read before any rule, and each rule's place after that.
    pub order: usize,
    /// What means what it claims for one of the party in particular, if
    /// anything does.
    pub calling: Option<Calling>,
}

/// The claim the loot rules make on a thing for the character `me`, called
/// `name` and already carrying `held` of it: `None` when they would not
/// take it, or cannot say until it is appraised.
///
/// Read as [`judge_loot`] reads it, the player's never and always lists
/// first. What those lists take is nobody's in particular, nor is what a
/// rule takes that asks nothing of the character. A salvage rule calls for
/// the salvager while the salvager salvages (`Looting::salvage`); any other
/// rule that asks about a skill calls for the best at the first it names.
pub fn claim_on(
    stats: &crate::items::ItemStats,
    id: Option<&ac_net::messages::Appraisal>,
    profile: &crate::profile::Profile,
    me: &crate::weapons::Wielder,
    name: &str,
    held: u32,
) -> Option<Claim> {
    if name_matches(&stats.name, &profile.looting.never) {
        return None;
    }
    if name_matches(&stats.name, &profile.looting.always) {
        return Some(Claim {
            order: 0,
            calling: None,
        });
    }
    let (at, rule) = profile.decided_by(stats, id, me, name, held)?;
    if !rule.action.takes() {
        return None;
    }
    let calling = if rule.action == LootAction::Salvage && profile.looting.salvage {
        Some(Calling::Salvage)
    } else {
        rule.skill_asked().map(Calling::Skill)
    };
    Some(Claim {
        order: at + 1,
        calling,
    })
}

/// One of a party a thing on a body could be for, as the loot rules read
/// it: its player guid, its name, its sheet, and how many of the thing it
/// carries (not known of a mate, which is taken to carry none).
#[derive(Clone, Copy, Debug)]
pub struct Taker<'a> {
    pub guid: u32,
    pub name: &'a str,
    pub sheet: &'a crate::weapons::Wielder,
    pub held: u32,
}

/// A fellow a thing on a body could be meant for, as its row says it:
/// player guid, name and sheet (see `Client::callable_to`).
pub(crate) type Fellow = (u32, String, crate::weapons::Wielder);

/// This character (`me`) and the `fellows` a thing could be meant for
/// instead, as [`called_to`] reads them.
pub(crate) fn takers_with<'a>(me: Taker<'a>, fellows: &'a [Fellow]) -> Vec<Taker<'a>> {
    std::iter::once(me)
        .chain(fellows.iter().map(|(guid, name, sheet)| Taker {
            guid: *guid,
            name,
            sheet,
            held: 0,
        }))
        .collect()
}

/// Which of `takers` a thing on a body is meant for, when the rules mean
/// it for one of the party in particular: `None` when they do not, and
/// then whoever has the body open takes it or leaves it by its own reading.
///
/// "If an item has a skill requirement in the loot settings, it should go
/// to whoever has the highest skill." Each taker's reading of the thing is
/// one rule's claim (see [`claim_on`]), and the first claim in the
/// profile's order that calls for someone decides, the way a character's
/// own first matching rule decides for it. It is for one of the takers
/// that same rule claims it for:
/// - a skill: the highest of that skill, buffs counted, which is what the
///   rule reads; ties to the name that sorts first, as the salvager's are
///   broken (see [`best_salvager`]);
/// - salvage: `salvager`, the team's, when it is one of them. When it is
///   not (out of reach, say), the thing is nobody's in particular, and the
///   hand-off carries it to the salvager afterwards as it always has.
pub fn called_to(
    stats: &crate::items::ItemStats,
    id: Option<&ac_net::messages::Appraisal>,
    profile: &crate::profile::Profile,
    takers: &[Taker],
    salvager: Option<u32>,
) -> Option<u32> {
    let claims: Vec<(&Taker, Claim)> = takers
        .iter()
        .filter_map(|t| Some((t, claim_on(stats, id, profile, t.sheet, t.name, t.held)?)))
        .collect();
    let (first, calling) = claims
        .iter()
        .filter_map(|(_, c)| Some((c.order, c.calling?)))
        .min_by_key(|(order, _)| *order)?;
    let mut claimed_for = claims
        .iter()
        .filter(|(_, c)| c.order == first)
        .map(|(t, _)| *t);
    match calling {
        Calling::Salvage => salvager.filter(|g| claimed_for.any(|t| t.guid == *g)),
        Calling::Skill(skill) => claimed_for
            .max_by(|a, b| {
                a.sheet
                    .skill(skill)
                    .cmp(&b.sheet.skill(skill))
                    .then_with(|| b.name.cmp(a.name))
            })
            .map(|t| t.guid),
    }
}

/// Of `me`'s skills, the ones in `asked`: what a character puts on its
/// board row for the others to judge a body by (see [`Mate::skills`]).
pub fn skills_asked_of(me: &crate::weapons::Wielder, asked: &[u32]) -> Vec<(u32, u32, u32, u32)> {
    me.skills
        .iter()
        .filter(|(id, ..)| asked.contains(id))
        .copied()
        .collect()
}

/// A thing still on a body a shut is judged on: what it is, its appraisal
/// if one came back, and how many of it this character carries already.
#[derive(Clone, Debug)]
pub struct Left<'a> {
    pub stats: crate::items::ItemStats,
    pub id: Option<&'a ac_net::messages::Appraisal>,
    pub held: u32,
}

/// Whether the rules would have the character with `sheet`, called `name`
/// and carrying `held` of it already, take `left` off a body.
///
/// Read the way the looting reads a body it stands over (see
/// `Client::corpse_now`): wanted is what a rule takes, and a thing a rule
/// could claim once appraised is wanted while appraising is allowed,
/// because the character would ask.
pub(crate) fn would_take(
    left: &Left,
    profile: &crate::profile::Profile,
    sheet: &crate::weapons::Wielder,
    name: &str,
    held: u32,
) -> bool {
    use crate::profile::Verdict;
    match judge_loot(&left.stats, left.id, Some(profile), sheet, name, held) {
        Verdict::Decided(action, _) => action.takes(),
        Verdict::NeedsId(_) => profile.looting.appraise,
        Verdict::None => false,
    }
}

/// What the loot rules make of an item, and whether they can say yet.
///
/// The character's loot profile decides; with none, nothing is taken.
pub fn judge_loot(
    stats: &crate::items::ItemStats,
    id: Option<&ac_net::messages::Appraisal>,
    profile: Option<&crate::profile::Profile>,
    me: &crate::weapons::Wielder,
    my_name: &str,
    held: u32,
) -> crate::profile::Verdict {
    use crate::profile::Verdict;
    // No profile, nothing decided. The profile is where a player says
    // what their things are worth, and a client with nothing to read
    // should take nothing rather than guess.
    let Some(p) = profile else {
        return Verdict::None;
    };
    // The player's own word comes first, whatever any rule says.
    if name_matches(&stats.name, &p.looting.never) {
        return Verdict::Decided(LootAction::Skip, "never take these".into());
    }
    if name_matches(&stats.name, &p.looting.always) {
        return Verdict::Decided(LootAction::Keep, "always take these".into());
    }
    // The buy list is the player's word too: a line on it says "keep
    // this many of these stocked", and up to that many of the thing
    // are kept, whatever the rules below would make of it. Without
    // this the tapers a character had just bought for its line were
    // judged by "the rest, to the counter" as they arrived, tagged to
    // sell, sold on the next trip and bought again at markup, for as
    // long as the profile stood. Beyond the line the rules answer: a
    // stack over what the line asks for is loot like any other, and
    // "sell the rest" sells it. Judged here, once, when the thing is
    // taken -- the list does not answer back to a tag already written.
    let stocked = p.stocked_count(&stats.name, me, my_name);
    if stocked > 0 && held < stocked {
        return Verdict::Decided(LootAction::Keep, "kept stocked".into());
    }
    p.judge(stats, id, me, my_name, held)
}

impl Client {
    /// How many of the same weenie are already carried, for the rules
    /// that stop at a number.
    pub(crate) fn already_carried(&self, wcid: u32) -> u32 {
        self.world
            .inventory()
            .filter(|o| o.weenie_class_id == wcid)
            .map(|o| o.stack_size.max(1))
            .sum()
    }

    /// The same, for a thing already in the pack: how many of its kind
    /// are carried besides it, which is what was held when it arrived.
    pub(crate) fn carried_besides(&self, stats: &crate::items::ItemStats) -> u32 {
        let own = self
            .world
            .objects
            .get(&stats.guid)
            .map_or(0, |o| o.stack_size.max(1));
        self.already_carried(stats.wcid).saturating_sub(own)
    }

    /// What the rules say to do with an item, now.
    /// The corpse in front of the character, as the looting rules need
    /// to see it.
    ///
    /// The judging stays here -- it needs the profile, the character's
    /// own skills, what is already carried, and whether this is the
    /// character's own body -- and arrives in `ac-loot` as a verdict
    /// already reached. What that crate decides is the *order*: what to
    /// ask about, what to take, when to stop, and when to shut it.
    /// What is to be done with an item: what it was taken for if that
    /// was written down, else what the profile makes of it now.
    ///
    /// The ledger alone is not the answer. It only knows what was
    /// decided about things the character has held since; something
    /// bought, traded or split off a stack a moment ago carries no
    /// entry yet, and a ledger-only reading would call that "nothing
    /// decided", which reads as "skip".
    pub fn loot_action(&self, stats: &crate::items::ItemStats) -> Option<LootAction> {
        if let Some(a) = self.autoplay.ledger.of(stats) {
            return Some(a);
        }
        match judge_loot(
            stats,
            self.appraisals.get(&stats.guid),
            self.loot_profile().as_deref(),
            &self.wielder(),
            &self.world.stats.name,
            self.already_carried(stats.wcid),
        ) {
            crate::profile::Verdict::Decided(a, _) => Some(a),
            crate::profile::Verdict::NeedsId(_) | crate::profile::Verdict::None => None,
        }
    }

    /// The loot profile this character reads, if the shelf has it. None
    /// means it does not loot.
    pub fn loot_profile(&self) -> Option<std::sync::Arc<crate::profile::Profile>> {
        self.profiles.get(&self.autoplay.config.loot.profile)
    }
}

#[cfg(test)]
mod tests;
