//! The Pull requests page's data and triage (t3code parity 7.3, D6): a
//! repository's GitHub pull requests and issues, listed and filtered, one
//! opened with its checks and comments, and the four triage moves (comment,
//! label, assign, close or reopen), all through the GitHub CLI the person
//! already has (`gh pr|issue list|view|comment|edit|close|reopen`). No new
//! service, token or cloud: `gh` is signed in as the person.
//!
//! - **Reads** reuse the PR tab's layer ([`crate::pr_card`]): the same detail
//!   parser (checks, reviews, comments, redacted and capped), the same `gh`
//!   binary lookup and environment ([`crate::build_changes::gh_command`]).
//!   A missing or signed-out `gh`, no remote, no network, rate limits and a
//!   repository with issues off are answers the page words, never errors that
//!   break it.
//! - **Triage** changes GitHub, so it is gated the way every outward action
//!   is. From the page it is the person's click after a confirm dialog that
//!   names the repository and the change (`github_triage`, plumbing). From an
//!   agent it is the External action `github.triage`, whose approval card
//!   names the repository and the exact text (it appears in Needs you like
//!   any other approval). Bodies go to `gh` on stdin, never in argv, and every
//!   typed value is checked before it reaches a command line.
//! - **Linear stays the tracker.** A title, description or branch that names a
//!   team AI issue (`ENG-123`, `ai-123-fix`) gets a link to the mirrored task
//!   and to Linear ([`crate::linear_link::find_task`]); nothing here creates,
//!   edits or syncs an issue, and other teams' keys are ignored.

use crate::build_changes::{gh_binary, gh_command};
use crate::linear_link::{self, TrackedIssue};
use crate::pr_card::{self, PrDetail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Most rows a list shows (one more is fetched to know there are others).
const LIST_MAX: usize = 50;
/// The description's cap, in characters, after redaction.
const BODY_MAX: usize = 6000;
/// A comment the person or an agent posts.
const COMMENT_MAX: usize = 10_000;
/// Most Linear links one row or detail carries.
const LINEAR_MAX: usize = 5;
/// Finds a mirrored team AI issue by key (`Sync`: the page's reads hold it across awaits).
pub(crate) type Lookup<'a> = &'a (dyn Fn(&str) -> Option<TrackedIssue> + Sync);
const READ_LIMIT: Duration = Duration::from_secs(25);
const WRITE_LIMIT: Duration = Duration::from_secs(40);

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GhLabel {
    pub name: String,
    pub color: String,
}

/// A team AI issue named by a pull request or issue. `tracked` means the
/// vault's tracker mirror has it, so it can open in Hollow's Tasks and in Linear.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LinearRef {
    pub key: String,
    pub title: Option<String>,
    /// The issue's own https link in Linear; none when it isn't mirrored.
    pub url: Option<String>,
    pub tracked: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GhRow {
    pub number: u64,
    pub title: String,
    /// open, closed or merged.
    pub state: String,
    pub url: String,
    pub draft: bool,
    pub author: String,
    pub labels: Vec<GhLabel>,
    pub assignees: Vec<String>,
    pub updated_at: String,
    pub comments: usize,
    /// APPROVED, CHANGES_REQUESTED or REVIEW_REQUIRED (pull requests).
    pub review_decision: Option<String>,
    /// success, failure or pending; none without checks (pull requests).
    pub checks: Option<String>,
    pub head: String,
    pub linear: Vec<LinearRef>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GhList {
    /// `pr` or `issue`.
    pub kind: String,
    /// `owner/name` when gh answered.
    pub repo: Option<String>,
    pub rows: Vec<GhRow>,
    /// There are more than the page shows.
    pub more: bool,
    /// Why there are no rows, in the page's words; None when there are rows or the list is simply empty.
    pub note: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GhDetail {
    pub kind: String,
    pub repo: Option<String>,
    /// The PR tab's shape: state, checks (a PR), comments and reviews.
    pub item: PrDetail,
    pub body: String,
    pub author: String,
    pub labels: Vec<GhLabel>,
    pub assignees: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub linear: Vec<LinearRef>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GhDetailView {
    pub detail: Option<GhDetail>,
    pub note: Option<String>,
}

/// The page's filters. Every text value is checked by [`list_args`].
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GhQuery {
    /// `pr` or `issue`.
    pub kind: String,
    /// open, closed, merged (pull requests) or all; open when absent.
    pub state: Option<String>,
    pub label: Option<String>,
    /// A login, or `@me`.
    pub assignee: Option<String>,
    pub author: Option<String>,
    pub search: Option<String>,
    /// Pull requests waiting for the person's review.
    pub review_requested: Option<bool>,
}

/// One triage move.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Triage {
    /// `pr` or `issue`.
    pub kind: String,
    pub number: u64,
    /// comment, label, unlabel, assign, unassign, close or reopen.
    pub action: String,
    /// The comment text, label name or login; none for close and reopen.
    pub value: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TriageDone {
    /// One sentence for the footer: what changed.
    pub message: String,
    pub repo: Option<String>,
}

// ── Words ──────────────────────────────────────────────────────────────

/// The page's words for a `gh` failure. Order matters: the specific causes
/// come before the broad "not found" and network ones.
pub(crate) fn failure_words(stderr: &str) -> String {
    let lower = stderr.to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| lower.contains(n));
    if has(&[
        "not a git repository",
        "no git remotes",
        "none of the git remotes",
        "unable to determine base repo",
        "no default remote",
    ]) {
        "This folder has no GitHub remote.".into()
    } else if has(&[
        "gh auth login",
        "not logged in",
        "authentication failed",
        "bad credentials",
        "http 401",
        "to authenticate",
    ]) {
        "The GitHub CLI isn't signed in. Run `gh auth login` in a terminal, then refresh.".into()
    } else if has(&["rate limit"]) {
        "GitHub is limiting requests for now. Wait a few minutes and refresh.".into()
    } else if has(&[
        "resource not accessible",
        "http 403",
        "forbidden",
        "must have push access",
        "permission",
    ]) {
        "Your GitHub sign-in isn't allowed to do that in this repository.".into()
    } else if has(&[
        "has disabled issues",
        "issues are disabled",
        "disabled issues",
    ]) {
        "Issues are turned off for this repository.".into()
    } else if lower.contains("label") && has(&["not found", "could not resolve", "does not exist"])
    {
        "That label doesn't exist in this repository. Create it on GitHub first.".into()
    } else if has(&[
        "could not resolve to",
        "http 404",
        "not found",
        "no pull requests found",
        "no issues found",
    ]) {
        "GitHub has no such pull request or issue in this repository.".into()
    } else if has(&["is already closed", "already closed"]) {
        "That is already closed.".into()
    } else if has(&[
        "is not closed",
        "already open",
        "cannot be reopened",
        "can't be reopened",
    ]) {
        "That can't be reopened.".into()
    } else if has(&[
        "could not resolve host",
        "timeout",
        "timed out",
        "network",
        "connection",
        "dial tcp",
    ]) {
        "Couldn't reach GitHub. Check the connection and refresh.".into()
    } else {
        "GitHub didn't answer. Refresh to try again.".into()
    }
}

fn no_gh_note() -> String {
    "Install the GitHub CLI (gh) and sign in to see pull requests and issues here.".into()
}

// ── Checked values ─────────────────────────────────────────────────────

fn kind_word(kind: &str) -> Result<&'static str, String> {
    match kind {
        "pr" => Ok("pr"),
        "issue" => Ok("issue"),
        _ => Err("The kind must be pr or issue.".into()),
    }
}

/// A value that goes on a command line: trimmed, not empty, one line, no
/// control characters, at most `max` characters.
fn plain_value(raw: &str, what: &str, max: usize) -> Result<String, String> {
    let v = raw.trim();
    if v.is_empty() {
        return Err(format!("{what} is empty."));
    }
    if v.chars().count() > max {
        return Err(format!("{what} is longer than {max} characters."));
    }
    if v.chars().any(char::is_control) {
        return Err(format!(
            "{what} can't contain line breaks or control characters."
        ));
    }
    Ok(v.to_string())
}

/// A GitHub login or `@me`.
fn login_value(raw: &str, what: &str) -> Result<String, String> {
    let v = plain_value(raw, what, 40)?;
    let body = v.strip_prefix('@').unwrap_or(&v);
    let ok = if v == "@me" {
        true
    } else {
        !v.starts_with('@')
            && !body.is_empty()
            && body
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'[' | b']'))
    };
    if ok {
        Ok(v)
    } else {
        Err(format!("{what} isn't a GitHub login."))
    }
}

