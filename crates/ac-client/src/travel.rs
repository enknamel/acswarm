//! Travel: "take me to Arwic". The journey is planned first as a trip
//! (`ac_world::trip`), which decides which portals to take: from
//! Holtburg to Arwic that is the Town Network, in through the town's
//! portal and out of the hub's. Each step of the trip is then walked
//! with a route on the world terrain grid (`ac_scene::worldroute`:
//! slopes, water and roads; no buildings), followed waypoint by
//! waypoint through the per-landblock move-to (`route::Steering`), which
//! steers around the buildings and fences on the way. Each leg handed to
//! the local planner is short and stays inside the current landblock, so
//! the landblock navigation graph can plan it. A portal step is walked
//! the same way, into the portal's mouth, and is done when the character
//! comes out somewhere else.
//!
//! The grid takes a few seconds to load the first time (and a few hundred
//! milliseconds from its cache after that); it is loaded on the first
//! travel request and kept.

use std::rc::Rc;
use std::time::{Duration, Instant};

use ac_formats::region::Region;
use ac_scene::worldgrid::WorldGrid;
use ac_scene::worldroute;
use ac_world::trip::{self, Prefs, Step, Trip};
use glam::{Vec2, Vec3};

use crate::Client;

/// A waypoint counts as reached within this distance (metres, flat).
pub const ARRIVE: f32 = 3.0;
/// Longest leg handed to the local move-to (metres).
pub const LEG: f32 = 60.0;

/// How far a goal may be and still be walked to directly. One landblock
/// across: beyond that the way there is a journey, not a stroll, and
/// the steering has no business trying.
pub const WALKABLE: f32 = 192.0;

/// The leg is cut this far short of the landblock edge so its end lies in
/// the block the character stands in.
const EDGE_MARGIN: f32 = 2.0;
/// A portal that has not taken us in this long is one we cannot use
/// (a level range, an unfinished quest, or it simply is not there): the
/// journey is planned again without it.
pub const PORTAL_GIVE_UP: Duration = Duration::from_secs(12);
/// A recall step waits this long after the step begins before casting,
/// so the character has come to a stop (moving disrupts the cast).
const RECALL_SETTLE: Duration = Duration::from_millis(800);
/// A recall that has not carried us off this long after the cast was
/// sent (the cast takes a few seconds and the server pauses two more
/// before the teleport) is cast again, and after [`RECALL_TRIES`] casts
/// the journey is planned again without that spell.
pub const RECALL_GIVE_UP: Duration = Duration::from_secs(15);
const RECALL_TRIES: u32 = 2;
/// A gem still in the pack this long after it was used was not taken:
/// the server refuses a use while the character is busy -- mid-cast,
/// say -- and leaves the gem where it was. It is used again.
const GEM_RETRY: Duration = Duration::from_secs(3);
/// How many times a gem is used before the journey goes another way.
const GEM_TRIES: u32 = 4;
/// A gem that was taken but has put no portal in front of the character,
/// or carried it nowhere, in this long is given up on. The summon is a
/// cast of a couple of seconds; a portal that has not appeared well after
/// that is one the server could not find room for.
const GEM_GIVE_UP: Duration = Duration::from_secs(8);
/// A summoned portal is used again this long after a use that did not
/// carry the character off.
const SUMMONED_USE_EVERY: Duration = Duration::from_secs(3);
/// How many times a summoned portal is used before it is given up on.
const SUMMONED_TRIES: u32 = 4;
/// How far off the portal a gem summoned may stand. ACE puts it three
/// metres in front of whoever used the gem.
const SUMMONED_REACH: f32 = 8.0;
/// Where the summoned portal stands: this far in front of the character.
const SUMMON_DISTANCE: f32 = 3.0;
/// How far ahead must be free of walls for the portal to fit: out to
/// where it stands and past it by its own width.
const SUMMON_CLEARANCE: f32 = 4.5;
/// The ground where the portal stands may be this much above or below
/// the character's feet.
const SUMMON_STEP: f32 = 1.5;
/// Nothing that stands in the world -- a creature, another portal, a
/// lifestone -- may be this close to where the portal would go.
const SUMMON_ROOM: f32 = 2.0;
/// The step taken to face open ground before using a gem.
const FACE_STEP: f32 = 1.5;
/// A step to face open ground that has not been finished in this long is
/// finished where the character stands.
const FACE_GIVE_UP: Duration = Duration::from_secs(4);
/// A step that has come no closer to its target in this long is planned
/// again from where the character stands.
pub const STEP_GIVE_UP: Duration = Duration::from_secs(30);
/// Coming this much closer to the step's target counts as progress.
const STEP_PROGRESS: f32 = 5.0;
/// No progress toward the current waypoint for this long: skip it.
/// Progress is measured along the route the steering is following
/// where there is one, since a detour around a wall walks away from the
/// waypoint for a good while before it comes back.
pub const STUCK_AFTER: Duration = Duration::from_secs(10);
/// How many times a step whose end cannot be reached is planned again
/// before the journey is given up.
const REPLANS: u32 = 3;
/// Getting closer to the waypoint by less than this is not progress.
const PROGRESS: f32 = 1.0;
/// The character stops this close to the end of a leg (the local move-to's
/// stop distance); legs are replaced before that, at [`ARRIVE`].
const LEG_STOP: f32 = 1.0;

/// The journey being made, and the terrain data it needs.
#[derive(Default)]
pub struct Travel {
    grid: Option<Rc<WorldGrid>>,
    region: Option<Rc<Region>>,
    /// The whole journey, portals included, and the step being made.
    trip: Option<Trip>,
    step: usize,
    /// The landblock the character was in when a portal step began, and
    /// when it began; the step is done when they are somewhere else, and
    /// given up on when the portal will not take them.
    portal_from: Option<u32>,
    portal_since: Option<Instant>,
    /// The landblock the current step is aimed into, and when the step
    /// began: a step that gets nowhere is planned again from where the
    /// character actually stands.
    step_block: Option<u32>,
    /// The cell the step is aimed at, so an indoor target is recognised
    /// even when the character stands outdoors in the same landblock.
    step_cell: Option<u32>,
    step_since: Option<Instant>,
    /// Closest the character has come to the step's target: the step is
    /// only "going nowhere" when this stops improving.
    step_best: f32,
    /// Where the step is aimed, to measure that.
    step_target: Option<Vec2>,
    /// Where the character stood last frame, to notice a portal that
    /// took them somewhere the journey did not ask for.
    last_seen: Option<Vec2>,
    /// When we last tried jumping into a portal that has not taken us.
    last_hop: Option<Instant>,
    /// How journeys are chosen: the quickest chain, or fewer hops.
    pub prefs: Prefs,
    /// The mouths of portals that would not take us *on this journey*:
    /// they are left out while the rest of the way is planned again, and
    /// forgotten as soon as the player asks to go somewhere else. Named
    /// by place, not by name, since every dungeon has a "Surface Portal".
    refused: Vec<Vec2>,
    /// Recall spells that did not carry us off *on this journey* (the
    /// server refused them, or they fizzled twice): left out of the
    /// next plan, forgotten with a new destination.
    refused_recalls: Vec<u32>,
    /// When the recall step began or its spell was last sent, and how
    /// many times it has been sent.
    recall_since: Option<Instant>,
    recall_casts: u32,
    /// The character was in peace mode before the cast: drop back to it
    /// once the recall has landed.
    recall_leave_combat: bool,
    /// When the gem step's gem was last used, and how many times.
    gem_used: Option<Instant>,
    gem_uses: u32,
    /// The spot walked to so that the gem is used facing open ground, and
    /// when that walk began.
    gem_spot: Option<(Vec2, Instant)>,
    /// When the portal that gem summoned was last used, and how many
    /// times.
    summoned_used: Option<Instant>,
    summoned_uses: u32,
    /// Gems that did not take us anywhere *on this journey*: left out of
    /// the next plan, like `refused_recalls`.
    refused_gems: Vec<u32>,
    /// The last tie spell cast (Lifestone Tie, a Portal Tie), so the
    /// "successfully linked" that follows is filed under its position.
    pub(crate) last_tie: Option<u32>,
    /// Where the journey is bound, to replan around a refusal.
    goal: Option<Vec2>,
    /// World xy waypoints of the step being walked, start and goal
    /// included.
    route: Option<Vec<Vec2>>,
    /// Index of the waypoint being walked to.
    next: usize,
    /// End of the leg the local move-to is aimed at right now.
    leg: Option<Vec2>,
    /// Closest the character has been to the current waypoint, and when
    /// that last improved.
    best: f32,
    last_progress: Option<Instant>,
    /// When the route that distance was last measured along was planned:
    /// a new route is a new yardstick.
    measured_on: Option<Instant>,
    /// Times the end of a step could not be reached and the journey was
    /// planned again from where the character stood.
    replans: u32,
}

