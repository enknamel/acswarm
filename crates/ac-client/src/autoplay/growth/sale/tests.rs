use std::time::Instant;

use super::*;
use crate::autoplay::tidy::Unpoured;
use crate::testkit::{
    as_a_war_mage, at_cindrues_counter, component_in_the_pack, component_named, salable,
    standing_at, tagged_by_the_profile, tapers_in_the_pack, the_rest_to_the_counter, with_a_pack,
    with_a_profile, word_rule,
};

#[test]
fn a_stack_worth_more_than_the_counter_allows_is_sold_in_pieces() {
    // A hundred Pyreal Peas: fifty thousand each and five million
    // the stack, and the server reckons a stack's value as the lot.
    // A counter that will not look at anything over a million
    // refuses all hundred -- so it takes twenty at a time.
    let peas = Salable {
        guid: 1,
        item_type: item_type::SPELL_COMPONENTS,
        value: 5_000_000,
        stack: 100,
    };
    assert_eq!(peas.each(), 50_000);
    assert_eq!(peas.at_once(1_000_000), 20);
    assert!(
        peas.taken_by(item_type::SPELL_COMPONENTS, 0, 1_000_000),
        "in pieces, but taken"
    );

    // A counter with no ceiling takes the lot in one go.
    assert_eq!(peas.at_once(0), 100);

    // One too dear even singly is not taken at all, and saying so
    // is better than splitting a stack down to nothing.
    let jewel = Salable {
        guid: 2,
        item_type: item_type::JEWELRY,
        value: 4_000_000,
        stack: 1,
    };
    assert_eq!(jewel.at_once(1_000_000), 0);
    assert!(!jewel.taken_by(item_type::JEWELRY, 0, 1_000_000));

    // And the counter's floor is about one of them, not the pile:
    // a stack of cheap things is not made sellable by being big.
    let chaff = Salable {
        guid: 3,
        item_type: item_type::MISC,
        value: 10_000,
        stack: 1_000,
    };
    assert_eq!(chaff.each(), 10);
    assert!(
        !chaff.taken_by(item_type::MISC, 100, 0),
        "ten is under the floor"
    );
}

