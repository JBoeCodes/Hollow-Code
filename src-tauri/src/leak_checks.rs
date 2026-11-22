//! Leak checks (context layers CX10): flag-only for a week, then enforced
//! (CX10b). Three places, all on the shared rules (`scan_rules`, the vendored
//! `vendor/ctx-scan/scan-rules.json`):
//!
//! - **Write time.** [`check_write`] runs `memory_guard::check_layer_as` in
//!   the current [`mode`]. In [`LeakMode::Flag`] (the default until CX10b) a
//!   Project write whose text names a computer or a person is written and one
//!   line is appended to the flags ledger (`context-leak-flags.jsonl` in the
//!   app data folder): time, layer, relative path, rule ids, severity, the
//!   layer the text belongs to, actor and chat. Never the text or a value. A
//!   secret (the safety scan, a key file name) is refused in both modes.
//!   `run_guard` adds its instruction-file scan hits to the same ledger
//!   ([`record_run_flags`]).
//! - **Push time.** [`install_at`] writes a local `pre-push` hook (never
//!   committed: the path git gives for `hooks/pre-push`, which must sit inside
//!   the repo's git folder) that runs a copy of `ctx-scan` through
//!   `/usr/bin/python3` with the repo's layer and stops a push only on a
//!   blocking finding. The copy lives in the app data folder (`ctx-scan/`),
//!   written from the binary on every install and audit: app data outlives
//!   app updates and moves, while the app bundle's own resources change path
//!   or vanish with them. [`remove_at`] removes only a hook with
//!   [`HOOK_MARKER`].
//! - **Audit.** [`audit`] runs the same copy over the org clone, the personal
//!   clone (`home/` at the home layer, the rest at personal, where Acme
//!   facts are allowed) and every connected project repo, and lists the
//!   week's flags. Findings are file, line and rule id only.
//!
//! **The switch.** One setting, `context.leakChecks` (`flag` | `block`,
//! stored in `context-leak-checks.json`), read by [`mode`]; while it is unset
//! [`DEFAULT_MODE`] applies. CX10b flips that one constant.

use crate::memory_guard::{self, LayerFlag, Refusal, Verdict, WriteLayer};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// What a scan-rule hit at write time does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeakMode {
    /// Write it and record a flag (the first week).
    Flag,
    /// Refuse it, naming the right layer (CX7's refusal).
    Block,
}

impl LeakMode {
    pub(crate) const VALUES: [&'static str; 2] = ["flag", "block"];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            LeakMode::Flag => "flag",
            LeakMode::Block => "block",
        }
    }

    pub(crate) fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "flag" => Some(LeakMode::Flag),
            "block" => Some(LeakMode::Block),
            _ => None,
        }
    }
}

/// The mode while `context.leakChecks` is unset. CX10b flips this to
/// [`LeakMode::Block`] after the flag-only week.
pub(crate) const DEFAULT_MODE: LeakMode = LeakMode::Flag;

const SETTINGS_FILE: &str = "context-leak-checks.json";
/// The flags ledger in the app data folder (outside the vault, so a run's
/// agent can't plant or erase a line).
pub(crate) const FLAGS_FILE: &str = "context-leak-flags.jsonl";
const SCANNER_DIR: &str = "ctx-scan";
/// The first line after the shebang of every hook Hollow writes.
pub(crate) const HOOK_MARKER: &str = "# hollow-context-check v1";
const PYTHON: &str = "/usr/bin/python3";
const SCANNER_SRC: &str = include_str!("../../vendor/ctx-scan/ctx-scan");
const LAYERS: [&str; 5] = ["home", "personal", "org", "project", "vault"];
const MAX_FIELD_CHARS: usize = 300;
const MAX_RULES: usize = 20;
/// Findings an audit lists per repo (the counts cover all of them): 200 for
/// up to three scans, 25 each beyond that, and 500 across the whole audit.
const AUDIT_FINDINGS_MAX: usize = 200;
const AUDIT_FINDINGS_MANY: usize = 25;
const AUDIT_FINDINGS_TOTAL: usize = 500;
/// One scan at most, and the whole audit at most.
const AUDIT_TIMEOUT: Duration = Duration::from_secs(120);
const AUDIT_DEADLINE: Duration = Duration::from_secs(300);
/// Project repos an audit or the Settings list looks at.
const PROJECTS_MAX: usize = 60;

static FLAGS_LOCK: Mutex<()> = Mutex::new(());
/// Makes temp and backup names unique within this process.
static UNIQUE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// `<pid>-<nanos>-<counter>`: no two calls share it, in one process or two.
fn unique_suffix() -> String {
    let n = UNIQUE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{}-{nanos}-{n}", std::process::id())
}

fn app_data() -> Result<PathBuf, String> {
    crate::agent_settings::app_data_dir()
}

// ---------------------------------------------------------------------------
// The switch
// ---------------------------------------------------------------------------

#[derive(Default, Serialize, Deserialize)]
struct ModeFile {
    #[serde(default)]
    mode: Option<LeakMode>,
}

/// The mode saved at `path`; [`DEFAULT_MODE`] when there is no file or it
/// names no mode. A file that exists but doesn't read (damaged, unreadable)
/// reads as [`LeakMode::Block`], the safer side.
pub(crate) fn mode_at(path: &Path) -> LeakMode {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return DEFAULT_MODE,
        Err(_) => return LeakMode::Block,
    };
    match serde_json::from_str::<ModeFile>(&text) {
        Ok(f) => f.mode.unwrap_or(DEFAULT_MODE),
        Err(_) => LeakMode::Block,
    }
}

/// The current leak-check mode (`context.leakChecks`).
pub(crate) fn mode() -> LeakMode {
    app_data()
        .map(|d| mode_at(&d.join(SETTINGS_FILE)))
        .unwrap_or(DEFAULT_MODE)
}

pub(crate) fn set_mode_at(path: &Path, value: &str) -> Result<LeakMode, String> {
    let mode = LeakMode::parse(value)
        .ok_or_else(|| format!("Leak checks are flag or block, not {value:?}."))?;
    let text =
        serde_json::to_string_pretty(&ModeFile { mode: Some(mode) }).map_err(|e| e.to_string())?;
    write_atomic(path, text.as_bytes(), 0o600)?;
    Ok(mode)
}

/// Save `context.leakChecks`.
pub(crate) fn set_mode(value: &str) -> Result<LeakMode, String> {
    set_mode_at(&app_data()?.join(SETTINGS_FILE), value)
}

fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    let dir = path.parent().ok_or("no folder")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let tmp = dir.join(format!(
        ".{}.hollow-tmp-{}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
        unique_suffix()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode);
    }
    let mut file = options
        .open(&tmp)
        .map_err(|e| format!("write {}: {e}", tmp.display()))?;
    if let Err(e) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        drop(file);
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("write {}: {e}", tmp.display()));
    }
    drop(file);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode));
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("write {}: {e}", path.display())
    })
}

// ---------------------------------------------------------------------------
// The flags ledger
// ---------------------------------------------------------------------------

