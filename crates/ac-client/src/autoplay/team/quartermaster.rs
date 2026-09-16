use std::time::{Duration, Instant};

use crate::autoplay::loot::choose::WILL_NOT_SPLIT;
use crate::autoplay::loot::take::{REACH_GIVE_UP, REACH_PROGRESS};
use crate::autoplay::{Doing, LootAction};
use crate::growth::Growth;
use crate::Client;

/// Close enough to hand something over (ACE's use radius, with room).
pub(crate) const GIVE_REACH: f32 = 2.0;

/// How often one character hands something to another. The server
/// takes one give at a time and answers in its own time.
pub(crate) const GIVE_EVERY: Duration = Duration::from_millis(700);

impl Client {
    /// The carried stack to hand a teammate short of `want`, by name.
    ///
    /// The stack the player said to sell first, since it is leaving
    /// anyway, and a stack they said to keep only when there is no
    /// other of the kind. The first stack whose name matched used to
    /// go, which was the kept stack as often as not while the one
    /// meant for a counter stayed.
    pub(crate) fn spare_for(&self, want: &str) -> Option<(u32, String)> {
        let want = want.to_lowercase();
        let ledger = &self.autoplay.ledger;
        self.world
            .inventory()
            .filter(|o| o.name.to_lowercase().contains(&want))
            .min_by_key(|o| match ledger.by_guid(o.guid) {
                Some(LootAction::Sell) => 0,
                Some(LootAction::Keep) => 2,
                _ => 1,
            })
            .map(|o| (o.guid, o.name.clone()))
    }

    /// Note what this character is running short of, so the others can
    /// hand it over.
    pub(crate) fn autoplay_stock(&mut self) {
        if !self.autoplay.config.team.enabled {
            self.autoplay.wants.clear();
            return;
        }
        // What this character is short of, from the same buy list that
        // decides what it shops for and what it will not sell. It used
        // to be a second list on the team settings, tested by hand here
        // with the same arithmetic the profile already does.
        //
        // This is what a teammate reads before handing anything over
        // (`Mate::wants`), so a character with nothing on its buy list
        // asks for nothing -- which is right, and is also why the list
        // matters more than it looks.
        //
        // Counted the way the character's own restock list counts it:
        // what is carried less what the profile is selling out of. The
        // two once differed, and a mate handed scarabs over while the
        // character's own list said it wanted none.
        let profile = self.profiles.get(&self.autoplay.config.loot.profile);
        let leaving = self.leaving_of(&self.item_stats());
        self.autoplay.wants = profile
            .map(|p| {
                p.shortfall(|what| self.carried_named(what).saturating_sub(leaving.named(what)))
                    .into_iter()
                    .map(|s| s.want.what.clone())
                    .collect()
            })
            .unwrap_or_default();
    }

    /// Loading the quartermaster and unloading it again.
    ///
    /// On a quartermaster run one character carries the party's sale
    /// loot to town and its shopping home, so there are two moments
    /// where items change hands: everyone gives it their loot before it
    /// leaves, and it gives everyone their order when it gets back.
    /// Both are the same shape -- walk into reach, hand one thing over,
    /// come back next frame for the next -- because the server takes
    /// one give at a time.
    ///
    /// True when it acted, which stops the rest of the rules for this
    /// frame: nothing else matters while the party is being loaded.
    pub(super) fn autoplay_quartermaster(&mut self, now: Instant) -> bool {
        use crate::logistics::{Plan, Stage};
        let team = self.autoplay.config.team.clone();
        if !team.enabled || !team.restock.together || team.restock.plan != Plan::Quartermaster {
            return false;
        }
        let Some(stage) = self.autoplay.growth.mode.stage() else {
            return false;
        };
        // Waiting on the last give. The frame is still this errand's:
        // let something else have it and the tidying merges away the
        // money that was just counted out.
        if self
            .autoplay
            .last_give
            .is_some_and(|t| now.duration_since(t) < GIVE_EVERY)
        {
            return matches!(stage, Stage::HandOver | Stage::HandOut);
        }
        let growth = self.autoplay.config.growth.clone();
        let Some(runner) = self.quartermaster_name(&growth, now) else {
            return false;
        };
        let am_runner = runner == self.world.stats.name;
        match stage {
            Stage::HandOver if !am_runner => self.load_the_quartermaster(&runner, &growth, now),
            Stage::HandOut if am_runner => self.unload_the_quartermaster(&growth, now),
            _ => false,
        }
    }

