//! A quest stamped on the fellowship (a `fellow:` need: Kivik Lir's sarcophagus): in one, founded
//! alone as the arena's is (Player_Fellowship.cs:19-31), a creature whose death stamps it is
//! hunted. The Death set runs for the last damager's fellowship (EmoteManager.cs:1998-2008,
//! 1252-1263); the client cannot read a fellowship's quests, so our kills' are remembered, and
//! forgotten out of it: they live on the Fellowship object (Fellowship.cs:67).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use ac_world::quests::{Want, Yield};
use glam::Vec3;

use super::{stuck, Ask, Hunting, PursuitStep};
use crate::Client;

/// Each creature whose death stamps quests on its killer's fellowship, and those quests.
fn stamped_on_fellows() -> &'static BTreeMap<u32, Vec<String>> {
    static KILLS: OnceLock<BTreeMap<u32, Vec<String>>> = OnceLock::new();
    KILLS.get_or_init(|| {
        let mut m: BTreeMap<u32, Vec<String>> = BTreeMap::new();
        for s in ac_world::quests::sources() {
            if let (Want::Fellow(q), Yield::Kill) = (&s.want, s.kind) {
                m.entry(s.wcid).or_default().push(q.clone());
            }
        }
        m
    })
}

impl Ask<'_> {
    /// Quest `id`'s step while its fellowship must hold `quest`: a fellowship founded, then a
    /// creature stamping it hunted (or its ground's way had first); else why not.
    pub(super) fn fellow_step(&self, id: &str, quest: &str) -> (PursuitStep, Option<Hunting>) {
        if self.arena.team_unfellowed {
            let why = format!("{quest} is stamped on a fellowship, and the team's is switched off");
            return (stuck(why), None);
        }
        if !self.arena.fellowship {
            let quest = quest.to_string();
            return (PursuitStep::Fellow { quest }, None);
        }
        let want = Want::Fellow(quest.to_ascii_lowercase());
        let step = self.obtain(&[want], 1, &format!("the fellowship's {quest}"), 0);
        self.way_first(id, step)
    }
}

impl Client {
    /// The quests the fellowship this character is in holds by our kills; none out of one.
    pub(crate) fn fellow_quests(&self) -> Vec<String> {
        match self.world.fellowship {
            Some(_) => self.autoplay.growth.fellow_earned.clone(),
            None => Vec::new(),
        }
    }

    /// Out of a fellowship, what its kills earned is gone with it (disbanded, or left).
    pub(crate) fn forget_fellow_quests(&mut self) {
        let earned = &mut self.autoplay.growth.fellow_earned;
        if self.world.fellowship.is_none() && !earned.is_empty() {
            earned.clear();
            self.quest_board_at = None;
        }
    }

    /// Our killing blow at `at`, in a fellowship: the quests a creature lying there stamps on it.
    pub(crate) fn note_fellow_kill(&mut self, at: Vec3) {
        if self.world.fellowship.is_none() {
            return;
        }
        let kills = stamped_on_fellows();
        let mut earned: Vec<String> = self
            .world
            .objects
            .values()
            .filter(|o| o.world_pos().is_some_and(|p| p.distance(at) < 1.0))
            .filter_map(|o| kills.get(&o.weenie_class_id))
            .flatten()
            .filter(|q| !self.autoplay.growth.fellow_earned.contains(q))
            .cloned()
            .collect();
        earned.sort_unstable();
        earned.dedup();
        if earned.is_empty() {
            return;
        }
        tracing::info!("quest: the fellowship holds {earned:?}");
        self.autoplay.growth.fellow_earned.extend(earned);
        self.quest_board_at = None;
    }
}
