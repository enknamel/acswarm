//! Region graphs built off the frame thread and planned on from it. One builder thread per process
//! assembles areas ([`ac_scene::navarea::Area`]) and builds their chunks; a session plans on the
//! shared areas at once ([`Pathfinder::plan`]), what is unbuilt answering [`Planned::Building`]
//! and queued, or has the builder plan the steering's walk ([`Pathfinder::ask`]). Idle, the
//! builder finishes the area used last, so its components become final.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use ac_nav::routing::{self, NoWay, Planned};
use ac_scene::collision::Capsule;
use ac_scene::navarea::{self, Area};
use ac_scene::{Assets, SharedArchives};
use ac_world::Place;
use glam::Vec3;

/// Send the builder a job no more often than this per handle (a plan on what is built is not held
/// back): a walk asked every frame would otherwise queue a job every frame.
const ASK_EVERY: Duration = Duration::from_millis(1500);
/// An answer planned from further than this (metres) from where we now stand is too old to follow.
const ANSWER_REACH: f32 = 15.0;
/// An answer to a goal this far (metres) from the one we now want is not ours.
const ANSWER_GOAL: f32 = 3.0;
/// Areas the process keeps, most recently used last; each is nine blocks of collision, about 15 MB.
const AREAS: usize = 4;
/// Jobs a handle that builds for itself runs for one answer before it says `Building`.
const OWN_TRIES: usize = 4;

/// An area in the registry, with what says which walks it holds, read without its lock.
struct Region {
    blocks: Vec<u32>,
    dungeon: bool,
    capsule: Capsule,
    area: RwLock<Area>,
}

impl Region {
    /// It holds a walk from `from` to `goal` for a body of `capsule`.
    fn is_holding(&self, from: Place, goal: Place, capsule: &Capsule) -> bool {
        self.capsule.same(capsule)
            && self.blocks.contains(&(from.cell & 0xFFFF_0000))
            && routing::is_held(&self.blocks, self.dungeon, from, goal)
    }
}

/// The process's areas, shared by the builder and every frame.
static REGIONS: Mutex<Vec<Arc<Region>>> = Mutex::new(Vec::new());

/// The area holding a walk, made the most recently used.
fn region_for(from: Place, goal: Place, capsule: &Capsule) -> Option<Arc<Region>> {
    let mut regions = REGIONS.lock().unwrap_or_else(|e| e.into_inner());
    let i = regions
        .iter()
        .position(|r| r.is_holding(from, goal, capsule))?;
    let r = regions.remove(i);
    regions.push(r.clone());
    Some(r)
}

/// An area assembled for a walk would hold it: not when the dungeon `from` is in is already
/// assembled without the goal, nor when the goal lies past the blocks a walk from `from` covers.
fn can_assemble(from: Place, goal: Place, capsule: &Capsule) -> bool {
    let regions = REGIONS.lock().unwrap_or_else(|e| e.into_inner());
    !regions
        .iter()
        .any(|r| r.dungeon && r.is_holding(from, from, capsule))
        && routing::is_held(&navarea::blocks_for(from.at, goal.at), false, from, goal)
}

/// The builder's answer to a walk asked: the job's number and the plan.
type Answer = (u64, Planned);

/// A walk a job is for: built for until it can be answered.
#[derive(Debug, Clone)]
struct Walk {
    /// Which handle asked: a newer job from the same one replaces it.
    who: u64,
    /// The job's number, counted by [`DONE`] once finished.
    number: u64,
    from: Place,
    goal: Place,
    /// How near the goal is there (metres, 3D).
    within: f32,
    capsule: Capsule,
    /// Where the answer goes, for a walk asked rather than only built for.
    reply: Option<Sender<Answer>>,
}

enum Job {
    /// Assemble the area around a walk, then build what the walk needs.
    Assemble(Walk),
    /// Build what a walk on an assembled area needs.
    Build(Arc<Region>, Walk),
}

