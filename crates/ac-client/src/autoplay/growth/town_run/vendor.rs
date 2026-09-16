use std::time::Instant;

use glam::Vec2;

use super::{Errand, VENDOR_REACH};
use crate::autoplay::growth::needs::{Need, NeedKind};
#[cfg(doc)]
use crate::autoplay::growth::sale::worth_a_sale_run;
use crate::autoplay::growth::sale::Salable;
use crate::autoplay::growth::{a_few, contains_fold, Growth};
use crate::Client;
use ac_world::{item_type, object_desc_flags};

/// How near a counter must stand to a *way out* -- a gem's exit, a
/// recall's landing, the character's own feet -- to be worth stopping
/// at on a run.
///
/// This used to be measured from the first counter of the run, which
/// meant one town and no further. But a run is not a walk around a
/// town: a mid-level character uses a Town Network gem, takes the
/// portal it summons, sells at the broker outside Cragstone, uses an
/// Archmage gem, takes that portal, restocks its components there, and
/// recalls back to where it was hunting. Every one of those counters is
/// a long way from the last one and a few paces from a way out, which
/// is the distance that actually costs anything.
pub(super) const NEAR_A_WAY_OUT: f32 = 300.0;

/// How far from a way out the first counter of a run made for loot
/// that merely adds up (see [`worth_a_sale_run`]) may stand: the town
/// the character is in, or the one its gem lands in. A pack that
/// cannot hunt on is walked anywhere; a few peas are not, or one Lead
/// Pea carried a quarter of an hour is a walk to an archmage three
/// towns over, and the same again for the next.
pub(super) const SALE_RUN_REACH: f32 = VENDOR_RINGS[0];

/// What the next counter of a run is chosen for, and among which (see
/// [`Client::pick_vendor`]).
#[derive(Clone, Copy)]
pub(super) struct Stop<'a> {
    pub(super) errand: Errand,
    /// No further than this from a way out, once a run is already out.
    pub(super) within: Option<f32>,
    /// The counters this run has already called at, by position.
    pub(super) visited: &'a [Vec2],
}

/// Where each carried gem can put the character, as ways out.
///
/// A gem that lands outdoors lands where it lands. One that lands
/// indoors -- the Town Network gem comes out in the hub -- is worth more
/// than its landing: nothing is sold in a hub and nobody can walk out of
/// one, but every portal standing in it is a few steps away and each of
/// those comes out somewhere. Judging the gem by the hub alone kept the
/// broker outside Cragstone off every run, gem in the pack or not.
///
/// The landing still counts too: a gem that comes out inside a building
/// leaves the rest of its landblock a walk away.
fn gem_ways(gems: &[ac_world::trip::Gem]) -> Vec<(Vec2, String)> {
    let mut out = Vec::new();
    for g in gems {
        out.push((g.exit, g.name.clone()));
        if g.exit_cell & 0xFFFF < 0x100 {
            continue;
        }
        for p in ac_world::portals::out_of(g.exit_cell) {
            out.push((p.to_xy(), format!("{}, then {}", g.name, p.name)));
        }
    }
    out
}

/// Which way out leaves the character nearest `at`, how far that is,
/// and what it is called.
pub(super) fn nearest_way(ways: &[(Vec2, String)], at: Vec2) -> (Vec2, f32, String) {
    ways.iter()
        .map(|(w, name)| (*w, at.distance(*w), name.clone()))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap_or((at, 0.0, "here".to_string()))
}

/// What a trip to one shop is expected to achieve, worked out from the
/// shop lists before the character sets off.
///
/// A run to town costs minutes of hunting, so it is worth knowing in
/// advance whether the counter has what the character came for and
/// whether there is money enough to pay for it. Without it the party
/// does what it used to: walk to a shop, find nothing it can afford,
/// and walk to the next one, for ever.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Forecast {
    /// The shop, for the log.
    pub shop: String,
    /// Needs it has on the shelf, and needs it has not.
    pub stocks: Vec<String>,
    pub missing: Vec<String>,
    /// What the needs it stocks would cost in full at its prices.
    pub bill: u32,
    /// The least one of anything wanted costs here; 0 when it stocks
    /// nothing that is wanted.
    pub cheapest: u32,
    /// Coin and notes in hand.
    pub purse: u32,
    /// What it would pay for what the character means to sell, and how
    /// many things that is.
    pub takings: u32,
    pub selling: usize,
}

