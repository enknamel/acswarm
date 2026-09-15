use super::*;
use crate::wire::Writer;

#[test]
fn chat_layouts() {
    // HearSpeech: "hi" from "Bob" (guid 0x50000001), speech (2).
    let mut w = Writer::new();
    w.string16("hi").string16("Bob").u32(0x5000_0001).u32(2);
    let l = ChatLine::parse_hear_speech(&w.finish()).unwrap();
    assert_eq!(
        l,
        ChatLine {
            text: "hi".into(),
            sender: "Bob".into(),
            sender_id: 0x5000_0001,
            kind: 2
        }
    );
    let mut w = Writer::new();
    w.string16("yo").string16("Al").u32(9).f32(30.0).u32(2);
    assert_eq!(
        ChatLine::parse_hear_ranged_speech(&w.finish())
            .unwrap()
            .kind,
        2
    );
    let mut w = Writer::new();
    w.string16("motd").i32(0);
    let l = ChatLine::parse_server_message(&w.finish()).unwrap();
    assert_eq!((l.text.as_str(), l.sender.as_str()), ("motd", ""));
    let mut w = Writer::new();
    w.u32(7).string16("Al").string16("waves.");
    let l = ChatLine::parse_emote_text(&w.finish()).unwrap();
    assert_eq!((l.sender_id, l.text.as_str()), (7, "waves."));
    assert!(ChatLine::parse_hear_speech(&[2, 0]).is_err());
}
