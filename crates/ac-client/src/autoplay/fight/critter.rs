use crate::autoplay::{name_matches, wanted_target, Fight};
use crate::Client;

/// Appraisal int: a creature's level (ACE `Level`), for the creatures
/// the table has no level for.
pub(crate) const CREATURE_LEVEL: u32 = 25;

/// The most health a creature the table knows can have and still be a
/// critter. A Black Rabbit has five, a Chicken three, a Bunny three;
/// the smallest thing in a hunting field that is really a monster -- a
/// Gnawer Shrethlet -- has eight, a Gnawer Shreth fifteen, a Mite
/// Snippet twenty, a Drudge Skulker forty-two. One swing ends anything
/// under this line, and there is nothing in it for a character that
/// can swing.
const CRITTER_HEALTH: u32 = 5;

/// The most health a creature the table does not know can have and
/// still be a critter. The table was read off one server's data and a
/// live server keeps animals that data never had: a Cow is level 8
/// with twenty health, docile until attacked, and the table's line of
/// five would have it killed. Thirty covers a Cow with room for a
/// server that gave it a little more, and stays under the Drudge
/// Skulker's forty-two and the Auroch Yearling's sixty-five, the
/// smallest things worth hunting at the level that outgrows a Cow. A
/// Mite Snippet has a Cow's twenty, which is why the table's own line
/// still holds for what the table knows: nothing but the table tells
/// those two apart.
const STRANGER_HEALTH: u32 = 30;

/// What a creature in view has been seen doing, which is all a client
/// can know about its temper. ACE never sends a creature's tolerance
/// -- it lives in the server's monster awareness and nowhere else --
/// but every creature shows what it does, and a passive one, by
/// definition, never starts a fight.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Seen {
    /// It has attacked this character lately: a blow landed, a swing
    /// missed or a spell cast at it (see `Client::hit_lately_by`).
    pub attacked_us: bool,
    /// The server has walked it at this character or at one of the
    /// team (see `WorldObject::walked_at`: the walk is remembered, since
    /// the live move target shows a chase only for the moment between
    /// the walk and the next position report).
    pub targets_us_or_mate: bool,
    /// It has walked at anyone at all, or something of ours -- a mate,
    /// a creature this character summoned -- is on it.
    pub fighting_anyone: bool,
}

impl Seen {
    /// Whether it has done nothing to anyone: what passive looks like
    /// from outside.
    pub fn quiet(self) -> bool {
        !(self.attacked_us || self.targets_us_or_mate || self.fighting_anyone)
    }
}

/// What the critter rule makes of a creature (see [`critter`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Critter {
    /// Worth the fight, or in one already.
    Fight,
    /// Beneath fighting: walked past.
    WalkPast,
    /// Nothing to judge it by yet: ask the server what it is before
    /// deciding, and leave it alone meanwhile.
    Appraise,
}

/// What the table knows about a creature, and how it was found (see
/// [`critter`]).
#[derive(Debug, Clone, Copy)]
pub enum Hint<'a> {
    /// By weenie: this very kind of creature, whose figures are its own.
    Weenie(&'a ac_world::elements::Creature),
    /// By the end of its name: a kind the table has not seen, that may
    /// be a stronger version of a familiar thing. Its temper is
    /// believed -- a thing called a Rabbit is not going to start a
    /// fight -- and its level and health are not: a "Dire Brown Rabbit" at
    /// level 40 with three hundred health was walked past on the
    /// Rabbit's four and five.
    Name(&'a ac_world::elements::Creature),
}

impl<'a> Hint<'a> {
    /// The row, however it was found.
    pub fn row(self) -> &'a ac_world::elements::Creature {
        match self {
            Hint::Weenie(k) | Hint::Name(k) => k,
        }
    }

    /// The row's level and health, when they are this creature's.
    fn figures(self) -> Option<&'a ac_world::elements::Creature> {
        match self {
            Hint::Weenie(k) => Some(k),
            Hint::Name(_) => None,
        }
    }
}

