//! The fellowship's planner: one plan for the party, made by its leader
//! and handed to each of the others as an order.
//!
//! Until now every character judged the world alone and read what the
//! others said about themselves. That gave the party focus fire, turns
//! at the bodies and a walk to town together, but it gave the party no
//! plan: six characters joined their leader on one Drudge while three
//! more hit the followers from behind, and each body fell to whichever
//! of them was free first. The leader can see the whole field on the
//! board -- where each of its own stands, what is hitting whom, what
//! lies on the ground -- and it is the one session that has the same
//! view the others have, so it plans (see `Client::plan_for_team`) and
//! the plan goes out on the board (`ac_plugin::team::PLAN_TOPIC`).
//!
//! Everything here is arithmetic over plain state, so it can be run in
//! a test with no server and no clock. Three decisions:
//!
//! - [`assign_targets`]: who fights what. A creature after one of the
//!   party is that one's fight; a hard creature is everyone's; the rest
//!   are spread so that no creature has more hands on it than its share
//!   while another has none, and a hand already on a creature stays on
//!   it while there is room, so a plan does not move people about for
//!   nothing.
//! - [`deal_bodies`]: whose turn each body is. One body a turn, round
//!   the party, to the free hand with the fewest turns lately, nearest
//!   first. What a skill rule leaves on a body for the best at it is
//!   still routed as it was (`Shut::left_for`): the deal only covers
//!   bodies nobody has opened yet.
//! - [`stragglers`]: whom the leader waits for before it moves the
//!   party on: a follower still fighting, or too far behind.
//!
//! An order is obeyed while it is fresh ([`ORDERS_LAST`]) and falls
//! away by itself: a leader that goes quiet leaves no order standing,
//! and every session falls back to judging for itself, which is what it
//! did before there was a plan. The group's own state machine (the
//! roster, the leader, the restocking) is untouched: the plan reads it
//! and never drives it.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// How long an order is good for after it was heard. The leader plans
/// every board round (half a second, `ac_plugin::team::SAY_EVERY`), so
/// this is a few rounds' grace for a word lost on the bus and no more:
/// a leader that has gone quiet leaves nobody under stale orders.
pub const ORDERS_LAST: Duration = Duration::from_millis(2500);

/// A body dealt to a character that has not gone for it in this long is
/// dealt to another. The turn was a plan, not a claim, and a character
/// that stays in a fight rather than come for its body has not lost the
/// fellowship the body.
pub const DEAL_PATIENCE: Duration = Duration::from_secs(8);

/// A follower more than this many follow distances from its leader is
/// left behind, and the leader waits for it before moving the party on.
pub const STRAGGLE_TIMES: f32 = 4.0;

/// The longest the leader waits for stragglers before it moves on
/// regardless. One stuck behind a wall, or dead across the field, is not
/// waited for all afternoon: the following brings it along.
pub const STRAGGLE_PATIENCE: Duration = Duration::from_secs(45);

/// A character as the planner reads it: a pair of hands.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hand {
    pub guid: u32,
    pub name: String,
    pub world: Vec3,
    pub health: f32,
    /// It fights on its own: autoplay on, alive, in the world.
    pub fights: bool,
    /// What it is fighting, alive, if anything.
    pub target: Option<u32>,
    /// The body it has open or is walking to.
    pub looting: Option<u32>,
    /// It would open a body at all, and has room for what is on one
    /// (see `Mate::turn`).
    pub opens: bool,
    /// It follows the leader about, so it is waited for on a move.
    pub following: bool,
    /// Bodies dealt to it lately: its turns.
    pub turns: u16,
    /// The names of what has attacked it lately.
    pub hit_by: Vec<String>,
}

/// A creature the party would fight.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Foe {
    pub guid: u32,
    pub world: Vec3,
    /// A hard fight (`Team::hard_fight_health`): everyone's.
    pub hard: bool,
    /// The hands it is after, by player guid: those it has walked at
    /// and those it has hit lately (see [`hunted_by`]).
    pub after: Vec<u32>,
}

