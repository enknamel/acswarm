use std::collections::BTreeMap;
use std::time::Duration;

use super::{fate, never_sell, never_sell_carried, offer_to_vendor, Fate, Growth};
use crate::autoplay::LootAction;
use crate::items::ItemStats;
use crate::Client;
use ac_world::{equip, item_type};

/// Whether what the pack holds for a counter is reason enough for a
/// run to town, and the reason if it is.
///
/// A run used to be made for a full pack, a heavy one or a supply run
/// short, and for nothing else: what the loot rules had tagged for a
/// counter never came into it, so two peas taken to sell sat in a
/// roomy pack for ever and the character never went. The ledger says
/// why each thing was taken; this is where "to sell" is acted on.
///
/// `sale` is what the selling rules would let go today (see
/// [`Client::salables`]), and `carried_for` how long any of it has
/// been in the pack. Three rules, any one enough: it is worth
/// `sell_run_value` at face, there are `sell_run_count` things, or it
/// has been carried for `sell_run_patience`. Each is off at 0.
///
/// This is only the reason. The waits between runs and after a futile
/// one are the caller's ([`Client::grow_town_run`]), which is what
/// keeps a lone character from wearing a path to town for one pea.
pub(super) fn worth_a_sale_run(
    sale: &[Salable],
    carried_for: Option<Duration>,
    cfg: &Growth,
) -> Option<String> {
    if sale.is_empty() {
        return None;
    }
    let worth: u32 = sale.iter().fold(0u32, |sum, s| sum.saturating_add(s.value));
    let count = sale.len() as u32;
    if cfg.sell_run_value > 0 && worth >= cfg.sell_run_value {
        return Some(format!(
            "carrying {worth} pyreals' worth for a counter ({count} thing(s))"
        ));
    }
    if cfg.sell_run_count > 0 && count >= cfg.sell_run_count {
        return Some(format!("carrying {count} things for a counter"));
    }
    // The config is hand-edited JSON: a patience no Duration can hold
    // (1e20, say) reads as the rule being off, not as a panic on every
    // frame a run could start.
    if let Some(patience) = Some(cfg.sell_run_patience)
        .filter(|p| *p > 0.0)
        .and_then(|p| Duration::try_from_secs_f32(p).ok())
    {
        if let Some(d) = carried_for.filter(|d| *d >= patience) {
            return Some(format!(
                "{count} thing(s) for a counter carried {} min",
                d.as_secs() / 60
            ));
        }
    }
    None
}

/// Something in the pack the rules allow to be sold, before any one
/// counter's tastes are applied to it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Salable {
    pub guid: u32,
    /// `ac_world::item_type` bits.
    pub item_type: u32,
    /// What the whole stack is worth. The server counts a stack's value
    /// as the lot, not as one of them, and a counter's limit is on that
    /// figure -- which is why a hundred Pyreal Peas worth five million
    /// are refused by a counter that will not look at anything over a
    /// million.
    pub value: u32,
    pub stack: u32,
}

impl Salable {
    /// What one of them is worth.
    pub fn each(&self) -> u32 {
        self.value / self.stack.max(1)
    }

    /// How many of them a counter with this ceiling will take at once.
    /// Zero when it will not take even one.
    ///
    /// A stack worth more than the ceiling is not refused for good: it
    /// is split and sold a handful at a time, which is what a player
    /// does and what the counter's limit is for.
    pub fn at_once(&self, max_value: u32) -> u32 {
        if max_value == 0 {
            return self.stack.max(1);
        }
        let each = self.each().max(1);
        (max_value / each).min(self.stack.max(1))
    }

    /// Whether a counter that buys `item_types` between `min_value` and
    /// `max_value` (a maximum of 0 being no maximum) would take it,
    /// whole or in pieces.
    pub(super) fn taken_by(&self, item_types: u32, min_value: u32, max_value: u32) -> bool {
        self.item_type & item_types != 0 && self.each() >= min_value && self.at_once(max_value) > 0
    }
}

