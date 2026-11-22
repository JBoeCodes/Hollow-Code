//! Vault-wide call benchmark (overnight A14): a synthetic vault of
//! `HOLLOW_VAULT_BENCH_WORKSPACES` workspaces (default 500) and
//! `HOLLOW_VAULT_BENCH_NOTES` notes (default 30,000), and how long the
//! vault-wide list and scan calls take on it. Run on purpose:
//!
//! ```sh
//! export TMPDIR=/tmp
//! cargo test --lib vault_bench::wide_calls -- --ignored --nocapture
//! ```
//!
//! It prints one line per call (median and worst of `HOLLOW_VAULT_BENCH_RUNS`,
//! default 5) and writes nothing outside its temp folder. The link-index
//! build stands in for cold start (the launch reads every note once).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn env_or(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// A brief like a real one: frontmatter, then a few KB of context.
pub(crate) fn brief(i: usize) -> String {
    let mut body = String::from("## Outcome\n\nShip the thing.\n\n## Current context\n\n");
    for line in 0..60 {
        body.push_str(&format!(
            "- 2026-10-0{} note {line}: progress on item {i}, with links [[Note {}]] and some detail to read.\n",
            1 + line % 9,
            i * 7 + line
        ));
    }
    format!(
        "---\nhollow_workspace: 1\ntitle: \"Workspace {i} report\"\nstatus: active\ncreated: 2026-09-01T00:00:00Z\n---\n\n# Workspace {i}\n\n{body}"
    )
}

/// Build the synthetic vault under `root`: `workspaces` workspaces (brief,
/// plan, a few outputs) and `notes` notes spread over folders.
pub(crate) fn synthetic_vault(root: &Path, workspaces: usize, notes: usize) {
    for i in 0..workspaces {
        let dir = root.join(format!("workspaces/ws-{i:04}"));
        std::fs::create_dir_all(dir.join("outputs")).unwrap();
        std::fs::write(dir.join("WORKSPACE.md"), brief(i)).unwrap();
        let plan: String = (0..12)
            .map(|r| {
                format!(
                    "- [{}] Step {r} of workspace {i}\n",
                    if r % 3 == 0 { "x" } else { " " }
                )
            })
            .collect();
        std::fs::write(dir.join("PLAN.md"), format!("# Plan\n\n{plan}")).unwrap();
        for o in 0..3 {
            std::fs::write(
                dir.join(format!("outputs/result-{o}.md")),
                format!("# Result {o}\n\nText.\n"),
            )
            .unwrap();
        }
    }
    for n in 0..notes {
        let dir = root.join(format!("notes/folder-{:03}", n % 300));
        if n < 300 {
            std::fs::create_dir_all(&dir).unwrap();
        }
        std::fs::write(
            dir.join(format!("Note {n}.md")),
            format!(
                "---\ntitle: Note {n}\n---\n\nA note linking [[Note {}]] and [[Workspace {}]].\n",
                n / 2,
                n % 500
            ),
        )
        .unwrap();
    }
}

/// Median and worst of `runs` timings of `f`.
pub(crate) fn time(runs: usize, mut f: impl FnMut()) -> (Duration, Duration) {
    let mut all: Vec<Duration> = (0..runs.max(1))
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed()
        })
        .collect();
    all.sort();
    (all[all.len() / 2], *all.last().unwrap())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "the vault-wide benchmark: run on purpose (see the module doc)"]
async fn wide_calls() {
    let workspaces = env_or("HOLLOW_VAULT_BENCH_WORKSPACES", 500);
    let notes = env_or("HOLLOW_VAULT_BENCH_NOTES", 30_000);
    let runs = env_or("HOLLOW_VAULT_BENCH_RUNS", 5);
    // Not `.tmp…`: the vault walk skips hidden folders, its root too.
    let dir = tempfile::Builder::new()
        .prefix("vault-bench-")
        .tempdir()
        .unwrap();
    let v: PathBuf = dir.path().to_path_buf();
    let t = Instant::now();
    synthetic_vault(&v, workspaces, notes);
    println!(
        "fixture: {workspaces} workspaces, {notes} notes in {:?}",
        t.elapsed()
    );
    let now = chrono::Utc::now();
    let line = |name: &str, (median, worst): (Duration, Duration)| {
        println!(
            "{name:<34} median {:>8.1} ms   worst {:>8.1} ms",
            median.as_secs_f64() * 1e3,
            worst.as_secs_f64() * 1e3
        );
    };
    line(
        "workspaces::list",
        time(runs, || {
            assert_eq!(crate::workspaces::list(&v).unwrap().len(), workspaces);
        }),
    );
    line(
        "vault::walk_notes (cold)",
        time(runs, || {
            crate::vault::invalidate_walk_cache();
            assert!(crate::vault::walk_notes(&v).len() >= notes);
        }),
    );
    line(
        "vault::build_index (cold start)",
        time(runs, || {
            let _ = crate::vault::build_index(&v);
        }),
    );
    line(
        "gardener::scan",
        time(runs, || {
            crate::gardener::scan(&v, now).unwrap();
        }),
    );
    line(
        "conflict_copies::scan",
        time(runs, || {
            crate::vault::invalidate_walk_cache();
            crate::conflict_copies::scan(&v);
        }),
    );
    line(
        "needs_you::list",
        time(runs, || {
            crate::needs_you::list(&v, Vec::new(), None, None, Some(50)).unwrap();
        }),
    );
}

