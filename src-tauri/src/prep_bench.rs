//! The preparation bench (agent effectiveness F8, wave 3 · lane O: O1
//! measured first, O2 measures its change). An ignored test, run on
//! purpose:
//!
//! ```text
//! cd src-tauri
//! export TMPDIR=/tmp HOLLOW_PREP_BENCH_OUT=/tmp/hollow-prep-bench
//! cargo test --lib prep_bench -- --ignored --nocapture
//! node ../scripts/agent-eval/prep-bench.mjs /tmp/hollow-prep-bench
//! ```
//!
//! It generates a vault (deterministic notes, approved learnings, a `me/`
//! profile and a workspace with four attached sources) and drives a chat
//! turn's preparation through the production steps in the order
//! `chat::execute_turn` runs them: a real admission (the vault snapshot),
//! the request and recall, the workspace's attached sources, then (O2) the
//! retrieval broker with its Jev rerank and "when relevant" selection over
//! 30 synthetic connectors with its Jev call, at once, under one preparation
//! deadline, through the production `prioritized_context` and
//! `select_connectors`, with the repository snapshot beside them on its own
//! budget (S29); then the host context compiler inside prompt assembly and
//! the Claude command. Each sample is
//! the same `preparation.timeline` a run's manifest carries
//! (`prep_timing::timeline`), with its Jev outcomes and its
//! `preparation.deadline`. `scripts/agent-eval/prep-bench.mjs` reports
//! p50/p95 per phase, for the stage the deadline bounds, and in total.
//!
//! Jev runs against the loopback fake (`jev::fake`, a literal 127.0.0.1
//! port with a placeholder key; nothing else is reached). Modes: `off` (no
//! request), `score` (every candidate scored 3), `timeout` (the fake never
//! answers), `malformed` (an unparseable answer), each sample through a
//! fresh `jev::Guard`; `score-shared` and `timeout-shared`, one guard for
//! every sample (rankings reused; the breaker); `semantic-slow` (Jev off, a
//! 10 s semantic half, which the deadline cuts); `codex` (O3: Jev off, a
//! Codex command whose native inventory a stand-in CLI lists for every
//! sample, as before the cache) and `codex-cached` (the same through one
//! `codex_inventory::Cache` kept across samples). The stand-in
//! (`codex_inventory::fake`) is shaped like the real CLI, a node wrapper
//! starting a native child, when node is on the app's PATH; the real
//! CLI's own start-up and config load are not in it.
//!
//! What it can't drive, and leaves `null`: the library load and the
//! Microsoft 365 refresh (`connectors::attach` reads the app's library,
//! which tests never do), the installed Codex's inventory (the `codex`
//! modes use the stand-in), the spawn and the first outputs (no agent runs; the bed's fake
//! agent measures those), and the real semantic half (no embedder in
//! tests). The vault scan uses the FTS index, engaged as the app engages it
//! (`vault_search_index::engage`; under `cfg(test)` its database lives in a
//! per-process temp folder), so it reads the BM25 candidates (OB12), not
//! every note. Until OB12 it engaged none and scanned linearly. Every
//! generated note shares the request's words, so the linear scan would read
//! them all.
//!
//! Environment: `HOLLOW_PREP_BENCH_OUT` (default `$TMPDIR/hollow-prep-bench`),
//! `HOLLOW_PREP_BENCH_RUNS` (samples per mode, default 20),
//! `HOLLOW_PREP_BENCH_MODES` (default every mode above),
//! `HOLLOW_PREP_BENCH_NOTES` (generated notes, default 600, half the ~1,200
//! eligible notes of the vault OB12 measured; pass 1500 to exceed it).
use crate::agent::{self, AgentKind};
use crate::context_rank::ConnectorCandidate;
use crate::jev::fake::{serve, Reply};
use crate::prep_deadline::BUDGET;
use crate::prep_timing::{self, Mark, Step};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// What the bench asks: words the generated notes, sources and learnings
/// share, so every step has candidates.
const REQUEST: &str =
    "Summarise the kestrel sensor calibration notes and the budget ceiling for the pilot sites";
