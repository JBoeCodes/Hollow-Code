//! J3a (2026-11-15): a chat turn's recall as a prefetch. Admission
//! (`chat_queue::admit_checked_turn`) starts the turn's inputs, recall
//! included, on a `hollow-recall-prefetch` thread before it takes the lease
//! (whose vault snapshot is about 126 ms at p50, run 15's speed baseline), so
//! the query embedding runs beside admission instead of after it. The turn
//! then takes the answer if it is ready within a short cutoff
//! ([`RECALL_PREFETCH_CUTOFF`], none at all while recalls run late:
//! `vault_embed_index::recall_contended`) and never past
//! `vault_embed_index::RECALL_MEANING_WAIT` after the prefetch started.
//! Past that it ranks by stems itself (0.3 ms) and the late answer is
//! dropped; the embedding it started still lands in `embed_model`'s query
//! cache, where retrieval finds it, as before. Each pickup is logged with
//! whether the person sent the turn or an agent did, so the two are read
//! apart (plan row J3a, N34).
//!
//! The budget is counted from the prefetch's start. For a turn on the vault
//! lease that is before the lease's snapshot (about 126 ms), so by the time
//! the turn asks the budget is usually spent and the cutoff means "take it
//! if it is done"; only a scoped turn, admitted faster, can wait at all.
//! The prefetch ranks recall without recording its usage
//! (`turn_request::unrecorded`); the turn records the recall it uses, once
//! (`chat_queue::turn_inputs_prefetched`), so a prefetch a deferred
//! admission throws away counts nothing. A row that already waited for the
//! lease starts its prefetch only once the lease is held.
//!
//! Generic over the answer so the timing rules are tested without a vault.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

/// The most a turn waits at its start for its prefetched recall. The
/// prefetch has had admission's time already; an idle warm query takes
/// 28 ms at p50 (`recall_probe`, J4), so this covers a turn admitted faster
/// than the query embeds and nothing more.
pub(crate) const RECALL_PREFETCH_CUTOFF: Duration = Duration::from_millis(25);

/// Work started ahead of the turn that reads it.
pub(crate) struct Prefetch<T> {
    started: Instant,
    /// `None` when the thread couldn't be spawned.
    answer: Option<Receiver<T>>,
}

/// How a turn found its prefetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Ready within the cutoff: the turn used it.
    Ready,
    /// Not ready within the cutoff: the turn went on without it.
    Late,
    /// No thread, or it died: the turn went on without it.
    Failed,
}

impl Outcome {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Outcome::Ready => "ready",
            Outcome::Late => "late",
            Outcome::Failed => "failed",
        }
    }
}

/// What a turn's pickup of its prefetch found, for the log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Pickup {
    pub outcome: Outcome,
    /// From the prefetch's start to the turn asking for it.
    pub age: Duration,
    /// How long the turn waited for it.
    pub waited: Duration,
}

/// Who a turn was for: the person's own message, or an agent's or
/// Hollow's (a subagent, a wake, a check-in). Reported apart (N34).
pub(crate) fn who(person_sent: bool) -> &'static str {
    if person_sent {
        "own"
    } else {
        "agent"
    }
}

impl<T: Send + 'static> Prefetch<T> {
    /// Start `work` on its own thread now.
    pub(crate) fn start(work: impl FnOnce() -> T + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let started = Instant::now();
        let spawned = std::thread::Builder::new()
            .name("hollow-recall-prefetch".into())
            .spawn(move || {
                // A turn that went on without it has dropped the receiver.
                let _ = tx.send(work());
            });
        Prefetch {
            started,
            answer: spawned.ok().map(|_| rx),
        }
    }

    /// The answer if it arrives within `cutoff` from now and within `budget`
    /// of the prefetch's start, whichever is sooner; otherwise `None`, and
    /// the answer, whenever it comes, is dropped.
    pub(crate) fn take(self, cutoff: Duration, budget: Duration) -> (Option<T>, Pickup) {
        let asked = Instant::now();
        let age = asked.saturating_duration_since(self.started);
        let Some(answer) = self.answer else {
            let pickup = Pickup {
                outcome: Outcome::Failed,
                age,
                waited: Duration::ZERO,
            };
            return (None, pickup);
        };
        let wait = cutoff.min(budget.saturating_sub(age));
        // A zero wait still takes an answer that is already there.
        let got = answer.recv_timeout(wait);
        let waited = asked.elapsed();
        match got {
            Ok(value) => (
                Some(value),
                Pickup {
                    outcome: Outcome::Ready,
                    age,
                    waited,
                },
            ),
            Err(RecvTimeoutError::Timeout) => (
                None,
                Pickup {
                    outcome: Outcome::Late,
                    age,
                    waited,
                },
            ),
            Err(RecvTimeoutError::Disconnected) => (
                None,
                Pickup {
                    outcome: Outcome::Failed,
                    age,
                    waited,
                },
            ),
        }
    }
}

