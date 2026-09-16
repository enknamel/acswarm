use super::*;
use crate::autoplay::{Room, ShutFor};
use crate::refusals::{OPEN_IN_USE_AGAIN, OPEN_NOT_OURS_AGAIN};
use crate::testkit::{corpse_at_hand, corpses_seen};

#[test]
fn a_corpse_the_rules_set_aside_is_shelved_and_not_written_off() {
    // The rules shut a corpse that will not give up its contents as
    // Blocked -- "later" -- and the client marked every shut corpse
    // looted, which is "never".
    use crate::did::Did;
    let t0 = Instant::now();
    let mut ap = Autoplay::default();
    let stubborn = 0x8000_2001;
    ap.take_up_corpse(stubborn, t0, LOOT_TIMEOUT);
    ap.corpse_shut(
        stubborn,
        &Did::blocked("it will not give up its contents"),
        None,
        ShutFor::default(),
        t0,
    );
    assert_eq!(ap.corpse, None, "still in hand");
    assert!(!ap.looted.contains(&stubborn), "written off for good");
    assert!(ap.shelved.held(&stubborn, t0), "not left alone for now");
    assert!(
        !ap.shelved.held(&stubborn, t0 + Duration::from_secs(60)),
        "never tried again"
    );
    // One the rules emptied is done with.
    let emptied = 0x8000_2002;
    ap.take_up_corpse(emptied, t0, LOOT_TIMEOUT);
    ap.corpse_shut(emptied, &Did::Done, None, ShutFor::default(), t0);
    assert!(ap.looted.contains(&emptied));
    assert!(!ap.shelved.held(&emptied, t0));
}

#[test]
fn a_refusal_about_another_body_leaves_the_one_in_hand_alone() {
    let t0 = Instant::now();
    let body = 0x8000_1221;
    let asked = t0 + Duration::from_secs(1);
    let answered = asked + Duration::from_millis(360);
    let mut ap = Autoplay {
        corpse_seen: corpses_seen([(body, t0)]),
        ..Default::default()
    };
    ap.take_up_corpse(body, asked, LOOT_TIMEOUT);

    // Nine characters stand in one huddle and the server answers all
    // of them: words about a body this character is not working
    // change nothing.
    assert!(!ap.corpse_refused_in_words(
        "The Corpse of Drudge Slave is already in use by someone else!",
        "Corpse of Hellion",
        Some(answered),
        answered,
    ));
    // Nor do words about something that is not a refusal.
    assert!(!ap.corpse_refused_in_words(
        "You're too busy",
        "Corpse of Hellion",
        Some(answered),
        answered
    ));
    // Nor an answer that came in before this ask went out: it was
    // the ask before it that was refused.
    let words = "You do not yet have the right to loot the Corpse of Hellion.";
    assert!(!ap.corpse_refused_in_words(words, "Corpse of Hellion", Some(asked), answered));
    assert!(!ap.corpse_refused_in_words(words, "Corpse of Hellion", None, answered));
    assert_eq!(
        ap.corpse.map(|c| c.0),
        Some(body),
        "let a body go for nothing"
    );
    assert!(ap.shelved.is_empty(), "set a body aside for nothing");

    // The same words, stamped after the ask, are this body's.
    assert!(ap.corpse_refused_in_words(words, "Corpse of Hellion", Some(answered), answered));
    assert_eq!(ap.corpse, None);
}

