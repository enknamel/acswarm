use super::*;

#[test]
fn room_line_round_trips() {
    // An inbound line the way ACE writes it.
    let mut payload = Writer::new();
    payload.u32(GENERAL);
    write_wstring(&mut payload, "+Admin");
    write_wstring(&mut payload, "hello général");
    payload.u32(0x0C).u32(0x5000_0002).u32(0).u32(GENERAL);
    let payload = payload.finish();
    let mut blob = Writer::new();
    blob.u32(EVENT_BINARY)
        .u32(1)
        .u32(1)
        .u32(0x000B_00B5)
        .u32(1)
        .u32(0x000B_00B5)
        .u32(0)
        .u32(payload.len() as u32)
        .bytes(&payload);
    let blob = blob.finish();
    let mut w = Writer::new();
    w.u32(blob.len() as u32).bytes(&blob);
    let line = parse(&w.finish()).unwrap().unwrap();
    assert_eq!(line.sender, "+Admin");
    assert_eq!(line.text, "hello général");
    assert_eq!((line.sender_id, line.kind), (GENERAL, KIND));
    assert_eq!(name(line.sender_id), "General");

    // Our own request has the opcode, the sizes and the room.
    let msg = encode(TRADE, 0x5000_0001, "wts bow", 7);
    let mut r = Reader::new(&msg);
    assert_eq!(r.u32().unwrap(), super::super::opcode::TURBINE_CHAT);
    let size = r.u32().unwrap() as usize;
    assert_eq!(size, msg.len() - 8);
    assert_eq!(r.u32().unwrap(), REQUEST_BINARY);
    assert_eq!(r.u32().unwrap(), SEND_TO_ROOM_BY_ID);
    assert!(parse(&msg[4..]).unwrap().is_none());
    assert_eq!(from_prefix("g"), Some(GENERAL));
    assert_eq!(from_prefix("cg"), Some(GENERAL));
    assert_eq!(from_prefix("soc"), Some(SOCIETY));
    assert_eq!(from_prefix("gu"), Some(ALLEGIANCE));
    // `rp` is the reply retail registered, not the Roleplay room.
    assert_eq!(from_prefix("rp"), None);
    assert_eq!(from_prefix("crp"), Some(ROLEPLAY));
    assert_eq!(chat_type(0x3300), ALLEGIANCE);
}
