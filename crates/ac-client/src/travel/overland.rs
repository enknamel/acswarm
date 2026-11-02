//! A walk past the region: the terrain route (`worldroute`) across the map as a corridor, walked
//! a region at a time. Each stretch ends where the corridor leaves the 3x3 blocks round the body
//! and is planned whole by the region planner; a stretch refused is kept clear of next time.

use ac_formats::region::Region;
use ac_scene::navarea::MAX_SIDE;
use ac_scene::worldgrid::{WorldGrid, SPACING};
use ac_scene::worldroute::{self, AVOID_RADIUS};
use glam::Vec2;

/// A stretch end is planned to within this (metres, 3D): a corridor point is known only to its
/// lattice's spacing (`worldgrid::SPACING`, 24 m).
pub(crate) const EXIT_REACH: f32 = SPACING / 2.0;

/// A landblock's side (metres).
const BLOCK: f32 = 192.0;

/// A walk end's overland corridor (`worldroute`), walked a region at a time.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Corridor {
    /// The terrain route from where it was planned to the walk end (world xy).
    points: Vec<Vec2>,
    /// Stretch ends the planner refused: the next corridor keeps clear of them.
    refused: Vec<Vec2>,
}

impl Corridor {
    /// The corridor from `from` to `to` kept off `avoid` (portal mouths) and the `refused` stretch
    /// ends, which it carries on; `None` when the terrain has none.
    pub(crate) fn plan(
        grid: &WorldGrid,
        region: &Region,
        (from, to): (Vec2, Vec2),
        avoid: &[Vec2],
        refused: Vec<Vec2>,
    ) -> Option<Corridor> {
        let mut away = avoid.to_vec();
        away.extend(&refused);
        let points = worldroute::find_avoiding(grid, region, from, to, &away)?;
        Some(Corridor { points, refused })
    }

    /// The corridor's points, from where it was planned to the walk end.
    pub(crate) fn points(&self) -> &[Vec2] {
        &self.points
    }

    /// Where the stretch from `me` ends: of the corridor resampled every `SPACING` from its point
    /// nearest `me`, the last before it first leaves the square round `me`; `None` off the square.
    pub(crate) fn exit_from(&self, me: Vec2) -> Option<Vec2> {
        let samples = resampled(&self.points);
        let nearest = samples
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.distance(me).total_cmp(&b.1.distance(me)))
            .map(|(i, _)| i)?;
        let (lo, hi) = square_round(me);
        let inside = |p: &Vec2| p.cmpge(lo).all() && p.cmple(hi).all();
        if !inside(&samples[nearest]) {
            return None;
        }
        samples[nearest..]
            .iter()
            .take_while(|p| inside(p))
            .last()
            .copied()
    }

    /// A stretch to `exit` was refused: the corridor is to be planned again, clear of it.
    pub(crate) fn refuse(&mut self, exit: Vec2) {
        self.refused.push(exit);
        self.points.clear();
    }

    /// The stretch ends refused so far, for the corridor planned next.
    pub(crate) fn refused(&self) -> Vec<Vec2> {
        self.refused.clone()
    }

    /// Whether `exit` lies within `worldroute::AVOID_RADIUS` of a stretch end refused.
    pub(crate) fn is_refused_near(&self, exit: Vec2) -> bool {
        self.refused.iter().any(|r| r.distance(exit) < AVOID_RADIUS)
    }
}

/// `points` walked along and sampled every `SPACING`, the first and last kept.
fn resampled(points: &[Vec2]) -> Vec<Vec2> {
    let mut out: Vec<Vec2> = points.first().copied().into_iter().collect();
    let mut carry = 0.0;
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = a.distance(b);
        let mut t = SPACING - carry;
        while t < len {
            out.push(a.lerp(b, t / len));
            t += SPACING;
        }
        carry = len - (t - SPACING);
    }
    if let Some(&last) = points.last() {
        if out.last() != Some(&last) {
            out.push(last);
        }
    }
    out
}