/// One flag: what matched where, never the text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LeakFlag {
    pub at: String,
    /// `write` (a context writer, [`check_write`]) or `run` (an automated
    /// run's instruction-file edit, `run_guard`).
    pub source: String,
    /// The layer of the target (`project`, ...).
    pub layer: String,
    /// The target, relative to its layer's root (or the vault).
    pub path: String,
    pub rules: Vec<String>,
    /// `warn` or `block`, the highest among `rules`.
    pub severity: String,
    /// Where the text belongs instead (`home`, `personal`), when known.
    #[serde(default)]
    pub belongs: Option<String>,
    #[serde(default)]
    pub actor: Option<String>,
    #[serde(default)]
    pub chat: Option<String>,
    /// The mode that let it through (`flag`).
    pub mode: String,
}

fn clip(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .take(MAX_FIELD_CHARS)
        .collect()
}

/// A rule id as the rules file spells them; anything else is dropped, so a
/// ledger line can never carry text.
fn rule_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 64
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_')
}

fn tidy(mut flag: LeakFlag) -> LeakFlag {
    flag.source = clip(&flag.source);
    flag.layer = clip(&flag.layer);
    flag.path = clip(&flag.path);
    flag.rules.retain(|r| rule_id(r));
    flag.rules.truncate(MAX_RULES);
    flag.belongs = flag.belongs.as_deref().map(clip);
    flag.actor = flag.actor.as_deref().map(clip);
    flag.chat = flag.chat.as_deref().map(clip);
    flag
}

pub(crate) fn record_flag_at(path: &Path, flag: LeakFlag) -> Result<(), String> {
    let line = serde_json::to_string(&tidy(flag)).map_err(|e| e.to_string())?;
    let _held = FLAGS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|e| format!("open the leak flags: {e}"))?;
    file.write_all(format!("{line}\n").as_bytes())
        .map_err(|e| format!("write the leak flags: {e}"))?;
    drop(file);
    trim_flags(path);
    Ok(())
}

/// The ledger stops growing at this size (about 2,000 flags).
const FLAGS_MAX_BYTES: u64 = 512 * 1024;

/// Past [`FLAGS_MAX_BYTES`], keep the newest half, cut at a line start. A
/// failure leaves the ledger as it was; the caller holds [`FLAGS_LOCK`].
fn trim_flags(path: &Path) {
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    if meta.len() <= FLAGS_MAX_BYTES {
        return;
    }
    let Ok(raw) = std::fs::read(path) else {
        return;
    };
    let from = raw.len() - raw.len() / 2;
    let start = raw[from..]
        .iter()
        .position(|b| *b == b'\n')
        .map_or(raw.len(), |i| from + i + 1);
    if let Err(e) = write_atomic(path, &raw[start..], 0o600) {
        eprintln!("[leak-checks] flags not trimmed: {e}");
    }
}

/// Append a flag to the ledger; a failure is logged, never fatal (the write
/// it describes already passed).
pub(crate) fn record_flag(flag: LeakFlag) {
    let result = app_data().and_then(|d| record_flag_at(&d.join(FLAGS_FILE), flag));
    if let Err(e) = result {
        eprintln!("[leak-checks] flag not recorded: {e}");
    }
}

/// The flags of a period, for CX10b's review and the monthly audit.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FlagsReport {
    /// The mode now: `flag` (written and flagged) or `block` (refused).
    pub mode: &'static str,
    pub since: Option<String>,
    pub total: usize,
    pub by_rule: BTreeMap<String, usize>,
    pub by_layer: BTreeMap<String, usize>,
    pub by_source: BTreeMap<String, usize>,
    /// Newest first, at most the limit asked for.
    pub flags: Vec<LeakFlag>,
    /// Lines that didn't read (left out of the counts).
    pub damaged_lines: usize,
}

pub(crate) fn flags_at(
    path: &Path,
    mode: LeakMode,
    since: Option<DateTime<Utc>>,
    limit: usize,
) -> Result<FlagsReport, String> {
    let raw = match std::fs::read(path) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("read the leak flags: {e}")),
    };
    let mut report = FlagsReport {
        mode: mode.as_str(),
        since: since.map(|s| s.to_rfc3339()),
        total: 0,
        by_rule: BTreeMap::new(),
        by_layer: BTreeMap::new(),
        by_source: BTreeMap::new(),
        flags: Vec::new(),
        damaged_lines: 0,
    };
    let mut kept: Vec<LeakFlag> = Vec::new();
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(flag) = serde_json::from_str::<LeakFlag>(line) else {
            report.damaged_lines += 1;
            continue;
        };
        if let Some(since) = since {
            let at = DateTime::parse_from_rfc3339(&flag.at).map(|t| t.with_timezone(&Utc));
            if !at.is_ok_and(|t| t >= since) {
                continue;
            }
        }
        report.total += 1;
        for rule in &flag.rules {
            *report.by_rule.entry(rule.clone()).or_default() += 1;
        }
        *report.by_layer.entry(flag.layer.clone()).or_default() += 1;
        *report.by_source.entry(flag.source.clone()).or_default() += 1;
        kept.push(flag);
    }
    kept.reverse();
    kept.truncate(limit);
    report.flags = kept;
    Ok(report)
}

/// `context.leak_flags`: the flags of the last `days` days (all when `None`).
pub(crate) fn leak_flags(days: Option<u32>, limit: usize) -> Result<FlagsReport, String> {
    let since = days.map(|d| Utc::now() - chrono::Duration::days(i64::from(d)));
    flags_at(&app_data()?.join(FLAGS_FILE), mode(), since, limit)
}

// ---------------------------------------------------------------------------
// Write time
// ---------------------------------------------------------------------------

fn layer_name(layer: WriteLayer) -> &'static str {
    match layer {
        WriteLayer::Home => "home",
        WriteLayer::Personal => "personal",
        WriteLayer::Org => "org",
        WriteLayer::Project => "project",
    }
}

/// The ledger line for a write that went through flagged.
pub(crate) fn write_flag(
    mode: LeakMode,
    layer: WriteLayer,
    path: &str,
    flag: &LayerFlag,
    actor: Option<&str>,
    chat: Option<&str>,
) -> LeakFlag {
    LeakFlag {
        at: Utc::now().to_rfc3339(),
        source: "write".into(),
        layer: layer_name(layer).into(),
        path: path.into(),
        rules: flag.rules.clone(),
        severity: flag.severity.into(),
        belongs: Some(flag.belongs.into()),
        actor: actor.map(str::to_string),
        chat: chat.map(str::to_string),
        mode: mode.as_str().into(),
    }
}

/// `memory_guard::check_layer` in the current [`mode`], recording a flag
/// when a rule matched in flag mode. The one entry point for context writers.
#[cfg_attr(not(test), allow(dead_code))] // Called by the context writers (CX6, CX8) once they land.
pub(crate) fn check_write(
    layer: WriteLayer,
    machine: Option<&str>,
    path: &str,
    fields: &[(&str, &str)],
    actor: Option<&str>,
    chat: Option<&str>,
) -> Result<Verdict, Refusal> {
    let mode = mode();
    let checked = memory_guard::check_layer_as(mode, layer, machine, path, fields)?;
    if let Some(flag) = &checked.flag {
        record_flag(write_flag(mode, layer, path, flag, actor, chat));
    }
    Ok(checked.verdict)
}

