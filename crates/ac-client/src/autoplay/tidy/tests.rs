use super::*;
use crate::testkit::{character_of_level, game_data};

#[test]
fn a_profile_with_tidy_pack_off_is_never_tidied() {
    assert_eq!(
        why_not_tidy(TidyGate {
            tidy_pack_off: true,
            ..TidyGate::default()
        }),
        Some("this profile leaves the pack as it is")
    );
}

#[test]
fn nothing_is_poured_with_a_counter_open() {
    // A sale holds what it has sent the vendor by guid, and a pour
    // makes one of those vanish out from under it.
    assert_eq!(
        why_not_tidy(TidyGate {
            counter_open: true,
            ..TidyGate::default()
        }),
        Some("a counter is open")
    );
}

#[test]
fn nor_while_the_quartermaster_is_loaded_or_unloaded() {
    // Money counted out into its own stack was poured straight back
    // into the pile it came from, a hundred and twenty six times.
    assert_eq!(
        why_not_tidy(TidyGate {
            quartermaster: true,
            ..TidyGate::default()
        }),
        Some("the quartermaster is being loaded or unloaded")
    );
}

#[test]
fn nor_while_ammunition_is_being_made() {
    assert_eq!(
        why_not_tidy(TidyGate {
            crafting: true,
            ..TidyGate::default()
        }),
        Some("ammunition is being made")
    );
}

#[test]
fn nor_just_after_a_hand_over_to_a_teammate() {
    assert_eq!(
        why_not_tidy(TidyGate {
            gave_lately: true,
            ..TidyGate::default()
        }),
        Some("something was just handed to a teammate")
    );
}

#[test]
fn nor_while_a_take_is_queued_or_in_the_air() {
    assert_eq!(
        why_not_tidy(TidyGate {
            take_in_air: true,
            ..TidyGate::default()
        }),
        Some("a take is queued or in the air")
    );
}

#[test]
fn with_nothing_in_the_way_the_pack_is_tidied() {
    assert_eq!(why_not_tidy(TidyGate::default()), None);
    // The first reason that applies is the one given.
    assert_eq!(
        why_not_tidy(TidyGate {
            counter_open: true,
            take_in_air: true,
            ..TidyGate::default()
        }),
        Some("a counter is open")
    );
}

