use std::time::Instant;

use super::*;
use crate::autoplay::loot::choose::LOOT_TIMEOUT;
use crate::autoplay::LootAction;
use crate::testkit::{asks, game_data, no_data, on_the_body, with_packs, BODY, SACK};

#[test]
fn a_pour_refused_while_a_take_is_in_the_air_is_not_the_takes_refusal() {
    // The tidying pours in the gaps between takes, so both can be
    // out at once. A refusal that names a stack in the pack is
    // about that stack, and the take goes on waiting for its own
    // answer rather than being written off.
    let (in_the_pack, on_the_corpse) = (0x8000_7001, 0x8000_7002);
    assert_eq!(
        refused_item(in_the_pack, 0, Some(on_the_corpse)),
        Some(in_the_pack)
    );
}

#[test]
fn a_daily_limit_is_a_wait_and_not_a_grudge() {
    use crate::did::{Because, Did, Patience};
    let t0 = Instant::now();
    let mut kinds: Patience<u32> = Patience::new();

    // YouHaveSolvedThisQuestTooRecently is what gates a once-a-day
    // drop. It is a wait: the thing comes back, and a session can
    // run for days.
    let too_recently = Did::Blocked(Because::server(0x043E));
    assert_eq!(too_recently.because().and_then(|b| b.code), Some(0x043E));
    kinds.note(7299, &too_recently, t0);
    assert!(kinds.held(&7299, t0), "left alone for now");
    // Not for ever, though: a day later it is asked about again.
    assert!(
        !kinds.held(&7299, t0 + Duration::from_secs(24 * 60 * 60)),
        "a day later it is worth another ask"
    );
    // Nor is "too many times", which a raised cap can lift.
    kinds.note(7299, &Did::Blocked(Because::server(0x043F)), t0);
    assert!(!kinds.held(&7299, t0 + Duration::from_secs(24 * 60 * 60)));
    // Only a thing no counter will ever take is for ever, and that
    // is a different answer entirely.
    kinds.note(1, &Did::refused("no vendor will take it"), t0);
    assert!(kinds.held(&1, t0 + Duration::from_secs(24 * 60 * 60)));
}

#[test]
fn a_quests_refusal_that_names_no_item_is_about_the_take_in_flight() {
    // ACE turns a drop that can only be had so often down naming item
    // 0, with only "You have solved this quest too recently". Read as
    // a refusal of nothing, the take was asked for again until the
    // corpse was given up on, and the kind was never remembered.
    let (gem, dagger) = (0x8000_7001, 0x8000_7002);
    assert_eq!(refused_item(0, 0x043E, Some(gem)), Some(gem));
    assert_eq!(refused_item(0, 0x043F, Some(gem)), Some(gem));
    // With nothing in flight there is nothing to pin it on.
    assert_eq!(refused_item(0, 0x043E, None), None);
    // A refusal naming no item for any other reason is not a take's.
    assert_eq!(refused_item(0, 0, Some(gem)), None);
    // One that names its item is about that item, whatever is flying.
    assert_eq!(refused_item(dagger, 0, Some(gem)), Some(dagger));
    assert!(only_so_often(0x043E) && only_so_often(0x043F));
    assert!(!only_so_often(0));
}

