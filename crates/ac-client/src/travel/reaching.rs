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
        // And where an NPC way or an event portal stands, leads, or is used first: Browerk's
        // Portal Device stands indoors on Qin Xikit's Island, 660 m from the gate's exit.
        let npcs = ac_world::portals::npc_ways().iter().flat_map(|w| {
            let first = w.first.iter().map(|f| f.1.cell);
            [w.cell, w.to_cell].into_iter().chain(first)
        });
        let events = ac_world::portals::event_portals()
            .iter()
            .flat_map(|e| [e.way.from_cell, e.way.to_cell]);
        let indoor: std::collections::BTreeSet<u32> = ac_world::portals::all()
            .iter()
            .flat_map(|p| [p.from_cell, p.to_cell])
            .chain(npcs)
            .chain(events)
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
        let ways = self.standing_ways();
        let planned = trip::plan_with_ways(
            from, to, level, &quests, &refused, &no_walks, &recalls, &gems, prefs, &ways,
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
        let planned = trip::plan_with_ways(
            from, to, level, &quests, &refused, &no_walks, &recalls, &gems, far, &ways,
        );
        if let (true, Some(t)) = (refusals, &planned) {
            tracing::info!("travel: no shorter way; the long walk, {:.0} s", t.seconds);
        }
        planned
    }

    /// The portals standing in view that a creature's death or an NPC opened
    /// (`portals::summoned`), or a camp put out (`portals::spawned`): ways a trip takes while they
    /// stand, as the Relic Watchman's into Aerfalle's Keep, or a Small Hive Portal into its hive.
    /// And the NPCs that open a way, their item carried (`portals::npc_ways`).
    pub(crate) fn standing_ways(&self) -> trip::Ways {
        let summoned = ac_world::portals::summoned();
        let portals = self.world.objects.values().filter_map(|o| {
            let (cell, mouth) = (o.position?.cell, o.world_pos()?);
            let class = o.weenie_class_id;
            if let Some(e) = ac_world::portals::event_portals()
                .iter()
                .find(|e| e.wcid == class)
            {
                return Some(ac_world::portals::Portal {
                    from_cell: cell,
                    from: mouth,
                    ..e.way.clone()
                });
            }
            match summoned.iter().find(|s| s.wcid == class) {
                Some(s) => Some(s.standing_at(cell, mouth)),
                None => Some(ac_world::portals::spawned_of(class)?.standing_at(cell, mouth)),
            }
        });
        // One whose event this character started stands where its generator puts it, out of
        // view yet: the journey goes there (`forget_unseen_portals` lets a closed one go).
        let now = std::time::Instant::now();
        let opened: Vec<ac_world::portals::Portal> = ac_world::portals::event_portals()
            .iter()
            .filter(|e| self.is_event_on(&e.event, now) && !self.is_class_in_view(e.wcid))
            .map(|e| e.way.clone())
            .collect();
        let portals = portals.chain(opened);
        let level = self.world.stats.level.max(1) as u32;
        let quests = &self.travel.quests_done;
        let npcs = ac_world::portals::npc_ways()
            .iter()
            .filter(|w| w.usable_by(level, quests) && !self.is_npc_way_refused(w.npc))
            .filter(|w| w.item.is_none_or(|i| self.carried_count(i) > 0));
        trip::Ways {
            portals: portals.collect(),
            npcs: npcs.cloned().collect(),
        }
    }

    /// Whether an object of class `wcid` stands in view (a portal a camp or an event put out).
    pub(crate) fn is_class_in_view(&self, wcid: u32) -> bool {
        let objects = self.world.objects.values();
        objects
            .filter(|o| o.weenie_class_id == wcid)
            .any(|o| o.position.is_some())
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
        let ways = self.standing_ways();
        let mut standing: Vec<u32> = ways.portals.iter().map(|p| p.to_cell).collect();
        standing.extend(ways.npcs.iter().map(|w| w.npc));
        standing.sort_unstable();
        standing.hash(&mut h);
        h.finish()
    }

    /// Whether indoor `cell` is a dungeon's room, not a building's: in a dungeon's landblock, or
    /// one not walked into from its landblock's ground (Ketnan's, under Tusker Island; not the
    /// Gear Knights' excavation). Without the grid, as dungeon.
    pub(crate) fn is_dungeon_cell(&mut self, cell: u32) -> bool {
        if cell & 0xFFFF < 0x100 || self.travel.dungeon_cells.contains_key(&cell) {
            return self.is_dungeon_seen(cell);
        }
        let dungeon = self.travel_terrain().is_none() || self.is_dungeon_seen(cell);
        self.travel.dungeon_cells.insert(cell, dungeon);
        dungeon
    }

    /// [`Self::is_dungeon_cell`] from the grid as loaded, the answer not kept.
    pub(crate) fn is_dungeon_seen(&self, cell: u32) -> bool {
        if cell & 0xFFFF < 0x100 {
            return false;
        }
        if let Some(known) = self.travel.dungeon_cells.get(&cell) {
            return *known;
        }
        self.travel
            .grid
            .as_ref()
            .is_none_or(|grid| grid.is_dungeon_block(cell))
            || ac_scene::interior::can_walk_in(&self.assets, cell) == Some(false)
    }
}
