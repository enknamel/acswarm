use glam::{Vec2, Vec3};

use super::*;
use crate::autoplay::Mate;
use crate::testkit;

/// The leader's own guid.
const LEADER: u32 = 0x5000_0002;
/// The Holtburg field, where every character here stands.
const HOLTBURG: u32 = 0xA9B4_0019;

#[test]
fn following_rows_are_rows_of_the_table() {
    for row in FOLLOW_ROWS {
        assert!(crate::steps::named(row).is_some(), "{row} is no STEPS row");
    }
}

#[test]
fn a_follower_strays_no_further_than_twice_its_distance() {
    assert_eq!(follow_break(4.0), 10.0);
    assert_eq!(follow_break(8.0), 16.0);
}

/// A leader called Verity, `away` metres east of the character.
fn leader_off(c: &Client, away: f32) -> Mate {
    let me = c.my_position().expect("on its feet");
    Mate {
        health: 1.0,
        world: me + Vec3::new(away, 0.0, 0.0),
        cell: HOLTBURG,
        leader: true,
        leads: true,
        ..testkit::mate(LEADER, "Verity")
    }
}

/// A level-20 character playing on its own in the Holtburg field, following a leader `away`
/// metres east.
fn following(away: f32) -> Client {
    following_over(testkit::no_data(), away)
}

/// [`following`] over `assets`: the game's, for a walk the region planner must hold.
fn following_over(assets: std::rc::Rc<ac_scene::Assets>, away: f32) -> Client {
    let mut c = testkit::character_of_level(assets, 20);
    testkit::stand(&mut c, HOLTBURG, Vec3::new(84.0, 84.0, 94.0));
    c.autoplay.config.enabled = true;
    let team = &mut c.autoplay.config.team;
    team.enabled = true;
    team.follow = true;
    team.lead = false;
    let leader = leader_off(&c, away);
    c.autoplay.team = testkit::view_of(vec![leader]);
    c
}

/// Where the leader stands on the map.
fn leader_xy(c: &Client) -> Vec2 {
    c.autoplay.team.mates[0].world.truncate()
}

/// Ticks of the rules, a tenth of a second apart from `now`.
fn ticks(c: &mut Client, now: Instant, n: u32) -> Instant {
    let mut t = now;
    for _ in 0..n {
        t += Duration::from_millis(100);
        c.tick_autoplay(t);
    }
    t
}

/// Whether the character still makes for `at`: a walk there, or a journey there under way.
fn makes_for(c: &Client, at: Vec2) -> bool {
    let walk = c
        .going
        .as_ref()
        .and_then(|g| g.goal.place())
        .is_some_and(|p| p.at.truncate().distance(at) < 0.5);
    let trip = c.traveling() && c.travel_goal_xy().is_some_and(|g| g.distance(at) < 0.5);
    walk || trip
}

#[test]
fn stopping_following_ends_the_walk_after_the_leader() {
    let mut c = following(30.0);
    let now = Instant::now();
    c.tick_autoplay(now);
    let at = leader_xy(&c);
    assert!(makes_for(&c, at), "it never set off after its leader");
    c.autoplay.config.team.follow = false;
    ticks(&mut c, now, 5);
    assert!(c.going.is_none(), "it walked on after it was stopped");
    assert!(!c.traveling());
}

