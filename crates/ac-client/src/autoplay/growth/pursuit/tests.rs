use std::collections::BTreeMap;

use ac_world::portals::Portal;
use ac_world::quests::{Kind, Source};
use glam::Vec3;
use std::time::{Duration, Instant};

use super::*;
use crate::quest_log::{considered_rows, quest_board};

const NOW: u64 = 1_000_000;

/// Chains a test writes out: what `quest_chain` will answer once its data is in.
#[derive(Default)]
struct Fake {
    chains: BTreeMap<&'static str, Vec<ChainLink>>,
    grounds: Vec<(Want, SourceGround)>,
    givers: BTreeMap<&'static str, Vec<FlagBy>>,
    /// Sources standing in for the data's: a want listed here has only these.
    sources: Vec<&'static Source>,
}

impl Chains for Fake {
    fn chain(&self, id: &str) -> Vec<ChainLink> {
        self.chains.get(id).cloned().unwrap_or_default()
    }

    fn grounds(&self, want: &Want) -> Vec<SourceGround> {
        let of = self.grounds.iter().filter(|(w, _)| w == want);
        of.map(|(_, g)| g.clone()).collect()
    }

    fn givers(&self, flag: &str) -> Vec<FlagBy> {
        self.givers.get(flag).cloned().unwrap_or_default()
    }

    fn sources(&self, want: &Want) -> Vec<&'static Source> {
        let own: Vec<&'static Source> = self
            .sources
            .iter()
            .copied()
            .filter(|s| s.want == *want)
            .collect();
        if own.is_empty() {
            ac_world::quests::sources_of(want).collect()
        } else {
            own
        }
    }
}

fn leak<T>(t: T) -> &'static T {
    Box::leak(Box::new(t))
}

/// A creature dropping `want` every kill.
fn dropper(want: Want, wcid: u32, name: &str, level: u32) -> &'static Source {
    leak(Source {
        want,
        kind: Yield::Drop,
        wcid,
        name: name.to_string(),
        level,
        chance: 1.0,
        landblocks: Vec::new(),
        places: Vec::new(),
        keys: Vec::new(),
    })
}

/// A portal into `lb` at `min_level`, asking `flag` ("" for none).
fn way_in(name: &str, lb: u32, min_level: u32, flag: &str) -> &'static Portal {
    let to_cell = lb << 16 | 0x0100;
    leak(Portal {
        name: name.to_string(),
        from_cell: 0x24EE_0100,
        from: Vec3::ZERO,
        to_cell,
        to: ac_world::landblock_origin(to_cell),
        min_level,
        max_level: 0,
        quest: flag.to_string(),
        usable: 32,
        bitmask: 0,
    })
}

/// `source`'s ground in the dungeon `lb`, reached by `portal`.
fn ground(lb: u32, source: &'static Source, portal: &'static Portal) -> SourceGround {
    SourceGround {
        landblock: lb,
        source,
        entries: 4,
        place: Some(Place {
            at: portal.to,
            cell: portal.to_cell,
        }),
        way_in: Some(portal),
    }
}

/// The jaw quests' sources, as `quest_sources.csv` has them, behind open portals.
fn jaws() -> Fake {
    let engorged = dropper(Want::Item(28725), 28849, "Engorged Eater", 80);
    let voracious = dropper(Want::Item(28726), 28850, "Voracious Eater", 100);
    let remote = way_in("Remote Empyrean Vault", 0x0001, 40, "");
    let ruined = way_in("Ruined Empyrean Vault", 0x0194, 70, "");
    Fake {
        grounds: vec![
            (Want::Item(28725), ground(0x0001, engorged, remote)),
            (Want::Item(28726), ground(0x0194, voracious, ruined)),
        ],
        ..Fake::default()
    }
}

fn facts(level: u32, pursue: &[(&str, bool)]) -> QuestFacts {
    let mut config = crate::autoplay::Quests::default();
    for &(id, again) in pursue {
        config.pursue.push(Pursuit {
            id: id.to_string(),
            again,
            since: NOW - 10,
        });
    }
    QuestFacts {
        level,
        config,
        now: NOW,
        ..QuestFacts::default()
    }
}

/// The pursuit list worked out as `refresh_quests` and `refresh_pursuit` do, with `fake`'s chains.
fn work(facts: &QuestFacts, fake: &dyn Chains, led: Option<&str>) -> Pursued {
    work_dropping(facts, fake, led, &|_| false)
}

/// [`work`], with the sources `dropped` says were refused lately left out.
fn work_dropping(
    facts: &QuestFacts,
    fake: &dyn Chains,
    led: Option<&str>,
    dropped: &dyn Fn(&Source) -> bool,
) -> Pursued {
    let holds = |f: &str| crate::quest_log::holds(facts, f);
    let chained = with_chain(&facts.config.pursue, &holds, fake);
    let mut f = facts.clone();
    f.config.pursue = chained.iter().map(|c| c.pursuit.clone()).collect();
    let board = quest_board(considered_rows(&f.config), &f);
    let carried = |w| f.carried.get(&w).copied().unwrap_or(0);
    let asking = Asking {
        level: f.level,
        held: &f.held,
        carried: &carried,
        refused: &|_, _| false,
        away: &|_| 0.0,
    };
    let held = BTreeMap::new();
    let ask = Ask {
        facts: &f,
        asking: &asking,
        tried: &|_: &Unlock| false,
        held: &held,
        led,
        chains: fake,
        dropped,
        unlocked: &|_| false,
    };
    pursuit_of(&board, &chained, &ask)
}

fn steps(p: &Pursued) -> Vec<(&str, &PursuitStep, bool)> {
    let lines = p.lines.iter();
    lines.map(|l| (l.id.as_str(), &l.step, l.working)).collect()
}

