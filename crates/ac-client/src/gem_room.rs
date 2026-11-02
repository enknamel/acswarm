//! A gem step's two decisions, standing where the gem is used: which way to face for room to
//! summon, and what to do next from what the pack and the summoned portal show.

use std::time::Duration;

use glam::Vec2;

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

/// The way to face to use a gem: the way the character already faces
/// when that is clear, else the nearest clear heading to it, turning a
/// twelfth of the circle at a time either side. `None` when nowhere is.
pub(crate) fn clear_heading(forward: Vec2, mut clear: impl FnMut(Vec2) -> bool) -> Option<Vec2> {
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
pub(crate) enum GemNext {
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
pub(crate) fn gem_next(
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
}
