//! The fellow family: the server's own fellowship, and the questions it asks
//! about joining one.

use super::{nothing_named, resolve, Outcome, Refused, Target};
use crate::Client;

pub(super) fn create(c: &mut Client, name: &str, share_xp: bool) -> Outcome {
    if name.trim().is_empty() {
        return Err(Refused::ours("a fellowship wants a name"));
    }
    c.fellowship_create(name, share_xp);
    Ok(())
}

pub(super) fn recruit(c: &mut Client, target: &Target) -> Outcome {
    let guid = resolve::object(c, target).ok_or_else(|| nothing_named(target))?;
    c.fellowship_recruit(guid);
    Ok(())
}

pub(super) fn dismiss(c: &mut Client, target: &Target) -> Outcome {
    let guid = resolve::object(c, target).ok_or_else(|| nothing_named(target))?;
    c.fellowship_dismiss(guid);
    Ok(())
}

pub(super) fn quit(c: &mut Client, disband: bool) -> Outcome {
    if c.world.fellowship.is_none() {
        return Err(Refused::ours("not in a fellowship"));
    }
    c.fellowship_quit(disband);
    Ok(())
}

/// Answer the question the server is waiting on, oldest first.
pub(super) fn confirm(c: &mut Client, yes: bool) -> Outcome {
    let Some(q) = c.world.confirmations.first().cloned() else {
        return Err(Refused::ours("nothing was asked"));
    };
    c.confirm(q.kind, q.context, yes);
    Ok(())
}
