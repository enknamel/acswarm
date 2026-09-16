use crate::testkit::{character_of_level, game_data, no_data, standing_in_the_field};

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

#[test]
fn a_pour_refused_while_a_take_is_in_the_air_is_not_the_takes_refusal() {
    // The tidying pours in the gaps between takes, so both can be
    // out at once. A refusal that names a stack in the pack is
    // about that stack, and the take goes on waiting for its own
    // answer rather than being written off.
    let (in_the_pack, on_the_corpse) = (0x8000_7001, 0x8000_7002);
    assert_eq!(
        refused_item(in_the_pack, 0, Some(on_the_corpse)),
        Some(in_the_pack)
    );
}

#[test]
fn a_profile_with_tidy_pack_off_is_never_tidied() {
    assert_eq!(
        why_not_tidy(TidyGate {
            tidy_pack_off: true,
            ..TidyGate::default()
        }),
        Some("this profile leaves the pack as it is")
    );
}

#[test]
fn nothing_is_poured_with_a_counter_open() {
    // A sale holds what it has sent the vendor by guid, and a pour
    // makes one of those vanish out from under it.
    assert_eq!(
        why_not_tidy(TidyGate {
            counter_open: true,
            ..TidyGate::default()
        }),
        Some("a counter is open")
    );
}

#[test]
fn nor_while_the_quartermaster_is_loaded_or_unloaded() {
    // Money counted out into its own stack was poured straight back
    // into the pile it came from, a hundred and twenty six times.
    assert_eq!(
        why_not_tidy(TidyGate {
            quartermaster: true,
            ..TidyGate::default()
        }),
        Some("the quartermaster is being loaded or unloaded")
    );
}

#[test]
fn nor_while_ammunition_is_being_made() {
    assert_eq!(
        why_not_tidy(TidyGate {
            crafting: true,
            ..TidyGate::default()
        }),
        Some("ammunition is being made")
    );
}

#[test]
fn nor_just_after_a_hand_over_to_a_teammate() {
    assert_eq!(
        why_not_tidy(TidyGate {
            gave_lately: true,
            ..TidyGate::default()
        }),
        Some("something was just handed to a teammate")
    );
}

#[test]
fn nor_while_a_take_is_queued_or_in_the_air() {
    assert_eq!(
        why_not_tidy(TidyGate {
            take_in_air: true,
            ..TidyGate::default()
        }),
        Some("a take is queued or in the air")
    );
}

#[test]
fn with_nothing_in_the_way_the_pack_is_tidied() {
    assert_eq!(why_not_tidy(TidyGate::default()), None);
    // The first reason that applies is the one given.
    assert_eq!(
        why_not_tidy(TidyGate {
            counter_open: true,
            take_in_air: true,
            ..TidyGate::default()
        }),
        Some("a counter is open")
    );
}

#[test]
fn every_errand_holding_a_stack_is_offered_up() {
    // Whatever another part of the rules is holding across ticks
    // must not be poured away under it.
    let mut ap = Autoplay::default();
    assert_eq!(ap.held_by_an_errand().iter().flatten().count(), 0);
    ap.pending_wield = Some(1);
    ap.wanted_ammo = Some(2);
    ap.put_down = Some(3);
    ap.crafting = Some((4, 5, Instant::now()));
    ap.handing = Some((6, Instant::now()));
    let held: Vec<u32> = ap.held_by_an_errand().iter().flatten().copied().collect();
    assert_eq!(held, vec![1, 2, 3, 4, 5, 6]);
}

#[test]
fn a_daily_limit_is_a_wait_and_not_a_grudge() {
    use crate::did::{Because, Did, Patience};
    let t0 = Instant::now();
    let mut kinds: Patience<u32> = Patience::new();

    // YouHaveSolvedThisQuestTooRecently is what gates a once-a-day
    // drop. It is a wait: the thing comes back, and a session can
    // run for days.
    let too_recently = Did::Blocked(Because::server(0x043E));
    assert_eq!(too_recently.because().and_then(|b| b.code), Some(0x043E));
    kinds.note(7299, &too_recently, t0);
    assert!(kinds.held(&7299, t0), "left alone for now");
    // Not for ever, though: a day later it is asked about again.
    assert!(
        !kinds.held(&7299, t0 + Duration::from_secs(24 * 60 * 60)),
        "a day later it is worth another ask"
    );
    // Nor is "too many times", which a raised cap can lift.
    kinds.note(7299, &Did::Blocked(Because::server(0x043F)), t0);
    assert!(!kinds.held(&7299, t0 + Duration::from_secs(24 * 60 * 60)));
    // Only a thing no counter will ever take is for ever, and that
    // is a different answer entirely.
    kinds.note(1, &Did::refused("no vendor will take it"), t0);
    assert!(kinds.held(&1, t0 + Duration::from_secs(24 * 60 * 60)));
}

#[test]
fn a_buff_pass_that_wants_a_wand_waits_rather_than_disarming_mid_charge() {
    use crate::Stance;
    // +Verity's buffing reached for her Training Wand every second
    // and a half while she was charging a Drudge Servant, and every
    // reach put her mace away and cancelled the charge with it.
    assert!(change_of_hands_waits(Stance::Melee, Stance::Magic, true));
    assert!(change_of_hands_waits(Stance::Missile, Stance::Magic, true));

    // Answered, the gap between two swings is hers: the wand goes in
    // then, and nothing is cancelled.
    assert!(!change_of_hands_waits(Stance::Melee, Stance::Magic, false));

    // A pass that already holds a wand changes nothing, so there is
    // nothing to wait for and the buff goes up mid-fight as before.
    assert!(!change_of_hands_waits(Stance::Magic, Stance::Magic, true));

    // The rule is about hands, not about wands: a character told to
    // fight with a bow waits for the swing just the same.
    assert!(change_of_hands_waits(Stance::Melee, Stance::Missile, true));
}

#[test]
fn a_refused_wield_is_left_alone_for_longer_every_time() {
    use crate::did::Patience;
    // ACE refuses a wield it will not make with no error code at all
    // -- a caster cannot go in while a shield is up, and it says so
    // with WeenieError.None -- so there is nothing to read and
    // nothing to do but wait. +Verity asked 150 times in a minute.
    const WAND: u32 = 0x8000_00C3;
    let t0 = Instant::now();
    let mut held: Patience<u32> = Patience::new();

    held.hold(WAND, WIELD_AGAIN, t0);
    assert!(held.held(&WAND, t0), "not asked for again at once");
    assert!(
        held.held(&WAND, t0 + WIELD_AGAIN - Duration::from_millis(1)),
        "nor a moment before the wait is up"
    );
    assert!(
        !held.held(&WAND, t0 + WIELD_AGAIN),
        "asked again once the wait is up"
    );

    // Refused again: the wait doubles, so an item the server will
    // never wield in this state costs a handful of asks rather than
    // one every buff pass.
    let second = t0 + WIELD_AGAIN;
    held.hold(WAND, WIELD_AGAIN, second);
    assert_eq!(held.waited(&WAND), Some(WIELD_AGAIN * 2));
    assert!(held.held(&WAND, second + WIELD_AGAIN));
    assert!(!held.held(&WAND, second + WIELD_AGAIN * 2));

    // A wield that lands forgets the wait: the hands have changed,
    // so whatever the server was objecting to has gone.
    held.forget(&WAND);
    assert!(!held.held(&WAND, second));
    assert_eq!(held.waited(&WAND), None);
}

use super::*;
use crate::items::ItemStats;
use crate::refusals::{OPEN_IN_USE_AGAIN, OPEN_NOT_OURS_AGAIN};

#[test]
fn ammunition_is_made_for_the_bow_and_the_targets_weakness() {
    use ac_world::elements::Element;
    use ac_world::fletching::ammo_type;
    // Plain arrowheads (4586), fire arrowheads (5341), arrowshafts
    // (4585) and quarrel shafts (5339), by guid.
    let carried = [(4586, 1), (5341, 2), (4585, 3), (5339, 4)];
    // Fletching enough for fire arrows, against something weak to fire.
    let (r, heads, shafts) =
        choose_recipe(ammo_type::ARROW, 100, &carried, Some(Element::Fire)).expect("fire");
    assert_eq!(
        (r.result_name.as_str(), heads, shafts),
        ("Fire Arrow", 2, 3)
    );
    // Weak to cold and no cold heads carried: the hardest recipe
    // that can be made, which is still the fire one.
    let (r, _, _) =
        choose_recipe(ammo_type::ARROW, 100, &carried, Some(Element::Cold)).expect("any");
    assert_eq!(r.result_name, "Fire Arrow");
    // Not skilled enough for fire arrows: plain ones.
    let (r, heads, shafts) =
        choose_recipe(ammo_type::ARROW, 10, &carried, Some(Element::Fire)).expect("plain");
    assert_eq!((r.result_name.as_str(), heads, shafts), ("Arrow", 1, 3));
    // A crossbow wants quarrels, made on the quarrel shafts.
    let (r, _, shafts) = choose_recipe(ammo_type::BOLT, 100, &carried, None).expect("quarrels");
    assert_eq!((r.result_name.as_str(), shafts), ("Fire Quarrel", 4));
    // No dart shafts: nothing for an atlatl.
    assert!(choose_recipe(ammo_type::ATLATL, 100, &carried, None).is_none());
    // Untrained (0): nothing at all.
    assert!(choose_recipe(ammo_type::ARROW, 0, &carried, None).is_none());
}

#[test]
fn a_follower_strays_no_further_than_twice_its_distance() {
    assert_eq!(follow_break(4.0), 10.0);
    assert_eq!(follow_break(8.0), 16.0);
}

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

/// A table row made up for a test.
fn row(tolerance: u32, health: u32, level: Option<u32>) -> ac_world::elements::Creature {
    ac_world::elements::Creature {
        wcid: 0,
        name: String::new(),
        health,
        takes: [None; 8],
        tolerance,
        level,
    }
}

/// The table's row for this very weenie.
fn weenie(row: &ac_world::elements::Creature) -> Option<Hint<'_>> {
    Some(Hint::Weenie(row))
}

#[test]
fn a_creature_the_table_knows_is_a_critter_when_it_is_quiet_outgrown_and_small() {
    use ac_world::elements::{creature_by_id, tolerance};
    let quiet = Seen::default();
    let rabbit = row(tolerance::RETALIATE, 5, Some(4));
    // A level 4 Rabbit with its five health: fought at 7, walked
    // past from 8 up.
    for mine in [5, 7] {
        assert_eq!(
            critter(quiet, None, None, weenie(&rabbit), mine),
            Critter::Fight
        );
    }
    for mine in [8, 20] {
        assert_eq!(
            critter(quiet, None, None, weenie(&rabbit), mine),
            Critter::WalkPast
        );
    }
    // A level 61 Revenant is passive and nobody outgrows it: 122
    // is past the level a character can reach.
    let revenant = creature_by_id(8592).expect("in the table");
    assert_eq!(revenant.level, Some(61));
    for mine in [100, 121] {
        assert_eq!(
            critter(quiet, None, None, weenie(revenant), mine),
            Critter::Fight
        );
    }
    // Something the table says attacks on sight is fought however
    // small, and before it has noticed the character.
    assert_eq!(
        critter(quiet, None, None, weenie(&row(0, 3, Some(1))), 275),
        Critter::Fight
    );
    // Every flag that means it leaves a passer-by alone counts.
    for flag in [
        tolerance::NO_ATTACK,
        tolerance::APPRAISE,
        tolerance::PROVOKE,
        tolerance::RETALIATE,
        tolerance::MONSTER,
    ] {
        assert_eq!(
            critter(quiet, None, None, weenie(&row(flag, 5, Some(4))), 20),
            Critter::WalkPast,
            "{flag}"
        );
    }
    // Something that cannot fight back at all is scenery with a
    // health bar, and hitting it is a chore whatever it can take: a
    // Portal Pillar has two thousand health and never swings.
    assert_eq!(
        critter(
            quiet,
            None,
            None,
            weenie(&row(tolerance::NO_ATTACK, 2001, Some(4))),
            50
        ),
        Critter::WalkPast
    );
    // The rest of the flags do fight back once hit, so what they
    // can take is what decides: a Drudge Skulker's forty-two is a
    // fight and so is a Mite Snippet's twenty, a Rabbit's five is
    // not. The table's line, not the stranger's: a Mite Snippet
    // has a Cow's health, and only the table tells them apart.
    for health in [42, 20, CRITTER_HEALTH + 1] {
        assert_eq!(
            critter(
                quiet,
                None,
                None,
                weenie(&row(tolerance::RETALIATE, health, Some(8))),
                50
            ),
            Critter::Fight,
            "{health}"
        );
    }
    // The ones that do not: "only fight back at whoever started it"
    // still starts fights with everyone else.
    assert_eq!(
        critter(quiet, None, None, weenie(&row(32, 5, Some(4))), 20),
        Critter::Fight
    );
    // A character with no level yet fights everything.
    assert_eq!(
        critter(quiet, None, None, weenie(&rabbit), 0),
        Critter::Fight
    );
    // Nothing is walked past on a guess, nor attacked on one: a
    // row with no level, or no health, is a question for the
    // server.
    assert_eq!(
        critter(
            quiet,
            None,
            None,
            weenie(&row(tolerance::RETALIATE, 5, None)),
            275
        ),
        Critter::Appraise
    );
    assert_eq!(
        critter(
            quiet,
            None,
            None,
            weenie(&row(tolerance::RETALIATE, 0, Some(4))),
            275
        ),
        Critter::Appraise
    );
    // And the answer stands in for the table's figure.
    assert_eq!(
        critter(
            quiet,
            Some(4),
            None,
            weenie(&row(tolerance::RETALIATE, 5, None)),
            275
        ),
        Critter::WalkPast
    );
    assert_eq!(
        critter(
            quiet,
            None,
            Some(5),
            weenie(&row(tolerance::RETALIATE, 0, Some(4))),
            275
        ),
        Critter::WalkPast
    );
    // Or beats it: a Rabbit by name that is level 40 by appraisal
    // is no Rabbit, and one with sixty health is no Rabbit either.
    assert_eq!(
        critter(quiet, Some(40), None, weenie(&rabbit), 50),
        Critter::Fight
    );
    assert_eq!(
        critter(quiet, None, Some(60), weenie(&rabbit), 50),
        Critter::Fight
    );
    // A row found by the end of the name is a guess at the kind,
    // and its figures are some other weenie's: a "Dire Brown Rabbit" is
    // asked about, not walked past on the Rabbit's four and five,
    // and its own answer decides. Its temper is still believed,
    // both ways.
    let by_name = Some(Hint::Name(&rabbit));
    assert_eq!(critter(quiet, None, None, by_name, 50), Critter::Appraise);
    assert_eq!(
        critter(quiet, Some(4), None, by_name, 50),
        Critter::Appraise
    );
    assert_eq!(
        critter(quiet, Some(4), Some(5), by_name, 50),
        Critter::WalkPast
    );
    assert_eq!(
        critter(quiet, Some(40), Some(300), by_name, 50),
        Critter::Fight
    );
    assert_eq!(
        critter(
            quiet,
            None,
            None,
            Some(Hint::Name(&row(0, 3, Some(1)))),
            275
        ),
        Critter::Fight,
        "the kind starts fights"
    );
    assert_eq!(
        critter(
            quiet,
            Some(4),
            None,
            Some(Hint::Name(&row(tolerance::NO_ATTACK, 2001, Some(4)))),
            50
        ),
        Critter::WalkPast,
        "the kind never swings"
    );
}

