use std::time::{Duration, Instant};

use crate::autoplay::{Doing, Mate, GIVE_REACH};
use crate::Client;

/// Least time between two hand-offs, and between two salvage batches
/// when the first has not been seen to go: a batch whose items have
/// left the pack is followed by the next at once.
const SALVAGE_EVERY: Duration = Duration::from_secs(3);

/// How long a salvage batch or a hand-off is given to take effect (the
/// items leaving the pack) before it counts as refused.
pub(crate) const SALVAGE_TIMEOUT: Duration = Duration::from_secs(5);

/// A batch or an item refused this many times is left alone.
pub(crate) const SALVAGE_TRIES: u8 = 3;

/// The salvager is walked to when within this; further off, the salvage
/// waits for the team to come together.
const HAND_OFF_RANGE: f32 = 30.0;

/// The Salvaging skill.
const SALVAGING: u32 = 40;

/// Who salvages for the team, out of `mates` (the caller includes
/// itself): the highest Salvaging among those with an Ust, ties to the
/// name that sorts first. `None` when nobody carries an Ust.
pub fn best_salvager<'a>(mates: impl Iterator<Item = &'a Mate>) -> Option<(String, u32)> {
    mates
        .filter(|m| m.has_ust && m.guid != 0)
        .max_by(|a, b| {
            a.salvaging
                .cmp(&b.salvaging)
                .then_with(|| b.name.cmp(&a.name))
        })
        .map(|m| (m.name.clone(), m.guid))
}

/// Where an item stands for salvaging, by its workmanship.
///
/// The server puts everything of one material salvaged in one go into
/// the same bag, and the bag's workmanship is the average of what went
/// in (ACE `TryAddSalvage`); a bag already carried is never added to.
/// So a workmanship 10 Iron mace salvaged beside a workmanship 6 one
/// makes a bag of 8, and the 10 is wasted. Skill cannot make up for it:
/// it decides how many units come out, never their workmanship. Below 9
/// nobody minds the averaging and everything goes in together; a 9 is
/// salvaged only with 9s, and a 10 only with 10s.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum SalvageGrade {
    /// Below workmanship 9.
    Common,
    Nine,
    Ten,
}

impl SalvageGrade {
    fn of(workmanship: f32) -> Self {
        if workmanship >= 10.0 {
            Self::Ten
        } else if workmanship >= 9.0 {
            Self::Nine
        } else {
            Self::Common
        }
    }
}

/// One salvage's worth out of `items` (guid, workmanship, and how many
/// times a salvage of it has come to nothing, in the order they are to
/// go), and the grade they share. `None` with nothing to salvage.
///
/// What has come to nothing least goes first, and of that every 10 when
/// there is one, else every 9, else the rest. The best go first, so that
/// ordinary loot turning up between batches never keeps them waiting;
/// each grade is a batch, and so a turn, of its own.
///
/// The refusals come before the grade because ACE skips some items
/// without a word -- a Retained one, say. Chosen as the best grade every
/// time, a 10 like that went out alone again after each timeout, and the
/// 9s and everything below waited behind three of them, where a single
/// salvage of everything used to take the rest at once.
fn next_salvage_batch(
    items: impl IntoIterator<Item = (u32, f32, u8)>,
) -> Option<(SalvageGrade, Vec<u32>)> {
    use std::cmp::Reverse;
    let turns: Vec<(u32, (Reverse<u8>, SalvageGrade))> = items
        .into_iter()
        .map(|(guid, workmanship, refused)| {
            (guid, (Reverse(refused), SalvageGrade::of(workmanship)))
        })
        .collect();
    let first = turns.iter().map(|(_, turn)| *turn).max()?;
    let batch = turns
        .into_iter()
        .filter(|(_, turn)| *turn == first)
        .map(|(guid, _)| guid)
        .collect();
    Some((first.1, batch))
}

impl Client {
    /// This character's Salvaging as it stands, buffs counted.
    pub fn salvaging(&self) -> u32 {
        self.skill_now(SALVAGING)
    }

