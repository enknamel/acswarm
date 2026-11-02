//! Routes on a [`NavGraph`] between two ends resolved by cell and floor, never by the nearest
//! node: the start's first leg and the goal's last are walked like every edge, and nothing
//! unchecked is appended. [`search_building`] builds the chunks it reaches, [`search`] none.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

use glam::Vec3;

use crate::collision::{Capsule, FOOTING};
use crate::nav::{flat, in_berth, line_clear, walk_kind, EdgeKind, Ground, NavGraph, NavNode};

/// A point further than this (metres) over or under a floor is not standing on it: the capsule's
/// step down.
pub const END_FLOOR: f32 = 1.5;
/// How far the ends look for nodes, in columns.
const NEAREST_REACH: i32 = 2;
/// How far the start looks when no node near it is walked to: a character leaning on a wall.
const STRANDED_REACH: i32 = 8;
/// Nodes tried per look with a walk from the start, or to the goal.
const JOIN_TRIES: usize = 8;
/// Extra clearance a smoothed route keeps from walls (metres).
const SMOOTH_MARGIN: f32 = 0.3;
/// Points ahead a smoothing shortcut is tried to.
const LOOKAHEAD: usize = 12;
/// Waypoints closer than this (metres) are one.
const SAME_POINT: f32 = 0.1;

/// Why there is no route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoWay {
    /// The goal stands on no floor of its own cell.
    OffFloor,
    /// No node is a walk from the start, even pushed out of the wall it leans on.
    Stranded,
    /// The region is built whole and the two ends lie in parts that do not join.
    Apart,
    /// The search ran out.
    Unreached,
    /// An end lies outside the region.
    OutOfRegion,
}

/// A walk asked of the graph. Cells are the graph's: an interior cell's id, 0 outdoors.
pub struct WalkQuery<'a> {
    pub from: (Vec3, u32),
    pub goal: (Vec3, u32),
    /// How near the goal (metres, 3D) is there.
    pub within: f32,
    /// Portal mouths kept `berth` metres off (see [`in_berth`]).
    pub avoid: &'a [Vec3],
    pub berth: f32,
    /// Interior nodes may be walked through.
    pub indoors_too: bool,
    /// Drop edges may be taken: not by the old steering, which runs off a ledge at a run.
    pub drops: bool,
}

/// What a search found: every waypoint after the start with the kind of edge into it, the last
/// within reach of the goal; no way and why; or the chunks it needs built first.
#[derive(Debug, Clone)]
pub enum Search {
    Found(Vec<(NavNode, EdgeKind)>),
    NoWay(NoWay),
    Unbuilt(Vec<(i32, i32)>),
}

/// The cell a body at `at` (feet) stands in, by the cells' BSPs: 0 outdoors.
pub fn locate(ground: &Ground, at: Vec3) -> u32 {
    ground
        .collision
        .cell_at(at + Vec3::new(0.0, 0.0, 0.1))
        .unwrap_or(0)
}

/// `at` on its cell's own floor: that cell's floor (outdoors, the open floors and the terrain)
/// nearest its height within [`END_FLOOR`], under the middle, else under the rest of the feet;
/// outdoors the terrain is the floor of last resort. `None` when the cell has no floor there.
pub fn floor_of(ground: &Ground, at: Vec3, cell: u32, cap: &Capsule) -> Option<Vec3> {
    let level = |x: f32, y: f32| -> Option<f32> {
        let mut levels: Vec<f32> = ground
            .collision
            .floors_at_xy(x, y)
            .into_iter()
            .filter(|f| f.1 == cell)
            .map(|f| f.0)
            .collect();
        if cell == 0 {
            levels.extend(ground.terrain_under(x, y));
        }
        levels
            .into_iter()
            .filter(|z| (z - at.z).abs() <= END_FLOOR)
            .min_by(|a, b| (a - at.z).abs().total_cmp(&(b - at.z).abs()))
    };
    let reach = cap.radius * FOOTING;
    let z = level(at.x, at.y)
        .or_else(|| {
            (0..8)
                .filter_map(|k| {
                    let a = k as f32 * std::f32::consts::FRAC_PI_4;
                    level(at.x + a.cos() * reach, at.y + a.sin() * reach)
                })
                .min_by(|a, b| (a - at.z).abs().total_cmp(&(b - at.z).abs()))
        })
        .or_else(|| {
            (cell == 0)
                .then(|| ground.terrain_under(at.x, at.y))
                .flatten()
        })?;
    Some(Vec3::new(at.x, at.y, z))
}

/// A route for `q`, building every chunk the search reaches.
pub fn search_building(ground: &Ground, nav: &mut NavGraph, q: &WalkQuery) -> Search {
    let mut l = Building { nav, ground };
    match route(ground, &mut l, q) {
        Ok(path) => Search::Found(path),
        Err(why) => Search::NoWay(why),
    }
}

/// A route for `q` on the chunks built so far; `Unbuilt` whenever it reached one not built, so a
/// route is only ever the one [`search_building`] gives, never a way round what is missing.
pub fn search(ground: &Ground, nav: &NavGraph, q: &WalkQuery) -> Search {
    let mut l = Built {
        nav,
        missing: Vec::new(),
    };
    let found = route(ground, &mut l, q);
    let mut missing = l.missing;
    missing.sort_unstable();
    missing.dedup();
    match found {
        Err(NoWay::OffFloor) => Search::NoWay(NoWay::OffFloor),
        _ if !missing.is_empty() => Search::Unbuilt(missing),
        Ok(path) => Search::Found(path),
        Err(why) => Search::NoWay(why),
    }
}

