use super::*;

const RING: u32 = 0x8000_0300;
const FULL: u32 = 0x8000_0301;
const EMPTY: u32 = 0x8000_0302;
const JUNK: u32 = 0x8000_0303;

/// The server's word on `guid`: its mana, when it holds any (an empty stone has none).
fn appraised(c: &mut Client, guid: u32, mana: Option<(i32, i32)>) {
    let ints = mana.map_or_else(Vec::new, |(cur, max)| vec![(107, cur), (108, max)]);
    c.appraisals.insert(
        guid,
        ac_net::messages::Appraisal {
            guid,
            success: true,
            ints,
            ..Default::default()
        },
    );
}

/// Verity wearing a ring at `ring` of its 600 mana, with a charged and an empty stone and a
/// Silver Bracelet tagged mana junk in the pack.
fn wearing(ring: i32) -> Client {
    let holtburg = 0xA9B4_0019;
    let mut c = crate::testkit::standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    crate::testkit::with_a_pack(&mut c, 50);
    let me = c.world.player_guid.unwrap();
    crate::testkit::thing_in_the_pack(&mut c, RING, "Gold Ring", 1);
    let o = c.world.objects.get_mut(&RING).unwrap();
    o.item_type = ac_world::item_type::JEWELRY;
    o.container = None;
    o.wielder = Some(me);
    appraised(&mut c, RING, Some((ring, 600)));
    for (g, name) in [(FULL, "Mana Stone"), (EMPTY, "Mana Stone")] {
        crate::testkit::thing_in_the_pack(&mut c, g, name, 1);
        c.world.objects.get_mut(&g).unwrap().item_type = ac_world::item_type::MANA_STONE;
    }
    // A charged stone's appraisal holds its ItemCurMana and no ItemMaxMana, as on Coldeve.
    c.appraisals.insert(
        FULL,
        ac_net::messages::Appraisal {
            guid: FULL,
            success: true,
            ints: vec![(107, 3_000)],
            ..Default::default()
        },
    );
    appraised(&mut c, EMPTY, None);
    crate::testkit::thing_in_the_pack(&mut c, JUNK, "Silver Bracelet", 1);
    c.world.objects.get_mut(&JUNK).unwrap().item_type = ac_world::item_type::JEWELRY;
    appraised(&mut c, JUNK, Some((800, 800)));
    let stats = c.stats_of(JUNK).unwrap();
    c.autoplay.tag(&stats, LootAction::Drain);
    c.autoplay.mana_stones.looked = Some(Instant::now());
    c
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_worn_thing_under_half_its_mana_is_charged_from_a_stone() {
    let mut c = wearing(240);
    let me = c.world.player_guid.unwrap();
    let now = Instant::now();
    c.autoplay_mana_stones(now);
    assert_eq!(
        c.last_used,
        Some(me),
        "the charged stone was not used on the character"
    );
    assert!(
        c.autoplay.cast_in_flight(now),
        "the next act waits on the stone's answer"
    );
    assert!(
        !c.appraisals.contains_key(&FULL),
        "the stone is not asked about again"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn mana_junk_fills_an_empty_stone_until_the_backup_is_charged() {
    let mut c = wearing(480);
    let now = Instant::now();
    // At 80% nothing is charged; one stone charged of five, so the empty one is filled.
    c.autoplay_mana_stones(now);
    assert_eq!(c.last_used, Some(JUNK), "the bracelet was not drained");
    // With the backup full, the junk waits.
    let mut c = wearing(480);
    c.autoplay.config.mana.backup = 1;
    c.autoplay_mana_stones(now);
    assert_eq!(c.last_used, None, "drained with the backup full");
    // Nor mid-fight: only a worn thing run low is seen to then.
    let mut c = wearing(480);
    let foe = crate::testkit::standing_by(&mut c, 0x8000_0103, "Tusker Guard", 8.0);
    c.attack_target = Some(foe.guid);
    if c.in_a_fight() {
        c.autoplay_mana_stones(now);
        assert_eq!(c.last_used, None, "drained mid-fight");
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_stone_not_yet_appraised_is_asked_about_and_not_used() {
    let mut c = wearing(240);
    c.appraisals.remove(&FULL);
    c.appraisals.remove(&EMPTY);
    c.autoplay_mana_stones(Instant::now());
    assert_eq!(c.last_used, None, "a stone of unknown charge was used");
    assert!(c.appraise_queue.contains(&FULL) && c.appraise_queue.contains(&EMPTY));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_drain_that_leaves_the_junk_is_not_sent_again() {
    // "The Lace Shirt is already full of mana", every five seconds: a stone taken for empty
    // poured into the shirt instead of draining it, and the same use went out again and again.
    let mut c = wearing(480);
    let now = Instant::now();
    c.autoplay_mana_stones(now);
    assert_eq!(c.last_used, Some(JUNK));
    c.autoplay.cast_answered();
    c.last_used = None;
    // Answered, the bracelet still in the pack and the stone looked at again, still "empty".
    appraised(&mut c, EMPTY, None);
    c.autoplay_mana_stones(now + ANSWER_WAIT);
    assert_eq!(c.last_used, None, "the same drain went out again");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_charge_waits_for_the_worn_things_to_be_read_again() {
    // "charging what is worn (Pathwarden Plate Leggings is low)" every five seconds, through
    // "Your items are fully charged": the old figures were read before the new ones came.
    let mut c = wearing(240);
    const SECOND: u32 = 0x8000_0304;
    crate::testkit::thing_in_the_pack(&mut c, SECOND, "Mana Stone", 1);
    c.world.objects.get_mut(&SECOND).unwrap().item_type = ac_world::item_type::MANA_STONE;
    c.appraisals.insert(
        SECOND,
        ac_net::messages::Appraisal {
            guid: SECOND,
            success: true,
            ints: vec![(107, 3_000)],
            ..Default::default()
        },
    );
    let me = c.world.player_guid.unwrap();
    let now = Instant::now();
    c.autoplay_mana_stones(now);
    assert_eq!(c.last_used, Some(me));
    c.autoplay.cast_answered();
    c.last_used = None;
    // Answered; the ring is asked about again, its old appraisal still saying 240 of 600.
    c.autoplay_mana_stones(now + ANSWER_WAIT);
    assert_ne!(c.last_used, Some(me), "a second stone on the old figures");
    assert!(
        c.appraise_queue.contains(&RING),
        "the ring was not asked about again"
    );
    // The fresh answer: full. Nothing more is charged.
    c.appraise_queue.clear();
    appraised(&mut c, RING, Some((600, 600)));
    c.autoplay.cast_answered();
    c.last_used = None;
    c.autoplay_mana_stones(now + ANSWER_WAIT * 3);
    assert_ne!(c.last_used, Some(me), "charged with the ring full");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn mana_junk_is_taken_only_while_a_stone_waits_for_it() {
    // Thirteen pieces of mana junk taken in forty minutes, no stone to drain them into, filled
    // the pack, and the town run went every few minutes to sell them.
    let mut c = wearing(480);
    // One empty stone, and the bracelet already carried for it: no more.
    assert!(
        !c.wants_mana_junk(),
        "junk taken with every stone spoken for"
    );
    c.world.objects.remove(&JUNK);
    assert!(
        c.wants_mana_junk(),
        "an empty stone waits, and nothing to fill it"
    );
    // The backup charged: none wanted, empty stones or not.
    c.autoplay.config.mana.backup = 1;
    assert!(!c.wants_mana_junk(), "taken with the backup charged");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn stones_are_taken_only_up_to_the_backup_and_one_to_fill() {
    // Fourteen stones carried for a backup of five: past six, one on a corpse is left there.
    let mut c = wearing(480);
    assert!(c.wants_mana_stone(), "two carried, six wanted");
    c.autoplay.config.mana.backup = 1;
    assert!(
        !c.wants_mana_stone(),
        "two carried, two wanted, and a third taken"
    );
    c.autoplay.config.mana.on = false;
    c.autoplay.config.mana.backup = 5;
    assert!(!c.wants_mana_stone(), "taken with the mana rules off");
}
