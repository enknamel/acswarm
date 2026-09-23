//! Headless tick timing: each loop iteration's work and lateness against the `--tick-hz`
//! period and each session's share, as one `perf:` line per status period and one at exit.
//! The monotonic clock stops while the machine sleeps and wall time does not, so a window
//! a sleep fell in is dropped with a warning rather than reported as a measurement.

use std::time::{Duration, Instant, SystemTime};

/// Wall time this far past the monotonic clock between two iterations is a machine sleep.
const SLEEP_GAP: Duration = Duration::from_secs(2);

/// Durations under 2^6 µs get a bucket each.
const EXACT_BITS: u32 = 6;
/// Above that, 2^5 buckets per doubling: a percentile is within 3% of the true value.
const SUB_BITS: u32 = 5;
/// Doublings covered past the exact range, up to 2^26 µs (67 s); longer shares the last bucket.
const DOUBLINGS: usize = 20;
const BUCKETS: usize = (1 << EXACT_BITS) + DOUBLINGS * (1 << SUB_BITS);

/// Durations in log-linear buckets, fixed in size so that recording one never allocates.
#[derive(Clone)]
pub struct Histogram {
    counts: [u32; BUCKETS],
    count: u64,
    min_us: u64,
    max_us: u64,
}

impl Default for Histogram {
    fn default() -> Self {
        Histogram {
            counts: [0; BUCKETS],
            count: 0,
            min_us: u64::MAX,
            max_us: 0,
        }
    }
}

/// The bucket holding `us` microseconds.
fn bucket_of(us: u64) -> usize {
    if us < 1 << EXACT_BITS {
        return us as usize;
    }
    let doubling = us.ilog2();
    let past = (doubling - EXACT_BITS) as usize;
    if past >= DOUBLINGS {
        return BUCKETS - 1;
    }
    let sub = (us >> (doubling - SUB_BITS)) as usize & ((1 << SUB_BITS) - 1);
    (1 << EXACT_BITS) + past * (1 << SUB_BITS) + sub
}

/// The middle of bucket `b`'s range, µs.
fn bucket_mid(b: usize) -> u64 {
    let Some(above) = b.checked_sub(1 << EXACT_BITS) else {
        return b as u64;
    };
    let shift = (above >> SUB_BITS) as u32 + EXACT_BITS - SUB_BITS;
    let sub = (above & ((1 << SUB_BITS) - 1)) as u64;
    (((1 << SUB_BITS) + sub) << shift) + (1 << shift) / 2
}

impl Histogram {
    pub fn record(&mut self, d: Duration) {
        let us = u64::try_from(d.as_micros()).unwrap_or(u64::MAX);
        self.counts[bucket_of(us)] += 1;
        self.count += 1;
        self.min_us = self.min_us.min(us);
        self.max_us = self.max_us.max(us);
    }

    pub fn count(&self) -> u64 {
        self.count
    }

    /// The nearest-rank `q` quantile (0..=1), clamped to the exact extremes; `None` when empty.
    pub fn quantile(&self, q: f64) -> Option<Duration> {
        if self.count == 0 {
            return None;
        }
        let rank = ((q.clamp(0.0, 1.0) * self.count as f64).ceil() as u64).max(1);
        let mut seen = 0u64;
        let b = self.counts.iter().position(|&n| {
            seen += n as u64;
            seen >= rank
        })?;
        let us = bucket_mid(b).clamp(self.min_us, self.max_us);
        Some(Duration::from_micros(us))
    }

    pub fn max(&self) -> Option<Duration> {
        (self.count > 0).then(|| Duration::from_micros(self.max_us))
    }

    pub fn merge(&mut self, other: &Histogram) {
        for (a, b) in self.counts.iter_mut().zip(&other.counts) {
            *a += b;
        }
        self.count += other.count;
        self.min_us = self.min_us.min(other.min_us);
        self.max_us = self.max_us.max(other.max_us);
    }

    pub fn clear(&mut self) {
        *self = Histogram::default();
    }
}

/// How long the machine slept between two iterations `mono` and `wall` apart, if it did.
pub fn sleep_gap(mono: Duration, wall: Duration) -> Option<Duration> {
    let gap = wall.saturating_sub(mono);
    (gap > SLEEP_GAP).then_some(gap)
}

/// `count` events over `span`, per minute; 0 over no time.
pub fn per_minute(count: u64, span: Duration) -> f64 {
    let minutes = span.as_secs_f64() / 60.0;
    if minutes > 0.0 {
        count as f64 / minutes
    } else {
        0.0
    }
}

/// One session slot's ticks and the time they took.
#[derive(Clone, Copy, Default)]
pub struct SessionCost {
    spent: Duration,
    ticks: u64,
}

