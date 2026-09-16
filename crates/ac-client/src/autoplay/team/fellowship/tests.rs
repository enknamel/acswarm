use super::{
    next_invitee, rival_leader, HELD_OFF_FIRST, RECRUIT_AGAIN, RECRUIT_FLOOR, TEAM_OPTIONS,
    YIELD_AFTER,
};
use crate::autoplay::{Mate, TeamView};
use crate::did::Patience;
use crate::testkit::standing_in_the_field;
use crate::Client;
use std::time::{Duration, Instant};

/// Nobody held off.
fn nobody() -> Patience<u32> {
    Patience::new()
}

#[test]
fn a_recruit_answered_already_a_member_is_not_asked_again_within_the_hold() {
    // Two mates were "brought into the fellowship" ten times each and
    // never came: the server's answer was chat the client never
    // read, and `next_invitee` asked again every RECRUIT_AGAIN. Read,
    // the answer holds that mate off for a doubling wait while the
    // others are still asked.
    let start = Instant::now();
    let (lyn, oth) = (0x1001, 0x1002);
    let waiting = [(lyn, 3.0), (oth, 5.0)];
    let mut held = nobody();
    let mut asked = vec![(lyn, start)];
    held.hold(lyn, HELD_OFF_FIRST, start);
    // The other is asked meanwhile.
    assert_eq!(
        next_invitee(&waiting, &asked, &held, Some(start), start + RECRUIT_FLOOR),
        Some(oth)
    );
    asked.push((oth, start + RECRUIT_FLOOR));
    // RECRUIT_AGAIN later the held one would have been asked again;
    // now it waits out its hold.
    let again = start + RECRUIT_AGAIN;
    assert_eq!(
        next_invitee(&waiting, &asked, &nobody(), Some(start), again),
        Some(lyn)
    );
    assert_eq!(
        next_invitee(&waiting, &asked, &held, Some(start), again),
        None
    );
    let free = start + HELD_OFF_FIRST;
    assert_eq!(
        next_invitee(&waiting, &asked, &held, Some(start), free),
        Some(lyn)
    );
    // Refused again: twice the wait. (The other, long since asked,
    // is asked again meanwhile, so only this one is on the list.)
    let only_lyn = [(lyn, 3.0)];
    held.hold(lyn, HELD_OFF_FIRST, free);
    assert_eq!(
        next_invitee(&only_lyn, &asked, &held, Some(start), free + HELD_OFF_FIRST),
        None
    );
    assert_eq!(
        next_invitee(
            &only_lyn,
            &asked,
            &held,
            Some(start),
            free + HELD_OFF_FIRST * 2
        ),
        Some(lyn)
    );
}

#[test]
fn a_refusal_in_words_holds_off_the_mate_it_names_and_only_one_that_was_asked() {
    let t0 = Instant::now();
    let (lyn, oth) = (0x1001, 0x1002);
    let mut ap = super::Autoplay::default();
    ap.team.mates = vec![
        Mate {
            name: "+Brynlyn".into(),
            guid: lyn,
            ..Default::default()
        },
        Mate {
            name: "+Brynoth".into(),
            guid: oth,
            ..Default::default()
        },
    ];
    ap.recruited = vec![(lyn, t0)];
    ap.hear_recruit_words("+Brynlyn is busy.", t0);
    assert!(ap.held_off.held(&lyn, t0 + RECRUIT_AGAIN));
    assert!(!ap.held_off.held(&lyn, t0 + HELD_OFF_FIRST));
    // The same words about a mate never invited are about something
    // else -- a patron busy with an oath says "is busy." too.
    ap.hear_recruit_words("+Brynoth is busy.", t0);
    assert!(!ap.held_off.held(&oth, t0));
    // And a stranger's name is nobody's.
    ap.hear_recruit_words("Ulgrim is already a member of a Fellowship.", t0);
    assert_eq!(ap.held_off.len(), 1);
}

