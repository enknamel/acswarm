//! Where the monsters are: the landblocks with creature spawns, open ground and dungeons, with
//! their levels and what a kill is worth, so a character can pick the ground that pays best.
//!
//! Spawns are server-side; `data/hunting.csv` is a copy (`reference/scripts/data/hunting.sh`).
//! A dungeon's way in is the portal into it that asks no quest (`portals::all`).

use glam::Vec2;
use std::sync::OnceLock;

/// The monsters of one landblock: its open ground, or the dungeon that is the whole of it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ground {
    /// Landblock id (the high half of a cell id).
    pub landblock: u32,
    /// World xy of the middle of the spawns; for a dungeon, where its portal lands.
    pub at: Vec2,
    /// How many spawn entries the block's generators hold: a rough
    /// measure of how busy it is.
    pub count: u32,
    pub min_level: u32,
    pub max_level: u32,
    /// The level of the creature that spawns most often.
    pub level: u32,
    /// That creature's name.
    pub name: String,
    /// The XP a kill awards, on average (XpOverride, Creature_Death.cs:193).
    pub xp: u32,
    /// Health a kill takes off, on average, over the creature's weakest resistance.
    pub health: u32,
    /// How many creatures the generators keep up at once.
    pub creatures: u32,
    /// A dungeon's way in; `None` for open ground.
    pub entrance: Option<Entrance>,
}

/// The portal into a dungeon: the cell it lands in (at the ground's `at`) and who may take it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entrance {
    pub cell: u32,
    /// The portal's level range, 0 for no limit.
    pub min_level: u32,
    pub max_level: u32,
}

/// Creatures at which a ground pays half its kill rate: thinner ones are walked between kills,
/// busier ones keep a character fighting. A guess at the curve's knee, not measured.
pub const HALF_BUSY: f32 = 10.0;

/// A ground with fewer spawn entries than this is a stray, not a
/// hunting ground: one chicken behind a house is not worth a journey.
pub const LEAST_SPAWNS: u32 = 2;

/// How to hunt a piece of ground.
///
/// Grounds are not alike and the same behaviour does not suit them. A
/// few spawn hard enough that a character standing still is never
/// idle, and walking about there only takes it away from the fight.
/// Most need covering on foot to turn anything up. A thin one is
/// emptied in a few minutes and is worth leaving.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Tactic {
    /// Work it out from how busy the ground is. The usual choice.
    #[default]
    Auto,
    /// Hold the spot and let them come.
    Camp,
    /// Walk a circuit of the ground, and keep walking it.
    Patrol,
    /// Walk it, and once it is quiet go and find another.
    Sweep,
}

impl Tactic {
    pub const ALL: [Tactic; 4] = [Tactic::Auto, Tactic::Camp, Tactic::Patrol, Tactic::Sweep];

    pub fn label(self) -> &'static str {
        match self {
            Tactic::Auto => "to suit the ground",
            Tactic::Camp => "hold the spot",
            Tactic::Patrol => "walk the ground",
            Tactic::Sweep => "clear it, then move on",
        }
    }
}

/// A ground with at least this many spawn entries keeps a character
/// busy standing still: they arrive faster than they can be killed.
///
/// Chosen against the table rather than guessed. Across the 991
/// grounds the middle is 7 entries and the top tenth begins at 30, so
/// this cut takes the busiest 14% -- the handful of places that really
/// are a queue -- and leaves the rest to be walked.
pub const CROWDED: u32 = 24;

/// Below this there is not enough to be worth staying for once what is
/// there has been killed. Just over half of all grounds are this thin.
pub const THIN: u32 = 8;

impl Ground {
    /// What [`Tactic::Auto`] settles on for this ground, from how many
    /// spawn entries its generators hold.
    pub fn suggested_tactic(&self) -> Tactic {
        if self.count >= CROWDED {
            Tactic::Camp
        } else if self.count < THIN {
            Tactic::Sweep
        } else {
            Tactic::Patrol
        }
    }
}

