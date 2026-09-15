use crate::wire::{Reader, Truncated};

/// ViewContents (0x0196): a container's item list `(guid, container type)`.
pub fn parse_view_contents(body: &[u8]) -> Result<(u32, Vec<(u32, u32)>), Truncated> {
    let mut r = Reader::new(body);
    let container = r.u32()?;
    let n = r.u32()? as usize;
    let mut items = Vec::with_capacity(n.min(256));
    for _ in 0..n {
        items.push((r.u32()?, r.u32()?));
    }
    Ok((container, items))
}
