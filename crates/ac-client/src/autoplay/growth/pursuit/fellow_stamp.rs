//! A quest stamped on the fellowship (a `fellow:` need: Kivik Lir's sarcophagus): in one, founded
//! alone as the arena's is (Player_Fellowship.cs:19-31), a creature whose death stamps it is
//! hunted. The Death set runs for the last damager's fellowship (EmoteManager.cs:1998-2008,
//! 1252-1263); the client cannot read a fellowship's quests, so our kills' are remembered, and
//! forgotten out of it: they live on the Fellowship object (Fellowship.cs:67).

use std::collections::BTreeMap;
use std::sync::OnceLock;

use ac_world::quests::{Want, Yield};
use glam::Vec3;

use super::{is_stuck, stuck, Ask, Hunting, PursuitStep};
use crate::quest_log::QuestLine;
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
    /// Quest `line`'s step while its fellowship must hold `quest`: a fellowship founded, then a
    /// creature stamping it hunted (or its ground's way had first); else why not. Another row's
    /// fellowship quest is worked when `quest` has no way: a level 120 has Kixkti Xri's stamp,
    /// not Ibrexi Jikti's, as the Guardian of the High Matriarch feeds each level band its own.
    pub(super) fn fellow_ways(
        &self,
        line: &QuestLine,
        quest: &str,
    ) -> (PursuitStep, Option<Hunting>) {
        if self.arena.team_unfellowed {
            let why = format!("{quest} is stamped on a fellowship, and the team's is switched off");
            return (stuck(why), None);
        }
        let rows = crate::quest_log::worked_rows(line).filter(|r| r.fits_level(self.facts.level));
        let mut quests: Vec<&str> = vec![quest];
        let others = rows
            .flat_map(|r| r.needs.iter())
            .filter(|n| !crate::quest_log::holds(self.facts, n))
            .filter_map(|n| n.strip_prefix("fellow:"));
        for q in others {
            if !quests.iter().any(|h| h.eq_ignore_ascii_case(q)) {
                quests.push(q);
            }
        }
        let mut first = None;
        for q in quests {
            let want = Want::Fellow(q.to_ascii_lowercase());
            let step = self.obtain(&[want], 1, &format!("the fellowship's {q}"), 0);
            let got = self.way_first(&line.id, step);
            if is_stuck(&got.0) {
                first.get_or_insert(got);
            } else if self.arena.fellowship {
                return got;
            } else {
                let quest = q.to_string();
                return (PursuitStep::Fellow { quest }, None);
            }
        }
        first.unwrap_or_else(|| (stuck(format!("nothing stamps {quest}")), None))
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
