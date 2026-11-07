//! What the quests on the board add to a kill on each ground, in the picker's units: XP a kill.
//! A kill task taken up pays its reward over its kills; an item quest its reward over the item's
//! count at the drop's chance. Spread over a ground by its quest creatures' share of its spawns.

use std::collections::BTreeMap;

use ac_loot::quest_wants::{wanted, QuestWant};
use ac_world::hunting::Ground;
use ac_world::quests::{Kind, Want, Yield};

use crate::quest_log::{QuestLine, QuestState};

/// Quest XP a kill earns times its creature's spawn entries, summed per landblock: divided by a
/// ground's own `count` ([`ground_bonus`]) it is the quest XP a kill there earns on average.
/// `left`: the items still wanted beyond those carried (`Client::quest_wants_left`).
pub(super) fn quest_pay(
    board: &[QuestLine],
    left: &[QuestWant],
    level: u32,
    to_next: u64,
) -> BTreeMap<u32, f32> {
    // Per want, the best quest's XP for one more kill or item: one item serves one hand-in.
    let mut per_unit: BTreeMap<&'static Want, f32> = BTreeMap::new();
    for line in board.iter().filter(|l| is_paying(l)) {
        for r in crate::quest_log::worked_rows(line) {
            let more = match r.want {
                Want::Item(w) => wanted(left, w, 0),
                _ => true,
            };
            if r.kind == Kind::Start || !r.fits_level(level) || !more {
                continue;
            }
            let xp = r.reward.xp_at(to_next) as f32 / r.count.max(1) as f32;
            let best = per_unit.entry(&r.want).or_insert(0.0);
            *best = best.max(xp);
        }
    }
    let mut pay = BTreeMap::new();
    for (want, xp) in per_unit {
        for s in ac_world::quests::sources_of(want) {
            if s.kind == Yield::Npc {
                continue;
            }
            for &(landblock, entries) in &s.landblocks {
                *pay.entry(landblock).or_insert(0.0) += xp * s.chance * entries as f32;
            }
        }
    }
    pay
}

/// The quest XP a kill on `g` earns on average, from [`quest_pay`].
pub(super) fn ground_bonus(pay: &BTreeMap<u32, f32>, g: &Ground) -> f32 {
    pay.get(&g.landblock)
        .map_or(0.0, |p| p / g.count.max(1) as f32)
}

