//! Refusals the server sends only in words, matched once in [`refused`] and answered by kind in [`answer`].
//! ACE sends many as a transient string or Broadcast line with no code; any event after carries no reason.
//! Every row quotes its ACE source, so a new refusal is a row here, never English matched elsewhere.
//! The words carry no guid: whether they answer our request is the caller's call (ac-client `refused.rs`),
//! as the in-use words come for any container and characters round one body hear each other's answers.

use crate::did::{Did, Patience};
use std::time::{Duration, Instant};

/// A thing the server said it would not do, out of one line of chat.
/// Names are the server's own, borrowed from the line: a character's with its `+`, an item's as listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal<'a> {
    /// It would not open the container `name`.
    Open { name: &'a str, why: OpenRefusal },
    /// It would not put `item` into the pack named in the request.
    Put { item: &'a str, why: PutRefusal },
    /// Too heavy to carry. The line names nothing, so it answers the take, pickup, split or merge just sent.
    Carry,
    /// It would not move `item` from where it lies without a pickup first:
    /// the request named a container the item's owner does not hold.
    PickUpFirst { item: &'a str },
    /// It would not recruit `name` into the fellowship.
    Recruit { name: &'a str, why: RecruitRefusal },
    /// It would not let the character attack `name`.
    Attack { name: &'a str },
    /// It would not cast `spell` at `target`: not a thing that spell can be cast on.
    Cast { spell: &'a str, target: &'a str },
    /// It would not use `item` from where it is: it has to be wielded, or carried, first.
    Use { item: &'a str, why: UseRefusal },
    /// It would not use `item` on `target`: not a thing it works on.
    UseWith { item: &'a str, target: &'a str },
    /// It would not buy `item` off the character, or would not pay for the sale.
    Sell {
        item: Option<&'a str>,
        why: SellRefusal,
    },
    /// It would not sell to the character: no room for what was asked.
    Buy { why: BuyRefusal },
    /// It would not summon with the essence used.
    Summon { why: SummonRefusal },
}

/// Why a container would not open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenRefusal {
    /// Someone has it open: the server hands a container to one viewer at a time, usually done in a moment.
    InUse,
    /// The killer's for now: a monster's body opens to everyone once the killer closes it or it half rots.
    NotYetOurs,
    /// The killer's for good: a body that made a rare, or a player killer's doing, is never shared.
    NeverOurs,
}

/// Why an item would not go into a pack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PutRefusal {
    /// The pack named is full until something leaves it; the sacks beside it may have room.
    NoRoom,
    /// The pack named is not a pack a thing can be put in: a corpse.
    NotThere,
}

/// Why somebody would not be recruited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecruitRefusal {
    /// In a fellowship already, this one or another.
    AlreadyAMember,
    /// Mid-use or mid-cast on a server that refuses busy recruits, or an invitation still unanswered.
    /// Over in the seconds those take.
    Busy,
    /// Has fellowship requests turned off.
    NotAccepting,
    /// Was asked, and said no.
    Declined,
    /// No room: a code, not words (WeenieError 0x041E `YourFellowshipIsFull`, Entity/Fellowship.cs:104).
    /// It names nobody, so it is about whoever was asked last.
    Full,
}

/// Why an item would not be used from where it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseRefusal {
    /// It works only wielded.
    MustWield,
    /// It works only from the pack, not from the ground.
    MustCarry,
}

/// Why a sale would not go through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SellRefusal {
    /// The counter does not deal in it, or it is marked unsellable or retained.
    Unsellable,
    /// Worth nothing.
    NoValue,
    /// In the trade window with somebody.
    BeingTraded,
    /// A pack with things still in it.
    NotEmpty,
    /// The coin it would fetch is more than the character can carry.
    Encumbered,
    /// The coin it would fetch has no slot to go in.
    NoRoom,
}

/// Why a purchase would not go through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuyRefusal {
    /// Too heavy for what the character can bear.
    Encumbered,
    /// No item slot for it.
    NoRoom,
    /// No pack slot for it: it is a pack.
    NoContainerSlots,
}