/// An autoplay with two mates on its roster, and the first of them
/// just invited.
fn having_asked_lyn(t0: Instant) -> (super::Autoplay, u32, u32) {
    let (lyn, oth) = (0x1001, 0x1002);
    let mut ap = super::Autoplay::default();
    ap.team.mates = vec![
        Mate {
            name: "+Brynlyn".into(),
            guid: lyn,
            ..Default::default()
        },
        Mate {
            name: "+Brynoth".into(),
            guid: oth,
            ..Default::default()
        },
    ];
    ap.recruited = vec![(lyn, t0)];
    (ap, lyn, oth)
}

#[test]
fn a_busy_mate_s_hold_does_not_outgrow_ten_seconds() {
    // The server's `fellow_busy_no_recruit` answers "is busy." for
    // a mate mid-use or mid-cast, which is over in seconds. Doubled
    // each time, a mage that happened to be casting at each of
    // eight asks was left out for hours; it is the same ten seconds
    // every time. "Already a member" is slower to change, and
    // doubles.
    let t0 = Instant::now();
    let (mut ap, lyn, _) = having_asked_lyn(t0);
    let mut now = t0;
    for _ in 0..8 {
        ap.recruited = vec![(lyn, now)];
        ap.hear_recruit_words("+Brynlyn is busy.", now);
        assert_eq!(ap.held_off.waited(&lyn), Some(HELD_OFF_FIRST));
        now += HELD_OFF_FIRST * 2;
    }
    ap.recruited = vec![(lyn, now)];
    ap.hear_recruit_words("+Brynlyn is already a member of a Fellowship.", now);
    assert_eq!(ap.held_off.waited(&lyn), Some(HELD_OFF_FIRST * 2));
}

#[test]
fn a_refusal_long_after_the_invitation_is_about_something_else() {
    // The list of the invited is pruned only on the next invitation.
    // "+Brynlyn is busy." ten minutes after the last invitation is
    // a patron who could not take an oath (ACE
    // `Player_Allegiance.cs`), not an answer to it.
    let t0 = Instant::now();
    let (mut ap, lyn, _) = having_asked_lyn(t0);
    let later = t0 + RECRUIT_AGAIN * 2;
    ap.hear_recruit_words("+Brynlyn is busy.", later);
    assert!(!ap.held_off.held(&lyn, later));
    assert_eq!(ap.held_off.len(), 0);
    // Within the wait for an answer, it is the answer.
    let soon = t0 + RECRUIT_AGAIN / 2;
    ap.hear_recruit_words("+Brynlyn is busy.", soon);
    assert!(ap.held_off.held(&lyn, soon));
}

#[test]
fn a_second_fellowship_is_given_up_by_the_leader_whose_name_does_not_sort_first() {
    // Two fellowships for one fleet: the one founded by the leader
    // that does not sort first goes, so the rightful leader can
    // recruit its members. The rival is read off two words that
    // have to agree -- the board's row says it is in a fellowship,
    // the world's record says not in this one.
    let me = 0x5000_0009;
    let rival = Mate {
        name: "+Brynith".into(),
        guid: 0x5000_0001,
        in_fellowship: true,
        leader: true,
        autoplay: true,
        ..Default::default()
    };
    let mine = [me, 0x5000_0005];
    let view = |m: Mate, settled: bool| TeamView {
        mates: vec![m],
        leader: false,
        settled,
        ..Default::default()
    };
    assert_eq!(
        rival_leader(&view(rival.clone(), true), &mine).map(|m| m.guid),
        Some(rival.guid)
    );
    // Not on a roster that has yet to settle: the rightful leader
    // may still be a frame away from being heard.
    assert!(rival_leader(&view(rival.clone(), false), &mine).is_none());
    // Nor while this character leads the team itself.
    let mut led = view(rival.clone(), true);
    led.leader = true;
    assert!(rival_leader(&led, &mine).is_none());
    // A rightful leader that let itself be recruited into this one
    // is a working party, not a rival.
    assert!(rival_leader(&view(rival.clone(), true), &[me, rival.guid]).is_none());
    // One in no fellowship at all is recruited, not yielded to.
    let free = Mate {
        in_fellowship: false,
        ..rival.clone()
    };
    assert!(rival_leader(&view(free, true), &mine).is_none());
    // A person standing in a fellowship of their own, with autoplay
    // off and not asking to lead, would recruit nobody: this one is
    // kept rather than given up to them.
    let person = Mate {
        autoplay: false,
        leads: false,
        ..rival.clone()
    };
    assert!(rival_leader(&view(person.clone(), true), &mine).is_none());
    let leads = Mate {
        leads: true,
        ..person
    };
    assert!(rival_leader(&view(leads, true), &mine).is_some());
}

