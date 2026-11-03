use super::*;
use crate::autoplay::growth::tests::run_to;
use crate::testkit::{character_of_level, no_data, stand, thing_in_the_pack, with_a_pack};

const HOLTBURG: u32 = 0xA9B4_0019;

/// A Red Rat Tail hand-in to a giver standing in `cell`, worth `xp`.
fn tail_to(cell: u32, xp: u64) -> HandIn {
    let row = ac_world::quests::rows_of("give:3684:2470").next().unwrap();
    HandIn {
        row,
        at: Place {
            at: ac_world::landblock_origin(cell),
            cell,
        },
        xp,
        pursued: false,
    }
}

/// Legs between points on a line, a metre a second; a stop at `nowhere` has no way in.
fn legs_on_a_line(from: f32, calls: &[f32], end: f32, nowhere: Option<f32>) -> Legs {
    Legs::plan(from, calls, Some(end), |a, b| {
        (Some(b) != nowhere).then_some((a - b).abs())
    })
}

fn counter_in(cell: u32) -> Place {
    Place {
        at: ac_world::landblock_origin(cell),
        cell,
    }
}

#[test]
fn four_stops_are_made_in_the_fastest_order_and_a_giver_may_come_before_the_counter() {
    // The counter at 100, givers at 50 and 150 in its town, one at 120 in the next landblock but
    // a 20-second leg off; the ground at 200.
    let legs = legs_on_a_line(0.0, &[100.0, 50.0, 150.0, 120.0], 200.0, None);
    let hand_ins = [
        tail_to(HOLTBURG, 1_000),
        tail_to(0xA9B4_0163, 1_000),
        tail_to(0xAAB4_0001, 1_000),
    ];
    let planned = plan_calls(&legs, &[counter_in(HOLTBURG)], &hand_ins, None);
    assert_eq!(planned.order, [1, 0, 3, 2]);
    assert!(planned.refused.is_empty());
}

#[test]
fn a_far_giver_waits_for_a_run_that_passes_unless_its_xp_beats_the_detour() {
    // 9,600 seconds out of the way, for 10,000 XP.
    let legs = legs_on_a_line(0.0, &[100.0, 5_000.0], 200.0, None);
    let far = [tail_to(0x1234_0001, 10_000)];
    let counter = [counter_in(HOLTBURG)];
    let unmeasured = plan_calls(&legs, &counter, &far, None);
    assert_eq!(unmeasured.order, [0], "joined with no rate measured");
    assert!(unmeasured.refused.is_empty(), "waiting is not refused");
    assert_eq!(plan_calls(&legs, &counter, &far, Some(7_200.0)).order, [0]);
    assert_eq!(
        plan_calls(&legs, &counter, &far, Some(3_600.0)).order,
        [0, 1]
    );
}

#[test]
fn a_giver_a_long_way_out_is_not_on_the_round_however_cheap_the_way_back() {
    // From the counter 900 s out; from the giver anywhere 30 s, a recall.
    let legs = Legs::plan(0.0, &[100.0, 1_000.0], Some(200.0), |a: f32, b: f32| {
        Some(if a == 1_000.0 { 30.0 } else { (a - b).abs() })
    });
    let far = [tail_to(0x1234_0001, 1_000)];
    assert_eq!(
        plan_calls(&legs, &[counter_in(HOLTBURG)], &far, None).order,
        [0]
    );
}

#[test]
fn a_giver_no_journey_reaches_is_refused_when_the_run_is_planned() {
    let legs = legs_on_a_line(0.0, &[100.0, 90.0, 400.0], 200.0, Some(400.0));
    let hand_ins = [tail_to(HOLTBURG, 1_000), tail_to(0x1234_0001, 1_000_000)];
    let planned = plan_calls(&legs, &[counter_in(HOLTBURG)], &hand_ins, Some(1.0));
    assert_eq!(planned.refused, [1]);
    assert_eq!(planned.order, [1, 0], "the reachable giver still goes");
}

/// A character beside a Collector, `tails` Red Rat Tails in its pack, on a run handing them in.
fn at_a_collector(tails: u32, now: Instant) -> (Client, &'static Row) {
    let mut c = character_of_level(no_data(), 20);
    with_a_pack(&mut c, 50);
    stand(&mut c, HOLTBURG, glam::Vec3::new(84.0, 7.1, 94.0));
    let row = ac_world::quests::rows_of("give:3684:2470").next().unwrap();
    let giver = 0x7000_0001;
    let me = c.player.as_ref().unwrap().world_position();
    c.world.objects.insert(
        giver,
        ac_world::WorldObject {
            guid: giver,
            name: row.giver.to_string(),
            item_type: ac_world::item_type::CREATURE,
            position: Some(ac_world::object::Position::new_flat(
                HOLTBURG,
                me + glam::Vec3::X - ac_world::landblock_origin(HOLTBURG),
            )),
            ..Default::default()
        },
    );
    for k in 0..tails {
        thing_in_the_pack(&mut c, 0x8000_0100 + k, "Red Rat Tail", 1);
        c.world
            .objects
            .get_mut(&(0x8000_0100 + k))
            .unwrap()
            .weenie_class_id = 3684;
    }
    let at = row.places[0];
    let mut run = run_to(at.at.truncate(), now);
    run.phase = Phase::HandingIn {
        row,
        at,
        giver,
        taken: 0,
        last: None,
        stalls: 0,
    };
    c.autoplay.growth.run = Some(run);
    (c, row)
}

