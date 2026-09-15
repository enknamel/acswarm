//! Everything a character might do, in the order it is considered.
//!
//! This was a chain of twenty `if this(now) { return }` lines. The
//! order was doing a great deal of work -- it silently settles a
//! hundred questions, such as whether a dying character heals or flees,
//! whether loot is taken before a leader is followed, whether the pack
//! is tidied before anything decides it is full -- and none of that was
//! readable, loggable or testable, because it was control flow.
//!
//! It is a table now. The same order, the same behaviour, but the order
//! is a thing the program has rather than a thing it is: a panel can
//! show it, the log can name the step that claimed a tick, and a test
//! can assert that healing comes before hunting.
//!
//! Two kinds of step, because they are not the same sort of thing (see
//! `docs/agent.md`):
//!
//! - [`Layer::Reflex`] is what must happen before anything is decided.
//!   A spell in the air, health at forty per cent. Fixed order, no
//!   scoring, and the cost is paid by every session on every tick.
//! - [`Layer::Goal`] is what the character does with its time when
//!   nothing is on fire. These are the ones whose fixed order will
//!   become utility scoring, and they are marked so that the change can
//!   be made to exactly them.
//!
//! [`Housekeeping`] is neither: a handful of things that run every tick
//! and never claim it -- a weapon into an empty hand, a quiver
//! restocked, loose stacks poured together, a rank bought with the
//! experience earned.

use crate::did::Did;
use crate::Client;
use std::time::Instant;

/// Which sort of step this is, and so what becomes of it later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    /// Runs before anything is chosen, in this order, always.
    Reflex,
    /// What to do with the time. Ordered by hand today; scored later.
    Goal,
}

/// One thing a character might do.
pub struct Step {
    /// What it is called, in the log and in the panel.
    pub name: &'static str,
    pub layer: Layer,
    /// Why it sits where it does. The reason the order matters, kept
    /// beside the order rather than in a comment above one branch of a
    /// chain.
    pub why: &'static str,
    /// What this is worth doing at this moment, for a goal. Higher
    /// wins. See [`Step::worth`].
    worth: fn(&Client, Instant) -> f32,
    run: fn(&mut Client, Instant) -> Did,
}

/// What a goal is worth when it has no opinion of its own: its place in
/// the table, so that goals which do not score behave exactly as they
/// did when the order was all there was.
///
/// Spaced ten apart, which leaves room for a goal to say "this is worth
/// rather more than usual" without leaping the whole table.
const BY_PLACE: f32 = 10.0;

impl Step {
    pub fn run(&self, client: &mut Client, now: Instant) -> Did {
        (self.run)(client, now)
    }

    /// What this goal is worth right now, in a table where the first
    /// entry is worth most.
    ///
    /// Zero means there is nothing to do and the step is not even
    /// called, which is the one place this can be cheaper than the
    /// chain it replaces: asking "is there a corpse" is a good deal
    /// less work than the looting step deciding there is not.
    ///
    /// A goal that does not score gets its place in the table, so
    /// nothing moves until a curve is written on purpose.
    pub fn worth(&self, client: &Client, now: Instant, place: usize) -> f32 {
        let base = (STEPS.len() - place) as f32 * BY_PLACE;
        let said = (self.worth)(client, now);
        if said == UNDECIDED {
            base
        } else if said <= 0.0 {
            0.0
        } else {
            said
        }
    }
}

/// What a step's scorer returns when it has no opinion.
const UNDECIDED: f32 = f32::NEG_INFINITY;

/// The scorer for a step that has nothing to say about its own worth.
fn by_place(_: &Client, _: Instant) -> f32 {
    UNDECIDED
}

/// The scorer for a row that is no longer a goal: worth nothing, so it
/// is never weighed and never run. The row stays where it is because
/// every other goal's worth is its place in this table, and taking a
/// row out moves all of them.
fn not_weighed(_: &Client, _: Instant) -> f32 {
    0.0
}

