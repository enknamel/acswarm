use std::time::{Duration, Instant};

use crate::autoplay::{Autoplay, Doing, GIVE_EVERY};
use crate::Client;

/// Everything that can stand between the pack and a tidier pack, as
/// `Client::autoplay_tidy` finds it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct TidyGate {
    /// The loot profile says this character does not tidy.
    pub(crate) tidy_pack_off: bool,
    /// A vendor's window is open.
    pub(crate) counter_open: bool,
    /// Loading or unloading the party's quartermaster.
    pub(crate) quartermaster: bool,
    /// Two bundles are waiting to be made into ammunition.
    pub(crate) crafting: bool,
    /// Something was handed to a teammate a moment ago.
    pub(crate) gave_lately: bool,
    /// A take is queued or on its way from a corpse.
    pub(crate) take_in_air: bool,
}

/// Why the pack is being left alone, or `None` to go ahead.
///
/// Beyond the profile's own say-so, every one of these is a moment when
/// something else is counting on a guid staying where it is. A merge
/// makes one stack vanish into another, and whatever was holding it --
/// a sale list, the money counted out for a runner, a take the server
/// has not answered -- is then waiting on something that does not exist.
fn why_not_tidy(g: TidyGate) -> Option<&'static str> {
    if g.tidy_pack_off {
        // Nothing stops a player tidying by hand; this is only the
        // rules keeping their hands off a pack they were told to.
        Some("this profile leaves the pack as it is")
    } else if g.counter_open {
        // A run to town holds what it has sent the vendor by guid, and
        // a merge makes one of those vanish mid-sale. Whatever was
        // bought is tidied the moment the window closes.
        Some("a counter is open")
    } else if g.quartermaster {
        // Money counted out for the runner is a stack of its own, and
        // tidying poured it straight back into the pile it came from:
        // count out, merge back, count out again, a hundred and twenty
        // six times in one watched run.
        Some("the quartermaster is being loaded or unloaded")
    } else if g.crafting {
        // The heads and the shafts are about to be used on each other.
        Some("ammunition is being made")
    } else if g.gave_lately {
        // A hand-over is a split and a give, and the server is still
        // working through it.
        Some("something was just handed to a teammate")
    } else if g.take_in_air {
        // The thing coming off the corpse may be the very stack a pour
        // would empty, and the server answers one at a time.
        Some("a take is queued or in the air")
    } else {
        None
    }
}

/// How often a stack is poured into another. The server takes one merge
/// at a time and answers in its own time.
pub(crate) const MERGE_EVERY: Duration = Duration::from_millis(600);

/// How long the tidying leaves the pack alone once weight is the only
/// thing standing between it and a tighter pack. Nothing the character
/// can do about that comes quickly: it has to sell, or hand something
/// over, and looking every tick only burns the time.
pub(super) const TIDY_LADEN_WAIT: Duration = Duration::from_secs(30);

/// Why no stack was poured into another (see `Client::pour_next`).
///
/// Four answers rather than one, because the callers do different
/// things with them: a town run stays where it is while a pour is in
/// the air, and walks on to the counter when the pack is tight or the
/// character too laden; the housekeeping says the laden one out loud
/// once and then leaves the pack alone for a while.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Unpoured {
    /// The pack is as tight as it goes.
    Tight,
    /// There is something to pour and no room to be handed it. The
    /// server weighs a pour as though the source were being picked up
    /// for the first time.
    Laden,
    /// The last pour has not been answered yet.
    Wait(crate::did::Did),
    /// This pour was turned down, by the server or by our own rules.
    Turned(crate::did::Did),
}