/// The turn's log line for its pickup (J3a): own and agent turns apart.
pub(crate) fn log_pickup(person_sent: bool, pickup: &Pickup) {
    eprintln!(
        "[recall] {} turn: prefetch {} {} ms after accept, turn waited {} ms",
        who(person_sent),
        pickup.outcome.name(),
        pickup.age.as_millis(),
        pickup.waited.as_millis()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    #[test]
    fn a_ready_prefetch_is_taken_without_waiting_out_the_cutoff() {
        let p = Prefetch::start(|| 7);
        // Let the thread answer first.
        std::thread::sleep(Duration::from_millis(20));
        let (got, pickup) = p.take(Duration::from_secs(5), Duration::from_secs(5));
        assert_eq!(got, Some(7));
        assert_eq!(pickup.outcome, Outcome::Ready);
        // timing: the cutoff must hold the turn to far less than the blocked work (several seconds)
        assert!(pickup.waited < Duration::from_secs(1), "{pickup:?}");
        assert!(pickup.age >= Duration::from_millis(20), "{pickup:?}");
    }

    #[test]
    fn a_late_prefetch_costs_the_turn_at_most_the_cutoff_and_is_dropped() {
        let (release, gate) = channel::<()>();
        let (done_tx, done) = channel::<()>();
        let p = Prefetch::start(move || {
            let _ = gate.recv();
            let _ = done_tx.send(());
            9
        });
        let (got, pickup) = p.take(Duration::from_millis(30), Duration::from_secs(5));
        assert_eq!(got, None);
        assert_eq!(pickup.outcome, Outcome::Late);
        assert!(pickup.waited >= Duration::from_millis(30), "{pickup:?}");
        // timing: the cutoff must hold the turn to far less than the blocked work (several seconds)
        assert!(pickup.waited < Duration::from_secs(2), "{pickup:?}");
        // The work still finishes on its own thread; its answer goes nowhere.
        release.send(()).unwrap();
        done.recv_timeout(Duration::from_secs(5)).unwrap();
    }

    #[test]
    fn the_budget_from_accept_shortens_the_turns_wait_to_nothing() {
        let (release, gate) = channel::<()>();
        let p = Prefetch::start(move || {
            let _ = gate.recv();
            1
        });
        std::thread::sleep(Duration::from_millis(40));
        // The budget ran out before the turn asked: it doesn't wait at all.
        let (got, pickup) = p.take(Duration::from_secs(5), Duration::from_millis(10));
        assert_eq!(got, None);
        assert_eq!(pickup.outcome, Outcome::Late);
        // timing: the cutoff must hold the turn to far less than the blocked work (several seconds)
        assert!(pickup.waited < Duration::from_secs(1), "{pickup:?}");
        release.send(()).unwrap();
    }

    #[test]
    fn a_zero_cutoff_still_takes_an_answer_already_there() {
        let p = Prefetch::start(|| "done");
        std::thread::sleep(Duration::from_millis(20));
        let (got, pickup) = p.take(Duration::ZERO, Duration::from_secs(5));
        assert_eq!(got, Some("done"));
        assert_eq!(pickup.outcome, Outcome::Ready);
    }

    #[test]
    fn a_prefetch_whose_work_panics_is_failed_not_late() {
        let p: Prefetch<u8> = Prefetch::start(|| panic!("prefetch test panic"));
        std::thread::sleep(Duration::from_millis(20));
        let (got, pickup) = p.take(Duration::from_millis(500), Duration::from_secs(5));
        assert_eq!(got, None);
        assert_eq!(pickup.outcome, Outcome::Failed);
    }

    #[test]
    fn own_and_agent_turns_are_named_apart() {
        assert_eq!(who(true), "own");
        assert_eq!(who(false), "agent");
    }
}
