//! J4 (overnight 11-08): the recall probe. Times what a turn's recall costs
//! and how its ranking moves, on this Mac's real embedding model over a
//! scratch copy of the vault's `learnings/` and a file of real requests:
//!   HOLLOW_RECALL_PROBE_VAULT=<copy holding learnings/> \
//!   HOLLOW_RECALL_PROBE_QUERIES=<one request per line> \
//!   [HOLLOW_RECALL_PROBE_OUT=<tsv>] \
//!   cargo test --manifest-path src-tauri/Cargo.toml \
//!     --config profile.test.opt-level=2 --config profile.test.debug-assertions=false \
//!     --config profile.test.overflow-checks=false --lib recall_probe -- --ignored --nocapture
//! (a debug build embeds ~8 times slower: use the optimized profile for the
//! numbers.) The model is read from `src-tauri/models/bge-small-en-v1.5` (a
//! link to the installed app's copy works). Per request it runs three
//! recalls: by stems only (no vectors), by meaning with the model resident
//! (warm) and by meaning right after the model was unloaded (cold). It
//! reports their wall times (what the turn waits), how often the vector
//! arrived within the wait, how long a cold load takes to deliver it, and how
//! much the top 5 moves between stems and meaning. This is the idle cost:
//! contention (a backfill batch, a reconcile) is what the 500 real runs'
//! manifests show on top of it (`recall-measurement.md`). Test builds keep the
//! index under a per-process temp folder, so nothing outside the scratch copy
//! is written. `#[ignore]`d: it needs the model and the copy.
//!
//! J3a (2026-11-15) adds [`recall_rank_probe`], the local half of the J3
//! comparison (`j3a-report.md` in the 11-14 build run's folder): ranking
//! quality on the notes pool and recall's latency under contention, from a
//! file of labelled queries (TSV `own|agent`, relevant vault-relative paths
//! comma separated, then the query: [`parse_labels`]), with your turns and
//! agents' reported apart:
//!   HOLLOW_RECALL_RANK_VAULT=<scratch vault copy> \
//!   HOLLOW_RECALL_RANK_LABELS=<labels.tsv> \
//!   [HOLLOW_RECALL_RANK_THREADS=4] [HOLLOW_RECALL_RANK_ADMIT_MS=126] \
//!   [HOLLOW_RECALL_RANK_OUT=<tsv>] \
//!   cargo test ... --lib recall_rank_probe -- --ignored --nocapture
//! (same optimized profile as above). Per labelled query: the first
//! relevant file's rank in the broker's lexical pool (24 passages, distinct
//! files) and in the turn's recall, as recall@5 and MRR per arm and per who;
//! then turn recall timed alone and with N at once (p50, p95, how often the
//! vector arrived); then the prefetch as shipped: started, taken an
//! admission's time later within `RECALL_PREFETCH_CUTOFF`, N at once (how
//! often it was ready, the most a turn waited). The output TSV holds ranks
//! and times, never a query or a path.
#![cfg(test)]

use crate::vault_embed_index as index;
use crate::vault_learning::{recall_with_scope, Meaning, MeaningSource, RecallMeaning};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[(((sorted.len() - 1) as f64) * p).round() as usize]
}

fn line(label: &str, mut ms: Vec<f64>) -> String {
    ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    format!(
        "{label}: n {} p50 {:.1} ms p90 {:.1} ms p99 {:.1} ms max {:.1} ms",
        ms.len(),
        pct(&ms, 0.5),
        pct(&ms, 0.9),
        pct(&ms, 0.99),
        ms.last().copied().unwrap_or(0.0)
    )
}

