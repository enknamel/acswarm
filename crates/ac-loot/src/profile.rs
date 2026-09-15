//! Loot profiles: the first rule whose conditions all hold decides an item; unmatched items stay.
//! Rules ask about the item and about the character reading it, so a party shares one profile.
//! [`Profile::judge`] stops at the first rule that could match but needs an appraisal
//! ([`Verdict::NeedsId`]), so cheap rules placed first settle most items with no round trip.
//! One JSON file each ([`Profile::save`]); [`Library`] shares them live across characters.

use crate::items::{ItemStats, NumKey, Op, Query, Term};
use crate::weapons::Wielder;
use ac_net::messages::Appraisal;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// What to do with an item a loot rule matches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LootAction {
    /// Take it and keep it.
    #[default]
    Keep,
    /// Take it and salvage it (or carry it to whoever salvages).
    Salvage,
    /// Take it to sell on the next run to town.
    Sell,
    /// Leave it on the corpse.
    Skip,
}

impl LootAction {
    pub const ALL: [LootAction; 4] = [
        LootAction::Keep,
        LootAction::Salvage,
        LootAction::Sell,
        LootAction::Skip,
    ];

    pub fn label(self) -> &'static str {
        match self {
            LootAction::Keep => "keep",
            LootAction::Salvage => "salvage",
            LootAction::Sell => "sell",
            LootAction::Skip => "skip",
        }
    }

    /// The action a word names ("keep", "salvage", "sell", "skip").
    pub fn parse(word: &str) -> Option<LootAction> {
        let w = word.trim().to_lowercase();
        LootAction::ALL.into_iter().find(|a| a.label() == w)
    }

    /// Whether an item the rule matches is picked up at all.
    pub fn takes(self) -> bool {
        self != LootAction::Skip
    }

    /// How cautious this action is: the higher, the less it gives away.
    /// Keeping is recoverable and selling or salvaging is not, so the cautious one wins a merge.
    fn caution(self) -> u8 {
        match self {
            LootAction::Keep => 3,
            LootAction::Salvage => 2,
            LootAction::Sell => 1,
            // Nothing is held under a skip, so it loses to anything taken.
            LootAction::Skip => 0,
        }
    }

    /// The more cautious of two decisions, for two stacks of one thing poured together.
    /// The halves may differ: a `keep_up_to` rule keeps up to its cap and the next sells the rest.
    pub fn safer_of(self, other: LootAction) -> LootAction {
        if other.caution() > self.caution() {
            other
        } else {
            self
        }
    }
}

/// Something a rule asks about the character holding it.
/// Only standing features, never the moment (place, target): a profile is a policy, not a plan.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Mine {
    /// The character's name contains this, case-insensitively.
    Name(String),
    /// This skill stands at least this high, buffs counted.
    Skill { skill: u32, op: Op, level: u32 },
    /// This skill is at least trained (2) or specialised (3): `ac_world::stats::sac`.
    Trained { skill: u32, at_least: u32 },
    /// The character's level.
    Level { op: Op, level: u32 },
}

impl Mine {
    /// Whether it holds for this character.
    pub fn holds(&self, me: &Wielder, name: &str) -> bool {
        match self {
            Mine::Name(want) => {
                let want = want.trim();
                !want.is_empty() && name.to_lowercase().contains(&want.to_lowercase())
            }
            Mine::Skill { skill, op, level } => op.test(me.skill(*skill) as f64, *level as f64),
            Mine::Trained { skill, at_least } => me.advancement_of(*skill) >= *at_least,
            Mine::Level { op, level } => op.test(me.level as f64, *level as f64),
        }
    }

    /// Said in words, for the editor and the log.
    pub fn tell(&self) -> String {
        use ac_world::stats::skill_name;
        match self {
            Mine::Name(n) => format!("my name has \"{n}\""),
            Mine::Skill { skill, op, level } => {
                format!("my {} {} {level}", skill_name(*skill), op.word())
            }
            Mine::Trained { skill, at_least } => format!(
                "my {} is {}",
                skill_name(*skill),
                ac_world::stats::sac_name(*at_least)
            ),
            Mine::Level { op, level } => format!("my level {} {level}", op.word()),
        }
    }
}

/// How a piece of text is matched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextOp {
    /// Contains it, ignoring case. What most rules want.
    #[default]
    Has,
    /// Does not contain it: "armour, but not Covenant" without a lookahead.
    HasNot,
    /// Matches this regular expression.
    Like,
    /// Does not match it.
    Unlike,
}

impl TextOp {
    pub const ALL: [TextOp; 4] = [TextOp::Has, TextOp::HasNot, TextOp::Like, TextOp::Unlike];

    pub fn word(self) -> &'static str {
        match self {
            TextOp::Has => "contains",
            TextOp::HasNot => "does not contain",
            TextOp::Like => "matches",
            TextOp::Unlike => "does not match",
        }
    }

    /// Whether it is a regular expression rather than a plain word.
    pub fn is_regex(self) -> bool {
        matches!(self, TextOp::Like | TextOp::Unlike)
    }

    /// Whether `text` answers this test against `pattern`.
    /// A pattern that will not compile matches nothing: a rule with a typo takes no loot, not all.
    pub fn holds(self, text: &str, pattern: &str) -> bool {
        match self {
            TextOp::Has => contains_fold(text, pattern),
            TextOp::HasNot => !contains_fold(text, pattern),
            TextOp::Like => regex_for(pattern).is_some_and(|r| r.is_match(text)),
            TextOp::Unlike => !regex_for(pattern).is_some_and(|r| r.is_match(text)),
        }
    }
}

fn contains_fold(text: &str, needle: &str) -> bool {
    let needle = needle.trim();
    !needle.is_empty() && text.to_lowercase().contains(&needle.to_lowercase())
}

/// The compiled pattern, cached because rules are read on every item of every corpse.
fn regex_for(pattern: &str) -> Option<regex::Regex> {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<std::collections::HashMap<String, Option<regex::Regex>>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let mut cache = cache.lock().ok()?;
    cache
        .entry(pattern.to_string())
        .or_insert_with(|| {
            regex::RegexBuilder::new(pattern)
                .case_insensitive(true)
                .size_limit(1 << 20)
                .build()
                .map_err(|e| tracing::warn!("loot rule: {pattern:?} is not a pattern: {e}"))
                .ok()
        })
        .clone()
}

/// Why a pattern will not compile, for the editor to say before the rule ever runs.
pub fn pattern_error(pattern: &str) -> Option<String> {
    regex::Regex::new(pattern).err().map(|e| e.to_string())
}

/// One condition of a rule.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Ask {
    /// About the item: every term the search language understands.
    Item(Term),
    /// A search line in the inventory window's language: `(ring or bracelet) not minors>0`.
    /// Prefer the typed conditions an editor draws; this gives `or` and `not`, which a rule lacks.
    Search(String),
    /// About the character reading the profile.
    Me(Mine),
    /// Any identify property by the number the server uses: `ac_world::properties`.
    /// Numbers, flags and data ids all compare as numbers, a flag being 1 or 0.
    Prop {
        kind: PropKind,
        id: u32,
        op: Op,
        value: f64,
    },
    /// The same, for a property whose value is text.
    Text { id: u32, op: TextOp, value: String },
    /// Any spell on the item whose name answers this.
    /// `Like` `^Legendary ` asks for any legendary cantrip; no list of spell names keeps up.
    Spell { op: TextOp, value: String },
}

/// Which bag of an identify a property lives in. A mirror of
/// `ac_world::properties::Kind` that can be written to a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PropKind {
    Int,
    Int64,
    Bool,
    Float,
    DataId,
}

impl PropKind {
    pub const ALL: [PropKind; 5] = [
        PropKind::Int,
        PropKind::Int64,
        PropKind::Bool,
        PropKind::Float,
        PropKind::DataId,
    ];

    pub fn kind(self) -> ac_world::properties::Kind {
        use ac_world::properties::Kind;
        match self {
            PropKind::Int => Kind::Int,
            PropKind::Int64 => Kind::Int64,
            PropKind::Bool => Kind::Bool,
            PropKind::Float => Kind::Float,
            PropKind::DataId => Kind::DataId,
        }
    }

