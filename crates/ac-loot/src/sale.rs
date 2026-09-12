//! What may go over a counter, and what may not.
//!
//! The bars a sale has to get past, in one place, because they were in
//! four and the path a loot profile took went through none of them.
//!
//! Nothing here needs a server, a socket or a pack: it is a question
//! about an item and about what the character has already decided, and
//! it is answered the same way whether it is asked before setting off
//! (is this trip worth making?) or at the counter itself.

use crate::items::ItemStats;
use crate::profile::LootAction;

/// What no counter will take, whatever any rule says: somebody's work,
/// something being worn, and the server's own word.
pub fn never_sell(item: &ItemStats) -> bool {
    item.wielded || item.tinks > 0 || item.inscribed || item.unsellable || item.value == 0
}

/// Why it will not go, for the log.
pub fn never_sell_because(item: &ItemStats) -> &'static str {
    if item.wielded {
        "it is equipped"
    } else if item.tinks > 0 {
        "it has been tinkered"
    } else if item.inscribed {
        "it is inscribed"
    } else if item.unsellable {
        "no vendor will take it"
    } else {
        "it is worth nothing"
    }
}

/// Whether any of `list` appears in `name`, case-insensitively: the
/// player's own word, written as they would write it.
pub fn name_matches(name: &str, list: &[String]) -> bool {
    let name = name.to_lowercase();
    list.iter()
        .any(|w| !w.trim().is_empty() && name.contains(&w.trim().to_lowercase()))
}

/// Foci: equipment that halves a school's components. A counter would
/// not take one anyway; the point is not to walk to town meaning to
/// sell it.
fn is_focus(wcid: u32) -> bool {
    FOCI.contains(&wcid)
}

/// The five foci, by weenie class id (ACE's world database).
const FOCI: [u32; 5] = [15271, 15270, 15268, 15269, 43173];