/// What is on its way out of the pack (see [`Client::leaving_of`]),
/// counted: how many of each kind, so that the rest of a kind can
/// still be short.
///
/// Counted, not merely noted. This once held only which kinds were
/// leaving, and a line or a component was dropped from the restock
/// list whenever any stack of it was: one stack of scarabs tagged to
/// sell hid the kept stack's shortfall, a Sell-tagged "Lead Scarab"
/// muted a buy line for "Scarab", and the party heard a different
/// count than the character used. Taking the leaving stacks off what
/// is held answers all three.
#[derive(Default)]
pub(crate) struct Leaving {
    /// How many of each weenie class are leaving.
    by_wcid: BTreeMap<u32, u32>,
    /// Each leaving stack's name, lower-cased, with its count, for the
    /// buy list's lines, which name a thing the way `carried_named`
    /// counts it: by what its name contains.
    names: Vec<(String, u32)>,
    /// How many rounds of ammunition are leaving.
    pub(super) ammo: u32,
}

impl Leaving {
    /// How many of this weenie class are leaving.
    pub(super) fn of(&self, wcid: u32) -> u32 {
        self.by_wcid.get(&wcid).copied().unwrap_or(0)
    }

    /// How many of what a buy-list line names are leaving.
    pub(crate) fn named(&self, line: &str) -> u32 {
        let want = line.trim().to_lowercase();
        if want.is_empty() {
            return 0;
        }
        self.names
            .iter()
            .filter(|(n, _)| n.contains(&want))
            .map(|(_, count)| *count)
            .sum()
    }
}

/// What the sale decision needs, gathered once (see
/// [`Client::sell_policy`]).
pub(crate) struct SellPolicy {
    burns: Vec<u32>,
    keep: Vec<String>,
    profile: Option<std::sync::Arc<crate::profile::Profile>>,
    wielder: crate::weapons::Wielder,
    me: String,
}

impl Client {
    /// What is on its way out of the pack: every carried thing the
    /// profile said to sell and the server will let go (see
    /// [`ac_loot::sale::fate`]), counted by weenie class and by name.
    ///
    /// What is held is counted less this before anything is called
    /// short. A thing that is leaving is not stock, whatever else says
    /// it is -- the buy list, the component table, the formulas of the
    /// spells this character casts -- because the player decided it.
    /// It is not the whole of the answer, though: what leaves is
    /// forgotten with it, and whether more of the kind should then be
    /// bought is a question about the kind, which the profile answers
    /// (see [`Client::bought_to_sell`]).
    pub(crate) fn leaving_of(&self, stats: &[ItemStats]) -> Leaving {
        let mut out = Leaving::default();
        for s in stats {
            if fate(self.autoplay.ledger.of(s), never_sell(s)) != Fate::Leaving {
                continue;
            }
            let count = s.stack.max(1);
            *out.by_wcid.entry(s.wcid).or_insert(0) += count;
            out.names.push((s.name.to_lowercase(), count));
            if s.valid_locations & equip::MISSILE_AMMO != 0 {
                out.ammo += count;
            }
        }
        out
    }

