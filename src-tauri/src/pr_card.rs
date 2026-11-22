//! The PR tab's data (t3code parity 5.4, D2 and D6): the pull request for the
//! chat folder's branch, with its checks and comments, read through the GitHub
//! CLI the person already has (`gh pr view --json`). Read-only: nothing is
//! created, commented or merged here. A missing `gh`, a branch with no PR, a
//! signed-out `gh` or no network are answers the tab words, never errors that
//! break the card. Comment text is redacted and capped before it leaves Rust.

use crate::build_changes::{gh_binary, gh_command};
use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

/// Most comments and checks the tab lists (the rest are counted).
const COMMENTS_MAX: usize = 30;
const CHECKS_MAX: usize = 60;
/// One comment's body cap, in characters.
const BODY_MAX: usize = 1500;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PrCheck {
    pub name: String,
    /// success, failure, pending or skipped.
    pub state: String,
    pub url: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PrComment {
    pub author: String,
    /// `comment` (the conversation) or `review` (a submitted review).
    pub kind: String,
    /// For a review: APPROVED, CHANGES_REQUESTED, COMMENTED or DISMISSED.
    pub state: Option<String>,
    pub body: String,
    pub created_at: String,
    pub url: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PrDetail {
    pub number: u64,
    pub title: String,
    /// open, merged or closed.
    pub state: String,
    pub url: String,
    pub draft: bool,
    pub review_decision: Option<String>,
    pub merge_state: Option<String>,
    pub head: String,
    pub base: String,
    pub checks: Vec<PrCheck>,
    pub more_checks: usize,
    pub comments: Vec<PrComment>,
    pub more_comments: usize,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PrView {
    pub detail: Option<PrDetail>,
    /// Why there is no detail, in the tab's words; None when there is one.
    pub note: Option<String>,
}

pub(crate) fn s(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

pub(crate) fn opt(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

pub(crate) fn author(v: &Value) -> String {
    v.get("author")
        .and_then(|a| a.get("login"))
        .and_then(Value::as_str)
        .unwrap_or("someone")
        .to_string()
}

/// One check as the person reads it: success, failure, pending or skipped.
pub(crate) fn check_state(item: &Value) -> &'static str {
    // A status context carries `state`; a check run carries `status` and `conclusion`.
    if let Some(state) = item.get("state").and_then(Value::as_str) {
        return match state {
            "SUCCESS" => "success",
            "FAILURE" | "ERROR" => "failure",
            _ => "pending",
        };
    }
    if item.get("status").and_then(Value::as_str) != Some("COMPLETED") {
        return "pending";
    }
    match item.get("conclusion").and_then(Value::as_str).unwrap_or("") {
        "SUCCESS" => "success",
        "SKIPPED" | "NEUTRAL" => "skipped",
        _ => "failure",
    }
}

/// Redact first, then cut: a secret split by the cap must not survive as a fragment.
pub(crate) fn clip(text: &str) -> String {
    crate::claude_runner::redact(text.trim())
        .chars()
        .take(BODY_MAX)
        .collect()
}

/// One PR object as the tab's detail; `None` for anything else (tests read single objects).
#[cfg(test)]
pub(crate) fn parse_detail(json: &str) -> Option<PrDetail> {
    let v: Value = serde_json::from_str(json.trim()).ok()?;
    detail_of(&v)
}

/// The PR for `branch` in `gh pr list --head ... --json ...` output: the one
/// whose head is exactly that branch in this repository (not a fork's same-named
/// branch), the newest first (gh lists newest first), open ones before closed.
pub(crate) fn pick_detail(json: &str, branch: &str) -> Option<PrDetail> {
    let v: Value = serde_json::from_str(json.trim()).ok()?;
    let own: Vec<&Value> = v
        .as_array()?
        .iter()
        .filter(|p| p.get("headRefName").and_then(Value::as_str) == Some(branch))
        .filter(|p| p.get("isCrossRepository").and_then(Value::as_bool) != Some(true))
        .collect();
    own.iter()
        .find(|p| p.get("state").and_then(Value::as_str) == Some("OPEN"))
        .or_else(|| own.first())
        .and_then(|p| detail_of(p))
}

pub(crate) fn detail_of(v: &Value) -> Option<PrDetail> {
    let mut checks: Vec<PrCheck> = v
        .get("statusCheckRollup")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|c| PrCheck {
                    name: opt(c, "name")
                        .or_else(|| opt(c, "context"))
                        .unwrap_or_else(|| "check".into()),
                    state: check_state(c).into(),
                    url: opt(c, "detailsUrl").or_else(|| opt(c, "targetUrl")),
                })
                .collect()
        })
        .unwrap_or_default();
    let more_checks = checks.len().saturating_sub(CHECKS_MAX);
    checks.truncate(CHECKS_MAX);
    let mut comments: Vec<PrComment> = Vec::new();
    for c in v
        .get("comments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        comments.push(PrComment {
            author: author(c),
            kind: "comment".into(),
            state: None,
            body: clip(&s(c, "body")),
            created_at: s(c, "createdAt"),
            url: opt(c, "url"),
        });
    }
    for r in v
        .get("reviews")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let body = clip(&s(r, "body"));
        let state = opt(r, "state");
        // A pending (draft) review is not submitted: not shown.
        if state.as_deref() == Some("PENDING") || opt(r, "submittedAt").is_none() {
            continue;
        }
        // An approval or change request with no text is still worth a line.
        if body.is_empty() && state.as_deref() == Some("COMMENTED") {
            continue;
        }
        comments.push(PrComment {
            author: author(r),
            kind: "review".into(),
            state,
            body,
            created_at: opt(r, "submittedAt").unwrap_or_default(),
            url: opt(r, "url"),
        });
    }
    comments.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    let more_comments = comments.len().saturating_sub(COMMENTS_MAX);
    // Keep the newest ones when there are too many.
    let comments = comments.split_off(more_comments);
    Some(PrDetail {
        number: v.get("number")?.as_u64()?,
        title: s(v, "title"),
        state: s(v, "state").to_lowercase(),
        url: v.get("url")?.as_str()?.to_string(),
        draft: v.get("isDraft").and_then(Value::as_bool).unwrap_or(false),
        review_decision: opt(v, "reviewDecision"),
        merge_state: opt(v, "mergeStateStatus"),
        head: s(v, "headRefName"),
        base: s(v, "baseRefName"),
        checks,
        more_checks,
        comments,
        more_comments,
    })
}