    /// The property's value on this identify, as a number.
    fn read(self, id: u32, of: &Appraisal) -> Option<f64> {
        match self {
            PropKind::Int => of
                .ints
                .iter()
                .find(|(k, _)| *k == id)
                .map(|(_, v)| *v as f64),
            PropKind::Int64 => of
                .int64s
                .iter()
                .find(|(k, _)| *k == id)
                .map(|(_, v)| *v as f64),
            PropKind::Bool => of
                .bools
                .iter()
                .find(|(k, _)| *k == id)
                .map(|(_, v)| u32::from(*v) as f64),
            PropKind::Float => of.floats.iter().find(|(k, _)| *k == id).map(|(_, v)| *v),
            PropKind::DataId => of
                .dids
                .iter()
                .find(|(k, _)| *k == id)
                .map(|(_, v)| *v as f64),
        }
    }

    /// What to call it, for the editor.
    pub fn name_of(self, id: u32) -> String {
        ac_world::properties::name_of(self.kind(), id)
            .map(str::to_string)
            .unwrap_or_else(|| format!("{} {id}", self.kind().label()))
    }
}

impl Ask {
    /// Whether answering this needs the item appraised first.
    /// Not for the character, nor the item's name, kind, worth or workmanship, which come with it.
    pub fn needs_id(&self) -> bool {
        let term = match self {
            Ask::Me(_) => return false,
            Ask::Prop { .. } | Ask::Text { .. } | Ask::Spell { .. } => return true,
            Ask::Search(line) => return Query::parse(line).needs_appraisal(),
            Ask::Item(t) => t,
        };
        match term {
            Term::Word(_)
            | Term::Kind(_)
            | Term::Material(_)
            | Term::Slot(_)
            | Term::Wielded
            | Term::Unappraised => false,
            Term::Spell(_) | Term::Skill(_) | Term::Tier(_) => true,
            Term::Num(key, _, _) => needs_id(*key),
        }
    }

    /// Said in words.
    pub fn tell(&self) -> String {
        match self {
            Ask::Item(t) => term_words(t),
            Ask::Search(line) => format!("it matches {line:?}"),
            Ask::Me(m) => m.tell(),
            Ask::Prop {
                kind,
                id,
                op,
                value,
            } => format!("{} {} {value}", kind.name_of(*id), op.word()),
            Ask::Text { id, op, value } => format!(
                "{} {} \"{value}\"",
                ac_world::properties::name_of(ac_world::properties::Kind::Text, *id)
                    .unwrap_or("text"),
                op.word()
            ),
            Ask::Spell { op, value } => format!("a spell {} \"{value}\"", op.word()),
        }
    }
}

/// Whether a numeric field is only known once the item has been appraised.
/// Worth, burden, workmanship and stack size come with the item; the rest need the server.
pub fn needs_id(key: NumKey) -> bool {
    !matches!(
        key,
        NumKey::Value | NumKey::Burden | NumKey::Workmanship | NumKey::Stack
    )
}

/// One rule: a name, what to do, and conditions that must all hold.
/// No "or" on purpose: two rules say it more plainly, and an editor can show a list but not a tree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rule {
    /// What the rule is for, in the player's words.
    pub name: String,
    /// An editor heading to file it under ("vendor trash"); the rules ignore it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub group: String,
    /// Turned off without being deleted.
    pub on: bool,
    /// What happens to an item this rule claims.
    pub action: LootAction,
    /// Every one of these must hold.
    pub all: Vec<Ask>,
    /// Stop once this many are carried. `None` means no limit.
    pub keep_up_to: Option<u32>,
}

impl Default for Rule {
    fn default() -> Self {
        Rule {
            name: String::new(),
            group: String::new(),
            on: true,
            action: LootAction::Keep,
            all: Vec::new(),
            keep_up_to: None,
        }
    }
}

impl Rule {
    /// Whether judging this rule needs the item appraised.
    pub fn needs_id(&self) -> bool {
        self.all.iter().any(Ask::needs_id)
    }

    /// The first skill a condition asks of the reader (`Mine::Skill`, `Mine::Trained`), if any.
    /// What such a rule takes is for whichever of the party has the most of that skill.
    pub fn skill_asked(&self) -> Option<u32> {
        self.all.iter().find_map(|a| match a {
            Ask::Me(Mine::Skill { skill, .. } | Mine::Trained { skill, .. }) => Some(*skill),
            _ => None,
        })
    }

    /// Whether every condition judgeable without an appraisal holds.
    /// A rule failing one is out whatever the server would say, which saves the round trip.
    fn cheap_half_holds(&self, item: &ItemStats, me: &Wielder, name: &str) -> bool {
        self.all
            .iter()
            .filter(|a| !a.needs_id())
            .all(|a| holds(a, item, None, me, name))
    }

    /// Whether the whole rule holds.
    fn holds(&self, item: &ItemStats, id: Option<&Appraisal>, me: &Wielder, name: &str) -> bool {
        self.all.iter().all(|a| holds(a, item, id, me, name))
    }

    /// The rule in words, for the editor's summary line.
    pub fn tell(&self) -> String {
        if self.all.is_empty() {
            return "anything".to_string();
        }
        self.all
            .iter()
            .map(Ask::tell)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn holds(ask: &Ask, item: &ItemStats, id: Option<&Appraisal>, me: &Wielder, name: &str) -> bool {
    match ask {
        // Name conditions match whole words; search lines, like the inventory's, match part of one:
        // "pea" is not in "Spear" (`the_starter_profile_is_one_a_player_would_recognise`).
        Ask::Item(Term::Word(w)) => item.has_word(w),
        Ask::Item(t) => item.matches_term(t),
        Ask::Search(line) => {
            let q = Query::parse(line);
            !q.is_empty() && item.matches(&q)
        }
        Ask::Me(m) => m.holds(me, name),
        Ask::Prop {
            kind,
            id: prop,
            op,
            value,
        } => id
            .and_then(|a| kind.read(*prop, a))
            .is_some_and(|x| op.test(x, *value)),
        Ask::Text {
            id: prop,
            op,
            value,
        } => {
            // A property the item lacks is empty text, so "does not contain" holds for it,
            // which is what a rule excluding a set name means.
            let text = id
                .and_then(|a| a.strings.iter().find(|(k, _)| k == prop))
                .map(|(_, v)| v.as_str())
                .unwrap_or("");
            op.holds(text, value)
        }
        Ask::Spell { op, value } => match op {
            // "Any spell that looks like this": true if one does.
            TextOp::Has | TextOp::Like => item.spells.iter().any(|s| op.holds(s, value)),
            // "No spell like this": negate "any", or any one other spell would pass.
            TextOp::HasNot => !item.spells.iter().any(|s| TextOp::Has.holds(s, value)),
            TextOp::Unlike => !item.spells.iter().any(|s| TextOp::Like.holds(s, value)),
        },
    }
}

/// What a profile makes of an item.
#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    /// This is what to do with it, and the rule that said so.
    Decided(LootAction, String),
    /// A rule might claim it once appraised: ask the server, then judge again.
    NeedsId(String),
    /// No rule wants it.
    None,
}

/// One thing to keep in the pack, and where to get it.
/// Not a rule, as no item is in hand: a floor on stock, where [`Rule::keep_up_to`] caps taking.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Buy {
    /// The counter's name for it, specific ("Peerless Healing Kit"): each level is its own item.
    pub what: String,
    /// How many to carry when stocked up.
    pub keep: u32,
    /// How few may be left before a trip to town is worth it; `None` is a quarter of `keep`.
    /// Zero means only when it runs out; see `a_line_is_only_urgent_once_it_is_low`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restock_at: Option<u32>,
    /// A particular counter by name, or whichever sells it when None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Turned off without being deleted.
    #[serde(default = "yes")]
    pub on: bool,
}

fn yes() -> bool {
    true
}

impl Buy {
    /// The count at or below which this line is worth a trip to town.
    pub fn low_mark(&self) -> u32 {
        self.restock_at.unwrap_or(self.keep / 4)
    }
}

/// One line of the buy list, against what is carried.
#[derive(Clone, Debug, PartialEq)]
pub struct Short<'a> {
    pub want: &'a Buy,
    /// How many are carried.
    pub have: u32,
    /// How many more to buy to be stocked up.
    pub short: u32,
    /// Low enough that it is worth going to town for on its own.
    pub urgent: bool,
}

/// The one counter everything for sale goes to.
/// One, unlike the buy list: all sells at the best rate in reach (mostly the Cragstone broker).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SellTo {
    /// The best rate in reach: a counter pays `value * buy_rate`, and rates spread nearly half.
    #[default]
    Best,
    /// This one, by name, wherever it is.
    Named(String),
}

