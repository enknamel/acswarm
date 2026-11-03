use super::*;
use ac_client::autoplay::{PursuitLine, PursuitStep};
use ac_world::quest_chain::SourceGround;
use ac_world::quests::{Kind, Repeat};

/// A synthetic card: one giver at one place, open levels, nothing else.
fn card(id: &str, title: &str, item: Option<&str>, giver: (&str, &str)) -> QuestCard {
    QuestCard {
        id: id.into(),
        title: title.into(),
        kind: Kind::Item,
        item: item.map(str::to_string),
        givers: vec![(giver.0.into(), giver.1.into())],
        creatures: Vec::new(),
        places: Vec::new(),
        levels: (0, 0),
        easiest: None,
        entry: 0,
        xp: 0,
        gives: Vec::new(),
        repeat: Repeat::Any,
        flags: Vec::new(),
        does: Vec::new(),
        unread: Vec::new(),
    }
}

/// The eater jaws, a rat tail and a kill task, as the data's cards would read.
fn cards() -> Vec<QuestCard> {
    let mut engorged = card(
        "engorgedeaterjaw",
        "Engorged Eater Jaw",
        Some("Engorged Eater Jaw"),
        ("Clemenza", "Westwatch"),
    );
    engorged.creatures = vec![("Engorged Eater".into(), 80, 1.0)];
    engorged.places = vec!["Remote Empyrean Vault".into()];
    engorged.easiest = Some(80);
    engorged.entry = 40;
    engorged.xp = 2_500_000;
    engorged.gives = vec!["Trade Note (75,000) x1".into()];
    engorged.repeat = Repeat::After(72_000);
    engorged.does = vec!["xp".into(), "item".into(), "takes".into()];
    let mut insatiable = card(
        "augmentationprovinggroundaccess",
        "Augmentation Proving Ground Access",
        Some("Insatiable Eater Jaw"),
        ("Fiun Rehlyun", "Fiun Outpost"),
    );
    insatiable.creatures = vec![("Insatiable Eater".into(), 160, 1.0)];
    insatiable.places = vec!["Insatiable Vault".into()];
    insatiable.easiest = Some(160);
    insatiable.entry = 125;
    insatiable.does = vec!["flag".into(), "takes".into()];
    let mut tail = card(
        "give:3684",
        "Red Rat Tail",
        Some("Red Rat Tail"),
        ("Yukara Miko the Collector", "Shoushi"),
    );
    tail.creatures = vec![("Red Rat".into(), 3, 0.5)];
    tail.places = vec!["Shoushi".into()];
    tail.easiest = Some(3);
    tail.xp = 1_000;
    tail.does = vec!["xp".into(), "type 75".into()];
    let mut task = card(
        "repugeaterkillcount",
        "Repugnant Eater Kill Task",
        None,
        ("Xun Yu", "Yanshi"),
    );
    task.kind = Kind::Kill;
    task.creatures = vec![("Repugnant Eater".into(), 95, 1.0)];
    task.levels = (80, 0);
    task.easiest = Some(95);
    task.xp = 70_000_000;
    task.repeat = Repeat::After(604_800);
    task.flags = vec!["repugeaterkillcount".into()];
    vec![engorged, insatiable, tail, task]
}

/// The ids `f` lists out of `cards`, in the data's order, at level `level`.
fn listed(cards: &[QuestCard], f: &Filter, level: u32) -> Vec<String> {
    let terms = parse_search(&f.search);
    let me = Me { level, margin: 10 };
    let cfg = Quests::default();
    cards
        .iter()
        .filter(|c| is_listed(c, None, &cfg, f, &terms, me))
        .map(|c| c.id.clone())
        .collect()
}

fn all() -> Filter {
    Filter {
        show: Show::All,
        ..Default::default()
    }
}

fn searching(text: &str) -> Filter {
    Filter {
        search: text.into(),
        ..all()
    }
}

