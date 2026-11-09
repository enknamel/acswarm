//! A gem step's two decisions, standing where the gem is used: which way to face for room to
//! summon, and what to do next from what the pack and the summoned portal show.

use std::time::{Duration, Instant};

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
/// The cooldown every portal gem shares, as the enchantment ACE puts on the user: 0x8000 with
/// CooldownId 1000 (EnchantmentManager.cs:284-306, 1186), 15 s; 483 of `gems.csv`'s 484.
const GEM_COOLDOWN: u16 = 0x8000 | 1000;
/// How far off the portal a gem summoned may stand. ACE puts it three metres in front of
/// whoever used the gem (WorldObject_Magic.cs:1257).
const SUMMONED_REACH: f32 = 8.0;

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

impl crate::Client {
    /// The portals within [`SUMMONED_REACH`], nearest first.
    pub(crate) fn portals_in_reach(&self) -> Vec<u32> {
        let Some(me) = self.my_position().map(|p| p.truncate()) else {
            return Vec::new();
        };
        let mut near: Vec<(u32, f32)> = self
            .world
            .objects
            .values()
            .filter(|o| o.item_type & ac_world::item_type::PORTAL != 0)
            .filter_map(|o| {
                let p = o.position?;
                let at = (ac_world::landblock_origin(p.cell) + p.local).truncate();
                let away = at.distance(me);
                (away <= SUMMONED_REACH).then_some((o.guid, away))
            })
            .collect();
        near.sort_by(|a, b| a.1.total_cmp(&b.1));
        near.into_iter().map(|(guid, _)| guid).collect()
    }

    /// The portal a gem has just put in front of the character: the nearest in reach that was
    /// not there, in `before`, as it was used (a dungeon's exit by its door is not it).
    pub(crate) fn summoned_portal(&self, before: &[u32]) -> Option<u32> {
        let near = self.portals_in_reach().into_iter();
        near.into_iter().find(|g| !before.contains(g))
    }

    /// A spell in the air, a hop not yet landed (even for a journey since put down), or the gems'
    /// cooldown running: a gem used now is turned away, its try spent.
    pub(crate) fn is_gem_held(&self, now: Instant) -> bool {
        self.autoplay.cast_in_flight(now) || self.is_gem_cooling()
    }

    /// Whether the gems' shared cooldown still runs: a gem used now is "used too recently"
    /// (WorldObject_Use.cs:327-336), and the step's tries are spent waiting it out.
    pub(crate) fn is_gem_cooling(&self) -> bool {
        let Some(now) = self.session.server_time() else {
            return false;
        };
        let stats = &self.world.stats;
        stats
            .enchantments
            .iter()
            .any(|e| e.spell_id == GEM_COOLDOWN && e.remaining(now).is_some_and(|l| l > 0.0))
    }

    /// How many of the gem `guid` the pack holds: 0 once the last is spent.
    pub(crate) fn gem_stack(&self, guid: u32) -> u32 {
        match self.world.objects.get(&guid) {
            Some(o) if self.world.is_carried(guid) => o.stack_size.max(1),
            _ => 0,
        }
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
