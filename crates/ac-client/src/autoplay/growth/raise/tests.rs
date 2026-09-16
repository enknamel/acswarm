use super::*;

fn offer(raise: Raise, cost: u32, weight: f32) -> Offer {
    Offer {
        raise,
        cost,
        weight,
    }
}

#[test]
fn the_best_value_rank_is_bought_and_none_beyond_the_pool() {
    let offers = [
        offer(Raise::Skill(47), 1000, 1.0),
        offer(Raise::Skill(24), 300, 0.3),
        offer(Raise::Attribute(3), 500, 0.6),
        offer(Raise::Vital(0), 400, 0.5),
    ];
    // Attribute: 500/0.6 = 833 beats 1000, 1000 and 800? No: health
    // 400/0.5 = 800 is the best value.
    assert_eq!(choose_raise(&offers, 5000), Some(Raise::Vital(0)));
    // With less to spend than the best buy costs, nothing: the pool
    // saves for it rather than going on a worse buy that fits.
    assert_eq!(choose_raise(&offers, 399), None);
    assert_eq!(choose_raise(&offers, 350), None);
    assert_eq!(choose_raise(&offers, 400), Some(Raise::Vital(0)));
    assert_eq!(choose_raise(&[], 100), None);
    // The main skill wins over a minor one at the same price.
    let tie = [
        offer(Raise::Skill(24), 300, 0.3),
        offer(Raise::Skill(47), 300, 1.0),
    ];
    assert_eq!(choose_raise(&tie, 300), Some(Raise::Skill(47)));
    // A zero weight is never bought, nor saved for.
    assert_eq!(choose_raise(&[offer(Raise::Skill(1), 1, 0.0)], 10), None);
    let free = [
        offer(Raise::Skill(1), 1, 0.0),
        offer(Raise::Skill(24), 300, 0.3),
    ];
    assert_eq!(choose_raise(&free, 300), Some(Raise::Skill(24)));
}

/// A column shaped like the XpTable's: the first rank costs `first`,
/// each after it a twentieth more, up to rank `top`.
fn column(first: u32, top: usize) -> Vec<u32> {
    let mut table = vec![0u32];
    let (mut total, mut step) = (0.0f64, f64::from(first));
    for _ in 0..top {
        total += step.round();
        table.push(total as u32);
        step *= 1.05;
    }
    table
}

/// A stat standing at `ranks` of `table`, nothing spent past them, and
/// not held.
fn climb(raise: Raise, table: &[u32], ranks: u32, weight: f32) -> Climb<'_> {
    Climb {
        raise,
        ladder: crate::advance::Ladder {
            table,
            ranks,
            spent: table[ranks as usize],
        },
        weight,
        held: false,
    }
}

/// Spend `pool` over `field` the way `grow_spend_xp` does, a message at
/// a time until nothing more is affordable, and take each message as
/// the server does: batched, or a rank a message as before batches.
/// `each` looks at the field after every message. How many messages
/// it took, and what is left of the pool.
fn spend_on_paper(
    field: &mut [Climb],
    mut pool: i64,
    batched: bool,
    mut each: impl FnMut(&[Climb]),
) -> (u32, i64) {
    let mut messages = 0;
    loop {
        let offers: Vec<Offer> = field
            .iter()
            .filter_map(|c| Some(offer(c.raise, c.ladder.next_cost().xp()?, c.weight)))
            .collect();
        let Some(pick) = choose_raise(&offers, pool) else {
            return (messages, pool);
        };
        let batch = if batched {
            let (raise, batch) = batch_raise(field, pool).expect("the best buy is affordable");
            assert_eq!(raise, pick, "sent for another stat than the best buy");
            batch
        } else {
            let o = offers.iter().find(|o| o.raise == pick).expect("on offer");
            Batch {
                ranks: 1,
                xp: o.cost,
            }
        };
        let c = field
            .iter_mut()
            .find(|c| c.raise == pick)
            .expect("in the field");
        let left = c.ladder.cost_to(c.ladder.top()).expect("not at the top");
        assert!(
            batch.xp <= left,
            "{pick:?}: {} is past the {left} left to the top",
            batch.xp
        );
        assert!(
            i64::from(batch.xp) <= pool,
            "{pick:?}: {} is more than the pool of {pool}",
            batch.xp
        );
        let ranks = c.ladder.rank_after(batch.xp);
        assert_eq!(ranks, c.ladder.ranks + batch.ranks, "{pick:?}: sized wrong");
        pool -= i64::from(batch.xp);
        c.ladder.ranks = ranks;
        c.ladder.spent += batch.xp;
        messages += 1;
        each(field);
    }
}

