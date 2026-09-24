use super::*;

#[test]
fn an_urgent_buff_waits_for_the_fight_only_when_it_costs_the_weapon() {
    // The urgent pass runs as a reflex, ahead of loot and ahead of
    // the fight, and ignored `out_of_combat_only` outright: a buff
    // with a minute still on it was enough to put the sword away
    // mid-swing. Under god mode that cost throughput; for a mortal
    // character it is a fight fought bare-handed.
    let (sword, wand) = (false, true);
    let mut cfg = Buffs {
        never_below: 60.0,
        top_up_within: 300.0,
        out_of_combat_only: true,
        ..Buffs::default()
    };
    assert_eq!(
        buff_within(&cfg, true, true, sword),
        0.0,
        "only what has lapsed"
    );
    // A buff that is not up at all reads as nought seconds left, so
    // it still goes back up in the middle of a fight. That is what
    // "never below" is for.
    assert!(0.0 <= buff_within(&cfg, true, true, sword));
    // With a wand already in hand the recast costs a cast and
    // nothing else, so `never_below` holds. Answering nought here
    // meant a mortal caster's protections were put back only after
    // they had lapsed -- the very window the setting names.
    assert_eq!(
        buff_within(&cfg, true, true, wand),
        60.0,
        "a free recast was still made to wait for the fight"
    );
    // Out of the fight the urgent pass is unchanged, and so is the
    // quiet one either way.
    assert_eq!(buff_within(&cfg, true, false, sword), 60.0);
    assert_eq!(buff_within(&cfg, false, true, sword), 300.0);
    // And a player who has not asked for the restraint keeps the
    // old behaviour: buffs go back up mid-fight.
    cfg.out_of_combat_only = false;
    assert_eq!(buff_within(&cfg, true, true, sword), 60.0);
}

