use std::time::{Duration, Instant};

use ac_world::object::MoveTarget;
use glam::Vec3;

use super::*;
use crate::testkit::{self, standing_at, ME};
use crate::walk_trial;

/// By the Holtburg lifestone, on the open ground.
fn at_holtburg() -> Client {
    let mut c = standing_at(0xA9B4_0019, Vec3::new(84.0, 7.1, 94.0));
    c.world.player_guid = Some(ME);
    c
}

/// The Tusker ground's floor, and a Female Tusker's ledge 6 m up the wall above it.
fn under_the_ledge() -> (Client, Place) {
    let t = walk_trial::named(
        20,
        "5C49 Tusker on a ledge over the room",
        Vec3::new(17_744.8, 13_910.7, 0.0),
        0x5C49_012A,
        Vec3::new(17_754.0, 13_929.0, 6.0),
    );
    let mut c = walk_trial::standing_at(20, &t);
    c.world.player_guid = Some(ME);
    let ledge = Place {
        at: t.goal,
        cell: t.goal_cell,
    };
    (c, ledge)
}

fn east(c: &Client, metres: f32) -> Goal {
    let me = c.my_position().expect("standing");
    Goal::At {
        place: c.place_at(me + Vec3::new(metres, 0.0, 0.0)),
        within: 1.0,
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_walk_with_no_way_is_blocked_on_its_first_ask() {
    let (mut c, ledge) = under_the_ledge();
    let now = Instant::now();
    let goal = Goal::At {
        place: ledge,
        within: 1.5,
    };
    let did = c.go(Owner::Hand, goal, now);
    let why = did.because().map(|b| b.what.clone()).unwrap_or_default();
    assert!(matches!(did, Did::Blocked(_)), "{did:?}");
    assert!(why.starts_with("no way there"), "{why}");
    // Asked again, it is told the same without planning again.
    let plans = c.pathfinder.plans;
    assert_eq!(c.go(Owner::Hand, goal, now), did);
    assert_eq!(c.pathfinder.plans, plans);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn asking_again_for_the_same_goal_plans_nothing() {
    let mut c = at_holtburg();
    let t0 = Instant::now();
    let goal = east(&c, 30.0);
    let plans = c.pathfinder.plans;
    for i in 0..10 {
        let now = t0 + Duration::from_millis(100 * i);
        assert_eq!(c.go(Owner::Hand, goal, now), Did::Acting);
    }
    assert_eq!(c.pathfinder.plans, plans + 1, "ten asks, one plan");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_walk_taken_by_another_owner_says_so_once() {
    let mut c = at_holtburg();
    let now = Instant::now();
    let (loot, fight) = (Owner::Step("loot"), Owner::Step("fight"));
    let (corpse, target) = (east(&c, 20.0), east(&c, -20.0));
    assert_eq!(c.go(loot, corpse, now), Did::Acting);
    assert_eq!(c.go(fight, target, now), Did::Acting);
    assert_eq!(c.going.as_ref().map(|g| g.owner), Some(fight));
    assert_eq!(c.go(loot, corpse, now), Did::waiting("taken by fight"));
    // Asked after that, it is a request like any other, and takes the walk back.
    assert_eq!(c.go(loot, corpse, now), Did::Acting);
    assert_eq!(c.going.as_ref().map(|g| g.owner), Some(loot));
    assert_eq!(c.go(fight, target, now), Did::waiting("taken by loot"));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_steps_walk_ends_the_tick_it_is_not_asked_for() {
    // A hunting field 40 m east: keeping to the area walks there, then the field is let go.
    let mut c = at_holtburg();
    let me = c.my_position().expect("standing");
    let (x, y) = (me.x + 40.0, me.y);
    c.autoplay.config.enabled = true;
    let fight = &mut c.autoplay.config.fight;
    fight.enabled = true;
    fight.area = Some(crate::hunt::HuntArea {
        name: "the field".into(),
        shape: crate::hunt::Shape::Outline {
            points: vec![[x, y], [x + 30.0, y], [x + 30.0, y + 30.0], [x, y + 30.0]],
        },
    });
    let dt = 0.05;
    let mut now = Instant::now();
    for _ in 0..20 {
        c.tick_autoplay(now);
        c.tick_player(crate::player::Input::default(), dt, now);
        now += Duration::from_secs_f32(dt);
    }
    let keep = Owner::Step("keep to the area");
    assert_eq!(c.going.as_ref().map(|g| g.owner), Some(keep));
    let walked = c.my_position().expect("standing");
    assert!(
        walked.distance(me) > 2.0,
        "walked {:.1} m",
        walked.distance(me)
    );
    // The step stops asking: its walk ends that tick, and the body stands the next frame.
    c.autoplay.config.fight.area = None;
    c.tick_autoplay(now);
    assert!(c.going.as_ref().is_none_or(|g| g.owner != keep));
    c.tick_player(crate::player::Input::default(), dt, now);
    assert!(c.walk_frame.is_none(), "still steered: {:?}", c.walk_frame);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_walk_without_progress_is_given_up_after_twenty_seconds() {
    // Planned, but the body is never moved along it.
    let mut c = at_holtburg();
    let t0 = Instant::now();
    let goal = east(&c, 30.0);
    assert_eq!(c.go(Owner::Hand, goal, t0), Did::Acting);
    let later = t0 + Duration::from_secs(10);
    assert_eq!(c.go(Owner::Hand, goal, later), Did::Acting);
    let past = t0 + GIVE_UP_AFTER + Duration::from_secs(1);
    let did = c.go(Owner::Hand, goal, past);
    assert_eq!(did, Did::blocked("no way there: no progress"));
    assert!(c.going_to().is_none(), "the body stands");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_goal_a_storey_up_is_not_arrived_at_from_below() {
    let mut c = at_holtburg();
    let now = Instant::now();
    let me = c.my_position().expect("standing");
    let above = Goal::At {
        place: c.place_at(me + Vec3::new(0.0, 0.0, 3.0)),
        within: 1.5,
    };
    assert_ne!(c.go(Owner::Hand, above, now), Did::Done);
    let here = Goal::At {
        place: c.place_at(me),
        within: 1.5,
    };
    assert_eq!(c.go(Owner::Hand, here, now), Did::Done);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_goal_out_of_the_region_is_a_journey() {
    // A corpse in the Holtburg Dungeon's rooms, from the lifestone: through the dungeon's portal.
    let mut c = at_holtburg();
    let now = Instant::now();
    let corpse = Place {
        at: Vec3::new(210.0, 47185.0, 0.0),
        cell: 0x01F6_0215,
    };
    assert!(!c.is_in_region(corpse));
    let goal = Goal::At {
        place: corpse,
        within: 1.0,
    };
    // A journey under way to somewhere else is not this one.
    assert!(c.travel_to(c.my_position().expect("standing").truncate() + glam::Vec2::X * 150.0));
    assert_eq!(c.go(Owner::Hand, goal, now), Did::Acting);
    assert!(c.traveling(), "a journey");
    assert_eq!(c.travel_goal_xy(), Some(corpse.at.truncate()));
    assert!(c.going.is_none(), "not a walk");
    // A goal on the next block is a walk.
    let next = c.place_at(c.my_position().expect("standing") + Vec3::new(200.0, 0.0, 0.0));
    assert!(c.is_in_region(next));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_refused_server_walk_is_let_go_at_once() {
    // The server walks the character at a Female Tusker on its ledge: no walk gets there.
    let (mut c, ledge) = under_the_ledge();
    let now = Instant::now();
    let guid = 0x8000_0101;
    let mut tusker = testkit::creature(guid, "Female Tusker");
    tusker.position = testkit::placed(ledge.cell, ledge.at);
    c.world.objects.insert(guid, tusker);
    c.move_to = Some(MoveTarget::Object(guid));
    c.move_to_since = now;
    c.tick_player(crate::player::Input::default(), 0.05, now);
    assert_eq!(c.move_to, None, "the server walk is let go");
    assert_eq!(c.server_walk_refused, Some(guid));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_scripts_walk_is_its_own_and_its_stop_ends_only_that() {
    let mut c = at_holtburg();
    let me = c.my_position().expect("standing");
    let there = me + Vec3::new(10.0, 0.0, 0.0);
    assert!(c.go_to(Owner::Script, there, 1.0).acting());
    assert_eq!(c.going.as_ref().map(|g| g.owner), Some(Owner::Script));
    c.stop_going(Some(Owner::Hand), "not the script's");
    assert!(
        c.going.is_some(),
        "another owner's stop ended the script's walk"
    );
    c.stop_going(Some(Owner::Script), "walk_stop");
    assert!(c.going.is_none());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_journey_set_off_frees_the_slot_for_its_leg() {
    // A script's walk arrived is kept in the slot until let go; a journey asked for takes the slot.
    let mut c = at_holtburg();
    let me = c.my_position().expect("standing");
    assert_eq!(c.go_to(Owner::Script, me, 1.0), Did::Done);
    assert!(c.travel_to(me.truncate() + glam::Vec2::new(150.0, 0.0)));
    assert!(c.going.is_none(), "the arrived walk held the journey back");
    let leg = c.walk_leg(Instant::now());
    assert!(leg.as_ref().is_some_and(Did::fine), "{leg:?}");
    assert_eq!(c.going.as_ref().map(|g| g.owner), Some(Owner::Journey));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_refusal_stands_while_another_goal_holds_the_slot() {
    // Each tick the salvager under the ledge is refused, and the fight's walk on the floor goes on.
    let (mut c, ledge) = under_the_ledge();
    let now = Instant::now();
    let (salvage, fight) = (Owner::Step("salvage"), Owner::Step("fight"));
    let up = Goal::At {
        place: ledge,
        within: 1.5,
    };
    let me = c.my_position().expect("standing");
    let place = c
        .standable_near(me, 8.0)
        .into_iter()
        .find(|p| p.at.distance(me) > 3.0)
        .expect("floor round the start");
    let floor = Goal::At { place, within: 1.0 };
    let mut plans = None;
    for tick in 0..6 {
        c.autoplay.tick = tick;
        assert!(matches!(c.go(salvage, up, now), Did::Blocked(_)));
        assert_eq!(c.go(fight, floor, now), Did::Acting, "tick {tick}");
        assert_eq!(c.going.as_ref().map(|g| g.owner), Some(fight));
        assert_eq!(*plans.get_or_insert(c.pathfinder.plans), c.pathfinder.plans);
    }
    // One owner asking for both, as the quartermaster's hand-out round does: planned once each.
    let qm = Owner::Step("quartermaster");
    for tick in 6..12 {
        c.autoplay.tick = tick;
        assert!(matches!(c.go(qm, up, now), Did::Blocked(_)));
        assert_eq!(c.go(qm, floor, now), Did::Acting, "tick {tick}");
    }
    assert_eq!(c.pathfinder.plans, plans.unwrap_or(0) + 2);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_walk_already_arrived_leaves_the_slot_alone() {
    // Back at the way in already: the explorer's walk is not taken, so it is not told so.
    let mut c = at_holtburg();
    let now = Instant::now();
    let (keep, explore) = (Owner::Step("keep to the area"), Owner::Step("explore"));
    let room = east(&c, 20.0);
    let here = east(&c, 0.0);
    for tick in 0..4 {
        c.autoplay.tick = tick;
        assert_eq!(c.go(keep, here, now), Did::Done);
        assert_eq!(c.go(explore, room, now), Did::Acting, "tick {tick}");
    }
    assert_eq!(c.going.as_ref().map(|g| g.owner), Some(explore));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_flown_walk_goes_straight_whatever_the_planner_says() {
    let (mut c, ledge) = under_the_ledge();
    if let Some(p) = c.player.as_mut() {
        p.noclip = true;
    }
    let goal = Goal::At {
        place: ledge,
        within: 1.5,
    };
    let later = Instant::now() + GIVE_UP_AFTER * 2;
    assert_eq!(c.go(Owner::Hand, goal, Instant::now()), Did::Acting);
    assert_eq!(c.go(Owner::Hand, goal, later), Did::Acting);
    assert_eq!(c.going_to().map(|(at, ..)| at), Some(ledge.at));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn an_arrived_walk_holds_no_journey_back() {
    // A script's walk 10 m east, arrived at and never let go; a journey planned from inside.
    let mut c = at_holtburg();
    let me = c.my_position().expect("standing");
    assert!(c.go_to(Owner::Script, me + Vec3::X * 10.0, 1.0).acting());
    testkit::stand(&mut c, 0xA9B4_0019, Vec3::new(94.0, 7.1, 94.0));
    let far = ac_world::Place::on_map(me.truncate() + glam::Vec2::X * 150.0);
    assert!(c.plan_trip(Goal::At {
        place: far,
        within: crate::travel::ARRIVE
    }));
    assert_eq!(c.going.as_ref().map(|g| g.owner), Some(Owner::Script));
    let leg = c.walk_leg(Instant::now());
    assert!(leg.as_ref().is_some_and(Did::fine), "{leg:?}");
    assert_eq!(c.going.as_ref().map(|g| g.owner), Some(Owner::Journey));
}
