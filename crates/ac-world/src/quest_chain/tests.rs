use super::*;
use crate::quests::{rows_of, Handing};

fn hunts(chain: &[ChainLink]) -> &[SourceGround] {
    chain
        .iter()
        .find_map(|l| match l {
            ChainLink::Hunt(g) => Some(g.as_slice()),
            _ => None,
        })
        .expect("a hunt link")
}

fn hand_ins(chain: &[ChainLink]) -> Vec<&'static Row> {
    chain
        .iter()
        .find_map(|l| match l {
            ChainLink::HandIn(rows) => Some(rows.clone()),
            _ => None,
        })
        .expect("a hand-in link")
}

/// Each link's kind in a word, in order, to read a chain's shape.
fn shape(chain: &[ChainLink]) -> Vec<&'static str> {
    chain
        .iter()
        .map(|l| match l {
            ChainLink::TakeUp(_) => "take up",
            ChainLink::Flag { .. } => "flag",
            ChainLink::Hunt(_) => "hunt",
            ChainLink::Fetch(_) => "fetch",
            ChainLink::HandIn(_) => "hand in",
        })
        .collect()
}

fn quest_stampers(by: &[FlagBy]) -> Vec<(&'static str, &'static str)> {
    by.iter()
        .filter_map(|b| match b {
            FlagBy::Quest(r) => Some((r.id, r.giver)),
            FlagBy::Unlock(_) => None,
        })
        .collect()
}

fn card(id: &str) -> &'static QuestCard {
    quest_cards()
        .iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("no card for {id}"))
}

#[test]
fn the_engorged_jaw_is_looted_in_the_remote_empyrean_vault() {
    let grounds = source_grounds(&Want::Item(28725));
    let best = &grounds[0];
    assert_eq!((best.landblock, best.entries), (0x0001, 4));
    assert_eq!(
        (best.source.wcid, best.source.name.as_str()),
        (28849, "Engorged Eater")
    );
    let place = best.place.expect("a journey's end");
    assert_eq!(place.cell >> 16, 0x0001, "lands in the vault");
    assert_eq!(place_name(place.cell), "Remote Empyrean Vault");
}

#[test]
fn the_vault_has_a_way_in_at_level_40_with_no_flag() {
    let ways = ways_in(0x0001);
    let vault = ways
        .iter()
        .find(|p| p.name == "Remote Empyrean Vault")
        .expect("the vault's portal");
    assert_eq!((vault.min_level, vault.quest.as_str()), (40, ""));
    assert!(ways.windows(2).all(|w| w[0].min_level <= w[1].min_level));
    let way_in = source_grounds(&Want::Item(28725))[0]
        .way_in
        .expect("by portal");
    assert_eq!(way_in.name, "Remote Empyrean Vault");
}

#[test]
fn a_warden_flag_names_its_warden() {
    let by = flag_givers("FacHubHuntersLeapPortal_Flag");
    let FlagBy::Unlock(u) = by[0] else {
        panic!("an unlock NPC first: {by:?}");
    };
    assert_eq!((u.npc.as_str(), u.min_level), ("Warden", 20));
}

#[test]
fn sir_ginazio_is_known_to_give_the_eaterpit_flag() {
    // The Eater Pit portal asks "Eaterpit"; the data stamps it lowercased.
    let pit = crate::portals::named("Eater Pit")
        .into_iter()
        .find(|p| !p.quest.is_empty())
        .expect("the Eater Pit portal");
    let by = flag_givers(&pit.quest);
    assert!(
        quest_stampers(&by).contains(&("ginsword", "Sir Ginazio")),
        "{by:?}"
    );
    let talk = by
        .iter()
        .find_map(|b| match b {
            FlagBy::Quest(r) if r.kind == Kind::Talk => Some(*r),
            _ => None,
        })
        .expect("a talk stamps it");
    assert_eq!(place_name(talk.places[0].cell), "near Silyun");
}

#[test]
fn a_flag_nobody_stamps_has_no_givers() {
    assert!(flag_givers("no such flag anywhere").is_empty());
}

#[test]
fn clemenza_stands_in_westwatch() {
    // The gazetteer's Westwatch pin is 12 map units off; the town's portal lands here.
    assert_eq!(place_name(0x23DA_002B), "Westwatch");
    let jaw = card("engorgedeaterjaw");
    assert_eq!(
        jaw.givers,
        [("Clemenza".to_string(), "Westwatch".to_string())]
    );
}

#[test]
fn a_dungeon_cell_is_named_by_its_portal() {
    assert_eq!(place_name(0x0001_02C6), "Remote Empyrean Vault");
    assert_eq!(place_name(0x0096_0321), "Insatiable Vault");
    // Only another dungeon's portal leads in.
    assert_eq!(place_name(0x0056_0459), "Augmentation Realm Main Level");
}

#[test]
fn a_spot_far_from_any_town_is_its_map_coordinates() {
    assert_eq!(place_name(0x0101_0001), "101.1S, 101.1W");
}

#[test]
fn the_jaw_chain_is_hunt_then_hand_in() {
    let chain = chain_of("engorgedeaterjaw");
    assert_eq!(shape(&chain), ["hunt", "hand in"]);
    assert_eq!(hunts(&chain)[0].landblock, 0x0001);
    let rows = hand_ins(&chain);
    assert_eq!(rows[0].giver, "Clemenza");
    assert_eq!(rows[0].hand, Handing::Give);
}

#[test]
fn a_kill_task_chain_starts_with_its_take_up() {
    let chain = chain_of("repugeaterkillcount");
    assert_eq!(shape(&chain), ["take up", "hunt", "hand in"], "{chain:?}");
    let ChainLink::TakeUp(start) = chain[0] else {
        unreachable!()
    };
    assert_eq!((start.kind, start.giver), (Kind::Start, "Xun Yu"));
    assert!(hunts(&chain).iter().all(|g| g.source.kind == Yield::Kill));
    assert_eq!(hand_ins(&chain)[0].kind, Kind::Kill);
}