fn label_value(raw: &str) -> Result<String, String> {
    let v = plain_value(raw, "The label", 100)?;
    if v.contains(',') {
        return Err("Add one label at a time (a label name can't contain a comma).".into());
    }
    Ok(v)
}

fn opt_text(raw: &Option<String>) -> Option<&str> {
    raw.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

const PR_FIELDS: &str = "number,title,state,url,isDraft,author,labels,assignees,updatedAt,comments,reviewDecision,headRefName,statusCheckRollup";
const ISSUE_FIELDS: &str = "number,title,state,url,author,labels,assignees,updatedAt,comments";
const PR_DETAIL_FIELDS: &str = "number,title,state,url,isDraft,reviewDecision,mergeStateStatus,headRefName,baseRefName,statusCheckRollup,comments,reviews,body,author,labels,assignees,createdAt,updatedAt";
const ISSUE_DETAIL_FIELDS: &str =
    "number,title,state,url,comments,body,author,labels,assignees,createdAt,updatedAt";

/// The `gh` arguments for a list. Filters travel as `--flag=value` so a value
/// is never read as an option.
pub(crate) fn list_args(q: &GhQuery) -> Result<Vec<String>, String> {
    let kind = kind_word(&q.kind)?;
    let state = opt_text(&q.state).unwrap_or("open");
    let states: &[&str] = if kind == "pr" {
        &["open", "closed", "merged", "all"]
    } else {
        &["open", "closed", "all"]
    };
    if !states.contains(&state) {
        return Err(format!(
            "A {kind} list can be open, closed, {}or all.",
            if kind == "pr" { "merged, " } else { "" }
        ));
    }
    let mut args: Vec<String> = vec![
        kind.into(),
        "list".into(),
        format!("--state={state}"),
        "--limit".into(),
        (LIST_MAX + 1).to_string(),
    ];
    if let Some(l) = opt_text(&q.label) {
        args.push(format!("--label={}", label_value(l)?));
    }
    if let Some(a) = opt_text(&q.assignee) {
        args.push(format!("--assignee={}", login_value(a, "The assignee")?));
    }
    if let Some(a) = opt_text(&q.author) {
        args.push(format!("--author={}", login_value(a, "The author")?));
    }
    let mut search = opt_text(&q.search)
        .map(|s| plain_value(s, "The search", 200))
        .transpose()?;
    if q.review_requested == Some(true) && kind == "pr" {
        search = Some(match search {
            Some(s) => format!("review-requested:@me {s}"),
            None => "review-requested:@me".into(),
        });
    }
    if let Some(s) = search {
        args.push(format!("--search={s}"));
    }
    args.push("--json".into());
    args.push(
        if kind == "pr" {
            PR_FIELDS
        } else {
            ISSUE_FIELDS
        }
        .into(),
    );
    Ok(args)
}

/// What a triage move runs: its `gh` arguments, the text for stdin (a comment),
/// and the sentence that says it is done. Nothing runs here.
pub(crate) fn triage_plan(t: &Triage) -> Result<(Vec<String>, Option<String>, String), String> {
    let kind = kind_word(&t.kind)?;
    if t.number == 0 {
        return Err("The number must be a pull request or issue number.".into());
    }
    let n = t.number.to_string();
    let noun = if kind == "pr" {
        "pull request"
    } else {
        "issue"
    };
    let value = t.value.as_deref().unwrap_or("");
    let edit = |flag: &str, v: String| {
        vec![
            kind.to_string(),
            "edit".into(),
            n.clone(),
            format!("{flag}={v}"),
        ]
    };
    Ok(match t.action.as_str() {
        "comment" => {
            let text = value.trim();
            if text.is_empty() {
                return Err("A comment needs some text. Nothing was posted.".into());
            }
            if text.chars().count() > COMMENT_MAX {
                return Err(format!(
                    "A comment can be at most {COMMENT_MAX} characters. Nothing was posted."
                ));
            }
            (
                vec![
                    kind.into(),
                    "comment".into(),
                    n,
                    "--body-file".into(),
                    "-".into(),
                ],
                Some(text.to_string()),
                format!("Commented on {noun} #{}", t.number),
            )
        }
        "label" => {
            let v = label_value(value)?;
            (
                edit("--add-label", v.clone()),
                None,
                format!("Added the label {v} to {noun} #{}", t.number),
            )
        }
        "unlabel" => {
            let v = label_value(value)?;
            (
                edit("--remove-label", v.clone()),
                None,
                format!("Removed the label {v} from {noun} #{}", t.number),
            )
        }
        "assign" => {
            let v = login_value(value, "The assignee")?;
            (
                edit("--add-assignee", v.clone()),
                None,
                format!("Assigned {v} to {noun} #{}", t.number),
            )
        }
        "unassign" => {
            let v = login_value(value, "The assignee")?;
            (
                edit("--remove-assignee", v.clone()),
                None,
                format!("Unassigned {v} from {noun} #{}", t.number),
            )
        }
        "close" => (
            vec![kind.into(), "close".into(), n],
            None,
            format!("Closed {noun} #{}", t.number),
        ),
        "reopen" => (
            vec![kind.into(), "reopen".into(), n],
            None,
            format!("Reopened {noun} #{}", t.number),
        ),
        _ => {
            return Err(
                "The action must be comment, label, unlabel, assign, unassign, close or reopen."
                    .into(),
            )
        }
    })
}

// ── Linear links ───────────────────────────────────────────────────────

/// Team ENG identifiers named in `texts` (`ENG-123`, `eng-123-fix`, `feature/ENG-9`),
/// upper-cased, in order, each once, at most [`LINEAR_MAX`]. A word must start at
/// a boundary and the number must end at one, so `chain-12` and `eng-3d` match nothing.
pub(crate) fn linear_keys(texts: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for text in texts {
        let b = text.as_bytes();
        let mut i = 0;
        while i + 4 < b.len() && out.len() < LINEAR_MAX {
            let starts = i == 0 || !b[i - 1].is_ascii_alphanumeric();
            if starts
                && b[i].eq_ignore_ascii_case(&b'e')
                && b[i + 1].eq_ignore_ascii_case(&b'n')
                && b[i + 2].eq_ignore_ascii_case(&b'g')
                && b[i + 3] == b'-'
            {
                let digits = b[i + 4..].iter().take_while(|c| c.is_ascii_digit()).count();
                let end = i + 4 + digits;
                let ends = end >= b.len() || !b[end].is_ascii_alphanumeric();
                if (1..=7).contains(&digits) && ends {
                    let key = format!("ENG-{}", &text[i + 4..end]);
                    if !out.contains(&key) {
                        out.push(key);
                    }
                    i = end;
                    continue;
                }
            }
            i += 1;
        }
    }
    out
}

/// The links for those keys, looked up in the vault's tracker mirror. A key the
/// mirror doesn't have stays plain text (no link, no guess at Linear's address).
pub(crate) fn linear_refs(texts: &[&str], lookup: Lookup<'_>) -> Vec<LinearRef> {
    linear_keys(texts)
        .into_iter()
        .map(|key| match lookup(&key) {
            Some(t) => LinearRef {
                title: Some(t.title).filter(|s| !s.is_empty() && *s != key),
                url: Some(t.url).filter(|u| u.starts_with("https://")),
                tracked: true,
                key,
            },
            None => LinearRef {
                key,
                title: None,
                url: None,
                tracked: false,
            },
        })
        .collect()
}

// ── Reading ────────────────────────────────────────────────────────────

fn labels_of(v: &Value) -> Vec<GhLabel> {
    v.get("labels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|l| {
            Some(GhLabel {
                name: pr_card::opt(l, "name")?,
                color: pr_card::opt(l, "color")
                    .filter(|c| c.len() == 6 && c.bytes().all(|b| b.is_ascii_hexdigit()))
                    .unwrap_or_default(),
            })
        })
        .collect()
}