#[test]
fn a_corpse_that_says_no_again_waits_twice_as_long() {
    // The looting tidied away each lapsed wait before choosing a
    // corpse, and a corpse is chosen again exactly when its wait is
    // up. So every refusal was the first: thirty seconds, never more,
    // and the character went back every half minute until it rotted.
    use crate::did::Did;
    let t0 = Instant::now();
    let s = Duration::from_secs;
    let mut ap = Autoplay::default();
    let (locked, rotted) = (0x8000_2101, 0x8000_2102);
    let no = Did::blocked("it will not open yet");
    ap.shelved.note(locked, &no, t0);
    ap.shelved.note(rotted, &no, t0);
    // Half a minute on, what `autoplay_loot` does before it chooses:
    // one body still lies there and the other has gone.
    let again = t0 + s(31);
    ap.forget_corpses_gone(|g| g == locked);
    assert_eq!(ap.shelved.len(), 1, "the rotted body is still remembered");
    assert!(
        ap.corpse_waiting(locked, again, Room::PLENTY),
        "never tried again"
    );
    // Chosen, and it says no again: a minute this time.
    ap.shelved.note(locked, &no, again);
    assert!(
        ap.shelved.held(&locked, again + s(59)),
        "back after thirty seconds again"
    );
    assert!(!ap.shelved.held(&locked, again + s(60)));
}

#[test]
fn an_ask_at_a_corpse_the_server_let_go_comes_back_with_nothing() {
    // Blargerton walked back to bodies ACE had let go while he was out
    // of sight of them, and asked at them for the rest of the session.
    let asked = Instant::now();
    let ms = Duration::from_millis;
    let (underfoot, done) = (CORPSE_REACH / 2.0, Some((0, asked + ms(80))));
    // Not there: done, no error, and not a word.
    assert!(answered_with_nothing(underfoot, asked, done, None));
    // A word from before the ask was about something else.
    assert!(answered_with_nothing(underfoot, asked, done, Some(asked)));
    // Locked to whoever killed it, or open to someone else: the server
    // says so before it is done.
    assert!(!answered_with_nothing(
        underfoot,
        asked,
        done,
        Some(asked + ms(40))
    ));
    // Too busy is a refusal, not an absence.
    assert!(!answered_with_nothing(
        underfoot,
        asked,
        Some((0x1D, asked + ms(80))),
        None
    ));
    // No answer yet, or only one that came in on the tick the ask went
    // out, which was to an earlier ask.
    assert!(!answered_with_nothing(underfoot, asked, None, None));
    assert!(!answered_with_nothing(
        underfoot,
        asked,
        Some((0, asked)),
        None
    ));
    // From across the room the server walks the character over first,
    // and a walk it cannot finish ends just as quietly.
    assert!(!answered_with_nothing(
        CORPSE_REACH * 4.0,
        asked,
        done,
        None
    ));
}

#[test]
fn a_corpse_is_taken_for_gone_only_when_every_ask_at_it_came_back_with_nothing() {
    let t0 = Instant::now();
    let mut ap = Autoplay::default();
    // What `autoplay_loot` does as each ask's wait runs out: note how
    // it came back, then ask again until the tries are used up. What
    // is said after the last ask decides what becomes of the body.
    let ask_until_given_up = |ap: &mut Autoplay, guid: u32, quiet: &[bool]| {
        ap.take_up_corpse(guid, t0, LOOT_TIMEOUT);
        let mut gone = false;
        for (tries, q) in (0..).zip(quiet) {
            gone = ap.nothing_came_back(*q);
            ap.let_go_of_corpse();
            if tries < LOOT_TRIES {
                ap.corpse = Some((guid, t0, LOOT_TIMEOUT, tries + 1));
            }
        }
        gone
    };
    let every = [true; LOOT_TRIES as usize + 1];
    assert!(
        ask_until_given_up(&mut ap, 0x8000_4001, &every),
        "a body that is not there is set aside to be asked at again"
    );
    // One ask that said something, or had no answer at all, and the
    // body is only set aside, as before. Nor does the last body's
    // count run on into this one.
    let mut once = every;
    once[0] = false;
    assert!(
        !ask_until_given_up(&mut ap, 0x8000_4002, &once),
        "a body that answered once is forgotten"
    );
    // Nothing in hand, nothing to say.
    assert!(!ap.nothing_came_back(true));
}

