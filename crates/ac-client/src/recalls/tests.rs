use super::*;
use crate::testkit::{offline_client, stand};

/// An offline session for `name` on a test server, keeping its files under `dir`.
fn session(dir: &std::path::Path, name: &str) -> Client {
    let mut c = offline_client();
    c.world.stats.name = name.into();
    c.config.host = "recall.test".into();
    std::fs::create_dir_all(dir.join("profiles")).unwrap();
    let shelf = std::sync::Arc::new(crate::profile::Library::default());
    shelf.open(dir.join("profiles"));
    c.profiles = shelf;
    c
}

#[test]
fn a_lifestone_recall_teaches_where_the_lifestone_is() {
    // Rawrtherton logged in with no lifestone known and walked 8.5 km for a counter /lifestone
    // and the Town Network reach in a minute: where /lifestone lands is the lifestone.
    let dir = std::env::temp_dir().join("acswarm-test-recall-lands");
    let _ = std::fs::remove_dir_all(&dir);
    let mut c = session(&dir, "Blargerton");
    stand(&mut c, 0xA87C_0006, Vec3::new(23.8, 140.4, 26.3));
    assert!(c
        .world
        .stats
        .recall_position(position_type::SANCTUARY)
        .is_none());
    c.recall_notice("Blargerton is recalling to the lifestone.");
    // The server puts us beside the Holtburg lifestone.
    let lifestone = Vec3::new(84.0, 7.1, 94.0);
    stand(&mut c, 0xA9B4_0019, lifestone);
    c.recall_landed();
    let p = c
        .world
        .stats
        .recall_position(position_type::SANCTUARY)
        .expect("learnt");
    assert_eq!((p.cell, p.local), (0xA9B4_0019, lifestone));
}

#[test]
fn a_server_correction_after_a_recall_is_not_its_landing() {
    let dir = std::env::temp_dir().join("acswarm-test-recall-nudge");
    let _ = std::fs::remove_dir_all(&dir);
    let mut c = session(&dir, "Blargerton");
    stand(&mut c, 0xA9B4_0019, Vec3::new(84.0, 7.1, 94.0));
    c.note_cast(recalls::spell::LIFESTONE_RECALL);
    stand(&mut c, 0xA9B4_0019, Vec3::new(86.0, 9.0, 94.0));
    c.recall_landed();
    assert!(c
        .world
        .stats
        .recall_position(position_type::LINKED_LIFESTONE)
        .is_none());
}

#[test]
fn what_was_learnt_is_read_back_at_the_next_login() {
    // ACE sends only the last death's position at login, so a lifestone learnt is kept on disk.
    let dir = std::env::temp_dir().join("acswarm-test-recall-kept");
    let _ = std::fs::remove_dir_all(&dir);
    let mut c = session(&dir, "Blargerton");
    let lifestone = Position::new_flat(0xA9B4_0019, Vec3::new(84.0, 7.1, 94.0));
    c.learn_recall_position(position_type::SANCTUARY, lifestone);

    let mut next = session(&dir, "Blargerton");
    assert!(next
        .world
        .stats
        .recall_position(position_type::SANCTUARY)
        .is_none());
    next.load_recall_positions();
    let p = next
        .world
        .stats
        .recall_position(position_type::SANCTUARY)
        .expect("read back");
    assert_eq!((p.cell, p.local), (lifestone.cell, lifestone.local));
    // Another character on the server has its own.
    let mut other = session(&dir, "Fletch");
    other.load_recall_positions();
    assert!(other
        .world
        .stats
        .recall_position(position_type::SANCTUARY)
        .is_none());
}
