use super::*;
use crate::autoplay::growth::tests::run_to;
use crate::autoplay::growth::town_run::VENDOR_REACH;
use crate::autoplay::growth::Growth;
use crate::testkit::{standing_at, standing_by};

/// Whether a run standing `away` metres from its counter is there.
fn there(away: f32) -> bool {
    away <= VENDOR_REACH
}

#[test]
fn a_walk_to_a_counter_broken_off_by_a_corpse_is_walked_on() {
    // +Verity, carrying as much as she meant to, set off for Shopkeeper
    // Renald the Elder 250 m away. A second later the looting walked
    // her to a fresh corpse, which ends a journey, and the run found no
    // journey under way 224 m short: "could not get to", sold nothing.
    assert_eq!(
        on_the_way(there(224.0), true, false, false),
        OnTheWay::WalkOn
    );
    // Not while what broke it off still has her: a fight on the way.
    assert_eq!(on_the_way(there(224.0), true, true, false), OnTheWay::Wait);
    // A journey that ended by itself short of the counter -- it gave
    // up, or arrived somewhere else -- still could not get there.
    assert_eq!(
        on_the_way(there(224.0), false, false, false),
        OnTheWay::Short
    );
    assert_eq!(
        on_the_way(there(224.0), false, true, false),
        OnTheWay::Short
    );
    // Near enough, she goes up to the counter however it ended.
    for (broken_off, busy) in [(false, false), (true, false), (true, true)] {
        assert_eq!(
            on_the_way(there(VENDOR_REACH), broken_off, busy, false),
            OnTheWay::There
        );
    }
}

