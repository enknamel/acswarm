//! Where the room is: the main pack and each side pack, counted apart.
//!
//! A character's slots are not one number. The main pack has its own
//! (102, for a player), and every side pack hanging from it has its
//! own, and the server never moves a thing from one to another on its
//! own account. A take off a corpse names the pack it is to go into,
//! and goes into that pack or nowhere: ACE's `PutItemInContainer` adds
//! to the named container with `limitToMainPackOnly`, so a take aimed
//! at the character goes into the main pack and, when that is full, is
//! turned down -- "Unable to put Pyreal into container" -- however
//! empty the sacks beside it. Only what the server creates itself (a
//! counter's payout, a purchase, a gift) spills from the main pack
//! into the side packs, through `TryCreateInInventory`.
//!
//! Counted as one sum, nine characters whose main packs had filled
//! believed they had room, offered every body for looting, and were
//! refused 2,114 takes in ten minutes; kills fell from eight a minute
//! to under three. So the room is kept pack by pack, and what a caller
//! means by "room" is asked for by name: what one take can use
//! ([`Packs::for_a_take`]) is the most room any one pack has, and what
//! the server may spread new stacks over ([`Packs::anywhere`]) is all
//! of it together.
//!
//! A stack is the exception. Money and spell components off a corpse
//! can be poured straight onto a stack already carried
//! (`StackableMerge`), which needs no slot at all; [`how_to_take`]
//! chooses that where it can.

use crate::pack::Stack;

/// One pack, as the room in it is counted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    pub guid: u32,
    /// Item slots it has. 0 when the server has not said, and a pack
    /// whose size is not known has no room anyone can count on.
    pub capacity: u32,
    /// Items in it. A pack in a pack slot is not one of the main
    /// pack's items, and nor is a Focus.
    pub used: u32,
    /// The server has said it is full, and nothing has left it since.
    /// What the server says outranks the count: a pack the count
    /// believes has room and the server has just refused has none.
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

    /// What one take can use: the most room any one pack has. A take
    /// goes into one pack, so two packs with a slot each are room for
    /// two takes but never for a thing that needs two slots -- and
    /// added together they read as a pack with room to spare when
    /// every one of them is down to its last slot.
    pub fn for_a_take(&self) -> u32 {
        self.all().map(Pack::room).max().unwrap_or(0)
    }

    /// What the server may spread new stacks over: every pack's room
    /// together. A counter's payout, a purchase and a gift are created
    /// in the pack by the server (`TryCreateInInventory`), which fills
    /// the main pack and then each side pack in turn, and a sale is
    /// judged against this same total (`GetFreeInventorySlots`, side
    /// packs included).
    pub fn anywhere(&self) -> u32 {
        self.all().map(Pack::room).sum()
    }

    /// The pack to name in a take: the main pack while it has a slot,
    /// else the side pack with the most room, the first of equals.
    ///
    /// The main pack first because that is where the server puts
    /// things itself, so a character that fills its main pack last
    /// keeps the most room for what it is handed; and the roomiest
    /// side pack next so that the sacks empty evenly rather than one
    /// filling while the rest sit empty.
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

/// Whether the server's word that a pack was full, given when it held
/// `held` items, still stands now that it holds `used`. It stands until
/// something leaves: a pack the server called full at 102 is full at
/// 102 however the count was reached, and has room again at 101.
pub fn still_full(held: u32, used: u32) -> bool {
    used >= held
}

/// The word as it is to be kept from one look to the next: the most
/// the pack has been seen to hold since the server said it was full
/// (`held`), against what it holds now (`used`). `None` once something
/// has left it, and otherwise the higher of the two.
///
/// The count climbs after the word as often as not: the server called
/// the pack full when the client had counted a hundred, and the two
/// descriptions it was still waiting on then arrive. Kept at a hundred,
/// the word would outlive the next two things sold; kept at the most
/// seen, one thing leaving is enough to lift it, which is what the
/// word meant.
pub fn full_mark(held: u32, used: u32) -> Option<u32> {
    still_full(held, used).then_some(held.max(used))
}

/// A thing lying loose -- on a corpse, on the ground -- as the choice
/// of how to take it needs to see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loose {
    pub wcid: u32,
    /// How many are in it (1 for a thing that does not stack).
    pub count: u32,
    /// It may be poured onto a stack already carried: money or a spell
    /// component. Nothing else is, however well it stacks. The server
    /// caps what can be had of a quest's drop and refuses a second of
    /// a unique, and a merge slips past the checks a take is put
    /// through; coin and components are what it never limits.
    pub pours: bool,
    /// It is a pack. A pack goes in a pack slot, of which the main pack
    /// holds the only ones, and the server turns a pack put into a
    /// side pack down without a word.
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

