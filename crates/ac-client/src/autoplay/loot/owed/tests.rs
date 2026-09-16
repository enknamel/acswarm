use super::*;
use crate::autoplay::loot::choose::LOOT_TIMEOUT;
use crate::autoplay::team::turns::{CLAIM_STALE, SAME_MOMENT};
use crate::autoplay::{LootAction, Mate, ShutFor};
use crate::testkit::{
    asks, character_of_level, corpse_at_hand, corpses_seen, game_data, looter, on_the_body,
    with_packs, BODY,
};

#[test]
fn a_body_where_a_kill_fell_is_ours_however_far() {
    let now = Instant::now();
    let spots = vec![(glam::Vec3::new(100.0, 100.0, 50.0), now)];
    // Where it fell, give or take the drift of dying.
    assert!(near_a_kill(glam::Vec3::new(103.0, 98.0, 51.0), &spots));
    // Somebody else's, a street away.
    assert!(!near_a_kill(glam::Vec3::new(120.0, 100.0, 50.0), &spots));
    assert!(!near_a_kill(glam::Vec3::new(100.0, 100.0, 50.0), &[]));
}

#[test]
fn the_fight_waits_only_on_bodies_the_looting_takes() {
    let now = Instant::now();
    let me = glam::Vec3::ZERO;
    let off = glam::Vec3::new(22.0, 0.0, 0.0);
    // Close by: looted, and waited on.
    assert!(corpse_is_ours(
        me,
        glam::Vec3::new(5.0, 0.0, 0.0),
        40.0,
        &[]
    ));
    // Twenty-two metres off where nothing of ours fell: neither. It
    // used to be waited on out to twenty-five and looted only to
    // twenty, and the character stood between the two for good.
    assert!(!corpse_is_ours(me, off, 40.0, &[]));
    // Where a kill of ours fell: both.
    assert!(corpse_is_ours(me, off, 40.0, &[(off, now)]));
    // But not past the fight radius.
    assert!(!corpse_is_ours(me, off, 20.0, &[(off, now)]));

    // Thirty metres off, where the summoned creature landed the last
    // blow: no kill notice came, so no kill spot, and it is not ours
    // as it lies. It is asked about instead.
    let pets = glam::Vec3::new(30.0, 0.0, 0.0);
    assert!(!corpse_is_ours(me, pets, 60.0, &[]));
    assert!(whose_to_ask(me, pets, 60.0, &[]));
    // Its description names the creature, so it is claimed where it
    // lies: looted, and waited on, and not asked about again.
    assert!(killed_by_us(
        "Killed by Blargerton's Mud Golem.",
        "Blargerton"
    ));
    let claimed = [(pets, now)];
    assert!(corpse_is_ours(me, pets, 60.0, &claimed));
    assert!(!whose_to_ask(me, pets, 60.0, &claimed));
    // One close by needs no asking, and one past the fight radius is
    // not asked about.
    assert!(!whose_to_ask(me, glam::Vec3::new(5.0, 0.0, 0.0), 60.0, &[]));
    assert!(!whose_to_ask(
        me,
        glam::Vec3::new(70.0, 0.0, 0.0),
        60.0,
        &[]
    ));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_moment_held_open_for_a_falling_body_ends_when_one_lands() {
    // The next fight is held for three seconds after a killing blow,
    // because the body comes a moment after the creature dies. Held
    // for the whole three regardless of what landed, nine characters
    // killing in one huddle each spent it standing over a body one
    // of the others was already opening: fighting was 13% of that
    // run and opening a corpse 53%.
    let mut c = character_of_level(game_data(), 20);
    let s = Duration::from_secs;
    let t0 = Instant::now();
    let spot = glam::Vec3::new(40.0, 40.0, 10.0);
    let body = |guid: u32, at: glam::Vec3| ac_world::WorldObject {
        guid,
        name: "Corpse of Drudge Slave".into(),
        object_desc_flags: ac_world::object_desc_flags::CORPSE,
        position: Some(ac_world::object::Position::new_flat(0, at)),
        ..Default::default()
    };
    assert!(!c.owes_a_corpse(), "owed a body with no kill behind it");

    // Its own killing blow: the body is owed while it is still
    // falling, and no longer.
    c.autoplay.last_kill = Some(t0);
    c.autoplay.kill_spots = vec![(spot, t0)];
    assert!(c.own_body_still_falling(t0));
    assert!(c.own_body_still_falling(t0 + CORPSE_APPEARS - s(1)));
    assert!(!c.own_body_still_falling(t0 + CORPSE_APPEARS));

    // A body noted before the blow is some other kill's and says
    // nothing about this one, wherever it lies.
    let (early, elsewhere, ours) = (0x8000_0001, 0x8000_0002, 0x8000_0003);
    c.world.objects.insert(early, body(early, spot));
    c.autoplay.corpse_seen = corpses_seen([(early, t0 - s(1))]);
    assert!(c.own_body_still_falling(t0));

    // Nor does a body that landed since the blow somewhere this
    // character killed nothing. Ending the hold on any corpse at
    // all, a huddle of nine had one come into view every couple of
    // seconds, and each one sent a character off after the next
    // fight leaving the body it had just made behind.
    let away = spot + glam::Vec3::new(KILL_SPOT * 2.0, 0.0, 0.0);
    c.world.objects.insert(elsewhere, body(elsewhere, away));
    c.autoplay.corpse_seen.mark(elsewhere, t0);
    assert!(
        c.own_body_still_falling(t0),
        "let go of its own body for somebody else's"
    );

    // Its own, landing where its kill fell, ends the wait: from here
    // the bodies on the floor are the answer, a mate's claim among
    // them.
    c.world.objects.insert(ours, body(ours, spot));
    c.autoplay.corpse_seen.mark(ours, t0);
    assert!(!c.own_body_still_falling(t0));
    assert!(!c.owes_a_corpse());
}

#[test]
fn a_far_corpse_is_asked_about_once_and_the_fight_waits_on_the_answer_briefly() {
    let t0 = Instant::now();
    let s = Duration::from_secs;
    let mut whose = Whose::default();
    // No creature seen out: nothing left bodies about.
    assert!(!whose.pet_lately(t0));
    whose.pet_out(t0);
    assert!(whose.pet_lately(t0 + s(30)));
    assert!(!whose.pet_lately(t0 + PET_KILLS_FOR));

    // Asked about: the fight waits a moment for the answer.
    assert!(!whose.waiting(t0));
    whose.ask(0x8000_0001, t0);
    assert!(whose.has_asked(0x8000_0001));
    assert!(whose.waiting(t0 + s(1)));
    // Asking again changes nothing, and does not restart the wait.
    whose.ask(0x8000_0001, t0 + s(2));
    assert_eq!(whose.out().collect::<Vec<_>>(), vec![0x8000_0001]);
    // An answer that never comes does not hold the fight for good.
    assert!(!whose.waiting(t0 + WHOSE_WAIT));

    // Once the answer is in, nothing is waited on or out, and the
    // corpse is still not asked about again.
    whose.ask(0x8000_0002, t0 + s(4));
    assert!(whose.waiting(t0 + s(5)));
    whose.answered(0x8000_0002);
    whose.answered(0x8000_0001);
    assert!(!whose.waiting(t0 + s(5)));
    assert_eq!(whose.out().count(), 0);
    assert!(whose.has_asked(0x8000_0002));

    // A corpse gone from view is forgotten.
    whose.tidy(|g| g == 0x8000_0002);
    assert!(!whose.has_asked(0x8000_0001));
    assert!(whose.has_asked(0x8000_0002));
}

#[test]
fn a_corpse_a_floor_below_is_not_close_however_near_it_lies_on_the_map() {
    // The Holtburg Dungeon's rooms are stacked: a body three metres
    // off on the map and eight metres down is in another room.
    let now = Instant::now();
    let me = glam::Vec3::new(0.0, 0.0, 8.0);
    let below = glam::Vec3::new(3.0, 0.0, 0.0);
    assert!(!corpse_is_ours(me, below, 60.0, &[]));
    // Nor one a floor above.
    assert!(!corpse_is_ours(below, me, 60.0, &[]));
    // On this floor it is, and a step or a doorsill up is still this
    // floor.
    assert!(corpse_is_ours(
        me,
        glam::Vec3::new(3.0, 0.0, 8.0),
        60.0,
        &[]
    ));
    assert!(corpse_is_ours(
        me,
        glam::Vec3::new(3.0, 0.0, 9.5),
        60.0,
        &[]
    ));
    // Where a kill of its own fell it is still its own: whether it
    // can be walked to is for the walk to find out.
    assert!(corpse_is_ours(me, below, 60.0, &[(below, now)]));
}

#[test]
fn a_corpse_is_ours_when_it_names_this_character_or_its_creature_as_the_killer() {
    assert!(killed_by_us("Killed by Blargerton.", "Blargerton"));
    assert!(killed_by_us(
        "Killed by Blargerton's Mud Golem.",
        "Blargerton"
    ));
    // The server leaves off a leading '+'; a line that kept it still
    // counts.
    assert!(killed_by_us("Killed by Fletch.", "+Fletch"));
    assert!(killed_by_us("Killed by Fletch's Mud Golem.", "+Fletch"));
    assert!(killed_by_us("Killed by +Fletch.", "+Fletch"));
    // A rare found on the body adds to the line.
    assert!(killed_by_us(
        "Killed by Blargerton. This corpse generated a rare item!",
        "Blargerton"
    ));
    // A name that only starts like ours is someone else.
    assert!(!killed_by_us("Killed by Blargertonia.", "Blargerton"));
    // Another player, or another player's creature, is not us.
    assert!(!killed_by_us("Killed by Grimble.", "Blargerton"));
    assert!(!killed_by_us(
        "Killed by Grimble's Mud Golem.",
        "Blargerton"
    ));
    assert!(!killed_by_us("Killed by misadventure.", "Blargerton"));
    // Nothing to go on is not ours.
    assert!(!killed_by_us("", "Blargerton"));
    assert!(!killed_by_us("Killed by .", ""));
}

#[test]
fn a_body_one_of_the_others_has_is_left_to_them() {
    // Nine characters in one huddle rank the bodies by the same
    // rule and so choose the same one: 1,386 opens for 41 bodies in
    // one run, because the server hands a container to one viewer
    // and refuses everyone else. Whoever has a body says so, and
    // the rest take another.
    let t0 = Instant::now();
    let (a, b) = (0x8000_0001, 0x8000_0002);
    let (here, there) = (glam::Vec3::ZERO, glam::Vec3::new(4.0, 0.0, 0.0));
    let me = 0x5000_0001;
    // Both fell a while back, so the tie-break has had its moment
    // and the claims are what speak.
    let mut ap = Autoplay {
        corpse_seen: corpses_seen([(a, t0), (b, t0)]),
        ..Default::default()
    };
    let now = t0 + CLAIM_SETTLE;

    // Alone on the board, nothing changes: both are ours.
    assert!(ap.ours_to_open(a, here, me, here, now));
    assert!(ap.ours_to_open(b, there, me, here, now));

    // One mate is at the first body, another at nothing.
    ap.team.mates = vec![
        looter(2, here, Some(a), Duration::ZERO),
        looter(3, here, None, Duration::ZERO),
    ];
    assert!(
        !ap.ours_to_open(a, here, me, here, now),
        "raced a mate for a body"
    );
    assert!(
        ap.ours_to_open(b, there, me, here, now),
        "left a free body lying"
    );

    // A claim goes stale. A mate that stalled or was dragged into a
    // fight over a body must not hold it for the five minutes it
    // lies there.
    ap.team.mates = vec![looter(2, here, Some(a), CLAIM_STALE)];
    assert!(
        ap.ours_to_open(a, here, me, here, now),
        "a stale claim still held"
    );
    ap.team.mates = vec![looter(
        2,
        here,
        Some(a),
        CLAIM_STALE - Duration::from_millis(1),
    )];
    assert!(!ap.ours_to_open(a, here, me, here, now));

    // A mate that died over the body it had open goes on saying so:
    // its client is still running. The same board row says its
    // health is nothing, and that is read rather than waited out.
    ap.team.mates = vec![Mate {
        health: 0.0,
        ..looter(2, here, Some(a), Duration::ZERO)
    }];
    assert!(
        ap.ours_to_open(a, here, me, here, now),
        "a dead mate held a body for the rest of the claim's life"
    );
}

#[test]
fn a_body_this_character_has_already_claimed_is_not_given_up_for_a_mates() {
    // Two characters that choose one body in the same tick each
    // hear the other's claim half a second later. Asking only
    // "does somebody claim it" made both stand off, each for the
    // other, and the walk each had started was never let go of --
    // so the claim kept going out, the body was opened by nobody,
    // and eight characters stood over it until every claim aged out
    // at once. A standing claim wins the tie instead of losing it.
    let t0 = Instant::now();
    let body = 0x8000_0001;
    let at = glam::Vec3::ZERO;
    let (me, lower, higher) = (0x5000_0005, 0x5000_0001, 0x5000_0009);
    let mut ap = Autoplay {
        corpse_seen: corpses_seen([(body, t0)]),
        ..Default::default()
    };
    ap.take_up_corpse(body, t0, LOOT_TIMEOUT);
    let now = t0 + CLAIM_SETTLE;

    // A mate that reached for it in the same moment and has the
    // higher guid gives way to us.
    ap.team.mates = vec![looter(higher, at, Some(body), CLAIM_SETTLE)];
    assert!(
        ap.ours_to_open(body, at, me, at, now),
        "gave up a body we were already holding"
    );

    // The lower guid in the same moment has it, and we let go.
    ap.team.mates = vec![looter(lower, at, Some(body), CLAIM_SETTLE)];
    assert!(!ap.ours_to_open(body, at, me, at, now));

    // Plainly older than ours, whatever the guid: theirs.
    ap.team.mates = vec![looter(
        higher,
        at,
        Some(body),
        CLAIM_SETTLE + SAME_MOMENT * 2,
    )];
    assert!(!ap.ours_to_open(body, at, me, at, now));

    // Plainly younger than ours, whatever the guid: ours.
    let later = t0 + SAME_MOMENT * 3;
    ap.team.mates = vec![looter(lower, at, Some(body), Duration::ZERO)];
    assert!(ap.ours_to_open(body, at, me, at, later));
}

#[test]
fn a_body_reached_with_a_full_pack_is_set_aside_and_owed_again_after_the_sale() {
    // A pack down to the slots kept for a counter's money: every body
    // in reach was walked to, shut on the spot as done with -- so
    // written off -- and the town run waited behind them. After the
    // sale none of them was gone back to, with minutes left on them.
    let t0 = Instant::now();
    let mut ap = Autoplay::default();
    let (me, at) = (glam::Vec3::ZERO, glam::Vec3::new(5.0, 0.0, 0.0));
    let body = 0x8000_8001;
    let full = Room {
        pack_low: true,
        ..Room::PLENTY
    };
    // No room, so no body is owed: none is walked to, and the fight is
    // not held for one.
    assert!(ap.corpse_owed(body, at, me, t0, Room::PLENTY));
    assert!(!ap.corpse_owed(body, at, me, t0, full), "owed with no room");
    // One already open when the pack filled is shut and set aside.
    ap.take_up_corpse(body, t0, LOOT_TIMEOUT);
    let mut open = corpse_at_hand(body, 1);
    open.slots_free = 3;
    open.room_anywhere = 3;
    open.keep_free = 3;
    let next = ap.loot_run.step(&open, t0);
    assert_eq!(next.act, Some(ac_loot::Act::Close), "{}", next.saying);
    ap.corpse_shut(
        body,
        &next.did,
        next.left_for_weight,
        ShutFor::default(),
        t0,
    );
    assert!(!ap.looted.contains(&body), "written off for good");
    // Sold down a few minutes later, it is owed again.
    let sold = t0 + Duration::from_secs(4 * 60);
    assert!(
        ap.corpse_owed(body, at, me, sold, Room::PLENTY),
        "never gone back to"
    );
}

#[test]
fn a_body_left_for_its_weight_waits_on_room_not_on_a_clock() {
    // Laden, a character shut every body with something heavy on it as
    // too laden and set it aside for half a minute. As each wait ran
    // out it went back, opened the body, took nothing and shut it
    // again, holding the next fight for it every time, for as long as
    // the body lay there.
    use crate::did::Did;
    let t0 = Instant::now();
    let s = Duration::from_secs;
    let mut ap = Autoplay::default();
    let (me, at) = (glam::Vec3::ZERO, glam::Vec3::new(5.0, 0.0, 0.0));
    let (body, rotted) = (0x8000_9001, 0x8000_9002);
    let mut open = corpse_at_hand(body, 1);
    open.items[0].burden = 300;
    open.carry_room = 40;
    ap.take_up_corpse(body, t0, LOOT_TIMEOUT);
    let next = ap.loot_run.step(&open, t0);
    assert_eq!(next.act, Some(ac_loot::Act::Close), "{}", next.saying);
    ap.corpse_shut(
        body,
        &next.did,
        next.left_for_weight,
        ShutFor::default(),
        t0,
    );
    assert!(!ap.looted.contains(&body), "written off for good");
    // Still forty short: not owed, however long it has waited.
    let laden = Room {
        carry: 40,
        ..Room::PLENTY
    };
    assert!(!ap.corpse_owed(body, at, me, t0 + s(31), laden));
    assert!(!ap.corpse_owed(body, at, me, t0 + s(4 * 60), laden));
    // It is what the character weighs its room against to count itself
    // laden, but only while the body is still lying there.
    ap.left_for_weight.insert(rotted, 20);
    assert_eq!(ap.lightest_left_for_weight(|g| g == body), Some(300));
    // Sold down: owed again at once.
    let sold = Room {
        carry: 4_000,
        ..Room::PLENTY
    };
    assert!(
        ap.corpse_owed(body, at, me, t0 + s(4 * 60), sold),
        "never gone back to"
    );
    // Emptied then, it is done with, and waits on nothing.
    ap.corpse_shut(body, &Did::Done, None, ShutFor::default(), t0 + s(4 * 60));
    assert_eq!(ap.lightest_left_for_weight(|g| g == body), None);
    // A body that has rotted is forgotten.
    ap.forget_corpses_gone(|g| g == body);
    assert!(ap.left_for_weight.is_empty());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn with_every_pack_full_the_body_is_set_aside_once_and_not_reopened() {
    let mut c = with_packs(game_data(), 2, 2, 3, 3);
    const DAGGER: u32 = 0x8000_0401;
    on_the_body(&mut c, DAGGER, "Dagger", 21, 0, 1);
    let t0 = Instant::now();
    assert_eq!(c.free_space(), 0);
    let room = c.room_for_loot();
    assert!(room.pack_low, "no pack has a slot");
    // The body at its feet is not waited on, and the rules shut one
    // already open as full, setting it aside rather than writing
    // it off.
    assert!(!c.autoplay.corpse_waiting(BODY, t0, room));
    let profile = crate::profile::Profile {
        rules: vec![asks("daggers", "dagger", LootAction::Sell)],
        ..Default::default()
    };
    c.autoplay.take_up_corpse(BODY, t0, LOOT_TIMEOUT);
    let open = c.corpse_now(BODY, &[DAGGER], &profile, t0, t0);
    assert_eq!(open.slots_free, 0);
    let next = c.autoplay.loot_run.step(&open, t0);
    assert_eq!(next.act, Some(ac_loot::Act::Close), "{}", next.saying);
    assert!(matches!(&next.did, crate::did::Did::Blocked(b) if b.what == "the pack is full"));
    c.autoplay.corpse_shut(
        BODY,
        &next.did,
        next.left_for_weight,
        ShutFor::default(),
        t0,
    );
    assert!(!c.autoplay.looted.contains(&BODY), "written off for good");
    // Set aside for room: with none, it is not gone back to however
    // long it lies there.
    let later = t0 + Duration::from_secs(10 * 60);
    assert!(!c.autoplay.corpse_waiting(BODY, later, c.room_for_loot()));
    // Room appears -- a dagger sold -- and it waits again.
    c.world.objects.remove(&0x8000_0500);
    let room = c.room_for_loot();
    assert!(!room.pack_low);
    assert!(c.autoplay.corpse_waiting(BODY, later, room));
}
