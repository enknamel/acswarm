use std::collections::BTreeMap;

use ac_world::portals::Portal;
use ac_world::quests::{Kind, Source};
use glam::Vec3;

use super::*;
use crate::quest_log::{considered_rows, quest_board};

const NOW: u64 = 1_000_000;

/// Chains a test writes out: what `quest_chain` will answer once its data is in.
#[derive(Default)]
struct Fake {
    chains: BTreeMap<&'static str, Vec<ChainLink>>,
    grounds: Vec<(Want, SourceGround)>,
    givers: BTreeMap<&'static str, Vec<FlagBy>>,
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
        margin: 5,
        config,
        now: NOW,
        ..QuestFacts::default()
    }
}

/// The pursuit list worked out as `refresh_quests` and `refresh_pursuit` do, with `fake`'s chains.
fn work(facts: &QuestFacts, fake: &dyn Chains, led: Option<&str>) -> Pursued {
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
    // Level 50 is below the Engorged Eater's 80 by more than twice the margin: it would die
    // there each try, so it is passed over saying why; 75 is within it and goes.
    let p = work(&facts(50, &[("engorgedeaterjaw", false)]), &jaws(), None);
    assert_eq!(
        why(&p.lines[0].step),
        "its easiest source is level 80; this character is 50"
    );
    let p = work(&facts(75, &[("engorgedeaterjaw", false)]), &jaws(), None);
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