/// [`check_write`] with `memory_guard::check_layer_in_root_as`'s link check.
/// Called by `memory_homes::save_home_fact`.
pub(crate) fn check_write_in_root(
    root: &Path,
    layer: WriteLayer,
    machine: Option<&str>,
    path: &str,
    fields: &[(&str, &str)],
    actor: Option<&str>,
    chat: Option<&str>,
) -> Result<Verdict, Refusal> {
    let mode = mode();
    let checked = memory_guard::check_layer_in_root_as(mode, root, layer, machine, path, fields)?;
    if let Some(flag) = &checked.flag {
        record_flag(write_flag(mode, layer, path, flag, actor, chat));
    }
    Ok(checked.verdict)
}

/// Where a rule group's text belongs instead.
fn belongs_of(groups: &[&str]) -> Option<&'static str> {
    if groups.contains(&"machine") {
        Some("home")
    } else if groups.contains(&"personal") {
        Some("personal")
    } else {
        None
    }
}

/// The ledger lines for a finished run's instruction-file flags that carry a
/// scan result (`run_guard`, CX7's scan on `RunFlag::scan`).
pub(crate) fn run_flags(
    vault: &Path,
    source: &str,
    source_id: Option<&str>,
    flags: &[crate::run_guard::RunFlag],
) -> Vec<LeakFlag> {
    use crate::scan_rules::Severity;
    let mut out = Vec::new();
    for flag in flags.iter().filter(|f| f.scan.is_some()) {
        let Some(findings) = crate::run_guard::instruction_findings(vault, &flag.path) else {
            continue;
        };
        let mut rules: Vec<String> = Vec::new();
        let mut groups: Vec<&str> = Vec::new();
        for f in &findings {
            if !rules.iter().any(|r| r == f.rule) {
                rules.push(f.rule.to_string());
            }
            groups.push(f.group);
        }
        rules.sort();
        let block = findings.iter().any(|f| f.severity == Severity::Block);
        out.push(LeakFlag {
            at: Utc::now().to_rfc3339(),
            source: "run".into(),
            layer: "project".into(),
            path: flag.path.clone(),
            rules,
            severity: if block { "block" } else { "warn" }.into(),
            belongs: belongs_of(&groups).map(str::to_string),
            actor: Some(source.to_string()),
            chat: source_id.map(str::to_string),
            mode: "flag".into(),
        });
    }
    out
}

/// Record [`run_flags`] (a run is never refused for them: flag-only).
pub(crate) fn record_run_flags(
    vault: &Path,
    source: &str,
    source_id: Option<&str>,
    flags: &[crate::run_guard::RunFlag],
) {
    for flag in run_flags(vault, source, source_id, flags) {
        record_flag(flag);
    }
}

// ---------------------------------------------------------------------------
// The scanner copy and the pre-push hook
// ---------------------------------------------------------------------------

/// Write the scanner and its rules into `dir` when they differ from the
/// binary's copy; returns the script's path. Each file is replaced in one
/// rename, so a running hook never reads half of one.
pub(crate) fn ensure_scanner_in(dir: &Path) -> Result<PathBuf, String> {
    for (name, text, mode) in [
        ("scan-rules.json", crate::scan_rules::RULES_JSON, 0o644),
        ("ctx-scan", SCANNER_SRC, 0o755),
    ] {
        let path = dir.join(name);
        let same = std::fs::read(&path).is_ok_and(|b| b == text.as_bytes());
        if !same {
            write_atomic(&path, text.as_bytes(), mode)?;
        }
    }
    Ok(dir.join("ctx-scan"))
}

/// The app data copy of the scanner, refreshed from this build.
pub(crate) fn ensure_scanner() -> Result<PathBuf, String> {
    ensure_scanner_in(&app_data()?.join(SCANNER_DIR))
}

fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The hook: run the scanner over what the repo would commit at `layer`, and
/// stop the push on a blocking finding (exit 1) or a scanner configuration
/// error (exit 2, e.g. a bad `.context-scan.yml` or forbidden directive). A
/// missing scanner or unexpected runtime error still lets the push through and
/// says so.
pub(crate) fn hook_script(scanner: &Path, layer: &str) -> String {
    let scanner = sh_quote(&scanner.to_string_lossy());
    let layer = sh_quote(layer);
    format!(
        r#"#!/bin/sh
{HOOK_MARKER}
# Installed by Hollow (Settings, Sources, Context: Install context check).
# A local hook, never committed. Remove it there, or delete this file.
# Runs ctx-scan over what this repo would commit and stops the push on
# a blocking finding (a secret, an SSN shape; data files in org and
# personal repos) or a configuration error (exit 2). Findings name file,
# line and rule, never a value.
scanner={scanner}
layer={layer}
if [ ! -f "$scanner" ]; then
  echo "Hollow context check: the scanner is missing ($scanner); this push was not checked. Install the check again in Hollow." >&2
  exit 0
fi
top=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
cd "$top" || exit 0
{PYTHON} -I "$scanner" --layer "$layer" --fail-on block --quiet .
status=$?
if [ "$status" -eq 1 ]; then
  echo "Hollow context check: blocking findings above, so the push stopped. Fix them, or add 'context-scan: allow <rule> <reason>' on a false alarm, then push again." >&2
  exit 1
fi
if [ "$status" -eq 2 ]; then
  echo "Hollow context check: invalid .context-scan.yml or scanner configuration (exit 2), so the push stopped. Fix the configuration, then push again." >&2
  exit 2
fi
if [ "$status" -ne 0 ]; then
  echo "Hollow context check: the scanner could not run (exit $status); this push was not checked." >&2
fi
exit 0
"#
    )
}

/// `git -C repo <args>` with the repo's own fsmonitor and untracked cache
/// off and no prompt. `keep_hooks_path` leaves `core.hooksPath` as the repo
/// sets it (only the `--git-path hooks/...` question needs its real value);
/// otherwise it is `/dev/null`, so no hook of the repo's runs.
fn git_out_with(repo: &Path, args: &[&str], keep_hooks_path: bool) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.args([
        "-c",
        "core.fsmonitor=false",
        "-c",
        "core.untrackedCache=false",
    ]);
    if !keep_hooks_path {
        cmd.args(["-c", "core.hooksPath=/dev/null"]);
    }
    let out = cmd
        .arg("-C")
        .arg(repo)
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git didn't run: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(clip(err.trim().lines().last().unwrap_or("git failed")));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn git_out(repo: &Path, args: &[&str]) -> Result<String, String> {
    git_out_with(repo, args, false)
}

