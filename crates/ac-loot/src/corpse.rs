//! What a corpse and the character looking into it look like to the rules.
//!
//! Plain values, so emptying one can be followed at a desk. Why an item is worth having is the
//! player's profile, judged elsewhere: it arrives as a [`Verdict`] carrying the decision, so
//! nothing downstream asks again and gets a different answer.

/// What the rules have been told about one thing in the corpse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Worth taking, and what for: pickup reads this rather than judging again, which disagrees
    /// once an appraisal lands or a cap fills (`a_verdict_carries_what_the_thing_is_wanted_for`).
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
    /// Fits whole on a carried stack (`ac_agent::room::how_to_take`): a full pack still takes it.
    pub needs_no_slot: bool,
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
    /// How far off the character is standing, in metres.
    pub away: f32,
    /// The window is open and its contents are known.
    pub open: bool,
    pub items: Vec<Lying>,
    /// Free slots in the roomiest pack (`ac_agent::room::Packs::for_a_take`), not the sum:
    /// a take goes into one pack.
    pub slots_free: u32,
    /// Free slots over every pack (`ac_agent::room::Packs::anywhere`): where a counter's coin goes.
    pub room_anywhere: u32,
    /// Slots left empty for a counter's coin, which it needs before it takes anything.
    /// Counted over every pack: the server fills the main pack, then each side pack in turn.
    pub keep_free: u32,
    /// Burden of loot it still means to carry (`growth::carry_room`); 0 once it has had enough.
    /// Worn, wielded and kept things count only against twice capacity and the server's wall.
    pub carry_room: u32,
    /// Asking the server to identify things is allowed.
    pub may_ask: bool,
    /// An identify is already out for these.
    pub asking: Vec<u32>,
    /// Takes the server turned down since the corpse was asked to open.
    /// It names the item and the item stays: too encumbered, a rate-limited drop, a unique carried.
    pub refused: Vec<u32>,
    /// Listed on the corpse but not yet described.
    /// The server sends the list first and each description a moment later.
    pub arriving: Vec<u32>,
}

/// How near, in metres, the character must be before a corpse will open for it.
pub const REACH: f32 = 2.5;

/// What something taken off the character's own corpse is for: the profile's answer, else Keep.
/// Keep for the lot would overwrite each Sell and Salvage already decided and clog the pack.
pub fn recovered(judged: Option<crate::profile::LootAction>) -> crate::profile::LootAction {
    match judged {
        Some(a) if a.takes() => a,
        // Unclaimed, or Skip: leaving is no answer about the character's own belongings.
        _ => crate::profile::LootAction::Keep,
    }
}

/// How many of each wcid have been claimed off this body so far.
/// A cap counts these too: none has reached the pack yet when the next copy is judged.
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
    /// The things worth taking, in the corpse's own order, not dearest first:
    /// the server moves one at a time and the character wants the lot.
    pub fn wanted(&self) -> impl Iterator<Item = &Lying> {
        self.items
            .iter()
            .filter(|i| matches!(i.verdict, Verdict::Take(_)))
    }

    /// The things only the server can settle, not already being asked about.
    /// Only these are identified: each identify is a round trip before anything is picked up.
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
            needs_no_slot: false,
        }
    }

    #[test]
    fn your_own_corpse_gives_back_what_each_thing_was_already_for() {
        // Each thing keeps what it was for: Keep over the lot makes a recovered pack unsellable.
        assert_eq!(recovered(Some(LootAction::Sell)), LootAction::Sell);
        assert_eq!(recovered(Some(LootAction::Salvage)), LootAction::Salvage);
        assert_eq!(recovered(Some(LootAction::Keep)), LootAction::Keep);
        // Neither "leave it" nor silence is an answer about the character's own belongings.
        assert_eq!(recovered(Some(LootAction::Skip)), LootAction::Keep);
        assert_eq!(recovered(None), LootAction::Keep);
    }

    #[test]
    fn a_cap_counts_what_is_already_spoken_for_on_this_body() {
        // Four kits on one corpse, a rule keeping two: judged against the pack alone, all four come.
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
        // Pickup needs what the item was taken for; asking the rules again answers differently once
        // an appraisal lands or a cap fills in between.
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
