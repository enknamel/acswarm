//! Pouring loose stacks together so slots are not wasted: slots, not weight, are what runs out.
//! [`next_merge`] picks one pour at a time, as the server takes one merge and answers in its own time.
//! [`pour_answer`] reads that answer; until a pour lands or is refused, the counts are from before it.
//! How a thing lying loose is taken into a pack lives in `room`.

use std::time::{Duration, Instant};

/// One stack in the pack, as the compactor sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct Stack {
    pub guid: u32,
    /// Two stacks merge only if these match: an Arrow and a Broadhead Arrow look alike and are not.
    pub wcid: u32,
    pub name: String,
    pub count: u32,
    /// Most it can hold (a trade note 250, an arrow 1,000); 1 or 0 does not stack.
    pub max: u32,
    /// In the character's hands (arrows in the quiver): may be topped up, never poured away.
    pub wielded: bool,
    /// The whole stack's weight in burden units, 0 when the server has not said.
    /// The server checks a pour like a first pickup (carried plus this vs. the ceiling), refusing silently.
    pub burden: u32,
}

impl Stack {
    fn stackable(&self) -> bool {
        self.max > 1
    }

    fn room(&self) -> u32 {
        self.max.saturating_sub(self.count)
    }
}

/// One stack poured into another.
#[derive(Clone, Debug, PartialEq)]
pub struct Merge {
    pub from: u32,
    pub to: u32,
    /// Never more than the source holds or the target has room for: the server refuses both.
    pub amount: u32,
    /// What is being poured, for the log.
    pub name: String,
    /// The move empties the source.
    pub frees_a_slot: bool,
}

/// The next merge worth making; `None` when the pack is as tight as it goes.
/// Slot-freeing pours first, then the largest (fewest round trips); ties to the lowest guid so frames agree.
pub fn next_merge(stacks: &[Stack]) -> Option<Merge> {
    next_merge_unless(stacks, u32::MAX, |_, _| false)
}

/// [`next_merge`] within `may_carry` more burden units, skipping the (from, to) guid pairs `skip` names.
/// The server refuses a too-heavy pour silently, and one pair it will not join must not stall the rest.
pub fn next_merge_unless(
    stacks: &[Stack],
    may_carry: u32,
    skip: impl Fn(u32, u32) -> bool,
) -> Option<Merge> {
    let mut best: Option<Merge> = None;
    for from in stacks {
        // Wielded stacks stay in the hands, and an empty one has nothing to give.
        if !from.stackable() || from.wielded || from.count == 0 {
            continue;
        }
        // Unknown weight (0) is no reason to refuse: ask, and let the server settle it.
        if from.burden > 0 && from.burden > may_carry {
            continue;
        }
        for to in stacks {
            if to.guid == from.guid || to.wcid != from.wcid || !to.stackable() {
                continue;
            }
            let room = to.room();
            if room == 0 {
                continue;
            }
            let amount = from.count.min(room);
            if amount == 0 {
                continue;
            }
            // Smaller into larger only, or a pair swaps back and forth, each frame deciding the other way.
            if (to.count, to.guid) <= (from.count, from.guid) {
                continue;
            }
            if skip(from.guid, to.guid) {
                continue;
            }
            let candidate = Merge {
                from: from.guid,
                to: to.guid,
                amount,
                name: from.name.clone(),
                frees_a_slot: amount == from.count,
            };
            if better(&candidate, &best) {
                best = Some(candidate);
            }
        }
    }
    best
}

/// A pour asked for and not yet answered.
#[derive(Clone, Debug, PartialEq)]
pub struct PourSent {
    pub merge: Merge,
    /// The target's count when the pour went out; the answer is read off how much it has grown.
    pub to_before: u32,
}

/// What became of a pour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PourAnswer {
    /// No word yet.
    InAir,
    /// The two stacks are one.
    Landed,
    /// The server will not make it, with its reason (0 for the half of refusals that carry none).
    Refused(u32),
    /// No word for so long that none is coming.
    Lost,
}

