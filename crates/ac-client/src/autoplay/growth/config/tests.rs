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
    // A config saved while loot alone could send a character to town still reads.
    let old: Growth =
        serde_json::from_str(r#"{"sell_run_value":0,"sell_run_count":3,"town_runs":false}"#)
            .unwrap();
    assert!(!old.town_runs);
}
