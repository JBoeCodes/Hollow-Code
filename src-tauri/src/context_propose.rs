//! `context.propose_org_change` (context layers CX8): the only way an agent
//! changes the organization layer (`example-org/org-context`).
//!
//! The org clone (`~/Hollow/Context/org`) is read only. A change goes: scan
//! every file with the shared scanner (CX1 `scan_rules` for the `org` layer
//! plus `memory_guard::check`), then, in a **throwaway clone** of the org
//! repo's remote, branch `hollow/<slug>-<id>` from the default branch, write
//! the files, one commit (`memory_guard::sync_commit_message`: actor chats
//! and files), push, and `gh pr create` with the repo's pull request template
//! filled in and the scan output in the body.
//!
//! **Who the actor is (plan decision D4).** The authenticated `gh` login
//! (`gh api user --jq .login`) is compared with the owner named in the
//! clone's `CODEOWNERS` (never hardcoded). Only when the login is the owner
//! **and** the call is from a chat (not an automation) is the PR "owner":
//! [`merge_own`], a separate step, then merges it with `gh pr merge --squash`
//! once the PR is open, not a draft, mergeable, and every check has passed
//! (none failing or pending). Every other identity leaves the PR open,
//! labelled "needs owner approval", and is not listed for the digest.
//!
//! **Order of effects.** Validation and the scan run before any `git` or `gh`
//! call; a refusal names the rule id, file and line, never the text.
//!
//! **The record.** Each opened PR appends one line to `context-org-prs.jsonl`
//! in Hollow's app data (url, title, branch, login, identity, status, reason,
//! chat, time, files, `digest`): the input of CX8b's daily digest
//! (`context_org_digest.rs`). `digest` is true for an owner PR (merged or still
//! waiting on checks) and false for a teammate's.
//!
//! Every process goes through [`Runner`], so tests drive a scratch bare repo
//! as `origin` and a fake `gh`, and never touch the network.

use crate::memory_guard;
use crate::scan_rules::{self, ScanOptions, Severity};
use serde::Serialize;
use serde_json::{json, Value};
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// Folders an org change may write in (the repo layout; `tools/` and
/// `.github/` are code, not context).
pub(crate) const ORG_FOLDERS: &[&str] = &[
    "ai-tools",
    "brand",
    "business-rules",
    "data-sources",
    "goals",
    "initiatives",
    "locations",
    "people",
    "processes",
    "projects",
    "stakeholders",
    "standards",
    "systems",
    "templates",
    "vendors",
];
/// Files an org change may write at the repo's top level.
pub(crate) const ORG_ROOT_FILES: &[&str] = &["glossary.md", "INDEX.md", "GUIDE.md"];
/// The always-on index's hard cap (plan section 3).
pub(crate) const INDEX_CAP: usize = 2048;
const MAX_FILES: usize = 16;
const MAX_FILE_BYTES: usize = 32 * 1024;
const MAX_TOTAL_BYTES: usize = 128 * 1024;
const MAX_TITLE_CHARS: usize = 120;
const MAX_REASON_CHARS: usize = 800;
const LABEL_WAITING: &str = "needs owner approval";
/// The record's file name inside Hollow's app data.
pub(crate) const LEDGER_FILE: &str = "context-org-prs.jsonl";

// ---------------------------------------------------------------------------
// The injectable runner
// ---------------------------------------------------------------------------

/// One process to run.
#[derive(Debug, Clone)]
pub(crate) struct Cmd {
    pub program: &'static str,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub stdin: Option<String>,
}

/// What a process printed and how it ended.
#[derive(Debug, Clone)]
pub(crate) struct Out {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Runs `git` and `gh`. The real one is [`SystemRunner`]; tests supply their
/// own.
pub(crate) trait Runner {
    fn run(&self, cmd: &Cmd) -> Result<Out, String>;
}

/// The real runner: no prompts, a timeout, its own process group.
pub(crate) struct SystemRunner;

const RUN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
/// How long to wait for a finished process's output after it exits.
const READ_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
/// Tries of the owner merge's readiness check, and the pause between them.
const MERGE_TRIES: usize = 6;
const MERGE_PAUSE: std::time::Duration = std::time::Duration::from_secs(10);

impl Runner for SystemRunner {
    fn run(&self, cmd: &Cmd) -> Result<Out, String> {
        use std::io::Read;
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};
        let mut command = Command::new(cmd.program);
        command
            .args(&cmd.args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes")
            .env("GH_PROMPT_DISABLED", "1")
            .env_remove("GIT_ASKPASS")
            .env_remove("SSH_ASKPASS")
            .stdin(if cmd.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        if let Some(cwd) = &cmd.cwd {
            command.current_dir(cwd);
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("{} did not start: {e}", cmd.program))?;
        if let (Some(text), Some(mut stdin)) = (cmd.stdin.clone(), child.stdin.take()) {
            std::thread::spawn(move || {
                let _ = stdin.write_all(text.as_bytes());
            });
        }
        let mut out = child.stdout.take();
        let mut err = child.stderr.take();
        // Readers report over a channel, so a pipe a grandchild still holds
        // open can't block the join: the wait below is bounded.
        let (out_tx, out_rx) = std::sync::mpsc::channel::<String>();
        let (err_tx, err_rx) = std::sync::mpsc::channel::<String>();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(p) = out.as_mut() {
                let _ = p.read_to_end(&mut bytes);
            }
            let _ = out_tx.send(String::from_utf8_lossy(&bytes).into_owned());
        });
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(p) = err.as_mut() {
                let _ = p.read_to_end(&mut bytes);
            }
            let _ = err_tx.send(String::from_utf8_lossy(&bytes).into_owned());
        });
        let started = std::time::Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() < RUN_TIMEOUT => {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                _ => {
                    if let Ok(pid) = i32::try_from(child.id()) {
                        // SAFETY: killpg only delivers a signal, to the group
                        // this process leads (spawned with process_group(0)).
                        unsafe {
                            libc::killpg(pid, libc::SIGKILL);
                        }
                    }
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("{} timed out", cmd.program));
                }
            }
        };
        Ok(Out {
            ok: status.success(),
            stdout: out_rx.recv_timeout(READ_WAIT).unwrap_or_default(),
            stderr: err_rx.recv_timeout(READ_WAIT).unwrap_or_default(),
        })
    }
}

fn git(dir: &Path, args: &[&str]) -> Cmd {
    Cmd {
        program: "git",
        args: args.iter().map(|a| a.to_string()).collect(),
        cwd: Some(dir.to_path_buf()),
        stdin: None,
    }
}

fn gh(dir: &Path, args: &[&str], stdin: Option<String>) -> Cmd {
    Cmd {
        program: "gh",
        args: args.iter().map(|a| a.to_string()).collect(),
        cwd: Some(dir.to_path_buf()),
        stdin,
    }
}

/// Runs `cmd`; a failure becomes one sentence naming the step, with the
/// tool's own last message line clipped (a tool's refusal, not an echo of the
/// files' text).
fn must(runner: &dyn Runner, step: &str, cmd: Cmd) -> Result<Out, String> {
    let out = runner.run(&cmd)?;
    if out.ok {
        Ok(out)
    } else {
        let msg: String = out
            .stderr
            .trim()
            .lines()
            .last()
            .unwrap_or("no message")
            .chars()
            .take(300)
            .collect();
        Err(format!("{step} failed: {msg}"))
    }
}

