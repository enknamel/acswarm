use super::*;
use crate::autoplay::Doing;

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

#[test]
fn a_config_saved_before_quests_loads_with_them_off_and_quests_round_trip() {
    let old: Config = serde_json::from_str(r#"{"enabled":true,"fight":{"radius":30.0}}"#).unwrap();
    assert_eq!(old.quests, Quests::default());
    assert!(
        !old.quests.is_on("give:3684:2470"),
        "off until the player ticks it"
    );
    let mut c = Config::default();
    c.quests.on = true;
    c.quests.off = vec!["killtaskbloodshreth".into()];
    let back: Config = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
    assert_eq!(back.quests, c.quests);
    assert!(back.quests.is_on("give:3684:2470"));
    assert!(!back.quests.is_on("killtaskbloodshreth"));
}

#[test]
fn quests_saved_before_the_chosen_list_load_with_nothing_chosen() {
    let old: Config =
        serde_json::from_str(r#"{"quests":{"on":true,"off":["killtaskbloodshreth"]}}"#).unwrap();
    assert!(old.quests.chosen.is_empty());
    assert_eq!(old.quests.off, ["killtaskbloodshreth"]);
    assert!(old.quests.is_on("give:3684:2470"));
}

#[test]
fn a_quest_paying_xp_starts_on_and_one_paying_none_only_once_chosen() {
    let mut q = Quests {
        on: true,
        ..Quests::default()
    };
    assert!(q.is_on("killtaskbloodshreth"), "XP: on until switched off");
    // Fiun Rehlyun's jaw for the Augmentation Realm flag pays no XP.
    let fiun = "augmentationprovinggroundaccess";
    assert!(!q.is_on(fiun));
    q.switch_on(fiun);
    assert!(q.is_on(fiun) && q.is_chosen(fiun));
    q.switch_off(fiun);
    assert!(!q.is_on(fiun) && !q.is_chosen(fiun));
    q.switch_off("killtaskbloodshreth");
    assert!(!q.is_on("killtaskbloodshreth"));
    q.switch_on("killtaskbloodshreth");
    assert!(q.is_on("killtaskbloodshreth") && q.off == [fiun]);
}

#[test]
fn quests_saved_before_the_pursuit_list_load_with_nothing_pursued() {
    let old: Config =
        serde_json::from_str(r#"{"quests":{"on":true,"chosen":["ginsword"]}}"#).unwrap();
    assert!(old.quests.pursue.is_empty());
    assert_eq!(old.quests.chosen, ["ginsword"]);
}

#[test]
fn doing_a_quest_puts_it_first_and_whenever_ready_last() {
    let mut q = Quests::default();
    q.pursue_quest("engorgedeaterjaw", true, 10);
    q.pursue_quest("ginsword", false, 20);
    q.pursue_quest("augmentationprovinggroundaccess", false, 30);
    let ids = |q: &Quests| q.pursue.iter().map(|p| p.id.clone()).collect::<Vec<_>>();
    assert_eq!(
        ids(&q),
        [
            "augmentationprovinggroundaccess",
            "ginsword",
            "engorgedeaterjaw"
        ]
    );
    q.move_pursuit("engorgedeaterjaw", true);
    assert_eq!(ids(&q)[1], "engorgedeaterjaw");
    q.move_pursuit("augmentationprovinggroundaccess", true);
    assert_eq!(
        ids(&q)[0],
        "augmentationprovinggroundaccess",
        "first stays first"
    );
    // Asked again, it moves and its time is the new one.
    q.pursue_quest("ginsword", false, 40);
    assert_eq!((ids(&q)[0].as_str(), q.pursue[0].since), ("ginsword", 40));
    q.stop_pursuing("ginsword");
    assert!(!q.is_pursued("ginsword") && q.pursue.len() == 2);
}

#[test]
fn a_pursued_quest_has_every_way_worked_as_one_chosen() {
    let fiun = ac_world::quests::rows_of("augmentationprovinggroundaccess")
        .find(|r| r.kind == ac_world::quests::Kind::Item)
        .unwrap();
    let mut q = Quests {
        on: true,
        ..Quests::default()
    };
    assert!(!q.is_worked(fiun));
    q.pursue_quest(fiun.id, false, 0);
    assert!(q.is_worked(fiun));
    let rows = crate::quest_log::considered_rows(&q);
    assert!(rows.iter().any(|r| std::ptr::eq(*r, fiun)));
}

#[test]
fn a_pursued_quest_is_on_with_quests_off() {
    let mut q = Quests::default();
    let fiun = "augmentationprovinggroundaccess";
    assert!(!q.is_on(fiun) && !q.is_on("engorgedeaterjaw"));
    q.pursue_quest(fiun, true, 0);
    assert!(q.is_on(fiun), "pursued, it is worked with quests off");
    assert!(!q.is_on("engorgedeaterjaw"), "and nothing else is");
    q.switch_off(fiun);
    assert!(q.is_on(fiun), "pursuit outranks the switch");
    q.stop_pursuing(fiun);
    assert!(!q.is_on(fiun));
}

#[test]
fn a_pursuit_saved_before_its_outcome_loads_handed_in_to_any_giver() {
    let old: Quests =
        serde_json::from_str(r#"{"pursue":[{"id":"give:8904","again":false,"since":5}]}"#).unwrap();
    assert_eq!(old.pursue[0].outcome, QuestEnd::HandIn);
    assert_eq!(old.outcome_of("give:8904"), &QuestEnd::HandIn);
    assert_eq!(old.outcome_of("not pursued"), &QuestEnd::HandIn);
}

#[test]
fn an_outcome_is_set_on_a_pursued_quest_and_round_trips_through_json() {
    let mut q = Quests::default();
    assert!(!q.set_outcome("give:8904", QuestEnd::Keep), "not pursued");
    q.pursue_quest("give:8904", false, 5);
    assert_eq!(
        q.pursue[0].outcome,
        QuestEnd::HandIn,
        "pursued, it hands in"
    );
    assert!(q.set_outcome("give:8904", QuestEnd::Keep));
    let text = serde_json::to_string(&q).unwrap();
    assert!(text.contains(r#""outcome":{"kind":"Keep"}"#), "{text}");
    let back: Quests = serde_json::from_str(&text).unwrap();
    assert_eq!(back, q);
    let to = QuestEnd::HandInTo {
        giver_wcid: 4122,
        gives: String::new(),
    };
    q.set_outcome("give:8904", to.clone());
    let back: Quests = serde_json::from_str(&serde_json::to_string(&q).unwrap()).unwrap();
    assert_eq!(back.outcome_of("give:8904"), &to);
}

#[test]
fn the_focusing_stone_offers_any_giver_le_ai_rea_and_keeping_the_stone() {
    let row = ac_world::quests::rows_of("give:8904").next().unwrap();
    assert_eq!(row.giver_wcid, 4122);
    let to = QuestEnd::hand_in_to(row);
    assert_eq!(
        quest_ends("give:8904"),
        [QuestEnd::HandIn, to.clone(), QuestEnd::Keep]
    );
    assert!(
        !QuestEnd::Keep.can_hand_in(row),
        "a kept item is never handed in"
    );
    assert!(to.can_hand_in(row) && QuestEnd::HandIn.can_hand_in(row));
    let kill = quest_ends("killtaskbloodshreth");
    assert!(!kill.contains(&QuestEnd::Keep), "nothing to keep: {kill:?}");
}
