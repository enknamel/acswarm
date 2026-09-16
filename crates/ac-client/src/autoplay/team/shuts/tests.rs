use std::time::{Duration, Instant};

use super::*;
use crate::autoplay::loot::choose::{LOOT_NEAR, LOOT_TIMEOUT};
use crate::autoplay::{
    judge_loot, skills_asked_of, LootAction, Room, ShutFor, TeamView, CLAIM_SETTLE, CORPSE_LIFE,
};
use crate::items::ItemStats;
use crate::testkit::{
    a_party_profile, asks, character_of_level, corpses_seen, game_data, item, looter, no_data,
    standing_in_the_field, turn_at, view_of,
};

#[test]
fn a_shut_body_says_who_shut_it_what_it_took_and_whom_it_is_left_for() {
    // A nine-character run's log named nobody, so nobody could say who
    // opened a body first or who came back to it for nothing.
    let names = |of: &[&str]| of.iter().map(|n| n.to_string()).collect::<Vec<_>>();
    let body = 0x8000_198f;
    assert_eq!(
        shut_line(
            "Bryn01",
            "Corpse of Biaka",
            body,
            3,
            &names(&["Bryn02", "Bryn03"]),
            &names(&["Bryn04"]),
            &names(&["Bryn05"]),
        ),
        "autoplay: Bryn01 shut Corpse of Biaka (0x8000198f), took 3; \
         done for Bryn02, Bryn03; left for Bryn04; Bryn05 standing by"
    );
    // Alone, it says only what it did.
    assert_eq!(
        shut_line("Bryn01", "Corpse of Biaka", body, 0, &[], &[], &[]),
        "autoplay: Bryn01 shut Corpse of Biaka (0x8000198f), took 0"
    );
    assert_eq!(
        shut_line(
            "Bryn01",
            "Corpse of Biaka",
            body,
            1,
            &[],
            &names(&["Bryn04"]),
            &[]
        ),
        "autoplay: Bryn01 shut Corpse of Biaka (0x8000198f), took 1; left for Bryn04"
    );
    // The one that stays away says whose word it took.
    assert_eq!(
        taken_in_line("Bryn02", "Corpse of Biaka", body, "Bryn01"),
        "autoplay: Bryn02: Corpse of Biaka (0x8000198f) done for me by Bryn01"
    );
    assert_eq!(
        standing_by_line(
            "Bryn05",
            "Corpse of Biaka",
            body,
            "Bryn01",
            &names(&["Bryn04"])
        ),
        "autoplay: Bryn05: Corpse of Biaka (0x8000198f) left for Bryn04 by Bryn01; standing by"
    );
}

/// A mate whose board row says it has `lockpick` Lockpick.
fn picking(guid: u32, lockpick: u32) -> Mate {
    use ac_world::stats::{sac, skill};
    Mate {
        level: 30,
        skills: vec![(skill::LOCKPICK, lockpick, lockpick, sac::TRAINED)],
        ..looter(guid, glam::Vec3::ZERO, None, Duration::ZERO)
    }
}

/// A fellow as a shut judges it, off its row.
fn as_fellow(m: &Mate) -> Fellow {
    (m.guid, m.name.clone(), m.wielder())
}

/// A thing lying on a body, this character carrying `held` of it.
fn lying(stats: ItemStats, held: u32) -> Option<Left<'static>> {
    Some(Left {
        stats,
        id: None,
        held,
    })
}

#[test]
fn a_mate_is_judged_by_the_skills_it_said_about_itself() {
    // One profile for the party, read differently for each of it: the
    // broken key is the lockpicker's to take and not the mage's. Judged
    // by another character, each is read with what its row said.
    use crate::profile::Verdict;
    let profile = a_party_profile();
    let heard = |said: Mate| -> Mate {
        serde_json::from_value(serde_json::to_value(&said).unwrap()).unwrap()
    };
    let (picker, mage) = (heard(picking(2, 300)), heard(picking(3, 5)));
    let key = item("Broken Marble Key", 0, 0);
    let judge = |m: &Mate| judge_loot(&key, None, Some(&profile), &m.wielder(), &m.name, 0);
    assert_eq!(
        judge(&picker),
        Verdict::Decided(LootAction::Keep, "broken keys, if I can mend them".into())
    );
    assert_eq!(judge(&mage), Verdict::None);
    // Shut by one that cannot mend it: left for the lockpicker, done for
    // the mage, and nothing the opener would take left on it.
    let opener = crate::weapons::Wielder::default();
    let me = Taker {
        guid: 1,
        name: "Bryn01",
        sheet: &opener,
        held: 0,
    };
    let fellows = [as_fellow(&picker), as_fellow(&mage)];
    assert_eq!(
        judge_shut(&[lying(key, 0)], &profile, me, &fellows, &fellows, None),
        ShutFor {
            left_for: vec![picker.guid],
            done_for: vec![mage.guid],
            ..Default::default()
        }
    );
}

