use std::time::Instant;

use super::server_walk::{reached, server_walk_over, standing_aside};
use crate::{dodge, player, Client, Standing};

/// What the character did this frame, for whoever draws it.
#[derive(Debug, Default)]
pub struct PlayerFrame {
    /// Position, look or pose changed: the model needs re-placing.
    pub dirty: bool,
    /// Part transforms from the current animation frame.
    pub pose: Option<Vec<glam::Mat4>>,
}

impl Client {
    pub(crate) fn tick_player(
        &mut self,
        input: player::Input,
        dt: f32,
        now: Instant,
    ) -> PlayerFrame {
        self.held_run = input.run;
        // The user taking the controls ends an overland trip.
        let manual = input.forward != 0.0 || input.strafe != 0.0;
        if manual && (self.traveling() || self.visiting().is_some()) {
            tracing::info!("travel: cancelled, the user took over");
            self.cancel_travel();
        }
        // A visit: the journey there, the walk up to the person, the use.
        // Before the journey's leg is read, so a visit that takes over
        // from its journey walks this frame.
        if !manual {
            self.tick_visit(now);
        }
        // The next leg of the overland route, if one is being walked.
        let travel_goal = self.travel_goal(now);
        let travelling = self.traveling();
        // Player movement, camera, and reporting.
        if let Some(pl) = self.player.as_mut() {
            let mut input = input;
            self.walk_frame = None;
            // Server-driven MoveTo (using something out of reach): run toward
            // the target until close enough, unless the user takes over.
            // Without one, the current leg of the overland route.
            if (self.move_to.is_none() && travel_goal.is_none() && self.follow.is_none()) || manual
            {
                self.steering.reset();
            }
            // A sidestep out of a spell's way comes before every other
            // goal, for the moment it lasts; it ends when its time is
            // up, when it is reached, or when the user takes over.
            let dodging = match self.dodge_to {
                Some((target, until)) => {
                    let d = target - pl.world_position();
                    let reached = glam::Vec2::new(d.x, d.y).length() < dodge::STOP;
                    if manual || now >= until || reached {
                        self.dodge_to = None;
                        None
                    } else {
                        Some(target)
                    }
                }
                None => None,
            };
            {
                let mut source = "dodge";
                let goal = if let Some(target) = dodging {
                    // Straight there: the step was checked for room
                    // when it was chosen, and there is no time to plan.
                    let d = target - pl.world_position();
                    let flat = glam::Vec2::new(d.x, d.y);
                    if flat.length() > 1e-3 {
                        pl.heading = (-flat.x).atan2(flat.y);
                    }
                    input.forward = 1.0;
                    input.run = true;
                    None
                } else {
                    source = match (self.move_to, travel_goal) {
                        (Some(_), _) => "server walk",
                        (None, Some(_)) => "journey",
                        (None, None) => "follow",
                    };
                    match self.move_to {
                        Some(ac_world::object::MoveTarget::Object(g)) => self
                            .world
                            .objects
                            .get(&g)
                            .and_then(|o| o.display.or(o.position))
                            .map(|p| (ac_world::landblock_origin(p.cell) + p.local, 1.0, p.cell)),
                        Some(ac_world::object::MoveTarget::Position { cell, local }) => {
                            Some((ac_world::landblock_origin(cell) + local, 0.3, cell))
                        }
                        None => travel_goal.or_else(|| {
                            self.follow.map(|f| {
                                // Indoors the goal is in the block we
                                // stand in: a dungeon's cells run past
                                // its outdoor square (the academy's lie
                                // at negative local y), so the world
                                // coordinates would name the wrong one
                                // and the steering would never plan.
                                let block = if pl.is_indoors() {
                                    pl.landblock()
                                } else {
                                    let bx = (f.target.x / 192.0).floor().clamp(0.0, 255.0) as u32;
                                    let by = (f.target.y / 192.0).floor().clamp(0.0, 255.0) as u32;
                                    (bx << 24) | (by << 16)
                                };
                                (f.target, f.stop, block)
                            })
                        }),
                    }
                };
                tracing::trace!(
                    "move: goal {goal:?} manual {manual} move_to {:?} travelling {}",
                    self.move_to,
                    travelling
                );
                // A server walk the server has answered for, walked all
                // the way, is over: nothing more is coming to end it.
                if dodging.is_none()
                    && server_walk_over(
                        self.move_to,
                        goal.map(|(at, stop, _)| (at, stop)),
                        pl.world_position(),
                        self.move_to_answered,
                    )
                {
                    tracing::debug!("server move-to over: there, and answered");
                    self.move_to = None;
                }
                if let Some((g, stop, goal_cell)) = goal {
                    // Whoever is steering says how far this frame may
                    // carry us; by default nothing does.
                    pl.step_cap = None;
                    let d = g - pl.world_position();
                    let flat = glam::Vec2::new(d.x, d.y);
                    if !manual && pl.noclip {
                        // Flying: straight there, up or down as needed,
                        // nothing in the way.
                        if flat.length() > stop {
                            pl.heading = (-flat.x).atan2(flat.y);
                            input.forward = 1.0;
                            input.run = true;
                        }
                        input.climb = if d.z > 1.0 {
                            1.0
                        } else if d.z < -1.0 {
                            -1.0
                        } else {
                            0.0
                        };
                    } else if !manual && !reached(pl.world_position(), g, stop) {
                        // Straight at the goal while nothing is in the
                        // way; through the waypoints of a route otherwise.
                        let mut standing = Standing {
                            player: pl,
                            assets: &self.assets,
                            wide: &mut self.pathfinder,
                        };
                        let aim = self.steering.steer(&mut standing, g, goal_cell, now);
                        tracing::trace!(
                            target: "steer",
                            "at {:?} goal {g:?} aim {aim:?}",
                            standing.player.world_position()
                        );
                        // What the server has put in the room -- a
                        // chest, a hook, a cart -- is not in the block's
                        // geometry, and the physics walks straight
                        // through it; the retail client did not. The
                        // leg to wherever the steering aims is walked
                        // round the first such thing on it, a frame at
                        // a time, once the steering has had its say:
                        // the graph and the planner are for what
                        // actually stops the character, and clutter
                        // must not send a walk to them.
                        let steered = aim;
                        let aim = match aim {
                            ac_nav::Aim::Go(at) => {
                                let me = pl.world_position();
                                let block = pl.landblock();
                                let cap = pl.capsule();
                                let clutter = self
                                    .clutter
                                    .refresh(&self.world, me, |id| self.assets.setup(id).ok());
                                ac_nav::Aim::Go(ac_nav::obstacles::detour(
                                    clutter,
                                    me,
                                    at,
                                    &cap,
                                    |a, b| !pl.line_blocked(&self.assets, block, a, b),
                                ))
                            }
                            ac_nav::Aim::NoWay => ac_nav::Aim::NoWay,
                        };
                        self.walk_frame = Some(crate::tally::WalkFrame {
                            at: now,
                            goal: g,
                            goal_cell,
                            source,
                            aim: match aim {
                                ac_nav::Aim::Go(at) => Some(at),
                                ac_nav::Aim::NoWay => None,
                            },
                            detoured: aim != steered,
                            route: self
                                .steering
                                .route
                                .as_ref()
                                .map(|r| (r.next, r.waypoints.len())),
                            wedged: pl.wedged(),
                        });
                        // No way there at all: the line is blocked and
                        // no route was found. Standing still is the
                        // whole of the answer.
                        //
                        // This is where every "it ran straight at the
                        // wall" actually came from. The steering had
                        // already been taught to say "nowhere to go" --
                        // but saying it means aiming at one's own feet,
                        // and the mover set off forward whatever the
                        // aim was, keeping the heading it had. So the
                        // character leaned on the wall it had just
                        // decided was in the way, and only the stuck
                        // detector ever stopped it.
                        match aim {
                            ac_nav::Aim::NoWay => input.forward = 0.0,
                            ac_nav::Aim::Go(at) => {
                                let d = at - pl.world_position();
                                let flat = glam::Vec2::new(d.x, d.y);
                                if flat.length() > 1e-3 {
                                    pl.heading = (-flat.x).atan2(flat.y);
                                }
                                pl.step_cap = Some(flat.length());
                                input.forward = 1.0;
                                input.run = true;
                            }
                        }
                    }
                }
            }
            // Stamina caps the jump (ACE refuses nothing but retail
            // greyed the charge bar; we cap the power at what is left).
            pl.max_jump_power = player::max_jump_power(self.world.stats.vitals[1].current, 0.0);
            // The Jump skill drives the height and the Run skill the
            // pace, both from the sheet at their current (buffed) value.
            let table = self.assets.skill_table().ok();
            let current = |stats: &ac_world::stats::PlayerStats, id: u32| {
                stats
                    .skill(id)
                    .map(|sk| stats.skill_value(sk, table.as_ref().and_then(|t| t.get(id))))
                    .unwrap_or(0)
            };
            let jump = current(&self.world.stats, ac_world::stats::skill::JUMP);
            if jump > 0 {
                pl.jump_skill = jump;
            }
            pl.run_rate = player::run_rate(current(&self.world.stats, ac_world::stats::skill::RUN));
            pl.speed_boost = self.speed_boost;
            pl.jump_height = self.jump_height;
            pl.set_limits(self.movement_rules.limits());
            if let Some(p) = self.pending_jump.take() {
                if !pl.noclip {
                    pl.jump(p);
                }
            }
            pl.update(&self.assets, &input, dt);
            if let Some(j) = pl.last_jump.take() {
                tracing::info!("jump power {:.2} velocity {:?}", j.power, j.velocity);
                self.session.send_action(
                    ac_net::messages::action::JUMP,
                    &ac_net::messages::jump(j.power, j.velocity.to_array(), 1),
                );
            }
            // One-shot motions the server broadcast for us (attacks, emotes).
            let mut cmds = Vec::new();
            if let Some(o) = self.world.player_mut() {
                while let Some(c) = o.commands.pop() {
                    cmds.push(c);
                }
            }
            for c in cmds {
                pl.play_command(&self.assets, c.command as u32, c.speed);
            }
            let pose = pl.animate(&self.assets, &input, dt);
            if pose.is_some() {
                pl.dirty = true;
            }
            let quiet = standing_aside(self.move_to, self.move_to_since, now);
            let mut ran_out = None;
            if !quiet && self.move_to.is_some() {
                tracing::debug!("server move-to timed out");
                ran_out = self.move_to.take();
            }
            pl.report(&mut self.session, &input, now, quiet);
            // What went out, for the world to know the server's echo of
            // it by when it comes back after the character has been put
            // somewhere else.
            if let Some((cell, local)) = pl.take_sent() {
                self.world.player_reported(cell, local);
            }
            let dirty = pl.dirty;
            if pl.dirty {
                pl.dirty = false;
                if let Some(o) = self.world.player_mut() {
                    o.position = Some(ac_world::Position {
                        cell: pl.cell,
                        local: pl.local,
                        rotation: pl.rotation(),
                    });
                }
                self.world.walked();
            }
            // A server walk let go short of what it was walking to: the
            // use it was for is walked the rest of the way (see `visit`).
            if ran_out.is_some() {
                self.server_walk_ran_out(ran_out, now);
            }
            return PlayerFrame { dirty, pose };
        }
        PlayerFrame::default()
    }
}
