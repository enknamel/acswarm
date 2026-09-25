//! One trip to a counter, one decision at a time.

use std::time::Instant;

use ac_agent::did::{Did, Patience};
use ac_agent::pack;
use ac_agent::refusals::Answer;

use crate::counter::{Item, Snapshot};

/// The one thing the rules want done next; each is something a player could do by hand.
/// At most one per turn: the server answers in its own time, and two can sell a stack merged away.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Act {
    /// Walk to the counter: it will not trade across a room.
    Approach { guid: u32 },
    /// Ask it to open its window.
    Open { guid: u32 },
    /// Pour one stack into another to free a slot.
    Merge { from: u32, to: u32, amount: u32 },
    /// Cut a piece off a stack worth more than the counter will look at.
    Split { guid: u32, amount: u32 },
    /// Hand these over in one go: a counter takes a whole armful (the sell action carries a list).
    /// The pack bounds an armful, not the counter: the takings come back as coin, needing slots.
    Sell { items: Vec<u32> },
    /// Buy this many of something on the shelf.
    Buy { wcid: u32, count: u32 },
    /// Turn trade notes back into coin to pay a bill.
    /// No change is given for a note, so selling notes back is the only way to buy something small.
    Cash { face: u32, count: u32 },
    /// Shut the window; the trip is over.
    Close,
}

/// Where the trip has got to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    /// Not at the counter yet.
    #[default]
    Walking,
    /// Waiting for the window.
    Opening,
    /// Compressing, selling and making notes, round after round.
    Selling,
    /// Spending what the selling raised.
    Buying,
    /// Nothing left to do here.
    Done,
}

/// What came of a turn: what to do, what it means, and how to say it.
#[derive(Clone, Debug, PartialEq)]
pub struct Next {
    pub act: Option<Act>,
    pub did: Did,
    /// One line for the log and the panel, in the character's voice.
    pub saying: String,
}

impl Next {
    fn acting(act: Act, saying: impl Into<String>) -> Next {
        Next {
            act: Some(act),
            did: Did::Acting,
            saying: saying.into(),
        }
    }

    fn nothing(did: Did, saying: impl Into<String>) -> Next {
        Next {
            act: None,
            did,
            saying: saying.into(),
        }
    }
}

/// A trip to one counter; holds only what no snapshot shows: phase, pending offers, refusals.
/// Cloned so a panel can ask a copy what the trip would do next without moving it on.
#[derive(Clone, Debug, Default)]
pub struct Run {
    pub phase: Phase,
    /// Handed to the counter and not yet gone from the pack.
    offered: Vec<u32>,
    /// Items no counter will take.
    refused: Patience<u32>,
    /// Pairs of stacks that would not join.
    wont_merge: Patience<(u32, u32)>,
    /// Items sold this trip.
    pub sold: u32,
    /// Goods to sell, no slot for their coin: selling stands aside and the visit says why.
    no_room: bool,
    /// The ware last asked for, until the counter answers: what a refusal in words is about.
    buying: Option<u32>,
    /// Wares the counter would not sell this visit, by weenie.
    wont_sell: Patience<u32>,
    /// Why the first ware still wanted was not bought here, for the visit to say.
    unbought: Option<String>,
}

impl Run {
    pub fn new() -> Run {
        Run::default()
    }

    /// How many offered items the counter has not yet answered for.
    pub fn waiting_on(&self) -> usize {
        self.offered.len()
    }

    /// Records that this side could not do `act` (a pour, a cut, a ware with no shelf line), as a
    /// counter's no is recorded: so it is not asked for again every turn, and nothing is waited on.
    pub fn refused(&mut self, act: &Act, now: Instant) {
        match act {
            Act::Merge { from, to, .. } => {
                self.wont_merge.note(
                    (*from, *to),
                    &Did::refused("the stacks would not pour"),
                    now,
                );
            }
            Act::Split { guid, .. } => {
                self.refused
                    .note(*guid, &Did::refused("the stack would not cut"), now);
            }
            Act::Buy { wcid, .. } => {
                self.wont_sell
                    .note(*wcid, &Did::refused("the counter would not sell it"), now);
            }
            // Every other act is decided afresh from each snapshot, which shows what failed.
            Act::Approach { .. }
            | Act::Open { .. }
            | Act::Sell { .. }
            | Act::Cash { .. }
            | Act::Close => {}
        }
    }

    /// The counter turned down the purchase in flight, in words (`Refusal::Buy`): that ware is
    /// held as the refusals table answers, and a snapshot the refusal left unchanged moves on.
    pub fn buy_refused(&mut self, answer: Answer, now: Instant) {
        if let Some(wcid) = self.buying.take() {
            answer.hold(
                &mut self.wont_sell,
                wcid,
                "the counter would not sell it",
                now,
            );
        }
    }