#[test]
fn a_large_pool_goes_out_a_message_a_stat_and_spread_as_a_rank_at_a_time_spreads_it() {
    let (attribute, vital, skill) = (column(110, 190), column(73, 196), column(23, 226));
    let start = [
        climb(Raise::Skill(47), &skill, 40, 1.0),
        climb(Raise::Skill(6), &skill, 20, 0.5),
        climb(Raise::Attribute(3), &attribute, 30, 0.6),
        climb(Raise::Attribute(0), &attribute, 10, 0.15),
        climb(Raise::Vital(0), &vital, 5, 0.5),
        climb(Raise::Vital(2), &vital, 0, 0.15),
    ];
    for pool in [500_000, 5_000_000] {
        // A rank a message, as it was: hundreds of messages.
        let mut one_at_a_time = start;
        let (slow, left) = spend_on_paper(&mut one_at_a_time, pool, false, |_| {});
        assert!(
            slow >= 300,
            "a pool of {pool} is worth only {slow} ranks, so this proves nothing"
        );

        // Batched: about a message a stat, and no stat is ever taken
        // past the rank a rank at a time leaves it at.
        let mut batched = start;
        let (fast, batched_left) = spend_on_paper(&mut batched, pool, true, |field| {
            for (now, end) in field.iter().zip(&one_at_a_time) {
                assert!(
                    now.ladder.ranks <= end.ladder.ranks,
                    "{:?} ran ahead to {} of the {} it ends at",
                    now.raise,
                    now.ladder.ranks,
                    end.ladder.ranks
                );
            }
        });
        assert!(
            fast <= 2 * start.len() as u32,
            "{fast} messages for {} stats, against {slow} a rank at a time",
            start.len()
        );
        // And it ends where a rank at a time ends, rank for rank.
        assert_eq!(batched_left, left);
        for (b, o) in batched.iter().zip(&one_at_a_time) {
            assert_eq!(b.ladder, o.ladder, "{:?} with a pool of {pool}", b.raise);
        }
    }
}

#[test]
fn a_kills_worth_of_experience_buys_the_one_rank_as_it_always_did() {
    let skill = column(23, 226);
    let field = [
        climb(Raise::Skill(47), &skill, 40, 1.0),
        climb(Raise::Skill(6), &skill, 40, 0.5),
    ];
    let rank = field[0].ladder.step(40).expect("a rank above");
    let one = Some((Raise::Skill(47), Batch { ranks: 1, xp: rank }));
    for pool in [rank, 3 * rank, BATCH_FROM as u32 * rank - 1] {
        assert_eq!(
            batch_raise(&field, i64::from(pool)),
            one,
            "a pool of {pool}"
        );
    }
    // Short of the rank, nothing; worth many of it, several.
    assert_eq!(batch_raise(&field, i64::from(rank) - 1), None);
    let (_, many) = batch_raise(&field, 100 * i64::from(rank)).expect("a batch");
    assert!(many.ranks > 1, "{many:?}");
    // Nothing while every stat is held.
    let held = field.map(|c| Climb { held: true, ..c });
    assert_eq!(batch_raise(&held, 1_000_000), None);
}

#[test]
fn a_trickle_of_kills_buys_what_the_same_experience_in_one_lump_buys() {
    // On paper, on the real XpTable: after a caster spent ten billion,
    // two billion more a kill at a time went only on ranks that cost
    // little -- Arcane Lore, Jump, Loyalty, Salvaging, Strength,
    // Coordination and Quickness ten each -- and War, Life, Focus,
    // Self and Health none, where the same two billion at once gave War
    // three and the others two. A kill never filled the pool to the
    // best buy before a cheap rank took it.
    let (attribute, vital, skill) = (column(110, 190), column(73, 196), column(23, 226));
    let mut spent = [
        climb(Raise::Skill(47), &skill, 0, 1.0),
        climb(Raise::Skill(6), &skill, 0, 0.6),
        climb(Raise::Skill(22), &skill, 0, 0.15),
        climb(Raise::Attribute(4), &attribute, 0, 0.6),
        climb(Raise::Attribute(0), &attribute, 0, 0.15),
        climb(Raise::Vital(0), &vital, 0, 0.5),
    ];
    let (_, left) = spend_on_paper(&mut spent, 5_000_000, true, |_| {});
    let (kill, kills): (i64, i64) = (2_000, 1_000);
    // The cheapest rank now costs several kills, and the best buy many
    // more.
    let next = |c: &Climb| c.ladder.step(c.ladder.ranks).expect("below the top");
    assert!(
        spent.iter().all(|c| i64::from(next(c)) > 4 * kill),
        "{spent:?}"
    );

    let mut lump = spent;
    let (_, lump_left) = spend_on_paper(&mut lump, left + kill * kills, true, |_| {});
    let mut trickle = spent;
    let mut pool = left;
    for _ in 0..kills {
        (_, pool) = spend_on_paper(&mut trickle, pool + kill, true, |_| {});
    }
    assert_eq!(pool, lump_left);
    for ((t, l), s) in trickle.iter().zip(&lump).zip(&spent) {
        assert_eq!(t.ladder, l.ladder, "{:?}", t.raise);
        assert!(
            t.ladder.ranks > s.ladder.ranks,
            "{:?} got nothing of {} kills",
            t.raise,
            kills
        );
    }
}

