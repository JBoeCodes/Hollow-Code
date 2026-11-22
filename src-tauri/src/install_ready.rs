//! Install readiness (overnight E2): the facts to approve an install of
//! `origin/main` with. Read-only: it never builds or installs.
//!
//! - **Provenance.** Each `npm run build` (part of `npm run gate`) and each
//!   cargo build (the crate's build script runs the same script) writes
//!   `src-tauri/build-provenance.json` in its checkout: the commit, whether
//!   the tree was dirty, and a fingerprint of the build inputs
//!   (`scripts/source-provenance.mjs`). Among the Hollow checkout and its
//!   worktrees, the newest provenance naming `origin/main`'s commit is
//!   checked: clean, and its fingerprint equal to one recomputed from
//!   `origin/main`'s own tree (the same inputs and hash, read from git). That
//!   says a build ran on exactly this tree; it doesn't by itself say the
//!   whole gate passed (the file is written when the build starts).
//! - **Gate records** (overnight 10-12/13, S12). A passing `npm run gate`
//!   ends with `scripts/gate-record.mjs`, which keeps the provenance in
//!   `<git common dir>/hollow-gate-records/<fingerprint>.json`: shared by the
//!   worktrees, never committed, and still there once the gated worktree is
//!   removed. Looked up by origin/main's own fingerprint, so a gated branch
//!   counts for the merge commit that carries exactly its files, and it
//!   stands for the CSS token check and the bundle budget (the gate ran both
//!   on those files). Worktree cleanup saves a clean build's provenance there
//!   too (`gate: "build"`, [`save_build_record`]): proof of the files, not of
//!   the checks.
//! - **CSS tokens and bundle budget.** `scripts/check-css-tokens.mjs` and
//!   `scripts/check-bundle-budget.mjs`, run with node in that checkout (they
//!   only read files: the sources, and `dist/`). Only while the checkout is
//!   still exactly origin/main (HEAD equal, nothing changed); the budget only
//!   when `dist/` is at least as new as the build record (a cargo build
//!   rewrites provenance without a frontend build). Otherwise skipped, with
//!   the reason.
//! - **CI.** `gh api …/commits/<sha>/check-runs` for the commit (100 a page):
//!   each check's name, status and conclusion; anything but success, neutral
//!   or skipped counts as not passed. A read request to GitHub, nothing more.
//!   CI is advisory for this repository, so a failure is reported, not a veto.
//!
//! **Repository** (fix chat 10-20). `hollow_repo::for_this_mac`: the last
//! install receipt's build workspace repository, else the build workspaces'
//! Hollow checkouts (live, newest, holding the installed commit first);
//! `repo` and `repoSource` name it. When its `origin/main` doesn't contain the
//! installed commit (the newest finished installer journal, else this
//! binary's), that is a reason and `ready` is false: it may be another
//! checkout, or installing its `origin/main` would go backwards. When the
//! installed commit is unknown, that is said first in `reasons` but doesn't
//! hold `ready` false: nothing contradicts the checkout, and readiness is
//! about `origin/main`'s own build (a Mac with no finished install journal
//! and an unidentified binary could otherwise never be ready).
//!
//! `ready` is true when the provenance matches cleanly and both checks pass;
//! every reason it isn't is listed in words.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    pub checkout: String,
    pub generated_at: String,
    pub dirty: bool,
    /// Its fingerprint equals the one recomputed from the target's tree.
    pub fingerprint_matches: bool,
    /// From a gate record: `npm run gate` passed on these build inputs.
    pub gated: bool,
}

/// Where passing gates are recorded, inside the common git directory.
pub(crate) const RECORDS_DIR: &str = "hollow-gate-records";

fn is_fingerprint(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// `<git common dir>/hollow-gate-records` for `repo` (any of its worktrees).
pub(crate) fn records_dir(repo: &Path) -> Option<PathBuf> {
    let common = git(repo, &["rev-parse", "--git-common-dir"])?;
    Some(repo.join(common).join(RECORDS_DIR))
}

/// The record for `fingerprint` in `dir`: clean, and naming the same
/// fingerprint inside as its file name says.
pub fn record_for(dir: &Path, fingerprint: &str) -> Option<Provenance> {
    if !is_fingerprint(fingerprint) {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join(format!("{fingerprint}.json"))).ok()?,
    )
    .ok()?;
    if v.get("sourceFingerprint")?.as_str()? != fingerprint
        || v.get("dirty").and_then(|d| d.as_bool()) != Some(false)
    {
        return None;
    }
    let text = |k: &str| v.get(k).and_then(|s| s.as_str()).unwrap_or("").to_string();
    Some(Provenance {
        checkout: text("checkout"),
        generated_at: text("generatedAt"),
        dirty: false,
        fingerprint_matches: true,
        gated: v.get("gate").and_then(|g| g.as_str()) == Some("passed"),
    })
}