fn assignees_of(v: &Value) -> Vec<String> {
    v.get("assignees")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|a| pr_card::opt(a, "login"))
        .collect()
}

/// The worst of a PR's checks: failure, pending or success; none without checks.
fn checks_state(v: &Value) -> Option<String> {
    let items = v.get("statusCheckRollup").and_then(Value::as_array)?;
    if items.is_empty() {
        return None;
    }
    let states: Vec<&str> = items.iter().map(pr_card::check_state).collect();
    Some(
        if states.contains(&"failure") {
            "failure"
        } else if states.contains(&"pending") {
            "pending"
        } else {
            "success"
        }
        .into(),
    )
}

fn row_of(kind: &str, v: &Value, lookup: Lookup<'_>) -> Option<GhRow> {
    let title = pr_card::s(v, "title");
    let head = pr_card::s(v, "headRefName");
    Some(GhRow {
        number: v.get("number")?.as_u64()?,
        state: pr_card::s(v, "state").to_lowercase(),
        url: pr_card::s(v, "url"),
        draft: kind == "pr" && v.get("isDraft").and_then(Value::as_bool).unwrap_or(false),
        author: pr_card::author(v),
        labels: labels_of(v),
        assignees: assignees_of(v),
        updated_at: pr_card::s(v, "updatedAt"),
        comments: v
            .get("comments")
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        review_decision: if kind == "pr" {
            pr_card::opt(v, "reviewDecision")
        } else {
            None
        },
        checks: if kind == "pr" { checks_state(v) } else { None },
        linear: linear_refs(&[title.as_str(), head.as_str()], lookup),
        head,
        title,
    })
}

/// The rows of `gh pr|issue list --json` output, and whether there were more
/// than the page shows. `None` when the answer isn't a list.
pub(crate) fn parse_list(kind: &str, json: &str, lookup: Lookup<'_>) -> Option<(Vec<GhRow>, bool)> {
    let v: Value = serde_json::from_str(json.trim()).ok()?;
    let mut rows: Vec<GhRow> = v
        .as_array()?
        .iter()
        .filter_map(|r| row_of(kind, r, lookup))
        .collect();
    let more = rows.len() > LIST_MAX;
    rows.truncate(LIST_MAX);
    Some((rows, more))
}

/// The detail of one `gh pr|issue view --json` answer.
pub(crate) fn parse_detail(
    kind: &str,
    repo: Option<String>,
    json: &str,
    lookup: Lookup<'_>,
) -> Option<GhDetail> {
    let v: Value = serde_json::from_str(json.trim()).ok()?;
    let item = pr_card::detail_of(&v)?;
    let body: String = crate::claude_runner::redact(pr_card::s(&v, "body").trim())
        .chars()
        .take(BODY_MAX)
        .collect();
    let linear = linear_refs(
        &[item.title.as_str(), body.as_str(), item.head.as_str()],
        lookup,
    );
    Some(GhDetail {
        kind: kind.into(),
        repo,
        author: pr_card::author(&v),
        labels: labels_of(&v),
        assignees: assignees_of(&v),
        created_at: pr_card::s(&v, "createdAt"),
        updated_at: pr_card::s(&v, "updatedAt"),
        item,
        body,
        linear,
    })
}

