use super::*;
use crate::testkit::{character_of_level, no_data, stand, thing_in_the_pack, with_a_pack};

const HOLTBURG: u32 = 0xA9B4_0019;
const SEEDS: u32 = 11692;
const TERESE: u32 = 11637;
const MULE: u32 = 0x5000_0002;
const NPC: u32 = 0x7000_0001;

/// The character in Holtburg with an empty pack.
fn in_holtburg() -> Client {
    let mut c = character_of_level(no_data(), 20);
    with_a_pack(&mut c, 50);
    stand(&mut c, HOLTBURG, glam::Vec3::new(84.0, 7.1, 94.0));
    c
}

/// Someone `metres` east of the character: a player, or an NPC of weenie class `wcid`.
fn beside(c: &mut Client, guid: u32, name: &str, wcid: u32, metres: f32) {
    let me = c.player.as_ref().unwrap().world_position();
    let o = ac_world::WorldObject {
        guid,
        name: name.into(),
        weenie_class_id: wcid,
        is_player: wcid == 0,
        item_type: ac_world::item_type::CREATURE,
        position: Some(ac_world::object::Position::new_flat(
            HOLTBURG,
            me + glam::Vec3::X * metres - ac_world::landblock_origin(HOLTBURG),
        )),
        ..Default::default()
    };
    c.world.objects.insert(guid, o);
}

/// A stack of `n` of weenie class `wcid` in the pack.
fn stack_of(c: &mut Client, guid: u32, wcid: u32, n: u32) {
    thing_in_the_pack(c, guid, "Little Green Seeds", n);
    c.world.objects.get_mut(&guid).unwrap().weenie_class_id = wcid;
}

/// Every give queued, as (to, item, amount).
fn gives_sent(c: &Client) -> Vec<(u32, u32, u32)> {
    let word = |b: &[u8], at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
    c.session
        .queued()
        .iter()
        .filter(|(_, b)| b.len() >= 24 && word(b, 0) == 0xF7B1)
        .filter(|(_, b)| word(b, 8) == ac_net::messages::action::GIVE_OBJECT_REQUEST)
        .map(|(_, b)| (word(b, 12), word(b, 16), word(b, 20)))
        .collect()
}

#[test]
fn give_all_sends_one_give_per_stack_each_once_the_last_has_left_the_pack() {
    let now = Instant::now();
    let mut c = in_holtburg();
    beside(&mut c, MULE, "Mule", 0, 1.0);
    stack_of(&mut c, 0x8000_0001, SEEDS, 5);
    stack_of(&mut c, 0x8000_0002, SEEDS, 3);
    assert!(c.give_all(MULE, SEEDS, None));
    c.tick_giving(now);
    assert_eq!(gives_sent(&c), [(MULE, 0x8000_0001, 5)]);
    // Not answered yet: nothing more goes.
    c.tick_giving(now);
    assert_eq!(
        gives_sent(&c).len(),
        1,
        "a give went before the last was answered"
    );
    // The player took the whole stack: the next goes.
    c.world.objects.remove(&0x8000_0001);
    c.tick_giving(now);
    assert_eq!(gives_sent(&c)[1], (MULE, 0x8000_0002, 3));
    c.world.objects.remove(&0x8000_0002);
    c.tick_giving(now);
    assert!(c.giving.is_none());
    let gave = c.gave.clone().expect("an end");
    assert_eq!((gave.given, gave.end), (8, GiveEnd::Done));
}

