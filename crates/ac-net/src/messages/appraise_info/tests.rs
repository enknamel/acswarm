use super::*;
use crate::wire::Writer;

#[test]
fn parses_profiles() {
    let mut w = Writer::new();
    w.u32(0x8000_0001);
    w.u32(
        Appraisal::FLAG_INT
            | Appraisal::FLAG_STRING
            | Appraisal::FLAG_SPELL_BOOK
            | Appraisal::FLAG_WEAPON_PROFILE
            | Appraisal::FLAG_ARMOR_PROFILE
            | Appraisal::FLAG_CREATURE_PROFILE
            | Appraisal::FLAG_ARMOR_LEVELS,
    );
    w.u32(1);
    w.u16(2).u16(16).u32(19).i32(150).u32(44).i32(12);
    w.u16(1).u16(8).u32(15).string16("A dagger.");
    w.u32(2).u32(2091).u32(1);
    // Armor profile (8 f32), then creature, then weapon.
    for v in [1.0f32, 1.2, 0.8, 0.5, 0.5, 0.5, 0.0, 0.5] {
        w.f32(v);
    }
    w.u32(0x9).u32(40).u32(50);
    for v in [10u32, 20, 30, 40, 50, 60, 70, 80, 90, 100] {
        w.u32(v);
    }
    w.u16(1).u16(2);
    w.u32(2)
        .u32(20)
        .u32(1)
        .u32(12)
        .f64(0.5)
        .f64(1.0)
        .f64(0.3)
        .f64(0.0)
        .f64(1.05)
        .u32(0);
    for v in 1..=9u32 {
        w.u32(v * 10);
    }
    let a = Appraisal::parse(&w.finish()).unwrap();
    assert!(a.success);
    assert_eq!(a.int(19), Some(150));
    assert_eq!(a.string(15), Some("A dagger."));
    assert_eq!(a.spells, vec![2091, 1]);
    assert_eq!(a.armor.as_ref().map(|p| p.pierce), Some(1.2));
    let c = a.creature.as_ref().unwrap();
    assert_eq!((c.health, c.health_max), (40, 50));
    assert_eq!(c.attributes, Some([10, 20, 30, 40, 50, 60]));
    assert_eq!((c.stamina, c.mana_max), (70, 100));
    assert_eq!(c.attribute_marks, Some((1, 2)));
    let wp = a.weapon.as_ref().unwrap();
    assert_eq!(
        (wp.damage_type, wp.speed, wp.skill, wp.damage),
        (2, 20, 1, 12)
    );
    assert!((wp.offense - 1.05).abs() < 1e-9);
    assert_eq!(a.armor_levels.map(|l| l[8]), Some(90));
}

#[test]
fn appraisal_layout() {
    let mut w = Writer::new();
    w.u32(0x8000_0001).u32(0x0001 | 0x0008 | 0x2000).u32(1);
    w.u16(1).u16(64).u32(25).i32(3);
    w.u16(1).u16(64).u32(1).u64(99);
    w.u16(2)
        .u16(16)
        .u32(15)
        .string16("A door.")
        .u32(16)
        .string16("It is shut.");
    let a = Appraisal::parse(&w.finish()).unwrap();
    assert!(a.success);
    assert_eq!(a.ints, vec![(25, 3)]);
    assert_eq!(a.int64s, vec![(1, 99)]);
    assert_eq!(a.string(Appraisal::STRING_SHORT_DESC), Some("A door."));
    assert_eq!(a.string(Appraisal::STRING_LONG_DESC), Some("It is shut."));
    assert_eq!(a.string(Appraisal::STRING_USE), None);
}
