//! Finding the fight in a dungeon.
//!
//! A dungeon is described to a client a room at a time. The server
//! sends the creatures standing in the cell the character is in and in
//! the cells that one can see, and nothing else at all: Holtburg
//! Dungeon holds fifty-eight creatures across seventy-four rooms, and
//! the room its portal drops you in holds two monster generators, both
//! of them server-side and never described to anybody. So a character
//! that arrives and waits for something to fight waits for ever, and
//! reports an empty dungeon. It was never empty; it was never walked.
//!
//! What comes out of here is the *next doorway*, not the next room:
//! a room four rooms off is not a walk the router can plan, because the
//! navigation lattice only covers what is loaded and a dungeon bends
//! round corners the whole way. One room at a time is a few metres in
//! a straight line, which is a walk anything can manage, and repeating
//! it walks the dungeon.

use std::collections::{HashMap, HashSet, VecDeque};

/// The openings out of a room: the cells its portals lead to. The cell
/// data says this outright (an `EnvCell`'s portal list), so nothing
/// here has to work it out from geometry.
pub trait Doors {
    fn beyond(&self, cell: u32) -> Vec<u32>;
}

/// The room to step into next: the first doorway on the way to the
/// nearest room not yet stood in. Rooms in `blocked` are ones that
/// could not be got into, and are neither entered nor walked through.
///
/// `None` when every room reachable from here has been seen -- which is
/// a dungeon walked, not a failure: the caller forgets what it has seen
/// and walks it again, because what it came for has respawned behind
/// it.
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
        // Everything up to the end of the line is walked; the only room
        // left is the branch off room two, three doorways back. The
        // answer is the first of those three, not the branch itself.
        assert_eq!(step(4, &[1, 2, 3, 4]), Some(3));
        // And from there, the next one along.
        assert_eq!(step(3, &[1, 2, 3, 4]), Some(2));
        // And now the branch is next door.
        assert_eq!(step(2, &[1, 2, 3, 4]), Some(9));
    }

    #[test]
    fn a_dungeon_walked_through_has_nowhere_left_to_step() {
        assert_eq!(step(1, &[1, 2, 3, 4, 9]), None);
    }

    #[test]
    fn a_room_that_cannot_be_got_into_is_not_walked_through() {
        // Room three will not admit us, so room four is unreachable and
        // only the branch is left.
        let seen = HashSet::from([1, 2]);
        let blocked = HashSet::from([3]);
        assert_eq!(next_step(2, &Map, &seen, &blocked), Some(9));
        // With the branch walked as well there is nowhere left at all.
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
