//! Creatures no spell but a projectile affects: NonProjectileMagicImmune (PropertyBool 103), a
//! vulnerability, Magic Yield or imperil answered "You fail to affect" (Player_Magic.cs:1109).
//! `data/magic_immune.csv` is the copy (`reference/scripts/data/magic_immune.sh`).

use std::sync::OnceLock;

const DATA: &str = include_str!("../data/magic_immune.csv");

fn kinds() -> &'static [u32] {
    static ALL: OnceLock<Vec<u32>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut out: Vec<u32> = DATA
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .filter_map(|l| l.split(',').next()?.parse().ok())
            .collect();
        out.sort_unstable();
        out
    })
}

/// Whether creatures of `wcid` take no spell but a projectile.
pub fn is_magic_immune(wcid: u32) -> bool {
    kinds().binary_search(&wcid).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_olthoi_slashers_and_slayers_take_no_debuff() {
        for wcid in [31007, 31008, 35146, 35150, 87830] {
            assert!(is_magic_immune(wcid), "{wcid}");
        }
        // A Drudge Sneaker is debuffed as ever.
        assert!(!is_magic_immune(940));
    }
}
