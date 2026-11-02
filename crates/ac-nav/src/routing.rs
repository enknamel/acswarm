//! A walk planned whole before a step is taken: from one [`Place`] to another on an area's
//! graph, every waypoint with its cell and the kind of step into it, or why there is no way.
//! Entry: [`plan`].

use ac_scene::nav::{EdgeKind, Ground, NavNode};
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

/// A plan on an area only read: a route, no way, or the chunks to build before it can answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Planned {
    Route(Route),
    NoWay(NoWay),
    Building(Vec<(i32, i32)>),
}

/// An area of `blocks` (a dungeon's, when `dungeon`) holds a walk from `from` to `goal`: a
/// dungeon by the places' cells, whose positions keep to no block square; outdoors by position.
pub fn is_held(blocks: &[u32], dungeon: bool, from: Place, goal: Place) -> bool {
    let within = |b: u32| blocks.contains(&b);
    if dungeon {
        within(from.cell & 0xFFFF_0000) && within(goal.cell & 0xFFFF_0000)
    } else {
        within(block_at(from.at)) && within(block_at(goal.at))
    }
}

/// The query for a walk on `area`, a berth kept from every portal not walked to; between two
/// outdoor places it first keeps out of buildings.
fn query_for<'a>(
    area: &Area,
    from: Place,
    goal: Place,
    within: f32,
    avoid: &'a [Vec3],
) -> Result<WalkQuery<'a>, NoWay> {
    if !is_held(&area.blocks, area.dungeon, from, goal) {
        return Err(NoWay::OutOfRegion);
    }
    let (f, g) = area.read_ground(|ground, _| (graph_end(ground, from), graph_end(ground, goal)));
    Ok(WalkQuery {
        from: f,
        goal: g,
        within,
        avoid,
        berth: PORTAL_BERTH,
        indoors_too: area.dungeon || f.1 != 0 || g.1 != 0,
        drops: false,
    })
}

/// The route a search found.
fn route_of(path: &[(NavNode, EdgeKind)]) -> Route {
    Route {
        waypoints: path
            .iter()
            .map(|(n, kind)| Waypoint {
                place: place_of(n),
                kind: *kind,
            })
            .collect(),
    }
}

/// The route from `from` to within `within` (metres, 3D) of `goal` on `area`, building the
/// chunks it needs, a berth kept from every portal not walked to. A walk between two outdoor
/// places keeps out of buildings unless only they lead there.
pub fn plan(area: &mut Area, from: Place, goal: Place, within: f32) -> Result<Route, NoWay> {
    let avoid = portal_mouths_to_avoid(from.at, goal.at);
    let mut q = query_for(area, from, goal, within, &avoid)?;
    let mut found = area.with_ground(|ground, nav| wayfind::search_building(ground, nav, &q));
    if matches!(found, Search::NoWay(NoWay::Unreached)) && !q.indoors_too {
        // No way outdoors: through the buildings, on the same graph.
        q.indoors_too = true;
        found = area.with_ground(|ground, nav| wayfind::search_building(ground, nav, &q));
    }
    match found {
        Search::Found(path) => Ok(route_of(&path)),
        Search::NoWay(why) => Err(why),
        // Searched building, so nothing is left unbuilt.
        Search::Unbuilt(_) => Err(NoWay::Unreached),
    }
}

/// [`plan`] on the chunks built so far, building none: `Building` names the chunks it needs
/// first, and is never a partial route or a guess.
pub fn plan_built(area: &Area, from: Place, goal: Place, within: f32) -> Planned {
    let avoid = portal_mouths_to_avoid(from.at, goal.at);
    let mut q = match query_for(area, from, goal, within, &avoid) {
        Ok(q) => q,
        Err(why) => return Planned::NoWay(why),
    };
    let mut found = area.read_ground(|ground, nav| wayfind::search(ground, nav, &q));
    if matches!(found, Search::NoWay(NoWay::Unreached)) && !q.indoors_too {
        q.indoors_too = true;
        found = area.read_ground(|ground, nav| wayfind::search(ground, nav, &q));
    }
    match found {
        Search::Found(path) => Planned::Route(route_of(&path)),
        Search::NoWay(why) => Planned::NoWay(why),
        Search::Unbuilt(chunks) => Planned::Building(chunks),
    }
}

/// Where a character can stand within `radius` (metres, flat) of `around`; once the area is built
/// whole, only where the body at `from` has a way to. `Err` names the chunks to build first.
pub fn standable_near(
    area: &Area,
    from: Place,
    around: Place,
    radius: f32,
) -> Result<Vec<Place>, Vec<(i32, i32)>> {
    area.read_ground(|ground, nav| {
        let start = graph_end(ground, from);
        let spots = wayfind::standable_near(ground, nav, start, around.at, radius)?;
        Ok(spots.iter().map(place_of).collect())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_scene::collision::Capsule;
    use ac_scene::Assets;

    #[test]
    #[ignore = "needs AC_DATA_DIR"]
    fn a_portal_mouth_on_a_neighbouring_cells_floor_is_planned_to() {
        // The Facility Hub's portal data names, for most mouths, a cell whose BSP holds the mouth
        // while the floor under it is a neighbouring cell's.
        let assets = Assets::open(ac_dat::test_data_dir()).unwrap();
        let block = 0x018A_0000;
        let mut area = Area::build(&assets, &[block], &Capsule::default(), block).unwrap();
        let mouths: Vec<_> = ac_world::portals::all()
            .iter()
            .filter(|p| p.from_cell & 0xFFFF_0000 == block)
            .collect();
        let start = mouths
            .iter()
            .find(|p| p.from_cell == 0x018A_0101)
            .expect("the Portal to Redspire");
        let from = Place {
            at: start.from,
            cell: start.from_cell,
        };
        let failed: Vec<(String, NoWay)> = mouths
            .iter()
            .filter(|p| p.from.distance(start.from) > PORTAL_BERTH)
            .filter_map(|p| {
                let goal = Place {
                    at: p.from,
                    cell: p.from_cell,
                };
                plan(&mut area, from, goal, PORTAL_BERTH)
                    .err()
                    .map(|why| (format!("{} {:#010x}", p.name, p.from_cell), why))
            })
            .collect();
        assert!(mouths.len() > 40, "{} mouths", mouths.len());
        assert!(
            failed.is_empty(),
            "{} of {}: {failed:?}",
            failed.len(),
            mouths.len()
        );
    }

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
            (
                cell,
                wayfind::floor_of(ground, corpse, cell, &cap).map(|f| f.0),
            )
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
