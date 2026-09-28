use std::time::{Duration, Instant};

use crate::autoplay::{Doing, Mate, GIVE_REACH};
use crate::Client;
use ac_loot::bands::{self, Band};

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

/// An item tagged for salvage, as a salvage call is chosen from them.
#[derive(Clone, Debug, PartialEq)]
struct Waiting {
    guid: u32,
    material: u32,
    workmanship: f32,
    /// The salvage rule whose bands it goes by (`ac_loot::bands`); empty for all together.
    rule: String,
    bands: Vec<Band>,
    /// Salvages of it that came to nothing.
    refused: u8,
}

/// A carried salvage bag with room left: the rule and band a salvage of ours made it in, when that
/// was written down (`ac_loot::ledger::Made`), its average workmanship, the units in it and how
/// many it holds.
#[derive(Clone, Debug, PartialEq)]
struct Partial {
    guid: u32,
    material: u32,
    made: Option<(String, Band)>,
    /// The header's: the average of what went in (WorldObject_Properties.cs:1560-1568).
    workmanship: f32,
    units: u32,
    holds: u32,
}

/// A salvage call whose bags are still to arrive: its material, rule and band, the bags of that
/// material carried when it went, and when.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Making {
    material: u32,
    rule: String,
    band: Band,
    before: Vec<u32>,
    sent: Instant,
}

/// How long a call's bags are waited for before it is forgotten.
const MAKING_WAIT: Duration = Duration::from_secs(30);

/// One salvage call: every item of one material in one band, and the partial bags of that material
/// in that band it tops up, sent first.
#[derive(Clone, Debug, PartialEq)]
struct Batch {
    material: u32,
    rule: String,
    band: Band,
    bags: Vec<u32>,
    items: Vec<u32>,
    /// Units in the bags topped up.
    units: u32,
}

impl Batch {
    /// The guids in the order the call sends them: a bag after an item overflows and its excess is
    /// lost (TryAddSalvage, Player_Crafting.cs:283-289), so bags go first.
    fn guids(&self) -> Vec<u32> {
        self.bags.iter().chain(&self.items).copied().collect()
    }
}