/// Where a repo's `pre-push` hook goes: the path git gives for
/// `hooks/pre-push` (worktree-aware, `core.hooksPath` included), which must
/// be inside the repo's own git folder, never in the work tree or a shared
/// hooks folder.
pub(crate) fn hook_path(repo: &Path) -> Result<PathBuf, String> {
    let canon =
        std::fs::canonicalize(repo).map_err(|_| "That folder doesn't exist.".to_string())?;
    let top = git_out(&canon, &["rev-parse", "--show-toplevel"])
        .map_err(|_| "That folder isn't a git repo.".to_string())?;
    if std::fs::canonicalize(&top).ok().as_deref() != Some(canon.as_path()) {
        return Err(
            "That folder is inside a git repo, not the top of one. Choose the repo's own folder."
                .into(),
        );
    }
    let common = git_out(
        &canon,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let hook = PathBuf::from(git_out_with(
        &canon,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "hooks/pre-push",
        ],
        true,
    )?);
    let common = std::fs::canonicalize(&common).map_err(|e| format!("the git folder: {e}"))?;
    let dir = hook.parent().ok_or("no hooks folder")?;
    let real_dir = match std::fs::canonicalize(dir) {
        Ok(d) => d,
        Err(_) => dir
            .parent()
            .and_then(|p| std::fs::canonicalize(p).ok())
            .map(|p| p.join(dir.file_name().unwrap_or_default()))
            .ok_or("the hooks folder can't be found")?,
    };
    if !real_dir.starts_with(&common) {
        return Err(format!(
            "This repo's hooks folder is {}, outside its git folder (core.hooksPath or a link). Hollow writes only into the repo's own git folder, so it leaves that alone.",
            clip(&real_dir.to_string_lossy())
        ));
    }
    Ok(real_dir.join("pre-push"))
}

/// What is at a hook path now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HookNow {
    None,
    Hollow,
    Other,
}

pub(crate) fn hook_now(hook: &Path) -> HookNow {
    match std::fs::symlink_metadata(hook) {
        Err(_) => HookNow::None,
        Ok(m) if !m.is_file() => HookNow::Other,
        Ok(_) => {
            let mut head = Vec::new();
            let read = std::fs::File::open(hook).and_then(|f| f.take(4096).read_to_end(&mut head));
            if read.is_ok()
                && String::from_utf8_lossy(&head)
                    .lines()
                    .any(|l| l.trim() == HOOK_MARKER)
            {
                HookNow::Hollow
            } else {
                HookNow::Other
            }
        }
    }
}

/// What [`install_at`] did or would do.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstallOutcome {
    pub applied: bool,
    pub repo: String,
    pub layer: String,
    pub hook: String,
    pub scanner: String,
    /// Where the file it replaced was kept.
    pub backup: Option<String>,
    /// The hook it writes (or would write).
    pub script: String,
    pub text: String,
}

/// Install (or, with `apply` false, describe) the hook in `repo` for
/// `layer`, with the scanner copy in `scanner_dir`.
pub(crate) fn install_at(
    repo: &Path,
    layer: &str,
    scanner_dir: &Path,
    apply: bool,
) -> Result<InstallOutcome, String> {
    if !LAYERS.contains(&layer) {
        return Err(format!("No context layer named {layer:?}."));
    }
    let hook = hook_path(repo)?;
    let now = hook_now(&hook);
    if now == HookNow::Other {
        return Err(format!(
            "This repo already has a pre-push hook of its own ({}). Hollow leaves it as it is; to add the check, call {PYTHON} -I <scanner> --layer {layer} --fail-on block from it.",
            clip(&hook.to_string_lossy())
        ));
    }
    let scanner = scanner_dir.join("ctx-scan");
    let script = hook_script(&scanner, layer);
    let mut outcome = InstallOutcome {
        applied: apply,
        repo: repo.to_string_lossy().into_owned(),
        layer: layer.into(),
        hook: hook.to_string_lossy().into_owned(),
        scanner: scanner.to_string_lossy().into_owned(),
        backup: None,
        script,
        text: String::new(),
    };
    if !apply {
        outcome.text = format!(
            "Would write {} (a local pre-push hook, not committed) checking at the {layer} layer{}. Nothing changed.",
            outcome.hook,
            if now == HookNow::Hollow { ", replacing Hollow's earlier one (kept as a backup)" } else { "" }
        );
        return Ok(outcome);
    }
    ensure_scanner_in(scanner_dir)?;
    if now == HookNow::Hollow {
        let backup = hook.with_file_name(format!(
            "pre-push.hollow-backup-{}-{}",
            Utc::now().format("%Y%m%d%H%M%S"),
            unique_suffix()
        ));
        std::fs::copy(&hook, &backup).map_err(|e| format!("keep a backup: {e}"))?;
        outcome.backup = Some(backup.to_string_lossy().into_owned());
    }
    write_atomic(&hook, outcome.script.as_bytes(), 0o755)?;
    outcome.text = format!(
        "Installed: pushes from this repo now stop on a blocking finding ({} layer). The hook is local and not committed.",
        layer
    );
    Ok(outcome)
}

/// What [`remove_at`] did or would do.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoveOutcome {
    pub applied: bool,
    pub repo: String,
    pub hook: String,
    pub removed: bool,
    pub text: String,
}

/// Remove Hollow's hook from `repo`; a hook without [`HOOK_MARKER`] is
/// never touched.
pub(crate) fn remove_at(repo: &Path, apply: bool) -> Result<RemoveOutcome, String> {
    let hook = hook_path(repo)?;
    let mut outcome = RemoveOutcome {
        applied: apply,
        repo: repo.to_string_lossy().into_owned(),
        hook: hook.to_string_lossy().into_owned(),
        removed: false,
        text: String::new(),
    };
    match hook_now(&hook) {
        HookNow::None => {
            outcome.text = "No context check is installed here; nothing to remove.".into();
            Ok(outcome)
        }
        HookNow::Other => Err(format!(
            "The pre-push hook here ({}) isn't Hollow's; Hollow leaves it as it is.",
            clip(&outcome.hook)
        )),
        HookNow::Hollow if !apply => {
            outcome.text = format!("Would remove {}. Nothing changed.", outcome.hook);
            Ok(outcome)
        }
        HookNow::Hollow => {
            std::fs::remove_file(&hook).map_err(|e| format!("remove the hook: {e}"))?;
            outcome.removed = true;
            outcome.text = "Removed: pushes from this repo are no longer checked.".into();
            Ok(outcome)
        }
    }
}

// ---------------------------------------------------------------------------
// The repos and their roles
// ---------------------------------------------------------------------------

/// A repo a context check can guard, and its state.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CheckTarget {
    pub title: String,
    pub path: String,
    /// `org`, `personal`, `vault` or `project`.
    pub layer: String,
    /// `installed`, `not_installed`, `other_hook` or `unavailable`.
    pub state: String,
    pub problem: Option<String>,
}

