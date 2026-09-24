use std::time::{Duration, Instant};

use super::*;
use crate::autoplay::CLAIM_SETTLE;
use crate::testkit::{looter, view_of};

#[test]
fn a_body_the_plan_deals_is_the_dealt_ones_while_the_plan_is_fresh_and_from_the_leader() {
    use crate::plan::{Order, Plan, ORDERS_LAST};
    let t0 = Instant::now();
    let (body, at) = (0x8000_0001, glam::Vec3::ZERO);
    let (me, other) = (2, 3);
    let mut ap = Autoplay::default();
    ap.config.enabled = true;
    ap.config.team.enabled = true;
    let mut leader = looter(1, at, None, Duration::ZERO);
    leader.leader = true;
    ap.team = view_of(vec![leader, looter(other, at, None, Duration::ZERO)]);
    ap.team.me = Some(looter(me, at, None, Duration::ZERO));
    ap.corpse_seen.mark(body, t0);
    let plan = |to: u32| {
        let mut p = Plan {
            leader: "Bryn01".into(),
            ..Default::default()
        };
        p.orders.insert(
            to,
            Order {
                target: None,
                body: Some(body),
            },
        );
        p
    };
    // Dealt to another: left to it, and the log says whose it is.
    ap.take_orders(plan(other), t0);
    assert!(!ap.ours_to_open(body, at, me, at, t0));
    assert_eq!(ap.whose_turn(body, at, me, at, t0), Some("Bryn03"));
    // Still the other's once the turns would have let anyone free
    // take it (see `CLAIM_SETTLE`): the deal is the leader's word.
    let settled = t0 + CLAIM_SETTLE * 2;
    assert!(!ap.ours_to_open(body, at, me, at, settled));
    // Dealt to this character: its own.
    ap.take_orders(plan(me), t0);
    assert!(ap.ours_to_open(body, at, me, at, t0));
    // The leader gone quiet: the plan lapses and the turns say again.
    ap.take_orders(plan(other), t0);
    let late = t0 + ORDERS_LAST + CLAIM_SETTLE * 2;
    assert!(
        ap.ours_to_open(body, at, me, at, late),
        "a stale plan held a body"
    );
    // A plan signed by one not leading is nobody's to obey.
    let mut theirs = plan(other);
    theirs.leader = "Bryn03".into();
    ap.take_orders(theirs, late);
    assert!(ap.ours_to_open(body, at, me, at, late));
    // And a mate's standing claim on the body outranks a deal to us:
    // the deal is a plan, the claim is a body already in hand.
    ap.take_orders(plan(me), late);
    ap.team.mates[1].looting = Some(body);
    ap.team.mates[1].looting_for = Duration::from_secs(2);
    assert!(!ap.ours_to_open(body, at, me, at, late));
    // Off the team, no plan is obeyed at all.
    ap.team.mates[1].looting = None;
    ap.config.team.enabled = false;
    assert_eq!(ap.body_dealt_to(body, late), None);
}

