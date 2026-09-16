//! Going to see someone: "take me to Archmage Cindrue".
//!
//! A person is a point in three dimensions, and often indoors: Cindrue
//! keeps her shop on the upper floor of a house in Holtburg. Travel alone
//! gets a character to a place on the flat -- a journey's last step is a
//! point on the terrain grid, and it has arrived within three metres of
//! it on the map -- which for her is the ground floor just inside the
//! door, three metres under her feet. It stopped there, said it had
//! arrived, and nobody was ever spoken to.
//!
//! A visit carries on from there. Far away, the journey is made the way
//! any journey is (portals, recalls, gems). Within walking distance the
//! last stretch goes to the steering with her real height
//! (`Client::head_toward`), which takes the character through the
//! doorway and up the stairs. Only once the character stands on her
//! floor, close by, is she used.
//!
//! The use waits for that on purpose. The server's reach for a use is a
//! cylinder and passes up through a ceiling: a use sent from the ground
//! floor under her is answered at once, with no walk, and the character
//! never climbs at all.
//!
//! A visit is also what becomes of a double-click on something the
//! server could not walk the character to in time. The client lets the
//! server's walk go after twelve seconds, and the first step it then
//! takes of its own ends the server's walk and the use with it, so a
//! long walk used to lose the use without a word. The rest of the way is
//! walked as a visit instead, and the use is sent again on arriving --
//! unless the server had already answered it, in which case the use
//! happened and there is nothing to carry on.

use std::time::{Duration, Instant};

use ac_world::landmarks::Landmark;
use ac_world::object::MoveTarget;
use ac_world::portals::Portal;
use ac_world::WorldObject;
use glam::Vec3;

use crate::travel::WALKABLE;
use crate::{Client, Follow, SAME_FLOOR};

/// Within this far of the person (metres, in three dimensions) and on
/// their floor, the use is sent. Inside the server's own reach for
/// talking to someone, so the use needs no walk of the server's.
pub const REACH: f32 = 3.0;
/// How close to the person the walk stops (flat): well inside
/// [`REACH`], so arriving is arriving in reach.
const STOP: f32 = 1.5;
/// A visit that has come no nearer in this long is given up. A journey
/// under way does not count against it: travel keeps its own clocks for
/// a portal that will not take the character, a recall that fizzles and
/// a step that goes nowhere, each ending in another plan or in giving
/// up, and a second coarser clock over them would cut an honest walk to
/// a distant portal short.
pub const GIVE_UP: Duration = Duration::from_secs(60);
/// Coming this much nearer counts as progress.
const PROGRESS: f32 = 1.0;
/// Someone found by name must stand this near where the visit is bound
/// to be the one it is bound for: the gazetteer names the same
/// shopkeeper in more than one town.
const NAMED_NEAR: f32 = 20.0;
/// A server walk that times out is carried on only for a use this
/// recent: an older one was something else.
const USE_REMEMBERED: Duration = Duration::from_secs(20);
/// Moved this far between two frames: carried off, not walked.
const CARRIED_OFF: f32 = 60.0;
/// Arrived and nobody of that name in view: wait this long for them to be
/// sent before saying so. The server sends the things of a spot as the
/// character comes up to it, and a long walk arrives in the very frame it
/// gets there -- a walk to the Holtburg Dungeon's portal found nothing and
/// stopped beside it, while a visit begun beside it went straight through.
const ARRIVAL_WAIT: Duration = Duration::from_secs(5);
/// A portal a visit has used is given this long to carry the character
/// off: a visit to it begun again in that time is the same visit, still
/// going. Beside a portal a visit is over in the frame it uses it, and a
/// caller asking every frame until the character is gone used the
/// Holtburg Dungeon's portal four times before the first use took.
const PORTAL_TAKES: Duration = Duration::from_secs(5);