/// The square a stretch from `me` keeps to: the `MAX_SIDE` blocks a side round `me`'s block (the
/// region planner's far case, `navarea::blocks_for`), shrunk by `SPACING`.
fn square_round(me: Vec2) -> (Vec2, Vec2) {
    let block = (me / BLOCK).floor();
    let half = (MAX_SIDE as f32 - 1.0) / 2.0;
    let lo = (block - half) * BLOCK + SPACING;
    let hi = (block + half + 1.0) * BLOCK - SPACING;
    (lo, hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A corridor of `points`, none refused.
    fn along(points: Vec<Vec2>) -> Corridor {
        Corridor {
            points,
            refused: Vec::new(),
        }
    }

    #[test]
    fn a_stretch_ends_where_the_corridor_leaves_the_blocks_round_the_body() {
        // From the middle of a block, 1 km east: the square's east edge is the next block's.
        let me = Vec2::new(192.0 * 10.0 + 96.0, 192.0 * 10.0 + 96.0);
        let c = along(vec![me, me + Vec2::new(1000.0, 0.0)]);
        let exit = c.exit_from(me).expect("an exit");
        let edge = 192.0 * 12.0 - SPACING;
        assert!(exit.x <= edge && exit.x > edge - SPACING, "{exit:?}");
        assert_eq!(exit.y, me.y);
        // From there, the next stretch ends a block further on, and the last at the walk end.
        let next = c.exit_from(exit).expect("an exit");
        assert!(next.x > exit.x + 150.0, "{next:?}");
        let end = me + Vec2::new(1000.0, 0.0);
        assert_eq!(c.exit_from(end - Vec2::new(30.0, 0.0)), Some(end));
    }

    #[test]
    fn a_stretch_follows_the_corridor_round_a_bend() {
        // North 100 m, then east: the stretch ends on the corridor, past the bend.
        let me = Vec2::new(192.0 * 10.0 + 96.0, 192.0 * 10.0 + 96.0);
        let bend = me + Vec2::new(0.0, 100.0);
        let c = along(vec![me, bend, bend + Vec2::new(1000.0, 0.0)]);
        let exit = c.exit_from(me).expect("an exit");
        assert!((exit.y - bend.y).abs() < 1e-3, "{exit:?}");
        assert!(exit.x > bend.x + 100.0, "{exit:?}");
    }

    #[test]
    fn a_body_drawn_off_the_corridor_has_no_stretch_on_it() {
        let me = Vec2::new(192.0 * 10.0 + 96.0, 192.0 * 10.0 + 96.0);
        let c = along(vec![me, me + Vec2::new(1000.0, 0.0)]);
        assert_eq!(c.exit_from(me + Vec2::new(0.0, 600.0)), None);
    }

    #[test]
    fn a_refused_end_is_kept_clear_of() {
        let mut c = along(vec![Vec2::ZERO, Vec2::new(500.0, 0.0)]);
        c.refuse(Vec2::new(300.0, 0.0));
        assert_eq!(
            c.exit_from(Vec2::ZERO),
            None,
            "the corridor is to be planned again"
        );
        assert!(c.is_refused_near(Vec2::new(310.0, 0.0)));
        assert!(!c.is_refused_near(Vec2::new(400.0, 0.0)));
        assert_eq!(c.refused(), vec![Vec2::new(300.0, 0.0)]);
    }

    #[test]
    fn a_corridor_is_sampled_every_spacing() {
        let s = resampled(&[Vec2::ZERO, Vec2::new(30.0, 0.0), Vec2::new(30.0, 40.0)]);
        let steps: Vec<f32> = s.windows(2).map(|w| w[0].distance(w[1])).collect();
        assert_eq!(s.first(), Some(&Vec2::ZERO));
        assert_eq!(s.last(), Some(&Vec2::new(30.0, 40.0)));
        // Every gap but the last is a spacing along the corridor, corners cut by the sample.
        assert!(
            steps[..steps.len() - 1]
                .iter()
                .all(|d| *d <= SPACING + 1e-3),
            "{steps:?}"
        );
    }
}