fn wait_idle(limit: Duration) -> bool {
    let start = Instant::now();
    while index::recall_in_flight_for_probe() {
        if start.elapsed() > limit {
            return false;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    true
}

/// Jaccard overlap of two top-5 lists.
fn overlap5(a: &[String], b: &[String]) -> f64 {
    let a: BTreeSet<&String> = a.iter().take(5).collect();
    let b: BTreeSet<&String> = b.iter().take(5).collect();
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    a.intersection(&b).count() as f64 / a.union(&b).count() as f64
}

#[test]
#[ignore = "measurement: needs the bundled model, HOLLOW_RECALL_PROBE_VAULT and HOLLOW_RECALL_PROBE_QUERIES"]
fn recall_probe() {
    let vault = PathBuf::from(
        std::env::var("HOLLOW_RECALL_PROBE_VAULT").expect("HOLLOW_RECALL_PROBE_VAULT"),
    );
    let queries: Vec<String> = std::fs::read_to_string(
        std::env::var("HOLLOW_RECALL_PROBE_QUERIES").expect("HOLLOW_RECALL_PROBE_QUERIES"),
    )
    .expect("queries file")
    .lines()
    .map(str::trim)
    .filter(|l| !l.is_empty())
    .map(str::to_string)
    .collect();
    let dir = crate::embed_model::default_model_dir();
    assert!(
        dir.join("model.safetensors").is_file(),
        "no model at {}: link src-tauri/models/bge-small-en-v1.5 to the installed app's copy",
        dir.display()
    );
    index::set_semantic_enabled(true);
    index::engage(&vault);
    let learnings = crate::vault_learning::list_context(&vault, true).len();

    // A cold load, then one query: what the first embedding after eviction costs.
    crate::embed_model::request_unload();
    let t = Instant::now();
    crate::embed_model::embed_query(&dir, "warm up the model").expect("model loads");
    let cold_load_ms = t.elapsed().as_secs_f64() * 1000.0;
    let t = Instant::now();
    let embedded = index::embed_engaged_for_probe(&vault, &dir);
    let index_ms = t.elapsed().as_secs_f64() * 1000.0;
    println!(
        "recall_probe corpus: {learnings} learnings, {embedded} chunks embedded in {index_ms:.0} ms; cold load plus first query {cold_load_ms:.0} ms; {} requests",
        queries.len()
    );

    let source = index::EngagedMeaning { vault: &vault };
    // One recall of `q`: (wall ms, whether the vector arrived, the items).
    let recall = |q: &str, meaning: bool| {
        let outcome = std::cell::Cell::new(RecallMeaning::Off);
        let lookup = |paths: &[String]| match source.cosines(q, paths) {
            Ok(c) => {
                outcome.set(RecallMeaning::Used);
                c
            }
            Err(why) => {
                outcome.set(why);
                BTreeMap::new()
            }
        };
        let dyn_lookup: Meaning<'_> = &lookup;
        let t = Instant::now();
        let result = recall_with_scope(
            &vault,
            q,
            &[],
            None,
            None,
            if meaning { Some(dyn_lookup) } else { None },
        );
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let paths: Vec<String> = result.items.iter().map(|i| i.path.clone()).collect();
        (ms, outcome.get() == RecallMeaning::Used, paths)
    };
    // One cold recall: the model unloaded first. (wall ms, vector arrived
    // in time, ms until the vector was ready).
    let cold = |q: &str| {
        assert!(wait_idle(Duration::from_secs(30)));
        crate::embed_model::request_unload();
        let t = Instant::now();
        let (ms, used, _) = recall(q, true);
        let ready = if wait_idle(Duration::from_secs(30)) {
            t.elapsed().as_secs_f64() * 1000.0
        } else {
            f64::NAN
        };
        (ms, used, ready)
    };

    let (mut stems_ms, mut warm_ms, mut cold_ms, mut ready_ms) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut warm_used, mut cold_used) = (0usize, 0usize);
    let (mut overlap, mut top1_same) = (0f64, 0usize);
    let mut tsv = String::from(
        "query_chars\tstems_ms\twarm_ms\twarm_used\tcold_ms\tcold_used\tcold_ready_ms\toverlap5\ttop1_same\n",
    );
    for q in &queries {
        let (a_ms, _, a_paths) = recall(q, false);
        // Warm: a different text refreshes the model, so this request's own
        // embedding is real, not a query-cache hit.
        let _ = crate::embed_model::embed_query(&dir, "keep the model warm");
        assert!(wait_idle(Duration::from_secs(20)));
        let (b_ms, b_used, b_paths) = recall(q, true);
        let (c_ms, c_used, ready) = cold(q);
        stems_ms.push(a_ms);
        warm_ms.push(b_ms);
        cold_ms.push(c_ms);
        if ready.is_finite() {
            ready_ms.push(ready);
        }
        warm_used += b_used as usize;
        cold_used += c_used as usize;
        let ov = overlap5(&a_paths, &b_paths);
        overlap += ov;
        let same = a_paths.first() == b_paths.first();
        top1_same += same as usize;
        tsv.push_str(&format!(
            "{}\t{a_ms:.2}\t{b_ms:.2}\t{}\t{c_ms:.2}\t{}\t{ready:.1}\t{ov:.2}\t{}\n",
            q.chars().count(),
            b_used as u8,
            c_used as u8,
            same as u8
        ));
    }
    let n = queries.len();
    println!("{}", line("recall_probe stems only (wall)", stems_ms));
    println!(
        "{}",
        line("recall_probe meaning, model warm (wall)", warm_ms)
    );
    println!(
        "{}",
        line("recall_probe meaning, model cold (wall)", cold_ms)
    );
    println!(
        "{}",
        line("recall_probe cold: call start to vector ready", ready_ms)
    );
    println!(
        "recall_probe vector arrived within the wait: warm {warm_used}/{n}, cold {cold_used}/{n}"
    );
    println!(
        "recall_probe ranking, stems vs meaning (warm): top-5 overlap (Jaccard) {:.2} mean, same first item {top1_same}/{n}",
        overlap / n.max(1) as f64
    );
    if let Ok(out) = std::env::var("HOLLOW_RECALL_PROBE_OUT") {
        std::fs::write(out, tsv).expect("write the TSV");
    }
}

/// Whose turn a labelled query came from (N34): the person's or an agent's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Who {
    Own,
    Agent,
}