#[test]
fn a_fellow_that_would_take_what_was_left_for_another_stands_by_and_is_never_told_it_is_done() {
    // "Healing kits, while my Healing is 100 or more, up to four", read
    // by four of a party. The kit goes to the best healer, who turns out
    // to carry four already. Told the body was done, the others wrote it
    // off, and the kit lay there though two of them wanted it.
    use ac_world::stats::{sac, skill};
    let mut profile = a_party_profile();
    profile.rules[1]
        .all
        .push(crate::profile::Ask::Me(crate::profile::Mine::Skill {
            skill: skill::HEALING,
            op: crate::items::Op::Ge,
            level: 100,
        }));
    let healer = |guid: u32, healing: u32| Mate {
        skills: vec![(skill::HEALING, healing, healing, sac::TRAINED)],
        ..picking(guid, 0)
    };
    let (a, b, c, d) = (healer(1, 200), healer(2, 300), healer(3, 150), healer(4, 0));
    let kit = item("Healing Kit", 50, 0);

    // A opens it carrying two.
    let a_sheet = a.wielder();
    let a_opens = Taker {
        guid: a.guid,
        name: &a.name,
        sheet: &a_sheet,
        held: 2,
    };
    let others = [as_fellow(&b), as_fellow(&c), as_fellow(&d)];
    assert_eq!(
        judge_shut(
            &[lying(kit.clone(), 2)],
            &profile,
            a_opens,
            &others,
            &others,
            None
        ),
        ShutFor {
            left_for: vec![b.guid],
            // C would take the kit too: it stands by for B.
            stand_by: vec![c.guid],
            // D cannot heal, and nothing on the body is for it.
            done_for: vec![d.guid],
            // A would take it too, and stands by rather than writing the
            // body off.
            waits: true,
            ..Default::default()
        }
    );

    // B carries four and leaves it. Nothing goes back to A, which shut
    // the body: the kit is C's now, and A stands by for C.
    let b_sheet = b.wielder();
    let b_opens = Taker {
        guid: b.guid,
        name: &b.name,
        sheet: &b_sheet,
        held: 4,
    };
    let judged = [as_fellow(&a), as_fellow(&c), as_fellow(&d)];
    let sendable = [as_fellow(&c), as_fellow(&d)];
    assert_eq!(
        judge_shut(
            &[lying(kit, 4)],
            &profile,
            b_opens,
            &judged,
            &sendable,
            None
        ),
        ShutFor {
            left_for: vec![c.guid],
            stand_by: vec![a.guid],
            done_for: vec![d.guid],
            ..Default::default()
        }
    );
}

#[test]
fn a_body_stood_by_for_a_fellow_that_cannot_come_is_opened_again() {
    // Salvage was left on every body for the one salvager, and the rest
    // wrote each body off: when the salvager died, followed its leader
    // out of reach or filled its pack first, the salvage lay there until
    // the body rotted.
    use crate::did::Did;
    let t0 = Instant::now();
    let s = Duration::from_secs;
    let (body, at) = (0x8000_0001, glam::Vec3::ZERO);
    let salvager = looter(2, at, None, Duration::ZERO);
    let mut ap = Autoplay {
        team: view_of(vec![salvager.clone()]),
        ..Default::default()
    };
    ap.config.team.enabled = true;
    let left = ShutFor {
        at,
        left_for: vec![salvager.guid],
        waits: true,
        ..Default::default()
    };
    ap.corpse_shut(body, &Did::Done, None, left, t0);
    assert!(
        !ap.looted.contains(&body),
        "wrote off what it would take itself"
    );
    assert!(
        !ap.corpse_waiting(body, t0 + s(1), Room::PLENTY),
        "did not leave it to the salvager"
    );
    let packed = crate::logistics::Supplies {
        laden: true,
        ..Default::default()
    };
    for (why, row) in [
        (
            "dead",
            Some(Mate {
                health: 0.0,
                ..salvager.clone()
            }),
        ),
        (
            "played by hand",
            Some(Mate {
                autoplay: false,
                ..salvager.clone()
            }),
        ),
        (
            "out of reach",
            Some(Mate {
                world: glam::Vec3::new(LOOT_NEAR + 5.0, 0.0, 0.0),
                ..salvager.clone()
            }),
        ),
        (
            "laden",
            Some(Mate {
                supplies: packed.clone(),
                ..salvager.clone()
            }),
        ),
        (
            "not looting",
            Some(Mate {
                opens_bodies: false,
                ..salvager.clone()
            }),
        ),
        ("off the board", None),
    ] {
        ap.team = view_of(row.into_iter().collect());
        assert!(
            ap.corpse_waiting(body, t0 + s(1), Room::PLENTY),
            "waited on a salvager {why}"
        );
    }
    // Able to come, it is waited on, but not for ever.
    ap.team = view_of(vec![salvager.clone()]);
    assert!(!ap.corpse_waiting(body, t0 + STAND_BY_FOR - s(1), Room::PLENTY));
    let later = t0 + STAND_BY_FOR;
    assert!(
        ap.corpse_waiting(body, later, Room::PLENTY),
        "waited for good on one that never came"
    );
    // Given up on, it is left nothing on that body again, however able
    // it looks: left it again, it was stood by for again, and again.
    ap.stop_standing_by(later);
    assert!(!ap.may_be_sent(body, &salvager, at));
    assert!(ap.may_be_sent(0x8000_0002, &salvager, at));
    assert!(ap.corpse_waiting(body, later, Room::PLENTY));
    // Opened again and emptied, it is done with, and going back to it
    // was no turn.
    ap.corpse_shut(body, &Did::Done, None, ShutFor::default(), later);
    assert!(ap.looted.contains(&body));
    assert_eq!(ap.opened_first(later), 1);
}