/// Why an essence would not summon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SummonRefusal {
    /// Not for this character: another mastery, or more skill than it has.
    NotForUs,
    /// A creature of the character's is still out; the essence is fine and its cooldown did not start.
    OneIsOut,
}

/// What the server refused, if this line of chat says it refused anything; `None` for every other line.
/// Each arm quotes its source under `reference/ext/ACE/Source/ACE.Server/`; ACE's parameters are borrowed
/// out of the line.
pub fn refused(text: &str) -> Option<Refusal<'_>> {
    let text = text.trim();

    // WorldObjects/Container.cs:740
    //   $"The {Name} is already in use by {currentViewer}!"
    // `currentViewer` is "someone else", or the viewer's name with `container_opener_name` on.
    if let Some((name, _who)) = text
        .strip_prefix("The ")
        .and_then(|s| s.strip_suffix('!'))
        .and_then(|s| s.split_once(" is already in use by "))
    {
        return Some(Refusal::Open {
            name,
            why: OpenRefusal::InUse,
        });
    }
    // WorldObjects/Corpse.cs:133
    //   $"You do not yet have the right to loot the {Name}."
    if let Some(name) = text
        .strip_prefix("You do not yet have the right to loot the ")
        .and_then(|s| s.strip_suffix('.'))
    {
        return Some(Refusal::Open {
            name,
            why: OpenRefusal::NotYetOurs,
        });
    }
    // WorldObjects/Corpse.cs:129
    //   $"You may not loot the {Name} because the {Name} has generated a rare item."
    // WorldObjects/Corpse.cs:131
    //   $"You may not loot the {Name} because the death was caused by a player killer."
    if let Some((name, _why)) = text
        .strip_prefix("You may not loot the ")
        .and_then(|s| s.split_once(" because "))
    {
        return Some(Refusal::Open {
            name,
            why: OpenRefusal::NeverOurs,
        });
    }

    // WorldObjects/Player_Inventory.cs:1322
    //   $"Unable to put {item.Name} into container"
    // Sent before an InventoryServerSaveFailed with no reason in it: only these words make it the pack's.
    if let Some(item) = text
        .strip_prefix("Unable to put ")
        .and_then(|s| s.strip_suffix(" into container"))
    {
        return Some(Refusal::Put {
            item,
            why: PutRefusal::NoRoom,
        });
    }
    // WorldObjects/Player_Inventory.cs:855 (and :2299, :2903 for a stack)
    //   $"You cannot put {item.Name} in that."
    if let Some(item) = text
        .strip_prefix("You cannot put ")
        .and_then(|s| s.strip_suffix(" in that."))
    {
        return Some(Refusal::Put {
            item,
            why: PutRefusal::NotThere,
        });
    }
    // WorldObjects/Player_Inventory.cs:841 (a put), :1555 (a wield off
    // the ground), :2285, :2637 (a split), :2881 (a merge)
    //   "You are too encumbered to carry that!"
    if text == "You are too encumbered to carry that!" {
        return Some(Refusal::Carry);
    }
    // WorldObjects/Player_Inventory.cs:1154
    //   $"You must first pick up the {item.Name}"
    if let Some(item) = text.strip_prefix("You must first pick up the ") {
        return Some(Refusal::PickUpFirst { item });
    }

    // Entity/Fellowship.cs:110
    //   $"{newMember.Name} is already a member of a Fellowship."
    if let Some(name) = text.strip_suffix(" is already a member of a Fellowship.") {
        return Some(Refusal::Recruit {
            name,
            why: RecruitRefusal::AlreadyAMember,
        });
    }
    // Entity/Fellowship.cs:116 (the `fellow_busy_no_recruit` rule),
    // :128 (an invitation already up for them, unanswered)
    //   $"{newMember.Name} is busy."
    // Also what a patron who cannot take an oath just now is called (WorldObjects/Player_Allegiance.cs:92);
    // the caller tells the two apart by whom it has invited.
    if let Some(name) = text.strip_suffix(" is busy.") {
        return Some(Refusal::Recruit {
            name,
            why: RecruitRefusal::Busy,
        });
    }
    // WorldObjects/Player_Fellowship.cs:100
    //   $"{newPlayer.Name} is not accepting fellowship requests."
    if let Some(name) = text.strip_suffix(" is not accepting fellowship requests.") {
        return Some(Refusal::Recruit {
            name,
            why: RecruitRefusal::NotAccepting,
        });
    }
    // Entity/Fellowship.cs:146
    //   $"{player.Name} declines your invite"
    if let Some(name) = text.strip_suffix(" declines your invite") {
        return Some(Refusal::Recruit {
            name,
            why: RecruitRefusal::Declined,
        });
    }

    // WorldObjects/Player_Melee.cs:122, WorldObjects/Player_Missile.cs:110
    //   $"You cannot attack {creatureTarget.Name}"
    if let Some(name) = text.strip_prefix("You cannot attack ") {
        return Some(Refusal::Attack { name });
    }
    // WorldObjects/Player_Magic.cs:417
    //   $"{spell.Name} cannot be cast on {target.Name}."
    if let Some((spell, target)) = text
        .strip_suffix('.')
        .and_then(|s| s.split_once(" cannot be cast on "))
    {
        return Some(Refusal::Cast { spell, target });
    }

    // WorldObjects/Player_Use.cs:73
    //   $"You must {action} the {sourceItem.Name} to use it."
    // `action` is "wield" or "contain", by the item's Usable flags.
    if let Some(rest) = text.strip_suffix(" to use it.") {
        if let Some(item) = rest.strip_prefix("You must wield the ") {
            return Some(Refusal::Use {
                item,
                why: UseRefusal::MustWield,
            });
        }
        if let Some(item) = rest.strip_prefix("You must contain the ") {
            return Some(Refusal::Use {
                item,
                why: UseRefusal::MustCarry,
            });
        }
    }
    // WorldObjects/Player_Use.cs:142
    //   $"Cannot use the {sourceItem.Name} with the {target.Name}"
    if let Some((item, target)) = text
        .strip_prefix("Cannot use the ")
        .and_then(|s| s.split_once(" with the "))
    {
        return Some(Refusal::UseWith { item, target });
    }

    // WorldObjects/Player_Commerce.cs:271
    //   $"The {itemName} is unsellable."
    if let Some(item) = text
        .strip_prefix("The ")
        .and_then(|s| s.strip_suffix(" is unsellable."))
    {
        return Some(Refusal::Sell {
            item: Some(item),
            why: SellRefusal::Unsellable,
        });
    }
    // WorldObjects/Player_Commerce.cs:278
    //   $"The {itemName} has no value and cannot be sold."
    if let Some(item) = text
        .strip_prefix("The ")
        .and_then(|s| s.strip_suffix(" has no value and cannot be sold."))
    {
        return Some(Refusal::Sell {
            item: Some(item),
            why: SellRefusal::NoValue,
        });
    }
    // WorldObjects/Player_Commerce.cs:285
    //   $"You cannot sell that! The {itemName} is currently being traded."
    // WorldObjects/Player_Commerce.cs:292
    //   $"You cannot sell that! The {itemName} must be empty."
    if let Some(rest) = text.strip_prefix("You cannot sell that! The ") {
        if let Some(item) = rest.strip_suffix(" is currently being traded.") {
            return Some(Refusal::Sell {
                item: Some(item),
                why: SellRefusal::BeingTraded,
            });
        }
        if let Some(item) = rest.strip_suffix(" must be empty.") {
            return Some(Refusal::Sell {
                item: Some(item),
                why: SellRefusal::NotEmpty,
            });
        }
    }
    // WorldObjects/Player_Commerce.cs:185
    //   "You are too encumbered to sell that!"
    // WorldObjects/Player_Commerce.cs:187
    //   "You do not have enough free pack space to sell that!"
    match text {
        "You are too encumbered to sell that!" => {
            return Some(Refusal::Sell {
                item: None,
                why: SellRefusal::Encumbered,
            })
        }
        "You do not have enough free pack space to sell that!" => {
            return Some(Refusal::Sell {
                item: None,
                why: SellRefusal::NoRoom,
            })
        }
        _ => {}
    }

    // WorldObjects/Vendor.cs:496
    //   "You are too encumbered to buy that!"
    // WorldObjects/Vendor.cs:498
    //   "You do not have enough pack space to buy that!"
    // WorldObjects/Vendor.cs:500
    //   "You do not have enough container slots to buy that!"
    match text {
        "You are too encumbered to buy that!" => {
            return Some(Refusal::Buy {
                why: BuyRefusal::Encumbered,
            })
        }
        "You do not have enough pack space to buy that!" => {
            return Some(Refusal::Buy {
                why: BuyRefusal::NoRoom,
            })
        }
        "You do not have enough container slots to buy that!" => {
            return Some(Refusal::Buy {
                why: BuyRefusal::NoContainerSlots,
            })
        }
        _ => {}
    }

    // WorldObjects/PetDevice.cs:115
    //   $"You must be a {SummoningMastery} to use the {Name}"
    // WeenieErrorWithString 0x04C9 (`YourIsTooLowToUseItemMagic`), as `weenie_errors` renders it:
    //   "Your Summoning is too low to use item magic"
    // The one row in our own English, since that code names the skill and nothing else.
    if (text.starts_with("You must be a ") && text.contains(" to use the "))
        || text == "Your Summoning is too low to use item magic"
    {
        return Some(Refusal::Summon {
            why: SummonRefusal::NotForUs,
        });
    }
    // WorldObjects/PetDevice.cs:130, :140; WorldObjects/Pet.cs:133, :162
    //   $"{player.CurrentActivePet.Name} is already active"
    if text.ends_with(" is already active") {
        return Some(Refusal::Summon {
            why: SummonRefusal::OneIsOut,
        });
    }

    None
}

