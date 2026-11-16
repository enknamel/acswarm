//! Coalesced Aetheria revealed as it is carried: the Aetheria Mana Stone used on it shows its sigil
//! and names it "Aetheria"; the stone is kept (Aetheria.cs:122-233). The stone is the Empyrean
//! Facility's upper-level reward: without one this does nothing.

use std::time::{Duration, Instant};

use crate::Client;

/// The Aetheria Mana Stone (Aetheria.cs:67).
const MANA_STONE: u32 = 42645;
/// Coalesced Aetheria: blue, red, yellow (Aetheria.cs:63-65); a revealed one keeps its wcid.
const AETHERIA: [u32; 3] = [42635, 42636, 42637];
/// The name of one not yet revealed: the server's own test (Aetheria.cs:200).
const UNREVEALED: &str = "Coalesced Aetheria";
/// How long a use waits for its answer before the next: a clap, then UseDone (Aetheria.cs:150-176).
const ANSWER_WAIT: Duration = Duration::from_secs(5);

impl Client {
    /// Reveal each Coalesced Aetheria carried with the Aetheria Mana Stone, between fights, one
    /// use at a time.
    pub(crate) fn autoplay_reveal_aetheria(&mut self, now: Instant) {
        if self.in_a_fight()
            || self.autoplay.cast_in_flight(now)
            || self.autoplay.is_slot_wanted(now)
        {
            return;
        }
        if let Some((aetheria, at)) = self.autoplay.aetheria_sent {
            let named = self.world.objects.get(&aetheria).map(|o| o.name.as_str());
            if named == Some(UNREVEALED) && now.duration_since(at) < ANSWER_WAIT {
                return;
            }
            self.autoplay.aetheria_sent = None;
        }
        let carried = |wanted: &dyn Fn(&ac_world::WorldObject) -> bool| {
            self.world.inventory().find(|o| wanted(o)).map(|o| o.guid)
        };
        let Some(stone) = carried(&|o| o.weenie_class_id == MANA_STONE) else {
            return;
        };
        let Some(aetheria) =
            carried(&|o| AETHERIA.contains(&o.weenie_class_id) && o.name == UNREVEALED)
        else {
            return;
        };
        tracing::info!("autoplay: revealing a Coalesced Aetheria ({aetheria:#010x})");
        if self.use_on(stone, aetheria) {
            self.autoplay.aetheria_sent = Some((aetheria, now));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use ac_world::WorldObject;

    const ME: u32 = 0x5000_0001;

    fn carried(guid: u32, wcid: u32, name: &str) -> WorldObject {
        WorldObject {
            guid,
            weenie_class_id: wcid,
            name: name.into(),
            container: Some(ME),
            ..Default::default()
        }
    }

    #[test]
    fn coalesced_aetheria_is_revealed_with_the_mana_stone_once() {
        let now = Instant::now();
        let mut c = crate::testkit::offline_client();
        c.world.player_guid = Some(ME);
        let blue = carried(0x8000_0001, 42635, "Coalesced Aetheria");
        c.world.objects.insert(blue.guid, blue);
        // No stone: nothing to do it with.
        c.autoplay_reveal_aetheria(now);
        assert_eq!(c.autoplay.aetheria_sent, None);
        let stone = carried(0x8000_0002, 42645, "Aetheria Mana Stone");
        c.world.objects.insert(stone.guid, stone);
        c.autoplay_reveal_aetheria(now);
        assert_eq!(c.autoplay.aetheria_sent, Some((0x8000_0001, now)));
        // Revealed (the server renames it): left alone.
        c.world.objects.get_mut(&0x8000_0001).unwrap().name = "Aetheria".into();
        c.autoplay_reveal_aetheria(now);
        assert_eq!(c.autoplay.aetheria_sent, None);
    }
}
