//! The pack, purse and counter as plain values for the vendoring rules.
//! Nothing here reaches a world, socket or character, so a trip is decided and replayed offline
//! exactly as it runs live.

use std::collections::BTreeMap;

pub use ac_loot::LootAction;

/// One thing in the pack, as the shopping needs to see it.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Item {
    pub guid: u32,
    /// Weenie class id; two stacks join only when these match.
    pub wcid: u32,
    pub name: String,
    /// Worth of the whole stack, as the server counts it and a counter's `max_value` measures it.
    pub value: u32,
    /// Burden of the whole stack; a side pack's includes everything inside it.
    pub burden: u32,
    pub stack: u32,
    /// Largest the stack may grow to; whether two things combine is this, never the count held.
    pub max_stack: u32,
    /// `ac_world::item_type` bits.
    pub item_type: u32,
    /// The container it sits in, or `None` for the main pack.
    pub pack: Option<u32>,
    pub wielded: bool,
    /// Why it must not be sold, whatever any rule says.
    pub keep: Keep,
    /// The ledger's word since pickup; `Sell` beats the shopping list ([`Snapshot::offers`]).
    /// Stacks pour together only when words agree: one stack carries one word (`Run::compress`).
    pub taken_for: Option<LootAction>,
}

/// Reasons an item is not for sale, whatever a profile says; no policy overrides them.
/// Tinkered/inscribed: somebody's work; equipped: worn; retained/unsellable: the server's word.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Keep {
    pub tinkered: bool,
    pub inscribed: bool,
    pub equipped: bool,
    pub retained: bool,
    pub unsellable: bool,
    /// Kept for the character: taken to keep or, undecided, a stocked spell component or the
    /// focus that halves them. Policy: `Snapshot::offers` spares a wanted kind only while short.
    pub mine: bool,
}

impl Keep {
    /// What forbids selling it, if anything.
    pub fn why(&self) -> Option<&'static str> {
        if self.tinkered {
            Some("it has been tinkered")
        } else if self.inscribed {
            Some("it has been inscribed")
        } else if self.equipped {
            Some("it is being worn")
        } else if self.retained {
            Some("it is retained")
        } else if self.unsellable {
            Some("the server marks it unsellable")
        } else if self.mine {
            Some("the character lives on it")
        } else {
            None
        }
    }

    pub fn forbidden(&self) -> bool {
        self.why().is_some()
    }
}

impl Item {
    /// The player's word that it goes.
    pub fn to_sell(&self) -> bool {
        self.taken_for == Some(LootAction::Sell)
    }

    /// What one of them is worth.
    pub fn each(&self) -> u32 {
        self.value / self.stack.max(1)
    }

    pub fn stackable(&self) -> bool {
        self.max_stack > 1
    }

    /// How many more it can take.
    pub fn room(&self) -> u32 {
        self.max_stack.saturating_sub(self.stack)
    }
}

/// Something a counter has on its shelf.
#[derive(Clone, Debug, PartialEq)]
pub struct Ware {
    pub wcid: u32,
    pub name: String,
    /// What one costs here, the counter's markup included.
    pub price: u32,
    /// How many it has, or `None` for a shelf that never empties.
    pub stock: Option<u32>,
    /// Burden of one, or 0 when the counter did not say; caps how many can be carried home.
    pub burden: u32,
    /// What one is worth, and the counter's rate on it (a note's is the server's own): together
    /// they price a stack, which is one item to the server (0 when unknown).
    pub value: u32,
    pub sell_rate: f32,
}

impl Ware {
    /// What `count` of it cost bought at once. The server charges each item it creates
    /// ceil(rate x value - 0.1) (Vendor.cs:536-541, 577-585), and a stack is one item whose value
    /// is the lot: 150 tapers at 1.55 x 22 are 5,115, not 150 x 34. The counter does not say
    /// which kind a ware is, so the dearer of the two is the bill.
    pub fn bill(&self, count: u32) -> u32 {
        let each = self.price.saturating_mul(count);
        let lot = self.value.saturating_mul(count) as f32;
        let stack = (self.sell_rate * lot - 0.1).ceil().max(0.0) as u32;
        each.max(stack)
    }

