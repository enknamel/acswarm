use super::*;

fn cells(r: &Row) -> Vec<u32> {
    r.places.iter().map(|p| p.cell).collect()
}

fn row(id: &str, kind: Kind, giver: &str) -> &'static Row {
    rows_of(id)
        .find(|r| r.kind == kind && r.giver == giver)
        .unwrap_or_else(|| panic!("{id}: no {kind:?} row from {giver}"))
}

#[test]
fn san_mings_blood_shreth_kill_task_loads_as_the_design_works_it() {
    let start = row("killtaskbloodshreth", Kind::Start, "San Ming");
    assert_eq!(start.stamps, ["killtaskbloodshreth"]);
    assert_eq!(start.reward.xp_at(1_000_000), 0);
    let kill = row("killtaskbloodshreth", Kind::Kill, "San Ming");
    assert_eq!(kill.title, "Blood Shreth Kill Task");
    assert_eq!((kill.giver_wcid, cells(kill)), (31641, vec![0xDA55_001D]));
    assert_eq!(kill.want, Want::Kill("killtaskbloodshreth".into()));
    assert_eq!(kill.count, 10);
    assert_eq!(kill.repeat, Repeat::After(72_000));
    assert_eq!(kill.timer, "killtaskbloodshrethwait");
    assert_eq!(kill.needs, ["killtaskbloodshreth"]);
    assert_eq!(kill.gives, [(31617, 1)]);
    assert_eq!(kill.reward.share, Share::Allegiance);
    // 1.33 of the XP to the next level, capped at 120,000.
    assert_eq!(kill.reward.xp_at(10_000), 13_300);
    assert_eq!(kill.reward.xp_at(1_000_000), 120_000);
    let shreth: Vec<&Source> = sources_of(&kill.want).collect();
    assert!(shreth
        .iter()
        .any(|s| s.wcid == 4110 && s.kind == Yield::Kill && s.level == 8));
    assert!(shreth[0].landblocks.contains(&(0x162, 17)));
}

#[test]
fn a_red_rat_tail_is_one_quest_per_reward_for_any_of_its_givers() {
    // Eight Collector weenies, one row each, in eleven places give Stamina Elixirs; Wa Shi Pyreals.
    let rows: Vec<&Row> = rows_of("give:3684:2470").collect();
    assert_eq!(rows.len(), 8);
    assert_eq!(rows.iter().map(|r| r.places.len()).sum::<usize>(), 11);
    assert!(rows
        .iter()
        .all(|r| r.title == "Red Rat Tail for Stamina Elixir"));
    assert_eq!(
        row("give:3684:273", Kind::Item, "Wa Shi").gives,
        [(273, 900)]
    );
    let yukara = row("give:3684:2470", Kind::Item, "Yukara Miko the Collector");
    assert_eq!(cells(yukara), [0x2B11_0112]);
    assert_eq!(yukara.hand, Handing::Give);
    assert_eq!((yukara.want.clone(), yukara.count), (Want::Item(3684), 1));
    assert_eq!(yukara.reward.xp_at(5_000_000), 1_000);
    assert_eq!(yukara.reward.share, Share::None);
    assert_eq!(yukara.gives, [(2470, 10)]);
    assert_eq!(yukara.repeat, Repeat::Any);
    let tail = yukara.item.as_ref().expect("the wanted item");
    assert_eq!(
        (tail.name, tail.item_type, tail.value),
        ("Red Rat Tail", 0x80, 2)
    );
    let rat = sources_of(&Want::Item(3684))
        .find(|s| s.wcid == 949)
        .expect("the Red Rat drops it");
    assert_eq!((rat.kind, rat.level, rat.chance), (Yield::Drop, 15, 0.15));
}

#[test]
fn holtburgs_barkeep_and_pathwarden_talks_chain() {
    let alcott = row("tothebarkeep", Kind::Talk, "Alcott");
    assert_eq!(cells(alcott), [0xA9B4_0019]);
    assert_eq!(alcott.reward.xp_at(0), 7_000);
    assert_eq!(alcott.repeat, Repeat::Once);
    assert_eq!(alcott.stamps, ["lifestonefound1111", "tothebarkeep"]);
    assert!(alcott.needs.is_empty());
    let buck = row("topathwarden", Kind::Talk, "Buckminster");
    assert_eq!(buck.title, "Sent to the Pathwarden");
    assert_eq!(buck.reward.xp_at(0), 9_300);
    assert_eq!(buck.gives, [(273, 500)]);
    // Buckminster pays with ToTheBarkeep held or not (sets 62063 and 62066), so it asks nothing.
    assert!(buck.needs.is_empty());
    let thorolf = row("pathwardenfound1111", Kind::Talk, "Pathwarden Thorolf");
    assert_eq!(thorolf.reward.xp_at(0), 12_500);
    assert_eq!(thorolf.needs, ["topathwarden"]);
    assert!(stamped_by("ToPathwarden").any(|r| r.giver == "Buckminster"));
}

