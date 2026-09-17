use std::time::Duration;

use glam::Vec3;

use super::*;
use crate::autoplay::{Mate, TeamView};
use crate::testkit;

/// The leader's own guid.
const LEADER: u32 = 0x5000_0002;

/// Put a leader called Verity `away` metres east of the character, on a
/// team this character is on with its player at the keyboard.
fn led_from(c: &mut Client, away: f32) {
    let team = &mut c.autoplay.config.team;
    team.enabled = true;
    team.follow = true;
    team.lead = false;
    let me = c.player.as_ref().expect("on its feet").world_position();
    c.autoplay.team = TeamView {
        mates: vec![Mate {
            health: 1.0,
            world: me + Vec3::new(away, 0.0, 0.0),
            // The one picking the targets, and the one that asked to.
            leader: true,
            leads: true,
            ..testkit::mate(LEADER, "Verity")
        }],
        ..Default::default()
    };
}

/// One tick of the rules, and how many actions went to the server on
/// it. The first tick of a session on a team sets the character options
/// a teammate keeps (see `Client::autoplay_accept_invites`), so what a
/// rule sends is counted from one tick to the next rather than from
/// nothing.
fn tick(c: &mut Client) -> u32 {
    let before = c.session.actions_sent();
    c.tick_autoplay(Instant::now());
    c.session.actions_sent() - before
}

/// A character on its feet in the Holtburg field, autoplay off, with a
/// leader `away` metres east.
fn played_by_hand(away: f32) -> Client {
    let mut c = testkit::offline_client();
    testkit::stand(&mut c, 0xA9B4_0019, Vec3::new(84.0, 84.0, 94.0));
    c.world.player_guid = Some(testkit::ME);
    c.autoplay.config.enabled = false;
    led_from(&mut c, away);
    c
}

#[test]
fn a_character_played_by_hand_still_follows_its_leader() {
    let mut c = played_by_hand(30.0);
    assert!(c.follow.is_none(), "nothing to walk to yet");
    c.tick_autoplay(Instant::now());
    assert!(
        c.follow.is_some(),
        "following needs no autoplay: the player asked for a leader, not for the rules"
    );
    assert_eq!(c.autoplay.doing, Doing::Following);
    assert!(
        c.autoplay.status.contains("Verity"),
        "the status says nothing about the leader: {:?}",
        c.autoplay.status
    );
}

#[test]
fn the_status_line_of_a_follower_survives_the_tick_that_wrote_it() {
    // With autoplay off the status is cleared every tick. A rule that
    // ran anyway had its line wiped the moment it wrote it, so the panel
    // and the fleet view showed a character walking and saying nothing.
    let mut c = played_by_hand(30.0);
    let now = Instant::now();
    c.tick_autoplay(now);
    c.tick_autoplay(now + Duration::from_millis(100));
    assert_eq!(c.autoplay.doing, Doing::Following);
    assert!(!c.autoplay.status.is_empty());
    // And it is not autoplay: no step of the table claimed the tick.
    assert_eq!(c.autoplay.step, None);
}

#[test]
fn the_status_is_cleared_again_once_the_leader_is_reached() {
    // The line stands only while a rule is acting: the clearing that
    // goes with autoplay off is deferred, not lost.
    let mut c = played_by_hand(30.0);
    let now = Instant::now();
    c.tick_autoplay(now);
    assert!(
        !c.autoplay.status.is_empty(),
        "it never said it was following"
    );
    let me = c.player.as_ref().expect("on its feet").world_position();
    c.autoplay.team.mates[0].world = me + Vec3::new(1.0, 0.0, 0.0);
    c.tick_autoplay(now + Duration::from_millis(100));
    assert_eq!(c.autoplay.doing, Doing::Idle);
    assert!(
        c.autoplay.status.is_empty(),
        "the leader is here and it is still saying {:?}",
        c.autoplay.status
    );
}