    /// Give the runner this character's sale loot, then say so.
    fn load_the_quartermaster(
        &mut self,
        runner: &str,
        growth: &crate::growth::Growth,
        now: Instant,
    ) -> bool {
        if self.autoplay.growth.handed_over {
            return false;
        }
        let Some(mate) = self
            .autoplay
            .team
            .mates
            .iter()
            .find(|m| m.name == runner)
            .cloned()
        else {
            return false;
        };
        let loot = self.loot_for_sale(growth);
        // Money travels with the loot. The runner is the one standing
        // at the counter, so it is the one that has to be able to pay,
        // and coin changes hands for nothing: it is trade notes that
        // cost to make (see `spare_coin`).
        let coin = self
            .autoplay
            .config
            .team
            .restock
            .share_money
            .then(|| self.spare_coin())
            .flatten();
        if loot.is_empty() && coin.is_none() {
            // Nothing to hand over: this character is loaded already.
            self.autoplay.growth.handed_over = true;
            return false;
        }
        if !self.step_into_reach(mate.world) {
            if self.reaching_too_long(mate.world, now) {
                // Cannot get to them -- a wall, a different building,
                // a floor above. The party is not held up over it.
                self.autoplay.growth.handed_over = true;
                self.autoplay
                    .note(format!("cannot get to {runner} to hand over"), now);
                return false;
            }
            self.autoplay
                .say(Doing::Helping, format!("taking the loot to {runner}"));
            return true;
        }
        if let Some((purse, amount)) = coin {
            // Part of a stack cannot be handed over as it stands: the
            // money is counted out first (see `hand_stack`).
            match self.hand_stack(mate.guid, purse, amount, now) {
                Some(true) => {
                    self.autoplay.say(
                        Doing::Helping,
                        format!("giving {runner} {amount} pyreals to shop with"),
                    );
                    return true;
                }
                None => {
                    self.autoplay.say(
                        Doing::Helping,
                        format!("counting out {amount} pyreals for {runner}"),
                    );
                    return true;
                }
                // The money will not come apart. The loot still can go,
                // and the runner may have enough of its own.
                Some(false) => {}
            }
        }
        let Some(&item) = loot.first() else {
            self.autoplay.growth.handed_over = true;
            return false;
        };
        let name = self.world.name_of(item).unwrap_or_default().to_string();
        if !self.give(mate.guid, item, None) {
            // The server would not take it; do not jam on this item.
            self.autoplay.growth.handed_over = true;
            return false;
        }
        self.autoplay.last_give = Some(now);
        self.autoplay
            .give_tries
            .note(item, &crate::did::Did::Done, now);
        self.autoplay
            .say(Doing::Helping, format!("giving {name} to {runner} to sell"));
        // Loaded once the last piece has gone.
        self.autoplay.growth.handed_over = loot.len() == 1;
        true
    }

    /// A carried stack of this weenie holding exactly this many, which
    /// is what a split leaves behind: the piece counted out to hand
    /// over. `None` for the weenie matches anything of the right size.
    fn piece_of(&self, wcid: Option<u32>, amount: u32) -> Option<u32> {
        self.world
            .inventory()
            .find(|o| {
                o.stack_size == amount
                    && match wcid {
                        Some(w) => o.weenie_class_id == w,
                        None => o.item_type & ac_world::item_type::MONEY != 0,
                    }
            })
            .map(|o| o.guid)
    }

