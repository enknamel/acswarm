use super::*;

#[test]
fn a_scroll_is_learned_within_fifty_of_its_power_and_by_anyone_trained_below_fifty_or_from_300() {
    // ACE CanReadScroll: power - 50 <= skill, but under 50 and from 300 on, any skill.
    assert!(can_learn(250, true, 200));
    assert!(!can_learn(251, true, 200));
    assert!(can_learn(25, true, 0));
    assert!(can_learn(300, true, 10));
    // A school not trained is no use to learn in.
    assert!(!can_learn(25, false, 0));
    assert!(!can_learn(100, false, 400));
}

/// A scroll of `spell` in the pack as `guid`.
fn scroll_in_the_pack(c: &mut Client, guid: u32, spell: u32) {
    crate::testkit::thing_in_the_pack(c, guid, "Scroll", 1);
    let o = c.world.objects.get_mut(&guid).unwrap();
    o.item_type = ac_world::item_type::WRITABLE;
    o.spell_id = spell;
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_scroll_the_character_can_learn_is_kept_and_read_once_nothing_is_fought() {
    // A trained war mage knowing Flame Bolt I: Flame Bolt II's scroll is learnable, a life
    // spell's is not (Life Magic untrained), and nothing is read while a fight is on.
    let now = Instant::now();
    let (mut c, _) = crate::testkit::a_caster_knowing(now, &["Flame Bolt I"]);
    crate::testkit::a_loot_profile(&mut c, "scrolls");
    let table = c.assets.spell_table().unwrap();
    let id_of = |name: &str| {
        let s = table.spells.iter().find(|(_, s)| s.name == name);
        s.map(|(id, _)| *id).unwrap_or_else(|| panic!("{name}"))
    };
    let (bolt, heal) = (id_of("Flame Bolt II"), id_of("Heal Self II"));
    scroll_in_the_pack(&mut c, 0x8000_0500, bolt);
    scroll_in_the_pack(&mut c, 0x8000_0501, heal);

    c.attack_target = Some(0x5000_0001);
    c.autoplay_learn_scrolls(now);
    assert!(!c.autoplay_read_scrolls(now));
    assert_eq!(c.last_used, None, "read mid-fight");
    let me = c.wielder();
    assert!(me.learnable.contains(&bolt) && !me.learnable.contains(&heal));
    let profile = c.loot_profile().unwrap();
    let judged = |guid| {
        let stats = c.stats_of(guid).unwrap();
        super::super::loot::judge::judge_loot(&stats, None, Some(&profile), &me, "Caster", 0)
    };
    assert_eq!(
        judged(0x8000_0500),
        crate::profile::Verdict::Decided(
            crate::autoplay::LootAction::Keep,
            super::super::loot::judge::LEARNABLE.into()
        )
    );
    assert_ne!(
        judged(0x8000_0501),
        crate::profile::Verdict::Decided(
            crate::autoplay::LootAction::Keep,
            super::super::loot::judge::LEARNABLE.into()
        )
    );

    // The fight over, but on the way to a giver: a read would leave the server busy at its use.
    c.attack_target = None;
    let giver = ac_world::landmarks::Landmark {
        kind: ac_world::landmarks::Kind::Npc,
        name: "San Ming".into(),
        cell: 0xA9B4_0019,
        at: glam::Vec3::ZERO,
    };
    c.visits.current = Some(crate::visit::Visit::landmark(&giver));
    c.autoplay_read_scrolls(now);
    assert_eq!(c.last_used, None, "read on the way to a giver");
    c.visits.current = None;

    // A spell in the air: a read sent behind it is turned away busy.
    c.autoplay.cast_sent = Some(now);
    c.autoplay_read_scrolls(now);
    assert_eq!(c.last_used, None, "read behind a cast");
    c.autoplay.cast_sent = None;

    // Between fights in a busy dungeon, walking to the next creature in reach: the read comes
    // first, claiming the tick (Blargerton read three in 48 minutes waiting for a quiet moment).
    c.autoplay.nearest_fight = Some(5.0);
    let me_at = c.my_position().unwrap();
    let ahead = crate::Goal::At {
        place: c.place_at(me_at + glam::Vec3::new(10.0, 0.0, 0.0)),
        within: 1.0,
    };
    c.go(crate::Owner::Script, ahead, now);
    assert!(c.going.is_some(), "no walk set");
    assert!(c.autoplay_read_scrolls(now));
    assert_eq!(
        c.last_used,
        Some(0x8000_0500),
        "the bolt is read, and the heal never"
    );
    c.last_used = None;
    c.going = None;
    // The read lasting, the row holds the tick: the character stands while the server reads.
    assert!(c.autoplay_read_scrolls(now));
    assert_eq!(c.last_used, None);
    // Hit meanwhile, it stands aside for the fight.
    c.autoplay.last_hit_us = Some(Instant::now());
    assert!(!c.autoplay_read_scrolls(now));
    c.autoplay.last_hit_us = None;
    // Blargerton's login buffs went out in the tick he began reading and were turned away (0x1D).
    assert!(c.autoplay.cast_in_flight(now), "a cast waits on the read");
    assert!(!c.autoplay.cast_in_flight(now + READ_TAKES));
    c.autoplay_read_scrolls(now + READ_TAKES);
    assert_eq!(c.last_used, None, "read again while its answer is awaited");

    // Learned, it is not learnable any more; switched off, nothing is read.
    c.world.stats.spells.push(bolt);
    c.autoplay_learn_scrolls(now + SCROLL_RETRY);
    c.autoplay_read_scrolls(now + SCROLL_RETRY);
    assert!(!c.wielder().learnable.contains(&bolt));
    assert_eq!(c.last_used, None);
}

/// The id of the spell called `name`.
fn spell_called(c: &Client, name: &str) -> u32 {
    let table = c.assets.spell_table().unwrap();
    let s = table.spells.iter().find(|(_, s)| s.name == name);
    s.map(|(id, _)| *id).unwrap_or_else(|| panic!("{name}"))
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_second_copy_of_a_scroll_carried_is_not_taken_to_learn() {
    // Blargerton carried two Mind Blossoms and two Drain Stamina Other VIs: one is read, the
    // other has nothing to teach.
    let now = Instant::now();
    let (mut c, _) = crate::testkit::a_caster_knowing(now, &["Flame Bolt I"]);
    crate::testkit::a_loot_profile(&mut c, "scrolls");
    let bolt = spell_called(&c, "Flame Bolt II");
    scroll_in_the_pack(&mut c, 0x8000_0500, bolt);
    c.autoplay_learn_scrolls(now);
    let me = c.wielder();
    let profile = c.loot_profile().unwrap();
    let stats = c.stats_of(0x8000_0500).unwrap();
    let learn = crate::profile::Verdict::Decided(
        crate::autoplay::LootAction::Keep,
        super::super::loot::judge::LEARNABLE.into(),
    );
    let judge = |held| {
        super::super::loot::judge::judge_loot(&stats, None, Some(&profile), &me, "Caster", held)
    };
    assert_eq!(judge(0), learn);
    assert_ne!(judge(1), learn, "a copy carried already");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_scroll_taken_to_learn_goes_to_the_counter_once_it_has_nothing_to_teach() {
    // Tagged to learn when looted; then one copy read and the spell known, or a second copy
    // carried: neither is read, and kept they cluttered Blargerton's pack.
    use super::super::loot::judge::LEARNABLE;
    use crate::autoplay::LootAction;
    let now = Instant::now();
    let (mut c, _) = crate::testkit::a_caster_knowing(now, &["Flame Bolt I"]);
    crate::testkit::a_loot_profile(&mut c, "scrolls");
    let (bolt, frost) = (
        spell_called(&c, "Flame Bolt II"),
        spell_called(&c, "Frost Bolt II"),
    );
    for (guid, spell, wcid) in [
        (0x8000_0500, bolt, 7001),
        (0x8000_0501, frost, 7002),
        (0x8000_0502, frost, 7002),
    ] {
        scroll_in_the_pack(&mut c, guid, spell);
        c.world.objects.get_mut(&guid).unwrap().weenie_class_id = wcid;
        let stats = c.stats_of(guid).unwrap();
        c.autoplay
            .tag_why(&stats, LootAction::Keep, Some(LEARNABLE));
    }
    c.world.stats.spells.push(bolt);
    c.autoplay_learn_scrolls(now);
    let tag = |c: &Client, g| {
        (
            c.autoplay.ledger.of(&c.stats_of(g).unwrap()),
            c.autoplay.ledger.why(g).map(str::to_string),
        )
    };
    let sell = (Some(LootAction::Sell), Some(NOTHING_TO_TEACH.to_string()));
    assert_eq!(tag(&c, 0x8000_0500), sell, "known now");
    assert_eq!(
        tag(&c, 0x8000_0501),
        (Some(LootAction::Keep), Some(LEARNABLE.to_string())),
        "the copy read"
    );
    assert_eq!(tag(&c, 0x8000_0502), sell, "a second copy");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn with_reading_off_only_the_scroll_a_pursued_quest_reads_is_read() {
    // A recall learned for a quest is read though the loot rules read no scroll: that one alone.
    let now = Instant::now();
    let (mut c, _) = crate::testkit::a_caster_knowing(now, &["Flame Bolt I"]);
    crate::testkit::a_loot_profile(&mut c, "scrolls-off");
    let mut p = (*c.loot_profile().unwrap()).clone();
    p.looting.learn_scrolls = false;
    c.profiles.put(p).unwrap();
    let table = c.assets.spell_table().unwrap();
    let id_of = |name: &str| {
        let s = table.spells.iter().find(|(_, s)| s.name == name);
        s.map(|(id, _)| *id).unwrap_or_else(|| panic!("{name}"))
    };
    let (bolt, other) = (id_of("Flame Bolt II"), id_of("Frost Bolt II"));
    scroll_in_the_pack(&mut c, 0x8000_0500, other);
    c.autoplay_learn_scrolls(now);
    c.autoplay_read_scrolls(now);
    assert_eq!(c.last_used, None, "read with reading off");
    scroll_in_the_pack(&mut c, 0x8000_0501, bolt);
    c.pursuit = vec![crate::autoplay::PursuitLine {
        id: "learn:test".into(),
        title: "a recall".into(),
        step: crate::autoplay::PursuitStep::Read {
            wcid: 0,
            name: "Scroll".into(),
            spell: bolt,
        },
        working: true,
        via: None,
    }];
    c.autoplay_learn_scrolls(now);
    c.autoplay_read_scrolls(now);
    assert_eq!(c.last_used, Some(0x8000_0501));
}