// ---------------------------------------------------------------------------
// The request
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileChange {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub(crate) struct Request {
    pub files: Vec<FileChange>,
    pub title: String,
    pub reason: String,
    /// The chat the change came from (named in the commit and the record).
    pub chat: Option<String>,
    /// The call is from an attended chat: a chat (not an automation), depth 0
    /// and not an unattended run. A precondition of every merge by Hollow.
    pub attended: bool,
}

/// Everything the run touches outside the request, injectable for tests.
pub(crate) struct Env<'a> {
    pub runner: &'a dyn Runner,
    /// The read-only org clone: only its `origin` URL is read.
    pub org_root: &'a Path,
    /// Where the throwaway clone is made.
    pub work_dir: &'a Path,
    /// The append-only record.
    pub ledger: &'a Path,
    /// Pauses between merge-readiness tries (`thread::sleep`; instant in tests).
    pub sleep: &'a dyn Fn(std::time::Duration),
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Outcome {
    pub url: String,
    pub branch: String,
    pub login: String,
    /// `owner` or `teammate`.
    pub identity: String,
    /// `merged` or `waiting`.
    pub status: String,
    /// Why it waits (empty when merged).
    pub reason: String,
    /// Scan findings that only warn (severity, rule, file, line).
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Validation and the scan
// ---------------------------------------------------------------------------

/// `rel` as a plain path under an allowed org folder, `.md` or `.yml` only.
pub(crate) fn validate_path(rel: &str) -> Result<(), String> {
    let bad = |why: &str| Err(format!("{:?} is refused: {why}.", clip(rel, 80)));
    if rel.is_empty() || rel.len() > 200 {
        return bad("the path is empty or too long");
    }
    if rel.starts_with('/') || rel.contains('\\') || rel.contains('\0') || rel.contains(':') {
        return bad("it must be a plain path relative to the org repo");
    }
    let parts: Vec<&str> = rel.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == "." || *p == ".." || p.starts_with('.'))
    {
        return bad("no empty, dot or .. parts");
    }
    if parts.iter().any(|p| {
        !p.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_. ".contains(c))
    }) {
        return bad("names are letters, digits, space, - _ and . only");
    }
    let lower = rel.to_ascii_lowercase();
    if !(lower.ends_with(".md") || lower.ends_with(".yml")) {
        return bad("only .md and .yml files");
    }
    let allowed = if parts.len() == 1 {
        ORG_ROOT_FILES.contains(&rel)
    } else {
        ORG_FOLDERS.contains(&parts[0])
    };
    if !allowed {
        return bad("not an org folder or top-level file (see the repo's layout)");
    }
    Ok(())
}

fn clip(s: &str, n: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(n).collect()
}

/// The checks that need no process. `Err` is the refusal sentence.
pub(crate) fn validate(req: &Request) -> Result<(), String> {
    let title = req.title.trim();
    if title.is_empty() || title.chars().count() > MAX_TITLE_CHARS || title.contains('\n') {
        return Err(format!(
            "The title must be one line of 1 to {MAX_TITLE_CHARS} characters."
        ));
    }
    if req.reason.trim().is_empty() || req.reason.chars().count() > MAX_REASON_CHARS {
        return Err(format!(
            "The reason must be 1 to {MAX_REASON_CHARS} characters: why this is true for everyone in your organization, and where it was learned."
        ));
    }
    if req.files.is_empty() || req.files.len() > MAX_FILES {
        return Err(format!("Send 1 to {MAX_FILES} files."));
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut total = 0usize;
    for f in &req.files {
        validate_path(&f.path)?;
        if !seen.insert(f.path.to_ascii_lowercase()) {
            return Err(format!("{:?} is listed twice.", clip(&f.path, 80)));
        }
        if f.content.contains('\0') {
            return Err(format!("{:?} has a NUL byte.", clip(&f.path, 80)));
        }
        if f.content.trim().is_empty() {
            return Err(format!("{:?} is empty.", clip(&f.path, 80)));
        }
        if f.content.len() > MAX_FILE_BYTES {
            return Err(format!(
                "{:?} is over {} KB: short facts only, long documents stay out of this repo.",
                clip(&f.path, 80),
                MAX_FILE_BYTES / 1024
            ));
        }
        total += f.content.len();
        // The file is written with a final newline, so count it.
        let written = f.content.len() + usize::from(!f.content.ends_with('\n'));
        if f.path == "INDEX.md" && written >= INDEX_CAP {
            return Err(format!(
                "INDEX.md would be {written} bytes; it stays under {INDEX_CAP} bytes (one line per file)."
            ));
        }
    }
    if total > MAX_TOTAL_BYTES {
        return Err(format!(
            "Together the files are over {} KB.",
            MAX_TOTAL_BYTES / 1024
        ));
    }
    Ok(())
}

/// The scan of every file and the PR text. `Err` names rules, files and
/// lines for a block finding; `Ok` carries the report lines (warnings too).
pub(crate) fn scan(req: &Request) -> Result<Vec<String>, String> {
    let opts = ScanOptions {
        layer: "org",
        groups: None,
        // Text an agent is about to publish must not allow itself.
        honor_allows: false,
    };
    let mut blocks: Vec<String> = Vec::new();
    let mut report: Vec<String> = Vec::new();
    let mut note = |name: &str, f: &scan_rules::Finding| {
        let at = if f.line > 0 {
            format!("{name}:{}:{}", f.line, f.column)
        } else {
            name.to_string()
        };
        let sev = if f.severity == Severity::Block {
            "block"
        } else {
            "warn"
        };
        let line = format!("{sev} {} {at}", f.rule);
        if f.severity == Severity::Block {
            blocks.push(line.clone());
        }
        report.push(line);
    };
    for f in &req.files {
        for finding in scan_rules::scan_path(&f.path, opts) {
            note(&f.path, &finding);
        }
        for finding in scan_rules::scan_text(&f.content, opts) {
            note(&f.path, &finding);
        }
    }
    for (name, text) in [
        ("title", req.title.as_str()),
        ("reason", req.reason.as_str()),
    ] {
        for finding in scan_rules::scan_text(text, opts) {
            note(name, &finding);
        }
    }
    if !blocks.is_empty() {
        return Err(format!(
            "The scan refused this change: {}. Nothing was pushed. Reword it without that and try again.",
            blocks.join("; ")
        ));
    }
    // Hollow's memory scan (injection phrases, stored secrets, invisible
    // characters): the text will be read as context by every agent.
    let mut fields: Vec<(&str, &str)> = req
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.content.as_str()))
        .collect();
    fields.push(("title", &req.title));
    fields.push(("reason", &req.reason));
    memory_guard::check(&fields).map_err(|r| {
        r.message()
            .replace("Nothing was saved.", "Nothing was pushed.")
    })?;
    if report.is_empty() {
        report.push("no findings".into());
    }
    Ok(report)
}

// ---------------------------------------------------------------------------
// CODEOWNERS and the PR body
// ---------------------------------------------------------------------------

/// The owner a `CODEOWNERS` text names for `*`: GitHub applies the **last**
/// matching line, so the last `*` line decides. `None` when there is none, or
/// when it names more than one owner (then nobody is "the owner").
pub(crate) fn codeowner(text: &str) -> Option<String> {
    let last = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .rfind(|l| l.split_whitespace().next() == Some("*"))?;
    let owners: Vec<&str> = last.split_whitespace().skip(1).collect();
    match owners.as_slice() {
        [one] => one.strip_prefix('@').map(str::to_string),
        _ => None,
    }
}

/// The owner from the clone at `dir`: `.github/CODEOWNERS`, then
/// `CODEOWNERS`, then `docs/CODEOWNERS` (GitHub's own order; the first file
/// that exists is the one it reads).
pub(crate) fn repo_owner(dir: &Path) -> Option<String> {
    [".github/CODEOWNERS", "CODEOWNERS", "docs/CODEOWNERS"]
        .iter()
        .find_map(|p| std::fs::read_to_string(dir.join(p)).ok())
        .and_then(|t| codeowner(&t))
}

fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.to_ascii_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
        if out.len() >= 40 {
            break;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "change".into()
    } else {
        out
    }
}

