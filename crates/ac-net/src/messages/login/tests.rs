use super::*;

#[test]
fn login_request_layout() {
    let b = login_request("acct", "pw", 7);
    let mut r = Reader::new(&b);
    assert_eq!(r.string16().unwrap(), "1802");
    let len = r.u32().unwrap() as usize;
    assert_eq!(len, r.remaining().len());
    assert_eq!(r.u32().unwrap(), 2);
    assert_eq!(r.u32().unwrap(), 0);
    assert_eq!(r.u32().unwrap(), 7);
    assert_eq!(r.string16().unwrap(), "acct");
    assert_eq!(r.string16().unwrap(), "");
    let pwlen = r.u32().unwrap() as usize;
    assert_eq!(pwlen, r.remaining().len());
    assert_eq!(r.u8().unwrap(), 2);
    assert_eq!(r.bytes(2).unwrap(), b"pw");
}

#[test]
fn delete_and_restore_bodies() {
    let d = character_delete("acct", 2);
    let mut r = Reader::new(&d);
    assert_eq!(r.u32().unwrap(), opcode::CHARACTER_DELETE);
    assert_eq!(r.string16().unwrap(), "acct");
    assert_eq!(r.u32().unwrap(), 2);
    assert!(r.remaining().is_empty());
    let rs = character_restore(0x5000_0001);
    assert_eq!(rs, [0xD9, 0xF7, 0, 0, 1, 0, 0, 0x50]);
}

#[test]
fn create_response_ok_and_failure() {
    let mut w = Writer::new();
    w.u32(1).u32(0x5000_0002).string16("Bob").u32(0);
    let r = CharacterCreateResponse::parse(&w.buf).unwrap();
    assert_eq!(
        (r.response, r.guid, r.name.as_str()),
        (1, 0x5000_0002, "Bob")
    );
    let r = CharacterCreateResponse::parse(&3u32.to_le_bytes()).unwrap();
    assert_eq!((r.response, r.guid, r.name.as_str()), (3, 0, ""));
}

#[test]
fn character_list_roundtrip() {
    let mut w = Writer::new();
    w.u32(0)
        .u32(1)
        .u32(0x5000_0001)
        .string16("Bob")
        .u32(0)
        .u32(0)
        .u32(11)
        .string16("acct")
        .u32(1)
        .u32(1);
    let cl = CharacterList::parse(&w.buf).unwrap();
    assert_eq!(cl.characters[0].name, "Bob");
    assert_eq!(cl.slot_count, 11);
    assert_eq!(cl.account, "acct");
}
