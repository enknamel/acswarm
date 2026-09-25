use super::*;

#[test]
fn the_order_says_what_the_character_cares_about() {
    let at = |name: &str| {
        STEPS
            .iter()
            .position(|s| s.name == name)
            .unwrap_or_else(|| panic!("no step called {name}"))
    };

    // Staying alive comes before everything. These are the ones
    // that cost a character its life when they are wrong.
    // Healing outranks everything, a spell already in the air
    // included. Stepping out of the way is worth little to a
    // character that dies while doing it, and healing is paced by
    // the server, so it gives the tick back between casts and the
    // dodge loses almost nothing by going second.
    assert!(at("survive") < at("dodge"), "heal before stepping out");
    // A body on the floor beats a fight that has to be walked to:
    // it is already dead, already ours, and rotting on a clock,
    // while the creature across the room will still be there.
    // A body on the floor beats a fight that has to be walked to.
    // Held as a constant rather than an assertion because it is one:
    // the two numbers are fixed, and the compiler checks it for
    // free the moment either changes.
    const _: () = assert!(WALK_TO_A_FIGHT < LOOT_AT_REST);
    assert_eq!(at("survive"), 0, "nothing comes before staying alive");
    assert!(at("survive") < at("fight"), "heal before fighting");
    assert!(at("survive") < at("loot"), "heal before looting");
    assert!(at("recover") < at("fight"), "deal with the corpse first");

    // A buff about to lapse goes up before the fight; the rest wait
    // until after it.
    assert!(at("urgent buffs") < at("fight"));
    assert!(at("fight") < at("buffs"));
    assert!(at("summon") < at("fight"), "the creature joins the fight");
    assert!(
        at("fight") < at("keep to the area"),
        "defend first, then go back"
    );
    assert!(at("keep to the area") < at("explore"));
    assert!(at("keep to the area") < at("grow"));

    // A corpse rots and a shop does not.
    assert!(at("loot") < at("town run"), "loot before shopping");
    assert!(at("loot") < at("salvage"));
    // A recall to town is free: supplies before the next fight, never instead of one in hand.
    assert!(at("town run") < at("summon"), "town before calling a pet");
    assert!(at("town run") < at("fight"), "town before the next fight");

    // And the last word is the one that finds something to do.
    assert_eq!(STEPS.last().map(|s| s.name), Some("grow"));
}

#[test]
fn the_pack_is_tidied_as_housekeeping_and_never_claims_a_tick() {
    // Pouring one carried stack into another is made by the server
    // on the spot, so it costs no tick -- and as a goal it never
    // won one, because a character that fights, loots and walks has
    // no quiet tick to give it.
    assert!(
        HOUSEKEEPING.iter().any(|h| h.name == "tidy the pack"),
        "tidying is housekeeping now"
    );
    assert!(
        named("tidy").is_none(),
        "and it holds no row: the scores are written out, so it needed no placeholder"
    );
}

#[test]
fn experience_is_spent_as_housekeeping_and_never_claims_a_tick() {
    // A rank is one message the server takes on the spot. As the
    // first thing the last goal did, it waited for a tick nothing
    // else wanted, and exploring always wanted it: a character given
    // a hundred billion experience spent none of it.
    assert!(
        HOUSEKEEPING.iter().any(|h| h.name == "spend experience"),
        "spending experience is housekeeping now"
    );
    let grow = named("grow").expect("growing is still a goal");
    assert!(
        !grow
            .why
            .starts_with("with nothing else to do: spend experience"),
        "and the goal no longer claims to spend it"
    );
}

#[test]
fn every_goal_keeps_the_worth_it_had_when_tidying_moved() {
    // Six goals are weighed against fixed scores, so these numbers are
    // the contract. They are written in the table now, so a row may come
    // or go without moving them.
    let base = |name: &str| STEPS.iter().find(|s| s.name == name).expect(name).base;
    assert_eq!(STEPS.len(), 19);
    assert_eq!(base("town run"), 95.0);
    assert_eq!(base("fight"), 80.0);
    assert_eq!(base("summon"), 90.0);
    assert_eq!(base("keep to the area"), 70.0);
    assert_eq!(base("buffs"), 60.0);
    assert_eq!(base("follow"), 50.0);
    assert_eq!(base("resume the journey"), 40.0);
}

