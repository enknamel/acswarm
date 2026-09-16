use std::time::{Duration, Instant};

use crate::{pack, SAME_FLOOR};

/// How long the client stands aside for a walk the server is making for
/// it before it takes the controls back (see [`standing_aside`]).
const SERVER_WALK_FOR: Duration = Duration::from_secs(12);

/// What a movement event for our own character did to the server walk
/// the client is carrying out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum ServerWalk {
    /// A walk began where none was under way.
    Began(ac_world::object::MoveTarget),
    /// The walk under way is over.
    Ended,
    /// A walk under way goes on, or none was under way and none began.
    Unchanged,
}

/// Carry a movement event for our own character over to the server walk
/// (`walk`, under way since `since`, `answered` once the server has
/// answered what it was for). `target` is where the event walks us; a
/// plain motion or a turn has none. A walk sent anew has had no answer
/// yet.
///
/// Only a walk is stood aside for. A turn to face something in reach
/// used to count as a walk to it, and nothing the server sends ends a
/// turn: after emptying a corpse the character stood beside it for
/// twelve seconds, its walk to the next corpse overruled by a walk that
/// was never coming.
pub(crate) fn heard_server_walk(
    walk: &mut Option<ac_world::object::MoveTarget>,
    since: &mut Instant,
    answered: &mut bool,
    target: Option<ac_world::object::MoveTarget>,
    now: Instant,
) -> ServerWalk {
    match (walk.is_some(), target) {
        (false, Some(t)) => {
            *walk = Some(t);
            *since = now;
            *answered = false;
            ServerWalk::Began(t)
        }
        (true, Some(t)) => {
            *walk = Some(t);
            *answered = false;
            ServerWalk::Unchanged
        }
        (true, None) => {
            *walk = None;
            ServerWalk::Ended
        }
        (false, None) => ServerWalk::Unchanged,
    }
}

/// Whether the client stands aside for a server walk at `now`: it sends
/// no MoveToState of its own, which ACE takes for calling the walk off
/// (and the use it is for), until the walk ends or has gone on for
/// [`SERVER_WALK_FOR`].
pub(super) fn standing_aside(
    walk: Option<ac_world::object::MoveTarget>,
    since: Instant,
    now: Instant,
) -> bool {
    walk.is_some() && now.saturating_duration_since(since) < SERVER_WALK_FOR
}

/// Whether a character at `me` has reached a goal at `at`: within `stop`
/// of it on the flat, and on its floor. Where the steering stops.
pub(super) fn reached(me: glam::Vec3, at: glam::Vec3, stop: f32) -> bool {
    let d = at - me;
    glam::Vec2::new(d.x, d.y).length() <= stop && d.z.abs() <= SAME_FLOOR
}

/// Whether a server walk (`walk`) is over though the server will say no
/// more about it: the server has answered what it was for (`answered`),
/// and the character stands at the walk's `goal` (a place and how close
/// is there), or has no goal to walk to because what it walked to is out
/// of view.
///
/// ACE ends a walk for a use on its own side. Arriving runs the use and
/// it sends the answer, but no motion, so the client stood aside for the
/// rest of its twelve seconds, and whatever came next (the next corpse,
/// the walk home) went nowhere. The answer alone is not enough: a
/// double-click sent again while walking has the first one answered
/// then, and letting go on that stopped the character a stride into
/// every walk. Standing there is what makes an answer the last word:
/// the server has seen the character in reach, or never will.
pub(crate) fn server_walk_over(
    walk: Option<ac_world::object::MoveTarget>,
    goal: Option<(glam::Vec3, f32)>,
    me: glam::Vec3,
    answered: bool,
) -> bool {
    walk.is_some() && answered && goal.is_none_or(|(at, stop)| reached(me, at, stop))
}

