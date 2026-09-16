use std::time::{Duration, Instant};

use super::*;
use crate::autoplay::loot::choose::LOOT_NEAR;
use crate::autoplay::{Mate, ShutFor};
use crate::testkit::{looter, turn_at, view_of};

#[test]
fn every_session_deals_a_newly_fallen_body_to_the_same_character() {
    // A claim is said every half second and a body is chosen within
    // a tick of falling, so for that first moment there is no claim
    // to read and the nine have to agree without one. Every session
    // works the same answer out of the same roster, and the rest
    // stand off.
    let t0 = Instant::now();
    let body = 0x8000_0001;
    let at = glam::Vec3::ZERO;
    let fleet: Vec<u32> = (0..9).map(|i| 0x5000_0010 + i).collect();
    let others = |me: u32| -> Vec<Mate> {
        fleet
            .iter()
            .filter(|g| **g != me)
            .map(|g| looter(*g, at, None, Duration::ZERO))
            .collect()
    };
    // Not the lowest guid, which used to open every body it stood
    // over: the deal for this body.
    let first = 0x5000_0012;
    for &me in &fleet {
        let ap = Autoplay {
            corpse_seen: vec![(body, t0)],
            team: view_of(others(me)),
            ..Default::default()
        };
        assert_eq!(
            ap.team.opens_first(body, at, turn_at(me, at, 0)),
            first,
            "disagreed about whose turn"
        );
        assert_eq!(
            ap.ours_to_open(body, at, me, at, t0),
            me == first,
            "{me:#010x} did not stand off"
        );
        // Once the board has had time to catch up the claims are
        // the truth, and whoever has not been told otherwise goes
        // ahead.
        assert!(ap.ours_to_open(body, at, me, at, t0 + CLAIM_SETTLE));
    }

    // Only the ones free to open it count. A mate played by hand, a
    // dead one, one not in the world, one already at another body,
    // one fighting, one with a full pack, one laden and one across
    // the field all leave it to us, whatever the deal.
    let mine = fleet[8];
    let field_away = glam::Vec3::new(LOOT_NEAR + 5.0, 0.0, 0.0);
    let packed = |pack_full, laden| crate::logistics::Supplies {
        pack_full,
        laden,
        ..Default::default()
    };
    let view = view_of(vec![
        Mate {
            autoplay: false,
            ..looter(fleet[0], at, None, Duration::ZERO)
        },
        Mate {
            health: 0.0,
            ..looter(fleet[1], at, None, Duration::ZERO)
        },
        looter(0, at, None, Duration::ZERO),
        looter(fleet[2], at, Some(0x8000_0002), Duration::ZERO),
        Mate {
            target: Some(0x7000_0001),
            ..looter(fleet[3], at, None, Duration::ZERO)
        },
        Mate {
            supplies: packed(true, false),
            ..looter(fleet[4], at, None, Duration::ZERO)
        },
        Mate {
            supplies: packed(false, true),
            ..looter(fleet[5], at, None, Duration::ZERO)
        },
        looter(fleet[6], field_away, None, Duration::ZERO),
    ]);
    assert_eq!(view.opens_first(body, at, turn_at(mine, at, 0)), mine);

    // And this character is judged by the same tests as the rest. A
    // body is owed to a caster out to the fight radius where one of
    // its kills fell, which is further than a mate has to stand to
    // count: with no reach test on ourselves, the caster called the
    // body its own while every mate's roster ruled the caster out,
    // and the two of them opened it in the same second.
    let over_it = view_of(vec![looter(fleet[0], at, None, Duration::ZERO)]);
    assert_eq!(
        over_it.opens_first(body, at, turn_at(mine, field_away, 0)),
        fleet[0],
        "claimed a body from across the field"
    );
    // Nor is this one dealt a turn while it is at another body, or
    // with no room for what is on this one.
    for busy in [
        Turn {
            looting: true,
            ..turn_at(mine, at, 0)
        },
        Turn {
            room: false,
            ..turn_at(mine, at, 0)
        },
    ] {
        assert_eq!(over_it.opens_first(body, at, busy), fleet[0]);
    }
    // Nobody within reach at all: it is ours to walk to.
    assert_eq!(
        view_of(vec![looter(fleet[0], field_away, None, Duration::ZERO)]).opens_first(
            body,
            at,
            turn_at(mine, field_away, 0)
        ),
        mine,
        "left a body nobody was near"
    );
}

#[test]
fn the_turns_go_round_body_by_body() {
    // Three characters over one spot, all free. Whoever opens a body
    // first says so on its row, and the next body goes to one that
    // has had fewer turns.
    let at = glam::Vec3::ZERO;
    let party = [0x5000_0021u32, 0x5000_0022, 0x5000_0023];
    let mut opened = [0u16; 3];
    let mut who_opened = Vec::new();
    for n in 0..6u32 {
        let body = 0x8000_0100 + n;
        // Each session works it out for itself, from what the others
        // said about their turns.
        let dealt: Vec<u32> = (0..3)
            .map(|i| {
                let mates = (0..3)
                    .filter(|j| *j != i)
                    .map(|j| Mate {
                        opened_first: opened[j],
                        ..looter(party[j], at, None, Duration::ZERO)
                    })
                    .collect();
                view_of(mates).opens_first(body, at, turn_at(party[i], at, opened[i]))
            })
            .collect();
        assert!(dealt.iter().all(|g| *g == dealt[0]), "{n}: {dealt:x?}");
        let i = party.iter().position(|g| *g == dealt[0]).unwrap();
        opened[i] += 1;
        who_opened.push(i);
    }
    // Each of the three once in every round of three bodies.
    for round in who_opened.chunks(3) {
        let mut round = round.to_vec();
        round.sort_unstable();
        assert_eq!(round, vec![0, 1, 2], "{who_opened:?}");
    }
}