/// How a character goes about looting, beside what it takes.
/// Kept with the rules, so one profile answers "how does this character loot".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Looting {
    /// Names always taken whatever the rules say; read before any rule, beaten only by `never`.
    pub always: Vec<String>,
    /// Never take these (by name), even when a rule matches.
    pub never: Vec<String>,
    /// Appraise a corpse's items before deciding, so rules on damage, armour and spells can judge.
    pub appraise: bool,
    /// Loot its own kills nearby before the next fight, unless something is hitting it.
    /// Off, a body outranks the next fight only once it is old enough to risk rotting.
    pub after_every_fight: bool,
    /// Salvage what the rules tagged, when this character is the team's best salvager.
    pub salvage: bool,
    /// Carry what the rules tagged to the team's best salvager, when that is someone else.
    pub hand_off: bool,
    /// Pour loose stacks of one thing together, so change from buying and looting wastes no slot.
    pub tidy_pack: bool,
    /// Loot to take on before going to sell, in multiples of carrying capacity (150 x Strength).
    /// At 1x comfortable; 2x slow, no Melee or Missile Defense; 3x cannot pick up, merge or move.
    /// Counts only what sell rules hand a counter: no sale takes off worn, wielded or kept things.
    /// The whole load stays under 2x unless this is set above 2 or kept weight alone reaches 2x.
    /// The 3x wall counts everything; arithmetic and tests: `loot_room`, ac-client growth.rs.
    pub carry_up_to: f32,
}

impl Default for Looting {
    fn default() -> Self {
        Looting {
            always: vec!["Pyreal".into()],
            never: Vec::new(),
            appraise: true,
            after_every_fight: true,
            salvage: true,
            hand_off: true,
            tidy_pack: true,
            carry_up_to: 1.5,
        }
    }
}

/// A named, shareable set of rules.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    /// What it is called, which is also its file name.
    pub name: String,
    /// What it is for, in the author's words.
    pub note: String,
    /// In order. The first rule that claims an item decides it.
    pub rules: Vec<Rule>,
    /// What to keep stocked: up to a line's count a thing is kept, past it the rules decide.
    /// Read in ac-client `judge_loot`; guards only untagged things in `sale::offer_to_vendor`.
    /// A tag written when the item was taken is the player's word; this list never overrides it.
    #[serde(default)]
    pub buy: Vec<Buy>,
    /// Where what is for sale goes.
    #[serde(default)]
    pub sell_to: SellTo,
    /// How the character loots, beside what it takes.
    #[serde(default)]
    pub looting: Looting,
}

impl Profile {
    /// What to do with `item` for this character; `held` is how many are carried, for capped rules.
    /// A rule that could claim it but awaits an appraisal stops the reading; no later rule decides.
    pub fn judge(
        &self,
        item: &ItemStats,
        id: Option<&Appraisal>,
        me: &Wielder,
        name: &str,
        held: u32,
    ) -> Verdict {
        match self.reading(item, id, me, name, held) {
            Some((_, rule, false)) => Verdict::Decided(rule.action, rule.name.clone()),
            Some((_, rule, true)) => Verdict::NeedsId(rule.name.clone()),
            None => Verdict::None,
        }
    }

    /// The deciding rule and its index in `rules`, off rules counted, read as `judge` reads them.
    /// `None` when no rule claims it or the first that might awaits an appraisal.
    pub fn decided_by(
        &self,
        item: &ItemStats,
        id: Option<&Appraisal>,
        me: &Wielder,
        name: &str,
        held: u32,
    ) -> Option<(usize, &Rule)> {
        match self.reading(item, id, me, name, held) {
            Some((at, rule, false)) => Some((at, rule)),
            Some((_, _, true)) | None => None,
        }
    }

    /// The first rule that claims `item`, its index in `rules`, and whether it awaits an appraisal.
    fn reading(
        &self,
        item: &ItemStats,
        id: Option<&Appraisal>,
        me: &Wielder,
        name: &str,
        held: u32,
    ) -> Option<(usize, &Rule, bool)> {
        for (at, rule) in self.rules.iter().enumerate().filter(|(_, r)| r.on) {
            if rule.keep_up_to.is_some_and(|cap| held >= cap) {
                continue;
            }
            if !rule.cheap_half_holds(item, me, name) {
                continue;
            }
            if rule.needs_id() && !item.appraised {
                return Some((at, rule, true));
            }
            if rule.holds(item, id, me, name) {
                return Some((at, rule, false));
            }
        }
        None
    }

    /// Tells whether written answers were reached under these rules; note and name excluded.
    /// A content hash, not a counter, so an edit made while the client was closed still counts.
    /// JSON-hashed as rules hold floats (no `Hash`); computed when rules change, never per frame.
    pub fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        serde_json::to_string(&self.rules)
            .unwrap_or_default()
            .hash(&mut h);
        // The buy list decides the undecided: stock is never offered to a counter unless a rule
        // said to (`ac_loot::sale::offer_to_vendor`).
        serde_json::to_string(&self.buy)
            .unwrap_or_default()
            .hash(&mut h);
        // Always and never are read before any rule, so they decide items too.
        (&self.looting.always, &self.looting.never).hash(&mut h);
        h.finish()
    }

    /// Whether any rule could ever ask for an appraisal. A profile that
    /// cannot decides every corpse without a single round trip.
    pub fn needs_id(&self) -> bool {
        self.rules.iter().filter(|r| r.on).any(Rule::needs_id)
    }

    /// Skill ids the switched-on rules ask of the reader, sorted and each once.
    /// Rules turn only on these, name and level: what a member tells others to judge bodies for it.
    pub fn skills_asked(&self) -> Vec<u32> {
        let mut asked: Vec<u32> = self
            .rules
            .iter()
            .filter(|r| r.on)
            .flat_map(|r| &r.all)
            .filter_map(|a| match a {
                Ask::Me(Mine::Skill { skill, .. } | Mine::Trained { skill, .. }) => Some(*skill),
                _ => None,
            })
            .collect();
        asked.sort_unstable();
        asked.dedup();
        asked
    }

    /// Whether a rule that is on takes things for one member rather than whoever opens the body.
    /// It asks a skill or, while [`Looting::salvage`], salvages: every character has Salvaging.
    pub fn sends_to_the_best(&self) -> bool {
        self.rules
            .iter()
            .filter(|r| r.on && r.action.takes())
            .any(|r| {
                r.skill_asked().is_some()
                    || (self.looting.salvage && r.action == LootAction::Salvage)
            })
    }

    /// Where a profile of this name lives.
    pub fn path_of(dir: &Path, name: &str) -> PathBuf {
        dir.join(format!("{}.json", tidy_name(name)))
    }

    /// The profiles in `dir`, by name, in alphabetical order.
    pub fn list(dir: &Path) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| {
                e.path()
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(str::to_string)
            })
            .collect();
        names.sort();
        names
    }

    /// Read one; an unparsable profile is an error, not an empty one that silently loots nothing.
    pub fn load(dir: &Path, name: &str) -> std::io::Result<Profile> {
        let path = Self::path_of(dir, name);
        let text = std::fs::read_to_string(&path)?;
        let mut p: Profile = serde_json::from_str(&text)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        if p.name.trim().is_empty() {
            p.name = name.to_string();
        }
        Ok(p)
    }

    /// Write it out, creating the directory if it is not there.
    pub fn save(&self, dir: &Path) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        let path = Self::path_of(dir, &self.name);
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(&path, text)?;
        Ok(path)
    }
}

impl Profile {
    /// The counter this profile names for selling, if it names one.
    /// A blank name, as an editor leaves after un-picking "this counter", means `Best` too.
    pub fn sell_to_named(&self) -> Option<&str> {
        match &self.sell_to {
            SellTo::Best => None,
            SellTo::Named(n) => Some(n.trim()).filter(|n| !n.is_empty()),
        }
    }

    /// Whether the character keeps this stocked, so no rule read at the counter may sell it.
    /// "Sell anything worth under a thousand" has not said "sell my Peas".
    pub fn stocks(&self, name: &str) -> bool {
        self.buy
            .iter()
            .filter(|b| b.on && !b.what.trim().is_empty())
            .any(|b| contains_fold(name, b.what.trim()))
    }