/// Somewhere to go, and whom to use once there.
#[derive(Clone, Debug)]
pub struct Visit {
    /// What the log calls it.
    pub label: String,
    /// Where: world position, real height included.
    pub goal: Vec3,
    /// The cell it stands in, for the log and for whoever reads it.
    pub cell: u32,
    /// Whom to look for by name, when not yet in view. `None` for a
    /// place (a lifestone): the visit ends on arriving.
    pub person: Option<String>,
    /// Whom to use, once seen (or from the start, for a double-click).
    pub guid: Option<u32>,
    /// The player used it by hand, or picked a portal to go to, so it is
    /// used on arriving whatever it is. A visit to a gazetteer row speaks
    /// only to someone: see [`someone_to_talk_to`].
    by_hand: bool,
    /// Going through a portal: the visit is over once the character has
    /// been carried off, which walking into the mouth usually does before
    /// the use is ever sent.
    through: bool,
    /// Where the character stood last frame, to see that happen.
    last_at: Option<Vec3>,
    /// When the character got there, while waiting for whoever it is for
    /// to be sent (see [`ARRIVAL_WAIT`]).
    arrived_at: Option<Instant>,
    /// A journey has been set off on, so a far goal and no journey
    /// means the journey ended short of it.
    journeyed: bool,
    /// The goal last handed to the steering, to tell our walk from
    /// someone else's.
    walking: Option<Vec3>,
    /// Nearest the character has come, and when that last improved.
    best: f32,
    since: Option<Instant>,
}

impl Visit {
    /// Going to a landmark: its real spot and cell, and a talk with
    /// whoever keeps it when someone does.
    pub fn landmark(l: &Landmark) -> Visit {
        Visit::new(
            l.name.clone(),
            l.at,
            l.cell,
            l.kind.may_be_someone().then(|| l.name.clone()),
            None,
        )
    }

    /// Going to a portal and through it: its mouth's real spot and cell,
    /// and a use of it on arriving. A portal picked to travel to is picked
    /// to be taken.
    pub fn portal(p: &Portal) -> Visit {
        let mut v = Visit::new(
            p.name.clone(),
            p.from,
            p.from_cell,
            Some(p.name.clone()),
            None,
        );
        v.by_hand = true;
        v.through = true;
        v
    }

    fn new(
        label: String,
        goal: Vec3,
        cell: u32,
        person: Option<String>,
        guid: Option<u32>,
    ) -> Visit {
        Visit {
            label,
            goal,
            cell,
            person,
            guid,
            by_hand: false,
            through: false,
            last_at: None,
            arrived_at: None,
            journeyed: false,
            walking: None,
            best: f32::INFINITY,
            since: None,
        }
    }

    /// Whether the visit has come no nearer (`away`, metres) for
    /// [`GIVE_UP`]. A journey under way is progress: see [`GIVE_UP`].
    fn stalled(&mut self, away: f32, traveling: bool, now: Instant) -> bool {
        if traveling {
            self.best = f32::INFINITY;
            self.since = Some(now);
            return false;
        }
        if away < self.best - PROGRESS {
            self.best = away;
            self.since = Some(now);
        }
        now.duration_since(*self.since.get_or_insert(now)) > GIVE_UP
    }
}

/// The visit being made, and the last use sent by hand.
#[derive(Default)]
pub struct Visits {
    pub(crate) current: Option<Visit>,
    /// The last thing in the world used by hand (a double-click) that the
    /// server has not yet answered, and when: what a server walk that runs
    /// out was walking to.
    pub(crate) last_use: Option<(u32, Instant)>,
    /// The last visit ended by getting there (whoever it was for used, or
    /// the portal gone through), for a caller who started it just now:
    /// beside a portal, the first frame is the whole visit.
    pub(crate) finished: bool,
    /// The portal a visit last used, by name, and when: see
    /// [`PORTAL_TAKES`].
    pub(crate) portal_used: Option<(String, Instant)>,
}

impl Visits {
    /// A use of `guid`, something in the world, has been sent.
    pub(crate) fn used(&mut self, guid: u32, now: Instant) {
        self.last_use = Some((guid, now));
    }

