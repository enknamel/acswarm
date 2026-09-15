//! Keeping the pack tidy: pouring loose stacks together so that slots
//! are not wasted on them.
//!
//! Slots are the scarce thing in Asheron's Call, not weight. Buy five
//! scarabs and they arrive as a stack of five, sitting in their own slot
//! next to the fifteen already carried; loot four arrows off a corpse
//! and they land beside the two hundred in the pack. Nothing warns you.
//! An afternoon of this fills a pack with change while the character
//! believes it has room.
//!
//! So the rules pour stacks together whenever two of the same thing are
//! carried loose. A stack has a maximum -- a trade note holds 250, an
//! arrow 1,000 -- so a large enough pile still needs more than one
//! stack; what it must not need is eleven.
//!
//! The server takes one merge at a time and answers in its own time,
//! so [`next_merge`] returns a single move and is asked again once that
//! move has landed. Reading that answer is [`pour_answer`]'s job: until
//! a pour is known to have landed or been turned down, the counts the
//! next choice would be made from are the ones before it.

use std::time::{Duration, Instant};

/// One stack in the pack, as the compactor needs to see it.
#[derive(Clone, Debug, PartialEq)]
pub struct Stack {
    pub guid: u32,
    /// What it is. Two stacks merge only if these match: an Arrow and a
    /// Broadhead Arrow look alike and are not.
    pub wcid: u32,
    pub name: String,
    /// How many are in it.
    pub count: u32,
    /// How many it could hold. 1 (or 0) is something that does not
    /// stack at all.
    pub max: u32,
    /// It is in the character's hands rather than the pack: arrows in
    /// the quiver slot, say. A wielded stack can be topped up, but is
    /// never the one poured away.
    pub wielded: bool,
    /// What the whole stack weighs, in burden units, or 0 when the
    /// server has not said.
    ///
    /// This is here for one reason. Pouring one stack into another
    /// changes what a character carries by nothing at all, but the
    /// server checks the pour as though the source were being picked
    /// up for the first time -- carried weight plus the source's whole
    /// weight against the ceiling -- and refuses it without a word
    /// when that does not fit. So a character near its limit cannot
    /// tidy, and must be told so rather than left asking.
    pub burden: u32,
}

impl Stack {
    /// Whether it stacks at all.
    fn stackable(&self) -> bool {
        self.max > 1
    }

    /// Room left in it.
    fn room(&self) -> u32 {
        self.max.saturating_sub(self.count)
    }
}

/// One stack poured into another.
#[derive(Clone, Debug, PartialEq)]
pub struct Merge {
    pub from: u32,
    pub to: u32,
    /// How many to move. Never more than the source holds, and never
    /// more than the target has room for: the server refuses both.
    pub amount: u32,
    /// What is being poured, for the log.
    pub name: String,
    /// The move empties the source, freeing its slot.
    pub frees_a_slot: bool,
}

/// The next merge worth making, or `None` when the pack is as tight as
/// it goes.
///
/// Of all the merges available, the one that frees a slot outright is
/// taken first: that is the whole point of the exercise, and a move
/// that only shuffles counts between two stacks can wait. Among equals
/// the largest pour wins, so the pack settles in as few round trips to
/// the server as it can, and ties go to the lowest guid so that the
/// answer does not wander between frames.
pub fn next_merge(stacks: &[Stack]) -> Option<Merge> {
    next_merge_unless(stacks, u32::MAX, |_, _| false)
}