impl Travel {
    fn restart_waypoint(&mut self) {
        self.leg = None;
        self.best = f32::INFINITY;
        self.last_progress = None;
        self.measured_on = None;
    }
}

impl Client {
    /// The world grid and region, loaded on first use.
    fn travel_terrain(&mut self) -> Option<(Rc<WorldGrid>, Rc<Region>)> {
        if self.travel.grid.is_none() {
            let t0 = Instant::now();
            // One copy per process: every session's journey reads the
            // same grid.
            match self.assets.world_grid() {
                Ok(g) => {
                    tracing::info!("travel: world grid loaded in {:?}", t0.elapsed());
                    self.travel.grid = Some(g);
                }
                Err(e) => {
                    tracing::warn!("travel: could not load the world grid: {e}");
                    return None;
                }
            }
        }
        if self.travel.region.is_none() {
            match self.assets.region() {
                Ok(r) => self.travel.region = Some(r),
                Err(e) => {
                    tracing::warn!("travel: could not load the region: {e}");
                    return None;
                }
            }
        }
        Some((self.travel.grid.clone()?, self.travel.region.clone()?))
    }

    /// Go there. The one way anything asks for movement.
    ///
    /// Everything that wants a character to be somewhere else says so
    /// here, and this decides *how*: a walk the steering can find its
    /// own way through, or a journey of portals, recalls and gems. The
    /// caller names a place and is told what came of it; it does not
    /// choose the means, and it does not touch `follow`.
    ///
    /// There used to be three ways to ask -- a server-driven walk, a
    /// journey, and a raw point handed straight to the steering -- and
    /// nine places that took the third. Each behaved differently when
    /// the way was blocked, so a fault fixed in one was still there in
    /// the others: a character was taught not to walk into a wall on
    /// its way to a corpse and went on doing it on its way to a shop.
    ///
    /// Within a landblock this is a walk: the steering knows how to get
    /// round what is in the way, and a goal it cannot reach it now
    /// refuses rather than leans on. Beyond one -- or out of a dungeon,
    /// where the feet lead nowhere at all -- it is a journey, planned
    /// with whatever the character can cast or carry.
    ///
    /// Asking for movement this way ends any visit being made
    /// (`visit`): whoever asks has taken the character somewhere else.
    pub fn head_for(&mut self, goal: glam::Vec3, stop: f32, why: &str) -> crate::did::Did {
        self.drop_visit(why);
        self.head_toward(goal, stop, why)
    }

    /// [`head_for`](Self::head_for) for a visit's own last stretch: the
    /// same walk or journey, without ending the visit that asked for it.
    pub(crate) fn head_toward(
        &mut self,
        goal: glam::Vec3,
        stop: f32,
        why: &str,
    ) -> crate::did::Did {
        use crate::did::Did;
        let Some(pl) = self.player.as_ref() else {
            return Did::waiting("not in the world yet");
        };
        let me = pl.world_position();
        let away = Vec2::new(goal.x - me.x, goal.y - me.y).length();
        // Near enough to walk to.
        //
        // Being underground does not make the rest of the dungeon
        // unwalkable -- a monster across the room is a walk like any
        // other, and ruling that out left a character unable to reach
        // anything it was fighting. It is only the way *out* that the
        // feet cannot manage, and that needs no special case here: a
        // dungeon keeps its own corner of the world, tens of thousands
        // of metres from the town above it, so anywhere outside is far
        // past `WALKABLE` and goes to the planner anyway.
        if away <= WALKABLE {
            if self.traveling() {
                self.end_trip();
            }
            self.follow = Some(crate::Follow { target: goal, stop });
            return Did::Acting;
        }
        // Already on the way there.
        if self.traveling() {
            return Did::Acting;
        }
        if self.plan_trip(Vec2::new(goal.x, goal.y)) {
            return Did::Acting;
        }
        tracing::info!("travel: no way to {why} from here");
        Did::blocked("no way there from here")
    }

    /// Plan a journey from where the character stands to `goal` (world
    /// xy) and start it. Portals are used when they are quicker than
    /// walking, and so are the recall spells the character can cast
    /// right now (`castable_recalls`) unless the journey's `Prefs` say
    /// `use_recalls: false`. False when nothing reaches the goal.
    ///
    /// A journey asked for from outside ends any visit being made.
    pub fn travel_to(&mut self, goal: Vec2) -> bool {
        self.drop_visit("travelling somewhere else");
        self.plan_trip(goal)
    }

