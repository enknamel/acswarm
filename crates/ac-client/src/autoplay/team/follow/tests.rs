use glam::{Vec2, Vec3};

use super::*;
use crate::autoplay::Mate;
use crate::testkit;

/// The leader's own guid.
const LEADER: u32 = 0x5000_0002;
/// The Holtburg field, where every character here stands.
const HOLTBURG: u32 = 0xA9B4_0019;

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
    let mut c = testkit::character_of_level(testkit::no_data(), 20);
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
        .follow
        .is_some_and(|f| f.target.truncate().distance(at) < 0.5);
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
    assert!(c.follow.is_none(), "it walked on after it was stopped");
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
fn a_stop_leaves_no_old_aim_behind_when_a_fight_broke_the_journey_off() {
    // The round-3 failure. A fight breaks the journey after a far leader off and puts it aside for
    // "resume the journey"; following, above that row, re-aims at the leader's new spot; the
    // player stops following. The copy put aside named where the leader had been, a stop keyed
    // on the new aim missed it, and the character set off for the old spot.
    let mut c = following(400.0);
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
        c.follow.is_none() && !c.traveling(),
        "it went on after the stop"
    );
    ticks(&mut c, t, 20);
    assert!(
        !makes_for(&c, first),
        "it set off for where the leader had been"
    );
    assert!(c.follow.is_none() && !c.traveling());
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
fn a_walk_another_goal_planted_after_following_outlives_the_stop() {
    // The walk is shared: looting, visits and town runs plant it too. Following planted first,
    // so the stop has a record to act on, and a corpse walk replaced it.
    let mut c = following(30.0);
    c.autoplay.config.enabled = false;
    let now = Instant::now();
    c.tick_autoplay(now);
    assert!(
        c.autoplay.follow_walk.is_some(),
        "following recorded no walk"
    );
    let me = c.my_position().expect("on its feet");
    let corpse = me + Vec3::new(0.0, 12.0, 0.0);
    assert!(c.head_for(corpse, 1.0, "a corpse").acting());
    c.autoplay.config.team.follow = false;
    c.tick_autoplay(now + Duration::from_millis(100));
    assert_eq!(c.autoplay.follow_walk, None, "the record outlived the stop");
    assert!(
        c.follow.is_some_and(|f| f.target == corpse),
        "the stop took the walk to the corpse"
    );
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
    // `head_for` plans a journey of its own past `WALKABLE`, which no record named.
    let mut c = following(WALKABLE + 50.0);
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