#[test]
fn a_corpse_lasts_five_minutes() {
    // ACE gives an unlooted monster corpse no timer until its first
    // heartbeat, when it takes the default of five minutes. That is
    // the whole window, so it is what the rules plan against.
    assert_eq!(CORPSE_LIFE, Duration::from_secs(300));
}

#[test]
fn breaking_off_a_fight_is_reserved_for_a_corpse_about_to_go() {
    // Loot keeps for minutes; the thing hitting you does not. The
    // urgency window has to be small enough that a fight is not
    // interrupted for a corpse with plenty of time left, and big
    // enough to actually reach one.
    assert!(CORPSE_URGENT < CORPSE_LIFE / 3, "too eager to break off");
    assert!(
        CORPSE_URGENT >= Duration::from_secs(30),
        "no time to get there"
    );
}

#[test]
fn the_corpse_opened_after_one_was_given_up_on_is_not_written_off_on_its_first_step() {
    // Blargerton: a corpse that would not empty was given up on after
    // forty-five seconds, and the loot rules' clock went on running
    // from it. The next body he opened was shut on its first step
    // for having taken too long, and marked looted with everything
    // still on it.
    let t0 = Instant::now();
    let mut ap = Autoplay::default();
    let (first, second) = (0x8000_1001, 0x8000_1002);
    ap.take_up_corpse(first, t0, LOOT_TIMEOUT);
    let next = ap.loot_run.step(&corpse_at_hand(first, 1), t0);
    assert_eq!(next.act, Some(ac_loot::Act::Take(1)), "{}", next.saying);
    // Let go of some way other than a shut by the rules: given up on
    // as it once was, or asked again when it would not open.
    let gave_up = t0 + ac_loot::run::KEEP_AT_IT + Duration::from_secs(1);
    ap.let_go_of_corpse();
    assert_eq!(ap.corpse, None);
    // The next corpse is chosen, and opens a moment later.
    ap.take_up_corpse(second, gave_up, LOOT_TIMEOUT);
    let opened = gave_up + Duration::from_secs(1);
    let next = ap.loot_run.step(&corpse_at_hand(second, 2), opened);
    assert_eq!(next.act, Some(ac_loot::Act::Take(2)), "{}", next.saying);
    assert!(!ap.looted.contains(&second), "written off unlooted");
}