impl Who {
    fn name(self) -> &'static str {
        crate::recall_prefetch::who(self == Who::Own)
    }
}

/// One labelled query: what the turn opened or cited is `relevant`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Labelled {
    who: Who,
    relevant: Vec<String>,
    query: String,
}

/// The labels file: `who<TAB>paths<TAB>query` per line, `who` `own` or
/// `agent`, paths vault-relative and comma separated. Blank and `#` lines
/// are skipped, and so is a line with another `who`, no path or no query.
fn parse_labels(text: &str) -> Vec<Labelled> {
    text.lines()
        .filter_map(|line| {
            if line.trim().is_empty() || line.starts_with('#') {
                return None;
            }
            let mut cols = line.splitn(3, '\t');
            let who = match cols.next()?.trim() {
                "own" => Who::Own,
                "agent" => Who::Agent,
                _ => return None,
            };
            let relevant: Vec<String> = cols
                .next()?
                .split(',')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .collect();
            let query = cols.next()?.trim().to_string();
            (!relevant.is_empty() && !query.is_empty()).then_some(Labelled {
                who,
                relevant,
                query,
            })
        })
        .collect()
}

/// `paths` made vault-relative, first occurrence only, in order.
fn distinct_files(vault: &Path, paths: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for path in paths {
        let rel = Path::new(&path)
            .strip_prefix(vault)
            .map(|r| r.to_string_lossy().into_owned())
            .unwrap_or_else(|_| path.clone());
        if seen.insert(rel.clone()) {
            out.push(rel);
        }
    }
    out
}

/// The 1-based rank of the first file of `ranked` that is relevant.
fn first_relevant(ranked: &[String], relevant: &[String]) -> Option<usize> {
    ranked
        .iter()
        .position(|file| relevant.iter().any(|r| r == file))
        .map(|i| i + 1)
}

/// recall@5 and MRR (a miss counts 0) over `ranks`, as `retrieval_eval`
/// computes them.
fn recall5_mrr(ranks: &[Option<usize>]) -> (f64, f64) {
    (
        crate::retrieval_eval::recall_at(ranks, 5),
        crate::retrieval_eval::mrr(ranks),
    )
}