#[test]
fn every_row_says_what_it_does_and_asks_its_own_kind_of_thing() {
    // Every quest is in, XP or not: a need nobody stamps or an item nothing drops stays a row
    // (the board says it can't yet), so only the row's own shape is checked.
    assert!(all().len() > 10_000, "{} rows", all().len());
    for r in all() {
        assert!(!r.does.is_empty(), "{}: does nothing", r.id);
        let fits = match r.kind {
            Kind::Kill => matches!(r.want, Want::Kill(_)),
            Kind::Item => matches!(r.want, Want::Item(_)),
            Kind::Talk | Kind::Start => r.want == Want::Nothing,
        };
        assert!(fits, "{}: {:?} asks {:?}", r.id, r.kind, r.want);
        assert_eq!(r.random, r.unread.contains(&"a random reply") && r.random);
    }
}

#[test]
fn sir_ginazio_stamps_the_eater_pit_flag() {
    let gin = stamped_by("Eaterpit")
        .find(|r| r.giver == "Sir Ginazio")
        .expect("Sir Ginazio stamps eaterpit");
    assert_eq!(
        (gin.id, gin.kind, gin.giver_wcid),
        ("ginsword", Kind::Talk, 29488)
    );
    assert_eq!(cells(gin), [0x2AEE_002C]);
    assert!(!gin.has_xp());
}

#[test]
fn fiun_rehlyun_takes_the_insatiable_jaw_for_augmentation_access() {
    let fiun = row(
        "augmentationprovinggroundaccess",
        Kind::Item,
        "Fiun Rehlyun",
    );
    assert_eq!(
        (fiun.want.clone(), fiun.hand),
        (Want::Item(28719), Handing::Give)
    );
    assert_eq!(fiun.stamps, ["augmentationprovinggroundaccess"]);
    assert!(!fiun.has_xp() && !fiun.has_unread());
    assert_eq!(fiun.does, ["flag", "takes"]);
    assert!(sources_of(&Want::Item(28719)).any(|s| s.wcid == 28848));
    // The Blank Augmentation Gem is one quest per trainer's gem, the player's choice, and one
    // for Nalicana's luminance on a check the client cannot read.
    let mut gems: Vec<&str> = all().iter().map(|r| r.id).collect();
    gems.retain(|id| id.starts_with("give:29295:"));
    gems.dedup();
    assert!(gems.len() > 20, "{} gems", gems.len());
    let piersanti = row("give:29295:29269", Kind::Item, "Piersanti Linante");
    assert_eq!(
        piersanti.title,
        "Blank Augmentation Gem for Critical Protection"
    );
    assert_eq!(rows_of("give:29295:29269").count(), 1);
    assert!(rows_of("give:29295:none").any(|r| r.does.contains(&"luminance") && r.has_unread()));
}

#[test]
fn a_random_reply_is_one_row_listing_every_reward() {
    let r = all()
        .iter()
        .find(|r| r.random && r.gives.len() > 3)
        .expect("a random reply with several rewards");
    assert!(r.unread.contains(&"a random reply"), "{}", r.id);
    assert!(!r.has_xp() || r.has_unread());
}

#[test]
fn every_line_of_both_files_parses_and_givers_stand_in_the_world() {
    assert_eq!(lines(QUESTS).count(), all().len());
    assert_eq!(lines(GIVERS).count(), givers().places.len());
    let from_file = sources().iter().filter(|s| s.kind != Yield::Vendor);
    assert_eq!(lines(SOURCES).count(), from_file.count());
    for s in sources().iter().filter(|s| !s.kind.is_hunted()) {
        assert_eq!(s.places.len(), s.landblocks.len(), "{} {}", s.wcid, s.name);
    }
    for r in all() {
        assert!(
            !r.places.is_empty() || r.hand == Handing::Read,
            "{}: nowhere",
            r.id
        );
    }
    for p in &givers().places {
        let (x, y) = (p.cell >> 24, (p.cell >> 16) & 0xFF);
        assert!(x < 0xFF && y < 0xFF, "cell {:08X}", p.cell);
        let local = p.at - crate::landblock_origin(p.cell);
        assert!(
            p.is_indoors() || (0.0..=192.0).contains(&local.x),
            "{:08X}",
            p.cell
        );
    }
}

