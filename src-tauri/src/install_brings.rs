//! What an install brings (overnight E1): the pull requests and commits
//! between the installed build and `origin/main`, each PR with its one-line
//! title and any "untested by hand" note from its build-log entry. Read-only:
//! no build, no install, and no fetch (it reads the local `origin/main` as of
//! the last fetch, and says when that was).
//!
//! **Installed commit.** The newest installer journal in a finished state
//! (`verified`, `healthy`, `running-mismatch`, as `install_targets` reads
//! them); else this binary's own build identity.
//!
//! **Repository.** `hollow_repo::for_this_mac`: the last install receipt's
//! build workspace repository, else the build workspaces' Hollow checkouts
//! (live, newest, holding the installed commit first). `repoSource` says
//! which; when that checkout's `origin/main` lacks the installed commit, or
//! the installed commit is unknown, the note says so.
//!
//! **What is listed.** `git log --first-parent installed..origin/main`:
//! merge commits titled `Merge #N: …` or `Merge pull request #N …` are pull
//! requests, the rest direct commits. A PR's notes are the lines of its
//! `docs/BUILD_LOG.md` entry (as on `origin/main`) that say something wasn't
//! checked by hand or run, matched by the entry heading's `(overnight X)` tag
//! or the PR title.

use serde::Serialize;
use std::path::Path;

/// Rows kept, newest first.
const LIMIT: usize = 120;
/// Notes kept per PR.
const NOTES: usize = 3;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub merge: String,
    pub merged_at: String,
    /// What its build-log entry says wasn't checked by hand or run.
    pub untested: Vec<String>,
    /// Whether its build-log entry was found (empty notes mean "none said").
    pub entry_found: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    pub sha: String,
    pub subject: String,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub missing: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build_id: Option<String>,
    pub installed: Option<String>,
    /// `journal` | `binary`.
    pub installed_from: String,
    pub repo: Option<String>,
    /// How `repo` was found (`installReceipt` | `buildWorkspace`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_source: Option<crate::hollow_repo::RepoSource>,
    pub target: String,
    pub target_commit: Option<String>,
    /// When the local `origin/main` was last fetched (its reflog), if known.
    pub fetched_at: Option<String>,
    pub pull_requests: Vec<PullRequest>,
    pub commits: Vec<Commit>,
    /// Why the list is empty or partial, in a sentence.
    pub note: Option<String>,
}

const INSTALLED_STATES: &[&str] = &["verified", "healthy", "running-mismatch"];

/// The newest finished install's commit, from the installer journals in `dir`.
pub fn installed_from_journals(dir: &Path) -> Option<String> {
    let mut found: Vec<(String, String)> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| {
            let v: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&p).ok()?).ok()?;
            let state = v.get("state")?.as_str()?;
            INSTALLED_STATES.contains(&state).then_some(())?;
            Some((
                v.get("startedAt")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .to_string(),
                v.get("commit")?
                    .as_str()
                    .filter(|c| !c.trim().is_empty())?
                    .to_string(),
            ))
        })
        .collect();
    found.sort();
    found.pop().map(|(_, c)| c)
}