#[test]
fn a_search_reads_prefixes_and_quoted_phrases() {
    assert_eq!(
        parse_search(r#"jaw at:"Remote Empyrean"  Giver:clemenza item: nope:x"#),
        vec![
            (Facet::Any, "jaw".to_string()),
            (Facet::At, "remote empyrean".to_string()),
            (Facet::Giver, "clemenza".to_string()),
            (Facet::Any, "nope:x".to_string()),
        ],
        "an empty prefix is dropped and an unknown one is a plain word"
    );
    assert!(parse_search("   ").is_empty());
}

#[test]
fn an_item_word_finds_the_jaw_quests() {
    let cards = cards();
    assert_eq!(
        listed(&cards, &searching("item:jaw"), 0),
        ["engorgedeaterjaw", "augmentationprovinggroundaccess"]
    );
    assert_eq!(
        listed(&cards, &searching("item:\"insatiable eater jaw\""), 0),
        ["augmentationprovinggroundaccess"]
    );
    assert_eq!(
        listed(&cards, &searching("jaw"), 0).len(),
        2,
        "a plain word reads the item too"
    );
}

#[test]
fn a_place_word_finds_quests_at_westwatch() {
    let cards = cards();
    assert_eq!(
        listed(&cards, &searching("at:westwatch"), 0),
        ["engorgedeaterjaw"]
    );
    assert_eq!(
        listed(&cards, &searching("at:\"remote empyrean\""), 0),
        ["engorgedeaterjaw"],
        "the place it comes from, as well as the giver's"
    );
    assert!(listed(&cards, &searching("giver:westwatch"), 0).is_empty());
}

#[test]
fn a_creature_word_finds_what_it_drops() {
    let cards = cards();
    assert_eq!(
        listed(&cards, &searching("creature:\"insatiable eater\""), 0),
        ["augmentationprovinggroundaccess"]
    );
    assert_eq!(
        listed(&cards, &searching("creature:eater"), 0),
        [
            "engorgedeaterjaw",
            "augmentationprovinggroundaccess",
            "repugeaterkillcount"
        ]
    );
    assert_eq!(
        listed(&cards, &searching("creature:eater reward:trade"), 0),
        ["engorgedeaterjaw"],
        "every word must match"
    );
}

#[test]
fn the_weekly_chip_keeps_seven_day_quests() {
    let cards = cards();
    let f = |repeat| Filter { repeat, ..all() };
    assert_eq!(
        listed(&cards, &f(RepeatChip::Weekly), 0),
        ["repugeaterkillcount"]
    );
    assert_eq!(
        listed(&cards, &f(RepeatChip::Daily), 0),
        ["engorgedeaterjaw"]
    );
    assert_eq!(
        listed(&cards, &f(RepeatChip::AnyTime), 0),
        ["augmentationprovinggroundaccess", "give:3684"]
    );
    assert!(listed(&cards, &f(RepeatChip::Once), 0).is_empty());
}

#[test]
fn the_kind_and_does_filters_keep_their_own() {
    let cards = cards();
    let kill = Filter {
        kind: Some(Kind::Kill),
        ..all()
    };
    assert_eq!(listed(&cards, &kill, 0), ["repugeaterkillcount"]);
    let flag = Filter {
        does: Some("flag"),
        ..all()
    };
    assert_eq!(
        listed(&cards, &flag, 0),
        ["augmentationprovinggroundaccess"]
    );
    let other = Filter {
        does: Some("type"),
        ..all()
    };
    assert_eq!(
        listed(&cards, &other, 0),
        ["give:3684"],
        "type 75 is a type"
    );
}

#[test]
fn the_level_band_hides_quests_outside_it() {
    let cards = cards();
    let band = |lo, hi| Filter {
        levels: (lo, hi),
        ..all()
    };
    assert_eq!(
        listed(&cards, &band(70, 100), 0),
        ["engorgedeaterjaw", "repugeaterkillcount"]
    );
    assert_eq!(listed(&cards, &band(0, 10), 0), ["give:3684"]);
    assert_eq!(listed(&cards, &band(0, 0), 0).len(), 4, "0 is open");
    // For my level: the giver's band, and the easiest source within twice the margin above.
    let mine = Filter::default();
    assert_eq!(
        listed(&cards, &mine, 75),
        ["engorgedeaterjaw", "give:3684"],
        "the kill task's Repugnant Eater (95) is beyond 75 + 2 x 10"
    );
    assert_eq!(
        listed(&cards, &mine, 50),
        ["give:3684"],
        "the Engorged Eater (80) is beyond 50 + 2 x 10"
    );
}

#[test]
fn the_state_chip_reads_the_browse_board() {
    let cards = cards();
    let mut browse = Browse::default();
    let line = |id: &str, state| QuestLine {
        id: id.into(),
        title: String::new(),
        kind: Kind::Item,
        state,
        xp: 0,
        givers: 1,
        giver: String::new(),
        on: true,
        every_way: true,
    };
    browse.set(
        vec![
            line("engorgedeaterjaw", QuestState::Ready),
            line("give:3684", QuestState::Done),
        ],
        std::time::Instant::now(),
    );
    let f = Filter {
        state: StateChip::Ready,
        ..all()
    };
    let terms = parse_search("");
    let shown: Vec<&str> = cards
        .iter()
        .filter(|c| {
            let l = browse.line(&c.id);
            is_listed(c, l, &Quests::default(), &f, &terms, Me::default())
        })
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(
        shown,
        ["engorgedeaterjaw"],
        "a quest with no state is left out"
    );
}

#[test]
fn sorting_by_xp_puts_the_best_first() {
    let cards = cards();
    let mut rows: Vec<usize> = (0..cards.len()).collect();
    let browse = Browse::default();
    sort_rows(&mut rows, &cards, &browse, Sort::default());
    let ids: Vec<&str> = rows.iter().map(|&i| cards[i].id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "repugeaterkillcount",
            "engorgedeaterjaw",
            "give:3684",
            "augmentationprovinggroundaccess"
        ]
    );
    let by_title = Sort {
        col: Column::Title,
        descending: false,
    };
    sort_rows(&mut rows, &cards, &browse, by_title);
    assert_eq!(cards[rows[0]].title, "Augmentation Proving Ground Access");
    assert_eq!(cards[rows[3]].title, "Repugnant Eater Kill Task");
}

