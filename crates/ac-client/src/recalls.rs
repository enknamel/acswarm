//! Recall spells from the character's side: which of them it can cast
//! right now and where each would land, for the trip planner
//! (`ac_world::trip::plan_with_recalls`), and the positions those spells
//! go to.
//!
//! A named recall (Aerlinthe Recall, Ulgrim's Recall...) goes to a fixed
//! place the world data knows (`ac_world::recalls::fixed`). Lifestone
//! Sending, Lifestone Recall, Portal Recall and the two Portal Recalls
//! go to positions the server saves for the character and never sends,
//! so they are learnt here from what the character does: using a
//! lifestone (the server says the spirit is attuned), tying one or a
//! portal (it says the link succeeded), walking through a portal (the
//! journey notices). Until one of those has happened in this session the
//! spell's destination is unknown and the planner leaves it alone. A
//! recall the server refuses forgets the position it was aimed at.

use ac_world::object::Position;
use ac_world::recalls::{self, position_type};
use ac_world::trip::Recall;
use glam::Vec3;

use crate::magic::CastCheck;
use crate::Client;

/// A lifestone or portal this close to the character is the one it
/// just used or tied (the server's use radius is about as much).
const NEXT_TO: f32 = 12.0;
/// How long after a recall is asked for a teleport is taken for its landing: a cast and
/// /lifestone's own wait come well inside it.
const RECALL_LANDS: std::time::Duration = std::time::Duration::from_secs(60);
/// Metres from where a recall was asked that a landing must be: less is a server correction.
const RECALL_MOVES: f32 = 30.0;

/// One learnt saved position as kept on disk between sessions; ACE sends only the last death's
/// at login (GameEventPlayerDescription.cs:202-210), so the rest would be lost at every logoff.
#[derive(serde::Serialize, serde::Deserialize)]
struct Kept {
    kind: u32,
    cell: u32,
    local: [f32; 3],
}

impl Client {
    /// Send one of the free recalls. Answers in the same words a cast
    /// does, so the journey need not care which kind it asked for.
    pub(crate) fn send_free_recall(&mut self, spell: u32) -> CastCheck {
        use ac_net::messages::action;
        let what = match spell {
            recalls::spell::FREE_LIFESTONE => action::TELE_TO_LIFESTONE,
            recalls::spell::FREE_MARKETPLACE => action::TELE_TO_MARKETPLACE,
            _ => return CastCheck::NotKnown,
        };
        if spell == recalls::spell::FREE_LIFESTONE {
            self.note_recall_sent(position_type::SANCTUARY);
        }
        // The server refuses it in combat, so drop out first: the
        // refusal costs a round trip and says nothing useful.
        if self.combat {
            self.toggle_combat();
        }
        tracing::info!("travel: asking for {spell:#x} (no spell needed)");
        self.session.send_action(what, &[]);
        CastCheck::Ok
    }

    /// The ways out that need no spell at all.
    ///
    /// `/lifestone` is not magic: the server takes it as its own action
    /// and asks only that a lifestone has been attuned. Every character
    /// has it from the first minute, which matters because the
    /// characters most likely to be stuck at the bottom of a dungeon
    /// are exactly the ones too junior to have learnt Lifestone Recall
    /// or Lifestone Sending. A level twelve character with no spells
    /// and no gems still has this, and planning as though it did not
    /// left one standing in Holtburg Dungeon walking into a wall.
    ///
    pub fn free_recalls(&self) -> Vec<Recall> {
        let mut out = Vec::new();
        // Where /lifestone puts the character: the lifestone last
        // attuned, which is what the server calls the sanctuary.
        if let Some(p) = self.world.stats.recall_position(position_type::SANCTUARY) {
            out.push(Recall {
                spell: recalls::spell::FREE_LIFESTONE,
                name: "/lifestone".to_string(),
                exit: ac_world::Place::of_position(&p),
            });
        }
        // `/marketplace` is the same kind of thing and lands among a
        // town's worth of counters, but nothing here knows where the
        // Marketplace is: it is not in the recall table, which holds
        // only the spells. Left out rather than guessed at.
        out
    }

    /// The recall spells the character can cast right now (known, with
    /// a caster in hand, the components and the mana) and where each
    /// would land; the trip planner's input. A spell whose destination
    /// is not known yet is left out.
    pub fn castable_recalls(&self) -> Vec<Recall> {
        let mut out = self.free_recalls();
        for &id in &self.world.stats.spells {
            let (name, exit) = if let Some(kind) = recalls::dynamic(id) {
                let Some(p) = self.world.stats.recall_position(kind) else {
                    continue;
                };
                (self.recall_name(id), ac_world::Place::of_position(&p))
            } else if let Some(r) = recalls::fixed(id) {
                let exit = ac_world::Place {
                    at: r.at,
                    cell: r.cell,
                };
                (r.name.clone(), exit)
            } else {
                continue;
            };
            if !matches!(self.can_cast_well(id), CastCheck::Ok) {
                continue;
            }
            out.push(Recall {
                spell: id,
                name,
                exit,
            });
        }
        out
    }