    /// [`travel_to`](Self::travel_to) from inside: planning the way again
    /// after a refusal, or a visit setting off, neither of which is a new
    /// destination as far as a visit is concerned.
    pub(crate) fn plan_trip(&mut self, goal: Vec2) -> bool {
        let Some(pl) = self.player.as_ref() else {
            tracing::warn!("travel: the character is not in the world");
            return false;
        };
        let me = pl.world_position();
        let cell = pl.cell;
        let t0 = Instant::now();
        // A new destination starts afresh: what turned us away on the
        // last journey says nothing about this one.
        if self.travel.goal.is_none_or(|g| g.distance(goal) > 1.0) {
            self.travel.refused.clear();
            self.travel.refused_recalls.clear();
            self.travel.refused_gems.clear();
            self.travel.replans = 0;
        }
        let level = self.world.stats.level.max(1) as u32;
        let mut refused = self.travel.refused.clone();
        // Underground, there is no walking out. A dungeon shares its
        // landblock number with the ground above it, so without saying
        // so the planner reads a vendor a hundred metres up as a
        // hundred-metre stroll -- which is a character setting off into
        // the nearest wall, saying it is going to the shops.
        let underground = {
            let assets = self.assets.clone();
            self.player
                .as_mut()
                .map(|pl| pl.is_indoors() && pl.in_dungeon(&assets))
                .unwrap_or(false)
        };
        let prefs = self.travel.prefs.in_dungeon(underground);
        tracing::info!("travel: planning to {goal:?} from {cell:#010x}, underground {underground}");
        // Portal gems in the pack are ways to get somewhere too, and
        // unlike a recall they need no skill or components: carrying one
        // is the whole requirement.
        let gems: Vec<trip::Gem> = self
            .carried_gems()
            .into_iter()
            .filter(|g| !self.travel.refused_gems.contains(&g.guid))
            .collect();
        let recalls: Vec<trip::Recall> = if prefs.use_recalls {
            let out = self.travel.refused_recalls.clone();
            self.castable_recalls()
                .into_iter()
                .filter(|r| !out.contains(&r.spell))
                .collect()
        } else {
            Vec::new()
        };
        // The planner prices a leg on foot by the straight line between
        // its ends, so it can pick a portal that stands across water or
        // up a cliff. Walk the plan through the terrain router before
        // setting off and, when a portal cannot be reached from where
        // the step before it leaves the character, plan again without
        // that one. This holds for the journey being planned only.
        let mut planned = None;
        for _ in 0..3 {
            let Some(t) = trip::plan_with_recalls_and_gems(
                Vec2::new(me.x, me.y),
                cell,
                goal,
                level,
                &[],
                &refused,
                &recalls,
                &gems,
                prefs,
            ) else {
                break;
            };
            // Only the first portal is checked here. Checking every one
            // meant a route search for each, and a search over the world
            // is slow enough that the client stopped answering while it
            // thought. The rest are caught as they come: a step that
            // cannot be walked plans the way again from where we stand.
            let here = Vec2::new(me.x, me.y);
            let outdoors = cell & 0xFFFF < 0x100;
            let bad = match t.steps.first() {
                Some(trip::Step::Portal {
                    mouth, mouth_cell, ..
                }) if outdoors
                    && mouth_cell & 0xFFFF < 0x100
                    && !self.travel_can_reach(here, *mouth) =>
                {
                    Some(*mouth)
                }
                _ => None,
            };
            match bad {
                Some(mouth) => {
                    tracing::info!(
                        "travel: no way on foot to the portal at {mouth:?}; another way"
                    );
                    refused.push(mouth);
                }
                None => {
                    planned = Some(t);
                    break;
                }
            }
        }
        let Some(trip) = planned else {
            // Say why. Often there is a way, but not one this character
            // may take yet: every chain to the far continents runs
            // through a portal with a level on it.
            let anyone = trip::plan_with(
                Vec2::new(me.x, me.y),
                cell,
                goal,
                0,
                &[],
                &[],
                trip::Prefs::far(),
            );
            let why = match anyone {
                Some(t) => {
                    let asks = t
                        .steps
                        .iter()
                        .filter_map(|s| match s {
                            Step::Portal { mouth, name, .. } => {
                                ac_world::portals::near(*mouth, 3.0)
                                    .first()
                                    .and_then(|p| p.refusal(level))
                                    .map(|r| format!("{name} {r}"))
                            }
                            // A gem needs nothing of the character but
                            // carrying it, so it can never be refused.
                            Step::Walk(_) | Step::Recall { .. } | Step::Gem { .. } => None,
                        })
                        .next();
                    match asks {
                        Some(asks) => format!(
                            "No way there yet: the only route runs through a portal that {asks}."
                        ),
                        None => "No way there from here.".to_string(),
                    }
                }
                None => "No way there from here.".to_string(),
            };
            tracing::warn!("travel: {why} ({goal:?}, level {level})");
            self.events.push(crate::Event::Chat { text: why, kind: 1 });
            return false;
        };
        self.travel.refused = refused;
        tracing::info!(
            "travel: {} ({} steps, planned in {:?})",
            trip.summary(),
            trip.steps.len(),
            t0.elapsed()
        );
        self.travel.trip = Some(trip);
        self.travel.step = 0;
        self.travel.route = None;
        self.travel.portal_from = None;
        self.travel.portal_since = None;
        self.travel.goal = Some(goal);
        self.travel.restart_waypoint();
        self.travel_start_step()
    }

    /// Whether the terrain router can get from `me` to `target`, or to
    /// somewhere just beside it.
    /// One route query, and only one: this runs on the frame the player
    /// asked to travel, and a search over the world takes long enough
    /// that a handful of them is a visible freeze.
    fn travel_can_reach(&mut self, me: Vec2, target: Vec2) -> bool {
        let Some((grid, region)) = self.travel_terrain() else {
            return true;
        };
        worldroute::find(&grid, &region, me, target).is_some()
    }

    /// Build the route for the step the trip is on. False when the step
    /// cannot be routed (and the journey is given up).
    fn travel_start_step(&mut self) -> bool {
        let Some(t) = self.travel.trip.as_ref() else {
            return false;
        };
        let Some(step) = t.steps.get(self.travel.step).cloned() else {
            tracing::info!("travel: arrived");
            // The journey's own end, not the player's: a visit it was
            // made for carries on from here.
            self.end_trip();
            return false;
        };
        let Some(pl) = self.player.as_ref() else {
            return false;
        };
        let me3 = pl.world_position();
        let me = Vec2::new(me3.x, me3.y);
        let indoors = pl.is_indoors();
        let pl_cell = pl.cell;
        // A recall step has nowhere to walk: stand still, cast, and wait
        // to be carried off, which `travel_goal` sees to. Like a portal
        // step it is done when the character is where the spell lands.
        if let Step::Recall { name, spell, .. } = &step {
            tracing::info!(
                "travel: step {} (recall {name:?}, spell {spell})",
                self.travel.step
            );
            self.travel.portal_from = Some(pl_cell & 0xFFFF_0000);
            self.travel.portal_since = None;
            self.travel.last_hop = None;
            self.travel.step_since = None;
            self.travel.step_best = f32::INFINITY;
            self.travel.step_target = None;
            self.travel.step_cell = None;
            self.travel.step_block = None;
            self.travel.route = None;
            self.travel.recall_since = Some(Instant::now());
            self.travel.recall_casts = 0;
            self.travel.recall_leave_combat = !self.combat && !self.magic;
            self.travel.restart_waypoint();
            return true;
        }
        // A gem is used where it stands. The usual kind puts a portal
        // in front of the character, which the next step walks into;
        // the rare kind carries them off itself. Either way the step is
        // done once they are somewhere else, the same as a recall.
        if let Step::Gem { name, summons, .. } = &step {
            let (name, summons) = (name.clone(), *summons);
            tracing::info!(
                "travel: step {} ({name:?}, {})",
                self.travel.step,
                if summons {
                    "summons a portal"
                } else {
                    "carries us off"
                }
            );
            self.travel.portal_from = Some(pl_cell & 0xFFFF_0000);
            self.travel.portal_since = None;
            self.travel.last_hop = None;
            self.travel.step_since = Some(Instant::now());
            self.travel.step_best = f32::INFINITY;
            self.travel.step_target = None;
            self.travel.step_cell = None;
            self.travel.step_block = None;
            self.travel.route = None;
            self.travel.restart_waypoint();
            // Not used yet: a summoned portal needs room to stand in,
            // and a gem used where there is none is spent for nothing.
            // The step finds the room first (see `travel_gem_wait`).
            self.travel.gem_used = None;
            self.travel.gem_uses = 0;
            self.travel.gem_spot = None;
            self.travel.summoned_used = None;
            self.travel.summoned_uses = 0;
            return true;
        }
        let (target, label, mouth_cell) = match &step {
            Step::Walk(p) => (*p, "walk".to_string(), None),
            Step::Portal {
                name,
                mouth,
                mouth_cell,
                ..
            } => (*mouth, format!("portal {name:?}"), Some(*mouth_cell)),
            // A recall is cast, and a gem is used, by the step driver
            // rather than walked to: nothing to aim at.
            Step::Recall { .. } | Step::Gem { .. } => return true,
        };
        self.travel.portal_from = mouth_cell.map(|_| pl_cell & 0xFFFF_0000);
        self.travel.portal_since = None;
        self.travel.last_hop = None;
        self.travel.step_since = Some(Instant::now());
        self.travel.step_best = f32::INFINITY;
        self.travel.step_target = Some(target);
        self.travel.step_cell = mouth_cell;
        self.travel.step_block = Some(match mouth_cell {
            Some(c) => c & 0xFFFF_0000,
            None => WorldGrid::block_of(target),
        });
        // Inside a landblock -- the character in it, or the target in one
        // of its interior cells -- the terrain grid says nothing. Aim
        // straight at the target and let the landblock's own navigation
        // graph steer. A dungeon's exit portal stands in an interior cell
        // of the block the character walks out into, so the target being
        // indoors matters as much as the character being indoors.
        let target_indoors = mouth_cell.is_some_and(|c| c & 0xFFFF >= 0x100);
        let same_block = self.travel.step_block == Some(pl_cell & 0xFFFF_0000);
        if indoors || (target_indoors && same_block) {
            tracing::info!("travel: step {} ({label}) inside", self.travel.step);
            self.travel.route = Some(vec![me, target]);
            self.travel.next = 1;
            self.travel.restart_waypoint();
            return true;
        }
        let Some((grid, region)) = self.travel_terrain() else {
            return false;
        };
        // The terrain router can fail on the last few metres to a portal
        // that stands against a cliff or inside a doorway. Walking to
        // just beside it is as good: the local move-to covers the rest.
        // Every other portal near the way is a trap: walking into one
        // takes the character wherever it leads. Give them a wide berth.
        let keep = mouth_cell.map(|_| target);
        let mid = (me + target) * 0.5;
        let reach = me.distance(target) * 0.5 + 200.0;
        let avoid: Vec<Vec2> = ac_world::portals::near(mid, reach)
            .into_iter()
            .filter(|p| p.mouth_outdoors())
            .map(|p| p.from_xy())
            .filter(|m| keep.is_none_or(|k| k.distance(*m) > 5.0))
            .collect();
        let route = worldroute::find_avoiding(&grid, &region, me, target, &avoid).or_else(|| {
            [8.0f32, 16.0, 28.0].into_iter().find_map(|r| {
                (0..8).find_map(|i| {
                    let a = i as f32 * std::f32::consts::TAU / 8.0;
                    let near = target + Vec2::new(a.cos(), a.sin()) * r;
                    worldroute::find_avoiding(&grid, &region, me, near, &avoid)
                })
            })
        });
        match route {
            Some(route) => {
                let len: f32 = route.windows(2).map(|w| w[0].distance(w[1])).sum();
                tracing::info!(
                    "travel: step {} ({label}): {} waypoints, {len:.0} m",
                    self.travel.step,
                    route.len()
                );
                self.travel.route = Some(route);
                self.travel.next = 0;
                self.travel.restart_waypoint();
                true
            }
            None => {
                // A portal we cannot walk to is no use, whatever the
                // straight line said: drop it and find another way.
                if let Some((_, mouth)) = self.travel_portal() {
                    let step = self.travel.step;
                    let target_cell = self.travel.step_cell.unwrap_or(0);
                    tracing::warn!(
                        "travel: step {step} ({label}): no way there on foot; going another way \
                         (me cell {pl_cell:#010x} indoors {indoors}, target {target:?} \
                         cell {target_cell:#010x}, same block {same_block})"
                    );
                    self.travel.refused.push(mouth);
                    let goal = self.travel.goal;
                    self.cancel_travel_keeping_refusals();
                    return goal.is_some_and(|g| self.plan_trip(g));
                }
                tracing::warn!(
                    "travel: step {} ({label}): no route to {target:?}, giving up",
                    self.travel.step
                );
                self.end_trip();
                false
            }
        }
    }

