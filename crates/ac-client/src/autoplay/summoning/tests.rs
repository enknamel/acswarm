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

const MUD_GOLEM: u32 = 48886;
const FROST_GRIEVVER_50: u32 = 49387;
const ACID_GRIEVVER_80: u32 = 49367;
const FROST_WASP_80: u32 = 49539;
const FIRE_MOAR_100: u32 = 49354;
const ACID_MOAR_150: u32 = 49342;

/// A ready essence of weenie class `wcid`, as the world data has it.
fn essence(guid: u32, wcid: u32) -> Essence {
    let s = summons::of(wcid).expect("an essence in the data");
    Essence {
        guid,
        wcid,
        ratings: Ratings::default(),
        element: s.element,
        level: Some(s.skill),
        ready: true,
    }
}

fn weak_to(element: Element, by: f32) -> impl Fn(Element) -> f32 {
    move |e| if e == element { by } else { 1.0 }
}

#[test]
fn the_most_damage_within_the_skill() {
    let carried = [
        essence(1, MUD_GOLEM),
        essence(2, FROST_GRIEVVER_50),
        essence(3, ACID_GRIEVVER_80),
        essence(4, FIRE_MOAR_100),
        essence(5, ACID_MOAR_150),
    ];
    // Skill 400 rules out the moar (150) at 475; nothing known of the target: the moar (100).
    assert_eq!(choose(&carried, 400, |_| 1.0), Some(4));
    // Too little skill for anything but the Mud Golem.
    assert_eq!(choose(&carried, 200, |_| 1.0), Some(1));
    assert_eq!(choose(&carried, 40, |_| 1.0), None);
}

#[test]
fn an_element_match_beats_a_higher_level_only_when_it_does_more() {
    let carried = [essence(2, FROST_GRIEVVER_50), essence(3, ACID_GRIEVVER_80)];
    // The (80) hits for 246 every 2.2 s, the (50) for 142 every 2.3 s: half again on cold is
    // not enough...
    assert_eq!(choose(&carried, 370, weak_to(Element::Cold, 1.5)), Some(3));
    // ...twice is.
    assert_eq!(choose(&carried, 370, weak_to(Element::Cold, 2.0)), Some(2));
    // And a target that takes nothing from acid gets the cold one whatever its level.
    assert_eq!(choose(&carried, 370, weak_to(Element::Acid, 0.0)), Some(2));
}

#[test]
fn a_phyntos_wasp_wins_only_with_the_element() {
    // A wasp (80) stings for 102 every 1.1 s, a grievver (80) hits for 246 every 2.2 s.
    let carried = [essence(2, FROST_WASP_80), essence(3, ACID_GRIEVVER_80)];
    assert_eq!(choose(&carried, 370, |_| 1.0), Some(3));
    assert_eq!(choose(&carried, 370, weak_to(Element::Cold, 1.5)), Some(2));
}

#[test]
fn the_essences_own_ratings_count() {
    let plain = essence(2, ACID_GRIEVVER_80);
    let mut rated = essence(3, ACID_GRIEVVER_80);
    rated.ratings.damage = 10;
    assert_eq!(choose(&[plain, rated], 370, |_| 1.0), Some(3));
    // On a target a fifth weaker to cold, a grievver (50) loses to the (80) bare, and wins with
    // the best an essence drops with (20 each: LootGenerationFactory_PetDevice.cs:33-44).
    let cold = weak_to(Element::Cold, 1.2);
    let mut lucky = essence(4, FROST_GRIEVVER_50);
    assert_eq!(choose(&[plain, lucky], 370, &cold), Some(2));
    lucky.ratings = Ratings {
        damage: 20,
        crit: 20,
        crit_damage: 20,
    };
    assert_eq!(choose(&[plain, lucky], 370, &cold), Some(4));
}

#[test]
fn one_not_ready_or_of_unknown_level_waits() {
    let mut cooling = essence(1, ACID_GRIEVVER_80);
    cooling.ready = false;
    let mut unknown = essence(2, ACID_GRIEVVER_80);
    unknown.level = None;
    let fallback = essence(3, MUD_GOLEM);
    assert_eq!(choose(&[cooling, unknown, fallback], 400, |_| 1.0), Some(3));
}

