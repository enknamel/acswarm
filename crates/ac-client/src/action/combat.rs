//! The combat family: what is attacked, what is held, and what is selected.

use super::{nothing_named, resolve, Outcome, Refused, Target};
use crate::Client;

/// Attack it: weapons out first, since the server drops an attack sent in
/// peace mode, then use it, which swings at what can be attacked.
pub(super) fn attack(c: &mut Client, target: &Target) -> Outcome {
    let guid = resolve::object(c, target).ok_or_else(|| nothing_named(target))?;
    if !c.combat {
        c.toggle_combat();
    }
    c.select(Some(guid));
    c.interact(guid);
    Ok(())
}

pub(super) fn cancel_attack(c: &mut Client) -> Outcome {
    c.cancel_attack();
    Ok(())
}

pub(super) fn combat(c: &mut Client, on: bool) -> Outcome {
    if c.combat != on {
        c.toggle_combat();
    }
    if !on {
        c.attack_target = None;
    }
    Ok(())
}

pub(super) fn wield(c: &mut Client, target: &Target) -> Outcome {
    let guid = resolve::pack_item(c, target).ok_or_else(|| nothing_named(target))?;
    if c.wield_guid(guid) {
        Ok(())
    } else {
        Err(Refused::ours("that cannot be wielded now"))
    }
}

pub(super) fn select(c: &mut Client, target: Option<&Target>) -> Outcome {
    let guid = match target {
        Some(t) => Some(resolve::object(c, t).ok_or_else(|| nothing_named(t))?),
        None => None,
    };
    c.select(guid);
    Ok(())
}
