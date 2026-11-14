//! Giving: one ask hands a run of carried items to one receiver, a give (GiveObjectRequest) at a
//! time, each sent only once the last is answered -- the pack count falls, or the server refuses
//! by code (InventoryServerSaveFailed) or in words (`hear_gift_refusal`) -- and the busy slot is
//! free (Player_Inventory.cs:3201-3203). How much one give takes is the receiver's: a Give emote
//! takes one whatever the amount, a taker or a player the amount asked (:3435-3443).

use std::time::{Duration, Instant};

use ac_world::quests::Want;

use crate::profile::{GiveTo, LootAction};
use crate::refusals::{Answer, GiftRefusal, Refusal};
use crate::{Client, Goal, Owner};

/// A receiver this near (metres, 3D) is in reach of a give: the town run's giver reach.
pub(crate) const GIVE_REACH: f32 = crate::autoplay::growth::GIVER_REACH;
/// A give the server has not answered in this long is given up: a backstop only, as every ACE
/// give path answers; ACE's own walk to the receiver gives up at 15 s (Player_Move.cs:16).
const ANSWER_LOST: Duration = Duration::from_secs(20);
/// ACE's `AttunedItem` (0x0426): an attuned item to a player (Player_Inventory.cs:3280-3283).
const ATTUNED_ITEM: u32 = 0x0426;
/// ACE's `TradeAiRefuseEmote` (0x04CD): an NPC's Refuse emote (Player_Inventory.cs:3465).
const REFUSE_EMOTE: u32 = 0x04CD;
/// PropertyInt Attuned (114), an assessment property; 2 is Sticky (AttunedStatus.cs).
const ATTUNED: u32 = 114;
const STICKY: i32 = 2;

/// Who an item would go to, as far as the client knows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    /// A player character: an attuned item is refused (Player_Inventory.cs:3280).
    Player,
    /// An NPC: one of `ac_world::takers`, and whether a quest row says its Give emote wants it.
    Npc {
        takes_anything: bool,
        wants_it: bool,
    },
}

/// Whether ACE lets an item of Attuned `attuned` (`None` unknown) go to `to`: NPCs never check
/// it but a taker a sticky one (Player_Inventory.cs:3435-3439); the unknown is the server's call.
pub fn may_give(to: Receiver, attuned: Option<i32>) -> bool {
    match to {
        Receiver::Player => attuned.is_none_or(|a| a == 0),
        Receiver::Npc { wants_it: true, .. } => true,
        Receiver::Npc {
            takes_anything: true,
            ..
        } => attuned != Some(STICKY),
        Receiver::Npc { .. } => true,
    }
}

/// A run of gives to one receiver (`Client.giving`).
#[derive(Clone, Debug)]
pub struct Giving {
    pub to: u32,
    pub to_name: String,
    /// The stacks still to give, in order.
    pub items: Vec<u32>,
    /// How many still to give in all; `None` for every one.
    pub left: Option<u32>,
    pub given: u32,
    /// A player's ask: walks into reach first, and ends when the player moves. Autoplay asks
    /// only from within reach.
    pub by_hand: bool,
    sent: Option<Sent>,
    /// A busy answer's wait: the same give goes again after it.
    again_at: Option<Instant>,
    /// Busy answers in a row.
    busy: u32,
    /// The receiver's refusal in words since the give in the air, and the table's answer.
    heard: Option<(String, Answer)>,
}

/// The give in the air: the stack, its size then, and when.
#[derive(Clone, Copy, Debug)]
struct Sent {
    item: u32,
    stack: u32,
    at: Instant,
}

/// How a run of gives ended (`Client.gave`).
#[derive(Clone, Debug, PartialEq)]
pub enum GiveEnd {
    /// Everything asked went, or nothing of it is left.
    Done,
    /// The receiver refused; `answer` is the refusals table's.
    Refused { why: String, answer: Answer },
    /// It could not go on from here: out of view, out of reach, no way there, no answer.
    Stopped(String),
}

/// The last run of gives, ended: to whom, how many, and how.
#[derive(Clone, Debug, PartialEq)]
pub struct Gave {
    pub to: u32,
    pub given: u32,
    pub end: GiveEnd,
}

impl Client {
    /// Give `most` (every one with `None`) of the carried items of weenie class `wcid`, stacks in
    /// pack order, to `to`, walking into reach first. False when none is carried or `to` is unseen.
    pub fn give_all(&mut self, to: u32, wcid: u32, most: Option<u32>) -> bool {
        let mut items: Vec<u32> = self
            .world
            .inventory()
            .filter(|o| o.weenie_class_id == wcid && o.wielder.is_none())
            .map(|o| o.guid)
            .collect();
        items.sort_unstable();
        self.start_giving(to, items, most, true)
    }

    /// Give these carried items to `to`, from where the character stands (autoplay's ask).
    pub(crate) fn give_items(&mut self, to: u32, items: Vec<u32>) -> bool {
        self.start_giving(to, items, None, false)
    }