fn short_id(seed: &str) -> String {
    use sha2::{Digest, Sha256};
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut h = Sha256::new();
    h.update(seed.as_bytes());
    h.update(nanos.to_le_bytes());
    h.update(std::process::id().to_le_bytes());
    h.finalize()
        .iter()
        .take(3)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The repo's template with each slot filled; a missing template becomes the
/// same headings. Always ends with the Hollow section.
fn pr_body(
    template: Option<&str>,
    req: &Request,
    scan_report: &[String],
    login: &str,
    owner: bool,
    attended: bool,
) -> String {
    let files: Vec<String> = req.files.iter().map(|f| format!("`{}`", f.path)).collect();
    let scan_text = scan_report.join("\n");
    let reviewed = if owner && attended {
        "**Reviewed:** opened from the owner's own Hollow chat; Hollow merges it once the checks pass."
    } else if owner {
        "**Reviewed:** opened under the owner's login by an unattended Hollow run; it waits for context.merge_org_change from an attended chat."
    } else {
        "**Reviewed:** not reviewed, because it was opened by an agent through Hollow; the owner approves it before it merges."
    };
    let first_reason = req.reason.trim().lines().next().unwrap_or("");
    let base = match template {
        Some(t) if t.contains("ctx-scan output") => {
            let mut out = String::new();
            for line in t.lines() {
                let l = if line == "One sentence." {
                    format!("{}: {first_reason}", req.title.trim())
                } else if line.starts_with("**Files or folders:**") {
                    format!("**Files or folders:** {}.", files.join(", "))
                } else if line.starts_with("**Why it is true for everyone in your organization:**") {
                    format!(
                        "**Why it is true for everyone in your organization:** {}",
                        req.reason.trim()
                    )
                } else if line.starts_with("**Reviewed:**") {
                    reviewed.to_string()
                } else if line.contains("No computer facts") || line.contains("No data files") {
                    // The scan truly backs these two (the first box also
                    // covers personal details about colleagues, which no
                    // pattern can promise); the rest stay for a person.
                    line.replacen("- [ ]", "- [x]", 1)
                } else if line.trim() == "paste the ctx-scan output here" {
                    scan_text.clone()
                } else {
                    line.to_string()
                };
                out.push_str(&l);
                out.push('\n');
            }
            out
        }
        _ => format!(
            "## What changed\n\n{}: {}\n\n**Files or folders:** {}.\n\n## ctx-scan output\n\n```\n{}\n```\n\n{}\n",
            req.title.trim(),
            req.reason.trim(),
            files.join(", "),
            scan_text,
            reviewed
        ),
    };
    format!(
        "{base}\n## Opened by Hollow\n\nGitHub login: `{login}`. Identity: **{}**.{}\n\nScan: Hollow's shared scanner (`scan_rules`, org layer) found no blocking finding in any file, the title or the reason (secrets, computer facts, pay and HR patterns, data files). Only the computer-facts and data-files boxes are ticked: personal details about colleagues are for the reviewer.\n",
        if owner { "owner" } else { "teammate" },
        if owner && attended {
            ""
        } else if owner {
            " Unattended run: Hollow did not merge this pull request; it waits for context.merge_org_change from an attended chat."
        } else {
            " Label: needs owner approval. Hollow does not merge this pull request."
        }
    )
}

// ---------------------------------------------------------------------------
// The flow
// ---------------------------------------------------------------------------

/// Branch, commit, push and open the pull request; then, for an owner PR,
/// [`merge_own`]. Nothing runs before validation and the scan pass.
pub(crate) fn propose(env: &Env, req: &Request) -> Result<Outcome, String> {
    validate(req)?;
    let scan_report = scan(req)?;
    let warnings: Vec<String> = scan_report
        .iter()
        .filter(|l| l.starts_with("warn"))
        .cloned()
        .collect();

    // The remote of the read-only clone; the clone itself is never written.
    let remote = must(
        env.runner,
        "reading the org clone's remote",
        git(env.org_root, &["remote", "get-url", "origin"]),
    )?
    .stdout
    .trim()
    .to_string();
    if remote.is_empty() {
        return Err("The org clone has no origin remote.".into());
    }
    std::fs::create_dir_all(env.work_dir).map_err(|e| format!("work folder: {e}"))?;
    let tmp = tempfile::Builder::new()
        .prefix("propose-")
        .tempdir_in(env.work_dir)
        .map_err(|e| format!("work folder: {e}"))?;
    let dir = tmp.path().join("repo");
    must(
        env.runner,
        "cloning the org repo",
        git(tmp.path(), &["clone", "--quiet", "--", &remote, "repo"]),
    )?;
    let base = must(
        env.runner,
        "reading the default branch",
        git(&dir, &["rev-parse", "--abbrev-ref", "HEAD"]),
    )?
    .stdout
    .trim()
    .to_string();

    let owner_name = repo_owner(&dir);
    let login = must(
        env.runner,
        "reading the GitHub login",
        gh(&dir, &["api", "user", "--jq", ".login"], None),
    )?
    .stdout
    .trim()
    .to_string();
    if login.is_empty()
        || login.len() > 80
        || !login
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("gh is not signed in (no login). Nothing was pushed.".into());
    }
    // Identity is the login alone; whether Hollow may merge also needs an
    // attended caller (`req.attended`).
    let owner = owner_name
        .as_deref()
        .is_some_and(|o| o.eq_ignore_ascii_case(&login));

    let branch = format!("hollow/{}-{}", slug(&req.title), short_id(&req.title));
    must(
        env.runner,
        "creating the branch",
        git(&dir, &["checkout", "--quiet", "-b", &branch]),
    )?;
    for f in &req.files {
        write_file(&dir, f)?;
    }
    let paths: Vec<&str> = req.files.iter().map(|f| f.path.as_str()).collect();
    let mut add = vec!["add", "--"];
    add.extend(paths.iter().copied());
    must(env.runner, "staging the files", git(&dir, &add))?;
    // Only a session key reaches the commit and the record: nothing free-form.
    let chat = req
        .chat
        .as_deref()
        .filter(|c| crate::agent_sessions::is_safe_session_key(c));
    let message =
        memory_guard::sync_commit_message(req.title.trim(), &[(login.as_str(), chat)], &paths);
    must(
        env.runner,
        "committing",
        git(&dir, &["commit", "--quiet", "-m", &message]),
    )?;
    must(
        env.runner,
        "pushing the branch",
        git(&dir, &["push", "--quiet", "origin", &branch]),
    )?;
    let head = must(
        env.runner,
        "reading the pushed commit",
        git(&dir, &["rev-parse", "HEAD"]),
    )?
    .stdout
    .trim()
    .to_string();

    let template = std::fs::read_to_string(dir.join(".github/PULL_REQUEST_TEMPLATE.md")).ok();
    let body = pr_body(
        template.as_deref(),
        req,
        &scan_report,
        &login,
        owner,
        req.attended,
    );
    let created = must(
        env.runner,
        "opening the pull request",
        gh(
            &dir,
            &[
                "pr",
                "create",
                "--base",
                &base,
                "--head",
                &branch,
                "--title",
                req.title.trim(),
                "--body-file",
                "-",
            ],
            Some(body),
        ),
    )?;
    let url = created
        .stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| l.starts_with("https://"))
        .ok_or("gh did not print the pull request's URL.")?
        .to_string();

    let mut outcome = Outcome {
        url: url.clone(),
        branch,
        login,
        identity: if owner { "owner" } else { "teammate" }.into(),
        status: "waiting".into(),
        reason: String::new(),
        warnings,
    };
    let (pr_owner, pr_repo, _) = parse_pr_url(&url)
        .ok_or("gh printed a URL that is not a GitHub pull request. Nothing was merged.")?;
    let expect = Expect {
        sha: head,
        branch: outcome.branch.clone(),
        base,
        repo: format!("{pr_owner}/{pr_repo}"),
    };
    if owner && req.attended {
        match merge_own(env, &dir, &url, &expect) {
            Ok(()) => outcome.status = "merged".into(),
            Err(why) => outcome.reason = why,
        }
    } else {
        if owner {
            // The owner's own unattended PR: no label (nobody else's approval
            // is awaited); it waits for the later merge step.
            outcome.reason = "waiting for owner: opened by an unattended run; merge it with context.merge_org_change from an attended chat".into();
        } else {
            outcome.reason = LABEL_WAITING.into();
            // Best effort: the label may not exist in the repo yet; the body
            // already says it.
            let _ = env.runner.run(&gh(
                &dir,
                &["pr", "edit", &url, "--add-label", LABEL_WAITING],
                None,
            ));
        }
    }
    record(env.ledger, &outcome, &expect, chat, &req.title, &paths);
    Ok(outcome)
}

fn write_file(dir: &Path, f: &FileChange) -> Result<(), String> {
    let target = dir.join(&f.path);
    // No part of the path may be a link (a fresh clone can carry one).
    let mut at = dir.to_path_buf();
    for part in f.path.split('/') {
        at.push(part);
        if let Ok(meta) = std::fs::symlink_metadata(&at) {
            if meta.file_type().is_symlink() {
                return Err(format!(
                    "{:?} goes through a link in the repo.",
                    clip(&f.path, 80)
                ));
            }
        }
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("writing {:?}: {e}", clip(&f.path, 80)))?;
    }
    let mut content = f.content.clone();
    if !content.ends_with('\n') {
        content.push('\n');
    }
    std::fs::write(&target, content).map_err(|e| format!("writing {:?}: {e}", clip(&f.path, 80)))
}

/// What the pushed pull request must be: the commit that was pushed, the
/// branch Hollow created and the base it targets.
#[derive(Debug, Clone)]
pub(crate) struct Expect {
    pub sha: String,
    pub branch: String,
    pub base: String,
    /// `owner/repo` of the pull request.
    pub repo: String,
}

/// `(owner, repo, number)` of `https://github.com/<owner>/<repo>/pull/<n>`.
pub(crate) fn parse_pr_url(url: &str) -> Option<(String, String, u64)> {
    let rest = url.strip_prefix("https://github.com/")?;
    let parts: Vec<&str> = rest.split('/').collect();
    let ok_name = |n: &str| {
        !n.is_empty()
            && n.len() <= 100
            && n != "."
            && n != ".."
            && n.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    };
    match parts.as_slice() {
        [o, r, "pull", n] if ok_name(o) && ok_name(r) => {
            Some((o.to_string(), r.to_string(), n.parse().ok()?))
        }
        _ => None,
    }
}

/// `owner/repo` of a GitHub remote URL (https or ssh form, `.git` optional).
pub(crate) fn parse_remote_repo(remote: &str) -> Option<String> {
    let rest = remote
        .strip_prefix("https://github.com/")
        .or_else(|| remote.strip_prefix("git@github.com:"))
        .or_else(|| remote.strip_prefix("ssh://git@github.com/"))?;
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let mut it = rest.split('/');
    let (o, r) = (it.next()?, it.next()?);
    (it.next().is_none() && !o.is_empty() && !r.is_empty()).then(|| format!("{o}/{r}"))
}

/// Percent-encodes everything but unreserved characters (a branch name goes
/// into an API path).
fn pct(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A required check: its name, and whether only a CheckRun may satisfy it
/// (the rule names an app, so a commit status of the same name doesn't count).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Required {
    name: String,
    run_only: bool,
}

/// One look at a pull request.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    /// Every condition holds.
    Go,
    /// Already merged (the second action is idempotent).
    AlreadyMerged,
    /// Not yet: pending checks or GitHub still computing mergeability.
    Wait(String),
    /// Never, as it stands: a mismatch, a failure, a draft, a conflict.
    Refuse(String),
}

/// The separate step that merges an owner's PR. Up to [`MERGE_TRIES`] looks,
/// [`MERGE_PAUSE`] apart, while it is only waiting (checks pending, mergeable
/// UNKNOWN, a required check not reported yet); a refusal ends it at once.
/// `Err` is why it waits. The merge is pinned to the pushed commit with
/// `--match-head-commit`, so a push after the look can't be merged unseen;
/// `--admin` is the owner's bypass of the required review (plan D2), used only
/// once the checks were read and passed.
pub(crate) fn merge_own(env: &Env, dir: &Path, url: &str, expect: &Expect) -> Result<(), String> {
    let mut last = String::new();
    for attempt in 0..MERGE_TRIES {
        if attempt > 0 {
            (env.sleep)(MERGE_PAUSE);
        }
        match look(env.runner, dir, url, expect)? {
            Step::AlreadyMerged => return Ok(()),
            Step::Go => {
                must(
                    env.runner,
                    "merging",
                    gh(
                        dir,
                        &[
                            "pr",
                            "merge",
                            url,
                            "--squash",
                            "--admin",
                            "-R",
                            &expect.repo,
                            "--match-head-commit",
                            &expect.sha,
                        ],
                        None,
                    ),
                )?;
                return Ok(());
            }
            Step::Wait(why) => last = why,
            Step::Refuse(why) => return Err(why),
        }
    }
    Err(format!("{last}; still not ready after {MERGE_TRIES} tries"))
}