#[test]
fn a_fellow_standing_by_goes_by_the_newest_word_on_the_body() {
    let t0 = Instant::now();
    let (body, at) = (0x8000_0001, glam::Vec3::ZERO);
    let (a, b, me, mage) = (1, 2, 3, 4);
    let row = |guid: u32, shut: Vec<Shut>| Mate {
        shut,
        ..looter(guid, at, None, Duration::ZERO)
    };
    let mut ap = Autoplay::default();
    ap.config.enabled = true;
    ap.config.team.enabled = true;
    // A shut it and left the kit on it for B. This character would take
    // the kit too, and stands by for B; the mage wants nothing on it.
    let a_shut = Shut {
        body,
        n: 0,
        done_for: vec![mage],
        stand_by: vec![me],
        left_for: vec![b],
    };
    ap.team = view_of(vec![row(a, vec![a_shut.clone()]), row(b, Vec::new())]);
    assert_eq!(
        ap.take_in_shuts(me, t0, |_| Some(at)),
        vec![TakenIn::StandBy {
            body,
            by: "Bryn01".into(),
            on: vec![b],
        }]
    );
    assert!(!ap.corpse_waiting(body, t0, Room::PLENTY));
    // Nothing on it is left for the mage, told it is done with it,
    // whatever a buff makes of its skills later.
    assert!(!ap.may_judge(body, &looter(mage, at, None, Duration::ZERO), at));
    assert!(ap.may_be_sent(body, &looter(b, at, None, Duration::ZERO), at));

    // B carried its fill already, and shut it leaving the kit to anyone
    // that wants it: this character opens it.
    let b_shut = Shut {
        body,
        n: 7,
        done_for: vec![mage],
        ..Default::default()
    };
    ap.team = view_of(vec![
        row(a, vec![a_shut.clone()]),
        row(b, vec![b_shut.clone()]),
    ]);
    assert!(ap.take_in_shuts(me, t0, |_| Some(at)).is_empty());
    assert!(
        ap.corpse_waiting(body, t0, Room::PLENTY),
        "the kit lay there though it was wanted"
    );
    // Nothing on it goes back to either that shut it.
    for shutter in [a, b] {
        assert!(!ap.may_be_sent(body, &looter(shutter, at, None, Duration::ZERO), at));
    }
    // The same shuts heard again change nothing.
    assert!(ap.take_in_shuts(me, t0, |_| Some(at)).is_empty());
    assert!(ap.corpse_waiting(body, t0, Room::PLENTY));
    // A's next shut of it, having opened it again, is new word.
    let again = Shut {
        body,
        n: 1,
        done_for: vec![me, mage],
        ..Default::default()
    };
    ap.team = view_of(vec![row(a, vec![again]), row(b, vec![b_shut])]);
    assert_eq!(
        ap.take_in_shuts(me, t0, |_| Some(at)),
        vec![TakenIn::Done {
            body,
            by: "Bryn01".into()
        }]
    );
    assert!(!ap.corpse_waiting(body, t0, Room::PLENTY));
}

#[test]
fn every_body_shut_as_emptied_is_said_so_the_others_know_who_shut_it() {
    // Said only when it was done for somebody, a shut that left
    // something for every fellow went unheard: going back to the body
    // counted as a turn, and things on it were left for the one that
    // had shut it already, which never came back.
    use crate::did::Did;
    let t0 = Instant::now();
    let (body, at) = (0x8000_0001, glam::Vec3::ZERO);
    let mut opener = Autoplay::default();
    opener.config.team.enabled = true;
    opener.corpse_shut(body, &Did::Done, None, ShutFor::default(), t0);
    let said = opener.shuts_to_say(t0);
    assert_eq!(
        said,
        vec![Shut {
            body,
            ..Default::default()
        }]
    );

    let mut ap = Autoplay {
        team: view_of(vec![Mate {
            shut: said,
            ..looter(1, at, None, Duration::ZERO)
        }]),
        ..Default::default()
    };
    ap.config.enabled = true;
    ap.config.team.enabled = true;
    assert!(ap.take_in_shuts(2, t0, |_| Some(at)).is_empty());
    assert!(ap.corpse_waiting(body, t0, Room::PLENTY));
    assert!(!ap.may_be_sent(body, &looter(1, at, None, Duration::ZERO), at));
    ap.corpse_shut(body, &Did::Done, None, ShutFor::default(), t0);
    assert_eq!(ap.opened_first(t0), 0, "going back to it counted as a turn");
}

#[test]
fn a_fellow_that_does_not_open_bodies_is_never_dealt_one_or_left_anything() {
    // An Ust carrier with no loot profile salvaged best and stood a few
    // metres from the leader, and never opened a body: the salvage left
    // for it rotted. One down to the slots kept for a sale opens none
    // either.
    let (body, at) = (0x8000_0001, glam::Vec3::ZERO);
    let ust = Mate {
        has_ust: true,
        salvaging: 400,
        opens_bodies: false,
        ..looter(2, at, None, Duration::ZERO)
    };
    assert_eq!(ust.turn(), None);
    assert!(!ust.could_come_for(at));
    let ap = Autoplay::default();
    assert!(!ap.may_judge(body, &ust, at));
    assert!(!ap.may_be_sent(body, &ust, at));
    // Not dealt a newly fallen body, though it has had no turns.
    let me = 3;
    assert_eq!(
        view_of(vec![ust.clone()]).opens_first(body, at, turn_at(me, at, 5)),
        me
    );
    // So its salvage is nobody's in particular: whoever opens the body
    // takes it, and the hand-off carries it to the salvager as ever.
    let profile = crate::profile::Profile {
        name: "party".into(),
        rules: vec![asks(
            "platemail to salvage",
            "platemail",
            LootAction::Salvage,
        )],
        ..Default::default()
    };
    let sheet = crate::weapons::Wielder::default();
    let taker = Taker {
        guid: me,
        name: "Bryn03",
        sheet: &sheet,
        held: 0,
    };
    let plate = item("Platemail", 100, 240);
    assert_eq!(
        called_to(&plate, None, &profile, &[taker], Some(ust.guid)),
        None
    );
}