    fn start_giving(&mut self, to: u32, items: Vec<u32>, left: Option<u32>, by_hand: bool) -> bool {
        let Some(name) = self.world.objects.get(&to).map(|o| o.name.clone()) else {
            return false;
        };
        if items.is_empty() || Some(to) == self.world.player_guid || left == Some(0) {
            return false;
        }
        tracing::info!("giving {} stack(s) to {name} ({to:#010x})", items.len());
        self.giving = Some(Giving {
            to,
            to_name: name,
            items,
            left,
            given: 0,
            by_hand,
            sent: None,
            again_at: None,
            busy: 0,
            heard: None,
        });
        true
    }

    /// The player taking the controls ends a give they asked for; autoplay's gives stand.
    pub(crate) fn hand_off_giving(&mut self) {
        if self.giving.as_ref().is_some_and(|g| g.by_hand) {
            self.stop_giving("the player moved");
        }
    }

    /// Stop the gives under way, saying `why`.
    pub fn stop_giving(&mut self, why: &str) {
        if let Some(g) = self.giving.as_ref() {
            let end = GiveEnd::Stopped(why.to_string());
            self.end_giving(g.to, g.given, end);
        }
    }

    /// The server's words on a give (`hear_refusal`): kept for the run if they name its receiver,
    /// or, mid-emote, which names nobody, if a give is in the air.
    pub(crate) fn hear_gift_refusal(&mut self, refusal: &Refusal, answer: Answer) {
        let Some(g) = self.giving.as_mut().filter(|g| g.sent.is_some()) else {
            return;
        };
        let to = g.to_name.as_str();
        let why = match refusal {
            Refusal::Give => format!("{to} is in the middle of something"),
            Refusal::Gift { name, why } if *name == to => match why {
                GiftRefusal::NotAccepting => format!("{to} is not accepting gifts"),
                GiftRefusal::Busy => format!("{to} is too busy to accept gifts"),
                GiftRefusal::NotWanted => format!("{to} does not want it"),
                GiftRefusal::Examined => format!("{to} looked it over and handed it back"),
            },
            _ => return,
        };
        g.heard = Some((why, answer));
    }

    /// Who `to` is to a give of `item`, from what the client knows of both.
    fn receiver(&self, to: u32, item: u32) -> Receiver {
        let target = self.world.objects.get(&to);
        if target.is_some_and(|o| o.is_character()) {
            return Receiver::Player;
        }
        let giver = target.map_or(0, |o| o.weenie_class_id);
        let wcid = self
            .world
            .objects
            .get(&item)
            .map_or(0, |o| o.weenie_class_id);
        let wants_it = ac_world::quests::all()
            .iter()
            .any(|r| r.giver_wcid == giver && r.want == Want::Item(wcid));
        Receiver::Npc {
            takes_anything: ac_world::takers::takes_anything(giver),
            wants_it,
        }
    }

    /// How many of `item` the pack holds: its stack, or none once it has left.
    fn stack_held(&self, item: u32) -> u32 {
        match self.world.objects.get(&item) {
            Some(o) if self.world.is_carried(item) => o.stack_size.max(1),
            _ => 0,
        }
    }

    fn end_giving(&mut self, to: u32, given: u32, end: GiveEnd) {
        let name = self.giving.take().map_or_else(String::new, |g| g.to_name);
        let said = match &end {
            GiveEnd::Done => format!("gave {given} to {name}"),
            GiveEnd::Refused { why, .. } => format!("gave {given} to {name}, then refused: {why}"),
            GiveEnd::Stopped(why) => format!("gave {given} to {name}, then stopped: {why}"),
        };
        self.autoplay.note(said, self.frame_at.max(Instant::now()));
        self.gave = Some(Gave { to, given, end });
    }

    /// One turn of the gives under way: read the answer to the last, then send the next.
    pub(crate) fn tick_giving(&mut self, now: Instant) {
        let Some(mut g) = self.giving.take() else {
            return;
        };
        let turn = self.giving_turn(&mut g, now);
        let (to, given) = (g.to, g.given);
        self.giving = Some(g);
        if let Some(end) = turn {
            self.end_giving(to, given, end);
        }
    }

