use super::*;
use crate::testkit::standing_at;

fn offer(raise: Raise, cost: u32, weight: f32) -> Offer {
    Offer {
        raise,
        cost,
        weight,
    }
}

#[test]
fn blargerton_has_room_to_loot_with_everything_he_keeps_on_him() {
    // Strength 60: capacity 9000, a limit of 13500 at 1.5, twice his
    // capacity at 18000 and the wall at 27000. He carried 13866 and
    // had taken nothing, so all of it was his own: plate 6540, four
    // foci 1600, a stack of tapers 1374, then weapons and the rest.
    let (carried, capacity) = (13_866, 9_000);
    // Counting all of it as loot left him nothing: the fault.
    assert_eq!(loot_room(carried, carried, capacity, 1.5), 0);
    // None of it is loot. He takes on loot until he reaches twice
    // his capacity, which comes before the limit does.
    assert_eq!(loot_room(carried, 0, capacity, 1.5), 18_000 - 13_866);
}

#[test]
fn kept_things_in_the_pack_do_not_fill_a_low_limit() {
    // Leaving out only what he wore still counted his foci, tapers
    // and weapons as loot: 7326 of it, over a limit of 7200 at 0.8,
    // and no room again.
    let (carried, worn, capacity) = (13_866, 6_540, 9_000);
    assert_eq!(loot_room(carried, carried - worn, capacity, 0.8), 0);
    // Measured on what a counter would take, there is room at every
    // setting the slider allows.
    for up_to in [0.5, 0.8, 1.0, 1.5, 2.0, 2.9] {
        assert!(loot_room(carried, 0, capacity, up_to) > 0, "at {up_to}");
    }
    // A lighter character at a low setting is stopped by the limit
    // itself: 4500 of loot at 0.5, whatever it keeps.
    assert_eq!(loot_room(3_000, 0, 9_000, 0.5), 4_500);
    assert_eq!(loot_room(3_000 + 4_500, 4_500, 9_000, 0.5), 0);
}

#[test]
fn loot_does_not_take_a_character_in_plate_past_twice_its_capacity() {
    // Plate 6540 and 11460 of loot is 18000, twice his capacity, and
    // no Melee or Missile Defense left. The limit alone would have
    // let him take 2040 more and fight on to 20040.
    assert_eq!(loot_room(18_000, 11_460, 9_000, 1.5), 0);
    // A thousand short of it, a thousand is all the room there is.
    assert_eq!(loot_room(17_000, 10_460, 9_000, 1.5), 1_000);
    // A player who asks for more than twice gets it: at 2.5 the line
    // is the limit's own, 22500.
    assert_eq!(loot_room(6_540, 0, 9_000, 2.5), 22_500 - 6_540);
    assert_eq!(loot_room(22_500, 15_960, 9_000, 2.5), 0);
}

#[test]
fn what_is_kept_past_twice_capacity_leaves_the_limit_and_the_wall() {
    // What it keeps is at twice its capacity on its own: loot cannot
    // slow it further and no sale gets it back under the line, so
    // the line gives way rather than leave every corpse untouched.
    assert_eq!(loot_room(18_000, 0, 9_000, 1.5), 9_000);
    assert_eq!(loot_room(20_000, 0, 9_000, 1.5), 7_000);
    assert_eq!(loot_room(20_000 + 5_000, 5_000, 9_000, 1.5), 2_000);
    // At a low setting the limit still stops it first.
    assert_eq!(loot_room(18_000, 0, 9_000, 0.5), 4_500);
    assert_eq!(loot_room(18_000 + 4_500, 4_500, 9_000, 0.5), 0);
}

#[test]
fn the_servers_wall_still_caps_the_room() {
    // A limit set by hand past three times: the limit and the line
    // both allow more than the server will hand over.
    assert_eq!(loot_room(20_000, 8_000, 9_000, 3.5), 27_000 - 20_000);
    // At the wall there is no room, however light the loot.
    assert_eq!(loot_room(27_000, 7_000, 9_000, 1.5), 0);
    assert_eq!(loot_room(30_000, 0, 9_000, 1.5), 0);
}

#[test]
fn the_room_never_passes_a_line_and_a_sale_always_makes_some() {
    let capacity = 9_000u32;
    let wall = 27_000u32;
    for up_to in [0.5f32, 0.8, 1.0, 1.5, 2.0, 2.5, 2.9] {
        let limit = (capacity as f32 * up_to) as u32;
        let line = (capacity as f32 * up_to.max(2.0)) as u32;
        for kept in (0..=30_000u32).step_by(250) {
            for loot in (0..=30_000u32).step_by(250) {
                let carried = kept + loot;
                let room = loot_room(carried, loot, capacity, up_to);
                let at = format!("up to {up_to}, kept {kept}, loot {loot}: room {room}");
                if room == 0 {
                    continue;
                }
                // Never past the limit on loot, nor the wall on
                // everything.
                assert!(loot + room <= limit, "limit: {at}");
                assert!(carried + room <= wall, "wall: {at}");
                // Never past twice capacity while what it keeps is
                // under it.
                if kept < line {
                    assert!(carried + room <= line, "line: {at}");
                }
            }
            // Sold down to what it keeps, it has room again -- unless
            // that is already at the wall, which no sale can help.
            let sold = loot_room(kept, 0, capacity, up_to);
            assert_eq!(
                sold == 0,
                kept >= wall,
                "sold down: up to {up_to}, kept {kept}"
            );
        }
    }
}

#[test]
fn no_capacity_or_no_limit_is_no_room() {
    // Strength not heard yet: capacity 0, so nothing is claimed.
    assert_eq!(loot_room(0, 0, 0, 1.5), 0);
    assert_eq!(loot_room(500, 0, 0, 1.5), 0);
    // A limit of nothing, less, or not a number is nothing.
    assert_eq!(loot_room(0, 0, 9_000, 0.0), 0);
    assert_eq!(loot_room(0, 0, 9_000, -1.0), 0);
    assert_eq!(loot_room(0, 0, 9_000, f32::NAN), 0);
    // A count of loot ahead of the server's total: all of it is loot.
    assert_eq!(loot_room(1_000, 1_500, 9_000, 1.5), 13_500 - 1_000);
}

#[test]
fn a_character_with_room_for_nothing_it_wants_goes_to_sell() {
    // Laden needed no room at all. The loot rules take only what fits,
    // so the room settled a little above nothing: forty short of a
    // mace, every body with one on it was shut as too laden, and the
    // character hunted on and never went to sell.
    let sold = 18_000 - 13_866;
    assert!(!had_enough(40, None, sold), "nothing left behind yet");
    assert!(had_enough(40, Some(300), sold), "forty short of a mace");
    // Room for it again -- tapers burnt, or a sale -- and it is not.
    assert!(!had_enough(300, Some(300), sold));
    assert!(!had_enough(sold, Some(300), sold));
    // No room at all is enough, as it always was.
    assert!(had_enough(0, None, sold));
    // A thing no sale could make room for is no reason to go: an anvil
    // heavier than the room left with every bit of loot sold.
    assert!(!had_enough(40, Some(9_000), sold));
}

