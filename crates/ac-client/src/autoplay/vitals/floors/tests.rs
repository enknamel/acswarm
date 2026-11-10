use super::*;
use crate::autoplay::vitals::buffs::BUFF_CHECK_EVERY;
use crate::testkit::{self, ME};

const TUSKER: u32 = 0x8000_1234;
const KIN: u32 = 0x8000_1235;

/// Health, stamina and mana bars of `most` points each, at these fractions.
fn bars_at(c: &mut Client, most: u32, health: f32, stamina: f32, mana: f32) {
    for (i, f) in [health, stamina, mana].into_iter().enumerate() {
        c.world.stats.vitals[i].base = most;
        c.world.stats.vitals[i].current = (most as f32 * f) as u32;
    }
}

/// A wand fighter in Holtburg knowing `spells`, its bars at full.
fn mage_knowing(now: Instant, spells: &[&str]) -> (Client, Vec<u32>) {
    let (mut c, known) = testkit::a_caster_knowing(now, spells);
    c.autoplay.config.fight.style = Style::Magic;
    bars_at(&mut c, 200, 1.0, 1.0, 1.0);
    (c, known)
}

/// A Tusker Guard a swing east of the character, hitting it and fought by it.
fn tusker_on(c: &mut Client) -> Vec3 {
    let mut o = testkit::standing_by(c, TUSKER, "Tusker Guard", 2.0);
    o.walked_at = Some(ME);
    c.world.objects.insert(TUSKER, o.clone());
    c.hear_attack("Tusker Guard", Instant::now());
    c.take_up_fight(TUSKER);
    o.world_pos().unwrap()
}

/// A Tusker Guard `metres` east, turned to the character and casting at it from there.
fn casting_from(c: &mut Client, guid: u32, metres: f32, now: Instant) {
    let mut o = testkit::standing_by(c, guid, "Tusker Guard", metres);
    o.turned_to = Some(ME);
    c.world.objects.insert(guid, o);
    c.hear_attack("Tusker Guard", now);
}

/// An offline wand fighter in Holtburg with stamina and mana under their floors.
fn mage_spent() -> Client {
    let mut c = testkit::offline_client();
    c.world.player_guid = Some(ME);
    testkit::stand(&mut c, 0xA9B4_0019, Vec3::new(84.0, 84.0, 94.0));
    c.autoplay.config.fight.style = Style::Magic;
    bars_at(&mut c, 200, 1.0, 0.1, 0.3);
    c
}

#[test]
fn an_attack_is_put_to_the_creature_of_that_name_after_the_character() {
    let now = Instant::now();
    let mut c = mage_spent();
    // Its kin stands nearer, after nobody: the one turned to the character cast.
    testkit::standing_by(&mut c, KIN, "Tusker Guard", 5.0);
    casting_from(&mut c, TUSKER, 50.0, now);
    assert!(c.hit_lately_from(TUSKER, now, HIT_LATELY));
    assert!(
        !c.hit_lately_from(KIN, now, HIT_LATELY),
        "the kin was blamed"
    );
    // None of the name after it: the nearest of that name.
    c.world.objects.get_mut(&TUSKER).unwrap().turned_to = None;
    c.hear_attack("Tusker Guard", now);
    assert!(c.hit_lately_from(KIN, now, HIT_LATELY));
}

#[test]
fn a_caster_hitting_from_fifty_metres_is_fled_till_the_character_is_cornered() {
    // Casters hit from 75 m and archers from 80; counted within 30 m only, one rested being shot.
    let now = Instant::now();
    let mut c = mage_spent();
    testkit::standing_by(&mut c, KIN, "Tusker Guard", 20.0);
    casting_from(&mut c, TUSKER, 50.0, now);
    let me = c.my_position().unwrap();
    let round = c.attackers_round(me, now);
    assert_eq!(round.len(), 1, "{round:?}");
    assert!((round[0].distance(me) - 50.0).abs() < 0.5, "{round:?}");
    // Between its casts it is still on the character; ten quiet seconds on, it is not.
    let between = now + Duration::from_secs(6);
    assert_eq!(c.attackers_round(me, between).len(), 1);
    assert!(c.attackers_round(me, now + HIT_LATELY).is_empty());
    // The legs are kept while it hits: walked them all, cornered, it is fought.
    c.autoplay.falling_back = Some(FallBack {
        legs: LEGS,
        ..FallBack::default()
    });
    assert!(!c.autoplay_fall_back(between), "rested while shot");
    assert!(c.is_cornered());
    assert!(!c.is_falling_back());
}

#[test]
fn a_cast_that_can_never_leave_the_floor_wants_a_nearly_full_bar() {
    // The 40% floor of 200 is 80: a 50-point cast wants 130, a 120-point one the whole bar.
    assert_eq!(mana_wanted(50, 200, 0.4), 130);
    assert_eq!(mana_wanted(120, 200, 0.4), 200);
    // 150 never leaves 80: the bar within 5% of its top, never past it.
    assert_eq!(mana_wanted(150, 200, 0.4), 190);
    assert_eq!(mana_wanted(400, 200, 0.4), 190);
}