    /// The turn itself; an end when the run is over.
    fn giving_turn(&mut self, g: &mut Giving, now: Instant) -> Option<GiveEnd> {
        if let Some(sent) = g.sent {
            let held = self.stack_held(sent.item);
            let refused = self
                .move_refused
                .get(&sent.item)
                .filter(|(_, at)| *at >= sent.at)
                .map(|(err, _)| *err);
            if held < sent.stack {
                let n = sent.stack - held;
                g.given += n;
                g.left = g.left.map(|l| l.saturating_sub(n));
                (g.sent, g.busy, g.heard) = (None, 0, None);
            } else if let Some((why, answer)) = g.heard.take() {
                g.sent = None;
                match answer {
                    Answer::Soon(wait) if g.busy < crate::autoplay::growth::GIVE_BUSY_TRIES => {
                        g.busy += 1;
                        g.again_at = Some(now + wait);
                        return None;
                    }
                    // The receiver's answer stands for the rest of the run too.
                    _ => {
                        g.items.iter().for_each(|&i| self.give_refused(i, &why));
                        return Some(GiveEnd::Refused { why, answer });
                    }
                }
            } else if let Some(err) = refused {
                g.sent = None;
                if err == ATTUNED_ITEM {
                    // That one stays; the rest may go (Player_Inventory.cs:3280-3283).
                    self.give_refused(sent.item, "attuned: no player can be given it");
                    g.items.retain(|&i| i != sent.item);
                } else {
                    let why = match err {
                        REFUSE_EMOTE => format!("{} handed it back", g.to_name),
                        _ => format!("{} took none (refused, error {err:#x})", g.to_name),
                    };
                    g.items.iter().for_each(|&i| self.give_refused(i, &why));
                    return Some(GiveEnd::Refused {
                        why,
                        answer: Answer::Never,
                    });
                }
            } else if now.saturating_duration_since(sent.at) < ANSWER_LOST {
                return None;
            } else {
                return Some(GiveEnd::Stopped("the server never answered".into()));
            }
        }
        if g.again_at.is_some_and(|t| now < t) {
            return None;
        }
        g.items.retain(|&i| self.stack_held(i) > 0);
        while let Some(&item) = g.items.first() {
            let attuned = self.appraisals.get(&item).and_then(|a| a.int(ATTUNED));
            if may_give(self.receiver(g.to, item), attuned) {
                break;
            }
            self.give_refused(item, "attuned: the receiver cannot take it");
            g.items.remove(0);
        }
        let Some(&item) = g.items.first().filter(|_| g.left != Some(0)) else {
            return Some(GiveEnd::Done);
        };
        let (Some(me), Some(there)) = (
            self.my_position(),
            self.world.objects.get(&g.to).and_then(|o| o.world_pos()),
        ) else {
            return Some(GiveEnd::Stopped(format!("{} is not in view", g.to_name)));
        };
        let toward = Goal::Near {
            guid: g.to,
            within: crate::visit::REACH,
        };
        if me.distance(there) > GIVE_REACH {
            if !g.by_hand {
                return Some(GiveEnd::Stopped(format!("{} is out of reach", g.to_name)));
            }
            let did = self.go(Owner::Hand, toward, now);
            return match did.because() {
                Some(why) if !did.fine() => Some(GiveEnd::Stopped(format!("no way: {why}"))),
                _ => None,
            };
        }
        if self
            .going
            .as_ref()
            .is_some_and(|w| w.owner == Owner::Hand && w.goal.is_same(toward))
        {
            self.stop_going(Some(Owner::Hand), "in reach to give");
        }
        if self.hands_busy(now) {
            return None;
        }
        let stack = self.stack_held(item);
        let amount = g.left.map_or(stack, |l| l.min(stack));
        if !self.give(g.to, item, Some(amount)) {
            return Some(GiveEnd::Stopped("that cannot be given".into()));
        }
        g.sent = Some(Sent {
            item,
            stack,
            at: now,
        });
        g.heard = None;
        None
    }

    /// A give tagged in the ledger turned away: held against the item for the ledger's wait.
    fn give_refused(&mut self, item: u32, why: &str) {
        if self.autoplay.ledger.by_guid(item) == Some(LootAction::Give) {
            self.autoplay
                .ledger
                .failed(item, why, crate::holdings::unix_now());
        }
        // Whatever its tag: a mule's load or delivery does not ask again.
        if !self.autoplay.muling.refused.contains(&item) {
            self.autoplay.muling.refused.push(item);
        }
        tracing::info!("give of {item:#010x} refused: {why}");
    }

    /// The carried, unworn items the ledger tagged Give under a rule whose receiver `fits`,
    /// none held back by a refusal lately.
    pub(crate) fn gifts_tagged(&self, fits: impl Fn(&GiveTo) -> bool) -> Vec<(u32, GiveTo)> {
        let Some(profile) = self.loot_profile() else {
            return Vec::new();
        };
        let unix = crate::holdings::unix_now();
        let ledger = &self.autoplay.ledger;
        let mut out: Vec<(u32, GiveTo)> = self
            .world
            .inventory()
            .filter(|o| o.wielder.is_none())
            .filter(|o| ledger.by_guid(o.guid) == Some(LootAction::Give))
            .filter(|o| ledger.why_failed(o.guid, unix).is_none())
            .filter_map(|o| {
                let rule = profile.rule_named(ledger.why(o.guid)?)?;
                let to = rule.give_to.clone().filter(|t| fits(t))?;
                Some((o.guid, to))
            })
            .collect();
        out.sort_by_key(|(g, _)| *g);
        out
    }
}

#[cfg(test)]
mod tests;
