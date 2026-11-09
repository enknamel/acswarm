//! Trap plates: pressure plates whose linked object casts a spell on whoever trips one (Nullify
//! All Magic Other, Mana Blight, the bolts). The server fires one when its sweep of the player
//! touches the plate (ACE `PressurePlate.cs`, `Player.cs:370-386`), and never says what a plate
//! is linked to; `data/traps.csv` is the copy (`reference/scripts/data/traps.sh`).

use glam::Vec3;
use std::sync::OnceLock;

/// One trap plate as placed.
#[derive(Clone, Debug, PartialEq)]
pub struct Trap {
    /// Cell it stands in, and its world position.
    pub cell: u32,
    pub at: Vec3,
    /// Its Setup, whose spheres times `scale` are what must not be touched.
    pub setup: u32,
    pub scale: f32,
    /// What its trap casts, `;`-separated.
    pub spells: String,
}

/// A plate's collision sphere by Setup, before scale: centre height and radius (metres), read
/// off the portal DAT with `acdat decode`. Any other is taken as the larger, to be sure.
const SPHERES: [(u32, f32, f32); 2] = [(0x0200_0450, 1.125, 1.125), (0x0200_025A, 0.5, 0.5)];

impl Trap {
    /// The sphere the server's sweep must not touch: its centre, world space, and radius.
    pub fn sphere(&self) -> (Vec3, f32) {
        let (h, r) = SPHERES
            .iter()
            .find(|s| s.0 == self.setup)
            .map_or((1.125, 1.125), |s| (s.1, s.2));
        (self.at + Vec3::Z * h * self.scale, r * self.scale)
    }

    /// The height (world z) of the sphere's top.
    pub fn top(&self) -> f32 {
        let (c, r) = self.sphere();
        c.z + r
    }
}

const DATA: &str = include_str!("../data/traps.csv");

fn parse(text: &str) -> Vec<Trap> {
    let mut out = Vec::with_capacity(600);
    for line in text.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        if f.len() != 7 {
            continue;
        }
        let hex = |s: &str| u32::from_str_radix(s, 16).ok();
        let num = |s: &str| s.parse::<f32>().ok();
        let (Some(cell), Some(setup)) = (hex(f[0]), hex(f[4])) else {
            continue;
        };
        let (Some(x), Some(y), Some(z), Some(scale)) = (num(f[1]), num(f[2]), num(f[3]), num(f[5]))
        else {
            continue;
        };
        out.push(Trap {
            cell,
            at: crate::landblock_origin(cell) + Vec3::new(x, y, z),
            setup,
            scale,
            spells: f[6].to_string(),
        });
    }
    out.sort_by_key(|t| t.cell);
    out
}

/// Every trap plate, parsed on first use, by cell.
pub fn all() -> &'static [Trap] {
    static TRAPS: OnceLock<Vec<Trap>> = OnceLock::new();
    TRAPS.get_or_init(|| parse(DATA))
}

/// The trap plates standing in landblock `lb` (any cell of it will do).
pub fn in_landblock(lb: u32) -> impl Iterator<Item = &'static Trap> {
    let lb = lb & 0xFFFF_0000;
    all().iter().filter(move |t| t.cell & 0xFFFF_0000 == lb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_reads_and_holds_the_buff_strippers() {
        let all = all();
        assert!(all.len() > 500, "{} trap plates", all.len());
        assert!(all
            .iter()
            .any(|t| t.spells.contains("Nullify All Magic Other")));
        assert!(all.iter().any(|t| t.spells.contains("Mana Blight")));
        assert!(all.iter().all(|t| t.scale > 0.0));
        // The tall ones are scaled threefold: 6.75 m to the top.
        assert!(all.iter().any(|t| (t.top() - t.at.z - 6.75).abs() < 0.01));
        let first = &all[0];
        assert!(in_landblock(first.cell).any(|t| t == first));
    }

    #[test]
    fn bad_lines_are_skipped() {
        let v = parse("# c\n\nbad,line\n1080100,53.37,-139.09,-30,2000450,1,Acid Stream I\n");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].setup, 0x0200_0450);
    }
}