#[test]
fn following_is_off_with_the_team_off_or_the_following_off() {
    for off in ["team", "follow"] {
        let mut c = played_by_hand(30.0);
        match off {
            "team" => c.autoplay.config.team.enabled = false,
            _ => c.autoplay.config.team.follow = false,
        }
        c.tick_autoplay(Instant::now());
        assert!(c.follow.is_none(), "{off} off and it walked anyway");
        assert!(c.autoplay.status.is_empty(), "{off} off and it said things");
    }
}

#[test]
fn with_everything_off_it_does_nothing_at_all() {
    let mut c = played_by_hand(30.0);
    c.autoplay.config.team.enabled = false;
    c.autoplay.config.team.follow = false;
    c.autoplay.config.team.focus_fire = false;
    assert_eq!(tick(&mut c), 0, "something went to the server");
    assert!(c.follow.is_none());
    assert_eq!(c.attack_target, None);
    assert_eq!(c.autoplay.doing, Doing::Idle);
    assert!(c.autoplay.status.is_empty());
}

#[test]
fn it_hits_what_the_leader_hits_and_picks_nothing_of_its_own() {
    let mut c = played_by_hand(3.0);
    let theirs = testkit::standing_by(&mut c, 0x8000_0001, "Drudge Skulker", 4.0);
    // And one the leader is not on, nearer: the picking would have
    // taken this one, and the picking is what a player keeps.
    testkit::standing_by(&mut c, 0x8000_0002, "Drudge Slinker", 2.0);
    c.autoplay.team.mates[0].target = Some(theirs.guid);
    c.tick_autoplay(Instant::now());
    assert_eq!(
        c.attack_target,
        Some(theirs.guid),
        "it did not join the leader's fight"
    );
    assert!(c.combat, "it swung without entering combat");
    assert!(
        c.autoplay.status.contains("Verity") && c.autoplay.status.contains("Drudge Skulker"),
        "the status says nothing about the help: {:?}",
        c.autoplay.status
    );
}

#[test]
fn with_the_leader_on_nothing_it_starts_no_fight() {
    let mut c = played_by_hand(3.0);
    testkit::standing_by(&mut c, 0x8000_0002, "Drudge Slinker", 2.0);
    tick(&mut c);
    assert_eq!(c.attack_target, None, "it picked a target of its own");
    assert_eq!(tick(&mut c), 0, "it sent the server something of its own");
}

#[test]
fn a_target_too_far_from_either_of_us_is_left_alone() {
    // Further than the character's own reach, and further than the
    // leader's: the pair `Client::pick_target` weighs, so assisting
    // takes on what fighting alongside would and no more.
    for (leader_away, target_away) in [(3.0, 40.0), (40.0, 44.0)] {
        let mut c = played_by_hand(leader_away);
        c.autoplay.config.team.follow = false;
        let theirs = testkit::standing_by(&mut c, 0x8000_0001, "Drudge Skulker", target_away);
        c.autoplay.team.mates[0].target = Some(theirs.guid);
        c.tick_autoplay(Instant::now());
        assert_eq!(
            c.attack_target, None,
            "it set off after a target {target_away} m away with its leader {leader_away} m away"
        );
    }
}

#[test]
fn a_caster_is_left_in_its_own_stance() {
    // A wand does not swing, and which spell to cast is the player's.
    let mut c = played_by_hand(3.0);
    testkit::a_weapon(
        &mut c,
        0x8000_0010,
        ac_world::item_type::CASTER,
        "Training Wand",
        true,
    );
    let theirs = testkit::standing_by(&mut c, 0x8000_0001, "Drudge Skulker", 4.0);
    c.autoplay.team.mates[0].target = Some(theirs.guid);
    tick(&mut c);
    assert!(!c.combat && !c.magic, "it changed the caster's stance");
    assert_eq!(c.attack_target, None);
    assert_eq!(tick(&mut c), 0, "it sent the server something of its own");
}