/// What one `gh` run gave back, as the page uses it.
struct Ran {
    ok: bool,
    stdout: String,
    stderr: String,
}

async fn gh_run(
    gh: &Path,
    dir: &Path,
    args: &[String],
    input: Option<&[u8]>,
    limit: Duration,
) -> Result<Ran, String> {
    let mut cmd = gh_command(gh, dir);
    cmd.args(args);
    let out = crate::process_group::run_to_completion(&mut cmd, input, limit, None)
        .await
        .map_err(|e| format!("couldn't run gh: {e}"))?;
    Ok(Ran {
        ok: !out.timed_out && out.status.is_some_and(|s| s.success()),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: if out.timed_out {
            "timeout".into()
        } else {
            out.stderr
        },
    })
}

fn folder(cwd: &str) -> Result<PathBuf, String> {
    let dir = Path::new(cwd);
    if !dir.is_absolute() || !dir.is_dir() {
        return Err("The folder must be an existing absolute path".into());
    }
    Ok(dir.to_path_buf())
}

/// `owner/name` of the folder's GitHub repository, or the page's words for why not.
async fn repo_of(gh: &Path, dir: &Path) -> Result<String, String> {
    let args: Vec<String> = ["repo", "view", "--json", "nameWithOwner"]
        .map(String::from)
        .to_vec();
    let ran = gh_run(gh, dir, &args, None, READ_LIMIT).await?;
    if !ran.ok {
        return Err(failure_words(&ran.stderr));
    }
    serde_json::from_str::<Value>(ran.stdout.trim())
        .ok()
        .and_then(|v| pr_card::opt(&v, "nameWithOwner"))
        .ok_or_else(|| "GitHub didn't name this repository. Refresh to try again.".into())
}

fn empty_list(kind: &str, repo: Option<String>, note: String) -> GhList {
    GhList {
        kind: kind.into(),
        repo,
        rows: Vec::new(),
        more: false,
        note: Some(note),
    }
}

/// The list for a folder's repository, through `gh`. Failures come back as the
/// list's own `note`; a bad filter is an error.
pub(crate) async fn list_with(
    gh: &Path,
    cwd: &str,
    q: &GhQuery,
    lookup: Lookup<'_>,
) -> Result<GhList, String> {
    let dir = folder(cwd)?;
    let args = list_args(q)?;
    let kind = kind_word(&q.kind)?;
    let repo = match repo_of(gh, &dir).await {
        Ok(r) => r,
        Err(note) => return Ok(empty_list(kind, None, note)),
    };
    let ran = gh_run(gh, &dir, &args, None, READ_LIMIT).await?;
    if !ran.ok {
        return Ok(empty_list(kind, Some(repo), failure_words(&ran.stderr)));
    }
    Ok(match parse_list(kind, &ran.stdout, lookup) {
        Some((rows, more)) => GhList {
            kind: kind.into(),
            repo: Some(repo),
            rows,
            more,
            note: None,
        },
        None => empty_list(
            kind,
            Some(repo),
            "GitHub's answer couldn't be read. Refresh to try again.".into(),
        ),
    })
}

/// One pull request or issue with its checks, comments, labels and Linear links.
pub(crate) async fn detail_with(
    gh: &Path,
    cwd: &str,
    kind: &str,
    number: u64,
    lookup: Lookup<'_>,
) -> Result<GhDetailView, String> {
    let dir = folder(cwd)?;
    let kind = kind_word(kind)?;
    if number == 0 {
        return Err("The number must be a pull request or issue number.".into());
    }
    let repo = repo_of(gh, &dir).await.ok();
    let args: Vec<String> = vec![
        kind.into(),
        "view".into(),
        number.to_string(),
        "--json".into(),
        if kind == "pr" {
            PR_DETAIL_FIELDS
        } else {
            ISSUE_DETAIL_FIELDS
        }
        .into(),
    ];
    let ran = gh_run(gh, &dir, &args, None, READ_LIMIT).await?;
    if !ran.ok {
        return Ok(GhDetailView {
            detail: None,
            note: Some(failure_words(&ran.stderr)),
        });
    }
    Ok(match parse_detail(kind, repo, &ran.stdout, lookup) {
        Some(detail) => GhDetailView {
            detail: Some(detail),
            note: None,
        },
        None => GhDetailView {
            detail: None,
            note: Some("GitHub's answer couldn't be read. Refresh to try again.".into()),
        },
    })
}

/// Run one triage move. The caller has already asked (the confirm dialog, or
/// the approval card); this checks the values, runs `gh` and words the result.
pub(crate) async fn triage_with(gh: &Path, cwd: &str, t: &Triage) -> Result<TriageDone, String> {
    let dir = folder(cwd)?;
    let (args, input, message) = triage_plan(t)?;
    let repo = repo_of(gh, &dir).await.ok();
    let ran = gh_run(
        gh,
        &dir,
        &args,
        input.as_deref().map(str::as_bytes),
        WRITE_LIMIT,
    )
    .await?;
    if !ran.ok {
        return Err(format!(
            "{} Nothing was changed.",
            failure_words(&ran.stderr)
        ));
    }
    Ok(TriageDone { message, repo })
}

fn mirror_lookup(vault: &Path) -> impl Fn(&str) -> Option<TrackedIssue> + '_ {
    move |key| linear_link::find_task(vault, key)
}

/// The page's list read.
pub async fn list(cwd: &str, q: &GhQuery, vault: &Path) -> Result<GhList, String> {
    let Some(gh) = gh_binary() else {
        folder(cwd)?;
        return Ok(empty_list(kind_word(&q.kind)?, None, no_gh_note()));
    };
    list_with(&gh, cwd, q, &mirror_lookup(vault)).await
}

/// The page's detail read.
pub async fn detail(
    cwd: &str,
    kind: &str,
    number: u64,
    vault: &Path,
) -> Result<GhDetailView, String> {
    let Some(gh) = gh_binary() else {
        return Ok(GhDetailView {
            detail: None,
            note: Some(no_gh_note()),
        });
    };
    detail_with(&gh, cwd, kind, number, &mirror_lookup(vault)).await
}

