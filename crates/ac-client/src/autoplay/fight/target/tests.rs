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
    assert_eq!(c.pick_target(&cfg, Instant::now()), Some(ON_THE_FLAT));
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
        c.pick_target(&cfg, Instant::now()),
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
        c.pick_target(&cfg, Instant::now()),
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
    assert_eq!(
        c.pick_target(&cfg, Instant::now()),
        Some(NEAR),
        "and then the nearest"
    );
}

/// A level-20 character on its feet in the Holtburg field, on nobody's
/// team: the block the fleet is measured in.
fn in_the_field() -> Client {
    const HOLTBURG: u32 = 0xA9B4_0019;
    let mut c = testkit::character_of_level(testkit::no_data(), 20);
    testkit::stand(&mut c, HOLTBURG, glam::vec3(84.0, 108.0, 94.0));
    c
}

/// Put `c` behind a leader it follows, standing `metres` east of it.
fn following_a_leader(c: &mut Client, metres: f32) {
    let me = c.my_position().expect("on its feet");
    c.autoplay.config.team.enabled = true;
    c.autoplay.config.team.follow = true;
    let mut boss = testkit::mate(0x5000_0002, "Fleetbot One");
    boss.leader = true;
    boss.leads = true;
    boss.world = me + glam::vec3(metres, 0.0, 0.0);
    c.autoplay.team = testkit::view_of(vec![boss]);
}

#[test]
fn the_swing_and_the_spell_pick_the_same_creature() {
    // The melee and missile path scanned inline in `autoplay_fight_as`
    // for the plain nearest creature inside `Fight::radius`, with
    // neither the leader's radius nor sight, so a follower whose leader
    // hunted a field away turned and fought whatever wandered up to it
    // while a caster beside it, picking through `pick_target`, did not.
    let now = Instant::now();
    let cfg = Fight::default();

    // Alone, the nearest is the pick, and the swing goes at it.
    let mut c = in_the_field();
    let it = testkit::standing_by(&mut c, 0x8000_0001, "Revenant", 5.0).guid;
    assert_eq!(c.pick_target(&cfg, now), Some(it));
    assert!(c.autoplay_fight_as(now, &cfg));
    assert_eq!(c.attack_target, Some(it));

    // The same creature with a leader forty metres off stands
    // thirty-five from it, outside the team's twenty-five metre fight
    // radius: both paths pick nothing.
    let mut c = in_the_field();
    following_a_leader(&mut c, 40.0);
    testkit::standing_by(&mut c, 0x8000_0001, "Revenant", 5.0);
    assert_eq!(c.pick_target(&cfg, now), None);
    assert!(!c.autoplay_fight_as(now, &cfg), "the inline scan swung");
    assert_eq!(c.attack_target, None);

    // And beside its leader it fights it again.
    let mut c = in_the_field();
    following_a_leader(&mut c, 3.0);
    let it = testkit::standing_by(&mut c, 0x8000_0001, "Revenant", 5.0).guid;
    assert_eq!(c.pick_target(&cfg, now), Some(it));
    assert!(c.autoplay_fight_as(now, &cfg));
    assert_eq!(c.attack_target, Some(it));
}

#[test]
fn a_follower_out_of_reach_of_its_leader_picks_nothing_not_the_nearest() {
    // Deliberate, and the whole point of the leader's radius: falling
    // back to the nearest is the pick this replaces, so a fallback
    // would undo the change on exactly the picks that differ. Standing
    // down hands the tick to `catch up` (120) and `follow` (50), which
    // close the gap, and the same creature is inside the radius once
    // the follower is back beside its leader.
    let now = Instant::now();
    let cfg = Fight::default();
    let mut c = in_the_field();
    following_a_leader(&mut c, 40.0);
    let near = testkit::standing_by(&mut c, 0x8000_0001, "Revenant", 2.0).guid;
    let far = testkit::standing_by(&mut c, 0x8000_0002, "Revenant", 20.0).guid;
    assert_eq!(c.pick_target(&cfg, now), Some(far), "the one by the leader");

    // The leader out of reach of both: nothing is picked at all, though
    // a creature stands two metres off.
    following_a_leader(&mut c, 120.0);
    assert_eq!(c.pick_target(&cfg, now), None);
    assert!(!c.autoplay_fight_as(now, &cfg));

    // A fight already joined is another matter: `can_keep_target` never
    // asks where the leader stands, so one chased past that radius is
    // finished rather than dropped.
    c.attack_target = Some(near);
    assert!(c.can_keep_target(near, false, now));
    assert!(c.autoplay_fight_as(now, &cfg), "still fighting it");
    assert_eq!(c.attack_target, Some(near));
    assert_eq!(c.pick_target(&cfg, now), None, "and picks no other");
}