/// The tab's words for a `gh` failure.
pub(crate) fn failure_note(stderr: &str, branch: &str) -> String {
    let lower = stderr.to_lowercase();
    if lower.contains("no pull requests found") || lower.contains("no open pull requests") {
        format!("No pull request for {branch} yet. Push the branch, then open one on GitHub.")
    } else if lower.contains("auth") || lower.contains("log in") || lower.contains("gh auth login")
    {
        "The GitHub CLI isn't signed in. Run `gh auth login` in a terminal, then refresh.".into()
    } else if lower.contains("could not resolve")
        || lower.contains("timeout")
        || lower.contains("network")
    {
        "Couldn't reach GitHub. Check the connection and refresh.".into()
    } else if lower.contains("not a git repository")
        || lower.contains("no git remotes")
        || lower.contains("none of the git remotes")
    {
        "This folder has no GitHub remote.".into()
    } else {
        "GitHub didn't answer for this branch. Refresh to try again.".into()
    }
}

/// The pull request for the branch checked out in `cwd` (`gh pr list --head`).
pub async fn view(cwd: &str) -> Result<PrView, String> {
    let dir = Path::new(cwd);
    if !dir.is_absolute() || !dir.is_dir() {
        return Err("The folder must be an existing absolute path".into());
    }
    let Some(gh) = gh_binary() else {
        return Ok(PrView {
            detail: None,
            note: Some("Install the GitHub CLI (gh) and sign in to see pull requests here.".into()),
        });
    };
    let branch = crate::build_workspaces::git(
        dir,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        crate::build_workspaces::QUICK,
    )
    .await
    .unwrap_or_default();
    if branch.is_empty() {
        return Ok(PrView {
            detail: None,
            note: Some("This folder isn't on a branch, so it has no pull request.".into()),
        });
    }
    // A branch can't be read as an option, and `--head=` keeps a numeric or URL-shaped
    // name from being taken for a PR number or address (`gh pr view <branch>` would).
    if branch.starts_with('-') {
        return Ok(PrView {
            detail: None,
            note: Some("This branch name can't be looked up.".into()),
        });
    }
    let mut cmd = gh_command(&gh, dir);
    let head = format!("--head={branch}");
    cmd.args([
        "pr",
        "list",
        head.as_str(),
        "--state",
        "all",
        "--limit",
        "10",
        "--json",
        "number,title,state,url,isDraft,isCrossRepository,reviewDecision,mergeStateStatus,headRefName,baseRefName,statusCheckRollup,comments,reviews",
    ]);
    let out =
        crate::process_group::run_to_completion(&mut cmd, None, Duration::from_secs(25), None)
            .await
            .map_err(|e| format!("couldn't run gh: {e}"))?;
    if out.timed_out {
        return Ok(PrView {
            detail: None,
            note: Some(failure_note("timeout", &branch)),
        });
    }
    if out.status.is_some_and(|st| st.success()) {
        return Ok(
            match pick_detail(&String::from_utf8_lossy(&out.stdout), &branch) {
                Some(detail) => PrView {
                    detail: Some(detail),
                    note: None,
                },
                // `gh pr list` answers an empty list, not an error, when there is none.
                None => PrView {
                    detail: None,
                    note: Some(failure_note("no pull requests found", &branch)),
                },
            },
        );
    }
    Ok(PrView {
        detail: None,
        note: Some(failure_note(&out.stderr, &branch)),
    })
}