/// Reads the pull request and the base branch's required checks and decides.
fn look(runner: &dyn Runner, dir: &Path, url: &str, expect: &Expect) -> Result<Step, String> {
    let view = must(
        runner,
        "reading the pull request's state",
        gh(
            dir,
            &[
                "pr",
                "view",
                url,
                "-R",
                &expect.repo,
                "--json",
                "state,isDraft,mergeable,headRefOid,headRefName,baseRefName,statusCheckRollup",
            ],
            None,
        ),
    )?;
    let v: Value = serde_json::from_str(&view.stdout)
        .map_err(|_| "the pull request's state was not readable".to_string())?;
    // TOCTOU: it must still be exactly what Hollow pushed.
    if v["headRefOid"].as_str() != Some(expect.sha.as_str())
        || v["headRefName"].as_str() != Some(expect.branch.as_str())
        || v["baseRefName"].as_str() != Some(expect.base.as_str())
    {
        return Ok(Step::Refuse(
            "the pull request is no longer the commit, branch and base Hollow pushed".into(),
        ));
    }
    let required = required_checks(runner, dir, &expect.repo, &expect.base)?;
    // A workflow that isn't required still must not be raced: with workflows
    // in the repo, an empty rollup means "not started yet".
    let workflows = std::fs::read_dir(dir.join(".github/workflows"))
        .map(|d| d.flatten().next().is_some())
        .unwrap_or(false);
    Ok(judge(&v, &required, workflows))
}

/// The base branch's required checks: the classic protection's `contexts`
/// and `checks`, plus the `required_status_checks` rules of any ruleset that
/// applies to it. A 404 from either is "none"; any other failure stops the
/// merge.
fn required_checks(
    runner: &dyn Runner,
    dir: &Path,
    repo: &str,
    base: &str,
) -> Result<Vec<Required>, String> {
    let mut names: Vec<Required> = Vec::new();
    let base = pct(base);
    let protection = format!("repos/{repo}/branches/{base}/protection/required_status_checks");
    if let Some(v) = api_json(runner, dir, &protection, "the required checks")? {
        for c in v["contexts"].as_array().into_iter().flatten() {
            if let Some(n) = c.as_str() {
                names.push(Required {
                    name: n.to_string(),
                    run_only: false,
                });
            }
        }
        for c in v["checks"].as_array().into_iter().flatten() {
            if let Some(n) = c["context"].as_str() {
                names.push(Required {
                    name: n.to_string(),
                    run_only: c["app_id"].as_i64().is_some_and(|id| id > 0),
                });
            }
        }
    }
    let rules = format!("repos/{repo}/rules/branches/{base}");
    if let Some(v) = api_json(runner, dir, &rules, "the branch rules")? {
        for rule in v.as_array().into_iter().flatten() {
            if rule["type"].as_str() != Some("required_status_checks") {
                continue;
            }
            for c in rule["parameters"]["required_status_checks"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if let Some(n) = c["context"].as_str() {
                    names.push(Required {
                        name: n.to_string(),
                        run_only: c["integration_id"].as_i64().is_some_and(|id| id > 0),
                    });
                }
            }
        }
    }
    // A name listed twice is as strict as its strictest entry.
    names.sort_by(|a, b| a.name.cmp(&b.name).then(b.run_only.cmp(&a.run_only)));
    names.dedup_by(|b, a| a.name == b.name);
    Ok(names)
}

/// `gh api <path>` as JSON: `None` on a 404 (not protected, no rules), `Err`
/// on any other failure.
fn api_json(
    runner: &dyn Runner,
    dir: &Path,
    path: &str,
    what: &str,
) -> Result<Option<Value>, String> {
    let out = runner.run(&gh(dir, &["api", path], None))?;
    if !out.ok {
        let err = out.stderr.to_lowercase();
        if err.contains("404") || err.contains("not found") || err.contains("not protected") {
            return Ok(None);
        }
        return Err(format!("reading {what} failed"));
    }
    serde_json::from_str(&out.stdout)
        .map(Some)
        .map_err(|_| format!("{what} were not readable"))
}

/// The rollup with only the latest entry per name (a re-run replaces the
/// older one: by `completedAt`, `startedAt` or `createdAt`, else array order).
/// Entries with no name stay as they are.
fn latest_per_name(rollup: Option<&Vec<Value>>) -> Vec<&Value> {
    let stamp = |c: &Value| {
        ["completedAt", "startedAt", "createdAt"]
            .iter()
            .find_map(|k| c[*k].as_str().filter(|t| !t.is_empty()))
            .unwrap_or("")
            .to_string()
    };
    let mut out: Vec<&Value> = Vec::new();
    for c in rollup.into_iter().flatten() {
        let name = c["name"].as_str().or_else(|| c["context"].as_str());
        let same = name.and_then(|n| {
            out.iter()
                .position(|o| o["name"].as_str().or_else(|| o["context"].as_str()) == Some(n))
        });
        match same {
            Some(at) if stamp(c) >= stamp(out[at]) => out[at] = c,
            Some(_) => {}
            None => out.push(c),
        }
    }
    out
}

/// How one rollup entry stands.
#[derive(PartialEq, Eq)]
enum Check {
    /// Final and SUCCESS.
    Success,
    /// Final and neutral or skipped: fine unless it is required.
    Neutral,
    Pending,
    Failed,
}

fn check_state(c: &Value) -> Check {
    // A StatusContext has `state`; a CheckRun has `status` and `conclusion`.
    if let Some(state) = c["state"].as_str() {
        return match state {
            "SUCCESS" => Check::Success,
            "PENDING" | "EXPECTED" => Check::Pending,
            _ => Check::Failed,
        };
    }
    if c["status"].as_str() != Some("COMPLETED") {
        return Check::Pending;
    }
    match c["conclusion"].as_str().unwrap_or("") {
        "SUCCESS" => Check::Success,
        "NEUTRAL" | "SKIPPED" => Check::Neutral,
        _ => Check::Failed,
    }
}

/// The merge conditions on `gh pr view --json` output and the required
/// check names. A required check must be in the rollup with a final SUCCESS;
/// with no required list an empty rollup is allowed, but any check present
/// must be final and passing.
fn judge(v: &Value, required: &[Required], workflows: bool) -> Step {
    match v["state"].as_str() {
        Some("MERGED") => return Step::AlreadyMerged,
        Some("OPEN") => {}
        _ => return Step::Refuse("the pull request is not open".into()),
    }
    if v["isDraft"].as_bool() == Some(true) {
        return Step::Refuse("the pull request is a draft".into());
    }
    match v["mergeable"].as_str() {
        Some("MERGEABLE") => {}
        Some("CONFLICTING") => return Step::Refuse("the pull request has conflicts".into()),
        _ => return Step::Wait("GitHub has not worked out mergeability yet".into()),
    }
    let rollup = latest_per_name(v["statusCheckRollup"].as_array());
    if rollup.is_empty() && workflows {
        return Step::Wait("the repo has workflows and no check has reported yet".into());
    }
    let mut pending = false;
    for c in &rollup {
        match check_state(c) {
            Check::Failed => return Step::Refuse("a check failed".into()),
            Check::Pending => pending = true,
            Check::Success | Check::Neutral => {}
        }
    }
    for Required { name, run_only } in required {
        let found = rollup.iter().find(|c| {
            (c["name"].as_str() == Some(name) || c["context"].as_str() == Some(name))
                && (!run_only || c["state"].as_str().is_none())
        });
        match found {
            None => {
                return Step::Wait(format!("required check {name} has not reported"));
            }
            Some(c) if check_state(c) == Check::Success => {}
            Some(c) if check_state(c) == Check::Pending => pending = true,
            Some(_) => return Step::Refuse(format!("required check {name} did not succeed")),
        }
    }
    if pending {
        return Step::Wait("the checks have not finished".into());
    }
    Step::Go
}

/// What `context.merge_org_change` returns.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MergeOutcome {
    pub url: String,
    /// `merged` or `waiting`.
    pub status: String,
    pub reason: String,
}

/// The later step: merge a pull request this feature opened as the owner and
/// left waiting. It re-runs every check [`propose`] would: the caller is
/// attended, the record shows this feature opened it as owner, the signed-in
/// login is still the owner in `CODEOWNERS` and the record's login, the head
/// is the pushed commit, and the required checks passed. Idempotent: an
/// already merged PR answers `merged`.
pub(crate) fn merge_existing(env: &Env, url: &str, attended: bool) -> Result<MergeOutcome, String> {
    if !attended {
        return Err(
            "Hollow merges only from an attended chat; this is an unattended or nested run. Nothing was merged."
                .into(),
        );
    }
    // The URL must be a pull request of the org repo itself (the clone's
    // origin, read raw so a local `insteadOf` can't hide it), and the record
    // must agree: Hollow never merges in another repo.
    let (url_owner, url_repo, _) =
        parse_pr_url(url).ok_or("That is not a GitHub pull request URL. Nothing was merged.")?;
    let url_slug = format!("{url_owner}/{url_repo}");
    let origin = must(
        env.runner,
        "reading the org clone's origin",
        git(env.org_root, &["config", "--get", "remote.origin.url"]),
    )?
    .stdout;
    if parse_remote_repo(origin.trim())
        .as_deref()
        .is_none_or(|r| !r.eq_ignore_ascii_case(&url_slug))
    {
        return Err(format!(
            "{url_slug} is not the org repo (the org clone's origin). Nothing was merged."
        ));
    }
    let opened = read_opened(env.ledger, url)
        .ok_or("That pull request is not one Hollow opened as the owner (no such line in the record). Nothing was merged.")?;
    if opened.identity != "owner" {
        return Err("That pull request was opened by a teammate's login and waits for the owner. Nothing was merged.".into());
    }
    if !opened.repo.eq_ignore_ascii_case(&url_slug) {
        return Err(
            "The record names a different repo for that pull request. Nothing was merged.".into(),
        );
    }
    let login = must(
        env.runner,
        "reading the GitHub login",
        gh(env.org_root, &["api", "user", "--jq", ".login"], None),
    )?
    .stdout
    .trim()
    .to_string();
    let owner = repo_owner(env.org_root);
    if !owner
        .as_deref()
        .is_some_and(|o| o.eq_ignore_ascii_case(&login))
        || !opened.login.eq_ignore_ascii_case(&login)
    {
        return Err("The signed-in GitHub login is not the owner who opened this pull request. Nothing was merged.".into());
    }
    let expect = Expect {
        sha: opened.head,
        branch: opened.branch,
        base: opened.base,
        repo: url_slug,
    };
    let (status, reason) = match merge_own(env, env.org_root, url, &expect) {
        Ok(()) => ("merged", String::new()),
        Err(why) => ("waiting", why),
    };
    let out = MergeOutcome {
        url: url.to_string(),
        status: status.into(),
        reason,
    };
    // No second digest line when the record already says merged.
    if status == "merged" && !opened.merged && opened.status != "merged" {
        record_merged(env.ledger, url, &login);
    }
    Ok(out)
}

