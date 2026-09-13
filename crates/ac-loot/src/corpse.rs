//! What a corpse and the character looking into it look like to the
//! rules.
//!
//! Plain values, so that emptying one can be followed at a desk. What
//! it deliberately does not describe is *why* an item is worth having:
//! that is the player's profile, judged elsewhere, and arrives here as
//! a verdict already reached -- carrying the decision with it, so that
//! nothing downstream has to ask again and get a different answer.

/// What the rules have been told about one thing in the corpse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Worth taking, and what for.
    ///
    /// The decision travels with the verdict because it used to be
    /// reached twice: once here to settle the order a corpse is emptied
    /// in, and again as each thing was picked up, to recover what it was
    /// picked up *for*. The two disagreed whenever an appraisal landed
    /// in between, or when a cap filled because an earlier item on the
    /// same body had been taken.
    Take(crate::profile::LootAction),
    /// Not worth taking.
    Leave,
    /// Cannot be judged without asking the server about it.
    MustAsk,
}

/// One thing lying in the corpse.
#[derive(Clone, Debug, PartialEq)]
pub struct Lying {
    pub guid: u32,
    pub name: String,
    /// What the whole stack weighs.
    pub burden: u32,
    /// What the profile made of it.
    pub verdict: Verdict,
}

impl Lying {
    /// What this was judged to be worth taking for, if it was.
    pub fn took(&self) -> Option<crate::profile::LootAction> {
        match self.verdict {
            Verdict::Take(a) => Some(a),
            Verdict::Leave | Verdict::MustAsk => None,
        }
    }
}

/// The corpse, and the character standing over it.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Open {
    /// The corpse itself.
    pub guid: u32,
    pub name: String,
    /// How far off the character is standing.
    pub away: f32,
    /// The window is open and its contents are known.
    pub open: bool,
    pub items: Vec<Lying>,
    /// Slots left in the pack.
    pub slots_free: u32,
    /// Slots to leave empty however much is lying here. A counter needs
    /// somewhere to put the coin before it takes anything, so a pack
    /// looted to its last slot cannot be sold out of at all.
    pub keep_free: u32,
    /// How much more the character means to carry (see
    /// `growth::carry_room`): zero when it has had enough.
    pub carry_room: u32,
    /// Asking the server to identify things is allowed.
    pub may_ask: bool,
    /// An identify is already out for these.
    pub asking: Vec<u32>,
}

/// How near the character must be before a corpse will open for it.
pub const REACH: f32 = 2.5;

/// What something taken off your own corpse is for.
///
/// Everything on it comes back -- it is all yours, and the wand and the
/// components are what the character needs to fight again -- but what
/// each thing is *for* is still the profile's answer. Answering "keep"
/// for the lot writes a Keep over every decision the character had
/// already made: twenty things meant for a counter and three for the
/// salvage bag come home unsellable and unsalvageable, and the pack
/// fills with loot it can no longer get rid of.
pub fn recovered(judged: Option<crate::profile::LootAction>) -> crate::profile::LootAction {
    match judged {
        Some(a) if a.takes() => a,
        // Nothing claimed it, or the rules said leave it -- which is not
        // an answer that applies to your own belongings.
        _ => crate::profile::LootAction::Keep,
    }
}

/// How many of each kind have been claimed off this body so far.
///
/// A cap ("keep at most two healing kits") counts what the character
/// will be carrying, so what has already been claimed off the body in
/// front of it counts towards that. Judging every item against the pack
/// as it was when the lid came up makes a cap of one take all four
/// copies lying there -- each of them judged against a pack holding
/// none, because none of them has arrived yet.
#[derive(Clone, Debug, Default)]
pub struct Claimed(std::collections::HashMap<u32, u32>);

impl Claimed {
    /// How many of `wcid` have been spoken for.
    pub fn of(&self, wcid: u32) -> u32 {
        self.0.get(&wcid).copied().unwrap_or(0)
    }