/// Before a worktree is removed: keep its clean build's provenance in `dir`
/// unless that fingerprint is recorded already (a gate record is never
/// replaced). `Ok(true)` when it wrote one.
pub(crate) fn save_build_record(checkout: &Path, dir: &Path) -> Result<bool, String> {
    let Ok(text) = std::fs::read_to_string(checkout.join("src-tauri/build-provenance.json")) else {
        return Ok(false);
    };
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Ok(false);
    };
    let fp = v
        .get("sourceFingerprint")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    if !is_fingerprint(&fp) || v.get("dirty").and_then(|d| d.as_bool()) != Some(false) {
        return Ok(false);
    }
    let path = dir.join(format!("{fp}.json"));
    if path.exists() {
        return Ok(false);
    }
    v["gate"] = serde_json::json!("build");
    v["recordedAt"] = serde_json::json!(chrono::Utc::now().to_rfc3339());
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!("{fp}.json.{}.tmp", std::process::id()));
    std::fs::write(&tmp, format!("{v:#}\n")).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(true)
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    /// `passed` | `failed` | `skipped`.
    pub outcome: String,
    /// The tool's last lines, or why it was skipped.
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CiRun {
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// The Hollow checkout read (see [`crate::hollow_repo`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// How `repo` was found (`installReceipt` | `buildWorkspace`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_source: Option<crate::hollow_repo::RepoSource>,
    pub target: String,
    pub target_commit: Option<String>,
    pub provenance: Option<Provenance>,
    pub css_tokens: Check,
    pub bundle_budget: Check,
    /// `None` when CI couldn't be asked (see `ci_note`).
    pub ci: Option<Vec<CiRun>>,
    pub ci_note: Option<String>,
    pub ready: bool,
    pub reasons: Vec<String>,
}

/// The build inputs `source-provenance.mjs` hashes (same pattern).
fn is_input(path: &str) -> bool {
    use once_cell::sync::Lazy;
    use regex::Regex;
    static INPUT: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"^(src/|src-tauri/src/|scripts/|public/|package(-lock)?\.json$|index\.html$|vite\.config\.ts$|tsconfig\.json$|rust-toolchain\.toml$|docs/(WHATS_NEW\.md|HOLLOW_APP_COMMAND_MAP\.tsv)$|src-tauri/(Cargo\.(toml|lock)|tauri(\.[a-z-]+)?\.conf\.json|build\.rs|capabilities/|icons/))").unwrap()
    });
    INPUT.is_match(path)
}

/// Hollow's hardened git ([`crate::build_workspaces::git_command`]: `-C
/// repo`, `GIT_*` stripped, the pinned `SUBMODULE_SAFE` flags, a linked
/// worktree pinned from its `.git` file). The checkout read here comes from
/// `src-tauri/build-provenance.json`, which an agent can write, so a planted
/// submodule entry there must run nothing (overnight 11-13 review).
fn git_bytes(repo: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let out = crate::build_workspaces::git_command(repo)
        .into_std()
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status.success().then_some(out.stdout)
}

fn git(repo: &Path, args: &[&str]) -> Option<String> {
    git_bytes(repo, args).map(|b| String::from_utf8_lossy(&b).trim().to_string())
}

/// Whether `checkout` has no change: `dirty`, so a submodule's new commit
/// still counts while git never enters a submodule (a planted `.gitmodules`
/// `ignore = none` can't override a command-line option).
fn checkout_clean(checkout: &Path) -> bool {
    git(
        checkout,
        &["status", "--porcelain", "--ignore-submodules=dirty"],
    )
    .is_some_and(|s| s.is_empty())
}