/// The later `merged` line for a pull request that waited (CX8b's digest
/// lists it on the day of this line).
pub(crate) fn record_merged(ledger: &Path, url: &str, login: &str) {
    append(
        ledger,
        &json!({
            "event": "merged",
            "at": chrono::Utc::now().to_rfc3339(),
            "url": url,
            "login": login,
            "identity": "owner",
            "status": "merged",
            "digest": true,
        }),
    );
}

struct Opened {
    status: String,
    repo: String,
    identity: String,
    login: String,
    head: String,
    branch: String,
    base: String,
    /// A later `merged` line exists.
    merged: bool,
}

/// The record's opened line for `url` (and whether a merge line followed).
fn read_opened(ledger: &Path, url: &str) -> Option<Opened> {
    let text = std::fs::read_to_string(ledger).ok()?;
    let mut opened: Option<Opened> = None;
    let mut merged = false;
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v["url"].as_str() != Some(url) {
            continue;
        }
        if v["event"].as_str() == Some("merged") {
            merged = true;
        } else if opened.is_none() {
            opened = Some(Opened {
                status: v["status"].as_str()?.to_string(),
                repo: v["repo"].as_str()?.to_string(),
                identity: v["identity"].as_str()?.to_string(),
                login: v["login"].as_str()?.to_string(),
                head: v["head"].as_str()?.to_string(),
                branch: v["branch"].as_str()?.to_string(),
                base: v["base"].as_str()?.to_string(),
                merged: false,
            });
        }
    }
    opened.map(|mut o| {
        o.merged = merged;
        o
    })
}

/// Append one line to the record. Failing to write it never fails the
/// proposal (the PR exists); it is logged.
fn append(ledger: &Path, line: &Value) {
    let result = (|| -> std::io::Result<()> {
        if let Some(parent) = ledger.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(ledger)?;
        f.write_all(format!("{line}\n").as_bytes())
    })();
    if let Err(e) = result {
        eprintln!("[context-propose] the record was not written: {e}");
    }
}

/// One `opened` line per pull request; a later `merged` line (same url) when
/// `context.merge_org_change` merges one that waited. The latest line for a
/// url is its state; CX8b's digest (`context_org_digest`) reads `digest`.
pub(crate) fn record(
    ledger: &Path,
    o: &Outcome,
    e: &Expect,
    chat: Option<&str>,
    title: &str,
    files: &[&str],
) {
    append(
        ledger,
        &json!({
            "event": "opened",
            "at": chrono::Utc::now().to_rfc3339(),
            "url": o.url,
            "title": clip(title.trim(), MAX_TITLE_CHARS),
            "branch": o.branch,
            "base": e.base,
            "repo": e.repo,
            "head": e.sha,
            "login": o.login,
            "identity": o.identity,
            "status": o.status,
            "reason": o.reason,
            "chat": chat,
            "files": files,
            "digest": o.identity == "owner",
        }),
    );
}