    /// The step is done: move to the next one.
    fn travel_next_step(&mut self) -> bool {
        self.travel.step += 1;
        self.travel.replans = 0;
        self.travel.step_since = None;
        self.travel.step_cell = None;
        self.travel.step_target = None;
        self.travel.step_best = f32::INFINITY;
        self.travel.route = None;
        self.travel.portal_from = None;
        self.travel.portal_since = None;
        self.travel.recall_since = None;
        self.travel.recall_casts = 0;
        self.travel.restart_waypoint();
        self.travel_start_step()
    }

    /// [`travel_to`](Self::travel_to) a place of the gazetteer by name
    /// (`ac_world::towns::find`: case-insensitive, prefix or substring).
    pub fn travel_to_place(&mut self, name: &str) -> Result<(), String> {
        let place = ac_world::towns::find(name).ok_or_else(|| format!("unknown place '{name}'"))?;
        if self.travel_to(place.world_xy()) {
            Ok(())
        } else {
            Err(format!("no way to {} from here", place.name))
        }
    }

    /// The route being walked (world xy waypoints), for a map to draw.
    pub fn travel_route(&self) -> Option<&[Vec2]> {
        self.travel.route.as_deref()
    }

    /// `(step being made, steps in the journey)` while travelling.
    pub fn travel_progress(&self) -> Option<(usize, usize)> {
        self.travel
            .trip
            .as_ref()
            .map(|t| (self.travel.step.min(t.steps.len()), t.steps.len()))
    }

    /// The journey being made, for a map to draw or describe.
    pub fn travel_trip(&self) -> Option<&Trip> {
        self.travel.trip.as_ref()
    }

    /// How journeys are chosen. `Prefs::quick()` takes the fastest chain
    /// of portals; `Prefs::steady()` prefers fewer hops, since each one
    /// is a chance to be turned away or land somewhere unexpected.
    pub fn set_travel_prefs(&mut self, prefs: Prefs) {
        self.travel.prefs = prefs;
    }

    pub fn travel_prefs(&self) -> Prefs {
        self.travel.prefs
    }

    pub fn traveling(&self) -> bool {
        self.travel.trip.is_some()
    }

    /// Stop a journey because the player is doing something with the
    /// world instead: talking to a vendor, opening a chest, attacking.
    /// The server walks the character to whatever they are using, and a
    /// trip that carried on afterwards would walk them away again.
    ///
    /// A visit ends the same way, walking or not: the player is doing
    /// something else. A visit's own use takes the visit out first.
    pub(crate) fn interrupt_travel(&mut self, what: &str) {
        self.drop_visit(what);
        if self.traveling() {
            tracing::info!("travel: stopped, {what}");
            self.end_trip();
        }
    }

    /// Where the journey is going, while one is under way.
    pub fn travel_goal_xy(&self) -> Option<Vec2> {
        self.travel.goal
    }

    /// Give the character back to whoever is at the keyboard.
    ///
    /// Everything the client does that moves someone on its own: the
    /// journey it was walking, the route it was following, the person
    /// it was keeping up with, the corpse it was closing on. A hand on
    /// the movement keys means none of that gets to steer, and a client
    /// that keeps pulling against the player is worse than one that
    /// does nothing.
    pub fn stop_moving_by_itself(&mut self) {
        let was_busy =
            self.travel.trip.is_some() || self.follow.is_some() || self.visits.current.is_some();
        self.cancel_travel();
        if self.follow.take().is_some() {
            self.steering.reset();
        }
        self.autoplay.growth.let_go();
        if was_busy {
            tracing::info!("travel: the player is steering; letting go");
        }
    }

    /// Stop the journey, and the visit it may be part of: the player
    /// pressed Cancel, a script said so, or something else took over.
    pub fn cancel_travel(&mut self) {
        self.drop_visit("travel cancelled");
        self.end_trip();
    }

    /// End the journey and nothing else: it arrived, it gave up, or it is
    /// about to be planned again. A visit it was made for carries on.
    pub(crate) fn end_trip(&mut self) {
        if self.travel.trip.take().is_some() {
            tracing::info!("travel: cancelled");
        }
        self.travel.route = None;
        self.travel.step = 0;
        self.travel.portal_from = None;
        self.travel.portal_since = None;
        self.travel.recall_since = None;
        self.travel.recall_casts = 0;
        self.travel.gem_used = None;
        self.travel.gem_uses = 0;
        self.travel.gem_spot = None;
        self.travel.summoned_used = None;
        self.travel.summoned_uses = 0;
        self.travel.refused.clear();
        self.travel.refused_recalls.clear();
        self.travel.refused_gems.clear();
        self.travel.goal = None;
        self.travel.restart_waypoint();
    }

