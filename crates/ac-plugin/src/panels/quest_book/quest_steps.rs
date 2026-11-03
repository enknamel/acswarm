//! A quest's chain and a pursued quest's step in the Quests window's words, each name a link
//! that fills the search; the steps a card alone tells while the data has no chain.

use ac_client::autoplay::{PursuitLine, PursuitStep, Quests};
use ac_client::quest_log::QuestLog;
use ac_world::quest_chain::{ChainLink, FlagBy, QuestCard, SourceGround};
use ac_world::quests::{Handing, Kind, Row, Source, Want, Yield};

use super::super::quest_list::time_left;
use super::quest_filter::{Facet, Me};

/// A run of a step's words: plain, or a name that fills the search when clicked.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Piece {
    Text(String),
    Link(Facet, String),
}

/// One step of a quest's chain in words.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Step {
    pub pieces: Vec<Piece>,
    /// Held or done for this character already.
    pub done: bool,
}

impl Step {
    pub(crate) fn say(mut self, text: impl Into<String>) -> Self {
        self.pieces.push(Piece::Text(text.into()));
        self
    }

    pub(crate) fn link(mut self, facet: Facet, name: impl Into<String>) -> Self {
        self.pieces.push(Piece::Link(facet, name.into()));
        self
    }

    /// The step's words run together.
    #[cfg(test)]
    pub(crate) fn text(&self) -> String {
        self.pieces
            .iter()
            .map(|p| match p {
                Piece::Text(t) | Piece::Link(_, t) => t.as_str(),
            })
            .collect()
    }
}

/// What a chain's words are checked against: the character, and a cell's place in words.
pub(crate) struct Known<'a> {
    pub held: &'a [String],
    pub log: &'a QuestLog,
    pub me: Me,
    pub place: &'a dyn Fn(u32) -> String,
}

impl Known<'_> {
    pub(crate) fn is_held(&self, flag: &str) -> bool {
        self.held.iter().any(|h| h.eq_ignore_ascii_case(flag))
    }
}

/// The best few hunting places a Hunt step names; the rest are counted.
pub(crate) const HUNT_PLACES: usize = 3;

/// A quest's chain ([`ac_world::quest_chain::chain_of`]) in numbered-step words.
pub(crate) fn chain_words(links: &[ChainLink], k: &Known) -> Vec<Step> {
    let mut steps = Vec::new();
    for link in links {
        match link {
            ChainLink::TakeUp(row) => {
                let taken = row
                    .stamps
                    .iter()
                    .any(|s| k.log.kills.contains_key(s) || k.is_held(s));
                let mut step = Step::default().say("Take it up from ");
                step = giver_at(step, row, k);
                step.done = taken;
                steps.push(step);
            }
            ChainLink::Flag { flag, by, portal } => steps.push(flag_step(flag, by, *portal, k)),
            ChainLink::Hunt(grounds) => {
                for (i, g) in grounds.iter().take(HUNT_PLACES).enumerate() {
                    let mut step = hunt_step(g, k);
                    let more = grounds.len().saturating_sub(HUNT_PLACES);
                    if i + 1 == HUNT_PLACES.min(grounds.len()) && more > 0 {
                        step = step.say(format!(" (or {more} more places)"));
                    }
                    steps.push(step);
                }
            }
            ChainLink::Fetch(source) => {
                let cell = source.landblocks.first().map_or(0, |l| l.0);
                steps.push(
                    Step::default()
                        .say("Get it from ")
                        .link(Facet::Creature, &source.name)
                        .say(" at ")
                        .link(Facet::At, (k.place)(cell))
                        .say(" (autoplay can't fetch this yet)"),
                );
            }
            ChainLink::HandIn(rows) => {
                if let Some(step) = hand_in_step(rows, k) {
                    steps.push(step);
                }
            }
        }
    }
    steps
}

