//! The fixed things in the world a player navigates to: lifestones,
//! vendors and the standing NPCs.
//!
//! Like the portals, these are server-side objects and so are not in the
//! client's own data files; `data/landmarks.csv` is a copy of the list
//! (see `reference/scripts/data/landmarks.sh`). The map searches it, so
//! a shop or a lifestone can be found and travelled to without having
//! stood next to it first.

use glam::{Vec2, Vec3};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Lifestone,
    Vendor,
    Npc,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Lifestone => "lifestone",
            Kind::Vendor => "vendor",
            Kind::Npc => "npc",
        }
    }

    /// Whether someone to talk to may stand there. A shopkeeper does. An
    /// npc row often is someone, but as often a Wailing Statue, an
    /// Exploration Marker or a generator, so whether to speak is decided
    /// by what is found there. A lifestone is only somewhere to go.
    pub fn may_be_someone(self) -> bool {
        matches!(self, Kind::Vendor | Kind::Npc)
    }

    fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "lifestone" => Kind::Lifestone,
            "vendor" => Kind::Vendor,
            "npc" => Kind::Npc,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Landmark {
    pub kind: Kind,
    pub name: String,
    pub cell: u32,
    /// World position.
    pub at: Vec3,
}

impl Landmark {
    pub fn xy(&self) -> Vec2 {
        self.at.truncate()
    }

    /// Whether it stands outdoors (a walk can reach it).
    pub fn outdoors(&self) -> bool {
        self.cell & 0xFFFF < 0x100
    }
}

const DATA: &str = include_str!("../data/landmarks.csv");

fn parse(text: &str) -> Vec<Landmark> {
    let mut out = Vec::with_capacity(6600);
    for line in text.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        if f.len() != 6 {
            continue;
        }
        let (Some(kind), Some(cell)) = (Kind::parse(f[0]), u32::from_str_radix(f[2], 16).ok())
        else {
            continue;
        };
        let num = |s: &str| s.parse::<f32>().ok();
        let (Some(x), Some(y), Some(z)) = (num(f[3]), num(f[4]), num(f[5])) else {
            continue;
        };
        out.push(Landmark {
            kind,
            name: f[1].to_string(),
            cell,
            at: crate::landblock_origin(cell) + Vec3::new(x, y, z),
        });
    }
    out
}

/// Every landmark, parsed on first use.
pub fn all() -> &'static [Landmark] {
    static LANDMARKS: OnceLock<Vec<Landmark>> = OnceLock::new();
    LANDMARKS.get_or_init(|| parse(DATA))
}

/// Landmarks within `radius` metres of `world`, nearest first.
pub fn near(world: Vec2, radius: f32) -> Vec<&'static Landmark> {
    let mut v: Vec<&Landmark> = all()
        .iter()
        .filter(|l| l.xy().distance(world) <= radius)
        .collect();
    v.sort_by(|a, b| a.xy().distance(world).total_cmp(&b.xy().distance(world)));
    v
}

/// Landmarks whose name contains `needle`, case-insensitively; when
/// `from` is given, nearest first.
pub fn search(needle: &str, from: Option<Vec2>) -> Vec<&'static Landmark> {
    let needle = needle.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut v: Vec<&Landmark> = all()
        .iter()
        .filter(|l| l.name.to_lowercase().contains(&needle))
        .collect();
    if let Some(from) = from {
        v.sort_by(|a, b| a.xy().distance(from).total_cmp(&b.xy().distance(from)));
    }
    v
}

/// The landmark called exactly `name` (case-insensitively), the one
/// nearest `from` when several share it.
///
/// Exact, unlike [`search`]: a script that says "Holtburg" means the
/// town, and a part of a name would find the Holtburg Town Crier first.
pub fn named(name: &str, from: Vec2) -> Option<&'static Landmark> {
    let want = name.trim().to_lowercase();
    if want.is_empty() {
        return None;
    }
    all()
        .iter()
        .filter(|l| l.name.to_lowercase() == want)
        .min_by(|a, b| a.xy().distance(from).total_cmp(&b.xy().distance(from)))
}

/// The lifestone nearest `world`.
pub fn nearest_lifestone(world: Vec2) -> Option<&'static Landmark> {
    all()
        .iter()
        .filter(|l| l.kind == Kind::Lifestone)
        .min_by(|a, b| a.xy().distance(world).total_cmp(&b.xy().distance(world)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_reads_and_finds_places() {
        let all = all();
        assert!(all.len() > 6000, "{} landmarks", all.len());
        let holtburg = crate::towns::find("Holtburg").unwrap().world_xy();
        // Holtburg has a lifestone and shops.
        let ls = nearest_lifestone(holtburg).unwrap();
        assert!(
            ls.xy().distance(holtburg) < 400.0,
            "nearest lifestone is {:.0} m away",
            ls.xy().distance(holtburg)
        );
        let around = near(holtburg, 300.0);
        assert!(
            around.iter().any(|l| l.kind == Kind::Vendor),
            "no vendor near Holtburg"
        );
        // Named search, nearest first.
        let smiths = search("blacksmith", Some(holtburg));
        assert!(!smiths.is_empty(), "no blacksmith anywhere");
        assert!(search("", None).is_empty());
        assert_eq!(Kind::Vendor.label(), "vendor");
    }

    #[test]
    fn a_person_upstairs_keeps_her_cell_and_her_height() {
        // Archmage Cindrue keeps a shop on the upper floor of a house in
        // Holtburg. Travelling to her flat position stopped three metres
        // under her, on the ground floor inside the door.
        let holtburg = crate::towns::find("Holtburg").unwrap().world_xy();
        let l = named("archmage cindrue", holtburg).expect("Archmage Cindrue is a landmark");
        assert_eq!(l.name, "Archmage Cindrue");
        assert_eq!(l.cell, 0xA9B4_011B);
        assert!(!l.outdoors());
        assert!(l.kind.may_be_someone());
        assert!((l.at.z - 69.0).abs() < 0.5, "{:?}", l.at);
        // Only the whole name: a part of one is not an exact match.
        assert!(named("Archmage Cind", holtburg).is_none());
        assert!(!Kind::Lifestone.may_be_someone());
    }

    #[test]
    fn bad_lines_are_skipped() {
        let v =
            parse("# c\n\nnope,x\nvendor,Sam the Smith,A9B40019,1.0,2.0,3.0\nghost,X,1,1,1,1\n");
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].name, "Sam the Smith");
        assert_eq!(v[0].kind, Kind::Vendor);
        assert!(v[0].outdoors());
    }
}
