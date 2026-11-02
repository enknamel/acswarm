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
use ac_world::Place;
use glam::{Vec2, Vec3};

use crate::body::server_walk::reached;
use crate::did::Did;
use crate::gem_room::{clear_heading, gem_next, GemNext};
use crate::{Client, Goal, Owner};

/// A waypoint counts as reached within this distance (metres, flat).
pub const ARRIVE: f32 = 3.0;
/// Longest leg handed to the local move-to (metres).
pub const LEG: f32 = 60.0;

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
/// before the teleport) is cast again, and after `RECALL_TRIES` casts
/// the journey is planned again without that spell.
pub const RECALL_GIVE_UP: Duration = Duration::from_secs(15);
const RECALL_TRIES: u32 = 2;
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
/// That step is walked to within this (metres, 3D): the heading is what it is for.
const FACE_REACH: f32 = 0.3;
/// No progress toward the current waypoint for this long: skip it.
/// Progress is measured along the route the steering is following
/// where there is one, since a detour around a wall walks away from the
/// waypoint for a good while before it comes back.
pub const STUCK_AFTER: Duration = Duration::from_secs(10);
/// Getting closer to the waypoint by less than this is not progress.
const PROGRESS: f32 = 1.0;

/// What the step being made keeps, replaced whole as a step begins: nothing of one step reaches
/// the next.
#[derive(Default)]
struct StepState {
    /// The landblock stood in as a portal, recall or gem step began; done once somewhere else.
    portal_from: Option<u32>,
    /// When the character first stood at the portal's mouth.
    portal_since: Option<Instant>,
    /// When we last tried jumping into a portal that has not taken us.
    last_hop: Option<Instant>,
    /// When the recall step's spell was last sent (or the step began), and how many times.
    recall_since: Option<Instant>,
    recall_casts: u32,
    /// The character was in peace mode before the cast: back to it once the recall has landed.
    recall_leave_combat: bool,
    /// When the gem step's gem was last used, and how many times.
    gem_used: Option<Instant>,
    gem_uses: u32,
    /// The open ground walked to before the gem is used, and whether that walk is over.
    gem_spot: Option<Place>,
    faced: bool,
    /// When the portal the gem summoned was last used, and how many times.
    summoned_used: Option<Instant>,
    summoned_uses: u32,
    /// The step's walk end on its floor, once the region holds it.
    end: Option<Place>,
    /// The stretch asked this frame is an overland leg.
    on_leg: bool,
    /// World xy waypoints of the overland route, start and end included.
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
}

impl StepState {
    fn restart_waypoint(&mut self) {
        self.leg = None;
        self.best = f32::INFINITY;
        self.last_progress = None;
        self.measured_on = None;
    }
}

/// The journey being made, and the terrain data it needs.
#[derive(Default)]
pub struct Travel {
    grid: Option<Rc<WorldGrid>>,
    region: Option<Rc<Region>>,
    /// The whole journey, portals included, and the step being made.
    trip: Option<Trip>,
    step: usize,
    /// What the step being made keeps.
    state: StepState,
    /// Where the character stood last frame, to notice a portal that
    /// took them somewhere the journey did not ask for.
    last_seen: Option<Vec2>,
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
    /// Gems that did not take us anywhere *on this journey*: left out of
    /// the next plan, like `refused_recalls`.
    refused_gems: Vec<u32>,
    /// The quest flags the character holds that a portal asks for (`crate::quest_flags`).
    pub(crate) quests_done: Vec<String>,
    /// The last tie spell cast (Lifestone Tie, a Portal Tie), so the
    /// "successfully linked" that follows is filed under its position.
    pub(crate) last_tie: Option<u32>,
    /// A recall asked for that goes to a saved position: that position's kind, where we stood and
    /// when. Where it lands is learnt as the position (see `Client::recall_landed`).
    pub(crate) recall_sent: Option<(u32, Vec3, Instant)>,
    /// Where the journey is bound (always `Goal::At`), to replan around a refusal.
    goal: Option<Goal>,
    /// Who set the journey off: a journey refused is kept refused for it (`Client.refusals`).
    owner: Option<Owner>,
    /// A step was left out: the next frame plans the journey again from where the body stands.
    pub(crate) replan_due: bool,
    /// Why the last journey was refused, for [`Client::set_off`] to answer with.
    why_refused: Option<String>,
    /// Last frame's overland leg was refused by the slot.
    leg_refused: bool,
    /// The last journey was broken off by something else the character
    /// went to do -- a corpse to open, a fight, a dodge -- rather than
    /// arriving, giving up or being cancelled. Cleared when the next one
    /// sets off (see [`Client::journey_broken_off`]).
    broken_off: bool,
    /// The journey under way is about the character's own hunting
    /// ground -- a roam, a patrol, a follower keeping up with its leader
    /// -- rather than a road to somewhere else (see
    /// [`Client::travel_about`]). Kept through a replan of the same
    /// journey, and let go with a new destination.
    about_the_ground: bool,
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

