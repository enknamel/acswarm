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

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_mate_no_walk_reaches_is_not_walked_at() {
    // The runner up on the Tusker ground's ledge (0x5C49): the money for it stays, the hand-over
    // is set aside at once and nothing walks at the ledge.
    let (mut c, ledge) = crate::testkit::under_the_ledge(20);
    c.autoplay.config.team.restock.share_money = true;
    c.autoplay.config.team.restock.float = 0;
    crate::testkit::coin_in_the_pack(&mut c, 0x8000_0201, 5_000);
    c.autoplay.team = crate::testkit::view_of(vec![crate::autoplay::Mate {
        world: ledge.at,
        cell: ledge.cell,
        ..crate::testkit::mate(0x5000_0002, "Runner")
    }]);
    let growth = crate::growth::Growth::default();
    let now = Instant::now();
    let sent = c.session.actions_sent();
    assert!(!c.load_the_quartermaster("Runner", &growth, now));
    assert!(c.autoplay.growth.handed_over, "the party is held up");
    assert!(!c.is_walking(), "it walks at the ledge");
    assert_eq!(c.session.actions_sent(), sent, "it handed over from below");
}
