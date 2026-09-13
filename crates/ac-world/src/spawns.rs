//! What spawns where: every creature the server's generators make, by the
//! cell they stand in, outdoors and in every dungeon room.
//!
//! A player choosing where to hunt wants to know what lives there before
//! walking over: a graveyard of level 30 undead is worth the trip, the
//! drudges on the road past it are not. The client is never told -- spawns
//! are the server's, and a room's creatures are only described once the
//! character can see into it -- so `data/spawns.csv.gz` is a copy of the
//! list (see `reference/scripts/data/spawns.sh`), like the hunting grounds
//! and the portals.
//!
//! One row per cell, creature and level: where the cell's generators for
//! that creature stand on average, and how many generator entries make it.
//! About a hundred thousand rows, so the table is kept gzipped and read the
//! first time it is asked for.

use std::collections::HashMap;
use std::io::Read;
use std::ops::Range;
use std::sync::OnceLock;

use glam::Vec3;

/// One creature that spawns in one cell.
#[derive(Clone, Debug, PartialEq)]
pub struct Spawn {
    /// The cell its generators stand in: an outdoor cell, or a dungeon
    /// room.
    pub cell: u32,
    /// World position of those generators, on average.
    pub at: Vec3,
    pub level: u32,
    /// How many generator entries make it there: a rough measure of how
    /// many of it there are at once.
    pub count: u32,
    pub name: String,
}

impl Spawn {
    /// Whether it spawns in a dungeon room (or a building's cell) rather
    /// than out in the open.
    pub fn indoors(&self) -> bool {
        self.cell & 0xFFFF >= 0x100
    }
}

/// The creatures of one cell, gathered: where the cell's generators stand
/// on average, and what they make. One of these is a mark on the map.
#[derive(Clone, Debug, PartialEq)]
pub struct Place<'a> {
    pub cell: u32,
    pub at: Vec3,
    pub spawns: Vec<&'a Spawn>,
}

impl Place<'_> {
    /// The lowest and highest level of what spawns here.
    pub fn levels(&self) -> (u32, u32) {
        let lo = self.spawns.iter().map(|s| s.level).min().unwrap_or(0);
        let hi = self.spawns.iter().map(|s| s.level).max().unwrap_or(0);
        (lo, hi)
    }
}

const DATA: &[u8] = include_bytes!("../data/spawns.csv.gz");

/// The rows of the table's text: `cell,x,y,z,level,count,name`, the cell
/// in hex and the position local to its landblock. Comment and malformed
/// lines are skipped.
pub fn parse(text: &str) -> Vec<Spawn> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.splitn(7, ',').collect();
        if f.len() != 7 {
            continue;
        }
        let (Ok(cell), Ok(x), Ok(y), Ok(z), Ok(level), Ok(count)) = (
            u32::from_str_radix(f[0], 16),
            f[1].parse::<f32>(),
            f[2].parse::<f32>(),
            f[3].parse::<f32>(),
            f[4].parse::<u32>(),
            f[5].parse::<u32>(),
        ) else {
            continue;
        };
        out.push(Spawn {
            cell,
            at: crate::landblock_origin(cell) + Vec3::new(x, y, z),
            level,
            count,
            name: f[6].replace(';', ","),
        });
    }
    out
}

struct Table {
    /// Sorted by landblock, then cell.
    spawns: Vec<Spawn>,
    /// Where each landblock's rows lie in `spawns`.
    blocks: HashMap<u32, Range<usize>>,
}

fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut text = String::new();
        if let Err(e) = flate2::read::GzDecoder::new(DATA).read_to_string(&mut text) {
            tracing::warn!("spawns: the table would not unpack: {e}");
        }
        index(parse(&text))
    })
}

fn index(mut spawns: Vec<Spawn>) -> Table {
    spawns.sort_by_key(|s| s.cell);
    let mut blocks: HashMap<u32, Range<usize>> = HashMap::new();
    let mut start = 0;
    for i in 1..=spawns.len() {
        let block = spawns[start].cell & 0xFFFF_0000;
        if i == spawns.len() || spawns[i].cell & 0xFFFF_0000 != block {
            blocks.insert(block, start..i);
            start = i;
        }
    }
    Table { spawns, blocks }
}

/// Every spawn in the table.
pub fn all() -> &'static [Spawn] {
    &table().spawns
}

/// The spawns of one landblock (`xxyy0000`; the low word is ignored),
/// outdoors and in its dungeon rooms.
pub fn in_block(landblock: u32) -> &'static [Spawn] {
    let t = table();
    t.blocks
        .get(&(landblock & 0xFFFF_0000))
        .map_or(&[], |r| &t.spawns[r.clone()])
}

/// `spawns` gathered one place to a cell, in the order given (a
/// landblock's rows come sorted by cell).
pub fn by_cell(spawns: &[Spawn]) -> Vec<Place<'_>> {
    let mut out: Vec<Place> = Vec::new();
    for s in spawns {
        match out.last_mut() {
            Some(p) if p.cell == s.cell => p.spawns.push(s),
            _ => out.push(Place {
                cell: s.cell,
                at: s.at,
                spawns: vec![s],
            }),
        }
    }
    for p in &mut out {
        let n = p.spawns.len() as f32;
        p.at = p.spawns.iter().map(|s| s.at).sum::<Vec3>() / n;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_is_a_creature_in_a_cell_at_a_world_position() {
        let text = "# comment\n\
                    A9B40024,100,50,60,12,3,Drudge Skulker\n\
                    1F6012F,106.7,-207.8,-18,20,1,Lich\n\
                    1F6012F,111.1,-208.6,-18,8,2,Undead\n\
                    not,a,row\n\
                    A9B40025,1,2,3,5,1,Mosswart; the Elder\n";
        let rows = parse(text);
        assert_eq!(rows.len(), 4);
        let skulker = &rows[0];
        assert_eq!(skulker.cell, 0xA9B4_0024);
        assert_eq!(
            skulker.at,
            crate::landblock_origin(0xA9B4_0024) + Vec3::new(100.0, 50.0, 60.0)
        );
        assert_eq!((skulker.level, skulker.count), (12, 3));
        assert!(!skulker.indoors());
        // A dungeon's cell, written without its leading zero.
        assert_eq!(rows[1].cell, 0x01F6_012F);
        assert!(rows[1].indoors());
        // Commas in a name are kept as semicolons in the file.
        assert_eq!(rows[3].name, "Mosswart, the Elder");

        let t = index(rows);
        assert_eq!(t.blocks.len(), 2);
        let dungeon = &t.spawns[t.blocks[&0x01F6_0000].clone()];
        let places = by_cell(dungeon);
        assert_eq!(places.len(), 1);
        assert_eq!(places[0].spawns.len(), 2);
        assert_eq!(places[0].levels(), (8, 20));
    }

    #[test]
    fn the_holtburg_dungeon_has_its_undead() {
        let rooms = in_block(0x01F6_0000);
        assert!(
            !rooms.is_empty(),
            "the table unpacked and holds the dungeon"
        );
        assert!(rooms.iter().all(|s| s.cell & 0xFFFF_0000 == 0x01F6_0000));
        assert!(rooms
            .iter()
            .any(|s| s.name == "Lich" && s.level == 20 && s.cell == 0x01F6_012F));
        // Nothing for a block with no generators, and no panic either.
        assert!(in_block(0xFFFF_0000).is_empty());
    }
}
