use super::*;
use crate::autoplay::Doing;

#[test]
fn config_round_trips_through_json() {
    let mut c = Config {
        enabled: true,
        ..Config::default()
    };
    c.buffs.spells = vec!["Strength Self".into()];
    c.fight.avoid = vec!["Olthoi".into()];
    let text = serde_json::to_string(&c).unwrap();
    let back: Config = serde_json::from_str(&text).unwrap();
    assert_eq!(back, c);
    // Missing fields fall back to the defaults.
    let partial: Config = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
    assert!(partial.enabled);
    assert_eq!(partial.survive.heal_below, Survive::default().heal_below);
    assert_eq!(Doing::Fighting.label(), "fighting");
    // A settings file written when the heal spell was still a
    // setting keeps loading, rest and all: the heal is chosen by
    // the rules now, and a name left in the file is no reason to
    // throw a player's whole configuration away.
    let old: Config =
        serde_json::from_str(r#"{"survive":{"heal_spell":"Heal Self VI","heal_below":0.45}}"#)
            .unwrap();
    assert_eq!(old.survive.heal_below, 0.45);
    assert!(old.survive.use_kits);
    // And one written before critters were walked past walks past
    // them: a bool left out would otherwise read as off. The same
    // goes for walking past what stands on the road.
    let before: Config = serde_json::from_str(r#"{"fight":{"radius":30.0}}"#).unwrap();
    assert!(before.fight.skip_critters);
    assert!(before.fight.walk_past_on_the_way);
}
