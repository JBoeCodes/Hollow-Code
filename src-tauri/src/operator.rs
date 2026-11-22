//! Operator profile — the durable "how the user works" digest compiled from
//! the vault's `me/` folder. Injected into chat sessions (the first turn of
//! each new native session, by `context_compiler.rs`) and workflow runs so the agent already knows the house style,
//! settled decisions, and anti-patterns without being re-briefed each turn.
//!
//! This is the backend half of the "real coworker, no spoon-feeding" thesis:
//! page-context tells the agent *what's on screen*; operator-context tells it
//! *how the user works*.

use crate::{frontmatter, vault};
use chrono::{DateTime, Days, Local, NaiveDate, Utc};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// The Settings "Share About you with agents" switch. Chat turns
/// (`context_compiler`) and workflow runs both read it, so the one switch
/// governs every surface.
///
/// **The backend's persisted value is the source of truth.** It lives in
/// `operator-context.json` in Hollow's app-data folder (beside
/// `agent-settings.json`) and is read the first time it is asked, so a turn
/// that runs before the window has loaded (an automation, a child an agent
/// started through `hollow_app`, a queued turn resumed at launch) gets the
/// person's choice (K1 follow-up, 2026-10-07). The window adopts it at
/// launch (`operator_context_switch`; `src/operatorSwitch.ts`) and pushes
/// only an explicit change (`set_operator_context_enabled`); its
/// localStorage copy is a display cache, never pushed back (wave 2
/// follow-up: a lost key used to turn an "off" back on at launch).
///
/// Never set (no file, the folder known): on, the product default. A file
/// that can't be read or parsed reads as off, and so does a missing
/// app-data folder: an unknown answer is not consent to send the profile.
pub(crate) struct Switch {
    path: Option<PathBuf>,
    /// `None` until first read. Writes hold the lock across the save, so
    /// the saved choice is always the one in force.
    state: Mutex<Option<SwitchState>>,
}

/// The switch as the window adopts it at launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchState {
    pub enabled: bool,
    /// A choice is recorded (the file exists, readable or not, or the
    /// switch was set this run). False only for a never-set switch, whose
    /// `enabled` is the default (on, or off with no app-data folder).
    pub chosen: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Stored {
    enabled: bool,
}

/// Distinct temp names for saves (with the pid, so two processes sharing
/// an app-data folder can't collide either).
static SAVE_SEQ: AtomicU64 = AtomicU64::new(0);

impl Switch {
    /// The switch whose state lives at `path` (`None`: no app-data folder;
    /// off until set, in memory only).
    pub(crate) fn at(path: Option<PathBuf>) -> Self {
        Switch {
            path,
            state: Mutex::new(None),
        }
    }

