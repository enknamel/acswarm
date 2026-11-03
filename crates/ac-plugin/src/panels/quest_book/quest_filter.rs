//! The Quests window's filter: the search parsed into facet words, the Show, Kind, State,
//! Repeat, Does and level filters as one pure test over a card, the browse board, and the order.

use std::collections::HashMap;
use std::time::Instant;

use ac_client::autoplay::Quests;
use ac_client::quest_log::{QuestLine, QuestState, QUEST_BOARD_EVERY};
use ac_client::Client;
use ac_world::quest_chain::QuestCard;
use ac_world::quests::{Kind, Repeat};

/// Where a search word looks: anywhere, or one facet named by its prefix (`item:jaw`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Facet {
    Any,
    Item,
    Creature,
    At,
    Giver,
    Reward,
    Flag,
}

impl Facet {
    const NAMED: [Facet; 6] = [
        Facet::Item,
        Facet::Creature,
        Facet::At,
        Facet::Giver,
        Facet::Reward,
        Facet::Flag,
    ];

    /// The prefix typed before the colon.
    pub(crate) fn word(self) -> &'static str {
        match self {
            Facet::Any => "",
            Facet::Item => "item",
            Facet::Creature => "creature",
            Facet::At => "at",
            Facet::Giver => "giver",
            Facet::Reward => "reward",
            Facet::Flag => "flag",
        }
    }

    fn of(prefix: &str) -> Option<Facet> {
        Facet::NAMED
            .into_iter()
            .find(|f| f.word().eq_ignore_ascii_case(prefix))
    }
}

/// The search split into lowercased words, each with its facet; a quoted phrase is one word.
pub(crate) fn parse_search(text: &str) -> Vec<(Facet, String)> {
    let mut out = Vec::new();
    let mut rest = text.trim_start();
    while !rest.is_empty() {
        let mut facet = Facet::Any;
        if let Some((head, tail)) = rest.split_once(':') {
            let plain = !head.contains(|c: char| c.is_whitespace() || c == '"');
            if let Some(f) = Facet::of(head).filter(|_| plain) {
                facet = f;
                rest = tail;
            }
        }
        let (word, after) = match rest.strip_prefix('"') {
            Some(quoted) => quoted.split_once('"').unwrap_or((quoted, "")),
            None => rest.split_once(char::is_whitespace).unwrap_or((rest, "")),
        };
        let word = word.trim().to_lowercase();
        if !word.is_empty() {
            out.push((facet, word));
        }
        rest = after.trim_start();
    }
    out
}

/// `facet:value` for the search, the value quoted when it holds a space.
pub(crate) fn search_word(facet: Facet, value: &str) -> String {
    let value = if value.contains(char::is_whitespace) {
        format!("\"{value}\"")
    } else {
        value.to_string()
    };
    match facet {
        Facet::Any => value,
        f => format!("{}:{value}", f.word()),
    }
}

/// Whether `hay` holds `needle` (lowercased), ignoring ASCII case, with no allocation.
pub(crate) fn has_word(hay: &str, needle: &str) -> bool {
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    n.is_empty() || h.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n))
}

/// Whether any of `list` holds `word`.
pub(crate) fn has_any<'s>(mut list: impl Iterator<Item = &'s str>, word: &str) -> bool {
    list.any(|s| has_word(s, word))
}

/// Whether the card has `word` in `facet` (anywhere for [`Facet::Any`]).
pub(crate) fn has_term(card: &QuestCard, facet: Facet, word: &str) -> bool {
    let item = || card.items.iter().map(String::as_str);
    let creatures = || card.creatures.iter().map(|c| c.0.as_str());
    let givers = || card.givers.iter().map(|g| g.0.as_str());
    let places = || {
        let at = card.givers.iter().map(|g| g.1.as_str());
        card.places.iter().map(String::as_str).chain(at)
    };
    let gives = || card.gives.iter().map(String::as_str);
    let flags = || card.flags.iter().map(String::as_str);
    match facet {
        Facet::Item => has_any(item(), word),
        Facet::Creature => has_any(creatures(), word),
        Facet::At => has_any(places(), word),
        Facet::Giver => has_any(givers(), word),
        Facet::Reward => has_any(gives(), word),
        Facet::Flag => has_any(flags(), word),
        Facet::Any => {
            has_word(&card.title, word)
                || has_word(&card.id, word)
                || has_any(
                    item()
                        .chain(creatures())
                        .chain(givers())
                        .chain(places())
                        .chain(gives())
                        .chain(flags()),
                    word,
                )
        }
    }
}