#[test]
fn a_walk_broken_off_again_as_soon_as_it_is_planned_waits_before_the_next_plan() {
    // Something that ends the walk to the counter the moment it is
    // planned -- a follower pulled back to its leader each time it
    // closed to the following distance -- had it planned again every
    // tick or two for the run's four minutes, a route search each time.
    assert_eq!(on_the_way(there(224.0), true, false, true), OnTheWay::Wait);
    // Once the moment is up it is planned again.
    assert_eq!(
        on_the_way(there(224.0), true, false, false),
        OnTheWay::WalkOn
    );
    // Neither giving up nor going up to the counter waits on it.
    assert_eq!(
        on_the_way(there(224.0), false, false, true),
        OnTheWay::Short
    );
    assert_eq!(
        on_the_way(there(VENDOR_REACH), true, false, true),
        OnTheWay::There
    );
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn what_stands_on_the_road_is_walked_past_and_what_swings_at_us_is_not() {
    // "Traveling to the hunting ground shouldn't have much fighting,
    // more ignoring the monsters on the way so you can get to the
    // hunting ground" -- the player, and the reason for the rule.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let fight = c.autoplay.config.fight.clone();
    let it = standing_by(&mut c, 0x8000_0001, "Revenant", 5.0);

    // Standing on its ground with nowhere to be: a fight.
    assert!(!c.passing_by(&it, &fight));
    assert!(c.would_fight(&it, &fight, false, now));

    // Bound for a hunting ground and walking there: walked past, and
    // the fight rules have nothing to pick.
    let ground = (0xA9B2, Vec2::new(32_580.0, 34_570.0), "Drudge".to_string());
    assert!(c.grow_travel(Vec2::new(me.x + 250.0, me.y), now));
    c.autoplay.growth.bound = Some(ground.clone());
    assert!(c.passing_by(&it, &fight));
    assert!(!c.would_fight(&it, &fight, false, now));

    // Unless it swings: the road does not get to decide that.
    c.autoplay.attacked_by("Revenant", now);
    assert!(!c.passing_by(&it, &fight));
    assert!(c.would_fight(&it, &fight, false, now));
    // Which says nothing about the one standing next to it.
    let other = standing_by(&mut c, 0x8000_0002, "Drudge Skulker", 6.0);
    assert!(c.passing_by(&other, &fight));
    c.autoplay.last_hit_us = None;
    c.autoplay.hit_by.clear();

    // Named in "only these", it is still the road. The list says which
    // kind to hunt at the far end, and every creature the fight could
    // pick already matches it, so as an override it switched the rule
    // off for anyone who kept one.
    let only = crate::autoplay::Fight {
        only: vec!["revenant".into()],
        ..fight.clone()
    };
    assert!(c.passing_by(&it, &only));
    assert!(!c.would_fight(&it, &only, false, now));

    // A creature of ours on it is no reason to stop either. A summoned
    // creature picks its own fights, the nearest monster it can see,
    // and following its lead stopped the character for each in turn.
    c.world.objects.insert(
        0x8000_0003,
        ac_world::WorldObject {
            guid: 0x8000_0003,
            name: "Fire Elemental".into(),
            item_type: ac_world::item_type::CREATURE,
            pet_owner: 0x5000_0001,
            walked_at: Some(it.guid),
            ..Default::default()
        },
    );
    assert!(c.passing_by(&it, &fight), "its pet went for it");
    assert!(!c.would_fight(&it, &fight, false, now));
    c.world.objects.remove(&0x8000_0003);

    // With the setting off, everything on the road is fought again.
    let all = crate::autoplay::Fight {
        walk_past_on_the_way: false,
        ..fight.clone()
    };
    assert!(!c.passing_by(&it, &all));
    assert!(c.would_fight(&it, &all, false, now));

    // A town run is the same errand: the counters are the point of
    // it, not whatever stands between here and them.
    c.autoplay.growth.bound = None;
    c.autoplay.growth.run = Some(run_to(Vec2::new(32_500.0, 34_500.0), now));
    assert!(c.passing_by(&it, &fight));

    // The errand ending does not end the walking past: the journey
    // still under way is a road whoever planned it (see
    // `a_road_is_a_road_whoever_planned_it`). The journey ending does.
    c.autoplay.growth.run = None;
    assert!(c.passing_by(&it, &fight), "the journey is still under way");
    c.cancel_travel();
    assert!(!c.passing_by(&it, &fight), "nowhere to be again");
    assert!(c.would_fight(&it, &fight, false, now));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn an_errand_is_on_its_way_only_while_its_walk_is() {
    // The errand outlives its walk. `bound` is let go only when the
    // hunting step next looks, and a party restocking does not call
    // it: a leader home from town stood on its own ground walking past
    // everything that had not swung at it until the slowest of the
    // party had shopped. And a body at its feet kept the grow step
    // from running at all, so the body beat the monster beside it.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let fight = c.autoplay.config.fight.clone();
    let it = standing_by(&mut c, 0x8000_0001, "Revenant", 5.0);
    let at = Vec2::new(me.x + 250.0, me.y);
    let lb = (((at.x / 192.0) as u32) << 8) | (at.y / 192.0) as u32;

    // Bound, and walking there.
    assert!(c.grow_travel(at, now), "no way to the ground");
    c.autoplay.growth.bound = Some((lb, at, "Drudge".into()));
    assert!(c.on_its_way());
    assert!(!c.would_fight(&it, &fight, false, now));

    // A body on the road breaks the walk off, and it is still the walk.
    c.interrupt_travel("walking to a corpse");
    assert!(c.on_its_way(), "a walk broken off is still the walk");
    assert!(!c.would_fight(&it, &fight, false, now));

    // Arrived: fighting again that tick, whatever `bound` still says.
    assert!(c.grow_travel(at, now));
    c.end_trip();
    assert!(c.autoplay.growth.bound.is_some(), "not noticed yet");
    assert!(!c.on_its_way(), "arrived, and still walking past");
    assert!(c.would_fight(&it, &fight, false, now));

    // Cancelled -- the player took the keys -- is no walk either.
    assert!(c.grow_travel(at, now));
    c.cancel_travel();
    assert!(!c.on_its_way());

    // Stepping out of a shop first is the start of the walk.
    c.autoplay.growth.after_out = Some(at);
    assert!(c.on_its_way());
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_party_on_the_road_walks_past_together_and_stops_together() {
    // The leader, bound for a new ground, walked past a Drudge. Its
    // followers keep up through the follow step, which sets no errand,
    // so they stopped and fought it; the leader walked on, they were
    // fetched after it and turned on the Drudge again each time they
    // closed, and the leader never helped.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let fight = c.autoplay.config.fight.clone();
    let drudge = standing_by(&mut c, 0x8000_0001, "Drudge Skulker", 5.0);
    let other = standing_by(&mut c, 0x8000_0002, "Drudge Slinker", 6.0);
    let team = &mut c.autoplay.config.team;
    team.enabled = true;
    team.follow = true;
    team.lead = false;
    let leader = |on_its_way: bool, target: Option<u32>| crate::autoplay::Mate {
        name: "Leader".into(),
        leader: true,
        leads: true,
        world: me,
        cell: holtburg,
        on_its_way,
        target,
        ..Default::default()
    };

    // A leader going nowhere: its follower fights what is about.
    c.autoplay.team.mates = vec![leader(false, None)];
    assert!(!c.on_its_way());
    assert!(c.would_fight(&drudge, &fight, false, now));

    // A leader on its way: so is the follower keeping up with it.
    c.autoplay.team.mates = vec![leader(true, None)];
    assert!(c.on_its_way(), "a follower is on its leader's way");
    assert!(!c.would_fight(&drudge, &fight, false, now));
    assert!(!c.joins_the_team_on(drudge.guid, &fight));

    // Something attacks the leader on the road and it turns to fight:
    // the follower turns with it and joins on it, but not on the one
    // standing by.
    c.autoplay.team.mates = vec![leader(true, Some(drudge.guid))];
    assert!(c.would_fight(&drudge, &fight, false, now));
    assert!(c.joins_the_team_on(drudge.guid, &fight));
    assert!(!c.would_fight(&other, &fight, false, now));
    assert!(!c.joins_the_team_on(other.guid, &fight));

    // On a run of its own while the party back at the ground fights:
    // that fight is not the road's, and neither focus fire nor the
    // debuffer turns the character round for it.
    c.autoplay.config.team.follow = false;
    let counter = Vec2::new(me.x + 250.0, me.y);
    assert!(c.grow_travel(counter, now));
    c.autoplay.growth.run = Some(run_to(counter, now));
    c.autoplay.team.mates = vec![leader(false, Some(drudge.guid))];
    assert!(c.on_its_way());
    assert!(
        !c.joins_the_team_on(drudge.guid, &fight),
        "turned back for the party"
    );
    // Until the Drudge attacks the character itself.
    c.autoplay.attacked_by("Drudge Skulker", Instant::now());
    assert!(c.joins_the_team_on(drudge.guid, &fight));
}

#[test]
fn a_road_is_under_way_unless_it_is_a_walk_about_the_ground() {
    // No journey, nothing remembered: not on a road.
    assert!(!road_under_way(None, None));
    // A journey to somewhere else, or one put down for a fight.
    assert!(road_under_way(Some(false), None));
    assert!(road_under_way(None, Some(false)));
    // A roam, a patrol: about the ground, so never a road.
    assert!(!road_under_way(Some(true), None));
    assert!(!road_under_way(None, Some(true)));
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_road_is_a_road_whoever_planned_it() {
    // The walk past the road read only the growth rules' errands, so
    // a journey a script asked for was not "on its way": a party sent
    // down the Singularity Caul by script fought everything between
    // the drop and the far end, fourteen fights on a walk of forty
    // seconds.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let fight = c.autoplay.config.fight.clone();
    let it = standing_by(&mut c, 0x8000_0001, "Revenant", 5.0);
    let at = Vec2::new(me.x + 250.0, me.y);
    assert!(c.autoplay.growth.bound.is_none() && c.autoplay.growth.run.is_none());

    // A script's journey: on its way, and walking past.
    assert!(c.travel_to(at), "no way there");
    assert!(c.on_its_way());
    assert!(!c.would_fight(&it, &fight, false, now));

    // Put down for a fight that came to it: still the road, and the
    // road again once the fight is over.
    c.remember_journey();
    c.interrupt_travel("attacking");
    assert!(!c.traveling());
    assert!(
        c.on_its_way(),
        "a road put down for a fight is still the road"
    );
    assert!(c.autoplay_resume_journey());
    assert!(c.traveling() && c.on_its_way());

    // Cancelled -- the player took the keys -- is no road.
    c.cancel_travel();
    assert!(!c.on_its_way());
    assert!(c.would_fight(&it, &fight, false, now));

    // A roam about the ground is not a road, and is not one after a
    // fight has put it down either: what stands about the ground is
    // what the character came for.
    assert!(c.travel_about(at));
    assert!(!c.on_its_way(), "a roam walked past the ground");
    assert!(c.would_fight(&it, &fight, false, now));
    c.remember_journey();
    c.interrupt_travel("attacking");
    assert!(!c.on_its_way());
    assert!(c.autoplay_resume_journey());
    assert!(c.traveling());
    assert!(!c.on_its_way(), "a roam picked up again became a road");

    // A road put down for a body on it is remembered like one put
    // down for a fight, since nothing else would bring it back.
    c.cancel_travel();
    assert!(c.travel_to(at));
    c.autoplay.resume_trip = None;
    let spot = glam::Vec3::new(me.x + 30.0, me.y, me.z);
    c.walk_to_corpse(0x8000_0002, "Corpse of Drudge", spot, 30.0, now);
    assert!(!c.traveling());
    assert_eq!(c.autoplay.resume_trip, Some(at));
    assert!(c.on_its_way(), "a road put down for a body was lost");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn walking_the_hunting_ground_is_not_being_on_the_way_to_it() {
    // The trap this rule had to be kept out of. A patrol of the
    // hunting area, and the roam around a ground, both travel --
    // `traveling()` is true for a character doing the very thing it
    // came here for. Keyed on that, a character would have stood in
    // its own hunting ground refusing to fight. Both paths set off
    // and return before `bound` is ever set, and that is the whole
    // of the difference.
    let holtburg = 0xA9B4_0019;
    let mut c = standing_at(holtburg, glam::Vec3::new(84.0, 7.1, 94.0));
    c.world.stats.level = 20;
    c.world.player_guid = Some(0x5000_0001);
    let me = c.player.as_ref().unwrap().world_position();
    let now = Instant::now();
    let cfg = Growth::default();
    c.autoplay.config.fight.area = Some(crate::hunt::HuntArea {
        name: "test".into(),
        shape: crate::hunt::Shape::Outline {
            points: vec![
                [me.x - 40.0, me.y - 5.0],
                [me.x + 40.0, me.y - 5.0],
                [me.x + 40.0, me.y + 60.0],
                [me.x - 40.0, me.y + 60.0],
            ],
        },
    });
    let fight = c.autoplay.config.fight.clone();
    let it = standing_by(&mut c, 0x8000_0001, "Revenant", 5.0);

    // Quiet long enough to walk the area, and it walks it.
    c.autoplay.growth.quiet_since = Some(now - Duration::from_secs(600));
    assert!(c.grow_hunt(now, &cfg), "the patrol stayed put");
    assert!(c.traveling(), "the patrol is a journey like any other");
    assert_eq!(c.autoplay.growth.bound, None, "the patrol is not an errand");
    assert!(!c.on_its_way());

    // So the creature it walked up to is still a fight.
    assert!(!c.passing_by(&it, &fight));
    assert!(c.would_fight(&it, &fight, false, now));
}