    /// Whether `count` of this spell component, bought at a counter,
    /// would be written down as loot to sell the moment they arrived
    /// -- in which case buying them is a round trip at the counter's
    /// markup, and they are not stock.
    ///
    /// Asked of the profile the way the arrival pass will ask it
    /// (`autoplay_tag_arrivals`): the same rules, over the thing a
    /// purchase arrives as. A verdict about the *kind* rather than
    /// about a stack in the pack, which is what the restock list
    /// needs: the moment a tagged stack goes over the counter its tag
    /// is forgotten with it, and a list that only asked "is any of
    /// this leaving?" bought the scarabs straight back in the same
    /// visit, the arrival pass tagged them to sell, and the next trip
    /// sold them again.
    ///
    /// `held` is what will be held when the purchase arrives, less
    /// anything leaving: the count a `keep_up_to` rule, or the buy
    /// list's own line, is judged against.
    pub(super) fn bought_to_sell(&self, wcid: u32, name: &str, count: u32, held: u32) -> bool {
        let Some(profile) = self.loot_profile() else {
            return false;
        };
        // Worth what a counter lists it at: the one in front of the
        // character, else the world's shops. A rule that sells by value
        // is asked about the stack a purchase makes.
        let each = self
            .world
            .open_vendor
            .as_ref()
            .and_then(|v| {
                v.items
                    .iter()
                    .find(|w| w.desc.weenie_class_id == wcid)
                    .map(|w| w.desc.value)
            })
            .or_else(|| {
                ac_world::shops::all()
                    .iter()
                    .flat_map(|s| &s.sells)
                    .find(|w| w.wcid == wcid)
                    .map(|w| w.value)
            })
            .unwrap_or(0);
        let count = count.max(1);
        let arrives = ItemStats {
            name: name.to_string(),
            wcid,
            item_type: item_type::SPELL_COMPONENTS,
            kind: crate::items::kind_name(item_type::SPELL_COMPONENTS),
            stack: count,
            max_stack: 1_000,
            value: each.saturating_mul(count),
            ..Default::default()
        };
        matches!(
            crate::autoplay::judge_loot(
                &arrives,
                None,
                Some(&profile),
                &self.wielder(),
                &self.world.stats.name,
                held,
            ),
            crate::profile::Verdict::Decided(LootAction::Sell, _)
        )
    }

    /// Everything the character is carrying that the rules would sell,
    /// whether or not a vendor is open. What the party hands its
    /// quartermaster before it leaves.
    pub fn loot_for_sale(&self, cfg: &Growth) -> Vec<u32> {
        // The same judgement a counter is handed. This was a third one:
        // it asked `sellable`, which knows nothing of the buy list and
        // has no rule for a character with no profile -- so the
        // quartermaster was handed exactly the two things the rest of
        // this was written to hold back, and sold them on the party's
        // behalf.
        let policy = self.sell_policy(cfg);
        self.world
            .inventory()
            .filter_map(|o| {
                let stats = self.stats_of(o.guid)?;
                let ammo = o.valid_locations & equip::MISSILE_AMMO != 0;
                self.offers_for_sale(&policy, &stats, ammo)
                    .then_some(o.guid)
            })
            .collect()
    }

    /// Every name that is never sold: the player's own list, whatever
    /// is kept stocked, and whatever the loot rules always keep.
    pub(crate) fn keep_names(&self) -> Vec<String> {
        // The player's own word, and nothing else: what the character
        // keeps stocked is barred from sale by the buy list itself
        // (`Profile::stocks`, passed separately), which used to be said
        // here a second time.
        self.loot_profile()
            .map(|p| p.looting.always.clone())
            .unwrap_or_default()
    }

    /// The pack items to sell to the open vendor, which takes only some
    /// kinds of thing. Weapons are only sold once appraised and found
    /// beyond the character.
    pub(super) fn sale_list(&self, cfg: &Growth) -> Vec<u32> {
        let Some(v) = self.world.open_vendor.as_ref() else {
            return Vec::new();
        };
        // The vendor buys some kinds of thing, within a range of values
        // (a range of 0 is no range at all).
        self.salables(cfg)
            .into_iter()
            .filter(|it| it.taken_by(v.item_types, v.min_value, v.max_value))
            .map(|it| it.guid)
            .collect()
    }

    /// Everything the sale decision needs, worked out once for a whole
    /// pack rather than per item.
    pub(crate) fn sell_policy(&self, cfg: &Growth) -> SellPolicy {
        SellPolicy {
            burns: self.burns(cfg),
            keep: self.keep_names(),
            profile: self.profiles.get(&self.autoplay.config.loot.profile),
            wielder: self.wielder(),
            me: self.world.stats.name.clone(),
        }
    }

