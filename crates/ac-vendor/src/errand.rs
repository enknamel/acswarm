//! What a whole trip to a counter will do, worked out before the character leaves.
//! [`plan`] walks the means forward (sell, cash, buy, convert): each act takes what it costs and
//! gives back what it yields, so "too laden, sell first" falls out of the order, not a rule.
//! Notes weigh nothing but a round trip through them loses 13%; [`crate::Run`] decides act by act.
//! See `docs/agent.md`.

use ac_agent::did::Because;

/// What a character has to spend on a trip, and what it has room for.
/// Coin and notes are apart because a counter takes coin: notes must be cashed to pay.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Means {
    /// Spendable at the counter, now.
    pub coin: u32,
    /// Face value of the trade notes carried.
    pub notes: u32,
    /// Burden units the character can still be handed.
    pub room: u32,
    /// Pack slots free.
    pub slots: u32,
}

impl Means {
    /// Everything the purse is worth, cashed or not.
    pub fn purse(&self) -> u32 {
        self.coin.saturating_add(self.notes)
    }
}

/// Something in the pack that this counter would buy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForSale {
    pub guid: u32,
    /// What the counter pays for the whole stack.
    pub pays: u32,
    /// Its burden, which selling gives back.
    pub weighs: u32,
}

/// A note in the pack, and what cashing it is worth.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub guid: u32,
    /// A note pays its face value back; only making one costs.
    pub face: u32,
}

/// A line on the shelf that answers something the character wants.
#[derive(Clone, Debug, PartialEq)]
pub struct Wanted {
    /// The vendor's stock line to buy from.
    pub line: u32,
    pub name: String,
    /// How many are wanted.
    pub want: u32,
    /// Price of one.
    pub each: u32,
    /// Burden of one.
    pub weighs: u32,
    /// How many the shelf has; `None` for an endless supply.
    pub stock: Option<u32>,
}

/// One thing the trip does, in the order it does it.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    /// Hand these over; always first, as it pays for the rest and frees room to carry it.
    Sell { items: Vec<u32>, takings: u32 },
    /// Turn these notes back into coin, because the counter takes coin.
    Cash { notes: Vec<u32>, coin: u32 },
    /// Buy this many off this line.
    Buy {
        line: u32,
        name: String,
        amount: u32,
        cost: u32,
    },
    /// Turn coin into `count` trade notes of this face, costing `spend`.
    /// Also mid-sale: the coin a sale takes in fills the pack (see [`COIN_STACK`]).
    Keep { face: u32, count: u32, spend: u32 },
}

/// What the trip will do, what it will leave behind, and what it will
/// not manage.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub acts: Vec<Act>,
    /// The means as the trip would leave them.
    pub left: Means,
    /// What was wanted and will not be had, and why not.
    pub unmet: Vec<(String, u32, Because)>,
}

impl Plan {
    /// Whether it sells or buys anything; cashing and making notes alone are no reason to go.
    pub fn worth_going(&self) -> bool {
        self.acts
            .iter()
            .any(|a| !matches!(a, Act::Keep { .. } | Act::Cash { .. }))
    }

    /// Coin spent on buying and on making notes.
    pub fn cost(&self) -> u32 {
        self.acts
            .iter()
            .map(|a| match a {
                Act::Buy { cost, .. } => *cost,
                Act::Keep { spend, .. } => *spend,
                _ => 0,
            })
            .sum()
    }

    pub fn takings(&self) -> u32 {
        self.acts
            .iter()
            .map(|a| match a {
                Act::Sell { takings, .. } => *takings,
                _ => 0,
            })
            .sum()
    }

    /// Said in a line, for the log.
    pub fn tell(&self) -> String {
        let bought: u32 = self
            .acts
            .iter()
            .filter_map(|a| match a {
                Act::Buy { amount, .. } => Some(*amount),
                _ => None,
            })
            .sum();
        let sold = self
            .acts
            .iter()
            .filter_map(|a| match a {
                Act::Sell { items, .. } => Some(items.len()),
                _ => None,
            })
            .sum::<usize>();
        let mut out = format!("sell {sold}, buy {bought}");
        if self.takings() > 0 {
            out.push_str(&format!(", take {}", self.takings()));
        }
        if self.cost() > 0 {
            out.push_str(&format!(", spend {}", self.cost()));
        }
        if !self.unmet.is_empty() {
            out.push_str(&format!("; {} line(s) short", self.unmet.len()));
        }
        out
    }
}

