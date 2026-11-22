//! One bounded preparation deadline (agent effectiveness F8, wave 3 · lane
//! O, PR O2).
//!
//! O1's baseline (docs/history/AGENT_EFFECTIVENESS.md, "Wave 3 baseline") showed a
//! chat turn's preparation waiting for two Jev timeouts one after the other:
//! the passage rerank inside retrieval, then connector selection, 4 s each,
//! so 8.0 s before the agent started. Retrieval and connector selection don't
//! depend on each other, so a run now prepares them at once
//! ([`concurrently`]), under ONE deadline ([`PrepDeadline`], [`BUDGET`] from
//! the start of that stage) for the optional, slow parts: both Jev calls,
//! the retrieval broker's semantic half and the Microsoft 365 token refresh.
//! When the deadline passes, each falls back locally and the run's manifest
//! says so (`preparation.deadline`, a `deadline` Jev outcome, a fallback):
//! Jev's decisions become the local ones (a connector the request names is
//! decided before Jev, so it is never affected), the semantic half leaves
//! retrieval lexical only, and a refresh still running finishes on its own
//! (the connector subprocess refreshes in memory when it needs to).
//!
//! Required local context is never cut: the lexical scan, recall, the
//! attached sources and every "always" or named connector run to completion
//! whatever the clock says. The deadline only stops the run *waiting* for
//! something optional.
use crate::prep_timing::Mark;
use std::future::Future;
use std::time::{Duration, Instant};

/// How long retrieval and connector selection together may wait for their
/// optional parts. Below Jev's own 4 s per-request bound, so a stalled Jev
/// is cut by the deadline (an attributable `deadline`), never by the
/// transport, and the two calls together cost at most this, not 4 s each.
pub(crate) const BUDGET: Duration = Duration::from_millis(3000);

/// What a Jev call cut by the deadline reads as (its `reason`, and
/// `jev::outcome_of` → `deadline`).
pub(crate) const JEV_MISSED: &str = "Jev missed the preparation deadline.";
/// Why the semantic half isn't blended when the deadline cut it.
pub(crate) const SEMANTIC_MISSED: &str =
    "the semantic half missed the preparation deadline (lexical only)";

/// The deadline for one run's preparation stage.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PrepDeadline {
    pub started: Instant,
    pub at: Instant,
}

impl PrepDeadline {
    /// A deadline [`BUDGET`] from now.
    pub(crate) fn start() -> Self {
        Self::with_budget(BUDGET)
    }

    /// A deadline `budget` from now (tests and the bench use short ones).
    pub(crate) fn with_budget(budget: Duration) -> Self {
        let started = Instant::now();
        PrepDeadline {
            started,
            at: started + budget,
        }
    }

    pub(crate) fn budget_ms(&self) -> u64 {
        u64::try_from(self.at.saturating_duration_since(self.started).as_millis())
            .unwrap_or(u64::MAX)
    }

    pub(crate) fn passed(&self) -> bool {
        Instant::now() >= self.at
    }

    /// `fut`'s output, or `None` once the deadline passed first (`fut` is
    /// dropped then: give it a task of its own when it must finish anyway).
    pub(crate) async fn bound<F: Future>(&self, fut: F) -> Option<F::Output> {
        tokio::time::timeout_at(self.at.into(), fut).await.ok()
    }
}

/// Run `a` and `b` at once (retrieval and connector selection: neither reads
/// the other's result), each with its own mark from the moment both start.
pub(crate) async fn concurrently<A: Future, B: Future>(
    a: A,
    b: B,
) -> ((A::Output, Mark), (B::Output, Mark)) {
    let started = Instant::now();
    tokio::join!(
        async {
            let out = a.await;
            (out, Mark::since(started))
        },
        async {
            let out = b.await;
            (out, Mark::since(started))
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bound_gives_up_at_the_deadline_and_passes_a_quick_answer() {
        let deadline = PrepDeadline::with_budget(Duration::from_millis(150));
        assert_eq!(deadline.budget_ms(), 150);
        assert!(!deadline.passed());
        assert_eq!(deadline.bound(async { 7 }).await, Some(7));
        let started = Instant::now();
        let slow = deadline
            .bound(tokio::time::sleep(Duration::from_secs(60)))
            .await;
        assert!(slow.is_none(), "cut");
        // Cut at 150 ms, not after the 60 s sleep. Not a paused-clock test:
        // `PrepDeadline` reads `std::time::Instant`, which paused time won't move.
        let waited = started.elapsed();
        assert!(waited < Duration::from_secs(30), "{waited:?}");
        assert!(deadline.passed());
        // Already passed: nothing is awaited.
        assert_eq!(deadline.bound(std::future::pending::<()>()).await, None);
    }

    /// Two 300 ms steps prepared together overlap, and both marks start
    /// together. Each step waits at a barrier only the other can open before
    /// its 300 ms sleep and again after it, so neither starts its sleep before
    /// the other is under way, nor finishes before the other's sleep ends:
    /// steps run one after the other would never finish (the timeout),
    /// whatever the machine's load: no "under 550 ms" race (S23).
    #[tokio::test]
    async fn the_two_steps_overlap() {
        let before = tokio::sync::Barrier::new(2);
        let after = tokio::sync::Barrier::new(2);
        let ((a, ma), (b, mb)) = tokio::time::timeout(
            Duration::from_secs(30),
            concurrently(
                async {
                    before.wait().await;
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    after.wait().await;
                    "evidence"
                },
                async {
                    before.wait().await;
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    after.wait().await;
                    "connectors"
                },
            ),
        )
        .await
        .expect("the two steps ran one after the other");
        assert_eq!((a, b), ("evidence", "connectors"));
        assert_eq!(ma.started, mb.started);
        assert!(ma.ms >= 300 && mb.ms >= 300);
    }
}