#[test]
fn every_step_says_what_it_is_and_why_it_is_there() {
    assert!(!STEPS.is_empty());
    for s in STEPS {
        assert!(!s.name.is_empty());
        assert!(
            s.why.len() > 20,
            "{} should say why it sits where it does",
            s.name
        );
    }
    // Names are unique, or naming one in a log is ambiguous.
    let mut names: Vec<&str> = STEPS.iter().map(|s| s.name).collect();
    names.sort_unstable();
    let before = names.len();
    names.dedup();
    assert_eq!(names.len(), before, "two steps share a name");
    assert!(named("fight").is_some());
    assert!(named("nonsense").is_none());
}

#[test]
fn a_goal_with_no_opinion_keeps_its_place() {
    // The whole safety of the change: scoring must reproduce the
    // table's order exactly until a curve is written on purpose.
    let places: Vec<usize> = STEPS
        .iter()
        .enumerate()
        .filter(|(_, s)| s.layer == Layer::Goal)
        .map(|(i, _)| i)
        .collect();
    let base = |place: usize| STEPS[place].base;
    for pair in places.windows(2) {
        assert!(
            base(pair[0]) > base(pair[1]),
            "{} should outrank {}",
            STEPS[pair[0]].name,
            STEPS[pair[1]].name
        );
    }
    // Every place is worth something, so a goal with no opinion is
    // always considered rather than skipped.
    assert!(places.iter().all(|&p| base(p) > 0.0));
    // And a thing worth a lot beats every place in the table.
    assert!(WORTH_A_LOT > base(0));
}

#[test]
fn a_body_gets_more_worth_going_back_for_the_longer_it_waits() {
    let base = |name: &str| STEPS.iter().find(|s| s.name == name).expect(name).base;
    let fight = base("fight");
    // The arithmetic of the curve, without a world to hang it on.
    let worth = |left_secs: f32, waiting: usize| {
        let aged = 1.0 - left_secs / crate::autoplay::CORPSE_LIFE.as_secs_f32();
        LOOT_AT_REST + aged * AGE_IS_WORTH + waiting.min(WAITING_COUNTS) as f32 * EACH_BODY_IS_WORTH
    };

    // Just dropped, nothing else waiting: the fight that made it
    // comes first.
    assert!(worth(300.0, 1) < fight, "a fresh body waits its turn");
    // A third of its life gone: worth going back for before
    // starting another fight. This is the point of the curve -- a
    // character that only breaks off for a rotting corpse never
    // clears the pile behind it.
    assert!(worth(200.0, 1) > fight, "an ageing body comes first");
    // And bodies piling up says the same thing sooner.
    assert!(
        worth(280.0, 5) > worth(280.0, 1),
        "a floor full of bodies is worth more than one"
    );
    assert!(worth(280.0, 6) > fight, "a pile outranks the next fight");
    // It never runs away with itself: even a heap of old bodies
    // does not outrank a corpse that is actually about to go.
    assert!(worth(80.0, 6) < WORTH_A_LOT);
}

#[test]
fn a_body_about_to_rot_outranks_the_next_fight() {
    // Written as a curve rather than as an exception buried in the
    // middle of choosing a corpse: a fight will still be there
    // afterwards and the body will not.
    let at = |name: &str| STEPS.iter().position(|s| s.name == name).expect(name);
    let base = |place: usize| STEPS[place].base;
    assert!(
        base(at("loot")) < WORTH_A_LOT,
        "an urgent corpse must be able to outrank its own place"
    );
    assert!(WORTH_A_LOT > base(at("fight")), "and to outrank the fight");
    // While a body will keep, looting stays where it always was:
    // after the reflexes and before the shops.
    assert!(base(at("loot")) > base(at("grow")));
}