#[test]
fn one_not_in_the_data_comes_last_by_element_then_level() {
    let known = essence(1, MUD_GOLEM);
    let stranger = |guid, element, level| Essence {
        guid,
        wcid: 0,
        ratings: Ratings::default(),
        element: Some(element),
        level: Some(level),
        ready: true,
    };
    let (acid, fire) = (
        stranger(2, Element::Acid, 310),
        stranger(3, Element::Fire, 370),
    );
    assert_eq!(choose(&[acid, fire, known], 400, |_| 1.0), Some(1));
    assert_eq!(
        choose(&[acid, fire], 400, weak_to(Element::Acid, 1.5)),
        Some(2)
    );
    assert_eq!(choose(&[acid, fire], 400, |_| 1.0), Some(3));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn an_essence_used_holds_the_next_cast_until_it_is_answered() {
    // Blargerton used a Sandstone Golem Essence and cast a vulnerability 19 ms later: the
    // essence's UseDone freed the slot, a bolt went up mid-cast, and the vuln's resist was missed.
    let holtburg = 0xA9B4_0019;
    let mut c = crate::testkit::standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    crate::testkit::with_a_pack(&mut c, 50);
    crate::testkit::trained_in(&mut c, ac_world::stats::skill::SUMMONING, 400);
    c.autoplay.config.fight.enabled = true;
    c.autoplay.config.fight.summon = true;
    let foe = crate::testkit::standing_by(&mut c, 0x8000_0103, "Tusker Guard", 8.0);
    c.autoplay.casting_at = Some(foe.guid);
    crate::testkit::thing_in_the_pack(&mut c, 0x8000_0200, "Sandstone Golem Essence", 1);
    if let Some(o) = c.world.objects.get_mut(&0x8000_0200) {
        o.max_structure = 50;
        o.structure = 50;
    }
    let now = Instant::now();
    assert!(!c.autoplay.cast_in_flight(now));
    assert!(c.autoplay_summon(now), "{}", c.autoplay.status);
    assert!(
        c.autoplay.cast_in_flight(now),
        "the next cast waits on the essence's answer"
    );
    c.autoplay.cast_answered();
    assert!(!c.autoplay.cast_in_flight(now));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn an_empty_essence_is_refilled_with_a_spirit_between_fights() {
    // An Encapsulated Spirit used on an essence fills its uses; it is spent either way, so only
    // an empty one is refilled, and never in a fight (the refill changes to peace first).
    let holtburg = 0xA9B4_0019;
    let mut c = crate::testkit::standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    crate::testkit::with_a_pack(&mut c, 50);
    c.autoplay.config.fight.enabled = true;
    c.autoplay.config.fight.summon = true;
    const ESSENCE: u32 = 0x8000_0200;
    const SPIRIT: u32 = 0x8000_0201;
    crate::testkit::thing_in_the_pack(&mut c, ESSENCE, "Sandstone Golem Essence", 1);
    crate::testkit::thing_in_the_pack(&mut c, SPIRIT, "Encapsulated Spirit", 3);
    c.world.objects.get_mut(&SPIRIT).unwrap().weenie_class_id = ENCAPSULATED_SPIRIT;
    let essence = c.world.objects.get_mut(&ESSENCE).unwrap();
    essence.max_structure = 50;
    essence.structure = 12;
    let now = Instant::now();
    c.autoplay_refill_essences(now);
    assert_eq!(
        c.last_used, None,
        "a spirit spent on an essence with uses left"
    );
    c.world.objects.get_mut(&ESSENCE).unwrap().structure = 0;
    let foe = crate::testkit::standing_by(&mut c, 0x8000_0103, "Tusker Guard", 8.0);
    c.attack_target = Some(foe.guid);
    if c.in_a_fight() {
        c.autoplay_refill_essences(now);
        assert_eq!(c.last_used, None, "refilled mid-fight");
    }
    c.attack_target = None;
    c.world.objects.remove(&foe.guid);
    assert!(!c.in_a_fight());
    c.autoplay_refill_essences(now);
    assert_eq!(
        c.last_used,
        Some(ESSENCE),
        "the empty essence was not refilled"
    );
    assert!(
        c.autoplay.cast_in_flight(now),
        "the refill holds the next act"
    );
    // Asked again before it shows: not sent twice.
    c.last_used = None;
    c.autoplay.cast_answered();
    c.autoplay_refill_essences(now + Duration::from_secs(1));
    assert_eq!(c.last_used, None, "a second spirit before the first showed");
}
