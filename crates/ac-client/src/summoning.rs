//! Summoning a creature to fight beside the character.
//!
//! Summoning is a skill, not a school of magic: the character uses an
//! essence carried in the pack, and the creature it names appears in
//! front of them, fights the nearest enemy until it or they are dead,
//! and is gone when its lifespan runs out (about 45 seconds). The
//! essence then waits out its own cooldown (also about 45 seconds, sent
//! with the item) before it will summon again, and holds 50 uses. One
//! summoned creature at a time: the server turns a second away while the
//! first is still out.
//!
//! Which essence:
//!
//! - one the character can use: its required Summoning level (the
//!   appraisal's, or the "(50)" in its name, or the golem it summons) no
//!   more than the Summoning skill **as buffed** -- the server checks the
//!   current value, buffs included;
//! - of the element the target takes most damage from, when that is
//!   known -- an Acid Moar for a creature weak to acid -- and otherwise
//!   any;
//! - of those, the highest level.
//!
//! Past level 50 a character picks a summoning mastery, and an essence
//! of another mastery is refused. The client is not told which mastery
//! the character has, so an essence that summons nothing is tried again
//! after its cooldown and set aside for a while after a few failures,
//! whatever the reason was.
//!
//! A summoned creature arrives named for its owner and carrying the
//! owner's guid (`WorldObject::pet_owner`): the fight rules never pick
//! one, the character's own or anyone else's.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use ac_world::elements::Element;

use crate::autoplay::Doing;
use crate::Client;

/// How long an essence waits between summons when the item does not
/// say (the wiki's figure).
pub const COOLDOWN: Duration = Duration::from_secs(45);
/// A creature that has not appeared this long after the essence was used
/// is not coming.
const APPEARS_WITHIN: Duration = Duration::from_secs(4);
/// This many summons in a row that bring nothing set an essence aside.
const FAILS_BEFORE_SET_ASIDE: u32 = 3;
/// And for this long.
const SET_ASIDE: Duration = Duration::from_secs(600);
/// Appraisal int: the Summoning level an essence needs (ACE
/// `UseRequiresSkillLevel`).
const USE_REQUIRES_SKILL_LEVEL: u32 = 367;
/// Appraisal int: the skill level an item needs (ACE
/// `ItemSkillLevelLimit`), which some essences carry instead.
const ITEM_SKILL_LEVEL_LIMIT: u32 = 115;

/// The summoning rules' state.
#[derive(Debug, Default)]
pub struct State {
    /// When each cooldown was last started, by cooldown id (the item's
    /// own guid when it has none).
    used: HashMap<u32, Instant>,
    /// The essence last used and when, until its creature turns up.
    pending: Option<(u32, Instant)>,
    /// Essences that brought nothing: how many times in a row, and when
    /// last.
    failed: HashMap<u32, (u32, Instant)>,
}

/// An essence as the choice sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Essence {
    pub guid: u32,
    pub element: Option<Element>,
    /// The Summoning level it needs, once known.
    pub level: Option<u32>,
    /// Off cooldown, with uses left, and not set aside.
    pub ready: bool,
}

/// Whether an item called `name` with `max_structure` uses is a summoning
/// essence: "Mud Golem Essence", "Frost K'nath Essence (50)".
pub fn is_essence(name: &str, max_structure: u32) -> bool {
    let bare = name.rsplit_once(" (").map_or(name, |(n, _)| n);
    max_structure > 0 && bare.ends_with(" Essence")
}

/// The level in an essence's name: "Fire Grievver Essence (50)" is 50.
pub fn level_in_name(name: &str) -> Option<u32> {
    let (_, tail) = name.rsplit_once(" (")?;
    tail.strip_suffix(')')?.parse().ok()
}

/// The Summoning level a golem essence needs: golems carry no level in
/// their names, and they come in a fixed ladder of materials.
pub fn golem_level(name: &str) -> Option<u32> {
    const GOLEMS: [(&str, u32); 7] = [
        ("Mud", 15),
        ("Sandstone", 30),
        ("Copper", 50),
        ("Oak", 80),
        ("Gold", 100),
        ("Coral", 125),
        ("Iron", 150),
    ];
    let lower = name.to_lowercase();
    if !lower.contains("golem") {
        return None;
    }
    GOLEMS
        .iter()
        .find(|(m, _)| lower.starts_with(&m.to_lowercase()))
        .map(|&(_, l)| l)
}

