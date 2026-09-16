use std::time::Duration;

use crate::Client;

/// Coming this much closer (metres) counts as getting somewhere.
pub(crate) const REACH_PROGRESS: f32 = 1.0;

/// Walking towards something for this long without getting closer is
/// not walking towards it any more.
pub(crate) const REACH_GIVE_UP: Duration = Duration::from_secs(20);

/// A pessimistic walking speed for pricing that walk, metres a second:
/// the way round a dungeon corner is longer than the line to it.
pub(super) const LOOT_WALK: f32 = 2.5;

/// Whether a refusal's code is one that gates a drop that can only be had
/// so often: YouHaveSolvedThisQuestTooRecently or TooManyTimes.
pub(crate) fn only_so_often(code: u32) -> bool {
    matches!(code, 0x043E | 0x043F)
}

/// Which item an inventory refusal (`InventoryServerSaveFailed`, `item`
/// and `err` as it came) is about, given the take in flight (`inflight`).
///
/// Usually the one it names. ACE turns a drop that can only be had so
/// often down naming no item at all, with only the quest's reason
/// (`QuestManager.HandleSolveError`). Read as a refusal of nothing, the
/// take was never known to be refused and was asked for again until the
/// corpse was given up on.
pub(crate) fn refused_item(item: u32, err: u32, inflight: Option<u32>) -> Option<u32> {
    match item {
        0 if only_so_often(err) => inflight,
        0 => None,
        item => Some(item),
    }
}

impl Client {
    /// The thing `guid`, lying loose, as the choice of how to take it
    /// sees it (see `room::Loose`).
    pub(super) fn loose(o: &ac_world::WorldObject) -> crate::room::Loose {
        crate::room::Loose {
            wcid: o.weenie_class_id,
            count: o.stack_size.max(1),
            // Coin and spell components: what the server never caps or
            // makes unique, so a pour that skips a take's checks skips
            // nothing that matters.
            pours: o.max_stack_size > 1
                && o.item_type
                    & (ac_world::item_type::MONEY | ac_world::item_type::SPELL_COMPONENTS)
                    != 0,
            is_pack: o.item_type & ac_world::item_type::CONTAINER != 0,
        }
    }

    /// How the loose thing `guid` is to be taken -- into which pack, or
    /// poured onto which carried stack -- or `None` when it is unknown
    /// or there is nowhere for it (see `room::how_to_take`).
    pub(crate) fn how_to_take(&self, guid: u32) -> Option<crate::room::Take> {
        let o = self.world.objects.get(&guid)?;
        let item = Self::loose(o);
        // Every stack carried is copied by name; only a thing that can
        // pour needs them.
        let carried = if item.pours {
            self.pack_stacks()
        } else {
            Vec::new()
        };
        crate::room::how_to_take(&item, &carried, &self.packs())
    }

    /// The pack the loose thing `guid` is to be picked up into, or
    /// `None` when it is unknown or no pack has a slot: the choice a
    /// take makes, less the pour. A pack off the ground goes to the
    /// main pack's pack slots whatever the room; named into a sack, the
    /// server turns it down without a word, and a pickup's answer is
    /// not read.
    pub(crate) fn where_to_pick_up(&self, guid: u32) -> Option<u32> {
        let o = self.world.objects.get(&guid)?;
        match crate::room::how_to_take(&Self::loose(o), &[], &self.packs())? {
            crate::room::Take::Put(into) => Some(into),
            crate::room::Take::Merge { .. } => None,
        }
    }

    /// Hold what the server said was full against what each pack holds
    /// now (see `room::full_mark`): the word is kept at the most the
    /// pack has been seen to hold since, so that one thing leaving
    /// lifts it, and lifted once something has.
    pub(crate) fn note_pack_counts(&mut self) {
        if self.packs_said_full.is_empty() {
            return;
        }
        let counts: Vec<(u32, u32)> = self.packs().all().map(|p| (p.guid, p.used)).collect();
        for (guid, used) in counts {
            if let Some(held) = self.packs_said_full.get(&guid).copied() {
                match crate::room::full_mark(held, used) {
                    Some(mark) => {
                        self.packs_said_full.insert(guid, mark);
                    }
                    None => {
                        self.packs_said_full.remove(&guid);
                    }
                }
            }
        }
    }

