use crate::{
    heading_quat, landblock_origin, object, reconcile, Applied, Motion, MoveTarget, MovementEvent,
    World, SAME_REPORT,
};

impl World {
    pub(super) fn apply_update_position(&mut self, body: &[u8]) -> Applied {
        match object::UpdatePosition::parse(body) {
            Ok(up) => {
                self.bring_back(up.guid);
                let is_player = self.player_guid == Some(up.guid);
                let applied = if let Some(o) = self.objects.get_mut(&up.guid) {
                    if is_player {
                        // Our own positions come back as echoes, a
                        // quarter of a second old: at a run that is
                        // metres behind where we are, and taking them
                        // dragged a fast character back every echo.
                        // The server counts its teleport and forced
                        // position sequences up when it moves us
                        // itself; only those moves are taken, plus
                        // anything so far off that something is wrong.
                        let seqs = (up.teleport_seq, up.force_seq);
                        let moved_by_server = match self.player_move_seqs {
                            Some(seen) => seen != seqs,
                            None => true,
                        };
                        self.player_move_seqs = Some(seqs);
                        // Where the server says we are, versus where we
                        // have run to. A normal echo lags a few metres
                        // and keeps pace with us; a move the server
                        // refused (flying through a wall, a jump it
                        // rejected) leaves its position frozen while
                        // ours runs away, so the gap grows and the
                        // server's own position barely moves.
                        let server_pos = landblock_origin(up.position.cell) + up.position.local;
                        let gap = match o.position {
                            Some(cur) => {
                                (landblock_origin(cur.cell) + cur.local - server_pos).length()
                            }
                            None => f32::INFINITY,
                        };
                        let server_step = self
                            .player_echo
                            .map(|e| (landblock_origin(e.cell) + e.local - server_pos).length())
                            .unwrap_or(f32::INFINITY);
                        // One of our own reports coming back is no move
                        // of the server's, however far the character has
                        // been put from it since. Taken for one, the
                        // echo of a report from a fall stood a character
                        // that had just been brought back from it in the
                        // air where the fall had got to, on nothing, for
                        // good.
                        let ours = self.player_reports.iter().any(|&(cell, local)| {
                            cell == up.position.cell
                                && local.distance(up.position.local) < SAME_REPORT
                        });
                        self.player_echo = Some(up.position);
                        if !reconcile(
                            gap,
                            server_step,
                            moved_by_server,
                            ours,
                            &mut self.desync_streak,
                        ) {
                            return Applied::Ignored;
                        }
                        tracing::info!(
                            "server moved the player to {:#010x} {:?} (gap {gap:.0} m)",
                            up.position.cell,
                            up.position.local
                        );
                    }
                    o.position = Some(up.position);
                    if o.display.is_none() {
                        o.display = Some(up.position);
                    }
                    if up.flags & 0x01 != 0 {
                        o.velocity = up.velocity;
                    }
                    // The server's own position supersedes local prediction.
                    o.target = None;
                    self.generation += 1;
                    if is_player {
                        Applied::PlayerMoved
                    } else {
                        Applied::Moved
                    }
                } else {
                    Applied::Ignored
                };
                if matches!(applied, Applied::PlayerMoved) {
                    self.arrived_in(up.position.cell);
                }
                applied
            }
            Err(e) => {
                tracing::warn!("UpdatePosition: {e}");
                Applied::Failed
            }
        }
    }

    pub(super) fn apply_movement_event(&mut self, body: &[u8]) -> Applied {
        match MovementEvent::parse(body) {
            Ok(ev) => {
                self.bring_back(ev.guid);
                let Some(o) = self.objects.get_mut(&ev.guid) else {
                    return Applied::Ignored;
                };
                o.motion = Motion {
                    forward: ev.forward,
                    forward_speed: ev.forward_speed,
                    style: ev.style,
                    run_rate: ev.run_rate,
                };
                o.target = ev.target;
                if let Some(MoveTarget::Object(at)) = ev.target {
                    o.walked_at = Some(at);
                }
                for &(cmd, seq, speed) in &ev.commands {
                    if o.commands.push(cmd, seq, speed) {
                        tracing::debug!(
                            "{:#010x} plays command {cmd:#06x} (seq {seq:#x}, speed {speed})",
                            ev.guid
                        );
                    }
                }
                if let (Some(h), Some(p)) = (ev.desired_heading, o.position.as_mut()) {
                    if !matches!(ev.target, Some(MoveTarget::Position { .. })) {
                        p.rotation = heading_quat(h);
                    }
                }
                self.generation += 1;
                if Some(ev.guid) == self.player_guid {
                    // A turn to face something is a motion, not a
                    // walk (see `MovementEvent::turn_to`).
                    if ev.target.is_some() {
                        Applied::PlayerMoveTo
                    } else {
                        Applied::PlayerMotion
                    }
                } else {
                    Applied::Moved
                }
            }
            Err(e) => {
                tracing::warn!("MovementEvent: {e}");
                Applied::Failed
            }
        }
    }

    pub(super) fn apply_vector_update(&mut self, body: &[u8]) -> Applied {
        match object::VectorUpdate::parse(body) {
            Ok(v) => {
                let Some(o) = self.objects.get_mut(&v.guid) else {
                    return Applied::Ignored;
                };
                o.velocity = v.velocity;
                self.generation += 1;
                Applied::Moved
            }
            Err(e) => {
                tracing::warn!("VectorUpdate: {e}");
                Applied::Failed
            }
        }
    }
}