/// One step-table tick: the urgent reflex first, the top-up goal only
/// when the reflex did not act, nothing else claiming the tick.
fn tick(c: &mut Client, now: Instant) -> bool {
    c.autoplay_buff(now, true) || c.autoplay_buff(now, false)
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_top_up_held_off_for_mana_does_not_keep_an_urgent_buff_down() {
    // Mana between the two reserves: the top-up (all of `keep_mana`)
    // holds off, the urgent recast (half of it) may go. With one clock
    // for both passes, the top-up looking every tick kept the urgent
    // pass from ever looking, and the buff stayed down.
    let now = Instant::now();
    let (mut c, _) = crate::testkit::a_caster_with_a_buff_due(now);
    c.world.stats.vitals[2].base = 1000;
    c.autoplay.config.buffs.keep_mana = 0.6;
    assert!(!c.autoplay_buff(now, false), "the top-up spent the reserve");
    let sent = c.session.actions_sent();
    let step = Duration::from_millis(50);
    let mut then = now;
    while !tick(&mut c, then) {
        then += step;
        assert!(
            then.duration_since(now) <= BUFF_CHECK_EVERY + step,
            "the urgent buff was still down after {:?}",
            then.duration_since(now)
        );
    }
    assert!(c.session.actions_sent() > sent, "the cast did not go out");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_top_up_pass_looks_again_only_once_a_second() {
    // Working out what is due walks the whole spellbook, and the
    // top-up goal is reached on most ticks of a fighter's day.
    let now = Instant::now();
    let (mut c, _) = crate::testkit::a_caster_with_a_buff_due(now);
    // No mana: the buff cannot be cast, so nothing is due.
    c.world.stats.vitals[2].current = 0;
    assert!(!c.autoplay_buff(now, false), "a cast with no mana");
    // The mana comes back: the pass sees it at its next look.
    c.world.stats.vitals[2].current = 500;
    let sent = c.session.actions_sent();
    for ms in [50, 500, 950] {
        let then = now + Duration::from_millis(ms);
        assert!(!c.autoplay_buff(then, false), "looked again after {ms} ms");
    }
    assert_eq!(c.session.actions_sent(), sent, "something went out");
    assert!(
        c.autoplay_buff(now + BUFF_CHECK_EVERY, false),
        "no look a second after the last"
    );
    assert!(c.session.actions_sent() > sent, "the cast did not go out");
}

#[test]
fn each_buff_pass_looks_once_a_second_on_its_own_clock() {
    // No game data: nothing is ever due, and each look is seen only on
    // its pass's clock.
    let mut c = crate::testkit::offline_client();
    let now = Instant::now();
    let later = now + Duration::from_millis(500);
    assert!(!c.autoplay_buff(now, false));
    assert_eq!(c.autoplay.top_ups_checked, Some(now), "no top-up look");
    assert!(!c.autoplay_buff(later, false));
    assert_eq!(
        c.autoplay.top_ups_checked,
        Some(now),
        "the top-up looked again within a second"
    );
    // The top-up's look half a second ago does not hold the urgent
    // pass back, and the urgent pass's own clock holds it.
    assert!(!c.autoplay_buff(later, true));
    assert_eq!(
        c.autoplay.urgent_buffs_checked,
        Some(later),
        "no urgent look"
    );
    assert!(!c.autoplay_buff(now + BUFF_CHECK_EVERY, true));
    assert_eq!(
        c.autoplay.urgent_buffs_checked,
        Some(later),
        "the urgent pass looked again within a second"
    );
    // A second on, the top-up looks again.
    assert!(!c.autoplay_buff(now + BUFF_CHECK_EVERY, false));
    assert_eq!(c.autoplay.top_ups_checked, Some(now + BUFF_CHECK_EVERY));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn counting_the_pack_once_answers_as_counting_it_for_every_spell() {
    use crate::magic::CastCheck;
    // Stocked for two buffs only, so most of the book is short of
    // something; every spell in the table in the book, and every
    // school's skill trained, a few beyond the caster's reach.
    let now = Instant::now();
    let (mut c, _) =
        crate::testkit::a_caster_knowing(now, &["Blade Protection Self I", "Strength Self I"]);
    let table = c.assets.spell_table().expect("the spell table");
    c.world.stats.spells = table.spells.iter().map(|(id, _)| *id).collect();
    for (skill, level) in [(32, 120), (34, 200), (43, 60)] {
        c.world.stats.skills.push(ac_world::stats::Skill {
            id: skill,
            advancement: ac_world::stats::sac::TRAINED,
            init_level: level,
            ..Default::default()
        });
    }
    let me = c.world.player_guid;
    let wand = c.wielded_caster().expect("a wand in hand");
    // As built; short of mana; with the Foci of Enchantment carried,
    // which turns the Creature formulas to scarabs; and with no wand.
    for state in ["as built", "no mana", "a focus", "no wand"] {
        match state {
            "no mana" => c.world.stats.vitals[2].current = 10,
            "a focus" => {
                c.world.stats.vitals[2].current = 500;
                c.world.objects.insert(
                    0x8000_0300,
                    ac_world::WorldObject {
                        guid: 0x8000_0300,
                        name: "Foci of Enchantment".into(),
                        weenie_class_id: 15268,
                        container: me,
                        ..Default::default()
                    },
                );
            }
            "no wand" => {
                let w = c.world.objects.get_mut(&wand).expect("the wand");
                w.wielder = None;
                w.container = me;
            }
            _ => {}
        }
        let carried = c.components();
        let mut answers = std::collections::BTreeMap::new();
        for &id in &c.world.stats.spells {
            let once = c.can_cast_from(id, Some(&carried));
            assert_eq!(once, c.can_cast(id), "{state}: spell {id}");
            // The answer's name, `MissingComponents` from `MissingComponents([..])`.
            let answer = format!("{once:?}");
            let kind = answer
                .split(['(', ' '])
                .next()
                .unwrap_or_default()
                .to_string();
            *answers.entry(kind).or_insert(0) += 1;
        }
        // The old wanted list, counting the pack for every spell.
        let per_spell =
            c.wanted_buffs_if(|id| matches!(c.can_cast(id), CastCheck::Ok | CastCheck::NoCaster));
        assert_eq!(c.wanted_buffs(), per_spell, "{state}");
        assert!(
            answers.len() > 1,
            "{state}: one answer for the whole book: {answers:?}"
        );
    }
}

#[test]
fn a_fighter_with_no_buff_due_is_not_held_back_between_swings() {
    // The pass booked the gap after the swing before it had looked for a buff at all, so a
    // character with nothing to cast had a swing held back on every look.
    let now = Instant::now();
    let mut c = Client::offline(crate::testkit::no_data());
    c.autoplay.config.buffs.auto = true;
    c.attack_pending = true;
    c.last_attack = now;
    assert!(c.mid_attack(), "the test needs a swing in the air");
    assert!(!c.autoplay_buff(now, true));
    assert!(
        !c.wants_the_hands,
        "a swing was held back for a buff that was never due"
    );
}
