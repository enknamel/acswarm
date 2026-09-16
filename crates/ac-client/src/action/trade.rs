//! The trade family: the secure trade window with another player, and buying
//! and selling at an open counter.

use super::{nothing_named, resolve, Outcome, Refused, Target};
use crate::Client;

pub(super) fn open(c: &mut Client, target: &Target) -> Outcome {
    let guid = resolve::object(c, target).ok_or_else(|| nothing_named(target))?;
    c.open_trade(guid);
    Ok(())
}

pub(super) fn add(c: &mut Client, target: &Target) -> Outcome {
    let guid = resolve::pack_item(c, target).ok_or_else(|| nothing_named(target))?;
    if c.add_to_trade(guid) {
        Ok(())
    } else {
        Err(Refused::ours("that cannot go on the table"))
    }
}

pub(super) fn accept(c: &mut Client) -> Outcome {
    c.accept_trade();
    Ok(())
}

pub(super) fn decline(c: &mut Client) -> Outcome {
    c.decline_trade();
    Ok(())
}

pub(super) fn reset(c: &mut Client) -> Outcome {
    c.reset_trade();
    Ok(())
}

pub(super) fn close(c: &mut Client) -> Outcome {
    c.close_trade();
    Ok(())
}

pub(super) fn buy(c: &mut Client, ware: &Target, amount: u32) -> Outcome {
    if c.world.open_vendor.is_none() {
        return Err(Refused::ours("no counter is open"));
    }
    let guid = resolve::ware(c, ware).ok_or_else(|| nothing_named(ware))?;
    c.buy_amount(guid, amount.max(1));
    Ok(())
}

pub(super) fn sell(c: &mut Client, item: &Target) -> Outcome {
    if c.world.open_vendor.is_none() {
        return Err(Refused::ours("no counter is open"));
    }
    let guid = resolve::pack_item(c, item).ok_or_else(|| nothing_named(item))?;
    c.sell(guid);
    Ok(())
}

pub(super) fn close_vendor(c: &mut Client) -> Outcome {
    if c.world.open_vendor.is_none() {
        return Err(Refused::ours("no counter is open"));
    }
    c.close_vendor();
    Ok(())
}
