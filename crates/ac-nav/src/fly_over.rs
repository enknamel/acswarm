//! A planned route's trap crossings flown: where a leg passes within reach of a trap plate
//! (`ac_world::traps`), the route rises straight up before it, crosses level above the plate's
//! top and comes down past it. The server fires a plate only when its sweep of the player touches
//! the sphere (ACE `Player.cs:370-386`), and a flown body's sweep passes over it.

use ac_scene::nav::EdgeKind;
use ac_world::traps::Trap;
use ac_world::Place;
use glam::{Vec2, Vec3};

use crate::routing::{Route, Waypoint};

/// Kept between the body's capsule and a plate's sphere on the flat, and above its top (metres).
pub const TRAP_BERTH: f32 = 0.5;
/// A capsule's floor-to-crown height: a leg this far below a sphere's bottom still touches it.
const BODY_HEIGHT: f32 = 2.0;

/// One stretch of the route to fly: from `s0` to `s1` metres along it, at least `above` (world z).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Stretch {
    s0: f32,
    s1: f32,
    above: f32,
}

/// `route`, walked from `start` by a body of capsule radius `radius`, with every stretch that
/// would touch a trap plate made rise, cross and land waypoints ([`EdgeKind::Fly`]).
pub fn over_traps(route: Route, start: Vec3, radius: f32) -> Route {
    let mut blocks: Vec<u32> = route
        .waypoints
        .iter()
        .map(|w| w.place.cell & 0xFFFF_0000)
        .collect();
    blocks.sort_unstable();
    blocks.dedup();
    let traps: Vec<&Trap> = blocks
        .iter()
        .flat_map(|b| ac_world::traps::in_landblock(*b))
        .collect();
    if traps.is_empty() {
        return route;
    }
    let mut ends = vec![(start, route.waypoints.first().map_or(0, |w| w.place.cell))];
    ends.extend(route.waypoints.iter().map(|w| (w.place.at, w.place.cell)));
    let stretches = merged(stretches_near(&ends, &traps, radius));
    if stretches.is_empty() {
        return route;
    }
    rebuilt(&route, &ends, &stretches)
}

/// Each leg's stretch within reach of a plate whose sphere its body would touch.
fn stretches_near(ends: &[(Vec3, u32)], traps: &[&Trap], radius: f32) -> Vec<Stretch> {
    let mut out = Vec::new();
    let mut s = 0.0;
    for leg in ends.windows(2) {
        let (a, b) = (leg[0].0, leg[1].0);
        let len = a.distance(b);
        for t in traps {
            let (c, r) = t.sphere();
            let low = a.z.min(b.z) - BODY_HEIGHT;
            if c.z - r > a.z.max(b.z) + BODY_HEIGHT || c.z + r < low {
                continue;
            }
            let reach = r + radius + TRAP_BERTH;
            if let Some((t0, t1)) = within_circle(a.truncate(), b.truncate(), c.truncate(), reach) {
                out.push(Stretch {
                    s0: s + t0 * len,
                    s1: s + t1 * len,
                    above: t.top() + TRAP_BERTH,
                });
            }
        }
        s += len;
    }
    out
}

/// The share of segment `a`..`b` (0..=1 each end) lying within `reach` of `c`, on the flat.
fn within_circle(a: Vec2, b: Vec2, c: Vec2, reach: f32) -> Option<(f32, f32)> {
    let d = b - a;
    let f = a - c;
    let qa = d.length_squared();
    if qa < 1e-9 {
        return (f.length() < reach).then_some((0.0, 1.0));
    }
    let qb = 2.0 * f.dot(d);
    let qc = f.length_squared() - reach * reach;
    let disc = qb * qb - 4.0 * qa * qc;
    if disc <= 0.0 {
        return None;
    }
    let root = disc.sqrt();
    let t0 = ((-qb - root) / (2.0 * qa)).max(0.0);
    let t1 = ((-qb + root) / (2.0 * qa)).min(1.0);
    (t0 < t1).then_some((t0, t1))
}