/// The element of the damage an essence's creature does, from its name:
/// the element word ("Acid Moar"), or the word the strongest of each
/// kind is named with ("Caustic Grievver", "Volcanic Moar"), or the
/// particular K'nath. Golems hit with bludgeoning.
pub fn element_of(name: &str) -> Option<Element> {
    let lower = name.to_lowercase();
    const WORDS: [(&str, Element); 26] = [
        ("golem", Element::Bludgeon),
        ("acid", Element::Acid),
        ("caustic", Element::Acid),
        ("blister", Element::Acid),
        ("corrosion", Element::Acid),
        ("y'nda", Element::Acid),
        ("frost", Element::Cold),
        ("arctic", Element::Cold),
        ("blizzard", Element::Cold),
        ("freezing", Element::Cold),
        ("frigid", Element::Cold),
        ("glacial", Element::Cold),
        ("r'ajed", Element::Cold),
        ("lightning", Element::Electric),
        ("electrified", Element::Electric),
        ("excited", Element::Electric),
        ("galvanic", Element::Electric),
        ("shocked", Element::Electric),
        ("voltaic", Element::Electric),
        ("t'soct", Element::Electric),
        ("fire", Element::Fire),
        ("charred", Element::Fire),
        ("incendiary", Element::Fire),
        ("scorched", Element::Fire),
        ("volcanic", Element::Fire),
        ("b'orret", Element::Fire),
    ];
    WORDS
        .iter()
        .find(|(w, _)| lower.contains(w))
        .map(|&(_, e)| e)
}

/// The essence to use: ready, needing no more than `skill`, of the
/// element the target takes most from (`takes`, 1.0 for one not known),
/// and of those the highest level.
pub fn choose(essences: &[Essence], skill: u32, takes: impl Fn(Element) -> f32) -> Option<u32> {
    essences
        .iter()
        .filter(|e| e.ready && e.level.is_some_and(|l| l <= skill))
        .map(|e| (e.element.map_or(1.0, &takes), e.level.unwrap_or(0), e.guid))
        .max_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
        .map(|(_, _, g)| g)
}

impl Client {
    /// Summon a creature to fight beside the character, when a fight is
    /// on, none of its creatures is out, and an essence is ready. True
    /// while it does.
    pub(crate) fn autoplay_summon(&mut self, now: Instant) -> bool {
        let cfg = &self.autoplay.config.fight;
        if !cfg.enabled || !cfg.summon {
            return false;
        }
        let Some(me) = self.world.player_guid else {
            return false;
        };
        let out = self
            .world
            .objects
            .values()
            .any(|o| o.pet_owner == me && !o.no_draw && o.health.unwrap_or(1.0) > 0.0);
        // What came of the last one.
        if let Some((guid, at)) = self.autoplay.summoning.pending {
            if out {
                self.autoplay.summoning.pending = None;
                self.autoplay.summoning.failed.remove(&guid);
            } else if now.duration_since(at) > APPEARS_WITHIN {
                self.autoplay.summoning.pending = None;
                let tries = self
                    .autoplay
                    .summoning
                    .failed
                    .entry(guid)
                    .or_insert((0, now));
                tries.0 += 1;
                tries.1 = now;
                tracing::info!(
                    "summon: essence {guid:#010x} brought nothing ({} in a row)",
                    tries.0
                );
            }
        }
        if out || self.autoplay.summoning.pending.is_some() || !self.in_a_fight() {
            return false;
        }
        // A use while a spell is in the air is turned away as too busy.
        if self.autoplay.cast_in_flight(now) {
            return false;
        }
        let Some(skill) = self.summoning_skill() else {
            return false;
        };
        let state = &self.autoplay.summoning;
        let mut unknown = Vec::new();
        let essences: Vec<(Essence, String)> = self
            .world
            .inventory()
            .filter(|o| is_essence(&o.name, o.max_structure))
            .map(|o| {
                let level = self
                    .appraisals
                    .get(&o.guid)
                    .and_then(|a| {
                        a.int(USE_REQUIRES_SKILL_LEVEL)
                            .or(a.int(ITEM_SKILL_LEVEL_LIMIT))
                    })
                    .map(|l| l.max(0) as u32)
                    .or_else(|| level_in_name(&o.name))
                    .or_else(|| golem_level(&o.name));
                if level.is_none() {
                    unknown.push(o.guid);
                }
                let key = if o.cooldown_id != 0 {
                    o.cooldown_id
                } else {
                    o.guid
                };
                let wait = if o.cooldown_duration > 0.0 {
                    Duration::from_secs_f64(o.cooldown_duration)
                } else {
                    COOLDOWN
                };
                let cooled = state
                    .used
                    .get(&key)
                    .is_none_or(|t| now.duration_since(*t) >= wait);
                let set_aside = state.failed.get(&o.guid).is_some_and(|(n, t)| {
                    *n >= FAILS_BEFORE_SET_ASIDE && now.duration_since(*t) < SET_ASIDE
                });
                let uses = o.structure > 0;
                let essence = Essence {
                    guid: o.guid,
                    element: element_of(&o.name),
                    level,
                    ready: cooled && uses && !set_aside,
                };
                (essence, o.name.clone())
            })
            .collect();
        if !unknown.is_empty() {
            self.appraise_many(unknown);
        }
        let target = [self.autoplay.casting_at(), self.attack_target]
            .into_iter()
            .flatten()
            .find_map(|g| self.world.objects.get(&g))
            .and_then(|o| ac_world::elements::known(o.weenie_class_id, &o.name));
        let list: Vec<Essence> = essences.iter().map(|(e, _)| *e).collect();
        let Some(guid) = choose(&list, skill, |e| target.map_or(1.0, |c| c.takes_from(e))) else {
            return false;
        };
        let Some(o) = self.world.objects.get(&guid) else {
            return false;
        };
        let key = if o.cooldown_id != 0 {
            o.cooldown_id
        } else {
            o.guid
        };
        let name = o.name.clone();
        self.interact(guid);
        self.autoplay.summoning.used.insert(key, now);
        self.autoplay.summoning.pending = Some((guid, now));
        self.autoplay
            .say(Doing::Fighting, format!("summoning with {name}"));
        true
    }

