use super::*;

#[test]
fn target_rules_read_names() {
    let mut f = Fight::default();
    assert!(wanted_target("Drudge Skulker", &f));
    f.only = vec!["drudge".into()];
    assert!(wanted_target("Drudge Skulker", &f));
    assert!(!wanted_target("Olthoi Grub", &f));
    f.only = vec!["  ".into()];
    assert!(wanted_target("anything", &f), "a blank rule means any");
    f.only = Vec::new();
    f.avoid = vec!["Olthoi".into()];
    assert!(!wanted_target("Olthoi Grub", &f));
    assert!(wanted_target("Drudge Skulker", &f));
    // Avoid wins over only.
    f.only = vec!["olthoi".into()];
    assert!(!wanted_target("Olthoi Grub", &f));
}

#[test]
fn each_release_lets_go_of_exactly_its_own_fields() {
    // The four shapes the call sites had before they shared one fn.
    let mut c = crate::testkit::offline_client();
    c.take_up_fight(1);
    c.let_go(Release::Cast);
    assert_eq!(
        c.fight_held(),
        (true, false, true, true),
        "the spell's target"
    );
    c.take_up_fight(1);
    c.let_go(Release::Targets);
    assert_eq!(c.fight_held(), (false, false, true, true), "both targets");
    c.take_up_fight(1);
    c.let_go(Release::Fight);
    assert_eq!(
        c.fight_held(),
        (false, false, false, false),
        "the whole fight"
    );
    c.take_up_fight(1);
    c.let_go(Release::Engagement);
    assert_eq!(
        c.fight_held(),
        (true, false, false, false),
        "the Academy's: the swing's target stays"
    );
}
