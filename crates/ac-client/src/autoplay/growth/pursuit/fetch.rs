//! A pursued item fetched, not hunted: bought on a town run through its buy line in the Quests
//! section (`section`), picked up off the floor, or taken out of a chest, a locked one opened with
//! its key first (UseWithTarget: `Key.cs:54-79`, `Lock.cs:119-193`), then emptied by the loot rules
//! as a corpse is, or made by using a carried item on a placed object (`RecipeManager.cs:43-145`).
//! A refused act drops that source for [`PURSUIT_RETRY`]; the next is worked.

use std::time::{Duration, Instant};

use ac_world::quests::{Source, Want, Yield};
use ac_world::Place;

use super::{Ask, PursuitLine, PursuitStep, PURSUIT_RETRY};
use crate::autoplay::hands::weapon::STANCE_CHANGE;
use crate::autoplay::Doing;
use crate::Client;

/// The walk to what is fetched ends this near it (metres, 3D): inside the server's use reach.
const FETCH_STOP: f32 = 1.5;
/// Further than this from it, the walk goes on (metres, 3D): a visit's talking reach.
const FETCH_REACH: f32 = crate::visit::REACH;
/// What is fetched stands this near its spot in the data (metres): a generator scatters a little.
const FOUND_NEAR: f32 = 20.0;
/// An act's answer is waited for this long before it counts as refused (guess).
pub(super) const ANSWER_WAIT: Duration = Duration::from_secs(5);
/// A take, key or item use the server never carried out (movement cancelled its walk:
/// `Player_Use.cs:157-163`, `Player_Inventory.cs:1014-1018`) is sent this many times.
pub(super) const UNLOCK_TRIES: u32 = 3;
/// A chest unlocked stays so this long: ACE resets it ResetInterval s on (weenie 27890: 20).
pub(crate) const UNLOCKED_FOR: Duration = Duration::from_secs(20);
/// ACE's LockAlreadyUnlocked: the chest opens all the same (`Lock.cs:174-176`).
const LOCK_ALREADY_UNLOCKED: u32 = 0x04AF;
/// ACE's YoureTooBusy: the key use turned away for an act of ours (`Player_Use.cs:152-156`).
const YOURE_TOO_BUSY: u32 = 0x001D;

/// What the pursued fetch last did and when, for its answer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Fetching {
    /// The wanted item and the source's weenie class: what a refusal drops.
    pub(crate) key: (u32, u32),
    /// The object acted on; 0 while it is looked for.
    pub(crate) guid: u32,
    pub(crate) act: Act,
    pub(crate) at: Instant,
    /// The key's or used item's weenie class: a key gone from the pack unlocked (`Lock.cs:69-90`).
    pub(crate) with: u32,
    /// How many times the key or item has been sent.
    pub(crate) tries: u32,
}

impl Fetching {
    /// Whether its act brings an item, still to come: a take, a talk, a key, a use or a chest.
    pub(crate) fn is_bringing(&self) -> bool {
        !matches!(self.act, Act::Look | Act::Peace)
    }
}

/// One act of a fetch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Act {
    /// Standing at the spot, waiting for the thing to be sent.
    Look,
    /// Picking it up.
    Take,
    /// The key used on the chest.
    Unlock,
    /// The chest opened for the loot rules to empty.
    Open,
    /// The NPC used, who hands the item over on a use.
    Talk,
    /// Out of combat for a use, refused in any stance but peace (`RecipeManager.cs:51-57`).
    Peace,
    /// A carried item used on the object, which makes the wanted item.
    Use,
}

/// How an act was answered.
enum Heard {
    Waiting,
    Done,
    Refused(String),
    /// No answer: sent again.
    Again,
}

