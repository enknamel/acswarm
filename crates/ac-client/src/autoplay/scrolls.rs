//! Spell scrolls: the spells this character could learn from one now, worked out again as its
//! spellbook and magic skills change, and a scroll the loot rules kept read from the pack while
//! no fight or journey is on (`Looting::learn_scrolls`).

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

    /// Work [`Self::learnable_spells`] out again when the spellbook or a school's skill changed.
    fn refresh_learnable(&mut self) {
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
        if key == self.autoplay.learnable.key && !self.autoplay.learnable.spells.is_empty() {
            return;
        }
        let Ok(table) = self.assets.spell_table() else {
            return;
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
    }

    /// Housekeeping: keep the learnable spells current, and read one scroll the pack holds whose
    /// spell is among them while nothing is fought or travelled (a use breaks a journey off).
    pub(crate) fn autoplay_learn_scrolls(&mut self, now: Instant) {
        self.refresh_learnable();
        if !self.loot_profile().is_some_and(|p| p.looting.learn_scrolls) {
            return;
        }
        let st = &self.autoplay.learnable;
        let busy = st
            .read_at
            .is_some_and(|t| now.duration_since(t) < READ_TAKES);
        let quiet = self.attack_target.is_none()
            && self.autoplay.nearest_fight.is_none()
            && !self.traveling()
            && !self.is_dead();
        if busy || !quiet {
            return;
        }
        let me = self.world.player_guid;
        let scroll = self.world.inventory().find(|o| {
            let s = crate::items::ItemStats::of(o, me);
            s.is_scroll()
                && st.spells.contains(&s.spell_id)
                && st
                    .tried
                    .get(&o.guid)
                    .is_none_or(|t| now.duration_since(*t) >= SCROLL_RETRY)
        });
        let Some((guid, name)) = scroll.map(|o| (o.guid, o.name.clone())) else {
            return;
        };
        let st = &mut self.autoplay.learnable;
        st.tried
            .retain(|_, t| now.duration_since(*t) < SCROLL_RETRY);
        st.tried.insert(guid, now);
        st.read_at = Some(now);
        tracing::info!("scroll: reading {name} ({guid:#010x})");
        self.autoplay.note(format!("reading {name}"), now);
        self.use_object(guid);
    }
}

#[cfg(test)]
mod tests;
