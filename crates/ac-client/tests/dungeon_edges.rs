//! Edges and falls in the Holtburg dungeon (landblock 0x01F6). Needs
//! AC_DATA_DIR.
//!
//! A character hunting there under autoplay could not see its target,
//! set off straight for it with no route to follow, left the floor and
//! fell for ever: the server, which takes the client's word for where
//! it is, followed it forty-five kilometres down.
//!
//! It went over no edge. It stood on the ramp in 0x01F6029F, which climbs
//! east over a hump and down through 02A3 and 028E to the storey at six
//! metres down, with another storey twelve metres down under the hump.
//! A headless client ticks ten or twenty times a second, and a frame
//! that builds a piece of the navigation graph takes longer still; a
//! stride that long up a ramp ends with the ramp more than a step over
//! the feet, the walk found no floor within a step and a floor under the
//! ramp, and the character fell through the ramp -- and ran on through
//! the air past the end of the floor below.
//!
//! Over the rooms 0x01F6022C, 022D, 023B and 023C the vaults rise into
//! the cells 0x01F60291, 0292, 0297 and 0298, and the tops of their ribs
//! are floor as far as the geometry is concerned: seven or eight metres
//! up, unrailed, with nothing joining them to the rooms below.

use ac_client::pathfinder::Pathfinder;
use ac_client::player::{Input, MovementLimits, Player};
use ac_nav::{Aim, Ground, Steering};
use ac_net::session::{Config, Session};
use ac_scene::Assets;
use glam::{Quat, Vec2, Vec3};
use std::time::{Duration, Instant};

const DUNGEON: u32 = 0x01F6_0000;

fn assets() -> Option<Assets> {
    let dir = std::env::var_os("AC_DATA_DIR")?;
    Some(Assets::open(std::path::Path::new(&dir)).unwrap())
}

/// The dungeon's floor under a world `(x, y)`, within a step of `z`.
///
/// Asked of the dungeon's own collision. `Player::ground_height` finds
/// the landblock by the position, and a dungeon's cells lie off the
/// side of its own square, in the block next door, where there is no
/// floor at all.
fn floor(assets: &Assets, at: Vec3) -> Vec3 {
    let coll = assets.block_collision(DUNGEON).unwrap();
    let (z, _) = coll
        .world
        .floor_at(at, 0.6, 1.5)
        .unwrap_or_else(|| panic!("no floor at {at}"));
    Vec3::new(at.x, at.y, z)
}

/// A character standing at the world position `at` in `cell`.
fn stand(assets: &Assets, cell: u32, at: Vec3) -> Player {
    let local = at - ac_world::landblock_origin(cell);
    let mut pl = Player::new(assets, cell, local, Quat::IDENTITY);
    pl.set_motion_table(assets, 0x0200_0001, 0x0900_0001);
    pl
}

/// The steering's questions answered as the client answers them, from
/// the character's own collision and, when there is one, the planner on
/// its thread -- waited for, so that its route is taken on the very next
/// frame, the soonest the client could take it.
struct Standing<'a> {
    player: &'a mut Player,
    assets: &'a Assets,
    wide: Option<&'a mut Pathfinder>,
}