fn git(repo: &Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

/// `Merge #451: Title` / `Merge pull request #451 from o/b` (title in the body).
pub fn pr_of(subject: &str, body: &str) -> Option<(u64, String)> {
    if let Some(rest) = subject.strip_prefix("Merge #") {
        let (n, title) = rest.split_once(':')?;
        return Some((n.trim().parse().ok()?, title.trim().to_string()));
    }
    let rest = subject.strip_prefix("Merge pull request #")?;
    let n: u64 = rest.split_whitespace().next()?.parse().ok()?;
    let title = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or(subject);
    Some((n, title.to_string()))
}

/// Sentences of a build-log entry that say something wasn't checked by hand
/// or wasn't run. The log wraps its lines, so bullets and paragraphs are
/// joined first, then split into sentences.
fn untested_lines(entry: &str) -> Vec<String> {
    use once_cell::sync::Lazy;
    use regex::Regex;
    // A sentence (after a "Label:") that starts by saying something wasn't
    // checked, or says it in a negated past ("wasn't run"); "may not run" is a
    // rule, not a note.
    static STARTS: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)^(not (yet )?(checked|tried|tested|run|verified|exercised|measured)|untested|nobody (tried|checked)|never (tried|run|tested)|no (dev server|test bed|live screen reader))\b").unwrap()
    });
    static NEGATED: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)\b(wasn't|weren't|isn't|aren't|didn't|hasn't been|haven't been|was not|were not) (run|checked|tried|tested|exercised|measured)\b").unwrap()
    });
    static LABEL: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[A-Z][A-Za-z ]{0,24}:\s+").unwrap());
    // Logical items: a bullet with its indented continuation, or a paragraph.
    let mut items: Vec<String> = Vec::new();
    for line in entry.lines().skip(1) {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            items.push(String::new());
            continue;
        }
        let bullet = t.starts_with("- ") || t.starts_with("* ");
        let text = t.trim_start_matches(['-', '*']).trim().replace("**", "");
        match items.last_mut() {
            Some(last) if !bullet && !last.is_empty() => {
                last.push(' ');
                last.push_str(&text);
            }
            _ => items.push(text),
        }
    }
    let mut out = Vec::new();
    for item in items.iter().filter(|i| !i.is_empty()) {
        for sentence in item.split_inclusive(". ").map(str::trim) {
            let body = LABEL.replace(sentence, "");
            if STARTS.is_match(&body) || NEGATED.is_match(sentence) {
                let s = if sentence.chars().count() > 240 {
                    format!("{}…", sentence.chars().take(239).collect::<String>())
                } else {
                    sentence.to_string()
                };
                out.push(s);
                if out.len() == NOTES {
                    return out;
                }
            }
        }
    }
    out
}

/// `S1`, `N4`… in a text's last parenthetical, when it names an overnight item.
fn tags(text: &str) -> Vec<String> {
    let Some(open) = text.rfind('(') else {
        return Vec::new();
    };
    let inside = &text[open..];
    if !inside.contains("overnight") {
        return Vec::new();
    }
    inside
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| {
            w.len() >= 2
                && w.len() <= 3
                && w.starts_with(|c: char| c.is_ascii_uppercase())
                && w[1..].chars().all(|c| c.is_ascii_digit())
        })
        .map(str::to_string)
        .collect()
}

fn loose(text: &str) -> String {
    text.to_lowercase()
        .replace(['—', '–'], ":")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" :", ":")
}

/// The build-log entry for a PR: one sharing an `(overnight X)` tag, else a
/// heading that starts like the title (case and dashes aside). `(found, notes)`.
pub fn notes_for(entries: &[&str], title: &str) -> (bool, Vec<String>) {
    let want = tags(title);
    let t = loose(title);
    let key: String = t.chars().take(30).collect();
    let hit = entries.iter().find(|e| {
        let head = e.lines().next().unwrap_or("");
        if !want.is_empty() {
            return tags(head).iter().any(|h| want.contains(h));
        }
        let h = loose(head);
        let h = h.split_once(" · ").map_or(h.as_str(), |(_, rest)| rest);
        !key.is_empty() && h.starts_with(&key)
    });
    match hit {
        Some(e) => (true, untested_lines(e)),
        None => (false, Vec::new()),
    }
}

/// The report for `repo` (if found) and the installed commit.
pub fn report(repo: Option<&Path>, installed: Option<String>, installed_from: &str) -> Report {
    report_target(
        repo,
        installed,
        installed_from,
        "origin/main",
        "origin/main",
        true,
    )
}

/// Exact receipt comparison, including divergent branch builds; never fetches.
pub fn report_between(repo: &Path, base: &str, target_rev: &str, target_label: &str) -> Report {
    report_target(
        Some(repo),
        Some(base.into()),
        "build receipt",
        target_rev,
        target_label,
        false,
    )
}

