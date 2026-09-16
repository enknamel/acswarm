//! The magic family: casting a spell by name or id, and restocking what casts
//! burn.

use super::{nothing_named, resolve, Outcome, Refused, SpellRef, Target};
use crate::autoplay::cast_problem;
use crate::magic::CastCheck;
use crate::Client;

pub(super) fn cast(c: &mut Client, spell: &SpellRef, at: Option<&Target>) -> Outcome {
    let id = resolve::spell(c, spell).ok_or_else(|| match spell {
        SpellRef::Name(n) => Refused::ours(format!("no spell called {n:?}")),
        SpellRef::Id(id) => Refused::ours(format!("no spell {id}")),
    })?;
    let check = match at {
        Some(t) => {
            let guid = resolve::object(c, t).ok_or_else(|| nothing_named(t))?;
            c.cast_at(id, guid)
        }
        None => c.try_cast(id),
    };
    match check {
        CastCheck::Ok => Ok(()),
        other => Err(Refused::ours(cast_problem(&other))),
    }
}

pub(super) fn fill_components(c: &mut Client) -> Outcome {
    if c.fill_components() == 0 {
        return Err(Refused::ours("nothing to restock at this counter"));
    }
    Ok(())
}