    /// The server's `UseDone`: it has finished with the use, done or
    /// failed. A server walk that runs out after this is not carried on.
    ///
    /// Answered from under her floor, the server's walk is never ended
    /// by the server (no motion follows a use once it is done), so the
    /// client's cut-off comes twelve seconds later all the same. Carrying
    /// that on would climb the stairs and talk to her a second time.
    ///
    /// Nothing says which use an answer is for. A second double-click
    /// while the first is walking ends the first, and the first's answer
    /// then clears the second: that walk, if it runs out, is left as it
    /// always was.
    pub(crate) fn answered(&mut self) {
        self.last_use = None;
    }
}

/// What a visit does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// Too far to walk: set off on a journey.
    Journey,
    /// Too far to walk, and the journey is under way.
    OnTheWay,
    /// Too far to walk, and the journey is over: it ended short.
    GiveUp,
    /// Walk the rest: through the door and up the stairs, if that is
    /// where they are.
    Walk,
    /// On their floor and close: use them, or stop there.
    Arrived,
}

/// Decide it from where the character stands (`me`), where the visit
/// goes, whether a journey is under way and whether one was set off on.
pub fn next(me: Vec3, goal: Vec3, traveling: bool, journeyed: bool) -> Next {
    let flat = me.truncate().distance(goal.truncate());
    if flat > WALKABLE {
        return if traveling {
            Next::OnTheWay
        } else if journeyed {
            Next::GiveUp
        } else {
            Next::Journey
        };
    }
    if arrived(me, goal) {
        Next::Arrived
    } else {
        Next::Walk
    }
}

/// Close to `goal` and on its floor. Three metres straight under
/// someone is close on the map and nowhere near them.
pub fn arrived(me: Vec3, goal: Vec3) -> bool {
    (me.z - goal.z).abs() <= SAME_FLOOR && me.distance(goal) <= REACH
}

/// Whether a visit that arrived at `since` still waits for whoever it is
/// for to be sent (see [`ARRIVAL_WAIT`]).
fn waits_for_them(since: Instant, now: Instant) -> bool {
    now.duration_since(since) < ARRIVAL_WAIT
}

/// Whether the portal called `name` was used by a visit so lately (see
/// `used`) that it may still be about to carry the character off.
fn still_taking(used: Option<&(String, Instant)>, name: &str, now: Instant) -> bool {
    used.is_some_and(|(portal, at)| portal == name && now.duration_since(*at) < PORTAL_TAKES)
}

/// Whether the character was carried off between the frame it stood at
/// `last` and this one at `me`: a jump no walk makes.
pub fn carried_off(last: Option<Vec3>, me: Vec3) -> bool {
    last.is_some_and(|l| l.distance(me) > CARRIED_OFF)
}

/// Whether a server walk to `guid` that has run out was walking for a
/// use sent by hand and not yet answered.
///
/// No waiting for the server's own walk to give up: the client's first
/// step after the cut-off is a movement of its own, and the server ends
/// its walk, use and all, on any movement the client sends.
fn continues(last_use: Option<(u32, Instant)>, guid: u32, now: Instant) -> bool {
    last_use.is_some_and(|(used, at)| used == guid && now.duration_since(at) <= USE_REMEMBERED)
}

/// Whether `o` is someone a visit speaks to on arriving: a shopkeeper,
/// or a creature that cannot be fought. The gazetteer's npc rows are
/// also Wailing Statues, Exploration Markers, generators and a Rolling
/// Ball; going to one of those is going there, not using it -- and in
/// combat mode a use of something attackable is an attack.
pub fn someone_to_talk_to(o: &WorldObject) -> bool {
    use ac_world::{item_type, object_desc_flags as f};
    if o.object_desc_flags & f::VENDOR != 0 {
        return true;
    }
    o.item_type & item_type::CREATURE != 0 && o.object_desc_flags & (f::ATTACKABLE | f::PLAYER) == 0
}

impl Client {
    /// Go to a landmark and, when someone keeps it, talk to them: a
    /// journey when it is far, a walk to their real spot, stairs and
    /// all, and a use once on their floor. False when there is no way.
    pub fn visit_landmark(&mut self, l: &Landmark) -> bool {
        self.start_visit(Visit::landmark(l))
    }

