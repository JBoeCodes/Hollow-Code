//! Refused canvas bridge messages (Canvas C6 P1).
//!
//! The host drops a frame's bridge message it won't take (`src/canvasBridge.ts`:
//! a malformed or oversized message, a `state` or `draft` for another canvas,
//! a `state` while the person isn't using the frame, a draft the gate
//! refuses). The frame is told nothing more than before; this keeps a small
//! diagnostic record of each refusal so a canvas that misbehaves can be
//! diagnosed after the fact.
//!
//! **What a record holds.** When (UTC), the message kind (one of the bridge's
//! child kinds, else `other`: an unknown kind's own text is never kept), its
//! size in bytes as JSON, the refusal reason (a fixed list), the canvas id
//! and the conversation key (each only when it passes its own id rule), and
//! how many refusals the rate limits dropped since the last record. Never
//! the payload, a summary, a state, a link or any other text a canvas wrote.
//!
//! **Where and how long.** `{app data}/canvas-bridge-refusals.jsonl`, never
//! the vault. Retention: when a record would take the file past
//! [`MAX_FILE_BYTES`] the file replaces `canvas-bridge-refusals.1.jsonl`
//! (the older one is dropped) and a new one starts, so the log never holds
//! more than two files, 128 KB in all; the oldest records go first.
//!
//! **Rate.** The frontend sends at most a few a second (`refusalLimiter`);
//! here at most [`PER_WINDOW`] records are written per [`WINDOW_SECS`]
//! seconds, process-wide; the rest are counted into the next record's
//! `suppressed`.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

/// The log, in the app data directory.
pub(crate) const FILE: &str = "canvas-bridge-refusals.jsonl";
/// The previous log, replaced at each rotation.
pub(crate) const OLD_FILE: &str = "canvas-bridge-refusals.1.jsonl";
/// The most one file holds before it rotates.
pub(crate) const MAX_FILE_BYTES: u64 = 64 * 1024;
/// Records written per window, process-wide.
pub(crate) const PER_WINDOW: u32 = 30;
pub(crate) const WINDOW_SECS: i64 = 60;

/// The bridge's child kinds (`CHILD_KINDS` in `src/canvasBridge.ts`) and
/// `other` for anything else.
const KINDS: [&str; 12] = [
    "size",
    "settled",
    "error",
    "navigate",
    "open",
    "blocked",
    "source",
    "state",
    "draft",
    "ask-invalid",
    "design",
    "other",
];

/// Why the host refused: a `Rejection` the parser gave (`source` and
/// `origin`, another window's message, are not this frame's and never
/// logged) or a `DraftRefusal` the gate gave (`confirm` and `cancelled`
/// are the person's choice, not a refusal of the frame).
const REASONS: [&str; 14] = [
    "not-object",
    "too-large",
    "version",
    "unknown-kind",
    "unsupported",
    "invalid",
    "canvas",
    "not-ask",
    "locked",
    "not-current",
    "unavailable",
    "focus",
    "unrequested",
    "busy",
];

/// One refusal as the frontend reports it (`BridgeRefusal` in
/// `src/canvasBridge.ts`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BridgeRefusal {
    pub kind: String,
    pub bytes: Option<u64>,
    pub reason: String,
    pub canvas_id: Option<String>,
    pub conversation: Option<String>,
    /// Refusals the frontend's own limit dropped since its last report.
    #[serde(default)]
    pub suppressed: u64,
}

/// One line of the log.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Record {
    pub at: String,
    pub kind: &'static str,
    pub bytes: Option<u64>,
    pub reason: &'static str,
    pub canvas_id: Option<String>,
    pub conversation: Option<String>,
    pub suppressed: u64,
}

