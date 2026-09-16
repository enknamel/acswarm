use super::*;

#[test]
fn blargerton_has_room_to_loot_with_everything_he_keeps_on_him() {
    // Strength 60: capacity 9000, a limit of 13500 at 1.5, twice his
    // capacity at 18000 and the wall at 27000. He carried 13866 and
    // had taken nothing, so all of it was his own: plate 6540, four
    // foci 1600, a stack of tapers 1374, then weapons and the rest.
    let (carried, capacity) = (13_866, 9_000);
    // Counting all of it as loot left him nothing: the fault.
    assert_eq!(loot_room(carried, carried, capacity, 1.5), 0);
    // None of it is loot. He takes on loot until he reaches twice
    // his capacity, which comes before the limit does.
    assert_eq!(loot_room(carried, 0, capacity, 1.5), 18_000 - 13_866);
}

#[test]
fn kept_things_in_the_pack_do_not_fill_a_low_limit() {
    // Leaving out only what he wore still counted his foci, tapers
    // and weapons as loot: 7326 of it, over a limit of 7200 at 0.8,
    // and no room again.
    let (carried, worn, capacity) = (13_866, 6_540, 9_000);
    assert_eq!(loot_room(carried, carried - worn, capacity, 0.8), 0);
    // Measured on what a counter would take, there is room at every
    // setting the slider allows.
    for up_to in [0.5, 0.8, 1.0, 1.5, 2.0, 2.9] {
        assert!(loot_room(carried, 0, capacity, up_to) > 0, "at {up_to}");
    }
    // A lighter character at a low setting is stopped by the limit
    // itself: 4500 of loot at 0.5, whatever it keeps.
    assert_eq!(loot_room(3_000, 0, 9_000, 0.5), 4_500);
    assert_eq!(loot_room(3_000 + 4_500, 4_500, 9_000, 0.5), 0);
}

#[test]
fn loot_does_not_take_a_character_in_plate_past_twice_its_capacity() {
    // Plate 6540 and 11460 of loot is 18000, twice his capacity, and
    // no Melee or Missile Defense left. The limit alone would have
    // let him take 2040 more and fight on to 20040.
    assert_eq!(loot_room(18_000, 11_460, 9_000, 1.5), 0);
    // A thousand short of it, a thousand is all the room there is.
    assert_eq!(loot_room(17_000, 10_460, 9_000, 1.5), 1_000);
    // A player who asks for more than twice gets it: at 2.5 the line
    // is the limit's own, 22500.
    assert_eq!(loot_room(6_540, 0, 9_000, 2.5), 22_500 - 6_540);
    assert_eq!(loot_room(22_500, 15_960, 9_000, 2.5), 0);
}

#[test]
fn what_is_kept_past_twice_capacity_leaves_the_limit_and_the_wall() {
    // What it keeps is at twice its capacity on its own: loot cannot
    // slow it further and no sale gets it back under the line, so
    // the line gives way rather than leave every corpse untouched.
    assert_eq!(loot_room(18_000, 0, 9_000, 1.5), 9_000);
    assert_eq!(loot_room(20_000, 0, 9_000, 1.5), 7_000);
    assert_eq!(loot_room(20_000 + 5_000, 5_000, 9_000, 1.5), 2_000);
    // At a low setting the limit still stops it first.
    assert_eq!(loot_room(18_000, 0, 9_000, 0.5), 4_500);
    assert_eq!(loot_room(18_000 + 4_500, 4_500, 9_000, 0.5), 0);
}

#[test]
fn the_servers_wall_still_caps_the_room() {
    // A limit set by hand past three times: the limit and the line
    // both allow more than the server will hand over.
    assert_eq!(loot_room(20_000, 8_000, 9_000, 3.5), 27_000 - 20_000);
    // At the wall there is no room, however light the loot.
    assert_eq!(loot_room(27_000, 7_000, 9_000, 1.5), 0);
    assert_eq!(loot_room(30_000, 0, 9_000, 1.5), 0);
}

#[test]
fn the_room_never_passes_a_line_and_a_sale_always_makes_some() {
    let capacity = 9_000u32;
    let wall = 27_000u32;
    for up_to in [0.5f32, 0.8, 1.0, 1.5, 2.0, 2.5, 2.9] {
        let limit = (capacity as f32 * up_to) as u32;
        let line = (capacity as f32 * up_to.max(2.0)) as u32;
        for kept in (0..=30_000u32).step_by(250) {
            for loot in (0..=30_000u32).step_by(250) {
                let carried = kept + loot;
                let room = loot_room(carried, loot, capacity, up_to);
                let at = format!("up to {up_to}, kept {kept}, loot {loot}: room {room}");
                if room == 0 {
                    continue;
                }
                // Never past the limit on loot, nor the wall on
                // everything.
                assert!(loot + room <= limit, "limit: {at}");
                assert!(carried + room <= wall, "wall: {at}");
                // Never past twice capacity while what it keeps is
                // under it.
                if kept < line {
                    assert!(carried + room <= line, "line: {at}");
                }
            }
            // Sold down to what it keeps, it has room again -- unless
            // that is already at the wall, which no sale can help.
            let sold = loot_room(kept, 0, capacity, up_to);
            assert_eq!(
                sold == 0,
                kept >= wall,
                "sold down: up to {up_to}, kept {kept}"
            );
        }
    }
}

#[test]
fn no_capacity_or_no_limit_is_no_room() {
    // Strength not heard yet: capacity 0, so nothing is claimed.
    assert_eq!(loot_room(0, 0, 0, 1.5), 0);
    assert_eq!(loot_room(500, 0, 0, 1.5), 0);
    // A limit of nothing, less, or not a number is nothing.
    assert_eq!(loot_room(0, 0, 9_000, 0.0), 0);
    assert_eq!(loot_room(0, 0, 9_000, -1.0), 0);
    assert_eq!(loot_room(0, 0, 9_000, f32::NAN), 0);
    // A count of loot ahead of the server's total: all of it is loot.
    assert_eq!(loot_room(1_000, 1_500, 9_000, 1.5), 13_500 - 1_000);
}

#[test]
fn a_character_with_room_for_nothing_it_wants_goes_to_sell() {
    // Laden needed no room at all. The loot rules take only what fits,
    // so the room settled a little above nothing: forty short of a
    // mace, every body with one on it was shut as too laden, and the
    // character hunted on and never went to sell.
    let sold = 18_000 - 13_866;
    assert!(!had_enough(40, None, sold), "nothing left behind yet");
    assert!(had_enough(40, Some(300), sold), "forty short of a mace");
    // Room for it again -- tapers burnt, or a sale -- and it is not.
    assert!(!had_enough(300, Some(300), sold));
    assert!(!had_enough(sold, Some(300), sold));
    // No room at all is enough, as it always was.
    assert!(had_enough(0, None, sold));
    // A thing no sale could make room for is no reason to go: an anvil
    // heavier than the room left with every bit of loot sold.
    assert!(!had_enough(40, Some(9_000), sold));
}

#[test]
fn past_the_servers_wall_not_even_a_coin_comes_off() {
    // +Verity: 36462 carried, 7500 capacity. The server hands nothing
    // to a character past three times its capacity, whatever it weighs.
    assert!(past_the_wall(36_462, 7_500));
    // At the wall a coin still fits, and under it so do light things.
    assert!(!past_the_wall(22_500, 7_500));
    assert!(!past_the_wall(13_866, 9_000));
    // Strength not heard yet is no reason to leave every body alone.
    assert!(!past_the_wall(13_866, 0));
}
