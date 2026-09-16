use std::time::{Duration, Instant};

use crate::{pack, room, Client, Event};

/// How long a take off a body -- a put or a pour -- is waited on before
/// it is given up for lost. The server walks to the body and stoops
/// before it answers; nine characters' takes were answered in 1.4 s at
/// the median and 2.1 s at the ninety-ninth.
const TAKE_LOST: Duration = Duration::from_secs(4);

/// A take off a body as it went out: what was asked for, where it was
/// to go, and what the server has said of it since.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TakeSent {
    pub(crate) item: u32,
    /// The pack named in the put, or the carried stack poured onto.
    pub(crate) into: u32,
    /// Sent once already and refused for room: this is its second try,
    /// into another pack, and there is no third.
    pub(crate) retried: bool,
    /// The server has said "Unable to put {item} into container" of it
    /// since it went out (see `refusals::Refusal::Put`).
    pub(crate) said_full: bool,
    /// The server has refused it, with this reason (0 for none). Kept
    /// with the take rather than acted on at once because the words
    /// that say why come as chat, and a tick reads its chat after its
    /// events -- or a packet later: the refusal is judged again when
    /// they arrive (see `Client::judge_take_refusal`).
    pub(crate) refused: Option<u32>,
}

impl Client {
    pub fn use_by_name(&mut self, name: &str) -> bool {
        match self.object_by_name(name) {
            Some(guid) => {
                self.select(Some(guid));
                self.interact(guid);
                true
            }
            None => {
                tracing::debug!("no object named {name:?} in view yet");
                false
            }
        }
    }

