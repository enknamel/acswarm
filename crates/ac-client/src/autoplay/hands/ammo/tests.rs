use super::*;
use crate::testkit::{character_of_level, no_data};

#[test]
fn ammunition_is_made_for_the_bow_and_the_targets_weakness() {
    use ac_world::elements::Element;
    use ac_world::fletching::ammo_type;
    // Plain arrowheads (4586), fire arrowheads (5341), arrowshafts
    // (4585) and quarrel shafts (5339), by guid.
    let carried = [(4586, 1), (5341, 2), (4585, 3), (5339, 4)];
    // Fletching enough for fire arrows, against something weak to fire.
    let (r, heads, shafts) =
        choose_recipe(ammo_type::ARROW, 100, &carried, Some(Element::Fire)).expect("fire");
    assert_eq!(
        (r.result_name.as_str(), heads, shafts),
        ("Fire Arrow", 2, 3)
    );
    // Weak to cold and no cold heads carried: the hardest recipe
    // that can be made, which is still the fire one.
    let (r, _, _) =
        choose_recipe(ammo_type::ARROW, 100, &carried, Some(Element::Cold)).expect("any");
    assert_eq!(r.result_name, "Fire Arrow");
    // Not skilled enough for fire arrows: plain ones.
    let (r, heads, shafts) =
        choose_recipe(ammo_type::ARROW, 10, &carried, Some(Element::Fire)).expect("plain");
    assert_eq!((r.result_name.as_str(), heads, shafts), ("Arrow", 1, 3));
    // A crossbow wants quarrels, made on the quarrel shafts.
    let (r, _, shafts) = choose_recipe(ammo_type::BOLT, 100, &carried, None).expect("quarrels");
    assert_eq!((r.result_name.as_str(), shafts), ("Fire Quarrel", 4));
    // No dart shafts: nothing for an atlatl.
    assert!(choose_recipe(ammo_type::ATLATL, 100, &carried, None).is_none());
    // Untrained (0): nothing at all.
    assert!(choose_recipe(ammo_type::ARROW, 0, &carried, None).is_none());
}

#[test]
fn a_busy_tick_is_not_an_empty_quiver() {
    // The one caller of `ready_ammo` reads a false as "no
    // ammunition" and goes off to fletch some, dropping out of
    // combat stance to do it. With the busy test on every wield, a
    // shot still unanswered or a spell in the air made every tick a
    // false one, and an archer with a full pack was sent to make
    // arrows it was already carrying.
    let mut c = character_of_level(no_data(), 20);
    const ARROWS: u32 = 0x8000_0201;
    let me = c.world.player_guid;
    c.world.objects.insert(
        ARROWS,
        ac_world::WorldObject {
            guid: ARROWS,
            name: "Arrow".into(),
            stack_size: 100,
            valid_locations: ac_world::equip::MISSILE_AMMO,
            container: me,
            ..Default::default()
        },
    );
    c.autoplay.wanted_ammo = Some(ARROWS);
    assert!(c.wielded_ammo().is_none(), "the slot is empty");

    // A spell in the air: the wield waits, and the quiver is still
    // not empty.
    let now = Instant::now();
    c.autoplay.cast_sent = Some(now);
    assert!(c.server_busy(now));
    assert!(c.ready_ammo(), "a busy tick read as an empty quiver");
    assert_eq!(c.autoplay.wield_asked, None, "nothing sent mid-cast");

    // The spell lands and the arrows go into the slot.
    c.autoplay.cast_sent = None;
    assert!(c.ready_ammo());
    assert_eq!(c.autoplay.wield_asked.map(|(g, _)| g), Some(ARROWS));
}