impl SessionCost {
    fn mean(&self) -> Option<Duration> {
        (self.ticks > 0).then(|| self.spent / u32::try_from(self.ticks).unwrap_or(u32::MAX))
    }
}

/// The iterations of one stretch of the run, a status window or the whole of it.
#[derive(Clone, Default)]
pub struct TickStats {
    work: Histogram,
    late: Histogram,
    /// Iterations whose work alone ran past the period.
    over: u64,
    /// Iterations after which the loop dropped its schedule and started again from now.
    behind: u64,
    /// Monotonic time the stretch covered.
    span: Duration,
    /// Every session tick together, for the mean.
    all: SessionCost,
    /// By session slot, for the costliest.
    slots: Vec<SessionCost>,
}

impl TickStats {
    fn merge(&mut self, other: &TickStats) {
        self.work.merge(&other.work);
        self.late.merge(&other.late);
        self.over += other.over;
        self.behind += other.behind;
        self.span += other.span;
        self.all.spent += other.all.spent;
        self.all.ticks += other.all.ticks;
        if self.slots.len() < other.slots.len() {
            self.slots.resize(other.slots.len(), SessionCost::default());
        }
        for (a, b) in self.slots.iter_mut().zip(&other.slots) {
            a.spent += b.spent;
            a.ticks += b.ticks;
        }
    }

    /// Back to empty, keeping the slots' storage.
    fn clear(&mut self) {
        self.work.clear();
        self.late.clear();
        self.over = 0;
        self.behind = 0;
        self.span = Duration::ZERO;
        self.all = SessionCost::default();
        self.slots.fill(SessionCost::default());
    }

    /// The slot with the highest mean tick cost, and that mean.
    fn costliest(&self) -> Option<(usize, Duration)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, c)| c.mean().map(|m| (i, m)))
            .max_by_key(|&(_, m)| m)
    }

    /// The line's body: sessions, ticks, work and lateness against `period`, the session costs.
    pub fn summary(&self, period: Duration, name_of: &dyn Fn(usize) -> String) -> String {
        let sessions = self.slots.iter().filter(|c| c.ticks > 0).count();
        let costliest = match self.costliest() {
            Some((i, mean)) => format!("{} {} ms", name_of(i), ms(Some(mean), 2)),
            None => "-".to_string(),
        };
        format!(
            "{sessions} sessions, {} ticks, work p50 {} ms p95 {} max {} of {} ms, over {}, \
             behind {}, late p95 {} ms, per session {} ms, costliest {costliest}",
            self.work.count(),
            ms(self.work.quantile(0.5), 1),
            ms(self.work.quantile(0.95), 1),
            ms(self.work.max(), 1),
            ms(Some(period), 1).trim_end_matches(".0"),
            self.over,
            self.behind,
            ms(self.late.quantile(0.95), 1),
            ms(self.all.mean(), 2),
        )
    }
}

/// A duration in milliseconds to `places` decimals, or `-` for none.
fn ms(d: Option<Duration>, places: usize) -> String {
    match d {
        Some(d) => format!("{:.*}", places, d.as_secs_f64() * 1000.0),
        None => "-".to_string(),
    }
}

/// The headless loop's timing, fed by `begin`, `session` and `end` once per iteration.
pub struct TickMeter {
    period: Duration,
    window: TickStats,
    run: TickStats,
    /// When the first iteration began, on both clocks, for the wall time the run spanned.
    first: Option<(Instant, SystemTime)>,
    /// When the last iteration began, on both clocks.
    last: Option<(Instant, SystemTime)>,
    /// The lateness of the iteration under way, recorded once its work is known.
    late: Duration,
    /// Windows dropped for a machine sleep.
    sleeps: u32,
    /// How long the machine slept, all the dropped windows together.
    slept: Duration,
}

impl TickMeter {
    pub fn new(period: Duration) -> Self {
        TickMeter {
            period,
            window: TickStats::default(),
            run: TickStats::default(),
            first: None,
            last: None,
            late: Duration::ZERO,
            sleeps: 0,
            slept: Duration::ZERO,
        }
    }

    /// An iteration begins at `now` (`wall` on the wall clock), scheduled for `due`. A sleep
    /// since the last one drops the window under way and is returned.
    pub fn begin(&mut self, now: Instant, wall: SystemTime, due: Instant) -> Option<Duration> {
        let mut slept = None;
        if let Some((mono0, wall0)) = self.last {
            let mono = now.saturating_duration_since(mono0);
            slept = wall
                .duration_since(wall0)
                .ok()
                .and_then(|w| sleep_gap(mono, w));
            match slept {
                Some(gap) => {
                    tracing::warn!(
                        "the machine slept for about {:.0} s: the wall clock ran that far past \
                         the monotonic one; this window is left out of the measurements",
                        gap.as_secs_f64()
                    );
                    self.sleeps += 1;
                    self.slept += gap;
                    self.window.clear();
                }
                None => self.window.span += mono,
            }
        }
        self.first.get_or_insert((now, wall));
        self.last = Some((now, wall));
        self.late = now.saturating_duration_since(due);
        slept
    }