impl Ask<'_> {
    /// The nearest fetched source of `wants`: a shop, the floor, a chest open or with its key
    /// carried, an object with its item carried. Missing keys or items go in `locked`, got first.
    pub(super) fn fetch_step(
        &self,
        wants: &[Want],
        more: u32,
        locked: &mut Vec<(u32, &'static str)>,
    ) -> Option<PursuitStep> {
        let me = self.facts.me;
        let mut best: Option<(f32, PursuitStep)> = None;
        for want in wants {
            let Want::Item(wcid) = *want else { continue };
            for s in self.chains.sources(want) {
                if s.kind.is_hunted() || (self.dropped)(s) {
                    continue;
                }
                // A combine is made where the character stands: no place of its own.
                let here = Place {
                    at: me.unwrap_or_default(),
                    cell: 0,
                };
                let combine = (s.kind == Yield::Combine).then_some(here);
                let Some(at) = s.nearest(me).or(combine) else {
                    continue;
                };
                let step = match s.kind {
                    Yield::Vendor => PursuitStep::Buy {
                        wcid,
                        name: ware_name(s, wcid).unwrap_or_else(|| format!("item {wcid}")),
                        vendor: s.name.clone(),
                        more,
                    },
                    Yield::Floor => PursuitStep::PickUp {
                        wcid,
                        name: s.name.clone(),
                        at,
                    },
                    Yield::Npc => PursuitStep::AskFor { source: s, at },
                    Yield::Combine => {
                        // Both carried, the first used on the second; a missing one is had first.
                        let missing: Vec<_> = s
                            .keys
                            .iter()
                            .filter(|k| (self.asking.carried)(k.0) == 0)
                            .collect();
                        match (missing.is_empty(), s.keys.as_slice()) {
                            (true, [with, on, ..]) => PursuitStep::Combine {
                                source: s,
                                with: with.0,
                                on: on.0,
                            },
                            _ => {
                                for (k, name) in missing {
                                    if !locked.iter().any(|l| l.0 == *k) {
                                        locked.push((*k, name.as_str()));
                                    }
                                }
                                continue;
                            }
                        }
                    }
                    Yield::Use => {
                        let carried = |k: &u32| (self.asking.carried)(*k) > 0;
                        match s.keys.iter().map(|k| k.0).find(carried) {
                            Some(with) => PursuitStep::UseOn {
                                source: s,
                                at,
                                with,
                            },
                            None => {
                                for (k, name) in &s.keys {
                                    if !locked.iter().any(|l| l.0 == *k) {
                                        locked.push((*k, name.as_str()));
                                    }
                                }
                                continue;
                            }
                        }
                    }
                    _ => {
                        let carried = |k: &u32| (self.asking.carried)(*k) > 0;
                        let key = s.keys.iter().map(|k| k.0).find(carried);
                        if key.is_none() && !s.keys.is_empty() && !(self.unlocked)(s) {
                            for (k, name) in &s.keys {
                                if !locked.iter().any(|l| l.0 == *k) {
                                    locked.push((*k, name.as_str()));
                                }
                            }
                            continue;
                        }
                        PursuitStep::OpenChest { chest: s, at, key }
                    }
                };
                let away = me.map_or(0.0, |m| at.at.distance(m));
                if best.as_ref().is_none_or(|b| away < b.0) {
                    best = Some((away, step));
                }
            }
        }
        best.map(|b| b.1)
    }
}

/// The name the shop `s` sells `wcid` under.
pub(super) fn ware_name(s: &Source, wcid: u32) -> Option<String> {
    let cell = s.landblocks.first().map_or(0, |l| l.0);
    let shop = ac_world::shops::all()
        .iter()
        .find(|p| p.wcid == s.wcid && p.cell == cell);
    let ware = shop.and_then(|p| p.sells.iter().find(|w| w.wcid == wcid));
    ware.map(|w| w.name.clone())
}

/// The wanted item and the source a fetch step works.
fn fetch_key(step: &PursuitStep) -> Option<(u32, u32)> {
    match step {
        PursuitStep::PickUp { wcid, .. } => Some((*wcid, *wcid)),
        PursuitStep::OpenChest { chest: s, .. }
        | PursuitStep::UseOn { source: s, .. }
        | PursuitStep::AskFor { source: s, .. }
        | PursuitStep::Combine { source: s, .. } => match s.want {
            Want::Item(w) => Some((w, s.wcid)),
            _ => None,
        },
        _ => None,
    }
}

impl Client {
    /// Whether a step at source `s` was refused within [`PURSUIT_RETRY`] of `now`.
    pub(crate) fn is_source_dropped(&self, s: &Source, now: Instant) -> bool {
        let Want::Item(w) = s.want else { return false };
        let dropped = &self.autoplay.growth.sources_dropped;
        dropped.within(&(w, s.wcid), now, PURSUIT_RETRY)
    }

