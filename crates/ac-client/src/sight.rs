//! What ACE counts as in the character's view, and what has been out of it long enough that the
//! server has let it go: ACE then sends no delete when it goes (ObjectMaint.cs:38-43), so it is set
//! aside (`World::put_out_of_view`) the way the retail client culls it, and brought back by the
//! server's next word of it, or by an appraisal answer once back in view (`tick_sight`).

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use crate::Client;

/// Out of view this long before set aside: ACE's 25 s (ObjectMaint.cs:21), up to 5 s more to its
/// heartbeat's sweep (Player_Tick.cs:126), and 5 s spare, so the server has let go first.
pub(crate) const OUT_OF_VIEW_FOR: Duration = Duration::from_secs(35);

/// Back in view this long, undescribed, before the server is asked whether it still has it.
pub(crate) const BACK_IN_VIEW_ASK: Duration = Duration::from_secs(3);

/// How long an ask about something back in view goes unanswered before it is asked again.
const ASK_AGAIN: Duration = Duration::from_secs(10);

/// How often the view is judged.
const SIGHT_EVERY: Duration = Duration::from_secs(1);

/// What the server shows a player standing in one cell (ObjectMaint.cs:337-388).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum View {
    /// Outdoors, or a building room seen from outside: this landblock and its eight neighbours.
    /// ACE's own view is no larger (a dungeon landblock has none, LandblockManager.cs:581).
    Around { block: u16, rooms: HashSet<u16> },
    /// A dungeon room: itself and the rooms the data lists as visible from it.
    Rooms { block: u16, rooms: HashSet<u16> },
    /// A cell the data does not have: everything counts as in view.
    Everything,
}

impl View {
    /// The view from cell `cell`, with `room` reading an indoor cell's visible rooms and whether it
    /// is seen from outside (`ac_scene::interior::cell_view`).
    pub(crate) fn from_cell(cell: u32, room: impl Fn(u32) -> Option<(Vec<u16>, bool)>) -> View {
        let block = (cell >> 16) as u16;
        let low = (cell & 0xFFFF) as u16;
        if low < 0x100 {
            return View::Around {
                block,
                rooms: HashSet::new(),
            };
        }
        let Some((visible, seen_outside)) = room(cell) else {
            return View::Everything;
        };
        let mut rooms: HashSet<u16> = visible.into_iter().collect();
        rooms.insert(low);
        if seen_outside {
            View::Around { block, rooms }
        } else {
            View::Rooms { block, rooms }
        }
    }

    /// Whether something in cell `cell` is in this view. Outdoors every cell of the nine landblocks
    /// counts, a dungeon room under them too: kept longer than ACE keeps it, never let go sooner.
    pub(crate) fn sees(&self, cell: u32) -> bool {
        let (b, low) = ((cell >> 16) as u16, (cell & 0xFFFF) as u16);
        match self {
            View::Everything => true,
            View::Rooms { block, rooms } => b == *block && rooms.contains(&low),
            View::Around { block, rooms } => {
                let near = |a: u16, c: u16| (i32::from(a) - i32::from(c)).abs() <= 1;
                let (x, y) = (b >> 8, b & 0xFF);
                let (bx, by) = (*block >> 8, *block & 0xFF);
                (near(x, bx) && near(y, by)) || (b == *block && rooms.contains(&low))
            }
        }
    }
}

/// The view and its clocks.
#[derive(Debug, Default)]
pub(crate) struct Sight {
    /// The cell the view was worked out from, and the view.
    view: Option<(u32, View)>,
    /// When each object in the world was last found out of view, while it stays so.
    unseen_since: HashMap<u32, Instant>,
    /// When each object set aside came back into view, while it stays so.
    back_since: HashMap<u32, Instant>,
    /// Set-aside objects asked about since back in view, and when.
    asked: HashMap<u32, Instant>,
    /// When the view was last judged.
    judged: Option<Instant>,
}

impl Client {
    /// Set aside what has been out of view [`OUT_OF_VIEW_FOR`], and ask about what was set aside
    /// once it has been back in view [`BACK_IN_VIEW_ASK`] with no word from the server.
    pub(crate) fn tick_sight(&mut self, now: Instant) {
        if self
            .sight
            .judged
            .is_some_and(|t| now.duration_since(t) < SIGHT_EVERY)
        {
            return;
        }
        self.sight.judged = Some(now);
        // The body's cell: what the character reports to the server, and so the cell ACE sees from.
        let Some(here) = self.player.as_ref().map(|p| p.cell) else {
            return;
        };
        if self.sight.view.as_ref().is_none_or(|(c, _)| *c != here) {
            let assets = self.assets.clone();
            let view = View::from_cell(here, |c| ac_scene::interior::cell_view(&assets, c));
            self.sight.view = Some((here, view));
        }
        let Some((_, view)) = self.sight.view.clone() else {
            return;
        };
        let mine = self.world.player_guid;
        let placed: Vec<(u32, bool)> = self
            .world
            .objects
            .values()
            .filter(|o| Some(o.guid) != mine && o.parent.is_none() && o.container.is_none())
            .filter_map(|o| o.position.map(|p| (o.guid, view.sees(p.cell))))
            .collect();
        let sight = &mut self.sight;
        sight
            .unseen_since
            .retain(|g, _| placed.iter().any(|(p, _)| p == g));
        let mut aside = Vec::new();
        for (guid, seen) in placed {
            if seen {
                sight.unseen_since.remove(&guid);
                continue;
            }
            let since = *sight.unseen_since.entry(guid).or_insert(now);
            if now.duration_since(since) >= OUT_OF_VIEW_FOR {
                aside.push(guid);
            }
        }
        for guid in &aside {
            self.sight.unseen_since.remove(guid);
            self.world.put_out_of_view(*guid);
        }
        if !aside.is_empty() {
            tracing::debug!("sight: set aside {} object(s) out of view", aside.len());
        }
        let back: Vec<u32> = self
            .world
            .out_of_view()
            .filter(|o| o.position.is_some_and(|p| view.sees(p.cell)))
            .map(|o| o.guid)
            .collect();
        let sight = &mut self.sight;
        sight.back_since.retain(|g, _| back.contains(g));
        sight.asked.retain(|g, _| back.contains(g));
        let mut ask = Vec::new();
        for guid in back {
            let since = *sight.back_since.entry(guid).or_insert(now);
            let due = sight
                .asked
                .get(&guid)
                .is_none_or(|at| now.duration_since(*at) >= ASK_AGAIN);
            if now.duration_since(since) >= BACK_IN_VIEW_ASK && due {
                sight.asked.insert(guid, now);
                ask.push(guid);
            }
        }
        for guid in ask {
            tracing::debug!("sight: {guid:#010x} back in view with no word of it; asking");
            self.appraise(guid);
            self.appraise_inflight.push((guid, now));
        }
    }
}

#[cfg(test)]
mod tests;
