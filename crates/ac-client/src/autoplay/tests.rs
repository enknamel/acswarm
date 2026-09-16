use crate::autoplay::hands::weapon::{SWAP_SETTLES, WIELD_ANSWERS_IN};
use crate::autoplay::ledger::salvage::{SALVAGE_TIMEOUT, SALVAGE_TRIES};
use crate::autoplay::loot::choose::LOOT_TIMEOUT;
use crate::autoplay::loot::take::only_so_often;
use crate::autoplay::team::turns::{CLAIM_STALE, SAME_MOMENT};
use crate::testkit::{
    a_party_profile, a_weapon, appraised_as, asks, character_of_level, evaded, game_data, in_view,
    item, looter, mid_fight, no_data, standing_in_the_field, STRANGER,
};

#[test]
fn a_quests_refusal_that_names_no_item_is_about_the_take_in_flight() {
    // ACE turns a drop that can only be had so often down naming item
    // 0, with only "You have solved this quest too recently". Read as
    // a refusal of nothing, the take was asked for again until the
    // corpse was given up on, and the kind was never remembered.
    let (gem, dagger) = (0x8000_7001, 0x8000_7002);
    assert_eq!(refused_item(0, 0x043E, Some(gem)), Some(gem));
    assert_eq!(refused_item(0, 0x043F, Some(gem)), Some(gem));
    // With nothing in flight there is nothing to pin it on.
    assert_eq!(refused_item(0, 0x043E, None), None);
    // A refusal naming no item for any other reason is not a take's.
    assert_eq!(refused_item(0, 0, Some(gem)), None);
    // One that names its item is about that item, whatever is flying.
    assert_eq!(refused_item(dagger, 0, Some(gem)), Some(dagger));
    assert!(only_so_often(0x043E) && only_so_often(0x043F));
    assert!(!only_so_often(0));
}

use super::*;
use crate::items::ItemStats;
use crate::refusals::{OPEN_IN_USE_AGAIN, OPEN_NOT_OURS_AGAIN};
use crate::Stance;

/// A shelf of its own holding one starter profile, so the looting
/// has rules to carry out without touching the one every session
/// shares.
fn a_loot_profile(c: &mut Client, name: &str) {
    let dir = std::env::temp_dir().join("acswarm-test-loot-profiles");
    std::fs::create_dir_all(&dir).ok();
    let shelf = std::sync::Arc::new(crate::profile::Library::default());
    shelf.open(&dir);
    let mut p = crate::profile::Profile::starter();
    p.name = name.into();
    shelf.put(p).ok();
    c.profiles = shelf;
    c.autoplay.config.loot.profile = name.into();
    assert!(c.loot_profile().is_some(), "no rules to loot by");
}

#[test]
fn a_rabbit_is_walked_past_once_it_is_outgrown_and_a_revenant_never_is() {
    // Brown Rabbit 2567 (passive, level 4), Revenant 8592 (passive,
    // level 61), Chicken 35499 (attacks on sight, level 8).
    let mut c = character_of_level(no_data(), 20);
    let cfg = Fight::default();
    let rabbit = in_view(&mut c, 0x8000_0001, 2567, "Brown Rabbit");
    let revenant = in_view(&mut c, 0x8000_0002, 8592, "Revenant");
    let chicken = in_view(&mut c, 0x8000_0003, 35499, "Chicken");
    assert!(c.a_critter(&rabbit, &cfg));
    assert!(!c.a_critter(&revenant, &cfg), "worth sixty levels");
    assert!(!c.a_critter(&chicken, &cfg), "this one starts fights");

    // A character the Rabbit is still worth something to fights it.
    c.world.stats.level = 5;
    assert!(!c.a_critter(&rabbit, &cfg));
    assert!(!c.a_critter(&revenant, &cfg));

    // Back at 20, and hitting back is never refused.
    c.world.stats.level = 20;
    assert!(c.a_critter(&rabbit, &cfg));
    c.autoplay.attacked_by("Brown Rabbit", Instant::now());
    assert!(!c.a_critter(&rabbit, &cfg), "it is hitting the character");
    // Which says nothing about the one next to it.
    let other = in_view(&mut c, 0x8000_0004, 2566, "Black Rabbit");
    assert!(c.a_critter(&other, &cfg));
    c.autoplay.last_hit_us = None;
    c.autoplay.hit_by.clear();

    // Named outright, it is what the player asked to hunt.
    let only = Fight {
        only: vec!["rabbit".into()],
        ..Fight::default()
    };
    assert!(!c.a_critter(&rabbit, &only));

    // A creature of ours already on it: its fight, and ours to end.
    c.world.objects.insert(
        0x8000_0005,
        ac_world::WorldObject {
            guid: 0x8000_0005,
            name: "Fire Elemental".into(),
            item_type: ac_world::item_type::CREATURE,
            health: Some(1.0),
            pet_owner: 0x5000_0001,
            walked_at: Some(rabbit.guid),
            ..Default::default()
        },
    );
    assert!(!c.a_critter(&rabbit, &cfg));
    assert!(c.a_critter(&other, &cfg), "the one it is not on");
    c.world.objects.remove(&0x8000_0005);

    // With the setting off, everything is fought.
    let all = Fight {
        skip_critters: false,
        ..Fight::default()
    };
    assert!(!c.a_critter(&rabbit, &all));
    assert!(!c.a_critter(&other, &all));
}

#[test]
fn a_critter_that_swings_and_misses_is_fought_rather_than_walked_past() {
    let mut c = character_of_level(no_data(), 20);
    let cfg = Fight::default();
    let rabbit = in_view(&mut c, 0x8000_0001, 2567, "Brown Rabbit");
    let other = in_view(&mut c, 0x8000_0002, 2566, "Black Rabbit");
    assert!(
        c.a_critter(&rabbit, &cfg),
        "outgrown, and it has done nothing"
    );

    // It swings and misses, over and over: the character never takes
    // a point of damage, so nothing but this says it is in a fight.
    c.chat_message(
        ac_net::messages::opcode::GAME_EVENT,
        &evaded(rabbit.guid, "Brown Rabbit"),
    );
    assert!(
        !c.a_critter(&rabbit, &cfg),
        "it is swinging at the character"
    );
    assert!(
        c.a_critter(&other, &cfg),
        "which says nothing about its neighbour"
    );
}

/// An appraisal of `guid` saying it is `level`.
fn appraised_at(c: &mut Client, guid: u32, level: i32) {
    appraised_as(c, guid, Some(level), 0);
}

#[test]
fn a_level_read_off_the_creature_itself_stands_in_for_the_tables() {
    let mut c = character_of_level(no_data(), 50);
    let cfg = Fight::default();
    // A Portal Pillar (32522) never attacks anything and the table
    // has no level for it. Nothing is walked past on a guess, nor
    // attacked on one: it is asked about, and left alone until the
    // appraisal says what it is worth.
    let pillar = in_view(&mut c, 0x8000_0011, 32522, "Portal Pillar");
    assert_eq!(
        ac_world::elements::creature_by_id(32522).and_then(|k| k.level),
        None
    );
    assert_eq!(c.critter_verdict(&pillar, &cfg), Critter::Appraise);
    assert!(c.a_critter(&pillar, &cfg), "attacked on a guess");
    appraised_at(&mut c, pillar.guid, 4);
    assert!(c.a_critter(&pillar, &cfg));
    // Asked and answered with a profile and no level at all --
    // not something ACE does -- it is fought rather than left for
    // ever.
    appraised_as(&mut c, pillar.guid, None, 2001);
    assert!(!c.a_critter(&pillar, &cfg));
    // Answered with nothing, not even a profile: ACE's word for a
    // thing it has not got. Not there to fight, and left alone.
    c.appraisals.insert(
        pillar.guid,
        ac_net::messages::Appraisal {
            guid: pillar.guid,
            ..Default::default()
        },
    );
    assert_eq!(c.critter_verdict(&pillar, &cfg), Critter::Appraise);
    assert!(
        c.a_critter(&pillar, &cfg),
        "attacked a thing the server has not got"
    );

    // A creature whose weenie is not in the table at all is known
    // only by the end of its name, and that row is some other
    // weenie: a Brown Rabbit is level 4, and this is not one.
    let stronger = in_view(&mut c, 0x8000_0012, 0x00FF_FFFF, "Weakened Brown Rabbit");
    assert_eq!(
        c.critter_verdict(&stronger, &cfg),
        Critter::Appraise,
        "the name says what kind of thing it is, not what it is worth"
    );
    assert!(c.a_critter(&stronger, &cfg), "left alone until it is known");
    appraised_at(&mut c, stronger.guid, 40);
    assert!(!c.a_critter(&stronger, &cfg), "not at forty it is not");
}

#[test]
fn a_stranger_named_like_a_critter_is_asked_about_and_judged_on_its_own_figures() {
    // On a live server a "Dire Brown Rabbit" the table has never heard of
    // is level 40 with three hundred health. Its name found the
    // Rabbit's row, and the Rabbit's four and five had it walked
    // past by anyone of level 8: the appraisal that was to beat
    // the row never came, because a creature walked past is never
    // asked about.
    let mut c = character_of_level(no_data(), 20);
    let cfg = Fight::default();
    let dire = in_view(&mut c, 0x8000_0021, STRANGER + 2, "Dire Brown Rabbit");
    assert!(
        ac_world::elements::creature_by_id(STRANGER + 2).is_none()
            && ac_world::elements::creature("Dire Brown Rabbit").is_some(),
        "known by name alone is what the test is about"
    );
    assert_eq!(c.critter_verdict(&dire, &cfg), Critter::Appraise);
    assert!(c.a_critter(&dire, &cfg), "left alone until it is known");
    // The answer says a Rabbit after all: walked past.
    appraised_as(&mut c, dire.guid, Some(4), 5);
    assert!(c.a_critter(&dire, &cfg));
    // The answer says a monster: fought.
    appraised_as(&mut c, dire.guid, Some(40), 300);
    assert!(!c.a_critter(&dire, &cfg), "not at forty it is not");
    // The one the table has by weenie is walked past on the table's
    // word, with no question asked.
    let rabbit = in_view(&mut c, 0x8000_0022, 2567, "Brown Rabbit");
    assert_eq!(c.critter_verdict(&rabbit, &cfg), Critter::WalkPast);
}

#[test]
fn a_creature_the_server_lets_go_of_takes_its_appraisal_with_it() {
    // ACE hands a gone creature's guid to a new one six hours on.
    // Kept, the Drudge's answer was read as the Cow's.
    let mut c = character_of_level(no_data(), 20);
    let cfg = Fight::default();
    let drudge = in_view(&mut c, 0x8000_0041, STRANGER, "Drudge Skulker");
    appraised_as(&mut c, drudge.guid, Some(8), 42);
    assert!(!c.a_critter(&drudge, &cfg), "worth the fight");
    // A corpse asked about keeps its answer: the loot rules read it.
    let corpse = 0x8000_0042;
    c.world.objects.insert(
        corpse,
        ac_world::WorldObject {
            guid: corpse,
            name: "Corpse of Drudge Skulker".into(),
            item_type: ac_world::item_type::CONTAINER,
            object_desc_flags: ac_world::object_desc_flags::CORPSE,
            ..Default::default()
        },
    );
    appraised_as(&mut c, corpse, None, 0);
    let delete = |guid: u32| {
        let mut w = ac_net::wire::Writer::new();
        w.u32(ac_net::messages::opcode::OBJECT_DELETE)
            .u32(guid)
            .u32(0);
        w.finish()
    };
    c.forget_the_departed(&delete(corpse));
    assert!(c.appraisals.contains_key(&corpse));
    c.forget_the_departed(&delete(drudge.guid));
    assert!(!c.appraisals.contains_key(&drudge.guid));
    // The guid comes back as a Cow: nothing known, asked about
    // afresh, and not fought on the Drudge's figures.
    let cow = in_view(&mut c, drudge.guid, STRANGER + 1, "Cow");
    assert_eq!(c.critter_verdict(&cow, &cfg), Critter::Appraise);
    assert!(
        c.a_critter(&cow, &cfg),
        "fought on another creature's answer"
    );
}