#[test]
fn something_hitting_the_follower_is_fought_though_the_leader_is_a_field_away() {
    // Every other walk-past rule yields to a creature already hitting
    // the character -- the hunting area (`area_allows`), the road
    // (`passing_by`), the critter rule (`a_critter`) -- and the
    // leader's radius is another walk-past rule. Asked of the fight
    // alone: in the table, catching up still comes first (below).
    let now = Instant::now();
    let cfg = Fight::default();
    let mut c = in_the_field();
    following_a_leader(&mut c, 40.0);
    let it = testkit::standing_by(&mut c, 0x8000_0001, "Revenant", 5.0).guid;
    assert_eq!(c.pick_target(&cfg, now), None, "nothing beside the leader");
    assert!(!c.autoplay_fight_as(now, &cfg));

    // The server names the attacker in what it sends, hit, miss or
    // spell (see `Client::hit_lately_by`). One picker, so this is the
    // swing's answer and the spell's alike.
    c.autoplay.attacked_by("Revenant", Instant::now());
    assert_eq!(c.pick_target(&cfg, now), Some(it));
    assert!(c.autoplay_fight_as(now, &cfg), "it swings back");
    assert_eq!(c.attack_target, Some(it));
}

/// `c` playing on its own with a Revenant `metres` east that has just attacked it.
fn hit_by_a_revenant(c: &mut Client, metres: f32) -> u32 {
    c.autoplay.config.enabled = true;
    let it = testkit::standing_by(c, 0x8000_0001, "Revenant", metres).guid;
    c.autoplay.attacked_by("Revenant", Instant::now());
    it
}

#[test]
fn a_follower_beside_its_leader_fights_back_at_what_hits_it_from_past_the_leaders_radius() {
    // The leader eight metres east, inside the catch-up break; the Revenant twenty west, so
    // twenty-eight from the leader and outside its radius. The fight has the tick, not `follow`.
    let mut c = in_the_field();
    following_a_leader(&mut c, 8.0);
    let it = hit_by_a_revenant(&mut c, -20.0);
    c.tick_autoplay(Instant::now());
    assert_eq!(c.autoplay.step, Some("fight"));
    assert_eq!(
        c.attack_target,
        Some(it),
        "it walked back to its leader under attack"
    );
}

#[test]
fn past_the_break_catching_up_still_comes_before_fighting_back() {
    // The user's call: a follower more than `follow_break` from its leader rejoins it first,
    // attacker or not (catch up, 120, over the fight, 80).
    let mut c = in_the_field();
    following_a_leader(&mut c, 40.0);
    hit_by_a_revenant(&mut c, 5.0);
    c.tick_autoplay(Instant::now());
    assert_eq!(c.autoplay.step, Some("catch up"));
    assert_eq!(c.attack_target, None);
    assert!(c.follow.is_some(), "not on its way back to the leader");
}

#[test]
fn a_stop_takes_the_fights_own_spell_back_and_stays_in_magic_mode() {
    // A cast already sent goes on to land unless something reaches it.
    // The character having been told to stop fighting, the combat-mode
    // change ACE turns into a failed cast is that something -- and the
    // mode sent is the one already held (`Client::stop_fight_cast`).
    let mut c = crate::testkit::offline_client();
    let sent = Instant::now();
    c.magic = true;
    c.autoplay.cast_sent = Some(sent);
    c.autoplay.fight_cast = Some((1, sent));
    c.autoplay.thrown = Some((1, sent));
    c.dodge.fired = Some((4483, sent, Vec::new()));
    c.take_up_fight(1);
    let before = c.session.actions_sent();
    c.let_go(Release::Stop);
    assert_eq!(
        c.session.actions_sent(),
        before + 1,
        "the combat mode went out"
    );
    assert!(
        c.magic,
        "and it is the one already held: no stance is spent"
    );
    assert_eq!(c.autoplay.fight_cast, None, "nothing left to take back");
    assert!(
        c.dodge.fired.is_none(),
        "a fizzled cast throws no projectile to read a speed off"
    );
    assert!(
        c.autoplay.thrown.is_none(),
        "and no shot of ours is on its way to that target"
    );
}

