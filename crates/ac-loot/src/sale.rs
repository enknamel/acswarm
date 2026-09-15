//! What may go over a counter: [`offer_to_vendor`], in the order [`fate`] sets.
//!
//! The server's refusals stand ahead of the profile's tag, and the tag ahead of every guess about
//! what the character uses. No server, socket or pack: the answer is the same before setting off
//! (is this trip worth making?) and at the counter itself.

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

/// [`never_sell`] for a carried thing, adding a pack that holds anything:
/// the server refuses one, and selling it would take its contents too.
pub fn never_sell_carried(item: &ItemStats, holds_anything: bool) -> bool {
    never_sell(item) || (item.item_type & ac_world::item_type::CONTAINER != 0 && holds_anything)
}

/// Whether any of `list` appears in `name`, case-insensitively: the
/// player's own word, written as they would write it.
pub fn name_matches(name: &str, list: &[String]) -> bool {
    let name = name.to_lowercase();
    list.iter()
        .any(|w| !w.trim().is_empty() && name.contains(&w.trim().to_lowercase()))
}

/// A focus: equipment that halves a school's components.
/// No counter takes one anyway; guarded so no trip to town is planned around selling it.
fn is_focus(wcid: u32) -> bool {
    FOCI.contains(&wcid)
}

/// The five foci, by weenie class id (ACE's world database).
const FOCI: [u32; 5] = [15271, 15270, 15268, 15269, 43173];

/// Where a carried thing stands: the server's refusal beats the profile's tag ([`crate::ledger`]),
/// and the tag beats every guess (restock list, component table, buy list, keep list).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fate {
    /// Tagged Sell and the server will take it: no guard may hold it back.
    Leaving,
    /// Tagged anything but Sell, or tagged Sell but the server will not take it.
    Staying,
    /// No tag: the guards, then the rules, answer.
    Undecided,
}

/// Where a carried thing stands, from the ledger's word (`tagged`) and
/// the server's (`refused`, see [`never_sell`]).
pub fn fate(tagged: Option<LootAction>, refused: bool) -> Fate {
    match tagged {
        Some(LootAction::Sell) if !refused => Fate::Leaving,
        Some(_) => Fate::Staying,
        None => Fate::Undecided,
    }
}