/// A pour within the pack unanswered this long is lost, so a reply that never comes cannot stop tidying.
/// Generous: the reply is two or three messages sent with the merge.
/// Off a body ACE walks and stoops as for a take (`HandleActionStackableMerge`, Player_Inventory.cs:2849),
/// and one take in ten is answered after 2 s (measured), so that pour waits a take's allowance instead.
pub const POUR_LOST: Duration = Duration::from_secs(2);

/// [`pour_answer_within`] for a pour within the pack, lost after [`POUR_LOST`].
pub fn pour_answer(
    sent: &PourSent,
    target_now: Option<u32>,
    refusal: Option<u32>,
    waited: Duration,
) -> PourAnswer {
    pour_answer_within(sent, target_now, refusal, waited, POUR_LOST)
}

/// What the server has said about a pour; `target_now` is `None` once the target itself is gone.
/// Read off the target, not the source: the server sends the target's new size last, whole pour or partial.
pub fn pour_answer_within(
    sent: &PourSent,
    target_now: Option<u32>,
    refusal: Option<u32>,
    waited: Duration,
    lost_after: Duration,
) -> PourAnswer {
    // A refusal wins over the counts: a stack that grew at the same moment grew for another reason.
    if let Some(code) = refusal {
        return PourAnswer::Refused(code);
    }
    // A target gone (poured elsewhere, sold, handed over) leaves nothing to wait for.
    let Some(count) = target_now else {
        return PourAnswer::Landed;
    };
    if count >= sent.to_before.saturating_add(sent.merge.amount) {
        return PourAnswer::Landed;
    }
    if waited >= lost_after {
        return PourAnswer::Lost;
    }
    PourAnswer::InAir
}

/// The refusal answering a pour sent at `sent_at`, out of the last (code, when) for source and target.
/// Both stacks: many refusals name the target (stuck, does not stack, in a trade window).
/// Only a stamp from `sent_at` on counts: refusals are kept by guid, never expire, and loot keeps its guid.
pub fn refusal_of(
    sent_at: Instant,
    from: Option<(u32, Instant)>,
    to: Option<(u32, Instant)>,
) -> Option<u32> {
    [from, to]
        .into_iter()
        .flatten()
        .find(|(_, when)| *when >= sent_at)
        .map(|(code, _)| code)
}

/// Whether `a` beats the best found so far.
fn better(a: &Merge, best: &Option<Merge>) -> bool {
    let Some(b) = best else { return true };
    (a.frees_a_slot, a.amount, b.from, b.to).gt(&(b.frees_a_slot, b.amount, a.from, a.to))
}

