//! Headless tick timing and pacing: the loop's `--tick-hz` schedule, each iteration's work and
//! lateness against it, the rate achieved, and each session's own cost apart from the plugins',
//! as a `perf:` line per status period and a `perf run:` line at exit (tools/load-test.sh reads
//! it). Instant stops while the machine sleeps and the boot clock does not, so a window a sleep
//! fell in is dropped with a warning rather than reported as a measurement.

use std::time::{Duration, Instant};

/// Boot clock time this far past Instant between two iterations is a machine sleep.
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

/// Time since boot with sleep counted, on a clock nobody can set: CLOCK_MONOTONIC on macOS and
/// CLOCK_BOOTTIME on Linux, where Instant (CLOCK_UPTIME_RAW, CLOCK_MONOTONIC) stops asleep.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn boot_clock() -> Duration {
    #[cfg(target_os = "macos")]
    const CLOCK: libc::clockid_t = libc::CLOCK_MONOTONIC;
    #[cfg(target_os = "linux")]
    const CLOCK: libc::clockid_t = libc::CLOCK_BOOTTIME;
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `clock_gettime` writes only into `ts`, which outlives the call; it fails only for
    // a clock the system lacks, and each of these is its own system's.
    unsafe { libc::clock_gettime(CLOCK, &mut ts) };
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

/// Elsewhere the wall clock, so setting it there reads as a sleep or hides one.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn boot_clock() -> Duration {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
}

