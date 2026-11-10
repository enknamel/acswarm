//! Spell scrolls: the spells this character could learn from one now, worked out again as its
//! spellbook and magic skills change; a scroll the loot rules kept is read between fights
//! (`Looting::learn_scrolls`), and one with nothing left to teach goes to the counter.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::Client;

/// Below this power or from this one on, ACE lets anyone learn the spell (Player_Skills.cs:462).
const ANYONE_BELOW: u32 = 50;
const ANYONE_FROM: u32 = 300;
/// Skill may fall this short of the spell's power (ACE `magicSkillCheckMargin`, Player_Skills.cs:456).
const SKILL_MARGIN: u32 = 50;
/// A scroll read is not read again for this long: the server said no, or it is still reading (guess).
const SCROLL_RETRY: Duration = Duration::from_secs(5 * 60);
/// Reading keeps the character busy this long, stance and motion (Scroll.cs:60-82, guess).
const READ_TAKES: Duration = Duration::from_secs(5);
/// The tag reason a scroll taken to learn goes to the counter under, once it has nothing to teach.
pub(crate) const NOTHING_TO_TEACH: &str = "a scroll with nothing left to teach";

/// The spells to learn, and what they were worked out from.
#[derive(Clone, Debug, Default)]
pub(crate) struct Learnable {
    key: u64,
    pub(crate) spells: Arc<BTreeSet<u32>>,
    /// Scrolls read lately, by guid.
    pub(crate) tried: HashMap<u32, Instant>,
    /// The last read sent.
    pub(crate) read_at: Option<Instant>,
}

impl Learnable {
    /// A read sent less than [`READ_TAKES`] ago: the server is busy with it, and turns a cast away.
    pub(crate) fn is_reading(&self, now: Instant) -> bool {
        self.read_at
            .is_some_and(|t| now.duration_since(t) < READ_TAKES)
    }
}

/// Whether a spell of `power` can be learned with `skill` in its school, `trained` or better:
/// ACE `CanReadScroll`, less a school the character cannot cast in at all.
pub(crate) fn can_learn(power: u32, trained: bool, skill: u32) -> bool {
    trained && (!(ANYONE_BELOW..ANYONE_FROM).contains(&power) || skill + SKILL_MARGIN >= power)
}

impl Client {
    /// The spells this character does not know and could learn from a scroll now.
    pub(crate) fn learnable_spells(&self) -> Arc<BTreeSet<u32>> {
        self.autoplay.learnable.spells.clone()
    }

    /// Work [`Self::learnable_spells`] out again when the spellbook or a school's skill changed;
    /// true when they did.
    pub(crate) fn refresh_learnable(&mut self) -> bool {
        use std::hash::{Hash, Hasher};
        let stats = &self.world.stats;
        let skills = self.assets.skill_table().ok();
        // School skill id, trained, current: what `can_learn` reads.
        let schools: Vec<(u32, bool, u32)> = [31u32, 32, 33, 34, 43]
            .into_iter()
            .map(|id| match stats.skill(id) {
                Some(s) => (
                    id,
                    s.advancement >= ac_world::stats::sac::TRAINED,
                    stats.skill_current(s, skills.as_ref().and_then(|t| t.get(id))),
                ),
                None => (id, false, 0),
            })
            .collect();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (&stats.spells, &schools).hash(&mut h);
        let key = h.finish();
        let was = self.autoplay.learnable.key;
        if key == was && !self.autoplay.learnable.spells.is_empty() {
            return false;
        }
        let Ok(table) = self.assets.spell_table() else {
            return false;
        };
        let known: BTreeSet<u32> = stats.spells.iter().copied().collect();
        let spells: BTreeSet<u32> = table
            .spells
            .iter()
            .filter(|(id, _)| !known.contains(id))
            .filter(|(_, sp)| {
                let school = Self::school_skill(sp.school);
                let Some(&(_, trained, skill)) = schools.iter().find(|s| Some(s.0) == school)
                else {
                    return false;
                };
                can_learn(sp.power, trained, skill)
            })
            .map(|(id, _)| *id)
            .collect();
        self.autoplay.learnable.key = key;
        self.autoplay.learnable.spells = Arc::new(spells);
        key != was
    }

    /// Housekeeping: keep the learnable spells current, and when they change judge again each
    /// scroll taken to learn: one now known, or a second copy, has nothing left to teach.
    pub(crate) fn autoplay_learn_scrolls(&mut self, _now: Instant) {
        if self.refresh_learnable() {
            self.judge_scrolls_again();
        }
    }

