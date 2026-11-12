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
        rates: Vec::new(),
        event: None,
        respawn: None,
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
        rate: ac_world::quests::SpawnRate {
            load: 4.0,
            per_hour: 40.0,
            others: 0.0,
        },
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
            ..Default::default()
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
    work_killing(facts, fake, led, dropped, &|_| Some(20.0))
}

/// [`work_dropping`], each kill taking what `kill_secs` says.
fn work_killing(
    facts: &QuestFacts,
    fake: &dyn Chains,
    led: Option<&str>,
    dropped: &dyn Fn(&Source) -> bool,
    kill_secs: &dyn Fn(u32) -> Option<f32>,
) -> Pursued {
    work_events(facts, fake, led, dropped, kill_secs, &|_| false)
}

/// [`work_killing`], the server events `event_on` says taken for on.
fn work_events(
    facts: &QuestFacts,
    fake: &dyn Chains,
    led: Option<&str>,
    dropped: &dyn Fn(&Source) -> bool,
    kill_secs: &dyn Fn(u32) -> Option<f32>,
    event_on: &dyn Fn(&str) -> bool,
) -> Pursued {
    let arena = arena::ArenaView::default();
    work_in(facts, fake, led, dropped, kill_secs, event_on, &arena)
}

/// [`work_events`], the arena run's stages read from `arena`.
fn work_in(
    facts: &QuestFacts,
    fake: &dyn Chains,
    led: Option<&str>,
    dropped: &dyn Fn(&Source) -> bool,
    kill_secs: &dyn Fn(u32) -> Option<f32>,
    event_on: &dyn Fn(&str) -> bool,
    arena: &arena::ArenaView,
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
        ways: &BTreeMap::new(),
        gem_grounds: &Default::default(),
        flag_ways: &BTreeMap::new(),
        here: None,
        bound: None,
        kill_secs,
        event_on,
        arena,
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
fn a_creature_it_cannot_kill_is_no_ground() {
    // Immune to every spell ready, or past a tier above the highest killed: the Engorged Eater's
    // vault is no ground, and the step says why.
    let f = facts(50, &[("engorgedeaterjaw", false)]);
    let none = |w: u32| (w != 28849).then_some(20.0);
    let p = work_killing(&f, &jaws(), None, &|_| false, &none);
    assert!(p.grounds.is_empty(), "{:?}", p.grounds);
    assert!(
        why(&p.lines[0].step).contains("cannot kill"),
        "{:?}",
        p.lines[0]
    );
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

#[test]
fn a_way_in_behind_a_flag_a_kill_stamps_hunts_that_kill_first() {
    let mut fake = jaws();
    let pit = way_in("Eater Pit", 0x0001, 40, "Eaterpit");
    fake.grounds[0].1 = ground(0x0001, fake.grounds[0].1.source, pit);
    let want = Want::Flag("eaterpit".into());
    let guardian = leak(Source {
        want: want.clone(),
        kind: Yield::Kill,
        ..dropper(Want::Nothing, 10824, "Guardian of the Singularity", 135).clone()
    });
    let hall = way_in("Singularity Bore", 0x0291, 50, "");
    fake.grounds.push((want, ground(0x0291, guardian, hall)));
    fake.givers = BTreeMap::from([("Eaterpit", vec![FlagBy::Kill(guardian)])]);
    let p = work(&facts(140, &[("engorgedeaterjaw", false)]), &fake, None);
    assert!(
        matches!(
            p.lines[0].step,
            PursuitStep::Hunt {
                landblock: 0x0291,
                ..
            }
        ),
        "{p:?}"
    );
    // Held, the hunt goes on through the portal.
    let mut f = facts(140, &[("engorgedeaterjaw", false)]);
    f.held.push("Eaterpit".into());
    let p = work(&f, &fake, None);
    assert!(
        matches!(
            p.lines[0].step,
            PursuitStep::Hunt {
                landblock: 0x0001,
                ..
            }
        ),
        "{p:?}"
    );
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
            ..Default::default()
        },
        Pursuit {
            id: "ginsword".into(),
            again: false,
            since: 7,
            ..Default::default()
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
    // Tier 3 killed: the Tusker Guards (tier 4) are one past it, as a pursuit may hunt.
    c.autoplay.growth.proven_tier = 3;
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
        .filter_map(|(lb, at)| {
            let there = c.pursued_pace(lb)?.pace.soonest();
            c.journey_plan(at, &quests).map(|t| (t.seconds + there, lb))
        })
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
        rates: Vec::new(),
        event: None,
        respawn: None,
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
        rates: Vec::new(),
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
    // The UseDone, which frees the slot the unlock held (session/apply.rs).
    c.use_done = Some((0, later));
    c.autoplay.cast_answered();
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

/// Mark `c`'s board worked out this session, as `refresh_quests` does, with nothing watched.
fn board_built(c: &mut Client) {
    c.quest_board_for = Some(crate::quest_log::BoardFor {
        quests: c.autoplay.config.quests.clone(),
        level: c.world.stats.level.max(0) as u32,
        log: Default::default(),
        held: Vec::new(),
        name: c.world.stats.name.trim().to_string(),
        spells: (0, 0),
        carried: Vec::new(),
    });
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
    board_built(&mut c);
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

const FOCUSING_QUEST: &str = "give:8904";

/// The Focusing Stone quest: the plain stone's Lich Overseer behind an open portal into 0x0188;
/// the imbued one comes from the data's Vortex.
fn focusing_stone() -> Fake {
    let lich = dropper(Want::Item(8903), 4124, "Lich Overseer", 50);
    let into = way_in("Lich Tower", 0x0188, 0, "");
    Fake {
        grounds: vec![(Want::Item(8903), ground(0x0188, lich, into))],
        ..Fake::default()
    }
}

#[test]
fn the_imbued_focusing_stone_is_had_by_hunting_the_plain_one_then_using_it_on_the_vortex() {
    let fake = focusing_stone();
    let mut f = facts(50, &[(FOCUSING_QUEST, true)]);
    // No plain stone: the Lich Overseer that drops it is hunted.
    let p = work(&f, &fake, None);
    match &p.lines[0].step {
        PursuitStep::Hunt {
            landblock,
            creature,
            ..
        } => assert_eq!((*landblock, creature.as_str()), (0x0188, "Lich Overseer")),
        other => panic!("{other:?}"),
    }
    assert_eq!(p.sought, [4124]);
    // The plain stone carried: it is used on the Vortex.
    f.carried.insert(8903, 1);
    let p = work(&f, &fake, None);
    match &p.lines[0].step {
        PursuitStep::UseOn { source, at, with } => {
            assert_eq!(
                (source.wcid, source.kind, at.cell, *with),
                (8907, Yield::Use, 0x02AB_010A, 8903)
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(p.lines[0].working);
    let words = p.lines[0].words(NOW);
    assert!(words.contains("use Focusing Stone on Vortex"), "{words}");
}

#[test]
fn the_focusing_stone_quest_keeps_the_plain_stone_while_the_imbued_one_is_wanted() {
    use crate::items::{NumKey, Op, Term};
    use crate::profile::Ask;
    let mut c = fletch_with_a_quests_section();
    pursue(
        &mut c,
        &facts(50, &[(FOCUSING_QUEST, true)]),
        &focusing_stone(),
    );
    let classes: Vec<f64> = section(&c)
        .rules
        .iter()
        .filter(|r| r.is_quest_entry("Fletch"))
        .filter_map(|r| {
            r.all.iter().find_map(|a| match a {
                Ask::Item(Term::Num(NumKey::Class, Op::Eq, w)) => Some(*w),
                _ => None,
            })
        })
        .collect();
    // The imbued stone the hand-in takes, and the plain one used on the Vortex to make it.
    assert_eq!(classes, [8904.0, 8903.0]);
}

/// [`facts`] pursuing `id` once, to end as `end`.
fn facts_ending(level: u32, id: &str, end: QuestEnd) -> QuestFacts {
    let mut f = facts(level, &[(id, false)]);
    f.config.pursue[0].outcome = end;
    f
}

#[test]
fn a_kept_focusing_stone_is_hunted_and_made_at_the_vortex_and_never_handed_in() {
    let fake = focusing_stone();
    let mut f = facts_ending(50, FOCUSING_QUEST, QuestEnd::Keep);
    let p = work(&f, &fake, None);
    assert!(
        matches!(&p.lines[0].step, PursuitStep::Hunt { creature, .. } if creature == "Lich Overseer"),
        "{:?}",
        p.lines[0].step
    );
    f.carried.insert(8903, 1);
    let p = work(&f, &fake, None);
    assert!(
        matches!(&p.lines[0].step, PursuitStep::UseOn { with: 8903, .. }),
        "{:?}",
        p.lines[0].step
    );
    // The imbued stone carried: kept, not handed in, where a hand-in would go to Le-Ai Rea.
    f.carried.insert(8904, 1);
    let p = work(&f, &fake, None);
    assert_eq!(p.lines[0].step, PursuitStep::Finished);
    assert!(!p.lines[0].working);
    f.config.pursue[0].outcome = QuestEnd::HandIn;
    let p = work(&f, &fake, None);
    match &p.lines[0].step {
        PursuitStep::HandIn { row, .. } => assert_eq!(row.giver_wcid, 4122),
        other => panic!("{other:?}"),
    }
}

/// [`fletch_with_a_quests_section`] in the world, pursuing the Focusing Stone to keep it.
fn fletch_keeping_the_stone() -> Client {
    let mut c = fletch_with_a_quests_section();
    c.world.player_guid = Some(crate::testkit::ME);
    c.world.stats.level = 50;
    c.ledger_loaded = true;
    let quests = &mut c.autoplay.config.quests;
    quests.pursue_quest(FOCUSING_QUEST, false, NOW);
    assert!(quests.set_outcome(FOCUSING_QUEST, QuestEnd::Keep));
    c
}

fn kept_rules(c: &Client) -> Vec<crate::profile::Rule> {
    let rules = section(c).rules.into_iter();
    rules.filter(|r| r.name.starts_with("Kept: ")).collect()
}

#[test]
fn a_kept_focusing_stone_carried_ends_the_pursuit_and_keeps_the_stone_in_the_players_rules() {
    use crate::items::{NumKey, Op, Term};
    use crate::profile::{Ask, LootAction, QUESTS};
    let mut c = fletch_keeping_the_stone();
    let now = Instant::now();
    c.refresh_quests(now);
    assert!(
        c.autoplay.config.quests.is_pursued(FOCUSING_QUEST),
        "nothing carried yet"
    );
    assert!(kept_rules(&c).is_empty());
    carrying(&mut c, 0x8000_0302, 8904, "Focusing Stone");
    c.refresh_quests(now);
    let quests = &c.autoplay.config.quests;
    assert!(!quests.is_pursued(FOCUSING_QUEST) && !quests.is_on(FOCUSING_QUEST));
    assert!(
        !c.pursuit
            .iter()
            .any(|l| matches!(l.step, PursuitStep::HandIn { .. })),
        "{:?}",
        c.pursuit
    );
    let kept = kept_rules(&c);
    assert_eq!(kept.len(), 1, "{kept:?}");
    let r = &kept[0];
    assert_eq!(
        (
            r.name.as_str(),
            r.action,
            r.keep_up_to,
            r.group.as_str(),
            r.quest.as_str()
        ),
        ("Kept: Focusing Stone", LootAction::Keep, Some(1), "", "")
    );
    assert_eq!(r.all, [Ask::Item(Term::Num(NumKey::Class, Op::Eq, 8904.0))]);
    // Filed after the Quests section, ahead of the rules that sell.
    let rules = section(&c).rules;
    let at = rules.iter().position(|x| x == r).unwrap();
    assert_eq!(rules[at - 1].group, QUESTS);
    assert!(rules[at + 1..].iter().all(|x| x.group != QUESTS));
    // Kept again: the rule is not written twice.
    let quests = &mut c.autoplay.config.quests;
    quests.pursue_quest(FOCUSING_QUEST, false, NOW);
    quests.set_outcome(FOCUSING_QUEST, QuestEnd::Keep);
    c.refresh_quests(now);
    assert!(!c.autoplay.config.quests.is_pursued(FOCUSING_QUEST));
    assert_eq!(kept_rules(&c).len(), 1);
}

const GEM_QUEST: &str = "anekshaygemofknowledgecollectedinamonth";

#[test]
fn a_hand_in_to_one_giver_goes_to_that_giver_alone() {
    let mut f = facts(275, &[(GEM_QUEST, false)]);
    f.carried.insert(47156, 1);
    let ends: Vec<QuestEnd> = crate::autoplay::quest_ends(GEM_QUEST)
        .into_iter()
        .filter(|e| matches!(e, QuestEnd::HandInTo { .. }))
        .collect();
    assert_eq!(ends.len(), 3, "three givers take the gem: {ends:?}");
    for end in ends {
        let QuestEnd::HandInTo { giver_wcid, .. } = end else {
            unreachable!()
        };
        f.config.pursue[0].outcome = end.clone();
        let p = work(&f, &Fake::default(), None);
        match &p.lines[0].step {
            PursuitStep::HandIn { row, .. } => assert_eq!(row.giver_wcid, giver_wcid),
            other => panic!("{end:?}: {other:?}"),
        }
    }
}

const STONE: u32 = 0x8000_0301;
const VORTEX: u32 = 0x7AB0_1000;

/// The data's Vortex standing at `at`, the plain stone to be used on it.
fn vortex(at: Place) -> PursuitStep {
    let source = Source {
        want: Want::Item(8904),
        kind: Yield::Use,
        wcid: 8907,
        name: "Vortex".into(),
        level: 0,
        chance: 1.0,
        landblocks: vec![(at.cell, 1)],
        places: vec![at],
        keys: vec![(8903, "Focusing Stone".into())],
        rates: Vec::new(),
        event: None,
        respawn: None,
    };
    PursuitStep::UseOn {
        source: leak(source),
        at,
        with: 8903,
    }
}

/// Beside the Vortex, the Vortex in the world and the plain stone carried.
fn at_the_vortex() -> (Client, PursuitLine) {
    let (mut c, line) = fetching(vortex);
    let PursuitStep::UseOn { at, .. } = line.step else {
        unreachable!()
    };
    lying(&mut c, VORTEX, 8907, "Vortex", at);
    carrying(&mut c, STONE, 8903, "Focusing Stone");
    (c, line)
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn at_the_vortex_the_plain_stone_is_used_on_it_and_the_imbued_one_carried_is_done() {
    let (mut c, line) = at_the_vortex();
    let now = Instant::now();
    assert_eq!(c.grow_fetch(&line, now), Some(true));
    assert_eq!(c.last_used, Some(VORTEX), "the stone is used on the Vortex");
    let f = c
        .autoplay
        .growth
        .fetching
        .clone()
        .expect("waiting on the use");
    assert_eq!((f.act, f.tries, f.with), (fetch::Act::Use, 1, 8903));
    assert!(c.is_unlocking(), "the unlock reflex holds the tick");
    // Waiting: nothing is sent again.
    c.last_used = None;
    let soon = now + Duration::from_millis(300);
    assert!(c.autoplay_unlock(soon));
    assert_eq!(c.last_used, None);
    // The plain stone used up, the imbued one in the pack: the board is worked out again.
    c.world.objects.remove(&STONE);
    carrying(&mut c, 0x8000_0302, 8904, "Focusing Stone");
    c.quest_board_at = Some(soon);
    assert!(c.autoplay_unlock(soon));
    assert_eq!(c.autoplay.growth.fetching, None);
    assert_eq!(c.quest_board_at, None);
    assert!(!c.is_unlocking());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn an_unanswered_use_on_the_vortex_is_sent_again() {
    let (mut c, line) = at_the_vortex();
    let now = Instant::now();
    assert_eq!(c.grow_fetch(&line, now), Some(true));
    c.last_used = None;
    let gone = now + Duration::from_secs(6);
    assert!(c.autoplay_unlock(gone));
    assert_eq!(c.last_used, Some(VORTEX), "the stone is used again");
    let f = c
        .autoplay
        .growth
        .fetching
        .clone()
        .expect("waiting on the use");
    assert_eq!((f.act, f.tries), (fetch::Act::Use, 2));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_use_on_the_vortex_waits_for_peace_mode_first() {
    // Crafting is refused in any stance but peace (RecipeManager.cs:51-57).
    let (mut c, line) = at_the_vortex();
    c.combat = true;
    let now = Instant::now();
    assert_eq!(c.grow_fetch(&line, now), Some(true));
    assert!(!c.combat, "out of combat");
    assert_eq!(c.last_used, None, "nothing used in a combat stance");
    assert!(c.is_unlocking());
    let changed = now + Duration::from_millis(1100);
    assert!(c.autoplay_unlock(changed));
    assert_eq!(c.last_used, Some(VORTEX));
    let f = c
        .autoplay
        .growth
        .fetching
        .clone()
        .expect("waiting on the use");
    assert_eq!((f.act, f.tries), (fetch::Act::Use, 1));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_use_on_the_vortex_refused_drops_it_for_the_pursuit() {
    let (mut c, line) = at_the_vortex();
    let now = Instant::now();
    c.grow_fetch(&line, now);
    // YouDoNotPassCraftingRequirements, 0x0437 (RecipeManager.cs:76-80).
    let later = now + Duration::from_millis(300);
    c.use_done = Some((0x0437, later));
    assert_eq!(c.grow_fetch(&line, later), Some(false));
    let PursuitStep::UseOn { source, .. } = &line.step else {
        unreachable!()
    };
    assert!(c.is_source_dropped(source, later));
    assert!(!c.is_unlocking());
}

#[test]
fn the_stamped_letter_is_had_by_the_key_then_the_stamp_then_stamping_the_letter() {
    // The Seasoned Explorer Baton (give:70307:45958): Brentsella's Key from one chest, the
    // Rithwic Town Stamp from the locked one, then the stamp used on the Lucky Gold Letter.
    let mut f = facts(80, &[("give:70307:45958", true)]);
    f.carried.insert(45875, 1);
    let step = |f: &QuestFacts| work(f, &ChainData, None).lines[0].step.clone();
    match step(&f) {
        PursuitStep::OpenChest {
            chest, key: None, ..
        } => assert_eq!(chest.wcid, 4790),
        other => panic!("{other:?}"),
    }
    f.carried.insert(4793, 1);
    match step(&f) {
        PursuitStep::OpenChest { chest, key, .. } => {
            assert_eq!((chest.wcid, key), (4791, Some(4793)))
        }
        other => panic!("{other:?}"),
    }
    f.carried.insert(8738, 1);
    match step(&f) {
        PursuitStep::Combine { with, on, .. } => assert_eq!((with, on), (8738, 45875)),
        other => panic!("{other:?}"),
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_stamp_is_used_on_the_letter_in_the_pack_and_the_stamped_one_is_done() {
    const STAMP: u32 = 0x8000_0401;
    const LETTER: u32 = 0x8000_0402;
    let source = ac_world::quests::sources_of(&ac_world::quests::Want::Item(70307))
        .find(|s| s.kind == Yield::Combine)
        .expect("the stamping");
    let (mut c, line) = fetching(|_| PursuitStep::Combine {
        source,
        with: 8738,
        on: 45875,
    });
    carrying(&mut c, STAMP, 8738, "Rithwic Town Stamp");
    carrying(&mut c, LETTER, 45875, "Lucky Gold Letter");
    let now = Instant::now();
    c.combat = false;
    c.magic = false;
    assert_eq!(c.grow_fetch(&line, now), Some(true));
    assert_eq!(c.last_used, Some(LETTER), "the stamp is used on the letter");
    assert!(c.is_unlocking(), "the unlock reflex holds the tick");
    c.world.objects.remove(&LETTER);
    carrying(
        &mut c,
        0x8000_0403,
        70307,
        "Stamped Rithwic Lucky Gold Letter",
    );
    c.quest_board_at = Some(now);
    assert!(c.autoplay_unlock(now + Duration::from_millis(300)));
    assert_eq!(c.autoplay.growth.fetching, None);
    assert_eq!(c.quest_board_at, None, "the board is worked out again");
}

#[test]
fn the_samsur_letter_is_had_by_picking_up_the_stamp_then_stamping_the_letter() {
    // The Seasoned Explorer Boots (give:70309:45975): the Samsur Town Stamp lies in Musansayn's
    // Vaults, put there by its generator's link; used on the Lucky Gold Letter, it is handed in.
    let mut f = facts(51, &[("give:70309:45975", false)]);
    f.carried.insert(45875, 3);
    let step = |f: &QuestFacts| work(f, &ChainData, None).lines[0].step.clone();
    match step(&f) {
        PursuitStep::PickUp { wcid, at, .. } => {
            assert_eq!(wcid, 8739);
            assert_eq!(at.cell >> 16, 0x0155);
        }
        other => panic!("{other:?}"),
    }
    f.carried.insert(8739, 1);
    match step(&f) {
        PursuitStep::Combine { with, on, .. } => assert_eq!((with, on), (8739, 45875)),
        other => panic!("{other:?}"),
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_take_waits_for_the_walk_to_stop_and_is_sent_again_when_cancelled() {
    // The Samsur Town Stamp's take went out as the trip there arrived: the stop cancelled the
    // server's walk to it (ActionCancelled), and the stamp was given up after one try.
    const STAMP: u32 = 0x8044_B10C;
    let (mut c, line) = fetching(|at| PursuitStep::PickUp {
        wcid: 8739,
        name: "Samsur Town Stamp".into(),
        at,
    });
    let PursuitStep::PickUp { at, .. } = line.step else {
        unreachable!()
    };
    lying(&mut c, STAMP, 8739, "Samsur Town Stamp", at);
    let now = Instant::now();
    let me = c.my_position().unwrap();
    let _ = c.go_to(
        crate::body::going::Owner::Step("grow"),
        me + Vec3::new(5.0, 0.0, 0.0),
        0.5,
    );
    assert!(c.is_walking(), "a walk under way");
    assert_eq!(c.grow_fetch(&line, now), Some(true));
    assert!(!c.loot_queue.contains(&STAMP), "taken while still walking");
    assert!(!c.is_walking(), "the walk was not stopped");
    let mut t = now;
    for round in 1..=fetch::UNLOCK_TRIES {
        t += Duration::from_millis(100);
        assert_eq!(c.grow_fetch(&line, t), Some(true), "round {round}");
        assert!(
            c.loot_queue.contains(&STAMP),
            "round {round}: not asked to pick it up"
        );
        c.loot_queue.clear();
        t += fetch::ANSWER_WAIT;
        // Cancelled: nothing came up, the stamp still lies there.
    }
    assert_eq!(c.grow_fetch(&line, t), Some(false), "tried for ever");
}

#[test]
fn the_nanto_letter_is_had_by_asking_shin_ro_for_the_key_then_the_chest_then_stamping() {
    // The Seasoned Explorer Gauntlets (give:70305:45963): Shin Ro hands the Water Temple key to
    // anyone who uses him, the key opens the chest holding the Nanto Town Stamp, and the stamp
    // goes on a Lucky Gold Letter. "Key comes from Shin Ro: autoplay cannot fetch it yet".
    let mut f = facts(51, &[("give:70305:45963", false)]);
    f.carried.insert(45875, 1);
    let step = |f: &QuestFacts| work(f, &ChainData, None).lines[0].step.clone();
    match step(&f) {
        PursuitStep::AskFor { source, at } => {
            assert_eq!((source.wcid, source.name.as_str()), (5121, "Shin Ro"));
            assert_eq!(at.cell >> 16, 0xE53E);
        }
        other => panic!("{other:?}"),
    }
    f.carried.insert(5757, 1);
    match step(&f) {
        PursuitStep::OpenChest { chest, key, .. } => {
            assert_eq!((chest.wcid, key), (5756, Some(5757)))
        }
        other => panic!("{other:?}"),
    }
    f.carried.insert(8737, 1);
    match step(&f) {
        PursuitStep::Combine { with, on, .. } => assert_eq!((with, on), (8737, 45875)),
        other => panic!("{other:?}"),
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn an_npc_is_used_for_the_item_and_its_arrival_ends_the_step() {
    let shin_ro = ChainData
        .sources(&Want::Item(5757))
        .into_iter()
        .find(|s| s.kind == Yield::Npc)
        .expect("Shin Ro gives the key");
    let (mut c, line) = fetching(|at| PursuitStep::AskFor {
        source: shin_ro,
        at,
    });
    let PursuitStep::AskFor { at, .. } = line.step else {
        unreachable!()
    };
    const NPC: u32 = 0x7AE5_3E01;
    lying(&mut c, NPC, 5121, "Shin Ro", at);
    let now = Instant::now();
    // A bane still in its recoil: the use would be turned away busy, so it waits, and says so.
    c.autoplay.cast_sent = Some(now);
    assert_eq!(c.grow_fetch(&line, now), Some(true));
    assert!(
        c.autoplay.growth.fetching.is_none(),
        "Shin Ro used mid-cast"
    );
    assert_eq!(c.autoplay.hands_wanted, Some(now));
    c.autoplay.cast_answered();
    assert_eq!(c.grow_fetch(&line, now), Some(true));
    assert!(
        c.autoplay.cast_in_flight(now),
        "the use does not hold the next cast"
    );
    c.autoplay.cast_answered();
    let f = c.autoplay.growth.fetching.clone().expect("nothing asked");
    assert_eq!(
        (f.act, f.guid),
        (fetch::Act::Talk, NPC),
        "Shin Ro was not used"
    );
    // Waiting on him: not used again at once.
    assert_eq!(
        c.grow_fetch(&line, now + Duration::from_secs(1)),
        Some(true)
    );
    assert_eq!(
        c.autoplay.growth.fetching.as_ref().map(|f| f.tries),
        Some(1)
    );
    // The key arrives: the step is done.
    carrying(&mut c, 0x8000_0500, 5757, "Key");
    assert_eq!(
        c.grow_fetch(&line, now + Duration::from_secs(2)),
        Some(true)
    );
    assert!(
        c.autoplay.growth.fetching.is_none(),
        "still waiting with the key carried"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn an_npc_is_not_asked_for_an_item_with_no_room_in_the_pack_for_it() {
    let shin_ro = ChainData
        .sources(&Want::Item(5757))
        .into_iter()
        .find(|s| s.kind == Yield::Npc)
        .expect("Shin Ro gives the key");
    let (mut c, line) = fetching(|at| PursuitStep::AskFor {
        source: shin_ro,
        at,
    });
    let PursuitStep::AskFor { at, .. } = line.step else {
        unreachable!()
    };
    lying(&mut c, 0x7AE5_3E01, 5121, "Shin Ro", at);
    // One slot, taken: no room for the key.
    crate::testkit::with_a_pack(&mut c, 1);
    carrying(&mut c, 0x8000_0501, 45875, "Lucky Gold Letter");
    assert_eq!(c.room_anywhere(), 0);
    assert_eq!(c.grow_fetch(&line, Instant::now()), Some(false));
    assert!(
        c.autoplay.growth.fetching.is_none(),
        "asked with no room for the key"
    );
}

/// A level 58 mage in Holtburg's field, named and with a profile of its own, pursuing `quest`.
fn mage_pursuing(quest: &str, profile: &str) -> Client {
    let holtburg = 0xA9B4_0011;
    let mut c =
        crate::testkit::standing_in_the_field(58, holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.name = "Blarg".into();
    c.ledger_loaded = true;
    crate::testkit::a_loot_profile(&mut c, profile);
    for school in [31, 32, 33, 34] {
        crate::testkit::trained_in(&mut c, school, 300);
    }
    // Blargerton's: tier 5 killed, so a pursuit's creatures up to tier 6 are hunted.
    c.autoplay.growth.proven_tier = 5;
    c.autoplay.config.quests.on = true;
    c.autoplay.config.quests.pursue_quest(quest, false, 0);
    c
}

/// `wcid` put in `c`'s pack as `guid`, or taken out of it with `count` 0.
fn in_the_pack(c: &mut Client, guid: u32, wcid: u32, name: &str, count: u32) {
    if count == 0 {
        c.world.objects.remove(&guid);
        return;
    }
    crate::testkit::thing_in_the_pack(c, guid, name, count);
    c.world.objects.get_mut(&guid).unwrap().weenie_class_id = wcid;
}

/// Work the board and the pursuit out again, as the next tick would.
fn again(c: &mut Client, now: std::time::Instant) -> PursuitStep {
    c.quest_board_at = None;
    c.refresh_quests(now);
    c.pursuit[0].step.clone()
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn an_item_whose_pickup_stamp_waits_is_waited_for_not_hunted() {
    // Blargerton killed a room of Putrid Moarsmen for the Ancient Key with its pickup stamp
    // (novquest1) 2.6 h off in his quest log: "You have solved this quest too recently!"
    let mut c = mage_pursuing("learn:2023", "recall-the-sanctuary");
    let now = std::time::Instant::now();
    let limit = ac_world::quests::pickup_limit(8511).expect("the Ancient Key's pickup limit");
    let until = crate::holdings::unix_now() + 9_000;
    c.quest_log.note_wait(limit.stamp, until);
    let step = again(&mut c, now);
    assert_eq!(step, PursuitStep::Waiting { until }, "{step:?}");
    c.quest_log.waits.clear();
    let step = again(&mut c, now);
    assert!(
        matches!(&step, PursuitStep::Hunt { creature, .. } if creature == "Putrid Moarsman"),
        "{step:?}"
    );
}

/// The Wedding Planner's invitation wanted for `learn:2023`, as `gem_way` wants it once the walk
/// over Ithaenc's shallows from the Freehold portal is refused live.
fn invitation_wanted(c: &mut Client) {
    let gem = ac_world::gems::all()
        .iter()
        .find(|g| g.name == "Invitation Ithaenc Cathedral")
        .expect("the invitation");
    let wanted = super::gem_way::GemWanted {
        quest: String::new(),
        wcid: gem.wcid,
        name: gem.name.clone(),
        had: false,
    };
    c.autoplay.growth.want_gem("learn:2023", wanted);
    c.quest_board_at = None;
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn recall_the_sanctuary_is_had_by_the_ancient_key_bought_invitation_and_scroll_read() {
    // The Healer's Heart quest, done for its recall: the Mistress' Chest in Ithaenc's cathedral
    // holds the scroll, opened with the Putrid Moarsman's Ancient Key. The island is waded to
    // from the Freehold portal; that walk refused, the Wedding Planner sells the invitation.
    let mut c = mage_pursuing("learn:2023", "recall-the-sanctuary");
    let now = std::time::Instant::now();
    c.refresh_quests(now);
    let hunt = c.pursuit[0].step.clone();
    assert!(
        matches!(&hunt, PursuitStep::Hunt { creature, .. } if creature == "Putrid Moarsman"),
        "{hunt:?}"
    );
    let quests = c.travel.quests_done.clone();
    let grounds = c.autoplay.growth.pursuit_grounds.clone();
    let walks = grounds.iter().filter_map(|g| c.journey_plan(g.1, &quests));
    assert!(walks.count() > 0, "no walk over the shallows");
    invitation_wanted(&mut c);
    assert!(c.autoplay.growth.pursuit_held.is_empty(), "passed over");
    // The invitation is the step now, and the Quests section buys it: a run goes for it.
    let step = again(&mut c, now);
    assert!(
        matches!(&step, PursuitStep::Buy { vendor, .. } if vendor == "Wedding Planner"),
        "{step:?}"
    );
    let p = (*c.loot_profile().unwrap()).clone();
    assert!(
        p.buy
            .iter()
            .any(|b| b.is_quest_entry("Blarg") && b.what.starts_with("Invitation")),
        "{:?}",
        p.buy
    );
    // Bought: the Moarsman again, the planner taking the invitation; used, it is wanted no more.
    in_the_pack(
        &mut c,
        0x5000_0001,
        14915,
        "Invitation Ithaenc Cathedral",
        1,
    );
    assert_eq!(again(&mut c, now), hunt);
    let quests = c.travel.quests_done.clone();
    let grounds = c.autoplay.growth.pursuit_grounds.clone();
    let trips: Vec<_> = grounds
        .iter()
        .map(|g| c.journey_plan(g.1, &quests))
        .collect();
    assert!(
        trips.iter().any(Option::is_some),
        "no journey with the invitation: {grounds:?}"
    );
    // And from the Wedding Planner's counter, where the run that bought it ends.
    crate::testkit::stand(&mut c, 0x7D64_014C, glam::Vec3::new(91.4, 35.7, 12.0));
    let trips: Vec<_> = grounds
        .iter()
        .map(|g| c.journey_plan(g.1, &quests))
        .collect();
    assert!(
        trips.iter().any(Option::is_some),
        "none from the Wedding Planner"
    );
    let cfg = c.autoplay.config.growth.clone();
    assert!(
        c.grow_hunt(now, &cfg),
        "{:?}",
        c.autoplay.growth.pursuit_held
    );
    c.cancel_travel();
    in_the_pack(&mut c, 0x5000_0001, 14915, "", 0);
    assert_eq!(again(&mut c, now), hunt);
    assert!(c.autoplay.growth.pursuit_gems.is_empty(), "another bought");
    // The scroll carried is read where it stands, and the spell known finishes the pursuit.
    in_the_pack(&mut c, 0x5000_0002, 8530, "Scroll of Sanctuary Recall", 1);
    let read = again(&mut c, now);
    assert!(
        matches!(read, PursuitStep::Read { spell: 2023, .. }),
        "{read:?}"
    );
    assert_eq!(c.grow_pursuit(now), Some(true), "walked off from the read");
    c.world.stats.spells.push(2023);
    c.refresh_quests(now);
    assert!(c.autoplay.config.quests.pursue.is_empty(), "still pursued");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_quests_section_keeps_every_item_the_chain_needs() {
    // The Virindi Essence was looked over in its chest and left: the section kept the hand-in and
    // chest keys only, not what the scepter's chain of quests and combines takes.
    let mut c = mage_pursuing("give:27841", "chained-quests");
    c.refresh_quests(std::time::Instant::now());
    let (rules, _) = section::quest_entries(&c.pursuit, &c.quest_board, "Blarg", &ChainData);
    let kept: Vec<f64> = rules
        .iter()
        .flat_map(|r| &r.all)
        .filter_map(|a| match a {
            crate::profile::Ask::Item(crate::items::Term::Num(
                crate::items::NumKey::Class,
                _,
                v,
            )) => Some(*v),
            _ => None,
        })
        .collect();
    for (wcid, name) in [
        (9125, "Virindi Essence"),
        (9117, "Tattered Virindi Cloak"),
        (6876, "Sturdy Iron Key"),
    ] {
        assert!(kept.contains(&(wcid as f64)), "{name} not kept: {kept:?}");
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_chest_whose_key_is_out_is_opened_not_another_key_hunted() {
    // Blargerton used the Ancient Key on the Mistress' Chest; the key left the pack half a second
    // before "The Mistress' Chest has been unlocked.", the board asked between, and he went down
    // the catacombs for another key.
    let mut c = mage_pursuing("learn:2023", "recall-the-sanctuary");
    let now = std::time::Instant::now();
    in_the_pack(&mut c, 0x5000_0003, 8511, "Ancient Key", 1);
    c.refresh_quests(now);
    let open = c.pursuit[0].step.clone();
    let PursuitStep::OpenChest {
        chest,
        key: Some(key),
        ..
    } = &open
    else {
        panic!("not to the chest with its key: {open:?}");
    };
    c.autoplay.growth.fetching = Some(super::Fetching {
        key: (8530, chest.wcid),
        guid: 0x7F41_8004,
        act: super::fetch::Act::Unlock,
        at: now,
        with: *key,
        tries: 1,
    });
    in_the_pack(&mut c, 0x5000_0003, 8511, "", 0);
    let step = again(&mut c, now);
    assert!(
        matches!(&step, PursuitStep::OpenChest { chest: s, .. } if s.wcid == chest.wcid),
        "another key hunted with the key out: {step:?}"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_copper_castellan_is_reached_by_the_gem_captain_ramelle_drops() {
    // Defense of Zaikhal: the Copper Legion Castellan stands in Corcima Castle's Copper Ward,
    // walked to from an Olthoi Tunnel's exit; that walk refused, the Copper Ward gem reaches it,
    // which Captain Ramelle and her bodyguards drop in the Copper Legion Keep by Zaikhal.
    let mut c = mage_pursuing("copperknightmedallion", "copper-castellan");
    let now = std::time::Instant::now();
    c.refresh_quests(now);
    let hunt = c.pursuit[0].step.clone();
    assert!(
        matches!(&hunt, PursuitStep::Hunt { creature, .. } if creature == "Copper Legion Castellan"),
        "{hunt:?}"
    );
    let cfg = c.autoplay.config.growth.clone();
    assert!(c.grow_hunt(now, &cfg), "no walk to the Copper Ward");
    c.cancel_travel();
    let gem = ac_world::gems::all()
        .iter()
        .find(|g| g.name == "Corcima Castle Copper Ward Portal Gem")
        .expect("the Copper Ward gem");
    let wanted = super::gem_way::GemWanted {
        quest: String::new(),
        wcid: gem.wcid,
        name: gem.name.clone(),
        had: false,
    };
    c.autoplay.growth.want_gem("copperknightmedallion", wanted);
    let step = again(&mut c, now);
    let PursuitStep::Hunt {
        landblock,
        creature,
        ..
    } = &step
    else {
        panic!("{step:?}")
    };
    assert_eq!(*landblock, 0x0018, "{creature}");
    assert!(c.grow_hunt(now, &cfg), "no way into the Copper Legion Keep");
    // Looted off her corpse: the Castellan's ground, passed over with no gem, is set off to now.
    c.cancel_travel();
    in_the_pack(
        &mut c,
        0x5000_0003,
        29444,
        "Corcima Castle Copper Ward Portal Gem",
        3,
    );
    assert_eq!(again(&mut c, now), hunt);
    assert!(
        c.grow_hunt(now, &cfg),
        "{:?}",
        c.autoplay.growth.pursuit_held
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_gem_bought_and_used_between_boards_is_no_errand_once_on_its_ground() {
    // Blargerton bought the invitation and used it before the board saw it carried: on the
    // Putrid Moarsman's ground the pursuit still asked for the gem, the ground was no longer its
    // own, and he left for another hunt and another invitation.
    let mut c = mage_pursuing("learn:2023", "gem-between-boards");
    let now = std::time::Instant::now();
    c.refresh_quests(now);
    let hunt = c.pursuit[0].step.clone();
    let cfg = c.autoplay.config.growth.clone();
    invitation_wanted(&mut c);
    assert!(matches!(again(&mut c, now), PursuitStep::Buy { .. }));
    // Through the invitation's portal and the catacombs', with no board between.
    crate::testkit::stand(&mut c, 0x02B7_02F1, glam::Vec3::new(70.0, -60.0, 0.005));
    assert_eq!(
        again(&mut c, now),
        hunt,
        "the gem still asked for on its ground"
    );
    // Walked on to the Moarsman's room at most, never off to another ground.
    c.grow_hunt(now, &cfg);
    let bound = c.autoplay.growth.bound_landblock();
    assert!(
        bound.is_none_or(|lb| lb == 0x02B7),
        "left its ground for {bound:x?}"
    );
    assert!(
        c.autoplay.growth.pursuit_gems.is_empty(),
        "the gem still wanted"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_quest_whose_ground_the_character_stands_on_is_worked_first() {
    // Logged in in Ithaenc's catacombs, Blargerton recalled out for the quest above it in the
    // list, then bought another invitation to come back for the Putrid Moarsman.
    let mut c = mage_pursuing("learn:2023", "worked-where-it-stands");
    c.autoplay
        .config
        .quests
        .pursue_quest("copperknightmedallion", false, 0);
    let now = std::time::Instant::now();
    c.refresh_quests(now);
    let working = |c: &Client| c.pursuit.iter().find(|l| l.working).map(|l| l.id.clone());
    assert_eq!(working(&c).as_deref(), Some("copperknightmedallion"));
    // Reached by the invitation, as only it reaches there: the gem wanted for it earlier.
    let gem = ac_world::gems::all()
        .iter()
        .find(|g| g.name == "Invitation Ithaenc Cathedral")
        .expect("the invitation");
    c.autoplay.growth.want_gem(
        "learn:2023",
        super::gem_way::GemWanted {
            quest: String::new(),
            wcid: gem.wcid,
            name: gem.name.clone(),
            had: false,
        },
    );
    crate::testkit::stand(&mut c, 0x02B7_02F1, glam::Vec3::new(70.0, -60.0, 0.005));
    again(&mut c, now);
    assert_eq!(
        working(&c).as_deref(),
        Some("learn:2023"),
        "left for another quest"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_gem_wanted_before_a_restart_is_still_bought_after_it() {
    // Restarted on the way to the Wedding Planner: the run set off for the invitation, the first
    // board rewrote the Quests section without it, and at the counter nothing was bought.
    let mut c = mage_pursuing("learn:2023", "gem-across-restarts");
    let now = std::time::Instant::now();
    c.refresh_quests(now);
    invitation_wanted(&mut c);
    assert!(matches!(again(&mut c, now), PursuitStep::Buy { .. }));
    let buys = |c: &Client| {
        let p = c.loot_profile().unwrap();
        p.buy
            .iter()
            .filter(|b| b.what.starts_with("Invitation"))
            .count()
    };
    assert_eq!(buys(&c), 1);
    // A new session: the wanted gems are gone, the saved section is not.
    c.autoplay.growth.pursuit_gems.clear();
    assert!(
        matches!(again(&mut c, now), PursuitStep::Buy { .. }),
        "no longer bought"
    );
    assert_eq!(buys(&c), 1, "the buy line went");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_gem_just_used_is_no_errand_on_the_way_to_its_ground() {
    // Through the invitation's portal into Ithaenc's ruins, the gem gone with the use: the town
    // run was short of it at once, and Lifestone Sending took him home to buy another.
    let mut c = mage_pursuing("learn:2023", "gem-on-the-road");
    let now = std::time::Instant::now();
    c.refresh_quests(now);
    let cfg = c.autoplay.config.growth.clone();
    invitation_wanted(&mut c);
    assert!(matches!(again(&mut c, now), PursuitStep::Buy { .. }));
    let invitation = |c: &Client| {
        c.grow_needs(&cfg)
            .iter()
            .any(|n| n.name.starts_with("Invitation"))
    };
    assert!(invitation(&c), "the invitation not wanted at the start");
    // Bought: the hunt is the step, the trip planner takes the gem.
    let gem = c.autoplay.growth.pursuit_gems[0].wcid;
    in_the_pack(&mut c, 0x8000_0700, gem, "Invitation Ithaenc Cathedral", 1);
    assert!(matches!(again(&mut c, now), PursuitStep::Hunt { .. }));
    // Used and landed by its portal, the trip on to the Moarsman's room under way, no board yet.
    in_the_pack(&mut c, 0x8000_0700, gem, "Invitation Ithaenc Cathedral", 0);
    crate::testkit::stand(&mut c, 0xF618_000C, glam::Vec3::new(40.0, 89.0, 182.005));
    let room = c.pursued_place(0x02B7).expect("the Moarsman's room");
    assert!(c.journey_to(room, 4.0), "no journey to the room");
    c.autoplay.growth.bound = Some((0x02B7, room.at.truncate(), "Putrid Moarsman".into()));
    assert!(!invitation(&c), "a run home for another invitation");
    // Logged in there, nothing under way yet: the town run's first look found it short, before
    // the hunt set off. From here a journey reaches the ground as things are.
    c.cancel_travel();
    c.autoplay.growth.bound = None;
    c.autoplay.growth.needs_seen = None;
    let needs = c.needs_now(now, &cfg);
    assert!(
        !needs.iter().any(|n| n.name.starts_with("Invitation")),
        "short of the invitation where it lands: {needs:?}"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_wanted_gem_bought_is_taken_up_at_once() {
    // The invitation bought, the town run ended before it reached the pack; the hunt went off
    // to another ground and cast Portal Recall, the board not due again for a while.
    let mut c = mage_pursuing("learn:2023", "gem-at-once");
    let now = std::time::Instant::now();
    c.refresh_quests(now);
    invitation_wanted(&mut c);
    assert!(matches!(again(&mut c, now), PursuitStep::Buy { .. }));
    let gem = c.autoplay.growth.pursuit_gems[0].wcid;
    in_the_pack(&mut c, 0x8000_0701, gem, "Invitation Ithaenc Cathedral", 1);
    c.refresh_quests(now + std::time::Duration::from_millis(500));
    assert!(
        matches!(c.pursuit[0].step, PursuitStep::Hunt { .. }),
        "{:?}",
        c.pursuit[0].step
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_chest_on_an_island_is_walked_to_over_its_shallows_not_dropped() {
    // The Ancient Key in hand, far from Ithaenc: the Mistress' Chest is waded to from the
    // Freehold portal (ACE walls off only a block all water), never dropped as a source.
    let mut c = mage_pursuing("learn:2023", "chest-by-gem");
    in_the_pack(&mut c, 0x8000_0702, 8511, "Ancient Key", 1);
    let now = std::time::Instant::now();
    c.refresh_quests(now);
    assert!(
        matches!(c.pursuit[0].step, PursuitStep::OpenChest { .. }),
        "{:?}",
        c.pursuit[0].step
    );
    let cfg = c.autoplay.config.growth.clone();
    assert!(c.grow_hunt(now, &cfg), "{}", c.autoplay.status);
    assert!(
        c.autoplay.status.contains("on the way to Mistress' Chest"),
        "{}",
        c.autoplay.status
    );
    assert!(
        matches!(again(&mut c, now), PursuitStep::OpenChest { .. }),
        "the chest dropped"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_sanatorium_chest_past_the_island_portal_has_virindiisland_had_first() {
    // The Asylum Chest Key in hand, no journey reaches the chest: the island's portals ask
    // VirindiIsland, stamped by the Guardian of the Singularity's death on its killer.
    let mut c = mage_pursuing("learn:2943", "caul-by-flag");
    c.world.stats.level = 135;
    c.autoplay.growth.proven_tier = 8;
    in_the_pack(&mut c, 0x8000_0702, 22923, "Asylum Chest Key", 1);
    let now = std::time::Instant::now();
    c.refresh_quests(now);
    assert!(
        matches!(c.pursuit[0].step, PursuitStep::OpenChest { .. }),
        "{:?}",
        c.pursuit[0].step
    );
    let cfg = c.autoplay.config.growth.clone();
    c.grow_hunt(now, &cfg);
    let wanted = c.autoplay.growth.pursuit_flags.get("learn:2943");
    assert_eq!(
        wanted.map(String::as_str),
        Some("VirindiIsland"),
        "{}",
        c.autoplay.status
    );
    match again(&mut c, now) {
        PursuitStep::Hunt {
            landblock,
            creature,
            ..
        } => {
            assert_eq!(landblock, 0x0291);
            assert_eq!(creature, "Guardian of the Singularity");
        }
        other => panic!("{other:?}"),
    }
    // Our killing blow holds the flag, and the chest is gone for again.
    let me = c.my_position().unwrap();
    let cell = c.player.as_ref().unwrap().cell;
    let mut guardian = crate::testkit::creature(0x5000_0901, "Guardian of the Singularity");
    guardian.weenie_class_id = 10824;
    guardian.position = crate::testkit::placed(cell, me);
    c.world.objects.insert(guardian.guid, guardian);
    c.note_sought_kill(me, now);
    assert!(c.holds_quest_flag("VirindiIsland"));
    // Ten portals from Holtburg, past any cap on their number.
    let chest = ac_world::Place {
        at: ac_world::landblock_origin(0x6048_01BB) + glam::Vec3::new(83.05, -21.49, -5.99),
        cell: 0x6048_01BB,
    };
    let quests = c.travel.quests_done.clone();
    let trip = c
        .journey_plan(chest, &quests)
        .expect("a journey to the chest");
    assert!(trip.portals() > 8, "{:?}", trip.steps);
    assert!(
        matches!(again(&mut c, now), PursuitStep::OpenChest { .. }),
        "{:?}",
        c.pursuit[0].step
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_sacrificial_dagger_on_aerlinthe_is_reached_by_a_note_handed_to_an_emissary() {
    // No fixed portal reaches Aerlinthe from the mainland: the dagger was dropped as a source,
    // "no way". An emissary handed a trade note summons the portal to the island.
    let mut c = mage_pursuing("give:40913", "aerfalle-by-note");
    c.world.stats.level = 66;
    let now = std::time::Instant::now();
    c.refresh_quests(now);
    assert!(
        matches!(&c.pursuit[0].step, PursuitStep::PickUp { name, .. } if name == "Sacrificial Dagger"),
        "{:?}",
        c.pursuit[0].step
    );
    let cfg = c.autoplay.config.growth.clone();
    c.grow_hunt(now, &cfg);
    let wanted: Vec<(u32, String)> = c
        .autoplay
        .growth
        .pursuit_gems
        .iter()
        .map(|g| (g.wcid, g.name.clone()))
        .collect();
    let [(note, name)] = &wanted[..] else {
        panic!("{wanted:?}: {}", c.autoplay.status);
    };
    assert!(name.starts_with("Trade Note"), "{name}");
    // Carried, the journey to the dagger goes by the emissary who takes it.
    in_the_pack(&mut c, 0x8000_0731, *note, name, 1);
    let dagger = ac_world::Place {
        at: ac_world::landblock_origin(0x02EE_025B) + glam::Vec3::new(147.65, -77.56, -41.93),
        cell: 0x02EE_025B,
    };
    let trip = c.plan_trip(dagger, false).expect("a journey to the dagger");
    let by = trip.steps.iter().find_map(|s| match s {
        ac_world::trip::Step::Npc { name, item, .. } => Some((name.clone(), *item)),
        _ => None,
    });
    let (emissary, item) = by.unwrap_or_else(|| panic!("{:?}", trip.steps));
    assert!(emissary.ends_with("the Emissary"), "{emissary}");
    assert_eq!(item, Some(*note));
    // The note is the way, not money the town run cashes.
    let cfg = crate::autoplay::growth::config::Growth::default();
    assert!(c.vendor_snapshot(&cfg).notes.is_empty());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_floor_item_taken_lately_is_waited_for_its_generator_s_respawn() {
    // At the Sacrificial Dagger's spot, Garaena's note spent to get there: no dagger, and it was
    // given up as "not there" in 5 s. Its generator puts it back 180 s after it is taken.
    let mut c = mage_pursuing("give:40913", "dagger-respawn");
    c.world.stats.level = 66;
    let at = ac_world::landblock_origin(0x02EE_025B) + Vec3::new(147.65, -77.56, -41.93);
    crate::testkit::stand(&mut c, 0x02EE_025B, Vec3::new(147.0, -77.0, -41.93));
    let now = std::time::Instant::now();
    c.refresh_quests(now);
    let line = c.pursuit[0].clone();
    assert!(
        matches!(&line.step, PursuitStep::PickUp { name, .. } if name == "Sacrificial Dagger"),
        "{:?}",
        line.step
    );
    assert_eq!(c.grow_fetch(&line, now), Some(true), "looked for");
    let later = now + Duration::from_secs(60);
    assert_eq!(
        c.grow_fetch(&line, later),
        Some(true),
        "given up before its respawn"
    );
    // Put back: taken at once.
    let mut dagger = crate::testkit::creature(0x8000_0751, "Sacrificial Dagger");
    dagger.weenie_class_id = 28065;
    dagger.position = crate::testkit::placed(0x02EE_025B, at);
    c.world.objects.insert(dagger.guid, dagger);
    let back = later + Duration::from_secs(1);
    assert_eq!(c.grow_fetch(&line, back), Some(true));
    let act = c.autoplay.growth.fetching.as_ref().map(|f| f.act);
    assert_eq!(act, Some(fetch::Act::Take), "not taken once back");
    // Never back: dropped once its respawn is out.
    c.world.objects.remove(&0x8000_0751);
    c.autoplay.growth.fetching = None;
    assert_eq!(c.grow_fetch(&line, back), Some(true), "looked for again");
    let out = back + Duration::from_secs(181);
    assert_eq!(c.grow_fetch(&line, out), Some(false), "kept waiting");
}

/// `mage_pursuing`'s mage with autoplay on, town runs and grounds on, no buy list of its own, and
/// `coin` pyreals carried.
fn mage_playing(quest: &str, profile: &str, coin: u32) -> Client {
    let mut c = mage_pursuing(quest, profile);
    c.autoplay.config.enabled = true;
    c.autoplay.config.growth.town_runs = true;
    c.autoplay.config.growth.hunt_grounds = true;
    // Only the quest's buys: the starter's own lines send a caster to its components first.
    let mut p = (*c.loot_profile().unwrap()).clone();
    p.buy.clear();
    c.profiles.put(p).unwrap();
    if coin > 0 {
        crate::testkit::coin_in_the_pack(&mut c, 0x8000_0900, coin);
    }
    c
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_gem_the_pursuit_buys_sets_the_run_off_that_frame_and_grow_sets_off_nothing() {
    // The invitation's buy line was written after the town run read its needs: the run waited
    // out the needs' 5 s cache, and grow recalled to another ground meanwhile.
    let mut c = mage_playing("learn:2023", "buy-handed-over", 200_000);
    let now = Instant::now();
    c.refresh_quests(now);
    let cfg = c.autoplay.config.growth.clone();
    // The run's look before the gem was wanted, then the walk over the shallows refused.
    c.needs_now(now, &cfg);
    invitation_wanted(&mut c);
    assert_eq!(c.autoplay.growth.pursuit_gems.len(), 1);
    c.tick_rules(now + Duration::from_millis(100));
    // Set off for the invitation (a giver on the way may be called on first).
    assert!(
        c.autoplay.growth.run.is_some(),
        "held back: {} ({:?})",
        c.autoplay.growth.held_back,
        c.pursuit[0].step
    );
    let buying: Vec<&str> = c
        .autoplay
        .growth
        .needs
        .iter()
        .map(|n| n.name.as_str())
        .collect();
    assert_eq!(buying, ["Invitation Ithaenc Cathedral"]);
    assert_eq!(c.autoplay.growth.bound, None, "grow set off for a ground");
    assert_eq!(c.journey_row(), Some("town run"));
    assert!(c.autoplay.growth.pursuit_held.is_empty(), "passed over");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_gem_used_takes_its_buy_line_out_of_the_section_that_frame() {
    // Through the invitation's portal, the board not due for 2 s: the section still bought the
    // invitation, and a run home for another went off from the landing.
    let mut c = mage_playing("learn:2023", "gem-used-section", 0);
    let now = Instant::now();
    c.refresh_quests(now);
    invitation_wanted(&mut c);
    assert!(matches!(again(&mut c, now), PursuitStep::Buy { .. }));
    let gem = c.autoplay.growth.pursuit_gems[0].wcid;
    in_the_pack(&mut c, 0x8000_0702, gem, "Invitation Ithaenc Cathedral", 1);
    c.refresh_quests(now + Duration::from_millis(100));
    assert!(matches!(c.pursuit[0].step, PursuitStep::Hunt { .. }));
    let buys = |c: &Client| {
        let p = c.loot_profile().unwrap();
        let named = |b: &&crate::profile::Buy| b.what.starts_with("Invitation");
        p.buy.iter().filter(named).count()
    };
    assert_eq!(buys(&c), 1, "the had gem's line");
    in_the_pack(&mut c, 0x8000_0702, gem, "Invitation Ithaenc Cathedral", 0);
    c.refresh_quests(now + Duration::from_millis(200));
    assert_eq!(buys(&c), 0, "the used gem still bought");
    assert!(c.autoplay.growth.pursuit_gems.is_empty(), "still wanted");
}

#[test]
fn a_saved_quest_buy_line_waits_for_the_first_board_and_the_servers_quest_log() {
    // At login the section saved last session bought for a quest the server's log had done: the
    // town run set off on it before the first board.
    use super::super::needs::NeedKind;
    let mut c = fletch_with_a_quests_section();
    pursue(
        &mut c,
        &facts(30, &[(NECKLACE_QUEST, true)]),
        &necklace_sold(),
    );
    // Logged in: the server's quest log asked and not yet heard.
    c.server_quests_read = false;
    c.server_quests_asked = Some(Instant::now());
    let line = section(&c)
        .buy
        .into_iter()
        .find(|b| b.is_quest_entry("Fletch"))
        .expect("the quest's line");
    let cfg = c.autoplay.config.growth.clone();
    let wants = |c: &Client| {
        c.grow_needs(&cfg)
            .iter()
            .any(|n| n.kind == NeedKind::Named(line.what.clone()))
    };
    assert!(!wants(&c), "bought before any board");
    board_built(&mut c);
    assert!(!wants(&c), "bought before the server's quest log");
    c.server_quests_read = true;
    assert!(wants(&c), "not bought once both are in");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_quest_buy_held_back_for_want_of_coin_leaves_grow_to_pick_a_ground() {
    // The run's waits are its own clock; a counter it cannot pay is no reason to stand about.
    let mut c = mage_playing("learn:2023", "buy-without-coin", 5);
    let now = Instant::now();
    c.refresh_quests(now);
    invitation_wanted(&mut c);
    assert!(matches!(again(&mut c, now), PursuitStep::Buy { .. }));
    c.tick_rules(now + Duration::from_millis(100));
    assert!(
        c.autoplay.growth.run.is_none(),
        "a run it cannot pay for: {:?}",
        c.autoplay.status
    );
    let grow = c.autoplay.growth.bound.is_some() || c.journey_row() == Some("grow");
    assert!(
        grow,
        "no ground: {} ({:?}, held back: {})",
        c.autoplay.status, c.autoplay.step, c.autoplay.growth.held_back
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_ground_under_foot_that_no_gem_reaches_waits_its_turn_in_the_list() {
    // Logged in in the Tusker Barracks, Blargerton hunted Tusker Guards for the kill counter, second
    // on his list, though the first only wanted the stamp he carried used on the letter he carried.
    let mut c = mage_pursuing("give:70303:45977", "order-kept");
    c.autoplay
        .config
        .quests
        .pursue_quest("tuskergkillcount", true, 0);
    in_the_pack(&mut c, 0x8000_0710, 45875, "Lucky Gold Letter", 1);
    in_the_pack(&mut c, 0x8000_0711, 8736, "Lytelthorpe Town Stamp", 1);
    c.quest_log.note_taken("tuskergkillcount");
    crate::testkit::stand(&mut c, 0x5A48_01C8, glam::Vec3::new(72.0, -123.0, -1.2));
    c.refresh_quests(std::time::Instant::now());
    let working = c.pursuit.iter().find(|l| l.working).map(|l| l.id.as_str());
    assert_eq!(working, Some("give:70303:45977"), "{:?}", c.pursuit);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_bound_singularity_scepter_with_the_essence_carried_hunts_the_mire_witch() {
    // Bound <- Singularity Scepter <- Glowing Virindi Cloak <- Virindi Essence on a Tattered
    // Virindi Cloak, which only the Mosswart Mire-Witch (9163) drops; her camp is put out by
    // Marae Lassel's shore generators in the encounter table, which the data once left out.
    let mut c = mage_pursuing("give:27841", "chained-quests");
    in_the_pack(&mut c, 0x8000_0723, 9125, "Virindi Essence", 1);
    let now = std::time::Instant::now();
    c.refresh_quests(now);
    let step = c.pursuit[0].step.clone();
    assert!(
        matches!(&step, PursuitStep::Hunt { creature, .. } if creature == "Mosswart Mire-Witch"),
        "no way to the Bound scepter: {step:?}"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_bound_scepter_with_the_cloak_carried_goes_for_the_essence_chest_s_key() {
    // The Virindi Essence is only in a locked chest in A Mosswart Hideout, opened with a Sturdy
    // Iron Key: four links down, past a three-deep search, "nobody known yields Virindi Essence".
    let now = std::time::Instant::now();
    let mut c = mage_pursuing("give:27841", "chained-quests");
    in_the_pack(&mut c, 0x8000_0721, 9117, "Tattered Virindi Cloak", 1);
    c.refresh_quests(now);
    let step = c.pursuit[0].step.clone();
    assert!(
        !matches!(step, PursuitStep::Stuck { .. }),
        "stuck on the essence: {step:?}"
    );
    in_the_pack(&mut c, 0x8000_0722, 6876, "Sturdy Iron Key", 1);
    let step = again(&mut c, now);
    assert!(
        matches!(&step, PursuitStep::OpenChest { chest, .. } if chest.wcid == 9168),
        "the key carried, not to the chest: {step:?}"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_gem_whose_pickup_stamp_waits_is_not_wanted_as_a_way() {
    // The Adjanite Gem (27772) the only way to the Essence chest, picked up 51000 s of its 72000
    // before: wanted, then dropped by the board's obtain, which waits on it, every 3 s.
    let mut c = mage_pursuing("give:27841", "chained-quests");
    // A goal where the gem lands: no other gem's way is shorter.
    let adjanite = 27772;
    let landing = ac_world::gems::all()
        .iter()
        .find(|g| g.wcid == adjanite)
        .map(|g| Place {
            at: g.at,
            cell: g.cell,
        })
        .expect("the Adjanite Gem");
    let goal = [(landing.cell >> 16, landing)];
    let gem = c.gem_way(&goal, &[]);
    assert_eq!(gem.map(|g| g.wcid), Some(adjanite));
    let limit = ac_world::quests::pickup_limit(adjanite).expect("its pickup stamp");
    c.quest_log
        .note_wait(limit.stamp, crate::holdings::unix_now() + 21_000);
    let other = c.gem_way(&goal, &[]);
    assert!(
        other.as_ref().is_none_or(|g| g.wcid != adjanite),
        "{other:?}"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn only_the_creatures_of_the_grounds_hunted_are_sought() {
    // The Sturdy Iron Key drops from 398 creatures, a Gotrok Tiatus at 2% among them: all were
    // sought, so one met on the road to the Virindi Inquisitor (80%) was fought, never walked past.
    let now = std::time::Instant::now();
    let mut c = mage_pursuing("give:27841", "chained-quests");
    in_the_pack(&mut c, 0x8000_0721, 9117, "Tattered Virindi Cloak", 1);
    c.refresh_quests(now);
    let sought = &c.autoplay.growth.pursuit_sought;
    assert!(sought.contains(&10815), "not the Inquisitor: {sought:?}");
    assert!(!sought.contains(&24497), "a Gotrok Tiatus sought");
    assert!(sought.len() < 20, "{} sought", sought.len());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_quests_item_another_quest_rewards_is_had_by_that_hand_in_first() {
    // The Ultimate Singularity Scepter of War Magic takes the Bound one, which the Virindi Castaway
    // gives for the Singularity Scepter carried: "nobody known yields Bound Singularity Scepter".
    let mut c = mage_pursuing("give:27841", "chained-quests");
    in_the_pack(
        &mut c,
        0x8000_0720,
        27840,
        "Singularity Scepter of War Magic",
        1,
    );
    let now = std::time::Instant::now();
    c.refresh_quests(now);
    match &c.pursuit[0].step {
        PursuitStep::HandIn { row, .. } => assert_eq!(row.id, "give:27840", "{row:?}"),
        other => panic!("not the Bound scepter's hand-in: {other:?}"),
    }
    // Back with the Bound one: its own hand-in, to the Executor Assistant.
    in_the_pack(
        &mut c,
        0x8000_0720,
        27840,
        "Singularity Scepter of War Magic",
        0,
    );
    in_the_pack(
        &mut c,
        0x8000_0721,
        27841,
        "Bound Singularity Scepter of War Magic",
        1,
    );
    match again(&mut c, now) {
        PursuitStep::HandIn { row, .. } => assert_eq!(row.id, "give:27841", "{row:?}"),
        other => panic!("not the Ultimate scepter's hand-in: {other:?}"),
    }
}

/// The Bound scepter's pursuit with `c` stood at the middle of the Mire-Witch's nearest ground;
/// that ground's landblock.
fn at_a_mire_witch_ground(now: std::time::Instant) -> (Client, u32) {
    at_the_nth_mire_witch_ground(now, 0)
}

/// [`at_a_mire_witch_ground`] on her `nth` ground on land: past the first, one left empty there
/// leaves the hunt a camping one (a rotating hunt's grounds are never waited at).
fn at_the_nth_mire_witch_ground(now: std::time::Instant, nth: usize) -> (Client, u32) {
    let mut c = mage_pursuing("give:27841", "chained-quests");
    // The essence had: else the chest's key is gone for first.
    in_the_pack(&mut c, 0x8000_0723, 9125, "Virindi Essence", 1);
    c.refresh_quests(now);
    // Her first ground on land: 0xB813's corner is sea, where no walk starts.
    let grounds = c.autoplay.growth.pursuit_grounds.clone();
    let (lb, at) = *grounds
        .iter()
        .filter(|(_, p)| !c.is_at_sea(p.at.truncate()))
        .nth(nth)
        .expect("a ground on land");
    let origin = ac_world::landblock_origin(at.cell);
    crate::testkit::stand(&mut c, at.cell, at.at - origin);
    again(&mut c, now);
    (c, lb)
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_mire_witch_shore_is_looked_over_then_camped_and_cleared() {
    // The Mosswart Mire-Witch's camp is 0.49% of a shore generator's rolls, but a shore cleared
    // rolls it about three times an hour: looked over, then waited at, everything on it fought.
    let now = std::time::Instant::now();
    let (mut c, here) = at_a_mire_witch_ground(now);
    assert!(!c.is_rotating_hunt());
    assert_eq!(c.grow_pursuit(now), Some(true), "looked over");
    assert_eq!(c.autoplay.growth.looking, Some((here, now)));
    assert!(
        c.is_clearing_ground(),
        "a camp that beats leaving is cleared"
    );
    let later = now + std::time::Duration::from_secs(16);
    assert_eq!(c.grow_pursuit(later), Some(true), "waited at");
    assert!(c.autoplay.growth.swept.since(&here).is_none(), "left");
    assert!(c.autoplay.growth.staying.is_some_and(|(lb, _)| lb == here));
    assert!(
        c.autoplay.status.contains("waiting for the"),
        "{}",
        c.autoplay.status
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn an_empty_shore_ground_is_left_rather_than_camped() {
    // A shore whose generators never make her, cleared or not, and next to none at load: once
    // looked over it is left for the next ground, not waited at.
    let now = std::time::Instant::now();
    let (mut c, here) = at_a_mire_witch_ground(now);
    let i = c
        .autoplay
        .growth
        .pursuit_grounds
        .iter()
        .position(|g| g.0 == here)
        .unwrap();
    let empty = pace::Dropper {
        chance: 1.0,
        kill: 10.0,
        rate: ac_world::quests::SpawnRate {
            load: 0.01,
            per_hour: 0.0,
            others: 0.0,
        },
    };
    c.autoplay.growth.pursuit_paces[i] = pace::Ground::of(vec![empty]).unwrap();
    assert_eq!(c.grow_pursuit(now), Some(true), "looked over");
    assert!(!c.is_clearing_ground(), "a ground left is not cleared");
    let later = now + std::time::Duration::from_secs(16);
    assert_eq!(c.grow_pursuit(later), Some(true), "on to the next");
    assert!(c.autoplay.growth.swept.since(&here).is_some());
    assert!(c.autoplay.growth.staying.is_none());
    let next = c.autoplay.growth.bound_landblock();
    assert!(next.is_some_and(|lb| lb != here), "{next:?}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_rare_spawn_in_sight_is_fought_where_it_stands() {
    let now = std::time::Instant::now();
    let (mut c, _) = at_a_mire_witch_ground(now);
    let me = c.my_position().unwrap();
    let cell = c.player.as_ref().unwrap().cell;
    let mut witch = crate::testkit::creature(0x5000_0901, "Mosswart Mire-Witch");
    witch.weenie_class_id = 9163;
    witch.position = crate::testkit::placed(cell, me + glam::Vec3::new(10.0, 0.0, 0.0));
    c.world.objects.insert(witch.guid, witch);
    assert_eq!(c.grow_pursuit(now), None, "left to the fight");
    assert_eq!(c.autoplay.growth.looking, None);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_rare_spawn_seen_on_the_way_between_grounds_is_gone_to() {
    // Martine's Mire-Witch was looked for only standing on a ground: met past the fight radius on
    // the road to the next, she was walked by. A common namesake (7102) is only noted. The road
    // ticks go through grow_hunt, whose walk under way never asked grow_pursuit.
    let now = std::time::Instant::now();
    let (mut c, here) = at_a_mire_witch_ground(now);
    c.autoplay.growth.swept.mark(here, now);
    assert_eq!(c.grow_pursuit(now), Some(true), "on to the next ground");
    assert!(c.autoplay.growth.bound.is_some(), "a ground walk under way");
    let cfg = crate::autoplay::growth::config::Growth::default();
    let me = c.my_position().unwrap();
    let cell = c.player.as_ref().unwrap().cell;
    let radius = c.autoplay.config.fight.radius;
    let far = me + glam::Vec3::new(radius * 2.0, 0.0, 0.0);
    let mut common = crate::testkit::creature(0x5000_0902, "Mosswart Mire-Witch");
    common.weenie_class_id = 7102;
    common.position = crate::testkit::placed(cell, far);
    c.world.objects.insert(common.guid, common);
    let later = now + std::time::Duration::from_secs(1);
    assert!(c.grow_hunt(later, &cfg), "the ground walk kept");
    let goal = c.travel_goal_xy().expect("travelling");
    assert!(
        goal.distance(far.truncate()) > radius,
        "went to the common one"
    );
    assert!(
        c.autoplay.noted.since(&"a Mosswart Mire-Witch (7102) in sight, not the one Bound Singularity Scepter of War Magic wants".to_string()).is_some(),
        "the namesake not noted"
    );
    let mut witch = crate::testkit::creature(0x5000_0901, "Mosswart Mire-Witch");
    witch.weenie_class_id = 9163;
    witch.position = crate::testkit::placed(cell, far);
    c.world.objects.insert(witch.guid, witch);
    assert!(c.grow_hunt(later, &cfg), "gone to her");
    assert!(c.autoplay.growth.bound.is_some(), "the ground walk kept");
    assert_eq!(c.autoplay.growth.chased, Some(0x5000_0901));
    let goal = c.travel_goal_xy().expect("travelling");
    assert!(
        goal.distance(far.truncate()) <= radius,
        "not to her: {goal:?} vs {far:?}"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_swept_ground_waits_for_its_fresh_roll_from_when_it_was_last_beside() {
    // A landblock stood in or beside stays loaded (Landblock.cs:1111-1123): counted from its
    // look-over, a cluster's ground was looked over again with the same roll on it.
    let now = std::time::Instant::now();
    let (mut c, here) = at_a_mire_witch_ground(now);
    c.autoplay.growth.swept.mark(here, now);
    let mins = |m: u64| now + std::time::Duration::from_secs(m * 60);
    c.grow_pursuit(mins(9));
    c.grow_pursuit(mins(15));
    assert!(
        c.autoplay.growth.swept.since(&here).is_some(),
        "looked over again too soon"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_sought_kill_s_body_is_stood_by() {
    // Martine fell, the walk to the next ground went on before her body came, and a hit stopped
    // its looting: the Tattered Virindi Cloak was left. Her body is stood by, walked back to.
    let now = std::time::Instant::now();
    let (mut c, here) = at_a_mire_witch_ground(now);
    c.autoplay.growth.swept.mark(here, now);
    assert_eq!(c.grow_pursuit(now), Some(true), "on to the next ground");
    let cfg = crate::autoplay::growth::config::Growth::default();
    let me = c.my_position().unwrap();
    let cell = c.player.as_ref().unwrap().cell;
    let spot = me + glam::Vec3::new(40.0, 0.0, 0.0);
    let mut witch = crate::testkit::creature(0x5000_0901, "Mosswart Mire-Witch");
    witch.weenie_class_id = 9163;
    witch.position = crate::testkit::placed(cell, spot);
    c.world.objects.insert(witch.guid, witch);
    c.note_sought_kill(spot, now);
    c.world.objects.remove(&0x5000_0901);
    let later = now + std::time::Duration::from_secs(1);
    assert!(c.grow_hunt(later, &cfg));
    assert!(c.autoplay.growth.bound.is_none(), "the ground walk kept");
    let goal = c.travel_goal_xy().expect("walking back");
    assert!(goal.distance(spot.truncate()) < 5.0, "{goal:?} vs {spot:?}");
    // Her body, emptied: let go of.
    let mut body = crate::testkit::corpse(0x5000_0903, "Corpse of Mosswart Mire-Witch");
    body.position = crate::testkit::placed(cell, spot);
    c.world.objects.insert(body.guid, body);
    assert!(c.grow_hunt(later, &cfg), "the body not yet emptied");
    assert!(c.autoplay.growth.sought_kill.is_some());
    c.autoplay.looted.push(0x5000_0903, later);
    c.grow_hunt(later, &cfg);
    assert!(
        c.autoplay.growth.sought_kill.is_none(),
        "stood by an emptied body"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_sought_kill_s_body_the_looting_will_not_open_is_not_stood_by() {
    // Aun Ralirea's body, his packs down to the slots kept for a counter's money: the looting
    // never opened it and he stood by it, waiting on it.
    let now = std::time::Instant::now();
    let (mut c, _) = at_a_mire_witch_ground(now);
    let cfg = crate::autoplay::growth::config::Growth::default();
    let me = c.my_position().unwrap();
    let cell = c.player.as_ref().unwrap().cell;
    let mut witch = crate::testkit::creature(0x5000_0901, "Mosswart Mire-Witch");
    witch.weenie_class_id = 9163;
    witch.position = crate::testkit::placed(cell, me);
    c.world.objects.insert(witch.guid, witch);
    c.note_sought_kill(me, now);
    c.world.objects.remove(&0x5000_0901);
    let mut body = crate::testkit::corpse(0x5000_0903, "Corpse of Mosswart Mire-Witch");
    body.position = crate::testkit::placed(cell, me);
    c.world.objects.insert(body.guid, body);
    c.autoplay.config.team.restock.money_slots = 10_000;
    assert!(c.pack_low_on_room());
    c.grow_hunt(now, &cfg);
    assert!(
        c.autoplay.growth.sought_kill.is_none(),
        "stood by a body the looting will not open: {}",
        c.autoplay.status
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_wait_for_another_hunts_ground_to_unload_is_not_kept() {
    // The rotating hold named a landblock and no hunt: taken up by the next quest, it walked out
    // of reach of a ground that quest never hunts and waited there.
    let now = std::time::Instant::now();
    let (mut c, _) = at_a_mire_witch_ground(now);
    let other = 0x0101;
    assert!(c.pursued_place(other).is_none());
    // Stood at its spot out of reach, as the hold had walked it there.
    let spot = c.place_at(c.my_position().unwrap());
    c.autoplay.growth.rotating = Some((other, spot));
    c.autoplay.growth.swept.mark(other, now);
    assert_eq!(
        c.wait_out_unload("Mire-Witch"),
        None,
        "{}",
        c.autoplay.status
    );
    assert_eq!(c.autoplay.growth.rotating, None);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_sought_kill_s_body_a_mate_is_opening_is_not_stood_by() {
    // The wait asked only whether the body waited, the looting whose turn it was too: a body a
    // mate was opening was stood by for up to five minutes.
    let now = std::time::Instant::now();
    let (mut c, _) = at_a_mire_witch_ground(now);
    let cfg = crate::autoplay::growth::config::Growth::default();
    let me = c.my_position().unwrap();
    let cell = c.player.as_ref().unwrap().cell;
    let mut witch = crate::testkit::creature(0x5000_0901, "Mosswart Mire-Witch");
    witch.weenie_class_id = 9163;
    witch.position = crate::testkit::placed(cell, me);
    c.world.objects.insert(witch.guid, witch);
    c.note_sought_kill(me, now);
    c.world.objects.remove(&0x5000_0901);
    let body = 0x5000_0903;
    let mut corpse = crate::testkit::corpse(body, "Corpse of Mosswart Mire-Witch");
    corpse.position = crate::testkit::placed(cell, me);
    c.world.objects.insert(body, corpse);
    let held = std::time::Duration::from_secs(5);
    let mate = crate::testkit::looter(0x5000_0002, me, Some(body), held);
    c.autoplay.team = crate::testkit::view_of(vec![mate]);
    c.grow_hunt(now, &cfg);
    assert!(
        c.autoplay.growth.sought_kill.is_none(),
        "stood by a mate's body: {}",
        c.autoplay.status
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_sought_kill_s_body_is_opened_under_blows() {
    // By the key Putrid Moarsman's body a room of its namesakes and wisps hit him: fought as
    // guarding the body (nothing was opened under blows), two minutes, before the body opened.
    let now = std::time::Instant::now();
    let (mut c, _) = at_a_mire_witch_ground(now);
    let me = c.my_position().unwrap();
    let cell = c.player.as_ref().unwrap().cell;
    let mut witch = crate::testkit::creature(0x5000_0901, "Mosswart Mire-Witch");
    witch.weenie_class_id = 9163;
    witch.position = crate::testkit::placed(cell, me);
    c.world.objects.insert(witch.guid, witch);
    c.note_sought_kill(me, now);
    let mut body = crate::testkit::corpse(0x5000_0907, "Corpse of Mosswart Mire-Witch");
    body.position = crate::testkit::placed(cell, me);
    c.world.objects.insert(body.guid, body);
    assert_eq!(c.sought_body(), Some(0x5000_0907));
    // A namesake killed first lies beside it, emptied: the body stood by is still the witch's.
    let mut kin = crate::testkit::corpse(0x5000_0908, "Corpse of Mosswart Mire-Witch");
    kin.position = crate::testkit::placed(cell, me);
    c.world.objects.insert(kin.guid, kin);
    c.autoplay.looted.push(0x5000_0908, now);
    assert_eq!(c.sought_body(), Some(0x5000_0907), "the emptied kin's body");
    let mut chomu = crate::testkit::creature(0x5000_0904, "Chomu Sclavus");
    chomu.weenie_class_id = 1;
    chomu.position = crate::testkit::placed(cell, me + glam::Vec3::new(2.0, 0.0, 0.0));
    c.world.objects.insert(chomu.guid, chomu.clone());
    c.autoplay.attacked_by("Chomu Sclavus", now);
    let cfg = c.autoplay.config.fight.clone();
    assert!(c.passing_by(&chomu, &cfg), "hitting us by the body: fought");
    let weighed = crate::steps::weigh(&c, now).ranking();
    assert!(
        weighed.iter().any(|r| r.0 == "loot"),
        "the body waits for the blows to stop: {weighed:?}"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_ground_of_linked_placements_is_camped_not_cleared() {
    // The Tusker Barracks' Tusker Guards are linked placements, each slot its own: killing what
    // else stands there frees none of them, so it is walked past, critters too.
    let tusker_barracks = 0x5A48_01C8;
    let mut c = mage_pursuing("tuskergkillcount", "chained-quests");
    c.quest_log.note_taken("tuskergkillcount");
    crate::testkit::stand(&mut c, tusker_barracks, glam::Vec3::new(72.0, -123.0, -1.2));
    let now = std::time::Instant::now();
    again(&mut c, now);
    assert!(!c.is_rotating_hunt());
    c.grow_pursuit(now);
    assert!(!c.is_clearing_ground());
    let mite = crate::testkit::creature(0x5000_0902, "Mite");
    let cfg = c.autoplay.config.fight.clone();
    assert!(
        c.passing_by(&mite, &cfg),
        "a Mite holds no Tusker Guard's slot"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_cleared_ground_is_cleared_of_all_it_holds() {
    // A Mire-Witch shore is a region generator's roll: what else it makes holds her slots, so
    // all of it is fought, critters too (at the Tusker Barracks what hit him was walked past).
    let now = std::time::Instant::now();
    let (mut c, _) = at_a_mire_witch_ground(now);
    c.grow_pursuit(now);
    assert!(c.is_clearing_ground());
    let critter = crate::testkit::creature(0x5000_0902, "Mite");
    let cfg = c.autoplay.config.fight.clone();
    assert_eq!(
        c.critter_verdict(&critter, &cfg),
        crate::autoplay::fight::critter::Critter::Fight
    );
    assert!(!c.passing_by(&critter, &cfg));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_cleared_ground_is_not_cleared_on_the_way_out() {
    // Set to clear, then walking off it for something else (a town run, a supply trip): what
    // stands there is walked past like any road's.
    let now = std::time::Instant::now();
    let (mut c, here) = at_a_mire_witch_ground(now);
    c.grow_pursuit(now);
    assert!(c.is_clearing_ground());
    let me = c.my_position().unwrap();
    let away = ac_world::Place::on_map(glam::Vec2::new(me.x + 400.0, me.y));
    assert!(c.grow_travel(away, crate::travel::ARRIVE));
    c.autoplay.growth.bound = Some((here + 2, away.at.truncate(), "Drudge".into()));
    assert!(!c.is_clearing_ground(), "on its way elsewhere");
    let mite = crate::testkit::creature(0x5000_0902, "Mite");
    let cfg = c.autoplay.config.fight.clone();
    assert!(c.passing_by(&mite, &cfg));
}

/// Blargerton as he hunted the Sturdy Iron Key: a level 60 Lightning War mage at Holtburg, tier 5
/// killed, the Tattered Virindi Cloak carried for the Bound scepter's chain.
fn blargerton_after_the_key(now: std::time::Instant) -> Client {
    let (mut c, _) = crate::testkit::a_caster_knowing(
        now,
        &["Lightning Bolt VI", "Lightning Vulnerability Other VI"],
    );
    for skill in [33, 34] {
        crate::testkit::trained_in(&mut c, skill, 400);
    }
    c.world.stats.level = 60;
    c.world.stats.name = "Blarg".into();
    c.ledger_loaded = true;
    crate::testkit::a_loot_profile(&mut c, "chained-quests");
    c.autoplay.growth.proven_tier = 5;
    c.autoplay.config.quests.on = true;
    c.autoplay
        .config
        .quests
        .pursue_quest("give:27841", false, 0);
    in_the_pack(&mut c, 0x8000_0721, 9117, "Tattered Virindi Cloak", 1);
    c.refresh_quests(now);
    c
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_near_dungeon_of_low_chance_sure_spawns_beats_a_far_high_chance_ground() {
    // "A dungeon of guaranteed 2% droppers that die in a cast or two beats a far ground of rare
    // 80% ones": the Chakron's Virindi Inquisitors (80%) take 6 casts each and a long road.
    let now = std::time::Instant::now();
    let mut c = blargerton_after_the_key(now);
    let quests = c.travel.quests_done.clone();
    let grounds = c.autoplay.growth.pursuit_grounds.clone();
    let mut totals: Vec<(f32, u32)> = Vec::new();
    for (lb, at) in grounds {
        let there = c.pursued_pace(lb).expect("a pace").pace.soonest();
        if let Some(t) = c.journey_plan(at, &quests) {
            println!(
                "{lb:04X}: {:.0} s there + {:.0} s on the road",
                there, t.seconds
            );
            totals.push((t.seconds + there, lb));
        }
    }
    totals.sort_by(|a, b| a.0.total_cmp(&b.0));
    let chakron = totals.iter().find(|t| t.1 == 0x02AE || t.1 == 0x02AF);
    let first = totals.first().expect("a ground reached").1;
    assert!(
        first != 0x02AE && first != 0x02AF,
        "the Chakron first: {totals:?}"
    );
    assert!(chakron.is_none_or(|c| c.0 > totals[0].0), "{totals:?}");
    assert_eq!(c.grow_pursuit(now), Some(true));
    assert_eq!(c.autoplay.growth.bound_landblock(), Some(first));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_ground_left_empty_is_rotated_when_unload_is_sooner_than_respawn() {
    // A ground whose one dropper stands there at load and never comes back while it is loaded,
    // the rest left lately: walked out of its reach to unload, and waited out there.
    let now = std::time::Instant::now();
    let (mut c, here) = at_the_nth_mire_witch_ground(now, 1);
    let grounds = c.autoplay.growth.pursuit_grounds.clone();
    for (lb, _) in grounds.iter().filter(|g| g.0 != here) {
        c.autoplay.growth.swept.mark(*lb, now);
    }
    let i = grounds.iter().position(|g| g.0 == here).unwrap();
    let at_load = pace::Dropper {
        chance: 1.0,
        kill: 10.0,
        rate: ac_world::quests::SpawnRate {
            load: 1.0,
            per_hour: 0.0,
            others: 0.0,
        },
    };
    c.autoplay.growth.pursuit_paces[i] = pace::Ground::of(vec![at_load]).unwrap();
    assert!(!c.is_rotating_hunt());
    assert_eq!(c.grow_pursuit(now), Some(true), "looked over");
    let later = now + std::time::Duration::from_secs(16);
    assert_eq!(c.grow_pursuit(later), Some(true), "out of its reach");
    let (lb, spot) = c.autoplay.growth.rotating.expect("rotating");
    assert_eq!(lb, here);
    let block = |at: glam::Vec3| ((at.x / 192.0).floor() as i32, (at.y / 192.0).floor() as i32);
    let (sx, sy) = block(spot.at);
    let (hx, hy) = ((here >> 8) as i32, (here & 0xFF) as i32);
    assert_eq!((sx - hx).abs().max((sy - hy).abs()), 2, "{spot:?}");
    assert!(
        c.autoplay.status.contains("unload"),
        "{}",
        c.autoplay.status
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_dropper_met_on_the_road_is_fought_only_when_its_chance_beats_the_plan() {
    // The Sturdy Iron Key drops 2% from a Gotrok Tiatus: met on the road to a ground five minutes
    // off it is walked past; with the plan hours away a kill of it saves more than it costs.
    let now = std::time::Instant::now();
    let mut c = blargerton_after_the_key(now);
    let gotrok = 24497;
    let kill = c
        .autoplay
        .growth
        .pursuit_droppers
        .iter()
        .find(|d| d.0 == gotrok);
    let &(_, chance, secs) = kill.expect("a Gotrok Tiatus he can kill");
    let mut tiatus = crate::testkit::creature(0x5000_0904, "Gotrok Tiatus");
    tiatus.weenie_class_id = gotrok;
    let cfg = c.autoplay.config.fight.clone();
    c.autoplay.growth.plan_secs = Some(secs / chance * 0.5);
    c.keep_road_droppers();
    assert!(c.passing_by(&tiatus, &cfg), "fought, the plan near");
    c.autoplay.growth.plan_secs = Some(secs / chance * 2.0);
    c.keep_road_droppers();
    // Worth the plan, but minding its own business: run past (the K'nath K'ram gone out to and let
    // go every tick). On the character, it is fought.
    assert!(
        c.passing_by(&tiatus, &cfg),
        "gone out to, not on the character"
    );
    tiatus.walked_at = c.world.player_guid;
    assert!(
        !c.passing_by(&tiatus, &cfg),
        "walked past with the plan far and it on the character"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_sought_dropper_seen_from_the_road_is_gone_to_only_when_worth_the_plan() {
    // Blargerton on his way to a ground: a sought dropper in sight past the fight radius was gone
    // to whatever its chance, the journey dropped. Only one the road rule finds worth it is.
    let now = std::time::Instant::now();
    let mut c = blargerton_after_the_key(now);
    assert!(!c.is_rotating_hunt());
    assert_eq!(c.grow_pursuit(now), Some(true), "set off");
    let st = &c.autoplay.growth;
    let &(wcid, chance, secs) = st
        .pursuit_droppers
        .iter()
        .filter(|d| st.pursuit_sought.contains(&d.0))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .expect("a sought dropper");
    let me = c.my_position().unwrap();
    let cell = c.player.as_ref().unwrap().cell;
    let far = me + glam::Vec3::new(c.autoplay.config.fight.radius * 2.0, 0.0, 0.0);
    let mut it = crate::testkit::creature(0x5000_0905, "Sought Dropper");
    it.weenie_class_id = wcid;
    it.position = crate::testkit::placed(cell, far);
    c.world.objects.insert(it.guid, it);
    let cfg = crate::autoplay::growth::config::Growth::default();
    let later = now + std::time::Duration::from_secs(1);
    c.autoplay.growth.plan_secs = Some(secs / chance * 0.5);
    c.keep_road_droppers();
    assert!(c.grow_hunt(later, &cfg));
    assert_eq!(c.autoplay.growth.chased, None, "gone to, the plan near");
    c.autoplay.growth.plan_secs = Some(secs / chance * 2.0);
    c.keep_road_droppers();
    assert!(c.grow_hunt(later, &cfg));
    assert_eq!(
        c.autoplay.growth.chased,
        Some(0x5000_0905),
        "walked by, the plan far"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_held_grounds_dropper_in_reach_on_the_road_is_fought_only_when_worth_the_plan() {
    // The road rule only added fights: a held ground's 2% dropper met in reach on the road was
    // fought whatever the plan, as one of the sought.
    let now = std::time::Instant::now();
    let mut c = blargerton_after_the_key(now);
    assert_eq!(c.grow_pursuit(now), Some(true), "set off");
    assert!(c.on_its_way());
    let st = &c.autoplay.growth;
    let &(wcid, chance, secs) = st
        .pursuit_droppers
        .iter()
        .filter(|d| st.pursuit_sought.contains(&d.0))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .expect("a sought dropper");
    let mut it = crate::testkit::creature(0x5000_0906, "Sought Dropper");
    it.weenie_class_id = wcid;
    let cfg = c.autoplay.config.fight.clone();
    c.autoplay.growth.plan_secs = Some(secs / chance * 0.5);
    c.keep_road_droppers();
    assert!(
        c.passing_by(&it, &cfg),
        "fought on the road, not worth the plan"
    );
    c.autoplay.growth.plan_secs = Some(secs / chance * 2.0);
    c.keep_road_droppers();
    it.walked_at = c.world.player_guid;
    assert!(!c.passing_by(&it, &cfg), "walked past, worth the plan");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_road_plan_ends_at_the_ground_and_with_the_hunt() {
    // The plan's seconds held after arriving, and after the hunt became another, and the road
    // rule went on fighting droppers for a journey that was over.
    let now = std::time::Instant::now();
    let (mut c, _) = at_a_mire_witch_ground(now);
    c.autoplay.growth.plan_secs = Some(1e9);
    c.keep_road_droppers();
    assert!(!c.autoplay.growth.pursuit_road.is_empty());
    c.grow_pursuit(now);
    assert_eq!(c.autoplay.growth.plan_secs, None, "there");
    assert!(c.autoplay.growth.pursuit_road.is_empty());
    c.autoplay.growth.plan_secs = Some(1e9);
    c.autoplay.growth.pursuit_droppers = vec![(1, 0.5, 10.0)];
    again(&mut c, now);
    assert_eq!(c.autoplay.growth.plan_secs, None, "another hunt");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_sought_creature_in_sight_with_no_way_to_it_is_set_aside() {
    // Martine seen out at sea: no walk reaches her, and every tick answered "fought where it
    // stands" for a fight that never came, the hunt held.
    let now = std::time::Instant::now();
    let (mut c, _) = at_a_mire_witch_ground(now);
    let me = c.my_position().unwrap();
    let radius = c.autoplay.config.fight.radius;
    // The nearest sea, by steps of 20 m out to half a kilometre, sixteen ways round.
    let sea = (0..16 * 24)
        .map(|i| {
            let a = (i % 16) as f32 * std::f32::consts::TAU / 16.0;
            let far = radius * 2.0 + (i / 16) as f32 * 20.0;
            me + glam::Vec3::new(a.cos(), a.sin(), 0.0) * far
        })
        .find(|at| c.is_at_sea(at.truncate()))
        .expect("the sea off her shore");
    let cell = c.player.as_ref().unwrap().cell;
    let mut witch = crate::testkit::creature(0x5000_0901, "Mosswart Mire-Witch");
    witch.weenie_class_id = 9163;
    witch.position = crate::testkit::placed(cell, sea);
    c.world.objects.insert(witch.guid, witch);
    assert_eq!(c.grow_pursuit(now), Some(true), "the ground hunted on");
    assert!(c.autoplay.given_up.since(&0x5000_0901).is_some());
    let later = now + std::time::Duration::from_secs(1);
    assert_eq!(c.grow_pursuit(later), Some(true), "and on");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn grounds_past_the_six_soonest_there_are_weighed_by_their_journey_too() {
    // The six soonest at the ground were the only ones planned: a near dungeon ranked seventh
    // there was never weighed against a far first.
    let now = std::time::Instant::now();
    let mut c = blargerton_after_the_key(now);
    let held = c.autoplay.growth.pursuit_grounds.clone();
    assert!(held.len() > HUNT_GROUNDS, "{} held", held.len());
    let quests = c.travel.quests_done.clone();
    let mut totals: Vec<(f32, u32)> = Vec::new();
    for (lb, at) in held {
        let there = c.pursued_pace(lb).expect("a pace").pace.soonest();
        if let Some(t) = c.journey_plan(at, &quests) {
            totals.push((t.seconds + there, lb));
        }
    }
    let first = totals
        .iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .expect("one")
        .1;
    let t0 = std::time::Instant::now();
    assert_eq!(c.grow_pursuit(now), Some(true));
    println!("chosen in {:?}", t0.elapsed());
    assert_eq!(c.autoplay.growth.bound_landblock(), Some(first));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_ground_set_off_for_past_the_soonest_few_has_its_creatures_sought() {
    // Twenty grounds weighed, the creatures of the six soonest there sought: one further down,
    // taken for its short road, is hunted for its own creatures once on the way.
    let now = std::time::Instant::now();
    let mut c = blargerton_after_the_key(now);
    let st = &c.autoplay.growth;
    let on = |lb: u32| -> Vec<u32> {
        let key = ac_world::quests::Want::Item(6876);
        let all = ac_world::quest_chain::source_grounds(&key);
        let here = all
            .iter()
            .filter(|g| g.landblock == lb)
            .map(|g| g.source.wcid);
        here.filter(|w| st.pursuit_droppers.iter().any(|d| d.0 == *w))
            .collect()
    };
    let (lb, at) = st.pursuit_grounds[HUNT_GROUNDS..]
        .iter()
        .copied()
        .find(|g| on(g.0).iter().all(|w| !st.pursuit_sought.contains(w)))
        .expect("a ground past the sixth with none of its creatures sought");
    let mine = on(lb);
    assert!(c.grow_travel(at, c.ground_reach(lb)));
    c.autoplay.growth.bound = Some((lb, at.at.truncate(), "it".into()));
    again(&mut c, now);
    let sought = &c.autoplay.growth.pursuit_sought;
    assert!(
        mine.iter().all(|w| sought.contains(w)),
        "{mine:?} not in {sought:?}"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_ground_waited_at_in_vain_is_not_waited_at_again() {
    // A shore with no spawn spot known was waited at, nothing came, and the same figures chose
    // the same wait again for ever: a wait that ends empty counts camping there as never coming.
    let now = std::time::Instant::now();
    let (mut c, here) = at_a_mire_witch_ground(now);
    assert_eq!(c.grow_pursuit(now), Some(true), "looked over");
    let later = now + std::time::Duration::from_secs(16);
    assert_eq!(c.grow_pursuit(later), Some(true), "waited at");
    let (_, until) = c.autoplay.growth.staying.expect("a wait");
    let after = until + std::time::Duration::from_secs(1);
    c.grow_pursuit(after);
    assert!(c.autoplay.growth.missed_stay.since(&here).is_some());
    assert!(
        c.autoplay.growth.staying.is_none_or(|(lb, _)| lb != here),
        "waited at again"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_ground_found_empty_again_on_return_is_not_rotated_to_again() {
    // Left to unload and come back to, its one dropper absent again (another player keeping it
    // loaded): its load counts as none for an hour, so it is not left to unload once more.
    let now = std::time::Instant::now();
    let (mut c, here) = at_the_nth_mire_witch_ground(now, 1);
    let grounds = c.autoplay.growth.pursuit_grounds.clone();
    for (lb, _) in grounds.iter().filter(|g| g.0 != here) {
        c.autoplay.growth.swept.mark(*lb, now);
    }
    let i = grounds.iter().position(|g| g.0 == here).unwrap();
    let at_load = pace::Dropper {
        chance: 1.0,
        kill: 10.0,
        rate: ac_world::quests::SpawnRate {
            load: 1.0,
            per_hour: 0.0,
            others: 0.0,
        },
    };
    c.autoplay.growth.pursuit_paces[i] = pace::Ground::of(vec![at_load]).unwrap();
    let before = now - std::time::Duration::from_secs(8 * 60);
    c.autoplay.growth.found_empty.mark(here, before);
    assert_eq!(c.grow_pursuit(now), Some(true), "looked over");
    let later = now + std::time::Duration::from_secs(16);
    c.grow_pursuit(later);
    assert!(c.autoplay.growth.missed_back.since(&here).is_some());
    assert_eq!(c.autoplay.growth.rotating, None);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn aun_ralirea_s_grounds_are_walked_on_through_never_waited_at() {
    // A peace generator's 0.2% pick at a landblock's load: stood 15 s looking at each ground,
    // then recalled off to wait out its unload. Each ground reached is the look; on at once.
    let now = std::time::Instant::now();
    let mut c = mage_pursuing("championkillquest1", "aun-ralirea");
    c.world.stats.level = 120;
    c.autoplay.growth.proven_tier = 8;
    c.refresh_quests(now);
    assert!(
        matches!(c.pursuit[0].step, PursuitStep::Hunt { .. }),
        "{:?}",
        c.pursuit[0].step
    );
    assert!(c.is_rotating_hunt());
    let (here, at) = c.autoplay.growth.pursuit_grounds[0];
    let origin = ac_world::landblock_origin(at.cell);
    crate::testkit::stand(&mut c, at.cell, at.at - origin);
    again(&mut c, now);
    assert_eq!(c.grow_pursuit(now), Some(true));
    assert!(c.autoplay.growth.swept.since(&here).is_some(), "left");
    assert!(c.autoplay.growth.rotating.is_none(), "an unload waited out");
    assert!(
        c.autoplay.status.contains("on the way to hunt"),
        "{}",
        c.autoplay.status
    );
}

/// Aerfalle's Token on the bundled data, `carried` and the events `on` as given: the working step.
fn aerfalle_step(carried: &[u32], on: &[&str]) -> PursuitStep {
    let token = "give:40913";
    let mut f = facts(150, &[(token, true)]);
    f.carried = carried.iter().map(|&w| (w, 1)).collect();
    let on = |e: &str| on.contains(&e);
    let p = work_events(&f, &ChainData, None, &|_| false, &|_| Some(20.0), &on);
    p.lines[0].step.clone()
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn aerfalles_token_is_worked_down_its_chain_of_server_events_on_the_bundled_data() {
    // The chest's Ashen Key drops from an Apprentice only the aerfallegen event puts out; the
    // Ghost of Dylaeral starts it for the Sacrificial Dagger, and so on down to the Sluice Gate
    // Bar handed to the Repair Golem (Aerlinthe's eruption).
    let (dagger, bellows, bar) = (28065, 7411, 7412);
    let name = |s: &PursuitStep| match s {
        PursuitStep::PickUp { name, .. } => format!("pick up {name}"),
        PursuitStep::GiveTo { source, .. } => format!("give to {}", source.name),
        PursuitStep::Hunt { creature, .. } => format!("hunt {creature}"),
        PursuitStep::OpenChest { chest, .. } => format!("open {}", chest.name),
        other => format!("{other:?}"),
    };
    assert_eq!(name(&aerfalle_step(&[], &[])), "pick up Sacrificial Dagger");
    assert_eq!(
        name(&aerfalle_step(&[dagger], &[])),
        "pick up Pyreal Forge Bellows"
    );
    let both = [dagger, bellows];
    assert_eq!(name(&aerfalle_step(&both, &[])), "pick up Sluice Gate Bar");
    let all = [dagger, bellows, bar];
    assert_eq!(name(&aerfalle_step(&all, &[])), "give to Repair Golem");
    let on = ["forgegolemgen"];
    assert_eq!(name(&aerfalle_step(&both, &on)), "give to Smithing Golem");
    let on = ["erupttenkarrdunbossgen"];
    assert_eq!(
        name(&aerfalle_step(&[dagger], &on)),
        "hunt Tenkarrdun Hellfire"
    );
    let on = ["eruptportbossgen"];
    assert_eq!(name(&aerfalle_step(&[dagger], &on)), "hunt Relic Watchman");
    let on = ["aerfalleubergen"];
    assert_eq!(
        name(&aerfalle_step(&[dagger], &on)),
        "give to Ghost of Dylaeral"
    );
    let on = ["aerfallegen"];
    assert_eq!(
        name(&aerfalle_step(&[], &on)),
        "hunt Aerfalle's Weakened Apprentice"
    );
    let key = 7409;
    assert_eq!(
        name(&aerfalle_step(&[key], &[])),
        "open Lady of Aerlinthe's Chest"
    );
}

const BAR: u32 = 0x8000_0303;
const REPAIR_GOLEM: u32 = 0x7AB0_1001;

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn at_the_repair_golem_the_bar_is_handed_over_and_its_event_taken_for_on_once_it_is_gone() {
    let golem = |e: &str| {
        let want = Want::Event(e.into());
        let mut gives = ac_world::quests::sources_of(&want).filter(|s| s.kind == Yield::Give);
        gives
            .find(|s| s.wcid == 7404)
            .expect("the Repair Golem starts it")
    };
    let source = golem("forgegolemgen");
    let (mut c, line) = fetching(|at| PursuitStep::GiveTo {
        source,
        at,
        item: 7412,
    });
    let PursuitStep::GiveTo { at, .. } = line.step else {
        unreachable!()
    };
    lying(&mut c, REPAIR_GOLEM, 7404, "Repair Golem", at);
    carrying(&mut c, BAR, 7412, "Sluice Gate Bar");
    let now = Instant::now();
    assert_eq!(c.grow_fetch(&line, now), Some(true));
    let f = c
        .autoplay
        .growth
        .fetching
        .clone()
        .expect("waiting on the give");
    assert_eq!(
        (f.act, f.guid, f.with),
        (fetch::Act::Give, REPAIR_GOLEM, 7412)
    );
    assert!(
        !c.is_event_on("forgegolemgen", now),
        "not before the golem takes it"
    );
    // The golem took the bar: the event is on, and the board is worked out again.
    c.world.objects.remove(&BAR);
    let soon = now + Duration::from_millis(300);
    c.quest_board_at = Some(soon);
    assert_eq!(c.grow_fetch(&line, soon), Some(true));
    assert!(c.is_event_on("forgegolemgen", soon));
    assert_eq!(c.autoplay.growth.fetching, None);
    assert_eq!(c.quest_board_at, None);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn an_event_is_taken_for_on_twice_its_creatures_spawn_wait_and_one_gating_npcs_till_they_are_missed(
) {
    // The Weakened Apprentice comes 10 an hour once aerfallegen is on (the data's rate).
    assert_eq!(
        events::belief("aerfallegen"),
        Some(Duration::from_secs(720))
    );
    assert_eq!(
        events::belief("forgegolemgen"),
        None,
        "the Smithing Golem: till missed"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn an_aun_ralirea_ground_reached_over_its_landblock_s_edge_is_swept_not_walked_to_again() {
    // Its walk ends 30 m from the ground's place, here in the landblock east of it: planned again
    // every tick, "arrived" every tick, while a Kithless Siraluun hit him (Timaru, 0x19B6).
    let now = std::time::Instant::now();
    let mut c = mage_pursuing("championkillquest1", "aun-ralirea");
    c.world.stats.level = 120;
    c.autoplay.growth.proven_tier = 8;
    c.refresh_quests(now);
    assert!(c.is_rotating_hunt());
    let ground = 0x19B6;
    let grounds = &c.autoplay.growth.pursuit_grounds;
    assert!(grounds.iter().any(|g| g.0 == ground), "{grounds:x?}");
    crate::testkit::stand(&mut c, 0x1AB6_0004, glam::Vec3::new(4.6, 87.8, 76.0));
    again(&mut c, now);
    assert_eq!(c.grow_pursuit(now), Some(true));
    assert!(c.autoplay.growth.swept.since(&ground).is_some(), "left");
    let to = c.autoplay.growth.bound.map(|b| b.0);
    assert_ne!(to, Some(ground), "{}", c.autoplay.status);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_pursued_ground_cleared_over_its_landblock_s_edge_is_cleared_there() {
    // Stood in 0x1AB6 within the reach of the 0x19B6 ground: the arrival found it, but clearing
    // read the landblock stood in, and what held its generators' slots was walked past.
    let now = std::time::Instant::now();
    let mut c = mage_pursuing("championkillquest1", "aun-ralirea");
    c.world.stats.level = 120;
    c.autoplay.growth.proven_tier = 8;
    c.refresh_quests(now);
    crate::testkit::stand(&mut c, 0x1AB6_0004, glam::Vec3::new(4.6, 87.8, 76.0));
    again(&mut c, now);
    c.autoplay.growth.clearing = Some(0x19B6);
    assert!(c.is_clearing_ground());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_wait_at_a_ground_of_several_spawn_rooms_is_stood_not_walked_round() {
    // The Silver Tusker's four rooms in the Tusker Pits, 7-18 m apart: the 90 s wait forgot the
    // rooms gone round and the next tick walked them again, a round every 1.7 s for good.
    let now = std::time::Instant::now();
    let mut c = mage_pursuing("give:22431", "silver-tusk");
    c.world.stats.level = 66;
    c.refresh_quests(now);
    assert!(
        matches!(&c.pursuit[0].step, PursuitStep::Hunt { creature, .. } if creature == "Silver Tusker"),
        "{:?}",
        c.pursuit[0].step
    );
    crate::testkit::stand(&mut c, 0x5A4A_01A0, glam::Vec3::new(107.3, -86.5, -42.0));
    again(&mut c, now);
    let rooms = [0x5A4A_019A, 0x5A4A_019C, 0x5A4A_019E, 0x5A4A_01A0];
    c.autoplay.growth.spawns_walked = rooms.to_vec();
    let hunt =
        |c: &mut Client, at| c.hunt_ground("Silver Tusker Tusk", "Silver Tusker", 0x5A4A, at);
    assert_eq!(hunt(&mut c, now), Some(Some(true)));
    assert!(c.autoplay.growth.staying.is_some(), "{}", c.autoplay.status);
    for secs in [0.1, 30.0] {
        let later = now + std::time::Duration::from_secs_f32(secs);
        assert_eq!(hunt(&mut c, later), Some(Some(true)));
        assert!(
            c.autoplay.status.contains("waiting for the Silver Tusker") && !c.traveling(),
            "walked round at {secs} s: {}",
            c.autoplay.status
        );
    }
}

/// Blargerton on the Tusker Guard Kill Counter, stood by the Tusker Barracks portal (F07F0039),
/// set off for its dungeon, with a Tusker Guard (`guid`) 83 m off and 9 m down in F17E.
fn at_the_tusker_barracks_portal(now: Instant, guid: u32) -> Client {
    let mut c = mage_pursuing("tuskergkillcount", "tusker-guards");
    c.world.stats.level = 100;
    c.autoplay.growth.proven_tier = 6;
    c.travel.quests_done.push("tuskergkillcount".into());
    c.quest_log.note_taken("tuskerg");
    c.quest_log.note_taken("tuskergkillcount");
    crate::testkit::stand(&mut c, 0xF07F_0039, Vec3::new(178.7, 12.0, 12.0));
    again(&mut c, now);
    assert!(
        matches!(&c.pursuit[0].step, PursuitStep::Hunt { .. }),
        "{:?}",
        c.pursuit[0].step
    );
    assert_eq!(c.grow_pursuit(now), Some(true), "{}", c.autoplay.status);
    assert!(c.autoplay.growth.bound.is_some(), "{}", c.autoplay.status);
    let wcid = c.autoplay.growth.pursuit_sought[0];
    let mut guard = crate::testkit::creature(guid, "Tusker Guard");
    guard.weenie_class_id = wcid;
    guard.position = crate::testkit::placed(0xF17E_000E, Vec3::new(46303.4, 24330.3, 2.8));
    c.world.objects.insert(guid, guard);
    c.autoplay.growth.plan_secs = Some(1e9);
    c.keep_road_droppers();
    c
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_chase_from_the_road_is_asked_again_not_the_ground_s_plan_every_other_tick() {
    // 29 minutes by the Tusker Barracks portal: the chase let the ground's walk go, the ground's
    // plan replaced the chase's walk, and each walk's give-up clock started again every 4.5 s.
    let now = Instant::now();
    let guid = 0x5000_0907;
    let mut c = at_the_tusker_barracks_portal(now, guid);
    let cfg = crate::autoplay::growth::config::Growth::default();
    let guard = Vec3::new(46303.4, 24330.3, 2.8).truncate();
    let radius = c.autoplay.config.fight.radius;
    for secs in [1, 2, 3] {
        let later = now + Duration::from_secs(secs);
        assert!(c.grow_hunt(later, &cfg), "{}", c.autoplay.status);
        let goal = c.travel_goal_xy().expect("travelling");
        assert!(
            goal.distance(guard) <= radius,
            "at {secs} s not to the guard but {goal:?}: {}",
            c.autoplay.status
        );
    }
    // The chase's journey over with the guard still out of reach: given up, and the ground's
    // journey walked on.
    c.end_trip();
    let later = now + Duration::from_secs(4);
    c.grow_hunt(later, &cfg);
    assert!(c.autoplay.given_up.since(&guid).is_some(), "not given up");
    assert!(c.grow_hunt(later, &cfg), "{}", c.autoplay.status);
    let goal = c.travel_goal_xy().expect("travelling");
    assert!(
        goal.distance(guard) > radius,
        "still to the guard: {}",
        c.autoplay.status
    );
    assert!(
        c.autoplay.growth.bound.is_some(),
        "the ground's walk let go"
    );
}

/// Each change of status in `trace`, with its second, for a failing trace test to show.
fn said(trace: &[crate::walk_trial::Frame]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut last = "";
    for f in trace {
        if f.status != last {
            out.push(format!("{:.1} {:?} {}", f.secs, f.step, f.status));
            last = &f.status;
        }
    }
    out
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn played_by_the_tusker_barracks_portal_the_chase_walks_on_till_in_reach() {
    // Live, the chase and the ground's plan replaced each other every 4.5 s for 29 minutes. Once
    // set off, the chase is walked, not re-planned against the ground's walk, till in reach.
    let now = Instant::now();
    let guid = 0x5000_0907;
    let mut c = at_the_tusker_barracks_portal(now, guid);
    c.autoplay.config.enabled = true;
    c.autoplay.config.growth.town_runs = false;
    let guard = Vec3::new(46303.4, 24330.3, 2.8);
    let radius = c.autoplay.config.fight.radius;
    let trace = crate::walk_trial::play(&mut c, now, 6.0);
    let shown = said(&trace).join("\n");
    let in_reach =
        |f: &crate::walk_trial::Frame| f.at.is_some_and(|at| at.distance(guard) <= radius * 0.8);
    let walked: Vec<_> = trace.iter().take_while(|f| !in_reach(f)).collect();
    assert!(walked.len() >= 40, "in reach at once:\n{shown}");
    for f in &walked {
        let to_guard = f
            .goal
            .is_some_and(|g| g.distance(guard.truncate()) <= radius);
        assert!(to_guard, "at {:.2} s to {:?}:\n{shown}", f.secs, f.goal);
    }
    let start = trace[0].at.unwrap().distance(guard);
    let end = walked.last().and_then(|f| f.at).unwrap().distance(guard);
    assert!(end < start - 5.0, "no nearer: {start:.1} -> {end:.1} m");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn played_over_an_aun_ralirea_ground_s_edge_the_ground_is_swept_not_walked_to() {
    // Stood in 0x1AB6 within reach of the 0x19B6 ground: planned and "arrived" every tick.
    let now = Instant::now();
    let mut c = mage_pursuing("championkillquest1", "aun-ralirea");
    c.world.stats.level = 120;
    c.autoplay.growth.proven_tier = 8;
    c.autoplay.config.enabled = true;
    c.autoplay.config.growth.town_runs = false;
    crate::testkit::stand(&mut c, 0x1AB6_0004, Vec3::new(4.6, 87.8, 76.0));
    let ground = 0x19B6;
    let trace = crate::walk_trial::play(&mut c, now, 4.0);
    let shown = said(&trace).join("\n");
    let place = c.pursued_place(ground).expect("its ground");
    assert!(
        c.autoplay.growth.swept.since(&ground).is_some(),
        "not swept:\n{shown}"
    );
    for f in &trace {
        let to_it = f
            .goal
            .is_some_and(|g| g.distance(place.at.truncate()) <= GROUND_REACH);
        assert!(
            f.bound != Some(ground) && !to_it,
            "walked to at {:.2} s:\n{shown}",
            f.secs
        );
    }
}

/// The Stone Fists Token (give:34454) on the bundled data: the working step for a level 150
/// character holding `held`, the events `on` as given, killing what `kills` says it can.
fn stone_fists_step(held: &[&str], on: &[&str], kills: &dyn Fn(u32) -> bool) -> PursuitStep {
    let mut f = facts(150, &[("give:34454", true)]);
    f.held = held.iter().map(|h| h.to_string()).collect();
    let on = |e: &str| on.contains(&e);
    let secs = |w: u32| kills(w).then_some(20.0);
    let p = work_events(&f, &ChainData, None, &|_| false, &secs, &on);
    p.lines[0].step.clone()
}

#[test]
fn stone_fists_a_character_who_cannot_kill_the_boss_is_told_so_and_not_sent_for_its_event() {
    // The boss is level 125 and comes only once eight golems die before its watcher: no golem
    // is worth killing, nor a flag worth earning, for a boss this character cannot kill.
    let step = stone_fists_step(&[], &[], &|w| w != 34607);
    assert_eq!(
        step,
        PursuitStep::Stuck {
            why: "Stone Fists Token drops only from creatures this character cannot kill yet"
                .into()
        }
    );
}

#[test]
fn a_flag_a_standing_portal_stamps_is_had_by_going_through_it() {
    // HarbingerEssenceless is stamped going through Harbinger's Lair (its Portal emote set,
    // Portal.cs:287), which stands from its landblock's load.
    let f = facts(150, &[]);
    let carried = |_| 0;
    let asking = Asking {
        level: f.level,
        held: &f.held,
        carried: &carried,
        refused: &|_, _| false,
        away: &|_| 0.0,
    };
    let held = BTreeMap::new();
    let arena = arena::ArenaView::default();
    let ask = Ask {
        facts: &f,
        asking: &asking,
        tried: &|_: &Unlock| false,
        held: &held,
        led: None,
        chains: &ChainData,
        dropped: &|_| false,
        unlocked: &|_| false,
        ways: &BTreeMap::new(),
        gem_grounds: &Default::default(),
        flag_ways: &BTreeMap::new(),
        here: None,
        bound: None,
        kill_secs: &|_| Some(20.0),
        event_on: &|_| false,
        arena: &arena,
    };
    match ask.flag_step("HarbingerEssenceless", 0).0 {
        PursuitStep::GoThrough {
            wcid, name, flag, ..
        } => assert_eq!(
            (wcid, name.as_str(), flag.as_str()),
            (81000, "Harbinger's Lair", "HarbingerEssenceless")
        ),
        other => panic!("{other:?}"),
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_portal_is_gone_through_and_its_flag_held_once_the_teleport_comes() {
    let (mut c, line) = fetching(|at| PursuitStep::GoThrough {
        wcid: 81000,
        name: "Harbinger's Lair".into(),
        at,
        flag: "HarbingerEssenceless".into(),
    });
    let PursuitStep::GoThrough { at, .. } = line.step else {
        unreachable!()
    };
    lying(&mut c, 0x7AB0_2001, 81000, "Harbinger's Lair", at);
    let now = Instant::now();
    assert_eq!(c.grow_arena(&line, now), Some(true));
    let sent = c.autoplay.growth.arena_deeding.expect("used");
    assert_eq!((sent.deed, sent.wcid), (arena::Deed::Enter, 81000));
    assert!(!c.holds_quest_flag("HarbingerEssenceless"));
    let soon = now + Duration::from_millis(500);
    c.teleported_at = Some(soon);
    assert_eq!(c.grow_arena(&line, soon), Some(true));
    assert!(c.holds_quest_flag("HarbingerEssenceless"));
    assert_eq!(c.autoplay.growth.arena_deeding, None);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn an_arena_bell_is_rung_and_a_ticket_handed_back_passes_the_quest_over() {
    let ring = |at| PursuitStep::Arena {
        flag: "accessboss3".into(),
        step: arena::Stage::Ring {
            room: 1,
            wcid: 34604,
            at,
        },
    };
    let (mut c, line) = fetching(ring);
    let PursuitStep::Arena {
        step: arena::Stage::Ring { at, .. },
        ..
    } = line.step
    else {
        unreachable!()
    };
    lying(&mut c, 0x7AB0_2002, 34604, "Arena Bell", at);
    let now = Instant::now();
    assert_eq!(c.grow_arena(&line, now), Some(true));
    let soon = now + Duration::from_millis(300);
    c.use_done = Some((0, soon));
    assert_eq!(c.grow_arena(&line, soon), Some(true));
    assert_eq!(c.autoplay.growth.arena_rung, [34604]);
    // The Master Arbitrator hands a ticket back with no arena free (EmoteManager.cs:500-517).
    let hand = |at| PursuitStep::Arena {
        flag: "accessboss3".into(),
        step: arena::Stage::HandTicket {
            giver: "Master Arbitrator".into(),
            wcid: 34441,
            item: 34449,
            at,
        },
    };
    let (mut c, line) = fetching(hand);
    let PursuitStep::Arena {
        step: arena::Stage::HandTicket { at, .. },
        ..
    } = line.step
    else {
        unreachable!()
    };
    lying(&mut c, 0x7AB0_2003, 34441, "Master Arbitrator", at);
    carrying(&mut c, 0x8000_0901, 34449, "Colosseum Ticket");
    assert_eq!(c.grow_arena(&line, now), Some(true));
    let sent = c.autoplay.growth.arena_deeding.expect("given");
    assert_eq!(sent.deed, arena::Deed::Give(34449));
    let later = now + fetch::ANSWER_WAIT;
    assert_eq!(c.grow_arena(&line, later), Some(false));
    assert!(c.autoplay.growth.pursuit_held.contains_key(&line.id));
    assert_eq!(c.autoplay.growth.arena_access, None);
}

/// The Stone Fists Token's working step at level 150, carrying `carried`, the arena seen as `v`.
fn arena_step_with(carried: &[u32], v: &arena::ArenaView) -> PursuitStep {
    let mut f = facts(150, &[("give:34454", true)]);
    f.carried = carried.iter().map(|&w| (w, 1)).collect();
    let p = work_in(
        &f,
        &ChainData,
        None,
        &|_| false,
        &|_| Some(20.0),
        &|_| false,
        v,
    );
    p.lines[0].step.clone()
}

#[test]
fn stone_fists_the_arena_flag_is_had_by_running_the_colosseum_s_rooms_one_to_two() {
    // AccessBoss3 is stamped only going through Arena 3, which room 2's waves passed reveal; room
    // 2 is reached through room 1's Arena 2; the arena by the Colosseum Arena's teleport for a
    // fellowship the Master Arbitrator let in for a ticket (arenas.csv, EmoteManager.cs:500-517).
    let stage = |s: &PursuitStep| match s {
        PursuitStep::Arena { step, .. } => Some(step.clone()),
        _ => None,
    };
    let mut v = arena::ArenaView::default();
    assert_eq!(
        stage(&arena_step_with(&[], &v)),
        Some(arena::Stage::Fellowship)
    );
    v.team_unfellowed = true;
    assert!(matches!(
        arena_step_with(&[], &v),
        PursuitStep::Stuck { .. }
    ));
    v.team_unfellowed = false;
    v.fellowship = true;
    let note = 20630;
    // The note itself, had from wherever is nearest (a shop, the Hermit's talk).
    match arena_step_with(&[], &v) {
        PursuitStep::Buy { wcid, .. } => assert_eq!(wcid, note),
        PursuitStep::AskFor { source, .. } => assert_eq!(source.want, Want::Item(note)),
        other => panic!("the ticket's trade note: {other:?}"),
    }
    match arena_step_with(&[note], &v) {
        PursuitStep::HandIn { row, .. } => assert_eq!(row.giver, "Colosseum Ticket Vendor"),
        other => panic!("the note handed for the ticket: {other:?}"),
    }
    let ticket = 34449;
    assert!(matches!(
        stage(&arena_step_with(&[ticket], &v)),
        Some(arena::Stage::HandTicket { wcid: 34441, .. })
    ));
    v.access = true;
    assert!(matches!(
        stage(&arena_step_with(&[], &v)),
        Some(arena::Stage::Enter { wcid: 34727, .. })
    ));
    // In arena One's room 1, where the teleport lands, then room 2, 90 m north.
    let origin = ac_world::landblock_origin(0x00B0_016B);
    v.at = Some((0xB0, origin + glam::Vec3::new(30.0, -1573.0, 0.0)));
    assert!(matches!(
        stage(&arena_step_with(&[], &v)),
        Some(arena::Stage::Ring {
            room: 1,
            wcid: 34604,
            ..
        })
    ));
    v.rung = vec![34604];
    assert!(matches!(
        stage(&arena_step_with(&[], &v)),
        Some(arena::Stage::Clear { room: 1, .. })
    ));
    let spot = |y: f32| ac_world::Place {
        at: origin + glam::Vec3::new(30.0, y, 0.0),
        cell: 0x00B0_0169,
    };
    v.portals = vec![(34517, spot(-1546.0))];
    assert!(matches!(
        stage(&arena_step_with(&[], &v)),
        Some(arena::Stage::Leave {
            room: 1,
            wcid: 34517,
            ..
        })
    ));
    v.at = Some((0xB0, origin + glam::Vec3::new(30.0, -1483.0, 0.0)));
    v.portals.clear();
    assert!(matches!(
        stage(&arena_step_with(&[], &v)),
        Some(arena::Stage::Ring {
            room: 2,
            wcid: 34718,
            ..
        })
    ));
    v.portals = vec![(34518, spot(-1456.0))];
    assert!(matches!(
        stage(&arena_step_with(&[], &v)),
        Some(arena::Stage::Leave {
            room: 2,
            wcid: 34518,
            ..
        })
    ));
}

#[test]
fn stone_fists_with_the_flag_the_golems_are_hunted_then_the_boss_once_its_event_is_on() {
    let hunt = |s: &PursuitStep| match s {
        PursuitStep::Hunt {
            landblock,
            creature,
            ..
        } => (*landblock, creature.clone()),
        other => panic!("{other:?}"),
    };
    let (lb, golem) = hunt(&stone_fists_step(&["AccessBoss3"], &[], &|_| true));
    assert_eq!(lb, 0xB6);
    assert!(
        ["Magma Golem", "Crystal Golem"].contains(&golem.as_str()),
        "{golem}"
    );
    let on = ["stonefistsbossstart"];
    let (lb, boss) = hunt(&stone_fists_step(&["AccessBoss3"], &on, &|_| true));
    assert_eq!((lb, boss.as_str()), (0xB6, "Stone Fists"));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn played_at_the_silver_tuskers_rooms_the_wait_is_stood_till_it_ends() {
    // The Tusker Pits' four Silver Tusker rooms, 7-18 m apart: the 90 s wait lasted a tick and a
    // round of the rooms was walked every 1.7 s. Gone round, the wait is stood: no walk, no step.
    let now = Instant::now();
    let mut c = mage_pursuing("give:22431", "silver-tusk");
    c.world.stats.level = 66;
    c.autoplay.config.enabled = true;
    c.autoplay.config.growth.town_runs = false;
    crate::testkit::stand(&mut c, 0x5A4A_01A0, Vec3::new(107.3, -86.5, -42.0));
    again(&mut c, now);
    c.autoplay.growth.spawns_walked = vec![0x5A4A_019A, 0x5A4A_019C, 0x5A4A_019E, 0x5A4A_01A0];
    let trace = crate::walk_trial::play(&mut c, now, 6.0);
    let shown = said(&trace).join("\n");
    let (_, until) = c.autoplay.growth.staying.expect("a wait held");
    assert!(until > now + Duration::from_secs(6), "{shown}");
    let start = trace[0].at.unwrap();
    for f in &trace[1..] {
        assert!(f.goal.is_none(), "walked at {:.2} s:\n{shown}", f.secs);
        assert!(
            f.status.contains("waiting for the Silver Tusker"),
            "{shown}"
        );
        let moved = f.at.unwrap().distance(start);
        assert!(
            moved < 1.0,
            "moved {moved:.1} m by {:.2} s:\n{shown}",
            f.secs
        );
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn blargerton_at_67_founds_a_fellowship_to_run_the_arena_for_the_stone_fists_token() {
    // He pursued give:34454 and stood "nobody known yields the stonefistsbossstart event": the
    // golems its watcher counts and the arena revealing its flag were not in the data. Alone, he
    // founds a fellowship of his own: the Master Arbitrator takes a ticket only from a fellow.
    let mut c = mage_pursuing("give:34454", "stone-fists");
    c.world.stats.level = 67;
    let now = std::time::Instant::now();
    c.refresh_quests(now);
    assert!(
        c.killing().secs(34607).is_some(),
        "his kill reckoning takes the boss on"
    );
    assert_eq!(
        c.pursuit[0].words(0),
        "Stone Fists Token: working: found a fellowship for the arena (the AccessBoss3 flag)"
    );
    assert_eq!(c.grow_pursuit(now), Some(true));
    assert_eq!(c.autoplay.founded, Some(now));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn stone_fists_golems_are_hunted_till_eight_of_our_kills_or_the_boss_is_seen() {
    // Eight is the most golems hunted: other players' kills count to the watcher too, and the
    // boss coming out ends the golems' turn whatever our count (EmoteManager.cs:1287-1290).
    let mut c = mage_pursuing("give:34454", "stone-fists");
    c.world.stats.level = 150;
    c.autoplay.growth.proven_tier = 8;
    c.hold_quest_flags(&["AccessBoss3".to_string()]);
    let now = std::time::Instant::now();
    let golem_hunt = |s: &PursuitStep| {
        matches!(s, PursuitStep::Hunt { creature, .. }
            if creature == "Magma Golem" || creature == "Crystal Golem")
    };
    let step = again(&mut c, now);
    assert!(golem_hunt(&step), "{step:?}");
    let spot = glam::Vec3::new(30.0, -30.0, -11.8);
    for i in 0..3u32 {
        let mut golem = crate::testkit::creature(0x5000_0B00 + i, "Magma Golem");
        golem.weenie_class_id = 199;
        golem.position = crate::testkit::placed(0x00B6_010A, spot);
        let at = golem.world_pos().unwrap();
        c.world.objects.insert(golem.guid, golem);
        c.note_event_kill(at, now);
        c.world.objects.remove(&(0x5000_0B00 + i));
    }
    let step = again(&mut c, now);
    assert!(golem_hunt(&step), "three of eight: {step:?}");
    let mut boss = crate::testkit::creature(0x5000_0C00, "Stone Fists");
    boss.weenie_class_id = 34607;
    boss.position = crate::testkit::placed(0x00B6_010A, spot);
    c.world.objects.insert(boss.guid, boss);
    let step = again(&mut c, now);
    assert!(
        matches!(&step, PursuitStep::Hunt { creature, .. } if creature == "Stone Fists"),
        "{step:?}"
    );
    assert!(c.autoplay.growth.event_deaths.is_empty());
}

#[test]
fn the_stone_fists_watcher_is_taken_for_started_only_after_eight_of_our_golem_kills() {
    // Its watcher stamps a counter per golem death it hears and starts the boss at eight
    // (InqMyQuestSolves StoneFistsCrittersKilled@8-8: EmoteManager.cs:666-676, 1287-1290).
    let mut c = crate::testkit::offline_client();
    let now = std::time::Instant::now();
    for i in 0..8u32 {
        assert!(
            !c.is_event_on("stonefistsbossstart", now),
            "after {i} kills"
        );
        let mut golem = crate::testkit::creature(0x5000_0A00 + i, "Magma Golem");
        golem.weenie_class_id = if i % 2 == 0 { 199 } else { 14800 };
        golem.position = crate::testkit::placed(0x00B6_010A, glam::Vec3::new(30.0, -30.0, -11.8));
        let at = golem.world_pos().unwrap();
        c.world.objects.insert(golem.guid, golem);
        c.note_event_kill(at, now);
        c.world.objects.remove(&(0x5000_0A00 + i));
    }
    assert!(c.is_event_on("stonefistsbossstart", now));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_follower_goes_into_the_arena_and_through_a_room_after_its_leader() {
    // The leader's fellowship holds the arena's access quest, so the Colosseum Arena sends its
    // fellows in too (InqFellowQuest, EmoteManager.cs:500-517); its rooms are left by portals.
    let hall = 0x00AF_010F;
    let origin = ac_world::landblock_origin(hall);
    let mut c = crate::testkit::standing_in_the_field(80, hall, Vec3::new(31.5, 4.5, 0.0));
    let npc = ac_world::Place {
        at: origin + Vec3::new(30.81, 4.51, 0.0),
        cell: hall,
    };
    lying(&mut c, 0x7AB0_3001, 34727, "Colosseum Arena", npc);
    let arena = ac_world::landblock_origin(0x00B0_016B);
    let mut leader = crate::testkit::mate(0x5000_0001, "Lead");
    leader.leads = true;
    leader.cell = 0x00B0_0167;
    leader.world = arena + Vec3::new(30.0, -1470.0, 0.0);
    let now = Instant::now();
    assert_eq!(c.follow_arena(&leader, now), Some(true));
    let sent = c.autoplay.growth.arena_deeding.expect("used");
    assert_eq!((sent.deed, sent.wcid), (arena::Deed::Enter, 34727));
    // Sent into room 1 beside its portal, with the leader a room on.
    let soon = now + Duration::from_millis(500);
    c.teleported_at = Some(soon);
    assert_eq!(c.follow_arena(&leader, soon), Some(true));
    crate::testkit::stand(&mut c, 0x00B0_0169, Vec3::new(30.0, -1547.5, 0.0));
    let portal = ac_world::Place {
        at: arena + Vec3::new(30.0, -1546.0, 0.0),
        cell: 0x00B0_0169,
    };
    lying(&mut c, 0x7AB0_3002, 34517, "Arena 2", portal);
    let later = soon + Duration::from_secs(1);
    assert_eq!(c.follow_arena(&leader, later), Some(true));
    let sent = c.autoplay.growth.arena_deeding.expect("used");
    assert_eq!((sent.deed, sent.wcid), (arena::Deed::Enter, 34517));
    // Beside the leader in its room: ordinary following.
    leader.world = arena + Vec3::new(30.0, -1547.0, 0.0);
    c.autoplay.growth.arena_deeding = None;
    assert_eq!(c.follow_arena(&leader, later), None);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_bar_carried_to_the_repair_golem_is_worked_by_the_pursuit_not_passed_to_the_hunt() {
    // Blargerton took the Sluice Gate Bar, the board said to give it to the Repair Golem, and
    // the pursuit answered nothing: the XP hunt sent him to the Colosseum instead.
    let mut c = mage_pursuing("give:40913", "aerfalle");
    let now = std::time::Instant::now();
    for (guid, wcid, name) in [
        (0x8000_0701, 28065, "Sacrificial Dagger"),
        (0x8000_0702, 7411, "Pyreal Forge Bellows"),
        (0x8000_0703, 7412, "Sluice Gate Bar"),
    ] {
        in_the_pack(&mut c, guid, wcid, name, 1);
    }
    let step = again(&mut c, now);
    assert!(
        matches!(&step, PursuitStep::GiveTo { source, item: 7412, .. } if source.wcid == 7404),
        "{step:?}"
    );
    // From Holtburg the way to the golem is Garaena's, for a Trade Note: that is wanted first.
    assert_eq!(c.grow_pursuit(now), Some(false), "the give passed over");
    let gems = &c.autoplay.growth.pursuit_gems;
    assert!(
        gems.iter()
            .any(|g| g.quest == "give:40913" && g.name.starts_with("Trade Note")),
        "{gems:?}"
    );
}