    /// Whether this carried thing goes over a counter.
    ///
    /// One answer, asked both by the forecast before the character sets
    /// off and by the snapshot the counter in front of it is handed.
    /// They were two, and they disagreed: the snapshot's had no
    /// fallback for a character with no profile, so the forecast said
    /// the trip was worth making and then nothing was offered.
    pub(crate) fn offers_for_sale(&self, p: &SellPolicy, s: &ItemStats, ammo: bool) -> bool {
        offer_to_vendor(
            s,
            ammo,
            &p.burns,
            &p.keep,
            p.profile.as_ref().is_some_and(|x| x.stocks(&s.name)),
            self.autoplay.ledger.of(s),
            // The profile is the source of truth. A character nobody
            // has given one to sells nothing, which is the right way for
            // this to be empty-handed: the alternative is a client
            // deciding on its own what somebody's things are worth.
            || match &p.profile {
                Some(pr) => matches!(
                    pr.judge(s, self.appraisals.get(&s.guid), &p.wielder, &p.me, 0),
                    crate::profile::Verdict::Decided(LootAction::Sell, _)
                ),
                None => false,
            },
        )
    }

    /// The counter the loot profile names for selling, if it names
    /// one. The policy is [`Profile::sell_to_named`](ac_loot::profile::Profile::sell_to_named); this is the
    /// lookup.
    pub(super) fn sell_to_named(&self) -> Option<String> {
        let loot = &self.autoplay.config.loot;
        let p = self.profiles.get(&loot.profile)?;
        p.sell_to_named().map(str::to_string)
    }

    /// Everything in the pack the selling rules allow to go, before any
    /// one counter's tastes are applied to it.
    ///
    /// [`Self::sale_list`] narrows this to the vendor standing in front
    /// of the character; a forecast narrows it to a shop the character
    /// has not walked to yet, which is how a trip is judged before it is
    /// started.
    pub(super) fn salables(&self, cfg: &Growth) -> Vec<Salable> {
        self.for_sale(cfg)
            .into_iter()
            .map(|o| Salable {
                guid: o.guid,
                item_type: o.item_type,
                value: o.value,
                stack: o.stack_size.max(1),
            })
            .collect()
    }

    /// The carried things a counter would be offered, as they lie in the
    /// pack: what the selling rules let go, less what no counter takes.
    fn for_sale(&self, cfg: &Growth) -> Vec<&ac_world::WorldObject> {
        // The same judgement the counter is handed (see
        // [`Client::offers_for_sale`]). It used to be a second one, and
        // the two disagreed.
        let policy = self.sell_policy(cfg);
        // A pack with things in it is not loot to be sold; the server
        // refuses it, and it holds the character's belongings. Which
        // packs hold anything is worked out once, not once per item:
        // this is asked on every turn spent over a corpse (see
        // [`Self::loot_burden`]).
        let holders: std::collections::BTreeSet<u32> = self
            .world
            .objects
            .values()
            .filter_map(|o| o.container)
            .collect();
        self.world
            .inventory()
            .filter(|o| {
                let Some(stats) = self.stats_of(o.guid) else {
                    return false;
                };
                let ammo = o.valid_locations & equip::MISSILE_AMMO != 0;
                !never_sell_carried(&stats, holders.contains(&o.guid))
                    && self.offers_for_sale(&policy, &stats, ammo)
            })
            .collect()
    }

    /// What the loot being carried weighs: everything a counter would be
    /// offered. A stack's burden is the whole stack's already, and a
    /// pack with anything in it is never offered, so nothing is counted
    /// twice.
    pub(super) fn loot_burden(&self, cfg: &Growth) -> u32 {
        self.for_sale(cfg)
            .iter()
            .fold(0u32, |sum, o| sum.saturating_add(o.burden))
    }
}

#[cfg(test)]
mod tests;
