use super::*;
use crate::autoplay::growth::tests::need;
use crate::autoplay::LootAction;
use crate::testkit::{
    as_a_war_mage, at_cindrues_counter, component_in_the_pack, component_named, pea_in_the_pack,
    standing_at, tagged_by_the_profile, tapers_in_the_pack, the_rest_to_the_counter, vendor_beside,
    window_of, with_a_buy_list, with_a_pack, with_a_profile, word_rule,
};
use ac_world::equip;

#[test]
fn a_want_that_names_its_counter_is_only_filled_there() {
    // Fletching supplies come in levels and elements that one bowyer
    // carries and the next does not, so a player who says where a
    // line comes from means it. The field was editable and saved and
    // read by nothing, so every counter that name-matched would do.
    let mut named = need(NeedKind::Named("Acid Arrowhead".into()), 500);
    named.from = Some("Thimrin Woodsetter".into());
    assert!(named.may_buy_at("Thimrin Woodsetter"));
    assert!(
        named.may_buy_at("thimrin woodsetter"),
        "however it is typed"
    );
    assert!(!named.may_buy_at("Scildith Dyrson the Bowyer"));

    // A line that names nobody is bought wherever it is sold, which
    // is every line a character has unless it says otherwise.
    let anywhere = need(NeedKind::Component(691), 1000);
    assert!(anywhere.may_buy_at("Anyone At All"));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_casters_peas_are_loot_to_sell_and_not_stock_to_buy() {
    // The report: "autovendoring doesn't seem to sell at all", from
    // a caster with a spellbook and a wand. The peas it looted sit
    // in the component table beside the scarabs, and every carried
    // component went on the restock list at the taper count for
    // want of a burn rate: two peas became two wants for ninety-nine
    // more, the counter skipped them as what the character came to
    // buy, and a counter with peas on the shelf would have bought
    // them at markup. The character that passed a live proof knew
    // spells and wielded nothing, so this never opened for it.
    use ac_vendor::Act;
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 20);
    as_a_war_mage(&mut c);
    pea_in_the_pack(&mut c, 0x8000_0010, "Iron Pea", 8328, 2_500);
    pea_in_the_pack(&mut c, 0x8000_0011, "Lead Pea", 8329, 500);
    tapers_in_the_pack(&mut c, 0x8000_0012, 78);
    with_a_buy_list(&mut c, &[("Prismatic Taper", 100, 25)]);
    let cfg = c.autoplay.config.growth.clone();

    // The gate is open: the tapers are stock, scaled from the line.
    let needs = c.grow_needs(&cfg);
    let taper = needs
        .iter()
        .find(|n| n.kind == NeedKind::Component(20631))
        .expect("the tapers it is short of");
    assert_eq!((taper.have, taper.keep, taper.want), (78, 100, 22));
    // And a pea is not: no cast burns one.
    let pea = needs.iter().find(|n| n.name.contains("Pea"));
    assert!(pea.is_none(), "a pea is stock: {pea:?}");
    let wants = c.vendor_shortfall(&cfg);
    assert!(
        wants.iter().all(|w| w.wcid != 8328 && w.wcid != 8329),
        "a pea on the shopping list: {wants:?}"
    );
    assert!(wants.iter().any(|w| w.wcid == 20631), "{wants:?}");

    // At a counter that buys components, the peas go and the
    // tapers stay.
    let cindrue = 0x8000_0002;
    vendor_beside(
        &mut c,
        cindrue,
        "Archmage Cindrue",
        glam::Vec3::new(1.0, 0.0, 0.0),
    );
    let mut window = window_of(cindrue);
    window.item_types = item_type::SPELL_COMPONENTS;
    c.world.open_vendor = Some(window);
    let snap = c.vendor_snapshot(&cfg);
    let mut offered: Vec<u32> = snap
        .items
        .iter()
        .filter(|i| snap.offers(i))
        .map(|i| i.guid)
        .collect();
    offered.sort_unstable();
    assert_eq!(offered, [0x8000_0010, 0x8000_0011]);
    assert!(
        snap.items
            .iter()
            .any(|i| i.guid == 0x8000_0010 && i.to_sell()),
        "the ledger's word travels with the pea"
    );
    let next = ac_vendor::Run::new().step(&snap, Instant::now());
    assert_eq!(
        next.act,
        Some(Act::Sell {
            items: vec![0x8000_0010, 0x8000_0011]
        }),
        "{}",
        next.saying
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_scarab_the_player_said_to_sell_goes_though_its_own_spells_burn_it() {
    // The principle, in the player's words: follow the loot
    // profile. A rule that says "sell lead scarabs" is the player's
    // decision about every lead scarab the character picks up, and
    // it was being overruled three ways at once: the component
    // guard kept it from the counter because Flame Bolt burns it,
    // the restock list wanted more of it for the same reason, and
    // the counter skipped it as what the character came to buy.
    use ac_vendor::Act;
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 20);
    as_a_war_mage(&mut c);
    let scarab = component_named(&c, "Lead Scarab");
    tapers_in_the_pack(&mut c, 0x8000_0012, 78);
    component_in_the_pack(&mut c, 0x8000_0013, "Lead Scarab", scarab, 3);
    with_a_profile(
        &mut c,
        "sells-scarabs",
        vec![
            word_rule("lead scarabs to sell", "lead scarab", LootAction::Sell),
            word_rule("components", "taper", LootAction::Keep),
        ],
        &[("Prismatic Taper", 100, 25)],
    );
    assert_eq!(tagged_by_the_profile(&mut c, 0x8000_0013), LootAction::Sell);
    let cfg = c.autoplay.config.growth.clone();
    assert!(
        c.burns(&cfg).contains(&scarab),
        "the spells do burn it; that is the point"
    );

    // Not stock: leaving, so never a need and never a want. The
    // tapers, which the player keeps, still are.
    let needs = c.grow_needs(&cfg);
    assert!(
        !needs.iter().any(|n| n.kind == NeedKind::Component(scarab)),
        "a need for what is being sold: {needs:?}"
    );
    assert!(needs.iter().any(|n| n.kind == NeedKind::Component(20631)));
    let wants = c.vendor_shortfall(&cfg);
    assert!(wants.iter().all(|w| w.wcid != scarab), "{wants:?}");

    // Offered, and sold, at a counter that buys components.
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
#[ignore = "needs AC_DATA_DIR"]
fn a_kind_the_rules_would_sell_on_arrival_is_not_bought() {
    // Whether to buy more of a thing is a question about the kind,
    // and the profile answers it the way it will answer for the
    // purchase when it arrives. Under "the rest, to the counter"
    // a bought scarab is tagged to sell as it lands and sold on
    // the next trip, so it is not bought, however low the pack is
    // and whatever the spells burn. Under the starter's rules,
    // which keep components, it is.
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
    let cfg = c.autoplay.config.growth.clone();
    let needs = c.grow_needs(&cfg);
    assert!(
        !needs.iter().any(|n| n.kind == NeedKind::Component(scarab)),
        "bought to be sold: {needs:?}"
    );
    // The tapers are on the buy list, which is the player's word
    // that they are stock: still a need, whatever the rules say.
    assert!(needs.iter().any(|n| n.kind == NeedKind::Component(20631)));

    with_a_buy_list(&mut c, &[("Prismatic Taper", 100, 25)]);
    let needs = c.grow_needs(&cfg);
    let need = needs
        .iter()
        .find(|n| n.kind == NeedKind::Component(scarab))
        .unwrap_or_else(|| panic!("the scarabs it is short of: {needs:?}"));
    assert_eq!(need.have, 1);
    assert!(need.want > 0);
    assert!(c.vendor_shortfall(&cfg).iter().any(|w| w.wcid == scarab));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn tapers_taken_to_keep_are_neither_sold_nor_wanted() {
    // On the buy list and written down as kept: the counter is not
    // offered them, and a pack holding its full line has nothing
    // to buy. A Keep says "do not sell this"; it does not say "do
    // not buy more", so a short line is still filled -- the buy
    // list is the player's word too.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 20);
    as_a_war_mage(&mut c);
    tapers_in_the_pack(&mut c, 0x8000_0012, 100);
    with_a_profile(
        &mut c,
        "keeps-tapers",
        vec![
            word_rule("components", "taper", LootAction::Keep),
            the_rest_to_the_counter(),
        ],
        &[("Prismatic Taper", 100, 25)],
    );
    assert_eq!(tagged_by_the_profile(&mut c, 0x8000_0012), LootAction::Keep);
    let cfg = c.autoplay.config.growth.clone();

    let wants = c.vendor_shortfall(&cfg);
    assert!(wants.iter().all(|w| w.wcid != 20631), "{wants:?}");
    let (offered, next) = at_cindrues_counter(&mut c, &cfg);
    assert!(offered.is_empty(), "{offered:x?}");
    assert!(
        !matches!(next.act, Some(ac_vendor::Act::Sell { .. })),
        "{}",
        next.saying
    );

    // Short of the line, still kept, and still bought.
    c.world.objects.get_mut(&0x8000_0012).unwrap().stack_size = 78;
    let wants = c.vendor_shortfall(&cfg);
    let taper = wants
        .iter()
        .find(|w| w.wcid == 20631)
        .unwrap_or_else(|| panic!("the tapers it is short of: {wants:?}"));
    assert_eq!(taper.short, 22);
    let snap = c.vendor_snapshot(&cfg);
    assert!(snap.items.iter().filter(|i| snap.offers(i)).count() == 0);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_stack_on_its_way_out_does_not_hide_the_shortfall_of_the_one_that_stays() {
    // Seventy-eight tapers kept and five hundred tagged to sell,
    // against a line of a hundred. The five hundred are not stock:
    // the line is twenty-two short, and the party hears the same.
    // The restock list once dropped the whole line while any of
    // the kind was leaving, and the party's broadcast counted the
    // leaving stack as stock, so a mate handed tapers over while
    // the character's own list said it wanted none.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 20);
    as_a_war_mage(&mut c);
    tapers_in_the_pack(&mut c, 0x8000_0012, 78);
    tapers_in_the_pack(&mut c, 0x8000_0015, 500);
    with_a_profile(
        &mut c,
        "sells-the-rest",
        vec![the_rest_to_the_counter()],
        &[("Prismatic Taper", 100, 25)],
    );
    for (guid, word) in [
        (0x8000_0012, LootAction::Keep),
        (0x8000_0015, LootAction::Sell),
    ] {
        let stats = c.stats_of(guid).unwrap();
        c.autoplay.tag(&stats, word);
    }
    let cfg = c.autoplay.config.growth.clone();
    let needs = c.grow_needs(&cfg);
    let taper = needs
        .iter()
        .find(|n| n.kind == NeedKind::Component(20631))
        .unwrap_or_else(|| panic!("the tapers it is short of: {needs:?}"));
    assert_eq!((taper.have, taper.want), (78, 22));
    let line = needs
        .iter()
        .find(|n| n.kind == NeedKind::Named("Prismatic Taper".into()))
        .unwrap_or_else(|| panic!("the line it is short of: {needs:?}"));
    assert_eq!((line.have, line.want), (78, 22));
    // And the party is told the same.
    c.autoplay.config.team.enabled = true;
    c.autoplay_stock();
    assert_eq!(c.autoplay.wants, vec!["Prismatic Taper".to_string()]);
    // With the five hundred sold, nothing changes but the count.
    c.world.objects.remove(&0x8000_0015);
    c.autoplay.ledger.forget(0x8000_0015);
    let needs = c.grow_needs(&cfg);
    let taper = needs
        .iter()
        .find(|n| n.kind == NeedKind::Component(20631))
        .unwrap();
    assert_eq!((taper.have, taper.want), (78, 22));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_scarab_sold_at_the_counter_is_not_bought_straight_back() {
    // The round trip the restock list is there to avoid, in one
    // visit: the scarab the player said to sell goes over the
    // counter, its tag is forgotten with it, and the same counter
    // has scarabs on the shelf. Asking only "is any of this
    // leaving?" said no the moment it had gone, and the run bought
    // it back at markup for the arrival pass to tag to sell again.
    use ac_vendor::Act;
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 20);
    as_a_war_mage(&mut c);
    let scarab = component_named(&c, "Lead Scarab");
    tapers_in_the_pack(&mut c, 0x8000_0012, 100);
    component_in_the_pack(&mut c, 0x8000_0013, "Lead Scarab", scarab, 3);
    let me = c.world.player_guid.unwrap();
    c.world.objects.insert(
        0x8000_0016,
        ac_world::WorldObject {
            guid: 0x8000_0016,
            name: "Pyreal".into(),
            weenie_class_id: 273,
            item_type: item_type::MONEY,
            value: 5_000,
            stack_size: 5_000,
            max_stack_size: 25_000,
            container: Some(me),
            ..Default::default()
        },
    );
    with_a_profile(
        &mut c,
        "sells-scarabs",
        vec![
            word_rule("lead scarabs to sell", "lead scarab", LootAction::Sell),
            word_rule("components", "taper", LootAction::Keep),
        ],
        &[("Prismatic Taper", 100, 25)],
    );
    assert_eq!(tagged_by_the_profile(&mut c, 0x8000_0013), LootAction::Sell);
    let cfg = c.autoplay.config.growth.clone();

    let cindrue = 0x8000_0002;
    vendor_beside(
        &mut c,
        cindrue,
        "Archmage Cindrue",
        glam::Vec3::new(1.0, 0.0, 0.0),
    );
    let mut window = window_of(cindrue);
    window.item_types = item_type::SPELL_COMPONENTS;
    window.items.push(ac_world::object::VendorItem {
        guid: 0x9000_0001,
        stack: 100,
        desc: ac_world::object::WeenieDesc {
            name: "Lead Scarab".into(),
            weenie_class_id: scarab,
            item_type: item_type::SPELL_COMPONENTS,
            value: 5,
            ..Default::default()
        },
    });
    c.world.open_vendor = Some(window);
    let mut run = ac_vendor::Run::new();
    let snap = c.vendor_snapshot(&cfg);
    assert!(
        snap.wants.iter().all(|w| w.wcid != scarab),
        "{:?}",
        snap.wants
    );
    let next = run.step(&snap, Instant::now());
    assert_eq!(
        next.act,
        Some(Act::Sell {
            items: vec![0x8000_0013]
        }),
        "{}",
        next.saying
    );
    // Sold: the server takes it, and the ledger forgets it.
    c.world.objects.remove(&0x8000_0013);
    c.autoplay.ledger.forget(0x8000_0013);
    let snap = c.vendor_snapshot(&cfg);
    assert!(
        snap.wants.iter().all(|w| w.wcid != scarab),
        "wanted back the moment it was gone: {:?}",
        snap.wants
    );
    let next = run.step(&snap, Instant::now());
    assert!(
        !matches!(next.act, Some(Act::Buy { wcid, .. }) if wcid == scarab),
        "bought straight back: {} ({:?})",
        next.saying,
        next.act
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_casters_heal_is_stocked_for_though_no_bolt_shares_its_herb() {
    // Heal Self V burns an herb, a powder, a potion and a talisman
    // (7, 26, 41, 61 in the dat) that no bolt or buff burns, and a
    // caster without a Life focus needs every one of them. The
    // restock list once stocked for the buffs and the bolts alone,
    // so when the pack ran out the heal stopped casting and nobody
    // went to town for it.
    const HEAL_SELF_V: u32 = 1160;
    const HERB: u32 = 7;
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 20);
    as_a_war_mage(&mut c);
    tapers_in_the_pack(&mut c, 0x8000_0012, 100);
    with_a_buy_list(&mut c, &[("Prismatic Taper", 100, 25)]);
    let herb = c
        .assets
        .spell_component_ids()
        .unwrap()
        .component_wcid(HERB)
        .expect("the herb's weenie");
    let cfg = c.autoplay.config.growth.clone();
    let needs = c.grow_needs(&cfg);
    assert!(
        !needs.iter().any(|n| n.kind == NeedKind::Component(herb)),
        "no spell of a war mage's burns the herb: {needs:?}"
    );

    c.world.stats.spells.push(HEAL_SELF_V);
    assert!(c.spells_cast().contains(&HEAL_SELF_V));
    let needs = c.grow_needs(&cfg);
    let need = needs
        .iter()
        .find(|n| n.kind == NeedKind::Component(herb))
        .unwrap_or_else(|| panic!("the heal's herb: {needs:?}"));
    assert_eq!(need.have, 0);
    assert!(need.want > 0);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn arrows_the_player_said_to_sell_are_not_counted_as_the_launchers_stock() {
    // Three hundred arrows in the pack and a bow in hand. Tagged to
    // sell, they are loot in the ammunition slot, not stock: the
    // launcher is short by the whole of what it keeps, and a
    // forecast that counted the arrows as stock would set off to
    // town for what it was about to sell -- or not set off at all.
    use ac_world::fletching::ammo_type;
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 20);
    let me = c.world.player_guid.unwrap();
    c.world.objects.insert(
        0x8000_0030,
        ac_world::WorldObject {
            guid: 0x8000_0030,
            name: "Yumi".into(),
            item_type: item_type::MISSILE_WEAPON,
            ammo_type: ammo_type::ARROW,
            value: 500,
            wielder: Some(me),
            parent: Some(me),
            ..Default::default()
        },
    );
    c.world.objects.insert(
        0x8000_0031,
        ac_world::WorldObject {
            guid: 0x8000_0031,
            name: "Arrow".into(),
            weenie_class_id: 300,
            item_type: item_type::MISSILE_WEAPON,
            valid_locations: equip::MISSILE_AMMO,
            value: 300,
            stack_size: 300,
            max_stack_size: 1_000,
            container: Some(me),
            ..Default::default()
        },
    );
    let mut cfg = c.autoplay.config.growth.clone();
    cfg.ammo_keep = 250;
    let needs = c.grow_needs(&cfg);
    assert!(
        !needs
            .iter()
            .any(|n| n.kind == NeedKind::Ammo(ammo_type::ARROW)),
        "three hundred in the pack: {needs:?}"
    );
    let stats = c.stats_of(0x8000_0031).unwrap();
    c.autoplay.tag(&stats, LootAction::Sell);
    let needs = c.grow_needs(&cfg);
    let arrows = needs
        .iter()
        .find(|n| n.kind == NeedKind::Ammo(ammo_type::ARROW))
        .unwrap_or_else(|| panic!("the arrows it will be short of: {needs:?}"));
    assert_eq!((arrows.have, arrows.want), (0, 250));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_counter_is_asked_for_the_named_stock_the_trip_was_made_for() {
    // A character with no spell that burns tapers wants them by the buy list's name. The trip
    // planner found a counter by that name, the counter's list only knew components by weenie,
    // and a character walked to Archmage Cindrue again and again and bought none.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 20);
    with_a_buy_list(&mut c, &[("Prismatic Taper", 100, 25)]);
    let cfg = c.autoplay.config.growth.clone();
    let stats = c.item_stats();
    let taper = ac_vendor::counter::Ware {
        wcid: 20631,
        name: "Prismatic Taper".into(),
        price: 300,
        stock: None,
        burden: 1,
    };
    let wants = c.vendor_shortfall_at(&cfg, &stats, std::slice::from_ref(&taper));
    assert!(
        wants.iter().any(|w| w.wcid == 20631 && w.short == 100),
        "{wants:?}"
    );
    // A shelf without it: nothing is asked for.
    assert!(c.vendor_shortfall_at(&cfg, &stats, &[]).is_empty());
}
