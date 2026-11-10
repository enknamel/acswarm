//! What each summoning essence's creature does in a fight, for summoning the one that hurts a
//! target most: its element, the damage of one attack and how often it attacks, worked out
//! from server data and the client's motion and combat tables (`data/summons.csv`, see
//! `reference/scripts/data/summons_rank.py`). The essence's own ratings are rolled per item, so
//! they come from its appraisal ([`Ratings`]).

use std::sync::OnceLock;

use crate::elements::Element;

/// A summoning essence and its creature.
#[derive(Clone, Debug, PartialEq)]
pub struct Summon {
    /// The essence's weenie class.
    pub essence: u32,
    pub name: String,
    /// The creature's weenie class.
    pub pet: u32,
    /// The creature's level.
    pub level: u32,
    /// The Summoning skill the essence needs (UseRequiresSkillLevel).
    pub skill: u32,
    /// SummoningMastery: 1 Primalist, 2 Necromancer, 3 Naturalist, 0 any.
    pub mastery: u32,
    /// The character level it needs (UseRequiresLevel), 0 for none.
    pub needs_level: u32,
    /// Whether Summoning must be specialized (UseRequiresSkillSpec).
    pub specialized: bool,
    /// What it hits with; `None` for a creature that never strikes.
    pub element: Option<Element>,
    /// Damage of one attack on average, before ratings and the target's armour and resistances.
    pub hit: f32,
    /// One attack at its strikes' maximum, what a critical doubles (DamageEvent.cs:276-287).
    pub top: f32,
    /// Mean seconds from one attack to the next; 0 for a creature that never strikes.
    pub secs: f32,
}

/// An essence's ratings, rolled when it dropped and copied onto its creature (CombatPet.cs:46-52).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ratings {
    /// GearDamage (PropertyInt 370).
    pub damage: u32,
    /// GearCrit (PropertyInt 372): a point is a percent of critical chance.
    pub crit: u32,
    /// GearCritDamage (PropertyInt 374).
    pub crit_damage: u32,
}

/// Appraisal ints holding [`Ratings`].
pub mod rating {
    pub const DAMAGE: u32 = 370;
    pub const CRIT: u32 = 372;
    pub const CRIT_DAMAGE: u32 = 374;
}

/// A monster's critical chance with no weapon (WorldObject_Weapon.cs:291).
const CRIT_CHANCE: f32 = 0.1;

impl Summon {
    /// Damage a second to a target taking `takes` of this creature's element, with the
    /// essence's `ratings`: ratings add up as percents and a critical doubles the attack at its
    /// maximum (DamageEvent.cs:232-287, WorldObject_Weapon.cs:296-372).
    pub fn damage_per_second(&self, ratings: Ratings, takes: f32) -> f32 {
        if self.secs <= 0.0 {
            return 0.0;
        }
        let chance = (CRIT_CHANCE + ratings.crit as f32 / 100.0).min(1.0);
        let normal = self.hit * (1.0 + ratings.damage as f32 / 100.0);
        let crit_rating = (ratings.damage + ratings.crit_damage) as f32 / 100.0;
        let crit = self.top * 2.0 * (1.0 + crit_rating);
        ((1.0 - chance) * normal + chance * crit) / self.secs * takes
    }
}

const DATA: &str = include_str!("../data/summons.csv");

fn parse(text: &str) -> Vec<Summon> {
    let mut out = Vec::with_capacity(260);
    for line in text.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(',').map(str::trim).collect();
        if f.len() != 12 {
            continue;
        }
        let int = |s: &str| s.parse::<u32>().ok();
        let num = |s: &str| s.parse::<f32>().ok();
        let (Some(essence), Some(pet), Some(level), Some(skill), Some(mastery)) =
            (int(f[0]), int(f[2]), int(f[3]), int(f[4]), int(f[5]))
        else {
            continue;
        };
        let (Some(needs_level), Some(specialized), Some(bit)) = (int(f[6]), int(f[7]), int(f[8]))
        else {
            continue;
        };
        let (Some(hit), Some(top), Some(secs)) = (num(f[9]), num(f[10]), num(f[11])) else {
            continue;
        };
        out.push(Summon {
            essence,
            name: f[1].replace(';', ","),
            pet,
            level,
            skill,
            mastery,
            needs_level,
            specialized: specialized != 0,
            element: Element::from_bit(bit),
            hit,
            top,
            secs,
        });
    }
    out.sort_by_key(|s| s.essence);
    out
}