#[test]
fn a_take_with_the_main_pack_full_goes_into_the_sack_with_room() {
    // Nine characters at 102/102 with a Sack at 7 of 24 aimed every
    // take at the main pack and were refused seven thousand times.
    let mut c = with_packs(no_data(), 2, 2, 24, 7);
    let me = c.world.player_guid.unwrap();
    const DAGGER: u32 = 0x8000_0401;
    on_the_body(&mut c, DAGGER, "Dagger", 21, 0, 1);
    assert_eq!(c.free_space(), 17);
    assert_eq!(c.how_to_take(DAGGER), Some(crate::room::Take::Put(SACK)));
    let now = Instant::now();
    c.take(DAGGER);
    c.tick_loot(now);
    let sent = c.loot_sent.clone().expect("a take went out");
    assert_eq!((sent.item, sent.into, sent.retried), (DAGGER, SACK, false));
    assert!(c.loot_merge.is_none(), "a put, not a pour");
    // Landed in the Sack: ours, though not in our own container.
    c.world.objects.get_mut(&DAGGER).unwrap().container = Some(SACK);
    c.tick_loot(now + Duration::from_millis(100));
    assert!(c.loot_inflight.is_none(), "the take is over");
    assert!(c.loot_sent.is_some(), "kept until the next take goes out");
    // With a slot in the main pack, that comes first, as ever.
    c.world.objects.get_mut(&me).unwrap().items_capacity = 4;
    assert_eq!(c.how_to_take(DAGGER), Some(crate::room::Take::Put(me)));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_servers_word_that_a_pack_is_full_sends_the_take_elsewhere_and_then_gives_up() {
    // The count said the main pack had two slots; the server said
    // "Unable to put Dagger into container". Believed, the next try
    // names the Sack; refused there too, the body is left for room.
    let mut c = with_packs(game_data(), 4, 2, 24, 23);
    let me = c.world.player_guid.unwrap();
    const DAGGER: u32 = 0x8000_0401;
    on_the_body(&mut c, DAGGER, "Dagger", 21, 0, 1);
    let t0 = Instant::now();
    c.take(DAGGER);
    c.tick_loot(t0);
    assert_eq!(c.loot_sent.as_ref().map(|s| s.into), Some(me));
    // In the order a tick reads them: the InventoryServerSaveFailed
    // with no reason in it first, with the words noted as said but
    // not yet read; then the words themselves, from the chat.
    let answered = t0 + Duration::from_millis(50);
    c.told = Some(answered);
    c.move_refused.insert(DAGGER, (0, answered));
    c.loot_inflight = None;
    c.take_refused(DAGGER, 0);
    assert!(
        c.packs_said_full.is_empty(),
        "something was said, and not read yet"
    );
    // Words about some other take are not about this one.
    c.hear_put_refusal("Pyreal");
    assert!(c.packs_said_full.is_empty());
    c.hear_put_refusal("Dagger");
    assert_eq!(
        c.packs_said_full.get(&me),
        Some(&2),
        "full at two, whatever the count said"
    );
    assert!(c.loot_sent.is_none(), "judged, and not read twice");
    assert_eq!(c.free_space(), 1, "the Sack's slot is all there is");
    assert_eq!(c.packs().container_for_a_take(), Some(SACK));
    assert_eq!(c.loot_queue.front(), Some(&DAGGER), "sent once more");
    assert_eq!(c.loot_retry, Some(DAGGER));
    assert!(
        !c.move_refused.contains_key(&DAGGER),
        "the refusal was the pack's, not the item's"
    );
    c.tick_loot(answered + Duration::from_millis(10));
    let sent = c.loot_sent.clone().expect("the second try");
    assert_eq!((sent.into, sent.retried), (SACK, true));
    // Refused there as well, in the same words.
    let again = answered + Duration::from_millis(60);
    c.move_refused.insert(DAGGER, (0, again));
    c.loot_inflight = None;
    c.take_refused(DAGGER, 0);
    c.hear_put_refusal("Dagger");
    assert_eq!(c.packs_said_full.get(&SACK), Some(&23));
    assert_eq!(c.free_space(), 0);
    assert!(c.loot_queue.is_empty(), "no third try");
    assert!(
        c.move_refused.contains_key(&DAGGER),
        "the rules read the refusal"
    );
    assert!(c.room_for_loot().pack_low, "the body is set aside for room");
    // Something leaves the main pack, and the server's word on it
    // lapses: the count is believed again until the server says
    // otherwise.
    c.world.objects.remove(&0x8000_0500);
    assert_eq!(c.free_space(), 3);
    assert_eq!(c.packs().container_for_a_take(), Some(me));
    assert!(
        !c.room_for_loot().pack_low,
        "room appeared; bodies wait again"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_refusal_with_no_reason_and_no_word_at_all_is_not_the_pack() {
    // A second of a unique: the server refuses it with no code, and
    // explains itself in the system chat, which is not a word about
    // the take. Read as the pack being full, it marked the main
    // pack, then the Sack, and sent the character to town with
    // seventeen slots free.
    let mut c = with_packs(game_data(), 4, 2, 24, 7);
    let me = c.world.player_guid.unwrap();
    const KEY: u32 = 0x8000_0401;
    on_the_body(&mut c, KEY, "Sturdy Iron Key", 9000, 0, 1);
    let t0 = Instant::now();
    c.take(KEY);
    c.tick_loot(t0);
    assert_eq!(c.loot_sent.as_ref().map(|s| s.into), Some(me));
    let answered = t0 + Duration::from_millis(50);
    c.move_refused.insert(KEY, (0, answered));
    c.loot_inflight = None;
    c.take_refused(KEY, 0);
    // And nothing more is said. The next tick finds nothing to send
    // and nothing to mark.
    c.tick_loot(answered);
    assert!(c.packs_said_full.is_empty(), "no word: not the pack");
    assert!(c.loot_queue.is_empty(), "and no second try");
    assert!(c.loot_inflight.is_none(), "no take in the air");
    assert!(
        c.move_refused.contains_key(&KEY),
        "the refusal was the item's, for the rules to read"
    );
    assert_eq!(c.free_space(), 17);
    assert!(!c.room_for_loot().pack_low);
}

#[test]
fn the_servers_words_count_when_they_come_a_packet_behind_the_refusal() {
    // The refusal in one packet, the words in the next: the tick
    // between has read the take as over. What was sent is kept
    // until the next goes out, so the words still find it.
    let mut c = with_packs(no_data(), 4, 2, 24, 7);
    let me = c.world.player_guid.unwrap();
    const DAGGER: u32 = 0x8000_0401;
    on_the_body(&mut c, DAGGER, "Dagger", 21, 0, 1);
    let t0 = Instant::now();
    c.take(DAGGER);
    c.tick_loot(t0);
    let answered = t0 + Duration::from_millis(50);
    c.move_refused.insert(DAGGER, (0, answered));
    c.loot_inflight = None;
    c.take_refused(DAGGER, 0);
    c.tick_loot(answered);
    assert!(c.loot_inflight.is_none());
    assert!(c.loot_sent.is_some(), "kept for the words");
    c.hear_put_refusal("Dagger");
    assert_eq!(c.packs_said_full.get(&me), Some(&2));
    assert_eq!(c.loot_queue.front(), Some(&DAGGER), "sent once more");
}

#[test]
fn the_servers_word_lifts_when_one_thing_leaves_however_the_count_climbed() {
    // Called full when the client had counted two; two descriptions
    // still on their way arrive, and the count reads four. One
    // sold: the word held at two would still stand, and the pack
    // would read as full until a third left.
    let mut c = with_packs(no_data(), 4, 2, 24, 24);
    let me = c.world.player_guid.unwrap();
    c.packs_said_full.insert(me, 2);
    let t0 = Instant::now();
    assert_eq!(c.free_space(), 0);
    for guid in [0x8000_0700, 0x8000_0701] {
        c.world.objects.insert(
            guid,
            ac_world::WorldObject {
                guid,
                name: "Dagger".into(),
                container: Some(me),
                ..Default::default()
            },
        );
    }
    c.tick_loot(t0);
    assert_eq!(
        c.packs_said_full.get(&me),
        Some(&4),
        "kept at the most seen"
    );
    assert_eq!(c.free_space(), 0);
    c.world.objects.remove(&0x8000_0700);
    c.tick_loot(t0 + Duration::from_millis(125));
    assert!(
        !c.packs_said_full.contains_key(&me),
        "one left: the word lapses"
    );
    assert_eq!(c.free_space(), 1);
}

#[test]
fn a_pack_off_the_ground_is_picked_up_into_the_main_pack_whatever_the_room() {
    // The main pack full and the Sack with room: a dagger on the
    // ground goes into the Sack, and a Pouch into the main pack's
    // pack slots, where the server would refuse it named into the
    // Sack without a word.
    let mut c = with_packs(no_data(), 2, 2, 24, 7);
    let me = c.world.player_guid.unwrap();
    const POUCH: u32 = 0x8000_0801;
    const DAGGER: u32 = 0x8000_0802;
    c.world.objects.insert(
        POUCH,
        ac_world::WorldObject {
            guid: POUCH,
            name: "Pouch".into(),
            weenie_class_id: 167,
            item_type: ac_world::item_type::CONTAINER,
            items_capacity: 12,
            position: Some(ac_world::object::Position::new_flat(0, Default::default())),
            ..Default::default()
        },
    );
    c.world.objects.insert(
        DAGGER,
        ac_world::WorldObject {
            guid: DAGGER,
            name: "Dagger".into(),
            position: Some(ac_world::object::Position::new_flat(0, Default::default())),
            ..Default::default()
        },
    );
    assert_eq!(c.where_to_pick_up(POUCH), Some(me));
    assert_eq!(c.where_to_pick_up(DAGGER), Some(SACK));
    assert_eq!(c.where_to_pick_up(0x8000_0803), None, "unknown");
}

#[test]
fn a_refusal_with_a_reason_or_other_words_is_not_read_as_a_full_pack() {
    let mut c = with_packs(no_data(), 4, 2, 24, 7);
    let me = c.world.player_guid.unwrap();
    const DAGGER: u32 = 0x8000_0401;
    on_the_body(&mut c, DAGGER, "Dagger", 21, 0, 1);
    let t0 = Instant::now();
    c.take(DAGGER);
    c.tick_loot(t0);
    // "You are too encumbered to carry that!" and the usual
    // reasonless refusal after it.
    let answered = t0 + Duration::from_millis(50);
    c.told = Some(answered);
    c.move_refused.insert(DAGGER, (0, answered));
    c.loot_inflight = None;
    c.take_refused(DAGGER, 0);
    assert!(c.packs_said_full.is_empty(), "the pack was not the reason");
    assert!(c.loot_queue.is_empty(), "and it is not asked for again");
    assert!(c.move_refused.contains_key(&DAGGER));
    // A quest's cap, with its code.
    c.take(DAGGER);
    c.tick_loot(answered);
    c.loot_inflight = None;
    c.take_refused(DAGGER, 0x043E);
    assert!(c.packs_said_full.is_empty());
    assert_eq!(c.free_space(), 17);
    assert_eq!(c.packs().container_for_a_take(), Some(me));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn coin_off_a_body_is_poured_onto_the_pile_carried_rather_than_given_a_slot() {
    // The main pack full but for the pyreals in it: the body's
    // coin joins the pile, spending no slot, and the rules take it
    // off a body they would otherwise shut as full.
    let mut c = with_packs(game_data(), 2, 1, 0, 0);
    c.world.objects.remove(&SACK);
    let me = c.world.player_guid.unwrap();
    const PILE: u32 = 0x8000_0600;
    const COIN: u32 = 0x8000_0401;
    c.world.objects.insert(
        PILE,
        ac_world::WorldObject {
            guid: PILE,
            name: "Pyreal".into(),
            weenie_class_id: 273,
            item_type: ac_world::item_type::MONEY,
            stack_size: 100,
            max_stack_size: 25_000,
            container: Some(me),
            ..Default::default()
        },
    );
    on_the_body(&mut c, COIN, "Pyreal", 273, ac_world::item_type::MONEY, 50);
    assert_eq!(c.free_space(), 0, "the pile took the last slot");
    assert_eq!(
        c.how_to_take(COIN),
        Some(crate::room::Take::Merge {
            to: PILE,
            amount: 50
        })
    );
    // The rules see it needs no slot and ask for it.
    let t0 = Instant::now();
    let profile = crate::profile::Profile {
        rules: vec![asks("coin", "pyreal", LootAction::Keep)],
        ..Default::default()
    };
    c.autoplay.take_up_corpse(BODY, t0, LOOT_TIMEOUT);
    let open = c.corpse_now(BODY, &[COIN], &profile, t0, t0);
    assert_eq!(open.slots_free, 0);
    assert!(open.items[0].needs_no_slot);
    let next = c.autoplay.loot_run.step(&open, t0);
    assert_eq!(next.act, Some(ac_loot::Act::Take(COIN)), "{}", next.saying);
    // Not while the tidying's own pour is in the air: the pile it
    // is emptying, or filling, may be the one the coin would join.
    c.take(COIN);
    c.autoplay.pour = Some((
        crate::pack::PourSent {
            merge: crate::pack::Merge {
                from: 0x8000_0601,
                to: PILE,
                amount: 10,
                name: "Pyreal".into(),
                frees_a_slot: true,
            },
            to_before: 100,
        },
        t0,
    ));
    c.tick_loot(t0);
    assert!(c.loot_sent.is_none(), "held while the tidying pours");
    assert_eq!(c.loot_queue.front(), Some(&COIN));
    c.autoplay.pour = None;
    // The take is a pour off the body, read like a tidying pour.
    c.tick_loot(t0);
    let pour = c.loot_merge.clone().expect("a pour went out");
    assert_eq!(
        (pour.merge.from, pour.merge.to, pour.merge.amount),
        (COIN, PILE, 50)
    );
    assert_eq!(pour.to_before, 100);
    assert_eq!(c.loot_sent.as_ref().map(|s| s.into), Some(PILE));
    // The source goes first; that is no answer yet.
    c.world.objects.remove(&COIN);
    c.tick_loot(t0 + Duration::from_millis(100));
    assert!(c.loot_inflight.is_some(), "the pile has not grown");
    // Nor is the tidying's two seconds without a word: the server
    // walks to the body and stoops for this as for a take, and one
    // take in ten took longer.
    c.tick_loot(t0 + Duration::from_millis(2_500));
    assert!(c.loot_inflight.is_some(), "waited on as a take is");
    // The pile grows: landed.
    c.world.objects.get_mut(&PILE).unwrap().stack_size = 150;
    c.tick_loot(t0 + Duration::from_millis(2_600));
    assert!(c.loot_inflight.is_none());
    assert!(c.loot_merge.is_some(), "kept until the next take goes out");
    // A pour the server turns down -- too heavy, by its reckoning --
    // is over, and the refusal is left for the rules to read: it is
    // not a pack being full, so nothing is marked.
    on_the_body(&mut c, COIN, "Pyreal", 273, ac_world::item_type::MONEY, 50);
    let t1 = t0 + Duration::from_secs(1);
    c.take(COIN);
    c.tick_loot(t1);
    assert!(c.loot_merge.is_some());
    c.move_refused
        .insert(COIN, (0, t1 + Duration::from_millis(50)));
    c.take_refused(COIN, 0);
    assert!(
        c.packs_said_full.is_empty(),
        "a pour refused says nothing of the packs"
    );
    c.tick_loot(t1 + Duration::from_millis(100));
    assert!(c.loot_inflight.is_none(), "the pour is over");
    assert!(c.move_refused.contains_key(&COIN));
    // A stack too big for the pile is put in a slot, when there is
    // one, and is not a thing that needs no slot.
    c.world.objects.get_mut(&COIN).unwrap().stack_size = 25_000;
    assert_eq!(c.how_to_take(COIN), None, "no slot, and no pile it fits");
    let open = c.corpse_now(BODY, &[COIN], &profile, t0, t1);
    assert!(!open.items[0].needs_no_slot);
}