/// Pyreals kept loose to shop with rather than made into notes, which cost to make (guess).
pub const FLOAT: u32 = 2_000;

/// Pyreals per pack slot (max stack); coin weighs nothing, so slots, not burden, stop a big sale.
pub const COIN_STACK: u32 = 25_000;
/// Trade notes per pack slot (max stack): a slot of 250,000 notes holds 62.5 million.
pub const NOTE_STACK: u32 = 250;

/// Trade-note price as a multiple of face, whatever the shop's rate (ACE Vendor.cs:581).
/// A note sells back at face (Vendor.cs:595), so a purse turned into notes and back is 13% lighter.
pub const NOTE_MARKUP: f32 = 1.15;

pub fn coin_slots(coin: u32) -> u32 {
    coin.div_ceil(COIN_STACK)
}

pub fn note_cost(face: u32) -> u32 {
    (face as f32 * NOTE_MARKUP).ceil() as u32
}

/// Work out what the trip does: sell, cash, buy, then carry what is left home as notes.
/// `note_face` is the note this counter makes; without one a sale stops when the pack is full.
pub fn plan(
    means: Means,
    sale: &[ForSale],
    notes: &[Note],
    wanted: &[Wanted],
    float: u32,
    note_face: Option<u32>,
) -> Plan {
    let mut left = means;
    let mut plan = Plan::default();

    // 1. Sell in rounds: coin fills slots, so a round stops to convert before the next.
    let mut to_sell: Vec<&ForSale> = sale.iter().collect();
    while !to_sell.is_empty() {
        let before = to_sell.len();
        let mut batch = Vec::new();
        let mut takings = 0;
        let mut lighter = 0;
        // Free slots as the round goes, since each sale frees the item's own slot.
        let mut slots = left.slots;
        let mut coin = left.coin;
        while let Some(item) = to_sell.first() {
            let would = coin.saturating_add(item.pays);
            // Sold only if the free slots, the item's own slot and the slots its coin already
            // fills cover the coin after it.
            let have = slots + 1 + coin_slots(coin);
            let needs = coin_slots(would);
            if have < needs {
                break;
            }
            let after = have - needs;
            batch.push(item.guid);
            takings += item.pays;
            lighter += item.weighs;
            coin = would;
            slots = after;
            to_sell.remove(0);
        }
        if !batch.is_empty() {
            left.coin = coin;
            left.slots = slots;
            left.room = left.room.saturating_add(lighter);
            plan.acts.push(Act::Sell {
                items: batch,
                takings,
            });
        }
        if to_sell.is_empty() {
            break;
        }
        // Pack full of change with loot left: converting is the only thing that makes room.
        if !convert(&mut left, &mut plan, note_face, float) || to_sell.len() == before {
            // Nothing to convert, or this round sold nothing: the rest of the loot goes home.
            let short = to_sell.len() as u32;
            plan.unmet.push((
                "loot to sell".to_string(),
                short,
                Because::ours("no room for the money it would make"),
            ));
            break;
        }
    }

    // 2. Cash notes only for what the shopping will spend: a counter takes coin, not notes.
    let bill = affordable_bill(&left, wanted);
    if bill > left.coin && !notes.is_empty() {
        let short = bill - left.coin;
        let mut cashed = Vec::new();
        let mut coin = 0;
        // Smallest first, so no more of the fortune is unpacked than the bill needs.
        let mut by_size: Vec<&Note> = notes.iter().collect();
        by_size.sort_by_key(|n| n.face);
        for note in by_size {
            if coin >= short {
                break;
            }
            coin += note.face;
            cashed.push(note.guid);
        }
        if !cashed.is_empty() {
            left.coin = left.coin.saturating_add(coin);
            left.notes = left.notes.saturating_sub(coin);
            plan.acts.push(Act::Cash {
                notes: cashed,
                coin,
            });
        }
    }

    // 3. Buy in the order asked, each line held to whichever of stock, purse and room ends first.
    for want in wanted {
        if want.want == 0 {
            continue;
        }
        let mut amount = want.want;
        let mut stopped: Option<Because> = None;
        if let Some(on_shelf) = want.stock {
            if on_shelf < amount {
                amount = on_shelf;
                stopped = Some(Because::ours("the shelf has no more"));
            }
        }
        // A free line is not held by the purse, nor a weightless one (a trade note) by burden.
        if let Some(afford) = left.coin.checked_div(want.each) {
            if afford < amount {
                amount = afford;
                stopped = Some(Because::ours("not enough money"));
            }
        }
        if let Some(carry) = left.room.checked_div(want.weighs) {
            if carry < amount {
                amount = carry;
                stopped = Some(Because::ours("too laden to carry any more"));
            }
        }
        if amount == 0 {
            plan.unmet.push((
                want.name.clone(),
                want.want,
                stopped.unwrap_or_else(|| Because::ours("nothing on the shelf")),
            ));
            continue;
        }
        let cost = amount.saturating_mul(want.each);
        left.coin = left.coin.saturating_sub(cost);
        left.room = left.room.saturating_sub(amount.saturating_mul(want.weighs));
        plan.acts.push(Act::Buy {
            line: want.line,
            name: want.name.clone(),
            amount,
            cost,
        });
        if amount < want.want {
            plan.unmet.push((
                want.name.clone(),
                want.want - amount,
                stopped.unwrap_or_else(|| Because::ours("not all of it was to be had")),
            ));
        }
    }

    // 4. Coin above the float goes home as notes, last so the shopping is already paid for
    //    (test: the_money_the_shopping_needs_is_not_turned_into_notes).
    convert(&mut left, &mut plan, note_face, float);

    plan.left = left;
    plan
}

