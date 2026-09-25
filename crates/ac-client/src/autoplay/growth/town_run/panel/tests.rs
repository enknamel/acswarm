use super::super::tests::nobody_near_buys_peas;
use super::*;
use crate::autoplay::growth::tests::{a_run_at_cindrue, run_to};
use crate::testkit::{
    a_caster_knowing, a_caster_with_a_buff_due, a_counter, coin_in_the_pack, pea_in_the_pack,
    shop_named, standing_at, vendor_beside, window_of, with_a_buy_list, with_a_pack,
};

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_run_asked_for_from_the_panel_chooses_a_counter_and_sets_off() {
    // The panel's Step and Run once put the shopping rules straight
    // to the character where it stood. With no window open the
    // rules answered "no counter" and called the trip done, so the
    // buttons did nothing -- and having called it done, went on
    // doing nothing once a window was opened. Now they start the
    // run autoplay would start: a counter is chosen and the walk
    // to it begins, with autoplay off and none of its throttles.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let now = Instant::now();
    assert!(!c.autoplay.config.enabled);
    assert_eq!(c.town_run_driver(), None);
    c.town_run_by_hand(now).expect("no run was started");
    assert_eq!(c.town_run_driver(), Some(Driver::Hand));
    assert!(c.traveling(), "chose a counter and did not set off");
    let v = c.town_run_view(now).expect("no run to show");
    assert_eq!(v.phase, "Going");
    assert_eq!(v.driver, Driver::Hand);
    assert_eq!(v.next, None, "nothing is decided on the walk");
    // The walk is a wait, and a Step waits it out.
    assert_eq!(c.town_run_step_by_hand(now), Turn::Waited);
    assert!(c.traveling());
    // Pressed again, the run under way is the one shown; a second
    // is not started.
    c.town_run_by_hand(now).unwrap();
    assert_eq!(c.town_run_view(now).unwrap().vendor, v.vendor);
    assert_eq!(c.autoplay.growth.run.as_ref().unwrap().since, now);

    // Autoplay turned on meanwhile leaves the run to the panel --
    // it neither steps it nor, with town runs off, lets it go --
    // but counts the character as busy with it.
    c.autoplay.config.enabled = true;
    c.autoplay.config.growth.town_runs = false;
    assert_eq!(c.town_run_driver(), Some(Driver::Hand));
    c.autoplay_town_run(now);
    assert!(
        c.autoplay.growth.town_run_under_way(),
        "let go for town runs being off"
    );
    c.autoplay.config.growth.town_runs = true;
    assert!(c.autoplay_town_run(now), "the run did not keep the tick");
    assert_eq!(c.town_run_driver(), Some(Driver::Hand));
    assert!(c.traveling(), "autoplay stepped the panel's run");

    // Stop leaves the character as it stands: no run, no walk,
    // nothing to walk up to, no window.
    c.town_run_stop(now);
    assert_eq!(c.town_run_driver(), None);
    assert!(!c.traveling());
    assert!(c.follow.is_none());
    assert!(c.world.open_vendor.is_none());
    assert_eq!(c.autoplay.growth.shop.phase, ac_vendor::Phase::Walking);
    // And autoplay, now running town runs, does not set straight
    // off on one of its own.
    assert!(!c.autoplay_town_run(now), "autoplay started a run at once");
    assert_eq!(c.town_run_driver(), None);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_run_started_under_autoplay_is_autoplays_to_step() {
    // With autoplay on and running town runs, Run from the panel
    // starts the run now, and autoplay's tick takes it from there:
    // the panel shows it and does not step it too.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    c.autoplay.config.enabled = true;
    let now = Instant::now();
    c.town_run_by_hand(now).expect("no run was started");
    assert_eq!(c.town_run_driver(), Some(Driver::Autoplay));
    assert!(c.autoplay_town_run(now));
    assert!(c.traveling());
    // Until the panel steps it itself, which makes it the panel's.
    assert_eq!(c.town_run_step_by_hand(now), Turn::Waited);
    assert_eq!(c.town_run_driver(), Some(Driver::Hand));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_hand_run_left_held_is_autoplays_again_once_it_runs_town_runs() {
    // Step once with autoplay off, close the panel, turn autoplay
    // on: the run was the panel's for good, autoplay claimed every
    // tick for it and stepped nothing, and the character stood on
    // the road with a blank status line. A run nothing has stepped
    // for a moment is autoplay's to carry on, when it runs town
    // runs.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let now = Instant::now();
    c.town_run_by_hand(now).expect("no run was started");
    assert_eq!(c.town_run_driver(), Some(Driver::Hand));
    c.autoplay.config.enabled = true;
    c.autoplay.config.growth.town_runs = true;
    // Just pressed: still the panel's, and not stepped by autoplay.
    assert!(c.autoplay_town_run(now));
    assert_eq!(c.town_run_driver(), Some(Driver::Hand));
    // Left alone: autoplay's, and stepped on.
    let later = now + HAND_HOLD;
    assert!(c.autoplay_town_run(later));
    assert_eq!(c.town_run_driver(), Some(Driver::Autoplay));
    assert!(c.autoplay.growth.town_run_under_way());
    assert!(c.traveling());
    // The panel stepping it again takes it back, for as long as it
    // keeps stepping.
    assert_eq!(c.town_run_step_by_hand(later), Turn::Waited);
    assert_eq!(c.town_run_driver(), Some(Driver::Hand));
    assert!(c.autoplay_town_run(later));
    assert_eq!(c.town_run_driver(), Some(Driver::Hand));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_hand_run_with_town_runs_off_keeps_the_hunting_off_the_counter() {
    // Autoplay on with town runs off is the panel's to drive, and
    // the run is kept for it -- but with town runs off the run was
    // never looked at by autoplay's tick, so nothing claimed the
    // tick for it and the hunting, finding nothing in sight, walked
    // the character off the open window to look about the ground.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    c.autoplay.config.enabled = true;
    c.autoplay.config.growth.town_runs = false;
    c.autoplay.config.growth.hunt_grounds = true;
    c.autoplay.config.growth.tactic = ac_world::hunting::Tactic::Patrol;
    // On its hunting ground, quiet long enough to move on.
    c.autoplay.growth.hunting_at = Some(holtburg >> 16);
    c.autoplay.growth.quiet_since = Some(now - Duration::from_secs(600));
    // At the counter, waiting on its window, held by the panel.
    let mut run = run_to(Vec2::new(me.x, me.y), now);
    run.phase = Phase::Opening {
        guid: 0x8000_0001,
        tries: 1,
        busy: None,
        asked_over: 0,
    };
    c.autoplay.growth.run = Some(run);
    c.autoplay.growth.by_hand = true;
    assert!(c.autoplay_town_run(now), "the run did not keep the tick");
    assert!(!c.traveling(), "the hunting walked off the counter");
    assert_eq!(c.autoplay.growth.bound, None);
    assert!(c.autoplay.growth.town_run_under_way());
    assert!(
        c.autoplay.status.contains("held by the vendoring panel"),
        "nothing said why the character stands still: {:?}",
        c.autoplay.status
    );
    // Without the run the next tick goes looking about the ground,
    // which is what the run was keeping it from.
    c.autoplay.growth.by_hand = false;
    assert!(c.autoplay_grow(now + Duration::from_millis(16)));
    assert!(
        !c.autoplay.growth.town_run_under_way(),
        "kept with town runs off"
    );
    assert!(c.traveling(), "the hunting did not move");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn run_pressed_at_a_counter_the_player_opened_sells_there() {
    // The panel shows what the rules would do at a window the
    // player opened by hand; Run then chose a counter of its own
    // and walked away from the one it had just been showing. At an
    // open window within reach the run is made there, and its
    // first turn is the appraising.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let now = Instant::now();
    let rakk = 0x8000_0002;
    vendor_beside(
        &mut c,
        rakk,
        "Rakk the Peddler",
        glam::Vec3::new(1.0, 0.0, 0.0),
    );
    c.world.open_vendor = Some(window_of(rakk));
    c.town_run_by_hand(now).expect("no run was started");
    let v = c.town_run_view(now).expect("no run to show");
    assert_eq!(v.vendor, "Rakk the Peddler");
    assert_eq!(v.phase, "Opening");
    assert!(!c.traveling(), "walked off to another counter");
    assert_eq!(c.town_run_step_by_hand(now), Turn::Acted);
    assert_eq!(c.town_run_view(now).unwrap().phase, "Appraising");
    assert!(c.world.open_vendor.is_some(), "the window was closed");

    // A window left open from across the town is not a counter at
    // hand: the run chooses one and sets off, as it always did.
    c.town_run_stop(now);
    vendor_beside(
        &mut c,
        rakk,
        "Rakk the Peddler",
        glam::Vec3::new(60.0, 0.0, 0.0),
    );
    c.world.open_vendor = Some(window_of(rakk));
    c.town_run_by_hand(now).expect("no run was started");
    assert_eq!(c.town_run_view(now).unwrap().phase, "Going");
    assert!(c.traveling());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_hand_run_is_refused_with_no_slot_for_the_money() {
    // Autoplay's own refusal lived only in its tick, so the panel's
    // Run walked a pack with no free slot to town, sold nothing
    // there, and came home futile -- which held autoplay's next run
    // back for the longer while.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    let me = 0x5000_0001;
    c.world.player_guid = Some(me);
    c.world.objects.insert(
        me,
        ac_world::WorldObject {
            guid: me,
            name: "Verity".into(),
            is_player: true,
            items_capacity: 1,
            ..Default::default()
        },
    );
    c.world.objects.insert(
        0x8000_0003,
        ac_world::WorldObject {
            guid: 0x8000_0003,
            name: "Dagger".into(),
            value: 50,
            container: Some(me),
            ..Default::default()
        },
    );
    assert_eq!(c.free_space(), 0);
    let now = Instant::now();
    assert_eq!(
        c.town_run_by_hand(now),
        Err("no free slot for a counter's money".into())
    );
    assert_eq!(c.town_run_driver(), None);
    assert!(!c.traveling());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_window_that_opens_after_the_run_stopped_is_closed() {
    // Stop pressed while the counter was being asked for its window
    // left the window to arrive afterwards, and nothing closed it:
    // the tidying refused every pour for "a counter is open" and
    // the panel showed a counter open with nobody at it.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.player_guid = Some(0x5000_0001);
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let rakk = 0x8000_0002;
    let asking = |c: &mut Client| {
        let mut run = run_to(Vec2::new(me.x, me.y), now);
        run.phase = Phase::Opening {
            guid: rakk,
            tries: 1,
            busy: None,
            asked_over: 0,
        };
        c.autoplay.growth.run = Some(run);
    };
    asking(&mut c);
    c.town_run_stop(now);
    assert!(c.world.open_vendor.is_none());
    // The window comes a moment later, and goes.
    c.world.open_vendor = Some(window_of(rakk));
    c.autoplay_close_unwanted_window(now + Duration::from_secs(1));
    assert!(c.world.open_vendor.is_none(), "the window was left open");
    // Another counter's window is not the one that was asked for.
    asking(&mut c);
    c.town_run_stop(now);
    c.world.open_vendor = Some(window_of(0x8000_0009));
    c.autoplay_close_unwanted_window(now + Duration::from_secs(1));
    assert!(
        c.world.open_vendor.is_some(),
        "somebody else's window was closed"
    );
    c.world.open_vendor = None;
    // Long after the run would have given up on it, a window for
    // that counter is the player's own doing.
    asking(&mut c);
    c.town_run_stop(now);
    let late = now + VENDOR_OPEN_TIMEOUT + Duration::from_secs(1);
    c.autoplay_close_unwanted_window(late);
    c.world.open_vendor = Some(window_of(rakk));
    c.autoplay_close_unwanted_window(late);
    assert!(
        c.world.open_vendor.is_some(),
        "the player's window was closed"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_hand_runs_status_stays_with_autoplay_off() {
    // With autoplay off its tick cleared the status every frame,
    // and the run's next turn said it again: a log line, an event
    // and a bus post a frame for the length of the walk. The run
    // the panel is driving keeps its line.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.player_guid = Some(0x5000_0001);
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    c.autoplay.config.enabled = false;
    c.autoplay.growth.run = Some(run_to(Vec2::new(me.x, me.y), now));
    c.autoplay.growth.by_hand = true;
    c.autoplay.say(Doing::Shopping, "going to the counter");
    let said = c.autoplay.announced.len();
    c.tick_autoplay(now);
    assert_eq!(
        c.autoplay.status, "going to the counter",
        "the status was cleared"
    );
    c.autoplay.say(Doing::Shopping, "going to the counter");
    assert_eq!(c.autoplay.announced.len(), said, "said again");
    // A run nobody is driving does not keep autoplay's line up.
    c.autoplay.growth.by_hand = false;
    c.tick_autoplay(now);
    assert!(c.autoplay.status.is_empty());
}

/// Something in the pack by name, `stack` of it, kept.
fn thing_in_the_pack(c: &mut Client, guid: u32, name: &str, stack: u32) {
    let me = c.world.player_guid.unwrap();
    c.world.objects.insert(
        guid,
        ac_world::WorldObject {
            guid,
            name: name.into(),
            value: 1,
            stack_size: stack,
            max_stack_size: 1_000,
            container: Some(me),
            ..Default::default()
        },
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_panel_run_with_a_list_and_a_pea_goes_to_the_counter_with_the_list() {
    // The archer has 180 of 250 arrows -- short, not urgent -- and
    // a pea. The panel's button, made a run to sell whenever the
    // pack held anything for a counter, went to the archmage, sold
    // the pea and ended in town with the arrows unbought. It goes
    // where the list is, and the pea to the archmage after.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 50);
    pea_in_the_pack(&mut c, 0x8000_0010, "Lead Pea", 8329, 500);
    thing_in_the_pack(&mut c, 0x8000_0020, "Bundle of Arrowshafts", 18);
    coin_in_the_pack(&mut c, 0x8000_0030, 1_000);
    with_a_buy_list(&mut c, &[("Bundle of Arrowshafts", 25, 6)]);
    let cfg = c.autoplay.config.growth.clone();
    let now = Instant::now();
    let needs = c.needs_now(now, &cfg);
    assert!(
        needs
            .iter()
            .any(|n| n.name == "Bundle of Arrowshafts" && !n.urgent && n.want == 7),
        "{needs:?}"
    );
    c.town_run_by_hand(now).expect("no run");
    let run = c.autoplay.growth.run.take().expect("no run");
    assert_eq!(run.errand, Errand::Buy);
    assert!(
        shop_named(&run.vendor)
            .stocks("Bundle of Arrowshafts")
            .is_some(),
        "{}",
        run.vendor
    );
    assert_ne!(run.vendor, "Archmage Cindrue");
    c.cancel_travel();
    assert!(
        c.grow_run_next(run, now, &cfg, None),
        "went home with the pea"
    );
    let on = c.autoplay.growth.run.as_ref().unwrap();
    assert_eq!(on.vendor, "Archmage Cindrue");
    assert_eq!(on.errand, Errand::Sell);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_panel_run_starts_even_when_nobody_buys_the_pea() {
    // Nothing on the list, a pea in the pack, and no counter anywhere
    // that takes it: the button still starts a run, to the nearest
    // counter, as it always did.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    with_a_pack(&mut c, 50);
    pea_in_the_pack(&mut c, 0x8000_0010, "Lead Pea", 8329, 500);
    with_a_buy_list(&mut c, &[]);
    let now = Instant::now();
    assert!(nobody_near_buys_peas(&mut c, f32::INFINITY, now) > 0);
    c.town_run_by_hand(now).expect("no run");
    let run = c.autoplay.growth.run.as_ref().expect("no run");
    assert_eq!(run.errand, Errand::Buy);
    assert_ne!(run.vendor, "Archmage Cindrue");
}

/// The Opening phase as a run enters it.
fn opening(guid: u32) -> Phase {
    Phase::Opening {
        guid,
        tries: 1,
        busy: None,
        asked_over: 0,
    }
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn buffs_wait_at_the_counter_and_go_up_on_the_walk_and_after() {
    // +Vesperi arrived at Archmage Cindrue with eight protections
    // lapsing and cast them one after another from the counter; the
    // buff pass held for a journey and for a fight, and standing at
    // a counter was neither.
    let now = Instant::now();
    let (mut c, _) = a_caster_with_a_buff_due(now);
    let me = c.player.as_ref().unwrap().world_position();
    let cindrue = 0x7a9b_4033;
    a_counter(
        &mut c,
        cindrue,
        "Archmage Cindrue",
        glam::Vec3::new(84.0, 7.1, 94.0),
    );
    // On the walk to town the urgent pass casts as it always did.
    let mut run = run_to(Vec2::new(me.x, me.y), now);
    run.vendor = "Archmage Cindrue".into();
    c.autoplay.growth.run = Some(run.clone());
    let sent = c.session.actions_sent();
    assert!(c.autoplay_buff(now, true), "no cast on the walk");
    assert!(c.session.actions_sent() > sent, "the cast did not go out");
    assert!(!c.autoplay.buffs_held_at_counter);
    // At the counter, from the Use going out, neither pass casts.
    let at_counter = |phase: Phase| {
        let mut run = run.clone();
        run.phase = phase;
        run
    };
    let mut then = now;
    for phase in [
        Phase::Opening {
            guid: cindrue,
            tries: 1,
            busy: None,
            asked_over: 0,
        },
        Phase::Appraising,
        Phase::Selling { sent: Vec::new() },
    ] {
        then += Duration::from_secs(2);
        c.autoplay.growth.run = Some(at_counter(phase.clone()));
        // The window is open once the counter has answered the Use.
        c.world.open_vendor = (!matches!(phase, Phase::Opening { .. })).then(|| window_of(cindrue));
        c.autoplay.cast_sent = None;
        let sent = c.session.actions_sent();
        assert!(!c.autoplay_buff(then, true), "an urgent cast at {phase:?}");
        assert!(!c.autoplay_buff(then, false), "a cast at {phase:?}");
        assert_eq!(
            c.session.actions_sent(),
            sent,
            "something went out at {phase:?}"
        );
        assert!(c.autoplay.buffs_held_at_counter, "the hold was not said");
    }
    // A window the player opened by hand holds the pass the same way.
    c.autoplay.growth.run = None;
    c.world.open_vendor = Some(window_of(cindrue));
    then += Duration::from_secs(2);
    assert!(!c.autoplay_buff(then, true), "a cast into an open window");
    // The window closed, the buff goes up.
    c.world.open_vendor = None;
    then += Duration::from_secs(2);
    let sent = c.session.actions_sent();
    assert!(
        c.autoplay_buff(then, true),
        "no cast once the window closed"
    );
    assert!(c.session.actions_sent() > sent, "the cast did not go out");
    assert!(
        !c.autoplay.buffs_held_at_counter,
        "the hold outlived the counter"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn urgent_buffs_go_up_in_a_fight_at_the_counter() {
    // A wandering monster catches the caster at the counter. The
    // fight outranks the run, which stays in its counter phase
    // throughout, and a hold that read the phase alone kept every
    // protection down for a window nobody was trading at until
    // the fight was over.
    let now = Instant::now();
    let (mut c, _) = a_caster_with_a_buff_due(now);
    let cindrue = 0x7a9b_4033;
    a_run_at_cindrue(&mut c, opening(cindrue), now);
    assert!(
        !c.autoplay_buff(now, true),
        "a cast at the counter with no fight on"
    );
    // Something is hitting the character.
    c.autoplay.last_hit_us = Some(Instant::now());
    let sent = c.session.actions_sent();
    assert!(
        c.autoplay_buff(now, true),
        "the urgent buff waited for the counter through the fight"
    );
    assert!(c.session.actions_sent() > sent, "the cast did not go out");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_window_left_open_from_across_the_town_holds_no_buff() {
    // Nothing closes a window but this client, since the server
    // keeps none. One the player opened and walked away from read
    // as a counter at hand for the rest of the session, and no
    // buff went up again.
    let now = Instant::now();
    let (mut c, _) = a_caster_with_a_buff_due(now);
    let cindrue = 0x7a9b_4033;
    // The counter is a hundred metres off.
    a_counter(
        &mut c,
        cindrue,
        "Archmage Cindrue",
        glam::Vec3::new(184.0, 7.1, 94.0),
    );
    c.world.open_vendor = Some(window_of(cindrue));
    assert_eq!(c.counter_at_hand(), None);
    let sent = c.session.actions_sent();
    assert!(
        c.autoplay_buff(now, true),
        "held for a window left open across the town"
    );
    assert!(c.session.actions_sent() > sent, "the cast did not go out");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn town_runs_switched_off_at_the_counter_close_its_window() {
    // A run let go for town runs being switched off left its
    // window open, and with town runs off no later run would ever
    // close it: a counter at hand for the rest of the session.
    let now = Instant::now();
    let mut c = standing_at(0xA9B4_0019, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.player_guid = Some(0x5000_0001);
    let cindrue = a_run_at_cindrue(&mut c, Phase::Selling { sent: Vec::new() }, now);
    c.world.open_vendor = Some(window_of(cindrue));
    assert_eq!(c.counter_at_hand().as_deref(), Some("Archmage Cindrue"));
    c.autoplay.config.growth.town_runs = false;
    c.autoplay_grow(now);
    assert!(
        !c.autoplay.growth.town_run_under_way(),
        "the run was not let go"
    );
    assert!(
        c.world.open_vendor.is_none(),
        "the run's window was left open"
    );
    assert_eq!(c.counter_at_hand(), None, "still at a counter");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn dying_at_the_counter_ends_the_visit() {
    // The run stood at its counter through the death, the window
    // stayed open, and recovery -- which puts the buffs back before
    // the walk to the corpse -- waited on buffs that waited on the
    // counter.
    let now = Instant::now();
    let mut c = standing_at(0xA9B4_0019, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.player_guid = Some(0x5000_0001);
    let cindrue = a_run_at_cindrue(&mut c, Phase::Selling { sent: Vec::new() }, now);
    c.world.open_vendor = Some(window_of(cindrue));
    // Dead: health at zero on a sheet with a name and an Endurance
    // to draw a maximum from.
    c.world.stats.name = "Vesperi".into();
    c.world.stats.attributes[1].base = 100;
    c.world.stats.vitals[0].current = 0;
    assert!(c.is_dead(), "not dead");
    c.autoplay_recover(now);
    assert!(
        !c.autoplay.growth.town_run_under_way(),
        "the run stood at the counter through the death"
    );
    assert!(c.world.open_vendor.is_none(), "the window was left open");
    assert_eq!(c.counter_at_hand(), None);
    // A run still on its walk is kept: the journey is resumed after.
    let me = c.player.as_ref().unwrap().world_position();
    c.autoplay.growth.run = Some(run_to(Vec2::new(me.x, me.y), now));
    c.counter_left_behind(now);
    assert!(
        c.autoplay.growth.town_run_under_way(),
        "a run on its walk was stopped for a death"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn vitals_wait_at_the_counter() {
    // Only the buffs were held. The vitals pass runs every tick
    // and casts the moment the last cast lands, so a Revitalize
    // thrown from the counter never left it a gap to open its
    // window in: the measured failure, one reflex over.
    let now = Instant::now();
    let (mut c, _) = a_caster_knowing(now, &["Revitalize Self I"]);
    c.autoplay.config.survive.manage_mana = true;
    // The stamina is gone.
    c.world.stats.vitals[1].current = 0;
    // Away from any counter the top-up goes out.
    let sent = c.session.actions_sent();
    assert!(
        c.autoplay_vitals(now),
        "no Revitalize with the stamina gone"
    );
    assert!(c.session.actions_sent() > sent, "the cast did not go out");
    c.autoplay.cast_sent = None;
    // At the counter it waits.
    let cindrue = 0x7a9b_4033;
    a_run_at_cindrue(&mut c, opening(cindrue), now);
    let sent = c.session.actions_sent();
    assert!(!c.autoplay_vitals(now), "a top-up cast at the counter");
    assert_eq!(c.session.actions_sent(), sent, "something went out");
    // Unless a fight has come to it there.
    c.autoplay.last_hit_us = Some(Instant::now());
    assert!(
        c.autoplay_vitals(now),
        "the top-up waited for the counter through a fight"
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_hold_is_let_go_by_a_pass_that_left_early() {
    // The note's flag was cleared only on the way past the hold. A
    // pass that turned back before it -- the top-ups, in a fight,
    // out of combat only -- left the flag set after the counter,
    // and the next visit's wait went unsaid.
    let now = Instant::now();
    let (mut c, _) = a_caster_with_a_buff_due(now);
    let cindrue = 0x7a9b_4033;
    a_run_at_cindrue(&mut c, opening(cindrue), now);
    assert!(!c.autoplay_buff(now, true));
    assert!(c.autoplay.buffs_held_at_counter, "the hold was not said");
    // Off the counter and in a fight, the top-up pass turns back
    // at once.
    c.autoplay.growth.run = None;
    c.autoplay.config.buffs.out_of_combat_only = true;
    c.attack_target = Some(0xdead);
    assert!(!c.autoplay_buff(now, false));
    assert!(
        !c.autoplay.buffs_held_at_counter,
        "the flag outlived the counter"
    );
}
