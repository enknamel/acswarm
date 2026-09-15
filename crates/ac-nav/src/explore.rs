//! Finding the fight in a dungeon by walking it: [`next_step`] gives the next doorway.
//! The server describes creatures only in the character's cell and the cells visible from it, so
//! waiting finds nothing (Holtburg Dungeon: 58 creatures in 74 rooms; its portal room's two
//! generators are server-side and never described).
//! A doorway, not a far room: the navigation lattice covers only loaded cells and a dungeon bends.

use std::collections::{HashMap, HashSet, VecDeque};

/// The cells a room's portals lead to, read from the `EnvCell`'s portal list rather than geometry.
pub trait Doors {
    fn beyond(&self, cell: u32) -> Vec<u32>;
}

/// The first doorway toward the nearest room not yet stood in; `blocked` rooms are neither entered
/// nor crossed. `None` is a dungeon walked: the caller forgets `seen` and walks again for respawns.
pub fn next_step(
    here: u32,
    doors: &impl Doors,
    seen: &HashSet<u32>,
    blocked: &HashSet<u32>,
) -> Option<u32> {
    let mut queue = VecDeque::from([here]);
    let mut walked = HashSet::from([here]);
    // The doorway out of `here` that each room was reached through.
    let mut first: HashMap<u32, u32> = HashMap::new();
    while let Some(cell) = queue.pop_front() {
        for next in doors.beyond(cell) {
            if blocked.contains(&next) || !walked.insert(next) {
                continue;
            }
            let hop = *first.get(&cell).unwrap_or(&next);
            if !seen.contains(&next) {
                return Some(hop);
            }
            first.insert(next, hop);
            queue.push_back(next);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rooms in a line: 1 - 2 - 3 - 4, with a branch 2 - 9.
    struct Map;
    impl Doors for Map {
        fn beyond(&self, cell: u32) -> Vec<u32> {
            match cell {
                1 => vec![2],
                2 => vec![1, 3, 9],
                3 => vec![2, 4],
                4 => vec![3],
                9 => vec![2],
                _ => vec![],
            }
        }
    }

    fn step(here: u32, seen: &[u32]) -> Option<u32> {
        next_step(here, &Map, &seen.iter().copied().collect(), &HashSet::new())
    }

    #[test]
    fn the_room_next_door_is_stepped_straight_into() {
        assert_eq!(step(1, &[1]), Some(2));
    }

    #[test]
    fn a_room_further_off_is_walked_to_one_doorway_at_a_time() {
        // Only the branch off room 2 is unseen, three doorways back: each answer is the next
        // doorway toward it, not the branch.
        assert_eq!(step(4, &[1, 2, 3, 4]), Some(3));
        assert_eq!(step(3, &[1, 2, 3, 4]), Some(2));
        assert_eq!(step(2, &[1, 2, 3, 4]), Some(9));
    }

    #[test]
    fn a_dungeon_walked_through_has_nowhere_left_to_step() {
        assert_eq!(step(1, &[1, 2, 3, 4, 9]), None);
    }

    #[test]
    fn a_room_that_cannot_be_got_into_is_not_walked_through() {
        // Room 3 will not admit us, so room 4 is unreachable and only the branch is left.
        let seen = HashSet::from([1, 2]);
        let blocked = HashSet::from([3]);
        assert_eq!(next_step(2, &Map, &seen, &blocked), Some(9));
        let seen = HashSet::from([1, 2, 9]);
        assert_eq!(next_step(2, &Map, &seen, &blocked), None);
    }

    #[test]
    fn a_room_with_no_doors_leads_nowhere() {
        assert_eq!(step(77, &[]), None);
    }

    #[test]
    fn the_room_we_stand_in_is_never_the_answer() {
        assert_eq!(step(1, &[]), Some(2), "not room one, where we are");
    }
}
