//! Five asks B4: timings of the IPC hot paths over a synthetic vault at the
//! scale of the owner's (215 workspaces with ~1.4 KB briefs and one 53 KB
//! brief, 150 conversations of 4 turns, ~1.5 MB of chat receipts, 500 tasks).
//!
//! `perf_hot_paths_report` prints the numbers and is `#[ignore]`d (its
//! fixture writes ~50 MB of transcripts):
//!   cd src-tauri && TMPDIR=/tmp cargo test --lib perf_hot_paths_report -- --ignored --nocapture
//! The other tests run in every gate on a lighter fixture with ceilings about
//! 3× the measured value, so a regression of the paths below fails loudly
//! while machine noise doesn't; with `--nocapture` (or when it fails) the
//! gate prints its first timing of each path too. The gate judges each path on the calling
//! thread's CPU time, not wall time, so a loaded machine's run-queue wait
//! never fails it (OF10; see `WALL_JUDGED` for the exception).
#![cfg(test)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::evidence::{EvidenceReceipt, ReceiptFile};

pub(crate) struct Scale {
    pub workspaces: usize,
    pub sessions: usize,
    pub turns: usize,
    pub transcript_bytes: usize,
    pub receipts: usize,
    pub files_per_receipt: usize,
    pub tasks: usize,
}

pub(crate) const OWNER: Scale = Scale {
    workspaces: 215,
    sessions: 150,
    turns: 4,
    transcript_bytes: 80_000,
    receipts: 500,
    files_per_receipt: 20,
    tasks: 500,
};

pub(crate) const GATE: Scale = Scale {
    workspaces: 215,
    sessions: 150,
    turns: 4,
    transcript_bytes: 4_000,
    receipts: 500,
    files_per_receipt: 20,
    tasks: 500,
};

pub(crate) fn fixture(name: &str, scale: &Scale) -> PathBuf {
    let vault = tempfile::Builder::new()
        .prefix(&format!("hollow-perf-{name}-"))
        .tempdir()
        .expect("temp vault")
        .keep();
    let vault = std::fs::canonicalize(&vault).expect("canonical vault");
    let body =
        "Keep the brief current: outcome, context, next step and decisions with their reason. "
            .repeat(16);
    let mut cwds = Vec::new();
    for i in 0..scale.workspaces {
        let id = format!("local-perf-{i:04}");
        let dir = vault.join("workspaces").join(&id);
        std::fs::create_dir_all(dir.join("outputs")).unwrap();
        let extra = if i == 7 {
            body.repeat(38)
        } else {
            String::new()
        };
        std::fs::write(
            dir.join("WORKSPACE.md"),
            format!(
                "---\nhollow_workspace: 1\ntitle: \"Location {i} operating plan\"\nstatus: active\ncreated: 2026-09-24T21:24:35+00:00\n---\n\n# Location {i}\n\n## Outcome\n\n{body}{extra}\n"
            ),
        )
        .unwrap();
        std::fs::write(
            dir.join("PLAN.md"),
            "# Plan\n\n- [ ] Review the layout\n- [x] Draft the brief\n",
        )
        .unwrap();
        cwds.push(dir.to_string_lossy().to_string());
    }
    let tasks = vault.join("tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    for i in 0..scale.tasks {
        std::fs::write(
            tasks.join(format!("TASK-{i}.md")),
            format!("---\ntitle: \"Task {i}: check the variance report\"\nstatus: {}\npriority: medium\ndue: 2026-10-{:02}\n---\n\nNotes for task {i}.\n", if i % 3 == 0 { "done" } else { "todo" }, 1 + i % 28),
        )
        .unwrap();
    }
    crate::session_store::seed_for_perf(
        &vault,
        &cwds,
        scale.sessions,
        scale.turns,
        scale.transcript_bytes,
    )
    .expect("seed sessions");
    let receipts = (0..scale.receipts)
        .map(|i| {
            let session = i % scale.sessions.max(1);
            EvidenceReceipt {
                id: format!("receipt-{i:020}"),
                run_id: format!("run-{session}-{}", i % scale.turns.max(1)),
                source: "chat".into(),
                started_at: format!("2026-10-10T{:02}:{:02}:00+00:00", i / 60 % 24, i % 60),
                finished_at: format!("2026-10-10T{:02}:{:02}:30+00:00", i / 60 % 24, i % 60),
                status: "succeeded".into(),
                verification_state: "unconfirmed".into(),
                session_key: Some(format!("s-20261010-{session:08x}")),
                files: (0..scale.files_per_receipt)
                    .map(|f| ReceiptFile {
                        path: format!(
                            "workspaces/local-perf-{:04}/outputs/report-{f}.md",
                            i % scale.workspaces.max(1)
                        ),
                        change_type: "modified".into(),
                        attribution: "confirmed".into(),
                        tool: Some("Edit".into()),
                        additions: Some(12),
                        deletions: Some(3),
                        not_recoverable: false,
                        also_changed_by: Vec::new(),
                        hollow_write: false,
                    })
                    .collect(),
                ..Default::default()
            }
        })
        .collect();
    crate::evidence::replace_all_for_test(&vault, receipts).expect("receipts");
    vault
}

