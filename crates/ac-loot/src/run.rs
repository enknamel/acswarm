//! Emptying one corpse, one decision at a time: [`Run::step`] answers the next [`Act`].
//!
//! Paced by the server: a take is answered by the item leaving, so the next goes out then.
//! Only what cannot be judged is asked about, all in one go; however a run ends, the corpse is shut.

use std::time::{Duration, Instant};

use ac_agent::did::{Because, Did};

use crate::corpse::{Lying, Open, Verdict, REACH};

/// How long a take not yet moved waits before it is asked again (guess).
/// Not the pace of looting, which is the corpse answering; only how long a lost request is left.
pub const TAKE_AGAIN: Duration = Duration::from_millis(400);
/// Time spent at one corpse before it is set aside (guess).
/// A corpse belongs to its killer for a while, so one that will not give up its contents may later.
pub const KEEP_AT_IT: Duration = Duration::from_secs(45);
/// How long a take may lie unmoved before it is stepped over.
/// Two of the client's 4 s resends (`TAKE_LOST`, ac-client) and a little over: none is still in air.
pub const NOT_COMING: Duration = Duration::from_secs(10);
/// A gap this long between turns is time something else (a fight) had, not counted (guess).
/// Safe because the rules are consulted many times a second while they have the character.
pub const BROKEN_OFF: Duration = Duration::from_secs(3);

/// The one thing the rules want done next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    /// Walk to it: a corpse will not open across a room.
    Approach,
    /// Ask it to open.
    Open,
    /// Identify these, all at once: the server answers each on its own.
    Ask(Vec<u32>),
    /// Take this one.
    Take(u32),
    /// Shut it, however the run ended: the server keeps the character standing over an open
    /// corpse, and the next cannot open until this one is let go.
    Close,
}

/// What came of a turn.
#[derive(Clone, Debug, PartialEq)]
pub struct Next {
    pub act: Option<Act>,
    pub did: Did,
    pub saying: String,
    /// Burden of the lightest thing left, when shut for want of carrying room.
    /// Going back to the body with less room than this takes nothing.
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
    /// The item last asked for: `(guid, first asked, last asked)`.
    asked: Option<(u32, Instant, Instant)>,
    /// Stepped over on this body: turned down, or unmoved after [`NOT_COMING`].
    passed: Vec<u32>,
    /// When this corpse was first stood over, pushed later by time spent elsewhere ([`BROKEN_OFF`]).
    began: Option<Instant>,
    /// When the rules were last consulted about it.
    last: Option<Instant>,
    /// How many things have been taken.
    pub taken: u32,
    /// Whether the corpse opened; one walked to and let go before that did not.
    pub opened: bool,
}

impl Run {
    pub fn new() -> Run {
        Run::default()
    }

    /// The next thing to do with this corpse, or why there is nothing.
    pub fn step(&mut self, at: &Open, now: Instant) -> Next {
        // A different body starts afresh, however the last was let go
        // (`the_next_corpse_gets_its_own_forty_five_seconds`).
        if self.corpse != Some(at.guid) {
            *self = Run {
                corpse: Some(at.guid),
                ..Run::default()
            };
        }
        // Count only time at this corpse: a fight mid-corpse has the character while the corpse
        // stays open on the server (`a_fight_while_a_corpse_is_open_does_not_count_against_it`).
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

        // No slot to spare, short of those kept for the money: set aside, not written off, as the
        // body keeps until the pack is sold down. Coin that pours onto a carried pile needs no slot.
        // `keep_free` is over every pack: the server puts the coin wherever there is room.
        let has_a_slot = at.slots_free > 0 && at.room_anywhere > at.keep_free;
        if !has_a_slot && !at.wanted().any(|i| i.needs_no_slot) {
            return Next::set_aside(
                "the pack is full",
                format!("pack full, leaving {} for now", at.name),
            );
        }
        // No `carry_room` check here on purpose: only what will not fit stays
        // (`a_character_carrying_all_it_means_to_still_takes_the_coin`).

        // Too long at it: set aside, not written off.
        if now.duration_since(began) > KEEP_AT_IT {
            return Next::set_aside(
                "it will not give up its contents",
                format!("leaving {} for now", at.name),
            );
        }

        // Not all described yet: judged now, a body with loot on it reads as empty
        // (`a_corpse_is_not_judged_until_everything_on_it_has_arrived`).
        if !at.arriving.is_empty() {
            return Next::wait(&format!("opening {}", at.name));
        }

        // Ask in one go, but not about what is too heavy to carry whatever it turns out to be.
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

        // Take in the corpse's order, paced by each item leaving the list, not by a clock.
        // Never two takes in the air: the server moves one thing at a time and answers a second with
        // "Source item not found" (`never_two_takes_in_the_air_at_once`). An unmoved take is waited
        // on; only one turned down, too heavy, or unmoved after NOT_COMING is stepped over.
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

        // Something wanted has no slot (the pack filled, or the server called it full): set aside
        // for room, not reopened until there is some, or it is refused the same take again.
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

        // Something wanted would not come: set aside, since a quest's wait lifts and a unique can be sold.
        if let Some(stuck) = at.wanted().find(|i| given_up(i.guid)) {
            return Next::set_aside(
                "it would not give something up",
                format!("{} would not come off {}", stuck.name, at.name),
            );
        }

        // Left for its weight (a MustAsk too heavy to ask about included): set aside until sold down.
        // The lightest burden goes with the close: with less room than that, go and sell.
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
        // Done either way, but "took nothing" is said apart from "emptied" for whoever is watching.
        if self.taken == 0 {
            return Next::done(match at.items.len() {
                0 => format!("nothing on {}", at.name),
                n => format!("nothing worth taking on {} ({n} item(s))", at.name),
            });
        }
        Next::done(format!("emptied {}", at.name))
    }
}

