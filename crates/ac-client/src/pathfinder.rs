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
/// Areas the process keeps, most recently used last, beyond those a session stands in (never
/// dropped); each is nine blocks of collision, about 15 MB.
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

/// What became of a job handed on (see `Pathfinder::dispatch`).
enum Done {
    /// Sent to the builder, or held back by [`ASK_EVERY`].
    Queued,
    /// Done on the asking thread, on this region.
    Here(Arc<Region>),
    /// Done on the asking thread, and no area could be assembled for it.
    Failed,
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
    /// Build these chunks of an assembled area: what a look round the walk's goal needs.
    Chunks(Arc<Region>, Vec<(i32, i32)>, Walk),
}

impl Job {
    fn walk(&self) -> &Walk {
        match self {
            Job::Assemble(w) | Job::Build(_, w) | Job::Chunks(_, _, w) => w,
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
    /// When the steering last asked, from where, and for what goal.
    asked_at: Option<(Instant, Vec3, Vec3)>,
    asked: Option<Asked>,
    /// Offline runs: each plan builds what it needs on the asking thread before it answers.
    builds_itself: bool,
    /// The handle the arrival prefetches go out as, so the walk's own jobs never replace one.
    ahead_id: u64,
    /// The walk last prepared for, asked once.
    prepared: Option<(Place, Place)>,
    /// The assets a handle that builds for itself builds with, opened on first use.
    own: Option<Assets>,
    /// The area our last plan or ask was on, held so no other session's area pushes it out.
    region: Option<Arc<Region>>,
    /// Calls of [`plan`](Self::plan), for the tests that count them.
    pub plans: u32,
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
            ahead_id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            prepared: None,
            own: None,
            region: None,
            plans: 0,
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
        self.plans += 1;
        self.plan_now(from, goal, within, capsule)
    }

    fn plan_now(&mut self, from: Place, goal: Place, within: f32, capsule: Capsule) -> Planned {
        let (mut tries, mut held) = (0, None);
        loop {
            let region = held.take().or_else(|| region_for(from, goal, &capsule));
            self.hold(&region);
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
            match self.dispatch(job, from, goal, within, capsule) {
                Done::Queued => return planned,
                Done::Here(r) => held = Some(r),
                Done::Failed => return Planned::NoWay(NoWay::OutOfRegion),
            }
        }
    }

    /// Where a character can stand within `radius` (metres, flat) of `around`, from what is built
    /// (see [`routing::standable_near`]); `None` while the chunks it looks at are not, then queued.
    pub fn standable_near(
        &mut self,
        from: Place,
        around: Place,
        radius: f32,
        capsule: Capsule,
    ) -> Option<Vec<Place>> {
        let (mut tries, mut held) = (0, None);
        loop {
            let region = held.take().or_else(|| region_for(from, around, &capsule));
            self.hold(&region);
            let looked = match &region {
                Some(r) => self
                    .read(r)
                    .map(|area| routing::standable_near(&area, from, around, radius)),
                None if can_assemble(from, around, &capsule) => None,
                None => return Some(Vec::new()),
            };
            let missing = match looked {
                Some(Ok(spots)) => return Some(spots),
                Some(Err(chunks)) => chunks,
                None => Vec::new(),
            };
            if tries == OWN_TRIES {
                return None;
            }
            tries += 1;
            let job = move |w| match region {
                Some(r) => Job::Chunks(r, missing, w),
                None => Job::Assemble(w),
            };
            match self.dispatch(job, from, around, radius, capsule) {
                Done::Here(r) => held = Some(r),
                Done::Queued => return None,
                Done::Failed => return Some(Vec::new()),
            }
        }
    }

    /// `place` on its cell's floor as a walk from `from` would end there (see
    /// [`routing::put_on_floor`]); `None` while the area holding both is assembled.
    pub fn put_on_floor(
        &mut self,
        from: Place,
        place: Place,
        capsule: Capsule,
    ) -> Result<Option<Place>, NoWay> {
        let (mut tries, mut held) = (0, None);
        loop {
            let region = held.take().or_else(|| region_for(from, place, &capsule));
            self.hold(&region);
            match &region {
                Some(r) => {
                    return self
                        .read(r)
                        .map(|area| routing::put_on_floor(&area, from, place))
                        .transpose()
                }
                None if can_assemble(from, place, &capsule) => {}
                None => return Err(NoWay::OutOfRegion),
            }
            if tries == OWN_TRIES {
                return Ok(None);
            }
            tries += 1;
            match self.dispatch(Job::Assemble, from, place, 0.0, capsule) {
                Done::Here(r) => held = Some(r),
                Done::Queued => return Ok(None),
                Done::Failed => return Err(NoWay::OutOfRegion),
            }
        }
    }

    /// Keep `region` as the one this session is on, if any.
    fn hold(&mut self, region: &Option<Arc<Region>>) {
        if let Some(r) = region {
            self.region = Some(r.clone());
        }
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
    /// for the builder (see [`send`](Self::send)).
    fn dispatch(
        &mut self,
        job: impl FnOnce(Walk) -> Job,
        from: Place,
        goal: Place,
        within: f32,
        capsule: Capsule,
    ) -> Done {
        if !self.builds_itself {
            self.send(job, from, goal, within, capsule);
            return Done::Queued;
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
        // The region built on is kept, not found again: other walks may push it out meanwhile.
        run(assets, job(walk), true).map_or(Done::Failed, Done::Here)
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

    /// Have the builder assemble, ahead of time, the area a walk from `from` to `goal` will want (or
    /// round `from` alone when that cannot hold both): a portal's far side, ready on arrival.
    pub fn prepare(&mut self, from: Place, goal: Place, capsule: Capsule) {
        let goal = if can_assemble(from, goal, &capsule) {
            goal
        } else {
            from
        };
        if self.builds_itself || self.dead || self.prepared == Some((from, goal)) {
            return;
        }
        self.prepared = Some((from, goal));
        let held = REGIONS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|r| r.is_holding(from, goal, &capsule));
        if held || !can_assemble(from, goal, &capsule) {
            return;
        }
        if self.line.is_none() {
            self.line = builder(&self.archives);
        }
        let Some((_, jobs)) = self.line.as_ref() else {
            return;
        };
        let walk = Walk {
            who: self.ahead_id,
            number: NEXT_JOB.fetch_add(1, Ordering::Relaxed),
            from,
            goal,
            within: 1.0,
            capsule,
            reply: None,
        };
        let _ = jobs.send(Job::Assemble(walk));
    }

    /// Have the builder plan the steering's walk to within `within` of `to`, so no search holds up
    /// a frame; [`take`](Self::take) picks the answer up. A handle that builds for itself answers now.
    pub fn ask(&mut self, from: Place, to: Place, within: f32, capsule: Capsule, now: Instant) {
        if self.dead || self.is_repeat(from.at, to.at, now) {
            return;
        }
        self.asked_at = Some((now, from.at, to.at));
        if self.builds_itself {
            let answer = self.plan_now(from, to, within, capsule);
            self.asked = Some(Asked {
                number: 0,
                from,
                goal: to,
                answer: Some(answer),
            });
            return;
        }
        let reply = Some(self.answers.0.clone());
        let found = region_for(from, to, &capsule);
        self.hold(&found);
        let number = match found {
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

    /// Whether a walk asked is the last one again (its start and goal, within [`ASK_EVERY`]) or
    /// one whose answer, still being planned, will do from here.
    fn is_repeat(&self, from: Vec3, goal: Vec3, now: Instant) -> bool {
        let pending = self.busy()
            && self.asked.as_ref().is_some_and(|a| {
                a.goal.at.distance(goal) <= ANSWER_GOAL && a.from.at.distance(from) <= ANSWER_REACH
            });
        pending
            || self.asked_at.is_some_and(|(t, f, g)| {
                now.duration_since(t) < ASK_EVERY
                    && g.distance(goal) <= ac_nav::steering::ARRIVE
                    && f.distance(from) <= ac_nav::steering::ARRIVE
            })
    }

    /// The answer to the walk asked, once it is back: the place it was planned from, and the plan,
    /// a no way with its reason. Dropped when planned from too far back, or for a goal moved on.
    pub fn take(&mut self, me: Vec3, goal: Vec3) -> Option<(Place, Planned)> {
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
        let planned = a.answer?;
        if let Planned::NoWay(why) = &planned {
            tracing::debug!("pathfinder: no way: {why:?}");
        }
        Some((a.from, planned))
    }

    /// Whether the walk test passes from `from` to `to` for a body of `capsule`, on the region
    /// holding it (see [`routing::can_walk`]): `false` with none, or while the builder writes to it.
    pub fn can_walk(&self, from: Place, to: Vec3, capsule: Capsule) -> bool {
        let goal = Place {
            at: to,
            cell: from.cell & 0xFFFF_0000,
        };
        let Some(region) = region_for(from, goal, &capsule) else {
            return false;
        };
        let walks = self
            .read(&region)
            .is_some_and(|area| routing::can_walk(&area, from, to));
        walks
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
            let _ = run(&assets, job, false);
            DONE.fetch_max(number, Ordering::Release);
        }
        // The jobs dropped for newer ones are done with too.
        DONE.fetch_max(last, Ordering::Release);
    }
}

/// Build what a walk needs, planning on what is built and building what that names, until it
/// names nothing more; the region built on, if one was assembled. `hold`: see [`build_chunk`].
fn run(assets: &Assets, job: Job, hold: bool) -> Option<Arc<Region>> {
    let (region, walk) = match job {
        Job::Build(region, walk) => (region, walk),
        Job::Chunks(region, chunks, _) => {
            for &c in &chunks {
                build_chunk(&region, c, hold);
            }
            return Some(region);
        }
        Job::Assemble(walk) => {
            let found = region_for(walk.from, walk.goal, &walk.capsule);
            let can = || can_assemble(walk.from, walk.goal, &walk.capsule);
            match found.or_else(|| can().then(|| assemble(assets, &walk)).flatten()) {
                Some(region) => (region, walk),
                None => {
                    answer(&walk, Planned::NoWay(NoWay::OutOfRegion));
                    return None;
                }
            }
        }
    };
    let started = Instant::now();
    let mut built = 0;
    let planned = loop {
        let planned = match region.area.read() {
            Ok(area) => routing::plan_built(&area, walk.from, walk.goal, walk.within),
            Err(_) => return None,
        };
        let Planned::Building(chunks) = &planned else {
            break planned;
        };
        let fresh = chunks
            .iter()
            .filter(|&&c| build_chunk(&region, c, hold))
            .count();
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
    Some(region)
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
    make_room(&mut regions);
    regions.push(region.clone());
    Some(region)
}

/// Drop the areas used longest ago until fewer than [`AREAS`] are kept, skipping any a session or
/// a job still holds: planned cold again, every walk there would stand waiting on the builder.
fn make_room<T>(kept: &mut Vec<Arc<T>>) {
    while kept.len() >= AREAS {
        let Some(i) = kept.iter().position(|r| Arc::strong_count(r) == 1) else {
            return;
        };
        kept.remove(i);
    }
}

/// Build chunk `c` of a region: made under a read lock, so frames plan on meanwhile, and added
/// under a write lock; with `hold`, made under the write lock too, so no other builder's chunk
/// voids it (a handle that builds for itself must answer now). Whether it was new.
fn build_chunk(region: &Region, c: (i32, i32), hold: bool) -> bool {
    if hold {
        let Ok(mut area) = region.area.write() else {
            return false;
        };
        let before = area.nav.chunk_count();
        area.with_ground(|ground, nav| {
            let built = nav.build_chunk(ground, c);
            nav.apply(built);
        });
        return area.nav.chunk_count() > before;
    }
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
    next.is_some_and(|c| build_chunk(&region, c, false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Client;

    const DUNGEON: u32 = 0x01F6_0000;

    /// A character beside the bookcases in the Holtburg Dungeon, and where it stands.
    fn by_the_bookcases() -> (Client, Place) {
        let c =
            crate::testkit::standing_in_the_field(25, 0x01F6_0224, Vec3::new(30.37, -51.89, 0.0));
        let pl = c.player.as_ref().expect("a body");
        let me = Place {
            at: pl.world_position(),
            cell: pl.cell,
        };
        (c, me)
    }

    fn there(local: Vec3, cell: u32) -> Place {
        Place {
            at: ac_world::landblock_origin(DUNGEON) + local,
            cell,
        }
    }

    /// Somewhere round the bookcases, in the next cell.
    fn round_them() -> Place {
        there(Vec3::new(36.0, -54.0, 0.0), 0x01F6_0233)
    }

    #[test]
    fn an_area_a_session_holds_is_never_pushed_out() {
        let held: Vec<Arc<usize>> = (0..AREAS + 2).map(Arc::new).collect();
        let mut kept = held.clone();
        make_room(&mut kept);
        assert_eq!(kept.len(), AREAS + 2, "all held, none dropped");
        let mut kept: Vec<Arc<usize>> = (0..AREAS).map(Arc::new).collect();
        let oldest = kept[0].clone();
        make_room(&mut kept);
        let left: Vec<usize> = kept.iter().map(|a| **a).collect();
        let mut want: Vec<usize> = (2..AREAS).collect();
        want.insert(0, 0);
        assert_eq!(left, want, "the oldest held kept, the next went");
        drop(oldest);
    }

    #[test]
    #[ignore = "needs AC_DATA_DIR"]
    fn an_offline_ask_is_answered_before_the_next_frame() {
        let (mut c, me) = by_the_bookcases();
        let cap = c.player.as_ref().unwrap().capsule();
        let goal = round_them();
        c.pathfinder.ask(me, goal, 0.25, cap, Instant::now());
        match c.pathfinder.take(me.at, goal.at) {
            Some((from, Planned::Route(r))) => {
                assert_eq!(from, me);
                assert!(!r.waypoints.is_empty());
            }
            other => panic!("not answered at once: {other:?}"),
        }
    }

    #[test]
    #[ignore = "needs AC_DATA_DIR"]
    fn a_new_walk_is_asked_at_once_and_a_repeat_waits() {
        let (mut c, me) = by_the_bookcases();
        let cap = c.player.as_ref().unwrap().capsule();
        let (goal, other) = (round_them(), there(Vec3::new(26.0, -51.89, 0.0), DUNGEON));
        let t0 = Instant::now();
        let soon = t0 + Duration::from_millis(100);
        c.pathfinder.ask(me, goal, 0.25, cap, t0);
        assert!(c.pathfinder.take(me.at, goal.at).is_some(), "asked");
        c.pathfinder.ask(me, goal, 0.25, cap, soon);
        assert!(c.pathfinder.take(me.at, goal.at).is_none(), "a repeat");
        c.pathfinder.ask(me, other, 0.25, cap, soon);
        assert!(c.pathfinder.take(me.at, other.at).is_some(), "another goal");
        let nudged = there(Vec3::new(26.0, -50.89, 0.0), DUNGEON);
        c.pathfinder.ask(me, nudged, 0.25, cap, soon);
        assert!(
            c.pathfinder.take(me.at, nudged.at).is_some(),
            "a goal a metre on is another walk"
        );
        let moved = Place {
            at: me.at + Vec3::new(-2.0, 0.0, 0.0),
            ..me
        };
        c.pathfinder.ask(moved, other, 0.25, cap, soon);
        assert!(
            c.pathfinder.take(moved.at, other.at).is_some(),
            "the same goal from elsewhere"
        );
        c.pathfinder.ask(moved, other, 0.25, cap, soon + ASK_EVERY);
        assert!(
            c.pathfinder.take(moved.at, other.at).is_some(),
            "a repeat once the throttle is past"
        );
    }

    #[test]
    #[ignore = "needs AC_DATA_DIR"]
    fn standing_room_not_built_yet_is_not_an_empty_answer() {
        // A handle with a builder thread, live: the area is queued, not answered as having none.
        let assets = Assets::open(ac_dat::test_data_dir()).expect("the DATs");
        let mut live = Pathfinder::new(&assets);
        let at = Place::on_map(glam::Vec2::new(
            0xE7 as f32 * 192.0 + 96.0,
            0x4E as f32 * 192.0 + 96.0,
        ));
        let spots = live.standable_near(at, at, 2.5, Capsule::default());
        assert_eq!(spots, None, "queued for the builder");
    }

    #[test]
    #[ignore = "needs AC_DATA_DIR"]
    fn a_portal_exit_prepared_ahead_is_assembled_before_anyone_plans_there() {
        // A live handle, as a journey's portal step begins: the far side is queued as its own job,
        // and is there to plan on when the character arrives.
        let assets = Assets::open(ac_dat::test_data_dir()).expect("the DATs");
        let mut live = Pathfinder::new(&assets);
        let exit = Place::on_map(glam::Vec2::new(
            0xC9 as f32 * 192.0 + 96.0,
            0x8B as f32 * 192.0 + 96.0,
        ));
        let next = Place::on_map(exit.at.truncate() + glam::Vec2::new(40.0, 20.0));
        let cap = Capsule::default();
        assert!(region_for(exit, next, &cap).is_none(), "not assembled yet");
        live.prepare(exit, next, cap);
        let until = Instant::now() + Duration::from_secs(60);
        while region_for(exit, next, &cap).is_none() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            region_for(exit, next, &cap).is_some(),
            "the builder never assembled it"
        );
        // Asked twice, it is queued once.
        live.prepare(exit, next, cap);
        assert_eq!(live.prepared, Some((exit, next)));
    }

    #[test]
    #[ignore = "needs AC_DATA_DIR"]
    fn can_walk_answers_the_walk_test_on_the_region() {
        let (mut c, me) = by_the_bookcases();
        let cap = c.player.as_ref().unwrap().capsule();
        // Planned first each time, so the region is there: other tests' areas may push it out.
        let mut walks = |to: Vec3| {
            assert!(matches!(c.plan_walk(round_them(), 0.25), Planned::Route(_)));
            c.pathfinder.can_walk(me, there(to, DUNGEON).at, cap)
        };
        assert!(walks(Vec3::new(26.0, -51.89, 0.0)), "across the room");
        // The top of a bookcase, 1.9 m up.
        assert!(!walks(Vec3::new(34.68, -50.7, 1.915)), "onto the bookcase");
    }

    #[test]
    #[ignore = "needs AC_DATA_DIR"]
    fn a_place_is_put_on_its_own_floor() {
        // A map point on the Holtburg field takes the terrain's height.
        let mut c =
            crate::testkit::standing_in_the_field(20, 0xA9B4_0019, Vec3::new(84.0, 84.0, 94.0));
        let assets = c.assets.clone();
        let pl = c.player.as_mut().expect("a body");
        let me = Place {
            at: pl.world_position(),
            cell: pl.cell,
        };
        let cap = pl.capsule();
        let field = Place::on_map(me.at.truncate() + glam::Vec2::new(20.0, 5.0));
        pl.collision(&assets, field.cell & 0xFFFF_0000);
        let ground = pl.terrain_height(field.at.x, field.at.y).expect("terrain");
        let on = c.pathfinder.put_on_floor(me, field, cap);
        let on = on.expect("a floor").expect("assembled");
        assert!(
            (on.at.z - ground).abs() < 0.05,
            "{on:?} over ground {ground}"
        );
        assert_eq!(on.cell, field.cell);
        // In the Holtburg Dungeon, a ledge 9.5 m over the room: on its own cell's floor.
        let rooms = c.assets.block_collision(DUNGEON).unwrap();
        let start = Vec3::new(223.5, 47_158.5, 0.0);
        let (z, cell) = rooms.world.floor_at(start, 0.6, 1.5).unwrap();
        let ledge = Vec3::new(262.0, 47_182.0, 9.5);
        let (high, ledge_cell) = rooms.world.floor_at(ledge, 0.6, 1.5).unwrap();
        crate::testkit::stand(
            &mut c,
            cell,
            start.with_z(z) - ac_world::landblock_origin(DUNGEON),
        );
        let me = Place {
            at: start.with_z(z),
            cell,
        };
        let up = Place {
            at: ledge.with_z(high + 0.8),
            cell: ledge_cell,
        };
        let on = c.pathfinder.put_on_floor(me, up, cap);
        let on = on.expect("a floor").expect("assembled");
        assert_eq!(on.cell, ledge_cell);
        assert!((on.at.z - high).abs() < 0.05, "{on:?}, the ledge at {high}");
    }
}
