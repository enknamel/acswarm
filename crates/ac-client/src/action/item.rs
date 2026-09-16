//! The item family: using, taking, dropping, handing over, packing, splitting,
//! pouring and salvaging what the character can reach.

use super::{nothing_named, resolve, Outcome, Refused, Target};
use crate::Client;

pub(super) fn use_thing(c: &mut Client, target: &Target) -> Outcome {
    let guid = resolve::object(c, target).ok_or_else(|| nothing_named(target))?;
    c.select(Some(guid));
    c.interact(guid);
    Ok(())
}

pub(super) fn use_on(c: &mut Client, item: &Target, target: &Target) -> Outcome {
    let item = resolve::pack_item(c, item).ok_or_else(|| nothing_named(item))?;
    let on = resolve::object(c, target).ok_or_else(|| nothing_named(target))?;
    done(c.use_on(item, on), "the server would not use it on that")
}

pub(super) fn take(c: &mut Client, target: &Target) -> Outcome {
    let guid = resolve::object(c, target).ok_or_else(|| nothing_named(target))?;
    let open = c
        .world
        .open_container
        .as_ref()
        .is_some_and(|(_, items)| items.contains(&guid));
    if !open {
        return Err(Refused::ours("that is not in the open container"));
    }
    c.take(guid);
    Ok(())
}

pub(super) fn take_all(c: &mut Client) -> Outcome {
    let items: Vec<u32> = c
        .world
        .open_container
        .as_ref()
        .map(|(_, items)| items.clone())
        .unwrap_or_default();
    if items.is_empty() {
        return Err(Refused::ours("nothing in the open container"));
    }
    for guid in items {
        c.take(guid);
    }
    Ok(())
}

pub(super) fn drop_thing(c: &mut Client, target: &Target) -> Outcome {
    let guid = resolve::pack_item(c, target).ok_or_else(|| nothing_named(target))?;
    done(c.drop_item(guid), "that cannot be dropped")
}

pub(super) fn give(c: &mut Client, item: &Target, to: &Target, amount: Option<u32>) -> Outcome {
    let item = resolve::pack_item(c, item).ok_or_else(|| nothing_named(item))?;
    let to = resolve::object(c, to).ok_or_else(|| nothing_named(to))?;
    done(c.give(to, item, amount), "that cannot be given")
}

pub(super) fn put_in(c: &mut Client, item: &Target, container: &Target) -> Outcome {
    let item = resolve::pack_item(c, item).ok_or_else(|| nothing_named(item))?;
    let container = resolve::object(c, container).ok_or_else(|| nothing_named(container))?;
    done(
        c.put_in_container(item, container),
        "that does not go in there",
    )
}

pub(super) fn split(c: &mut Client, item: &Target, amount: u32) -> Outcome {
    let item = resolve::pack_item(c, item).ok_or_else(|| nothing_named(item))?;
    done(
        c.split_stack(item, None, amount),
        "that stack will not split",
    )
}

pub(super) fn merge(c: &mut Client, from: &Target, to: &Target) -> Outcome {
    let from = resolve::pack_item(c, from).ok_or_else(|| nothing_named(from))?;
    let to = resolve::pack_item(c, to).ok_or_else(|| nothing_named(to))?;
    done(c.merge_stacks(from, to, None), "those stacks will not pour")
}

pub(super) fn salvage(c: &mut Client, items: &[Target]) -> Outcome {
    let mut guids = Vec::with_capacity(items.len());
    for t in items {
        guids.push(resolve::pack_item(c, t).ok_or_else(|| nothing_named(t))?);
    }
    done(c.salvage(&guids), "the Ust would not take those")
}

pub(super) fn appraise(c: &mut Client, target: &Target) -> Outcome {
    let guid = resolve::object(c, target).ok_or_else(|| nothing_named(target))?;
    c.appraise(guid);
    Ok(())
}

pub(super) fn close_container(c: &mut Client) -> Outcome {
    if c.world.open_container.is_none() {
        return Err(Refused::ours("no container is open"));
    }
    c.close_container();
    Ok(())
}

/// What a `Client` method that answers with a bool came to.
fn done(sent: bool, why: &str) -> Outcome {
    if sent {
        Ok(())
    } else {
        Err(Refused::ours(why))
    }
}
