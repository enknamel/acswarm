//! The fastest way (`fastest`): every way on one scale, the journey and repeat tries counted.

use super::*;

/// An item no quest gives, so only the sources a test writes out come by it.
const NOWHERE: u32 = 0x7FFF_0001;

/// `want` from a placed source of `kind` at `at`, a try yielding at `chance`.
fn placed(want: u32, kind: Yield, wcid: u32, chance: f32, at: Place) -> &'static Source {
    leak(Source {
        want: Want::Item(want),
        kind,
        wcid,
        name: format!("source {wcid}"),
        level: 0,
        chance,
        landblocks: vec![(at.cell, 1)],
        places: vec![at],
        keys: Vec::new(),
        rates: Vec::new(),
        event: None,
        needs: Vec::new(),
        respawn: None,
        band: None,
        drain: None,
    })
}

/// `item` dropped at `chance` by a creature in the dungeon 0x0001 behind an open portal.
fn hunted_at(fake: &mut Fake, item: u32, chance: f32) {
    let creature = leak(Source {
        chance,
        ..dropper(Want::Item(item), 0x7FFF_1000, "Dropper", 50).clone()
    });
    let portal = way_in("Dropper Vault", 0x0001, 0, "");
    fake.grounds
        .push((Want::Item(item), ground(0x0001, creature, portal)));
}

/// `obtain` of `more` of `item` by `f`'s character carrying `coin`, with `fake`'s sources.
fn obtained_with(
    f: &QuestFacts,
    fake: &dyn Chains,
    item: u32,
    more: u32,
    coin: u32,
) -> PursuitStep {
    let carried = |w| f.carried.get(&w).copied().unwrap_or(0);
    let asking = Asking {
        level: f.level,
        held: &f.held,
        carried: &carried,
        refused: &|_, _| false,
        away: &|_| 0.0,
    };
    let held = BTreeMap::new();
    let arena = arena::ArenaView::default();
    let ask = Ask {
        facts: f,
        asking: &asking,
        tried: &|_: &Unlock| false,
        held: &held,
        led: None,
        chains: fake,
        dropped: &|_| false,
        unlocked: &|_| false,
        ways: &BTreeMap::new(),
        gem_grounds: &Default::default(),
        flag_ways: &BTreeMap::new(),
        event_ways: &BTreeMap::new(),
        portal_ways: &BTreeMap::new(),
        here: None,
        bound: None,
        kill_secs: &|_| Some(20.0),
        event_on: &|_| false,
        arena: &arena,
        journey: &walked(f.me),
        spendable: coin,
        chosen: Default::default(),
    };
    ask.obtain(&[Want::Item(item)], more, "the item", 0).0
}

/// [`obtained_with`] with coin enough for anything.
fn obtained(f: &QuestFacts, fake: &dyn Chains, item: u32, more: u32) -> PursuitStep {
    obtained_with(f, fake, item, more, u32::MAX)
}

/// A spot in Holtburg, and one `metres` east of it.
fn holtburg(metres: f32) -> Place {
    let at = ac_world::landblock_origin(0xA9B4_0000) + Vec3::new(84.0 + metres, 7.1, 94.0);
    Place {
        at,
        cell: 0xA9B4_0011,
    }
}

#[test]
fn a_rare_chest_beside_is_slower_than_a_thirty_percent_drop() {
    // A 0.2% chest 20 m off is 500 tries, each after its two-minute refill: the hunt's 30% drop
    // comes first even with the walk to its dungeon.
    let mut f = facts(120, &[]);
    f.me = Some(holtburg(0.0).at);
    let mut fake = Fake::default();
    let chest = placed(
        NOWHERE,
        Yield::Container,
        0x7FFF_2000,
        0.002,
        holtburg(20.0),
    );
    fake.sources.push(chest);
    hunted_at(&mut fake, NOWHERE, 0.3);
    let step = obtained(&f, &fake, NOWHERE, 1);
    assert!(
        matches!(&step, PursuitStep::Hunt { creature, .. } if creature == "Dropper"),
        "{step:?}"
    );
    // A sure chest beside is the way.
    let mut fake = Fake::default();
    let chest = placed(NOWHERE, Yield::Container, 0x7FFF_2000, 1.0, holtburg(20.0));
    fake.sources.push(chest);
    hunted_at(&mut fake, NOWHERE, 0.3);
    let step = obtained(&f, &fake, NOWHERE, 1);
    assert!(matches!(step, PursuitStep::OpenChest { .. }), "{step:?}");
}

