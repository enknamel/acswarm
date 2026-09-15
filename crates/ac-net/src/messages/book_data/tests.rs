use super::*;
use crate::wire::Writer;

#[test]
fn parses_book_and_page() {
    let mut w = Writer::new();
    w.u32(0x8000_0010).u32(10).u32(2).u32(1000).u32(2);
    for (name, text) in [("Asheron", Some("Page one.")), ("", None)] {
        w.u32(1)
            .string16(name)
            .string16("beer good")
            .u32(0xFFFF_0002);
        match text {
            Some(t) => {
                w.u32(1).u32(0).string16(t);
            }
            None => {
                w.u32(0).u32(0);
            }
        }
    }
    w.string16("BASICS OF MAGIC").u32(0).string16("Asheron");
    let b = BookData::parse(&w.finish()).unwrap();
    assert_eq!(b.inscription, "BASICS OF MAGIC");
    assert_eq!(b.pages.len(), 2);
    assert_eq!(b.pages[0].text.as_deref(), Some("Page one."));
    assert!(b.pages[1].text.is_none());
    let mut w = Writer::new();
    w.u32(0x8000_0010)
        .u32(1)
        .u32(1)
        .string16("Asheron")
        .string16("x")
        .u32(0xFFFF_0002)
        .u32(1)
        .u32(0)
        .string16("Page two.");
    let (g, i, p) = BookData::parse_page(&w.finish()).unwrap();
    assert_eq!((g, i), (0x8000_0010, 1));
    assert_eq!(p.text.as_deref(), Some("Page two."));
}