#[test]
fn the_switch_puts_an_xp_quest_back_and_chooses_any_other() {
    let mut cfg = Quests {
        on: true,
        ..Default::default()
    };
    // Real ids: whether a quest works on its own is the data's.
    assert!(is_switched_on(&cfg, "engorgedeaterjaw"), "it pays XP");
    assert!(!is_switched_on(&cfg, "augmentationprovinggroundaccess"));
    set_switch(&mut cfg, "engorgedeaterjaw", false);
    assert!(!is_switched_on(&cfg, "engorgedeaterjaw"));
    set_switch(&mut cfg, "engorgedeaterjaw", true);
    assert!(is_switched_on(&cfg, "engorgedeaterjaw"));
    assert!(
        cfg.off.is_empty() && cfg.chosen.is_empty(),
        "back as it started"
    );
    set_switch(&mut cfg, "augmentationprovinggroundaccess", true);
    assert_eq!(cfg.chosen, ["augmentationprovinggroundaccess"]);
    assert!(cfg.is_on("augmentationprovinggroundaccess"));
}

fn known<'a>(log: &'a QuestLog, held: &'a [String], place: &'a dyn Fn(u32) -> String) -> Known<'a> {
    Known {
        held,
        log,
        me: Me {
            level: 50,
            margin: 10,
        },
        place,
    }
}

/// Places in words, as the data names these two.
fn place_of(cell: u32) -> String {
    match cell >> 16 {
        0x23DA => "Westwatch".into(),
        0x0001 => "Remote Empyrean Vault".into(),
        _ => format!("{cell:08X}"),
    }
}

#[test]
fn the_jaw_chain_reads_in_plain_words() {
    let rows: Vec<_> = ac_world::quests::rows_of("engorgedeaterjaw").collect();
    let source = ac_world::quests::sources_of(&rows[0].want)
        .next()
        .expect("the jaw drops from an eater");
    let links = [
        ChainLink::Hunt(vec![SourceGround {
            landblock: 0x0001,
            source,
            entries: 4,
            place: None,
            way_in: None,
        }]),
        ChainLink::HandIn(rows),
    ];
    let log = QuestLog::default();
    let steps = chain_words(&links, &known(&log, &[], &place_of));
    let said: Vec<String> = steps.iter().map(Step::text).collect();
    assert_eq!(
        said,
        [
            "Loot Engorged Eater (level 80) in Remote Empyrean Vault, 4 spawns - 30 levels above \
             you (no way in known)",
            "Hand Engorged Eater Jaw to Clemenza at Westwatch",
        ]
    );
    assert!(steps[1]
        .pieces
        .contains(&Piece::Link(Facet::Giver, "Clemenza".into())));
}

#[test]
fn a_flag_nobody_gives_says_so_and_a_held_one_is_done() {
    let link = |flag: &str| ChainLink::Flag {
        flag: flag.into(),
        by: Vec::new(),
        portal: None,
    };
    let log = QuestLog::default();
    let held = ["Eaterpit".to_string()];
    let steps = chain_words(
        &[link("eaterpit"), link("other")],
        &known(&log, &held, &place_of),
    );
    assert_eq!(
        steps[0].text(),
        "Flag eaterpit: nobody known gives it (autoplay can't get it)"
    );
    assert!(steps[0].done, "flag names compare case-insensitively");
    assert!(!steps[1].done);
}