/// Overlapping stretches as one, flown at the highest top among them.
fn merged(mut v: Vec<Stretch>) -> Vec<Stretch> {
    v.sort_by(|a, b| a.s0.total_cmp(&b.s0));
    let mut out: Vec<Stretch> = Vec::new();
    for x in v {
        match out.last_mut() {
            Some(last) if x.s0 <= last.s1 => {
                last.s1 = last.s1.max(x.s1);
                last.above = last.above.max(x.above);
            }
            _ => out.push(x),
        }
    }
    out
}

/// The point `s` metres along the polyline `ends`, with the cell of the leg end nearer it.
fn point_at(ends: &[(Vec3, u32)], s: f32) -> (Vec3, u32) {
    let mut left = s;
    for leg in ends.windows(2) {
        let len = leg[0].0.distance(leg[1].0);
        if left <= len {
            let t = if len > 1e-6 { left / len } else { 1.0 };
            let at = leg[0].0.lerp(leg[1].0, t);
            let cell = if t < 0.5 { leg[0].1 } else { leg[1].1 };
            return (at, cell);
        }
        left -= len;
    }
    *ends.last().expect("a start at least")
}

/// The route with each stretch's own waypoints swapped for walk-in, rise, cross and land.
fn rebuilt(route: &Route, ends: &[(Vec3, u32)], stretches: &[Stretch]) -> Route {
    let place = |(at, cell): (Vec3, u32)| Place { at, cell };
    let flown = |at: Vec3, cell: u32| Waypoint {
        place: Place { at, cell },
        kind: EdgeKind::Fly,
        floor: None,
    };
    let mut out = Vec::with_capacity(route.waypoints.len() + 4 * stretches.len());
    let mut s = 0.0;
    let mut next = stretches.iter().peekable();
    for (i, w) in route.waypoints.iter().enumerate() {
        let len = ends[i].0.distance(ends[i + 1].0);
        let (s_in, s_out) = (s, s + len);
        s = s_out;
        // Every stretch starting on this leg: walk to its edge, then fly it.
        let mut skip = false;
        while let Some(st) = next.peek().copied() {
            if st.s0 > s_out {
                break;
            }
            if st.s0 > s_in + 1e-3 {
                out.push(Waypoint {
                    place: place(point_at(ends, st.s0)),
                    kind: w.kind,
                    floor: None,
                });
            }
            let (from, from_cell) = point_at(ends, st.s0);
            let (to, to_cell) = point_at(ends, st.s1);
            let high = st.above.max(from.z).max(to.z);
            out.push(flown(Vec3::new(from.x, from.y, high), from_cell));
            out.push(flown(Vec3::new(to.x, to.y, high), to_cell));
            out.push(flown(to, to_cell));
            skip = st.s1 >= s_out - 1e-3;
            next.next();
        }
        // A waypoint inside a stretch flown is flown past.
        let inside = stretches
            .iter()
            .any(|st| s_out > st.s0 + 1e-3 && s_out < st.s1 - 1e-3);
        if !skip && !inside {
            out.push(*w);
        }
    }
    Route { waypoints: out }
}

impl Route {
    /// The route walked where it would fly: for a body not allowed to (server-safe movement).
    pub fn walked(self) -> Route {
        let mut out: Vec<Waypoint> = Vec::with_capacity(self.waypoints.len());
        let n = self.waypoints.len();
        for (i, w) in self.waypoints.iter().enumerate() {
            let landing = w.kind == EdgeKind::Fly
                && self
                    .waypoints
                    .get(i + 1)
                    .is_none_or(|x| x.kind != EdgeKind::Fly);
            if w.kind != EdgeKind::Fly || i + 1 == n || landing {
                out.push(Waypoint {
                    kind: if w.kind == EdgeKind::Fly {
                        EdgeKind::Walk
                    } else {
                        w.kind
                    },
                    ..*w
                });
            }
        }
        Route { waypoints: out }
    }

    /// Whether any leg of it is flown.
    pub fn is_flown(&self) -> bool {
        self.waypoints.iter().any(|w| w.kind == EdgeKind::Fly)
    }
}

#[cfg(test)]
mod tests;
