use super::*;

#[test]
fn player_killed_and_account_banned_parse() {
    let mut w = Writer::new();
    w.string16("Reborn is killed by a Drudge!")
        .u32(0x5000_0001)
        .u32(0x8000_0030);
    let (text, victim, killer) = parse_player_killed(&w.finish()).unwrap();
    assert_eq!(text, "Reborn is killed by a Drudge!");
    assert_eq!((victim, killer), (0x5000_0001, 0x8000_0030));
    assert!(parse_player_killed(&[3, 0, b'a']).is_err());
    let mut w = Writer::new();
    w.u32(3600).string16("Speedhacking");
    assert_eq!(
        parse_account_banned(&w.finish()).unwrap(),
        (3600, "Speedhacking".to_string())
    );
    // The reason is optional.
    assert_eq!(
        parse_account_banned(&60u32.to_le_bytes()).unwrap(),
        (60, String::new())
    );
    let mut w = Writer::new();
    w.u32(0x8000_0030).u32(0x51).f32(2.0);
    assert_eq!(
        parse_play_effect(&w.finish()).unwrap(),
        (0x8000_0030, 0x51, 2.0)
    );
}

#[test]
fn names_follow_ace() {
    assert_eq!(
        opcode::name(opcode::UPDATE_POSITION),
        Some("UpdatePosition")
    );
    assert_eq!(opcode::name(opcode::PLAY_EFFECT), Some("PlayEffect"));
    assert_eq!(opcode::name(0x1234), None);
    assert_eq!(
        event::name(event::FELLOWSHIP_FELLOW_UPDATE_DONE),
        Some("FellowshipFellowUpdateDone")
    );
    assert_eq!(
        event::name(event::WEENIE_ERROR_WITH_STRING),
        Some("WeenieErrorWithString")
    );
    assert_eq!(event::name(0x0001), None);
    assert_eq!(fellowship_update_request(true), vec![1, 0, 0, 0]);
}