#[test]
fn every_errand_holding_a_stack_is_offered_up() {
    // Whatever another part of the rules is holding across ticks
    // must not be poured away under it.
    let mut ap = Autoplay::default();
    assert_eq!(ap.held_by_an_errand().iter().flatten().count(), 0);
    ap.pending_wield = Some(1);
    ap.wanted_ammo = Some(2);
    ap.put_down = Some(3);
    ap.crafting = Some((4, 5, Instant::now()));
    ap.handing = Some((6, Instant::now()));
    let held: Vec<u32> = ap.held_by_an_errand().iter().flatten().copied().collect();
    assert_eq!(held, vec![1, 2, 3, 4, 5, 6]);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn tidying_the_pack_never_pours_one_salvage_bag_into_another() {
    // Two bags of Iron share a wcid, but only an Ust puts bags
    // together. The server sends a bag with no stack size, which
    // leaves it at 1, and both pours -- the tidy chore and the one
    // at a vendor's counter -- read that to decide what stacks.
    let mut c = character_of_level(game_data(), 20);
    let me = c.world.player_guid;
    let bag = |guid: u32, workmanship: f32| ac_world::WorldObject {
        guid,
        weenie_class_id: 20986,
        name: "Salvaged Iron".into(),
        item_type: ac_world::item_type::TINKERING_MATERIAL,
        material: 0x3D,
        workmanship,
        structure: 50,
        max_structure: 100,
        stack_size: 1,
        max_stack_size: 1,
        container: me,
        ..Default::default()
    };
    let (tens, sixes) = (0x8000_0201, 0x8000_0202);
    c.world.objects.insert(tens, bag(tens, 10.0));
    c.world.objects.insert(sixes, bag(sixes, 6.0));
    // A pair that does pour, so the bags are not passed over only
    // because nothing is looked at.
    let arrows = |guid: u32, count: u32| ac_world::WorldObject {
        guid,
        weenie_class_id: 300,
        name: "Arrow".into(),
        stack_size: count,
        max_stack_size: 250,
        container: me,
        ..Default::default()
    };
    let (few, many) = (0x8000_0203, 0x8000_0204);
    c.world.objects.insert(few, arrows(few, 5));
    c.world.objects.insert(many, arrows(many, 40));
    let m = crate::pack::next_merge(&c.pack_stacks()).expect("the arrows pour");
    assert_eq!((m.from, m.to), (few, many));
    c.world.objects.remove(&few);
    assert_eq!(crate::pack::next_merge(&c.pack_stacks()), None);
    let counter = c.vendor_snapshot(&crate::growth::Growth::default());
    let bags: Vec<_> = counter
        .items
        .iter()
        .filter(|i| i.guid == tens || i.guid == sixes)
        .collect();
    assert_eq!(bags.len(), 2);
    assert!(bags.iter().all(|i| i.max_stack <= 1), "{bags:?}");
}

/// The character whose pack these tests are about.
const TIDIER: u32 = 0x5000_0001;

/// A character carrying `stacks` of `(guid, wcid, count, max)`,
/// offline, over the game's archives.
fn carrying(stacks: &[(u32, u32, u32, u32)]) -> Client {
    let mut c = Client::offline(crate::testkit::game_data());
    c.world.player_guid = Some(TIDIER);
    for &(guid, wcid, count, max) in stacks {
        c.world.objects.insert(
            guid,
            ac_world::WorldObject {
                guid,
                weenie_class_id: wcid,
                name: format!("thing {wcid}"),
                stack_size: count,
                max_stack_size: max,
                container: Some(TIDIER),
                parent: Some(TIDIER),
                ..Default::default()
            },
        );
    }
    c
}

/// The server's answer to a whole pour: the source forgotten, the
/// target's new size.
fn poured(c: &mut Client, from: u32, to: u32, now: u32) {
    c.world.objects.remove(&from);
    if let Some(o) = c.world.objects.get_mut(&to) {
        o.stack_size = now;
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn lead_peas_taken_in_two_stacks_are_poured_together_without_a_tick_of_their_own() {
    // The report: peas looted off one corpse after another sit in
    // stacks of their own. Nothing here waits for a quiet moment --
    // there is a fight on -- because the server makes a pack-to-pack
    // merge on the spot.
    let mut c = carrying(&[(1, 8329, 40, 100), (2, 8329, 5, 100)]);
    c.attack_target = Some(0x8000_30F2);
    let t0 = Instant::now();
    c.autoplay_tidy(t0);
    let sent = c.autoplay.pour.clone().expect("a pour went out").0;
    assert_eq!(
        (sent.merge.from, sent.merge.to, sent.merge.amount),
        (2, 1, 5)
    );
    assert_eq!(sent.to_before, 40);
    // Nothing else is asked for while the server has not answered:
    // the counts a second choice would be made from are stale.
    c.autoplay_tidy(t0 + Duration::from_millis(100));
    assert_eq!(
        c.autoplay.pour.as_ref().map(|(p, _)| p.merge.clone()),
        Some(sent.merge.clone())
    );
    // The answer, read off the target's new size.
    poured(&mut c, 2, 1, 45);
    c.autoplay_tidy(t0 + Duration::from_millis(700));
    assert!(c.autoplay.pour.is_none(), "the pour landed");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_pair_the_server_turns_down_does_not_stop_the_rest_being_tidied() {
    // One stubborn pair used to be the only answer ever offered, so
    // it was asked for every 600 ms and nothing else in the pack was
    // ever poured together.
    let mut c = carrying(&[
        (1, 273, 5000, 25000),
        (2, 273, 900, 25000),
        (3, 8329, 40, 100),
        (4, 8329, 5, 100),
    ]);
    let t0 = Instant::now();
    c.autoplay_tidy(t0);
    let first = c.autoplay.pour.clone().expect("a pour went out").0.merge;
    assert_eq!((first.from, first.to), (2, 1));
    // InventoryServerSaveFailed naming the source, with no reason
    // at all, which is half of them.
    c.move_refused
        .insert(2, (0, t0 + Duration::from_millis(50)));
    c.autoplay_tidy(t0 + Duration::from_millis(700));
    assert!(
        !c.move_refused.contains_key(&2),
        "the refusal was this pour's and is spent"
    );
    let next = c.autoplay.pour.clone().expect("the next pair").0.merge;
    assert_eq!((next.from, next.to), (4, 3), "the peas go in instead");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_refusal_naming_the_target_is_read_as_this_pours_answer() {
    // A stack that is stuck, or being traded, is refused under the
    // target's guid. Read only under the source's, these were never
    // seen at all and the same pair was offered every 600 ms.
    let mut c = carrying(&[(1, 8329, 40, 100), (2, 8329, 5, 100)]);
    let t0 = Instant::now();
    c.autoplay_tidy(t0);
    c.move_refused
        .insert(1, (0x29, t0 + Duration::from_millis(50)));
    c.autoplay_tidy(t0 + Duration::from_millis(700));
    assert!(c.autoplay.pour.is_none(), "the pour is over");
    assert!(!c.move_refused.contains_key(&1));
    // And the pair is left alone for a while rather than asked again.
    assert!(c
        .autoplay
        .growth
        .wont_merge
        .held(&(2, 1), t0 + Duration::from_millis(800)));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn stacks_whose_words_disagree_are_never_poured_together() {
    use ac_loot::LootAction;
    let stats = |guid: u32| crate::items::ItemStats {
        guid,
        wcid: 8329,
        name: "Lead Pea".into(),
        ..Default::default()
    };
    // The big stack is meant for a counter, the small one is kept.
    // Pouring the small one in would settle the survivor as kept
    // (the ledger takes the cautious answer), and forty peas the
    // player said to sell would stay in the pack for good: the
    // player's Sell overruled by a tidy. So nothing is poured.
    let mut c = carrying(&[(1, 8329, 40, 100), (2, 8329, 5, 100)]);
    c.autoplay.ledger.remember(&stats(1), LootAction::Sell);
    c.autoplay.ledger.remember(&stats(2), LootAction::Keep);
    let t0 = Instant::now();
    c.autoplay_tidy(t0);
    assert!(
        c.autoplay.pour.is_none(),
        "poured across words: {:?}",
        c.autoplay.pour
    );
    assert_eq!(c.autoplay.ledger.by_guid(1), Some(LootAction::Sell));

    // Nor into a stack nothing was decided about. "Undecided" is
    // not "sell": the guards answer for the survivor, and a Sell
    // poured into it is a Sell lost.
    let mut c = carrying(&[(1, 8329, 40, 100), (2, 8329, 5, 100)]);
    c.autoplay.ledger.remember(&stats(2), LootAction::Sell);
    c.autoplay_tidy(t0);
    assert!(c.autoplay.pour.is_none(), "{:?}", c.autoplay.pour);

    // Kept and undecided both stay in the pack, so they pour: a stack
    // of tapers bought (kept) and one carried from before (no word)
    // were left as two part-stacks for good.
    let mut c = carrying(&[(1, 8329, 40, 100), (2, 8329, 5, 100)]);
    c.autoplay.ledger.remember(&stats(1), LootAction::Keep);
    c.autoplay_tidy(t0);
    assert!(
        c.autoplay.pour.is_some(),
        "a kept and an undecided stack were not poured"
    );

    // Two stacks with one word are poured, and the word survives
    // the pour.
    let mut c = carrying(&[(1, 8329, 40, 100), (2, 8329, 5, 100)]);
    c.autoplay.ledger.remember(&stats(1), LootAction::Sell);
    c.autoplay.ledger.remember(&stats(2), LootAction::Sell);
    c.autoplay_tidy(t0);
    let sent = c.autoplay.pour.clone().expect("a pour went out").0;
    assert_eq!((sent.merge.from, sent.merge.to), (2, 1));
    poured(&mut c, 2, 1, 45);
    c.autoplay_tidy(t0 + Duration::from_millis(700));
    assert_eq!(c.autoplay.ledger.by_guid(1), Some(LootAction::Sell));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_pour_with_no_word_at_all_is_given_up_on_and_the_pack_read_afresh() {
    let mut c = carrying(&[(1, 8329, 40, 100), (2, 8329, 5, 100)]);
    let t0 = Instant::now();
    c.autoplay_tidy(t0);
    assert!(c.autoplay.pour.is_some());
    // Nothing comes back: no refusal, no new counts.
    c.autoplay_tidy(t0 + crate::pack::POUR_LOST);
    assert!(c.autoplay.pour.is_none(), "given up as lost");
}
