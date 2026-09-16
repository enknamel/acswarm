use std::time::Instant;

#[cfg(doc)]
use super::Autoplay;
use super::KILL_SPOT;
use crate::refusals::OpenRefusal;
use crate::Client;

/// The creature a line says was reached and not hurt: "X resists your
/// spell" (ACE `TryResistSpell`, projectile or not) or "X evades your
/// attack.". Either way the shot got there.
pub(super) fn arrived_unharmed(text: &str) -> Option<&str> {
    text.strip_suffix(" resists your spell")
        .or_else(|| text.strip_suffix(" evades your attack."))
}

/// Who a line says has just cast a spell at this character to hurt it: a
/// war spell that landed ("Drudge Shaman blasts you for 12 points with
/// Flame Bolt I."), a drain ("Drudge Shaman casts Harm Other I and drains
/// 9 points of your health."), a vital taken ("You lose 20 points of mana
/// due to Drudge Shaman casting Mana to Health Other I on you"), or a
/// spell resisted ("You resist the spell cast by Drudge Shaman").
///
/// ACE tells a character it was hit by a swing with a notification, and
/// by a spell with nothing but one of these lines (`SpellProjectile`,
/// `WorldObject_Magic`). A caster working on the character from twenty
/// metres never closes in to swing, so without these it never counted as
/// attacking it at all.
///
/// "X cast Y on you" is left out on purpose: a fellow's buff reads the
/// same as a monster's curse, and a buff is not a fight.
pub(super) fn spell_attacker(text: &str) -> Option<&str> {
    if let Some(who) = text.strip_prefix("You resist the spell cast by ") {
        return Some(who);
    }
    if let Some(rest) = text.strip_prefix("You lose ") {
        let (_, by) = rest.split_once(" due to ")?;
        let (who, _) = by.strip_suffix(" on you")?.split_once(" casting ")?;
        return Some(who);
    }
    // A bolt that landed can come with any of these in front of it.
    let mut line = text;
    while let Some(rest) = ["Critical hit! ", "Overpower! ", "Sneak Attack! "]
        .into_iter()
        .find_map(|p| line.strip_prefix(p))
    {
        line = rest;
    }
    if let Some((before, after)) = line.split_once(" you for ") {
        // The verb is one word, and says how hard: "blasts", "singes".
        return after
            .contains(" points with ")
            .then(|| before.rsplit_once(' ').map(|(who, _)| who))
            .flatten()
            .filter(|who| !who.is_empty());
    }
    let (who, rest) = line.split_once(" casts ")?;
    (rest.contains(" and drains ") && rest.contains(" points of your ")).then_some(who)
}

impl Client {
    /// The server's words on a target it will not let us fight, "You
    /// cannot attack {name}" (see `refusals::Refusal::Attack`): the
    /// target in hand, when it is the one named, is given up at once
    /// rather than when the stall clock runs out. What makes a creature
    /// unattackable does not change while we stand there swinging.
    pub(crate) fn hear_attack_refusal(&mut self, name: &str, now: Instant) {
        let Some(guid) = self
            .attack_target
            .or_else(|| self.autoplay.engaged.map(|(g, ..)| g))
        else {
            return;
        };
        if self
            .world
            .objects
            .get(&guid)
            .is_some_and(|o| o.name == name)
        {
            self.give_up_target(guid, "the server will not let us attack it", now);
        }
    }

    /// The server's words refusing to open the container `named`, while
    /// a body is waiting to open. One refusing that body is acted on at
    /// once rather than waited out (see [`Autoplay::corpse_refused`]),
    /// and the walk to it, if there was one, ends with it.
    pub(crate) fn hear_corpse_refusal(&mut self, named: &str, why: OpenRefusal, now: Instant) {
        let Some((guid, ..)) = self.autoplay.corpse else {
            return;
        };
        // What the server calls it. Without the name there is no telling
        // this body's refusal from another's, so nothing is done.
        let Some(name) = self.world.objects.get(&guid).map(|o| o.name.clone()) else {
            return;
        };
        if self
            .autoplay
            .corpse_refused(named, why, &name, self.told, now)
        {
            self.stop_walking_to_loot();
        }
    }

    /// A line saying something cast a spell at this character to hurt it
    /// (see [`spell_attacker`]), kept in the same two fields a swing sets.
    /// Every "but fight back when it attacks you" carve-out reads those,
    /// and a caster attacks as surely as a creature that swings.
    pub(crate) fn hear_spell_attack(&mut self, text: &str, now: Instant) {
        let Some(who) = spell_attacker(text) else {
            return;
        };
        self.autoplay.attacked_by(who, now);
    }

    /// A line saying a shot got to something and did not hurt it (see
    /// [`arrived_unharmed`]): noted against the target it names.
    pub(crate) fn hear_arrival(&mut self, text: &str) {
        let Some(name) = arrived_unharmed(text) else {
            return;
        };
        let target = [self.autoplay.casting_at, self.attack_target]
            .into_iter()
            .flatten()
            .find(|g| self.world.objects.get(g).is_some_and(|o| o.name == name));
        if let Some(g) = target {
            self.autoplay.thrown = self.autoplay.thrown.filter(|(t, _)| *t != g);
            // Resisted or evaded, it still got there: the target is in
            // reach and being worked on, whatever its health says. Counting
            // only damage gave a creature that resisted a run of spells up
            // as out of reach.
            if let Some((engaged, _, health)) = self.autoplay.engaged {
                if engaged == g {
                    self.autoplay.engaged = Some((g, Instant::now(), health));
                }
            }
        }
    }

    /// A shot has gone out at `guid`: the clock on it getting there
    /// starts now, unless one is already running for that target.
    pub(super) fn throw_at(&mut self, guid: u32, now: Instant) {
        if self.autoplay.thrown.is_none_or(|(g, _)| g != guid) {
            self.autoplay.thrown = Some((guid, now));
        }
    }

    /// Where the creature a kill message names was standing: the one
    /// being fought when the message names it, else the nearest creature
    /// it names, the longest name that fits first (a "Mite Scion" is not
    /// a "Mite").
    pub(crate) fn killed_in(&self, text: &str) -> Option<glam::Vec3> {
        let me = self.my_position()?;
        let named = |name: &str| !name.is_empty() && text.contains(name);
        let fought = [self.attack_target, self.autoplay.casting_at]
            .into_iter()
            .flatten()
            .filter_map(|g| self.world.objects.get(&g))
            .find(|o| named(&o.name))
            .and_then(|o| o.world_pos());
        fought.or_else(|| {
            self.world
                .objects
                .values()
                .filter(|o| o.item_type & ac_world::item_type::CREATURE != 0)
                .filter(|o| named(&o.name))
                .filter_map(|o| Some((o.name.len(), o.world_pos()?)))
                .max_by(|a, b| {
                    a.0.cmp(&b.0)
                        .then(b.1.distance(me).total_cmp(&a.1.distance(me)))
                })
                .map(|(_, at)| at)
        })
    }

    /// A corpse is done with: the kill spot it lay at is too.
    pub(super) fn forget_kill_spot(&mut self, corpse: u32) {
        if let Some(at) = self.world.objects.get(&corpse).and_then(|o| o.world_pos()) {
            self.autoplay
                .kill_spots
                .retain(|(k, _)| k.truncate().distance(at.truncate()) > KILL_SPOT);
        }
    }
}

#[cfg(test)]
mod tests;
