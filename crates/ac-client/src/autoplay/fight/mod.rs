pub(crate) mod critter;
pub(crate) mod melee;
pub(crate) mod soften;
pub(crate) mod spells;
pub(crate) mod target;

use std::time::{Duration, Instant};

use self::melee::{GIVE_UP_FOR, ROAD_REACH};
use self::target::wanted_target;
#[cfg(doc)]
use crate::autoplay::hear::spell_attacker;
use crate::autoplay::{Autoplay, Fight};
use crate::Client;

/// Swung at this recently, the character is in a fight whether or not
/// it chose one.
const UNDER_ATTACK: Duration = Duration::from_secs(4);

/// Whether a fight taken on the road is over because the creature has
/// dropped out of it: the character is on its way somewhere, the
/// creature has not attacked it lately, is not walking at it or a mate,
/// nobody on the road is fighting it, and it stands `away` metres off,
/// beyond a swing's reach (`ROAD_REACH`).
///
/// On the road a character takes on only what attacks it (see
/// `Client::passing_by`), and a creature that swung once and then fell
/// behind -- the party outran it, it lost interest, it was never going
/// to keep up -- was chased until it died or until twenty seconds
/// without a hit gave it up (`STALL_AFTER`), the road forgotten
/// meanwhile. The fight ends when the creature stops following. One
/// still in reach is finished: leaving a creature at half health a
/// swing away is a chase the other way round.
pub fn road_fight_over(
    on_the_road: bool,
    attacking_us: bool,
    coming_at_us: bool,
    a_mate_is_on_it: bool,
    away: f32,
) -> bool {
    on_the_road && !attacking_us && !coming_at_us && !a_mate_is_on_it && away > ROAD_REACH
}

impl Autoplay {
    /// Something called `who` attacked the character at `now`: a blow,
    /// a miss or a spell (see [`Autoplay::hit_by`]). Each name is kept
    /// once, at its latest, and what has not attacked for a while is
    /// let go.
    pub(crate) fn attacked_by(&mut self, who: &str, now: Instant) {
        self.last_hit_us = Some(now);
        self.hit_by.expire(now, UNDER_ATTACK);
        self.hit_by.mark(who.to_string(), now);
    }
}

impl Client {
    /// The names of what has attacked this character lately (see
    /// `Autoplay::attacked_by`), in name order, one row a name.
    pub fn attackers_lately(&self) -> Vec<String> {
        let now = Instant::now();
        self.autoplay
            .hit_by
            .iter()
            .filter(|(_, when)| now.duration_since(*when) < UNDER_ATTACK)
            .map(|(who, _)| who.clone())
            .collect()
    }

    /// Something has attacked the character in the last few seconds --
    /// landed a blow, swung and missed, or cast a spell at it. A creature
    /// that keeps missing is in a fight with this character just as much
    /// as one that hits, and so is one that stands off and casts; the
    /// server says so each time.
    pub fn under_attack(&self) -> bool {
        self.autoplay
            .last_hit_us
            .is_some_and(|t| t.elapsed() < UNDER_ATTACK)
    }

    /// Whether something that has attacked the character lately is
    /// called `name`. The server names the attacker in what it sends,
    /// hit, miss or spell, so this is all there is to go on, and it is
    /// enough: it is what says a creature outside the hunting area, or
    /// one otherwise walked past, is a fight the character is already
    /// in. Two Cows, one of which kicked back, are both fought on it;
    /// the server does not say which.
    ///
    /// A creature's move target being this character is not asked here.
    /// It says something is walking at us, which our own pets, a fellow
    /// and a wandering townsfolk all do, and it carries no time, so it
    /// could not say when the fight began. The two notifications and the
    /// spell lines (see [`spell_attacker`]) say outright that an attack
    /// was aimed at this character, and when.
    pub(crate) fn hit_lately_by(&self, name: &str) -> bool {
        self.autoplay
            .hit_by
            .iter()
            .any(|(who, when)| who == name && when.elapsed() < UNDER_ATTACK)
    }

    /// Whether `o` is a creature to walk past because the character is
    /// on its way somewhere (see [`Self::on_its_way`]).
    ///
    /// Going somewhere is an errand of its own. The fighting is what the
    /// ground at the far end is for, and a character that stops for
    /// every drudge between here and there arrives an hour late or not
    /// at all. It answers what the character is *doing*, which is why it
    /// is a rule of its own rather than another clause in
    /// [`Self::a_critter`]: that one answers what a creature *is*, and
    /// the same Drudge is worth fighting once the walk is over.
    ///
    /// Two things outrank it, and in both the fight is already happening:
    /// the creature is attacking the character, with a swing or a spell,
    /// or one of the party on the road with it is fighting it (see
    /// [`Self::a_mate_on_the_road_is_on`]). The road does not get to decide
    /// either. Nothing has to be undone at the end of the road: the rule
    /// ends with the walk, and the character is fighting again the tick it
    /// arrives.
    ///
    /// `a_critter`'s other overrides are not this rule's. A name in
    /// "only these" says which kind to hunt, not where: every creature the
    /// pickers could choose already matches it, so as an override it
    /// switched the walking past off for anyone who kept the list, and a
    /// Drudge hunter stopped for every Drudge on the road to the Drudge
    /// ground. And a summoned creature picks its own fights. ACE's combat
    /// pet goes for the nearest monster it can see, whether or not that one
    /// is doing anything, so taking its target for a fight already on had
    /// the character stop for each creature its pet went for in turn --
    /// the very walk this rule is for.
    ///
    /// A creature standing in the character's path is not carved out,
    /// and that is a decision rather than an oversight. Nothing this
    /// client walks with can be stopped by one: its own physics collides
    /// with the landblock's static geometry and with nothing else, which
    /// is why a character walks straight through a closed door, and the
    /// steering plans its way round that same geometry. A creature is an
    /// object like the door is. And anything that could get in the way
    /// and matter is aggressive, which means it attacks -- the first
    /// override, and the character turns and fights it. If some ground
    /// proves otherwise, the walk's own four-minute timeout
    /// (`growth::WALK_TIMEOUT`) still ends it and another ground is
    /// chosen.
    pub(crate) fn passing_by(&self, o: &ac_world::WorldObject, cfg: &Fight) -> bool {
        if !cfg.walk_past_on_the_way || !self.on_its_way() {
            return false;
        }
        !(self.hit_lately_by(&o.name) || self.a_mate_on_the_road_is_on(o.guid))
    }