    /// The Summoning skill as buffed, when it is trained: what the
    /// server holds an essence's level against.
    fn summoning_skill(&self) -> Option<u32> {
        use ac_world::stats::{sac, skill};
        let s = self.world.stats.skill(skill::SUMMONING)?;
        if s.advancement < sac::TRAINED {
            return None;
        }
        let table = self.assets.skill_table().ok();
        Some(
            self.world
                .stats
                .skill_value(s, table.as_ref().and_then(|t| t.get(skill::SUMMONING))),
        )
    }

    /// Whether a fight is on: something alive being attacked within the
    /// fight radius, or something hitting the character.
    fn in_a_fight(&self) -> bool {
        if self.under_attack() {
            return true;
        }
        let Some(me) = self.player.as_ref().map(|p| p.world_position()) else {
            return false;
        };
        let radius = self.autoplay.config.fight.radius;
        [self.autoplay.casting_at(), self.attack_target]
            .into_iter()
            .flatten()
            .filter_map(|g| self.world.objects.get(&g))
            .any(|o| {
                o.pet_owner == 0
                    && o.health.unwrap_or(1.0) > 0.0
                    && o.world_pos().is_some_and(|at| at.distance(me) <= radius)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn essences_are_known_by_name_and_uses() {
        assert!(is_essence("Mud Golem Essence", 50));
        assert!(is_essence("Frost K'nath Essence (50)", 50));
        assert!(!is_essence("Frost K'nath Essence (50)", 0));
        assert!(!is_essence("Essence of Mana", 50));
        assert!(!is_essence("Encapsulated Spirit", 0));
    }

    #[test]
    fn the_level_is_in_the_name_or_the_golem() {
        assert_eq!(level_in_name("Fire Grievver Essence (50)"), Some(50));
        assert_eq!(level_in_name("Mud Golem Essence"), None);
        assert_eq!(golem_level("Mud Golem Essence"), Some(15));
        assert_eq!(golem_level("Sandstone Golem Essence"), Some(30));
        assert_eq!(golem_level("Fire Grievver Essence (50)"), None);
    }

    #[test]
    fn the_element_is_in_the_name() {
        assert_eq!(element_of("Acid Moar Essence (80)"), Some(Element::Acid));
        assert_eq!(element_of("Frost K'nath Essence (50)"), Some(Element::Cold));
        assert_eq!(
            element_of("Lightning Phyntos Wasp Essence (50)"),
            Some(Element::Electric)
        );
        assert_eq!(
            element_of("Volcanic Moar Essence (200)"),
            Some(Element::Fire)
        );
        assert_eq!(
            element_of("K'nath Y'nda Essence (200)"),
            Some(Element::Acid)
        );
        assert_eq!(element_of("Mud Golem Essence"), Some(Element::Bludgeon));
    }

    fn essence(guid: u32, element: Element, level: u32) -> Essence {
        Essence {
            guid,
            element: Some(element),
            level: Some(level),
            ready: true,
        }
    }

    #[test]
    fn the_targets_weakness_first_then_the_highest_level() {
        let carried = [
            essence(1, Element::Bludgeon, 30),
            essence(2, Element::Acid, 50),
            essence(3, Element::Acid, 80),
            essence(4, Element::Fire, 100),
            essence(5, Element::Acid, 150),
        ];
        let weak_to_acid = |e| if e == Element::Acid { 1.5 } else { 1.0 };
        // Skill 100: the acid 80, not the fire 100 or the acid 150.
        assert_eq!(choose(&carried, 100, weak_to_acid), Some(3));
        // Nothing known about the target: the highest level allowed.
        assert_eq!(choose(&carried, 100, |_| 1.0), Some(4));
        // Too little skill for anything but the golem.
        assert_eq!(choose(&carried, 40, weak_to_acid), Some(1));
        assert_eq!(choose(&carried, 10, weak_to_acid), None);
    }

    #[test]
    fn one_not_ready_or_of_unknown_level_waits() {
        let mut cooling = essence(1, Element::Acid, 80);
        cooling.ready = false;
        let unknown = Essence {
            guid: 2,
            element: Some(Element::Acid),
            level: None,
            ready: true,
        };
        let fallback = essence(3, Element::Bludgeon, 15);
        assert_eq!(choose(&[cooling, unknown, fallback], 200, |_| 1.0), Some(3));
    }
}