fn report_target(
    repo: Option<&Path>,
    installed: Option<String>,
    installed_from: &str,
    target_rev: &str,
    target_label: &str,
    require_ancestor: bool,
) -> Report {
    let target = target_label.to_string();
    let mut r = Report {
        missing: 0,
        build_id: None,
        installed: installed.clone(),
        installed_from: installed_from.into(),
        repo: repo.map(|p| p.to_string_lossy().to_string()),
        repo_source: None,
        target: target.clone(),
        target_commit: None,
        fetched_at: None,
        pull_requests: Vec::new(),
        commits: Vec::new(),
        note: None,
    };
    let Some(repo) = repo else {
        r.note = Some("No Hollow checkout is known (no build workspace was cut from one), so nothing can be compared.".into());
        return r;
    };
    let Some(installed) = installed else {
        r.note =
            Some("No finished install is recorded, so there is nothing to compare against.".into());
        return r;
    };
    r.target_commit = git(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{target_rev}^{{commit}}"),
        ],
    )
    .map(|s| s.trim().to_string());
    if r.target_commit.is_none() {
        r.note = Some(format!("This checkout has no {target_label} commit."));
        return r;
    }
    let target_rev = r.target_commit.clone().unwrap();
    let Some(base) = git(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{installed}^{{commit}}"),
        ],
    )
    .map(|s| s.trim().to_string()) else {
        r.note = Some(if require_ancestor {
            "The installed commit isn't on origin/main here (or is unavailable).".into()
        } else {
            "The build receipt's installed base commit is unavailable; what it drops could not be checked.".into()
        });
        return r;
    };
    // When the ref last moved: the reflog entry's own time, not a commit date.
    r.fetched_at = if require_ancestor {
        git(
            repo,
            &[
                "reflog",
                "show",
                "-1",
                "--date=iso-strict",
                "--format=%gd",
                "refs/remotes/origin/main",
            ],
        )
        .and_then(|s| {
            let s = s.trim();
            let open = s.find("@{")? + 2;
            Some(s[open..s.len().checked_sub(1)?].to_string())
        })
        .filter(|s| !s.is_empty())
    } else {
        None
    };
    if require_ancestor && git(repo, &["merge-base", "--is-ancestor", &base, &target_rev]).is_none()
    {
        r.note = Some(format!(
            "The installed commit {} isn't on origin/main here (a branch build, or the checkout hasn't fetched it).",
            installed.chars().take(8).collect::<String>()
        ));
        return r;
    }
    let Some(missing) = git(
        repo,
        &["rev-list", "--count", &format!("{target_rev}..{base}")],
    )
    .and_then(|s| s.trim().parse().ok()) else {
        r.note = Some("The commits this build drops could not be counted.".into());
        return r;
    };
    r.missing = missing;
    let range = format!("{base}..{target_rev}");
    let max = format!("--max-count={}", LIMIT + 1);
    let Some(out) = git(
        repo,
        &[
            "log",
            "--first-parent",
            &max,
            "--format=%H%x1f%s%x1f%cI%x1f%b%x1e",
            &range,
        ],
    ) else {
        r.note = Some("git log failed.".into());
        return r;
    };
    let log = git(repo, &["show", &format!("{target_rev}:docs/BUILD_LOG.md")]).unwrap_or_default();
    let entries: Vec<&str> = log.split("\n## ").collect();
    let records: Vec<&str> = out
        .split('\x1e')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if records.len() > LIMIT {
        r.note = Some(format!(
            "Only the newest {LIMIT} are listed; there are more."
        ));
    }
    for rec in records.into_iter().take(LIMIT) {
        let f: Vec<&str> = rec.split('\x1f').collect();
        let field = |i: usize| f.get(i).copied().unwrap_or("");
        let (sha, subject, at, body) = (field(0), field(1), field(2), field(3));
        match pr_of(subject, body) {
            Some((number, title)) => {
                let (entry_found, untested) = notes_for(&entries, &title);
                r.pull_requests.push(PullRequest {
                    number,
                    title,
                    merge: sha.chars().take(8).collect(),
                    merged_at: at.to_string(),
                    untested,
                    entry_found,
                });
            }
            None => r.commits.push(Commit {
                sha: sha.chars().take(8).collect(),
                subject: subject.to_string(),
                at: at.to_string(),
            }),
        }
    }
    if r.pull_requests.is_empty() && r.commits.is_empty() {
        r.note = Some(if r.missing == 0 {
            format!("The installed build is already {target_label}.")
        } else {
            "This build adds no commits over its installed base.".into()
        });
    }
    r
}

