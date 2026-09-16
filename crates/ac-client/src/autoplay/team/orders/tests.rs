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
    ap.corpse_seen.push((body, t0));
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