    /// The next thing to do, or why there is nothing.
    pub fn step(&mut self, snap: &Snapshot, now: Instant) -> Next {
        // Asked once the counter has answered the last act, and its words come ahead of that
        // answer (Vendor.cs:496-500, then UseDone at Player_Commerce.cs:51): a buy unrefused went.
        self.buying = None;
        let Some(counter) = snap.counter.as_ref() else {
            self.phase = Phase::Done;
            return Next::nothing(Did::blocked("there is no counter here"), "no counter");
        };

        // Stand at it first: asked from across the room, the server tells the character to walk
        // there, and a client that does not walk waits for a window that never opens.
        if counter.away > snap.rules.reach {
            self.phase = Phase::Walking;
            return Next::acting(
                Act::Approach { guid: counter.guid },
                format!("walking up to {}", counter.name),
            );
        }
        if !counter.open {
            self.phase = Phase::Opening;
            return Next::acting(
                Act::Open { guid: counter.guid },
                format!("talking to {}", counter.name),
            );
        }

        // Offered but still in the pack: the item itself refused; kind and worth were checked.
        if !self.offered.is_empty() {
            let still: Vec<u32> = self
                .offered
                .iter()
                .copied()
                .filter(|g| snap.item(*g).is_some())
                .collect();
            self.sold += (self.offered.len() - still.len()) as u32;
            self.offered.clear();
            for guid in still {
                self.refused
                    .note(guid, &Did::refused("no counter will take it"), now);
            }
        }

        if matches!(self.phase, Phase::Walking | Phase::Opening) {
            self.phase = Phase::Selling;
        }

        if self.phase == Phase::Selling {
            // 1. Compress before any sale and again each round: a sale lightens the character,
            //    and the server weighs a pour against what is carried.
            if let Some(next) = self.compress(snap, now) {
                return next;
            }
            // 2. Make notes once the pack is low on room.
            if snap.slots_free <= snap.rules.keep_slots {
                if let Some(next) = self.make_notes(snap, now) {
                    return next;
                }
            }
            // 3. Sell an armful.
            if let Some(next) = self.sell_one(snap, now) {
                return next;
            }
            self.phase = Phase::Buying;
        }

        if self.phase == Phase::Buying {
            if let Some(next) = self.buy_one(snap, now) {
                return next;
            }
            self.phase = Phase::Done;
        }

        if self.no_room && self.sold == 0 {
            return Next {
                act: Some(Act::Close),
                did: Did::blocked("no free slot for the money"),
                saying: format!("no room in the pack for what {} would pay", counter.name),
            };
        }
        let unbought = self
            .unbought
            .as_ref()
            .map(|why| format!("; bought no {why}"))
            .unwrap_or_default();
        Next {
            act: Some(Act::Close),
            did: Did::Done,
            saying: format!("sold {} item(s) to {}{unbought}", self.sold, counter.name),
        }
    }

    /// Pour one pair of stacks together, if one is worth pouring and light enough for the server.
    fn compress(&mut self, snap: &Snapshot, now: Instant) -> Option<Next> {
        let stacks: Vec<pack::Stack> = snap
            .items
            .iter()
            .map(|i| pack::Stack {
                guid: i.guid,
                wcid: i.wcid,
                name: i.name.clone(),
                count: i.stack.max(1),
                max: i.max_stack,
                wielded: i.wielded,
                burden: i.burden,
            })
            .collect();
        let room = snap.burden_room();
        // Never pour past the counter's ceiling: selling cuts such a stack down, and pouring the
        // piece straight back would have the two alternate for ever.
        let ceiling = snap
            .counter
            .as_ref()
            .map(|c| c.max_value)
            .filter(|c| *c > 0);
        let worth = |guid: u32| snap.item(guid).map(|i| i.value).unwrap_or(0);
        // Never pour stacks whose words differ: one stack carries one word, and the ledger settles
        // a mix as kept (the cautious answer) or undecided, so a Sell tag would be overruled.
        let word = |guid: u32| snap.item(guid).map(|i| i.taken_for);
        let held = |from: u32, to: u32| {
            if self.wont_merge.held(&(from, to), now) {
                return true;
            }
            if word(from) != word(to) {
                return true;
            }
            match ceiling {
                Some(max) => worth(from).saturating_add(worth(to)) > max,
                None => false,
            }
        };
        match pack::next_merge_unless(&stacks, room, held) {
            Some(m) => {
                self.wont_merge.note(
                    (m.from, m.to),
                    &Did::waiting("the stacks have not joined yet"),
                    now,
                );
                Some(Next::acting(
                    Act::Merge {
                        from: m.from,
                        to: m.to,
                        amount: m.amount,
                    },
                    format!("putting {} {} with the rest", m.amount, m.name),
                ))
            }
            None => None,
        }
    }

    /// Turn the takings into trade notes, so the selling can go on.
    fn make_notes(&mut self, snap: &Snapshot, now: Instant) -> Option<Next> {
        let counter = snap.counter.as_ref()?;
        let face = counter.note_face.filter(|f| *f > 0)?;
        // Loses the 15% markup, but a slot of Mayoi notes holds 62.5 million, coin 25,000.
        let each = crate::errand::note_cost(face).max(1);
        // Each new stack needs a slot free before any coin leaves, a carried note stack no help
        // (ItemsToReceive.cs:97-108; Vendor.cs:471-503): with none free, the selling goes first.
        let count = (snap.coin.saturating_sub(snap.rules.float) / each)
            .min(snap.slots_free.saturating_mul(crate::errand::NOTE_STACK));
        if count == 0 {
            return None;
        }
        let wcid = counter.note_wcid.filter(|w| !self.wont_sell.held(w, now))?;
        self.buying = Some(wcid);
        Some(Next::acting(
            Act::Buy { wcid, count },
            format!("packing the takings into {count} note(s)"),
        ))
    }