#[test]
fn level_proportional_xp_rounds_half_to_even_as_dotnet_does() {
    let half = Reward {
        xp: 0,
        xp_level: 0.5,
        xp_min: 0,
        xp_max: 0,
        share: Share::Allegiance,
    };
    assert_eq!(half.xp_at(5), 2, "2.5 rounds to 2");
    assert_eq!(half.xp_at(7), 4, "3.5 rounds to 4");
    let floored = Reward {
        xp_min: 100,
        ..half
    };
    assert_eq!(floored.xp_at(10), 100);
}

#[test]
fn a_giver_in_a_quest_only_dungeon_is_locked_until_the_flag_is_held() {
    let shut = all().iter().find_map(|r| r.locked_by(&[]).map(|f| (r, f)));
    if let Some((r, flag)) = shut {
        assert_eq!(r.locked_by(&[flag.to_string()]), None, "{}", r.id);
    }
    assert_eq!(
        row("tothebarkeep", Kind::Talk, "Alcott").locked_by(&[]),
        None
    );
}

#[test]
fn an_items_own_pickup_limit_and_a_level_ladder_are_read_into_the_rows() {
    // Southern Tumerok Insignia: one looted a 20 h wait (Player_Inventory.cs:1249-1262).
    let ufet = rows_of("give:43419").next().expect("Ufet's insignia");
    assert_eq!(ufet.repeat, Repeat::After(72_000));
    assert_eq!(ufet.timer, "southerntumerokinsignia");
    // Carpenter Wasp Wing: CarpWing allows one pickup ever.
    assert!(rows_of("waspwingdone").all(|r| r.repeat == Repeat::Once));
    // Rat King's Tail asks the flag its pickup is restricted to (QuestRestriction).
    let tail = row("ratcatcherquestcompleted", Kind::Item, "Lo Raen");
    assert!(tail.needs.contains(&"ratcatcherquestacquired".to_string()));
    // Syltyn Rillon's bands, each read from the failure of the one before (sets 48828-48837).
    // Her talk that hands the sword over (no XP, another quest's wait unread) is a row too.
    let bands: Vec<(u32, u32)> = rows_of("gotfrozenfuryaward")
        .filter(|r| r.kind == Kind::Item)
        .map(|r| (r.min_level, r.max_level))
        .collect();
    assert_eq!(bands, [(1, 49), (50, 64), (65, 79), (80, 0)]);
    // Archmage Ichihiri's start talk asks her 20 h wait; the hand-in carries it.
    let blood = row("tuskerbloodcollectionwait", Kind::Item, "Archmage Ichihiri");
    assert_eq!(blood.repeat, Repeat::After(72_000));
}

#[test]
fn a_chests_guardians_spawn_nowhere() {
    // Arwic's Legendary Chests (0xC6A9) hold a generator put in them (where_Create Contain): the
    // creatures it lists come only for whoever opens one, so Arwic is no Blood Shreth spawn.
    assert!(crate::hunting::at(0xC6A9).is_none(), "a ground in Arwic");
    let shreth = Want::Kill("killtaskbloodshreth".into());
    let at: Vec<u32> = sources_of(&shreth)
        .flat_map(|s| s.landblocks.iter().map(|&(lb, _)| lb))
        .collect();
    assert!(!at.contains(&0xC6A9), "{at:x?}");
    assert!(at.contains(&0xCE4E), "{at:x?}");
}

#[test]
fn the_necklace_of_leerargh_comes_from_a_locked_chest_whose_key_a_scamp_drops() {
    // magicitemtraining3: Bleeargh takes the necklace (27902), which only chest 27890 holds
    // (its generator), locked with KeyMosswartNecklaceLeerargh; the Smelly Key (27896) opens it.
    let hand_in = rows_of("magicitemtraining3").find(|r| r.kind == Kind::Item);
    assert_eq!(hand_in.map(|r| &r.want), Some(&Want::Item(27902)));
    let from: Vec<&Source> = sources_of(&Want::Item(27902)).collect();
    assert_eq!(from.len(), 1, "{from:?}");
    let chest = from[0];
    assert_eq!((chest.kind, chest.wcid), (Yield::Container, 27890));
    assert_eq!(chest.landblocks, [(0x0177_0297, 1)]);
    let local = chest.places[0].at - crate::landblock_origin(0x0177_0297);
    assert!(
        local.distance(Vec3::new(20.37, -271.08, 24.0)) < 0.1,
        "{local}"
    );
    assert_eq!(chest.keys, [(27896, "A Smelly Key".to_string())]);
    let key: Vec<&Source> = sources_of(&Want::Item(27896)).collect();
    assert!(
        key.iter()
            .any(|s| s.kind == Yield::Drop && s.wcid == 27892 && s.landblocks == [(0x177, 1)]),
        "{key:?}"
    );
    let grounds = crate::quest_chain::source_grounds(&Want::Item(27896));
    assert!(
        grounds.first().is_some_and(|g| g.place.is_some()),
        "{grounds:?}"
    );
}

