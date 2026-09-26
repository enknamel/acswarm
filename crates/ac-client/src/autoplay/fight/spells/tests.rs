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

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_fights_spell_is_remembered_as_its_own_and_a_stop_takes_it_back() {
    let (mut c, wand) = mid_fight(game_data());
    const MACE: u32 = 0x8000_0101;
    const CREATURE: u32 = 0x8000_0103;
    // Incantation of Lightning Vulnerability Other, a targeted spell.
    const VULNERABILITY: u32 = 4483;
    a_weapon(
        &mut c,
        MACE,
        ac_world::item_type::MELEE_WEAPON,
        "Mace",
        false,
    );
    a_weapon(&mut c, wand, ac_world::item_type::CASTER, "Wand", true);
    let now = Instant::now();
    assert!(
        c.cast_fight_spell(VULNERABILITY, CREATURE, now),
        "a wand is wielded and the creature is there: it went out"
    );
    assert!(c.magic, "a cast enters magic mode");
    assert_eq!(c.autoplay.cast_sent, Some(now), "and waits for the answer");
    assert_eq!(
        c.autoplay.fight_cast,
        Some((CREATURE, now)),
        "the unanswered cast is the fight's own"
    );
    // The stop reaches it: ACE fails a cast still being wound up when
    // the combat mode changes (Player_Combat.cs:787-788).
    let before = c.session.actions_sent();
    c.let_go(Release::Stop);
    assert_eq!(
        c.session.actions_sent(),
        before + 1,
        "a spell was left to land after the stop"
    );
    assert!(c.magic, "the mode sent is the one already held");
    assert_eq!(c.autoplay.fight_cast, None, "nothing left to take back");
}

#[test]
fn a_target_the_server_cannot_find_is_forgotten() {
    // Blargerton cast Frost Arc III at a Drudge Servant the server no longer had near him: 44, 95
    // and 17 "Target not acquired" in a row, closing on it and backing off, a ghost on the screen.
    let now = Instant::now();
    let mut c = crate::testkit::offline_client();
    crate::testkit::stand(&mut c, 0xA9B4_0019, glam::Vec3::new(84.0, 84.0, 94.0));
    let ghost = crate::testkit::standing_by(&mut c, 0x8000_0001, "Drudge Servant", 5.0);
    c.autoplay.casting_at = Some(ghost.guid);
    c.autoplay.fight_cast = Some((ghost.guid, now));
    // Busy is not gone.
    c.hear_cast_refused(crate::YOURE_TOO_BUSY, now);
    assert!(c.world.objects.contains_key(&ghost.guid));
    assert_eq!(c.autoplay.casting_at(), Some(ghost.guid));
    c.hear_cast_refused(TARGET_NOT_ACQUIRED, now);
    assert!(
        !c.world.objects.contains_key(&ghost.guid),
        "the ghost is still in the world"
    );
    assert_eq!(c.autoplay.casting_at(), None, "still casting at it");
    assert!(c.autoplay.given_up.since(&ghost.guid).is_some());
}
