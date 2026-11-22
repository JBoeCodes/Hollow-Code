//! Git contribution heatmap for the Today dashboard.
//!
//! Answers the only question the user actually asked of it — "how many days
//! did I make a commit / work on something" — by scanning the git repos under
//! a root directory (default: the vault's *parent*, since the vault itself
//! isn't a repo but Alex's projects all live beside it) and aggregating
//! commit counts per calendar day over the trailing ~53 weeks, GitHub-style.
//!
//! These are the user's own working copies, so every commit on a day is a
//! "I shipped something" signal — there is no author filter. If a shared repo
//! ever lands under the root that changes the math, the fix is a
//! `--author=<me>` arg here; left out deliberately to keep it zero-config.
//!
//! Pure read. One `git log` per repo, run concurrently, never writes.

use crate::types::{GitActivity, GitDay};
use chrono::{Datelike, Duration, Local, NaiveDate};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;
use tokio::process::Command;

/// Number of week-columns in the grid (GitHub shows 53).
const WEEKS: i64 = 53;

/// TTL cache for the scan, keyed by root. Commit history changes on the
/// scale of hours, but the heatmap resource can refetch far more often
/// (Today's "Refresh all", warm-up). One `git log` sweep per repo every
/// 15 minutes is plenty; a fresh commit shows on the next expiry.
static CACHE: Lazy<Mutex<Option<(PathBuf, Instant, GitActivity)>>> = Lazy::new(|| Mutex::new(None));
const CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(900);

/// Scan `root` for git repos and build the trailing-year contribution grid.
pub async fn collect(root: &Path) -> GitActivity {
    if let Ok(g) = CACHE.lock() {
        if let Some((key, at, cached)) = g.as_ref() {
            if key == root && at.elapsed() < CACHE_TTL {
                return cached.clone();
            }
        }
    }
    let fresh = collect_fresh(root).await;
    if let Ok(mut g) = CACHE.lock() {
        *g = Some((root.to_path_buf(), Instant::now(), fresh.clone()));
    }
    fresh
}

async fn collect_fresh(root: &Path) -> GitActivity {
    let today = Local::now().date_naive();
    // Align the grid to whole week-columns: walk back to the Sunday on/before
    // (today − 52 weeks) so the first column is a full Sun→Sat week, exactly
    // like the GitHub graph.
    let earliest = today - Duration::weeks(WEEKS - 1);
    let start = earliest - Duration::days(earliest.weekday().num_days_from_sunday() as i64);
    let since = start.format("%Y-%m-%d").to_string();

    let repos = discover_repos(root);

    // One `git log` per repo, concurrently — each yields a flat list of
    // local-timezone commit dates.
    let mut handles = Vec::with_capacity(repos.len());
    for repo in &repos {
        let repo = repo.clone();
        let since = since.clone();
        handles.push(tokio::spawn(async move {
            repo_commit_dates(&repo, &since).await
        }));
    }

    let mut counts: HashMap<NaiveDate, u32> = HashMap::new();
    let mut total: usize = 0;
    for h in handles {
        if let Ok(dates) = h.await {
            for d in dates {
                // Clamp to the visible window — `--since` is fuzzy at the edge.
                if d < start || d > today {
                    continue;
                }
                *counts.entry(d).or_insert(0) += 1;
                total += 1;
            }
        }
    }

    // Dense day array start..=today so the frontend just chunks it into 7-row
    // columns without doing any date math of its own.
    let mut days = Vec::new();
    let mut cur = start;
    while cur <= today {
        days.push(GitDay {
            date: cur.format("%Y-%m-%d").to_string(),
            count: counts.get(&cur).copied().unwrap_or(0),
        });
        cur += Duration::days(1);
    }

    let active_days = counts.len();
    let (current_streak, longest_streak) = streaks(&counts, today);

    GitActivity {
        days,
        total_commits: total,
        active_days,
        current_streak,
        longest_streak,
        repos_scanned: repos.len(),
        start: start.format("%Y-%m-%d").to_string(),
        end: today.format("%Y-%m-%d").to_string(),
    }
}