/// Which quests the list shows before the search.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Show {
    /// Those whose giver's band and easiest source suit the character's level.
    #[default]
    ForLevel,
    Pursued,
    On,
    All,
}

impl Show {
    pub(crate) const ALL: [Show; 4] = [Show::ForLevel, Show::Pursued, Show::On, Show::All];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Show::ForLevel => "for my level",
            Show::Pursued => "pursued",
            Show::On => "switched on",
            Show::All => "all",
        }
    }
}

/// The State filter, read off the browse board.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum StateChip {
    #[default]
    Any,
    Ready,
    Needs,
    NotTaken,
    Waiting,
    CantYet,
    WontComplete,
    Done,
}

impl StateChip {
    pub(crate) const ALL: [StateChip; 8] = [
        StateChip::Any,
        StateChip::Ready,
        StateChip::Needs,
        StateChip::NotTaken,
        StateChip::Waiting,
        StateChip::CantYet,
        StateChip::WontComplete,
        StateChip::Done,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            StateChip::Any => "any state",
            StateChip::Ready => "ready",
            StateChip::Needs => "needs more",
            StateChip::NotTaken => "not taken up",
            StateChip::Waiting => "waiting",
            StateChip::CantYet => "can't yet",
            StateChip::WontComplete => "won't complete",
            StateChip::Done => "done",
        }
    }

    /// Whether a quest in `state` passes; one with no state yet passes only "any".
    pub(crate) fn is_kept(self, state: Option<&QuestState>) -> bool {
        let Some(state) = state else {
            return self == StateChip::Any;
        };
        match self {
            StateChip::Any => true,
            StateChip::Ready => *state == QuestState::Ready,
            StateChip::Needs => matches!(state, QuestState::Needs { .. }),
            StateChip::NotTaken => matches!(state, QuestState::NotTaken { .. }),
            StateChip::Waiting => matches!(state, QuestState::Waiting { .. }),
            StateChip::CantYet => matches!(state, QuestState::CantYet(_)),
            StateChip::WontComplete => matches!(state, QuestState::WontComplete { .. }),
            StateChip::Done => *state == QuestState::Done,
        }
    }
}

/// How often a quest may be done: the Repeat chips.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum RepeatChip {
    #[default]
    Any,
    AnyTime,
    /// A wait of a day or less.
    Daily,
    /// More than a day, up to seven.
    Weekly,
    Longer,
    Once,
}

/// Seconds in a day and a week: the Repeat chips' bounds.
pub(crate) const DAY: u32 = 86_400;
pub(crate) const WEEK: u32 = 7 * DAY;

impl RepeatChip {
    pub(crate) const ALL: [RepeatChip; 6] = [
        RepeatChip::Any,
        RepeatChip::AnyTime,
        RepeatChip::Daily,
        RepeatChip::Weekly,
        RepeatChip::Longer,
        RepeatChip::Once,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            RepeatChip::Any => "any",
            RepeatChip::AnyTime => "any time",
            RepeatChip::Daily => "daily",
            RepeatChip::Weekly => "weekly",
            RepeatChip::Longer => "longer",
            RepeatChip::Once => "once",
        }
    }

    pub(crate) fn is_kept(self, repeat: Repeat) -> bool {
        match (self, repeat) {
            (RepeatChip::Any, _) => true,
            (RepeatChip::AnyTime, Repeat::Any) | (RepeatChip::Once, Repeat::Once) => true,
            (RepeatChip::Daily, Repeat::After(s)) => s <= DAY,
            (RepeatChip::Weekly, Repeat::After(s)) => s > DAY && s <= WEEK,
            (RepeatChip::Longer, Repeat::After(s)) => s > WEEK,
            _ => false,
        }
    }
}

/// The data's `does` words the Does filter offers, with their labels.
pub(crate) const DOES: &[(&str, &str)] = &[
    ("xp", "XP"),
    ("item", "an item"),
    ("flag", "a flag"),
    ("luminance", "luminance"),
    ("spell", "a spell"),
    ("skill", "a skill"),
    ("stat", "a stat"),
    ("title", "a title"),
    ("teleport", "a teleport"),
    ("contract", "a contract"),
    ("enlightenment", "enlightenment"),
    ("takes", "takes items"),
    ("type", "something else"),
];

/// Whether one of the card's `does` words is `word` ("type 75" is `type`).
pub(crate) fn has_deed(card: &QuestCard, word: &str) -> bool {
    card.does.iter().any(|d| d.split(' ').next() == Some(word))
}

