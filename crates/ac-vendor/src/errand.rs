//! The counter's money arithmetic, shared by the sale at the window (`crate::Run`): coin and notes
//! by the pack slot, and what one armful of a sale may be paid into.

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

/// Which of `pays` (indices, in order) one armful sells, and what the counter pays for it; one
/// whose payment would not fit is left for later and cheaper ones still go. Whole new coin stacks
/// weighed against the slots free before any goods leave, so the sold slots and the carried pile do
/// not count (Player_Commerce.cs:178,:182,:199; ItemsToReceive.cs:41,:103).
pub fn armful_within_slots(pays: &[u32], slots_free: u32) -> (Vec<usize>, u32) {
    let mut takings = 0u32;
    let mut taken = Vec::new();
    for (i, pay) in pays.iter().enumerate() {
        let after = takings.saturating_add(*pay);
        if coin_slots(after) > slots_free {
            continue;
        }
        takings = after;
        taken.push(i);
    }
    (taken, takings)
}

pub fn note_cost(face: u32) -> u32 {
    (face as f32 * NOTE_MARKUP).ceil() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_armful_is_what_the_free_slots_can_be_paid_into() {
        // One slot free: 25,000 in coin fits, the next 30,000 would need a second stack and waits,
        // and a cheaper thing after it still goes.
        let (taken, takings) = armful_within_slots(&[20_000, 30_000, 4_000], 1);
        assert_eq!(taken, vec![0, 2]);
        assert_eq!(takings, 24_000);
        // No slot at all, nothing is sold: the counter would have nowhere to put the money.
        assert_eq!(armful_within_slots(&[10], 0), (vec![], 0));
    }

    #[test]
    fn a_note_costs_its_face_and_the_counter_s_markup() {
        assert_eq!(note_cost(250_000), 287_500);
        assert_eq!(coin_slots(25_001), 2);
    }
}
