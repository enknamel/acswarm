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
//! - one the character can use: the Summoning skill it needs (the
//!   appraisal's, or until that comes the ladder its name stands on -- a
//!   "(50)" essence needs 310, not 50) no more than the Summoning skill
//!   **as buffed** -- the server checks the current value, buffs included;
//! - of the element the target takes most damage from, when that is
//!   known -- an Acid Moar for a creature weak to acid -- and otherwise
//!   any;
//! - of those, the highest level.
//!
//! Past level 50 a character picks a summoning mastery, and an essence
//! of another mastery is refused. The client is not told which mastery
//! the character has. An essence the server turns away outright -- of
//! another mastery, or needing more skill than there is -- is set aside
//! at once; one that summons nothing for no reason given is tried again
//! after its cooldown and set aside for a while after a few failures.
//!
//! A summoned creature arrives named for its owner and carrying the
//! owner's guid (`WorldObject::pet_owner`): the fight rules never pick
//! one, the character's own or anyone else's.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use ac_agent::recent::Recent;
use ac_world::elements::Element;

use crate::autoplay::Doing;
use crate::refusals::SummonRefusal;
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
    used: Recent<u32>,
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

/// The Summoning skill an essence needs, from its name: for until the
/// server's own figure (the appraisal's `UseRequiresSkillLevel`) comes.
///
/// The number in the name is the creature's level, not the skill: every
/// "(50)" essence asks for 310 (two of them 320), and a Mud Golem, with no
/// number at all, for 50. Taking the creature's level for the skill had a
/// character with 200 Summoning reaching for essences that need 310 and
/// being turned away each time. The ladder is the world database's
/// (`UseRequiresSkillLevel` on every summoning essence). An essence named
/// with neither a number on the ladder nor a golem ("Acid Maiden
/// Essence", which needs 570) waits for its appraisal.
pub fn required_skill(name: &str) -> Option<u32> {
    const TIERS: [(u32, u32); 6] = [
        (50, 310),
        (80, 370),
        (100, 400),
        (125, 430),
        (150, 475),
        (180, 530),
    ];
    const GOLEMS: [(&str, u32); 7] = [
        ("Mud", 50),
        ("Sandstone", 220),
        ("Copper", 310),
        ("Oak", 370),
        ("Gold", 400),
        ("Coral", 430),
        ("Iron", 475),
    ];
    if let Some(tier) = tier_in_name(name) {
        return TIERS
            .iter()
            .find(|(t, _)| *t == tier)
            .map(|&(_, skill)| skill);
    }
    let lower = name.to_lowercase();
    if !lower.contains("golem") {
        return None;
    }
    GOLEMS
        .iter()
        .find(|(m, _)| lower.starts_with(&m.to_lowercase()))
        .map(|&(_, skill)| skill)
}

/// The creature's level in an essence's name: "Fire Grievver Essence
/// (50)" is 50.
fn tier_in_name(name: &str) -> Option<u32> {
    let (_, tail) = name.rsplit_once(" (")?;
    tail.strip_suffix(')')?.parse().ok()
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
    /// A line from the server just after an essence was used. One turned
    /// away for good is set aside at once: waiting for it to bring nothing
    /// three times was three cooldowns, over two minutes of fighting with
    /// no creature, before an essence of the wrong mastery gave way to one
    /// the character could use.
    pub(crate) fn hear_summoning(&mut self, why: SummonRefusal, text: &str, now: Instant) {
        let Some((guid, at)) = self.autoplay.summoning.pending else {
            return;
        };
        if now.duration_since(at) > APPEARS_WITHIN {
            return;
        }
        match why {
            SummonRefusal::NotForUs => {
                self.autoplay.summoning.pending = None;
                self.autoplay
                    .summoning
                    .failed
                    .insert(guid, (FAILS_BEFORE_SET_ASIDE, now));
                tracing::info!("summon: essence {guid:#010x} turned away ({text}); set aside");
            }
            SummonRefusal::OneIsOut => {
                self.autoplay.summoning.pending = None;
                let key = self.world.objects.get(&guid).map_or(guid, |o| {
                    if o.cooldown_id != 0 {
                        o.cooldown_id
                    } else {
                        o.guid
                    }
                });
                self.autoplay.summoning.used.forget(&key);
            }
        }
    }

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
                // The server's own figure once appraised, and until then the
                // name's (see `required_skill`). Every essence is appraised:
                // the name's figure is right for all but a few.
                let appraisal = self.appraisals.get(&o.guid);
                if appraisal.is_none() {
                    unknown.push(o.guid);
                }
                let level = appraisal
                    .and_then(|a| {
                        a.int(USE_REQUIRES_SKILL_LEVEL)
                            .or(a.int(ITEM_SKILL_LEVEL_LIMIT))
                    })
                    .map(|l| l.max(0) as u32)
                    .or_else(|| required_skill(&o.name));
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
                let cooled = !state.used.within(&key, now, wait);
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
        self.autoplay.summoning.used.mark(key, now);
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
    pub(crate) fn in_a_fight(&self) -> bool {
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
mod tests;