#[test]
fn a_collector_is_handed_every_tail_one_at_a_time_and_the_run_ends_when_none_is_left() {
    let now = Instant::now();
    let (mut c, _) = at_a_collector(2, now);
    let cfg = c.autoplay.config.growth.clone();
    let phase = |c: &Client| c.autoplay.growth.run.as_ref().map(|r| r.phase.clone());

    assert_eq!(c.grow_run_step(now, &cfg), Turn::Acted);
    assert!(
        matches!(
            phase(&c),
            Some(Phase::HandingIn {
                taken: 0,
                last: Some((_, 2)),
                ..
            })
        ),
        "{:?}",
        phase(&c)
    );
    assert!(c.quest_asked.is_some(), "the giver's answer is not watched");
    // Not taken yet: the next waits for it.
    assert_eq!(c.grow_run_step(now, &cfg), Turn::Waited);
    // The server took one: the second goes.
    c.world.objects.remove(&0x8000_0100);
    assert_eq!(c.grow_run_step(now, &cfg), Turn::Acted);
    assert!(
        matches!(
            phase(&c),
            Some(Phase::HandingIn {
                taken: 1,
                last: Some((_, 1)),
                ..
            })
        ),
        "{:?}",
        phase(&c)
    );
    c.world.objects.remove(&0x8000_0101);
    assert_eq!(c.grow_run_step(now, &cfg), Turn::Over);
    assert!(c.autoplay.growth.run.is_none());
    assert!(
        c.autoplay.growth.run_was_futile,
        "handing in sells and buys nothing: the next run waits as after any futile one"
    );
}