/// The same, within a weight budget and skipping pairs the caller has
/// been told no about.
///
/// `may_carry` is how much more the server believes the character may
/// be handed. A pour it reckons too heavy is refused silently, so a
/// pour that would not fit is not worth asking for.
///
/// The skip is the other half: a pair the server will not join must
/// not stop the rest of the pack being tidied. Without it, one
/// stubborn pair meant nothing else was ever poured together, because
/// it is the only answer ever offered.
pub fn next_merge_unless(
    stacks: &[Stack],
    may_carry: u32,
    skip: impl Fn(u32, u32) -> bool,
) -> Option<Merge> {
    let mut best: Option<Merge> = None;
    for from in stacks {
        // A stack in the character's hands stays there, and a full one
        // has nothing spare to give.
        if !from.stackable() || from.wielded || from.count == 0 {
            continue;
        }
        // Nothing known about the weight is not a reason to refuse:
        // ask, and let the server's answer settle it.
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
            // Pour the smaller into the larger. Without this the pair
            // would swap back and forth for ever, each frame deciding
            // the other way round.
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
    /// What the target held when the pour went out. The answer is read
    /// off how much the target has grown, so this is half of it.
    pub to_before: u32,
}

/// What became of a pour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PourAnswer {
    /// No word yet.
    InAir,
    /// The two stacks are one.
    Landed,
    /// The server will not make it, with whatever reason it gave (0 for
    /// the half of them that carry none).
    Refused(u32),
    /// Long enough with no word at all that none is coming.
    Lost,
}

/// How long a pour within the pack is waited on before it is given up
/// for lost.
///
/// The server's answer is two or three messages in the same breath as
/// the merge, so a second is already generous; what this is really for
/// is the pour that is answered with nothing whatever -- a stack whose
/// size the server failed to adjust, or a packet that never arrived --
/// where the alternative is a character that never tidies again.
///
/// Within the pack only. A pour off a body is walked to and stooped
/// for like a take (ACE's `HandleActionStackableMerge` runs the same
/// move-to and pickup chain as a put when either stack is in the
/// world), and one take in ten was answered after two seconds; given
/// up at two, the source still lying there was asked for again and a
/// second merge went out behind the first. Such a pour is given a
/// take's allowance instead (see [`pour_answer_within`]).
pub const POUR_LOST: Duration = Duration::from_secs(2);

/// What the server has said about a pour within the pack, given what
/// the target holds now (`target_now`, `None` when the target itself is
/// gone), any refusal that names either stack, and how long since it
/// was asked for. Lost after [`POUR_LOST`].
pub fn pour_answer(
    sent: &PourSent,
    target_now: Option<u32>,
    refusal: Option<u32>,
    waited: Duration,
) -> PourAnswer {
    pour_answer_within(sent, target_now, refusal, waited, POUR_LOST)
}

