use super::*;
use crate::testkit::{a_weapon, game_data, mid_fight};

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn no_spell_is_sent_at_a_creature_that_has_already_died() {
    // 36 "Target not acquired" in a run, every one a cast at a
    // creature that had died since the tick that chose it: ACE looks
    // the target up before the windup and answers TargetNotAcquired.
    // The swing has always made this test; the cast never did.
    let (mut c, wand) = mid_fight(game_data());
    const MACE: u32 = 0x8000_0101;
    const CREATURE: u32 = 0x8000_0103;
    // Incantation of Lightning Vulnerability Other, one of the
    // softening spells the run cast.
    const VULNERABILITY: u32 = 4483;
    a_weapon(
        &mut c,
        MACE,
        ac_world::item_type::MELEE_WEAPON,
        "Mace",
        false,
    );
    a_weapon(&mut c, wand, ac_world::item_type::CASTER, "Wand", true);
    c.select(Some(CREATURE));
    assert_eq!(
        c.try_cast(VULNERABILITY),
        crate::magic::CastCheck::Ok,
        "it is alive and in view"
    );
    // Dead: the server replaces the creature with a corpse of
    // another guid, so its own simply goes.
    c.world.objects.remove(&CREATURE);
    assert_eq!(c.try_cast(VULNERABILITY), crate::magic::CastCheck::NoTarget);
    // And a cast that never went out earns no wait for an answer.
    // Refused, the server used to answer TargetNotAcquired with a
    // UseDone, which ended the wait; declined here, nothing comes
    // back at all, and the whole backstop would be spent standing
    // over a corpse the character was not allowed to take from.
    c.autoplay.cast_sent = None;
    c.cast_paced(VULNERABILITY, Instant::now());
    assert_eq!(c.autoplay.cast_sent, None, "nothing to wait for");
    assert!(!c.server_busy(Instant::now()));
}