/// Triage, after the person's confirm dialog or an agent's approved card.
pub async fn triage(cwd: &str, t: &Triage) -> Result<TriageDone, String> {
    let Some(gh) = gh_binary() else {
        return Err(no_gh_note());
    };
    triage_with(&gh, cwd, t).await
}

// ── The approval card's words ──────────────────────────────────────────

/// `owner/name` from a git remote address (https or scp style, credentials
/// dropped); none for a remote that isn't on github.com.
pub(crate) fn repo_from_remote(url: &str) -> Option<String> {
    let rest = url.trim().strip_suffix(".git").unwrap_or(url.trim());
    let path = rest
        .strip_prefix("git@github.com:")
        .or_else(|| rest.strip_prefix("ssh://git@github.com/"))
        .or_else(|| {
            let after = rest
                .strip_prefix("https://")
                .or_else(|| rest.strip_prefix("http://"))?;
            let after = after.rsplit_once('@').map_or(after, |(_, host)| host);
            after.strip_prefix("github.com/")
        })?;
    let (owner, name) = path.trim_matches('/').split_once('/')?;
    let plain = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    (plain(owner) && plain(name)).then(|| format!("{owner}/{name}"))
}

/// The repository a folder's `origin` names, for an approval card; the folder's
/// own words when it can't be read.
pub(crate) fn card_repo(cwd: &str) -> String {
    std::process::Command::new("git")
        .args([
            "-C",
            cwd,
            "--no-optional-locks",
            "remote",
            "get-url",
            "origin",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| repo_from_remote(String::from_utf8_lossy(&o.stdout).trim()))
        .map(|r| format!("{r} on GitHub"))
        .unwrap_or_else(|| "this folder's GitHub repository".into())
}

/// The lines and destination of the approval card for an agent's triage move.
pub(crate) fn card_lines(t: &Triage) -> Result<Vec<String>, String> {
    let (args, input, done) = triage_plan(t)?;
    let mut lines = vec![format!("{done}.")];
    if let Some(text) = input {
        // The whole text, never a cut: what the person approves is exactly what is posted (COMMENT_MAX caps it).
        lines.push(format!("The comment will read: \u{201c}{text}\u{201d}"));
    }
    lines.push(format!(
        "Runs: gh {}",
        args.iter().take(4).cloned().collect::<Vec<_>>().join(" ")
    ));
    Ok(lines)
}

// ── Tauri commands (the person's clicks) ───────────────────────────────

#[tauri::command]
pub async fn github_list(
    cwd: String,
    query: GhQuery,
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<GhList, String> {
    list(&cwd, &query, &state.config.vault_path).await
}

#[tauri::command]
pub async fn github_detail(
    cwd: String,
    kind: String,
    number: u64,
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<GhDetailView, String> {
    detail(&cwd, &kind, number, &state.config.vault_path).await
}

/// The person's click after the page's confirm dialog. Agents use `github.triage`, which asks on a card.
#[tauri::command]
pub async fn github_triage(cwd: String, triage: Triage) -> Result<TriageDone, String> {
    self::triage(&cwd, &triage).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none(_: &str) -> Option<TrackedIssue> {
        None
    }

    fn mirrored(key: &str) -> Option<TrackedIssue> {
        (key == "ENG-12").then(|| TrackedIssue {
            key: key.into(),
            title: "Fix the lot map".into(),
            url: "https://linear.app/x/issue/ENG-12".into(),
            rel: "tasks/a.md".into(),
            workspace: None,
        })
    }

    fn q(kind: &str) -> GhQuery {
        GhQuery {
            kind: kind.into(),
            ..Default::default()
        }
    }

    #[test]
    fn list_args_default_to_open_and_carry_each_filter_as_one_flag() {
        let a = list_args(&q("pr")).unwrap();
        assert_eq!(&a[..5], ["pr", "list", "--state=open", "--limit", "51"]);
        assert_eq!(a[a.len() - 2], "--json");
        let mut f = q("issue");
        f.state = Some("closed".into());
        f.label = Some("bug".into());
        f.assignee = Some("@me".into());
        f.author = Some("ann".into());
        f.search = Some("crash on launch".into());
        let a = list_args(&f).unwrap();
        for want in [
            "--state=closed",
            "--label=bug",
            "--assignee=@me",
            "--author=ann",
            "--search=crash on launch",
        ] {
            assert!(a.iter().any(|x| x == want), "{want} in {a:?}");
        }
        let mut r = q("pr");
        r.review_requested = Some(true);
        r.search = Some("lanes".into());
        assert!(list_args(&r)
            .unwrap()
            .contains(&"--search=review-requested:@me lanes".to_string()));
        // Review requested is a pull request filter only.
        let mut i = q("issue");
        i.review_requested = Some(true);
        assert!(!list_args(&i)
            .unwrap()
            .iter()
            .any(|x| x.starts_with("--search")));
    }

    #[test]
    fn list_args_refuse_values_that_are_not_plain() {
        let mut bad = q("issue");
        bad.state = Some("merged".into());
        assert!(list_args(&bad).is_err());
        assert!(list_args(&q("branch")).is_err());
        for (field, value) in [
            ("label", "a,b"),
            ("label", "x\ny"),
            ("assignee", "ann smith"),
            ("assignee", "@ann"),
            ("author", "$(x)"),
            ("search", "a\u{0}b"),
        ] {
            let mut f = q("pr");
            match field {
                "label" => f.label = Some(value.into()),
                "assignee" => f.assignee = Some(value.into()),
                "author" => f.author = Some(value.into()),
                _ => f.search = Some(value.into()),
            }
            assert!(list_args(&f).is_err(), "{field}={value:?}");
        }
        // A value that starts with a dash is carried inside its flag, never read as an option.
        let mut dash = q("pr");
        dash.label = Some("--web".into());
        assert_eq!(list_args(&dash).unwrap()[5], "--label=--web");
    }

    #[test]
    fn each_triage_move_has_its_command_and_a_comment_goes_by_stdin() {
        let t = |action: &str, value: Option<&str>| Triage {
            kind: "pr".into(),
            number: 7,
            action: action.into(),
            value: value.map(Into::into),
        };
        let (args, input, done) =
            triage_plan(&t("comment", Some("  Thanks, merging soon.  "))).unwrap();
        assert_eq!(args, ["pr", "comment", "7", "--body-file", "-"]);
        assert_eq!(input.as_deref(), Some("Thanks, merging soon."));
        assert_eq!(done, "Commented on pull request #7");
        assert!(
            !args.iter().any(|a| a.contains("Thanks")),
            "the text never reaches argv"
        );
        assert_eq!(
            triage_plan(&t("label", Some("bug"))).unwrap().0,
            ["pr", "edit", "7", "--add-label=bug"]
        );
        assert_eq!(
            triage_plan(&t("unlabel", Some("bug"))).unwrap().0,
            ["pr", "edit", "7", "--remove-label=bug"]
        );
        assert_eq!(
            triage_plan(&t("assign", Some("@me"))).unwrap().0,
            ["pr", "edit", "7", "--add-assignee=@me"]
        );
        assert_eq!(
            triage_plan(&t("unassign", Some("ann"))).unwrap().0,
            ["pr", "edit", "7", "--remove-assignee=ann"]
        );
        assert_eq!(
            triage_plan(&t("close", None)).unwrap().0,
            ["pr", "close", "7"]
        );
        let issue = Triage {
            kind: "issue".into(),
            number: 9,
            action: "reopen".into(),
            value: None,
        };
        let (args, _, done) = triage_plan(&issue).unwrap();
        assert_eq!(args, ["issue", "reopen", "9"]);
        assert_eq!(done, "Reopened issue #9");
    }

    #[test]
    fn a_bad_triage_move_is_refused_before_anything_runs() {
        let mk = |kind: &str, number: u64, action: &str, value: &str| Triage {
            kind: kind.into(),
            number,
            action: action.into(),
            value: Some(value.into()),
        };
        assert!(triage_plan(&mk("pr", 1, "comment", "   "))
            .unwrap_err()
            .contains("some text"));
        assert!(triage_plan(&mk("pr", 1, "comment", &"x".repeat(10_001))).is_err());
        assert!(triage_plan(&mk("pr", 0, "close", "")).is_err());
        assert!(triage_plan(&mk("repo", 1, "close", "")).is_err());
        assert!(
            triage_plan(&mk("pr", 1, "merge", "")).is_err(),
            "merging is not a triage move"
        );
        assert!(triage_plan(&mk("pr", 1, "label", "a,b")).is_err());
        assert!(triage_plan(&mk("pr", 1, "assign", "not a login")).is_err());
    }

    #[test]
    fn linear_keys_find_team_ai_only_at_word_edges() {
        assert_eq!(
            linear_keys(&[
                "Fix ENG-12 and eng-13",
                "feature/eng-14-lot-map",
                "(ENG-12)"
            ]),
            ["ENG-12", "ENG-13", "ENG-14"]
        );
        assert!(linear_keys(&[
            "chain-12",
            "eng-3d printing",
            "PER-5",
            "ENG-12345678",
            "eng-"
        ])
        .is_empty());
        assert_eq!(
            linear_keys(&["ENG-1, ENG-2, ENG-3, ENG-4, ENG-5, ENG-6, ENG-7"]).len(),
            5
        );
    }

    #[test]
    fn a_mirrored_issue_links_and_an_unknown_one_stays_plain() {
        let refs = linear_refs(&["ENG-12 and ENG-99"], &mirrored);
        assert_eq!(
            refs[0],
            LinearRef {
                key: "ENG-12".into(),
                title: Some("Fix the lot map".into()),
                url: Some("https://linear.app/x/issue/ENG-12".into()),
                tracked: true
            }
        );
        assert_eq!(
            refs[1],
            LinearRef {
                key: "ENG-99".into(),
                title: None,
                url: None,
                tracked: false
            }
        );
        assert!(linear_refs(&["no key here"], &none).is_empty());
    }

    const PR_LIST: &str = r#"[
      {"number":5,"title":"ENG-12 Add the lot map","state":"OPEN","url":"https://github.com/o/r/pull/5","isDraft":true,
       "author":{"login":"ann"},"labels":[{"name":"bug","color":"d73a4a"},{"name":"odd","color":"zz"}],
       "assignees":[{"login":"bob"}],"updatedAt":"2026-11-20T10:00:00Z","comments":[{"body":"x"},{"body":"y"}],
       "reviewDecision":"REVIEW_REQUIRED","headRefName":"eng-12-lot-map",
       "statusCheckRollup":[{"name":"gate","status":"COMPLETED","conclusion":"SUCCESS"},{"name":"e2e","status":"IN_PROGRESS"}]},
      {"number":4,"title":"Plain","state":"MERGED","url":"https://github.com/o/r/pull/4","author":{"login":"cy"},"headRefName":"plain","statusCheckRollup":[]}
    ]"#;

    #[test]
    fn a_pr_list_reads_state_labels_checks_and_links() {
        let (rows, more) = parse_list("pr", PR_LIST, &mirrored).unwrap();
        assert!(!more);
        assert_eq!(rows.len(), 2);
        let r = &rows[0];
        assert_eq!(
            (r.number, r.state.as_str(), r.draft, r.comments),
            (5, "open", true, 2)
        );
        assert_eq!(r.checks.as_deref(), Some("pending"));
        assert_eq!(r.review_decision.as_deref(), Some("REVIEW_REQUIRED"));
        assert_eq!(
            r.labels[0],
            GhLabel {
                name: "bug".into(),
                color: "d73a4a".into()
            }
        );
        assert_eq!(r.labels[1].color, "", "a colour that isn't hex is dropped");
        assert_eq!(r.assignees, ["bob"]);
        assert_eq!(r.linear.len(), 1, "the title and the branch name one issue");
        assert!(r.linear[0].tracked);
        assert_eq!(
            (rows[1].state.as_str(), rows[1].checks.clone()),
            ("merged", None)
        );
        assert_eq!(rows[1].author, "cy");
    }

    #[test]
    fn an_issue_list_has_no_pr_fields_and_a_long_list_says_there_is_more() {
        let json = r#"[{"number":3,"title":"Crash","state":"OPEN","url":"u","isDraft":true,"reviewDecision":"APPROVED","author":{"login":"a"},"comments":[]}]"#;
        let (rows, _) = parse_list("issue", json, &none).unwrap();
        assert!(!rows[0].draft && rows[0].review_decision.is_none() && rows[0].checks.is_none());
        let many: Vec<String> = (1..=51)
            .map(|n| format!(r#"{{"number":{n},"title":"t","state":"OPEN","url":"u"}}"#))
            .collect();
        let (rows, more) = parse_list("issue", &format!("[{}]", many.join(",")), &none).unwrap();
        assert_eq!((rows.len(), more), (50, true));
        assert!(parse_list("issue", "not json", &none).is_none());
        assert!(parse_list("issue", "{}", &none).is_none());
    }

    #[test]
    fn a_detail_carries_the_description_labels_comments_and_links() {
        let json = r#"{"number":5,"title":"Add it","state":"OPEN","url":"https://github.com/o/r/pull/5","isDraft":false,
          "body":"Closes ENG-12. Token ghp_abcdefghijklmnopqrstuvwxyz0123456789","author":{"login":"ann"},
          "labels":[{"name":"bug","color":"d73a4a"}],"assignees":[],"createdAt":"2026-11-19T10:00:00Z","updatedAt":"2026-11-20T10:00:00Z",
          "headRefName":"topic","baseRefName":"main","reviewDecision":"APPROVED","mergeStateStatus":"CLEAN",
          "statusCheckRollup":[{"name":"gate","status":"COMPLETED","conclusion":"SUCCESS"}],
          "comments":[{"author":{"login":"bob"},"body":"ok","createdAt":"2026-11-20T09:00:00Z"}],"reviews":[]}"#;
        let d = parse_detail("pr", Some("o/r".into()), json, &mirrored).unwrap();
        assert_eq!(
            (d.kind.as_str(), d.repo.as_deref(), d.author.as_str()),
            ("pr", Some("o/r"), "ann")
        );
        assert_eq!(d.item.checks.len(), 1);
        assert_eq!(d.item.comments.len(), 1);
        assert!(
            !d.body.contains("ghp_abcdefghijklmnopqrstuvwxyz"),
            "secrets are redacted: {}",
            d.body
        );
        assert_eq!(d.linear[0].key, "ENG-12");
        let issue = r#"{"number":8,"title":"Crash","state":"CLOSED","url":"https://github.com/o/r/issues/8","body":"","comments":[]}"#;
        let d = parse_detail("issue", None, issue, &none).unwrap();
        assert_eq!(d.item.state, "closed");
        assert!(d.item.checks.is_empty() && d.linear.is_empty());
        assert!(parse_detail("pr", None, "[]", &none).is_none());
        let long = format!(r#"{{"number":1,"url":"u","body":"{}"}}"#, "y".repeat(9000));
        assert_eq!(
            parse_detail("issue", None, &long, &none)
                .unwrap()
                .body
                .chars()
                .count(),
            6000
        );
    }

    #[test]
    fn gh_failures_get_the_pages_words() {
        let words = |e: &str| failure_words(e);
        assert!(words("fatal: not a git repository").contains("no GitHub remote"));
        assert!(
            words("To get started with GitHub CLI, please run:  gh auth login")
                .contains("gh auth login")
        );
        assert!(words("API rate limit exceeded for user").contains("limiting requests"));
        assert!(
            words("GraphQL: Resource not accessible by personal access token (HTTP 403)")
                .contains("isn't allowed")
        );
        assert!(words("the 'o/r' repository has disabled issues").contains("Issues are turned off"));
        assert!(
            words("failed to update: 'bogus' not found (label)").contains("label doesn't exist")
        );
        assert!(words(
            "GraphQL: Could not resolve to an issue or pull request with the number of 99."
        )
        .contains("no such"));
        assert!(words("Pull request #5 is already closed").contains("already closed"));
        assert!(
            words("dial tcp: lookup api.github.com: could not resolve host")
                .starts_with("Couldn't reach GitHub")
        );
        assert!(words("boom").contains("didn't answer"));
        // Raw stderr never reaches the page.
        assert!(!words("token ghp_secret leaked in boom").contains("ghp_"));
    }

    #[test]
    fn a_remote_names_its_repository_without_credentials() {
        for (url, want) in [
            ("git@github.com:o/r.git", Some("o/r")),
            ("https://github.com/o/r", Some("o/r")),
            ("https://user:tok@github.com/o/r.git", Some("o/r")),
            ("ssh://git@github.com/o/r.git", Some("o/r")),
            ("https://example.com/o/r.git", None),
            ("git@github.com:onlyowner", None),
            ("https://github.com/o/r r", None),
        ] {
            assert_eq!(repo_from_remote(url).as_deref(), want, "{url}");
        }
    }

    #[test]
    fn the_approval_card_shows_the_text_and_the_command() {
        let t = Triage {
            kind: "issue".into(),
            number: 3,
            action: "comment".into(),
            value: Some("Fixed in the next build.".into()),
        };
        let lines = card_lines(&t).unwrap();
        assert_eq!(lines[0], "Commented on issue #3.");
        assert!(lines[1].contains("Fixed in the next build."));
        assert_eq!(lines[2], "Runs: gh issue comment 3 --body-file");
        assert!(card_lines(&Triage {
            action: "merge".into(),
            ..t
        })
        .is_err());
    }

    /// A fake `gh`: logs each call's arguments, saves stdin for comments, and
    /// answers from the files `repo`, `list`, `view` and `fail` beside it.
    fn fake_gh(
        dir: &Path,
        repo: Option<&str>,
        list: &str,
        view: &str,
        fail: Option<&str>,
    ) -> PathBuf {
        std::fs::write(dir.join("list.json"), list).unwrap();
        std::fs::write(dir.join("view.json"), view).unwrap();
        if let Some(r) = repo {
            std::fs::write(dir.join("repo.json"), r).unwrap();
        }
        if let Some(f) = fail {
            std::fs::write(dir.join("fail.txt"), f).unwrap();
        }
        let d = dir.display();
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{d}/calls.txt'\n\
             if [ \"$1 $2\" = \"repo view\" ]; then [ -f '{d}/repo.json' ] && cat '{d}/repo.json' && exit 0; echo 'could not resolve host' >&2; exit 1; fi\n\
             if [ -f '{d}/fail.txt' ]; then cat '{d}/fail.txt' >&2; exit 1; fi\n\
             case \"$2\" in list) cat '{d}/list.json';; view) cat '{d}/view.json';; comment) cat > '{d}/stdin.txt';; *) :;; esac\n"
        );
        let gh = dir.join("gh");
        std::fs::write(&gh, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        gh
    }

    #[tokio::test]
    async fn a_list_reads_through_gh_and_names_the_repository() {
        let tmp = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let gh = fake_gh(
            tmp.path(),
            Some(r#"{"nameWithOwner":"o/r"}"#),
            PR_LIST,
            "{}",
            None,
        );
        let mut f = q("pr");
        f.label = Some("bug".into());
        let l = list_with(&gh, work.path().to_str().unwrap(), &f, &mirrored)
            .await
            .unwrap();
        assert_eq!(
            (l.repo.as_deref(), l.rows.len(), l.more, l.note.clone()),
            (Some("o/r"), 2, false, None)
        );
        let calls = std::fs::read_to_string(tmp.path().join("calls.txt")).unwrap();
        assert!(
            calls.contains("pr list --state=open --limit 51 --label=bug --json"),
            "{calls}"
        );
    }

    #[tokio::test]
    async fn gh_failures_come_back_as_the_lists_note_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let cwd = work.path().to_str().unwrap();
        // gh can't name the repository (offline): the note says so.
        let gh = fake_gh(tmp.path(), None, "[]", "{}", None);
        let l = list_with(&gh, cwd, &q("issue"), &none).await.unwrap();
        assert!(l.rows.is_empty() && l.repo.is_none());
        assert!(l.note.unwrap().starts_with("Couldn't reach GitHub"));
        // The repository is known but the list is refused (signed out).
        let tmp2 = tempfile::tempdir().unwrap();
        let gh = fake_gh(
            tmp2.path(),
            Some(r#"{"nameWithOwner":"o/r"}"#),
            "[]",
            "{}",
            Some("To get started with GitHub CLI, please run:  gh auth login"),
        );
        let l = list_with(&gh, cwd, &q("pr"), &none).await.unwrap();
        assert_eq!(l.repo.as_deref(), Some("o/r"));
        assert!(l.note.unwrap().contains("gh auth login"));
        // Garbage output is worded, not thrown.
        let tmp3 = tempfile::tempdir().unwrap();
        let gh = fake_gh(
            tmp3.path(),
            Some(r#"{"nameWithOwner":"o/r"}"#),
            "not json",
            "{}",
            None,
        );
        assert!(list_with(&gh, cwd, &q("pr"), &none)
            .await
            .unwrap()
            .note
            .unwrap()
            .contains("couldn't be read"));
        // A bad folder or filter is an error before gh runs.
        assert!(list_with(&gh, "relative", &q("pr"), &none).await.is_err());
        let mut bad = q("pr");
        bad.label = Some("a,b".into());
        assert!(list_with(&gh, cwd, &bad, &none).await.is_err());
        // No gh in the test build: the list says to install it.
        let none_gh = list(cwd, &q("pr"), work.path()).await.unwrap();
        assert!(none_gh.note.unwrap().contains("GitHub CLI"));
        assert!(detail(cwd, "pr", 1, work.path())
            .await
            .unwrap()
            .note
            .unwrap()
            .contains("GitHub CLI"));
        assert!(triage(
            cwd,
            &Triage {
                kind: "pr".into(),
                number: 1,
                action: "close".into(),
                value: None
            }
        )
        .await
        .unwrap_err()
        .contains("GitHub CLI"));
    }

    #[tokio::test]
    async fn a_detail_reads_and_a_missing_item_is_worded() {
        let tmp = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let cwd = work.path().to_str().unwrap();
        let view = r#"{"number":8,"title":"Crash ENG-12","state":"OPEN","url":"https://github.com/o/r/issues/8","body":"b","comments":[]}"#;
        let gh = fake_gh(
            tmp.path(),
            Some(r#"{"nameWithOwner":"o/r"}"#),
            "[]",
            view,
            None,
        );
        let v = detail_with(&gh, cwd, "issue", 8, &mirrored).await.unwrap();
        let d = v.detail.unwrap();
        assert_eq!(
            (d.repo.as_deref(), d.item.number, d.linear[0].tracked),
            (Some("o/r"), 8, true)
        );
        let calls = std::fs::read_to_string(tmp.path().join("calls.txt")).unwrap();
        assert!(calls.contains("issue view 8 --json"), "{calls}");
        assert!(detail_with(&gh, cwd, "issue", 0, &none).await.is_err());
        let tmp2 = tempfile::tempdir().unwrap();
        let gh = fake_gh(
            tmp2.path(),
            Some(r#"{"nameWithOwner":"o/r"}"#),
            "[]",
            "{}",
            Some("GraphQL: Could not resolve to an issue or pull request with the number of 99."),
        );
        let v = detail_with(&gh, cwd, "pr", 99, &none).await.unwrap();
        assert!(v.detail.is_none() && v.note.unwrap().contains("no such"));
    }

    #[tokio::test]
    async fn triage_runs_the_planned_command_and_words_a_refusal() {
        let tmp = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        let cwd = work.path().to_str().unwrap();
        let gh = fake_gh(
            tmp.path(),
            Some(r#"{"nameWithOwner":"o/r"}"#),
            "[]",
            "{}",
            None,
        );
        let done = triage_with(
            &gh,
            cwd,
            &Triage {
                kind: "pr".into(),
                number: 7,
                action: "comment".into(),
                value: Some("Ship it\nplease".into()),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            (done.message.as_str(), done.repo.as_deref()),
            ("Commented on pull request #7", Some("o/r"))
        );
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("stdin.txt")).unwrap(),
            "Ship it\nplease"
        );
        let calls = std::fs::read_to_string(tmp.path().join("calls.txt")).unwrap();
        assert!(
            calls.contains("pr comment 7 --body-file -") && !calls.contains("Ship it"),
            "{calls}"
        );
        triage_with(
            &gh,
            cwd,
            &Triage {
                kind: "issue".into(),
                number: 2,
                action: "label".into(),
                value: Some("bug".into()),
            },
        )
        .await
        .unwrap();
        assert!(std::fs::read_to_string(tmp.path().join("calls.txt"))
            .unwrap()
            .contains("issue edit 2 --add-label=bug"));
        // A check that fails before gh runs leaves gh untouched.
        let before = std::fs::read_to_string(tmp.path().join("calls.txt")).unwrap();
        assert!(triage_with(
            &gh,
            cwd,
            &Triage {
                kind: "pr".into(),
                number: 7,
                action: "comment".into(),
                value: Some(" ".into())
            }
        )
        .await
        .is_err());
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("calls.txt")).unwrap(),
            before
        );
        // GitHub refuses: plain words, and the change did not happen.
        let tmp2 = tempfile::tempdir().unwrap();
        let gh = fake_gh(
            tmp2.path(),
            Some(r#"{"nameWithOwner":"o/r"}"#),
            "[]",
            "{}",
            Some("GraphQL: Resource not accessible by integration (HTTP 403)"),
        );
        let err = triage_with(
            &gh,
            cwd,
            &Triage {
                kind: "pr".into(),
                number: 7,
                action: "close".into(),
                value: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(
            err,
            "Your GitHub sign-in isn't allowed to do that in this repository. Nothing was changed."
        );
    }
}