/// What looting is worth: nothing at all when there is no body, its
/// usual place while the body will keep, and more than the next fight
/// once it will not.
///
/// This is the first thing said as a curve that was already being said
/// as an exception. `autoplay_loot` has always broken off a fight for a
/// corpse about to rot and left one alone otherwise, written as a
/// comparison buried in the middle of choosing a corpse. Here it is the
/// reason looting is chosen at all, where it can be read.
/// What fighting is worth: its place in the table when there is
/// something to hit here, less than a body on the floor when there is
/// not.
///
/// A fight you have to walk to is not worth leaving loot behind for.
/// The body is already dead, it is already yours, it rots on a clock,
/// and picking it up costs the few seconds it takes to bend down --
/// while the creature across the room will still be there afterwards
/// and is no further away for the wait. So a target that needs a walk
/// scores under [`LOOT_AT_REST`], and a body at rest outranks it; one
/// already in reach keeps its usual place and outranks the body, so a
/// character does not stop to loot with something swinging at it.
fn worth_fighting(client: &Client, _now: Instant) -> f32 {
    let Some(me) = client.player.as_ref().map(|p| p.world_position()) else {
        return UNDECIDED;
    };
    // Already swinging at something that is still alive: that is a
    // fight in hand whatever the distance, and it is not interrupted.
    //
    // Still *alive* is the whole of it. `attack_target` holds the
    // creature until another is chosen, so the moment after a kill it
    // names a corpse -- and reading that as "in a fight" sent the
    // character off to the next creature every time, past the body it
    // had just made. Which is exactly the complaint: it never loots if
    // there is anything at all nearby.
    let engaged = client
        .attack_target
        .and_then(|t| client.world.objects.get(&t))
        .is_some_and(|o| o.health.unwrap_or(0.0) > 0.0);
    if engaged {
        return UNDECIDED;
    }
    // Something is hitting the character: that fight has come to it,
    // and it is fought before anything is picked up.
    if client.under_attack() {
        return UNDECIDED;
    }
    // Told to finish what it kills: while one of its own bodies is
    // still unlooted -- or about to appear -- another fight can wait.
    if client.waits_for_a_corpse() {
        return fight_worth(false, true, 0.0);
    }
    // A critter at the character's feet is no fight in reach: the fight
    // rules will walk past it, and reading it as one held the loot back.
    // Nor is one the character is walking past on its way somewhere, for
    // the same reason -- it is not going to be fought either.
    let fight = &client.autoplay.config.fight;
    let nearest = client
        .world
        .objects
        .values()
        .filter(|o| o.item_type & ac_world::item_type::CREATURE != 0)
        .filter(|o| o.health.unwrap_or(0.0) > 0.0)
        .filter(|o| !client.a_critter(o, fight))
        .filter(|o| !client.passing_by(o, fight))
        .filter_map(|o| o.world_pos())
        .map(|at| at.distance(me))
        .fold(f32::MAX, f32::min);
    fight_worth(false, false, nearest)
}

/// The same judgement with the world left out, so it can be argued
/// about on its own.
///
/// Every one of the three arguments has been got wrong in a live run:
/// `engaged` read from a target that had already died, which sent the
/// character past the body it had just made; `owes_a_body` not read at
/// all, so bodies piled up; and `nearest` not read at all, so a fight
/// across the room outranked loot at the character's feet.
fn fight_worth(engaged: bool, owes_a_body: bool, nearest: f32) -> f32 {
    if engaged {
        return UNDECIDED;
    }
    if owes_a_body || nearest > IN_REACH_OF_A_FIGHT {
        WALK_TO_A_FIGHT
    } else {
        UNDECIDED
    }
}

fn worth_looting(client: &Client, now: Instant) -> f32 {
    use crate::autoplay::CORPSE_LIFE;
    let room = client.room_for_loot();
    let Some(me) = client.player.as_ref().map(|p| p.world_position()) else {
        return 0.0;
    };
    // The body in hand, or the one being walked to: whatever the board
    // says about it now, this character has it open on the server or is
    // most of the way there, and only the looting closes it. Scoring it
    // out would stop the looting on a held body and leave it open.
    let in_hand = client.autoplay.corpse_claim(now).map(|(guid, _)| guid);
    let waiting = client
        .world
        .objects
        .values()
        // Exactly the bodies the looting would go to (see
        // [`Client::corpse_for_us`]). A body the looting has finished
        // with, set aside, out of reach, or claimed by a teammate is not
        // one it will go to, and counting those ranked looting above the
        // next fight with nothing for it to do: nine characters stopped
        // fighting for a floor of bodies that were all somebody else's.
        .filter(|o| in_hand == Some(o.guid) || client.corpse_for_us(o, me, now, room))
        .map(|o| {
            // A corpse we never saw appear is taken as fresh, which is
            // what the looting step assumes too.
            let seen = client
                .autoplay
                .corpse_seen
                .iter()
                .find(|(g, _)| *g == o.guid)
                .map(|(_, t)| *t)
                .unwrap_or(now);
            CORPSE_LIFE.saturating_sub(now.duration_since(seen))
        });
    worth_of_bodies(waiting)
}