/// Whether a carried thing goes over the counter: the tag read through [`fate`] is final.
/// The guards (`stocked`: on the buy list) and then `judge` answer only untagged things.
/// `ammo` means "fits the ammunition slot", a mage's looted arrows too: a guess, not a refusal.
pub fn offer_to_vendor(
    stats: &ItemStats,
    ammo: bool,
    burns: &[u32],
    keep: &[String],
    stocked: bool,
    tagged: Option<LootAction>,
    judge: impl FnOnce() -> bool,
) -> bool {
    match fate(tagged, never_sell(stats)) {
        Fate::Leaving => true,
        Fate::Staying => false,
        Fate::Undecided => {
            let guarded = never_sell(stats)
                || ammo
                || stocked
                || is_focus(stats.wcid)
                || (stats.item_type & ac_world::item_type::SPELL_COMPONENTS != 0
                    && burns.contains(&stats.wcid))
                || name_matches(&stats.name, keep);
            !guarded && judge()
        }
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
    fn what_is_never_sold_is_never_sold() {
        // Somebody's work, something worn, the server's word: no rule or profile gets past these.
        let plain = item("Dagger", item_type::MELEE_WEAPON, 900);
        assert!(!never_sell(&plain));

        for (what, mut it) in [
            ("tinkered", plain.clone()),
            ("inscribed", plain.clone()),
            ("equipped", plain.clone()),
            ("unsellable", plain.clone()),
            ("worthless", plain.clone()),
        ] {
            match what {
                "tinkered" => it.tinks = 1,
                "inscribed" => it.inscribed = true,
                "equipped" => it.wielded = true,
                "unsellable" => it.unsellable = true,
                _ => it.value = 0,
            }
            assert!(never_sell(&it), "a {what} item");
            assert!(!never_sell_because(&it).is_empty());
        }

        // The server refuses a pack only while it holds something, which a sale would carry off.
        let sack = item("Sack", item_type::CONTAINER, 5);
        assert!(never_sell_carried(&sack, true), "a full sack stays");
        assert!(!never_sell_carried(&sack, false), "an empty one may go");
    }

    #[test]
    fn the_rules_read_now_cannot_sell_what_the_guards_forbid() {
        // Untagged, even a "sell everything" profile cannot pass a guard. A tag is another
        // matter: see `the_profiles_word_beats_every_guard_but_the_servers`.
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

        // A focus is equipment: it lives in a pack slot and halves its school's components.
        let mut focus = item("Foci of Strife", item_type::MISC, 0);
        focus.wcid = 15271;
        focus.value = 5_000;
        assert!(
            !offer_to_vendor(&focus, false, &burns, &[], false, None, sell_it_all),
            "a focus is equipment, not stock"
        );

        let arrow = item("Arrowhead", item_type::MISSILE_WEAPON, 20);
        assert!(
            !offer_to_vendor(&arrow, true, &burns, &[], false, None, sell_it_all),
            "ammunition is not loot"
        );
    }

    #[test]
    fn what_was_picked_up_to_keep_is_never_sold_later() {
        // Decided once, when taken: judging again at the counter sells a keeper on the next trip.
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
    fn the_shopping_list_decides_only_what_the_profile_did_not() {
        // "Sell anything under a thousand" is not "sell my Peas" (a Blue Pea is 3,125 pyreals to
        // replace): untagged on the buy list, it stays; tagged Sell, the list may not answer back.
        let mut pea = item("Blue Pea", item_type::SPELL_COMPONENTS, 3_125);
        pea.wcid = 8346;
        assert!(
            !offer_to_vendor(&pea, false, &[], &[], true, None, || true),
            "on the buy list and undecided, so not sold"
        );
        assert!(
            offer_to_vendor(&pea, false, &[], &[], true, Some(LootAction::Sell), || true),
            "the profile said sell: the buy list does not overrule it"
        );
        assert!(
            offer_to_vendor(&pea, false, &[], &[], false, None, || true),
            "off the list, it is vendor trash like any other"
        );
    }

    #[test]
    fn the_profiles_word_beats_every_guard_but_the_servers() {
        // Each guard guesses what the character uses and may not contradict the player's tag. Only a
        // server refusal stands ahead of it: a sale the server will not make is nobody's decision.
        const LEAD_SCARAB: u32 = 691;
        let burns = [LEAD_SCARAB];
        let never = || false;

        let mut scarab = item("Lead Scarab", item_type::SPELL_COMPONENTS, 5);
        scarab.wcid = LEAD_SCARAB;
        assert!(
            !offer_to_vendor(&scarab, false, &burns, &[], false, None, || true),
            "undecided and burnt by its own spells, it stays"
        );
        assert!(
            offer_to_vendor(
                &scarab,
                false,
                &burns,
                &[],
                false,
                Some(LootAction::Sell),
                never
            ),
            "the player said sell: the spells that burn it do not overrule that"
        );

        let named = item("Ornate Ring", item_type::JEWELRY, 900);
        let keep = ["ornate ring".to_string()];
        assert!(
            offer_to_vendor(
                &named,
                false,
                &[],
                &keep,
                false,
                Some(LootAction::Sell),
                never
            ),
            "a name on the keep list is not the player's word on this ring"
        );

        let mut focus = item("Foci of Strife", item_type::MISC, 5_000);
        focus.wcid = 15271;
        assert!(
            offer_to_vendor(
                &focus,
                false,
                &[],
                &[],
                false,
                Some(LootAction::Sell),
                never
            ),
            "even the focus goes when the profile said so"
        );

        let arrow = item("Arrow", item_type::MISSILE_WEAPON, 20);
        assert!(
            offer_to_vendor(&arrow, true, &[], &[], false, Some(LootAction::Sell), never),
            "ammunition is a guess about what the character shoots, not the player's word"
        );

        // The server's refusals are the one thing ahead of the tag.
        let mut worn = item("Dagger", item_type::MELEE_WEAPON, 900);
        worn.wielded = true;
        assert!(
            !offer_to_vendor(
                &worn,
                false,
                &[],
                &[],
                false,
                Some(LootAction::Sell),
                || true
            ),
            "wielded: the server will not take it, whatever the profile said"
        );
        let mut tinkered = item("Dagger", item_type::MELEE_WEAPON, 900);
        tinkered.tinks = 2;
        assert!(
            !offer_to_vendor(
                &tinkered,
                false,
                &[],
                &[],
                false,
                Some(LootAction::Sell),
                || true
            ),
            "tinkered: somebody's work"
        );
    }

    #[test]
    fn a_things_fate_is_the_profiles_word_and_then_the_servers() {
        // The one ordering every site reads, so counter, restock list and component guard agree.
        assert_eq!(fate(Some(LootAction::Sell), false), Fate::Leaving);
        assert_eq!(
            fate(Some(LootAction::Sell), true),
            Fate::Staying,
            "sell, but the server will not take it"
        );
        for kept in [LootAction::Keep, LootAction::Salvage, LootAction::Skip] {
            assert_eq!(fate(Some(kept), false), Fate::Staying, "{}", kept.label());
        }
        assert_eq!(fate(None, false), Fate::Undecided);
        assert_eq!(
            fate(None, true),
            Fate::Undecided,
            "nothing decided: the guards answer, and never_sell is the first of them"
        );
    }

    #[test]
    fn a_character_with_no_profile_sells_nothing() {
        // No profile sells nothing: the client never decides alone what somebody's things are worth.
        let junk = item("Pyreal Pea", item_type::SPELL_COMPONENTS, 3_125);
        assert!(
            !offer_to_vendor(&junk, false, &[], &[], false, None, || false),
            "nothing decided and nobody to ask: it stays in the pack"
        );
    }
}
