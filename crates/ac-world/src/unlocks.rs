//! Quest flags earned by talking to someone at a level: the Facility Hub Wardens, each of whom
//! flags a character for a portal (Hunter's Leap, Mite Maze...) that is closed to it otherwise.
//!
//! Server data, copied in `data/unlocks.csv` (`reference/scripts/data/unlocks.sh`). A portal asks
//! for its flag in [`crate::portals::Portal::quest`]; `trip` plans through it once the flag is held.

use glam::Vec3;
use std::sync::OnceLock;

/// Someone who flags a character for a portal when talked to.
#[derive(Clone, Debug, PartialEq)]
pub struct Unlock {
    /// The quest flag stamped, as a portal's `quest` names it.
    pub flag: String,
    /// The level the NPC checks before stamping it.
    pub min_level: u32,
    /// Who to talk to.
    pub npc: String,
    /// Their cell, and their world position.
    pub cell: u32,
    pub at: Vec3,
}

impl Unlock {
    /// The portals this flag opens.
    pub fn portals(&self) -> impl Iterator<Item = &'static crate::portals::Portal> + '_ {
        crate::portals::all()
            .iter()
            .filter(|p| p.quest.eq_ignore_ascii_case(&self.flag))
    }
}

const DATA: &str = include_str!("../data/unlocks.csv");

fn parse(text: &str) -> Vec<Unlock> {
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .filter_map(|line| {
            let f: Vec<&str> = line.split(',').collect();
            if f.len() != 7 {
                return None;
            }
            let cell = u32::from_str_radix(f[3], 16).ok()?;
            let num = |s: &str| s.parse::<f32>().ok();
            let local = Vec3::new(num(f[4])?, num(f[5])?, num(f[6])?);
            Some(Unlock {
                flag: f[0].to_string(),
                min_level: f[1].parse().ok()?,
                npc: f[2].to_string(),
                cell,
                at: crate::landblock_origin(cell) + local,
            })
        })
        .collect()
}

/// Every unlock, parsed on first use.
pub fn all() -> &'static [Unlock] {
    static UNLOCKS: OnceLock<Vec<Unlock>> = OnceLock::new();
    UNLOCKS.get_or_init(|| parse(DATA))
}

/// The unlocks a character of `level` can earn and does not hold, lowest level first.
pub fn earnable(level: u32, held: &[String]) -> Vec<&'static Unlock> {
    all()
        .iter()
        .filter(|u| u.min_level <= level)
        .filter(|u| !held.iter().any(|h| h.eq_ignore_ascii_case(&u.flag)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_facility_hub_wardens_flag_its_portals() {
        let leap = all()
            .iter()
            .find(|u| u.flag == "fachubhuntersleapportal_flag")
            .expect("Hunter's Leap's flag");
        assert_eq!((leap.npc.as_str(), leap.min_level), ("Warden", 20));
        assert_eq!(leap.cell >> 16, 0x8A02, "in the Facility Hub");
        assert!(
            leap.portals().any(|p| p.name == "Hunter's Leap"),
            "the portal that asks for it"
        );
        // Every flag opens some portal the planner knows.
        for u in all() {
            assert!(u.portals().next().is_some(), "{} opens nothing", u.flag);
        }
    }

    #[test]
    fn what_can_be_earned_follows_the_level_and_what_is_held() {
        let at_15: Vec<&str> = earnable(15, &[]).iter().map(|u| u.flag.as_str()).collect();
        assert!(at_15.contains(&"fachubcolierportal_flag"));
        assert!(
            !at_15.contains(&"fachubhuntersleapportal_flag"),
            "a level 20 flag"
        );
        let held = vec!["FACHUBCOLIERPORTAL_FLAG".to_string()];
        assert!(!earnable(15, &held)
            .iter()
            .any(|u| u.flag == "fachubcolierportal_flag"));
    }
}