/// What the server has said about a pour, given what the target holds
/// now (`target_now`, `None` when the target itself is gone), any
/// refusal that names either stack, how long since it was asked for,
/// and how long it is waited on before it is given up for lost -- a
/// take's allowance for a pour off a body, [`POUR_LOST`] within the
/// pack.
///
/// The target's count is the signal, not the source's. The server sends
/// the target's new size last for both a whole pour and a partial one,
/// so a choice made on "the source is gone" would be made from a target
/// count that is still the old one, and would offer the same pour again.
pub fn pour_answer_within(
    sent: &PourSent,
    target_now: Option<u32>,
    refusal: Option<u32>,
    waited: Duration,
    lost_after: Duration,
) -> PourAnswer {
    // A refusal is the whole answer, whatever the counts say: a stack
    // that grew by the right amount in the same moment grew for some
    // other reason.
    if let Some(code) = refusal {
        return PourAnswer::Refused(code);
    }
    // The target gone -- poured on somewhere else, sold, handed over --
    // is nothing this pour can still be waiting for.
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

/// The refusal that answers a pour sent at `sent_at`, out of what the
/// server last said about the source (`from`) and the target (`to`),
/// each a reason and when it was said.
///
/// Both stacks are looked at because the server names the target in a
/// good few of its refusals -- a stack that is stuck, one that does not
/// stack, one in a trade window -- and a tidier that only ever read the
/// source's entry never saw those at all.
///
/// The stamp is what makes it this pour's answer. Refusals are kept by
/// guid and never expire, and a looted item keeps its guid once it is in
/// the pack, so the entry sitting there may be about something that
/// happened minutes ago.
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

/// Whether `a` is a better move than what has been found so far.
fn better(a: &Merge, best: &Option<Merge>) -> bool {
    let Some(b) = best else { return true };
    (a.frees_a_slot, a.amount, b.from, b.to).gt(&(b.frees_a_slot, b.amount, a.from, a.to))
}

/// How many slots the stacks are using, and how few they could use.
/// What the tidying is worth, for the log and the UI.
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
            // Weightless, so the tests that are about slots stay
            // about slots. The weight budget has tests of its own.
            burden: 0,
        }
    }

    #[test]
    fn five_scarabs_bought_go_into_the_fifteen_already_carried() {
        // The case that started this: a purchase arrives in its own
        // slot beside what is already there.
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
        // Two swords are two swords, however alike.
        let pack = [stack(1, 555, 1, 1), stack(2, 555, 1, 1)];
        assert_eq!(next_merge(&pack), None);
        let unstackable = [stack(1, 555, 1, 0), stack(2, 555, 1, 0)];
        assert_eq!(next_merge(&unstackable), None);
    }

    #[test]
    fn different_things_never_merge() {
        // An Arrow and a Broadhead Arrow look alike and are not.
        let pack = [stack(1, 300, 50, 1000), stack(2, 301, 50, 1000)];
        assert_eq!(next_merge(&pack), None);
    }

    #[test]
    fn a_full_stack_is_not_poured_into() {
        let pack = [stack(1, 690, 100, 100), stack(2, 690, 5, 100)];
        // The only stack with room is the small one, and pouring the
        // full one into it would free nothing.
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
        // Two things to do: top up a nearly-full note stack (frees
        // nothing), or empty a stray taper stack (frees a slot).
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
        // Run the moves out to the end: it must terminate, and the
        // result must be as few stacks as the maximum allows.
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
        // Asked twice about the same pack, the same move comes back;
        // and the order the stacks arrive in does not change it.
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
        // The report this was written for: three stacks of the same
        // component in the pack where one would do.
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
        // The tapers were never part of it.
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
        // 180 at a hundred to a stack is two, and no pour spilled.
        assert_eq!(pack.len(), 2);
        assert_eq!(pack.iter().map(|s| s.count).sum::<u32>(), 180);
        assert_eq!(slots_wasted(&pack), 0);
    }

    #[test]
    fn a_pair_the_server_turned_down_does_not_hold_up_the_lead_peas() {
        // The pyreal pair is the better move and the server will not
        // make it. Skipped, the peas are offered instead of nothing.
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
        // The money counted out for a runner, say: another part of the
        // client is holding that guid across ticks.
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
        // And with both of that pair held, nothing of theirs moves.
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
        // A partial pour: five of the forty went into a stack with
        // only five slots left.
        let partial = sent(95, 5);
        assert_eq!(
            pour_answer(&partial, Some(100), None, Duration::ZERO),
            PourAnswer::Landed
        );
    }

    #[test]
    fn a_pour_is_in_the_air_until_the_target_has_grown() {
        // The server forgets the source before it says what the target
        // holds, and reading the first as the answer would choose the
        // next pour from a stale count.
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
        // Walked to and stooped for like a take: one in ten was
        // answered after two seconds. Given up then, the coin still on
        // the body was asked for again behind the first merge.
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
        // Landed and refused are read the same whatever the allowance.
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
        // A spell eats one of the target's peas while the pour is out.
        // One short of the mark is not the mark.
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
        // Refusals are kept by guid and never expire, and a looted
        // stack keeps the guid it was refused under.
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
        // A single stack wastes nothing, whatever its size.
        assert_eq!(slots_wasted(&[stack(1, 693, 3, 25)]), 0);
        assert_eq!(slots_wasted(&[]), 0);
    }
}
