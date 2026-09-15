use super::*;
use crate::messages::parse_view_contents;

#[test]
fn combat_layouts() {
    let mut w = Writer::new();
    w.string16("Golem").u32(4).f64(0.25).u32(7).u32(1).u64(0);
    let a = AttackNotice::parse_attacker(&w.finish()).unwrap();
    assert_eq!((a.name.as_str(), a.damage, a.critical), ("Golem", 7, true));
    let mut w = Writer::new();
    w.string16("Golem")
        .u32(4)
        .f64(0.1)
        .u32(2)
        .u32(3)
        .u32(0)
        .u64(0);
    let d = AttackNotice::parse_defender(&w.finish()).unwrap();
    assert_eq!((d.damage, d.critical), (2, false));
    let mut w = Writer::new();
    w.u32(0x8000_0001).f32(0.5);
    assert_eq!(
        parse_update_health(&w.finish()).unwrap(),
        (0x8000_0001, 0.5)
    );
    let mut w = Writer::new();
    w.u32(0x9000_0001)
        .u32(2)
        .u32(0x8000_0002)
        .u32(0)
        .u32(0x8000_0003)
        .u32(1);
    let (c, items) = parse_view_contents(&w.finish()).unwrap();
    assert_eq!(c, 0x9000_0001);
    assert_eq!(items, vec![(0x8000_0002, 0), (0x8000_0003, 1)]);
}