fn why(step: &PursuitStep) -> &str {
    match step {
        PursuitStep::Stuck { why } => why,
        other => panic!("not stuck: {other:?}"),
    }
}

#[test]
fn a_jaw_not_carried_is_hunted_where_it_drops() {
    // The Engorged Eater is level 80: a level 50 hunts it all the same, the game allowing it.
    let p = work(&facts(50, &[("engorgedeaterjaw", false)]), &jaws(), None);
    match steps(&p)[..] {
        [(
            "engorgedeaterjaw",
            PursuitStep::Hunt {
                landblock,
                creature,
                more,
                ..
            },
            true,
        )] => {
            assert_eq!(
                (*landblock, creature.as_str(), *more),
                (0x0001, "Engorged Eater", 1)
            );
        }
        ref other => panic!("{other:?}"),
    }
    assert_eq!(p.grounds.iter().map(|g| g.0).collect::<Vec<_>>(), [0x0001]);
    assert_eq!(p.sought, [28849], "the jaw's eater, not its namesake 28642");
}

#[test]
fn a_jaw_carried_is_handed_in() {
    let mut f = facts(85, &[("engorgedeaterjaw", false)]);
    f.carried.insert(28725, 1);
    let p = work(&f, &jaws(), None);
    match steps(&p)[..] {
        [(_, PursuitStep::HandIn { row, .. }, true)] => {
            assert_eq!(
                (row.giver, row.want.clone()),
                ("Clemenza", Want::Item(28725))
            );
        }
        ref other => panic!("{other:?}"),
    }
    assert!(p.grounds.is_empty() && p.sought.is_empty());
}

#[test]
fn a_kill_task_not_taken_up_is_taken_up_first() {
    // Xun Yu's fifty Repugnant Eaters: his talk stamps the counter the kills count under.
    let f = facts(85, &[("repugeaterkillcount", false)]);
    let p = work(&f, &Fake::default(), None);
    match steps(&p)[..] {
        [(_, PursuitStep::TakeUp { row, .. }, true)] => {
            assert_eq!((row.kind, row.giver), (Kind::Start, "Xun Yu"));
        }
        ref other => panic!("{other:?}"),
    }
}

#[test]
fn a_waiting_quest_passes_to_the_next() {
    let both = [("engorgedeaterjaw", true), ("voraciouseaterjaw", true)];
    let mut f = facts(110, &both);
    f.log.waits.insert("engorgedeaterjaw".into(), NOW + 3_600);
    let p = work(&f, &jaws(), None);
    match steps(&p)[..] {
        [(_, PursuitStep::Waiting { until }, false), (_, PursuitStep::Hunt { landblock, .. }, true)] =>
        {
            assert_eq!((*until, *landblock), (NOW + 3_600, 0x0194));
        }
        ref other => panic!("{other:?}"),
    }
    let words = p.lines[0].words(NOW);
    assert!(words.contains("waits 1h 0m"), "{words}");
}

#[test]
fn a_once_pursuit_handed_in_since_is_finished() {
    let row = ac_world::quests::rows_of("engorgedeaterjaw")
        .next()
        .unwrap();
    let mut f = facts(85, &[("engorgedeaterjaw", false)]);
    f.log.note_done(row, NOW - 20);
    // Handed in before it was asked for: this pursuit is still to do, after the wait.
    let p = work(&f, &jaws(), None);
    assert!(
        matches!(p.lines[0].step, PursuitStep::Waiting { .. }),
        "{p:?}"
    );
    f.log.note_done(row, NOW - 5);
    let p = work(&f, &jaws(), None);
    assert_eq!(p.lines[0].step, PursuitStep::Finished);
    // Again, the wait is kept and then it is done once more.
    f.config.pursue[0].again = true;
    let p = work(&f, &jaws(), None);
    assert!(
        matches!(p.lines[0].step, PursuitStep::Waiting { .. }),
        "{p:?}"
    );
}

#[test]
fn a_way_in_behind_a_flag_nobody_gives_says_so_and_moves_on() {
    let mut fake = jaws();
    let pit = way_in("Eater Pit", 0x0001, 40, "Eaterpit");
    fake.grounds[0].1 = ground(0x0001, fake.grounds[0].1.source, pit);
    let both = [("engorgedeaterjaw", false), ("voraciouseaterjaw", false)];
    let p = work(&facts(110, &both), &fake, None);
    assert_eq!(
        why(&p.lines[0].step),
        "nobody known gives the Eaterpit flag"
    );
    assert!(!p.lines[0].working);
    assert!(matches!(
        p.lines[1].step,
        PursuitStep::Hunt {
            landblock: 0x0194,
            ..
        }
    ));
    assert!(p.lines[1].working);
}

#[test]
fn a_flag_a_warden_gives_is_earned_first() {
    let warden = &ac_world::unlocks::all()[0];
    let mut fake = jaws();
    let gate = way_in("Warden's portal", 0x0001, 40, &warden.flag);
    fake.grounds[0].1 = ground(0x0001, fake.grounds[0].1.source, gate);
    let flag: &'static str = warden.flag.clone().leak();
    fake.givers = BTreeMap::from([(flag, vec![FlagBy::Unlock(warden)])]);
    let level = warden.min_level.max(85);
    let p = work(&facts(level, &[("engorgedeaterjaw", false)]), &fake, None);
    match &p.lines[0].step {
        PursuitStep::EarnFlag { flag, from } => {
            assert_eq!(flag, &warden.flag);
            assert_eq!(from.npc, warden.npc);
        }
        other => panic!("{other:?}"),
    }
    // Held, its case aside, the hunt goes on through the portal.
    let mut f = facts(level, &[("engorgedeaterjaw", false)]);
    f.held.push(warden.flag.to_ascii_uppercase());
    let p = work(&f, &fake, None);
    assert!(matches!(p.lines[0].step, PursuitStep::Hunt { .. }), "{p:?}");
}