impl Autoplay {
    /// Every guid some other errand is holding on to across ticks.
    ///
    /// Each of these is a thing another part of the rules has written
    /// down and will come back to: a weapon to wield once the hands are
    /// free, the arrows chosen for this target, the two bundles waiting
    /// to be made into ammunition, the stack counted out for a teammate.
    /// Pouring one of those into another stack makes its guid vanish
    /// under the errand, which then waits on something that no longer
    /// exists. The money one was watched: counted out, poured straight
    /// back, counted out again.
    pub(crate) fn held_by_an_errand(&self) -> [Option<u32>; 6] {
        let (craft_from, craft_to) = match self.crafting {
            Some((from, to, _)) => (Some(from), Some(to)),
            None => (None, None),
        };
        [
            self.pending_wield,
            self.wanted_ammo,
            self.put_down,
            craft_from,
            craft_to,
            self.handing.map(|(item, _)| item),
        ]
    }
}

impl Client {
    /// Every stack in the packs, as the compactor sees them.
    ///
    /// What the character is holding is here too: a quiver of arrows
    /// can be topped up from the pack, and is worth topping up, but is
    /// never the stack poured away.
    pub fn pack_stacks(&self) -> Vec<crate::pack::Stack> {
        let me = self.world.player_guid;
        let describe = |o: &ac_world::WorldObject, wielded: bool| crate::pack::Stack {
            guid: o.guid,
            wcid: o.weenie_class_id,
            name: o.name.clone(),
            count: o.stack_size.max(1),
            max: o.max_stack_size,
            wielded,
            burden: o.burden,
        };
        self.world
            .inventory()
            .map(|o| describe(o, false))
            .chain(
                self.world
                    .objects
                    .values()
                    .filter(|o| me.is_some() && o.wielder == me)
                    .map(|o| describe(o, true)),
            )
            .collect()
    }

    /// Pour loose stacks together, a pour at a time.
    ///
    /// Slots are the scarce thing, not weight, and nothing warns a
    /// player that a purchase landed beside a pile of the same, or that
    /// the peas taken off four corpses are sitting in four stacks. This
    /// is housekeeping rather than a goal because it must not have to
    /// win a tick to happen: a character that fights, loots and walks
    /// all afternoon never has a quiet one, and tidying that waits for
    /// one never runs. It costs nothing to let it go first -- the server
    /// makes a pack-to-pack merge on the spot, with no walk, no animation
    /// and no busy check, so a pour cannot get in the way of a take, a
    /// cast or a walk.
    pub(crate) fn autoplay_tidy(&mut self, now: Instant) {
        // Read the last pour's answer before any gate. A counter opened
        // or a fight started in the meantime is no reason to go on
        // believing a pour is still in the air.
        match self.settle_pour(now) {
            Some(crate::did::Did::Waiting(_)) => return,
            Some(did) => self.aside("tidy the pack", &did, now),
            None => {}
        }
        let gate = TidyGate {
            // With no profile the pack is still tidied: it is not looting.
            tidy_pack_off: !self.loot_profile().is_none_or(|p| p.looting.tidy_pack),
            counter_open: self.world.open_vendor.is_some(),
            quartermaster: matches!(
                self.autoplay.growth.mode.stage(),
                Some(crate::logistics::Stage::HandOver | crate::logistics::Stage::HandOut)
            ),
            crafting: self.autoplay.crafting.is_some(),
            gave_lately: self
                .autoplay
                .last_give
                .is_some_and(|t| now.duration_since(t) < GIVE_EVERY),
            take_in_air: self.loot_inflight.is_some() || !self.loot_queue.is_empty(),
        };
        if let Some(why) = why_not_tidy(gate) {
            tracing::trace!("not tidying the pack: {why}");
            return;
        }
        if self.autoplay.tidy_laden_until.is_some_and(|t| now < t) {
            return;
        }
        // Deciding what to pour copies the name of every stack carried,
        // and no answer can change faster than the server gives one.
        if self
            .autoplay
            .tidy_looked
            .is_some_and(|t| now.duration_since(t) < MERGE_EVERY)
        {
            return;
        }
        self.autoplay.tidy_looked = Some(now);
        match self.pour_next(now) {
            // A note, not a `say`: this is not what the character is
            // doing. Said as a status it would flicker against the
            // fighting or looting that is.
            Ok(m) => self.autoplay.note(
                format!("putting {} {} with the rest", m.amount, m.name),
                now,
            ),
            Err(Unpoured::Laden) => {
                self.autoplay.tidy_laden_until = Some(now + TIDY_LADEN_WAIT);
                let did = crate::did::Did::blocked("too laden to put stacks together");
                self.aside("tidy the pack", &did, now);
            }
            Err(Unpoured::Turned(did)) => self.aside("tidy the pack", &did, now),
            Err(Unpoured::Tight | Unpoured::Wait(_)) => {}
        }
    }

