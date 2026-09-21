use std::time::Instant;

use super::melee::GIVE_UP_FOR;
use crate::autoplay::{Fight, Style};
use crate::dodge::How;
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
        let Some(me) = self.my_position() else {
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

    /// Test-only: hold a fight the way a caster in one holds it, so what
    /// a front end's stop lets go can be read from outside the crate.
    #[cfg(any(test, feature = "testkit"))]
    pub fn take_up_fight(&mut self, guid: u32) {
        self.attack_target = Some(guid);
        self.autoplay.casting_at = Some(guid);
        self.autoplay.engaged = Some((guid, Instant::now(), 1.0));
        self.autoplay.closing = Some((guid, 1.0));
    }

    /// Test-only: what of a fight is still held -- the swing's target,
    /// the spell's, the engagement, the closing walk.
    #[cfg(any(test, feature = "testkit"))]
    pub fn fight_held(&self) -> (bool, bool, bool, bool) {
        (
            self.attack_target.is_some(),
            self.autoplay.casting_at.is_some(),
            self.autoplay.engaged.is_some(),
            self.autoplay.closing.is_some(),
        )
    }

    /// Let the target go and leave it alone for [`GIVE_UP_FOR`], saying
    /// why once.
    pub(crate) fn give_up_target(&mut self, guid: u32, why: &str, now: Instant) {
        let name = self.world.name_or_hex(guid);
        self.autoplay
            .note(format!("giving up on {name}: {why}"), now);
        self.autoplay.given_up.expire(now, GIVE_UP_FOR);
        self.autoplay.given_up.mark(guid, now);
        self.let_go(Release::Fight);
    }

    /// The attack this character is about to make at `guid`, for the
    /// sight test: out of `ready` the spell the cast would throw at that
    /// creature, the wielded launcher's shot, or a swing.
    ///
    /// `stance` is what is in the character's hands, and
    /// `Client::missile` is not: that is the combat mode the server has
    /// been told about, which a caster's magic stance clears and which
    /// an archer has not entered before its first shot of a fight.
    ///
    /// The spell is named per creature by the very fn the cast chooses
    /// with (`autoplay_fight_with_spells`), so the flight tested and the
    /// flight thrown are one answer. `None` when a caster can name no
    /// spell: there is no attack to test, and no `How` says so -- a
    /// `How::Melee` would name a swing it cannot make and a reach it
    /// does not have (`Client::attack_range`).
    fn attack_kind(&self, stance: Stance, ready: &[u32], guid: u32) -> Option<How> {
        match stance {
            Stance::Magic => {
                let o = self.world.objects.get(&guid);
                let wcid = o.map_or(0, |o| o.weenie_class_id);
                let name = o.map_or("", |o| o.name.as_str());
                ac_world::elements::best_spell(wcid, name, ready).map(|(id, _)| How::Spell(id))
            }
            Stance::Missile => Some(How::Missile),
            Stance::Melee => Some(How::Melee),
        }
    }

    /// The nearest creature the name rules allow, within the radius.
    pub(super) fn pick_target(&mut self, cfg: &Fight) -> Option<u32> {
        let underground = self.underground();
        let me = self.my_position()?;
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
        // With nothing to choose between, the sight test cannot change
        // the answer, and `ready_spells` walks the packs: it is not paid
        // to rank one creature, or none.
        if candidates.len() < 2 {
            return candidates.first().map(|(g, _)| *g);
        }
        // The nearest one we can actually hit: one behind a wall is
        // taken only when nothing is in sight, and then the fight rules
        // walk round to it. In sight of what is the attack being made:
        // an arc is lobbed over what stops a bolt, an arrow falls on the
        // way and past its reach gets nowhere (see `crate::aim`).
        let stance = self.combat_stance();
        let ready: Vec<u32> = match stance {
            Stance::Magic => self
                .ready_spells(cfg)
                .into_iter()
                .map(|(id, _)| id)
                .collect(),
            _ => Vec::new(),
        };
        candidates
            .into_iter()
            .map(|(guid, at)| {
                // A caster with no spell to name has no flight to test,
                // and then the nearest is as good an answer as any.
                let how = self.attack_kind(stance, &ready, guid);
                let seen = how.is_none_or(|how| self.shot_clears(guid, how));
                ((!seen) as u8, at.distance(me), guid)
            })
            .min_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)))
            .map(|(_, _, g)| g)
    }
}

#[cfg(test)]
mod tests;
