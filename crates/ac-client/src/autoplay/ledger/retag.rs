use std::time::{Duration, Instant};

use crate::autoplay::{judge_loot, Autoplay, LootAction};
use crate::Client;

/// How long an item that arrived in the pack waits for its appraisal
/// before the rules judge it as it is.
pub(crate) const TAG_TIMEOUT: Duration = Duration::from_secs(15);

/// How often the pack is judged afresh while a re-judge is waiting on
/// appraisals.
///
/// The answers cannot change faster than the server sends them, so
/// there is nothing to be had from asking every frame -- and a hundred
/// and fifty items judged sixty times a second for the fifteen seconds
/// an appraisal may take is real work for no answer.
pub(crate) const RETAG_EVERY: Duration = Duration::from_millis(250);

/// Each carried thing paired with how many of its kind came before
/// it, oldest first.
///
/// `carried` is `(guid, wcid, stack)`. The server hands out rising ids,
/// so sorting by guid is the order the character came by the things in,
/// and the running count *before* each is what a rule with a
/// `keep_up_to` on it was answered with when they arrived one at a
/// time: `held` is what was already in the pack when the thing was
/// judged, on the corpse path and everywhere else, and a rule with a
/// cap keeps while `held` is under it.
///
/// The whole point is not to hand every item the pack's total. A rule
/// that keeps up to two rings, asked about three rings and told three
/// times that three are carried, claims none of them -- and a profile
/// edit would turn a set of keepers into a set of vendor trash in one
/// pass. Nor the count with the item itself in it, which this once
/// was: told it was the second of two, the second ring was over a cap
/// of two, and the pass kept one ring fewer than the cap.
fn in_arrival_order(carried: &mut [(u32, u32, u32)]) -> Vec<(u32, u32)> {
    carried.sort_unstable();
    let mut seen_of: std::collections::BTreeMap<u32, u32> = std::collections::BTreeMap::new();
    carried
        .iter()
        .map(|(guid, wcid, stack)| {
            let n = seen_of.entry(*wcid).or_insert(0);
            let before = *n;
            *n += stack;
            (*guid, before)
        })
        .collect()
}

/// What to write down about something that turned up in the pack
/// (given, bought, made) rather than off a corpse.
///
/// The same judgement as a corpse item's, and by the same profile: a
/// bundle of arrowheads is a bundle of arrowheads whether it came off a
/// drudge or over a counter, and it used to be judged by two different
/// sets of rules depending on which. `Keep` is written down too, where
/// this once answered only Salvage or Sell -- "the character means to
/// keep this" is exactly what the vendor side needs to hear, and
/// silence let it be sold.
pub fn arrival_tag(
    stats: &crate::items::ItemStats,
    id: Option<&ac_net::messages::Appraisal>,
    profile: Option<&crate::profile::Profile>,
    me: &crate::weapons::Wielder,
    my_name: &str,
    held: u32,
) -> Option<LootAction> {
    match judge_loot(stats, id, profile, me, my_name, held) {
        crate::profile::Verdict::Decided(LootAction::Skip, _) => None,
        crate::profile::Verdict::Decided(a, _) => Some(a),
        // Not judgeable yet, or nothing claimed it: nothing to write
        // down, and the pack keeps it either way.
        crate::profile::Verdict::NeedsId(_) | crate::profile::Verdict::None => None,
    }
}

impl Autoplay {
    /// Write down what an item was taken for (the loot pass, a script,
    /// the inventory panel): what the salvage and vendor passes do with
    /// it from now on.
    pub fn tag(&mut self, stats: &crate::items::ItemStats, action: LootAction) {
        self.ledger.remember(stats, action);
        self.seen.insert(stats.guid);
    }

    /// What was decided could not be done -- a counter that would not
    /// take it, a salvage refused. Not a new decision: the thing is
    /// still meant for what it was meant for.
    pub fn tag_failed(&mut self, guid: u32, why: impl Into<String>) {
        self.ledger.failed(guid, why, crate::holdings::unix_now());
    }

    /// What each item was taken for, by guid.
    pub fn tags(&self) -> std::collections::BTreeMap<u32, LootAction> {
        self.ledger.actions()
    }
}

