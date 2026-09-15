//! What came of trying something ([`Did`], [`Because`]) and when to try again ([`Patience`]).
//! One policy for every system, so none asks again at once for ever by default; see `docs/agent.md`.

use std::fmt;
use std::time::{Duration, Instant};

/// Why something did not happen: a phrase for the log and panel, and the server's code if it gave one.
/// The code lets the retry policy tell a cooldown from a full pack without matching English.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Because {
    /// In a few words: "the pack is full", "no vendor will take it".
    pub what: String,
    /// The server's own code, when it gave one (see [`crate::weenie_errors`]).
    pub code: Option<u32>,
}

impl Because {
    /// A reason of our own.
    pub fn ours(what: impl Into<String>) -> Because {
        Because {
            what: what.into(),
            code: None,
        }
    }

    /// A reason the server gave, named by its own error code.
    pub fn server(code: u32) -> Because {
        Because {
            what: crate::weenie_errors::text(code)
                .map(str::to_string)
                .unwrap_or_else(|| format!("the server said no ({code:#06x})")),
            code: Some(code),
        }
    }

    /// The server's code with a reason of our own for context.
    pub fn server_because(code: u32, what: impl Into<String>) -> Because {
        Because {
            what: what.into(),
            code: Some(code),
        }
    }
}

impl fmt::Display for Because {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.what)
    }
}

/// What came of trying something.
/// The three unhappy answers differ in how long to wait before asking again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Did {
    /// It claimed the tick and is working; nothing else runs.
    Acting,
    /// Nothing to do, or finished; let something else have the tick.
    Done,
    /// Cannot go on yet, for a reason that passes by itself (a cast, a walk, a reply): ask again soon.
    Waiting(Because),
    /// Cannot go on until something changes (a full pack, an empty purse, too laden): back off.
    Blocked(Because),
    /// Will never work for this thing (no vendor takes it, a quest door not done): never ask again.
    Refused(Because),
}

impl Did {
    /// It claimed the tick; the caller stops here.
    pub fn acting(&self) -> bool {
        matches!(self, Did::Acting)
    }

    /// Nothing went wrong: it is acting, or had nothing to do.
    pub fn fine(&self) -> bool {
        matches!(self, Did::Acting | Did::Done)
    }

    /// Why not, when something did go wrong.
    pub fn because(&self) -> Option<&Because> {
        match self {
            Did::Acting | Did::Done => None,
            Did::Waiting(b) | Did::Blocked(b) | Did::Refused(b) => Some(b),
        }
    }

    /// Shorthand for the common unhappy answers.
    pub fn waiting(what: impl Into<String>) -> Did {
        Did::Waiting(Because::ours(what))
    }

    pub fn blocked(what: impl Into<String>) -> Did {
        Did::Blocked(Because::ours(what))
    }

    pub fn refused(what: impl Into<String>) -> Did {
        Did::Refused(Because::ours(what))
    }

    /// How long before trying this again the first time; `None` means never.
    pub fn wait(&self) -> Option<Duration> {
        match self {
            Did::Acting | Did::Done => Some(Duration::ZERO),
            Did::Waiting(_) => Some(WAIT_AGAIN),
            Did::Blocked(_) => Some(BLOCKED_AGAIN),
            Did::Refused(_) => None,
        }
    }
}

/// Wait after a `Waiting`: a few ticks, as what it waits on is usually the server's reply.
const WAIT_AGAIN: Duration = Duration::from_millis(250);
/// First wait after a `Blocked`: something has to change first, and nothing does in a quarter second.
const BLOCKED_AGAIN: Duration = Duration::from_secs(30);
/// Ceiling of a doubling wait, short against a daily cooldown on purpose: a failed ask costs one message.
/// Sessions run for days, so nothing is given up for good except by `Did::Refused`.
const AT_MOST: Duration = Duration::from_secs(4 * 60 * 60);

/// When to try one particular thing again: each refusal doubles the wait to a ceiling; success clears it.
/// Biased short: asking again costs one message, not asking when the answer changed costs the thing.
#[derive(Clone, Debug, Default)]
pub struct Patience<K: Ord + Clone> {
    held: std::collections::BTreeMap<K, Held>,
}

#[derive(Clone, Copy, Debug)]
struct Held {
    /// When it was last refused.
    since: Instant,
    /// How long to leave it from `since`.
    wait: Duration,
    /// Refused for good.
    never: bool,
}

