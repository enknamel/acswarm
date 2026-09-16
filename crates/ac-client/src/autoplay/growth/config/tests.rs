use super::*;

#[test]
fn growth_config_has_defaults_and_round_trips() {
    let g: Growth = serde_json::from_str("{}").unwrap();
    assert_eq!(g, Growth::default());
    assert!(g.auto_xp && g.hunt_grounds && g.town_runs);
    let text = serde_json::to_string(&g).unwrap();
    let back: Growth = serde_json::from_str(&text).unwrap();
    assert_eq!(back, g);
    let partial: Growth = serde_json::from_str(r#"{"auto_xp":false,"level_margin":3}"#).unwrap();
    assert!(!partial.auto_xp);
    assert_eq!(partial.level_margin, 3);
    assert!(partial.town_runs);
    // A config saved before the sale rules had thresholds reads
    // with the defaults, and the defaults are on.
    assert_eq!(partial.sell_run_value, 5_000);
    assert_eq!(partial.sell_run_count, 8);
    assert_eq!(partial.sell_run_patience, 15.0 * 60.0);
    let set: Growth = serde_json::from_str(r#"{"sell_run_value":0,"sell_run_count":3}"#).unwrap();
    assert_eq!(set.sell_run_value, 0);
    assert_eq!(set.sell_run_count, 3);
}