/// What to do about a refusal, by its kind alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Ask again after this wait, doubling on each further refusal: what it waits on changes slowly.
    Again(Duration),
    /// Ask again after this same wait every time: it passes in the seconds a use or a cast takes, and
    /// a doubling wait would leave out a mate that happened to be casting at each ask.
    Soon(Duration),
    /// Never ask that again: the thing is done with, or dropped from its list, and the log says so once.
    Never,
}

/// Wait on a container someone else has open: long enough not to ask over them, and emptying one takes
/// a few seconds.
pub const OPEN_IN_USE_AGAIN: Duration = Duration::from_secs(3);

/// Wait on a body not ours yet: it is everyone's once anyone closes it, usually seconds after the refusal.
/// ACE `Corpse.Close` (Corpse.cs:209) sets `IsLooted`, which `HasPermission` (:152) reads before the clock.
pub const OPEN_NOT_OURS_AGAIN: Duration = Duration::from_secs(5);

/// Wait before asking a turned-down invitee again: short, as "busy" passes when a use or a cast ends.
/// "Already a member" settles within a round: that mate's own board row, and the rightful leader disbanding.
pub const RECRUIT_HELD_OFF: Duration = Duration::from_secs(10);

/// Wait on a target the server would not let us attack: a vendor, a town guard or a player not at war
/// stays one while we stand there, and a fight given up on is left this long anyway.
pub const ATTACK_AGAIN: Duration = Duration::from_secs(90);

