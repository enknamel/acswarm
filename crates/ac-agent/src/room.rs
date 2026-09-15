//! Room pack by pack ([`Packs`]) and how to take a loose thing into it ([`how_to_take`]).
//! A take goes into the pack it names or nowhere (ACE `PutItemInContainer`, `limitToMainPackOnly`),
//! refused when full ("Unable to put Pyreal into container", read by `refusals`) however empty the sacks.
//! Only what the server creates (payout, purchase, gift) spills into side packs (`TryCreateInInventory`).
//! Coin and components can pour onto a carried stack (`StackableMerge`) with no slot; pours live in `pack`.

use crate::pack::Stack;

/// One pack, as the room in it is counted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    pub guid: u32,
    /// Item slots (102 in a player's main pack); 0 when unknown, which is room nobody can count on.
    pub capacity: u32,
    /// Items in it; a pack in a pack slot and a Focus are not main-pack items.
    pub used: u32,
    /// The server said it is full and nothing has left since; this outranks the count.
    pub said_full: bool,
}

impl Pack {
    /// Slots free in it.
    pub fn room(&self) -> u32 {
        if self.said_full {
            0
        } else {
            self.capacity.saturating_sub(self.used)
        }
    }
}

/// The main pack and the side packs hanging from it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Packs {
    pub main: Pack,
    pub side: Vec<Pack>,
}

impl Packs {
    /// Every pack, the main one first.
    pub fn all(&self) -> impl Iterator<Item = &Pack> {
        std::iter::once(&self.main).chain(self.side.iter())
    }

    /// What one take can use: the most room in any one pack.
    /// A take goes into one pack, so a sum reads as room to spare when every pack is on its last slot.
    pub fn for_a_take(&self) -> u32 {
        self.all().map(Pack::room).max().unwrap_or(0)
    }

    /// Every pack's room together: a payout, purchase or gift fills the main pack, then each side pack
    /// (`TryCreateInInventory`), and a sale is judged on this total (`GetFreeInventorySlots`, Container.cs:214).
    pub fn anywhere(&self) -> u32 {
        self.all().map(Pack::room).sum()
    }

    /// Pack to name in a take: main while it has a slot, else the roomiest side pack, ties to the lowest guid.
    /// Main first because the server puts things there itself; roomiest next so the sacks fill evenly.
    pub fn container_for_a_take(&self) -> Option<u32> {
        if self.main.room() > 0 {
            return Some(self.main.guid);
        }
        self.side
            .iter()
            .filter(|p| p.room() > 0)
            .max_by(|a, b| a.room().cmp(&b.room()).then(b.guid.cmp(&a.guid)))
            .map(|p| p.guid)
    }
}

/// Whether the server's "full", said when the pack held `held` items, still stands at `used` items.
/// It stands until something leaves: full at 102 is full at 102 however the count got there, not at 101.
pub fn still_full(held: u32, used: u32) -> bool {
    used >= held
}

/// The `held` to keep for the server's "full": the higher of `held` and `used`, `None` once something left.
/// The count often climbs after the word as awaited descriptions arrive; kept at the most seen, one leaving lifts it.
pub fn full_mark(held: u32, used: u32) -> Option<u32> {
    still_full(held, used).then_some(held.max(used))
}

/// A thing lying loose (on a corpse, on the ground), as [`how_to_take`] sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loose {
    pub wcid: u32,
    /// How many are in it (1 for a thing that does not stack).
    pub count: u32,
    /// Money or a spell component, which may be poured onto a carried stack; nothing else, however it stacks.
    /// A merge slips past a take's checks (quest drop caps, a second unique); coin and components are never capped.
    pub pours: bool,
    /// It is a pack: only the main pack has pack slots, and a pack put into a side pack is refused silently.
    pub is_pack: bool,
}

/// How a loose thing is to be taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Take {
    /// Put into this pack.
    Put(u32),
    /// Poured onto this carried stack, all of it.
    Merge { to: u32, amount: u32 },
}