/// Whether a creature is beneath fighting: it has started nothing with
/// anyone, it dies to a swing or two, and the character has long since
/// outgrown it. A Rabbit, a Chicken, a Bunny, a Cow.
///
/// What it has been seen doing comes first, because that is known for
/// every creature and the table is not. Something that has attacked
/// the character, or is walking at it or at a mate, or is fighting
/// anyone, is a fight already, whatever else is known about it. This
/// used to ask the table first and fight whatever the table did not
/// know, and the table was read off one server's data: a live
/// server's Cow was not in it, so it was killed for not being in it.
///
/// Passive on its own is not the answer. A Revenant stands there until
/// it is hit, and so do Cursed Bones and a Silver Tusker: half of what
/// a hunting ground is for waits to be provoked, and those are worth
/// sixty levels or more. So the level has to be well below the
/// character's, twice over.
///
/// Nor is passive and far below by level, which is what this asked at
/// first and what emptied the fields it was meant to tidy. ACE gives
/// the newbie-field spawns Retaliate so they do not come at a new
/// player -- generators 2007 and 5150 around Holtburg put out nothing
/// but Drudge Skulkers, Gnawer Shreths, Mites and Mosswarts, every one
/// of them passive and level 8 -- so twice the level left a level 16
/// character with nothing in reach to attack anywhere in Holtburg, and
/// no reason to walk anywhere either. The level is a poor separator in
/// any case: a Black Rabbit is level 4 and a Gnawer Shrethlet level 2.
/// What does separate them is how much they can take, and that is the
/// figure this leans on for anything that will fight back.
///
/// The table, where it knows the creature, is a hint and not a gate.
/// Its tolerance says outright whether the thing starts fights, which
/// a creature that has not noticed the character yet does not show,
/// so that is believed. Its level and health stand in for a creature
/// found by weenie until an appraisal gives this very creature's,
/// which beat them. A row found by name is a guess at the kind of
/// thing it is, and its figures are not this creature's at all (see
/// [`Hint::Name`]): the appraisal is waited for. A creature the table
/// knows is held to the table's own line (`CRITTER_HEALTH`) and one
/// it does not to a wider one (`STRANGER_HEALTH`); not being in the
/// table is never itself a reason to fight.
///
/// The one flag that stands on its own is "never attacks anything":
/// that is not a creature with a health bar, it is scenery with one --
/// an Egg, a Totem, a Pillar, a Reinforced Door -- and hitting it is a
/// chore, not a fight, however much health it has.
///
/// With no level to judge by, or no health, the answer is to ask: an
/// appraisal brings both, and a creature is neither walked past on a
/// guess nor attacked on one.
pub fn critter(
    seen: Seen,
    level: Option<u32>,
    health: Option<u32>,
    hint: Option<Hint<'_>>,
    mine: i32,
) -> Critter {
    use ac_world::elements::tolerance as flag;
    if !seen.quiet() {
        return Critter::Fight;
    }
    let row = hint.map(Hint::row);
    // The table knows it starts fights: it will, once it notices.
    if row.is_some_and(|k| !k.passive()) {
        return Critter::Fight;
    }
    let figures = hint.and_then(Hint::figures);
    let Some(level) = level.or(figures.and_then(|k| k.level)) else {
        return Critter::Appraise;
    };
    if i64::from(level) * 2 > i64::from(mine) {
        return Critter::Fight;
    }
    if row.is_some_and(|k| k.tolerance & flag::NO_ATTACK != 0) {
        return Critter::WalkPast;
    }
    let Some(health) = health.or(figures.map(|k| k.health).filter(|h| *h > 0)) else {
        return Critter::Appraise;
    };
    let line = if row.is_some() {
        CRITTER_HEALTH
    } else {
        STRANGER_HEALTH
    };
    if health <= line {
        Critter::WalkPast
    } else {
        Critter::Fight
    }
}

