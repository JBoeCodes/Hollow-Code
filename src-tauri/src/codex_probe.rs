//! Codex helper compatibility probe (agent effectiveness F9, wave 4 · V1).
//!
//! Hollow's tool-free Codex helpers (`codex_transform.rs`) rest on two pieces
//! of evidence per CLI version, both ignored tests against a loopback
//! Responses fixture with an empty `CODEX_HOME` (no provider, no account):
//! `installed_codex_advertises_zero_tools_to_local_responses_fixture` checks
//! that the request advertises no tools, for the default, an explicit and a
//! custom model, before any inference; and (since night ST12)
//! `installed_codex_json_output_parses_from_a_local_fixture_answer` checks
//! that the CLI's real `--json` output for an answer parses through the
//! helper. A version passes both before it joins [`VERIFIED`]; any other
//! version fails closed.
//!
//! This module asks the installed CLI its version once per binary
//! (`codex --version`, bounded) and keeps the answer while the binary is
//! the same file: its canonical path, size, modification and change times
//! and inode, and those of the `package.json` beside a node wrapper's
//! folder, which an npm upgrade replaces. A failed read is never kept. The
//! same answer drives the transform's guard and what Settings, Models
//! and a helper's usage row say, so a helper on an unverified CLI
//! stops with a sentence that names the version, the verified ones and the
//! fallback (nothing runs; Hollow never switches agents by itself).
use crate::process_group;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// Every `codex --version` line whose tool-free boundary the local fixture
/// check proved. 0.146.0 when the transform was written (2026-09); 0.153.4
/// on 2026-10-07 (wave 4 · V1, `ae4-v1-codex-qualify.log`); 0.162.1 on
/// 2026-10-14 (night ST12: zero tools for the default, an explicit and a
/// custom model, and its real `--json` answer parsed through the helper,
/// both against loopback fixtures; pinned in `codex_0_162_1_exec_json_parses`).
pub(crate) const VERIFIED: &[&str] = &[
    "codex-cli 0.146.0",
    "codex-cli 0.153.4",
    "codex-cli 0.162.1",
];

/// How long reading the version may take.
pub(crate) const PROBE_LIMIT: Duration = Duration::from_secs(5);

/// The verified versions, for sentences: "0.146.0, 0.153.4".
pub(crate) fn verified_list() -> String {
    VERIFIED
        .iter()
        .map(|v| v.trim_start_matches("codex-cli "))
        .collect::<Vec<_>>()
        .join(", ")
}

/// What a version read established, for Settings and receipts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexHelperStatus {
    /// `verified`, `unverified`, `missing` or `unknown`.
    pub status: String,
    /// What `codex --version` printed (its first line), when it ran.
    pub version: Option<String>,
    /// The versions Hollow verified ([`VERIFIED`]).
    pub verified_versions: Vec<String>,
    /// `probed` (the CLI was asked) or `cached` (the same binary's earlier
    /// answer); absent when nothing was read.
    pub served: Option<String>,
    /// One sentence: the state and, when helpers can't run, the fallback.
    pub message: String,
}

impl CodexHelperStatus {
    pub(crate) fn verified(&self) -> bool {
        self.status == "verified"
    }

    fn new(status: &str, version: Option<String>, served: Option<Served>, message: String) -> Self {
        CodexHelperStatus {
            status: status.into(),
            version,
            verified_versions: VERIFIED.iter().map(|v| v.to_string()).collect(),
            served: served.map(|s| s.as_str().to_string()),
            message,
        }
    }

    /// The status for a version line.
    pub(crate) fn of_version(version: &str, served: Served) -> Self {
        let version = version.trim().to_string();
        if VERIFIED.contains(&version.as_str()) {
            let message = format!(
                "Codex helpers run tool-free on {version}, a version Hollow verified with its local check."
            );
            Self::new("verified", Some(version), Some(served), message)
        } else {
            let message = format!(
                "Codex helpers don't run on {version}: Hollow hasn't verified its tool-free mode on this version (verified: {}). Nothing ran, and Hollow doesn't switch agents by itself: choose Claude Code as the default agent for helpers, or install a verified Codex.",
                verified_list()
            );
            Self::new("unverified", Some(version), Some(served), message)
        }
    }