#[test]
fn every_option_a_teammate_keeps_on_is_one_the_table_knows() {
    // The options are named in words and looked up by those words,
    // so a name that no longer matches the table is an option that
    // is silently never set -- which is how nine characters hunted
    // for ten minutes without loot sharing.
    for name in TEAM_OPTIONS {
        assert!(
            crate::options::option_by_name(name).is_some(),
            "no option called {name:?}"
        );
    }
}

#[test]
fn loot_sharing_is_the_option_ace_reads_off_the_founder() {
    // ACE takes `CharacterOption.ShareFellowshipLoot` (0x11, bit
    // 0x0010_0000 of the first word) off the leader in the
    // `Fellowship` constructor, and `Corpse.HasPermission` lets a
    // fellow open a fresh body through that clause alone.
    let o = crate::options::option_by_name("share fellowship loot").expect("no such option");
    assert_eq!(o.id, 0x11);
    assert_eq!(o.bit, 0x0010_0000);
    assert!(!o.inverted, "the bit means share, not ignore");
}

#[test]
fn the_others_are_asked_while_one_invitee_waits() {
    // The bug: one timer for the whole party, so nine characters
    // took forty-one seconds to join, at five seconds apart. Each
    // tick now asks somebody who has not been asked yet.
    let start = Instant::now();
    let waiting = [(0x1001, 3.0), (0x1002, 5.0), (0x1003, 9.0)];
    let first = next_invitee(&waiting, &[], &nobody(), None, start).expect("nobody asked");
    assert_eq!(first, 0x1001, "the nearest is asked first");
    let asked = [(first, start)];
    let then = start + RECRUIT_FLOOR;
    assert_eq!(
        next_invitee(&waiting, &asked, &nobody(), Some(start), then),
        Some(0x1002)
    );
}

#[test]
fn two_invitations_do_not_leave_in_the_same_breath() {
    // No rate limit on recruiting was found in ACE, but that is
    // read from the source and untested against a live server, so
    // the invitations keep a small gap.
    let start = Instant::now();
    let waiting = [(0x1001, 3.0), (0x1002, 5.0)];
    assert_eq!(
        next_invitee(&waiting, &[], &nobody(), Some(start), start),
        None
    );
    let soon = start + RECRUIT_FLOOR / 2;
    assert_eq!(
        next_invitee(&waiting, &[], &nobody(), Some(start), soon),
        None
    );
    assert!(next_invitee(&waiting, &[], &nobody(), Some(start), start + RECRUIT_FLOOR).is_some());
}

#[test]
fn an_invitee_that_never_came_is_asked_again_later() {
    // A member the server thought busy is refused with nothing the
    // client can act on, so the only way to tell "not yet" from
    // "never" is to ask again once the others have been asked.
    let start = Instant::now();
    let waiting = [(0x1001, 3.0)];
    let asked = [(0x1001, start)];
    let soon = start + RECRUIT_AGAIN / 2;
    assert_eq!(
        next_invitee(&waiting, &asked, &nobody(), Some(start), soon),
        None
    );
    let later = start + RECRUIT_AGAIN;
    assert_eq!(
        next_invitee(&waiting, &asked, &nobody(), Some(start), later),
        Some(0x1001)
    );
}

