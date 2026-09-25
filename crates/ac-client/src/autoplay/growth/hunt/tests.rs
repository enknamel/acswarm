use super::*;
use crate::testkit::standing_at;

#[test]
fn a_named_ground_is_not_judged_on_level() {
    // Hunting a place is about its loot, its money, its trophies.
    // A ground the player named must not be refused because the
    // level table disapproves, and `suits` is the only thing that
    // would refuse it.
    let ground = ac_world::hunting::all()
        .iter()
        .find(|g| g.max_level < 50)
        .expect("a low-level ground");
    // Far too low for a level 275 character by the usual rule...
    assert!(!ground.suits(275, 8));
    // ...but naming it is a landblock id, and looking one up asks
    // nothing about levels.
    assert_eq!(
        ac_world::hunting::at(ground.landblock).map(|g| g.landblock),
        Some(ground.landblock)
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_quiet_ground_is_read_off_the_world_and_not_off_the_status_line() {
    // The roam clock counted only the frames where the engine had
    // found nothing to do, and put itself back to nothing the moment
    // anything ran. Asking a corpse that would not open is something
    // running, so nine characters queueing at one body restarted it
    // every couple of seconds: in ten minutes it never once reached
    // its minute, and they worked a 40 x 32 m corner of a
    // 197 x 192 m field.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 84.0, 94.0));
    let s = Duration::from_secs;
    let now = Instant::now();
    c.world.stats.level = 20;

    // Nothing about: the clock starts.
    c.autoplay_watch_the_ground(now);
    assert_eq!(c.autoplay.growth.quiet_since, Some(now));

    // And keeps running through everything the character does
    // meanwhile. This is the whole of the fix.
    c.autoplay
        .say(crate::autoplay::Doing::Looting, "opening a corpse");
    c.autoplay_watch_the_ground(now + s(5));
    assert_eq!(
        c.autoplay.growth.quiet_since,
        Some(now),
        "a corpse put the clock back to nothing"
    );

    // Something worth fighting inside the radius stops it.
    let guid = 0x8000_0001;
    let me = c.player.as_ref().unwrap().world_position();
    c.world.objects.insert(
        guid,
        ac_world::WorldObject {
            guid,
            weenie_class_id: 8592,
            name: "Revenant".into(),
            item_type: ac_world::item_type::CREATURE,
            object_desc_flags: ac_world::object_desc_flags::ATTACKABLE,
            health: Some(1.0),
            position: Some(ac_world::object::Position::new_flat(
                holtburg,
                me + glam::Vec3::new(5.0, 0.0, 0.0) - ac_world::landblock_origin(holtburg),
            )),
            ..Default::default()
        },
    );
    c.autoplay_watch_the_ground(now + s(6));
    assert_eq!(c.autoplay.growth.quiet_since, None);

    // A dead one is no fight, and the clock starts again from there.
    c.world.objects.get_mut(&guid).unwrap().health = Some(0.0);
    c.autoplay_watch_the_ground(now + s(7));
    assert_eq!(c.autoplay.growth.quiet_since, Some(now + s(7)));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_body_is_noted_when_it_appears_and_not_when_the_looting_gets_round_to_it() {
    // The bodies used to be noted inside the looting. A character
    // the claim tie-break tells to stand off scores that body
    // nothing, so its loot goal is never run, so it never notes the
    // body -- and a body it never noted is for ever "newly fallen",
    // which is exactly when the tie-break governs. One mate that
    // could not loot locked every body near it away from the other
    // eight for good.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 84.0, 94.0));
    let me = c.player.as_ref().unwrap().world_position();
    let s = Duration::from_secs;
    let now = Instant::now();
    let body = 0x8000_0001;
    c.world.objects.insert(
        body,
        ac_world::WorldObject {
            guid: body,
            name: "Corpse of Drudge Slave".into(),
            object_desc_flags: object_desc_flags::CORPSE,
            position: Some(ac_world::object::Position::new_flat(
                holtburg,
                me - ac_world::landblock_origin(holtburg),
            )),
            ..Default::default()
        },
    );
    // No loot profile, so nothing this character does will ever
    // reach past the first line of the looting step.
    c.autoplay.config.loot.profile = String::new();
    c.autoplay_watch_the_ground(now);
    assert_eq!(
        c.autoplay.corpse_seen.since(&body),
        Some(now),
        "a body nobody looted was never noted"
    );
    // Noted once, not restamped every tick: the age is what the
    // tie-break and the rotting order both read.
    c.autoplay_watch_the_ground(now + s(5));
    assert_eq!(c.autoplay.corpse_seen.since(&body), Some(now));
    assert_eq!(c.autoplay.corpse_seen.len(), 1);
    // Forgotten once emptied, so the list stays the size of what is
    // on the ground.
    c.autoplay.looted.push(body, now + s(5));
    c.autoplay_watch_the_ground(now + s(6));
    assert!(c.autoplay.corpse_seen.is_empty());
}

#[test]
fn a_hunting_area_is_walked_about_sooner_than_a_whole_new_ground_is_chosen() {
    // Moving on inside an area costs only the walk; moving on without
    // one means picking a whole new ground and travelling to it,
    // which is worth being slow about.
    let mut cfg = Growth::default();
    assert_eq!(quiet_before_move(&cfg, false), cfg.idle_before_move);
    assert_eq!(quiet_before_move(&cfg, true), PATROL_AFTER);
    assert!(PATROL_AFTER < cfg.idle_before_move);
    // A player who asked for less still gets less.
    cfg.idle_before_move = 4.0;
    assert_eq!(quiet_before_move(&cfg, true), 4.0);
    assert_eq!(quiet_before_move(&cfg, false), 4.0);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn only_the_one_leading_walks_the_area_looking_for_a_fight() {
    // Nine characters each picking their own corner of an outline
    // scatter the party across two hundred metres instead of moving
    // it. The followers keep to their leader and it takes them.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let cfg = Growth::default();
    c.world.stats.level = 20;
    c.autoplay.config.fight.area = Some(crate::hunt::HuntArea {
        name: "test".into(),
        shape: crate::hunt::Shape::Outline {
            points: vec![
                [me.x - 40.0, me.y - 5.0],
                [me.x + 40.0, me.y - 5.0],
                [me.x + 40.0, me.y + 60.0],
                [me.x - 40.0, me.y + 60.0],
            ],
        },
    });
    let team = &mut c.autoplay.config.team;
    team.enabled = true;
    team.follow = true;
    team.lead = false;
    c.autoplay.team.mates = vec![crate::autoplay::Mate {
        name: "Leader".into(),
        leader: true,
        leads: true,
        world: me,
        cell: holtburg,
        ..Default::default()
    }];
    // Quiet for long enough that the patrol would go.
    c.autoplay.growth.quiet_since = Some(now - Duration::from_secs(60));
    assert!(!c.grow_hunt(now, &cfg), "a follower wandered off");
    assert!(!c.traveling());

    // The one leading walks it.
    c.autoplay.team.mates.clear();
    c.autoplay.team.leader = true;
    assert!(c.grow_hunt(now, &cfg), "the leader stayed put");
    assert!(c.traveling());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_follower_with_no_hunting_area_does_not_go_looking_for_a_ground_of_its_own() {
    // The guard that keeps a follower from walking an area of its
    // own covered the roam as well, but not the tail below it: a
    // party with no area configured had its followers fail the roam
    // on the first quiet minute and fall straight through to picking
    // a landblock and travelling to it -- which is the scattering
    // the guard was added to stop, only sooner than before.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let cfg = Growth::default();
    c.world.stats.level = 20;
    let team = &mut c.autoplay.config.team;
    team.enabled = true;
    team.follow = true;
    team.lead = false;
    // A leader with no ground of its own to take, so nothing below
    // the roam can be following anybody.
    c.autoplay.team.mates = vec![crate::autoplay::Mate {
        name: "Leader".into(),
        leader: true,
        leads: true,
        world: me,
        cell: holtburg,
        ..Default::default()
    }];
    // Standing on the ground it is hunting, quiet long enough to
    // move on, and past its roams so the roam itself is refused.
    c.autoplay.growth.hunting_at = Some(holtburg >> 16);
    c.autoplay.growth.roams = ROAMS;
    c.autoplay.growth.quiet_since = Some(now - Duration::from_secs(600));
    assert!(!c.grow_hunt(now, &cfg), "a follower went hunting alone");
    assert!(!c.traveling());
    assert_eq!(
        c.autoplay.growth.bound, None,
        "bound for a ground of its own"
    );

    // The one leading still moves the party on.
    c.autoplay.team.mates.clear();
    c.autoplay.team.leader = true;
    assert!(c.grow_hunt(now, &cfg), "the leader stayed put");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_walk_to_a_ground_broken_off_by_a_corpse_is_walked_on() {
    // A body on the road -- a fellow's kill, shared -- took the
    // character off its walk to the ground, and walking to a corpse
    // ends a journey. Once the body was looted the hunting step found
    // no journey under way, took that for a walk that could not get
    // there, put the ground on the skip list and set off for another.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let cfg = Growth::default();
    let at = Vec2::new(me.x + 250.0, me.y);
    let lb = (((at.x / 192.0) as u32) << 8) | (at.y / 192.0) as u32;
    assert!(c.grow_travel(at, now), "no way to the ground");
    c.autoplay.growth.bound = Some((lb, at, "Drudge".into()));
    c.autoplay.growth.bound_since = Some(now);

    c.interrupt_travel("walking to a corpse");
    assert!(c.grow_hunt(now, &cfg), "gave the walk up after a corpse");
    assert!(c.traveling(), "and did not walk on");
    assert!(c.autoplay.growth.bound.is_some());
    assert!(c.autoplay.growth.skip.is_empty(), "the ground was skipped");

    // Broken off again as soon as it is planned, it waits a moment
    // before planning the walk once more.
    c.interrupt_travel("walking to a corpse");
    assert!(c.grow_hunt(now + Duration::from_secs(1), &cfg));
    assert!(!c.traveling(), "planned again straight away");
    assert!(c.grow_hunt(now + WALK_ON_EVERY, &cfg));
    assert!(c.traveling());

    // A walk that ended by itself short of the ground still could not
    // get there.
    c.cancel_travel();
    assert!(!c.grow_hunt(now + WALK_ON_EVERY, &cfg));
    assert_eq!(c.autoplay.growth.bound, None);
    assert!(c.autoplay.growth.skip.iter().any(|(g, _)| *g == lb));
}

#[test]
fn a_walk_to_its_own_ground_is_let_go_once_it_follows() {
    // Going with the leader: a follower has no ground of its own to walk to. One it was bound for
    // goes when following begins, and one set while it follows (a town run ends with one) goes
    // when the hunting step next looks.
    let now = Instant::now();
    let mut c = crate::testkit::character_of_level(crate::testkit::no_data(), 20);
    crate::testkit::stand(&mut c, 0xA9B4_0019, glam::Vec3::new(84.0, 84.0, 94.0));
    c.autoplay.config.enabled = true;
    c.autoplay.config.growth.town_runs = false;
    let me = c.my_position().expect("on its feet");
    let ground = (
        0xA9B3_0000,
        Vec2::new(me.x, me.y - 200.0),
        "south".to_string(),
    );
    c.autoplay.growth.bound = Some(ground.clone());
    c.autoplay.growth.bound_since = Some(now);
    let team = &mut c.autoplay.config.team;
    team.enabled = true;
    team.follow = true;
    let mut leader = crate::testkit::mate(0x5000_0002, "Verity");
    leader.leader = true;
    leader.leads = true;
    leader.world = me + glam::Vec3::new(8.0, 0.0, 0.0);
    c.autoplay.team = crate::testkit::view_of(vec![leader]);
    c.tick_autoplay(now);
    assert_eq!(c.autoplay.step, Some("follow"));
    assert_eq!(
        c.autoplay.growth.bound, None,
        "the walk to its ground outlived following"
    );
    // Beside the leader, with nothing else to do, the hunting step looks.
    c.autoplay.team.mates[0].world = me + glam::Vec3::new(2.0, 0.0, 0.0);
    c.autoplay.growth.bound = Some(ground);
    c.tick_autoplay(now + Duration::from_millis(100));
    assert_eq!(
        c.autoplay.growth.bound, None,
        "a follower kept a ground walk of its own"
    );
    assert!(!c.traveling(), "it set off for a ground of its own");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_camp_nothing_comes_to_is_looked_about() {
    // The Blood Shrethlet ground in 0xA9B2 has thirty spawn entries, so it is held rather than
    // walked; held for good, a character stood at it doing nothing while none came.
    let block = 0xA9B2_0000;
    let ground = ac_world::hunting::at(block >> 16).expect("the Blood Shrethlet ground");
    assert_eq!(ground.suggested_tactic(), ac_world::hunting::Tactic::Camp);
    let outside = glam::Vec3::new(60.0, 60.0, 94.0);
    let mut c = standing_at(ac_world::outdoor_cell(block, outside), outside);
    c.world.stats.level = 7;
    c.autoplay.growth.hunting_at = Some(block >> 16);
    let cfg = Growth::default();
    let now = Instant::now();
    // Inside its quiet minute the spot is held.
    c.autoplay.growth.quiet_since = Some(now - Duration::from_secs(30));
    assert!(!c.grow_hunt(now, &cfg), "held for the quiet minute");
    assert!(!c.traveling());
    // Past it, nothing is coming: the ground is looked about.
    let later = now + Duration::from_secs(40);
    assert!(
        c.grow_hunt(later, &cfg),
        "stood on at a camp nothing came to"
    );
    assert!(c.traveling());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_camp_is_not_held_from_inside_a_building() {
    // The same ground's spot, the average of its spawn points, falls inside a building (cell
    // 0xA9B2011A): a character arriving there stood 134 s where no creature comes.
    let block = 0xA9B2_0000;
    let mut c = standing_at(0xA9B2_011A, glam::Vec3::new(74.0, 83.0, 94.0));
    c.world.stats.level = 7;
    c.autoplay.growth.hunting_at = Some(block >> 16);
    let cfg = Growth::default();
    let now = Instant::now();
    c.autoplay.growth.quiet_since = Some(now - Duration::from_secs(12));
    assert!(c.grow_hunt(now, &cfg), "stood in the building");
    assert!(c.traveling());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_character_off_any_ground_goes_to_one_at_once() {
    // After a town run four characters stood 40-47 s in Holtburg's shops before choosing a ground:
    // the quiet minute is for a ground going quiet, and nothing spawns in town to wait for.
    let mut c = standing_at(0xA9B4_0019, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    assert_eq!(c.autoplay.growth.hunting_at, None);
    let cfg = Growth::default();
    let now = Instant::now();
    c.autoplay.growth.quiet_since = Some(now - Duration::from_secs(1));
    assert!(c.grow_hunt(now, &cfg), "stood about in town");
    assert!(c.traveling());
}