impl Client {
    /// What is known about the kind of creature `guid` is.
    pub(crate) fn creature_known(
        &self,
        guid: u32,
    ) -> Option<&'static ac_world::elements::Creature> {
        let o = self.world.objects.get(&guid)?;
        ac_world::elements::known(o.weenie_class_id, &o.name)
    }

    /// Whether `o` is a critter to walk past rather than fight (see
    /// [`critter`]), or one left alone until the server has said what
    /// it is.
    ///
    /// A name in "only these" outranks the rule: that is a player
    /// saying outright what to hunt. The rest of what outranks it --
    /// the creature is attacking the character, walking at it or at a
    /// mate, or fighting anyone, a creature this one summoned included
    /// -- is the behaviour the rule itself reads first.
    pub(crate) fn a_critter(&self, o: &ac_world::WorldObject, cfg: &Fight) -> bool {
        match self.critter_verdict(o, cfg) {
            Critter::Fight => false,
            Critter::WalkPast => true,
            // Left alone while the question is out. ACE answers about a
            // creature it has with its profile every time, assessed or
            // not, and about one it has not got with nothing at all: an
            // answer with no profile says the thing is not there to
            // fight, and it is left alone. Answered with a profile and
            // still nothing to judge by -- not something ACE does, whose
            // every profile comes with a level -- it is fought: it can
            // be attacked, and nothing waits for ever on a second
            // answer.
            Critter::Appraise => self
                .appraisals
                .get(&o.guid)
                .is_none_or(|a| a.creature.is_none()),
        }
    }

    /// What the critter rule makes of `o`: what it has been seen doing,
    /// what an appraisal has said about it, and what the table knows
    /// (see [`critter`]).
    ///
    /// This runs for every creature in view every tick -- `would_fight`
    /// asks it -- so the one question that walks the whole object map,
    /// whether a creature this character summoned is on it, is asked
    /// last and only of something the rest would have walked past or
    /// asked about.
    pub(crate) fn critter_verdict(&self, o: &ac_world::WorldObject, cfg: &Fight) -> Critter {
        if !cfg.skip_critters || name_matches(&o.name, &cfg.only) {
            return Critter::Fight;
        }
        let appraisal = self.appraisals.get(&o.guid);
        let level = appraisal
            .and_then(|a| a.int(CREATURE_LEVEL))
            .and_then(|l| u32::try_from(l).ok());
        let health = appraisal
            .and_then(|a| a.creature.as_ref())
            .map(|c| c.health_max);
        let mates = &self.autoplay.team.mates;
        let ours = |g: u32| self.world.player_guid == Some(g) || mates.iter().any(|m| m.guid == g);
        let seen = Seen {
            attacked_us: self.hit_lately_by(&o.name),
            targets_us_or_mate: o.walked_at.is_some_and(ours),
            fighting_anyone: o.walked_at.is_some()
                || mates.iter().any(|m| m.target == Some(o.guid)),
        };
        let hint = ac_world::elements::creature_by_id(o.weenie_class_id)
            .map(Hint::Weenie)
            .or_else(|| ac_world::elements::creature(&o.name).map(Hint::Name));
        let verdict = critter(seen, level, health, hint, self.world.stats.level);
        // Its fight, and the character's to finish.
        if verdict != Critter::Fight && self.a_pet_is_on(o.guid) {
            return Critter::Fight;
        }
        verdict
    }

    /// Ask the server about the creature in reach the critter rule
    /// cannot judge yet (see [`Critter::Appraise`]): the answer brings
    /// the level and the health it judges by, and until it comes the
    /// creature is left alone rather than attacked. Asked as a target
    /// is being picked, which is when the answer is wanted, and the
    /// queue asks about each once.
    ///
    /// Asking is not always free. ACE wakes an idle creature whose
    /// tolerance is "attacks once appraised" onto whoever appraised it
    /// (`Player.OnAppraisal`): a Wisp, a Scarecrow, but a Virindi
    /// Observer too, and the table's own such rows all carry a level
    /// and are never asked. A stranger of that kind is provoked by the
    /// question, and then fought as anything that attacks is. So the
    /// nearest is asked about, alone, and nothing is asked while
    /// something is attacking the character: what the question wakes
    /// comes one at a time, with the last fight over first.
    pub(crate) fn ask_about_strangers(&mut self, me: glam::Vec3, cfg: &Fight) {
        if !cfg.skip_critters || self.under_attack() {
            return;
        }
        let underground = self.underground();
        let nearest = self
            .world
            .objects
            .values()
            .filter(|o| {
                o.item_type & ac_world::item_type::CREATURE != 0
                    && o.object_desc_flags & ac_world::object_desc_flags::ATTACKABLE != 0
                    && o.object_desc_flags & ac_world::object_desc_flags::PLAYER == 0
                    && o.health.unwrap_or(1.0) > 0.0
                    && !o.is_player
                    && o.pet_owner == 0
                    && !self.appraisals.contains_key(&o.guid)
            })
            .filter_map(|o| {
                let away = o.world_pos()?.distance(me);
                (away <= cfg.radius).then_some((o, away))
            })
            .filter(|(o, _)| self.area_allows(o, underground) && wanted_target(&o.name, cfg))
            .filter(|(o, _)| self.critter_verdict(o, cfg) == Critter::Appraise)
            .min_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(o, _)| o.guid);
        // Already asked about and not yet answered: the next waits on
        // the answer, which is the one at a time.
        if let Some(guid) = nearest {
            if self.appraise_many([guid]) > 0 {
                tracing::debug!(
                    "autoplay: asking about {guid:#010x}, which the critter rule cannot judge"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests;