/// The placeholder bearer the loopback fake ignores.
const KEY: &str = "hollow-prep-bench-loopback";
const CONNECTORS: usize = 30;

const WORDS: [&str; 24] = [
    "kestrel",
    "sensor",
    "calibration",
    "budget",
    "ceiling",
    "pilot",
    "sites",
    "harbour",
    "rollout",
    "vendor",
    "invoice",
    "schedule",
    "field",
    "report",
    "battery",
    "gateway",
    "firmware",
    "survey",
    "parking",
    "level",
    "wireless",
    "meeting",
    "decision",
    "review",
];

/// A deterministic word stream (a linear congruential generator).
struct Words(u64);

impl Words {
    fn next(&mut self) -> &'static str {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        WORDS[((self.0 >> 33) % WORDS.len() as u64) as usize]
    }

    fn line(&mut self, n: usize) -> String {
        (0..n).map(|_| self.next()).collect::<Vec<_>>().join(" ")
    }
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// The generated vault: `notes` notes of 24 lines, 12 approved learnings,
/// a `me/` profile, and a workspace with four sources. Returns the
/// workspace's folder.
fn generate(vault: &Path, notes: usize) -> PathBuf {
    let mut words = Words(42);
    for i in 0..notes {
        let body: Vec<String> = (0..24).map(|_| words.line(12)).collect();
        write(
            &vault.join(format!("notes/area-{}/note-{i:04}.md", i % 12)),
            &format!("# Note {i}\n\n{}\n", body.join("\n")),
        );
    }
    for i in 0..12 {
        write(
            &vault.join(format!("learnings/bench-{i:02}.md")),
            &format!(
                "---\ntype: learned-context\ntitle: Bench learning {i}\nsubject: {}\ntrust: approved\n---\n\n{}.\n",
                words.next(),
                words.line(14)
            ),
        );
    }
    write(
        &vault.join("me/Identity.md"),
        &format!("# Identity\n\n{}\n", words.line(60)),
    );
    write(
        &vault.join("me/Decisions.md"),
        &format!(
            "# Decisions\n\n- {}\n- {}\n",
            words.line(20),
            words.line(20)
        ),
    );
    let workspace = crate::workspaces::create(vault, "Prep bench", "Measure preparation").unwrap();
    let cwd = PathBuf::from(&workspace.cwd);
    for (name, lines) in [("a", 400), ("b", 300), ("c", 200), ("d", 60)] {
        let body: Vec<String> = (0..lines).map(|_| words.line(10)).collect();
        write(
            &cwd.join(format!("sources/{name}-source.md")),
            &format!("# Source {name}\n\n{}\n", body.join("\n")),
        );
    }
    cwd
}

/// One chat turn's preparation, timed: the timeline, each Jev use's outcome
/// and what the deadline cut. Jev (at `jev`, the loopback fake; `None`:
/// off) goes through a fresh guard, so no sample reuses another's ranking or
/// sees another's failures.
async fn prepare_once(vault: &Path, cwd: &Path, jev: Option<&str>, sample: usize) -> Value {
    let guard = crate::jev::Guard::new(crate::jev::Tuning::APP);
    prepare_with(vault, cwd, jev, sample, &guard).await
}

/// [`prepare_once`] through `guard` (the `-shared` modes keep one across
/// samples, to measure reuse and the breaker).
async fn prepare_with(
    vault: &Path,
    cwd: &Path,
    jev: Option<&str>,
    sample: usize,
    guard: &crate::jev::Guard,
) -> Value {
    prepare_within(vault, cwd, jev, sample, guard, BUDGET).await
}

/// [`prepare_with`] under a preparation deadline of `budget` instead of
/// the app's [`BUDGET`] (the tests below wait out a shorter one; the bench
/// always measures the app's).
async fn prepare_within(
    vault: &Path,
    cwd: &Path,
    jev: Option<&str>,
    sample: usize,
    guard: &crate::jev::Guard,
    budget: Duration,
) -> Value {
    prepare_as(vault, cwd, jev, sample, guard, None, budget).await
}