/// Every essence whose creature fights, parsed on first use.
pub fn all() -> &'static [Summon] {
    static SUMMONS: OnceLock<Vec<Summon>> = OnceLock::new();
    SUMMONS.get_or_init(|| parse(DATA))
}

/// The essence with this weenie class, if it is one.
pub fn of(essence: u32) -> Option<&'static Summon> {
    let all = all();
    all.binary_search_by_key(&essence, |s| s.essence)
        .ok()
        .map(|i| &all[i])
}

#[cfg(test)]
mod tests {
    use super::*;

    const FROST_GRIEVVER_50: u32 = 49387;
    const FROST_WASP_50: u32 = 49538;
    const FROST_SWARM: u32 = 49544;
    const ACID_KNATH_80: u32 = 49283;

    #[test]
    fn every_essence_has_a_creature_and_the_skill_it_needs() {
        let all = all();
        assert!(all.len() > 250, "{} essences", all.len());
        for s in all {
            assert!(s.skill >= 50, "{}: skill {}", s.name, s.skill);
            assert!(
                s.element.is_some() && s.secs > 0.0,
                "{} never strikes",
                s.name
            );
            assert!(
                s.top >= s.hit && s.hit > 0.0,
                "{}: {} {}",
                s.name,
                s.hit,
                s.top
            );
        }
        let grievver = of(FROST_GRIEVVER_50).unwrap();
        assert_eq!(grievver.name, "Frost Grievver Essence (50)");
        assert_eq!((grievver.skill, grievver.mastery), (310, 3));
        assert_eq!((grievver.needs_level, grievver.specialized), (40, false));
        let swarm = of(FROST_SWARM).unwrap();
        assert_eq!(
            (swarm.skill, swarm.needs_level, swarm.specialized),
            (570, 185, true)
        );
        assert_eq!(grievver.element, Some(Element::Cold));
        assert_eq!(of(1), None);
    }

    #[test]
    fn a_rolled_attack_the_motion_table_lacks_is_played_as_its_first() {
        // Wasps lack the AttackHigh2, Med2 and Low3 the generic table rolls; ACE plays the High1,
        // Med1 and Low1 (Monster_Melee.cs:282-308): a 0.5 s sting, then 0.1 + 0.5 s of waiting.
        let wasp = of(FROST_WASP_50).unwrap();
        assert!((wasp.secs - 1.1).abs() < 0.01, "{} s", wasp.secs);
    }

    #[test]
    fn a_phyntos_wasp_stings_lightly_and_often() {
        let none = Ratings::default();
        for wasp in all().iter().filter(|s| s.name.contains("Phyntos Wasp")) {
            let rate = wasp.damage_per_second(none, 1.0);
            let peers = all().iter().filter(|s| s.level == wasp.level);
            for s in peers.filter(|s| !s.name.contains("Phyntos")) {
                assert!(s.secs > wasp.secs, "{} attacks as often", s.name);
                let more = s.damage_per_second(none, 1.0) > rate;
                let kind = ["K'nath", "Wisp", "Grievver"]
                    .iter()
                    .any(|k| s.name.contains(k));
                if kind || s.name.contains("Golem") {
                    assert_eq!(more, kind, "{} against {}", s.name, wasp.name);
                }
            }
        }
        let swarm = of(FROST_SWARM).unwrap().damage_per_second(none, 1.0);
        let wasp = of(FROST_WASP_50).unwrap().damage_per_second(none, 1.0);
        assert!(swarm > wasp * 10.0, "the swarm strikes three times a sting");
    }

    #[test]
    fn ratings_and_criticals_raise_the_rate() {
        let s = of(ACID_KNATH_80).unwrap();
        let bare = s.damage_per_second(Ratings::default(), 1.0);
        let rated = Ratings {
            damage: 10,
            ..Ratings::default()
        };
        let critical = Ratings {
            crit: 10,
            crit_damage: 10,
            ..Ratings::default()
        };
        assert!(s.damage_per_second(rated, 1.0) > bare * 1.09);
        assert!(s.damage_per_second(critical, 1.0) > bare);
        assert_eq!(s.damage_per_second(Ratings::default(), 0.0), 0.0);
        let expected = (0.9 * s.hit + 0.1 * 2.0 * s.top) / s.secs;
        assert!((bare - expected).abs() < 0.01, "{bare} vs {expected}");
    }
}