    /// Plan a journey from where the character stands to `goal` (world
    /// xy) and start it. Portals are used when they are quicker than
    /// walking, and so are the recall spells the character can cast
    /// right now (`castable_recalls`) unless the journey's `Prefs` say
    /// `use_recalls: false`. False when nothing reaches the goal.
    ///
    /// A journey asked for from outside ends any visit being made, and the walk in the slot.
    pub fn travel_to(&mut self, goal: Vec2) -> bool {
        self.journey_to(Place::on_map(goal), ARRIVE)
    }

    /// [`travel_to`](Self::travel_to) somewhere about the character's own
    /// hunting ground: a roam, a patrol, a follower keeping up. The
    /// journey is not a road: what stands about the ground is what the
    /// character came to fight, and a patrol that walked past it would
    /// never fight anything. Every other journey is a road, whoever set
    /// it off -- a script, a hunting area's portal, an errand of the
    /// growth rules, the player -- because going somewhere is the same
    /// errand whoever asked for it (see `Client::on_its_way`).
    pub(crate) fn travel_about(&mut self, goal: Goal) -> bool {
        let ok = self.journey_to_goal(goal);
        if ok {
            self.travel.about_the_ground = true;
        }
        ok
    }

    /// Say the journey just set off on is about the character's own ground (see
    /// [`Self::travel_about`]), for a walk begun some other way.
    pub(crate) fn mark_ground_walk(&mut self) {
        self.travel.about_the_ground = true;
    }

    /// Whether the journey under way, or the last one, was about the
    /// character's own ground (see [`Self::travel_about`]). Read by the
    /// walk past the road, and when a journey is put down for a fight,
    /// so that it is picked up again as what it was.
    pub(crate) fn travel_about_the_ground(&self) -> bool {
        self.travel.about_the_ground
    }

    /// [`travel_to`](Self::travel_to) a place, in its cell, to within `within` (metres, 3D).
    pub fn journey_to(&mut self, place: Place, within: f32) -> bool {
        self.journey_to_goal(Goal::At { place, within })
    }

    /// [`journey_to`](Self::journey_to) a goal: a place and how near, set off for the running row.
    fn journey_to_goal(&mut self, goal: Goal) -> bool {
        self.drop_visit("travelling somewhere else");
        let owner = self.step_owner();
        // The frame's clock, or the wall's when no frame has run since it was set.
        let now = self.frame_at.max(Instant::now());
        self.set_off(owner, goal, now).fine()
    }

    /// Plan a journey to `goal` for `owner` and set off on it: the trip from where the body stands,
    /// then its first stretch asked of the slot at once, so a first walk with no way refuses it
    /// before a step (a_journey_whose_first_walk_has_no_way_is_refused_at_once).
    pub(crate) fn set_off(&mut self, owner: Owner, goal: Goal, now: Instant) -> Did {
        let trip = match self.plan_journey(goal) {
            Ok(trip) => trip,
            Err(why) => return Did::blocked(why),
        };
        if let Some(me) = self.my_position() {
            self.replace_walk(owner, me);
        }
        self.start_trip(owner, trip, goal);
        self.walk_journey(now);
        if self.traveling() {
            return Did::Acting;
        }
        match self.travel.why_refused.take() {
            Some(why) => Did::blocked(why),
            None => Did::Done,
        }
    }