    fn read(&self) -> SwitchState {
        let Some(path) = &self.path else {
            eprintln!("[operator] no app-data folder for the About you switch; treating it as off");
            return SwitchState {
                enabled: false,
                chosen: false,
            };
        };
        let enabled = match std::fs::read(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return SwitchState {
                    enabled: true,
                    chosen: false,
                }
            }
            Err(e) => {
                eprintln!(
                    "[operator] the About you switch could not be read ({e}); treating it as off"
                );
                false
            }
            Ok(bytes) => match serde_json::from_slice::<Stored>(&bytes) {
                Ok(stored) => stored.enabled,
                Err(e) => {
                    eprintln!(
                        "[operator] the About you switch is unreadable ({e}); treating it as off"
                    );
                    false
                }
            },
        };
        SwitchState {
            enabled,
            chosen: true,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<SwitchState>> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The switch and whether a choice is recorded.
    pub(crate) fn state(&self) -> SwitchState {
        let mut state = self.lock();
        *state.get_or_insert_with(|| self.read())
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.state().enabled
    }

    /// Switch it, and persist the choice for the next launch (best effort:
    /// a failed write is logged and the switch still holds for this run).
    pub(crate) fn set(&self, v: bool) {
        let mut state = self.lock();
        *state = Some(SwitchState {
            enabled: v,
            chosen: true,
        });
        let Some(path) = &self.path else {
            return;
        };
        let tmp = path.with_extension(format!(
            "json.{}-{}.tmp",
            std::process::id(),
            SAVE_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let write = || -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&tmp, serde_json::json!({ "enabled": v }).to_string())?;
            std::fs::rename(&tmp, path)
        };
        if let Err(e) = write() {
            let _ = std::fs::remove_file(&tmp);
            eprintln!("[operator] the About you switch could not be saved: {e}");
        }
    }
}

static SWITCH: once_cell::sync::Lazy<Switch> = once_cell::sync::Lazy::new(|| {
    Switch::at(
        crate::agent_settings::app_data_dir()
            .ok()
            .map(|dir| dir.join("operator-context.json")),
    )
});

pub fn set_enabled(v: bool) {
    SWITCH.set(v);
}

pub fn is_enabled() -> bool {
    SWITCH.is_enabled()
}

/// The persisted switch, for the window to adopt at launch.
pub fn switch_state() -> SwitchState {
    SWITCH.state()
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OperatorContext {
    /// Ready-to-inject markdown digest. Empty when no `me/` profile files exist.
    pub body: String,
    /// Vault-relative paths that contributed, in include order.
    pub files: Vec<String>,
    /// Character count of `body` — surfaced in Settings so the user can see
    /// how much gets prepended to each new session.
    pub char_count: usize,
    /// True if any single file or the whole digest hit its cap.
    pub truncated: bool,
    /// What `body` holds of each file, `(vault-relative path, its section's
    /// text)` in include order: the frontmatter stripped and the per-file
    /// cut applied, exactly as sent. A session's profile baseline is this
    /// (the context compiler's revision delta compares what a session was
    /// given, not the files). Not sent to the frontend.
    #[serde(skip)]
    pub sections: Vec<(String, String)>,
    /// Each present `me/` file's whole text (`(vault-relative path, body)`,
    /// frontmatter stripped, never cut), whether or not the digest holds
    /// it. The revision delta reports a line removed only when it is gone
    /// from here, not merely cut from the digest. Not sent to the frontend.
    #[serde(skip)]
    pub sources: Vec<(String, String)>,
    /// Review freshness for all five durable operator files, including files
    /// that are missing or have not adopted freshness frontmatter yet.
    pub freshness: Vec<OperatorFreshness>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FreshnessStatus {
    Fresh,
    Due,
    Overdue,
    NeverReviewed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperatorFreshness {
    pub path: String,
    pub title: String,
    pub exists: bool,
    /// Newer of the file's declared content-write time and filesystem mtime.
    /// Informational only; freshness is based on `last_reviewed` instead.
    pub last_updated: Option<String>,
    /// Most recent explicit human review. Incidental edits must not bump it.
    pub last_reviewed: Option<String>,
    pub review_cadence_days: u32,
    pub status: FreshnessStatus,
    /// Local calendar date on which the next review is due (`YYYY-MM-DD`).
    pub due_at: Option<String>,
    /// Negative only when overdue; null when the file was never reviewed.
    pub days_until_due: Option<i64>,
}

/// Files treated as the durable operator profile, in priority (lead-first)
/// order. The ritual prompts (`today-prompt.md`, `closeday-prompt.md`, …)
/// also live in `me/` but are task-specific, not general house rules, so
/// they're intentionally excluded here.
#[derive(Debug, Clone, Copy)]
struct ProfileFile {
    name: &'static str,
    title: &'static str,
    review_cadence_days: u32,
}

const PROFILE_FILES: &[ProfileFile] = &[
    ProfileFile {
        name: "Identity.md",
        title: "Identity",
        review_cadence_days: 90,
    },
    ProfileFile {
        name: "Communication.md",
        title: "Communication",
        review_cadence_days: 90,
    },
    ProfileFile {
        name: "Anti-patterns.md",
        title: "Anti-patterns",
        review_cadence_days: 60,
    },
    ProfileFile {
        name: "Guardrails.md",
        title: "Guardrails",
        review_cadence_days: 90,
    },
    ProfileFile {
        name: "Decisions.md",
        title: "Decisions",
        review_cadence_days: 30,
    },
];

/// Per-file cap so one long file (Decisions.md tends to grow) can't dominate
/// the digest, and a total cap so the whole block stays a lean preamble.
const PER_FILE_CAP: usize = 6_000;
const TOTAL_CAP: usize = 16_000;
/// The line that ends a file's section the per-file cap cut (after a blank
/// line; the text before it may end in a cut fragment of a line).
pub(crate) const TRUNCATION_MARK: &str = "…(truncated)";
/// The digest's first line (the context compiler reuses it when the only
/// sections are the context layers).
pub(crate) const HEADING: &str =
    "# Operator profile \u{2014} how the user works. Apply this to every reply.";

/// The settled part of the profile (agent effectiveness F6, wave 3 · G2):
/// kept whole (up to the per-file cap) however long the rest is. Two
/// 6,000-byte files always fit the 16,000-byte total.
const RESERVED_FILES: [&str; 2] = ["Decisions.md", "Guardrails.md"];
/// The prose files in the order they keep their room: when the profile is
/// over its total, Identity gives way first, then Communication, then
/// Anti-patterns. Until G2 the digest filled in reading order and stopped,
/// so `Decisions.md`, read last, was the first file cut.
const PROSE_KEEP_ORDER: [&str; 3] = ["Anti-patterns.md", "Communication.md", "Identity.md"];
/// A prose file with less room than this left is left out rather than cut
/// to a stub.
const MIN_PROSE_PART: usize = 200;

/// Largest index `<= max` that lands on a UTF-8 char boundary, so truncation
/// never splits a multi-byte char (`str::truncate` would panic otherwise).
fn floor_boundary(s: &str, max: usize) -> usize {
    if max >= s.len() {
        return s.len();
    }
    let mut i = max;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Read the operator profile from `<vault>/me/` and compile it into a single
/// injectable block. Missing files are skipped; a missing `me/` folder yields
/// an empty (but valid) context.
pub fn compile(vault: &Path) -> OperatorContext {
    compile_at(vault, Local::now().date_naive(), usize::MAX)
}

/// [`compile`] with at most `prose_room` bytes for the prose files
/// (Identity, Communication, Anti-patterns) besides what the 16,000-byte
/// total already allows: a turn's context budget (`context_budget.rs`)
/// passes what it has left for profile prose. The settled files
/// (Decisions, Guardrails) are never charged against it.
pub fn compile_within(vault: &Path, prose_room: usize) -> OperatorContext {
    compile_at(vault, Local::now().date_naive(), prose_room)
}

/// `body` cut to whole lines within `room` bytes, the truncation mark
/// included; `None` when not even [`MIN_PROSE_PART`] fits.
fn cut_to_room(body: &str, room: usize) -> Option<String> {
    let mark = 2 + TRUNCATION_MARK.len();
    if room < MIN_PROSE_PART.max(mark + 1) {
        return None;
    }
    let limit = floor_boundary(body, room - mark);
    let end = body[..limit].rfind('\n')? + 1;
    Some(format!("{}\n{TRUNCATION_MARK}", &body[..end]))
}

fn compile_at(vault: &Path, today: NaiveDate, prose_room: usize) -> OperatorContext {
    let me_dir = vault.join("me");
    let mut sources: Vec<(String, String)> = Vec::new();
    let mut freshness: Vec<OperatorFreshness> = Vec::with_capacity(PROFILE_FILES.len());
    let mut truncated = false;
    // Each present file's section text after the per-file cut, in reading
    // order: `(file name, trimmed body, chunk)`.
    let mut chunks: Vec<(&'static str, String, String)> = Vec::new();

    for profile in PROFILE_FILES {
        let path = me_dir.join(profile.name);
        let raw = std::fs::read_to_string(&path).ok();
        freshness.push(freshness_from_raw(
            profile,
            raw.as_deref(),
            file_mtime(&path),
            today,
        ));
        let Some(raw) = raw else {
            continue; // file absent — fine, profile is best-effort
        };
        // Strip frontmatter so YAML keys (title/tags/type) don't leak in.
        let body = frontmatter::parse(&raw).body;
        let trimmed = body.trim();
        if trimmed.is_empty() {
            continue;
        }
        sources.push((format!("me/{}", profile.name), trimmed.to_string()));
        let mut chunk = trimmed.to_string();
        if chunk.len() > PER_FILE_CAP {
            let cut = floor_boundary(&chunk, PER_FILE_CAP);
            chunk.truncate(cut);
            chunk.push_str("\n\n");
            chunk.push_str(TRUNCATION_MARK);
            truncated = true;
        }
        chunks.push((profile.name, trimmed.to_string(), chunk));
    }

    // The settled files first, whole; then the prose files in their keep
    // order, each whole while it fits, the one that doesn't cut to whole
    // lines, the rest left out.
    let mut kept: Vec<(&'static str, String)> = Vec::new();
    let mut left = TOTAL_CAP;
    for name in RESERVED_FILES {
        if let Some((_, _, chunk)) = chunks.iter().find(|(n, _, _)| *n == name) {
            left = left.saturating_sub(chunk.len());
            kept.push((name, chunk.clone()));
        }
    }
    let mut prose_left = left.min(prose_room);
    for name in PROSE_KEEP_ORDER {
        let Some((_, trimmed, chunk)) = chunks.iter().find(|(n, _, _)| *n == name) else {
            continue;
        };
        if chunk.len() <= prose_left {
            prose_left -= chunk.len();
            kept.push((name, chunk.clone()));
            continue;
        }
        // This one is cut to whole lines (or left out); the rest are left out.
        truncated = true;
        if let Some(part) = cut_to_room(trimmed, prose_left.min(PER_FILE_CAP)) {
            kept.push((name, part));
        }
        prose_left = 0;
    }

    let mut files: Vec<String> = Vec::new();
    let mut sections: Vec<String> = Vec::new();
    let mut given: Vec<(String, String)> = Vec::new();
    for profile in PROFILE_FILES {
        let Some((_, chunk)) = kept.iter().find(|(n, _)| *n == profile.name) else {
            continue;
        };
        files.push(format!("me/{}", profile.name));
        sections.push(format!("## me/{}\n\n{chunk}", profile.name));
        given.push((format!("me/{}", profile.name), chunk.clone()));
    }

    let body = if sections.is_empty() {
        String::new()
    } else {
        format!("{HEADING}\n\n{}", sections.join("\n\n"))
    };
    let char_count = body.chars().count();
    OperatorContext {
        body,
        files,
        char_count,
        truncated,
        freshness,
        sections: given,
        sources,
    }
}

fn freshness_from_raw(
    profile: &ProfileFile,
    raw: Option<&str>,
    fallback_last_updated: Option<String>,
    today: NaiveDate,
) -> OperatorFreshness {
    let parsed = raw.map(frontmatter::parse);
    let field = |key: &str| {
        parsed
            .as_ref()
            .and_then(|fm| fm.get(key))
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    let title = field("title").unwrap_or_else(|| profile.title.to_string());
    // The effective content-write time is the newer of declared metadata and
    // filesystem mtime. That keeps legacy files useful, while a scaffold's
    // initial `last_updated` cannot freeze the field forever as Obsidian/Hollow
    // edits naturally advance mtime. Review-only writes preserve mtime below.
    let last_updated = effective_last_updated(field("last_updated"), fallback_last_updated);
    let last_reviewed = field("last_reviewed");
    let review_cadence_days = field("review_cadence")
        .as_deref()
        .and_then(parse_cadence_days)
        .unwrap_or(profile.review_cadence_days);

    let (status, due_at, days_until_due) =
        match last_reviewed.as_deref().and_then(parse_review_date) {
            None => (FreshnessStatus::NeverReviewed, None, None),
            Some(reviewed) => {
                let due = reviewed
                    .checked_add_days(Days::new(review_cadence_days as u64))
                    .unwrap_or(reviewed);
                let until = due.signed_duration_since(today).num_days();
                let elapsed = today.signed_duration_since(reviewed).num_days().max(0);
                // The last quarter of a cadence is the review window: visible
                // before the deadline, but distinct from truly overdue context.
                let status = if until < 0 {
                    FreshnessStatus::Overdue
                } else if elapsed.saturating_mul(4)
                    >= i64::from(review_cadence_days).saturating_mul(3)
                {
                    FreshnessStatus::Due
                } else {
                    FreshnessStatus::Fresh
                };
                (
                    status,
                    Some(due.format("%Y-%m-%d").to_string()),
                    Some(until),
                )
            }
        };

    OperatorFreshness {
        path: format!("me/{}", profile.name),
        title,
        exists: raw.is_some(),
        last_updated,
        last_reviewed,
        review_cadence_days,
        status,
        due_at,
        days_until_due,
    }
}

fn parse_cadence_days(value: &str) -> Option<u32> {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    let numeric = lower
        .strip_prefix('p')
        .and_then(|v| v.strip_suffix('d'))
        .or_else(|| lower.strip_suffix("days"))
        .or_else(|| lower.strip_suffix("day"))
        .or_else(|| lower.strip_suffix('d'))
        .unwrap_or(&lower)
        .trim();
    numeric.parse::<u32>().ok().filter(|days| *days > 0)
}

fn parse_review_date(value: &str) -> Option<NaiveDate> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Local).date_naive())
        .or_else(|| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
}

fn effective_last_updated(
    declared: Option<String>,
    filesystem_mtime: Option<String>,
) -> Option<String> {
    match (declared, filesystem_mtime) {
        (Some(declared), Some(mtime)) => {
            match (parse_update_time(&declared), parse_update_time(&mtime)) {
                (Some(declared_at), Some(mtime_at)) => Some(if declared_at >= mtime_at {
                    declared
                } else {
                    mtime
                }),
                (Some(_), None) => Some(declared),
                // An invalid declared value must not mask a valid filesystem
                // timestamp; this is a read-only compatibility fallback.
                (None, Some(_)) => Some(mtime),
                (None, None) => Some(declared),
            }
        }
        (Some(declared), None) => Some(declared),
        (None, Some(mtime)) => Some(mtime),
        (None, None) => None,
    }
}

fn parse_update_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|| {
            NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .ok()?
                .and_hms_opt(0, 0, 0)
                .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc))
        })
}

fn file_mtime(path: &Path) -> Option<String> {
    path.metadata()
        .ok()?
        .modified()
        .ok()
        .map(DateTime::<Utc>::from)
        .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

fn profile_for_path(path: &str) -> Option<&'static ProfileFile> {
    PROFILE_FILES
        .iter()
        .find(|profile| path == format!("me/{}", profile.name))
}

/// Record an explicit user review of one of the five operator files. The
/// allowlist prevents this narrowly-scoped mutation from becoming a generic
/// vault writer. Only `last_reviewed` is inserted/replaced; body bytes and all
/// other frontmatter lines are preserved verbatim.
pub fn mark_reviewed(vault_root: &Path, path: &str) -> Result<OperatorFreshness, String> {
    let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    mark_reviewed_at(vault_root, path, &now, Local::now().date_naive())
}

fn mark_reviewed_at(
    vault_root: &Path,
    path: &str,
    reviewed_at: &str,
    today: NaiveDate,
) -> Result<OperatorFreshness, String> {
    let profile =
        profile_for_path(path).ok_or_else(|| format!("not an operator-profile file: {path}"))?;
    vault::mutate_note_preserve_mtime(vault_root, path, |content| {
        let next = upsert_frontmatter_scalar(content, "last_reviewed", reviewed_at);
        // Hollow's own write of a protected file: a Settings "mark reviewed"
        // during an automated run is not flagged, while an agent edit made
        // earlier in that run still is (see `run_guard`).
        crate::run_guard::note_host_vault_write(
            vault_root,
            path,
            Some(content.as_bytes()),
            Some(next.as_bytes()),
        );
        Ok(next)
    })?;
    let raw = std::fs::read_to_string(vault_root.join(path))
        .map_err(|e| format!("read reviewed operator profile: {e}"))?;
    Ok(freshness_from_raw(
        profile,
        Some(&raw),
        file_mtime(&vault_root.join(path)),
        today,
    ))
}

/// Textual frontmatter mutation rather than parse-and-render: the canonical
/// scalar parser intentionally drops YAML list items, so rebuilding a block
/// from parsed pairs could corrupt unrelated user-authored metadata.
fn upsert_frontmatter_scalar(content: &str, key: &str, value: &str) -> String {
    let newline = if content.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let scalar = if frontmatter::needs_quoting(value) {
        frontmatter::quote(value)
    } else {
        value.to_string()
    };

    if let Some((block_start, closing_start)) = frontmatter_bounds(content) {
        let mut cursor = block_start;
        let mut found: Option<(usize, usize)> = None;
        while cursor < closing_start {
            let line_end = content[cursor..closing_start]
                .find('\n')
                .map(|offset| cursor + offset + 1)
                .unwrap_or(closing_start);
            let line = content[cursor..line_end].trim_end_matches(['\r', '\n']);
            if line
                .split_once(':')
                .map(|(candidate, _)| {
                    candidate == candidate.trim_start() && candidate.trim_end() == key
                })
                .unwrap_or(false)
            {
                found = Some((cursor, line_end));
            }
            if line_end == cursor {
                break;
            }
            cursor = line_end;
        }

        let replacement = format!("{key}: {scalar}{newline}");
        let (start, end) = found.unwrap_or((closing_start, closing_start));
        let mut out = String::with_capacity(content.len() + replacement.len());
        out.push_str(&content[..start]);
        out.push_str(&replacement);
        out.push_str(&content[end..]);
        return out;
    }

    let bom_len = if content.starts_with('\u{feff}') {
        '\u{feff}'.len_utf8()
    } else {
        0
    };
    let mut out = String::with_capacity(content.len() + scalar.len() + 32);
    out.push_str(&content[..bom_len]);
    out.push_str("---");
    out.push_str(newline);
    out.push_str(key);
    out.push_str(": ");
    out.push_str(&scalar);
    out.push_str(newline);
    out.push_str("---");
    out.push_str(newline);
    out.push_str(&content[bom_len..]);
    out
}

/// Byte offsets for the first frontmatter content line and closing fence.
pub(crate) fn frontmatter_bounds(content: &str) -> Option<(usize, usize)> {
    let bom_len = if content.starts_with('\u{feff}') {
        '\u{feff}'.len_utf8()
    } else {
        0
    };
    let after_bom = &content[bom_len..];
    let trimmed = after_bom.trim_start();
    let opening_start = bom_len + (after_bom.len() - trimmed.len());
    let opening_tail = &content[opening_start..];
    if !(opening_tail.starts_with("---\n") || opening_tail.starts_with("---\r\n")) {
        return None;
    }
    let first_newline = opening_tail.find('\n')?;
    let block_start = opening_start + first_newline + 1;
    let mut cursor = block_start;
    while cursor <= content.len() {
        let line_end = content[cursor..]
            .find('\n')
            .map(|offset| cursor + offset)
            .unwrap_or(content.len());
        if content[cursor..line_end].trim_end_matches('\r') == "---" {
            return Some((block_start, cursor));
        }
        if line_end == content.len() {
            break;
        }
        cursor = line_end + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A fresh folder unique to this call, removed when dropped (a failed
    /// test too). A fixed `$TMPDIR/hollow-operator-<tag>` let two worktrees
    /// running `cargo test` with one `TMPDIR` delete each other's fixtures.
    struct Scratch(tempfile::TempDir);

    impl std::ops::Deref for Scratch {
        type Target = Path;
        fn deref(&self) -> &Path {
            self.0.path()
        }
    }

    impl AsRef<Path> for Scratch {
        fn as_ref(&self) -> &Path {
            self.0.path()
        }
    }

    fn scratch(tag: &str) -> Scratch {
        Scratch(
            tempfile::Builder::new()
                .prefix(&format!("hollow-operator-{tag}-"))
                .tempdir()
                .expect("operator test scratch"),
        )
    }

    #[test]
    fn scratch_folders_are_unique_per_call() {
        let (a, b) = (scratch("same"), scratch("same"));
        assert_ne!(a.to_path_buf(), b.to_path_buf());
        assert!(a.is_dir() && b.is_dir());
        let kept = a.to_path_buf();
        drop(a);
        assert!(!kept.exists(), "dropping the scratch removes it");
    }

    fn set_mtime(path: &Path, epoch_seconds: u64) {
        let modified = std::time::UNIX_EPOCH + std::time::Duration::from_secs(epoch_seconds);
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();
    }

    #[test]
    fn empty_when_no_me_dir() {
        let dir = scratch("empty");
        fs::create_dir_all(&dir).unwrap();
        let ctx = compile(&dir);
        assert!(ctx.body.is_empty());
        assert!(ctx.files.is_empty());
        assert_eq!(ctx.char_count, 0);
        assert!(!ctx.truncated);
        assert_eq!(ctx.freshness.len(), 5);
        assert!(ctx
            .freshness
            .iter()
            .all(|item| { !item.exists && item.status == FreshnessStatus::NeverReviewed }));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn compiles_in_priority_order_and_strips_frontmatter() {
        let dir = scratch("compile");
        let me = dir.join("me");
        fs::create_dir_all(&me).unwrap();
        // Written out of order — output must still lead with Identity.
        fs::write(
            me.join("Communication.md"),
            "---\ntitle: Communication\ntags:\n  - x\n---\nNo em dashes. Be direct.",
        )
        .unwrap();
        fs::write(
            me.join("Identity.md"),
            "---\ntitle: Identity\n---\nAVP of AI.",
        )
        .unwrap();
        let ctx = compile(&dir);
        let id_at = ctx
            .body
            .find("## me/Identity.md")
            .expect("identity present");
        let comm_at = ctx
            .body
            .find("## me/Communication.md")
            .expect("comm present");
        assert!(id_at < comm_at, "Identity must lead Communication");
        assert!(ctx.body.contains("AVP of AI."));
        assert!(ctx.body.contains("No em dashes. Be direct."));
        // Frontmatter keys must not leak into the digest.
        assert!(!ctx.body.contains("title: Communication"));
        assert_eq!(
            ctx.files,
            vec![
                "me/Identity.md".to_string(),
                "me/Communication.md".to_string()
            ]
        );
        assert!(!ctx.truncated);
        let _ = fs::remove_dir_all(&dir);
    }

    /// F6 acceptance (G2, red first): a profile over the 16,000-byte total
    /// keeps its settled decisions and guardrails whole. The digest used to
    /// fill in reading order and stop, so `Decisions.md`, last, went first.
    /// Prose gives way instead (Identity first, then Communication, then
    /// Anti-patterns), and the reading order is unchanged.
    #[test]
    fn an_overflowing_profile_keeps_decisions_and_guardrails() {
        let dir = scratch("overflow");
        let me = dir.join("me");
        fs::create_dir_all(&me).unwrap();
        let prose = |name: &str| -> String {
            (1..=100)
                .map(|i| format!("{name} line {i:03} {}\n", "p".repeat(40)))
                .collect()
        };
        fs::write(me.join("Identity.md"), prose("Identity")).unwrap();
        fs::write(me.join("Communication.md"), prose("Comms")).unwrap();
        fs::write(me.join("Anti-patterns.md"), prose("Anti")).unwrap();
        fs::write(
            me.join("Guardrails.md"),
            "- Never send email without asking (guardrail amber-fox).\n",
        )
        .unwrap();
        fs::write(
            me.join("Decisions.md"),
            "- Churn is measured on paid seats (decision quartz-heron).\n",
        )
        .unwrap();
        let ctx = compile(&dir);
        assert!(ctx.truncated);
        assert!(
            ctx.body.contains("decision quartz-heron"),
            "Decisions was cut: {:?}",
            ctx.files
        );
        assert!(
            ctx.body.contains("guardrail amber-fox"),
            "Guardrails was cut: {:?}",
            ctx.files
        );
        let total: usize = ctx.sections.iter().map(|(_, text)| text.len()).sum();
        assert!(total <= TOTAL_CAP, "{total}");
        // Reading order stays Identity → … → Decisions.
        let at = |name: &str| ctx.body.find(&format!("## me/{name}"));
        assert!(at("Guardrails.md").unwrap() < at("Decisions.md").unwrap());
        if let (Some(identity), Some(anti)) = (at("Identity.md"), at("Anti-patterns.md")) {
            assert!(identity < anti);
        }
        // Prose gives way from Identity: Anti-patterns is kept whole.
        assert!(ctx.body.contains("Anti line 100"), "Anti-patterns was cut");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn per_file_cap_truncates() {
        let dir = scratch("trunc");
        let me = dir.join("me");
        fs::create_dir_all(&me).unwrap();
        fs::write(me.join("Decisions.md"), "y".repeat(PER_FILE_CAP + 500)).unwrap();
        let ctx = compile(&dir);
        assert!(ctx.truncated);
        assert!(ctx.body.contains("…(truncated)"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn freshness_uses_reviewed_not_updated_and_honors_per_file_cadence() {
        let dir = scratch("freshness");
        let me = dir.join("me");
        fs::create_dir_all(&me).unwrap();
        let identity_path = me.join("Identity.md");
        fs::write(
            &identity_path,
            "---\ntitle: My identity\nlast_updated: 2026-06-14\nlast_reviewed: 2026-03-07\nreview_cadence: 100\n---\nIdentity body",
        )
        .unwrap();
        set_mtime(&identity_path, 1_700_000_000);
        fs::write(
            me.join("Communication.md"),
            "---\nlast_updated: 2020-01-01\nlast_reviewed: 2026-06-01\nreview_cadence: P100D\n---\nCommunication body",
        )
        .unwrap();
        fs::write(
            me.join("Anti-patterns.md"),
            "---\nlast_updated: 2026-06-14\nlast_reviewed: 2026-01-01\nreview_cadence: 30d\n---\nAnti-pattern body",
        )
        .unwrap();
        fs::write(
            me.join("Guardrails.md"),
            "---\nlast_updated: 2026-06-14\nreview_cadence: 365 days\n---\nGuardrail body",
        )
        .unwrap();

        let today = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
        let ctx = compile_at(&dir, today, usize::MAX);
        assert_eq!(ctx.freshness.len(), 5);
        let status = |path: &str| ctx.freshness.iter().find(|item| item.path == path).unwrap();
        let identity = status("me/Identity.md");
        assert_eq!(identity.title, "My identity");
        assert_eq!(identity.last_updated.as_deref(), Some("2026-06-14"));
        assert_eq!(identity.review_cadence_days, 100);
        assert_eq!(identity.status, FreshnessStatus::Due);
        assert_eq!(identity.due_at.as_deref(), Some("2026-06-15"));
        assert_eq!(identity.days_until_due, Some(0));

        // An old write does not make a recently reviewed file stale.
        assert_eq!(status("me/Communication.md").status, FreshnessStatus::Fresh);
        assert_eq!(
            status("me/Anti-patterns.md").status,
            FreshnessStatus::Overdue
        );
        assert_eq!(
            status("me/Guardrails.md").status,
            FreshnessStatus::NeverReviewed
        );
        let missing = status("me/Decisions.md");
        assert!(!missing.exists);
        assert_eq!(missing.status, FreshnessStatus::NeverReviewed);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mark_reviewed_preserves_body_and_unrelated_frontmatter() {
        let dir = scratch("mark-existing");
        let me = dir.join("me");
        fs::create_dir_all(&me).unwrap();
        let original = "---\r\ntitle: Guardrails\r\nlast_updated: 2026-01-02\r\nlast_reviewed: 2026-01-03\r\nreview_cadence: 90\r\ntags:\r\n  - operator\r\n---\r\n# Guardrails\r\n\r\n- Never rewrite this body.\r\n";
        fs::write(me.join("Guardrails.md"), original).unwrap();
        let original_body = frontmatter::parse(original).body;
        let reviewed_at = "2026-06-15T12:00:00Z";
        let item = mark_reviewed_at(
            &dir,
            "me/Guardrails.md",
            reviewed_at,
            NaiveDate::from_ymd_opt(2026, 6, 15).unwrap(),
        )
        .unwrap();

        let updated = fs::read_to_string(me.join("Guardrails.md")).unwrap();
        assert_eq!(frontmatter::parse(&updated).body, original_body);
        assert!(updated.contains("last_updated: 2026-01-02\r\n"));
        assert!(updated.contains("review_cadence: 90\r\n"));
        assert!(updated.contains("tags:\r\n  - operator\r\n"));
        assert!(updated.contains("last_reviewed: \"2026-06-15T12:00:00Z\"\r\n"));
        assert_eq!(item.last_reviewed.as_deref(), Some(reviewed_at));
        assert_eq!(item.status, FreshnessStatus::Fresh);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mark_reviewed_adds_frontmatter_without_changing_legacy_body() {
        let dir = scratch("mark-legacy");
        let me = dir.join("me");
        fs::create_dir_all(&me).unwrap();
        let original = "\u{feff}<!-- keep this first comment -->\n# Identity\nLegacy content.\n";
        let path = me.join("Identity.md");
        fs::write(&path, original).unwrap();
        let original_mtime = file_mtime(&path).expect("legacy file mtime");
        let item = mark_reviewed_at(
            &dir,
            "me/Identity.md",
            "2026-06-15T12:00:00Z",
            NaiveDate::from_ymd_opt(2026, 6, 15).unwrap(),
        )
        .unwrap();

        let updated = fs::read_to_string(&path).unwrap();
        assert!(updated.starts_with("\u{feff}---\nlast_reviewed:"));
        assert_eq!(
            frontmatter::parse(&updated).body,
            "<!-- keep this first comment -->\n# Identity\nLegacy content.\n"
        );
        assert_eq!(item.status, FreshnessStatus::Fresh);
        assert_eq!(item.review_cadence_days, 90);
        assert_eq!(item.last_updated.as_deref(), Some(original_mtime.as_str()));
        assert_eq!(file_mtime(&path).as_deref(), Some(original_mtime.as_str()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn last_updated_uses_newer_of_declared_and_file_mtime() {
        let dir = scratch("mtime-fallback");
        let me = dir.join("me");
        fs::create_dir_all(&me).unwrap();
        let path = me.join("Identity.md");
        // Scaffold-like file: a declared creation date plus a real filesystem
        // write time. The more precise/later mtime is the effective value.
        fs::write(
            &path,
            "---\nlast_updated: 2023-11-14\n---\n# Identity\nScaffold body.\n",
        )
        .unwrap();
        set_mtime(&path, 1_700_000_000);

        let read = || {
            compile_at(
                &dir,
                NaiveDate::from_ymd_opt(2026, 6, 15).unwrap(),
                usize::MAX,
            )
            .freshness
            .into_iter()
            .find(|item| item.path == "me/Identity.md")
            .unwrap()
            .last_updated
            .unwrap()
        };
        assert_eq!(read(), "2023-11-14T22:13:20Z");

        // A later Hollow/Obsidian content edit advances mtime even though its
        // submitted frontmatter still carries the scaffold's original date.
        set_mtime(&path, 1_700_086_400);
        assert_eq!(read(), "2023-11-15T22:13:20Z");

        // A genuinely newer declared timestamp still wins (for writers that
        // explicitly maintain the field themselves).
        fs::write(
            &path,
            "---\nlast_updated: 2030-01-02\n---\n# Identity\nDeclared wins.\n",
        )
        .unwrap();
        set_mtime(&path, 1_700_086_400);
        assert_eq!(read(), "2030-01-02");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mark_reviewed_inserts_into_the_existing_frontmatter_block() {
        let dir = scratch("mark-insert");
        let me = dir.join("me");
        fs::create_dir_all(&me).unwrap();
        let original =
            "---\ntitle: Communication\ntags:\n  - operator\n---\n# Communication\nKeep me.\n";
        fs::write(me.join("Communication.md"), original).unwrap();
        mark_reviewed_at(
            &dir,
            "me/Communication.md",
            "2026-06-15T12:00:00Z",
            NaiveDate::from_ymd_opt(2026, 6, 15).unwrap(),
        )
        .unwrap();

        let updated = fs::read_to_string(me.join("Communication.md")).unwrap();
        assert_eq!(
            updated.matches("---").count(),
            2,
            "must not stack YAML blocks"
        );
        assert!(updated.contains("tags:\n  - operator\nlast_reviewed:"));
        assert_eq!(
            frontmatter::parse(&updated).body,
            "# Communication\nKeep me.\n"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mark_reviewed_does_not_rewrite_an_indented_nested_key() {
        let dir = scratch("mark-nested");
        let me = dir.join("me");
        fs::create_dir_all(&me).unwrap();
        let original = "---\ntitle: Identity\nreview_history:\n  last_reviewed: 2025-01-01\n---\n# Identity\nKeep me.\n";
        fs::write(me.join("Identity.md"), original).unwrap();
        mark_reviewed_at(
            &dir,
            "me/Identity.md",
            "2026-06-15T12:00:00Z",
            NaiveDate::from_ymd_opt(2026, 6, 15).unwrap(),
        )
        .unwrap();

        let updated = fs::read_to_string(me.join("Identity.md")).unwrap();
        assert!(updated.contains("  last_reviewed: 2025-01-01\n"));
        assert!(updated.contains("\nlast_reviewed: \"2026-06-15T12:00:00Z\"\n---"));
        assert_eq!(updated.matches("last_reviewed:").count(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mark_reviewed_rejects_paths_outside_the_operator_allowlist() {
        let dir = scratch("mark-reject");
        fs::create_dir_all(dir.join("notes")).unwrap();
        fs::write(dir.join("notes/Identity.md"), "do not touch").unwrap();
        let err = mark_reviewed_at(
            &dir,
            "notes/Identity.md",
            "2026-06-15T12:00:00Z",
            NaiveDate::from_ymd_opt(2026, 6, 15).unwrap(),
        )
        .unwrap_err();
        assert!(err.contains("not an operator-profile file"));
        assert_eq!(
            fs::read_to_string(dir.join("notes/Identity.md")).unwrap(),
            "do not touch"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn freshness_wire_shape_is_camel_case() {
        let item = freshness_from_raw(
            &PROFILE_FILES[0],
            Some("---\nlast_reviewed: 2026-06-15\n---\nbody"),
            None,
            NaiveDate::from_ymd_opt(2026, 6, 15).unwrap(),
        );
        let wire = serde_json::to_value(item).unwrap();
        for key in [
            "path",
            "title",
            "exists",
            "lastUpdated",
            "lastReviewed",
            "reviewCadenceDays",
            "status",
            "dueAt",
            "daysUntilDue",
        ] {
            assert!(wire.get(key).is_some(), "missing key {key}");
        }
        assert_eq!(wire["status"], "fresh");
    }

    /// K1 follow-up (red first): the switch is the persisted setting, so a
    /// backend child, an automation or any turn before the frontend's boot
    /// call reads the person's choice, not a process default of "on".
    #[test]
    fn the_about_you_switch_is_read_from_its_persisted_setting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("operator-context.json");
        // Switched off in an earlier launch; this process hasn't heard from
        // the frontend yet.
        fs::write(&path, r#"{"enabled":false}"#).unwrap();
        let fresh = Switch::at(Some(path.clone()));
        assert!(
            !fresh.is_enabled(),
            "a turn before the frontend boots got the profile it was told not to send"
        );
        // A change is persisted for the next launch.
        fresh.set(true);
        assert!(Switch::at(Some(path.clone())).is_enabled());
        fresh.set(false);
        assert!(!Switch::at(Some(path.clone())).is_enabled());
        // Never set: the product default, on.
        let unset = dir.path().join("never.json");
        assert!(Switch::at(Some(unset)).is_enabled());
        // A setting that can't be read is not taken as consent: off.
        fs::write(&path, "not json").unwrap();
        assert!(!Switch::at(Some(path)).is_enabled());
    }

    /// Wave 2 follow-up: the window adopts the backend's switch at launch
    /// (`operator_context_switch`), whatever its own copy says. A file that
    /// says off is reported off and chosen, so a window that lost its
    /// localStorage key stays off; a never-set switch is on and not chosen
    /// (the only case where the window may carry an earlier "off" over).
    #[test]
    fn the_window_adopts_the_persisted_switch_at_launch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("operator-context.json");
        fs::write(&path, r#"{"enabled":false}"#).unwrap();
        assert_eq!(
            Switch::at(Some(path.clone())).state(),
            SwitchState {
                enabled: false,
                chosen: true
            }
        );
        let wire = serde_json::to_value(Switch::at(Some(path.clone())).state()).unwrap();
        assert_eq!(
            wire,
            serde_json::json!({ "enabled": false, "chosen": true })
        );
        let never = Switch::at(Some(dir.path().join("never.json")));
        assert_eq!(
            never.state(),
            SwitchState {
                enabled: true,
                chosen: false
            }
        );
        never.set(false);
        assert_eq!(
            Switch::at(Some(dir.path().join("never.json"))).state(),
            SwitchState {
                enabled: false,
                chosen: true
            },
            "a carried-over off is persisted"
        );
        // Unreadable: off, and a choice exists (it is not overwritten by a
        // default at launch).
        fs::write(&path, "not json").unwrap();
        assert_eq!(
            Switch::at(Some(path)).state(),
            SwitchState {
                enabled: false,
                chosen: true
            }
        );
    }

    /// Wave 2 follow-up (red first): with no app-data folder there is no
    /// record of the person's choice, so the switch reads off (a never-set
    /// switch is on only where a choice could have been recorded).
    #[test]
    fn the_about_you_switch_is_off_without_an_app_data_folder() {
        let unknown = Switch::at(None);
        assert!(
            !unknown.is_enabled(),
            "no app-data folder read as consent to send the profile"
        );
        unknown.set(true);
        assert!(unknown.is_enabled(), "a choice made this run still holds");
    }

    /// Wave 2 follow-up (red first): concurrent changes shared one temp
    /// file, so a rename could land out of order (or fail) and leave the
    /// saved choice different from the one in force. The saved choice is
    /// always the last one made.
    #[test]
    fn concurrent_changes_save_the_choice_in_force() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("operator-context.json");
        for round in 0..20 {
            let switch = std::sync::Arc::new(Switch::at(Some(path.clone())));
            let threads: Vec<_> = (0..8)
                .map(|i| {
                    let switch = switch.clone();
                    std::thread::spawn(move || {
                        for k in 0..10 {
                            switch.set((i + k + round) % 2 == 0);
                        }
                    })
                })
                .collect();
            for t in threads {
                t.join().unwrap();
            }
            let saved = Switch::at(Some(path.clone())).is_enabled();
            assert_eq!(
                saved,
                switch.is_enabled(),
                "round {round}: the saved choice is not the one in force"
            );
        }
        let left: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name != "operator-context.json")
            .collect();
        assert!(left.is_empty(), "temp files left behind: {left:?}");
    }
}