/// The tactic to actually use: the one asked for, or the one the
/// ground suggests when that is [`Tactic::Auto`]. Without a ground
/// (nothing known about where we stand) `Auto` walks, which turns
/// something up either way.
pub fn tactic_for(asked: Tactic, ground: Option<&Ground>) -> Tactic {
    match asked {
        Tactic::Auto => ground.map_or(Tactic::Patrol, |g| g.suggested_tactic()),
        chosen => chosen,
    }
}

/// Grounds whose most common creature's name contains `needle`, or
/// whose landblock is written in it (`"A9B4"`), nearest `from` first.
///
/// This is what a player types to say where the party should hunt.
pub fn search(needle: &str, from: Vec2) -> Vec<&'static Ground> {
    let needle = needle.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let as_block = u32::from_str_radix(needle.trim_start_matches("0x"), 16).ok();
    let mut v: Vec<&Ground> = all()
        .iter()
        .filter(|g| {
            g.name.to_lowercase().contains(&needle)
                || as_block.is_some_and(|b| g.landblock >> 16 == b || g.landblock == b)
        })
        .collect();
    v.sort_by(|a, b| a.at.distance(from).total_cmp(&b.at.distance(from)));
    v
}

impl Ground {
    /// Whether a character of `level` should hunt here, allowing
    /// `margin` levels either way: what spawns most must be within the
    /// margin, something there must be no harder than the margin
    /// allows, and nothing there may be so far above the character
    /// that a stray of it is a death (twice the margin).
    pub fn suits(&self, level: u32, margin: u32) -> bool {
        if self.count < LEAST_SPAWNS {
            return false;
        }
        let low = level.saturating_sub(margin);
        let high = level + margin;
        self.level >= low
            && self.level <= high
            && self.min_level <= high
            && self.max_level <= level + 2 * margin
            // A dungeon's rooms are walked through, not round: nothing in it may pass the margin.
            && (!self.is_dungeon() || self.max_level <= high)
    }

    /// Whether this is a dungeon (entered by its portal) rather than open ground.
    pub fn is_dungeon(&self) -> bool {
        self.entrance.is_some()
    }

    /// The cell to travel to: a dungeon's landing cell, 0 for open ground (known by position).
    pub fn cell(&self) -> u32 {
        self.entrance.map_or(0, |e| e.cell)
    }

    /// Whether a character of `level` may take the way in.
    pub fn can_enter(&self, level: u32) -> bool {
        self.entrance.is_none_or(|e| {
            (e.min_level == 0 || level >= e.min_level) && (e.max_level == 0 || level <= e.max_level)
        })
    }

    /// What hunting here pays, in XP per point of health taken off, scaled by how busy it is
    /// ([`HALF_BUSY`]). XP comes in level steps while health varies 2-5x within one.
    pub fn worth(&self) -> f32 {
        let c = self.creatures as f32;
        self.xp as f32 / self.health.max(1) as f32 * c / (c + HALF_BUSY)
    }
}

const DATA: &str = include_str!("../data/hunting.csv");

fn parse(text: &str) -> Vec<Ground> {
    let mut out = Vec::with_capacity(1000);
    for line in text.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        if f.len() != 12 {
            continue;
        }
        let Ok(landblock) = u32::from_str_radix(f[0], 16) else {
            continue;
        };
        let num = |s: &str| s.parse::<f32>().ok();
        let int = |s: &str| s.parse::<u32>().ok();
        let (Some(x), Some(y)) = (num(f[1]), num(f[2])) else {
            continue;
        };
        let (Some(count), Some(min_level), Some(max_level), Some(level)) =
            (int(f[3]), int(f[4]), int(f[5]), int(f[6]))
        else {
            continue;
        };
        let (Some(xp), Some(health), Some(creatures)) = (int(f[8]), int(f[9]), int(f[10])) else {
            continue;
        };
        // A dungeon's way in is filled in from the portals (`with_entrances`).
        let entrance = match f[11] {
            "o" => None,
            "d" => Some(Entrance::default()),
            _ => continue,
        };
        let origin = crate::landblock_origin(landblock << 16);
        out.push(Ground {
            landblock,
            at: Vec2::new(origin.x + x, origin.y + y),
            count,
            min_level,
            max_level,
            level,
            name: f[7].to_string(),
            xp,
            health,
            creatures,
            entrance,
        });
    }
    out
}

