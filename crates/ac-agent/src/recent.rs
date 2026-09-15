//! When each thing was last seen, forgotten a fixed while later.
//! Separate from [`Patience`](crate::did::Patience): a window here never doubles.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// When each key was last marked, for lists that let go after a fixed window.
#[derive(Clone, Debug, Default)]
pub struct Recent<K: Ord + Clone> {
    seen: BTreeMap<K, Instant>,
}

impl<K: Ord + Clone> Recent<K> {
    pub fn new() -> Self {
        Recent {
            seen: BTreeMap::new(),
        }
    }

    /// Seen at `now`: replaces any earlier mark, so the window starts again.
    pub fn mark(&mut self, key: K, now: Instant) {
        self.seen.insert(key, now);
    }

    /// When this one was last marked, if it is remembered.
    pub fn since(&self, key: &K) -> Option<Instant> {
        self.seen.get(key).copied()
    }

    /// Whether this one was marked less than `window` before `now`. A mark after `now` counts.
    pub fn within(&self, key: &K, now: Instant, window: Duration) -> bool {
        self.seen.get(key).is_some_and(|&at| fresh(at, now, window))
    }

    /// Forget every mark `window` or more before `now`.
    pub fn expire(&mut self, now: Instant, window: Duration) {
        self.seen.retain(|_, at| fresh(*at, now, window));
    }

    /// Forget this one, whenever it was marked.
    pub fn forget(&mut self, key: &K) {
        self.seen.remove(key);
    }

    /// How many are remembered, for the log.
    pub fn len(&self) -> usize {
        self.seen.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }

    /// Each remembered key with its mark, in key order (not the order marked).
    pub fn iter(&self) -> impl Iterator<Item = (&K, Instant)> + '_ {
        self.seen.iter().map(|(k, at)| (k, *at))
    }
}

/// A mark at `at` still counts at `now`: less than `window` old, or later than `now`.
fn fresh(at: Instant, now: Instant, window: Duration) -> bool {
    now.checked_duration_since(at).is_none_or(|d| d < window)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Duration = Duration::from_secs(60);

    #[test]
    fn a_mark_counts_for_its_window_and_not_after() {
        let t0 = Instant::now();
        let mut r: Recent<u32> = Recent::new();
        assert!(!r.within(&1, t0, WINDOW));
        assert_eq!(r.since(&1), None);

        r.mark(1, t0);
        assert_eq!(r.since(&1), Some(t0));
        assert!(r.within(&1, t0, WINDOW));
        assert!(r.within(&1, t0 + WINDOW - Duration::from_millis(1), WINDOW));
        assert!(!r.within(&1, t0 + WINDOW, WINDOW));
    }

    #[test]
    fn marking_again_restarts_the_window_without_growing_it() {
        let t0 = Instant::now();
        let mut r: Recent<u32> = Recent::new();
        let t1 = t0 + WINDOW / 2;
        r.mark(1, t0);
        r.mark(1, t1);
        assert_eq!(r.since(&1), Some(t1));
        assert_eq!(r.len(), 1);
        assert!(r.within(&1, t0 + WINDOW, WINDOW));
        // Unlike Patience, a second mark does not double anything.
        assert!(!r.within(&1, t1 + WINDOW, WINDOW));
    }

    #[test]
    fn a_mark_later_than_now_still_counts() {
        let t0 = Instant::now();
        let mut r: Recent<u32> = Recent::new();
        r.mark(1, t0 + WINDOW * 2);
        assert!(r.within(&1, t0, WINDOW));
        r.expire(t0, WINDOW);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn expiring_forgets_only_what_has_lapsed() {
        let t0 = Instant::now();
        let later = t0 + Duration::from_secs(10);
        let mut r: Recent<String> = Recent::new();
        r.mark("Drudge Skulker".into(), t0);
        r.mark("Banderling Scout".into(), later);
        r.mark("Mite".into(), later);

        r.expire(t0 + WINDOW, WINDOW);
        assert_eq!(r.since(&"Drudge Skulker".into()), None);
        let kept: Vec<(&String, Instant)> = r.iter().collect();
        assert_eq!(
            kept,
            [
                (&"Banderling Scout".to_string(), later),
                (&"Mite".to_string(), later)
            ]
        );

        r.forget(&"Mite".into());
        r.forget(&"Banderling Scout".into());
        assert!(r.is_empty());
    }
}