/// The character the list is filtered for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Me {
    pub level: u32,
    /// `Growth::level_margin`.
    pub margin: u32,
}

/// The filter bar.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Filter {
    pub search: String,
    pub show: Show,
    /// `None` for any kind.
    pub kind: Option<Kind>,
    pub state: StateChip,
    pub repeat: RepeatChip,
    /// A [`DOES`] word, `None` for anything.
    pub does: Option<&'static str>,
    /// Lowest and highest quest level, 0 for open.
    pub levels: (u32, u32),
}

/// The level a quest asks: the giver's lowest, the easiest source's, the way in's, whichever is
/// highest.
pub(crate) fn quest_level(card: &QuestCard) -> u32 {
    card.levels.0.max(card.easiest.unwrap_or(0)).max(card.entry)
}

/// Whether the giver's band holds the level and the easiest source is within twice the margin
/// above it, as the board's `Blocked::TooHard` judges.
pub(crate) fn is_suited(card: &QuestCard, me: Me) -> bool {
    let (min, max) = card.levels;
    let band = me.level >= min && (max == 0 || me.level <= max);
    let source = card.easiest.is_none_or(|e| e <= me.level + 2 * me.margin);
    band && source && card.entry <= me.level
}

/// Whether `id` is on for autoplay: chosen by hand, or paying XP with nothing unread and not off.
/// "do quests" itself is not asked.
pub(crate) fn is_switched_on(cfg: &Quests, id: &str) -> bool {
    cfg.is_chosen(id)
        || (ac_client::quest_log::is_automatic_quest(id) && !cfg.off.iter().any(|o| o == id))
}

/// Switch `id` on or off: one paying XP on its own is taken off the off list, any other chosen.
pub(crate) fn set_switch(cfg: &mut Quests, id: &str, on: bool) {
    if !on {
        cfg.switch_off(id);
    } else if ac_client::quest_log::is_automatic_quest(id) {
        cfg.off.retain(|o| o != id);
    } else {
        cfg.switch_on(id);
    }
}

/// Whether the list shows `card`, whose browse line is `line`, under `f` with its search parsed
/// into `terms` ([`parse_search`]): every word must match.
pub(crate) fn is_listed(
    card: &QuestCard,
    line: Option<&QuestLine>,
    cfg: &Quests,
    f: &Filter,
    terms: &[(Facet, String)],
    me: Me,
) -> bool {
    let shown = match f.show {
        Show::ForLevel => is_suited(card, me),
        Show::Pursued => cfg.is_pursued(&card.id),
        Show::On => is_switched_on(cfg, &card.id),
        Show::All => true,
    };
    let level = quest_level(card);
    let (lo, hi) = f.levels;
    let in_band = (lo == 0 || (level >= lo && (card.levels.1 == 0 || card.levels.1 >= lo)))
        && (hi == 0 || level <= hi);
    shown
        && in_band
        && f.kind
            .is_none_or(|k| card.kinds.contains(&k) || card.kind == k)
        && f.repeat.is_kept(card.repeat)
        && f.does.is_none_or(|w| has_deed(card, w))
        && f.state.is_kept(line.map(|l| &l.state))
        && terms.iter().all(|(facet, w)| has_term(card, *facet, w))
}

/// A list column the header sorts by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Column {
    Title,
    Kind,
    Giver,
    Where,
    Levels,
    #[default]
    Xp,
    Repeat,
    State,
}

impl Column {
    pub(crate) const ALL: [Column; 8] = [
        Column::Title,
        Column::Kind,
        Column::Giver,
        Column::Where,
        Column::Levels,
        Column::Xp,
        Column::Repeat,
        Column::State,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Column::Title => "Quest",
            Column::Kind => "Kind",
            Column::Giver => "Giver",
            Column::Where => "Where",
            Column::Levels => "Levels",
            Column::Xp => "XP",
            Column::Repeat => "Repeat",
            Column::State => "State",
        }
    }
}

/// The list's order: a column, highest first when `descending`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Sort {
    pub col: Column,
    pub descending: bool,
}

impl Default for Sort {
    /// XP, highest first.
    fn default() -> Self {
        Sort {
            col: Column::Xp,
            descending: true,
        }
    }
}

/// The browse board: every quest's state as if on, worked out every 2 s while the window is open.
#[derive(Debug, Default)]
pub(crate) struct Browse {
    at: Option<Instant>,
    lines: Vec<QuestLine>,
    by_id: HashMap<String, usize>,
    /// Bumped on each new board, so the list is filtered again.
    pub(crate) stamp: u64,
}

