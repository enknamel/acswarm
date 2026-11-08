//! A journey of walks, portals (the Town Network, a dungeon's "Surface" portal, a town's own) and
//! recalls the character can cast ([`Recall`], `crate::recalls`: a hop from anywhere for the cast),
//! searched on positions: a walk is priced by its straight line times [`DETOUR`], enough to pick
//! *which* portals; the caller's region planner plans each walk whole. Only a portal whose mouth
//! can be walked to counts, a walk only within [`WALK_REACH`]: across the sea is all portals.

use crate::portals::{self, Portal};
use crate::Place;
use glam::Vec2;
use std::collections::{BinaryHeap, HashSet};
use std::sync::OnceLock;

/// Indoor cells portals stand or land in that are buildings, not dungeons, told once by the
/// client, which reads the cells (`set_building_cells`): walked to as the ground outside them.
static BUILDING_CELLS: OnceLock<HashSet<u32>> = OnceLock::new();

/// Tell the planner which indoor portal cells are in buildings (the first telling stands).
pub fn set_building_cells(cells: HashSet<u32>) {
    let _ = BUILDING_CELLS.set(cells);
}

/// Whether the planner has been told the building cells.
pub fn building_cells_known() -> bool {
    BUILDING_CELLS.get().is_some()
}

/// Indoors and not in a building: a dungeon, or the hub, walked only within its own landblock.
fn shut_in(p: Place) -> bool {
    p.is_indoors() && !BUILDING_CELLS.get().is_some_and(|s| s.contains(&p.cell))
}

/// How fast the character covers ground (m/s), for pricing walks.
pub const WALK_SPEED: f32 = 5.0;
/// A walk is longer than the straight line between its ends.
pub const DETOUR: f32 = 1.3;
/// Taking a portal costs this many seconds (walking into it, the load).
pub const PORTAL_SECONDS: f32 = 20.0;
/// Casting a recall costs this many seconds: coming to a stop, the cast
/// itself, the server's two-second pause before the teleport, the load.
pub const RECALL_SECONDS: f32 = 10.0;
/// The farthest one walk is believed (metres); longer, and the trip has to find a portal.
pub const WALK_REACH: f32 = 1200.0;
/// Portals this far from a spot are candidates for the next hop.
pub const PORTAL_REACH: f32 = 900.0;
/// A walk refused from a spot is refused from a node this near it (metres, flat): where a portal
/// leaves the character is counted as arrived within 30 m (`ac-client` `travel.rs`).
const NO_WALK_NEAR: f32 = 30.0;

/// A walk the caller's region planner found no way for, from where the character stood to its
/// end: the next plan walks there from nowhere near `from`, in its block and inside or not alike.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoWalk {
    pub from: Place,
    pub to: Place,
}

impl NoWalk {
    /// Whether this refusal rules out walking from `a` to `b`.
    fn is_walk(&self, a: Place, b: Place) -> bool {
        let near = |p: Place, q: Place, metres: f32| {
            p.cell & 0xFFFF_0000 == q.cell & 0xFFFF_0000
                && p.is_indoors() == q.is_indoors()
                && p.at.truncate().distance(q.at.truncate()) <= metres
        };
        near(a, self.from, NO_WALK_NEAR) && near(b, self.to, 2.0)
    }
}

/// A recall spell the character can cast right now, and where it lands:
/// what the caller hands the planner. The destination of a named
/// recall comes from `crate::recalls::fixed`; that of Lifestone Sending
/// or Portal Recall is one of the character's saved positions.
#[derive(Clone, Debug, PartialEq)]
pub struct Recall {
    pub spell: u32,
    pub name: String,
    pub exit: Place,
}

/// A portal gem the character is carrying, and where using it lands
/// them. The same shape as a [`Recall`] and used the same way, except
/// that the gem is spent: a journey uses each one once.
#[derive(Clone, Debug, PartialEq)]
pub struct Gem {
    /// The carried item, so the caller knows which one to use.
    pub guid: u32,
    pub name: String,
    pub exit: Place,
    /// Whether using it summons a portal to walk into, or moves the
    /// character itself. Nearly all of them summon, which is two
    /// actions rather than one and is priced as such.
    pub summons: bool,
}

/// One step of a journey.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Walk to this place: the journey's end.
    Walk(Place),
    /// Use a carried portal gem and come out at its exit. Most gems
    /// summon a portal that then has to be walked into; a few move the
    /// character themselves.
    Gem {
        guid: u32,
        name: String,
        exit: Place,
        summons: bool,
    },
    /// Walk into this portal's mouth and come out at its exit.
    Portal {
        name: String,
        mouth: Place,
        exit: Place,
    },
    /// Stand still, cast this spell, and come out at its destination.
    Recall {
        spell: u32,
        name: String,
        exit: Place,
    },
}

