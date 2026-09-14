//! Emptying one corpse, one decision at a time.

use std::time::{Duration, Instant};

use ac_agent::did::{Because, Did};

use crate::corpse::{Lying, Open, Verdict, REACH};

/// How long an item asked for and not moved is left before asking
/// again. Not the pace of the looting -- that is set by the corpse
/// answering -- only how long a lost request is left.
pub const TAKE_AGAIN: Duration = Duration::from_millis(400);
/// How long to keep at one corpse before leaving it be. A corpse
/// belongs to whoever killed it for a while, so one that will not give
/// up its contents now may well later.
pub const KEEP_AT_IT: Duration = Duration::from_secs(45);
/// How long an item asked for may lie there before it is taken to be
/// not coming. The client sends a lost take again after four seconds, so
/// this is two asks and a little over, not one slow answer -- and by
/// then nothing asked for is still in the air.
pub const NOT_COMING: Duration = Duration::from_secs(10);
/// A gap this long between two turns at a corpse means something else
/// had the character meanwhile -- a fight that came to it -- and that
/// time is not counted against the corpse. The rules are consulted many
/// times a second while they have the character.
pub const BROKEN_OFF: Duration = Duration::from_secs(3);

/// The one thing the rules want done next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    /// Walk to it: a corpse will not open across a room.
    Approach,
    /// Ask it to open.
    Open,
    /// Ask the server what these are. Several at once: it answers each
    /// on its own, and asking one at a time was most of what made
    /// standing over a body slow.
    Ask(Vec<u32>),
    /// Take this one.
    Take(u32),
    /// Shut it. A corpse left open is one the server still has the
    /// character standing over, and the next cannot be opened until it
    /// is let go.
    Close,
}

/// What came of a turn.
#[derive(Clone, Debug, PartialEq)]
pub struct Next {
    pub act: Option<Act>,
    pub did: Did,
    pub saying: String,
    /// When the corpse is shut for want of room to carry the rest: what
    /// the lightest thing left on it weighs. Room for that is what the
    /// body is waiting on, and until the character has it, going back
    /// to the body takes nothing.
    pub left_for_weight: Option<u32>,
}

impl Next {
    fn act(act: Act, saying: impl Into<String>) -> Next {
        Next {
            act: Some(act),
            did: Did::Acting,
            saying: saying.into(),
            left_for_weight: None,
        }
    }

    fn done(saying: impl Into<String>) -> Next {
        Next {
            act: Some(Act::Close),
            did: Did::Done,
            saying: saying.into(),
            left_for_weight: None,
        }
    }

    fn wait(why: &str) -> Next {
        Next {
            act: None,
            did: Did::waiting(why.to_string()),
            saying: why.to_string(),
            left_for_weight: None,
        }
    }

    /// Shut the corpse and set it aside, not written off.
    fn set_aside(because: &str, saying: impl Into<String>) -> Next {
        Next {
            act: Some(Act::Close),
            did: Did::Blocked(Because::ours(because)),
            saying: saying.into(),
            left_for_weight: None,
        }
    }
}

/// Emptying one corpse.
#[derive(Debug, Default)]
pub struct Run {
    /// The corpse this run is emptying.
    corpse: Option<u32>,
    /// The item last asked for: which, when it was first asked for, and
    /// when last.
    asked: Option<(u32, Instant, Instant)>,
    /// What was stepped over on this body because it would not come
    /// out: turned down, or asked for and not moved in [`NOT_COMING`].
    passed: Vec<u32>,
    /// When this corpse was first stood over, moved on by any time the
    /// character spent on something else (see [`BROKEN_OFF`]).
    began: Option<Instant>,
    /// When the rules were last consulted about it.
    last: Option<Instant>,
    /// How many things have been taken.
    pub taken: u32,
    /// Whether the corpse opened for it. A body walked to and let go
    /// before it did has not.
    pub opened: bool,
}

impl Run {
    pub fn new() -> Run {
        Run::default()
    }