impl Client {
    /// Notice the rules changing and re-judge what is already carried.
    ///
    /// A decision stands for as long as the rules that made it do. It
    /// has to: a character that judged its pack afresh every time it
    /// looked would sell the ring it kept the moment the pack filled,
    /// which is the whole reason the ledger exists. But the *rules*
    /// changing is the one thing that should reach back. The player has
    /// just said what their things are worth, and a decision written
    /// down under the old rules is an answer to a question nobody is
    /// asking any more.
    ///
    /// Runs whatever else the character is doing, autoplay off
    /// included: the ledger is what the Items window reads, and it
    /// should not be telling somebody their mule is holding things for
    /// a rule they deleted.
    pub(crate) fn tick_retag(&mut self, now: Instant) {
        let profile = self.loot_profile();
        // The cheap half, run every frame: has the shelf handed out a
        // different profile? It replaces the whole profile on every edit,
        // name lists and all, so identity answers it without reading a
        // rule.
        let untouched = match &self.autoplay.judged_under {
            Some(was) => match (was, &profile) {
                (None, None) => true,
                (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
                _ => false,
            },
            None => false,
        };
        if !untouched {
            // A character that has not entered the world yet has no
            // pack to judge and no name to file a ledger under. Leave
            // the rules unrecorded so this runs again once it has.
            if self.world.stats.name.trim().is_empty() {
                return;
            }
            let rules = self.rules_fingerprint(profile.as_deref());
            self.autoplay.judged_under = Some(profile);
            // Something was handed out, but were the rules themselves
            // any different? Typing in a profile's note replaces it
            // without changing a single answer, and neither does
            // opening a client that has been shut since the last edit.
            //
            // This is also what keeps a decision from being re-made on
            // every login. Judging the pack afresh each time is the one
            // thing the ledger exists to prevent: a rule that keeps up
            // to a number reads the pack it is in, and a pack that has
            // since filled turns yesterday's keepers into today's
            // vendor trash.
            if self.autoplay.ledger.rules() == Some(rules) {
                return;
            }
            self.autoplay.ledger.judged_under(rules);
            self.autoplay.retagging = Some(now);
            self.autoplay.retag_due = Some(now);
        }
        let Some(since) = self.autoplay.retagging else {
            return;
        };
        if self.autoplay.retag_due.is_some_and(|due| now < due) {
            return;
        }
        self.autoplay.retag_due = Some(now + RETAG_EVERY);
        if self.retag_pack(now.duration_since(since) >= TAG_TIMEOUT) {
            self.autoplay.retagging = None;
            self.autoplay.retag_due = None;
        }
    }

    /// Everything that decides an item, as one number (see
    /// `Profile::fingerprint`). A character reading no profile still has
    /// a fingerprint, so being given one counts as a change.
    fn rules_fingerprint(&self, profile: Option<&crate::profile::Profile>) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        profile.map(|p| p.fingerprint()).hash(&mut h);
        h.finish()
    }

    /// Judge everything carried against the rules as they stand now and
    /// write the answers down. True when nothing is left waiting on an
    /// appraisal, so the caller can stop.
    ///
    /// `settle` says to take the answer as it is rather than go on
    /// waiting for the server.
    ///
    /// Items are gone through oldest first and each is told how many of
    /// its kind come at or before it, rather than how many are carried
    /// altogether. That is what a rule with a `keep_up_to` on it was
    /// answered with when the things arrived one at a time, and telling
    /// all three rings that three rings are carried would put every one
    /// of them over a cap of two -- turning a set of keepers into a set
    /// of vendor trash in one pass.
    fn retag_pack(&mut self, settle: bool) -> bool {
        let profile = self.loot_profile();
        let wielder = self.wielder();
        let who = self.world.stats.name.clone();
        let mut carried: Vec<(u32, u32, u32)> = self
            .world
            .inventory()
            .chain(self.world.wielded())
            .map(|o| (o.guid, o.weenie_class_id, o.stack_size.max(1)))
            .collect();
        let mut waiting = Vec::new();
        for (guid, held) in in_arrival_order(&mut carried) {
            let Some(stats) = self.stats_of(guid) else {
                continue;
            };
            match judge_loot(
                &stats,
                self.appraisals.get(&guid),
                profile.as_deref(),
                &wielder,
                &who,
                held,
            ) {
                crate::profile::Verdict::Decided(LootAction::Skip, _)
                | crate::profile::Verdict::None => {
                    // Nothing claims it any more. That is not a decision
                    // to be rid of it -- it is no decision at all, and
                    // an item with no entry is never sold.
                    self.autoplay.ledger.forget(guid);
                }
                crate::profile::Verdict::Decided(action, _) => {
                    if self.autoplay.ledger.of(&stats) != Some(action) {
                        self.autoplay.tag(&stats, action);
                    }
                }
                // A rule wants it but cannot say so until the server has
                // identified it. Yesterday's answer stands in the
                // meantime rather than being thrown away over a question
                // that has not been answered.
                crate::profile::Verdict::NeedsId(_) if !settle && !stats.appraised => {
                    waiting.push(guid);
                }
                crate::profile::Verdict::NeedsId(_) => {}
            }
        }
        if waiting.is_empty() {
            return true;
        }
        self.appraise_many(waiting);
        false
    }