/// `CANVAS_ID`: `^[a-z0-9-]{1,64}$`.
fn canvas_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The record for `r`, or an error for a reason outside the list (a
/// frontend bug, not a canvas: the frontend only sends known reasons).
/// Anything else that isn't what its field allows is dropped, not kept.
pub(crate) fn record(r: &BridgeRefusal, at: String, suppressed: u64) -> Result<Record, String> {
    let reason = REASONS
        .iter()
        .find(|x| **x == r.reason)
        .ok_or_else(|| "unknown refusal reason".to_string())?;
    let kind = KINDS.iter().find(|k| **k == r.kind).unwrap_or(&"other");
    Ok(Record {
        at,
        kind,
        bytes: r.bytes,
        reason,
        canvas_id: r.canvas_id.clone().filter(|id| canvas_id(id)),
        conversation: r
            .conversation
            .clone()
            .filter(|key| crate::agent_sessions::is_safe_session_key(key)),
        suppressed: suppressed.saturating_add(r.suppressed.min(1_000_000)),
    })
}

/// The process-wide write limit: [`PER_WINDOW`] records per window.
#[derive(Debug, Default)]
pub(crate) struct Limiter {
    window: i64,
    count: u32,
    suppressed: u64,
}

impl Limiter {
    /// `Some(dropped since the last admitted record)` when a record may be
    /// written at `now` (seconds), else `None` (counted).
    pub(crate) fn admit(&mut self, now: i64) -> Option<u64> {
        if now - self.window >= WINDOW_SECS || now < self.window {
            self.window = now;
            self.count = 0;
        }
        if self.count >= PER_WINDOW {
            self.suppressed = self.suppressed.saturating_add(1);
            return None;
        }
        self.count += 1;
        Some(std::mem::take(&mut self.suppressed))
    }
}

static LIMITER: Mutex<Limiter> = Mutex::new(Limiter {
    window: 0,
    count: 0,
    suppressed: 0,
});

/// Append `line` (one JSON record and its newline) to the log in `dir`,
/// rotating first when it would pass [`MAX_FILE_BYTES`]. A log that is a
/// symbolic link is refused, never followed.
pub(crate) fn append(dir: &Path, line: &str) -> Result<(), String> {
    let path = dir.join(FILE);
    let len = match std::fs::symlink_metadata(&path) {
        Ok(m) if m.file_type().is_symlink() || !m.is_file() => {
            return Err("the refusal log is not a plain file".into())
        }
        Ok(m) => m.len(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
        Err(e) => return Err(e.to_string()),
    };
    if len > 0 && len + line.len() as u64 > MAX_FILE_BYTES {
        std::fs::rename(&path, dir.join(OLD_FILE)).map_err(|e| e.to_string())?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(&path).map_err(|e| e.to_string())?;
    file.write_all(line.as_bytes()).map_err(|e| e.to_string())
}

/// Log one refusal in `dir` (the app data directory), unless the write
/// limit is spent: then it only counts toward the next record.
pub(crate) fn log(
    dir: &Path,
    r: &BridgeRefusal,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), String> {
    let admitted = LIMITER
        .lock()
        .map_err(|_| "refusal log limiter poisoned".to_string())?
        .admit(now.timestamp());
    let Some(dropped) = admitted else {
        return Ok(());
    };
    let rec = record(
        r,
        now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        dropped,
    )?;
    let mut line = serde_json::to_string(&rec).map_err(|e| e.to_string())?;
    line.push('\n');
    append(dir, &line)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BRIDGE_TS: &str = include_str!("../../src/canvasBridge.ts");

    fn refusal(kind: &str, reason: &str) -> BridgeRefusal {
        BridgeRefusal {
            kind: kind.into(),
            bytes: Some(120),
            reason: reason.into(),
            canvas_id: Some("release-choices".into()),
            conversation: Some("s-20261005-abc".into()),
            suppressed: 0,
        }
    }

    #[test]
    fn a_record_keeps_only_its_fields_and_drops_what_breaks_their_rules() {
        let at = "2026-10-05T12:00:00.000Z".to_string();
        let ok = record(&refusal("draft", "busy"), at.clone(), 2).unwrap();
        assert_eq!(
            serde_json::to_value(&ok).unwrap(),
            serde_json::json!({"at": at, "kind": "draft", "bytes": 120, "reason": "busy",
                "canvasId": "release-choices", "conversation": "s-20261005-abc", "suppressed": 2})
        );
        // An unknown kind's own text is never kept; a bad id or key is null.
        let mut odd = refusal("<script>steal()</script>", "unknown-kind");
        odd.canvas_id = Some("../Not An Id".into());
        odd.conversation = Some("../../etc".into());
        odd.suppressed = u64::MAX;
        let r = record(&odd, at.clone(), 1).unwrap();
        assert_eq!((r.kind, r.canvas_id, r.conversation), ("other", None, None));
        assert_eq!(r.suppressed, 1_000_001, "the frontend's count is capped");
        // A reason outside the list is a frontend bug: refused, not logged.
        assert!(record(&refusal("state", "source"), at.clone(), 0).is_err());
        assert!(record(&refusal("draft", "confirm"), at, 0).is_err());
        // No field for a payload, a summary or any other canvas text.
        let extra = serde_json::json!({"kind": "draft", "bytes": 1, "reason": "busy",
            "canvasId": null, "conversation": null, "summary": "agent text"});
        assert!(serde_json::from_value::<BridgeRefusal>(extra).is_err());
    }

    #[test]
    fn kinds_and_reasons_are_the_bridges() {
        assert!(crate::source_pins::contains(
            BRIDGE_TS,
            "export const CHILD_KINDS = ['size', 'settled', 'error', 'navigate', 'open', 'blocked', 'source', 'state', 'draft', 'ask-invalid', 'design'] as const;"
        ));
        for reason in REASONS {
            assert!(BRIDGE_TS.contains(&format!("'{reason}'")), "{reason}");
        }
        assert!(crate::source_pins::contains(
            BRIDGE_TS,
            "export const LOGGED_REASONS = ['not-object', 'too-large', 'version', 'unknown-kind', 'unsupported', 'invalid', 'canvas', 'not-ask', 'locked', 'not-current', 'unavailable', 'focus', 'unrequested', 'busy'] as const;"
        ));
    }

    #[test]
    fn the_limiter_admits_thirty_a_minute_and_counts_the_rest() {
        let mut l = Limiter::default();
        let t = 1_000_000;
        for _ in 0..PER_WINDOW {
            assert_eq!(l.admit(t), Some(0));
        }
        assert_eq!(l.admit(t + 1), None);
        assert_eq!(l.admit(t + 59), None);
        assert_eq!(
            l.admit(t + 60),
            Some(2),
            "the next window reports the dropped two"
        );
        assert_eq!(l.admit(t + 61), Some(0));
        // A clock that steps back starts a new window rather than stalling.
        assert_eq!(l.admit(t - 500), Some(0));
    }

    #[test]
    fn the_log_rotates_at_its_cap_and_never_holds_more_than_two_files() {
        let dir = tempfile::tempdir().unwrap();
        let line = format!("{}\n", "x".repeat(999));
        for _ in 0..200 {
            append(dir.path(), &line).unwrap();
        }
        let cur = std::fs::metadata(dir.path().join(FILE)).unwrap().len();
        let old = std::fs::metadata(dir.path().join(OLD_FILE)).unwrap().len();
        assert!(
            cur <= MAX_FILE_BYTES && old <= MAX_FILE_BYTES,
            "{cur} {old}"
        );
        assert_eq!(old % 1000, 0, "whole records only");
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_log_that_is_a_symbolic_link_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("elsewhere");
        std::fs::write(&target, "keep").unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join(FILE)).unwrap();
        assert!(append(dir.path(), "{}\n").is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
    }

    #[test]
    fn logging_writes_one_json_line_in_the_given_directory() {
        let dir = tempfile::tempdir().unwrap();
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-05T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        log(dir.path(), &refusal("state", "focus"), now).unwrap();
        let text = std::fs::read_to_string(dir.path().join(FILE)).unwrap();
        let v: serde_json::Value = serde_json::from_str(text.trim_end()).unwrap();
        assert_eq!(v["kind"], "state");
        assert_eq!(v["reason"], "focus");
        assert_eq!(v["at"], "2026-10-05T12:00:00.000Z");
    }
}
