//! Publish to Main and Withdraw (updates design 2026-11-14, sections 9 and
//! 10; lane U row U8). Backend only: the Settings, Updates card is built on
//! these commands.
//!
//! - **Who publishes:** a Mac whose update settings say `publisher: true`
//!   ("This Mac publishes to Main", off by default; Alex turns it on on
//!   his own Mac). Row U9's People reads the same field.
//! - **Where it runs:** on the release builder, through this Mac's runner
//!   (`remote_session::configured_runner`, the host `hollow-slot --remote`
//!   uses), as one ssh call. The runner exports `scripts/release` from its
//!   builder clone's `origin/main` into a temporary folder and runs
//!   `promote.mjs` there with the builder's own env file (where the R2 write
//!   credential lives), so this Mac never holds it ([`remote_command`]).
//! - **The proposed build:** the newest Nightly that ran on this Mac at
//!   least an hour without a go back ([`eligible_versions`], from
//!   `run-history.json`, which [`record_run`] keeps at launch and every 30
//!   minutes), unless the person picks another.
//! - **What's new since the last Main:** `notes.mjs` on the runner (the same
//!   code the Nightly notes use), returned by `promote.mjs status`.
//! - **The nudge:** on the publisher Mac, at most every [`NUDGE_CHECK_EVERY`],
//!   one notification per Nightly version when Nightly is 3 days or 20
//!   changes ahead of Main ([`nudge_for`]).
//! - **The agent action** `build.publish_main` is on `ALWAYS_ASK`: its card
//!   ([`publish_card_for`]) names the build and binds its commit, and only
//!   the person's Allow publishes. The Settings button is the person's own
//!   click and calls [`update_publish_main`] directly.

use super::settings::UpdateSettings;
use crate::app_actions::{Args, Card, Cx};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The run history, in the updates folder beside the staging folder.
pub(crate) const RUN_HISTORY: &str = "run-history.json";
/// The nudge's memory (which Nightly it already spoke about).
pub(crate) const NUDGE_FILE: &str = "publish-nudge.json";
/// How long a version must have run here to be proposed.
pub(crate) const MIN_RUN_MS: u64 = 60 * 60 * 1000;
/// Runs kept in the history (oldest dropped).
pub(crate) const MAX_RUNS: usize = 60;
/// A longer silence than this (about two 30-minute ticks) ends a stretch:
/// Hollow was quit or the Mac slept, so the version wasn't running.
pub(crate) const MAX_RUN_GAP_MS: u64 = 2 * 30 * 60 * 1000 + 5 * 60 * 1000;
/// At least this long between two nudges, whatever changed.
pub(crate) const NUDGE_MIN_GAP_MS: u64 = 24 * 60 * 60 * 1000;
/// How long one `promote.mjs` call over ssh may take before it is killed.
pub(crate) const PROMOTE_DEADLINE: Duration = Duration::from_secs(120);
/// How long the runner's `git fetch` of `origin/main` may take.
pub(crate) const REMOTE_FETCH_SECS: u32 = 60;
/// How often the publisher Mac asks the runner whether to nudge.
pub(crate) const NUDGE_CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// The marker line `promote.mjs` prints its result on.
pub(crate) const RESULT_PREFIX: &str = "HOLLOW_PROMOTE_RESULT ";
/// The builder's env file on the runner (docs/UPDATES.md, "The release builder").
pub(crate) const BUILDER_ENV: &str = ".config/hollow-nightly/env";

pub(crate) const NOT_PUBLISHER: &str = "This Mac doesn't publish to Main. Turn on This Mac publishes to Main in Settings, Updates first.";
pub(crate) const NO_RUNNER: &str = "No runner is set up. Publish to Main runs on the release builder through this Mac's runner (Settings, Devices).";

// ── Run history ─────────────────────────────────────────────────────────

/// One stretch of a version running on this Mac.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunRecord {
    pub version: String,
    pub first_seen_ms: u64,
    pub last_seen_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct RunHistory {
    pub runs: Vec<RunRecord>,
}

