//! The magic family: casting a spell by name or id, and restocking what casts
//! burn.

use super::{nothing_named, resolve, Action, Outcome, Refused, SpellRef, Target};
use crate::autoplay::cast_problem;
use crate::magic::CastCheck;
use crate::Client;

pub(super) fn cast(c: &mut Client, spell: &SpellRef, at: Option<&Target>) -> Outcome {
    let id = resolve::spell(c, spell).ok_or_else(|| match spell {
        SpellRef::Name(n) => Refused::ours(format!("no spell called {n:?}")),
        SpellRef::Id(id) => Refused::ours(format!("no spell {id}")),
    })?;
    let check = match at {
        Some(t) => {
            let guid = resolve::object(c, t).ok_or_else(|| nothing_named(t))?;
            c.cast_at(id, guid)
        }
        None => c.try_cast(id),
    };
    match check {
        CastCheck::Ok => Ok(()),
        other => Err(Refused::ours(cast_problem(&other))),
    }
}

pub(super) fn fill_components(c: &mut Client, kind: Option<u32>, budget: Option<u32>) -> Outcome {
    if c.fill_components(kind, budget) == 0 {
        return Err(Refused::ours("nothing to restock at this counter"));
    }
    Ok(())
}

/// Forget what the character wants to keep of each component.
pub(super) fn clear_components(c: &mut Client) -> Outcome {
    if c.clear_desired_components() == 0 {
        return Err(Refused::ours("no component list to clear"));
    }
    Ok(())
}

/// What `/fillcomps` was asked for: every kind or one of them, up to a bill
/// in pyreals, or the word that forgets the wants. None wants the usage line.
pub(super) fn fill_command(args: &str) -> Option<Action> {
    // Retail read at most two words: a kind, then what to spend on it. It
    // refused a bill of none or less, and took a second word that is no
    // number at all as no bill at all.
    let bill = |word: &str| match word.parse::<i64>() {
        Ok(n) if n > 0 => Some(Some(n.min(u32::MAX.into()) as u32)),
        Ok(_) => None,
        Err(_) => Some(None),
    };
    let words: Vec<&str> = args.split_whitespace().collect();
    Some(match words[..] {
        [] => Action::FillComponents {
            kind: None,
            budget: None,
        },
        [one] if one.eq_ignore_ascii_case("clear") => Action::ClearComponents,
        [one] => match component_kind(one) {
            Some(kind) => Action::FillComponents {
                kind: Some(kind),
                budget: None,
            },
            // One word that is no kind is the bill, and nothing else.
            None => Action::FillComponents {
                kind: None,
                budget: Some(one.parse::<u32>().ok().filter(|&n| n > 0)?),
            },
        },
        [one, two] => Action::FillComponents {
            kind: Some(component_kind(one)?),
            budget: bill(two)?,
        },
        _ => return None,
    })
}

/// A component kind by the word `/fillcomps` takes, singular or plural:
/// retail's own list, onto the component table's own numbering.
pub(super) fn component_kind(word: &str) -> Option<u32> {
    use ac_formats::spell_components::component_type as kind;
    let word = word.trim().to_ascii_lowercase();
    Some(match word.strip_suffix('s').unwrap_or(&word) {
        "scarab" => kind::SCARAB,
        "herb" => kind::HERB,
        "powderedgem" | "powder" => kind::POWDER,
        "alchemicalsubstance" | "potion" => kind::POTION,
        "talisman" => kind::TALISMAN,
        "taper" => kind::TAPER,
        "pea" => kind::PEA,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kind_is_named_the_way_retail_named_it() {
        use ac_formats::spell_components::component_type as kind;
        assert_eq!(component_kind("Scarab"), Some(kind::SCARAB));
        assert_eq!(component_kind("scarabs"), Some(kind::SCARAB));
        assert_eq!(component_kind("PowderedGem"), Some(kind::POWDER));
        assert_eq!(component_kind("powders"), Some(kind::POWDER));
        assert_eq!(component_kind("AlchemicalSubstances"), Some(kind::POTION));
        assert_eq!(component_kind("Peas"), Some(kind::PEA));
        assert_eq!(component_kind("chorizite"), None);
    }
}
