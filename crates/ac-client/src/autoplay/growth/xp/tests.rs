use super::*;
use crate::autoplay::growth::raise::{choose_raise, Batch};
use crate::testkit::{standing_at, standing_by};

/// A level 10 character playing on its own where the Holtburg
/// Dungeon's portal drops it -- a dungeon, so exploring always has a
/// room to walk to -- with `xp` to spend and nothing spent yet.
fn with_experience_to_spend(xp: i64) -> Client {
    let mut c = standing_at(0x01F6_0289, glam::Vec3::new(96.7, -10.0, 0.0));
    c.world.player_guid = Some(0x5000_0001);
    c.world.stats.level = 10;
    c.world.stats.available_xp = xp;
    c.autoplay.config.enabled = true;
    c
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn experience_is_spent_while_exploring_claims_every_tick() {
    // On the local server a character granted a hundred billion
    // experience spent none of it in three minutes. Spending was part
    // of the last goal, and exploring claimed every tick before it.
    let mut c = with_experience_to_spend(1_000_000);
    let start = Instant::now();
    let mut raised: Vec<Instant> = Vec::new();
    let mut t = start;
    while t < start + RAISE_EVERY * 5 {
        c.tick_autoplay(t);
        assert_eq!(
            c.autoplay.step,
            Some("explore"),
            "exploring did not claim the tick at {:?}",
            t - start
        );
        if let Some(at) = c.autoplay.growth.last_raise {
            if !raised.contains(&at) {
                raised.push(at);
                // The server takes it.
                let (pick, batch) = sent(&c);
                server_takes(&mut c, pick, batch);
            }
        }
        t += Duration::from_millis(100);
    }
    assert!(
        raised.first().is_some_and(|at| *at - start <= RAISE_EVERY),
        "nothing was raised within {RAISE_EVERY:?}"
    );
    // And it goes on, a message at a time.
    assert!(raised.len() >= 4, "only {} ranks", raised.len());
    assert!(
        raised.windows(2).all(|w| w[1] - w[0] >= RAISE_EVERY),
        "ranks closer together than {RAISE_EVERY:?}"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn nothing_is_sent_with_auto_xp_off_or_nothing_to_spend() {
    let mut c = with_experience_to_spend(1_000_000);
    let start = Instant::now();
    let at = |s: u64| start + Duration::from_secs(s);

    c.autoplay.config.growth.auto_xp = false;
    for s in 0..30 {
        c.autoplay_spend_xp(at(s));
    }
    assert_eq!(c.session.actions_sent(), 0, "spent with auto_xp off");

    // On, with an empty pool, or one short of the cheapest rank.
    c.autoplay.config.growth.auto_xp = true;
    for (s, pool) in [0, -5, 1].into_iter().enumerate() {
        c.world.stats.available_xp = pool;
        c.autoplay_spend_xp(at(30 + s as u64));
    }
    assert_eq!(c.session.actions_sent(), 0, "spent with nothing to spend");

    // With something in the pool, a rank goes out.
    c.world.stats.available_xp = 1_000_000;
    c.autoplay_spend_xp(at(40));
    assert_eq!(c.session.actions_sent(), 1);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_rank_the_server_refuses_is_not_asked_for_again_while_it_sulks() {
    let mut c = with_experience_to_spend(1_000_000);
    let start = Instant::now();
    assert!(c.grow_spend_xp(start));
    let (refused, _) = sent(&c);

    // Nothing comes back. Nothing more is asked for until the server
    // has had time to answer, from the look after it went out.
    let out = start + Duration::from_millis(100);
    assert!(!c.grow_spend_xp(out));
    assert!(!c.grow_spend_xp(out + RAISE_SETTLE / 2));
    assert_eq!(c.session.actions_sent(), 1);

    // Then it is taken as refused, and the rest are bought instead for
    // as long as the refusal holds, the server taking each.
    let sulked = out + RAISE_SETTLE;
    let mut t = sulked;
    let mut bought = 0;
    while t < sulked + SULK_FOR {
        if c.grow_spend_xp(t) {
            let (pick, batch) = sent(&c);
            assert_ne!(
                pick,
                refused,
                "asked again {:?} after it was refused",
                t - start
            );
            server_takes(&mut c, pick, batch);
            bought += 1;
        }
        t += Duration::from_secs(5);
    }
    assert!(bought > 0, "nothing else was bought");

    // Once the sulk is over it is the best buy again, and what is left
    // of the pool was kept for it.
    assert!(c.grow_spend_xp(sulked + SULK_FOR + Duration::from_secs(5)));
    assert_eq!(sent(&c).0, refused);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_raise_is_not_given_up_on_before_it_has_gone_out() {
    // A round of nine headless characters took four and a half seconds
    // on the local server. A raise queued in it goes on the wire at the
    // top of the next tick, and was given up on in that same tick,
    // before any answer could come: nine of them, and all nine landed a
    // third of a second later.
    let mut c = with_experience_to_spend(10_000_000_000);
    let start = Instant::now();
    let round = Duration::from_millis(4_600);
    assert!(c.grow_spend_xp(start));
    let (pick, batch) = sent(&c);
    assert!(!c.grow_spend_xp(start + round));
    assert!(
        c.autoplay.growth.sulking.is_empty(),
        "{pick:?} was given up on as it went out"
    );
    // The answer comes, and the next raise goes out on the round after.
    server_takes(&mut c, pick, batch);
    assert!(c.grow_spend_xp(start + round * 2));
    assert!(c.autoplay.growth.sulking.is_empty());
    assert_eq!(c.session.actions_sent(), 2);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn an_answer_that_comes_late_ends_the_wait_and_finds_its_share_kept() {
    // When the local server stopped answering for forty seconds, nine
    // characters gave up on ninety raises, and nearly all of them
    // landed afterwards. A stat given up on must not lose its share of
    // the pool to the others meanwhile, nor stay shut out once its
    // answer is in.
    let pool = 10_000_000_000;
    let (mut slow, mut calm) = (
        with_experience_to_spend(pool),
        with_experience_to_spend(pool),
    );
    let step = Duration::from_millis(100);
    let start = Instant::now();
    let spend_until = |c: &mut Client, from: Instant, until: Instant| {
        let mut t = from;
        while t < until {
            if c.grow_spend_xp(t) {
                let (pick, batch) = sent(c);
                server_takes(c, pick, batch);
            }
            t += step;
        }
    };
    spend_until(&mut calm, start, start + Duration::from_secs(120));

    // The first raise goes unanswered, and is given up on.
    assert!(slow.grow_spend_xp(start));
    let (late, late_batch) = sent(&slow);
    let mut t = start + step;
    loop {
        assert!(t < start + Duration::from_secs(5), "never given up on");
        let went = slow.grow_spend_xp(t);
        t += step;
        if slow.autoplay.growth.sulking.is_empty() {
            assert!(!went, "another raise went out before it was given up on");
            continue;
        }
        // Given up on; the next raise goes out in the same look.
        if went {
            let (pick, batch) = sent(&slow);
            assert_ne!(pick, late, "sized again before its answer came");
            server_takes(&mut slow, pick, batch);
        }
        break;
    }
    // The rest go out and are answered at once, and none of them is
    // the late one sized again.
    let mut others = 0;
    while t < start + Duration::from_secs(20) {
        if slow.grow_spend_xp(t) {
            let (pick, batch) = sent(&slow);
            assert_ne!(pick, late, "sized again before its answer came");
            server_takes(&mut slow, pick, batch);
            others += 1;
        }
        t += step;
    }
    assert!(others > 0, "nothing else went out");
    // Twenty seconds on, the answer comes. The pool still holds it.
    server_takes(&mut slow, late, late_batch);
    spend_until(&mut slow, t, t + Duration::from_secs(100));
    assert!(
        slow.autoplay.growth.sulking.is_empty(),
        "still waiting on an answer that came"
    );
    // And it all comes to what a server that answers at once gives.
    let (s, c) = (&slow.world.stats, &calm.world.stats);
    assert_eq!(s.available_xp, c.available_xp);
    assert_eq!(s.attributes, c.attributes);
    assert_eq!(s.vitals, c.vitals);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_stat_is_not_sized_again_before_its_answer_comes() {
    // A kill's experience moves the pool too. Taken for the answer, it
    // let the stat be sized again from the record the answer had not
    // reached yet, and bought twice.
    let mut c = with_experience_to_spend(10_000_000_000);
    let start = Instant::now();
    assert!(c.grow_spend_xp(start));
    let (pick, batch) = sent(&c);
    c.world.stats.available_xp += 2_000_000;
    for n in 1..=5 {
        assert!(
            !c.grow_spend_xp(start + RAISE_EVERY * n),
            "a raise went out {:?} after {pick:?}, before its answer",
            RAISE_EVERY * n
        );
    }
    server_takes(&mut c, pick, batch);
    assert!(c.grow_spend_xp(start + RAISE_EVERY * 6));
    let (next, next_batch) = sent(&c);
    server_takes(&mut c, next, next_batch);
    assert_eq!(c.session.actions_sent(), 2);
}

/// The raise last sent, still waiting on its answer.
fn sent(c: &Client) -> (Raise, Batch) {
    let p = c.autoplay.growth.pending.expect("a raise was sent");
    (p.raise, p.batch)
}

/// The server taking a raise as ACE does. It would refuse one past the
/// pool or past the experience left to the top, so neither may ever be
/// sent; otherwise the pool pays, the stat takes all of it, and its
/// rank is the highest the experience spent on it reaches.
fn server_takes(c: &mut Client, pick: Raise, batch: Batch) {
    let table = c.assets.xp_table().expect("the XpTable");
    let ladder = c.raise_ladder(&table, pick).expect("on the sheet");
    let left = ladder.cost_to(ladder.top()).expect("not at the top");
    assert!(
        batch.xp <= left,
        "{pick:?}: {} xp is past the {left} left to the top",
        batch.xp
    );
    assert!(
        i64::from(batch.xp) <= c.world.stats.available_xp,
        "{pick:?}: {} xp is more than the pool of {}",
        batch.xp,
        c.world.stats.available_xp
    );
    let ranks = ladder.rank_after(batch.xp);
    assert_eq!(
        ranks,
        ladder.ranks + batch.ranks,
        "{pick:?}: the server's rank is not the one it was sized for"
    );
    let stats = &mut c.world.stats;
    stats.available_xp -= i64::from(batch.xp);
    match pick {
        Raise::Skill(id) => {
            let s = stats
                .skills
                .iter_mut()
                .find(|s| s.id == id)
                .expect("on the sheet");
            s.ranks = ranks as u16;
            s.xp += batch.xp;
        }
        Raise::Attribute(i) => {
            stats.attributes[i].ranks = ranks;
            stats.attributes[i].xp += batch.xp;
        }
        Raise::Vital(i) => {
            stats.vitals[i].ranks = ranks;
            stats.vitals[i].xp += batch.xp;
        }
    }
}

/// Up to `messages` raises, a `RAISE_EVERY` apart from `from`, the
/// server taking each one.
fn buy_ranks(c: &mut Client, from: Instant, messages: u32) -> Vec<Raise> {
    let mut bought = Vec::new();
    for n in 0..messages {
        if !c.grow_spend_xp(from + RAISE_EVERY * n) {
            continue;
        }
        let (pick, batch) = sent(c);
        server_takes(c, pick, batch);
        bought.push(pick);
    }
    bought
}

/// Spending looked at every 100 ms for `how_long` from `from`, the
/// server taking each raise as it goes out. When it stopped.
fn spend_for(c: &mut Client, from: Instant, how_long: Duration) -> Instant {
    let mut t = from;
    while t < from + how_long {
        if c.grow_spend_xp(t) {
            let (pick, batch) = sent(c);
            server_takes(c, pick, batch);
        }
        t += Duration::from_millis(100);
    }
    t
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn raises_sent_while_the_server_is_not_answering_come_to_no_more_than_the_pool() {
    // For forty seconds the local server answered nothing. Each raise
    // was given up on after RAISE_SETTLE and the next one sized on a
    // pool not yet charged for those before it, and without the stats
    // given up on, so each was given their shares too. On paper, on the
    // real XpTable, a caster given ten billion queued eighteen raises
    // for thirty-six billion, and the server refuses all it cannot pay.
    let pool = 10_000_000_000;
    let (mut stalled, mut calm) = (
        with_experience_to_spend(pool),
        with_experience_to_spend(pool),
    );
    let start = Instant::now();
    spend_for(&mut calm, start, Duration::from_secs(120));

    let mut queued = Vec::new();
    let mut t = start;
    while t < start + Duration::from_secs(90) {
        if stalled.grow_spend_xp(t) {
            queued.push(sent(&stalled));
        }
        t += Duration::from_millis(100);
    }
    assert!(queued.len() > 1, "only {queued:?} went out");
    let total: i64 = queued.iter().map(|(_, b)| i64::from(b.xp)).sum();
    assert!(
        total <= pool,
        "{total} xp queued against a pool of {pool}: {queued:?}"
    );
    // The server wakes and takes them in order, refusing none, and the
    // rest of the pool goes as it would have gone.
    for (pick, batch) in queued {
        server_takes(&mut stalled, pick, batch);
    }
    spend_for(&mut stalled, t, Duration::from_secs(120));
    let (s, c) = (&stalled.world.stats, &calm.world.stats);
    assert_eq!(s.available_xp, c.available_xp);
    assert_eq!(s.attributes, c.attributes);
    assert_eq!(s.vitals, c.vitals);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_fight_does_not_sell_off_the_share_kept_for_the_maximums() {
    // On paper, on the real XpTable: a caster given ten billion in the
    // middle of a fight had every other stat bought to its share in
    // thirteen messages, and then sold a rank a message more of them
    // out of the share kept for the maximums. Fifty-six seconds in, Self,
    // Health and Mana came out of the fight ninety ranks short.
    let pool = 10_000_000_000;
    let (mut fighting, mut calm) = (
        with_experience_to_spend(pool),
        with_experience_to_spend(pool),
    );
    let start = Instant::now();
    spend_for(&mut calm, start, Duration::from_secs(120));

    let foe = standing_by(&mut fighting, 0x8000_0301, "Revenant", 3.0).guid;
    fighting.attack_target = Some(foe);
    let bought = buy_ranks(&mut fighting, start, 80);
    assert!(!bought.is_empty(), "spending stopped for the fight");
    assert!(
        !bought.iter().copied().any(raises_a_maximum),
        "a maximum was raised mid-fight: {bought:?}"
    );
    use crate::advance::ATTRIBUTE_NAMES;
    let (f, c) = (&fighting.world.stats, &calm.world.stats);
    for (i, name) in ATTRIBUTE_NAMES.iter().enumerate() {
        let (f, c) = (f.attributes[i].ranks, c.attributes[i].ranks);
        assert!(f <= c, "{name} ran ahead to {f} in the fight, against {c}");
    }

    // The fight is over, and what was kept for the maximums buys them.
    fighting
        .world
        .objects
        .get_mut(&foe)
        .expect("the Revenant")
        .health = Some(0.0);
    spend_for(
        &mut fighting,
        start + RAISE_EVERY * 80,
        Duration::from_secs(120),
    );
    let (f, c) = (&fighting.world.stats, &calm.world.stats);
    assert_eq!(f.available_xp, c.available_xp);
    assert_eq!(f.attributes, c.attributes);
    assert_eq!(f.vitals, c.vitals);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_rank_that_raises_a_maximum_waits_for_the_fight_to_be_over() {
    // A character with a large pool bought Health between swings. Its
    // maximum rose and what was left of it did not, until the fraction
    // left fell under the heal line and it healed, mid-fight, health
    // it had never lost.
    let pool = 100_000_000_000;
    let (mut calm, mut fighting) = (
        with_experience_to_spend(pool),
        with_experience_to_spend(pool),
    );
    let start = Instant::now();
    // With nothing to fight, the maximums are among the best buys.
    let bought = buy_ranks(&mut calm, start, 20);
    assert!(
        bought.iter().copied().any(raises_a_maximum),
        "no maximum is worth buying here, so this proves nothing: {bought:?}"
    );

    let foe = standing_by(&mut fighting, 0x8000_0301, "Revenant", 3.0).guid;
    fighting.attack_target = Some(foe);
    let bought = buy_ranks(&mut fighting, start, 20);
    assert!(
        !bought.iter().copied().any(raises_a_maximum),
        "a maximum was raised mid-fight: {bought:?}"
    );
    // Spending did not stop for the fight: a pool this size takes every
    // attribute that raises no maximum to the top, a message each, and
    // leaves the ones that do where they were.
    let top = fighting
        .assets
        .xp_table()
        .expect("the XpTable")
        .attribute
        .len() as u32
        - 1;
    for (i, a) in fighting.world.stats.attributes.iter().enumerate() {
        let held = raises_a_maximum(Raise::Attribute(i));
        assert_eq!(
            a.ranks,
            if held { 0 } else { top },
            "{} after spending through the fight: {bought:?}",
            crate::advance::ATTRIBUTE_NAMES[i]
        );
    }

    // The Revenant is dead, its experience comes in, and the maximums
    // are bought again.
    fighting
        .world
        .objects
        .get_mut(&foe)
        .expect("the Revenant")
        .health = Some(0.0);
    fighting.world.stats.available_xp += 1_000;
    let bought = buy_ranks(&mut fighting, start + RAISE_EVERY * 20, 20);
    assert!(bought.iter().copied().any(raises_a_maximum), "{bought:?}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_pool_worth_hundreds_of_ranks_is_spent_within_a_minute() {
    // Nine characters granted billions on the local server bought
    // about a rank a second each: a large pool would have taken days.
    let pool = 1_000_000;
    let (mut batched, mut one_at_a_time) = (
        with_experience_to_spend(pool),
        with_experience_to_spend(pool),
    );
    // A rank a message, as it was, to the end of the pool.
    let mut ranks = 0;
    while let Some(pick) = choose_raise(
        &one_at_a_time.raise_offers(),
        one_at_a_time.world.stats.available_xp,
    ) {
        let xp = one_at_a_time.raise_cost(pick).xp().expect("a price");
        server_takes(&mut one_at_a_time, pick, Batch { ranks: 1, xp });
        ranks += 1;
    }
    assert!(
        ranks >= 300,
        "the pool is worth only {ranks} ranks, so this proves nothing"
    );

    let start = Instant::now();
    let (mut t, mut messages) = (start, 0);
    while t < start + Duration::from_secs(60) {
        if batched.grow_spend_xp(t) {
            let (pick, batch) = sent(&batched);
            server_takes(&mut batched, pick, batch);
            messages += 1;
        }
        t += Duration::from_millis(100);
    }
    // All of it that a rank at a time spends: what is left is saved for
    // the next best buy.
    let left = batched.world.stats.available_xp;
    assert_eq!(
        left, one_at_a_time.world.stats.available_xp,
        "{left} of {pool} left after a minute and {messages} messages"
    );
    assert!(
        messages * 5 < ranks,
        "{messages} messages for {ranks} ranks"
    );
    // Spread over the attributes and vitals as a rank at a time
    // spreads it.
    use crate::advance::{ATTRIBUTE_NAMES, VITAL_NAMES};
    let (b, o) = (&batched.world.stats, &one_at_a_time.world.stats);
    for (i, name) in ATTRIBUTE_NAMES.iter().enumerate() {
        let (b, o) = (b.attributes[i].ranks, o.attributes[i].ranks);
        assert!(b.abs_diff(o) <= 1, "{name}: {b} against {o}");
    }
    for (i, name) in VITAL_NAMES.iter().enumerate() {
        let (b, o) = (b.vitals[i].ranks, o.vitals[i].ranks);
        assert!(b.abs_diff(o) <= 1, "{name}: {b} against {o}");
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_kills_worth_of_experience_still_goes_a_rank_a_message() {
    let mut c = with_experience_to_spend(400);
    let start = Instant::now();
    let (mut t, mut messages) = (start, 0);
    while t < start + Duration::from_secs(30) {
        if c.grow_spend_xp(t) {
            let (pick, batch) = sent(&c);
            let one = c.raise_cost(pick).xp().map(|xp| Batch { ranks: 1, xp });
            assert_eq!(Some(batch), one, "{pick:?}");
            server_takes(&mut c, pick, batch);
            messages += 1;
        }
        t += Duration::from_millis(100);
    }
    assert!(messages >= 2, "only {messages} ranks");
}
