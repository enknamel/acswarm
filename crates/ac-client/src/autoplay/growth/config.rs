use serde::{Deserialize, Serialize};

/// Growing the character and keeping it supplied.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Growth {
    /// Spend unassigned experience on skills, attributes and vitals.
    pub auto_xp: bool,
    /// Go to a hunting ground that suits the level when nothing is about.
    pub hunt_grounds: bool,
    /// How many levels either side of the character's a ground may be
    /// (see `ac_world::hunting::Ground::suits`).
    pub level_margin: u32,
    /// Hunt this landblock and no other, 0 to pick whatever suits.
    /// A party follows its leader's choice.
    #[serde(default)]
    pub hunt_at: u32,
    /// How to hunt the ground once there.
    #[serde(default)]
    pub tactic: ac_world::hunting::Tactic,
    /// Seconds with nothing to fight before moving on.
    pub idle_before_move: f32,
    /// Walk to a vendor when the pack is full or supplies are short.
    pub town_runs: bool,
    /// How much ammunition to carry for a bow or crossbow. A run to
    /// town is made when a quarter of this is left and none can be
    /// made from what is carried.
    pub ammo_keep: u32,
    /// Quest flags this character has earned, for the counters that
    /// ask for one (the Rossu Morta and Whispering Blade chapter
    /// houses, the Academy stores, and so on -- see
    /// `ac_world::shops::Gate`).
    ///
    /// The server never tells a client which quests it has done, so
    /// this is the player's word for it. Empty means those doors are
    /// treated as shut, which costs a walk to the next counter rather
    /// than a walk to a door that will not open. Society membership is
    /// not listed here: the character carries that itself.
    pub gates_open: Vec<String>,
}

impl Default for Growth {
    fn default() -> Self {
        Growth {
            auto_xp: true,
            hunt_grounds: true,
            level_margin: 8,
            hunt_at: 0,
            tactic: ac_world::hunting::Tactic::default(),
            idle_before_move: 60.0,
            town_runs: true,
            ammo_keep: 250,
            gates_open: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests;