/// Corpses opened and things taken over many runs, for the panel.
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
            room_anywhere: 20,
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
        // Each identify is a round trip: only what the profile cannot settle is asked, in one ask.
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
        // Nothing to do yet is said with no act and never as a close, which would mark the body
        // looted while its items are still off being appraised.
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
        // The corpse answers by the item leaving the list; no fixed wait between takes.
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
        // Still there after NOT_COMING: stepped over, and the dagger taken instead.
        let long_after = now + NOT_COMING + Duration::from_millis(1);
        let next = run.step(&at, long_after);
        assert_eq!(next.act, Some(Act::Take(2)), "{}", next.saying);
        assert_eq!(run.taken, 1, "the stuck thing was never taken");
        // With the dagger out, the corpse is set aside for the stuck thing, not written off.
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
        // "You are too encumbered to carry that!", a rate-limited drop, a unique already carried: the
        // server names the item and it stays put, and must not hold up what is listed after it.
        let now = Instant::now();
        let mut run = Run::new();
        let mut at = body(vec![
            thing(1, "Quest Gem", Verdict::Take(LootAction::Keep)),
            thing(2, "Pyreal", Verdict::Take(LootAction::Keep)),
        ]);
        assert_eq!(run.step(&at, now).act, Some(Act::Take(1)));
        at.refused = vec![1];
        // Turned down: the coin goes out at once, and the gem is never asked for again.
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
        // Giving up still closes: a corpse dropped without telling the server stays open, and the
        // next cannot be opened.
        let mut run = Run::new();
        let mut at = body(vec![thing(1, "Dagger", Verdict::Take(LootAction::Keep))]);
        at.slots_free = 0;
        at.room_anywhere = 0;
        let next = run.step(&at, Instant::now());
        assert_eq!(next.act, Some(Act::Close), "{}", next.saying);
        // Set aside, not emptied: the body is gone back to once the pack is sold down.
        assert!(
            matches!(next.did, Did::Blocked(_)),
            "written off for good: {:?}",
            next.did
        );
    }

    #[test]
    fn the_slots_kept_for_the_money_are_not_looted_into() {
        // With the last slots filled, a counter has nowhere for the coin and turns every sale away.
        let mut at = body(vec![thing(1, "Dagger", Verdict::Take(LootAction::Keep))]);
        at.keep_free = 3;
        at.slots_free = 3;
        at.room_anywhere = 3;
        assert_eq!(Run::new().step(&at, Instant::now()).act, Some(Act::Close));
        at.slots_free = 4;
        at.room_anywhere = 4;
        assert_ne!(Run::new().step(&at, Instant::now()).act, Some(Act::Close));
    }

    #[test]
    fn the_slots_kept_for_the_money_are_counted_over_every_pack() {
        // Two slots in the main pack, two in the sack, three kept: the coin goes wherever there is
        // room, so the take has a slot and the counter its three. With three in all, none is spare.
        let mut at = body(vec![thing(1, "Dagger", Verdict::Take(LootAction::Keep))]);
        at.keep_free = 3;
        at.slots_free = 2;
        at.room_anywhere = 4;
        let next = Run::new().step(&at, Instant::now());
        assert_eq!(next.act, Some(Act::Take(1)), "{}", next.saying);
        at.slots_free = 1;
        at.room_anywhere = 3;
        assert_eq!(Run::new().step(&at, Instant::now()).act, Some(Act::Close));
        // Room the server could spread coin over is not a slot a take can go into.
        at.slots_free = 0;
        at.room_anywhere = 5;
        assert_eq!(Run::new().step(&at, Instant::now()).act, Some(Act::Close));
    }

    #[test]
    fn coin_that_pours_onto_the_pile_carried_is_taken_off_a_body_the_pack_has_no_slot_for() {
        // Pack down to the kept slots: the coin joins the carried pile and spends no slot, so it is
        // taken; the dagger has nowhere to go, and the body is set aside for room.
        let now = Instant::now();
        let mut run = Run::new();
        let mut coin = thing(1, "Pyreal", Verdict::Take(LootAction::Keep));
        coin.needs_no_slot = true;
        let dagger = thing(2, "Dagger", Verdict::Take(LootAction::Sell));
        let mut at = body(vec![dagger, coin]);
        at.keep_free = 3;
        at.slots_free = 3;
        at.room_anywhere = 3;
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
        // The server called the pack full on the first take (`slots_free` falls to 0): set aside for
        // room, not for the refused coin, or it reopens when that wait lifts to be refused the dagger.
        let now = Instant::now();
        let mut run = Run::new();
        let mut coin = thing(1, "Pyreal", Verdict::Take(LootAction::Keep));
        coin.needs_no_slot = true;
        let mut at = body(vec![
            coin,
            thing(2, "Dagger", Verdict::Take(LootAction::Sell)),
        ]);
        assert_eq!(run.step(&at, now).act, Some(Act::Take(1)));
        at.refused.push(1);
        at.slots_free = 0;
        at.room_anywhere = 0;
        let next = run.step(&at, now + Duration::from_millis(500));
        assert_eq!(next.act, Some(Act::Close), "{}", next.saying);
        assert!(
            matches!(&next.did, Did::Blocked(b) if b.what == "the pack is full"),
            "{:?}",
            next.did
        );
        // With a slot for it, the dagger is taken and the coin's refusal is what the body is left for.
        let mut run = Run::new();
        at.slots_free = 5;
        at.room_anywhere = 5;
        assert_eq!(run.step(&at, now).act, Some(Act::Take(2)));
        at.items.retain(|i| i.guid != 2);
        let next = run.step(&at, now + Duration::from_millis(500));
        assert!(
            matches!(&next.did, Did::Blocked(b) if b.what == "it would not give something up"),
            "{:?}",
            next.did
        );
    }

    #[test]
    fn a_character_carrying_all_it_means_to_still_takes_the_coin() {
        // Laden is no reason to shut a corpse unopened: pyreals weigh nothing and are still taken.
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
        // Nothing fits: shut, not written off, since once sold down there is room and the body keeps.
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
        // It says what the lightest weighs: going back with less room takes nothing, and means laden.
        assert_eq!(next.left_for_weight, Some(50));
        // A body shut for any other reason is waiting on no weight.
        let mut run = Run::new();
        at.carry_room = 10_000;
        at.slots_free = 0;
        at.room_anywhere = 0;
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
        // With the dagger out, only the anvil is left: set aside for it rather than written off.
        at.items = vec![heavy];
        let next = run.step(&at, now);
        assert_eq!(next.act, Some(Act::Close));
        assert!(matches!(next.did, Did::Blocked(_)), "{:?}", next.did);
    }

    #[test]
    fn what_could_not_be_carried_whatever_it_is_is_not_asked_about() {
        // An identify is a round trip, wasted on what stays for its weight whatever the answer.
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
        // Said apart from "emptied", so a profile turning everything down shows as such.
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
        // A body is its killer's for a while and may give up its contents later: Blocked, never Refused.
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
        // A minute's fight mid-corpse, the corpse open on the server all the while, is not time
        // spent at the corpse.
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
        // ACE sends a corpse's list and then, on a later pass, the things on it; judged in between,
        // the body reads as empty.
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
        // A body whose things never come is set aside in the end, not written off.
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
        // A corpse let go without a close here (given up on, or asked again when it would not open)
        // must not leave its clock running for the next.
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
        // Its own clock runs from there and gives up in the end: the shield came out, and what is
        // left waits on an appraisal that never lands.
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
        // The server moves one thing at a time and answers a second take with "Source item not found".
        let now = Instant::now();
        let mut run = Run::new();
        let at = body(vec![
            thing(1, "First", Verdict::Take(LootAction::Keep)),
            thing(2, "Second", Verdict::Take(LootAction::Keep)),
            thing(3, "Third", Verdict::Take(LootAction::Keep)),
        ]);
        assert_eq!(run.step(&at, now).act, Some(Act::Take(1)));
        // Nothing has left the corpse, so nothing else is asked for, however often consulted.
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
        // The count tells a character that took nothing from one that never looted.
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
