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
fn the_board_wants_an_any_time_hand_in_every_one_and_nothing_once_switched_off() {
    let mut facts = on();
    let board = quest_board(&rows("give:3684"), &facts);
    assert_eq!(board[0].givers, 12);
    assert_eq!(
        quest_wants(&board),
        [QuestWant {
            wcid: 3684,
            need: None
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