/// The nodes within `radius` (metres, flat) of `around`; once the graph is built whole, only those
/// joined to a node the body at `from` walks to. `Err` names the chunks to build first.
pub fn standable_near(
    ground: &Ground,
    nav: &NavGraph,
    from: (Vec3, u32),
    around: Vec3,
    radius: f32,
) -> Result<Vec<NavNode>, Vec<(i32, i32)>> {
    let r = (radius / nav.spacing).ceil() as i32;
    let (gx, gy) = nav.column_of(around);
    let missing = nav.unbuilt(gx - r, gx + r, gy - r, gy + r);
    if !missing.is_empty() {
        return Err(missing);
    }
    let mut out: Vec<u32> = Vec::new();
    for dx in -r..=r {
        for dy in -r..=r {
            out.extend(
                nav.column_ids((gx + dx, gy + dy))
                    .iter()
                    .filter(|&&id| flat(nav.nodes[id as usize].pos - around).length() <= radius),
            );
        }
    }
    if nav.is_complete() {
        let q = WalkQuery {
            from,
            goal: from,
            within: 0.0,
            avoid: &[],
            berth: 0.0,
            indoors_too: true,
            drops: false,
        };
        let cap = nav.capsule;
        let s = floor_of(ground, from.0, from.1, &cap).unwrap_or(from.0);
        let mut l = Built {
            nav,
            missing: Vec::new(),
        };
        let roots: Vec<u32> = sources(ground, &mut l, &q, &Berths(Vec::new()), s, from.1, &cap)
            .map(|(_, ids)| ids.iter().map(|&(id, _)| nav.component(id)).collect())
            .unwrap_or_default();
        out.retain(|&id| roots.contains(&nav.component(id)));
    }
    Ok(out.iter().map(|&id| nav.nodes[id as usize]).collect())
}

/// The graph a search runs on, and what it does on reaching columns not built yet.
trait Lattice {
    fn nav(&self) -> &NavGraph;
    /// The columns within `r` of `p`, built or noted.
    fn reach(&mut self, p: Vec3, r: i32);
    /// The columns round `node`, so its edges are all there, built or noted.
    fn expand(&mut self, node: u32);
}

struct Building<'g, 'a> {
    nav: &'g mut NavGraph,
    ground: &'g Ground<'a>,
}

impl Lattice for Building<'_, '_> {
    fn nav(&self) -> &NavGraph {
        self.nav
    }
    fn reach(&mut self, p: Vec3, r: i32) {
        let (gx, gy) = self.nav.column_of(p);
        self.nav
            .ensure_columns(self.ground, gx - r, gx + r, gy - r, gy + r);
    }
    fn expand(&mut self, node: u32) {
        self.nav.ensure_neighbours(self.ground, node);
    }
}

struct Built<'g> {
    nav: &'g NavGraph,
    missing: Vec<(i32, i32)>,
}

impl Lattice for Built<'_> {
    fn nav(&self) -> &NavGraph {
        self.nav
    }
    fn reach(&mut self, p: Vec3, r: i32) {
        let (gx, gy) = self.nav.column_of(p);
        let more = self.nav.unbuilt(gx - r, gx + r, gy - r, gy + r);
        self.missing.extend(more);
    }
    fn expand(&mut self, node: u32) {
        let (gx, gy) = self.nav.nodes[node as usize].column;
        let more = self.nav.unbuilt(gx - 1, gx + 1, gy - 1, gy + 1);
        self.missing.extend(more);
    }
}

/// Where a route ends from a goal node: at the node, or where a walk from it to the goal stops.
#[derive(Clone, Copy)]
struct End {
    cost: f32,
    at: Option<Vec3>,
}

fn route(
    ground: &Ground,
    l: &mut impl Lattice,
    q: &WalkQuery,
) -> Result<Vec<(NavNode, EdgeKind)>, NoWay> {
    let cap = l.nav().capsule;
    let (goal_at, goal_cell) = q.goal;
    let g = floor_of(ground, goal_at, goal_cell, &cap).ok_or(NoWay::OffFloor)?;
    let (from_at, from_cell) = q.from;
    // A start on no floor of its cell (in the air, a cell a step behind) is where the body is: its
    // first leg is walked from there like any other.
    let s = floor_of(ground, from_at, from_cell, &cap).unwrap_or(from_at);
    let berths = Berths::of(q, s);
    let (start, sources) = sources(ground, l, q, &berths, s, from_cell, &cap)?;
    let ends = ends(ground, l, q, &berths, g, goal_cell, &cap);
    if ends.is_empty() {
        return Err(NoWay::Unreached);
    }
    // Built whole, the graph's components are final: in different ones there is no way at all.
    let nav = l.nav();
    if nav.is_complete()
        && !sources
            .iter()
            .any(|&(a, _)| ends.keys().any(|&b| nav.component(a) == nav.component(b)))
    {
        return Err(NoWay::Apart);
    }
    let path = astar(l, q, &berths, &sources, &ends, g)?;
    let nav = l.nav();
    let mut points: Vec<(NavNode, EdgeKind)> = Vec::with_capacity(path.len() + 1);
    let mut cell = from_cell;
    for &(id, kind) in &path {
        let n = nav.nodes[id as usize];
        let kind = kind.unwrap_or_else(|| walk_kind(cell, n.cell));
        points.push((n, kind));
        cell = n.cell;
    }
    let last = path.last().map(|&(id, _)| id).ok_or(NoWay::Unreached)?;
    if let Some(at) = ends[&last].at {
        let end = NavNode {
            pos: at,
            cell: goal_cell,
            column: nav.column_of(at),
        };
        points.push((end, walk_kind(cell, goal_cell)));
    }
    Ok(smooth(ground, &berths, start, from_cell, points, &cap))
}

/// The portal mouths a walk keeps off, each with its berth: a berth the walk starts in shrinks to
/// where it stands, so the body walks out of it and never nearer the mouth.
struct Berths(Vec<(Vec3, f32)>);