pub(crate) fn read_runs(path: &Path) -> RunHistory {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Note that `version` runs now: the latest stretch of the same version
/// grows when it was seen within [`MAX_RUN_GAP_MS`]; any other version, or a
/// longer silence, starts a new stretch (coming back to a version starts
/// again, so a stretch is always continuous).
pub(crate) fn record_run(path: &Path, version: &str, now_ms: u64) -> Result<(), String> {
    let mut history = read_runs(path);
    match history.runs.last_mut() {
        Some(last)
            if last.version == version
                && now_ms >= last.first_seen_ms
                && now_ms.saturating_sub(last.last_seen_ms) <= MAX_RUN_GAP_MS =>
        {
            last.last_seen_ms = last.last_seen_ms.max(now_ms);
        }
        _ => history.runs.push(RunRecord {
            version: version.to_string(),
            first_seen_ms: now_ms,
            last_seen_ms: now_ms,
        }),
    }
    if history.runs.len() > MAX_RUNS {
        let extra = history.runs.len() - MAX_RUNS;
        history.runs.drain(..extra);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Could not save the run history: {e}"))?;
    }
    let bytes = serde_json::to_vec_pretty(&history).map_err(|e| e.to_string())?;
    crate::vault::atomic_write(path, &bytes)
        .map_err(|e| format!("Could not save the run history: {e}"))
}

/// `n` of a `0.6.n` channel version.
pub(crate) fn release_number(version: &str) -> Option<u64> {
    let n = version.strip_prefix("0.6.")?;
    if n.is_empty()
        || n.len() > 9
        || !n.bytes().all(|b| b.is_ascii_digit())
        || (n.len() > 1 && n.starts_with('0'))
    {
        return None;
    }
    n.parse().ok()
}

/// Versions that may be proposed, newest first: a channel version that ran
/// here for at least [`MIN_RUN_MS`] in one stretch, and that this Mac didn't
/// go back from. Going back is the version Settings skips (row U6), or any
/// stretch followed later by a lower version (an install of an older build
/// after it, whoever did it).
pub(crate) fn eligible_versions(runs: &[RunRecord], skipped: Option<&str>) -> Vec<String> {
    let mut out: Vec<(u64, String)> = Vec::new();
    for (i, run) in runs.iter().enumerate() {
        let Some(n) = release_number(&run.version) else {
            continue;
        };
        if skipped == Some(run.version.as_str()) {
            continue;
        }
        let went_back = runs[i + 1..]
            .iter()
            .any(|later| release_number(&later.version).is_some_and(|m| m < n));
        if went_back || run.last_seen_ms.saturating_sub(run.first_seen_ms) < MIN_RUN_MS {
            continue;
        }
        if !out.iter().any(|(m, _)| *m == n) {
            out.push((n, run.version.clone()));
        }
    }
    out.sort_by_key(|e| std::cmp::Reverse(e.0));
    out.into_iter().map(|(_, v)| v).collect()
}

// ── The runner ──────────────────────────────────────────────────────────

/// What one `promote.mjs` call printed.
#[derive(Debug, Clone)]
pub(crate) struct RunnerOutput {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Runs `promote.mjs <args>` on the release builder. `Err` only when the
/// runner couldn't be reached or the call couldn't start; a refusal is an
/// `Ok` whose output says so.
pub(crate) trait Runner: Send + Sync {
    fn host(&self) -> &str;
    fn promote(&self, args: &[String]) -> Result<RunnerOutput, String>;
}

/// The runner over ssh, with remote sessions' ssh options (batch mode, no
/// agent forwarding, time limits).
pub(crate) struct SshRunner {
    pub host: String,
}

impl Runner for SshRunner {
    fn host(&self) -> &str {
        &self.host
    }

    fn promote(&self, args: &[String]) -> Result<RunnerOutput, String> {
        use crate::remote_status::{run_bounded, Bounded};
        let (code, stdout, stderr) =
            match run_bounded("ssh", &self.host, &remote_command(args), PROMOTE_DEADLINE) {
                Ok(out) => out,
                Err(Bounded::Start(why)) => return Err(why),
                Err(Bounded::Stalled) => return Err(stalled_message(&self.host)),
            };
        let stderr = crate::claude_runner::redact(stderr.trim());
        if code == Some(255) {
            return Err(crate::remote_session::unreachable_message(
                &self.host, &stderr,
            ));
        }
        Ok(RunnerOutput {
            code,
            stdout,
            stderr,
        })
    }
}

/// What the person reads when the call passed [`PROMOTE_DEADLINE`].
pub(crate) fn stalled_message(host: &str) -> String {
    format!(
        "The runner ({host}) didn't finish within {} s, so Hollow stopped the call. A publish or withdraw may still have finished there: open Settings, Updates again to see what Main serves.",
        PROMOTE_DEADLINE.as_secs()
    )
}

/// The shell command the runner runs: the builder's env file, a fetch of
/// `origin/main` into its clone (stopped after [`REMOTE_FETCH_SECS`]; a
/// failed fetch stops the call with code `fetch`, so nothing runs from a
/// stale checkout), `scripts/release` (and what it imports) exported from
/// that `origin/main` into a temporary folder, then `promote.mjs` with
/// `args`, each shell-quoted. Nothing secret is in it.
pub(crate) fn remote_command(args: &[String]) -> String {
    let quoted: Vec<String> = args
        .iter()
        .map(|a| crate::remote_session::shell_quote(a))
        .collect();
    format!(
        "cfg=\"$HOME/{BUILDER_ENV}\"; \
         if [ ! -r \"$cfg\" ]; then echo '{RESULT_PREFIX}{{\"ok\":false,\"code\":\"config\",\"error\":\"The runner has no release builder set up (no ~/{BUILDER_ENV}).\"}}'; exit 2; fi; \
         set -a; . \"$cfg\"; set +a; \
         r=\"${{HOLLOW_NIGHTLY_REPO:-$HOME/hollow-nightly/repo}}\"; export HOLLOW_NIGHTLY_REPO=\"$r\"; \
         export PATH=\"/opt/homebrew/bin:/usr/local/bin:$PATH\"; \
         t=$(mktemp -d \"${{TMPDIR:-/tmp}}/hollow-promote.XXXXXX\") || exit 2; trap 'rm -rf \"$t\"' EXIT; \
         git -C \"$r\" fetch -q --no-tags --no-prune origin '+refs/heads/main:refs/remotes/origin/main' </dev/null >/dev/null 2>&1 & p=$!; \
         ( sleep {REMOTE_FETCH_SECS}; kill $p ) </dev/null >/dev/null 2>&1 & w=$!; \
         wait $p; f=$?; kill $w 2>/dev/null; \
         if [ $f -ne 0 ]; then printf '%s{{\"ok\":false,\"code\":\"fetch\",\"error\":\"The runner could not fetch origin/main within {REMOTE_FETCH_SECS} s (git exit %s), so nothing ran.\"}}\\n' '{RESULT_PREFIX}' \"$f\"; exit 2; fi; \
         git -C \"$r\" archive refs/remotes/origin/main scripts/release scripts/version.mjs scripts/check-cargo-target.mjs infra/updates-worker/src | tar -x -C \"$t\" || {{ echo '{RESULT_PREFIX}{{\"ok\":false,\"code\":\"config\",\"error\":\"The runner could not export scripts/release from origin/main.\"}}'; exit 2; }}; \
         node \"$t/scripts/release/promote.mjs\" {}",
        quoted.join(" ")
    )
}

/// The result line of `promote.mjs`'s output.
pub(crate) fn parse_result(out: &RunnerOutput) -> Result<Value, String> {
    let line = out
        .stdout
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix(RESULT_PREFIX))
        .ok_or_else(|| {
            let last =
                crate::claude_runner::redact(out.stderr.lines().last().unwrap_or("no output"));
            format!(
                "The runner's promote script said nothing Hollow could read (exit {}): {last}",
                out.code
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "none".into())
            )
        })?;
    serde_json::from_str(line).map_err(|e| format!("The runner's promote result isn't JSON: {e}"))
}

