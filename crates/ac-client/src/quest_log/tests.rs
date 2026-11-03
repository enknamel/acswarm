use super::*;

fn rows(id: &str) -> Vec<Row> {
    ac_world::quests::rows_of(id).cloned().collect()
}

fn on() -> QuestFacts {
    QuestFacts {
        level: 20,
        config: Quests {
            on: true,
            off: Vec::new(),
        },
        now: 1_000_000,
        ..QuestFacts::default()
    }
}

fn state(id: &str, facts: &QuestFacts) -> QuestState {
    let board = quest_board(&rows(id), facts);
    assert_eq!(board.len(), 1, "{id}: one line");
    board[0].state.clone()
}

#[test]
fn a_kill_task_is_taken_up_counted_then_ready_and_waits_once_handed_in() {
    let id = "killtaskbloodshreth";
    assert_eq!(state(id, &QuestFacts::default()), QuestState::Off);
    let mut facts = on();
    assert_eq!(
        state(id, &facts),
        QuestState::NotTaken {
            from: "San Ming".into()
        }
    );
    facts.log.note_taken(id);
    facts.log.note_kills(id, 7);
    assert_eq!(
        state(id, &facts),
        QuestState::Needs {
            more: 3,
            of: "Blood Shreth".into()
        }
    );
    facts.log.note_kills(id, 10);
    assert_eq!(state(id, &facts), QuestState::Ready);
    let kill = rows(id).into_iter().find(|r| r.kind == Kind::Kill).unwrap();
    facts.log.note_done(&kill, facts.now);
    assert!(!facts.log.kills.contains_key(id), "ACE erased the counter");
    assert_eq!(
        state(id, &facts),
        QuestState::Waiting {
            until: facts.now + 72_000
        }
    );
}

#[test]
fn an_item_quest_needs_its_item_carried_and_one_done_by_talk_stays_done() {
    let mut facts = on();
    assert_eq!(
        state("give:3684", &facts),
        QuestState::Needs {
            more: 1,
            of: "Red Rat Tail".into()
        }
    );
    facts.carried.insert(3684, 1);
    assert_eq!(state("give:3684", &facts), QuestState::Ready);
    assert_eq!(state("tothebarkeep", &facts), QuestState::Ready);
    let alcott = rows("tothebarkeep").remove(0);
    facts.log.note_done(&alcott, facts.now);
    assert_eq!(state("tothebarkeep", &facts), QuestState::Done);
}

#[test]
fn the_board_wants_a_few_of_an_any_time_hand_in_and_nothing_once_switched_off() {
    let mut facts = on();
    let board = quest_board(&rows("give:3684"), &facts);
    assert_eq!(board[0].givers, 12);
    assert_eq!(
        quest_wants(&board),
        [QuestWant {
            wcid: 3684,
            need: Some(ANY_HAND_INS)
        }]
    );
    facts.config.off.push("give:3684".into());
    let board = quest_board(&rows("give:3684"), &facts);
    assert!(!board[0].on);
    assert!(quest_wants(&board).is_empty());
}

#[test]
fn a_detour_pays_only_once_the_rate_is_measured() {
    assert!(!worth_detour(10_000, 60.0, None));
    assert!(worth_detour(10_000, 60.0, Some(600_000.0)));
    assert!(
        !worth_detour(10_000, 600.0, Some(100_000.0)),
        "16,667 XP of hunting lost"
    );
}

fn at_level(level: u32) -> QuestFacts {
    QuestFacts {
        level,
        margin: 8,
        ..on()
    }
}

#[test]
fn a_kill_task_waits_for_the_level_its_giver_asks_and_its_creatures_suit() {
    let id = "diremattekarkillcount";
    assert_eq!(
        state(id, &at_level(15)),
        QuestState::CantYet(Blocked::Level { min: 20, max: 0 })
    );
    assert_eq!(
        state(id, &at_level(20)),
        QuestState::CantYet(Blocked::TooHard { level: 40 }),
        "Dire Mattekar are level 40, past 20 + 2 * 8"
    );
    assert_eq!(
        state(id, &at_level(25)),
        QuestState::NotTaken {
            from: "Fergal the Dire".into()
        }
    );
    let mut off = at_level(15);
    off.config.on = false;
    assert!(
        unsuited_quests(&rows(id), &off).contains(id),
        "switched off too"
    );
    assert!(unsuited_quests(&rows(id), &at_level(20)).contains(id));
    assert!(unsuited_quests(&rows(id), &at_level(25)).is_empty());
}