    /// Leave the source `key` (wanted item, source class) out for [`PURSUIT_RETRY`], saying `why`;
    /// the board is worked out again at once, so the next source takes its turn.
    pub(crate) fn drop_source(&mut self, key: (u32, u32), why: String, now: Instant) {
        tracing::info!("pursuit: source {} of {} dropped: {why}", key.1, key.0);
        self.autoplay.note(format!("pursuit: {why}"), now);
        let st = &mut self.autoplay.growth;
        st.sources_dropped.mark(key, now);
        st.fetching = None;
        self.quest_board_at = None;
    }

    /// The working pursuit's walk to what it fetches and its act there, from `grow_pursuit`.
    pub(super) fn grow_fetch(&mut self, line: &PursuitLine, now: Instant) -> Option<bool> {
        let key = fetch_key(&line.step)?;
        let (place, source, label) = match &line.step {
            PursuitStep::PickUp { wcid, name, at } => (*at, *wcid, name.clone()),
            PursuitStep::OpenChest { chest: s, at, .. }
            | PursuitStep::UseOn { source: s, at, .. }
            | PursuitStep::AskFor { source: s, at } => (*at, s.wcid, s.name.clone()),
            PursuitStep::Combine { source: s, .. } => {
                let p = self.player.as_ref()?;
                let here = Place {
                    at: p.world_position(),
                    cell: p.cell,
                };
                (here, s.wcid, s.name.clone())
            }
            _ => return None,
        };
        if let Some(f) = self.autoplay.growth.fetching.clone() {
            if f.key != key {
                self.autoplay.growth.fetching = None;
            } else {
                match self.heard(&f, now) {
                    Heard::Waiting => return Some(true),
                    Heard::Again if f.tries < UNLOCK_TRIES => {
                        self.autoplay.growth.fetching = None;
                        self.autoplay.growth.unlock_tries = f.tries;
                    }
                    Heard::Again => {
                        let why = match f.act {
                            Act::Take => format!("{label}: would not come up"),
                            Act::Talk => format!("{label}: gave nothing"),
                            Act::Use => format!("{label}: no answer to the use"),
                            _ => format!("{label}: no answer to the key"),
                        };
                        self.drop_source(key, why, now);
                        return Some(false);
                    }
                    Heard::Refused(why) => {
                        self.drop_source(key, format!("{label}: {why}"), now);
                        return Some(false);
                    }
                    // Taken or emptied: the board says what is next.
                    Heard::Done if f.act != Act::Unlock => {
                        self.autoplay.growth.fetching = None;
                        self.quest_board_at = None;
                        return Some(true);
                    }
                    Heard::Done => {
                        self.autoplay.growth.chests_unlocked.mark(source, now);
                    }
                }
            }
        }
        // Two carried items: the one used on is the target, and there is nowhere to walk.
        if let PursuitStep::Combine { on, .. } = line.step {
            let target = self
                .world
                .inventory()
                .find(|o| o.weenie_class_id == on && o.wielder.is_none())?
                .guid;
            if self.hands_busy(now) {
                return Some(true);
            }
            let act = self.fetch_act(&line.step, target, 0.0, now)?;
            self.fetch_sent(key, target, act, now);
            let said = format!("for {}: {}", line.title, line.step.words(0));
            self.autoplay.say(Doing::Looting, said);
            return Some(true);
        }
        let me = self.my_position()?;
        let found = self.lying_near(source, place);
        let goal = found.map_or(place, |(_, p)| p);
        if me.distance(goal.at) > FETCH_REACH {
            self.autoplay.growth.fetching = None;
            if self.journey_to(goal, FETCH_STOP) {
                let to = ac_world::quest_chain::place_name(goal.cell);
                let why = format!("for {}: on the way to {label} in {to}", line.title);
                self.autoplay.say(Doing::Traveling, why);
                return Some(true);
            }
            // No way but from a gem's exit (the Mistress' Chest on Ithaenc's island): the gem
            // first, as for a hunt's ground; dropped only when no gem leads there either.
            let quests = self.travel.quests_done.clone();
            if let Some(gem) = self.gem_way(&[(goal.cell >> 16, goal)], &quests) {
                let why = format!("for {}: {} first, to reach {label}", line.title, gem.name);
                tracing::info!("pursuit: {}: {why}", line.id);
                self.autoplay.note(why, now);
                self.autoplay.growth.want_gem(&line.id, gem);
                self.quest_board_at = None;
                return Some(false);
            }
            self.drop_source(key, format!("no way to {label}"), now);
            return Some(false);
        }
        let Some((guid, _)) = found else {
            if self.autoplay.growth.fetching.is_none() {
                self.fetch_sent(key, 0, Act::Look, now);
            }
            self.autoplay
                .say(Doing::Looting, format!("looking for {label}"));
            return Some(true);
        };
        // The server walks the character to what it takes or uses, and movement of ours ends that
        // walk cancelled (Player_Inventory.cs:1014-1018): the trip here stops a tick first.
        if self.traveling() || self.is_walking() {
            self.interrupt_travel("at the quest's source");
            self.stop_going(None, "at the quest's source");
            return Some(true);
        }
        if self.hands_busy(now) {
            return Some(true);
        }
        if !self.has_room_for(line, now) {
            return Some(false);
        }
        let act = self.fetch_act(&line.step, guid, me.distance(goal.at), now)?;
        self.fetch_sent(key, guid, act, now);
        self.autoplay
            .say(Doing::Looting, format!("for {}: {label}", line.title));
        Some(true)
    }