/// Give each dungeon its way in: of the working portals into it that ask no quest, the one with
/// the lowest level. A dungeon only a quest opens is dropped: nobody can be sent there.
fn with_entrances(grounds: Vec<Ground>, portals: &[crate::portals::Portal]) -> Vec<Ground> {
    grounds
        .into_iter()
        .filter_map(|mut g| {
            if g.entrance.is_none() {
                return Some(g);
            }
            let p = portals
                .iter()
                .filter(|p| p.works() && p.quest.is_empty() && !p.exit_outdoors())
                .filter(|p| p.to_cell >> 16 == g.landblock)
                .min_by_key(|p| p.min_level)?;
            g.at = p.to.truncate();
            g.entrance = Some(Entrance {
                cell: p.to_cell,
                min_level: p.min_level,
                max_level: p.max_level,
            });
            Some(g)
        })
        .collect()
}

/// Every ground, parsed on first use.
pub fn all() -> &'static [Ground] {
    static GROUNDS: OnceLock<Vec<Ground>> = OnceLock::new();
    GROUNDS.get_or_init(|| with_entrances(parse(DATA), crate::portals::all()))
}

/// The ground of a landblock, if it has monsters.
pub fn at(landblock: u32) -> Option<&'static Ground> {
    all().iter().find(|g| g.landblock == landblock)
}

/// The grounds that suit a character of `level` (see [`Ground::suits`]) and that it may enter,
/// best paying first ([`Ground::worth`]), leaving out the landblocks in `skip`: the one just
/// hunted out, one it could not reach, one it died on.
pub fn ranked_for(level: u32, margin: u32, skip: &[u32]) -> Vec<&'static Ground> {
    ranked_among(all(), level, margin, skip)
}