    /// Session slot `slot` took `spent` this iteration: its tick, events and plugins.
    pub fn session(&mut self, slot: usize, spent: Duration) {
        let slots = &mut self.window.slots;
        if slots.len() <= slot {
            slots.resize(slot + 1, SessionCost::default());
        }
        slots[slot].spent += spent;
        slots[slot].ticks += 1;
        self.window.all.spent += spent;
        self.window.all.ticks += 1;
    }

    /// The iteration's work took `work`; `behind` when the loop then dropped its schedule.
    pub fn end(&mut self, work: Duration, behind: bool) {
        self.window.work.record(work);
        self.window.late.record(self.late);
        self.window.over += u64::from(work > self.period);
        self.window.behind += u64::from(behind);
    }

    /// Sessions were stopped and the slots after them moved down: a slot names another session.
    pub fn forget_slots(&mut self) {
        self.window.slots.clear();
        self.run.slots.clear();
    }

    /// The `perf:` line for the window under way.
    pub fn window_line(&self, name_of: &dyn Fn(usize) -> String) -> String {
        format!("perf: {}", self.window.summary(self.period, name_of))
    }

    /// Fold the window under way into the run and start the next one.
    pub fn close_window(&mut self) {
        self.run.merge(&self.window);
        self.window.clear();
    }

    /// Log the window's line and start the next window.
    pub fn report(&mut self, name_of: &dyn Fn(usize) -> String) {
        tracing::info!("{}", self.window_line(name_of));
        self.close_window();
    }

    /// The `perf run:` line: the whole run, with what was left out of it and why.
    pub fn run_line(&self, name_of: &dyn Fn(usize) -> String) -> String {
        let wall = match (self.first, self.last) {
            (Some((_, w0)), Some((_, w1))) => w1.duration_since(w0).unwrap_or_default(),
            _ => Duration::ZERO,
        };
        format!(
            "perf run: {}, {:.1} overruns/min, {:.0} s measured of {:.0} s wall, \
             {} sleep windows excluded ({:.0} s asleep)",
            self.run.summary(self.period, name_of),
            per_minute(self.run.over, self.run.span),
            self.run.span.as_secs_f64(),
            wall.as_secs_f64(),
            self.sleeps,
            self.slept.as_secs_f64(),
        )
    }

