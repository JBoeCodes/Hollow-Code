//! Disposable enqueue-time preparation. No prompt, source text, grant or
//! attachment is reused. Only the existing exact score/vector caches warm.

use super::chat_queue::{self, ChatTurnPayload};
use once_cell::sync::Lazy;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MAX_JOBS: usize = 2;
const MAX_PAYLOAD_BYTES: usize = 128 * 1024;
const BUDGET: Duration = Duration::from_secs(5);

struct Job {
    vault: PathBuf,
    row: i64,
    session: String,
    stopped: AtomicBool,
    started: Instant,
}

#[derive(Default)]
struct Registry(Mutex<Vec<Arc<Job>>>);

struct Permit {
    registry: Arc<Registry>,
    job: Arc<Job>,
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.registry
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|job| !Arc::ptr_eq(job, &self.job));
    }
}

impl Registry {
    fn reserve(self: &Arc<Self>, vault: PathBuf, row: i64, session: String) -> Option<Permit> {
        let mut jobs = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if jobs.len() >= MAX_JOBS
            || jobs
                .iter()
                .any(|job| job.vault == vault && job.session == session)
        {
            return None;
        }
        let job = Arc::new(Job {
            vault,
            row,
            session,
            stopped: AtomicBool::new(false),
            started: Instant::now(),
        });
        jobs.push(job.clone());
        Some(Permit {
            registry: self.clone(),
            job,
        })
    }
}

impl Job {
    fn still_queued(&self, payload: &serde_json::Value) -> bool {
        // Reserve first, then reread. If admission/Stop already passed before
        // reservation, skip optional work. A later Stop sees this registered job.
        crate::run_queue::get(&self.vault, self.row)
            .ok()
            .flatten()
            .is_some_and(|row| row.status == crate::run_queue::RowStatus::Queued)
            && self.current(payload)
    }

    fn current(&self, payload: &serde_json::Value) -> bool {
        if super::preparation_stop_requested(&self.session, self.row) {
            self.stopped.store(true, Ordering::Release);
        }
        !self.stopped.load(Ordering::Acquire)
            && self.started.elapsed() < BUDGET
            && crate::run_queue::get(&self.vault, self.row)
                .ok()
                .flatten()
                .is_some_and(|row| {
                    !row.status.is_terminal()
                        && !row.cancel_requested
                        && row.source_id.as_deref() == Some(self.session.as_str())
                        && &row.payload == payload
                })
    }
}

static JOBS: Lazy<Arc<Registry>> = Lazy::new(|| Arc::new(Registry::default()));

pub(super) fn cancel(vault: &Path, row: i64) {
    let vault = vault.canonicalize().unwrap_or_else(|_| vault.to_path_buf());
    for job in JOBS.0.lock().unwrap_or_else(|p| p.into_inner()).iter() {
        if job.vault == vault && job.row == row {
            job.stopped.store(true, Ordering::Release);
        }
    }
}

pub(super) fn cancel_session(session: &str) {
    for job in JOBS.0.lock().unwrap_or_else(|p| p.into_inner()).iter() {
        if job.session == session {
            job.stopped.store(true, Ordering::Release);
        }
    }
}

pub(super) fn cancel_session_row(session: &str, row: i64) {
    for job in JOBS.0.lock().unwrap_or_else(|p| p.into_inner()).iter() {
        if job.session == session && job.row == row {
            job.stopped.store(true, Ordering::Release);
        }
    }
}

