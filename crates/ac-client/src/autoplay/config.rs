use serde::{Deserialize, Serialize};

#[cfg(doc)]
use super::critter;

/// Staying alive.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Survive {
    /// Heal when health falls below this fraction of its maximum.
    pub heal_below: f32,
    /// Use a carried healing kit.
    pub use_kits: bool,
    /// Keep mana up by pouring stamina into it, and stamina up with
    /// Revitalize, the way a caster does: the transfer gives more mana
    /// than the Revitalize costs, so the round is a gain.
    pub manage_mana: bool,
    /// Pour stamina into mana when mana is under this fraction.
    pub mana_below: f32,
    /// Revitalize when stamina is under this fraction.
    pub stamina_below: f32,
    /// After a death, go back for the corpse and take the gear off it
    /// (see `crate::recovery`).
    pub recover_corpse: bool,
    /// Give up on the corpse when it has not been reached in this many
    /// minutes.
    pub corpse_minutes: f32,
    /// While the vitae penalty is at least `vitae_above`, leave the
    /// hard fights and whatever killed us alone.
    pub vitae_wait: bool,
    /// The vitae penalty, as a fraction, from which the fights are
    /// picked with care: 0.25 is five deaths' worth.
    pub vitae_above: f32,
    /// Step out of the way of spells flying at us instead of standing
    /// in them (see `crate::dodge`).
    pub dodge: bool,
}

impl Default for Survive {
    fn default() -> Self {
        Survive {
            heal_below: 0.6,
            use_kits: true,
            manage_mana: true,
            mana_below: 0.4,
            stamina_below: 0.3,
            recover_corpse: true,
            corpse_minutes: 10.0,
            vitae_wait: true,
            vitae_above: 0.25,
            dodge: true,
        }
    }
}

/// Buffs to keep up.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Buffs {
    /// Work the buffs out from the character itself: every Life and
    /// Creature self-enchantment it knows for the skills it has trained,
    /// the Item auras for the way it fights, and the armour spells on
    /// each piece worn; the highest level of each (see `crate::buffs`).
    pub auto: bool,
    /// The lowest chance of a cast landing that is still worth the mana, for every spell autoplay
    /// casts (`can_cast_well`): one that fizzles more often is passed over for one below it. Half
    /// is the point where the school's skill equals the spell's power.
    pub least_chance: f32,
    /// Spell names to keep on the character as well, by hand.
    pub spells: Vec<String>,
    /// In a quiet moment, put back any buff with this many seconds or
    /// fewer left. Wide on purpose: refreshing a few at every lull
    /// spreads the work out, so the set never all runs out at once and
    /// the character is never stood still for twenty casts in a row.
    pub top_up_within: f32,
    /// A buff with this many seconds or fewer left is put back at once,
    /// fight or no fight, swapping to a wand for it if need be. A buff
    /// must never be allowed to run out: the protections going down in
    /// the middle of a fight is how a character dies.
    pub never_below: f32,
    /// Only top up out of combat. This is about the weapon: with a wand
    /// already in hand `never_below` holds mid-fight as it always did,
    /// and otherwise a buff that has actually run out still goes back
    /// up while one with time left on it waits for the fight to end
    /// rather than costing the character its weapon for a tick (see
    /// `buff_within`).
    pub out_of_combat_only: bool,
    /// Keep this fraction of mana back from buffing, for healing and
    /// fighting. A character that spends its last point on Quickness
    /// Self cannot heal, and buffs are the one thing that can wait.
    pub keep_mana: f32,
}

impl Default for Buffs {
    fn default() -> Self {
        Buffs {
            auto: true,
            least_chance: 0.5,
            spells: Vec::new(),
            top_up_within: 300.0,
            never_below: 60.0,
            out_of_combat_only: true,
            keep_mana: 0.35,
        }
    }
}

