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
            ..Default::default()
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
            from: "San Ming".into(),
            flag: "killtaskbloodshreth".into(),
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
        state("give:3684:2470", &facts),
        QuestState::Needs {
            more: 1,
            of: "Red Rat Tail".into()
        }
    );
    facts.carried.insert(3684, 1);
    assert_eq!(state("give:3684:2470", &facts), QuestState::Ready);
    assert_eq!(state("tothebarkeep", &facts), QuestState::Ready);
    let alcott = rows("tothebarkeep").remove(0);
    facts.log.note_done(&alcott, facts.now);
    assert_eq!(state("tothebarkeep", &facts), QuestState::Done);
}

#[test]
fn the_board_wants_a_few_of_an_any_time_hand_in_and_nothing_once_switched_off() {
    let mut facts = on();
    let board = quest_board(&rows("give:3684:2470"), &facts);
    assert_eq!(board[0].givers, 11);
    assert_eq!(
        quest_wants(&board),
        [QuestWant {
            wcid: 3684,
            need: Some(ANY_HAND_INS)
        }]
    );
    facts.config.off.push("give:3684:2470".into());
    let board = quest_board(&rows("give:3684:2470"), &facts);
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
    QuestFacts { level, ..on() }
}

#[test]
fn a_kill_task_waits_for_the_level_its_giver_asks_and_never_for_its_creatures() {
    let id = "diremattekarkillcount";
    assert_eq!(
        state(id, &at_level(15)),
        QuestState::CantYet(Blocked::Level { min: 20, max: 0 })
    );
    // Dire Mattekar are level 40: the game lets a level 20 take the task, so it is doable.
    assert_eq!(
        state(id, &at_level(20)),
        QuestState::NotTaken {
            from: "Fergal the Dire".into(),
            flag: "diremattekarkillcount".into(),
        }
    );
    let mut off = at_level(15);
    off.config.on = false;
    assert!(
        unsuited_quests(&rows(id), &off).contains(id),
        "switched off too"
    );
    assert!(unsuited_quests(&rows(id), &at_level(20)).is_empty());
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
fn a_hand_in_that_needs_a_first_one_names_it_at_any_level_its_dropper_is() {
    // Only Captain Burnja (level 135) drops the board: no game rule keeps a level 20 from it.
    let id = "give:35407";
    assert_eq!(
        state(id, &at_level(20)),
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
            from: "Fergal the Dire".into(),
            flag: "diremattekarkillcount".into(),
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
    let tail = ac_world::quests::rows_of("give:3684:2470").next().unwrap();
    crate::testkit::thing_in_the_pack(&mut c, 0x8000_0100, "Red Rat Tail", 1);
    c.world
        .objects
        .get_mut(&0x8000_0100)
        .unwrap()
        .weenie_class_id = 3684;
    c.ask_giver(tail, tail.places[0], t0);
    c.watch_quest_answer(t0 + XP_WATCH);
    assert!(!c.quest_log.done.contains_key("give:3684:2470"));
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

#[test]
fn a_quest_paying_no_xp_is_wanted_and_ready_only_once_chosen() {
    // Fiun Rehlyun stamps the Augmentation Realm flag for an Insatiable Eater Jaw: no XP.
    let id = "augmentationprovinggroundaccess";
    let mut facts = at_level(150);
    facts.carried.insert(28719, 1);
    let board = quest_board(&rows(id), &facts);
    assert!(board.is_empty(), "not considered: {board:?}");
    assert!(quest_wants(&board).is_empty(), "nothing kept for it");
    facts.config.switch_on(id);
    let board = quest_board(&rows(id), &facts);
    assert_eq!(board[0].state, QuestState::Ready);
    assert!(board[0].every_way);
    let wants = quest_wants(&board);
    assert_eq!(wants.iter().map(|w| w.wcid).collect::<Vec<_>>(), [28719]);
}

#[test]
fn a_way_asking_what_the_client_cannot_check_is_worked_only_once_chosen() {
    // Thorolf's talk pays 12,500 XP only without pathwardencomplete, which nothing shows: on
    // its own, autoplay works the Pathwarden Token's hand-in instead.
    let id = "pathwardenfound1111";
    let mut facts = at_level(5);
    facts.held.push("ToPathwarden".into());
    assert!(rows(id)
        .iter()
        .any(|r| r.kind == Kind::Talk && r.has_unread()));
    assert_eq!(
        state(id, &facts),
        QuestState::Needs {
            more: 1,
            of: "Pathwarden Token".into()
        }
    );
    facts.config.switch_on(id);
    assert_eq!(state(id, &facts), QuestState::Ready, "the talk, chosen");
}

#[test]
fn the_board_is_worked_out_over_the_quests_autoplay_considers_alone() {
    let fiun = "augmentationprovinggroundaccess";
    let mut cfg = Quests {
        on: true,
        ..Quests::default()
    };
    let ids = |cfg: &Quests| {
        let mut ids: Vec<&str> = considered_rows(cfg).iter().map(|r| r.id).collect();
        ids.dedup();
        ids
    };
    let automatic = ids(&cfg);
    assert!(automatic.contains(&"killtaskbloodshreth"));
    assert!(!automatic.contains(&fiun), "pays no XP");
    assert!(
        automatic.windows(2).all(|w| w[0] < w[1]),
        "in id order, once each"
    );
    cfg.switch_on(fiun);
    let chosen = considered_rows(&cfg);
    assert_eq!(
        chosen.iter().filter(|r| r.id == fiun).count(),
        rows(fiun).len(),
        "every row of a chosen quest"
    );
    assert_eq!(ids(&cfg).len(), automatic.len() + 1);
}

#[test]
fn the_browse_board_lists_every_quest_as_if_chosen_whatever_the_switches() {
    let mut facts = at_level(150);
    facts.config.on = false;
    facts.carried.insert(28719, 1);
    let board = browse_board(&facts);
    let ids: BTreeSet<&str> = ac_world::quests::all().iter().map(|r| r.id).collect();
    assert_eq!(board.len(), ids.len());
    assert!(board.iter().all(|l| l.on && l.every_way));
    let fiun = board
        .iter()
        .find(|l| l.id == "augmentationprovinggroundaccess")
        .unwrap();
    assert_eq!(fiun.state, QuestState::Ready, "the jaw carried");
}

/// Quest `id` pursued at `level`, quests otherwise off.
fn pursued_at(id: &str, level: u32) -> QuestFacts {
    let mut facts = at_level(level);
    facts.config.on = false;
    facts.config.pursue_quest(id, false, 0);
    facts
}

#[test]
fn a_quest_whose_creatures_are_far_above_the_character_is_wanted() {
    // The jaw's Engorged Eater is level 80: at 50 the jaw is wanted, on or pursued.
    let id = "engorgedeaterjaw";
    let needs = QuestState::Needs {
        more: 1,
        of: "Engorged Eater Jaw".into(),
    };
    assert_eq!(state(id, &at_level(50)), needs);
    let facts = pursued_at(id, 50);
    let board = quest_board(&rows(id), &facts);
    assert_eq!(
        board[0].state,
        QuestState::Needs {
            more: 1,
            of: "Engorged Eater Jaw".into()
        }
    );
    assert!(board[0].on, "pursued with quests off");
    assert!(quest_wants(&board).iter().any(|w| w.wcid == 28725));
}

#[test]
fn a_pursued_quest_outside_the_givers_band_stays_out() {
    // Fergal the Dire takes the Dire Mattekar task from level 20.
    let id = "diremattekarkillcount";
    assert_eq!(
        state(id, &pursued_at(id, 15)),
        QuestState::CantYet(Blocked::Level { min: 20, max: 0 })
    );
}

#[test]
fn a_pursuit_asked_for_once_comes_off_the_list_when_done_and_one_asked_again_stays() {
    let mut c = crate::testkit::character_of_level(crate::testkit::no_data(), 5);
    let quests = &mut c.autoplay.config.quests;
    quests.pursue_quest("tothebarkeep", false, 0);
    quests.pursue_quest("give:3684:2470", true, 0);
    let alcott = ac_world::quests::rows_of("tothebarkeep").next().unwrap();
    c.quest_log.note_done(alcott, crate::holdings::unix_now());
    c.refresh_quests(Instant::now());
    let ids: Vec<&str> = c
        .autoplay
        .config
        .quests
        .pursue
        .iter()
        .map(|p| p.id.as_str())
        .collect();
    assert_eq!(ids, ["give:3684:2470"]);
}

#[test]
fn an_unpaid_talk_the_client_cannot_check_is_not_done_and_a_flag_refused_unlearns_its_quest() {
    let mut c = crate::testkit::character_of_level(crate::testkit::no_data(), 5);
    let t0 = Instant::now();
    let talk = ac_world::quests::all()
        .iter()
        .find(|r| r.kind == Kind::Talk && r.has_unread() && !r.has_xp() && r.repeat != Repeat::Any)
        .expect("a talk on an unread condition");
    c.ask_giver(talk, talk.places[0], t0);
    c.watch_quest_answer(t0 + XP_WATCH);
    assert!(!c.quest_log.done.contains_key(talk.id), "{}", talk.id);
    let key = (talk.giver_wcid, talk.places[0].cell);
    assert!(c.autoplay.growth.givers_refused.since(&key).is_some());

    // Done in the log, its stamp reads held; a portal refusing the flag says it is not.
    let alcott = ac_world::quests::rows_of("tothebarkeep").next().unwrap();
    c.quest_log.note_done(alcott, 1);
    assert!(holds(&c.quest_facts(), "ToTheBarkeep"));
    c.drop_quest_flag("ToTheBarkeep");
    assert!(!holds(&c.quest_facts(), "ToTheBarkeep"));
}

#[test]
fn a_kill_task_handed_in_short_and_unpaid_is_not_done_and_starts_no_wait() {
    let mut c = crate::testkit::character_of_level(crate::testkit::no_data(), 41);
    let t0 = Instant::now();
    let kill = ac_world::quests::rows_of("killtaskbloodshreth")
        .find(|r| r.kind == Kind::Kill)
        .unwrap();
    c.quest_log.note_taken("killtaskbloodshreth");
    c.ask_giver(kill, kill.places[0], t0);
    c.watch_quest_answer(t0 + XP_WATCH);
    assert!(!c.quest_log.done.contains_key(kill.id));
    assert!(c.quest_log.waits.is_empty(), "{:?}", c.quest_log.waits);
    assert_eq!(c.quest_log.kills.get("killtaskbloodshreth"), Some(&0));
}

#[test]
fn a_quest_asked_again_after_its_hand_in_is_done() {
    // Blargerton handed Bleeargh the Necklace of Leerargh and the run used him again 0.3 s on:
    // that ask, the necklace gone already, replaced the first, and "gave nothing" was heard.
    let mut c = crate::testkit::character_of_level(crate::testkit::game_data(), 51);
    let row = ac_world::quests::rows_of("magicitemtraining3")
        .find(|r| r.kind == ac_world::quests::Kind::Item)
        .unwrap();
    crate::testkit::thing_in_the_pack(&mut c, 0x8000_0100, "Necklace of Leerargh", 1);
    c.world
        .objects
        .get_mut(&0x8000_0100)
        .unwrap()
        .weenie_class_id = 27902;
    let t0 = Instant::now();
    c.ask_giver(row, row.places[0], t0);
    c.world.objects.remove(&0x8000_0100);
    c.ask_giver(
        row,
        row.places[0],
        t0 + std::time::Duration::from_millis(300),
    );
    c.watch_quest_answer(t0 + XP_WATCH * 2);
    assert!(c.quest_log.done.contains_key("magicitemtraining3"));
}

#[test]
fn the_servers_quest_log_says_a_quest_handed_in_is_waiting() {
    // Blargerton's necklace, handed in under a client that heard "gave nothing": the server's
    // own log (`@myquests`) has its stamp and its 20 h timer.
    let mut c = crate::testkit::character_of_level(crate::testkit::no_data(), 51);
    c.autoplay.config.quests.on = true;
    let now = crate::holdings::unix_now();
    c.hear_server_quest(&format!(
        "magicitemtraining3 - 1 solves ({})\"Learned item enchantment\" -1 0",
        now - 60
    ));
    c.hear_server_quest(&format!(
        "mosswartexodusnecklace - 1 solves ({})\"Mosswart Exodus Necklace\" -1 72000",
        now - 60
    ));
    assert!(c.quest_log.done.contains_key("magicitemtraining3"));
    assert_eq!(
        c.quest_log.wait_until("mosswartexodusnecklace", now),
        Some(now - 60 + 72000)
    );
    assert!(c.holds_quest_flag("magicitemtraining3"));
}

#[test]
fn a_kill_task_the_servers_log_does_not_list_is_taken_up_again() {
    // Blargerton's log kept the Tusker Guard counter from an earlier session; the server had
    // erased it, so he hunted guards that counted for nothing instead of taking the task up.
    let mut c = crate::testkit::character_of_level(crate::testkit::no_data(), 51);
    c.autoplay.config.quests.on = true;
    let now = crate::holdings::unix_now();
    c.quest_log.note_taken("tuskergkillcount");
    c.quest_log.note_kills("killtaskbloodshreth", 4);
    c.quest_log.note_wait("tuskerg", now + 600);
    c.hold_quest_flags(&["tuskergkillcount".to_string(), "tuskerg".to_string()]);
    let t0 = Instant::now();
    c.hear_server_quest(&format!(
        "tuskerg - 1 solves ({})\"Tusker Guard Kill Task Main Flag\" -1 72000",
        now - 100_000
    ));
    c.hear_server_quest(&format!(
        "killtaskbloodshreth - 4 solves ({})\"Blood Shreth Kill Task\" 10 0",
        now - 100
    ));
    // Mid-listing nothing is dropped: more lines may be on their way.
    c.settle_server_quests(Instant::now());
    assert!(c.quest_log.kills.contains_key("tuskergkillcount"));
    c.settle_server_quests(t0 + LISTING_QUIET + Duration::from_millis(100));
    assert!(!c.quest_log.kills.contains_key("tuskergkillcount"));
    assert_eq!(c.quest_log.kills.get("killtaskbloodshreth"), Some(&4));
    assert!(!c.holds_quest_flag("tuskergkillcount"));
    assert!(c.holds_quest_flag("tuskerg"));
    assert_eq!(
        c.quest_log.wait_until("tuskerg", now),
        None,
        "its wait is over"
    );
    c.refresh_quests(Instant::now() + Duration::from_secs(5));
    let line = c.quest_board.iter().find(|l| l.id == "tuskergkillcount");
    assert!(
        matches!(line.map(|l| &l.state), Some(QuestState::NotTaken { .. })),
        "{line:?}"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_hand_in_paying_an_item_and_no_xp_is_done_once_the_item_comes() {
    // Blargerton handed Jilsaya bint Dah a Stamped Samsur Lucky Gold Letter for the Seasoned
    // Explorer Boots: no XP, so "gave nothing" was heard, and another letter was stamped.
    let mut c = crate::testkit::character_of_level(crate::testkit::game_data(), 51);
    let row = ac_world::quests::rows_of("give:70309:45975")
        .find(|r| r.kind == ac_world::quests::Kind::Item)
        .unwrap();
    assert!(
        row.gives.iter().any(|(w, _)| *w == 45975),
        "{:?}",
        row.gives
    );
    const LETTER: u32 = 0x8000_0100;
    crate::testkit::thing_in_the_pack(&mut c, LETTER, "Stamped Samsur Lucky Gold Letter", 1);
    c.world.objects.get_mut(&LETTER).unwrap().weenie_class_id = 70309;
    let t0 = Instant::now();
    c.ask_giver(row, row.places[0], t0);
    c.world.objects.remove(&LETTER);
    assert!(c.is_awaiting_answer(row.id), "the hand-in is not awaited");
    c.watch_quest_answer(t0 + Duration::from_millis(500));
    assert!(
        c.is_awaiting_answer(row.id),
        "answered before the boots came"
    );
    crate::testkit::thing_in_the_pack(&mut c, 0x8000_0101, "Seasoned Explorer Boots", 1);
    c.world
        .objects
        .get_mut(&0x8000_0101)
        .unwrap()
        .weenie_class_id = 45975;
    c.watch_quest_answer(t0 + Duration::from_millis(1500));
    assert!(!c.is_awaiting_answer(row.id));
    assert!(
        c.quest_log.done.contains_key(row.id),
        "the boots counted for nothing"
    );
}
