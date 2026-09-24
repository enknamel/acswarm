use super::*;
use crate::autoplay::growth::needs::NeedKind;
use crate::autoplay::growth::town_run::Errand;
use crate::testkit::{a_counter, standing_at};

/// A run on its way to a counter at `at`, set off at `now`.
pub(super) fn run_to(at: Vec2, now: Instant) -> Run {
    Run {
        vendor: "Shopkeeper Renald the Elder".into(),
        at,
        phase: Phase::Going,
        since: now,
        last_sell: None,
        town: at,
        stops: 1,
        sold: 0,
        reason: "carrying as much as it means to".into(),
        errand: Errand::Sell,
        visited: vec![at],
        walked_on: None,
        walk_limit: super::road::WALK_TIMEOUT,
    }
}

pub(super) fn need(kind: NeedKind, want: u32) -> Need {
    Need {
        name: "x".into(),
        want,
        have: 0,
        keep: want,
        urgent: true,
        buyable: true,
        from: None,
        kind,
    }
}

#[test]
fn a_long_list_is_cut_short() {
    assert_eq!(a_few(&[]), "nothing");
    assert_eq!(a_few(&["Myrrh"]), "Myrrh");
    assert_eq!(a_few(&["a", "b", "c", "d"]), "a, b, c, d");
    assert_eq!(a_few(&["a", "b", "c", "d", "e"]), "a, b, c, d and 1 more");
}

#[test]
fn case_folds_without_allocating() {
    assert!(contains_fold("Prismatic Taper", "taper"));
    assert!(contains_fold("PRISMATIC TAPER", "prismatic"));
    assert!(!contains_fold("Lead Scarab", "taper"));
    assert!(!contains_fold("Tap", "taper"));
    assert!(!contains_fold("anything", ""));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn an_errand_whose_setting_is_turned_off_is_let_go() {
    // Nothing carries a run on with town runs turned off, nor a walk
    // to a ground with grounds off, so neither was ever let go. The
    // errand read as under way for the rest of the session.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let now = Instant::now();
    let to = Vec2::new(32_500.0, 34_500.0);
    let growth = &mut c.autoplay.config.growth;
    growth.auto_xp = false;
    growth.town_runs = false;
    growth.hunt_grounds = false;
    c.autoplay.growth.run = Some(run_to(to, now));
    c.autoplay.growth.bound = Some((0xA9B2, to, "Drudge".into()));
    c.autoplay_grow(now);
    assert!(!c.autoplay.growth.town_run_under_way(), "the run was kept");
    assert_eq!(c.autoplay.growth.bound, None, "the walk was kept");
}

/// A run standing at Archmage Cindrue's counter, the Use just
/// gone out: the counter in view where the character stands, and
/// the run in `phase` there.
pub(super) fn a_run_at_cindrue(c: &mut Client, phase: Phase, now: Instant) -> u32 {
    let me = c.player.as_ref().unwrap().world_position();
    let cindrue = 0x7a9b_4033;
    a_counter(
        c,
        cindrue,
        "Archmage Cindrue",
        glam::Vec3::new(84.0, 7.1, 94.0),
    );
    let mut run = run_to(Vec2::new(me.x, me.y), now);
    run.vendor = "Archmage Cindrue".into();
    run.phase = phase;
    c.autoplay.growth.run = Some(run);
    cindrue
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_refusal_past_the_busy_asks_is_the_counters_own() {
    // Past the asks a busy refusal earns the run is on the ordinary
    // clock, but a later refusal was still kept, and the status
    // promised an ask "once the cast lands" that was never coming.
    let now = Instant::now();
    let mut c = standing_at(0xA9B4_0019, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.player_guid = Some(0x5000_0001);
    let cindrue = 0x7a9b_4033;
    a_run_at_cindrue(
        &mut c,
        Phase::Opening {
            guid: cindrue,
            tries: 1,
            busy: None,
            asked_over: BUSY_ASKS,
        },
        now,
    );
    c.autoplay.growth.counter_said_busy(now);
    assert!(
        matches!(
            c.autoplay.growth.run.as_ref().unwrap().phase,
            Phase::Opening { busy: None, .. }
        ),
        "a refusal past the cap was kept"
    );
    let saying = c.town_run_view(now).unwrap().saying;
    assert!(
        saying.contains("waiting for"),
        "the status still promises an ask: {saying}"
    );
    // Under the cap it is heard.
    a_run_at_cindrue(
        &mut c,
        Phase::Opening {
            guid: cindrue,
            tries: 1,
            busy: None,
            asked_over: BUSY_ASKS - 1,
        },
        now,
    );
    c.autoplay.growth.counter_said_busy(now);
    assert!(matches!(
        c.autoplay.growth.run.as_ref().unwrap().phase,
        Phase::Opening { busy: Some(_), .. }
    ));
}

#[test]
fn the_spells_cast_are_kept_a_second_and_worked_out_again_when_one_is_learned() {
    let mut c = Client::offline(crate::testkit::no_data());
    c.world.stats.spells = vec![1];
    let first = c.spells_cast();
    let when = |c: &Client| {
        c.autoplay
            .spells_cast_memo
            .borrow()
            .as_ref()
            .map(|m| (m.0, m.1))
    };
    let kept = when(&c);
    assert_eq!(c.spells_cast(), first);
    assert_eq!(when(&c), kept, "worked out again within the second");
    c.world.stats.spells.push(2);
    c.spells_cast();
    assert_eq!(
        when(&c).map(|w| w.1),
        Some(2),
        "a spell learned was not noticed"
    );
}

#[test]
fn a_long_planned_walk_is_given_twice_its_time_and_a_short_one_the_fixed_four_minutes() {
    use super::road::{walk_limit, WALK_TIMEOUT};
    assert_eq!(walk_limit(None), WALK_TIMEOUT);
    assert_eq!(walk_limit(Some(30.0)), WALK_TIMEOUT);
    assert_eq!(
        walk_limit(Some(654.0)),
        std::time::Duration::from_secs(1308)
    );
    assert_eq!(
        walk_limit(Some(f32::INFINITY)),
        WALK_TIMEOUT,
        "a planner's nonsense is not believed"
    );
}