/// The runner this Mac uses, if any.
pub(crate) fn configured() -> Option<SshRunner> {
    crate::remote_session::configured_runner().map(|host| SshRunner { host })
}

// ── IPC types ───────────────────────────────────────────────────────────

/// A live channel's build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct UpdatePublishLive {
    pub version: String,
    pub commit: Option<String>,
    pub pub_date: Option<String>,
}

/// A kept Nightly that can be published (it has a manifest record).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct UpdatePublishBuild {
    pub version: String,
    pub commit: Option<String>,
    pub pub_date: Option<String>,
    /// Ran on this Mac an hour or more without a go back.
    pub ran_here: bool,
    /// Above the live Main.
    pub newer_than_main: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct UpdatePublishPr {
    pub number: u32,
    pub title: String,
    pub merge: String,
    pub merged_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct UpdatePublishWhatsNew {
    pub title: String,
    pub what: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct UpdatePublishCommit {
    pub sha: String,
    pub subject: String,
}

/// What the proposed build brings since the live Main.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct UpdatePublishNotes {
    pub pull_requests: Vec<UpdatePublishPr>,
    pub whats_new: Vec<UpdatePublishWhatsNew>,
    pub commits: Vec<UpdatePublishCommit>,
    /// Why the list is partial ("Only the newest 120 are listed", no Main yet).
    pub note: Option<String>,
}

/// One Main publish or withdraw, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct UpdatePublishHistoryEntry {
    pub version: String,
    /// `publish` or `withdraw`.
    pub action: String,
    pub at: Option<String>,
    /// The version Main served before.
    pub from: Option<String>,
}

/// `update_publish_status`: everything the Publish to Main card shows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct UpdatePublishStatus {
    /// This Mac publishes to Main (`UpdateSettings.publisher`).
    pub publisher: bool,
    /// The runner the promotion runs on, when one is set.
    pub runner: Option<String>,
    /// The runner answered: the lists below are real.
    pub available: bool,
    /// Why not, in words, when not available.
    pub unavailable_reason: Option<String>,
    pub nightly: Option<UpdatePublishLive>,
    pub main: Option<UpdatePublishLive>,
    /// Kept builds with a manifest record, newest first.
    pub builds: Vec<UpdatePublishBuild>,
    /// The build the card proposes (the picker's default, or the pick).
    pub proposed: Option<String>,
    /// Why nothing is proposed.
    pub proposed_why: Option<String>,
    /// What the proposed build brings since the live Main.
    pub notes: Option<UpdatePublishNotes>,
    /// Changes (PRs and direct commits) the live Nightly has over Main.
    pub changes_ahead: Option<u32>,
    /// Days since the oldest build Main lacks was published.
    pub days_ahead: Option<u32>,
    /// Withdraw is possible: there is a Main before the live one.
    pub can_withdraw: bool,
    /// The version Withdraw goes back to.
    pub previous_main: Option<String>,
    pub history: Vec<UpdatePublishHistoryEntry>,
    /// Nightly is far enough ahead for the nudge.
    pub nudge: bool,
}