/// `source-provenance.mjs`'s fingerprint for `commit`'s own tree: every input
/// path in order, `path \0 bytes \0`, SHA-256. (A clean checkout of the
/// commit hashes the same files; checked against the script's own output.)
/// One `git cat-file --batch` reads every blob. Symlinks and submodules would
/// differ from the script (it follows links); the tree has neither.
pub fn fingerprint_of(repo: &Path, commit: &str) -> Option<String> {
    use std::io::{BufRead, BufReader, Read, Write};
    let list = git(repo, &["ls-tree", "-r", "--name-only", "-z", commit])?;
    let mut paths: Vec<&str> = list
        .split('\0')
        .filter(|p| !p.is_empty() && is_input(p))
        .collect();
    paths.sort_unstable();
    paths.dedup();
    let mut child = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["cat-file", "--batch"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let requests: String = paths.iter().map(|p| format!("{commit}:{p}\n")).collect();
    let writer = std::thread::spawn(move || {
        if let Some(mut stdin) = stdin {
            let _ = stdin.write_all(requests.as_bytes());
        }
    });
    let hashed = (|| {
        let mut out = BufReader::new(stdout?);
        let mut hash = Sha256::new();
        for p in &paths {
            let mut header = String::new();
            out.read_line(&mut header).ok()?;
            // `<sha> blob <size>`; anything else (missing) fails the fingerprint.
            let size: usize = header.split_whitespace().nth(2)?.parse().ok()?;
            let mut bytes = vec![0u8; size + 1];
            out.read_exact(&mut bytes).ok()?;
            bytes.truncate(size);
            hash.update(p.as_bytes());
            hash.update([0]);
            hash.update(&bytes);
            hash.update([0]);
        }
        Some(
            hash.finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
        )
    })();
    // Always reaped, an early return included.
    let _ = child.kill();
    let _ = child.wait();
    let _ = writer.join();
    hashed
}

/// The checkout and its worktrees.
fn checkouts(repo: &Path) -> Vec<PathBuf> {
    let mut out = vec![repo.to_path_buf()];
    if let Some(list) = git(repo, &["worktree", "list", "--porcelain"]) {
        for l in list.lines() {
            if let Some(p) = l.strip_prefix("worktree ") {
                let p = PathBuf::from(p);
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
    }
    out
}

/// The newest provenance among `checkouts` naming `commit`, checked against
/// `expected` (the target tree's fingerprint).
pub fn provenance_for(
    checkouts: &[PathBuf],
    commit: &str,
    expected: Option<&str>,
) -> Option<Provenance> {
    let mut found: Vec<(String, Provenance)> = checkouts
        .iter()
        .filter_map(|c| {
            let v: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(c.join("src-tauri/build-provenance.json")).ok()?,
            )
            .ok()?;
            (v.get("commit")?.as_str()? == commit).then_some(())?;
            let generated = v
                .get("generatedAt")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let fp = v
                .get("sourceFingerprint")
                .and_then(|s| s.as_str())
                .unwrap_or("");
            Some((
                generated.clone(),
                Provenance {
                    checkout: c.to_string_lossy().to_string(),
                    generated_at: generated,
                    dirty: v.get("dirty").and_then(|d| d.as_bool()).unwrap_or(true),
                    fingerprint_matches: expected.is_some_and(|e| e == fp),
                    gated: false,
                },
            ))
        })
        .collect();
    // The newest clean, matching build if there is one, else the newest.
    found.sort_by(|a, b| a.0.cmp(&b.0));
    let good = found
        .iter()
        .rposition(|(_, p)| !p.dirty && p.fingerprint_matches);
    match good {
        Some(i) => Some(found.swap_remove(i).1),
        None => found.pop().map(|(_, p)| p),
    }
}

/// Runs a node script in a checkout: `(passed, last lines)`.
pub type NodeRunner<'a> = &'a dyn Fn(&Path, &str) -> Result<(bool, String), String>;
/// Asks CI about a commit of `owner/repo`.
pub type CiAsker<'a> = &'a dyn Fn(&str, &str) -> Result<Vec<CiRun>, String>;

fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(3)..].join("\n")
}

/// `owner/repo` from a GitHub remote URL.
pub fn github_slug(url: &str) -> Option<String> {
    let rest = url
        .trim()
        .strip_prefix("git@github.com:")
        .or_else(|| url.trim().split_once("github.com/").map(|(_, r)| r))?;
    let slug = rest.trim_end_matches('/').trim_end_matches(".git");
    (slug.split('/').count() == 2).then(|| slug.to_string())
}

/// `dist/assets` was written at or after the build record (`generated_at`).
fn dist_is_fresh(checkout: &Path, generated_at: &str) -> bool {
    let Ok(at) = chrono::DateTime::parse_from_rfc3339(generated_at) else {
        return false;
    };
    std::fs::metadata(checkout.join("dist/assets"))
        .and_then(|m| m.modified())
        .map(|m| chrono::DateTime::<chrono::Utc>::from(m) >= at.with_timezone(&chrono::Utc))
        .unwrap_or(false)
}