#[test]
fn a_bar_is_short_under_its_floor_and_rested_only_past_the_margin() {
    let cfg = Survive::default();
    // Mana at 30% is under its 40% floor, for a character whose mana binds.
    assert_eq!(short_bar([1.0, 1.0, 0.3], &cfg, true, 0.0), Some(2));
    assert_eq!(short_bar([1.0, 1.0, 0.3], &cfg, false, 0.0), None);
    // Back at 45% it is above the floor but not yet past the margin.
    assert_eq!(short_bar([1.0, 1.0, 0.45], &cfg, true, 0.0), None);
    assert_eq!(
        short_bar([1.0, 1.0, 0.45], &cfg, true, RESTED_MARGIN),
        Some(2)
    );
    // Health and stamina always bind; a floor of nought is off.
    assert_eq!(short_bar([1.0, 0.2, 1.0], &cfg, false, 0.0), Some(1));
    let off = Survive {
        heal_below: 0.0,
        ..Survive::default()
    };
    assert_eq!(short_bar([0.1, 1.0, 1.0], &off, true, 0.0), None);
    // A floor and margin past the top are met by a full bar.
    let high = Survive {
        heal_below: 0.95,
        ..Survive::default()
    };
    assert_eq!(short_bar([1.0, 1.0, 1.0], &high, true, RESTED_MARGIN), None);
}

#[test]
fn away_is_straight_from_the_attackers_middle() {
    let me = Vec3::ZERO;
    let way = away_from(me, &[Vec3::new(2.0, 0.0, 0.0), Vec3::new(2.0, 2.0, 0.0)]);
    assert!(
        (way - Vec2::new(-1.0, -0.5).normalize()).length() < 1e-5,
        "{way}"
    );
    // Stood on: any way at all.
    assert_eq!(away_from(me, &[me]), Vec2::X);
}

#[test]
fn the_fall_back_rests_and_ends_once_the_bars_are_back() {
    let now = Instant::now();
    let mut c = testkit::offline_client();
    c.world.player_guid = Some(ME);
    testkit::stand(&mut c, 0xA9B4_0019, Vec3::new(84.0, 84.0, 94.0));
    c.autoplay.config.fight.style = Style::Magic;
    bars_at(&mut c, 200, 1.0, 0.1, 0.3);
    c.autoplay.falling_back = Some(FallBack::default());
    // Nothing on it: it rests where it stands, and no fight is worth picking.
    assert!(c.autoplay_fall_back(now), "the rest held no tick");
    assert_eq!(c.autoplay.doing, Doing::Resting);
    assert!(c.is_falling_back());
    // Mana above its floor but short of the margin: still resting.
    bars_at(&mut c, 200, 1.0, 1.0, 0.5);
    assert!(c.autoplay_fall_back(now), "rested short of the margin");
    // Every bar back past its floor and the margin: the fights come back.
    bars_at(&mut c, 200, 1.0, 1.0, 0.6);
    assert!(!c.autoplay_fall_back(now), "the rest did not end");
    assert!(c.autoplay.falling_back.is_none());
    assert!(!c.is_falling_back());
}