#[tauri::command]
pub async fn pr_card_view(cwd: String) -> Result<PrView, String> {
    view(&cwd).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "number": 42, "title": "Add the thing", "state": "OPEN", "url": "https://github.com/o/r/pull/42",
      "isDraft": true, "reviewDecision": "CHANGES_REQUESTED", "mergeStateStatus": "BLOCKED",
      "headRefName": "topic", "baseRefName": "main",
      "statusCheckRollup": [
        {"__typename":"CheckRun","name":"gate","status":"COMPLETED","conclusion":"SUCCESS","detailsUrl":"https://x/1"},
        {"__typename":"CheckRun","name":"lint","status":"IN_PROGRESS","conclusion":""},
        {"__typename":"CheckRun","name":"e2e","status":"COMPLETED","conclusion":"TIMED_OUT"},
        {"__typename":"CheckRun","name":"docs","status":"COMPLETED","conclusion":"SKIPPED"},
        {"__typename":"StatusContext","context":"ci/legacy","state":"FAILURE","targetUrl":"https://x/2"}
      ],
      "comments": [{"author":{"login":"ann"},"body":"Looks good","createdAt":"2026-11-20T10:00:00Z","url":"https://x/c1"}],
      "reviews": [
        {"author":{"login":"bob"},"body":"","state":"COMMENTED","submittedAt":"2026-11-20T09:00:00Z"},
        {"author":{"login":"cy"},"body":"Please fix","state":"CHANGES_REQUESTED","submittedAt":"2026-11-20T11:00:00Z"},
        {"author":{"login":"me"},"body":"unsent","state":"PENDING"}
      ]
    }"#;

    #[test]
    fn a_pr_parses_with_checks_comments_and_reviews() {
        let d = parse_detail(SAMPLE).unwrap();
        assert_eq!((d.number, d.state.as_str(), d.draft), (42, "open", true));
        assert_eq!(d.review_decision.as_deref(), Some("CHANGES_REQUESTED"));
        let states: Vec<_> = d
            .checks
            .iter()
            .map(|c| (c.name.as_str(), c.state.as_str()))
            .collect();
        assert_eq!(
            states,
            [
                ("gate", "success"),
                ("lint", "pending"),
                ("e2e", "failure"),
                ("docs", "skipped"),
                ("ci/legacy", "failure")
            ]
        );
        assert_eq!(d.checks[4].url.as_deref(), Some("https://x/2"));
        // The empty COMMENTED review is dropped; the rest are oldest first.
        let who: Vec<_> = d
            .comments
            .iter()
            .map(|c| (c.author.as_str(), c.kind.as_str()))
            .collect();
        assert_eq!(who, [("ann", "comment"), ("cy", "review")]);
        assert_eq!(d.comments[1].state.as_deref(), Some("CHANGES_REQUESTED"));
    }

    #[test]
    fn a_pending_review_is_not_listed_and_the_branch_must_match_exactly() {
        let json = r#"[
          {"number":1,"url":"u1","state":"OPEN","headRefName":"topic","isCrossRepository":true},
          {"number":2,"url":"u2","state":"CLOSED","headRefName":"topic","isCrossRepository":false},
          {"number":3,"url":"u3","state":"OPEN","headRefName":"topic","isCrossRepository":false,
            "reviews":[{"author":{"login":"me"},"body":"draft","state":"PENDING"},
                       {"author":{"login":"you"},"body":"ok","state":"APPROVED","submittedAt":"2026-11-20T10:00:00Z"}]},
          {"number":4,"url":"u4","state":"OPEN","headRefName":"topic-2","isCrossRepository":false}
        ]"#;
        let d = pick_detail(json, "topic").unwrap();
        // The fork's PR (1) and another branch's (4) are skipped; an open one beats a closed one.
        assert_eq!(d.number, 3);
        assert_eq!(d.comments.len(), 1);
        assert_eq!(d.comments[0].author, "you");
        assert!(pick_detail("[]", "topic").is_none());
        assert!(pick_detail(json, "other").is_none());
        let only_closed = r#"[{"number":9,"url":"u","state":"MERGED","headRefName":"b","isCrossRepository":false}]"#;
        assert_eq!(pick_detail(only_closed, "b").unwrap().state, "merged");
    }

    #[test]
    fn garbage_is_none_and_long_lists_keep_the_newest() {
        assert!(parse_detail("").is_none());
        assert!(parse_detail("{}").is_none());
        let comments: Vec<String> = (0..35)
            .map(|i| format!(r#"{{"author":{{"login":"u{i}"}},"body":"b","createdAt":"2026-11-20T10:{i:02}:00Z"}}"#))
            .collect();
        let json = format!(
            r#"{{"number":1,"url":"u","comments":[{}]}}"#,
            comments.join(",")
        );
        let d = parse_detail(&json).unwrap();
        assert_eq!((d.comments.len(), d.more_comments), (30, 5));
        assert_eq!(d.comments[0].author, "u5");
        assert_eq!(d.comments[29].author, "u34");
    }

    #[test]
    fn comment_bodies_are_capped() {
        let long = "x".repeat(5000);
        let json = format!(
            r#"{{"number":1,"url":"u","comments":[{{"author":{{"login":"a"}},"body":"{long}","createdAt":"t"}}]}}"#
        );
        assert_eq!(
            parse_detail(&json).unwrap().comments[0]
                .body
                .chars()
                .count(),
            1500
        );
    }

    #[test]
    fn gh_failures_get_the_tabs_words() {
        assert!(
            failure_note("no pull requests found for branch \"topic\"", "topic")
                .starts_with("No pull request for topic")
        );
        assert!(failure_note(
            "To get started with GitHub CLI, please run:  gh auth login",
            "t"
        )
        .contains("gh auth login"));
        assert!(failure_note(
            "dial tcp: lookup api.github.com: could not resolve host",
            "t"
        )
        .starts_with("Couldn't reach GitHub"));
        assert!(failure_note(
            "none of the git remotes configured for this repository point to a known GitHub host",
            "t"
        )
        .contains("no GitHub remote"));
        assert!(failure_note("boom", "t").contains("didn't answer"));
    }

    #[tokio::test]
    async fn a_missing_gh_or_folder_is_worded_not_thrown() {
        assert!(view("relative").await.is_err());
        let dir = tempfile::tempdir().unwrap();
        // Tests never see a real gh (`gh_binary` is None under cfg(test)).
        let v = view(dir.path().to_str().unwrap()).await.unwrap();
        assert!(v.detail.is_none() && v.note.unwrap().contains("GitHub CLI"));
    }
}