    /// The server has said "Unable to put {item} into container" of
    /// `name` (see `refusals::Refusal::Put`). About the take in flight,
    /// when it names that take's item: the refusal that goes with it
    /// says nothing more, and has usually been read already -- a tick
    /// reads its chat after its events -- so the refusal is judged
    /// again now.
    pub(crate) fn hear_put_refusal(&mut self, name: &str) {
        let Some(sent) = self.loot_sent.as_mut() else {
            return;
        };
        if self
            .world
            .objects
            .get(&sent.item)
            .is_some_and(|o| o.name == name)
        {
            sent.said_full = true;
            self.judge_take_refusal();
        }
    }

    /// The server has turned down the take `item` with `err`. Kept with
    /// the take and judged (see [`Client::judge_take_refusal`]): the
    /// words that say why, when there are any, are still to come.
    ///
    /// A pour off the body is not read this way: a pour is refused for
    /// weight or for a full stack, never for a slot, and its answer is
    /// read where it was sent (see `Client::tick_loot`).
    pub(crate) fn take_refused(&mut self, item: u32, err: u32) {
        if self.loot_merge.is_some() {
            return;
        }
        let Some(sent) = self.loot_sent.as_mut().filter(|s| s.item == item) else {
            return;
        };
        sent.refused = Some(err);
        self.judge_take_refusal();
    }

    /// Read a refused take as the pack it named being full when the
    /// server said so in words (see `refusals::Refusal::Put`), and only
    /// then: that pack is full until something leaves it, and the take
    /// goes once more into another pack with room. There is no third
    /// try, and a body nothing on it can go into is set aside for room
    /// by the loot rules rather than opened again -- reopened, it was
    /// refused the same take thirty times over.
    ///
    /// The words are the whole of it. A refusal with no reason and no
    /// word is not the pack: it is what the server sends for a second
    /// of a unique, whose explanation comes in the system chat where
    /// nothing here reads it, and for a pack put into a sack. Read as
    /// the pack being full, either marked every pack in turn and sent
    /// the character to town with its slots free.
    fn judge_take_refusal(&mut self) {
        let Some(sent) = self.loot_sent.clone() else {
            return;
        };
        if sent.refused.is_none() || !sent.said_full {
            return;
        }
        let item = sent.item;
        // A pack goes in a pack slot, and a pack slot refused says
        // nothing about the item slots.
        let is_pack = self
            .world
            .objects
            .get(&item)
            .is_some_and(|o| o.item_type & ac_world::item_type::CONTAINER != 0);
        if is_pack {
            return;
        }
        // Judged: whatever else is said of it is not read twice.
        self.loot_sent = None;
        let held = self
            .packs()
            .all()
            .find(|p| p.guid == sent.into)
            .map(|p| p.used)
            .unwrap_or(0);
        self.packs_said_full.insert(sent.into, held);
        let pack = self.pack_name(sent.into);
        if sent.retried {
            tracing::info!("the server says {pack} is full too ({held} items); leaving the rest");
            return;
        }
        match self
            .packs()
            .container_for_a_take()
            .filter(|c| *c != sent.into)
        {
            Some(other) => {
                let name = self.world.name_or_hex(item);
                tracing::info!(
                    "the server says {pack} is full ({held} items); taking {name} into {} instead",
                    self.pack_name(other)
                );
                // The refusal was the pack's, not the item's: left in
                // place, the rules would pass the item over.
                self.move_refused.remove(&item);
                self.loot_queue.push_front(item);
                self.loot_retry = Some(item);
            }
            None => {
                tracing::info!(
                    "the server says {pack} is full ({held} items), and no pack has room"
                );
            }
        }
    }

    /// What to call the pack `guid` in the log: the main pack, or the
    /// side pack's own name.
    fn pack_name(&self, guid: u32) -> String {
        if Some(guid) == self.world.player_guid {
            return "the main pack".into();
        }
        self.world.name_or_hex(guid)
    }
}

#[cfg(test)]
mod tests;
