use std::time::Instant;

use super::melee::GIVE_UP_FOR;
use crate::autoplay::{Fight, Style};
use crate::{Client, Stance};

/// Whether `name` contains any of `list`, case-insensitively. An empty
/// list matches nothing.
pub fn name_matches(name: &str, list: &[String]) -> bool {
    let name = name.to_lowercase();
    list.iter()
        .any(|w| !w.trim().is_empty() && name.contains(&w.trim().to_lowercase()))
}

/// Whether a creature called `name` is one to fight.
pub fn wanted_target(name: &str, f: &Fight) -> bool {
    if name_matches(name, &f.avoid) {
        return false;
    }
    f.only.iter().all(|w| w.trim().is_empty()) || name_matches(name, &f.only)
}

/// How much of a fight to let go of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Release {
    /// The spell's target only. A swing keeps `Client::attack_target`,
    /// which the server goes on driving until it is told otherwise.
    Cast,
    /// Both targets, while the engagement stands: another rule has this tick.
    Targets,
    /// Both targets and the engagement: this fight is over.
    Fight,
    /// The engagement and the spell's target, keeping the swing's. Only
    /// the Academy's turn towards a corpse lets go this far.
    Engagement,
}

impl Client {
    /// Whether there is a fight to be had where the character stands:
    /// something it would take on within its fight radius, or one it has
    /// already joined.
    ///
    /// This is what says a spot has gone quiet, and it reads the world
    /// rather than the status line (see
    /// `Client::autoplay_watch_the_ground`).
    pub(crate) fn a_fight_in_sight(&mut self, now: Instant) -> bool {
        // Not in the world yet: nothing to say about the ground, and
        // nothing that should start a clock running on it.
        let Some(me) = self.player.as_ref().map(|p| p.world_position()) else {
            return true;
        };
        // A fight already joined counts wherever it has led. A creature
        // chased past the radius is still a fight, and walking off to
        // look for one in the middle of it is not looking for a fight.
        let joined = self
            .attack_target
            .or_else(|| self.autoplay.casting_at())
            .and_then(|g| self.world.objects.get(&g))
            .is_some_and(|o| o.health.unwrap_or(0.0) > 0.0);
        if joined {
            return true;
        }
        // Every session pays for this on every tick, so the cheap
        // question -- is it near enough -- is asked first.
        let underground = self.underground();
        let cfg = &self.autoplay.config.fight;
        self.world
            .objects
            .values()
            .filter(|o| {
                o.world_pos()
                    .is_some_and(|at| at.distance(me) <= cfg.radius)
            })
            .any(|o| self.would_fight(o, cfg, underground, now))
    }

    /// Whether a creature this character summoned is already on `guid`:
    /// its fight, and the character's to finish. Read off what the pet
    /// last walked at, which outlasts the position reports of the walk.
    pub(super) fn a_pet_is_on(&self, guid: u32) -> bool {
        let Some(me) = self.world.player_guid else {
            return false;
        };
        self.world
            .objects
            .values()
            .any(|o| o.pet_owner == me && o.walked_at == Some(guid))
    }

    /// How this character is fighting right now: what its hands give.
    pub fn fighting_style(&self) -> Style {
        match self.combat_stance() {
            Stance::Melee => Style::Melee,
            Stance::Missile => Style::Missile,
            Stance::Magic => Style::Magic,
        }
    }

    /// Let the fight go, as far as `how` says. Giving up on the creature
    /// for a while is `give_up_target`, which is this plus the list.
    pub fn let_go(&mut self, how: Release) {
        if !matches!(how, Release::Cast | Release::Engagement) {
            self.attack_target = None;
        }
        self.autoplay.casting_at = None;
        if matches!(how, Release::Fight | Release::Engagement) {
            self.autoplay.engaged = None;
            self.autoplay.closing = None;
        }
    }

    /// Let the target go and leave it alone for [`GIVE_UP_FOR`], saying
    /// why once.
    pub(crate) fn give_up_target(&mut self, guid: u32, why: &str, now: Instant) {
        let name = self
            .world
            .objects
            .get(&guid)
            .map(|o| o.name.clone())
            .unwrap_or_else(|| format!("{guid:#010x}"));
        self.autoplay
            .note(format!("giving up on {name}: {why}"), now);
        self.autoplay.given_up.expire(now, GIVE_UP_FOR);
        self.autoplay.given_up.mark(guid, now);
        self.let_go(Release::Fight);
    }

    /// The nearest creature the name rules allow, within the radius.
    pub(super) fn pick_target(&mut self, cfg: &Fight) -> Option<u32> {
        let underground = self.underground();
        let me = self.player.as_ref()?.world_position();
        let now = Instant::now();
        // A follower fights beside its leader, not wherever a monster
        // happens to be.
        let leader_at = self.followed_leader().map(|m| m.world);
        let fight_radius = self.autoplay.config.team.fight_radius.max(1.0);
        // What cannot be judged yet is asked about, not attacked.
        self.ask_about_strangers(me, cfg);
        let candidates: Vec<(u32, glam::Vec3)> = self
            .world
            .objects
            .values()
            .filter(|o| self.would_fight(o, cfg, underground, now))
            .filter_map(|o| {
                let at = o.world_pos()?;
                let near_leader = leader_at.is_none_or(|l| at.distance(l) <= fight_radius);
                (at.distance(me) <= cfg.radius && near_leader).then_some((o.guid, at))
            })
            .collect();
        // The nearest one we can actually hit: one behind a wall is
        // taken only when nothing is in sight, and then the fight rules
        // walk round to it.
        let how = if self.missile {
            crate::dodge::How::Missile
        } else {
            crate::dodge::How::Melee
        };
        candidates
            .into_iter()
            .map(|(guid, at)| {
                let seen = self.shot_clears(guid, how);
                ((!seen) as u8, at.distance(me), guid)
            })
            .min_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)))
            .map(|(_, _, g)| g)
    }
}

#[cfg(test)]
mod tests;