    /// Hand `amount` out of a carried `stack` to a teammate.
    ///
    /// The server takes whole objects: a give of part of a stack goes
    /// into the void unanswered. So anything short of the whole stack
    /// is counted out into a stack of its own first, and that is what
    /// changes hands. `None` while the counting-out is still going on,
    /// `Some(true)` when a give went out, `Some(false)` when the thing
    /// will not come apart and the party should get on without it.
    fn hand_stack(&mut self, to: u32, stack: u32, amount: u32, now: Instant) -> Option<bool> {
        let (whole, wcid) = self
            .world
            .objects
            .get(&stack)
            .map(|o| (o.stack_size.max(1), o.weenie_class_id))
            .unwrap_or((1, 0));
        let piece = if amount >= whole {
            Some(stack)
        } else {
            self.piece_of(Some(wcid), amount)
        };
        if let Some(g) = piece {
            if self.give(to, g, None) {
                self.autoplay.last_give = Some(now);
                self.autoplay
                    .give_tries
                    .note(stack, &crate::did::Did::Done, now);
                return Some(true);
            }
            return Some(false);
        }
        // The counting-out has been asked for and the piece has not
        // appeared. Waiting rather than blocked: a split is one message
        // and the server answers in its own time, so the first few asks
        // are a quarter of a second apart and double from there.
        if self
            .autoplay
            .give_tries
            .waited(&stack)
            .is_some_and(|w| w > WILL_NOT_SPLIT)
        {
            return Some(false);
        }
        self.autoplay.give_tries.note(
            stack,
            &crate::did::Did::waiting("the money has not come apart yet"),
            now,
        );
        self.split_stack(stack, None, amount);
        self.autoplay.last_give = Some(now);
        None
    }

    /// Give everyone what they ordered.
    fn unload_the_quartermaster(&mut self, growth: &crate::growth::Growth, now: Instant) -> bool {
        let party = self.party_supplies(growth, now);
        // Work the whole party's split out for each thing carried, so
        // that a short run is shared rather than filling the first
        // order and leaving the last character with nothing.
        for (item, _) in crate::logistics::merged_order(&party) {
            let carried: Vec<(u32, u32, String)> = self
                .world
                .inventory()
                .filter(|o| o.name.eq_ignore_ascii_case(&item))
                .map(|o| (o.guid, o.stack_size.max(1), o.name.clone()))
                .collect();
            let brought: u32 = carried.iter().map(|(_, n, _)| n).sum();
            if brought == 0 {
                continue;
            }
            for (who, share) in crate::logistics::hand_out(&party, &item, brought) {
                if who == self.world.stats.name || share == 0 {
                    continue;
                }
                let Some(mate) = self
                    .autoplay
                    .team
                    .mates
                    .iter()
                    .find(|m| m.name == who)
                    .cloned()
                else {
                    continue;
                };
                if !self.step_into_reach(mate.world) {
                    if self.reaching_too_long(mate.world, now) {
                        self.autoplay
                            .note(format!("cannot get to {who} to hand out"), now);
                        continue;
                    }
                    self.autoplay
                        .say(Doing::Helping, format!("taking {who} their supplies"));
                    return true;
                }
                let (guid, stack, name) = carried[0].clone();
                let share = share.min(stack);
                match self.hand_stack(mate.guid, guid, share, now) {
                    Some(true) => {
                        self.autoplay
                            .say(Doing::Helping, format!("giving {share} {name} to {who}"));
                        return true;
                    }
                    None => {
                        self.autoplay.say(
                            Doing::Helping,
                            format!("counting out {share} {name} for {who}"),
                        );
                        return true;
                    }
                    Some(false) => continue,
                }
            }
        }
        false
    }