    pub(crate) fn missing() -> Self {
        Self::new(
            "missing",
            None,
            None,
            "Codex isn't installed, so Codex helpers can't run.".into(),
        )
    }

    pub(crate) fn unknown(reason: &str) -> Self {
        Self::new(
            "unknown",
            None,
            None,
            format!(
                "Hollow couldn't read the Codex version ({}), so Codex helpers don't run. Nothing ran.",
                crate::claude_runner::truncate(reason, 160)
            ),
        )
    }

    /// The short form for a usage row's note (at most 200 characters).
    pub(crate) fn note(&self) -> String {
        match (self.status.as_str(), &self.version) {
            ("unverified", Some(version)) => format!(
                "Not run: {version} isn't verified for tool-free helpers (verified: {}).",
                verified_list()
            ),
            ("missing", _) => "Not run: Codex isn't installed.".into(),
            _ => crate::claude_runner::truncate(&self.message, 200),
        }
    }
}

/// Whether a version came from the CLI or from the cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Served {
    Probed,
    Cached,
}

impl Served {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Served::Probed => "probed",
            Served::Cached => "cached",
        }
    }
}

/// One file's identity. The change time and inode catch a replacement that
/// keeps the size and modification time (npm unpacks with fixed times).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FileId {
    path: PathBuf,
    size: u64,
    modified: Option<std::time::SystemTime>,
    changed: i64,
    changed_ns: i64,
    inode: u64,
}

impl FileId {
    fn of(path: &Path) -> Option<Self> {
        let path = std::fs::canonicalize(path).ok()?;
        let meta = std::fs::metadata(&path).ok()?;
        #[cfg(unix)]
        let (changed, changed_ns, inode) = {
            use std::os::unix::fs::MetadataExt;
            (meta.ctime(), meta.ctime_nsec(), meta.ino())
        };
        #[cfg(not(unix))]
        let (changed, changed_ns, inode) = (0, 0, 0);
        Some(FileId {
            size: meta.len(),
            modified: meta.modified().ok(),
            changed,
            changed_ns,
            inode,
            path,
        })
    }
}

/// What a version answer depends on: the binary and, for a node wrapper
/// (`…/@openai/codex/bin/codex.js`), its package's manifest.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct BinaryKey {
    bin: FileId,
    package: Option<FileId>,
}

impl BinaryKey {
    /// `None` when `bin` doesn't resolve to a file (not installed).
    pub(crate) fn of(bin: &Path) -> Option<Self> {
        let bin = FileId::of(bin)?;
        let package = bin
            .path
            .parent()
            .and_then(Path::parent)
            .map(|p| p.join("package.json"))
            .and_then(|p| FileId::of(&p));
        Some(BinaryKey { bin, package })
    }
}

/// Versions by binary. Only successful reads are kept.
#[derive(Default)]
pub(crate) struct Cache {
    entries: Mutex<HashMap<BinaryKey, String>>,
}

/// At most this many binaries are remembered (the oldest is dropped first
/// by clearing: a process sees one or two in practice).
const MAX_ENTRIES: usize = 8;

impl Cache {
    pub(crate) fn get(&self, key: &BinaryKey) -> Option<String> {
        self.entries.lock().ok()?.get(key).cloned()
    }

    pub(crate) fn put(&self, key: BinaryKey, version: String) {
        if let Ok(mut entries) = self.entries.lock() {
            if entries.len() >= MAX_ENTRIES && !entries.contains_key(&key) {
                entries.clear();
            }
            entries.insert(key, version);
        }
    }
}