/// A body on the ground nobody has opened.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub guid: u32,
    pub world: Vec3,
}

/// A deal still standing from an earlier plan: the body, whom it went
/// to, and how long ago.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Standing {
    pub body: u32,
    pub to: u32,
    pub age: Duration,
}

/// One character's orders.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Order {
    /// What to fight.
    pub target: Option<u32>,
    /// The body to open.
    pub body: Option<u32>,
}

/// The plan, as it goes out on the board.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Plan {
    /// Who made it. A session takes orders from its own leader's plan
    /// and nobody else's.
    pub leader: String,
    /// Which plan this is, counting up: a newer one replaces an older.
    pub n: u32,
    /// Each character's orders, by player guid.
    pub orders: BTreeMap<u32, Order>,
    /// Whom the leader is waiting for before it moves the party on.
    pub waiting_for: Vec<String>,
}

impl Plan {
    /// The orders for the character `guid`.
    pub fn order_for(&self, guid: u32) -> Option<Order> {
        self.orders.get(&guid).copied()
    }

    /// Whom the body `body` is dealt to, if anyone.
    pub fn body_dealt_to(&self, body: u32) -> Option<u32> {
        self.orders
            .iter()
            .find(|(_, o)| o.body == Some(body))
            .map(|(who, _)| *who)
    }
}

/// A plan as one session holds it, and when it was heard.
#[derive(Clone, Debug, PartialEq)]
pub struct Orders {
    pub plan: Plan,
    pub heard: Instant,
}

impl Orders {
    /// The plan while it is still fresh (see [`ORDERS_LAST`]).
    pub fn current(&self, now: Instant) -> Option<&Plan> {
        (now.saturating_duration_since(self.heard) < ORDERS_LAST).then_some(&self.plan)
    }
}

/// What the leader remembers between plans: the deals it has made and
/// the turns each character has had.
#[derive(Clone, Debug, Default)]
pub struct Planner {
    /// The bodies dealt, to whom, and when: `(body, to, since)`.
    pub deals: Vec<(u32, u32, Instant)>,
    /// A turn a character was dealt, and when: what makes the deal go
    /// round.
    pub turns: Vec<(u32, Instant)>,
    /// How many plans have been made.
    pub n: u32,
}

impl Planner {
    /// The deals as they stand at `now`, for the next plan.
    pub fn standing(&self, now: Instant) -> Vec<Standing> {
        self.deals
            .iter()
            .map(|(body, to, since)| Standing {
                body: *body,
                to: *to,
                age: now.saturating_duration_since(*since),
            })
            .collect()
    }

    /// How many turns `who` has had within `window`.
    pub fn turns_of(&self, who: u32, now: Instant, window: Duration) -> u16 {
        let n = self
            .turns
            .iter()
            .filter(|(to, t)| *to == who && now.saturating_duration_since(*t) < window)
            .count();
        u16::try_from(n).unwrap_or(u16::MAX)
    }

    /// Take in the deals of the plan just made: a deal already standing
    /// keeps its clock, a new one starts it and is a turn.
    pub fn dealt(&mut self, deals: &[(u32, u32)], now: Instant, window: Duration) {
        let old = std::mem::take(&mut self.deals);
        self.deals = deals
            .iter()
            .map(|&(body, to)| {
                let since = old
                    .iter()
                    .find(|(b, t, _)| *b == body && *t == to)
                    .map_or(now, |(_, _, since)| *since);
                if since == now {
                    self.turns.push((to, now));
                }
                (body, to, since)
            })
            .collect();
        self.turns
            .retain(|(_, t)| now.saturating_duration_since(*t) < window);
    }
}