/// Slots the pack's stacks use beyond the fewest they could: what tidying is worth, for the log and UI.
pub fn slots_wasted(stacks: &[Stack]) -> u32 {
    use std::collections::BTreeMap;
    let mut piles: BTreeMap<u32, (u32, u32, u32)> = BTreeMap::new();
    for s in stacks {
        if !s.stackable() || s.wielded {
            continue;
        }
        let e = piles.entry(s.wcid).or_insert((0, 0, s.max));
        e.0 += s.count;
        e.1 += 1;
    }
    piles
        .values()
        .map(|(total, used, max)| {
            let needed = total.div_ceil((*max).max(1));
            used.saturating_sub(needed)
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(guid: u32, wcid: u32, count: u32, max: u32) -> Stack {
        Stack {
            guid,
            wcid,
            name: format!("thing {wcid}"),
            count,
            max,
            wielded: false,
            // Weight unknown, so the tests about slots stay about slots.
            burden: 0,
        }
    }

    #[test]
    fn five_scarabs_bought_go_into_the_fifteen_already_carried() {
        // A purchase arrives in its own slot beside the stack already carried.
        let pack = [stack(1, 690, 15, 100), stack(2, 690, 5, 100)];
        let m = next_merge(&pack).expect("a merge");
        assert_eq!(m.from, 2, "poured the big stack into the small one");
        assert_eq!(m.to, 1);
        assert_eq!(m.amount, 5);
        assert!(m.frees_a_slot);
    }

    #[test]
    fn a_tidy_pack_has_nothing_to_do() {
        let pack = [stack(1, 690, 100, 100), stack(2, 693, 40, 100)];
        assert_eq!(next_merge(&pack), None);
        assert_eq!(slots_wasted(&pack), 0);
    }

    #[test]
    fn things_that_do_not_stack_are_left_alone() {
        let pack = [stack(1, 555, 1, 1), stack(2, 555, 1, 1)];
        assert_eq!(next_merge(&pack), None);
        let unstackable = [stack(1, 555, 1, 0), stack(2, 555, 1, 0)];
        assert_eq!(next_merge(&unstackable), None);
    }

    #[test]
    fn different_things_never_merge() {
        let pack = [stack(1, 300, 50, 1000), stack(2, 301, 50, 1000)];
        assert_eq!(next_merge(&pack), None);
    }

    #[test]
    fn a_full_stack_is_not_poured_into() {
        let pack = [stack(1, 690, 100, 100), stack(2, 690, 5, 100)];
        assert_eq!(next_merge(&pack), None);
    }

    #[test]
    fn a_pour_never_overflows_the_target() {
        // 250 to a stack: 200 already there takes 50 of the 90 offered.
        let pack = [stack(1, 2621, 200, 250), stack(2, 2621, 90, 250)];
        let m = next_merge(&pack).expect("a merge");
        assert_eq!(m.amount, 50);
        assert!(!m.frees_a_slot, "40 are left behind");
    }

    #[test]
    fn freeing_a_slot_comes_before_shuffling_counts() {
        // Topping up the note stack frees nothing; emptying the stray taper stack frees a slot.
        let pack = [
            stack(1, 2621, 240, 250),
            stack(2, 2621, 200, 250),
            stack(3, 20631, 300, 500),
            stack(4, 20631, 20, 500),
        ];
        let m = next_merge(&pack).expect("a merge");
        assert!(m.frees_a_slot, "{m:?}");
        assert_eq!(m.from, 4);
        assert_eq!(m.to, 3);
    }

    #[test]
    fn the_pack_settles_and_stays_settled() {
        let mut pack = vec![
            stack(1, 690, 15, 100),
            stack(2, 690, 5, 100),
            stack(3, 690, 90, 100),
            stack(4, 690, 40, 100),
            stack(5, 693, 7, 25),
            stack(6, 693, 7, 25),
            stack(7, 693, 7, 25),
            stack(8, 693, 7, 25),
        ];
        let mut moves = 0;
        while let Some(m) = next_merge(&pack) {
            moves += 1;
            assert!(moves < 100, "the pack never settles");
            let from = pack.iter().position(|s| s.guid == m.from).unwrap();
            let to = pack.iter().position(|s| s.guid == m.to).unwrap();
            assert!(pack[to].count + m.amount <= pack[to].max, "overflowed");
            pack[from].count -= m.amount;
            pack[to].count += m.amount;
            pack.retain(|s| s.count > 0);
        }
        // 150 scarabs at 100 to a stack is two; 28 tapers at 25 is two.
        assert_eq!(pack.iter().filter(|s| s.wcid == 690).count(), 2);
        assert_eq!(pack.iter().filter(|s| s.wcid == 693).count(), 2);
        assert_eq!(pack.iter().map(|s| s.count).sum::<u32>(), 150 + 28);
        assert_eq!(slots_wasted(&pack), 0);
    }

    #[test]
    fn the_answer_does_not_wander_between_frames() {
        let mut pack = vec![
            stack(1, 690, 15, 100),
            stack(2, 690, 5, 100),
            stack(3, 690, 40, 100),
        ];
        let first = next_merge(&pack).expect("a merge");
        assert_eq!(next_merge(&pack), Some(first.clone()));
        pack.reverse();
        assert_eq!(next_merge(&pack), Some(first));
    }

    #[test]
    fn arrows_in_the_quiver_are_topped_up_but_never_emptied() {
        let mut quiver = stack(1, 300, 100, 1000);
        quiver.wielded = true;
        let pack = [quiver, stack(2, 300, 50, 1000)];
        let m = next_merge(&pack).expect("a merge");
        assert_eq!(m.from, 2, "took the arrows out of the quiver");
        assert_eq!(m.to, 1);
        assert_eq!(m.amount, 50);
    }

    #[test]
    fn lead_peas_looted_into_stacks_of_their_own_are_poured_into_one() {
        let mut pack = vec![
            stack(1, 8329, 40, 100),
            stack(2, 8329, 3, 100),
            stack(3, 8329, 5, 100),
            stack(4, 693, 12, 25),
        ];
        let mut moves = 0;
        while let Some(m) = next_merge(&pack) {
            moves += 1;
            assert!(moves < 100, "the pack never settles");
            assert!(m.frees_a_slot, "every pour here empties its stack");
            let from = pack.iter().position(|s| s.guid == m.from).unwrap();
            let to = pack.iter().position(|s| s.guid == m.to).unwrap();
            pack[from].count -= m.amount;
            pack[to].count += m.amount;
            pack.retain(|s| s.count > 0);
        }
        assert_eq!(moves, 2, "two pours for three stacks");
        let peas: Vec<&Stack> = pack.iter().filter(|s| s.wcid == 8329).collect();
        assert_eq!(peas.len(), 1);
        assert_eq!(peas[0].count, 48);
        assert_eq!(pack.iter().filter(|s| s.wcid == 693).count(), 1);
    }

    #[test]
    fn a_pile_of_lead_peas_bigger_than_a_stack_settles_into_as_few_stacks_as_it_needs() {
        let mut pack = vec![
            stack(1, 8329, 90, 100),
            stack(2, 8329, 60, 100),
            stack(3, 8329, 30, 100),
        ];
        let mut moves = 0;
        while let Some(m) = next_merge(&pack) {
            moves += 1;
            assert!(moves < 100, "the pack never settles");
            let from = pack.iter().position(|s| s.guid == m.from).unwrap();
            let to = pack.iter().position(|s| s.guid == m.to).unwrap();
            assert!(pack[to].count + m.amount <= pack[to].max, "overflowed");
            pack[from].count -= m.amount;
            pack[to].count += m.amount;
            pack.retain(|s| s.count > 0);
        }
        // 180 at 100 to a stack is two.
        assert_eq!(pack.len(), 2);
        assert_eq!(pack.iter().map(|s| s.count).sum::<u32>(), 180);
        assert_eq!(slots_wasted(&pack), 0);
    }

    #[test]
    fn a_pair_the_server_turned_down_does_not_hold_up_the_lead_peas() {
        // The pyreal pair is the better move and the server refuses it; skipped, the peas are offered.
        let pack = [
            stack(1, 273, 5000, 25000),
            stack(2, 273, 900, 25000),
            stack(3, 8329, 40, 100),
            stack(4, 8329, 5, 100),
        ];
        let held = next_merge(&pack).expect("a merge");
        assert_eq!((held.from, held.to), (2, 1), "the pyreals are the pick");
        let m = next_merge_unless(&pack, u32::MAX, |from, to| (from, to) == (2, 1))
            .expect("something else to pour");
        assert_eq!((m.from, m.to), (4, 3));
    }

    #[test]
    fn a_stack_an_errand_holds_is_neither_poured_away_nor_poured_into() {
        let pack = [
            stack(1, 273, 5000, 25000),
            stack(2, 273, 900, 25000),
            stack(3, 8329, 40, 100),
            stack(4, 8329, 5, 100),
        ];
        let errand = 1;
        let m = next_merge_unless(&pack, u32::MAX, |from, to| from == errand || to == errand)
            .expect("the other pair is still offered");
        assert_eq!((m.from, m.to), (4, 3));
        let m = next_merge_unless(&pack, u32::MAX, |from, to| [from, to].contains(&2));
        assert_eq!(m.map(|m| (m.from, m.to)), Some((4, 3)));
    }

    /// A pour of `amount` into a target that held `to_before`.
    fn sent(to_before: u32, amount: u32) -> PourSent {
        PourSent {
            merge: Merge {
                from: 2,
                to: 1,
                amount,
                name: "Lead Pea".into(),
                frees_a_slot: true,
            },
            to_before,
        }
    }

    #[test]
    fn a_pour_has_landed_once_the_target_has_grown_by_the_amount() {
        let whole = sent(40, 5);
        assert_eq!(
            pour_answer(&whole, Some(45), None, Duration::ZERO),
            PourAnswer::Landed
        );
        // A partial pour: five into a target with room for only five.
        let partial = sent(95, 5);
        assert_eq!(
            pour_answer(&partial, Some(100), None, Duration::ZERO),
            PourAnswer::Landed
        );
    }

    #[test]
    fn a_pour_is_in_the_air_until_the_target_has_grown() {
        // The server forgets the source before it says what the target holds; reading the first as
        // the answer would choose the next pour from a stale count.
        let s = sent(40, 5);
        assert_eq!(
            pour_answer(&s, Some(40), None, Duration::from_millis(100)),
            PourAnswer::InAir
        );
    }

    #[test]
    fn a_pour_whose_target_has_gone_is_over() {
        let s = sent(40, 5);
        assert_eq!(
            pour_answer(&s, None, None, Duration::from_millis(100)),
            PourAnswer::Landed
        );
    }

    #[test]
    fn a_refusal_is_the_answer_even_if_the_count_moved() {
        let s = sent(40, 5);
        assert_eq!(
            pour_answer(&s, Some(45), Some(0), Duration::ZERO),
            PourAnswer::Refused(0)
        );
    }

    #[test]
    fn a_pour_with_no_word_for_two_seconds_is_lost() {
        let s = sent(40, 5);
        assert_eq!(pour_answer(&s, Some(40), None, POUR_LOST), PourAnswer::Lost);
        assert_eq!(
            pour_answer(&s, Some(40), None, POUR_LOST - Duration::from_millis(1)),
            PourAnswer::InAir
        );
    }

    #[test]
    fn a_pour_off_a_body_is_waited_on_as_long_as_a_take_would_be() {
        // Walked and stooped for like a take, one in ten answered after 2 s: given up at 2 s, the coin
        // still on the body is asked for again behind the first merge.
        let s = sent(40, 5);
        let takes = Duration::from_secs(4);
        assert_eq!(
            pour_answer_within(&s, Some(40), None, Duration::from_secs(3), takes),
            PourAnswer::InAir
        );
        assert_eq!(
            pour_answer_within(&s, Some(40), None, takes, takes),
            PourAnswer::Lost
        );
        assert_eq!(
            pour_answer_within(&s, Some(45), None, Duration::from_secs(3), takes),
            PourAnswer::Landed
        );
        assert_eq!(
            pour_answer_within(&s, Some(40), Some(0), Duration::ZERO, takes),
            PourAnswer::Refused(0)
        );
    }

    #[test]
    fn a_component_burned_from_the_target_mid_pour_is_not_taken_for_the_answer() {
        // A spell eats one of the target's peas mid-pour: one short of the mark is not the mark.
        let s = sent(40, 5);
        assert_eq!(
            pour_answer(&s, Some(44), None, Duration::from_millis(100)),
            PourAnswer::InAir
        );
        assert_eq!(pour_answer(&s, Some(44), None, POUR_LOST), PourAnswer::Lost);
    }

    #[test]
    fn a_refusal_naming_the_target_is_the_pours_answer() {
        let at = Instant::now();
        let after = at + Duration::from_millis(50);
        assert_eq!(refusal_of(at, None, Some((0x29, after))), Some(0x29));
        // The source comes first when both have something to say.
        assert_eq!(
            refusal_of(at, Some((0x427, after)), Some((0x29, after))),
            Some(0x427)
        );
    }

    #[test]
    fn a_refusal_from_before_the_pour_is_not_its_answer() {
        // Refusals are kept by guid and never expire; a looted stack keeps the guid it was refused under.
        let before = Instant::now();
        let at = before + Duration::from_secs(30);
        assert_eq!(refusal_of(at, Some((0x427, before)), None), None);
        assert_eq!(refusal_of(at, None, Some((0x29, before))), None);
        assert_eq!(refusal_of(at, None, None), None);
    }

    #[test]
    fn what_the_tidying_is_worth_is_counted() {
        // Four stacks of seven tapers where two would do.
        let pack = [
            stack(1, 693, 7, 25),
            stack(2, 693, 7, 25),
            stack(3, 693, 7, 25),
            stack(4, 693, 7, 25),
        ];
        assert_eq!(slots_wasted(&pack), 2);
        assert_eq!(slots_wasted(&[stack(1, 693, 3, 25)]), 0);
        assert_eq!(slots_wasted(&[]), 0);
    }
}
