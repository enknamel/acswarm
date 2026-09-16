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
    /// What this goal is worth when its scorer has no opinion. Written
    /// out so that adding or dropping a row moves nothing else.
    pub base: f32,
    /// Why it sits where it does. The reason the order matters, kept
    /// beside the order rather than in a comment above one branch of a
    /// chain.
    pub why: &'static str,
    /// What this is worth doing at this moment, for a goal. Higher
    /// wins. See [`Step::worth`].
    worth: fn(&Client, Instant) -> f32,
    run: fn(&mut Client, Instant) -> Did,
}

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
    pub fn worth(&self, client: &Client, now: Instant) -> f32 {
        let base = self.base;
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
        base: 190.0,
        why: "healing comes before everything, a spell in the air included: a character that dodges well and dies is no better off",
        worth: by_place,
        run: claimed!(Client::autoplay_survive),
    },
    Step {
        name: "dodge",
        layer: Layer::Reflex,
        base: 180.0,
        why: "a spell already in the air is stepped out of before anything but healing",
        worth: by_place,
        run: claimed!(Client::autoplay_dodge),
    },
    Step {
        name: "recover",
        layer: Layer::Reflex,
        base: 170.0,
        why: "dead, or on the way back from it: nothing else until the corpse is dealt with",
        worth: by_place,
        run: claimed!(Client::autoplay_recover),
    },
    Step {
        name: "academy",
        layer: Layer::Reflex,
        base: 160.0,
        why: "a new character finishes the tutorial before it is let loose on anything else",
        worth: by_place,
        run: claimed!(Client::autoplay_academy),
    },
    Step {
        name: "urgent buffs",
        layer: Layer::Reflex,
        base: 150.0,
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
        base: 140.0,
        why: "mana and stamina are kept up between everything else",
        worth: by_place,
        run: claimed!(Client::autoplay_vitals),
    },
    Step {
        name: "loot",
        layer: Layer::Goal,
        base: 130.0,
        why: "a corpse keeps for five minutes and rots; the leader and the shops do not",
        worth: worth_looting,
        run: claimed!(Client::autoplay_loot),
    },
    Step {
        name: "catch up",
        layer: Layer::Goal,
        base: 120.0,
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
        base: 110.0,
        why: "debuff the party's target, hand a teammate what it is short of, heal whoever is worst",
        worth: by_place,
        run: claimed!(Client::autoplay_team),
    },
    Step {
        name: "salvage",
        layer: Layer::Goal,
        base: 100.0,
        why: "salvage sits between fights: left alone while anything is being fought",
        worth: by_place,
        run: claimed!(Client::autoplay_salvage),
    },
    Step {
        name: "summon",
        layer: Layer::Goal,
        base: 90.0,
        worth: worth_fighting,
        why: "a summoned creature fights beside the character: called as a fight begins, and again whenever the last one is gone and an essence is ready",
        run: claimed!(Client::autoplay_summon),
    },
    Step {
        name: "fight",
        layer: Layer::Goal,
        base: 80.0,
        worth: worth_fighting,
        why: "what the character is mostly for",
        run: claimed!(Client::autoplay_fight),
    },
    Step {
        name: "keep to the area",
        layer: Layer::Goal,
        base: 70.0,
        worth: by_place,
        why: "a hunting area is where the fights are to be had: outside it, with nothing to fight, go back",
        run: claimed!(Client::autoplay_keep_to_area),
    },
    Step {
        name: "buffs",
        layer: Layer::Goal,
        base: 60.0,
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
        base: 50.0,
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
        base: 40.0,
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
        name: "explore",
        layer: Layer::Goal,
        base: 20.0,
        why: "a dungeon is described a room at a time, so a character that waits at the entrance waits in the one room that is empty; walk it before deciding the ground is dead",
        worth: by_place,
        run: claimed!(Client::autoplay_explore),
    },
    Step {
        name: "grow",
        layer: Layer::Goal,
        base: 10.0,
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
        let worth = step.worth(client, now);
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
            if step.worth(client, now) > 0.0 {
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
mod tests;