/// The repos, by role: the org clone (org), the personal clone (personal),
/// the vault when it is a git repo (vault) and each connected project repo
/// (project). A folder that isn't a git repo is left out.
pub(crate) fn candidates(vault: &Path) -> Vec<(String, PathBuf, &'static str)> {
    use crate::context_roots::{root, Layer};
    let is_git = |p: &Path| std::fs::symlink_metadata(p.join(".git")).is_ok();
    let mut out: Vec<(String, PathBuf, &'static str)> = Vec::new();
    if let Some(p) = root(Layer::Org).filter(|p| is_git(p)) {
        out.push(("Organization".into(), p, "org"));
    }
    if let Some(p) = root(Layer::Personal).filter(|p| is_git(p)) {
        out.push(("Personal".into(), p, "personal"));
    }
    if is_git(vault) {
        out.push(("Vault".into(), vault.to_path_buf(), "vault"));
    }
    if let Ok(c) = crate::directory_connections::status(vault) {
        for p in c.projects.iter().take(PROJECTS_MAX) {
            let path = PathBuf::from(&p.path);
            if is_git(&path) {
                out.push((p.title.clone(), path, "project"));
            }
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    out.retain(|(_, p, _)| seen.insert(std::fs::canonicalize(p).unwrap_or_else(|_| p.clone())));
    out
}

/// The layer a repo is checked at, from its role; `None` for a folder Hollow
/// doesn't know as a context or project repo.
pub(crate) fn role_of(vault: &Path, repo: &Path) -> Option<(String, &'static str)> {
    let want = std::fs::canonicalize(repo).ok()?;
    candidates(vault)
        .into_iter()
        .find(|(_, p, _)| std::fs::canonicalize(p).ok().as_deref() == Some(want.as_path()))
        .map(|(title, _, layer)| (title, layer))
}

/// Every candidate with its hook state (Settings, Context).
pub(crate) fn targets(vault: &Path) -> Vec<CheckTarget> {
    candidates(vault)
        .into_iter()
        .map(|(title, path, layer)| {
            let (state, problem) = match hook_path(&path) {
                Ok(hook) => match hook_now(&hook) {
                    HookNow::Hollow => ("installed", None),
                    HookNow::None => ("not_installed", None),
                    HookNow::Other => (
                        "other_hook",
                        Some("Has a pre-push hook of its own; Hollow leaves it.".to_string()),
                    ),
                },
                Err(e) => ("unavailable", Some(e)),
            };
            CheckTarget {
                title,
                path: path.to_string_lossy().into_owned(),
                layer: layer.into(),
                state: state.into(),
                problem,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The audit
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ScanJson {
    #[serde(default)]
    files: u64,
    #[serde(default)]
    findings: Vec<ScanJsonFinding>,
}

#[derive(Deserialize)]
struct ScanJsonFinding {
    path: String,
    #[serde(default)]
    line: u64,
    rule: String,
    severity: String,
}

/// One finding: file, line and rule, never text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuditFinding {
    pub path: String,
    /// 0 for a whole-file finding (a data file, the size limit).
    pub line: u64,
    pub rule: String,
    pub severity: String,
}

/// One repo (or one layer of the personal clone) in an audit.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuditRepo {
    pub title: String,
    pub layer: String,
    pub path: String,
    pub files: u64,
    pub block: usize,
    pub warn: usize,
    pub findings: Vec<AuditFinding>,
    /// Findings beyond the listed ones.
    pub more: usize,
    pub error: Option<String>,
}

/// `context.audit`'s answer.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuditReport {
    pub at: String,
    pub mode: &'static str,
    pub repos: Vec<AuditRepo>,
    pub block: usize,
    pub warn: usize,
    /// Repos not scanned because the audit's five minutes ran out.
    pub skipped: Vec<String>,
    /// Findings counted but not listed (the per-repo and total caps).
    pub left_out: usize,
    /// The flags of the last 31 days.
    pub flags: FlagsReport,
}

/// Run the scanner in `dir` over `paths` at `layer` (JSON, fail on block).
fn run_scanner(
    scanner: &Path,
    dir: &Path,
    layer: &str,
    paths: &[&str],
    timeout: Duration,
) -> Result<ScanJson, String> {
    let mut child = Command::new(PYTHON)
        .arg("-I")
        .arg(scanner)
        .args(["--layer", layer, "--fail-on", "block", "--format", "json"])
        .args(paths)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        // The scanner's own `git ls-files` runs no fsmonitor of the repo's
        // and never prompts.
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "core.fsmonitor")
        .env("GIT_CONFIG_VALUE_0", "false")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("the scanner didn't start ({PYTHON}): {e}"))?;
    let mut stdout = child.stdout.take().ok_or("no scanner output")?;
    let stderr = child.stderr.take().ok_or("no scanner output")?;
    let out_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let err_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.take(64 * 1024).read_to_end(&mut buf);
        buf
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "the scan took longer than {} s and was stopped",
                    timeout.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(format!("the scanner: {e}")),
        }
    };
    let out = out_reader.join().unwrap_or_default();
    let err = err_reader.join().unwrap_or_default();
    match status.code() {
        Some(0) | Some(1) => serde_json::from_slice::<ScanJson>(&out)
            .map_err(|_| "the scanner's report didn't read".to_string()),
        code => {
            // ctx-scan prints only an error's type, never file text.
            let err = String::from_utf8_lossy(&err);
            Err(format!(
                "the scanner couldn't run (exit {}): {}",
                code.map_or("signal".to_string(), |c| c.to_string()),
                clip(err.trim().lines().last().unwrap_or(""))
            ))
        }
    }
}

/// How much an audit may list and how long it may take.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AuditLimits {
    pub per_scan: Duration,
    pub total: Duration,
    /// Listed findings per repo when more than three scans run.
    pub many_cap: usize,
    pub total_cap: usize,
}

pub(crate) const AUDIT_LIMITS: AuditLimits = AuditLimits {
    per_scan: AUDIT_TIMEOUT,
    total: AUDIT_DEADLINE,
    many_cap: AUDIT_FINDINGS_MANY,
    total_cap: AUDIT_FINDINGS_TOTAL,
};

/// What [`audit_repos`] returns.
#[derive(Debug, Clone)]
pub(crate) struct AuditRun {
    pub repos: Vec<AuditRepo>,
    pub skipped: Vec<String>,
    pub left_out: usize,
}

fn audit_repo(
    title: &str,
    layer: &str,
    path: &Path,
    scan: Result<ScanJson, String>,
    prefix: &str,
    skip_prefix: Option<&str>,
    cap: usize,
) -> AuditRepo {
    let mut repo = AuditRepo {
        title: title.into(),
        layer: layer.into(),
        path: path.to_string_lossy().into_owned(),
        files: 0,
        block: 0,
        warn: 0,
        findings: Vec::new(),
        more: 0,
        error: None,
    };
    match scan {
        Err(e) => repo.error = Some(e),
        Ok(json) => {
            repo.files = json.files;
            // Block findings are listed first (a stable sort keeps the
            // scanner's order otherwise), so a cap never cuts one for warns.
            let mut findings: Vec<ScanJsonFinding> = json
                .findings
                .into_iter()
                .filter(|f| !skip_prefix.is_some_and(|p| f.path.starts_with(p)))
                .collect();
            findings.sort_by_key(|f| f.severity != "block");
            for f in findings {
                if f.severity == "block" {
                    repo.block += 1;
                } else {
                    repo.warn += 1;
                }
                if repo.findings.len() < cap {
                    repo.findings.push(AuditFinding {
                        path: clip(&format!("{prefix}{}", f.path)),
                        line: f.line,
                        rule: if rule_id(&f.rule) {
                            f.rule
                        } else {
                            "unknown".into()
                        },
                        severity: if f.severity == "block" {
                            "block"
                        } else {
                            "warn"
                        }
                        .into(),
                    });
                } else {
                    repo.more += 1;
                }
            }
        }
    }
    repo
}

