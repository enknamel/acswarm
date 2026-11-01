use super::*;
use crate::testkit::{creature, standing_in_the_field};
use glam::Vec3;

const LEAP: &str = "fachubhuntersleapportal_flag";

fn leap() -> &'static ac_world::unlocks::Unlock {
    ac_world::unlocks::all()
        .iter()
        .find(|u| u.flag == LEAP)
        .expect("Hunter's Leap's Warden")
}

/// A character of `level` beside Hunter's Leap's Warden, who is in view as `guid` 0x7A02_0001.
pub(crate) fn beside_the_warden(level: i32) -> Client {
    let w = leap();
    let o = ac_world::landblock_origin(w.cell);
    let mut c = standing_in_the_field(level, w.cell, w.at - o + Vec3::new(1.0, 0.0, 0.0));
    let mut warden = creature(0x7A02_0001, &w.npc);
    warden.position = Some(ac_world::object::Position::new_flat(w.cell, w.at - o));
    c.world.objects.insert(warden.guid, warden);
    c
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn talking_to_a_warden_at_the_level_earns_the_flag() {
    let mut low = beside_the_warden(15);
    low.note_unlock_talk(0x7A02_0001);
    assert!(!low.holds_quest_flag(LEAP), "a level 15 is not flagged");
    let mut c = beside_the_warden(20);
    c.note_unlock_talk(0x7A02_0001);
    assert!(c.holds_quest_flag(LEAP));
    // Talked to again, it is held once.
    c.note_unlock_talk(0x7A02_0001);
    assert_eq!(c.quest_flags().len(), 1);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_flag_held_opens_its_portal_and_a_refusal_closes_it() {
    // From the Facility Hub to where Hunter's Leap comes out: the portal is the way once held.
    let p = ac_world::portals::all()
        .iter()
        .find(|p| p.quest.eq_ignore_ascii_case(LEAP))
        .expect("the Hunter's Leap portal");
    let goal = p.to.truncate();
    let through_leap = |c: &Client| {
        c.travel_trip().is_some_and(|t| {
            t.steps.iter().any(|s| {
                matches!(s, ac_world::trip::Step::Portal { mouth, .. }
                    if mouth.distance(p.from_xy()) < 1.0)
            })
        })
    };
    let mut c = beside_the_warden(20);
    c.travel_to_in(goal, p.to_cell);
    assert!(!through_leap(&c), "planned through a portal not open to it");
    c.cancel_travel();
    c.note_unlock_talk(0x7A02_0001);
    assert!(c.travel_to_in(goal, p.to_cell));
    assert!(
        through_leap(&c),
        "{:?}",
        c.travel_trip().map(|t| t.summary())
    );
    assert_eq!(
        c.travel_portal()
            .map(|(_, m)| m.distance(p.from_xy()) < 1.0),
        Some(true),
        "Hunter's Leap is not the first portal"
    );
    // The portal turns it away for the quest: the flag goes, and is not planned through again.
    c.portal_quest_error(NO_QUEST_FOR_PORTAL);
    assert!(!c.holds_quest_flag(LEAP));
}