#[test]
fn a_character_with_nobody_to_restock_with_goes_to_town_on_its_own() {
    // Blargerton: team rules on, restocking together on, no party.
    // The party's mode decided his trips, a party of one never left
    // for weight, and he hunted on laden with every body left full.
    let team = crate::autoplay::Team {
        enabled: true,
        restock: crate::logistics::Restock {
            together: true,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(!restocks_as_a_party(&team, 0), "alone is alone");
    assert!(restocks_as_a_party(&team, 1));
    // Either setting off, it is alone whoever else is about.
    let apart = crate::autoplay::Team {
        restock: crate::logistics::Restock {
            together: false,
            ..Default::default()
        },
        ..team.clone()
    };
    assert!(!restocks_as_a_party(&apart, 3));
    let off = crate::autoplay::Team {
        enabled: false,
        ..team
    };
    assert!(!restocks_as_a_party(&off, 3));
}

/// Whether a run standing `away` metres from its counter is there.
fn there(away: f32) -> bool {
    away <= VENDOR_REACH
}

#[test]
fn a_walk_to_a_counter_broken_off_by_a_corpse_is_walked_on() {
    // +Verity, carrying as much as she meant to, set off for Shopkeeper
    // Renald the Elder 250 m away. A second later the looting walked
    // her to a fresh corpse, which ends a journey, and the run found no
    // journey under way 224 m short: "could not get to", sold nothing.
    assert_eq!(
        on_the_way(there(224.0), true, false, false),
        OnTheWay::WalkOn
    );
    // Not while what broke it off still has her: a fight on the way.
    assert_eq!(on_the_way(there(224.0), true, true, false), OnTheWay::Wait);
    // A journey that ended by itself short of the counter -- it gave
    // up, or arrived somewhere else -- still could not get there.
    assert_eq!(
        on_the_way(there(224.0), false, false, false),
        OnTheWay::Short
    );
    assert_eq!(
        on_the_way(there(224.0), false, true, false),
        OnTheWay::Short
    );
    // Near enough, she goes up to the counter however it ended.
    for (broken_off, busy) in [(false, false), (true, false), (true, true)] {
        assert_eq!(
            on_the_way(there(VENDOR_REACH), broken_off, busy, false),
            OnTheWay::There
        );
    }
}

#[test]
fn a_walk_broken_off_again_as_soon_as_it_is_planned_waits_before_the_next_plan() {
    // Something that ends the walk to the counter the moment it is
    // planned -- a follower pulled back to its leader each time it
    // closed to the following distance -- had it planned again every
    // tick or two for the run's four minutes, a route search each time.
    assert_eq!(on_the_way(there(224.0), true, false, true), OnTheWay::Wait);
    // Once the moment is up it is planned again.
    assert_eq!(
        on_the_way(there(224.0), true, false, false),
        OnTheWay::WalkOn
    );
    // Neither giving up nor going up to the counter waits on it.
    assert_eq!(
        on_the_way(there(224.0), false, false, true),
        OnTheWay::Short
    );
    assert_eq!(
        on_the_way(there(VENDOR_REACH), true, false, true),
        OnTheWay::There
    );
}

/// A run on its way to a counter at `at`, set off at `now`.
fn run_to(at: Vec2, now: Instant) -> Run {
    Run {
        vendor: "Shopkeeper Renald the Elder".into(),
        at,
        phase: Phase::Going,
        since: now,
        last_sell: None,
        town: at,
        stops: 1,
        sold: 0,
        reason: "carrying as much as it means to".into(),
        errand: Errand::Sell,
        visited: vec![at],
        walked_on: None,
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_follower_on_its_own_town_run_is_not_pulled_back_to_its_leader() {
    // A party restocking with everyone going: each follower makes its
    // own run while the leader goes on leading. Following ranks above
    // the run, so each time the follower closed to its following
    // distance the walk after the leader ended the run's journey, the
    // run planned it again, and the walk after the leader ended it
    // again, for the run's four minutes.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    let me = c.player.as_ref().unwrap().world_position();
    let team = &mut c.autoplay.config.team;
    team.enabled = true;
    team.follow = true;
    team.lead = false;
    team.follow_distance = 4.0;
    let leader_off = |metres: f32| crate::autoplay::Mate {
        name: "Leader".into(),
        leader: true,
        leads: true,
        world: me + glam::Vec3::new(metres, 0.0, 0.0),
        cell: holtburg,
        ..Default::default()
    };
    let cfg = c.autoplay.config.growth.clone();
    let now = Instant::now();
    let counter = Vec2::new(me.x + 250.0, me.y);
    assert!(c.grow_travel(counter, now), "no way to the counter");
    c.autoplay.growth.run = Some(run_to(counter, now));

    // The leader walks off and stops, and walks off again: the
    // follower keeps to its own walk.
    for (tick, metres) in [12.0, 3.0, 12.0, 3.0, 30.0, 3.0].into_iter().enumerate() {
        c.autoplay.team.mates = vec![leader_off(metres)];
        assert!(!c.autoplay_follow(now, true), "caught up at tick {tick}");
        assert!(!c.autoplay_follow(now, false), "followed at tick {tick}");
        assert!(
            c.traveling(),
            "its walk to the counter ended at tick {tick}"
        );
        assert!(c.grow_run_step(now, &cfg).goes_on());
        assert!(c.traveling());
    }

    // A corpse on the way does break the walk off, and the run walks
    // on once it is dealt with. Broken off again straight away, the run
    // waits a moment before planning it once more.
    c.interrupt_travel("walking to a corpse");
    assert!(c.journey_broken_off());
    assert!(c.grow_run_step(now, &cfg).goes_on());
    assert!(c.traveling(), "the run did not walk on after a corpse");
    c.interrupt_travel("walking to a corpse");
    let soon = now + Duration::from_secs(1);
    assert!(c.grow_run_step(soon, &cfg).goes_on());
    assert!(!c.traveling(), "planned again straight away");
    assert!(c.grow_run_step(now + WALK_ON_EVERY, &cfg).goes_on());
    assert!(c.traveling());

    // With no run of its own the leader is followed. The walk after it
    // ends the journey without breaking it off: nothing is to pick that
    // journey up again.
    c.autoplay.growth.run = None;
    c.autoplay.team.mates = vec![leader_off(12.0)];
    assert!(c.autoplay_follow(now, false));
    assert!(!c.traveling());
    assert!(!c.journey_broken_off());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_town_run_between_journeys_is_not_taken_exploring_or_back_to_the_area() {
    // Underground, a corpse on the way ends the run's journey, and
    // exploring ranks above the run: a room chosen then kept the tick
    // for good, so the run never planned its walk again and its clock
    // was never read. Keeping to a hunting area ranks above the run as
    // well, and would have walked the character back to it.
    let renald = Vec2::new(32_587.2, 34_578.3);
    let now = Instant::now();

    // Where the Holtburg Dungeon's portal drops a character.
    let mut c = standing_at(0x01F6_0289, glam::Vec3::new(96.7, -10.0, 0.0));
    c.autoplay.growth.run = Some(run_to(renald, now));
    assert!(!c.traveling());
    assert!(!c.autoplay_explore(now), "went exploring on a run to town");
    assert!(c.follow.is_none());
    // With no run, the same dungeon is explored.
    c.autoplay.growth.run = None;
    assert!(c.autoplay_explore(now));

    // Outdoors by the Holtburg lifestone, a field to hunt 40 m off.
    let mut c = standing_at(0xA9B4_0019, glam::Vec3::new(84.0, 7.1, 94.0));
    let me = c.player.as_ref().unwrap().world_position();
    let (x, y) = (me.x + 40.0, me.y);
    let fight = &mut c.autoplay.config.fight;
    fight.enabled = true;
    fight.area = Some(crate::hunt::HuntArea {
        name: "the field".into(),
        shape: crate::hunt::Shape::Outline {
            points: vec![[x, y], [x + 30.0, y], [x + 30.0, y + 30.0], [x, y + 30.0]],
        },
    });
    c.autoplay.growth.run = Some(run_to(renald, now));
    assert!(
        !c.autoplay_keep_to_area(now),
        "walked back to the area on a run to town"
    );
    assert!(c.follow.is_none());
    // With no run, outside the field, it goes back to it.
    c.autoplay.growth.run = None;
    assert!(c.autoplay_keep_to_area(now));
}

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

#[test]
fn a_run_that_could_not_get_there_is_tried_again_once_its_wait_is_up() {
    // A run that really cannot reach its counter comes home with
    // nothing, so it is futile, and the counter is left alone for a
    // while. Neither is for good: the next run is due once the wait
    // between runs is up, and the counter can be chosen again by then.
    let t0 = Instant::now();
    let renald = spot(Vec2::new(32_587.2, 34_578.3));
    let mut skip = crate::did::Patience::new();
    skip.note(
        renald,
        &crate::did::Did::blocked("that counter was no use"),
        t0,
    );
    for party_restocking in [false, true] {
        let wait = wait_between_runs(party_restocking, true);
        assert!(wait > Duration::ZERO, "straight back to the same counter");
        assert!(wait <= RUN_EVERY);
        assert!(
            !skip.held(&renald, t0 + wait),
            "the counter is still skipped when the next run is due"
        );
    }
    // A party that did buy or sell something may go again at once.
    assert_eq!(wait_between_runs(true, false), Duration::ZERO);
}

#[test]
fn past_the_servers_wall_not_even_a_coin_comes_off() {
    // +Verity: 36462 carried, 7500 capacity. The server hands nothing
    // to a character past three times its capacity, whatever it weighs.
    assert!(past_the_wall(36_462, 7_500));
    // At the wall a coin still fits, and under it so do light things.
    assert!(!past_the_wall(22_500, 7_500));
    assert!(!past_the_wall(13_866, 9_000));
    // Strength not heard yet is no reason to leave every body alone.
    assert!(!past_the_wall(13_866, 0));
}

#[test]
fn a_gem_into_the_hub_puts_the_towns_within_reach() {
    let gem = |g: &ac_world::gems::Gem| ac_world::trip::Gem {
        guid: 1,
        name: g.name.clone(),
        exit: g.xy(),
        exit_cell: g.cell,
        summons: true,
    };
    let named = |n: &str| {
        ac_world::gems::all()
            .iter()
            .find(|g| g.name == n)
            .unwrap_or_else(|| panic!("no {n}"))
    };
    // A gem that lands outdoors is its landing and nothing more.
    let farms = named("Cragstone Farms Portal Gem");
    assert_eq!(gem_ways(&[gem(farms)]).len(), 1);

    // The Town Network gem comes out in the hub, indoors...
    let network = named("Town Network Portal Gem");
    assert!(network.cell & 0xFFFF >= 0x100, "the hub is indoors");
    let ways = gem_ways(&[gem(network)]);
    assert!(ways.len() > 30, "{} ways", ways.len());
    // ...and the counter a mid-level character sells at, the
    // Arcanum Broker outside Cragstone, is a short walk from where
    // one of the hub's portals comes out.
    let broker = ac_world::shops::all()
        .iter()
        .find(|s| s.name == "Arcanum Broker" && s.cell & 0xFFFF_0000 == 0xBB9F_0000)
        .expect("the broker outside Cragstone");
    let (far, via) = ways
        .iter()
        .map(|(w, n)| (broker.xy().distance(*w), n.as_str()))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .unwrap();
    assert!(far <= NEAR_A_WAY_OUT, "{far} m from {via}");
    assert_eq!(via, "Town Network Portal Gem, then Portal to Cragstone");
    // Judged by the hub alone it was out of reach: the fault.
    assert!(broker.xy().distance(network.xy()) > NEAR_A_WAY_OUT);
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

#[test]
fn a_stack_worth_more_than_the_counter_allows_is_sold_in_pieces() {
    // A hundred Pyreal Peas: fifty thousand each and five million
    // the stack, and the server reckons a stack's value as the lot.
    // A counter that will not look at anything over a million
    // refuses all hundred -- so it takes twenty at a time.
    let peas = Salable {
        guid: 1,
        item_type: item_type::SPELL_COMPONENTS,
        value: 5_000_000,
        stack: 100,
    };
    assert_eq!(peas.each(), 50_000);
    assert_eq!(peas.at_once(1_000_000), 20);
    assert!(
        peas.taken_by(item_type::SPELL_COMPONENTS, 0, 1_000_000),
        "in pieces, but taken"
    );

    // A counter with no ceiling takes the lot in one go.
    assert_eq!(peas.at_once(0), 100);

    // One too dear even singly is not taken at all, and saying so
    // is better than splitting a stack down to nothing.
    let jewel = Salable {
        guid: 2,
        item_type: item_type::JEWELRY,
        value: 4_000_000,
        stack: 1,
    };
    assert_eq!(jewel.at_once(1_000_000), 0);
    assert!(!jewel.taken_by(item_type::JEWELRY, 0, 1_000_000));

    // And the counter's floor is about one of them, not the pile:
    // a stack of cheap things is not made sellable by being big.
    let chaff = Salable {
        guid: 3,
        item_type: item_type::MISC,
        value: 10_000,
        stack: 1_000,
    };
    assert_eq!(chaff.each(), 10);
    assert!(
        !chaff.taken_by(item_type::MISC, 100, 0),
        "ten is under the floor"
    );
}

fn need(kind: NeedKind, want: u32) -> Need {
    Need {
        name: "x".into(),
        want,
        have: 0,
        keep: want,
        urgent: true,
        buyable: true,
        from: None,
        kind,
    }
}

#[test]
fn a_want_that_names_its_counter_is_only_filled_there() {
    // Fletching supplies come in levels and elements that one bowyer
    // carries and the next does not, so a player who says where a
    // line comes from means it. The field was editable and saved and
    // read by nothing, so every counter that name-matched would do.
    let mut named = need(NeedKind::Named("Acid Arrowhead".into()), 500);
    named.from = Some("Thimrin Woodsetter".into());
    assert!(named.may_buy_at("Thimrin Woodsetter"));
    assert!(
        named.may_buy_at("thimrin woodsetter"),
        "however it is typed"
    );
    assert!(!named.may_buy_at("Scildith Dyrson the Bowyer"));

    // A line that names nobody is bought wherever it is sold, which
    // is every line a character has unless it says otherwise.
    let anywhere = need(NeedKind::Component(691), 1000);
    assert!(anywhere.may_buy_at("Anyone At All"));
}

#[test]
fn a_long_list_is_cut_short() {
    assert_eq!(a_few(&[]), "nothing");
    assert_eq!(a_few(&["Myrrh"]), "Myrrh");
    assert_eq!(a_few(&["a", "b", "c", "d"]), "a, b, c, d");
    assert_eq!(a_few(&["a", "b", "c", "d", "e"]), "a, b, c, d and 1 more");
}

fn ware(wcid: u32, name: &str, value: u32) -> ac_world::shops::Ware {
    ac_world::shops::Ware {
        wcid,
        name: name.into(),
        item_type: ac_world::item_type::SPELL_COMPONENTS,
        value,
    }
}

fn shop(name: &str, sells: Vec<ac_world::shops::Ware>) -> ac_world::shops::Shop {
    ac_world::shops::Shop {
        wcid: 1,
        name: name.into(),
        cell: 0x0001_0001,
        at: glam::Vec3::ZERO,
        sells,
        buys: ac_world::item_type::GEM,
        min_value: 10,
        max_value: 0,
        buy_rate: 0.5,
        sell_rate: 2.0,
        gate: None,
    }
}

fn salable(guid: u32, item_type: u32, value: u32, stack: u32) -> Salable {
    Salable {
        guid,
        item_type,
        value,
        stack,
    }
}

#[test]
fn a_counter_behind_a_door_is_not_planned_around() {
    use ac_world::shops::Gate;
    let mut hall = shop(
        "Vermilia the Archmage",
        vec![ware(37155, "Mana Scarab", 15_000)],
    );
    hall.gate = Some(Gate::Society(4));
    // A very good archmage, and no use at all to anyone else's
    // society or to nobody's.
    assert!(hall.open_to(4, &[]));
    assert!(!hall.open_to(2, &[]));
    assert!(!hall.open_to(0, &[]));
    let mut chapter = shop("Rossu Morta Quartermaster", vec![]);
    chapter.gate = Some(Gate::Quest("RossuMortaChapterhouse_Flag".into()));
    assert!(!chapter.open_to(4, &[]), "no society opens a quest door");
    assert!(chapter.open_to(0, &["rossumortachapterhouse_flag".to_string()]));
}

#[test]
fn case_folds_without_allocating() {
    assert!(contains_fold("Prismatic Taper", "taper"));
    assert!(contains_fold("PRISMATIC TAPER", "prismatic"));
    assert!(!contains_fold("Lead Scarab", "taper"));
    assert!(!contains_fold("Tap", "taper"));
    assert!(!contains_fold("anything", ""));
}

#[test]
fn the_counter_that_pays_best_wins_when_neither_has_the_order() {
    // Around Cragstone the Scriveners are eighty metres away and pay
    // half; the Arcanum Broker is three hundred metres further on
    // and pays 0.95. Neither stocks what a hunting character came to
    // buy, so the order is a tie -- and the old ranking fell through
    // to distance, walked past the broker every time, and took half
    // price on every sale.
    let look = |takings: u32, stocks: usize| Forecast {
        takings,
        stocks: vec!["something".into(); stocks],
        ..Default::default()
    };
    let near = look(500, 0);
    let far = look(950, 0);
    assert_eq!(
        better_counter(Errand::Buy, (&far, 380.0), (&near, 80.0)),
        std::cmp::Ordering::Less,
        "the broker is worth the extra three hundred metres"
    );

    // But only when the order is a tie: a counter that has what the
    // character came for still beats a richer one that does not.
    let stocked_but_poor = look(0, 2);
    assert_eq!(
        better_counter(Errand::Buy, (&stocked_but_poor, 380.0), (&far, 80.0)),
        std::cmp::Ordering::Less,
        "what it came to buy comes first"
    );
}

#[test]
fn distance_only_settles_what_pay_and_stock_leave_even() {
    let same = Forecast {
        takings: 100,
        ..Default::default()
    };
    assert_eq!(
        better_counter(Errand::Buy, (&same, 50.0), (&same, 900.0)),
        std::cmp::Ordering::Less,
        "all else equal, the nearer one"
    );
}

#[test]
fn a_trip_is_judged_before_it_is_walked() {
    let mut tapers = need(NeedKind::Component(691), 100);
    tapers.name = "Prismatic Taper".into();
    let mut scarabs = need(NeedKind::Component(690), 20);
    scarabs.name = "Lead Scarab".into();
    let needs = [tapers, scarabs];
    let wants: Vec<(&Need, String)> = needs.iter().map(|n| (n, String::new())).collect();

    let mage = shop(
        "Archmage",
        vec![
            ware(691, "Prismatic Taper", 5),
            ware(690, "Lead Scarab", 30),
        ],
    );
    let smith = shop("Blacksmith", vec![ware(20630, "Trade Note", 250)]);

    // Money in hand, everything on the shelf: the whole order in one
    // stop, and the trip is worth walking.
    let rich = forecast(&mage, &wants, 10_000, &[]);
    assert_eq!(rich.bill, 100 * 10 + 20 * 60);
    assert_eq!(rich.cheapest, 10);
    assert!(rich.missing.is_empty());
    assert!(rich.covers_it());
    assert!(rich.worth_going());

    // Enough for some of it is still worth walking: a mage that can
    // afford half its tapers is a mage that can keep casting.
    let thin = forecast(&mage, &wants, 50, &[]);
    assert!(!thin.covers_it());
    assert!(thin.worth_going());

    // Not a copper, and nothing to sell: the trip buys nothing and
    // is not made.
    let broke = forecast(&mage, &wants, 0, &[]);
    assert_eq!(broke.funds(), 0);
    assert!(!broke.worth_going());

    // Unless there is something to sell, which pays for the rest.
    //
    // Three gems, twelve hundred the lot: a stack's `value` is the
    // whole stack's, which is how the server reckons it and what a
    // counter's limits are measured against. Four hundred each, the
    // shop pays half, so six hundred for the three.
    let loot = [salable(9, ac_world::item_type::GEM, 1_200, 3)];
    let selling = forecast(&mage, &wants, 0, &loot);
    assert_eq!(selling.takings, 200 * 3);
    assert_eq!(selling.selling, 1);
    assert!(selling.worth_going());
    // What it will not buy does not count towards the trip.
    let armour = [salable(9, ac_world::item_type::ARMOR, 400, 1)];
    assert_eq!(forecast(&mage, &wants, 0, &armour).takings, 0);
    // Nor does what is beneath its notice -- and "beneath" is
    // about one of them, not the pile.
    let trinket = [salable(9, ac_world::item_type::GEM, 5, 1)];
    assert_eq!(forecast(&mage, &wants, 0, &trinket).takings, 0);

    // A counter that stocks none of it is no help however rich the
    // character is.
    let wrong = forecast(&smith, &wants, 100_000, &[]);
    assert_eq!(wrong.cheapest, 0);
    assert_eq!(wrong.missing.len(), 2);
    assert!(!wrong.covers_it());
    assert!(!wrong.worth_going());
}

#[test]
fn a_named_ground_is_not_judged_on_level() {
    // Hunting a place is about its loot, its money, its trophies.
    // A ground the player named must not be refused because the
    // level table disapproves, and `suits` is the only thing that
    // would refuse it.
    let ground = ac_world::hunting::all()
        .iter()
        .find(|g| g.max_level < 50)
        .expect("a low-level ground");
    // Far too low for a level 275 character by the usual rule...
    assert!(!ground.suits(275, 8));
    // ...but naming it is a landblock id, and looking one up asks
    // nothing about levels.
    assert_eq!(
        ac_world::hunting::at(ground.landblock).map(|g| g.landblock),
        Some(ground.landblock)
    );
}

#[test]
fn the_search_widens_rather_than_crossing_the_world() {
    // The bug: ranking on what a shop stocks alone sent a character
    // twenty-five kilometres to a counter with one more line on the
    // shelf. The rings mean a good enough shop in this town wins.
    let r = vendor_rings(None);
    assert_eq!(r.first().copied(), Some(600.0), "the town first");
    assert!(r.windows(2).all(|w| w[0] < w[1]), "{r:?} does not widen");
    assert_eq!(r.last().copied(), Some(f32::INFINITY), "and then anywhere");
}

#[test]
fn a_capped_search_never_looks_past_the_cap() {
    // The next stop of a run stays in the same town.
    let r = vendor_rings(Some(500.0));
    assert_eq!(r, vec![500.0]);
    assert!(r.iter().all(|x| *x <= 500.0));
    // A cap between rings keeps the ones below it.
    let mid = vendor_rings(Some(1_000.0));
    assert_eq!(mid, vec![600.0, 1_000.0]);
}

#[test]
fn there_is_always_a_last_ring_to_fall_back_on() {
    // Whatever the cap, the search ends somewhere rather than
    // leaving the character with nowhere to go.
    for cap in [1.0f32, 600.0, 3_000.0, 15_000.0, 100_000.0] {
        let r = vendor_rings(Some(cap));
        assert!(!r.is_empty(), "cap {cap}");
        assert_eq!(r.last().copied(), Some(cap));
    }
}

#[test]
fn a_healing_kit_is_not_bought_for_someone_who_cannot_use_one() {
    // Untrained Healing: a kit restores next to nothing, so it is
    // not worth the money or the trip to a vendor.
    assert!(!worth_stocking("Healing Kit", false));
    assert!(!worth_stocking("Excellent Healing Kit", false));
    // Trained, it is worth having again.
    assert!(worth_stocking("Healing Kit", true));
    // Everything else is judged on its own, either way.
    assert!(worth_stocking("Prismatic Taper", false));
    assert!(worth_stocking("Mana Stone", false));
}

#[test]
fn ammunition_is_the_plain_kind() {
    use ac_world::fletching::ammo_type;
    assert!(ammo_stock("Arrow", ammo_type::ARROW));
    assert!(ammo_stock("arrow ", ammo_type::ARROW));
    assert!(!ammo_stock("Fire Arrow", ammo_type::ARROW));
    assert!(!ammo_stock("Arrow", ammo_type::BOLT));
    assert!(ammo_stock("Quarrel", ammo_type::BOLT));
    assert!(ammo_stock("Atlatl Dart", ammo_type::ATLATL));
}

/// The character whose pack these tests are about.
const TIDIER: u32 = 0x5000_0001;

/// A character carrying `stacks` of `(guid, wcid, count, max)`,
/// offline, over the game's archives.
fn carrying(stacks: &[(u32, u32, u32, u32)]) -> Client {
    let mut c = Client::offline(crate::testkit::game_data());
    c.world.player_guid = Some(TIDIER);
    for &(guid, wcid, count, max) in stacks {
        c.world.objects.insert(
            guid,
            ac_world::WorldObject {
                guid,
                weenie_class_id: wcid,
                name: format!("thing {wcid}"),
                stack_size: count,
                max_stack_size: max,
                container: Some(TIDIER),
                parent: Some(TIDIER),
                ..Default::default()
            },
        );
    }
    c
}

/// The server's answer to a whole pour: the source forgotten, the
/// target's new size.
fn poured(c: &mut Client, from: u32, to: u32, now: u32) {
    c.world.objects.remove(&from);
    if let Some(o) = c.world.objects.get_mut(&to) {
        o.stack_size = now;
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn lead_peas_taken_in_two_stacks_are_poured_together_without_a_tick_of_their_own() {
    // The report: peas looted off one corpse after another sit in
    // stacks of their own. Nothing here waits for a quiet moment --
    // there is a fight on -- because the server makes a pack-to-pack
    // merge on the spot.
    let mut c = carrying(&[(1, 8329, 40, 100), (2, 8329, 5, 100)]);
    c.attack_target = Some(0x8000_30F2);
    let t0 = Instant::now();
    c.autoplay_tidy(t0);
    let sent = c.autoplay.pour.clone().expect("a pour went out").0;
    assert_eq!(
        (sent.merge.from, sent.merge.to, sent.merge.amount),
        (2, 1, 5)
    );
    assert_eq!(sent.to_before, 40);
    // Nothing else is asked for while the server has not answered:
    // the counts a second choice would be made from are stale.
    c.autoplay_tidy(t0 + Duration::from_millis(100));
    assert_eq!(
        c.autoplay.pour.as_ref().map(|(p, _)| p.merge.clone()),
        Some(sent.merge.clone())
    );
    // The answer, read off the target's new size.
    poured(&mut c, 2, 1, 45);
    c.autoplay_tidy(t0 + Duration::from_millis(700));
    assert!(c.autoplay.pour.is_none(), "the pour landed");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_pair_the_server_turns_down_does_not_stop_the_rest_being_tidied() {
    // One stubborn pair used to be the only answer ever offered, so
    // it was asked for every 600 ms and nothing else in the pack was
    // ever poured together.
    let mut c = carrying(&[
        (1, 273, 5000, 25000),
        (2, 273, 900, 25000),
        (3, 8329, 40, 100),
        (4, 8329, 5, 100),
    ]);
    let t0 = Instant::now();
    c.autoplay_tidy(t0);
    let first = c.autoplay.pour.clone().expect("a pour went out").0.merge;
    assert_eq!((first.from, first.to), (2, 1));
    // InventoryServerSaveFailed naming the source, with no reason
    // at all, which is half of them.
    c.move_refused
        .insert(2, (0, t0 + Duration::from_millis(50)));
    c.autoplay_tidy(t0 + Duration::from_millis(700));
    assert!(
        !c.move_refused.contains_key(&2),
        "the refusal was this pour's and is spent"
    );
    let next = c.autoplay.pour.clone().expect("the next pair").0.merge;
    assert_eq!((next.from, next.to), (4, 3), "the peas go in instead");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_refusal_naming_the_target_is_read_as_this_pours_answer() {
    // A stack that is stuck, or being traded, is refused under the
    // target's guid. Read only under the source's, these were never
    // seen at all and the same pair was offered every 600 ms.
    let mut c = carrying(&[(1, 8329, 40, 100), (2, 8329, 5, 100)]);
    let t0 = Instant::now();
    c.autoplay_tidy(t0);
    c.move_refused
        .insert(1, (0x29, t0 + Duration::from_millis(50)));
    c.autoplay_tidy(t0 + Duration::from_millis(700));
    assert!(c.autoplay.pour.is_none(), "the pour is over");
    assert!(!c.move_refused.contains_key(&1));
    // And the pair is left alone for a while rather than asked again.
    assert!(c
        .autoplay
        .growth
        .wont_merge
        .held(&(2, 1), t0 + Duration::from_millis(800)));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn stacks_whose_words_disagree_are_never_poured_together() {
    use ac_loot::LootAction;
    let stats = |guid: u32| crate::items::ItemStats {
        guid,
        wcid: 8329,
        name: "Lead Pea".into(),
        ..Default::default()
    };
    // The big stack is meant for a counter, the small one is kept.
    // Pouring the small one in would settle the survivor as kept
    // (the ledger takes the cautious answer), and forty peas the
    // player said to sell would stay in the pack for good: the
    // player's Sell overruled by a tidy. So nothing is poured.
    let mut c = carrying(&[(1, 8329, 40, 100), (2, 8329, 5, 100)]);
    c.autoplay.ledger.remember(&stats(1), LootAction::Sell);
    c.autoplay.ledger.remember(&stats(2), LootAction::Keep);
    let t0 = Instant::now();
    c.autoplay_tidy(t0);
    assert!(
        c.autoplay.pour.is_none(),
        "poured across words: {:?}",
        c.autoplay.pour
    );
    assert_eq!(c.autoplay.ledger.by_guid(1), Some(LootAction::Sell));

    // Nor into a stack nothing was decided about. "Undecided" is
    // not "sell": the guards answer for the survivor, and a Sell
    // poured into it is a Sell lost.
    let mut c = carrying(&[(1, 8329, 40, 100), (2, 8329, 5, 100)]);
    c.autoplay.ledger.remember(&stats(2), LootAction::Sell);
    c.autoplay_tidy(t0);
    assert!(c.autoplay.pour.is_none(), "{:?}", c.autoplay.pour);

    // Two stacks with one word are poured, and the word survives
    // the pour.
    let mut c = carrying(&[(1, 8329, 40, 100), (2, 8329, 5, 100)]);
    c.autoplay.ledger.remember(&stats(1), LootAction::Sell);
    c.autoplay.ledger.remember(&stats(2), LootAction::Sell);
    c.autoplay_tidy(t0);
    let sent = c.autoplay.pour.clone().expect("a pour went out").0;
    assert_eq!((sent.merge.from, sent.merge.to), (2, 1));
    poured(&mut c, 2, 1, 45);
    c.autoplay_tidy(t0 + Duration::from_millis(700));
    assert_eq!(c.autoplay.ledger.by_guid(1), Some(LootAction::Sell));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_pour_with_no_word_at_all_is_given_up_on_and_the_pack_read_afresh() {
    let mut c = carrying(&[(1, 8329, 40, 100), (2, 8329, 5, 100)]);
    let t0 = Instant::now();
    c.autoplay_tidy(t0);
    assert!(c.autoplay.pour.is_some());
    // Nothing comes back: no refusal, no new counts.
    c.autoplay_tidy(t0 + crate::pack::POUR_LOST);
    assert!(c.autoplay.pour.is_none(), "given up as lost");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_quiet_ground_is_read_off_the_world_and_not_off_the_status_line() {
    // The roam clock counted only the frames where the engine had
    // found nothing to do, and put itself back to nothing the moment
    // anything ran. Asking a corpse that would not open is something
    // running, so nine characters queueing at one body restarted it
    // every couple of seconds: in ten minutes it never once reached
    // its minute, and they worked a 40 x 32 m corner of a
    // 197 x 192 m field.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 84.0, 94.0));
    let s = Duration::from_secs;
    let now = Instant::now();
    c.world.stats.level = 20;

    // Nothing about: the clock starts.
    c.autoplay_watch_the_ground(now);
    assert_eq!(c.autoplay.growth.quiet_since, Some(now));

    // And keeps running through everything the character does
    // meanwhile. This is the whole of the fix.
    c.autoplay
        .say(crate::autoplay::Doing::Looting, "opening a corpse");
    c.autoplay_watch_the_ground(now + s(5));
    assert_eq!(
        c.autoplay.growth.quiet_since,
        Some(now),
        "a corpse put the clock back to nothing"
    );

    // Something worth fighting inside the radius stops it.
    let guid = 0x8000_0001;
    let me = c.player.as_ref().unwrap().world_position();
    c.world.objects.insert(
        guid,
        ac_world::WorldObject {
            guid,
            weenie_class_id: 8592,
            name: "Revenant".into(),
            item_type: ac_world::item_type::CREATURE,
            object_desc_flags: ac_world::object_desc_flags::ATTACKABLE,
            health: Some(1.0),
            position: Some(ac_world::object::Position::new_flat(
                holtburg,
                me + glam::Vec3::new(5.0, 0.0, 0.0) - ac_world::landblock_origin(holtburg),
            )),
            ..Default::default()
        },
    );
    c.autoplay_watch_the_ground(now + s(6));
    assert_eq!(c.autoplay.growth.quiet_since, None);

    // A dead one is no fight, and the clock starts again from there.
    c.world.objects.get_mut(&guid).unwrap().health = Some(0.0);
    c.autoplay_watch_the_ground(now + s(7));
    assert_eq!(c.autoplay.growth.quiet_since, Some(now + s(7)));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_body_is_noted_when_it_appears_and_not_when_the_looting_gets_round_to_it() {
    // The bodies used to be noted inside the looting. A character
    // the claim tie-break tells to stand off scores that body
    // nothing, so its loot goal is never run, so it never notes the
    // body -- and a body it never noted is for ever "newly fallen",
    // which is exactly when the tie-break governs. One mate that
    // could not loot locked every body near it away from the other
    // eight for good.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 84.0, 94.0));
    let me = c.player.as_ref().unwrap().world_position();
    let s = Duration::from_secs;
    let now = Instant::now();
    let body = 0x8000_0001;
    c.world.objects.insert(
        body,
        ac_world::WorldObject {
            guid: body,
            name: "Corpse of Drudge Slave".into(),
            object_desc_flags: object_desc_flags::CORPSE,
            position: Some(ac_world::object::Position::new_flat(
                holtburg,
                me - ac_world::landblock_origin(holtburg),
            )),
            ..Default::default()
        },
    );
    // No loot profile, so nothing this character does will ever
    // reach past the first line of the looting step.
    c.autoplay.config.loot.profile = String::new();
    c.autoplay_watch_the_ground(now);
    assert_eq!(
        c.autoplay.corpse_seen,
        vec![(body, now)],
        "a body nobody looted was never noted"
    );
    // Noted once, not restamped every tick: the age is what the
    // tie-break and the rotting order both read.
    c.autoplay_watch_the_ground(now + s(5));
    assert_eq!(c.autoplay.corpse_seen, vec![(body, now)]);
    // Forgotten once emptied, so the list stays the size of what is
    // on the ground.
    c.autoplay.looted.push(body, now + s(5));
    c.autoplay_watch_the_ground(now + s(6));
    assert!(c.autoplay.corpse_seen.is_empty());
}

#[test]
fn a_hunting_area_is_walked_about_sooner_than_a_whole_new_ground_is_chosen() {
    // Moving on inside an area costs only the walk; moving on without
    // one means picking a whole new ground and travelling to it,
    // which is worth being slow about.
    let mut cfg = Growth::default();
    assert_eq!(quiet_before_move(&cfg, false), cfg.idle_before_move);
    assert_eq!(quiet_before_move(&cfg, true), PATROL_AFTER);
    assert!(PATROL_AFTER < cfg.idle_before_move);
    // A player who asked for less still gets less.
    cfg.idle_before_move = 4.0;
    assert_eq!(quiet_before_move(&cfg, true), 4.0);
    assert_eq!(quiet_before_move(&cfg, false), 4.0);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn only_the_one_leading_walks_the_area_looking_for_a_fight() {
    // Nine characters each picking their own corner of an outline
    // scatter the party across two hundred metres instead of moving
    // it. The followers keep to their leader and it takes them.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let cfg = Growth::default();
    c.world.stats.level = 20;
    c.autoplay.config.fight.area = Some(crate::hunt::HuntArea {
        name: "test".into(),
        shape: crate::hunt::Shape::Outline {
            points: vec![
                [me.x - 40.0, me.y - 5.0],
                [me.x + 40.0, me.y - 5.0],
                [me.x + 40.0, me.y + 60.0],
                [me.x - 40.0, me.y + 60.0],
            ],
        },
    });
    let team = &mut c.autoplay.config.team;
    team.enabled = true;
    team.follow = true;
    team.lead = false;
    c.autoplay.team.mates = vec![crate::autoplay::Mate {
        name: "Leader".into(),
        leader: true,
        leads: true,
        world: me,
        cell: holtburg,
        ..Default::default()
    }];
    // Quiet for long enough that the patrol would go.
    c.autoplay.growth.quiet_since = Some(now - Duration::from_secs(60));
    assert!(!c.grow_hunt(now, &cfg), "a follower wandered off");
    assert!(!c.traveling());

    // The one leading walks it.
    c.autoplay.team.mates.clear();
    c.autoplay.team.leader = true;
    assert!(c.grow_hunt(now, &cfg), "the leader stayed put");
    assert!(c.traveling());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_follower_with_no_hunting_area_does_not_go_looking_for_a_ground_of_its_own() {
    // The guard that keeps a follower from walking an area of its
    // own covered the roam as well, but not the tail below it: a
    // party with no area configured had its followers fail the roam
    // on the first quiet minute and fall straight through to picking
    // a landblock and travelling to it -- which is the scattering
    // the guard was added to stop, only sooner than before.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let cfg = Growth::default();
    c.world.stats.level = 20;
    let team = &mut c.autoplay.config.team;
    team.enabled = true;
    team.follow = true;
    team.lead = false;
    // A leader with no ground of its own to take, so nothing below
    // the roam can be following anybody.
    c.autoplay.team.mates = vec![crate::autoplay::Mate {
        name: "Leader".into(),
        leader: true,
        leads: true,
        world: me,
        cell: holtburg,
        ..Default::default()
    }];
    // Standing on the ground it is hunting, quiet long enough to
    // move on, and past its roams so the roam itself is refused.
    c.autoplay.growth.hunting_at = Some(holtburg >> 16);
    c.autoplay.growth.roams = ROAMS;
    c.autoplay.growth.quiet_since = Some(now - Duration::from_secs(600));
    assert!(!c.grow_hunt(now, &cfg), "a follower went hunting alone");
    assert!(!c.traveling());
    assert_eq!(
        c.autoplay.growth.bound, None,
        "bound for a ground of its own"
    );

    // The one leading still moves the party on.
    c.autoplay.team.mates.clear();
    c.autoplay.team.leader = true;
    assert!(c.grow_hunt(now, &cfg), "the leader stayed put");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_walk_to_a_ground_broken_off_by_a_corpse_is_walked_on() {
    // A body on the road -- a fellow's kill, shared -- took the
    // character off its walk to the ground, and walking to a corpse
    // ends a journey. Once the body was looted the hunting step found
    // no journey under way, took that for a walk that could not get
    // there, put the ground on the skip list and set off for another.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let cfg = Growth::default();
    let at = Vec2::new(me.x + 250.0, me.y);
    let lb = (((at.x / 192.0) as u32) << 8) | (at.y / 192.0) as u32;
    assert!(c.grow_travel(at, now), "no way to the ground");
    c.autoplay.growth.bound = Some((lb, at, "Drudge".into()));
    c.autoplay.growth.bound_since = Some(now);

    c.interrupt_travel("walking to a corpse");
    assert!(c.grow_hunt(now, &cfg), "gave the walk up after a corpse");
    assert!(c.traveling(), "and did not walk on");
    assert!(c.autoplay.growth.bound.is_some());
    assert!(c.autoplay.growth.skip.is_empty(), "the ground was skipped");

    // Broken off again as soon as it is planned, it waits a moment
    // before planning the walk once more.
    c.interrupt_travel("walking to a corpse");
    assert!(c.grow_hunt(now + Duration::from_secs(1), &cfg));
    assert!(!c.traveling(), "planned again straight away");
    assert!(c.grow_hunt(now + WALK_ON_EVERY, &cfg));
    assert!(c.traveling());

    // A walk that ended by itself short of the ground still could not
    // get there.
    c.cancel_travel();
    assert!(!c.grow_hunt(now + WALK_ON_EVERY, &cfg));
    assert_eq!(c.autoplay.growth.bound, None);
    assert!(c.autoplay.growth.skip.iter().any(|(g, _)| *g == lb));
}

/// A Revenant called `name` standing `metres` east of the character:
/// a real fight at level 20, not a critter walked past for its own
/// sake.
fn standing_by(c: &mut Client, guid: u32, name: &str, metres: f32) -> ac_world::WorldObject {
    let pl = c.player.as_ref().unwrap();
    let (cell, me) = (pl.cell, pl.world_position());
    let o = ac_world::WorldObject {
        guid,
        weenie_class_id: 8592,
        name: name.into(),
        item_type: ac_world::item_type::CREATURE,
        object_desc_flags: ac_world::object_desc_flags::ATTACKABLE,
        health: Some(1.0),
        position: Some(ac_world::object::Position::new_flat(
            cell,
            me + glam::Vec3::new(metres, 0.0, 0.0) - ac_world::landblock_origin(cell),
        )),
        ..Default::default()
    };
    c.world.objects.insert(guid, o.clone());
    o
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn what_stands_on_the_road_is_walked_past_and_what_swings_at_us_is_not() {
    // "Traveling to the hunting ground shouldn't have much fighting,
    // more ignoring the monsters on the way so you can get to the
    // hunting ground" -- the player, and the reason for the rule.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let fight = c.autoplay.config.fight.clone();
    let it = standing_by(&mut c, 0x8000_0001, "Revenant", 5.0);

    // Standing on its ground with nowhere to be: a fight.
    assert!(!c.passing_by(&it, &fight));
    assert!(c.would_fight(&it, &fight, false, now));

    // Bound for a hunting ground and walking there: walked past, and
    // the fight rules have nothing to pick.
    let ground = (0xA9B2, Vec2::new(32_580.0, 34_570.0), "Drudge".to_string());
    assert!(c.grow_travel(Vec2::new(me.x + 250.0, me.y), now));
    c.autoplay.growth.bound = Some(ground.clone());
    assert!(c.passing_by(&it, &fight));
    assert!(!c.would_fight(&it, &fight, false, now));

    // Unless it swings: the road does not get to decide that.
    c.autoplay.attacked_by("Revenant", now);
    assert!(!c.passing_by(&it, &fight));
    assert!(c.would_fight(&it, &fight, false, now));
    // Which says nothing about the one standing next to it.
    let other = standing_by(&mut c, 0x8000_0002, "Drudge Skulker", 6.0);
    assert!(c.passing_by(&other, &fight));
    c.autoplay.last_hit_us = None;
    c.autoplay.hit_by.clear();

    // Named in "only these", it is still the road. The list says which
    // kind to hunt at the far end, and every creature the fight could
    // pick already matches it, so as an override it switched the rule
    // off for anyone who kept one.
    let only = crate::autoplay::Fight {
        only: vec!["revenant".into()],
        ..fight.clone()
    };
    assert!(c.passing_by(&it, &only));
    assert!(!c.would_fight(&it, &only, false, now));

    // A creature of ours on it is no reason to stop either. A summoned
    // creature picks its own fights, the nearest monster it can see,
    // and following its lead stopped the character for each in turn.
    c.world.objects.insert(
        0x8000_0003,
        ac_world::WorldObject {
            guid: 0x8000_0003,
            name: "Fire Elemental".into(),
            item_type: ac_world::item_type::CREATURE,
            pet_owner: 0x5000_0001,
            walked_at: Some(it.guid),
            ..Default::default()
        },
    );
    assert!(c.passing_by(&it, &fight), "its pet went for it");
    assert!(!c.would_fight(&it, &fight, false, now));
    c.world.objects.remove(&0x8000_0003);

    // With the setting off, everything on the road is fought again.
    let all = crate::autoplay::Fight {
        walk_past_on_the_way: false,
        ..fight.clone()
    };
    assert!(!c.passing_by(&it, &all));
    assert!(c.would_fight(&it, &all, false, now));

    // A town run is the same errand: the counters are the point of
    // it, not whatever stands between here and them.
    c.autoplay.growth.bound = None;
    c.autoplay.growth.run = Some(run_to(Vec2::new(32_500.0, 34_500.0), now));
    assert!(c.passing_by(&it, &fight));

    // The errand ending does not end the walking past: the journey
    // still under way is a road whoever planned it (see
    // `a_road_is_a_road_whoever_planned_it`). The journey ending does.
    c.autoplay.growth.run = None;
    assert!(c.passing_by(&it, &fight), "the journey is still under way");
    c.cancel_travel();
    assert!(!c.passing_by(&it, &fight), "nowhere to be again");
    assert!(c.would_fight(&it, &fight, false, now));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn an_errand_is_on_its_way_only_while_its_walk_is() {
    // The errand outlives its walk. `bound` is let go only when the
    // hunting step next looks, and a party restocking does not call
    // it: a leader home from town stood on its own ground walking past
    // everything that had not swung at it until the slowest of the
    // party had shopped. And a body at its feet kept the grow step
    // from running at all, so the body beat the monster beside it.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let fight = c.autoplay.config.fight.clone();
    let it = standing_by(&mut c, 0x8000_0001, "Revenant", 5.0);
    let at = Vec2::new(me.x + 250.0, me.y);
    let lb = (((at.x / 192.0) as u32) << 8) | (at.y / 192.0) as u32;

    // Bound, and walking there.
    assert!(c.grow_travel(at, now), "no way to the ground");
    c.autoplay.growth.bound = Some((lb, at, "Drudge".into()));
    assert!(c.on_its_way());
    assert!(!c.would_fight(&it, &fight, false, now));

    // A body on the road breaks the walk off, and it is still the walk.
    c.interrupt_travel("walking to a corpse");
    assert!(c.on_its_way(), "a walk broken off is still the walk");
    assert!(!c.would_fight(&it, &fight, false, now));

    // Arrived: fighting again that tick, whatever `bound` still says.
    assert!(c.grow_travel(at, now));
    c.end_trip();
    assert!(c.autoplay.growth.bound.is_some(), "not noticed yet");
    assert!(!c.on_its_way(), "arrived, and still walking past");
    assert!(c.would_fight(&it, &fight, false, now));

    // Cancelled -- the player took the keys -- is no walk either.
    assert!(c.grow_travel(at, now));
    c.cancel_travel();
    assert!(!c.on_its_way());

    // Stepping out of a shop first is the start of the walk.
    c.autoplay.growth.after_out = Some(at);
    assert!(c.on_its_way());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_party_on_the_road_walks_past_together_and_stops_together() {
    // The leader, bound for a new ground, walked past a Drudge. Its
    // followers keep up through the follow step, which sets no errand,
    // so they stopped and fought it; the leader walked on, they were
    // fetched after it and turned on the Drudge again each time they
    // closed, and the leader never helped.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let fight = c.autoplay.config.fight.clone();
    let drudge = standing_by(&mut c, 0x8000_0001, "Drudge Skulker", 5.0);
    let other = standing_by(&mut c, 0x8000_0002, "Drudge Slinker", 6.0);
    let team = &mut c.autoplay.config.team;
    team.enabled = true;
    team.follow = true;
    team.lead = false;
    let leader = |on_its_way: bool, target: Option<u32>| crate::autoplay::Mate {
        name: "Leader".into(),
        leader: true,
        leads: true,
        world: me,
        cell: holtburg,
        on_its_way,
        target,
        ..Default::default()
    };

    // A leader going nowhere: its follower fights what is about.
    c.autoplay.team.mates = vec![leader(false, None)];
    assert!(!c.on_its_way());
    assert!(c.would_fight(&drudge, &fight, false, now));

    // A leader on its way: so is the follower keeping up with it.
    c.autoplay.team.mates = vec![leader(true, None)];
    assert!(c.on_its_way(), "a follower is on its leader's way");
    assert!(!c.would_fight(&drudge, &fight, false, now));
    assert!(!c.joins_the_team_on(drudge.guid, &fight));

    // Something attacks the leader on the road and it turns to fight:
    // the follower turns with it and joins on it, but not on the one
    // standing by.
    c.autoplay.team.mates = vec![leader(true, Some(drudge.guid))];
    assert!(c.would_fight(&drudge, &fight, false, now));
    assert!(c.joins_the_team_on(drudge.guid, &fight));
    assert!(!c.would_fight(&other, &fight, false, now));
    assert!(!c.joins_the_team_on(other.guid, &fight));

    // On a run of its own while the party back at the ground fights:
    // that fight is not the road's, and neither focus fire nor the
    // debuffer turns the character round for it.
    c.autoplay.config.team.follow = false;
    let counter = Vec2::new(me.x + 250.0, me.y);
    assert!(c.grow_travel(counter, now));
    c.autoplay.growth.run = Some(run_to(counter, now));
    c.autoplay.team.mates = vec![leader(false, Some(drudge.guid))];
    assert!(c.on_its_way());
    assert!(
        !c.joins_the_team_on(drudge.guid, &fight),
        "turned back for the party"
    );
    // Until the Drudge attacks the character itself.
    c.autoplay.attacked_by("Drudge Skulker", Instant::now());
    assert!(c.joins_the_team_on(drudge.guid, &fight));
}

#[test]
fn a_road_is_under_way_unless_it_is_a_walk_about_the_ground() {
    // No journey, nothing remembered: not on a road.
    assert!(!road_under_way(None, None));
    // A journey to somewhere else, or one put down for a fight.
    assert!(road_under_way(Some(false), None));
    assert!(road_under_way(None, Some(false)));
    // A roam, a patrol: about the ground, so never a road.
    assert!(!road_under_way(Some(true), None));
    assert!(!road_under_way(None, Some(true)));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_road_is_a_road_whoever_planned_it() {
    // The walk past the road read only the growth rules' errands, so
    // a journey a script asked for was not "on its way": a party sent
    // down the Singularity Caul by script fought everything between
    // the drop and the far end, fourteen fights on a walk of forty
    // seconds.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let fight = c.autoplay.config.fight.clone();
    let it = standing_by(&mut c, 0x8000_0001, "Revenant", 5.0);
    let at = Vec2::new(me.x + 250.0, me.y);
    assert!(c.autoplay.growth.bound.is_none() && c.autoplay.growth.run.is_none());

    // A script's journey: on its way, and walking past.
    assert!(c.travel_to(at), "no way there");
    assert!(c.on_its_way());
    assert!(!c.would_fight(&it, &fight, false, now));

    // Put down for a fight that came to it: still the road, and the
    // road again once the fight is over.
    c.remember_journey();
    c.interrupt_travel("attacking");
    assert!(!c.traveling());
    assert!(
        c.on_its_way(),
        "a road put down for a fight is still the road"
    );
    assert!(c.autoplay_resume_journey());
    assert!(c.traveling() && c.on_its_way());

    // Cancelled -- the player took the keys -- is no road.
    c.cancel_travel();
    assert!(!c.on_its_way());
    assert!(c.would_fight(&it, &fight, false, now));

    // A roam about the ground is not a road, and is not one after a
    // fight has put it down either: what stands about the ground is
    // what the character came for.
    assert!(c.travel_about(at));
    assert!(!c.on_its_way(), "a roam walked past the ground");
    assert!(c.would_fight(&it, &fight, false, now));
    c.remember_journey();
    c.interrupt_travel("attacking");
    assert!(!c.on_its_way());
    assert!(c.autoplay_resume_journey());
    assert!(c.traveling());
    assert!(!c.on_its_way(), "a roam picked up again became a road");

    // A road put down for a body on it is remembered like one put
    // down for a fight, since nothing else would bring it back.
    c.cancel_travel();
    assert!(c.travel_to(at));
    c.autoplay.resume_trip = None;
    let spot = glam::Vec3::new(me.x + 30.0, me.y, me.z);
    c.walk_to_corpse(0x8000_0002, "Corpse of Drudge", spot, 30.0, now);
    assert!(!c.traveling());
    assert_eq!(c.autoplay.resume_trip, Some(at));
    assert!(c.on_its_way(), "a road put down for a body was lost");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn an_errand_whose_setting_is_turned_off_is_let_go() {
    // Nothing carries a run on with town runs turned off, nor a walk
    // to a ground with grounds off, so neither was ever let go. The
    // errand read as under way for the rest of the session.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let now = Instant::now();
    let to = Vec2::new(32_500.0, 34_500.0);
    let growth = &mut c.autoplay.config.growth;
    growth.auto_xp = false;
    growth.town_runs = false;
    growth.hunt_grounds = false;
    c.autoplay.growth.run = Some(run_to(to, now));
    c.autoplay.growth.bound = Some((0xA9B2, to, "Drudge".into()));
    c.autoplay_grow(now);
    assert!(!c.autoplay.growth.town_run_under_way(), "the run was kept");
    assert_eq!(c.autoplay.growth.bound, None, "the walk was kept");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_run_asked_for_from_the_panel_chooses_a_counter_and_sets_off() {
    // The panel's Step and Run once put the shopping rules straight
    // to the character where it stood. With no window open the
    // rules answered "no counter" and called the trip done, so the
    // buttons did nothing -- and having called it done, went on
    // doing nothing once a window was opened. Now they start the
    // run autoplay would start: a counter is chosen and the walk
    // to it begins, with autoplay off and none of its throttles.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let now = Instant::now();
    assert!(!c.autoplay.config.enabled);
    assert_eq!(c.town_run_driver(), None);
    c.town_run_by_hand(now).expect("no run was started");
    assert_eq!(c.town_run_driver(), Some(Driver::Hand));
    assert!(c.traveling(), "chose a counter and did not set off");
    let v = c.town_run_view(now).expect("no run to show");
    assert_eq!(v.phase, "Going");
    assert_eq!(v.driver, Driver::Hand);
    assert_eq!(v.next, None, "nothing is decided on the walk");
    // The walk is a wait, and a Step waits it out.
    assert_eq!(c.town_run_step_by_hand(now), Turn::Waited);
    assert!(c.traveling());
    // Pressed again, the run under way is the one shown; a second
    // is not started.
    c.town_run_by_hand(now).unwrap();
    assert_eq!(c.town_run_view(now).unwrap().vendor, v.vendor);
    assert_eq!(c.autoplay.growth.run.as_ref().unwrap().since, now);

    // Autoplay turned on meanwhile leaves the run to the panel --
    // it neither steps it nor, with town runs off, lets it go --
    // but counts the character as busy with it.
    c.autoplay.config.enabled = true;
    c.autoplay.config.growth.town_runs = false;
    assert_eq!(c.town_run_driver(), Some(Driver::Hand));
    c.autoplay_grow(now);
    assert!(
        c.autoplay.growth.town_run_under_way(),
        "let go for town runs being off"
    );
    c.autoplay.config.growth.town_runs = true;
    assert!(c.autoplay_grow(now), "the run did not keep the tick");
    assert_eq!(c.town_run_driver(), Some(Driver::Hand));
    assert!(c.traveling(), "autoplay stepped the panel's run");

    // Stop leaves the character as it stands: no run, no walk,
    // nothing to walk up to, no window.
    c.town_run_stop(now);
    assert_eq!(c.town_run_driver(), None);
    assert!(!c.traveling());
    assert!(c.follow.is_none());
    assert!(c.world.open_vendor.is_none());
    assert_eq!(c.autoplay.growth.shop.phase, ac_vendor::Phase::Walking);
    // And autoplay, now running town runs, does not set straight
    // off on one of its own.
    assert!(!c.autoplay_grow(now), "autoplay started a run at once");
    assert_eq!(c.town_run_driver(), None);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_run_started_under_autoplay_is_autoplays_to_step() {
    // With autoplay on and running town runs, Run from the panel
    // starts the run now, and autoplay's tick takes it from there:
    // the panel shows it and does not step it too.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    c.autoplay.config.enabled = true;
    let now = Instant::now();
    c.town_run_by_hand(now).expect("no run was started");
    assert_eq!(c.town_run_driver(), Some(Driver::Autoplay));
    assert!(c.autoplay_grow(now));
    assert!(c.traveling());
    // Until the panel steps it itself, which makes it the panel's.
    assert_eq!(c.town_run_step_by_hand(now), Turn::Waited);
    assert_eq!(c.town_run_driver(), Some(Driver::Hand));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_shopping_rules_start_every_counter_fresh() {
    // The rules remember what a trip offered, what was refused and
    // whether it is done, and were made afresh only when a trip
    // closed its counter. Asked once with no window open they call
    // the trip done, and a run dropped part-way leaves them where
    // they were; either way the next counter opened was closed at
    // once, with nothing sold. Now they are started fresh as each
    // counter's selling begins.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.player_guid = Some(0x5000_0001);
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let cfg = c.autoplay.config.growth.clone();
    // Done, and with a count from some earlier counter.
    let snap = c.vendor_snapshot(&cfg);
    c.autoplay.growth.shop.step(&snap, now);
    assert_eq!(c.autoplay.growth.shop.phase, ac_vendor::Phase::Done);
    c.autoplay.growth.shop.sold = 7;

    // A run at the counter with the pack looked over. Nothing goes
    // out on the way from there to selling, so a Step goes on
    // through it to the first act of the selling.
    let mut run = run_to(Vec2::new(me.x, me.y), now);
    run.phase = Phase::Appraising;
    run.since = now - SETTLE * 3;
    c.autoplay.growth.run = Some(run);
    assert_eq!(c.grow_run_step(now, &cfg), Turn::Waited);
    assert!(matches!(
        c.autoplay.growth.run.as_ref().unwrap().phase,
        Phase::Selling { .. }
    ));
    assert_eq!(
        c.autoplay.growth.shop.phase,
        ac_vendor::Phase::Walking,
        "the rules were not started fresh"
    );
    assert_eq!(c.autoplay.growth.shop.sold, 0);
    assert_eq!(c.town_run_view(now).unwrap().sold, 0);

    // Waiting for a window is a wait; asking again after the
    // counter's time is up is an act.
    let mut run = run_to(Vec2::new(me.x, me.y), now);
    run.phase = Phase::Opening {
        guid: 0x8000_0001,
        tries: 1,
        busy: None,
        asked_over: 0,
    };
    c.autoplay.growth.run = Some(run);
    assert_eq!(c.grow_run_step(now, &cfg), Turn::Waited);
    assert_eq!(c.town_run_view(now).unwrap().phase, "Opening");
    let later = now + VENDOR_OPEN_TIMEOUT + Duration::from_secs(1);
    assert_eq!(c.grow_run_step(later, &cfg), Turn::Acted);
    assert!(matches!(
        c.autoplay.growth.run.as_ref().unwrap().phase,
        Phase::Opening { tries: 2, .. }
    ));
}

/// A vendor's window as the server sends it, with nothing on the
/// shelf.
fn window_of(vendor: u32) -> ac_world::object::ApproachVendor {
    ac_world::object::ApproachVendor {
        vendor,
        item_types: 0,
        min_value: 0,
        max_value: 0,
        magical: false,
        buy_rate: 1.0,
        sell_rate: 1.0,
        alt_currency: 0,
        alt_amount: 0,
        alt_name: String::new(),
        items: Vec::new(),
    }
}

/// A vendor standing `off` from the character, in view.
fn vendor_beside(c: &mut Client, guid: u32, name: &str, off: glam::Vec3) {
    let holtburg = 0xA9B4_0019;
    let me = c.player.as_ref().unwrap().world_position();
    c.world.objects.insert(
        guid,
        ac_world::WorldObject {
            guid,
            name: name.into(),
            item_type: ac_world::item_type::CREATURE,
            object_desc_flags: object_desc_flags::VENDOR,
            position: Some(ac_world::object::Position::new_flat(
                holtburg,
                me + off - ac_world::landblock_origin(holtburg),
            )),
            ..Default::default()
        },
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_hand_run_left_held_is_autoplays_again_once_it_runs_town_runs() {
    // Step once with autoplay off, close the panel, turn autoplay
    // on: the run was the panel's for good, autoplay claimed every
    // tick for it and stepped nothing, and the character stood on
    // the road with a blank status line. A run nothing has stepped
    // for a moment is autoplay's to carry on, when it runs town
    // runs.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let now = Instant::now();
    c.town_run_by_hand(now).expect("no run was started");
    assert_eq!(c.town_run_driver(), Some(Driver::Hand));
    c.autoplay.config.enabled = true;
    c.autoplay.config.growth.town_runs = true;
    // Just pressed: still the panel's, and not stepped by autoplay.
    assert!(c.autoplay_grow(now));
    assert_eq!(c.town_run_driver(), Some(Driver::Hand));
    // Left alone: autoplay's, and stepped on.
    let later = now + HAND_HOLD;
    assert!(c.autoplay_grow(later));
    assert_eq!(c.town_run_driver(), Some(Driver::Autoplay));
    assert!(c.autoplay.growth.town_run_under_way());
    assert!(c.traveling());
    // The panel stepping it again takes it back, for as long as it
    // keeps stepping.
    assert_eq!(c.town_run_step_by_hand(later), Turn::Waited);
    assert_eq!(c.town_run_driver(), Some(Driver::Hand));
    assert!(c.autoplay_grow(later));
    assert_eq!(c.town_run_driver(), Some(Driver::Hand));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_hand_run_with_town_runs_off_keeps_the_hunting_off_the_counter() {
    // Autoplay on with town runs off is the panel's to drive, and
    // the run is kept for it -- but with town runs off the run was
    // never looked at by autoplay's tick, so nothing claimed the
    // tick for it and the hunting, finding nothing in sight, walked
    // the character off the open window to look about the ground.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    c.autoplay.config.enabled = true;
    c.autoplay.config.growth.town_runs = false;
    c.autoplay.config.growth.hunt_grounds = true;
    c.autoplay.config.growth.tactic = ac_world::hunting::Tactic::Patrol;
    // On its hunting ground, quiet long enough to move on.
    c.autoplay.growth.hunting_at = Some(holtburg >> 16);
    c.autoplay.growth.quiet_since = Some(now - Duration::from_secs(600));
    // At the counter, waiting on its window, held by the panel.
    let mut run = run_to(Vec2::new(me.x, me.y), now);
    run.phase = Phase::Opening {
        guid: 0x8000_0001,
        tries: 1,
        busy: None,
        asked_over: 0,
    };
    c.autoplay.growth.run = Some(run);
    c.autoplay.growth.by_hand = true;
    assert!(c.autoplay_grow(now), "the run did not keep the tick");
    assert!(!c.traveling(), "the hunting walked off the counter");
    assert_eq!(c.autoplay.growth.bound, None);
    assert!(c.autoplay.growth.town_run_under_way());
    assert!(
        c.autoplay.status.contains("held by the vendoring panel"),
        "nothing said why the character stands still: {:?}",
        c.autoplay.status
    );
    // Without the run the same tick goes looking about the ground,
    // which is what the run was keeping it from.
    c.autoplay.growth.by_hand = false;
    assert!(c.autoplay_grow(now));
    assert!(
        !c.autoplay.growth.town_run_under_way(),
        "kept with town runs off"
    );
    assert!(c.traveling(), "the hunting did not move");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn run_pressed_at_a_counter_the_player_opened_sells_there() {
    // The panel shows what the rules would do at a window the
    // player opened by hand; Run then chose a counter of its own
    // and walked away from the one it had just been showing. At an
    // open window within reach the run is made there, and its
    // first turn is the appraising.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let now = Instant::now();
    let rakk = 0x8000_0002;
    vendor_beside(
        &mut c,
        rakk,
        "Rakk the Peddler",
        glam::Vec3::new(1.0, 0.0, 0.0),
    );
    c.world.open_vendor = Some(window_of(rakk));
    c.town_run_by_hand(now).expect("no run was started");
    let v = c.town_run_view(now).expect("no run to show");
    assert_eq!(v.vendor, "Rakk the Peddler");
    assert_eq!(v.phase, "Opening");
    assert!(!c.traveling(), "walked off to another counter");
    assert_eq!(c.town_run_step_by_hand(now), Turn::Acted);
    assert_eq!(c.town_run_view(now).unwrap().phase, "Appraising");
    assert!(c.world.open_vendor.is_some(), "the window was closed");

    // A window left open from across the town is not a counter at
    // hand: the run chooses one and sets off, as it always did.
    c.town_run_stop(now);
    vendor_beside(
        &mut c,
        rakk,
        "Rakk the Peddler",
        glam::Vec3::new(60.0, 0.0, 0.0),
    );
    c.world.open_vendor = Some(window_of(rakk));
    c.town_run_by_hand(now).expect("no run was started");
    assert_eq!(c.town_run_view(now).unwrap().phase, "Going");
    assert!(c.traveling());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_hand_run_is_refused_with_no_slot_for_the_money() {
    // Autoplay's own refusal lived only in its tick, so the panel's
    // Run walked a pack with no free slot to town, sold nothing
    // there, and came home futile -- which held autoplay's next run
    // back for the longer while.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    let me = 0x5000_0001;
    c.world.player_guid = Some(me);
    c.world.objects.insert(
        me,
        ac_world::WorldObject {
            guid: me,
            name: "Verity".into(),
            is_player: true,
            items_capacity: 1,
            ..Default::default()
        },
    );
    c.world.objects.insert(
        0x8000_0003,
        ac_world::WorldObject {
            guid: 0x8000_0003,
            name: "Dagger".into(),
            value: 50,
            container: Some(me),
            ..Default::default()
        },
    );
    assert_eq!(c.free_space(), 0);
    let now = Instant::now();
    assert_eq!(
        c.town_run_by_hand(now),
        Err("no free slot for a counter's money".into())
    );
    assert_eq!(c.town_run_driver(), None);
    assert!(!c.traveling());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_window_that_opens_after_the_run_stopped_is_closed() {
    // Stop pressed while the counter was being asked for its window
    // left the window to arrive afterwards, and nothing closed it:
    // the tidying refused every pour for "a counter is open" and
    // the panel showed a counter open with nobody at it.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.player_guid = Some(0x5000_0001);
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let rakk = 0x8000_0002;
    let asking = |c: &mut Client| {
        let mut run = run_to(Vec2::new(me.x, me.y), now);
        run.phase = Phase::Opening {
            guid: rakk,
            tries: 1,
            busy: None,
            asked_over: 0,
        };
        c.autoplay.growth.run = Some(run);
    };
    asking(&mut c);
    c.town_run_stop(now);
    assert!(c.world.open_vendor.is_none());
    // The window comes a moment later, and goes.
    c.world.open_vendor = Some(window_of(rakk));
    c.autoplay_close_unwanted_window(now + Duration::from_secs(1));
    assert!(c.world.open_vendor.is_none(), "the window was left open");
    // Another counter's window is not the one that was asked for.
    asking(&mut c);
    c.town_run_stop(now);
    c.world.open_vendor = Some(window_of(0x8000_0009));
    c.autoplay_close_unwanted_window(now + Duration::from_secs(1));
    assert!(
        c.world.open_vendor.is_some(),
        "somebody else's window was closed"
    );
    c.world.open_vendor = None;
    // Long after the run would have given up on it, a window for
    // that counter is the player's own doing.
    asking(&mut c);
    c.town_run_stop(now);
    let late = now + VENDOR_OPEN_TIMEOUT + Duration::from_secs(1);
    c.autoplay_close_unwanted_window(late);
    c.world.open_vendor = Some(window_of(rakk));
    c.autoplay_close_unwanted_window(late);
    assert!(
        c.world.open_vendor.is_some(),
        "the player's window was closed"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_hand_runs_status_stays_with_autoplay_off() {
    // With autoplay off its tick cleared the status every frame,
    // and the run's next turn said it again: a log line, an event
    // and a bus post a frame for the length of the walk. The run
    // the panel is driving keeps its line.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.player_guid = Some(0x5000_0001);
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    c.autoplay.config.enabled = false;
    c.autoplay.growth.run = Some(run_to(Vec2::new(me.x, me.y), now));
    c.autoplay.growth.by_hand = true;
    c.autoplay.say(Doing::Shopping, "going to the counter");
    let said = c.autoplay.announced.len();
    c.tick_autoplay(now);
    assert_eq!(
        c.autoplay.status, "going to the counter",
        "the status was cleared"
    );
    c.autoplay.say(Doing::Shopping, "going to the counter");
    assert_eq!(c.autoplay.announced.len(), said, "said again");
    // A run nobody is driving does not keep autoplay's line up.
    c.autoplay.growth.by_hand = false;
    c.tick_autoplay(now);
    assert!(c.autoplay.status.is_empty());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn walking_the_hunting_ground_is_not_being_on_the_way_to_it() {
    // The trap this rule had to be kept out of. A patrol of the
    // hunting area, and the roam around a ground, both travel --
    // `traveling()` is true for a character doing the very thing it
    // came here for. Keyed on that, a character would have stood in
    // its own hunting ground refusing to fight. Both paths set off
    // and return before `bound` is ever set, and that is the whole
    // of the difference.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let cfg = Growth::default();
    c.autoplay.config.fight.area = Some(crate::hunt::HuntArea {
        name: "test".into(),
        shape: crate::hunt::Shape::Outline {
            points: vec![
                [me.x - 40.0, me.y - 5.0],
                [me.x + 40.0, me.y - 5.0],
                [me.x + 40.0, me.y + 60.0],
                [me.x - 40.0, me.y + 60.0],
            ],
        },
    });
    let fight = c.autoplay.config.fight.clone();
    let it = standing_by(&mut c, 0x8000_0001, "Revenant", 5.0);

    // Quiet long enough to walk the area, and it walks it.
    c.autoplay.growth.quiet_since = Some(now - Duration::from_secs(600));
    assert!(c.grow_hunt(now, &cfg), "the patrol stayed put");
    assert!(c.traveling(), "the patrol is a journey like any other");
    assert_eq!(c.autoplay.growth.bound, None, "the patrol is not an errand");
    assert!(!c.on_its_way());

    // So the creature it walked up to is still a fight.
    assert!(!c.passing_by(&it, &fight));
    assert!(c.would_fight(&it, &fight, false, now));
}

#[test]
fn growth_config_has_defaults_and_round_trips() {
    let g: Growth = serde_json::from_str("{}").unwrap();
    assert_eq!(g, Growth::default());
    assert!(g.auto_xp && g.hunt_grounds && g.town_runs);
    let text = serde_json::to_string(&g).unwrap();
    let back: Growth = serde_json::from_str(&text).unwrap();
    assert_eq!(back, g);
    let partial: Growth = serde_json::from_str(r#"{"auto_xp":false,"level_margin":3}"#).unwrap();
    assert!(!partial.auto_xp);
    assert_eq!(partial.level_margin, 3);
    assert!(partial.town_runs);
    // A config saved before the sale rules had thresholds reads
    // with the defaults, and the defaults are on.
    assert_eq!(partial.sell_run_value, 5_000);
    assert_eq!(partial.sell_run_count, 8);
    assert_eq!(partial.sell_run_patience, 15.0 * 60.0);
    let set: Growth = serde_json::from_str(r#"{"sell_run_value":0,"sell_run_count":3}"#).unwrap();
    assert_eq!(set.sell_run_value, 0);
    assert_eq!(set.sell_run_count, 3);
}

#[test]
fn loot_for_a_counter_is_reason_enough_for_a_run() {
    // An Iron Pea and a Lead Pea, taken to sell, in a pack with room
    // to spare. The ledger said what they were for and nothing read
    // it: a run was made for a full pack, a heavy one or a supply
    // short, and the peas were carried about for ever.
    use ac_world::item_type::{ARMOR, GEM, SPELL_COMPONENTS};
    let cfg = Growth::default();
    let minutes = |m: u64| Some(Duration::from_secs(m * 60));
    let two_peas = [
        salable(1, SPELL_COMPONENTS, 2_500, 1),
        salable(2, SPELL_COMPONENTS, 500, 1),
    ];
    // Three thousand at face is not worth the walk on its own...
    assert_eq!(worth_a_sale_run(&two_peas, None, &cfg), None);
    assert_eq!(worth_a_sale_run(&two_peas, minutes(5), &cfg), None);
    // ...but it is not carried about all afternoon either.
    let why = worth_a_sale_run(&two_peas, minutes(15), &cfg).expect("a quarter of an hour");
    assert!(why.contains("carried 15 min"), "{why}");
    // Worth enough is reason at once, and a stack is worth the lot.
    let gem = [salable(3, GEM, 5_000, 1)];
    let why = worth_a_sale_run(&gem, None, &cfg).expect("five thousand");
    assert!(why.contains("5000 pyreals"), "{why}");
    let peas = [salable(4, SPELL_COMPONENTS, 10 * 500, 10)];
    assert!(worth_a_sale_run(&peas, None, &cfg).is_some());
    // So is an armful, however cheap: the slots are going.
    let junk: Vec<Salable> = (0..8).map(|i| salable(10 + i, ARMOR, 50, 1)).collect();
    let why = worth_a_sale_run(&junk, None, &cfg).expect("an armful");
    assert!(why.contains("8 things"), "{why}");
    assert_eq!(worth_a_sale_run(&junk[..7], None, &cfg), None);
    // The count is of stacks -- the slots going -- not of things: a
    // stack of eight cheap things is one, and seven singles and a
    // stack are eight.
    assert_eq!(
        worth_a_sale_run(&[salable(5, ARMOR, 400, 8)], None, &cfg),
        None
    );
    let mut seven_and_a_stack = junk[..7].to_vec();
    seven_and_a_stack.push(salable(20, ARMOR, 50, 10));
    assert!(worth_a_sale_run(&seven_and_a_stack, None, &cfg).is_some());
    // Nothing for a counter is no reason, however long since.
    assert_eq!(worth_a_sale_run(&[], minutes(60), &cfg), None);
    // Each rule is off at zero.
    let off = Growth {
        sell_run_value: 0,
        sell_run_count: 0,
        sell_run_patience: 0.0,
        ..cfg
    };
    assert_eq!(worth_a_sale_run(&junk, minutes(60), &off), None);
    assert_eq!(worth_a_sale_run(&gem, minutes(60), &off), None);
}

#[test]
fn a_sale_is_ranked_on_what_the_counter_pays() {
    // A tailor with the shopping list on the shelf and no use for
    // the pack, and an archmage with the pack's worth in her purse
    // and nothing on the list. A trip to buy goes to the tailor; a
    // trip to sell goes to the archmage, and the tailor is not so
    // much as a candidate for it.
    let look = |takings: u32, selling: usize, stocks: usize| Forecast {
        takings,
        selling,
        stocks: vec!["something".into(); stocks],
        cheapest: if stocks > 0 { 10 } else { 0 },
        purse: 100,
        ..Default::default()
    };
    let tailor = look(0, 0, 2);
    let archmage = look(2_700, 2, 0);
    assert!(Errand::Buy.served_by(&tailor));
    assert!(!Errand::Sell.served_by(&tailor), "buys none of it");
    assert!(Errand::Sell.served_by(&archmage));
    assert_eq!(
        better_counter(Errand::Buy, (&tailor, 300.0), (&archmage, 20.0)),
        std::cmp::Ordering::Less
    );
    assert_eq!(
        better_counter(Errand::Sell, (&archmage, 300.0), (&tailor, 20.0)),
        std::cmp::Ordering::Less
    );
    // Between two that buy: what they pay, then how much of the
    // pack they take, then the shelf, then the walk.
    let scriveners = look(500, 2, 0);
    let broker = look(950, 2, 0);
    assert_eq!(
        better_counter(Errand::Sell, (&broker, 380.0), (&scriveners, 80.0)),
        std::cmp::Ordering::Less
    );
    let takes_more = look(950, 3, 0);
    assert_eq!(
        better_counter(Errand::Sell, (&takes_more, 380.0), (&broker, 80.0)),
        std::cmp::Ordering::Less
    );
    let stocked_too = look(950, 3, 1);
    assert_eq!(
        better_counter(Errand::Sell, (&stocked_too, 380.0), (&takes_more, 80.0)),
        std::cmp::Ordering::Less
    );
    assert_eq!(
        better_counter(Errand::Sell, (&broker, 80.0), (&broker, 380.0)),
        std::cmp::Ordering::Less
    );
}

#[test]
fn a_run_to_sell_goes_to_a_counter_that_buys_what_it_carries() {
    // Holtburg, with two peas in the pack. Fourteen counters stand
    // in the town and one of them, the archmage, buys spell
    // components; the shops that do so at a worse rate are a few
    // hundred metres out. Ranked the buying way the choice fell to
    // whoever had the most of the shopping list, and the peas went
    // to a counter that would not look at them.
    use ac_world::item_type::{LIFESTONE, SPELL_COMPONENTS};
    let from = ac_world::towns::find("Holtburg").unwrap().world_xy();
    let in_town = || {
        ac_world::shops::all()
            .iter()
            .filter(|s| s.gate.is_none())
            .map(|s| (s, s.xy().distance(from)))
            .filter(|(_, d)| *d <= VENDOR_RINGS[0])
    };
    let peas = [
        salable(1, SPELL_COMPONENTS, 2_500, 1),
        salable(2, SPELL_COMPONENTS, 500, 1),
    ];
    let (shop, look) =
        choose_counter(in_town(), Errand::Sell, &[], 0, &peas).expect("nobody buys peas");
    assert_eq!(shop.name, "Archmage Cindrue");
    assert_eq!(look.selling, 2);
    assert_eq!(look.takings, 2_250 + 450, "nine tenths of both");
    assert!(shop.pays_for(SPELL_COMPONENTS, 2_500).is_some());

    // With a healing kit on the list, the trip to buy goes to a
    // counter with kits -- the pawn shop out on the road, as it
    // happens, which also takes peas at eight tenths...
    let mut kits = need(NeedKind::Named("Healing Kit".into()), 1);
    kits.name = "Healing Kit".into();
    let wants = [(&kits, "healing kit".to_string())];
    let (to_buy, look) =
        choose_counter(in_town(), Errand::Buy, &wants, 10_000, &peas).expect("no kits");
    assert!(to_buy.stocks("Healing Kit").is_some());
    assert!(look.covers_it());
    assert_ne!(to_buy.name, "Archmage Cindrue");
    // ...while the trip to sell, list and all, goes where the peas
    // fetch most.
    let (to_sell, look) = choose_counter(in_town(), Errand::Sell, &wants, 10_000, &peas).unwrap();
    assert_eq!(to_sell.name, "Archmage Cindrue");
    assert!(look.takings > 2_400, "{}", look.takings);

    // Nothing in town buys a lifestone: no counter, and no falling
    // back on the nearest one.
    let odd = [salable(3, LIFESTONE, 1_000, 1)];
    assert!(choose_counter(in_town(), Errand::Sell, &[], 0, &odd).is_none());
    // Nothing wanted and nothing anyone buys is no trip to buy
    // either; the nearest-counter fallback is `pick_vendor`'s own.
    assert!(choose_counter(in_town(), Errand::Buy, &[], 0, &odd).is_none());
}

/// The character, with `capacity` slots in its main pack.
fn with_a_pack(c: &mut Client, capacity: u32) {
    let me = 0x5000_0001;
    c.world.player_guid = Some(me);
    c.world.objects.insert(
        me,
        ac_world::WorldObject {
            guid: me,
            name: "Verity".into(),
            is_player: true,
            items_capacity: capacity,
            ..Default::default()
        },
    );
}

/// A pea in the pack, taken to sell.
fn pea_in_the_pack(c: &mut Client, guid: u32, name: &str, wcid: u32, value: u32) {
    let me = c.world.player_guid.unwrap();
    c.world.objects.insert(
        guid,
        ac_world::WorldObject {
            guid,
            name: name.into(),
            weenie_class_id: wcid,
            item_type: item_type::SPELL_COMPONENTS,
            value,
            stack_size: 1,
            max_stack_size: 100,
            container: Some(me),
            ..Default::default()
        },
    );
    let stats = c.stats_of(guid).unwrap();
    c.autoplay.tag(&stats, LootAction::Sell);
}

/// A wand in hand, a bolt in the book and the Foci of Strife in the
/// pack: a war mage, whose every cast burns a scarab and a prismatic
/// taper.
fn as_a_war_mage(c: &mut Client) {
    const FLAME_BOLT_I: u32 = 27;
    const FOCI_OF_STRIFE: u32 = 15271;
    let me = c.world.player_guid.unwrap();
    c.world.stats.spells = vec![FLAME_BOLT_I];
    c.world.objects.insert(
        0x8000_0040,
        ac_world::WorldObject {
            guid: 0x8000_0040,
            name: "Wand".into(),
            item_type: item_type::CASTER,
            value: 100,
            wielder: Some(me),
            parent: Some(me),
            ..Default::default()
        },
    );
    c.world.objects.insert(
        0x8000_0041,
        ac_world::WorldObject {
            guid: 0x8000_0041,
            name: "Foci of Strife".into(),
            weenie_class_id: FOCI_OF_STRIFE,
            value: 100,
            container: Some(me),
            ..Default::default()
        },
    );
}

/// `stack` prismatic tapers in the pack.
fn tapers_in_the_pack(c: &mut Client, guid: u32, stack: u32) {
    let me = c.world.player_guid.unwrap();
    c.world.objects.insert(
        guid,
        ac_world::WorldObject {
            guid,
            name: "Prismatic Taper".into(),
            weenie_class_id: 20631,
            item_type: item_type::SPELL_COMPONENTS,
            value: stack,
            stack_size: stack,
            max_stack_size: 1_000,
            container: Some(me),
            ..Default::default()
        },
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_casters_peas_are_loot_to_sell_and_not_stock_to_buy() {
    // The report: "autovendoring doesn't seem to sell at all", from
    // a caster with a spellbook and a wand. The peas it looted sit
    // in the component table beside the scarabs, and every carried
    // component went on the restock list at the taper count for
    // want of a burn rate: two peas became two wants for ninety-nine
    // more, the counter skipped them as what the character came to
    // buy, and a counter with peas on the shelf would have bought
    // them at markup. The character that passed a live proof knew
    // spells and wielded nothing, so this never opened for it.
    use ac_vendor::Act;
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 20);
    as_a_war_mage(&mut c);
    pea_in_the_pack(&mut c, 0x8000_0010, "Iron Pea", 8328, 2_500);
    pea_in_the_pack(&mut c, 0x8000_0011, "Lead Pea", 8329, 500);
    tapers_in_the_pack(&mut c, 0x8000_0012, 78);
    with_a_buy_list(&mut c, &[("Prismatic Taper", 100, 25)]);
    let cfg = c.autoplay.config.growth.clone();

    // The gate is open: the tapers are stock, scaled from the line.
    let needs = c.grow_needs(&cfg);
    let taper = needs
        .iter()
        .find(|n| n.kind == NeedKind::Component(20631))
        .expect("the tapers it is short of");
    assert_eq!((taper.have, taper.keep, taper.want), (78, 100, 22));
    // And a pea is not: no cast burns one.
    let pea = needs.iter().find(|n| n.name.contains("Pea"));
    assert!(pea.is_none(), "a pea is stock: {pea:?}");
    let wants = c.vendor_shortfall(&cfg);
    assert!(
        wants.iter().all(|w| w.wcid != 8328 && w.wcid != 8329),
        "a pea on the shopping list: {wants:?}"
    );
    assert!(wants.iter().any(|w| w.wcid == 20631), "{wants:?}");

    // At a counter that buys components, the peas go and the
    // tapers stay.
    let cindrue = 0x8000_0002;
    vendor_beside(
        &mut c,
        cindrue,
        "Archmage Cindrue",
        glam::Vec3::new(1.0, 0.0, 0.0),
    );
    let mut window = window_of(cindrue);
    window.item_types = item_type::SPELL_COMPONENTS;
    c.world.open_vendor = Some(window);
    let snap = c.vendor_snapshot(&cfg);
    let mut offered: Vec<u32> = snap
        .items
        .iter()
        .filter(|i| snap.offers(i))
        .map(|i| i.guid)
        .collect();
    offered.sort_unstable();
    assert_eq!(offered, [0x8000_0010, 0x8000_0011]);
    assert!(
        snap.items
            .iter()
            .any(|i| i.guid == 0x8000_0010 && i.to_sell()),
        "the ledger's word travels with the pea"
    );
    let next = ac_vendor::Run::new().step(&snap, Instant::now());
    assert_eq!(
        next.act,
        Some(Act::Sell {
            items: vec![0x8000_0010, 0x8000_0011]
        }),
        "{}",
        next.saying
    );
}

/// A stack of `stack` `name` in the pack, a spell component, with
/// nothing written down about it.
fn component_in_the_pack(c: &mut Client, guid: u32, name: &str, wcid: u32, stack: u32) {
    let me = c.world.player_guid.unwrap();
    c.world.objects.insert(
        guid,
        ac_world::WorldObject {
            guid,
            name: name.into(),
            weenie_class_id: wcid,
            item_type: item_type::SPELL_COMPONENTS,
            value: 5 * stack,
            stack_size: stack,
            max_stack_size: 1_000,
            container: Some(me),
            ..Default::default()
        },
    );
}

/// The weenie class the archives give a component of this name.
fn component_named(c: &Client, name: &str) -> u32 {
    let table = c.assets.spell_components().unwrap();
    let id = table.find_by_name(name).expect(name);
    c.assets
        .spell_component_ids()
        .unwrap()
        .component_wcid(id)
        .expect(name)
}

/// Write down what the profile makes of this item now, as the
/// arrival pass does: the item is judged by the rules once, when it
/// is taken, and the answer travels with it.
fn tagged_by_the_profile(c: &mut Client, guid: u32) -> LootAction {
    let stats = c.stats_of(guid).unwrap();
    let action = c.loot_action(&stats).expect("a rule claims it");
    c.autoplay.tag(&stats, action);
    action
}

/// The war mage in front of Cindrue's open window, which buys
/// components: what she is offered, and what the run does first.
fn at_cindrues_counter(c: &mut Client, cfg: &Growth) -> (Vec<u32>, ac_vendor::Next) {
    let cindrue = 0x8000_0002;
    vendor_beside(
        c,
        cindrue,
        "Archmage Cindrue",
        glam::Vec3::new(1.0, 0.0, 0.0),
    );
    let mut window = window_of(cindrue);
    window.item_types = item_type::SPELL_COMPONENTS;
    c.world.open_vendor = Some(window);
    let snap = c.vendor_snapshot(cfg);
    let mut offered: Vec<u32> = snap
        .items
        .iter()
        .filter(|i| snap.offers(i))
        .map(|i| i.guid)
        .collect();
    offered.sort_unstable();
    let next = ac_vendor::Run::new().step(&snap, Instant::now());
    (offered, next)
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_scarab_the_player_said_to_sell_goes_though_its_own_spells_burn_it() {
    // The principle, in the player's words: follow the loot
    // profile. A rule that says "sell lead scarabs" is the player's
    // decision about every lead scarab the character picks up, and
    // it was being overruled three ways at once: the component
    // guard kept it from the counter because Flame Bolt burns it,
    // the restock list wanted more of it for the same reason, and
    // the counter skipped it as what the character came to buy.
    use ac_vendor::Act;
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 20);
    as_a_war_mage(&mut c);
    let scarab = component_named(&c, "Lead Scarab");
    tapers_in_the_pack(&mut c, 0x8000_0012, 78);
    component_in_the_pack(&mut c, 0x8000_0013, "Lead Scarab", scarab, 3);
    with_a_profile(
        &mut c,
        "sells-scarabs",
        vec![
            word_rule("lead scarabs to sell", "lead scarab", LootAction::Sell),
            word_rule("components", "taper", LootAction::Keep),
        ],
        &[("Prismatic Taper", 100, 25)],
    );
    assert_eq!(tagged_by_the_profile(&mut c, 0x8000_0013), LootAction::Sell);
    let cfg = c.autoplay.config.growth.clone();
    assert!(
        c.burns(&cfg).contains(&scarab),
        "the spells do burn it; that is the point"
    );

    // Not stock: leaving, so never a need and never a want. The
    // tapers, which the player keeps, still are.
    let needs = c.grow_needs(&cfg);
    assert!(
        !needs.iter().any(|n| n.kind == NeedKind::Component(scarab)),
        "a need for what is being sold: {needs:?}"
    );
    assert!(needs.iter().any(|n| n.kind == NeedKind::Component(20631)));
    let wants = c.vendor_shortfall(&cfg);
    assert!(wants.iter().all(|w| w.wcid != scarab), "{wants:?}");

    // Offered, and sold, at a counter that buys components.
    let (offered, next) = at_cindrues_counter(&mut c, &cfg);
    assert_eq!(offered, [0x8000_0013]);
    assert_eq!(
        next.act,
        Some(Act::Sell {
            items: vec![0x8000_0013]
        }),
        "{}",
        next.saying
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_scarab_nothing_was_decided_about_is_kept_by_the_guard() {
    // The guards still answer for what the profile did not decide.
    // A scarab with no entry in the ledger, burnt by the spells
    // this mage casts, stays out of the counter's hands: "the rest,
    // to the counter" would sell it today, and the guard says no.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 20);
    as_a_war_mage(&mut c);
    let scarab = component_named(&c, "Lead Scarab");
    tapers_in_the_pack(&mut c, 0x8000_0012, 78);
    component_in_the_pack(&mut c, 0x8000_0013, "Lead Scarab", scarab, 1);
    with_a_profile(
        &mut c,
        "sells-the-rest",
        vec![the_rest_to_the_counter()],
        &[("Prismatic Taper", 100, 25)],
    );
    assert_eq!(c.autoplay.ledger.by_guid(0x8000_0013), None);
    let cfg = c.autoplay.config.growth.clone();
    let (offered, next) = at_cindrues_counter(&mut c, &cfg);
    assert!(offered.is_empty(), "{offered:x?}");
    assert!(
        !matches!(next.act, Some(ac_vendor::Act::Sell { .. })),
        "{}",
        next.saying
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_kind_the_rules_would_sell_on_arrival_is_not_bought() {
    // Whether to buy more of a thing is a question about the kind,
    // and the profile answers it the way it will answer for the
    // purchase when it arrives. Under "the rest, to the counter"
    // a bought scarab is tagged to sell as it lands and sold on
    // the next trip, so it is not bought, however low the pack is
    // and whatever the spells burn. Under the starter's rules,
    // which keep components, it is.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 20);
    as_a_war_mage(&mut c);
    let scarab = component_named(&c, "Lead Scarab");
    tapers_in_the_pack(&mut c, 0x8000_0012, 78);
    component_in_the_pack(&mut c, 0x8000_0013, "Lead Scarab", scarab, 1);
    with_a_profile(
        &mut c,
        "sells-the-rest",
        vec![the_rest_to_the_counter()],
        &[("Prismatic Taper", 100, 25)],
    );
    let cfg = c.autoplay.config.growth.clone();
    let needs = c.grow_needs(&cfg);
    assert!(
        !needs.iter().any(|n| n.kind == NeedKind::Component(scarab)),
        "bought to be sold: {needs:?}"
    );
    // The tapers are on the buy list, which is the player's word
    // that they are stock: still a need, whatever the rules say.
    assert!(needs.iter().any(|n| n.kind == NeedKind::Component(20631)));

    with_a_buy_list(&mut c, &[("Prismatic Taper", 100, 25)]);
    let needs = c.grow_needs(&cfg);
    let need = needs
        .iter()
        .find(|n| n.kind == NeedKind::Component(scarab))
        .unwrap_or_else(|| panic!("the scarabs it is short of: {needs:?}"));
    assert_eq!(need.have, 1);
    assert!(need.want > 0);
    assert!(c.vendor_shortfall(&cfg).iter().any(|w| w.wcid == scarab));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn tapers_taken_to_keep_are_neither_sold_nor_wanted() {
    // On the buy list and written down as kept: the counter is not
    // offered them, and a pack holding its full line has nothing
    // to buy. A Keep says "do not sell this"; it does not say "do
    // not buy more", so a short line is still filled -- the buy
    // list is the player's word too.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 20);
    as_a_war_mage(&mut c);
    tapers_in_the_pack(&mut c, 0x8000_0012, 100);
    with_a_profile(
        &mut c,
        "keeps-tapers",
        vec![
            word_rule("components", "taper", LootAction::Keep),
            the_rest_to_the_counter(),
        ],
        &[("Prismatic Taper", 100, 25)],
    );
    assert_eq!(tagged_by_the_profile(&mut c, 0x8000_0012), LootAction::Keep);
    let cfg = c.autoplay.config.growth.clone();

    let wants = c.vendor_shortfall(&cfg);
    assert!(wants.iter().all(|w| w.wcid != 20631), "{wants:?}");
    let (offered, next) = at_cindrues_counter(&mut c, &cfg);
    assert!(offered.is_empty(), "{offered:x?}");
    assert!(
        !matches!(next.act, Some(ac_vendor::Act::Sell { .. })),
        "{}",
        next.saying
    );

    // Short of the line, still kept, and still bought.
    c.world.objects.get_mut(&0x8000_0012).unwrap().stack_size = 78;
    let wants = c.vendor_shortfall(&cfg);
    let taper = wants
        .iter()
        .find(|w| w.wcid == 20631)
        .unwrap_or_else(|| panic!("the tapers it is short of: {wants:?}"));
    assert_eq!(taper.short, 22);
    let snap = c.vendor_snapshot(&cfg);
    assert!(snap.items.iter().filter(|i| snap.offers(i)).count() == 0);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn what_the_server_will_not_take_stays_whatever_the_profile_said() {
    // The one word ahead of the profile's is the server's own. A
    // dagger in hand and a tinkered ring, both written down as
    // meant for a counter, are not offered: a sale the server will
    // not make is not a decision anybody gets to take.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 20);
    let me = c.world.player_guid.unwrap();
    c.world.objects.insert(
        0x8000_0020,
        ac_world::WorldObject {
            guid: 0x8000_0020,
            name: "Dagger".into(),
            weenie_class_id: 300,
            item_type: item_type::MELEE_WEAPON,
            value: 900,
            wielder: Some(me),
            parent: Some(me),
            ..Default::default()
        },
    );
    c.world.objects.insert(
        0x8000_0021,
        ac_world::WorldObject {
            guid: 0x8000_0021,
            name: "Ornate Ring".into(),
            weenie_class_id: 301,
            item_type: item_type::JEWELRY,
            value: 900,
            container: Some(me),
            ..Default::default()
        },
    );
    // Tinkered twice, by the server's appraisal (int 171).
    c.appraisals.insert(
        0x8000_0021,
        ac_net::messages::Appraisal {
            guid: 0x8000_0021,
            success: true,
            ints: vec![(171, 2)],
            ..Default::default()
        },
    );
    with_a_profile(
        &mut c,
        "sells-the-rest",
        vec![the_rest_to_the_counter()],
        &[],
    );
    for guid in [0x8000_0020, 0x8000_0021] {
        let stats = c.stats_of(guid).unwrap();
        c.autoplay.tag(&stats, LootAction::Sell);
    }
    let cfg = c.autoplay.config.growth.clone();
    assert!(c.for_sale(&cfg).is_empty());
    let (offered, next) = at_cindrues_counter(&mut c, &cfg);
    assert!(offered.is_empty(), "{offered:x?}");
    assert!(
        !matches!(next.act, Some(ac_vendor::Act::Sell { .. })),
        "{}",
        next.saying
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn what_was_taken_to_keep_is_not_swept_up_by_the_rest_to_the_counter() {
    // The decision is made once, when the item is taken. A ring
    // taken under "keep ornate rings" stays kept when the rules are
    // later just "the rest, to the counter": asking again at the
    // counter is how a thing taken to keep gets sold on the next
    // run to town.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 20);
    let me = c.world.player_guid.unwrap();
    c.world.objects.insert(
        0x8000_0021,
        ac_world::WorldObject {
            guid: 0x8000_0021,
            name: "Ornate Ring".into(),
            weenie_class_id: 301,
            item_type: item_type::JEWELRY,
            value: 900,
            container: Some(me),
            ..Default::default()
        },
    );
    with_a_profile(
        &mut c,
        "keeps-rings",
        vec![
            word_rule("ornate rings", "ornate", LootAction::Keep),
            the_rest_to_the_counter(),
        ],
        &[],
    );
    assert_eq!(tagged_by_the_profile(&mut c, 0x8000_0021), LootAction::Keep);
    // The rules change under it: today they would sell it.
    with_a_profile(
        &mut c,
        "sells-the-rest",
        vec![the_rest_to_the_counter()],
        &[],
    );
    let stats = c.stats_of(0x8000_0021).unwrap();
    let policy = c.sell_policy(&c.autoplay.config.growth.clone());
    assert!(
        policy.profile.as_ref().is_some_and(|p| matches!(
            p.judge(&stats, None, &policy.wielder, &policy.me, 0),
            crate::profile::Verdict::Decided(LootAction::Sell, _)
        )),
        "the rules as they read now would sell it"
    );
    let cfg = c.autoplay.config.growth.clone();
    assert!(c.for_sale(&cfg).is_empty(), "but it was taken to keep");
    let (offered, _) = at_cindrues_counter(&mut c, &cfg);
    assert!(offered.is_empty(), "{offered:x?}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_stack_to_sell_beside_one_to_keep_goes_whole_and_is_not_poured_into_it() {
    // Two stacks of scarabs, one word each: ten the player said to
    // sell and ninety-five they said to keep. The tidy that runs
    // before every sale used to pour the ten into the ninety-five,
    // the ledger settled the lot as kept, and the counter was
    // offered nothing. The ten go over the counter whole.
    use ac_vendor::Act;
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 20);
    as_a_war_mage(&mut c);
    let scarab = component_named(&c, "Lead Scarab");
    tapers_in_the_pack(&mut c, 0x8000_0012, 100);
    component_in_the_pack(&mut c, 0x8000_0013, "Lead Scarab", scarab, 10);
    component_in_the_pack(&mut c, 0x8000_0014, "Lead Scarab", scarab, 95);
    with_a_profile(
        &mut c,
        "keeps-some",
        vec![word_rule("components", "taper", LootAction::Keep)],
        &[("Prismatic Taper", 100, 25)],
    );
    for (guid, word) in [
        (0x8000_0013, LootAction::Sell),
        (0x8000_0014, LootAction::Keep),
    ] {
        let stats = c.stats_of(guid).unwrap();
        c.autoplay.tag(&stats, word);
    }
    let cfg = c.autoplay.config.growth.clone();
    // The client's own tidy leaves them apart too.
    assert!(
        matches!(c.pour_next(Instant::now()), Err(Unpoured::Tight)),
        "the tidy poured across words"
    );
    let (offered, next) = at_cindrues_counter(&mut c, &cfg);
    assert_eq!(offered, [0x8000_0013]);
    assert_eq!(
        next.act,
        Some(Act::Sell {
            items: vec![0x8000_0013]
        }),
        "{}",
        next.saying
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_stack_on_its_way_out_does_not_hide_the_shortfall_of_the_one_that_stays() {
    // Seventy-eight tapers kept and five hundred tagged to sell,
    // against a line of a hundred. The five hundred are not stock:
    // the line is twenty-two short, and the party hears the same.
    // The restock list once dropped the whole line while any of
    // the kind was leaving, and the party's broadcast counted the
    // leaving stack as stock, so a mate handed tapers over while
    // the character's own list said it wanted none.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 20);
    as_a_war_mage(&mut c);
    tapers_in_the_pack(&mut c, 0x8000_0012, 78);
    tapers_in_the_pack(&mut c, 0x8000_0015, 500);
    with_a_profile(
        &mut c,
        "sells-the-rest",
        vec![the_rest_to_the_counter()],
        &[("Prismatic Taper", 100, 25)],
    );
    for (guid, word) in [
        (0x8000_0012, LootAction::Keep),
        (0x8000_0015, LootAction::Sell),
    ] {
        let stats = c.stats_of(guid).unwrap();
        c.autoplay.tag(&stats, word);
    }
    let cfg = c.autoplay.config.growth.clone();
    let needs = c.grow_needs(&cfg);
    let taper = needs
        .iter()
        .find(|n| n.kind == NeedKind::Component(20631))
        .unwrap_or_else(|| panic!("the tapers it is short of: {needs:?}"));
    assert_eq!((taper.have, taper.want), (78, 22));
    let line = needs
        .iter()
        .find(|n| n.kind == NeedKind::Named("Prismatic Taper".into()))
        .unwrap_or_else(|| panic!("the line it is short of: {needs:?}"));
    assert_eq!((line.have, line.want), (78, 22));
    // And the party is told the same.
    c.autoplay.config.team.enabled = true;
    c.autoplay_stock();
    assert_eq!(c.autoplay.wants, vec!["Prismatic Taper".to_string()]);
    // With the five hundred sold, nothing changes but the count.
    c.world.objects.remove(&0x8000_0015);
    c.autoplay.ledger.forget(0x8000_0015);
    let needs = c.grow_needs(&cfg);
    let taper = needs
        .iter()
        .find(|n| n.kind == NeedKind::Component(20631))
        .unwrap();
    assert_eq!((taper.have, taper.want), (78, 22));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_scarab_sold_at_the_counter_is_not_bought_straight_back() {
    // The round trip the restock list is there to avoid, in one
    // visit: the scarab the player said to sell goes over the
    // counter, its tag is forgotten with it, and the same counter
    // has scarabs on the shelf. Asking only "is any of this
    // leaving?" said no the moment it had gone, and the run bought
    // it back at markup for the arrival pass to tag to sell again.
    use ac_vendor::Act;
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 20);
    as_a_war_mage(&mut c);
    let scarab = component_named(&c, "Lead Scarab");
    tapers_in_the_pack(&mut c, 0x8000_0012, 100);
    component_in_the_pack(&mut c, 0x8000_0013, "Lead Scarab", scarab, 3);
    let me = c.world.player_guid.unwrap();
    c.world.objects.insert(
        0x8000_0016,
        ac_world::WorldObject {
            guid: 0x8000_0016,
            name: "Pyreal".into(),
            weenie_class_id: 273,
            item_type: item_type::MONEY,
            value: 5_000,
            stack_size: 5_000,
            max_stack_size: 25_000,
            container: Some(me),
            ..Default::default()
        },
    );
    with_a_profile(
        &mut c,
        "sells-scarabs",
        vec![
            word_rule("lead scarabs to sell", "lead scarab", LootAction::Sell),
            word_rule("components", "taper", LootAction::Keep),
        ],
        &[("Prismatic Taper", 100, 25)],
    );
    assert_eq!(tagged_by_the_profile(&mut c, 0x8000_0013), LootAction::Sell);
    let cfg = c.autoplay.config.growth.clone();

    let cindrue = 0x8000_0002;
    vendor_beside(
        &mut c,
        cindrue,
        "Archmage Cindrue",
        glam::Vec3::new(1.0, 0.0, 0.0),
    );
    let mut window = window_of(cindrue);
    window.item_types = item_type::SPELL_COMPONENTS;
    window.items.push(ac_world::object::VendorItem {
        guid: 0x9000_0001,
        stack: 100,
        desc: ac_world::object::WeenieDesc {
            name: "Lead Scarab".into(),
            weenie_class_id: scarab,
            item_type: item_type::SPELL_COMPONENTS,
            value: 5,
            ..Default::default()
        },
    });
    c.world.open_vendor = Some(window);
    let mut run = ac_vendor::Run::new();
    let snap = c.vendor_snapshot(&cfg);
    assert!(
        snap.wants.iter().all(|w| w.wcid != scarab),
        "{:?}",
        snap.wants
    );
    let next = run.step(&snap, Instant::now());
    assert_eq!(
        next.act,
        Some(Act::Sell {
            items: vec![0x8000_0013]
        }),
        "{}",
        next.saying
    );
    // Sold: the server takes it, and the ledger forgets it.
    c.world.objects.remove(&0x8000_0013);
    c.autoplay.ledger.forget(0x8000_0013);
    let snap = c.vendor_snapshot(&cfg);
    assert!(
        snap.wants.iter().all(|w| w.wcid != scarab),
        "wanted back the moment it was gone: {:?}",
        snap.wants
    );
    let next = run.step(&snap, Instant::now());
    assert!(
        !matches!(next.act, Some(Act::Buy { wcid, .. }) if wcid == scarab),
        "bought straight back: {} ({:?})",
        next.saying,
        next.act
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_casters_heal_is_stocked_for_though_no_bolt_shares_its_herb() {
    // Heal Self V burns an herb, a powder, a potion and a talisman
    // (7, 26, 41, 61 in the dat) that no bolt or buff burns, and a
    // caster without a Life focus needs every one of them. The
    // restock list once stocked for the buffs and the bolts alone,
    // so when the pack ran out the heal stopped casting and nobody
    // went to town for it.
    const HEAL_SELF_V: u32 = 1160;
    const HERB: u32 = 7;
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 20);
    as_a_war_mage(&mut c);
    tapers_in_the_pack(&mut c, 0x8000_0012, 100);
    with_a_buy_list(&mut c, &[("Prismatic Taper", 100, 25)]);
    let herb = c
        .assets
        .spell_component_ids()
        .unwrap()
        .component_wcid(HERB)
        .expect("the herb's weenie");
    let cfg = c.autoplay.config.growth.clone();
    let needs = c.grow_needs(&cfg);
    assert!(
        !needs.iter().any(|n| n.kind == NeedKind::Component(herb)),
        "no spell of a war mage's burns the herb: {needs:?}"
    );

    c.world.stats.spells.push(HEAL_SELF_V);
    assert!(c.spells_cast().contains(&HEAL_SELF_V));
    let needs = c.grow_needs(&cfg);
    let need = needs
        .iter()
        .find(|n| n.kind == NeedKind::Component(herb))
        .unwrap_or_else(|| panic!("the heal's herb: {needs:?}"));
    assert_eq!(need.have, 0);
    assert!(need.want > 0);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn arrows_the_player_said_to_sell_are_not_counted_as_the_launchers_stock() {
    // Three hundred arrows in the pack and a bow in hand. Tagged to
    // sell, they are loot in the ammunition slot, not stock: the
    // launcher is short by the whole of what it keeps, and a
    // forecast that counted the arrows as stock would set off to
    // town for what it was about to sell -- or not set off at all.
    use ac_world::fletching::ammo_type;
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 20);
    let me = c.world.player_guid.unwrap();
    c.world.objects.insert(
        0x8000_0030,
        ac_world::WorldObject {
            guid: 0x8000_0030,
            name: "Yumi".into(),
            item_type: item_type::MISSILE_WEAPON,
            ammo_type: ammo_type::ARROW,
            value: 500,
            wielder: Some(me),
            parent: Some(me),
            ..Default::default()
        },
    );
    c.world.objects.insert(
        0x8000_0031,
        ac_world::WorldObject {
            guid: 0x8000_0031,
            name: "Arrow".into(),
            weenie_class_id: 300,
            item_type: item_type::MISSILE_WEAPON,
            valid_locations: equip::MISSILE_AMMO,
            value: 300,
            stack_size: 300,
            max_stack_size: 1_000,
            container: Some(me),
            ..Default::default()
        },
    );
    let mut cfg = c.autoplay.config.growth.clone();
    cfg.ammo_keep = 250;
    let needs = c.grow_needs(&cfg);
    assert!(
        !needs
            .iter()
            .any(|n| n.kind == NeedKind::Ammo(ammo_type::ARROW)),
        "three hundred in the pack: {needs:?}"
    );
    let stats = c.stats_of(0x8000_0031).unwrap();
    c.autoplay.tag(&stats, LootAction::Sell);
    let needs = c.grow_needs(&cfg);
    let arrows = needs
        .iter()
        .find(|n| n.kind == NeedKind::Ammo(ammo_type::ARROW))
        .unwrap_or_else(|| panic!("the arrows it will be short of: {needs:?}"));
    assert_eq!((arrows.have, arrows.want), (0, 250));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_counter_that_buys_none_of_the_loot_is_walked_past_on_a_run_to_sell() {
    // At the counter, with the window open, the pack looked over
    // and nothing on the sale list: the run used to stand there and
    // sell nothing, say "sold 0 item(s)", and walk home with the
    // peas. It says who would not buy them and goes on to who will.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 20);
    pea_in_the_pack(&mut c, 0x8000_0010, "Iron Pea", 8328, 2_500);
    pea_in_the_pack(&mut c, 0x8000_0011, "Lead Pea", 8329, 500);
    let cfg = c.autoplay.config.growth.clone();
    assert_eq!(c.salables(&cfg).len(), 2, "the peas are for a counter");
    let me = c.player.as_ref().unwrap().world_position();
    let me = Vec2::new(me.x, me.y);
    let now = Instant::now();
    let rakk = 0x8000_0002;
    vendor_beside(
        &mut c,
        rakk,
        "Rakk the Peddler",
        glam::Vec3::new(1.0, 0.0, 0.0),
    );
    // A tailor's window: armour and clothing, no components.
    let mut window = window_of(rakk);
    window.item_types = item_type::ARMOR | item_type::CLOTHING;
    c.world.open_vendor = Some(window);
    assert!(c.sale_list(&cfg).is_empty(), "Rakk buys peas");

    let mut run = run_to(me, now);
    run.vendor = "Rakk the Peddler".into();
    run.phase = Phase::Appraising;
    run.since = now - SETTLE * 3;
    c.autoplay.growth.run = Some(run);
    c.grow_run_step(now, &cfg);
    assert!(
        c.autoplay
            .status
            .contains("Rakk the Peddler buys none of this"),
        "{}",
        c.autoplay.status
    );
    assert!(c.world.open_vendor.is_none(), "the window was left open");
    assert!(
        c.autoplay.growth.skip_vendors.held(&spot(me), now),
        "Rakk is not left alone"
    );
    // And left alone for at least a run's wait: on the half minute
    // a blocked counter gets, tidied away at the end of the run,
    // Rakk was the best-paying choice again every RUN_EVERY.
    assert!(
        c.autoplay
            .growth
            .skip_vendors
            .held(&spot(me), now + RUN_EVERY - Duration::from_secs(1)),
        "Rakk is the next run's counter"
    );
    // Not standing at Rakk's counter selling nothing: on to a
    // counter that buys peas, of which Holtburg has one.
    let on = c
        .autoplay
        .growth
        .run
        .as_ref()
        .expect("walked home with the peas");
    assert_eq!(on.vendor, "Archmage Cindrue");
    assert_eq!(on.stops, 2);
    assert_eq!(on.errand, Errand::Sell);
    assert!(c.traveling());

    // A run made to buy stays and shops: the counter is the one the
    // player opened, or the one with the tapers on the shelf.
    c.town_run_stop(now);
    c.world.open_vendor = Some({
        let mut w = window_of(rakk);
        w.item_types = item_type::ARMOR;
        w
    });
    let mut run = run_to(me, now);
    run.vendor = "Rakk the Peddler".into();
    run.errand = Errand::Buy;
    run.phase = Phase::Appraising;
    run.since = now - SETTLE * 3;
    c.autoplay.growth.run = Some(run);
    c.grow_run_step(now, &cfg);
    let stayed = c.autoplay.growth.run.as_ref().expect("the run ended");
    assert_eq!(stayed.vendor, "Rakk the Peddler");
    assert!(matches!(stayed.phase, Phase::Selling { .. }));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn peas_for_a_counter_send_a_roomy_pack_to_town_once_the_waits_are_up() {
    // The user's report: two peas tagged to sell, room in the pack,
    // nothing short, and no run was ever made. Now the peas are the
    // reason -- once the waits between runs are up, as for any
    // other reason, so one pea does not wear a path to town.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 50);
    pea_in_the_pack(&mut c, 0x8000_0010, "Iron Pea", 8328, 2_500);
    pea_in_the_pack(&mut c, 0x8000_0011, "Iron Pea", 8328, 2_500);
    let cfg = c.autoplay.config.growth.clone();
    assert!(!c.pack_low_on_room() && !c.laden(&cfg), "the old reasons");
    let now = Instant::now();

    // Just back from a run: the wait between runs holds.
    c.autoplay.growth.last_run = Some(now);
    assert!(!c.grow_town_run(now, &cfg));
    assert!(
        c.autoplay
            .growth
            .held_back
            .contains("to wait since the last run"),
        "{}",
        c.autoplay.growth.held_back
    );
    // A futile one holds for its own while.
    c.autoplay.growth.run_was_futile = true;
    let soon = now + FUTILE_RUN_WAIT - Duration::from_secs(1);
    assert!(!c.grow_town_run(soon, &cfg));
    c.autoplay.growth.run_was_futile = false;
    // Waits up, five thousand at face in the pack: off to the one
    // counter in town that buys peas.
    let later = now + RUN_EVERY;
    assert!(
        c.grow_town_run(later, &cfg),
        "{}",
        c.autoplay.growth.held_back
    );
    let run = c.autoplay.growth.run.as_ref().expect("no run");
    assert_eq!(run.errand, Errand::Sell);
    assert!(run.reason.contains("5000 pyreals' worth"), "{}", run.reason);
    assert_eq!(run.vendor, "Archmage Cindrue");
    assert!(c.traveling());

    // One Lead Pea is not worth the trip on its own...
    c.town_run_stop(later);
    c.world.objects.remove(&0x8000_0010);
    c.world.objects.remove(&0x8000_0011);
    pea_in_the_pack(&mut c, 0x8000_0012, "Lead Pea", 8329, 500);
    let again = later + RUN_EVERY;
    assert!(!c.grow_town_run(again, &cfg));
    assert!(
        c.autoplay
            .growth
            .held_back
            .contains("1 thing(s) for a counter, not yet worth the trip"),
        "{}",
        c.autoplay.growth.held_back
    );
    // ...until it has been carried a quarter of an hour.
    let patience = Duration::from_secs_f32(cfg.sell_run_patience);
    assert!(!c.grow_town_run(again + patience / 2, &cfg));
    assert!(
        c.grow_town_run(again + patience, &cfg),
        "{}",
        c.autoplay.growth.held_back
    );
    let run = c.autoplay.growth.run.as_ref().expect("no run");
    assert!(run.reason.contains("carried 15 min"), "{}", run.reason);
    assert_eq!(run.vendor, "Archmage Cindrue");
    // Setting off put the clock back: what comes home unsold is
    // counted afresh.
    assert_eq!(c.autoplay.growth.sale_since, None);
}

#[test]
fn a_patience_too_large_for_a_duration_turns_the_rule_off() {
    // The config is hand-edited JSON, and 1e20 is a finite f32 that
    // no Duration holds: it used to panic on the first frame a run
    // could start with anything for a counter in the pack.
    use ac_world::item_type::SPELL_COMPONENTS;
    let cfg = Growth {
        sell_run_patience: 1e20,
        ..Growth::default()
    };
    let pea = [salable(1, SPELL_COMPONENTS, 500, 1)];
    assert_eq!(
        worth_a_sale_run(&pea, Some(Duration::from_secs(60 * 60)), &cfg),
        None
    );
}

/// Coin in the pack.
fn coin_in_the_pack(c: &mut Client, guid: u32, amount: u32) {
    let me = c.world.player_guid.unwrap();
    c.world.objects.insert(
        guid,
        ac_world::WorldObject {
            guid,
            name: "Pyreal".into(),
            item_type: item_type::MONEY,
            value: 1,
            stack_size: amount,
            max_stack_size: 25_000,
            container: Some(me),
            ..Default::default()
        },
    );
}

/// Something in the pack by name, `stack` of it, kept.
fn thing_in_the_pack(c: &mut Client, guid: u32, name: &str, stack: u32) {
    let me = c.world.player_guid.unwrap();
    c.world.objects.insert(
        guid,
        ac_world::WorldObject {
            guid,
            name: name.into(),
            value: 1,
            stack_size: stack,
            max_stack_size: 1_000,
            container: Some(me),
            ..Default::default()
        },
    );
}

/// The character's buy list, and nothing else on it: `what`, `keep`
/// of them, urgent at `restock_at` or fewer. The starter's rules.
fn with_a_buy_list(c: &mut Client, lines: &[(&str, u32, u32)]) {
    with_a_profile(c, "wants", crate::profile::Profile::starter().rules, lines);
}

/// A profile of the character's own, `name`, with these `rules` in
/// this order and this buy list. A shelf of its own, so the one
/// every session shares is not touched.
fn with_a_profile(
    c: &mut Client,
    name: &str,
    rules: Vec<crate::profile::Rule>,
    lines: &[(&str, u32, u32)],
) {
    let dir = std::env::temp_dir().join("acswarm-test-growth-profiles");
    std::fs::create_dir_all(&dir).ok();
    let shelf = std::sync::Arc::new(crate::profile::Library::default());
    shelf.open(&dir);
    let mut p = crate::profile::Profile::starter();
    p.name = name.into();
    p.rules = rules;
    p.buy.clear();
    for (what, keep, restock_at) in lines {
        p.buy.push(crate::profile::Buy {
            what: (*what).into(),
            keep: *keep,
            restock_at: Some(*restock_at),
            on: true,
            ..Default::default()
        });
    }
    shelf.put(p).ok();
    c.profiles = shelf;
    c.autoplay.config.loot.profile = name.into();
}

/// One rule: `action` for anything whose name contains `word`.
fn word_rule(name: &str, word: &str, action: LootAction) -> crate::profile::Rule {
    crate::profile::Rule {
        name: name.into(),
        action,
        all: vec![crate::profile::Ask::Item(crate::items::Term::Word(
            word.into(),
        ))],
        ..Default::default()
    }
}

/// "The rest, to the counter": a rule that claims anything at all.
fn the_rest_to_the_counter() -> crate::profile::Rule {
    crate::profile::Rule {
        name: "the rest".into(),
        action: LootAction::Sell,
        all: vec![crate::profile::Ask::Item(crate::items::Term::Num(
            crate::items::NumKey::Value,
            crate::items::Op::Ge,
            0.0,
        ))],
        ..Default::default()
    }
}

/// The shop of this name.
fn shop_named(name: &str) -> &'static ac_world::shops::Shop {
    ac_world::shops::all()
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no shop {name}"))
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn an_urgent_need_comes_before_the_loot_and_the_loot_is_sold_on_the_way() {
    // An archer out of arrows with an armful of cheap peas. Ranked
    // for the sale first, the run went to the archmage and the
    // arrows waited on a second stop that may not reach the bowyer;
    // the supply comes first, and the peas are sold on the way at
    // the counter a hundred metres on, not carried home for a run
    // of their own.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 50);
    for i in 0..8 {
        pea_in_the_pack(&mut c, 0x8000_0010 + i, "Lead Pea", 8329, 100);
    }
    coin_in_the_pack(&mut c, 0x8000_0030, 1_000);
    // Arrowshafts by their full name: a want called "Arrow" is
    // answered by the archmage's Yarrow.
    with_a_buy_list(&mut c, &[("Bundle of Arrowshafts", 10, 5)]);
    let cfg = c.autoplay.config.growth.clone();
    assert_eq!(c.salables(&cfg).len(), 8, "an armful for a counter");
    let now = Instant::now();
    assert!(
        c.grow_town_run(now, &cfg),
        "{}",
        c.autoplay.growth.held_back
    );
    let run = c.autoplay.growth.run.take().expect("no run");
    assert_eq!(run.errand, Errand::Buy);
    assert!(
        run.reason.contains("short of Bundle of Arrowshafts"),
        "{}",
        run.reason
    );
    assert!(
        shop_named(&run.vendor)
            .stocks("Bundle of Arrowshafts")
            .is_some(),
        "{} has no arrowshafts",
        run.vendor
    );
    assert_ne!(run.vendor, "Archmage Cindrue");
    // Done at the bowyer: on to the one counter in town that takes
    // the peas, on the same run.
    c.cancel_travel();
    assert!(c.grow_run_next(run, now, &cfg, None), "went home");
    let on = c
        .autoplay
        .growth
        .run
        .as_ref()
        .expect("went home with the peas");
    assert_eq!(on.vendor, "Archmage Cindrue");
    assert_eq!(on.errand, Errand::Sell);
    assert_eq!(on.stops, 2);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_panel_run_with_a_list_and_a_pea_goes_to_the_counter_with_the_list() {
    // The archer has 180 of 250 arrows -- short, not urgent -- and
    // a pea. The panel's button, made a run to sell whenever the
    // pack held anything for a counter, went to the archmage, sold
    // the pea and ended in town with the arrows unbought. It goes
    // where the list is, and the pea to the archmage after.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 50);
    pea_in_the_pack(&mut c, 0x8000_0010, "Lead Pea", 8329, 500);
    thing_in_the_pack(&mut c, 0x8000_0020, "Bundle of Arrowshafts", 18);
    coin_in_the_pack(&mut c, 0x8000_0030, 1_000);
    with_a_buy_list(&mut c, &[("Bundle of Arrowshafts", 25, 6)]);
    let cfg = c.autoplay.config.growth.clone();
    let now = Instant::now();
    let needs = c.needs_now(now, &cfg);
    assert!(
        needs
            .iter()
            .any(|n| n.name == "Bundle of Arrowshafts" && !n.urgent && n.want == 7),
        "{needs:?}"
    );
    c.town_run_by_hand(now).expect("no run");
    let run = c.autoplay.growth.run.take().expect("no run");
    assert_eq!(run.errand, Errand::Buy);
    assert!(
        shop_named(&run.vendor)
            .stocks("Bundle of Arrowshafts")
            .is_some(),
        "{}",
        run.vendor
    );
    assert_ne!(run.vendor, "Archmage Cindrue");
    c.cancel_travel();
    assert!(
        c.grow_run_next(run, now, &cfg, None),
        "went home with the pea"
    );
    let on = c.autoplay.growth.run.as_ref().unwrap();
    assert_eq!(on.vendor, "Archmage Cindrue");
    assert_eq!(on.errand, Errand::Sell);
}

/// Every counter within `reach` of the character that would pay for
/// a pea, held off.
fn nobody_near_buys_peas(c: &mut Client, reach: f32, now: Instant) -> usize {
    use ac_world::item_type::SPELL_COMPONENTS;
    let me = c.player.as_ref().unwrap().world_position();
    let me = Vec2::new(me.x, me.y);
    let buyers: Vec<Vec2> = ac_world::shops::all()
        .iter()
        .filter(|s| s.xy().distance(me) <= reach)
        .filter(|s| s.pays_for(SPELL_COMPONENTS, 500).is_some())
        .map(|s| s.xy())
        .collect();
    for at in &buyers {
        c.autoplay
            .growth
            .skip_vendors
            .hold(spot(*at), Duration::from_secs(60 * 60), now);
    }
    buyers.len()
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_panel_run_starts_even_when_nobody_buys_the_pea() {
    // Nothing on the list, a pea in the pack, and no counter near
    // that takes it: the button still starts a run, to the nearest
    // counter, as it always did.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 50);
    pea_in_the_pack(&mut c, 0x8000_0010, "Lead Pea", 8329, 500);
    with_a_buy_list(&mut c, &[]);
    let now = Instant::now();
    assert!(nobody_near_buys_peas(&mut c, SALE_RUN_REACH, now) > 0);
    c.town_run_by_hand(now).expect("no run");
    let run = c.autoplay.growth.run.as_ref().expect("no run");
    assert_eq!(run.errand, Errand::Buy);
    assert_ne!(run.vendor, "Archmage Cindrue");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_run_to_sell_for_a_light_reason_stays_within_the_town() {
    // One Lead Pea, carried a quarter of an hour, with nobody in
    // the town buying it: not a walk to an archmage three towns
    // over. A pack that cannot hunt on is worth a walk anywhere; a
    // pea is worth the town the character is in.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 50);
    pea_in_the_pack(&mut c, 0x8000_0010, "Lead Pea", 8329, 500);
    with_a_buy_list(&mut c, &[]);
    let cfg = c.autoplay.config.growth.clone();
    let now = Instant::now();
    assert!(nobody_near_buys_peas(&mut c, SALE_RUN_REACH, now) > 0);
    let patience = Duration::from_secs_f32(cfg.sell_run_patience);
    c.autoplay.growth.sale_since = Some(now - patience);
    assert!(!c.grow_town_run(now, &cfg));
    assert!(c.autoplay.growth.run.is_none());
    assert!(
        c.autoplay
            .growth
            .held_back
            .contains("nobody within 600 m of a way out buys any of the 1 thing(s)"),
        "{}",
        c.autoplay.growth.held_back
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_party_run_sells_what_this_character_carries_first() {
    // Two on a team that restocks together, the party gone shopping
    // for a mate's full pack, and this one carrying two Iron Peas.
    // The party's run was always a run to buy, so its first stop
    // was whichever counter had the most of the list, with the
    // peas along for the walk; its own pack decides its errand.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 50);
    pea_in_the_pack(&mut c, 0x8000_0010, "Iron Pea", 8328, 2_500);
    pea_in_the_pack(&mut c, 0x8000_0011, "Iron Pea", 8328, 2_500);
    with_a_buy_list(&mut c, &[]);
    c.autoplay.config.team.enabled = true;
    c.autoplay.config.team.restock.together = true;
    c.autoplay.team.mates = vec![crate::autoplay::Mate {
        name: "+Brynith".into(),
        guid: 0x5000_0002,
        ..Default::default()
    }];
    let cfg = c.autoplay.config.growth.clone();
    let now = Instant::now();
    // What it tells the party: loot enough for a trip.
    assert!(c.supplies(&cfg, now).sale);
    c.autoplay.growth.mode = logistics::GroupMode::Restocking(Stage::Shopping);
    c.autoplay.growth.mode_because = "+Brynith's pack is full".into();
    assert!(
        c.grow_town_run(now, &cfg),
        "{}",
        c.autoplay.growth.held_back
    );
    let run = c.autoplay.growth.run.as_ref().expect("no run");
    assert_eq!(run.reason, "+Brynith's pack is full");
    assert_eq!(run.errand, Errand::Sell);
    assert_eq!(run.vendor, "Archmage Cindrue");
}

#[test]
fn a_use_turned_away_for_our_own_cast_is_sent_over_not_given_up() {
    use OnOpening::*;
    let (open, shut) = (true, false);
    let (flying, landed) = (true, false);
    let soon = Duration::from_secs(1);
    let long = VENDOR_OPEN_TIMEOUT + Duration::from_secs(1);
    // An open window ends the wait whatever else is going on.
    assert_eq!(on_opening(open, Some(soon), flying, 0, long, 2), Opened);
    // Turned away as busy: wait for our own cast, then a cast's
    // length more (the counter's UseDone frees the slot early), then
    // ask over -- and the retry is not spent, however long it took.
    assert_eq!(on_opening(shut, Some(soon), flying, 0, soon, 1), Wait);
    assert_eq!(on_opening(shut, Some(soon), landed, 0, soon, 1), Wait);
    assert_eq!(on_opening(shut, Some(soon), flying, 0, long, 1), Wait);
    assert_eq!(
        on_opening(shut, Some(BUSY_REASK), landed, 0, long, 1),
        AskOver
    );
    assert_eq!(
        on_opening(shut, Some(BUSY_REASK), landed, 0, long, 2),
        AskOver
    );
    // A counter that keeps saying busy with nothing of ours in the
    // air is taken at its word: the ordinary silence rules apply.
    assert_eq!(
        on_opening(shut, Some(BUSY_REASK), landed, BUSY_ASKS, soon, 1),
        Wait
    );
    assert_eq!(
        on_opening(shut, Some(BUSY_REASK), landed, BUSY_ASKS, long, 1),
        AskAgain
    );
    assert_eq!(
        on_opening(shut, Some(BUSY_REASK), landed, BUSY_ASKS, long, 2),
        GiveUp
    );
    // Silence: one more ask, then the counter is given up.
    assert_eq!(on_opening(shut, None, landed, 0, soon, 1), Wait);
    assert_eq!(on_opening(shut, None, landed, 0, long, 1), AskAgain);
    assert_eq!(on_opening(shut, None, landed, 0, long, 2), GiveUp);
}

#[test]
fn the_counter_is_stood_at_from_the_use_to_the_window_closing() {
    let opening = Phase::Opening {
        guid: 1,
        tries: 1,
        busy: None,
        asked_over: 0,
    };
    let selling = Phase::Selling { sent: Vec::new() };
    assert_eq!(
        counter_asked(Some(&Phase::Going), Some(2)),
        None,
        "still walking: a window open now is one left over"
    );
    assert_eq!(counter_asked(Some(&opening), None), Some(1));
    assert_eq!(counter_asked(Some(&Phase::Appraising), Some(1)), Some(1));
    assert_eq!(counter_asked(Some(&selling), Some(1)), Some(1));
    assert_eq!(
        counter_asked(None, Some(2)),
        Some(2),
        "a window open by hand"
    );
    assert_eq!(counter_asked(None, None), None);
}

/// A weenie error as the server sends it, as a game event body.
fn weenie_error(code: u32) -> Vec<u8> {
    let mut w = ac_net::wire::Writer::new();
    w.u32(0x5000_0001)
        .u32(0)
        .u32(ac_net::messages::event::WEENIE_ERROR)
        .u32(code);
    w.finish()
}

/// A counter in view at `at`.
fn a_counter(c: &mut Client, guid: u32, name: &str, at: glam::Vec3) {
    let mut o = ac_world::WorldObject {
        guid,
        name: name.into(),
        item_type: item_type::CREATURE,
        object_desc_flags: object_desc_flags::VENDOR,
        ..Default::default()
    };
    o.position = Some(ac_world::object::Position {
        cell: 0xA9B4_0019,
        local: at,
        rotation: glam::Quat::IDENTITY,
    });
    c.world.objects.insert(guid, o);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_counter_that_said_busy_is_asked_again_once_the_cast_lands() {
    // +Vesperi's Use of Archmage Cindrue went out with a protection
    // still going up, ACE turned it away as YoureTooBusy, and the
    // run read the counter's silence as a counter that would not
    // trade: one retry spent on the same refusal, then the counter
    // held off as no use, the peas still in the pack.
    let holtburg = 0xA9B4_0019;
    let here = glam::Vec3::new(84.0, 7.1, 94.0);
    let mut c = standing_at(holtburg, here);
    c.world.player_guid = Some(0x5000_0001);
    let me = c.player.as_ref().unwrap().world_position();
    let cfg = Growth::default();
    let now = Instant::now();
    let cindrue = 0x7a9b_4033;
    a_counter(&mut c, cindrue, "Archmage Cindrue", here);
    let mut run = run_to(Vec2::new(me.x, me.y), now);
    run.vendor = "Archmage Cindrue".into();
    run.phase = Phase::Opening {
        guid: cindrue,
        tries: 1,
        busy: None,
        asked_over: 0,
    };
    c.autoplay.growth.run = Some(run);
    // A cast is in the air, and the counter says so.
    c.autoplay.cast_sent = Some(now);
    c.chat_message(
        ac_net::messages::opcode::GAME_EVENT,
        &weenie_error(crate::YOURE_TOO_BUSY),
    );
    assert!(
        matches!(
            c.autoplay.growth.run.as_ref().unwrap().phase,
            Phase::Opening { busy: Some(_), .. }
        ),
        "the refusal was not heard as ours"
    );
    let sent = c.session.actions_sent();
    assert_eq!(c.grow_run_step(now, &cfg), Turn::Waited);
    assert!(
        c.town_run_view(now)
            .unwrap()
            .saying
            .contains("turned the Use away"),
        "the wait is not said"
    );
    // The counter's own UseDone frees the slot a moment after the
    // refusal, with the cast still going up: the ask waits a cast's
    // length after the refusal as well as for the slot.
    c.autoplay.cast_sent = None;
    let soon = now + Duration::from_secs(1);
    assert_eq!(c.grow_run_step(soon, &cfg), Turn::Waited);
    assert_eq!(
        c.session.actions_sent(),
        sent,
        "the Use went out into the cast"
    );
    // Long past the counter's time, with a cast in the air again:
    // no retry is spent and nothing is given up.
    let late = now + VENDOR_OPEN_TIMEOUT + Duration::from_secs(1);
    c.autoplay.cast_sent = Some(late);
    assert_eq!(c.grow_run_step(late, &cfg), Turn::Waited);
    assert_eq!(
        c.session.actions_sent(),
        sent,
        "the Use went out into the cast"
    );
    // The cast lands: the Use goes out again.
    c.autoplay.cast_sent = None;
    let landed = late;
    assert_eq!(c.grow_run_step(landed, &cfg), Turn::Acted);
    assert_eq!(
        c.session.actions_sent(),
        sent + 1,
        "the Use was not sent over"
    );
    let run = c.autoplay.growth.run.as_ref().expect("the run goes on");
    assert!(
        matches!(
            run.phase,
            Phase::Opening {
                tries: 1,
                busy: None,
                asked_over: 1,
                ..
            }
        ),
        "the retry was spent, or the refusal kept: {:?}",
        run.phase
    );
    assert_eq!(run.since, landed, "the counter's time did not start over");
    assert!(
        !c.autoplay.growth.skip_vendors.held(&spot(run.at), landed),
        "the counter was held off as no use"
    );
    // Silence from here on is the counter's own: the ordinary
    // retry, then given up.
    let quiet = landed + VENDOR_OPEN_TIMEOUT + Duration::from_secs(1);
    assert_eq!(c.grow_run_step(quiet, &cfg), Turn::Acted);
    assert!(matches!(
        c.autoplay.growth.run.as_ref().unwrap().phase,
        Phase::Opening { tries: 2, .. }
    ));
    let quieter = quiet + VENDOR_OPEN_TIMEOUT + Duration::from_secs(1);
    c.grow_run_step(quieter, &cfg);
    assert!(
        c.autoplay
            .growth
            .skip_vendors
            .held(&spot(Vec2::new(me.x, me.y)), quieter),
        "a counter that never answered was not held off"
    );
}

/// A caster standing in Holtburg with a wand in hand, the
/// components, the skill and the mana for each of `spells`, and
/// the server's clock known. The spells, in the order asked for.
fn a_caster_knowing(now: Instant, spells: &[&str]) -> (Client, Vec<u32>) {
    let holtburg = 0xA9B4_0019;
    let here = glam::Vec3::new(84.0, 7.1, 94.0);
    let mut c = standing_at(holtburg, here);
    let me = 0x5000_0001;
    c.world.player_guid = Some(me);
    // The server's clock, without which nothing is ever due.
    let clock = ac_net::packet::build(
        ac_net::packet::Header {
            flags: ac_net::packet::flags::TIME_SYNC,
            ..Default::default()
        },
        &1000.0f64.to_le_bytes(),
        &[],
        0,
    );
    c.session.receive(&clock, now);
    assert!(c.session.server_time().is_some(), "the clock was not taken");
    let table = c.assets.spell_table().expect("the spell table");
    let mut known = Vec::new();
    for name in spells {
        let (spell, sp) = table
            .spells
            .iter()
            .find(|(_, sp)| sp.name == *name)
            .map(|(id, sp)| (*id, sp.clone()))
            .unwrap_or_else(|| panic!("the spell table knows {name}"));
        c.world.stats.spells.push(spell);
        let skill = Client::school_skill(sp.school).expect("a school with a skill");
        if !c.world.stats.skills.iter().any(|s| s.id == skill) {
            c.world.stats.skills.push(ac_world::stats::Skill {
                id: skill,
                advancement: ac_world::stats::sac::TRAINED,
                init_level: 300,
                ..Default::default()
            });
        }
        known.push(spell);
    }
    c.world.stats.vitals[2].current = 500;
    const WAND: u32 = 0x8000_0102;
    c.world.objects.insert(
        WAND,
        ac_world::WorldObject {
            guid: WAND,
            name: "Training Wand".into(),
            item_type: item_type::CASTER,
            valid_locations: equip::HELD,
            wielder: Some(me),
            ..Default::default()
        },
    );
    let mapper = c.assets.spell_component_ids().expect("the component ids");
    let mut guid = 0x8000_0200;
    for spell in &known {
        for component in c.current_formula(*spell) {
            let wcid = mapper
                .component_wcid(component)
                .expect("a component with a weenie");
            c.world.objects.insert(
                guid,
                ac_world::WorldObject {
                    guid,
                    name: format!("Component {component}"),
                    weenie_class_id: wcid,
                    stack_size: 20,
                    container: Some(me),
                    ..Default::default()
                },
            );
            guid += 1;
        }
    }
    for spell in &known {
        assert!(
            matches!(c.can_cast(*spell), crate::magic::CastCheck::Ok),
            "the caster cannot cast {spell}: {:?}",
            c.can_cast(*spell)
        );
    }
    (c, known)
}

/// A caster standing in Holtburg with a wand in hand, the
/// components and mana for Blade Protection Self, the server's
/// clock known and no protection up: one buff due, urgent or not.
fn a_caster_with_a_buff_due(now: Instant) -> (Client, u32) {
    let (mut c, known) = a_caster_knowing(now, &["Blade Protection Self I"]);
    c.autoplay.config.buffs.auto = false;
    c.autoplay.config.buffs.spells = vec!["Blade Protection Self".into()];
    (c, known[0])
}

/// A run standing at Archmage Cindrue's counter, the Use just
/// gone out: the counter in view where the character stands, and
/// the run in `phase` there.
fn a_run_at_cindrue(c: &mut Client, phase: Phase, now: Instant) -> u32 {
    let me = c.player.as_ref().unwrap().world_position();
    let cindrue = 0x7a9b_4033;
    a_counter(
        c,
        cindrue,
        "Archmage Cindrue",
        glam::Vec3::new(84.0, 7.1, 94.0),
    );
    let mut run = run_to(Vec2::new(me.x, me.y), now);
    run.vendor = "Archmage Cindrue".into();
    run.phase = phase;
    c.autoplay.growth.run = Some(run);
    cindrue
}

/// The Opening phase as a run enters it.
fn opening(guid: u32) -> Phase {
    Phase::Opening {
        guid,
        tries: 1,
        busy: None,
        asked_over: 0,
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn buffs_wait_at_the_counter_and_go_up_on_the_walk_and_after() {
    // +Vesperi arrived at Archmage Cindrue with eight protections
    // lapsing and cast them one after another from the counter; the
    // buff pass held for a journey and for a fight, and standing at
    // a counter was neither.
    let now = Instant::now();
    let (mut c, _) = a_caster_with_a_buff_due(now);
    let me = c.player.as_ref().unwrap().world_position();
    let cindrue = 0x7a9b_4033;
    a_counter(
        &mut c,
        cindrue,
        "Archmage Cindrue",
        glam::Vec3::new(84.0, 7.1, 94.0),
    );
    // On the walk to town the urgent pass casts as it always did.
    let mut run = run_to(Vec2::new(me.x, me.y), now);
    run.vendor = "Archmage Cindrue".into();
    c.autoplay.growth.run = Some(run.clone());
    let sent = c.session.actions_sent();
    assert!(c.autoplay_buff(now, true), "no cast on the walk");
    assert!(c.session.actions_sent() > sent, "the cast did not go out");
    assert!(!c.autoplay.buffs_held_at_counter);
    // At the counter, from the Use going out, neither pass casts.
    let at_counter = |phase: Phase| {
        let mut run = run.clone();
        run.phase = phase;
        run
    };
    let mut then = now;
    for phase in [
        Phase::Opening {
            guid: cindrue,
            tries: 1,
            busy: None,
            asked_over: 0,
        },
        Phase::Appraising,
        Phase::Selling { sent: Vec::new() },
    ] {
        then += Duration::from_secs(2);
        c.autoplay.growth.run = Some(at_counter(phase.clone()));
        // The window is open once the counter has answered the Use.
        c.world.open_vendor = (!matches!(phase, Phase::Opening { .. })).then(|| window_of(cindrue));
        c.autoplay.cast_sent = None;
        let sent = c.session.actions_sent();
        assert!(!c.autoplay_buff(then, true), "an urgent cast at {phase:?}");
        assert!(!c.autoplay_buff(then, false), "a cast at {phase:?}");
        assert_eq!(
            c.session.actions_sent(),
            sent,
            "something went out at {phase:?}"
        );
        assert!(c.autoplay.buffs_held_at_counter, "the hold was not said");
    }
    // A window the player opened by hand holds the pass the same way.
    c.autoplay.growth.run = None;
    c.world.open_vendor = Some(window_of(cindrue));
    then += Duration::from_secs(2);
    assert!(!c.autoplay_buff(then, true), "a cast into an open window");
    // The window closed, the buff goes up.
    c.world.open_vendor = None;
    then += Duration::from_secs(2);
    let sent = c.session.actions_sent();
    assert!(
        c.autoplay_buff(then, true),
        "no cast once the window closed"
    );
    assert!(c.session.actions_sent() > sent, "the cast did not go out");
    assert!(
        !c.autoplay.buffs_held_at_counter,
        "the hold outlived the counter"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn urgent_buffs_go_up_in_a_fight_at_the_counter() {
    // A wandering monster catches the caster at the counter. The
    // fight outranks the run, which stays in its counter phase
    // throughout, and a hold that read the phase alone kept every
    // protection down for a window nobody was trading at until
    // the fight was over.
    let now = Instant::now();
    let (mut c, _) = a_caster_with_a_buff_due(now);
    let cindrue = 0x7a9b_4033;
    a_run_at_cindrue(&mut c, opening(cindrue), now);
    assert!(
        !c.autoplay_buff(now, true),
        "a cast at the counter with no fight on"
    );
    // Something is hitting the character.
    c.autoplay.last_hit_us = Some(Instant::now());
    let sent = c.session.actions_sent();
    assert!(
        c.autoplay_buff(now, true),
        "the urgent buff waited for the counter through the fight"
    );
    assert!(c.session.actions_sent() > sent, "the cast did not go out");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_window_left_open_from_across_the_town_holds_no_buff() {
    // Nothing closes a window but this client, since the server
    // keeps none. One the player opened and walked away from read
    // as a counter at hand for the rest of the session, and no
    // buff went up again.
    let now = Instant::now();
    let (mut c, _) = a_caster_with_a_buff_due(now);
    let cindrue = 0x7a9b_4033;
    // The counter is a hundred metres off.
    a_counter(
        &mut c,
        cindrue,
        "Archmage Cindrue",
        glam::Vec3::new(184.0, 7.1, 94.0),
    );
    c.world.open_vendor = Some(window_of(cindrue));
    assert_eq!(c.counter_at_hand(), None);
    let sent = c.session.actions_sent();
    assert!(
        c.autoplay_buff(now, true),
        "held for a window left open across the town"
    );
    assert!(c.session.actions_sent() > sent, "the cast did not go out");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn town_runs_switched_off_at_the_counter_close_its_window() {
    // A run let go for town runs being switched off left its
    // window open, and with town runs off no later run would ever
    // close it: a counter at hand for the rest of the session.
    let now = Instant::now();
    let mut c = standing_at(0xA9B4_0019, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.player_guid = Some(0x5000_0001);
    let cindrue = a_run_at_cindrue(&mut c, Phase::Selling { sent: Vec::new() }, now);
    c.world.open_vendor = Some(window_of(cindrue));
    assert_eq!(c.counter_at_hand().as_deref(), Some("Archmage Cindrue"));
    c.autoplay.config.growth.town_runs = false;
    c.autoplay_grow(now);
    assert!(
        !c.autoplay.growth.town_run_under_way(),
        "the run was not let go"
    );
    assert!(
        c.world.open_vendor.is_none(),
        "the run's window was left open"
    );
    assert_eq!(c.counter_at_hand(), None, "still at a counter");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn dying_at_the_counter_ends_the_visit() {
    // The run stood at its counter through the death, the window
    // stayed open, and recovery -- which puts the buffs back before
    // the walk to the corpse -- waited on buffs that waited on the
    // counter.
    let now = Instant::now();
    let mut c = standing_at(0xA9B4_0019, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.player_guid = Some(0x5000_0001);
    let cindrue = a_run_at_cindrue(&mut c, Phase::Selling { sent: Vec::new() }, now);
    c.world.open_vendor = Some(window_of(cindrue));
    // Dead: health at zero on a sheet with a name and an Endurance
    // to draw a maximum from.
    c.world.stats.name = "Vesperi".into();
    c.world.stats.attributes[1].base = 100;
    c.world.stats.vitals[0].current = 0;
    assert!(c.is_dead(), "not dead");
    c.autoplay_recover(now);
    assert!(
        !c.autoplay.growth.town_run_under_way(),
        "the run stood at the counter through the death"
    );
    assert!(c.world.open_vendor.is_none(), "the window was left open");
    assert_eq!(c.counter_at_hand(), None);
    // A run still on its walk is kept: the journey is resumed after.
    let me = c.player.as_ref().unwrap().world_position();
    c.autoplay.growth.run = Some(run_to(Vec2::new(me.x, me.y), now));
    c.counter_left_behind(now);
    assert!(
        c.autoplay.growth.town_run_under_way(),
        "a run on its walk was stopped for a death"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn vitals_wait_at_the_counter() {
    // Only the buffs were held. The vitals pass runs every tick
    // and casts the moment the last cast lands, so a Revitalize
    // thrown from the counter never left it a gap to open its
    // window in: the measured failure, one reflex over.
    let now = Instant::now();
    let (mut c, _) = a_caster_knowing(now, &["Revitalize Self I"]);
    c.autoplay.config.survive.manage_mana = true;
    // The stamina is gone.
    c.world.stats.vitals[1].current = 0;
    // Away from any counter the top-up goes out.
    let sent = c.session.actions_sent();
    assert!(
        c.autoplay_vitals(now),
        "no Revitalize with the stamina gone"
    );
    assert!(c.session.actions_sent() > sent, "the cast did not go out");
    c.autoplay.cast_sent = None;
    // At the counter it waits.
    let cindrue = 0x7a9b_4033;
    a_run_at_cindrue(&mut c, opening(cindrue), now);
    let sent = c.session.actions_sent();
    assert!(!c.autoplay_vitals(now), "a top-up cast at the counter");
    assert_eq!(c.session.actions_sent(), sent, "something went out");
    // Unless a fight has come to it there.
    c.autoplay.last_hit_us = Some(Instant::now());
    assert!(
        c.autoplay_vitals(now),
        "the top-up waited for the counter through a fight"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_use_waits_for_a_cast_on_the_walk_in_to_land() {
    // The first Use went out the moment the character stood at
    // the counter, whatever was in the air: a protection put back
    // on the walk in met it with its recoil, and the counter turned
    // the Use away for it.
    let now = Instant::now();
    let mut c = standing_at(0xA9B4_0019, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.player_guid = Some(0x5000_0001);
    let cfg = Growth::default();
    a_run_at_cindrue(&mut c, Phase::Going, now);
    c.autoplay.cast_sent = Some(now);
    let sent = c.session.actions_sent();
    assert_eq!(c.grow_run_step(now, &cfg), Turn::Waited);
    assert_eq!(
        c.session.actions_sent(),
        sent,
        "the Use went out into the cast"
    );
    assert!(matches!(
        c.autoplay.growth.run.as_ref().unwrap().phase,
        Phase::Going
    ));
    // The cast lands: the counter is asked.
    c.autoplay.cast_sent = None;
    assert_eq!(c.grow_run_step(now, &cfg), Turn::Acted);
    assert_eq!(c.session.actions_sent(), sent + 1, "the Use was not sent");
    assert!(matches!(
        c.autoplay.growth.run.as_ref().unwrap().phase,
        Phase::Opening { .. }
    ));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_refusal_past_the_busy_asks_is_the_counters_own() {
    // Past the asks a busy refusal earns the run is on the ordinary
    // clock, but a later refusal was still kept, and the status
    // promised an ask "once the cast lands" that was never coming.
    let now = Instant::now();
    let mut c = standing_at(0xA9B4_0019, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.player_guid = Some(0x5000_0001);
    let cindrue = 0x7a9b_4033;
    a_run_at_cindrue(
        &mut c,
        Phase::Opening {
            guid: cindrue,
            tries: 1,
            busy: None,
            asked_over: BUSY_ASKS,
        },
        now,
    );
    c.autoplay.growth.counter_said_busy(now);
    assert!(
        matches!(
            c.autoplay.growth.run.as_ref().unwrap().phase,
            Phase::Opening { busy: None, .. }
        ),
        "a refusal past the cap was kept"
    );
    let saying = c.town_run_view(now).unwrap().saying;
    assert!(
        saying.contains("waiting for"),
        "the status still promises an ask: {saying}"
    );
    // Under the cap it is heard.
    a_run_at_cindrue(
        &mut c,
        Phase::Opening {
            guid: cindrue,
            tries: 1,
            busy: None,
            asked_over: BUSY_ASKS - 1,
        },
        now,
    );
    c.autoplay.growth.counter_said_busy(now);
    assert!(matches!(
        c.autoplay.growth.run.as_ref().unwrap().phase,
        Phase::Opening { busy: Some(_), .. }
    ));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_swing_in_the_air_does_not_hold_the_ask() {
    // ACE is never busy for a swing, and a swing's answer can go
    // missing: read raw, one lost AttackDone parked a busy wait in
    // Opening with no clock at all.
    let now = Instant::now();
    let mut c = standing_at(0xA9B4_0019, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.player_guid = Some(0x5000_0001);
    let cfg = Growth::default();
    let cindrue = 0x7a9b_4033;
    a_run_at_cindrue(
        &mut c,
        Phase::Opening {
            guid: cindrue,
            tries: 1,
            busy: Some(now),
            asked_over: 0,
        },
        now,
    );
    c.attack_pending = true;
    c.autoplay.cast_sent = None;
    let sent = c.session.actions_sent();
    let later = now + BUSY_REASK;
    assert_eq!(
        c.grow_run_step(later, &cfg),
        Turn::Acted,
        "the ask waited on a swing"
    );
    assert_eq!(
        c.session.actions_sent(),
        sent + 1,
        "the Use was not sent over"
    );
    assert!(matches!(
        c.autoplay.growth.run.as_ref().unwrap().phase,
        Phase::Opening {
            busy: None,
            asked_over: 1,
            ..
        }
    ));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_hold_is_let_go_by_a_pass_that_left_early() {
    // The note's flag was cleared only on the way past the hold. A
    // pass that turned back before it -- the top-ups, in a fight,
    // out of combat only -- left the flag set after the counter,
    // and the next visit's wait went unsaid.
    let now = Instant::now();
    let (mut c, _) = a_caster_with_a_buff_due(now);
    let cindrue = 0x7a9b_4033;
    a_run_at_cindrue(&mut c, opening(cindrue), now);
    assert!(!c.autoplay_buff(now, true));
    assert!(c.autoplay.buffs_held_at_counter, "the hold was not said");
    // Off the counter and in a fight, the top-up pass turns back
    // at once.
    c.autoplay.growth.run = None;
    c.autoplay.config.buffs.out_of_combat_only = true;
    c.attack_target = Some(0xdead);
    assert!(!c.autoplay_buff(now, false));
    assert!(
        !c.autoplay.buffs_held_at_counter,
        "the flag outlived the counter"
    );
}