    /// Read the server's answer to the pour in the air, if there is one.
    ///
    /// `None` means there is nothing to wait for: no pour was sent, or
    /// the one sent is over. `Some(Waiting)` means it is still out
    /// there and the counts a new choice would be made from are stale.
    /// Anything else is what the server said about it.
    pub(crate) fn settle_pour(&mut self, now: Instant) -> Option<crate::did::Did> {
        use crate::did::{Because, Did};
        let (sent, at) = self.autoplay.pour.clone()?;
        let (from, to) = (sent.merge.from, sent.merge.to);
        let target_now = self.world.objects.get(&to).map(|o| o.stack_size.max(1));
        let refusal = crate::pack::refusal_of(
            at,
            self.move_refused.get(&from).copied(),
            self.move_refused.get(&to).copied(),
        );
        let waited = now.saturating_duration_since(at);
        match crate::pack::pour_answer(&sent, target_now, refusal, waited) {
            crate::pack::PourAnswer::InAir => {
                Some(Did::waiting("the last pour has not landed yet"))
            }
            crate::pack::PourAnswer::Landed => {
                // What the surviving stack was taken for is settled
                // here rather than when the pour was sent: a refused
                // pour used to raise the target's tag anyway, so a
                // stack meant for a counter quietly became one to keep
                // and was never sold.
                self.autoplay.ledger.merged(from, to);
                self.autoplay
                    .growth
                    .wont_merge
                    .note((from, to), &Did::Done, now);
                self.autoplay.pour = None;
                tracing::debug!(
                    "pour landed: {} {} into {to:#010x}",
                    sent.merge.amount,
                    sent.merge.name
                );
                None
            }
            crate::pack::PourAnswer::Refused(code) => {
                // Spent: the refusal was about this pour, and leaving
                // it behind would answer the next one too.
                for g in [from, to] {
                    if self
                        .move_refused
                        .get(&g)
                        .is_some_and(|(_, when)| *when >= at)
                    {
                        self.move_refused.remove(&g);
                    }
                }
                let did = Did::Blocked(match code {
                    0 => Because::ours("the server would not put those two together"),
                    code => Because::server(code),
                });
                self.autoplay.growth.wont_merge.note((from, to), &did, now);
                self.autoplay.pour = None;
                Some(did)
            }
            crate::pack::PourAnswer::Lost => {
                // No word at all. Not a refusal -- the pair is left a
                // moment and offered again -- but the pack has to be
                // read afresh before anything else is asked for.
                self.autoplay.growth.wont_merge.note(
                    (from, to),
                    &Did::waiting("no word on the pour"),
                    now,
                );
                self.autoplay.pour = None;
                None
            }
        }
    }