    /// Fold in the last window and log the run's line.
    pub fn finish(&mut self, name_of: &dyn Fn(usize) -> String) {
        self.close_window();
        tracing::info!("{}", self.run_line(name_of));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn millis(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn filled(values_us: &[u64]) -> Histogram {
        let mut h = Histogram::default();
        for &us in values_us {
            h.record(Duration::from_micros(us));
        }
        h
    }

    #[test]
    fn an_empty_histogram_has_no_quantiles() {
        let h = Histogram::default();
        assert_eq!(h.quantile(0.5), None);
        assert_eq!(h.quantile(0.95), None);
        assert_eq!(h.max(), None);
        assert_eq!(h.count(), 0);
    }

    #[test]
    fn one_sample_is_every_quantile_exactly() {
        let h = filled(&[3_217]);
        let d = Duration::from_micros(3_217);
        assert_eq!(h.quantile(0.0), Some(d));
        assert_eq!(h.quantile(0.5), Some(d));
        assert_eq!(h.quantile(0.95), Some(d));
        assert_eq!(h.max(), Some(d));
    }

    #[test]
    fn equal_samples_give_that_value_at_every_quantile() {
        let h = filled(&[12_345; 200]);
        let d = Duration::from_micros(12_345);
        assert_eq!(h.quantile(0.5), Some(d));
        assert_eq!(h.quantile(0.95), Some(d));
        assert_eq!(h.max(), Some(d));
    }

    #[test]
    fn quantiles_are_nearest_rank_within_three_percent() {
        // 1..=1000 ms: p50 is the 500th value, p95 the 950th.
        let values: Vec<u64> = (1..=1000).map(|n| n * 1_000).collect();
        let h = filled(&values);
        for (q, want) in [(0.5, 500_000.0), (0.95, 950_000.0), (1.0, 1_000_000.0)] {
            let got = h.quantile(q).unwrap().as_micros() as f64;
            assert!((got - want).abs() / want < 0.03, "q{q}: {got} vs {want}");
        }
        assert_eq!(h.max(), Some(millis(1000)));
        // Small values are exact.
        let small = filled(&[1, 2, 3, 4, 63]);
        assert_eq!(small.quantile(0.5), Some(Duration::from_micros(3)));
    }

    #[test]
    fn buckets_are_ordered_and_hold_their_values() {
        let mut last = 0;
        for us in (0..5_000_000u64).step_by(997) {
            let b = bucket_of(us);
            assert!(b >= last, "{us} µs went backwards");
            last = b;
            let mid = bucket_mid(b) as f64;
            let err = (mid - us as f64).abs() / (us as f64).max(1.0);
            assert!(err <= 1.0 / 32.0, "{us} µs in a bucket centred on {mid}");
        }
        assert_eq!(bucket_of(u64::MAX), BUCKETS - 1, "past the range clamps");
        let huge = filled(&[1 << 40]);
        assert_eq!(huge.max(), Some(Duration::from_micros(1 << 40)));
        assert_eq!(huge.quantile(0.5), huge.max(), "clamped to the exact max");
    }

    #[test]
    fn merging_is_recording_both() {
        let mut a = filled(&[100, 200, 300]);
        let b = filled(&[400, 50_000]);
        a.merge(&b);
        assert_eq!(a.count(), 5);
        assert_eq!(a.max(), Some(millis(50)));
        assert_eq!(
            a.quantile(0.5),
            Some(Duration::from_micros(300)),
            "the third of five"
        );
    }

    #[test]
    fn only_a_wall_clock_well_ahead_is_a_sleep() {
        assert_eq!(sleep_gap(millis(50), millis(50)), None);
        assert_eq!(sleep_gap(millis(50), millis(1_900)), None, "under the gap");
        assert_eq!(
            sleep_gap(millis(50), millis(467_050)),
            Some(millis(467_000))
        );
        assert_eq!(
            sleep_gap(millis(500), millis(100)),
            None,
            "wall behind is not a sleep"
        );
    }

    #[test]
    fn overruns_per_minute_need_time() {
        assert_eq!(per_minute(3, Duration::ZERO), 0.0);
        assert_eq!(per_minute(3, Duration::from_secs(30)), 6.0);
    }

    /// Runs `n` iterations `step` apart starting at `t`, each with `work`, one session each.
    fn iterate(m: &mut TickMeter, t: &mut (Instant, SystemTime), n: u32, step: Duration) {
        for _ in 0..n {
            m.begin(t.0, t.1, t.0);
            m.session(0, millis(1));
            m.end(millis(2), false);
            t.0 += step;
            t.1 += step;
        }
    }

    #[test]
    fn a_sleep_drops_its_window_and_is_counted() {
        let mut m = TickMeter::new(millis(50));
        let mut t = (
            Instant::now(),
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000),
        );
        let name = |i: usize| format!("s{i}");
        iterate(&mut m, &mut t, 200, millis(50));
        m.close_window();
        iterate(&mut m, &mut t, 100, millis(50));
        // The lid closes: the wall clock runs on, the monotonic one does not.
        t.1 += Duration::from_secs(467);
        assert_eq!(m.begin(t.0, t.1, t.0), Some(Duration::from_secs(467)));
        m.end(millis(2), false);
        m.close_window();
        let line = m.run_line(&name);
        assert!(line.contains(" 201 ticks"), "{line}");
        assert!(
            line.contains("1 sleep windows excluded (467 s asleep)"),
            "{line}"
        );
        assert!(line.contains("10 s measured of 482 s wall"), "{line}");
    }

    #[test]
    fn an_empty_window_prints_dashes() {
        let m = TickMeter::new(millis(50));
        let line = m.window_line(&|i| format!("s{i}"));
        assert_eq!(
            line,
            "perf: 0 sessions, 0 ticks, work p50 - ms p95 - max - of 50 ms, over 0, behind 0, \
             late p95 - ms, per session - ms, costliest -"
        );
    }

    #[test]
    fn the_line_names_the_costliest_session_and_counts_overruns() {
        let mut m = TickMeter::new(millis(50));
        let t = Instant::now();
        let wall = SystemTime::UNIX_EPOCH;
        m.begin(t, wall, t);
        m.session(0, millis(1));
        m.session(1, millis(9));
        m.end(millis(10), false);
        m.begin(t + millis(80), wall + millis(80), t + millis(50));
        m.session(0, millis(1));
        m.session(1, millis(59));
        m.end(millis(60), true);
        let line = m.window_line(&|i| ["+Ann", "+Bob"][i].to_string());
        assert!(line.starts_with("perf: 2 sessions, 2 ticks,"), "{line}");
        assert!(
            line.contains("max 60.0 of 50 ms, over 1, behind 1"),
            "{line}"
        );
        assert!(line.contains("late p95 30.0 ms"), "{line}");
        assert!(line.contains("per session 17.50 ms"), "{line}");
        assert!(line.ends_with("costliest +Bob 34.00 ms"), "{line}");
        m.forget_slots();
        assert!(m.window_line(&|_| unreachable!()).ends_with("costliest -"));
    }
}