#[test]
fn a_counter_sells_it_sooner_than_a_hunt() {
    let mut f = facts(120, &[]);
    f.me = Some(holtburg(0.0).at);
    let mut fake = Fake::default();
    hunted_at(&mut fake, NOWHERE, 0.3);
    let shop = placed(NOWHERE, Yield::Vendor, 0x7FFF_3000, 1.0, holtburg(500.0));
    fake.sources.push(shop);
    let step = obtained(&f, &fake, NOWHERE, 5);
    assert!(
        matches!(
            step,
            PursuitStep::Buy {
                wcid: NOWHERE,
                more: 5,
                ..
            }
        ),
        "{step:?}"
    );
    // A Mortar and Pestle at a real counter: bought with the coin for it, hunted without.
    const MORTAR: u32 = 4751;
    let mut fake = Fake::default();
    hunted_at(&mut fake, MORTAR, 0.3);
    let rich = obtained_with(&f, &fake, MORTAR, 5, u32::MAX);
    assert!(matches!(rich, PursuitStep::Buy { .. }), "{rich:?}");
    let poor = obtained_with(&f, &fake, MORTAR, 5, 0);
    assert!(matches!(poor, PursuitStep::Hunt { .. }), "{poor:?}");
}

#[test]
fn a_carried_hand_in_item_is_handed_in_outright() {
    // A Shadow Cursed Totem carried: an Agent of the Arcanum gives a Writ of Refuge for it.
    let mut f = facts(120, &[]);
    f.me = Some(holtburg(0.0).at);
    f.carried.insert(30810, 1);
    let step = obtained(&f, &ChainData, 11710, 10);
    // To the Agent in Holtburg, not the data's first, in dungeon 018A (regression: live 04:43).
    match step {
        PursuitStep::HandIn { row, at } => {
            assert_eq!(row.want, Want::Item(30810));
            assert_eq!(at.cell >> 16, 0xA9B4, "{at:?}");
        }
        other => panic!("not handed in: {other:?}"),
    }
}

#[test]
fn a_talk_behind_a_task_is_no_sure_ask() {
    // Aun Teriona gives a Writ of Refuge only once a liver task is done (her talks need a
    // diseased...liver_completed flag): never a sure ask beside a Writ's hand-in chains.
    let mut f = facts(120, &[]);
    let aun = ac_world::quests::sources_of(&Want::Item(11710))
        .find(|s| s.wcid == 34253)
        .expect("Aun Teriona gives a Writ");
    f.me = aun.places.first().map(|p| p.at);
    let step = obtained(&f, &ChainData, 11710, 10);
    assert!(
        !matches!(step, PursuitStep::AskFor { source, .. } if source.wcid == 34253),
        "{step:?}"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_writ_of_refuge_goal_is_had_the_fastest_way_from_holtburg() {
    // Blargerton's live case: ten Writs of Refuge, a level 120 war mage at Holtburg's lifestone.
    let mut c = mage_pursuing("give:24176", "writ-goal");
    c.autoplay.config.quests.pursue.clear();
    c.world.stats.level = 120;
    crate::testkit::stand(&mut c, 0xA9B4_0019, Vec3::new(84.0, 7.1, 94.0));
    c.autoplay.config.quests.pursue_item(11710, 10, 0);
    let now = Instant::now();
    let step = settled(&mut c, now);
    assert!(
        !matches!(&step, PursuitStep::OpenChest { chest, .. } if chest.name == "Black Marrow Reliquary"),
        "a 0.2% reliquary chosen: {step:?}"
    );
    assert!(step.can_progress(), "{step:?}");
    let said = c.autoplay.growth.fastest_said.get("item:11710");
    let said = said.expect("the choice logged once settled");
    assert!(said.best.1 < 86_400.0, "{}", said.words());
    eprintln!("Writ of Refuge from Holtburg: {}", said.words());
    // A Shadow Cursed Totem carried is handed in for its Writ.
    in_the_pack(&mut c, 0x8000_0901, 30810, "Shadow Cursed Totem", 1);
    match settled(&mut c, now) {
        PursuitStep::HandIn { row, at } => {
            assert_eq!(row.want, Want::Item(30810));
            assert_eq!(at.cell >> 16, 0xA9B4, "{at:?}");
        }
        other => panic!("not handed in: {other:?}"),
    }
}