    /// Whether the packs have a slot for what an act at the source brings: one with none would
    /// see the item go nowhere, so the quest is passed over until something has gone.
    fn has_room_for(&mut self, line: &PursuitLine, now: Instant) -> bool {
        let brings = matches!(
            line.step,
            PursuitStep::PickUp { .. } | PursuitStep::AskFor { .. } | PursuitStep::OpenChest { .. }
        );
        if !brings || self.room_anywhere() > 0 {
            return true;
        }
        let why = format!("no room in the pack for {}", line.step.words(0));
        self.hold_pursuit(&line.id, why, now);
        false
    }

    /// Do the next act on `guid`, `away` metres off: pick it up, unlock it, open it, or use on it.
    fn fetch_act(&mut self, step: &PursuitStep, guid: u32, away: f32, now: Instant) -> Option<Act> {
        let unlocked = self
            .autoplay
            .growth
            .fetching
            .as_ref()
            .is_some_and(|f| f.guid == guid && f.act == Act::Unlock);
        match *step {
            PursuitStep::PickUp { .. } => {
                self.take(guid);
                Some(Act::Take)
            }
            PursuitStep::AskFor { .. } => {
                self.interact(guid);
                Some(Act::Talk)
            }
            PursuitStep::OpenChest { key: Some(k), .. } if !unlocked => {
                let held = self
                    .world
                    .inventory()
                    .find(|o| o.weenie_class_id == k && o.wielder.is_none())?
                    .guid;
                self.autoplay.growth.unlock_with = k;
                self.use_on(held, guid).then_some(Act::Unlock)
            }
            PursuitStep::UseOn { with, .. } | PursuitStep::Combine { with, .. } => {
                if self.combat || self.magic {
                    self.leave_combat();
                    return Some(Act::Peace);
                }
                let held = self
                    .world
                    .inventory()
                    .find(|o| o.weenie_class_id == with && o.wielder.is_none())?
                    .guid;
                self.autoplay.growth.unlock_with = with;
                self.use_on(held, guid).then_some(Act::Use)
            }
            PursuitStep::OpenChest { .. } => {
                // Emptied as a corpse is: the loot rules keep what the quest rule asks for.
                self.remember_journey();
                self.interact(guid);
                let allow = crate::autoplay::loot::walk::loot_wait(away);
                self.autoplay.take_up_corpse(guid, now, allow);
                Some(Act::Open)
            }
            _ => None,
        }
    }

    fn fetch_sent(&mut self, key: (u32, u32), guid: u32, act: Act, at: Instant) {
        // Answered with a UseDone, which frees the slot a cast would wait on (Player_Use.cs).
        if matches!(act, Act::Talk | Act::Unlock | Act::Use) {
            self.autoplay.cast_sent = Some(at);
        }
        let (with, tries) = match (act, &self.autoplay.growth.fetching) {
            (Act::Unlock | Act::Use, _) => {
                let tries = std::mem::take(&mut self.autoplay.growth.unlock_tries) + 1;
                (self.autoplay.growth.unlock_with, tries)
            }
            (Act::Take | Act::Talk, _) => (
                0,
                std::mem::take(&mut self.autoplay.growth.unlock_tries) + 1,
            ),
            (_, Some(f)) => (f.with, f.tries),
            // Kept for the use that follows: `unlock_tries` is taken by it.
            (Act::Peace, None) => (0, self.autoplay.growth.unlock_tries),
            _ => (0, 0),
        };
        self.autoplay.growth.fetching = Some(Fetching {
            key,
            guid,
            act,
            at,
            with,
            tries,
        });
    }