#[test]
fn give_all_to_a_give_emote_npc_sends_one_give_per_item() {
    // Terese's Give emote takes one seed a give, whatever the amount (Player_Inventory.cs:3442).
    let now = Instant::now();
    let mut c = in_holtburg();
    beside(&mut c, NPC, "Terese", TERESE, 1.0);
    stack_of(&mut c, 0x8000_0001, SEEDS, 3);
    assert!(c.give_all(NPC, SEEDS, None));
    for left in [3, 2, 1] {
        c.tick_giving(now);
        assert_eq!(gives_sent(&c).last(), Some(&(NPC, 0x8000_0001, left)));
        let o = c.world.objects.get_mut(&0x8000_0001).unwrap();
        o.stack_size -= 1;
        if o.stack_size == 0 {
            c.world.objects.remove(&0x8000_0001);
        }
    }
    c.tick_giving(now);
    assert_eq!(gives_sent(&c).len(), 3);
    assert_eq!(c.gave.as_ref().map(|g| g.given), Some(3));
}

#[test]
fn a_give_waits_for_the_busy_slot() {
    let now = Instant::now();
    let mut c = in_holtburg();
    beside(&mut c, MULE, "Mule", 0, 1.0);
    stack_of(&mut c, 0x8000_0001, SEEDS, 1);
    c.autoplay.cast_sent = Some(now);
    assert!(c.give_all(MULE, SEEDS, None));
    c.tick_giving(now);
    assert!(gives_sent(&c).is_empty(), "a give into a cast in the air");
    c.autoplay.cast_sent = None;
    c.tick_giving(now);
    assert_eq!(gives_sent(&c).len(), 1);
}

#[test]
fn a_mule_not_accepting_gifts_ends_the_gives_with_its_reason() {
    let now = Instant::now();
    let mut c = in_holtburg();
    beside(&mut c, MULE, "Mule", 0, 1.0);
    stack_of(&mut c, 0x8000_0001, SEEDS, 1);
    stack_of(&mut c, 0x8000_0002, SEEDS, 1);
    assert!(c.give_all(MULE, SEEDS, None));
    c.tick_giving(now);
    // Player_Inventory.cs:3308-3309: the words, then the failed save naming the item.
    c.hear_refusal("Mule is not accepting gifts right now", now);
    c.move_refused.insert(0x8000_0001, (0, now));
    c.tick_giving(now);
    assert!(c.giving.is_none());
    let end = c.gave.clone().unwrap().end;
    assert!(
        matches!(&end, GiveEnd::Refused { why, answer: Answer::Never } if why.contains("not accepting")),
        "{end:?}"
    );
    assert_eq!(
        gives_sent(&c).len(),
        1,
        "the second stack went after a refusal"
    );
}

#[test]
fn an_npc_that_does_not_want_it_ends_the_gives() {
    let now = Instant::now();
    let mut c = in_holtburg();
    beside(&mut c, NPC, "Terese", TERESE, 1.0);
    stack_of(&mut c, 0x8000_0001, 1, 1);
    assert!(c.give_all(NPC, 1, None));
    c.tick_giving(now);
    c.hear_refusal("Terese doesn't know what to do with that", now);
    c.tick_giving(now);
    let end = c.gave.clone().unwrap().end;
    assert!(
        matches!(&end, GiveEnd::Refused { why, .. } if why.contains("does not want")),
        "{end:?}"
    );
}

#[test]
fn a_refuse_emote_by_code_alone_ends_the_gives() {
    let now = Instant::now();
    let mut c = in_holtburg();
    beside(&mut c, NPC, "Terese", TERESE, 1.0);
    stack_of(&mut c, 0x8000_0001, 1, 1);
    assert!(c.give_all(NPC, 1, None));
    c.tick_giving(now);
    c.move_refused.insert(0x8000_0001, (REFUSE_EMOTE, now));
    c.tick_giving(now);
    assert!(matches!(
        c.gave.clone().unwrap().end,
        GiveEnd::Refused { .. }
    ));
}