#[test]
fn a_held_cast_rests_till_its_mana_is_there_too() {
    let now = Instant::now();
    let mut c = testkit::offline_client();
    c.world.player_guid = Some(ME);
    testkit::stand(&mut c, 0xA9B4_0019, Vec3::new(84.0, 84.0, 94.0));
    bars_at(&mut c, 200, 1.0, 1.0, 0.6);
    // The floor (80) and a 50-point cast: 130 wanted, past the floor and margin's 110.
    c.autoplay.falling_back = Some(FallBack {
        mana_for: 130,
        ..FallBack::default()
    });
    assert!(c.autoplay_fall_back(now), "rested with 120 mana of 130");
    bars_at(&mut c, 200, 1.0, 1.0, 0.65);
    assert!(!c.autoplay_fall_back(now), "130 mana and still resting");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn at_thirty_percent_mana_with_stamina_under_its_floor_the_fight_is_broken_off_away_from_the_attacker(
) {
    // Blargerton in the Tusker Barracks: stamina spent on Stamina to Mana, mana at 0-3, and he
    // fought on with Heal Self out of reach until his health went.
    let now = Instant::now();
    let (mut c, _) = mage_knowing(now, &["Flame Bolt I", "Stamina to Mana Self I"]);
    bars_at(&mut c, 200, 1.0, 0.1, 0.3);
    let tusker = tusker_on(&mut c);
    let me = c.my_position().unwrap();
    let mut walking = false;
    for _ in 0..50 {
        assert!(c.autoplay_fall_back(now), "the fight was not broken off");
        if c.going.is_some() {
            walking = true;
            break;
        }
    }
    assert!(walking, "no walk away: {}", c.autoplay.status);
    assert_eq!(
        c.fight_held(),
        (false, false, false, false),
        "the fight is held"
    );
    let spot = c.going.as_ref().unwrap().goal.place().unwrap().at;
    let (off, was) = (spot.distance(tusker), me.distance(tusker));
    assert!(
        off > was + LEG * 0.5,
        "{off:.1} m from the tusker, from {was:.1}"
    );
    // No fight is picked meanwhile.
    assert!(c.is_falling_back());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn an_attack_spell_waits_under_the_mana_floor_while_stamina_can_restore_it() {
    let now = Instant::now();
    let (mut c, known) = mage_knowing(now, &["Flame Bolt I", "Stamina to Mana Self I"]);
    let bolt = known[0];
    // Mana full: the bolt goes.
    assert!(!c.hold_for_mana(bolt, now));
    // Under the floor with a full stamina bar: the transfer goes first.
    bars_at(&mut c, 200, 1.0, 1.0, 0.3);
    assert!(c.hold_for_mana(bolt, now), "the bolt went under the floor");
    assert!(
        c.autoplay.status.contains("pouring stamina into mana"),
        "{}",
        c.autoplay.status
    );
    assert!(
        c.autoplay.falling_back.is_none(),
        "broke off with a restore to hand"
    );
    // Stamina under its own floor as well: nothing restores mana here, and the fight is broken off.
    c.autoplay.cast_answered();
    bars_at(&mut c, 200, 1.0, 0.1, 0.3);
    assert!(c.hold_for_mana(bolt, now));
    assert!(
        c.autoplay.falling_back.is_some(),
        "fought on under the floor"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_transfer_that_would_take_stamina_under_its_floor_waits() {
    let now = Instant::now();
    let (mut c, _) = mage_knowing(now, &["Stamina to Mana Self I"]);
    // Half the stamina bar, capped at 50, leaves 50 of 200: 25%, under the 30% floor.
    bars_at(&mut c, 200, 1.0, 0.5, 0.2);
    assert!(
        !c.autoplay_vitals(now),
        "the transfer took stamina under its floor"
    );
    bars_at(&mut c, 200, 1.0, 1.0, 0.2);
    assert!(c.autoplay_vitals(now), "a full bar poured nothing");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_buff_that_would_end_under_the_mana_floor_waits() {
    let now = Instant::now();
    let (mut c, _) = testkit::a_caster_with_a_buff_due(now);
    c.world.stats.vitals[2].base = 1000;
    // Over the 35% buff reserve after the cast, under the 40% floor.
    c.world.stats.vitals[2].current = 401;
    assert!(
        !c.autoplay_buff(now, false),
        "the buff went under the mana floor"
    );
    c.world.stats.vitals[2].current = 1000;
    assert!(
        c.autoplay_buff(now + BUFF_CHECK_EVERY, false),
        "the buff waited with mana to spare"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_tick_falls_back_before_the_fight_step_casts() {
    // The whole tick, not the row alone: no reflex or goal above it casts the bolt first.
    let now = Instant::now();
    let (mut c, _) = mage_knowing(now, &["Flame Bolt I", "Stamina to Mana Self I"]);
    c.autoplay.config.enabled = true;
    bars_at(&mut c, 200, 1.0, 0.1, 0.3);
    tusker_on(&mut c);
    for _ in 0..5 {
        c.tick_autoplay(now);
        assert_eq!(c.autoplay.step, Some("fall back"), "{}", c.autoplay.status);
    }
    assert!(
        c.autoplay.casting_at().is_none(),
        "still casting at the tusker"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_cast_dearer_than_the_bar_over_its_floor_goes_from_a_full_bar() {
    let now = Instant::now();
    let (mut c, known) = mage_knowing(now, &["Flame Bolt I"]);
    let bolt = known[0];
    let cost = c.spell(bolt).map(|sp| c.mana_cost(&sp)).unwrap();
    assert!(cost > 0);
    // A bar of `cost` points: the floor and the cast are past its top.
    bars_at(&mut c, cost, 1.0, 1.0, 1.0);
    assert!(c.can_spare_mana(bolt), "a full bar held the bolt");
    assert!(!c.hold_for_mana(bolt, now));
    // Half the bar and nothing to restore it: rested back to full, then the bolt goes.
    bars_at(&mut c, cost, 1.0, 1.0, 0.5);
    assert!(c.hold_for_mana(bolt, now));
    bars_at(&mut c, cost, 1.0, 1.0, 1.0);
    assert!(!c.autoplay_fall_back(now), "a full bar rested on");
    assert!(c.autoplay.falling_back.is_none());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_healer_heals_a_fellow_under_its_own_mana_floor() {
    let now = Instant::now();
    let (mut c, _) = testkit::a_caster_knowing(now, &["Heal Other I"]);
    bars_at(&mut c, 200, 1.0, 1.0, 0.3);
    let team = &mut c.autoplay.config.team;
    team.enabled = true;
    team.role = crate::autoplay::Role::Healer;
    team.fellowship = false;
    team.share_supplies = false;
    let mut aldric = testkit::mate(0x5000_0002, "Aldric");
    aldric.health = 0.3;
    c.autoplay.team.mates.push(aldric);
    assert!(c.autoplay_team(now), "no heal: {}", c.autoplay.status);
    assert!(
        c.autoplay.status.contains("healing Aldric"),
        "{}",
        c.autoplay.status
    );
}