impl Forecast {
    /// What there is to spend once the selling is done.
    pub fn funds(&self) -> u32 {
        self.purse.saturating_add(self.takings)
    }

    /// Why this shop was the one chosen, in a few words.
    ///
    /// The choice is a ring search outwards, taking the best shop in
    /// the first ring with anything worth the walk, ordered by whether
    /// it fills the whole order, then by how many lines of it it
    /// stocks, then by how near it is. So the answer is always some
    /// mixture of those three, and worth saying out loud: "it went
    /// there" is a bug report, "it went there because it was the only
    /// one stocking quarrels" is an explanation.
    pub fn why_it_was_picked(&self) -> String {
        if self.stocks.is_empty() {
            return match self.selling {
                0 => "nothing wanted is sold here; nearest counter".to_string(),
                n => format!("nowhere sells what is wanted; {n} thing(s) to sell"),
            };
        }
        let what = a_few(&self.stocks.iter().map(|s| s.as_str()).collect::<Vec<_>>());
        let covers = if self.covers_it() {
            "the whole order".to_string()
        } else if self.missing.is_empty() {
            format!("{what}, but short of coin")
        } else {
            format!(
                "{what} (not {})",
                a_few(&self.missing.iter().map(|s| s.as_str()).collect::<Vec<_>>())
            )
        };
        match self.selling {
            0 => covers,
            n => format!("{covers}; {n} thing(s) to sell here"),
        }
    }

    /// Whether the trip fills the whole order.
    pub fn covers_it(&self) -> bool {
        self.missing.is_empty() && !self.stocks.is_empty() && self.funds() >= self.bill
    }

    /// Whether the trip achieves anything at all: something to sell, or
    /// one thing it wants that it can pay for. Buying part of an order
    /// counts -- a mage that can afford half its tapers is a mage that
    /// can keep casting -- but buying none of it does not.
    pub fn worth_going(&self) -> bool {
        self.selling > 0 || (self.cheapest > 0 && self.funds() >= self.cheapest)
    }

    /// Said aloud, so whoever is watching knows why the party did or
    /// did not set off.
    pub fn tell(&self) -> String {
        let asked = self.stocks.len() + self.missing.len();
        let mut out = format!("{} of {asked} on the shelf", self.stocks.len());
        if self.bill > 0 {
            out.push_str(&format!(", {} to buy", self.bill));
        }
        out.push_str(&format!(", {} in hand", self.purse));
        if self.selling > 0 {
            out.push_str(&format!(", {} for {} item(s)", self.takings, self.selling));
        }
        if !self.missing.is_empty() {
            let missing: Vec<&str> = self.missing.iter().map(String::as_str).collect();
            out.push_str(&format!("; no {}", a_few(&missing)));
        }
        out
    }
}

/// The cheapest ware on this shelf that answers the need, if any.
fn shop_ware<'a>(
    shop: &'a ac_world::shops::Shop,
    need: &Need,
    needle: &str,
) -> Option<&'a ac_world::shops::Ware> {
    shop.sells
        .iter()
        .filter(|w| match &need.kind {
            NeedKind::Named(_) => contains_fold(&w.name, needle),
            NeedKind::Ammo(kind) => ammo_stock(&w.name, *kind),
            NeedKind::Component(wcid) => w.wcid == *wcid,
        })
        .min_by_key(|w| w.value)
}