#[test]
fn stopping_following_ends_the_journey_after_the_leader() {
    let mut c = following(400.0);
    let now = Instant::now();
    c.tick_autoplay(now);
    let at = leader_xy(&c);
    assert!(c.traveling(), "no journey after a leader 400 m off");
    assert!(makes_for(&c, at));
    c.autoplay.config.team.follow = false;
    ticks(&mut c, now, 5);
    assert!(!c.traveling(), "the journey after the leader went on");
    assert!(!makes_for(&c, at));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_stop_leaves_no_old_aim_behind_when_a_fight_broke_the_journey_off() {
    // The round-3 failure. A fight breaks the journey after a far leader off and puts it aside for
    // "resume the journey"; following, above that row, re-aims at the leader's new spot; the
    // player stops following. The copy put aside named where the leader had been, a stop keyed
    // on the new aim missed it, and the character set off for the old spot.
    let mut c = following_over(testkit::game_data(), 400.0);
    let now = Instant::now();
    c.tick_autoplay(now);
    let first = leader_xy(&c);
    assert!(c.traveling(), "no journey after a leader 400 m off");
    // A fight breaks it off, as `autoplay_fight_as` does.
    c.remember_journey();
    c.interrupt_travel("a fight");
    assert_eq!(
        c.autoplay.resume_trip, None,
        "following's own journey was put aside to be picked up again"
    );
    // The leader comes back nearer, and following re-aims at it.
    let me = c.my_position().expect("on its feet");
    c.autoplay.team.mates[0].world = me + Vec3::new(30.0, 0.0, 0.0);
    let t = ticks(&mut c, now, 1);
    assert!(makes_for(&c, leader_xy(&c)), "following did not re-aim");
    // Stopped: nothing makes for either spot, then or later.
    c.autoplay.config.team.follow = false;
    let t = ticks(&mut c, t, 1);
    assert!(
        c.going.is_none() && !c.traveling(),
        "it went on after the stop"
    );
    ticks(&mut c, t, 20);
    assert!(
        !makes_for(&c, first),
        "it set off for where the leader had been"
    );
    assert!(c.going.is_none() && !c.traveling());
}

#[test]
fn a_stop_with_autoplay_off_still_lets_the_leader_go() {
    // Played by hand the team rules still follow (`autoplay_by_hand`), and a stop reaches them.
    for away in [30.0, 400.0] {
        let mut c = following(away);
        c.autoplay.config.enabled = false;
        let now = Instant::now();
        c.tick_autoplay(now);
        let at = leader_xy(&c);
        assert!(makes_for(&c, at), "not following by hand at {away} m");
        c.autoplay.config.team.follow = false;
        ticks(&mut c, now, 3);
        assert!(!makes_for(&c, at), "it went on after the stop at {away} m");
    }
}

#[test]
fn every_way_following_ends_lets_the_leader_go() {
    // No switch but the follow switch is a stop's own, and the board ends it with no switch at all.
    for why in [
        "team off",
        "lead on",
        "made leader",
        "leader stops leading",
        "leader gone",
    ] {
        for away in [30.0, 400.0] {
            let mut c = following(away);
            let now = Instant::now();
            c.tick_autoplay(now);
            let at = leader_xy(&c);
            assert!(makes_for(&c, at), "not following at {away} m ({why})");
            match why {
                "team off" => c.autoplay.config.team.enabled = false,
                "lead on" => c.autoplay.config.team.lead = true,
                "made leader" => c.autoplay.team.leader = true,
                "leader stops leading" => c.autoplay.team.mates[0].leads = false,
                _ => c.autoplay.team.mates.clear(),
            }
            ticks(&mut c, now, 3);
            assert!(!makes_for(&c, at), "{why}: it went on at {away} m");
        }
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_walk_another_goal_planted_after_following_outlives_the_stop() {
    // The slot is shared: looting, visits and town runs walk in it too. Following walked first,
    // and a walk by another owner took the slot from it; a stop ends following's only.
    let mut c = following_over(testkit::game_data(), 30.0);
    c.autoplay.config.enabled = false;
    let now = Instant::now();
    c.tick_autoplay(now);
    assert!(
        matches!(
            c.going.as_ref().map(|g| g.owner),
            Some(crate::Owner::Step(_))
        ),
        "following walked nowhere"
    );
    let me = c.my_position().expect("on its feet");
    let corpse = me + Vec3::new(0.0, 12.0, 0.0);
    let goal = crate::Goal::At {
        place: c.place_at(corpse),
        within: 1.0,
    };
    assert!(c.go(crate::Owner::Hand, goal, now).acting());
    c.autoplay.config.team.follow = false;
    c.tick_autoplay(now + Duration::from_millis(100));
    assert!(
        c.going.as_ref().and_then(|g| g.goal.place()).map(|p| p.at) == Some(corpse),
        "the stop took the walk to the corpse"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_leader_the_region_holds_is_walked_to_however_far() {
    // 150 m east, over the next block: a walk of following's own, not a journey.
    let mut c = following_over(testkit::game_data(), 150.0);
    c.autoplay.team.mates[0].cell = 0;
    c.tick_autoplay(Instant::now());
    assert!(
        matches!(
            c.going.as_ref().map(|g| g.owner),
            Some(crate::Owner::Step(row)) if FOLLOW_ROWS.contains(&row)
        ),
        "not walked: {:?}",
        c.going.as_ref().map(|g| g.owner)
    );
    assert!(!c.traveling(), "a journey after a leader the region holds");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_follower_with_no_walk_to_its_leader_is_refused_a_journey_too() {
    // The leader up on the Tusker ground's ledge (0x5C49), which no walk reaches: the follower is
    // told so on its first ask, and the journey it tries instead is refused as it sets off, its
    // first walk planned whole. Nothing walks at the wall. (Flying, the ledge is flown to.)
    let (mut c, ledge) = testkit::under_the_ledge(20);
    c.movement_rules = crate::player::MovementRules::ServerSafe;
    c.autoplay.config.enabled = true;
    let team = &mut c.autoplay.config.team;
    team.enabled = true;
    team.follow = true;
    team.lead = false;
    c.autoplay.team = testkit::view_of(vec![Mate {
        health: 1.0,
        world: ledge.at,
        cell: ledge.cell,
        leader: true,
        leads: true,
        ..testkit::mate(LEADER, "Verity")
    }]);
    let now = Instant::now();
    assert!(!c.autoplay_follow(now, false), "it walks at the leader");
    assert!(!c.is_walking(), "the slot walks at the ledge");
    assert!(!c.traveling(), "a journey at the ledge");
    // Asked again, both refusals stand: nothing is planned.
    let plans = c.pathfinder.plans;
    assert!(!c.autoplay_follow(now + Duration::from_millis(100), false));
    assert!(!c.traveling());
    assert_eq!(c.pathfinder.plans, plans, "the walk was planned again");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_journey_carried_by_another_row_is_claimed() {
    // Catch up sets off after a leader 400 m off; the follow row, running next, takes the journey
    // over, and its stretches are walked for "follow".
    let mut c = following_over(testkit::game_data(), 400.0);
    c.autoplay.running = Some("catch up");
    assert!(c.autoplay_follow(Instant::now(), true), "no catching up");
    assert_eq!(c.journey_row(), Some("catch up"));
    c.autoplay.running = Some("follow");
    assert!(c.autoplay_follow(Instant::now(), false), "no following");
    c.autoplay.running = None;
    assert_eq!(c.journey_row(), Some("follow"), "not claimed");
    let start = c.my_position().expect("on its feet");
    let t0 = Instant::now();
    for frame in 0..100 {
        let now = t0 + Duration::from_millis(50 * frame);
        c.tick_player(crate::player::Input::default(), 0.05, now);
    }
    let stretch = c.going.as_ref().filter(|g| g.journey).map(|g| g.owner);
    assert_eq!(stretch, Some(crate::Owner::Step("follow")));
    let moved = c.my_position().expect("on its feet").distance(start);
    assert!(moved > 10.0, "walked {moved:.1} m in 5 s");
}

#[test]
fn a_journey_another_goal_planned_after_following_outlives_the_stop() {
    // A town run's journey, say, planned over the one after the leader.
    let mut c = following(400.0);
    c.autoplay.config.enabled = false;
    let now = Instant::now();
    c.tick_autoplay(now);
    assert!(c.is_follow_journey(), "following planned no journey");
    let me = c.my_position().expect("on its feet").truncate();
    let counter = me + Vec2::new(0.0, 300.0);
    // Under way whatever the planner answers: offline it cannot route the first walk.
    c.travel_to(counter);
    assert!(makes_for(&c, counter), "no journey to the counter");
    c.autoplay.config.team.follow = false;
    c.tick_autoplay(now + Duration::from_millis(100));
    assert_eq!(c.autoplay.follow_trip, None, "the record outlived the stop");
    assert!(
        makes_for(&c, counter),
        "the stop took the journey to the counter"
    );
}

#[test]
fn a_flying_leader_past_a_walk_is_followed_by_a_journey_a_stop_ends() {
    // The slot plans a journey of its own past the region: following's for a stop.
    let mut c = following(400.0);
    c.autoplay.team.mates[0].flying = true;
    let now = Instant::now();
    c.tick_autoplay(now);
    assert!(
        c.is_follow_journey(),
        "the journey after a flying leader is not on the record"
    );
    c.autoplay.config.team.follow = false;
    ticks(&mut c, now, 2);
    assert!(!c.traveling(), "it flew on after the stop");
}

#[test]
fn a_death_on_the_way_after_the_leader_keeps_no_journey_for_afterwards() {
    // The recovery puts the journey under way at death aside to resume once it is done; following's
    // own is planned again by following, and kept it outlived a stop the same way.
    let mut c = following(400.0);
    let now = Instant::now();
    c.tick_autoplay(now);
    assert!(c.is_follow_journey(), "following planned no journey");
    // Dead, with an Endurance to draw a maximum from.
    c.world.stats.name = "Bryn".into();
    c.world.stats.attributes[1].base = 100;
    c.world.stats.vitals[0].current = 0;
    c.tick_autoplay(now + Duration::from_millis(100));
    assert_eq!(c.autoplay.step, Some("recover"));
    assert_eq!(
        c.autoplay.recovery.trip, None,
        "the journey after the leader was kept"
    );
}

#[test]
fn a_follower_does_not_walk_back_into_its_area_away_from_its_leader() {
    // Going with the leader: the hunting area limits what a follower fights, not where it goes, so
    // "keep to the area" (70) no longer outranks "follow" (50).
    let mut c = following(8.0);
    let me = c.my_position().expect("on its feet");
    c.autoplay.config.fight.area = Some(crate::hunt::HuntArea {
        name: "the north field".into(),
        shape: crate::hunt::Shape::Outline {
            points: vec![
                [me.x - 20.0, me.y + 100.0],
                [me.x + 20.0, me.y + 100.0],
                [me.x + 20.0, me.y + 140.0],
                [me.x - 20.0, me.y + 140.0],
            ],
        },
    });
    let now = Instant::now();
    c.tick_autoplay(now);
    assert_eq!(c.autoplay.step, Some("follow"));
    assert!(makes_for(&c, leader_xy(&c)), "it made for its area");
    // Beside its leader, it stands there.
    c.autoplay.team.mates[0].world = me + Vec3::new(2.0, 0.0, 0.0);
    ticks(&mut c, now, 3);
    assert!(
        c.going.is_none() && !c.traveling(),
        "it walked off to its area"
    );
}

/// A level-20 follower in the Holtburg field over the game data, hunting the Holtburg Dungeon with
/// town runs off; its leader stands in `cell`.
fn hunting_the_dungeon_led_from(cell: u32) -> Client {
    let mut c = testkit::character_of_level(testkit::game_data(), 20);
    testkit::stand(&mut c, HOLTBURG, Vec3::new(84.0, 84.0, 94.0));
    c.autoplay.config.enabled = true;
    c.autoplay.config.growth.town_runs = false;
    c.autoplay.config.fight.area = Some(crate::hunt::HuntArea {
        name: "the Holtburg Dungeon".into(),
        shape: crate::hunt::Shape::Dungeon {
            landblock: 0x01F6_0000,
            rooms: Vec::new(),
        },
    });
    let team = &mut c.autoplay.config.team;
    team.enabled = true;
    team.follow = true;
    let mut leader = leader_off(&c, 0.0);
    leader.cell = cell;
    leader.world = ac_world::landblock_origin(cell) + Vec3::new(96.7, -10.0, 0.0);
    c.autoplay.team = testkit::view_of(vec![leader]);
    c
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_follower_goes_into_its_dungeon_after_a_leader_already_in_it() {
    // A party hunting a dungeon gets its followers in by "keep to the area" (70): following waits
    // outside a dungeon the leader stands in, so with the area stepped aside nobody went in.
    let mut c = hunting_the_dungeon_led_from(0x01F6_0289);
    let mut t = Instant::now();
    for n in 0..6 {
        t = ticks(&mut c, t, 1);
        assert_eq!(c.autoplay.step, Some("keep to the area"), "tick {n}");
    }
    assert_eq!(
        c.visiting(),
        Some("Holtburg Dungeon"),
        "the follower stayed outside its dungeon"
    );
    // A leader in some other dungeon is waited for outside, as before: the area does not lead.
    let mut c = hunting_the_dungeon_led_from(0x01F7_0105);
    ticks(&mut c, Instant::now(), 3);
    assert_ne!(c.autoplay.step, Some("keep to the area"));
    assert_eq!(c.visiting(), None, "it went into its own dungeon alone");
}

#[test]
fn a_journey_put_aside_before_following_is_not_taken_up_after_a_stop() {
    // A journey a fight broke off while the character played alone, then following switched on:
    // "follow" (50) has every tick while the leader is eight metres off, so "resume the journey"
    // (40) never sees the copy -- and after a stop it would have set off on it.
    let mut c = following(8.0);
    c.autoplay.config.team.enabled = false;
    let me = c.my_position().expect("on its feet").truncate();
    let away = me + Vec2::new(0.0, 300.0);
    c.travel_to(away);
    c.remember_journey();
    c.interrupt_travel("a fight");
    assert_eq!(
        c.autoplay.resume_trip.and_then(crate::Goal::place),
        Some(ac_world::Place::on_map(away)),
        "nothing was put aside"
    );
    c.autoplay.config.team.enabled = true;
    let now = Instant::now();
    let t = ticks(&mut c, now, 3);
    assert_eq!(c.autoplay.step, Some("follow"));
    assert_eq!(c.autoplay.resume_trip, None, "kept through following");
    c.autoplay.config.team.follow = false;
    ticks(&mut c, t, 5);
    assert!(
        !makes_for(&c, away),
        "it set off on the journey from before"
    );
}

#[test]
fn a_follower_takes_up_no_journey_of_its_own_beside_its_leader() {
    // One left from before it was led -- its own town run's, say -- is let go by "resume the
    // journey", not walked off on and not kept for after a stop.
    let mut c = following(2.0);
    let now = Instant::now();
    let t = ticks(&mut c, now, 1);
    let me = c.my_position().expect("on its feet").truncate();
    let away = me + Vec2::new(0.0, 300.0);
    c.autoplay.resume_trip = Some(crate::Goal::At {
        place: ac_world::Place::on_map(away),
        within: crate::travel::ARRIVE,
    });
    ticks(&mut c, t, 3);
    assert!(!makes_for(&c, away), "it set off on a journey of its own");
    assert_eq!(c.autoplay.resume_trip, None);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_journey_broken_off_while_following_is_not_taken_up_after_a_stop() {
    // Nothing is put aside while led: a journey another planted -- a script's, say -- that a fight
    // breaks off while "follow" (50) has every tick would be walked off on once following stopped.
    let mut c = following_over(testkit::game_data(), 8.0);
    let t = ticks(&mut c, Instant::now(), 1);
    let me = c.my_position().expect("on its feet").truncate();
    let away = me + Vec2::new(0.0, 300.0);
    c.travel_to(away);
    assert!(makes_for(&c, away), "no journey to put aside");
    c.remember_journey();
    c.interrupt_travel("a fight");
    assert_eq!(c.autoplay.resume_trip, None, "put aside while led");
    let t = ticks(&mut c, t, 3);
    assert_eq!(c.autoplay.step, Some("follow"));
    c.autoplay.config.team.follow = false;
    ticks(&mut c, t, 5);
    assert!(
        !makes_for(&c, away),
        "it set off on the journey after the stop"
    );
}

#[test]
fn a_follower_back_from_the_dead_takes_up_no_trip_it_had() {
    // The recovery hands back the trip under way at death once it is done. Led, none is taken up,
    // and none is left for "resume the journey" to walk off on after a stop.
    let mut c = following(8.0);
    let t = ticks(&mut c, Instant::now(), 1);
    let me = c.my_position().expect("on its feet").truncate();
    let away = me + Vec2::new(0.0, 300.0);
    // Landed at the lifestone with no death spot to go back to: the next step finishes.
    let rec = &mut c.autoplay.recovery;
    rec.phase = crate::recovery::Phase::Landed;
    rec.since = t;
    rec.trip = Some(crate::Goal::At {
        place: ac_world::Place::on_map(away),
        within: crate::travel::ARRIVE,
    });
    let t = ticks(&mut c, t + Duration::from_secs(5), 1);
    assert!(!c.autoplay.recovery.active(), "the recovery did not finish");
    assert_eq!(c.autoplay.resume_trip, None, "the trip was handed back");
    let t = ticks(&mut c, t, 3);
    c.autoplay.config.team.follow = false;
    ticks(&mut c, t, 5);
    assert!(
        !makes_for(&c, away),
        "it set off on the trip from before its death"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_follower_in_a_dungeon_keeps_to_its_leader_rather_than_exploring() {
    // Where the Holtburg Dungeon's portal drops a character, with rooms to walk: alone it explores,
    // following it stays beside its leader.
    let mut c = testkit::standing_at(0x01F6_0289, Vec3::new(96.7, -10.0, 0.0));
    c.world.player_guid = Some(testkit::ME);
    c.world.stats.level = 10;
    c.autoplay.config.enabled = true;
    let now = Instant::now();
    c.tick_autoplay(now);
    assert_eq!(c.autoplay.step, Some("explore"), "alone it did not explore");
    let mut c = testkit::standing_at(0x01F6_0289, Vec3::new(96.7, -10.0, 0.0));
    c.world.player_guid = Some(testkit::ME);
    c.world.stats.level = 10;
    c.autoplay.config.enabled = true;
    let team = &mut c.autoplay.config.team;
    team.enabled = true;
    team.follow = true;
    let mut leader = leader_off(&c, 2.0);
    leader.cell = 0x01F6_0289;
    c.autoplay.team = testkit::view_of(vec![leader]);
    ticks(&mut c, now, 3);
    assert_ne!(c.autoplay.step, Some("explore"));
    assert!(
        c.going.is_none() && !c.traveling(),
        "it walked off to a room"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_leader_on_the_floor_above_is_walked_up_to_not_stood_under() {
    // The jeweler's deck at F682 stands 11.5 m over the ground below it, the same spot on the map:
    // a leader up there is not beside a follower standing under it.
    let t = crate::walk_trial::named(
        20,
        "under the F682 deck",
        Vec3::new(47_313.9, 24_988.56, 51.93),
        0xF682_001A,
        Vec3::new(47_313.904, 24_988.559, 63.4465),
    );
    let mut c = crate::walk_trial::standing_at(20, &t);
    c.autoplay.config.enabled = true;
    let team = &mut c.autoplay.config.team;
    team.enabled = true;
    team.follow = true;
    team.lead = false;
    let leader = Mate {
        health: 1.0,
        world: t.goal,
        cell: t.goal_cell,
        leader: true,
        leads: true,
        ..testkit::mate(LEADER, "Verity")
    };
    c.autoplay.team = testkit::view_of(vec![leader]);
    let t0 = Instant::now();
    c.tick_autoplay(t0);
    assert!(c.going.is_some(), "it stood under its leader");
    let dt = Duration::from_millis(50);
    let mut now = t0;
    // Ticked a while past arriving: it stays up beside its leader.
    while now < t0 + Duration::from_secs(40) {
        now += dt;
        c.tick_autoplay(now);
        c.tick_player(crate::player::Input::default(), dt.as_secs_f32(), now);
        c.pathfinder.settle(Duration::from_secs(5));
    }
    let me = c.my_position().expect("on its feet");
    assert!(
        crate::body::server_walk::reached(me, t.goal, 4.0),
        "not up beside its leader: at {me}, the leader at {}",
        t.goal
    );
}