    /// The next thing to do with this corpse, or why there is nothing.
    pub fn step(&mut self, at: &Open, now: Instant) -> Next {
        // A run is for one body, and a different body starts afresh.
        // The clock used to be reset only when a corpse was shut here,
        // so after one was let go any other way, the next one opened
        // more than forty-five seconds later was already "too long" on
        // its first step: shut at once, left full, and written off.
        if self.corpse != Some(at.guid) {
            *self = Run {
                corpse: Some(at.guid),
                ..Run::default()
            };
        }
        // The clock counts the time spent at this corpse, not the time
        // since it was begun. A pack of Drudges arriving mid-corpse has
        // the character for as long as the fight lasts, the corpse stays
        // open on the server all the while, and counting that time had
        // the body given up on the moment the pack was dead.
        let began = match (self.began, self.last) {
            (Some(began), Some(last)) if now.duration_since(last) > BROKEN_OFF => {
                began + now.duration_since(last)
            }
            (Some(began), _) => began,
            (None, _) => now,
        };
        self.began = Some(began);
        self.last = Some(now);

        if at.away > REACH {
            return Next::act(Act::Approach, format!("going to {}", at.name));
        }
        if !at.open {
            return Next::act(Act::Open, format!("opening {}", at.name));
        }
        self.opened = true;

        // No room to spare: leave it rather than stand here asking. The
        // body keeps for a while and will still be here once the pack has
        // been sold down, so it is set aside, not emptied. Shut as done
        // with, every body reached with a low pack was written off with
        // its coin and gems still on it, and never gone back to after the
        // sale. "No room" stops short of the last slot: the few kept free
        // are where a counter puts the money.
        //
        // Unless something on it needs no slot: coin poured onto the
        // pile carried spends nothing the counter wants, so that much is
        // still taken off a body the pack has no slot for.
        let has_a_slot = at.slots_free > at.keep_free;
        if !has_a_slot && !at.wanted().any(|i| i.needs_no_slot) {
            return Next::set_aside(
                "the pack is full",
                format!("pack full, leaving {} for now", at.name),
            );
        }
        // Carrying as much as it means to is not a reason to leave the
        // corpse unopened: only what will not fit stays. Shutting it
        // then and there left Blargerton's coin lying on every body he
        // killed, and wrote each one off as looted besides.

        // Long enough. A corpse that will not give up its contents is
        // set aside, not written off.
        if now.duration_since(began) > KEEP_AT_IT {
            return Next::set_aside(
                "it will not give up its contents",
                format!("leaving {} for now", at.name),
            );
        }

        // Not all here yet. Judged between the list and the things on
        // it, a body with loot on it read as empty: Blargerton shut it,
        // wrote it off as looted, and its items arrived a frame later.
        if !at.arriving.is_empty() {
            return Next::wait(&format!("opening {}", at.name));
        }

        // What cannot be judged without asking. Asked for in one go --
        // but not what could not be carried whatever it turned out to
        // be, or a laden character spends a round trip on every item of
        // every body only to leave it there.
        if at.may_ask {
            let ask: Vec<u32> = at
                .unjudged()
                .filter(|i| i.burden <= at.carry_room)
                .map(|i| i.guid)
                .collect();
            if !ask.is_empty() {
                let n = ask.len();
                return Next::act(Act::Ask(ask), format!("looking over {n} item(s)"));
            }
            // Still waiting to hear about some of them.
            if at
                .items
                .iter()
                .any(|i| i.verdict == Verdict::MustAsk && at.asking.contains(&i.guid))
            {
                return Next::wait("waiting to hear what they are");
            }
        }

        // Take them, in the order the corpse lists them.
        //
        // Paced by the corpse, not by a clock: a take is answered by
        // the item leaving, which is what takes it off this list, so
        // when the front has changed the last one is done and the next
        // goes at once.
        // The first thing worth taking that fits.
        //
        // Skipped is not the same as waiting, and conflating the two
        // is a bug with a name: the server moves one thing at a time
        // and answers a second with "Source item not found", so an item
        // that has been asked for and has not moved yet must be waited
        // on, never stepped over. Only what will not come at all is
        // skipped: heavier than the character can carry, turned down by
        // the server, or asked for and still lying there after
        // [`NOT_COMING`], by when nothing asked for is in the air.
        //
        // A take turned down used to be asked for again every four
        // hundred milliseconds -- a hundred times over for "You are too
        // encumbered to carry that!" -- and nothing listed after it was
        // ever taken before the corpse was given up on with all of it
        // still there.
        if let Some((guid, first, _)) = self.asked {
            let lying = at.items.iter().any(|i| i.guid == guid);
            if lying && (at.refused.contains(&guid) || now.duration_since(first) > NOT_COMING) {
                self.passed.push(guid);
                // Asked for, never taken.
                self.taken = self.taken.saturating_sub(1);
                self.asked = None;
            }
        }
        let given_up = |guid: u32| self.passed.contains(&guid) || at.refused.contains(&guid);
        // What could be taken now: light enough, not given up on, and
        // with somewhere to go -- a slot, or a pile it pours onto.
        let could_take = |i: &Lying| {
            i.burden <= at.carry_room && !given_up(i.guid) && (has_a_slot || i.needs_no_slot)
        };
        let next = at
            .wanted()
            .find(|i| could_take(i))
            .map(|i| (i.guid, i.name.clone()));
        if let Some((guid, name)) = next {
            let first = match self.asked {
                // Asked for and still lying there: wait for it.
                Some((last, first, when)) if last == guid => {
                    if now.duration_since(when) < TAKE_AGAIN {
                        return Next::wait("it has not come out yet");
                    }
                    first
                }
                _ => {
                    self.taken += 1;
                    now
                }
            };
            self.asked = Some((guid, first, now));
            return Next::act(Act::Take(guid), format!("taking {name}"));
        }

        // Everything with somewhere to go is out, and something it
        // wanted has nowhere: the pack ran out of slots over this body,
        // or the server said a pack the count believed had room was full.
        // Set aside for room, as a body reached with a full pack is, and
        // not reopened until there is some: reopened, it was refused the
        // same take thirty times over.
        if !has_a_slot
            && at
                .wanted()
                .any(|i| i.burden <= at.carry_room && !given_up(i.guid) && !i.needs_no_slot)
        {
            return Next::set_aside(
                "the pack is full",
                format!("pack full, leaving the rest of {} for now", at.name),
            );
        }

        // Everything else is out, and something it wanted would not
        // come. Set aside, not emptied: a quest's wait lifts, a unique
        // can be sold, and the body keeps for a while.
        if let Some(stuck) = at.wanted().find(|i| given_up(i.guid)) {
            return Next::set_aside(
                "it would not give something up",
                format!("{} would not come off {}", stuck.name, at.name),
            );
        }

        // Everything that fitted has been taken, and something it wanted
        // -- or might have, had it been light enough to ask about -- is
        // still lying there for its weight. Set aside, not emptied: once
        // the pack has been sold down it will fit, and the body keeps.
        // What the lightest of it weighs goes with the close: going back
        // before there is room for that takes nothing, and a character
        // with less room than that has had enough and goes to sell.
        let left_for_weight = at
            .items
            .iter()
            .filter(|i| i.verdict != Verdict::Leave && i.burden > at.carry_room)
            .map(|i| i.burden)
            .min();
        if let Some(lightest) = left_for_weight {
            return Next {
                left_for_weight: Some(lightest),
                ..Next::set_aside(
                    "too laden to take the rest",
                    format!("too laden to take the rest of {}", at.name),
                )
            };
        }
        // Done either way, but said apart. Every body Blargerton shut read
        // "emptied", the ones the profile turned everything down on as much
        // as the ones he cleared, so nobody watching could tell "took
        // nothing" from "did not loot".
        if self.taken == 0 {
            return Next::done(match at.items.len() {
                0 => format!("nothing on {}", at.name),
                n => format!("nothing worth taking on {} ({n} item(s))", at.name),
            });
        }
        Next::done(format!("emptied {}", at.name))
    }
}