/// [`prepare_within`], building a Codex command whose inventory comes from
/// the fake CLI through `codex`'s cache (F8, O3) when given; a Claude
/// command otherwise.
async fn prepare_as(
    vault: &Path,
    cwd: &Path,
    jev: Option<&str>,
    sample: usize,
    guard: &crate::jev::Guard,
    codex: Option<(&crate::codex_inventory::Cli, &crate::codex_inventory::Cache)>,
    budget: Duration,
) -> Value {
    let lease = crate::automated_history::try_begin_owned(vault, "chat", None).expect("admitted");
    let mut manifest = crate::run_manifest::RunManifest::new();
    let started = Instant::now();
    let request = crate::turn_request::TurnRequest::from_text(vault, REQUEST);
    let _recall = request.recall(vault);
    manifest.timer.0.record(Step::Recall, Mark::since(started));
    let started = Instant::now();
    let sources = crate::workspaces::context_for(
        vault,
        cwd,
        &crate::workspaces::SourceRequest {
            query: &request.source_query(),
            reader: crate::workspaces::SourceReader::ReadSource,
            child_packet: None,
        },
    )
    .expect("workspace context");
    manifest.attached_sources = sources.as_ref().and_then(|s| s.sources.clone());
    if sources.is_some() {
        manifest
            .timer
            .0
            .record(Step::AttachedSources, Mark::since(started));
    }
    let owned: Vec<(String, String, String)> = (1..=CONNECTORS)
        .map(|i| {
            (
                format!("bench-{i:02}"),
                format!("Bench service {i}"),
                format!("Synthetic {} connector", WORDS[i % WORDS.len()]),
            )
        })
        .collect();
    let candidates: Vec<ConnectorCandidate<'_>> = owned
        .iter()
        .map(|(id, name, description)| ConnectorCandidate {
            id,
            name,
            description,
        })
        .collect();
    let query = request.connector_query();
    // As `chat::execute_turn` prepares (F8, O2): retrieval and connector
    // selection at once, under one deadline, Jev through `guard`.
    let target = crate::jev::Target {
        guard,
        generation: guard.generation(),
        allowed: None,
        key: jev.map(|_| KEY.to_string()),
        endpoint: jev.unwrap_or_default().to_string(),
    };
    let deadline = crate::prep_deadline::PrepDeadline::with_budget(budget);
    let retrieval_and_connectors = crate::prep_deadline::concurrently(
        crate::directory_connections::prioritized_context(
            vault.to_path_buf(),
            cwd.to_path_buf(),
            REQUEST.to_string(),
            &deadline,
            &target,
        ),
        crate::context_rank::select_connectors(&query, &candidates, &deadline, &target),
    );
    // The repository snapshot beside them (S29), on its own budget.
    let repo_snapshot = async {
        let started = Instant::now();
        let baseline = crate::repo_diff::begin(vault, cwd, None).await;
        (baseline, Mark::since(started))
    };
    let (((prepared, retrieval), (selection, connectors)), (_baseline, snapshot)) =
        tokio::join!(retrieval_and_connectors, repo_snapshot);
    manifest.timer.0.record(Step::RepoSnapshot, snapshot);
    let prepared = prepared.expect("retrieval");
    manifest.timer.0.record(Step::Retrieval, retrieval);
    manifest.set_retrieval(prepared.retrieval, prepared.jev);
    manifest.timer.0.record(Step::Connectors, connectors);
    manifest.set_connectors(selection);
    manifest.set_deadline(&deadline, false);
    let assembly = Instant::now();
    let started = Instant::now();
    let compiled = crate::context_compiler::compile_turn(
        vault,
        // A new conversation each sample, as a first turn compiles.
        &format!("s-prep-bench-{sample}"),
        &[],
        0,
        crate::context_compiler::Entry::for_depth(0),
        crate::context_compiler::Session::New,
        true,
        crate::task_state::Reader::None,
        REQUEST,
    );
    manifest
        .timer
        .0
        .record(Step::ContextCompile, Mark::since(started));
    let body = format!(
        "{}{}{}",
        sources.map(|s| s.text).unwrap_or_default(),
        prepared.text,
        compiled.body
    );
    manifest
        .timer
        .0
        .record(Step::PromptAssembly, Mark::since(assembly));
    let body_bytes = body.len();
    let spec = agent::RunSpec {
        prompt: body,
        resume: None,
        model: None,
        perm: "acceptEdits".into(),
        cwd: cwd.to_path_buf(),
        allowed_tools: Vec::new(),
        disallowed_tools: Vec::new(),
        interactive: true,
        mcp_config: None,
        append_system_prompt: None,
        process_scope: agent::ProcessScope::GuiOwned,
        usage_stream: false,
        max_budget_usd: None,
        extras: Default::default(),
    };
    let (_cmd, built, inventory) = match codex {
        None => prep_timing::timed_command(AgentKind::Claude, &spec),
        Some((cli, cache)) => {
            let started = Instant::now();
            let (cmd, inventory) =
                agent::build_command_via(AgentKind::Codex, &spec, |spec| cache.servers(cli, spec));
            (cmd, Mark::since(started), inventory)
        }
    };
    manifest.timer.0.record(Step::Command, built);
    manifest.set_codex_inventory(inventory);
    let clock = prep_timing::RunClock {
        admitted: Some(lease.admitted_at()),
        baseline_ms: lease.baseline_ms(),
        ..Default::default()
    };
    let timeline = prep_timing::timeline(&manifest.timer.0, &clock).expect("admitted");
    let _ = lease.finish("cancelled");
    // Sizes (no text): what retrieval scanned and kept, what the
    // attached sources used, and the assembled body.
    let stats = manifest
        .retrieval
        .as_ref()
        .and_then(|r| r.stats.clone())
        .unwrap_or_default();
    let sizes = json!({
        "notesEligible": stats.notes_eligible,
        "candidateSource": stats.candidate_source,
        "notesRead": stats.notes_read,
        "notesMatched": stats.notes_matched,
        "bytesRead": stats.bytes_read,
        "pool": stats.pool,
        "kept": manifest.retrieval.as_ref().map_or(0, |r| r.candidates.iter().filter(|c| c.kept).count()),
        "attachedUsed": manifest.attached_sources.as_ref().map_or(0, |a| a.used),
        "bodyBytes": body_bytes,
    });
    let uses: Vec<Value> = manifest
        .jev
        .iter()
        .map(|u| json!({"use": u.kind, "outcome": u.outcome, "items": u.items}))
        .collect();
    let deadline = json!(manifest.preparation.deadline);
    json!({ "timeline": timeline, "jev": uses, "sizes": sizes, "deadline": deadline, "semantic": stats.semantic, "codexInventory": manifest.preparation.codex_inventory })
}

