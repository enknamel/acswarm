//! Journeys: a trip (`ac_world::trip`) of walks, portals, recalls and gems, set off on by
//! `Client::set_off`. Each step's walk is planned whole by the region planner before a step:
//! to its end when the region holds it, else a stretch along an overland corridor
//! (`overland`), walked by the slot (`Client::walk_journey`). A step with no walk is left out and
//! the journey planned again; the journey's own end with none refuses the journey.

use std::rc::Rc;
use std::time::{Duration, Instant};

use ac_formats::region::Region;
use ac_scene::worldgrid::WorldGrid;
use ac_world::trip::{self, NoWalk, Prefs, Step, Trip};
use ac_world::Place;
use glam::{Vec2, Vec3};

use crate::body::server_walk::reached;
use crate::did::Did;
use crate::gem_room::{clear_heading, gem_next, GemNext};
use crate::{Client, Goal, Owner};
use overland::{Corridor, EXIT_REACH};

mod flown;
mod mouth;
mod overland;
mod reaching;
mod turned_away;

/// A map point, or a portal's mouth stood at, is reached within this (metres, 3D).
pub const ARRIVE: f32 = 3.0;
/// A portal that has not taken us in this long is one we cannot use
/// (a level range, an unfinished quest, or it simply is not there): the
/// journey is planned again without it.
pub const PORTAL_GIVE_UP: Duration = Duration::from_secs(12);
/// Weenie error 0x04A7, "You have been teleported too recently" (`Portal.cs:155`).
const TELEPORTED_TOO_RECENTLY: u32 = 0x04A7;
/// Wait before walking back into a portal that said so: ACE's 3.5 s between portals
/// (`Portal.cs:120`), within which it turns a second try away without a word (`Portal.cs:145-150`).
const PORTAL_COOLDOWN: Duration = Duration::from_secs(4);
/// The step back off the mouth meanwhile (metres): a portal takes whoever walks into it, not
/// whoever stands in it.
const PORTAL_STEP_BACK: f32 = 3.0;
/// The spots of a portal step's walk kept for the step back (at most), a metre apart.
const TRAIL_KEEP: usize = 8;
/// A recall step waits this long after the step begins before casting,
/// so the character has come to a stop (moving disrupts the cast).
const RECALL_SETTLE: Duration = Duration::from_millis(800);

/// Metres a recall cast must have moved us to count as carried off, wherever it landed.
const RECALL_CARRIES: f32 = 60.0;
/// A recall that has not carried us off this long after the cast was
/// sent (the cast takes a few seconds and the server pauses two more
/// before the teleport) is cast again, and after `RECALL_TRIES` casts
/// the journey is planned again without that spell.
pub const RECALL_GIVE_UP: Duration = Duration::from_secs(15);
/// The same for `/lifestone`, whose teleport waits out the LifestoneRecall animation
/// (Player_Location.cs:180), 15.2 s on Coldeve: sent again at 15 s, a second recall began.
const LIFESTONE_GIVE_UP: Duration = Duration::from_secs(25);
const RECALL_TRIES: u32 = 2;
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
    /// Turned away as teleported too recently: when to walk back in, and where to stand till then.
    too_soon: Option<(Instant, Place)>,
    /// Where the portal step's walk stood, a metre apart, newest last: floor the step back holds.
    trail: Vec<Place>,
    /// When the recall step's spell was last sent (or the step began), and how many times.
    recall_since: Option<Instant>,
    recall_casts: u32,
    /// The character was in peace mode before the cast: back to it once the recall has landed.
    recall_leave_combat: bool,
    /// When the gem step's gem was last used, and how many times.
    gem_used: Option<Instant>,
    gem_uses: u32,
    /// The gem's stack when first used: one fewer is a use taken, the rest still carried.
    gem_stack: u32,
    /// The open ground walked to before the gem is used, and whether that walk is over.
    gem_spot: Option<Place>,
    faced: bool,
    /// The portals in reach as the gem was used: none of them is the one it summons.
    portals_before: Vec<u32>,
    /// When the portal the gem summoned was last used, and how many times.
    summoned_used: Option<Instant>,
    summoned_uses: u32,
    /// The step's walk end on its floor, once the region holds it.
    end: Option<Place>,
    /// The overland corridor, while the region does not hold the walk end.
    corridor: Option<Corridor>,
    /// The corridor stretch being walked: where it leaves the region, on its floor.
    stretch: Option<Place>,
}

