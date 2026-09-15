//! What may go over a counter, and what may not.
//!
//! One place, because it was four and the path a loot profile took
//! went through none of them. And one order ([`fate`]): the profile's
//! word first, then the server's, and only for a thing neither has
//! decided the guesses about what the character uses.
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

/// The same, for something the character is actually carrying: a pack
/// with anything in it is refused by the server, and would take its
/// contents with it if it were not.
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

/// Foci: equipment that halves a school's components. A counter would
/// not take one anyway; the point is not to walk to town meaning to
/// sell it.
fn is_focus(wcid: u32) -> bool {
    FOCI.contains(&wcid)
}

/// The five foci, by weenie class id (ACE's world database).
const FOCI: [u32; 5] = [15271, 15270, 15268, 15269, 43173];

/// What a carried thing is for, once the profile has spoken and the
/// server has had its word.
///
/// This is the one place the two are put in order, and the order is
/// the point. The profile is the player's decision about every item
/// the character picks up, written down when it was taken
/// ([`crate::ledger`]), and nothing downstream may overrule it: not a
/// restock list, not the spell-component table, not a buy list, not a
/// name on a keep list. Every one of those is a guess about what the
/// character uses, and every one of them was at some time wrapped
/// around the profile's answer and quietly vetoed it, which is how a
/// mage that had said "sell the peas" carried its peas for good.
///
/// The guesses are still worth making, for a thing the profile said
/// nothing about. The server's own refusals stand ahead of everything,
/// because a sale the server will not make is not a decision anybody
/// gets to take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fate {
    /// Picked up to sell, and nothing stops it going: it is on its way
    /// out of the pack. Not stock, not a need, not a want, and not
    /// anybody's to guard.
    Leaving,
    /// Not going over a counter: picked up to keep or to salvage, or
    /// picked up to sell but the server will not take it.
    Staying,
    /// The profile said nothing about it. The guards and the rules
    /// answer.
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

/// Whether something in the pack goes over the counter.
///
/// The profile decides. What was written down when the item was picked
/// up ([`fate`]) is read first and is final: taken to sell, it goes,
/// unless the server itself will not take it; taken to keep or to
/// salvage, it stays, whatever any rule would make of it today.
/// Deciding again at the counter is how a thing taken to keep gets
/// sold on the next run to town.
///
/// The guards only fill in for an item the profile did not decide --
/// bought, traded, or in the pack from before there was a profile.
/// For those, nothing gets past them: ammunition, the focus that
/// halves a school's components, a component this character's own
/// spells burn, whatever is on the buy list, and anything the player
/// named by hand. Then `judge`, the rules as they read now.
///
/// The guards used to come first, and vetoed the tag. They were
/// written in place of a profile path that ran through none of them,
/// and once the profile was the path they were wrapped around it
/// anyway, so a buy list, a name in `keep`, or a spell that burns the
/// thing each overruled the player's own "sell this". Ammunition sits
/// with the guesses rather than with the server's refusals on purpose:
/// `ammo` is anything that goes in the ammunition slot, a mage's
/// looted arrows included, and it says what the character might use,
/// not what the player decided.
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
        // Somebody's work, something being worn, and the server's own
        // word. No rule and no profile gets past these.
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

        // A pack is only refused while it has something in it: the
        // server will not take a full one, and it would carry its
        // contents off with it if it did.
        let sack = item("Sack", item_type::CONTAINER, 5);
        assert!(never_sell_carried(&sack, true), "a full sack stays");
        assert!(!never_sell_carried(&sack, false), "an empty one may go");
    }

    #[test]
    fn the_rules_read_now_cannot_sell_what_the_guards_forbid() {
        // An item nothing was written down about is judged by the rules
        // as they read today, and those do not get past the guards.
        // Giving a character a loot profile used to switch the guards
        // off entirely: the profile path never called `sellable`, so
        // the focus guard, the player's keep list and the guard on the
        // components its own spells burn all went with it. A profile
        // whose rules say "sell everything" is the test, because that
        // is the rule a player writes and then wonders where their
        // Peas went. (What the ledger has written down is another
        // matter: see `the_profiles_word_beats_every_guard_but_the_servers`.)
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

        // A focus is equipment: it lives in a pack slot and halves the
        // components of its school.
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
    fn the_shopping_list_decides_only_what_the_profile_did_not() {
        // A player who writes "sell anything worth under a thousand"
        // has not said "sell my Peas", and a Blue Pea is 3,125 pyreals
        // to replace: on the buy list and undecided, it stays. But a
        // player who writes "sell peas" has said exactly that, and the
        // list is not allowed to answer back. It did, and a mage that
        // had said "sell the peas" carried them for good.
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
        // Each guard is a guess about what the character uses: a
        // component its spells burn, a name on the keep list, the focus,
        // the ammunition slot. A guess is worth making about a thing
        // the player has not decided; it may not contradict a thing
        // they have. Only the server's own refusals stand ahead of the
        // tag, because a sale the server will not make is not a
        // decision anybody gets to take.
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
        // The one ordering every site reads, so the counter, the
        // restock list and the component guard cannot drift apart
        // again.
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
