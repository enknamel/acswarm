use super::*;

#[test]
fn essences_are_known_by_name_and_uses() {
    assert!(is_essence("Mud Golem Essence", 50));
    assert!(is_essence("Frost K'nath Essence (50)", 50));
    assert!(!is_essence("Frost K'nath Essence (50)", 0));
    assert!(!is_essence("Essence of Mana", 50));
    assert!(!is_essence("Encapsulated Spirit", 0));
}

#[test]
fn the_skill_an_essence_needs_is_not_the_number_in_its_name() {
    assert_eq!(required_skill("Fire Grievver Essence (50)"), Some(310));
    assert_eq!(required_skill("Acid Wisp Essence (180)"), Some(530));
    assert_eq!(required_skill("Mud Golem Essence"), Some(50));
    assert_eq!(required_skill("Sandstone Golem Essence"), Some(220));
    assert_eq!(required_skill("Iron Golem Essence"), Some(475));
    // Neither a golem nor a number on the ladder: the appraisal says.
    assert_eq!(required_skill("Acid Maiden Essence"), None);
    assert_eq!(required_skill("Volcanic Moar Essence (200)"), None);
}

#[test]
fn the_element_is_in_the_name() {
    assert_eq!(element_of("Acid Moar Essence (80)"), Some(Element::Acid));
    assert_eq!(element_of("Frost K'nath Essence (50)"), Some(Element::Cold));
    assert_eq!(
        element_of("Lightning Phyntos Wasp Essence (50)"),
        Some(Element::Electric)
    );
    assert_eq!(
        element_of("Volcanic Moar Essence (200)"),
        Some(Element::Fire)
    );
    assert_eq!(
        element_of("K'nath Y'nda Essence (200)"),
        Some(Element::Acid)
    );
    assert_eq!(element_of("Mud Golem Essence"), Some(Element::Bludgeon));
}

fn essence(guid: u32, element: Element, level: u32) -> Essence {
    Essence {
        guid,
        element: Some(element),
        level: Some(level),
        ready: true,
    }
}

#[test]
fn the_targets_weakness_first_then_the_highest_level() {
    let carried = [
        essence(1, Element::Bludgeon, 50),
        essence(2, Element::Acid, 310),
        essence(3, Element::Acid, 370),
        essence(4, Element::Fire, 400),
        essence(5, Element::Acid, 475),
    ];
    let weak_to_acid = |e| if e == Element::Acid { 1.5 } else { 1.0 };
    // Skill 400: the acid (80) at 370, not the fire (100) at 400 or the
    // acid (150) at 475.
    assert_eq!(choose(&carried, 400, weak_to_acid), Some(3));
    // Nothing known about the target: the highest allowed.
    assert_eq!(choose(&carried, 400, |_| 1.0), Some(4));
    // Too little skill for anything but the Mud Golem.
    assert_eq!(choose(&carried, 200, weak_to_acid), Some(1));
    assert_eq!(choose(&carried, 40, weak_to_acid), None);
}

#[test]
fn one_not_ready_or_of_unknown_level_waits() {
    let mut cooling = essence(1, Element::Acid, 80);
    cooling.ready = false;
    let unknown = Essence {
        guid: 2,
        element: Some(Element::Acid),
        level: None,
        ready: true,
    };
    let fallback = essence(3, Element::Bludgeon, 50);
    assert_eq!(choose(&[cooling, unknown, fallback], 200, |_| 1.0), Some(3));
}
