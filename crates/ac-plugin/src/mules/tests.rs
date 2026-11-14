use super::*;

const HOST: &str = "play.example:9000";

fn mule(account: &str, room: Option<u32>) -> Mule {
    Mule {
        host: HOST.into(),
        account: account.into(),
        character: format!("{account} char"),
        room,
        ready: room.is_some(),
        ..Default::default()
    }
}

#[test]
fn the_mule_with_most_room_is_picked_and_a_full_or_running_one_never() {
    let roster = vec![
        mule("a", Some(40)),
        mule("b", Some(3)),
        mule("c", Some(90)),
        mule("d", Some(200)),
    ];
    assert_eq!(
        pick(&roster, HOST, &["d".into()]),
        Some(2),
        "d is logged in"
    );
    assert_eq!(pick(&roster, "other:9000", &[]), None, "another server's");
    let full = vec![mule("b", Some(3))];
    assert_eq!(pick(&full, HOST, &[]), None, "a new one is made");
}

#[test]
fn a_mule_never_in_the_world_counts_as_roomy() {
    let roster = vec![mule("a", Some(40)), mule("new", None)];
    assert_eq!(pick(&roster, HOST, &[]), Some(1));
}

#[test]
fn mules_are_named_after_their_character_with_letters_only() {
    assert_eq!(
        names_for("rawrtherton", "Blargerton", 1),
        ("rawrthertonmule1".into(), "Blargerton Mule A".into())
    );
    assert_eq!(
        names_for("Raw-r", "+Scn Seller", 27).1,
        "Scn Seller Mule AA"
    );
}

#[test]
fn a_password_is_long_and_never_the_same_twice() {
    let (a, b) = (password(), password());
    assert_eq!(a.len(), 20);
    assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
    assert_ne!(a, b);
}

#[test]
fn a_failed_mule_is_never_picked_and_its_number_never_used_again() {
    let mut failed = mule("rawrmule2", None);
    failed.failed = true;
    let mules = Mules {
        roster: vec![mule("rawrmule1", Some(3)), failed],
        ..Default::default()
    };
    assert_eq!(pick(&mules.roster, HOST, &[]), None);
    assert_eq!(mules.made_on(HOST), 2, "the next is rawrmule3");
}