    /// The coin this character can hand over, as `(stack, amount)`:
    /// what it carries less the float it keeps for itself.
    ///
    /// Coin, not trade notes. A note is the lighter way to carry a
    /// fortune, but the server charges 1.15 times a note's face value
    /// to make one and pays only face value to cash it back, so turning
    /// a purse into notes and back costs the party thirteen percent of
    /// it. Handing over pyreals costs nothing and buys exactly as much.
    fn spare_coin(&self) -> Option<(u32, u32)> {
        let float = self.autoplay.config.team.restock.float;
        let purse = self
            .world
            .inventory()
            .filter(|o| o.item_type & ac_world::item_type::MONEY != 0)
            .map(|o| (o.guid, o.stack_size.max(1)))
            .max_by_key(|(_, n)| *n)?;
        let spare = purse.1.checked_sub(float)?;
        (spare > 0).then_some((purse.0, spare))
    }

    /// Walk towards a spot until close enough to hand something over.
    /// True once in reach.
    fn step_into_reach(&mut self, spot: glam::Vec3) -> bool {
        let Some(me) = self.my_position() else {
            return false;
        };
        if glam::Vec2::new(spot.x - me.x, spot.y - me.y).length() <= Self::REACH {
            self.autoplay.reaching = None;
            if self.follow.take().is_some() {
                self.steering.reset();
            }
            return true;
        }
        self.head_for(spot, Self::REACH * 0.6, "the spot");
        false
    }

    /// Whether walking to `spot` has gone on too long to be walking any
    /// more. Indoors a counter can stand behind a wall the steering
    /// cannot get round, and a teammate can be in the next building;
    /// without this the character presses towards it for ever.
    ///
    /// The clock starts when a new spot is aimed at and is reset by any
    /// real progress towards it, so a long walk is fine and a stopped
    /// one is not.
    fn reaching_too_long(&mut self, spot: glam::Vec3, now: Instant) -> bool {
        let Some(me) = self.my_position() else {
            return false;
        };
        let away = glam::Vec2::new(spot.x - me.x, spot.y - me.y).length();
        match self.autoplay.reaching {
            Some((at, best, since)) if at.distance(spot) < 1.0 => {
                if away < best - REACH_PROGRESS {
                    self.autoplay.reaching = Some((spot, away, now));
                    return false;
                }
                if now.duration_since(since) > REACH_GIVE_UP {
                    self.autoplay.reaching = None;
                    return true;
                }
                false
            }
            _ => {
                self.autoplay.reaching = Some((spot, away, now));
                false
            }
        }
    }
}

impl Client {
    /// Whether this character is carrying something another character
    /// asked for. On a quartermaster run that is how the party knows
    /// the runner still has goods to hand out; on any other it is
    /// simply false, since nobody has asked it for anything.
    ///
    /// Worked out from the others' orders alone, never from who the
    /// runner is: the runner is chosen from these reports, so asking
    /// would be circular.
    pub(crate) fn holding_orders(&self) -> bool {
        let me = self.world.stats.name.as_str();
        let wanted: Vec<&str> = self
            .autoplay
            .team
            .mates
            .iter()
            .filter(|m| m.name != me)
            .flat_map(|m| m.supplies.order.iter())
            .filter(|(_, count)| *count > 0)
            .map(|(name, _)| name.as_str())
            .collect();
        if wanted.is_empty() {
            return false;
        }
        self.world
            .inventory()
            .any(|o| wanted.iter().any(|w| o.name.eq_ignore_ascii_case(w)))
    }

    /// Who is doing the party's shopping, when it sends one character
    /// rather than all going.
    pub fn quartermaster_name(&self, cfg: &Growth, now: Instant) -> Option<String> {
        if self.autoplay.config.team.restock.plan != crate::logistics::Plan::Quartermaster {
            return None;
        }
        crate::logistics::quartermaster(&self.party_supplies(cfg, now)).map(|m| m.name.clone())
    }

    /// Whether this character is the one doing the shopping.
    pub fn is_quartermaster(&self, cfg: &Growth, now: Instant) -> bool {
        let me = self.world.stats.name.as_str();
        !me.is_empty() && self.quartermaster_name(cfg, now).as_deref() == Some(me)
    }
}

#[cfg(test)]
mod tests;