/// How long the machine slept between two iterations `mono` (Instant) and `boot` apart, if it did.
pub fn sleep_gap(mono: Duration, boot: Duration) -> Option<Duration> {
    let gap = boot.saturating_sub(mono);
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

/// One session's ticks and the time they took.
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

/// Add `spent` over `ticks` to `account`'s row, trying row `hint` (its slot, as a rule) first.
fn add_to_account(
    rows: &mut Vec<(String, SessionCost)>,
    hint: usize,
    account: &str,
    spent: Duration,
    ticks: u64,
) {
    let row = match rows.get(hint) {
        Some((a, _)) if a == account => hint,
        _ => match rows.iter().position(|(a, _)| a == account) {
            Some(row) => row,
            None => {
                rows.push((account.to_string(), SessionCost::default()));
                rows.len() - 1
            }
        },
    };
    rows[row].1.spent += spent;
    rows[row].1.ticks += ticks;
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
    /// Instant time the stretch covered: one interval between begins per iteration.
    span: Duration,
    /// Every session tick together, for the mean.
    all: SessionCost,
    /// The plugin host's time, every session's calls together.
    plugins: Duration,
    /// By account, which a stop that moves the slots down does not change; for the costliest.
    accounts: Vec<(String, SessionCost)>,
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
        self.plugins += other.plugins;
        for (row, (account, cost)) in other.accounts.iter().enumerate() {
            if cost.ticks > 0 {
                add_to_account(&mut self.accounts, row, account, cost.spent, cost.ticks);
            }
        }
    }

    /// Back to empty, keeping the accounts' rows.
    fn clear(&mut self) {
        self.work.clear();
        self.late.clear();
        self.over = 0;
        self.behind = 0;
        self.span = Duration::ZERO;
        self.all = SessionCost::default();
        self.plugins = Duration::ZERO;
        for (_, cost) in &mut self.accounts {
            *cost = SessionCost::default();
        }
    }

    /// The account with the highest mean tick cost, and that mean.
    fn costliest(&self) -> Option<(&str, Duration)> {
        self.accounts
            .iter()
            .filter_map(|(a, c)| c.mean().map(|m| (a.as_str(), m)))
            .max_by_key(|&(_, m)| m)
    }

    /// The line's body: sessions, ticks and their rate, work and lateness against `period`, and
    /// the sessions' and plugins' costs; `name_of` turns an account into what the line calls it.
    pub fn summary(&self, period: Duration, name_of: &dyn Fn(&str) -> String) -> String {
        let sessions = self.accounts.iter().filter(|(_, c)| c.ticks > 0).count();
        let ticks = self.work.count();
        let rate = match ticks {
            0 => "-".to_string(),
            _ if self.span.is_zero() => "-".to_string(),
            n => format!("{:.1}", n as f64 / self.span.as_secs_f64()),
        };
        let plugins = (ticks > 0).then(|| self.plugins / u32::try_from(ticks).unwrap_or(u32::MAX));
        let costliest = match self.costliest() {
            Some((account, mean)) => format!("{} {} ms", name_of(account), ms(Some(mean), 2)),
            None => "-".to_string(),
        };
        format!(
            "{sessions} sessions, {ticks} ticks at {rate} of {:.0} Hz, work p50 {} ms p95 {} max \
             {} of {} ms, over {}, behind {}, late p95 {} ms, per session {} ms, plugins {} ms, \
             costliest {costliest}",
            1.0 / period.as_secs_f64(),
            ms(self.work.quantile(0.5), 1),
            ms(self.work.quantile(0.95), 1),
            ms(self.work.max(), 1),
            ms(Some(period), 1).trim_end_matches(".0"),
            self.over,
            self.behind,
            ms(self.late.quantile(0.95), 1),
            ms(self.all.mean(), 2),
            ms(plugins, 2),
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

/// The headless loop's schedule and timing, fed by `begin`, `session`, `plugins` and `end` once
/// per iteration; `end` says how long to sleep, so the schedule measured is the one kept.
pub struct TickMeter {
    period: Duration,
    /// When the next iteration is due.
    next: Instant,
    /// When the iteration under way was due: the tick it missed when the loop started over.
    due: Instant,
    window: TickStats,
    run: TickStats,
    /// When the first iteration began, on Instant and the boot clock, for the wall time spanned.
    first: Option<(Instant, Duration)>,
    /// When the last iteration began, on both clocks.
    last: Option<(Instant, Duration)>,
    /// The lateness of the iteration under way, recorded once its work is known.
    late: Duration,
    /// The plugin host's time in the iteration under way.
    plugins: Duration,
    /// Windows dropped for a machine sleep.
    sleeps: u32,
    /// How long the machine slept, all the dropped windows together.
    slept: Duration,
}

impl TickMeter {
    /// A loop ticking every `period`, the first iteration due at `start`.
    pub fn new(period: Duration, start: Instant) -> Self {
        TickMeter {
            period,
            next: start,
            due: start,
            window: TickStats::default(),
            run: TickStats::default(),
            first: None,
            last: None,
            late: Duration::ZERO,
            plugins: Duration::ZERO,
            sleeps: 0,
            slept: Duration::ZERO,
        }
    }

    /// An iteration begins at `now` (`boot` on [`boot_clock`]). A sleep since the last one
    /// drops the window under way and is returned.
    pub fn begin(&mut self, now: Instant, boot: Duration) -> Option<Duration> {
        let mut slept = None;
        if let Some((mono0, boot0)) = self.last {
            let mono = now.saturating_duration_since(mono0);
            slept = sleep_gap(mono, boot.saturating_sub(boot0));
            if let Some(gap) = slept {
                tracing::warn!(
                    "the machine slept for about {:.0} s: the boot clock ran that far past the \
                     monotonic one; this window is left out of the measurements",
                    gap.as_secs_f64()
                );
                self.sleeps += 1;
                self.slept += gap;
                self.window.clear();
            }
            // Its awake part only, so that the span and the ticks count the same iterations.
            self.window.span += mono;
        }
        self.first.get_or_insert((now, boot));
        self.last = Some((now, boot));
        self.late = now.saturating_duration_since(self.due);
        self.plugins = Duration::ZERO;
        slept
    }

    /// Session `slot`, logged in as `account`, took `spent` of its own this iteration: its
    /// tick, its events and the lines it typed, not the plugin host's frame.
    pub fn session(&mut self, slot: usize, account: &str, spent: Duration) {
        add_to_account(&mut self.window.accounts, slot, account, spent, 1);
        self.window.all.spent += spent;
        self.window.all.ticks += 1;
    }

    /// The plugin host took `spent` this iteration, on whichever session's call.
    pub fn plugins(&mut self, spent: Duration) {
        self.plugins += spent;
    }

    /// The iteration that began at `now` ended its work at `after`: the sleep until the next is
    /// due, or `None` when the loop fell behind and gives the missed ticks up rather than burst.
    pub fn end(&mut self, now: Instant, after: Instant) -> Option<Duration> {
        let work = after.saturating_duration_since(now);
        self.next += self.period;
        // Kept when the loop starts over, so the next iteration is late by what this one overran.
        self.due = self.next;
        let behind = self.next <= after;
        self.window.work.record(work);
        self.window.late.record(self.late);
        self.window.over += u64::from(work > self.period);
        self.window.behind += u64::from(behind);
        self.window.plugins += self.plugins;
        if behind {
            self.next = after;
            None
        } else {
            Some(self.next - after)
        }
    }

    /// The `perf:` line for the window under way.
    pub fn window_line(&self, name_of: &dyn Fn(&str) -> String) -> String {
        format!("perf: {}", self.window.summary(self.period, name_of))
    }

    /// Fold the window under way into the run and start the next one.
    pub fn close_window(&mut self) {
        self.run.merge(&self.window);
        self.window.clear();
    }

    /// Log the window's line and start the next window.
    pub fn report(&mut self, name_of: &dyn Fn(&str) -> String) {
        tracing::info!("{}", self.window_line(name_of));
        self.close_window();
    }

    /// The `perf run:` line: the whole run, with what was left out of it and why. Its shape is
    /// pinned by `the_run_line_has_the_shape_the_harness_reads`: tools/load-test.sh parses it.
    pub fn run_line(&self, name_of: &dyn Fn(&str) -> String) -> String {
        let wall = match (self.first, self.last) {
            (Some((_, b0)), Some((_, b1))) => b1.saturating_sub(b0),
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
    pub fn finish(&mut self, name_of: &dyn Fn(&str) -> String) {
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
    fn only_a_boot_clock_well_ahead_is_a_sleep() {
        assert_eq!(sleep_gap(millis(50), millis(50)), None);
        assert_eq!(sleep_gap(millis(50), millis(1_900)), None, "under the gap");
        assert_eq!(
            sleep_gap(millis(50), millis(467_050)),
            Some(millis(467_000))
        );
        assert_eq!(
            sleep_gap(millis(500), millis(100)),
            None,
            "boot clock behind is not a sleep"
        );
    }

    #[test]
    fn the_boot_clock_keeps_pace_with_instant_while_awake() {
        let (b0, i0) = (boot_clock(), Instant::now());
        std::thread::sleep(millis(20));
        let (b1, i1) = (boot_clock(), Instant::now());
        let (boot, mono) = (b1 - b0, i1 - i0);
        assert!(boot >= millis(20), "{boot:?}");
        assert_eq!(sleep_gap(mono, boot), None, "{boot:?} against {mono:?}");
    }

    #[test]
    fn overruns_per_minute_need_time() {
        assert_eq!(per_minute(3, Duration::ZERO), 0.0);
        assert_eq!(per_minute(3, Duration::from_secs(30)), 6.0);
    }

    /// Runs `n` iterations `step` apart from `t` (Instant, boot clock), 2 ms of work each.
    fn iterate(m: &mut TickMeter, t: &mut (Instant, Duration), n: u32, step: Duration) {
        for _ in 0..n {
            m.begin(t.0, t.1);
            m.session(0, "ann", millis(1));
            m.end(t.0, t.0 + millis(2));
            t.0 += step;
            t.1 += step;
        }
    }

    #[test]
    fn a_sleep_drops_its_window_and_is_counted() {
        let start = Instant::now();
        let mut m = TickMeter::new(millis(50), start);
        let mut t = (start, Duration::from_secs(1_000_000));
        let name = |a: &str| a.to_string();
        iterate(&mut m, &mut t, 200, millis(50));
        m.close_window();
        iterate(&mut m, &mut t, 100, millis(50));
        // The lid closes: the boot clock runs on, Instant does not.
        t.1 += Duration::from_secs(467);
        assert_eq!(m.begin(t.0, t.1), Some(Duration::from_secs(467)));
        m.end(t.0, t.0 + millis(2));
        m.close_window();
        let line = m.run_line(&name);
        assert!(line.contains(" 201 ticks at 20.1 of 20 Hz"), "{line}");
        assert!(
            line.contains("1 sleep windows excluded (467 s asleep)"),
            "{line}"
        );
        assert!(line.contains("10 s measured of 482 s wall"), "{line}");
    }

    #[test]
    fn a_loop_that_keeps_up_sleeps_to_its_schedule() {
        let t = Instant::now();
        let mut m = TickMeter::new(millis(50), t);
        m.begin(t, Duration::ZERO);
        assert_eq!(m.end(t, t + millis(1)), Some(millis(49)));
        // The sleep ran 10 ms long; the schedule does not move for it.
        m.begin(t + millis(60), millis(60));
        assert_eq!(m.end(t + millis(60), t + millis(61)), Some(millis(39)));
        let line = m.window_line(&|a| a.to_string());
        assert!(line.contains("late p95 10.0 ms"), "{line}");
        assert!(line.contains("over 0, behind 0"), "{line}");
    }

    #[test]
    fn a_loop_that_falls_behind_is_late_by_what_it_overran() {
        let t = Instant::now();
        let mut m = TickMeter::new(millis(50), t);
        let mut now = t;
        // 52 ms of work a 50 ms tick: each iteration starts 2 ms past the tick it missed.
        for _ in 0..40 {
            m.begin(now, now - t);
            assert_eq!(m.end(now, now + millis(52)), None);
            now += millis(52);
        }
        m.begin(now, now - t);
        let line = m.window_line(&|a| a.to_string());
        assert!(line.contains("40 ticks at 19.2 of 20 Hz"), "{line}");
        assert!(
            line.contains("over 40, behind 40, late p95 2.0 ms"),
            "{line}"
        );
    }

    #[test]
    fn an_empty_window_prints_dashes() {
        let m = TickMeter::new(millis(50), Instant::now());
        let line = m.window_line(&|a| a.to_string());
        assert_eq!(
            line,
            "perf: 0 sessions, 0 ticks at - of 20 Hz, work p50 - ms p95 - max - of 50 ms, over 0, \
             behind 0, late p95 - ms, per session - ms, plugins - ms, costliest -"
        );
    }

    #[test]
    fn the_run_line_has_the_shape_the_harness_reads() {
        let t = Instant::now();
        let mut m = TickMeter::new(millis(50), t);
        // (began, work ms, +Bob's ms): the second overruns, so the third starts 22 ms late.
        for (at, work, bob) in [(0, 12, 9), (60, 62, 59), (122, 12, 9)] {
            m.begin(t + millis(at), millis(at));
            m.session(0, "ann", millis(1));
            m.session(1, "bob", millis(bob));
            m.plugins(millis(2));
            m.end(t + millis(at), t + millis(at + work));
        }
        m.begin(t + millis(172), millis(172));
        m.close_window();
        let names = |a: &str| if a == "bob" { "+Bob" } else { "+Ann" }.to_string();
        assert_eq!(
            m.run_line(&names),
            "perf run: 2 sessions, 3 ticks at 17.4 of 20 Hz, work p50 12.0 ms p95 62.0 max 62.0 \
             of 50 ms, over 1, behind 1, late p95 21.8 ms, per session 13.33 ms, plugins 2.00 \
             ms, costliest +Bob 25.67 ms, 348.8 overruns/min, 0 s measured of 0 s wall, 0 sleep \
             windows excluded (0 s asleep)"
        );
    }

    #[test]
    fn costs_follow_the_account_when_a_stop_moves_the_slots() {
        let t = Instant::now();
        let mut m = TickMeter::new(millis(50), t);
        m.begin(t, Duration::ZERO);
        m.session(0, "ann", millis(1));
        m.session(1, "bob", millis(9));
        m.end(t, t + millis(10));
        m.close_window();
        // Ann is stopped and Bob moves down to slot 0.
        m.begin(t + millis(50), millis(50));
        m.session(0, "bob", millis(11));
        m.end(t + millis(50), t + millis(61));
        m.close_window();
        let line = m.run_line(&|a| a.to_string());
        assert!(line.starts_with("perf run: 2 sessions, 2 ticks"), "{line}");
        assert!(line.contains("costliest bob 10.00 ms"), "{line}");
    }
}