#[test]
fn only_fellows_that_could_open_a_body_are_judged_at_its_shut() {
    // A character played by hand was named in "left for", and a body
    // found done for it was written off while its rules were off: the
    // player turned them on beside the body, and it was walked past.
    let t0 = Instant::now();
    let (body, at) = (0x8000_0001, glam::Vec3::ZERO);
    let ap = Autoplay::default();
    let able = looter(2, at, None, Duration::ZERO);
    assert!(ap.may_judge(body, &able, at));
    for (why, m) in [
        (
            "played by hand",
            Mate {
                autoplay: false,
                ..able.clone()
            },
        ),
        (
            "dead",
            Mate {
                health: 0.0,
                ..able.clone()
            },
        ),
        (
            "across the field",
            Mate {
                world: glam::Vec3::new(LOOT_NEAR + 5.0, 0.0, 0.0),
                ..able.clone()
            },
        ),
        (
            "with a full pack",
            Mate {
                supplies: crate::logistics::Supplies {
                    pack_full: true,
                    ..Default::default()
                },
                ..able.clone()
            },
        ),
    ] {
        assert!(!ap.may_judge(body, &m, at), "judged a fellow {why}");
    }

    // With its own rules off, a character notes who shut the body and
    // writes nothing off.
    let me = 3;
    let mut ap = Autoplay::default();
    ap.config.enabled = false;
    ap.config.team.enabled = true;
    ap.team.mates = vec![Mate {
        shut: vec![Shut {
            body,
            done_for: vec![me],
            ..Default::default()
        }],
        ..looter(1, at, None, Duration::ZERO)
    }];
    assert!(ap.take_in_shuts(me, t0, |_| Some(at)).is_empty());
    assert!(ap.has_shut(body, 1));
    assert!(
        ap.corpse_waiting(body, t0, Room::PLENTY),
        "wrote a body off while played by hand"
    );
}

#[test]
fn a_body_emptied_long_ago_is_forgotten_before_its_guid_can_come_back() {
    // ACE hands a released guid out again once it has been free for six
    // hours. Remembered for a whole long session, a new body that came
    // with an emptied one's guid was never opened.
    let t0 = Instant::now();
    let body = 0x8000_0001;
    let mut ap = Autoplay::default();
    ap.looted.push(body, t0);
    ap.looted
        .forget_old(t0 + EMPTIED_KEPT - Duration::from_secs(1));
    assert!(!ap.corpse_waiting(body, t0 + CORPSE_LIFE, Room::PLENTY));
    ap.looted.forget_old(t0 + EMPTIED_KEPT);
    assert!(
        ap.corpse_waiting(body, t0 + EMPTIED_KEPT, Room::PLENTY),
        "a new body with an old guid was never opened"
    );
}

#[test]
fn a_session_deals_itself_by_what_it_last_said_about_itself() {
    // The others read this character off its row, up to half a second
    // old. Read as it is now -- at another body, three turns in -- it
    // dealt a newly fallen body to a mate while the mate, reading the
    // row, dealt it back, and for a second nobody opened it.
    let t0 = Instant::now();
    let (body, other_body, at) = (0x8000_0001, 0x8000_0002, glam::Vec3::ZERO);
    let me = 0x5000_0011;
    let mut ap = Autoplay {
        corpse_seen: corpses_seen([(body, t0)]),
        first_opens: vec![t0; 3],
        team: view_of(vec![Mate {
            opened_first: 1,
            ..looter(0x5000_0012, at, None, Duration::ZERO)
        }]),
        ..Default::default()
    };
    ap.take_up_corpse(other_body, t0, LOOT_TIMEOUT);
    // What it last said: free, and no turns yet.
    ap.team.me = Some(looter(me, at, None, Duration::ZERO));
    assert!(
        ap.ours_to_open(body, at, me, at, t0),
        "dealt itself out on what the others had not heard"
    );
    // Once it has said it is at another body, with three turns, the
    // mate has it, as the mate reckons too.
    ap.team.me = Some(Mate {
        opened_first: 3,
        ..looter(me, at, Some(other_body), Duration::ZERO)
    });
    assert!(!ap.ours_to_open(body, at, me, at, t0));
}

#[test]
fn only_the_skills_the_rules_ask_about_go_on_the_row() {
    use ac_world::stats::{sac, skill};
    let me = crate::weapons::Wielder {
        level: 30,
        skills: vec![
            (skill::LOCKPICK, 200, 260, sac::TRAINED),
            (skill::SALVAGING, 100, 100, sac::TRAINED),
            (skill::WAR_MAGIC, 300, 340, sac::SPECIALIZED),
        ],
        ..Default::default()
    };
    // Buffs and all: a rule on a skill reads it as it stands.
    assert_eq!(
        skills_asked_of(&me, &[skill::LOCKPICK]),
        vec![(skill::LOCKPICK, 200, 260, sac::TRAINED)]
    );
    // A skill asked about that the sheet lacks is not made up: off the
    // row it reads as nothing, as it does for the character itself.
    let row = Mate {
        skills: skills_asked_of(&me, &[skill::LOCKPICK, skill::HEALING]),
        ..Default::default()
    };
    assert_eq!(row.skills.len(), 1);
    assert_eq!(
        row.wielder().skill(skill::HEALING),
        me.skill(skill::HEALING)
    );
    assert_eq!(row.wielder().skill(skill::LOCKPICK), 260);
    // Rules that ask about no skill put none on the row.
    assert!(skills_asked_of(&me, &[]).is_empty());
}