impl Berths {
    fn of(q: &WalkQuery, start: Vec3) -> Self {
        let keep = |m: &Vec3| {
            let r = if in_berth(*m, start, q.berth) {
                flat(start - *m).length()
            } else {
                q.berth
            };
            (*m, r)
        };
        Berths(if q.berth > 0.0 {
            q.avoid.iter().map(keep).collect()
        } else {
            Vec::new()
        })
    }

    /// Whether feet at `p` stand in a berth.
    fn holds(&self, p: Vec3) -> bool {
        self.0.iter().any(|&(m, r)| in_berth(m, p, r))
    }

    /// Whether the segment `a`..`b` passes through a berth that `a` does not already stand in.
    fn crosses(&self, a: Vec3, b: Vec3) -> bool {
        let len = flat(b - a).length();
        let steps = (len / 0.5).ceil().max(1.0) as usize;
        self.0.iter().any(|&(m, r)| {
            flat(a - m).length() < r + len
                && !in_berth(m, a, r)
                && (1..=steps).any(|k| in_berth(m, a.lerp(b, k as f32 / steps as f32), r))
        })
    }
}

/// Whether a node may be stood on in this walk.
fn can_stand_on(q: &WalkQuery, berths: &Berths, n: &NavNode) -> bool {
    (q.indoors_too || n.cell == 0) && !berths.holds(n.pos)
}

/// A leg of a route walked by the body, clear of the berths.
fn can_walk_leg(
    ground: &Ground,
    berths: &Berths,
    a: (Vec3, u32),
    b: (Vec3, u32),
    cap: &Capsule,
) -> bool {
    !berths.crosses(a.0, b.0) && ground.can_walk(a, b, cap)
}

/// A leg from where the body stands: the body's own step, whatever the chest ray meets, since the
/// body is there and its step is what it will do.
fn can_leave_start(
    ground: &Ground,
    berths: &Berths,
    a: (Vec3, u32),
    b: (Vec3, u32),
    cap: &Capsule,
) -> bool {
    !berths.crosses(a.0, b.0) && ground.body_reaches(a.0, a.1, b.0, cap)
}

/// The nodes a route may begin at, each with the cost of the checked walk to it from the start;
/// the start pushed out of a wall when nothing nearer is walked to.
fn sources(
    ground: &Ground,
    l: &mut impl Lattice,
    q: &WalkQuery,
    berths: &Berths,
    s: Vec3,
    cell: u32,
    cap: &Capsule,
) -> Result<(Vec3, Vec<(u32, f32)>), NoWay> {
    let pushed = ground
        .collision
        .resolve_above(s, cap.radius, cap.height, cap.step_up);
    let mut tried: HashSet<(u32, u32)> = HashSet::new();
    for (look, (from, r)) in [
        (s, NEAREST_REACH),
        (s, STRANDED_REACH),
        (pushed, STRANDED_REACH),
    ]
    .into_iter()
    .enumerate()
    {
        l.reach(from, r);
        let nav = l.nav();
        let key = u32::from(look == 2);
        let mut near: Vec<(bool, f32, u32)> = nav
            .near(from, r)
            .into_iter()
            .map(|(_, id)| id)
            .filter(|id| {
                can_stand_on(q, berths, &nav.nodes[*id as usize]) && !tried.contains(&(key, *id))
            })
            .map(|id| {
                let n = nav.nodes[id as usize];
                (n.cell != cell, n.pos.distance(from), id)
            })
            .collect();
        near.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
        let mut kept = Vec::new();
        for (_, d, id) in near.into_iter().take(JOIN_TRIES) {
            tried.insert((key, id));
            let n = nav.nodes[id as usize];
            if can_leave_start(ground, berths, (from, cell), (n.pos, n.cell), cap) {
                kept.push((id, s.distance(from) + d));
            }
        }
        if !kept.is_empty() {
            return Ok((from, kept));
        }
    }
    Err(NoWay::Stranded)
}

/// The goal set: nodes within `within` of `g` on its floor in sight of it, and nodes from which
/// the body walks to `g`, or to where it stops within `within` of it.
fn ends(
    ground: &Ground,
    l: &mut impl Lattice,
    q: &WalkQuery,
    berths: &Berths,
    g: Vec3,
    cell: u32,
    cap: &Capsule,
) -> HashMap<u32, End> {
    let spacing = l.nav().spacing;
    let r = NEAREST_REACH.max((q.within / spacing).ceil() as i32);
    l.reach(g, r);
    let nav = l.nav();
    let mut ends: HashMap<u32, End> = HashMap::new();
    let mut rest: Vec<(bool, f32, u32)> = Vec::new();
    for (_, id) in nav.near(g, r) {
        let n = nav.nodes[id as usize];
        if !can_stand_on(q, berths, &n) {
            continue;
        }
        let d = n.pos.distance(g);
        if d <= q.within
            && (n.pos.z - g.z).abs() <= cap.step_up
            && line_clear(ground.collision, n.pos, g)
        {
            ends.insert(
                id,
                End {
                    cost: 0.0,
                    at: None,
                },
            );
        } else if nav.column_of(n.pos).0.abs_diff(nav.column_of(g).0) <= NEAREST_REACH as u32
            && nav.column_of(n.pos).1.abs_diff(nav.column_of(g).1) <= NEAREST_REACH as u32
        {
            rest.push((n.cell != cell, d, id));
        }
    }
    rest.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    for (_, _, id) in rest.into_iter().take(JOIN_TRIES) {
        let n = nav.nodes[id as usize];
        if berths.crosses(n.pos, g) {
            continue;
        }
        if let Some(p) = ground.body_stops_within(n.pos, n.cell, g, q.within, cap) {
            ends.insert(
                id,
                End {
                    cost: n.pos.distance(p),
                    at: Some(p),
                },
            );
        }
    }
    ends
}