#[test]
fn a_card_alone_tells_its_steps() {
    let log = QuestLog::default();
    let steps = card_steps(&cards()[3], &known(&log, &[], &place_of));
    let said: Vec<String> = steps.iter().map(Step::text).collect();
    assert_eq!(
        said,
        [
            "Hold repugeaterkillcount",
            "Kill Repugnant Eater (level 95)",
            "Report the kills to Xun Yu at Yanshi",
        ]
    );
}

#[test]
fn a_pursued_step_reads_in_words() {
    let hunt = PursuitStep::Hunt {
        landblock: 0x0001,
        creature: "Engorged Eater".into(),
        place: "Remote Empyrean Vault".into(),
        more: 1,
    };
    assert_eq!(
        step_words(&hunt, 0, &place_of),
        "hunting Engorged Eater in Remote Empyrean Vault (1 to go)"
    );
    let wait = PursuitStep::Waiting {
        until: 5 * 3_600 + 12 * 60,
    };
    assert_eq!(step_words(&wait, 0, &place_of), "waits 5h 12m");
    let stuck = PursuitStep::Stuck {
        why: "nobody known gives the eaterpit flag".into(),
    };
    assert_eq!(
        step_words(&stuck, 0, &place_of),
        "passed over: nobody known gives the eaterpit flag"
    );
    let line = PursuitLine {
        id: "ginsword".into(),
        title: "Sir Ginazio's Sword".into(),
        step: hunt,
        working: true,
        via: Some("Eater Pit".into()),
    };
    assert_eq!(
        line_words(&line, 0, &place_of),
        "working: hunting Engorged Eater in Remote Empyrean Vault (1 to go) (first, for Eater Pit)"
    );
}

#[test]
fn the_autoplay_section_is_one_line_and_a_button() {
    let mut cfg = Quests {
        on: true,
        ..Default::default()
    };
    assert_eq!(pursuit_summary(&[], &cfg, 0, &place_of), "nothing pursued");
    cfg.pursue_quest("engorgedeaterjaw", true, 0);
    let demo = demo(0);
    assert_eq!(
        pursuit_summary(&demo.pursuit, &cfg, 0, &place_of),
        "pursuing Augmentation Proving Ground Access: hunting Insatiable Eater in Insatiable \
         Vault (1 to go)"
    );
    assert_eq!(
        pursuit_summary(&demo.pursuit[1..], &cfg, 0, &place_of),
        "1 quest pursued; it can't go on now"
    );
    let before = cfg.clone();
    let ctx = egui::Context::default();
    let mut pressed = true;
    let mut out = ctx.run_ui(Default::default(), |ui| {
        pressed = summary(ui, &mut cfg, &demo.browse, &demo.pursuit, 0);
    });
    out.textures_delta.clear();
    assert!(!pressed, "nobody clicked the button");
    assert_eq!(cfg, before);
}

#[test]
fn a_frame_of_the_quest_book_draws_and_changes_nothing_unclicked() {
    let demo = demo(1_000);
    let cfg = Quests {
        on: true,
        pursue: vec![ac_client::autoplay::Pursuit {
            id: "engorgedeaterjaw".into(),
            again: true,
            since: 0,
        }],
        ..Default::default()
    };
    let log = QuestLog::default();
    let no_chain = |_: &str| Vec::new();
    let v = BookView {
        cards: &demo.cards,
        cfg: &cfg,
        acting: &demo.browse,
        pursuit: &demo.pursuit,
        me: Me {
            level: 85,
            margin: 10,
        },
        held: &[],
        log: &log,
        now: 1_000,
        chain: &no_chain,
        place: &place_of,
    };
    let mut st = BookState::showing("engorgedeaterjaw");
    st.browse
        .set(demo.browse.clone(), std::time::Instant::now());
    let ctx = egui::Context::default();
    for _ in 0..2 {
        let mut edited = Some(Quests::default());
        let mut out = ctx.run_ui(Default::default(), |_| {
            edited = draw(&ctx, &v, &mut st);
        });
        out.textures_delta.clear();
        assert_eq!(edited, None);
    }
    assert_eq!(st.selected.as_deref(), Some("engorgedeaterjaw"));
    assert_eq!(st.listed(&v).len(), demo.cards.len(), "all shown");
    st.filter.search = "giver:clemenza".into();
    assert_eq!(st.listed(&v).len(), 1, "a new search lists again");
}
