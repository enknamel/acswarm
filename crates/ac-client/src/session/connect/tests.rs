use super::*;

#[test]
fn an_offline_session_starts_with_the_defaults_of_a_connected_one() {
    // One constructor builds both. The numbers are pinned here so
    // that grouping these fields behind a derived Default cannot zero
    // them unseen.
    let assets = std::rc::Rc::new(ac_scene::Assets::empty());
    let offline = Client::offline(assets.clone());
    let online = Client::connect(
        Config {
            host: "127.0.0.1:1".into(),
            account: "acreborn".into(),
            password: "x".into(),
            character: None,
            auto_enter: true,
        },
        assets,
    )
    .unwrap();
    assert!(offline.socket.is_none());
    assert!(online.socket.is_some());
    assert_eq!(
        (offline.primary, offline.secondary),
        (online.primary, online.secondary)
    );
    assert_eq!(offline.session.state(), online.session.state());
    for c in [&offline, &online] {
        assert_eq!(c.attack_height, 2);
        assert_eq!(c.attack_power, 0.5);
        assert_eq!(c.attack_backoff, Duration::from_millis(300));
        assert_eq!(c.speed_boost, 2.0);
        assert_eq!(c.jump_height, 9.0);
        assert_eq!(c.turbine_context, 1);
    }
}
