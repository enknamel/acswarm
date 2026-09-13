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
//! ([`Client::head_toward`]), which takes the character through the
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
    /// The player used it by hand, so it is used on arriving whatever it
    /// is. A visit to a gazetteer row speaks only to someone: see
    /// [`someone_to_talk_to`].
    by_hand: bool,
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

    /// What a visit is going to, while one is being made.
    pub fn visiting(&self) -> Option<&str> {
        self.visits.current.as_ref().map(|v| v.label.as_str())
    }

    fn start_visit(&mut self, v: Visit) -> bool {
        if self.player.is_none() {
            tracing::warn!("visit: the character is not in the world");
            return false;
        }
        self.drop_visit("another visit");
        tracing::info!(
            "visit: going to {} at {:?} in {:#010x}",
            v.label,
            v.goal,
            v.cell
        );
        self.visits.current = Some(v);
        self.tick_visit(Instant::now());
        self.visits.current.is_some()
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
        let Some(me) = self.player.as_ref().map(|p| p.world_position()) else {
            self.visits.current = Some(v);
            return;
        };
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
                self.finish_visit(v);
                return;
            }
        }
        self.visits.current = Some(v);
    }

    /// On their floor and close: stop, and use them.
    fn finish_visit(&mut self, v: Visit) {
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
        let Some(me) = self.player.as_ref().map(|p| p.world_position()) else {
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