#[test]
fn a_batch_stops_at_the_top_rank_whatever_the_pool() {
    // A stat partway to its next rank, a column whose top is nearly all
    // a u32 holds, and a pool of a hundred billion.
    let table = [0u32, 1_000, 3_000, 4_000_000_000, u32::MAX - 5];
    let partway = Climb {
        raise: Raise::Attribute(0),
        ladder: crate::advance::Ladder {
            table: &table,
            ranks: 1,
            spent: 2_500,
        },
        weight: 1.0,
        held: false,
    };
    let batch = batch_raise(&[partway], 100_000_000_000).expect("a batch");
    // To the top and no further: exactly the experience left to it,
    // which is all the server takes and fits the u32 a message carries.
    assert_eq!(
        batch,
        (
            partway.raise,
            Batch {
                ranks: 3,
                xp: u32::MAX - 5 - 2_500
            }
        )
    );
    assert_eq!(
        Some(batch.1.xp),
        partway.ladder.cost_to(partway.ladder.top())
    );
    // At the top, nothing.
    let topped = Climb {
        ladder: crate::advance::Ladder {
            ranks: 4,
            spent: u32::MAX - 5,
            ..partway.ladder
        },
        ..partway
    };
    assert_eq!(batch_raise(&[topped], 100_000_000_000), None);
}

#[test]
fn a_maximum_held_for_the_fight_keeps_its_share_of_the_pool() {
    let (skill, vital) = (column(23, 226), column(73, 196));
    let weapon = climb(Raise::Skill(47), &skill, 50, 1.0);
    let health = climb(Raise::Vital(0), &vital, 10, 0.5);
    let held = Climb {
        held: true,
        ..health
    };
    let pool = 1_000_000;
    // With nothing else in the running, the weapon's skill takes the
    // pool.
    let (_, alone) = batch_raise(&[weapon], pool).expect("a batch");
    assert!(i64::from(alone.xp) > pool * 9 / 10, "{alone:?}");
    // With Health held for the fight but still in the running, only
    // its own share, and Health nothing.
    let (raise, sharing) = batch_raise(&[weapon, held], pool).expect("a batch");
    assert_eq!(raise, weapon.raise);
    assert!(
        sharing.ranks < alone.ranks && sharing.xp < alone.xp,
        "{sharing:?} against {alone:?}"
    );

    // Nor is a rank of the weapon's sold out of Health's share when the
    // pool runs short. Health's next three ranks are better buys than
    // the weapon's next, and the one after them is not.
    let (h, w) = (
        |r| health.ladder.step(r).expect("below the top"),
        weapon.ladder.step(50).expect("below the top"),
    );
    let worth = |cost: u32, weight: f32| cost as f32 / weight;
    assert!(worth(h(12), 0.5) < worth(w, 1.0) && worth(h(13), 0.5) > worth(w, 1.0));
    let kept = h(10) + h(11) + h(12);
    // A pool one short of those and the weapon's rank: the weapon waits.
    assert_eq!(batch_raise(&[weapon, held], i64::from(kept + w - 1)), None);
    // Enough for both: the weapon's one rank, and no more.
    assert_eq!(
        batch_raise(&[weapon, held], i64::from(kept + w)),
        Some((weapon.raise, Batch { ranks: 1, xp: w }))
    );
    // Not held, Health is bought first.
    assert_eq!(
        batch_raise(&[weapon, health], i64::from(kept + w - 1)).map(|(r, _)| r),
        Some(health.raise)
    );
}

#[test]
fn skills_are_weighed_by_how_the_character_fights() {
    // An archer: bows first, fletching worth something, war magic not.
    assert_eq!(skill_weight(47, Some(47), false), 1.0);
    assert!(skill_weight(37, Some(47), false) > skill_weight(37, Some(44), false));
    assert!(skill_weight(34, Some(47), false) < 0.5);
    // A caster: war magic first, life magic close behind.
    assert_eq!(skill_weight(34, Some(0), true), 1.0);
    assert!(skill_weight(33, Some(0), true) > 0.8);
    // Another weapon skill is nearly worthless.
    assert!(skill_weight(44, Some(47), false) < 0.2);
}
