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
    let set = |c: &mut Client| {
        c.attack_target = Some(1);
        c.autoplay.casting_at = Some(2);
        c.autoplay.engaged = Some((3, Instant::now(), 1.0));
        c.autoplay.closing = Some((3, 1.0));
    };
    let held = |c: &Client| {
        (
            c.attack_target.is_some(),
            c.autoplay.casting_at.is_some(),
            c.autoplay.engaged.is_some(),
            c.autoplay.closing.is_some(),
        )
    };
    let mut c = crate::testkit::offline_client();
    set(&mut c);
    c.let_go(Release::Cast);
    assert_eq!(held(&c), (true, false, true, true), "the spell's target");
    set(&mut c);
    c.let_go(Release::Targets);
    assert_eq!(held(&c), (false, false, true, true), "both targets");
    set(&mut c);
    c.let_go(Release::Fight);
    assert_eq!(held(&c), (false, false, false, false), "the whole fight");
    set(&mut c);
    c.let_go(Release::Engagement);
    assert_eq!(
        held(&c),
        (true, false, false, false),
        "the Academy's: the swing's target stays"
    );
}