/// p50 and p95 (nearest rank) of `ms`.
fn p50_p95(mut ms: Vec<f64>) -> (f64, f64) {
    ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (pct(&ms, 0.5), pct(&ms, 0.95))
}

/// `each` over every item, `threads` at once (each thread takes the next
/// item): every call's wall ms and answer, in item order.
fn contended<T: Sync, R: Send>(
    threads: usize,
    items: &[T],
    each: &(dyn Fn(&T) -> R + Sync),
) -> Vec<(f64, R)> {
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<Option<(f64, R)>>> = Mutex::new((0..items.len()).map(|_| None).collect());
    std::thread::scope(|s| {
        for _ in 0..threads.max(1) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(item) = items.get(i) else {
                    break;
                };
                let t = Instant::now();
                let answer = each(item);
                let ms = t.elapsed().as_secs_f64() * 1000.0;
                out.lock().unwrap()[i] = Some((ms, answer));
            });
        }
    });
    out.into_inner()
        .unwrap()
        .into_iter()
        .map(|x| x.expect("every item ran"))
        .collect()
}

/// The lexical broker's pool for `query`, as distinct vault-relative files.
fn broker_files(vault: &Path, query: &str) -> Vec<String> {
    let scope = crate::retrieval_broker::Scope::all();
    let req = crate::retrieval_broker::Request {
        vault,
        query,
        scope: &scope,
        kinds: None,
    };
    let ranked =
        crate::retrieval_broker::retrieve_lexical(&req, "recall rank probe: lexical half only");
    distinct_files(vault, ranked.passages.into_iter().map(|p| p.path))
}

/// A turn's recall of `query` through the app's entry point: whether its
/// vector arrived in time, and the recalled paths in order.
fn turn_recall(vault: &Path, query: &str) -> (bool, Vec<String>) {
    let request = crate::turn_request::TurnRequest::from_text(vault, query);
    let result = request.recall_cued_by(vault, None, &request.request);
    (
        result.meaning == RecallMeaning::Used,
        result.items.into_iter().map(|i| i.path).collect(),
    )
}