/// The creatures `hand` is fighting off, by what has hit it lately: for
/// each name in its `hit_by`, the nearest creature of that name to it.
/// The server names an attacker and no more, so this is the best the
/// leader can do for a mate; a creature the server has walked at a
/// player says so itself (`WorldObject::walked_at`).
pub fn hunted_by(foes: &[(u32, &str, Vec3)], hand: &Hand) -> Vec<u32> {
    let mut out: Vec<u32> = hand
        .hit_by
        .iter()
        .filter_map(|name| {
            foes.iter()
                .filter(|(_, n, _)| n == name)
                .min_by(|a, b| {
                    a.2.distance(hand.world)
                        .total_cmp(&b.2.distance(hand.world))
                        .then(a.0.cmp(&b.0))
                })
                .map(|(g, _, _)| *g)
        })
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Who fights what: each fighting hand's target, by player guid.
///
/// In order: a creature after one of the party is fought by that one,
/// whatever else the plan says, because that fight is already
/// happening. A hard creature is everyone's, the way focus fire has
/// always had it. Then the rest are shared out: no creature gets more
/// hands than its share (the hands over the creatures, rounded up)
/// while another has none. A hand already on a creature keeps it while
/// its share allows, lowest guid first, so as few move as need to; a
/// creature nobody is on gets the nearest free hand, the ones after the
/// most people first; and what is still free goes to the nearest
/// creature with room.
///
/// `prior` is the last plan's answer: what a hand was told to fight is
/// what it keeps, ahead of what its row on the board says it is
/// fighting, because the row is up to a board round behind the order
/// and read off the row two hands moved between two creatures swapped
/// back every round.
///
/// The same input gives the same answer whatever order it came in, so
/// the leader's plan is one every session could have reached.
pub fn assign_targets(
    hands: &[Hand],
    foes: &[Foe],
    prior: &BTreeMap<u32, u32>,
) -> BTreeMap<u32, u32> {
    let mut out = BTreeMap::new();
    let mut fighters: Vec<&Hand> = hands.iter().filter(|h| h.fights && h.guid != 0).collect();
    fighters.sort_by_key(|h| h.guid);
    if fighters.is_empty() || foes.is_empty() {
        return out;
    }
    let mut foes: Vec<&Foe> = foes.iter().collect();
    foes.sort_by_key(|f| f.guid);
    let centre = fighters.iter().map(|h| h.world).sum::<Vec3>() / fighters.len() as f32;
    // A hard fight is everyone's.
    if let Some(hard) = foes.iter().filter(|f| f.hard).min_by(|a, b| {
        a.world
            .distance(centre)
            .total_cmp(&b.world.distance(centre))
            .then(a.guid.cmp(&b.guid))
    }) {
        for h in &fighters {
            out.insert(h.guid, hard.guid);
        }
        return out;
    }
    let share = fighters.len().div_ceil(foes.len());
    let mut on: BTreeMap<u32, usize> = foes.iter().map(|f| (f.guid, 0)).collect();
    fn put(out: &mut BTreeMap<u32, u32>, on: &mut BTreeMap<u32, usize>, hand: u32, foe: u32) {
        out.insert(hand, foe);
        *on.entry(foe).or_default() += 1;
    }
    // What is after somebody is that somebody's fight.
    for f in &foes {
        for &who in &f.after {
            if fighters.iter().any(|h| h.guid == who) && !out.contains_key(&who) {
                put(&mut out, &mut on, who, f.guid);
            }
        }
    }
    // A hand on something keeps it while its share allows: what it was
    // last told to fight first, else what its row says it is fighting.
    for h in &fighters {
        if out.contains_key(&h.guid) {
            continue;
        }
        let room = |t: &u32| on.get(t).is_some_and(|n| *n < share);
        let keep = prior
            .get(&h.guid)
            .copied()
            .filter(room)
            .or_else(|| h.target.filter(room));
        if let Some(t) = keep {
            put(&mut out, &mut on, h.guid, t);
        }
    }
    // Every creature gets a hand before any gets a second: the ones
    // after the most people first, then the nearest to the party.
    let mut untended: Vec<&Foe> = foes.iter().copied().filter(|f| on[&f.guid] == 0).collect();
    untended.sort_by(|a, b| {
        b.after
            .len()
            .cmp(&a.after.len())
            .then(
                a.world
                    .distance(centre)
                    .total_cmp(&b.world.distance(centre)),
            )
            .then(a.guid.cmp(&b.guid))
    });
    for f in untended {
        let nearest = fighters
            .iter()
            .filter(|h| !out.contains_key(&h.guid))
            .min_by(|a, b| {
                a.world
                    .distance(f.world)
                    .total_cmp(&b.world.distance(f.world))
                    .then(a.guid.cmp(&b.guid))
            })
            .map(|h| h.guid);
        match nearest {
            Some(h) => put(&mut out, &mut on, h, f.guid),
            None => break,
        }
    }
    // The rest go to the nearest creature with room.
    for h in &fighters {
        if out.contains_key(&h.guid) {
            continue;
        }
        let room = foes
            .iter()
            .filter(|f| on[&f.guid] < share)
            .min_by(|a, b| {
                a.world
                    .distance(h.world)
                    .total_cmp(&b.world.distance(h.world))
                    .then(a.guid.cmp(&b.guid))
            })
            .or_else(|| {
                // Every creature is at its share: the nearest, then.
                foes.iter().min_by(|a, b| {
                    a.world
                        .distance(h.world)
                        .total_cmp(&b.world.distance(h.world))
                        .then(a.guid.cmp(&b.guid))
                })
            })
            .map(|f| f.guid);
        if let Some(f) = room {
            put(&mut out, &mut on, h.guid, f);
        }
    }
    out
}

/// Whose turn each body is: `(body, player guid)`.
///
/// A deal standing from the last plan stands while the body lies there
/// and the one it went to is still about and either at it or not yet
/// past its patience ([`DEAL_PATIENCE`]). Each body not dealt goes to a
/// free hand within `reach` of it -- one that opens bodies, has room,
/// is at no body and holds no other deal -- the ones not fighting
/// first, then the fewest turns lately, then the nearest, then the
/// lowest guid. A hand is dealt one body a plan. A body no hand can be
/// dealt is left undealt, and the sessions judge it as they did before
/// there was a plan.
pub fn deal_bodies(
    hands: &[Hand],
    bodies: &[Body],
    standing: &[Standing],
    reach: impl Fn(Vec3, Vec3) -> bool,
) -> Vec<(u32, u32)> {
    let lying: BTreeSet<u32> = bodies.iter().map(|b| b.guid).collect();
    let mut deals: Vec<(u32, u32)> = standing
        .iter()
        .filter(|s| lying.contains(&s.body))
        .filter(|s| {
            hands.iter().any(|h| {
                h.guid == s.to && h.opens && (h.looting == Some(s.body) || s.age < DEAL_PATIENCE)
            })
        })
        .map(|s| (s.body, s.to))
        .collect();
    deals.sort_unstable();
    deals.dedup_by_key(|(body, _)| *body);
    let mut held: BTreeSet<u32> = deals.iter().map(|(_, to)| *to).collect();
    let mut bodies: Vec<&Body> = bodies
        .iter()
        .filter(|b| !deals.iter().any(|(body, _)| *body == b.guid))
        .collect();
    bodies.sort_by_key(|b| b.guid);
    for b in bodies {
        let pick = hands
            .iter()
            .filter(|h| h.guid != 0 && h.opens && h.looting.is_none() && !held.contains(&h.guid))
            .filter(|h| reach(h.world, b.world))
            .min_by(|x, y| {
                x.target
                    .is_some()
                    .cmp(&y.target.is_some())
                    .then(x.turns.cmp(&y.turns))
                    .then(
                        x.world
                            .distance(b.world)
                            .total_cmp(&y.world.distance(b.world)),
                    )
                    .then(x.guid.cmp(&y.guid))
            });
        if let Some(h) = pick {
            held.insert(h.guid);
            deals.push((b.guid, h.guid));
        }
    }
    deals.sort_unstable();
    deals
}

/// Why a follower is being waited for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Behind {
    /// Still fighting something.
    Fighting,
    /// This far from the leader, metres.
    Off(f32),
}