    /// Go to a portal and through it: a journey when it is far, a walk to
    /// its mouth, and a use of it on arriving. False when there is no way.
    pub fn visit_portal(&mut self, p: &Portal) -> bool {
        self.start_visit(Visit::portal(p))
    }

    /// What a visit is going to, while one is being made.
    pub fn visiting(&self) -> Option<&str> {
        self.visits.current.as_ref().map(|v| v.label.as_str())
    }

    fn start_visit(&mut self, v: Visit) -> bool {
        if self.player.is_none() {
            tracing::warn!("visit: the character is not in the world");
            return false;
        }
        let now = Instant::now();
        if v.through && still_taking(self.visits.portal_used.as_ref(), &v.label, now) {
            return true;
        }
        self.drop_visit("another visit");
        tracing::info!(
            "visit: going to {} at {:?} in {:#010x}",
            v.label,
            v.goal,
            v.cell
        );
        self.visits.current = Some(v);
        self.visits.finished = false;
        self.tick_visit(now);
        // Still under way, or already there.
        self.visits.current.is_some() || self.visits.finished
    }

    /// Stop the visit being made, if there is one, and the walk it asked
    /// for. The journey it set off on is the caller's to end.
    pub(crate) fn drop_visit(&mut self, why: &str) {
        let Some(v) = self.visits.current.take() else {
            return;
        };
        tracing::info!("visit: {} dropped: {why}", v.label);
        self.let_go_of(&v);
    }

    /// Stop the walk this visit asked for, and leave anyone else's.
    /// Every way a visit ends goes through here: a walk left behind by a
    /// visit that is gone would keep pulling the character along with
    /// nothing to show for it and no Cancel to stop it.
    fn let_go_of(&mut self, v: &Visit) {
        let ours = v.walking.map(|target| Follow { target, stop: STOP });
        if ours.is_some() && self.follow == ours {
            self.follow = None;
            self.steering.reset();
        }
    }

    /// Per frame: make the journey, walk the last stretch, and use the
    /// person on arriving.
    pub(crate) fn tick_visit(&mut self, now: Instant) {
        let Some(mut v) = self.visits.current.take() else {
            return;
        };
        let Some(me) = self.my_position() else {
            self.visits.current = Some(v);
            return;
        };
        // Through the portal: its mouth took the character on the way in.
        // Not while a journey is under way, whose own portals carry the
        // character off on the way there.
        if v.through && !self.traveling() && carried_off(v.last_at, me) {
            tracing::info!("visit: through {}", v.label);
            self.visits.finished = true;
            self.let_go_of(&v);
            return;
        }
        v.last_at = Some(me);
        // Once in sight, where they really stand beats the gazetteer.
        if let Some((guid, at, cell)) = self.visit_person(&v) {
            if v.guid != Some(guid) {
                tracing::info!("visit: {} is in view ({guid:#010x})", v.label);
            }
            v.guid = Some(guid);
            v.goal = at;
            v.cell = cell;
        }
        let traveling = self.traveling();
        if v.stalled(me.distance(v.goal), traveling, now) {
            tracing::warn!(
                "visit: no nearer to {} in {GIVE_UP:?}; giving up ({:.1} m away)",
                v.label,
                me.distance(v.goal)
            );
            self.let_go_of(&v);
            return;
        }
        match next(me, v.goal, traveling, v.journeyed) {
            Next::OnTheWay => {}
            Next::Journey => {
                v.journeyed = true;
                if !self.plan_trip(v.goal.truncate()) {
                    tracing::warn!("visit: no way to {} from here", v.label);
                    self.let_go_of(&v);
                    return;
                }
            }
            Next::GiveUp => {
                tracing::warn!("visit: the journey to {} ended short of it", v.label);
                self.let_go_of(&v);
                return;
            }
            Next::Walk => {
                let ours = v.walking.map(|target| Follow { target, stop: STOP });
                if ours.is_some() && self.follow.is_some() && self.follow != ours {
                    tracing::info!("visit: {} dropped: something else is steering", v.label);
                    return;
                }
                // Walked afresh when nothing is (a sidestep out of a
                // spell's way lets go of the walk) or when the person
                // has moved.
                let moved = v.walking.is_none_or(|w| w.distance(v.goal) > 0.5);
                if self.follow.is_none() || moved {
                    let did = self.head_toward(v.goal, STOP, &v.label);
                    if !did.fine() {
                        tracing::warn!("visit: cannot walk to {}", v.label);
                        self.let_go_of(&v);
                        return;
                    }
                    v.walking = Some(v.goal);
                }
            }
            Next::Arrived => {
                // Standing where they should be with nobody of that name
                // sent yet: stop walking and give the server a moment.
                if v.person.is_some()
                    && v.guid.is_none()
                    && waits_for_them(*v.arrived_at.get_or_insert(now), now)
                {
                    self.let_go_of(&v);
                    self.visits.current = Some(v);
                    return;
                }
                self.finish_visit(v);
                return;
            }
        }
        self.visits.current = Some(v);
    }