/// Overflow, expired work and restart all use ordinary preparation. There
/// is no pending-work queue and the sender never waits for a worker.
pub(super) fn start(vault: &Path, row: i64, session: &str) {
    // Unit fixtures must never start optional model or account work. The
    // registry and ranking transport have their own synthetic regressions.
    if cfg!(test) {
        return;
    }
    let Ok(Some(stored)) = crate::run_queue::get(vault, row) else {
        return;
    };
    if stored.status != crate::run_queue::RowStatus::Queued
        || stored.cancel_requested
        || serde_json::to_vec(&stored.payload).map_or(true, |v| v.len() > MAX_PAYLOAD_BYTES)
    {
        return;
    }
    let Ok(payload) = serde_json::from_value::<ChatTurnPayload>(stored.payload.clone()) else {
        return;
    };
    let Some(agent) = crate::agent::AgentKind::from_opt(&payload.agent) else {
        return;
    };
    let vault = vault.canonicalize().unwrap_or_else(|_| vault.to_path_buf());
    if crate::mcp_attach::bare_scratch(&vault, Path::new(&payload.cwd)) {
        return;
    }
    let Some(permit) = JOBS.reserve(vault, row, session.into()) else {
        return;
    };
    if !permit.job.still_queued(&stored.payload) {
        return;
    }
    // The blocking worker owns capacity until it actually exits. In
    // particular, dropping an optional future never releases its permit
    // while the scanner it awaited is still running.
    tauri::async_runtime::spawn_blocking(move || {
        let permit = permit;
        let job = &permit.job;
        if !job.current(&stored.payload) {
            return;
        }
        let inputs = chat_queue::turn_inputs_by(
            &job.vault,
            &payload,
            &job.session,
            chat_queue::asks_resume(payload.session_id.as_deref()),
            false,
        );
        if !job.current(&stored.payload) {
            return;
        }
        crate::context_compiler::prepare_reads(&job.vault, &job.session);
        if !job.current(&stored.payload) {
            return;
        }
        tauri::async_runtime::block_on(async {
            let deadline = crate::prep_deadline::PrepDeadline::with_budget(
                BUDGET.saturating_sub(job.started.elapsed()),
            );
            let mut jev = crate::jev::Target::app().await;
            if !job.current(&stored.payload) {
                return;
            }
            let query = inputs
                .retrieval_basis
                .query
                .clone()
                .unwrap_or(payload.asked.clone());
            let (owned_job, identity) = (job.clone(), stored.payload.clone());
            let allowed: crate::jev::Allowed = Arc::new(move || owned_job.current(&identity));
            jev.allowed = Some(allowed.clone());
            let retrieval = async {
                if !inputs.retrieval_basis.basis.skips() {
                    crate::directory_connections::prepare_ranking(
                        job.vault.clone(),
                        PathBuf::from(&payload.cwd),
                        query,
                        &deadline,
                        &jev,
                        allowed.clone(),
                    )
                    .await;
                }
            };
            let keeps_connectors = inputs.retrieval_basis.basis
                == crate::turn_request::QueryBasis::SkippedHost
                || (inputs.retrieval_basis.basis.skips()
                    && crate::evidence::last_connector_selection(&job.vault, &job.session)
                        .is_some());
            let connectors = async {
                if !keeps_connectors {
                    let session = crate::context_rank::session_key(&job.vault, &job.session);
                    crate::connectors::prepare_relevance(
                        agent,
                        payload.permission_mode.as_deref().unwrap_or("default"),
                        &session,
                        &inputs.connector_query,
                        &deadline,
                        &jev,
                        &*allowed,
                    )
                    .await;
                }
            };
            // Both finish or reach their own deadline. Do not wrap this
            // join in a timeout that detaches an outstanding blocking scan.
            tokio::join!(retrieval, connectors);
        });
        eprintln!(
            "[prep] enqueue read preparation: {} ms",
            job.started.elapsed().as_millis()
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preparation_is_bounded_per_session_and_capacity_lasts_until_exit() {
        let registry = Arc::new(Registry::default());
        let first = registry.reserve("/v".into(), 1, "one".into()).unwrap();
        assert!(registry.reserve("/v".into(), 2, "one".into()).is_none());
        let second = registry.reserve("/v".into(), 3, "two".into()).unwrap();
        assert!(registry.reserve("/v".into(), 4, "three".into()).is_none());
        first.job.stopped.store(true, Ordering::Release);
        assert!(registry.reserve("/v".into(), 5, "one".into()).is_none());
        drop(first);
        let replacement = registry.reserve("/v".into(), 5, "one".into()).unwrap();
        drop(second);
        drop(replacement);
        assert!(registry.0.lock().unwrap().is_empty());
    }

    #[test]
    fn preparation_checks_full_payload_identity_and_terminal_state() {
        let dir = tempfile::tempdir().unwrap();
        let payload = serde_json::json!({"asked": "same", "futureField": 1});
        let row = crate::run_queue::NewRow::new(
            crate::run_queue::RunSource::Chat,
            Some("s-20261116-12345678"),
            "Chat",
            "chat",
        )
        .with_payload(payload.clone());
        let now = chrono::Utc::now();
        let id = crate::run_queue::enqueue(dir.path(), row, now)
            .unwrap()
            .id();
        let registry = Arc::new(Registry::default());
        let permit = registry
            .reserve(dir.path().to_path_buf(), id, "s-20261116-12345678".into())
            .unwrap();
        assert!(permit.job.current(&payload));
        let changed = serde_json::json!({"asked": "same", "futureField": 2});
        assert!(!permit.job.current(&changed));
        crate::run_queue::cancel(dir.path(), id, now).unwrap();
        assert!(!permit.job.current(&payload));
    }

    #[test]
    fn preparation_observes_and_latches_a_running_turn_stop() {
        let _turns = super::super::TURN_TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let payload = serde_json::json!({"asked": "stop me"});
        let key = "s-20261116-12345679";
        let row = crate::run_queue::NewRow::new(
            crate::run_queue::RunSource::Chat,
            Some(key),
            "Chat",
            "chat",
        )
        .with_payload(payload.clone());
        let id = crate::run_queue::enqueue(dir.path(), row, chrono::Utc::now())
            .unwrap()
            .id();
        let permit = JOBS
            .reserve(dir.path().to_path_buf(), id, key.into())
            .unwrap();
        crate::run_queue::claim(dir.path(), id, chrono::Utc::now()).unwrap();
        assert!(crate::run_queue::mark_running(
            dir.path(),
            id,
            "synthetic-preparation-run",
            chrono::Utc::now()
        )
        .unwrap());
        let registration = super::super::register_stop_row(key, Some(id));
        assert!(permit.job.current(&payload));
        assert!(!super::super::cancel_running_row(key, id + 1));
        assert!(permit.job.current(&payload));
        assert!(super::super::cancel_running_row(key, id));
        drop(registration);
        assert!(
            !permit.job.current(&payload),
            "dropping the stop slot cannot revive preparation"
        );
        drop(permit);
        let late = JOBS
            .reserve(dir.path().to_path_buf(), id, key.into())
            .unwrap();
        assert!(
            !late.job.still_queued(&payload),
            "a reservation after admission and Stop must not start optional work"
        );
    }
}