#[test]
fn a_talk_that_needs_another_quest_first_names_it_until_that_one_is_done() {
    let id = "pathwardenfound1111";
    let mut facts = at_level(5);
    // The Pathwarden Token's hand-in pays the same XP and asks no flag.
    assert_eq!(
        state(id, &facts),
        QuestState::Needs {
            more: 1,
            of: "Pathwarden Token".into()
        }
    );
    // Thorolf's talk asks ToPathwarden, a way only without pathwardencomplete (unread): read
    // alone with that taken as so, it is the talk the XP-only data had.
    let mut talk = rows(id).into_iter().find(|r| r.kind == Kind::Talk).unwrap();
    talk.unread.clear();
    let talk_state = |facts: &QuestFacts| {
        quest_board(std::slice::from_ref(&talk), facts)[0]
            .state
            .clone()
    };
    assert_eq!(
        talk_state(&facts),
        QuestState::CantYet(Blocked::NeedsFirst {
            title: "Sent to the Pathwarden".into()
        })
    );
    let buckminster = rows("topathwarden").remove(0);
    facts.log.note_done(&buckminster, facts.now);
    assert_eq!(talk_state(&facts), QuestState::Ready, "the log's own stamp");
    let mut flagged = at_level(5);
    flagged.held.push("ToPathwarden".into());
    assert_eq!(
        talk_state(&flagged),
        QuestState::Ready,
        "a flag in any case"
    );
}

#[test]
fn a_hand_in_that_needs_a_first_one_names_it_and_its_dropper_rules_out_low_levels() {
    let id = "give:35407";
    assert_eq!(
        state(id, &at_level(20)),
        QuestState::CantYet(Blocked::TooHard { level: 135 }),
        "only Captain Burnja drops the board"
    );
    assert_eq!(
        state(id, &at_level(150)),
        QuestState::CantYet(Blocked::NeedsFirst {
            title: "Player has already turned in 1 Burnjas Board with nails".into()
        })
    );
}

#[test]
fn a_giver_behind_a_flagged_portal_is_locked_until_the_flag_is_held() {
    let id = "tanadahouseoffirecompleted";
    let mut facts = at_level(100);
    assert_eq!(
        state(id, &facts),
        QuestState::CantYet(Blocked::Locked {
            flag: "TanadaHouseofFireStarted".into()
        })
    );
    facts.held.push("tanadahouseoffirestarted".into());
    assert_eq!(state(id, &facts), QuestState::Ready);
}

#[test]
fn the_board_is_worked_out_again_at_once_when_quests_are_switched_on_or_the_level_changes() {
    let mut c = crate::testkit::character_of_level(crate::testkit::no_data(), 15);
    let t0 = Instant::now();
    c.refresh_quests(t0);
    assert!(!c.quest_board.is_empty());
    assert!(c.quest_board.iter().all(|l| l.state == QuestState::Off));
    assert!(c.quests_unsuited.contains("diremattekarkillcount"));
    let soon = t0 + Duration::from_millis(100);
    c.autoplay.config.quests.on = true;
    c.refresh_quests(soon);
    let line = |c: &Client, id: &str| {
        let l = c.quest_board.iter().find(|l| l.id == id).cloned();
        l.unwrap().state
    };
    assert_eq!(
        line(&c, "diremattekarkillcount"),
        QuestState::CantYet(Blocked::Level { min: 20, max: 0 })
    );
    c.world.stats.level = 26;
    c.refresh_quests(soon + Duration::from_millis(100));
    assert_eq!(
        line(&c, "diremattekarkillcount"),
        QuestState::NotTaken {
            from: "Fergal the Dire".into()
        }
    );
    assert!(!c.quests_unsuited.contains("diremattekarkillcount"));
}

fn server_says(c: &mut Client, text: &str) {
    let mut w = ac_net::wire::Writer::new();
    w.string16(text).u32(0x11);
    c.chat_message(ac_net::messages::opcode::SERVER_MESSAGE, &w.finish());
}