    /// The portal the current step is aimed at: its name and its mouth.
    fn travel_portal(&self) -> Option<(String, Vec2)> {
        match self.travel.trip.as_ref()?.steps.get(self.travel.step)? {
            Step::Portal { name, mouth, .. } => Some((name.clone(), *mouth)),
            Step::Walk(_) | Step::Recall { .. } | Step::Gem { .. } => None,
        }
    }

    /// The recall spell the current step is casting, while it is one.
    pub(crate) fn travel_recall_spell(&self) -> Option<u32> {
        match self.travel.trip.as_ref()?.steps.get(self.travel.step)? {
            Step::Recall { spell, .. } => Some(*spell),
            Step::Walk(_) | Step::Portal { .. } | Step::Gem { .. } => None,
        }
    }

    /// The recall being cast will not carry us off (the server refused
    /// it, or it has fizzled too often): plan the rest of the way
    /// without it. The new plan starts next frame.
    pub(crate) fn travel_recall_refused(&mut self) {
        let Some(spell) = self.travel_recall_spell() else {
            return;
        };
        tracing::warn!("travel: recall spell {spell} did not carry us off; going another way");
        self.travel.refused_recalls.push(spell);
        if self.travel.recall_leave_combat {
            self.leave_combat();
        }
        let goal = self.travel.goal;
        self.cancel_travel_keeping_refusals();
        if let Some(goal) = goal {
            self.plan_trip(goal);
        }
    }

    /// A gem step, a frame at a time: find room for the portal the gem
    /// summons, use the gem, use it again when the server did not take
    /// it, use the portal it put in front of the character, and go
    /// another way when none of that works. The one walk it asks for is a
    /// short step to face open ground, returned as the leg to walk.
    fn travel_gem_wait(
        &mut self,
        guid: u32,
        name: &str,
        summons: bool,
        now: Instant,
    ) -> Option<(Vec3, f32, u32)> {
        if self.travel.gem_used.is_none() {
            return self.travel_gem_first_use(guid, name, summons, now);
        }
        let since_use = now.duration_since(self.travel.gem_used.unwrap_or(now));
        let portal = if summons {
            self.summoned_portal()
        } else {
            None
        };
        let since_portal = self.travel.summoned_used.map(|t| now.duration_since(t));
        match gem_next(
            self.world.is_carried(guid),
            summons,
            since_use,
            self.travel.gem_uses,
            portal,
            since_portal,
            self.travel.summoned_uses,
        ) {
            GemNext::Wait => {}
            GemNext::UseGem if self.autoplay.cast_in_flight(now) => {}
            GemNext::UseGem => {
                tracing::info!("travel: {name} was not taken; using it again");
                self.travel.gem_uses += 1;
                self.travel.gem_used = Some(now);
                self.interact(guid);
            }
            GemNext::UsePortal(portal) => {
                tracing::info!("travel: using the portal {name} summoned");
                self.travel.summoned_uses += 1;
                self.travel.summoned_used = Some(now);
                // Sent straight to the server. Using something in the
                // world through `interact` stops the journey first, so
                // it does not walk us away afterwards -- and this use
                // *is* the journey.
                self.session
                    .send_action(ac_net::messages::action::USE, &portal.to_le_bytes());
            }
            GemNext::GiveUp => self.travel_gem_refused(guid, name, "did not take us anywhere"),
        }
        None
    }

    /// Before a gem is used: face somewhere the portal will fit, then use
    /// it. A summoned portal stands a few paces in front of the character
    /// and the server puts it nowhere at all when there is no room --
    /// having taken the gem anyway.
    fn travel_gem_first_use(
        &mut self,
        guid: u32,
        name: &str,
        summons: bool,
        now: Instant,
    ) -> Option<(Vec3, f32, u32)> {
        let (me3, block, forward) = {
            let pl = self.player.as_ref()?;
            (
                pl.world_position(),
                pl.cell & 0xFFFF_0000,
                pl.forward().truncate(),
            )
        };
        let me = me3.truncate();
        let walked_there = match self.travel.gem_spot {
            Some((spot, since)) => {
                me.distance(spot) <= ARRIVE.min(1.0) || now.duration_since(since) > FACE_GIVE_UP
            }
            None => false,
        };
        if summons && !walked_there {
            if let Some((spot, _)) = self.travel.gem_spot {
                return Some((Vec3::new(spot.x, spot.y, me3.z), 0.3, block));
            }
            match clear_heading(forward.normalize_or(Vec2::Y), |dir| {
                self.room_to_summon(dir)
            }) {
                None => {
                    self.travel_gem_refused(guid, name, "has no room to summon its portal here");
                    return None;
                }
                Some(dir) if dir.dot(forward.normalize_or(Vec2::Y)) < 0.99 => {
                    let spot = me + dir * FACE_STEP;
                    tracing::info!("travel: stepping to open ground to use {name}");
                    self.travel.gem_spot = Some((spot, now));
                    return Some((Vec3::new(spot.x, spot.y, me3.z), 0.3, block));
                }
                Some(_) => {}
            }
        }
        if self.autoplay.cast_in_flight(now) {
            // A spell is still on its way, and the use would be turned
            // away with the gem kept: wait for the slot.
            return None;
        }
        tracing::info!("travel: using {name}");
        self.travel.gem_spot = None;
        self.travel.gem_used = Some(now);
        self.travel.gem_uses = 1;
        self.interact(guid);
        None
    }

    /// Whether a portal summoned facing `dir` would have room: no wall
    /// between the character and past where it stands, ground there near
    /// the height of the character's feet, and nothing standing on it.
    fn room_to_summon(&mut self, dir: Vec2) -> bool {
        let assets = self.assets.clone();
        let Some(pl) = self.player.as_mut() else {
            return false;
        };
        let feet = pl.world_position();
        let chest = feet + Vec3::Z;
        let ahead = chest + dir.extend(0.0) * SUMMON_CLEARANCE;
        if pl.first_wall(&assets, chest, ahead).is_some() {
            return false;
        }
        let spot = feet.truncate() + dir * SUMMON_DISTANCE;
        let level = pl
            .ground_height(&assets, spot.x, spot.y, feet.z)
            .is_some_and(|h| (h - feet.z).abs() <= SUMMON_STEP);
        if !level {
            return false;
        }
        use ac_world::item_type::{CREATURE, LIFESTONE, PORTAL};
        let me = self.world.player_guid;
        !self.world.objects.values().any(|o| {
            Some(o.guid) != me
                && o.container.is_none()
                && o.wielder.is_none()
                && o.item_type & (CREATURE | PORTAL | LIFESTONE) != 0
                && o.position.is_some_and(|p| {
                    (ac_world::landblock_origin(p.cell) + p.local)
                        .truncate()
                        .distance(spot)
                        < SUMMON_ROOM
                })
        })
    }

    /// A gem that will not do its part on this journey: leave it out and
    /// plan the rest of the way without it.
    fn travel_gem_refused(&mut self, guid: u32, name: &str, why: &str) {
        tracing::warn!("travel: {name} {why}; going another way");
        self.travel.refused_gems.push(guid);
        let goal = self.travel.goal;
        self.cancel_travel_keeping_refusals();
        if let Some(goal) = goal {
            self.plan_trip(goal);
        }
    }