/// The journey being made, and the terrain data it needs.
#[derive(Default)]
pub struct Travel {
    grid: Option<Rc<WorldGrid>>,
    /// Indoor cells by whether they are a dungeon's rooms (`Client::is_dungeon_cell`).
    dungeon_cells: std::collections::HashMap<u32, bool>,
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
    /// Walks the region planner found no way for *on this journey*, like `refused_recalls`.
    refused_walks: Vec<NoWalk>,
    /// This journey has been carried into another region (a portal, recall or gem): a walk with
    /// no way there is left out and the journey planned again, not refused.
    hopped: bool,
    /// The quest flags the character holds that a portal asks for (`crate::quest_flags`).
    pub(crate) quests_done: Vec<String>,
    /// The last tie spell cast (Lifestone Tie, a Portal Tie), so the
    /// "successfully linked" that follows is filed under its position.
    pub(crate) last_tie: Option<u32>,
    /// A recall asked for that goes to a saved position: that position's kind, where we stood and
    /// when. Where it lands is learnt as the position (see `Client::recall_landed`).
    pub(crate) recall_sent: Option<(u32, Vec3, Instant)>,
    /// The destination (world xy) a journey flies its stretches to: no way left on foot (`flown`).
    fly_to: Option<Vec2>,
    /// When travel last cast a recall: it lands whatever journey follows (`RECALL_LANDS`).
    pub(crate) recall_cast: Option<Instant>,
    /// Where the journey is bound (always `Goal::At`), to replan around a refusal.
    goal: Option<Goal>,
    /// Whose journey it is: the STEPS row that set it off or took it over, whose stretches the slot
    /// walks (`Client::journey_owner`); a journey refused is kept refused for it (`Client.refusals`).
    owner: Option<Owner>,
    /// A step was left out: the next frame plans the journey again from where the body stands.
    pub(crate) replan_due: bool,
    /// Another row acted last tick: the stretch is let go and the body stands, trip and step kept.
    held: bool,
    /// Its row stood aside or was passed over this tick: unless taken over, put down at its end.
    row_aside: bool,
    /// Why the last journey was refused, for [`Client::set_off`] to answer with.
    why_refused: Option<String>,
    /// The last journey was broken off by something else the character went to do -- a corpse,
    /// a fight, a dodge -- or put down as its row stood aside, rather than arriving, giving up or
    /// being cancelled. Cleared when the next one sets off (see [`Client::journey_broken_off`]).
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
    pub(crate) fn travel_terrain(&mut self) -> Option<(Rc<WorldGrid>, Rc<Region>)> {
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

    /// [`journey_to`](Self::journey_to) a goal: a place and how near, set off for the running row;
    /// the journey under way to it is taken over, not planned again.
    fn journey_to_goal(&mut self, goal: Goal) -> bool {
        self.drop_visit("travelling somewhere else");
        if self.keep_journey_to(goal) {
            return true;
        }
        let owner = self.step_owner();
        // The frame's clock, or the wall's when no frame has run since it was set.
        let now = self.frame_at.max(Instant::now());
        self.set_off_or_fly(owner, goal, now).fine()
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
        let Some((from, underground)) = self.planning_from() else {
            tracing::warn!("travel: the character is not in the world");
            return Err("not in the world yet".into());
        };
        let (me, cell) = (from.at, from.cell);
        let t0 = Instant::now();
        // A new destination starts afresh: what turned us away on the
        // last journey says nothing about this one.
        if self.travel_goal_xy().is_none_or(|g| g.distance(xy) > 1.0) {
            self.travel.refused.clear();
            self.travel.refused_recalls.clear();
            self.travel.refused_gems.clear();
            self.travel.refused_walks.clear();
            self.travel.hopped = false;
            self.travel.fly_to = self.travel.fly_to.filter(|f| f.distance(xy) <= 1.0);
            // A new journey is a road until whoever planned it says
            // otherwise (see `travel_about`).
            self.travel.about_the_ground = false;
        }
        let level = self.world.stats.level.max(1) as u32;
        tracing::info!(
            "travel: planning to {xy:?} in {:#010x} from {cell:#010x}, underground {underground}",
            place.cell
        );
        let planned = self.plan_trip(place, true);
        let Some(mut trip) = planned else {
            // Say why. Often there is a way, but not one this character
            // may take yet: every chain to the far continents runs
            // through a portal with a level on it.
            let to = self.planning_place(place);
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

    /// Where the body stands as the trip planner is told it (see [`planning_cell`]), and whether
    /// underground: a dungeon shares its landblock number with the ground above, and without saying
    /// so a vendor a hundred metres up reads as a hundred-metre stroll into the nearest wall.
    fn planning_from(&mut self) -> Option<(Place, bool)> {
        let assets = self.assets.clone();
        let pl = self.player.as_mut()?;
        let at = pl.world_position();
        let underground = pl.is_indoors() && pl.in_dungeon(&assets);
        let cell = planning_cell(pl.cell, at, underground);
        Some((Place { at, cell }, underground))
    }

    /// Take `trip` to `goal`, set off by `owner`, as the journey under way, at its first step.
    pub(crate) fn start_trip(&mut self, owner: Owner, trip: Trip, goal: Goal) {
        self.travel.trip = Some(trip);
        self.travel.step = 0;
        self.travel.goal = Some(goal);
        self.travel.owner = Some(owner);
        self.travel.broken_off = false;
        self.travel.replan_due = false;
        self.travel.held = false;
        self.travel.row_aside = false;
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
        // The far side's graph is built while the character walks to the portal or casts.
        if let (Some(exit), Some(capsule)) =
            (step.exit(), self.player.as_ref().map(|p| p.capsule()))
        {
            let next = self
                .travel
                .trip
                .as_ref()
                .and_then(|t| t.steps.get(n + 1))
                .and_then(Step::walk_end)
                .unwrap_or(exit);
            self.pathfinder.prepare(exit, next, capsule);
        }
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
    /// Whether the journey's step under way is a portal's or a gem's: a teleport now is theirs.
    pub(crate) fn is_portal_step(&self) -> bool {
        self.traveling()
            && matches!(
                self.travel_step(),
                Some(Step::Portal { .. } | Step::Gem { .. })
            )
    }

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
        self.travel.state.corridor.as_ref().map(Corridor::points)
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

    /// The saved prefs, for the character as it stands now: PK status and vitae.
    pub fn travel_prefs(&self) -> Prefs {
        self.travel.prefs.for_traveller(self.traveller())
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
        // Another row's journey is held while this row acts (`settle_journey`), not broken off.
        let theirs = self
            .autoplay
            .running
            .is_some_and(|r| self.journey_row().is_some_and(|j| j != r));
        if self.traveling() && !theirs {
            tracing::info!("travel: stopped, {what}");
            self.end_trip();
            self.travel.broken_off = true;
        }
    }

    /// Whether the character is between journeys because the last one was broken off by something
    /// else it went to do (a use, a fight, a corpse) or put down as its row stood aside: whatever
    /// set it off plans it again once free. One that arrived, gave up or was cancelled answers no.
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

    /// The STEPS row whose journey is under way; `None` for one no row owns (a script's, a visit's).
    pub(crate) fn journey_row(&self) -> Option<&'static str> {
        match self.travel.owner.filter(|_| self.traveling())? {
            Owner::Step(row) => Some(row),
            Owner::Journey | Owner::Visit | Owner::Script | Owner::Hand => None,
        }
    }

    /// Who the journey's stretches are walked for: its row, else `Owner::Journey`.
    pub(crate) fn journey_owner(&self) -> Owner {
        self.journey_row().map_or(Owner::Journey, Owner::Step)
    }

    /// Whether the journey under way is the running row's (outside any row, one of no row).
    pub(crate) fn is_own_journey(&self) -> bool {
        self.traveling() && self.journey_row() == self.autoplay.running
    }

    /// Take the journey under way for the running row: its stretches are walked for that row.
    pub(crate) fn keep_journey(&mut self) {
        self.take_journey(self.step_owner());
    }

    /// Take the journey under way for `owner`, if one is, saying so when its row changes.
    pub(crate) fn take_journey(&mut self, owner: Owner) {
        if !self.traveling() {
            return;
        }
        let was = self.journey_owner();
        self.travel.owner = Some(owner);
        self.travel.row_aside = false;
        if self.journey_owner() != was {
            tracing::info!(
                "travel: {} takes the journey from {}",
                self.journey_owner().label(),
                was.label()
            );
        }
    }

    /// `row` stood aside or was passed over this tick: its journey is put down unless taken over.
    pub(crate) fn journey_row_aside(&mut self, row: &'static str) {
        if self.journey_row() == Some(row) {
            self.travel.row_aside = true;
        }
    }

    /// End of a tick (`settle_going`): a row's journey walks on while its row acts, is held (its
    /// stretch let go, the body standing) while another acts, else is put down for its row.
    pub(crate) fn settle_journey(&mut self) {
        let aside = std::mem::take(&mut self.travel.row_aside);
        let ticked = self.autoplay.config.enabled && self.world.player_guid.is_some();
        let Some(row) = self.journey_row().filter(|_| ticked) else {
            self.travel.held = false;
            return;
        };
        if self.autoplay.step == Some(row) {
            if std::mem::take(&mut self.travel.held) {
                tracing::info!("travel: {row} acts again; its journey walks on");
            }
            return;
        }
        // Put down, not refused: its row reads a journey broken off and walks it on when it acts.
        let Some(acting) = self.autoplay.step.filter(|_| !aside) else {
            tracing::info!("travel: {row} stood aside; its journey is put down");
            self.end_trip();
            self.travel.broken_off = true;
            return;
        };
        if !self.travel.held {
            tracing::info!("travel: {row}'s journey held while {acting} acts");
        }
        self.travel.held = true;
        // Standing at a mouth while held is not the portal refusing us.
        self.travel.state.portal_since = None;
        self.stop_stretch("held");
    }

    /// Whether the journey under way is held while a row other than its own acts.
    pub(crate) fn is_journey_held(&self) -> bool {
        self.traveling() && self.travel.held
    }

    /// Whether a journey to `goal` is under way, taken over for the running row if so.
    pub(crate) fn keep_journey_to(&mut self, goal: Goal) -> bool {
        let bound = self.traveling() && self.travel.goal.is_some_and(|g| g.is_same(goal));
        if bound {
            self.keep_journey();
        }
        bound
    }

    /// The place the trip planner is told for `goal`: a building's room is planned as the
    /// ground outside it, whose door is walked through; a dungeon's keeps its cell.
    pub(crate) fn planning_place(&mut self, goal: Place) -> Place {
        if !goal.is_indoors() {
            return goal;
        }
        if self.is_dungeon_cell(goal.cell) {
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
        self.travel.refused_walks.clear();
        self.travel.hopped = false;
        self.travel.goal = None;
        self.travel.owner = None;
        self.travel.replan_due = false;
        self.travel.held = false;
        self.travel.row_aside = false;
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
            self.summoned_portal(&self.travel.state.portals_before)
        } else {
            None
        };
        let since_portal = self
            .travel
            .state
            .summoned_used
            .map(|t| now.duration_since(t));
        // A stack of gems stays carried once one is spent: "not taken" is the stack unchanged.
        let untaken = self.gem_stack(guid) >= self.travel.state.gem_stack.max(1);
        match gem_next(
            untaken,
            summons,
            since_use,
            self.travel.state.gem_uses,
            portal,
            since_portal,
            self.travel.state.summoned_uses,
        ) {
            GemNext::Wait => {}
            GemNext::UseGem if self.is_gem_held(now) => {}
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
        if self.is_gem_held(now) {
            // A spell is still on its way, and the use would be turned
            // away with the gem kept: wait for the slot.
            return None;
        }
        tracing::info!("travel: using {name}");
        self.travel.state.portals_before = self.portals_in_reach();
        self.travel.state.gem_spot = None;
        self.travel.state.gem_used = Some(now);
        self.travel.state.gem_uses = 1;
        self.travel.state.gem_stack = self.gem_stack(guid);
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

    /// The recall being cast fizzled: cast again as soon as the
    /// character has settled rather than wait out the whole timeout.
    pub(crate) fn travel_recall_fizzled(&mut self) {
        if self.travel_recall_spell().is_some() && self.travel.state.recall_casts > 0 {
            self.travel.state.recall_since = Some(
                Instant::now()
                    .checked_sub(LIFESTONE_GIVE_UP)
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
        let refused_walks = std::mem::take(&mut self.travel.refused_walks);
        let (goal, owner, hopped) = (self.travel.goal, self.travel.owner, self.travel.hopped);
        self.end_trip();
        self.travel.refused = refused;
        self.travel.refused_recalls = refused_recalls;
        self.travel.refused_gems = refused_gems;
        self.travel.refused_walks = refused_walks;
        self.travel.hopped = hopped;
        self.travel.goal = goal;
        self.travel.owner = owner;
    }

    /// Plan the journey again from where the body stands, leaving out what it was refused, and set
    /// off; with no way left it is refused, and kept refused for whoever set it off.
    pub(crate) fn plan_again(&mut self, now: Instant) -> Did {
        let (Some(goal), Some(owner)) = (self.travel.goal, self.travel.owner) else {
            self.end_trip();
            return Did::blocked("no journey to plan again");
        };
        let hopping = self.is_hopping();
        self.cancel_travel_keeping_refusals();
        let mut did = self.set_off(owner, goal, now);
        if matches!(did, Did::Blocked(_)) && hopping {
            did = self.fly_journey(owner, goal, now);
        }
        if let (Did::Blocked(why), Some(me), Some(p)) = (&did, self.my_position(), goal.place()) {
            self.refuse(owner, goal, me, p.at, &why.what);
            self.end_trip();
        }
        did
    }

    /// The step's walk has no way: a portal is left out and the journey planned again next frame,
    /// and so is the walk to its end once carried into this region; else the journey is refused.
    fn refuse_step(&mut self, why: &str) {
        let n = self.travel.step;
        match self.travel_step().cloned() {
            Some(Step::Portal { name, mouth, .. }) => {
                tracing::warn!("travel: step {n} (portal {name:?}): {why}; going another way");
                self.travel.refused.push(mouth.at.truncate());
                self.travel.replan_due = true;
            }
            // An end no floor of its cell holds is the goal's fault, not the way's: no plan helps.
            Some(Step::Walk(to)) if self.is_refused_off_floor(to.at) => self.refuse_journey(why),
            Some(Step::Walk(to)) if self.travel.hopped => {
                let Some((from, _)) = self.planning_from() else {
                    return self.refuse_journey(why);
                };
                tracing::warn!(
                    "travel: step {n} (walk to {:?}): {why}; going another way",
                    to.at
                );
                let to = self.planning_place(to);
                self.travel.refused_walks.push(NoWalk { from, to });
                // And from where the hop came out, where a plan's walk there starts: refused only
                // where it stood, Portal Recall and the same walk were planned again for ever
                // (a_walk_refused_after_a_recall_is_not_planned_again_from_its_landing).
                let landed = n.checked_sub(1).and_then(|i| {
                    let trip = self.travel.trip.as_ref()?;
                    trip.steps.get(i).and_then(Step::exit)
                });
                if let Some(landed) = landed {
                    let from = self.planning_place(landed);
                    self.travel.refused_walks.push(NoWalk { from, to });
                }
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

    /// What the slot answered the step's stretch: the walk's end reached is the next step, a
    /// corridor stretch's the next stretch (true: asked on this frame); a corridor stretch refused
    /// is kept clear of; any other refusal leaves the step out; the step to open ground is over.
    pub(crate) fn stretch_answered(&mut self, did: &Did) -> bool {
        // The step back off a portal that said "too recently" is no answer about the way.
        if self.travel.state.too_soon.is_some() {
            return false;
        }
        let gem = matches!(self.travel_step(), Some(Step::Gem { .. }));
        let portal = self.travel_portal().is_some();
        let stretch = self.travel.state.stretch;
        match (did, stretch) {
            (Did::Done | Did::Blocked(_), _) if gem => self.travel.state.faced = true,
            (Did::Done, Some(_)) => {
                self.travel.state.stretch = None;
                return true;
            }
            (Did::Done, None) if !portal => {
                self.travel_next_step();
            }
            (Did::Blocked(why), Some(exit)) => {
                tracing::info!(
                    "travel: step {}: no way to {:?} on the terrain route ({}); round it",
                    self.travel.step,
                    exit.at,
                    why.what
                );
                self.travel.state.stretch = None;
                if let Some(c) = self.travel.state.corridor.as_mut() {
                    c.refuse(exit.at.truncate());
                }
            }
            // Stood in the mouth, the wait there (`wait_at_mouth`) gives up on the portal, not a stall.
            (Did::Blocked(_), None) if portal && self.travel.state.portal_since.is_some() => {}
            (Did::Blocked(why), None) => self.refuse_step(&why.what),
            _ => {}
        }
        false
    }

    /// The stretch of the step under way to walk this frame (`walk_journey`): the step's whole walk
    /// when the region holds its end, else the overland leg. Before it, portal, recall and gem
    /// arrivals and clocks; `None` when the step walks nowhere this frame.
    pub(crate) fn step_goal(&mut self, now: Instant) -> Option<Goal> {
        self.travel.trip.as_ref()?;
        let (me3, block, cell) = {
            let pl = self.player.as_ref()?;
            (pl.world_position(), pl.cell & 0xFFFF_0000, pl.cell)
        };
        let me = me3.truncate();
        let trail = &mut self.travel.state.trail;
        if trail.last().is_none_or(|p| p.at.distance(me3) >= 1.0) {
            trail.push(Place { at: me3, cell });
            if trail.len() > TRAIL_KEEP {
                trail.remove(0);
            }
        }
        // A portal step is over when the character is where the portal
        // comes out. Merely being in another landblock is not enough:
        // walking to a portal often crosses a boundary, and counting
        // that as having gone through skipped the rest of the journey.
        if let Some(from) = self.travel.state.portal_from {
            let step = self.travel_step().cloned();
            let exit = step.as_ref().and_then(|s| s.exit());
            let arrived = match (exit, &step) {
                // A recall can land in the landblock we are already in, so the spot counts, or
                // any jump once a cast is out: the saved position it was planned by can be stale.
                (Some(exit), Some(Step::Recall { .. })) => {
                    let jumped = self
                        .travel
                        .last_seen
                        .is_some_and(|l| me.distance(l) > RECALL_CARRIES);
                    self.travel.state.recall_casts > 0
                        && (me.distance(exit.at.truncate()) < 30.0 || jumped)
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
                self.travel.hopped = true;
                match step {
                    Some(Step::Recall { name, .. }) => {
                        tracing::info!("travel: {name} carried us off");
                        if self.travel.state.recall_leave_combat {
                            self.leave_combat();
                        }
                        // Somewhere else than planned (`recall_landed` learns where): the rest of
                        // the journey is planned again from here, not recast for 26 s.
                        if exit.is_some_and(|e| me.distance(e.at.truncate()) >= 30.0) {
                            tracing::info!(
                                "travel: {name} landed at {me:?}; planning again from here"
                            );
                            self.travel.replan_due = true;
                            return None;
                        }
                    }
                    _ => {
                        tracing::info!("travel: through the portal");
                        if let Some((_, mouth)) = self.travel_portal() {
                            self.travel.refused.retain(|r| r.distance(mouth) > 2.0);
                            // Portal Recall goes back to where the last recallable
                            // portal led: this one, unless it is marked NoRecall.
                            if let Some(p) = ac_world::portals::near(mouth, 3.0)
                                .first()
                                .filter(|p| p.is_recallable())
                            {
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
                self.travel.hopped = true;
                self.travel.replan_due = true;
                return None;
            }
        }
        self.travel.last_seen = Some(me);
        let step = self.travel_step()?.clone();
        let end = step.walk_end()?;
        let portal = matches!(step, Step::Portal { .. });
        if let Some((until, back)) = self.travel.state.too_soon.filter(|_| portal) {
            if now < until {
                return Some(Goal::At {
                    place: back,
                    within: 0.5,
                });
            }
            self.travel.state.too_soon = None;
            self.travel.state.portal_since = None;
            self.walk_afresh();
        }
        if portal && reached(me3, end.at, ARRIVE) && !self.wait_at_mouth(now) {
            return None;
        }
        if self.is_in_region(end) {
            self.travel.state.stretch = None;
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
        if self.is_journey_flown() {
            return Some(Goal::At {
                place: end,
                within: ARRIVE,
            });
        }
        self.overland_goal(end.at.truncate())
    }

    /// The recall step, standing still: the cast sent once the character has settled, again when
    /// the first has not worked in a while, and after that the journey goes another way.
    fn travel_recall_wait(&mut self, spell: u32, name: &str, now: Instant) {
        let since = *self.travel.state.recall_since.get_or_insert(now);
        let waited = now.duration_since(since);
        let casts = self.travel.state.recall_casts;
        let give_up = if ac_world::recalls::spell::is_free(spell) {
            LIFESTONE_GIVE_UP
        } else {
            RECALL_GIVE_UP
        };
        let due = if casts == 0 {
            waited >= RECALL_SETTLE
        } else {
            waited >= give_up
        };
        if !due {
            return;
        }
        if casts >= RECALL_TRIES {
            self.travel_recall_refused();
            return;
        }
        // Sent behind a spell still in the air it is turned away busy, and the try waits out the
        // give-up clock (25 s for `/lifestone`).
        if self.autoplay.cast_in_flight(now) {
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
                self.travel.recall_cast = Some(now);
            }
            why => {
                tracing::warn!("travel: cannot cast {name}: {why:?}");
                self.travel_recall_refused();
            }
        }
    }

    /// `end` on its floor, kept for the step (see [`Self::on_floor`]).
    fn end_on_floor(&mut self, end: Place) -> Option<Place> {
        if let Some(p) = self.travel.state.end {
            return Some(p);
        }
        let on = self.on_floor(end)?;
        self.travel.state.end = Some(on);
        Some(on)
    }

    /// `place` on its floor by the planner's rule; `None` while its area is assembled. A place
    /// with no floor is handed on as it is, and the planner says why not when walked to.
    fn on_floor(&mut self, place: Place) -> Option<Place> {
        let pl = self.player.as_ref()?;
        let me = Place {
            at: pl.world_position(),
            cell: pl.cell,
        };
        let cap = pl.capsule();
        match self.pathfinder.put_on_floor(me, place, cap) {
            Ok(Some(p)) => Some(p),
            Ok(None) => None,
            Err(_) => Some(place),
        }
    }

    /// The corridor stretch toward `target` (world xy) past the region: to where the corridor
    /// leaves the blocks round the body, on its floor; planned again off the corridor or round a
    /// stretch refused, and the step left out when the terrain has no way or only a refused one.
    fn overland_goal(&mut self, target: Vec2) -> Option<Goal> {
        let (me, indoors) = {
            let pl = self.player.as_ref()?;
            (pl.world_position().truncate(), pl.is_indoors())
        };
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
        if let Some(place) = self.travel.state.stretch.filter(|p| self.is_in_region(*p)) {
            let within = EXIT_REACH;
            return Some(Goal::At { place, within });
        }
        let corridor = self.travel.state.corridor.take();
        let exit = corridor.as_ref().and_then(|c| c.exit_from(me));
        let (corridor, exit) = match (corridor, exit) {
            (Some(c), Some(exit)) => (c, exit),
            (old, _) => {
                let refused = old.map(|c| c.refused()).unwrap_or_default();
                // No terrain to plan over (no game data): the step stands, refused by nothing.
                let (grid, region) = self.travel_terrain()?;
                let Some(c) = self.plan_corridor(&grid, &region, (me, target), refused) else {
                    self.refuse_step("no way there over the terrain");
                    return None;
                };
                let Some(exit) = c.exit_from(me) else {
                    self.refuse_step("the terrain route leaves the region at once");
                    return None;
                };
                (c, exit)
            }
        };
        let refused = corridor.is_refused_near(exit);
        self.travel.state.corridor = Some(corridor);
        if refused {
            self.refuse_step("no way along the terrain route");
            return None;
        }
        let place = self.on_floor(Place::on_map(exit))?;
        self.travel.state.stretch = Some(place);
        let within = EXIT_REACH;
        Some(Goal::At { place, within })
    }

    /// The step's corridor from `me` to `target`: off every outdoor portal mouth but the step's
    /// own (walking into one takes the character wherever it leads), and the `refused` ends.
    fn plan_corridor(
        &self,
        grid: &WorldGrid,
        region: &Region,
        (me, target): (Vec2, Vec2),
        refused: Vec<Vec2>,
    ) -> Option<Corridor> {
        let keep = self.travel_portal().map(|(_, m)| m);
        let mid = (me + target) * 0.5;
        let reach = me.distance(target) * 0.5 + 200.0;
        let avoid: Vec<Vec2> = ac_world::portals::near(mid, reach)
            .into_iter()
            .filter(|p| p.mouth_outdoors())
            .map(|p| p.from_xy())
            .filter(|m| keep.is_none_or(|k| k.distance(*m) > 5.0))
            .collect();
        let c = Corridor::plan(grid, region, (me, target), &avoid, refused)?;
        let len: f32 = c.points().windows(2).map(|w| w[0].distance(w[1])).sum();
        tracing::info!(
            "travel: step {}: a corridor of {} points, {len:.0} m",
            self.travel.step,
            c.points().len()
        );
        Some(c)
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

#[cfg(test)]
mod tests;