    /// The most of it `purse` pays for, by [`Ware::bill`].
    pub fn affordable(&self, purse: u32) -> u32 {
        if self.price == 0 {
            return 0;
        }
        // The stack's charge passes `price` times the count by at most a pyreal in ten, so this
        // steps down a handful of times at most.
        let mut n = purse / self.price;
        while n > 0 && self.bill(n) > purse {
            n -= 1;
        }
        n
    }
}

/// The counter being stood at.
#[derive(Clone, Debug, PartialEq)]
pub struct Counter {
    pub guid: u32,
    pub name: String,
    /// The window is open and it will trade.
    pub open: bool,
    /// `ac_world::item_type` bits it will take.
    pub buys: u32,
    /// The most it will pay for one thing; 0 for no limit.
    pub max_value: u32,
    pub wares: Vec<Ware>,
    /// Face of the trade note it sells, if any; the only note worth making, as coin takes slots.
    pub note_face: Option<u32>,
    /// That note's own weenie on this shelf: bought by this, never by a price near its face.
    pub note_wcid: Option<u32>,
    /// Horizontal distance from the character, in metres (world x/y).
    pub away: f32,
}

/// Something the character wants to leave with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Want {
    pub wcid: u32,
    pub name: String,
    /// How many more are wanted than are carried.
    pub short: u32,
    /// It cannot hunt without this, so the trip is for nothing without it.
    pub urgent: bool,
}

/// The rules of the trip, as the player set them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rules {
    /// Free slots at which selling stops to make notes; the next handful of coin still needs one.
    pub keep_slots: u32,
    /// Coin kept loose rather than turned into notes, so small change needs no note broken.
    pub float: u32,
    /// Do not bother selling anything worth less than this.
    pub floor_value: u32,
    /// How near the counter must be before it will trade, in metres (as `Counter::away`).
    pub reach: f32,
}

impl Default for Rules {
    fn default() -> Self {
        Rules {
            keep_slots: 3,
            float: 2_000,
            floor_value: 0,
            reach: 3.0,
        }
    }
}

/// Everything the shopping is decided from, at one moment.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Snapshot {
    pub items: Vec<Item>,
    /// Loose coin.
    pub coin: u32,
    /// Trade notes held: count by face value.
    pub notes: BTreeMap<u32, u32>,
    /// Burden carried.
    pub carried: u32,
    /// Burden capacity: 150 x Strength before augmentations (ACE Player_Inventory.cs:50).
    pub capacity: u32,
    /// Slots free across the main pack and the side packs.
    pub slots_free: u32,
    pub counter: Option<Counter>,
    pub wants: Vec<Want>,
    pub rules: Rules,
}

impl Snapshot {
    /// What the character can spend: coin and notes both, since a counter takes either.
    pub fn purse(&self) -> u32 {
        self.notes
            .iter()
            .fold(self.coin, |t, (face, n)| t.saturating_add(face * n))
    }

    /// Burden that may still be added before the server refuses.
    /// The hard ceiling is 3 x capacity (ACE Player_Inventory.cs:56).
    pub fn burden_room(&self) -> u32 {
        self.capacity.saturating_mul(3).saturating_sub(self.carried)
    }

    pub fn item(&self, guid: u32) -> Option<&Item> {
        self.items.iter().find(|i| i.guid == guid)
    }

    /// Whether the counter is offered it: not kept, a kind it buys, worth the floor, not wanted.
    /// The one answer for both the selling and the panel's "for sale here" count.
    pub fn offers(&self, it: &Item) -> bool {
        let Some(counter) = self.counter.as_ref() else {
            return false;
        };
        if it.keep.forbidden() || it.wielded {
            return false;
        }
        if counter.buys != 0 && it.item_type & counter.buys == 0 {
            return false;
        }
        if it.value < self.rules.floor_value || it.value == 0 {
            return false;
        }
        // Skip what the character came to buy (selling and rebuying costs the markup),
        // unless the player tagged it Sell: then it is loot whatever the list says.
        let wanted = self.wants.iter().any(|w| w.wcid == it.wcid && w.short > 0);
        !wanted || it.to_sell()
    }
}