impl Job {
    fn walk(&self) -> &Walk {
        match self {
            Job::Assemble(w) | Job::Build(_, w) => w,
        }
    }
}

/// The builder's line (and which start of it this is), started by the first handle that needs
/// it, so a process that never plans never pays for the thread.
static BUILDER: Mutex<Option<(u64, Sender<Job>)>> = Mutex::new(None);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
/// Jobs are numbered as they are sent; [`DONE`] is the highest number finished or dropped.
static NEXT_JOB: AtomicU64 = AtomicU64::new(1);
static DONE: AtomicU64 = AtomicU64::new(0);

fn builder(archives: &SharedArchives) -> Option<(u64, Sender<Job>)> {
    let mut slot = BUILDER.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(line) = slot.as_ref() {
        return Some(line.clone());
    }
    let (jobs_tx, jobs_rx) = std::sync::mpsc::channel::<Job>();
    let archives = archives.clone();
    match std::thread::Builder::new()
        .name("pathfinder".into())
        .spawn(move || build_forever(archives, jobs_rx))
    {
        Ok(_) => {
            let line = (NEXT_ID.fetch_add(1, Ordering::Relaxed), jobs_tx);
            *slot = Some(line.clone());
            Some(line)
        }
        Err(e) => {
            tracing::warn!("pathfinder: could not start: {e}");
            None
        }
    }
}

/// Forget builder start `generation`, whose thread has gone, so the next job starts one.
fn builder_gone(generation: u64) {
    let mut slot = BUILDER.lock().unwrap_or_else(|e| e.into_inner());
    if slot.as_ref().is_some_and(|(g, _)| *g == generation) {
        *slot = None;
    }
}

/// The steering's walk: asked as job `number`, and its answer once back.
struct Asked {
    number: u64,
    from: Place,
    goal: Place,
    answer: Option<Planned>,
}

/// The handle a session holds: plans on the shared areas, and queues what they lack.
pub struct Pathfinder {
    archives: SharedArchives,
    id: u64,
    /// Our line to the builder, and which start of it.
    line: Option<(u64, Sender<Job>)>,
    /// The builder's answers to our walks asked, and the sending end cloned into each.
    answers: (Sender<Answer>, Receiver<Answer>),
    /// When the last job went out.
    sent: Option<Instant>,
    /// Our last job, until it is done.
    job: Option<u64>,
    /// The builder could not be started.
    dead: bool,
    /// When the steering last asked.
    asked_at: Option<Instant>,
    asked: Option<Asked>,
    /// Offline runs: each plan builds what it needs on the asking thread before it answers.
    builds_itself: bool,
    /// The assets a handle that builds for itself builds with, opened on first use.
    own: Option<Assets>,
}

impl Pathfinder {
    /// A handle planning over `assets`' archives (the builder shares their mapping; see
    /// [`Assets::archives`]).
    pub fn new(assets: &Assets) -> Self {
        Pathfinder {
            archives: assets.archives(),
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            line: None,
            answers: std::sync::mpsc::channel(),
            sent: None,
            job: None,
            dead: false,
            asked_at: None,
            asked: None,
            builds_itself: false,
            own: None,
        }
    }

    /// Offline runs only: plans build what they need on the asking thread before they answer, as
    /// a live frame has its answer a frame or two later, so no test hangs on the builder's timing.
    #[cfg(any(test, feature = "testkit"))]
    pub fn build_when_planning(&mut self) {
        self.builds_itself = true;
    }

    /// The route from `from` to within `within` (metres, 3D) of `goal` for a body of `capsule`, on
    /// what is built now; `Building` when the area or its chunks are not, which are then queued.
    pub fn plan(&mut self, from: Place, goal: Place, within: f32, capsule: Capsule) -> Planned {
        let mut tries = 0;
        loop {
            let region = region_for(from, goal, &capsule);
            let planned = match &region {
                Some(r) => self.read(r).map_or(Planned::Building(Vec::new()), |area| {
                    routing::plan_built(&area, from, goal, within)
                }),
                None if can_assemble(from, goal, &capsule) => Planned::Building(Vec::new()),
                None => return Planned::NoWay(NoWay::OutOfRegion),
            };
            if !matches!(planned, Planned::Building(_)) || tries == OWN_TRIES {
                return planned;
            }
            tries += 1;
            let job = move |w| match region {
                Some(r) => Job::Build(r, w),
                None => Job::Assemble(w),
            };
            if !self.dispatch(job, from, goal, within, capsule) {
                return planned;
            }
        }
    }

