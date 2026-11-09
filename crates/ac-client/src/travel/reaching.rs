//! Whether and how a place can be got to, asked without setting off: the town run's counters.

use ac_world::trip::{self, Trip};
use ac_world::Place;

use crate::Client;

impl Client {
    /// Tell the planner, once a process, which indoor portal cells are buildings: read off the
    /// cells, which it has no copy of. Not until the world grid is loaded, which tells dungeons.
    /// The landmasses too, the same once: no walk is planned across the sea.
    pub(crate) fn tell_building_cells(&mut self) {
        if !trip::landmasses_known() {
            if let Some((grid, region)) = self.travel_terrain() {
                let land = ac_scene::landmass::Landmasses::of_world(&grid, &region);
                trip::set_landmasses(move |at| land.at(at));
            }
        }
        if trip::building_cells_known() || self.travel_terrain().is_none() {
            return;
        }
        let indoor: std::collections::BTreeSet<u32> = ac_world::portals::all()
            .iter()
            .flat_map(|p| [p.from_cell, p.to_cell])
            .filter(|c| c & 0xFFFF >= 0x100)
            .collect();
        let buildings = indoor
            .into_iter()
            .filter(|c| !self.is_dungeon_cell(*c))
            .collect();
        trip::set_building_cells(buildings);
    }

    /// The trip to `place` from where the body stands, the long walk the last resort; `None` when
    /// nothing reaches it. With `refusals`, what this journey was turned away from is left out.
    pub(crate) fn plan_trip(&mut self, place: Place, refusals: bool) -> Option<Trip> {
        self.tell_building_cells();
        let (from, underground) = self.planning_from()?;
        let level = self.world.stats.level.max(1) as u32;
        let (refused, no_walks) = if refusals {
            (
                self.travel.refused.clone(),
                self.travel.refused_walks.clone(),
            )
        } else {
            Default::default()
        };
        let prefs = self.travel_prefs().in_dungeon(underground);
        // Portal gems in the pack are ways to get somewhere too, and
        // unlike a recall they need no skill or components: carrying one
        // is the whole requirement.
        let gems: Vec<trip::Gem> = self
            .carried_gems()
            .into_iter()
            .filter(|g| !refusals || !self.travel.refused_gems.contains(&g.guid))
            .collect();
        let recalls: Vec<trip::Recall> = if prefs.use_recalls {
            let out = if refusals {
                self.travel.refused_recalls.clone()
            } else {
                Vec::new()
            };
            self.castable_recalls()
                .into_iter()
                .filter(|r| !out.contains(&r.spell))
                .collect()
        } else {
            Vec::new()
        };
        let quests = self.travel.quests_done.clone();
        let to = self.planning_place(place);
        let planned = trip::plan_with_recalls_and_gems(
            from, to, level, &quests, &refused, &no_walks, &recalls, &gems, prefs,
        );
        if planned.is_some() {
            return planned;
        }
        // Nothing within a believable walk or a portal: walk the whole way, as the last resort.
        // A ground reached by a one-way portal has no way back otherwise, and the character was
        // left there (a_ground_reached_through_a_one_way_portal_has_a_way_home_on_foot).
        let far = trip::Prefs {
            walk_reach: trip::Prefs::far().walk_reach,
            portal_reach: trip::Prefs::far().portal_reach,
            ..prefs
        };
        let planned = trip::plan_with_recalls_and_gems(
            from, to, level, &quests, &refused, &no_walks, &recalls, &gems, far,
        );
        if let (true, Some(t)) = (refusals, &planned) {
            tracing::info!("travel: no shorter way; the long walk, {:.0} s", t.seconds);
        }
        planned
    }

    /// What the trip planner may use, as one number: level, quest flags, the gems carried and
    /// the recalls castable. A journey's refusal stands only while it is the same.
    pub(crate) fn journey_means(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.world.stats.level.hash(&mut h);
        self.travel.quests_done.hash(&mut h);
        let mut gems: Vec<u32> = self.carried_gems().iter().map(|g| g.guid).collect();
        gems.sort_unstable();
        gems.hash(&mut h);
        if self.travel_prefs().use_recalls {
            let mut recalls: Vec<u32> = self.castable_recalls().iter().map(|r| r.spell).collect();
            recalls.sort_unstable();
            recalls.hash(&mut h);
        }
        h.finish()
    }

    /// Whether indoor `cell` is a dungeon's room, not a building's: in a dungeon's landblock, or
    /// one the outdoors cannot see (Ketnan's, under Tusker Island). Without the grid, as dungeon.
    pub(crate) fn is_dungeon_cell(&mut self, cell: u32) -> bool {
        if cell & 0xFFFF < 0x100 {
            return false;
        }
        if let Some(known) = self.travel.dungeon_cells.get(&cell) {
            return *known;
        }
        let dungeon = self
            .travel_terrain()
            .is_none_or(|(grid, _)| grid.is_dungeon_block(cell))
            || ac_scene::interior::seen_outside(&self.assets, cell) == Some(false);
        self.travel.dungeon_cells.insert(cell, dungeon);
        dungeon
    }
}