impl<K: Ord + Clone> Patience<K> {
    pub fn new() -> Self {
        Patience {
            held: std::collections::BTreeMap::new(),
        }
    }

    /// Whether this one is still inside its wait; a thing refused for good is always held.
    pub fn held(&self, key: &K, now: Instant) -> bool {
        self.held.get(key).is_some_and(|h| {
            h.never
                || now
                    .checked_duration_since(h.since)
                    .is_none_or(|d| d < h.wait)
        })
    }

    /// Remember what came of trying it: going well forgets any wait, being told no sets or doubles one.
    pub fn note(&mut self, key: K, did: &Did, now: Instant) {
        match did {
            Did::Acting | Did::Done => {
                self.held.remove(&key);
            }
            Did::Refused(_) => {
                self.held.insert(
                    key,
                    Held {
                        since: now,
                        wait: AT_MOST,
                        never: true,
                    },
                );
            }
            Did::Waiting(_) | Did::Blocked(_) => {
                let first = did.wait().unwrap_or(BLOCKED_AGAIN);
                let held = self.held.entry(key).or_insert(Held {
                    since: now,
                    wait: first,
                    never: false,
                });
                // The first refusal sets the wait; each later one doubles it: asked less, never dropped.
                if held.since != now {
                    held.wait = (held.wait * 2).min(AT_MOST);
                }
                held.since = now;
            }
        }
    }

    /// How long the wait on this one has grown to, if it is held at all.
    /// Tells "not yet" from "not coming" for a caller that asks in order: a doubled wait has not budged.
    pub fn waited(&self, key: &K) -> Option<Duration> {
        self.held.get(key).map(|h| h.wait)
    }

    /// Hold one off for a first wait the caller chooses, doubling from there like any other.
    /// For callers that know better: a town trip that came to nothing is worth minutes, not half a minute.
    pub fn hold(&mut self, key: K, first: Duration, now: Instant) {
        match self.held.get_mut(&key) {
            Some(held) if !held.never => {
                held.wait = (held.wait * 2).min(AT_MOST);
                held.since = now;
            }
            Some(_) => {}
            None => {
                self.held.insert(
                    key,
                    Held {
                        since: now,
                        wait: first.min(AT_MOST),
                        never: false,
                    },
                );
            }
        }
    }

    /// Forget one, whatever was remembered: the thing it waited on has changed.
    pub fn forget(&mut self, key: &K) {
        self.held.remove(key);
    }

    /// Forget everything.
    pub fn clear(&mut self) {
        self.held.clear();
    }

    /// Keep what `keep` wants (a corpse not yet rotted) with lapsed waits intact, so the next refusal doubles.
    /// `tidy` would restart it: a_lapsed_wait_that_is_kept_doubles_where_a_tidied_one_starts_again.
    pub fn retain(&mut self, mut keep: impl FnMut(&K) -> bool) {
        self.held.retain(|k, _| keep(k));
    }

    /// Drop lapsed waits that are not permanent, keeping the table the size of what is held off.
    pub fn tidy(&mut self, now: Instant) {
        self.held.retain(|_, h| {
            h.never
                || now
                    .checked_duration_since(h.since)
                    .is_none_or(|d| d < h.wait)
        });
    }

