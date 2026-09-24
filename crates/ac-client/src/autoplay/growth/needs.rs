use std::time::{Duration, Instant};

use super::town_run::vendor::{answers_need, worth_stocking};
use super::Growth;
use crate::items::ItemStats;
use crate::Client;
use ac_world::item_type;

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
    /// A spell component, by the weenie class of the item.
    Component(u32),
}

impl Client {
    /// What the character is short of.
    pub(super) fn grow_needs(&self, cfg: &Growth) -> Vec<Need> {
        self.grow_needs_with(cfg, &self.item_stats())
    }

    /// The same, over a pack already read: the counter's snapshot reads
    /// it once for the sale and once more here, every tick of the
    /// shopping loop, and the second reading is the same pack.
    fn grow_needs_with(&self, cfg: &Growth, stats: &[ItemStats]) -> Vec<Need> {
        let mut needs = Vec::new();
        let leaving = self.leaving_of(stats);
        // What is held as stock: what is carried, less what is on its
        // way out. Both the character and the party read this count
        // (see `autoplay_stock`), so they agree on what is short.
        let stock = |what: &str| self.carried_named(what).saturating_sub(leaving.named(what));
        // The profile's buy list first: it is where a player says what
        // to keep stocked now, and it is the same list that makes those
        // things unsellable. `keep_stocked` is what it grew out of and
        // is still read, so nobody's settings go quiet.
        // The profile's buy list, through the profile's own reckoning
        // of what is short. `grow_needs` used to re-implement that
        // filter, which is two statements of one rule and the way they
        // come to disagree.
        let named: Vec<(String, u32, u32, Option<String>, bool)> = self
            .profiles
            .get(&self.autoplay.config.loot.profile)
            .map(|p| {
                p.shortfall(stock)
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
            if !worth_stocking(&name, self.heals_with_kits()) {
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
        if let Some((kind, carried)) = self.ammo_carried() {
            // Less the rounds the player said to sell: ammunition the
            // profile is selling out of is loot in the ammunition
            // slot, not stock, and it goes over the counter. The
            // launcher is still short by what is left after it does.
            let have = carried.saturating_sub(leaving.ammo);
            if have < cfg.ammo_keep {
                needs.push(Need {
                    name: ac_world::fletching::ammo_type::name(kind).to_string(),
                    want: cfg.ammo_keep - have,
                    have,
                    keep: cfg.ammo_keep,
                    urgent: have < cfg.ammo_keep / 4 && !self.can_craft_ammo(kind),
                    buyable: true,
                    from: None,
                    kind: NeedKind::Ammo(kind),
                });
            }
        }
        // Components: for a character with spells and a wand. Those it
        // carries, and those its buffs ask for that it has run out of.
        // How many tapers to carry, from the buy list. Everything else
        // a caster burns is scaled to it (see `component_targets`),
        // which is why one number buys forty kinds of thing.
        let tapers = self
            .profiles
            .get(&self.autoplay.config.loot.profile)
            .map_or(0, |p| p.stocked_count("Prismatic Taper"));
        if tapers > 0 && !self.world.stats.spells.is_empty() {
            let has_wand = self.wielded_caster().is_some()
                || self
                    .world
                    .inventory()
                    .any(|o| o.item_type & item_type::CASTER != 0);
            if has_wand {
                if let Ok(mapper) = self.assets.spell_component_ids() {
                    let targets = self.component_targets(tapers);
                    // Every component the spells this character casts
                    // will burn, and nothing else.
                    //
                    // The targets are what matters. Asking only what is
                    // in the pack, as this once did, leaves a caster
                    // that has run right out of something unable to
                    // notice: with none of it carried there is nothing
                    // to count, so nothing is short, so it never goes
                    // to town for more.
                    //
                    // And what is carried is not asked at all. It used
                    // to be added to the targets, at the taper count
                    // for want of a burn rate, and the peas a mage
                    // loots sit in the component table too (seventy-odd
                    // of them, at 113-186, 189 and 191): each became a
                    // need for ninety-nine more. A thing no cast burns
                    // is not stock, whatever table it is in, and the
                    // formulas already say which things are burnt. The
                    // spells autoplay casts, not every spell in the
                    // book, so nothing is bought for a spell that is
                    // never cast.
                    //
                    // That is a rule about what to stock, and it is not
                    // what keeps the player's loot off this list. The
                    // profile does: a stack tagged to sell is not
                    // counted as stock even when a spell burns it, and
                    // a kind the rules would tag to sell the moment it
                    // was bought is not bought, because a need for it
                    // would only buy back at markup what the counter
                    // was just handed, to be sold again next trip.
                    let carried = self.components();
                    for (&id, &keep) in &targets {
                        let Some(wcid) = mapper.component_wcid(id) else {
                            continue;
                        };
                        // What this one burns at, not what a taper does.
                        let c = carried.iter().find(|c| c.component_id == id);
                        let have = c.map_or(0, |c| c.count).saturating_sub(leaving.of(wcid));
                        let buyable = ac_world::shops::sold_anywhere(wcid);
                        if have < keep {
                            // What it is called in the client's own
                            // component table, which is what a vendor
                            // and a person both call it. The enum name
                            // is the last resort: "LeadScarab" is a
                            // symbol, not a thing you can ask for.
                            let name = c
                                .map(|c| c.name.clone())
                                .or_else(|| {
                                    self.assets
                                        .spell_components()
                                        .ok()
                                        .and_then(|t| t.get(id).map(|c| c.name.clone()))
                                })
                                .or_else(|| mapper.name_of(id).map(str::to_string))
                                .unwrap_or_else(|| format!("component {id}"));
                            if self.bought_to_sell(wcid, &name, keep - have, have) {
                                continue;
                            }
                            needs.push(Need {
                                name,
                                want: keep - have,
                                have,
                                keep,
                                urgent: have < keep / 4 && buyable,
                                buyable,
                                from: None,
                                kind: NeedKind::Component(wcid),
                            });
                        }
                    }
                }
            }
        }
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
                let wcid = match &n.kind {
                    NeedKind::Component(wcid) => *wcid,
                    kind => {
                        let needle = match kind {
                            NeedKind::Named(t) => t.trim().to_lowercase(),
                            _ => String::new(),
                        };
                        wares
                            .iter()
                            .filter(|w| answers_need(kind, &w.name, w.wcid, &needle))
                            .min_by_key(|w| w.price)?
                            .wcid
                    }
                };
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