impl Step {
    /// Where this step leaves the character.
    pub fn end(&self) -> Place {
        match self {
            Step::Walk(p) => *p,
            Step::Portal { exit, .. } | Step::Recall { exit, .. } | Step::Gem { exit, .. } => *exit,
        }
    }

    /// Where a portal, recall or gem comes out: `None` for a walk.
    pub fn exit(&self) -> Option<Place> {
        match self {
            Step::Walk(_) => None,
            Step::Portal { exit, .. } | Step::Recall { exit, .. } | Step::Gem { exit, .. } => {
                Some(*exit)
            }
        }
    }

    /// Where the step walks to: a walk's place, a portal's mouth; a recall or gem walks nowhere.
    pub fn walk_end(&self) -> Option<Place> {
        match self {
            Step::Walk(p) => Some(*p),
            Step::Portal { mouth, .. } => Some(*mouth),
            Step::Recall { .. } | Step::Gem { .. } => None,
        }
    }
}

/// A planned journey and what it costs.
#[derive(Clone, Debug, PartialEq)]
pub struct Trip {
    pub steps: Vec<Step>,
    /// Rough time in seconds.
    pub seconds: f32,
}

impl Trip {
    /// How many portals it takes.
    pub fn portals(&self) -> usize {
        self.steps
            .iter()
            .filter(|s| matches!(s, Step::Portal { .. }))
            .count()
    }

    /// How many recall spells it casts.
    pub fn recalls(&self) -> usize {
        self.steps
            .iter()
            .filter(|s| matches!(s, Step::Recall { .. }))
            .count()
    }

    /// A line for the player: "on foot, about 3 min", "2 portals, about
    /// 4 min", "Lifestone Sending, then one portal, about 2 min".
    pub fn summary(&self) -> String {
        let mins = (self.seconds / 60.0).round() as u32;
        let recall = self.steps.iter().find_map(|s| match s {
            Step::Recall { name, .. } => Some(name.as_str()),
            _ => None,
        });
        let hops = match self.portals() {
            0 => "on foot".to_string(),
            1 => "one portal".to_string(),
            n => format!("{n} portals"),
        };
        match (recall, self.portals()) {
            (None, _) => format!("{hops}, about {mins} min"),
            (Some(r), 0) => format!("{r}, about {mins} min"),
            (Some(r), _) => format!("{r}, then {hops}, about {mins} min"),
        }
    }
}

fn walk_seconds(a: Place, b: Place) -> f32 {
    a.at.truncate().distance(b.at.truncate()) * DETOUR / WALK_SPEED
}

/// Whether a leg on foot between two spots is believable: short enough,
/// and not between an indoor cell and somewhere else (the character
/// cannot walk out of the Town Network hub into the countryside).
/// The same, told whether `a` is inside a dungeon. From one, nothing is
/// walkable but the rest of the dungeon: the way out is a portal or a
/// recall, never a stroll.
fn can_walk_from(a: Place, b: Place, reach: f32, in_dungeon: bool) -> bool {
    if in_dungeon {
        return b.is_indoors() && a.cell & 0xFFFF_0000 == b.cell & 0xFFFF_0000;
    }
    can_walk_plain(a, b, reach)
}

fn can_walk_plain(a: Place, b: Place, reach: f32) -> bool {
    // A place known by its position alone (`Place::on_map`) is outdoors in the block its
    // position lies in: from inside a shop in that block, the walk out of the door is a walk.
    let same_block = a.cell & 0xFFFF_0000 == b.cell & 0xFFFF_0000;
    // A building's door opens on the ground: the Mines of Despair portal stands in one at
    // 0xC7210100 (the_mines_of_despair_are_reached_through_the_portal_in_a_building).
    if shut_in(a) || shut_in(b) {
        // Inside, only within the same landblock (the hub, a dungeon).
        return same_block;
    }
    a.at.truncate().distance(b.at.truncate()) <= reach
}

#[derive(PartialEq)]
struct Queued {
    cost: f32,
    est: f32,
    idx: usize,
}

impl Eq for Queued {}

impl Ord for Queued {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // A min-heap on cost + estimate.
        other
            .est
            .total_cmp(&self.est)
            .then_with(|| other.cost.total_cmp(&self.cost))
    }
}