/// Turn the coin above the float into trade notes, and say whether any were made.
/// Always loses the markup, but a slot of notes holds 2,500 times what a slot of coin does.
fn convert(left: &mut Means, plan: &mut Plan, face: Option<u32>, float: u32) -> bool {
    // One denomination, the largest (Mayoi, 250,000): every note costs 15% over face, so a
    // smaller one pays the same toll to carry a fraction as much.
    let Some(face) = face.filter(|f| *f > 0) else {
        return false;
    };
    let each = note_cost(face).max(1);
    let count = left.coin.saturating_sub(float) / each;
    if count == 0 {
        return false;
    }
    // Only when it frees a slot, which a Mayoi always does: 250,000 in coin is ten slots.
    let spend = count * each;
    if spend < COIN_STACK {
        return false;
    }
    let before = coin_slots(left.coin);
    left.coin -= spend;
    left.notes = left.notes.saturating_add(count * face);
    left.slots = left.slots.saturating_sub(count.div_ceil(NOTE_STACK));
    left.slots = left.slots + before - coin_slots(left.coin);
    plan.acts.push(Act::Keep { face, count, spend });
    true
}

/// What the wanted lines would cost as far as shelf and burden allow.
/// Caps the notes cashed, so a fortune is not unpacked for goods that cannot be carried.
fn affordable_bill(means: &Means, wanted: &[Wanted]) -> u32 {
    let mut room = means.room;
    let mut bill: u32 = 0;
    for want in wanted {
        let mut amount = want.want;
        if let Some(on_shelf) = want.stock {
            amount = amount.min(on_shelf);
        }
        if let Some(carry) = room.checked_div(want.weighs) {
            amount = amount.min(carry);
        }
        room = room.saturating_sub(amount.saturating_mul(want.weighs));
        bill = bill.saturating_add(amount.saturating_mul(want.each));
    }
    bill
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Mayoi trade note's face, 250,000: the only note worth making.
    const MMD: Option<u32> = Some(250_000);

    fn means(coin: u32, notes: u32, room: u32) -> Means {
        Means {
            coin,
            notes,
            room,
            slots: 20,
        }
    }

    fn want(line: u32, name: &str, n: u32, each: u32, weighs: u32) -> Wanted {
        Wanted {
            line,
            name: name.into(),
            want: n,
            each,
            weighs,
            stock: None,
        }
    }

    fn bought(plan: &Plan, name: &str) -> u32 {
        plan.acts
            .iter()
            .filter_map(|a| match a {
                Act::Buy {
                    name: n, amount, ..
                } if n == name => Some(*amount),
                _ => None,
            })
            .sum()
    }

    #[test]
    fn selling_comes_first_because_it_pays_for_the_rest() {
        // No coin, but loot: whether the shopping is affordable is judged after the sale.
        let loot = [
            ForSale {
                guid: 1,
                pays: 5_000,
                weighs: 300,
            },
            ForSale {
                guid: 2,
                pays: 3_000,
                weighs: 200,
            },
        ];
        let p = plan(
            means(0, 0, 100),
            &loot,
            &[],
            &[want(9, "Lead Scarab", 100, 50, 10)],
            FLOAT,
            MMD,
        );
        assert!(matches!(p.acts.first(), Some(Act::Sell { .. })));
        assert_eq!(p.takings(), 8_000);
        // The loot's 500 burden plus 100 room carries 60 scarabs at 10 each, not the 100 wanted.
        assert_eq!(bought(&p, "Lead Scarab"), 60);
        assert!(p
            .unmet
            .iter()
            .any(|(n, _, b)| n == "Lead Scarab" && b.what.contains("laden")));
    }

    #[test]
    fn weight_holds_an_order_back_as_surely_as_money_does() {
        // Rich, with no burden room: nothing bought, and the plan says why.
        let p = plan(
            means(1_000_000, 0, 0),
            &[],
            &[],
            &[want(9, "Lead Scarab", 100, 5, 10)],
            FLOAT,
            MMD,
        );
        assert_eq!(bought(&p, "Lead Scarab"), 0);
        assert!(!p.worth_going(), "nothing to sell and nothing it can lift");
        assert_eq!(
            p.unmet.first().map(|(_, n, b)| (*n, b.what.as_str())),
            Some((100, "too laden to carry any more"))
        );

        // Room for a quarter of it buys a quarter of it.
        let p = plan(
            means(1_000_000, 0, 250),
            &[],
            &[],
            &[want(9, "Lead Scarab", 100, 5, 10)],
            FLOAT,
            MMD,
        );
        assert_eq!(bought(&p, "Lead Scarab"), 25);
    }

    #[test]
    fn the_money_the_shopping_needs_is_not_turned_into_notes() {
        let p = plan(
            means(10_000, 0, 100_000),
            &[],
            &[],
            &[want(9, "Prismatic Taper", 500, 15, 1)],
            FLOAT,
            MMD,
        );
        assert_eq!(bought(&p, "Prismatic Taper"), 500);
        // 7,500 spent leaves 2,500: the 500 above the float buys no Mayoi note, so it stays change.
        assert_eq!(p.cost(), 500 * 15);
        assert!(!p.acts.iter().any(|a| matches!(a, Act::Keep { .. })));
        assert_eq!(p.left.coin, 2_500);
        // Buying comes before converting, always: with a fortune both happen, in that order.
        let rich = plan(
            means(1_000_000, 0, 100_000),
            &[],
            &[],
            &[want(9, "Prismatic Taper", 500, 15, 1)],
            FLOAT,
            MMD,
        );
        let buy_at = rich
            .acts
            .iter()
            .position(|a| matches!(a, Act::Buy { .. }))
            .expect("bought something");
        let keep_at = rich
            .acts
            .iter()
            .position(|a| matches!(a, Act::Keep { .. }))
            .expect("kept something");
        assert!(buy_at < keep_at, "the tapers are paid for first");
        // All the tapers were bought before any coin was packed into notes.
        assert_eq!(bought(&rich, "Prismatic Taper"), 500);
    }

    #[test]
    fn a_big_sale_stops_to_convert_and_goes_on_selling() {
        // 40 x 50,000 is two million pyreals: eighty slots of coin, and ten slots free.
        let loot: Vec<ForSale> = (0..40)
            .map(|i| ForSale {
                guid: 100 + i,
                pays: 50_000,
                weighs: 100,
            })
            .collect();
        let p = plan(
            Means {
                coin: 0,
                notes: 0,
                room: 1_000_000,
                slots: 10,
            },
            &loot,
            &[],
            &[],
            FLOAT,
            MMD,
        );

        // It sells in rounds, converting in between.
        let sells = p
            .acts
            .iter()
            .filter(|a| matches!(a, Act::Sell { .. }))
            .count();
        let keeps = p
            .acts
            .iter()
            .filter(|a| matches!(a, Act::Keep { .. }))
            .count();
        assert!(sells > 1, "one sale of forty was never going to work");
        assert!(keeps >= 1, "and it has to convert to carry on");

        // Every piece of loot goes.
        let sold: usize = p
            .acts
            .iter()
            .filter_map(|a| match a {
                Act::Sell { items, .. } => Some(items.len()),
                _ => None,
            })
            .sum();
        assert_eq!(sold, 40, "the whole pile went over the counter");
        assert!(p.unmet.is_empty(), "nothing was carried home again");

        // The takings came home as notes, not eighty slots of change.
        assert!(p.left.notes > 1_000_000, "{} in notes", p.left.notes);
        // Coin left is under one Mayoi's 287,500 cost plus the float: there is no smaller lump.
        assert!(
            p.left.coin < note_cost(250_000) + FLOAT,
            "{} left as coin",
            p.left.coin
        );
    }

    #[test]
    fn a_counter_that_makes_no_notes_sells_what_it_can_and_says_so() {
        // No note to convert into: the sale stops when the pack does, and says so.
        let loot: Vec<ForSale> = (0..40)
            .map(|i| ForSale {
                guid: 100 + i,
                pays: 50_000,
                weighs: 100,
            })
            .collect();
        let p = plan(
            Means {
                coin: 0,
                notes: 0,
                room: 1_000_000,
                slots: 4,
            },
            &loot,
            &[],
            &[],
            FLOAT,
            None,
        );
        let sold: usize = p
            .acts
            .iter()
            .filter_map(|a| match a {
                Act::Sell { items, .. } => Some(items.len()),
                _ => None,
            })
            .sum();
        assert!(sold > 0, "it sold what it had room for");
        assert!(sold < 40, "and not a penny more");
        assert!(
            p.unmet
                .iter()
                .any(|(what, _, why)| what == "loot to sell" && why.what.contains("no room")),
            "and said why the rest came home"
        );
    }

    #[test]
    fn notes_are_cashed_for_the_shopping_and_no_further() {
        // Notes and no coin: it cashes what the bill needs, smallest first, and leaves the rest.
        let notes = [
            Note { guid: 1, face: 250 },
            Note {
                guid: 2,
                face: 5_000,
            },
            Note {
                guid: 3,
                face: 50_000,
            },
        ];
        let p = plan(
            means(0, 55_250, 100_000),
            &[],
            &notes,
            &[want(9, "Lead Scarab", 100, 40, 10)],
            FLOAT,
            MMD,
        );
        let cashed = p.acts.iter().find_map(|a| match a {
            Act::Cash { notes, coin } => Some((notes.clone(), *coin)),
            _ => None,
        });
        let (cashed, coin) = cashed.expect("cashed something");
        assert_eq!(
            coin, 5_250,
            "the small ones covered a bill of four thousand"
        );
        assert_eq!(cashed, vec![1, 2], "the fifty thousand stayed a note");
        assert_eq!(bought(&p, "Lead Scarab"), 100);
        // Cashing comes before buying, or the counter is handed paper.
        let cash_at = p
            .acts
            .iter()
            .position(|a| matches!(a, Act::Cash { .. }))
            .expect("cashed");
        let buy_at = p
            .acts
            .iter()
            .position(|a| matches!(a, Act::Buy { .. }))
            .expect("bought");
        assert!(cash_at < buy_at);
    }

    #[test]
    fn a_shelf_that_runs_out_is_said_so_rather_than_asked_again() {
        let mut scarce = want(9, "Mana Scarab", 100, 15_000, 5);
        scarce.stock = Some(6);
        let p = plan(
            means(1_000_000, 0, 100_000),
            &[],
            &[],
            &[scarce],
            FLOAT,
            MMD,
        );
        assert_eq!(bought(&p, "Mana Scarab"), 6);
        assert_eq!(
            p.unmet.first().map(|(_, n, b)| (*n, b.what.as_str())),
            Some((94, "the shelf has no more"))
        );
    }

    #[test]
    fn a_trip_that_does_nothing_says_so_before_it_is_walked() {
        // No money and nothing to sell: whatever is on the shelf, no reason to go.
        let p = plan(
            means(0, 0, 100_000),
            &[],
            &[],
            &[want(9, "Lead Scarab", 100, 50, 10)],
            FLOAT,
            MMD,
        );
        assert!(!p.worth_going());
        assert_eq!(p.cost(), 0);
        assert!(p.unmet.iter().any(|(_, _, b)| b.what.contains("money")));

        // Something to sell is reason enough, even with nothing to buy.
        let p = plan(
            means(0, 0, 100_000),
            &[ForSale {
                guid: 1,
                pays: 40,
                weighs: 10,
            }],
            &[],
            &[],
            FLOAT,
            MMD,
        );
        assert!(p.worth_going());
        assert_eq!(p.takings(), 40);
    }

    #[test]
    fn the_whole_errand_reads_as_one_line() {
        let p = plan(
            means(0, 0, 100_000),
            &[ForSale {
                guid: 1,
                pays: 9_000,
                weighs: 100,
            }],
            &[],
            &[want(9, "Lead Scarab", 100, 40, 10)],
            FLOAT,
            MMD,
        );
        let said = p.tell();
        assert!(said.contains("sell 1"), "{said}");
        assert!(said.contains("buy 100"), "{said}");
        assert!(said.contains("take 9000"), "{said}");
    }
}