    /// The working fetch's key out on its chest, or item on its object: worked on ahead of every
    /// rule but healing, dodging and dying, until answered (`is_unlocking`). True while it holds.
    pub(crate) fn autoplay_unlock(&mut self, now: Instant) -> bool {
        if !self.is_unlocking() {
            return false;
        }
        let Some(line) = self
            .pursuit
            .iter()
            .find(|l| l.working && !self.is_awaiting_answer(&l.id))
            .cloned()
        else {
            self.autoplay.growth.fetching = None;
            return false;
        };
        self.grow_fetch(&line, now) == Some(true)
    }

    /// Whether the working fetch has used its key on a chest not yet opened, or is using an item
    /// on an object: nothing acts between, as a cast cancels the server's use (`UNLOCKED_FOR`).
    pub(crate) fn is_unlocking(&self) -> bool {
        self.autoplay
            .growth
            .fetching
            .as_ref()
            .is_some_and(|f| matches!(f.act, Act::Unlock | Act::Peace | Act::Use))
    }

    /// How the server answered the act `f`: by its `UseDone` for a key, by what the pack holds
    /// once a take, a use or the loot rules are done.
    fn heard(&self, f: &Fetching, now: Instant) -> Heard {
        let waited = now.duration_since(f.at) >= ANSWER_WAIT;
        let has_item = self.carried_count(f.key.0) > 0;
        match f.act {
            Act::Look if waited => Heard::Refused("not there".to_string()),
            Act::Look => Heard::Waiting,
            Act::Take | Act::Talk if has_item => Heard::Done,
            Act::Take | Act::Talk if waited => Heard::Again,
            Act::Take | Act::Talk => Heard::Waiting,
            // A one-use key is used up by the unlock; a UseDone with no error is also what a
            // cancelled walk to the chest sends (`Player_Use.cs:157-163`), so it is not the answer.
            Act::Unlock if f.with != 0 && self.carried_count(f.with) == 0 => Heard::Done,
            Act::Unlock => match self.use_done.filter(|(_, at)| *at >= f.at) {
                Some((LOCK_ALREADY_UNLOCKED, _)) => Heard::Done,
                Some((err, _)) if err != 0 && err != YOURE_TOO_BUSY => {
                    let said = crate::weenie_errors::text(err).unwrap_or("refused");
                    Heard::Refused(said.to_string())
                }
                _ if waited => Heard::Again,
                _ => Heard::Waiting,
            },
            // The stance changed, the use goes out (`autoplay_craft_ammo` waits as long).
            Act::Peace if now.duration_since(f.at) >= STANCE_CHANGE => Heard::Again,
            Act::Peace => Heard::Waiting,
            // The made item comes before the UseDone (`RecipeManager.cs:135-143`).
            Act::Use if has_item => Heard::Done,
            Act::Use => match self.use_done.filter(|(_, at)| *at >= f.at) {
                Some((err, _)) if err != 0 && err != YOURE_TOO_BUSY => {
                    let said = crate::weenie_errors::text(err).unwrap_or("refused");
                    Heard::Refused(said.to_string())
                }
                _ if waited => Heard::Again,
                _ => Heard::Waiting,
            },
            Act::Open if self.autoplay.is_at_corpse(f.guid) => Heard::Waiting,
            Act::Open if has_item => Heard::Done,
            Act::Open => Heard::Refused("held nothing wanted".to_string()),
        }
    }

    /// The nearest object of weenie class `wcid` lying in the world within [`FOUND_NEAR`] of
    /// `place`, and where it stands.
    fn lying_near(&self, wcid: u32, place: Place) -> Option<(u32, Place)> {
        let me = self.world.player_guid;
        self.world
            .objects
            .values()
            .filter(|o| o.weenie_class_id == wcid && Some(o.guid) != me)
            .filter(|o| o.container.is_none() && o.wielder.is_none())
            .filter_map(|o| {
                let cell = o.position?.cell;
                Some((
                    o.guid,
                    Place {
                        at: o.world_pos()?,
                        cell,
                    },
                ))
            })
            .filter(|(_, p)| p.at.distance(place.at) <= FOUND_NEAR)
            .min_by(|a, b| {
                let d = |p: &Place| p.at.distance(place.at);
                d(&a.1).total_cmp(&d(&b.1))
            })
    }
}
