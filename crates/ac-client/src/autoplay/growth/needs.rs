use std::time::{Duration, Instant};

use super::town_run::vendor::answers_need;
use super::Growth;
use crate::items::ItemStats;
use crate::Client;

/// What the character is short of is worked out at most this often
/// while it waits for something to do.
const NEEDS_EVERY: Duration = Duration::from_secs(5);

/// Something the character is short of.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Need {
    /// What it is, for the log.
    pub(super) name: String,
    /// How many more to buy.
    pub(super) want: u32,
    /// How many are carried, and how many the rules ask for. The
    /// shortfall alone does not say how close to empty a line is, and
    /// that is what decides whether the party stops hunting.
    pub(super) have: u32,
    pub(super) keep: u32,
    /// Whether it is short enough to be worth a run to town.
    pub(super) urgent: bool,
    /// Whether enough of the world sells it to go and buy it. What is
    /// not is farmed instead: the Void components, and in practice the
    /// Diamond Scarab, whose one seller is a curiosity shop. A line
    /// like that must never drive a trip and must never be counted
    /// against the character's supplies, or a caster is out of stock
    /// for ever and the party restocks for ever.
    pub(super) buyable: bool,
    /// The counter the player named for this line, if they named one.
    ///
    /// A want that says where it comes from is filled there and nowhere
    /// else: fletching supplies come in levels and elements that one
    /// bowyer carries and the next does not, and healing kits come in
    /// levels. Without this the field was editable, saved to the
    /// profile and read by nothing.
    pub(super) from: Option<String>,
    pub(super) kind: NeedKind,
}