/// `update_publish_main` and `update_withdraw_main`. Tagged by `outcome`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub enum UpdatePublishOutcome {
    /// Main now serves `version` (it served `from`).
    #[serde(rename_all = "camelCase")]
    Published {
        version: String,
        from: Option<String>,
        at: String,
        /// The manifest is live but the builder's history file wasn't written.
        history_warning: Option<String>,
    },
    /// Main went back to `version` from `from`.
    #[serde(rename_all = "camelCase")]
    Withdrawn {
        version: String,
        from: String,
        at: String,
        history_warning: Option<String>,
    },
    /// Not done. `code`: unchanged, older, no-record, bad-record,
    /// missing-file, signature, commit, no-main, no-previous, moved,
    /// history, busy, config, fetch, usage (nothing was written), or failed
    /// (the result couldn't be read: look at what Main serves).
    #[serde(rename_all = "camelCase")]
    Refused { code: String, reason: String },
}

// ── Status, publish, withdraw (pure over a Runner) ─────────────────────

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

fn live(v: &Value) -> Option<UpdatePublishLive> {
    Some(UpdatePublishLive {
        version: s(v, "version")?,
        commit: s(v, "commit"),
        pub_date: s(v, "pubDate"),
    })
}

fn notes_of(v: &Value) -> Option<UpdatePublishNotes> {
    if !v.is_object() {
        return None;
    }
    let list = |k: &str| {
        v.get(k)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    Some(UpdatePublishNotes {
        pull_requests: list("pullRequests")
            .iter()
            .map(|p| UpdatePublishPr {
                number: p.get("number").and_then(Value::as_u64).unwrap_or(0) as u32,
                title: s(p, "title").unwrap_or_default(),
                merge: s(p, "merge").unwrap_or_default(),
                merged_at: s(p, "mergedAt").unwrap_or_default(),
            })
            .collect(),
        whats_new: list("whatsNew")
            .iter()
            .map(|w| UpdatePublishWhatsNew {
                title: s(w, "title").unwrap_or_default(),
                what: s(w, "what").unwrap_or_default(),
            })
            .collect(),
        commits: list("commits")
            .iter()
            .map(|c| UpdatePublishCommit {
                sha: s(c, "sha").unwrap_or_default(),
                subject: s(c, "subject").unwrap_or_default(),
            })
            .collect(),
        note: s(v, "note"),
    })
}

fn unavailable(
    publisher: bool,
    runner: Option<&str>,
    why: impl Into<String>,
) -> UpdatePublishStatus {
    UpdatePublishStatus {
        publisher,
        runner: runner.map(str::to_string),
        unavailable_reason: Some(why.into()),
        ..UpdatePublishStatus::default()
    }
}

/// The card's status. Asks the runner only for a publisher Mac with one.
pub(crate) fn publish_status(
    settings: &UpdateSettings,
    runner: Option<&dyn Runner>,
    runs: &[RunRecord],
    pick: Option<&str>,
) -> UpdatePublishStatus {
    let host = runner.map(|r| r.host().to_string());
    if !settings.publisher {
        return unavailable(false, host.as_deref(), NOT_PUBLISHER);
    }
    let Some(runner) = runner else {
        return unavailable(true, None, NO_RUNNER);
    };
    let eligible = eligible_versions(runs, settings.skipped_version.as_deref());
    let mut args = vec!["status".to_string()];
    if !eligible.is_empty() {
        args.push("--eligible".into());
        args.push(eligible.join(","));
    }
    if let Some(pick) = pick {
        if release_number(pick).is_none() {
            return unavailable(
                true,
                host.as_deref(),
                format!("{pick} is not a release version (0.6.<n>)."),
            );
        }
        args.push("--pick".into());
        args.push(pick.to_string());
    }
    let v = match runner.promote(&args).and_then(|out| parse_result(&out)) {
        Ok(v) => v,
        Err(why) => return unavailable(true, host.as_deref(), why),
    };
    if v.get("ok").and_then(Value::as_bool) != Some(true) {
        return unavailable(
            true,
            host.as_deref(),
            s(&v, "error").unwrap_or_else(|| "The runner's promote script refused.".into()),
        );
    }
    let main = v.get("main").and_then(live);
    let main_n = main.as_ref().and_then(|m| release_number(&m.version));
    let builds = v
        .get("builds")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|b| {
                    let version = s(b, "version")?;
                    let n = release_number(&version)?;
                    Some(UpdatePublishBuild {
                        ran_here: eligible.contains(&version),
                        newer_than_main: main_n.is_none_or(|m| n > m),
                        version,
                        commit: s(b, "commit"),
                        pub_date: s(b, "pubDate"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let history = v
        .get("history")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|e| {
                    Some(UpdatePublishHistoryEntry {
                        version: s(e, "version")?,
                        action: s(e, "action").unwrap_or_else(|| "publish".into()),
                        at: s(e, "promotedAt"),
                        from: s(e, "from"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let count = |k: &str| {
        v.get(k)
            .and_then(Value::as_u64)
            .map(|n| n.min(u64::from(u32::MAX)) as u32)
    };
    UpdatePublishStatus {
        publisher: true,
        runner: host,
        available: true,
        unavailable_reason: None,
        nightly: v.get("nightly").and_then(live),
        main,
        builds,
        proposed: s(&v, "proposed"),
        proposed_why: s(&v, "proposedWhy"),
        notes: v.get("notes").and_then(notes_of),
        changes_ahead: count("changesAhead"),
        days_ahead: count("daysAhead"),
        can_withdraw: v
            .get("canWithdraw")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        previous_main: s(&v, "previousMain"),
        history,
        nudge: v.get("nudge").and_then(Value::as_bool).unwrap_or(false),
    }
}

/// The typed outcome of one publish or withdraw call. Success needs exit
/// 0, `ok: true`, a release version and a time; anything else is a
/// `failed` refusal, never a `Published` with an empty version.
fn outcome_of(out: &RunnerOutput) -> Result<UpdatePublishOutcome, String> {
    let v = parse_result(out)?;
    let failed = |reason: String| UpdatePublishOutcome::Refused {
        code: "failed".into(),
        reason,
    };
    if v.get("ok").and_then(Value::as_bool) != Some(true) {
        return Ok(UpdatePublishOutcome::Refused {
            code: s(&v, "code").unwrap_or_else(|| "failed".into()),
            reason: s(&v, "error").unwrap_or_else(|| "The runner's promote script refused.".into()),
        });
    }
    if out.code != Some(0) {
        return Ok(failed(format!(
            "The runner's promote script said done but exited {}; check Settings, Updates for what Main serves.",
            out.code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "by a signal".into())
        )));
    }
    let (Some(version), Some(at)) = (
        s(&v, "version").filter(|v| release_number(v).is_some()),
        s(&v, "at").filter(|a| !a.is_empty()),
    ) else {
        return Ok(failed(
            "The runner's promote result names no version or time; check Settings, Updates for what Main serves.".into(),
        ));
    };
    let history_warning = s(&v, "historyError");
    Ok(match s(&v, "action").as_deref() {
        Some("withdraw") => UpdatePublishOutcome::Withdrawn {
            version,
            from: s(&v, "from").unwrap_or_default(),
            at,
            history_warning,
        },
        Some("publish") => UpdatePublishOutcome::Published {
            version,
            from: s(&v, "from"),
            at,
            history_warning,
        },
        _ => failed("The runner's promote result names no action.".into()),
    })
}

fn is_commit(c: &str) -> bool {
    c.len() == 40 && c.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Publish `version` to Main, bound to `commit` (the build the person saw)
/// and `expected_main` (the Main they saw; None: no Main yet), so a
/// different build under the same version, or a Main that moved since, is
/// refused there. Refused (not an error) unless this Mac publishes; an
/// error only when the runner can't be reached.
pub(crate) fn publish_main(
    settings: &UpdateSettings,
    runner: Option<&dyn Runner>,
    version: &str,
    commit: &str,
    expected_main: Option<&str>,
) -> Result<UpdatePublishOutcome, String> {
    let refused = |code: &str, reason: &str| {
        Ok(UpdatePublishOutcome::Refused {
            code: code.into(),
            reason: reason.into(),
        })
    };
    if !settings.publisher {
        return refused("not-publisher", NOT_PUBLISHER);
    }
    let Some(runner) = runner else {
        return refused("no-runner", NO_RUNNER);
    };
    if release_number(version).is_none() {
        return refused("usage", "That is not a release version (0.6.<n>).");
    }
    if !is_commit(commit) {
        return refused(
            "usage",
            "The expected commit must be 40 lowercase hex characters.",
        );
    }
    if expected_main.is_some_and(|m| release_number(m).is_none()) {
        return refused(
            "usage",
            "The expected Main is not a release version (0.6.<n>).",
        );
    }
    let args = vec![
        "promote".to_string(),
        version.to_string(),
        "--expect-commit".into(),
        commit.to_string(),
        "--expect-main".into(),
        expected_main.unwrap_or("none").to_string(),
    ];
    outcome_of(&runner.promote(&args)?)
}

/// Point Main back at the previous Main. `expected`: the Main version the
/// person saw (refused when Main moved since).
pub(crate) fn withdraw_main(
    settings: &UpdateSettings,
    runner: Option<&dyn Runner>,
    expected: Option<&str>,
) -> Result<UpdatePublishOutcome, String> {
    if !settings.publisher {
        return Ok(UpdatePublishOutcome::Refused {
            code: "not-publisher".into(),
            reason: NOT_PUBLISHER.into(),
        });
    }
    let Some(runner) = runner else {
        return Ok(UpdatePublishOutcome::Refused {
            code: "no-runner".into(),
            reason: NO_RUNNER.into(),
        });
    };
    let mut args = vec!["withdraw".to_string()];
    if let Some(v) = expected {
        if release_number(v).is_none() {
            return Ok(UpdatePublishOutcome::Refused {
                code: "usage".into(),
                reason: "That is not a release version (0.6.<n>).".into(),
            });
        }
        args.push("--expect-version".into());
        args.push(v.to_string());
    }
    outcome_of(&runner.promote(&args)?)
}

// ── The nudge ───────────────────────────────────────────────────────────

/// What the nudge remembers between checks.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct NudgeState {
    /// The Nightly version the last nudge named.
    pub nudged_for: Option<String>,
    /// The Main version the last nudge compared against (`none`: no Main).
    pub nudged_main: Option<String>,
    /// When the last nudge was posted (ms since the epoch).
    pub last_nudge_ms: Option<u64>,
    /// The last time it asked the runner (ms since the epoch).
    pub last_check_ms: Option<u64>,
}

pub(crate) fn read_nudge(path: &Path) -> NudgeState {
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn write_nudge(path: &Path, state: &NudgeState) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(state) {
        let _ = crate::vault::atomic_write(path, &bytes);
    }
}

/// Whether it is time to ask the runner again.
pub(crate) fn nudge_check_due(state: &NudgeState, now_ms: u64) -> bool {
    state
        .last_check_ms
        .is_none_or(|last| now_ms < last || now_ms - last >= NUDGE_CHECK_EVERY.as_millis() as u64)
}

/// The Main a nudge compares against, as [`NudgeState::nudged_main`] keeps it.
fn main_key(status: &UpdatePublishStatus) -> String {
    status
        .main
        .as_ref()
        .map(|m| m.version.clone())
        .unwrap_or_else(|| "none".into())
}

/// The notification for `status`: `(nightly version, title, body)`, or None
/// when Nightly isn't far enough ahead, a nudge already spoke about this
/// Main (a new Nightly every night would otherwise nudge daily until Main
/// moves), or the last nudge was under [`NUDGE_MIN_GAP_MS`] ago.
pub(crate) fn nudge_for(
    status: &UpdatePublishStatus,
    state: &NudgeState,
    now_ms: u64,
) -> Option<(String, String, String)> {
    if !status.available || !status.nudge {
        return None;
    }
    let nightly = status.nightly.as_ref()?;
    if state.nudged_main.as_deref() == Some(main_key(status).as_str()) {
        return None;
    }
    if state
        .last_nudge_ms
        .is_some_and(|last| now_ms >= last && now_ms - last < NUDGE_MIN_GAP_MS)
    {
        return None;
    }
    let main = status
        .main
        .as_ref()
        .map(|m| m.version.as_str())
        .unwrap_or("no Main yet");
    let title = match (status.changes_ahead, status.days_ahead) {
        (Some(n), _) if n > 0 => {
            format!("Nightly is {n} changes ahead of Main: review and publish")
        }
        (_, Some(d)) => format!("Nightly is {d} days ahead of Main: review and publish"),
        _ => "Nightly is ahead of Main: review and publish".to_string(),
    };
    let body = format!(
        "Nightly {} is live; Main is {main}. Settings, Updates, Publish to Main shows the proposed build and what's new.",
        nightly.version
    );
    Some((nightly.version.clone(), title, body))
}

/// One tick on a publisher Mac: note the running version, and at most every
/// [`NUDGE_CHECK_EVERY`] ask the runner and post the nudge ([`nudge_for`]).
pub(crate) fn tick(
    settings: &UpdateSettings,
    paths: &PublishPaths,
    runner: Option<&dyn Runner>,
    running_version: &str,
    now_ms: u64,
    notify: &mut dyn FnMut(String, String),
) {
    let _ = record_run(&paths.runs, running_version, now_ms);
    if !settings.publisher || runner.is_none() {
        return;
    }
    let mut state = read_nudge(&paths.nudge);
    if !nudge_check_due(&state, now_ms) {
        return;
    }
    let status = publish_status(settings, runner, &read_runs(&paths.runs).runs, None);
    if !status.available {
        // Try again at the next tick rather than in six hours.
        return;
    }
    state.last_check_ms = Some(now_ms);
    if let Some((version, title, body)) = nudge_for(&status, &state, now_ms) {
        notify(title, body);
        state.nudged_for = Some(version);
        state.nudged_main = Some(main_key(&status));
        state.last_nudge_ms = Some(now_ms);
    }
    write_nudge(&paths.nudge, &state);
}

/// Where the run history and the nudge's memory live.
#[derive(Debug, Clone)]
pub(crate) struct PublishPaths {
    pub runs: PathBuf,
    pub nudge: PathBuf,
}

impl PublishPaths {
    pub(crate) fn in_dir(updates: &Path) -> Self {
        Self {
            runs: updates.join(RUN_HISTORY),
            nudge: updates.join(NUDGE_FILE),
        }
    }

    fn real() -> Option<Self> {
        crate::agent_settings::app_data_dir()
            .ok()
            .map(|d| Self::in_dir(&d.join("updates")))
    }
}

/// Start the publish tick (launch, then every 30 minutes) beside the update
/// client's schedule. Not in a review copy.
pub(crate) fn start(app: tauri::AppHandle, client: std::sync::Arc<super::client::Client>) {
    use tauri::Manager;
    let Some(paths) = PublishPaths::real() else {
        return;
    };
    let version = crate::app_menu::build_identity().version;
    let _ = std::thread::Builder::new()
        .name("hollow-publish".into())
        .spawn(move || {
            std::thread::sleep(super::client::LAUNCH_DELAY);
            loop {
                let settings = client.settings();
                let runner = configured();
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let vault = app
                    .try_state::<crate::commands::AppState>()
                    .map(|s| s.config.vault_path.clone());
                let mut notify = |title: String, body: String| {
                    if let Some(vault) = &vault {
                        let _ = crate::notifications::create(
                            &app,
                            vault,
                            crate::notifications::NotificationInput {
                                title,
                                body,
                                kind: "info".into(),
                                source: Some("updates.publish".into()),
                                action_label: Some("Review".into()),
                                action_url: Some("settings:updates".into()),
                                notify_native: true,
                            },
                        );
                    }
                };
                tick(
                    &settings,
                    &paths,
                    runner.as_ref().map(|r| r as &dyn Runner),
                    &version,
                    now,
                    &mut notify,
                );
                std::thread::sleep(super::client::EVERY);
            }
        });
}

// ── The agent action's card ─────────────────────────────────────────────

/// How many PR titles the card lists before "and N more".
pub(crate) const CARD_PRS: usize = 10;

/// What a card binds: the build and its commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PublishBind {
    pub version: String,
    pub commit: String,
    /// The Main the card showed (None: no Main yet).
    #[serde(default)]
    pub main: Option<String>,
}

/// The `build.publish_main` card for `version`, from a status picked at it.
pub(crate) fn publish_card_for(
    status: &UpdatePublishStatus,
    version: &str,
) -> Result<Card, String> {
    if !status.publisher {
        return Err(format!("{NOT_PUBLISHER} Nothing was published."));
    }
    if !status.available {
        return Err(format!(
            "{} Nothing was published.",
            status
                .unavailable_reason
                .as_deref()
                .unwrap_or("The runner didn't answer.")
        ));
    }
    let build = status
        .builds
        .iter()
        .find(|b| b.version == version)
        .ok_or_else(|| format!("{version} isn't a kept Nightly with a manifest record (build.publish_status lists them). Nothing was published."))?;
    let commit = build
        .commit
        .clone()
        .filter(|c| is_commit(c))
        .ok_or_else(|| format!("{version}'s manifest names no commit. Nothing was published."))?;
    if status.main.as_ref().is_some_and(|m| m.version == version) {
        return Err(format!(
            "Main already serves {version}. Nothing was published."
        ));
    }
    if !build.newer_than_main {
        return Err(format!(
            "{version} is older than the live Main {}. Withdraw (Settings, Updates) is the way back. Nothing was published.",
            status.main.as_ref().map(|m| m.version.as_str()).unwrap_or("?")
        ));
    }
    let mut card = Card::new("Hollow app tools: publish to Main")
        .line(format!(
            "Publish Hollow {version} (commit {}) to Main: every Mac on the Main channel is offered it at its next check.",
            &commit[..12]
        ))
        .line(match &status.main {
            Some(m) => format!("Main now serves {}.", m.version),
            None => "Nothing has been published to Main yet.".to_string(),
        });
    if !build.ran_here {
        card = card.line("It hasn't run on this Mac for an hour yet.");
    }
    if let Some(notes) = &status.notes {
        card = card.line(format!(
            "What's new since Main: {} pull requests, {} What's new entries, {} direct commits.",
            notes.pull_requests.len(),
            notes.whats_new.len(),
            notes.commits.len()
        ));
        for pr in notes.pull_requests.iter().take(CARD_PRS) {
            card = card.line(format!(
                "  #{} {}",
                pr.number,
                crate::self_install::short(&pr.title, 120)
            ));
        }
        if notes.pull_requests.len() > CARD_PRS {
            card = card.line(format!(
                "  and {} more",
                notes.pull_requests.len() - CARD_PRS
            ));
        }
    }
    card = card
        .line(format!(
            "It runs on the release builder through {}: the build's own manifest is copied to channels/main.json, no rebuild, the same signed bytes. Allowing publishes exactly this commit.",
            status.runner.as_deref().unwrap_or("the runner")
        ))
        .to("the update host: channels/main.json (every Mac on Main)");
    card.bind = Some(
        serde_json::to_value(PublishBind {
            version: version.to_string(),
            commit,
            main: status.main.as_ref().map(|m| m.version.clone()),
        })
        .map_err(|e| e.to_string())?,
    );
    Ok(card)
}

fn this_mac_settings() -> Result<UpdateSettings, String> {
    super::client().map(|c| c.settings()).ok_or_else(|| {
        "Hollow's updater isn't running in this window. Nothing was published.".to_string()
    })
}

/// `build.publish_main`'s card builder.
pub(crate) fn publish_main_card(_cx: &Cx, a: &Args) -> Result<Option<Card>, String> {
    let version =
        a.0.get("version")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
    if release_number(&version).is_none() {
        return Err(format!(
            "{version} is not a release version (0.6.<n>). Nothing was published."
        ));
    }
    let settings = this_mac_settings()?;
    let runner = configured();
    let runs = PublishPaths::real()
        .map(|p| read_runs(&p.runs).runs)
        .unwrap_or_default();
    let status = publish_status(
        &settings,
        runner.as_ref().map(|r| r as &dyn Runner),
        &runs,
        Some(&version),
    );
    publish_card_for(&status, &version).map(Some)
}

/// The build the card bound, checked against the call.
pub(crate) fn approved_publish(
    approval: Option<&Card>,
    version: &str,
) -> Result<PublishBind, String> {
    let bind: PublishBind = approval
        .and_then(|card| card.bind.clone())
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| e.to_string())?
        .ok_or(
            "build.publish_main runs only with the build its card recorded. Nothing was published.",
        )?;
    if bind.version != version {
        return Err(format!(
            "The approved build is {}, not {version}. Nothing was published.",
            bind.version
        ));
    }
    Ok(bind)
}

/// `build.publish_main`'s handler: only after the person allowed its card,
/// bound to the card's commit.
pub(crate) async fn publish_main_action(cx: &Cx, a: &Args) -> Result<Value, String> {
    let version =
        a.0.get("version")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
    let bind = approved_publish(cx.approval.as_ref(), &version)?;
    let outcome = crate::vault::spawn_blocking_logged(move || {
        let settings = this_mac_settings()?;
        let runner = configured();
        publish_main(
            &settings,
            runner.as_ref().map(|r| r as &dyn Runner),
            &bind.version,
            &bind.commit,
            bind.main.as_deref(),
        )
    })
    .await
    .map_err(|e| e.to_string())??;
    serde_json::to_value(outcome).map_err(|e| e.to_string())
}

// ── Tauri commands ──────────────────────────────────────────────────────

/// Run `f` on a plain thread and wait for it (ssh never holds the event loop).
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("hollow-publish-call".into())
        .spawn(move || {
            let _ = tx.send(f());
        })
        .map_err(|e| format!("Could not start the publish call: {e}"))?;
    rx.await
        .map_err(|_| "The publish call stopped.".to_string())?
}

fn settings_now() -> Result<UpdateSettings, String> {
    super::client()
        .map(|c| c.settings())
        .ok_or_else(|| "Hollow's updater isn't running in this window.".to_string())
}

/// The Publish to Main card's status. `pick`: the version the picker
/// chose (None: the proposed default). Asks the runner (one ssh call).
#[tauri::command]
pub async fn update_publish_status(pick: Option<String>) -> Result<UpdatePublishStatus, String> {
    blocking(move || {
        let settings = settings_now()?;
        let runner = configured();
        let runs = PublishPaths::real()
            .map(|p| read_runs(&p.runs).runs)
            .unwrap_or_default();
        Ok(publish_status(
            &settings,
            runner.as_ref().map(|r| r as &dyn Runner),
            &runs,
            pick.as_deref(),
        ))
    })
    .await
}

/// Publish: the person's click on the card (it is the approval). `commit`
/// (required) is the commit the card showed for that build and
/// `expected_main` the Main it showed (None: no Main yet), so a different
/// build under the same version, or a Main that moved, is refused.
#[tauri::command]
pub async fn update_publish_main(
    version: String,
    commit: String,
    expected_main: Option<String>,
) -> Result<UpdatePublishOutcome, String> {
    blocking(move || {
        let settings = settings_now()?;
        let runner = configured();
        publish_main(
            &settings,
            runner.as_ref().map(|r| r as &dyn Runner),
            &version,
            &commit,
            expected_main.as_deref(),
        )
    })
    .await
}

/// Withdraw: point Main back at the previous Main. `expected`: the Main
/// version the card showed.
#[tauri::command]
pub async fn update_withdraw_main(
    expected: Option<String>,
) -> Result<UpdatePublishOutcome, String> {
    blocking(move || {
        let settings = settings_now()?;
        let runner = configured();
        withdraw_main(
            &settings,
            runner.as_ref().map(|r| r as &dyn Runner),
            expected.as_deref(),
        )
    })
    .await
}

/// "This Mac publishes to Main".
#[tauri::command]
pub async fn update_set_publisher(enabled: bool) -> Result<UpdateSettings, String> {
    blocking(move || {
        super::client()
            .ok_or_else(|| "Hollow's updater isn't running in this window.".to_string())?
            .set_publisher(enabled)
    })
    .await
}