/// A* from every source to the cheapest finish over the goal set: the nodes walked, each with the
/// kind of graph edge into it (`None` for the first, reached by the start's leg).
fn astar(
    l: &mut impl Lattice,
    q: &WalkQuery,
    berths: &Berths,
    sources: &[(u32, f32)],
    ends: &HashMap<u32, End>,
    g: Vec3,
) -> Result<Vec<(u32, Option<EdgeKind>)>, NoWay> {
    #[derive(PartialEq)]
    struct Open {
        f: f32,
        node: u32,
        finish: bool,
    }
    impl Eq for Open {}
    impl PartialOrd for Open {
        fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
            Some(self.cmp(o))
        }
    }
    impl Ord for Open {
        fn cmp(&self, o: &Self) -> Ordering {
            o.f.total_cmp(&self.f)
        }
    }
    let h = |p: Vec3| (p.distance(g) - q.within).max(0.0);
    let mut best: HashMap<u32, f32> = HashMap::new();
    let mut came: HashMap<u32, (u32, EdgeKind)> = HashMap::new();
    let mut closed: HashSet<u32> = HashSet::new();
    let mut open = BinaryHeap::new();
    for &(id, cost) in sources {
        if best.get(&id).is_none_or(|&b| cost < b) {
            best.insert(id, cost);
            open.push(Open {
                f: cost + h(l.nav().nodes[id as usize].pos),
                node: id,
                finish: false,
            });
        }
    }
    while let Some(Open { node, finish, .. }) = open.pop() {
        if finish {
            let mut path = vec![(node, None)];
            let mut cur = node;
            while let Some(&(prev, kind)) = came.get(&cur) {
                path.last_mut().expect("the path holds its end").1 = Some(kind);
                path.push((prev, None));
                cur = prev;
            }
            path.reverse();
            return Ok(path);
        }
        if !closed.insert(node) {
            continue;
        }
        let cost = best[&node];
        if let Some(end) = ends.get(&node) {
            open.push(Open {
                f: cost + end.cost,
                node,
                finish: true,
            });
        }
        l.expand(node);
        let nav = l.nav();
        let here = nav.nodes[node as usize].pos;
        for e in nav.neighbours(node) {
            let next = nav.nodes[e.to as usize];
            if closed.contains(&e.to)
                || !can_stand_on(q, berths, &next)
                || berths.crosses(here, next.pos)
                || (e.kind == EdgeKind::Drop && !q.drops)
            {
                continue;
            }
            let g2 = cost + here.distance(next.pos);
            if best.get(&e.to).is_none_or(|&b| g2 < b) {
                best.insert(e.to, g2);
                came.insert(e.to, (node, e.kind));
                open.push(Open {
                    f: g2 + h(next.pos),
                    node: e.to,
                    finish: false,
                });
            }
        }
    }
    Err(NoWay::Unreached)
}

/// String-pulling: from each kept point, on to the furthest of the next [`LOOKAHEAD`] that a
/// [`can_cut_to`] with [`SMOOTH_MARGIN`] to spare reaches, never across a drop. A waypoint into
/// another cell than the one before it is a doorway.
fn smooth(
    ground: &Ground,
    berths: &Berths,
    start: Vec3,
    start_cell: u32,
    mut points: Vec<(NavNode, EdgeKind)>,
    cap: &Capsule,
) -> Vec<(NavNode, EdgeKind)> {
    // Pulled tight, a route brushes every corner it turns and the walker catches on them; where
    // there is no room to spare the lattice points stay, placed with the true radius.
    let wide = Capsule {
        radius: cap.radius + SMOOTH_MARGIN,
        ..*cap
    };
    let mut k = 0;
    while k + 1 < points.len() {
        let near_next = points[k].0.pos.distance(points[k + 1].0.pos) < SAME_POINT;
        let near_start = k == 0 && points[k].0.pos.distance(start) < SAME_POINT;
        if (near_next || near_start) && points[k + 1].1 != EdgeKind::Drop {
            points.remove(k);
        } else {
            k += 1;
        }
    }
    let mut out = Vec::with_capacity(points.len());
    let (mut from, mut from_cell) = (start, start_cell);
    let mut i = 0;
    while i < points.len() {
        let last = (i + LOOKAHEAD).min(points.len() - 1);
        let best = (i + 1..=last)
            .rev()
            .find(|&j| {
                let p = points[j].0;
                points[i..=j].iter().all(|w| w.1 != EdgeKind::Drop)
                    && can_cut_to(
                        ground,
                        berths,
                        (from, from_cell),
                        (p.pos, p.cell),
                        cap,
                        &wide,
                    )
            })
            .unwrap_or(i);
        let (n, kind) = points[best];
        let kind = match kind {
            EdgeKind::Drop if best == i => EdgeKind::Drop,
            _ => walk_kind(from_cell, n.cell),
        };
        out.push((n, kind));
        (from, from_cell) = (n.pos, n.cell);
        i = best + 1;
    }
    out
}