    /// Where a character can stand within `radius` (metres, flat) of `around`, from what is built
    /// (see [`routing::standable_near`]); none while the area is not assembled.
    pub fn standable_near(
        &mut self,
        from: Place,
        around: Place,
        radius: f32,
        capsule: Capsule,
    ) -> Vec<Place> {
        let Some(region) = region_for(from, around, &capsule) else {
            if can_assemble(from, around, &capsule) {
                self.send(Job::Assemble, from, around, radius, capsule);
            }
            return Vec::new();
        };
        let found = region
            .area
            .try_read()
            .map(|area| routing::standable_near(&area, from, around, radius));
        found.unwrap_or_default()
    }

    /// A region's area to plan on: waited for by a handle that builds for itself, else `None`
    /// while the builder adds a chunk, which takes microseconds.
    fn read<'r>(&self, region: &'r Region) -> Option<std::sync::RwLockReadGuard<'r, Area>> {
        if self.builds_itself {
            region.area.read().ok()
        } else {
            region.area.try_read().ok()
        }
    }

    /// Have a job done for a walk: here and now by a handle that builds for itself, else queued
    /// for the builder (see [`send`](Self::send)). Whether it is done.
    fn dispatch(
        &mut self,
        job: impl FnOnce(Walk) -> Job,
        from: Place,
        goal: Place,
        within: f32,
        capsule: Capsule,
    ) -> bool {
        if !self.builds_itself {
            self.send(job, from, goal, within, capsule);
            return false;
        }
        let walk = Walk {
            who: self.id,
            number: 0,
            from,
            goal,
            within,
            capsule,
            reply: None,
        };
        let archives = &self.archives;
        let assets = self
            .own
            .get_or_insert_with(|| Assets::with_archives(archives.clone()));
        run(assets, job(walk));
        true
    }

    /// Queue a job for a walk, unless one went out within [`ASK_EVERY`].
    fn send(
        &mut self,
        job: impl FnOnce(Walk) -> Job,
        from: Place,
        goal: Place,
        within: f32,
        capsule: Capsule,
    ) {
        let now = Instant::now();
        if !self.dead && self.sent.is_none_or(|t| now.duration_since(t) >= ASK_EVERY) {
            let _ = self.send_now(job, from, goal, within, capsule, None);
        }
    }

    /// Queue a job for a walk now, its answer sent to `reply`. Its number, if sent.
    fn send_now(
        &mut self,
        job: impl FnOnce(Walk) -> Job,
        from: Place,
        goal: Place,
        within: f32,
        capsule: Capsule,
        reply: Option<Sender<Answer>>,
    ) -> Option<u64> {
        if self.line.is_none() {
            self.line = builder(&self.archives);
            if self.line.is_none() {
                self.dead = true;
                return None;
            }
        }
        let (generation, jobs) = self.line.as_ref()?;
        let number = NEXT_JOB.fetch_add(1, Ordering::Relaxed);
        let walk = Walk {
            who: self.id,
            number,
            from,
            goal,
            within,
            capsule,
            reply,
        };
        if jobs.send(job(walk)).is_err() {
            tracing::warn!("pathfinder: the builder stopped");
            builder_gone(*generation);
            self.line = None;
            return None;
        }
        self.sent = Some(Instant::now());
        self.job = Some(number);
        Some(number)
    }

    /// Our last job is finished, or there is none.
    fn is_job_done(&self) -> bool {
        self.job.is_none_or(|n| DONE.load(Ordering::Acquire) >= n)
    }

    /// Have the builder plan the steering's walk to within `within` of `to`, at most once per
    /// `ASK_EVERY`, so no search holds up a frame; [`take`](Self::take) picks the answer up.
    pub fn ask(&mut self, from: Place, to: Place, within: f32, capsule: Capsule, now: Instant) {
        if self.dead
            || self
                .asked_at
                .is_some_and(|t| now.duration_since(t) < ASK_EVERY)
        {
            return;
        }
        self.asked_at = Some(now);
        let reply = Some(self.answers.0.clone());
        let number = match region_for(from, to, &capsule) {
            Some(region) => {
                self.send_now(|w| Job::Build(region, w), from, to, within, capsule, reply)
            }
            None if can_assemble(from, to, &capsule) => {
                self.send_now(Job::Assemble, from, to, within, capsule, reply)
            }
            None => {
                self.asked = Some(Asked {
                    number: 0,
                    from,
                    goal: to,
                    answer: Some(Planned::NoWay(NoWay::OutOfRegion)),
                });
                return;
            }
        };
        self.asked = number.map(|number| Asked {
            number,
            from,
            goal: to,
            answer: None,
        });
    }

    /// The answer to the walk asked, once it is back: its waypoints, or `None` with no way.
    /// Dropped when planned from too far back, or for a goal moved on.
    pub fn take(&mut self, me: Vec3, goal: Vec3) -> Option<Vec<Vec3>> {
        while let Ok((number, planned)) = self.answers.1.try_recv() {
            if let Some(a) = self.asked.as_mut().filter(|a| a.number == number) {
                a.answer = Some(planned);
            }
        }
        self.asked.as_ref()?.answer.as_ref()?;
        let a = self.asked.take()?;
        if a.goal.at.distance(goal) > ANSWER_GOAL {
            tracing::debug!("pathfinder: answer for a goal we no longer want");
            return None;
        }
        if a.from.at.distance(me) > ANSWER_REACH {
            tracing::debug!(
                "pathfinder: answer planned {:.0} m back, too stale to follow",
                a.from.at.distance(me)
            );
            return None;
        }
        match a.answer? {
            Planned::Route(r) => Some(r.points()),
            Planned::NoWay(why) => {
                tracing::debug!("pathfinder: no way: {why:?}");
                None
            }
            Planned::Building(_) => None,
        }
    }

    /// A walk asked waits on the builder.
    pub fn busy(&self) -> bool {
        self.asked.as_ref().is_some_and(|a| a.answer.is_none()) && !self.is_job_done()
    }

    /// Wait up to `within` for the builder to finish our last job, so an offline run sees its
    /// answer as soon as a live frame would rather than many simulated seconds later.
    #[cfg(any(test, feature = "testkit"))]
    pub fn settle(&mut self, within: Duration) {
        let until = Instant::now() + within;
        while !self.is_job_done() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Forget the walk asked (the goal went away).
    pub fn reset(&mut self) {
        self.asked = None;
        self.asked_at = None;
    }
}