    /// Hand over an armful, cutting down first a stack worth more than the counter will look at.
    /// The armful stops before its coin needs more slots than are free; the next round makes notes.
    fn sell_one(&mut self, snap: &Snapshot, now: Instant) -> Option<Next> {
        let counter = snap.counter.as_ref()?;
        // The answer the panel counts too (`Snapshot::offers`), less this trip's refusals.
        let mut offer: Vec<&Item> = snap
            .items
            .iter()
            .filter(|it| snap.offers(it) && !self.refused.held(&it.guid, now))
            .collect();
        // Dearest first: most worth their slot, and the ones the armful must include.
        offer.sort_by_key(|i| std::cmp::Reverse(i.value));

        // A stack too dear for this counter is cut on its own turn: the piece is a new object, and
        // an armful cannot name a guid that does not exist yet.
        if let Some(it) = offer.first().copied() {
            if counter.max_value > 0 && it.value > counter.max_value {
                let each = it.each().max(1);
                let take = (counter.max_value / each).max(1).min(it.stack);
                if take < it.stack {
                    return Some(Next::acting(
                        Act::Split {
                            guid: it.guid,
                            amount: take,
                        },
                        format!("cutting {take} off the {}", it.name),
                    ));
                }
                // Set aside, and the rest still sold: an answer with no act closes the visit.
                self.refused.note(
                    it.guid,
                    &Did::refused("worth more than this counter will look at"),
                    now,
                );
                offer.remove(0);
            }
        }

        // One armful is one payment, and the pack must hold it before the goods go: the errand
        // plans the trip by this same rule (`errand::armful_within_slots`).
        let pays: Vec<u32> = offer.iter().map(|it| it.value).collect();
        let (taken, _) = crate::errand::armful_within_slots(&pays, snap.slots_free);
        let items: Vec<u32> = taken.iter().map(|&i| offer[i].guid).collect();
        if items.is_empty() {
            // No room for the money is not the counter's to fix: stand aside so cashing and buying
            // get their turn, and the visit ends saying why nothing sold.
            self.no_room |= !offer.is_empty();
            return None;
        }
        self.offered.extend(items.iter().copied());
        let saying = match items.len() {
            1 => format!("selling {} item", items.len()),
            n => format!("selling {n} items"),
        };
        Some(Next::acting(Act::Sell { items }, saying))
    }