/// A follower the leader waits for.
#[derive(Clone, Debug, PartialEq)]
pub struct Straggler {
    pub name: String,
    pub why: Behind,
}

/// The followers the leader, standing at `leader` and keeping them to
/// `keep` metres, waits for before moving the party on: one still
/// fighting, or further off than [`STRAGGLE_TIMES`] times `keep`. Only
/// those alive and following; one hunting on its own, or dead across
/// the field, is not waited for.
pub fn stragglers(leader: Vec3, keep: f32, hands: &[Hand]) -> Vec<Straggler> {
    let too_far = keep.max(1.5) * STRAGGLE_TIMES;
    let mut out: Vec<Straggler> = hands
        .iter()
        .filter(|h| h.following && h.health > 0.0)
        .filter_map(|h| {
            if h.target.is_some() {
                return Some(Straggler {
                    name: h.name.clone(),
                    why: Behind::Fighting,
                });
            }
            let flat = h.world.truncate().distance(leader.truncate());
            (flat > too_far).then(|| Straggler {
                name: h.name.clone(),
                why: Behind::Off(flat),
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// The status line for a leader waiting: "waiting for Brynne (fighting),
/// Brynno (42 m off)".
pub fn waiting_line(behind: &[Straggler]) -> String {
    let who: Vec<String> = behind
        .iter()
        .map(|s| match s.why {
            Behind::Fighting => format!("{} (fighting)", s.name),
            Behind::Off(m) => format!("{} ({} m off)", s.name, m.round()),
        })
        .collect();
    format!("waiting for {}", who.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hand(guid: u32, x: f32) -> Hand {
        Hand {
            guid,
            name: format!("Bryn{guid:02}"),
            world: Vec3::new(x, 0.0, 0.0),
            health: 1.0,
            fights: true,
            opens: true,
            following: guid != 1,
            ..Default::default()
        }
    }

    fn foe(guid: u32, x: f32) -> Foe {
        Foe {
            guid,
            world: Vec3::new(x, 5.0, 0.0),
            ..Default::default()
        }
    }

    fn on(plan: &BTreeMap<u32, u32>, foe: u32) -> Vec<u32> {
        plan.iter()
            .filter(|(_, f)| **f == foe)
            .map(|(h, _)| *h)
            .collect()
    }

    #[test]
    fn six_hands_and_two_creatures_are_three_and_three_not_six_and_none() {
        // The bug: everyone joined the leader on one creature while the
        // other stood hitting a follower.
        let hands: Vec<Hand> = (1..=6).map(|g| hand(g, g as f32)).collect();
        let foes = [foe(0x77, 1.0), foe(0x78, 6.0)];
        let plan = assign_targets(&hands, &foes, &BTreeMap::new());
        assert_eq!(plan.len(), 6, "everyone has a fight");
        assert_eq!(on(&plan, 0x77).len(), 3);
        assert_eq!(on(&plan, 0x78).len(), 3);
        // By distance: the low guids stand near the first creature.
        assert_eq!(on(&plan, 0x77), vec![1, 2, 3]);
    }

    #[test]
    fn a_creature_after_somebody_is_that_somebodys_fight() {
        let hands: Vec<Hand> = (1..=3).map(|g| hand(g, g as f32)).collect();
        let mut chaser = foe(0x78, 3.0);
        chaser.after = vec![1];
        let foes = [foe(0x77, 1.0), chaser];
        let plan = assign_targets(&hands, &foes, &BTreeMap::new());
        assert_eq!(plan[&1], 0x78, "it is being hit by 0x78, so it fights 0x78");
        // The other creature is not left alone for it.
        assert!(!on(&plan, 0x77).is_empty());
        // And more hands than the share still go on what is hitting them.
        let mut mob = foe(0x79, 2.0);
        mob.after = vec![1, 2, 3];
        let plan = assign_targets(&hands, &[foe(0x77, 1.0), mob], &BTreeMap::new());
        assert_eq!(on(&plan, 0x79), vec![1, 2, 3]);
    }

    #[test]
    fn a_hard_creature_is_everyones() {
        let hands: Vec<Hand> = (1..=4).map(|g| hand(g, g as f32)).collect();
        let mut boss = foe(0x80, 20.0);
        boss.hard = true;
        let foes = [foe(0x77, 1.0), boss, foe(0x78, 2.0)];
        let plan = assign_targets(&hands, &foes, &BTreeMap::new());
        assert_eq!(on(&plan, 0x80), vec![1, 2, 3, 4]);
    }

    #[test]
    fn a_hand_keeps_what_it_is_on_while_there_is_room_and_moves_when_there_is_not() {
        let mut hands: Vec<Hand> = (1..=4).map(|g| hand(g, g as f32)).collect();
        // All four are on 0x77 from before; a second creature turns up.
        for h in &mut hands {
            h.target = Some(0x77);
        }
        let none = BTreeMap::new();
        let plan = assign_targets(&hands, &[foe(0x77, 1.0), foe(0x78, 4.0)], &none);
        // Two stay, two move: the lowest guids stay.
        assert_eq!(on(&plan, 0x77), vec![1, 2]);
        assert_eq!(on(&plan, 0x78), vec![3, 4]);
        // With room, nobody moves for nothing: the same plan again gives
        // the same answer.
        for h in &mut hands {
            h.target = plan.get(&h.guid).copied();
        }
        assert_eq!(
            assign_targets(&hands, &[foe(0x77, 1.0), foe(0x78, 4.0)], &none),
            plan
        );
        // And the rows lag the orders by a board round: read off the rows,
        // hand 3 (still saying 0x77) would take 0x77 back off hand 2, and
        // the two would swap every round. The last plan is what is kept.
        for h in &mut hands {
            h.target = Some(0x77);
        }
        assert_eq!(
            assign_targets(&hands, &[foe(0x77, 1.0), foe(0x78, 4.0)], &plan),
            plan
        );
    }

    #[test]
    fn the_plan_is_the_same_whatever_order_the_board_gave_it_in() {
        let hands: Vec<Hand> = (1..=5).map(|g| hand(g, g as f32 * 1.5)).collect();
        let foes = [foe(0x79, 7.0), foe(0x77, 1.0), foe(0x78, 4.0)];
        let none = BTreeMap::new();
        let plan = assign_targets(&hands, &foes, &none);
        let mut backwards = hands.clone();
        backwards.reverse();
        let mut foes_back = foes.clone();
        foes_back.reverse();
        assert_eq!(assign_targets(&backwards, &foes_back, &none), plan);
        // Everyone fights, nothing has more than its share (two).
        assert_eq!(plan.len(), 5);
        for f in &foes {
            assert!(on(&plan, f.guid).len() <= 2, "{:#x} has too many", f.guid);
            assert!(!on(&plan, f.guid).is_empty(), "{:#x} has nobody", f.guid);
        }
    }

    #[test]
    fn nothing_is_planned_for_nobody_and_a_hand_that_does_not_fight_gets_no_order() {
        let none = BTreeMap::new();
        assert!(assign_targets(&[], &[foe(0x77, 1.0)], &none).is_empty());
        assert!(assign_targets(&[hand(1, 0.0)], &[], &none).is_empty());
        let mut idle = hand(2, 1.0);
        idle.fights = false;
        let plan = assign_targets(&[hand(1, 0.0), idle], &[foe(0x77, 1.0)], &none);
        assert_eq!(plan.get(&2), None);
        assert_eq!(plan.get(&1), Some(&0x77));
    }

    #[test]
    fn what_hit_a_mate_is_the_nearest_creature_of_that_name_to_it() {
        let mut me = hand(1, 0.0);
        me.hit_by = vec!["Drudge Skulker".into(), "Cow".into()];
        let foes = [
            (0x10, "Drudge Skulker", Vec3::new(9.0, 0.0, 0.0)),
            (0x11, "Drudge Skulker", Vec3::new(2.0, 0.0, 0.0)),
            (0x12, "Rabbit", Vec3::new(1.0, 0.0, 0.0)),
        ];
        assert_eq!(
            hunted_by(&foes, &me),
            vec![0x11],
            "the near one, and no Cow in sight"
        );
    }

    fn body(guid: u32, x: f32) -> Body {
        Body {
            guid,
            world: Vec3::new(x, 0.0, 0.0),
        }
    }

    fn near(a: Vec3, b: Vec3) -> bool {
        a.distance(b) <= 20.0
    }

    #[test]
    fn two_bodies_go_to_two_hands_the_fewest_turns_first() {
        let mut hands: Vec<Hand> = (1..=3).map(|g| hand(g, g as f32)).collect();
        hands[0].turns = 2;
        hands[1].turns = 0;
        hands[2].turns = 1;
        let deals = deal_bodies(&hands, &[body(0x8001, 1.0), body(0x8002, 2.0)], &[], near);
        assert_eq!(deals, vec![(0x8001, 2), (0x8002, 3)]);
    }

    #[test]
    fn a_hand_at_a_body_fighting_or_without_room_is_dealt_after_or_not_at_all() {
        let mut hands: Vec<Hand> = (1..=4).map(|g| hand(g, g as f32)).collect();
        hands[0].looting = Some(0x9000);
        hands[1].target = Some(0x77);
        hands[2].opens = false;
        // Only 4 is free: it gets the first body; the second goes to the
        // one fighting, which will come for it when it is done.
        let deals = deal_bodies(
            &hands,
            &[body(0x8001, 1.0), body(0x8002, 2.0), body(0x8003, 3.0)],
            &[],
            near,
        );
        assert_eq!(deals, vec![(0x8001, 4), (0x8002, 2)]);
        // Out of reach of everyone, a body is nobody's to be dealt.
        let deals = deal_bodies(&hands, &[body(0x8001, 500.0)], &[], near);
        assert!(deals.is_empty());
    }

    #[test]
    fn a_deal_stands_while_the_body_lies_and_lapses_when_nobody_came() {
        let hands: Vec<Hand> = (1..=2).map(|g| hand(g, g as f32)).collect();
        let bodies = [body(0x8001, 1.0)];
        let stood = |age, to| Standing {
            body: 0x8001,
            to,
            age,
        };
        // Fresh: stands, and the body is not dealt to the nearer hand.
        let deals = deal_bodies(&hands, &bodies, &[stood(Duration::from_secs(2), 2)], near);
        assert_eq!(deals, vec![(0x8001, 2)]);
        // Past patience with nobody at it: dealt again.
        let deals = deal_bodies(&hands, &bodies, &[stood(DEAL_PATIENCE, 2)], near);
        assert_eq!(deals, vec![(0x8001, 1)]);
        // Past patience but at it: stands.
        let mut at_it = hands.clone();
        at_it[1].looting = Some(0x8001);
        let deals = deal_bodies(&at_it, &bodies, &[stood(DEAL_PATIENCE * 3, 2)], near);
        assert_eq!(deals, vec![(0x8001, 2)]);
        // The body gone: the deal goes with it.
        assert!(deal_bodies(&hands, &[], &[stood(Duration::ZERO, 2)], near).is_empty());
        // The hand gone from the board: dealt to whoever is left.
        let deals = deal_bodies(&hands[..1], &bodies, &[stood(Duration::ZERO, 2)], near);
        assert_eq!(deals, vec![(0x8001, 1)]);
    }

    #[test]
    fn the_planner_keeps_a_deals_clock_and_counts_a_new_deal_as_a_turn() {
        let t0 = Instant::now();
        let window = Duration::from_secs(120);
        let mut p = Planner::default();
        p.dealt(&[(0x8001, 2)], t0, window);
        assert_eq!(p.turns_of(2, t0, window), 1);
        p.dealt(
            &[(0x8001, 2), (0x8002, 1)],
            t0 + Duration::from_secs(3),
            window,
        );
        assert_eq!(
            p.turns_of(2, t0 + Duration::from_secs(3), window),
            1,
            "the same deal is one turn"
        );
        assert_eq!(p.turns_of(1, t0 + Duration::from_secs(3), window), 1);
        let standing = p.standing(t0 + Duration::from_secs(5));
        assert_eq!(
            standing,
            vec![
                Standing {
                    body: 0x8001,
                    to: 2,
                    age: Duration::from_secs(5)
                },
                Standing {
                    body: 0x8002,
                    to: 1,
                    age: Duration::from_secs(2)
                },
            ]
        );
        // Turns fall out of the window.
        assert_eq!(p.turns_of(2, t0 + window, window), 0);
    }

    #[test]
    fn the_leader_waits_for_a_follower_fighting_or_far_behind_and_for_nobody_else() {
        let leader = Vec3::ZERO;
        let mut hands: Vec<Hand> = (1..=5).map(|g| hand(g, 0.0)).collect();
        hands[1].target = Some(0x77);
        hands[2].world = Vec3::new(42.0, 0.0, 0.0);
        hands[3].world = Vec3::new(0.0, 12.0, 0.0);
        hands[4].world = Vec3::new(60.0, 0.0, 0.0);
        hands[4].health = 0.0;
        let behind = stragglers(leader, 4.0, &hands);
        assert_eq!(
            behind,
            vec![
                Straggler {
                    name: "Bryn02".into(),
                    why: Behind::Fighting
                },
                Straggler {
                    name: "Bryn03".into(),
                    why: Behind::Off(42.0)
                },
            ]
        );
        assert_eq!(
            waiting_line(&behind),
            "waiting for Bryn02 (fighting), Bryn03 (42 m off)"
        );
        // One not following is hunting on its own and is not waited for.
        hands[2].following = false;
        assert_eq!(stragglers(leader, 4.0, &hands).len(), 1);
    }

    #[test]
    fn orders_are_good_for_a_few_rounds_and_no_longer() {
        let t0 = Instant::now();
        let mut plan = Plan {
            leader: "+Brynna".into(),
            n: 3,
            ..Default::default()
        };
        plan.orders.insert(
            2,
            Order {
                target: Some(0x77),
                body: Some(0x8001),
            },
        );
        let orders = Orders {
            plan: plan.clone(),
            heard: t0,
        };
        assert_eq!(orders.current(t0 + ORDERS_LAST / 2), Some(&plan));
        assert_eq!(
            orders.current(t0 + ORDERS_LAST),
            None,
            "a quiet leader leaves no orders"
        );
        assert_eq!(plan.order_for(2).and_then(|o| o.target), Some(0x77));
        assert_eq!(plan.order_for(3), None);
        assert_eq!(plan.body_dealt_to(0x8001), Some(2));
        assert_eq!(plan.body_dealt_to(0x8002), None);
    }

    #[test]
    fn a_plan_crosses_the_bus_as_json_and_an_older_builds_word_still_reads() {
        let mut plan = Plan {
            leader: "+Brynna".into(),
            n: 9,
            waiting_for: vec!["+Brynne".into()],
            ..Default::default()
        };
        plan.orders.insert(
            0x5000_0001,
            Order {
                target: Some(0x77),
                body: None,
            },
        );
        let json = serde_json::to_value(&plan).expect("a plan is JSON");
        let back: Plan = serde_json::from_value(json).expect("and comes back");
        assert_eq!(back, plan);
        let old: Plan = serde_json::from_value(serde_json::json!({"leader": "+Brynna"}))
            .expect("a bare plan reads");
        assert!(old.orders.is_empty() && old.waiting_for.is_empty());
    }
}