#[test]
fn a_kill_tasks_count_heard_goes_into_the_log_under_its_counter() {
    let mut c = crate::testkit::character_of_level(crate::testkit::no_data(), 20);
    server_says(
        &mut c,
        "You have killed 7 Blood Shreth! You must kill 10 to complete your task.",
    );
    assert_eq!(c.quest_log.kills.get("killtaskbloodshreth"), Some(&7));
    let counter = ac_world::quests::sources()
        .iter()
        .find(|s| s.name == "Guruk Monstrosity" && s.kind == Yield::Kill)
        .map(|s| s.want.clone());
    let Some(Want::Kill(counter)) = counter else {
        panic!("a counter for Guruk Monstrosity");
    };
    let done = rows(&counter)
        .iter()
        .find(|r| r.kind == Kind::Kill)
        .unwrap()
        .count;
    server_says(
        &mut c,
        &format!("You have killed {done} Guruk Monstrosities! Your task is complete!"),
    );
    assert_eq!(c.quest_log.kills.get(&counter), Some(&done), "the plural");
    assert_eq!(c.quest_log.kills.len(), 2);
    server_says(&mut c, "You have killed 3 Nobodies! Your task is complete!");
    assert_eq!(c.quest_log.kills.len(), 2, "two taken up: whose is unknown");

    let mut one = crate::testkit::character_of_level(crate::testkit::no_data(), 20);
    one.quest_log.note_taken("killtaskbloodshreth");
    server_says(
        &mut one,
        "You have killed 2 Zzyzx Beasts! You must kill 10 to complete your task.",
    );
    assert_eq!(
        one.quest_log.kills.get("killtaskbloodshreth"),
        Some(&2),
        "a plural no creature's name starts: the only task taken up"
    );
}

#[test]
fn a_giver_that_pays_marks_the_quest_done_and_one_that_takes_nothing_is_let_be() {
    let mut c = crate::testkit::character_of_level(crate::testkit::no_data(), 5);
    c.autoplay.config.quests.on = true;
    let t0 = Instant::now();
    let alcott = ac_world::quests::rows_of("tothebarkeep")
        .find(|r| r.giver == "Alcott")
        .unwrap();
    c.ask_giver(alcott, alcott.places[0], t0);
    c.watch_quest_answer(t0 + Duration::from_secs(1));
    assert!(c.quest_asked.is_some(), "no answer yet: still watched");
    c.world.stats.total_xp += 7_000;
    c.watch_quest_answer(t0 + Duration::from_secs(2));
    assert!(c.quest_asked.is_none());
    assert!(c.quest_log.done.contains_key("tothebarkeep"));
    assert!(c.holds_quest_flag("ToTheBarkeep") && c.holds_quest_flag("LifestoneFound1111"));
    c.refresh_quests(t0 + Duration::from_secs(3));
    let line = c.quest_board.iter().find(|l| l.id == "tothebarkeep");
    assert_eq!(line.map(|l| l.state.clone()), Some(QuestState::Done));

    // A Collector handed a tail it never took: no XP, the tail still carried.
    let tail = ac_world::quests::rows_of("give:3684").next().unwrap();
    crate::testkit::thing_in_the_pack(&mut c, 0x8000_0100, "Red Rat Tail", 1);
    c.world
        .objects
        .get_mut(&0x8000_0100)
        .unwrap()
        .weenie_class_id = 3684;
    c.ask_giver(tail, tail.places[0], t0);
    c.watch_quest_answer(t0 + XP_WATCH);
    assert!(!c.quest_log.done.contains_key("give:3684"));
    let key = (tail.giver_wcid, tail.places[0].cell);
    assert!(c.autoplay.growth.givers_refused.since(&key).is_some());
}

#[test]
fn a_once_quest_whose_flag_is_held_from_before_the_log_is_done() {
    let mut facts = at_level(5);
    assert_eq!(state("tothebarkeep", &facts), QuestState::Ready);
    facts.held.push("ToTheBarkeep".into());
    assert_eq!(state("tothebarkeep", &facts), QuestState::Done);
}

#[test]
fn a_quest_taking_any_of_several_items_is_ready_with_any_one_and_wants_each() {
    // Aun Mareura takes any one of five items for aunkiller.
    let id = "aunkiller";
    let mut facts = at_level(200);
    facts.carried.insert(7604, 1);
    assert_eq!(state(id, &facts), QuestState::Ready, "a Yellow Jewel");
    let wants = quest_wants(&quest_board(&rows(id), &facts));
    let wcids: Vec<u32> = wants.iter().map(|w| w.wcid).collect();
    for w in [3679, 7604, 9292, 11341, 11353] {
        assert!(wcids.contains(&w), "{w} not wanted: {wants:?}");
    }
}

#[test]
fn a_mate_is_told_only_what_is_still_wanted_beyond_what_is_carried() {
    let mut c = crate::testkit::character_of_level(crate::testkit::no_data(), 20);
    c.quest_wants = vec![
        QuestWant {
            wcid: 19209,
            need: Some(1),
        },
        QuestWant {
            wcid: 3684,
            need: Some(5),
        },
    ];
    crate::testkit::thing_in_the_pack(&mut c, 0x8000_0100, "Coil", 1);
    c.world
        .objects
        .get_mut(&0x8000_0100)
        .unwrap()
        .weenie_class_id = 19209;
    assert_eq!(
        c.quest_wants_left(),
        [QuestWant {
            wcid: 3684,
            need: Some(5)
        }]
    );
}