    /// How many are being held off, for the log.
    pub fn len(&self) -> usize {
        self.held.len()
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_says_how_long_to_wait() {
        assert!(Did::Acting.acting());
        assert!(Did::Acting.fine() && Did::Done.fine());
        assert!(!Did::waiting("the server has not answered").fine());
        assert_eq!(Did::Done.wait(), Some(Duration::ZERO));
        assert_eq!(Did::waiting("x").wait(), Some(WAIT_AGAIN));
        assert_eq!(Did::blocked("x").wait(), Some(BLOCKED_AGAIN));
        assert_eq!(Did::refused("no vendor will take it").wait(), None);
        assert_eq!(
            Did::blocked("the pack is full")
                .because()
                .map(|b| b.what.as_str()),
            Some("the pack is full")
        );
        assert_eq!(Did::Done.because(), None);
    }

    #[test]
    fn a_reason_from_the_server_keeps_its_code() {
        // 0x043E YouHaveSolvedThisQuestTooRecently: the code tells a cooldown from a full pack.
        let b = Because::server(0x043E);
        assert_eq!(b.code, Some(0x043E));
        assert_eq!(b.what, "You have solved this quest too recently");
        let odd = Because::server(0xDEAD);
        assert_eq!(odd.code, Some(0xDEAD));
        assert!(odd.what.contains("0xdead"), "{}", odd.what);
        assert_eq!(Because::ours("the pack is full").code, None);
    }

    #[test]
    fn a_refusal_doubles_the_wait_and_going_well_forgets_it() {
        let t0 = Instant::now();
        let mut p: Patience<u32> = Patience::new();
        assert!(!p.held(&1, t0));

        p.note(1, &Did::blocked("the pack is full"), t0);
        assert!(p.held(&1, t0));
        assert!(p.held(&1, t0 + BLOCKED_AGAIN - Duration::from_secs(1)));
        assert!(!p.held(&1, t0 + BLOCKED_AGAIN));

        let t1 = t0 + BLOCKED_AGAIN;
        p.note(1, &Did::blocked("the pack is full"), t1);
        assert!(p.held(&1, t1 + BLOCKED_AGAIN));
        assert!(!p.held(&1, t1 + BLOCKED_AGAIN * 2));

        p.note(1, &Did::Done, t1 + BLOCKED_AGAIN * 2);
        assert!(!p.held(&1, t1 + BLOCKED_AGAIN * 2));
        assert!(p.is_empty());
    }

    #[test]
    fn a_caller_may_choose_its_own_first_wait() {
        let t0 = Instant::now();
        let mut p: Patience<u32> = Patience::new();
        let five = Duration::from_secs(5 * 60);
        p.hold(1, five, t0);
        assert!(p.held(&1, t0 + five - Duration::from_secs(1)));
        assert!(!p.held(&1, t0 + five));
        p.hold(1, five, t0 + five);
        assert!(p.held(&1, t0 + five * 2));
        assert!(!p.held(&1, t0 + five * 3));
        let mut q: Patience<u32> = Patience::new();
        q.hold(1, Duration::from_secs(365 * 24 * 60 * 60), t0);
        assert_eq!(q.waited(&1), Some(AT_MOST));
        // A chosen wait leaves a thing already refused for good refused.
        q.note(2, &Did::refused("never"), t0);
        q.hold(2, five, t0);
        assert!(q.held(&2, t0 + Duration::from_secs(24 * 60 * 60)));
    }

    #[test]
    fn only_a_refusal_is_for_ever() {
        let t0 = Instant::now();
        let mut p: Patience<u32> = Patience::new();
        p.note(7, &Did::refused("no vendor will take it"), t0);
        assert!(p.held(&7, t0));
        assert!(p.held(&7, t0 + Duration::from_secs(24 * 60 * 60)));
        p.note(8, &Did::blocked("too heavy"), t0);
        assert!(!p.held(&8, t0 + AT_MOST));
        p.tidy(t0 + AT_MOST);
        assert_eq!(p.len(), 1);
        assert!(p.held(&7, t0 + AT_MOST));
        p.forget(&7);
        assert!(p.is_empty());
    }

    #[test]
    fn a_lapsed_wait_that_is_kept_doubles_where_a_tidied_one_starts_again() {
        // Tidied at its lapse, a corpse's next refusal counts as the first, so it is asked every 30 s
        // until it rots.
        let t0 = Instant::now();
        let no = Did::blocked("it will not open yet");
        let again = t0 + BLOCKED_AGAIN;

        let mut tidied: Patience<u32> = Patience::new();
        tidied.note(1, &no, t0);
        tidied.tidy(again);
        tidied.note(1, &no, again);
        assert!(!tidied.held(&1, again + BLOCKED_AGAIN), "it doubled");

        let mut kept: Patience<u32> = Patience::new();
        kept.note(1, &no, t0);
        kept.note(2, &no, t0);
        kept.retain(|k| *k == 1);
        assert_eq!(kept.len(), 1);
        assert!(!kept.held(&1, again), "a lapsed wait is still a wait");
        kept.note(1, &no, again);
        assert!(kept.held(&1, again + BLOCKED_AGAIN), "it did not double");
        assert!(!kept.held(&1, again + BLOCKED_AGAIN * 2));
    }

    #[test]
    fn a_wait_never_grows_past_the_ceiling() {
        let t0 = Instant::now();
        let mut p: Patience<u32> = Patience::new();
        let mut at = t0;
        for _ in 0..20 {
            p.note(1, &Did::blocked("still no"), at);
            at += AT_MOST;
        }
        p.note(1, &Did::blocked("still no"), at);
        assert!(!p.held(&1, at + AT_MOST));
        assert!(AT_MOST < Duration::from_secs(24 * 60 * 60));
    }
}
