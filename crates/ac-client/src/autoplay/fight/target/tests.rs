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
    assert_eq!(archer.combat_stance(), Stance::Missile);
    assert_eq!(
        archer.attack_kind(Stance::Missile, &[], 0),
        Some(How::Missile)
    );
    let mut swordsman = testkit::character_of_level(testkit::no_data(), 20);
    testkit::a_weapon(
        &mut swordsman,
        SWORD,
        ac_world::item_type::MELEE_WEAPON,
        "Shortsword",
        true,
    );
    assert_eq!(swordsman.combat_stance(), Stance::Melee);
    assert_eq!(
        swordsman.attack_kind(Stance::Melee, &[], 0),
        Some(How::Melee)
    );
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

/// An Ice Golem standing at `at`, in the character's own landblock: the
/// element table has it take nothing at all from cold and full damage
/// from fire (wcid 196, `crates/ac-world/data/creatures.csv`).
fn ice_golem_at(c: &mut Client, guid: u32, at: glam::Vec3) {
    let cell = c.player.as_ref().expect("standing somewhere").cell;
    let o = ac_world::WorldObject {
        weenie_class_id: 196,
        object_desc_flags: ac_world::object_desc_flags::ATTACKABLE,
        position: testkit::placed(cell, at),
        ..testkit::creature(guid, "Ice Golem")
    };
    c.world.objects.insert(guid, o);
}

/// The world point of the open ground at landblock-local `(x, y)` in
/// `cell`'s block: creatures stand on it, so a rise between two of them
/// is the rise their attacker's shots have to get over.
fn ground_at(c: &Client, cell: u32, x: f32, y: f32) -> glam::Vec3 {
    let id = (cell & 0xFFFF_0000) | 0xFFFF;
    let bytes = c.assets.cell.read(id).expect("the landblock's cells");
    let lb = ac_formats::landblock::CellLandblock::parse(id, &bytes).expect("a landblock");
    let region = c.assets.region().expect("the region");
    let z = ac_scene::scenery::TerrainSampler::new(&lb, &region.land_defs.land_height_table)
        .height_at(glam::vec3(x, y, 0.0))
        .expect("inside the block");
    ac_world::landblock_origin(cell) + glam::vec3(x, y, z)
}

/// A cell of the Holtburg landblock whose ground rises between local
/// (96, 96) and (124.2, 106.3): a bolt thrown at something standing
/// there strikes the rise, an arc is lobbed over it.
const HILLSIDE: u32 = 0xA9B4_0019;