/// The app's cache: one per process.
pub(crate) static APP: once_cell::sync::Lazy<Cache> = once_cell::sync::Lazy::new(Cache::default);

/// Why a version read ended without an answer.
#[derive(Debug)]
pub(crate) enum ReadEnd {
    /// The binary isn't there.
    Missing,
    /// The caller's stop signal.
    Stopped,
    /// The time limit.
    TimedOut,
    /// It ran and failed, or printed nothing.
    Failed(String),
}

/// The CLI's version line through `cache`: the same binary's earlier answer,
/// else `bin --version` (in `dir`, bounded by `limit` and `stop`, its whole
/// process group ended), kept only when it succeeded. `outcome` folds in
/// how the read's processes ended.
pub(crate) async fn read_version(
    cache: &Cache,
    bin: &Path,
    dir: &Path,
    limit: Duration,
    stop: Option<&process_group::StopSignal>,
    outcome: &mut Option<process_group::StopOutcome>,
) -> Result<(String, Served), ReadEnd> {
    let Some(key) = BinaryKey::of(bin) else {
        return Err(ReadEnd::Missing);
    };
    if let Some(version) = cache.get(&key) {
        return Ok((version, Served::Cached));
    }
    if stop.is_some_and(process_group::StopSignal::requested) {
        return Err(ReadEnd::Stopped);
    }
    let mut command = tokio::process::Command::new(bin);
    command
        .arg("--version")
        .current_dir(dir)
        .env("PATH", crate::agent::cli_path());
    let output = process_group::run_to_completion(&mut command, None, limit, stop)
        .await
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => ReadEnd::Missing,
            _ => ReadEnd::Failed(format!("Read Codex version: {e}")),
        })?;
    *outcome = process_group::StopOutcome::combine(*outcome, output.outcome);
    if output.stopped {
        return Err(ReadEnd::Stopped);
    }
    if output.timed_out {
        return Err(ReadEnd::TimedOut);
    }
    if !output.status.is_some_and(|status| status.success()) {
        return Err(ReadEnd::Failed("codex --version failed".into()));
    }
    let version = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_string();
    if version.is_empty() {
        return Err(ReadEnd::Failed("codex --version printed nothing".into()));
    }
    // The answer belongs to the file that ran; a binary replaced while it
    // ran is read again next time.
    if BinaryKey::of(bin).as_ref() == Some(&key) {
        cache.put(key, version.clone());
    }
    Ok((version, Served::Probed))
}

/// The status of the Codex at `bin` (None: not installed), through `cache`.
pub(crate) async fn status_of(cache: &Cache, bin: Option<&Path>) -> CodexHelperStatus {
    let Some(bin) = bin else {
        return CodexHelperStatus::missing();
    };
    let dir = std::env::temp_dir();
    let mut outcome = None;
    match read_version(cache, bin, &dir, PROBE_LIMIT, None, &mut outcome).await {
        Ok((version, served)) => CodexHelperStatus::of_version(&version, served),
        Err(ReadEnd::Missing) => CodexHelperStatus::missing(),
        Err(ReadEnd::TimedOut) => CodexHelperStatus::unknown("it didn't answer in 5 s"),
        Err(ReadEnd::Stopped) => CodexHelperStatus::unknown("stopped"),
        Err(ReadEnd::Failed(reason)) => CodexHelperStatus::unknown(&reason),
    }
}

/// The installed Codex's status, through the app's cache (Settings, Models,
/// IPC `codex_helper_status`).
pub(crate) async fn installed_status() -> CodexHelperStatus {
    let bin = crate::agent::installed_binary(crate::agent::AgentKind::Codex);
    status_of(&APP, bin.as_deref()).await
}

#[cfg(all(test, unix))]
pub(crate) mod tests {
    use super::*;