    /// Buy one thing the character came for, within the purse and the burden it can still lift.
    fn buy_one(&mut self, snap: &Snapshot, now: Instant) -> Option<Next> {
        let counter = snap.counter.as_ref()?;
        let purse = snap.purse();
        for want in &snap.wants {
            if want.short == 0 {
                continue;
            }
            let mut skip = |why: String| {
                if self.unbought.is_none() {
                    self.unbought = Some(format!("{}: {why}", want.name));
                }
            };
            if self.wont_sell.held(&want.wcid, now) {
                skip("the counter would not sell it".into());
                continue;
            }
            let Some(ware) = counter.wares.iter().find(|w| w.wcid == want.wcid) else {
                skip("not on this shelf".into());
                continue;
            };
            if ware.price == 0 || ware.price > purse {
                skip(format!("{} each, {purse} in coin", ware.price));
                continue;
            }
            let afford = ware.affordable(purse);
            // Unknown burden (0) is no reason to refuse: buy, and let the server answer for it.
            let liftable = match ware.burden {
                0 => u32::MAX,
                each => snap.burden_room() / each,
            };
            let count = want
                .short
                .min(afford)
                .min(liftable)
                .min(ware.stock.unwrap_or(u32::MAX));
            if count == 0 {
                let why = if liftable == 0 {
                    format!("no burden room ({} to carry)", snap.burden_room())
                } else {
                    "none left on the shelf".to_string()
                };
                if self.unbought.is_none() {
                    self.unbought = Some(format!("{}: {why}", want.name));
                }
                continue;
            }
            let bill = ware.bill(count);
            // Coin first: a purse all in notes cannot pay a small bill, so cash the smallest note
            // that covers it rather than break a fortune for a handful of tapers.
            if bill > snap.coin {
                let short = bill - snap.coin;
                if let Some((face, have)) = snap
                    .notes
                    .iter()
                    .find(|(face, n)| **n > 0 && **face >= short)
                    .or_else(|| snap.notes.iter().rev().find(|(_, n)| **n > 0))
                {
                    let need = short.div_ceil((*face).max(1)).min(*have);
                    if need > 0 {
                        return Some(Next::acting(
                            Act::Cash {
                                face: *face,
                                count: need,
                            },
                            format!("cashing {need} note(s) to pay for {}", want.name),
                        ));
                    }
                }
                continue;
            }
            self.buying = Some(want.wcid);
            return Some(Next::acting(
                Act::Buy {
                    wcid: want.wcid,
                    count,
                },
                format!("buying {count} {}", want.name),
            ));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::counter::{Counter, Keep, LootAction, Rules, Want, Ware};
    use std::collections::BTreeMap;

    const COIN: u32 = 273; // the pyreal's own weenie
    const NOTE: u32 = 20630; // Mayoi trade note, face 250,000

    fn item(guid: u32, name: &str, value: u32, stack: u32, max: u32) -> Item {
        Item {
            guid,
            wcid: guid,
            name: name.into(),
            value,
            burden: 10,
            stack,
            max_stack: max,
            item_type: 1,
            pack: None,
            wielded: false,
            keep: Keep::default(),
            taken_for: None,
        }
    }

    fn counter() -> Counter {
        Counter {
            guid: 900,
            name: "Ulgrim".into(),
            open: true,
            buys: 0,
            max_value: 0,
            wares: vec![Ware {
                wcid: NOTE,
                name: "Mayoi Trade Note".into(),
                price: 287_500,
                stock: None,
                burden: 1,
                value: 0,
                sell_rate: 0.0,
            }],
            note_face: Some(250_000),
            note_wcid: Some(NOTE),
            away: 0.0,
        }
    }

    fn snap(items: Vec<Item>) -> Snapshot {
        Snapshot {
            items,
            coin: 0,
            notes: BTreeMap::new(),
            carried: 1_000,
            capacity: 30_000,
            slots_free: 20,
            counter: Some(counter()),
            wants: Vec::new(),
            rules: Rules::default(),
        }
    }

    #[test]
    fn a_stack_is_charged_as_one_item_and_a_purse_is_sized_to_it() {
        // Magus Guthima sells a Prismatic Taper, worth 22, at 1.55: 34 each, but 150 in one stack
        // are one item of 3,300 and cost 5,115. Sized at 34 each, 150 were asked for with 5,109
        // in the purse, and the server refused that some 1,200 times in three minutes.
        let taper = Ware {
            wcid: 20631,
            name: "Prismatic Taper".into(),
            price: 34,
            stock: None,
            burden: 1,
            value: 22,
            sell_rate: 1.55,
        };
        assert_eq!(taper.bill(150), 5115);
        assert_eq!(taper.affordable(5109), 149);
        assert!(taper.bill(149) <= 5109);
        // Worth unknown: one charge each, as the counter's own price says.
        let unknown = Ware {
            value: 0,
            sell_rate: 0.0,
            ..taper.clone()
        };
        assert_eq!(unknown.bill(150), 5100);
        assert_eq!(unknown.affordable(5109), 150);
    }

    #[test]
    fn it_stands_at_the_counter_before_asking_it_to_trade() {
        let mut run = Run::new();
        let mut s = snap(vec![item(1, "Dagger", 500, 1, 1)]);
        s.counter.as_mut().unwrap().away = 12.0;
        assert_eq!(
            run.step(&s, Instant::now()).act,
            Some(Act::Approach { guid: 900 })
        );
        // Near enough, but the window is shut.
        s.counter.as_mut().unwrap().away = 1.0;
        s.counter.as_mut().unwrap().open = false;
        assert_eq!(
            run.step(&s, Instant::now()).act,
            Some(Act::Open { guid: 900 })
        );
    }

    #[test]
    fn the_pack_is_compressed_before_anything_is_sold() {
        let mut run = Run::new();
        // Two part stacks join before the dagger sells: slots are what a sale runs out of.
        let mut a = item(1, "Taper", 100, 37, 1000);
        let mut b = item(2, "Taper", 100, 41, 1000);
        a.wcid = 20631;
        b.wcid = 20631;
        let s = snap(vec![a, b, item(3, "Dagger", 500, 1, 1)]);
        assert_eq!(
            run.step(&s, Instant::now()).act,
            Some(Act::Merge {
                from: 1,
                to: 2,
                amount: 37
            })
        );
    }

    #[test]
    fn an_act_refused_on_this_side_is_not_asked_for_again() {
        // Told a pour was refused on this side, the rules move on as after a counter's no.
        let now = Instant::now();
        let mut run = Run::new();
        let mut a = item(1, "Taper", 100, 37, 1000);
        let mut b = item(2, "Taper", 100, 41, 1000);
        a.wcid = 20631;
        b.wcid = 20631;
        let s = snap(vec![a, b, item(3, "Dagger", 500, 1, 1)]);
        let pour = run.step(&s, now).act.expect("nothing asked for");
        assert!(matches!(pour, Act::Merge { .. }));
        run.refused(&pour, now);
        let next = run.step(&s, now).act;
        assert!(
            matches!(next, Some(Act::Sell { .. })),
            "asked for the refused pour again: {next:?}"
        );

        // A too-dear stack is cut first; the cut refused, it is left be and the visit goes on.
        let mut run = Run::new();
        let mut s = snap(vec![item(4, "Pyreal Motes", 2_000, 10, 100)]);
        s.counter.as_mut().unwrap().max_value = 500;
        let cut = run.step(&s, now).act.expect("nothing asked for");
        assert_eq!(cut, Act::Split { guid: 4, amount: 2 });
        run.refused(&cut, now);
        let next = run.step(&s, now).act;
        assert_eq!(next, Some(Act::Close), "asked for the refused cut again");
    }

    #[test]
    fn one_thing_too_dear_for_the_counter_is_set_aside_and_the_rest_still_sold() {
        let now = Instant::now();
        let mut run = Run::new();
        // A single piece worth more than the counter will look at cannot be cut, so it is left be,
        // and the cheaper ones beside it are sold in the same visit rather than the visit ending.
        let mut s = snap(vec![
            item(1, "Heirloom Sword", 2_000_000, 1, 1),
            item(2, "Dagger", 500, 1, 1),
            item(3, "Buckler", 300, 1, 1),
        ]);
        s.counter.as_mut().unwrap().max_value = 1_000_000;
        let next = run.step(&s, now).act;
        assert_eq!(
            next,
            Some(Act::Sell { items: vec![2, 3] }),
            "the rest was not sold"
        );
    }

    #[test]
    fn a_visit_that_buys_nothing_it_came_for_says_why() {
        let now = Instant::now();
        let mut run = Run::new();
        let mut s = snap(vec![]);
        s.coin = 10_000;
        s.wants = vec![Want {
            wcid: 20631,
            name: "Prismatic Taper".into(),
            short: 99,
            urgent: true,
        }];
        let next = run.step(&s, now);
        assert_eq!(next.act, Some(Act::Close));
        assert!(
            next.saying
                .contains("bought no Prismatic Taper: not on this shelf"),
            "{}",
            next.saying
        );
    }

    #[test]
    fn a_pour_heavier_than_the_room_is_not_asked_for() {
        let mut run = Run::new();
        let mut a = item(1, "Taper", 100, 37, 1000);
        let mut b = item(2, "Taper", 100, 41, 1000);
        a.wcid = 20631;
        b.wcid = 20631;
        a.burden = 456;
        b.burden = 500;
        let mut s = snap(vec![a, b, item(3, "Dagger", 500, 1, 1)]);
        // 17 burden under the ceiling, as a live character was: the server weighs a merge as
        // though the source were picked up afresh, and refuses it without a word.
        s.carried = s.capacity * 3 - 17;
        // So it sells instead, which is what makes the character light enough to pour later.
        let next = run.step(&s, Instant::now());
        assert!(
            matches!(&next.act, Some(Act::Sell { .. })),
            "expected selling, got {:?} -- {}",
            next.act,
            next.saying
        );
    }

    #[test]
    fn a_sale_needs_room_for_its_money_before_the_goods_leave() {
        // The server counts the payment in new coin stacks and finds room before taking anything:
        // with no slot free nothing sells, and the run says so rather than asking again.
        let mut s = snap(vec![item(3, "Dagger", 500, 1, 1)]);
        s.slots_free = 0;
        let next = Run::new().step(&s, Instant::now());
        assert_eq!(next.act, Some(Act::Close), "{}", next.saying);
        assert!(matches!(next.did, Did::Blocked(_)), "{:?}", next.did);
        // One free slot holds one coin stack's worth of goods; slots they will free do not count.
        let mut s = snap(vec![
            item(3, "Dagger", 13_000, 1, 1),
            item(4, "Sword", 13_000, 1, 1),
        ]);
        s.slots_free = 1;
        let next = Run::new().step(&s, Instant::now());
        assert!(
            matches!(&next.act, Some(Act::Sell { items }) if items.len() == 1),
            "{:?} -- {}",
            next.act,
            next.saying
        );
    }

    #[test]
    fn selling_stops_to_make_notes_when_the_pack_is_low_on_room() {
        let mut run = Run::new();
        let mut s = snap(vec![item(3, "Dagger", 500, 1, 1)]);
        s.coin = 1_000_000;
        s.slots_free = 3; // at the reserve, not out of slots
        let next = run.step(&s, Instant::now());
        assert_eq!(
            next.act,
            Some(Act::Buy {
                wcid: NOTE,
                count: 3
            }),
            "{}",
            next.saying
        );
        // With room to spare it gets on with the selling instead.
        s.slots_free = 20;
        assert_eq!(
            run.step(&s, Instant::now()).act,
            Some(Act::Sell { items: vec![3] })
        );
    }

    #[test]
    fn the_note_bought_is_the_note_not_the_first_dear_ware_on_the_shelf() {
        // Sedor Wystan's shelf in the order the server sends it: a basinet at 1,750 stands ahead
        // of his largest note, the 1,000 at 1,150, and a ware priced over the face is not a note.
        let ware = |wcid: u32, name: &str, price: u32, burden: u32| Ware {
            wcid,
            name: name.into(),
            price,
            stock: None,
            burden,
            value: 0,
            sell_rate: 0.0,
        };
        let mut s = snap(vec![item(3, "Dagger", 500, 1, 1)]);
        let c = s.counter.as_mut().unwrap();
        c.name = "Sedor Wystan the Blacksmith".into();
        c.wares = vec![
            ware(4190, "Cestus", 63, 50),
            ware(35, "Chainmail Basinet", 1_750, 320),
            ware(2621, "Trade Note (100)", 115, 1),
            ware(2623, "Trade Note (1,000)", 1_150, 1),
            ware(2622, "Trade Note (500)", 575, 1),
        ];
        c.note_face = Some(1_000);
        c.note_wcid = Some(2623);
        s.coin = 201_307;
        s.slots_free = 2;
        let next = Run::new().step(&s, Instant::now());
        assert_eq!(
            next.act,
            Some(Act::Buy {
                wcid: 2623,
                count: 173
            }),
            "{}",
            next.saying
        );
        // A counter with no note on its shelf makes none, whatever else it sells dear.
        let c = s.counter.as_mut().unwrap();
        c.note_face = None;
        c.note_wcid = None;
        let next = Run::new().step(&s, Instant::now());
        assert!(
            !matches!(next.act, Some(Act::Buy { .. })),
            "{:?} -- {}",
            next.act,
            next.saying
        );
    }

    #[test]
    fn a_note_needs_a_free_slot_before_the_coin_leaves() {
        // The server finds a slot for each new stack before taking the coin, and a note stack
        // already carried lends no room: with none free no note is asked for, and the visit
        // ends saying why rather than asking for what the server must refuse.
        let mut s = snap(vec![item(3, "Dagger", 500, 1, 1)]);
        s.coin = 1_000_000;
        s.notes.insert(250_000, 1);
        s.slots_free = 0;
        let mut run = Run::new();
        let next = run.step(&s, Instant::now());
        assert!(
            !matches!(next.act, Some(Act::Buy { .. })),
            "{:?} -- {}",
            next.act,
            next.saying
        );
        assert_eq!(next.act, Some(Act::Close), "{}", next.saying);
        assert!(matches!(next.did, Did::Blocked(_)), "{:?}", next.did);
        // One slot holds one stack, 250 notes, however much more the purse would buy.
        s.coin = 100_000_000;
        s.slots_free = 1;
        s.counter.as_mut().unwrap().note_face = Some(1_000);
        let next = Run::new().step(&s, Instant::now());
        assert_eq!(
            next.act,
            Some(Act::Buy {
                wcid: NOTE,
                count: crate::errand::NOTE_STACK
            }),
            "{}",
            next.saying
        );
    }

    #[test]
    fn a_purchase_the_counter_turned_down_is_not_asked_for_again_this_visit() {
        // The server answers a refused purchase in words and then its usual UseDone, and the pack
        // is as it was: told of the words, the rules go on to the selling instead of asking again.
        let now = Instant::now();
        let mut s = snap(vec![item(3, "Leather Cap", 640, 1, 1)]);
        s.coin = 1_000_000;
        s.slots_free = 2;
        let mut run = Run::new();
        let asked = run.step(&s, now).act;
        assert_eq!(
            asked,
            Some(Act::Buy {
                wcid: NOTE,
                count: 3
            })
        );
        run.buy_refused(Answer::Never, now);
        let next = run.step(&s, now);
        assert_eq!(
            next.act,
            Some(Act::Sell { items: vec![3] }),
            "{}",
            next.saying
        );
        for _ in 0..50 {
            let next = run.step(&s, now);
            assert_ne!(next.act, asked, "asked again: {}", next.saying);
        }
        // For this visit: the next counter's rules ask for their notes afresh.
        assert_eq!(Run::new().step(&s, now).act, asked);
    }

    #[test]
    fn a_refusal_with_no_purchase_in_flight_holds_nothing() {
        // Words about a purchase that went through, or one made by hand, are nothing to the run.
        let now = Instant::now();
        let mut s = snap(vec![item(3, "Leather Cap", 640, 1, 1)]);
        s.coin = 1_000_000;
        s.slots_free = 2;
        let mut run = Run::new();
        let asked = run.step(&s, now).act;
        assert!(matches!(asked, Some(Act::Buy { .. })), "{asked:?}");
        // The notes came, and the next turn sells: the words heard now are about nothing of ours.
        s.coin = 2_000;
        let next = run.step(&s, now);
        assert_eq!(
            next.act,
            Some(Act::Sell { items: vec![3] }),
            "{}",
            next.saying
        );
        run.buy_refused(Answer::Never, now);
        assert!(run.wont_sell.is_empty(), "a refusal was pinned on nothing");
    }

    #[test]
    fn a_ware_refused_on_this_side_is_not_bought_again() {
        // No shelf line to name, say: the rules are told, and the trip finishes without it.
        let now = Instant::now();
        let mut s = snap(Vec::new());
        s.coin = 10_000;
        s.wants = vec![Want {
            wcid: 20631,
            name: "Prismatic Taper".into(),
            short: 100,
            urgent: true,
        }];
        s.counter.as_mut().unwrap().wares.push(Ware {
            wcid: 20631,
            name: "Prismatic Taper".into(),
            price: 26,
            stock: None,
            burden: 6,
            value: 0,
            sell_rate: 0.0,
        });
        let mut run = Run::new();
        let buy = run.step(&s, now).act.expect("nothing asked for");
        assert!(matches!(buy, Act::Buy { wcid: 20631, .. }), "{buy:?}");
        run.refused(&buy, now);
        let next = run.step(&s, now);
        assert_eq!(next.act, Some(Act::Close), "{}", next.saying);
        assert_eq!(run.phase, Phase::Done);
    }

    #[test]
    fn the_things_the_character_lives_on_are_not_stock() {
        // The loot policy's keep (`Keep::mine`) is decided before the snapshot and covers a full
        // stock: the wants skip spares only a component the character is short of.
        let now = Instant::now();
        let mut tapers = item(4, "Prismatic Taper", 5_000, 1000, 1000);
        tapers.keep.mine = true;
        let mut run = Run::new();
        let next = run.step(&snap(vec![tapers]), now);
        assert_ne!(
            next.act,
            Some(Act::Sell { items: vec![4] }),
            "sold the tapers: {}",
            next.saying
        );
        assert_eq!(
            Keep {
                mine: true,
                ..Default::default()
            }
            .why(),
            Some("the character lives on it")
        );
    }

    #[test]
    fn what_must_never_be_sold_is_never_offered() {
        let now = Instant::now();
        for (what, mut keep) in [
            ("tinkered", Keep::default()),
            ("inscribed", Keep::default()),
            ("equipped", Keep::default()),
            ("retained", Keep::default()),
            ("unsellable", Keep::default()),
            ("mine", Keep::default()),
        ] {
            match what {
                "tinkered" => keep.tinkered = true,
                "inscribed" => keep.inscribed = true,
                "equipped" => keep.equipped = true,
                "retained" => keep.retained = true,
                "mine" => keep.mine = true,
                _ => keep.unsellable = true,
            }
            let mut it = item(3, "Sword", 50_000, 1, 1);
            it.keep = keep;
            let mut run = Run::new();
            let next = run.step(&snap(vec![it]), now);
            assert_ne!(
                next.act,
                Some(Act::Sell { items: vec![3] }),
                "offered a {what} item: {}",
                next.saying
            );
        }
    }

    #[test]
    fn a_stack_too_dear_for_the_counter_is_cut_down_rather_than_given_up() {
        let mut run = Run::new();
        // A hundred peas worth five million, at a counter that will not
        // look at anything over a million.
        let mut s = snap(vec![item(4, "Pyreal Pea", 5_000_000, 100, 100)]);
        s.counter.as_mut().unwrap().max_value = 1_000_000;
        assert_eq!(
            run.step(&s, Instant::now()).act,
            Some(Act::Split {
                guid: 4,
                amount: 20
            })
        );
    }

    #[test]
    fn an_item_the_counter_would_not_take_is_not_offered_twice() {
        let now = Instant::now();
        let mut run = Run::new();
        let s = snap(vec![item(3, "Dagger", 500, 1, 1)]);
        assert_eq!(run.step(&s, now).act, Some(Act::Sell { items: vec![3] }));
        // Still in the pack a moment later, so the counter would not have it: not offered again.
        let next = run.step(&s, now);
        assert!(
            !matches!(&next.act, Some(Act::Sell { items }) if items.contains(&3)),
            "{}",
            next.saying
        );
    }

    #[test]
    fn what_the_character_came_to_buy_is_not_sold_first() {
        let mut run = Run::new();
        let mut taper = item(1, "Taper", 2_000, 78, 1000);
        taper.wcid = 20631;
        let mut s = snap(vec![taper, item(3, "Dagger", 500, 1, 1)]);
        s.wants = vec![Want {
            wcid: 20631,
            name: "Prismatic Taper".into(),
            short: 500,
            urgent: true,
        }];
        // The dagger, not the tapers, however much they are worth.
        let next = run.step(&s, Instant::now());
        assert_eq!(
            next.act,
            Some(Act::Sell { items: vec![3] }),
            "{}",
            next.saying
        );
    }

    #[test]
    fn what_the_player_said_to_sell_goes_even_when_it_is_on_the_list() {
        // A pea tagged Sell that is also on the shopping list is still offered: the player's word
        // beats the list's skip (`Snapshot::offers`), and the list still keeps the tapers.
        let mut run = Run::new();
        let mut pea = item(1, "Lead Pea", 500, 1, 100);
        pea.wcid = 8329;
        pea.taken_for = Some(LootAction::Sell);
        let mut taper = item(2, "Prismatic Taper", 2_000, 78, 1000);
        taper.wcid = 20631;
        let mut s = snap(vec![pea, taper]);
        s.wants = vec![
            Want {
                wcid: 8329,
                name: "Lead Pea".into(),
                short: 99,
                urgent: false,
            },
            Want {
                wcid: 20631,
                name: "Prismatic Taper".into(),
                short: 500,
                urgent: true,
            },
        ];
        assert!(s.offers(&s.items[0]), "the pea the player said to sell");
        assert!(!s.offers(&s.items[1]), "the tapers it is short of");
        let next = run.step(&s, Instant::now());
        assert_eq!(
            next.act,
            Some(Act::Sell { items: vec![1] }),
            "{}",
            next.saying
        );
    }

    #[test]
    fn a_stack_the_player_said_to_sell_is_not_poured_into_one_they_said_to_keep() {
        // Ten scarabs tagged Sell past a cap, ninety-five kept: a pour would settle the whole as
        // kept and offer nothing, so the two stay apart and the ten go over the counter whole.
        let mut run = Run::new();
        let mut to_sell = item(1, "Lead Scarab", 50, 10, 100);
        to_sell.wcid = 691;
        to_sell.taken_for = Some(LootAction::Sell);
        let mut kept = item(2, "Lead Scarab", 475, 95, 100);
        kept.wcid = 691;
        kept.taken_for = Some(LootAction::Keep);
        kept.keep.mine = true;
        let s = snap(vec![to_sell, kept]);
        let next = run.step(&s, Instant::now());
        assert_eq!(
            next.act,
            Some(Act::Sell { items: vec![1] }),
            "{}",
            next.saying
        );

        // The same for a stack nothing was decided about: undecided is
        // not the same word as "sell", and a pour would make it so.
        let mut run = Run::new();
        let mut to_sell = item(1, "Lead Scarab", 50, 10, 100);
        to_sell.wcid = 691;
        to_sell.taken_for = Some(LootAction::Sell);
        let mut undecided = item(2, "Lead Scarab", 475, 95, 100);
        undecided.wcid = 691;
        undecided.keep.mine = true;
        let s = snap(vec![to_sell, undecided]);
        let next = run.step(&s, Instant::now());
        assert_eq!(
            next.act,
            Some(Act::Sell { items: vec![1] }),
            "{}",
            next.saying
        );

        // Two stacks with the same word are still tidied.
        let mut run = Run::new();
        let mut a = item(1, "Lead Scarab", 50, 10, 100);
        a.wcid = 691;
        a.taken_for = Some(LootAction::Sell);
        let mut b = item(2, "Lead Scarab", 100, 20, 100);
        b.wcid = 691;
        b.taken_for = Some(LootAction::Sell);
        let s = snap(vec![a, b]);
        let next = run.step(&s, Instant::now());
        assert_eq!(
            next.act,
            Some(Act::Merge {
                from: 1,
                to: 2,
                amount: 10
            }),
            "{}",
            next.saying
        );
    }

    #[test]
    fn the_counter_is_offered_only_what_it_buys() {
        // The panel's count and the selling ask one question: a tailor's window does not take
        // a pea, whatever the player said about it.
        let mut pea = item(1, "Lead Pea", 500, 1, 100);
        pea.item_type = 0x1000;
        pea.taken_for = Some(LootAction::Sell);
        let mut s = snap(vec![pea, item(2, "Tunic", 300, 1, 1)]);
        s.items[1].item_type = 0x8;
        assert!(s.offers(&s.items[0]), "a counter that buys anything");
        s.counter.as_mut().unwrap().buys = 0x8 | 0x4;
        assert!(!s.offers(&s.items[0]), "a tailor does not take peas");
        assert!(s.offers(&s.items[1]), "but takes a tunic");
        assert_eq!(s.items.iter().filter(|i| s.offers(i)).count(), 1);
        // No counter, nothing on offer.
        s.counter = None;
        assert!(!s.offers(&s.items[1]));
    }

    #[test]
    fn a_purse_of_notes_is_cashed_before_a_small_bill_is_paid() {
        let mut run = Run::new();
        let mut s = snap(Vec::new());
        s.coin = 0;
        s.notes.insert(250_000, 4);
        s.wants = vec![Want {
            wcid: 20631,
            name: "Prismatic Taper".into(),
            short: 100,
            urgent: true,
        }];
        s.counter.as_mut().unwrap().wares.push(Ware {
            wcid: 20631,
            name: "Prismatic Taper".into(),
            price: 26,
            stock: None,
            burden: 6,
            value: 0,
            sell_rate: 0.0,
        });
        // 2,600 of bill and not a coin to pay it with.
        let next = run.step(&s, Instant::now());
        assert_eq!(
            next.act,
            Some(Act::Cash {
                face: 250_000,
                count: 1
            }),
            "{}",
            next.saying
        );
    }

    #[test]
    fn a_laden_character_buys_only_what_it_can_carry_home() {
        let mut run = Run::new();
        let mut s = snap(Vec::new());
        s.coin = 10_000_000;
        s.rules.float = 10_000_000;
        // Sixty units of room, and a taper weighs six.
        s.carried = s.capacity * 3 - 60;
        s.wants = vec![Want {
            wcid: 20631,
            name: "Prismatic Taper".into(),
            short: 500,
            urgent: true,
        }];
        s.counter.as_mut().unwrap().wares.push(Ware {
            wcid: 20631,
            name: "Prismatic Taper".into(),
            price: 26,
            stock: None,
            burden: 6,
            value: 0,
            sell_rate: 0.0,
        });
        let next = run.step(&s, Instant::now());
        assert_eq!(
            next.act,
            Some(Act::Buy {
                wcid: 20631,
                count: 10
            }),
            "{}",
            next.saying
        );
    }

    #[test]
    fn a_counter_is_handed_an_armful_not_one_thing_at_a_time() {
        let mut run = Run::new();
        let s = snap(vec![
            item(1, "Dagger", 500, 1, 1),
            item(2, "Shield", 900, 1, 1),
            item(3, "Helm", 700, 1, 1),
        ]);
        match run.step(&s, Instant::now()).act {
            Some(Act::Sell { items }) => {
                // Dearest first, and all three in one go.
                assert_eq!(items, vec![2, 3, 1]);
            }
            other => panic!("expected an armful, got {other:?}"),
        }
    }

    #[test]
    fn the_armful_stops_where_the_coin_would_fill_the_pack() {
        let mut run = Run::new();
        // Each ingot pays two slots of coin, and only five slots are free.
        let mut s = snap((1..=8).map(|g| item(g, "Ingot", 50_000, 1, 1)).collect());
        s.slots_free = 5;
        s.rules.keep_slots = 3;
        match run.step(&s, Instant::now()).act {
            Some(Act::Sell { items }) => {
                assert!(
                    items.len() < 8,
                    "sold the lot with nowhere to put the money: {items:?}"
                );
                assert!(!items.is_empty());
            }
            other => panic!("expected an armful, got {other:?}"),
        }
    }

    #[test]
    fn buying_comes_after_selling_and_stays_inside_the_purse() {
        let mut run = Run::new();
        let mut s = snap(Vec::new());
        s.coin = 600_000;
        s.rules.float = 600_000; // nothing spare to turn into notes
        s.wants = vec![Want {
            wcid: NOTE,
            name: "Mayoi Trade Note".into(),
            short: 10,
            urgent: false,
        }];
        // Nothing to sell, so it buys -- but only what it can pay for:
        // 600,000 buys two at 287,500, not the ten asked for.
        let next = run.step(&s, Instant::now());
        assert_eq!(
            next.act,
            Some(Act::Buy {
                wcid: NOTE,
                count: 2
            }),
            "{}",
            next.saying
        );
        let _ = COIN;
    }
}