#[test]
fn a_quest_needing_another_names_its_stamper() {
    let chain = chain_of("chdsrepeat0608");
    let ChainLink::Flag { flag, by, portal } = &chain[0] else {
        panic!("a flag first: {chain:?}");
    };
    assert_eq!((flag.as_str(), *portal), ("chdscompletedonce0608", None));
    assert!(quest_stampers(by)
        .iter()
        .any(|(id, _)| *id == "chdscompletedonce0608"));
}

#[test]
fn the_augmentation_access_chain_hunts_the_insatiable_eater_then_hands_fiun_the_jaw() {
    let chain = chain_of("augmentationprovinggroundaccess");
    assert_eq!(shape(&chain), ["hunt", "hand in"], "{chain:?}");
    let best = &hunts(&chain)[0];
    assert_eq!(
        (best.landblock, best.source.wcid, best.source.name.as_str()),
        (0x0096, 28848, "Insatiable Eater")
    );
    assert_eq!(best.entries, 4);
    let way_in = best.way_in.expect("the vault's portal");
    assert_eq!((way_in.min_level, way_in.quest.as_str()), (125, ""));
    let fiun = hand_ins(&chain)[0];
    assert_eq!(
        (fiun.giver, &fiun.want),
        ("Fiun Rehlyun", &Want::Item(28719))
    );
    assert_eq!(fiun.stamps, ["augmentationprovinggroundaccess"]);
    assert_eq!(place_name(fiun.places[0].cell), "Fiun Outpost");
}

#[test]
fn the_augmentation_realm_portal_flag_names_fiuns_quest_as_its_stamper() {
    let realm = crate::portals::named("Augmentation Realm Main Level");
    let flag = &realm[0].quest;
    assert_eq!(flag, "AugmentationProvingGroundAccess");
    let by = flag_givers(flag);
    assert!(
        quest_stampers(&by).contains(&("augmentationprovinggroundaccess", "Fiun Rehlyun")),
        "{by:?}"
    );
}

#[test]
fn a_blank_augmentation_gem_is_hunted_behind_fiuns_flag_and_given_to_a_trainer() {
    let chain = chain_of("give:29295:29269");
    assert_eq!(shape(&chain), ["flag", "hunt", "hand in"], "{chain:?}");
    let ChainLink::Flag { flag, by, portal } = &chain[0] else {
        unreachable!()
    };
    assert!(flag.eq_ignore_ascii_case("augmentationprovinggroundaccess"));
    assert!(portal.is_some_and(|p| p.to_cell >> 16 == 0x0056));
    assert!(quest_stampers(by)
        .iter()
        .any(|(_, giver)| *giver == "Fiun Rehlyun"));
    assert_eq!(hunts(&chain)[0].source.name, "Sir Bellas");
    let trainers = hand_ins(&chain);
    assert!(trainers.iter().all(|r| r.giver == "Piersanti Linante"));
    assert!(trainers.iter().all(|r| r.want == Want::Item(29295)));
    let gem = card("give:29295:29269");
    assert_eq!(gem.title, "Blank Augmentation Gem for Critical Protection");
    assert!(gem.flags.iter().any(|f| f.eq_ignore_ascii_case(flag)));
}

#[test]
fn every_quest_has_one_card_and_internal_titles_read_as_the_item() {
    let cards = quest_cards();
    let ids: BTreeSet<&str> = crate::quests::all().iter().map(|r| r.id).collect();
    assert_eq!(cards.len(), ids.len());
    assert!(cards.len() > 5_000, "every quest, XP or not");
    assert!(cards.iter().map(|c| c.id.as_str()).eq(ids.iter().copied()));
    assert_eq!(
        card("upperinsatiablejaw").title,
        "Upper Insatiable Eater Jaw"
    );
    assert_eq!(
        card("burunliberator_wait").title,
        "Link of a Falatacot Chain"
    );
    assert_eq!(
        card("blackmarketadvancementwait").title,
        "Trade Note (250,000)"
    );
    assert!(cards
        .iter()
        .all(|c| !c.title.is_empty() && c.title == c.title.trim()));
    let fiun = card("augmentationprovinggroundaccess");
    assert_eq!(fiun.item.as_deref(), Some("Insatiable Eater Jaw"));
    assert_eq!((fiun.xp, fiun.entry), (0, 125));
    assert!(fiun.does.iter().any(|d| d == "flag"));
    assert!(fiun.places.iter().any(|p| p == "Insatiable Vault"));
    assert_eq!(
        card("engorgedeaterjaw").creatures,
        [("Engorged Eater".to_string(), 80, 1.0)]
    );
    let ginazio = card("ginsword");
    assert!(ginazio.unread.iter().any(|u| u == "another quest's wait"));
}

#[test]
fn trade_notes_are_named_from_the_shops() {
    assert_eq!(reward_name(20630), Some("Trade Note (250,000)"));
    assert!(card("engorgedeaterjaw")
        .gives
        .iter()
        .all(|g| g.starts_with("Trade Note (")));
    assert_eq!(rows_of("engorgedeaterjaw").count(), 1);
}

#[test]
fn a_card_lists_every_item_its_quest_takes() {
    // aunkiller's givers take any of 35 items: the item filter sees each, not the first alone.
    let aun = card("aunkiller");
    assert_eq!(aun.items.len(), 35, "{:?}", aun.items);
    assert!(aun.items.iter().any(|i| i == "Aun Pendant"));
    assert_eq!(aun.item.as_ref(), aun.items.first());
}