#[test]
fn the_words_a_body_is_refused_in_say_how_long_to_leave_it() {
    // 1,044 refusals arrived in that run, a third of a second after
    // the ask, and every one of them was thrown away: the take-up
    // ended on a clock instead, and asked again three times more.
    let t0 = Instant::now();
    let name = "Corpse of Hellion";
    let answered = t0 + Duration::from_millis(360);
    let in_use = format!("The {name} is already in use by someone else!");
    let not_ours = format!("You do not yet have the right to loot the {name}.");
    let (held, locked, rare) = (0x8000_1221, 0x8000_1222, 0x8000_1223);
    let mut ap = Autoplay {
        corpse_seen: corpses_seen([(held, t0), (locked, t0), (rare, t0)]),
        ..Default::default()
    };

    // Someone is inside it: let it go and have it the moment they
    // are done, not in the half minute anything blocked waits.
    ap.take_up_corpse(held, t0, LOOT_TIMEOUT);
    assert!(ap.corpse_refused_in_words(&in_use, name, Some(answered), answered));
    assert_eq!(ap.corpse, None, "still holding a body it cannot open");
    assert_eq!(ap.shelved.waited(&held), Some(OPEN_IN_USE_AGAIN));
    assert!(ap.shelved.held(
        &held,
        answered + OPEN_IN_USE_AGAIN - Duration::from_millis(1)
    ));
    assert!(!ap.shelved.held(&held, answered + OPEN_IN_USE_AGAIN));

    // The killer's for now: a short wait, because a corpse becomes
    // everyone's the moment whoever has it closes it and not only
    // when it half rots.
    ap.take_up_corpse(locked, t0, LOOT_TIMEOUT);
    assert!(ap.corpse_refused_in_words(&not_ours, name, Some(answered), answered));
    assert_eq!(ap.corpse, None);
    assert_eq!(ap.shelved.waited(&locked), Some(OPEN_NOT_OURS_AGAIN));
    assert!(ap.shelved.held(
        &locked,
        answered + OPEN_NOT_OURS_AGAIN - Duration::from_millis(1)
    ));
    assert!(!ap.shelved.held(&locked, answered + OPEN_NOT_OURS_AGAIN));

    // A body refused twice, in two different sets of words, waits
    // longer the second time -- and the wait it is given is a wait
    // and not a deadline. Handing the shelf an exact "come back in
    // 117 seconds" doubled the three seconds already on the body
    // instead: six, then twelve, then twenty-four, five more asks
    // and five more refusals before it reached the wait that was
    // meant the first time.
    let both = 0x8000_1224;
    ap.corpse_seen.mark(both, t0);
    ap.take_up_corpse(both, t0, LOOT_TIMEOUT);
    assert!(ap.corpse_refused_in_words(&in_use, name, Some(answered), answered));
    assert_eq!(ap.shelved.waited(&both), Some(OPEN_IN_USE_AGAIN));
    let again = answered + OPEN_IN_USE_AGAIN;
    ap.take_up_corpse(both, again, LOOT_TIMEOUT);
    let told = again + Duration::from_millis(360);
    assert!(ap.corpse_refused_in_words(&not_ours, name, Some(told), told));
    assert_eq!(
        ap.shelved.waited(&both),
        Some(OPEN_IN_USE_AGAIN * 2),
        "a doubling wait, not a deadline the shelf then doubled"
    );

    // The killer's for good: not waited on at all.
    ap.take_up_corpse(rare, t0, LOOT_TIMEOUT);
    let words =
        format!("You may not loot the {name} because the {name} has generated a rare item.");
    assert!(ap.corpse_refused_in_words(&words, name, Some(answered), answered));
    assert!(ap.shelved.held(&rare, t0 + CORPSE_LIFE));
}

#[test]
fn every_body_opened_is_counted_for_the_panel_however_it_was_let_go() {
    // Blargerton's log said "emptied" whether he took something or
    // nothing. The panel's count tells the two apart, and a body given
    // up on still counts for what came off it.
    use crate::did::Did;
    let t0 = Instant::now();
    let mut ap = Autoplay::default();
    // Opened, the dagger asked for, then given up on.
    let first = 0x8000_3001;
    ap.take_up_corpse(first, t0, LOOT_TIMEOUT);
    let next = ap.loot_run.step(&corpse_at_hand(first, 1), t0);
    assert_eq!(next.act, Some(ac_loot::Act::Take(1)), "{}", next.saying);
    ap.let_go_of_corpse();
    // Shut by the rules with nothing worth taking on it.
    let second = 0x8000_3002;
    ap.take_up_corpse(second, t0, LOOT_TIMEOUT);
    let mut bare = corpse_at_hand(second, 2);
    bare.items[0].verdict = ac_loot::Verdict::Leave;
    let next = ap.loot_run.step(&bare, t0);
    assert_eq!(next.did, Did::Done, "{}", next.saying);
    ap.corpse_shut(
        second,
        &next.did,
        next.left_for_weight,
        ShutFor::default(),
        t0,
    );
    // Walked to and never opened, then the next body taken up.
    let third = 0x8000_3003;
    ap.take_up_corpse(third, t0, LOOT_TIMEOUT);
    let mut unopened = corpse_at_hand(third, 3);
    unopened.open = false;
    ap.loot_run.step(&unopened, t0);
    ap.take_up_corpse(0x8000_3004, t0, LOOT_TIMEOUT);
    assert_eq!(
        ap.loot_tally,
        ac_loot::Tally {
            opened: 2,
            taken: 1
        }
    );
}