#[test]
fn nine_are_all_asked_inside_ten_seconds() {
    // What the party is judged on: everyone in the fellowship soon
    // after they arrive, rather than the last one joining after the
    // first two minutes of every body's life have gone by.
    let start = Instant::now();
    let waiting: Vec<(u32, f32)> = (0..9).map(|i| (0x1000 + i, i as f32)).collect();
    let mut asked: Vec<(u32, Instant)> = Vec::new();
    let mut last = None;
    let mut now = start;
    while asked.len() < waiting.len() {
        match next_invitee(&waiting, &asked, &nobody(), last, now) {
            Some(guid) => {
                asked.push((guid, now));
                last = Some(now);
            }
            None => now += RECRUIT_FLOOR / 4,
        }
        assert!(
            now.duration_since(start) < std::time::Duration::from_secs(10),
            "only {} of nine asked in ten seconds",
            asked.len()
        );
    }
}
#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_leader_that_does_not_sort_first_gives_up_the_fellowship_it_founded() {
    // Two fellowships for one fleet, and this character founded the
    // one whose leader does not sort first. Once the rightful leader
    // has been seen in a fellowship of its own for a few seconds,
    // this one is disbanded, so the rightful leader can recruit its
    // members and this character with them.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_in_the_field(20, holtburg, glam::Vec3::new(84.0, 84.0, 10.0));
    let t0 = Instant::now();
    let me = c.world.player_guid.unwrap();
    let member = 0x5000_0005;
    c.autoplay.config.team.enabled = true;
    c.autoplay.config.team.fellowship = true;
    let fellow = |guid| ac_world::Fellow {
        guid,
        ..Default::default()
    };
    c.world.fellowship = Some(ac_world::Fellowship {
        name: "acreborn".into(),
        leader: me,
        members: vec![fellow(me), fellow(member)],
        ..Default::default()
    });
    c.autoplay.founded = Some(t0);
    c.autoplay.team = TeamView {
        mates: vec![
            Mate {
                name: "+Brynith".into(),
                guid: 0x5000_0002,
                in_fellowship: true,
                leader: true,
                autoplay: true,
                ..Default::default()
            },
            Mate {
                name: "+Brynwyn".into(),
                guid: member,
                in_fellowship: true,
                ..Default::default()
            },
        ],
        leader: false,
        settled: true,
        ..Default::default()
    };
    // The first sight starts the clock; the board's word on a mate
    // is up to a round old, so it is given a few rounds to agree
    // with the world's.
    assert!(!c.autoplay_fellowship(t0));
    assert!(!c.autoplay_fellowship(t0 + YIELD_AFTER / 2));
    assert!(c.autoplay_fellowship(t0 + YIELD_AFTER), "not given up");
    assert!(c.autoplay.founded.is_none());
    assert!(
        c.autoplay.status.contains("disbanding"),
        "{}",
        c.autoplay.status
    );
    // One this character did not found is never given up, whoever
    // leads: the note on a fellowship "not founded by me" stands.
    c.autoplay.founded = None;
    c.autoplay.status.clear();
    assert!(!c.autoplay_fellowship(t0 + YIELD_AFTER * 4));
    assert!(!c.autoplay_fellowship(t0 + YIELD_AFTER * 8));
    assert!(!c.autoplay.status.contains("disbanding"));
    // Nor is one this character founded while it leads the team.
    c.autoplay.founded = Some(t0);
    c.autoplay.team.leader = true;
    assert!(!c.autoplay_fellowship(t0 + YIELD_AFTER * 12));
    assert!(c.autoplay.founded.is_some());
}

