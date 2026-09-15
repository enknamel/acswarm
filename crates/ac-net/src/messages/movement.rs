use super::motion;
use crate::wire::Writer;

/// A position as the client reports it: cell id, local origin, orientation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WirePosition {
    pub cell: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub qw: f32,
    pub qx: f32,
    pub qy: f32,
    pub qz: f32,
}

fn write_position(w: &mut Writer, p: &WirePosition) {
    w.u32(p.cell)
        .f32(p.x)
        .f32(p.y)
        .f32(p.z)
        .f32(p.qw)
        .f32(p.qx)
        .f32(p.qy)
        .f32(p.qz);
}

/// Body of the AutonomousPosition action: where the client says it is.
/// `contact` is true when standing on the ground.
pub fn autonomous_position(p: &WirePosition, instance_seq: u16, contact: bool) -> Vec<u8> {
    let mut w = Writer::new();
    write_position(&mut w, p);
    w.u16(instance_seq).u16(0).u16(0).u16(0);
    w.u8(contact as u8);
    w.align4();
    w.finish()
}

/// Jump (0xF61B) body: extent (charge 0..=1), launch velocity in the
/// character's local frame, then the instance/control/teleport/force
/// sequences.
pub fn jump(power: f32, velocity: [f32; 3], instance_seq: u16) -> Vec<u8> {
    let mut w = Writer::new();
    w.f32(power)
        .f32(velocity[0])
        .f32(velocity[1])
        .f32(velocity[2])
        .u16(instance_seq)
        .u16(0)
        .u16(0)
        .u16(0)
        // Trailing object guid and spell id the server reads (unused).
        .u32(0)
        .u32(0);
    w.finish()
}

/// The raw input state the client reports in MoveToState.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RawMotion {
    pub running: bool,
    /// `motion::WALK_FORWARD`, `WALK_BACKWARDS`, or 0 when idle.
    pub forward: u32,
    /// `motion::SIDE_STEP_LEFT/RIGHT` or 0.
    pub sidestep: u32,
    /// `motion::TURN_LEFT/RIGHT` or 0.
    pub turn: u32,
    /// One-shot commands (emotes) played this update, as full
    /// MotionCommand ids; the server relays them to everyone in view.
    pub commands: Vec<u32>,
}

/// Body of the MoveToState action: input state plus position.
pub fn move_to_state(m: &RawMotion, p: &WirePosition, instance_seq: u16, contact: bool) -> Vec<u8> {
    const CURRENT_HOLD_KEY: u32 = 0x1;
    const CURRENT_STYLE: u32 = 0x2;
    const FORWARD_COMMAND: u32 = 0x4;
    const FORWARD_SPEED: u32 = 0x10;
    const SIDESTEP_COMMAND: u32 = 0x20;
    const SIDESTEP_SPEED: u32 = 0x80;
    const TURN_COMMAND: u32 = 0x100;
    const TURN_SPEED: u32 = 0x400;
    let mut flags = CURRENT_STYLE;
    if m.running {
        flags |= CURRENT_HOLD_KEY;
    }
    if m.forward != 0 {
        flags |= FORWARD_COMMAND | FORWARD_SPEED;
    }
    if m.sidestep != 0 {
        flags |= SIDESTEP_COMMAND | SIDESTEP_SPEED;
    }
    if m.turn != 0 {
        flags |= TURN_COMMAND | TURN_SPEED;
    }
    let mut w = Writer::new();
    // The command list length lives above the 11 flag bits.
    w.u32(flags | ((m.commands.len() as u32) << 11));
    if m.running {
        w.u32(motion::HOLD_KEY_RUN);
    }
    w.u32(motion::STANCE_NON_COMBAT);
    if m.forward != 0 {
        w.u32(m.forward).f32(1.0);
    }
    if m.sidestep != 0 {
        w.u32(m.sidestep).f32(1.0);
    }
    if m.turn != 0 {
        w.u32(m.turn).f32(1.0);
    }
    for (i, cmd) in m.commands.iter().enumerate() {
        // Raw command (the low 16 bits), packed sequence with the
        // autonomous bit, speed; ACE only takes emotes at speed 1.
        w.u16((cmd & 0xFFFF) as u16)
            .u16(0x8000 | (i as u16 & 0x7FFF))
            .f32(1.0);
    }
    write_position(&mut w, p);
    w.u16(instance_seq).u16(0).u16(0).u16(0);
    w.u8(contact as u8);
    w.align4();
    w.finish()
}