    /// How many of a thing the buy list says to carry, 0 when it does not mention it.
    pub fn stocked_count(&self, what: &str) -> u32 {
        self.buy
            .iter()
            .filter(|b| b.on)
            .find(|b| contains_fold(what, b.what.trim()))
            .map_or(0, |b| b.keep)
    }

    /// What is short, against what is carried: the shopping list.
    /// `held` counts a thing in the pack by the name the counter lists it under.
    pub fn shortfall(&self, held: impl Fn(&str) -> u32) -> Vec<Short<'_>> {
        self.buy
            .iter()
            .filter(|b| b.on && b.keep > 0 && !b.what.trim().is_empty())
            .filter_map(|b| {
                let have = held(&b.what);
                (have < b.keep).then(|| Short {
                    want: b,
                    have,
                    short: b.keep - have,
                    urgent: have <= b.low_mark(),
                })
            })
            .collect()
    }
}

impl Profile {
    /// A profile to start from, meant to be edited rather than obeyed.
    /// Rules needing no appraisal come first, so most of a corpse settles before any identify.
    pub fn starter() -> Profile {
        let word = |w: &str| Ask::Item(Term::Word(w.into()));
        let kind = |k: &str| Ask::Item(Term::Kind(k.into()));
        let num = |k: NumKey, op: Op, v: f64| Ask::Item(Term::Num(k, op, v));
        Profile {
            name: "Starter".into(),
            note: "A place to start: money and components kept, vendor \
                   trash sold, the good things looked over."
                .into(),
            rules: vec![
                Rule {
                    name: "money".into(),
                    action: LootAction::Keep,
                    all: vec![kind("money")],
                    ..Default::default()
                },
                Rule {
                    name: "trade notes".into(),
                    action: LootAction::Keep,
                    all: vec![kind("note")],
                    ..Default::default()
                },
                Rule {
                    name: "healing kits, a few".into(),
                    action: LootAction::Keep,
                    all: vec![word("healing kit")],
                    keep_up_to: Some(4),
                    ..Default::default()
                },
                // Peas pay for the trip, and a pea is a spell component by item type,
                // so this comes before the rule that keeps components.
                Rule {
                    name: "peas to sell".into(),
                    action: LootAction::Sell,
                    all: vec![word("pea")],
                    ..Default::default()
                },
                Rule {
                    name: "spell components".into(),
                    action: LootAction::Keep,
                    all: vec![kind("comps")],
                    ..Default::default()
                },
                Rule {
                    name: "gems to sell".into(),
                    action: LootAction::Sell,
                    all: vec![kind("gem"), num(NumKey::Value, Op::Ge, 100.0)],
                    ..Default::default()
                },
                // Material and workmanship both come with the item, so no appraisal.
                Rule {
                    name: "good salvage".into(),
                    action: LootAction::Salvage,
                    all: vec![kind("salvage"), num(NumKey::Workmanship, Op::Ge, 8.0)],
                    ..Default::default()
                },
                // Early, so the rules below never ask the server about junk.
                Rule {
                    name: "nothing cheap".into(),
                    action: LootAction::Skip,
                    all: vec![num(NumKey::Value, Op::Le, 250.0)],
                    ..Default::default()
                },
                // From here down the server is asked, each about a thing worth the round trip.
                Rule {
                    name: "anything legendary".into(),
                    action: LootAction::Keep,
                    all: vec![Ask::Spell {
                        op: TextOp::Like,
                        value: "^Legendary ".into(),
                    }],
                    ..Default::default()
                },
                Rule {
                    name: "anything epic".into(),
                    action: LootAction::Keep,
                    all: vec![Ask::Spell {
                        op: TextOp::Like,
                        value: "^Epic ".into(),
                    }],
                    ..Default::default()
                },
                Rule {
                    name: "armour worth wearing".into(),
                    action: LootAction::Keep,
                    all: vec![kind("armor"), num(NumKey::Armor, Op::Ge, 250.0)],
                    ..Default::default()
                },
                Rule {
                    name: "the rest, to the counter".into(),
                    action: LootAction::Sell,
                    all: vec![num(NumKey::Value, Op::Ge, 1_000.0)],
                    ..Default::default()
                },
            ],
            // A caster's components scale from the taper, which every cast burns; kits go by name.
            // Everything listed is used, so none reaches a counter unless its loot tag says sell.
            buy: vec![
                Buy {
                    what: "Prismatic Taper".into(),
                    keep: 1000,
                    // A quarter: a caster with 250 tapers is not in trouble yet, with 200 it is.
                    restock_at: Some(250),
                    from: None,
                    on: true,
                },
                Buy {
                    what: "Healing Kit".into(),
                    keep: 2,
                    // Down to the last one is reason enough.
                    restock_at: Some(1),
                    from: None,
                    on: true,
                },
            ],
            sell_to: SellTo::Best,
            looting: Looting::default(),
        }
    }
}

/// A name safe as a file name: what a player types, less the characters a path cannot hold.
pub fn tidy_name(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        "profile".to_string()
    } else {
        cleaned
    }
}

/// A term in words, for the editor.
fn term_words(t: &Term) -> String {
    match t {
        Term::Word(w) => format!("name has the word \"{w}\""),
        Term::Spell(s) => format!("a spell like \"{s}\""),
        Term::Kind(k) => format!("is {k}"),
        Term::Material(m) => format!("made of {m}"),
        Term::Skill(s) => format!("uses {s}"),
        Term::Slot(_) => "worn in that slot".to_string(),
        Term::Tier(t) => format!("has a {} cantrip", t.word()),
        Term::Wielded => "is wielded".to_string(),
        Term::Unappraised => "has not been looked over".to_string(),
        Term::Num(k, op, v) => format!("{} {} {v}", num_word(*k), op.word()),
    }
}

/// A numeric field's name, as the editor lists it.
pub fn num_word(k: NumKey) -> &'static str {
    match k {
        NumKey::Damage => "damage",
        NumKey::Armor => "armour",
        NumKey::Value => "value",
        NumKey::Burden => "burden",
        NumKey::Workmanship => "workmanship",
        NumKey::Speed => "speed",
        NumKey::Wield => "wield level",
        NumKey::Mana => "mana",
        NumKey::Spellcraft => "spellcraft",
        NumKey::Uses => "uses",
        NumKey::Tinks => "tinkers",
        NumKey::Stack => "stack",
        NumKey::Attack => "attack",
        NumKey::Defense => "defense",
        NumKey::Spells => "spells",
        NumKey::Cantrips => "cantrips",
        NumKey::Minors => "minors",
        NumKey::Moderates => "moderates",
        NumKey::Majors => "majors",
        NumKey::Epics => "epics",
        NumKey::Legendaries => "legendaries",
    }
}

