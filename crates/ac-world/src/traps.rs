//! Traps: pressure plates whose linked object casts a spell on whoever trips one (Nullify All Magic
//! Other, Mana Blight, the bolts; `PressurePlate.cs`, `Player.cs:370-386`), and hotspots that hurt
//! whoever touches them (acid pools, fire; `Hotspot.cs:39-80`). The server never says what a plate
//! is linked to or what a hotspot does; `data/traps.csv` is the copy (`reference/scripts/data/traps.sh`).

use glam::Vec3;
use std::sync::OnceLock;

/// One trap as placed: a plate, or a hotspot that hurts.
#[derive(Clone, Debug, PartialEq)]
pub struct Trap {
    /// Cell it stands in, and its world position.
    pub cell: u32,
    pub at: Vec3,
    /// Its Setup, whose spheres times `scale` are what must not be touched.
    pub setup: u32,
    pub scale: f32,
    /// What its trap casts, `;`-separated; a hotspot's name and "(hotspot)".
    pub spells: String,
}

/// A trap's shape by Setup, before scale: what must not be touched.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Shape {
    /// A sphere: its centre's height above the trap's origin, and its radius (metres).
    Sphere(f32, f32),
    /// An upright cylinder standing on the origin: radius and height (metres).
    Cylinder(f32, f32),
}

/// Shapes read off the portal DAT with `acdat decode` (spheres and cylinder spheres of each Setup).
/// A plate not here is taken as the larger plate; a hotspot with no shape of its own as a disc.
const SHAPES: [(u32, Shape); 26] = [
    (0x0200_0450, Shape::Sphere(1.125, 1.125)),
    (0x0200_025A, Shape::Sphere(0.5, 0.5)),
    (0x0200_0638, Shape::Sphere(0.0, 0.5)),
    (0x0200_08D3, Shape::Sphere(0.0, 0.5)),
    (0x0200_08D8, Shape::Sphere(0.0, 0.5)),
    (0x0200_08D9, Shape::Sphere(0.0, 0.5)),
    (0x0200_05AE, Shape::Sphere(0.55, 0.55)),
    (0x0200_08C2, Shape::Sphere(0.55, 0.55)),
    (0x0200_08CE, Shape::Sphere(0.55, 0.55)),
    (0x0200_0639, Shape::Sphere(4.75, 4.75)),
    (0x0200_06F9, Shape::Sphere(0.0, 1.265)),
    (0x0200_03CB, Shape::Sphere(0.28, 0.24)),
    // The acid and mag-ma pools: 6.7 m across a 1.5 m depth.
    (0x0200_07A7, Shape::Cylinder(6.714, 1.476)),
    (0x0200_0DA8, Shape::Cylinder(6.714, 1.476)),
    (0x0200_0DA9, Shape::Cylinder(6.714, 1.476)),
    (0x0200_0BF4, Shape::Cylinder(6.714, 1.476)),
    (0x0200_0BF5, Shape::Cylinder(6.714, 1.476)),
    (0x0200_068F, Shape::Cylinder(6.714, 1.476)),
    (0x0200_0999, Shape::Cylinder(6.714, 1.476)),
    (0x0200_0E6E, Shape::Cylinder(6.714, 1.476)),
    (0x0200_0469, Shape::Cylinder(0.5, 0.1)),
    // Four small spheres 1.25 m out: one disc round them.
    (0x0200_16F3, Shape::Cylinder(1.5, 0.5)),
    // No spheres of their own (flames, firespurts, vents): a disc of their scale.
    (0x0200_0485, Shape::Cylinder(1.0, 0.5)),
    (0x0200_06A9, Shape::Cylinder(1.0, 0.5)),
    (0x0200_06A8, Shape::Cylinder(1.0, 0.5)),
    (0x0200_0840, Shape::Cylinder(1.0, 0.5)),
];

impl Trap {
    fn shape(&self) -> Shape {
        let hotspot = self.spells.ends_with("(hotspot)");
        let unknown = if hotspot {
            Shape::Cylinder(1.0, 0.5)
        } else {
            Shape::Sphere(1.125, 1.125)
        };
        SHAPES
            .iter()
            .find(|s| s.0 == self.setup)
            .map_or(unknown, |s| s.1)
    }

    /// The sphere the server's sweep must not touch: its centre, world space, and radius. A
    /// cylinder's is its middle and its radius, for what reads it on the flat.
    pub fn sphere(&self) -> (Vec3, f32) {
        let k = self.scale;
        match self.shape() {
            Shape::Sphere(h, r) => (self.at + Vec3::Z * h * k, r * k),
            Shape::Cylinder(r, h) => (self.at + Vec3::Z * h * k / 2.0, r * k),
        }
    }

    /// How far out from its centre it reaches on the flat (metres).
    pub fn reach(&self) -> f32 {
        self.sphere().1
    }

    /// The height (world z) of its bottom.
    pub fn bottom(&self) -> f32 {
        match self.shape() {
            Shape::Sphere(..) => {
                let (c, r) = self.sphere();
                c.z - r
            }
            Shape::Cylinder(..) => self.at.z,
        }
    }

    /// The height (world z) of its top.
    pub fn top(&self) -> f32 {
        match self.shape() {
            Shape::Sphere(..) => {
                let (c, r) = self.sphere();
                c.z + r
            }
            Shape::Cylinder(_, h) => self.at.z + h * self.scale,
        }
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

/// The traps standing in landblock `lb` (any cell of it will do): a slice of the sorted table.
pub fn in_landblock(lb: u32) -> impl Iterator<Item = &'static Trap> {
    let lb = lb & 0xFFFF_0000;
    let all = all();
    let from = all.partition_point(|t| t.cell < lb);
    let to = all.partition_point(|t| t.cell <= lb | 0xFFFF);
    all[from..to].iter()
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