/// A character leading a fellowship it founded, with `others` in
/// it besides itself, on a settled team.
fn leading_a_fellowship(c: &mut Client, others: &[u32], t0: Instant) {
    let me = c.world.player_guid.unwrap();
    c.autoplay.config.team.enabled = true;
    c.autoplay.config.team.fellowship = true;
    let fellow = |guid| ac_world::Fellow {
        guid,
        ..Default::default()
    };
    c.world.fellowship = Some(ac_world::Fellowship {
        name: "acreborn".into(),
        leader: me,
        members: std::iter::once(me)
            .chain(others.iter().copied())
            .map(fellow)
            .collect(),
        ..Default::default()
    });
    c.autoplay.founded = Some(t0);
    c.autoplay.team.settled = true;
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn whoever_leads_the_fellowship_brings_in_the_rightful_leader_standing_outside() {
    // +Brynlyn founded and gathered the seven that came on with it;
    // +Brynith, first by name, came on a few seconds later. Every
    // roster's leader flipped to +Brynith the moment it was heard:
    // +Brynlyn stopped recruiting, since it led the team no longer,
    // and +Brynith, with nobody free to found with, founded
    // nothing. Seven in a fellowship and the team's leader outside
    // it for the life of the run. Whoever leads a fellowship brings
    // the team's mates into it, the rightful leader among them.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_in_the_field(20, holtburg, glam::Vec3::new(84.0, 84.0, 10.0));
    let t0 = Instant::now();
    let here = c.player.as_ref().unwrap().world_position();
    let (rightful, member) = (0x5000_0002, 0x5000_0005);
    leading_a_fellowship(&mut c, &[member], t0);
    c.autoplay.team.leader = false;
    c.autoplay.team.mates = vec![
        Mate {
            name: "+Brynith".into(),
            guid: rightful,
            in_fellowship: false,
            leader: true,
            autoplay: true,
            world: here,
            ..Default::default()
        },
        Mate {
            name: "+Brynwyn".into(),
            guid: member,
            in_fellowship: true,
            world: here,
            ..Default::default()
        },
    ];
    assert!(c.autoplay_fellowship(t0), "{}", c.autoplay.status);
    assert!(
        c.autoplay
            .status
            .contains("bringing +Brynith into the fellowship"),
        "{}",
        c.autoplay.status
    );
    assert_eq!(
        c.autoplay
            .recruited
            .iter()
            .map(|(g, _)| *g)
            .collect::<Vec<_>>(),
        vec![rightful]
    );
    // The other way about, nothing: a team leader that let itself
    // be recruited into a mate's fellowship cannot recruit into it
    // (the server answers 0x041D), and leaves the gathering to that
    // mate.
    c.autoplay.recruited.clear();
    c.autoplay.last_recruit = None;
    c.world.fellowship.as_mut().unwrap().leader = member;
    c.autoplay.founded = None;
    c.autoplay.team.leader = true;
    c.autoplay.team.mates[0].leader = false;
    assert!(!c.autoplay_fellowship(t0 + RECRUIT_AGAIN));
    assert!(c.autoplay.recruited.is_empty());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_full_fellowship_asks_nobody_else() {
    // Nine in, a tenth on the team: the server answered 0x041E to
    // every ask and the tenth was asked every five seconds for the
    // life of the run. The count says so first, once; and the code,
    // should the server's count differ, holds off whoever was asked
    // last.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_in_the_field(20, holtburg, glam::Vec3::new(84.0, 84.0, 10.0));
    let t0 = Instant::now();
    let here = c.player.as_ref().unwrap().world_position();
    let eight: Vec<u32> = (0..8).map(|i| 0x5000_0010 + i).collect();
    leading_a_fellowship(&mut c, &eight, t0);
    c.autoplay.team.leader = true;
    let tenth = 0x5000_0030;
    c.autoplay.team.mates = eight
        .iter()
        .map(|&guid| Mate {
            name: format!("+Bryn{guid:x}"),
            guid,
            in_fellowship: true,
            world: here,
            ..Default::default()
        })
        .chain(std::iter::once(Mate {
            name: "+Brynzed".into(),
            guid: tenth,
            world: here,
            ..Default::default()
        }))
        .collect();
    // A hold on somebody no longer on the team goes with them.
    let gone = 0x5000_0099;
    c.autoplay.held_off.hold(gone, HELD_OFF_FIRST, t0);
    assert!(!c.autoplay_fellowship(t0));
    assert!(c.autoplay.recruited.is_empty(), "the tenth was asked");
    assert!(
        c.autoplay
            .noted
            .iter()
            .any(|(t, _)| t.contains("the fellowship is full at 9: 1 mate(s)")),
        "{:?}",
        c.autoplay.noted
    );
    assert!(!c.autoplay.held_off.held(&gone, t0));
    // The server's own word on it, about the last one asked.
    c.autoplay.recruited = vec![(tenth, t0)];
    let soon = t0 + Duration::from_secs(1);
    c.autoplay.hear_fellowship_full(soon);
    assert!(c.autoplay.held_off.held(&tenth, soon));
    assert!(!c.autoplay.held_off.held(&tenth, soon + HELD_OFF_FIRST));
}
