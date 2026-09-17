//! Questions every system asks of the world: is it alive, what is it called.

use crate::{World, WorldObject};

impl WorldObject {
    /// Alive unless the server has said its health is gone: what picking a target, fighting and
    /// looting ask. ACE sends a health only for the selected target (Player_Vitals.cs:173) or an
    /// appraisal (WorldObject.cs:617), so most creatures in view have none at all.
    pub fn alive_or_unknown(&self) -> bool {
        self.health.unwrap_or(1.0) > 0.0
    }

    /// Alive on the server's word alone: what a claim to be in a fight already asks, since that
    /// claim outranks how far off the fight is and an unknown health is no ground for it.
    pub fn known_alive(&self) -> bool {
        self.health.unwrap_or(0.0) > 0.0
    }
}

impl World {
    /// The name of an object the world holds, empty or not.
    pub fn name_of(&self, guid: u32) -> Option<&str> {
        self.objects.get(&guid).map(|o| o.name.as_str())
    }

    /// The object's name, or its guid in hex when the world does not hold it.
    pub fn name_or_hex(&self, guid: u32) -> String {
        self.name_of(guid)
            .map_or_else(|| format!("{guid:#010x}"), str::to_string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_health(health: Option<f32>) -> WorldObject {
        WorldObject {
            health,
            ..Default::default()
        }
    }

    #[test]
    fn unknown_health_is_alive_to_one_and_dead_to_the_other() {
        let unknown = with_health(None);
        assert!(unknown.alive_or_unknown());
        assert!(!unknown.known_alive());
    }

    #[test]
    fn known_health_answers_both_the_same() {
        for (health, alive) in [(Some(0.5), true), (Some(1.0), true), (Some(0.0), false)] {
            let o = with_health(health);
            assert_eq!(o.alive_or_unknown(), alive, "{health:?}");
            assert_eq!(o.known_alive(), alive, "{health:?}");
        }
    }

    #[test]
    fn a_name_is_looked_up_by_guid_with_hex_when_missing() {
        const SKULKER: u32 = 0x8000_0001;
        let mut world = World::default();
        world.objects.insert(
            SKULKER,
            WorldObject {
                guid: SKULKER,
                name: "Drudge Skulker".into(),
                ..Default::default()
            },
        );
        assert_eq!(world.name_of(SKULKER), Some("Drudge Skulker"));
        assert_eq!(world.name_or_hex(SKULKER), "Drudge Skulker");
        assert_eq!(world.name_of(0xFF), None);
        assert_eq!(world.name_or_hex(0xFF), "0x000000ff");
    }
}