/// What to fight.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fight {
    pub enabled: bool,
    /// How to fight: with a weapon in hand, at range, or with spells.
    pub style: Style,
    /// Attack spells to throw, by name. Empty means every attack spell
    /// in the spellbook: of the ones that can be cast right now, the one
    /// the target is weakest to is used. Only read when fighting with
    /// magic.
    pub spells: Vec<String>,
    /// Wield the best weapon carried for whatever is being fought: the
    /// one whose element it takes most damage from, rending and
    /// criticals counted (see `crate::weapons`).
    pub pick_weapon: bool,
    /// Only attack creatures whose name contains one of these; empty
    /// means anything that can be attacked.
    pub only: Vec<String>,
    /// Never attack creatures whose name contains one of these.
    pub avoid: Vec<String>,
    /// Walk past a creature that has started nothing with anyone when
    /// a swing or two would end it and the character has outgrown it:
    /// a Rabbit, a Chicken, a Bunny, a Cow (see [`critter`]). Never a
    /// reason not to hit back.
    ///
    /// Defaulted by name, because serde fills a missing field from its
    /// type and a settings file written before this existed would
    /// otherwise turn it off.
    #[serde(default = "yes")]
    pub skip_critters: bool,
    /// Walk past what stands about while the character is on its way
    /// somewhere it decided to go: out to a hunting ground, back from
    /// town, round the counters. Getting there is the errand; the
    /// fighting is what the ground at the far end is for. Never a
    /// reason not to hit back.
    ///
    /// Defaulted by name for the same reason as `skip_critters`.
    #[serde(default = "yes")]
    pub walk_past_on_the_way: bool,
    /// Farthest creature to pick, metres.
    pub radius: f32,
    /// Make more ammunition when out, from a bundle of heads and a
    /// bundle of shafts carried, if Fletching is up to it.
    pub craft_ammo: bool,
    /// Summon a creature from an essence carried to fight beside the
    /// character (see `crate::summoning`).
    pub summon: bool,
    /// Hunt only here (see `crate::hunt`): fight what stands inside it,
    /// let what leaves it go, and come back to it. `None` hunts wherever
    /// the character is.
    pub area: Option<crate::hunt::HuntArea>,
}

impl Default for Fight {
    fn default() -> Self {
        Fight {
            enabled: true,
            style: Style::Auto,
            spells: Vec::new(),
            pick_weapon: true,
            craft_ammo: true,
            summon: true,
            area: None,
            only: Vec::new(),
            avoid: Vec::new(),
            skip_critters: true,
            walk_past_on_the_way: true,
            radius: 25.0,
        }
    }
}

/// What a bool setting defaults to when a settings file leaves it out.
fn yes() -> bool {
    true
}

/// Which weapon the character should be holding.
///
/// How a character fights is not a setting: it follows what is in its
/// hands. A wand, orb or staff casts, a bow or crossbow shoots, a sword
/// swings. So this does not choose a stance, it chooses a weapon:
/// [`Style::Auto`] fights with whatever is already held, and the other
/// three wield a weapon of that kind first, for a character that
/// carries more than one and should only use the one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Style {
    #[default]
    Auto,
    Melee,
    Missile,
    Magic,
}

impl Style {
    pub fn label(self) -> &'static str {
        match self {
            Style::Auto => "whatever is held",
            Style::Melee => "a melee weapon",
            Style::Missile => "a bow or thrown weapon",
            Style::Magic => "a wand or staff",
        }
    }

    pub const ALL: [Style; 4] = [Style::Auto, Style::Melee, Style::Missile, Style::Magic];
}

/// Which loot profile this character reads.
///
/// Everything about looting -- what to take, what to do with it, when a
/// body outranks the next fight, whether to salvage -- is the profile's
/// (`crate::profile::Looting`). A character without one does not loot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Loot {
    /// The profile's name. None, or a name nothing on the shelf answers
    /// to, means nothing is looted at all.
    ///
    /// Defaulted by name rather than by `Default::default`, because a
    /// settings file that mentions `loot` at all and leaves this out
    /// would otherwise get an empty string -- serde fills a missing
    /// field from its type, not from the struct's own default -- and a
    /// character would quietly stop reading its profile.
    #[serde(default = "starter")]
    pub profile: String,
}

/// The profile the shelf seeds itself with, which is what a character
/// reads when nobody has said otherwise.
fn starter() -> String {
    "Starter".to_string()
}

impl Default for Loot {
    fn default() -> Self {
        Loot { profile: starter() }
    }
}

/// What this character does for the others playing alongside it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// Attacks the team's target.
    #[default]
    Fighter,
    /// Lands the debuffs on the team's target before the others hit it.
    Debuffer,
    /// Heals whoever is worst off, and fights only when everyone is
    /// healthy.
    Healer,
}

impl Role {
    pub fn label(self) -> &'static str {
        match self {
            Role::Fighter => "fighter",
            Role::Debuffer => "debuffer",
            Role::Healer => "healer",
        }
    }
}

/// Hunting with the other characters being played.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Team {
    pub enabled: bool,
    pub role: Role,
    /// Attack whatever the team is attacking rather than picking alone.
    pub focus_fire: bool,
    /// Form a fellowship and recruit the others.
    pub fellowship: bool,
    /// The fellowship's name.
    pub fellowship_name: String,
    /// Spells a debuffer lands on the team's target, in order
    /// ("Imperil", "Magic Yield Other").
    pub debuffs: Vec<String>,
    /// Hand a teammate standing next to us what they are short of.
    pub share_supplies: bool,
    /// This character leads: the others come to it, follow it about and
    /// fly when it flies. The one played by hand, usually. Without one
    /// the leader is whoever's name sorts first, and nobody follows.
    pub lead: bool,
    /// Follow the leader about (a character that leads never does).
    pub follow: bool,
    /// How close to keep to the leader, metres.
    pub follow_distance: f32,
    /// A follower fights only what stands within this of its leader,
    /// metres: further off, a monster would draw it away.
    pub fight_radius: f32,
    /// When the party stops hunting to restock, how it makes the trip,
    /// and what it does about money. How much of each thing to carry
    /// is the loot profile's buy list.
    pub restock: crate::logistics::Restock,
}

