//! What the quests switched on want carried: an item and how many, handed in from the client's
//! quest board, so the loot rule "wanted by an active quest" stays a pure function of a snapshot.

/// One item an active quest wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct QuestWant {
    pub wcid: u32,
    /// How many are wanted; `None` when every one pays (a quest repeated any time).
    pub need: Option<u32>,
}

/// Whether an item of `wcid` is wanted while `held` are carried.
pub fn wanted(wants: &[QuestWant], wcid: u32, held: u32) -> bool {
    wants
        .iter()
        .any(|w| w.wcid == wcid && w.need.is_none_or(|n| held < n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wanted_while_fewer_are_held_than_needed_and_always_when_any_number_pays() {
        let five = [QuestWant {
            wcid: 3684,
            need: Some(5),
        }];
        assert!(wanted(&five, 3684, 4));
        assert!(!wanted(&five, 3684, 5));
        assert!(!wanted(&five, 1, 0), "another item");
        let every = [QuestWant {
            wcid: 3684,
            need: None,
        }];
        assert!(wanted(&every, 3684, 1_000));
        assert!(!wanted(&[], 3684, 0), "no quest on, nothing wanted");
    }
}