#[test]
fn a_creature_the_table_does_not_know_is_judged_by_what_it_does() {
    use ac_world::elements::tolerance;
    let quiet = Seen::default();
    // A Cow: in no table, level 8 with twenty health by appraisal,
    // docile until attacked. Outgrown from 16 up, and a level 15
    // still fights it. Not in the table is not a reason to fight.
    for mine in [16, 20, 275] {
        assert_eq!(
            critter(quiet, Some(8), Some(20), None, mine),
            Critter::WalkPast
        );
    }
    assert_eq!(critter(quiet, Some(8), Some(20), None, 15), Critter::Fight);
    // A level 3 one, likewise.
    assert_eq!(
        critter(quiet, Some(3), Some(20), None, 20),
        Critter::WalkPast
    );
    // Nothing known about it yet: asked about, neither attacked nor
    // walked past. An appraisal brings the level and the health
    // together, so half an answer is no answer.
    assert_eq!(critter(quiet, None, None, None, 20), Critter::Appraise);
    assert_eq!(critter(quiet, Some(8), None, None, 20), Critter::Appraise);
    assert_eq!(critter(quiet, None, Some(20), None, 20), Critter::Appraise);
    // Too big to be a critter: an Auroch Yearling is level 8 with
    // sixty-five health, and worth the fight.
    assert_eq!(critter(quiet, Some(8), Some(65), None, 50), Critter::Fight);
    // The stranger's line is wider than the table's, and it is a
    // line.
    assert_eq!(
        critter(quiet, Some(8), Some(STRANGER_HEALTH), None, 50),
        Critter::WalkPast
    );
    assert_eq!(
        critter(quiet, Some(8), Some(STRANGER_HEALTH + 1), None, 50),
        Critter::Fight
    );
    // What it does outranks everything: attacking the character,
    // walking at it or a mate, or fighting anyone at all is a fight
    // already, whatever is or is not known about it.
    let rabbit = row(tolerance::RETALIATE, 5, Some(4));
    for seen in [
        Seen {
            attacked_us: true,
            ..Seen::default()
        },
        Seen {
            targets_us_or_mate: true,
            ..Seen::default()
        },
        Seen {
            fighting_anyone: true,
            ..Seen::default()
        },
    ] {
        assert!(!seen.quiet());
        assert_eq!(
            critter(seen, Some(8), Some(20), None, 20),
            Critter::Fight,
            "{seen:?}"
        );
        assert_eq!(
            critter(seen, None, None, None, 20),
            Critter::Fight,
            "{seen:?}"
        );
        assert_eq!(
            critter(seen, None, None, weenie(&rabbit), 20),
            Critter::Fight,
            "{seen:?}"
        );
    }
}

#[test]
fn the_holtburg_fields_are_still_a_hunting_ground_at_any_level() {
    use ac_world::elements::creature_by_id;
    let quiet = Seen::default();
    // Every creature the two encounter generators around Holtburg
    // put out (2007 newbietownaluviangen, 5150 harmlessaluviangen),
    // by weenie. ACE gives the first eight Retaliate so they do not
    // come at a new player, and they are all level 8: by level
    // alone a level 16 character had nothing left to attack
    // anywhere in Holtburg. None of them ever attacks first, so
    // what they have been seen doing says nothing, and the table
    // is what keeps them a hunting ground.
    let field = [
        19257, // Drudge Skulker
        19258, // Drudge Slinker
        19263, // Gnawer Shreth
        19261, // Creeper Mosswart
        19262, // Young Mosswart
        19256, // Young Banderling
        19260, // Mite Snippet
        19259, // Mite Scion
    ];
    for wcid in field {
        let c = creature_by_id(wcid).expect("in the table");
        assert!(c.passive(), "{} is what the test is about", c.name);
        for mine in [16, 20, 50, 275] {
            assert_eq!(
                critter(quiet, None, None, weenie(c), mine),
                Critter::Fight,
                "{} is what the fields are for, at level {mine}",
                c.name
            );
        }
    }
    // The two that really are critters are still walked past.
    for wcid in [2566, 24937] {
        let c = creature_by_id(wcid).expect("in the table");
        assert_eq!(
            critter(quiet, None, None, weenie(c), 20),
            Critter::WalkPast,
            "{} is worth nothing to a level 20 character",
            c.name
        );
    }
}

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