/// One implementation for the UI command and build.install_brings {id}.
pub fn for_build(vault: &Path, id: &str) -> Result<Report, String> {
    let status = crate::self_install::status(vault, id);
    if status.state != crate::self_install::BuildState::Built {
        return Err(format!(
            "No finished build is available for {id}. Build the app first."
        ));
    }
    let receipt = status
        .receipt
        .ok_or("The finished build has no receipt. Build it again.")?;
    Ok(report_for_receipt(
        &receipt,
        status.branch.as_deref().unwrap_or("this build"),
    ))
}

fn report_for_receipt(receipt: &crate::self_install::BuildReceipt, label: &str) -> Report {
    let mut report = report_between(
        Path::new(&receipt.worktree),
        &receipt.base_commit,
        &receipt.commit,
        label,
    );
    report.build_id = Some(receipt.build_id.clone());
    if receipt.dirty {
        let warning = "This build also includes uncommitted changes that aren't listed.";
        report.note = Some(match report.note {
            Some(note) => format!("{note} {warning}"),
            None => warning.into(),
        });
    }
    report
}

/// This Mac's installed commit: the newest finished installer journal, else
/// this binary's build identity. `(commit, journal | binary | binary (dirty))`.
pub(crate) fn installed_on_this_mac() -> (Option<String>, &'static str) {
    let journals = dirs::data_dir().map(|d| d.join("com.hollow.app/installer/journal"));
    match journals.as_deref().and_then(installed_from_journals) {
        Some(c) => (Some(c), "journal"),
        None => {
            let b = crate::app_menu::build_identity();
            (
                (!b.commit.is_empty()).then_some(b.commit),
                if b.dirty { "binary (dirty)" } else { "binary" },
            )
        }
    }
}

/// The report against `found` (see [`crate::hollow_repo`]): its source, and
/// its warning appended to the note.
pub(crate) fn report_for_repo(
    found: Option<&crate::hollow_repo::HollowRepo>,
    installed: Option<String>,
    installed_from: &str,
) -> Report {
    let mut r = report(found.map(|f| f.path.as_path()), installed, installed_from);
    if let Some(f) = found {
        r.repo_source = Some(f.source);
        if let Some(w) = &f.warning {
            r.note = Some(match r.note.take() {
                Some(note) => format!("{note} {w}"),
                None => w.clone(),
            });
        }
    }
    r
}