/// What a trip to `shop` would buy, cost and fetch. `wants` is the
/// needs paired with their names already folded, since this is asked of
/// every counter in range.
fn forecast(
    shop: &ac_world::shops::Shop,
    wants: &[(&Need, String)],
    purse: u32,
    salables: &[Salable],
) -> Forecast {
    let mut f = Forecast {
        shop: shop.name.clone(),
        purse,
        ..Default::default()
    };
    for (need, needle) in wants {
        // A line that names its counter is only filled there. To every
        // other shop it reads as a line they do not carry, which is the
        // truth as far as this character is concerned, and the ranking
        // then sends it to the one that does.
        if !need.may_buy_at(&shop.name) {
            f.missing.push(need.name.clone());
            continue;
        }
        match shop_ware(shop, need, needle) {
            Some(w) => {
                let unit = shop.charges_for(w).max(1);
                f.bill = f.bill.saturating_add(unit.saturating_mul(need.want));
                f.cheapest = if f.cheapest == 0 {
                    unit
                } else {
                    f.cheapest.min(unit)
                };
                f.stocks.push(need.name.clone());
            }
            None => f.missing.push(need.name.clone()),
        }
    }
    for it in salables
        .iter()
        .filter(|it| it.taken_by(shop.buys, shop.min_value, shop.max_value))
    {
        // A counter's limits are about one of a thing, and its payment
        // is for all of them. Judging a stack by its total refuses a
        // hundred Pyreal Peas outright -- five million against a
        // ceiling of one -- and then reckons the character has nothing
        // to sell, which is how it came to walk thirty kilometres past
        // a counter that would have bought them twenty at a time.
        if let Some(paid) = shop.pays_for(it.item_type, it.each()) {
            f.selling += 1;
            f.takings = f
                .takings
                .saturating_add(paid.saturating_mul(it.stack.max(1)));
        }
    }
    f
}

/// Where a counter stands, to the metre: what a skipped vendor is
/// remembered by. Two vendors a metre apart are the same counter for
/// this purpose, which is the same slack the old comparison allowed.
pub(super) fn spot(at: Vec2) -> (i32, i32) {
    (at.x.round() as i32, at.y.round() as i32)
}

/// Whether a stock line is plain ammunition of `kind`: an "Arrow", not
/// a "Bundle of Arrowheads" or a "Fire Arrow" (the plain kind is what
/// is cheap and always there).
fn ammo_stock(name: &str, kind: u32) -> bool {
    use ac_world::fletching::ammo_type;
    let plain = match kind {
        ammo_type::ARROW => "arrow",
        ammo_type::BOLT => "quarrel",
        ammo_type::ATLATL => "atlatl dart",
        _ => return false,
    };
    name.trim().to_lowercase() == plain
}

/// What the vendor charges for an item of `value` (ACE's SellPrice), at
/// least a pyreal.
/// Whether a name on the keep-stocked list is worth buying. A healing
/// kit does nothing for a character that has not trained Healing -- it
/// restores next to nothing untrained -- so buying one wastes the money
/// and the trip. Such a character heals with a spell instead.
pub(crate) fn worth_stocking(name: &str, heals_with_kits: bool) -> bool {
    if heals_with_kits {
        return true;
    }
    !name.to_lowercase().contains("healing kit")
}

/// How far to look for a shop before settling for a nearer one with
/// less on its shelves: the town we are in, the towns around it, then a
/// long walk, then anywhere at all.
const VENDOR_RINGS: [f32; 3] = [600.0, 3_000.0, 15_000.0];

/// The distances to search, widening, never past `within`.
///
/// The last one is `within` itself (or everything), so a character with
/// nothing nearby still finds a shop rather than standing still.
fn vendor_rings(within: Option<f32>) -> Vec<f32> {
    let cap = within.unwrap_or(f32::INFINITY);
    let mut out: Vec<f32> = VENDOR_RINGS.iter().copied().filter(|r| *r < cap).collect();
    out.push(cap);
    out
}

