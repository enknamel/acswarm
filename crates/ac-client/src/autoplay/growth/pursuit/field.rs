//! A pursued quest done by using a carried item while standing in an object that hears its
//! signal (`Handing::Use`: the Aetheria fields, Gem.cs:69-72, Landblock.cs:985-998): the object
//! walked to, the creatures its `clears` names killed there (the fight step takes them), the body
//! stood inside it, the item used. The server's quest log, asked after, says whether it took.

use std::time::{Duration, Instant};

use ac_world::quests::{Handing, Row, Want};
use ac_world::Place;

use super::{Ask, Hunting, PursuitLine, PursuitStep};
use crate::autoplay::Doing;
use crate::quest_log::QuestLine;
use crate::Client;

/// The walk onto the object ends this near its centre (metres): ACE hears the use only from a
/// body whose cylinder overlaps it, radius 0 (HearLocalSignalsRadius, WorldObject_Use.cs:47-55).
const FIELD_STOP: f32 = 0.3;
/// Further than this from its centre (metres, flat), the body is walked onto it again.
const IN_FIELD: f32 = 0.6;
/// Further than this (metres, 3D), the object is travelled to: another room or floor.
const FIELD_NEAR: f32 = 20.0;
/// Creatures to clear count this near the object (metres): the wisps stand 2 m off (data).
const CLEAR_NEAR: f32 = 15.0;
/// Stood at the object this long before the first use: what stands there has been sent (guess).
const LOOK_FIRST: Duration = Duration::from_secs(2);
/// After a use, the server's quest log is asked this long on: its emotes run first (guess).
const ASK_AFTER: Duration = Duration::from_secs(3);
/// A use unanswered this long counts as silent: past the stone's 10 s cooldown (weenie 42710,
/// PropertyFloat 167), so the next may go out.
const USE_ANSWER: Duration = Duration::from_secs(12);
/// Silent uses before the quest is passed over: an event off says nothing (EmoteManager.cs:2055).
const USE_TRIES: u32 = 3;

/// The object a pursued quest's item is used in, as stood in, and the uses sent.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Using {
    /// The quest it is for: another working pursuit starts afresh.
    pub(crate) quest: String,
    /// When the body first stood at the object.
    pub(crate) arrived: Instant,
    /// When the last use went out, and whether the quest log has been asked since.
    pub(crate) sent: Option<(Instant, bool)>,
    pub(crate) tries: u32,
}

impl Ask<'_> {
    /// The use of `line`'s carried item in the nearest object it is used in, when one of its
    /// rows is done so and the item is carried; the creatures to kill there first are sought.
    pub(super) fn use_step(&self, line: &QuestLine) -> Option<(PursuitStep, Option<Hunting>)> {
        let rows = crate::quest_log::worked_rows(line).filter(|r| r.hand == Handing::Use);
        let mut best: Option<(f32, &'static Row, Place)> = None;
        for r in rows.filter(|r| r.fits_level(self.facts.level)) {
            let Want::Item(item) = r.want else { continue };
            if (self.asking.carried)(item) == 0 {
                continue;
            }
            let open = r
                .places
                .iter()
                .filter(|p| ac_world::quests::locked_at(p, &self.facts.held).is_none());
            for p in open {
                let away = (self.asking.away)(p);
                if best.as_ref().is_none_or(|b| away < b.0) {
                    best = Some((away, r, *p));
                }
            }
        }
        let (_, row, at) = best?;
        let hunting = Hunting {
            sought: row.clears.iter().map(|c| c.0).collect(),
            ..Hunting::default()
        };
        Some((PursuitStep::UseHere { row, at }, Some(hunting)))
    }
}

