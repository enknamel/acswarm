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

#[test]
fn a_delivery_asked_for_is_taken_once_and_keeps_only_items_on_mules() {
    let mut board = crate::Blackboard::default();
    let ask = |board: &mut crate::Blackboard, items: Vec<Wanted>| {
        let mut asked: Vec<Request> = board
            .get(DELIVER_KEY)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        asked.push(Request {
            to: "rawrtherton".into(),
            items,
        });
        board.set_local(DELIVER_KEY, serde_json::to_value(asked).unwrap());
    };
    let w = |account: &str, guid: u32| Wanted {
        account: account.into(),
        guid,
    };
    ask(&mut board, vec![w("a", 1), w("stranger", 2)]);
    ask(&mut board, vec![w("a", 1), w("b", 3)]);
    let mut mules = Mules {
        roster: vec![mule("a", Some(40)), mule("b", Some(90))],
        ..Default::default()
    };
    let mut settings = crate::Settings::new();
    let mut icons = crate::IconCache::default();
    let mut cx = crate::Ctx {
        clients: Vec::new(),
        index: 0,
        board: &mut board,
        settings: &mut settings,
        icons: &mut icons,
        dt: 0.05,
        now: std::time::Instant::now(),
        chat: Vec::new(),
        activate: None,
        quit: false,
        pick_data_dir: false,
        start_sessions: Vec::new(),
        stop_sessions: Vec::new(),
    };
    mules.take_requests(&mut cx);
    assert_eq!(mules.deliveries.len(), 1, "one receiver, one delivery");
    assert_eq!(
        mules.deliveries[0].items,
        vec![("a".to_string(), 1), ("b".to_string(), 3)],
        "the stranger's is no mule's, and the repeat is asked once"
    );
    assert_eq!(mules.wanted(), vec!["a".to_string(), "b".to_string()]);
    mules.take_requests(&mut cx);
    assert_eq!(
        mules.deliveries[0].items.len(),
        2,
        "a request is taken once"
    );
}
