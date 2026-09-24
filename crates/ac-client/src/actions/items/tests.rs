use super::*;
use ac_net::messages::{SalvageResult, SalvageYield};

#[test]
fn salvage_text_lists_materials() {
    let res = SalvageResult {
        skill: 40,
        skipped: vec![1],
        yields: vec![
            SalvageYield {
                material: 0x4B,
                workmanship: 8.0,
                units: 3,
            },
            SalvageYield {
                material: 0x3D,
                workmanship: 5.5,
                units: 1,
            },
        ],
        bonus_percent: 0,
    };
    assert_eq!(
            salvage_text(&res),
            "You obtain 3 Oak (workmanship 8.00), 1 Iron (workmanship 5.50) using your Salvaging skill. 1 item(s) could not be salvaged."
        );
    let mut w = ac_net::wire::Writer::new();
    w.u32(28).u32(0).u32(1).u32(0x40).f64(3.0).u32(2).u32(0);
    let parsed = SalvageResult::parse(&w.finish()).unwrap();
    assert_eq!(parsed.skill, 28);
    assert_eq!(parsed.yields[0].units, 2);
    assert_eq!(
        salvage_text(&parsed),
        "You obtain 2 Steel (workmanship 3.00) using your Weapon Tinkering skill."
    );
}

/// The GameAction numbers queued so far, in order.
fn actions_sent(c: &Client) -> Vec<u32> {
    let word = |b: &[u8], at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
    c.session
        .queued()
        .iter()
        .filter(|(_, b)| b.len() >= 12 && word(b, 0) == 0xF7B1)
        .map(|(_, b)| word(b, 8))
        .collect()
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_use_of_something_in_the_world_reports_the_stop_before_the_use() {
    use ac_net::messages::action::{MOVE_TO_STATE, USE};
    let mut c = crate::testkit::standing_at(0xA9B4_0019, glam::Vec3::new(84.0, 7.1, 94.0));
    // Moving, as far as the server knows: a stop is owed.
    c.held_run = true;
    let npc = crate::testkit::creature(0x8000_1234, "Boddry the Chancy");
    c.world.objects.insert(npc.guid, npc);
    assert!(c.use_object(0x8000_1234));
    let sent = actions_sent(&c);
    let stop = sent.iter().position(|a| *a == MOVE_TO_STATE);
    let used = sent.iter().position(|a| *a == USE).expect("the use goes out");
    assert!(
        stop.is_some_and(|s| s < used),
        "the stop goes out before the use: {sent:x?}"
    );
}