/// Repos to scan: `root` itself if it's a repo, plus every immediate
/// subdirectory that contains a `.git`. One level deep on purpose — we don't
/// want to descend into a repo's own nested worktrees/submodules and
/// double-count.
fn discover_repos(root: &Path) -> Vec<PathBuf> {
    let mut repos = Vec::new();
    if root.join(".git").exists() {
        repos.push(root.to_path_buf());
    }
    if let Ok(rd) = std::fs::read_dir(root) {
        let mut children: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && p.join(".git").exists())
            .collect();
        children.sort();
        repos.extend(children);
    }
    repos
}

/// Local-timezone author dates (`YYYY-MM-DD`) of non-merge commits since
/// `since`. Author date (not committer date) matches GitHub semantics and is
/// stable across rebases. Failures (not a repo, no git binary) yield an empty
/// list rather than blowing up the whole heatmap.
async fn repo_commit_dates(repo: &Path, since: &str) -> Vec<NaiveDate> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .arg("log")
        .arg("--no-merges")
        .arg(format!("--since={since} 00:00:00"))
        .arg("--pretty=format:%ad")
        .arg("--date=format-local:%Y-%m-%d")
        .output()
        .await;
    let Ok(out) = out else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| NaiveDate::parse_from_str(l.trim(), "%Y-%m-%d").ok())
        .collect()
}

/// `(current, longest)` consecutive-active-day streaks.
///
/// Current streak ends today, or yesterday when today has no commit yet — so a
/// real streak isn't reported as broken just because you haven't committed in
/// the last few hours.
fn streaks(counts: &HashMap<NaiveDate, u32>, today: NaiveDate) -> (usize, usize) {
    let mut active: Vec<NaiveDate> = counts.keys().copied().collect();
    active.sort();

    let mut longest = 0usize;
    let mut run = 0usize;
    let mut prev: Option<NaiveDate> = None;
    for d in &active {
        run = match prev {
            Some(p) if *d == p + Duration::days(1) => run + 1,
            _ => 1,
        };
        longest = longest.max(run);
        prev = Some(*d);
    }

    let mut current = 0usize;
    let mut cur = if counts.contains_key(&today) {
        today
    } else {
        today - Duration::days(1)
    };
    while counts.contains_key(&cur) {
        current += 1;
        cur -= Duration::days(1);
    }

    (current, longest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn longest_streak_finds_consecutive_run() {
        let mut c = HashMap::new();
        // A 3-day run, a gap, then a 2-day run.
        for day in [
            "2026-01-01",
            "2026-01-02",
            "2026-01-03",
            "2026-01-10",
            "2026-01-11",
        ] {
            c.insert(d(day), 1);
        }
        let (_, longest) = streaks(&c, d("2026-02-01"));
        assert_eq!(longest, 3);
    }

    #[test]
    fn current_streak_counts_back_from_today() {
        let mut c = HashMap::new();
        for day in ["2026-01-28", "2026-01-29", "2026-01-30"] {
            c.insert(d(day), 2);
        }
        // Today is the 30th — streak of 3 ending today.
        let (current, _) = streaks(&c, d("2026-01-30"));
        assert_eq!(current, 3);
    }

    #[test]
    fn current_streak_tolerates_no_commit_yet_today() {
        let mut c = HashMap::new();
        for day in ["2026-01-28", "2026-01-29"] {
            c.insert(d(day), 1);
        }
        // Nothing committed on the 30th yet — the 2-day streak still stands.
        let (current, _) = streaks(&c, d("2026-01-30"));
        assert_eq!(current, 2);
    }

    #[test]
    fn current_streak_breaks_after_a_gap() {
        let mut c = HashMap::new();
        c.insert(d("2026-01-20"), 1);
        // Last commit was 10 days ago — no current streak.
        let (current, _) = streaks(&c, d("2026-01-30"));
        assert_eq!(current, 0);
    }

    #[test]
    fn empty_history_is_all_zeros() {
        let c: HashMap<NaiveDate, u32> = HashMap::new();
        let (current, longest) = streaks(&c, d("2026-01-30"));
        assert_eq!((current, longest), (0, 0));
    }
}