impl PartialOrd for Queued {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// How a journey should be chosen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prefs {
    /// What one portal is reckoned to cost, in seconds. Raise it and the
    /// journey takes fewer, longer hops; lower it and it hops more.
    pub portal_seconds: f32,
    /// Give up on a chain longer than this many portals.
    pub max_portals: usize,
    /// The farthest a single leg on foot is believed, metres. A player
    /// will walk a long way between towns; too small a figure here and
    /// journeys that plainly exist are reported as impossible.
    pub walk_reach: f32,
    /// How far from a spot a portal is still a candidate for the next
    /// hop, metres.
    pub portal_reach: f32,
    /// What casting a recall is reckoned to cost, in seconds.
    pub recall_seconds: f32,
    /// Whether to cast recall spells at all. Off, the recalls handed to
    /// the planner are ignored: a corpse run that must not leave the
    /// dungeon, a character saving its components.
    pub use_recalls: bool,
    /// The character is in a dungeon, so there is no walking out.
    ///
    /// A dungeon shares its landblock number with the ground above it,
    /// and the only thing telling them apart is knowing which you are
    /// in -- which the client does and this table does not. Without
    /// being told, the planner reads a vendor a hundred metres up as a
    /// hundred-metre stroll and sets off into the nearest wall. A
    /// building is not the same thing: you really can walk out of a
    /// shop door, so only a dungeon sets this.
    pub in_dungeon: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs::quick()
    }
}

impl Prefs {
    /// The quickest way, however many portals it takes.
    pub fn quick() -> Self {
        Prefs {
            portal_seconds: PORTAL_SECONDS,
            max_portals: 8,
            walk_reach: WALK_REACH,
            portal_reach: PORTAL_REACH,
            recall_seconds: RECALL_SECONDS,
            use_recalls: true,
            in_dungeon: false,
        }
    }

    /// The same journey, knowing the character is underground and
    /// cannot simply walk out.
    pub fn in_dungeon(self, yes: bool) -> Self {
        Prefs {
            in_dungeon: yes,
            ..self
        }
    }

    /// The same journey without casting anything.
    pub fn without_recalls(self) -> Self {
        Prefs {
            use_recalls: false,
            ..self
        }
    }

    /// Fewer hops: every portal is a chance to be turned away, to land
    /// somewhere unexpected, or to walk into something on the way.
    pub fn steady() -> Self {
        Prefs {
            portal_seconds: PORTAL_SECONDS * 6.0,
            max_portals: 3,
            ..Prefs::quick()
        }
    }

    /// Walk as far as it takes: the last resort when nothing shorter
    /// reaches the goal.
    pub fn far() -> Self {
        Prefs {
            walk_reach: 12_000.0,
            portal_reach: 8_000.0,
            ..Prefs::quick()
        }
    }
}

/// Plan a journey from `from` to `goal` for a character of any level,
/// ignoring what portals ask for.
pub fn plan(from: Place, goal: Place) -> Option<Trip> {
    plan_for(from, goal, 0, &[], &[])
}

/// Plan a journey the character can actually make: portals with a level
/// range outside `level`, or a quest not in `quests_done`, are left out,
/// as are the ones standing at any of `avoid` (mouths that have already
/// turned us away). Hundreds of portals share a name -- every dungeon
/// has a "Surface Portal" -- so one is named by where it stands.
///
/// Returns `None` when nothing reaches the goal: no walk is short enough
/// and no portal it can take comes out near it.
pub fn plan_for(
    from: Place,
    goal: Place,
    level: u32,
    quests_done: &[String],
    avoid: &[Vec2],
) -> Option<Trip> {
    plan_with(from, goal, level, quests_done, avoid, Prefs::default())
}

/// [`plan_for`] with the journey's preferences (see [`Prefs`]).
pub fn plan_with(
    from: Place,
    goal: Place,
    level: u32,
    quests_done: &[String],
    avoid: &[Vec2],
    prefs: Prefs,
) -> Option<Trip> {
    plan_with_recalls(from, goal, level, quests_done, avoid, &[], prefs)
}

/// Which way a node of the search was reached.
#[derive(Clone, Copy)]
enum Edge {
    Portal(usize),
    Recall(usize),
    Gem(usize),
}

/// [`plan_with`] for a character that can cast `recalls` right now
/// (known, with the components and mana, and somewhere for each to
/// go). A recall is a hop from the start straight to its destination
/// for [`Prefs::recall_seconds`]; it is taken when that beats walking
/// and portals, and the journey goes on from where it lands, through
/// portals if need be. One cast at most: a second recall from where the
/// first lands would have been no better cast from the start.
pub fn plan_with_recalls(
    from: Place,
    goal: Place,
    level: u32,
    quests_done: &[String],
    avoid: &[Vec2],
    recalls: &[Recall],
    prefs: Prefs,
) -> Option<Trip> {
    plan_with_recalls_and_gems(
        from,
        goal,
        level,
        quests_done,
        avoid,
        &[],
        recalls,
        &[],
        prefs,
    )
}

