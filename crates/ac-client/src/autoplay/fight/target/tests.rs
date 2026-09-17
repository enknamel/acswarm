use super::*;
use crate::testkit;

#[test]
fn target_rules_read_names() {
    let mut f = Fight::default();
    assert!(wanted_target("Drudge Skulker", &f));
    f.only = vec!["drudge".into()];
    assert!(wanted_target("Drudge Skulker", &f));
    assert!(!wanted_target("Olthoi Grub", &f));
    f.only = vec!["  ".into()];
    assert!(wanted_target("anything", &f), "a blank rule means any");
    f.only = Vec::new();
    f.avoid = vec!["Olthoi".into()];
    assert!(!wanted_target("Olthoi Grub", &f));
    assert!(wanted_target("Drudge Skulker", &f));
    // Avoid wins over only.
    f.only = vec!["olthoi".into()];
    assert!(!wanted_target("Olthoi Grub", &f));
}

#[test]
fn each_release_lets_go_of_exactly_its_own_fields() {
    // The four shapes the call sites had before they shared one fn.
    let mut c = crate::testkit::offline_client();
    c.take_up_fight(1);
    c.let_go(Release::Cast);
    assert_eq!(
        c.fight_held(),
        (true, false, true, true),
        "the spell's target"
    );
    c.take_up_fight(1);
    c.let_go(Release::Targets);
    assert_eq!(c.fight_held(), (false, false, true, true), "both targets");
    c.take_up_fight(1);
    c.let_go(Release::Fight);
    assert_eq!(
        c.fight_held(),
        (false, false, false, false),
        "the whole fight"
    );
    c.take_up_fight(1);
    c.let_go(Release::Engagement);
    assert_eq!(
        c.fight_held(),
        (true, false, false, false),
        "the Academy's: the swing's target stays"
    );
}

/// A Revenant standing at `at`, in the character's own landblock: a real
/// fight at level 20, not a critter (see `testkit::standing_by`).
fn revenant_at(c: &mut Client, guid: u32, at: glam::Vec3) {
    let cell = c.player.as_ref().expect("standing somewhere").cell;
    let o = ac_world::WorldObject {
        weenie_class_id: 8592,
        object_desc_flags: ac_world::object_desc_flags::ATTACKABLE,
        position: testkit::placed(cell, at),
        ..testkit::creature(guid, "Revenant")
    };
    c.world.objects.insert(guid, o);
}

#[test]
fn the_sight_test_asks_about_the_attack_in_hand() {
    const BOW: u32 = 0x8000_0201;
    const SWORD: u32 = 0x8000_0202;
    let cfg = Fight::default();
    // The missile combat mode is entered on the first shot of a fight,
    // and the pick that chooses what to shoot comes before it.
    let mut archer = testkit::character_of_level(testkit::no_data(), 20);
    testkit::a_weapon(
        &mut archer,
        BOW,
        ac_world::item_type::MISSILE_WEAPON,
        "Yumi",
        true,
    );
    assert!(!archer.missile);
    assert_eq!(archer.attack_kind(&cfg), How::Missile);
    let mut swordsman = testkit::character_of_level(testkit::no_data(), 20);
    testkit::a_weapon(
        &mut swordsman,
        SWORD,
        ac_world::item_type::MELEE_WEAPON,
        "Shortsword",
        true,
    );
    assert_eq!(swordsman.attack_kind(&cfg), How::Melee);
}

#[test]
fn an_archer_picks_what_an_arrow_reaches_not_what_a_bolt_would() {
    const HOLTBURG: u32 = 0xA9B4_0019;
    const BOW: u32 = 0x8000_0201;
    const ON_THE_WALL: u32 = 0x8000_0202;
    const ON_THE_FLAT: u32 = 0x8000_0203;
    let mut c = testkit::character_of_level(testkit::no_data(), 20);
    testkit::stand(&mut c, HOLTBURG, glam::vec3(84.0, 108.0, 94.0));
    testkit::a_weapon(
        &mut c,
        BOW,
        ac_world::item_type::MISSILE_WEAPON,
        "Yumi",
        true,
    );
    assert_eq!(c.combat_stance(), Stance::Missile);
    assert!(!c.missile, "the first pick comes before `enter_combat`");
    let me = c.my_position().expect("standing somewhere");
    // Twenty-five metres up and four out, on the wall over the archer's
    // head: no arrow leaving a 20 m/s launcher gets there, though it is
    // the nearer of the two by seven metres.
    revenant_at(&mut c, ON_THE_WALL, me + glam::vec3(4.0, 0.0, 25.0));
    revenant_at(&mut c, ON_THE_FLAT, me + glam::vec3(32.0, 0.0, 0.0));
    assert!(!c.shot_clears(ON_THE_WALL, How::Missile));
    assert!(c.shot_clears(ON_THE_FLAT, How::Missile));
    assert!(
        c.shot_clears(ON_THE_WALL, How::Melee),
        "a swing's line flies straight, and nothing is in the way of it"
    );
    let cfg = Fight {
        radius: 40.0,
        ..Fight::default()
    };
    assert_eq!(c.pick_target(&cfg), Some(ON_THE_FLAT));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_casters_pick_traces_its_arc_where_a_swing_traces_a_bolt() {
    use crate::aim::Shot;
    // Flame Arc I, whose projectile does not track and is lobbed: it
    // goes over a wall a bolt strikes (see `crate::aim`).
    const FLAME_ARC_I: u32 = 2739;
    let mut c = testkit::character_of_level(testkit::game_data(), 20);
    testkit::as_a_war_mage(&mut c);
    c.world.stats.spells = vec![FLAME_ARC_I];
    let cfg = Fight::default();
    // Magic stance clears the missile combat mode, so reading that flag
    // asked whether the caster could swing at the creature instead.
    assert_eq!(c.combat_stance(), Stance::Magic);
    assert!(!c.missile);
    assert_eq!(c.attack_kind(&cfg), How::Spell(FLAME_ARC_I));
    assert!(
        matches!(c.shot_for(c.attack_kind(&cfg)), Shot::Arc { .. }),
        "the pick follows the arc the caster would throw"
    );
    assert_eq!(c.shot_for(How::Melee), Shot::Bolt, "a swing's line is not");
}