/// The record's path in Hollow's app data.
pub(crate) fn ledger_path() -> Result<PathBuf, String> {
    Ok(crate::agent_settings::app_data_dir()?.join(LEDGER_FILE))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const READY: &str = r#"{"state":"OPEN","isDraft":false,"mergeable":"MERGEABLE","headRefOid":"@SHA@","headRefName":"@BRANCH@","baseRefName":"main","statusCheckRollup":[{"__typename":"CheckRun","name":"scan","status":"COMPLETED","conclusion":"SUCCESS"}]}"#;

    fn no_sleep(_: std::time::Duration) {}

    /// Real `git` against a scratch bare repo; a fake `gh` that records its
    /// calls. `login` is what `gh api user` answers; `views` are the answers
    /// of successive `gh pr view` calls (the last repeats), with `@SHA@` and
    /// `@BRANCH@` filled from what was pushed; `required` is the protection
    /// API's answer (`None`: 404, no protection).
    struct Fake {
        login: RefCell<String>,
        views: RefCell<std::collections::VecDeque<String>>,
        last_view: RefCell<String>,
        required: RefCell<Option<String>>,
        rules: RefCell<Result<String, String>>,
        sha: RefCell<String>,
        branch: RefCell<String>,
        pr_no: RefCell<u32>,
        calls: RefCell<Vec<String>>,
        git_calls: RefCell<usize>,
        stdin_seen: RefCell<Vec<String>>,
    }

    impl Fake {
        fn new(login: &str) -> Self {
            Fake {
                login: RefCell::new(login.into()),
                views: RefCell::new(Default::default()),
                last_view: RefCell::new(READY.into()),
                required: RefCell::new(None),
                rules: RefCell::new(Ok("[]".into())),
                sha: RefCell::new(String::new()),
                branch: RefCell::new(String::new()),
                pr_no: RefCell::new(7),
                calls: RefCell::new(vec![]),
                git_calls: RefCell::new(0),
                stdin_seen: RefCell::new(vec![]),
            }
        }
        fn set_views(&self, views: &[&str]) {
            *self.views.borrow_mut() = views.iter().map(|v| v.to_string()).collect();
        }
        fn gh_calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
        fn count(&self, prefix: &str) -> usize {
            self.gh_calls()
                .iter()
                .filter(|c| c.starts_with(prefix))
                .count()
        }
    }

    fn scrubbed_git(dir: &Path, args: &[String]) -> std::io::Result<std::process::Output> {
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
    }

    impl Runner for Fake {
        fn run(&self, cmd: &Cmd) -> Result<Out, String> {
            if cmd.program == "git" {
                *self.git_calls.borrow_mut() += 1;
                let out = scrubbed_git(cmd.cwd.as_ref().unwrap(), &cmd.args)
                    .map_err(|e| e.to_string())?;
                if cmd.args == ["rev-parse", "HEAD"] {
                    *self.sha.borrow_mut() = String::from_utf8_lossy(&out.stdout).trim().into();
                }
                return Ok(Out {
                    ok: out.status.success(),
                    stdout: String::from_utf8_lossy(&out.stdout).into(),
                    stderr: String::from_utf8_lossy(&out.stderr).into(),
                });
            }
            assert_eq!(cmd.program, "gh", "only git and gh");
            let line = cmd.args.join(" ");
            self.calls.borrow_mut().push(line.clone());
            if let Some(s) = &cmd.stdin {
                self.stdin_seen.borrow_mut().push(s.clone());
            }
            if line.starts_with("api repos/") && line.contains("/rules/branches/") {
                return Ok(match &*self.rules.borrow() {
                    Ok(json) => Out {
                        ok: true,
                        stdout: json.clone(),
                        stderr: String::new(),
                    },
                    Err(e) => Out {
                        ok: false,
                        stdout: String::new(),
                        stderr: e.clone(),
                    },
                });
            }
            if line.starts_with("api repos/") {
                return Ok(match &*self.required.borrow() {
                    None => Out {
                        ok: false,
                        stdout: String::new(),
                        stderr: "gh: Not Found (HTTP 404)".into(),
                    },
                    Some(json) => Out {
                        ok: true,
                        stdout: json.clone(),
                        stderr: String::new(),
                    },
                });
            }
            let stdout = if line.starts_with("api user") {
                format!("{}\n", self.login.borrow())
            } else if line.starts_with("pr create") {
                if let Some(at) = cmd.args.iter().position(|a| a == "--head") {
                    *self.branch.borrow_mut() = cmd.args[at + 1].clone();
                }
                {
                    let n = *self.pr_no.borrow();
                    *self.pr_no.borrow_mut() += 1;
                    format!("https://github.com/example/org-context/pull/{n}\n")
                }
            } else if line.starts_with("pr view") {
                let next = self.views.borrow_mut().pop_front();
                if let Some(n) = next {
                    *self.last_view.borrow_mut() = n;
                }
                self.last_view
                    .borrow()
                    .replace("@SHA@", &self.sha.borrow())
                    .replace("@BRANCH@", &self.branch.borrow())
            } else {
                String::new()
            };
            Ok(Out {
                ok: true,
                stdout,
                stderr: String::new(),
            })
        }
    }

    struct Scratch {
        _dir: tempfile::TempDir,
        bare: PathBuf,
        org: PathBuf,
        work: PathBuf,
        ledger: PathBuf,
    }

    fn sh(dir: &Path, args: &[&str]) {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let out = scrubbed_git(dir, &args).unwrap();
        assert!(out.status.success(), "git {args:?}");
    }

    fn scratch() -> Scratch {
        scratch_with(false)
    }

    fn scratch_with(workflows: bool) -> Scratch {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let bare = base.join("origin.git");
        let seed = base.join("seed");
        std::fs::create_dir_all(&bare).unwrap();
        std::fs::create_dir_all(seed.join(".github")).unwrap();
        sh(
            &bare,
            &["init", "--quiet", "--bare", "--initial-branch=main"],
        );
        sh(&seed, &["init", "--quiet", "--initial-branch=main"]);
        std::fs::write(seed.join("CODEOWNERS"), "# owned\n* @TheOwner\n").unwrap();
        std::fs::write(seed.join("INDEX.md"), "# Index\n").unwrap();
        if workflows {
            std::fs::create_dir_all(seed.join(".github/workflows")).unwrap();
            std::fs::write(seed.join(".github/workflows/ci.yml"), "name: ci\n").unwrap();
        }
        std::fs::write(
            seed.join(".github/PULL_REQUEST_TEMPLATE.md"),
            "## What changed\n\nOne sentence.\n\n**Files or folders:** for example `systems/`.\n\n## Which layer\n\n**Why it is true for everyone in your organization:** one sentence.\n\n## ctx-scan output\n\n```\npaste the ctx-scan output here\n```\n\n## Checks\n\n- [ ] No secrets, tokens, keys or passwords, and no personal data.\n- [ ] No computer facts (paths).\n- [ ] No tasks or status.\n- [ ] No data files (csv).\n\n**Reviewed:** one line: who read it.\n",
        )
        .unwrap();
        sh(&seed, &["add", "."]);
        sh(&seed, &["commit", "--quiet", "-m", "seed"]);
        sh(&seed, &["remote", "add", "origin", bare.to_str().unwrap()]);
        sh(&seed, &["push", "--quiet", "origin", "main"]);
        sh(&base, &["clone", "--quiet", bare.to_str().unwrap(), "org"]);
        // The clone's origin reads as the GitHub repo (what the repo check
        // parses); `insteadOf` still routes git to the scratch bare repo.
        let gh_url = "https://github.com/example/org-context.git";
        sh(&base.join("org"), &["config", "remote.origin.url", gh_url]);
        sh(
            &base.join("org"),
            &[
                "config",
                &format!("url.{}.insteadOf", bare.display()),
                gh_url,
            ],
        );
        Scratch {
            work: base.join("work"),
            ledger: base.join("ledger/context-org-prs.jsonl"),
            org: base.join("org"),
            _dir: dir,
            bare,
        }
    }

    fn request(files: Vec<(&str, &str)>) -> Request {
        Request {
            files: files
                .into_iter()
                .map(|(p, c)| FileChange {
                    path: p.into(),
                    content: c.into(),
                })
                .collect(),
            title: "Add the parking glossary entry".into(),
            reason: "Every location uses the term; it came from the ops handbook.".into(),
            chat: Some("s-20261115-abc".into()),
            attended: true,
        }
    }

    fn env<'a>(s: &'a Scratch, fake: &'a Fake) -> Env<'a> {
        Env {
            runner: fake,
            org_root: &s.org,
            work_dir: &s.work,
            ledger: &s.ledger,
            sleep: &no_sleep,
        }
    }

    fn ledger_lines(s: &Scratch) -> Vec<Value> {
        std::fs::read_to_string(&s.ledger)
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn bare_git(s: &Scratch, args: &[&str]) -> String {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let out = scrubbed_git(&s.bare, &args).unwrap();
        String::from_utf8_lossy(&out.stdout).into()
    }

    #[test]
    fn a_dry_run_pr_pushes_a_branch_with_actors_template_and_scan() {
        let s = scratch();
        let fake = Fake::new("someone-else");
        let out = propose(
            &env(&s, &fake),
            &request(vec![("glossary.md", "# Glossary\n\nSMF: a short fact.\n")]),
        )
        .unwrap();
        assert_eq!(out.url, "https://github.com/example/org-context/pull/7");
        assert!(out
            .branch
            .starts_with("hollow/add-the-parking-glossary-entry-"));
        assert!(bare_git(&s, &["branch", "--list"]).contains(&out.branch));
        let msg = bare_git(&s, &["log", "-1", "--format=%B", &out.branch]);
        assert!(
            msg.contains("Actor: someone-else (chat s-20261115-abc)"),
            "{msg}"
        );
        assert!(msg.contains("File: glossary.md"), "{msg}");
        let body = fake.stdin_seen.borrow().last().unwrap().clone();
        assert!(body.contains("## What changed"));
        assert!(body.contains("**Files or folders:** `glossary.md`."));
        assert!(body.contains("## ctx-scan output"));
        assert!(body.contains("no findings"));
        assert!(!body.contains("paste the ctx-scan output here"));
        // Only the boxes the scan truly backs are ticked.
        assert!(body.contains("- [ ] No secrets"));
        assert!(body.contains("- [x] No computer facts"));
        assert!(body.contains("- [x] No data files"));
        assert!(body.contains("needs owner approval"));
        // The read-only clone was never written.
        assert!(!s.org.join("glossary.md").exists());
    }

    #[test]
    fn the_owners_own_chat_merges_through_the_fake_gh() {
        let s = scratch();
        let fake = Fake::new("theowner");
        let out = propose(
            &env(&s, &fake),
            &request(vec![("glossary.md", "# Glossary\n\nA fact.\n")]),
        )
        .unwrap();
        assert_eq!(
            (out.identity.as_str(), out.status.as_str()),
            ("owner", "merged")
        );
        let calls = fake.gh_calls();
        assert!(
            calls
                .iter()
                .any(|c| c.starts_with("pr merge") && c.contains("--squash")),
            "{calls:?}"
        );
        assert!(!calls.iter().any(|c| c.contains("needs owner approval")));
        let sha = bare_git(&s, &["rev-parse", &out.branch]).trim().to_string();
        assert!(
            calls
                .iter()
                .any(|c| c.starts_with("pr merge")
                    && c.ends_with(&format!("--match-head-commit {sha}"))),
            "{calls:?}"
        );
        let rows = ledger_lines(&s);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["status"], "merged");
        assert_eq!(rows[0]["digest"], true);
    }

    fn owner_propose(s: &Scratch, fake: &Fake) -> Outcome {
        propose(
            &env(s, fake),
            &request(vec![("glossary.md", "# G\n\nA fact.\n")]),
        )
        .unwrap()
    }

    fn rollup(entries: &str) -> String {
        READY.replace(
            r#""statusCheckRollup":[{"__typename":"CheckRun","name":"scan","status":"COMPLETED","conclusion":"SUCCESS"}]"#,
            &format!(r#""statusCheckRollup":[{entries}]"#),
        )
    }

    #[test]
    fn an_owner_pr_with_pending_checks_waits_after_six_tries() {
        let s = scratch();
        let fake = Fake::new("theowner");
        fake.set_views(&[&rollup(
            r#"{"__typename":"CheckRun","name":"scan","status":"IN_PROGRESS","conclusion":""}"#,
        )]);
        let pauses = RefCell::new(Vec::new());
        let count = |d: std::time::Duration| pauses.borrow_mut().push(d);
        let e = Env {
            sleep: &count,
            ..env(&s, &fake)
        };
        let out = propose(&e, &request(vec![("glossary.md", "# G\n\nA fact.\n")])).unwrap();
        assert_eq!(out.status, "waiting");
        assert!(out.reason.contains("checks"), "{}", out.reason);
        assert_eq!(fake.count("pr view"), MERGE_TRIES);
        assert_eq!(fake.count("pr merge"), 0);
        assert_eq!(*pauses.borrow(), vec![MERGE_PAUSE; 5]);
        assert_eq!(MERGE_PAUSE, std::time::Duration::from_secs(10));
    }

    #[test]
    fn an_empty_rollup_with_a_required_list_waits() {
        let s = scratch();
        let fake = Fake::new("theowner");
        *fake.required.borrow_mut() = Some(r#"{"contexts":["ctx-scan"],"checks":[]}"#.into());
        fake.set_views(&[&rollup("")]);
        let out = owner_propose(&s, &fake);
        assert_eq!(out.status, "waiting");
        assert!(
            out.reason
                .contains("required check ctx-scan has not reported"),
            "{}",
            out.reason
        );
        assert_eq!(fake.count("pr merge"), 0);
    }

    #[test]
    fn an_empty_rollup_with_no_required_checks_may_merge() {
        let s = scratch();
        let fake = Fake::new("theowner");
        fake.set_views(&[&rollup("")]);
        assert_eq!(owner_propose(&s, &fake).status, "merged");
    }

    #[test]
    fn a_missing_required_check_waits_and_a_present_one_merges() {
        let s = scratch();
        let fake = Fake::new("theowner");
        *fake.required.borrow_mut() =
            Some(r#"{"contexts":["a"],"checks":[{"context":"b","app_id":1}]}"#.into());
        let only_a = rollup(r#"{"__typename":"StatusContext","context":"a","state":"SUCCESS"}"#);
        fake.set_views(&[&only_a]);
        let out = owner_propose(&s, &fake);
        assert_eq!(out.status, "waiting");
        assert!(out.reason.contains("required check b"), "{}", out.reason);
        assert_eq!(fake.count("pr merge"), 0);
        let both = rollup(
            r#"{"__typename":"StatusContext","context":"a","state":"SUCCESS"},{"__typename":"CheckRun","name":"b","status":"COMPLETED","conclusion":"SUCCESS"}"#,
        );
        fake.set_views(&[&both]);
        assert_eq!(owner_propose(&s, &fake).status, "merged");
    }

    #[test]
    fn a_head_that_is_not_the_pushed_commit_is_refused() {
        let s = scratch();
        let fake = Fake::new("theowner");
        fake.set_views(&[&READY.replace("@SHA@", "deadbeef")]);
        let out = owner_propose(&s, &fake);
        assert_eq!(out.status, "waiting");
        assert!(
            out.reason.contains("no longer the commit"),
            "{}",
            out.reason
        );
        assert_eq!(fake.count("pr view"), 1, "a refusal does not retry");
        assert_eq!(fake.count("pr merge"), 0);
        // A different base is refused the same way.
        let fake = Fake::new("theowner");
        fake.set_views(&[&READY.replace(r#""baseRefName":"main""#, r#""baseRefName":"other""#)]);
        let out = owner_propose(&s, &fake);
        assert!(
            out.reason.contains("no longer the commit"),
            "{}",
            out.reason
        );
    }

    #[test]
    fn unknown_mergeability_is_retried_then_merges() {
        let s = scratch();
        let fake = Fake::new("theowner");
        fake.set_views(&[&READY.replace("MERGEABLE", "UNKNOWN"), READY]);
        let out = owner_propose(&s, &fake);
        assert_eq!(out.status, "merged");
        assert_eq!(fake.count("pr view"), 2);
        assert_eq!(fake.count("pr merge"), 1);
    }

    #[test]
    fn an_unattended_owner_run_opens_the_pr_and_does_not_merge() {
        let s = scratch();
        let fake = Fake::new("theowner");
        let mut req = request(vec![("glossary.md", "# G\n\nA fact.\n")]);
        req.attended = false;
        let out = propose(&env(&s, &fake), &req).unwrap();
        assert_eq!(out.status, "waiting");
        assert!(out.reason.contains("unattended"), "{}", out.reason);
        assert_eq!(fake.count("pr view") + fake.count("pr merge"), 0);
        // The owner's own PR carries no approval label, and says what it waits for.
        assert_eq!(fake.count("pr edit"), 0);
        let body = fake.stdin_seen.borrow().last().unwrap().clone();
        assert!(body.contains("context.merge_org_change"), "{body}");
        assert!(!body.contains("needs owner approval"), "{body}");
    }

    #[test]
    fn the_second_action_merges_a_waiting_owner_pr_once_ready_and_is_idempotent() {
        let s = scratch();
        let fake = Fake::new("theowner");
        fake.set_views(&[&READY.replace("MERGEABLE", "UNKNOWN")]);
        let out = owner_propose(&s, &fake);
        assert_eq!(out.status, "waiting");
        assert_eq!(fake.count("pr merge"), 0);
        // Not attended: refused, nothing read.
        let views_before = fake.count("pr view");
        assert!(merge_existing(&env(&s, &fake), &out.url, false).is_err());
        assert_eq!(fake.count("pr view"), views_before);
        // Ready now.
        fake.set_views(&[READY]);
        let merged = merge_existing(&env(&s, &fake), &out.url, true).unwrap();
        assert_eq!(merged.status, "merged");
        assert_eq!(fake.count("pr merge"), 1);
        let sha = bare_git(&s, &["rev-parse", &out.branch]).trim().to_string();
        assert!(fake
            .gh_calls()
            .iter()
            .any(|c| c.starts_with("pr merge") && c.ends_with(&sha)));
        let rows = ledger_lines(&s);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1]["event"], "merged");
        // Again, now that GitHub says MERGED: no second merge, no second line.
        fake.set_views(&[&READY.replace(r#""state":"OPEN""#, r#""state":"MERGED""#)]);
        let again = merge_existing(&env(&s, &fake), &out.url, true).unwrap();
        assert_eq!(again.status, "merged");
        assert_eq!(fake.count("pr merge"), 1);
        assert_eq!(ledger_lines(&s).len(), 2);
    }

    #[test]
    fn the_second_action_refuses_a_teammate_record_a_stranger_login_and_an_unknown_url() {
        let s = scratch();
        let fake = Fake::new("sam");
        let theirs = owner_propose(&s, &fake);
        assert_eq!(theirs.identity, "teammate");
        // Even with the owner signed in now, a teammate-record PR is refused.
        *fake.login.borrow_mut() = "theowner".into();
        let err = merge_existing(&env(&s, &fake), &theirs.url, true).unwrap_err();
        assert!(err.contains("teammate"), "{err}");
        assert!(merge_existing(&env(&s, &fake), "https://github.com/x/y/pull/9", true).is_err());
        // An owner record, but someone else signed in now.
        let fake2 = Fake::new("theowner");
        *fake2.pr_no.borrow_mut() = 21;
        fake2.set_views(&[&READY.replace("MERGEABLE", "UNKNOWN")]);
        let mine = owner_propose(&s, &fake2);
        *fake2.login.borrow_mut() = "sam".into();
        let err = merge_existing(&env(&s, &fake2), &mine.url, true).unwrap_err();
        assert!(err.contains("not the owner"), "{err}");
        assert_eq!(fake.count("pr merge") + fake2.count("pr merge"), 0);
    }

    #[test]
    fn a_teammate_identity_waits_with_no_merge_call() {
        let s = scratch();
        let fake = Fake::new("sam");
        let out = propose(
            &env(&s, &fake),
            &request(vec![("glossary.md", "# G\n\nA fact.\n")]),
        )
        .unwrap();
        assert_eq!(
            (out.identity.as_str(), out.status.as_str()),
            ("teammate", "waiting")
        );
        let calls = fake.gh_calls();
        assert!(
            !calls
                .iter()
                .any(|c| c.starts_with("pr merge") || c.starts_with("pr view")),
            "{calls:?}"
        );
        assert!(calls
            .iter()
            .any(|c| c.contains("--add-label needs owner approval")));
        assert_eq!(ledger_lines(&s)[0]["digest"], false);
    }

    #[test]
    fn a_secret_is_refused_before_any_git_or_gh_call() {
        let s = scratch();
        let fake = Fake::new("theowner");
        let key = format!("-----BEGIN {} KEY-----\nabc\n", "PRIVATE");
        let err = propose(&env(&s, &fake), &request(vec![("glossary.md", &key)])).unwrap_err();
        assert!(err.contains("secret.private_key"), "{err}");
        assert!(err.contains("glossary.md:1"), "{err}");
        assert!(!err.contains("abc"));
        assert_eq!(*fake.git_calls.borrow(), 0);
        assert!(fake.gh_calls().is_empty());
        assert!(!s.ledger.exists());
    }

    #[test]
    fn a_machine_fact_is_refused_naming_the_rule() {
        let s = scratch();
        let fake = Fake::new("theowner");
        let text = format!("Config lives in /Users/{}/Hollow.\n", "dev");
        let err = propose(&env(&s, &fake), &request(vec![("systems/x.md", &text)])).unwrap_err();
        assert!(err.contains("machine."), "{err}");
        assert_eq!(*fake.git_calls.borrow(), 0);
    }

    #[test]
    fn bad_paths_are_refused_before_any_call() {
        let s = scratch();
        let fake = Fake::new("theowner");
        for path in [
            "../x.md",
            "/etc/x.md",
            "systems/../../x.md",
            "systems/a.txt",
            "tools/x.md",
            ".github/x.md",
            "systems/.hidden.md",
            "README.md",
            "systems//x.md",
            "other/x.md",
            "systems/a:b.md",
            "CODEOWNERS",
        ] {
            let err = propose(&env(&s, &fake), &request(vec![(path, "# x\n")])).unwrap_err();
            assert!(err.contains("refused"), "{path}: {err}");
        }
        assert_eq!(*fake.git_calls.borrow(), 0);
        assert!(fake.gh_calls().is_empty());
        assert!(validate_path("systems/a-b.md").is_ok());
        assert!(validate_path("standards/x.yml").is_ok());
        assert!(validate_path("INDEX.md").is_ok());
    }

    #[test]
    fn index_over_two_kb_is_refused() {
        let s = scratch();
        let fake = Fake::new("theowner");
        let big = "- a line of index\n".repeat(150);
        assert!(big.len() > INDEX_CAP);
        let err = propose(&env(&s, &fake), &request(vec![("INDEX.md", &big)])).unwrap_err();
        assert!(err.contains("INDEX.md") && err.contains("2048"), "{err}");
        assert_eq!(*fake.git_calls.borrow(), 0);
        // Exactly 2048 bytes is refused; 2047 is not.
        let mut req = request(vec![("INDEX.md", &"x".repeat(INDEX_CAP - 1))]);
        assert!(validate(&req).is_err());
        req.files[0].content = "x".repeat(INDEX_CAP - 2);
        assert!(validate(&req).is_ok());
        let ok = "- a line of index\n".repeat(50);
        assert!(ok.len() < INDEX_CAP);
        propose(&env(&s, &fake), &request(vec![("INDEX.md", &ok)])).unwrap();
    }

    #[test]
    fn the_record_has_one_line_per_pr() {
        let s = scratch();
        let fake = Fake::new("sam");
        propose(
            &env(&s, &fake),
            &request(vec![("glossary.md", "# G\n\nOne.\n")]),
        )
        .unwrap();
        let fake2 = Fake::new("theowner");
        propose(
            &env(&s, &fake2),
            &request(vec![("goals/a.md", "# G\n\nTwo.\n")]),
        )
        .unwrap();
        let rows = ledger_lines(&s);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["identity"], "teammate");
        assert_eq!(rows[1]["identity"], "owner");
        assert!(rows[1]["url"].as_str().unwrap().starts_with("https://"));
        assert!(rows[1]["at"].as_str().unwrap().contains('T'));
    }

    #[test]
    fn codeowners_take_the_last_star_line_and_one_owner() {
        assert_eq!(
            codeowner("# c\n* @example-user\n").as_deref(),
            Some("example-user")
        );
        assert_eq!(codeowner("* @First\n* @Last\n").as_deref(), Some("Last"));
        assert_eq!(codeowner("* @A @B\n"), None);
        assert_eq!(codeowner("docs/ @x\n"), None);
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("docs")).unwrap();
        std::fs::create_dir_all(dir.path().join(".github")).unwrap();
        std::fs::write(dir.path().join("docs/CODEOWNERS"), "* @Docs\n").unwrap();
        assert_eq!(repo_owner(dir.path()).as_deref(), Some("Docs"));
        std::fs::write(dir.path().join("CODEOWNERS"), "* @Root\n").unwrap();
        assert_eq!(repo_owner(dir.path()).as_deref(), Some("Root"));
        std::fs::write(dir.path().join(".github/CODEOWNERS"), "* @Gh\n").unwrap();
        assert_eq!(repo_owner(dir.path()).as_deref(), Some("Gh"));
    }

    #[test]
    fn the_merge_rules_hold_on_a_rollup() {
        let v = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        let open = |extra: &str| {
            v(&format!(
                r#"{{"state":"OPEN","isDraft":false,"mergeable":"MERGEABLE",{extra}}}"#
            ))
        };
        let req = |n: &str| Required {
            name: n.into(),
            run_only: false,
        };
        let empty = open(r#""statusCheckRollup":[]"#);
        assert_eq!(judge(&empty, &[], false), Step::Go);
        // Workflows in the repo: an empty rollup is "not started", never Go.
        assert!(matches!(judge(&empty, &[], true), Step::Wait(_)));
        assert!(matches!(judge(&empty, &[req("x")], false), Step::Wait(_)));
        assert!(matches!(
            judge(
                &v(r#"{"state":"OPEN","isDraft":true,"mergeable":"MERGEABLE"}"#),
                &[],
                false
            ),
            Step::Refuse(_)
        ));
        assert!(matches!(
            judge(
                &v(r#"{"state":"OPEN","isDraft":false,"mergeable":"CONFLICTING"}"#),
                &[],
                false
            ),
            Step::Refuse(_)
        ));
        assert!(matches!(
            judge(
                &v(r#"{"state":"OPEN","isDraft":false,"mergeable":"UNKNOWN"}"#),
                &[],
                false
            ),
            Step::Wait(_)
        ));
        // A present check must be final and passing even when none is required.
        assert!(matches!(
            judge(
                &open(r#""statusCheckRollup":[{"status":"COMPLETED","conclusion":"FAILURE"}]"#),
                &[],
                false
            ),
            Step::Refuse(_)
        ));
        assert!(matches!(
            judge(
                &open(r#""statusCheckRollup":[{"status":"QUEUED"}]"#),
                &[],
                false
            ),
            Step::Wait(_)
        ));
        assert!(matches!(
            judge(
                &open(r#""statusCheckRollup":[{"context":"s","state":"PENDING"}]"#),
                &[],
                false
            ),
            Step::Wait(_)
        ));
        // A required check that only went neutral did not succeed.
        assert!(matches!(
            judge(
                &open(
                    r#""statusCheckRollup":[{"name":"r","status":"COMPLETED","conclusion":"SKIPPED"}]"#
                ),
                &[req("r")],
                false
            ),
            Step::Refuse(_)
        ));
        assert_eq!(
            judge(&v(r#"{"state":"MERGED"}"#), &[], false),
            Step::AlreadyMerged
        );
    }

    #[test]
    fn a_required_check_with_an_app_needs_a_check_run_not_a_status() {
        let v = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        let open = |rollup: &str| {
            v(&format!(
                r#"{{"state":"OPEN","isDraft":false,"mergeable":"MERGEABLE","statusCheckRollup":[{rollup}]}}"#
            ))
        };
        let need = [Required {
            name: "ci".into(),
            run_only: true,
        }];
        let status = open(r#"{"context":"ci","state":"SUCCESS"}"#);
        assert!(matches!(judge(&status, &need, false), Step::Wait(_)));
        let run = open(r#"{"name":"ci","status":"COMPLETED","conclusion":"SUCCESS"}"#);
        assert_eq!(judge(&run, &need, false), Step::Go);
    }

    #[test]
    fn only_the_latest_rollup_entry_per_name_counts() {
        let v = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        let old_fail = r#"{"name":"ci","status":"COMPLETED","conclusion":"FAILURE","completedAt":"2026-11-15T10:00:00Z"}"#;
        let new_ok = r#"{"name":"ci","status":"COMPLETED","conclusion":"SUCCESS","completedAt":"2026-11-15T11:00:00Z"}"#;
        for order in [
            format!("{old_fail},{new_ok}"),
            format!("{new_ok},{old_fail}"),
        ] {
            let pr = v(&format!(
                r#"{{"state":"OPEN","isDraft":false,"mergeable":"MERGEABLE","statusCheckRollup":[{order}]}}"#
            ));
            assert_eq!(judge(&pr, &[], false), Step::Go, "{order}");
        }
    }

    #[test]
    fn ruleset_required_checks_count_and_a_rules_error_stops_the_merge() {
        let s = scratch();
        let fake = Fake::new("theowner");
        *fake.rules.borrow_mut() = Ok(r#"[{"type":"deletion"},{"type":"required_status_checks","parameters":{"required_status_checks":[{"context":"ci"}]}}]"#.into());
        let out = owner_propose(&s, &fake);
        assert_eq!(out.status, "waiting");
        assert!(out.reason.contains("required check ci"), "{}", out.reason);
        assert_eq!(fake.count("pr merge"), 0);
        let with_ci = rollup(
            r#"{"__typename":"CheckRun","name":"ci","status":"COMPLETED","conclusion":"SUCCESS"}"#,
        );
        fake.set_views(&[&with_ci]);
        assert_eq!(owner_propose(&s, &fake).status, "merged");
        // Any failure but a 404 stops it.
        let fake = Fake::new("theowner");
        *fake.rules.borrow_mut() = Err("gh: HTTP 500".into());
        let out = owner_propose(&s, &fake);
        assert_eq!(out.status, "waiting");
        assert!(out.reason.contains("branch rules"), "{}", out.reason);
        assert_eq!(fake.count("pr merge"), 0);
        assert!(fake
            .gh_calls()
            .iter()
            .any(|c| c.contains("/rules/branches/main")));
    }

    #[test]
    fn a_repo_with_workflows_and_an_empty_rollup_waits() {
        let s = scratch_with(true);
        let fake = Fake::new("theowner");
        fake.set_views(&[&rollup("")]);
        let out = owner_propose(&s, &fake);
        assert_eq!(out.status, "waiting");
        assert!(out.reason.contains("workflows"), "{}", out.reason);
        assert_eq!(fake.count("pr view"), MERGE_TRIES);
        assert_eq!(fake.count("pr merge"), 0);
    }

    #[test]
    fn the_second_action_refuses_another_repo_before_any_gh_call() {
        let s = scratch();
        let fake = Fake::new("theowner");
        for url in [
            "https://github.com/other/repo/pull/7",
            "https://github.com/example/org-context-evil/pull/7",
            "https://example.com/example/org-context/pull/7",
            "https://github.com/example/org-context/issues/7",
        ] {
            let err = merge_existing(&env(&s, &fake), url, true).unwrap_err();
            assert!(
                err.contains("org repo") || err.contains("not a GitHub"),
                "{err}"
            );
        }
        assert!(fake.gh_calls().is_empty());
        // A record that names another repo for the same URL is refused too.
        let fake = Fake::new("theowner");
        fake.set_views(&[&READY.replace("MERGEABLE", "UNKNOWN")]);
        let out = owner_propose(&s, &fake);
        let text = std::fs::read_to_string(&s.ledger).unwrap();
        std::fs::write(
            &s.ledger,
            text.replace(
                "\"repo\":\"example/org-context\"",
                "\"repo\":\"other/repo\"",
            ),
        )
        .unwrap();
        let err = merge_existing(&env(&s, &fake), &out.url, true).unwrap_err();
        assert!(err.contains("different repo"), "{err}");
        assert_eq!(fake.count("pr merge"), 0);
    }

    #[test]
    fn urls_remotes_and_branches_parse_and_encode() {
        assert_eq!(
            parse_pr_url("https://github.com/a/b/pull/12"),
            Some(("a".into(), "b".into(), 12))
        );
        assert_eq!(parse_pr_url("https://github.com/a/b/pull/x"), None);
        assert_eq!(parse_pr_url("https://github.com/a/b/pull/1/files"), None);
        assert_eq!(parse_pr_url("https://github.com/../b/pull/1"), None);
        for r in [
            "https://github.com/a/b.git",
            "git@github.com:a/b.git",
            "ssh://git@github.com/a/b",
            "https://github.com/a/b/",
        ] {
            assert_eq!(parse_remote_repo(r).as_deref(), Some("a/b"), "{r}");
        }
        assert_eq!(parse_remote_repo("/tmp/origin.git"), None);
        assert_eq!(pct("release/v1 x"), "release%2Fv1%20x");
        assert_eq!(pct("main"), "main");
    }

    #[test]
    fn a_merged_record_gets_no_second_merged_line() {
        let s = scratch();
        let fake = Fake::new("theowner");
        let out = owner_propose(&s, &fake);
        assert_eq!(out.status, "merged");
        fake.set_views(&[&READY.replace(r#""state":"OPEN""#, r#""state":"MERGED""#)]);
        let again = merge_existing(&env(&s, &fake), &out.url, true).unwrap();
        assert_eq!(again.status, "merged");
        assert_eq!(ledger_lines(&s).len(), 1);
        assert_eq!(fake.count("pr merge"), 1);
    }

    #[test]
    fn the_org_refusal_text_names_this_action() {
        let err = memory_guard::check_layer(
            memory_guard::WriteLayer::Org,
            None,
            "glossary.md",
            &[("text", "x")],
        )
        .unwrap_err();
        assert!(err.message().contains("context.propose_org_change"));
    }
}