/// [`plan_with_recalls`] for a character also carrying portal gems. A
/// gem is a hop from the start to where it lands, like a recall, and
/// like a recall a journey can go on from there. It costs the same as a
/// recall to use and is spent when it is.
///
/// The goal's cell says which block it lies in: a dungeon's rooms reach outside the square of
/// its landblock, so a position alone can name the block next door (a corpse in the Holtburg
/// Dungeon's armoredillo rooms read as 0x01F5, and there was no way back to it).
/// The walks in `no_walks` are left out, as the mouths at `avoid` are.
#[allow(clippy::too_many_arguments)]
pub fn plan_with_recalls_and_gems(
    from: Place,
    goal: Place,
    level: u32,
    quests_done: &[String],
    avoid: &[Vec2],
    no_walks: &[NoWalk],
    recalls: &[Recall],
    gems: &[Gem],
    prefs: Prefs,
) -> Option<Trip> {
    let recalls: &[Recall] = if prefs.use_recalls { recalls } else { &[] };
    let can_walk = |a: Place, b: Place, stuck: bool| {
        can_walk_from(a, b, prefs.walk_reach, stuck) && !no_walks.iter().any(|n| n.is_walk(a, b))
    };
    // Straight there, when that is a believable walk and there is no
    // spell that might be quicker.
    if recalls.is_empty() && can_walk(from, goal, prefs.in_dungeon) {
        return Some(Trip {
            steps: vec![Step::Walk(goal)],
            seconds: walk_seconds(from, goal),
        });
    }
    // Otherwise search over portal exits. A node is a place to stand:
    // the start, or where a portal comes out. Only the ones this
    // character may take are in the search: walking to a portal that
    // turns us away wastes the whole trip.
    // Landblocks that have a portal standing in them: coming out inside
    // one of those is safe, because there is a way on. A portal whose
    // exit is indoors somewhere with no portal of its own is a one-way
    // trip into a dungeon the planner cannot walk out of.
    let mut has_portal = std::collections::HashSet::new();
    for p in portals::all() {
        has_portal.insert(p.from_cell & 0xFFFF_0000);
    }
    let usable: Vec<&Portal> = portals::all()
        .iter()
        .filter(|p| p.works())
        .filter(|p| level == 0 || p.usable_by(level, quests_done))
        .filter(|p| !avoid.iter().any(|a| a.distance(p.from_xy()) < 2.0))
        .filter(|p| p.exit_outdoors() || has_portal.contains(&(p.to_cell & 0xFFFF_0000)))
        .collect();
    let portals = &usable[..];
    let mouth_of = |p: &Portal| Place {
        at: p.from,
        cell: p.from_cell,
    };
    let exit_of = |p: &Portal| Place {
        at: p.to,
        cell: p.to_cell,
    };
    let mut nodes = vec![from];
    let mut came: Vec<Option<(usize, Edge)>> = vec![None]; // (node, how)
    let mut cost = vec![0.0f32];
    let mut queue = BinaryHeap::new();
    queue.push(Queued {
        cost: 0.0,
        est: walk_seconds(from, goal),
        idx: 0,
    });
    let mut seen_portal = vec![false; portals.len()];
    let mut best: Option<(f32, usize)> = None;
    let mut expanded = 0;
    while let Some(q) = queue.pop() {
        if q.cost > cost[q.idx] {
            continue;
        }
        let node = nodes[q.idx];
        // Could we simply walk the rest of the way?
        // Only the start is known to be underground; a node reached
        // by a portal has been walked to from wherever that portal
        // came out.
        let stuck =
            prefs.in_dungeon && node.at.truncate() == from.at.truncate() && node.cell == from.cell;
        if can_walk(node, goal, stuck) {
            let total = q.cost + walk_seconds(node, goal);
            if best.map(|(b, _)| total < b).unwrap_or(true) {
                best = Some((total, q.idx));
            }
        }
        if best.map(|(b, _)| q.cost >= b).unwrap_or(false) {
            continue;
        }
        expanded += 1;
        if expanded > 4000 {
            break;
        }
        // Cast a recall: from the start only, since a spell that goes to
        // the same place from anywhere is never better cast later.
        if q.idx == 0 {
            for (ri, r) in recalls.iter().enumerate() {
                let next = q.cost + prefs.recall_seconds;
                if best.map(|(b, _)| next >= b).unwrap_or(false) {
                    continue;
                }
                nodes.push(r.exit);
                came.push(Some((q.idx, Edge::Recall(ri))));
                cost.push(next);
                let idx = nodes.len() - 1;
                queue.push(Queued {
                    cost: next,
                    est: next + walk_seconds(r.exit, goal),
                    idx,
                });
            }
        }
        // Use a carried gem: from the start, for the same reason a
        // recall is. A gem is spent, so each is offered once.
        if q.idx == 0 {
            for (gi, g) in gems.iter().enumerate() {
                // Using it, and for the usual kind walking into the
                // portal it puts in front of you.
                let mut next = q.cost + prefs.recall_seconds;
                if g.summons {
                    next += prefs.portal_seconds;
                }
                if best.map(|(b, _)| next >= b).unwrap_or(false) {
                    continue;
                }
                nodes.push(g.exit);
                came.push(Some((q.idx, Edge::Gem(gi))));
                cost.push(next);
                let idx = nodes.len() - 1;
                queue.push(Queued {
                    cost: next,
                    est: next + walk_seconds(g.exit, goal),
                    idx,
                });
            }
        }
        // Take a portal whose mouth we can reach from here.
        for (pi, p) in portals.iter().enumerate() {
            if seen_portal[pi] {
                continue;
            }
            let mouth = mouth_of(p);
            if !can_walk(node, mouth, stuck) {
                continue;
            }
            let far = node.at.truncate().distance(p.from_xy()) > prefs.portal_reach;
            if far && !node.is_indoors() {
                continue;
            }
            let step_cost = walk_seconds(node, mouth) + prefs.portal_seconds;
            let next = q.cost + step_cost;
            if best.map(|(b, _)| next >= b).unwrap_or(false) {
                continue;
            }
            seen_portal[pi] = true;
            nodes.push(exit_of(p));
            came.push(Some((q.idx, Edge::Portal(pi))));
            cost.push(next);
            let idx = nodes.len() - 1;
            queue.push(Queued {
                cost: next,
                est: next + walk_seconds(exit_of(p), goal),
                idx,
            });
        }
    }
    let (seconds, end) = best?;
    // Too long a chain of portals is more chance to go wrong than it is
    // worth; the caller asked for fewer.
    let mut hops = 0;
    let mut at = end;
    while let Some((prev, how)) = came[at] {
        if matches!(how, Edge::Portal(_)) {
            hops += 1;
        }
        at = prev;
    }
    if hops > prefs.max_portals {
        return None;
    }
    // Walk back through the portals taken and the spell cast.
    let mut steps = Vec::new();
    let mut at = end;
    while let Some((prev, how)) = came[at] {
        steps.push(match how {
            Edge::Portal(pi) => {
                let p: &Portal = portals[pi];
                Step::Portal {
                    name: p.name.clone(),
                    mouth: mouth_of(p),
                    exit: exit_of(p),
                }
            }
            Edge::Recall(ri) => {
                let r = &recalls[ri];
                Step::Recall {
                    spell: r.spell,
                    name: r.name.clone(),
                    exit: r.exit,
                }
            }
            Edge::Gem(gi) => {
                let g = &gems[gi];
                Step::Gem {
                    guid: g.guid,
                    name: g.name.clone(),
                    exit: g.exit,
                    summons: g.summons,
                }
            }
        });
        at = prev;
    }
    steps.reverse();
    steps.push(Step::Walk(goal));
    Some(Trip { steps, seconds })
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn there_is_no_walking_out_of_a_dungeon() {
        // A dungeon lies under the landblock it belongs to, so a
        // counter a hundred metres up is a hundred metres away and a
        // hundred metres of rock in between. Told nothing, the planner
        // calls that a stroll -- which is a character setting off into
        // the nearest wall saying it is going to the shops.
        let deep = Place {
            at: Vec3::new(32_500.0, 34_600.0, 0.0),
            cell: 0xA9B4_0120,
        };
        let surface = Place::on_map(Vec2::new(32_560.0, 34_660.0));
        assert!(
            can_walk_plain(deep, surface, 1000.0),
            "a building's door is still a walk"
        );
        assert!(
            !can_walk_from(deep, surface, 1000.0, true),
            "walked out of a dungeon"
        );
        // The rest of the dungeon is still walkable.
        let room = Place {
            at: Vec3::new(32_510.0, 34_610.0, 0.0),
            cell: 0xA9B4_0135,
        };
        assert!(can_walk_from(deep, room, 1000.0, true));
    }

    #[test]
    fn a_walk_out_of_a_shop_to_its_own_block_is_a_walk() {
        // Inside a Holtburg shop (cell A9B4016A), bound for a spot
        // outdoors in the same block (cell unknown): a walk.
        let xy = Vec2::new(0xA9 as f32 * 192.0 + 100.0, 0xB4 as f32 * 192.0 + 100.0);
        let inside = Place {
            at: xy.extend(0.0),
            cell: 0xA9B4_016A,
        };
        let outside = Place::on_map(xy + Vec2::new(20.0, 10.0));
        assert!(can_walk_plain(inside, outside, 1000.0));
        // The next block over is not walked to from indoors.
        let next = Place::on_map(xy + Vec2::new(192.0, 0.0));
        assert!(!can_walk_plain(inside, next, 1000.0));
    }
    use crate::towns;

    fn place(name: &str) -> Place {
        Place::on_map(towns::find(name).unwrap().world_xy())
    }

    /// Where the Holtburg lifestone stands.
    fn holtburg() -> Place {
        Place {
            at: crate::landblock_origin(0xA9B4_0019) + Vec3::new(84.0, 7.1, 94.0),
            cell: 0xA9B4_0019,
        }
    }

    fn walk_by(p: Place, d: Vec2) -> Place {
        Place::on_map(p.at.truncate() + d)
    }

    #[test]
    fn a_short_hop_is_walked() {
        let from = holtburg();
        let near = walk_by(from, Vec2::new(120.0, 60.0));
        let t = plan(from, near).unwrap();
        assert_eq!(t.steps, vec![Step::Walk(near)]);
        assert_eq!(t.portals(), 0);
        assert!(t.summary().starts_with("on foot"));
    }

    #[test]
    fn holtburg_to_arwic_takes_portals() {
        let from = holtburg();
        let goal = place("Arwic");
        let t0 = std::time::Instant::now();
        let trip = plan(from, goal).expect("no trip to Arwic");
        let took = t0.elapsed();
        assert!(took.as_secs_f32() < 5.0, "planning took {took:?}");
        assert!(trip.portals() >= 1, "should use a portal: {trip:?}");
        // It ends by walking to the goal, and every portal step follows
        // one the character could reach.
        assert_eq!(trip.steps.last(), Some(&Step::Walk(goal)));
        // And it is quicker than the long walk.
        assert!(
            trip.seconds < walk_seconds(from, goal),
            "{} s by portal vs {} s on foot",
            trip.seconds,
            walk_seconds(from, goal)
        );
        // The first hop is a portal the character can reach on foot.
        if let Some(Step::Portal { mouth, .. }) = trip.steps.first() {
            let away = from.at.truncate().distance(mouth.at.truncate());
            assert!(away <= PORTAL_REACH, "first portal is {away:.0} m away");
        }
    }

    #[test]
    fn a_portal_that_turns_us_away_is_left_out() {
        let from = holtburg();
        let goal = place("Arwic");
        let trip = plan_for(from, goal, 20, &[], &[]).expect("no trip");
        // Nothing in the plan asks for more than we have.
        for step in &trip.steps {
            if let Step::Portal { name, .. } = step {
                for p in crate::portals::named(name) {
                    if p.from_xy().distance(from.at.truncate()) < 1e-3 {
                        assert!(p.usable_by(20, &[]), "{name} is not usable at level 20");
                    }
                }
            }
        }
        // Told that the first portal refused us, it finds another way.
        let first = match trip.steps.first() {
            Some(Step::Portal { mouth, .. }) => mouth.at.truncate(),
            other => panic!("expected a portal first, got {other:?}"),
        };
        let again = plan_for(from, goal, 20, &[], &[first]);
        if let Some(t) = again {
            assert!(
                !matches!(t.steps.first(), Some(Step::Portal { mouth, .. }) if mouth.at.truncate() == first),
                "took the refused portal again"
            );
        }
    }

    #[test]
    fn a_walk_refused_from_a_spot_is_planned_from_nowhere_near_it() {
        let from = holtburg();
        let goal = place("Arwic");
        let plan_without = |no_walks: &[NoWalk]| {
            plan_with_recalls_and_gems(
                from,
                goal,
                20,
                &[],
                &[],
                no_walks,
                &[],
                &[],
                Prefs::default(),
            )
        };
        let trip = plan_without(&[]).expect("no trip");
        let last_exit = trip.steps.iter().rev().find_map(Step::exit).expect("a hop");
        // The last walk refused from where the last portal leaves us: no plan walks it from there.
        let refused = NoWalk {
            from: last_exit,
            to: goal,
        };
        if let Some(t) = plan_without(&[refused]) {
            let exit = t.steps.iter().rev().find_map(Step::exit).expect("a hop");
            assert!(!refused.is_walk(exit, goal), "walked from {exit:?} again");
        }
        // A short hop refused from the start is not walked from there.
        let near = walk_by(from, Vec2::new(120.0, 60.0));
        let refused = NoWalk { from, to: near };
        let t = plan_with_recalls_and_gems(
            from,
            near,
            0,
            &[],
            &[],
            &[refused],
            &[],
            &[],
            Prefs::default(),
        );
        assert!(
            t.is_none_or(|t| t.steps != vec![Step::Walk(near)]),
            "walked again"
        );
    }

    fn recall_to(name: &str, spell: u32, exit: Place) -> Recall {
        Recall {
            spell,
            name: name.to_string(),
            exit,
        }
    }

    #[test]
    fn a_recall_makes_a_far_journey_short() {
        let from = holtburg();
        let goal = place("Arwic");
        let by_portal = plan(from, goal).expect("no trip to Arwic");
        // A spell that lands a short walk from Arwic beats the Town Network.
        let recalls = [recall_to(
            "Arwic Recall",
            9001,
            walk_by(goal, Vec2::new(80.0, 0.0)),
        )];
        let trip = plan_with_recalls(from, goal, 0, &[], &[], &recalls, Prefs::quick())
            .expect("no trip with the recall");
        assert!(
            matches!(&trip.steps[0], Step::Recall { spell: 9001, name, .. } if name == "Arwic Recall"),
            "{trip:?}"
        );
        assert_eq!(trip.steps.last(), Some(&Step::Walk(goal)));
        assert_eq!(trip.portals(), 0);
        assert_eq!(trip.recalls(), 1);
        assert!(
            trip.seconds < by_portal.seconds,
            "{} vs {}",
            trip.seconds,
            by_portal.seconds
        );
        assert!(
            trip.summary().starts_with("Arwic Recall"),
            "{}",
            trip.summary()
        );
        assert_eq!(trip.steps[0].exit(), Some(recalls[0].exit));
        // Switched off, the same journey goes by portal.
        let walked = plan_with_recalls(
            from,
            goal,
            0,
            &[],
            &[],
            &recalls,
            Prefs::quick().without_recalls(),
        )
        .expect("no trip without the recall");
        assert_eq!(walked.recalls(), 0);
        assert_eq!(walked.steps, by_portal.steps);
    }

    #[test]
    fn a_recall_that_lands_farther_than_walking_is_not_used() {
        let from = holtburg();
        let near = walk_by(from, Vec2::new(150.0, 80.0));
        // Lands on the far side of the continent: worse than the walk.
        let far = [recall_to("Yaraq Recall", 9002, place("Yaraq"))];
        let trip =
            plan_with_recalls(from, near, 0, &[], &[], &far, Prefs::quick()).expect("no trip");
        assert_eq!(trip.steps, vec![Step::Walk(near)]);
        // Lands where we already stand: the cast is wasted.
        let here = [recall_to("Holtburg Recall", 9003, from)];
        let trip =
            plan_with_recalls(from, near, 0, &[], &[], &here, Prefs::quick()).expect("no trip");
        assert_eq!(trip.steps, vec![Step::Walk(near)]);
        let goal = place("Arwic");
        let trip =
            plan_with_recalls(from, goal, 0, &[], &[], &here, Prefs::quick()).expect("no trip");
        assert_eq!(trip.recalls(), 0, "{trip:?}");
        assert!(trip.portals() >= 1);
    }

    #[test]
    fn a_recall_can_be_followed_by_portals() {
        // From the open sea nothing can be walked to; a recall to
        // Holtburg puts the Town Network in reach of Arwic.
        let sea = Place::on_map(Vec2::new(0x60 as f32 * 192.0, 0x70 as f32 * 192.0));
        let goal = place("Arwic");
        let recalls = [recall_to("Lifestone Sending", 1636, holtburg())];
        let trip = plan_with_recalls(sea, goal, 0, &[], &[], &recalls, Prefs::quick())
            .expect("no trip from the sea");
        assert!(
            matches!(&trip.steps[0], Step::Recall { spell: 1636, .. }),
            "{trip:?}"
        );
        assert!(trip.portals() >= 1, "{trip:?}");
        assert!(
            trip.summary().starts_with("Lifestone Sending, then"),
            "{}",
            trip.summary()
        );
        assert_eq!(trip.steps.last(), Some(&Step::Walk(goal)));
    }

    #[test]
    fn a_warden_in_the_facility_hub_is_planned_through_its_portal() {
        // The Warden of fachubeasthamportals_flag (unlocks.csv): its rooms run south of the hub's
        // square, so its position alone names the block below, open ground with no walk to it.
        let local = Vec3::new(108.34, -46.2, 0.0);
        let warden = Place {
            at: crate::landblock_origin(0x8A02_01F7) + local,
            cell: 0x8A02_01F7,
        };
        let trip = plan(holtburg(), warden).expect("no way to the Warden");
        assert_eq!(trip.portals(), 2, "{trip:?}");
        assert!(
            matches!(&trip.steps[1], Step::Portal { exit, .. } if exit.cell == 0x8A02_0212),
            "not through the Facility Hub portal: {trip:?}"
        );
        assert_eq!(trip.steps.last(), Some(&Step::Walk(warden)));
        assert!(trip.seconds < 5.0 * 60.0, "{}", trip.summary());
        // On the map alone it is the long way round, by Qalaba'r and a walk off the coast.
        let on_map = Place::on_map(warden.at.truncate());
        assert_eq!(on_map.cell >> 16, 0x8A01);
        let far = plan_with(holtburg(), on_map, 0, &[], &[], Prefs::far()).expect("a far way");
        assert!(far.seconds > 20.0 * 60.0, "{}", far.summary());
    }

    #[test]
    fn nothing_reaches_the_open_sea() {
        let from = holtburg();
        // The middle of the inland sea, far from any portal exit.
        let sea = Place::on_map(Vec2::new(0x60 as f32 * 192.0, 0x70 as f32 * 192.0));
        let trip = plan(from, sea);
        assert!(trip.is_none(), "planned a trip to the sea: {trip:?}");
    }
    #[test]
    fn a_carried_gem_is_a_way_to_get_there() {
        // A gem in the pack is a hop straight to where it lands, and a
        // journey goes on from there: the whole point, since a gem
        // rarely drops you exactly on the spot you wanted.
        let goal = Place::on_map(Vec2::new(1_691.0, 1_757.0));
        let gem = Gem {
            guid: 0x8000_0001,
            name: "Somewhere Portal Gem".into(),
            exit: Place {
                at: Vec3::new(1_739.0, 957.0, 0.0),
                cell: 0x0904_0008,
            },
            summons: true,
        };
        let far = holtburg();
        // On foot it is hopeless: half the world away.
        assert!(plan_with_recalls(far, goal, 275, &[], &[], &[], Prefs::default()).is_none());
        // With the gem it is one hop and a walk.
        let trip = plan_with_recalls_and_gems(
            far,
            goal,
            275,
            &[],
            &[],
            &[],
            &[],
            std::slice::from_ref(&gem),
            Prefs::default(),
        )
        .expect("a way there");
        assert!(
            trip.steps
                .iter()
                .any(|s| matches!(s, Step::Gem { guid, .. } if *guid == gem.guid)),
            "{:?}",
            trip.steps
        );
        // And it ends where we asked, not where the gem dropped us.
        assert_eq!(trip.steps.last().map(|s| s.end()), Some(goal));
    }

    #[test]
    fn a_gem_that_lands_no_nearer_is_not_used() {
        // Using one up for nothing is worse than walking.
        let here = Place {
            at: Vec3::new(1_700.0, 1_700.0, 0.0),
            cell: 0x0904_0008,
        };
        let goal = Place::on_map(Vec2::new(1_720.0, 1_720.0));
        let gem = Gem {
            guid: 1,
            name: "Wrong Way Gem".into(),
            exit: Place {
                at: Vec3::new(30_000.0, 30_000.0, 0.0),
                cell: 0x1234_0001,
            },
            summons: true,
        };
        let trip = plan_with_recalls_and_gems(
            here,
            goal,
            275,
            &[],
            &[],
            &[],
            &[],
            std::slice::from_ref(&gem),
            Prefs::default(),
        )
        .expect("a short walk");
        assert!(
            !trip.steps.iter().any(|s| matches!(s, Step::Gem { .. })),
            "{:?}",
            trip.steps
        );
    }
    #[test]
    fn a_gem_that_summons_costs_more_than_one_that_moves_you() {
        // Summoning puts a portal in front of you and you still have to
        // walk into it. Priced the same, a planner would reach for the
        // slower one as readily as the quicker.
        let far = holtburg();
        let goal = Place::on_map(Vec2::new(1_691.0, 1_757.0));
        let make = |summons| Gem {
            guid: 1,
            name: "A Gem".into(),
            exit: Place {
                at: Vec3::new(1_739.0, 957.0, 0.0),
                cell: 0x0904_0008,
            },
            summons,
        };
        let cost = |summons| {
            plan_with_recalls_and_gems(
                far,
                goal,
                275,
                &[],
                &[],
                &[],
                &[],
                &[make(summons)],
                Prefs::default(),
            )
            .expect("a way there")
            .seconds
        };
        assert!(cost(true) > cost(false), "summoning was not dearer");
    }
}