fn ranked_among<'a>(
    grounds: &'a [Ground],
    level: u32,
    margin: u32,
    skip: &[u32],
) -> Vec<&'a Ground> {
    let mut v: Vec<&Ground> = grounds
        .iter()
        .filter(|g| g.suits(level, margin) && g.can_enter(level) && g.worth() > 0.0)
        .filter(|g| !skip.contains(&g.landblock))
        .collect();
    v.sort_by(|a, b| b.worth().total_cmp(&a.worth()));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ground(landblock: u32, x: f32, count: u32, lo: u32, hi: u32, level: u32) -> Ground {
        Ground {
            landblock,
            at: Vec2::new(x, 0.0),
            count,
            min_level: lo,
            max_level: hi,
            level,
            name: "Thing".into(),
            xp: 1000,
            health: 100,
            creatures: count,
            entrance: None,
        }
    }

    #[test]
    fn the_table_reads_and_places_the_grounds() {
        let all = all();
        assert!(all.len() > 900, "{} grounds", all.len());
        // Open ground stands where its landblock is; a dungeon where its portal lands, inside it.
        for g in all {
            assert!(g.min_level <= g.level && g.level <= g.max_level, "{g:?}");
            match g.entrance {
                None => {
                    let origin = crate::landblock_origin(g.landblock << 16);
                    assert!(g.at.x >= origin.x && g.at.x <= origin.x + 192.0, "{g:?}");
                    assert!(g.at.y >= origin.y && g.at.y <= origin.y + 192.0, "{g:?}");
                }
                Some(e) => {
                    assert_eq!(e.cell >> 16, g.landblock, "{g:?}");
                    assert!(e.cell & 0xFFFF >= 0x100, "lands indoors: {g:?}");
                }
            }
        }
        assert!(all.iter().filter(|g| g.is_dungeon()).count() > 300);
        // A beginner's ground, and something for the strong too.
        let g = ranked_for(2, 8, &[]);
        assert!(g.first().is_some_and(|g| g.level <= 10), "{:?}", g.first());
        assert!(!ranked_for(150, 20, &[]).is_empty());
        assert!(at(g[0].landblock).is_some());
    }

    #[test]
    fn suitability_is_a_window_on_the_usual_level() {
        // A level 10 with a margin of 5 hunts levels 5..=15.
        assert!(ground(1, 0.0, 5, 8, 12, 10).suits(10, 5));
        assert!(ground(1, 0.0, 5, 5, 5, 5).suits(10, 5));
        assert!(!ground(1, 0.0, 5, 4, 4, 4).suits(10, 5), "too easy");
        assert!(!ground(1, 0.0, 5, 16, 16, 16).suits(10, 5), "too hard");
        // A rare spawn far above the character rules the place out.
        assert!(!ground(1, 0.0, 5, 8, 40, 10).suits(10, 5));
        assert!(
            ground(1, 0.0, 5, 8, 20, 10).suits(10, 5),
            "twice the margin is allowed"
        );
        // A single stray is no hunting ground.
        assert!(!ground(1, 0.0, 1, 10, 10, 10).suits(10, 5));
        // A low level's window does not go below zero.
        assert!(ground(1, 0.0, 5, 1, 3, 2).suits(1, 5));
    }

    #[test]
    fn the_best_paying_suitable_ground_comes_first_and_skips_are_honoured() {
        let soft = Ground {
            health: 50,
            ..ground(0xB, 50.0, 5, 10, 10, 10)
        };
        let grounds = vec![
            ground(0xA, 100.0, 5, 10, 10, 10),
            soft,
            ground(0xC, 10.0, 5, 50, 50, 50),
            ground(0xD, 20.0, 1, 10, 10, 10),
            Ground {
                xp: 0,
                ..ground(0xE, 0.0, 5, 10, 10, 10)
            },
        ];
        let ids = |v: Vec<&Ground>| v.iter().map(|g| g.landblock).collect::<Vec<_>>();
        // Half the health for the same XP pays twice as well; a stray and a ground paying no XP
        // are no grounds at all.
        assert_eq!(ids(ranked_among(&grounds, 10, 5, &[])), vec![0xB, 0xA]);
        assert_eq!(ids(ranked_among(&grounds, 10, 5, &[0xB])), vec![0xA]);
        assert_eq!(ids(ranked_among(&grounds, 50, 5, &[])), vec![0xC]);
    }

    #[test]
    fn a_busy_ground_pays_more() {
        let thin = ground(1, 0.0, 5, 10, 10, 10);
        let busy = Ground {
            creatures: 80,
            ..thin.clone()
        };
        assert!(busy.worth() > 1.5 * thin.worth());
    }

    #[test]
    fn a_dungeon_is_entered_by_its_portal_and_only_at_its_level() {
        // Tusker Cave, under the Facility Hub: 81 Tusker Crimsonbacks at level 30.
        let cave = at(0x5B46).expect("Tusker Cave");
        assert!(cave.is_dungeon());
        assert_eq!(cave.cell() >> 16, 0x5B46);
        assert_eq!((cave.level, cave.xp), (30, 5000));
        assert!(cave.can_enter(35));
        let barracks = Ground {
            entrance: Some(Entrance {
                cell: 0x5A48_01B5,
                min_level: 40,
                max_level: 0,
            }),
            ..cave.clone()
        };
        assert!(!barracks.can_enter(35) && barracks.can_enter(40));
        // A level 35 is sent to a dungeon: the Tusker Cave pays more than any open ground.
        let best = ranked_for(35, 8, &[]);
        assert!(
            best.first().is_some_and(|g| g.is_dungeon()),
            "{:?}",
            best.first()
        );
        assert!(cave.worth() > at(0xA2BD).expect("Freshwater Armoredillos").worth());
    }

    #[test]
    fn bad_lines_are_skipped() {
        let v = parse(
            "# c\n\nnope\nA9B2,74,83,30,7,20,7,Blood Shrethlet,1411,47,34,o\n\
             ZZ,1,1,1,1,1,1,x,1,1,1,o\nA9B3,1,1,1,1,1,1,x,1,1,1,q\n",
        );
        assert_eq!(v.len(), 1);
        assert_eq!((v[0].xp, v[0].health, v[0].creatures), (1411, 47, 34));
        assert_eq!(v[0].name, "Blood Shrethlet");
        assert_eq!(v[0].landblock, 0xA9B2);
        assert_eq!(
            v[0].at,
            Vec2::new(0xA9 as f32 * 192.0 + 74.0, 0xB2 as f32 * 192.0 + 83.0)
        );
    }
    #[test]
    fn a_crowded_ground_is_worth_standing_still_on() {
        let busy = Ground {
            landblock: 0xA9B4,
            at: Vec2::ZERO,
            count: 40,
            min_level: 1,
            max_level: 10,
            level: 5,
            name: "Drudge".into(),
            ..Default::default()
        };
        assert_eq!(busy.suggested_tactic(), Tactic::Camp);
        // A middling one needs covering on foot.
        let middling = Ground {
            count: 12,
            ..busy.clone()
        };
        assert_eq!(middling.suggested_tactic(), Tactic::Patrol);
        // A thin one is emptied and left.
        let thin = Ground {
            count: 3,
            ..busy.clone()
        };
        assert_eq!(thin.suggested_tactic(), Tactic::Sweep);
    }

    #[test]
    fn asking_for_a_tactic_beats_the_suggestion() {
        let busy = Ground {
            landblock: 1,
            at: Vec2::ZERO,
            count: 40,
            min_level: 1,
            max_level: 10,
            level: 5,
            name: "Drudge".into(),
            ..Default::default()
        };
        assert_eq!(tactic_for(Tactic::Auto, Some(&busy)), Tactic::Camp);
        assert_eq!(tactic_for(Tactic::Patrol, Some(&busy)), Tactic::Patrol);
        assert_eq!(tactic_for(Tactic::Sweep, Some(&busy)), Tactic::Sweep);
        // Nothing known about where we stand: walking turns something
        // up either way.
        assert_eq!(tactic_for(Tactic::Auto, None), Tactic::Patrol);
        assert_eq!(tactic_for(Tactic::Camp, None), Tactic::Camp);
    }

    #[test]
    fn a_ground_can_be_found_by_name_or_by_landblock() {
        let from = Vec2::ZERO;
        // Every ground names the creature that spawns most often.
        let named = search("drudge", from);
        assert!(!named.is_empty(), "no drudge ground anywhere");
        // Nearest first.
        for pair in named.windows(2) {
            assert!(pair[0].at.distance(from) <= pair[1].at.distance(from));
        }
        // And by landblock, the way a player reads it off the map.
        let block = named[0].landblock >> 16;
        let by_block = search(&format!("{block:04X}"), from);
        assert!(
            by_block.iter().any(|g| g.landblock == named[0].landblock),
            "{block:04X} did not find its own ground"
        );
        assert!(search("", from).is_empty());
    }

    #[test]
    fn a_landblock_with_no_spawns_is_not_a_ground() {
        assert!(at(0xFFFF_0000).is_none());
        let some = all().first().expect("a ground");
        assert_eq!(
            at(some.landblock).map(|g| g.landblock),
            Some(some.landblock)
        );
    }
}