/// Which of two counters is worth walking to, better first.
///
/// The whole order in one stop beats part of it, then more of the order
/// beats less, then what the counter *pays* -- which used not to be
/// asked at all. What a shop gives for an item is `value * buy_rate`
/// and the spread is nearly half: around Cragstone the Scriveners
/// eighty metres off pay 0.5 and the Arcanum Broker three hundred
/// metres further on pays 0.95, and ranking on distance alone walked
/// past the broker every time. Distance settles what is left.
///
/// That is the order for a trip to buy. A trip to sell turns it round:
/// what the counter pays for the loot comes first, then how much of
/// the loot it takes, and what it stocks only settles ties. Ranked the
/// buying way, a run made to empty a pack of peas went to whichever
/// counter had the most of the shopping list on its shelf, and the
/// peas came home.
fn better_counter(errand: Errand, a: (&Forecast, f32), b: (&Forecast, f32)) -> std::cmp::Ordering {
    let (fa, near_a) = a;
    let (fb, near_b) = b;
    let to_buy = || {
        fb.covers_it()
            .cmp(&fa.covers_it())
            .then_with(|| fb.stocks.len().cmp(&fa.stocks.len()))
            .then_with(|| fb.takings.cmp(&fa.takings))
    };
    match errand {
        Errand::Buy => to_buy(),
        Errand::Sell => fb
            .takings
            .cmp(&fa.takings)
            .then_with(|| fb.selling.cmp(&fa.selling))
            .then_with(to_buy),
    }
    .then_with(|| near_a.total_cmp(&near_b))
}

/// The best of these counters for the errand, each with how far off it
/// is, and the forecast it was chosen on; `None` when none of them
/// serves it. The heart of [`Client::pick_vendor`], with the walking
/// of the shop list and the character's own circumstances left to it,
/// so that the choice can be tested on its own.
fn choose_counter<'a>(
    counters: impl Iterator<Item = (&'a ac_world::shops::Shop, f32)>,
    errand: Errand,
    wants: &[(&Need, String)],
    purse: u32,
    salables: &[Salable],
) -> Option<(&'a ac_world::shops::Shop, Forecast)> {
    counters
        .map(|(s, reach)| (s, reach, forecast(s, wants, purse, salables)))
        .filter(|(_, _, f)| errand.served_by(f))
        .min_by(|(_, ra, fa), (_, rb, fb)| better_counter(errand, (fa, *ra), (fb, *rb)))
        .map(|(s, _, f)| (s, f))
}