/// "<giver> at <place>", at the giver's first placement.
pub(crate) fn giver_at(step: Step, row: &Row, k: &Known) -> Step {
    let cell = row.places.first().map_or(0, |p| p.cell);
    let step = step
        .link(Facet::Giver, row.giver)
        .say(" at ")
        .link(Facet::At, (k.place)(cell));
    match row.places.len() {
        0 | 1 => step,
        n => step.say(format!(" (or {} other places)", n - 1)),
    }
}

pub(crate) fn flag_step(
    flag: &str,
    by: &[FlagBy],
    portal: Option<&ac_world::portals::Portal>,
    k: &Known,
) -> Step {
    let mut step = Step::default().say("Flag ").link(Facet::Flag, flag);
    step = match by.first() {
        None => step.say(": nobody known gives it (autoplay can't get it)"),
        Some(FlagBy::Unlock(u)) => step
            .say(" from ")
            .link(Facet::Giver, &u.npc)
            .say(" at ")
            .link(Facet::At, (k.place)(u.cell))
            .say(format!(" (level {})", u.min_level)),
        Some(FlagBy::Quest(row)) => giver_at(step.say(" from "), row, k)
            .say(" by '")
            .say(row.title.trim())
            .say("'"),
    };
    if by.len() > 1 {
        step = step.say(format!(" (or {} others)", by.len() - 1));
    }
    if let Some(p) = portal {
        step = step.say(format!(
            ", to enter the {} portal (level {})",
            p.name, p.min_level
        ));
    }
    step.done = k.is_held(flag);
    step
}

pub(crate) fn hunt_step(g: &SourceGround, k: &Known) -> Step {
    let s: &Source = g.source;
    let verb = match s.kind {
        Yield::Drop => "Loot ",
        Yield::Kill => "Kill ",
        Yield::Npc => "Get it from ",
    };
    let mut said = format!(" (level {}", s.level);
    if s.kind == Yield::Drop && s.chance > 0.0 && s.chance < 1.0 {
        said += &format!(", 1 in {}", (1.0 / s.chance).round() as u32);
    }
    said += ")";
    let cell = g.place.map_or(g.landblock << 16, |p| p.cell);
    let mut step = Step::default()
        .say(verb)
        .link(Facet::Creature, &s.name)
        .say(said)
        .say(" in ")
        .link(Facet::At, (k.place)(cell))
        .say(match g.entries {
            1 => ", 1 spawn".to_string(),
            n => format!(", {n} spawns"),
        });
    if let Some(p) = g.way_in {
        step = step.say(format!("; way in: {} (level {})", p.name, p.min_level));
        if !p.quest.is_empty() {
            step = step.say(" asking ").link(Facet::Flag, &p.quest);
        }
    }
    if let (Yield::Kill, Want::Kill(counter)) = (s.kind, &s.want) {
        let have = k.log.kills.get(counter).copied().unwrap_or(0);
        step = step.say(format!(" (have {have})"));
    }
    let above = s.level.saturating_sub(k.me.level);
    if k.me.level > 0 && s.level > k.me.level + 2 * k.me.margin {
        step = step.say(format!(" - {above} levels above you"));
    }
    if g.place.is_none() {
        step = step.say(" (no way in known)");
    }
    step
}

pub(crate) fn hand_in_step(rows: &[&Row], k: &Known) -> Option<Step> {
    let row = rows.first()?;
    let step = match (row.kind, row.hand) {
        (Kind::Kill, _) => Step::default().say("Report the kills to "),
        (_, Handing::Give) => {
            let item = row.item.as_ref().map_or("it", |i| i.name);
            let step = Step::default().say("Hand ");
            let step = if row.item.is_some() {
                step.link(Facet::Item, item)
            } else {
                step.say(item)
            };
            step.say(" to ")
        }
        (_, Handing::Talk) => Step::default().say("Talk to "),
    };
    let mut step = giver_at(step, row, k);
    let mut others: Vec<&str> = rows.iter().map(|r| r.giver).collect();
    others.sort_unstable();
    others.dedup();
    if others.len() > 1 {
        step = step.say(format!(" (or {} other givers)", others.len() - 1));
    }
    Some(step)
}