#[test]
fn bodies_falling_together_go_to_different_characters() {
    // Nine bodies in one tick, before anyone's turn is on the board:
    // the lowest guid used to be dealt every one of them.
    let at = glam::Vec3::ZERO;
    let fleet: Vec<u32> = (0..9).map(|i| 0x5000_0010 + i).collect();
    let view = view_of(
        fleet[1..]
            .iter()
            .map(|g| looter(*g, at, None, Duration::ZERO))
            .collect(),
    );
    let dealt: Vec<u32> = (1..=9)
        .map(|n| view.opens_first(0x8000_0000 + n, at, turn_at(fleet[0], at, 0)))
        .collect();
    let mut to = dealt.clone();
    to.sort_unstable();
    to.dedup();
    assert_eq!(to.len(), 7, "{dealt:x?}");
    for g in &to {
        assert!(dealt.iter().filter(|d| *d == g).count() <= 2);
    }
}

#[test]
fn the_dealing_mix_gives_the_same_numbers_everywhere() {
    // Worked out by hand from splitmix64's finish: no build, process
    // or platform may deal a body differently.
    assert_eq!(deal(0, 0), 0xe220_a839_7b1d_cdaf);
    assert_eq!(deal(0x8000_0001, 0x5000_0010), 0xfe12_cdc0_9d21_d7ba);
    assert_eq!(deal(0x8000_0001, 0x5000_0011), 0xd972_5b91_3475_6d9c);
    assert_eq!(deal(0x8000_198f, 0x5000_0001), 0x2ac7_ac48_90d6_3ca3);
}

#[test]
fn a_busy_fellows_turn_passes_on_and_the_body_is_still_opened() {
    // Best effort: a character fighting when a body falls loses its
    // turn to one that is free, and a turn nobody takes never leaves
    // the body lying.
    let t0 = Instant::now();
    let (body, at, foe) = (0x8000_0001, glam::Vec3::ZERO, 0x7000_0001);
    let party = [0x5000_0031u32, 0x5000_0032, 0x5000_0033];
    let session = |me: u32, fighting: &[u32]| {
        let row = |g: u32| Mate {
            target: fighting.contains(&g).then_some(foe),
            ..looter(g, at, None, Duration::ZERO)
        };
        let mut ap = Autoplay {
            corpse_seen: vec![(body, t0)],
            team: view_of(
                party
                    .iter()
                    .filter(|g| **g != me)
                    .map(|g| row(*g))
                    .collect(),
            ),
            ..Default::default()
        };
        // Each reads itself off what it said about itself.
        ap.team.me = Some(row(me));
        ap
    };
    let opening = |fighting: &[u32], now: Instant| -> Vec<u32> {
        party
            .iter()
            .copied()
            .filter(|me| session(*me, fighting).ours_to_open(body, at, *me, at, now))
            .collect()
    };
    // All free: one of them has the turn.
    let dealt = opening(&[], t0);
    assert_eq!(dealt.len(), 1, "{dealt:x?}");
    // That one fighting: another, free, has it instead.
    let passed = opening(&dealt, t0);
    assert_eq!(passed.len(), 1, "{passed:x?}");
    assert_ne!(passed, dealt, "the turn waited on one fighting");
    // After the first second the body is anyone's who is free, the
    // one that was fighting too once it is done.
    assert_eq!(opening(&dealt, t0 + CLAIM_SETTLE), party.to_vec());
    // Everyone fighting: nobody stands off for anybody.
    assert_eq!(opening(&party, t0), party.to_vec());
}

#[test]
fn a_body_opened_first_is_a_turn_and_one_left_for_this_character_is_not() {
    use crate::did::Did;
    let t0 = Instant::now();
    let me = 2;
    let mut ap = Autoplay::default();
    ap.config.enabled = true;
    ap.config.team.enabled = true;
    assert_eq!(ap.opened_first(t0), 0);
    ap.corpse_shut(0x8000_0001, &Did::Done, None, ShutFor::default(), t0);
    assert_eq!(ap.opened_first(t0), 1);
    // One of the others shut this one first and left it for us: going
    // back for what it left is no turn.
    let passed_on = 0x8000_0002;
    ap.team.mates = vec![Mate {
        shut: vec![Shut {
            body: passed_on,
            done_for: vec![3],
            ..Default::default()
        }],
        ..looter(1, glam::Vec3::ZERO, None, Duration::ZERO)
    }];
    assert!(
        ap.take_in_shuts(me, t0, |_| None).is_empty(),
        "left for us, not done for us"
    );
    assert!(ap.has_shut(passed_on, 1));
    ap.corpse_shut(passed_on, &Did::Done, None, ShutFor::default(), t0);
    assert_eq!(ap.opened_first(t0), 1);
    // Nor is a body set aside.
    ap.corpse_shut(
        0x8000_0003,
        &Did::blocked("it would not give something up"),
        None,
        ShutFor::default(),
        t0,
    );
    assert_eq!(ap.opened_first(t0), 1);
    // Turns are counted lately, not for ever.
    assert_eq!(ap.opened_first(t0 + DEAL_WINDOW), 0);
    // And what was heard goes with the body.
    ap.forget_corpses_gone(|g| g != passed_on);
    assert!(!ap.has_shut(passed_on, 1));
}
