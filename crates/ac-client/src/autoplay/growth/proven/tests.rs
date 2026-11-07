#[test]
fn a_kill_tasks_count_proves_its_creatures_tier() {
    // Five hundred Tusker Guards counted: their loot tier, 4, is survived.
    let mut c = crate::testkit::offline_client();
    assert_eq!(c.proven_tier(), 0);
    c.quest_log.kills.insert("tuskergkillcount".into(), 500);
    assert_eq!(c.proven_tier(), 4);
}

#[test]
fn a_kill_of_ours_raises_the_tier_and_a_lesser_one_does_not_lower_it() {
    let mut c = crate::testkit::offline_client();
    let mut guard = crate::testkit::creature(0x8000_0103, "Tusker Guard");
    guard.weenie_class_id = 1629;
    c.world.objects.insert(guard.guid, guard);
    c.autoplay.casting_at = Some(0x8000_0103);
    c.note_kill_tier("You flatten Tusker Guard's body with the force of your assault!");
    assert_eq!(c.proven_tier(), 4);
    let mut lieutenant = crate::testkit::creature(0x8000_0104, "Tumerok Lieutenant");
    lieutenant.weenie_class_id = 11895;
    c.world.objects.insert(lieutenant.guid, lieutenant);
    c.autoplay.casting_at = Some(0x8000_0104);
    c.note_kill_tier("Tumerok Lieutenant is fatally punctured!");
    assert_eq!(c.proven_tier(), 4, "a tier 2 kill proves nothing more");
}