/// Every body opened and every thing taken over many runs, for the
/// panel. Blargerton's log said "emptied" whether he took something or
/// nothing; the count says which.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    /// Corpses that opened.
    pub opened: u32,
    /// Things taken off them.
    pub taken: u32,
}

impl Tally {
    /// Count a run that is over, however its corpse was let go.
    pub fn count(&mut self, run: &Run) {
        self.opened += u32::from(run.opened);
        self.taken += run.taken;
    }

    /// The panel's line.
    pub fn line(&self) -> String {
        format!(
            "this session: {} corpse(s) opened, {} thing(s) taken",
            self.opened, self.taken
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::LootAction;

    fn thing(guid: u32, name: &str, verdict: Verdict) -> Lying {
        Lying {
            guid,
            name: name.into(),
            burden: 10,
            verdict,
            needs_no_slot: false,
        }
    }

    fn body(items: Vec<Lying>) -> Open {
        Open {
            guid: 900,
            name: "Corpse of a Drudge".into(),
            away: 0.0,
            open: true,
            items,
            slots_free: 20,
            keep_free: 0,
            carry_room: 10_000,
            may_ask: true,
            asking: Vec::new(),
            refused: Vec::new(),
            arriving: Vec::new(),
        }
    }

    #[test]
    fn it_walks_to_the_body_and_opens_it_before_anything_else() {
        let mut run = Run::new();
        let mut at = body(vec![thing(1, "Dagger", Verdict::Take(LootAction::Keep))]);
        at.away = 8.0;
        assert_eq!(run.step(&at, Instant::now()).act, Some(Act::Approach));
        at.away = 1.0;
        at.open = false;
        assert_eq!(run.step(&at, Instant::now()).act, Some(Act::Open));
    }

    #[test]
    fn only_what_cannot_be_judged_is_asked_about_and_all_at_once() {
        // One identify per item, waited on in turn, was most of the
        // time a character spent standing over a body. And a profile
        // that can settle an item on its name and worth should cost no
        // round trip at all.
        let mut run = Run::new();
        let at = body(vec![
            thing(1, "Dagger", Verdict::Take(LootAction::Keep)),
            thing(2, "Pyreal", Verdict::Take(LootAction::Keep)),
            thing(3, "Odd Ring", Verdict::MustAsk),
            thing(4, "Odd Gem", Verdict::MustAsk),
        ]);
        match run.step(&at, Instant::now()).act {
            Some(Act::Ask(ids)) => assert_eq!(ids, vec![3, 4], "asked about the wrong things"),
            other => panic!("expected one ask for both, got {other:?}"),
        }
    }

    #[test]
    fn waiting_on_an_appraisal_is_not_the_end_of_the_corpse() {
        // Nothing to do yet is said without an act, and never as a
        // close. The client once read "no act" as "done": it closed a
        // corpse the moment its items went off to be appraised, marked
        // it looted, and left four thousand pyreals of essence on it.
        let mut run = Run::new();
        let mut at = body(vec![
            thing(1, "Rock", Verdict::Leave),
            thing(3, "Frost Wisp Essence", Verdict::MustAsk),
        ]);
        at.asking = vec![3];
        let next = run.step(&at, Instant::now());
        assert_eq!(next.act, None, "{}", next.saying);
        assert!(matches!(next.did, Did::Waiting(_)), "{:?}", next.did);
    }

    #[test]
    fn a_corpse_it_can_judge_alone_is_never_asked_about() {
        let mut run = Run::new();
        let at = body(vec![
            thing(1, "Dagger", Verdict::Take(LootAction::Keep)),
            thing(2, "Rock", Verdict::Leave),
        ]);
        assert_eq!(run.step(&at, Instant::now()).act, Some(Act::Take(1)));
    }

    #[test]
    fn the_next_thing_goes_out_as_soon_as_the_last_one_leaves() {
        // The corpse answers by the item leaving, which is what takes
        // it off the list. Waiting four hundred milliseconds between
        // every item meant ten things took four seconds.
        let now = Instant::now();
        let mut run = Run::new();
        let at = body(vec![
            thing(1, "Dagger", Verdict::Take(LootAction::Keep)),
            thing(2, "Shield", Verdict::Take(LootAction::Keep)),
        ]);
        assert_eq!(run.step(&at, now).act, Some(Act::Take(1)));
        // It came out: the same instant, the next one goes.
        let at = body(vec![thing(2, "Shield", Verdict::Take(LootAction::Keep))]);
        assert_eq!(run.step(&at, now).act, Some(Act::Take(2)));
    }

    #[test]
    fn one_that_does_not_come_out_is_asked_again_then_left() {
        let now = Instant::now();
        let mut run = Run::new();
        let at = body(vec![
            thing(1, "Stuck Thing", Verdict::Take(LootAction::Keep)),
            thing(2, "Dagger", Verdict::Take(LootAction::Keep)),
        ]);
        assert_eq!(run.step(&at, now).act, Some(Act::Take(1)));
        // Still there a moment later: not asked again immediately.
        assert_eq!(run.step(&at, now).act, None);
        // After the wait, asked again.
        let later = now + TAKE_AGAIN + Duration::from_millis(1);
        assert_eq!(run.step(&at, later).act, Some(Act::Take(1)));
        // Still lying there long after it was first asked for: not
        // coming. It is stepped over and the dagger taken instead.
        let long_after = now + NOT_COMING + Duration::from_millis(1);
        let next = run.step(&at, long_after);
        assert_eq!(next.act, Some(Act::Take(2)), "{}", next.saying);
        assert_eq!(run.taken, 1, "the stuck thing was never taken");
        // With the dagger out, the corpse is set aside for the stuck
        // thing, not written off.
        let at = body(vec![thing(
            1,
            "Stuck Thing",
            Verdict::Take(LootAction::Keep),
        )]);
        let next = run.step(&at, long_after);
        assert_eq!(next.act, Some(Act::Close), "{}", next.saying);
        assert!(matches!(next.did, Did::Blocked(_)), "{:?}", next.did);
    }

    #[test]
    fn a_take_the_server_turns_down_is_stepped_over_and_the_rest_taken() {
        // "You are too encumbered to carry that!", a drop that can only be
        // had so often, a unique already carried: the server names the
        // item and it stays put. Asked for again every four hundred
        // milliseconds, it held up everything listed after it until the
        // corpse was given up on with all of it still there.
        let now = Instant::now();
        let mut run = Run::new();
        let mut at = body(vec![
            thing(1, "Quest Gem", Verdict::Take(LootAction::Keep)),
            thing(2, "Pyreal", Verdict::Take(LootAction::Keep)),
        ]);
        assert_eq!(run.step(&at, now).act, Some(Act::Take(1)));
        at.refused = vec![1];
        // Turned down: the coin goes out at once, and the gem is not
        // asked for again however long the rules are consulted.
        let next = run.step(&at, now);
        assert_eq!(next.act, Some(Act::Take(2)), "{}", next.saying);
        let at = Open {
            items: vec![thing(1, "Quest Gem", Verdict::Take(LootAction::Keep))],
            ..at
        };
        for ms in [0, 500, 1_000, 5_000] {
            let next = run.step(&at, now + Duration::from_millis(ms));
            assert_ne!(next.act, Some(Act::Take(1)), "asked for again at {ms} ms");
            assert_eq!(next.act, Some(Act::Close), "{}", next.saying);
            // Set aside, not emptied: a quest's wait lifts.
            assert!(matches!(next.did, Did::Blocked(_)), "{:?}", next.did);
        }
        assert_eq!(run.taken, 1, "only the coin was taken");
    }

    #[test]
    fn a_full_pack_closes_the_corpse_rather_than_standing_over_it() {
        // This one was live: the give-up path dropped the corpse
        // without telling the server, so it stayed open and the next
        // could not be opened.
        let mut run = Run::new();
        let mut at = body(vec![thing(1, "Dagger", Verdict::Take(LootAction::Keep))]);
        at.slots_free = 0;
        let next = run.step(&at, Instant::now());
        assert_eq!(next.act, Some(Act::Close), "{}", next.saying);
        // Set aside, not emptied. Shut as done with, every body reached
        // with a low pack was written off with its coin still on it, and
        // never gone back to once the pack had been sold down.
        assert!(
            matches!(next.did, Did::Blocked(_)),
            "written off for good: {:?}",
            next.did
        );
    }

    #[test]
    fn the_slots_kept_for_the_money_are_not_looted_into() {
        // Filling the last slots is how a run arrived at the counter with
        // nowhere for the coin to go: every sale was turned away.
        let mut at = body(vec![thing(1, "Dagger", Verdict::Take(LootAction::Keep))]);
        at.keep_free = 3;
        at.slots_free = 3;
        assert_eq!(Run::new().step(&at, Instant::now()).act, Some(Act::Close));
        at.slots_free = 4;
        assert_ne!(Run::new().step(&at, Instant::now()).act, Some(Act::Close));
    }

    #[test]
    fn coin_that_pours_onto_the_pile_carried_is_taken_off_a_body_the_pack_has_no_slot_for() {
        // The pack down to the slots kept for the money, and a body
        // with pyreals on it: the coin joins the pile already carried
        // and spends no slot, so it is taken; the dagger beside it has
        // nowhere to go, and the body is set aside for room after.
        let now = Instant::now();
        let mut run = Run::new();
        let mut coin = thing(1, "Pyreal", Verdict::Take(LootAction::Keep));
        coin.needs_no_slot = true;
        let dagger = thing(2, "Dagger", Verdict::Take(LootAction::Sell));
        let mut at = body(vec![dagger, coin]);
        at.keep_free = 3;
        at.slots_free = 3;
        let next = run.step(&at, now);
        assert_eq!(next.act, Some(Act::Take(1)), "{}", next.saying);
        // The coin is gone; the dagger is still there with no slot.
        at.items.retain(|i| i.guid != 1);
        let next = run.step(&at, now + Duration::from_millis(500));
        assert_eq!(next.act, Some(Act::Close), "{}", next.saying);
        assert!(
            matches!(&next.did, Did::Blocked(b) if b.what == "the pack is full"),
            "set aside for room: {:?}",
            next.did
        );
        assert_eq!(run.taken, 1);
    }

    #[test]
    fn a_pack_the_server_called_full_over_the_body_sets_it_aside_for_room_not_for_good() {
        // The pack was believed to have room when the body was opened
        // and the server said otherwise on the first take: the count
        // is corrected (`slots_free` falls to nothing) and the body is
        // set aside for room, the same as one reached with a full pack,
        // rather than opened again to be refused the same take.
        let now = Instant::now();
        let mut run = Run::new();
        let mut at = body(vec![
            thing(1, "Pyreal", Verdict::Take(LootAction::Keep)),
            thing(2, "Dagger", Verdict::Take(LootAction::Sell)),
        ]);
        assert_eq!(run.step(&at, now).act, Some(Act::Take(1)));
        at.refused.push(1);
        at.slots_free = 0;
        let next = run.step(&at, now + Duration::from_millis(500));
        assert_eq!(next.act, Some(Act::Close), "{}", next.saying);
        assert!(
            matches!(&next.did, Did::Blocked(b) if b.what == "the pack is full"),
            "{:?}",
            next.did
        );
    }

    #[test]
    fn a_character_carrying_all_it_means_to_still_takes_the_coin() {
        // Blargerton, laden: every corpse was shut before anything on it
        // was looked at, so even pyreals, which weigh nothing, stayed on
        // the body.
        let mut run = Run::new();
        let mut coin = thing(1, "Pyreal", Verdict::Take(LootAction::Keep));
        coin.burden = 0;
        let mut at = body(vec![coin]);
        at.carry_room = 0;
        let next = run.step(&at, Instant::now());
        assert_eq!(next.act, Some(Act::Take(1)), "{}", next.saying);
    }

    #[test]
    fn a_character_too_laden_for_what_is_left_sets_the_corpse_aside() {
        // Nothing on it fits, so it is shut -- but not written off: sold
        // down, the character has room for it, and the body keeps.
        let mut run = Run::new();
        let mut mace = thing(1, "Mace", Verdict::Take(LootAction::Sell));
        mace.burden = 50;
        let mut shield = thing(2, "Tower Shield", Verdict::MustAsk);
        shield.burden = 300;
        let mut at = body(vec![shield, mace]);
        at.carry_room = 40;
        let next = run.step(&at, Instant::now());
        assert_eq!(next.act, Some(Act::Close), "{}", next.saying);
        assert!(
            matches!(next.did, Did::Blocked(_)),
            "written off as emptied: {:?}",
            next.did
        );
        // And it says what the lightest of it weighs. Forty short of the
        // mace, the character went back to the body every half minute to
        // take nothing, and never counted itself laden.
        assert_eq!(next.left_for_weight, Some(50));
        // A body shut for any other reason is waiting on no weight.
        let mut run = Run::new();
        at.carry_room = 10_000;
        at.slots_free = 0;
        assert_eq!(run.step(&at, Instant::now()).left_for_weight, None);
    }

    #[test]
    fn something_heavier_than_it_can_carry_is_left_and_the_rest_taken() {
        let now = Instant::now();
        let mut run = Run::new();
        let mut heavy = thing(1, "Anvil", Verdict::Take(LootAction::Keep));
        heavy.burden = 9_000;
        let mut at = body(vec![
            heavy.clone(),
            thing(2, "Dagger", Verdict::Take(LootAction::Keep)),
        ]);
        at.carry_room = 100;
        // The anvil is stepped over, and the dagger goes in the pack.
        assert_eq!(run.step(&at, now).act, Some(Act::Take(2)));
        // With the dagger out, the anvil is all there is: the corpse is
        // set aside for it rather than written off.
        at.items = vec![heavy];
        let next = run.step(&at, now);
        assert_eq!(next.act, Some(Act::Close));
        assert!(matches!(next.did, Did::Blocked(_)), "{:?}", next.did);
    }

    #[test]
    fn what_could_not_be_carried_whatever_it_is_is_not_asked_about() {
        // An identify is a round trip. Spending one on a thing that will
        // be left for its weight whatever the answer is waste.
        let mut run = Run::new();
        let mut heavy = thing(1, "Odd Statue", Verdict::MustAsk);
        heavy.burden = 5_000;
        let mut at = body(vec![heavy, thing(2, "Odd Ring", Verdict::MustAsk)]);
        at.carry_room = 100;
        match run.step(&at, Instant::now()).act {
            Some(Act::Ask(ids)) => assert_eq!(ids, vec![2]),
            other => panic!("expected an ask for the ring alone, got {other:?}"),
        }
    }

    #[test]
    fn an_emptied_corpse_is_shut_behind_it() {
        let mut run = Run::new();
        let at = body(vec![thing(1, "Rock", Verdict::Leave)]);
        let next = run.step(&at, Instant::now());
        assert_eq!(next.act, Some(Act::Close));
        assert_eq!(next.did, Did::Done);
    }

    #[test]
    fn a_corpse_with_nothing_worth_taking_says_so_rather_than_emptied() {
        // Blargerton's log said "emptied" for every body he shut, and so
        // could not tell a profile that turned everything down from a
        // character that never looted at all.
        let mut run = Run::new();
        let at = body(vec![
            thing(1, "Rock", Verdict::Leave),
            thing(2, "Old Bone", Verdict::Leave),
        ]);
        let next = run.step(&at, Instant::now());
        assert_eq!(next.act, Some(Act::Close));
        assert_eq!(
            next.did,
            Did::Done,
            "still done with: nothing on it is wanted"
        );
        assert_eq!(
            next.saying,
            "nothing worth taking on Corpse of a Drudge (2 item(s))"
        );
        // A body with nothing on it at all says that instead.
        let next = Run::new().step(&body(Vec::new()), Instant::now());
        assert_eq!(next.did, Did::Done);
        assert_eq!(next.saying, "nothing on Corpse of a Drudge");
    }

    #[test]
    fn a_corpse_something_was_taken_from_is_emptied() {
        let now = Instant::now();
        let mut run = Run::new();
        let at = body(vec![
            thing(1, "Dagger", Verdict::Take(LootAction::Keep)),
            thing(2, "Rock", Verdict::Leave),
        ]);
        assert_eq!(run.step(&at, now).act, Some(Act::Take(1)));
        // The dagger came out; the rock is all that is left.
        let at = body(vec![thing(2, "Rock", Verdict::Leave)]);
        let next = run.step(&at, now);
        assert_eq!(next.did, Did::Done);
        assert_eq!(next.saying, "emptied Corpse of a Drudge");
    }

    #[test]
    fn a_corpse_that_will_not_empty_is_set_aside_not_written_off() {
        // A body belongs to whoever killed it for a while, so one that
        // will not give up its contents now may well later. Blocked,
        // never Refused.
        let now = Instant::now();
        let mut run = Run::new();
        // Waiting on an appraisal that never lands.
        let mut at = body(vec![thing(1, "Odd Ring", Verdict::MustAsk)]);
        at.asking = vec![1];
        for s in 0..=KEEP_AT_IT.as_secs() {
            let next = run.step(&at, now + Duration::from_secs(s));
            assert_eq!(next.act, None, "gave up at {s} s: {}", next.saying);
        }
        let next = run.step(&at, now + KEEP_AT_IT + Duration::from_secs(1));
        assert_eq!(next.act, Some(Act::Close));
        assert!(
            matches!(next.did, Did::Blocked(_)),
            "written off for good: {:?}",
            next.did
        );
    }

    #[test]
    fn a_fight_while_a_corpse_is_open_does_not_count_against_it() {
        // Blargerton opens a body and a pack of Drudges arrives. The fight
        // has him for a minute, the corpse stays open on the server, and
        // once the pack was dead the minute counted: the body was given up
        // on at once with its loot still on it.
        let now = Instant::now();
        let s = Duration::from_secs;
        let mut run = Run::new();
        let mut at = body(vec![thing(1, "Odd Ring", Verdict::MustAsk)]);
        at.asking = vec![1];
        // Ten seconds at it.
        for t in 0..=10 {
            assert_eq!(run.step(&at, now + s(t)).act, None);
        }
        // A minute fighting, then back at it: still well inside its time.
        let back = now + s(70);
        for t in 0..30 {
            let next = run.step(&at, back + s(t));
            assert_eq!(next.act, None, "gave up {t} s after the fight");
        }
        // The time spent at it does run out in the end.
        let mut closed = None;
        for t in 30..40 {
            let next = run.step(&at, back + s(t));
            if next.act.is_some() {
                closed = Some((t, next));
                break;
            }
        }
        let (t, next) = closed.expect("never gave up");
        assert!(t >= 35, "gave up after only {} s at it", 10 + t);
        assert!(matches!(next.did, Did::Blocked(_)), "{:?}", next.did);
    }

    #[test]
    fn a_corpse_is_not_judged_until_everything_on_it_has_arrived() {
        // ACE sends a corpse's list and then, on a later pass, the
        // things on it. A frame in between saw an empty body: shut,
        // written off as looted, and the loot arrived a frame too late.
        let now = Instant::now();
        let mut run = Run::new();
        let mut at = body(Vec::new());
        at.arriving = vec![1, 2];
        let next = run.step(&at, now);
        assert_eq!(next.act, None, "{}", next.saying);
        assert!(matches!(next.did, Did::Waiting(_)), "{:?}", next.did);
        // One has come and the other has not: still nothing is judged.
        at.items = vec![thing(1, "Pyreal", Verdict::Take(LootAction::Keep))];
        at.arriving = vec![2];
        assert_eq!(run.step(&at, now).act, None);
        // All here: emptied as usual.
        at.items
            .push(thing(2, "Dagger", Verdict::Take(LootAction::Keep)));
        at.arriving.clear();
        assert_eq!(run.step(&at, now).act, Some(Act::Take(1)));
        // A body whose things never come is set aside in the end, not
        // written off.
        let mut run = Run::new();
        let mut stuck = body(Vec::new());
        stuck.arriving = vec![3];
        for s in 0..=KEEP_AT_IT.as_secs() {
            assert_eq!(run.step(&stuck, now + Duration::from_secs(s)).act, None);
        }
        let next = run.step(&stuck, now + KEEP_AT_IT + Duration::from_secs(1));
        assert_eq!(next.act, Some(Act::Close));
        assert!(matches!(next.did, Did::Blocked(_)), "{:?}", next.did);
    }

    #[test]
    fn the_next_corpse_gets_its_own_forty_five_seconds() {
        // The clock was kept from the first body stood over. A corpse let
        // go without being shut here -- given up on, or asked again when
        // it would not open -- left it running, and the next corpse
        // opened a minute later was shut on its first step and written
        // off with everything still on it.
        let now = Instant::now();
        let mut run = Run::new();
        let first = body(vec![thing(1, "Dagger", Verdict::Take(LootAction::Keep))]);
        assert_eq!(run.step(&first, now).act, Some(Act::Take(1)));
        let mut second = body(vec![thing(2, "Shield", Verdict::Take(LootAction::Keep))]);
        second.guid = 901;
        let later = now + KEEP_AT_IT + Duration::from_secs(1);
        let next = run.step(&second, later);
        assert_eq!(next.act, Some(Act::Take(2)), "{}", next.saying);
        assert_eq!(run.taken, 1, "the count is for this body alone");
        // Its own clock runs from there, and does give up in the end. The
        // shield came out; what is left waits on an appraisal that never
        // lands.
        let mut second = body(vec![thing(3, "Odd Ring", Verdict::MustAsk)]);
        second.guid = 901;
        second.asking = vec![3];
        for s in 0..=KEEP_AT_IT.as_secs() {
            let next = run.step(&second, later + Duration::from_secs(s));
            assert_eq!(next.act, None, "gave up at {s} s: {}", next.saying);
        }
        let next = run.step(&second, later + KEEP_AT_IT + Duration::from_secs(1));
        assert_eq!(next.act, Some(Act::Close));
        assert!(matches!(next.did, Did::Blocked(_)), "{:?}", next.did);
    }

    #[test]
    fn never_two_takes_in_the_air_at_once() {
        // The invariant with a name. The server moves one thing at a
        // time and answers a second with "Source item not found"; a
        // version that stepped over the item it was waiting on earned
        // thirty-two of those in one watched run.
        let now = Instant::now();
        let mut run = Run::new();
        let at = body(vec![
            thing(1, "First", Verdict::Take(LootAction::Keep)),
            thing(2, "Second", Verdict::Take(LootAction::Keep)),
            thing(3, "Third", Verdict::Take(LootAction::Keep)),
        ]);
        assert_eq!(run.step(&at, now).act, Some(Act::Take(1)));
        // Nothing has left the corpse, so nothing else is asked for,
        // however many times the rules are consulted.
        for _ in 0..10 {
            assert_eq!(run.step(&at, now).act, None, "asked for a second item");
        }
        // Only when the first is gone does the next go out.
        let at = body(vec![
            thing(2, "Second", Verdict::Take(LootAction::Keep)),
            thing(3, "Third", Verdict::Take(LootAction::Keep)),
        ]);
        assert_eq!(run.step(&at, now).act, Some(Act::Take(2)));
    }

    #[test]
    fn the_tally_counts_bodies_that_opened_and_things_taken_off_them() {
        // "emptied" in the log could not tell a character that took
        // nothing from one that never looted. The count can.
        let now = Instant::now();
        let mut tally = Tally::default();
        // Walked to and let go before it opened: no body counted.
        let mut run = Run::new();
        let mut far = body(vec![thing(1, "Dagger", Verdict::Take(LootAction::Keep))]);
        far.away = 8.0;
        run.step(&far, now);
        tally.count(&run);
        assert_eq!(tally, Tally::default());
        // A dagger taken off one.
        let mut run = Run::new();
        let dagger = body(vec![thing(1, "Dagger", Verdict::Take(LootAction::Keep))]);
        assert_eq!(run.step(&dagger, now).act, Some(Act::Take(1)));
        run.step(&body(Vec::new()), now);
        tally.count(&run);
        // Nothing worth taking on another.
        let mut run = Run::new();
        run.step(&body(vec![thing(2, "Rock", Verdict::Leave)]), now);
        tally.count(&run);
        assert_eq!(
            tally,
            Tally {
                opened: 2,
                taken: 1
            }
        );
        assert_eq!(
            tally.line(),
            "this session: 2 corpse(s) opened, 1 thing(s) taken"
        );
    }
}