/// Whether an AttackDone ends the server walk `walk`: it was a walk to
/// `attacked`, the creature the last attack was sent at.
///
/// A charge the server gives up on ("You charged too far", the target
/// gone) is answered with a weenie error and AttackDone, and no motion.
/// Left standing, the walk kept the character still for twelve seconds
/// where the charge stopped, fighting nothing, while the creature went
/// on hitting it.
pub(crate) fn attack_ended_walk(
    walk: Option<ac_world::object::MoveTarget>,
    attacked: Option<u32>,
) -> bool {
    matches!(
        (walk, attacked),
        (Some(ac_world::object::MoveTarget::Object(g)), Some(a)) if g == a
    )
}

/// The longest an unanswered attack keeps the character's hands to
/// itself. ACE answers every swing with AttackDone, so this only
/// matters when the answer goes missing: one lost message must not
/// leave the hands untouchable for the rest of the session.
const ATTACK_ANSWERED_WITHIN: Duration = Duration::from_secs(10);

/// Whether a swing or a charge is out and still unanswered, so that the
/// character's hands and its combat mode must be left alone.
///
/// ACE turns every combat-mode change into HandleActionCancelAttack,
/// which cancels the walk the charge is riding on and answers with
/// AttackDone -- and wielding a weapon of another kind is a combat-mode
/// change. +Verity's in-fight buffing reached for her wand every second
/// and a half, and every reach put her mace away and took the charge
/// down with it: a minute of "Action cancelled" and almost nothing
/// landed on the Drudge Servant that was hitting her throughout.
pub(crate) fn attack_unanswered(pending: bool, sent: Instant, now: Instant) -> bool {
    pending && now.duration_since(sent) < ATTACK_ANSWERED_WITHIN
}

/// Whether an answer from the server about the things in `about` is the
/// answer to the server walk `walk`: a refusal or a UseDone for what the
/// walk is for. A refused inventory action is about the item it names
/// and whatever holds it; a UseDone is about `used`, what the last use
/// was sent for. ACE walks to a portal's place rather than to the
/// portal, so a walk to a place is for `used` too.
///
/// A charge at `attacked` is never answered this way: AttackDone or a
/// motion ends it (see [`attack_ended_walk`]). Any answer at all used to
/// do. +Verity's in-fight buffing asked for her wand every second and a
/// half and was refused every time, and each refusal counted as the
/// answer to whichever charge was under way: once within a metre of the
/// creature she took the controls back, ran off towards her own goal
/// and was told "You charged too far".
pub(crate) fn answers_walk(
    walk: Option<ac_world::object::MoveTarget>,
    about: &[u32],
    used: Option<u32>,
    attacked: Option<u32>,
) -> bool {
    let walked_for = match walk {
        Some(ac_world::object::MoveTarget::Object(g)) => Some(g),
        Some(ac_world::object::MoveTarget::Position { .. }) => used,
        None => None,
    };
    walked_for.is_some_and(|g| Some(g) != attacked && about.contains(&g))
}

/// Whether a refused inventory action naming `item` is the answer to
/// the pour the tidying has out, rather than an answer to anything else.
///
/// A pour is two stacks already in the pack, so its refusal is never
/// about a walk: a walk is for something out in the world. It has to be
/// said, though, because a looted stack keeps the guid it was fetched
/// under -- so the refusal of a pour of that stack looks exactly like
/// the refusal of the pickup the character is still walking for, and
/// taking the controls back on it stopped the walk a stride in (see
/// [`answers_walk`]).
///
/// Only while the pour is still in the air. The settler is housekeeping
/// and housekeeping does not run with autoplay off, or while the
/// character is dodging, dying or in the Academy -- so a pour sent a
/// moment before any of those stays recorded for as long as the
/// character stands there, and without this it went on claiming every
/// refusal those two guids were ever named in. [`pack::POUR_LOST`] is
/// the same wall the settler gives up at.
pub(crate) fn answers_a_pour(
    pour: Option<&(pack::PourSent, Instant)>,
    item: u32,
    now: Instant,
) -> bool {
    pour.is_some_and(|(p, at)| {
        now.saturating_duration_since(*at) < pack::POUR_LOST
            && (p.merge.from == item || p.merge.to == item)
    })
}

#[cfg(test)]
mod tests;