#[test]
fn a_cow_the_table_has_never_heard_of_is_walked_past_until_it_starts_something() {
    // The table was read off a server with no Cow in it, and the
    // rule fought whatever the table did not know: a level 20
    // character killed a Cow on a server that has them.
    let mut c = character_of_level(no_data(), 20);
    let cfg = Fight::default();
    let now = Instant::now();
    assert!(
        ac_world::elements::known(STRANGER, "Cow").is_none(),
        "the table knows a Cow after all"
    );
    let cow = in_view(&mut c, 0x8000_0001, STRANGER, "Cow");
    // Nothing known about it: asked about, and not attacked meanwhile.
    assert_eq!(c.critter_verdict(&cow, &cfg), Critter::Appraise);
    assert!(c.a_critter(&cow, &cfg), "attacked on nothing");
    // The answer: level 8, twenty health, and it has done nothing.
    appraised_as(&mut c, cow.guid, Some(8), 20);
    assert!(c.a_critter(&cow, &cfg), "a Cow, killed");
    // A level 15 has not outgrown it.
    c.world.stats.level = 15;
    assert!(!c.a_critter(&cow, &cfg));
    c.world.stats.level = 20;

    // It kicks: fought back.
    c.autoplay.attacked_by("Cow", now);
    assert!(!c.a_critter(&cow, &cfg), "it is hitting the character");
    c.autoplay.last_hit_us = None;
    c.autoplay.hit_by.clear();
    assert!(c.a_critter(&cow, &cfg));

    // It comes at the character, or at a mate, or at anyone: read
    // off what it walked at, since the live move target is cleared
    // by every position report of the chase (see
    // `WorldObject::walked_at`).
    let me = c.world.player_guid.unwrap();
    let mate = 0x5000_0002;
    c.autoplay.team.mates = vec![Mate {
        name: "Aldric".into(),
        guid: mate,
        ..Default::default()
    }];
    for at in [me, mate, 0x8000_0009] {
        let mut charging = cow.clone();
        charging.walked_at = Some(at);
        charging.target = None;
        assert!(!c.a_critter(&charging, &cfg), "walked at {at:#010x}");
    }
    // A mate is on it.
    c.autoplay.team.mates[0].target = Some(cow.guid);
    assert!(!c.a_critter(&cow, &cfg), "a mate is fighting it");
    c.autoplay.team.mates.clear();
    assert!(c.a_critter(&cow, &cfg));

    // Named outright, it is what the player asked to hunt.
    let only = Fight {
        only: vec!["cow".into()],
        ..Fight::default()
    };
    assert!(!c.a_critter(&cow, &only));
    // With the setting off, everything is fought.
    let all = Fight {
        skip_critters: false,
        ..Fight::default()
    };
    assert!(!c.a_critter(&cow, &all));

    // A stranger that is no Cow: an Auroch Yearling's sixty-five
    // health is a fight.
    let auroch = in_view(&mut c, 0x8000_0002, STRANGER + 1, "Auroch Yearling");
    appraised_as(&mut c, auroch.guid, Some(8), 65);
    assert!(!c.a_critter(&auroch, &cfg));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_fight_picker_asks_about_a_stranger_rather_than_attacking_it() {
    let holtburg = 0xA9B4_0019;
    let mut c = standing_in_the_field(20, holtburg, glam::Vec3::new(84.0, 84.0, 10.0));
    let me = c.player.as_ref().unwrap().world_position();
    let cfg = Fight::default();
    let now = Instant::now();
    let cow = 0x8000_0001;
    c.world.objects.insert(
        cow,
        ac_world::WorldObject {
            guid: cow,
            weenie_class_id: STRANGER,
            name: "Cow".into(),
            item_type: ac_world::item_type::CREATURE,
            object_desc_flags: ac_world::object_desc_flags::ATTACKABLE,
            health: Some(1.0),
            position: Some(ac_world::object::Position::new_flat(
                holtburg,
                me + glam::Vec3::new(5.0, 0.0, 0.0) - ac_world::landblock_origin(holtburg),
            )),
            ..Default::default()
        },
    );
    // A second stranger, farther off.
    let far = 0x8000_0002;
    c.world.objects.insert(
        far,
        ac_world::WorldObject {
            guid: far,
            weenie_class_id: STRANGER + 1,
            name: "Yak".into(),
            item_type: ac_world::item_type::CREATURE,
            object_desc_flags: ac_world::object_desc_flags::ATTACKABLE,
            health: Some(1.0),
            position: Some(ac_world::object::Position::new_flat(
                holtburg,
                me + glam::Vec3::new(12.0, 0.0, 0.0) - ac_world::landblock_origin(holtburg),
            )),
            ..Default::default()
        },
    );
    assert!(c.appraise_queue.is_empty());
    // Under attack, nothing is asked: the fight in hand comes first,
    // and a question can wake what it is asked about.
    c.autoplay.attacked_by("Drudge Skulker", now);
    assert!(!c.autoplay_fight_as(now, &cfg), "attacked a stranger");
    assert!(c.appraise_queue.is_empty(), "asked while under attack");
    c.autoplay.last_hit_us = None;
    c.autoplay.hit_by.clear();
    assert!(!c.autoplay_fight_as(now, &cfg), "attacked a stranger");
    assert_eq!(
        c.appraise_queue.iter().copied().collect::<Vec<_>>(),
        [cow],
        "the nearest, alone"
    );
    // Asked again before the answer: still the one.
    assert!(!c.autoplay_fight_as(now, &cfg));
    assert_eq!(c.appraise_queue.iter().copied().collect::<Vec<_>>(), [cow]);
    // The answer says a Cow: walked past, and the next is asked about.
    appraised_as(&mut c, cow, Some(8), 20);
    c.appraise_queue.clear();
    assert!(!c.autoplay_fight_as(now, &cfg), "a Cow, attacked");
    assert_eq!(c.appraise_queue.iter().copied().collect::<Vec<_>>(), [far]);
    appraised_as(&mut c, far, Some(8), 20);
    // The answer says something bigger: attacked.
    appraised_as(&mut c, cow, Some(8), 65);
    assert!(c.autoplay_fight_as(now, &cfg), "a fight, walked past");
    // With the setting off, nothing is asked: it is attacked.
    c.appraisals.remove(&cow);
    c.appraise_queue.clear();
    c.autoplay.last_attack = None;
    let all = Fight {
        skip_critters: false,
        ..Fight::default()
    };
    assert!(c.autoplay_fight_as(now, &all));
    assert!(c.appraise_queue.is_empty());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_swing_waits_for_a_swap_the_buff_pass_started() {
    let (mut c, wand) = mid_fight(game_data());
    // The in-fight buffing reaches for the wand. The mace goes back
    // in the pack and the wand waits on empty hands.
    assert!(c.wield_for(Stance::Magic), "the swap went out");
    assert_eq!(c.autoplay.pending_wield, Some(wand));
    // Which the fight can now see. Before this, only the arming
    // code stamped the swap clock, so a swap the buff pass or the
    // softening started was invisible and the next swing went out
    // into the empty hands: +Verity put her Flaming Takuba away for
    // a wand and punched a Spikey Armoredillo.
    assert!(c.hands_changing(Instant::now()), "the swap is under way");
    // The hands still say melee -- the server has not answered the
    // put yet -- so nothing but this holds the swing back.
    assert_eq!(c.combat_stance(), Stance::Melee);
    c.tick_combat();
    assert!(!c.attack_pending, "no swing into the empty hands");
    // And the wait is bounded: a swap the server never finishes
    // cannot stop the character fighting.
    c.autoplay.last_rewield = Some(Instant::now() - SWAP_SETTLES);
    c.tick_combat();
    assert!(c.attack_pending, "swinging again once the swap is stale");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_weapon_comes_back_out_of_the_pack_after_a_buff() {
    let (mut c, wand) = mid_fight(game_data());
    const MACE: u32 = 0x8000_0101;
    // The buff pass put the mace down and took the wand up.
    a_weapon(
        &mut c,
        MACE,
        ac_world::item_type::MELEE_WEAPON,
        "Mace",
        false,
    );
    a_weapon(
        &mut c,
        wand,
        ac_world::item_type::CASTER,
        "Training Wand",
        true,
    );
    c.autoplay.put_down = Some(MACE);
    c.autoplay_rearm();
    // ACE will not put a mace in a hand that holds a caster -- it
    // refuses the wield outright, with no error to read -- so the
    // wand goes back in the pack first and the mace waits on empty
    // hands. Asking straight out was refused every single time.
    assert_eq!(c.autoplay.pending_wield, Some(MACE));
    assert_eq!(c.autoplay.wield_asked, None, "nothing was asked for yet");
    assert_eq!(c.autoplay.put_down, None, "the errand passed on");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn an_errand_the_server_is_refusing_is_kept_rather_than_dropped() {
    let (mut c, _) = mid_fight(game_data());
    const MACE: u32 = 0x8000_0101;
    const SHIELD: u32 = 0x8000_0104;
    let me = c.world.player_guid;
    a_weapon(
        &mut c,
        MACE,
        ac_world::item_type::MELEE_WEAPON,
        "Mace",
        false,
    );
    c.autoplay.put_down = Some(MACE);
    c.hold_off_wield(MACE, Instant::now());
    c.autoplay_rearm();
    // Asking now sends nothing, so clearing the errand would leave
    // the mace in the pack with nothing left to ask again.
    assert_eq!(c.autoplay.put_down, Some(MACE), "still owed the weapon");
    assert_eq!(c.autoplay.pending_wield, None);

    // The shield keeps its errand the same way.
    c.world.objects.insert(
        SHIELD,
        ac_world::WorldObject {
            guid: SHIELD,
            name: "Buckler".into(),
            valid_locations: ac_world::equip::SHIELD,
            container: me,
            ..Default::default()
        },
    );
    c.autoplay.wanted_shield = Some(SHIELD);
    c.hold_off_wield(SHIELD, Instant::now());
    c.autoplay_shield(Instant::now());
    assert_eq!(c.autoplay.wanted_shield, Some(SHIELD), "still owed it");
}

#[test]
fn the_shield_goes_on_between_swings_and_not_during_one() {
    let (mut c, _) = mid_fight(no_data());
    const SHIELD: u32 = 0x8000_0104;
    let me = c.world.player_guid;
    c.world.objects.insert(
        SHIELD,
        ac_world::WorldObject {
            guid: SHIELD,
            name: "Buckler".into(),
            valid_locations: ac_world::equip::SHIELD,
            container: me,
            ..Default::default()
        },
    );
    c.autoplay.wanted_shield = Some(SHIELD);
    // A swing is in the air. ACE shuffles the stance on every
    // successful equip, a shield included, and a combat-mode change
    // cancels the attack -- so the shield waits for the gap.
    c.attack_pending = true;
    c.last_attack = Instant::now();
    assert!(c.mid_attack());
    c.autoplay_shield(Instant::now());
    assert_eq!(c.autoplay.wanted_shield, Some(SHIELD), "still to go on");
    assert_eq!(c.autoplay.wield_asked, None, "nothing sent mid-swing");
    assert!(c.wants_the_hands, "the gap after the swing is booked");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_weapon_choice_put_off_for_a_swing_is_made_once_the_fight_is_joined() {
    let (mut c, _) = mid_fight(game_data());
    let cfg = Fight::default();
    // The choice was put off: a swing was in the air when the target
    // was picked, so `arm_for` booked the gap and returned without
    // recording what it had armed for. The attack went out anyway --
    // the target that owned the swing had left the area or been
    // given up on, which clears `attack_target` and leaves
    // `attack_pending` set.
    assert_eq!(c.autoplay.armed_for, None);
    assert!(c.appraise_queue.is_empty());
    // Nothing in the air now, and the fight is joined.
    assert!(!c.mid_attack());
    assert!(c.autoplay_fight_as(Instant::now(), &cfg), "fighting");
    // Only this branch runs from here on: the picker is never
    // reached again while the target is alive. Without it the
    // character fought the whole creature with whatever the buff
    // pass had left in its hands, and a bow with no arrows chosen.
    assert!(
        !c.appraise_queue.is_empty(),
        "the weapons are being weighed for the choice"
    );
}

/// A shelf holding one profile of `rules`, in a directory of its
/// own so that two tests never read each other's files.
fn shelf(named: &str, rules: Vec<crate::profile::Rule>) -> crate::profile::Library {
    let dir = std::env::temp_dir().join(format!("acswarm-{named}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let library = crate::profile::Library::default();
    library.open(&dir);
    library
        .put(crate::profile::Profile {
            name: "test".into(),
            rules,
            ..Default::default()
        })
        .expect("saved");
    library
}

fn judged(
    stats: &ItemStats,
    library: &crate::profile::Library,
    profile: &str,
) -> crate::profile::Verdict {
    judge_loot(
        stats,
        None,
        library.get(profile).as_deref(),
        &crate::weapons::Wielder::default(),
        "Aldric",
        0,
    )
}

/// Set the name lists on the test profile. They are the profile's
/// now, so they change for everyone reading it.
fn name_lists(library: &crate::profile::Library, always: &[&str], never: &[&str]) {
    let mut p = (*library.get("test").expect("the test profile")).clone();
    p.looting.always = always.iter().map(|s| s.to_string()).collect();
    p.looting.never = never.iter().map(|s| s.to_string()).collect();
    library.put(p).expect("put");
}

#[test]
fn the_players_own_word_comes_before_the_profile() {
    use crate::profile::Verdict;
    let library = shelf(
        "own-word",
        vec![
            asks("keepers", "value>250", LootAction::Keep),
            asks("trash", "rusty", LootAction::Sell),
        ],
    );
    name_lists(&library, &[], &[]);
    let ring = item("Ornate Ring", 900, 0);
    let nail = item("Rusty Nail", 3, 0);
    assert_eq!(
        judged(&ring, &library, "test"),
        Verdict::Decided(LootAction::Keep, "keepers".into())
    );
    assert_eq!(
        judged(&nail, &library, "test"),
        Verdict::Decided(LootAction::Sell, "trash".into())
    );
    // Never wins over a rule that would have kept it.
    name_lists(&library, &[], &["ornate"]);
    assert_eq!(
        judged(&ring, &library, "test"),
        Verdict::Decided(LootAction::Skip, "never take these".into())
    );
    // Always wins over a rule that would have sold it, and loses to
    // never, which is read first.
    name_lists(&library, &["rusty"], &[]);
    assert_eq!(
        judged(&nail, &library, "test"),
        Verdict::Decided(LootAction::Keep, "always take these".into())
    );
    name_lists(&library, &["rusty"], &["rusty"]);
    assert_eq!(
        judged(&nail, &library, "test"),
        Verdict::Decided(LootAction::Skip, "never take these".into())
    );
    let _ = std::fs::remove_dir_all(library.dir());
}

/// Set the buy list on the test profile.
fn buy_list(library: &crate::profile::Library, lines: &[(&str, u32)]) {
    let mut p = (*library.get("test").expect("the test profile")).clone();
    p.buy = lines
        .iter()
        .map(|(what, keep)| crate::profile::Buy {
            what: what.to_string(),
            keep: *keep,
            restock_at: None,
            from: None,
            on: true,
        })
        .collect();
    library.put(p).expect("put");
}

#[test]
fn the_buy_list_keeps_its_line_and_the_rules_answer_for_the_rest() {
    // A line on the buy list is the player's word that this many
    // are stock. Up to the line, a taper is kept whatever the rules
    // make of it -- "sell the rest" once tagged the tapers just
    // bought for the line, and the next trip sold them and bought
    // them again. Over the line, the rules answer, so a surplus
    // under "sell the rest" still goes.
    use crate::profile::Verdict;
    let library = shelf(
        "stock",
        vec![asks("the rest", "value>=0", LootAction::Sell)],
    );
    buy_list(&library, &[("Prismatic Taper", 100)]);
    let profile = library.get("test");
    let me = crate::weapons::Wielder::default();
    let judge = |stats: &ItemStats, held: u32| {
        judge_loot(stats, None, profile.as_deref(), &me, "Aldric", held)
    };
    let taper = item("Prismatic Taper", 500, 0);
    assert_eq!(
        judge(&taper, 0),
        Verdict::Decided(LootAction::Keep, "kept stocked".into())
    );
    assert_eq!(
        judge(&taper, 99),
        Verdict::Decided(LootAction::Keep, "kept stocked".into()),
        "one short of the line: the stack that fills it is kept"
    );
    assert_eq!(
        judge(&taper, 100),
        Verdict::Decided(LootAction::Sell, "the rest".into()),
        "the line is full: the rules answer"
    );
    // What the list does not name is the rules' from the start.
    assert_eq!(
        judge(&item("Lead Scarab", 5, 0), 0),
        Verdict::Decided(LootAction::Sell, "the rest".into())
    );
    // A line switched off says nothing.
    let mut p = (*library.get("test").unwrap()).clone();
    p.buy[0].on = false;
    library.put(p).unwrap();
    let profile = library.get("test");
    assert_eq!(
        judge_loot(&taper, None, profile.as_deref(), &me, "Aldric", 0),
        Verdict::Decided(LootAction::Sell, "the rest".into())
    );
    let _ = std::fs::remove_dir_all(library.dir());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn what_arrives_is_judged_against_what_was_held_before_it() {
    // "Keep up to four healing kits", three in the pack, a fourth
    // bought. The arrival pass once counted the arrival itself, so
    // the fourth was the fourth of four, over the cap, and "the
    // rest, to the counter" tagged it to sell: the kit just bought
    // for the line went back over the counter. Held is what was
    // held before it, on this path as on the corpse's, and a fifth
    // is the one over the cap.
    let mut c = character_of_level(game_data(), 20);
    let dir = std::env::temp_dir().join("acswarm-test-arrival-profiles");
    std::fs::create_dir_all(&dir).ok();
    let shelf = std::sync::Arc::new(crate::profile::Library::default());
    shelf.open(&dir);
    let mut kits = crate::profile::Rule {
        name: "kits".into(),
        action: LootAction::Keep,
        all: vec![crate::profile::Ask::Item(crate::items::Term::Word(
            "healing kit".into(),
        ))],
        ..Default::default()
    };
    kits.keep_up_to = Some(4);
    shelf
        .put(crate::profile::Profile {
            name: "four-kits".into(),
            rules: vec![kits, asks("the rest", "value>=0", LootAction::Sell)],
            ..Default::default()
        })
        .unwrap();
    c.profiles = shelf;
    c.autoplay.config.loot.profile = "four-kits".into();
    let me = c.world.player_guid.unwrap();
    let kit = |guid: u32| ac_world::WorldObject {
        guid,
        name: "Healing Kit".into(),
        weenie_class_id: 4000,
        item_type: ac_world::item_type::MISC,
        value: 100,
        stack_size: 1,
        container: Some(me),
        ..Default::default()
    };
    for guid in [0x8000_0001, 0x8000_0002, 0x8000_0003] {
        c.world.objects.insert(guid, kit(guid));
    }
    let t0 = Instant::now();
    // The first pass only notes what is carried.
    c.autoplay_tag_arrivals(t0);
    c.world.objects.insert(0x8000_0004, kit(0x8000_0004));
    c.autoplay_tag_arrivals(t0);
    assert_eq!(
        c.autoplay.ledger.by_guid(0x8000_0004),
        Some(LootAction::Keep),
        "the fourth of four is under the cap"
    );
    c.world.objects.insert(0x8000_0005, kit(0x8000_0005));
    c.autoplay_tag_arrivals(t0);
    assert_eq!(
        c.autoplay.ledger.by_guid(0x8000_0005),
        Some(LootAction::Sell),
        "the fifth is over it"
    );
}

#[test]
fn nothing_is_decided_without_a_profile() {
    use crate::profile::Verdict;
    let library = shelf(
        "no-profile",
        vec![asks("keepers", "value>250", LootAction::Keep)],
    );
    name_lists(&library, &["ornate"], &[]);
    let ring = item("Ornate Ring", 900, 0);
    // A character reading no profile, or one not on the shelf, takes
    // nothing -- the name lists included, since they are the
    // profile's and not the character's.
    assert_eq!(judged(&ring, &library, ""), Verdict::None);
    assert_eq!(judged(&ring, &library, "missing"), Verdict::None);
    assert_eq!(
        judged(&ring, &library, "test"),
        Verdict::Decided(LootAction::Keep, "always take these".into())
    );
    let _ = std::fs::remove_dir_all(library.dir());
}

#[test]
fn what_arrives_in_the_pack_is_judged_like_what_lies_on_a_corpse() {
    // A bundle of arrowheads is a bundle of arrowheads whether it
    // came off a drudge or over a counter.
    let library = shelf(
        "arrival",
        vec![
            asks("keepers", "value>250", LootAction::Keep),
            asks("plate", "type:armor al>=200", LootAction::Sell),
        ],
    );
    name_lists(&library, &[], &[]);
    let me = crate::weapons::Wielder::default();
    let tag =
        |s: &ItemStats| arrival_tag(s, None, library.get("test").as_deref(), &me, "Aldric", 0);
    assert_eq!(tag(&item("Ornate Ring", 900, 0)), Some(LootAction::Keep));
    assert_eq!(tag(&item("Platemail", 100, 240)), Some(LootAction::Sell));
    // Nothing claimed it, so there is nothing to write down...
    assert_eq!(tag(&item("Rusty Nail", 3, 0)), None);
    // ...and neither has a skip, which is a decision to leave it.
    name_lists(&library, &[], &["ornate"]);
    assert_eq!(tag(&item("Ornate Ring", 900, 0)), None);
    // An item that cannot be judged until it is appraised is not
    // written down either: the answer is not in yet.
    name_lists(&library, &[], &[]);
    let unread = ItemStats {
        appraised: false,
        ..item("Platemail", 100, 240)
    };
    assert!(matches!(
        judged(&unread, &library, "test"),
        crate::profile::Verdict::NeedsId(_)
    ));
    assert_eq!(tag(&unread), None);
    assert_eq!(LootAction::parse("Salvage"), Some(LootAction::Salvage));
    assert_eq!(LootAction::parse("burn"), None);
    assert!(!LootAction::Skip.takes());
    let _ = std::fs::remove_dir_all(library.dir());
}

/// A level 20 character that salvages for itself: an Ust in the pack,
/// and a loot profile of its own called `profile`.
fn a_salvager(assets: std::rc::Rc<ac_scene::Assets>, profile: &str) -> Client {
    let mut c = character_of_level(assets, 20);
    a_loot_profile(&mut c, profile);
    let ust = 0x8000_0100;
    c.world.objects.insert(
        ust,
        ac_world::WorldObject {
            guid: ust,
            weenie_class_id: ac_world::material::UST_WCID,
            name: "Ust".into(),
            container: c.world.player_guid,
            ..Default::default()
        },
    );
    c
}

/// An Iron mace of `workmanship` in `container`, tagged for salvage.
fn a_mace_to_salvage(c: &mut Client, guid: u32, workmanship: f32, container: Option<u32>) {
    c.world.objects.insert(
        guid,
        ac_world::WorldObject {
            guid,
            name: "Iron Mace".into(),
            material: 0x3D,
            workmanship,
            container,
            ..Default::default()
        },
    );
    let stats = c.stats_of(guid).expect("carried");
    c.autoplay.tag(&stats, LootAction::Salvage);
}

/// The salvage the salvager is waiting on.
fn salvage_on_its_way(c: &Client) -> Option<Vec<u32>> {
    c.autoplay.salvaging.as_ref().map(|(g, _)| g.clone())
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn what_the_team_hands_the_salvager_is_salvaged_a_grade_at_a_time() {
    // Teammates hand salvage over one item at a time, and the
    // salvager salvages it along with its own: that is where a 10
    // one teammate carried would meet another's 6.
    let mut c = a_salvager(game_data(), "salvage test");
    let me = c.world.player_guid;
    // The 9 is in a side pack: the server finds it there, and so
    // must the salvage.
    let pack = 0x8000_0110;
    c.world.objects.insert(
        pack,
        ac_world::WorldObject {
            guid: pack,
            name: "Pack".into(),
            container: me,
            ..Default::default()
        },
    );
    let (ten, six, nine) = (0x8000_0101, 0x8000_0102, 0x8000_0103);
    for (guid, workmanship, container) in [(ten, 10.0, me), (six, 6.0, me), (nine, 9.0, Some(pack))]
    {
        a_mace_to_salvage(&mut c, guid, workmanship, container);
    }
    let mut now = Instant::now();
    assert!(c.autoplay_salvage(now), "salvaging");
    assert_eq!(salvage_on_its_way(&c), Some(vec![ten]));
    for (done, next) in [(ten, nine), (nine, six)] {
        now += Duration::from_millis(100);
        assert!(c.autoplay_salvage(now), "not waited on");
        assert_eq!(salvage_on_its_way(&c), Some(vec![done]));
        // The server takes it, and the next grade goes at once. Sitting
        // out the gap gave the tick to the fight, and the grades still
        // to come waited a whole fight for their turn.
        c.world.objects.remove(&done);
        now += Duration::from_millis(100);
        assert!(c.autoplay_salvage(now), "the next grade waited");
        assert_eq!(salvage_on_its_way(&c), Some(vec![next]));
    }
    c.world.objects.remove(&six);
    now += Duration::from_millis(100);
    assert!(!c.autoplay_salvage(now), "nothing left to salvage");
    assert_eq!(salvage_on_its_way(&c), None);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_ten_the_server_skips_holds_up_none_of_the_grades_below_it() {
    // ACE skips a Retained item without a word, and its salvage times
    // out. Chosen again as the best grade, the 10 went out alone after
    // every timeout, and the 9 and the 6 waited behind all three.
    let mut c = a_salvager(game_data(), "salvage skipped");
    let me = c.world.player_guid;
    let (ten, nine, six) = (0x8000_0121, 0x8000_0122, 0x8000_0123);
    for (guid, workmanship) in [(ten, 10.0), (nine, 9.0), (six, 6.0)] {
        a_mace_to_salvage(&mut c, guid, workmanship, me);
    }
    let mut now = Instant::now();
    assert!(c.autoplay_salvage(now));
    assert_eq!(salvage_on_its_way(&c), Some(vec![ten]));
    // Nothing comes of it.
    now += SALVAGE_TIMEOUT;
    assert!(c.autoplay_salvage(now));
    assert_eq!(
        salvage_on_its_way(&c),
        Some(vec![nine]),
        "the 10 went again before the grades not yet tried"
    );
    for (done, next) in [(nine, six), (six, ten)] {
        c.world.objects.remove(&done);
        now += Duration::from_millis(100);
        assert!(c.autoplay_salvage(now));
        assert_eq!(salvage_on_its_way(&c), Some(vec![next]));
    }
    // Still on its own, until the third try sets it aside.
    now += SALVAGE_TIMEOUT;
    assert!(c.autoplay_salvage(now));
    assert_eq!(salvage_on_its_way(&c), Some(vec![ten]));
    now += SALVAGE_TIMEOUT;
    assert!(!c.autoplay_salvage(now), "tried {SALVAGE_TRIES} times");
    assert_eq!(salvage_on_its_way(&c), None);
}

#[test]
fn a_corpse_is_ours_when_it_names_this_character_or_its_creature_as_the_killer() {
    assert!(killed_by_us("Killed by Blargerton.", "Blargerton"));
    assert!(killed_by_us(
        "Killed by Blargerton's Mud Golem.",
        "Blargerton"
    ));
    // The server leaves off a leading '+'; a line that kept it still
    // counts.
    assert!(killed_by_us("Killed by Fletch.", "+Fletch"));
    assert!(killed_by_us("Killed by Fletch's Mud Golem.", "+Fletch"));
    assert!(killed_by_us("Killed by +Fletch.", "+Fletch"));
    // A rare found on the body adds to the line.
    assert!(killed_by_us(
        "Killed by Blargerton. This corpse generated a rare item!",
        "Blargerton"
    ));
    // A name that only starts like ours is someone else.
    assert!(!killed_by_us("Killed by Blargertonia.", "Blargerton"));
    // Another player, or another player's creature, is not us.
    assert!(!killed_by_us("Killed by Grimble.", "Blargerton"));
    assert!(!killed_by_us(
        "Killed by Grimble's Mud Golem.",
        "Blargerton"
    ));
    assert!(!killed_by_us("Killed by misadventure.", "Blargerton"));
    // Nothing to go on is not ours.
    assert!(!killed_by_us("", "Blargerton"));
    assert!(!killed_by_us("Killed by .", ""));
}

#[test]
fn a_body_one_of_the_others_has_is_left_to_them() {
    // Nine characters in one huddle rank the bodies by the same
    // rule and so choose the same one: 1,386 opens for 41 bodies in
    // one run, because the server hands a container to one viewer
    // and refuses everyone else. Whoever has a body says so, and
    // the rest take another.
    let t0 = Instant::now();
    let (a, b) = (0x8000_0001, 0x8000_0002);
    let (here, there) = (glam::Vec3::ZERO, glam::Vec3::new(4.0, 0.0, 0.0));
    let me = 0x5000_0001;
    // Both fell a while back, so the tie-break has had its moment
    // and the claims are what speak.
    let mut ap = Autoplay {
        corpse_seen: vec![(a, t0), (b, t0)],
        ..Default::default()
    };
    let now = t0 + CLAIM_SETTLE;

    // Alone on the board, nothing changes: both are ours.
    assert!(ap.ours_to_open(a, here, me, here, now));
    assert!(ap.ours_to_open(b, there, me, here, now));

    // One mate is at the first body, another at nothing.
    ap.team.mates = vec![
        looter(2, here, Some(a), Duration::ZERO),
        looter(3, here, None, Duration::ZERO),
    ];
    assert!(
        !ap.ours_to_open(a, here, me, here, now),
        "raced a mate for a body"
    );
    assert!(
        ap.ours_to_open(b, there, me, here, now),
        "left a free body lying"
    );

    // A claim goes stale. A mate that stalled or was dragged into a
    // fight over a body must not hold it for the five minutes it
    // lies there.
    ap.team.mates = vec![looter(2, here, Some(a), CLAIM_STALE)];
    assert!(
        ap.ours_to_open(a, here, me, here, now),
        "a stale claim still held"
    );
    ap.team.mates = vec![looter(
        2,
        here,
        Some(a),
        CLAIM_STALE - Duration::from_millis(1),
    )];
    assert!(!ap.ours_to_open(a, here, me, here, now));

    // A mate that died over the body it had open goes on saying so:
    // its client is still running. The same board row says its
    // health is nothing, and that is read rather than waited out.
    ap.team.mates = vec![Mate {
        health: 0.0,
        ..looter(2, here, Some(a), Duration::ZERO)
    }];
    assert!(
        ap.ours_to_open(a, here, me, here, now),
        "a dead mate held a body for the rest of the claim's life"
    );
}

#[test]
fn a_body_this_character_has_already_claimed_is_not_given_up_for_a_mates() {
    // Two characters that choose one body in the same tick each
    // hear the other's claim half a second later. Asking only
    // "does somebody claim it" made both stand off, each for the
    // other, and the walk each had started was never let go of --
    // so the claim kept going out, the body was opened by nobody,
    // and eight characters stood over it until every claim aged out
    // at once. A standing claim wins the tie instead of losing it.
    let t0 = Instant::now();
    let body = 0x8000_0001;
    let at = glam::Vec3::ZERO;
    let (me, lower, higher) = (0x5000_0005, 0x5000_0001, 0x5000_0009);
    let mut ap = Autoplay {
        corpse_seen: vec![(body, t0)],
        ..Default::default()
    };
    ap.take_up_corpse(body, t0, LOOT_TIMEOUT);
    let now = t0 + CLAIM_SETTLE;

    // A mate that reached for it in the same moment and has the
    // higher guid gives way to us.
    ap.team.mates = vec![looter(higher, at, Some(body), CLAIM_SETTLE)];
    assert!(
        ap.ours_to_open(body, at, me, at, now),
        "gave up a body we were already holding"
    );

    // The lower guid in the same moment has it, and we let go.
    ap.team.mates = vec![looter(lower, at, Some(body), CLAIM_SETTLE)];
    assert!(!ap.ours_to_open(body, at, me, at, now));

    // Plainly older than ours, whatever the guid: theirs.
    ap.team.mates = vec![looter(
        higher,
        at,
        Some(body),
        CLAIM_SETTLE + SAME_MOMENT * 2,
    )];
    assert!(!ap.ours_to_open(body, at, me, at, now));

    // Plainly younger than ours, whatever the guid: ours.
    let later = t0 + SAME_MOMENT * 3;
    ap.team.mates = vec![looter(lower, at, Some(body), Duration::ZERO)];
    assert!(ap.ours_to_open(body, at, me, at, later));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_walk_to_a_body_this_character_has_given_up_on_is_let_go_of() {
    // The one way out of the looting that did not stop the walk. A
    // character that chose a body, set off for it, and then heard a
    // better claim on it walked on -- and the walk is itself a claim
    // (see `Autoplay::corpse_claim`), so it went on telling the
    // others the body was taken, and went on holding the looting's
    // own worth up, for a body it would never open.
    let holtburg = 0xA9B4_0019;
    let at = glam::Vec3::new(84.0, 84.0, 10.0);
    let mut c = standing_in_the_field(20, holtburg, at);
    a_loot_profile(&mut c, "walk test");
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let body = 0x8000_0001;
    c.world.objects.insert(
        body,
        ac_world::WorldObject {
            guid: body,
            name: "Corpse of Drudge Slave".into(),
            object_desc_flags: ac_world::object_desc_flags::CORPSE,
            position: Some(ac_world::object::Position::new_flat(
                holtburg,
                me + glam::Vec3::new(7.0, 0.0, 0.0) - ac_world::landblock_origin(holtburg),
            )),
            ..Default::default()
        },
    );
    c.autoplay.corpse_seen = vec![(body, now)];
    // Walking to it, seven metres off, and a mate says it claimed
    // the same body well before we did.
    c.autoplay.walking_to = Some(CorpseWalk::new(body, 7.0, now));
    c.autoplay.team.mates = vec![looter(
        0x5000_0002,
        me,
        Some(body),
        SAME_MOMENT + Duration::from_secs(5),
    )];

    assert!(!c.autoplay_loot(now), "went to a body that is not ours");
    assert_eq!(c.autoplay.walking_to.map(|w| w.guid), None, "still walking");
    assert_eq!(c.autoplay.corpse_claim(now), None, "still claiming it");
}

#[test]
fn a_thing_a_skill_rule_takes_is_for_the_highest_of_that_skill_as_it_stands() {
    // "If an item has a skill requirement in the loot settings, it
    // should go to whoever has the highest skill."
    use ac_world::stats::{sac, skill};
    let sheet = |base: u32, now: u32| crate::weapons::Wielder {
        level: 50,
        skills: vec![(skill::LOCKPICK, base, now, sac::TRAINED)],
        ..Default::default()
    };
    let profile = a_party_profile();
    let key = item("Broken Marble Key", 0, 0);
    let (mine, steady, buffed, sapped) = (
        sheet(300, 300),
        sheet(320, 320),
        sheet(280, 360),
        sheet(400, 200),
    );
    let taker = |guid, name, sheet| Taker {
        guid,
        name,
        sheet,
        held: 0,
    };
    let takers = [
        taker(1, "Bryn01", &mine),
        taker(2, "Bryn02", &steady),
        taker(3, "Bryn03", &buffed),
        taker(4, "Bryn04", &sapped),
    ];
    // Buffs counted, as the rule counts them: the highest as it
    // stands. The highest before buffs is drained below what the rule
    // asks, and the rule does not take the key for it at all.
    assert_eq!(called_to(&key, None, &profile, &takers, None), Some(3));
    // Two as high: the name that sorts first, in every session.
    let twin = sheet(360, 360);
    let tied = [takers[2], taker(5, "Aldric", &twin)];
    assert_eq!(called_to(&key, None, &profile, &tied, None), Some(5));
    // Alone, it is this character's, as it always was.
    assert_eq!(called_to(&key, None, &profile, &takers[..1], None), Some(1));
    // What no rule asking about a skill takes is nobody's in particular.
    let kits = item("Healing Kit", 50, 0);
    assert_eq!(called_to(&kits, None, &profile, &takers, Some(2)), None);

    // Salvage is for whoever salvages for the team.
    let mut salvaging = profile.clone();
    salvaging.rules.push(asks(
        "platemail to salvage",
        "platemail",
        LootAction::Salvage,
    ));
    let plate = item("Platemail", 100, 240);
    assert_eq!(
        called_to(&plate, None, &salvaging, &takers, Some(2)),
        Some(2)
    );
    // When it is not one of those that could take it -- out of reach,
    // or nobody carries an Ust -- it is not waited on: the thing is
    // nobody's in particular, and the hand-off takes it there later.
    assert_eq!(called_to(&plate, None, &salvaging, &takers, Some(9)), None);
    assert_eq!(called_to(&plate, None, &salvaging, &takers, None), None);

    // A key two rules would take goes by the first of them in the
    // profile, as each character reads its rules: the lockpick rule,
    // though for the one it does not hold for the salvage rule further
    // down would take it.
    salvaging
        .rules
        .push(asks("keys to salvage", "broken", LootAction::Salvage));
    assert_eq!(called_to(&key, None, &salvaging, &takers, Some(4)), Some(3));
    // And a rule ahead of both that takes it for anyone makes it
    // nobody's in particular.
    salvaging
        .rules
        .insert(0, asks("every key", "key", LootAction::Keep));
    assert_eq!(called_to(&key, None, &salvaging, &takers, Some(4)), None);
}

#[test]
fn nobody_is_sent_anything_when_no_rule_asks_about_a_skill_and_nobody_salvages() {
    let mut profile = crate::profile::Profile {
        name: "plain".into(),
        rules: vec![
            asks("gems", "diamond", LootAction::Sell),
            asks("platemail to salvage", "platemail", LootAction::Salvage),
            asks("kits", "healing kit", LootAction::Keep),
        ],
        ..Default::default()
    };
    profile.looting.salvage = false;
    assert!(!profile.sends_to_the_best());
    let weak = crate::weapons::Wielder::default();
    let strong = crate::weapons::Wielder {
        level: 200,
        skills: vec![(ac_world::stats::skill::SALVAGING, 400, 450, 3)],
        ..Default::default()
    };
    let takers = [
        Taker {
            guid: 1,
            name: "Bryn01",
            sheet: &weak,
            held: 0,
        },
        Taker {
            guid: 2,
            name: "Bryn02",
            sheet: &strong,
            held: 0,
        },
    ];
    for thing in [
        item("Diamond", 5000, 0),
        item("Platemail", 100, 240),
        item("Healing Kit", 50, 0),
    ] {
        assert_eq!(
            called_to(&thing, None, &profile, &takers, Some(2)),
            None,
            "{}",
            thing.name
        );
    }
    // Nor under the starter, while its salvager does not salvage.
    let mut starter = crate::profile::Profile::starter();
    starter.looting.salvage = false;
    assert!(!starter.sends_to_the_best());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_salvage_on_an_open_body_is_left_for_the_salvager_standing_by() {
    use crate::logistics::Supplies;
    let holtburg = 0xA9B4_0019;
    let mut c = standing_in_the_field(20, holtburg, glam::Vec3::new(84.0, 84.0, 10.0));
    let me = c.player.as_ref().unwrap().world_position();
    let t0 = Instant::now();
    let (body, plate, gem) = (0x8000_5001, 0x8000_5002, 0x8000_5003);
    c.world.objects.insert(
        body,
        ac_world::WorldObject {
            guid: body,
            name: "Corpse of Drudge Slave".into(),
            object_desc_flags: ac_world::object_desc_flags::CORPSE,
            position: Some(ac_world::object::Position::new_flat(
                holtburg,
                me + glam::Vec3::new(2.0, 0.0, 0.0) - ac_world::landblock_origin(holtburg),
            )),
            ..Default::default()
        },
    );
    for (guid, name) in [(plate, "Platemail"), (gem, "Diamond")] {
        c.world.objects.insert(
            guid,
            ac_world::WorldObject {
                guid,
                name: name.into(),
                container: Some(body),
                ..Default::default()
            },
        );
    }
    let mut profile = crate::profile::Profile {
        name: "party".into(),
        rules: vec![
            asks("platemail to salvage", "platemail", LootAction::Salvage),
            asks("gems", "diamond", LootAction::Sell),
        ],
        ..Default::default()
    };
    // Bryn02 salvages for the party and stands by; Bryn03 does not
    // salvage. This character carries no Ust.
    let salvager = Mate {
        has_ust: true,
        salvaging: 300,
        ..looter(2, me, None, Duration::ZERO)
    };
    let other = looter(3, me, None, Duration::ZERO);
    c.autoplay.team.mates = vec![salvager.clone(), other.clone()];
    let verdicts = |c: &mut Client, profile: &crate::profile::Profile| {
        let open = c.corpse_now(body, &[plate, gem], profile, t0, t0);
        let of = |g| open.items.iter().find(|i| i.guid == g).map(|i| i.verdict);
        (of(plate), of(gem))
    };
    let take = |a| Some(ac_loot::Verdict::Take(a));
    assert_eq!(
        verdicts(&mut c, &profile),
        (Some(ac_loot::Verdict::Leave), take(LootAction::Sell)),
        "took the salvager's salvage, or left the rest"
    );
    // Shut with the platemail on it: left for the salvager. Bryn03 would
    // take it too and stands by for the salvager, and so does this
    // character, rather than either writing the body off.
    let shut = c.shut_for(body, &[plate], &profile);
    assert_eq!(
        (shut.done_for, shut.stand_by, shut.left_for, shut.waits),
        (Vec::<u32>::new(), vec![3], vec![2], true)
    );

    // Never waited on: dead, off the board, with no room, not looting,
    // or having shut this body already. Each time it is taken as ever.
    let salvage = (take(LootAction::Salvage), take(LootAction::Sell));
    c.autoplay.team.mates = vec![
        Mate {
            health: 0.0,
            ..salvager.clone()
        },
        other.clone(),
    ];
    assert_eq!(verdicts(&mut c, &profile), salvage, "dead");
    c.autoplay.team.mates = vec![other.clone()];
    assert_eq!(verdicts(&mut c, &profile), salvage, "off the board");
    c.autoplay.team.mates = vec![
        Mate {
            supplies: Supplies {
                laden: true,
                ..Default::default()
            },
            ..salvager.clone()
        },
        other.clone(),
    ];
    assert_eq!(verdicts(&mut c, &profile), salvage, "laden");
    c.autoplay.team.mates = vec![
        Mate {
            opens_bodies: false,
            ..salvager.clone()
        },
        other.clone(),
    ];
    assert_eq!(verdicts(&mut c, &profile), salvage, "not looting");
    c.autoplay.config.team.enabled = true;
    c.autoplay.team.mates = vec![
        Mate {
            shut: vec![Shut {
                body,
                done_for: vec![3],
                ..Default::default()
            }],
            ..salvager.clone()
        },
        other.clone(),
    ];
    c.autoplay.take_in_shuts(0x5000_0001, t0, |_| None);
    assert_eq!(verdicts(&mut c, &profile), salvage, "shut it already");

    // Rules that mean nothing for anyone in particular, and alone.
    c.autoplay.team.mates = vec![salvager, other];
    c.autoplay.shut_by.clear();
    c.autoplay.done_with.clear();
    profile.looting.salvage = false;
    assert_eq!(verdicts(&mut c, &profile), salvage, "nobody salvages");
    profile.looting.salvage = true;
    c.autoplay.team.mates.clear();
    assert_eq!(verdicts(&mut c, &profile), salvage, "alone");
}

#[test]
fn what_an_item_was_taken_for_is_written_down() {
    let mut ap = Autoplay::default();
    let ring = item("Ornate Ring", 900, 0);
    assert_eq!(ap.tags().get(&ring.guid), None);
    ap.tag(&ring, LootAction::Salvage);
    assert_eq!(ap.tags().get(&ring.guid), Some(&LootAction::Salvage));
    assert_eq!(Doing::Salvaging.label(), "salvaging");
}

/// A corpse open at the character's feet, as the loot rules see it,
/// with one thing on it worth taking.
fn corpse_at_hand(guid: u32, item: u32) -> ac_loot::Open {
    ac_loot::Open {
        guid,
        name: "Corpse of a Drudge Skulker".into(),
        open: true,
        items: vec![ac_loot::Lying {
            guid: item,
            name: "Dagger".into(),
            burden: 10,
            verdict: ac_loot::Verdict::Take(LootAction::Keep),
            needs_no_slot: false,
        }],
        slots_free: 20,
        room_anywhere: 20,
        carry_room: 10_000,
        ..Default::default()
    }
}

#[test]
fn the_corpse_opened_after_one_was_given_up_on_is_not_written_off_on_its_first_step() {
    // Blargerton: a corpse that would not empty was given up on after
    // forty-five seconds, and the loot rules' clock went on running
    // from it. The next body he opened was shut on its first step
    // for having taken too long, and marked looted with everything
    // still on it.
    let t0 = Instant::now();
    let mut ap = Autoplay::default();
    let (first, second) = (0x8000_1001, 0x8000_1002);
    ap.take_up_corpse(first, t0, LOOT_TIMEOUT);
    let next = ap.loot_run.step(&corpse_at_hand(first, 1), t0);
    assert_eq!(next.act, Some(ac_loot::Act::Take(1)), "{}", next.saying);
    // Let go of some way other than a shut by the rules: given up on
    // as it once was, or asked again when it would not open.
    let gave_up = t0 + ac_loot::run::KEEP_AT_IT + Duration::from_secs(1);
    ap.let_go_of_corpse();
    assert_eq!(ap.corpse, None);
    // The next corpse is chosen, and opens a moment later.
    ap.take_up_corpse(second, gave_up, LOOT_TIMEOUT);
    let opened = gave_up + Duration::from_secs(1);
    let next = ap.loot_run.step(&corpse_at_hand(second, 2), opened);
    assert_eq!(next.act, Some(ac_loot::Act::Take(2)), "{}", next.saying);
    assert!(!ap.looted.contains(&second), "written off unlooted");
}

#[test]
fn the_words_a_body_is_refused_in_say_how_long_to_leave_it() {
    // 1,044 refusals arrived in that run, a third of a second after
    // the ask, and every one of them was thrown away: the take-up
    // ended on a clock instead, and asked again three times more.
    let t0 = Instant::now();
    let name = "Corpse of Hellion";
    let answered = t0 + Duration::from_millis(360);
    let in_use = format!("The {name} is already in use by someone else!");
    let not_ours = format!("You do not yet have the right to loot the {name}.");
    let (held, locked, rare) = (0x8000_1221, 0x8000_1222, 0x8000_1223);
    let mut ap = Autoplay {
        corpse_seen: vec![(held, t0), (locked, t0), (rare, t0)],
        ..Default::default()
    };

    // Someone is inside it: let it go and have it the moment they
    // are done, not in the half minute anything blocked waits.
    ap.take_up_corpse(held, t0, LOOT_TIMEOUT);
    assert!(ap.corpse_refused_in_words(&in_use, name, Some(answered), answered));
    assert_eq!(ap.corpse, None, "still holding a body it cannot open");
    assert_eq!(ap.shelved.waited(&held), Some(OPEN_IN_USE_AGAIN));
    assert!(ap.shelved.held(
        &held,
        answered + OPEN_IN_USE_AGAIN - Duration::from_millis(1)
    ));
    assert!(!ap.shelved.held(&held, answered + OPEN_IN_USE_AGAIN));

    // The killer's for now: a short wait, because a corpse becomes
    // everyone's the moment whoever has it closes it and not only
    // when it half rots.
    ap.take_up_corpse(locked, t0, LOOT_TIMEOUT);
    assert!(ap.corpse_refused_in_words(&not_ours, name, Some(answered), answered));
    assert_eq!(ap.corpse, None);
    assert_eq!(ap.shelved.waited(&locked), Some(OPEN_NOT_OURS_AGAIN));
    assert!(ap.shelved.held(
        &locked,
        answered + OPEN_NOT_OURS_AGAIN - Duration::from_millis(1)
    ));
    assert!(!ap.shelved.held(&locked, answered + OPEN_NOT_OURS_AGAIN));

    // A body refused twice, in two different sets of words, waits
    // longer the second time -- and the wait it is given is a wait
    // and not a deadline. Handing the shelf an exact "come back in
    // 117 seconds" doubled the three seconds already on the body
    // instead: six, then twelve, then twenty-four, five more asks
    // and five more refusals before it reached the wait that was
    // meant the first time.
    let both = 0x8000_1224;
    ap.corpse_seen.push((both, t0));
    ap.take_up_corpse(both, t0, LOOT_TIMEOUT);
    assert!(ap.corpse_refused_in_words(&in_use, name, Some(answered), answered));
    assert_eq!(ap.shelved.waited(&both), Some(OPEN_IN_USE_AGAIN));
    let again = answered + OPEN_IN_USE_AGAIN;
    ap.take_up_corpse(both, again, LOOT_TIMEOUT);
    let told = again + Duration::from_millis(360);
    assert!(ap.corpse_refused_in_words(&not_ours, name, Some(told), told));
    assert_eq!(
        ap.shelved.waited(&both),
        Some(OPEN_IN_USE_AGAIN * 2),
        "a doubling wait, not a deadline the shelf then doubled"
    );

    // The killer's for good: not waited on at all.
    ap.take_up_corpse(rare, t0, LOOT_TIMEOUT);
    let words =
        format!("You may not loot the {name} because the {name} has generated a rare item.");
    assert!(ap.corpse_refused_in_words(&words, name, Some(answered), answered));
    assert!(ap.shelved.held(&rare, t0 + CORPSE_LIFE));
}

#[test]
fn a_body_reached_with_a_full_pack_is_set_aside_and_owed_again_after_the_sale() {
    // A pack down to the slots kept for a counter's money: every body
    // in reach was walked to, shut on the spot as done with -- so
    // written off -- and the town run waited behind them. After the
    // sale none of them was gone back to, with minutes left on them.
    let t0 = Instant::now();
    let mut ap = Autoplay::default();
    let (me, at) = (glam::Vec3::ZERO, glam::Vec3::new(5.0, 0.0, 0.0));
    let body = 0x8000_8001;
    let full = Room {
        pack_low: true,
        ..Room::PLENTY
    };
    // No room, so no body is owed: none is walked to, and the fight is
    // not held for one.
    assert!(ap.corpse_owed(body, at, me, t0, Room::PLENTY));
    assert!(!ap.corpse_owed(body, at, me, t0, full), "owed with no room");
    // One already open when the pack filled is shut and set aside.
    ap.take_up_corpse(body, t0, LOOT_TIMEOUT);
    let mut open = corpse_at_hand(body, 1);
    open.slots_free = 3;
    open.room_anywhere = 3;
    open.keep_free = 3;
    let next = ap.loot_run.step(&open, t0);
    assert_eq!(next.act, Some(ac_loot::Act::Close), "{}", next.saying);
    ap.corpse_shut(
        body,
        &next.did,
        next.left_for_weight,
        ShutFor::default(),
        t0,
    );
    assert!(!ap.looted.contains(&body), "written off for good");
    // Sold down a few minutes later, it is owed again.
    let sold = t0 + Duration::from_secs(4 * 60);
    assert!(
        ap.corpse_owed(body, at, me, sold, Room::PLENTY),
        "never gone back to"
    );
}

#[test]
fn a_body_left_for_its_weight_waits_on_room_not_on_a_clock() {
    // Laden, a character shut every body with something heavy on it as
    // too laden and set it aside for half a minute. As each wait ran
    // out it went back, opened the body, took nothing and shut it
    // again, holding the next fight for it every time, for as long as
    // the body lay there.
    use crate::did::Did;
    let t0 = Instant::now();
    let s = Duration::from_secs;
    let mut ap = Autoplay::default();
    let (me, at) = (glam::Vec3::ZERO, glam::Vec3::new(5.0, 0.0, 0.0));
    let (body, rotted) = (0x8000_9001, 0x8000_9002);
    let mut open = corpse_at_hand(body, 1);
    open.items[0].burden = 300;
    open.carry_room = 40;
    ap.take_up_corpse(body, t0, LOOT_TIMEOUT);
    let next = ap.loot_run.step(&open, t0);
    assert_eq!(next.act, Some(ac_loot::Act::Close), "{}", next.saying);
    ap.corpse_shut(
        body,
        &next.did,
        next.left_for_weight,
        ShutFor::default(),
        t0,
    );
    assert!(!ap.looted.contains(&body), "written off for good");
    // Still forty short: not owed, however long it has waited.
    let laden = Room {
        carry: 40,
        ..Room::PLENTY
    };
    assert!(!ap.corpse_owed(body, at, me, t0 + s(31), laden));
    assert!(!ap.corpse_owed(body, at, me, t0 + s(4 * 60), laden));
    // It is what the character weighs its room against to count itself
    // laden, but only while the body is still lying there.
    ap.left_for_weight.insert(rotted, 20);
    assert_eq!(ap.lightest_left_for_weight(|g| g == body), Some(300));
    // Sold down: owed again at once.
    let sold = Room {
        carry: 4_000,
        ..Room::PLENTY
    };
    assert!(
        ap.corpse_owed(body, at, me, t0 + s(4 * 60), sold),
        "never gone back to"
    );
    // Emptied then, it is done with, and waits on nothing.
    ap.corpse_shut(body, &Did::Done, None, ShutFor::default(), t0 + s(4 * 60));
    assert_eq!(ap.lightest_left_for_weight(|g| g == body), None);
    // A body that has rotted is forgotten.
    ap.forget_corpses_gone(|g| g == body);
    assert!(ap.left_for_weight.is_empty());
}

#[test]
fn every_body_opened_is_counted_for_the_panel_however_it_was_let_go() {
    // Blargerton's log said "emptied" whether he took something or
    // nothing. The panel's count tells the two apart, and a body given
    // up on still counts for what came off it.
    use crate::did::Did;
    let t0 = Instant::now();
    let mut ap = Autoplay::default();
    // Opened, the dagger asked for, then given up on.
    let first = 0x8000_3001;
    ap.take_up_corpse(first, t0, LOOT_TIMEOUT);
    let next = ap.loot_run.step(&corpse_at_hand(first, 1), t0);
    assert_eq!(next.act, Some(ac_loot::Act::Take(1)), "{}", next.saying);
    ap.let_go_of_corpse();
    // Shut by the rules with nothing worth taking on it.
    let second = 0x8000_3002;
    ap.take_up_corpse(second, t0, LOOT_TIMEOUT);
    let mut bare = corpse_at_hand(second, 2);
    bare.items[0].verdict = ac_loot::Verdict::Leave;
    let next = ap.loot_run.step(&bare, t0);
    assert_eq!(next.did, Did::Done, "{}", next.saying);
    ap.corpse_shut(
        second,
        &next.did,
        next.left_for_weight,
        ShutFor::default(),
        t0,
    );
    // Walked to and never opened, then the next body taken up.
    let third = 0x8000_3003;
    ap.take_up_corpse(third, t0, LOOT_TIMEOUT);
    let mut unopened = corpse_at_hand(third, 3);
    unopened.open = false;
    ap.loot_run.step(&unopened, t0);
    ap.take_up_corpse(0x8000_3004, t0, LOOT_TIMEOUT);
    assert_eq!(
        ap.loot_tally,
        ac_loot::Tally {
            opened: 2,
            taken: 1
        }
    );
}

/// A character with a mace in hand and a wand in the pack, and
/// nothing the server is waiting on: no swing out, no spell in the
/// air.
fn hands_free(assets: std::rc::Rc<ac_scene::Assets>) -> Client {
    let (mut c, _) = mid_fight(assets);
    c.attack_target = None;
    c.attack_pending = false;
    c.autoplay.cast_sent = None;
    assert!(!c.server_busy(Instant::now()), "nothing owed to the server");
    c
}

#[test]
fn the_weapon_already_in_the_hand_is_not_asked_for_again() {
    // All 71 "You must remove your X to wield Y" lines of a
    // ten-minute run had X and Y the same item: the client asking
    // the server to wield what it was already holding.
    let mut c = hands_free(no_data());
    const MACE: u32 = 0x8000_0101;
    assert!(c.wield_guid(MACE), "the mace is in hand, so the ask stands");
    assert_eq!(c.autoplay.wield_asked, None, "and nothing was sent");
}

#[test]
fn one_wield_goes_out_once_however_often_it_is_asked_for() {
    let mut c = hands_free(no_data());
    const WAND: u32 = 0x8000_0102;
    assert!(c.wield_guid(WAND), "the wand is asked for");
    let first = c.autoplay.wield_asked;
    assert!(matches!(first, Some((WAND, _))));
    // The client thinks at 8 Hz and the answer takes a few hundred
    // milliseconds. Three ticks of asking used to be three sends,
    // and the server refused the last two for the first having
    // worked.
    assert!(c.wield_guid(WAND), "the ask already stands");
    assert!(c.wield_guid(WAND));
    assert_eq!(c.autoplay.wield_asked, first, "nothing else was sent");
    // A wield the server never answers at all is asked for again.
    c.autoplay.wield_asked = Some((WAND, Instant::now() - WIELD_ANSWERS_IN));
    assert!(c.wield_guid(WAND));
    assert_ne!(c.autoplay.wield_asked, first, "asked again once stale");
}

#[test]
fn a_refused_wield_that_worked_forgets_the_wait_rather_than_doubling_it() {
    let mut c = hands_free(no_data());
    const WAND: u32 = 0x8000_0102;
    let now = Instant::now();
    // The wand was asked for twice and taken up once. The second
    // ask comes back refused, and the world already shows the wand
    // in hand.
    a_weapon(&mut c, WAND, ac_world::item_type::CASTER, "Wand", true);
    c.autoplay.wield_asked = Some((WAND, now));
    c.wield_refused(WAND, now);
    assert!(
        !c.wield_held_off(WAND),
        "the wield worked; there is nothing to wait for"
    );
    assert_eq!(c.autoplay.wield_asked, None, "and the ask is answered");

    // A refusal for something still in the pack is a real refusal
    // and still earns its wait.
    a_weapon(&mut c, WAND, ac_world::item_type::CASTER, "Wand", false);
    c.autoplay.wield_asked = Some((WAND, now));
    c.wield_refused(WAND, now);
    assert!(c.wield_held_off(WAND), "left alone for a while");
}

#[test]
fn nothing_is_sent_to_move_an_item_while_the_server_has_us_busy() {
    // ACE refuses a take made while the character is busy and spends
    // two messages saying so -- YoureTooBusy and an
    // InventoryServerSaveFailed with nothing in it. Nine characters
    // bought 89 of those pairs in ten minutes, taking from a body
    // with a spell in the air.
    let mut c = hands_free(no_data());
    const WAND: u32 = 0x8000_0102;
    const LOOT: u32 = 0x8000_0105;
    let now = Instant::now();
    c.autoplay.cast_sent = Some(now);
    assert!(c.server_busy(now));

    assert!(!c.wield_guid(WAND), "the wand waits for a free tick");
    assert_eq!(c.autoplay.wield_asked, None, "nothing sent mid-cast");

    c.loot_queue.push_back(LOOT);
    c.tick_loot(now);
    assert!(c.loot_inflight.is_none(), "the take waits too");
    assert_eq!(
        c.loot_queue.front(),
        Some(&LOOT),
        "and keeps its place in the queue"
    );

    // A swing in the air is a different matter. ACE sets no IsBusy
    // for one -- nothing in its melee or missile path does, and the
    // wield handler does not read it at all -- so the take goes out.
    // The wield still waits, because a wield mid-swing lands and
    // cancels the swing doing it.
    c.autoplay.cast_sent = None;
    c.attack_pending = true;
    c.last_attack = now;
    assert!(!c.server_busy(now), "a swing is not the server being busy");
    assert!(c.wield_must_wait(WAND, now), "not worth a cancelled swing");
    assert!(!c.wield_guid(WAND));
    c.tick_loot(now);
    assert_eq!(
        c.loot_inflight.map(|(g, _)| g),
        Some(LOOT),
        "the take was held back for a rule the server does not have"
    );
    assert!(c.loot_queue.is_empty());

    // The swing lands, and the wand goes out too.
    c.attack_pending = false;
    assert!(c.wield_guid(WAND));
}

/// A Sack hanging from the main pack, with `capacity` slots.
const SACK: u32 = 0x8000_0300;
/// A body at the character's feet.
const BODY: u32 = 0x8000_0400;

/// A character whose main pack has `main_slots` slots, `main_used`
/// of them taken by daggers, and a Sack of `sack_slots` slots with
/// `sack_used` daggers in it.
fn with_packs(
    assets: std::rc::Rc<ac_scene::Assets>,
    main_slots: u32,
    main_used: u32,
    sack_slots: u32,
    sack_used: u32,
) -> Client {
    let mut c = character_of_level(assets, 20);
    let me = c.world.player_guid.unwrap();
    c.world.objects.insert(
        me,
        ac_world::WorldObject {
            guid: me,
            name: "Verity".into(),
            is_player: true,
            items_capacity: main_slots,
            ..Default::default()
        },
    );
    c.world.objects.insert(
        SACK,
        ac_world::WorldObject {
            guid: SACK,
            name: "Sack".into(),
            weenie_class_id: 166,
            item_type: ac_world::item_type::CONTAINER,
            items_capacity: sack_slots,
            container: Some(me),
            ..Default::default()
        },
    );
    let mut next = 0x8000_0500;
    for (holder, n) in [(me, main_used), (SACK, sack_used)] {
        for _ in 0..n {
            c.world.objects.insert(
                next,
                ac_world::WorldObject {
                    guid: next,
                    name: "Dagger".into(),
                    container: Some(holder),
                    ..Default::default()
                },
            );
            next += 1;
        }
    }
    c.world.objects.insert(
        BODY,
        ac_world::WorldObject {
            guid: BODY,
            name: "Corpse of a Drudge Skulker".into(),
            object_desc_flags: ac_world::object_desc_flags::CORPSE,
            ..Default::default()
        },
    );
    // No slots kept back for a counter's money: these are about the
    // packs being full, not low.
    c.autoplay.config.team.restock.keep_slots = 0;
    assert!(!c.server_busy(Instant::now()));
    c
}

/// A thing of `wcid` lying on the body, `count` to the stack.
fn on_the_body(c: &mut Client, guid: u32, name: &str, wcid: u32, kind: u32, count: u32) {
    c.world.objects.insert(
        guid,
        ac_world::WorldObject {
            guid,
            name: name.into(),
            weenie_class_id: wcid,
            item_type: kind,
            stack_size: count,
            max_stack_size: if count > 1 { 25_000 } else { 1 },
            container: Some(BODY),
            ..Default::default()
        },
    );
    match &mut c.world.open_container {
        Some((body, items)) if *body == BODY => items.push(guid),
        _ => c.world.open_container = Some((BODY, vec![guid])),
    }
}

#[test]
fn free_space_is_what_one_take_can_use_and_not_the_sum_over_the_packs() {
    // Main pack 2 of 4, Sack 19 of 24: seventeen free in all, and
    // five for any one take.
    let c = with_packs(no_data(), 4, 2, 24, 19);
    let me = c.world.player_guid.unwrap();
    assert_eq!(c.free_space(), 5);
    assert_eq!(c.room_anywhere(), 7);
    assert!(!c.pack_full());
    let packs = c.packs();
    assert_eq!((packs.main.capacity, packs.main.used), (4, 2));
    assert_eq!(packs.side.len(), 1);
    assert_eq!((packs.side[0].guid, packs.side[0].used), (SACK, 19));
    // The Sack sits in a pack slot, not an item slot.
    assert_eq!(packs.container_for_a_take(), Some(me));
    // Every pack down to its last slot: room for one take, however
    // many packs there are.
    let mut c = with_packs(no_data(), 4, 3, 24, 23);
    assert_eq!(c.free_space(), 1);
    assert_eq!(c.room_anywhere(), 2);
    // A character with an empty main pack reads as it always did.
    c.world
        .objects
        .retain(|_, o| o.name != "Dagger" && o.guid != SACK);
    assert_eq!(c.free_space(), 4);
    assert_eq!(c.room_anywhere(), 4);
}

#[test]
fn a_take_with_the_main_pack_full_goes_into_the_sack_with_room() {
    // Nine characters at 102/102 with a Sack at 7 of 24 aimed every
    // take at the main pack and were refused seven thousand times.
    let mut c = with_packs(no_data(), 2, 2, 24, 7);
    let me = c.world.player_guid.unwrap();
    const DAGGER: u32 = 0x8000_0401;
    on_the_body(&mut c, DAGGER, "Dagger", 21, 0, 1);
    assert_eq!(c.free_space(), 17);
    assert_eq!(c.how_to_take(DAGGER), Some(crate::room::Take::Put(SACK)));
    let now = Instant::now();
    c.take(DAGGER);
    c.tick_loot(now);
    let sent = c.loot_sent.clone().expect("a take went out");
    assert_eq!((sent.item, sent.into, sent.retried), (DAGGER, SACK, false));
    assert!(c.loot_merge.is_none(), "a put, not a pour");
    // Landed in the Sack: ours, though not in our own container.
    c.world.objects.get_mut(&DAGGER).unwrap().container = Some(SACK);
    c.tick_loot(now + Duration::from_millis(100));
    assert!(c.loot_inflight.is_none(), "the take is over");
    assert!(c.loot_sent.is_some(), "kept until the next take goes out");
    // With a slot in the main pack, that comes first, as ever.
    c.world.objects.get_mut(&me).unwrap().items_capacity = 4;
    assert_eq!(c.how_to_take(DAGGER), Some(crate::room::Take::Put(me)));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn with_every_pack_full_the_body_is_set_aside_once_and_not_reopened() {
    let mut c = with_packs(game_data(), 2, 2, 3, 3);
    const DAGGER: u32 = 0x8000_0401;
    on_the_body(&mut c, DAGGER, "Dagger", 21, 0, 1);
    let t0 = Instant::now();
    assert_eq!(c.free_space(), 0);
    let room = c.room_for_loot();
    assert!(room.pack_low, "no pack has a slot");
    // The body at its feet is not waited on, and the rules shut one
    // already open as full, setting it aside rather than writing
    // it off.
    assert!(!c.autoplay.corpse_waiting(BODY, t0, room));
    let profile = crate::profile::Profile {
        rules: vec![asks("daggers", "dagger", LootAction::Sell)],
        ..Default::default()
    };
    c.autoplay.take_up_corpse(BODY, t0, LOOT_TIMEOUT);
    let open = c.corpse_now(BODY, &[DAGGER], &profile, t0, t0);
    assert_eq!(open.slots_free, 0);
    let next = c.autoplay.loot_run.step(&open, t0);
    assert_eq!(next.act, Some(ac_loot::Act::Close), "{}", next.saying);
    assert!(matches!(&next.did, crate::did::Did::Blocked(b) if b.what == "the pack is full"));
    c.autoplay.corpse_shut(
        BODY,
        &next.did,
        next.left_for_weight,
        ShutFor::default(),
        t0,
    );
    assert!(!c.autoplay.looted.contains(&BODY), "written off for good");
    // Set aside for room: with none, it is not gone back to however
    // long it lies there.
    let later = t0 + Duration::from_secs(10 * 60);
    assert!(!c.autoplay.corpse_waiting(BODY, later, c.room_for_loot()));
    // Room appears -- a dagger sold -- and it waits again.
    c.world.objects.remove(&0x8000_0500);
    let room = c.room_for_loot();
    assert!(!room.pack_low);
    assert!(c.autoplay.corpse_waiting(BODY, later, room));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_servers_word_that_a_pack_is_full_sends_the_take_elsewhere_and_then_gives_up() {
    // The count said the main pack had two slots; the server said
    // "Unable to put Dagger into container". Believed, the next try
    // names the Sack; refused there too, the body is left for room.
    let mut c = with_packs(game_data(), 4, 2, 24, 23);
    let me = c.world.player_guid.unwrap();
    const DAGGER: u32 = 0x8000_0401;
    on_the_body(&mut c, DAGGER, "Dagger", 21, 0, 1);
    let t0 = Instant::now();
    c.take(DAGGER);
    c.tick_loot(t0);
    assert_eq!(c.loot_sent.as_ref().map(|s| s.into), Some(me));
    // In the order a tick reads them: the InventoryServerSaveFailed
    // with no reason in it first, with the words noted as said but
    // not yet read; then the words themselves, from the chat.
    let answered = t0 + Duration::from_millis(50);
    c.told = Some(answered);
    c.move_refused.insert(DAGGER, (0, answered));
    c.loot_inflight = None;
    c.take_refused(DAGGER, 0);
    assert!(
        c.packs_said_full.is_empty(),
        "something was said, and not read yet"
    );
    // Words about some other take are not about this one.
    c.hear_put_refusal("Pyreal");
    assert!(c.packs_said_full.is_empty());
    c.hear_put_refusal("Dagger");
    assert_eq!(
        c.packs_said_full.get(&me),
        Some(&2),
        "full at two, whatever the count said"
    );
    assert!(c.loot_sent.is_none(), "judged, and not read twice");
    assert_eq!(c.free_space(), 1, "the Sack's slot is all there is");
    assert_eq!(c.packs().container_for_a_take(), Some(SACK));
    assert_eq!(c.loot_queue.front(), Some(&DAGGER), "sent once more");
    assert_eq!(c.loot_retry, Some(DAGGER));
    assert!(
        !c.move_refused.contains_key(&DAGGER),
        "the refusal was the pack's, not the item's"
    );
    c.tick_loot(answered + Duration::from_millis(10));
    let sent = c.loot_sent.clone().expect("the second try");
    assert_eq!((sent.into, sent.retried), (SACK, true));
    // Refused there as well, in the same words.
    let again = answered + Duration::from_millis(60);
    c.move_refused.insert(DAGGER, (0, again));
    c.loot_inflight = None;
    c.take_refused(DAGGER, 0);
    c.hear_put_refusal("Dagger");
    assert_eq!(c.packs_said_full.get(&SACK), Some(&23));
    assert_eq!(c.free_space(), 0);
    assert!(c.loot_queue.is_empty(), "no third try");
    assert!(
        c.move_refused.contains_key(&DAGGER),
        "the rules read the refusal"
    );
    assert!(c.room_for_loot().pack_low, "the body is set aside for room");
    // Something leaves the main pack, and the server's word on it
    // lapses: the count is believed again until the server says
    // otherwise.
    c.world.objects.remove(&0x8000_0500);
    assert_eq!(c.free_space(), 3);
    assert_eq!(c.packs().container_for_a_take(), Some(me));
    assert!(
        !c.room_for_loot().pack_low,
        "room appeared; bodies wait again"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_refusal_with_no_reason_and_no_word_at_all_is_not_the_pack() {
    // A second of a unique: the server refuses it with no code, and
    // explains itself in the system chat, which is not a word about
    // the take. Read as the pack being full, it marked the main
    // pack, then the Sack, and sent the character to town with
    // seventeen slots free.
    let mut c = with_packs(game_data(), 4, 2, 24, 7);
    let me = c.world.player_guid.unwrap();
    const KEY: u32 = 0x8000_0401;
    on_the_body(&mut c, KEY, "Sturdy Iron Key", 9000, 0, 1);
    let t0 = Instant::now();
    c.take(KEY);
    c.tick_loot(t0);
    assert_eq!(c.loot_sent.as_ref().map(|s| s.into), Some(me));
    let answered = t0 + Duration::from_millis(50);
    c.move_refused.insert(KEY, (0, answered));
    c.loot_inflight = None;
    c.take_refused(KEY, 0);
    // And nothing more is said. The next tick finds nothing to send
    // and nothing to mark.
    c.tick_loot(answered);
    assert!(c.packs_said_full.is_empty(), "no word: not the pack");
    assert!(c.loot_queue.is_empty(), "and no second try");
    assert!(c.loot_inflight.is_none(), "no take in the air");
    assert!(
        c.move_refused.contains_key(&KEY),
        "the refusal was the item's, for the rules to read"
    );
    assert_eq!(c.free_space(), 17);
    assert!(!c.room_for_loot().pack_low);
}

#[test]
fn the_servers_words_count_when_they_come_a_packet_behind_the_refusal() {
    // The refusal in one packet, the words in the next: the tick
    // between has read the take as over. What was sent is kept
    // until the next goes out, so the words still find it.
    let mut c = with_packs(no_data(), 4, 2, 24, 7);
    let me = c.world.player_guid.unwrap();
    const DAGGER: u32 = 0x8000_0401;
    on_the_body(&mut c, DAGGER, "Dagger", 21, 0, 1);
    let t0 = Instant::now();
    c.take(DAGGER);
    c.tick_loot(t0);
    let answered = t0 + Duration::from_millis(50);
    c.move_refused.insert(DAGGER, (0, answered));
    c.loot_inflight = None;
    c.take_refused(DAGGER, 0);
    c.tick_loot(answered);
    assert!(c.loot_inflight.is_none());
    assert!(c.loot_sent.is_some(), "kept for the words");
    c.hear_put_refusal("Dagger");
    assert_eq!(c.packs_said_full.get(&me), Some(&2));
    assert_eq!(c.loot_queue.front(), Some(&DAGGER), "sent once more");
}

#[test]
fn the_servers_word_lifts_when_one_thing_leaves_however_the_count_climbed() {
    // Called full when the client had counted two; two descriptions
    // still on their way arrive, and the count reads four. One
    // sold: the word held at two would still stand, and the pack
    // would read as full until a third left.
    let mut c = with_packs(no_data(), 4, 2, 24, 24);
    let me = c.world.player_guid.unwrap();
    c.packs_said_full.insert(me, 2);
    let t0 = Instant::now();
    assert_eq!(c.free_space(), 0);
    for guid in [0x8000_0700, 0x8000_0701] {
        c.world.objects.insert(
            guid,
            ac_world::WorldObject {
                guid,
                name: "Dagger".into(),
                container: Some(me),
                ..Default::default()
            },
        );
    }
    c.tick_loot(t0);
    assert_eq!(
        c.packs_said_full.get(&me),
        Some(&4),
        "kept at the most seen"
    );
    assert_eq!(c.free_space(), 0);
    c.world.objects.remove(&0x8000_0700);
    c.tick_loot(t0 + Duration::from_millis(125));
    assert!(
        !c.packs_said_full.contains_key(&me),
        "one left: the word lapses"
    );
    assert_eq!(c.free_space(), 1);
}

#[test]
fn a_pack_off_the_ground_is_picked_up_into_the_main_pack_whatever_the_room() {
    // The main pack full and the Sack with room: a dagger on the
    // ground goes into the Sack, and a Pouch into the main pack's
    // pack slots, where the server would refuse it named into the
    // Sack without a word.
    let mut c = with_packs(no_data(), 2, 2, 24, 7);
    let me = c.world.player_guid.unwrap();
    const POUCH: u32 = 0x8000_0801;
    const DAGGER: u32 = 0x8000_0802;
    c.world.objects.insert(
        POUCH,
        ac_world::WorldObject {
            guid: POUCH,
            name: "Pouch".into(),
            weenie_class_id: 167,
            item_type: ac_world::item_type::CONTAINER,
            items_capacity: 12,
            position: Some(ac_world::object::Position::new_flat(0, Default::default())),
            ..Default::default()
        },
    );
    c.world.objects.insert(
        DAGGER,
        ac_world::WorldObject {
            guid: DAGGER,
            name: "Dagger".into(),
            position: Some(ac_world::object::Position::new_flat(0, Default::default())),
            ..Default::default()
        },
    );
    assert_eq!(c.where_to_pick_up(POUCH), Some(me));
    assert_eq!(c.where_to_pick_up(DAGGER), Some(SACK));
    assert_eq!(c.where_to_pick_up(0x8000_0803), None, "unknown");
}

#[test]
fn a_refusal_with_a_reason_or_other_words_is_not_read_as_a_full_pack() {
    let mut c = with_packs(no_data(), 4, 2, 24, 7);
    let me = c.world.player_guid.unwrap();
    const DAGGER: u32 = 0x8000_0401;
    on_the_body(&mut c, DAGGER, "Dagger", 21, 0, 1);
    let t0 = Instant::now();
    c.take(DAGGER);
    c.tick_loot(t0);
    // "You are too encumbered to carry that!" and the usual
    // reasonless refusal after it.
    let answered = t0 + Duration::from_millis(50);
    c.told = Some(answered);
    c.move_refused.insert(DAGGER, (0, answered));
    c.loot_inflight = None;
    c.take_refused(DAGGER, 0);
    assert!(c.packs_said_full.is_empty(), "the pack was not the reason");
    assert!(c.loot_queue.is_empty(), "and it is not asked for again");
    assert!(c.move_refused.contains_key(&DAGGER));
    // A quest's cap, with its code.
    c.take(DAGGER);
    c.tick_loot(answered);
    c.loot_inflight = None;
    c.take_refused(DAGGER, 0x043E);
    assert!(c.packs_said_full.is_empty());
    assert_eq!(c.free_space(), 17);
    assert_eq!(c.packs().container_for_a_take(), Some(me));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn coin_off_a_body_is_poured_onto_the_pile_carried_rather_than_given_a_slot() {
    // The main pack full but for the pyreals in it: the body's
    // coin joins the pile, spending no slot, and the rules take it
    // off a body they would otherwise shut as full.
    let mut c = with_packs(game_data(), 2, 1, 0, 0);
    c.world.objects.remove(&SACK);
    let me = c.world.player_guid.unwrap();
    const PILE: u32 = 0x8000_0600;
    const COIN: u32 = 0x8000_0401;
    c.world.objects.insert(
        PILE,
        ac_world::WorldObject {
            guid: PILE,
            name: "Pyreal".into(),
            weenie_class_id: 273,
            item_type: ac_world::item_type::MONEY,
            stack_size: 100,
            max_stack_size: 25_000,
            container: Some(me),
            ..Default::default()
        },
    );
    on_the_body(&mut c, COIN, "Pyreal", 273, ac_world::item_type::MONEY, 50);
    assert_eq!(c.free_space(), 0, "the pile took the last slot");
    assert_eq!(
        c.how_to_take(COIN),
        Some(crate::room::Take::Merge {
            to: PILE,
            amount: 50
        })
    );
    // The rules see it needs no slot and ask for it.
    let t0 = Instant::now();
    let profile = crate::profile::Profile {
        rules: vec![asks("coin", "pyreal", LootAction::Keep)],
        ..Default::default()
    };
    c.autoplay.take_up_corpse(BODY, t0, LOOT_TIMEOUT);
    let open = c.corpse_now(BODY, &[COIN], &profile, t0, t0);
    assert_eq!(open.slots_free, 0);
    assert!(open.items[0].needs_no_slot);
    let next = c.autoplay.loot_run.step(&open, t0);
    assert_eq!(next.act, Some(ac_loot::Act::Take(COIN)), "{}", next.saying);
    // Not while the tidying's own pour is in the air: the pile it
    // is emptying, or filling, may be the one the coin would join.
    c.take(COIN);
    c.autoplay.pour = Some((
        crate::pack::PourSent {
            merge: crate::pack::Merge {
                from: 0x8000_0601,
                to: PILE,
                amount: 10,
                name: "Pyreal".into(),
                frees_a_slot: true,
            },
            to_before: 100,
        },
        t0,
    ));
    c.tick_loot(t0);
    assert!(c.loot_sent.is_none(), "held while the tidying pours");
    assert_eq!(c.loot_queue.front(), Some(&COIN));
    c.autoplay.pour = None;
    // The take is a pour off the body, read like a tidying pour.
    c.tick_loot(t0);
    let pour = c.loot_merge.clone().expect("a pour went out");
    assert_eq!(
        (pour.merge.from, pour.merge.to, pour.merge.amount),
        (COIN, PILE, 50)
    );
    assert_eq!(pour.to_before, 100);
    assert_eq!(c.loot_sent.as_ref().map(|s| s.into), Some(PILE));
    // The source goes first; that is no answer yet.
    c.world.objects.remove(&COIN);
    c.tick_loot(t0 + Duration::from_millis(100));
    assert!(c.loot_inflight.is_some(), "the pile has not grown");
    // Nor is the tidying's two seconds without a word: the server
    // walks to the body and stoops for this as for a take, and one
    // take in ten took longer.
    c.tick_loot(t0 + Duration::from_millis(2_500));
    assert!(c.loot_inflight.is_some(), "waited on as a take is");
    // The pile grows: landed.
    c.world.objects.get_mut(&PILE).unwrap().stack_size = 150;
    c.tick_loot(t0 + Duration::from_millis(2_600));
    assert!(c.loot_inflight.is_none());
    assert!(c.loot_merge.is_some(), "kept until the next take goes out");
    // A pour the server turns down -- too heavy, by its reckoning --
    // is over, and the refusal is left for the rules to read: it is
    // not a pack being full, so nothing is marked.
    on_the_body(&mut c, COIN, "Pyreal", 273, ac_world::item_type::MONEY, 50);
    let t1 = t0 + Duration::from_secs(1);
    c.take(COIN);
    c.tick_loot(t1);
    assert!(c.loot_merge.is_some());
    c.move_refused
        .insert(COIN, (0, t1 + Duration::from_millis(50)));
    c.take_refused(COIN, 0);
    assert!(
        c.packs_said_full.is_empty(),
        "a pour refused says nothing of the packs"
    );
    c.tick_loot(t1 + Duration::from_millis(100));
    assert!(c.loot_inflight.is_none(), "the pour is over");
    assert!(c.move_refused.contains_key(&COIN));
    // A stack too big for the pile is put in a slot, when there is
    // one, and is not a thing that needs no slot.
    c.world.objects.get_mut(&COIN).unwrap().stack_size = 25_000;
    assert_eq!(c.how_to_take(COIN), None, "no slot, and no pile it fits");
    let open = c.corpse_now(BODY, &[COIN], &profile, t0, t1);
    assert!(!open.items[0].needs_no_slot);
}