#[test]
fn the_reflexes_come_first_and_are_few() {
    // Once the goals start, no reflex follows: a reflex that ran
    // after a goal had claimed the tick would never run at all.
    let first_goal = STEPS
        .iter()
        .position(|s| s.layer == Layer::Goal)
        .expect("some goals");
    assert!(
        STEPS[first_goal..].iter().all(|s| s.layer == Layer::Goal),
        "a reflex is sitting below a goal"
    );
    // They are the per-tick cost every session pays, so there
    // should not be many.
    let reflexes = STEPS.iter().filter(|s| s.layer == Layer::Reflex).count();
    assert!(reflexes <= 8, "{reflexes} reflexes is a lot to pay a tick");
}

#[test]
fn a_fight_in_hand_is_never_interrupted() {
    // Swinging at something still alive outranks everything below
    // it, whatever is on the floor.
    assert_eq!(fight_worth(true, true, 100.0), UNDECIDED);
}

#[test]
fn a_body_it_made_comes_before_the_next_fight() {
    // The option: finish what you kill. Even with something to hit
    // right here, the body goes first.
    assert!(fight_worth(false, true, 0.0) < LOOT_AT_REST);
}

#[test]
fn a_fight_across_the_room_does_not_beat_loot_at_your_feet() {
    assert!(fight_worth(false, false, IN_REACH_OF_A_FIGHT + 1.0) < LOOT_AT_REST);
}

#[test]
fn a_fight_in_reach_still_beats_a_resting_body() {
    // Nothing stops to loot with something swinging at it.
    assert_eq!(fight_worth(false, false, 1.0), UNDECIDED);
}

#[test]
fn a_due_town_run_waits_only_for_the_bodies_of_ours() {
    // No body: its place, over starting any fight.
    assert_eq!(town_run_worth(0.0), UNDECIDED);
    assert!(named("town run").expect("row").base > named("fight").expect("row").base);
    // A body waiting: just under looting it, however much that is worth.
    assert!(town_run_worth(LOOT_AT_REST + 12.0) < LOOT_AT_REST + 12.0);
    assert!(
        town_run_worth(LOOT_AT_REST) > WALK_TO_A_FIGHT,
        "over a fight that waits on it"
    );
    assert!(
        town_run_worth(WORTH_A_LOT) <= TOWN_RUN,
        "never over the team or salvage"
    );
}

#[test]
fn a_run_under_way_keeps_its_old_place() {
    // Starting a run outranks a fight; carrying one on sits where the grow step carried it, under
    // buffs, following and exploring (which defers to it), over grow.
    let base = |name: &str| named(name).expect(name).base;
    assert!(RUN_UNDER_WAY < base("explore"));
    assert!(RUN_UNDER_WAY < base("buffs") && RUN_UNDER_WAY < base("follow"));
    assert!(RUN_UNDER_WAY > base("grow"));
}

#[test]
fn a_town_run_gives_way_to_a_fight_in_hand() {
    let mut c = in_the_field();
    c.autoplay.config.growth.town_runs = true;
    let now = Instant::now();
    let row = named("town run").expect("row");
    assert_eq!(row.worth(&c, now), TOWN_RUN);
    let it = crate::testkit::standing_by(&mut c, 0x8000_0001, "Drudge Skulker", 3.0);
    c.attack_target = Some(it.guid);
    assert_eq!(row.worth(&c, now), 0.0, "the fight is finished first");
    c.attack_target = None;
    c.autoplay.config.growth.town_runs = false;
    assert_eq!(row.worth(&c, now), 0.0, "town runs off");
}

/// A character on its feet in the Holtburg field, level 20, with the
/// fight rules at their defaults and nothing about it.
fn in_the_field() -> Client {
    const HOLTBURG: u32 = 0xA9B4_0019;
    let mut c = crate::testkit::offline_client();
    c.world.player_guid = Some(crate::testkit::ME);
    c.world.stats.level = 20;
    crate::testkit::stand(&mut c, HOLTBURG, glam::vec3(84.0, 84.0, 94.0));
    c
}