/// What looting is worth with bodies waiting that each have `left` of
/// their lives: nothing with none, and otherwise more the older the
/// oldest is and the more of them there are.
fn worth_of_bodies(left: impl Iterator<Item = std::time::Duration>) -> f32 {
    use crate::autoplay::{CORPSE_LIFE, CORPSE_URGENT};
    let mut best: Option<std::time::Duration> = None;
    let mut waiting = 0usize;
    for l in left {
        best = Some(best.map_or(l, |b| b.min(l)));
        waiting += 1;
    }
    let Some(left) = best else {
        // No body: do not even ask the looting step, which is the one
        // place weighing is cheaper than the chain it replaces.
        return 0.0;
    };
    if left <= CORPSE_URGENT {
        // About to go. Nothing outranks this: there is no second
        // chance at it, and the fight will still be there afterwards.
        return WORTH_A_LOT;
    }
    // Otherwise it grows as the body ages and as bodies pile up.
    //
    // A character that only breaks off for a corpse about to rot never
    // goes back for the older ones: in a busy dungeon there is always
    // another fight, and the pile behind it is never cleared. So a body
    // is worth a little more every second it waits, and each one still
    // on the floor adds to it. Somewhere past the first third of its
    // life a body outranks starting another fight, which is the
    // behaviour worth having and was impossible to say when the order
    // was fixed.
    let aged = 1.0 - left.as_secs_f32() / CORPSE_LIFE.as_secs_f32();
    let waiting = waiting.min(WAITING_COUNTS) as f32;
    LOOT_AT_REST + aged * AGE_IS_WORTH + waiting * EACH_BODY_IS_WORTH
}

/// What looting is worth with a body just dropped and nothing else
/// waiting: below fighting, because the fight is what made the body.
const LOOT_AT_REST: f32 = 45.0;
/// How much a body's whole life is worth, spread across it. Enough that
/// a body past the first third of its life outranks fighting.
const AGE_IS_WORTH: f32 = 90.0;
/// What each body still on the floor adds.
const EACH_BODY_IS_WORTH: f32 = 12.0;
/// Past this many, another body says nothing new.
const WAITING_COUNTS: usize = 6;

/// More than any goal gets from its place in the table. For the few
/// things that genuinely outrank everything else for a moment.
const WORTH_A_LOT: f32 = 1_000.0;
/// Near enough that a fight is happening rather than being gone to.
const IN_REACH_OF_A_FIGHT: f32 = 12.0;
/// What a fight worth walking to scores: under a body at rest, so the
/// floor is cleared before the character sets off.
const WALK_TO_A_FIGHT: f32 = LOOT_AT_REST - 5.0;

/// Wrap one of the old `-> bool` steps: true meant it claimed the tick.
///
/// Every step will say more than this in time (see `crate::did`); until
/// it does, "it acted" and "it did not" is exactly what the old chain
/// knew, so nothing is lost or invented in the meantime.
macro_rules! claimed {
    ($f:expr) => {
        |client: &mut Client, now: Instant| {
            if $f(client, now) {
                Did::Acting
            } else {
                Did::Done
            }
        }
    };
}

