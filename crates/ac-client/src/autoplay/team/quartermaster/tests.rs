use super::*;
use crate::testkit::{character_of_level, game_data};

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn supplies_are_handed_over_from_the_stack_that_is_leaving_anyway() {
    // A mate short of tapers: the stack the player said to sell
    // goes first, and the one they said to keep only when it is
    // the only one.
    let mut c = character_of_level(game_data(), 20);
    let me = c.world.player_guid.unwrap();
    let tapers = |guid: u32, stack: u32| ac_world::WorldObject {
        guid,
        name: "Prismatic Taper".into(),
        weenie_class_id: 20631,
        item_type: ac_world::item_type::SPELL_COMPONENTS,
        value: stack,
        stack_size: stack,
        max_stack_size: 1_000,
        container: Some(me),
        ..Default::default()
    };
    c.world
        .objects
        .insert(0x8000_0001, tapers(0x8000_0001, 1_000));
    let kept = c.stats_of(0x8000_0001).unwrap();
    c.autoplay.tag(&kept, LootAction::Keep);
    assert_eq!(
        c.spare_for("prismatic taper").map(|(g, _)| g),
        Some(0x8000_0001)
    );
    c.world.objects.insert(0x8000_0002, tapers(0x8000_0002, 40));
    assert_eq!(
        c.spare_for("prismatic taper").map(|(g, _)| g),
        Some(0x8000_0002),
        "nothing decided about it beats a keeper"
    );
    c.world.objects.insert(0x8000_0003, tapers(0x8000_0003, 12));
    let to_sell = c.stats_of(0x8000_0003).unwrap();
    c.autoplay.tag(&to_sell, LootAction::Sell);
    assert_eq!(
        c.spare_for("prismatic taper").map(|(g, _)| g),
        Some(0x8000_0003),
        "leaving anyway"
    );
    assert_eq!(c.spare_for("lead scarab"), None);
}