/// The decision for a refusal of this kind, and the whole of the policy: the systems that act on a
/// refusal apply it to their own tables, and none of them chooses a wait of its own.
pub fn answer(refusal: &Refusal) -> Answer {
    match refusal {
        Refusal::Open { why, .. } => match why {
            OpenRefusal::InUse => Answer::Again(OPEN_IN_USE_AGAIN),
            OpenRefusal::NotYetOurs => Answer::Again(OPEN_NOT_OURS_AGAIN),
            // Never, not a wait: it still lies there, and each lapsed wait is another walk back to it.
            OpenRefusal::NeverOurs => Answer::Never,
        },
        // Not into that pack again (full until something leaves, and a corpse is never a pack); another
        // pack with room is the looting's business, not the words'.
        Refusal::Put { .. } => Answer::Never,
        // The item stays where it lies; the loot rules pass it over for something lighter.
        Refusal::Carry => Answer::Never,
        Refusal::PickUpFirst { .. } => Answer::Never,
        Refusal::Recruit { why, .. } => match why {
            RecruitRefusal::Busy => Answer::Soon(RECRUIT_HELD_OFF),
            RecruitRefusal::AlreadyAMember
            | RecruitRefusal::NotAccepting
            | RecruitRefusal::Declined
            | RecruitRefusal::Full => Answer::Again(RECRUIT_HELD_OFF),
        },
        Refusal::Attack { .. } => Answer::Again(ATTACK_AGAIN),
        // A spell that cannot be cast on a thing never can be.
        Refusal::Cast { .. } => Answer::Never,
        // Wielding or picking up is the caller's to do first; asking again from here gets the same answer.
        Refusal::Use { .. } | Refusal::UseWith { .. } => Answer::Never,
        // Nothing at the counter changes what it will not buy or has no room to pay for: the trip goes
        // on without that item, or ends.
        Refusal::Sell { .. } | Refusal::Buy { .. } => Answer::Never,
        Refusal::Summon { why } => match why {
            SummonRefusal::NotForUs => Answer::Never,
            // Ready again the moment the creature out is gone.
            SummonRefusal::OneIsOut => Answer::Soon(Duration::ZERO),
        },
    }
}