    /// Choose the next pour, send it, and remember it until the server
    /// answers ([`Unpoured`] says why one was not sent).
    ///
    /// One at a time, because the server answers in its own time and
    /// the counts a second choice would be made from are the ones from
    /// before the first. What a pour must not touch is settled here: a
    /// pair the server has turned down, a stack another errand is
    /// holding, and weight the server will not hand the character.
    pub(crate) fn pour_next(&mut self, now: Instant) -> Result<crate::pack::Merge, Unpoured> {
        use crate::did::Did;
        // Nothing is chosen over counts the server has not settled.
        if let Some(did) = self.settle_pour(now) {
            return Err(match did {
                Did::Waiting(_) => Unpoured::Wait(did),
                did => Unpoured::Turned(did),
            });
        }
        let may_carry = self.burden_room();
        let stacks = self.pack_stacks();
        let errands = self.autoplay.held_by_an_errand();
        // A pair the server has turned down waits its turn out, and a
        // stack another errand is holding is left where it is. Without
        // the first, one stubborn pair was the only answer ever given
        // and nothing else in the pack was ever poured.
        //
        // And two stacks whose words disagree are never poured
        // together. A pour makes one stack of two, and one stack can
        // carry one word: a stack the player said to sell poured into
        // one they said to keep was settled as kept (the ledger takes
        // the cautious answer), and poured into one nothing was
        // decided about it was settled as nothing -- either way the
        // Sell was gone before the character set off for town. Kept
        // apart, each stack goes where its own word says.
        let ledger = &self.autoplay.ledger;
        let skip = |from: u32, to: u32| {
            self.autoplay.growth.wont_merge.held(&(from, to), now)
                || errands.iter().flatten().any(|g| *g == from || *g == to)
                || ledger.by_guid(from) != ledger.by_guid(to)
        };
        let Some(m) = crate::pack::next_merge_unless(&stacks, may_carry, skip) else {
            // Nothing to pour -- or nothing light enough. The two are
            // worth telling apart: one is a tidy pack, the other is a
            // character that must sell something first. Asked with the
            // same skips, so a pack whose only pours are held is not
            // called too laden.
            return Err(
                if crate::pack::next_merge_unless(&stacks, u32::MAX, skip).is_some() {
                    Unpoured::Laden
                } else {
                    Unpoured::Tight
                },
            );
        };
        if self
            .autoplay
            .last_merge
            .is_some_and(|t| now.duration_since(t) < MERGE_EVERY)
        {
            return Err(Unpoured::Wait(Did::waiting(
                "the last pour has not landed yet",
            )));
        }
        let to_before = self
            .world
            .objects
            .get(&m.to)
            .map(|o| o.stack_size.max(1))
            .unwrap_or(0);
        self.autoplay.last_merge = Some(now);
        if !self.send_merge(m.from, m.to, Some(m.amount)) {
            // Our own rules turned it down: not both carried, not the
            // same weenie, or the target does not stack at all.
            let did = Did::refused("those two will never join");
            self.autoplay
                .growth
                .wont_merge
                .note((m.from, m.to), &did, now);
            return Err(Unpoured::Turned(did));
        }
        self.autoplay.pour = Some((
            crate::pack::PourSent {
                merge: m.clone(),
                to_before,
            },
            now,
        ));
        Ok(m)
    }

    /// Pour one loose stack into another, and say what came of it.
    ///
    /// `Did::Done` means the pack is as tight as it goes. `Blocked`
    /// means nothing can be poured until the character is lighter:
    /// the server weighs a pour as though the source were being picked
    /// up for the first time, so a character near its ceiling is
    /// refused a move that changes its burden by nothing at all. The
    /// refusal carries no message and no code, which is how a tidier
    /// that could not read it spent whole afternoons asking.
    ///
    /// This is the town run's way in, where tidying is the step and is
    /// worth a status line of its own. Everywhere else the pack is
    /// tidied as housekeeping (`Client::autoplay_tidy`).
    pub(crate) fn compress(&mut self, now: Instant) -> crate::did::Did {
        use crate::did::{Because, Did};
        match self.pour_next(now) {
            Ok(m) => {
                self.autoplay.say(
                    Doing::Tidying,
                    format!("putting {} {} with the rest", m.amount, m.name),
                );
                Did::Acting
            }
            Err(Unpoured::Tight) => Did::Done,
            Err(Unpoured::Laden) => Did::Blocked(Because::ours("too laden to put stacks together")),
            Err(Unpoured::Wait(did) | Unpoured::Turned(did)) => did,
        }
    }
}

#[cfg(test)]
mod tests;