impl Client {
    /// The best vendor to make for, given every way the character has
    /// of being somewhere else: not one being avoided, not one in
    /// `visited`, within `within` metres of a way out when given, and
    /// -- this is the point of it -- one the trip is known in advance
    /// to achieve something at.
    ///
    /// `ways` is where the character can cheaply be (see
    /// [`Self::ways_out`]); a shop is judged by its distance to the
    /// nearest of them.
    ///
    /// Comes back with the forecast it was chosen on, so the caller can
    /// decide whether to set off at all and can say why.
    ///
    /// `stop` says what the trip is for and where it may go. A trip to
    /// sell goes to a counter that pays for what is carried and to
    /// nothing else: the shops are ranked on what they pay, and there
    /// is no "any counter will do" when none of them buys any of it
    /// (see [`Errand`]).
    pub(super) fn pick_vendor(
        &self,
        cfg: &Growth,
        needs: &[Need],
        ways: &[(Vec2, String)],
        stop: Stop<'_>,
        now: Instant,
    ) -> Option<(String, Vec2, Forecast)> {
        let Stop {
            errand,
            within,
            visited,
        } = stop;
        // How far a shop is: from the nearest way out, not from the
        // feet.
        let reach = |at: Vec2| {
            ways.iter()
                .map(|(w, _)| at.distance(*w))
                .fold(f32::MAX, f32::min)
        };
        // Counters this character can actually trade at. A society's
        // archmage is a very good archmage and no use at all to
        // somebody else's society, and a chapter house behind a quest
        // portal is a journey to a door that will not open.
        let society = self.society();
        let quests = cfg.gates_open.clone();
        let skip = &self.autoplay.growth.skip_vendors;
        let allowed = |at: Vec2| {
            within.is_none_or(|w| reach(at) <= w)
                && !visited.iter().any(|p| p.distance(at) < 1.0)
                && !skip.held(&spot(at), now)
        };
        // Worked out once for the whole search rather than per shop:
        // this walks every counter in range.
        let wants: Vec<(&Need, String)> = needs
            .iter()
            .filter(|n| n.want > 0 && n.buyable)
            .map(|n| {
                let needle = match &n.kind {
                    NeedKind::Named(t) => t.trim().to_lowercase(),
                    _ => String::new(),
                };
                (n, needle)
            })
            .collect();
        let purse = self.spendable();
        let salables = self.salables(cfg);

        // A shop that has what the character came for is worth a longer
        // walk than one that does not: an archer out of quarrels is not
        // helped by the archmage next door. But only so much longer.
        // Ranking on what a shop stocks alone sent a character twenty-
        // five kilometres to a counter with one more line on the shelf,
        // so the search widens in rings and takes the best shop in the
        // first ring that has anything.
        // The counter the profile names, when it names one. A player
        // who has said where to sell has said it; the rings below are
        // for finding one when nobody has.
        let named = self.sell_to_named();
        if let Some(want) = named.as_deref() {
            // Named, but not exempt, and not the answer to every
            // question.
            //
            // `allowed` is what remembers the counters this run has
            // already emptied its pack at and the ones lately found to
            // be no use; without it a run spent every one of its stops
            // walking back to the same counter, because the name matches
            // just as well the second time.
            //
            // And it is only taken when the trip to it is worth making.
            // This is where a character goes to *sell*; returning it
            // whatever the forecast said made it the answer for buying
            // too, and the ring search -- the thing that finds an
            // archmage -- never ran. A mage out of tapers, with coin in
            // its purse and nothing in its pack worth selling, was sent
            // to a counter that stocks no components, told the trip was
            // not worth making, and sent there again on every run after.
            // It never bought tapers again.
            if let Some(found) = ac_world::shops::all()
                .iter()
                .filter(|s| s.open_to(society, &quests) && allowed(s.xy()))
                .find(|s| s.name.eq_ignore_ascii_case(want))
            {
                let f = forecast(found, &wants, purse, &salables);
                if errand.served_by(&f) {
                    return Some((found.name.clone(), found.xy(), f));
                }
            }
        }
        for ring in vendor_rings(within) {
            let counters = ac_world::shops::all()
                .iter()
                .filter(|s| allowed(s.xy()) && reach(s.xy()) <= ring)
                .filter(|s| s.open_to(society, &quests))
                .map(|s| (s, reach(s.xy())));
            if let Some((s, f)) = choose_counter(counters, errand, &wants, purse, &salables) {
                return Some((s.name.clone(), s.xy(), f));
            }
        }
        // A trip to sell has nowhere to go: no counter in reach buys any
        // of what is carried. Walking to one that does not is the trip
        // there and the trip back with the same pack, and there is no
        // nearest-counter fallback for it.
        if errand == Errand::Sell {
            return None;
        }
        // Nothing sells what is wanted, nothing is wanted at all, or
        // there is no money for any of it: any counter will do, which is
        // the case when the trip is to stop in town rather than to buy
        // something. The forecast comes back saying as much, and the
        // caller decides whether that is reason enough to walk.
        ac_world::landmarks::all()
            .iter()
            .filter(|l| l.kind == ac_world::landmarks::Kind::Vendor)
            .filter(|l| allowed(l.xy()))
            .min_by(|a, b| reach(a.xy()).total_cmp(&reach(b.xy())))
            .map(|l| {
                let look = Forecast {
                    shop: l.name.clone(),
                    missing: wants.iter().map(|(n, _)| n.name.clone()).collect(),
                    purse,
                    ..Default::default()
                };
                (l.name.clone(), l.xy(), look)
            })
    }

