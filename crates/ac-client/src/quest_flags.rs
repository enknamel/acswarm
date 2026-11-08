//! The quest flags a portal asks for, from the character's side: which it holds, earned by
//! talking to whoever stamps one (`ac_world::unlocks`), kept between sessions, and handed to every
//! journey plan (`Travel::quests_done`) so the portals they open are taken.
//!
//! The server never sends a character's quest registry; a flag is held from the use of its NPC in
//! reach at its level, and let go when a portal asking for it turns the character away.

use crate::Client;

/// Someone this near an unlock's listed spot is its NPC (metres): the data places each one.
const IS_THE_NPC: f32 = 3.0;
/// WeenieError YouMustCompleteQuestToUsePortal (WeenieError.cs:543).
const NO_QUEST_FOR_PORTAL: u32 = 0x0474;
/// WeenieError QuestSolvedTooLongAgo (WeenieError.cs:474): a timed flag run out.
const QUEST_TOO_LONG_AGO: u32 = 0x0446;

impl Client {
    /// The quest flags this character holds.
    pub fn quest_flags(&self) -> &[String] {
        &self.travel.quests_done
    }

    /// `guid` was used: if it is the NPC of an unlock the character's level earns, the flag is
    /// held from now on. The NPC's own check is the level alone (`unlocks.sh`).
    pub(crate) fn note_unlock_talk(&mut self, guid: u32) {
        let Some(o) = self.world.objects.get(&guid) else {
            return;
        };
        let Some(p) = o.display.or(o.position) else {
            return;
        };
        let at = ac_world::landblock_origin(p.cell) + p.local;
        let level = self.world.stats.level.max(0) as u32;
        let earned: Vec<String> = ac_world::unlocks::all()
            .iter()
            .filter(|u| u.npc == o.name && u.cell >> 16 == p.cell >> 16)
            .filter(|u| u.at.distance(at) <= IS_THE_NPC && u.min_level <= level)
            .filter(|u| !self.holds_quest_flag(&u.flag))
            .map(|u| u.flag.clone())
            .collect();
        if earned.is_empty() {
            return;
        }
        tracing::info!("quest: talked to {}; flagged for {earned:?}", o.name);
        self.travel.quests_done.extend(earned);
        self.save_quest_flags();
    }

    /// A quest done stamped `flags`: held from now on, and saved.
    pub(crate) fn hold_quest_flags(&mut self, flags: &[String]) {
        let new: Vec<String> = flags
            .iter()
            .filter(|f| !self.holds_quest_flag(f))
            .cloned()
            .collect();
        if new.is_empty() {
            return;
        }
        tracing::info!("quest: flagged for {new:?}");
        self.travel.quests_done.extend(new);
        self.save_quest_flags();
    }

    /// A portal asking for `flag` turned the character away: it is not held after all.
    pub(crate) fn drop_quest_flag(&mut self, flag: &str) {
        let before = self.travel.quests_done.len();
        self.travel
            .quests_done
            .retain(|f| !f.eq_ignore_ascii_case(flag));
        if self.travel.quests_done.len() != before {
            tracing::info!("quest: {flag} is not held after all");
            self.save_quest_flags();
        }
        self.forget_stampers(flag);
    }

    /// A weenie error came in: a portal refusing for its quest flag (Portal.cs:246-258, sent by
    /// QuestManager.cs:537-553) means the flag the journey's portal asks for is not held.
    pub(crate) fn portal_quest_error(&mut self, code: u32) {
        if !matches!(code, NO_QUEST_FOR_PORTAL | QUEST_TOO_LONG_AGO) {
            return;
        }
        // The portal stood in, not the journey's next: that may be anywhere.
        let Some(flag) = self
            .portal_at_hand()
            .map(|p| p.quest.clone())
            .filter(|q| !q.is_empty())
        else {
            return;
        };
        self.drop_quest_flag(&flag);
    }

    /// Whether the character holds `flag`.
    pub fn holds_quest_flag(&self, flag: &str) -> bool {
        self.travel
            .quests_done
            .iter()
            .any(|f| f.eq_ignore_ascii_case(flag))
    }

    /// Go and talk to `u`'s NPC: a visit, journey and all. False when there is no way.
    pub(crate) fn visit_unlock(&mut self, u: &ac_world::unlocks::Unlock) -> bool {
        let l = ac_world::landmarks::Landmark {
            kind: ac_world::landmarks::Kind::Npc,
            name: u.npc.clone(),
            cell: u.cell,
            at: u.at,
        };
        let ok = self.visit_landmark(&l);
        tracing::info!("quest: to {} for {} ({ok})", u.npc, u.flag);
        ok
    }

    /// This character's file `<name><suffix>` under `quests/<host>/`, beside the loot profiles.
    pub(crate) fn quest_file(&self, suffix: &str) -> Option<std::path::PathBuf> {
        let name = self.world.stats.name.trim();
        if name.is_empty() {
            return None;
        }
        let beside = self.profiles.dir().parent()?.to_path_buf();
        Some(
            beside
                .join("quests")
                .join(ac_store::file_safe(&self.config.host))
                .join(format!("{}{suffix}", ac_store::file_safe(name))),
        )
    }

    fn quest_flags_path(&self) -> Option<std::path::PathBuf> {
        self.quest_file(".json")
    }

    /// Read back the flags earned in earlier sessions, at login.
    pub(crate) fn load_quest_flags(&mut self) {
        let Some(path) = self.quest_flags_path() else {
            return;
        };
        match ac_store::read_json::<Vec<String>>(&path) {
            Ok(Some(flags)) => {
                for f in flags {
                    if !self.holds_quest_flag(&f) {
                        self.travel.quests_done.push(f);
                    }
                }
                tracing::info!("quest: flags read back: {:?}", self.travel.quests_done);
            }
            Ok(None) => {}
            Err(e) => tracing::warn!("quest: reading {}: {e}", path.display()),
        }
    }

    pub(crate) fn save_quest_flags(&self) {
        let Some(path) = self.quest_flags_path() else {
            return;
        };
        let flags = &self.travel.quests_done;
        if let Err(e) = ac_store::write_json_atomic(&path, flags, ac_store::Visibility::Normal) {
            tracing::warn!("quest: writing {}: {e}", path.display());
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