/// What one bench mode does: the fake's reply (`None`: Jev off), whether
/// one guard (and, for Codex, one inventory cache) serves every sample, a
/// slow semantic half, and whether the command is Codex's (built with the
/// fake CLI's inventory).
struct Mode {
    reply: Option<Reply>,
    shared: bool,
    slow_semantic: bool,
    codex: bool,
}

fn mode(name: &str) -> Mode {
    let codex = name.starts_with("codex");
    let (reply, shared, slow_semantic) = match name {
        // Jev off; a Codex command, its inventory listed by the fake CLI
        // for every sample (a fresh cache each time: before O3).
        "codex" => (None, false, false),
        // The same through one cache kept across samples (O3).
        "codex-cached" => (None, true, false),
        "off" => (None, false, false),
        "score" => (Some(Reply::ScoreAll(3.0)), false, false),
        "timeout" => (Some(Reply::Silent), false, false),
        "malformed" => (Some(Reply::Raw("{\"answers\": ".into())), false, false),
        // One guard for every sample: identical rankings are reused.
        "score-shared" => (Some(Reply::ScoreAll(3.0)), true, false),
        // One guard for every sample: the breaker opens after 3 failures.
        "timeout-shared" => (Some(Reply::Silent), true, false),
        // Jev off; a semantic half that takes 10 s (the deadline cuts it).
        "semantic-slow" => (None, false, true),
        other => panic!("unknown mode {other}"),
    };
    Mode {
        reply,
        shared,
        slow_semantic,
        codex,
    }
}

