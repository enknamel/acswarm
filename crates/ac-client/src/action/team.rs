//! The team family: the switches a person throws over how the character plays
//! on its own -- autoplay itself, hunting with the others, leading, following
//! and fighting.

use super::Outcome;
use crate::autoplay::Role;
use crate::Client;

pub(super) fn autoplay(c: &mut Client, on: bool) -> Outcome {
    c.autoplay.config.enabled = on;
    Ok(())
}

pub(super) fn team(c: &mut Client, on: bool) -> Outcome {
    c.autoplay.config.team.enabled = on;
    Ok(())
}

/// Leading is playing as a team, so switching it on switches that on too.
pub(super) fn lead(c: &mut Client, on: bool) -> Outcome {
    c.autoplay.config.team.lead = on;
    if on {
        c.autoplay.config.team.enabled = true;
    }
    Ok(())
}

pub(super) fn role(c: &mut Client, role: Role) -> Outcome {
    c.autoplay.config.team.role = role;
    Ok(())
}

pub(super) fn follow(c: &mut Client, on: bool) -> Outcome {
    c.autoplay.config.team.follow = on;
    if !on {
        c.follow = None;
    }
    Ok(())
}

pub(super) fn fight(c: &mut Client, on: bool) -> Outcome {
    c.autoplay.config.fight.enabled = on;
    if !on {
        c.attack_target = None;
    }
    Ok(())
}