/// Sir Ginazio's talk, which stamps the eaterpit flag (quest ginsword).
fn ginazio() -> &'static Row {
    let mut rows = ac_world::quests::rows_of("ginsword");
    rows.find(|r| r.kind == Kind::Talk).unwrap()
}

/// Fiun Rehlyun's jaw quest with a made-up way in behind the Eater Pit's flag, Sir Ginazio giving
/// it: a chain of three for the mechanics. The bundled data has none (see the test after).
fn fiun_behind_the_pit() -> Fake {
    let insatiable = dropper(Want::Item(28719), 28848, "Insatiable Eater", 160);
    let pit = way_in("Eater Pit", 0x0096, 125, "Eaterpit");
    let flag = ChainLink::Flag {
        flag: "Eaterpit".into(),
        by: vec![FlagBy::Quest(ginazio())],
        portal: Some(pit),
    };
    Fake {
        chains: BTreeMap::from([("augmentationprovinggroundaccess", vec![flag])]),
        grounds: vec![(Want::Item(28719), ground(0x0096, insatiable, pit))],
        givers: BTreeMap::from([("Eaterpit", vec![FlagBy::Quest(ginazio())])]),
        ..Fake::default()
    }
}

#[test]
fn a_quest_needing_another_works_that_one_first() {
    let fake = fiun_behind_the_pit();
    let fiun = "augmentationprovinggroundaccess";
    let p = work(&facts(150, &[(fiun, true)]), &fake, None);
    let ids: Vec<(&str, Option<&str>)> = p
        .lines
        .iter()
        .map(|l| (l.id.as_str(), l.via.as_deref()))
        .collect();
    assert_eq!(ids, [("ginsword", Some(fiun)), (fiun, None)]);
    assert!(p.lines[0].working);
    assert!(matches!(
        p.lines[0].step,
        PursuitStep::HandIn { row, .. } if std::ptr::eq(row, ginazio())
    ));
    let why = why(&p.lines[1].step);
    assert!(
        why.contains("the Eaterpit flag comes first, from Sir Ginazio"),
        "{why}"
    );
}

#[test]
fn a_follower_pursues_nothing() {
    let f = facts(85, &[("engorgedeaterjaw", false)]);
    let p = work(&f, &jaws(), Some("Blargerton"));
    assert_eq!(why(&p.lines[0].step), "following Blargerton; waiting");
    assert!(!p.lines.iter().any(|l| l.working) && p.grounds.is_empty());
}

#[test]
fn a_flagged_chain_walks_from_its_flag_to_the_jaw_and_round_again_each_wait() {
    // The made-up chain: flagged by Sir Ginazio, the jaw from the Insatiable Eaters, handed to
    // Fiun Rehlyun, whose wait is six days; asked for whenever it is ready.
    let fake = fiun_behind_the_pit();
    let fiun = "augmentationprovinggroundaccess";
    let mut f = facts(150, &[(fiun, true)]);
    let working = |p: &Pursued| {
        let l = p.lines.iter().find(|l| l.working)?;
        Some((l.id.clone(), l.step.clone()))
    };

    // Not flagged: Sir Ginazio is talked to first.
    let p = work(&f, &fake, None);
    let (id, step) = working(&p).expect("something worked");
    assert_eq!(id, "ginsword");
    let PursuitStep::HandIn { row: talk, .. } = step else {
        panic!("{step:?}");
    };

    // The talk heard (`quest_answered`): its wait begun, the flag held. The jaw is hunted.
    f.log.note_done(talk, NOW);
    f.held.extend(talk.stamps.iter().cloned());
    let p = work(&f, &fake, None);
    assert_eq!(
        p.lines.len(),
        1,
        "the flag held, its quest is not first: {p:?}"
    );
    assert!(
        matches!(
            &p.lines[0].step,
            PursuitStep::Hunt { landblock: 0x0096, creature, .. } if creature == "Insatiable Eater"
        ),
        "{p:?}"
    );
    assert_eq!(p.sought, [28848]);

    // Carried: handed to Fiun Rehlyun.
    f.carried.insert(28719, 1);
    let p = work(&f, &fake, None);
    let Some((_, PursuitStep::HandIn { row: hand_in, .. })) = working(&p) else {
        panic!("{p:?}");
    };
    assert_eq!(hand_in.giver, "Fiun Rehlyun");

    // Handed in: the jaw gone, six days to wait, and nothing worked meanwhile.
    f.carried.clear();
    f.log.note_done(hand_in, NOW + 60);
    let p = work(&f, &fake, None);
    let until = NOW + 60 + 518_400;
    assert_eq!(p.lines[0].step, PursuitStep::Waiting { until });
    assert!(working(&p).is_none());

    // The wait over: the jaw is hunted again, the flag still held.
    f.now = until + 1;
    let p = work(&f, &fake, None);
    assert!(
        matches!(
            working(&p),
            Some((
                _,
                PursuitStep::Hunt {
                    landblock: 0x0096,
                    ..
                }
            ))
        ),
        "{p:?}"
    );
}