/// The steps a card alone tells, while the data has no chain for the quest.
pub(crate) fn card_steps(card: &QuestCard, k: &Known) -> Vec<Step> {
    let mut steps = Vec::new();
    for flag in &card.flags {
        let mut step = Step::default().say("Hold ").link(Facet::Flag, flag);
        step.done = k.is_held(flag);
        steps.push(step);
    }
    if !card.creatures.is_empty() {
        let mut step = Step::default().say(match card.kind {
            Kind::Kill => "Kill ",
            _ => "From ",
        });
        for (i, (name, level, _)) in card.creatures.iter().take(HUNT_PLACES).enumerate() {
            if i > 0 {
                step = step.say(", ");
            }
            step = step
                .link(Facet::Creature, name)
                .say(format!(" (level {level})"));
        }
        if card.creatures.len() > HUNT_PLACES {
            step = step.say(format!(" or {} more", card.creatures.len() - HUNT_PLACES));
        }
        steps.push(step);
    }
    if !card.places.is_empty() {
        let mut step = Step::default().say("Found in ");
        for (i, place) in card.places.iter().take(HUNT_PLACES).enumerate() {
            if i > 0 {
                step = step.say(", ");
            }
            step = step.link(Facet::At, place);
        }
        if card.places.len() > HUNT_PLACES {
            step = step.say(format!(" or {} more", card.places.len() - HUNT_PLACES));
        }
        steps.push(step);
    }
    if let Some((giver, at)) = card.givers.first() {
        let mut step = match (&card.item, card.kind) {
            (Some(item), Kind::Item) => Step::default()
                .say("Hand ")
                .link(Facet::Item, item)
                .say(" to "),
            (_, Kind::Kill) => Step::default().say("Report the kills to "),
            _ => Step::default().say("Talk to "),
        };
        step = step
            .link(Facet::Giver, giver)
            .say(" at ")
            .link(Facet::At, at);
        if card.givers.len() > 1 {
            step = step.say(format!(" (or {} other places)", card.givers.len() - 1));
        }
        steps.push(step);
    }
    steps
}

/// A pursued quest's step in the window's words.
pub(crate) fn step_words(step: &PursuitStep, now: u64, place: &dyn Fn(u32) -> String) -> String {
    match step {
        PursuitStep::TakeUp { row, at } => {
            format!("taking it up from {} at {}", row.giver, place(at.cell))
        }
        PursuitStep::EarnFlag { flag, from } => {
            format!("earning the {flag} flag from {}", from.npc)
        }
        PursuitStep::Hunt {
            creature,
            place: at,
            more,
            ..
        } => format!("hunting {creature} in {at} ({more} to go)"),
        PursuitStep::HandIn { row, at } => {
            format!("handing it in to {} at {}", row.giver, place(at.cell))
        }
        PursuitStep::Waiting { until } => {
            format!("waits {}", time_left(until.saturating_sub(now)))
        }
        PursuitStep::Finished => "finished".into(),
        PursuitStep::Stuck { why } => format!("passed over: {why}"),
    }
}

/// "working: hunting ...", or a line that waits, with the quest it is done first for.
pub(crate) fn line_words(line: &PursuitLine, now: u64, place: &dyn Fn(u32) -> String) -> String {
    let mut said = step_words(&line.step, now, place);
    if line.working {
        said = format!("working: {said}");
    }
    if let Some(via) = &line.via {
        said += &format!(" (first, for {via})");
    }
    said
}

/// The autoplay window's one pursuit line: the quest worked now, else how many wait.
pub(crate) fn pursuit_summary(
    lines: &[PursuitLine],
    cfg: &Quests,
    now: u64,
    place: &dyn Fn(u32) -> String,
) -> String {
    if let Some(line) = lines.iter().find(|l| l.working) {
        return format!(
            "pursuing {}: {}",
            line.title,
            step_words(&line.step, now, place)
        );
    }
    match cfg.pursue.len() {
        0 => "nothing pursued".into(),
        1 => "1 quest pursued; it can't go on now".into(),
        n => format!("{n} quests pursued; none can go on now"),
    }
}