    /// Who salvages for the team, this character included: the highest
    /// Salvaging among those carrying an Ust (see [`best_salvager`]).
    /// Off the team it is this character, if it has an Ust.
    pub fn best_salvager(&self) -> Option<(String, u32)> {
        let me = Mate {
            name: self.world.stats.name.clone(),
            guid: self.world.player_guid.unwrap_or(0),
            salvaging: self.salvaging(),
            has_ust: self.salvage_tool().is_some(),
            ..Default::default()
        };
        best_salvager(std::iter::once(&me).chain(self.autoplay.team.mates.iter()))
    }

    /// Carried items tagged for salvage that can go: not worn, not
    /// wanted by a blank tag. `bags` says whether salvage bags count
    /// (they are handed on, never salvaged again). Each comes with its
    /// name, for the log, and its workmanship, which decides the batch
    /// it is salvaged in (see `SalvageGrade`).
    fn salvage_tagged(&self, bags: bool) -> Vec<(u32, String, f32)> {
        let me = self.world.player_guid;
        let mut items: Vec<(u32, String, f32)> = self
            .autoplay
            .ledger
            .for_salvage(crate::holdings::unix_now())
            .into_iter()
            .filter_map(|g| self.world.objects.get(&g))
            .filter(|o| o.wielder != me && self.world.is_carried(o.guid))
            .filter(|o| {
                let bag = o.name.starts_with("Salvaged ");
                if bag {
                    bags
                } else {
                    o.material != 0 && o.workmanship > 0.0
                }
            })
            .filter(|o| {
                self.autoplay
                    .refused
                    .get(&o.guid)
                    .is_none_or(|n| *n < SALVAGE_TRIES)
            })
            .map(|o| (o.guid, o.name.clone(), o.workmanship))
            .collect();
        items.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
        items
    }