/// The next salvage call out of `waiting`, topping up bags from `partial`; `None` with nothing to
/// salvage. ACE puts all of one material in one call into one bag and averages its workmanship
/// (Player_Crafting.cs:251, 279), so a call holds one material in one band of one rule.
///
/// What has come to nothing least goes first (ACE skips a Retained item without a word), then the
/// best band, so ordinary loot arriving between calls never keeps it waiting. A bag a salvage of
/// ours made is topped up only in its own rule and band; an older one, which says only its average,
/// in the band its average falls in (the player's call: it is fine to include them); the fullest
/// that fit together in one bag's worth: a bag input past it loses the excess.
fn next_salvage_batch(waiting: &[Waiting], partial: &[Partial]) -> Option<Batch> {
    use std::cmp::Reverse;
    let turn = |w: &Waiting| {
        let band = bands::band_of(&w.bands, w.workmanship);
        (
            Reverse(w.refused),
            band.1,
            band.0,
            Reverse(w.material),
            w.rule.clone(),
        )
    };
    let first = waiting.iter().max_by_key(|w| turn(w))?;
    let key = turn(first);
    let band = bands::band_of(&first.bands, first.workmanship);
    let items: Vec<u32> = waiting
        .iter()
        .filter(|w| turn(w) == key)
        .map(|w| w.guid)
        .collect();
    let mut fits: Vec<&Partial> = partial
        .iter()
        .filter(|b| b.material == first.material && b.units < b.holds)
        .filter(|b| match &b.made {
            Some((rule, made)) => *rule == first.rule && *made == band,
            None => bands::band_of(&first.bands, b.workmanship) == band,
        })
        .collect();
    fits.sort_by_key(|b| (Reverse(b.units), b.guid));
    let (mut bags, mut units) = (Vec::new(), 0);
    for b in fits {
        if units + b.units <= b.holds {
            bags.push(b.guid);
            units += b.units;
        }
    }
    Some(Batch {
        material: first.material,
        rule: first.rule.clone(),
        band,
        bags,
        items,
        units,
    })
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

    /// Carried items tagged for salvage that can go: not worn, not refused too often, and
    /// appraised and found not inscribed (never salvaged, the player's word; only an appraisal
    /// says). `bags` says whether salvage bags tagged for it count, for the hand-off. Each comes
    /// with its name, for the log; second, what is not yet appraised, to ask about.
    fn salvage_tagged(&self, bags: bool) -> (Vec<(u32, String)>, Vec<u32>) {
        let me = self.world.player_guid;
        let (mut items, mut unknown) = (Vec::new(), Vec::new());
        for o in self
            .autoplay
            .ledger
            .for_salvage(crate::holdings::unix_now())
            .into_iter()
            .filter_map(|g| self.world.objects.get(&g))
            .filter(|o| o.wielder != me && self.world.is_carried(o.guid))
            .filter(|o| {
                self.autoplay
                    .refused
                    .get(&o.guid)
                    .is_none_or(|n| *n < SALVAGE_TRIES)
            })
        {
            if o.name.starts_with("Salvaged ") {
                if bags {
                    items.push((o.guid, o.name.clone()));
                }
                continue;
            }
            if o.material == 0 || o.workmanship <= 0.0 {
                continue;
            }
            match self.stats_of(o.guid) {
                Some(st) if st.appraised && !st.inscribed => items.push((o.guid, o.name.clone())),
                Some(st) if st.appraised => {}
                _ => unknown.push(o.guid),
            }
        }
        items.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
        (items, unknown)
    }

    /// The salvage rule an item tagged for salvage goes by, and its bands: the rule that tagged it,
    /// or, for a tag written without one or a name two rules share, the salvage rule that would take
    /// it now; else no rule, all together.
    fn salvage_bands(&self, profile: &crate::profile::Profile, guid: u32) -> (String, Vec<Band>) {
        use crate::autoplay::LootAction::Salvage;
        let rule = self
            .autoplay
            .ledger
            .why(guid)
            .and_then(|r| profile.rule_named(r))
            .filter(|r| r.action == Salvage)
            .or_else(|| {
                let stats = self.stats_of(guid)?;
                profile
                    .decided_by(
                        &stats,
                        self.appraisals.get(&guid),
                        &self.wielder(),
                        &self.world.stats.name,
                        self.carried_besides(&stats),
                    )
                    .map(|(_, r)| r)
                    .filter(|r| r.action == Salvage)
            });
        rule.map_or_else(
            || (String::new(), vec![bands::ALL]),
            |r| (r.name.clone(), r.bands()),
        )
    }

    /// Write down which call made each salvage bag that has turned up since it went: the new bags of
    /// its material, for its rule and band, the oldest call first. A call answered with none is
    /// forgotten after [`MAKING_WAIT`].
    fn note_bags_made(&mut self, now: Instant) {
        let me = self.world.player_guid;
        let mut making = std::mem::take(&mut self.autoplay.making);
        making.retain(|m| {
            let new: Vec<crate::items::ItemStats> = self
                .world
                .inventory()
                .filter(|o| o.name.starts_with("Salvaged ") && o.material == m.material)
                .filter(|o| o.wielder != me && !m.before.contains(&o.guid))
                .filter_map(|o| self.stats_of(o.guid))
                .filter(|st| self.autoplay.ledger.made_by(st).is_none())
                .collect();
            for st in &new {
                self.autoplay.ledger.made(st, &m.rule, m.band);
            }
            new.is_empty() && now.duration_since(m.sent) < MAKING_WAIT
        });
        self.autoplay.making = making;
    }

    /// Carried salvage bags with room left that may be topped up: any not meant for a counter.
    fn partial_bags(&self) -> Vec<Partial> {
        let me = self.world.player_guid;
        self.world
            .inventory()
            .filter(|o| o.name.starts_with("Salvaged ") && o.material != 0 && o.wielder != me)
            .filter_map(|o| {
                let st = self.stats_of(o.guid)?;
                let meant = self.autoplay.ledger.of(&st);
                if matches!(
                    meant,
                    Some(crate::autoplay::LootAction::Sell | crate::autoplay::LootAction::Skip)
                ) {
                    return None;
                }
                Some(Partial {
                    guid: o.guid,
                    material: o.material,
                    made: self
                        .autoplay
                        .ledger
                        .made_by(&st)
                        .map(|m| (m.rule.clone(), m.band)),
                    workmanship: o.workmanship,
                    units: o.structure,
                    // MaxStructure 100 unless the bag says (Player_Crafting.cs:237-244).
                    holds: if o.max_structure > 0 {
                        o.max_structure
                    } else {
                        100
                    },
                })
            })
            .filter(|b| b.units > 0 && b.units < b.holds)
            .collect()
    }

    /// Salvage what the rules tagged, or carry it to whoever salvages
    /// for the team. Runs between fights. True while busy with it.
    pub(crate) fn autoplay_salvage(&mut self, now: Instant) -> bool {
        if self.world.player_guid.is_none() {
            return false;
        }
        self.autoplay_tag_arrivals(now);
        self.note_bags_made(now);
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
                    let name = self.world.name_of(item).unwrap_or_default().to_string();
                    self.autoplay
                        .tag_failed(item, "the salvager would not take it");
                    self.autoplay
                        .note(format!("{name} was not taken, setting it aside"), now);
                }
            }
        }
        let Some((who, guid)) = self.best_salvager() else {
            if !self.salvage_tagged(false).0.is_empty() {
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
            let (items, unknown) = self.salvage_tagged(false);
            // Asked about before anything goes: only an appraisal tells an inscribed item.
            if !unknown.is_empty() {
                self.appraise_many(unknown);
            }
            if items.is_empty() || !rate_ok {
                return false;
            }
            // The server salvages in peace mode only.
            if self.combat {
                self.toggle_combat();
                return true;
            }
            // One material in one band a call, the rule's own bands, bags of it topped up first.
            // What teammates handed over is batched the same way: tagged when it arrived.
            let waiting: Vec<Waiting> = items
                .iter()
                .filter_map(|(g, _)| {
                    let o = self.world.objects.get(g)?;
                    let (rule, bands) = self.salvage_bands(&profile, *g);
                    Some(Waiting {
                        guid: *g,
                        material: o.material,
                        workmanship: o.workmanship,
                        rule,
                        bands,
                        refused: self.autoplay.refused.get(g).copied().unwrap_or(0),
                    })
                })
                .collect();
            let Some(batch) = next_salvage_batch(&waiting, &self.partial_bags()) else {
                return false;
            };
            let guids = batch.guids();
            let before: Vec<u32> = self
                .world
                .inventory()
                .filter(|o| o.name.starts_with("Salvaged ") && o.material == batch.material)
                .map(|o| o.guid)
                .collect();
            if !self.salvage(&guids) {
                return false;
            }
            self.autoplay.making.push(Making {
                material: batch.material,
                rule: batch.rule.clone(),
                band: batch.band,
                before,
                sent: now,
            });
            self.autoplay.salvaging = Some((guids, now));
            self.autoplay.last_salvage = Some(now);
            let material = ac_world::material::name(batch.material);
            let topping = if batch.bags.is_empty() {
                String::new()
            } else {
                format!(
                    ", topping up {} bag(s) of {} units",
                    batch.bags.len(),
                    batch.units
                )
            };
            self.autoplay.say(
                Doing::Salvaging,
                format!(
                    "salvaging {} {material} item(s), workmanship {}{topping}",
                    batch.items.len(),
                    bands::tell(batch.band)
                ),
            );
            return true;
        }
        if !cfg.hand_off {
            return false;
        }
        let (items, unknown) = self.salvage_tagged(true);
        if !unknown.is_empty() {
            self.appraise_many(unknown);
        }
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
        let (item, name) = items[0].clone();
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