impl Client {
    /// The working pursuit's walk to the object its item is used in, the creatures there
    /// cleared, and the use, from `grow_pursuit`.
    pub(super) fn grow_field(&mut self, line: &PursuitLine, now: Instant) -> Option<bool> {
        let PursuitStep::UseHere { row, at } = line.step else {
            return None;
        };
        let Want::Item(item) = row.want else {
            return None;
        };
        let st = &mut self.autoplay.growth;
        if st.using.as_ref().is_some_and(|u| u.quest != line.id) {
            st.using = None;
        }
        let me = self.my_position()?;
        if me.distance(at.at) > FIELD_NEAR {
            self.autoplay.growth.using = None;
            if self.journey_to(at, FIELD_STOP) {
                let why = format!("for {}: on the way to {}", line.title, row.giver);
                self.autoplay.say(Doing::Traveling, why);
                return Some(true);
            }
            self.hold_pursuit(&line.id, format!("no way to {}", row.giver), now);
            return Some(false);
        }
        // The creatures it asks dead: the fight step, ahead of this one, kills them.
        let alive = self.left_to_clear(row, at);
        if alive > 0 {
            let why = format!("for {}: {alive} to kill at {}", line.title, row.giver);
            self.autoplay.say(Doing::Fighting, why);
            return Some(true);
        }
        if me.truncate().distance(at.at.truncate()) > IN_FIELD {
            if self.journey_to(at, FIELD_STOP) {
                let why = format!("for {}: into {}", line.title, row.giver);
                self.autoplay.say(Doing::Traveling, why);
                return Some(true);
            }
            self.hold_pursuit(&line.id, format!("no way into {}", row.giver), now);
            return Some(false);
        }
        let fresh = Using {
            quest: line.id.clone(),
            arrived: now,
            sent: None,
            tries: 0,
        };
        let using = self.autoplay.growth.using.get_or_insert(fresh).clone();
        let waited = |since: Instant, d: Duration| now.saturating_duration_since(since) >= d;
        match using.sent {
            None if !waited(using.arrived, LOOK_FIRST) => return Some(true),
            Some((at_use, asked)) if !waited(at_use, USE_ANSWER) => {
                // Confirmed only by the server's quest log (or the stat it sets), never by words.
                if !asked && waited(at_use, ASK_AFTER) {
                    if let Some(u) = self.autoplay.growth.using.as_mut() {
                        u.sent = Some((at_use, true));
                    }
                    self.server_quests_due = true;
                }
                let why = format!("for {}: waiting on {}", line.title, row.giver);
                self.autoplay.say(Doing::Idle, why);
                return Some(true);
            }
            Some(_) if using.tries >= USE_TRIES => {
                self.autoplay.growth.using = None;
                let why = format!("{} said nothing to the stone {USE_TRIES} times", row.giver);
                self.hold_pursuit(&line.id, why, now);
                return Some(false);
            }
            _ => {}
        }
        // Stood still: the body must overlap the object where the server has it when the use lands.
        if self.traveling() || self.is_walking() {
            self.interrupt_travel("in the field");
            self.stop_going(None, "in the field");
            return Some(true);
        }
        if self.hands_busy(now) {
            return Some(true);
        }
        let held = self
            .world
            .inventory()
            .find(|o| o.weenie_class_id == item && o.wielder.is_none())?
            .guid;
        self.use_object(held);
        if let Some(u) = self.autoplay.growth.using.as_mut() {
            u.sent = Some((now, false));
            u.tries += 1;
        }
        let name = row.item.as_ref().map_or("it", |i| i.name);
        let why = format!("for {}: using {name} in {}", line.title, row.giver);
        tracing::info!("pursuit: {}: {why}", line.id);
        self.autoplay.say(Doing::Idle, why);
        Some(true)
    }

    /// How many of the creatures `row` asks killed stand alive within [`CLEAR_NEAR`] of `at`.
    fn left_to_clear(&self, row: &Row, at: Place) -> usize {
        let wanted = |w: u32| row.clears.iter().any(|c| c.0 == w);
        let me = self.world.player_guid;
        self.world
            .objects
            .values()
            .filter(|o| wanted(o.weenie_class_id) && Some(o.guid) != me)
            .filter(|o| o.health.is_none_or(|h| h > 0.0))
            .filter(|o| {
                o.world_pos()
                    .is_some_and(|p| p.distance(at.at) <= CLEAR_NEAR)
            })
            .count()
    }
}

/// A field step in words: "kill 4 Shivering Crystalline Wisp, then use Aligned Mana Stone ...".
pub(super) fn use_words(row: &Row) -> String {
    let item = row.item.as_ref().map_or("it", |i| i.name);
    let kill = |&(wcid, n): &(u32, u32)| {
        let name = ac_world::elements::creature_by_id(wcid);
        let name = name.map_or_else(|| format!("creature {wcid}"), |c| c.name.clone());
        format!("kill {n} {name}, then ")
    };
    let first: String = row.clears.iter().map(kill).collect();
    format!("{first}use {item} standing in {}", row.giver)
}