/// A level 20 war mage standing on the [`HILLSIDE`] at (96, 96), with
/// `spells` in its book, the components for them and the mana to pay:
/// `can_cast` says `Ok` to each, so all of them are
/// [`Client::ready_spells`].
fn a_war_mage_on_the_hill(spells: &[u32]) -> Client {
    let mut c = testkit::character_of_level(testkit::game_data(), 20);
    testkit::as_a_war_mage(&mut c);
    c.world.stats.spells = spells.to_vec();
    // A level-one war spell's foci formula is one lead scarab and one
    // prismatic taper (`magic::foci_formula`).
    let scarab = testkit::component_named(&c, "Lead Scarab");
    testkit::component_in_the_pack(&mut c, 0x8000_0050, "Lead Scarab", scarab, 50);
    testkit::tapers_in_the_pack(&mut c, 0x8000_0051, 50);
    c.world.stats.vitals[2].current = 500;
    let at = ground_at(&c, HILLSIDE, 96.0, 96.0);
    testkit::stand(&mut c, HILLSIDE, at - ac_world::landblock_origin(HILLSIDE));
    for id in spells {
        assert_eq!(c.can_cast(*id), crate::magic::CastCheck::Ok, "spell {id}");
    }
    c
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_caster_picks_what_its_arc_gets_over_not_what_a_swing_would_reach() {
    use crate::aim::Shot;
    // Frost Arc I, whose projectile does not track and is lobbed.
    const FROST_ARC_I: u32 = 2725;
    const BEHIND_THE_RISE: u32 = 0x8000_0202;
    const IN_THE_OPEN: u32 = 0x8000_0203;
    let mut c = a_war_mage_on_the_hill(&[FROST_ARC_I]);
    // Magic stance clears the missile combat mode, so reading that flag
    // asked whether the caster could swing at the creature instead.
    assert_eq!(c.combat_stance(), Stance::Magic);
    assert!(!c.missile);
    let how = c.attack_kind(Stance::Magic, &[FROST_ARC_I], 0);
    assert_eq!(how, Some(How::Spell(FROST_ARC_I)));
    assert!(matches!(c.shot_for(how.unwrap()), Shot::Arc { .. }));
    assert_eq!(c.shot_for(How::Melee), Shot::Bolt, "a swing's line is not");
    let near = ground_at(&c, HILLSIDE, 124.2, 106.3);
    let far = ground_at(&c, HILLSIDE, 96.0, 60.0);
    revenant_at(&mut c, BEHIND_THE_RISE, near);
    revenant_at(&mut c, IN_THE_OPEN, far);
    let me = c.my_position().expect("standing somewhere");
    assert!(near.distance(me) < far.distance(me), "and it is the nearer");
    assert!(!c.shot_clears(BEHIND_THE_RISE, How::Melee), "a bolt's line");
    assert!(c.shot_clears(BEHIND_THE_RISE, How::Spell(FROST_ARC_I)));
    assert!(c.shot_clears(IN_THE_OPEN, How::Melee));
    let cfg = Fight {
        radius: 45.0,
        ..Fight::default()
    };
    assert_eq!(
        c.pick_target(&cfg),
        Some(BEHIND_THE_RISE),
        "the arc gets over the rise, so the nearer one is in sight"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_pick_traces_the_spell_this_creature_will_be_thrown() {
    use crate::aim::Shot;
    const FROST_ARC_I: u32 = 2725;
    const FLAME_BOLT_I: u32 = 27;
    const BEHIND_THE_RISE: u32 = 0x8000_0202;
    const IN_THE_OPEN: u32 = 0x8000_0203;
    // Two ready spells, the arc first in the rules and so the first the
    // book offers.
    let cfg = Fight {
        radius: 45.0,
        spells: vec!["Frost Arc I".into(), "Flame Bolt I".into()],
        ..Fight::default()
    };
    let mut c = a_war_mage_on_the_hill(&[FROST_ARC_I, FLAME_BOLT_I]);
    let ready: Vec<u32> = c.ready_spells(&cfg).into_iter().map(|(id, _)| id).collect();
    assert_eq!(ready, vec![FROST_ARC_I, FLAME_BOLT_I]);
    // An Ice Golem takes nothing from cold, so the cast throws the
    // bolt at it however the rules are ordered.
    assert_eq!(
        ac_world::elements::best_spell(196, "Ice Golem", &ready).map(|(id, _)| id),
        Some(FLAME_BOLT_I)
    );
    assert_eq!(
        c.attack_kind(Stance::Magic, &ready, 0),
        Some(How::Spell(FROST_ARC_I)),
        "with no creature to ask about, the first ready one"
    );
    let near = ground_at(&c, HILLSIDE, 124.2, 106.3);
    let far = ground_at(&c, HILLSIDE, 96.0, 60.0);
    ice_golem_at(&mut c, BEHIND_THE_RISE, near);
    ice_golem_at(&mut c, IN_THE_OPEN, far);
    // The sight test asks about the bolt, because the bolt is what
    // this creature is going to be thrown.
    let how = c.attack_kind(Stance::Magic, &ready, BEHIND_THE_RISE);
    assert_eq!(how, Some(How::Spell(FLAME_BOLT_I)));
    assert_eq!(c.shot_for(how.unwrap()), Shot::Bolt);
    assert!(!c.shot_clears(BEHIND_THE_RISE, how.unwrap()));
    assert!(
        c.shot_clears(BEHIND_THE_RISE, How::Spell(FROST_ARC_I)),
        "the arc the caster leads with would have got over"
    );
    assert_eq!(
        c.pick_target(&cfg),
        Some(IN_THE_OPEN),
        "naming one spell for the whole pick would have walked it round the rise"
    );
}

#[test]
fn a_caster_with_nothing_to_throw_names_no_attack() {
    const WAND: u32 = 0x8000_0201;
    const NEAR: u32 = 0x8000_0202;
    const FAR: u32 = 0x8000_0203;
    const HOLTBURG: u32 = 0xA9B4_0019;
    let mut c = testkit::character_of_level(testkit::no_data(), 20);
    testkit::stand(&mut c, HOLTBURG, glam::vec3(84.0, 108.0, 94.0));
    testkit::a_weapon(&mut c, WAND, ac_world::item_type::CASTER, "Wand", true);
    assert_eq!(c.combat_stance(), Stance::Magic);
    // Out of mana, out of components or nothing learnt: there is no
    // attack to test, and a swing is not the answer -- `How::Melee`
    // would hand the fight rules a reach of `dodge::STICKY_REACH`.
    assert!(c.ready_spells(&Fight::default()).is_empty());
    assert_eq!(c.attack_kind(Stance::Magic, &[], NEAR), None);
    let me = c.my_position().expect("standing somewhere");
    revenant_at(&mut c, FAR, me + glam::vec3(30.0, 0.0, 0.0));
    revenant_at(&mut c, NEAR, me + glam::vec3(10.0, 0.0, 0.0));
    let cfg = Fight {
        radius: 40.0,
        ..Fight::default()
    };
    assert_eq!(c.pick_target(&cfg), Some(NEAR), "and then the nearest");
}