impl Need {
    /// Whether this counter is one this line may be bought at. A line
    /// that names none is bought wherever it is sold.
    pub(super) fn may_buy_at(&self, shop: &str) -> bool {
        match &self.from {
            None => true,
            Some(want) => shop.eq_ignore_ascii_case(want.trim()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum NeedKind {
    /// Stock whose name contains this.
    Named(String),
    /// Ammunition of this kind (`ac_world::fletching::ammo_type`).
    Ammo(u32),
}

impl Client {
    /// What the character is short of.
    pub(super) fn grow_needs(&self, cfg: &Growth) -> Vec<Need> {
        self.grow_needs_with(cfg, &self.item_stats())
    }

    /// The same, over a pack already read: the counter's snapshot reads
    /// it once for the sale and once more here, every tick of the
    /// shopping loop, and the second reading is the same pack.
    fn grow_needs_with(&self, _cfg: &Growth, stats: &[ItemStats]) -> Vec<Need> {
        let mut needs = Vec::new();
        let leaving = self.leaving_of(stats);
        let ammo = self.ammo_carried();
        // What is held as stock: what is carried, less what is on its way out (both the character
        // and the party read it: `held_stock`). The ammunition line counts the launcher's rounds.
        let stock = |what: &str| self.held_stock(what, &leaving, ammo);
        let (me, my_name) = (self.wielder(), &self.world.stats.name);
        // The profile's buy list, through the profile's own reckoning of what is short: the whole
        // of what is bought.
        let named: Vec<(String, u32, u32, Option<String>, bool)> = self
            .profiles
            .get(&self.autoplay.config.loot.profile)
            .map(|p| {
                p.shortfall(stock, &me, my_name)
                    .into_iter()
                    .map(|s| {
                        (
                            s.want.what.clone(),
                            s.have,
                            s.want.keep,
                            s.want.from.clone(),
                            s.urgent,
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (name, have, least, from, urgent) in named {
            if name.trim().is_empty() || least == 0 {
                continue;
            }
            // The launcher's ammunition: of its kind, and nothing without a launcher in hand.
            if name.trim().eq_ignore_ascii_case(crate::profile::AMMUNITION) {
                if let Some((kind, _)) = ammo.filter(|_| have < least) {
                    needs.push(Need {
                        name: ac_world::fletching::ammo_type::name(kind).to_string(),
                        want: least - have,
                        have,
                        keep: least,
                        urgent,
                        buyable: true,
                        from,
                        kind: NeedKind::Ammo(kind),
                    });
                }
                continue;
            }
            if have < least
                && !needs
                    .iter()
                    .any(|n: &Need| n.kind == NeedKind::Named(name.clone()))
            {
                needs.push(Need {
                    name: name.clone(),
                    want: least - have,
                    have,
                    keep: least,
                    // Being one short is not a reason to walk to town:
                    // the line says how low it may get first.
                    urgent,
                    buyable: true,
                    from,
                    kind: NeedKind::Named(name),
                });
            }
        }
        // Nothing else is bought: the profile's buy list is the whole of what to stock, components
        // and ammunition included (test: only_the_profiles_buy_list_is_shopped_for).
        needs
    }

    /// Roughly what filling a need will cost, for working out who has
    /// to be handed money before the party can shop. A vendor sells
    /// above an item's own value, so this is an underestimate rather
    /// than a promise; it is only ever compared against a purse.
    pub(super) fn rough_cost(&self, need: &Need) -> u32 {
        let unit = self
            .world
            .inventory()
            .find(|o| o.name.eq_ignore_ascii_case(&need.name) && o.value > 0)
            .map(|o| o.value)
            .unwrap_or(0);
        unit.saturating_mul(need.want)
    }

    /// What the town-run rule last found short, and how many seconds ago it looked: telemetry's
    /// record of supplies, and of a rule that never got its turn.
    pub fn supplies_seen(&self, now: Instant) -> Option<(f32, Vec<crate::tally::SupplyLine>)> {
        let (t, needs) = self.autoplay.growth.needs_seen.as_ref()?;
        let lines = needs
            .iter()
            .map(|n| crate::tally::SupplyLine {
                name: n.name.clone(),
                have: n.have,
                keep: n.keep,
                urgent: n.urgent,
            })
            .collect();
        Some((now.saturating_duration_since(*t).as_secs_f32(), lines))
    }

    /// What the character is short of, worked out afresh at most once
    /// every [`NEEDS_EVERY`]. Counting the pack is not free and the
    /// answer does not change between frames.
    pub(super) fn needs_now(&mut self, now: Instant, cfg: &Growth) -> Vec<Need> {
        if let Some(n) = self
            .autoplay
            .growth
            .needs_seen
            .as_ref()
            .filter(|(t, _)| now.duration_since(*t) < NEEDS_EVERY)
            .map(|(_, n)| n.clone())
        {
            return n;
        }
        let n = self.grow_needs(cfg);
        self.autoplay.growth.needs_seen = Some((now, n.clone()));
        n
    }

    /// What the character is short of, as the shopping rules want it:
    /// a weenie class, a name and how many more to buy.
    ///
    /// Only lines that can actually be bought. What no counter stocks
    /// is farmed instead, and a want nobody can fill would hold a trip
    /// open for ever.
    #[cfg(test)]
    pub(crate) fn vendor_shortfall(&self, cfg: &Growth) -> Vec<ac_vendor::counter::Want> {
        self.vendor_shortfall_at(cfg, &self.item_stats(), &[])
    }

    /// The same at a counter whose shelf holds `wares`: named stock and ammunition are known on a
    /// shelf by name, and taken to the ware the trip planner chose the counter for (`answers_need`),
    /// or the trip was made for what the counter was never asked for.
    pub(crate) fn vendor_shortfall_at(
        &self,
        cfg: &Growth,
        stats: &[ItemStats],
        wares: &[ac_vendor::counter::Ware],
    ) -> Vec<ac_vendor::counter::Want> {
        self.grow_needs_with(cfg, stats)
            .into_iter()
            .filter(|n| n.buyable && n.want > 0)
            .filter_map(|n| {
                let needle = match &n.kind {
                    NeedKind::Named(t) => t.trim().to_lowercase(),
                    NeedKind::Ammo(_) => String::new(),
                };
                let wcid = wares
                    .iter()
                    .filter(|w| answers_need(&n.kind, &w.name, &needle))
                    .min_by_key(|w| w.price)?
                    .wcid;
                Some(ac_vendor::counter::Want {
                    wcid,
                    name: n.name,
                    short: n.want,
                    urgent: n.urgent,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