#[test]
fn assisting_is_off_with_focus_fire_off() {
    let mut c = played_by_hand(3.0);
    c.autoplay.config.team.focus_fire = false;
    let theirs = testkit::standing_by(&mut c, 0x8000_0001, "Drudge Skulker", 4.0);
    c.autoplay.team.mates[0].target = Some(theirs.guid);
    c.tick_autoplay(Instant::now());
    assert_eq!(c.attack_target, None);
}

#[test]
fn no_errand_of_its_own_is_started_while_the_player_plays() {
    // Everything the rules like is there for the taking: a million
    // experience to spend and a sword in the pack better than the empty
    // hands. Thirty seconds of ticks, and the only thing that moved is
    // the walk after the leader.
    let mut c = played_by_hand(30.0);
    c.world.stats.available_xp = 1_000_000;
    testkit::a_weapon(
        &mut c,
        0x8000_0020,
        ac_world::item_type::MELEE_WEAPON,
        "Shou-jen Long Sword",
        false,
    );
    let start = Instant::now();
    // The first tick sets the character options a teammate keeps; what
    // the rules send is counted from there.
    c.tick_autoplay(start);
    let settled = c.session.actions_sent();
    let mut t = start;
    while t < start + Duration::from_secs(30) {
        c.tick_autoplay(t);
        t += Duration::from_millis(100);
    }
    // A raise, a wield and a vendor's window are all GameActions, so one
    // count answers for the three of them.
    assert_eq!(
        c.session.actions_sent(),
        settled,
        "the rules sent the server something while the player was playing"
    );
    assert!(c.autoplay.growth.hunting_at.is_none(), "it chose a ground");
    assert!(
        !c.autoplay.growth.town_run_under_way(),
        "it set off on a town run"
    );
    assert!(!c.traveling(), "it set off somewhere of its own");
    assert_eq!(c.autoplay.step, None, "a step of the table claimed a tick");
    assert!(c.follow.is_some(), "and yet it did keep up with its leader");
    // The control: the same character, the same moment, the rules on.
    // The table had a row worth running all along, so the count above is
    // worth something.
    c.autoplay.config.enabled = true;
    c.tick_autoplay(t);
    assert_eq!(c.autoplay.step, Some("catch up"));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn its_player_keeps_the_legs_and_the_hands() {
    // A level 10 character where the Holtburg Dungeon's portal drops it,
    // with a million experience to spend: everything the growth rules
    // want -- a rank to buy, a ground to hunt, rooms to explore -- is
    // there for the taking the moment the rules are on.
    let mut c = testkit::standing_in_the_field(10, 0x01F6_0289, Vec3::new(96.7, -10.0, 0.0));
    c.world.stats.available_xp = 1_000_000;
    c.autoplay.config.enabled = false;
    led_from(&mut c, 30.0);
    let start = Instant::now();
    // The first tick sets the character options a teammate keeps; what
    // the rules send is counted from there.
    c.tick_autoplay(start);
    let settled = c.session.actions_sent();
    let mut t = start;
    while t < start + Duration::from_secs(30) {
        c.tick_autoplay(t);
        t += Duration::from_millis(100);
    }
    assert_eq!(
        c.session.actions_sent(),
        settled,
        "the rules sent the server something while the player was playing"
    );
    assert!(c.hunting_ground().is_none(), "it chose a hunting ground");
    assert!(!c.traveling(), "it set off somewhere of its own");
    assert_eq!(c.autoplay.step, None, "a step of the table claimed a tick");
    assert!(c.follow.is_some(), "and yet it did keep up with its leader");
    // The same character with the rules on: they were live all along and
    // had plenty to do, which is what the count above is worth.
    c.autoplay.config.enabled = true;
    c.tick_autoplay(t);
    assert!(
        c.autoplay.step.is_some(),
        "nothing for autoplay to do either, so the count above says nothing"
    );
}