/// A creature of `wcid` standing in view, and a copy of it to ask
/// the fight rules about.
fn in_view(c: &mut Client, guid: u32, wcid: u32, name: &str) -> ac_world::WorldObject {
    let o = ac_world::WorldObject {
        weenie_class_id: wcid,
        ..crate::testkit::creature(guid, name)
    };
    c.world.objects.insert(guid, o.clone());
    o
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

/// The GameEvent behind "You evade <name>'s attack.": the server's
/// word that `name` swung at the character and missed.
fn evaded(guid: u32, name: &str) -> Vec<u8> {
    let mut w = ac_net::wire::Writer::new();
    w.u32(guid)
        .u32(0)
        .u32(ac_net::messages::event::EVASION_DEFENDER_NOTIFICATION)
        .string16(name);
    w.finish()
}

#[test]
fn an_evaded_swing_counts_as_being_attacked() {
    let mut c = character_of_level(no_data(), 20);
    assert!(!c.under_attack(), "nothing has happened yet");
    c.chat_message(
        ac_net::messages::opcode::GAME_EVENT,
        &evaded(0x8000_0001, "Drudge Skulker"),
    );
    assert!(c.under_attack(), "a miss is an attack all the same");
    assert_eq!(
        c.autoplay
            .hit_by
            .iter()
            .map(|(who, _)| who.as_str())
            .collect::<Vec<_>>(),
        ["Drudge Skulker"],
        "and the server named who swung"
    );
    assert!(c.hit_lately_by("Drudge Skulker"));
    assert!(!c.hit_lately_by("Drudge Robber"), "the one standing by");
}

#[test]
fn a_spell_cast_at_the_character_names_its_caster() {
    // The lines ACE sends the target of a spell, and nothing else is
    // sent: SpellProjectile's damage and drain, WorldObject_Magic's
    // drain, transfer and resist.
    for (line, who) in [
        (
            "Drudge Shaman blasts you for 12 points with Flame Bolt I.",
            "Drudge Shaman",
        ),
        (
            "Critical hit! Sneak Attack! Drudge Shaman scorches you for 40 points \
             with Flame Bolt II.",
            "Drudge Shaman",
        ),
        (
            "Overpower! Mite hits you for 3 points with Acid Stream I. Your \
             augmentation allows you to avoid a critical hit!",
            "Mite",
        ),
        (
            "Drudge Shaman casts Harm Other I and drains 9 points of your health.",
            "Drudge Shaman",
        ),
        (
            "You lose 20 points of mana due to Drudge Shaman casting Mana to \
             Health Other I on you",
            "Drudge Shaman",
        ),
        (
            "You resist the spell cast by Drudge Shaman",
            "Drudge Shaman",
        ),
    ] {
        assert_eq!(spell_attacker(line), Some(who), "{line}");
    }
    // The character's own spells, and a fellow's help, are no attack.
    for line in [
        "You blast Drudge Shaman for 12 points with Flame Bolt I.",
        "Drudge Shaman resists your spell",
        "With Harm Other I you drain 9 points of health from Drudge Shaman.",
        "Aldric casts Heal Other I and restores 30 points of your health.",
        "Aldric cast Strength Other I on you",
        "You gain 20 points of health due to Aldric casting Stamina to Health \
         Other I on you",
        "You lose 50 points of stamina due to casting Stamina to Mana Other I \
         on Aldric",
        "Drudge Skulker hits you for 5 points.",
    ] {
        assert_eq!(spell_attacker(line), None, "{line}");
    }
}

/// A line of system chat as the server sends it (ServerMessage 0xF7E0):
/// the text, and its ChatMessageType, Magic here.
fn magic_line(text: &str) -> Vec<u8> {
    let mut w = ac_net::wire::Writer::new();
    w.string16(text).u32(7);
    w.finish()
}

#[test]
fn a_caster_that_only_casts_is_attacking_the_character() {
    // A shaman turns and casts from its spell range and never closes
    // in to swing, so neither notification a swing brings ever came:
    // `under_attack` stayed false while it worked the character over,
    // and every "but fight back when it attacks you" carve-out walked
    // on past it.
    let mut c = character_of_level(no_data(), 20);
    let op = ac_net::messages::opcode::SERVER_MESSAGE;
    assert!(!c.under_attack(), "nothing has happened yet");
    c.chat_message(
        op,
        &magic_line("Drudge Shaman blasts you for 12 points with Flame Bolt I."),
    );
    assert!(c.under_attack(), "a bolt that landed is an attack");
    assert!(c.hit_lately_by("Drudge Shaman"));
    assert!(!c.hit_lately_by("Drudge Skulker"), "the one standing by");

    // Resisted, it was still cast at the character.
    c.autoplay.last_hit_us = None;
    c.autoplay.hit_by.clear();
    c.chat_message(
        op,
        &magic_line("You resist the spell cast by Drudge Shaman"),
    );
    assert!(c.hit_lately_by("Drudge Shaman"));

    // A fellow's heal is not.
    c.autoplay.last_hit_us = None;
    c.autoplay.hit_by.clear();
    c.chat_message(
        op,
        &magic_line("Aldric casts Heal Other I and restores 30 points of your health."),
    );
    assert!(!c.under_attack());
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

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_fight_on_the_road_ends_when_the_creature_stops_following() {
    // Off the road the fight is the fight, however far it has got.
    assert!(!road_fight_over(false, false, false, false, 20.0));
    // On the road, a creature that has stopped attacking and fallen
    // behind is let go.
    assert!(road_fight_over(true, false, false, false, 20.0));
    // Not while it is still attacking, walking at us, or being fought
    // by one of the party on the road.
    assert!(!road_fight_over(true, true, false, false, 20.0));
    assert!(!road_fight_over(true, false, true, false, 20.0));
    assert!(!road_fight_over(true, false, false, true, 20.0));
    // And one a swing away is finished, not left at half health.
    assert!(!road_fight_over(
        true,
        false,
        false,
        false,
        ROAD_REACH - 1.0
    ));

    // The same, read off the world: a Drudge that swung once on the
    // road and fell twenty metres behind.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_in_the_field(20, holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let guid = 0x8000_0001;
    let place = |c: &mut Client, metres: f32| {
        let mut o = in_view(c, guid, 0, "Drudge Skulker");
        o.position = Some(ac_world::object::Position::new_flat(
            holtburg,
            me + glam::Vec3::new(metres, 0.0, 0.0) - ac_world::landblock_origin(holtburg),
        ));
        c.world.objects.insert(guid, o);
    };
    place(&mut c, 20.0);
    assert!(!c.left_behind_on_the_road(guid, now), "not on a road");
    assert!(
        c.travel_to(glam::Vec2::new(me.x + 250.0, me.y)),
        "no way there"
    );
    assert!(c.on_its_way());
    assert!(c.left_behind_on_the_road(guid, now), "it fell behind");
    c.autoplay.attacked_by("Drudge Skulker", now);
    assert!(
        !c.left_behind_on_the_road(guid, now),
        "it is still swinging"
    );
    c.autoplay.hit_by.clear();
    c.autoplay.last_hit_us = None;
    c.world.objects.get_mut(&guid).unwrap().walked_at = Some(0x5000_0001);
    assert!(!c.left_behind_on_the_road(guid, now), "it is coming at us");
    place(&mut c, 3.0);
    assert!(
        !c.left_behind_on_the_road(guid, now),
        "a swing away: finished"
    );
}

/// An appraisal of `guid` saying it is `level`, with the health the
/// server sends whether or not the assessment succeeded.
fn appraised_as(c: &mut Client, guid: u32, level: Option<i32>, health: u32) {
    c.appraisals.insert(
        guid,
        ac_net::messages::Appraisal {
            guid,
            success: true,
            ints: level.map(|l| (CREATURE_LEVEL, l)).into_iter().collect(),
            creature: Some(ac_net::messages::CreatureProfile {
                health,
                health_max: health,
                ..Default::default()
            }),
            ..Default::default()
        },
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
#[ignore = "needs AC_DATA_DIR"]
fn a_target_left_half_a_world_away_is_not_fought_by_spell_either() {
    // Brynvor, 2026-09-15: teleported from the Holtburg Dungeon to
    // the town above it with a Swamp Rat as the casting target, and
    // stood there ten minutes casting at it from 34 km. The swing
    // already let such a target go; the spell now does too.
    let mut c = standing_in_the_field(999, 0xA9B4_002E, glam::Vec3::new(125.0, 132.0, 67.0));
    let rat = in_view(&mut c, 0x8000_20AD, 0, "Swamp Rat");
    let put = |c: &mut Client, cell: u32, local: glam::Vec3| {
        c.world.objects.get_mut(&rat.guid).unwrap().position = Some(ac_world::Position {
            cell,
            local,
            rotation: glam::Quat::IDENTITY,
        });
    };
    put(&mut c, 0x01F6_01FA, glam::Vec3::new(94.0, -63.0, -6.0));
    assert!(
        c.fight_target_gone(rat.guid, false),
        "not a walk away, so not here"
    );
    put(&mut c, 0xA9B4_002E, glam::Vec3::new(130.0, 132.0, 67.0));
    assert!(
        !c.fight_target_gone(rat.guid, false),
        "beside us, it is a fight"
    );
}

#[test]
fn a_second_attacker_does_not_silence_the_first() {
    // Two creatures on the character, swinging in turn: the last
    // name alone had the other walked past between its swings, and
    // the target moved off something still in melee with the
    // character.
    let mut c = character_of_level(no_data(), 20);
    let cfg = Fight::default();
    let now = Instant::now();
    let cow = in_view(&mut c, 0x8000_0031, STRANGER, "Cow");
    appraised_as(&mut c, cow.guid, Some(8), 20);
    assert!(c.a_critter(&cow, &cfg));
    c.autoplay.attacked_by("Cow", now);
    c.autoplay
        .attacked_by("Drudge Skulker", now + Duration::from_millis(500));
    assert!(!c.a_critter(&cow, &cfg), "still hitting the character");
    assert!(c.hit_lately_by("Drudge Skulker"));
    assert_eq!(c.autoplay.hit_by.len(), 2);
    // The same name again is the same attacker, at its latest.
    c.autoplay.attacked_by("Cow", now + Duration::from_secs(1));
    assert_eq!(c.autoplay.hit_by.len(), 2);
    // One that has not swung for a while is let go of.
    c.autoplay
        .attacked_by("Drudge Robber", now + UNDER_ATTACK + Duration::from_secs(2));
    assert_eq!(
        c.autoplay
            .hit_by
            .iter()
            .map(|(who, _)| who.as_str())
            .collect::<Vec<_>>(),
        ["Drudge Robber"]
    );
}

/// A weenie no table has heard of.
const STRANGER: u32 = 0x00FF_FFF0;

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

/// A weapon in the character's hand, or in its pack.
fn a_weapon(c: &mut Client, guid: u32, kind: u32, name: &str, in_hand: bool) {
    let me = c.world.player_guid;
    let locations = if kind == ac_world::item_type::CASTER {
        ac_world::equip::HELD
    } else {
        ac_world::equip::MELEE_WEAPON
    };
    c.world.objects.insert(
        guid,
        ac_world::WorldObject {
            guid,
            name: name.into(),
            item_type: kind,
            value: 100,
            valid_locations: locations,
            container: if in_hand { None } else { me },
            wielder: if in_hand { me } else { None },
            ..Default::default()
        },
    );
}

/// A character with a mace in hand, a wand in the pack, and a
/// creature it is swinging at.
fn mid_fight(assets: std::rc::Rc<ac_scene::Assets>) -> (Client, u32) {
    const MACE: u32 = 0x8000_0101;
    const WAND: u32 = 0x8000_0102;
    const CREATURE: u32 = 0x8000_0103;
    let mut c = character_of_level(assets, 20);
    a_weapon(
        &mut c,
        MACE,
        ac_world::item_type::MELEE_WEAPON,
        "Mace",
        true,
    );
    a_weapon(
        &mut c,
        WAND,
        ac_world::item_type::CASTER,
        "Training Wand",
        false,
    );
    in_view(&mut c, CREATURE, 19257, "Drudge Skulker");
    c.combat = true;
    c.attack_target = Some(CREATURE);
    c.last_attack = Instant::now() - Duration::from_secs(5);
    (c, WAND)
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

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_softening_with_no_wand_to_be_had_lets_the_fight_go_ahead() {
    let (mut c, wand) = mid_fight(game_data());
    const CREATURE: u32 = 0x8000_0103;
    // A Drudge Skulker is weakest to cold, fire and electricity
    // alike; whichever it picks, the character knows the
    // vulnerability for it.
    for element in [
        ac_world::elements::Element::Cold,
        ac_world::elements::Element::Fire,
        ac_world::elements::Element::Electric,
    ] {
        for id in ac_world::elements::vulnerabilities(element) {
            c.world.stats.spells.push(id);
        }
    }
    assert_eq!(c.combat_stance(), Stance::Melee);
    assert!(!c.mid_attack());
    // With a wand to be had, the softening takes the tick to reach
    // for one. This is the setup working, and what makes the second
    // half mean anything.
    assert!(
        c.autoplay_soften(CREATURE, "Drudge Skulker", Instant::now()),
        "reaching for the wand"
    );
    assert_eq!(c.autoplay.pending_wield, Some(wand));

    // Now the server is refusing that wand. Nothing can be sent, so
    // the softening gives the tick back rather than holding fire on
    // the target for ever: every expiry of the wait earned one more
    // refusal and doubled the next, up towards four hours.
    let (mut c, wand) = mid_fight(game_data());
    for element in [
        ac_world::elements::Element::Cold,
        ac_world::elements::Element::Fire,
        ac_world::elements::Element::Electric,
    ] {
        for id in ac_world::elements::vulnerabilities(element) {
            c.world.stats.spells.push(id);
        }
    }
    c.hold_off_wield(wand, Instant::now());
    assert!(
        !c.autoplay_soften(CREATURE, "Drudge Skulker", Instant::now()),
        "the fight may go ahead unsoftened"
    );
    assert_eq!(c.autoplay.pending_wield, None, "nothing was sent");
}

fn item(name: &str, value: u32, armor: u32) -> ItemStats {
    ItemStats {
        name: name.into(),
        value,
        armor_level: armor,
        appraised: true,
        kind: if armor > 0 { "armor" } else { "misc" },
        ..Default::default()
    }
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

/// A rule that claims what `line` matches, in the inventory's own
/// search language.
fn asks(name: &str, line: &str, action: LootAction) -> crate::profile::Rule {
    crate::profile::Rule {
        name: name.into(),
        action,
        all: vec![crate::profile::Ask::Search(line.into())],
        ..Default::default()
    }
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
#[ignore = "needs AC_DATA_DIR"]
fn supplies_are_handed_over_from_the_stack_that_is_leaving_anyway() {
    // A mate short of tapers: the stack the player said to sell
    // goes first, and the one they said to keep only when it is
    // the only one.
    let mut c = character_of_level(game_data(), 20);
    let me = c.world.player_guid.unwrap();
    let tapers = |guid: u32, stack: u32| ac_world::WorldObject {
        guid,
        name: "Prismatic Taper".into(),
        weenie_class_id: 20631,
        item_type: ac_world::item_type::SPELL_COMPONENTS,
        value: stack,
        stack_size: stack,
        max_stack_size: 1_000,
        container: Some(me),
        ..Default::default()
    };
    c.world
        .objects
        .insert(0x8000_0001, tapers(0x8000_0001, 1_000));
    let kept = c.stats_of(0x8000_0001).unwrap();
    c.autoplay.tag(&kept, LootAction::Keep);
    assert_eq!(
        c.spare_for("prismatic taper").map(|(g, _)| g),
        Some(0x8000_0001)
    );
    c.world.objects.insert(0x8000_0002, tapers(0x8000_0002, 40));
    assert_eq!(
        c.spare_for("prismatic taper").map(|(g, _)| g),
        Some(0x8000_0002),
        "nothing decided about it beats a keeper"
    );
    c.world.objects.insert(0x8000_0003, tapers(0x8000_0003, 12));
    let to_sell = c.stats_of(0x8000_0003).unwrap();
    c.autoplay.tag(&to_sell, LootAction::Sell);
    assert_eq!(
        c.spare_for("prismatic taper").map(|(g, _)| g),
        Some(0x8000_0003),
        "leaving anyway"
    );
    assert_eq!(c.spare_for("lead scarab"), None);
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

#[test]
fn the_best_salvager_has_an_ust_and_the_highest_skill() {
    let mate = |name: &str, guid: u32, salvaging: u32, has_ust: bool| Mate {
        name: name.into(),
        guid,
        salvaging,
        has_ust,
        ..Default::default()
    };
    let team = [
        mate("Zed", 1, 300, true),
        mate("Amy", 2, 300, true),
        mate("Bob", 3, 400, false),
        mate("Cal", 4, 200, true),
    ];
    // Bob's skill is highest but he has no Ust; Amy and Zed tie and
    // the name that sorts first wins.
    assert_eq!(best_salvager(team.iter()), Some(("Amy".into(), 2)));
    assert_eq!(best_salvager(team[3..].iter()), Some(("Cal".into(), 4)));
    assert_eq!(best_salvager(team[2..3].iter()), None);
    assert_eq!(best_salvager(std::iter::empty()), None);
    // Someone not yet in the world (guid 0) cannot be handed anything.
    assert_eq!(best_salvager([mate("Nobody", 0, 999, true)].iter()), None);
}

/// `items` (guid and workmanship), none of them refused yet.
fn never_refused(items: &[(u32, f32)]) -> Vec<(u32, f32, u8)> {
    items.iter().map(|(g, w)| (*g, *w, 0)).collect()
}

/// Every salvage sent for `items` (guid and workmanship), the server
/// taking each batch before the next is chosen.
fn salvages(items: &[(u32, f32)]) -> Vec<Vec<u32>> {
    let mut left = items.to_vec();
    let mut sent = Vec::new();
    while let Some((_, batch)) = next_salvage_batch(never_refused(&left)) {
        left.retain(|(g, _)| !batch.contains(g));
        sent.push(batch);
    }
    sent
}

#[test]
fn a_salvage_that_came_to_nothing_waits_behind_the_grades_not_yet_tried() {
    // ACE skips a Retained item without a word. Chosen as the best
    // grade every time, a 10 like that went out alone after each
    // timeout, and everything below it waited behind all three.
    let (ten, nine, six, five) = (1, 2, 3, 4);
    assert_eq!(
        next_salvage_batch([(ten, 10.0, 1), (nine, 9.0, 0), (six, 6.0, 0)]),
        Some((SalvageGrade::Nine, vec![nine]))
    );
    assert_eq!(
        next_salvage_batch([(ten, 10.0, 1), (six, 6.0, 0)]),
        Some((SalvageGrade::Common, vec![six]))
    );
    // Once the rest are gone it is asked for again, still on its own.
    assert_eq!(
        next_salvage_batch([(ten, 10.0, 1)]),
        Some((SalvageGrade::Ten, vec![ten]))
    );
    // Refused alike, the grades still keep apart.
    assert_eq!(
        next_salvage_batch([(six, 6.0, 1), (ten, 10.0, 1), (five, 5.0, 1)]),
        Some((SalvageGrade::Ten, vec![ten]))
    );
    // And the one refused least goes first.
    assert_eq!(
        next_salvage_batch([(six, 6.0, 2), (five, 5.0, 1)]),
        Some((SalvageGrade::Common, vec![five]))
    );
}

#[test]
fn a_workmanship_10_iron_mace_is_not_salvaged_with_a_6() {
    // In one salvage both go into the same bag of Iron, and the bag
    // comes out a workmanship 8.
    let (six, ten) = (0x8000_0001, 0x8000_0002);
    assert_eq!(
        salvages(&[(six, 6.0), (ten, 10.0)]),
        vec![vec![ten], vec![six]]
    );
}

#[test]
fn nines_and_tens_never_share_a_salvage() {
    let items = [(1, 9.0), (2, 10.0), (3, 6.0), (4, 9.0), (5, 10.0), (6, 3.0)];
    assert_eq!(
        next_salvage_batch(never_refused(&items)),
        Some((SalvageGrade::Ten, vec![2, 5]))
    );
    // The best first, each grade alone, in the order they were given.
    assert_eq!(salvages(&items), vec![vec![2, 5], vec![1, 4], vec![3, 6]]);
}

#[test]
fn everything_below_nine_goes_in_one_salvage() {
    let items = [(1, 1.0), (2, 8.0), (3, 5.0), (4, 8.0)];
    assert_eq!(
        next_salvage_batch(never_refused(&items)),
        Some((SalvageGrade::Common, vec![1, 2, 3, 4]))
    );
    assert_eq!(salvages(&items), vec![vec![1, 2, 3, 4]]);
}

#[test]
fn a_grade_with_nothing_in_it_sends_no_salvage() {
    assert_eq!(next_salvage_batch([]), None);
    assert_eq!(salvages(&[]), Vec::<Vec<u32>>::new());
    // No 9s: the 10 and the rest, and no empty salvage between.
    assert_eq!(salvages(&[(1, 6.0), (2, 10.0)]), vec![vec![2], vec![1]]);
    // Only 9s: one salvage.
    assert_eq!(salvages(&[(1, 9.0), (2, 9.0)]), vec![vec![1, 2]]);
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
#[ignore = "needs AC_DATA_DIR"]
fn tidying_the_pack_never_pours_one_salvage_bag_into_another() {
    // Two bags of Iron share a wcid, but only an Ust puts bags
    // together. The server sends a bag with no stack size, which
    // leaves it at 1, and both pours -- the tidy chore and the one
    // at a vendor's counter -- read that to decide what stacks.
    let mut c = character_of_level(game_data(), 20);
    let me = c.world.player_guid;
    let bag = |guid: u32, workmanship: f32| ac_world::WorldObject {
        guid,
        weenie_class_id: 20986,
        name: "Salvaged Iron".into(),
        item_type: ac_world::item_type::TINKERING_MATERIAL,
        material: 0x3D,
        workmanship,
        structure: 50,
        max_structure: 100,
        stack_size: 1,
        max_stack_size: 1,
        container: me,
        ..Default::default()
    };
    let (tens, sixes) = (0x8000_0201, 0x8000_0202);
    c.world.objects.insert(tens, bag(tens, 10.0));
    c.world.objects.insert(sixes, bag(sixes, 6.0));
    // A pair that does pour, so the bags are not passed over only
    // because nothing is looked at.
    let arrows = |guid: u32, count: u32| ac_world::WorldObject {
        guid,
        weenie_class_id: 300,
        name: "Arrow".into(),
        stack_size: count,
        max_stack_size: 250,
        container: me,
        ..Default::default()
    };
    let (few, many) = (0x8000_0203, 0x8000_0204);
    c.world.objects.insert(few, arrows(few, 5));
    c.world.objects.insert(many, arrows(many, 40));
    let m = crate::pack::next_merge(&c.pack_stacks()).expect("the arrows pour");
    assert_eq!((m.from, m.to), (few, many));
    c.world.objects.remove(&few);
    assert_eq!(crate::pack::next_merge(&c.pack_stacks()), None);
    let counter = c.vendor_snapshot(&crate::growth::Growth::default());
    let bags: Vec<_> = counter
        .items
        .iter()
        .filter(|i| i.guid == tens || i.guid == sixes)
        .collect();
    assert_eq!(bags.len(), 2);
    assert!(bags.iter().all(|i| i.max_stack <= 1), "{bags:?}");
}

#[test]
fn a_body_where_a_kill_fell_is_ours_however_far() {
    let now = Instant::now();
    let spots = vec![(glam::Vec3::new(100.0, 100.0, 50.0), now)];
    // Where it fell, give or take the drift of dying.
    assert!(near_a_kill(glam::Vec3::new(103.0, 98.0, 51.0), &spots));
    // Somebody else's, a street away.
    assert!(!near_a_kill(glam::Vec3::new(120.0, 100.0, 50.0), &spots));
    assert!(!near_a_kill(glam::Vec3::new(100.0, 100.0, 50.0), &[]));
}

#[test]
fn a_resist_or_an_evasion_got_there() {
    assert_eq!(
        arrived_unharmed("Drudge Skulker resists your spell"),
        Some("Drudge Skulker")
    );
    assert_eq!(
        arrived_unharmed("Mite Scion evades your attack."),
        Some("Mite Scion")
    );
    // Ours, not theirs.
    assert_eq!(
        arrived_unharmed("You resist the spell cast by Drudge Skulker"),
        None
    );
    let now = Instant::now();
    let long_ago = now.checked_sub(CLOSE_IN_AFTER * 2).unwrap();
    // Thrown a while ago, and nothing has got there since.
    assert!(nothing_arrived(Some(long_ago), now));
    // Only just thrown: still on its way.
    assert!(!nothing_arrived(Some(now), now));
    // Nothing thrown yet -- still walking to a clear shot -- is no miss.
    assert!(!nothing_arrived(None, now));
}

#[test]
fn walking_up_to_a_target_is_working_on_it_while_it_gets_nearer() {
    // The first stretch of the walk, and a new target, both count.
    assert!(came_nearer(None, 7, 40.0));
    assert!(came_nearer(Some((8, 5.0)), 7, 40.0));
    // Nearer by a pace and more: still closing.
    assert!(came_nearer(Some((7, 40.0)), 7, 38.5));
    // Shuffling on the spot, or backing off round a wall, is not.
    assert!(!came_nearer(Some((7, 40.0)), 7, 39.5));
    assert!(!came_nearer(Some((7, 40.0)), 7, 45.0));
}

#[test]
fn the_fight_waits_only_on_bodies_the_looting_takes() {
    let now = Instant::now();
    let me = glam::Vec3::ZERO;
    let off = glam::Vec3::new(22.0, 0.0, 0.0);
    // Close by: looted, and waited on.
    assert!(corpse_is_ours(
        me,
        glam::Vec3::new(5.0, 0.0, 0.0),
        40.0,
        &[]
    ));
    // Twenty-two metres off where nothing of ours fell: neither. It
    // used to be waited on out to twenty-five and looted only to
    // twenty, and the character stood between the two for good.
    assert!(!corpse_is_ours(me, off, 40.0, &[]));
    // Where a kill of ours fell: both.
    assert!(corpse_is_ours(me, off, 40.0, &[(off, now)]));
    // But not past the fight radius.
    assert!(!corpse_is_ours(me, off, 20.0, &[(off, now)]));

    // Thirty metres off, where the summoned creature landed the last
    // blow: no kill notice came, so no kill spot, and it is not ours
    // as it lies. It is asked about instead.
    let pets = glam::Vec3::new(30.0, 0.0, 0.0);
    assert!(!corpse_is_ours(me, pets, 60.0, &[]));
    assert!(whose_to_ask(me, pets, 60.0, &[]));
    // Its description names the creature, so it is claimed where it
    // lies: looted, and waited on, and not asked about again.
    assert!(killed_by_us(
        "Killed by Blargerton's Mud Golem.",
        "Blargerton"
    ));
    let claimed = [(pets, now)];
    assert!(corpse_is_ours(me, pets, 60.0, &claimed));
    assert!(!whose_to_ask(me, pets, 60.0, &claimed));
    // One close by needs no asking, and one past the fight radius is
    // not asked about.
    assert!(!whose_to_ask(me, glam::Vec3::new(5.0, 0.0, 0.0), 60.0, &[]));
    assert!(!whose_to_ask(
        me,
        glam::Vec3::new(70.0, 0.0, 0.0),
        60.0,
        &[]
    ));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_moment_held_open_for_a_falling_body_ends_when_one_lands() {
    // The next fight is held for three seconds after a killing blow,
    // because the body comes a moment after the creature dies. Held
    // for the whole three regardless of what landed, nine characters
    // killing in one huddle each spent it standing over a body one
    // of the others was already opening: fighting was 13% of that
    // run and opening a corpse 53%.
    let mut c = character_of_level(game_data(), 20);
    let s = Duration::from_secs;
    let t0 = Instant::now();
    let spot = glam::Vec3::new(40.0, 40.0, 10.0);
    let body = |guid: u32, at: glam::Vec3| ac_world::WorldObject {
        guid,
        name: "Corpse of Drudge Slave".into(),
        object_desc_flags: ac_world::object_desc_flags::CORPSE,
        position: Some(ac_world::object::Position::new_flat(0, at)),
        ..Default::default()
    };
    assert!(!c.owes_a_corpse(), "owed a body with no kill behind it");

    // Its own killing blow: the body is owed while it is still
    // falling, and no longer.
    c.autoplay.last_kill = Some(t0);
    c.autoplay.kill_spots = vec![(spot, t0)];
    assert!(c.own_body_still_falling(t0));
    assert!(c.own_body_still_falling(t0 + CORPSE_APPEARS - s(1)));
    assert!(!c.own_body_still_falling(t0 + CORPSE_APPEARS));

    // A body noted before the blow is some other kill's and says
    // nothing about this one, wherever it lies.
    let (early, elsewhere, ours) = (0x8000_0001, 0x8000_0002, 0x8000_0003);
    c.world.objects.insert(early, body(early, spot));
    c.autoplay.corpse_seen = vec![(early, t0 - s(1))];
    assert!(c.own_body_still_falling(t0));

    // Nor does a body that landed since the blow somewhere this
    // character killed nothing. Ending the hold on any corpse at
    // all, a huddle of nine had one come into view every couple of
    // seconds, and each one sent a character off after the next
    // fight leaving the body it had just made behind.
    let away = spot + glam::Vec3::new(KILL_SPOT * 2.0, 0.0, 0.0);
    c.world.objects.insert(elsewhere, body(elsewhere, away));
    c.autoplay.corpse_seen.push((elsewhere, t0));
    assert!(
        c.own_body_still_falling(t0),
        "let go of its own body for somebody else's"
    );

    // Its own, landing where its kill fell, ends the wait: from here
    // the bodies on the floor are the answer, a mate's claim among
    // them.
    c.world.objects.insert(ours, body(ours, spot));
    c.autoplay.corpse_seen.push((ours, t0));
    assert!(!c.own_body_still_falling(t0));
    assert!(!c.owes_a_corpse());
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
fn a_far_corpse_is_asked_about_once_and_the_fight_waits_on_the_answer_briefly() {
    let t0 = Instant::now();
    let s = Duration::from_secs;
    let mut whose = Whose::default();
    // No creature seen out: nothing left bodies about.
    assert!(!whose.pet_lately(t0));
    whose.pet_out(t0);
    assert!(whose.pet_lately(t0 + s(30)));
    assert!(!whose.pet_lately(t0 + PET_KILLS_FOR));

    // Asked about: the fight waits a moment for the answer.
    assert!(!whose.waiting(t0));
    whose.ask(0x8000_0001, t0);
    assert!(whose.has_asked(0x8000_0001));
    assert!(whose.waiting(t0 + s(1)));
    // Asking again changes nothing, and does not restart the wait.
    whose.ask(0x8000_0001, t0 + s(2));
    assert_eq!(whose.out().collect::<Vec<_>>(), vec![0x8000_0001]);
    // An answer that never comes does not hold the fight for good.
    assert!(!whose.waiting(t0 + WHOSE_WAIT));

    // Once the answer is in, nothing is waited on or out, and the
    // corpse is still not asked about again.
    whose.ask(0x8000_0002, t0 + s(4));
    assert!(whose.waiting(t0 + s(5)));
    whose.answered(0x8000_0002);
    whose.answered(0x8000_0001);
    assert!(!whose.waiting(t0 + s(5)));
    assert_eq!(whose.out().count(), 0);
    assert!(whose.has_asked(0x8000_0002));

    // A corpse gone from view is forgotten.
    whose.tidy(|g| g == 0x8000_0002);
    assert!(!whose.has_asked(0x8000_0001));
    assert!(whose.has_asked(0x8000_0002));
}

#[test]
fn a_corpse_a_floor_below_is_not_close_however_near_it_lies_on_the_map() {
    // The Holtburg Dungeon's rooms are stacked: a body three metres
    // off on the map and eight metres down is in another room.
    let now = Instant::now();
    let me = glam::Vec3::new(0.0, 0.0, 8.0);
    let below = glam::Vec3::new(3.0, 0.0, 0.0);
    assert!(!corpse_is_ours(me, below, 60.0, &[]));
    // Nor one a floor above.
    assert!(!corpse_is_ours(below, me, 60.0, &[]));
    // On this floor it is, and a step or a doorsill up is still this
    // floor.
    assert!(corpse_is_ours(
        me,
        glam::Vec3::new(3.0, 0.0, 8.0),
        60.0,
        &[]
    ));
    assert!(corpse_is_ours(
        me,
        glam::Vec3::new(3.0, 0.0, 9.5),
        60.0,
        &[]
    ));
    // Where a kill of its own fell it is still its own: whether it
    // can be walked to is for the walk to find out.
    assert!(corpse_is_ours(me, below, 60.0, &[(below, now)]));
}

/// One of the others, standing at `at`, with `looting` in hand for
/// `held`.
fn looter(guid: u32, at: glam::Vec3, looting: Option<u32>, held: Duration) -> Mate {
    Mate {
        name: format!("Bryn{guid:02}"),
        guid,
        world: at,
        health: 1.0,
        autoplay: true,
        looting,
        looting_for: held,
        opens_bodies: true,
        ..Default::default()
    }
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

/// A character as the turns read it: standing at `at`, free, with
/// room, having opened `opened` bodies first lately.
fn turn_at(guid: u32, at: glam::Vec3, opened: u16) -> Turn {
    Turn {
        guid,
        world: at,
        looting: false,
        fighting: false,
        room: true,
        opened,
    }
}

#[test]
fn every_session_deals_a_newly_fallen_body_to_the_same_character() {
    // A claim is said every half second and a body is chosen within
    // a tick of falling, so for that first moment there is no claim
    // to read and the nine have to agree without one. Every session
    // works the same answer out of the same roster, and the rest
    // stand off.
    let t0 = Instant::now();
    let body = 0x8000_0001;
    let at = glam::Vec3::ZERO;
    let fleet: Vec<u32> = (0..9).map(|i| 0x5000_0010 + i).collect();
    let others = |me: u32| -> Vec<Mate> {
        fleet
            .iter()
            .filter(|g| **g != me)
            .map(|g| looter(*g, at, None, Duration::ZERO))
            .collect()
    };
    // Not the lowest guid, which used to open every body it stood
    // over: the deal for this body.
    let first = 0x5000_0012;
    for &me in &fleet {
        let ap = Autoplay {
            corpse_seen: vec![(body, t0)],
            team: view_of(others(me)),
            ..Default::default()
        };
        assert_eq!(
            ap.team.opens_first(body, at, turn_at(me, at, 0)),
            first,
            "disagreed about whose turn"
        );
        assert_eq!(
            ap.ours_to_open(body, at, me, at, t0),
            me == first,
            "{me:#010x} did not stand off"
        );
        // Once the board has had time to catch up the claims are
        // the truth, and whoever has not been told otherwise goes
        // ahead.
        assert!(ap.ours_to_open(body, at, me, at, t0 + CLAIM_SETTLE));
    }

    // Only the ones free to open it count. A mate played by hand, a
    // dead one, one not in the world, one already at another body,
    // one fighting, one with a full pack, one laden and one across
    // the field all leave it to us, whatever the deal.
    let mine = fleet[8];
    let field_away = glam::Vec3::new(LOOT_NEAR + 5.0, 0.0, 0.0);
    let packed = |pack_full, laden| crate::logistics::Supplies {
        pack_full,
        laden,
        ..Default::default()
    };
    let view = view_of(vec![
        Mate {
            autoplay: false,
            ..looter(fleet[0], at, None, Duration::ZERO)
        },
        Mate {
            health: 0.0,
            ..looter(fleet[1], at, None, Duration::ZERO)
        },
        looter(0, at, None, Duration::ZERO),
        looter(fleet[2], at, Some(0x8000_0002), Duration::ZERO),
        Mate {
            target: Some(0x7000_0001),
            ..looter(fleet[3], at, None, Duration::ZERO)
        },
        Mate {
            supplies: packed(true, false),
            ..looter(fleet[4], at, None, Duration::ZERO)
        },
        Mate {
            supplies: packed(false, true),
            ..looter(fleet[5], at, None, Duration::ZERO)
        },
        looter(fleet[6], field_away, None, Duration::ZERO),
    ]);
    assert_eq!(view.opens_first(body, at, turn_at(mine, at, 0)), mine);

    // And this character is judged by the same tests as the rest. A
    // body is owed to a caster out to the fight radius where one of
    // its kills fell, which is further than a mate has to stand to
    // count: with no reach test on ourselves, the caster called the
    // body its own while every mate's roster ruled the caster out,
    // and the two of them opened it in the same second.
    let over_it = view_of(vec![looter(fleet[0], at, None, Duration::ZERO)]);
    assert_eq!(
        over_it.opens_first(body, at, turn_at(mine, field_away, 0)),
        fleet[0],
        "claimed a body from across the field"
    );
    // Nor is this one dealt a turn while it is at another body, or
    // with no room for what is on this one.
    for busy in [
        Turn {
            looting: true,
            ..turn_at(mine, at, 0)
        },
        Turn {
            room: false,
            ..turn_at(mine, at, 0)
        },
    ] {
        assert_eq!(over_it.opens_first(body, at, busy), fleet[0]);
    }
    // Nobody within reach at all: it is ours to walk to.
    assert_eq!(
        view_of(vec![looter(fleet[0], field_away, None, Duration::ZERO)]).opens_first(
            body,
            at,
            turn_at(mine, field_away, 0)
        ),
        mine,
        "left a body nobody was near"
    );
}

#[test]
fn the_turns_go_round_body_by_body() {
    // Three characters over one spot, all free. Whoever opens a body
    // first says so on its row, and the next body goes to one that
    // has had fewer turns.
    let at = glam::Vec3::ZERO;
    let party = [0x5000_0021u32, 0x5000_0022, 0x5000_0023];
    let mut opened = [0u16; 3];
    let mut who_opened = Vec::new();
    for n in 0..6u32 {
        let body = 0x8000_0100 + n;
        // Each session works it out for itself, from what the others
        // said about their turns.
        let dealt: Vec<u32> = (0..3)
            .map(|i| {
                let mates = (0..3)
                    .filter(|j| *j != i)
                    .map(|j| Mate {
                        opened_first: opened[j],
                        ..looter(party[j], at, None, Duration::ZERO)
                    })
                    .collect();
                view_of(mates).opens_first(body, at, turn_at(party[i], at, opened[i]))
            })
            .collect();
        assert!(dealt.iter().all(|g| *g == dealt[0]), "{n}: {dealt:x?}");
        let i = party.iter().position(|g| *g == dealt[0]).unwrap();
        opened[i] += 1;
        who_opened.push(i);
    }
    // Each of the three once in every round of three bodies.
    for round in who_opened.chunks(3) {
        let mut round = round.to_vec();
        round.sort_unstable();
        assert_eq!(round, vec![0, 1, 2], "{who_opened:?}");
    }
}

#[test]
fn bodies_falling_together_go_to_different_characters() {
    // Nine bodies in one tick, before anyone's turn is on the board:
    // the lowest guid used to be dealt every one of them.
    let at = glam::Vec3::ZERO;
    let fleet: Vec<u32> = (0..9).map(|i| 0x5000_0010 + i).collect();
    let view = view_of(
        fleet[1..]
            .iter()
            .map(|g| looter(*g, at, None, Duration::ZERO))
            .collect(),
    );
    let dealt: Vec<u32> = (1..=9)
        .map(|n| view.opens_first(0x8000_0000 + n, at, turn_at(fleet[0], at, 0)))
        .collect();
    let mut to = dealt.clone();
    to.sort_unstable();
    to.dedup();
    assert_eq!(to.len(), 7, "{dealt:x?}");
    for g in &to {
        assert!(dealt.iter().filter(|d| *d == g).count() <= 2);
    }
}

#[test]
fn the_dealing_mix_gives_the_same_numbers_everywhere() {
    // Worked out by hand from splitmix64's finish: no build, process
    // or platform may deal a body differently.
    assert_eq!(deal(0, 0), 0xe220_a839_7b1d_cdaf);
    assert_eq!(deal(0x8000_0001, 0x5000_0010), 0xfe12_cdc0_9d21_d7ba);
    assert_eq!(deal(0x8000_0001, 0x5000_0011), 0xd972_5b91_3475_6d9c);
    assert_eq!(deal(0x8000_198f, 0x5000_0001), 0x2ac7_ac48_90d6_3ca3);
}

#[test]
fn a_busy_fellows_turn_passes_on_and_the_body_is_still_opened() {
    // Best effort: a character fighting when a body falls loses its
    // turn to one that is free, and a turn nobody takes never leaves
    // the body lying.
    let t0 = Instant::now();
    let (body, at, foe) = (0x8000_0001, glam::Vec3::ZERO, 0x7000_0001);
    let party = [0x5000_0031u32, 0x5000_0032, 0x5000_0033];
    let session = |me: u32, fighting: &[u32]| {
        let row = |g: u32| Mate {
            target: fighting.contains(&g).then_some(foe),
            ..looter(g, at, None, Duration::ZERO)
        };
        let mut ap = Autoplay {
            corpse_seen: vec![(body, t0)],
            team: view_of(
                party
                    .iter()
                    .filter(|g| **g != me)
                    .map(|g| row(*g))
                    .collect(),
            ),
            ..Default::default()
        };
        // Each reads itself off what it said about itself.
        ap.team.me = Some(row(me));
        ap
    };
    let opening = |fighting: &[u32], now: Instant| -> Vec<u32> {
        party
            .iter()
            .copied()
            .filter(|me| session(*me, fighting).ours_to_open(body, at, *me, at, now))
            .collect()
    };
    // All free: one of them has the turn.
    let dealt = opening(&[], t0);
    assert_eq!(dealt.len(), 1, "{dealt:x?}");
    // That one fighting: another, free, has it instead.
    let passed = opening(&dealt, t0);
    assert_eq!(passed.len(), 1, "{passed:x?}");
    assert_ne!(passed, dealt, "the turn waited on one fighting");
    // After the first second the body is anyone's who is free, the
    // one that was fighting too once it is done.
    assert_eq!(opening(&dealt, t0 + CLAIM_SETTLE), party.to_vec());
    // Everyone fighting: nobody stands off for anybody.
    assert_eq!(opening(&party, t0), party.to_vec());
}

/// One profile for a whole party: a broken key for whoever can mend
/// it, and healing kits up to four.
fn a_party_profile() -> crate::profile::Profile {
    use crate::profile::{Ask, Mine, Profile, Rule};
    Profile {
        name: "party".into(),
        rules: vec![
            Rule {
                name: "broken keys, if I can mend them".into(),
                action: LootAction::Keep,
                all: vec![
                    Ask::Search("broken".into()),
                    Ask::Me(Mine::Skill {
                        skill: ac_world::stats::skill::LOCKPICK,
                        op: crate::items::Op::Ge,
                        level: 250,
                    }),
                ],
                ..Default::default()
            },
            Rule {
                name: "healing kits, a few".into(),
                action: LootAction::Keep,
                all: vec![Ask::Search("healing kit".into())],
                keep_up_to: Some(4),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

/// A mate whose board row says it has `lockpick` Lockpick.
fn picking(guid: u32, lockpick: u32) -> Mate {
    use ac_world::stats::{sac, skill};
    Mate {
        level: 30,
        skills: vec![(skill::LOCKPICK, lockpick, lockpick, sac::TRAINED)],
        ..looter(guid, glam::Vec3::ZERO, None, Duration::ZERO)
    }
}

#[test]
fn a_shut_body_says_who_shut_it_what_it_took_and_whom_it_is_left_for() {
    // A nine-character run's log named nobody, so nobody could say who
    // opened a body first or who came back to it for nothing.
    let names = |of: &[&str]| of.iter().map(|n| n.to_string()).collect::<Vec<_>>();
    let body = 0x8000_198f;
    assert_eq!(
        shut_line(
            "Bryn01",
            "Corpse of Biaka",
            body,
            3,
            &names(&["Bryn02", "Bryn03"]),
            &names(&["Bryn04"]),
            &names(&["Bryn05"]),
        ),
        "autoplay: Bryn01 shut Corpse of Biaka (0x8000198f), took 3; \
         done for Bryn02, Bryn03; left for Bryn04; Bryn05 standing by"
    );
    // Alone, it says only what it did.
    assert_eq!(
        shut_line("Bryn01", "Corpse of Biaka", body, 0, &[], &[], &[]),
        "autoplay: Bryn01 shut Corpse of Biaka (0x8000198f), took 0"
    );
    assert_eq!(
        shut_line(
            "Bryn01",
            "Corpse of Biaka",
            body,
            1,
            &[],
            &names(&["Bryn04"]),
            &[]
        ),
        "autoplay: Bryn01 shut Corpse of Biaka (0x8000198f), took 1; left for Bryn04"
    );
    // The one that stays away says whose word it took.
    assert_eq!(
        taken_in_line("Bryn02", "Corpse of Biaka", body, "Bryn01"),
        "autoplay: Bryn02: Corpse of Biaka (0x8000198f) done for me by Bryn01"
    );
    assert_eq!(
        standing_by_line(
            "Bryn05",
            "Corpse of Biaka",
            body,
            "Bryn01",
            &names(&["Bryn04"])
        ),
        "autoplay: Bryn05: Corpse of Biaka (0x8000198f) left for Bryn04 by Bryn01; standing by"
    );
}

/// A fellow as a shut judges it, off its row.
fn as_fellow(m: &Mate) -> Fellow {
    (m.guid, m.name.clone(), m.wielder())
}

/// A thing lying on a body, this character carrying `held` of it.
fn lying(stats: ItemStats, held: u32) -> Option<Left<'static>> {
    Some(Left {
        stats,
        id: None,
        held,
    })
}

#[test]
fn a_mate_is_judged_by_the_skills_it_said_about_itself() {
    // One profile for the party, read differently for each of it: the
    // broken key is the lockpicker's to take and not the mage's. Judged
    // by another character, each is read with what its row said.
    use crate::profile::Verdict;
    let profile = a_party_profile();
    let heard = |said: Mate| -> Mate {
        serde_json::from_value(serde_json::to_value(&said).unwrap()).unwrap()
    };
    let (picker, mage) = (heard(picking(2, 300)), heard(picking(3, 5)));
    let key = item("Broken Marble Key", 0, 0);
    let judge = |m: &Mate| judge_loot(&key, None, Some(&profile), &m.wielder(), &m.name, 0);
    assert_eq!(
        judge(&picker),
        Verdict::Decided(LootAction::Keep, "broken keys, if I can mend them".into())
    );
    assert_eq!(judge(&mage), Verdict::None);
    // Shut by one that cannot mend it: left for the lockpicker, done for
    // the mage, and nothing the opener would take left on it.
    let opener = crate::weapons::Wielder::default();
    let me = Taker {
        guid: 1,
        name: "Bryn01",
        sheet: &opener,
        held: 0,
    };
    let fellows = [as_fellow(&picker), as_fellow(&mage)];
    assert_eq!(
        judge_shut(&[lying(key, 0)], &profile, me, &fellows, &fellows, None),
        ShutFor {
            left_for: vec![picker.guid],
            done_for: vec![mage.guid],
            ..Default::default()
        }
    );
}

#[test]
fn a_fellow_that_would_take_what_was_left_for_another_stands_by_and_is_never_told_it_is_done() {
    // "Healing kits, while my Healing is 100 or more, up to four", read
    // by four of a party. The kit goes to the best healer, who turns out
    // to carry four already. Told the body was done, the others wrote it
    // off, and the kit lay there though two of them wanted it.
    use ac_world::stats::{sac, skill};
    let mut profile = a_party_profile();
    profile.rules[1]
        .all
        .push(crate::profile::Ask::Me(crate::profile::Mine::Skill {
            skill: skill::HEALING,
            op: crate::items::Op::Ge,
            level: 100,
        }));
    let healer = |guid: u32, healing: u32| Mate {
        skills: vec![(skill::HEALING, healing, healing, sac::TRAINED)],
        ..picking(guid, 0)
    };
    let (a, b, c, d) = (healer(1, 200), healer(2, 300), healer(3, 150), healer(4, 0));
    let kit = item("Healing Kit", 50, 0);

    // A opens it carrying two.
    let a_sheet = a.wielder();
    let a_opens = Taker {
        guid: a.guid,
        name: &a.name,
        sheet: &a_sheet,
        held: 2,
    };
    let others = [as_fellow(&b), as_fellow(&c), as_fellow(&d)];
    assert_eq!(
        judge_shut(
            &[lying(kit.clone(), 2)],
            &profile,
            a_opens,
            &others,
            &others,
            None
        ),
        ShutFor {
            left_for: vec![b.guid],
            // C would take the kit too: it stands by for B.
            stand_by: vec![c.guid],
            // D cannot heal, and nothing on the body is for it.
            done_for: vec![d.guid],
            // A would take it too, and stands by rather than writing the
            // body off.
            waits: true,
            ..Default::default()
        }
    );

    // B carries four and leaves it. Nothing goes back to A, which shut
    // the body: the kit is C's now, and A stands by for C.
    let b_sheet = b.wielder();
    let b_opens = Taker {
        guid: b.guid,
        name: &b.name,
        sheet: &b_sheet,
        held: 4,
    };
    let judged = [as_fellow(&a), as_fellow(&c), as_fellow(&d)];
    let sendable = [as_fellow(&c), as_fellow(&d)];
    assert_eq!(
        judge_shut(
            &[lying(kit, 4)],
            &profile,
            b_opens,
            &judged,
            &sendable,
            None
        ),
        ShutFor {
            left_for: vec![c.guid],
            stand_by: vec![a.guid],
            done_for: vec![d.guid],
            ..Default::default()
        }
    );
}

#[test]
fn a_body_stood_by_for_a_fellow_that_cannot_come_is_opened_again() {
    // Salvage was left on every body for the one salvager, and the rest
    // wrote each body off: when the salvager died, followed its leader
    // out of reach or filled its pack first, the salvage lay there until
    // the body rotted.
    use crate::did::Did;
    let t0 = Instant::now();
    let s = Duration::from_secs;
    let (body, at) = (0x8000_0001, glam::Vec3::ZERO);
    let salvager = looter(2, at, None, Duration::ZERO);
    let mut ap = Autoplay {
        team: view_of(vec![salvager.clone()]),
        ..Default::default()
    };
    ap.config.team.enabled = true;
    let left = ShutFor {
        at,
        left_for: vec![salvager.guid],
        waits: true,
        ..Default::default()
    };
    ap.corpse_shut(body, &Did::Done, None, left, t0);
    assert!(
        !ap.looted.contains(&body),
        "wrote off what it would take itself"
    );
    assert!(
        !ap.corpse_waiting(body, t0 + s(1), Room::PLENTY),
        "did not leave it to the salvager"
    );
    let packed = crate::logistics::Supplies {
        laden: true,
        ..Default::default()
    };
    for (why, row) in [
        (
            "dead",
            Some(Mate {
                health: 0.0,
                ..salvager.clone()
            }),
        ),
        (
            "played by hand",
            Some(Mate {
                autoplay: false,
                ..salvager.clone()
            }),
        ),
        (
            "out of reach",
            Some(Mate {
                world: glam::Vec3::new(LOOT_NEAR + 5.0, 0.0, 0.0),
                ..salvager.clone()
            }),
        ),
        (
            "laden",
            Some(Mate {
                supplies: packed.clone(),
                ..salvager.clone()
            }),
        ),
        (
            "not looting",
            Some(Mate {
                opens_bodies: false,
                ..salvager.clone()
            }),
        ),
        ("off the board", None),
    ] {
        ap.team = view_of(row.into_iter().collect());
        assert!(
            ap.corpse_waiting(body, t0 + s(1), Room::PLENTY),
            "waited on a salvager {why}"
        );
    }
    // Able to come, it is waited on, but not for ever.
    ap.team = view_of(vec![salvager.clone()]);
    assert!(!ap.corpse_waiting(body, t0 + STAND_BY_FOR - s(1), Room::PLENTY));
    let later = t0 + STAND_BY_FOR;
    assert!(
        ap.corpse_waiting(body, later, Room::PLENTY),
        "waited for good on one that never came"
    );
    // Given up on, it is left nothing on that body again, however able
    // it looks: left it again, it was stood by for again, and again.
    ap.stop_standing_by(later);
    assert!(!ap.may_be_sent(body, &salvager, at));
    assert!(ap.may_be_sent(0x8000_0002, &salvager, at));
    assert!(ap.corpse_waiting(body, later, Room::PLENTY));
    // Opened again and emptied, it is done with, and going back to it
    // was no turn.
    ap.corpse_shut(body, &Did::Done, None, ShutFor::default(), later);
    assert!(ap.looted.contains(&body));
    assert_eq!(ap.opened_first(later), 1);
}

#[test]
fn a_fellow_standing_by_goes_by_the_newest_word_on_the_body() {
    let t0 = Instant::now();
    let (body, at) = (0x8000_0001, glam::Vec3::ZERO);
    let (a, b, me, mage) = (1, 2, 3, 4);
    let row = |guid: u32, shut: Vec<Shut>| Mate {
        shut,
        ..looter(guid, at, None, Duration::ZERO)
    };
    let mut ap = Autoplay::default();
    ap.config.enabled = true;
    ap.config.team.enabled = true;
    // A shut it and left the kit on it for B. This character would take
    // the kit too, and stands by for B; the mage wants nothing on it.
    let a_shut = Shut {
        body,
        n: 0,
        done_for: vec![mage],
        stand_by: vec![me],
        left_for: vec![b],
    };
    ap.team = view_of(vec![row(a, vec![a_shut.clone()]), row(b, Vec::new())]);
    assert_eq!(
        ap.take_in_shuts(me, t0, |_| Some(at)),
        vec![TakenIn::StandBy {
            body,
            by: "Bryn01".into(),
            on: vec![b],
        }]
    );
    assert!(!ap.corpse_waiting(body, t0, Room::PLENTY));
    // Nothing on it is left for the mage, told it is done with it,
    // whatever a buff makes of its skills later.
    assert!(!ap.may_judge(body, &looter(mage, at, None, Duration::ZERO), at));
    assert!(ap.may_be_sent(body, &looter(b, at, None, Duration::ZERO), at));

    // B carried its fill already, and shut it leaving the kit to anyone
    // that wants it: this character opens it.
    let b_shut = Shut {
        body,
        n: 7,
        done_for: vec![mage],
        ..Default::default()
    };
    ap.team = view_of(vec![
        row(a, vec![a_shut.clone()]),
        row(b, vec![b_shut.clone()]),
    ]);
    assert!(ap.take_in_shuts(me, t0, |_| Some(at)).is_empty());
    assert!(
        ap.corpse_waiting(body, t0, Room::PLENTY),
        "the kit lay there though it was wanted"
    );
    // Nothing on it goes back to either that shut it.
    for shutter in [a, b] {
        assert!(!ap.may_be_sent(body, &looter(shutter, at, None, Duration::ZERO), at));
    }
    // The same shuts heard again change nothing.
    assert!(ap.take_in_shuts(me, t0, |_| Some(at)).is_empty());
    assert!(ap.corpse_waiting(body, t0, Room::PLENTY));
    // A's next shut of it, having opened it again, is new word.
    let again = Shut {
        body,
        n: 1,
        done_for: vec![me, mage],
        ..Default::default()
    };
    ap.team = view_of(vec![row(a, vec![again]), row(b, vec![b_shut])]);
    assert_eq!(
        ap.take_in_shuts(me, t0, |_| Some(at)),
        vec![TakenIn::Done {
            body,
            by: "Bryn01".into()
        }]
    );
    assert!(!ap.corpse_waiting(body, t0, Room::PLENTY));
}

#[test]
fn every_body_shut_as_emptied_is_said_so_the_others_know_who_shut_it() {
    // Said only when it was done for somebody, a shut that left
    // something for every fellow went unheard: going back to the body
    // counted as a turn, and things on it were left for the one that
    // had shut it already, which never came back.
    use crate::did::Did;
    let t0 = Instant::now();
    let (body, at) = (0x8000_0001, glam::Vec3::ZERO);
    let mut opener = Autoplay::default();
    opener.config.team.enabled = true;
    opener.corpse_shut(body, &Did::Done, None, ShutFor::default(), t0);
    let said = opener.shuts_to_say(t0);
    assert_eq!(
        said,
        vec![Shut {
            body,
            ..Default::default()
        }]
    );

    let mut ap = Autoplay {
        team: view_of(vec![Mate {
            shut: said,
            ..looter(1, at, None, Duration::ZERO)
        }]),
        ..Default::default()
    };
    ap.config.enabled = true;
    ap.config.team.enabled = true;
    assert!(ap.take_in_shuts(2, t0, |_| Some(at)).is_empty());
    assert!(ap.corpse_waiting(body, t0, Room::PLENTY));
    assert!(!ap.may_be_sent(body, &looter(1, at, None, Duration::ZERO), at));
    ap.corpse_shut(body, &Did::Done, None, ShutFor::default(), t0);
    assert_eq!(ap.opened_first(t0), 0, "going back to it counted as a turn");
}

#[test]
fn a_fellow_that_does_not_open_bodies_is_never_dealt_one_or_left_anything() {
    // An Ust carrier with no loot profile salvaged best and stood a few
    // metres from the leader, and never opened a body: the salvage left
    // for it rotted. One down to the slots kept for a sale opens none
    // either.
    let (body, at) = (0x8000_0001, glam::Vec3::ZERO);
    let ust = Mate {
        has_ust: true,
        salvaging: 400,
        opens_bodies: false,
        ..looter(2, at, None, Duration::ZERO)
    };
    assert_eq!(ust.turn(), None);
    assert!(!ust.could_come_for(at));
    let ap = Autoplay::default();
    assert!(!ap.may_judge(body, &ust, at));
    assert!(!ap.may_be_sent(body, &ust, at));
    // Not dealt a newly fallen body, though it has had no turns.
    let me = 3;
    assert_eq!(
        view_of(vec![ust.clone()]).opens_first(body, at, turn_at(me, at, 5)),
        me
    );
    // So its salvage is nobody's in particular: whoever opens the body
    // takes it, and the hand-off carries it to the salvager as ever.
    let profile = crate::profile::Profile {
        name: "party".into(),
        rules: vec![asks(
            "platemail to salvage",
            "platemail",
            LootAction::Salvage,
        )],
        ..Default::default()
    };
    let sheet = crate::weapons::Wielder::default();
    let taker = Taker {
        guid: me,
        name: "Bryn03",
        sheet: &sheet,
        held: 0,
    };
    let plate = item("Platemail", 100, 240);
    assert_eq!(
        called_to(&plate, None, &profile, &[taker], Some(ust.guid)),
        None
    );
}

#[test]
fn only_fellows_that_could_open_a_body_are_judged_at_its_shut() {
    // A character played by hand was named in "left for", and a body
    // found done for it was written off while its rules were off: the
    // player turned them on beside the body, and it was walked past.
    let t0 = Instant::now();
    let (body, at) = (0x8000_0001, glam::Vec3::ZERO);
    let ap = Autoplay::default();
    let able = looter(2, at, None, Duration::ZERO);
    assert!(ap.may_judge(body, &able, at));
    for (why, m) in [
        (
            "played by hand",
            Mate {
                autoplay: false,
                ..able.clone()
            },
        ),
        (
            "dead",
            Mate {
                health: 0.0,
                ..able.clone()
            },
        ),
        (
            "across the field",
            Mate {
                world: glam::Vec3::new(LOOT_NEAR + 5.0, 0.0, 0.0),
                ..able.clone()
            },
        ),
        (
            "with a full pack",
            Mate {
                supplies: crate::logistics::Supplies {
                    pack_full: true,
                    ..Default::default()
                },
                ..able.clone()
            },
        ),
    ] {
        assert!(!ap.may_judge(body, &m, at), "judged a fellow {why}");
    }

    // With its own rules off, a character notes who shut the body and
    // writes nothing off.
    let me = 3;
    let mut ap = Autoplay::default();
    ap.config.enabled = false;
    ap.config.team.enabled = true;
    ap.team.mates = vec![Mate {
        shut: vec![Shut {
            body,
            done_for: vec![me],
            ..Default::default()
        }],
        ..looter(1, at, None, Duration::ZERO)
    }];
    assert!(ap.take_in_shuts(me, t0, |_| Some(at)).is_empty());
    assert!(ap.has_shut(body, 1));
    assert!(
        ap.corpse_waiting(body, t0, Room::PLENTY),
        "wrote a body off while played by hand"
    );
}

#[test]
fn a_body_emptied_long_ago_is_forgotten_before_its_guid_can_come_back() {
    // ACE hands a released guid out again once it has been free for six
    // hours. Remembered for a whole long session, a new body that came
    // with an emptied one's guid was never opened.
    let t0 = Instant::now();
    let body = 0x8000_0001;
    let mut ap = Autoplay::default();
    ap.looted.push(body, t0);
    ap.looted
        .forget_old(t0 + EMPTIED_KEPT - Duration::from_secs(1));
    assert!(!ap.corpse_waiting(body, t0 + CORPSE_LIFE, Room::PLENTY));
    ap.looted.forget_old(t0 + EMPTIED_KEPT);
    assert!(
        ap.corpse_waiting(body, t0 + EMPTIED_KEPT, Room::PLENTY),
        "a new body with an old guid was never opened"
    );
}

#[test]
fn a_session_deals_itself_by_what_it_last_said_about_itself() {
    // The others read this character off its row, up to half a second
    // old. Read as it is now -- at another body, three turns in -- it
    // dealt a newly fallen body to a mate while the mate, reading the
    // row, dealt it back, and for a second nobody opened it.
    let t0 = Instant::now();
    let (body, other_body, at) = (0x8000_0001, 0x8000_0002, glam::Vec3::ZERO);
    let me = 0x5000_0011;
    let mut ap = Autoplay {
        corpse_seen: vec![(body, t0)],
        first_opens: vec![t0; 3],
        team: view_of(vec![Mate {
            opened_first: 1,
            ..looter(0x5000_0012, at, None, Duration::ZERO)
        }]),
        ..Default::default()
    };
    ap.take_up_corpse(other_body, t0, LOOT_TIMEOUT);
    // What it last said: free, and no turns yet.
    ap.team.me = Some(looter(me, at, None, Duration::ZERO));
    assert!(
        ap.ours_to_open(body, at, me, at, t0),
        "dealt itself out on what the others had not heard"
    );
    // Once it has said it is at another body, with three turns, the
    // mate has it, as the mate reckons too.
    ap.team.me = Some(Mate {
        opened_first: 3,
        ..looter(me, at, Some(other_body), Duration::ZERO)
    });
    assert!(!ap.ours_to_open(body, at, me, at, t0));
}

#[test]
fn only_the_skills_the_rules_ask_about_go_on_the_row() {
    use ac_world::stats::{sac, skill};
    let me = crate::weapons::Wielder {
        level: 30,
        skills: vec![
            (skill::LOCKPICK, 200, 260, sac::TRAINED),
            (skill::SALVAGING, 100, 100, sac::TRAINED),
            (skill::WAR_MAGIC, 300, 340, sac::SPECIALIZED),
        ],
        ..Default::default()
    };
    // Buffs and all: a rule on a skill reads it as it stands.
    assert_eq!(
        skills_asked_of(&me, &[skill::LOCKPICK]),
        vec![(skill::LOCKPICK, 200, 260, sac::TRAINED)]
    );
    // A skill asked about that the sheet lacks is not made up: off the
    // row it reads as nothing, as it does for the character itself.
    let row = Mate {
        skills: skills_asked_of(&me, &[skill::LOCKPICK, skill::HEALING]),
        ..Default::default()
    };
    assert_eq!(row.skills.len(), 1);
    assert_eq!(
        row.wielder().skill(skill::HEALING),
        me.skill(skill::HEALING)
    );
    assert_eq!(row.wielder().skill(skill::LOCKPICK), 260);
    // Rules that ask about no skill put none on the row.
    assert!(skills_asked_of(&me, &[]).is_empty());
}

#[test]
fn a_body_one_of_the_others_emptied_for_everyone_is_not_opened_again() {
    // Nine characters opened 83 bodies 697 times, and 42% of the opens
    // took nothing: the others opened a body one of them had emptied,
    // to find it so. The one that shuts it says whom it found nothing
    // left on it for, and those leave it alone.
    let t0 = Instant::now();
    let (me, other) = (2, 3);
    let (body, another) = (0x8000_0001, 0x8000_0002);
    let (here, at) = (glam::Vec3::ZERO, glam::Vec3::new(3.0, 0.0, 0.0));
    let mut ap = Autoplay {
        corpse_seen: vec![(body, t0), (another, t0)],
        ..Default::default()
    };
    ap.config.enabled = true;
    ap.config.team.enabled = true;
    ap.left_for_weight.insert(body, 300);
    assert!(ap.corpse_owed(body, at, here, t0, Room::PLENTY));
    ap.team.mates = vec![Mate {
        shut: vec![Shut {
            body,
            done_for: vec![me, other],
            ..Default::default()
        }],
        ..looter(1, here, None, Duration::ZERO)
    }];
    assert_eq!(
        ap.take_in_shuts(me, t0, |_| None),
        vec![TakenIn::Done {
            body,
            by: "Bryn01".into()
        }]
    );
    assert!(
        !ap.corpse_waiting(body, t0, Room::PLENTY),
        "went to open it again"
    );
    assert!(!ap.corpse_owed(body, at, here, t0, Room::PLENTY));
    // Nothing waits on room for it any more.
    assert_eq!(ap.lightest_left_for_weight(|g| g == body), None);
    // The rest of the ground is as it was.
    assert!(ap.corpse_owed(another, at, here, t0, Room::PLENTY));
    // Heard again the next round: taken in, and logged, once.
    assert!(ap.take_in_shuts(me, t0, |_| None).is_empty());
    assert_eq!(ap.looted.len(), 1);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_body_left_for_a_mate_still_waits_on_that_mate() {
    // One party, one profile, and a broken key left on a body whose
    // opener cannot mend it. The mage is told the body is done with and
    // stays away; the lockpicker is not, and still goes to it.
    use crate::did::Did;
    let mut c = character_of_level(game_data(), 30);
    let t0 = Instant::now();
    let profile = a_party_profile();
    let (body, key) = (0x8000_3001, 0x8000_3002);
    c.world.objects.insert(
        body,
        ac_world::WorldObject {
            guid: body,
            name: "Corpse of Drudge Slave".into(),
            object_desc_flags: ac_world::object_desc_flags::CORPSE,
            position: Some(ac_world::object::Position::new_flat(
                0xA9B4_0019,
                glam::Vec3::new(84.0, 84.0, 0.0),
            )),
            ..Default::default()
        },
    );
    let at = c.world.objects[&body].world_pos().expect("in the world");
    c.world.objects.insert(
        key,
        ac_world::WorldObject {
            guid: key,
            name: "Broken Marble Key".into(),
            container: Some(body),
            ..Default::default()
        },
    );
    let (picker, mage) = (
        Mate {
            world: at,
            ..picking(2, 300)
        },
        Mate {
            world: at,
            ..picking(3, 5)
        },
    );
    c.autoplay.team.mates = vec![picker.clone(), mage.clone()];
    let shut = c.shut_for(body, &[key], &profile);
    assert_eq!(
        shut,
        ShutFor {
            at,
            done_for: vec![mage.guid],
            left_for: vec![picker.guid],
            ..Default::default()
        }
    );

    // Shut, said on the board, and heard by each.
    c.autoplay.config.team.enabled = true;
    c.autoplay.corpse_shut(body, &Did::Done, None, shut, t0);
    let said = Mate {
        shut: c.autoplay.shuts_to_say(t0),
        ..looter(1, at, None, Duration::ZERO)
    };
    let still_waiting_for = |guid: u32| {
        let mut ap = Autoplay {
            team: view_of(vec![said.clone()]),
            ..Default::default()
        };
        ap.config.enabled = true;
        ap.config.team.enabled = true;
        ap.take_in_shuts(guid, t0, |_| Some(at));
        ap.corpse_waiting(body, t0, Room::PLENTY)
    };
    assert!(
        !still_waiting_for(mage.guid),
        "the mage went to open it for nothing"
    );
    assert!(
        still_waiting_for(picker.guid),
        "the key was not waited on for the lockpicker"
    );

    // In a fellowship with only the mage, the lockpicker is not judged
    // at all: it opens the body or not by its own lights, as before.
    let fellow = |guid| ac_world::Fellow {
        guid,
        ..Default::default()
    };
    c.world.fellowship = Some(ac_world::Fellowship {
        members: vec![fellow(0x5000_0001), fellow(mage.guid)],
        ..Default::default()
    });
    assert_eq!(
        c.shut_for(body, &[key], &profile),
        ShutFor {
            at,
            done_for: vec![mage.guid],
            ..Default::default()
        }
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_leader_that_does_not_sort_first_gives_up_the_fellowship_it_founded() {
    // Two fellowships for one fleet, and this character founded the
    // one whose leader does not sort first. Once the rightful leader
    // has been seen in a fellowship of its own for a few seconds,
    // this one is disbanded, so the rightful leader can recruit its
    // members and this character with them.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_in_the_field(20, holtburg, glam::Vec3::new(84.0, 84.0, 10.0));
    let t0 = Instant::now();
    let me = c.world.player_guid.unwrap();
    let member = 0x5000_0005;
    c.autoplay.config.team.enabled = true;
    c.autoplay.config.team.fellowship = true;
    let fellow = |guid| ac_world::Fellow {
        guid,
        ..Default::default()
    };
    c.world.fellowship = Some(ac_world::Fellowship {
        name: "acreborn".into(),
        leader: me,
        members: vec![fellow(me), fellow(member)],
        ..Default::default()
    });
    c.autoplay.founded = Some(t0);
    c.autoplay.team = TeamView {
        mates: vec![
            Mate {
                name: "+Brynith".into(),
                guid: 0x5000_0002,
                in_fellowship: true,
                leader: true,
                autoplay: true,
                ..Default::default()
            },
            Mate {
                name: "+Brynwyn".into(),
                guid: member,
                in_fellowship: true,
                ..Default::default()
            },
        ],
        leader: false,
        settled: true,
        ..Default::default()
    };
    // The first sight starts the clock; the board's word on a mate
    // is up to a round old, so it is given a few rounds to agree
    // with the world's.
    assert!(!c.autoplay_fellowship(t0));
    assert!(!c.autoplay_fellowship(t0 + YIELD_AFTER / 2));
    assert!(c.autoplay_fellowship(t0 + YIELD_AFTER), "not given up");
    assert!(c.autoplay.founded.is_none());
    assert!(
        c.autoplay.status.contains("disbanding"),
        "{}",
        c.autoplay.status
    );
    // One this character did not found is never given up, whoever
    // leads: the note on a fellowship "not founded by me" stands.
    c.autoplay.founded = None;
    c.autoplay.status.clear();
    assert!(!c.autoplay_fellowship(t0 + YIELD_AFTER * 4));
    assert!(!c.autoplay_fellowship(t0 + YIELD_AFTER * 8));
    assert!(!c.autoplay.status.contains("disbanding"));
    // Nor is one this character founded while it leads the team.
    c.autoplay.founded = Some(t0);
    c.autoplay.team.leader = true;
    assert!(!c.autoplay_fellowship(t0 + YIELD_AFTER * 12));
    assert!(c.autoplay.founded.is_some());
}

/// A character leading a fellowship it founded, with `others` in
/// it besides itself, on a settled team.
fn leading_a_fellowship(c: &mut Client, others: &[u32], t0: Instant) {
    let me = c.world.player_guid.unwrap();
    c.autoplay.config.team.enabled = true;
    c.autoplay.config.team.fellowship = true;
    let fellow = |guid| ac_world::Fellow {
        guid,
        ..Default::default()
    };
    c.world.fellowship = Some(ac_world::Fellowship {
        name: "acreborn".into(),
        leader: me,
        members: std::iter::once(me)
            .chain(others.iter().copied())
            .map(fellow)
            .collect(),
        ..Default::default()
    });
    c.autoplay.founded = Some(t0);
    c.autoplay.team.settled = true;
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn whoever_leads_the_fellowship_brings_in_the_rightful_leader_standing_outside() {
    // +Brynlyn founded and gathered the seven that came on with it;
    // +Brynith, first by name, came on a few seconds later. Every
    // roster's leader flipped to +Brynith the moment it was heard:
    // +Brynlyn stopped recruiting, since it led the team no longer,
    // and +Brynith, with nobody free to found with, founded
    // nothing. Seven in a fellowship and the team's leader outside
    // it for the life of the run. Whoever leads a fellowship brings
    // the team's mates into it, the rightful leader among them.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_in_the_field(20, holtburg, glam::Vec3::new(84.0, 84.0, 10.0));
    let t0 = Instant::now();
    let here = c.player.as_ref().unwrap().world_position();
    let (rightful, member) = (0x5000_0002, 0x5000_0005);
    leading_a_fellowship(&mut c, &[member], t0);
    c.autoplay.team.leader = false;
    c.autoplay.team.mates = vec![
        Mate {
            name: "+Brynith".into(),
            guid: rightful,
            in_fellowship: false,
            leader: true,
            autoplay: true,
            world: here,
            ..Default::default()
        },
        Mate {
            name: "+Brynwyn".into(),
            guid: member,
            in_fellowship: true,
            world: here,
            ..Default::default()
        },
    ];
    assert!(c.autoplay_fellowship(t0), "{}", c.autoplay.status);
    assert!(
        c.autoplay
            .status
            .contains("bringing +Brynith into the fellowship"),
        "{}",
        c.autoplay.status
    );
    assert_eq!(
        c.autoplay
            .recruited
            .iter()
            .map(|(g, _)| *g)
            .collect::<Vec<_>>(),
        vec![rightful]
    );
    // The other way about, nothing: a team leader that let itself
    // be recruited into a mate's fellowship cannot recruit into it
    // (the server answers 0x041D), and leaves the gathering to that
    // mate.
    c.autoplay.recruited.clear();
    c.autoplay.last_recruit = None;
    c.world.fellowship.as_mut().unwrap().leader = member;
    c.autoplay.founded = None;
    c.autoplay.team.leader = true;
    c.autoplay.team.mates[0].leader = false;
    assert!(!c.autoplay_fellowship(t0 + RECRUIT_AGAIN));
    assert!(c.autoplay.recruited.is_empty());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_full_fellowship_asks_nobody_else() {
    // Nine in, a tenth on the team: the server answered 0x041E to
    // every ask and the tenth was asked every five seconds for the
    // life of the run. The count says so first, once; and the code,
    // should the server's count differ, holds off whoever was asked
    // last.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_in_the_field(20, holtburg, glam::Vec3::new(84.0, 84.0, 10.0));
    let t0 = Instant::now();
    let here = c.player.as_ref().unwrap().world_position();
    let eight: Vec<u32> = (0..8).map(|i| 0x5000_0010 + i).collect();
    leading_a_fellowship(&mut c, &eight, t0);
    c.autoplay.team.leader = true;
    let tenth = 0x5000_0030;
    c.autoplay.team.mates = eight
        .iter()
        .map(|&guid| Mate {
            name: format!("+Bryn{guid:x}"),
            guid,
            in_fellowship: true,
            world: here,
            ..Default::default()
        })
        .chain(std::iter::once(Mate {
            name: "+Brynzed".into(),
            guid: tenth,
            world: here,
            ..Default::default()
        }))
        .collect();
    // A hold on somebody no longer on the team goes with them.
    let gone = 0x5000_0099;
    c.autoplay.held_off.hold(gone, HELD_OFF_FIRST, t0);
    assert!(!c.autoplay_fellowship(t0));
    assert!(c.autoplay.recruited.is_empty(), "the tenth was asked");
    assert!(
        c.autoplay
            .noted
            .iter()
            .any(|(t, _)| t.contains("the fellowship is full at 9: 1 mate(s)")),
        "{:?}",
        c.autoplay.noted
    );
    assert!(!c.autoplay.held_off.held(&gone, t0));
    // The server's own word on it, about the last one asked.
    c.autoplay.recruited = vec![(tenth, t0)];
    let soon = t0 + Duration::from_secs(1);
    c.autoplay.hear_fellowship_full(soon);
    assert!(c.autoplay.held_off.held(&tenth, soon));
    assert!(!c.autoplay.held_off.held(&tenth, soon + HELD_OFF_FIRST));
}

#[test]
fn a_body_emptied_by_a_mate_stays_emptied_after_the_mate_goes_quiet() {
    // The row goes from the board once its mate has been quiet for six
    // seconds, and a shut is said for twenty; what it said stays said.
    let t0 = Instant::now();
    let (me, body) = (2, 0x8000_0001);
    let mut ap = Autoplay::default();
    ap.config.enabled = true;
    ap.config.team.enabled = true;
    ap.team.mates = vec![Mate {
        shut: vec![Shut {
            body,
            done_for: vec![me],
            ..Default::default()
        }],
        ..looter(1, glam::Vec3::ZERO, None, Duration::ZERO)
    }];
    assert_eq!(ap.take_in_shuts(me, t0, |_| None).len(), 1);
    ap.team = TeamView::default();
    assert!(!ap.corpse_waiting(body, t0 + SHUT_SAID_FOR * 3, Room::PLENTY));
    // Nor does the mate that said it saying nothing more undo it.
    ap.team.mates = vec![looter(1, glam::Vec3::ZERO, None, Duration::ZERO)];
    assert!(ap.take_in_shuts(me, t0, |_| None).is_empty());
    assert!(!ap.corpse_waiting(body, t0 + SHUT_SAID_FOR * 3, Room::PLENTY));
}

#[test]
fn a_body_set_aside_or_left_for_its_weight_is_never_done_for_anyone_else() {
    // Shut for want of room, or because something would not come off,
    // a body still has on it what somebody wanted.
    use crate::did::Did;
    let t0 = Instant::now();
    let mut ap = Autoplay::default();
    ap.config.team.enabled = true;
    let (stubborn, heavy, emptied) = (0x8000_4001, 0x8000_4002, 0x8000_4003);
    let for_both = || ShutFor {
        done_for: vec![2, 3],
        ..Default::default()
    };
    ap.corpse_shut(
        stubborn,
        &Did::blocked("it would not give something up"),
        None,
        for_both(),
        t0,
    );
    ap.corpse_shut(
        heavy,
        &Did::blocked("too laden to take the rest"),
        Some(300),
        for_both(),
        t0,
    );
    assert!(
        ap.shuts_to_say(t0).is_empty(),
        "said a body with something on it was emptied"
    );
    ap.corpse_shut(emptied, &Did::Done, None, for_both(), t0);
    assert_eq!(
        ap.shuts_to_say(t0),
        vec![Shut {
            body: emptied,
            n: 0,
            done_for: vec![2, 3],
            ..Default::default()
        }]
    );
}

#[test]
fn a_mate_under_its_cap_is_left_the_healing_kits_the_opener_would_not_take() {
    use crate::profile::Verdict;
    let profile = a_party_profile();
    let kits = ItemStats {
        stack: 5,
        ..item("Healing Kit", 50, 0)
    };
    // The opener carries four already: the rule stops there, and
    // nothing else claims them.
    let opener = picking(1, 0);
    assert_eq!(
        judge_loot(&kits, None, Some(&profile), &opener.wielder(), "Bryn01", 4),
        Verdict::None
    );
    // Nobody knows what a mate carries, so the mate is not done with the
    // body, and opens it by its own lights: the rule asks about no
    // skill, so the kits are nobody's in particular.
    let mate = picking(2, 0);
    let sheet = opener.wielder();
    let me = Taker {
        guid: opener.guid,
        name: &opener.name,
        sheet: &sheet,
        held: 4,
    };
    let fellows = [as_fellow(&mate)];
    assert_eq!(
        judge_shut(&[lying(kits, 4)], &profile, me, &fellows, &fellows, None),
        ShutFor::default()
    );
}

#[test]
fn a_mate_that_would_need_an_appraisal_is_not_counted_done() {
    let mut profile = a_party_profile();
    profile
        .rules
        .push(asks("good armour", "type:armor al>=200", LootAction::Sell));
    let mage = picking(3, 0);
    let sheet = crate::weapons::Wielder::default();
    let me = Taker {
        guid: 1,
        name: "Bryn01",
        sheet: &sheet,
        held: 0,
    };
    let done = |lying: &[Option<Left>], profile: &crate::profile::Profile| {
        judge_shut(lying, profile, me, &[as_fellow(&mage)], &[], None).done_for == vec![mage.guid]
    };
    let unread = ItemStats {
        appraised: false,
        ..item("Platemail", 100, 240)
    };
    // Not appraised, it might be good armour: the mate would ask.
    assert!(!done(&[lying(unread.clone(), 0)], &profile));
    // Appraised by the one that shut it, it is judged outright.
    assert!(!done(&[lying(item("Platemail", 100, 240), 0)], &profile));
    assert!(done(&[lying(item("Platemail", 100, 50), 0)], &profile));
    // Where the rules never appraise, the mate would never ask either.
    profile.looting.appraise = false;
    assert!(done(&[lying(unread, 0)], &profile));
    // A thing not described yet cannot be judged, and is waited on.
    assert!(!done(&[None], &profile));
}

#[test]
fn a_mate_outside_the_fellowship_is_never_counted_done_or_left_anything() {
    let at = glam::Vec3::ZERO;
    let view = view_of(vec![
        looter(1, at, None, Duration::ZERO),
        looter(2, at, None, Duration::ZERO),
        // Not in the world yet.
        looter(0, at, None, Duration::ZERO),
    ]);
    let judged = |fellows: Option<&[u32]>| {
        view.judged_at_a_shut(fellows)
            .map(|m| m.guid)
            .collect::<Vec<_>>()
    };
    // Out of a fellowship, everyone in the world.
    assert_eq!(judged(None), vec![1, 2]);
    // In one, its fellows only.
    assert_eq!(judged(Some(&[9, 1])), vec![1]);
}

#[test]
fn what_a_shut_says_is_bounded_and_ages_off() {
    use crate::did::Did;
    let t0 = Instant::now();
    let s = Duration::from_secs;
    let mut ap = Autoplay::default();
    // Off the team there is nobody to say it to.
    ap.corpse_shut(0x8000_0001, &Did::Done, None, ShutFor::default(), t0);
    assert!(ap.shuts_to_say(t0).is_empty());
    ap.config.team.enabled = true;
    // Only the newest are said.
    let first = 0x8000_1000;
    for n in 0..SHUTS_SAID as u32 + 4 {
        let at = t0 + Duration::from_millis(n as u64);
        let shut = ShutFor {
            done_for: vec![2],
            ..Default::default()
        };
        ap.corpse_shut(first + n, &Did::Done, None, shut, at);
    }
    let said = ap.shuts_to_say(t0 + s(1));
    assert_eq!(said.len(), SHUTS_SAID);
    assert_eq!(said.first().map(|shut| shut.body), Some(first + 4));
    // A body shut again is said once, as a new shut.
    let again = ShutFor {
        done_for: vec![2, 3],
        ..Default::default()
    };
    ap.corpse_shut(first + 10, &Did::Done, None, again, t0 + s(2));
    let said = ap.shuts_to_say(t0 + s(2));
    assert_eq!(said.len(), SHUTS_SAID);
    let tenth: Vec<&Shut> = said.iter().filter(|shut| shut.body == first + 10).collect();
    assert_eq!(tenth.len(), 1);
    assert_eq!(tenth[0].n, SHUTS_SAID as u32 + 4);
    // And only for a while.
    assert!(ap.shuts_to_say(t0 + s(2) + SHUT_SAID_FOR).is_empty());
    // Done with here all the same, said or not.
    assert!(ap.looted.contains(&0x8000_0001) && ap.looted.contains(&first));
}

#[test]
fn a_character_alone_loots_exactly_as_it_did() {
    // Nobody on the board: nothing to say, nothing to hear, and every
    // body its own as before.
    use crate::did::Did;
    let t0 = Instant::now();
    let (me, body) = (2, 0x8000_0001);
    let (here, at) = (glam::Vec3::ZERO, glam::Vec3::new(3.0, 0.0, 0.0));
    let mut ap = Autoplay {
        corpse_seen: vec![(body, t0)],
        ..Default::default()
    };
    ap.config.enabled = true;
    for team in [false, true] {
        ap.config.team.enabled = team;
        assert!(ap.ours_to_open(body, at, me, here, t0));
        assert!(ap.corpse_owed(body, at, here, t0, Room::PLENTY));
        assert_eq!(ap.team.judged_at_a_shut(None).count(), 0);
        assert!(ap.take_in_shuts(me, t0, |_| None).is_empty());
        assert!(ap.looted.is_empty());
    }
    ap.config.team.enabled = false;
    ap.take_up_corpse(body, t0, LOOT_TIMEOUT);
    ap.corpse_shut(body, &Did::Done, None, ShutFor::default(), t0);
    assert!(ap.looted.contains(&body) && ap.looted.len() == 1);
    assert!(ap.shuts_to_say(t0).is_empty(), "told nobody about it");
    assert!(!ap.corpse_waiting(body, t0, Room::PLENTY));

    // And a character alone finds nobody to judge a body for.
    let c = character_of_level(no_data(), 30);
    assert_eq!(
        c.shut_for(body, &[0x8000_0002], &a_party_profile()),
        ShutFor::default()
    );
}

#[test]
fn a_character_off_the_team_takes_in_no_shuts() {
    let t0 = Instant::now();
    let (me, body) = (2, 0x8000_0001);
    let mut ap = Autoplay::default();
    ap.config.enabled = true;
    assert!(!ap.config.team.enabled);
    ap.team.mates = vec![Mate {
        shut: vec![Shut {
            body,
            done_for: vec![me],
            ..Default::default()
        }],
        ..looter(1, glam::Vec3::ZERO, None, Duration::ZERO)
    }];
    assert!(ap.take_in_shuts(me, t0, |_| None).is_empty());
    assert!(ap.looted.is_empty());
    assert!(ap.corpse_waiting(body, t0, Room::PLENTY));
    // Back on the team, the same word is taken in.
    ap.config.team.enabled = true;
    assert_eq!(
        ap.take_in_shuts(me, t0, |_| None),
        vec![TakenIn::Done {
            body,
            by: "Bryn01".into()
        }]
    );
}

fn view_of(mates: Vec<Mate>) -> TeamView {
    TeamView {
        mates,
        ..Default::default()
    }
}

#[test]
fn a_body_the_plan_deals_is_the_dealt_ones_while_the_plan_is_fresh_and_from_the_leader() {
    use crate::plan::{Order, Plan, ORDERS_LAST};
    let t0 = Instant::now();
    let (body, at) = (0x8000_0001, glam::Vec3::ZERO);
    let (me, other) = (2, 3);
    let mut ap = Autoplay::default();
    ap.config.enabled = true;
    ap.config.team.enabled = true;
    let mut leader = looter(1, at, None, Duration::ZERO);
    leader.leader = true;
    ap.team = view_of(vec![leader, looter(other, at, None, Duration::ZERO)]);
    ap.team.me = Some(looter(me, at, None, Duration::ZERO));
    ap.corpse_seen.push((body, t0));
    let plan = |to: u32| {
        let mut p = Plan {
            leader: "Bryn01".into(),
            ..Default::default()
        };
        p.orders.insert(
            to,
            Order {
                target: None,
                body: Some(body),
            },
        );
        p
    };
    // Dealt to another: left to it, and the log says whose it is.
    ap.take_orders(plan(other), t0);
    assert!(!ap.ours_to_open(body, at, me, at, t0));
    assert_eq!(ap.whose_turn(body, at, me, at, t0), Some("Bryn03"));
    // Still the other's once the turns would have let anyone free
    // take it (see `CLAIM_SETTLE`): the deal is the leader's word.
    let settled = t0 + CLAIM_SETTLE * 2;
    assert!(!ap.ours_to_open(body, at, me, at, settled));
    // Dealt to this character: its own.
    ap.take_orders(plan(me), t0);
    assert!(ap.ours_to_open(body, at, me, at, t0));
    // The leader gone quiet: the plan lapses and the turns say again.
    ap.take_orders(plan(other), t0);
    let late = t0 + ORDERS_LAST + CLAIM_SETTLE * 2;
    assert!(
        ap.ours_to_open(body, at, me, at, late),
        "a stale plan held a body"
    );
    // A plan signed by one not leading is nobody's to obey.
    let mut theirs = plan(other);
    theirs.leader = "Bryn03".into();
    ap.take_orders(theirs, late);
    assert!(ap.ours_to_open(body, at, me, at, late));
    // And a mate's standing claim on the body outranks a deal to us:
    // the deal is a plan, the claim is a body already in hand.
    ap.take_orders(plan(me), late);
    ap.team.mates[1].looting = Some(body);
    ap.team.mates[1].looting_for = Duration::from_secs(2);
    assert!(!ap.ours_to_open(body, at, me, at, late));
    // Off the team, no plan is obeyed at all.
    ap.team.mates[1].looting = None;
    ap.config.team.enabled = false;
    assert_eq!(ap.body_dealt_to(body, late), None);
}

#[test]
fn a_leader_holds_the_next_fight_for_a_body_it_dealt_to_another_while_it_lies_there() {
    use crate::plan::{Order, Plan, ORDERS_LAST};
    let t0 = Instant::now();
    let (me, other) = (1, 2);
    let (body, gone) = (0x8000_0001, 0x8000_0002);
    let mut ap = Autoplay::default();
    ap.config.enabled = true;
    ap.config.team.enabled = true;
    ap.team = view_of(vec![looter(other, glam::Vec3::ZERO, None, Duration::ZERO)]);
    ap.team.leader = true;
    ap.team.me = Some(looter(me, glam::Vec3::ZERO, None, Duration::ZERO));
    // Nothing dealt: nothing held.
    assert_eq!(ap.body_dealt_to_another(me, t0, |_| true), None);
    // A plan dealing one body to the other, one to this character.
    let mut plan = Plan {
        leader: "Bryn01".into(),
        ..Default::default()
    };
    plan.orders.insert(
        other,
        Order {
            target: None,
            body: Some(body),
        },
    );
    plan.orders.insert(
        me,
        Order {
            target: None,
            body: Some(gone),
        },
    );
    ap.planner.dealt(
        &crate::plan::Dealt {
            deals: vec![(body, other), (gone, me)],
            lapsed: Vec::new(),
        },
        t0,
        DEAL_WINDOW,
    );
    ap.take_orders(plan, t0);
    assert_eq!(
        ap.body_dealt_to_another(me, t0, |_| true),
        Some((body, other)),
        "the other's body holds the leader; its own is its own to loot"
    );
    // Gone from the ground (rotted, or out of reach): nothing to hold for.
    assert_eq!(ap.body_dealt_to_another(me, t0, |g| g != body), None);
    // Said emptied by the one it went to: done with.
    ap.shut_by.insert((body, other, 0));
    assert_eq!(ap.body_dealt_to_another(me, t0, |_| true), None);
    ap.shut_by.clear();
    // A leader that has stopped planning holds nothing.
    assert_eq!(
        ap.body_dealt_to_another(me, t0 + ORDERS_LAST, |_| true),
        None
    );
}

#[test]
fn a_body_opened_first_is_a_turn_and_one_left_for_this_character_is_not() {
    use crate::did::Did;
    let t0 = Instant::now();
    let me = 2;
    let mut ap = Autoplay::default();
    ap.config.enabled = true;
    ap.config.team.enabled = true;
    assert_eq!(ap.opened_first(t0), 0);
    ap.corpse_shut(0x8000_0001, &Did::Done, None, ShutFor::default(), t0);
    assert_eq!(ap.opened_first(t0), 1);
    // One of the others shut this one first and left it for us: going
    // back for what it left is no turn.
    let passed_on = 0x8000_0002;
    ap.team.mates = vec![Mate {
        shut: vec![Shut {
            body: passed_on,
            done_for: vec![3],
            ..Default::default()
        }],
        ..looter(1, glam::Vec3::ZERO, None, Duration::ZERO)
    }];
    assert!(
        ap.take_in_shuts(me, t0, |_| None).is_empty(),
        "left for us, not done for us"
    );
    assert!(ap.has_shut(passed_on, 1));
    ap.corpse_shut(passed_on, &Did::Done, None, ShutFor::default(), t0);
    assert_eq!(ap.opened_first(t0), 1);
    // Nor is a body set aside.
    ap.corpse_shut(
        0x8000_0003,
        &Did::blocked("it would not give something up"),
        None,
        ShutFor::default(),
        t0,
    );
    assert_eq!(ap.opened_first(t0), 1);
    // Turns are counted lately, not for ever.
    assert_eq!(ap.opened_first(t0 + DEAL_WINDOW), 0);
    // And what was heard goes with the body.
    ap.forget_corpses_gone(|g| g != passed_on);
    assert!(!ap.has_shut(passed_on, 1));
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
fn a_mate_dead_or_missing_from_the_board_is_never_waited_on() {
    let t0 = Instant::now();
    let (body, at) = (0x8000_0001, glam::Vec3::ZERO);
    let alive = looter(2, at, None, Duration::ZERO);
    assert!(alive.could_come_for(at));
    // At another body or fighting, it comes once it is done.
    assert!(Mate {
        looting: Some(0x8000_0002),
        target: Some(0x7000_0001),
        ..alive.clone()
    }
    .could_come_for(at));
    for gone in [
        Mate {
            health: 0.0,
            ..alive.clone()
        },
        Mate {
            autoplay: false,
            ..alive.clone()
        },
        Mate {
            guid: 0,
            ..alive.clone()
        },
        Mate {
            opens_bodies: false,
            ..alive.clone()
        },
    ] {
        assert_eq!(gone.turn(), None);
        assert!(!gone.could_come_for(at));
    }
    // Nor one across the field, or with no room for what is left.
    assert!(!Mate {
        world: glam::Vec3::new(LOOT_NEAR + 5.0, 0.0, 0.0),
        ..alive.clone()
    }
    .could_come_for(at));
    for (pack_full, laden) in [(true, false), (false, true)] {
        let full = Mate {
            supplies: crate::logistics::Supplies {
                pack_full,
                laden,
                ..Default::default()
            },
            ..alive.clone()
        };
        assert!(!full.could_come_for(at));
    }

    // Nor is a body's turn dealt to one dead, or to one gone quiet and
    // off the board: either way the body is this character's.
    let me = 3;
    let mut ap = Autoplay {
        corpse_seen: vec![(body, t0)],
        first_opens: vec![t0; 3],
        team: view_of(vec![looter(1, at, None, Duration::ZERO)]),
        ..Default::default()
    };
    assert!(!ap.ours_to_open(body, at, me, at, t0), "had more turns");
    // And standing off it, this character says whose turn it is.
    assert_eq!(ap.whose_turn(body, at, me, at, t0), Some("Bryn01"));
    assert_eq!(ap.whose_turn(body, at, me, at, t0 + CLAIM_SETTLE), None);
    ap.team.mates[0].health = 0.0;
    assert!(
        ap.ours_to_open(body, at, me, at, t0),
        "waited on a dead mate's turn"
    );
    assert_eq!(ap.whose_turn(body, at, me, at, t0), None);
    ap.team.mates.clear();
    assert!(ap.ours_to_open(body, at, me, at, t0));
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
fn a_body_a_step_or_two_off_is_walked_back_to_and_not_let_go_of() {
    // The server does not mind the distance: a corpse has no reset
    // interval, so it stays open until its viewer shuts it. Shutting
    // one the character was already holding handed it back to the
    // other eight and the walk back had to win it again -- fifty
    // times in one run.
    //
    // A dodge or a knock-back moves a character several metres in a
    // frame, and that is the drift this covers.
    assert!(still_holding_at(CORPSE_REACH + 0.5), "no room to drift");
    assert!(still_holding_at(HOLD_ON_WITHIN));
    // Really gone: shut it and walk back to it like any other body.
    assert!(!still_holding_at(HOLD_ON_WITHIN + 0.5));
    // Never as far as the twenty metres the looting calls its own,
    // or a body held would be one nothing else could ever have.
    assert!(!still_holding_at(LOOT_NEAR));
}

#[test]
fn a_walk_to_a_corpse_that_cannot_arrive_is_given_up_and_the_corpse_set_aside() {
    // Blargerton walked every tick at bodies through a floor or
    // behind a wall. Nothing timed the walk, so the looting, and the
    // fight waiting on the body, held until it rotted.
    let t0 = Instant::now();
    let s = Duration::from_secs;
    let guid = 0x8000_3001;

    // A long walk that keeps getting nearer goes on however long it
    // takes.
    let mut walk = CorpseWalk::new(guid, 40.0, t0);
    for i in 1..=30 {
        assert!(walk.goes_on(40.0 - i as f32 * 1.2, false, t0 + s(i)));
    }

    // Pressed against a wall and no nearer: given up once that has
    // gone on too long, and not before.
    let mut walk = CorpseWalk::new(guid, 12.0, t0);
    for i in 1..=REACH_GIVE_UP.as_secs() {
        assert!(walk.goes_on(11.5, false, t0 + s(i)), "gave up at {i} s");
    }
    let gave_up = t0 + REACH_GIVE_UP + s(1);
    assert!(!walk.goes_on(11.5, false, gave_up));

    // The steering finding no way there is given up on sooner, but
    // not at the first word of it.
    let mut walk = CorpseWalk::new(guid, 12.0, t0);
    assert!(walk.goes_on(12.0, true, t0 + s(1)));
    assert!(walk.goes_on(12.0, false, t0 + s(2)), "a way was found");
    let said = t0 + s(3);
    for i in 3..3 + NO_WAY_FOR.as_secs() {
        assert!(walk.goes_on(12.0, true, t0 + s(i)), "gave up at {i} s");
    }
    assert!(!walk.goes_on(12.0, true, said + NO_WAY_FOR));

    // A walk broken off for a fight is a new walk when it is taken
    // up again.
    let mut walk = CorpseWalk::new(guid, 12.0, t0);
    assert!(walk.goes_on(12.0, false, t0 + s(1)));
    assert!(walk.goes_on(12.0, false, t0 + s(40)));

    // What `autoplay_loot` does with a walk given up: the body is set
    // aside, not written off, and neither the looting nor the next
    // fight waits on it meanwhile.
    let mut ap = Autoplay::default();
    let (me, at) = (glam::Vec3::ZERO, glam::Vec3::new(12.0, 0.0, 0.0));
    let room = Room::PLENTY;
    assert!(ap.corpse_owed(guid, at, me, gave_up, room));
    ap.set_aside_out_of_reach(guid, gave_up);
    assert!(!ap.looted.contains(&guid), "written off for good");
    assert!(!ap.corpse_owed(guid, at, me, gave_up, room), "still owed");
    assert!(
        ap.corpse_owed(guid, at, me, gave_up + s(60), room),
        "never tried again"
    );
}

#[test]
fn a_ranged_attacker_closes_in_before_it_gives_up() {
    // Nothing landing from forty metres: try from twenty, then ten,
    // then as near as is worth being -- and only then give up.
    assert_eq!(closer_stand_off(40.0), Some(20.0));
    assert_eq!(closer_stand_off(20.0), Some(10.0));
    assert_eq!(closer_stand_off(10.0), Some(MIN_STAND_OFF));
    assert_eq!(closer_stand_off(MIN_STAND_OFF), None);
    assert_eq!(closer_stand_off(MIN_STAND_OFF + 0.5), None);
}

#[test]
fn a_re_judged_pack_counts_each_kind_as_it_goes() {
    // Three rings and two piles of tapers, in the order they were
    // come by. Each ring is told how many rings were held before
    // it -- none, one, two -- not that three are carried, so a rule
    // that keeps up to two claims the first two and not the third.
    // Told instead that it was the second of two, the second ring
    // sat over the cap and only one was kept.
    let mut carried = vec![
        (30, 500, 1),   // third ring
        (10, 500, 1),   // first ring
        (25, 691, 300), // second pile of tapers
        (20, 500, 1),   // second ring
        (15, 691, 120), // first pile of tapers
    ];
    assert_eq!(
        in_arrival_order(&mut carried),
        vec![(10, 0), (15, 0), (20, 1), (25, 120), (30, 2)]
    );
    // A stack counts for what it holds, not for one.
    let mut two = vec![(7, 691, 4059), (8, 691, 1)];
    assert_eq!(in_arrival_order(&mut two), vec![(7, 0), (8, 4059)]);
    assert!(in_arrival_order(&mut []).is_empty());
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
fn a_corpse_the_rules_set_aside_is_shelved_and_not_written_off() {
    // The rules shut a corpse that will not give up its contents as
    // Blocked -- "later" -- and the client marked every shut corpse
    // looted, which is "never".
    use crate::did::Did;
    let t0 = Instant::now();
    let mut ap = Autoplay::default();
    let stubborn = 0x8000_2001;
    ap.take_up_corpse(stubborn, t0, LOOT_TIMEOUT);
    ap.corpse_shut(
        stubborn,
        &Did::blocked("it will not give up its contents"),
        None,
        ShutFor::default(),
        t0,
    );
    assert_eq!(ap.corpse, None, "still in hand");
    assert!(!ap.looted.contains(&stubborn), "written off for good");
    assert!(ap.shelved.held(&stubborn, t0), "not left alone for now");
    assert!(
        !ap.shelved.held(&stubborn, t0 + Duration::from_secs(60)),
        "never tried again"
    );
    // One the rules emptied is done with.
    let emptied = 0x8000_2002;
    ap.take_up_corpse(emptied, t0, LOOT_TIMEOUT);
    ap.corpse_shut(emptied, &Did::Done, None, ShutFor::default(), t0);
    assert!(ap.looted.contains(&emptied));
    assert!(!ap.shelved.held(&emptied, t0));
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
fn a_refusal_about_another_body_leaves_the_one_in_hand_alone() {
    let t0 = Instant::now();
    let body = 0x8000_1221;
    let asked = t0 + Duration::from_secs(1);
    let answered = asked + Duration::from_millis(360);
    let mut ap = Autoplay {
        corpse_seen: vec![(body, t0)],
        ..Default::default()
    };
    ap.take_up_corpse(body, asked, LOOT_TIMEOUT);

    // Nine characters stand in one huddle and the server answers all
    // of them: words about a body this character is not working
    // change nothing.
    assert!(!ap.corpse_refused_in_words(
        "The Corpse of Drudge Slave is already in use by someone else!",
        "Corpse of Hellion",
        Some(answered),
        answered,
    ));
    // Nor do words about something that is not a refusal.
    assert!(!ap.corpse_refused_in_words(
        "You're too busy",
        "Corpse of Hellion",
        Some(answered),
        answered
    ));
    // Nor an answer that came in before this ask went out: it was
    // the ask before it that was refused.
    let words = "You do not yet have the right to loot the Corpse of Hellion.";
    assert!(!ap.corpse_refused_in_words(words, "Corpse of Hellion", Some(asked), answered));
    assert!(!ap.corpse_refused_in_words(words, "Corpse of Hellion", None, answered));
    assert_eq!(
        ap.corpse.map(|c| c.0),
        Some(body),
        "let a body go for nothing"
    );
    assert!(ap.shelved.is_empty(), "set a body aside for nothing");

    // The same words, stamped after the ask, are this body's.
    assert!(ap.corpse_refused_in_words(words, "Corpse of Hellion", Some(answered), answered));
    assert_eq!(ap.corpse, None);
}

#[test]
fn no_body_waits_on_a_character_the_server_will_hand_nothing() {
    // +Verity, 36462 carried of a 7500 capacity, on her way to sell:
    // the looting walked her to a corpse for a Pyreal, the server said
    // "You are too encumbered to carry that!", and the walk to town
    // was lost. Past the wall no body is owed, so none is walked to.
    let t0 = Instant::now();
    let ap = Autoplay::default();
    let (me, at) = (glam::Vec3::ZERO, glam::Vec3::new(5.0, 0.0, 0.0));
    let body = 0x8000_9001;
    let walled = Room {
        past_the_wall: true,
        ..Room::PLENTY
    };
    assert!(ap.corpse_owed(body, at, me, t0, Room::PLENTY));
    assert!(!ap.corpse_waiting(body, t0, walled));
    assert!(!ap.corpse_owed(body, at, me, t0, walled));
    // Short of it, a character with no room left for loot still goes
    // to a body: coins weigh nothing, and light things may fit.
    let laden = Room {
        carry: 0,
        ..Room::PLENTY
    };
    assert!(ap.corpse_owed(body, at, me, t0, laden));
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
fn a_corpse_that_says_no_again_waits_twice_as_long() {
    // The looting tidied away each lapsed wait before choosing a
    // corpse, and a corpse is chosen again exactly when its wait is
    // up. So every refusal was the first: thirty seconds, never more,
    // and the character went back every half minute until it rotted.
    use crate::did::Did;
    let t0 = Instant::now();
    let s = Duration::from_secs;
    let mut ap = Autoplay::default();
    let (locked, rotted) = (0x8000_2101, 0x8000_2102);
    let no = Did::blocked("it will not open yet");
    ap.shelved.note(locked, &no, t0);
    ap.shelved.note(rotted, &no, t0);
    // Half a minute on, what `autoplay_loot` does before it chooses:
    // one body still lies there and the other has gone.
    let again = t0 + s(31);
    ap.forget_corpses_gone(|g| g == locked);
    assert_eq!(ap.shelved.len(), 1, "the rotted body is still remembered");
    assert!(
        ap.corpse_waiting(locked, again, Room::PLENTY),
        "never tried again"
    );
    // Chosen, and it says no again: a minute this time.
    ap.shelved.note(locked, &no, again);
    assert!(
        ap.shelved.held(&locked, again + s(59)),
        "back after thirty seconds again"
    );
    assert!(!ap.shelved.held(&locked, again + s(60)));
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

#[test]
fn an_ask_at_a_corpse_the_server_let_go_comes_back_with_nothing() {
    // Blargerton walked back to bodies ACE had let go while he was out
    // of sight of them, and asked at them for the rest of the session.
    let asked = Instant::now();
    let ms = Duration::from_millis;
    let (underfoot, done) = (CORPSE_REACH / 2.0, Some((0, asked + ms(80))));
    // Not there: done, no error, and not a word.
    assert!(answered_with_nothing(underfoot, asked, done, None));
    // A word from before the ask was about something else.
    assert!(answered_with_nothing(underfoot, asked, done, Some(asked)));
    // Locked to whoever killed it, or open to someone else: the server
    // says so before it is done.
    assert!(!answered_with_nothing(
        underfoot,
        asked,
        done,
        Some(asked + ms(40))
    ));
    // Too busy is a refusal, not an absence.
    assert!(!answered_with_nothing(
        underfoot,
        asked,
        Some((0x1D, asked + ms(80))),
        None
    ));
    // No answer yet, or only one that came in on the tick the ask went
    // out, which was to an earlier ask.
    assert!(!answered_with_nothing(underfoot, asked, None, None));
    assert!(!answered_with_nothing(
        underfoot,
        asked,
        Some((0, asked)),
        None
    ));
    // From across the room the server walks the character over first,
    // and a walk it cannot finish ends just as quietly.
    assert!(!answered_with_nothing(
        CORPSE_REACH * 4.0,
        asked,
        done,
        None
    ));
}

#[test]
fn a_corpse_is_taken_for_gone_only_when_every_ask_at_it_came_back_with_nothing() {
    let t0 = Instant::now();
    let mut ap = Autoplay::default();
    // What `autoplay_loot` does as each ask's wait runs out: note how
    // it came back, then ask again until the tries are used up. What
    // is said after the last ask decides what becomes of the body.
    let ask_until_given_up = |ap: &mut Autoplay, guid: u32, quiet: &[bool]| {
        ap.take_up_corpse(guid, t0, LOOT_TIMEOUT);
        let mut gone = false;
        for (tries, q) in (0..).zip(quiet) {
            gone = ap.nothing_came_back(*q);
            ap.let_go_of_corpse();
            if tries < LOOT_TRIES {
                ap.corpse = Some((guid, t0, LOOT_TIMEOUT, tries + 1));
            }
        }
        gone
    };
    let every = [true; LOOT_TRIES as usize + 1];
    assert!(
        ask_until_given_up(&mut ap, 0x8000_4001, &every),
        "a body that is not there is set aside to be asked at again"
    );
    // One ask that said something, or had no answer at all, and the
    // body is only set aside, as before. Nor does the last body's
    // count run on into this one.
    let mut once = every;
    once[0] = false;
    assert!(
        !ask_until_given_up(&mut ap, 0x8000_4002, &once),
        "a body that answered once is forgotten"
    );
    // Nothing in hand, nothing to say.
    assert!(!ap.nothing_came_back(true));
}

#[test]
fn config_round_trips_through_json() {
    let mut c = Config {
        enabled: true,
        ..Config::default()
    };
    c.buffs.spells = vec!["Strength Self".into()];
    c.fight.avoid = vec!["Olthoi".into()];
    let text = serde_json::to_string(&c).unwrap();
    let back: Config = serde_json::from_str(&text).unwrap();
    assert_eq!(back, c);
    // Missing fields fall back to the defaults.
    let partial: Config = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
    assert!(partial.enabled);
    assert_eq!(partial.survive.heal_below, Survive::default().heal_below);
    assert_eq!(Doing::Fighting.label(), "fighting");
    // A settings file written when the heal spell was still a
    // setting keeps loading, rest and all: the heal is chosen by
    // the rules now, and a name left in the file is no reason to
    // throw a player's whole configuration away.
    let old: Config =
        serde_json::from_str(r#"{"survive":{"heal_spell":"Heal Self VI","heal_below":0.45}}"#)
            .unwrap();
    assert_eq!(old.survive.heal_below, 0.45);
    assert!(old.survive.use_kits);
    // And one written before critters were walked past walks past
    // them: a bool left out would otherwise read as off. The same
    // goes for walking past what stands on the road.
    let before: Config = serde_json::from_str(r#"{"fight":{"radius":30.0}}"#).unwrap();
    assert!(before.fight.skip_critters);
    assert!(before.fight.walk_past_on_the_way);
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

#[test]
fn a_busy_tick_is_not_an_empty_quiver() {
    // The one caller of `ready_ammo` reads a false as "no
    // ammunition" and goes off to fletch some, dropping out of
    // combat stance to do it. With the busy test on every wield, a
    // shot still unanswered or a spell in the air made every tick a
    // false one, and an archer with a full pack was sent to make
    // arrows it was already carrying.
    let mut c = character_of_level(no_data(), 20);
    const ARROWS: u32 = 0x8000_0201;
    let me = c.world.player_guid;
    c.world.objects.insert(
        ARROWS,
        ac_world::WorldObject {
            guid: ARROWS,
            name: "Arrow".into(),
            stack_size: 100,
            valid_locations: ac_world::equip::MISSILE_AMMO,
            container: me,
            ..Default::default()
        },
    );
    c.autoplay.wanted_ammo = Some(ARROWS);
    assert!(c.wielded_ammo().is_none(), "the slot is empty");

    // A spell in the air: the wield waits, and the quiver is still
    // not empty.
    let now = Instant::now();
    c.autoplay.cast_sent = Some(now);
    assert!(c.server_busy(now));
    assert!(c.ready_ammo(), "a busy tick read as an empty quiver");
    assert_eq!(c.autoplay.wield_asked, None, "nothing sent mid-cast");

    // The spell lands and the arrows go into the slot.
    c.autoplay.cast_sent = None;
    assert!(c.ready_ammo());
    assert_eq!(c.autoplay.wield_asked.map(|(g, _)| g), Some(ARROWS));
}

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
fn an_urgent_buff_waits_for_the_fight_only_when_it_costs_the_weapon() {
    // The urgent pass runs as a reflex, ahead of loot and ahead of
    // the fight, and ignored `out_of_combat_only` outright: a buff
    // with a minute still on it was enough to put the sword away
    // mid-swing. Under god mode that cost throughput; for a mortal
    // character it is a fight fought bare-handed.
    let (sword, wand) = (false, true);
    let mut cfg = Buffs {
        never_below: 60.0,
        top_up_within: 300.0,
        out_of_combat_only: true,
        ..Buffs::default()
    };
    assert_eq!(
        buff_within(&cfg, true, true, sword),
        0.0,
        "only what has lapsed"
    );
    // A buff that is not up at all reads as nought seconds left, so
    // it still goes back up in the middle of a fight. That is what
    // "never below" is for.
    assert!(0.0 <= buff_within(&cfg, true, true, sword));
    // With a wand already in hand the recast costs a cast and
    // nothing else, so `never_below` holds. Answering nought here
    // meant a mortal caster's protections were put back only after
    // they had lapsed -- the very window the setting names.
    assert_eq!(
        buff_within(&cfg, true, true, wand),
        60.0,
        "a free recast was still made to wait for the fight"
    );
    // Out of the fight the urgent pass is unchanged, and so is the
    // quiet one either way.
    assert_eq!(buff_within(&cfg, true, false, sword), 60.0);
    assert_eq!(buff_within(&cfg, false, true, sword), 300.0);
    // And a player who has not asked for the restraint keeps the
    // old behaviour: buffs go back up mid-fight.
    cfg.out_of_combat_only = false;
    assert_eq!(buff_within(&cfg, true, true, sword), 60.0);
}