/// How to take `item`, given the stacks carried and the room in the
/// packs, or `None` when there is no way to.
///
/// A pour comes first wherever the whole stack fits onto one carried:
/// it costs the same round trip as a put, spends no slot, and saves
/// the pour the tidying would otherwise make of the two stacks later.
/// Only the whole of it, though. Half a stack poured leaves the other
/// half lying on the body, asked for again and again until it is given
/// up on; a stack too big for any pile it could join is put in a slot
/// instead, and left for room when there is none.
///
/// The pile chosen is the fullest that still fits it, so the piles stay
/// few and full rather than each being topped up a little.
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

/// The item the server's words say would not go into its pack, out of
/// a line of chat: "Unable to put Pyreal into container". ACE's whole
/// word on a take aimed at a pack with no slot, sent ahead of an
/// `InventoryServerSaveFailed` with no reason in it -- and its only
/// word: these words are what makes a refusal the pack's. A refusal
/// with no reason and no word at all is not the pack, however much it
/// looks like one with the words still on their way: it is what ACE
/// sends for a second of a unique (`CheckUniques` explains itself in
/// the system chat, not in a game event), and for a pack put into a
/// sack. Read as the pack being full, a unique on a body marked every
/// pack full in turn and sent the character to town with its slots
/// free.
pub fn unable_to_put(text: &str) -> Option<&str> {
    text.strip_prefix("Unable to put ")?
        .strip_suffix(" into container")
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
        // Nine characters at 102/102 with a sack at 7 of 24: summed,
        // seventeen free; for a take, seventeen -- in the sack, and
        // none in the main pack the takes were aimed at.
        let packs = Packs {
            main: pack(ME, 102, 100),
            side: vec![pack(SACK, 24, 19), pack(POUCH, 24, 21)],
        };
        assert_eq!(packs.for_a_take(), 5);
        assert_eq!(packs.anywhere(), 2 + 5 + 3);
        // Every pack down to its last slot is room for one take, not
        // three.
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
        // The client's count of the main pack was two short of the
        // server's -- a description missed, a thing counted twice --
        // and the take went to it four hundred times.
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
        // Called full when the client had counted a hundred; the two
        // descriptions still on their way arrive, and the count reads
        // 102. Two sold: kept at a hundred the word would still stand,
        // and the pack would read as full until a third left.
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
        // Even with a slot to spare: the pour costs the same round trip
        // and saves the slot and the tidying's pour of the two later.
        let packs = Packs {
            main: pack(ME, 102, 50),
            side: vec![],
        };
        let carried = [pile(1, 273, 5_000, 25_000), pile(2, 8329, 40, 100)];
        assert_eq!(
            how_to_take(&pyreals(320), &carried, &packs),
            Some(Take::Merge { to: 1, amount: 320 })
        );
        // Onto the fullest pile it fits, so the piles stay few.
        let two_piles = [pile(1, 273, 5_000, 25_000), pile(2, 273, 24_000, 25_000)];
        assert_eq!(
            how_to_take(&pyreals(320), &two_piles, &packs),
            Some(Take::Merge { to: 2, amount: 320 })
        );
        // A pile it does not wholly fit is passed over for one it does.
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
        // But what fits whole is poured with no slot at all.
        assert_eq!(
            how_to_take(&pyreals(500), &carried, &full),
            Some(Take::Merge { to: 1, amount: 500 })
        );
    }

    #[test]
    fn only_money_and_components_are_poured() {
        // Arrows stack, and a quiver-full is carried, but a take is
        // what the server puts through its checks; a merge is not.
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
        // Nor onto a pile in the character's hands: the quiver is
        // topped up by the tidying, not off a body.
        let mut quiver = pile(1, 273, 100, 25_000);
        quiver.wielded = true;
        assert_eq!(
            how_to_take(&pyreals(5), &[quiver], &packs),
            Some(Take::Put(ME))
        );
    }

    #[test]
    fn a_pack_off_a_body_goes_to_the_main_pack_whatever_the_room() {
        // Pack slots are the main pack's alone, and the server refuses
        // a pack put into a sack without a word.
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

    #[test]
    fn the_servers_words_name_what_would_not_go_in() {
        assert_eq!(
            unable_to_put("Unable to put Pyreal into container"),
            Some("Pyreal")
        );
        assert_eq!(
            unable_to_put("Unable to put Major Mana Stone into container"),
            Some("Major Mana Stone")
        );
        assert_eq!(unable_to_put("You are too encumbered to carry that!"), None);
        assert_eq!(unable_to_put("Unable to put"), None);
    }
}
