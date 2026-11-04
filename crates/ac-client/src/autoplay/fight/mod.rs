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
/// On the road a character picks no fight (see `Client::passing_by`); one in hand as the road
/// begins, of a creature that swung once and then fell
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

    /// Whether `o` is a creature to walk past because the character is on its way somewhere (see
    /// [`Self::on_its_way`]): every one, attacking or not. On the road it runs and heals; a fight
    /// there only made it late or dead (test: a_creature_hitting_the_character_on_the_road_is_run_past).
    pub(crate) fn passing_by(&self, _o: &ac_world::WorldObject, cfg: &Fight) -> bool {
        cfg.walk_past_on_the_way && self.on_its_way()
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
    /// The debuffer and the assist played by hand take the team's target
    /// off the board rather than choosing through [`Self::would_fight`],
    /// so they ask this instead; the fight asks [`Self::can_take_on`].
    /// Without it a character setting off for town turned round for
    /// whatever the party back at the ground was hitting, from as far off
    /// as it could see it.
    pub(crate) fn joins_the_team_on(&self, guid: u32, cfg: &Fight) -> bool {
        self.world
            .objects
            .get(&guid)
            .is_some_and(|o| o.alive_or_unknown() && !self.passing_by(o, cfg))
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
        // With the vitae high, the hard ones and the killer wait.
        self.would_join(o, cfg, underground, now) && !self.shy_of(o)
    }

    /// [`Self::would_fight`] less the vitae (`shy_of`), which is for the fights a character picks: a
    /// hard fight is the whole party's (`plan.rs`), and its softener is waited for (`soften.rs`).
    fn would_join(
        &self,
        o: &ac_world::WorldObject,
        cfg: &Fight,
        underground: bool,
        now: Instant,
    ) -> bool {
        o.item_type & ac_world::item_type::CREATURE != 0
            && o.object_desc_flags & ac_world::object_desc_flags::ATTACKABLE != 0
            && o.object_desc_flags & ac_world::object_desc_flags::PLAYER == 0
            && o.alive_or_unknown()
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
            // And one there is no getting to is not a fight on offer, till it comes and attacks.
            && (!self.autoplay.given_up.within(&o.guid, now, GIVE_UP_FOR)
                || self.is_attacking_in_reach(o))
    }

    /// Whether `o` has come to the character: of a name that attacked it lately, and within a
    /// swing's reach (`STICKY_REACH`, 3D), which a refused walk's goal up a wall or a ledge is not.
    fn is_attacking_in_reach(&self, o: &ac_world::WorldObject) -> bool {
        let near = |me: glam::Vec3| {
            o.world_pos()
                .is_some_and(|p| p.distance(me) <= crate::dodge::STICKY_REACH)
        };
        self.hit_lately_by(&o.name) && self.my_position().is_some_and(near)
    }

    /// [`Self::would_join`] asked of `guid`: the one gate a target the team proposes -- the leader's
    /// order or its target on the board -- passes before a fight takes it, so neither goes round it.
    pub(crate) fn can_take_on(
        &self,
        guid: u32,
        cfg: &Fight,
        underground: bool,
        now: Instant,
    ) -> bool {
        self.world
            .objects
            .get(&guid)
            .is_some_and(|o| self.would_join(o, cfg, underground, now))
    }
}

#[cfg(test)]
mod tests;