    /// The portal gems in the pack, as ways to get somewhere.
    ///
    /// A gem needs no skill, no components and no mana: carrying it is
    /// the whole requirement, which makes it the cheapest hop a
    /// character has and often the only one to a place with no portal
    /// near it. It is spent when used, so a journey uses each once.
    pub fn carried_gems(&self) -> Vec<ac_world::trip::Gem> {
        self.world
            .inventory()
            .filter_map(|o| {
                let g = ac_world::gems::of(o.weenie_class_id)?;
                Some(ac_world::trip::Gem {
                    guid: o.guid,
                    name: g.name.clone(),
                    exit: ac_world::Place {
                        at: g.at,
                        cell: g.cell,
                    },
                    summons: g.how() == ac_world::gems::Use::Summons,
                })
            })
            .collect()
    }

    /// Where a recall spell would land the character, if known: for a
    /// map, or a script deciding whether to cast it.
    pub fn recall_destination(&self, spell: u32) -> Option<Position> {
        if let Some(kind) = recalls::dynamic(spell) {
            return self.world.stats.recall_position(kind);
        }
        let r = recalls::fixed(spell)?;
        Some(Position::new_flat(
            r.cell,
            r.at - ac_world::landblock_origin(r.cell),
        ))
    }

    fn recall_name(&self, spell: u32) -> String {
        self.spell(spell)
            .map(|s| s.name)
            .or_else(|| self.known_spells.get(&spell).cloned())
            .unwrap_or_else(|| format!("spell {spell}"))
    }

    /// Where the character stands, as a saved position.
    fn standing(&self) -> Option<Position> {
        let pl = self.player.as_ref()?;
        Some(Position::new_flat(pl.cell, pl.local))
    }

    /// Record a saved position the client has worked out.
    pub fn learn_recall_position(&mut self, kind: u32, p: Position) {
        tracing::info!(
            "recall: position {kind} is now {:#010x} {:?}",
            p.cell,
            p.local
        );
        self.world.stats.set_recall_position(kind, p);
        self.save_recall_positions();
    }

    /// Where this character's learnt saved positions are kept: beside its loot ledger, by server.
    fn recalls_path(&self) -> Option<std::path::PathBuf> {
        let name = self.world.stats.name.trim();
        if name.is_empty() {
            return None;
        }
        let beside = self.profiles.dir().parent()?.to_path_buf();
        Some(
            beside
                .join("recalls")
                .join(ac_store::file_safe(&self.config.host))
                .join(format!("{}.json", ac_store::file_safe(name))),
        )
    }

    /// Read back the saved positions learnt in earlier sessions, at login. One the server has
    /// sent already is the server's.
    pub(crate) fn load_recall_positions(&mut self) {
        let Some(path) = self.recalls_path() else {
            return;
        };
        let kept: Vec<Kept> = match ac_store::read_json(&path) {
            Ok(Some(k)) => k,
            Ok(None) => return,
            Err(e) => {
                tracing::warn!("recall: reading {}: {e}", path.display());
                return;
            }
        };
        for k in kept {
            if self.world.stats.recall_position(k.kind).is_none() {
                let p = Position::new_flat(k.cell, Vec3::from(k.local));
                self.world.stats.set_recall_position(k.kind, p);
                tracing::info!(
                    "recall: position {} read back: {:#010x} {:?}",
                    k.kind,
                    k.cell,
                    k.local
                );
            }
        }
    }

    /// Write out every saved position known, so the next session plans with them.
    fn save_recall_positions(&self) {
        let Some(path) = self.recalls_path() else {
            return;
        };
        let kept: Vec<Kept> = self
            .world
            .stats
            .positions
            .iter()
            .map(|(kind, p)| Kept {
                kind: *kind,
                cell: p.cell,
                local: p.local.to_array(),
            })
            .collect();
        if let Err(e) = ac_store::write_json_atomic(&path, &kept, ac_store::Visibility::Normal) {
            tracing::warn!("recall: writing {}: {e}", path.display());
        }
    }

    /// A spell was sent: remember a tie, so the "successfully linked"
    /// that follows is filed under the right saved position.
    pub(crate) fn note_cast(&mut self, spell: u32) {
        if recalls::tie_sets(spell).is_some() {
            self.travel.last_tie = Some(spell);
        }
        if let Some(kind) = recalls::dynamic(spell) {
            self.note_recall_sent(kind);
        }
    }

    /// A recall to the saved position `kind` was asked for, by a spell or `/lifestone`, from the
    /// journey, a key or the chat box: where it lands is that position (see [`Self::recall_landed`]).
    pub(crate) fn note_recall_sent(&mut self, kind: u32) {
        if let Some(at) = self.my_position() {
            self.travel.recall_sent = Some((kind, at, std::time::Instant::now()));
        }
    }