    /// Everywhere the character can get to cheaply, and what takes it
    /// there.
    ///
    /// A journey is not measured from the feet. A character carries
    /// ways of being somewhere else -- a lifestone recall, the two
    /// portal recalls, whatever gems are in the pack -- and each lands
    /// it at a known spot for the price of one cast. The counter worth
    /// going to is the one nearest *any* of those, not the one nearest
    /// where it happens to be standing.
    ///
    /// It matters most underground, where the feet are the one place
    /// that leads nowhere: a dungeon lies under the landblock it
    /// belongs to, so the nearest counter to a character in Holtburg
    /// Dungeon is one in Holtburg, a hundred metres up through rock.
    /// But it is just as true on the surface -- a recall to Arwic beats
    /// a two-kilometre walk to the shop over the hill.
    pub(super) fn ways_out(&self, me: Vec2) -> Vec<(Vec2, String)> {
        let mut out: Vec<(Vec2, String)> = Vec::new();
        // Where it stands, unless where it stands leads nowhere.
        if !self.autoplay.growth.in_dungeon {
            out.push((me, "on foot".to_string()));
        }
        let world_xy = |p: ac_world::Position| {
            let o = ac_world::landblock_origin(p.cell);
            Vec2::new(o.x + p.local.x, o.y + p.local.y)
        };
        for r in self.castable_recalls() {
            if let Some(p) = self.recall_destination(r.spell) {
                out.push((world_xy(p), r.name.clone()));
            }
        }
        // A gem needs no skill and no components: carrying one is the
        // whole requirement, which often makes it the cheapest way out
        // a character has.
        out.extend(gem_ways(&self.carried_gems()));
        // And the dungeon's own door. Every portal's mouth and its
        // landing are known, so a character underground can say which
        // dungeon it is in and where the way out comes up -- and be
        // judged on the shops near *that*, which is what walking out
        // would actually achieve.
        if self.autoplay.growth.in_dungeon {
            if let Some(pl) = self.player.as_ref() {
                let block = pl.cell & 0xFFFF_0000;
                for p in ac_world::portals::out_of(block) {
                    out.push((p.to_xy(), format!("{} (the way out)", p.name)));
                }
            }
        }
        // Nothing to hand: the feet are all there is, wherever they are.
        if out.is_empty() {
            out.push((me, "on foot".to_string()));
        }
        out
    }

    /// Which society this character belongs to, as `Faction1Bits`: 1
    /// the Celestial Hand, 2 the Eldrytch Web, 4 the Radiant Blood, 0
    /// none. The server sends it with the rest of the character's
    /// properties, so this is knowledge rather than the player's word
    /// for it -- and it is what the societies' own counters ask for.
    pub fn society(&self) -> u32 {
        const FACTION1_BITS: u32 = 281;
        self.world
            .stats
            .ints
            .iter()
            .find(|(k, _)| *k == FACTION1_BITS)
            .map(|(_, v)| (*v).max(0) as u32)
            .unwrap_or(0)
    }

    /// The vendor object nearest `at`, by name when one matches.
    pub(super) fn vendor_object(&self, name: &str, at: Vec2) -> Option<u32> {
        let vendors: Vec<(f32, bool, u32)> = self
            .world
            .objects
            .values()
            .filter(|o| {
                o.object_desc_flags & object_desc_flags::VENDOR != 0
                    || (o.item_type & item_type::CREATURE != 0
                        && o.object_desc_flags & object_desc_flags::ATTACKABLE == 0
                        && !o.is_player
                        && o.name == name)
            })
            .filter_map(|o| {
                let p = o.world_pos()?;
                let d = Vec2::new(p.x, p.y).distance(at);
                (d <= VENDOR_REACH + 10.0).then_some((d, o.name == name, o.guid))
            })
            .collect();
        vendors
            .iter()
            .filter(|(_, named, _)| *named)
            .chain(vendors.iter())
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, _, g)| *g)
    }
}

#[cfg(test)]
mod tests;