/// `build.install_brings`: this Mac's installed commit against its Hollow checkout.
pub fn for_this_mac(vault: &Path) -> Report {
    let (installed, from) = installed_on_this_mac();
    let found = crate::hollow_repo::for_this_mac(vault, installed.as_deref());
    report_for_repo(found.as_ref(), installed, from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn merge_subjects_name_their_pull_request() {
        assert_eq!(
            pr_of("Merge #451: Save diagnostics (overnight C6)", ""),
            Some((451, "Save diagnostics (overnight C6)".into()))
        );
        assert_eq!(
            pr_of("Merge pull request #12 from o/feat", "\nAdd the thing\n"),
            Some((12, "Add the thing".into()))
        );
        assert_eq!(pr_of("Fix a typo", ""), None);
    }

    #[test]
    fn notes_come_from_the_prs_own_build_log_entry() {
        let log = "# Build log\n\n## 2026-10-10 · Home's Due card (overnight E8)\n\n**What:** a card. A new module adds its row by hand. A runbook may not run in Quick chats.\n\n**Note:** the mock shows it. The smoke suite\nwasn't run tonight.\n- Not checked by hand in the native\n  app.\n\n## 2026-10-10 · Linked edits reach Notion (N4, overnight S1)\n\n- Left: didn't run against a real page.\n\n## 2026-09-20 · Login override: say which account\n\n- Not tried with two accounts.\n";
        let entries: Vec<&str> = log.split("\n## ").collect();
        let (found, n) = notes_for(&entries, "Home's Due card: overdue tasks (overnight E8)");
        assert!(found);
        assert_eq!(
            n,
            ["The smoke suite wasn't run tonight.", "Not checked by hand in the native app."],
            "whole sentences across wrapped lines; \"by hand\" alone and \"may not run\" aren't notes"
        );
        // Any shared tag, in any order; a title that starts like the heading.
        assert_eq!(
            notes_for(&entries, "Linked edits sync (overnight S1, N4)").1,
            ["Left: didn't run against a real page."]
        );
        assert_eq!(
            notes_for(&entries, "Login override — say which account").1,
            ["Not tried with two accounts."]
        );
        assert_eq!(notes_for(&entries, "Something else"), (false, Vec::new()));
    }

    #[test]
    fn the_newest_finished_install_wins_over_a_failed_newer_one() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("a.json"),
            r#"{"state":"healthy","commit":"aaa","startedAt":"2026-10-09T10:00:00Z"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("b.json"),
            r#"{"state":"healthy","commit":"bbb","startedAt":"2026-10-09T20:00:00Z"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("c.json"),
            r#"{"state":"failed","commit":"ccc","startedAt":"2026-10-09T23:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(installed_from_journals(dir.path()).as_deref(), Some("bbb"));
        assert_eq!(installed_from_journals(&dir.path().join("none")), None);
        // A journal whose commit couldn't be read never wins.
        std::fs::write(
            dir.path().join("d.json"),
            r#"{"state":"healthy","commit":"","startedAt":"2026-10-10T01:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(installed_from_journals(dir.path()).as_deref(), Some("bbb"));
    }

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

    #[test]
    fn a_branch_build_lists_its_pull_requests_and_what_it_drops() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        sh(d, &["init", "-q", "-b", "main"]);
        std::fs::create_dir_all(d.join("docs")).unwrap();
        std::fs::write(d.join("docs/BUILD_LOG.md"), "# Build log\n").unwrap();
        sh(d, &["add", "."]);
        sh(d, &["commit", "-qm", "root"]);
        let root = sh(d, &["rev-parse", "HEAD"]);
        sh(
            d,
            &["commit", "--allow-empty", "-qm", "Installed-only change"],
        );
        let base = sh(d, &["rev-parse", "HEAD"]);
        sh(d, &["update-ref", "refs/remotes/origin/main", &base]);
        sh(d, &["checkout", "-qb", "candidate", &root]);
        std::fs::write(d.join("docs/BUILD_LOG.md"), "# Build log\n\n## 2026-10-18 · Candidate feature\n\nNot checked by hand in the native app.\n").unwrap();
        sh(d, &["add", "."]);
        sh(d, &["commit", "-qm", "Merge #771: Candidate feature"]);
        let target = sh(d, &["rev-parse", "HEAD"]);
        let r = report_between(d, &base, &target, "candidate");
        assert_eq!(r.missing, 1);
        assert_eq!(r.target, "candidate");
        assert_eq!(r.target_commit.as_deref(), Some(target.as_str()));
        assert_eq!(r.pull_requests.len(), 1);
        assert_eq!(r.pull_requests[0].number, 771);
        assert_eq!(
            r.pull_requests[0].untested,
            ["Not checked by hand in the native app."]
        );
        assert!(r.pull_requests[0].entry_found);
        assert!(r.note.is_none());
        assert!(r.fetched_at.is_none());
        let missing = report_between(d, "unknown-base", &target, "candidate");
        assert!(missing.note.unwrap().contains("unavailable"));
        let receipt = crate::self_install::BuildReceipt {
            build_id: "r15-dirty".into(),
            workspace_id: "fixture".into(),
            worktree: d.to_string_lossy().into(),
            bundle: String::new(),
            bundle_digest: String::new(),
            source_fingerprint: String::new(),
            commit: target.clone(),
            dirty: true,
            base_commit: base.clone(),
            recorded_at: String::new(),
        };
        let dirty = report_for_receipt(&receipt, "candidate");
        assert!(dirty.note.unwrap().contains("uncommitted changes"));
        assert_eq!(dirty.build_id.as_deref(), Some("r15-dirty"));
        assert_eq!(dirty.pull_requests.len(), 1);
        let unknown_dirty = report_for_receipt(
            &crate::self_install::BuildReceipt {
                base_commit: "unknown-base".into(),
                ..receipt
            },
            "candidate",
        );
        let note = unknown_dirty.note.unwrap();
        assert!(note.contains("unavailable") && note.contains("uncommitted"));
        let rollback = report_between(d, &base, &root, "older build");
        assert_eq!(rollback.missing, 1);
        assert_eq!(
            rollback.note.as_deref(),
            Some("This build adds no commits over its installed base.")
        );
    }

    #[test]
    fn prs_and_direct_commits_between_the_installed_build_and_origin_main() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        sh(d, &["init", "-q", "-b", "main"]);
        std::fs::create_dir_all(d.join("docs")).unwrap();
        std::fs::write(d.join("docs/BUILD_LOG.md"), "# Build log\n\n## 2026-10-10 · Disk guard (overnight D1)\n\n- The smoke suite wasn't run.\n").unwrap();
        sh(d, &["add", "."]);
        sh(d, &["commit", "-q", "-m", "base"]);
        let installed = sh(d, &["rev-parse", "HEAD"]);
        sh(d, &["checkout", "-q", "-b", "feat"]);
        std::fs::write(d.join("a.txt"), "a").unwrap();
        sh(d, &["add", "."]);
        sh(d, &["commit", "-q", "-m", "Disk guard (overnight D1)"]);
        sh(d, &["checkout", "-q", "main"]);
        sh(
            d,
            &[
                "merge",
                "-q",
                "--no-ff",
                "feat",
                "-m",
                "Merge #459: Disk guard (overnight D1)",
            ],
        );
        std::fs::write(d.join("b.txt"), "b").unwrap();
        sh(d, &["add", "."]);
        sh(d, &["commit", "-q", "-m", "A direct fix"]);
        let head = sh(d, &["rev-parse", "HEAD"]);
        sh(d, &["update-ref", "refs/remotes/origin/main", &head]);
        let r = report(Some(d), Some(installed.clone()), "journal");
        assert_eq!(r.note, None, "{r:?}");
        // The ref's own reflog time, today, not a commit date.
        let fetched = r.fetched_at.clone().expect("update-ref writes a reflog");
        assert!(
            fetched.starts_with(&chrono::Local::now().format("%Y-").to_string()),
            "{fetched}"
        );
        assert_eq!(r.pull_requests.len(), 1);
        assert_eq!(r.pull_requests[0].number, 459);
        assert!(r.pull_requests[0].entry_found);
        assert_eq!(r.pull_requests[0].untested, ["The smoke suite wasn't run."]);
        assert_eq!(
            r.commits
                .iter()
                .map(|c| c.subject.as_str())
                .collect::<Vec<_>>(),
            ["A direct fix"]
        );
        // Already there: nothing to bring.
        let same = report(Some(d), Some(head), "journal");
        assert_eq!(
            same.note.as_deref(),
            Some("The installed build is already origin/main.")
        );
        // A commit main doesn't have.
        let off = report(
            Some(d),
            Some("0000000000000000000000000000000000000000".into()),
            "journal",
        );
        assert!(off.note.unwrap().contains("isn't on origin/main"));
        // Through hollow_repo: its source, and a checkout lacking the
        // installed commit said in the note (fix chat 10-20).
        let found = crate::hollow_repo::HollowRepo {
            path: d.to_path_buf(),
            source: crate::hollow_repo::RepoSource::InstallReceipt,
            warning: None,
            installed_unknown: false,
        };
        let r = report_for_repo(Some(&found), Some(installed.clone()), "journal");
        assert_eq!(
            r.repo_source,
            Some(crate::hollow_repo::RepoSource::InstallReceipt)
        );
        assert_eq!(r.pull_requests.len(), 1);
        assert_eq!(
            serde_json::to_value(&r).unwrap()["repoSource"],
            "installReceipt"
        );
        let warned = crate::hollow_repo::HollowRepo {
            warning: Some("W: may not be the repo this app was built from.".into()),
            ..found
        };
        let note = report_for_repo(
            Some(&warned),
            Some("0000000000000000000000000000000000000000".into()),
            "journal",
        )
        .note
        .unwrap();
        assert!(
            note.contains("isn't on origin/main")
                && note.ends_with("W: may not be the repo this app was built from."),
            "{note}"
        );
        assert!(report(Some(d), Some(installed.clone()), "journal")
            .repo_source
            .is_none());
        // No repository, no install.
        assert!(report(None, Some(installed), "journal")
            .note
            .unwrap()
            .contains("No Hollow checkout"));
        assert!(report(Some(d), None, "binary")
            .note
            .unwrap()
            .contains("No finished install"));
    }
}