#[test]
fn a_body_one_of_the_others_emptied_for_everyone_is_not_opened_again() {
    // Nine characters opened 83 bodies 697 times, and 42% of the opens
    // took nothing: the others opened a body one of them had emptied,
    // to find it so. The one that shuts it says whom it found nothing
    // left on it for, and those leave it alone.
    let t0 = Instant::now();
    let (me, other) = (2, 3);
    let (body, another) = (0x8000_0001, 0x8000_0002);
    let (here, at) = (glam::Vec3::ZERO, glam::Vec3::new(3.0, 0.0, 0.0));
    let mut ap = Autoplay {
        corpse_seen: corpses_seen([(body, t0), (another, t0)]),
        ..Default::default()
    };
    ap.config.enabled = true;
    ap.config.team.enabled = true;
    ap.left_for_weight.insert(body, 300);
    assert!(ap.corpse_owed(body, at, here, t0, Room::PLENTY));
    ap.team.mates = vec![Mate {
        shut: vec![Shut {
            body,
            done_for: vec![me, other],
            ..Default::default()
        }],
        ..looter(1, here, None, Duration::ZERO)
    }];
    assert_eq!(
        ap.take_in_shuts(me, t0, |_| None),
        vec![TakenIn::Done {
            body,
            by: "Bryn01".into()
        }]
    );
    assert!(
        !ap.corpse_waiting(body, t0, Room::PLENTY),
        "went to open it again"
    );
    assert!(!ap.corpse_owed(body, at, here, t0, Room::PLENTY));
    // Nothing waits on room for it any more.
    assert_eq!(ap.lightest_left_for_weight(|g| g == body), None);
    // The rest of the ground is as it was.
    assert!(ap.corpse_owed(another, at, here, t0, Room::PLENTY));
    // Heard again the next round: taken in, and logged, once.
    assert!(ap.take_in_shuts(me, t0, |_| None).is_empty());
    assert_eq!(ap.looted.len(), 1);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_body_left_for_a_mate_still_waits_on_that_mate() {
    // One party, one profile, and a broken key left on a body whose
    // opener cannot mend it. The mage is told the body is done with and
    // stays away; the lockpicker is not, and still goes to it.
    use crate::did::Did;
    let mut c = character_of_level(game_data(), 30);
    let t0 = Instant::now();
    let profile = a_party_profile();
    let (body, key) = (0x8000_3001, 0x8000_3002);
    c.world.objects.insert(
        body,
        ac_world::WorldObject {
            guid: body,
            name: "Corpse of Drudge Slave".into(),
            object_desc_flags: ac_world::object_desc_flags::CORPSE,
            position: Some(ac_world::object::Position::new_flat(
                0xA9B4_0019,
                glam::Vec3::new(84.0, 84.0, 0.0),
            )),
            ..Default::default()
        },
    );
    let at = c.world.objects[&body].world_pos().expect("in the world");
    c.world.objects.insert(
        key,
        ac_world::WorldObject {
            guid: key,
            name: "Broken Marble Key".into(),
            container: Some(body),
            ..Default::default()
        },
    );
    let (picker, mage) = (
        Mate {
            world: at,
            ..picking(2, 300)
        },
        Mate {
            world: at,
            ..picking(3, 5)
        },
    );
    c.autoplay.team.mates = vec![picker.clone(), mage.clone()];
    let shut = c.shut_for(body, &[key], &profile);
    assert_eq!(
        shut,
        ShutFor {
            at,
            done_for: vec![mage.guid],
            left_for: vec![picker.guid],
            ..Default::default()
        }
    );

    // Shut, said on the board, and heard by each.
    c.autoplay.config.team.enabled = true;
    c.autoplay.corpse_shut(body, &Did::Done, None, shut, t0);
    let said = Mate {
        shut: c.autoplay.shuts_to_say(t0),
        ..looter(1, at, None, Duration::ZERO)
    };
    let still_waiting_for = |guid: u32| {
        let mut ap = Autoplay {
            team: view_of(vec![said.clone()]),
            ..Default::default()
        };
        ap.config.enabled = true;
        ap.config.team.enabled = true;
        ap.take_in_shuts(guid, t0, |_| Some(at));
        ap.corpse_waiting(body, t0, Room::PLENTY)
    };
    assert!(
        !still_waiting_for(mage.guid),
        "the mage went to open it for nothing"
    );
    assert!(
        still_waiting_for(picker.guid),
        "the key was not waited on for the lockpicker"
    );

    // In a fellowship with only the mage, the lockpicker is not judged
    // at all: it opens the body or not by its own lights, as before.
    let fellow = |guid| ac_world::Fellow {
        guid,
        ..Default::default()
    };
    c.world.fellowship = Some(ac_world::Fellowship {
        members: vec![fellow(0x5000_0001), fellow(mage.guid)],
        ..Default::default()
    });
    assert_eq!(
        c.shut_for(body, &[key], &profile),
        ShutFor {
            at,
            done_for: vec![mage.guid],
            ..Default::default()
        }
    );
}

#[test]
fn a_body_emptied_by_a_mate_stays_emptied_after_the_mate_goes_quiet() {
    // The row goes from the board once its mate has been quiet for six
    // seconds, and a shut is said for twenty; what it said stays said.
    let t0 = Instant::now();
    let (me, body) = (2, 0x8000_0001);
    let mut ap = Autoplay::default();
    ap.config.enabled = true;
    ap.config.team.enabled = true;
    ap.team.mates = vec![Mate {
        shut: vec![Shut {
            body,
            done_for: vec![me],
            ..Default::default()
        }],
        ..looter(1, glam::Vec3::ZERO, None, Duration::ZERO)
    }];
    assert_eq!(ap.take_in_shuts(me, t0, |_| None).len(), 1);
    ap.team = TeamView::default();
    assert!(!ap.corpse_waiting(body, t0 + SHUT_SAID_FOR * 3, Room::PLENTY));
    // Nor does the mate that said it saying nothing more undo it.
    ap.team.mates = vec![looter(1, glam::Vec3::ZERO, None, Duration::ZERO)];
    assert!(ap.take_in_shuts(me, t0, |_| None).is_empty());
    assert!(!ap.corpse_waiting(body, t0 + SHUT_SAID_FOR * 3, Room::PLENTY));
}

#[test]
fn a_body_set_aside_or_left_for_its_weight_is_never_done_for_anyone_else() {
    // Shut for want of room, or because something would not come off,
    // a body still has on it what somebody wanted.
    use crate::did::Did;
    let t0 = Instant::now();
    let mut ap = Autoplay::default();
    ap.config.team.enabled = true;
    let (stubborn, heavy, emptied) = (0x8000_4001, 0x8000_4002, 0x8000_4003);
    let for_both = || ShutFor {
        done_for: vec![2, 3],
        ..Default::default()
    };
    ap.corpse_shut(
        stubborn,
        &Did::blocked("it would not give something up"),
        None,
        for_both(),
        t0,
    );
    ap.corpse_shut(
        heavy,
        &Did::blocked("too laden to take the rest"),
        Some(300),
        for_both(),
        t0,
    );
    assert!(
        ap.shuts_to_say(t0).is_empty(),
        "said a body with something on it was emptied"
    );
    ap.corpse_shut(emptied, &Did::Done, None, for_both(), t0);
    assert_eq!(
        ap.shuts_to_say(t0),
        vec![Shut {
            body: emptied,
            n: 0,
            done_for: vec![2, 3],
            ..Default::default()
        }]
    );
}

#[test]
fn a_mate_under_its_cap_is_left_the_healing_kits_the_opener_would_not_take() {
    use crate::profile::Verdict;
    let profile = a_party_profile();
    let kits = ItemStats {
        stack: 5,
        ..item("Healing Kit", 50, 0)
    };
    // The opener carries four already: the rule stops there, and
    // nothing else claims them.
    let opener = picking(1, 0);
    assert_eq!(
        judge_loot(&kits, None, Some(&profile), &opener.wielder(), "Bryn01", 4),
        Verdict::None
    );
    // Nobody knows what a mate carries, so the mate is not done with the
    // body, and opens it by its own lights: the rule asks about no
    // skill, so the kits are nobody's in particular.
    let mate = picking(2, 0);
    let sheet = opener.wielder();
    let me = Taker {
        guid: opener.guid,
        name: &opener.name,
        sheet: &sheet,
        held: 4,
    };
    let fellows = [as_fellow(&mate)];
    assert_eq!(
        judge_shut(&[lying(kits, 4)], &profile, me, &fellows, &fellows, None),
        ShutFor::default()
    );
}

#[test]
fn a_mate_that_would_need_an_appraisal_is_not_counted_done() {
    let mut profile = a_party_profile();
    profile
        .rules
        .push(asks("good armour", "type:armor al>=200", LootAction::Sell));
    let mage = picking(3, 0);
    let sheet = crate::weapons::Wielder::default();
    let me = Taker {
        guid: 1,
        name: "Bryn01",
        sheet: &sheet,
        held: 0,
    };
    let done = |lying: &[Option<Left>], profile: &crate::profile::Profile| {
        judge_shut(lying, profile, me, &[as_fellow(&mage)], &[], None).done_for == vec![mage.guid]
    };
    let unread = ItemStats {
        appraised: false,
        ..item("Platemail", 100, 240)
    };
    // Not appraised, it might be good armour: the mate would ask.
    assert!(!done(&[lying(unread.clone(), 0)], &profile));
    // Appraised by the one that shut it, it is judged outright.
    assert!(!done(&[lying(item("Platemail", 100, 240), 0)], &profile));
    assert!(done(&[lying(item("Platemail", 100, 50), 0)], &profile));
    // Where the rules never appraise, the mate would never ask either.
    profile.looting.appraise = false;
    assert!(done(&[lying(unread, 0)], &profile));
    // A thing not described yet cannot be judged, and is waited on.
    assert!(!done(&[None], &profile));
}

#[test]
fn a_mate_outside_the_fellowship_is_never_counted_done_or_left_anything() {
    let at = glam::Vec3::ZERO;
    let view = view_of(vec![
        looter(1, at, None, Duration::ZERO),
        looter(2, at, None, Duration::ZERO),
        // Not in the world yet.
        looter(0, at, None, Duration::ZERO),
    ]);
    let judged = |fellows: Option<&[u32]>| {
        view.judged_at_a_shut(fellows)
            .map(|m| m.guid)
            .collect::<Vec<_>>()
    };
    // Out of a fellowship, everyone in the world.
    assert_eq!(judged(None), vec![1, 2]);
    // In one, its fellows only.
    assert_eq!(judged(Some(&[9, 1])), vec![1]);
}

#[test]
fn what_a_shut_says_is_bounded_and_ages_off() {
    use crate::did::Did;
    let t0 = Instant::now();
    let s = Duration::from_secs;
    let mut ap = Autoplay::default();
    // Off the team there is nobody to say it to.
    ap.corpse_shut(0x8000_0001, &Did::Done, None, ShutFor::default(), t0);
    assert!(ap.shuts_to_say(t0).is_empty());
    ap.config.team.enabled = true;
    // Only the newest are said.
    let first = 0x8000_1000;
    for n in 0..SHUTS_SAID as u32 + 4 {
        let at = t0 + Duration::from_millis(n as u64);
        let shut = ShutFor {
            done_for: vec![2],
            ..Default::default()
        };
        ap.corpse_shut(first + n, &Did::Done, None, shut, at);
    }
    let said = ap.shuts_to_say(t0 + s(1));
    assert_eq!(said.len(), SHUTS_SAID);
    assert_eq!(said.first().map(|shut| shut.body), Some(first + 4));
    // A body shut again is said once, as a new shut.
    let again = ShutFor {
        done_for: vec![2, 3],
        ..Default::default()
    };
    ap.corpse_shut(first + 10, &Did::Done, None, again, t0 + s(2));
    let said = ap.shuts_to_say(t0 + s(2));
    assert_eq!(said.len(), SHUTS_SAID);
    let tenth: Vec<&Shut> = said.iter().filter(|shut| shut.body == first + 10).collect();
    assert_eq!(tenth.len(), 1);
    assert_eq!(tenth[0].n, SHUTS_SAID as u32 + 4);
    // And only for a while.
    assert!(ap.shuts_to_say(t0 + s(2) + SHUT_SAID_FOR).is_empty());
    // Done with here all the same, said or not.
    assert!(ap.looted.contains(&0x8000_0001) && ap.looted.contains(&first));
}

#[test]
fn a_character_alone_loots_exactly_as_it_did() {
    // Nobody on the board: nothing to say, nothing to hear, and every
    // body its own as before.
    use crate::did::Did;
    let t0 = Instant::now();
    let (me, body) = (2, 0x8000_0001);
    let (here, at) = (glam::Vec3::ZERO, glam::Vec3::new(3.0, 0.0, 0.0));
    let mut ap = Autoplay {
        corpse_seen: corpses_seen([(body, t0)]),
        ..Default::default()
    };
    ap.config.enabled = true;
    for team in [false, true] {
        ap.config.team.enabled = team;
        assert!(ap.ours_to_open(body, at, me, here, t0));
        assert!(ap.corpse_owed(body, at, here, t0, Room::PLENTY));
        assert_eq!(ap.team.judged_at_a_shut(None).count(), 0);
        assert!(ap.take_in_shuts(me, t0, |_| None).is_empty());
        assert!(ap.looted.is_empty());
    }
    ap.config.team.enabled = false;
    ap.take_up_corpse(body, t0, LOOT_TIMEOUT);
    ap.corpse_shut(body, &Did::Done, None, ShutFor::default(), t0);
    assert!(ap.looted.contains(&body) && ap.looted.len() == 1);
    assert!(ap.shuts_to_say(t0).is_empty(), "told nobody about it");
    assert!(!ap.corpse_waiting(body, t0, Room::PLENTY));

    // And a character alone finds nobody to judge a body for.
    let c = character_of_level(no_data(), 30);
    assert_eq!(
        c.shut_for(body, &[0x8000_0002], &a_party_profile()),
        ShutFor::default()
    );
}

#[test]
fn a_character_off_the_team_takes_in_no_shuts() {
    let t0 = Instant::now();
    let (me, body) = (2, 0x8000_0001);
    let mut ap = Autoplay::default();
    ap.config.enabled = true;
    assert!(!ap.config.team.enabled);
    ap.team.mates = vec![Mate {
        shut: vec![Shut {
            body,
            done_for: vec![me],
            ..Default::default()
        }],
        ..looter(1, glam::Vec3::ZERO, None, Duration::ZERO)
    }];
    assert!(ap.take_in_shuts(me, t0, |_| None).is_empty());
    assert!(ap.looted.is_empty());
    assert!(ap.corpse_waiting(body, t0, Room::PLENTY));
    // Back on the team, the same word is taken in.
    ap.config.team.enabled = true;
    assert_eq!(
        ap.take_in_shuts(me, t0, |_| None),
        vec![TakenIn::Done {
            body,
            by: "Bryn01".into()
        }]
    );
}

#[test]
fn a_mate_dead_or_missing_from_the_board_is_never_waited_on() {
    let t0 = Instant::now();
    let (body, at) = (0x8000_0001, glam::Vec3::ZERO);
    let alive = looter(2, at, None, Duration::ZERO);
    assert!(alive.could_come_for(at));
    // At another body or fighting, it comes once it is done.
    assert!(Mate {
        looting: Some(0x8000_0002),
        target: Some(0x7000_0001),
        ..alive.clone()
    }
    .could_come_for(at));
    for gone in [
        Mate {
            health: 0.0,
            ..alive.clone()
        },
        Mate {
            autoplay: false,
            ..alive.clone()
        },
        Mate {
            guid: 0,
            ..alive.clone()
        },
        Mate {
            opens_bodies: false,
            ..alive.clone()
        },
    ] {
        assert_eq!(gone.turn(), None);
        assert!(!gone.could_come_for(at));
    }
    // Nor one across the field, or with no room for what is left.
    assert!(!Mate {
        world: glam::Vec3::new(LOOT_NEAR + 5.0, 0.0, 0.0),
        ..alive.clone()
    }
    .could_come_for(at));
    for (pack_full, laden) in [(true, false), (false, true)] {
        let full = Mate {
            supplies: crate::logistics::Supplies {
                pack_full,
                laden,
                ..Default::default()
            },
            ..alive.clone()
        };
        assert!(!full.could_come_for(at));
    }

    // Nor is a body's turn dealt to one dead, or to one gone quiet and
    // off the board: either way the body is this character's.
    let me = 3;
    let mut ap = Autoplay {
        corpse_seen: corpses_seen([(body, t0)]),
        first_opens: vec![t0; 3],
        team: view_of(vec![looter(1, at, None, Duration::ZERO)]),
        ..Default::default()
    };
    assert!(!ap.ours_to_open(body, at, me, at, t0), "had more turns");
    // And standing off it, this character says whose turn it is.
    assert_eq!(ap.whose_turn(body, at, me, at, t0), Some("Bryn01"));
    assert_eq!(ap.whose_turn(body, at, me, at, t0 + CLAIM_SETTLE), None);
    ap.team.mates[0].health = 0.0;
    assert!(
        ap.ours_to_open(body, at, me, at, t0),
        "waited on a dead mate's turn"
    );
    assert_eq!(ap.whose_turn(body, at, me, at, t0), None);
    ap.team.mates.clear();
    assert!(ap.ours_to_open(body, at, me, at, t0));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_salvage_on_an_open_body_is_left_for_the_salvager_standing_by() {
    use crate::logistics::Supplies;
    let holtburg = 0xA9B4_0019;
    let mut c = standing_in_the_field(20, holtburg, glam::Vec3::new(84.0, 84.0, 10.0));
    let me = c.player.as_ref().unwrap().world_position();
    let t0 = Instant::now();
    let (body, plate, gem) = (0x8000_5001, 0x8000_5002, 0x8000_5003);
    c.world.objects.insert(
        body,
        ac_world::WorldObject {
            guid: body,
            name: "Corpse of Drudge Slave".into(),
            object_desc_flags: ac_world::object_desc_flags::CORPSE,
            position: Some(ac_world::object::Position::new_flat(
                holtburg,
                me + glam::Vec3::new(2.0, 0.0, 0.0) - ac_world::landblock_origin(holtburg),
            )),
            ..Default::default()
        },
    );
    for (guid, name) in [(plate, "Platemail"), (gem, "Diamond")] {
        c.world.objects.insert(
            guid,
            ac_world::WorldObject {
                guid,
                name: name.into(),
                container: Some(body),
                ..Default::default()
            },
        );
    }
    let mut profile = crate::profile::Profile {
        name: "party".into(),
        rules: vec![
            asks("platemail to salvage", "platemail", LootAction::Salvage),
            asks("gems", "diamond", LootAction::Sell),
        ],
        ..Default::default()
    };
    // Bryn02 salvages for the party and stands by; Bryn03 does not
    // salvage. This character carries no Ust.
    let salvager = Mate {
        has_ust: true,
        salvaging: 300,
        ..looter(2, me, None, Duration::ZERO)
    };
    let other = looter(3, me, None, Duration::ZERO);
    c.autoplay.team.mates = vec![salvager.clone(), other.clone()];
    let verdicts = |c: &mut Client, profile: &crate::profile::Profile| {
        let open = c.corpse_now(body, &[plate, gem], profile, t0, t0);
        let of = |g| open.items.iter().find(|i| i.guid == g).map(|i| i.verdict);
        (of(plate), of(gem))
    };
    let take = |a| Some(ac_loot::Verdict::Take(a));
    assert_eq!(
        verdicts(&mut c, &profile),
        (Some(ac_loot::Verdict::Leave), take(LootAction::Sell)),
        "took the salvager's salvage, or left the rest"
    );
    // Shut with the platemail on it: left for the salvager. Bryn03 would
    // take it too and stands by for the salvager, and so does this
    // character, rather than either writing the body off.
    let shut = c.shut_for(body, &[plate], &profile);
    assert_eq!(
        (shut.done_for, shut.stand_by, shut.left_for, shut.waits),
        (Vec::<u32>::new(), vec![3], vec![2], true)
    );

    // Never waited on: dead, off the board, with no room, not looting,
    // or having shut this body already. Each time it is taken as ever.
    let salvage = (take(LootAction::Salvage), take(LootAction::Sell));
    c.autoplay.team.mates = vec![
        Mate {
            health: 0.0,
            ..salvager.clone()
        },
        other.clone(),
    ];
    assert_eq!(verdicts(&mut c, &profile), salvage, "dead");
    c.autoplay.team.mates = vec![other.clone()];
    assert_eq!(verdicts(&mut c, &profile), salvage, "off the board");
    c.autoplay.team.mates = vec![
        Mate {
            supplies: Supplies {
                laden: true,
                ..Default::default()
            },
            ..salvager.clone()
        },
        other.clone(),
    ];
    assert_eq!(verdicts(&mut c, &profile), salvage, "laden");
    c.autoplay.team.mates = vec![
        Mate {
            opens_bodies: false,
            ..salvager.clone()
        },
        other.clone(),
    ];
    assert_eq!(verdicts(&mut c, &profile), salvage, "not looting");
    c.autoplay.config.team.enabled = true;
    c.autoplay.team.mates = vec![
        Mate {
            shut: vec![Shut {
                body,
                done_for: vec![3],
                ..Default::default()
            }],
            ..salvager.clone()
        },
        other.clone(),
    ];
    c.autoplay.take_in_shuts(0x5000_0001, t0, |_| None);
    assert_eq!(verdicts(&mut c, &profile), salvage, "shut it already");

    // Rules that mean nothing for anyone in particular, and alone.
    c.autoplay.team.mates = vec![salvager, other];
    c.autoplay.shut_by.clear();
    c.autoplay.done_with.clear();
    profile.looting.salvage = false;
    assert_eq!(verdicts(&mut c, &profile), salvage, "nobody salvages");
    profile.looting.salvage = true;
    c.autoplay.team.mates.clear();
    assert_eq!(verdicts(&mut c, &profile), salvage, "alone");
}