/// The report for `repo`, with the slow and external parts injected.
pub fn report(repo: Option<&Path>, node: NodeRunner, ci: CiAsker) -> Report {
    let skipped = |why: &str| Check {
        outcome: "skipped".into(),
        detail: why.into(),
    };
    let mut r = Report {
        repo: repo.map(|p| p.to_string_lossy().to_string()),
        repo_source: None,
        target: "origin/main".into(),
        target_commit: None,
        provenance: None,
        css_tokens: skipped("No clean build of origin/main was found."),
        bundle_budget: skipped("No clean build of origin/main was found."),
        ci: None,
        ci_note: None,
        ready: false,
        reasons: Vec::new(),
    };
    let Some(repo) = repo else {
        r.reasons
            .push("No Hollow checkout is known (no build workspace was cut from one).".into());
        return r;
    };
    let Some(commit) = git(repo, &["rev-parse", "--verify", "origin/main"]) else {
        r.reasons.push("This checkout has no origin/main.".into());
        return r;
    };
    r.target_commit = Some(commit.clone());
    let expected = fingerprint_of(repo, &commit);
    let recorded = expected
        .as_deref()
        .zip(records_dir(repo))
        .and_then(|(fp, dir)| record_for(&dir, fp));
    let built = provenance_for(&checkouts(repo), &commit, expected.as_deref());
    // A gate record is the strongest proof; then a clean matching build in a
    // checkout; then a build record kept at cleanup.
    r.provenance = match (recorded, built) {
        (Some(g), _) if g.gated => Some(g),
        (_, Some(b)) if !b.dirty && b.fingerprint_matches => Some(b),
        (Some(g), _) => Some(g),
        (None, b) => b,
    };
    match &r.provenance {
        None => r.reasons.push(format!(
            "No checkout has built {} (no build-provenance.json names it, and no gate record has its files): run the gate on origin/main first.",
            &commit[..commit.len().min(8)]
        )),
        Some(p) if p.dirty => r.reasons.push(format!("The build of it in {} had uncommitted changes.", p.checkout)),
        Some(p) if !p.fingerprint_matches => r.reasons.push(format!(
            "The build in {} doesn't match origin/main's files (its inputs differ).",
            p.checkout
        )),
        Some(p) if p.gated => {
            let passed = Check {
                outcome: "passed".into(),
                detail: format!(
                    "Passed in the full gate (npm run gate) on exactly these build inputs, in {} at {}.",
                    p.checkout, p.generated_at
                ),
            };
            r.css_tokens = passed.clone();
            r.bundle_budget = passed;
        }
        Some(p) => {
            let checkout = PathBuf::from(&p.checkout);
            // The checks read the checkout as it is now: only meaningful while
            // it is still exactly origin/main, with nothing changed.
            let head = git(&checkout, &["rev-parse", "HEAD"]);
            let clean = checkout_clean(&checkout);
            if !checkout.is_dir() {
                let why = format!("{} was removed after that build, so the checks can't run: run the gate on these files again.", p.checkout);
                r.css_tokens = skipped(&why);
                r.bundle_budget = skipped(&why);
            } else if head.as_deref() != Some(commit.as_str()) || !clean {
                let why = format!("{} has moved on or changed since that build, so the checks would test another tree.", p.checkout);
                r.css_tokens = skipped(&why);
                r.bundle_budget = skipped(&why);
            } else {
                let run = |script: &str| match node(&checkout, script) {
                    Ok((true, out)) => Check { outcome: "passed".into(), detail: tail(&out) },
                    Ok((false, out)) => Check { outcome: "failed".into(), detail: tail(&out) },
                    Err(e) => Check { outcome: "skipped".into(), detail: e },
                };
                r.css_tokens = run("scripts/check-css-tokens.mjs");
                r.bundle_budget = if dist_is_fresh(&checkout, &p.generated_at) {
                    run("scripts/check-bundle-budget.mjs")
                } else {
                    skipped("dist/ is older than that build record (a cargo build, or a frontend build that stopped early): run npm run build there to measure it.")
                };
            }
        }
    }
    for (name, c) in [
        ("CSS token check", &r.css_tokens),
        ("Bundle budget", &r.bundle_budget),
    ] {
        if c.outcome == "failed" {
            r.reasons.push(format!(
                "{name} failed: {}",
                c.detail.lines().last().unwrap_or("")
            ));
        } else if c.outcome == "skipped"
            && r.provenance
                .as_ref()
                .is_some_and(|p| !p.dirty && p.fingerprint_matches)
        {
            r.reasons.push(format!("{name} couldn't run: {}", c.detail));
        }
    }
    match git(repo, &["remote", "get-url", "origin"])
        .as_deref()
        .and_then(github_slug)
    {
        None => r.ci_note = Some("origin isn't a GitHub remote, so CI wasn't asked.".into()),
        Some(slug) => match ci(&slug, &commit) {
            Ok(runs) => {
                let failed: Vec<&str> = runs
                    .iter()
                    .filter(|c| {
                        c.status == "completed"
                            && !matches!(
                                c.conclusion.as_deref(),
                                Some("success" | "neutral" | "skipped")
                            )
                    })
                    .map(|c| c.name.as_str())
                    .collect();
                let pending = runs.iter().filter(|c| c.status != "completed").count();
                r.ci_note = Some(if runs.is_empty() {
                    "CI reported no checks for this commit.".into()
                } else if !failed.is_empty() {
                    format!(
                        "CI failed: {} (advisory for this repository).",
                        failed.join(", ")
                    )
                } else if pending > 0 {
                    format!("{pending} CI check(s) still running.")
                } else {
                    "CI passed.".into()
                });
                r.ci = Some(runs);
            }
            Err(e) => r.ci_note = Some(format!("CI couldn't be asked: {e}")),
        },
    }
    r.ready = r
        .provenance
        .as_ref()
        .is_some_and(|p| !p.dirty && p.fingerprint_matches)
        && r.css_tokens.outcome == "passed"
        && r.bundle_budget.outcome == "passed";
    r
}

/// A tool on the build PATH (`self_install::tool_path`).
fn tool(name: &str) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let path = crate::self_install::tool_path(std::env::var("PATH").ok().as_deref(), &home);
    path.split(':')
        .map(|d| Path::new(d).join(name))
        .find(|p| p.is_file())
}

fn run_bounded(cmd: &mut std::process::Command, limit: Duration) -> Result<(bool, String), String> {
    use std::io::Read;
    let mut child = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("didn't start ({e})"))?;
    let read = |mut r: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = r.read_to_string(&mut s);
            s
        })
    };
    let out = read(Box::new(child.stdout.take().unwrap()));
    let err = read(Box::new(child.stderr.take().unwrap()));
    let started = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
            break s;
        }
        if started.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("took longer than {} s", limit.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let text = format!(
        "{}{}",
        out.join().unwrap_or_default(),
        err.join().unwrap_or_default()
    );
    Ok((status.success(), text))
}