    /// Judge a carried item by this character's profile and write down
    /// what it is for, the way the arrival pass does for something that
    /// turns up in the pack ([`arrival_tag`]).
    ///
    /// `None` when nothing claimed it, and then nothing is written:
    /// silence is not a decision to leave it, and an item with no entry
    /// is judged afresh next time.
    pub fn tag_loot(&mut self, guid: u32) -> Option<LootAction> {
        let stats = self.stats_of(guid)?;
        let action = arrival_tag(
            &stats,
            self.appraisals.get(&guid),
            self.loot_profile().as_deref(),
            &self.wielder(),
            &self.world.stats.name.clone(),
            self.already_carried(stats.wcid),
        )?;
        self.autoplay.tag(&stats, action);
        Some(action)
    }

    /// Look at what has turned up in the pack since last time and tag
    /// what the rules would salvage or sell (see [`arrival_tag`]): the
    /// salvage a teammate handed over, mostly. The first pass only
    /// notes what is carried.
    pub(crate) fn autoplay_tag_arrivals(&mut self, now: Instant) {
        let profile = self.loot_profile();
        let wielder = self.wielder();
        let who = self.world.stats.name.clone();
        let carried: Vec<u32> = self
            .world
            .inventory()
            .chain(self.world.wielded())
            .map(|o| o.guid)
            .collect();
        let ap = &mut self.autoplay;
        if !ap.baselined {
            ap.seen = carried.iter().copied().collect();
            ap.baselined = true;
            return;
        }
        ap.seen.retain(|g| carried.contains(g));
        // An entry lives only as long as the thing does: this is what
        // stops a recycled id ever being mistaken for the item it used
        // to name (see `ac_loot::ledger`).
        ap.ledger.forget_gone(&carried);
        ap.refused.retain(|g, _| carried.contains(g));
        ap.pending_tags.retain(|(g, _)| carried.contains(g));
        for g in &carried {
            if ap.seen.insert(*g) && ap.ledger.by_guid(*g).is_none() {
                ap.pending_tags.push((*g, now));
            }
        }
        // Whether an identify is worth asking for: the profile says,
        // since it is the only thing that judges anything now.
        let needs = profile
            .as_ref()
            .is_some_and(|p| p.looting.appraise && p.needs_id());
        let pending = std::mem::take(&mut self.autoplay.pending_tags);
        for (g, since) in pending {
            let Some(stats) = self.stats_of(g) else {
                continue;
            };
            if needs && !stats.appraised && now.duration_since(since) < TAG_TIMEOUT {
                self.appraise_many([g]);
                self.autoplay.pending_tags.push((g, since));
                continue;
            }
            // What was held before it arrived: the count every other
            // path judges against, and the count a cap is a cap on.
            // With the arrival itself counted, the fourth kit under
            // "keep up to four" was the fourth of four, over the cap,
            // and "the rest, to the counter" had it.
            let held = self.carried_besides(&stats);
            if let Some(action) = arrival_tag(
                &stats,
                self.appraisals.get(&g),
                profile.as_deref(),
                &wielder,
                &who,
                held,
            ) {
                let arrived = format!("{} arrived, tagged {}", stats.name, action.label());
                tracing::info!("autoplay: {arrived}");
                // Every arrival, in telemetry too: what a purchase or a corpse really put in the pack.
                self.events.push(crate::Event::Noted(arrived));
                self.autoplay.tag(&stats, action);
            }
        }
    }
}

#[cfg(test)]
mod tests;
