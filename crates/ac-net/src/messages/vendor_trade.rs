use crate::wire::Writer;

/// Buy/Sell body: vendor, count, then `(amount, guid)` per item, and the
/// alternate currency id (0 for pyreals).
pub fn trade(vendor: u32, items: &[(u32, i32)]) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(vendor).u32(items.len() as u32);
    for (guid, amount) in items {
        w.i32(*amount).u32(*guid);
    }
    w.u32(0);
    w.finish()
}
