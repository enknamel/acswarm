//! Deliveries: items on mules brought to a character of this process. A request (from the Items
//! window, a script, the bus) names the receiver's account and each item's mule and guid; every
//! mule holding some is logged in, hands over those items only, and the receiver goes from one
//! to the next. An item is delivered once the mule carried it and no longer does.

use serde::{Deserialize, Serialize};

use super::{session_of, Mules};
use crate::Ctx;

/// Blackboard key (this process): a list of [`Request`]s, taken when read.
pub const DELIVER_KEY: &str = "mules.deliver";
/// Blackboard key (this process): the roster's mules as `{host, account, character}`.
pub const ACCOUNTS_KEY: &str = "mules.accounts";

/// Bring `items` to the character logged in on account `to`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub to: String,
    pub items: Vec<Wanted>,
}

/// One item on a mule: its account and the item's guid.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wanted {
    pub account: String,
    pub guid: u32,
}

/// A delivery under way.
#[derive(Clone, Debug, Default)]
pub(super) struct Delivery {
    /// The receiver's account.
    to: String,
    /// (mule account, item guid) still to come.
    pub(super) items: Vec<(String, u32)>,
    /// Items seen on their mule since the delivery began: gone from it, they were handed over.
    seen: Vec<u32>,
    /// Looks an item never seen was missing from its mule in the world: at [`MISSING_LOOKS`]
    /// it is not there (a stale snapshot), and struck off.
    missing: std::collections::BTreeMap<u32, u32>,
}

/// Looks (2 s each) an item may be missing from its mule before it is struck off.
const MISSING_LOOKS: u32 = 5;

impl Mules {
    /// New requests off the blackboard, each joined to the receiver's delivery under way.
    pub(super) fn take_requests(&mut self, cx: &mut Ctx) {
        let Some(v) = cx.board.get(DELIVER_KEY).cloned() else {
            return;
        };
        cx.board.set_local(DELIVER_KEY, serde_json::Value::Null);
        let Ok(asked) = serde_json::from_value::<Vec<Request>>(v) else {
            return;
        };
        for r in asked {
            let known: Vec<(String, u32)> = r
                .items
                .into_iter()
                .filter(|w| self.roster.iter().any(|m| m.account == w.account))
                .map(|w| (w.account, w.guid))
                .collect();
            tracing::info!("mules: {} item(s) asked for, for {}", known.len(), r.to);
            // Kept from the moment they are asked for: none is loaded back on its way in.
            let kept = self.delivered.entry(r.to.clone()).or_default();
            for (_, g) in &known {
                if !kept.contains(g) {
                    kept.push(*g);
                }
            }
            match self.deliveries.iter_mut().find(|d| d.to == r.to) {
                Some(d) => {
                    for k in known {
                        if !d.items.contains(&k) {
                            d.items.push(k);
                        }
                    }
                }
                None if !known.is_empty() => self.deliveries.push(Delivery {
                    to: r.to,
                    items: known,
                    ..Default::default()
                }),
                None => {}
            }
        }
    }

    /// Each delivery: what has come struck off, every mule of it told what to hand over, and the
    /// receiver sent to the first that stands ready; over once nothing is left.
    pub(super) fn tend_deliveries(&mut self, cx: &mut Ctx) {
        let mut deliveries = std::mem::take(&mut self.deliveries);
        deliveries.retain_mut(|d| self.tend_delivery(cx, d));
        self.deliveries = deliveries;
    }

    /// One delivery tended; false once it is over.
    fn tend_delivery(&mut self, cx: &mut Ctx, d: &mut Delivery) -> bool {
        let Some(r) = cx.clients.iter().position(|c| c.config.account == d.to) else {
            tracing::warn!(
                "mules: the delivery for {} is dropped, it is not logged in",
                d.to
            );
            return false;
        };
        let receiver = cx.clients[r].world.stats.name.trim().to_string();
        let mut mules: Vec<String> = Vec::new();
        for (m, _) in &d.items {
            if !mules.contains(m) {
                mules.push(m.clone());
            }
        }
        for m in &mules {
            let Some(host) = self
                .roster
                .iter()
                .find(|x| &x.account == m)
                .map(|x| x.host.clone())
            else {
                continue;
            };
            let Some(s) = session_of(cx, &host, m).filter(|s| cx.clients[*s].is_mule_holding())
            else {
                continue;
            };
            let mule = &mut cx.clients[s];
            let carried = |g: u32| mule.world.is_carried(g);
            for (_, g) in d.items.iter().filter(|(a, _)| a == m) {
                if carried(*g) && !d.seen.contains(g) {
                    d.seen.push(*g);
                }
            }
            let seen = d.seen.clone();
            let came: Vec<u32> = d
                .items
                .iter()
                .filter(|(a, g)| a == m && !mule.world.is_carried(*g) && seen.contains(g))
                .map(|(_, g)| *g)
                .collect();
            for (_, g) in d
                .items
                .iter()
                .filter(|(a, g)| a == m && !d.seen.contains(g))
            {
                *d.missing.entry(*g).or_default() += 1;
            }
            let mut gone: Vec<u32> = d
                .missing
                .iter()
                .filter(|(_, n)| **n >= MISSING_LOOKS)
                .map(|(g, _)| *g)
                .collect();
            // What the server will not let go of (attuned, no room) is not coming.
            gone.extend(
                d.items
                    .iter()
                    .filter(|(a, g)| a == m && mule.is_give_refused(*g))
                    .map(|(_, g)| *g),
            );
            if !gone.is_empty() {
                tracing::warn!("mules: {} item(s) asked of {m} are not on it", gone.len());
            }
            d.items
                .retain(|(a, g)| a != m || !(came.contains(g) || gone.contains(g)));
            let items: Vec<u32> = d
                .items
                .iter()
                .filter(|(a, _)| a == m)
                .map(|(_, g)| *g)
                .collect();
            mule.set_mule_delivery((!items.is_empty()).then(|| ac_client::autoplay::Delivery {
                to: receiver.clone(),
                items,
            }));
        }
        let next = mules
            .iter()
            .filter(|m| d.items.iter().any(|(a, _)| a == *m))
            .find_map(|m| self.place_of(cx, m));
        cx.clients[r].set_mule_fetch(next);
        if d.items.is_empty() {
            tracing::info!("mules: the delivery for {receiver} has all come");
            return false;
        }
        true
    }

    /// A mule that cannot be had: its items struck off every delivery.
    pub(super) fn drop_mule_deliveries(&mut self, cx: &mut Ctx, account: &str) {
        for d in &mut self.deliveries {
            let before = d.items.len();
            d.items.retain(|(a, _)| a != account);
            if d.items.len() != before {
                tracing::warn!(
                    "mules: {} item(s) for {} cannot come",
                    before - d.items.len(),
                    d.to
                );
            }
        }
        let (done, left): (Vec<_>, Vec<_>) = std::mem::take(&mut self.deliveries)
            .into_iter()
            .partition(|d| d.items.is_empty());
        self.deliveries = left;
        for d in done {
            if let Some(r) = cx.clients.iter().position(|c| c.config.account == d.to) {
                cx.clients[r].set_mule_fetch(None);
            }
        }
    }
}