    /// The portal a gem has just put in front of the character: the
    /// nearest portal in the world within [`SUMMONED_REACH`].
    fn summoned_portal(&self) -> Option<u32> {
        let me = self.player.as_ref()?.world_position().truncate();
        self.world
            .objects
            .values()
            .filter(|o| o.item_type & ac_world::item_type::PORTAL != 0)
            .filter_map(|o| {
                let p = o.position?;
                let at = (ac_world::landblock_origin(p.cell) + p.local).truncate();
                let away = at.distance(me);
                (away <= SUMMONED_REACH).then_some((o.guid, away))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(guid, _)| guid)
    }

    /// The recall being cast fizzled: cast again as soon as the
    /// character has settled rather than wait out the whole timeout.
    pub(crate) fn travel_recall_fizzled(&mut self) {
        if self.travel_recall_spell().is_some() && self.travel.recall_casts > 0 {
            self.travel.recall_since = Some(
                Instant::now()
                    .checked_sub(RECALL_GIVE_UP)
                    .unwrap_or_else(Instant::now),
            );
        }
    }

    /// Give up the journey but remember which portals turned us away, so
    /// the next plan leaves them out.
    fn cancel_travel_keeping_refusals(&mut self) {
        let refused = std::mem::take(&mut self.travel.refused);
        let refused_recalls = std::mem::take(&mut self.travel.refused_recalls);
        let refused_gems = std::mem::take(&mut self.travel.refused_gems);
        let goal = self.travel.goal;
        self.end_trip();
        self.travel.refused = refused;
        self.travel.refused_recalls = refused_recalls;
        self.travel.refused_gems = refused_gems;
        self.travel.goal = goal;
    }

    /// The route no longer starts where the character stands (a teleport
    /// or server correction): aim the next leg afresh.
    pub(crate) fn travel_displaced(&mut self) {
        self.travel.leg = None;
    }

    /// Per frame while travelling: advance past reached waypoints, skip
    /// one the character cannot get closer to, and return the end of the
    /// current leg for the local move-to as `(world position, stop
    /// distance, cell)`. `None` when the route is done (or none is set).
    pub(crate) fn travel_goal(&mut self, now: Instant) -> Option<(Vec3, f32, u32)> {
        self.travel.trip.as_ref()?;
        let (me3, block, indoors) = {
            let pl = self.player.as_ref()?;
            (pl.world_position(), pl.cell & 0xFFFF_0000, pl.is_indoors())
        };
        let me = Vec2::new(me3.x, me3.y);
        // A portal step is over when the character is where the portal
        // comes out. Merely being in another landblock is not enough:
        // walking to a portal often crosses a boundary, and counting
        // that as having gone through skipped the rest of the journey.
        if let Some(from) = self.travel.portal_from {
            let step = self
                .travel
                .trip
                .as_ref()
                .and_then(|t| t.steps.get(self.travel.step))
                .cloned();
            let exit = step.as_ref().and_then(|s| s.exit());
            let arrived = match (exit, &step) {
                // A recall can land in the landblock we are already in,
                // so only the spot counts, and only once a cast is out.
                (Some((exit, _)), Some(Step::Recall { .. })) => {
                    self.travel.recall_casts > 0 && me.distance(exit) < 30.0
                }
                (Some((exit, exit_cell)), _) => {
                    block == exit_cell & 0xFFFF_0000 || me.distance(exit) < 30.0
                }
                (None, _) => block != from,
            };
            if arrived {
                // The jump was the portal's (or the spell's) doing, not
                // a stray one.
                self.travel.last_seen = Some(me);
                match step {
                    Some(Step::Recall { name, .. }) => {
                        tracing::info!("travel: {name} carried us off");
                        if self.travel.recall_leave_combat {
                            self.leave_combat();
                        }
                    }
                    _ => {
                        tracing::info!("travel: through the portal");
                        if let Some((_, mouth)) = self.travel_portal() {
                            self.travel.refused.retain(|r| r.distance(mouth) > 2.0);
                            // Portal Recall goes back to where the last
                            // portal led: this one, now.
                            if let Some(p) = ac_world::portals::near(mouth, 3.0).first() {
                                let local = p.to - ac_world::landblock_origin(p.to_cell);
                                self.learn_recall_position(
                                    ac_world::recalls::position_type::LAST_PORTAL,
                                    ac_world::object::Position::new_flat(p.to_cell, local),
                                );
                            }
                        }
                    }
                }
                if !self.travel_next_step() {
                    return None;
                }
            } else if let Some(Step::Recall { spell, name, .. }) = step {
                // Standing still, casting, and waiting to be carried
                // off. The cast is sent once the character has settled,
                // again when the first has not worked in a while, and
                // after that the journey goes another way.
                let since = *self.travel.recall_since.get_or_insert(now);
                let waited = now.duration_since(since);
                let casts = self.travel.recall_casts;
                let due = if casts == 0 {
                    waited >= RECALL_SETTLE
                } else {
                    waited >= RECALL_GIVE_UP
                };
                if due && casts < RECALL_TRIES {
                    // `/lifestone` is not a spell: the server has its
                    // own action for it and asks for no skill,
                    // components or casting at all. It is planned
                    // beside the spells and sent quite differently.
                    let sent = if ac_world::recalls::spell::is_free(spell) {
                        self.send_free_recall(spell)
                    } else {
                        // On ourselves, whatever is selected: a few
                        // recalls are written as targeted spells.
                        match self.world.player_guid {
                            Some(me) => self.cast_at(spell, me),
                            None => self.try_cast(spell),
                        }
                    };
                    match sent {
                        crate::magic::CastCheck::Ok => {
                            tracing::info!("travel: casting {name} ({})", casts + 1);
                            self.travel.recall_casts = casts + 1;
                            self.travel.recall_since = Some(now);
                        }
                        why => {
                            tracing::warn!("travel: cannot cast {name}: {why:?}");
                            self.travel_recall_refused();
                        }
                    }
                } else if due {
                    self.travel_recall_refused();
                }
                // Nothing to walk to: the move-to is left idle so the
                // character stands still for the cast.
                return None;
            } else if let Some(Step::Gem {
                guid,
                name,
                summons,
                ..
            }) = step
            {
                // Finding room for the portal, using the gem, and seeing
                // that it works. A step to open ground is the only walk.
                return self.travel_gem_wait(guid, &name, summons, now);
            }
        }
        // Coming no closer to this step's target for a long while: plan
        // again from where the character actually stands. This comes
        // before the indoor walk below, which hands back its leg and
        // returns: after it, a character standing still inside a hall
        // was never noticed and walked at a portal for four minutes.
        if let Some(target) = self.travel.step_target {
            let d = me.distance(target);
            if d < self.travel.step_best - STEP_PROGRESS {
                self.travel.step_best = d;
                self.travel.step_since = Some(now);
            }
        }
        if self
            .travel
            .step_since
            .is_some_and(|t| now.duration_since(t) > STEP_GIVE_UP)
        {
            if let Some(goal) = self.travel.goal {
                tracing::warn!(
                    "travel: step {} is going nowhere; planning again",
                    self.travel.step
                );
                // A portal that could not be got near is left out of the
                // new plan, or the plan would be the same one again.
                if let Some((name, mouth)) = self.travel_portal() {
                    tracing::warn!(
                        "travel: could not get to the portal {name:?}; going another way"
                    );
                    self.travel.refused.push(mouth);
                    self.cancel_travel_keeping_refusals();
                }
                self.travel.step_since = Some(now);
                self.plan_trip(goal);
            }
            // The new plan starts next frame: planning again from inside
            // this call could go round for ever.
            return None;
        }
        // Indoors (the Town Network hub, a dungeon) the world grid says
        // nothing and a landblock's cells can lie outside its own square,
        // so the leg is aimed straight at the target in the character's
        // own landblock and the landblock's navigation graph steers.
        // Inside the landblock the step is aimed into (the Town Network
        // hub, a dungeon), walk straight at the target and let the
        // landblock's own graph steer. Inside a *building* on the way,
        // with the target elsewhere, keep to the overland legs so the
        // character walks back out instead of pressing against a wall.
        let target_indoors = self.travel.step_cell.is_some_and(|c| c & 0xFFFF >= 0x100);
        if (indoors || target_indoors) && self.travel.step_block == Some(block) {
            let target = *self.travel.route.as_ref()?.last()?;
            if me.distance(target) > ARRIVE {
                return Some((Vec3::new(target.x, target.y, me3.z), LEG_STOP, block));
            }
            // At the target. A walk is done; a portal's mouth is waited
            // at below, like outdoors: jumped into, and given up on when
            // it will not take us. Aiming at the mouth for ever from on
            // top of it left a character standing on a dungeon's exit
            // portal that never fired.
            if self.travel.portal_from.is_none() && !self.travel_next_step() {
                return None;
            }
        }
        // A portal takes whoever touches it, so the character can be
        // carried off mid-walk by one the journey never meant to use.
        // Wherever they have landed, plan again from there.
        if let Some(last) = self.travel.last_seen {
            let jumped = me.distance(last) > 60.0;
            let expected = self
                .travel
                .trip
                .as_ref()
                .and_then(|t| t.steps.get(self.travel.step))
                .and_then(|s| s.exit())
                .is_some_and(|(exit, _)| me.distance(exit) < 60.0);
            if jumped && !expected {
                self.travel.last_seen = Some(me);
                // Whatever swept us up can still be walked into, so it is
                // not refused: refusing it once shut the way off the
                // continent. The route already pays to pass near one, and
                // the plan from here takes the new position as it is.
                if let Some(goal) = self.travel.goal {
                    tracing::info!("travel: carried off to {me:?}; planning again from here");
                    self.plan_trip(goal);
                }
                // Whatever the new plan is, it starts next frame: planning
                // again from inside this call could go round for ever.
                return None;
            }
        }
        self.travel.last_seen = Some(me);
        loop {
            let n = self.travel.route.as_ref()?.len();
            let waypoint = |t: &Travel, i: usize| t.route.as_ref().map(|r| r[i]);
            let mut next = self.travel.next;
            while next < n && waypoint(&self.travel, next).is_some_and(|w| me.distance(w) <= ARRIVE)
            {
                next += 1;
            }
            if next != self.travel.next {
                self.travel.next = next;
                self.travel.restart_waypoint();
                // Reaching a waypoint is progress even when the detour
                // it is on leads away from the step's target for a while
                // (all the way round a town wall).
                self.travel.step_since = Some(now);
            }
            if next >= n {
                // The step's route is walked. A portal step is not done
                // until the character has actually gone through, so wait
                // at its mouth rather than give up.
                if self.travel.portal_from.is_some() {
                    // The clock starts when we reach the mouth, not when
                    // the step began: the walk there can be long.
                    if self.travel.portal_since.is_none() {
                        self.travel.portal_since = Some(now);
                    }
                    // Standing at its mouth and still here: it will not
                    // take us. Plan the rest of the way without it.
                    if self
                        .travel
                        .portal_since
                        .is_some_and(|t| now.duration_since(t) > PORTAL_GIVE_UP)
                    {
                        if let Some((name, mouth)) = self.travel_portal() {
                            let level = self.world.stats.level.max(1) as u32;
                            let why = ac_world::portals::near(mouth, 3.0)
                                .first()
                                .and_then(|p| p.refusal(level))
                                .unwrap_or_else(|| "would not take us".into());
                            tracing::warn!("travel: the portal {name:?} {why}; going another way");
                            self.travel.refused.push(mouth);
                        }
                        let goal = self.travel.goal;
                        self.cancel_travel_keeping_refusals();
                        if let Some(goal) = goal {
                            self.plan_trip(goal);
                        }
                        // Next frame walks the new plan: replanning and
                        // carrying on inside one call risks going round.
                        return None;
                    }
                    // A few portals sit above the ground and have to be
                    // jumped into; try that while we wait.
                    if self
                        .travel
                        .portal_since
                        .is_some_and(|t| now.duration_since(t) > Duration::from_secs(3))
                        && self
                            .travel
                            .last_hop
                            .is_none_or(|t| now.duration_since(t) > Duration::from_secs(2))
                    {
                        self.travel.last_hop = Some(now);
                        tracing::info!("travel: jumping into the portal");
                        self.jump(1.0);
                    }
                    let mouth = waypoint(&self.travel, n - 1)?;
                    let z = self
                        .travel
                        .grid
                        .as_ref()
                        .map(|g| g.height_at(mouth))
                        .unwrap_or(me.extend(0.0).z);
                    return Some((
                        Vec3::new(mouth.x, mouth.y, z),
                        0.0,
                        WorldGrid::block_of(mouth),
                    ));
                }
                if !self.travel_next_step() {
                    return None;
                }
                continue;
            }
            let wp = waypoint(&self.travel, next)?;
            // How far there is still to go: along the route being steered
            // (and from its end on to the waypoint) while there is one,
            // as the crow flies otherwise.
            let (d, plan) = match self.steering.remaining(me3) {
                Some((along, end, planned)) => {
                    (along + Vec2::new(end.x, end.y).distance(wp), Some(planned))
                }
                None => (me.distance(wp), None),
            };
            // A different route is a different yardstick: progress is
            // measured from here on, but the clock keeps running, so a
            // character that is not moving is still caught.
            if plan != self.travel.measured_on {
                self.travel.measured_on = plan;
                self.travel.best = d;
                self.travel.last_progress.get_or_insert(now);
            }
            match self.travel.last_progress {
                Some(t) if d >= self.travel.best - PROGRESS => {
                    if now.duration_since(t) >= STUCK_AFTER {
                        // The last waypoint is where the step ends; not
                        // reaching it is not arriving. Plan the journey
                        // again from here, a few times, before giving up.
                        if next + 1 >= n && me.distance(wp) > 2.0 * ARRIVE {
                            if self.travel.replans >= REPLANS {
                                tracing::warn!("travel: cannot reach {wp:?} from here; giving up");
                                self.end_trip();
                                return None;
                            }
                            tracing::warn!(
                                "travel: no way to the end of the step at {wp:?}; planning again from here"
                            );
                            let replans = self.travel.replans + 1;
                            if let Some(goal) = self.travel.goal {
                                self.plan_trip(goal);
                            }
                            self.travel.replans = replans;
                            return None;
                        }
                        tracing::warn!(
                            "travel: no progress toward waypoint {next}/{n} at {wp:?} for {:?}, skipping it",
                            STUCK_AFTER
                        );
                        self.travel.next = next + 1;
                        self.travel.restart_waypoint();
                        continue;
                    }
                }
                _ => {
                    self.travel.best = d;
                    self.travel.last_progress = Some(now);
                }
            }
            let leg = match self.travel.leg {
                Some(l) if me.distance(l) > ARRIVE && me.distance(l) <= 2.0 * LEG => l,
                _ => {
                    let l = leg_end(me, wp);
                    self.travel.leg = Some(l);
                    l
                }
            };
            let z = self
                .travel
                .grid
                .as_ref()
                .map(|g| g.height_at(leg))
                .unwrap_or(0.0);
            return Some((
                Vec3::new(leg.x, leg.y, z),
                LEG_STOP,
                WorldGrid::block_of(leg),
            ));
        }
    }
}

/// Where the next leg from `me` toward `wp` ends: at most [`LEG`] away,
/// and inside the landblock `me` is in (cut short of its edge) unless
/// that would leave nothing to walk, in which case the leg crosses the
/// edge straight and the next one is planned in the new block.
pub fn leg_end(me: Vec2, wp: Vec2) -> Vec2 {
    let d = wp - me;
    let dist = d.length();
    if dist < 1e-3 {
        return wp;
    }
    let dir = d / dist;
    let far = if dist <= LEG { wp } else { me + dir * LEG };
    let block = 192.0;
    let lo = (me / block).floor() * block;
    let hi = lo + Vec2::splat(block);
    // Parametric distance along the leg to the block boundary.
    let mut t_exit = f32::INFINITY;
    for axis in 0..2 {
        let (p, v) = (me[axis], (far - me)[axis]);
        if v > 1e-6 {
            t_exit = t_exit.min((hi[axis] - p) / v);
        } else if v < -1e-6 {
            t_exit = t_exit.min((lo[axis] - p) / v);
        }
    }
    if t_exit >= 1.0 {
        return far;
    }
    let clipped = me + (far - me) * t_exit - dir * EDGE_MARGIN;
    if me.distance(clipped) > ARRIVE + 1.0 {
        clipped
    } else {
        far
    }
}

/// The way to face to use a gem: the way the character already faces
/// when that is clear, else the nearest clear heading to it, turning a
/// twelfth of the circle at a time either side. `None` when nowhere is.
fn clear_heading(forward: Vec2, mut clear: impl FnMut(Vec2) -> bool) -> Option<Vec2> {
    const STEPS: i32 = 6;
    let turn = std::f32::consts::PI / STEPS as f32;
    let mut tried = vec![0];
    for k in 1..STEPS {
        tried.push(k);
        tried.push(-k);
    }
    tried.push(STEPS);
    tried
        .into_iter()
        .map(|k| Vec2::from_angle(turn * k as f32).rotate(forward))
        .find(|d| clear(*d))
}

/// What a gem step does next, standing where the gem was used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GemNext {
    /// Nothing yet: stand still.
    Wait,
    /// The server did not take the gem: use it again.
    UseGem,
    /// Use this portal, the one the gem summoned. A summoned portal is
    /// used, not walked into.
    UsePortal(u32),
    /// It is not working: go another way.
    GiveUp,
}

