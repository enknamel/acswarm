use ac_net::wire::Reader;

use crate::{Applied, Trade, World};

impl World {
    pub(super) fn apply_register_trade(&mut self, rest: &[u8]) -> Applied {
        let mut r = Reader::new(rest);
        match (r.u32(), r.u32()) {
            (Ok(initiator), Ok(partner)) => {
                let stamp = r.u64().unwrap_or(0) as i64;
                let me = self.player_guid.unwrap_or(0);
                let other = if initiator == me { partner } else { initiator };
                self.trade = Some(Trade {
                    partner: other,
                    initiator,
                    stamp,
                    ..Default::default()
                });
                self.generation += 1;
                Applied::Trade
            }
            _ => Applied::Failed,
        }
    }

    pub(super) fn apply_close_trade(&mut self) -> Applied {
        self.trade = None;
        self.generation += 1;
        Applied::Trade
    }

    pub(super) fn apply_add_to_trade(&mut self, rest: &[u8]) -> Applied {
        let mut r = Reader::new(rest);
        match (r.u32(), r.u32()) {
            (Ok(item), Ok(side)) => {
                if let Some(t) = self.trade.as_mut() {
                    let list = if side == 1 {
                        &mut t.mine
                    } else {
                        &mut t.theirs
                    };
                    if !list.contains(&item) {
                        list.push(item);
                    }
                    t.i_accepted = false;
                    t.they_accepted = false;
                }
                Applied::Trade
            }
            _ => Applied::Failed,
        }
    }

    pub(super) fn apply_remove_from_trade(&mut self, rest: &[u8]) -> Applied {
        let mut r = Reader::new(rest);
        if let (Ok(item), Some(t)) = (r.u32(), self.trade.as_mut()) {
            t.mine.retain(|&g| g != item);
            t.theirs.retain(|&g| g != item);
            t.i_accepted = false;
            t.they_accepted = false;
        }
        Applied::Trade
    }

    pub(super) fn apply_accept_trade(&mut self, rest: &[u8]) -> Applied {
        let who = Reader::new(rest).u32().unwrap_or(0);
        let me = self.player_guid;
        if let Some(t) = self.trade.as_mut() {
            if Some(who) == me {
                t.i_accepted = true;
            } else {
                t.they_accepted = true;
            }
        }
        Applied::Trade
    }

    pub(super) fn apply_decline_trade(&mut self, rest: &[u8]) -> Applied {
        let who = Reader::new(rest).u32().unwrap_or(0);
        let me = self.player_guid;
        if let Some(t) = self.trade.as_mut() {
            if Some(who) == me {
                t.i_accepted = false;
            } else {
                t.they_accepted = false;
            }
        }
        Applied::Trade
    }

    pub(super) fn apply_reset_trade(&mut self) -> Applied {
        if let Some(t) = self.trade.as_mut() {
            t.mine.clear();
            t.theirs.clear();
            t.i_accepted = false;
            t.they_accepted = false;
        }
        Applied::Trade
    }

    pub(super) fn apply_clear_trade_acceptance(&mut self) -> Applied {
        if let Some(t) = self.trade.as_mut() {
            t.i_accepted = false;
            t.they_accepted = false;
        }
        Applied::Trade
    }

    pub(super) fn apply_trade_failure(&mut self, rest: &[u8]) -> Applied {
        let mut r = Reader::new(rest);
        if let (Ok(item), Ok(reason), Some(t)) = (r.u32(), r.u32(), self.trade.as_mut()) {
            t.failure = Some((item, reason));
        }
        Applied::Trade
    }
}
