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
    assert!(
        c.is_walking() || c.traveling(),
        "the leader's patrol does not move"
    );
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
    assert!(
        c.grow_travel(ac_world::Place::on_map(at), crate::travel::ARRIVE),
        "no way to the ground"
    );
    c.autoplay.growth.bound = Some((lb, at, "Drudge".into()));

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
    assert!(
        c.is_walking() || c.traveling(),
        "the look about does not move"
    );
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

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_level_35_in_holtburg_sets_off_for_a_dungeon_and_keeps_moving() {
    // The easiest XP at 35 is a dungeon of Tuskers or Olthoi, 80-150 creatures at the step's XP,
    // not the five Armoredillos two kilometres out that the nearest ground was.
    let mut c =
        crate::testkit::standing_in_the_field(35, 0xA9B4_0019, glam::Vec3::new(84.0, 7.1, 94.0));
    let cfg = Growth::default();
    let t0 = Instant::now();
    // The Facility Hub's portal flags already earned (otherwise the Wardens come first).
    for u in ac_world::unlocks::earnable(35, &[]) {
        c.travel.quests_done.push(u.flag.clone());
    }
    c.autoplay.growth.quiet_since = Some(t0 - Duration::from_secs(1));
    assert!(c.grow_hunt(t0, &cfg), "stood about in town");
    let (lb, _, name) = c.autoplay.growth.bound.clone().expect("bound for a ground");
    let g = ac_world::hunting::at(lb).expect("a ground");
    println!(
        "bound for {name} {lb:#06x}: {:?}",
        c.travel_trip().map(|t| t.summary())
    );
    assert!(g.is_dungeon(), "{name} {lb:#06x} is open ground");
    assert!(g.max_level <= 35 + cfg.level_margin, "{g:?}");
    // And the journey is walked: never ten seconds in one place on the way to its first portal
    // (offline, nothing takes the character through it).
    let mouth = c
        .travel_trip()
        .and_then(|t| {
            t.steps.iter().find_map(|s| match s {
                ac_world::trip::Step::Portal { mouth, .. } => Some(mouth.at.truncate()),
                _ => None,
            })
        })
        .expect("a portal on the way");
    let dt = 0.05;
    let mut last = (c.my_position().unwrap(), t0);
    for frame in 0..90 * 20 {
        let now = t0 + Duration::from_secs_f32(frame as f32 * dt);
        c.tick_player(crate::player::Input::default(), dt, now);
        let at = c.my_position().unwrap();
        if at.truncate().distance(mouth) < 2.0 {
            return;
        }
        if at.distance(last.0) > 2.0 {
            last = (at, now);
        }
        assert!(
            now.duration_since(last.1) < Duration::from_secs(10),
            "stood at {at} for 10 s, the portal at {mouth}"
        );
    }
    panic!("not at the portal at {mouth} after 90 s");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_dungeon_ground_is_arrived_at_underground_and_walked_by_the_explorer() {
    // Tusker Cave, where its portal lands. Arriving is being in the dungeon, not near its spot
    // in world xy; and with it quiet the rooms are explored rather than a roam aimed at the
    // surface above.
    let cave = ac_world::hunting::at(0x5B46).expect("Tusker Cave");
    let e = cave.entrance.expect("a way in");
    let p = ac_world::portals::all()
        .iter()
        .find(|p| p.to_cell == e.cell)
        .expect("the portal");
    let o = ac_world::landblock_origin(e.cell);
    let mut c = crate::testkit::standing_in_the_field(30, e.cell, p.to - o);
    let cfg = Growth::default();
    let now = Instant::now();
    c.autoplay.growth.bound = Some((cave.landblock, cave.at, cave.name.clone()));
    assert!(c.underground(), "the landing is not in the dungeon");
    assert!(!c.grow_hunt(now, &cfg));
    assert_eq!(c.autoplay.growth.hunting_at, Some(0x5B46));
    // The explorer, a step above the ground's, walks the rooms.
    assert!(
        c.autoplay_explore(now),
        "the explorer does not walk Tusker Cave"
    );
    // And should the hunt rules be asked with it quiet past the patrol's ten seconds, no roam
    // aims at the surface over the cave.
    let mut c2 = crate::testkit::standing_in_the_field(30, e.cell, p.to - o);
    c2.autoplay.growth.hunting_at = Some(0x5B46);
    c2.autoplay.growth.quiet_since = Some(now - Duration::from_secs(30));
    c2.grow_hunt(now, &cfg);
    assert_ne!(
        c2.autoplay.status,
        "nothing in sight; looking about the ground"
    );
}

/// Autoplay and the body, ticked together for up to `seconds` until a journey is under way; the
/// notes said on the way.
fn play_noting_till_travelling(c: &mut Client, t0: Instant, seconds: u32) -> Vec<String> {
    let dt = 0.05;
    let mut notes = Vec::new();
    for frame in 0..seconds * 20 {
        let now = t0 + Duration::from_secs_f32(frame as f32 * dt);
        c.tick_autoplay(now);
        c.tick_player(crate::player::Input::default(), dt, now);
        for e in std::mem::take(&mut c.autoplay.announced) {
            if let crate::Event::Noted(n) = e {
                notes.push(n);
            }
        }
        if c.traveling() {
            break;
        }
    }
    notes
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_dungeon_crossed_on_the_way_to_a_ground_is_not_explored_in_its_place() {
    // Blargerton, bound for the Hea Hunters of the Tumerok Cave through Mountain Keep, was drawn
    // into a fight there; the journey ended, and the explorer, a step above the ground's, walked
    // Mountain Keep instead of taking the walk up again, round three rooms with no way into them.
    let mut c =
        crate::testkit::standing_in_the_field(35, 0x01D2_0309, glam::Vec3::new(57.5, -70.0, 12.0));
    c.autoplay.config.enabled = true;
    c.autoplay.config.fight.enabled = true;
    let cave = ac_world::hunting::at(0x0286).expect("the Tumerok Cave");
    let t0 = Instant::now();
    c.autoplay.growth.bound = Some((cave.landblock, cave.at, cave.name.clone()));
    // Until the walk is taken up: offline no portal takes the character on, and one reached
    // down the drops leaves it below with only the explorer's rooms to walk.
    let notes = play_noting_till_travelling(&mut c, t0, 30);
    let round = notes.iter().filter(|n| n.contains("no way into")).count();
    println!(
        "{notes:#?}\nstep {:?} status {:?} travelling {} bound {:?}",
        c.autoplay.step_name(),
        c.autoplay.status,
        c.traveling(),
        c.autoplay.growth.bound_landblock()
    );
    assert!(round <= 3, "went round Mountain Keep's rooms {round} times");
    assert!(c.traveling(), "the walk was never taken up again");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_dungeon_portal_below_its_landing_is_walked_down_to_by_its_drops() {
    // Mountain Keep's "Holtburg" portal, 18 m under its landing: no stairs lead down, the drops
    // off its ledges do. The journey takes it along that plan only, and the walk arrives.
    let mut c =
        crate::testkit::standing_in_the_field(35, 0x01D2_02F7, glam::Vec3::new(47.9, -69.9, 12.0));
    let cave = ac_world::hunting::at(0x0286).expect("the Tumerok Cave");
    // The clock the first walk is planned on: offline, planning takes seconds.
    let t0 = Instant::now();
    assert!(
        c.journey_to(cave.place(), crate::travel::ARRIVE),
        "no journey at all"
    );
    let o = ac_world::landblock_origin(0x01D2_0000);
    let inner = o + glam::Vec3::new(80.1, -57.5, -6.0);
    let first = c.travel_trip().and_then(|t| {
        t.steps.iter().find_map(|s| match s {
            ac_world::trip::Step::Portal { mouth, .. } => Some(mouth.at.truncate()),
            _ => None,
        })
    });
    assert!(
        first.is_some_and(|m| m.distance(inner.truncate()) < 3.0),
        "not bound for the portal below: {first:?}"
    );
    let dt = 0.05;
    let mut dropped = false;
    for frame in 0..30 * 20 {
        let now = t0 + Duration::from_secs_f32(frame as f32 * dt);
        c.tick_player(crate::player::Input::default(), dt, now);
        dropped |= c.steering.route.as_ref().is_some_and(|r| {
            r.waypoints
                .iter()
                .any(|w| w.kind == ac_scene::nav::EdgeKind::Drop)
        });
        let at = c.player.as_ref().unwrap().world_position();
        if at.distance(inner) < 1.5 {
            assert!(dropped, "came down by no planned drop");
            return;
        }
    }
    let at = c.player.as_ref().unwrap().world_position() - o;
    panic!("not at the portal below after 30 s: at {at}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_better_ground_further_off_is_chosen() {
    // A level 37 by the Hea dungeon's portal: the Tusker dungeons pay more, and a journey of
    // minutes against a stay of hours is not worth less XP an hour.
    let mut c = crate::testkit::standing_in_the_field(
        37,
        0x17B2_0038,
        glam::Vec3::new(4465.6 - 4416.0, 34254.7 - 34176.0, 0.0),
    );
    let cfg = Growth::default();
    let ranked = ac_world::hunting::ranked_for(37, cfg.level_margin, &[]);
    let best = c.best_ground(37, cfg.level_margin, &[]).expect("a ground");
    let reachable_better = ranked
        .iter()
        .take_while(|g| g.landblock != best.landblock)
        .map(|g| format!("{} {:#06x}", g.name, g.landblock))
        .collect::<Vec<_>>();
    println!(
        "chose {} {:#06x}; passed over {reachable_better:?}",
        best.name, best.landblock
    );
    assert_ne!(best.landblock, 0x0286, "the nearer Hea dungeon");
    assert!(best.worth() >= ranked[1].worth(), "{best:?}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_character_choosing_a_ground_goes_to_the_warden_its_level_earns_first() {
    let mut c = crate::quest_flags::tests::beside_the_warden(20);
    let cfg = Growth::default();
    let now = Instant::now();
    c.autoplay.growth.quiet_since = Some(now - std::time::Duration::from_secs(1));
    assert!(c.grow_hunt(now, &cfg));
    assert!(
        c.autoplay.status.contains("Warden"),
        "{}",
        c.autoplay.status
    );
    // Standing beside him, the visit is over in the frame it starts: he was talked to.
    assert!(
        c.visiting().is_some() || c.holds_quest_flag("fachubhuntersleapportal_flag"),
        "neither on the way to the Warden nor flagged"
    );
    // Every flag the level earns held: on to a ground.
    let mut c = crate::quest_flags::tests::beside_the_warden(20);
    for u in ac_world::unlocks::earnable(20, &[]) {
        c.travel.quests_done.push(u.flag.clone());
    }
    c.autoplay.growth.quiet_since = Some(now - std::time::Duration::from_secs(1));
    assert!(c.grow_hunt(now, &cfg));
    assert!(c.visiting().is_none());
    assert!(
        c.autoplay.growth.bound_landblock().is_some(),
        "{}",
        c.autoplay.status
    );
}