/// Whether something in the pack goes over the counter.
///
/// The bars come first and nothing gets past them: what is equipped,
/// tinkered, inscribed or worth nothing, ammunition, the focus that
/// halves a school's components, a component this character's own
/// spells burn, and anything the player named by hand.
///
/// Those bars used to live inside ac-client's `sellable`, which the
/// profile path
/// did not call -- so giving a character a loot profile quietly
/// switched off the focus guard, the keep list and the component guard,
/// and a profile with a broad "sell the cheap stuff" rule would take the
/// Peas out of a mage's pack. Peas are bought, not looted.
///
/// Then `tagged`: what was decided when the item was picked up. That
/// decision stands. Deciding again at the counter is how a thing taken
/// to keep gets sold on the next run to town. `judge` is asked only for
/// what carries no decision -- bought, traded, or in the pack from
/// before there was a profile.
pub fn offer_to_vendor(
    stats: &ItemStats,
    ammo: bool,
    burns: &[u32],
    keep: &[String],
    stocked: bool,
    tagged: Option<LootAction>,
    judge: impl FnOnce() -> bool,
) -> bool {
    let forbidden = never_sell(stats)
        || ammo
        // A thing the character goes to town to buy is not a thing the
        // character goes to town to sell. Membership of the profile's
        // buy list says so on its own, which is one rule in place of
        // the four guards that used to try to say it and the profile
        // path that heard none of them.
        || stocked
        || is_focus(stats.wcid)
        || (stats.item_type & ac_world::item_type::SPELL_COMPONENTS != 0 && burns.contains(&stats.wcid))
        || name_matches(&stats.name, keep);
    if forbidden {
        return false;
    }
    match tagged {
        Some(LootAction::Sell) => true,
        Some(_) => false,
        None => judge(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_world::item_type;

    fn item(name: &str, item_type: u32, value: u32) -> ItemStats {
        ItemStats {
            guid: 1,
            name: name.into(),
            item_type,
            value,
            max_stack: 1,
            ..Default::default()
        }
    }

    #[test]
    fn a_profile_cannot_sell_what_the_bars_forbid() {
        // Giving a character a loot profile used to switch off the bars:
        // the profile path never called `sellable`, so the focus guard,
        // the player's keep list and the guard on the components its own
        // spells burn all went with it. A profile whose rules say "sell
        // everything" is the test, because that is the rule a player
        // writes and then wonders where their Peas went.
        const TAPER: u32 = 691;
        let burns = [TAPER];
        let sell_it_all = || true;

        let mut taper = item("Prismatic Taper", item_type::SPELL_COMPONENTS, 5);
        taper.wcid = TAPER;
        assert!(
            !offer_to_vendor(&taper, false, &burns, &[], false, None, sell_it_all),
            "the components its own spells burn are never offered"
        );

        let mut pea = item("Pyreal Pea", item_type::SPELL_COMPONENTS, 50_000);
        pea.wcid = 8330;
        assert!(
            offer_to_vendor(&pea, false, &burns, &[], false, None, sell_it_all),
            "a component it does not burn still pays for the trip"
        );

        let named = item("Ornate Ring", item_type::JEWELRY, 900);
        assert!(
            !offer_to_vendor(
                &named,
                false,
                &burns,
                &["ornate ring".to_string()],
                false,
                None,
                sell_it_all
            ),
            "the player's own word beats the rules"
        );

        let arrow = item("Arrowhead", item_type::MISSILE_WEAPON, 20);
        assert!(
            !offer_to_vendor(&arrow, true, &burns, &[], false, None, sell_it_all),
            "ammunition is not loot"
        );
    }

    #[test]
    fn what_was_picked_up_to_keep_is_never_sold_later() {
        // The decision is made once, when the item is taken. Asking
        // again at the counter is how a thing taken to keep gets sold on
        // the next run to town -- and the profile path did ask again.
        let ring = item("Ornate Ring", item_type::JEWELRY, 900);
        let sell_it_all = || true;

        for kept in [LootAction::Keep, LootAction::Salvage, LootAction::Skip] {
            assert!(
                !offer_to_vendor(&ring, false, &[], &[], false, Some(kept), sell_it_all),
                "taken to {}, so not sold",
                kept.label()
            );
        }
        assert!(
            offer_to_vendor(
                &ring,
                false,
                &[],
                &[],
                false,
                Some(LootAction::Sell),
                || false
            ),
            "taken to sell, so sold, whatever the rules say now"
        );
        assert!(
            offer_to_vendor(&ring, false, &[], &[], false, None, sell_it_all),
            "nothing decided: the rules answer"
        );
    }

    #[test]
    fn nothing_on_the_shopping_list_is_ever_offered() {
        // The one line that replaces four guards. A player who writes
        // "sell anything worth under a thousand" has not said "sell my
        // Peas", and a Blue Pea is 3,125 pyreals to replace.
        let mut pea = item("Blue Pea", item_type::SPELL_COMPONENTS, 3_125);
        pea.wcid = 8346;
        assert!(
            !offer_to_vendor(&pea, false, &[], &[], true, None, || true),
            "on the buy list, so never sold"
        );
        assert!(
            !offer_to_vendor(&pea, false, &[], &[], true, Some(LootAction::Sell), || true),
            "not even when something tagged it to sell"
        );
        assert!(
            offer_to_vendor(&pea, false, &[], &[], false, None, || true),
            "off the list, it is vendor trash like any other"
        );
    }

    #[test]
    fn a_character_with_no_profile_sells_nothing() {
        // The profile is the source of truth, so the right way to be
        // empty-handed is to be empty-handed. The alternative is a
        // client deciding on its own what somebody's things are worth,
        // which is how a mage's Peas got sold.
        let junk = item("Pyreal Pea", item_type::SPELL_COMPONENTS, 3_125);
        assert!(
            !offer_to_vendor(&junk, false, &[], &[], false, None, || false),
            "nothing decided and nobody to ask: it stays in the pack"
        );
    }
}
