//! "Uncertain" as a state, and one use per approval (ND9, 2026-11-20;
//! Hollow's own design).
//!
//! An outward action (tier External: it leaves this Mac) is written to a
//! journal in app data before it runs and closed when it returns:
//! `started`, then `done` or `failed`. A process that dies between the two
//! (a crash, a quit, a panic, a lost reply) leaves `started` behind, and a
//! `started` record from another launch, or one a panic closed as
//! `uncertain`, means the effect may have happened. Rules the broker applies
//! ([`check`]):
//!
//! - `done` in the same run: refused. One approval is one use; the agent
//!   asking again in the run it already ran in does not reach a card.
//! - `started` in this launch: refused (the same call is running now).
//! - `uncertain` (or `started` from an earlier launch): never retried on
//!   its own. The card is forced even under a standing approval, says an
//!   earlier attempt may already have happened and must be checked first,
//!   and only a chat with a person watching can ask it (an automation or an
//!   unattended check-in is refused). A new Allow replaces the record.
//! - `failed`, or `done` in another run: a new approval is a new use.
//!
//! Records are keyed by the call's fingerprint (`approval_terms`), live in
//! app data outside the vault (the agent cannot edit them) and are pruned
//! after a week. A write that fails stops the action: no record, no run.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const DIR: &str = "ApprovalUses";
const KEEP_SECS: u64 = 7 * 24 * 3600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum State {
    Started,
    Done,
    Failed,
    Uncertain,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Use {
    pub fingerprint: String,
    pub action: String,
    pub run_id: String,
    /// The process that began it (start time and pid in hex).
    pub launch: String,
    /// That process's pid, to tell a call running in another Hollow process
    /// (`hollow serve`) from one whose process died.
    #[serde(default)]
    pub pid: u32,
    /// An earlier attempt of this call was uncertain when this one began, so
    /// a clean failure of this one must not clear the doubt.
    #[serde(default)]
    pub carried_uncertain: bool,
    pub state: State,
    /// RFC 3339, of the last change.
    pub at: String,
}

/// What [`check`] found for a call about to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Prior {
    Clear,
    /// Refuse, with the words for the agent.
    Refuse(String),
    /// An earlier attempt may have happened: when it began.
    Uncertain(String),
}

fn launch_id() -> &'static str {
    static LAUNCH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    LAUNCH.get_or_init(|| {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        format!("{ms:x}{:x}", std::process::id())
    })
}

/// The journal folder, for a test that seeds a record.
#[cfg(test)]
pub(crate) fn root_for_test() -> PathBuf {
    root().unwrap()
}

fn root() -> Result<PathBuf, String> {
    crate::agent_settings::app_data_dir().map(|d| d.join(DIR))
}

fn path_in(root: &Path, fingerprint: &str) -> Option<PathBuf> {
    (!fingerprint.is_empty() && fingerprint.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| root.join(format!("{fingerprint}.json")))
}

enum Read {
    Missing,
    Found(Use),
    /// A record that cannot be read or understood: never read as "clear".
    Unreadable,
}

fn read_in(root: &Path, fingerprint: &str) -> Read {
    let Some(path) = path_in(root, fingerprint) else {
        return Read::Unreadable;
    };
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Read::Found)
            .unwrap_or(Read::Unreadable),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Read::Missing,
        Err(_) => Read::Unreadable,
    }
}

fn write_in(root: &Path, record: &Use) -> Result<(), String> {
    let path = path_in(root, &record.fingerprint).ok_or("invalid approval fingerprint")?;
    std::fs::create_dir_all(root).map_err(|e| format!("couldn't prepare the use record: {e}"))?;
    let bytes = serde_json::to_vec_pretty(record).map_err(|e| e.to_string())?;
    crate::vault::atomic_write(&path, &bytes)
        .map_err(|e| format!("couldn't write the use record: {e}"))
}