    /// Say what the character is walking past on its way somewhere: the
    /// nearest creature in reach that would be fought were the road not
    /// being walked (see [`Self::passing_by`]). Once per creature every
    /// few seconds, so a road can be read back from the log; nothing is
    /// asked when the character is not on a road.
    fn note_walked_past(&mut self, me: glam::Vec3, cfg: &Fight, underground: bool, now: Instant) {
        if !cfg.walk_past_on_the_way || !self.on_its_way() {
            return;
        }
        let mut standing = cfg.clone();
        standing.walk_past_on_the_way = false;
        let passed = self
            .world
            .objects
            .values()
            .filter(|o| self.would_fight(o, &standing, underground, now))
            .filter_map(|o| {
                let d = o.world_pos()?.distance(me);
                (d <= cfg.radius).then_some((d, o.name.clone()))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, name)) = passed {
            self.autoplay
                .note(format!("walking past {name} on the road"), now);
        }
    }

    /// Whether one of the party, on its way as well, is fighting `guid`.
    ///
    /// A character on the road takes on nothing but what attacks it, so
    /// this is a fight that came to one of its own. Without it the leader
    /// walked on while a follower turned to fight, and the follower was
    /// fetched after it and turned again, alone, each time it closed. A
    /// mate that is not on the road -- hunting back at the ground while
    /// this one goes to town -- is no reason to turn round.
    fn a_mate_on_the_road_is_on(&self, guid: u32) -> bool {
        self.autoplay
            .team
            .mates
            .iter()
            .any(|m| m.on_its_way && m.target == Some(guid))
    }

    /// Whether to go on at `guid` with the rest of the team: it is alive,
    /// and not one this character is walking past on its way somewhere
    /// (see [`Self::passing_by`]).
    ///
    /// Focus fire and the debuffer take the team's target off the board
    /// rather than choosing through [`Self::would_fight`], so they ask
    /// this instead. Without it a character setting off for town turned
    /// round for whatever the party back at the ground was hitting, from
    /// as far off as it could see it.
    pub(crate) fn joins_the_team_on(&self, guid: u32, cfg: &Fight) -> bool {
        self.world
            .objects
            .get(&guid)
            .is_some_and(|o| o.health.unwrap_or(1.0) > 0.0 && !self.passing_by(o, cfg))
    }

    /// Whether `o` is something this character would take on: a live
    /// creature, nobody's summoned pet and no player, inside the hunting
    /// area, of a kind it hunts, not a critter beneath it, not one the
    /// vitae has it keeping clear of, and not one it has given up
    /// reaching. Where it stands is the caller's business.
    ///
    /// Asked by the fight when it picks a target, and by the clock that
    /// says the ground has gone quiet (see
    /// [`Client::a_fight_in_sight`]), so the two agree about what
    /// counts as something to fight.
    pub(crate) fn would_fight(
        &self,
        o: &ac_world::WorldObject,
        cfg: &Fight,
        underground: bool,
        now: Instant,
    ) -> bool {
        o.item_type & ac_world::item_type::CREATURE != 0
            && o.object_desc_flags & ac_world::object_desc_flags::ATTACKABLE != 0
            && o.object_desc_flags & ac_world::object_desc_flags::PLAYER == 0
            && o.health.unwrap_or(1.0) > 0.0
            && !o.is_player
            // A summoned creature is its owner's, ours or anyone's.
            && o.pet_owner == 0
            // Inside the hunting area, or hitting the character.
            && self.area_allows(o, underground)
            && wanted_target(&o.name, cfg)
            // A Rabbit the character has outgrown is walked past.
            && !self.a_critter(o, cfg)
            // And so is whatever stands about while it is on its way
            // somewhere: the trip is the errand, not the road.
            && !self.passing_by(o, cfg)
            // With the vitae high, the hard ones and the killer wait.
            && !self.shy_of(o)
            // And one there is no getting to is not a fight on offer.
            && !self.autoplay.given_up.within(&o.guid, now, GIVE_UP_FOR)
    }
}

#[cfg(test)]
mod tests;
