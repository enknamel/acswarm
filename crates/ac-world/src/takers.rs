//! NPCs that take anything handed to them (AiAcceptEverything, PropertyBool 79), and where.
//! The client never sees bool 79 (not an assessment property), so `data/takers.csv` is a copy
//! (`reference/scripts/data/takers.sh`). Such an NPC takes the whole amount asked in one give,
//! and anything but a sticky-attuned item (Player_Inventory.cs:3435-3443).

use glam::{Vec2, Vec3};
use std::sync::OnceLock;

use crate::Place;

/// One placement of an NPC that takes anything.
#[derive(Clone, Debug, PartialEq)]
pub struct Taker {
    pub wcid: u32,
    pub name: String,
    pub place: Place,
}

impl Taker {
    pub fn xy(&self) -> Vec2 {
        self.place.at.truncate()
    }
}

const DATA: &str = include_str!("../data/takers.csv");

fn parse(text: &str) -> Vec<Taker> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(',').map(str::trim).collect();
        if f.len() != 6 {
            continue;
        }
        let (Ok(wcid), Ok(cell)) = (f[0].parse::<u32>(), u32::from_str_radix(f[2], 16)) else {
            continue;
        };
        let num = |s: &str| s.parse::<f32>().ok();
        let (Some(x), Some(y), Some(z)) = (num(f[3]), num(f[4]), num(f[5])) else {
            continue;
        };
        out.push(Taker {
            wcid,
            name: f[1].to_string(),
            place: Place {
                at: crate::landblock_origin(cell) + Vec3::new(x, y, z),
                cell,
            },
        });
    }
    out
}

/// Every placement, parsed on first use.
pub fn all() -> &'static [Taker] {
    static TAKERS: OnceLock<Vec<Taker>> = OnceLock::new();
    TAKERS.get_or_init(|| parse(DATA))
}

/// Whether an NPC of this weenie class takes anything handed to it, whole stacks at a time.
pub fn takes_anything(wcid: u32) -> bool {
    all().iter().any(|t| t.wcid == wcid)
}

/// The placement nearest `from` (world xy) among those `fits` lets through.
pub fn nearest(from: Vec2, fits: impl Fn(&Taker) -> bool) -> Option<&'static Taker> {
    all()
        .iter()
        .filter(|t| fits(t))
        .min_by(|a, b| a.xy().distance(from).total_cmp(&b.xy().distance(from)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_reads_and_places_every_taker() {
        let all = all();
        assert!(all.len() >= 30, "{} takers", all.len());
        assert!(all.iter().all(|t| !t.name.is_empty() && t.wcid != 0));
        assert!(all.iter().any(|t| t.name == "Town Crier"));
        // A short cell is a landblock with leading zeros left off: 0x00AF0118.
        assert!(all.iter().any(|t| t.place.cell == 0x00AF_0118));
    }

    #[test]
    fn a_town_crier_takes_anything_and_terese_does_not() {
        // Town Crier 5772 stands in Holtburg's neighbours; Terese (11637) takes only her seeds.
        assert!(takes_anything(5772));
        assert!(!takes_anything(11637));
    }

    #[test]
    fn the_nearest_taker_is_the_nearest_placement() {
        let t = all()[0].clone();
        let found = nearest(t.xy(), |_| true).unwrap();
        assert_eq!(found.xy().distance(t.xy()), 0.0);
        assert!(nearest(t.xy(), |_| false).is_none());
    }

    #[test]
    fn bad_lines_are_skipped() {
        let v = parse("# c\n\nnope,x\n7,A Barrel,A9B40019,1.0,2.0,3.0\nx,Bad,ZZ,1,2,3\n");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].wcid, 7);
        assert_eq!(v[0].place.cell, 0xA9B4_0019);
    }
}
