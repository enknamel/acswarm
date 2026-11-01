//! A walk planned whole before a step is taken: from one [`Place`] to another on an area's
//! graph, every waypoint with its cell and the kind of step into it, or why there is no way.
//! Entry: [`plan`].

use ac_scene::nav::{EdgeKind, Ground, NavGraph, NavNode};
use ac_scene::navarea::{block_at, Area};
use ac_scene::wayfind::{self, Search, WalkQuery};
use ac_world::Place;
use glam::Vec3;

pub use ac_scene::wayfind::NoWay;

/// How close a walk may come to a portal it is not going to.
pub const PORTAL_BERTH: f32 = 3.0;

/// The mouths of the portals within reach of a walk from `from` to
/// `to`, except any the walk is going to (within a stride of `to`).
pub fn portal_mouths_to_avoid(from: Vec3, to: Vec3) -> Vec<Vec3> {
    let (f, t) = (glam::Vec2::new(from.x, from.y), glam::Vec2::new(to.x, to.y));
    let mid = (f + t) * 0.5;
    let reach = f.distance(t) * 0.5 + 200.0;
    ac_world::portals::near(mid, reach)
        .into_iter()
        .filter(|p| p.from_xy().distance(t) > PORTAL_BERTH + 1.5)
        .map(|p| p.from)
        .collect()
}

/// One point of a route: where it is, and the kind of edge that leads there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Waypoint {
    pub place: Place,
    pub kind: EdgeKind,
}

/// A planned walk: every waypoint from the first step to the end, the last within reach of the
/// goal, each leg checked by the walk test.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub waypoints: Vec<Waypoint>,
}

impl Route {
    /// The waypoints' positions, in order.
    pub fn points(&self) -> Vec<Vec3> {
        self.waypoints.iter().map(|w| w.place.at).collect()
    }
}

/// A place as the graph names it: its position, and its interior cell or 0 outdoors. A place
/// that names only its block is located by the cells' own BSPs, never by the nearest node.
pub fn graph_end(ground: &Ground, p: Place) -> (Vec3, u32) {
    let cell = if p.is_indoors() {
        p.cell
    } else if p.is_located() {
        0
    } else {
        wayfind::locate(ground, p.at)
    };
    (p.at, cell)
}

/// The place of a graph node: its interior cell, or the outdoor cell under it.
pub fn place_of(n: &NavNode) -> Place {
    let cell = if n.cell != 0 {
        n.cell
    } else {
        let block = block_at(n.pos);
        ac_world::outdoor_cell(block, n.pos - ac_world::landblock_origin(block))
    };
    Place { at: n.pos, cell }
}

/// The route from `from` to within `within` (metres, 3D) of `goal` on `area`, building the
/// chunks it needs. A walk between two outdoor places keeps out of buildings unless only they
/// lead there.
pub fn plan(area: &mut Area, from: Place, goal: Place, within: f32) -> Result<Route, NoWay> {
    let held = if area.dungeon {
        [from, goal]
            .iter()
            .all(|p| area.blocks.contains(&(p.cell & 0xFFFF_0000)))
    } else {
        area.holds(from.at, goal.at)
    };
    if !held {
        return Err(NoWay::OutOfRegion);
    }
    let (f, g) = area.with_ground(|ground, _| (graph_end(ground, from), graph_end(ground, goal)));
    let outdoors = !area.dungeon && f.1 == 0 && g.1 == 0;
    if area.outdoors_only != outdoors {
        // The graph built so far was for the other rule.
        area.outdoors_only = outdoors;
        area.nav = fresh(&area.nav);
    }
    let mut found = search_on(area, f, g, within);
    if matches!(found, Search::NoWay(NoWay::Unreached)) && area.outdoors_only {
        // No way outdoors: through the buildings, on a graph of its own, since the one built so
        // far left the interiors out.
        let inside = fresh(&area.nav);
        let outside = std::mem::replace(&mut area.nav, inside);
        area.outdoors_only = false;
        found = search_on(area, f, g, within);
        area.outdoors_only = true;
        area.nav = outside;
    }
    match found {
        Search::Found(path) => Ok(Route {
            waypoints: path
                .iter()
                .map(|(n, kind)| Waypoint {
                    place: place_of(n),
                    kind: *kind,
                })
                .collect(),
        }),
        Search::NoWay(why) => Err(why),
        // Searched building, so nothing is left unbuilt.
        Search::Unbuilt(_) => Err(NoWay::Unreached),
    }
}

/// An empty graph over the same lattice as `nav`.
fn fresh(nav: &NavGraph) -> NavGraph {
    NavGraph::new(nav.min(), nav.max(), nav.spacing, &nav.capsule)
}

fn search_on(area: &mut Area, from: (Vec3, u32), goal: (Vec3, u32), within: f32) -> Search {
    let avoid = area.avoid.clone();
    let q = WalkQuery {
        from,
        goal,
        within,
        avoid: &avoid,
        berth: area.berth,
        indoors_too: !area.outdoors_only,
        drops: false,
    };
    area.with_ground(|ground, nav| wayfind::search_building(ground, nav, &q))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_scene::collision::Capsule;
    use ac_scene::Assets;

    #[test]
    #[ignore = "needs AC_DATA_DIR"]
    fn a_corpse_on_the_floor_below_is_planned_to_on_that_floor() {
        // The route harness's "0286 corpse on the floor below": the corpse lies a storey under
        // the character's own floor, named only by its block, as a follow names it.
        let assets = Assets::open(ac_dat::test_data_dir()).unwrap();
        let block = 0x0286_0000;
        let mut area = Area::build(&assets, &[block], &Capsule::default(), block).unwrap();
        let from = Place {
            at: Vec3::new(419.98, 25_673.98, 0.0),
            cell: 0x0286_02BF,
        };
        let corpse = Vec3::new(386.725, 25_627.996, -6.0);
        let goal = Place {
            at: corpse,
            cell: block,
        };
        let cap = area.capsule();
        let (cell, floor) = area.with_ground(|ground, _| {
            let cell = wayfind::locate(ground, corpse);
            (cell, wayfind::floor_of(ground, corpse, cell, &cap))
        });
        assert!(
            cell & 0xFFFF >= 0x100,
            "the corpse is in no cell: {cell:#010x}"
        );
        let floor = floor.expect("the corpse's cell has a floor under it");
        let route = plan(&mut area, from, goal, 1.5).expect("a way down to the corpse");
        let end = route.waypoints.last().unwrap().place;
        assert_eq!(end.cell, cell, "ends in another cell, at {:?}", end.at);
        assert!(
            (end.at.z - floor.z).abs() < 0.6,
            "ends at {:?}, off the corpse's floor at {}",
            end.at,
            floor.z
        );
        assert!(
            end.at.distance(floor) <= 1.5,
            "ends {:?}, out of reach",
            end.at
        );
    }
}
