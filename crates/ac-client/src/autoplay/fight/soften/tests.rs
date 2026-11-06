use super::*;

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_leader_bolts_the_guard_it_made_vulnerable_and_does_not_move_on_to_the_next() {
    // Blargerton led with focus fire on: once Piercing Vulnerability V was on a Tusker Guard, the
    // party plan sent him to the next guard not yet vulned, 21 vulns and no bolt in two minutes.
    let now = Instant::now();
    let (mut c, ids) = crate::testkit::a_caster_knowing(
        now,
        &["Force Bolt VI", "Piercing Vulnerability Other VI"],
    );
    c.autoplay.config.enabled = true;
    c.autoplay.config.fight.enabled = true;
    c.autoplay.config.team.enabled = true;
    c.autoplay.config.team.focus_fire = true;
    c.autoplay.team.leader = true;
    let (first, second) = (0x8000_0103, 0x8000_0104);
    for (guid, metres) in [(first, 6.0), (second, 8.0)] {
        crate::testkit::standing_by(&mut c, guid, "Tusker Guard", metres);
        if let Some(o) = c.world.objects.get_mut(&guid) {
            o.weenie_class_id = 1629;
        }
    }
    c.autoplay.casting_at = Some(first);
    c.autoplay.mark_vulned(first, Some((Element::Pierce, 2.1)));
    let plan = c.plan_for_team(now);
    let me = c.world.player_guid.expect("a player");
    assert_eq!(
        plan.orders.get(&me).and_then(|o| o.target),
        Some(first),
        "{plan:?}"
    );
    let cfg = c.autoplay.config.fight.clone();
    c.autoplay_fight_with_spells(now, &cfg);
    assert_eq!(c.autoplay.casting_at, Some(first));
    assert_eq!(
        c.autoplay.attack_spell,
        Some(ids[0]),
        "Force Bolt VI thrown"
    );
}