#[cfg(test)]
impl Profile {
    /// `judge` for a test that has no raw identify to hand.
    fn judge_test(&self, item: &ItemStats, me: &Wielder, name: &str, held: u32) -> Verdict {
        self.judge(item, None, me, name, held)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::Tier;
    use ac_world::item_type;
    use ac_world::stats::{sac, skill};

    fn item(name: &str, kind: u32, value: u32) -> ItemStats {
        ItemStats {
            name: name.into(),
            item_type: kind,
            kind: crate::items::kind_name(kind),
            value,
            ..Default::default()
        }
    }

    fn me(level: u32, skills: &[(u32, u32, u32)]) -> Wielder {
        Wielder {
            level,
            skills: skills.iter().map(|(s, v, a)| (*s, *v, *v, *a)).collect(),
            ..Default::default()
        }
    }

    fn rule(name: &str, action: LootAction, all: Vec<Ask>) -> Rule {
        Rule {
            name: name.into(),
            action,
            all,
            ..Default::default()
        }
    }

    #[test]
    fn a_fingerprint_moves_with_the_rules_and_not_with_the_prose() {
        let mut p = Profile {
            name: "test".into(),
            note: "a note".into(),
            rules: vec![rule(
                "peas",
                LootAction::Sell,
                vec![Ask::Item(Term::Word("pea".into()))],
            )],
            ..Default::default()
        };
        let was = p.fingerprint();
        p.note = "a much longer note about what this profile is for".into();
        p.name = "renamed".into();
        assert_eq!(p.fingerprint(), was);
        p.rules[0].on = false;
        assert_ne!(p.fingerprint(), was);
        p.rules[0].on = true;
        assert_eq!(p.fingerprint(), was);
        p.rules[0].action = LootAction::Keep;
        assert_ne!(p.fingerprint(), was);
        p.rules[0].action = LootAction::Sell;
        p.rules[0].keep_up_to = Some(3);
        assert_ne!(p.fingerprint(), was);
        p.rules[0].keep_up_to = None;
        assert_eq!(p.fingerprint(), was);
        // The buy list is a rule too: stock is never offered to a counter.
        p.buy.push(Buy {
            what: "Prismatic Taper".into(),
            keep: 1000,
            ..Default::default()
        });
        assert_ne!(p.fingerprint(), was);
    }

    #[test]
    fn the_first_rule_that_claims_an_item_decides_it() {
        let profile = Profile {
            name: "test".into(),
            note: String::new(),
            rules: vec![
                rule(
                    "keep the good stuff",
                    LootAction::Keep,
                    vec![Ask::Item(Term::Num(NumKey::Value, Op::Ge, 5_000.0))],
                ),
                rule(
                    "sell the rest",
                    LootAction::Sell,
                    vec![Ask::Item(Term::Kind("gem".into()))],
                ),
            ],
            ..Default::default()
        };
        let me = me(50, &[]);
        let dear = item("Ruby", item_type::GEM, 9_000);
        let cheap = item("Quartz", item_type::GEM, 40);
        let boots = item("Leather Boots", item_type::ARMOR, 40);
        assert_eq!(
            profile.judge_test(&dear, &me, "Aldric", 0),
            Verdict::Decided(LootAction::Keep, "keep the good stuff".into())
        );
        assert_eq!(
            profile.judge_test(&cheap, &me, "Aldric", 0),
            Verdict::Decided(LootAction::Sell, "sell the rest".into())
        );
        // Nothing claims it, so it stays on the corpse.
        assert_eq!(profile.judge_test(&boots, &me, "Aldric", 0), Verdict::None);
    }

    #[test]
    fn one_profile_reads_differently_for_each_of_a_group() {
        // The party shares a profile; only the one who can mend a broken key carries one home.
        let profile = Profile {
            name: "group".into(),
            note: String::new(),
            rules: vec![rule(
                "broken keys, if I can mend them",
                LootAction::Keep,
                vec![
                    Ask::Item(Term::Word("broken".into())),
                    Ask::Me(Mine::Skill {
                        skill: skill::LOCKPICK,
                        op: Op::Ge,
                        level: 250,
                    }),
                ],
            )],
            ..Default::default()
        };
        let key = item("Broken Marble Key", item_type::KEY, 0);
        let picker = me(100, &[(skill::LOCKPICK, 300, sac::TRAINED)]);
        let mage = me(100, &[(skill::LOCKPICK, 0, sac::UNTRAINED)]);
        assert_eq!(
            profile.judge_test(&key, &picker, "Bryn", 0),
            Verdict::Decided(LootAction::Keep, "broken keys, if I can mend them".into())
        );
        assert_eq!(profile.judge_test(&key, &mage, "Aldric", 0), Verdict::None);
    }

    #[test]
    fn only_the_skills_a_rule_asks_of_me_are_named() {
        // A character tells the others only the skills its rules turn on, to judge a body for it.
        let lockpick = |level| {
            Ask::Me(Mine::Skill {
                skill: skill::LOCKPICK,
                op: Op::Ge,
                level,
            })
        };
        let mut switched_off = rule(
            "kits, if I can heal",
            LootAction::Keep,
            vec![Ask::Me(Mine::Skill {
                skill: skill::HEALING,
                op: Op::Ge,
                level: 100,
            })],
        );
        switched_off.on = false;
        let profile = Profile {
            name: "group".into(),
            rules: vec![
                rule("keys", LootAction::Keep, vec![lockpick(250)]),
                rule(
                    "salvage, if I salvage",
                    LootAction::Salvage,
                    vec![Ask::Me(Mine::Trained {
                        skill: skill::SALVAGING,
                        at_least: sac::TRAINED,
                    })],
                ),
                // Asked twice, named once.
                rule("more keys", LootAction::Keep, vec![lockpick(300)]),
                // Who it is and how far on are not skills.
                rule(
                    "mine",
                    LootAction::Keep,
                    vec![
                        Ask::Me(Mine::Name("Bryn".into())),
                        Ask::Me(Mine::Level {
                            op: Op::Ge,
                            level: 10,
                        }),
                    ],
                ),
                switched_off,
            ],
            ..Default::default()
        };
        assert_eq!(
            profile.skills_asked(),
            vec![skill::LOCKPICK, skill::SALVAGING]
        );
        // The starter asks nothing of the character, so a party reading it shares nothing more.
        assert!(Profile::starter().skills_asked().is_empty());
    }

    #[test]
    fn a_rule_that_asks_about_a_skill_or_salvages_sends_what_it_takes_to_the_best() {
        let lockpick = Ask::Me(Mine::Skill {
            skill: skill::LOCKPICK,
            op: Op::Ge,
            level: 250,
        });
        let keys = rule(
            "keys",
            LootAction::Keep,
            vec![Ask::Search("broken".into()), lockpick.clone()],
        );
        assert_eq!(keys.skill_asked(), Some(skill::LOCKPICK));
        // The first skill named is the one it asks about.
        let both = rule(
            "both",
            LootAction::Keep,
            vec![
                Ask::Me(Mine::Level {
                    op: Op::Ge,
                    level: 10,
                }),
                Ask::Me(Mine::Trained {
                    skill: skill::SALVAGING,
                    at_least: sac::TRAINED,
                }),
                lockpick.clone(),
            ],
        );
        assert_eq!(both.skill_asked(), Some(skill::SALVAGING));
        assert_eq!(
            rule("peas", LootAction::Keep, vec![Ask::Search("pea".into())]).skill_asked(),
            None
        );

        let salvage = rule(
            "salvage",
            LootAction::Salvage,
            vec![Ask::Search("platemail".into())],
        );
        let sell = rule("sell", LootAction::Sell, vec![Ask::Search("gem".into())]);
        let group = |rules: Vec<Rule>, salvaging: bool| {
            let mut p = Profile {
                name: "group".into(),
                rules,
                ..Default::default()
            };
            p.looting.salvage = salvaging;
            p
        };
        // No skill asked and nobody salvaging: nothing is meant for anyone in particular.
        assert!(!group(vec![sell.clone(), salvage.clone()], false).sends_to_the_best());
        // A salvage rule, while the salvager salvages.
        assert!(group(vec![sell.clone(), salvage.clone()], true).sends_to_the_best());
        assert!(group(vec![sell.clone(), keys.clone()], false).sends_to_the_best());
        // But not by a rule switched off, or one that leaves what it claims.
        let mut off = keys.clone();
        off.on = false;
        let mut skip = keys;
        skip.action = LootAction::Skip;
        assert!(!group(vec![sell, off, skip], false).sends_to_the_best());
        // The starter salvages, and its salvager salvages.
        assert!(Profile::starter().sends_to_the_best());
    }

    #[test]
    fn the_rule_that_decides_an_item_is_the_one_the_verdict_names() {
        let mut capped = rule("kits", LootAction::Keep, vec![Ask::Search("kit".into())]);
        capped.keep_up_to = Some(4);
        let mut off = rule("off", LootAction::Keep, vec![Ask::Search("kit".into())]);
        off.on = false;
        let profile = Profile {
            name: "order".into(),
            rules: vec![
                off,
                capped,
                rule(
                    "sell kits",
                    LootAction::Sell,
                    vec![Ask::Search("kit".into())],
                ),
                rule(
                    "armour",
                    LootAction::Keep,
                    vec![Ask::Prop {
                        kind: PropKind::Int,
                        id: 28,
                        op: Op::Ge,
                        value: 200.0,
                    }],
                ),
            ],
            ..Default::default()
        };
        let me = Wielder::default();
        let kit = ItemStats {
            name: "Healing Kit".into(),
            appraised: true,
            ..Default::default()
        };
        // Where it stands counts the rules switched off.
        let (at, by) = profile.decided_by(&kit, None, &me, "Bryn", 0).unwrap();
        assert_eq!((at, by.name.as_str()), (1, "kits"));
        assert_eq!(
            profile.judge(&kit, None, &me, "Bryn", 0),
            Verdict::Decided(LootAction::Keep, "kits".into())
        );
        // Past the cap the next rule decides.
        let (at, by) = profile.decided_by(&kit, None, &me, "Bryn", 4).unwrap();
        assert_eq!((at, by.name.as_str()), (2, "sell kits"));
        // One that has to be appraised first decides nothing yet.
        let plate = ItemStats {
            name: "Platemail".into(),
            appraised: false,
            ..Default::default()
        };
        assert_eq!(
            profile.judge(&plate, None, &me, "Bryn", 0),
            Verdict::NeedsId("armour".into())
        );
        assert_eq!(profile.decided_by(&plate, None, &me, "Bryn", 0), None);
    }

    #[test]
    fn an_appraisal_is_asked_for_only_when_it_would_change_the_answer() {
        let profile = Profile {
            name: "test".into(),
            note: String::new(),
            rules: vec![rule(
                "armour with a major on it",
                LootAction::Keep,
                vec![
                    Ask::Item(Term::Kind("armor".into())),
                    Ask::Item(Term::Tier(Tier::Major)),
                ],
            )],
            ..Default::default()
        };
        let me = me(50, &[]);
        // Not armour: the cheap half already fails, so the server is never asked.
        let gem = item("Ruby", item_type::GEM, 9_000);
        assert_eq!(profile.judge_test(&gem, &me, "Aldric", 0), Verdict::None);
        // Armour: the rest of the rule waits for the server.
        let mut plate = item("Platemail Hauberk", item_type::ARMOR, 4_000);
        assert_eq!(
            profile.judge_test(&plate, &me, "Aldric", 0),
            Verdict::NeedsId("armour with a major on it".into())
        );
        // Once appraised it is judged for real.
        plate.appraised = true;
        assert_eq!(profile.judge_test(&plate, &me, "Aldric", 0), Verdict::None);
        plate.spells = vec!["Major Armor".into()];
        assert_eq!(
            profile.judge_test(&plate, &me, "Aldric", 0),
            Verdict::Decided(LootAction::Keep, "armour with a major on it".into())
        );
    }

    #[test]
    fn a_capped_rule_stops_once_enough_are_carried() {
        let profile = Profile {
            name: "test".into(),
            note: String::new(),
            rules: vec![Rule {
                name: "two healing kits".into(),
                keep_up_to: Some(2),
                all: vec![Ask::Item(Term::Word("healing kit".into()))],
                ..Default::default()
            }],
            ..Default::default()
        };
        let me = me(50, &[]);
        let kit = item("Healing Kit", item_type::MISC, 100);
        assert!(matches!(
            profile.judge_test(&kit, &me, "Aldric", 1),
            Verdict::Decided(LootAction::Keep, _)
        ));
        assert_eq!(profile.judge_test(&kit, &me, "Aldric", 2), Verdict::None);
    }

    #[test]
    fn any_property_the_server_sends_can_be_asked_about() {
        // Int 265 is the armour set: the server's EquipmentSetId, other looters' ArmorSetId.
        // The editor shows the server's name, because that is what the number means.
        const ARMOR_SET_ID: u32 = 265;
        let profile = Profile {
            name: "sets".into(),
            note: String::new(),
            rules: vec![rule(
                "any armour of a set",
                LootAction::Keep,
                vec![Ask::Prop {
                    kind: PropKind::Int,
                    id: ARMOR_SET_ID,
                    op: Op::Ge,
                    value: 1.0,
                }],
            )],
            ..Default::default()
        };
        let me = me(50, &[]);
        let mut plate = item("Platemail Hauberk", item_type::ARMOR, 4_000);
        plate.appraised = true;
        // No identify to read: nothing to say yet.
        assert_eq!(profile.judge(&plate, None, &me, "Aldric", 0), Verdict::None);
        let plain = Appraisal {
            ints: vec![(19, 4_000)],
            ..Default::default()
        };
        assert_eq!(
            profile.judge(&plate, Some(&plain), &me, "Aldric", 0),
            Verdict::None
        );
        let set = Appraisal {
            ints: vec![(ARMOR_SET_ID, 12)],
            ..Default::default()
        };
        assert_eq!(
            profile.judge(&plate, Some(&set), &me, "Aldric", 0),
            Verdict::Decided(LootAction::Keep, "any armour of a set".into())
        );
        // The property is named for the editor rather than numbered.
        assert_eq!(PropKind::Int.name_of(ARMOR_SET_ID), "EquipmentSetId");
        assert_eq!(PropKind::Int.name_of(999_999), "number 999999");
    }

    #[test]
    fn a_pattern_catches_what_a_list_of_names_cannot() {
        // Any legendary cantrip, whatever it is of: no list of spell names keeps up.
        let profile = Profile {
            name: "legendaries".into(),
            note: String::new(),
            rules: vec![rule(
                "anything with a legendary on it",
                LootAction::Keep,
                vec![Ask::Spell {
                    op: TextOp::Like,
                    value: "^Legendary ".into(),
                }],
            )],
            ..Default::default()
        };
        let me = me(50, &[]);
        let mut ring = item("Gold Ring", item_type::JEWELRY, 3_000);
        ring.appraised = true;
        ring.spells = vec!["Major Strength".into(), "Legendary Focus".into()];
        assert!(matches!(
            profile.judge_test(&ring, &me, "Aldric", 0),
            Verdict::Decided(LootAction::Keep, _)
        ));
        ring.spells = vec!["Major Strength".into(), "Blood Drinker".into()];
        assert_eq!(profile.judge_test(&ring, &me, "Aldric", 0), Verdict::None);
        // Anchoring is the point of a pattern: "Legendary" mid-name is not a legendary cantrip.
        ring.spells = vec!["Surge of Legendary Regret".into()];
        assert_eq!(profile.judge_test(&ring, &me, "Aldric", 0), Verdict::None);

        // "Not Covenant" said plainly, as Rust's regex has no lookahead.
        assert!(TextOp::HasNot.holds("Platemail Hauberk", "Covenant"));
        assert!(!TextOp::HasNot.holds("Covenant Hauberk", "covenant"));
        // Case is ignored, both ways.
        assert!(TextOp::Has.holds("Platemail Hauberk", "HAUBERK"));
        assert!(TextOp::Like.holds("Legendary Focus", "^legendary"));
        // An uncompilable pattern takes nothing: a typo must not empty the corpse into the pack.
        assert!(!TextOp::Like.holds("anything", "("));
        assert!(pattern_error("(").is_some());
        assert!(pattern_error("^Legendary ").is_none());
    }

    #[test]
    fn what_the_character_buys_it_never_sells() {
        // A thing bought in town is not sold there: the buy list beats "sell anything cheap".
        let p = Profile::starter();
        assert!(p.stocks("Prismatic Taper"));
        assert!(p.stocks("Healing Kit"), "by the name the counter uses");
        assert!(p.stocks("Lesser Healing Kit"), "and its levels");
        assert!(!p.stocks("Pyreal Pea"), "a component it does not stock");
        assert!(!p.stocks("Ornate Ring"));
    }

    #[test]
    fn the_shopping_list_is_what_is_short() {
        let mut p = Profile::starter();
        p.buy = vec![
            Buy {
                what: "Prismatic Taper".into(),
                keep: 1000,
                restock_at: None,
                from: None,
                on: true,
            },
            Buy {
                what: "Peerless Healing Kit".into(),
                keep: 5,
                restock_at: None,
                from: Some("Fletcher".into()),
                on: true,
            },
            Buy {
                what: "Acid Arrowhead".into(),
                keep: 500,
                restock_at: None,
                from: None,
                on: false,
            },
        ];
        let held = |what: &str| match what {
            "Prismatic Taper" => 400,
            "Peerless Healing Kit" => 5,
            _ => 0,
        };
        let short = p.shortfall(held);
        assert_eq!(short.len(), 1, "the kits are stocked, the heads are off");
        assert_eq!(short[0].want.what, "Prismatic Taper");
        assert_eq!(short[0].short, 600, "six hundred short of a thousand");
        assert_eq!(short[0].have, 400);
    }

    #[test]
    fn the_suit_being_built_is_not_sold_for_pocket_change() {
        // A cantrip marks a piece of a suit being built, so the starter's cantrip rules sit above
        // the rule that sells to the counter: selling it at face value is the costliest mistake.
        let p = Profile::starter();
        let me = me(50, &[]);

        let mut hauberk = item("Hauberk", item_type::ARMOR, 900);
        hauberk.spells = vec!["Epic Life Magic Aptitude".into()];
        // Spells are unknown until the server is asked, so the profile asks rather than guesses.
        assert!(
            matches!(
                p.judge_test(&hauberk, &me, "Aldric", 0),
                Verdict::NeedsId(_)
            ),
            "unappraised, the answer is 'ask'"
        );
        hauberk.appraised = true;
        assert!(
            matches!(
                p.judge_test(&hauberk, &me, "Aldric", 0),
                Verdict::Decided(LootAction::Keep, _)
            ),
            "and once asked, a spelled piece is kept"
        );

        let mut ring = item("Ring", item_type::JEWELRY, 400);
        ring.spells = vec!["Legendary Endurance".into()];
        ring.appraised = true;
        assert!(matches!(
            p.judge_test(&ring, &me, "Aldric", 0),
            Verdict::Decided(LootAction::Keep, _)
        ));

        // And the plain cap still goes.
        let mut cap = item("Leather Cap", item_type::ARMOR, 1_200);
        cap.appraised = true;
        assert!(
            matches!(
                p.judge_test(&cap, &me, "Aldric", 0),
                Verdict::Decided(LootAction::Sell, _)
            ),
            "unspelled armour of no great worth is what pays for the trip"
        );
    }

    #[test]
    fn a_line_is_only_urgent_once_it_is_low() {
        // Without a low mark one short is urgent, and a caster holding 900 tapers walks to town.
        let mut p = Profile::starter();
        p.buy = vec![Buy {
            what: "Prismatic Taper".into(),
            keep: 1000,
            restock_at: Some(250),
            from: None,
            on: true,
        }];
        let at = |have: u32| {
            p.shortfall(|_| have)
                .first()
                .map(|s| s.urgent)
                .expect("short of a thousand")
        };
        assert!(!at(999), "one short is not a reason to go");
        assert!(!at(251));
        assert!(at(250), "at the mark");
        assert!(at(3), "and below it");
    }

    #[test]
    fn a_line_with_no_mark_falls_back_to_a_quarter() {
        let quarter = Buy {
            what: "Arrow".into(),
            keep: 250,
            restock_at: None,
            from: None,
            on: true,
        };
        assert_eq!(quarter.low_mark(), 62);
        // Zero is a real answer, not a missing one: only when it is out.
        let last_resort = Buy {
            restock_at: Some(0),
            ..quarter.clone()
        };
        assert_eq!(last_resort.low_mark(), 0);
    }

    #[test]
    fn a_want_can_name_its_own_counter() {
        // Components from an archmage, arrowheads from a bowyer: one character, several shelves.
        let mut p = Profile::starter();
        p.buy = vec![
            Buy {
                what: "Blue Pea".into(),
                keep: 500,
                restock_at: None,
                from: None,
                on: true,
            },
            Buy {
                what: "Acid Arrowhead".into(),
                keep: 500,
                restock_at: None,
                from: Some("Thimrin Woodsetter".into()),
                on: true,
            },
        ];
        let short = p.shortfall(|_| 0);
        assert_eq!(short.len(), 2);
        assert_eq!(short[0].want.from, None, "whoever sells it");
        assert_eq!(short[1].want.from.as_deref(), Some("Thimrin Woodsetter"));
    }

    #[test]
    fn selling_goes_to_one_counter_and_by_default_the_best_paying() {
        let mut p = Profile::starter();
        assert_eq!(p.sell_to, SellTo::Best);
        assert_eq!(p.sell_to_named(), None, "work it out");

        p.sell_to = SellTo::Named("  Arcanum Broker  ".into());
        assert_eq!(p.sell_to_named(), Some("Arcanum Broker"), "trimmed");

        // A blank name, as an editor leaves after un-picking "this counter", means none was named.
        p.sell_to = SellTo::Named("   ".into());
        assert_eq!(p.sell_to_named(), None);
    }

    #[test]
    fn an_older_profile_file_still_reads() {
        // A file with no `buy` or `sell_to` must load with an empty list and the default counter.
        let text = r#"{"name":"Old","note":"","rules":[]}"#;
        let p: Profile = serde_json::from_str(text).unwrap();
        assert_eq!(p.name, "Old");
        assert!(p.buy.is_empty());
        assert_eq!(p.sell_to, SellTo::Best);
    }

    #[test]
    fn the_starter_profile_is_one_a_player_would_recognise() {
        let p = Profile::starter();
        let me = me(50, &[]);
        let keeps = |name: &str, kind: u32, value: u32| {
            matches!(
                p.judge_test(&item(name, kind, value), &me, "Aldric", 0),
                Verdict::Decided(LootAction::Keep, _)
            )
        };
        let sells = |name: &str, kind: u32, value: u32| {
            matches!(
                p.judge_test(&item(name, kind, value), &me, "Aldric", 0),
                Verdict::Decided(LootAction::Sell, _)
            )
        };
        assert!(keeps("Pyreal", item_type::MONEY, 1_000));
        assert!(keeps(
            "Trade Note (50,000)",
            item_type::PROMISSORY_NOTE,
            50_000
        ));
        assert!(keeps("Prismatic Taper", item_type::SPELL_COMPONENTS, 5));
        assert!(sells("Copper Pea", item_type::MISC, 40));
        // A pea is a spell component by item type yet sold: its rule comes before the one that
        // keeps components, because peas pay for the trip.
        assert!(sells("Pyreal Pea", item_type::SPELL_COMPONENTS, 50_000));
        // "pea" is a word of its own: a Spear is not one.
        assert!(!sells("Spear", item_type::MELEE_WEAPON, 40));
        assert!(sells("Ruby", item_type::GEM, 9_000));
        // Junk is skipped by a rule placed before every rule that needs the server.
        let junk = item("Quartz", item_type::GEM, 20);
        assert!(matches!(
            p.judge_test(&junk, &me, "Aldric", 0),
            Verdict::Decided(LootAction::Skip, _)
        ));
        let kit = item("Healing Kit", item_type::MISC, 100);
        assert!(matches!(
            p.judge_test(&kit, &me, "Aldric", 0),
            Verdict::Decided(LootAction::Keep, _)
        ));
        assert!(!matches!(
            p.judge_test(&kit, &me, "Aldric", 4),
            Verdict::Decided(LootAction::Keep, _)
        ));
        // Something dear enough to be worth a look is looked at rather than guessed about.
        let plate = item("Platemail Hauberk", item_type::ARMOR, 4_000);
        assert!(matches!(
            p.judge_test(&plate, &me, "Aldric", 0),
            Verdict::NeedsId(_)
        ));
        // Needing the server sometimes is the point of the ordering, not a fault in it.
        assert!(p.needs_id());
    }

    #[test]
    fn the_cheap_rules_settle_most_of_a_corpse_without_asking_the_server() {
        // The shape players write: rules on cheap fields first, the one needing an appraisal last.
        let profile = Profile {
            name: "tuned".into(),
            note: String::new(),
            rules: vec![
                rule(
                    "peas",
                    LootAction::Sell,
                    vec![Ask::Item(Term::Word("pea".into()))],
                ),
                rule(
                    "nothing cheap",
                    LootAction::Skip,
                    vec![Ask::Item(Term::Num(NumKey::Value, Op::Le, 100.0))],
                ),
                rule(
                    "armour with a legendary on it",
                    LootAction::Keep,
                    vec![
                        Ask::Item(Term::Kind("armor".into())),
                        Ask::Spell {
                            op: TextOp::Like,
                            value: "^Legendary ".into(),
                        },
                    ],
                ),
            ],
            ..Default::default()
        };
        let me = me(50, &[]);
        let corpse = [
            item("Copper Pea", item_type::MISC, 40),
            item("Quartz", item_type::GEM, 20),
            item("Platemail Hauberk", item_type::ARMOR, 4_000),
        ];
        let verdicts: Vec<Verdict> = corpse
            .iter()
            .map(|it| profile.judge_test(it, &me, "Aldric", 0))
            .collect();
        // The pea and the quartz are decided without the server being asked anything.
        assert!(matches!(verdicts[0], Verdict::Decided(LootAction::Sell, _)));
        assert!(matches!(verdicts[1], Verdict::Decided(LootAction::Skip, _)));
        assert!(matches!(verdicts[2], Verdict::NeedsId(_)));
        let to_look_over = verdicts
            .iter()
            .filter(|v| matches!(v, Verdict::NeedsId(_)))
            .count();
        assert_eq!(to_look_over, 1, "one round trip, not three");
    }

    #[test]
    fn an_edit_is_live_at_once_and_written_a_moment_later() {
        let dir = std::env::temp_dir().join(format!("acswarm-writes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let library = Library::default();
        library.open(&dir);

        // A new profile is written at once, so the file exists to be found and shared.
        let mut profile = Profile {
            name: "notes".into(),
            note: "first".into(),
            rules: Vec::new(),
            ..Default::default()
        };
        library.put(profile.clone()).expect("saved");
        assert_eq!(Profile::load(&dir, "notes").expect("on disk").note, "first");

        // Typing in it is live at once...
        profile.note = "a much longer note, typed a letter at a time".into();
        library.put(profile.clone()).expect("held");
        assert_eq!(library.get("notes").expect("live").note, profile.note);
        // ...but not yet on disk: a file write per keystroke is what this avoids.
        assert_eq!(
            Profile::load(&dir, "notes").expect("still there").note,
            "first"
        );

        library.flush_all();
        assert_eq!(
            Profile::load(&dir, "notes").expect("caught up").note,
            profile.note
        );
        // And a flush with nothing waiting does nothing at all.
        library.flush();
        library.flush_all();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rule_switched_off_is_off_for_everybody_at_once() {
        let dir = std::env::temp_dir().join(format!("acswarm-library-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let library = Library::default();
        library.open(&dir);

        let mut profile = Profile {
            name: "party".into(),
            note: String::new(),
            rules: vec![rule(
                "peas",
                LootAction::Sell,
                vec![Ask::Item(Term::Word("pea".into()))],
            )],
            ..Default::default()
        };
        library.put(profile.clone()).expect("saved");

        // Two characters, both reading the same profile by name.
        let me = me(50, &[]);
        let pea = item("Copper Pea", item_type::MISC, 40);
        let look = || {
            library
                .get("party")
                .map(|p| p.judge(&pea, None, &me, "anyone", 0))
        };
        assert!(matches!(
            look(),
            Some(Verdict::Decided(LootAction::Sell, _))
        ));

        // Switched off: nobody re-reads or is told; the next item judged uses the new rules.
        profile.rules[0].on = false;
        library.put(profile).expect("saved again");
        assert_eq!(look(), Some(Verdict::None));

        // On disk that way once writing catches up, for the next start and whoever gets the file.
        library.flush_all();
        assert_eq!(library.reload(), 1);
        assert_eq!(look(), Some(Verdict::None));
        assert_eq!(library.names(), vec!["party".to_string()]);

        // A character told to use no profile has none, which is not an error.
        assert!(library.get("").is_none());
        assert!(library.get("no such profile").is_none());

        library.remove("party").expect("removed");
        assert!(library.get("party").is_none());
        assert!(Profile::list(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_profile_is_a_file_that_can_be_handed_to_a_friend() {
        let dir = std::env::temp_dir().join(format!("acswarm-profiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let profile = Profile {
            name: "Peas and rares".into(),
            note: "what the party takes".into(),
            rules: vec![rule(
                "peas",
                LootAction::Sell,
                vec![Ask::Item(Term::Word("pea".into()))],
            )],
            ..Default::default()
        };
        let path = profile.save(&dir).expect("saved");
        assert!(path.exists());
        assert_eq!(Profile::list(&dir), vec!["Peas and rares".to_string()]);
        let back = Profile::load(&dir, "Peas and rares").expect("read back");
        assert_eq!(back, profile);
        // A name that would not do as a file name still makes one.
        assert_eq!(tidy_name("mage/archer"), "mage-archer");
        assert_eq!(tidy_name("   "), "profile");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Every profile in one place, shared by every character in the process.
/// Nothing keeps a copy: each use looks a profile up by name, so an edit reaches everyone at once.
#[derive(Debug, Default)]
pub struct Library {
    dir: std::sync::RwLock<PathBuf>,
    by_name: std::sync::RwLock<std::collections::BTreeMap<String, std::sync::Arc<Profile>>>,
    /// Profiles changed but not yet written, and since when (see `put`).
    unwritten: std::sync::Mutex<std::collections::BTreeMap<String, std::time::Instant>>,
}

/// How long an edit waits before its file is written: no file write per keystroke (guess).
const WRITE_AFTER: std::time::Duration = std::time::Duration::from_millis(750);

impl Library {
    /// The one every session shares; a test makes its own with `Library::default()`.
    pub fn shared() -> std::sync::Arc<Library> {
        static SHARED: std::sync::OnceLock<std::sync::Arc<Library>> = std::sync::OnceLock::new();
        SHARED.get_or_init(Default::default).clone()
    }

    /// Where the files live. Setting it reads them.
    pub fn open(&self, dir: impl Into<PathBuf>) -> usize {
        *self.dir.write().unwrap_or_else(|e| e.into_inner()) = dir.into();
        self.reload()
    }

    pub fn dir(&self) -> PathBuf {
        self.dir.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Read the shelf, adding the starter profile when it is bare,
    /// so a player opening the editor first finds something to change rather than a blank page.
    pub fn open_or_start(&self, dir: impl Into<PathBuf>) -> usize {
        let n = self.open(dir);
        if n > 0 {
            return n;
        }
        match self.put(Profile::starter()) {
            Ok(()) => 1,
            Err(e) => {
                tracing::warn!("cannot write a starter loot profile: {e}");
                0
            }
        }
    }

    /// Read every profile in the directory, replacing what is held; returns how many were read.
    /// One that will not parse is logged and left out rather than taking the rest down with it.
    pub fn reload(&self) -> usize {
        let dir = self.dir();
        let mut found = std::collections::BTreeMap::new();
        for name in Profile::list(&dir) {
            match Profile::load(&dir, &name) {
                Ok(p) => {
                    found.insert(p.name.clone(), std::sync::Arc::new(p));
                }
                Err(e) => tracing::warn!("loot profile {name}: {e}"),
            }
        }
        let n = found.len();
        *self.by_name.write().unwrap_or_else(|e| e.into_inner()) = found;
        n
    }

    /// The profile of this name; an empty name is a character told to use none, not an error.
    pub fn get(&self, name: &str) -> Option<std::sync::Arc<Profile>> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        self.by_name
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(name)
            .cloned()
    }

    /// What is on the shelf, in order.
    pub fn names(&self) -> Vec<String> {
        self.by_name
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    /// Make a profile live for every character at once; an editor calls this on every keystroke.
    /// A new profile is written at once so its file exists; others wait for [`Library::flush`].
    pub fn put(&self, profile: Profile) -> std::io::Result<()> {
        let name = profile.name.clone();
        let is_new = !self
            .by_name
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&name);
        self.by_name
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(name.clone(), std::sync::Arc::new(profile));
        if is_new {
            return self.write_out(&name);
        }
        self.unwritten
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(name)
            .or_insert_with(std::time::Instant::now);
        Ok(())
    }

    /// Write out anything changed at least `WRITE_AFTER` ago.
    /// Called once a frame, so it is almost always a lock and a look at the clock.
    pub fn flush(&self) {
        let due: Vec<String> = {
            let waiting = self.unwritten.lock().unwrap_or_else(|e| e.into_inner());
            if waiting.is_empty() {
                return;
            }
            let now = std::time::Instant::now();
            waiting
                .iter()
                .filter(|(_, since)| now.duration_since(**since) >= WRITE_AFTER)
                .map(|(name, _)| name.clone())
                .collect()
        };
        for name in due {
            if let Err(e) = self.write_out(&name) {
                tracing::warn!("cannot write loot profile {name}: {e}");
            }
        }
    }

    /// Write everything outstanding now, for shutting down, where a moment later never comes.
    pub fn flush_all(&self) {
        let due: Vec<String> = self
            .unwritten
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect();
        for name in due {
            if let Err(e) = self.write_out(&name) {
                tracing::warn!("cannot write loot profile {name}: {e}");
            }
        }
    }

    fn write_out(&self, name: &str) -> std::io::Result<()> {
        let dir = self.dir();
        let profile = self
            .by_name
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(name)
            .cloned();
        self.unwritten
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(name);
        match profile {
            Some(p) => p.save(&dir).map(|_| ()),
            None => Ok(()),
        }
    }

    /// Forget one and delete its file.
    pub fn remove(&self, name: &str) -> std::io::Result<()> {
        let dir = self.dir();
        self.unwritten
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(name.trim());
        self.by_name
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(name.trim());
        let path = Profile::path_of(&dir, name);
        match std::fs::remove_file(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }
}