    /// The trip to `goal` from where the character stands, leaving out what this goal's journey
    /// was refused; why not, said in chat, when nothing reaches it.
    fn plan_journey(&mut self, goal: Goal) -> Result<Trip, String> {
        let Some(place) = goal.place() else {
            return Err("not a place".into());
        };
        let xy = place.at.truncate();
        let Some(pl) = self.player.as_ref() else {
            tracing::warn!("travel: the character is not in the world");
            return Err("not in the world yet".into());
        };
        let me = pl.world_position();
        let cell = pl.cell;
        let t0 = Instant::now();
        // A new destination starts afresh: what turned us away on the
        // last journey says nothing about this one.
        if self.travel_goal_xy().is_none_or(|g| g.distance(xy) > 1.0) {
            self.travel.refused.clear();
            self.travel.refused_recalls.clear();
            self.travel.refused_gems.clear();
            // A new journey is a road until whoever planned it says
            // otherwise (see `travel_about`).
            self.travel.about_the_ground = false;
        }
        let level = self.world.stats.level.max(1) as u32;
        let refused = self.travel.refused.clone();
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
        tracing::info!(
            "travel: planning to {xy:?} in {:#010x} from {cell:#010x}, underground {underground}",
            place.cell
        );
        let cell = planning_cell(cell, me, underground);
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
        let quests = self.travel.quests_done.clone();
        let from = Place { at: me, cell };
        let to = self.planning_place(place);
        let mut planned = trip::plan_with_recalls_and_gems(
            from, to, level, &quests, &refused, &recalls, &gems, prefs,
        );
        // Nothing within a believable walk or a portal: walk the whole way, as the last resort.
        // A ground reached by a one-way portal has no way back otherwise, and the character was
        // left there (a_ground_reached_through_a_one_way_portal_has_a_way_home_on_foot).
        if planned.is_none() {
            let far = trip::Prefs {
                walk_reach: trip::Prefs::far().walk_reach,
                portal_reach: trip::Prefs::far().portal_reach,
                ..prefs
            };
            planned = trip::plan_with_recalls_and_gems(
                from, to, level, &quests, &refused, &recalls, &gems, far,
            );
            if let Some(t) = &planned {
                tracing::info!("travel: no shorter way; the long walk, {:.0} s", t.seconds);
            }
        }
        let Some(mut trip) = planned else {
            // Say why. Often there is a way, but not one this character
            // may take yet: every chain to the far continents runs
            // through a portal with a level on it.
            let anyone = trip::plan_with(from, to, 0, &[], &[], trip::Prefs::far());
            let why = match anyone {
                Some(t) => {
                    let asks = t
                        .steps
                        .iter()
                        .filter_map(|s| match s {
                            Step::Portal { mouth, name, .. } => {
                                ac_world::portals::near(mouth.at.truncate(), 3.0)
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
            tracing::warn!("travel: {why} ({xy:?}, level {level})");
            // The planner's inputs, so telemetry can replay the refusal.
            self.events.push(crate::Event::Noted(format!(
                "travel: {why} (to {xy:?} in {:#010x} from {cell:#010x} at {me:?}, level {level})",
                place.cell
            )));
            self.events.push(crate::Event::Chat {
                text: why.clone(),
                kind: 1,
            });
            return Err(why);
        };
        // Planned to the ground outside a building, the walk still ends at the goal itself.
        if let Some(Step::Walk(end)) = trip.steps.last_mut() {
            *end = place;
        }
        tracing::info!(
            "travel: {} ({} steps, planned in {:?})",
            trip.summary(),
            trip.steps.len(),
            t0.elapsed()
        );
        Ok(trip)
    }

    /// Take `trip` to `goal`, set off by `owner`, as the journey under way, at its first step.
    pub(crate) fn start_trip(&mut self, owner: Owner, trip: Trip, goal: Goal) {
        self.travel.trip = Some(trip);
        self.travel.step = 0;
        self.travel.goal = Some(goal);
        self.travel.owner = Some(owner);
        self.travel.broken_off = false;
        self.travel.replan_due = false;
        self.travel.why_refused = None;
        self.travel.last_seen = self.my_position().map(Vec3::truncate);
        self.travel_start_step();
    }

    /// Begin the step the trip is on, its state afresh. False when the trip is done (arrived).
    fn travel_start_step(&mut self) -> bool {
        self.travel.state = StepState::default();
        let Some(step) = self.travel_step().cloned() else {
            tracing::info!("travel: arrived");
            // The journey's own end, not the player's: a visit it was
            // made for carries on from here.
            self.end_trip();
            return false;
        };
        let Some(block) = self.player.as_ref().map(|p| p.cell & 0xFFFF_0000) else {
            return false;
        };
        let n = self.travel.step;
        match &step {
            // A recall step has nowhere to walk: stand still, cast, and wait to be carried off,
            // which `step_goal` sees to; done when the character is where the spell lands.
            Step::Recall { name, spell, .. } => {
                tracing::info!("travel: step {n} (recall {name:?}, spell {spell})");
                self.travel.state.portal_from = Some(block);
                self.travel.state.recall_leave_combat = !self.combat && !self.magic;
            }
            // A gem is used where it stands: the usual kind puts a portal in front of the
            // character, the rare kind carries them off itself; done once somewhere else.
            Step::Gem { name, summons, .. } => {
                let does = if *summons {
                    "summons a portal"
                } else {
                    "carries us off"
                };
                tracing::info!("travel: step {n} ({name:?}, {does})");
                self.travel.state.portal_from = Some(block);
            }
            Step::Portal { name, mouth, .. } => {
                tracing::info!(
                    "travel: step {n} (portal {name:?} at {:?} in {:#010x})",
                    mouth.at,
                    mouth.cell
                );
                self.travel.state.portal_from = Some(block);
            }
            Step::Walk(p) => {
                tracing::info!("travel: step {n} (walk to {:?} in {:#010x})", p.at, p.cell);
            }
        }
        true
    }

    /// The step is done: move to the next one.
    fn travel_next_step(&mut self) -> bool {
        self.travel.step += 1;
        self.travel_start_step()
    }

    /// The step the trip is on.
    fn travel_step(&self) -> Option<&Step> {
        self.travel.trip.as_ref()?.steps.get(self.travel.step)
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
        self.travel.state.route.as_deref()
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
            self.travel.broken_off = true;
        }
    }

    /// Whether the character is between journeys because the last one was
    /// broken off by something else it went to do: using something, a
    /// fight, a walk to a corpse. Such a journey was never going to end
    /// by itself, and whatever set it off can plan it again once the
    /// character is free. One that arrived, gave up or was cancelled
    /// answers no.
    pub(crate) fn journey_broken_off(&self) -> bool {
        !self.traveling() && self.travel.broken_off
    }

    /// Where the journey is going (world xy), while one is under way.
    pub fn travel_goal_xy(&self) -> Option<Vec2> {
        self.journey_goal()
            .and_then(Goal::place)
            .map(|p| p.at.truncate())
    }

    /// The journey's goal: its place, in its cell, and how near.
    pub fn journey_goal(&self) -> Option<Goal> {
        self.travel.goal
    }

    /// The place the trip planner is told for `goal`: a building's room is planned as the
    /// ground outside it, whose door is walked through; a dungeon's keeps its cell.
    pub(crate) fn planning_place(&mut self, goal: Place) -> Place {
        if !goal.is_indoors() {
            return goal;
        }
        let dungeon = self
            .travel_terrain()
            .is_none_or(|(grid, _)| grid.is_dungeon_block(goal.cell));
        if dungeon {
            return goal;
        }
        Place {
            cell: planning_cell(goal.cell, goal.at, false),
            ..goal
        }
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
            self.travel.trip.is_some() || self.going.is_some() || self.visits.current.is_some();
        self.cancel_travel();
        if self.going.take().is_some() {
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
        // Cancelled is meant: nothing picks this journey up again.
        self.travel.broken_off = false;
    }

    /// End the journey and nothing else: it arrived, it gave up, or it is
    /// about to be planned again. A visit it was made for carries on.
    pub(crate) fn end_trip(&mut self) {
        if self.travel.trip.take().is_some() {
            tracing::info!("travel: cancelled");
        }
        self.travel.step = 0;
        self.travel.state = StepState::default();
        self.travel.refused.clear();
        self.travel.refused_recalls.clear();
        self.travel.refused_gems.clear();
        self.travel.goal = None;
        self.travel.owner = None;
        self.travel.replan_due = false;
        self.travel.leg_refused = false;
    }

    /// The portal the current step is aimed at: its name and its mouth.
    pub(crate) fn travel_portal(&self) -> Option<(String, Vec2)> {
        match self.travel_step()? {
            Step::Portal { name, mouth, .. } => Some((name.clone(), mouth.at.truncate())),
            Step::Walk(_) | Step::Recall { .. } | Step::Gem { .. } => None,
        }
    }

    /// The recall spell the current step is casting, while it is one.
    pub(crate) fn travel_recall_spell(&self) -> Option<u32> {
        match self.travel_step()? {
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
        if self.travel.state.recall_leave_combat {
            self.leave_combat();
        }
        self.travel.replan_due = true;
    }

    /// A gem step, a frame at a time: find room for the portal the gem
    /// summons, use the gem, use it again when the server did not take
    /// it, use the portal it put in front of the character, and go
    /// another way when none of that works. The one walk it asks for is a
    /// short step to face open ground, returned as the stretch to walk.
    fn travel_gem_wait(
        &mut self,
        guid: u32,
        name: &str,
        summons: bool,
        now: Instant,
    ) -> Option<Goal> {
        if self.travel.state.gem_used.is_none() {
            return self.travel_gem_first_use(guid, name, summons, now);
        }
        let since_use = now.duration_since(self.travel.state.gem_used.unwrap_or(now));
        let portal = if summons {
            self.summoned_portal()
        } else {
            None
        };
        let since_portal = self
            .travel
            .state
            .summoned_used
            .map(|t| now.duration_since(t));
        match gem_next(
            self.world.is_carried(guid),
            summons,
            since_use,
            self.travel.state.gem_uses,
            portal,
            since_portal,
            self.travel.state.summoned_uses,
        ) {
            GemNext::Wait => {}
            GemNext::UseGem if self.autoplay.cast_in_flight(now) => {}
            GemNext::UseGem => {
                tracing::info!("travel: {name} was not taken; using it again");
                self.travel.state.gem_uses += 1;
                self.travel.state.gem_used = Some(now);
                self.interact(guid);
            }
            GemNext::UsePortal(portal) => {
                tracing::info!("travel: using the portal {name} summoned");
                self.travel.state.summoned_uses += 1;
                self.travel.state.summoned_used = Some(now);
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
    ) -> Option<Goal> {
        let (me3, forward) = {
            let pl = self.player.as_ref()?;
            (pl.world_position(), pl.forward().truncate())
        };
        if summons && !self.travel.state.faced {
            // The step to open ground is the slot's walk; arrived or refused, the gem is used.
            if let Some(place) = self.travel.state.gem_spot {
                let within = FACE_REACH;
                return Some(Goal::At { place, within });
            }
            let ahead = forward.normalize_or(Vec2::Y);
            match clear_heading(ahead, |dir| self.room_to_summon(dir)) {
                None => {
                    self.travel_gem_refused(guid, name, "has no room to summon its portal here");
                    return None;
                }
                Some(dir) if dir.dot(ahead) < 0.99 => {
                    let spot = me3.truncate() + dir * FACE_STEP;
                    let assets = self.assets.clone();
                    let floor = self
                        .player
                        .as_mut()
                        .and_then(|pl| pl.ground_height(&assets, spot.x, spot.y, me3.z));
                    if let Some(z) = floor {
                        tracing::info!("travel: stepping to open ground to use {name}");
                        let place = self.place_at(spot.extend(z));
                        self.travel.state.gem_spot = Some(place);
                        let within = FACE_REACH;
                        return Some(Goal::At { place, within });
                    }
                }
                Some(_) => {}
            }
            self.travel.state.faced = true;
        }
        if self.autoplay.cast_in_flight(now) {
            // A spell is still on its way, and the use would be turned
            // away with the gem kept: wait for the slot.
            return None;
        }
        tracing::info!("travel: using {name}");
        self.travel.state.gem_spot = None;
        self.travel.state.gem_used = Some(now);
        self.travel.state.gem_uses = 1;
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
    /// plan the rest of the way without it, next frame.
    fn travel_gem_refused(&mut self, guid: u32, name: &str, why: &str) {
        tracing::warn!("travel: {name} {why}; going another way");
        self.travel.refused_gems.push(guid);
        self.travel.replan_due = true;
    }

    /// The portal a gem has just put in front of the character: the
    /// nearest portal in the world within [`SUMMONED_REACH`].
    fn summoned_portal(&self) -> Option<u32> {
        let me = self.my_position()?.truncate();
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
        if self.travel_recall_spell().is_some() && self.travel.state.recall_casts > 0 {
            self.travel.state.recall_since = Some(
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
        let (goal, owner) = (self.travel.goal, self.travel.owner);
        self.end_trip();
        self.travel.refused = refused;
        self.travel.refused_recalls = refused_recalls;
        self.travel.refused_gems = refused_gems;
        self.travel.goal = goal;
        self.travel.owner = owner;
    }

    /// The route no longer starts where the character stands (a teleport
    /// or server correction): aim the next leg afresh.
    pub(crate) fn travel_displaced(&mut self) {
        self.travel.state.leg = None;
    }

    /// Plan the journey again from where the body stands, leaving out what it was refused, and set
    /// off; with no way left it is refused, and kept refused for whoever set it off.
    pub(crate) fn plan_again(&mut self, now: Instant) -> Did {
        let (Some(goal), Some(owner)) = (self.travel.goal, self.travel.owner) else {
            self.end_trip();
            return Did::blocked("no journey to plan again");
        };
        self.cancel_travel_keeping_refusals();
        let did = self.set_off(owner, goal, now);
        if let (Did::Blocked(why), Some(me), Some(p)) = (&did, self.my_position(), goal.place()) {
            self.refuse(owner, goal, me, p.at, &why.what);
            self.end_trip();
        }
        did
    }

    /// The step's walk has no way: a portal is left out and the journey planned again next frame;
    /// the journey's own end refuses the journey.
    fn refuse_step(&mut self, why: &str) {
        match self.travel_step().cloned() {
            Some(Step::Portal { name, mouth, .. }) => {
                tracing::warn!(
                    "travel: step {} (portal {name:?}): {why}; going another way",
                    self.travel.step
                );
                self.travel.refused.push(mouth.at.truncate());
                self.travel.replan_due = true;
            }
            _ => self.refuse_journey(why),
        }
    }

    /// No way to the journey's end from here: it ends, kept refused for whoever set it off.
    fn refuse_journey(&mut self, why: &str) {
        tracing::warn!(
            "travel: step {}: {why}; the journey is refused",
            self.travel.step
        );
        let (goal, owner, me) = (self.travel.goal, self.travel.owner, self.my_position());
        if let (Some(goal), Some(owner), Some(me)) = (goal, owner, me) {
            if let Some(p) = goal.place() {
                self.refuse(owner, goal, me, p.at, why);
            }
        }
        self.end_trip();
        self.travel.why_refused = Some(why.to_string());
    }

    /// What the slot answered the step's stretch: the walk's end reached is the next step; a leg
    /// refused is skipped; any other refusal leaves the step out; the step to open ground is over.
    pub(crate) fn stretch_answered(&mut self, did: &Did) {
        let gem = matches!(self.travel_step(), Some(Step::Gem { .. }));
        let portal = self.travel_portal().is_some();
        let leg = self.travel.state.on_leg;
        match did {
            Did::Done | Did::Blocked(_) if gem => self.travel.state.faced = true,
            Did::Done if !portal && !leg => {
                self.travel_next_step();
            }
            Did::Blocked(_) if leg => self.travel.leg_refused = true,
            Did::Blocked(why) => self.refuse_step(&why.what),
            _ => {}
        }
    }

    /// The stretch of the step under way to walk this frame (`walk_journey`): the step's whole walk
    /// when the region holds its end, else the overland leg. Before it, portal, recall and gem
    /// arrivals and clocks; `None` when the step walks nowhere this frame.
    pub(crate) fn step_goal(&mut self, now: Instant) -> Option<Goal> {
        self.travel.trip.as_ref()?;
        let (me3, block) = {
            let pl = self.player.as_ref()?;
            (pl.world_position(), pl.cell & 0xFFFF_0000)
        };
        let me = me3.truncate();
        self.travel.leg_refused = std::mem::take(&mut self.travel.state.on_leg)
            && std::mem::take(&mut self.travel.leg_refused);
        // A portal step is over when the character is where the portal
        // comes out. Merely being in another landblock is not enough:
        // walking to a portal often crosses a boundary, and counting
        // that as having gone through skipped the rest of the journey.
        if let Some(from) = self.travel.state.portal_from {
            let step = self.travel_step().cloned();
            let exit = step.as_ref().and_then(|s| s.exit());
            let arrived = match (exit, &step) {
                // A recall can land in the landblock we are already in,
                // so only the spot counts, and only once a cast is out.
                (Some(exit), Some(Step::Recall { .. })) => {
                    self.travel.state.recall_casts > 0 && me.distance(exit.at.truncate()) < 30.0
                }
                (Some(exit), _) => {
                    block == exit.cell & 0xFFFF_0000 || me.distance(exit.at.truncate()) < 30.0
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
                        if self.travel.state.recall_leave_combat {
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
                // The next step's walk is planned on the region arrived in, this frame.
                if !self.travel_next_step() {
                    return None;
                }
            } else if let Some(Step::Recall { spell, name, .. }) = step {
                self.travel_recall_wait(spell, &name, now);
                // Nothing to walk to: the character stands still for the cast.
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
        // A portal takes whoever touches it, so the character can be
        // carried off mid-walk by one the journey never meant to use.
        // Wherever they have landed, plan again from there.
        if let Some(last) = self.travel.last_seen {
            let jumped = me.distance(last) > 60.0;
            let expected = self
                .travel_step()
                .and_then(|s| s.exit())
                .is_some_and(|exit| me.distance(exit.at.truncate()) < 60.0);
            if jumped && !expected {
                self.travel.last_seen = Some(me);
                // Whatever swept us up can still be walked into, so it is
                // not refused: refusing it once shut the way off the
                // continent. The route already pays to pass near one, and
                // the plan from here takes the new position as it is.
                tracing::info!("travel: carried off to {me:?}; planning again from here");
                self.travel.replan_due = true;
                return None;
            }
        }
        self.travel.last_seen = Some(me);
        let step = self.travel_step()?.clone();
        let end = step.walk_end()?;
        let portal = matches!(step, Step::Portal { .. });
        if portal && reached(me3, end.at, ARRIVE) && !self.wait_at_mouth(now) {
            return None;
        }
        if self.is_in_region(end) {
            // A mouth is walked into on its own floor; the journey's end is put on its floor.
            let (place, within) = match portal {
                true => (end, 0.0),
                false => (
                    self.end_on_floor(end)?,
                    self.travel.goal.map_or(ARRIVE, Goal::within),
                ),
            };
            return Some(Goal::At { place, within });
        }
        self.travel.state.on_leg = true;
        self.overland_goal(end.at.truncate(), now)
    }

    /// Standing at the portal's mouth and still here: jump into it after a while, and after
    /// [`PORTAL_GIVE_UP`] leave it out. False once it is left out.
    fn wait_at_mouth(&mut self, now: Instant) -> bool {
        let since = *self.travel.state.portal_since.get_or_insert(now);
        if now.duration_since(since) > PORTAL_GIVE_UP {
            if let Some((name, mouth)) = self.travel_portal() {
                let level = self.world.stats.level.max(1) as u32;
                let why = ac_world::portals::near(mouth, 3.0)
                    .first()
                    .and_then(|p| p.refusal(level))
                    .unwrap_or_else(|| "would not take us".into());
                tracing::warn!("travel: the portal {name:?} {why}; going another way");
                self.travel.refused.push(mouth);
            }
            self.travel.replan_due = true;
            return false;
        }
        // A few portals sit above the ground and have to be jumped into; try that while we wait.
        let hop_due = self
            .travel
            .state
            .last_hop
            .is_none_or(|t| now.duration_since(t) > Duration::from_secs(2));
        if now.duration_since(since) > Duration::from_secs(3) && hop_due {
            self.travel.state.last_hop = Some(now);
            tracing::info!("travel: jumping into the portal");
            self.jump(1.0);
        }
        true
    }

    /// The recall step, standing still: the cast sent once the character has settled, again when
    /// the first has not worked in a while, and after that the journey goes another way.
    fn travel_recall_wait(&mut self, spell: u32, name: &str, now: Instant) {
        let since = *self.travel.state.recall_since.get_or_insert(now);
        let waited = now.duration_since(since);
        let casts = self.travel.state.recall_casts;
        let due = if casts == 0 {
            waited >= RECALL_SETTLE
        } else {
            waited >= RECALL_GIVE_UP
        };
        if !due {
            return;
        }
        if casts >= RECALL_TRIES {
            self.travel_recall_refused();
            return;
        }
        // `/lifestone` is not a spell: the server has its own action for it and asks for no
        // skill, components or casting at all. It is planned beside the spells and sent quite
        // differently.
        let sent = if ac_world::recalls::spell::is_free(spell) {
            self.send_free_recall(spell)
        } else {
            // On ourselves, whatever is selected: a few recalls are written as targeted spells.
            match self.world.player_guid {
                Some(me) => self.cast_at(spell, me),
                None => self.try_cast(spell),
            }
        };
        match sent {
            crate::magic::CastCheck::Ok => {
                tracing::info!("travel: casting {name} ({})", casts + 1);
                self.travel.state.recall_casts = casts + 1;
                self.travel.state.recall_since = Some(now);
            }
            why => {
                tracing::warn!("travel: cannot cast {name}: {why:?}");
                self.travel_recall_refused();
            }
        }
    }

    /// `end` on its floor by the planner's rule, kept for the step; `None` while its area is
    /// assembled. A place with no floor is handed on as it is, and the planner says why not.
    fn end_on_floor(&mut self, end: Place) -> Option<Place> {
        if let Some(p) = self.travel.state.end {
            return Some(p);
        }
        let pl = self.player.as_ref()?;
        let me = Place {
            at: pl.world_position(),
            cell: pl.cell,
        };
        let cap = pl.capsule();
        let on = match self.pathfinder.put_on_floor(me, end, cap) {
            Ok(Some(p)) => p,
            Ok(None) => return None,
            Err(_) => end,
        };
        self.travel.state.end = Some(on);
        Some(on)
    }

    /// The overland leg toward `target` (world xy) past the region: along a terrain route
    /// (`worldroute`), a waypoint skipped when no progress is made toward it.
    fn overland_goal(&mut self, target: Vec2, now: Instant) -> Option<Goal> {
        let (me3, block, indoors) = {
            let pl = self.player.as_ref()?;
            (pl.world_position(), pl.cell & 0xFFFF_0000, pl.is_indoors())
        };
        let me = me3.truncate();
        let underground = indoors && {
            let assets = self.assets.clone();
            self.player
                .as_mut()
                .is_some_and(|pl| pl.in_dungeon(&assets))
        };
        // Underground there is no terrain to walk: only a portal leaves a dungeon.
        if underground {
            self.refuse_step("not in this dungeon");
            return None;
        }
        if self.travel.state.route.is_none() {
            let (grid, region) = self.travel_terrain()?;
            // Every other portal near the way is a trap: walking into one takes the character
            // wherever it leads. Give them a wide berth.
            let keep = self.travel_portal().map(|(_, m)| m);
            let mid = (me + target) * 0.5;
            let reach = me.distance(target) * 0.5 + 200.0;
            let avoid: Vec<Vec2> = ac_world::portals::near(mid, reach)
                .into_iter()
                .filter(|p| p.mouth_outdoors())
                .map(|p| p.from_xy())
                .filter(|m| keep.is_none_or(|k| k.distance(*m) > 5.0))
                .collect();
            let Some(route) = worldroute::find_avoiding(&grid, &region, me, target, &avoid) else {
                self.refuse_step("no way there over the terrain");
                return None;
            };
            let len: f32 = route.windows(2).map(|w| w[0].distance(w[1])).sum();
            tracing::info!(
                "travel: step {}: {} waypoints, {len:.0} m",
                self.travel.step,
                route.len()
            );
            self.travel.state.route = Some(route);
            self.travel.state.next = 0;
            self.travel.state.restart_waypoint();
        }
        let route = self.travel.state.route.clone()?;
        let n = route.len();
        loop {
            let mut next = self.travel.state.next;
            while next < n && me.distance(route[next]) <= ARRIVE {
                next += 1;
            }
            if next != self.travel.state.next {
                self.travel.state.next = next;
                self.travel.state.restart_waypoint();
            }
            if next >= n {
                // Walked to the route's end and the region still does not hold the step's.
                self.refuse_step("the terrain route ends short of it");
                return None;
            }
            let wp = route[next];
            // How far there is still to go: along the route being steered
            // (and from its end on to the waypoint) while there is one,
            // as the crow flies otherwise.
            let (d, plan) = match self.steering.remaining(me3) {
                Some((along, end, planned)) => (along + end.truncate().distance(wp), Some(planned)),
                None => (me.distance(wp), None),
            };
            // A different route is a different yardstick: progress is
            // measured from here on, but the clock keeps running, so a
            // character that is not moving is still caught.
            let s = &mut self.travel.state;
            if plan != s.measured_on {
                s.measured_on = plan;
                s.best = d;
                s.last_progress.get_or_insert(now);
            }
            match s.last_progress {
                Some(t) if d >= s.best - PROGRESS => {
                    if now.duration_since(t) >= STUCK_AFTER || self.travel.leg_refused {
                        // The last waypoint is where the step ends; not reaching it leaves the
                        // step out.
                        if next + 1 >= n {
                            self.refuse_step("no way to the end of the terrain route");
                            return None;
                        }
                        tracing::warn!(
                            "travel: no progress toward waypoint {next}/{n} at {wp:?} for {:?}, skipping it",
                            STUCK_AFTER
                        );
                        self.travel.leg_refused = false;
                        self.travel.state.next = next + 1;
                        self.travel.state.restart_waypoint();
                        continue;
                    }
                }
                _ => {
                    s.best = d;
                    s.last_progress = Some(now);
                }
            }
            let leg = match self.travel.state.leg {
                Some(l) if me.distance(l) > ARRIVE && me.distance(l) <= 2.0 * LEG => l,
                _ => {
                    let l = leg_end(me, wp);
                    self.travel.state.leg = Some(l);
                    l
                }
            };
            let z = leg_height(self.travel.grid.as_deref(), leg, me3, false);
            let place = Place {
                at: leg.extend(z),
                cell: steering_block(leg, block, false),
            };
            return Some(Goal::At {
                place,
                within: ARRIVE,
            });
        }
    }
}

/// The cell a journey is planned from, for a character in `cell` at world
/// position `at`.
///
/// To the planner an indoor cell is a place left only by portal -- right
/// for the Town Network hub and a dungeon, wrong for a building on the
/// surface, whose door can simply be walked out of. From upstairs beside
/// the Holtburg Archmage that sent a trip to a dungeon three hundred
/// metres away through two portals and the hub. A building is planned
/// from the outdoor cell under the character; underground keeps its own.
pub(crate) fn planning_cell(cell: u32, at: Vec3, underground: bool) -> u32 {
    if underground || cell & 0xFFFF < 0x100 {
        return cell;
    }
    ac_world::outdoor_cell(cell, at - ac_world::landblock_origin(cell))
}

/// The landblock the steering is told a leg ending at `at` lies in, for
/// a character standing in `cell`.
///
/// On the surface a position names its landblock. Underground it need
/// not: a dungeon's rooms reach outside its landblock's square -- the
/// Holtburg Dungeon's lie at negative local y -- and there a position
/// names the block next door. Told that a corpse in those rooms lay in
/// 0x01F5, the steering had no graph to plan the way through them on,
/// ran at the walls of the room the dungeon's portal drops you in, and
/// recovery gave up sixty metres from the corpse. Only a portal leaves a
/// dungeon, so every walk in one ends in the block the character stands
/// in.
fn steering_block(at: Vec2, cell: u32, underground: bool) -> u32 {
    if underground {
        cell & 0xFFFF_0000
    } else {
        WorldGrid::block_of(at)
    }
}

/// The height a leg ending at `at` is aimed at, for a character at `me`:
/// the ground there by the world grid, or underground the character's own
/// floor -- the grid knows only the surface, and off the side of a
/// dungeon's square only the surface of the block next door.
fn leg_height(grid: Option<&WorldGrid>, at: Vec2, me: Vec3, underground: bool) -> f32 {
    match grid {
        _ if underground => me.z,
        Some(g) => g.height_at(at),
        None => 0.0,
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

#[cfg(test)]
mod tests;
