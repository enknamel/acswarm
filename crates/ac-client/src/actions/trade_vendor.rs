use std::time::Instant;

use crate::Client;

impl Client {
    /// Ask another player to trade (OpenTradeNegotiations 0x01F6). Both
    /// must be in peace mode and close by; the server answers with
    /// RegisterTrade for both (`world.trade`).
    pub fn open_trade(&mut self, player: u32) {
        self.interrupt_travel("trading");
        use ac_net::messages::action;
        tracing::info!("trade with {player:#010x}");
        self.session
            .send_action(action::OPEN_TRADE_NEGOTIATIONS, &player.to_le_bytes());
    }

    /// Put a carried item in the trade window (AddToTrade 0x01F8).
    pub fn add_to_trade(&mut self, item: u32) -> bool {
        use ac_net::messages::action;
        let Some(t) = self.world.trade.as_ref() else {
            return false;
        };
        let slot = t.mine.len() as u32;
        let mut w = ac_net::wire::Writer::new();
        w.u32(item).u32(slot);
        self.session.send_action(action::ADD_TO_TRADE, &w.finish());
        true
    }

    /// Accept the offers as they stand (AcceptTrade 0x01FA: partner, the
    /// trade stamp, status, initiator, both acceptance flags; the server
    /// only cares that it arrived).
    pub fn accept_trade(&mut self) {
        let Some(t) = self.world.trade.as_ref() else {
            return;
        };
        let me = self.world.player_guid.unwrap_or(0);
        let (i_am_initiator, they) = (t.initiator == me, t.they_accepted);
        let mut w = ac_net::wire::Writer::new();
        w.u32(t.partner)
            .f64(t.stamp as f64)
            .u32(0)
            .u32(t.initiator)
            .u32(u32::from(if i_am_initiator { true } else { they }))
            .u32(u32::from(if i_am_initiator { they } else { true }));
        self.session
            .send_action(ac_net::messages::action::ACCEPT_TRADE, &w.finish());
    }

    pub fn decline_trade(&mut self) {
        self.session
            .send_action(ac_net::messages::action::DECLINE_TRADE, &[]);
    }

    /// Take everything back out of the window (ResetTrade 0x0204).
    pub fn reset_trade(&mut self) {
        self.session
            .send_action(ac_net::messages::action::RESET_TRADE, &[]);
    }

    pub fn close_trade(&mut self) {
        self.session
            .send_action(ac_net::messages::action::CLOSE_TRADE_NEGOTIATIONS, &[]);
        self.world.trade = None;
    }

    /// Stop looking into the open ground container.
    pub fn close_container(&mut self) {
        use ac_net::messages::action;
        if let Some((c, _)) = self.world.open_container.take() {
            self.session
                .send_action(action::NO_LONGER_VIEWING_CONTENTS, &c.to_le_bytes());
        }
    }

    /// Buy one of a vendor's stock items.
    pub fn buy(&mut self, guid: u32) {
        self.buy_amount(guid, 1);
    }

    /// Buy `amount` of a vendor's stock item (a stack for stackables).
    pub fn buy_amount(&mut self, guid: u32, amount: u32) {
        use ac_net::messages::{action, trade};
        let Some(vendor) = self.world.open_vendor.as_ref().map(|v| v.vendor) else {
            return;
        };
        if amount == 0 {
            return;
        }
        tracing::info!("buy {amount} x {guid:#010x} from {vendor:#010x}");
        self.session
            .send_action(action::BUY, &trade(vendor, &[(guid, amount as i32)]));
        // The counter answers with `UseDone`; until it does, what was handed
        // over is still in the pack, and reading that as a refusal is how a
        // run left a counter before its sale landed.
        self.autoplay.cast_sent = Some(Instant::now());
    }

    /// Sell several pack items (each its whole stack) to the open
    /// vendor in one go.
    ///
    /// The sell action has always carried a list and the server has
    /// always handled one; sending them singly was a round trip per
    /// dagger, and a round trip is where a run finds new ways to lose
    /// track of what it has offered.
    pub fn sell_many(&mut self, guids: &[u32]) {
        use ac_net::messages::{action, trade};
        let Some(vendor) = self.world.open_vendor.as_ref().map(|v| v.vendor) else {
            return;
        };
        let lot: Vec<(u32, i32)> = guids
            .iter()
            .filter_map(|g| {
                let o = self.world.objects.get(g)?;
                Some((*g, o.stack_size.max(1) as i32))
            })
            .collect();
        if lot.is_empty() {
            return;
        }
        tracing::info!("sell {} item(s) to {vendor:#010x}", lot.len());
        self.session.send_action(action::SELL, &trade(vendor, &lot));
        // The counter answers with `UseDone`; until it does, what was handed
        // over is still in the pack, and reading that as a refusal is how a
        // run left a counter before its sale landed.
        self.autoplay.cast_sent = Some(Instant::now());
    }

    /// Sell a pack item (its whole stack) to the open vendor.
    pub fn sell(&mut self, guid: u32) {
        use ac_net::messages::{action, trade};
        let Some(vendor) = self.world.open_vendor.as_ref().map(|v| v.vendor) else {
            return;
        };
        let amount = self
            .world
            .objects
            .get(&guid)
            .map(|o| o.stack_size.max(1) as i32)
            .unwrap_or(1);
        tracing::info!("sell {guid:#010x} to {vendor:#010x}");
        self.session
            .send_action(action::SELL, &trade(vendor, &[(guid, amount)]));
        // The counter answers with `UseDone`; until it does, what was handed
        // over is still in the pack, and reading that as a refusal is how a
        // run left a counter before its sale landed.
        self.autoplay.cast_sent = Some(Instant::now());
    }

    pub fn close_vendor(&mut self) {
        self.world.open_vendor = None;
    }
}
