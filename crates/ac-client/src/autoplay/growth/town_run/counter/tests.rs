use super::*;
use crate::autoplay::growth::tests::{a_run_at_cindrue, run_to};
use crate::testkit::{
    a_counter, pea_in_the_pack, standing_at, vendor_beside, window_of, with_a_pack,
};
use ac_world::item_type;

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
fn a_followers_own_town_run_goes_on_through_following_and_a_fight() {
    // Going with the leader leaves a follower's own run alone: following waits it out, the steps
    // that step aside for a leader do not step aside from it, and following beginning mid-run
    // drops none of the run's own plans.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.player_guid = Some(crate::testkit::ME);
    c.world.stats.level = 20;
    c.autoplay.config.enabled = true;
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let counter = Vec2::new(me.x + 250.0, me.y);
    assert!(c.grow_travel(counter, now), "no way to the counter");
    c.autoplay.growth.run = Some(run_to(counter, now));
    let bound_for = |c: &Client| c.traveling() && c.travel_goal_xy() == Some(counter);
    // Following begins, the leader thirty metres the other way.
    let team = &mut c.autoplay.config.team;
    team.enabled = true;
    team.follow = true;
    c.autoplay.team.mates = vec![crate::autoplay::Mate {
        name: "Leader".into(),
        leader: true,
        leads: true,
        world: me - glam::Vec3::new(30.0, 0.0, 0.0),
        cell: holtburg,
        ..Default::default()
    }];
    c.tick_autoplay(now);
    assert!(bound_for(&c), "following pulled it off its run");
    // A fight breaks the walk off; once it is over the run walks on, taken up as it would be alone:
    // by "resume the journey", which steps aside for a leader only off a run of its own.
    c.remember_journey();
    c.interrupt_travel("a fight");
    let mut t = now + Duration::from_millis(100);
    c.tick_autoplay(t);
    assert_eq!(c.autoplay.step, Some("resume the journey"));
    for _ in 0..4 {
        t += Duration::from_millis(100);
        c.tick_autoplay(t);
    }
    assert!(bound_for(&c), "the run did not walk on after the fight");
    assert!(c.autoplay.growth.town_run_under_way());
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