/// Where `build_index` spends its time (overnight A14): reading, links,
/// titles. `cargo test --lib vault_bench::index_phases -- --ignored --nocapture`.
#[test]
#[ignore = "the vault-wide benchmark: run on purpose (see the module doc)"]
fn index_phases() {
    let notes = env_or("HOLLOW_VAULT_BENCH_NOTES", 30_000);
    let dir = tempfile::Builder::new()
        .prefix("vault-bench-")
        .tempdir()
        .unwrap();
    let v: PathBuf = dir.path().to_path_buf();
    synthetic_vault(&v, 500, notes);
    crate::vault::invalidate_walk_cache();
    let list = crate::vault::walk_notes(&v);
    let t = Instant::now();
    let texts: Vec<String> = list
        .iter()
        .map(|n| std::fs::read_to_string(v.join(&n.path)).unwrap_or_default())
        .collect();
    println!(
        "read {} files       {:>8.1} ms",
        texts.len(),
        t.elapsed().as_secs_f64() * 1e3
    );
    let t = Instant::now();
    let links: usize = texts
        .iter()
        .map(|c| crate::vault::parse_wikilinks(c).len())
        .sum();
    println!(
        "parse_wikilinks ({links})   {:>8.1} ms",
        t.elapsed().as_secs_f64() * 1e3
    );
    let t = Instant::now();
    for (n, c) in list.iter().zip(&texts) {
        let _ = crate::vault::note_title_and_summary(c, &n.name);
    }
    println!(
        "titles             {:>8.1} ms",
        t.elapsed().as_secs_f64() * 1e3
    );
    let t = Instant::now();
    let _ = crate::vault::build_index(&v);
    println!(
        "build_index        {:>8.1} ms",
        t.elapsed().as_secs_f64() * 1e3
    );
}

/// What each Waiting on you source costs (overnight A14).
/// `cargo test --lib vault_bench::needs_you_kinds -- --ignored --nocapture`.
#[test]
#[ignore = "the vault-wide benchmark: run on purpose (see the module doc)"]
fn needs_you_kinds() {
    let dir = tempfile::Builder::new()
        .prefix("vault-bench-")
        .tempdir()
        .unwrap();
    let v: PathBuf = dir.path().to_path_buf();
    synthetic_vault(
        &v,
        env_or("HOLLOW_VAULT_BENCH_WORKSPACES", 500),
        env_or("HOLLOW_VAULT_BENCH_NOTES", 30_000),
    );
    for kind in [
        "approval",
        "question",
        "review",
        "result",
        "failure",
        "conflict",
        "canvas",
        "interrupted",
        "copies",
    ] {
        let kinds = [kind.to_string()];
        let (median, _) = time(3, || {
            crate::needs_you::list(&v, Vec::new(), Some(&kinds), None, Some(50)).unwrap();
        });
        println!(
            "needs_you {kind:<12} {:>8.1} ms",
            median.as_secs_f64() * 1e3
        );
    }
}

/// Guards (overnight A14, run in the gate). A cold link-index build of
/// 3,000 notes takes the threaded path where the Mac has cores, and stays
/// under a generous bound (a return to quadratic or per-link work fails it).
#[test]
fn a_cold_link_index_of_3000_notes_is_threaded_and_fast() {
    let dir = tempfile::Builder::new()
        .prefix("vault-bench-")
        .tempdir()
        .unwrap();
    let v: PathBuf = dir.path().to_path_buf();
    synthetic_vault(&v, 50, 3_000);
    crate::vault::invalidate_walk_cache();
    let threaded = crate::vault::INDEX_THREADED_BUILDS.load(std::sync::atomic::Ordering::Relaxed);
    let t = Instant::now();
    let idx = crate::vault::build_index(&v);
    let took = t.elapsed();
    assert!(idx.by_path.len() >= 3_000);
    // Generous for a loaded gate; a return to quadratic work takes far longer.
    assert!(took < Duration::from_secs(10), "build_index took {took:?}");
    if std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        > 1
    {
        assert!(
            crate::vault::INDEX_THREADED_BUILDS.load(std::sync::atomic::Ordering::Relaxed)
                > threaded,
            "the threaded path"
        );
    }
}

/// The hand wikilink scan stays faster than the regex it replaced, measured
/// side by side: rounds alternate the two, and each keeps its best, so load
/// affects both alike.
#[test]
fn the_wikilink_scan_stays_faster_than_the_regex() {
    let text: String = (0..2_000)
        .map(|i| {
            format!(
                "Line {i} links [[Note {i}]] and [[Other {}|alias]] and text.\n",
                i / 3
            )
        })
        .collect();
    let regex = regex::Regex::new(r"\[\[([^\]\n|#]+?)(?:\|[^\]\n]+)?(?:#[^\]\n]+)?\]\]").unwrap();
    let once = |f: &dyn Fn() -> usize| {
        let t = Instant::now();
        assert!(f() > 0);
        t.elapsed()
    };
    let (mut hand, mut old) = (Duration::MAX, Duration::MAX);
    for _ in 0..11 {
        hand = hand.min(once(&|| crate::vault::parse_wikilinks(&text).len()));
        old = old.min(once(&|| {
            regex.captures_iter(&text).filter_map(|c| c.get(1)).count()
        }));
    }
    assert!(hand < old, "hand scan {hand:?} vs regex {old:?}");
}