#[test]
fn a_giver_that_never_takes_the_item_is_not_called_on_again_for_a_while() {
    let now = Instant::now();
    let (mut c, row) = at_a_collector(1, now);
    let cfg = c.autoplay.config.growth.clone();
    let mut t = now;
    while c.autoplay.growth.run.is_some() && t < now + Duration::from_secs(60) {
        c.grow_run_step(t, &cfg);
        t += GIVE_PAUSE;
    }
    assert!(c.autoplay.growth.run.is_none(), "the call never ended");
    let key = (row.giver_wcid, row.places[0].cell);
    assert!(c
        .autoplay
        .growth
        .givers_refused
        .within(&key, t, GIVER_RETRY));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_giver_in_a_dungeon_is_priced_and_called_on_with_its_cell() {
    // A Warden in the Facility Hub, from the Holtburg lifestone.
    let mut c =
        crate::testkit::standing_in_the_field(25, HOLTBURG, glam::Vec3::new(84.0, 7.1, 94.0));
    let row = ac_world::quests::rows_of("fachubfolthid")
        .find(|r| r.kind == Kind::Item)
        .unwrap();
    let at = row.places[0];
    assert!(at.is_indoors());
    let legs = c.plan_legs(&[at]).unwrap();
    assert!(
        legs.leg(Stop::Start, Stop::Call(0)).is_some(),
        "no journey to the Warden"
    );
    let hand_in = [HandIn {
        row,
        at,
        xp: 100_000,
        pursued: false,
    }];
    assert_eq!(plan_calls(&legs, &[], &hand_in, Some(1.0)).order, [0]);
    let now = Instant::now();
    let mut run = run_to(Vec2::new(32_587.2, 34_578.3), now);
    c.autoplay.running = Some("town run");
    assert!(c.make_call(&mut run, giver_call(row, at), now));
    c.autoplay.running = None;
    assert_eq!(run.phase, Phase::ToGiver { row, at, tries: 1 });
    let goal = c.journey_goal().and_then(crate::Goal::place);
    assert_eq!(goal.map(|p| p.cell), Some(at.cell), "{goal:?}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_ready_talk_in_the_counters_town_joins_the_run_and_one_switched_off_does_not() {
    // Alcott stands in Holtburg, where the run's counter is.
    let mut c =
        crate::testkit::standing_in_the_field(5, HOLTBURG, glam::Vec3::new(84.0, 7.1, 94.0));
    c.autoplay.config.quests.on = true;
    let line = crate::quest_log::QuestLine {
        id: "tothebarkeep".into(),
        title: "Sent to the Barkeep".into(),
        kind: Kind::Talk,
        state: QuestState::Ready,
        xp: 7_000,
        givers: 1,
        giver: "Alcott".into(),
        on: true,
        every_way: false,
    };
    c.quest_board = vec![line.clone()];
    let now = Instant::now();
    let renald = Vec2::new(32_587.2, 34_578.3);
    c.autoplay.growth.run = Some(run_to(renald, now));
    c.autoplay.running = Some("town run");
    c.add_hand_ins(now);
    let run = c.autoplay.growth.run.clone().expect("the run");
    let givers: Vec<&str> = std::iter::once(&run.vendor)
        .map(String::as_str)
        .chain(run.calls.iter().filter_map(|k| match &k.on {
            CallOn::Giver { row } => Some(row.giver),
            CallOn::Counter { .. } => None,
        }))
        .filter(|n| *n == "Alcott")
        .collect();
    assert_eq!(givers, ["Alcott"], "{run:?}");
    // Switched off, it is not on the board as ready, and the run is the counter's alone.
    c.quest_board = vec![crate::quest_log::QuestLine {
        on: false,
        state: QuestState::Off,
        ..line
    }];
    c.autoplay.growth.run = Some(run_to(renald, now));
    c.add_hand_ins(now);
    c.autoplay.running = None;
    assert!(c.autoplay.growth.run.as_ref().unwrap().calls.is_empty());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_quest_not_taken_up_is_a_talk_to_whoever_starts_it_and_once_heard_it_is_begun() {
    // Lieutenant Grenlin starts the Bronze Gauntlet Trooper kill count: a talk that pays nothing
    // and stamps its counter, from level 30. Not taken up, the kills never count.
    let cell = 0x8755_0016;
    let mut c =
        crate::testkit::standing_in_the_field(35, cell, glam::Vec3::new(60.0, 124.0, 10.01));
    c.autoplay.config.quests.on = true;
    c.quest_board = vec![crate::quest_log::QuestLine {
        id: "bronzegauntlettrooperkilltask".into(),
        title: "Bronze Gauntlet Trooper kill task".into(),
        kind: Kind::Kill,
        state: QuestState::NotTaken {
            from: "Lieutenant Grenlin".into(),
            flag: "bronzegauntlettrooperkillcount".into(),
        },
        xp: 20_000,
        givers: 1,
        giver: "Lieutenant Grenlin".into(),
        on: true,
        every_way: false,
    }];
    let now = Instant::now();
    let me = c.my_position().unwrap().truncate();
    let calls = c.ready_hand_ins(&[me], 5, now);
    let HandIn { row: start, at, .. } = calls.first().expect("a talk to take it up").clone();
    assert_eq!(
        (start.kind, start.giver),
        (Kind::Start, "Lieutenant Grenlin")
    );
    assert_eq!(calls[0].xp, 20_000);
    // Givers share names: a talk is the one stamping the quest's flag, else none.
    if let QuestState::NotTaken { flag, .. } = &mut c.quest_board[0].state {
        *flag = "nosuchflag".into();
    }
    assert!(c.ready_hand_ins(&[me], 5, now).is_empty());
    // Switched off, nothing is offered.
    c.quest_board[0].on = false;
    assert!(c.ready_hand_ins(&[me], 5, now).is_empty());
    // Heard with no XP: the counter is held, and the giver is not taken for refusing.
    c.ask_giver(start, at, now);
    c.watch_quest_answer(now + Duration::from_secs(11));
    assert!(c.holds_quest_flag("bronzegauntlettrooperkillcount"));
    // Its kills count from nothing: the board hunts it, and does not send it back to talk.
    let counter = "bronzegauntlettrooperkillcount";
    assert_eq!(c.quest_log.kills.get(counter), Some(&0));
    let mut facts = c.quest_facts();
    facts.level = 80; // the Troopers' own, so only the counter decides
    let board = crate::quest_log::quest_board(ac_world::quests::rows_of(counter), &facts);
    assert!(
        matches!(board[0].state, QuestState::Needs { .. }),
        "{:?}",
        board[0].state
    );
    let key = (start.giver_wcid, at.cell);
    assert!(!c
        .autoplay
        .growth
        .givers_refused
        .within(&key, now, GIVER_RETRY));
}

#[test]
fn a_jaw_for_a_quest_paying_no_xp_is_handed_in_only_once_chosen() {
    // Fiun Rehlyun (Fiun Outpost, outdoors) takes the Insatiable Eater Jaw for a flag, no XP.
    let id = "augmentationprovinggroundaccess";
    let fiun = ac_world::quests::rows_of(id)
        .find(|r| r.kind == Kind::Item)
        .unwrap();
    let at = fiun.places[0];
    let mut c = character_of_level(no_data(), 150);
    with_a_pack(&mut c, 50);
    stand(&mut c, at.cell, at.at - ac_world::landblock_origin(at.cell));
    thing_in_the_pack(&mut c, 0x8000_0100, "Insatiable Eater Jaw", 1);
    c.world
        .objects
        .get_mut(&0x8000_0100)
        .unwrap()
        .weenie_class_id = 28719;
    c.autoplay.config.quests.on = true;
    let now = Instant::now();
    let me = c.my_position().unwrap().truncate();
    let to_fiun = |c: &Client| {
        let calls = c.ready_hand_ins(&[me], 5, now);
        calls.iter().filter(|h| h.row.id == id).count()
    };
    c.refresh_quests(now);
    assert_eq!(to_fiun(&c), 0, "never offered unchosen");
    c.autoplay.config.quests.switch_on(id);
    c.refresh_quests(now + Duration::from_millis(1));
    assert_eq!(to_fiun(&c), 1, "{:?}", c.quest_board);
}

#[test]
fn a_pursued_hand_in_rides_an_ordinary_run() {
    // 9,600 seconds out of the way for no XP: a pursued quest's giver is called on all the same.
    let legs = legs_on_a_line(0.0, &[100.0, 5_000.0], 200.0, None);
    let mut far = [tail_to(0x1234_0001, 0)];
    let counter = [counter_in(HOLTBURG)];
    assert_eq!(plan_calls(&legs, &counter, &far, Some(3_600.0)).order, [0]);
    far[0].pursued = true;
    assert_eq!(
        plan_calls(&legs, &counter, &far, Some(3_600.0)).order,
        [0, 1]
    );
}

/// Beside Clemenza with the Engorged Eater Jaw, its quest pursued and town runs off.
fn beside_clemenza_with_the_jaw() -> (Client, &'static Row) {
    let row = ac_world::quests::rows_of("engorgedeaterjaw")
        .next()
        .unwrap();
    let at = row.places[0];
    let mut c = character_of_level(no_data(), 85);
    with_a_pack(&mut c, 50);
    stand(&mut c, at.cell, at.at - ac_world::landblock_origin(at.cell));
    thing_in_the_pack(&mut c, 0x8000_0100, "Engorged Eater Jaw", 1);
    c.world
        .objects
        .get_mut(&0x8000_0100)
        .unwrap()
        .weenie_class_id = 28725;
    c.autoplay.config.growth.town_runs = false;
    c.autoplay.config.quests.pursue_quest(row.id, true, 0);
    c.pursuit = vec![crate::autoplay::PursuitLine {
        id: row.id.into(),
        title: row.title.into(),
        step: crate::autoplay::PursuitStep::HandIn { row, at },
        working: true,
        via: None,
    }];
    (c, row)
}

#[test]
fn a_pursued_jaw_carried_starts_a_call_on_its_giver_at_once() {
    let (mut c, row) = beside_clemenza_with_the_jaw();
    let now = Instant::now();
    // A futile run just now would hold an ordinary run back five minutes.
    c.autoplay.growth.last_run = Some(now);
    c.autoplay.growth.run_was_futile = true;
    c.autoplay.running = Some("town run");
    assert!(c.autoplay_town_run(now), "{}", c.autoplay.status);
    c.autoplay.running = None;
    let run = c.autoplay.growth.run.clone().expect("a call on Clemenza");
    assert!(run.for_quest);
    assert!(
        matches!(run.phase, Phase::ToGiver { row: r, .. } if std::ptr::eq(r, row)),
        "{:?}",
        run.phase
    );
    assert_eq!(run.reason, "for Engorged Eater Jaw");
}

#[test]
fn a_quest_call_is_kept_with_town_runs_off_and_is_not_futile() {
    let (mut c, row) = beside_clemenza_with_the_jaw();
    let now = Instant::now();
    assert!(c.start_quest_call(row, row.places[0], now));
    assert!(c.autoplay.growth.is_quest_run());
    // Town runs off: an ordinary run is let go, this one kept.
    assert!(c
        .autoplay
        .growth
        .drop_what_is_turned_off(false, false)
        .is_none());
    assert!(c.autoplay.growth.run.is_some());
    // Over with nothing sold or bought, the next run need not wait.
    let run = c.autoplay.growth.run.take().unwrap();
    let cfg = c.autoplay.config.growth.clone();
    c.end_run(run, now, &cfg, &[], false, None);
    assert!(!c.autoplay.growth.run_was_futile);
}
