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
    /// Loot taken for a counter is reason enough for a run to town on
    /// its own -- the pack need not be full nor a supply short -- once
    /// it is worth this much at face value, in pyreals. 0 turns the
    /// rule off. See `worth_a_sale_run`.
    pub sell_run_value: u32,
    /// The same, once this many things are being carried for a
    /// counter, whatever they are worth: they are slots as well as
    /// money. 0 turns the rule off.
    pub sell_run_count: u32,
    /// The same, once anything has been carried for a counter this
    /// long, in seconds, however little it is: one Lead Pea is not
    /// worth a trip, but it is not worth carrying about all afternoon
    /// either. 0 turns the rule off.
    pub sell_run_patience: f32,
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
            // Five thousand at face is a few thousand in hand at any
            // counter's rate: minutes of hunting at the levels a
            // character first carries loot, and worth the minutes the
            // walk costs. An Iron Pea and a Lead Pea, three thousand
            // between them, are not -- they go on the count or the
            // patience.
            sell_run_value: 5_000,
            // Eight things for a counter is an armful: slots are going,
            // and it is well short of the pack filling by itself.
            sell_run_count: 8,
            // A quarter of an hour, near twice the least wait between
            // runs (`RUN_EVERY`): a character with one pea to sell goes
            // at most once in that while, and is not still carrying it
            // at dinner.
            sell_run_patience: 15.0 * 60.0,
        }
    }
}

#[cfg(test)]
mod tests;