impl Ground for Standing<'_> {
    fn at(&self) -> Vec3 {
        self.player.world_position()
    }
    fn cell(&self) -> u32 {
        self.player.cell
    }
    fn block(&self) -> u32 {
        self.player.landblock()
    }
    fn line_blocked(&mut self, block: u32, from: Vec3, to: Vec3) -> bool {
        self.player.line_blocked(self.assets, block, from, to)
    }
    fn line_drops(&mut self, block: u32, from: Vec3, to: Vec3) -> bool {
        self.player.line_drops(self.assets, block, from, to)
    }
    fn find_path(&mut self, block: u32, from: Vec3, to: Vec3, cell: u32) -> Option<Vec<Vec3>> {
        self.player.find_path(self.assets, block, from, to, cell)
    }
    fn ask_wide(&mut self, from: Vec3, to: Vec3, block: u32, outdoors: bool, exact: bool) {
        let cap = self.player.capsule();
        if let Some(wide) = self.wide.as_deref_mut() {
            let ends = ac_client::pathfinder::Ends {
                outdoors,
                exact_to: exact,
            };
            wide.ask(from, to, cap, block, ends, Instant::now());
        }
    }
    fn take_wide(&mut self, from: Vec3, to: Vec3) -> Option<Vec<Vec3>> {
        let wide = self.wide.as_deref_mut()?;
        for _ in 0..2000 {
            if let Some(route) = wide.take(from, to) {
                return Some(route);
            }
            if !wide.busy() {
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        None
    }
}

/// Where the steering sends the character towards `goal`, asked the
/// way the client asks it indoors: the goal named by the block the
/// character stands in.
fn steer(assets: &Assets, pl: &mut Player, goal: Vec3) -> Aim {
    let block = pl.landblock();
    let mut st = Steering::new(Instant::now());
    let mut ground = Standing {
        player: pl,
        assets,
        wide: None,
    };
    st.steer(&mut ground, goal, block, Instant::now())
}

/// Run at the goal the way the client does, frames of `dt` long, for
/// `secs`: steered toward it while further off than the autoplay stops
/// short of, standing still on a refusal. Every frame's position, and
/// whether the character was in the air at it.
fn chase(
    assets: &Assets,
    pl: &mut Player,
    wide: Option<&mut Pathfinder>,
    goal: Vec3,
    dt: f32,
    secs: f32,
) -> Vec<(Vec3, bool)> {
    let mut st = Steering::new(Instant::now());
    let mut wide = wide;
    let mut now = Instant::now();
    let mut frames = Vec::new();
    for _ in 0..(secs / dt).ceil() as usize {
        let at = pl.world_position();
        let d = goal - at;
        let mut input = Input::default();
        pl.step_cap = None;
        if Vec2::new(d.x, d.y).length() > 2.5 || d.z.abs() > 2.0 {
            let block = pl.landblock();
            let aim = {
                let mut ground = Standing {
                    player: pl,
                    assets,
                    wide: wide.as_deref_mut(),
                };
                st.steer(&mut ground, goal, block, now)
            };
            if let Aim::Go(to) = aim {
                let d = to - at;
                pl.heading = (-d.x).atan2(d.y);
                pl.step_cap = Some(Vec2::new(d.x, d.y).length());
                input.forward = 1.0;
                input.run = true;
            }
        }
        pl.update(assets, &input, dt);
        frames.push((pl.world_position(), pl.is_airborne()));
        now += Duration::from_secs_f32(dt);
    }
    frames
}

/// The top of a rib in 0x01F60291, over the room 0x01F6022C, where the
/// graph stands an island of nodes.
const RIB: Vec3 = Vec3::new(225.0, 47_166.0, 7.2);
const RIB_CELL: u32 = 0x01F6_0291;
/// The floor of the room below, off the rib's open south-west side.
const BELOW: Vec3 = Vec3::new(218.0, 47_159.0, 0.0);
/// Somewhere to stand in the room below, with the rib ahead and above.
const ROOM: Vec3 = Vec3::new(223.5, 47_158.5, 0.0);
const ROOM_CELL: u32 = 0x01F6_022C;

#[test]
fn a_straight_walk_off_a_vault_rib_onto_the_floor_below_is_leaned_on() {
    let Some(assets) = assets() else {
        return;
    };
    let rib = floor(&assets, RIB);
    assert!(rib.z > 6.0, "not up on the rib: {rib}");
    let goal = floor(&assets, BELOW);
    let mut pl = stand(&assets, RIB_CELL, rib);
    assert!(pl.line_blocked(&assets, DUNGEON, rib, goal));
    assert!(
        pl.find_path(&assets, DUNGEON, rib, goal, DUNGEON).is_none(),
        "the rib is an island: nothing leads down from it"
    );
    // Off the edge onto the room's floor is the only way down there is,
    // and a fall that lands. Refused, the character stood on the rib for
    // as long as what it wanted stayed below.
    assert!(!pl.line_drops(&assets, DUNGEON, rib, goal));
    assert_eq!(steer(&assets, &mut pl, goal), Aim::Go(goal));
    let frames = chase(&assets, &mut pl, None, goal, 1.0 / 30.0, 4.0);
    let (end, air) = *frames.last().unwrap();
    assert!(
        !air && end.z.abs() < 0.5 && Vec2::new(goal.x - end.x, goal.y - end.y).length() < 3.0,
        "leaned off the rib and came to {end}"
    );
}

#[test]
fn a_straight_walk_from_the_floor_to_a_goal_up_on_a_rib_is_refused() {
    let Some(assets) = assets() else {
        return;
    };
    let me = floor(&assets, ROOM);
    let goal = floor(&assets, RIB);
    assert!(goal.z > 6.0, "not up on the rib: {goal}");
    let mut pl = stand(&assets, ROOM_CELL, me);
    assert!(pl.line_blocked(&assets, DUNGEON, me, goal));
    assert!(pl.find_path(&assets, DUNGEON, me, goal, DUNGEON).is_none());
    // Nothing solid in the way and floor all the way to the spot under
    // the goal: leaning on that gets a character to stand beneath it
    // and push, for ever.
    assert!(pl.line_drops(&assets, DUNGEON, me, goal));
    assert_eq!(steer(&assets, &mut pl, goal), Aim::NoWay);
}

#[test]
fn a_walk_across_the_room_is_still_a_walk() {
    let Some(assets) = assets() else {
        return;
    };
    let me = floor(&assets, ROOM);
    // Into the next room south, 0x01F6022D, on the same floor.
    let goal = floor(&assets, Vec3::new(222.0, 47_150.0, 0.0));
    let mut pl = stand(&assets, ROOM_CELL, me);
    assert!(!pl.line_drops(&assets, DUNGEON, me, goal));
    assert!(matches!(steer(&assets, &mut pl, goal), Aim::Go(_)));
}

#[test]
fn the_stair_corridor_is_walked_not_refused() {
    let Some(assets) = assets() else {
        return;
    };
    // Down the corridor 0x01F6029F -> 0x01F602A3 -> 0x01F6028E: from the
    // top at six metres to the foot at minus six, near twelve metres
    // down in twenty along.
    let top = floor(&assets, Vec3::new(277.5, 47_152.0, 6.0));
    let foot = floor(&assets, Vec3::new(296.5, 47_152.0, -6.0));
    assert!(top.z - foot.z > 10.0, "{top} to {foot}");
    let mut pl = stand(&assets, 0x01F6_02A3, top);
    assert!(!pl.line_drops(&assets, DUNGEON, top, foot), "down");
    assert!(!pl.line_drops(&assets, DUNGEON, foot, top), "up");
    assert!(matches!(steer(&assets, &mut pl, foot), Aim::Go(_)));

    // And walked, all the way down, feet on the stairs.
    let mut st = Steering::new(Instant::now());
    let mut now = Instant::now();
    let dt = 1.0 / 30.0;
    let run = Input {
        forward: 1.0,
        run: true,
        ..Input::default()
    };
    for _ in 0..300 {
        let at = pl.world_position();
        if Vec2::new(foot.x - at.x, foot.y - at.y).length() < 0.5 {
            break;
        }
        let aim = {
            let mut ground = Standing {
                player: &mut pl,
                assets: &assets,
                wide: None,
            };
            st.steer(&mut ground, foot, DUNGEON, now)
        };
        let Aim::Go(to) = aim else {
            panic!("refused the stairs at {at}");
        };
        let d = to - at;
        pl.heading = (-d.x).atan2(d.y);
        pl.step_cap = Some(Vec2::new(d.x, d.y).length());
        pl.update(&assets, &run, dt);
        assert!(
            !pl.is_airborne(),
            "left the stairs at {}",
            pl.world_position()
        );
        now += Duration::from_secs_f32(dt);
    }
    let end = pl.world_position();
    assert!(
        end.distance(foot) < 1.0,
        "stopped at {end}, the foot is {foot}"
    );
}

#[test]
fn a_fall_off_a_rib_onto_the_floor_below_still_lands_there() {
    let Some(assets) = assets() else {
        return;
    };
    let rib = floor(&assets, RIB);
    let mut pl = stand(&assets, RIB_CELL, rib);
    // Straight off it, the way the refused walk would have gone.
    pl.heading = (-(BELOW.x - rib.x)).atan2(BELOW.y - rib.y);
    let run = Input {
        forward: 1.0,
        run: true,
        ..Input::default()
    };
    let mut fell = false;
    for _ in 0..120 {
        pl.update(&assets, &run, 1.0 / 30.0);
        fell |= pl.is_airborne();
        if fell && !pl.is_airborne() {
            break;
        }
    }
    let end = pl.world_position();
    assert!(fell, "never left the rib: {end}");
    assert!(!pl.is_airborne(), "still falling at {end}");
    assert!(end.z.abs() < 0.5, "a real fall lands on the floor: {end}");
    assert!(
        pl.owes_position(),
        "where it came down is owed to the server, which has it in the air"
    );
}

/// Forty metres south of the rooms, with no floor anywhere under it in
/// the dungeon or in the landblock the position lies in.
const VOID: Vec2 = Vec2::new(222.0, 47_120.0);

/// Fly from where the character is, level, through the walls to over
/// [`VOID`], and let go. Returns how far it fell below where it let go
/// at its lowest, and where it came to rest.
fn fly_out_and_let_go(assets: &Assets, pl: &mut Player) -> (f32, Vec3) {
    pl.set_limits(MovementLimits::UNRESTRICTED);
    pl.speed_boost = 4.0;
    assert!(pl.set_noclip(true));
    let void = VOID.extend(pl.world_position().z);
    let fly = Input {
        forward: 1.0,
        run: true,
        ..Input::default()
    };
    for _ in 0..900 {
        let d = void - pl.world_position();
        if Vec2::new(d.x, d.y).length() < 0.5 {
            break;
        }
        pl.heading = (-d.x).atan2(d.y);
        pl.update(assets, &fly, 1.0 / 30.0);
    }
    let over = pl.world_position();
    assert!(over.distance(void) < 1.0, "flew to {over}, not {void}");
    pl.speed_boost = 1.0;
    pl.set_noclip(false);
    assert!(pl.is_airborne());
    let mut lowest = over.z;
    for _ in 0..600 {
        pl.update(assets, &Input::default(), 1.0 / 30.0);
        lowest = lowest.min(pl.world_position().z);
        if !pl.is_airborne() {
            break;
        }
    }
    (over.z - lowest, pl.world_position())
}

#[test]
fn a_fall_with_nothing_under_it_comes_back_to_the_last_floor() {
    let Some(assets) = assets() else {
        return;
    };
    let me = floor(&assets, ROOM);
    let mut pl = stand(&assets, ROOM_CELL, me);
    for _ in 0..10 {
        pl.update(&assets, &Input::default(), 1.0 / 30.0);
    }
    let (fell, end) = fly_out_and_let_go(&assets, &mut pl);
    assert!(!pl.is_airborne(), "still falling at {end}");
    assert!(end.distance(me) < 0.1, "came back to {end}, stood at {me}");
    // As long as a leap stays up, and no longer: under forty metres of
    // falling, not forty-five kilometres.
    assert!(fell > 5.0 && fell < 45.0, "fell {fell} m");
}

#[test]
fn a_character_put_somewhere_else_comes_back_to_where_it_was_put() {
    let Some(assets) = assets() else {
        return;
    };
    let me = floor(&assets, ROOM);
    let mut pl = stand(&assets, ROOM_CELL, me);
    for _ in 0..10 {
        pl.update(&assets, &Input::default(), 1.0 / 30.0);
    }
    // The server moves it to the foot of the stairs, as a teleport or a
    // correction does: straight into the cell and the position, with no
    // frame of its own walk in between.
    let foot = floor(&assets, Vec3::new(296.5, 47_152.0, -6.0));
    pl.cell = 0x01F6_028E;
    pl.local = foot - ac_world::landblock_origin(pl.cell);
    let (_, end) = fly_out_and_let_go(&assets, &mut pl);
    assert!(!pl.is_airborne(), "still falling at {end}");
    assert!(
        end.distance(foot) < 0.1,
        "came back to {end}, not to {foot} where the server put it"
    );
}

/// A run rate for a character of about the one that fell: Run near two
/// hundred, fifteen metres a second.
const RUN_RATE: f32 = 2.5;

/// The longest frame the headless client lets a tick be.
const HEADLESS_FRAME: f32 = 0.25;

#[test]
fn a_long_frame_running_up_the_stairs_out_of_028e_keeps_its_feet() {
    let Some(assets) = assets() else {
        return;
    };
    // The cell the character was last placed in and the way it went,
    // west: up the flight out of 0x01F6028E toward 02A3. Under the flight
    // there is nothing; under 02A3, the storey twelve metres down, ending
    // at the foot of the hump -- the floor it fell past.
    let start = floor(&assets, Vec3::new(293.5, 47_152.0, -4.0));
    let run = Input {
        forward: 1.0,
        run: true,
        ..Input::default()
    };
    for dt in [0.1, HEADLESS_FRAME] {
        let mut pl = stand(&assets, 0x01F6_028E, start);
        pl.run_rate = RUN_RATE;
        pl.heading = (1.0f32).atan2(0.0);
        let mut top = pl.world_position();
        for _ in 0..(3.0 / dt) as usize {
            pl.update(&assets, &run, dt);
            let at = pl.world_position();
            assert!(
                !pl.is_airborne(),
                "at {dt} s a frame, fell through the stairs at {at}"
            );
            top = at;
            if at.x < 278.0 {
                break;
            }
        }
        // Over the top, and standing on the stairs wherever the last
        // frame left it.
        let under = floor(&assets, top);
        assert!(
            top.x < 278.0 && (top.z - under.z).abs() < 0.1,
            "at {dt} s a frame, stopped at {top} short of the top, or off the stairs"
        );
    }
}

#[test]
fn the_walk_that_fell_keeps_to_the_floor_at_a_headless_frame() {
    let Some(assets) = assets() else {
        return;
    };
    // Where the watcher had the character when it set off, on the ramp in
    // 0x01F6029F, and something twenty-nine metres off with no clear shot
    // at it: on the storey six metres down to the west, reached over the
    // hump and down the stairs, the way the graph and the neighbourhood
    // planner both lead.
    let me = floor(&assets, Vec3::new(274.0, 47_152.0, 4.5));
    let goal = floor(&assets, Vec3::new(246.0, 47_152.0, -6.0));
    assert!((me.distance(goal) - 29.1).abs() < 1.0, "{me} to {goal}");
    let mut wide = Pathfinder::new(&assets);
    for dt in [0.1, HEADLESS_FRAME] {
        let mut pl = stand(&assets, 0x01F6_029F, me);
        pl.run_rate = RUN_RATE;
        wide.reset();
        let frames = chase(&assets, &mut pl, Some(&mut wide), goal, dt, 8.0);
        if let Some((at, _)) = frames.iter().find(|(_, air)| *air) {
            panic!("at {dt} s a frame, left the floor at {at}");
        }
        // Down on the storey it was after. A quarter of a second is
        // nearly four metres at a run, more than the distance the walk
        // stops short at, so how close it comes at that pace is left to
        // the shorter frame.
        let (end, _) = *frames.last().unwrap();
        assert!(
            (goal.z - end.z).abs() < 2.0,
            "at {dt} s a frame, never got down to {goal}: {end}"
        );
        if dt < HEADLESS_FRAME {
            assert!(
                Vec2::new(goal.x - end.x, goal.y - end.y).length() < 3.0,
                "at {dt} s a frame, got no nearer than {end}"
            );
        }
    }
}

#[test]
fn a_stride_longer_than_a_step_can_climb_is_held_at_the_stairs_not_dropped_through_them() {
    let Some(assets) = assets() else {
        return;
    };
    // Faster than any run there is, so that even cut as fine as a frame
    // is cut each step still rises more than a step up the flight out of
    // 0x01F6028E. The stairs are in the way, and the storey twelve metres
    // down past their top is not somewhere to go.
    let start = floor(&assets, Vec3::new(293.5, 47_152.0, -4.0));
    let mut pl = stand(&assets, 0x01F6_028E, start);
    pl.set_limits(MovementLimits::UNRESTRICTED);
    pl.speed_boost = 4.0;
    pl.run_rate = 20.0;
    pl.heading = (1.0f32).atan2(0.0);
    let run = Input {
        forward: 1.0,
        run: true,
        ..Input::default()
    };
    for _ in 0..8 {
        pl.update(&assets, &run, HEADLESS_FRAME);
        let at = pl.world_position();
        assert!(!pl.is_airborne(), "dropped through the stairs at {at}");
        assert!(at.z > -6.5, "under the stairs at {at}");
    }
}

#[test]
fn a_character_put_back_on_its_feet_tells_the_server_at_once_standing_still() {
    let Some(assets) = assets() else {
        return;
    };
    let me = floor(&assets, ROOM);
    let mut pl = stand(&assets, ROOM_CELL, me);
    for _ in 0..10 {
        pl.update(&assets, &Input::default(), 1.0 / 30.0);
    }
    let (_, end) = fly_out_and_let_go(&assets, &mut pl);
    assert!(end.distance(me) < 0.1, "came back to {end}");
    // Standing still since, with no quarter of a second gone: nothing
    // that waits for movement, or for the next quarter, would send it.
    assert!(pl.owes_position());
    pl.update(&assets, &Input::default(), 1.0 / 30.0);
    assert!(pl.owes_position(), "forgotten before it was sent");
    let now = Instant::now();
    let mut session = Session::new(
        Config {
            account: String::new(),
            password: String::new(),
            dats: Vec::new(),
            echo_interval: Duration::from_secs(5),
            ack_interval: Duration::from_secs(1),
        },
        now,
    );
    pl.report(&mut session, &Input::default(), now, false);
    assert!(!pl.owes_position(), "reported and still owed");
    let (cell, local) = pl.take_sent().expect("nothing was sent");
    assert_eq!(cell, ROOM_CELL);
    let sent = ac_world::landblock_origin(cell) + local;
    assert!(sent.distance(me) < 0.1, "sent {sent}, stands at {me}");
}

#[test]
fn a_step_down_from_an_upper_floor_the_graph_has_no_way_down_from_is_leaned_on() {
    let Some(assets) = assets() else {
        return;
    };
    // Upstairs in a Holtburg house, on a floor reached on foot from the
    // lifestone; the goal in the room under it, two and a half metres
    // down over the edge, where the graph stands a single node with no
    // way to it.
    const HOLTBURG: u32 = 0xA9B4_0000;
    let me = Vec3::new(32_553.0, 34_594.5, 97.5);
    let goal = Vec3::new(32_550.17, 34_597.33, 94.92);
    let mut pl = stand(&assets, 0xA9B4_0160, me);
    assert!(pl.line_blocked(&assets, HOLTBURG, me, goal));
    assert!(pl
        .find_path(&assets, HOLTBURG, me, goal, HOLTBURG)
        .is_none());
    assert!(!pl.line_drops(&assets, HOLTBURG, me, goal));
    assert_eq!(steer(&assets, &mut pl, goal), Aim::Go(goal));
    let frames = chase(&assets, &mut pl, None, goal, 1.0 / 30.0, 4.0);
    let (end, air) = *frames.last().unwrap();
    assert!(
        !air && (end.z - goal.z).abs() < 0.5
            && Vec2::new(goal.x - end.x, goal.y - end.y).length() < 3.0,
        "stepped off toward {goal} and came to {end}"
    );
}
