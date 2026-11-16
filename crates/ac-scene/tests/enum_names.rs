//! The portal DAT's enum names the panels show.

use ac_scene::Assets;

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_slayers_creature_type_is_named_by_the_portal_dat() {
    let assets = Assets::open(ac_dat::test_data_dir()).unwrap();
    let types = assets.creature_types().unwrap();
    // CreatureType 19 is Virindi (ACE CreatureType.cs), 1 Olthoi.
    assert_eq!(types.get(19), Some("Virindi"));
    assert_eq!(types.get(1), Some("Olthoi"));
}