/// The builder thread: does the newest job of every handle, dropping those that piled up behind
/// it, and between jobs finishes the area used last, a chunk at a time.
fn build_forever(archives: SharedArchives, jobs: Receiver<Job>) {
    let assets = Assets::with_archives(archives);
    loop {
        let first = match jobs.try_recv() {
            Ok(j) => j,
            Err(TryRecvError::Disconnected) => return,
            Err(TryRecvError::Empty) => {
                if build_idle() {
                    continue;
                }
                match jobs.recv() {
                    Ok(j) => j,
                    Err(_) => return,
                }
            }
        };
        let mut batch = vec![first];
        while let Ok(next) = jobs.try_recv() {
            // A walk asked is replaced only by another asked.
            let (who, asked) = (next.walk().who, next.walk().reply.is_some());
            batch.retain(|j| j.walk().who != who || (j.walk().reply.is_some() && !asked));
            batch.push(next);
        }
        let last = batch.iter().map(|j| j.walk().number).max().unwrap_or(0);
        for job in batch {
            let number = job.walk().number;
            run(&assets, job);
            DONE.fetch_max(number, Ordering::Release);
        }
        // The jobs dropped for newer ones are done with too.
        DONE.fetch_max(last, Ordering::Release);
    }
}

/// Build what a walk needs, planning on what is built and building what that names, until it
/// names nothing more.
fn run(assets: &Assets, job: Job) {
    let (region, walk) = match job {
        Job::Build(region, walk) => (region, walk),
        Job::Assemble(walk) => {
            let found = region_for(walk.from, walk.goal, &walk.capsule);
            let can = || can_assemble(walk.from, walk.goal, &walk.capsule);
            match found.or_else(|| can().then(|| assemble(assets, &walk)).flatten()) {
                Some(region) => (region, walk),
                None => {
                    answer(&walk, Planned::NoWay(NoWay::OutOfRegion));
                    return;
                }
            }
        }
    };
    let started = Instant::now();
    let mut built = 0;
    let planned = loop {
        let planned = match region.area.read() {
            Ok(area) => routing::plan_built(&area, walk.from, walk.goal, walk.within),
            Err(_) => return,
        };
        let Planned::Building(chunks) = &planned else {
            break planned;
        };
        let fresh = chunks.iter().filter(|&&c| build_chunk(&region, c)).count();
        if fresh == 0 {
            break planned;
        }
        built += fresh;
    };
    tracing::debug!(
        "pathfinder: built {built} chunks for a walk in {:?}",
        started.elapsed()
    );
    answer(&walk, planned);
}

