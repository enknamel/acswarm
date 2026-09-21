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
    c.let_go(Release::Stop);
    assert_eq!(
        c.fight_held(),
        (false, false, false, false),
        "a stop: the whole fight, and the spell in the air with it"
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
fn a_stop_takes_the_fights_own_spell_back_and_stays_in_magic_mode() {
    // A cast already sent goes on to land unless something reaches it.
    // The character having been told to stop fighting, the combat-mode
    // change ACE turns into a failed cast is that something -- and the
    // mode sent is the one already held (`Client::stop_fight_cast`).
    let mut c = crate::testkit::offline_client();
    let sent = Instant::now();
    c.magic = true;
    c.autoplay.cast_sent = Some(sent);
    c.autoplay.fight_cast = Some((1, sent));
    c.autoplay.thrown = Some((1, sent));
    c.dodge.fired = Some((4483, sent, Vec::new()));
    c.take_up_fight(1);
    let before = c.session.actions_sent();
    c.let_go(Release::Stop);
    assert_eq!(
        c.session.actions_sent(),
        before + 1,
        "the combat mode went out"
    );
    assert!(
        c.magic,
        "and it is the one already held: no stance is spent"
    );
    assert_eq!(c.autoplay.fight_cast, None, "nothing left to take back");
    assert!(
        c.dodge.fired.is_none(),
        "a fizzled cast throws no projectile to read a speed off"
    );
    assert!(
        c.autoplay.thrown.is_none(),
        "and no shot of ours is on its way to that target"
    );
}

#[test]
fn a_re_target_leaves_the_spell_in_the_air_to_land() {
    // Taking the cast back costs the spell, so only a stop spends it.
    // A rule that goes on fighting -- a re-target, a creature given up
    // on -- is better off letting the spell land.
    let mut c = crate::testkit::offline_client();
    let sent = Instant::now();
    c.magic = true;
    c.autoplay.cast_sent = Some(sent);
    c.autoplay.fight_cast = Some((1, sent));
    c.take_up_fight(1);
    let before = c.session.actions_sent();
    c.let_go(Release::Fight);
    assert_eq!(c.session.actions_sent(), before, "nothing was sent");
    assert_eq!(
        c.autoplay.fight_cast,
        Some((1, sent)),
        "and the cast is still the fight's"
    );
    assert!(c.magic, "nor was the stance dropped");
}

#[test]
fn a_stop_leaves_a_heal_and_a_cast_already_answered_for_alone() {
    // A heal, a healing kit and a counter set the same clock. A stop
    // that fizzled one of those would take the heal a character lives by.
    let mut c = crate::testkit::offline_client();
    c.magic = true;
    c.autoplay.cast_sent = Some(Instant::now());
    c.autoplay.fight_cast = None;
    c.take_up_fight(1);
    let before = c.session.actions_sent();
    c.let_go(Release::Stop);
    assert_eq!(
        c.session.actions_sent(),
        before,
        "the heal was left to land"
    );

    // And a fight cast the server has answered for is spent: there is
    // nothing to take back and no message worth spending on it.
    c.autoplay.cast_sent = None;
    c.autoplay.fight_cast = Some((1, Instant::now()));
    c.take_up_fight(1);
    let before = c.session.actions_sent();
    c.let_go(Release::Stop);
    assert_eq!(c.session.actions_sent(), before, "the cast was spent");
    assert_eq!(c.autoplay.fight_cast, None);
}

#[test]
fn a_fight_cast_the_client_declined_is_not_the_heals_to_take_back() {
    // Without a wand `try_cast` sends nothing, so `cast_sent` keeps
    // whatever set it -- a heal's instant, here -- and the fight has no
    // cast of its own to take back.
    let mut c = crate::testkit::offline_client();
    const SPELL: u32 = 4483;
    c.world.stats.spells.push(SPELL);
    let heal = Instant::now();
    c.magic = true;
    c.autoplay.cast_sent = Some(heal);
    let before = c.session.actions_sent();
    assert!(
        !c.cast_fight_spell(SPELL, 1, Instant::now()),
        "no caster wielded, no cast"
    );
    assert_eq!(c.session.actions_sent(), before, "and nothing on the wire");
    assert_eq!(c.autoplay.fight_cast, None, "the heal's cast is not ours");
    assert_eq!(
        c.autoplay.cast_sent,
        Some(heal),
        "and its wait is untouched"
    );
    c.let_go(Release::Stop);
    assert_eq!(
        c.session.actions_sent(),
        before,
        "so the stop left it alone"
    );
}

#[test]
fn the_answer_that_frees_the_slot_ends_the_fights_own_cast_too() {
    let mut c = crate::testkit::offline_client();
    let sent = Instant::now();
    c.autoplay.cast_sent = Some(sent);
    c.autoplay.fight_cast = Some((1, sent));
    c.autoplay.cast_answered();
    assert_eq!(c.autoplay.cast_sent, None);
    assert_eq!(c.autoplay.fight_cast, None, "a spent cast cannot linger");
}