/// A smoothing shortcut: the walk test of an edge for the body itself, and for one `wide` enough
/// to keep off the walls; the wide one's feet bridge cracks the body falls through.
fn can_cut_to(
    ground: &Ground,
    berths: &Berths,
    a: (Vec3, u32),
    b: (Vec3, u32),
    cap: &Capsule,
    wide: &Capsule,
) -> bool {
    can_walk_leg(ground, berths, a, b, wide) && can_walk_leg(ground, berths, a, b, cap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::CollisionWorld;
    use crate::nav::tests::{a_ledge_in_cells, floor, inside, quad, two_rooms, wall_x};
    use glam::Vec2;

    fn graph(min: Vec2, max: Vec2) -> NavGraph {
        NavGraph::new(min, max, 1.0, &Capsule::default())
    }

    /// A walk from `from` to `goal` (each with its cell) on `nav`, near enough within `within`.
    fn ask(
        ground: &Ground,
        nav: &mut NavGraph,
        from: (Vec3, u32),
        goal: (Vec3, u32),
        within: f32,
    ) -> Search {
        let q = WalkQuery {
            from,
            goal,
            within,
            avoid: &[],
            berth: 0.0,
            indoors_too: true,
            drops: false,
        };
        search_building(ground, nav, &q)
    }

    fn points(found: Search) -> Option<Vec<Vec3>> {
        match found {
            Search::Found(p) => Some(p.into_iter().map(|(n, _)| n.pos).collect()),
            _ => None,
        }
    }

    fn assert_every_leg_walks(ground: &Ground, start: (Vec3, u32), path: &[Vec3], cap: &Capsule) {
        let mut from = start;
        for &p in path {
            let to = (p, locate_floor(ground, p));
            assert!(ground.can_walk(from, to, cap), "{from:?} -> {p} not walked");
            from = to;
        }
    }

    /// The cell of the floor under a waypoint, from the floors' own cells.
    fn locate_floor(ground: &Ground, p: Vec3) -> u32 {
        ground
            .collision
            .floors_at_xy(p.x, p.y)
            .into_iter()
            .min_by(|a, b| (a.0 - p.z).abs().total_cmp(&(b.0 - p.z).abs()))
            .map_or(0, |f| f.1)
    }

    /// A storey (cell 2, z 3, x 0 to 6) over a ground floor (cell 1, z 0, x 0 to 12, y -3 to 3), and
    /// a ramp of cell 2 off its side down to a landing at y 9, joined to the ground floor round it.
    fn two_storeys() -> CollisionWorld {
        let mut w = CollisionWorld::default();
        floor(&mut w, 0.0, 12.0, -3.0, 3.0, 0.0, 1);
        floor(&mut w, 6.0, 12.0, 3.0, 9.0, 0.0, 1);
        floor(&mut w, 0.0, 12.0, 9.0, 12.0, 0.0, 1);
        floor(&mut w, 0.0, 6.0, -3.0, 3.0, 3.0, 2);
        quad(
            &mut w,
            Vec3::new(0.0, 3.0, 3.0),
            Vec3::new(6.0, 3.0, 3.0),
            Vec3::new(6.0, 9.0, 0.0),
            Vec3::new(0.0, 9.0, 0.0),
            2,
        );
        w
    }

    #[test]
    fn two_rooms_joined_by_a_door() {
        let w = two_rooms(Some(1.6));
        let ground = inside(&w);
        let cap = Capsule::default();
        let mut g = graph(Vec2::new(0.0, -4.0), Vec2::new(16.0, 4.0));
        g.build_all(&ground);
        assert!(g.len() > 50, "{} nodes", g.len());
        assert!(g.edge_count() > g.len(), "{} edges", g.edge_count());
        // Nodes hug the walls no closer than the capsule's radius.
        for n in &g.nodes {
            assert!(
                n.pos.x >= 0.4 - 1e-3 && n.pos.x <= 16.0 - 0.4 + 1e-3,
                "{n:?}"
            );
            assert!(n.pos.y.abs() <= 4.0 - 0.4 + 1e-3, "{n:?}");
        }
        let start = Vec3::new(1.0, 3.0, 0.0);
        let goal = Vec3::new(15.0, 3.0, 0.0);
        // The straight line crosses the wall.
        assert!(!line_clear(&w, start, goal));
        let path = points(ask(&ground, &mut g, (start, 1), (goal, 2), 0.0))
            .expect("a path through the door");
        assert!(path.last().unwrap().distance(goal) < 0.02, "{path:?}");
        // It passes through the doorway, not the wall.
        let chain: Vec<Vec3> = std::iter::once(start).chain(path.iter().copied()).collect();
        let crossing = chain
            .windows(2)
            .find(|s| (s[0].x - 8.0) * (s[1].x - 8.0) <= 0.0)
            .expect("crosses x = 8");
        let t = (8.0 - crossing[0].x) / (crossing[1].x - crossing[0].x);
        let y = crossing[0].y + (crossing[1].y - crossing[0].y) * t;
        assert!(y.abs() < 0.8, "crossed the wall at y = {y}: {path:?}");
        assert_every_leg_walks(&ground, (start, 1), &path, &cap);
        // Smoothing cut it down to a handful of turns.
        assert!(path.len() <= 4, "{} waypoints: {path:?}", path.len());
    }

    #[test]
    fn every_leg_of_a_route_passes_the_walk_test() {
        let w = two_rooms(Some(1.6));
        let ground = inside(&w);
        let cap = Capsule::default();
        let mut g = graph(Vec2::new(0.0, -4.0), Vec2::new(16.0, 4.0));
        let start = Vec3::new(1.0, -3.0, 0.0);
        let found = ask(
            &ground,
            &mut g,
            (start, 1),
            (Vec3::new(14.0, 2.5, 0.0), 2),
            0.5,
        );
        let Search::Found(route) = found else {
            panic!("no route: {found:?}");
        };
        let path: Vec<Vec3> = route.iter().map(|(n, _)| n.pos).collect();
        assert_every_leg_walks(&ground, (start, 1), &path, &cap);
        // Into the other room is through its doorway.
        assert!(
            route.iter().any(|(_, k)| *k == EdgeKind::Doorway),
            "{route:?}"
        );
    }

    #[test]
    fn a_start_against_a_wall_begins_at_a_node_it_walks_to() {
        // A node stands only where the capsule fits clear of everything, so a character leaning
        // on a wall has none under it; a pace back is never far, and the route starts there.
        let w = two_rooms(Some(1.6));
        let ground = inside(&w);
        let cap = Capsule::default();
        let mut g = graph(Vec2::new(0.0, -4.0), Vec2::new(16.0, 4.0));
        g.build_all(&ground);
        let against = Vec3::new(cap.radius * 0.5, 0.0, 0.0);
        assert!(
            !ground.fits_here(against, &cap),
            "the fixture is wrong: the capsule fits here"
        );
        let path = points(ask(
            &ground,
            &mut g,
            (against, 1),
            (Vec3::new(12.0, 0.0, 0.0), 2),
            0.5,
        ))
        .expect("no way out for a character leaning on a wall");
        let pushed = w.resolve_above(against, cap.radius, cap.height, cap.step_up);
        assert!(
            ground.body_reaches(against, 1, path[0], &cap)
                || ground.can_walk((pushed, 1), (path[0], 1), &cap),
            "begins at {:?}, which it does not walk to",
            path[0]
        );
    }

    #[test]
    fn no_path_through_a_solid_wall() {
        let w = two_rooms(None);
        let ground = inside(&w);
        let mut g = graph(Vec2::new(0.0, -4.0), Vec2::new(16.0, 4.0));
        g.build_all(&ground);
        let start = Vec3::new(1.0, 3.0, 0.0);
        let goal = Vec3::new(15.0, 3.0, 0.0);
        let found = ask(&ground, &mut g, (start, 1), (goal, 2), 0.0);
        assert!(matches!(found, Search::NoWay(NoWay::Apart)), "{found:?}");
        // Within one room the path is direct.
        let near = Vec3::new(7.0, -3.0, 0.0);
        let path = points(ask(&ground, &mut g, (start, 1), (near, 1), 0.0)).unwrap();
        assert_eq!(path.len(), 1, "{path:?}");
        assert!(path[0].distance(near) < 0.02, "{path:?}");
    }

    #[test]
    fn a_door_too_narrow_for_the_capsule_is_closed() {
        let w = two_rooms(Some(0.6));
        let ground = inside(&w);
        let mut g = graph(Vec2::new(0.0, -4.0), Vec2::new(16.0, 4.0));
        g.build_all(&ground);
        let start = Vec3::new(1.0, 3.0, 0.0);
        let goal = Vec3::new(15.0, 3.0, 0.0);
        let found = ask(&ground, &mut g, (start, 1), (goal, 2), 0.0);
        assert!(matches!(found, Search::NoWay(_)), "{found:?}");
    }

    #[test]
    fn ledges_are_one_way_and_levels_stay_apart() {
        // A low floor and a 1 m higher shelf next to it: walk off the shelf, but not up onto it. A
        // second floor 3.5 m up is a separate level with its own nodes.
        let mut w = CollisionWorld::default();
        floor(&mut w, 0.0, 6.0, -3.0, 3.0, 0.0, 1);
        floor(&mut w, 6.0, 12.0, -3.0, 3.0, 1.0, 1);
        floor(&mut w, 0.0, 12.0, -3.0, 3.0, 3.5, 2);
        let ground = inside(&w);
        let mut g = graph(Vec2::new(0.0, -3.0), Vec2::new(12.0, 3.0));
        g.build_all(&ground);
        let low = (Vec3::new(3.0, 0.0, 0.0), 1);
        let high = (Vec3::new(9.0, 0.0, 1.0), 1);
        let upstairs = (Vec3::new(3.0, 0.0, 3.5), 2);
        assert!(
            points(ask(&ground, &mut g, high, low, 0.0)).is_some(),
            "walk off the shelf"
        );
        assert!(
            points(ask(&ground, &mut g, low, high, 0.0)).is_none(),
            "climb the shelf"
        );
        assert!(
            points(ask(&ground, &mut g, low, upstairs, 0.0)).is_none(),
            "levels joined"
        );
        let across = (Vec3::new(9.0, 0.0, 3.5), 2);
        assert!(points(ask(&ground, &mut g, upstairs, across, 0.0)).is_some());
        let levels = g
            .nodes
            .iter()
            .filter(|n| (n.pos.x - 3.0).abs() < 0.1 && n.pos.y.abs() < 0.1)
            .count();
        assert_eq!(levels, 2, "one node per level in a column");
    }

    #[test]
    fn terrain_fills_in_outdoors_and_steep_slopes_do_not_connect() {
        let w = CollisionWorld::default();
        // A cliff: flat up to x = 10, then rising 2 m per metre.
        let terrain = |x: f32, _y: f32| Some(if x < 10.0 { 0.0 } else { (x - 10.0) * 2.0 });
        let ground = Ground {
            terrain: Some(&terrain),
            ..inside(&w)
        };
        let mut g = NavGraph::new(
            Vec2::new(0.0, 0.0),
            Vec2::new(20.0, 8.0),
            2.0,
            &Capsule::default(),
        );
        g.build_all(&ground);
        assert!(g.len() > 40);
        let flat_a = (Vec3::new(1.0, 1.0, 0.0), 0);
        let flat_b = (Vec3::new(9.0, 7.0, 0.0), 0);
        assert!(points(ask(&ground, &mut g, flat_a, flat_b, 0.0)).is_some());
        let top = (Vec3::new(18.0, 4.0, 16.0), 0);
        assert!(
            points(ask(&ground, &mut g, flat_a, top, 0.0)).is_none(),
            "climbed the cliff"
        );
    }

    #[test]
    fn a_goal_upstairs_is_routed_to_the_upper_floor_by_its_cell() {
        let w = two_storeys();
        let ground = inside(&w);
        let cap = Capsule::default();
        let mut g = graph(Vec2::new(0.0, -3.0), Vec2::new(12.0, 12.0));
        let start = (Vec3::new(10.0, 0.0, 0.0), 1);
        // Half way between the floors: the cell names which one.
        let up = points(ask(
            &ground,
            &mut g,
            start,
            (Vec3::new(3.0, 0.0, 1.6), 2),
            0.5,
        ))
        .expect("the way up the ramp");
        let end = *up.last().unwrap();
        assert!(
            (end.z - 3.0).abs() < 0.1,
            "ends at {end} for the upper floor"
        );
        assert!(up.iter().any(|p| p.y > 4.0), "not by the ramp: {up:?}");
        assert_every_leg_walks(&ground, start, &up, &cap);
        let down = points(ask(
            &ground,
            &mut g,
            start,
            (Vec3::new(3.0, 0.0, 1.4), 1),
            0.5,
        ))
        .expect("the way along the ground floor");
        assert!(
            down.iter().all(|p| p.z < 0.1),
            "climbed for the ground floor: {down:?}"
        );
    }

    #[test]
    fn a_goal_on_no_floor_of_its_cell_is_no_way() {
        let w = two_storeys();
        let ground = inside(&w);
        let mut g = graph(Vec2::new(0.0, -3.0), Vec2::new(12.0, 12.0));
        // On the ground floor's height, named in the upper floor's cell.
        let found = ask(
            &ground,
            &mut g,
            (Vec3::new(10.0, 0.0, 0.0), 1),
            (Vec3::new(3.0, 0.0, 0.0), 2),
            0.5,
        );
        assert!(matches!(found, Search::NoWay(NoWay::OffFloor)), "{found:?}");
    }

    #[test]
    fn the_route_ends_where_the_body_gets_to_not_at_the_goal() {
        // A goal a hand's breadth in front of a wall, where no body stands.
        let w = two_rooms(None);
        let ground = inside(&w);
        let cap = Capsule::default();
        let mut g = graph(Vec2::new(0.0, -4.0), Vec2::new(16.0, 4.0));
        let start = (Vec3::new(2.0, 0.0, 0.0), 1);
        let goal = Vec3::new(7.8, 1.3, 0.0);
        let path = points(ask(&ground, &mut g, start, (goal, 1), 0.5)).expect("a way near it");
        let end = *path.last().unwrap();
        assert!(end.distance(goal) <= 0.5, "ends {end}, short of {goal}");
        assert!(
            end.distance(goal) > 0.1,
            "ends at the goal itself, in the wall"
        );
        assert!(
            ground.fits_here(end, &cap) || end.x <= 8.0 - cap.radius + 0.05,
            "{end}"
        );
        // Nearer than the body comes is no way at all.
        let found = ask(&ground, &mut g, start, (goal, 1), 0.25);
        assert!(matches!(found, Search::NoWay(_)), "{found:?}");
    }

    #[test]
    fn a_counter_is_reached_across_the_counter() {
        // One room, a waist-high counter right across it at x 6.5 to 7.1, the vendor behind it.
        let mut w = CollisionWorld::default();
        floor(&mut w, 0.0, 12.0, -4.0, 4.0, 0.0, 1);
        for x in [6.5, 7.1] {
            quad(
                &mut w,
                Vec3::new(x, -4.0, 0.0),
                Vec3::new(x, 4.0, 0.0),
                Vec3::new(x, 4.0, 0.9),
                Vec3::new(x, -4.0, 0.9),
                1,
            );
        }
        w.add_tri(
            Vec3::new(6.5, -4.0, 0.9),
            Vec3::new(7.1, -4.0, 0.9),
            Vec3::new(7.1, 4.0, 0.9),
            1,
            true,
        );
        w.add_tri(
            Vec3::new(6.5, -4.0, 0.9),
            Vec3::new(7.1, 4.0, 0.9),
            Vec3::new(6.5, 4.0, 0.9),
            1,
            true,
        );
        wall_x(&mut w, 0.0, -4.0, 4.0, 1);
        let ground = inside(&w);
        let mut g = graph(Vec2::new(0.0, -4.0), Vec2::new(12.0, 4.0));
        let vendor = Vec3::new(8.0, 0.0, 0.0);
        let path = points(ask(
            &ground,
            &mut g,
            (Vec3::new(2.0, 0.0, 0.0), 1),
            (vendor, 1),
            2.0,
        ))
        .expect("across the counter");
        let end = *path.last().unwrap();
        assert!(end.x < 6.5, "went round to {end}, behind the counter");
        assert!(end.distance(vendor) <= 2.0, "ends {end}, out of reach");
    }

    #[test]
    fn a_search_on_unbuilt_chunks_names_them_and_builds_nothing() {
        let w = two_rooms(Some(1.6));
        let ground = inside(&w);
        let mut g = graph(Vec2::new(0.0, -4.0), Vec2::new(16.0, 4.0));
        let q = WalkQuery {
            from: (Vec3::new(1.0, 3.0, 0.0), 1),
            goal: (Vec3::new(15.0, 3.0, 0.0), 2),
            within: 0.0,
            avoid: &[],
            berth: 0.0,
            indoors_too: true,
            drops: false,
        };
        let first = search(&ground, &g, &q);
        assert!(
            matches!(&first, Search::Unbuilt(c) if !c.is_empty()),
            "{first:?}"
        );
        assert_eq!(g.chunk_count(), 0, "it built");
        g.build_all(&ground);
        assert!(matches!(search(&ground, &g, &q), Search::Found(_)));
    }

    #[test]
    fn a_route_on_part_of_a_graph_waits_for_the_chunks_its_search_reached() {
        // One long room, three chunks along x; only the first is built, and the goal stands on
        // its last column, so the search reaches the next chunk before it finds the goal.
        let mut w = CollisionWorld::default();
        floor(&mut w, 0.0, 70.0, 0.0, 6.0, 0.0, 1);
        let ground = inside(&w);
        let q = WalkQuery {
            from: (Vec3::new(2.0, 3.0, 0.0), 1),
            goal: (Vec3::new(22.5, 3.0, 0.0), 1),
            within: 0.5,
            avoid: &[],
            berth: 0.0,
            indoors_too: true,
            drops: false,
        };
        let mut whole = graph(Vec2::new(0.0, 0.0), Vec2::new(70.0, 6.0));
        let Search::Found(want) = search_building(&ground, &mut whole, &q) else {
            panic!("the room is walked across");
        };
        let mut part = graph(Vec2::new(0.0, 0.0), Vec2::new(70.0, 6.0));
        let built = part.build_chunk(&ground, (0, 0));
        part.apply(built);
        let first = search(&ground, &part, &q);
        let Search::Unbuilt(chunks) = first else {
            panic!("a route round the chunk not built yet: {first:?}");
        };
        for c in chunks {
            let built = part.build_chunk(&ground, c);
            part.apply(built);
        }
        let Search::Found(got) = search(&ground, &part, &q) else {
            panic!("built, the room is walked across");
        };
        let at = |p: &[(NavNode, EdgeKind)]| p.iter().map(|(n, _)| n.pos).collect::<Vec<_>>();
        assert_eq!(at(&got), at(&want), "the route the whole graph gives");
    }

    #[test]
    fn a_drop_is_taken_only_when_asked_for() {
        // Off a ledge 3 m up to the floor past it: the only way down is the drop.
        let w = a_ledge_in_cells(Some(3.0));
        let ground = inside(&w);
        let mut g = graph(Vec2::new(0.0, -2.0), Vec2::new(12.0, 2.0));
        let mut q = WalkQuery {
            from: (Vec3::new(1.0, 0.0, 3.0), 0x0101),
            goal: (Vec3::new(10.0, 0.0, 0.0), 0x0102),
            within: 0.5,
            avoid: &[],
            berth: 0.0,
            indoors_too: true,
            drops: false,
        };
        let refused = search_building(&ground, &mut g, &q);
        assert!(matches!(refused, Search::NoWay(_)), "{refused:?}");
        q.drops = true;
        let Search::Found(route) = search_building(&ground, &mut g, &q) else {
            panic!("no way down with drops allowed");
        };
        assert!(route.iter().any(|(_, k)| *k == EdgeKind::Drop), "{route:?}");
    }

    #[test]
    fn a_berth_is_kept_per_walk_not_baked_into_the_graph() {
        // One room with a portal's mouth in its middle: one graph, a walk keeping off it and a walk
        // to it, each answered from the same nodes.
        let mut w = CollisionWorld::default();
        floor(&mut w, 0.0, 12.0, -4.0, 4.0, 0.0, 0x0101);
        let ground = inside(&w);
        let mut g = graph(Vec2::new(0.0, -4.0), Vec2::new(12.0, 4.0));
        g.build_all(&ground);
        let nodes = g.len();
        let mouth = [Vec3::new(6.0, 0.0, 0.0)];
        let mut q = WalkQuery {
            from: (Vec3::new(1.0, 0.0, 0.0), 0x0101),
            goal: (Vec3::new(11.0, 0.0, 0.0), 0x0101),
            within: 0.5,
            avoid: &mouth,
            berth: 2.0,
            indoors_too: true,
            drops: false,
        };
        let Search::Found(round) = search_building(&ground, &mut g, &q) else {
            panic!("no way round the berth");
        };
        let mut prev = q.from.0;
        for (n, _) in &round {
            for k in 0..=10 {
                let p = prev.lerp(n.pos, k as f32 / 10.0);
                assert!(!in_berth(mouth[0], p, 2.0), "into the berth at {p}");
            }
            prev = n.pos;
        }
        q.berth = 0.0;
        let Search::Found(straight) = search_building(&ground, &mut g, &q) else {
            panic!("no straight way");
        };
        assert!(
            straight.len() < round.len(),
            "{straight:?} against {round:?}"
        );
        assert_eq!(g.len(), nodes, "the berth was built into the graph");
    }

    #[test]
    fn a_walk_from_inside_a_berth_leaves_it_and_comes_no_nearer() {
        let mut w = CollisionWorld::default();
        floor(&mut w, 0.0, 12.0, -4.0, 4.0, 0.0, 0x0101);
        let ground = inside(&w);
        let mut g = graph(Vec2::new(0.0, -4.0), Vec2::new(12.0, 4.0));
        let mouth = [Vec3::new(6.0, 0.0, 0.0)];
        let start = Vec3::new(5.0, 0.0, 0.0);
        let q = WalkQuery {
            from: (start, 0x0101),
            goal: (Vec3::new(1.0, 3.0, 0.0), 0x0101),
            within: 0.5,
            avoid: &mouth,
            berth: 3.0,
            indoors_too: true,
            drops: false,
        };
        let Search::Found(route) = search_building(&ground, &mut g, &q) else {
            panic!("stranded in the berth");
        };
        let near = start.distance(mouth[0]);
        for (n, _) in &route {
            assert!(
                n.pos.distance(mouth[0]) >= near - 1e-3,
                "nearer the mouth at {:?}",
                n.pos
            );
        }
    }

    #[test]
    fn rooms_with_no_door_between_are_apart_once_built() {
        let w = two_rooms(None);
        let ground = inside(&w);
        let (from, to) = (
            (Vec3::new(2.0, 0.0, 0.0), 1),
            (Vec3::new(14.0, 0.0, 0.0), 2),
        );
        // Built as the search goes, the rest may yet join them: only searched out.
        let mut lazy = graph(Vec2::new(0.0, -4.0), Vec2::new(32.0, 4.0));
        let found = ask(&ground, &mut lazy, from, to, 0.5);
        assert!(
            !lazy.is_complete(),
            "the fixture is built whole by one search"
        );
        assert!(
            matches!(found, Search::NoWay(NoWay::Unreached)),
            "{found:?}"
        );
        // Built whole: apart before any search.
        let mut whole = graph(Vec2::new(0.0, -4.0), Vec2::new(32.0, 4.0));
        whole.build_all(&ground);
        assert!(whole.is_complete());
        let found = ask(&ground, &mut whole, from, to, 0.5);
        assert!(matches!(found, Search::NoWay(NoWay::Apart)), "{found:?}");
        // And the same room is one component.
        let found = ask(
            &ground,
            &mut whole,
            from,
            (Vec3::new(6.0, 3.0, 0.0), 1),
            0.5,
        );
        assert!(matches!(found, Search::Found(_)), "{found:?}");
    }
}