    /// Salvage what the rules tagged, or carry it to whoever salvages
    /// for the team. Runs between fights. True while busy with it.
    pub(crate) fn autoplay_salvage(&mut self, now: Instant) -> bool {
        if self.world.player_guid.is_none() {
            return false;
        }
        self.autoplay_tag_arrivals(now);
        let Some(profile) = self.loot_profile() else {
            return false;
        };
        let cfg = profile.looting.clone();
        if self.attack_target.is_some() || self.autoplay.corpse.is_some() {
            return false;
        }
        // A batch on its way: wait for the items to go, and count a
        // refusal against each when they do not.
        let mut answered = false;
        if let Some((items, since)) = self.autoplay.salvaging.clone() {
            let left: Vec<u32> = items
                .iter()
                .copied()
                .filter(|g| self.world.is_carried(*g))
                .collect();
            if left.is_empty() {
                self.autoplay.salvaging = None;
                answered = true;
                self.autoplay.say(
                    Doing::Salvaging,
                    format!("salvaged {} item(s)", items.len()),
                );
            } else if now.duration_since(since) < SALVAGE_TIMEOUT {
                return true;
            } else {
                self.autoplay.salvaging = None;
                for g in left {
                    let n = self.autoplay.refused.entry(g).or_default();
                    *n += 1;
                    if *n >= SALVAGE_TRIES {
                        let name = self.world.name_of(g).map(str::to_string);
                        // A refusal is not a new decision. Rewriting it
                        // to Keep made the thing eligible for nothing
                        // while it went on holding a slot.
                        self.autoplay.tag_failed(g, "could not be salvaged");
                        self.autoplay.note(
                            format!(
                                "could not salvage {}, setting it aside",
                                name.unwrap_or_default()
                            ),
                            now,
                        );
                    }
                }
            }
        }
        if let Some((item, since)) = self.autoplay.handing {
            if !self.world.is_carried(item) {
                self.autoplay.handing = None;
            } else if now.duration_since(since) < SALVAGE_TIMEOUT {
                return true;
            } else {
                self.autoplay.handing = None;
                let n = self.autoplay.refused.entry(item).or_default();
                *n += 1;
                if *n >= SALVAGE_TRIES {
                    let name = self
                        .world
                        .objects
                        .get(&item)
                        .map(|o| o.name.clone())
                        .unwrap_or_default();
                    self.autoplay
                        .tag_failed(item, "the salvager would not take it");
                    self.autoplay
                        .note(format!("{name} was not taken, setting it aside"), now);
                }
            }
        }
        let Some((who, guid)) = self.best_salvager() else {
            if !self.salvage_tagged(false).is_empty() {
                self.autoplay
                    .note("salvage waiting: nobody on the team carries an Ust", now);
            }
            return false;
        };
        // A batch the server has just salvaged is answered, and the next
        // grade goes at once. Sitting out the gap after it returned the
        // tick to the goals below, the fight among them, and the grades
        // still to come waited out a whole fight and its looting.
        let rate_ok = answered
            || self
                .autoplay
                .last_salvage
                .is_none_or(|t| now.duration_since(t) >= SALVAGE_EVERY);
        if Some(guid) == self.world.player_guid {
            if !cfg.salvage {
                return false;
            }
            let items = self.salvage_tagged(false);
            if items.is_empty() || !rate_ok {
                return false;
            }
            // The server salvages in peace mode only.
            if self.combat {
                self.toggle_combat();
                return true;
            }
            // A 9 or a 10 goes only with its own grade, and the rest wait
            // their turn. What teammates handed over is batched the same
            // way: it was tagged when it arrived, like anything looted.
            let refused = |g: &u32| self.autoplay.refused.get(g).copied().unwrap_or(0);
            let Some((grade, guids)) =
                next_salvage_batch(items.iter().map(|(g, _, w)| (*g, *w, refused(g))))
            else {
                return false;
            };
            if !self.salvage(&guids) {
                return false;
            }
            self.autoplay.salvaging = Some((guids.clone(), now));
            self.autoplay.last_salvage = Some(now);
            let apart = match grade {
                SalvageGrade::Common => "",
                SalvageGrade::Nine => " of workmanship 9, on their own",
                SalvageGrade::Ten => " of workmanship 10, on their own",
            };
            self.autoplay.say(
                Doing::Salvaging,
                format!("salvaging {} item(s){apart}", guids.len()),
            );
            return true;
        }
        if !cfg.hand_off {
            return false;
        }
        let items = self.salvage_tagged(true);
        if items.is_empty() {
            return false;
        }
        let Some(mate) = self
            .autoplay
            .team
            .mates
            .iter()
            .find(|m| m.guid == guid)
            .cloned()
        else {
            return false;
        };
        let Some(me) = self.my_position() else {
            return false;
        };
        // Not while it is fighting, and not from across the map.
        if mate.target.is_some() {
            return false;
        }
        let distance = mate.world.distance(me);
        if distance > HAND_OFF_RANGE {
            self.autoplay.note(
                format!(
                    "salvage waiting: {who} is {distance:.0} m off (salvaging {})",
                    mate.salvaging
                ),
                now,
            );
            return false;
        }
        if distance > GIVE_REACH {
            if self
                .follow
                .is_none_or(|f| f.target.distance(mate.world) > 1.0)
            {
                self.interrupt_travel("taking salvage to the salvager");
                self.steering.reset();
            }
            self.head_for(mate.world, GIVE_REACH * 0.8, &who);
            self.autoplay
                .say(Doing::Salvaging, format!("taking salvage to {who}"));
            return true;
        }
        if self.follow.take().is_some() {
            self.steering.reset();
        }
        if !rate_ok {
            return true;
        }
        // One item at a time, so a hand-off never mixes grades: the
        // salvager batches what arrives like the rest of its own.
        let (item, name, _) = items[0].clone();
        if !self.give(guid, item, None) {
            return false;
        }
        self.autoplay.handing = Some((item, now));
        self.autoplay.last_salvage = Some(now);
        self.autoplay.say(
            Doing::Salvaging,
            format!(
                "giving {name} to {who} to salvage ({} left)",
                items.len() - 1
            ),
        );
        true
    }
}

#[cfg(test)]
mod tests;