    /// The server put us somewhere: if a recall was asked for lately and this is far from where it
    /// was asked, it landed here, and that is where it goes from now on.
    pub(crate) fn recall_landed(&mut self) {
        let Some((kind, from, sent)) = self.travel.recall_sent else {
            return;
        };
        if sent.elapsed() > RECALL_LANDS {
            self.travel.recall_sent = None;
            return;
        }
        // A teleport while the journey goes through a portal or a gem is theirs, not the recall's
        // (a_portal_taken_while_a_recall_is_under_way_is_not_where_it_lands).
        if self.is_portal_step() {
            return;
        }
        let (Some(here), Some(at)) = (self.standing(), self.my_position()) else {
            return;
        };
        if at.distance(from) < RECALL_MOVES {
            return;
        }
        self.travel.recall_sent = None;
        self.learn_recall_position(kind, here);
    }

    /// A line of chat from the server: the ones that mean a saved
    /// position changed. Using a lifestone attunes the spirit to it;
    /// Lifestone Tie and the Portal Ties say the link succeeded.
    pub(crate) fn recall_notice(&mut self, text: &str) {
        let lower = text.to_lowercase();
        // The server's word on a /lifestone however it was asked (retail's "is recalling to the
        // lifestone"): where it lands is the lifestone.
        // Anyone in range is told of a recall (Player_Location.cs:169): only our own counts.
        let me = self.world.stats.name.to_lowercase();
        let ours = !me.is_empty() && lower.starts_with(&format!("{me} is recalling"));
        if ours
            && lower.contains("is recalling to the lifestone")
            && self.travel.recall_sent.is_none()
        {
            self.note_recall_sent(position_type::SANCTUARY);
        }
        if lower.contains("attuned your spirit") || lower.contains("resurrection point") {
            // Only next to a lifestone: the words alone could be anything.
            let Some(here) = self.standing() else {
                return;
            };
            let at = (ac_world::landblock_origin(here.cell) + here.local).truncate();
            let near = ac_world::landmarks::nearest_lifestone(at)
                .is_some_and(|l| l.xy().distance(at) <= NEXT_TO * 2.0);
            if near || self.nearest_of(ac_world::item_type::LIFESTONE).is_some() {
                self.learn_recall_position(position_type::SANCTUARY, here);
            }
        } else if lower.contains("successfully linked with the life stone") {
            if let Some(here) = self.standing() {
                self.learn_recall_position(position_type::LINKED_LIFESTONE, here);
            }
            self.travel.last_tie = None;
        } else if lower.contains("successfully linked with the portal") {
            let Some(kind) = self.travel.last_tie.take().and_then(recalls::tie_sets) else {
                return;
            };
            // The portal just tied is the one beside us; the recall
            // lands where it leads.
            let Some(mouth) = self.nearest_of(ac_world::item_type::PORTAL) else {
                tracing::info!("recall: linked with a portal, but none is in view");
                return;
            };
            let exit = ac_world::portals::near(mouth.truncate(), NEXT_TO)
                .first()
                .map(|p| (p.to_cell, p.to));
            match exit {
                Some((cell, to)) => {
                    let p = Position::new_flat(cell, to - ac_world::landblock_origin(cell));
                    self.learn_recall_position(kind, p);
                }
                None => tracing::info!("recall: the portal tied at {mouth:?} is not in the table"),
            }
        }
    }

    /// The server refused something: a recall aimed at a position it
    /// does not have, or a fizzle. The journey handles the retry; here
    /// the position the spell was aimed at is forgotten, since the
    /// server says there is nothing there.
    pub(crate) fn recall_error(&mut self, code: u32) {
        let kind = match code {
            // YouMustLinkToLifestoneToRecall
            0x049E => Some(position_type::LINKED_LIFESTONE),
            // YouMustLinkToPortalToRecall, YouCannotRecallPortal: which
            // portal recall it was is whatever the journey is casting.
            0x04A3 | 0x04AD => self.travel_recall_spell().and_then(recalls::dynamic),
            _ => None,
        };
        if let Some(kind) = kind {
            tracing::info!("recall: the server has no position {kind}; forgetting it");
            self.world.stats.clear_recall_position(kind);
            self.save_recall_positions();
            self.travel_recall_refused();
        }
        // YouHaveMovedTooFar (Player_Location.cs:187): the recall was called off, nothing lands.
        if code == 0x0498 {
            self.travel.recall_sent = None;
        }
        // YourSpellFizzled: try again straight away rather than wait.
        if code == 0x0402 {
            self.travel_recall_fizzled();
        }
    }

    /// The world position of the nearest object of an item type in
    /// view, within [`NEXT_TO`] of the character.
    fn nearest_of(&self, item_type: u32) -> Option<Vec3> {
        let me = self.my_position()?;
        self.world
            .objects
            .values()
            .filter(|o| o.item_type & item_type != 0)
            .filter_map(|o| {
                o.position
                    .map(|p| ac_world::landblock_origin(p.cell) + p.local)
            })
            .filter(|p| p.truncate().distance(me.truncate()) <= NEXT_TO)
            .min_by(|a, b| {
                a.truncate()
                    .distance(me.truncate())
                    .total_cmp(&b.truncate().distance(me.truncate()))
            })
    }
}

#[cfg(test)]
mod tests;