    /// A carried scroll tagged [`LEARNABLE`] whose spell no longer is (or whose spell an earlier
    /// copy teaches) takes what the rules say of it, else goes to the counter: it was taken only
    /// to be read, and kept it is clutter.
    fn judge_scrolls_again(&mut self) {
        use crate::autoplay::loot::judge::{judge_loot, LEARNABLE};
        use crate::autoplay::LootAction;
        let profile = self.loot_profile();
        let me = self.wielder();
        let who = self.world.stats.name.clone();
        // Oldest first, each told the copies before it: the first is the one read.
        let mut carried: Vec<(u32, u32, u32)> = self
            .world
            .inventory()
            .map(|o| (o.guid, o.weenie_class_id, o.stack_size.max(1)))
            .collect();
        for (guid, before) in crate::autoplay::ledger::retag::in_arrival_order(&mut carried) {
            let Some(stats) = self.stats_of(guid).filter(|s| s.is_scroll()) else {
                continue;
            };
            if self.autoplay.ledger.why(guid) != Some(LEARNABLE) {
                continue;
            }
            let id = self.appraisals.get(&guid);
            let (action, rule) = match judge_loot(&stats, id, profile.as_deref(), &me, &who, before)
            {
                crate::profile::Verdict::Decided(LootAction::Keep, r) if r == LEARNABLE => continue,
                crate::profile::Verdict::Decided(a, r) if a.takes() => (a, r),
                _ => (LootAction::Sell, NOTHING_TO_TEACH.to_string()),
            };
            tracing::info!("scroll: {} now {action:?} ({rule})", stats.name);
            self.autoplay.tag_why(&stats, action, Some(&rule));
        }
    }

    /// Goal: read one scroll the pack holds whose spell is learnable, between fights: no live
    /// target, nothing hitting, no body being emptied. Claiming the tick stands the character still
    /// while the server reads (Scroll.cs:66-82; it drops to peace mode itself). With the loot rules'
    /// reading off, only a pursued quest's scroll is read (a `learn:` quest's `Read` step).
    pub(crate) fn autoplay_read_scrolls(&mut self, now: Instant) -> bool {
        let every = self.loot_profile().is_some_and(|p| p.looting.learn_scrolls);
        let asked: Vec<u32> = self
            .pursuit
            .iter()
            .filter_map(|l| match l.step {
                crate::autoplay::PursuitStep::Read { spell, .. } => Some(spell),
                _ => None,
            })
            .collect();
        if !every && asked.is_empty() {
            return false;
        }
        // A target unknown to the world counts as alive: it is still being fought.
        let engaged = [self.autoplay.casting_at(), self.attack_target]
            .into_iter()
            .flatten()
            .any(|g| {
                self.world
                    .objects
                    .get(&g)
                    .is_none_or(|o| o.alive_or_unknown())
            });
        // A use of a giver or a counter while the server reads is dropped, so not on those calls.
        let between_fights = !engaged
            && !self.under_attack()
            && self.autoplay.corpse.is_none()
            && self.visits.current.is_none()
            && !self.autoplay.growth.town_run_under_way()
            && !self.is_dead();
        if !between_fights {
            return false;
        }
        if self.autoplay.learnable.is_reading(now) {
            return true;
        }
        // One sent behind a cast is turned away busy (0x1D); `cast_in_flight` counts the read in.
        if self.autoplay.cast_in_flight(now) {
            return false;
        }
        let st = &self.autoplay.learnable;
        let me = self.world.player_guid;
        let scroll = self.world.inventory().find(|o| {
            let s = crate::items::ItemStats::of(o, me);
            s.is_scroll()
                && st.spells.contains(&s.spell_id)
                && (every || asked.contains(&s.spell_id))
                && st
                    .tried
                    .get(&o.guid)
                    .is_none_or(|t| now.duration_since(*t) >= SCROLL_RETRY)
        });
        let Some((guid, name)) = scroll.map(|o| (o.guid, o.name.clone())) else {
            return false;
        };
        let st = &mut self.autoplay.learnable;
        st.tried
            .retain(|_, t| now.duration_since(*t) < SCROLL_RETRY);
        st.tried.insert(guid, now);
        st.read_at = Some(now);
        tracing::info!("scroll: reading {name} ({guid:#010x})");
        self.autoplay.note(format!("reading {name}"), now);
        self.use_object(guid);
        true
    }
}

#[cfg(test)]
mod tests;
