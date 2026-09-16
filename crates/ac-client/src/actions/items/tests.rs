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