#[test]
fn loot_for_a_counter_is_reason_enough_for_a_run() {
    // An Iron Pea and a Lead Pea, taken to sell, in a pack with room
    // to spare. The ledger said what they were for and nothing read
    // it: a run was made for a full pack, a heavy one or a supply
    // short, and the peas were carried about for ever.
    use ac_world::item_type::{ARMOR, GEM, SPELL_COMPONENTS};
    let cfg = Growth::default();
    let minutes = |m: u64| Some(Duration::from_secs(m * 60));
    let two_peas = [
        salable(1, SPELL_COMPONENTS, 2_500, 1),
        salable(2, SPELL_COMPONENTS, 500, 1),
    ];
    // Three thousand at face is not worth the walk on its own...
    assert_eq!(worth_a_sale_run(&two_peas, None, &cfg), None);
    assert_eq!(worth_a_sale_run(&two_peas, minutes(5), &cfg), None);
    // ...but it is not carried about all afternoon either.
    let why = worth_a_sale_run(&two_peas, minutes(15), &cfg).expect("a quarter of an hour");
    assert!(why.contains("carried 15 min"), "{why}");
    // Worth enough is reason at once, and a stack is worth the lot.
    let gem = [salable(3, GEM, 5_000, 1)];
    let why = worth_a_sale_run(&gem, None, &cfg).expect("five thousand");
    assert!(why.contains("5000 pyreals"), "{why}");
    let peas = [salable(4, SPELL_COMPONENTS, 10 * 500, 10)];
    assert!(worth_a_sale_run(&peas, None, &cfg).is_some());
    // So is an armful, however cheap: the slots are going.
    let junk: Vec<Salable> = (0..8).map(|i| salable(10 + i, ARMOR, 50, 1)).collect();
    let why = worth_a_sale_run(&junk, None, &cfg).expect("an armful");
    assert!(why.contains("8 things"), "{why}");
    assert_eq!(worth_a_sale_run(&junk[..7], None, &cfg), None);
    // The count is of stacks -- the slots going -- not of things: a
    // stack of eight cheap things is one, and seven singles and a
    // stack are eight.
    assert_eq!(
        worth_a_sale_run(&[salable(5, ARMOR, 400, 8)], None, &cfg),
        None
    );
    let mut seven_and_a_stack = junk[..7].to_vec();
    seven_and_a_stack.push(salable(20, ARMOR, 50, 10));
    assert!(worth_a_sale_run(&seven_and_a_stack, None, &cfg).is_some());
    // Nothing for a counter is no reason, however long since.
    assert_eq!(worth_a_sale_run(&[], minutes(60), &cfg), None);
    // Each rule is off at zero.
    let off = Growth {
        sell_run_value: 0,
        sell_run_count: 0,
        sell_run_patience: 0.0,
        ..cfg
    };
    assert_eq!(worth_a_sale_run(&junk, minutes(60), &off), None);
    assert_eq!(worth_a_sale_run(&gem, minutes(60), &off), None);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_scarab_nothing_was_decided_about_is_kept_by_the_guard() {
    // The guards still answer for what the profile did not decide.
    // A scarab with no entry in the ledger, burnt by the spells
    // this mage casts, stays out of the counter's hands: "the rest,
    // to the counter" would sell it today, and the guard says no.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 20);
    as_a_war_mage(&mut c);
    let scarab = component_named(&c, "Lead Scarab");
    tapers_in_the_pack(&mut c, 0x8000_0012, 78);
    component_in_the_pack(&mut c, 0x8000_0013, "Lead Scarab", scarab, 1);
    with_a_profile(
        &mut c,
        "sells-the-rest",
        vec![the_rest_to_the_counter()],
        &[("Prismatic Taper", 100, 25)],
    );
    assert_eq!(c.autoplay.ledger.by_guid(0x8000_0013), None);
    let cfg = c.autoplay.config.growth.clone();
    let (offered, next) = at_cindrues_counter(&mut c, &cfg);
    assert!(offered.is_empty(), "{offered:x?}");
    assert!(
        !matches!(next.act, Some(ac_vendor::Act::Sell { .. })),
        "{}",
        next.saying
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn what_the_server_will_not_take_stays_whatever_the_profile_said() {
    // The one word ahead of the profile's is the server's own. A
    // dagger in hand and a tinkered ring, both written down as
    // meant for a counter, are not offered: a sale the server will
    // not make is not a decision anybody gets to take.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 20);
    let me = c.world.player_guid.unwrap();
    c.world.objects.insert(
        0x8000_0020,
        ac_world::WorldObject {
            guid: 0x8000_0020,
            name: "Dagger".into(),
            weenie_class_id: 300,
            item_type: item_type::MELEE_WEAPON,
            value: 900,
            wielder: Some(me),
            parent: Some(me),
            ..Default::default()
        },
    );
    c.world.objects.insert(
        0x8000_0021,
        ac_world::WorldObject {
            guid: 0x8000_0021,
            name: "Ornate Ring".into(),
            weenie_class_id: 301,
            item_type: item_type::JEWELRY,
            value: 900,
            container: Some(me),
            ..Default::default()
        },
    );
    // Tinkered twice, by the server's appraisal (int 171).
    c.appraisals.insert(
        0x8000_0021,
        ac_net::messages::Appraisal {
            guid: 0x8000_0021,
            success: true,
            ints: vec![(171, 2)],
            ..Default::default()
        },
    );
    with_a_profile(
        &mut c,
        "sells-the-rest",
        vec![the_rest_to_the_counter()],
        &[],
    );
    for guid in [0x8000_0020, 0x8000_0021] {
        let stats = c.stats_of(guid).unwrap();
        c.autoplay.tag(&stats, LootAction::Sell);
    }
    let cfg = c.autoplay.config.growth.clone();
    assert!(c.for_sale(&cfg).is_empty());
    let (offered, next) = at_cindrues_counter(&mut c, &cfg);
    assert!(offered.is_empty(), "{offered:x?}");
    assert!(
        !matches!(next.act, Some(ac_vendor::Act::Sell { .. })),
        "{}",
        next.saying
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn what_was_taken_to_keep_is_not_swept_up_by_the_rest_to_the_counter() {
    // The decision is made once, when the item is taken. A ring
    // taken under "keep ornate rings" stays kept when the rules are
    // later just "the rest, to the counter": asking again at the
    // counter is how a thing taken to keep gets sold on the next
    // run to town.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 20);
    let me = c.world.player_guid.unwrap();
    c.world.objects.insert(
        0x8000_0021,
        ac_world::WorldObject {
            guid: 0x8000_0021,
            name: "Ornate Ring".into(),
            weenie_class_id: 301,
            item_type: item_type::JEWELRY,
            value: 900,
            container: Some(me),
            ..Default::default()
        },
    );
    with_a_profile(
        &mut c,
        "keeps-rings",
        vec![
            word_rule("ornate rings", "ornate", LootAction::Keep),
            the_rest_to_the_counter(),
        ],
        &[],
    );
    assert_eq!(tagged_by_the_profile(&mut c, 0x8000_0021), LootAction::Keep);
    // The rules change under it: today they would sell it.
    with_a_profile(
        &mut c,
        "sells-the-rest",
        vec![the_rest_to_the_counter()],
        &[],
    );
    let stats = c.stats_of(0x8000_0021).unwrap();
    let policy = c.sell_policy(&c.autoplay.config.growth.clone());
    assert!(
        policy.profile.as_ref().is_some_and(|p| matches!(
            p.judge(&stats, None, &policy.wielder, &policy.me, 0),
            crate::profile::Verdict::Decided(LootAction::Sell, _)
        )),
        "the rules as they read now would sell it"
    );
    let cfg = c.autoplay.config.growth.clone();
    assert!(c.for_sale(&cfg).is_empty(), "but it was taken to keep");
    let (offered, _) = at_cindrues_counter(&mut c, &cfg);
    assert!(offered.is_empty(), "{offered:x?}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_stack_to_sell_beside_one_to_keep_goes_whole_and_is_not_poured_into_it() {
    // Two stacks of scarabs, one word each: ten the player said to
    // sell and ninety-five they said to keep. The tidy that runs
    // before every sale used to pour the ten into the ninety-five,
    // the ledger settled the lot as kept, and the counter was
    // offered nothing. The ten go over the counter whole.
    use ac_vendor::Act;
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 20);
    as_a_war_mage(&mut c);
    let scarab = component_named(&c, "Lead Scarab");
    tapers_in_the_pack(&mut c, 0x8000_0012, 100);
    component_in_the_pack(&mut c, 0x8000_0013, "Lead Scarab", scarab, 10);
    component_in_the_pack(&mut c, 0x8000_0014, "Lead Scarab", scarab, 95);
    with_a_profile(
        &mut c,
        "keeps-some",
        vec![word_rule("components", "taper", LootAction::Keep)],
        &[("Prismatic Taper", 100, 25)],
    );
    for (guid, word) in [
        (0x8000_0013, LootAction::Sell),
        (0x8000_0014, LootAction::Keep),
    ] {
        let stats = c.stats_of(guid).unwrap();
        c.autoplay.tag(&stats, word);
    }
    let cfg = c.autoplay.config.growth.clone();
    // The client's own tidy leaves them apart too.
    assert!(
        matches!(c.pour_next(Instant::now()), Err(Unpoured::Tight)),
        "the tidy poured across words"
    );
    let (offered, next) = at_cindrues_counter(&mut c, &cfg);
    assert_eq!(offered, [0x8000_0013]);
    assert_eq!(
        next.act,
        Some(Act::Sell {
            items: vec![0x8000_0013]
        }),
        "{}",
        next.saying
    );
}

#[test]
fn a_patience_too_large_for_a_duration_turns_the_rule_off() {
    // The config is hand-edited JSON, and 1e20 is a finite f32 that
    // no Duration holds: it used to panic on the first frame a run
    // could start with anything for a counter in the pack.
    use ac_world::item_type::SPELL_COMPONENTS;
    let cfg = Growth {
        sell_run_patience: 1e20,
        ..Growth::default()
    };
    let pea = [salable(1, SPELL_COMPONENTS, 500, 1)];
    assert_eq!(
        worth_a_sale_run(&pea, Some(Duration::from_secs(60 * 60)), &cfg),
        None
    );
}
