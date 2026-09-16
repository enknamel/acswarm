use crate::autoplay::growth::burden::past_the_wall;
use crate::autoplay::Autoplay;
use crate::Client;

/// What the character has room for, as a corpse waiting on it sees it
/// (see `Client::room_for_loot`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Room {
    /// The pack is down to the slots kept for a counter's money.
    pub(crate) pack_low: bool,
    /// Carrying more than three times its capacity: the server hands it
    /// nothing at all, not even a coin.
    pub(crate) past_the_wall: bool,
    /// How much more loot it means to carry (see `Client::carry_room`).
    pub(crate) carry: u32,
}

impl Room {
    /// Room for anything.
    #[cfg(test)]
    pub(crate) const PLENTY: Room = Room {
        pack_low: false,
        past_the_wall: false,
        carry: u32::MAX,
    };
}

impl Autoplay {
    /// What the lightest thing left for its weight on a body still lying
    /// about weighs (`there` says which bodies are), if anything was.
    pub(crate) fn lightest_left_for_weight(&self, there: impl Fn(u32) -> bool) -> Option<u32> {
        self.left_for_weight
            .iter()
            .filter(|(g, _)| there(**g))
            .map(|(_, burden)| *burden)
            .min()
    }
}

impl Client {
    /// The room in the packs, pack by pack: the main pack and each side
    /// pack with its own slots and its own count (see `room::Packs`).
    ///
    /// Counted apart because the server keeps them apart. A take names
    /// the pack it goes into and is refused when that one is full,
    /// however empty the sacks beside it; only what the server creates
    /// itself -- a counter's payout, a purchase -- spills from the main
    /// pack into the side packs. Added into one sum, nine characters
    /// whose main packs had filled offered every body for looting and
    /// were refused seven thousand takes.
    ///
    /// Pack slots are a separate count and are left out: a pack, and
    /// each of the five Foci, sits in one of those in the main pack
    /// (see `ac_world::pack_slot`). What the server has said is full
    /// stays full until something leaves it (`packs_said_full`).
    pub(crate) fn packs(&self) -> crate::room::Packs {
        let me = self.world.player_guid;
        let capacity = self
            .world
            .player()
            .map(|p| p.items_capacity)
            .filter(|c| *c > 0)
            .unwrap_or(102);
        let mut packs = crate::room::Packs {
            main: crate::room::Pack {
                guid: me.unwrap_or(0),
                capacity,
                used: 0,
                said_full: false,
            },
            side: Vec::new(),
        };
        let in_a_pack_slot = |o: &ac_world::WorldObject| {
            o.container == me
                && ac_world::pack_slot::used_by(
                    o.weenie_class_id,
                    o.item_type & ac_world::item_type::CONTAINER != 0,
                )
        };
        // The side packs first, so what is in them has somewhere to be
        // counted; by guid, so the choice among equals does not wander
        // between frames.
        for o in self.world.main_pack() {
            if in_a_pack_slot(o) && o.item_type & ac_world::item_type::CONTAINER != 0 {
                packs.side.push(crate::room::Pack {
                    guid: o.guid,
                    capacity: o.items_capacity,
                    used: 0,
                    said_full: false,
                });
            }
        }
        packs.side.sort_by_key(|p| p.guid);
        for o in self.world.inventory() {
            if in_a_pack_slot(o) {
                continue;
            }
            if o.container == me {
                packs.main.used += 1;
            } else if let Some(p) = packs.side.iter_mut().find(|p| Some(p.guid) == o.container) {
                p.used += 1;
            }
        }
        for p in std::iter::once(&mut packs.main).chain(packs.side.iter_mut()) {
            p.said_full = self
                .packs_said_full
                .get(&p.guid)
                .is_some_and(|held| crate::room::still_full(*held, p.used));
        }
        packs
    }

    /// No pack has a slot for a take.
    pub fn pack_full(&self) -> bool {
        self.packs().for_a_take() == 0
    }

    /// Free item slots as one take can use them: the most room any one
    /// pack has (see `room::Packs::for_a_take`). A take goes into the
    /// pack it names, so the slots of the main pack and each side pack
    /// are not added: added, a character with every pack down to its
    /// last slot read as having room for a dozen takes.
    pub fn free_space(&self) -> u32 {
        self.packs().for_a_take()
    }

    /// Free item slots as the server spreads what it creates over them:
    /// every pack's room together (see `room::Packs::anywhere`). A
    /// counter's payout, a purchase and a gift are created in the main
    /// pack and spill into the side packs, and a sale is judged
    /// against this same total.
    pub fn room_anywhere(&self) -> u32 {
        self.packs().anywhere()
    }

    /// No pack has a slot for a take, or the packs together are down to
    /// the slots kept free for a counter's money (`restock.keep_slots`):
    /// time to sell, while a sale can still be paid for. The server
    /// finds room for the coin before it takes the goods, so a pack with
    /// no slot at all cannot be sold out of -- and it finds that room
    /// in any pack, so the slots kept are counted over all of them.
    /// Counted in the one pack a take could use, a character with two
    /// slots in the main pack and two in the sack went to town with
    /// room for its money twice over.
    pub fn pack_low_on_room(&self) -> bool {
        let packs = self.packs();
        packs.for_a_take() == 0 || packs.anywhere() <= self.autoplay.config.team.restock.keep_slots
    }

    /// What the character has room for, as a corpse waiting on it sees
    /// it (see `Autoplay::corpse_waiting`).
    pub(crate) fn room_for_loot(&self) -> Room {
        let (carried, capacity) = self.burden();
        Room {
            pack_low: self.pack_low_on_room(),
            past_the_wall: past_the_wall(carried, capacity),
            // Weighed only while a body is waiting on it: what the loot
            // weighs is judged item by item, and this is asked on every
            // tick a corpse lies about.
            carry: if self.autoplay.left_for_weight.is_empty() {
                u32::MAX
            } else {
                self.carry_room(&self.autoplay.config.growth)
            },
        }
    }
}

#[cfg(test)]
mod tests;