/// The order. Reading down this table is reading what the character
/// cares about, most pressing first.
pub const STEPS: &[Step] = &[
    Step {
        name: "survive",
        layer: Layer::Reflex,
        why: "healing comes before everything, a spell in the air included: a character that dodges well and dies is no better off",
        worth: by_place,
        run: claimed!(Client::autoplay_survive),
    },
    Step {
        name: "dodge",
        layer: Layer::Reflex,
        why: "a spell already in the air is stepped out of before anything but healing",
        worth: by_place,
        run: claimed!(Client::autoplay_dodge),
    },
    Step {
        name: "recover",
        layer: Layer::Reflex,
        why: "dead, or on the way back from it: nothing else until the corpse is dealt with",
        worth: by_place,
        run: claimed!(Client::autoplay_recover),
    },
    Step {
        name: "academy",
        layer: Layer::Reflex,
        why: "a new character finishes the tutorial before it is let loose on anything else",
        worth: by_place,
        run: claimed!(Client::autoplay_academy),
    },
    Step {
        name: "urgent buffs",
        layer: Layer::Reflex,
        why: "a buff about to lapse goes back up before anything else, fight or no fight",
        worth: by_place,
        run: |c, now| {
            if c.autoplay_buff(now, true) {
                Did::Acting
            } else {
                Did::Done
            }
        },
    },
    Step {
        name: "vitals",
        layer: Layer::Reflex,
        why: "mana and stamina are kept up between everything else",
        worth: by_place,
        run: claimed!(Client::autoplay_vitals),
    },
    Step {
        name: "loot",
        layer: Layer::Goal,
        why: "a corpse keeps for five minutes and rots; the leader and the shops do not",
        worth: worth_looting,
        run: claimed!(Client::autoplay_loot),
    },
    Step {
        name: "catch up",
        layer: Layer::Goal,
        why: "a leader that has got well away is caught up with before anything else is considered",
        worth: by_place,
        run: |c, now| {
            if c.autoplay_follow(now, true) {
                Did::Acting
            } else {
                Did::Done
            }
        },
    },
    Step {
        name: "team",
        layer: Layer::Goal,
        why: "debuff the party's target, hand a teammate what it is short of, heal whoever is worst",
        worth: by_place,
        run: claimed!(Client::autoplay_team),
    },
    Step {
        name: "salvage",
        layer: Layer::Goal,
        why: "salvage sits between fights: left alone while anything is being fought",
        worth: by_place,
        run: claimed!(Client::autoplay_salvage),
    },
    Step {
        name: "summon",
        layer: Layer::Goal,
        worth: worth_fighting,
        why: "a summoned creature fights beside the character: called as a fight begins, and again whenever the last one is gone and an essence is ready",
        run: claimed!(Client::autoplay_summon),
    },
    Step {
        name: "fight",
        layer: Layer::Goal,
        worth: worth_fighting,
        why: "what the character is mostly for",
        run: claimed!(Client::autoplay_fight),
    },
    Step {
        name: "keep to the area",
        layer: Layer::Goal,
        worth: by_place,
        why: "a hunting area is where the fights are to be had: outside it, with nothing to fight, go back",
        run: claimed!(Client::autoplay_keep_to_area),
    },
    Step {
        name: "buffs",
        layer: Layer::Goal,
        why: "the buffs that were not urgent, once the fighting is done",
        worth: by_place,
        run: |c, now| {
            if c.autoplay_buff(now, false) {
                Did::Acting
            } else {
                Did::Done
            }
        },
    },
    Step {
        name: "follow",
        layer: Layer::Goal,
        why: "a leader near at hand is followed once the fighting is done",
        worth: by_place,
        run: |c, now| {
            if c.autoplay_follow(now, false) {
                Did::Acting
            } else {
                Did::Done
            }
        },
    },
    Step {
        name: "resume the journey",
        layer: Layer::Goal,
        why: "a walk broken off by a fight is picked up again",
        worth: by_place,
        run: |c, _| {
            if c.autoplay_resume_journey() {
                Did::Acting
            } else {
                Did::Done
            }
        },
    },
    Step {
        name: "tidy",
        layer: Layer::Goal,
        why: "holds tidy's old place so no goal's worth moves; the pack is tidied every tick as housekeeping, which needs no tick of its own",
        // Never weighed, so never run: every other goal's worth is its
        // place in this table, and taking the row out would move six of
        // them against the fixed scores that some goals do give.
        worth: not_weighed,
        run: |_, _| Did::Done,
    },
    Step {
        name: "explore",
        layer: Layer::Goal,
        why: "a dungeon is described a room at a time, so a character that waits at the entrance waits in the one room that is empty; walk it before deciding the ground is dead",
        worth: by_place,
        run: claimed!(Client::autoplay_explore),
    },
    Step {
        name: "grow",
        layer: Layer::Goal,
        why: "with nothing else to do: find monsters, run to town; experience is spent as housekeeping, since this is reached only on a tick nothing else wants",
        worth: by_place,
        run: claimed!(Client::autoplay_grow),
    },
];

/// The things that run every tick and never claim it: they put a
/// weapon in an empty hand, raise a shield, restock a quiver from the
/// pack. None of them is a reason to stop considering anything else.
pub struct Housekeeping {
    pub name: &'static str,
    run: fn(&mut Client, Instant),
}

impl Housekeeping {
    pub fn run(&self, client: &mut Client, now: Instant) {
        (self.run)(client, now)
    }
}