impl Answer {
    /// Apply the decision to `key` in `held`, the table of things held off; `why` is the log's note.
    pub fn hold<K: Ord + Clone>(self, held: &mut Patience<K>, key: K, why: &str, now: Instant) {
        match self {
            Answer::Again(first) => held.hold(key, first, now),
            Answer::Soon(first) => {
                held.forget(&key);
                held.hold(key, first, now);
            }
            Answer::Never => held.note(key, &Did::refused(why), now),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_container_that_would_not_open_is_named_and_why() {
        assert_eq!(
            refused("The Corpse of Hellion is already in use by someone else!"),
            Some(Refusal::Open {
                name: "Corpse of Hellion",
                why: OpenRefusal::InUse
            })
        );
        // A server with `container_opener_name` on names the viewer, and a chest is refused the same.
        assert_eq!(
            refused("The Chest is already in use by +Brynna!"),
            Some(Refusal::Open {
                name: "Chest",
                why: OpenRefusal::InUse
            })
        );
        assert_eq!(
            refused("You do not yet have the right to loot the Corpse of Hellion."),
            Some(Refusal::Open {
                name: "Corpse of Hellion",
                why: OpenRefusal::NotYetOurs
            })
        );
        assert_eq!(
            refused(
                "You may not loot the Corpse of Hellion because the Corpse of Hellion has generated a rare item."
            ),
            Some(Refusal::Open {
                name: "Corpse of Hellion",
                why: OpenRefusal::NeverOurs
            })
        );
        assert_eq!(
            refused(
                "You may not loot the Corpse of Brynna because the death was caused by a player killer."
            ),
            Some(Refusal::Open {
                name: "Corpse of Brynna",
                why: OpenRefusal::NeverOurs
            })
        );
        assert_eq!(refused("The Corpse of Hellion is open"), None);
        assert_eq!(refused("You do not have permission to loot"), None);
    }

    #[test]
    fn a_thing_that_would_not_go_into_a_pack_is_named() {
        assert_eq!(
            refused("Unable to put Pyreal into container"),
            Some(Refusal::Put {
                item: "Pyreal",
                why: PutRefusal::NoRoom
            })
        );
        assert_eq!(
            refused("Unable to put Major Mana Stone into container"),
            Some(Refusal::Put {
                item: "Major Mana Stone",
                why: PutRefusal::NoRoom
            })
        );
        assert_eq!(
            refused("You cannot put Dagger in that."),
            Some(Refusal::Put {
                item: "Dagger",
                why: PutRefusal::NotThere
            })
        );
        assert_eq!(
            refused("You are too encumbered to carry that!"),
            Some(Refusal::Carry)
        );
        assert_eq!(
            refused("You must first pick up the Leather Cap"),
            Some(Refusal::PickUpFirst {
                item: "Leather Cap"
            })
        );
        assert_eq!(refused("Unable to put"), None);
    }

    #[test]
    fn a_recruit_that_came_to_nothing_names_the_mate() {
        assert_eq!(
            refused("+Brynlyn is already a member of a Fellowship."),
            Some(Refusal::Recruit {
                name: "+Brynlyn",
                why: RecruitRefusal::AlreadyAMember
            })
        );
        assert_eq!(
            refused("+Brynoth is busy."),
            Some(Refusal::Recruit {
                name: "+Brynoth",
                why: RecruitRefusal::Busy
            })
        );
        assert_eq!(
            refused("Fleetbot One is not accepting fellowship requests."),
            Some(Refusal::Recruit {
                name: "Fleetbot One",
                why: RecruitRefusal::NotAccepting
            })
        );
        assert_eq!(
            refused("+Caius declines your invite"),
            Some(Refusal::Recruit {
                name: "+Caius",
                why: RecruitRefusal::Declined
            })
        );
        // The code's own words for being busy are not a recruit.
        assert_eq!(refused("You're too busy"), None);
        assert_eq!(refused("You are too busy."), None);
        assert_eq!(refused("Brynoth is busy"), None);
    }

    #[test]
    fn a_target_that_would_not_be_fought_is_named() {
        assert_eq!(
            refused("You cannot attack Town Crier"),
            Some(Refusal::Attack { name: "Town Crier" })
        );
        assert_eq!(
            refused("Flame Bolt I cannot be cast on Ulgrim the Unpleasant."),
            Some(Refusal::Cast {
                spell: "Flame Bolt I",
                target: "Ulgrim the Unpleasant"
            })
        );
    }

    #[test]
    fn a_thing_that_would_not_be_used_is_named() {
        assert_eq!(
            refused("You must wield the Training Wand to use it."),
            Some(Refusal::Use {
                item: "Training Wand",
                why: UseRefusal::MustWield
            })
        );
        assert_eq!(
            refused("You must contain the Handy Healing Kit to use it."),
            Some(Refusal::Use {
                item: "Handy Healing Kit",
                why: UseRefusal::MustCarry
            })
        );
        assert_eq!(
            refused("Cannot use the Handy Healing Kit with the Leather Cap"),
            Some(Refusal::UseWith {
                item: "Handy Healing Kit",
                target: "Leather Cap"
            })
        );
    }

    #[test]
    fn a_sale_or_a_purchase_the_counter_would_not_make() {
        assert_eq!(
            refused("The Leather Cap is unsellable."),
            Some(Refusal::Sell {
                item: Some("Leather Cap"),
                why: SellRefusal::Unsellable
            })
        );
        assert_eq!(
            refused("The Pyreal Peas has no value and cannot be sold."),
            Some(Refusal::Sell {
                item: Some("Pyreal Peas"),
                why: SellRefusal::NoValue
            })
        );
        assert_eq!(
            refused("You cannot sell that! The Amber is currently being traded."),
            Some(Refusal::Sell {
                item: Some("Amber"),
                why: SellRefusal::BeingTraded
            })
        );
        assert_eq!(
            refused("You cannot sell that! The Sack must be empty."),
            Some(Refusal::Sell {
                item: Some("Sack"),
                why: SellRefusal::NotEmpty
            })
        );
        assert_eq!(
            refused("You are too encumbered to sell that!"),
            Some(Refusal::Sell {
                item: None,
                why: SellRefusal::Encumbered
            })
        );
        assert_eq!(
            refused("You do not have enough free pack space to sell that!"),
            Some(Refusal::Sell {
                item: None,
                why: SellRefusal::NoRoom
            })
        );
        assert_eq!(
            refused("You are too encumbered to buy that!"),
            Some(Refusal::Buy {
                why: BuyRefusal::Encumbered
            })
        );
        assert_eq!(
            refused("You do not have enough pack space to buy that!"),
            Some(Refusal::Buy {
                why: BuyRefusal::NoRoom
            })
        );
        assert_eq!(
            refused("You do not have enough container slots to buy that!"),
            Some(Refusal::Buy {
                why: BuyRefusal::NoContainerSlots
            })
        );
    }

    #[test]
    fn a_summon_turned_away_says_whether_the_essence_is_any_good() {
        assert_eq!(
            refused("You must be a Primalist to use the Acid Wisp Essence (50)"),
            Some(Refusal::Summon {
                why: SummonRefusal::NotForUs
            })
        );
        assert_eq!(
            refused("Your Summoning is too low to use item magic"),
            Some(Refusal::Summon {
                why: SummonRefusal::NotForUs
            })
        );
        assert_eq!(
            refused("Verity's Mud Golem is already active"),
            Some(Refusal::Summon {
                why: SummonRefusal::OneIsOut
            })
        );
    }

    #[test]
    fn what_is_not_a_refusal_is_not_read_as_one() {
        for line in [
            "",
            "You give +Caius Pyreal.",
            "+Caius gives you Leather Cap.",
            "Your fellow +Delia has died!",
            "+Bryn is now level 10!",
            "Cast efficiency: 98%",
            "You evade Drudge Skulker's attack.",
            "You are out of ammunition!",
            "Teleporting...",
        ] {
            assert_eq!(refused(line), None, "{line:?}");
        }
    }

    #[test]
    fn the_answer_is_by_kind_and_maps_onto_a_patience() {
        let now = Instant::now();
        let later = now + Duration::from_secs(1);
        let key = 0x8000_0001u32;

        // Busy passes in seconds: the same short wait every time, never doubling.
        let busy = Refusal::Recruit {
            name: "+Caius",
            why: RecruitRefusal::Busy,
        };
        assert_eq!(answer(&busy), Answer::Soon(RECRUIT_HELD_OFF));
        let mut held = Patience::new();
        answer(&busy).hold(&mut held, key, "busy", now);
        answer(&busy).hold(&mut held, key, "busy", later);
        assert_eq!(held.waited(&key), Some(RECRUIT_HELD_OFF));
        assert!(held.held(&key, later));
        assert!(!held.held(&key, later + RECRUIT_HELD_OFF));

        let member = Refusal::Recruit {
            name: "+Caius",
            why: RecruitRefusal::AlreadyAMember,
        };
        assert_eq!(answer(&member), Answer::Again(RECRUIT_HELD_OFF));
        let mut held = Patience::new();
        answer(&member).hold(&mut held, key, "a member", now);
        answer(&member).hold(&mut held, key, "a member", later);
        assert_eq!(held.waited(&key), Some(RECRUIT_HELD_OFF * 2));

        let in_use = Refusal::Open {
            name: "Corpse of Hellion",
            why: OpenRefusal::InUse,
        };
        assert_eq!(answer(&in_use), Answer::Again(OPEN_IN_USE_AGAIN));
        let rare = Refusal::Open {
            name: "Corpse of Hellion",
            why: OpenRefusal::NeverOurs,
        };
        assert_eq!(answer(&rare), Answer::Never);
        let mut held = Patience::new();
        answer(&rare).hold(&mut held, key, "the killer's alone", now);
        assert!(held.held(&key, now + Duration::from_secs(3600)));

        assert_eq!(
            answer(&Refusal::Put {
                item: "Dagger",
                why: PutRefusal::NoRoom
            }),
            Answer::Never
        );
        assert_eq!(answer(&Refusal::Carry), Answer::Never);
        assert_eq!(
            answer(&Refusal::Attack { name: "Town Crier" }),
            Answer::Again(ATTACK_AGAIN)
        );
    }
}