    /// On their floor and close: stop, and use them.
    fn finish_visit(&mut self, v: Visit) {
        self.visits.finished = true;
        self.let_go_of(&v);
        if self.traveling() {
            self.end_trip();
        }
        if v.person.is_none() && v.guid.is_none() {
            tracing::info!("visit: at {}", v.label);
            return;
        }
        let Some(guid) = v.guid else {
            tracing::info!(
                "visit: at {}'s spot but nobody of that name is in view; stopping",
                v.label
            );
            return;
        };
        let wanted = v.by_hand
            || self
                .world
                .objects
                .get(&guid)
                .is_some_and(someone_to_talk_to);
        if !wanted {
            tracing::info!("visit: at {}, which is not someone to talk to", v.label);
            return;
        }
        tracing::info!("visit: at {}; using {guid:#010x}", v.label);
        self.interact(guid);
        if v.through {
            self.visits.portal_used = Some((v.label.clone(), Instant::now()));
        }
        // This use was the visit's: should the server's walk for it run
        // out too, it is not carried on again.
        self.visits.last_use = None;
    }

    /// The person the visit is for, when the server has sent them: by
    /// guid when known, else the one of that name nearest the goal.
    /// Their world position and cell.
    fn visit_person(&self, v: &Visit) -> Option<(u32, Vec3, u32)> {
        let place = |o: &WorldObject| {
            let p = o.display.or(o.position)?;
            Some((ac_world::landblock_origin(p.cell) + p.local, p.cell))
        };
        if let Some(guid) = v.guid {
            let (at, cell) = place(self.world.objects.get(&guid)?)?;
            return Some((guid, at, cell));
        }
        let name = v.person.as_deref()?;
        let me = self.world.player_guid;
        self.world
            .objects
            .values()
            .filter(|o| {
                Some(o.guid) != me
                    && o.container.is_none()
                    && o.wielder.is_none()
                    && o.name.eq_ignore_ascii_case(name)
            })
            .filter_map(|o| place(o).map(|(at, cell)| (o.guid, at, cell)))
            .filter(|(_, at, _)| at.distance(v.goal) <= NAMED_NEAR)
            .min_by(|a, b| a.1.distance(v.goal).total_cmp(&b.1.distance(v.goal)))
    }