#[test]
fn the_imbued_focusing_stone_is_made_by_using_the_plain_one_on_the_vortex() {
    // Le-Ai Rea takes the imbued stone (8904); recipe 1838 makes it from the plain Focusing Stone
    // (8903) used on the Vortex (8907), placed once at 0x02AB010A; a Lich Overseer drops 8903.
    let hand_in = rows_of("give:8904").find(|r| r.kind == Kind::Item);
    assert_eq!(hand_in.map(|r| r.giver_wcid), Some(4122));
    let from: Vec<&Source> = sources_of(&Want::Item(8904)).collect();
    assert_eq!(from.len(), 1, "{from:?}");
    let vortex = from[0];
    assert_eq!((vortex.kind, vortex.wcid), (Yield::Use, 8907));
    assert_eq!(vortex.name, "Vortex");
    assert_eq!(vortex.landblocks, [(0x02AB_010A, 1)]);
    let local = vortex.places[0].at - crate::landblock_origin(0x02AB_010A);
    assert!(
        local.distance(Vec3::new(0.0, -180.0, -12.0)) < 0.1,
        "{local}"
    );
    assert_eq!(vortex.keys, [(8903, "Focusing Stone".to_string())]);
    let stone: Vec<&Source> = sources_of(&Want::Item(8903)).collect();
    assert!(
        stone
            .iter()
            .any(|s| s.kind == Yield::Drop && s.wcid == 4124 && s.landblocks == [(0x188, 1)]),
        "{stone:?}"
    );
}

#[test]
fn a_shop_selling_a_wanted_item_is_a_source_of_it() {
    let sold = sources()
        .iter()
        .find(|s| s.kind == Yield::Vendor)
        .expect("a shop source");
    let Want::Item(w) = sold.want else {
        panic!("{sold:?}")
    };
    let shop = crate::shops::all()
        .iter()
        .find(|s| s.wcid == sold.wcid)
        .expect("its shop");
    assert!(shop.stocks_wcid(w));
    assert_eq!(sold.places.len(), 1);
}

#[test]
fn the_stamped_rithwic_letter_is_made_by_using_the_town_stamp_on_a_lucky_gold_letter() {
    // Recipe 1777: the Rithwic Town Stamp (8738) used on a carried Lucky Gold Letter (45875).
    let from: Vec<&Source> = sources_of(&Want::Item(70307)).collect();
    let combine = from
        .iter()
        .find(|s| s.kind == Yield::Combine)
        .expect("no combine");
    let keys: Vec<u32> = combine.keys.iter().map(|k| k.0).collect();
    assert_eq!(keys, [8738, 45875], "the stamp used on the letter");
    assert!(combine.landblocks.is_empty());
}

#[test]
fn a_gift_punished_with_harm_is_no_quest_and_a_recall_scroll_is_one_to_read() {
    // The Healer's Heart handed to the temple Guardian is met with Soul Dissolution.
    assert_eq!(rows_of("give:8552").count(), 0);
    // What players go for is the scroll beside it in the Mistress' Chest.
    let learn: Vec<&Row> = rows_of("learn:2023").collect();
    assert_eq!(learn.len(), 1);
    let r = learn[0];
    assert_eq!(
        (r.hand, &r.want, r.teaches(), r.repeat),
        (Handing::Read, &Want::Item(8530), Some(2023), Repeat::Once)
    );
    assert!(r.places.is_empty() && r.giver.is_empty());
    let chest = sources_of(&Want::Item(8530)).next().expect("a source");
    assert_eq!(
        (chest.kind, chest.name.as_str()),
        (Yield::Container, "Mistress' Chest")
    );
    assert!(chest.keys.iter().any(|k| k.0 == 8511));
    assert!(
        sources_of(&Want::Item(8511)).any(|s| s.kind == Yield::Drop && s.name == "Putrid Moarsman")
    );
}

#[test]
fn a_portal_gem_comes_from_its_shop_or_its_drop() {
    // The invitation to Ithaenc is bought; the Copper Ward gem drops from Captain Ramelle.
    assert!(sources_of(&Want::Item(14915))
        .any(|s| s.kind == Yield::Vendor && s.name == "Wedding Planner"));
    assert!(sources_of(&Want::Item(29444))
        .any(|s| s.kind == Yield::Drop && s.name == "Captain Ramelle" && s.chance == 1.0));
}