#[test]
fn a_leader_holds_the_next_fight_for_a_body_it_dealt_to_another_while_it_lies_there() {
    use crate::plan::{Order, Plan, ORDERS_LAST};
    let t0 = Instant::now();
    let (me, other) = (1, 2);
    let (body, gone) = (0x8000_0001, 0x8000_0002);
    let mut ap = Autoplay::default();
    ap.config.enabled = true;
    ap.config.team.enabled = true;
    ap.team = view_of(vec![looter(other, glam::Vec3::ZERO, None, Duration::ZERO)]);
    ap.team.leader = true;
    ap.team.me = Some(looter(me, glam::Vec3::ZERO, None, Duration::ZERO));
    // Nothing dealt: nothing held.
    assert_eq!(ap.body_dealt_to_another(me, t0, |_| true), None);
    // A plan dealing one body to the other, one to this character.
    let mut plan = Plan {
        leader: "Bryn01".into(),
        ..Default::default()
    };
    plan.orders.insert(
        other,
        Order {
            target: None,
            body: Some(body),
        },
    );
    plan.orders.insert(
        me,
        Order {
            target: None,
            body: Some(gone),
        },
    );
    ap.planner.dealt(
        &crate::plan::Dealt {
            deals: vec![(body, other), (gone, me)],
            lapsed: Vec::new(),
        },
        t0,
        DEAL_WINDOW,
    );
    ap.take_orders(plan, t0);
    assert_eq!(
        ap.body_dealt_to_another(me, t0, |_| true),
        Some((body, other)),
        "the other's body holds the leader; its own is its own to loot"
    );
    // Gone from the ground (rotted, or out of reach): nothing to hold for.
    assert_eq!(ap.body_dealt_to_another(me, t0, |g| g != body), None);
    // Said emptied by the one it went to: done with.
    ap.shut_by.insert((body, other, 0));
    assert_eq!(ap.body_dealt_to_another(me, t0, |_| true), None);
    ap.shut_by.clear();
    // A leader that has stopped planning holds nothing.
    assert_eq!(
        ap.body_dealt_to_another(me, t0 + ORDERS_LAST, |_| true),
        None
    );
}

/// The leader's own guid.
const LEADER: u32 = 0x5000_0002;

/// A level-20 character playing on its own in the Holtburg field, fighting beside a leader called
/// Verity three metres off, with focus fire on.
fn beside_the_leader() -> Client {
    let mut c = crate::testkit::character_of_level(crate::testkit::no_data(), 20);
    crate::testkit::stand(&mut c, 0xA9B4_0019, glam::Vec3::new(84.0, 84.0, 94.0));
    c.autoplay.config.enabled = true;
    let team = &mut c.autoplay.config.team;
    team.enabled = true;
    team.follow = true;
    team.focus_fire = true;
    let me = c.my_position().expect("on its feet");
    let mut leader = crate::testkit::mate(LEADER, "Verity");
    leader.leader = true;
    leader.leads = true;
    leader.world = me + glam::vec3(3.0, 0.0, 0.0);
    c.autoplay.team = view_of(vec![leader]);
    c
}

/// Verity's plan, heard at `now`, ordering this character onto `target`.
fn ordered_onto(c: &mut Client, target: u32, now: Instant) {
    use crate::plan::{Order, Plan};
    let mut plan = Plan {
        leader: "Verity".into(),
        ..Default::default()
    };
    let me = c.world.player_guid.expect("a guid");
    plan.orders.insert(
        me,
        Order {
            target: Some(target),
            body: None,
        },
    );
    c.autoplay.take_orders(plan, now);
}

#[test]
fn neither_the_order_nor_the_board_puts_a_follower_on_a_creature_it_avoids() {
    // The round-3 failure: the order's check refused the avoided creature, and the fight fell
    // straight back to the leader's own target on the board -- in focus fire the same creature --
    // asked only whether it was alive and off the road.
    let now = Instant::now();
    let mut c = beside_the_leader();
    let avoided = crate::testkit::standing_by(&mut c, 0x8000_0001, "Drudge Slinker", 4.0).guid;
    let other = crate::testkit::standing_by(&mut c, 0x8000_0002, "Revenant", 8.0).guid;
    let mut cfg = c.autoplay.config.fight.clone();
    cfg.avoid = vec!["Slinker".into()];
    c.autoplay.config.fight = cfg.clone();
    c.autoplay.team.mates[0].target = Some(avoided);
    // The board alone, then the order and the board together.
    assert!(c.autoplay_fight_as(now, &cfg));
    assert_eq!(c.attack_target, Some(other), "the board's target was taken");
    let mut c2 = beside_the_leader();
    crate::testkit::standing_by(&mut c2, avoided, "Drudge Slinker", 4.0);
    crate::testkit::standing_by(&mut c2, other, "Revenant", 8.0);
    c2.autoplay.config.fight = cfg.clone();
    c2.autoplay.team.mates[0].target = Some(avoided);
    ordered_onto(&mut c2, avoided, now);
    assert!(c2.autoplay_fight_as(now, &cfg));
    assert_eq!(
        c2.attack_target,
        Some(other),
        "the ordered target was taken"
    );
    // And the order does not pull it off the fight it picked instead.
    let underground = c2.underground();
    assert!(!c2.ordered_elsewhere(other, &cfg, underground, now));
}