    /// The client has let a server walk go (see `tick_player`). When it
    /// was walking to something used by hand that the server has not
    /// answered, and the character is not yet in reach of it, the use
    /// would be lost without a word: walk the rest as a visit and use it
    /// on arriving.
    pub(crate) fn server_walk_ran_out(&mut self, target: Option<MoveTarget>, now: Instant) {
        let Some(MoveTarget::Object(guid)) = target else {
            return;
        };
        if !continues(self.visits.last_use, guid, now) {
            return;
        }
        self.visits.last_use = None;
        let Some(o) = self.world.objects.get(&guid) else {
            return;
        };
        let Some(p) = o.display.or(o.position) else {
            return;
        };
        let name = o.name.clone();
        let at = ac_world::landblock_origin(p.cell) + p.local;
        let Some(me) = self.my_position() else {
            return;
        };
        if arrived(me, at) {
            return;
        }
        tracing::info!(
            "visit: the server's walk to {name} ran out {:.1} m short; walking the rest",
            me.distance(at)
        );
        let mut v = Visit::new(name, at, p.cell, None, Some(guid));
        v.by_hand = true;
        self.start_visit(v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_world::landmarks::Kind;

    fn cindrue() -> Landmark {
        Landmark {
            kind: Kind::Vendor,
            name: "Archmage Cindrue".into(),
            cell: 0xA9B4_011B,
            at: ac_world::landblock_origin(0xA9B4_011B) + Vec3::new(152.3, 132.5, 69.0),
        }
    }

    #[test]
    fn a_landmark_visit_keeps_its_cell_and_its_height() {
        let v = Visit::landmark(&cindrue());
        assert_eq!(v.cell, 0xA9B4_011B);
        assert_eq!(v.goal.z, 69.0);
        assert_eq!(v.goal, cindrue().at);
        // Someone is looked for on arriving; a lifestone is only reached.
        assert_eq!(v.person.as_deref(), Some("Archmage Cindrue"));
        assert!(!v.by_hand);
        let stone = Landmark {
            kind: Kind::Lifestone,
            ..cindrue()
        };
        assert_eq!(Visit::landmark(&stone).person, None);
    }

    #[test]
    fn a_portal_visit_goes_to_its_mouth_and_takes_it() {
        let p = ac_world::portals::named("Holtburg Dungeon")
            .into_iter()
            .find(|p| p.name == "Holtburg Dungeon" && p.mouth_outdoors())
            .expect("the Holtburg Dungeon portal is in the data");
        let v = Visit::portal(p);
        assert_eq!(v.goal, p.from);
        assert_eq!(v.cell, p.from_cell);
        // Found by name on arriving, and used whatever it is.
        assert_eq!(v.person.as_deref(), Some("Holtburg Dungeon"));
        assert!(v.by_hand && v.through);
        // A landmark is not gone through.
        assert!(!Visit::landmark(&cindrue()).through);
    }

    #[test]
    fn arriving_before_they_are_sent_waits_a_moment() {
        let t0 = Instant::now();
        let s = Duration::from_secs;
        // Just got there: the portal may not have been sent yet.
        assert!(waits_for_them(t0, t0));
        assert!(waits_for_them(t0, t0 + s(4)));
        // Long enough: nobody of that name is coming.
        assert!(!waits_for_them(t0, t0 + s(6)));
    }

    #[test]
    fn a_portal_just_used_is_left_to_take_the_character() {
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let used = ("Holtburg Dungeon".to_string(), t0);
        // Asked again in the frames after the use: the same visit.
        assert!(still_taking(Some(&used), "Holtburg Dungeon", t0));
        assert!(still_taking(Some(&used), "Holtburg Dungeon", t0 + s(4)));
        // Another portal, nothing used, or long enough that the use failed.
        assert!(!still_taking(Some(&used), "Holtburg Town", t0 + s(1)));
        assert!(!still_taking(None, "Holtburg Dungeon", t0));
        assert!(!still_taking(Some(&used), "Holtburg Dungeon", t0 + s(6)));
    }

    #[test]
    fn being_carried_off_is_a_jump_no_walk_makes() {
        let here = Vec3::new(32380.0, 34923.0, 28.0);
        assert!(!carried_off(None, here));
        // A frame's stride, even a fast one.
        assert!(!carried_off(Some(here), here + Vec3::new(2.5, 0.0, 0.0)));
        // Into a dungeon: another landblock and far below.
        assert!(carried_off(Some(here), Vec3::new(200.0, 47000.0, -10.0)));
    }

    #[test]
    fn far_is_a_journey_near_is_a_walk_and_her_floor_is_the_use() {
        let her = cindrue().at;
        let far = her + Vec3::new(2000.0, 0.0, -3.0);
        assert_eq!(next(far, her, false, false), Next::Journey);
        assert_eq!(next(far, her, true, true), Next::OnTheWay);
        // The journey is over and she is still far: it ended short.
        assert_eq!(next(far, her, false, true), Next::GiveUp);
        // Across the town: walk, even with the journey's last step still
        // being made -- the walk knows her height and the journey does not.
        let street = her + Vec3::new(140.0, 0.0, -3.0);
        assert_eq!(next(street, her, false, false), Next::Walk);
        assert_eq!(next(street, her, true, true), Next::Walk);
        // Inside the door, right under her: close on the map, a floor
        // below. Keep walking; a use from here would never climb.
        let under = her - Vec3::new(0.5, 0.0, 3.0);
        assert_eq!(next(under, her, false, true), Next::Walk);
        // Up the stairs and beside her.
        let beside = her + Vec3::new(1.2, 0.8, 0.1);
        assert_eq!(next(beside, her, false, true), Next::Arrived);
        // On her floor but across the room: not yet.
        let across = her + Vec3::new(6.0, 0.0, 0.0);
        assert_eq!(next(across, her, false, true), Next::Walk);
    }

    #[test]
    fn a_visit_that_comes_no_nearer_is_given_up() {
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let mut v = Visit::landmark(&cindrue());
        assert!(!v.stalled(50.0, false, t0));
        // Nearer: the clock starts again.
        assert!(!v.stalled(30.0, false, t0 + s(50)));
        assert!(!v.stalled(30.0, false, t0 + s(100)));
        // A pace to and fro is not progress.
        assert!(!v.stalled(29.5, false, t0 + s(105)));
        assert!(v.stalled(29.5, false, t0 + s(111)));
        // A journey under way is never stalled, and its end starts afresh.
        let mut v = Visit::landmark(&cindrue());
        assert!(!v.stalled(3000.0, true, t0));
        assert!(!v.stalled(3000.0, true, t0 + s(500)));
        assert!(!v.stalled(3000.0, false, t0 + s(501)));
        assert!(!v.stalled(3000.0, false, t0 + s(560)));
        assert!(v.stalled(3000.0, false, t0 + s(562)));
    }

    #[test]
    fn only_an_unanswered_recent_use_by_hand_is_carried_on() {
        let t0 = Instant::now();
        let s = Duration::from_secs;
        // The walk ran out twelve seconds after the use, unanswered:
        // carried on, and the use sent again on arriving.
        let mut uses = Visits::default();
        uses.used(7, t0);
        assert!(continues(uses.last_use, 7, t0 + s(12)));
        // Walking to something else, or long ago, or nothing used.
        assert!(!continues(uses.last_use, 8, t0 + s(12)));
        assert!(!continues(uses.last_use, 7, t0 + s(60)));
        assert!(!continues(None, 7, t0 + s(12)));
        // UseDone before the cut-off: the server did the use (from under
        // her floor, through the ceiling) and nothing is carried on --
        // that would climb up and talk to her twice.
        uses.answered();
        assert!(!continues(uses.last_use, 7, t0 + s(12)));
        // A new use afterwards is remembered again.
        uses.used(7, t0 + s(30));
        assert!(continues(uses.last_use, 7, t0 + s(42)));
    }

    #[test]
    fn only_someone_is_spoken_to_on_arriving() {
        use ac_world::{item_type, object_desc_flags as f};
        let thing = |item: u32, flags: u32| WorldObject {
            item_type: item,
            object_desc_flags: flags,
            ..Default::default()
        };
        // A shopkeeper, and a townsman who cannot be fought.
        assert!(someone_to_talk_to(&thing(
            item_type::CREATURE,
            f::STUCK | f::VENDOR
        )));
        assert!(someone_to_talk_to(&thing(item_type::CREATURE, f::STUCK)));
        // A Wailing Statue, an Exploration Marker: not creatures.
        assert!(!someone_to_talk_to(&thing(item_type::MISC, f::STUCK)));
        // Something to fight (a use in combat mode is an attack), and
        // another player (a use opens a trade).
        assert!(!someone_to_talk_to(&thing(
            item_type::CREATURE,
            f::ATTACKABLE
        )));
        assert!(!someone_to_talk_to(&thing(item_type::CREATURE, f::PLAYER)));
    }
}