impl Browse {
    pub(crate) fn set(&mut self, lines: Vec<QuestLine>, now: Instant) {
        self.by_id = lines
            .iter()
            .enumerate()
            .map(|(i, l)| (l.id.clone(), i))
            .collect();
        self.lines = lines;
        self.at = Some(now);
        self.stamp += 1;
    }

    /// Work the board out for `c` when [`QUEST_BOARD_EVERY`] has passed.
    pub(crate) fn refresh(&mut self, c: &Client, now: Instant) {
        if self
            .at
            .is_some_and(|at| now.duration_since(at) < QUEST_BOARD_EVERY)
        {
            return;
        }
        let facts = c.quest_facts();
        self.set(ac_client::quest_log::browse_board(&facts), now);
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub(crate) fn line(&self, id: &str) -> Option<&QuestLine> {
        self.by_id.get(id).map(|&i| &self.lines[i])
    }
}

/// The giver the list names: the browse line's nearest, else the card's first.
pub(crate) fn giver_of<'c>(card: &'c QuestCard, line: Option<&'c QuestLine>) -> (&'c str, &'c str) {
    let name = line.map_or_else(
        || card.givers.first().map_or("", |g| g.0.as_str()),
        |l| l.giver.as_str(),
    );
    let at = card
        .givers
        .iter()
        .find(|g| g.0 == name)
        .or(card.givers.first())
        .map_or("", |g| g.1.as_str());
    (name, at)
}

/// The XP at the character's level when the board has it, else the card's fixed XP.
pub(crate) fn xp_of(card: &QuestCard, line: Option<&QuestLine>) -> u64 {
    line.map_or(card.xp, |l| l.xp)
}

/// Any time first, then by wait, once last.
pub(crate) fn repeat_rank(r: Repeat) -> u64 {
    match r {
        Repeat::Any => 0,
        Repeat::After(s) => u64::from(s),
        Repeat::Once => u64::MAX,
    }
}

/// Ready first, then what is going on, then what cannot be done.
pub(crate) fn state_rank(state: Option<&QuestState>) -> u8 {
    match state {
        Some(QuestState::Ready) => 0,
        Some(QuestState::Needs { .. }) => 1,
        Some(QuestState::NotTaken { .. }) => 2,
        Some(QuestState::WontComplete { .. }) => 3,
        Some(QuestState::Waiting { .. }) => 4,
        Some(QuestState::CantYet(_)) => 5,
        Some(QuestState::Done) => 6,
        Some(QuestState::Off) | None => 7,
    }
}

/// Order `rows` (card indices) by `sort`; ties keep the data's order.
pub(crate) fn sort_rows(rows: &mut [usize], cards: &[QuestCard], browse: &Browse, sort: Sort) {
    let line = |i: usize| browse.line(&cards[i].id);
    let text = |i: usize| -> String {
        let card = &cards[i];
        match sort.col {
            Column::Title => card.title.to_lowercase(),
            Column::Kind => kind_word(card.kind).to_string(),
            Column::Giver => giver_of(card, line(i)).0.to_lowercase(),
            _ => giver_of(card, line(i)).1.to_lowercase(),
        }
    };
    let number = |i: usize| -> u64 {
        let card = &cards[i];
        match sort.col {
            Column::Levels => u64::from(quest_level(card)),
            Column::Xp => xp_of(card, line(i)),
            Column::Repeat => repeat_rank(card.repeat),
            _ => u64::from(state_rank(line(i).map(|l| &l.state))),
        }
    };
    match sort.col {
        Column::Title | Column::Kind | Column::Giver | Column::Where => {
            order_by(rows, text, sort.descending)
        }
        _ => order_by(rows, number, sort.descending),
    }
}

/// Sort `rows` by `key`, highest first when `descending`, ties by index.
pub(crate) fn order_by<K: Ord>(rows: &mut [usize], key: impl Fn(usize) -> K, descending: bool) {
    let mut keyed: Vec<(K, usize)> = rows.iter().map(|&i| (key(i), i)).collect();
    keyed.sort_by(|a, b| {
        let by = a.0.cmp(&b.0);
        let by = if descending { by.reverse() } else { by };
        by.then(a.1.cmp(&b.1))
    });
    for (slot, (_, i)) in rows.iter_mut().zip(keyed) {
        *slot = i;
    }
}

pub(crate) fn kind_word(kind: Kind) -> &'static str {
    match kind {
        Kind::Item => "item",
        Kind::Kill => "kill",
        Kind::Talk => "talk",
        Kind::Start => "take-up",
    }
}