#[test]
fn a_re_target_leaves_the_spell_in_the_air_to_land() {
    // Taking the cast back costs the spell, so only a stop spends it.
    // A rule that goes on fighting -- a re-target, a creature given up
    // on -- is better off letting the spell land.
    let mut c = crate::testkit::offline_client();
    let sent = Instant::now();
    c.magic = true;
    c.autoplay.cast_sent = Some(sent);
    c.autoplay.fight_cast = Some((1, sent));
    c.take_up_fight(1);
    let before = c.session.actions_sent();
    c.let_go(Release::Fight);
    assert_eq!(c.session.actions_sent(), before, "nothing was sent");
    assert_eq!(
        c.autoplay.fight_cast,
        Some((1, sent)),
        "and the cast is still the fight's"
    );
    assert!(c.magic, "nor was the stance dropped");
}

#[test]
fn a_stop_leaves_a_heal_and_a_cast_already_answered_for_alone() {
    // A heal, a healing kit and a counter set the same clock. A stop
    // that fizzled one of those would take the heal a character lives by.
    let mut c = crate::testkit::offline_client();
    c.magic = true;
    c.autoplay.cast_sent = Some(Instant::now());
    c.autoplay.fight_cast = None;
    c.take_up_fight(1);
    let before = c.session.actions_sent();
    c.let_go(Release::Stop);
    assert_eq!(
        c.session.actions_sent(),
        before,
        "the heal was left to land"
    );

    // And a fight cast the server has answered for is spent: there is
    // nothing to take back and no message worth spending on it.
    c.autoplay.cast_sent = None;
    c.autoplay.fight_cast = Some((1, Instant::now()));
    c.take_up_fight(1);
    let before = c.session.actions_sent();
    c.let_go(Release::Stop);
    assert_eq!(c.session.actions_sent(), before, "the cast was spent");
    assert_eq!(c.autoplay.fight_cast, None);
}

#[test]
fn a_fight_cast_the_client_declined_is_not_the_heals_to_take_back() {
    // Without a wand `try_cast` sends nothing, so `cast_sent` keeps
    // whatever set it -- a heal's instant, here -- and the fight has no
    // cast of its own to take back.
    let mut c = crate::testkit::offline_client();
    const SPELL: u32 = 4483;
    c.world.stats.spells.push(SPELL);
    let heal = Instant::now();
    c.magic = true;
    c.autoplay.cast_sent = Some(heal);
    let before = c.session.actions_sent();
    assert!(
        !c.cast_fight_spell(SPELL, 1, Instant::now()),
        "no caster wielded, no cast"
    );
    assert_eq!(c.session.actions_sent(), before, "and nothing on the wire");
    assert_eq!(c.autoplay.fight_cast, None, "the heal's cast is not ours");
    assert_eq!(
        c.autoplay.cast_sent,
        Some(heal),
        "and its wait is untouched"
    );
    c.let_go(Release::Stop);
    assert_eq!(
        c.session.actions_sent(),
        before,
        "so the stop left it alone"
    );
}

#[test]
fn the_answer_that_frees_the_slot_ends_the_fights_own_cast_too() {
    let mut c = crate::testkit::offline_client();
    let sent = Instant::now();
    c.autoplay.cast_sent = Some(sent);
    c.autoplay.fight_cast = Some((1, sent));
    c.autoplay.cast_answered();
    assert_eq!(c.autoplay.cast_sent, None);
    assert_eq!(c.autoplay.fight_cast, None, "a spent cast cannot linger");
}