pub const HOUSEKEEPING: &[Housekeeping] = &[
    Housekeeping {
        name: "take up a weapon",
        run: Client::autoplay_pending_wield,
    },
    Housekeeping {
        name: "shield",
        run: Client::autoplay_shield,
    },
    Housekeeping {
        name: "rearm",
        run: |c, _| c.autoplay_rearm(),
    },
    Housekeeping {
        name: "restock from the pack",
        run: |c, _| c.autoplay_stock(),
    },
    Housekeeping {
        name: "claim the summoned creature's kills",
        run: Client::autoplay_claim_pet_kills,
    },
    Housekeeping {
        name: "tidy the pack",
        // A pour of one carried stack into another is made on the spot:
        // no walk, no animation, nothing the server calls being busy.
        // So it costs no tick, and as a goal it never got one -- a
        // character that fights, loots and walks all afternoon is never
        // idle, and the pack filled with part stacks while tidying
        // waited its turn.
        run: Client::autoplay_tidy,
    },
    Housekeeping {
        name: "spend experience",
        // A rank is one message the server takes on the spot, fight or
        // no fight. It was the first thing the last goal did, and the
        // last goal is only reached when nothing else wants the tick:
        // exploring always had another room to walk to, and a character
        // given a hundred billion experience spent none of it.
        run: Client::autoplay_spend_xp,
    },
];

/// The reflexes, in their fixed order.
pub fn reflexes() -> impl Iterator<Item = &'static Step> {
    STEPS.iter().filter(|s| s.layer == Layer::Reflex)
}

/// The goals, worth most first.
///
/// Scoring happens once and the order is settled there; a goal that is
/// worth nothing is not scored again and not run at all. The list is
/// small and lives on the stack, because this runs for every session on
/// every tick.
pub fn weigh(client: &Client, now: Instant) -> Weighed {
    let mut order: Vec<(usize, f32)> = Vec::with_capacity(STEPS.len());
    for (place, step) in STEPS.iter().enumerate() {
        if step.layer != Layer::Goal {
            continue;
        }
        let worth = step.worth(client, now, place);
        if worth > 0.0 {
            order.push((place, worth));
        }
    }
    // Worth most first; ties keep their place in the table, which is
    // what makes a goal with no opinion behave exactly as it did.
    order.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Weighed { order, next: 0 }
}

/// The goals of one tick, in the order they were judged worth doing.
pub struct Weighed {
    order: Vec<(usize, f32)>,
    next: usize,
}

impl Weighed {
    /// The next goal to try, and what it was worth.
    ///
    /// Takes the client because a goal that has become worthless since
    /// the weighing -- the corpse rotted while the fight was being had
    /// -- should not be run on a stale score.
    pub fn next(&mut self, client: &Client, now: Instant) -> Option<(&'static Step, f32)> {
        while let Some(&(place, worth)) = self.order.get(self.next) {
            self.next += 1;
            let step = &STEPS[place];
            if step.worth(client, now, place) > 0.0 {
                return Some((step, worth));
            }
        }
        None
    }

    /// What was weighed, worth most first, for the log and the panel.
    pub fn ranking(&self) -> Vec<(&'static str, f32)> {
        self.order
            .iter()
            .map(|&(place, worth)| (STEPS[place].name, worth))
            .collect()
    }
}