/// Scan each `(title, path, layer)` with the scanner at `scanner`. The
/// personal clone is two scans: `home/` at the home layer, the rest at
/// personal. Bounded by `limits`: each scan gets at most `per_scan` and what
/// is left of `total`; once that is spent the rest are skipped and named.
pub(crate) fn audit_repos(
    scanner: &Path,
    repos: &[(String, PathBuf, &'static str)],
    limits: AuditLimits,
) -> AuditRun {
    // Each planned scan: (title, layer, path, scan layer, paths, prefix, skip).
    type Plan<'a> = (
        &'a str,
        &'static str,
        &'a Path,
        &'static [&'static str],
        &'static str,
        Option<&'static str>,
    );
    let mut plan: Vec<Plan> = Vec::new();
    for (title, path, layer) in repos {
        if *layer == "personal" {
            if std::fs::symlink_metadata(path.join("home")).is_ok_and(|m| m.is_dir()) {
                plan.push((
                    "Home (personal clone home/)",
                    "home",
                    path,
                    &["home"],
                    "home/",
                    None,
                ));
            }
            plan.push((title, "personal", path, &["."], "", Some("home/")));
        } else {
            plan.push((title, layer, path, &["."], "", None));
        }
    }
    let cap = if plan.len() > 3 {
        limits.many_cap.min(AUDIT_FINDINGS_MAX)
    } else {
        AUDIT_FINDINGS_MAX
    };
    let started = Instant::now();
    let mut run = AuditRun {
        repos: Vec::new(),
        skipped: Vec::new(),
        left_out: 0,
    };
    let mut listed = 0usize;
    for (title, layer, path, paths, prefix, skip) in plan {
        let left = limits.total.saturating_sub(started.elapsed());
        if left.is_zero() {
            run.skipped.push(format!("{title} ({layer})"));
            continue;
        }
        let scan = run_scanner(scanner, path, layer, paths, limits.per_scan.min(left));
        let room = cap.min(limits.total_cap.saturating_sub(listed));
        let repo = audit_repo(title, layer, path, scan, prefix, skip, room);
        listed += repo.findings.len();
        run.left_out += repo.more;
        run.repos.push(repo);
    }
    run
}

/// `context.audit`: every context and project repo, plus the last 31 days
/// of flags. Five minutes at most; see [`AuditLimits`].
pub(crate) fn audit(vault: &Path) -> Result<AuditReport, String> {
    let scanner = ensure_scanner()?;
    let run = audit_repos(&scanner, &candidates(vault), AUDIT_LIMITS);
    let block = run.repos.iter().map(|r| r.block).sum();
    let warn = run.repos.iter().map(|r| r.warn).sum();
    Ok(AuditReport {
        at: Utc::now().to_rfc3339(),
        mode: mode().as_str(),
        repos: run.repos,
        block,
        warn,
        skipped: run.skipped,
        left_out: run.left_out,
        flags: leak_flags(Some(31), 200)?,
    })
}

// ---------------------------------------------------------------------------
// Tauri commands (Settings, Context)
// ---------------------------------------------------------------------------

/// Settings, Context: the repos a context check can guard, the mode.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CheckStatus {
    pub mode: &'static str,
    pub repos: Vec<CheckTarget>,
}

pub(crate) fn check_status(vault: &Path) -> CheckStatus {
    CheckStatus {
        mode: mode().as_str(),
        repos: targets(vault),
    }
}

/// Install the check in a known repo at its role's layer.
pub(crate) fn install_check(
    vault: &Path,
    repo: &str,
    apply: bool,
) -> Result<InstallOutcome, String> {
    let path = PathBuf::from(repo.trim());
    let Some((_, layer)) = role_of(vault, &path) else {
        return Err("That folder isn't one of Hollow's context or project repos (Settings, Context lists them). Nothing changed.".into());
    };
    install_at(&path, layer, &app_data()?.join(SCANNER_DIR), apply)
}

/// Remove Hollow's check from a repo.
pub(crate) fn remove_check(repo: &str, apply: bool) -> Result<RemoveOutcome, String> {
    remove_at(Path::new(repo.trim()), apply)
}