/// Send a walk asked its answer. A handle that has gone is nobody's loss.
fn answer(walk: &Walk, planned: Planned) {
    if let Some(reply) = &walk.reply {
        let _ = reply.send((walk.number, planned));
    }
}

/// Assemble the area around a walk and keep it, dropping the one used longest ago.
fn assemble(assets: &Assets, walk: &Walk) -> Option<Arc<Region>> {
    let started = Instant::now();
    let here = walk.from.cell & 0xFFFF_0000;
    let want = navarea::blocks_for(walk.from.at, walk.goal.at);
    let area = match Area::build(assets, &want, &walk.capsule, here) {
        Ok(a) => a,
        Err(e) => {
            tracing::debug!("pathfinder: cannot assemble around {here:#010x}: {e}");
            return None;
        }
    };
    tracing::debug!(
        "pathfinder: assembled {} blocks around {here:#010x} ({} triangles) in {:?}",
        area.blocks.len(),
        area.collision.tris.len(),
        started.elapsed()
    );
    let region = Arc::new(Region {
        blocks: area.blocks.clone(),
        dungeon: area.dungeon,
        capsule: area.capsule(),
        area: RwLock::new(area),
    });
    let mut regions = REGIONS.lock().unwrap_or_else(|e| e.into_inner());
    while regions.len() >= AREAS {
        regions.remove(0);
    }
    regions.push(region.clone());
    Some(region)
}

/// Build chunk `c` of a region: made under a read lock, so frames plan on meanwhile, and added
/// under a write lock. Whether it was new.
fn build_chunk(region: &Region, c: (i32, i32)) -> bool {
    let built = match region.area.read() {
        Ok(area) => area.read_ground(|ground, nav| nav.build_chunk(ground, c)),
        Err(_) => return false,
    };
    match region.area.write() {
        Ok(mut area) => {
            let before = area.nav.chunk_count();
            area.nav.apply(built);
            area.nav.chunk_count() > before
        }
        Err(_) => false,
    }
}

/// Build one more chunk of the area used last, unless it is complete. Whether one was built.
fn build_idle() -> bool {
    let last = REGIONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .last()
        .cloned();
    let Some(region) = last else {
        return false;
    };
    let next = match region.area.read() {
        Ok(area) => area.nav.next_unbuilt(),
        Err(_) => None,
    };
    next.is_some_and(|c| build_chunk(&region, c))
}