/// Whether a kill or a drop may bring the quest nearer: a kill task taken up with kills to go
/// (one not taken up counts nothing, QuestManager.cs:606-607), an item quest open; [`quest_pay`]
/// then counts only the items still wanted.
fn is_paying(line: &QuestLine) -> bool {
    matches!(
        (&line.state, line.kind),
        (QuestState::Needs { .. }, Kind::Kill)
            | (QuestState::Needs { .. } | QuestState::Ready, Kind::Item)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autoplay::{LootAction, Quests};
    use crate::quest_log::{quest_board, quest_wants, QuestFacts, ANY_HAND_INS};

    const TAIL: &str = "give:3684:2470";
    const SHRETH: &str = "killtaskbloodshreth";
    /// XP from level 15 to 16 is not read here: any number scales the level-proportional part.
    const TO_NEXT: u64 = 50_000;

    /// The board of one quest, everything else switched off.
    fn board(only: &str, facts: impl FnOnce(&mut QuestFacts)) -> Vec<QuestLine> {
        let mut f = QuestFacts {
            level: 15,
            to_next: TO_NEXT,
            config: Quests {
                on: true,
                off: Vec::new(),
                ..Default::default()
            },
            now: 1_000_000,
            ..QuestFacts::default()
        };
        facts(&mut f);
        let rows: Vec<_> = ac_world::quests::rows_of(only).cloned().collect();
        quest_board(&rows, &f)
    }

    /// [`quest_pay`] with the board's wants less `held` tails carried.
    fn pay_of(lines: &[QuestLine], held: u32) -> BTreeMap<u32, f32> {
        let left: Vec<QuestWant> = quest_wants(lines)
            .into_iter()
            .map(|w| QuestWant {
                need: w
                    .need
                    .map(|n| n.saturating_sub(if w.wcid == 3684 { held } else { 0 })),
                ..w
            })
            .filter(|w| w.need != Some(0))
            .collect();
        quest_pay(lines, &left, 15, TO_NEXT)
    }

    fn rat_ground() -> Ground {
        ac_world::hunting::at(0x1C4)
            .expect("the Red Rat ground")
            .clone()
    }

    #[test]
    fn a_ground_where_an_active_quests_item_drops_wins_over_a_slightly_better_one() {
        let lines = board(TAIL, |_| {});
        assert!(
            matches!(lines[0].state, QuestState::Needs { .. }),
            "{lines:?}"
        );
        let pay = pay_of(&lines, 0);
        let rats = rat_ground();
        // 1,000 XP at 0.15 a Red Rat, 17 of the ground's 57 spawn entries.
        let bonus = ground_bonus(&pay, &rats);
        assert!(
            (bonus - 1000.0 * 0.15 * 17.0 / 57.0).abs() < 0.01,
            "{bonus}"
        );
        let better = Ground {
            landblock: 0xFFFE,
            xp: rats.xp + 20,
            ..rats.clone()
        };
        assert!(better.worth() > rats.worth());
        assert_eq!(ground_bonus(&pay, &better), 0.0);
        assert!(rats.worth_with(bonus) > better.worth_with(ground_bonus(&pay, &better)));
        // Handed in any time, a tail already carried still pays for the next.
        let carrying = board(TAIL, |f| {
            f.carried.insert(3684, 1);
        });
        assert_eq!(carrying[0].state, QuestState::Ready);
        assert_eq!(pay_of(&carrying, 1), pay);
        // Enough kept for the hand-ins a run takes: another tail pays nothing yet.
        let full = board(TAIL, |f| {
            f.carried.insert(3684, ANY_HAND_INS);
        });
        assert!(pay_of(&full, ANY_HAND_INS).is_empty());
    }

    #[test]
    fn a_quest_switched_off_adds_nothing() {
        let off = board(TAIL, |f| f.config.off.push(TAIL.into()));
        assert_eq!(off[0].state, QuestState::Off);
        assert!(pay_of(&off, 0).is_empty());
        let quests_off = board(TAIL, |f| f.config.on = false);
        assert!(pay_of(&quests_off, 0).is_empty());
    }

    #[test]
    fn a_quest_that_cannot_complete_adds_nothing() {
        let mut lines = board(TAIL, |_| {});
        lines[0].state = QuestState::WontComplete {
            rule: "nothing cheap".into(),
            does: LootAction::Skip,
        };
        assert!(pay_of(&lines, 0).is_empty());
    }

    #[test]
    fn a_kill_task_pays_its_reward_over_its_kills_only_while_taken_up_with_kills_to_go() {
        // Not taken up: the server counts no kill for it.
        assert!(pay_of(&board(SHRETH, |_| {}), 0).is_empty());
        let taken = board(SHRETH, |f| f.log.note_kills(SHRETH, 3));
        let pay = pay_of(&taken, 0);
        // 1.33 of the level's XP over 10 kills, 17 Blood Shreth of the 36 entries in 0x0162.
        let g = ac_world::hunting::at(0x162).expect("a Blood Shreth ground");
        let each = (1.33 * TO_NEXT as f64).round() as f32 / 10.0;
        let bonus = ground_bonus(&pay, g);
        assert!((bonus - each * 17.0 / 36.0).abs() < 0.01, "{bonus}");
        // Its kills done, it waits for the hand-in, not for more kills.
        let done = board(SHRETH, |f| f.log.note_kills(SHRETH, 10));
        assert_eq!(done[0].state, QuestState::Ready);
        assert!(pay_of(&done, 0).is_empty());
    }

    #[test]
    fn a_kill_task_taken_up_draws_the_picker_to_where_its_creature_spawns() {
        let taken = board(SHRETH, |f| f.log.note_kills(SHRETH, 0));
        let pay = pay_of(&taken, 0);
        // Among the tier 1-2 grounds a new character survives.
        let easy = |g: &ac_world::hunting::Ground| g.tier().is_some_and(|t| t <= 2);
        let plain =
            ac_world::hunting::ranked_with(15, &[], |g| if easy(g) { g.worth() } else { 0.0 });
        let drawn = ac_world::hunting::ranked_with(15, &[], |g| {
            if easy(g) {
                g.worth_with(ground_bonus(&pay, g))
            } else {
                0.0
            }
        });
        let first = drawn.first().expect("a ground");
        assert!(ground_bonus(&pay, first) > 0.0, "{first:?}");
        assert_ne!(plain.first().map(|g| g.landblock), Some(first.landblock));
    }
}