/// What the fight step is worth where the character stands, the tick's
/// own measurement taken first the way `tick_autoplay` takes it before
/// it weighs anything (`Client::autoplay_watch_the_ground`).
fn fighting_is_worth(c: &mut Client, now: Instant) -> f32 {
    c.autoplay_watch_the_ground(now);
    named("fight").expect("the fight step").worth(c, now)
}

#[test]
fn a_creature_the_server_has_sent_no_health_for_is_a_fight_in_reach() {
    // ACE sends a health only for the target a player has selected
    // (Player_Vitals.cs:173) or one it has appraised
    // (WorldObject.cs:617), so a creature standing about has none at
    // all -- nearly every creature in view. Read as dead, the field was
    // hidden from this scan: fighting was worth a walk with a Revenant
    // two metres off, under keeping to the area (70), buffs (60),
    // follow (50) and a body at rest, while the fight step -- which
    // picks through `would_fight`, where an unknown health is alive --
    // would have gone and hit it.
    let mut c = in_the_field();
    let it = crate::testkit::standing_by(&mut c, 0x8000_0001, "Revenant", 2.0);
    c.world.objects.get_mut(&it.guid).expect("in view").health = None;
    let now = Instant::now();
    assert!(fighting_is_worth(&mut c, now) > LOOT_AT_REST);
    // One the server has said is dead is no fight at all.
    c.world.objects.get_mut(&it.guid).expect("in view").health = Some(0.0);
    assert_eq!(fighting_is_worth(&mut c, now), WALK_TO_A_FIGHT);
}

#[test]
fn what_the_character_would_not_fight_is_no_fight_in_reach() {
    // The scan asks the fight step's whole question and not a predicate
    // or two of it. Each of these stands at the character's feet and
    // none of them will be fought, so counting any one pinned the
    // distance near zero and turned the curve this scorer draws into a
    // constant: fighting outranked the body at its feet for as long as
    // the thing stood there.
    let now = Instant::now();

    // The creature it summoned itself, which is on by default and
    // fights at its owner's heel.
    let mut c = in_the_field();
    let pet = crate::testkit::standing_by(&mut c, 0x8000_0002, "Fire Elemental", 2.0);
    let o = c.world.objects.get_mut(&pet.guid).expect("in view");
    o.weenie_class_id = 0;
    o.pet_owner = crate::testkit::ME;
    o.walked_at = Some(0x8000_0009);
    assert_eq!(fighting_is_worth(&mut c, now), WALK_TO_A_FIGHT);

    // Another player: a player carries the creature item type in this
    // world model, which is why every other reader subtracts them by
    // hand.
    let mut c = in_the_field();
    let mate = crate::testkit::standing_by(&mut c, 0x5000_0002, "Bryn02", 2.0);
    c.world
        .objects
        .get_mut(&mate.guid)
        .expect("in view")
        .is_player = true;
    assert_eq!(fighting_is_worth(&mut c, now), WALK_TO_A_FIGHT);

    // And one it has given up reaching, which goes on standing there.
    let mut c = in_the_field();
    let it = crate::testkit::standing_by(&mut c, 0x8000_0003, "Revenant", 2.0);
    assert!(
        fighting_is_worth(&mut c, now) > LOOT_AT_REST,
        "a fight it would take on"
    );
    c.give_up_target(it.guid, "there is no way to it", now);
    assert_eq!(fighting_is_worth(&mut c, now), WALK_TO_A_FIGHT);
}