    /// A fake `codex` that prints `version` and counts its runs in `runs`.
    pub(crate) fn fake_codex(dir: &Path, version: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.join("codex");
        let runs = dir.join("runs");
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\necho run >> '{}'\nif [ \"$1\" = \"--version\" ]; then echo '{version}'; exit 0; fi\nexit 3\n",
                runs.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin
    }

    fn runs(dir: &Path) -> usize {
        std::fs::read_to_string(dir.join("runs"))
            .map(|s| s.lines().count())
            .unwrap_or(0)
    }

    #[tokio::test]
    async fn a_verified_version_is_probed_once_per_binary() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_codex(dir.path(), "codex-cli 0.153.4");
        let cache = Cache::default();
        let first = status_of(&cache, Some(&bin)).await;
        assert!(first.verified(), "{first:?}");
        assert_eq!(first.version.as_deref(), Some("codex-cli 0.153.4"));
        assert_eq!(first.served.as_deref(), Some("probed"));
        let again = status_of(&cache, Some(&bin)).await;
        assert_eq!(again.served.as_deref(), Some("cached"));
        assert_eq!(runs(dir.path()), 1, "the CLI ran once");
        for version in VERIFIED {
            let other = tempfile::tempdir().unwrap();
            let bin = fake_codex(other.path(), version);
            assert!(status_of(&cache, Some(&bin)).await.verified(), "{version}");
        }
    }

    #[tokio::test]
    async fn an_unverified_version_says_so_with_the_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_codex(dir.path(), "codex-cli 0.160.0");
        let cache = Cache::default();
        let status = status_of(&cache, Some(&bin)).await;
        assert_eq!(status.status, "unverified");
        assert!(!status.verified());
        for part in [
            "codex-cli 0.160.0",
            "0.146.0, 0.153.4",
            "Nothing ran",
            "Claude Code",
        ] {
            assert!(status.message.contains(part), "{part}: {}", status.message);
        }
        let note = status.note();
        assert!(
            note.len() <= 200 && note.contains("codex-cli 0.160.0"),
            "{note}"
        );
        // A patch release of a verified minor is not verified by itself.
        let patch = tempfile::tempdir().unwrap();
        let bin = fake_codex(patch.path(), "codex-cli 0.153.5");
        assert_eq!(status_of(&cache, Some(&bin)).await.status, "unverified");
    }

    #[tokio::test]
    async fn a_missing_or_failing_codex_is_reported_and_never_cached() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::default();
        assert_eq!(status_of(&cache, None).await.status, "missing");
        let absent = dir.path().join("nowhere").join("codex");
        assert_eq!(status_of(&cache, Some(&absent)).await.status, "missing");
        // A CLI that fails its version read: unknown, asked again next time.
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.path().join("codex");
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\necho run >> '{}'\nexit 2\n",
                dir.path().join("runs").display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let failed = status_of(&cache, Some(&bin)).await;
        assert_eq!(failed.status, "unknown");
        assert!(failed.message.contains("Nothing ran"));
        let _ = status_of(&cache, Some(&bin)).await;
        assert_eq!(runs(dir.path()), 2, "a failure is never kept");
    }

    #[tokio::test]
    async fn an_upgraded_binary_is_probed_again() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_codex(dir.path(), "codex-cli 0.146.0");
        let cache = Cache::default();
        assert!(status_of(&cache, Some(&bin)).await.verified());
        // Same path, same size: the replacement's change time and inode
        // still tell it apart.
        let replacement = dir.path().join("codex.new");
        std::fs::copy(&bin, &replacement).unwrap();
        let text = std::fs::read_to_string(&replacement)
            .unwrap()
            .replace("0.146.0", "0.199.0");
        std::fs::write(&replacement, text).unwrap();
        std::fs::rename(&replacement, &bin).unwrap();
        let after = status_of(&cache, Some(&bin)).await;
        assert_eq!(after.status, "unverified");
        assert_eq!(after.served.as_deref(), Some("probed"));
    }
}