/// How to take `item` (`None`: no room): whole onto the fullest carried pile it fits, so piles stay few; else put.
/// A pour costs no slot and no extra trip; half a pour leaves the rest on the body, asked for again and again.
pub fn how_to_take(item: &Loose, carried: &[Stack], packs: &Packs) -> Option<Take> {
    if item.is_pack {
        return Some(Take::Put(packs.main.guid));
    }
    if item.pours && item.count > 0 {
        let pile = carried
            .iter()
            .filter(|s| {
                s.wcid == item.wcid
                    && !s.wielded
                    && s.max > 1
                    && s.max.saturating_sub(s.count) >= item.count
            })
            .max_by(|a, b| a.count.cmp(&b.count).then(b.guid.cmp(&a.guid)));
        if let Some(pile) = pile {
            return Some(Take::Merge {
                to: pile.guid,
                amount: item.count,
            });
        }
    }
    packs.container_for_a_take().map(Take::Put)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: u32 = 0x5000_0001;
    const SACK: u32 = 0x8000_0010;
    const POUCH: u32 = 0x8000_0011;

    fn pack(guid: u32, capacity: u32, used: u32) -> Pack {
        Pack {
            guid,
            capacity,
            used,
            said_full: false,
        }
    }

    fn pile(guid: u32, wcid: u32, count: u32, max: u32) -> Stack {
        Stack {
            guid,
            wcid,
            name: format!("thing {wcid}"),
            count,
            max,
            wielded: false,
            burden: 0,
        }
    }

    #[test]
    fn room_for_a_take_is_the_roomiest_pack_and_not_the_sum() {
        // A sum counts sack room for takes aimed at a full main pack, so every body is offered and
        // every take refused.
        let packs = Packs {
            main: pack(ME, 102, 100),
            side: vec![pack(SACK, 24, 19), pack(POUCH, 24, 21)],
        };
        assert_eq!(packs.for_a_take(), 5);
        assert_eq!(packs.anywhere(), 2 + 5 + 3);
        let last_slots = Packs {
            main: pack(ME, 102, 101),
            side: vec![pack(SACK, 24, 23), pack(POUCH, 24, 23)],
        };
        assert_eq!(last_slots.for_a_take(), 1);
        assert_eq!(last_slots.anywhere(), 3);
    }

    #[test]
    fn a_take_goes_into_the_main_pack_while_it_has_a_slot_and_then_the_roomiest_sack() {
        let mut packs = Packs {
            main: pack(ME, 102, 101),
            side: vec![pack(SACK, 24, 20), pack(POUCH, 24, 10)],
        };
        assert_eq!(packs.container_for_a_take(), Some(ME));
        packs.main.used = 102;
        assert_eq!(packs.container_for_a_take(), Some(POUCH));
        packs.side[1].used = 24;
        assert_eq!(packs.container_for_a_take(), Some(SACK));
        packs.side[0].used = 24;
        assert_eq!(packs.container_for_a_take(), None);
        assert_eq!(packs.for_a_take(), 0);
    }

    #[test]
    fn the_first_of_equal_sacks_is_chosen_so_the_answer_does_not_wander() {
        let packs = Packs {
            main: pack(ME, 102, 102),
            side: vec![pack(POUCH, 24, 10), pack(SACK, 24, 10)],
        };
        assert_eq!(packs.container_for_a_take(), Some(SACK));
        let mut reversed = packs.clone();
        reversed.side.reverse();
        assert_eq!(reversed.container_for_a_take(), Some(SACK));
    }

    #[test]
    fn what_the_server_called_full_has_no_room_however_the_count_reads() {
        // Guards a count two short of the server's (a description missed): 400 takes to a full pack.
        let mut main = pack(ME, 102, 100);
        assert_eq!(main.room(), 2);
        main.said_full = true;
        assert_eq!(main.room(), 0);
        let packs = Packs {
            main,
            side: vec![pack(SACK, 24, 24)],
        };
        assert_eq!(packs.container_for_a_take(), None);
        assert_eq!(packs.for_a_take(), 0);
        assert_eq!(packs.anywhere(), 0);
    }

    #[test]
    fn the_servers_word_holds_until_something_leaves_the_pack() {
        assert!(still_full(102, 102));
        assert!(still_full(102, 103), "counted past what the server had");
        assert!(!still_full(102, 101), "something left");
    }

    #[test]
    fn the_word_is_kept_at_the_most_seen_so_one_thing_leaving_lifts_it() {
        // Full said at 100, then two awaited descriptions make it 102: kept at 100, the pack would read
        // full after two sold, until a third left.
        assert_eq!(full_mark(100, 100), Some(100));
        assert_eq!(full_mark(100, 102), Some(102), "the count caught up");
        assert_eq!(full_mark(102, 102), Some(102));
        assert_eq!(full_mark(102, 101), None, "one left: room again");
        assert_eq!(full_mark(100, 99), None);
    }

    #[test]
    fn a_pack_with_no_known_size_has_no_room() {
        assert_eq!(pack(SACK, 0, 0).room(), 0);
    }

    fn pyreals(count: u32) -> Loose {
        Loose {
            wcid: 273,
            count,
            pours: true,
            is_pack: false,
        }
    }

    #[test]
    fn coin_is_poured_onto_the_pile_carried_rather_than_given_a_slot() {
        // Even with a slot to spare: the same round trip, no slot, and no tidying pour later.
        let packs = Packs {
            main: pack(ME, 102, 50),
            side: vec![],
        };
        let carried = [pile(1, 273, 5_000, 25_000), pile(2, 8329, 40, 100)];
        assert_eq!(
            how_to_take(&pyreals(320), &carried, &packs),
            Some(Take::Merge { to: 1, amount: 320 })
        );
        let two_piles = [pile(1, 273, 5_000, 25_000), pile(2, 273, 24_000, 25_000)];
        assert_eq!(
            how_to_take(&pyreals(320), &two_piles, &packs),
            Some(Take::Merge { to: 2, amount: 320 })
        );
        assert_eq!(
            how_to_take(&pyreals(1_500), &two_piles, &packs),
            Some(Take::Merge {
                to: 1,
                amount: 1_500
            })
        );
    }

    #[test]
    fn a_stack_that_fits_no_pile_whole_takes_a_slot_and_is_left_when_there_is_none() {
        let carried = [pile(1, 273, 24_000, 25_000)];
        let roomy = Packs {
            main: pack(ME, 102, 102),
            side: vec![pack(SACK, 24, 7)],
        };
        assert_eq!(
            how_to_take(&pyreals(1_500), &carried, &roomy),
            Some(Take::Put(SACK))
        );
        let full = Packs {
            main: pack(ME, 102, 102),
            side: vec![pack(SACK, 24, 24)],
        };
        assert_eq!(how_to_take(&pyreals(1_500), &carried, &full), None);
        assert_eq!(
            how_to_take(&pyreals(500), &carried, &full),
            Some(Take::Merge { to: 1, amount: 500 })
        );
    }

    #[test]
    fn only_money_and_components_are_poured() {
        // Arrows stack, but the server puts a take through its checks and a merge not.
        let arrows = Loose {
            wcid: 300,
            count: 50,
            pours: false,
            is_pack: false,
        };
        let carried = [pile(1, 300, 100, 1_000)];
        let packs = Packs {
            main: pack(ME, 102, 50),
            side: vec![],
        };
        assert_eq!(how_to_take(&arrows, &carried, &packs), Some(Take::Put(ME)));
        // Nor onto a wielded pile: the tidying tops up the quiver, not a take off a body.
        let mut quiver = pile(1, 273, 100, 25_000);
        quiver.wielded = true;
        assert_eq!(
            how_to_take(&pyreals(5), &[quiver], &packs),
            Some(Take::Put(ME))
        );
    }

    #[test]
    fn a_pack_off_a_body_goes_to_the_main_pack_whatever_the_room() {
        // Pack slots are the main pack's alone; a pack put into a sack is refused without a word.
        let sack = Loose {
            wcid: 166,
            count: 1,
            pours: false,
            is_pack: true,
        };
        let full = Packs {
            main: pack(ME, 102, 102),
            side: vec![pack(SACK, 24, 7)],
        };
        assert_eq!(how_to_take(&sack, &[], &full), Some(Take::Put(ME)));
    }
}
