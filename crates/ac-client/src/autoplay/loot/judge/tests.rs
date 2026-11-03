use super::*;
use crate::items::ItemStats;
use crate::testkit::{a_party_profile, asks, item, judged, name_lists, shelf};

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
            when: Vec::new(),
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
    // Nor under the starter while its salvager does not salvage, but for its kits, kept only by
    // a character trained in Healing, and its quest items, kept by one whose quests want them.
    let mut starter = crate::profile::Profile::starter();
    starter.looting.salvage = false;
    assert!(
        starter.sends_to_the_best(),
        "kits go to whoever heals with them"
    );
    starter.rules.retain(|r| r.skill_asked().is_none());
    assert!(starter.sends_to_the_best(), "quest items go to the quester");
    starter
        .rules
        .retain(|r| !r.all.contains(&crate::profile::Ask::Quest));
    assert!(!starter.sends_to_the_best());
}

#[test]
fn a_worn_item_a_salvage_rule_claims_is_kept() {
    // "Never salvage any item that the player is wearing."
    use crate::profile::Verdict;
    let library = shelf(
        "worn salvage",
        vec![asks("salvage steel", "mat:steel", LootAction::Salvage)],
    );
    let steel = ItemStats {
        material: "Steel",
        ..item("Chainmail Hauberk", 50, 0)
    };
    assert_eq!(
        judged(&steel, &library, "test"),
        Verdict::Decided(LootAction::Salvage, "salvage steel".into())
    );
    let worn = ItemStats {
        wielded: true,
        ..steel
    };
    assert_eq!(
        judged(&worn, &library, "test"),
        Verdict::Decided(LootAction::Keep, super::WORN.into())
    );
    let _ = std::fs::remove_dir_all(library.dir());
}

/// A Red Rat Tail as quests.csv has it.
fn red_rat_tail() -> ac_world::quests::Item {
    ac_world::quests::rows_of("give:3684:2470")
        .find_map(|r| r.item.clone())
        .expect("the Collectors' row names its item")
}

fn profile_of(rules: Vec<crate::profile::Rule>) -> crate::profile::Profile {
    crate::profile::Profile {
        rules,
        ..Default::default()
    }
}

#[test]
fn the_cross_check_names_the_rule_above_the_quest_rule_that_would_lose_a_quest_item() {
    let cheap = asks("nothing cheap", "value<=250", LootAction::Skip);
    let quest = crate::profile::quest_rule();
    let me = crate::weapons::Wielder::default();
    let check =
        |p: &crate::profile::Profile| quest_item_blocked(Some(p), &red_rat_tail(), &me, "Bryn", 0);
    assert_eq!(check(&profile_of(vec![quest.clone(), cheap.clone()])), None);
    assert_eq!(
        check(&profile_of(vec![cheap.clone(), quest.clone()])),
        Some(("nothing cheap".into(), LootAction::Skip))
    );
    let sell = asks("tails to sell", "tail", LootAction::Sell);
    assert_eq!(
        check(&profile_of(vec![sell, quest.clone()])),
        Some(("tails to sell".into(), LootAction::Sell))
    );
    // A mule whose saved profile predates the rule: nothing claims a trophy worth 2.
    let mule = profile_of(vec![asks("the rest", "value>=1000", LootAction::Sell)]);
    assert_eq!(check(&mule), Some((NO_RULE_KEEPS.into(), LootAction::Skip)));
    let mut never = profile_of(vec![quest.clone()]);
    never.looting.never = vec!["Red Rat".into()];
    assert_eq!(
        check(&never),
        Some(("never take these".into(), LootAction::Skip))
    );
    // Any keeping rule above it keeps the tail just as well.
    let keep = asks("tails", "tail", LootAction::Keep);
    assert_eq!(check(&profile_of(vec![keep, cheap])), None);
    assert_eq!(
        quest_item_blocked(None, &red_rat_tail(), &me, "Bryn", 0),
        None
    );
}

#[test]
fn the_board_says_a_quest_wont_complete_and_names_the_rule() {
    use crate::quest_log::{quest_board, QuestFacts, QuestState};
    let rows: Vec<_> = ac_world::quests::rows_of("give:3684:2470")
        .cloned()
        .collect();
    let cheap = asks("nothing cheap", "value<=250", LootAction::Skip);
    let mut facts = QuestFacts {
        level: 20,
        config: crate::autoplay::Quests {
            on: true,
            off: Vec::new(),
            ..Default::default()
        },
        profile: Some(std::sync::Arc::new(profile_of(vec![
            cheap.clone(),
            crate::profile::quest_rule(),
        ]))),
        ..QuestFacts::default()
    };
    let state = |f: &QuestFacts| quest_board(&rows, f)[0].state.clone();
    assert_eq!(
        state(&facts),
        QuestState::WontComplete {
            rule: "nothing cheap".into(),
            does: LootAction::Skip
        }
    );
    // Still wanted while blocked, so the line can name the rule and the rule may be moved.
    assert_eq!(
        crate::quest_log::quest_wants(&quest_board(&rows, &facts)).len(),
        1
    );
    facts.profile = Some(std::sync::Arc::new(profile_of(vec![
        crate::profile::quest_rule(),
        cheap,
    ])));
    assert!(matches!(state(&facts), QuestState::Needs { more: 1, .. }));
}

#[test]
fn a_tail_on_a_body_is_called_to_the_one_whose_quest_wants_it() {
    let profile = crate::profile::Profile::starter();
    let tail = ItemStats {
        wcid: 3684,
        item_type: ac_world::item_type::MISC,
        kind: "misc",
        ..item("Red Rat Tail", 2, 0)
    };
    let idle = crate::weapons::Wielder::default();
    let questing = crate::weapons::Wielder {
        quest_wants: vec![ac_loot::QuestWant {
            wcid: 3684,
            need: None,
        }],
        ..Default::default()
    };
    let taker = |guid, name, sheet| Taker {
        guid,
        name,
        sheet,
        held: 0,
    };
    // The opener has no quest for it; the mate has, so the body's tail is the mate's.
    let takers = [taker(1, "Bryn01", &idle), taker(2, "Bryn02", &questing)];
    assert_eq!(called_to(&tail, None, &profile, &takers, None), Some(2));
    let both = [taker(1, "Bryn01", &questing), taker(2, "Bryn02", &questing)];
    assert_eq!(
        called_to(&tail, None, &profile, &both, None),
        Some(1),
        "ties by name"
    );
    let neither = [taker(1, "Bryn01", &idle), taker(2, "Bryn02", &idle)];
    assert_eq!(called_to(&tail, None, &profile, &neither, None), None);
    // What a mate says of itself carries its wants to the judging.
    let mate = crate::autoplay::Mate {
        quest_wants: questing.quest_wants.clone(),
        ..Default::default()
    };
    assert_eq!(mate.wielder().quest_wants, questing.quest_wants);
}