/// The start of the first and the end of the last of retrieval and
/// connector selection on a sample's timeline: the stage the preparation
/// deadline bounds.
fn stage_ms(timeline: &Value) -> u64 {
    let spans: Vec<(u64, u64)> = ["retrieval", "connectors"]
        .iter()
        .filter_map(|name| {
            let p = timeline["phases"]
                .as_array()?
                .iter()
                .find(|p| p["name"] == *name)?;
            let start = p["startMs"].as_u64()?;
            Some((start, start + p["durationMs"].as_u64()?))
        })
        .collect();
    let first = spans.iter().map(|s| s.0).min().unwrap_or(0);
    spans.iter().map(|s| s.1).max().unwrap_or(0) - first
}

fn env_or<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "the preparation baseline: run on purpose (see the module doc)"]
async fn preparation_baseline() {
    let out = std::env::var_os("HOLLOW_PREP_BENCH_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("hollow-prep-bench"));
    std::fs::create_dir_all(&out).unwrap();
    let runs: usize = env_or("HOLLOW_PREP_BENCH_RUNS", 20);
    let notes: usize = env_or("HOLLOW_PREP_BENCH_NOTES", 600);
    let modes = std::env::var("HOLLOW_PREP_BENCH_MODES").unwrap_or_else(|_| {
        "off,score,timeout,malformed,score-shared,timeout-shared,semantic-slow,codex,codex-cached"
            .into()
    });
    // Not `.tmp…`: the vault walk skips hidden folders, the root included.
    let dir = tempfile::Builder::new()
        .prefix("hollow-prep-bench-")
        .tempdir()
        .unwrap();
    let vault = dir.path().canonicalize().unwrap();
    let cwd = generate(&vault, notes);
    // The app engages the FTS index at startup; so does the bench (OB12).
    crate::vault_search_index::engage(&vault);
    // One warm-up, discarded: the first read of a fresh vault is colder
    // than any turn's.
    prepare_once(&vault, &cwd, None, 0).await;
    // The `codex` modes' stand-in CLI (O3): shaped like the real one (a
    // node wrapper starting a native child) when node is on the app's PATH,
    // else a shell script. Its own CODEX_HOME, outside the vault.
    let codex_tmp = tempfile::Builder::new()
        .prefix("hollow-prep-bench-codex-")
        .tempdir()
        .unwrap();
    let codex_dir = codex_tmp.path().canonicalize().unwrap();
    let codex_home = codex_dir.join("home");
    std::fs::create_dir_all(&codex_home).unwrap();
    std::fs::write(
        codex_home.join("config.toml"),
        "[mcp_servers.mail]\ncommand = \"mail\"\n",
    )
    .unwrap();
    crate::codex_inventory::fake::set_servers(&codex_home, &["mail", "custom.slack"]);
    let (codex_bin, codex_shape) = match crate::codex_inventory::fake::node_wrapper(&codex_dir) {
        Some(bin) => (bin, "node-wrapper"),
        None => (crate::codex_inventory::fake::shell(&codex_dir), "shell"),
    };
    let fake_codex = crate::codex_inventory::Cli {
        bin: codex_bin,
        home: Some(codex_home),
    };
    let mut sample = 0;
    for name in modes.split(',').map(str::trim).filter(|m| !m.is_empty()) {
        let Mode {
            reply,
            shared,
            slow_semantic,
            codex,
        } = mode(name);
        let server = reply.map(serve);
        let url = server.as_ref().map(|s| s.url.clone());
        assert!(url
            .as_deref()
            .is_none_or(|u| crate::instance::loopback_jev_url(u).is_some()));
        if slow_semantic {
            crate::retrieval_broker::standin::slow_semantic(
                &vault,
                std::time::Duration::from_secs(10),
            );
        }
        let guard = crate::jev::Guard::new(crate::jev::Tuning::APP);
        let shared_cache = crate::codex_inventory::Cache::new();
        let mut samples = Vec::with_capacity(runs);
        for _ in 0..runs {
            sample += 1;
            samples.push(if codex {
                let fresh = crate::codex_inventory::Cache::new();
                let cache = if shared { &shared_cache } else { &fresh };
                prepare_as(
                    &vault,
                    &cwd,
                    None,
                    sample,
                    &guard,
                    Some((&fake_codex, cache)),
                    BUDGET,
                )
                .await
            } else if shared {
                prepare_with(&vault, &cwd, url.as_deref(), sample, &guard).await
            } else {
                prepare_once(&vault, &cwd, url.as_deref(), sample).await
            });
        }
        if slow_semantic {
            crate::retrieval_broker::standin::clear(&vault);
        }
        let mode = name;
        let report = json!({
            "kind": "hollow-prep-bench",
            "schema": 1,
            "mode": mode,
            "runs": runs,
            "vault": { "notes": notes, "learnings": 12, "sources": 4, "connectors": CONNECTORS },
            "prefilter": "linear",
            "codexFake": if codex { Some(codex_shape) } else { None },
            "samples": samples,
        });
        let file = out.join(format!("{mode}.json"));
        std::fs::write(&file, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        println!("prep-bench: {mode}: {runs} samples -> {}", file.display());
    }
}

/// A small generated vault for the tests below (the bench's own is 600
/// notes).
fn small_vault() -> (tempfile::TempDir, PathBuf, PathBuf) {
    // Not `.tmp…`: the vault walk skips hidden folders, the root included.
    let dir = tempfile::Builder::new()
        .prefix("hollow-prep-deadline-")
        .tempdir()
        .unwrap();
    let vault = dir.path().canonicalize().unwrap();
    let cwd = generate(&vault, 60);
    (dir, vault, cwd)
}

fn outcomes(sample: &Value) -> Vec<(String, String, u64)> {
    sample["jev"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| {
            (
                u["use"].as_str().unwrap().to_string(),
                u["outcome"].as_str().unwrap().to_string(),
                u["items"].as_u64().unwrap(),
            )
        })
        .collect()
}

fn phase(timeline: &Value, name: &str) -> (Option<u64>, Option<u64>) {
    let p = timeline["phases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == name)
        .unwrap();
    (p["startMs"].as_u64(), p["durationMs"].as_u64())
}

/// The preparation deadline the tests below wait out while the fake Jev
/// never answers (overnight S04: waiting out the app's 3 s five times made
/// these two of the slowest library tests). Below Jev's own 4 s
/// per-request bound, as the app's is, so the deadline cuts the stall.
const STALL_BUDGET: Duration = Duration::from_millis(2000);
// The app's deadline and this one both cut a stall before Jev's own
// per-request bound would (a `deadline`, never a `timeout`).
const _: () = assert!(
    BUDGET.as_millis() < crate::jev::TIMEOUT.as_millis()
        && STALL_BUDGET.as_millis() < crate::jev::TIMEOUT.as_millis()
);

/// Agent effectiveness F8 (O2; red first at O1, `ae3-o2-red.log`: 8,014 ms):
/// with the fake Jev never answering, retrieval and connector selection run
/// at once and together stay within the preparation deadline, not 4 s ×
/// calls. Both Jev calls fall back as `deadline`, the manifest lists them
/// as cut, and the evidence kept is what Jev off keeps (required local
/// context is never cut).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_timeout_on_every_jev_call_stays_within_the_preparation_deadline() {
    let (_dir, vault, cwd) = small_vault();
    let off = prepare_once(&vault, &cwd, None, 0).await;
    let server = serve(Reply::Silent);
    let budget = u64::try_from(STALL_BUDGET.as_millis()).unwrap();
    let mut stages = Vec::new();
    for sample in 1..=3 {
        // A fresh guard each sample, as `prepare_once`.
        let guard = crate::jev::Guard::new(crate::jev::Tuning::APP);
        let s = prepare_within(
            &vault,
            &cwd,
            Some(&server.url),
            sample,
            &guard,
            STALL_BUDGET,
        )
        .await;
        stages.push(stage_ms(&s["timeline"]));
        assert_eq!(
            outcomes(&s),
            [
                ("passages".into(), "deadline".into(), 24),
                ("connectors".into(), "deadline".into(), 24)
            ],
            "both calls were sent and cut"
        );
        assert_eq!(s["deadline"]["budgetMs"], budget);
        assert_eq!(
            s["deadline"]["cut"],
            json!(["retrievalJev", "connectorsJev"])
        );
        assert_eq!(s["sizes"]["kept"], off["sizes"]["kept"], "evidence kept");
        // At once: connector selection starts before retrieval ends.
        let (rs, rd) = phase(&s["timeline"], "retrieval");
        let (cs, _) = phase(&s["timeline"], "connectors");
        assert!(
            cs.unwrap() < rs.unwrap() + rd.unwrap(),
            "{:?}",
            s["timeline"]
        );
        let (js, jd) = phase(&s["timeline"], "connectorsJev");
        assert!(js.is_some() && jd.unwrap() <= budget + 500);
    }
    let p95 = *stages.iter().max().unwrap();
    assert!(
        p95 <= budget + 700,
        "retrieval and connectors took {stages:?} ms with Jev timing out: not bounded by the {budget} ms deadline"
    );
    // Each sample sent both requests (fresh guards: no reuse, no breaker).
    assert_eq!(server.seen.lock().unwrap().len(), 6);
}

/// Through one guard (as the app's): an identical ranking is reused with no
/// request, and after three failed calls the breaker skips Jev, so the
/// third timed-out turn costs nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_guard_reuses_identical_rankings_and_pauses_a_failing_jev() {
    let (_dir, vault, cwd) = small_vault();
    let guard = crate::jev::Guard::new(crate::jev::Tuning::APP);
    let healthy = serve(Reply::ScoreAll(3.0));
    let first = prepare_with(&vault, &cwd, Some(&healthy.url), 1, &guard).await;
    let second = prepare_with(&vault, &cwd, Some(&healthy.url), 2, &guard).await;
    assert_eq!(
        outcomes(&first),
        [
            ("passages".into(), "ok".into(), 24),
            ("connectors".into(), "ok".into(), 24)
        ]
    );
    assert_eq!(
        outcomes(&second),
        [
            ("passages".into(), "cached".into(), 24),
            ("connectors".into(), "cached".into(), 24)
        ]
    );
    assert_eq!(
        healthy.seen.lock().unwrap().len(),
        2,
        "the second sent none"
    );
    assert_eq!(phase(&second["timeline"], "retrievalJev"), (None, None));
    assert_eq!(second["sizes"]["kept"], first["sizes"]["kept"]);

    let stalled = serve(Reply::Silent);
    let guard = crate::jev::Guard::new(crate::jev::Tuning::APP);
    for sample in 1..=2 {
        let s = prepare_within(
            &vault,
            &cwd,
            Some(&stalled.url),
            sample,
            &guard,
            STALL_BUDGET,
        )
        .await;
        assert!(outcomes(&s).iter().all(|(_, o, _)| o == "deadline"), "{s}");
    }
    // The app's deadline for the third, so waiting would show (3,000 ms).
    let started = Instant::now();
    let third = prepare_with(&vault, &cwd, Some(&stalled.url), 3, &guard).await;
    assert_eq!(
        outcomes(&third),
        [
            ("passages".into(), "circuit-open".into(), 0),
            ("connectors".into(), "circuit-open".into(), 0)
        ]
    );
    assert_eq!(
        third["deadline"]["cut"],
        json!([]),
        "nothing was waited for"
    );
    // Under the 3,000 ms deadline with room for a loaded machine.
    assert!(stage_ms(&third["timeline"]) < 2000, "{}", third["timeline"]);
    // The stage time and the empty cut above prove nothing waited; the wall
    // clock (the whole preparation) only catches a hang. A 3 s bound flaked
    // under a loaded gate on 2026-10-10.
    assert!(started.elapsed() < std::time::Duration::from_secs(20));
    assert_eq!(stalled.seen.lock().unwrap().len(), 4, "the third sent none");
}