/// Decide it from what can be seen: whether the gem is still carried,
/// whether it is the kind that summons a portal, how long since it was
/// last used and how often, and the summoned portal if one is in reach,
/// with how long since that was last used and how often.
///
/// The server takes a gem only when a use gets through, so a gem still
/// in the pack is the plain sign of a refusal -- which a busy character
/// gets, and which says so in no other way the journey can see.
fn gem_next(
    carried: bool,
    summons: bool,
    since_use: Duration,
    uses: u32,
    portal: Option<u32>,
    since_portal: Option<Duration>,
    portal_uses: u32,
) -> GemNext {
    if carried {
        return if since_use < GEM_RETRY {
            GemNext::Wait
        } else if uses < GEM_TRIES {
            GemNext::UseGem
        } else {
            GemNext::GiveUp
        };
    }
    let waited_out = if since_use < GEM_GIVE_UP {
        GemNext::Wait
    } else {
        GemNext::GiveUp
    };
    // The rare gem that carries the character off itself: the arrival
    // ends the step, and there is nothing to use.
    if !summons {
        return waited_out;
    }
    match portal {
        None => waited_out,
        Some(_) if since_portal.is_some_and(|d| d < SUMMONED_USE_EVERY) => GemNext::Wait,
        Some(_) if portal_uses >= SUMMONED_TRIES => GemNext::GiveUp,
        Some(guid) => GemNext::UsePortal(guid),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gem_is_used_facing_the_nearest_open_ground() {
        let north = Vec2::Y;
        // Room straight ahead: that way, no turning.
        assert_eq!(clear_heading(north, |_| true), Some(north));
        // A wall ahead and to the right: the nearest clear heading, just
        // to the left, not the far side of the circle.
        let d = clear_heading(north, |d| d.x < -0.4).unwrap();
        assert!(d.x < -0.4 && d.y > 0.8, "{d:?}");
        // Boxed in: nowhere, so the gem is not spent.
        assert_eq!(clear_heading(north, |_| false), None);
        // Only straight behind is clear: still found.
        let back = clear_heading(north, |d| d.y < -0.99).unwrap();
        assert!(back.y < -0.99, "{back:?}");
        // Every heading is tried exactly once.
        let mut n = 0;
        clear_heading(north, |_| {
            n += 1;
            false
        });
        assert_eq!(n, 12);
    }

    #[test]
    fn a_gem_is_used_again_until_the_server_takes_it() {
        let s = Duration::from_secs;
        // Just used: give the server a moment to take it.
        assert_eq!(gem_next(true, true, s(1), 1, None, None, 0), GemNext::Wait);
        // Still in the pack a while later: refused, most likely because
        // the character was mid-cast. Use it again.
        assert_eq!(
            gem_next(true, true, s(4), 1, None, None, 0),
            GemNext::UseGem
        );
        // Refused every time: go another way rather than stand here.
        assert_eq!(
            gem_next(true, true, s(4), GEM_TRIES, None, None, 0),
            GemNext::GiveUp
        );
    }

    #[test]
    fn a_summoned_portal_is_used_not_walked_into() {
        let s = Duration::from_secs;
        // Taken, and the portal is up: use it.
        assert_eq!(
            gem_next(false, true, s(1), 1, Some(7), None, 0),
            GemNext::UsePortal(7)
        );
        // Just used: give the server time to carry us off.
        assert_eq!(
            gem_next(false, true, s(2), 1, Some(7), Some(s(1)), 1),
            GemNext::Wait
        );
        // Still here after that: use it again, and in the end give up.
        assert_eq!(
            gem_next(false, true, s(9), 1, Some(7), Some(s(4)), 1),
            GemNext::UsePortal(7)
        );
        assert_eq!(
            gem_next(false, true, s(20), 1, Some(7), Some(s(4)), SUMMONED_TRIES),
            GemNext::GiveUp
        );
        // Taken, but nothing has appeared yet: wait, and not for ever.
        assert_eq!(gem_next(false, true, s(2), 1, None, None, 0), GemNext::Wait);
        assert_eq!(
            gem_next(false, true, GEM_GIVE_UP, 1, None, None, 0),
            GemNext::GiveUp
        );
        // A gem that carries the character off itself has nothing to
        // use, whatever portal happens to be standing nearby.
        assert_eq!(
            gem_next(false, false, s(2), 1, Some(7), None, 0),
            GemNext::Wait
        );
    }

    #[test]
    fn legs_are_short_and_stay_in_the_block() {
        // Mid-block, far goal along +x: a LEG-long leg.
        let me = Vec2::new(192.0 * 10.0 + 20.0, 192.0 * 10.0 + 96.0);
        let wp = me + Vec2::new(1000.0, 0.0);
        let l = leg_end(me, wp);
        assert!((l - (me + Vec2::new(LEG, 0.0))).length() < 1e-3, "{l:?}");
        // Near the block's east edge with the goal beyond it: cut short
        // of the edge...
        let me = Vec2::new(192.0 * 11.0 - 30.0, 192.0 * 10.0 + 96.0);
        let l = leg_end(me, wp);
        assert!((l.x - (192.0 * 11.0 - EDGE_MARGIN)).abs() < 1e-3, "{l:?}");
        assert_eq!(WorldGrid::block_of(l), WorldGrid::block_of(me));
        // ...unless that leaves nothing to walk: then straight across.
        let me = Vec2::new(192.0 * 11.0 - 3.0, 192.0 * 10.0 + 96.0);
        let l = leg_end(me, wp);
        assert!(l.x > 192.0 * 11.0, "{l:?}");
        // A close goal is the leg's end.
        let wp = me + Vec2::new(0.0, -10.0);
        assert_eq!(leg_end(me, wp), wp);
        // Diagonal toward the north-west corner: the nearer edge cuts.
        let me = Vec2::new(192.0 * 10.0 + 10.0, 192.0 * 11.0 - 40.0);
        let l = leg_end(me, me + Vec2::new(-100.0, 100.0));
        assert_eq!(WorldGrid::block_of(l), WorldGrid::block_of(me));
        assert!(l.x > 192.0 * 10.0 && l.y < 192.0 * 11.0, "{l:?}");
    }
}
