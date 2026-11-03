use super::*;

/// A journey's goal: `place` to within [`ARRIVE`].
fn within_reach(place: Place) -> Goal {
    Goal::At {
        place,
        within: ARRIVE,
    }
}

/// A journey's goal on the map alone.
fn on_map(xy: Vec2) -> Goal {
    within_reach(Place::on_map(xy))
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn autoplays_walk_ends_when_autoplay_is_switched_off() {
    // Blargerton, autoplay switched off in the Holtburg Dungeon: the walk it had set stayed on
    // the body and walked him on after the recall, then stood 25 min on "no way".
    let now = Instant::now();
    let mut c = crate::testkit::standing_at(0xA9B4_0019, Vec3::new(84.0, 84.0, 94.0));
    c.world.player_guid = Some(crate::testkit::ME);
    c.autoplay.config.enabled = true;
    c.tick_autoplay(now);
    let me = c.my_position().unwrap();
    let walk = crate::Goal::At {
        place: c.place_at(me + Vec3::new(10.0, 0.0, 0.0)),
        within: 1.0,
    };
    c.go(crate::Owner::Script, walk, now);
    c.autoplay.config.enabled = false;
    c.tick_autoplay(now);
    assert!(c.going.is_none(), "walked on with autoplay off");
    // A walk set with it off is the player's or a script's, and stays.
    c.go(crate::Owner::Script, walk, now);
    c.tick_autoplay(now);
    assert_eq!(c.going.map(|g| g.goal), Some(walk));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_walk_left_behind_by_a_recall_is_let_go() {
    let now = Instant::now();
    let mut c = crate::testkit::standing_at(0xA9B4_0019, Vec3::new(84.0, 84.0, 94.0));
    let me = c.my_position().unwrap();
    let near = crate::Goal::At {
        place: c.place_at(me + Vec3::new(30.0, 0.0, 0.0)),
        within: 1.0,
    };
    c.go(crate::Owner::Hand, near, now);
    c.going_to();
    assert_eq!(
        c.going.as_ref().map(|g| g.goal),
        Some(near),
        "a walk in reach is kept"
    );
    // The dungeon spot the recall left behind, 34 km from the Holtburg lifestone.
    let walk = c.going.as_mut().expect("walking");
    walk.goal = crate::Goal::At {
        place: ac_world::Place {
            at: Vec3::new(264.5, 47182.0, 0.1),
            cell: 0x01F6_0000,
        },
        within: 1.0,
    };
    c.going_to();
    assert!(c.going.is_none());
}

#[test]
fn a_building_is_planned_from_the_ground_outside_it() {
    let upstairs = 0xA9B4_011B;
    let local = Vec3::new(152.3, 132.5, 69.0);
    let at = ac_world::landblock_origin(upstairs) + local;
    let cell = planning_cell(upstairs, at, false);
    assert_eq!(cell, ac_world::outdoor_cell(upstairs, local));
    assert!(cell & 0xFFFF < 0x100, "{cell:#x}");
    // Underground, or already outdoors, the cell stands.
    assert_eq!(planning_cell(upstairs, at, true), upstairs);
    assert_eq!(planning_cell(0xA9B4_002E, at, false), 0xA9B4_002E);
}

#[test]
fn the_dungeon_by_holtburg_is_a_walk_from_the_archmage() {
    let upstairs = 0xA9B4_011B;
    let at = ac_world::landblock_origin(upstairs) + Vec3::new(152.3, 132.5, 69.0);
    let dungeon =
        (ac_world::landblock_origin(0xA8B5_0000) + Vec3::new(126.6, 173.4, 0.0)).truncate();
    let walks = |cell| {
        let from = Place { at, cell };
        trip::plan_with(from, Place::on_map(dungeon), 0, &[], &[], Prefs::quick())
            .is_some_and(|t| t.portals() == 0)
    };
    // Planned from the shop's own cell, the door was not a way out.
    assert!(!walks(upstairs));
    assert!(walks(planning_cell(upstairs, at, false)));
}

#[test]
fn a_corpse_in_the_holtburg_dungeon_is_reached_through_its_portal() {
    // Died in the armoredillo rooms, up again at the Holtburg
    // lifestone: the way back is the dungeon's portal.
    let lifestone = ac_world::landblock_origin(0xA9B4_0019) + Vec3::new(84.0, 7.1, 94.0);
    let corpse = Vec2::new(210.0, 47185.0);
    // Its position alone reads as the landblock next door (the rooms
    // reach below the dungeon's square); its cell says where it is.
    let trip = trip::plan_with_recalls_and_gems(
        Place {
            at: lifestone,
            cell: 0xA9B4_0019,
        },
        Place {
            at: corpse.extend(0.0),
            cell: 0x01F6_0215,
        },
        5,
        &[],
        &[],
        &[],
        &[],
        &[],
        Prefs::quick(),
    );
    assert!(trip.as_ref().is_some_and(|t| t.portals() == 1), "{trip:?}");
}

/// Walk the journey the client is on for up to `seconds` of game time, frame by frame the way
/// `tick_player` does: the stretch asked of the slot, the steering along it, the player's own
/// physics. Stops when the journey ends; the last answer the slot gave its stretch. The clock
/// starts at `Client.frame_at`, which the test sets as it sets off: offline, planning takes seconds.
fn walk_the_journey(c: &mut Client, seconds: f32) -> Option<crate::did::Did> {
    walk_watching(c, seconds, |_, _| {})
}

/// [`walk_the_journey`], `each` shown the client after the slot is asked, every frame.
fn walk_watching(
    c: &mut Client,
    seconds: f32,
    mut each: impl FnMut(&mut Client, Instant),
) -> Option<crate::did::Did> {
    use crate::player::Input;
    let dt = 1.0 / 20.0;
    let t0 = c.frame_at;
    let mut last = None;
    for frame in 0..(seconds / dt) as u32 {
        let now = t0 + Duration::from_secs_f32(frame as f32 * dt);
        if !c.traveling() {
            return last;
        }
        last = c.walk_journey(now).or(last);
        each(c, now);
        let goal = c.going_to();
        let pl = c.player.as_mut().unwrap();
        let mut input = Input::default();
        pl.step_cap = None;
        match goal {
            Some((g, stop, cell)) => {
                if !crate::body::server_walk::reached(pl.world_position(), g, stop) {
                    let mut standing = crate::Standing {
                        player: pl,
                        pathfinder: &mut c.pathfinder,
                    };
                    let to = ac_world::Place { at: g, cell };
                    let aim = c.steering.steer(&mut standing, to, stop, now);
                    if let ac_nav::Aim::Go(at) = aim {
                        let d = at - pl.world_position();
                        let flat = Vec2::new(d.x, d.y);
                        if flat.length() > 1e-3 {
                            pl.heading = (-flat.x).atan2(flat.y);
                        }
                        pl.step_cap = Some(flat.length());
                        pl.pace_cap = c.steering.pace();
                        input.forward = 1.0;
                        input.run = true;
                    }
                }
            }
            None => c.steering.reset(),
        }
        pl.update(&c.assets, &input, dt);
    }
    last
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_lifestone_known_is_the_way_to_a_far_counter() {
    // Rawrtherton, south of Holtburg at 0xA87C0006 with an Arcanum Broker to sell to, walked
    // 8.5 km for 33 minutes: he had logged in, and the client had not kept where his
    // lifestone was. With it known, /lifestone and the Town Network make it a short trip.
    let mut c =
        crate::testkit::standing_in_the_field(31, 0xA87C_0006, Vec3::new(23.8, 140.4, 26.3));
    c.world.player_guid = Some(crate::testkit::ME);
    let holtburg = ac_world::landmarks::nearest_lifestone(Vec2::new(32532.0, 34567.0))
        .expect("Holtburg's lifestone");
    let at = holtburg.at;
    c.world.stats.set_recall_position(
        ac_world::recalls::position_type::SANCTUARY,
        ac_world::object::Position::new_flat(
            holtburg.cell,
            at - ac_world::landblock_origin(holtburg.cell),
        ),
    );
    assert!(c.travel_to(Vec2::new(36064.7, 30639.0)));
    let trip = c.travel.trip.as_ref().expect("a trip");
    let first = format!("{:?}", trip.steps.first());
    assert!(first.contains("/lifestone"), "first step {first}");
    assert!(trip.seconds < 600.0, "{:.0} s", trip.seconds);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_facility_hub_is_walked_down_to_its_lower_portals() {
    // Blargerton, on his way to the Tusker Barracks, landed in the Facility Hub and stood: the
    // berths of the portals on its upper floor blanked out the corridors under them, and the
    // graph had no way down to the Tusker portals 18 m below.
    let o = ac_world::landblock_origin(0x8A02_0000);
    let landed = Vec3::new(26558.4, 298.0, 6.0);
    let mut c = crate::testkit::standing_in_the_field(31, 0x8A02_0212, landed - o);
    c.world.player_guid = Some(crate::testkit::ME);
    let t0 = Instant::now();
    assert!(c.travel_to(Vec2::new(46257.1, 24399.7)));
    let dt = 0.05;
    let mut first = None;
    for frame in 0..30 * 20 {
        let now = t0 + Duration::from_secs_f32(frame as f32 * dt);
        let goal = c.step_goal(now).and_then(Goal::place).map(|p| p.at);
        first = first.or(goal);
        c.tick_player(crate::player::Input::default(), dt, now);
        let at = c.player.as_ref().unwrap().world_position();
        if goal == first && first.is_some_and(|g| at.distance(g) < 1.5) {
            return;
        }
    }
    let at = c.player.as_ref().unwrap().world_position() - o;
    panic!(
        "not at the first portal after 30 s: at {at}, it is at {:?}",
        first.map(|g| g - o)
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_stairs_up_to_a_scrivener_are_walked_down_too() {
    // Blargerton, upstairs by the Scrivener of Creature Magic in Holtburg, set off for Boddry's
    // counter and stood: the graph had the stairs up and none down, and the journey's first two
    // points lie on the block line, which the steering did not plan to.
    let mut c =
        crate::testkit::standing_in_the_field(30, 0xA9B3_0108, Vec3::new(185.5, 88.3, 119.6));
    c.world.player_guid = Some(crate::testkit::ME);
    let boddry = Vec2::new(32538.6, 34266.0);
    let t0 = Instant::now();
    assert!(c.travel_to(boddry));
    let dt = 0.05;
    for frame in 0..60 * 20 {
        let now = t0 + Duration::from_secs_f32(frame as f32 * dt);
        c.tick_player(crate::player::Input::default(), dt, now);
        let at = c.player.as_ref().unwrap().world_position();
        if frame == 10 * 20 {
            assert!(at.z < 118.0, "still on the upper floor after 10 s at {at}");
        }
    }
    let at = c.player.as_ref().unwrap().world_position().truncate();
    assert!(
        at.distance(boddry) <= 2.0 * ARRIVE,
        "stopped {:.0} m short",
        at.distance(boddry)
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_walk_out_of_a_shop_to_a_spot_outside_the_door_reaches_the_next_counter() {
    // Inside Archmage Cindrue's shop, where a scenario's mage stood four minutes: from indoors
    // the block's graph finds no way to Shopkeeper Renald 112 m off, but it does to a spot
    // outside the door, and from there the journey on is an ordinary walk.
    let mut c =
        crate::testkit::standing_in_the_field(20, 0xA9B4_0117, Vec3::new(151.8, 129.8, 66.0));
    let renald = Vec2::new(32587.2, 34578.3);
    c.frame_at = Instant::now();
    assert!(
        c.travel_to(Vec2::new(32588.0, 34691.0)),
        "8 m out, on the way in"
    );
    walk_the_journey(&mut c, 20.0);
    let cell = c.player.as_ref().unwrap().cell;
    assert!(cell & 0xFFFF < 0x100, "outdoors, not {cell:#010x}");
    c.end_trip();
    c.frame_at = Instant::now();
    assert!(c.travel_to(renald));
    walk_the_journey(&mut c, 90.0);
    let at = c.player.as_ref().unwrap().world_position().truncate();
    assert!(
        at.distance(renald) <= 2.0 * ARRIVE,
        "stopped {:.0} m short",
        at.distance(renald)
    );
}

/// Walk `c` at `target` for `seconds`, steered or straight, as `tick_player` does.
fn drive(c: &mut Client, target: Vec3, block: u32, steer: bool, seconds: f32) {
    use crate::player::Input;
    let dt = 1.0 / 20.0;
    let stop = 0.5;
    let t0 = Instant::now();
    for frame in 0..(seconds / dt) as u32 {
        let now = t0 + Duration::from_secs_f32(frame as f32 * dt);
        let pl = c.player.as_mut().unwrap();
        let aim = if steer {
            let mut standing = crate::Standing {
                player: pl,
                pathfinder: &mut c.pathfinder,
            };
            let to = ac_world::Place {
                at: target,
                cell: block,
            };
            c.steering.steer(&mut standing, to, stop, now)
        } else {
            ac_nav::Aim::Go(target)
        };
        // As `tick_player` does: stop at the goal, and otherwise walk at the aim however near.
        let mut input = Input::default();
        let there = Vec2::new(
            target.x - pl.world_position().x,
            target.y - pl.world_position().y,
        )
        .length()
            <= stop;
        if let (ac_nav::Aim::Go(at), false) = (aim, there) {
            let d = at - pl.world_position();
            let flat = Vec2::new(d.x, d.y);
            if flat.length() > 1e-3 {
                pl.heading = (-flat.x).atan2(flat.y);
            }
            pl.step_cap = Some(flat.length());
            pl.pace_cap = if steer { c.steering.pace() } else { None };
            input.forward = 1.0;
            input.run = true;
        }
        pl.update(&c.assets, &input, dt);
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_character_in_a_pocket_among_props_is_planned_out_of_it() {
    // Two pockets among props, at the Mosswart ground and beside the Academy spawn: the body
    // walked in, so a walk-tested start exists and the plan from inside leads out.
    for (block, cell, from, pocket, goal) in [
        (
            0xBAAD_0000u32,
            0xBAAD_0017u32,
            Vec3::new(58.9, 156.0, 88.9),
            Vec3::new(61.4, 148.8, 88.4),
            Vec3::new(66.1, 131.1, 88.0),
        ),
        (
            0x8602_0000,
            0x8602_01AD,
            Vec3::new(12.3, -28.5, 0.0),
            Vec3::new(14.5, -25.5, 0.0),
            Vec3::new(22.1, -19.1, 0.0),
        ),
    ] {
        let origin = ac_world::landblock_origin(block);
        let mut c = crate::testkit::standing_in_the_field(20, cell, from);
        drive(&mut c, origin + pocket, block, false, 4.0);
        c.steering.reset();
        let to = ac_world::Place {
            at: origin + goal,
            cell: block,
        };
        let plan = c.plan_walk(to, 0.5);
        assert!(
            matches!(plan, ac_nav::routing::Planned::Route(_)),
            "no plan out of the pocket in {block:#010x}: {plan:?}"
        );
        drive(&mut c, origin + goal, block, true, 30.0);
        let at = c.player.as_ref().unwrap().world_position() - origin;
        assert!(
            at.truncate().distance(goal.truncate()) <= 2.0,
            "stuck at {at:?} in {block:#010x}, short of {goal:?}"
        );
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_character_against_a_bookcase_walks_round_it() {
    // Blargerton in the Holtburg Dungeon, squeezed between a row of bookcases and a chest
    // 0.75 m apart, stood 12 min: the graph nodes nearest him were snapped out to the far sides
    // of the two, so every plan began behind the bookcases and the body never got there.
    let block = 0x01F6_0000u32;
    let origin = ac_world::landblock_origin(block);
    let from = Vec3::new(35.3157, -51.0781, 0.0);
    let corpse = Vec3::new(30.3719, -51.8906, 0.0042);
    let mut c = crate::testkit::standing_in_the_field(25, 0x01F6_0233, from);
    let to = ac_world::Place {
        at: origin + corpse,
        cell: block,
    };
    let path = match c.plan_walk(to, 0.5) {
        ac_nav::routing::Planned::Route(r) => r.points(),
        other => panic!("no way round: {other:?}"),
    };
    assert!(
        path[0].x - origin.x > 34.87,
        "starts behind the bookcases: {path:?}"
    );
    drive(&mut c, origin + corpse, block, true, 30.0);
    let at = c.player.as_ref().unwrap().world_position() - origin;
    assert!(
        at.truncate().distance(corpse.truncate()) <= 1.0,
        "stuck at {at:?}, short of {corpse:?}"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_planner_answers_from_the_region() {
    // "If there's no path to the corpse, don't bother": asked once as the walk sets off. In the
    // Holtburg Dungeon, beside the bookcases, the room round them is reachable and the top of a
    // bookcase is not.
    let block = 0x01F6_0000u32;
    let origin = ac_world::landblock_origin(block);
    let mut c =
        crate::testkit::standing_in_the_field(25, 0x01F6_0224, Vec3::new(30.37, -51.89, 0.0));
    let round = Place {
        at: origin + Vec3::new(36.0, -54.0, 0.0),
        cell: 0x01F6_0233,
    };
    let planned = c.plan_walk(round, 0.25);
    assert!(
        matches!(planned, ac_nav::routing::Planned::Route(_)),
        "round the bookcases: {planned:?}"
    );
    // On top of a bookcase, 1.9 m up: the planner checks the last leg, so no route ends there.
    let on_top = Place {
        at: origin + Vec3::new(34.68, -50.7, 1.915),
        cell: 0x01F6_0224,
    };
    let planned = c.plan_walk(on_top, 2.5);
    assert!(
        matches!(planned, ac_nav::routing::Planned::NoWay(_)),
        "{planned:?}"
    );
    // Another block is past the region: the journey planner's.
    let far = Place {
        at: Vec3::new(10.0, 10.0, 0.0),
        cell: 0x0101_0001,
    };
    let planned = c.plan_walk(far, 0.25);
    assert_eq!(
        planned,
        ac_nav::routing::Planned::NoWay(ac_nav::routing::NoWay::OutOfRegion)
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_mine_is_walked_down_to_its_floor() {
    // The mine at ACB5: a door at 60 m, stairs, a ramp, and a floor at 40.4 where Small
    // Fledgling Mukkirs live. The graph found no way in or out (an overlay cell's floor over
    // the ramp, a door frame and a landing's lip the body slides past), so casters stood on
    // the hill above them "getting them in sight" and a seller stood 390 s inside.
    let block = 0xACB5_0000u32;
    let origin = ac_world::landblock_origin(block);
    let outside = Vec3::new(103.0, 162.5, 60.0);
    let bottom = Vec3::new(125.5, 134.1, 40.4);
    let mut c = crate::testkit::standing_in_the_field(21, 0xACB5_0027, outside);
    drive(&mut c, origin + bottom, 0xACB5_01E2, true, 60.0);
    let at = c.player.as_ref().unwrap().world_position() - origin;
    assert!(
        at.distance(bottom) <= 1.5,
        "stopped at {at:?}, short of the mine's floor"
    );
    c.steering.reset();
    drive(&mut c, origin + outside, block, true, 60.0);
    let at = c.player.as_ref().unwrap().world_position() - origin;
    assert!(
        at.truncate().distance(outside.truncate()) <= 2.0,
        "stopped at {at:?} on the way out"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_goal_on_a_roof_is_walked_to_on_the_roof() {
    // A Mite Sentry on the roof of a building in 0xBDAF, 4.3 m over a caster inside. The goal
    // outdoors and far above the terrain read as floating off a hillside and was dropped to the
    // ground, so the walk ended under the creature; the way up is a ramp round the back.
    let block = 0xBDAF_0000u32;
    let origin = ac_world::landblock_origin(block);
    let mut c =
        crate::testkit::standing_in_the_field(20, 0xBDAF_0103, Vec3::new(181.0, 129.6, 92.1));
    let roof = origin + Vec3::new(181.0, 129.6, 96.4);
    let me = c.my_position().unwrap();
    let assets = c.assets.clone();
    let path = match c.plan_walk(
        ac_world::Place {
            at: roof,
            cell: block,
        },
        0.5,
    ) {
        ac_nav::routing::Planned::Route(r) => r.points(),
        other => panic!("no way up: {other:?}"),
    };
    let shown = path.iter().map(|w| *w - origin).collect::<Vec<_>>();
    let end = *path.last().unwrap();
    assert!(
        end.distance(roof) <= 0.5,
        "climbs to the roof rather than stopping under it: {shown:?}"
    );
    // And the body walks every leg of the way up, the ramp included.
    let pl = c.player.as_mut().unwrap();
    let cap = pl.capsule();
    let walked = pl.on_ground(&assets, block, |ground, _| {
        let mut from = (me, 0xBDAF_0103);
        path.iter().all(|&p| {
            let walks = ground.body_reaches(from.0, from.1, p, &cap);
            from = (p, ac_scene::wayfind::locate(ground, p));
            walks
        })
    });
    assert_eq!(walked, Some(true), "{shown:?}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_ground_reached_through_a_one_way_portal_has_a_way_home_on_foot() {
    // Where a scenario's caster ran out of tapers: the Mosswart ground, reached from Holtburg
    // through a portal with none back, 3.5 km from town and 2.5 km from Magus Guthima.
    let mut c =
        crate::testkit::standing_in_the_field(20, 0xBAAD_0017, Vec3::new(65.4, 144.0, 88.0));
    for goal in [Vec2::new(32532.0, 34567.1), Vec2::new(38076.7, 32340.6)] {
        c.travel = Default::default();
        assert!(set_off(&mut c, on_map(goal)).fine(), "no way to {goal:?}");
        let steps = c.travel_trip().map(|t| t.steps.clone());
        assert_eq!(
            steps,
            Some(vec![Step::Walk(Place::on_map(goal))]),
            "the whole way on foot"
        );
        assert!(
            c.travel_route().is_some_and(|r| r.len() > 2),
            "routed over the terrain"
        );
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_corpse_in_the_holtburg_dungeon_is_walked_to_from_its_portal() {
    let assets = crate::testkit::game_data();
    let rooms = assets.block_collision(0x01F6_0000).unwrap();
    let mut c = Client::offline(assets.clone());
    // Both corpses recovery walked at from the arrival rooms, on the
    // local server, and never reached.
    for corpse in [
        Vec2::new(220.832_12, 47_190.047),
        Vec2::new(239.048_95, 47_221.76),
    ] {
        let (floor, cell) = rooms
            .world
            .floor_at(corpse.extend(10.0), 10.0, 60.0)
            .expect("the corpse lies on a floor of the dungeon");
        eprintln!("corpse {corpse:?}: floor {floor:.1} in {cell:#010x}");
        // Where the Holtburg Dungeon's portal drops a character.
        crate::testkit::stand(&mut c, 0x01F6_0289, Vec3::new(96.7, -10.0, 0.0));
        c.steering.reset();
        let place = Place {
            at: corpse.extend(floor),
            cell,
        };
        assert!(
            set_off(&mut c, within_reach(place)).fine(),
            "no way to {corpse:?}"
        );
        assert_eq!(
            c.travel_trip().map(|t| t.steps.clone()),
            Some(vec![Step::Walk(place)])
        );
        walk_the_journey(&mut c, 180.0);
        let pl = c.player.as_ref().unwrap();
        let at = pl.world_position();
        let origin = ac_world::landblock_origin(pl.cell);
        assert!(
            at.truncate().distance(corpse) <= 2.0 * ARRIVE,
            "stopped at {:?} in {:#010x}, {:.0} m from the corpse at {corpse:?} \
             (still travelling {})",
            at - origin,
            pl.cell,
            at.truncate().distance(corpse),
            c.traveling()
        );
        c.end_trip();
    }
}

/// Where Blargerton stood in 0x0286 on Coldeve, and his corpse on the floor below.
fn below_in_0286(c: &mut Client) -> Place {
    let rooms = c.assets.block_collision(0x0286_0000).unwrap();
    let origin = ac_world::landblock_origin(0x0286_0000);
    let start = Vec2::new(419.98, 25_673.98);
    let corpse = Vec2::new(386.725, 25_627.996);
    let (z, cell) = rooms
        .world
        .floor_at(start.extend(10.0), 10.0, 60.0)
        .unwrap();
    let below = rooms.world.floor_at(corpse.extend(z), 0.5, 60.0).unwrap();
    assert!(below.0 < z - 3.0, "a floor below: {below:?}");
    crate::testkit::stand(c, cell, (start - origin.truncate()).extend(z - origin.z));
    c.steering.reset();
    Place {
        at: corpse.extend(below.0),
        cell: below.1,
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_corpse_on_a_floor_below_in_a_dungeon_is_walked_to() {
    let mut c = Client::offline(crate::testkit::game_data());
    let below = below_in_0286(&mut c);
    let corpse = below.at.truncate();
    assert!(
        set_off(&mut c, within_reach(below)).fine(),
        "no way to the corpse"
    );
    walk_the_journey(&mut c, 180.0);
    let at = c.player.as_ref().unwrap().world_position().truncate();
    assert!(
        at.distance(corpse) <= 2.0 * ARRIVE,
        "{:.1} m short",
        at.distance(corpse)
    );
}

/// A Holtburg Dungeon room, where `c` is set down, and the ledge 9.5 m over it that nothing
/// climbs to: where it stands, and the ledge's place.
fn under_the_01f6_ledge(c: &mut Client) -> (Vec3, Place) {
    let rooms = c.assets.block_collision(0x01F6_0000).unwrap();
    let origin = ac_world::landblock_origin(0x01F6_0000);
    let start = Vec3::new(223.5, 47_158.5, 0.0);
    let (z, cell) = rooms.world.floor_at(start, 0.6, 1.5).unwrap();
    let ledge = Vec3::new(262.0, 47_182.0, 9.5);
    let (high, ledge_cell) = rooms.world.floor_at(ledge, 0.6, 1.5).unwrap();
    crate::testkit::stand(c, cell, start.with_z(z) - origin);
    c.steering.reset();
    let place = Place {
        at: ledge.with_z(high),
        cell: ledge_cell,
    };
    (start.with_z(z), place)
}

/// [`Client::set_off`] as the hand would, the frame's clock set to now.
fn set_off(c: &mut Client, goal: Goal) -> crate::did::Did {
    c.frame_at = Instant::now();
    c.set_off(crate::Owner::Hand, goal, c.frame_at)
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_journey_whose_first_walk_has_no_way_is_refused_at_once() {
    // From the room to the ledge: the first walk is planned as the journey sets off, and refused.
    let mut c = Client::offline(crate::testkit::game_data());
    let (start, ledge) = under_the_01f6_ledge(&mut c);
    let did = set_off(&mut c, within_reach(ledge));
    let why = did.because().map(|b| b.what.clone());
    assert!(
        why.as_ref().is_some_and(|w| w.contains("Planner")),
        "{did:?}"
    );
    assert!(!c.traveling(), "still travelling");
    assert!(c.going.is_none(), "a walk was left in the slot");
    assert!(!c.journey_to(ledge, ARRIVE), "set off the second time");
    let me = c.my_position().unwrap();
    assert!(me.distance(start) < 0.01, "walked to {me}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_journey_that_loses_a_step_is_planned_again_without_it() {
    // Handed a trip whose next portal stands on the ledge, as if just arrived in the room: the
    // mouth is refused on the first frame, the trip planned again has no step at it, and the
    // journey arrives at its goal.
    let mut c = Client::offline(crate::testkit::game_data());
    let (start, ledge) = under_the_01f6_ledge(&mut c);
    let goal = c
        .standable_near(start, 10.0)
        .expect("built offline")
        .into_iter()
        .find(|p| p.at.distance(start) > 6.0)
        .expect("floor round the start");
    let trip = Trip {
        steps: vec![
            Step::Portal {
                name: "a portal on the ledge".into(),
                mouth: ledge,
                exit: Place::on_map(Vec2::new(32_532.0, 34_567.1)),
            },
            Step::Walk(goal),
        ],
        seconds: 60.0,
    };
    crate::testkit::set_out_on(&mut c, trip, within_reach(goal));
    c.frame_at = Instant::now();
    let first = c.walk_journey(c.frame_at);
    assert!(
        matches!(first, Some(crate::did::Did::Blocked(_))),
        "{first:?}"
    );
    let again = c.walk_journey(c.frame_at);
    assert!(
        again.as_ref().is_some_and(crate::did::Did::fine),
        "{again:?}"
    );
    let steps = c.travel_trip().map(|t| t.steps.clone());
    assert_eq!(steps, Some(vec![Step::Walk(goal)]), "planned again");
    walk_the_journey(&mut c, 30.0);
    let me = c.my_position().unwrap();
    assert!(!c.traveling(), "still travelling at {me}");
    assert!(
        me.distance(goal.at) <= ARRIVE,
        "stopped at {me}, short of {goal:?}"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_walk_with_no_way_after_a_hop_is_left_out_and_the_journey_planned_again() {
    // As if just come into the Holtburg Dungeon room by a portal, the ledge the trip's last walk:
    // the walk is refused on the first frame and the journey planned again from here without it
    // (refused when nothing else reaches the ledge), never walked from here again.
    let mut c = Client::offline(crate::testkit::game_data());
    let (start, ledge) = under_the_01f6_ledge(&mut c);
    let trip = Trip {
        steps: vec![Step::Walk(ledge)],
        seconds: 10.0,
    };
    crate::testkit::set_out_on(&mut c, trip, within_reach(ledge));
    c.travel.hopped = true;
    c.frame_at = Instant::now();
    let first = c.walk_journey(c.frame_at);
    assert!(
        matches!(first, Some(crate::did::Did::Blocked(_))),
        "{first:?}"
    );
    assert!(c.traveling(), "refused, not planned again");
    assert_eq!(c.travel.refused_walks.len(), 1);
    let again = c.walk_journey(c.frame_at);
    let me = c.my_position().expect("on its feet");
    assert!(me.distance(start) < 0.01, "walked to {me}");
    match c.travel_trip() {
        Some(t) => assert_ne!(t.steps.first(), Some(&Step::Walk(ledge)), "{t:?}"),
        None => assert!(
            matches!(again, Some(crate::did::Did::Blocked(_))),
            "{again:?}"
        ),
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_journey_whose_end_stands_on_no_floor_is_refused_not_planned_again() {
    // A goal its own cell holds no floor under (Ketnan's counter, 0xF784039F, lies two blocks off
    // its block): every way there ends in that one walk, so after a hop it refuses the journey.
    let mut c = Client::offline(crate::testkit::game_data());
    let (start, ledge) = under_the_01f6_ledge(&mut c);
    let nowhere = Place {
        at: ledge.at - Vec3::new(0.0, 0.0, 5.0),
        cell: ledge.cell,
    };
    let trip = Trip {
        steps: vec![Step::Walk(nowhere)],
        seconds: 10.0,
    };
    crate::testkit::set_out_on(&mut c, trip, within_reach(nowhere));
    c.travel.hopped = true;
    c.frame_at = Instant::now();
    let first = c.walk_journey(c.frame_at);
    assert!(
        matches!(first, Some(crate::did::Did::Blocked(_))),
        "{first:?}"
    );
    assert!(
        !c.traveling(),
        "planned again round a goal no floor holds: {first:?}"
    );
    let me = c.my_position().expect("on its feet");
    assert!(me.distance(start) < 0.01, "walked to {me}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_portal_in_a_hall_is_walked_to_on_its_own_floor() {
    // From Archmage Cindrue's first floor at 69, the Holtburg Meeting Hall Portal's mouth at 96:
    // the stretch is the mouth on its own floor and in its own cell.
    let hall = ac_world::portals::named("Holtburg Meeting Hall Portal")
        .into_iter()
        .find(|p| p.from_cell == 0xA9B4_017A)
        .expect("the Holtburg meeting hall portal is in the data");
    let mut c =
        crate::testkit::standing_in_the_field(20, 0xA9B4_011B, Vec3::new(152.3, 132.5, 69.0));
    let exit = Place {
        at: hall.to,
        cell: hall.to_cell,
    };
    assert!(c.journey_to(exit, ARRIVE), "no way through the hall");
    let first = c.travel_trip().and_then(|t| t.steps.first().cloned());
    assert!(matches!(first, Some(Step::Portal { .. })), "{first:?}");
    let stretch = c.going.as_ref().and_then(|g| g.goal.place());
    let stretch = stretch.expect("no stretch in the slot");
    assert_eq!(stretch.cell, 0xA9B4_017A);
    assert!((stretch.at.z - 96.0).abs() < 0.01, "{stretch:?}");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_journey_to_an_indoor_goal_arrives_in_its_cell() {
    // From the A9B2 platform to Archmage Cindrue upstairs (0xA9B4011B), 440 m: a corridor
    // stretch, then into the house and up the stairs, ending in that cell on that floor.
    let local = Vec3::new(78.2, 85.9, 97.5);
    let cell = ac_world::outdoor_cell(0xA9B2_0000, local);
    let mut c = crate::testkit::standing_in_the_field(20, cell, local);
    let upstairs = Place {
        at: ac_world::landblock_origin(0xA9B4_011B) + Vec3::new(152.3, 132.5, 69.0),
        cell: 0xA9B4_011B,
    };
    assert!(
        set_off(&mut c, within_reach(upstairs)).fine(),
        "no way upstairs"
    );
    assert!(c.travel_route().is_some(), "not walked by a corridor");
    walk_the_journey(&mut c, 120.0);
    let pl = c.player.as_ref().unwrap();
    let me = pl.world_position();
    assert!(!c.traveling(), "still travelling at {me}");
    assert_eq!(pl.cell, 0xA9B4_011B, "ended at {me}");
    let d = me - upstairs.at;
    assert!(
        d.truncate().length() <= ARRIVE && d.z.abs() <= crate::SAME_FLOOR,
        "{me}"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn an_overland_walk_is_planned_a_region_at_a_time() {
    // The Mosswart ground toward Holtburg, 3.5 km on foot: each stretch ends inside the blocks
    // round where it was asked, is routed before a step, and the body keeps moving.
    let mut c =
        crate::testkit::standing_in_the_field(20, 0xBAAD_0017, Vec3::new(65.4, 144.0, 88.0));
    let holtburg = Vec2::new(32_532.0, 34_567.1);
    assert!(set_off(&mut c, on_map(holtburg)).fine(), "no way home");
    let mut stretches = Vec::new();
    let mut at = Vec::new();
    let mut last = None;
    walk_watching(&mut c, 90.0, |c, now| {
        let me = c.my_position().unwrap();
        let stretch = c.going.as_ref().and_then(|g| g.goal.place());
        if let Some(s) = stretch.filter(|_| stretch != last) {
            let (block, there) = (
                (me.truncate() / 192.0).floor(),
                (s.at.truncate() / 192.0).floor(),
            );
            assert!(
                (block - there).abs().max_element() <= 1.0,
                "{s:?} from {me}"
            );
            assert!(
                c.steering.remaining(me).is_some(),
                "not routed to {s:?} from {me}"
            );
            stretches.push(s);
        }
        last = stretch;
        if at
            .last()
            .is_none_or(|(t, _)| now.duration_since(*t) >= Duration::from_secs(10))
        {
            at.push((now, me));
        }
    });
    assert!(stretches.len() >= 2, "{stretches:?}");
    let moved: Vec<f32> = at.windows(2).map(|w| w[0].1.distance(w[1].1)).collect();
    assert!(moved.iter().all(|m| *m > 20.0), "stood: {moved:?}");
}

/// A level-20 follower in the Holtburg field on its journey after a leader 400 m east, the
/// journey then ended by a refusal whose replan found no way: the goal is left, no trip.
fn after_a_failed_replan() -> (Client, Vec2, Instant) {
    let mut c = crate::testkit::character_of_level(crate::testkit::no_data(), 20);
    crate::testkit::stand(&mut c, 0xA9B4_0019, Vec3::new(84.0, 84.0, 94.0));
    c.autoplay.config.enabled = true;
    let team = &mut c.autoplay.config.team;
    team.enabled = true;
    team.follow = true;
    let me = c.my_position().expect("on its feet");
    let mut leader = crate::testkit::mate(0x5000_0002, "Verity");
    leader.leader = true;
    leader.leads = true;
    leader.cell = 0xA9B4_0019;
    leader.world = me + Vec3::new(400.0, 0.0, 0.0);
    c.autoplay.team = crate::testkit::view_of(vec![leader]);
    let now = Instant::now();
    c.tick_autoplay(now);
    assert!(c.is_follow_journey(), "following planned no journey");
    let goal = c.travel_goal_xy().expect("a goal");
    c.cancel_travel_keeping_refusals();
    assert!(!c.traveling());
    assert_eq!(c.travel_goal_xy(), Some(goal));
    (c, goal, now)
}

/// Kill `c` at `now`, as the recovery sees a death.
fn die(c: &mut Client, now: Instant) {
    c.world.stats.name = "Bryn".into();
    c.world.stats.attributes[1].base = 100;
    c.world.stats.vitals[0].current = 0;
    c.tick_autoplay(now);
    assert_eq!(c.autoplay.step, Some("recover"));
}

/// Up at the lifestone with no death spot to go back to, at `t`: the recovery finishes.
fn back_from_the_dead(c: &mut Client, t: Instant) -> Instant {
    c.world.stats.vitals[0].current = 100;
    let rec = &mut c.autoplay.recovery;
    rec.phase = crate::recovery::Phase::Landed;
    rec.since = t;
    rec.death = None;
    let t = t + Duration::from_secs(5);
    c.tick_autoplay(t);
    assert!(!c.autoplay.recovery.active(), "the recovery did not finish");
    for n in 1..=5 {
        c.tick_autoplay(t + Duration::from_millis(100 * n));
    }
    t
}

#[test]
fn a_stop_after_a_failed_replan_leaves_no_goal_for_a_death_to_keep() {
    // Refused at a portal after the leader with no other way, then stopped, then dead before
    // any other journey: the recovery kept the goal left over and walked back to it after.
    let (mut c, goal, now) = after_a_failed_replan();
    c.autoplay.config.team.follow = false;
    c.tick_autoplay(now + Duration::from_millis(100));
    assert_eq!(c.travel_goal_xy(), None, "the goal outlived the stop");
    die(&mut c, now + Duration::from_millis(200));
    assert_eq!(
        c.autoplay.recovery.trip, None,
        "kept for after the recovery"
    );
    back_from_the_dead(&mut c, now + Duration::from_secs(1));
    assert!(
        !(c.traveling() && c.travel_goal_xy() == Some(goal)),
        "it set off for where the leader was when following stopped"
    );
}

#[test]
fn a_death_while_led_keeps_no_goal_left_by_a_failed_replan() {
    // Dead while still following, and stopped before the recovery hands its trip back.
    let (mut c, goal, now) = after_a_failed_replan();
    die(&mut c, now + Duration::from_millis(100));
    assert_eq!(
        c.autoplay.recovery.trip, None,
        "kept for after the recovery"
    );
    c.autoplay.config.team.follow = false;
    back_from_the_dead(&mut c, now + Duration::from_secs(1));
    assert_eq!(c.autoplay.resume_trip, None);
    assert!(
        !(c.traveling() && c.travel_goal_xy() == Some(goal)),
        "it set off for where the leader was when it died"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_journey_put_down_for_a_fight_is_resumed_to_its_place() {
    // Bound upstairs to Archmage Cindrue from the Holtburg lifestone, a swing breaks it off:
    // picked up again, it is bound for that floor's cell, not the map point under it.
    let mut c = crate::testkit::standing_in_the_field(20, 0xA9B4_0019, Vec3::new(84.0, 7.1, 94.0));
    let upstairs = Place {
        at: ac_world::landblock_origin(0xA9B4_011B) + Vec3::new(152.3, 132.5, 69.0),
        cell: 0xA9B4_011B,
    };
    assert!(c.journey_to(upstairs, ARRIVE), "no way upstairs");
    c.remember_journey();
    c.interrupt_travel("attacking");
    assert!(!c.traveling());
    assert!(c.autoplay_resume_journey(), "not taken up again");
    assert_eq!(c.journey_goal(), Some(within_reach(upstairs)));
    let end = c.travel_trip().and_then(|t| t.steps.last().cloned());
    assert_eq!(end, Some(Step::Walk(upstairs)));
}

/// The end of an autoplay tick in which `row` acted, as `tick_autoplay` closes one.
fn tick_as(c: &mut Client, row: &'static str, now: Instant) {
    c.autoplay.tick += 1;
    c.frame_at = now;
    c.autoplay.step = Some(row);
    c.settle_going();
}

/// `n` frames of the body (`tick_player`) from `now`, a twentieth of a second apart, each waiting
/// out the planner's thread as a frame's real time would; the clock after.
fn frames(c: &mut Client, now: Instant, n: u32) -> Instant {
    let mut t = now;
    for _ in 0..n {
        c.tick_player(crate::player::Input::default(), 0.05, t);
        c.pathfinder.settle(Duration::from_secs(5));
        t += Duration::from_millis(50);
    }
    t
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_journey_stands_while_its_row_stands_aside() {
    // Grow's journey 250 m east of the Holtburg lifestone. While the fight row acts the stretch
    // leaves the slot and the body stands, trip and step kept; grow acts again and it walks on.
    let mut c = crate::testkit::standing_in_the_field(20, 0xA9B4_0019, Vec3::new(84.0, 7.1, 94.0));
    c.autoplay.config.enabled = true;
    let me = c.my_position().expect("on its feet");
    c.autoplay.running = Some("grow");
    let east = Place::on_map(me.truncate() + Vec2::new(250.0, 0.0));
    c.frame_at = Instant::now();
    assert!(c.journey_to(east, ARRIVE), "no way east");
    c.autoplay.running = None;
    let mut now = c.frame_at;
    for _ in 0..10 {
        tick_as(&mut c, "grow", now);
        now = frames(&mut c, now, 2);
    }
    let walked = c.my_position().expect("on its feet");
    assert!(walked.distance(me) > 2.0, "never walked");
    let (trip, step) = (c.travel_trip().cloned(), c.travel_progress());
    tick_as(&mut c, "fight", now);
    now = frames(&mut c, now, 2);
    let stood = c.my_position().expect("on its feet");
    for _ in 0..20 {
        tick_as(&mut c, "fight", now);
        now = frames(&mut c, now, 2);
        assert!(
            c.going.as_ref().is_none_or(|g| !g.journey),
            "a stretch walked"
        );
    }
    let here = c.my_position().expect("on its feet");
    assert!(
        here.distance(stood) < 0.1,
        "walked {:.1} m",
        here.distance(stood)
    );
    assert!(c.traveling(), "the journey ended");
    assert_eq!(
        (c.travel_trip().cloned(), c.travel_progress()),
        (trip, step)
    );
    let plans = c.pathfinder.plans;
    for _ in 0..10 {
        tick_as(&mut c, "grow", now);
        now = frames(&mut c, now, 2);
    }
    let stretch = c.going.as_ref().filter(|g| g.journey).map(|g| g.owner);
    assert_eq!(stretch, Some(crate::Owner::Step("grow")));
    assert_eq!(c.pathfinder.plans, plans + 1, "not one stretch planned");
    let on = c.my_position().expect("on its feet");
    assert!(on.distance(here) > 2.0, "stood on after grow acted");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_journey_whose_row_is_passed_over_is_put_down() {
    // The town run's journey 250 m east, then town runs switched off: the row is weighed worth
    // nothing, so its journey is put down on the next tick, not held with the body standing.
    let mut c = crate::testkit::standing_in_the_field(20, 0xA9B4_0019, Vec3::new(84.0, 7.1, 94.0));
    c.autoplay.config.enabled = true;
    let me = c.my_position().expect("on its feet");
    c.autoplay.running = Some("town run");
    let east = Place::on_map(me.truncate() + Vec2::new(250.0, 0.0));
    c.frame_at = Instant::now();
    assert!(c.journey_to(east, ARRIVE), "no way east");
    c.autoplay.running = None;
    c.autoplay.config.growth.town_runs = false;
    c.tick_autoplay(c.frame_at);
    assert_ne!(c.autoplay.step, Some("town run"));
    assert!(!c.traveling(), "held for a row that is never run");
    assert!(c.journey_broken_off(), "put down reads as refused");
}

#[test]
fn a_journey_of_no_row_says_travelling() {
    // A script's journey, autoplay on and no row acting: the status says where it goes.
    let mut c = crate::testkit::character_of_level(crate::testkit::no_data(), 20);
    crate::testkit::stand(&mut c, 0xA9B4_0019, Vec3::new(84.0, 7.1, 94.0));
    c.autoplay.config.enabled = true;
    let me = c.my_position().expect("on its feet");
    // Past the region, under way whatever the planner answers: offline there is no terrain.
    c.travel_to(me.truncate() + Vec2::new(400.0, 0.0));
    assert!(c.traveling(), "no journey");
    c.tick_autoplay(Instant::now());
    assert_eq!(c.autoplay.step, None, "a row acted");
    assert_eq!(c.autoplay.doing, crate::autoplay::Doing::Traveling);
    assert!(
        c.autoplay.status.starts_with("travelling to"),
        "{}",
        c.autoplay.status
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_journey_put_down_for_a_fight_arrives_at_its_place() {
    // A script's journey from the Holtburg lifestone to Archmage Cindrue upstairs; a swing breaks
    // it off 5 s in, and the resume row walks it to her floor's cell.
    let mut c = crate::testkit::standing_in_the_field(20, 0xA9B4_0019, Vec3::new(84.0, 7.1, 94.0));
    c.autoplay.config.enabled = true;
    let upstairs = Place {
        at: ac_world::landblock_origin(0xA9B4_011B) + Vec3::new(152.3, 132.5, 69.0),
        cell: 0xA9B4_011B,
    };
    c.frame_at = Instant::now();
    assert!(c.journey_to(upstairs, ARRIVE), "no way upstairs");
    let start = c.my_position().expect("on its feet");
    let set_off = c.frame_at;
    let mut now = frames(&mut c, set_off, 100);
    assert!(
        c.my_position().expect("on its feet").distance(start) > 5.0,
        "never walked"
    );
    c.autoplay.running = Some("fight");
    c.remember_journey();
    c.interrupt_travel("attacking");
    c.autoplay.running = None;
    tick_as(&mut c, "fight", now);
    assert!(!c.traveling(), "the swing left the journey under way");
    for _ in 0..600 {
        c.autoplay.running = Some("resume the journey");
        let acted = c.autoplay_resume_journey();
        c.autoplay.running = None;
        if !acted {
            break;
        }
        tick_as(&mut c, "resume the journey", now);
        now = frames(&mut c, now, 2);
    }
    let pl = c.player.as_ref().expect("on its feet");
    let me = pl.world_position();
    assert!(!c.traveling(), "still travelling at {me}");
    assert_eq!(pl.cell, 0xA9B4_011B, "ended at {me}");
    assert!(
        crate::body::server_walk::reached(me, upstairs.at, ARRIVE),
        "{me}"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_portal_that_says_too_recently_is_stepped_back_from_and_walked_into_again() {
    // The Town Network passes portal to portal faster than ACE's 3.5 s allows: weenie error
    // 0x04A7. Standing in the mouth never takes the character, and waited out it was left out.
    let holtburg = 0xA9B4_0019;
    let mut c = crate::testkit::standing_at(holtburg, Vec3::new(84.0, 7.1, 94.0));
    let me = c.my_position().unwrap();
    let mouth = c.place_at(me);
    let trip = Trip {
        steps: vec![Step::Portal {
            name: "Portal to Town Network".into(),
            mouth,
            exit: Place::on_map(Vec2::new(44_170.4, 9_636.1)),
        }],
        seconds: 10.0,
    };
    let exit = Place::on_map(Vec2::new(44_170.4, 9_636.1));
    crate::testkit::set_out_on(&mut c, trip, within_reach(exit));
    let now = Instant::now();
    c.portal_too_soon(0x0474, now);
    assert!(
        c.travel.state.too_soon.is_none(),
        "another error is not this one"
    );
    c.portal_too_soon(0x04A7, now);
    let back = match c.step_goal(now) {
        Some(Goal::At { place, .. }) => place,
        other => panic!("{other:?}"),
    };
    let off = back.at.distance(me);
    assert!((2.0..=3.5).contains(&off), "stepped back {off:.1} m");
    match c.step_goal(now + PORTAL_COOLDOWN + Duration::from_millis(100)) {
        Some(Goal::At { place, .. }) => assert_eq!(place.at, mouth.at, "walked back in"),
        other => panic!("{other:?}"),
    }
    assert!(c.travel.state.too_soon.is_none() && c.traveling());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_portal_walked_into_again_after_too_recently_is_not_refused_as_stalled() {
    // Blargerton in the Town Network, off the Holtburg portal and into the Facility Hub's: turned
    // away, it stood in the mouth and the walk back in was refused, the Warden given up.
    let arrival = 0x0007_0145;
    let mut c = crate::testkit::standing_in_the_field(40, arrival, Vec3::new(70.0, -80.0, 0.0));
    let mouth = Place {
        at: Vec3::new(53.6, 1244.0, -0.1),
        cell: 0x0007_0126,
    };
    let exit = Place::on_map(Vec2::new(26_502.1, 312.6));
    let trip = Trip {
        steps: vec![Step::Portal {
            name: "Portal to Facility Hub".into(),
            mouth,
            exit,
        }],
        seconds: 10.0,
    };
    crate::testkit::set_out_on(&mut c, trip, within_reach(exit));
    let mut now = Instant::now();
    let mut walked = false;
    for _ in 0..200 {
        now = frames(&mut c, now, 1);
        let at = c.my_position().expect("on its feet");
        if at.truncate().distance(mouth.at.truncate()) < 0.3 {
            walked = true;
            break;
        }
    }
    assert!(walked, "never reached the mouth");
    // The server answers a second later, as live.
    now = frames(&mut c, now, 20);
    c.portal_too_soon(0x04A7, now);
    now = frames(&mut c, now, 40);
    let back = c.my_position().expect("on its feet");
    let off = back.truncate().distance(mouth.at.truncate());
    assert!(off >= 2.0, "stood {off:.1} m off the mouth while it waited");
    let mut again = false;
    for _ in 0..80 {
        now = frames(&mut c, now, 1);
        let at = c.my_position().expect("on its feet");
        again |= at.truncate().distance(mouth.at.truncate()) < 0.3;
    }
    assert!(
        c.traveling(),
        "the journey ended: {:?}",
        c.travel.why_refused
    );
    assert!(c.travel.refused.is_empty(), "the mouth was left out");
    assert!(again, "never walked back into the mouth");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn lifestone_is_not_sent_again_while_its_first_is_still_under_way() {
    // ACE's /lifestone waits out its animation, 15.2 s on Coldeve, before the teleport: sent
    // again at 15 s, the second began in Holtburg as the first landed there.
    let mut c = crate::testkit::standing_in_the_field(40, 0x8F68_0014, Vec3::new(50.0, 50.0, 14.0));
    let exit = Place {
        at: Vec3::new(32532.0, 34567.0, 94.0),
        cell: 0xA9B4_0019,
    };
    let trip = ac_world::trip::Trip {
        steps: vec![
            ac_world::trip::Step::Recall {
                spell: ac_world::recalls::spell::FREE_LIFESTONE,
                name: "/lifestone".into(),
                exit,
            },
            ac_world::trip::Step::Walk(exit),
        ],
        seconds: 60.0,
    };
    let goal = crate::Goal::At {
        place: exit,
        within: 5.0,
    };
    crate::testkit::set_out_on(&mut c, trip, goal);
    let t0 = std::time::Instant::now();
    let at = |s: f32| t0 + std::time::Duration::from_secs_f32(s);
    c.step_goal(at(0.0));
    c.step_goal(at(1.0));
    assert_eq!(c.travel.state.recall_casts, 1, "the first /lifestone");
    c.step_goal(at(17.0));
    assert_eq!(c.travel.state.recall_casts, 1, "sent again at 16 s");
    c.step_goal(at(27.0));
    assert_eq!(c.travel.state.recall_casts, 2, "never sent again");
}