/// The step of this name, for a log line or a panel row.
pub fn named(name: &str) -> Option<&'static Step> {
    STEPS.iter().find(|s| s.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_order_says_what_the_character_cares_about() {
        let at = |name: &str| {
            STEPS
                .iter()
                .position(|s| s.name == name)
                .unwrap_or_else(|| panic!("no step called {name}"))
        };

        // Staying alive comes before everything. These are the ones
        // that cost a character its life when they are wrong.
        // Healing outranks everything, a spell already in the air
        // included. Stepping out of the way is worth little to a
        // character that dies while doing it.
        assert!(at("survive") < at("dodge"), "heal before stepping out");
        // A body on the floor beats a fight that has to be walked to:
        // it is already dead, already ours, and rotting on a clock,
        // while the creature across the room will still be there.
        // A body on the floor beats a fight that has to be walked to.
        // Held as a constant rather than an assertion because it is one:
        // the two numbers are fixed, and the compiler checks it for
        // free the moment either changes.
        const _: () = assert!(WALK_TO_A_FIGHT < LOOT_AT_REST);
        assert_eq!(at("survive"), 0, "nothing comes before staying alive");
        assert!(at("survive") < at("fight"), "heal before fighting");
        assert!(at("survive") < at("loot"), "heal before looting");
        assert!(at("recover") < at("fight"), "deal with the corpse first");

        // A buff about to lapse goes up before the fight; the rest wait
        // until after it.
        assert!(at("urgent buffs") < at("fight"));
        assert!(at("fight") < at("buffs"));
        assert!(at("summon") < at("fight"), "the creature joins the fight");
        assert!(
            at("fight") < at("keep to the area"),
            "defend first, then go back"
        );
        assert!(at("keep to the area") < at("explore"));
        assert!(at("keep to the area") < at("grow"));

        // A corpse rots and a shop does not.
        assert!(at("loot") < at("grow"), "loot before shopping");
        assert!(at("loot") < at("salvage"));

        // Tidying comes before anything that decides the pack is full.
        assert!(at("tidy") < at("grow"));

        // And the last word is the one that finds something to do.
        assert_eq!(STEPS.last().map(|s| s.name), Some("grow"));
    }

    #[test]
    fn the_pack_is_tidied_as_housekeeping_and_never_claims_a_tick() {
        // Pouring one carried stack into another is made by the server
        // on the spot, so it costs no tick -- and as a goal it never
        // won one, because a character that fights, loots and walks has
        // no quiet tick to give it.
        assert!(
            HOUSEKEEPING.iter().any(|h| h.name == "tidy the pack"),
            "tidying is housekeeping now"
        );
        let tidy = named("tidy").expect("the row is still there");
        assert!(
            std::ptr::fn_addr_eq(tidy.worth, not_weighed as fn(&Client, Instant) -> f32),
            "the row is a placeholder, not a goal"
        );
        let at = |name: &str| STEPS.iter().position(|s| s.name == name).expect(name);
        assert!(at("tidy") < at("grow"), "and it keeps its old place");
    }

    #[test]
    fn experience_is_spent_as_housekeeping_and_never_claims_a_tick() {
        // A rank is one message the server takes on the spot. As the
        // first thing the last goal did, it waited for a tick nothing
        // else wanted, and exploring always wanted it: a character given
        // a hundred billion experience spent none of it.
        assert!(
            HOUSEKEEPING.iter().any(|h| h.name == "spend experience"),
            "spending experience is housekeeping now"
        );
        let grow = named("grow").expect("growing is still a goal");
        assert!(
            !grow
                .why
                .starts_with("with nothing else to do: spend experience"),
            "and the goal no longer claims to spend it"
        );
    }

    #[test]
    fn every_goal_keeps_the_worth_it_had_when_tidying_moved() {
        // The reason the row stays: every goal with no opinion is worth
        // its place in this table, and six of them are weighed against
        // fixed scores. Taking a row out shifts all of them.
        let base = |name: &str| {
            let place = STEPS.iter().position(|s| s.name == name).expect(name);
            (STEPS.len() - place) as f32 * BY_PLACE
        };
        assert_eq!(STEPS.len(), 19);
        assert_eq!(base("fight"), 80.0);
        assert_eq!(base("summon"), 90.0);
        assert_eq!(base("keep to the area"), 70.0);
        assert_eq!(base("buffs"), 60.0);
        assert_eq!(base("follow"), 50.0);
        assert_eq!(base("resume the journey"), 40.0);
    }

    #[test]
    fn every_step_says_what_it_is_and_why_it_is_there() {
        assert!(!STEPS.is_empty());
        for s in STEPS {
            assert!(!s.name.is_empty());
            assert!(
                s.why.len() > 20,
                "{} should say why it sits where it does",
                s.name
            );
        }
        // Names are unique, or naming one in a log is ambiguous.
        let mut names: Vec<&str> = STEPS.iter().map(|s| s.name).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "two steps share a name");
        assert!(named("fight").is_some());
        assert!(named("nonsense").is_none());
    }

    #[test]
    fn a_goal_with_no_opinion_keeps_its_place() {
        // The whole safety of the change: scoring must reproduce the
        // table's order exactly until a curve is written on purpose.
        let places: Vec<usize> = STEPS
            .iter()
            .enumerate()
            .filter(|(_, s)| s.layer == Layer::Goal)
            .map(|(i, _)| i)
            .collect();
        let base = |place: usize| (STEPS.len() - place) as f32 * BY_PLACE;
        for pair in places.windows(2) {
            assert!(
                base(pair[0]) > base(pair[1]),
                "{} should outrank {}",
                STEPS[pair[0]].name,
                STEPS[pair[1]].name
            );
        }
        // Every place is worth something, so a goal with no opinion is
        // always considered rather than skipped.
        assert!(places.iter().all(|&p| base(p) > 0.0));
        // And a thing worth a lot beats every place in the table.
        assert!(WORTH_A_LOT > base(0));
    }

    #[test]
    fn a_body_gets_more_worth_going_back_for_the_longer_it_waits() {
        let base = |name: &str| {
            let place = STEPS.iter().position(|s| s.name == name).expect(name);
            (STEPS.len() - place) as f32 * BY_PLACE
        };
        let fight = base("fight");
        // The arithmetic of the curve, without a world to hang it on.
        let worth = |left_secs: f32, waiting: usize| {
            let aged = 1.0 - left_secs / crate::autoplay::CORPSE_LIFE.as_secs_f32();
            LOOT_AT_REST
                + aged * AGE_IS_WORTH
                + waiting.min(WAITING_COUNTS) as f32 * EACH_BODY_IS_WORTH
        };

        // Just dropped, nothing else waiting: the fight that made it
        // comes first.
        assert!(worth(300.0, 1) < fight, "a fresh body waits its turn");
        // A third of its life gone: worth going back for before
        // starting another fight. This is the point of the curve -- a
        // character that only breaks off for a rotting corpse never
        // clears the pile behind it.
        assert!(worth(200.0, 1) > fight, "an ageing body comes first");
        // And bodies piling up says the same thing sooner.
        assert!(
            worth(280.0, 5) > worth(280.0, 1),
            "a floor full of bodies is worth more than one"
        );
        assert!(worth(280.0, 6) > fight, "a pile outranks the next fight");
        // It never runs away with itself: even a heap of old bodies
        // does not outrank a corpse that is actually about to go.
        assert!(worth(80.0, 6) < WORTH_A_LOT);
    }

    #[test]
    fn a_body_about_to_rot_outranks_the_next_fight() {
        // Written as a curve rather than as an exception buried in the
        // middle of choosing a corpse: a fight will still be there
        // afterwards and the body will not.
        let at = |name: &str| STEPS.iter().position(|s| s.name == name).expect(name);
        let base = |place: usize| (STEPS.len() - place) as f32 * BY_PLACE;
        assert!(
            base(at("loot")) < WORTH_A_LOT,
            "an urgent corpse must be able to outrank its own place"
        );
        assert!(WORTH_A_LOT > base(at("fight")), "and to outrank the fight");
        // While a body will keep, looting stays where it always was:
        // after the reflexes and before the shops.
        assert!(base(at("loot")) > base(at("grow")));
    }

    #[test]
    fn the_reflexes_come_first_and_are_few() {
        // Once the goals start, no reflex follows: a reflex that ran
        // after a goal had claimed the tick would never run at all.
        let first_goal = STEPS
            .iter()
            .position(|s| s.layer == Layer::Goal)
            .expect("some goals");
        assert!(
            STEPS[first_goal..].iter().all(|s| s.layer == Layer::Goal),
            "a reflex is sitting below a goal"
        );
        // They are the per-tick cost every session pays, so there
        // should not be many.
        let reflexes = STEPS.iter().filter(|s| s.layer == Layer::Reflex).count();
        assert!(reflexes <= 8, "{reflexes} reflexes is a lot to pay a tick");
    }

    #[test]
    fn a_fight_in_hand_is_never_interrupted() {
        // Swinging at something still alive outranks everything below
        // it, whatever is on the floor.
        assert_eq!(fight_worth(true, true, 100.0), UNDECIDED);
    }

    #[test]
    fn a_body_it_made_comes_before_the_next_fight() {
        // The option: finish what you kill. Even with something to hit
        // right here, the body goes first.
        assert!(fight_worth(false, true, 0.0) < LOOT_AT_REST);
    }

    #[test]
    fn a_fight_across_the_room_does_not_beat_loot_at_your_feet() {
        assert!(fight_worth(false, false, IN_REACH_OF_A_FIGHT + 1.0) < LOOT_AT_REST);
    }

    #[test]
    fn a_fight_in_reach_still_beats_a_resting_body() {
        // Nothing stops to loot with something swinging at it.
        assert_eq!(fight_worth(false, false, 1.0), UNDECIDED);
    }

    #[test]
    fn a_body_emptied_or_set_aside_does_not_make_looting_worth_more() {
        // Every body on the floor counted as one waiting to be looted:
        // those the looting had finished with, those set aside, and the
        // ones the server had let go that Blargerton kept in the Holtburg
        // Dungeon.
        use crate::autoplay::{Autoplay, Room, CORPSE_LIFE};
        use crate::did::Did;
        let now = Instant::now();
        let mut ap = Autoplay::default();
        let (emptied, locked, fresh) = (0x8000_5001, 0x8000_5002, 0x8000_5003);
        ap.looted.push(emptied, now);
        ap.shelved
            .note(locked, &Did::blocked("it will not open yet"), now);
        let room = Room::PLENTY;
        let waiting: Vec<u32> = [emptied, locked, fresh]
            .into_iter()
            .filter(|g| ap.corpse_waiting(*g, now, room))
            .collect();
        assert_eq!(waiting, vec![fresh]);
        // So the floor is worth one fresh body, not three.
        let one = worth_of_bodies([CORPSE_LIFE].into_iter());
        assert_eq!(one, LOOT_AT_REST + EACH_BODY_IS_WORTH);
        assert!(one < worth_of_bodies([CORPSE_LIFE; 3].into_iter()));
        // Nothing waiting is worth nothing.
        assert_eq!(worth_of_bodies(std::iter::empty()), 0.0);
        // The one set aside waits again once its wait is up.
        let later = now + std::time::Duration::from_secs(60);
        assert!(ap.corpse_waiting(locked, later, room));
        // And none waits on a pack with no room to take anything, so the
        // looting is worth nothing and the town run is not held up.
        let full = Room {
            pack_low: true,
            ..room
        };
        assert!(!ap.corpse_waiting(fresh, now, full));
    }

    #[test]
    #[ignore = "needs AC_DATA_DIR"]
    fn looting_is_worth_only_the_bodies_this_character_would_go_to() {
        // The two predicates disagreed: what looting was worth counted
        // every body waiting, ownership and all, while the looting step
        // and the fight both counted only the ones this character was
        // owed. Eight bodies another character had locked lifted the
        // loot score past the fight in hand, and the looting then won
        // the tick with nothing to go to. Nine characters spent 53% of
        // a run standing over bodies and 13% fighting.
        let holtburg = 0xA9B4_0019;
        let mut c = crate::testkit::standing_at(holtburg, glam::Vec3::new(84.0, 84.0, 94.0));
        c.world.player_guid = Some(0x5000_0009);
        let me = c.player.as_ref().unwrap().world_position();
        let now = Instant::now();
        let body = |guid: u32, at: glam::Vec3| ac_world::WorldObject {
            position: crate::testkit::placed(holtburg, at),
            ..crate::testkit::corpse(guid, "Corpse of Drudge Slave")
        };
        // One at the character's feet, and five more: three beside it
        // that teammates have claimed, and two lying out of reach.
        let mine = 0x8000_0001;
        c.world.objects.insert(mine, body(mine, me));
        let claimed: Vec<u32> = (2..5).map(|i| 0x8000_0000 + i).collect();
        for (n, &guid) in claimed.iter().enumerate() {
            let at = me + glam::Vec3::new(n as f32 + 1.0, 0.0, 0.0);
            c.world.objects.insert(guid, body(guid, at));
            c.autoplay.team.mates.push(crate::autoplay::Mate {
                autoplay: true,
                health: 1.0,
                world: at,
                looting: Some(guid),
                looting_for: std::time::Duration::ZERO,
                ..crate::testkit::mate(0x5000_0001 + n as u32, &format!("Mate {n}"))
            });
        }
        for i in 5..7u32 {
            let guid = 0x8000_0000 + i;
            let far = me + glam::Vec3::new(0.0, 60.0, 0.0);
            c.world.objects.insert(guid, body(guid, far));
        }
        // All six are waiting to be emptied; only one is this
        // character's to go to.
        let room = c.room_for_loot();
        assert_eq!(
            c.world
                .objects
                .values()
                .filter(|o| c.autoplay.corpse_waiting(o.guid, now, room))
                .count(),
            6
        );
        assert_eq!(
            c.world
                .objects
                .values()
                .filter(|o| c.corpse_for_us(o, me, now, room))
                .map(|o| o.guid)
                .collect::<Vec<_>>(),
            vec![mine]
        );
        // So the score is one body's, which is under the fight's.
        let one = worth_of_bodies([crate::autoplay::CORPSE_LIFE].into_iter());
        assert_eq!(worth_looting(&c, now), one);
        let fight = STEPS
            .iter()
            .position(|s| s.name == "fight")
            .map(|p| (STEPS.len() - p) as f32 * BY_PLACE)
            .expect("the fight is in the table");
        assert!(
            worth_looting(&c, now) < fight,
            "a body somebody else is opening outranked the fight"
        );

        // The body in hand is the looting's whatever the board says: a
        // teammate that claimed it too must not leave this character
        // holding one open on the server with no step left to close it.
        c.autoplay.team.mates.push(crate::autoplay::Mate {
            guid: 0x5000_0008,
            name: "Racer".into(),
            autoplay: true,
            health: 1.0,
            world: me,
            looting: Some(mine),
            looting_for: std::time::Duration::ZERO,
            ..Default::default()
        });
        assert!(!c.corpse_for_us(&body(mine, me), me, now, room));
        assert_eq!(worth_looting(&c, now), 0.0, "nothing to go to");
        c.autoplay
            .take_up_corpse(mine, now, std::time::Duration::from_secs(10));
        assert_eq!(worth_looting(&c, now), one, "let go of a body in hand");
    }
}
