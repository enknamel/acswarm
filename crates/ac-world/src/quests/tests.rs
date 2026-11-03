use super::*;

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
    assert_eq!((kill.giver_wcid, kill.place.cell), (31641, 0xDA55_001D));
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
fn a_red_rat_tail_is_one_quest_for_any_of_twelve_givers() {
    let rows: Vec<&Row> = rows_of("give:3684").collect();
    assert_eq!(rows.len(), 12);
    let yukara = row("give:3684", Kind::Item, "Yukara Miko the Collector");
    assert_eq!(yukara.place.cell, 0x2B11_0112);
    assert_eq!(yukara.hand, Hand::Give);
    assert_eq!((yukara.want.clone(), yukara.count), (Want::Item(3684), 1));
    assert_eq!(yukara.reward.xp_at(5_000_000), 1_000);
    assert_eq!(yukara.reward.share, Share::None);
    assert_eq!(yukara.gives, [(2470, 10)]);
    assert_eq!(yukara.repeat, Repeat::Any);
    let tail = yukara.item.as_ref().expect("the wanted item");
    assert_eq!(
        (tail.name.as_str(), tail.item_type, tail.value),
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
    assert_eq!(alcott.place.cell, 0xA9B4_0019);
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
fn every_need_is_stamped_and_every_ask_has_a_source() {
    assert!(all().len() > 1_500, "{} rows", all().len());
    for r in all() {
        for n in &r.needs {
            assert!(
                stamped_by(n).next().is_some(),
                "{}: nobody stamps {n}",
                r.id
            );
        }
        match r.kind {
            Kind::Kill => {
                assert!(
                    rows_of(&r.id).any(|s| s.kind == Kind::Start),
                    "{}: no start",
                    r.id
                );
                assert!(
                    sources_of(&r.want).next().is_some(),
                    "{}: no creature",
                    r.id
                );
            }
            Kind::Item => {
                assert!(sources_of(&r.want).next().is_some(), "{}: no source", r.id);
                assert!(r.item.is_some(), "{}: no item", r.id);
            }
            Kind::Talk | Kind::Start => assert_eq!(r.want, Want::Nothing),
        }
    }
}

#[test]
fn every_line_of_both_files_parses_and_givers_stand_in_the_world() {
    assert_eq!(lines(QUESTS).count(), all().len());
    assert_eq!(lines(SOURCES).count(), sources().len());
    for r in all() {
        let (x, y) = (r.place.cell >> 24, (r.place.cell >> 16) & 0xFF);
        assert!(x < 0xFF && y < 0xFF, "{}: cell {:08X}", r.id, r.place.cell);
        let local = r.place.at - crate::landblock_origin(r.place.cell);
        assert!(
            r.place.is_indoors() || (0.0..=192.0).contains(&local.x),
            "{}",
            r.id
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
    // Rat King's Tail asks a flag no row stamps (QuestRestriction), so the quest is out.
    assert!(rows_of("ratcatcherquestcompleted").next().is_none());
    // Syltyn Rillon's bands, each read from the failure of the one before (sets 48828-48837).
    let bands: Vec<(u32, u32)> = rows_of("gotfrozenfuryaward")
        .map(|r| (r.min_level, r.max_level))
        .collect();
    assert_eq!(bands, [(1, 49), (50, 64), (65, 79), (80, 0)]);
    // Archmage Ichihiri's start talk asks her 20 h wait; the hand-in carries it.
    let blood = row("tuskerbloodcollectionwait", Kind::Item, "Archmage Ichihiri");
    assert_eq!(blood.repeat, Repeat::After(72_000));
}