    /// Speak for `n` more of `wcid`.
    pub fn take(&mut self, wcid: u32, n: u32) {
        *self.0.entry(wcid).or_default() += n;
    }
}

impl Open {
    /// The things still worth taking, dearest first is not the order --
    /// a corpse is emptied in the order it lists, because the server
    /// moves one at a time and the character wants the lot.
    pub fn wanted(&self) -> impl Iterator<Item = &Lying> {
        self.items
            .iter()
            .filter(|i| matches!(i.verdict, Verdict::Take(_)))
    }

    /// The things whose fate cannot be settled without the server.
    ///
    /// Only these are asked about. An identify is a round trip each,
    /// and on a corpse of eight that is eight of them before anything
    /// is picked up; a profile whose early rules ask about name, kind
    /// and worth empties a corpse without a single one.
    pub fn unjudged(&self) -> impl Iterator<Item = &Lying> {
        self.items
            .iter()
            .filter(|i| i.verdict == Verdict::MustAsk && !self.asking.contains(&i.guid))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::LootAction;

    fn lying(guid: u32, verdict: Verdict) -> Lying {
        Lying {
            guid,
            name: format!("thing {guid}"),
            burden: 10,
            verdict,
        }
    }

    #[test]
    fn your_own_corpse_gives_back_what_each_thing_was_already_for() {
        // Everything on it comes back. What it is for is what it was
        // for: the ring meant for a counter is still meant for one, and
        // writing Keep over the lot is how a recovered pack becomes
        // unsellable for good.
        assert_eq!(recovered(Some(LootAction::Sell)), LootAction::Sell);
        assert_eq!(recovered(Some(LootAction::Salvage)), LootAction::Salvage);
        assert_eq!(recovered(Some(LootAction::Keep)), LootAction::Keep);
        // "Leave it" is not an answer about your own belongings, and
        // neither is silence.
        assert_eq!(recovered(Some(LootAction::Skip)), LootAction::Keep);
        assert_eq!(recovered(None), LootAction::Keep);
    }

    #[test]
    fn a_cap_counts_what_is_already_spoken_for_on_this_body() {
        // Four healing kits on one corpse and a rule that says keep two.
        // Judged against the pack alone, every one of the four is the
        // first, and all four come home.
        const KIT: u32 = 500;
        let mut claimed = Claimed::default();
        let carried = 0;
        let keep_up_to = 2;
        let mut taken = 0;
        for _ in 0..4 {
            if carried + claimed.of(KIT) < keep_up_to {
                claimed.take(KIT, 1);
                taken += 1;
            }
        }
        assert_eq!(taken, 2, "two, not four");
        assert_eq!(claimed.of(KIT), 2);
        assert_eq!(claimed.of(999), 0, "another kind is counted apart");
    }

    #[test]
    fn a_verdict_carries_what_the_thing_is_wanted_for() {
        // The whole reason it is not a bare `Take`: whoever picks the
        // item up needs to know what it was picked up for, and asking
        // the rules a second time gave a different answer whenever an
        // appraisal had landed or a cap had filled in between.
        let take = lying(1, Verdict::Take(LootAction::Salvage));
        assert_eq!(take.took(), Some(LootAction::Salvage));
        assert_eq!(lying(2, Verdict::Leave).took(), None);
        assert_eq!(lying(3, Verdict::MustAsk).took(), None);
    }

    #[test]
    fn what_is_wanted_is_everything_worth_taking_whatever_it_is_for() {
        let open = Open {
            items: vec![
                lying(1, Verdict::Take(LootAction::Keep)),
                lying(2, Verdict::Leave),
                lying(3, Verdict::Take(LootAction::Sell)),
                lying(4, Verdict::MustAsk),
            ],
            ..Default::default()
        };
        let wanted: Vec<u32> = open.wanted().map(|i| i.guid).collect();
        assert_eq!(wanted, vec![1, 3], "kept and sold alike are taken");
        let asked: Vec<u32> = open.unjudged().map(|i| i.guid).collect();
        assert_eq!(asked, vec![4]);
    }
}
