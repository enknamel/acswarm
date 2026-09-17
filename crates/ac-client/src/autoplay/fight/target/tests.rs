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

#[test]
fn a_stop_takes_the_fights_own_spell_back_and_leaves_a_heal_to_land() {
    // A cast already sent goes on to land unless something reaches it.
    // The fight being over, the combat-mode change ACE turns into a
    // failed cast is that something (see `Client::stop_fight_cast`).
    let mut c = crate::testkit::offline_client();
    let sent = Instant::now();
    c.magic = true;
    c.autoplay.cast_sent = Some(sent);
    c.autoplay.fight_cast = Some(sent);
    c.take_up_fight(1);
    c.let_go(Release::Fight);
    assert!(!c.magic, "a spell was left to land after the stop");
    assert_eq!(c.autoplay.fight_cast, None, "nothing left to take back");

    // A heal, a healing kit and a counter set the same clock. A stop
    // that fizzled one of those would take the heal a character lives by.
    c.magic = true;
    c.autoplay.cast_sent = Some(Instant::now());
    c.autoplay.fight_cast = None;
    c.take_up_fight(1);
    c.let_go(Release::Fight);
    assert!(c.magic, "a stop fizzled a heal that was not the fight's");

    // A fight spell the server has already answered for is spent: there
    // is nothing to take back, and no stance to spend on trying.
    c.autoplay.cast_sent = None;
    c.autoplay.fight_cast = Some(Instant::now());
    c.take_up_fight(1);
    c.let_go(Release::Fight);
    assert!(c.magic, "the stance went for a cast already answered");
}