/// One path's timing: wall-clock time and the CPU time of the calling
/// thread. Under load the wall clock also counts the run-queue wait (two
/// toolchain slots, a busy CPU); the thread's CPU time counts only the
/// path's own work, so the gate judges on it (OF10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Timing {
    pub wall: Duration,
    pub cpu: Duration,
}

impl Timing {
    /// Each clock's best of the two (a re-timing keeps the best of each).
    fn best(self, other: Timing) -> Timing {
        Timing {
            wall: self.wall.min(other.wall),
            cpu: self.cpu.min(other.cpu),
        }
    }
}

/// This thread's CPU time so far (user + system).
#[cfg(unix)]
fn thread_cpu() -> Duration {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid, writable timespec, and the thread CPU-time
    // clock exists on macOS (10.12+) and Linux.
    let rc = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
    assert_eq!(rc, 0, "clock_gettime(CLOCK_THREAD_CPUTIME_ID)");
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

/// One call of `f`, timed on both clocks. Off unix there is no thread CPU
/// clock here, so CPU time is the wall time (every path judged on wall).
fn timed<T>(f: &mut impl FnMut() -> T) -> Timing {
    #[cfg(unix)]
    let cpu = thread_cpu();
    let wall = Instant::now();
    std::hint::black_box(f());
    let wall = wall.elapsed();
    #[cfg(unix)]
    let cpu = thread_cpu().saturating_sub(cpu);
    #[cfg(not(unix))]
    let cpu = wall;
    Timing { wall, cpu }
}

/// Median of `runs` timed calls of `f` on each clock, after one untimed
/// warm-up call.
pub(crate) fn median<T>(runs: usize, mut f: impl FnMut() -> T) -> Timing {
    std::hint::black_box(f());
    let times: Vec<Timing> = (0..runs).map(|_| timed(&mut f)).collect();
    let mid = |mut clock: Vec<Duration>| {
        clock.sort();
        clock[clock.len() / 2]
    };
    Timing {
        wall: mid(times.iter().map(|t| t.wall).collect()),
        cpu: mid(times.iter().map(|t| t.cpu).collect()),
    }
}

fn ms(d: Duration) -> f64 {
    (d.as_secs_f64() * 10_000.0).round() / 10.0
}

pub(crate) fn report(vault: &Path) -> Vec<(&'static str, Timing)> {
    report_only(vault, |_| true)
}

/// `report`, timing only the paths `keep` accepts (the gate re-times just the
/// ones over their ceiling).
fn report_only(vault: &Path, keep: impl Fn(&str) -> bool) -> Vec<(&'static str, Timing)> {
    let first = crate::session_store::list_sessions(vault, 1)
        .unwrap()
        .into_iter()
        .next()
        .expect("a session")
        .key;
    let mut out = Vec::new();
    macro_rules! time {
        ($name:expr, $f:expr) => {
            if keep($name) {
                out.push(($name, median(7, $f)));
            }
        };
    }
    time!("list_workspaces", || crate::workspaces::list(vault)
        .unwrap());
    time!("workspace_icons", || crate::project_icons::list(vault));
    time!("workspace_sessions", || {
        crate::workspaces::sessions_by_workspace(vault).unwrap()
    });
    time!("list_agent_sessions(1000)", || {
        crate::session_store::list_sessions(vault, 1000).unwrap()
    });
    time!("read_agent_conversation", || {
        crate::session_store::read_conversation(vault, &first).unwrap()
    });
    time!("evidence_list(chat)", || {
        crate::evidence::list(vault, Some("chat"), None, 1000).unwrap()
    });
    time!("tasks", || crate::vault_tasks::list(vault));
    // Overnight 10-11/12 P5: what a workspace switch and the palette ask.
    time!("read_workspace", || {
        crate::workspaces::read(vault, "local-perf-0007").unwrap()
    });
    // The chat search (2026-11-04): a word no conversation holds reads every
    // message of the person's (about 2.2 MB here); one every title holds
    // stops at the first page, streamed from `idx_sessions_recent`. Since
    // 2026-11-08 a search reindexes at most a bounded batch first, so the
    // fixture's backfill is drained before timing: this times the search.
    if keep("search_sessions(plan)") || keep("search_sessions(conversation)") {
        crate::session_store::drain_search_index(vault, std::time::Duration::ZERO).unwrap();
    }
    time!("search_sessions(plan)", || {
        crate::workspaces::search_sessions(
            vault,
            "plan",
            None,
            None,
            crate::session_store::MessageText::AsWritten,
        )
        .unwrap()
    });
    time!("search_sessions(conversation)", || {
        crate::workspaces::search_sessions(
            vault,
            "conversation",
            None,
            None,
            crate::session_store::MessageText::AsWritten,
        )
        .unwrap()
    });
    time!("vault_search(operating plan)", || {
        crate::vault::search(vault, "operating plan", 50, None)
    });
    time!("vault_search(q3)", || crate::vault::search(
        vault, "q3", 50, None
    ));
    out
}

/// What each hot path sends over IPC, as serialized JSON bytes (profiling night
/// 11-20: payload size beside time). The same calls as `report_only`, one run
/// each; `docs/reference/architecture.md` "Performance profile" holds the table.
pub(crate) fn payload_report(vault: &Path) -> Vec<(&'static str, usize)> {
    fn size<T: serde::Serialize>(v: &T) -> usize {
        serde_json::to_vec(v).map(|b| b.len()).unwrap_or(0)
    }
    let first = crate::session_store::list_sessions(vault, 1)
        .unwrap()
        .into_iter()
        .next()
        .expect("a session")
        .key;
    vec![
        (
            "list_workspaces",
            size(&crate::workspaces::list(vault).unwrap()),
        ),
        ("workspace_icons", size(&crate::project_icons::list(vault))),
        (
            "workspace_sessions",
            size(&crate::workspaces::sessions_by_workspace(vault).unwrap()),
        ),
        (
            "list_agent_sessions(1000)",
            size(&crate::session_store::list_sessions(vault, 1000).unwrap()),
        ),
        (
            "read_agent_conversation",
            size(&crate::session_store::read_conversation(vault, &first).unwrap()),
        ),
        (
            "evidence_list(chat)",
            size(&crate::evidence::list(vault, Some("chat"), None, 1000).unwrap()),
        ),
        ("tasks", size(&crate::vault_tasks::list(vault))),
        (
            "read_workspace",
            size(&crate::workspaces::read(vault, "local-perf-0007").unwrap()),
        ),
        (
            "vault_search(operating plan)",
            size(&crate::vault::search(vault, "operating plan", 50, None)),
        ),
    ]
}

#[test]
#[ignore = "prints timings at the owner's scale; ~50 MB fixture"]
fn perf_hot_paths_report() {
    let vault = fixture("report", &OWNER);
    for (name, t) in report(&vault) {
        eprintln!(
            "perf_hot_paths {name:<28} {:>8.1} ms wall {:>8.1} ms cpu",
            ms(t.wall),
            ms(t.cpu)
        );
    }
    for (name, bytes) in payload_report(&vault) {
        eprintln!("perf_hot_paths_payload {name:<28} {bytes:>10} bytes");
    }
    let _ = std::fs::remove_dir_all(&vault);
}

/// Ceilings for the gate (test profile, unoptimized, beside the rest of the
/// suite, possibly on a 3-core CI runner). Measured on 2026-10-10 at GATE
/// scale on the owner's 18-core Mac, after B4: list 7 ms, workspace_sessions
/// 15 ms, conversation 1.8 ms, receipts 1.2 ms, tasks 11.5 ms. The ceilings
/// are generous so noise never fails a gate; they catch a path growing by an
/// order of magnitude. Parsing the receipts on every call again is caught
/// exactly by `receipts_parse_once_until_the_file_changes`, not here. Those
/// were wall times on an idle machine, where CPU time is about the same; the
/// gate now compares the ceilings with CPU time ([`over_ceiling`], OF10).
/// The chat search on 2026-11-04 (the report, Mac mini): `plan`, which
/// nothing holds, 9.2 ms over the 2.2 MB of messages; `conversation`, a first
/// page of title hits, 1.8 ms. Its index is held exactly by
/// `session_store`'s `the_search_streams_from_the_recent_index`. Full-text
/// search with a context per hit (#1243) took `conversation` to 124 ms CPU
/// in the report and 27 ms at gate scale (idle Mac mini; about 38 ms beside
/// the suite): each hit redacted its whole request and outcome (67 ms in the
/// report), copied every turn seven times to blank placeholders, refreshed
/// an index with nothing marked on three connections of its own, and every
/// brief was read for the work order. On 2026-11-08, with only a head
/// redacted (`claude_runner::redact_prefix`), 17.5 ms and 9.2 ms (9.7 ms in
/// the gate test beside the report); `plan` 3.0 ms.
const CEILINGS: &[(&str, u64)] = &[
    ("list_workspaces", 80),
    ("workspace_icons", 80),
    ("workspace_sessions", 150),
    ("list_agent_sessions(1000)", 40),
    ("read_agent_conversation", 40),
    ("evidence_list(chat)", 40),
    ("tasks", 120),
    ("read_workspace", 80),
    ("search_sessions(plan)", 40),
    ("search_sessions(conversation)", 40),
    ("vault_search(operating plan)", 200),
    ("vault_search(q3)", 200),
];

/// How many times the gate times a path before calling it over its ceiling.
const TRIES: usize = 3;

/// Paths judged on wall-clock time instead of the calling thread's CPU time:
/// a path that hands work to other threads (a thread pool, `spawn_blocking`,
/// a scoped fan-out) spends CPU the calling thread's clock never sees, so
/// only the wall clock (plus the retries) can catch it growing. None of the
/// timed paths does today (checked 2026-10-16: the vault search, the session
/// queries and the rest all run on the calling thread; `vault::build_index`'s
/// fan-out is not on any of them). List a path here if it starts to.
/// Thread CPU time also misses blocking (a lock wait, a sleep, SQLite busy
/// retries, uncached disk reads): a regression of that kind is caught by
/// the report's wall column, not by this gate.
const WALL_JUDGED: &[&str] = &[];

/// Which clock the gate judges a path on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Judge {
    /// The calling thread's CPU time: the path does its work on that thread.
    Cpu,
    /// Wall time: the path hands work to other threads ([`WALL_JUDGED`]).
    Wall,
}