#[test]
fn an_order_outside_the_hunting_area_is_not_taken_and_dropped_each_tick() {
    // Taken, the order's creature was let go again the next tick by `fight_target_gone` (outside
    // the area), and taken again, for as long as the plan named it.
    let now = Instant::now();
    let mut c = beside_the_leader();
    let me = c.my_position().expect("on its feet");
    c.autoplay.config.fight.area = Some(crate::hunt::HuntArea {
        name: "test".into(),
        shape: crate::hunt::Shape::Outline {
            points: vec![
                [me.x - 15.0, me.y - 15.0],
                [me.x + 15.0, me.y - 15.0],
                [me.x + 15.0, me.y + 15.0],
                [me.x - 15.0, me.y + 15.0],
            ],
        },
    });
    let outside = crate::testkit::standing_by(&mut c, 0x8000_0001, "Revenant", 20.0).guid;
    let inside = crate::testkit::standing_by(&mut c, 0x8000_0002, "Revenant", 6.0).guid;
    assert!(c.fight_target_gone(outside, false), "not outside the area");
    ordered_onto(&mut c, outside, now);
    c.autoplay.team.mates[0].target = Some(outside);
    let cfg = c.autoplay.config.fight.clone();
    assert!(c.autoplay_fight_as(now, &cfg));
    assert_eq!(
        c.attack_target,
        Some(inside),
        "the order out there was taken"
    );
    // The fight in hand stands, tick after tick, with no swing sent at it again.
    assert!(!c.ordered_elsewhere(inside, &cfg, false, now));
    let sent = c.session.actions_sent();
    for n in 1..4 {
        let t = now + Duration::from_secs(2 * n);
        assert!(c.autoplay_fight_as(t, &cfg));
        assert_eq!(c.attack_target, Some(inside), "let go at tick {n}");
    }
    assert_eq!(c.session.actions_sent(), sent, "let go and taken again");
}

#[test]
fn a_creature_given_up_on_is_not_taken_again_off_the_board() {
    let now = Instant::now();
    let mut c = beside_the_leader();
    let stuck = crate::testkit::standing_by(&mut c, 0x8000_0001, "Revenant", 4.0).guid;
    let other = crate::testkit::standing_by(&mut c, 0x8000_0002, "Revenant", 8.0).guid;
    c.autoplay.team.mates[0].target = Some(stuck);
    let cfg = c.autoplay.config.fight.clone();
    c.give_up_target(stuck, "no damage in a while", now);
    assert!(c.autoplay_fight_as(now, &cfg));
    assert_eq!(
        c.attack_target,
        Some(other),
        "the given-up one was taken again"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_caster_takes_no_order_on_a_creature_it_avoids() {
    // The spell path's own join, through the same gate as the swing's.
    let now = Instant::now();
    let (mut c, _) = crate::testkit::a_caster_knowing(now, &["Flame Bolt I"]);
    c.autoplay.config.enabled = true;
    let team = &mut c.autoplay.config.team;
    team.enabled = true;
    team.follow = true;
    team.focus_fire = true;
    let me = c.my_position().expect("on its feet");
    let mut leader = crate::testkit::mate(LEADER, "Verity");
    leader.leader = true;
    leader.leads = true;
    leader.world = me + glam::vec3(3.0, 0.0, 0.0);
    c.autoplay.team = view_of(vec![leader]);
    let avoided = crate::testkit::standing_by(&mut c, 0x8000_0001, "Drudge Slinker", 6.0).guid;
    let mut cfg = c.autoplay.config.fight.clone();
    cfg.avoid = vec!["Slinker".into()];
    ordered_onto(&mut c, avoided, now);
    c.autoplay_fight_as(now, &cfg);
    assert_eq!(
        c.autoplay.casting_at(),
        None,
        "a spell went at the avoided one"
    );
}