/// The report against `found` (see [`crate::hollow_repo`]): its source, and
/// its warning the first reason. One saying origin/main lacks the installed
/// commit keeps `ready` false; one saying the installed commit is unknown
/// doesn't.
pub fn report_for_repo(
    found: Option<&crate::hollow_repo::HollowRepo>,
    node: NodeRunner,
    ci: CiAsker,
) -> Report {
    let mut r = report(found.map(|f| f.path.as_path()), node, ci);
    if let Some(f) = found {
        r.repo_source = Some(f.source);
        if let Some(w) = &f.warning {
            r.reasons.insert(0, w.clone());
            if !f.installed_unknown {
                r.ready = false;
            }
        }
    }
    r
}

/// `build.install_ready` on this Mac.
pub fn for_this_mac(vault: &Path) -> Report {
    let (installed, _) = crate::install_brings::installed_on_this_mac();
    let found = crate::hollow_repo::for_this_mac(vault, installed.as_deref());
    let node = |checkout: &Path, script: &str| -> Result<(bool, String), String> {
        let bin = tool("node").ok_or("node isn't installed where Hollow looks")?;
        run_bounded(
            std::process::Command::new(bin)
                .arg(script)
                .current_dir(checkout),
            Duration::from_secs(60),
        )
    };
    let ci = |slug: &str, sha: &str| -> Result<Vec<CiRun>, String> {
        let gh = tool("gh").ok_or("the GitHub CLI (gh) isn't installed")?;
        let (ok, out) = run_bounded(
            std::process::Command::new(gh).args([
                "api",
                &format!("repos/{slug}/commits/{sha}/check-runs?per_page=100"),
                "--jq",
                ".check_runs[] | [.name, .status, (.conclusion // \"\")] | @tsv",
            ]),
            Duration::from_secs(30),
        )?;
        if !ok {
            return Err(tail(&out));
        }
        Ok(out
            .lines()
            .filter_map(|l| {
                let mut f = l.split('\t');
                let name = f.next()?.to_string();
                let status = f.next()?.to_string();
                let conclusion = f.next().map(str::to_string).filter(|c| !c.is_empty());
                Some(CiRun {
                    name,
                    status,
                    conclusion,
                })
            })
            .collect())
    };
    report_for_repo(found.as_ref(), &node, &ci)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn sh(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A repo whose origin/main has inputs and a non-input, plus a GitHub remote.
    fn repo() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        sh(d, &["init", "-q", "-b", "main"]);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(d.join("src/a.ts"), "export const a = 1;\n").unwrap();
        std::fs::write(d.join("package.json"), "{}\n").unwrap();
        std::fs::write(d.join("README.md"), "not an input\n").unwrap();
        std::fs::write(
            d.join(".gitignore"),
            "src-tauri/build-provenance.json\ndist/\n",
        )
        .unwrap();
        sh(d, &["add", "."]);
        sh(d, &["commit", "-q", "-m", "base"]);
        let head = sh(d, &["rev-parse", "HEAD"]);
        sh(d, &["update-ref", "refs/remotes/origin/main", &head]);
        sh(
            d,
            &[
                "remote",
                "add",
                "origin",
                "git@github.com:example-org/hollow-code.git",
            ],
        );
        (dir, head)
    }

    fn fresh_dist(d: &Path) {
        std::fs::create_dir_all(d.join("dist/assets")).unwrap();
    }

    fn provenance(d: &Path, commit: &str, fp: &str, dirty: bool) {
        std::fs::create_dir_all(d.join("src-tauri")).unwrap();
        std::fs::write(
            d.join("src-tauri/build-provenance.json"),
            serde_json::json!({ "sourceFingerprint": fp, "commit": commit, "dirty": dirty, "generatedAt": "2026-10-10T08:00:00Z" }).to_string(),
        )
        .unwrap();
    }

    #[test]
    fn the_fingerprint_hashes_only_inputs_in_path_order_as_the_script_does() {
        let (dir, head) = repo();
        let fp = fingerprint_of(dir.path(), &head).unwrap();
        let mut h = Sha256::new();
        for (p, b) in [
            ("package.json", "{}\n"),
            ("src/a.ts", "export const a = 1;\n"),
        ] {
            h.update(p.as_bytes());
            h.update([0]);
            h.update(b.as_bytes());
            h.update([0]);
        }
        let want: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(fp, want, "README.md is not an input");
        assert!(is_input("src-tauri/tauri.conf.json") && is_input("src-tauri/capabilities/x.json"));
        assert!(is_input("rust-toolchain.toml"));
        assert!(is_input("docs/WHATS_NEW.md"));
        assert!(is_input("docs/HOLLOW_APP_COMMAND_MAP.tsv"));
        assert!(!is_input("docs/BUILD_LOG.md") && !is_input("src-tauri/target/x"));
    }

    #[test]
    fn ready_needs_a_clean_matching_build_and_both_checks() {
        let (dir, head) = repo();
        let d = dir.path();
        let fp = fingerprint_of(d, &head).unwrap();
        let pass = |_: &Path, s: &str| Ok((true, format!("{s}: ok")));
        let ci_ok = |slug: &str, _: &str| {
            assert_eq!(slug, "example-org/hollow-code");
            Ok(vec![CiRun {
                name: "gate".into(),
                status: "completed".into(),
                conclusion: Some("success".into()),
            }])
        };
        // Nothing built yet.
        let r = report(Some(d), &pass, &ci_ok);
        assert!(!r.ready);
        assert!(
            r.reasons[0].contains("No checkout has built"),
            "{:?}",
            r.reasons
        );
        assert_eq!(r.css_tokens.outcome, "skipped");
        // A clean build of exactly this tree, both checks passing, CI green.
        provenance(d, &head, &fp, false);
        fresh_dist(d);
        let r = report(Some(d), &pass, &ci_ok);
        assert!(r.ready, "{:?}", r.reasons);
        assert!(r.reasons.is_empty());
        assert_eq!(r.ci_note.as_deref(), Some("CI passed."));
        // A failing budget, and CI red (reported, not a veto on its own).
        let budget_fails = |_: &Path, s: &str| {
            Ok((
                !s.contains("bundle"),
                "bundle-budget: over\nApp chunk 360 KB > 340 KB".to_string(),
            ))
        };
        let ci_red = |_: &str, _: &str| {
            Ok(vec![CiRun {
                name: "smoke".into(),
                status: "completed".into(),
                conclusion: Some("failure".into()),
            }])
        };
        let r = report(Some(d), &budget_fails, &ci_red);
        assert!(!r.ready);
        assert!(
            r.reasons
                .iter()
                .any(|x| x.starts_with("Bundle budget failed: App chunk 360 KB > 340 KB")),
            "{:?}",
            r.reasons
        );
        assert_eq!(
            r.ci_note.as_deref(),
            Some("CI failed: smoke (advisory for this repository).")
        );
        // A dirty build, or one whose inputs differ, isn't proof.
        provenance(d, &head, &fp, true);
        assert!(report(Some(d), &pass, &ci_ok).reasons[0].contains("uncommitted changes"));
        provenance(d, &head, "other", false);
        assert!(
            report(Some(d), &pass, &ci_ok).reasons[0].contains("doesn't match origin/main's files")
        );
        // CI unreachable: said, not fatal.
        provenance(d, &head, &fp, false);
        let ci_down = |_: &str, _: &str| Err("network unreachable".to_string());
        let r = report(Some(d), &pass, &ci_down);
        assert!(r.ready);
        assert_eq!(
            r.ci_note.as_deref(),
            Some("CI couldn't be asked: network unreachable")
        );
    }

    fn record(d: &Path, fp: &str, body: serde_json::Value) {
        let dir = d.join(".git").join(RECORDS_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{fp}.json")), body.to_string()).unwrap();
    }

    #[test]
    fn a_gate_record_outlives_its_worktree_and_stands_for_the_gate() {
        let (dir, head) = repo();
        let d = dir.path();
        let fp = fingerprint_of(d, &head).unwrap();
        let never = |_: &Path, s: &str| -> Result<(bool, String), String> {
            panic!("ran {s}: the gate already did")
        };
        let ci_ok = |_: &str, _: &str| Ok(vec![]);
        // The gated branch commit differs from origin/main's merge commit and
        // its worktree is gone; only its files match.
        record(
            d,
            &fp,
            serde_json::json!({ "sourceFingerprint": fp, "commit": "b".repeat(40), "checkout": "/gone/lane",
                "generatedAt": "2026-10-13T06:00:00Z", "dirty": false, "gate": "passed", "recordedAt": "2026-10-13T06:20:00Z" }),
        );
        // It wins over a dirty build of the same commit in the checkout.
        provenance(d, &head, &fp, true);
        let r = report(Some(d), &never, &ci_ok);
        assert!(r.ready, "{:?}", r.reasons);
        let p = r.provenance.unwrap();
        assert!(p.gated && p.fingerprint_matches && !p.dirty);
        assert_eq!(p.checkout, "/gone/lane");
        assert_eq!(r.css_tokens.outcome, "passed");
        assert!(
            r.bundle_budget.detail.contains("full gate"),
            "{}",
            r.bundle_budget.detail
        );
        // A record only of a build (saved at cleanup) is proof of the files,
        // not of the checks: they can't run without the checkout.
        record(
            d,
            &fp,
            serde_json::json!({ "sourceFingerprint": fp, "commit": "b".repeat(40), "checkout": "/gone/lane",
                "generatedAt": "2026-10-13T06:00:00Z", "dirty": false, "gate": "build" }),
        );
        std::fs::remove_file(d.join("src-tauri/build-provenance.json")).unwrap();
        let pass = |_: &Path, _: &str| Ok((true, String::new()));
        let r = report(Some(d), &pass, &ci_ok);
        assert!(!r.ready);
        let p = r.provenance.unwrap();
        assert!(!p.gated && p.fingerprint_matches);
        assert!(
            r.css_tokens.detail.contains("was removed"),
            "{}",
            r.css_tokens.detail
        );
        assert_eq!(r.css_tokens.outcome, "skipped");
        // A record whose contents don't match its name, or a dirty one, is no proof.
        for bad in [
            serde_json::json!({ "sourceFingerprint": "c".repeat(64), "commit": "b", "dirty": false, "gate": "passed" }),
            serde_json::json!({ "sourceFingerprint": fp, "commit": "b", "dirty": true, "gate": "passed" }),
        ] {
            record(d, &fp, bad);
            let r = report(Some(d), &pass, &ci_ok);
            assert!(!r.ready);
            assert!(
                r.reasons[0].contains("No checkout has built"),
                "{:?}",
                r.reasons
            );
        }
    }

    #[test]
    fn cleanup_saves_a_clean_build_once_and_never_over_a_gate_record() {
        let (dir, head) = repo();
        let d = dir.path();
        let records = tempfile::tempdir().unwrap();
        let fp = "e".repeat(64);
        // Nothing to save: no provenance, or a dirty one.
        assert_eq!(save_build_record(d, records.path()), Ok(false));
        provenance(d, &head, &fp, true);
        assert_eq!(save_build_record(d, records.path()), Ok(false));
        provenance(d, &head, &fp, false);
        assert_eq!(save_build_record(d, records.path()), Ok(true));
        let path = records.path().join(format!("{fp}.json"));
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(saved["gate"], "build");
        assert_eq!(saved["commit"], head.as_str());
        assert_eq!(save_build_record(d, records.path()), Ok(false));
        std::fs::write(&path, r#"{"gate":"passed"}"#).unwrap();
        assert_eq!(save_build_record(d, records.path()), Ok(false));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"{"gate":"passed"}"#
        );
        // A fingerprint that isn't one never names a file.
        provenance(d, &head, "../../x", false);
        assert_eq!(save_build_record(d, records.path()), Ok(false));
    }

    #[test]
    fn checks_never_run_against_a_tree_other_than_the_build() {
        let (dir, head) = repo();
        let d = dir.path();
        let fp = fingerprint_of(d, &head).unwrap();
        let pass = |_: &Path, s: &str| Ok((true, format!("{s}: ok")));
        let ci_ok = |_: &str, _: &str| Ok(Vec::new());
        provenance(d, &head, &fp, false);
        // No dist/ newer than the record (a cargo build rewrote it): budget skipped, said.
        let r = report(Some(d), &pass, &ci_ok);
        assert_eq!(r.css_tokens.outcome, "passed");
        assert_eq!(r.bundle_budget.outcome, "skipped");
        assert!(r.bundle_budget.detail.contains("dist/ is older"));
        assert!(!r.ready);
        // The checkout moved on after the build: neither check runs.
        fresh_dist(d);
        std::fs::write(d.join("src/a.ts"), "export const a = 2;\n").unwrap();
        let r = report(Some(d), &pass, &ci_ok);
        assert_eq!(
            (
                r.css_tokens.outcome.as_str(),
                r.bundle_budget.outcome.as_str()
            ),
            ("skipped", "skipped")
        );
        assert!(
            r.reasons.iter().any(|x| x.contains("moved on or changed")),
            "{:?}",
            r.reasons
        );
        assert!(!r.ready);
        assert_eq!(
            r.ci_note.as_deref(),
            Some("CI reported no checks for this commit.")
        );
    }

    #[test]
    fn only_success_neutral_or_skipped_ci_counts_as_passed_and_clean_builds_win() {
        let (dir, head) = repo();
        let d = dir.path();
        let fp = fingerprint_of(d, &head).unwrap();
        provenance(d, &head, &fp, false);
        fresh_dist(d);
        let pass = |_: &Path, s: &str| Ok((true, format!("{s}: ok")));
        let ci = |_: &str, _: &str| {
            Ok(vec![
                CiRun {
                    name: "lint".into(),
                    status: "completed".into(),
                    conclusion: Some("neutral".into()),
                },
                CiRun {
                    name: "deploy".into(),
                    status: "completed".into(),
                    conclusion: Some("action_required".into()),
                },
            ])
        };
        assert_eq!(
            report(Some(d), &pass, &ci).ci_note.as_deref(),
            Some("CI failed: deploy (advisory for this repository).")
        );
        // A newer dirty build elsewhere doesn't hide an older clean, matching one.
        let other = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(other.path().join("src-tauri")).unwrap();
        std::fs::write(
            other.path().join("src-tauri/build-provenance.json"),
            serde_json::json!({ "sourceFingerprint": fp, "commit": head, "dirty": true, "generatedAt": "2026-10-10T09:00:00Z" }).to_string(),
        )
        .unwrap();
        let p = provenance_for(
            &[other.path().to_path_buf(), d.to_path_buf()],
            &head,
            Some(&fp),
        )
        .unwrap();
        assert!(!p.dirty && p.fingerprint_matches, "{p:?}");
    }

    // Fix chat 10-20: the checkout comes from hollow_repo, is named, and a
    // checkout whose origin/main lacks the installed commit is never ready.
    #[test]
    fn a_checkout_without_the_installed_commit_is_named_and_never_ready() {
        let (dir, _) = repo();
        let d = dir.path();
        std::fs::create_dir_all(d.join("src-tauri")).unwrap();
        std::fs::write(
            d.join("src-tauri/tauri.conf.json"),
            r#"{"identifier":"com.hollow.app"}"#,
        )
        .unwrap();
        sh(d, &["add", "."]);
        sh(d, &["commit", "-qm", "hollow"]);
        let head = sh(d, &["rev-parse", "HEAD"]);
        sh(d, &["update-ref", "refs/remotes/origin/main", &head]);
        let fp = fingerprint_of(d, &head).unwrap();
        provenance(d, &head, &fp, false);
        fresh_dist(d);
        let vault = tempfile::tempdir().unwrap();
        crate::build_workspaces::bind_repo_for_test(vault.path(), "ws", d, d);
        let pass = |_: &Path, s: &str| Ok((true, format!("{s}: ok")));
        let ci_ok = |_: &str, _: &str| Ok(Vec::new());
        let canon = d.canonicalize().unwrap().to_string_lossy().to_string();
        // The installed commit is on this origin/main: ready, and named.
        let found = crate::hollow_repo::hollow_repo_in(vault.path(), None, Some(&head));
        let r = report_for_repo(found.as_ref(), &pass, &ci_ok);
        assert!(r.ready, "{:?}", r.reasons);
        assert_eq!(r.repo.as_deref(), Some(canon.as_str()));
        assert_eq!(
            r.repo_source,
            Some(crate::hollow_repo::RepoSource::BuildWorkspace)
        );
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["repoSource"], "buildWorkspace");
        // It isn't: the warning leads the reasons and ready is false.
        let other = "1".repeat(40);
        let found = crate::hollow_repo::hollow_repo_in(vault.path(), None, Some(&other));
        let r = report_for_repo(found.as_ref(), &pass, &ci_ok);
        assert!(!r.ready);
        assert!(
            r.reasons[0].contains("may not be the repo this app was built from")
                && r.reasons[0].contains(&canon)
                && r.reasons[0].contains("11111111"),
            "{:?}",
            r.reasons
        );
        // The installed commit unknown: said first, but still ready.
        let found = crate::hollow_repo::hollow_repo_in(vault.path(), None, None);
        let r = report_for_repo(found.as_ref(), &pass, &ci_ok);
        assert!(r.ready, "{:?}", r.reasons);
        assert_eq!(r.reasons, [crate::hollow_repo::UNKNOWN_INSTALLED]);
        // No checkout known: no repo field.
        let r = report_for_repo(None, &pass, &ci_ok);
        assert!(r.repo.is_none() && serde_json::to_value(&r).unwrap().get("repo").is_none());
    }

    #[test]
    fn github_remotes_name_their_repository() {
        assert_eq!(
            github_slug("git@github.com:example-org/hollow-code.git").as_deref(),
            Some("example-org/hollow-code")
        );
        assert_eq!(
            github_slug("https://github.com/example-org/hollow-code").as_deref(),
            Some("example-org/hollow-code")
        );
        assert_eq!(github_slug("https://gitlab.com/x/y.git"), None);
    }

    /// Overnight 11-13 review: the checkout named by the (agent-writable)
    /// provenance file is read with the hardened git, so a planted gitlink,
    /// a decoy with a `core.fsmonitor` payload and a `.gitmodules`
    /// `ignore = none` run nothing, and the planted entry still reads as a
    /// change. Real git.
    #[test]
    fn the_clean_check_runs_no_planted_submodule_payload() {
        let git = |dir: &Path, args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(["-c", "user.email=t@example.com", "-c", "user.name=t"])
                .args(args)
                .current_dir(dir)
                .env_remove("GIT_DIR")
                .env_remove("GIT_WORK_TREE")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        let top = root_path.join("repo");
        std::fs::create_dir_all(&top).unwrap();
        git(&top, &["init", "-q", "-b", "main"]);
        std::fs::write(
            top.join(".gitmodules"),
            "[submodule \"decoy\"]\n\tpath = decoy\n\turl = ./decoy\n\tignore = none\n",
        )
        .unwrap();
        git(&top, &["add", ".gitmodules"]);
        git(&top, &["commit", "-q", "-m", "init"]);
        assert!(checkout_clean(&top));
        let decoy = top.join("decoy");
        std::fs::create_dir_all(&decoy).unwrap();
        git(&decoy, &["init", "-q", "."]);
        git(&decoy, &["commit", "-q", "--allow-empty", "-m", "d"]);
        let sha = git(&decoy, &["rev-parse", "HEAD"]);
        let ran = root_path.join("RAN");
        git(
            &decoy,
            &[
                "config",
                "core.fsmonitor",
                &format!("touch {}; true", ran.display()),
            ],
        );
        git(
            &top,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("160000,{sha},decoy"),
            ],
        );
        git(&top, &["commit", "-q", "-m", "planted"]);
        git(&top, &["status", "--porcelain"]);
        assert!(ran.exists(), "control: plain git status runs the payload");
        std::fs::remove_file(&ran).unwrap();
        assert!(checkout_clean(&top), "a committed submodule is no change");
        std::fs::write(decoy.join("f"), "x").unwrap();
        git(&decoy, &["add", "f"]);
        git(&decoy, &["commit", "-q", "-m", "moved"]);
        // The test's own git in the decoy runs its payload; clear that.
        let _ = std::fs::remove_file(&ran);
        assert!(!checkout_clean(&top), "a submodule's new commit counts");
        assert!(!ran.exists(), "the clean check ran the planted fsmonitor");
    }
}