#[test]
fn a_give_turned_away_mid_emote_goes_again_once_the_wait_is_over() {
    let now = Instant::now();
    let mut c = in_holtburg();
    beside(&mut c, NPC, "Terese", TERESE, 1.0);
    stack_of(&mut c, 0x8000_0001, SEEDS, 1);
    assert!(c.give_all(NPC, SEEDS, None));
    c.tick_giving(now);
    c.move_refused.insert(0x8000_0001, (0, now));
    c.hear_refusal("AI refuse item during emote", now);
    c.tick_giving(now);
    assert!(c.giving.is_some(), "given up mid-emote");
    c.tick_giving(now + Duration::from_millis(500));
    assert_eq!(
        gives_sent(&c).len(),
        1,
        "given again before the emote could end"
    );
    c.tick_giving(now + crate::refusals::GIVE_BUSY_AGAIN);
    assert_eq!(gives_sent(&c).len(), 2);
}

#[test]
fn autoplay_gives_only_in_reach_and_a_player_walks_first() {
    let now = Instant::now();
    let mut c = in_holtburg();
    beside(&mut c, MULE, "Mule", 0, 10.0);
    stack_of(&mut c, 0x8000_0001, SEEDS, 1);
    // Autoplay's ask stands where it is: out of reach, it stops.
    assert!(c.give_items(MULE, vec![0x8000_0001]));
    c.tick_giving(now);
    assert!(gives_sent(&c).is_empty());
    assert!(
        matches!(&c.gave.as_ref().unwrap().end, GiveEnd::Stopped(w) if w.contains("out of reach"))
    );
    // A player's ask walks there first, and gives nothing from ten metres.
    c.gave = None;
    assert!(c.give_all(MULE, SEEDS, None));
    c.tick_giving(now);
    assert!(gives_sent(&c).is_empty(), "given from out of reach");
}

#[test]
fn an_attuned_item_to_a_player_is_skipped_and_the_rest_go() {
    let now = Instant::now();
    let mut c = in_holtburg();
    beside(&mut c, MULE, "Mule", 0, 1.0);
    stack_of(&mut c, 0x8000_0001, SEEDS, 1);
    stack_of(&mut c, 0x8000_0002, SEEDS, 1);
    // Known attuned from its appraisal: not sent at all.
    let attuned = ac_net::messages::Appraisal {
        ints: vec![(ATTUNED, 1)],
        ..Default::default()
    };
    c.appraisals.insert(0x8000_0001, attuned);
    assert!(c.give_all(MULE, SEEDS, None));
    c.tick_giving(now);
    assert_eq!(gives_sent(&c), [(MULE, 0x8000_0002, 1)]);
    // Not known, the server's AttunedItem says so (Player_Inventory.cs:3280-3283), and on it goes.
    let mut c = in_holtburg();
    beside(&mut c, MULE, "Mule", 0, 1.0);
    stack_of(&mut c, 0x8000_0001, SEEDS, 1);
    stack_of(&mut c, 0x8000_0002, SEEDS, 1);
    assert!(c.give_all(MULE, SEEDS, None));
    c.tick_giving(now);
    c.move_refused.insert(0x8000_0001, (ATTUNED_ITEM, now));
    c.tick_giving(now);
    assert_eq!(gives_sent(&c)[1], (MULE, 0x8000_0002, 1));
}

#[test]
fn who_may_be_given_an_attuned_item_is_aces_rule() {
    let quest_npc = Receiver::Npc {
        takes_anything: false,
        wants_it: true,
    };
    let crier = Receiver::Npc {
        takes_anything: true,
        wants_it: false,
    };
    // A Give emote has no attuned check (Player_Inventory.cs:3437-3439).
    assert!(may_give(quest_npc, Some(1)));
    assert!(may_give(quest_npc, Some(STICKY)));
    // A player is never given an attuned one (:3280).
    assert!(!may_give(Receiver::Player, Some(1)));
    assert!(may_give(Receiver::Player, Some(0)));
    assert!(
        may_give(Receiver::Player, None),
        "unknown: the server decides"
    );
    // A taker takes all but a sticky-attuned one (:3435).
    assert!(!may_give(crier, Some(STICKY)));
    assert!(may_give(crier, Some(1)));
    assert!(
        ac_world::takers::takes_anything(5772),
        "a Town Crier takes anything"
    );
}