#[test]
fn a_fight_out_of_reach_leaves_the_body_at_its_feet_first() {
    // The arithmetic 8e7366b settled, in the shape a fleet run has it:
    // a character with no loot profile at all, so nothing holds the
    // fight back for a body (`waits_for_a_corpse` reads the profile's
    // `after_every_fight`, and there is no profile), a summoned
    // creature at its heel, and the nearest real fight past
    // `IN_REACH_OF_A_FIGHT`. The body goes first.
    let now = Instant::now();
    let mut c = in_the_field();
    assert!(c.loot_profile().is_none(), "no profile, so nothing is owed");
    let pet = crate::testkit::standing_by(&mut c, 0x8000_0004, "Fire Elemental", 2.0);
    let o = c.world.objects.get_mut(&pet.guid).expect("in view");
    o.weenie_class_id = 0;
    o.pet_owner = crate::testkit::ME;
    o.walked_at = Some(0x8000_0009);
    crate::testkit::standing_by(&mut c, 0x8000_0005, "Revenant", IN_REACH_OF_A_FIGHT + 1.0);
    assert_eq!(fighting_is_worth(&mut c, now), WALK_TO_A_FIGHT);
    const _: () = assert!(WALK_TO_A_FIGHT < LOOT_AT_REST);
}

#[test]
fn a_body_emptied_or_set_aside_does_not_make_looting_worth_more() {
    // Every body on the floor counted as one waiting to be looted:
    // those the looting had finished with, those set aside, and the
    // ones the server had let go that Blargerton kept in the Holtburg
    // Dungeon.
    use crate::autoplay::{Autoplay, Room, CORPSE_LIFE};
    use crate::did::Did;
    let now = Instant::now();
    let mut ap = Autoplay::default();
    let (emptied, locked, fresh) = (0x8000_5001, 0x8000_5002, 0x8000_5003);
    ap.looted.push(emptied, now);
    ap.shelved
        .note(locked, &Did::blocked("it will not open yet"), now);
    let room = Room::PLENTY;
    let waiting: Vec<u32> = [emptied, locked, fresh]
        .into_iter()
        .filter(|g| ap.corpse_waiting(*g, now, room))
        .collect();
    assert_eq!(waiting, vec![fresh]);
    // So the floor is worth one fresh body, not three.
    let one = worth_of_bodies([CORPSE_LIFE].into_iter());
    assert_eq!(one, LOOT_AT_REST + EACH_BODY_IS_WORTH);
    assert!(one < worth_of_bodies([CORPSE_LIFE; 3].into_iter()));
    // Nothing waiting is worth nothing.
    assert_eq!(worth_of_bodies(std::iter::empty()), 0.0);
    // The one set aside waits again once its wait is up.
    let later = now + std::time::Duration::from_secs(60);
    assert!(ap.corpse_waiting(locked, later, room));
    // And none waits on a pack with no room to take anything, so the
    // looting is worth nothing and the town run is not held up.
    let full = Room {
        pack_low: true,
        ..room
    };
    assert!(!ap.corpse_waiting(fresh, now, full));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn looting_is_worth_only_the_bodies_this_character_would_go_to() {
    // The two predicates disagreed: what looting was worth counted
    // every body waiting, ownership and all, while the looting step
    // and the fight both counted only the ones this character was
    // owed. Eight bodies another character had locked lifted the
    // loot score past the fight in hand, and the looting then won
    // the tick with nothing to go to. Nine characters spent 53% of
    // a run standing over bodies and 13% fighting.
    let holtburg = 0xA9B4_0019;
    let mut c = crate::testkit::standing_at(holtburg, glam::Vec3::new(84.0, 84.0, 94.0));
    c.world.player_guid = Some(0x5000_0009);
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let body = |guid: u32, at: glam::Vec3| ac_world::WorldObject {
        position: crate::testkit::placed(holtburg, at),
        ..crate::testkit::corpse(guid, "Corpse of Drudge Slave")
    };
    // One at the character's feet, and five more: three beside it
    // that teammates have claimed, and two lying out of reach.
    let mine = 0x8000_0001;
    c.world.objects.insert(mine, body(mine, me));
    let claimed: Vec<u32> = (2..5).map(|i| 0x8000_0000 + i).collect();
    for (n, &guid) in claimed.iter().enumerate() {
        let at = me + glam::Vec3::new(n as f32 + 1.0, 0.0, 0.0);
        c.world.objects.insert(guid, body(guid, at));
        c.autoplay.team.mates.push(crate::autoplay::Mate {
            autoplay: true,
            health: 1.0,
            world: at,
            looting: Some(guid),
            looting_for: std::time::Duration::ZERO,
            ..crate::testkit::mate(0x5000_0001 + n as u32, &format!("Mate {n}"))
        });
    }
    for i in 5..7u32 {
        let guid = 0x8000_0000 + i;
        let far = me + glam::Vec3::new(0.0, 60.0, 0.0);
        c.world.objects.insert(guid, body(guid, far));
    }
    // All six are waiting to be emptied; only one is this
    // character's to go to.
    let room = c.room_for_loot();
    assert_eq!(
        c.world
            .objects
            .values()
            .filter(|o| c.autoplay.corpse_waiting(o.guid, now, room))
            .count(),
        6
    );
    assert_eq!(
        c.world
            .objects
            .values()
            .filter(|o| c.corpse_for_us(o, me, now, room))
            .map(|o| o.guid)
            .collect::<Vec<_>>(),
        vec![mine]
    );
    // So the score is one body's, which is under the fight's.
    let one = worth_of_bodies([crate::autoplay::CORPSE_LIFE].into_iter());
    assert_eq!(worth_looting(&c, now), one);
    let fight = STEPS
        .iter()
        .find(|s| s.name == "fight")
        .map(|s| s.base)
        .expect("the fight is in the table");
    assert!(
        worth_looting(&c, now) < fight,
        "a body somebody else is opening outranked the fight"
    );

    // The body in hand is the looting's whatever the board says: a
    // teammate that claimed it too must not leave this character
    // holding one open on the server with no step left to close it.
    c.autoplay.team.mates.push(crate::autoplay::Mate {
        guid: 0x5000_0008,
        name: "Racer".into(),
        autoplay: true,
        health: 1.0,
        world: me,
        looting: Some(mine),
        looting_for: std::time::Duration::ZERO,
        ..Default::default()
    });
    assert!(!c.corpse_for_us(&body(mine, me), me, now, room));
    assert_eq!(worth_looting(&c, now), 0.0, "nothing to go to");
    c.autoplay
        .take_up_corpse(mine, now, std::time::Duration::from_secs(10));
    assert_eq!(worth_looting(&c, now), one, "let go of a body in hand");
}

#[test]
fn the_score_column_falls_from_first_to_last() {
    // The numbers are the ones the old arithmetic gave, less the 30 the
    // tidy row held: a row may come or go now without moving the rest,
    // which is the whole point of writing them down.
    assert_eq!(STEPS.first().map(|s| s.base), Some(190.0));
    assert_eq!(STEPS.last().map(|s| s.base), Some(10.0));
    for pair in STEPS.windows(2) {
        assert!(
            pair[0].base > pair[1].base,
            "{} must outrank {}",
            pair[0].name,
            pair[1].name
        );
    }
}

#[test]
fn a_reflex_that_claims_the_tick_names_the_step_that_did() {
    // Every reflex runs once a tick, from this table and nowhere
    // else. Four of them were called by hand above the table as well
    // as sitting in it: one that acted returned before the table was
    // reached, so it claimed the tick without naming itself, and on
    // every tick none of them acted all four ran a second time, in
    // the other order.
    let now = Instant::now();
    let mut c = crate::testkit::offline_client();
    crate::testkit::stand(&mut c, 0xA9B4_0019, glam::Vec3::new(84.0, 84.0, 94.0));
    c.world.player_guid = Some(crate::testkit::ME);
    c.autoplay.config.enabled = true;
    // Dead, with an Endurance to draw a maximum from: the recovery
    // reflex has the character and nothing below it gets the tick.
    c.world.stats.name = "Verity".into();
    c.world.stats.attributes[1].base = 100;
    c.world.stats.vitals[0].current = 0;
    assert!(c.is_dead(), "not dead");
    c.tick_autoplay(now);
    assert_eq!(
        c.autoplay.step,
        Some("recover"),
        "the reflex that claimed the tick did not name itself"
    );
}