#[test]
fn a_chained_quest_already_pursued_keeps_its_own_terms_ahead_of_the_one_needing_it() {
    let fake = fiun_behind_the_pit();
    let pursue = [
        Pursuit {
            id: "augmentationprovinggroundaccess".into(),
            again: true,
            since: 0,
        },
        Pursuit {
            id: "ginsword".into(),
            again: false,
            since: 7,
        },
    ];
    let chained = with_chain(&pursue, &|_| false, &fake);
    let got: Vec<(&str, bool, Option<&str>)> = chained
        .iter()
        .map(|c| (c.pursuit.id.as_str(), c.pursuit.again, c.via.as_deref()))
        .collect();
    assert_eq!(
        got,
        [
            ("ginsword", false, None),
            ("augmentationprovinggroundaccess", true, None)
        ]
    );
}

#[test]
fn a_quest_worked_first_waits_while_the_one_it_is_for_waits() {
    // Fiun's wait running and the Eater Pit's flag lost: Sir Ginazio is not talked to till it is due.
    let fake = fiun_behind_the_pit();
    let fiun = "augmentationprovinggroundaccess";
    let mut f = facts(150, &[(fiun, true)]);
    f.log.waits.insert(fiun.into(), NOW + 600);
    let p = work(&f, &fake, None);
    assert_eq!(p.lines[0].id, "ginsword");
    assert_eq!(p.lines[0].step, PursuitStep::Waiting { until: NOW + 600 });
    assert!(!p.lines.iter().any(|l| l.working), "{p:?}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn fiuns_jaw_on_the_bundled_data_is_hunted_with_no_quest_first() {
    // The Insatiable Eaters' dungeon (0x0096) is entered by the Insatiable Vault, level 125 and
    // no flag: pursuing Fiun's quest hunts the jaw at once, Sir Ginazio not involved.
    let fiun = "augmentationprovinggroundaccess";
    let p = work(&facts(150, &[(fiun, true)]), &ChainData, None);
    let ids: Vec<&str> = p.lines.iter().map(|l| l.id.as_str()).collect();
    assert_eq!(ids, [fiun], "{p:?}");
    assert!(
        matches!(&p.lines[0].step, PursuitStep::Hunt { creature, .. } if creature == "Insatiable Eater"),
        "{p:?}"
    );
    assert!(p.lines[0].working);
    assert!(p.sought.contains(&28848), "{:?}", p.sought);
    // Below the vault's level the reason is said.
    let p = work(&facts(110, &[(fiun, true)]), &ChainData, None);
    assert!(!p.lines[0].step.can_progress(), "{p:?}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_blood_shreth_kill_task_is_taken_up_hunted_handed_in_and_waits_on_the_bundled_data() {
    // Blargerton's: level 41, San Ming's ten Blood Shreth (level 8), twenty hours between.
    let id = "killtaskbloodshreth";
    let mut f = facts(41, &[(id, true)]);
    let p = work(&f, &ChainData, None);
    let PursuitStep::TakeUp { row, .. } = &p.lines[0].step else {
        panic!("{p:?}");
    };
    assert_eq!(row.giver, "San Ming");

    // Taken up (`quest_answered`): the counter held and at nothing. Blood Shreth are hunted.
    f.held.push(id.into());
    f.log.note_taken(id);
    let p = work(&f, &ChainData, None);
    assert!(
        matches!(&p.lines[0].step, PursuitStep::Hunt { creature, more: 10, .. } if creature == "Blood Shreth"),
        "{p:?}"
    );
    assert!(!p.grounds.is_empty() && p.sought.contains(&4110), "{p:?}");

    // Ten counted (`hear_kill_count`): handed in to San Ming.
    f.log.note_kills(id, 10);
    let p = work(&f, &ChainData, None);
    let PursuitStep::HandIn { row, .. } = &p.lines[0].step else {
        panic!("{p:?}");
    };
    assert_eq!((row.giver, row.kind), ("San Ming", Kind::Kill));

    // Handed in: the counter gone and twenty hours to wait.
    f.log.note_done(row, NOW);
    let p = work(&f, &ChainData, None);
    assert_eq!(
        p.lines[0].step,
        PursuitStep::Waiting {
            until: NOW + 72_000
        }
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn blargertons_saved_blood_shreth_task_is_hunted_at_login() {
    // His files at the crash: the counter held and at nothing, the task pursued again.
    let arwic = 0xC6A9_0016;
    let mut c =
        crate::testkit::standing_in_the_field(41, arwic, glam::Vec3::new(52.5, 126.0, 42.0));
    c.autoplay.config.quests.on = true;
    c.autoplay
        .config
        .quests
        .pursue_quest("killtaskbloodshreth", true, 0);
    c.travel.quests_done.push("killtaskbloodshreth".into());
    c.quest_log.note_taken("killtaskbloodshreth");
    c.refresh_quests(std::time::Instant::now());
    let line = c.pursuit.first().expect("the task pursued");
    assert!(
        matches!(line.step, PursuitStep::Hunt { .. }),
        "{:?} / board {:?}",
        line.step,
        c.quest_board.iter().find(|l| l.id == "killtaskbloodshreth")
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_ground_journey_set_off_before_the_quest_was_taken_up_gives_way_to_its_hunt() {
    // Live: the tick Shoichi's task was taken up, the board still said "take it up" and the XP
    // ground picker sent Blargerton toward the Viamontians; that walk was kept to its portal.
    let arwic = 0xC6A9_0016;
    let mut c =
        crate::testkit::standing_in_the_field(41, arwic, glam::Vec3::new(52.5, 126.0, 42.0));
    c.autoplay.config.quests.on = true;
    c.autoplay
        .config
        .quests
        .pursue_quest("killtaskbloodshreth", true, 0);
    c.travel.quests_done.push("killtaskbloodshreth".into());
    c.quest_log.note_taken("killtaskbloodshreth");
    let now = std::time::Instant::now();
    c.refresh_quests(now);
    let viamontians = (
        0x02EB,
        glam::Vec2::new(384.0, 45_100.0),
        "Viamontian Executioner".into(),
    );
    c.autoplay.growth.bound = Some(viamontians);
    let cfg = c.autoplay.config.growth.clone();
    assert!(c.grow_hunt(now, &cfg));
    let lb = c.autoplay.growth.bound.as_ref().map(|b| b.0);
    assert!(
        lb.is_some_and(|lb| c.pursued_place(lb).is_some()),
        "bound for {lb:x?}, the task's grounds {:x?}",
        c.autoplay.growth.pursuit_grounds
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_character_not_yet_placed_works_no_pursuit_and_holds_none() {
    // Live, at login: the board still from before the saved log, the body not yet placed, the
    // call on San Ming failed, and the task was passed over ten minutes for "no way to San Ming".
    let mut c = crate::testkit::character_of_level(crate::testkit::game_data(), 41);
    assert!(c.player.is_none() && c.world.player_guid.is_some());
    c.autoplay.config.enabled = true;
    c.autoplay.config.quests.on = true;
    c.autoplay
        .config
        .quests
        .pursue_quest("killtaskbloodshreth", true, 0);
    c.refresh_quests(std::time::Instant::now());
    assert!(c.pursuit_call().is_some(), "{:?}", c.pursuit);
    c.tick_autoplay(std::time::Instant::now());
    assert!(
        c.autoplay.growth.pursuit_held.is_empty(),
        "{:?}",
        c.autoplay.growth.pursuit_held
    );
    assert!(c.visiting().is_none());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_kill_task_is_hunted_on_the_ground_reached_soonest_not_the_one_with_most_spawns() {
    // Live, from Lin: the Black Spawn Den (64 Tusker Guards, 75 minutes away) was set off for;
    // the Tusker Barracks (49, about 3 minutes) is the soonest.
    let lin = 0xDA3B_0035;
    let mut c = crate::testkit::standing_in_the_field(41, lin, glam::Vec3::new(40.0, 120.0, 20.0));
    c.autoplay.config.quests.on = true;
    c.autoplay
        .config
        .quests
        .pursue_quest("tuskergkillcount", true, 0);
    c.travel.quests_done.push("tuskergkillcount".into());
    c.quest_log.note_taken("tuskergkillcount");
    let now = std::time::Instant::now();
    c.refresh_quests(now);
    let quests = c.travel.quests_done.clone();
    let soonest = c
        .autoplay
        .growth
        .pursuit_grounds
        .clone()
        .into_iter()
        .filter_map(|(lb, at)| c.journey_plan(at, &quests).map(|t| (t.seconds, lb)))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, lb)| lb);
    assert_eq!(soonest, Some(0x5A48), "the Tusker Barracks");
    let cfg = c.autoplay.config.growth.clone();
    assert!(c.grow_hunt(now, &cfg));
    assert_eq!(c.autoplay.growth.bound.as_ref().map(|b| b.0), soonest);
}

/// Bleeargh's Necklace of Leerargh (magicitemtraining3): the data's chest and key, and the key's
/// Scamp behind an open portal into its dungeon.
fn necklace() -> Fake {
    let scamp = dropper(Want::Item(27896), 27892, "Burun Ruuk Scamp", 40);
    let into = way_in("Mosswart Dungeon", 0x0177, 0, "");
    Fake {
        grounds: vec![(Want::Item(27896), ground(0x0177, scamp, into))],
        ..Fake::default()
    }
}

/// A `kind` source of the necklace standing at `at`, with no keys.
fn necklace_from(kind: Yield, wcid: u32, name: &str, at: Place) -> Source {
    Source {
        want: Want::Item(27902),
        kind,
        wcid,
        name: name.to_string(),
        level: 0,
        chance: 1.0,
        landblocks: vec![(at.cell, 1)],
        places: vec![at],
        keys: Vec::new(),
    }
}

#[test]
fn the_necklace_is_had_by_hunting_the_key_then_opening_the_chest_then_handing_it_in() {
    let fake = necklace();
    let mut f = facts(30, &[("magicitemtraining3", true)]);
    // No key: the Scamp that drops it is hunted, and the board keeps the key it brings.
    let p = work(&f, &fake, None);
    match &p.lines[0].step {
        PursuitStep::Hunt {
            landblock,
            creature,
            ..
        } => assert_eq!(
            (*landblock, creature.as_str()),
            (0x0177, "Burun Ruuk Scamp")
        ),
        other => panic!("{other:?}"),
    }
    assert_eq!(p.sought, [27892]);
    // The key carried: the chest, opened with it.
    f.carried.insert(27896, 1);
    let p = work(&f, &fake, None);
    match &p.lines[0].step {
        PursuitStep::OpenChest { chest, at, key } => {
            assert_eq!(
                (chest.wcid, at.cell, *key),
                (27890, 0x0177_0297, Some(27896))
            );
        }
        other => panic!("{other:?}"),
    }
    let words = p.lines[0].words(NOW);
    assert!(
        words.contains("open An old Chest with A Smelly Key"),
        "{words}"
    );
    // The necklace carried: handed in to Bleeargh.
    f.carried.insert(27902, 1);
    let p = work(&f, &fake, None);
    match &p.lines[0].step {
        PursuitStep::HandIn { row, .. } => {
            assert!(row.giver.starts_with("Bleeargh"), "{}", row.giver)
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_necklace_quest_wants_the_key_kept_while_the_necklace_is_wanted() {
    let f = facts(30, &[("magicitemtraining3", true)]);
    let board = quest_board(considered_rows(&f.config), &f);
    let wants = crate::quest_log::quest_wants(&board);
    let has = |w: u32| wants.iter().any(|q| q.wcid == w);
    assert!(has(27902) && has(27896), "{wants:?}");
}

#[test]
fn a_chest_refused_lately_is_left_out() {
    let mut f = facts(30, &[("magicitemtraining3", true)]);
    f.carried.insert(27896, 1);
    let chest = |s: &Source| s.kind == Yield::Container;
    let p = work_dropping(&f, &necklace(), None, &chest);
    assert!(
        !matches!(p.lines[0].step, PursuitStep::OpenChest { .. }),
        "{p:?}"
    );
}

#[test]
fn an_item_a_shop_sells_is_bought_on_a_town_run() {
    let at = Place {
        at: Vec3::new(100.0, 100.0, 0.0),
        cell: 0xA9B4_0019,
    };
    let fake = Fake {
        sources: vec![leak(necklace_from(Yield::Vendor, 9678, "Abia", at))],
        ..necklace()
    };
    let p = work(&facts(30, &[("magicitemtraining3", true)]), &fake, None);
    match &p.lines[0].step {
        PursuitStep::Buy {
            wcid, vendor, more, ..
        } => assert_eq!((*wcid, vendor.as_str(), *more), (27902, "Abia", 1)),
        other => panic!("{other:?}"),
    }
    assert!(p.lines[0].working);
}

#[test]
fn an_item_lying_on_the_floor_is_walked_to_and_picked_up() {
    let at = Place {
        at: Vec3::new(100.0, 100.0, 0.0),
        cell: 0x0177_0297,
    };
    let floor = necklace_from(Yield::Floor, 27902, "Necklace of Leerargh", at);
    let fake = Fake {
        sources: vec![leak(floor)],
        ..necklace()
    };
    let p = work(&facts(30, &[("magicitemtraining3", true)]), &fake, None);
    assert_eq!(
        p.lines[0].step,
        PursuitStep::PickUp {
            wcid: 27902,
            name: "Necklace of Leerargh".into(),
            at
        }
    );
}

const KEY: u32 = 0x8000_0101;
const CHEST: u32 = 0x7017_7039;

/// A level 30 at Holtburg working `step`'s pursuit, built from the place beside it.
fn fetching(step: impl Fn(Place) -> PursuitStep) -> (Client, PursuitLine) {
    let at = Vec3::new(80.0, 10.0, 0.0);
    let mut c = crate::testkit::standing_in_the_field(30, 0xA9B4_0019, at);
    let me = c.my_position().expect("standing");
    let beside = Place {
        at: me + Vec3::new(1.0, 0.0, 0.0),
        cell: 0xA9B4_0019,
    };
    let line = PursuitLine {
        id: "magicitemtraining3".into(),
        title: "Necklace".into(),
        step: step(beside),
        working: true,
        via: None,
    };
    c.pursuit = vec![line.clone()];
    (c, line)
}

/// Put an object of `wcid` in the world at `at`.
fn lying(c: &mut Client, guid: u32, wcid: u32, name: &str, at: Place) {
    c.world.objects.insert(
        guid,
        ac_world::WorldObject {
            guid,
            weenie_class_id: wcid,
            name: name.into(),
            position: crate::testkit::placed(at.cell, at.at),
            ..Default::default()
        },
    );
}

/// Carry one of `wcid` as `guid`.
fn carrying(c: &mut Client, guid: u32, wcid: u32, name: &str) {
    crate::testkit::thing_in_the_pack(c, guid, name, 1);
    c.world
        .objects
        .get_mut(&guid)
        .expect("packed")
        .weenie_class_id = wcid;
}

/// The data's chest, locked with the Smelly Key, standing at `at`.
fn old_chest(at: Place) -> PursuitStep {
    let chest = Source {
        keys: vec![(27896, "A Smelly Key".into())],
        ..necklace_from(Yield::Container, 27890, "An old Chest", at)
    };
    PursuitStep::OpenChest {
        chest: leak(chest),
        at,
        key: Some(27896),
    }
}

fn chest_at(line: &PursuitLine) -> Place {
    match line.step {
        PursuitStep::OpenChest { at, .. } => at,
        _ => unreachable!(),
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn at_the_chest_the_key_is_used_then_the_chest_opened_and_emptied_by_the_loot_rules() {
    let (mut c, line) = fetching(old_chest);
    lying(&mut c, CHEST, 27890, "An old Chest", chest_at(&line));
    carrying(&mut c, KEY, 27896, "A Smelly Key");
    let now = Instant::now();
    assert_eq!(c.grow_fetch(&line, now), Some(true));
    assert_eq!(c.last_used, Some(CHEST), "the key is used on the chest");
    let f = c
        .autoplay
        .growth
        .fetching
        .clone()
        .expect("waiting on the key");
    assert_eq!(f.act, fetch::Act::Unlock);
    assert!(c.is_unlocking(), "the unlock reflex holds the tick");
    // Unlocked, the one-use key used up (Lock.cs:69-90): the chest is opened for the loot rules.
    let later = now + Duration::from_millis(300);
    c.use_done = Some((0, later));
    c.world.objects.remove(&KEY);
    assert!(c.autoplay_unlock(later));
    assert!(
        c.autoplay.is_at_corpse(CHEST),
        "the loot rules have the chest"
    );
    // Emptied, the necklace in the pack: the board is worked out again for the hand-in.
    c.autoplay.let_go_of_corpse();
    carrying(&mut c, 0x8000_0102, 27902, "Necklace of Leerargh");
    c.quest_board_at = Some(later);
    assert_eq!(c.grow_fetch(&line, later), Some(true));
    assert_eq!(c.autoplay.growth.fetching, None);
    assert_eq!(c.quest_board_at, None);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_key_use_cancelled_is_sent_again_not_taken_for_an_unlock() {
    // Blargerton used the Smelly Key and cast at once: ACE's walk to the chest failed and its
    // UseDone had no error (Player_Use.cs:157-163), the key still in the pack, the chest locked.
    let (mut c, line) = fetching(old_chest);
    lying(&mut c, CHEST, 27890, "An old Chest", chest_at(&line));
    carrying(&mut c, KEY, 27896, "A Smelly Key");
    let now = Instant::now();
    assert!(c.autoplay_unlock(now) || c.grow_fetch(&line, now) == Some(true));
    let later = now + Duration::from_millis(300);
    c.use_done = Some((0, later));
    c.last_used = None;
    assert!(c.autoplay_unlock(later), "still waiting on the key");
    assert!(!c.autoplay.is_at_corpse(CHEST), "a locked chest opened");
    let gone = now + Duration::from_secs(6);
    assert!(c.autoplay_unlock(gone));
    assert_eq!(c.last_used, Some(CHEST), "the key is used again");
    let f = c
        .autoplay
        .growth
        .fetching
        .clone()
        .expect("waiting on the key");
    assert_eq!((f.act, f.tries), (fetch::Act::Unlock, 2));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_key_that_does_not_fit_drops_the_chest_for_the_pursuit() {
    let (mut c, line) = fetching(old_chest);
    lying(&mut c, CHEST, 27890, "An old Chest", chest_at(&line));
    carrying(&mut c, KEY, 27896, "A Smelly Key");
    let now = Instant::now();
    c.grow_fetch(&line, now);
    // KeyDoesntFitThisLock (Lock.cs:184-186).
    let later = now + Duration::from_millis(300);
    c.use_done = Some((0x04B2, later));
    assert_eq!(c.grow_fetch(&line, later), Some(false));
    let PursuitStep::OpenChest { chest, .. } = &line.step else {
        unreachable!()
    };
    assert!(c.is_source_dropped(chest, later));
    assert!(!c.autoplay.is_at_corpse(CHEST));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn an_item_on_the_floor_beside_the_character_is_picked_up() {
    const NECKLACE: u32 = 0x8000_0201;
    let (mut c, line) = fetching(|at| PursuitStep::PickUp {
        wcid: 27902,
        name: "Necklace of Leerargh".into(),
        at,
    });
    let PursuitStep::PickUp { at, .. } = line.step else {
        unreachable!()
    };
    lying(&mut c, NECKLACE, 27902, "Necklace of Leerargh", at);
    assert_eq!(c.grow_fetch(&line, Instant::now()), Some(true));
    assert!(c.loot_queue.contains(&NECKLACE), "asked to pick it up");
}

const NECKLACE_QUEST: &str = "magicitemtraining3";

/// The board `work` works the pursuit out over.
fn worked_board(facts: &QuestFacts, fake: &dyn Chains) -> Vec<QuestLine> {
    let holds = |flag: &str| crate::quest_log::holds(facts, flag);
    let chained = with_chain(&facts.config.pursue, &holds, fake);
    let mut f = facts.clone();
    f.config.pursue = chained.iter().map(|c| c.pursuit.clone()).collect();
    quest_board(considered_rows(&f.config), &f)
}

/// The necklace sold at Abia's counter, as well as lying in its chest.
fn necklace_sold() -> Fake {
    let at = Place {
        at: Vec3::new(100.0, 100.0, 0.0),
        cell: 0xA9B4_0019,
    };
    Fake {
        sources: vec![leak(necklace_from(Yield::Vendor, 9678, "Abia", at))],
        ..necklace()
    }
}

/// Fletch, reading a profile of its own: the starter's rules, the quest rule first, then the
/// player's own rule and another character's quest entry in the Quests section, and a player's
/// buy line filed there.
fn fletch_with_a_quests_section() -> Client {
    use crate::profile::{Ask, Buy, Mine, Rule, QUESTS};
    let mut c = crate::testkit::offline_client();
    c.world.stats.name = "Fletch".into();
    let mut rules = crate::profile::Profile::starter().rules;
    assert!(rules[0].all.contains(&Ask::Quest), "the quest rule first");
    let own = Rule {
        name: "my keys".into(),
        group: QUESTS.into(),
        all: vec![Ask::Item(crate::items::Term::Word("Key".into()))],
        ..Default::default()
    };
    let brynns = Rule {
        name: "Brynn's necklace".into(),
        group: QUESTS.into(),
        quest: NECKLACE_QUEST.into(),
        all: vec![Ask::Me(Mine::Name("Brynn".into()))],
        ..Default::default()
    };
    rules.insert(1, own);
    rules.insert(2, brynns);
    crate::testkit::with_a_profile(
        &mut c,
        "fletch-quests",
        rules,
        &[("Prismatic Taper", 100, 25)],
    );
    let mut p = (*c.loot_profile().expect("its profile")).clone();
    p.buy.push(Buy {
        what: "Mana Scarab".into(),
        keep: 5,
        on: true,
        group: QUESTS.into(),
        ..Default::default()
    });
    c.profiles.put(p).expect("put");
    c
}

/// Work the pursuit out for `c` over `f` and `fake`, and keep the Quests section, as
/// `refresh_pursuit` does.
fn pursue(c: &mut Client, f: &QuestFacts, fake: &Fake) {
    c.pursuit = work(f, fake, None).lines;
    c.quest_board = worked_board(f, fake);
    c.keep_quest_section(fake);
}

fn section(c: &Client) -> crate::profile::Profile {
    (*c.loot_profile().expect("its profile")).clone()
}

#[test]
fn a_pursued_quest_puts_its_items_key_and_buy_into_the_quests_section_marked_with_its_id() {
    use crate::items::{NumKey, Op, Term};
    use crate::profile::{Ask, Mine, QUESTS};
    let mut c = fletch_with_a_quests_section();
    pursue(
        &mut c,
        &facts(30, &[(NECKLACE_QUEST, true)]),
        &necklace_sold(),
    );
    let p = section(&c);
    let names: Vec<&str> = p.rules.iter().map(|r| r.name.as_str()).collect();
    let added: Vec<_> = p
        .rules
        .iter()
        .filter(|r| r.is_quest_entry("Fletch"))
        .collect();
    let wcids: Vec<(f64, Option<u32>)> = added
        .iter()
        .map(|r| {
            assert_eq!(
                (r.quest.as_str(), r.group.as_str()),
                (NECKLACE_QUEST, QUESTS)
            );
            assert!(
                r.all.contains(&Ask::Me(Mine::Name("Fletch".into()))),
                "{r:?}"
            );
            let class = r.all.iter().find_map(|a| match a {
                Ask::Item(Term::Num(NumKey::Class, Op::Eq, w)) => Some(*w),
                _ => None,
            });
            (class.expect("by weenie class"), r.keep_up_to)
        })
        .collect();
    // The necklace the hand-in takes, and the Smelly Key its chest asks while it is wanted.
    assert_eq!(wcids, [(27902.0, Some(1)), (27896.0, Some(1))], "{names:?}");
    // Filed after the section's last rule: the quest rule, the player's, Brynn's, then these.
    assert_eq!(names[..3], ["quest items", "my keys", "Brynn's necklace"]);
    assert!(p.rules[3].is_quest_entry("Fletch") && p.rules[4].is_quest_entry("Fletch"));
    let buys: Vec<_> = p
        .buy
        .iter()
        .filter(|b| b.is_quest_entry("Fletch"))
        .collect();
    assert_eq!(buys.len(), 1, "{:?}", p.buy);
    let b = buys[0];
    assert_eq!(
        (b.keep, b.restock_at, b.from.as_deref(), b.quest.as_str()),
        (1, Some(0), Some("Abia"), NECKLACE_QUEST)
    );
    assert_eq!(
        p.buy.last(),
        Some(b),
        "after the player's own line in the section"
    );
    // Worked out again with nothing new, nothing is written.
    let mut again = p.clone();
    let (rules, buy) =
        section::quest_entries(&c.pursuit, &c.quest_board, "Fletch", &necklace_sold());
    assert!(!again.set_quest_entries("Fletch", rules, buy));
}

#[test]
fn the_town_runs_needs_hold_a_pursued_quests_buy_line_urgent_at_its_counter() {
    use super::super::needs::NeedKind;
    let mut c = fletch_with_a_quests_section();
    let cfg = c.autoplay.config.growth.clone();
    let before = c.grow_needs(&cfg);
    pursue(
        &mut c,
        &facts(30, &[(NECKLACE_QUEST, true)]),
        &necklace_sold(),
    );
    let line = section(&c)
        .buy
        .into_iter()
        .find(|b| b.is_quest_entry("Fletch"))
        .expect("the quest's line");
    let needs = c.grow_needs(&cfg);
    let need = needs
        .iter()
        .find(|n| n.kind == NeedKind::Named(line.what.clone()))
        .unwrap_or_else(|| panic!("{:?} not among {needs:?}", line.what));
    assert_eq!((need.want, need.have, need.urgent), (1, 0, true));
    assert!(need.may_buy_at("Abia") && !need.may_buy_at("Thimble"));
    assert_eq!(
        needs.len(),
        before.len() + 1,
        "nothing else is bought: {needs:?}"
    );
}

#[test]
fn a_quest_handed_in_or_dropped_takes_out_only_what_its_pursuit_added() {
    let mut c = fletch_with_a_quests_section();
    let start = section(&c);
    let mut f = facts(30, &[(NECKLACE_QUEST, false)]);
    pursue(&mut c, &f, &necklace_sold());
    assert_ne!(section(&c), start);
    // Handed in since it was asked for: done, and its entries go; the player's and Brynn's stay.
    let hand_in = ac_world::quests::rows_of(NECKLACE_QUEST)
        .find(|r| r.kind == Kind::Item)
        .expect("its hand-in");
    f.log.note_done(hand_in, NOW - 5);
    pursue(&mut c, &f, &necklace_sold());
    assert_eq!(c.pursuit[0].step, PursuitStep::Finished);
    assert_eq!(section(&c), start);
    // Pursued again and then dropped: gone again.
    let f = facts(30, &[(NECKLACE_QUEST, true)]);
    pursue(&mut c, &f, &necklace_sold());
    assert_ne!(section(&c), start);
    c.pursuit.clear();
    c.keep_quest_section(&necklace_sold());
    assert_eq!(section(&c), start);
}

#[test]
fn the_necklaces_key_leaves_the_section_once_the_necklace_is_carried() {
    let mut c = fletch_with_a_quests_section();
    let mut f = facts(30, &[(NECKLACE_QUEST, true)]);
    f.carried.insert(27902, 1);
    pursue(&mut c, &f, &necklace());
    let p = section(&c);
    let added: Vec<&str> = p
        .rules
        .iter()
        .filter(|r| r.is_quest_entry("Fletch"))
        .map(|r| r.name.as_str())
        .collect();
    assert_eq!(added.len(), 1, "the necklace alone: {added:?}");
    assert!(
        !p.buy.iter().any(|b| b.is_quest_entry("Fletch")),
        "no shop sells it here"
    );
}
