//! Creatures that cast rings: projectiles out in every direction from the caster, each bursting
//! once past its spell's range from where it spawned (`WorldObject_Tick.cs:351-362`). A monster's
//! spell book is server data, so `data/ring_casters.csv` is the copy
//! (`reference/scripts/data/ring_casters.sh`); keyed by weenie class id.

use std::collections::HashMap;
use std::sync::OnceLock;

/// One ring a creature may cast.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ring {
    pub spell: u32,
    /// Metres a projectile flies from its spawn before it bursts (`BaseRangeConstant`).
    pub range: f32,
    /// Metres a second the projectiles fly (`MaximumVelocity` of the projectile weenie).
    pub speed: f32,
}

const DATA: &str = include_str!("../data/ring_casters.csv");

fn parse(text: &str) -> HashMap<u32, Vec<Ring>> {
    let mut out: HashMap<u32, Vec<Ring>> = HashMap::new();
    for line in text.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        if f.len() != 6 {
            continue;
        }
        let (Ok(wcid), Ok(spell), Ok(range), Ok(speed)) = (
            f[0].parse::<u32>(),
            f[1].parse::<u32>(),
            f[2].parse::<f32>(),
            f[3].parse::<f32>(),
        ) else {
            continue;
        };
        out.entry(wcid).or_default().push(Ring {
            spell,
            range,
            speed,
        });
    }
    out
}

fn all() -> &'static HashMap<u32, Vec<Ring>> {
    static RINGS: OnceLock<HashMap<u32, Vec<Ring>>> = OnceLock::new();
    RINGS.get_or_init(|| parse(DATA))
}

/// The rings creature `wcid` may cast; empty for one that casts none.
pub fn rings_of(wcid: u32) -> &'static [Ring] {
    all().get(&wcid).map_or(&[], Vec::as_slice)
}

/// The worst any ring of `wcid` can do: the longest range and the fastest projectiles, which
/// may be two rings. A cast seen starting does not say which spell it is.
pub fn widest_ring(wcid: u32) -> Option<Ring> {
    widest(rings_of(wcid))
}

/// The longest range of `rings` with the fastest flight among them, named for the longest.
fn widest(rings: &[Ring]) -> Option<Ring> {
    rings.iter().copied().reduce(|a, b| Ring {
        spell: if b.range > a.range { b.spell } else { a.spell },
        range: a.range.max(b.range),
        speed: a.speed.max(b.speed),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_reads_and_holds_the_chakron_inquisitors() {
        assert!(all().len() > 400, "{} ring casters", all().len());
        // Virindi Inquisitor (10814): Horizon's Blades and Cassius' Ring of Fire, 4.5 m at 2 m/s.
        let inquisitor = rings_of(10814);
        assert_eq!(inquisitor.len(), 2);
        assert!(inquisitor.iter().any(|r| r.spell == 1785));
        let widest = widest_ring(10814).unwrap();
        assert_eq!((widest.range, widest.speed), (4.5, 2.0));
        // A Drudge Skulker casts no ring.
        assert!(rings_of(8592).is_empty());
        assert!(widest_ring(8592).is_none());
        assert!(all()
            .values()
            .flatten()
            .all(|r| r.range >= 0.0 && r.speed > 0.0));
    }

    #[test]
    fn the_widest_ring_takes_the_longest_range_and_the_fastest_flight() {
        let t = parse("# c\n\nbad,line\n7,1,4.5,15,A,B\n7,2,8,2,A,C\n9,3,x,2,A,D\n");
        assert_eq!(t.len(), 1);
        assert_eq!(
            widest(&t[&7]),
            Some(Ring {
                spell: 2,
                range: 8.0,
                speed: 15.0
            })
        );
    }
}