impl Judge {
    fn label(self) -> &'static str {
        match self {
            Judge::Cpu => "cpu",
            Judge::Wall => "wall",
        }
    }

    /// The clock this judge reads from a timing.
    fn clock(self, t: Timing) -> Duration {
        match self {
            Judge::Cpu => t.cpu,
            Judge::Wall => t.wall,
        }
    }
}

/// Whether a timing is over its ceiling on the clock its path is judged on.
/// Pure. Wall time over the ceiling with CPU time under it is the machine's
/// load (run-queue wait), not the path, so a CPU-judged path isn't over.
fn over_ceiling(t: Timing, ceiling_ms: u64, judge: Judge) -> bool {
    judge.clock(t).as_millis() as u64 > ceiling_ms
}

/// The paths still over their ceiling after up to `tries` timings, each path
/// keeping its best median on each clock. Only the paths over their ceiling
/// are timed again (`remeasure` gets their names), so a gate on a loaded
/// machine (two toolchains, a busy CPU) doesn't fail on one slow sample,
/// while a path that really grew stays over on every try. The ceilings
/// themselves don't move. A path judged on CPU time whose wall time is over
/// is only noted (eprintln), never failed.
fn still_over_after_retries(
    first: Vec<(&'static str, Timing)>,
    ceilings: &[(&str, u64)],
    wall_judged: &[&str],
    tries: usize,
    mut remeasure: impl FnMut(&[&'static str]) -> Vec<(&'static str, Timing)>,
) -> Vec<String> {
    let limit = |name: &str| {
        ceilings
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, l)| *l)
            .expect("a ceiling per path")
    };
    let judge = |name: &str| {
        if wall_judged.contains(&name) {
            Judge::Wall
        } else {
            Judge::Cpu
        }
    };
    let mut best = first;
    let over = |best: &[(&'static str, Timing)]| -> Vec<&'static str> {
        best.iter()
            .filter(|(name, t)| over_ceiling(*t, limit(name), judge(name)))
            .map(|(name, _)| *name)
            .collect()
    };
    for _ in 1..tries {
        let names = over(&best);
        if names.is_empty() {
            break;
        }
        for (name, t) in remeasure(&names) {
            if let Some(slot) = best.iter_mut().find(|(n, _)| *n == name) {
                slot.1 = slot.1.best(t);
            }
        }
    }
    for (name, t) in &best {
        if !over_ceiling(*t, limit(name), judge(name)) && over_ceiling(*t, limit(name), Judge::Wall)
        {
            eprintln!(
                "perf_hot_paths note: {name} wall {:.1} ms > {} ms but cpu {:.1} ms (load)",
                ms(t.wall),
                limit(name),
                ms(t.cpu)
            );
        }
    }
    over(&best)
        .into_iter()
        .map(|name| {
            let t = best.iter().find(|(n, _)| *n == name).expect("timed").1;
            let on = judge(name);
            format!(
                "{name} {} {:.1} ms > {} ms",
                on.label(),
                ms(on.clock(t)),
                limit(name)
            )
        })
        .collect()
}

#[test]
fn hot_paths_stay_under_their_ceilings() {
    let vault = fixture("gate", &GATE);
    let first = report(&vault);
    // Seen with `--nocapture` or when the gate fails: each path's headroom.
    for (name, t) in &first {
        eprintln!(
            "perf_hot_paths gate {name:<28} {:>8.1} ms wall {:>8.1} ms cpu",
            ms(t.wall),
            ms(t.cpu)
        );
    }
    let over = still_over_after_retries(first, CEILINGS, WALL_JUDGED, TRIES, |names| {
        report_only(&vault, |name| names.contains(&name))
    });
    let _ = std::fs::remove_dir_all(&vault);
    assert!(
        over.is_empty(),
        "IPC hot paths over their ceilings after {TRIES} tries: {over:?}"
    );
}

/// A timing of `wall` and `cpu` milliseconds.
fn t(wall: u64, cpu: u64) -> Timing {
    Timing {
        wall: Duration::from_millis(wall),
        cpu: Duration::from_millis(cpu),
    }
}

#[test]
fn only_a_path_over_its_ceiling_on_every_try_fails_the_gate() {
    let ceilings = [
        ("fast", 10),
        ("noisy", 10),
        ("grown", 10),
        ("loaded", 10),
        ("handed", 10),
    ];
    let first = vec![
        ("fast", t(2, 2)),
        ("noisy", t(40, 40)),
        ("grown", t(90, 90)),
        // Wall over, CPU under: the machine's load. Never re-timed or failed.
        ("loaded", t(90, 2)),
        // Hands its work to other threads, so judged (and re-timed) on wall.
        ("handed", t(30, 1)),
    ];
    let mut asked: Vec<Vec<&str>> = Vec::new();
    let mut tries_left = vec![
        // second try: noisy recovers, grown and handed are still slow
        vec![
            ("noisy", t(4, 4)),
            ("grown", t(80, 80)),
            ("handed", t(25, 1)),
        ],
        // third try: only the paths still over are timed again; handed
        // recovers, grown is still over
        vec![("grown", t(85, 85)), ("handed", t(6, 1))],
    ]
    .into_iter();
    let over = still_over_after_retries(first, &ceilings, &["handed"], 3, |names| {
        asked.push(names.to_vec());
        tries_left.next().expect("at most two re-timings")
    });
    assert_eq!(over, vec!["grown cpu 80.0 ms > 10 ms".to_string()]);
    assert_eq!(
        asked,
        vec![vec!["noisy", "grown", "handed"], vec!["grown", "handed"]]
    );
    // With one try (the old single-shot rule) the noisy path fails on its
    // single slow sample.
    let single = still_over_after_retries(
        vec![("fast", t(2, 2)), ("noisy", t(40, 40))],
        &ceilings,
        &[],
        1,
        |_| unreachable!("one try never re-times"),
    );
    assert_eq!(single, vec!["noisy cpu 40.0 ms > 10 ms".to_string()]);
}

/// OF10: under two-slot load `read_agent_conversation` took 42.9 ms of
/// wall time against its 40 ms ceiling (1.8 ms idle). The gate judges a
/// path on its own thread's CPU time; only a path that hands work to other
/// threads is still judged on the wall clock.
#[test]
fn a_path_is_over_its_ceiling_on_its_own_cpu_time_unless_it_hands_work_off() {
    assert!(
        !over_ceiling(t(90, 5), 40, Judge::Cpu),
        "wall over with CPU under is the machine's load"
    );
    assert!(
        over_ceiling(t(90, 85), 40, Judge::Cpu),
        "CPU over is the path itself"
    );
    assert!(
        over_ceiling(t(90, 5), 40, Judge::Wall),
        "a path that hands work off is judged on the wall clock"
    );
}

fn small_vault(name: &str) -> PathBuf {
    let vault = tempfile::Builder::new()
        .prefix(&format!("hollow-perf-{name}-"))
        .tempdir()
        .expect("temp vault")
        .keep();
    std::fs::canonicalize(&vault).expect("canonical vault")
}

fn chat_receipt(id: &str, run_id: &str, finished_at: &str) -> EvidenceReceipt {
    EvidenceReceipt {
        id: id.into(),
        run_id: run_id.into(),
        source: "chat".into(),
        started_at: finished_at.into(),
        finished_at: finished_at.into(),
        status: "succeeded".into(),
        verification_state: "unconfirmed".into(),
        ..Default::default()
    }
}

#[test]
fn receipts_parse_once_until_the_file_changes() {
    let vault = small_vault("receipts");
    crate::evidence::replace_all_for_test(
        &vault,
        vec![chat_receipt("receipt-a", "run-a", "2026-10-10T10:00:00Z")],
    )
    .unwrap();
    crate::evidence::list(&vault, None, None, 10).unwrap();
    let first = crate::evidence::cached_parse_number(&vault).expect("parsed");
    crate::evidence::list(&vault, None, None, 10).unwrap();
    crate::evidence::find_by_history_run(&vault, "none").unwrap();
    assert_eq!(
        crate::evidence::cached_parse_number(&vault),
        Some(first),
        "an unchanged file is parsed once"
    );

    // Our own write.
    crate::evidence::persist_for_test(
        &vault,
        chat_receipt("receipt-b", "run-b", "2026-10-10T11:00:00Z"),
    )
    .unwrap();
    let ids: Vec<String> = crate::evidence::list(&vault, Some("chat"), None, 10)
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(ids, ["receipt-b", "receipt-a"]);
    assert_ne!(crate::evidence::cached_parse_number(&vault), Some(first));

    // Someone else's write (another process, an agent editing the vault).
    let path = vault.join(crate::evidence::FILE_NAME);
    let raw = std::fs::read_to_string(&path)
        .unwrap()
        .replace("receipt-a", "receipt-z");
    std::fs::write(&path, raw).unwrap();
    let ids: Vec<String> = crate::evidence::list(&vault, Some("chat"), None, 10)
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(ids, ["receipt-b", "receipt-z"]);

    // Gone: an empty store, not the cached one.
    std::fs::remove_file(&path).unwrap();
    assert!(crate::evidence::list(&vault, None, None, 10)
        .unwrap()
        .is_empty());
    let _ = std::fs::remove_dir_all(&vault);
}

#[test]
fn a_conversation_turn_gets_the_newest_receipt_of_its_run() {
    let vault = small_vault("conversation");
    let cwd = vault.to_string_lossy().to_string();
    crate::session_store::seed_for_perf(&vault, &[cwd], 1, 2, 100).unwrap();
    crate::evidence::replace_all_for_test(
        &vault,
        vec![
            chat_receipt("receipt-old", "run-0-1", "2026-10-10T10:00:00Z"),
            chat_receipt("receipt-new", "run-0-1", "2026-10-10T12:00:00Z"),
            chat_receipt("receipt-other", "run-9-9", "2026-10-10T13:00:00Z"),
        ],
    )
    .unwrap();
    let key = crate::session_store::list_sessions(&vault, 1)
        .unwrap()
        .remove(0)
        .key;
    let conversation = crate::session_store::read_conversation(&vault, &key).unwrap();
    let ids: Vec<&str> = conversation
        .turns
        .iter()
        .filter_map(|t| t.receipt.as_ref())
        .map(|r| r.id.as_str())
        .collect();
    assert_eq!(ids, ["receipt-new"]);
    let _ = std::fs::remove_dir_all(&vault);
}

#[test]
fn a_recreated_session_store_gets_its_schema_again() {
    let vault = small_vault("schema");
    let cwd = vault.to_string_lossy().to_string();
    crate::session_store::seed_for_perf(&vault, std::slice::from_ref(&cwd), 2, 1, 10).unwrap();
    assert_eq!(
        crate::session_store::list_sessions(&vault, 10)
            .unwrap()
            .len(),
        2
    );
    let db = crate::session_store::db_path_for(&vault).unwrap();
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", db.display()));
    }
    // A new file (new inode): the schema runs again instead of "no such table".
    assert!(crate::session_store::list_sessions(&vault, 10)
        .unwrap()
        .is_empty());
    crate::session_store::seed_for_perf(&vault, &[cwd], 1, 1, 10).unwrap();
    assert_eq!(
        crate::session_store::list_sessions(&vault, 10)
            .unwrap()
            .len(),
        1
    );
    let _ = std::fs::remove_dir_all(&vault);
}

#[test]
fn an_unreadable_receipts_store_is_an_error_not_an_empty_list() {
    use std::os::unix::fs::PermissionsExt;
    let vault = small_vault("unreadable");
    crate::evidence::replace_all_for_test(
        &vault,
        vec![chat_receipt("receipt-a", "run-a", "2026-10-10T10:00:00Z")],
    )
    .unwrap();
    std::fs::set_permissions(&vault, std::fs::Permissions::from_mode(0o600)).unwrap();
    let result = crate::evidence::list(&vault, None, None, 10);
    std::fs::set_permissions(&vault, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(result.is_err(), "{result:?}");
    let _ = std::fs::remove_dir_all(&vault);
}