impl Default for Team {
    fn default() -> Self {
        Team {
            enabled: false,
            role: Role::Fighter,
            focus_fire: true,
            fellowship: true,
            fellowship_name: "acswarm".into(),
            debuffs: Vec::new(),
            share_supplies: true,
            lead: false,
            follow: true,
            follow_distance: 4.0,
            fight_radius: 25.0,
            restock: crate::logistics::Restock::default(),
        }
    }
}

/// A quest autoplay works before hunting for XP (`crate::autoplay::growth::pursuit`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Pursuit {
    /// The `ac_world::quests` id.
    pub id: String,
    /// Kept after a hand-in and done again whenever its wait is over.
    pub again: bool,
    /// Unix second it was asked for: a hand-in at or after it finishes a pursuit not `again`.
    pub since: u64,
}

/// The quests autoplay considers (`crate::quest_log`); off, the quest rule wants nothing.
/// A quest paying XP is on until switched off; any other is off until switched on by hand.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Quests {
    /// Autoplay considers quests at all.
    pub on: bool,
    /// Quests switched off, by `ac_world::quests` id; one paying XP the data adds later starts on.
    pub off: Vec<String>,
    /// Quests switched on by hand: every way to complete one is worked, XP or not, unread or not.
    pub chosen: Vec<String>,
    /// Quests worked before hunting for XP, first first (the Quests window, `pursue_quest`);
    /// every way to complete one is worked, as for one chosen.
    pub pursue: Vec<Pursuit>,
}

impl Quests {
    /// Whether the quest `id` is one autoplay considers: pursued, quests off or on; else chosen, or
    /// paying XP and not off.
    pub fn is_on(&self, id: &str) -> bool {
        self.is_pursued(id)
            || (self.on
                && (self.is_chosen(id)
                    || (crate::quest_log::is_automatic_quest(id)
                        && !self.off.iter().any(|o| o == id))))
    }

    /// Whether `id` was switched on by hand.
    pub fn is_chosen(&self, id: &str) -> bool {
        self.chosen.iter().any(|c| c == id)
    }

    /// Whether autoplay works `row` of a quest it considers: any row of one chosen or pursued,
    /// else only one it works on its own ([`crate::quest_log::is_automatic`]).
    pub fn is_worked(&self, row: &ac_world::quests::Row) -> bool {
        crate::quest_log::is_automatic(row) || self.is_chosen(row.id) || self.is_pursued(row.id)
    }

    /// Whether `id` is on the pursuit list.
    pub fn is_pursued(&self, id: &str) -> bool {
        self.pursue.iter().any(|p| p.id == id)
    }

    /// Pursue `id` from `now`: once goes first, again goes last; one already listed moves.
    pub fn pursue_quest(&mut self, id: &str, again: bool, now: u64) {
        self.stop_pursuing(id);
        let p = Pursuit {
            id: id.to_string(),
            again,
            since: now,
        };
        if again {
            self.pursue.push(p);
        } else {
            self.pursue.insert(0, p);
        }
    }

    /// Take `id` off the pursuit list.
    pub fn stop_pursuing(&mut self, id: &str) {
        self.pursue.retain(|p| p.id != id);
    }

    /// Move `id` one place up (`up`) or down the pursuit list.
    pub fn move_pursuit(&mut self, id: &str, up: bool) {
        let Some(i) = self.pursue.iter().position(|p| p.id == id) else {
            return;
        };
        let j = if up {
            i.checked_sub(1)
        } else {
            Some(i + 1).filter(|&j| j < self.pursue.len())
        };
        if let Some(j) = j {
            self.pursue.swap(i, j);
        }
    }

    /// Switch `id` on by hand: every way to complete it is worked.
    pub fn switch_on(&mut self, id: &str) {
        self.off.retain(|o| o != id);
        if !self.is_chosen(id) {
            self.chosen.push(id.to_string());
        }
    }

    /// Switch `id` off, chosen or not.
    pub fn switch_off(&mut self, id: &str) {
        self.chosen.retain(|c| c != id);
        if !self.off.iter().any(|o| o == id) {
            self.off.push(id.to_string());
        }
    }
}

/// Everything the character does on its own.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub enabled: bool,
    pub survive: Survive,
    pub buffs: Buffs,
    pub fight: Fight,
    pub loot: Loot,
    pub team: Team,
    /// Growing and keeping supplied over the hours (see `crate::growth`).
    pub growth: crate::growth::Growth,
    /// Getting a new character through the Training Academy (see
    /// `crate::academy`).
    pub academy: crate::academy::Academy,
    /// Quests to do, and which are switched off.
    pub quests: Quests,
}

#[cfg(test)]
mod tests;