#[tauri::command]
pub async fn context_check_status(
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<CheckStatus, String> {
    let vault = state.config.vault_path.clone();
    tauri::async_runtime::spawn_blocking(move || check_status(&vault))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn context_install_check(
    state: tauri::State<'_, crate::commands::AppState>,
    path: String,
    apply: Option<bool>,
) -> Result<InstallOutcome, String> {
    let vault = state.config.vault_path.clone();
    let apply = apply.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || install_check(&vault, &path, apply))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn context_remove_check(
    path: String,
    apply: Option<bool>,
) -> Result<RemoveOutcome, String> {
    let apply = apply.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || remove_check(&path, apply))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .expect("git runs")
            .status
            .success();
        assert!(ok, "git {args:?}");
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::Builder::new()
            .prefix("leak-repo-")
            .tempdir()
            .unwrap();
        git(dir.path(), &["init", "-q"]);
        std::fs::write(
            dir.path().join("README.md"),
            "Run `npm test` before a PR.\n",
        )
        .unwrap();
        dir
    }

    fn fake_key() -> String {
        // Built at run time: no key-shaped string sits in the repo.
        ["sk-", "ant-api03-", &"Q7x".repeat(10)].concat()
    }

    fn run_hook(hook: &Path, cwd: &Path) -> i32 {
        Command::new("/bin/sh")
            .arg(hook)
            .current_dir(cwd)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .stdin(Stdio::null())
            .output()
            .expect("the hook runs")
            .status
            .code()
            .unwrap_or(-1)
    }

    #[test]
    fn the_mode_defaults_to_flag_and_is_one_saved_switch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        assert_eq!(DEFAULT_MODE, LeakMode::Flag);
        assert_eq!(mode_at(&path), LeakMode::Flag);
        assert_eq!(set_mode_at(&path, "block"), Ok(LeakMode::Block));
        assert_eq!(mode_at(&path), LeakMode::Block);
        assert!(set_mode_at(&path, "loud").is_err());
        assert_eq!(mode_at(&path), LeakMode::Block);
        // A file that names no mode is the default; one that doesn't read
        // is the safer side.
        std::fs::write(&path, "{}").unwrap();
        assert_eq!(mode_at(&path), DEFAULT_MODE);
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(mode_at(&path), LeakMode::Block);
    }

    #[test]
    fn the_flags_ledger_is_capped_and_keeps_the_newest_whole_lines() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = dir.path().join(FLAGS_FILE);
        let flag = |n: usize| LeakFlag {
            at: Utc::now().to_rfc3339(),
            source: "write".into(),
            layer: "project".into(),
            path: format!("docs/{n}.md"),
            rules: vec!["machine.macbook".into()],
            severity: "warn".into(),
            belongs: None,
            actor: None,
            chat: None,
            mode: "flag".into(),
        };
        // About 250 bytes a line: 3,000 flags pass the cap more than once.
        let n = 3_000;
        for i in 0..=n {
            record_flag_at(&ledger, flag(i)).unwrap();
        }
        let len = std::fs::metadata(&ledger).unwrap().len();
        assert!(len <= FLAGS_MAX_BYTES, "capped: {len}");
        let report = flags_at(&ledger, LeakMode::Flag, None, 5).unwrap();
        assert_eq!(report.damaged_lines, 0, "cut at a line start");
        assert_eq!(report.flags[0].path, format!("docs/{n}.md"), "newest kept");
    }

    #[test]
    fn a_flag_is_recorded_without_the_text_and_counted() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = dir.path().join(FLAGS_FILE);
        let text = "Builds run on the MacBook with 18 cores.";
        let checked = memory_guard::check_layer_as(
            LeakMode::Flag,
            WriteLayer::Project,
            Some("m"),
            "docs/CLAUDE.md",
            &[("text", text)],
        )
        .unwrap();
        let flag = checked.flag.expect("flagged");
        record_flag_at(
            &ledger,
            write_flag(
                LeakMode::Flag,
                WriteLayer::Project,
                "docs/CLAUDE.md",
                &flag,
                Some("agent"),
                Some("s-1"),
            ),
        )
        .unwrap();
        // A forged rule id with text in it never reaches the ledger.
        let mut forged = write_flag(
            LeakMode::Flag,
            WriteLayer::Project,
            "x.md",
            &flag,
            None,
            None,
        );
        forged.rules.push("the MacBook with 18 cores".into());
        record_flag_at(&ledger, forged).unwrap();
        let raw = std::fs::read_to_string(&ledger).unwrap();
        assert!(!raw.contains("18 cores"), "{raw}");
        assert!(raw.contains("machine.macbook"), "{raw}");
        let report = flags_at(&ledger, LeakMode::Flag, None, 10).unwrap();
        assert_eq!(report.total, 2);
        assert_eq!(report.by_layer.get("project"), Some(&2));
        assert_eq!(report.flags[0].path, "x.md", "newest first");
        assert_eq!(report.flags[1].belongs.as_deref(), Some("home"));
        assert_eq!(report.flags[1].chat.as_deref(), Some("s-1"));
        let later = flags_at(
            &ledger,
            LeakMode::Flag,
            Some(Utc::now() + chrono::Duration::days(1)),
            10,
        )
        .unwrap();
        assert_eq!(later.total, 0);
    }

    #[test]
    fn check_write_flags_by_default_and_refuses_once_block_is_saved() {
        // The test process's app data folder: unset means DEFAULT_MODE (flag).
        let data = app_data().unwrap();
        let _ = std::fs::remove_file(data.join(SETTINGS_FILE));
        let ledger = data.join(FLAGS_FILE);
        let before = flags_at(&ledger, LeakMode::Flag, None, 0).unwrap().total;
        let fields: &[(&str, &str)] = &[("text", "Builds run on the Mac mini.")];
        assert_eq!(
            check_write(
                WriteLayer::Project,
                Some("m"),
                "CLAUDE.md",
                fields,
                Some("agent"),
                Some("s-9")
            ),
            Ok(Verdict::Write)
        );
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            check_write_in_root(
                root.path(),
                WriteLayer::Project,
                Some("m"),
                "docs/a.md",
                fields,
                None,
                None
            ),
            Ok(Verdict::Write)
        );
        let after = flags_at(&ledger, LeakMode::Flag, None, 10).unwrap();
        assert!(after.total >= before + 2);
        assert!(after
            .flags
            .iter()
            .any(|f| f.path == "CLAUDE.md" && f.chat.as_deref() == Some("s-9")));
        let key = fake_key();
        assert!(check_write(
            WriteLayer::Project,
            Some("m"),
            "CLAUDE.md",
            &[("text", &key)],
            None,
            None
        )
        .is_err());
        // Saving `block` flips the same call to a refusal naming Home.
        assert_eq!(set_mode("block"), Ok(LeakMode::Block));
        let refused = check_write(
            WriteLayer::Project,
            Some("m"),
            "CLAUDE.md",
            fields,
            None,
            None,
        );
        let _ = std::fs::remove_file(data.join(SETTINGS_FILE));
        let msg = refused.expect_err("refused in block mode").message();
        assert!(msg.contains("home/m.md"), "{msg}");
    }

    #[test]
    fn the_hook_stops_a_push_only_on_a_blocking_finding() {
        let scanner_dir = tempfile::tempdir().unwrap();
        let r = repo();
        let out = install_at(r.path(), "project", scanner_dir.path(), false).unwrap();
        assert!(!out.applied);
        assert!(!Path::new(&out.hook).exists(), "a dry run writes nothing");
        let out = install_at(r.path(), "project", scanner_dir.path(), true).unwrap();
        let hook = PathBuf::from(&out.hook);
        assert!(hook.starts_with(std::fs::canonicalize(r.path().join(".git")).unwrap()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&hook).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "executable");
        }
        // Clean repo, and a machine fact (a warning in a project repo): passes.
        assert_eq!(run_hook(&hook, r.path()), 0);
        std::fs::write(r.path().join("notes.md"), "Runs on the MacBook.\n").unwrap();
        assert_eq!(run_hook(&hook, r.path()), 0);
        // A planted secret: the push stops.
        std::fs::write(r.path().join("setup.md"), format!("key: {}\n", fake_key())).unwrap();
        assert_eq!(run_hook(&hook, r.path()), 1);
        std::fs::remove_file(r.path().join("setup.md")).unwrap();
        assert_eq!(run_hook(&hook, r.path()), 0);
        // Installing again replaces Hollow's own hook and keeps a backup.
        let again = install_at(r.path(), "project", scanner_dir.path(), true).unwrap();
        assert!(again
            .backup
            .as_deref()
            .is_some_and(|b| Path::new(b).exists()));
        // Remove takes only Hollow's hook.
        assert!(remove_at(r.path(), false)
            .unwrap()
            .text
            .contains("Would remove"));
        assert!(hook.exists());
        assert!(remove_at(r.path(), true).unwrap().removed);
        assert!(!hook.exists());
    }

    #[test]
    fn an_org_hook_blocks_machine_facts_and_a_missing_scanner_lets_the_push_through() {
        let scanner_dir = tempfile::tempdir().unwrap();
        let r = repo();
        let out = install_at(r.path(), "org", scanner_dir.path(), true).unwrap();
        let hook = PathBuf::from(&out.hook);
        std::fs::write(r.path().join("notes.md"), "Runs on the MacBook.\n").unwrap();
        assert_eq!(run_hook(&hook, r.path()), 1, "a machine fact blocks in org");
        std::fs::remove_file(scanner_dir.path().join("ctx-scan")).unwrap();
        assert_eq!(
            run_hook(&hook, r.path()),
            0,
            "no scanner: not checked, not stopped"
        );
    }

    #[test]
    fn a_scanner_config_error_fails_closed_and_stops_the_push() {
        let scanner_dir = tempfile::tempdir().unwrap();
        let r = repo();
        let out = install_at(r.path(), "project", scanner_dir.path(), true).unwrap();
        let hook = PathBuf::from(&out.hook);
        std::fs::write(r.path().join("setup.md"), format!("key: {}\n", fake_key())).unwrap();
        assert_eq!(run_hook(&hook, r.path()), 1);

        // Forbidden directive in .context-scan.yml: ctx-scan exits 2, hook fails closed.
        std::fs::write(r.path().join(".context-scan.yml"), "disable:\n  - secret\n").unwrap();
        assert_eq!(
            run_hook(&hook, r.path()),
            2,
            "forbidden .context-scan.yml directive must stop the push"
        );

        // Malformed .context-scan.yml: ctx-scan exits 2, hook fails closed.
        std::fs::write(r.path().join(".context-scan.yml"), "not valid yaml line\n").unwrap();
        assert_eq!(
            run_hook(&hook, r.path()),
            2,
            "malformed .context-scan.yml must stop the push"
        );
        std::fs::remove_file(r.path().join(".context-scan.yml")).unwrap();

        // Damaged rules file: ctx-scan exits 2, hook fails closed.
        std::fs::write(scanner_dir.path().join("scan-rules.json"), "not json").unwrap();
        assert_eq!(
            run_hook(&hook, r.path()),
            2,
            "damaged rules file (exit 2) must stop the push"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_hooks_folder_that_leaves_the_git_folder_is_refused() {
        let r = repo();
        let outside = tempfile::tempdir().unwrap();
        let hooks = r.path().join(".git/hooks");
        let _ = std::fs::remove_dir_all(&hooks);
        std::os::unix::fs::symlink(outside.path(), &hooks).unwrap();
        let err = hook_path(r.path()).unwrap_err();
        assert!(err.contains("outside its git folder"), "{err}");
        let scanner_dir = tempfile::tempdir().unwrap();
        assert!(install_at(r.path(), "project", scanner_dir.path(), true).is_err());
        assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none());
    }

    #[test]
    fn the_audit_stops_at_its_deadline_and_caps_what_it_lists() {
        let scanner_dir = tempfile::tempdir().unwrap();
        let scanner = ensure_scanner_in(scanner_dir.path()).unwrap();
        let mut repos = Vec::new();
        let mut keep = Vec::new();
        for i in 0..4 {
            let r = repo();
            let lines: String = (0..30)
                .map(|n| format!("Runs on the MacBook {n}.\n"))
                .collect();
            std::fs::write(r.path().join("notes.md"), lines).unwrap();
            repos.push((format!("r{i}"), r.path().to_path_buf(), "project"));
            keep.push(r);
        }
        // Four scans: 25 listed each, the total cap stops the rest.
        let limits = AuditLimits {
            total_cap: 60,
            ..AUDIT_LIMITS
        };
        let run = audit_repos(&scanner, &repos, limits);
        assert!(run.skipped.is_empty());
        assert_eq!(run.repos[0].findings.len(), 25);
        assert_eq!(run.repos[0].warn, 30);
        let listed: usize = run.repos.iter().map(|r| r.findings.len()).sum();
        assert_eq!(listed, 60);
        assert_eq!(run.left_out, 120 - 60);
        // No time left: every repo is named as skipped, none scanned.
        let none = audit_repos(
            &scanner,
            &repos,
            AuditLimits {
                total: Duration::ZERO,
                ..AUDIT_LIMITS
            },
        );
        assert!(none.repos.is_empty());
        assert_eq!(none.skipped.len(), 4);
    }

    #[test]
    fn a_cap_never_cuts_a_block_finding_for_warns() {
        let warn = |n: u64| ScanJsonFinding {
            path: "notes.md".into(),
            line: n,
            rule: "machine.macbook".into(),
            severity: "warn".into(),
        };
        let mut findings: Vec<ScanJsonFinding> = (1..=30).map(warn).collect();
        findings.push(ScanJsonFinding {
            path: "setup.md".into(),
            line: 1,
            rule: "secret.anthropic".into(),
            severity: "block".into(),
        });
        let scan = Ok(ScanJson { files: 2, findings });
        let repo = audit_repo("r", "project", Path::new("/r"), scan, "", None, 25);
        assert_eq!((repo.block, repo.warn, repo.more), (1, 30, 6));
        assert_eq!(repo.findings.len(), 25);
        assert_eq!(repo.findings[0].rule, "secret.anthropic");
        assert_eq!(repo.findings[1].line, 1, "warns keep their order");
    }

    #[test]
    fn a_hook_that_is_not_hollows_is_never_replaced_or_removed() {
        let scanner_dir = tempfile::tempdir().unwrap();
        let r = repo();
        let hook = hook_path(r.path()).unwrap();
        std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
        std::fs::write(&hook, "#!/bin/sh\nexit 0\n").unwrap();
        assert!(install_at(r.path(), "project", scanner_dir.path(), true).is_err());
        assert!(remove_at(r.path(), true).is_err());
        assert_eq!(
            std::fs::read_to_string(&hook).unwrap(),
            "#!/bin/sh\nexit 0\n"
        );
        assert!(install_at(r.path(), "nowhere", scanner_dir.path(), false).is_err());
    }

    #[test]
    fn a_hooks_path_outside_the_git_folder_is_refused() {
        let r = repo();
        git(r.path(), &["config", "core.hooksPath", ".githooks"]);
        let err = hook_path(r.path()).unwrap_err();
        assert!(err.contains("core.hooksPath"), "{err}");
        // A subfolder isn't a repo top.
        let fresh = repo();
        std::fs::create_dir_all(fresh.path().join("docs")).unwrap();
        assert!(hook_path(&fresh.path().join("docs")).is_err());
    }

    #[test]
    fn the_audit_reports_rules_and_lines_never_values() {
        let scanner_dir = tempfile::tempdir().unwrap();
        let scanner = ensure_scanner_in(scanner_dir.path()).unwrap();
        let project = repo();
        std::fs::write(project.path().join("notes.md"), "Runs on the MacBook.\n").unwrap();
        std::fs::write(
            project.path().join("setup.md"),
            format!("key: {}\n", fake_key()),
        )
        .unwrap();
        let personal = repo();
        std::fs::create_dir_all(personal.path().join("home")).unwrap();
        // Acme facts and a machine fact are allowed in the personal clone.
        std::fs::write(
            personal.path().join("memory.md"),
            "Pay runs through Acme payroll on the MacBook.\n",
        )
        .unwrap();
        std::fs::write(
            personal.path().join("home/m.md"),
            "18 cores on the MacBook.\n",
        )
        .unwrap();
        std::fs::write(personal.path().join("roster.csv"), "a,b\n").unwrap();
        let repos = vec![
            ("p".to_string(), project.path().to_path_buf(), "project"),
            (
                "Personal".to_string(),
                personal.path().to_path_buf(),
                "personal",
            ),
        ];
        let report = audit_repos(&scanner, &repos, AUDIT_LIMITS).repos;
        let text = serde_json::to_string(&report).unwrap();
        assert!(!text.contains(&fake_key()), "never a value");
        let p = &report[0];
        assert!(p.error.is_none(), "{:?}", p.error);
        assert!(p.findings.iter().any(|f| f.rule.starts_with("secret.")
            && f.severity == "block"
            && f.path == "setup.md"
            && f.line == 1));
        assert!(p
            .findings
            .iter()
            .any(|f| f.rule == "machine.macbook" && f.severity == "warn"));
        let home = report.iter().find(|r| r.layer == "home").expect("home row");
        assert!(home.findings.is_empty(), "{:?}", home.findings);
        let pers = report
            .iter()
            .find(|r| r.layer == "personal")
            .expect("personal row");
        assert!(pers.findings.iter().all(|f| !f.path.starts_with("home/")));
        assert!(!pers
            .findings
            .iter()
            .any(|f| f.rule.starts_with("machine.") || f.rule.starts_with("pii.")));
        assert!(pers
            .findings
            .iter()
            .any(|f| f.rule == "file.data_type" && f.path == "roster.csv"));
    }
}
