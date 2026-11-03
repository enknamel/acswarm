//! The order a run makes its calls in. Every leg is priced once into [`Legs`]; then every order of
//! up to [`EVERY_ORDER`] calls is tried and the fastest kept, and past that the nearest call goes
//! next. A leg no journey makes is `None`, and an order needing one is no order.

/// Calls up to this many are tried in every order (720 at six); more go nearest first.
pub const EVERY_ORDER: usize = 6;

/// One stop of a run as the legs name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// Where the run sets off from.
    Start,
    /// The call at this index of the calls planned.
    Call(usize),
    /// Where the run ends: the ground it goes back to.
    End,
}

/// The seconds of every leg between a run's stops, planned once.
#[derive(Clone, Debug, PartialEq)]
pub struct Legs {
    calls: usize,
    end: bool,
    /// Row-major over `Start, Call(0..calls), End`.
    secs: Vec<Option<f32>>,
}

impl Legs {
    /// Price every leg a tour may take: start to each call, call to call, each call to the end,
    /// and start to end. `cost(a, b)` is a journey's seconds, `None` when there is no way.
    pub fn plan<P: Copy>(
        from: P,
        calls: &[P],
        end: Option<P>,
        mut cost: impl FnMut(P, P) -> Option<f32>,
    ) -> Legs {
        let n = calls.len();
        let size = n + 2;
        let mut secs = vec![None; size * size];
        for (i, &c) in calls.iter().enumerate() {
            secs[i + 1] = cost(from, c);
            for (j, &d) in calls.iter().enumerate() {
                if i != j {
                    secs[(i + 1) * size + j + 1] = cost(c, d);
                }
            }
            if let Some(e) = end {
                secs[(i + 1) * size + n + 1] = cost(c, e);
            }
        }
        if let Some(e) = end {
            secs[n + 1] = cost(from, e);
        }
        Legs {
            calls: n,
            end: end.is_some(),
            secs,
        }
    }

    /// How many calls the legs were planned for.
    pub fn calls(&self) -> usize {
        self.calls
    }

    fn index(&self, s: Stop) -> usize {
        match s {
            Stop::Start => 0,
            Stop::Call(i) => i + 1,
            Stop::End => self.calls + 1,
        }
    }

    /// The seconds from `a` to `b`; `None` with no way, or to an end the run does not have.
    pub fn leg(&self, a: Stop, b: Stop) -> Option<f32> {
        if (a == Stop::End || b == Stop::End) && !self.end {
            return None;
        }
        self.secs[self.index(a) * (self.calls + 2) + self.index(b)]
    }

    /// The seconds from the last call made (or the start) to the end; nothing when the run has
    /// no end to go back to.
    fn home(&self, last: Stop) -> Option<f32> {
        if self.end {
            self.leg(last, Stop::End)
        } else {
            Some(0.0)
        }
    }

    /// The fastest order to make the calls `among` in (indices of the planned calls), with its
    /// seconds, start to end; `None` when no order makes them all.
    pub fn tour(&self, among: &[usize]) -> Option<(Vec<usize>, f32)> {
        if among.len() > EVERY_ORDER {
            return self.nearest_first(among);
        }
        let mut best: Option<(Vec<usize>, f32)> = None;
        let mut path = Vec::with_capacity(among.len());
        let mut left = among.to_vec();
        self.every_order(Stop::Start, 0.0, &mut path, &mut left, &mut best);
        best
    }

    /// Depth first over the calls `left`, keeping in `best` the fastest complete order.
    fn every_order(
        &self,
        at: Stop,
        so_far: f32,
        path: &mut Vec<usize>,
        left: &mut Vec<usize>,
        best: &mut Option<(Vec<usize>, f32)>,
    ) {
        if best.as_ref().is_some_and(|(_, b)| so_far >= *b) {
            return;
        }
        if left.is_empty() {
            if let Some(home) = self.home(at) {
                let total = so_far + home;
                if best.as_ref().is_none_or(|(_, b)| total < *b) {
                    *best = Some((path.clone(), total));
                }
            }
            return;
        }
        for k in 0..left.len() {
            let call = left[k];
            let Some(leg) = self.leg(at, Stop::Call(call)) else {
                continue;
            };
            left.remove(k);
            path.push(call);
            self.every_order(Stop::Call(call), so_far + leg, path, left, best);
            path.pop();
            left.insert(k, call);
        }
    }

    /// Past [`EVERY_ORDER`]: from each stop, the nearest call not yet made.
    fn nearest_first(&self, among: &[usize]) -> Option<(Vec<usize>, f32)> {
        let mut left = among.to_vec();
        let (mut at, mut total, mut order) = (Stop::Start, 0.0, Vec::new());
        while !left.is_empty() {
            let (k, leg) = left
                .iter()
                .enumerate()
                .filter_map(|(k, &c)| Some((k, self.leg(at, Stop::Call(c))?)))
                .min_by(|a, b| a.1.total_cmp(&b.1))?;
            let call = left.remove(k);
            order.push(call);
            total += leg;
            at = Stop::Call(call);
        }
        Some((order, total + self.home(at)?))
    }
}

/// The fastest order to make `calls` in from `from`, ending at `end` when there is one: the
/// order (indices into `calls`) and its seconds. `None` when no order makes every call.
pub fn tour<P: Copy>(
    from: P,
    calls: &[P],
    end: Option<P>,
    cost: impl FnMut(P, P) -> Option<f32>,
) -> Option<(Vec<usize>, f32)> {
    let legs = Legs::plan(from, calls, end, cost);
    let all: Vec<usize> = (0..calls.len()).collect();
    legs.tour(&all)
}

#[cfg(test)]
mod tests;
