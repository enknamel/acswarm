//! Walk, or make a journey of it: arithmetic over two positions, apart from anything that sends a
//! packet. Entry: [`how_to_get_there`].

/// Farthest goal walked to directly (metres): one landblock across; beyond it is a journey.
pub const WALKABLE: f32 = 192.0;

/// How to get somewhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Means {
    /// Near enough to walk to, working round whatever is in between.
    Walk,
    /// Too far to walk: portals, recalls, gems.
    Journey,
    /// A journey there is already under way; leave it alone.
    Already,
}

/// `away` in metres. Underground is deliberately no special case: a dungeon lies tens of kilometres
/// from its town, so leaving one is a journey on distance alone while its rooms stay a walk.
pub fn how_to_get_there(away: f32, travelling: bool) -> Means {
    if away <= WALKABLE {
        Means::Walk
    } else if travelling {
        Means::Already
    } else {
        Means::Journey
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn across_a_room_is_a_walk_wherever_the_room_is() {
        assert_eq!(how_to_get_there(4.0, false), Means::Walk);
        assert_eq!(how_to_get_there(120.0, false), Means::Walk);
        // Underground too: ruling that out left a character unable to reach what it was fighting.
        assert_eq!(how_to_get_there(WALKABLE, false), Means::Walk);
    }

    #[test]
    fn out_of_a_dungeon_is_never_a_walk() {
        // Holtburg Dungeon sits at (192, 47232), its town at (32448, 34560): the distance says what
        // the cell ids cannot.
        let away = ((32448.0f32 - 192.0).powi(2) + (34560.0f32 - 47232.0).powi(2)).sqrt();
        assert!(away > WALKABLE);
        assert_eq!(how_to_get_there(away, false), Means::Journey);
    }

    #[test]
    fn a_journey_under_way_is_not_started_again() {
        assert_eq!(how_to_get_there(5_000.0, true), Means::Already);
        // A short hop is walked, not left to a journey heading somewhere else.
        assert_eq!(how_to_get_there(5.0, true), Means::Walk);
    }
}