fn env_number(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

#[test]
#[ignore = "measurement (J3a): needs HOLLOW_RECALL_RANK_VAULT and HOLLOW_RECALL_RANK_LABELS, and the bundled model for meaning"]
fn recall_rank_probe() {
    let vault =
        PathBuf::from(std::env::var("HOLLOW_RECALL_RANK_VAULT").expect("HOLLOW_RECALL_RANK_VAULT"));
    let labels = parse_labels(
        &std::fs::read_to_string(
            std::env::var("HOLLOW_RECALL_RANK_LABELS").expect("HOLLOW_RECALL_RANK_LABELS"),
        )
        .expect("labels file"),
    );
    assert!(!labels.is_empty(), "no labelled queries");
    let threads = env_number("HOLLOW_RECALL_RANK_THREADS", 4) as usize;
    let admit = Duration::from_millis(env_number("HOLLOW_RECALL_RANK_ADMIT_MS", 126));
    // The keyword index under the test's temp root, deleted at the end.
    let _search_root = crate::retrieval_eval::SearchIndexRoot::new();
    crate::vault_search_index::engage(&vault);
    let model = crate::embed_model::default_model_dir();
    let meaning = model.join("model.safetensors").is_file();
    if meaning {
        index::set_semantic_enabled(true);
        index::engage(&vault);
        index::embed_engaged_for_probe(&vault, &model);
    }
    println!(
        "recall_rank_probe: {} labelled queries ({} own, {} agent), {threads} at once, admission {} ms, meaning {}",
        labels.len(),
        labels.iter().filter(|l| l.who == Who::Own).count(),
        labels.iter().filter(|l| l.who == Who::Agent).count(),
        admit.as_millis(),
        if meaning { "on" } else { "off (no model)" }
    );

    // Ranking, per arm and per who.
    let mut ranks: BTreeMap<(Who, &str), Vec<Option<usize>>> = BTreeMap::new();
    let mut tsv = String::from(
        "who\tquery_chars\tbroker_rank\trecall_rank\tbroker_ms\tidle_ms\tidle_used\tloaded_ms\tloaded_used\tprefetch\tturn_waited_ms\n",
    );
    let mut rows = Vec::new();
    for l in &labels {
        let t = Instant::now();
        let pool = broker_files(&vault, &l.query);
        let broker_ms = t.elapsed().as_secs_f64() * 1000.0;
        let broker = first_relevant(&pool, &l.relevant);
        assert!(wait_idle(Duration::from_secs(30)));
        let recalled = turn_recall(&vault, &l.query).1;
        let recall = first_relevant(&recalled, &l.relevant);
        ranks.entry((l.who, "broker")).or_default().push(broker);
        ranks.entry((l.who, "recall")).or_default().push(recall);
        rows.push((broker, recall, broker_ms));
    }
    for ((who, arm), r) in &ranks {
        let (r5, rr) = recall5_mrr(r);
        println!(
            "recall_rank_probe {} {arm}: n {} recall@5 {r5:.3} MRR {rr:.3}",
            who.name(),
            r.len()
        );
    }

    // Turn recall alone, then `threads` at once.
    assert!(wait_idle(Duration::from_secs(30)));
    let idle = contended(1, &labels, &|l: &Labelled| turn_recall(&vault, &l.query).0);
    assert!(wait_idle(Duration::from_secs(30)));
    let loaded = contended(threads, &labels, &|l: &Labelled| {
        turn_recall(&vault, &l.query).0
    });
    // The prefetch as shipped: started at accept, taken an admission later.
    assert!(wait_idle(Duration::from_secs(30)));
    let pickups = contended(threads, &labels, &|l: &Labelled| {
        let (v, q) = (vault.clone(), l.query.clone());
        let prefetch = crate::recall_prefetch::Prefetch::start(move || turn_recall(&v, &q).0);
        std::thread::sleep(admit);
        prefetch
            .take(
                crate::recall_prefetch::RECALL_PREFETCH_CUTOFF,
                index::RECALL_MEANING_WAIT,
            )
            .1
    });
    for who in [Who::Own, Who::Agent] {
        let mine = |v: &[(f64, bool)]| -> (Vec<f64>, usize) {
            let picked: Vec<&(f64, bool)> = v
                .iter()
                .zip(&labels)
                .filter(|(_, l)| l.who == who)
                .map(|(x, _)| x)
                .collect();
            (
                picked.iter().map(|(ms, _)| *ms).collect(),
                picked.iter().filter(|(_, used)| *used).count(),
            )
        };
        let (idle_ms, idle_used) = mine(&idle);
        let (loaded_ms, loaded_used) = mine(&loaded);
        let n = idle_ms.len();
        if n == 0 {
            continue;
        }
        let (i50, i95) = p50_p95(idle_ms);
        let (l50, l95) = p50_p95(loaded_ms);
        let picks: Vec<&crate::recall_prefetch::Pickup> = pickups
            .iter()
            .zip(&labels)
            .filter(|(_, l)| l.who == who)
            .map(|((_, p), _)| p)
            .collect();
        let ready = picks
            .iter()
            .filter(|p| p.outcome == crate::recall_prefetch::Outcome::Ready)
            .count();
        let most = picks
            .iter()
            .map(|p| p.waited.as_secs_f64() * 1000.0)
            .fold(0.0, f64::max);
        println!(
            "recall_rank_probe {} turn recall: alone p50 {i50:.1} p95 {i95:.1} ms, vector {idle_used}/{n}; {threads} at once p50 {l50:.1} p95 {l95:.1} ms, vector {loaded_used}/{n}; prefetch ready {ready}/{n}, turn waited at most {most:.1} ms",
            who.name()
        );
    }
    for (i, l) in labels.iter().enumerate() {
        let (broker, recall, broker_ms) = rows[i];
        let rank = |r: Option<usize>| r.map_or("-".to_string(), |r| r.to_string());
        tsv.push_str(&format!(
            "{}\t{}\t{}\t{}\t{broker_ms:.1}\t{:.1}\t{}\t{:.1}\t{}\t{}\t{:.1}\n",
            l.who.name(),
            l.query.chars().count(),
            rank(broker),
            rank(recall),
            idle[i].0,
            idle[i].1 as u8,
            loaded[i].0,
            loaded[i].1 as u8,
            pickups[i].1.outcome.name(),
            pickups[i].1.waited.as_secs_f64() * 1000.0
        ));
    }
    if let Ok(out) = std::env::var("HOLLOW_RECALL_RANK_OUT") {
        std::fs::write(out, tsv).expect("write the TSV");
    }
}

#[test]
fn labels_read_who_paths_and_query_and_skip_the_rest() {
    let text = "# who\tpaths\tquery\n\
                own\tlibrary/a.md, tasks/b.md\tWhere is the deploy runbook?\n\
                agent\tlearnings/c.md\tfix the\tflaky test\n\
                \n\
                someone\tlibrary/a.md\tnot a who\n\
                own\t\tno paths\n\
                agent\tlibrary/a.md\t  \n";
    let labels = parse_labels(text);
    assert_eq!(
        labels,
        vec![
            Labelled {
                who: Who::Own,
                relevant: vec!["library/a.md".into(), "tasks/b.md".into()],
                query: "Where is the deploy runbook?".into(),
            },
            Labelled {
                who: Who::Agent,
                relevant: vec!["learnings/c.md".into()],
                query: "fix the\tflaky test".into(),
            },
        ]
    );
    assert_eq!(Who::Own.name(), "own");
    assert_eq!(Who::Agent.name(), "agent");
}

#[test]
fn the_rank_is_the_first_relevant_distinct_file() {
    let vault = Path::new("/v");
    let files = distinct_files(
        vault,
        [
            "/v/library/a.md",
            "/v/library/a.md",
            "/v/tasks/b.md",
            "elsewhere/c.md",
            "/v/learnings/d.md",
        ]
        .map(String::from),
    );
    assert_eq!(
        files,
        vec![
            "library/a.md",
            "tasks/b.md",
            "elsewhere/c.md",
            "learnings/d.md"
        ]
    );
    let rel = |p: &[&str]| p.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(first_relevant(&files, &rel(&["learnings/d.md"])), Some(4));
    assert_eq!(
        first_relevant(&files, &rel(&["learnings/d.md", "tasks/b.md"])),
        Some(2)
    );
    assert_eq!(first_relevant(&files, &rel(&["nowhere.md"])), None);
}

#[test]
fn recall_at_5_and_mrr_count_a_miss_as_zero() {
    let (r5, rr) = recall5_mrr(&[Some(1), Some(3), Some(6), None]);
    assert!((r5 - 0.5).abs() < 1e-9, "{r5}");
    assert!(
        (rr - (1.0 + 1.0 / 3.0 + 1.0 / 6.0) / 4.0).abs() < 1e-9,
        "{rr}"
    );
    assert_eq!(recall5_mrr(&[]), (0.0, 0.0));
    let (p50, p95) = p50_p95((1..=100).map(f64::from).collect());
    assert_eq!((p50, p95), (51.0, 95.0));
}

#[test]
fn contended_runs_every_item_on_at_most_the_threads_asked() {
    let running = AtomicUsize::new(0);
    let most = AtomicUsize::new(0);
    let items: Vec<u64> = (0..8).collect();
    let out = contended(4, &items, &|i: &u64| {
        let now = running.fetch_add(1, Ordering::SeqCst) + 1;
        most.fetch_max(now, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(30));
        running.fetch_sub(1, Ordering::SeqCst);
        *i * 2
    });
    assert_eq!(
        out.iter().map(|(_, v)| *v).collect::<Vec<_>>(),
        vec![0, 2, 4, 6, 8, 10, 12, 14]
    );
    assert!(out.iter().all(|(ms, _)| *ms >= 30.0));
    // At least one at a time and never more than the threads asked for (a
    // loaded machine may run them one by one: no lower bound above 1).
    let most = most.load(Ordering::SeqCst);
    assert!((1..=4).contains(&most), "{most} at once");
}