    pub(crate) fn appraisal(&mut self, body: &[u8]) {
        use ac_net::messages::Appraisal;
        let a = match Appraisal::parse(body) {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!("appraisal: {e}");
                return;
            }
        };
        let name = self.world.name_or_hex(a.guid);
        let mut lines = vec![name.clone()];
        for key in [
            Appraisal::STRING_SHORT_DESC,
            Appraisal::STRING_LONG_DESC,
            Appraisal::STRING_USE,
        ] {
            if let Some(t) = a.string(key) {
                if !t.is_empty() {
                    lines.push(t.to_string());
                }
            }
        }
        if !a.success {
            lines.push("(appraisal failed)".into());
        }
        tracing::info!(
            "appraise {name}: {} properties",
            a.ints.len() + a.strings.len()
        );
        // A background appraisal (appraise_all) fills the cache without
        // opening the window or writing to the chat.
        let asked = self.appraise_inflight.iter().any(|(g, _)| *g == a.guid);
        self.appraise_inflight.retain(|(g, _)| *g != a.guid);
        if !asked {
            for l in lines {
                self.events.push(Event::Chat { text: l, kind: 1 });
            }
            self.last_appraisal = Some(a.guid);
            self.appraisal_seq += 1;
        }
        self.appraisals.insert(a.guid, a);
    }

    /// A creature the server lets go of takes its appraisal with it.
    /// ACE reuses a guid six hours after the thing that had it is gone
    /// (`GuidManager`), and a session runs longer than that: kept, the
    /// answer about a Drudge was read as the answer about the Cow that
    /// got its guid, and the Cow was fought on it. A creature that only
    /// went out of range is let go of the same way and asked about
    /// afresh when it comes back, which costs one question. Corpses and
    /// items keep theirs: the loot rules read those, and a guid that
    /// was a corpse comes back as nothing the loot rules would be asked
    /// about before the answer has long stopped mattering.
    pub(crate) fn forget_the_departed(&mut self, msg: &[u8]) {
        use ac_net::messages::{opcode, split};
        let Some((opcode::OBJECT_DELETE, body)) = split(msg) else {
            return;
        };
        let Some(guid) = body
            .get(..4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        else {
            return;
        };
        let creature = self
            .world
            .objects
            .get(&guid)
            .is_some_and(|o| o.item_type & ac_world::item_type::CREATURE != 0 && !o.is_player);
        if creature {
            self.appraisals.remove(&guid);
        }
    }

    /// Use an object where it is (Use 0x0036), never picking it up: read
    /// a book lying on the ground, open a chest, talk to an NPC. Retail's
    /// "Use Selected" action. False when the object is unknown.
    pub fn use_object(&mut self, guid: u32) -> bool {
        self.interrupt_travel("using something");
        let Some(o) = self.world.objects.get(&guid) else {
            return false;
        };
        tracing::info!("use {} ({guid:#010x}) in place", o.name);
        self.last_used = Some(guid);
        self.session
            .send_action(ac_net::messages::action::USE, &guid.to_le_bytes());
        true
    }

    /// Pick a loose item up into the pack (PutItemInContainer, naming
    /// the pack with room: the main pack while it has a slot, else the
    /// roomiest side pack, since the server puts a thing into the pack
    /// named and nowhere else). False for anything that is not a loose
    /// item.
    pub fn pick_up(&mut self, guid: u32) -> bool {
        self.interrupt_travel("picking something up");
        let Some(me) = self.world.player_guid else {
            return false;
        };
        let Some(o) = self.world.objects.get(&guid) else {
            return false;
        };
        let stuck = o.object_desc_flags & ac_world::object_desc_flags::STUCK != 0
            || o.item_type & ac_world::item_type::CREATURE != 0;
        if stuck || o.position.is_none() || o.container.is_some() || o.wielder.is_some() {
            return false;
        }
        tracing::info!("pick up {} ({guid:#010x})", o.name);
        self.last_used = Some(guid);
        // The same choice a take makes, less the pour (see
        // `Client::where_to_pick_up`). With no room anywhere the main
        // pack is named, and the server says so; a caller that meant to
        // check first has `pack_full`.
        let into = self.where_to_pick_up(guid).unwrap_or(me);
        let mut w = ac_net::wire::Writer::new();
        w.u32(guid).u32(into).u32(0);
        self.session
            .send_action(ac_net::messages::action::PUT_ITEM_IN_CONTAINER, &w.finish());
        true
    }

    /// Ask for one page of the open book (BookPageData 0x00AE); the
    /// answer fills `book.pages[index]`.
    pub fn read_page(&mut self, guid: u32, index: u32) {
        let mut w = ac_net::wire::Writer::new();
        w.u32(guid).i32(index as i32);
        self.session
            .send_action(ac_net::messages::action::BOOK_PAGE_DATA, &w.finish());
    }

    /// Open a carried book without "using" it (BookData 0x00AA).
    pub fn open_book(&mut self, guid: u32) {
        self.session
            .send_action(ac_net::messages::action::BOOK_DATA, &guid.to_le_bytes());
    }

    /// Select an item in a panel and appraise it, what a single click on
    /// a pack, container, vendor or trade row does.
    pub fn inspect(&mut self, guid: u32) {
        self.select(Some(guid));
        self.appraise(guid);
    }

    /// Ask the server to appraise an object (IdentifyObject 0x00C8); the
    /// answer lands in `appraisals` and opens the appraisal panel.
    pub fn appraise(&mut self, guid: u32) {
        self.session.send_action(
            ac_net::messages::action::IDENTIFY_OBJECT,
            &guid.to_le_bytes(),
        );
    }

    pub fn tick_loot(&mut self, now: Instant) {
        use ac_net::messages::action;
        let me = self.world.player_guid.unwrap_or(0);
        // What the server said was full is held against what each pack
        // holds now, before anything is judged from it.
        self.note_pack_counts();
        // The in-flight pickup is done once the item is ours, gone, or stale.
        if let Some((guid, since)) = self.loot_inflight {
            let over = match self.loot_merge.as_ref() {
                // A pour off the body onto a carried stack is answered
                // the way a tidying pour is: the target's count grows,
                // or a refusal names one of the two stacks (see
                // `pack::pour_answer`). The source going is no answer --
                // it goes before the target's new count arrives.
                Some(sent) => {
                    let (from, to) = (sent.merge.from, sent.merge.to);
                    let target_now = self.world.objects.get(&to).map(|o| o.stack_size.max(1));
                    let refusal = pack::refusal_of(
                        since,
                        self.move_refused.get(&from).copied(),
                        self.move_refused.get(&to).copied(),
                    );
                    // Waited on as long as a put would be: the server
                    // walks to the body and stoops for a merge off it
                    // exactly as for a take, and one take in ten took
                    // longer than the tidy's two seconds. Given up
                    // sooner, the coin still lying there was asked for
                    // again and a second merge went out behind the
                    // first, to fail on a stack that had gone.
                    let waited = now.saturating_duration_since(since);
                    match pack::pour_answer_within(sent, target_now, refusal, waited, TAKE_LOST) {
                        pack::PourAnswer::InAir => false,
                        pack::PourAnswer::Landed => {
                            // What the pile is for is settled now that
                            // the coin is on it, as a tidying pour's is.
                            self.autoplay.ledger.merged(from, to);
                            true
                        }
                        // The refusal is left where it is for the loot
                        // rules to read (`ac_loot::Open::refused`).
                        pack::PourAnswer::Refused(_) | pack::PourAnswer::Lost => true,
                    }
                }
                None => {
                    // Ours wherever it landed: a take named a side pack
                    // when the main one was full, and an item in a side
                    // pack is not in the character's own container.
                    let landed = self
                        .world
                        .objects
                        .get(&guid)
                        .is_none_or(|o| self.world.is_carried(guid) || o.wielder == Some(me));
                    landed || now.saturating_duration_since(since) > TAKE_LOST
                }
            };
            if over {
                self.loot_inflight = None;
            }
        }
        // What was sent (`loot_sent`, `loot_merge`) is kept past the
        // take's end, until the next goes out: the words that say why
        // a take was refused come as chat, and can come a packet behind
        // the refusal itself.
        //
        // Not while the server has us busy: it refuses the take outright
        // and spends two messages saying so (see [`Client::server_busy`]).
        // The item stays at the head of the queue and goes out on the
        // first free tick, which costs a frame and never a pickup.
        if self.loot_inflight.is_none() && !self.server_busy(now) {
            // Where it goes is decided as it is sent, from the room
            // there is now: the pack with a slot, or the carried pile
            // it pours onto (see `room::how_to_take`).
            let next = self
                .loot_queue
                .front()
                .map(|guid| (*guid, self.how_to_take(*guid)));
            // Not a pour while the tidying's own pour is in the air.
            // The tidying holds off while a take is (see
            // `autoplay::TidyGate`), and this is the other half of it:
            // the pile the coin would join may be the one the tidying is
            // emptying, or the one it is filling, whose count the two
            // answers would then be read off together. A pour within
            // the pack is answered in a tick or two, and this waits the
            // tick.
            let tidy_pouring = self.autoplay.pour.is_some();
            if let Some((guid, how)) = next
                .filter(|(_, how)| !(tidy_pouring && matches!(how, Some(room::Take::Merge { .. }))))
            {
                self.loot_queue.pop_front();
                let retried = self.loot_retry.take() == Some(guid);
                let name = self.world.name_of(guid).unwrap_or_default().to_string();
                let mut w = ac_net::wire::Writer::new();
                self.loot_merge = None;
                let into = match how {
                    Some(room::Take::Merge { to, amount }) => {
                        tracing::info!("take {name} ({guid:#010x}): pouring onto {to:#010x}");
                        let to_before = self
                            .world
                            .objects
                            .get(&to)
                            .map(|o| o.stack_size.max(1))
                            .unwrap_or(0);
                        w.u32(guid).u32(to).i32(amount as i32);
                        self.session
                            .send_action(action::STACKABLE_MERGE, &w.finish());
                        self.loot_merge = Some(pack::PourSent {
                            merge: pack::Merge {
                                from: guid,
                                to,
                                amount,
                                name,
                                frees_a_slot: true,
                            },
                            to_before,
                        });
                        to
                    }
                    put => {
                        // Nowhere the room model can find is the main
                        // pack, as every take once was: the server's
                        // answer says whether it was right.
                        let into = match put {
                            Some(room::Take::Put(into)) => into,
                            _ => me,
                        };
                        tracing::info!("take {name} ({guid:#010x}) into {into:#010x}");
                        w.u32(guid).u32(into).u32(0);
                        self.session
                            .send_action(action::PUT_ITEM_IN_CONTAINER, &w.finish());
                        into
                    }
                };
                self.loot_inflight = Some((guid, now));
                self.loot_sent = Some(TakeSent {
                    item: guid,
                    into,
                    retried,
                    said_full: false,
                    refused: None,
                });
            }
        }
    }

    /// Queue an item of the open container to be picked up.
    pub fn take(&mut self, guid: u32) {
        if !self.loot_queue.contains(&guid) && self.loot_inflight.map(|(g, _)| g) != Some(guid) {
            self.loot_queue.push_back(guid);
        }
    }

    /// Drop a carried item on the ground in front of the character
    /// (DropItem 0x001B). The server answers with InventoryPutObjectIn3D
    /// and creates the object in the world.
    pub fn drop_item(&mut self, guid: u32) -> bool {
        use ac_net::messages::action;
        let me = self.world.player_guid;
        let Some(o) = self.world.objects.get(&guid) else {
            return false;
        };
        if me.is_none() || (o.container != me && o.wielder != me) {
            return false;
        }
        tracing::info!("drop {} ({guid:#010x})", o.name);
        self.session
            .send_action(action::DROP_ITEM, &guid.to_le_bytes());
        true
    }

    /// Move a carried item into a container (PutItemInContainer 0x0019):
    /// our own guid for the main pack, a carried side pack, or the ground
    /// container we are looking into (a chest; corpses refuse).
    pub fn put_in_container(&mut self, item: u32, container: u32) -> bool {
        use ac_net::messages::action;
        let me = self.world.player_guid;
        let Some(o) = self.world.objects.get(&item) else {
            return false;
        };
        if me.is_none() || (o.container != me && o.wielder != me) || item == container {
            return false;
        }
        let name = o.name.clone();
        let target = self
            .world
            .objects
            .get(&container)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "pack".into());
        tracing::info!("put {name} ({item:#010x}) in {target} ({container:#010x})");
        // Whatever the server says about this item now is about the put,
        // not about some earlier ask to wield it: a refused put must not
        // be counted against the wield (see `Client::hold_off_wield`).
        if self.autoplay.wield_asked.is_some_and(|(g, _)| g == item) {
            self.autoplay.wield_asked = None;
        }
        let mut w = ac_net::wire::Writer::new();
        w.u32(item).u32(container).u32(0);
        self.session
            .send_action(action::PUT_ITEM_IN_CONTAINER, &w.finish());
        true
    }

    /// Put a carried item into a container standing in the world (a chest,
    /// a house hook or a storage chest). The server only takes items into
    /// an open container ("The container is closed" otherwise), so when it
    /// is not the one we are looking into this uses it first and stores
    /// the item once its contents arrive. False when the item is not ours.
    pub fn store_in(&mut self, item: u32, container: u32) -> bool {
        let me = self.world.player_guid;
        let ours = self
            .world
            .objects
            .get(&item)
            .is_some_and(|o| me.is_some() && (o.container == me || o.wielder == me));
        if !ours {
            return false;
        }
        if self.world.open_container.as_ref().map(|(g, _)| *g) == Some(container) {
            return self.put_in_container(item, container);
        }
        tracing::info!("store {item:#010x} in {container:#010x}: opening it first");
        self.pending_store = Some((item, container, Instant::now()));
        self.interact(container);
        true
    }

    /// Finish a `store_in` once its container is open; give up after a
    /// few seconds (out of reach, no permission).
    pub(crate) fn tick_store(&mut self) {
        let Some((item, container, since)) = self.pending_store else {
            return;
        };
        if self.world.open_container.as_ref().map(|(g, _)| *g) == Some(container) {
            self.pending_store = None;
            self.put_in_container(item, container);
        } else if since.elapsed() > Duration::from_secs(5) {
            tracing::info!("store {item:#010x}: {container:#010x} did not open");
            self.pending_store = None;
        }
    }

    /// Hand a carried item (or `amount` of a stack; the whole stack when
    /// None) to an NPC or another player (GiveObjectRequest 0x00CD). The
    /// target must be in reach; NPCs answer with an emote, players must
    /// allow gifts in their options.
    pub fn give(&mut self, target: u32, item: u32, amount: Option<u32>) -> bool {
        use ac_net::messages::action;
        let me = self.world.player_guid;
        let Some(o) = self.world.objects.get(&item) else {
            return false;
        };
        if me.is_none() || (o.container != me && o.wielder != me) || Some(target) == me {
            return false;
        }
        let amount = amount.unwrap_or(o.stack_size.max(1)).max(1);
        let name = o.name.clone();
        let who = self
            .world
            .objects
            .get(&target)
            .map(|t| t.name.clone())
            .unwrap_or_default();
        tracing::info!("give {amount} x {name} ({item:#010x}) to {who} ({target:#010x})");
        let mut w = ac_net::wire::Writer::new();
        w.u32(target).u32(item).u32(amount);
        self.session
            .send_action(action::GIVE_OBJECT_REQUEST, &w.finish());
        true
    }

    /// The Ust we carry, if any: the salvaging tool.
    pub fn salvage_tool(&self) -> Option<u32> {
        self.world
            .inventory()
            .find(|o| o.weenie_class_id == ac_world::material::UST_WCID)
            .map(|o| o.guid)
    }

    /// Carried, unwielded items the server would salvage: those with a
    /// material and a workmanship (loot, not vendor stock).
    pub fn salvageable(&self) -> Vec<u32> {
        let me = self.world.player_guid;
        let mut items: Vec<&ac_world::WorldObject> = self
            .world
            .inventory()
            .filter(|o| {
                o.wielder != me
                    && o.material != 0
                    && o.workmanship > 0.0
                    && !o.name.starts_with("Salvaged ")
            })
            .collect();
        items.sort_by(|a, b| a.name.cmp(&b.name).then(a.guid.cmp(&b.guid)));
        items.into_iter().map(|o| o.guid).collect()
    }

    /// Salvage carried items with the Ust (CreateTinkeringTool 0x027D:
    /// tool, count, guids). The server destroys them and answers with
    /// SalvageOperationsResult per skill used, shown in chat, and the
    /// salvage bags appear in the pack. False without an Ust or items.
    pub fn salvage(&mut self, items: &[u32]) -> bool {
        let Some(tool) = self.salvage_tool() else {
            return false;
        };
        let me = self.world.player_guid;
        // Side packs count: the server looks for each item in them too
        // (ACE `GetInventoryItem`). Leaving them out sent nothing for a
        // batch that was all in a side pack, and the autoplay, which
        // salvages one workmanship grade at a time, was stuck on it.
        let items: Vec<u32> = items
            .iter()
            .copied()
            .filter(|g| {
                self.world.is_carried(*g)
                    || self.world.objects.get(g).is_some_and(|o| o.wielder == me)
            })
            .collect();
        if items.is_empty() {
            return false;
        }
        tracing::info!("salvage {} items with {tool:#010x}", items.len());
        let mut w = ac_net::wire::Writer::new();
        w.u32(tool).u32(items.len() as u32);
        for g in &items {
            w.u32(*g);
        }
        self.session
            .send_action(ac_net::messages::action::CREATE_TINKERING_TOOL, &w.finish());
        true
    }

    /// Free item slots in one of the character's packs: the main pack
    /// when `pack` is the character, a side pack otherwise.
    ///
    /// Packs and Foci sitting in the main pack take a pack slot rather
    /// than an item slot, so they are not counted against it.
    fn free_slots_in(&self, pack: u32) -> u32 {
        let me = self.world.player_guid;
        let capacity = if Some(pack) == me {
            self.world
                .player()
                .map(|p| p.items_capacity)
                .filter(|c| *c > 0)
                .unwrap_or(102)
        } else {
            self.world
                .objects
                .get(&pack)
                .map_or(0, |o| o.items_capacity)
        };
        let used = self
            .world
            .objects
            .values()
            .filter(|o| {
                o.container == Some(pack)
                    && !(Some(pack) == me
                        && ac_world::pack_slot::used_by(
                            o.weenie_class_id,
                            o.item_type & ac_world::item_type::CONTAINER != 0,
                        ))
            })
            .count() as u32;
        capacity.saturating_sub(used)
    }

    /// A pack with room for one more item, `prefer` first: where a piece
    /// cut off a stack can land. None when every pack is full.
    fn pack_with_room(&self, prefer: Option<u32>) -> Option<u32> {
        let me = self.world.player_guid;
        prefer
            .filter(|p| self.free_slots_in(*p) > 0)
            .or_else(|| me.filter(|m| self.free_slots_in(*m) > 0))
            .or_else(|| {
                // The lowest guid among the side packs with room, so
                // that the answer does not wander between frames.
                self.world
                    .objects
                    .values()
                    .filter(|o| {
                        o.container == me
                            && o.item_type & ac_world::item_type::CONTAINER != 0
                            && self.free_slots_in(o.guid) > 0
                    })
                    .map(|o| o.guid)
                    .min()
            })
    }

    /// Split `amount` off a carried stack into a container (the pack the
    /// stack is already in when None; StackableSplitToContainer
    /// 0x0055). The server creates the new stack and updates the old
    /// one's size. False when the amount is not below the stack size,
    /// or when no pack has a slot for the piece.
    pub fn split_stack(&mut self, item: u32, container: Option<u32>, amount: u32) -> bool {
        use ac_net::messages::action;
        let me = self.world.player_guid;
        let Some(o) = self.world.objects.get(&item) else {
            return false;
        };
        // A stack in a side pack is as much in hand as one in the main
        // pack: the server looks for it anywhere the character can move
        // things from, which is what [`Self::merge_stacks`] already
        // allows. Asking only about the main pack made a pile kept in a
        // sack uncuttable and said nothing about it, and a counter that
        // will not take the pile whole then waited for a piece that
        // could never be cut -- for the length of the visit.
        if me.is_none() || !(self.world.is_carried(item) || o.wielder == me) {
            return false;
        }
        if amount == 0 || amount >= o.stack_size {
            return false;
        }
        // Where the piece goes. The server puts a split into the pack it
        // is told and no other -- `limitToMainPackOnly` -- so naming a
        // full pack is a refusal ("TryAddToInventory failed!") and not a
        // spill into a side one. Left to itself the cut stays where the
        // pile is, and only looks elsewhere if that pack is full.
        let Some(target) = container.or_else(|| self.pack_with_room(o.container)) else {
            return false;
        };
        tracing::info!(
            "split {} off {} ({item:#010x}) into {target:#010x}",
            amount,
            o.name
        );
        let mut w = ac_net::wire::Writer::new();
        w.u32(item).u32(target).i32(0).i32(amount as i32);
        self.session
            .send_action(action::STACKABLE_SPLIT_TO_CONTAINER, &w.finish());
        true
    }

    /// Drop `amount` of a carried stack on the ground
    /// (StackableSplitTo3D 0x0056); the whole stack goes with `drop_item`.
    pub fn split_to_ground(&mut self, item: u32, amount: u32) -> bool {
        use ac_net::messages::action;
        let me = self.world.player_guid;
        let Some(o) = self.world.objects.get(&item) else {
            return false;
        };
        if me.is_none() || o.container != me || amount == 0 || amount >= o.stack_size {
            return false;
        }
        tracing::info!("drop {} of {} ({item:#010x})", amount, o.name);
        let mut w = ac_net::wire::Writer::new();
        w.u32(item).i32(amount as i32);
        self.session
            .send_action(action::STACKABLE_SPLIT_TO_3D, &w.finish());
        true
    }

    /// Merge a carried stack into another of the same kind, and settle
    /// what the surviving stack was taken for.
    ///
    /// This is the way in for anything that asks for a merge and does
    /// not follow it up: a panel's drag, a script, the shopping. The
    /// rules' own tidying sends it with `Self::send_merge` and settles
    /// the ledger when the server says the pour landed, since a refused
    /// pour settles nothing.
    pub fn merge_stacks(&mut self, from: u32, to: u32, amount: Option<u32>) -> bool {
        if !self.send_merge(from, to, amount) {
            return false;
        }
        // While both entries are still there to read: the source's goes
        // when the thing itself does.
        self.autoplay.ledger.merged(from, to);
        true
    }

    /// The merge itself (StackableMerge 0x0054: from, to, amount; the
    /// whole source when `amount` is None). The server caps at the
    /// target's maximum stack and leaves the rest in the source.
    pub(crate) fn send_merge(&mut self, from: u32, to: u32, amount: Option<u32>) -> bool {
        use ac_net::messages::action;
        let me = self.world.player_guid;
        let (Some(a), Some(b)) = (self.world.objects.get(&from), self.world.objects.get(&to))
        else {
            return false;
        };
        // Either stack may sit in a side pack: the server searches
        // everywhere the character can move things from, and a purchase
        // that landed in the main pack often belongs with a pile kept
        // in a side one. The target may also be in hand, which is how a
        // quiver gets topped up; the source may not, since emptying
        // what the character is holding is not tidying.
        let to_in_hand = b.wielder == me;
        if me.is_none()
            || from == to
            || !self.world.is_carried(from)
            || !(self.world.is_carried(to) || to_in_hand)
            || a.weenie_class_id != b.weenie_class_id
            || b.max_stack_size <= 1
        {
            return false;
        }
        let amount = amount.unwrap_or(a.stack_size).clamp(1, a.stack_size);
        tracing::info!(
            "merge {} of {} ({from:#010x}) into {to:#010x}",
            amount,
            a.name
        );
        let mut w = ac_net::wire::Writer::new();
        w.u32(from).u32(to).i32(amount as i32);
        self.session
            .send_action(action::STACKABLE_MERGE, &w.finish());
        true
    }

    /// Apply a carried item to a target (UseWithTarget 0x0035): a healing
    /// kit on yourself or a fellow, a mana stone on an item, a key or
    /// lockpick on a chest or door. The server walks us into reach first.
    pub fn use_on(&mut self, item: u32, target: u32) -> bool {
        use ac_net::messages::action;
        let me = self.world.player_guid;
        let Some(o) = self.world.objects.get(&item) else {
            return false;
        };
        // Anything carried will do, a side pack included: a bundle of
        // arrowheads lives in one as often as not.
        if me.is_none() || (!self.world.is_carried(item) && o.wielder != me) {
            return false;
        }
        let what = o.name.clone();
        let who = self
            .world
            .objects
            .get(&target)
            .map(|t| t.name.clone())
            .unwrap_or_default();
        tracing::info!("use {what} ({item:#010x}) on {who} ({target:#010x})");
        self.last_used = Some(target);
        let mut w = ac_net::wire::Writer::new();
        w.u32(item).u32(target);
        self.session
            .send_action(action::USE_WITH_TARGET, &w.finish());
        true
    }
}

/// The chat line for a salvage result: "You obtain 3 Oak (workmanship
/// 8.00) using your Salvaging skill." per material.
pub fn salvage_text(res: &ac_net::messages::SalvageResult) -> String {
    let skill = match res.skill {
        40 => "Salvaging",
        18 => "Item Tinkering",
        28 => "Weapon Tinkering",
        29 => "Armor Tinkering",
        30 => "Magic Item Tinkering",
        _ => "salvaging",
    };
    if res.yields.is_empty() {
        return format!("You salvaged nothing using your {skill} skill.");
    }
    let parts: Vec<String> = res
        .yields
        .iter()
        .map(|y| {
            format!(
                "{} {} (workmanship {:.2})",
                y.units,
                ac_world::material::name(y.material),
                y.workmanship
            )
        })
        .collect();
    let mut text = format!("You obtain {} using your {skill} skill.", parts.join(", "));
    if res.bonus_percent > 0 {
        text.push_str(&format!(" ({}% from augmentations)", res.bonus_percent));
    }
    if !res.skipped.is_empty() {
        text.push_str(&format!(
            " {} item(s) could not be salvaged.",
            res.skipped.len()
        ));
    }
    text
}

#[cfg(test)]
mod tests;