/// Drop old `done` and `failed` records. A `started` or `uncertain` one stays
/// until the person resolves it with a new Allow.
fn prune(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age.as_secs() > KEEP_SECS);
        if !old {
            continue;
        }
        let closed = std::fs::read(entry.path())
            .ok()
            .and_then(|b| serde_json::from_slice::<Use>(&b).ok())
            .is_some_and(|u| matches!(u.state, State::Done | State::Failed));
        if closed {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// What the journal says about this call before it asks or runs.
pub(crate) fn check(fingerprint: &str, run_id: &str) -> Result<Prior, String> {
    Ok(check_in(&root()?, fingerprint, run_id, launch_id()))
}

pub(crate) fn check_in(root: &Path, fingerprint: &str, run_id: &str, launch: &str) -> Prior {
    let prior = match read_in(root, fingerprint) {
        Read::Missing => return Prior::Clear,
        Read::Unreadable => return Prior::Uncertain(UNREADABLE.to_string()),
        Read::Found(prior) => prior,
    };
    match prior.state {
        State::Done if prior.run_id == run_id => Prior::Refuse(format!(
            "{} already ran in this run (at {}) on an approval that covered one use, so it was not run again. Ask the person if it should happen again.",
            prior.action, prior.at
        )),
        State::Started if prior.launch == launch => Prior::Refuse(format!(
            "{} is already running on an earlier approval (since {}); it was not started twice.",
            prior.action, prior.at
        )),
        State::Started if prior.pid != 0 && prior.pid != std::process::id() && pid_alive(prior.pid) => {
            Prior::Refuse(format!(
                "{} is already running in another Hollow process (since {}); it was not started twice.",
                prior.action, prior.at
            ))
        }
        State::Started | State::Uncertain => Prior::Uncertain(prior.at),
        State::Done | State::Failed => Prior::Clear,
    }
}

/// What an unreadable record's card says instead of a time.
const UNREADABLE: &str = "an unknown time (its record could not be read)";

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    // SAFETY: signal 0 only checks that the process exists and can be signalled.
    let rc = unsafe { libc::kill(pid as i32, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
fn pid_alive(_pid: u32) -> bool {
    false
}

/// Serializes the re-check and the write of [`begin_in`], so two identical
/// calls in this process cannot both start.
static GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The open use of a call that is running. Dropped without [`Self::finish`]
/// (a panic unwinding) it closes as `uncertain`.
pub(crate) struct Open {
    root: PathBuf,
    record: Use,
    closed: bool,
}

/// Record that the call is about to run, re-checking the journal under a
/// lock first: this is the gate, the earlier [`check`] only decided what the
/// card says. `acknowledged` is the time of the uncertain earlier attempt the
/// card showed (from [`Prior::Uncertain`]); an uncertain record the card did
/// not show, or one that appeared while the card waited, refuses. An error
/// means it is not recorded, so the caller must not run the call.
pub(crate) fn begin(
    fingerprint: &str,
    action: &str,
    run_id: &str,
    acknowledged: Option<&str>,
) -> Result<Open, String> {
    begin_in(
        &root()?,
        fingerprint,
        action,
        run_id,
        launch_id(),
        acknowledged,
    )
}

pub(crate) fn begin_in(
    root: &Path,
    fingerprint: &str,
    action: &str,
    run_id: &str,
    launch: &str,
    acknowledged: Option<&str>,
) -> Result<Open, String> {
    let _gate = GATE.lock().unwrap_or_else(|p| p.into_inner());
    let carried = match check_in(root, fingerprint, run_id, launch) {
        Prior::Clear => false,
        Prior::Refuse(why) => return Err(why),
        Prior::Uncertain(at) if acknowledged == Some(at.as_str()) => true,
        Prior::Uncertain(_) => {
            return Err(format!(
                "An earlier attempt of {action} may already have happened and the card did not say so, so it was not run. Ask again."
            ))
        }
    };
    prune(root);
    let record = Use {
        fingerprint: fingerprint.to_string(),
        action: action.to_string(),
        run_id: run_id.to_string(),
        launch: launch.to_string(),
        pid: std::process::id(),
        carried_uncertain: carried,
        state: State::Started,
        at: chrono::Utc::now().to_rfc3339(),
    };
    write_in(root, &record)?;
    Ok(Open {
        root: root.to_path_buf(),
        record,
        closed: false,
    })
}

impl Open {
    /// Close the use with how the action ended. Fail closed: an error is
    /// `uncertain` unless its words say nothing was sent, and a clean failure
    /// of a retry keeps the doubt about the attempt before it.
    pub(crate) fn finish<T>(mut self, outcome: &Result<T, String>) {
        let state = match outcome {
            Ok(_) => State::Done,
            Err(why) if definitely_not_sent(why) && !self.record.carried_uncertain => State::Failed,
            Err(_) => State::Uncertain,
        };
        self.close(state);
    }

    fn close(&mut self, state: State) {
        self.closed = true;
        self.record.state = state;
        self.record.at = chrono::Utc::now().to_rfc3339();
        // Best effort: a record left `started` reads as uncertain next time.
        let _ = write_in(&self.root, &self.record);
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        if !self.closed {
            self.close(State::Uncertain);
        }
    }
}

/// Whether an error says the call was refused before anything was sent.
/// Anything else (a dropped connection, a gateway error, a reply that never
/// came) may have reached the other side.
fn definitely_not_sent(why: &str) -> bool {
    let lower = why.to_lowercase();
    [
        "nothing changed",
        "nothing ran",
        "nothing was sent",
        "nothing sent",
        "not approved",
    ]
    .iter()
    .any(|w| lower.contains(w))
}

/// The line a forced card shows when an earlier attempt is uncertain.
pub(crate) fn uncertain_line(at: &str) -> String {
    format!(
        "An earlier attempt of this exact call (from {at}) was interrupted and may already have happened. Check the destination first; Allow only if it did not."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const FP: &str = "abc123";

    fn root() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("uses-").tempdir().unwrap()
    }

    fn begin(r: &Path, run: &str, launch: &str) -> Open {
        begin_in(r, FP, "x.send", run, launch, None).unwrap()
    }

    #[test]
    fn a_new_call_is_clear_and_done_in_the_same_run_is_refused() {
        let dir = root();
        let r = dir.path();
        assert_eq!(check_in(r, FP, "run-1", "L1"), Prior::Clear);
        let open = begin(r, "run-1", "L1");
        // In flight now, same launch: a second start is refused, by check and by begin.
        assert!(
            matches!(check_in(r, FP, "run-1", "L1"), Prior::Refuse(w) if w.contains("already running"))
        );
        assert!(begin_in(r, FP, "x.send", "run-1", "L1", None).is_err());
        open.finish(&Ok::<(), String>(()));
        assert!(
            matches!(check_in(r, FP, "run-1", "L1"), Prior::Refuse(w) if w.contains("one use"))
        );
        assert!(
            begin_in(r, FP, "x.send", "run-1", "L1", None).is_err(),
            "begin is the gate"
        );
        // Another run's new approval is a new use.
        assert_eq!(check_in(r, FP, "run-2", "L1"), Prior::Clear);
    }

    #[test]
    fn only_a_refused_before_send_failure_clears_and_any_other_error_is_uncertain() {
        let dir = root();
        let r = dir.path();
        begin(r, "run-1", "L1").finish(&Err::<(), String>(
            "Nothing was sent: not signed in.".into(),
        ));
        assert_eq!(check_in(r, FP, "run-1", "L1"), Prior::Clear);
        for why in [
            "connection closed",
            "502 bad gateway",
            "operation canceled",
            "timed out",
        ] {
            begin(r, "run-1", "L1").finish(&Err::<(), String>(why.into()));
            assert!(
                matches!(check_in(r, FP, "run-1", "L1"), Prior::Uncertain(_)),
                "{why}"
            );
            // A new Allow after the card resolves it.
            let Prior::Uncertain(at) = check_in(r, FP, "run-1", "L1") else {
                unreachable!()
            };
            begin_in(r, FP, "x.send", "run-1", "L1", Some(&at))
                .unwrap()
                .finish(&Ok::<(), String>(()));
            std::fs::remove_file(path_in(r, FP).unwrap()).unwrap();
        }
    }

    #[test]
    fn an_interrupted_call_is_uncertain_and_only_a_shown_card_starts_it_again() {
        let dir = root();
        let r = dir.path();
        // A launch that died between begin and finish leaves `started`.
        std::mem::forget(begin_in(r, FP, "x.send", "run-1", "OLD", None).unwrap());
        let mut record = match read_in(r, FP) {
            Read::Found(u) => u,
            _ => panic!("record"),
        };
        record.pid = 0;
        write_in(r, &record).unwrap();
        let Prior::Uncertain(at) = check_in(r, FP, "run-2", "NEW") else {
            panic!("a started record from another launch is uncertain");
        };
        assert!(uncertain_line(&at).contains("may already have happened"));
        // Without the card's time, or with a stale one, begin refuses.
        assert!(begin_in(r, FP, "x.send", "run-2", "NEW", None).is_err());
        assert!(begin_in(r, FP, "x.send", "run-2", "NEW", Some("earlier")).is_err());
        // The person's Allow starts a new use over it; a clean failure of that
        // retry keeps the doubt about the first attempt.
        begin_in(r, FP, "x.send", "run-2", "NEW", Some(&at))
            .unwrap()
            .finish(&Err::<(), String>("Nothing was sent.".into()));
        assert!(matches!(
            check_in(r, FP, "run-3", "NEW"),
            Prior::Uncertain(_)
        ));
        // A success of the retry closes it.
        let Prior::Uncertain(at) = check_in(r, FP, "run-3", "NEW") else {
            unreachable!()
        };
        begin_in(r, FP, "x.send", "run-3", "NEW", Some(&at))
            .unwrap()
            .finish(&Ok::<(), String>(()));
        assert_eq!(check_in(r, FP, "run-4", "NEW"), Prior::Clear);
    }

    #[test]
    fn a_started_record_of_a_live_other_process_is_running_not_uncertain() {
        let dir = root();
        let r = dir.path();
        // This test process stands in for `hollow serve`: another launch id, a live pid.
        std::mem::forget(begin_in(r, FP, "x.send", "run-1", "SERVE", None).unwrap());
        let mut record = match read_in(r, FP) {
            Read::Found(u) => u,
            _ => panic!("record"),
        };
        record.pid = std::process::id() + 1;
        // A pid that is not ours and not alive reads as uncertain.
        record.pid = u32::MAX - 1;
        write_in(r, &record).unwrap();
        assert!(matches!(
            check_in(r, FP, "run-2", "APP"),
            Prior::Uncertain(_)
        ));
        // The parent of this test process is alive and is not us.
        #[cfg(unix)]
        {
            record.pid = std::os::unix::process::parent_id();
            write_in(r, &record).unwrap();
            assert!(
                matches!(check_in(r, FP, "run-2", "APP"), Prior::Refuse(w) if w.contains("another Hollow process"))
            );
        }
    }

    #[test]
    fn a_panic_closes_as_uncertain_and_a_corrupt_record_is_never_clear() {
        let dir = root();
        let r = dir.path();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _open = begin(r, "run-1", "L1");
            panic!("handler crashed");
        }));
        assert!(result.is_err());
        assert!(matches!(read_in(r, FP), Read::Found(u) if u.state == State::Uncertain));
        assert!(matches!(
            check_in(r, FP, "run-1", "L1"),
            Prior::Uncertain(_)
        ));

        std::fs::write(path_in(r, FP).unwrap(), b"{ truncated").unwrap();
        let Prior::Uncertain(at) = check_in(r, FP, "run-1", "L1") else {
            panic!("an unreadable record must not read as clear");
        };
        assert_eq!(at, UNREADABLE);
        assert!(begin_in(r, FP, "x.send", "run-1", "L1", None).is_err());
    }

    #[test]
    fn a_bad_fingerprint_is_refused_and_prune_keeps_open_records() {
        let dir = root();
        let r = dir.path();
        assert!(begin_in(r, "../x", "a", "r", "L", None).is_err());
        assert!(path_in(r, "").is_none());
        // Old closed records go; an old uncertain one stays.
        begin(r, "run-1", "L1").finish(&Ok::<(), String>(()));
        std::mem::forget(begin_in(r, "dead", "x.send", "run-1", "OLD", None).unwrap());
        for name in [FP, "dead"] {
            let file = std::fs::File::options()
                .write(true)
                .open(path_in(r, name).unwrap())
                .unwrap();
            file.set_modified(
                std::time::SystemTime::now() - std::time::Duration::from_secs(KEEP_SECS + 60),
            )
            .unwrap();
        }
        prune(r);
        assert!(!path_in(r, FP).unwrap().exists());
        assert!(path_in(r, "dead").unwrap().exists());
    }
}
