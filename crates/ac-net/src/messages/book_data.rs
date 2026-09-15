use crate::wire::{Reader, Truncated};

/// One page of a book (ACE `PageData`): who wrote it and, once read,
/// its text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BookPage {
    pub author_id: u32,
    pub author: String,
    pub text: Option<String>,
}

/// BookDataResponse (game event 0x00B4), the answer to using a book,
/// sign or plaque: the book, its page limits, the pages (texts only when
/// included; BookPageData 0x00AE fetches one), the inscription and the
/// scribe.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BookData {
    pub guid: u32,
    pub max_pages: u32,
    pub max_chars: u32,
    pub pages: Vec<BookPage>,
    pub inscription: String,
    pub author: String,
}

fn read_page(r: &mut Reader) -> Result<BookPage, Truncated> {
    let author_id = r.u32()?;
    let author = r.string16()?;
    let _account = r.string16()?;
    let _flags = r.u32()?;
    let included = r.u32()? != 0;
    let _ignore_author = r.u32()?;
    let text = if included { Some(r.string16()?) } else { None };
    Ok(BookPage {
        author_id,
        author,
        text,
    })
}

impl BookData {
    pub fn parse(body: &[u8]) -> Result<Self, Truncated> {
        let mut r = Reader::new(body);
        let guid = r.u32()?;
        let max_pages = r.u32()?;
        let _num_pages = r.u32()?;
        let max_chars = r.u32()?;
        let n = r.u32()? as usize;
        let mut pages = Vec::with_capacity(n.min(256));
        for _ in 0..n {
            pages.push(read_page(&mut r)?);
        }
        let inscription = r.string16()?;
        let _author_id = r.u32()?;
        let author = r.string16()?;
        Ok(BookData {
            guid,
            max_pages,
            max_chars,
            pages,
            inscription,
            author,
        })
    }

    /// BookPageDataResponse (0x00B8): book guid, page index, the page.
    pub fn parse_page(body: &[u8]) -> Result<(u32, u32, BookPage), Truncated> {
        let mut r = Reader::new(body);
        let guid = r.u32()?;
        let index = r.u32()?;
        let page = read_page(&mut r)?;
        Ok((guid, index, page))
    }
}

#[cfg(test)]
mod tests;
